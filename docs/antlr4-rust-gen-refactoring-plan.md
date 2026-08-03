# `antlr4-rust-gen` source decomposition plan

Status: proposed

Prepared: 2026-08-02

Repository baseline: `f85c971d913d9f544f0b6609c4d55a3a97755b4c`

Revision: adversarially reviewed for optimization issues `#125`, `#128`-`#131`,
and multi-recognizer issue `#276`

## 1. Decision summary

Refactor and package the generator in three deliberately separate structural
stages, with behavior-changing optimization work tracked separately:

1. Split `src/bin/antlr4-rust-gen.rs` into real Rust modules under
   `src/bin_support/codegen/`, while it remains part of the current package.
   Keep the CLI, generated files, manifests, diagnostics, and performance
   behavior unchanged. This is the work that directly removes the merge
   hotspot.
2. Once parser surface modeling has its own boundary, investigate and implement
   [issue #276](https://github.com/ophi-dev/antlr-rust-runtime/issues/276) as a
   separate, intentionally output-changing stage. Move recognizer-independent
   typed-context operations behind a narrow, independently revisioned runtime
   support contract while preserving the public generated API. This stage must
   prove benefits in binaries that link one, two, and three recognizers;
   generated source compaction alone is not evidence of linked-code
   deduplication.
3. After those module and runtime-support boundaries have survived normal
   feature work, convert
   the repository to a workspace with a separately published codegen package.
   Keep the runtime package at the repository root and keep the two published
   packages on one lockstep release version.

The module split must model the whole compiler pipeline, not just decompose the
Rust source renderer. Future optimizations need explicit boundaries at the
integrated grammar, ATN, parser IR, routing, and structured render-model stages.
Each representation gets a stage-specific pass contract; there is no universal
"optimizer" trait and no optimization of generated Rust text.

The recommended order is stage A source decomposition, separately reviewed
optimization/runtime behavior changes, then stage B workspace conversion. R0
and the disposable R0.5 ABI spike can run in parallel with A7-A11 after A6
establishes the surface boundary; issue #276 must not block the rest of the
source split. After A11, O0 and R1-R2 are independent behavior-changing tracks.
Prefer landing the supported runtime ABI and generated binding change after A11.
The workspace must not be combined with either earlier stage. It gives the
compiler its correct dependency and release boundary, but it neither solves
conflicts inside one 23,000-line file nor makes generated helper code shareable.
It is a packaging boundary, not the optimization or generated-support
architecture.

Stage A must not increment `__ANTLR4_RUST_CODEGEN_API`; moving generator
implementation code does not alter the generated-source/runtime contract.
The stage-R generated binding change must allocate the next global revision
(`N+1`, where `N` is current when that change lands) because newly generated
source will require runtime support that revision `N` lacks. The context-support
contract is versioned independently as `context_v1`; global codegen revisions
map to support-contract revisions rather than sharing their numbers. Stage B
does not require another increment unless it independently changes generated
Rust or runtime APIs.

## 2. Current state

At the baseline above:

| Area | Size or count |
| --- | ---: |
| `src/bin/antlr4-rust-gen.rs` | 23,132 lines |
| Production portion of that file | 16,939 lines |
| In-file unit-test portion | 6,193 lines |
| In-file `#[test]` functions | 161 |
| In-file Insta calls | 64 |
| Snapshot files under `src/bin/snapshots/` | 63 |
| `tests/antlr4_rust_gen_cli.rs` | 6,668 lines, 129 tests |
| `src/bin_support/embedded.rs` | 8,107 lines: 5,062 production, 3,045 tests |
| `src/bin_support/rust_syntax/mod.rs` | 2,775 lines: 1,863 production, 912 tests |
| Generated Rust-syntax recognizers | 92,795 generated lines; not a manual refactor target |
| Checked-in generated parser contexts | 282 total: 67 ANTLRv4 and 215 Rust |
| Checked-in generated parser source | 118,515 lines, approximately 6.06 MB |
| Existing grammar compiler under `src/bin_support/grammar/` | already split by phase |

At review time, the sister Mehen workspace links independent generated Java and
Kotlin parser crates and is adding a third ANTLR parser. Its two current parser
modules alone contain 317 generated contexts across 107,439 lines and
approximately 6.24 MB. This is the representative consumer shape for stage R,
not an edge case optimized only for this repository's checked-in frontends.

The main file's production responsibilities are approximately:

| Baseline lines | Responsibility |
| --- | --- |
| 1-391 | process entry, compilation orchestration, output writing, diagnostics |
| 392-2257 | semantic policy, pattern-file parsing, inventories, JSON manifests |
| 2258-2473 | CLI parsing and usage |
| 2474-3691 | codegen input bag and structural grammar/ATN projection |
| 3692-4170 | lexer source generation |
| 4171-9022 | generated-parser IR, lowering, routing heuristics, and rule rendering |
| 9023-9927 | LL(1)/fixed-lookahead decision analysis and reporting |
| 9928-14475 | embedded parser data, antlr4rust compatibility, and typed tree APIs |
| 14476-16939 | action/predicate templates, hooks, SemIR tables, metadata helpers |
| 16940-23132 | unit tests and test fixtures |

These are not merely adjacent helpers. They are different compiler phases with
different change reasons and review expertise.

### 2.1 Structural problems

The single module currently hides dependencies instead of controlling them:

- Every item can refer to every other private item, so ownership is implicit.
- `CodegenData` is one lexer-or-parser bag with optional lexer ATN, parser ATN,
  semantic model, graph, provenance, and source fields. Callers discover the
  valid combination at runtime.
- `render_parser_with_decision_report` performs analysis, policy decisions,
  generated-rule selection, API-surface construction, and final string
  assembly in one path.
- Semantic domain types are declared far from the collectors and renderers that
  own them. For example, `PredicateTemplate` is below the parser renderer but
  is used by semantic-pattern parsing near the top.
- Parser lowering and parser source rendering share the same data structures,
  making it difficult to test or change either independently.
- Optional transform selection is assembled as flag-specific conditionals in
  `main`, so each new pass would add configuration and ordering logic to the
  driver.
- `src/bin_support/grammar/transform.rs` is itself 2,320 lines and combines
  import integration, pass execution, validation, reporting DTOs, and JSON
  serialization. Its shared `TransformCandidateReport` already contains
  precedence-ladder-specific fields such as rungs and grouping changes.
- `src/bin_support/embedded.rs` has become a second large hand-written hotspot.
  It combines structural models, member parsing, `$`-attribute translation,
  antlr4rust compatibility lowering, lexical scope analysis, and tests.
- `optimizations.json` is currently rendered from the grammar transform report
  before parser decision classification. It cannot correlate a source rewrite
  with downstream ATN, adaptive-decision, generated-IR, or artifact-size
  effects required by #125.
- `render_embedded_context_types` interleaves grammar-specific context identity,
  attributes, accessors, and callback dispatch with grammar-independent
  stored/active/validated constructors, child iteration, common accessors, and
  display mechanics. The latter bodies are emitted for every context.
- Generated parser modules also repeat once-per-recognizer wrappers, validation
  infrastructure, traversal bridges, facade delegation, and ATN cache setup.
  These have different extraction and performance tradeoffs from per-context
  duplication and must not be treated as one renderer-layout problem.
- The generated-code compatibility test proves the current revision and a
  synthetic unsupported revision, but not two accepted generated revisions
  linked into one binary.
- Unit tests are grouped by historical arrival rather than by the component
  they protect.
- The CLI integration suite is a second large conflict surface even though it
  already exercises separable user-facing areas.

### 2.2 Existing boundaries worth preserving

The refactor should build on boundaries that already work:

- `src/bin_support/grammar/` owns `.g4` loading, syntax, transforms, semantic
  analysis, provenance, and ATN construction. Preserve this compiler/codegen
  boundary, but split its mixed transform/integration/reporting internals.
- `src/bin_support/embedded.rs` owns the embedded-action structural model and
  `$`-attribute and antlr4rust compatibility translation. Preserve its
  target-language boundary, but split its internal concerns.
- `src/bin_support/rust_syntax/` owns grammar-backed Rust syntax analysis used
  by embedded compatibility lowering. Keep its checked-in generated
  recognizers isolated from hand-written modules and reproducible by the
  existing updater.
- `src/bin_support/stack_member.rs` owns its small lowering DSL.
- `src/bin_support/templates.rs` owns target-template tokenization helpers.
- `src/bin_support/rust_names.rs` owns Rust identifier conversion.
- Runtime APIs stay in `src/`; codegen-only models must not migrate into the
  runtime library merely to make imports convenient.
- Runtime tree primitives already own `RuleNodeView`, active
  `ParserRuleContext`, terminal/error views, child iteration, and the generic
  tree walker. Stage R should build a narrow generated-support ABI over those
  primitives rather than copy codegen models into the runtime.

## 3. Goals and non-goals

### 3.1 Goals

1. Make lexer, parser prediction, typed-tree API, embedded compatibility,
   semantic lowering, grammar frontend, and CLI work land in different files.
2. Give each compiler phase an explicit input and output instead of access to
   the entire generator module.
3. Keep generated source and manifests byte-for-byte stable during the module
   extraction.
4. Keep diagnostics, diagnostic order, and exit behavior stable.
5. Move tests next to the implementation they protect and retain one shared
   fixture-builder module.
6. Make the binary entry point small enough that ordinary feature work never
   touches it.
7. Prepare a codegen crate boundary without prematurely splitting compiler
   phases into separately versioned crates.
8. Give grammar, ATN, parser-IR, routing, and render-model optimizations explicit
   stage-local inputs, invariants, validation, and reporting.
9. Make pass selection, safety policy, ordering, analysis invalidation, and
   cross-phase metrics scale without pass-specific booleans in the CLI driver.
10. Make the grammar-specific parser surface model independently consumable by
    source rendering and generated-support ABI lowering.
11. Prepare issue #276 so recognizer-independent mechanics can have one runtime
    implementation per binary without moving grammar identities, attributes,
    accessors, or control flow into the runtime.
12. Measure generated source, clean compile cost, linked code/data size, and
    runtime behavior separately for controlled zero-, one-, two-, and
    three-parser consumers before validating against repository and Mehen
    shapes.

### 3.2 Non-goals

- During stage A, do not change generated Rust APIs, generated bytes, or the
  codegen API revision.
- During stage R, preserve the public generated API while intentionally changing
  generated implementation details and the codegen API revision.
- Do not change parser routing, fixed-lookahead classification, semantic
  policy, output formatting, or CLI syntax while extracting modules.
- Do not replace string rendering with a template engine in this project.
- Do not add serde solely to replace the deterministic manifest writers.
- Do not expose the grammar compiler through `antlr4_runtime`.
- Do not create separate crates for grammar frontend, semantic analysis,
  parser lowering, and rendering. Those APIs are still evolving together.
- Do not force grammar transforms, ATN canonicalization, parser-IR passes, and
  render-layout passes through one generic trait.
- Do not optimize by rewriting emitted Rust strings. Render-only optimization
  must operate on a structured render model and cannot alter recognition,
  recovery, diagnostics, tree shape, or generated APIs.
- Do not add grammar names, language rule names, language file extensions, or
  fixture-specific semantic behavior to generic compiler/codegen modules.
- Do not move grammar metadata, serialized ATN/DFA data, rule/token/alternative
  identities, attributes, or typed accessor signatures into the runtime.
- Do not remove or rename generated nominal APIs such as `__RuleAttrsN`,
  context wrappers, validation types, or callback traits as part of internal
  storage deduplication.
- Do not claim that a runtime macro or generic helper creates one linked copy;
  macro expansion and monomorphization must be measured and reported honestly.

## 4. Design rules

### 4.1 Dependency direction

The target dependency direction is:

```text
cli -> CompilerConfig
             |
             v
driver -> compiler pipeline ---------------------------------> artifact writer
             |                                                       ^
             v                                                       |
        loaded syntax                                                |
             |                                                       |
             v                                                       |
      integrated grammar                                             |
             |                                                       |
             v                                                       |
   optional grammar-model passes                                     |
             |                                                       |
             v                                                       |
      semantic grammar                                               |
   (mandatory rewrites/checks)                                       |
             |                                                       |
             v                                                       |
       mutable ATN graph                                              |
             |                                                       |
             v                                                       |
  mandatory ATN canonicalization                                     |
             |                                                       |
             v                                                       |
   finalized/analyzed artifacts                                      |
        |                    |                                       |
        |                    +-> parser surface model                 |
        |                    |        |                               |
        |                    |        +-> generated-support bindings  |
        |                    +-> lowered parser IR                    |
        |                    +-> decision analysis                    |
        |                    +-> optional parser-IR passes            |
        |                    +-> routing plan                         |
        |                    +-> parser render model -----------------+
        |                                                            |
        +-> lexer render model ---------------------------------------+

Every stage -----------------> compilation/optimization report
```

Detailed constraints:

- `grammar` must not depend on `codegen`.
- The CLI driver converts arguments to `CompilerConfig`, invokes one pipeline,
  and commits its artifacts. It does not know concrete pass types or order.
- Stage order and validation live in the compiler pipeline, not in renderers or
  pass-specific CLI branches.
- Mandatory grammar rewrites and ATN canonicalization are compiler semantics,
  not user-selectable optimizations.
- `cli` may depend on option/domain types, but no renderer may read
  `std::env::args` directly.
- Filesystem writes belong to the driver/artifact boundary. Renderers return
  values.
- Lexer modules must not import parser render modules, and parser modules must
  not import lexer render modules.
- Shared semantic enums and DTOs belong in `semantics::model`, not in either
  renderer.
- Decision report DTOs belong to parser decision analysis. The manifest writer
  consumes them but does not classify decisions.
- Common Rust output helpers are leaves. They must not call back into lexer or
  parser planning. Sharing a renderer helper inside codegen does not make the
  Rust emitted by that helper shared in a consumer binary.
- The runtime must not depend on codegen. Generated-support contract operations
  consume only runtime-owned tree/parser/token types, scalar indexes, and
  neutral events; generated wrappers retain grammar-owned descriptors.
- Cross-module visibility should default to private, then `pub(super)`.
  `pub(crate)` is reserved for the small codegen facade and genuinely shared
  domain types. Do not make every moved symbol `pub(crate)` to get the first
  build passing.

### 4.2 Real modules, not textual fragments

Do not use permanent `include!` files. They would reduce physical conflicts but
retain one namespace, unrestricted coupling, and poor rustdoc/compiler
diagnostics. A temporary local extraction commit may use mechanical aids, but
the reviewed result must use `mod` boundaries and explicit imports.

### 4.3 Planning before rendering

The final design uses successive typed artifacts rather than one monolithic
all-purpose parser plan:

```rust
struct LexerRenderModel<'a> { /* fully analyzed lexer emission input */ }
struct LoweredParserIr<'a> { /* ATN lowered to generated-rule steps */ }
struct OptimizedParserIr<'a> { /* validated result of optional IR passes */ }
struct ParserDecisionAnalysis { /* LL(1), adaptive, fixed-lookahead facts */ }
struct RoutingPlan { /* generated/interpreted engine selection */ }
struct ParserSurfaceModel<'a> { /* grammar-specific public API shape */ }
struct GeneratedSupportBindings { /* global revision -> support contracts */ }
struct ParserRenderModel<'a> { /* final parser module assembly input */ }

struct GeneratedArtifacts {
    files: BTreeMap<PathBuf, String>,
}
```

Exact fields should follow the implementation, but the contracts matter:

- Every stage artifact is deterministic, validated, and carries stable
  provenance needed by later reports.
- `LexerRenderModel` contains everything required by lexer rendering.
  `ParserRenderModel` composes independently built execution and surface
  submodels for top-level module assembly; a surface renderer never receives
  parser IR, routing, or decision internals.
- `ParserSurfaceModel` owns generated API shape independently of whether its
  mechanics are emitted inline (stage A) or bound to runtime support (stage R).
- `GeneratedSupportBindings` selects the generated-code API revision and
  fully-qualified support paths. It does not own grammar analysis or Rust
  formatting.
- Rendering a render model must not repeat grammar/ATN/IR analysis.
- A renderer must not read source files, environment variables, or CLI state.
- `GeneratedArtifacts` detects normalized module-name collisions before any
  file is written.

Introduce the artifacts after the mechanical file moves. Combining a move with
deduplicating analysis would make output or diagnostic-order regressions hard
to attribute. Do not create empty pass traits speculatively: establish each
typed stage now, and add a stage-specific trait when that stage has a second
selectable implementation.

### 4.4 Typed lexer and parser input

Replace `CodegenData` after extraction with typed views:

```rust
struct CommonCodegenData<'a> {
    literal_names: &'a [Option<String>],
    symbolic_names: &'a [Option<String>],
    rule_names: &'a [String],
    channels: ChannelTable<'a>,
    modes: ModeTable<'a>,
    semantic: &'a SemanticGrammar,
    graph: &'a FinalizedAtnGraph,
    provenance: &'a ProvenanceIndex,
    sources: &'a SourceSet,
}

struct LexerCodegenData<'a> {
    common: CommonCodegenData<'a>,
    atn: &'a LexerAtn,
    atn_words: &'a [i32],
    dfa_words: &'a [u32],
}

struct ParserCodegenData<'a> {
    common: CommonCodegenData<'a>,
    atn: &'a ParserAtn,
}
```

This removes invalid lexer/parser states and the repeated
`"artifact is unavailable"` checks. It also makes module ownership visible in
function signatures. Preserve borrowing where practical; do not turn this
refactor into an allocation campaign.

### 4.5 Multi-recognizer generated-support boundary

Generated code has three ownership classes. The surface model must classify
each emitted item before stage R moves anything:

| Ownership | Examples | Required treatment |
| --- | --- | --- |
| Candidate recognizer-independent runtime core | non-generic stored/active context operations over runtime-owned nodes; named child/terminal iterator implementations; invocation-state chain lookup/formatting; neutral child scanning and validation-event primitives | Prototype behind a narrow generated-support contract. Prefer non-generic or type-erased operations when the goal is one linked implementation per binary; retain only candidates that the R0.5 spike proves useful. |
| Grammar-branded generated data and public API | metadata and ATN/DFA words; rule/token/alternative IDs and names; context newtypes; recovered/active/stored/validated context types and errors; every `__RuleAttrsN` type and trait implementation; typed accessors; labels/cardinalities; listener/visitor traits and callback dispatch | Keep generated. Internal storage and lookup bodies may delegate to the runtime, but public nominal types, names, signatures, trait behavior, and grammar-specific dispatch remain generated. |
| Generated execution and specialization | recursive-descent rule bodies; prediction and semantic code; validation policy; context/accessor wrappers; parser/lexer, hook, listener, and visitor adapters; surfaces enabled only by generator flags | Keep generated unless measurements prove a runtime move preserves optimization, auto-trait behavior, and feature isolation. |

Issue #276 should introduce a path such as
`antlr4_runtime::generated::support::context_v1`. The exact item names may
follow the implementation, but the contract rules are fixed:

- it is `#[doc(hidden)]` but is still a supported public source ABI for every
  accepted generated-code revision mapped to it;
- the context-support revision is independent from the global
  `__ANTLR4_RUST_CODEGEN_API` revision. `GeneratedSupportBindings` maps global
  revision `N+1` to `context_v1`, and a later global revision may continue
  using `context_v1` when this contract has not changed;
- support is namespaced by contract revision rather than exposed through
  unstable unversioned aliases, and it is not re-exported from the runtime
  crate root;
- it depends only on runtime-owned representations, not
  `ParserSurfaceModel`, grammar frontend types, or codegen options;
- generated references are fully qualified, and any exported declaration macro
  uses `$crate`-qualified paths;
- macros are limited to thin type declarations and impl plumbing. Common work
  intended to have one machine-code copy lives in non-generic runtime
  functions; a macro expansion or generic monomorphization is not counted as
  deduplication;
- public generated names and signatures remain unchanged, including context
  newtypes, recovered/active/stored/validated types and errors, typed accessors,
  `__RuleAttrsN` types and trait implementations, and listener/visitor
  callbacks;
- rules without attributes may use zero-sized payloads and remove redundant
  internal storage/lookups, but their existing public `__RuleAttrsN` nominal
  APIs remain until an explicit compatibility policy permits removing them;
- validation may consume neutral runtime scans or events, but grammar-branded
  validation types, diagnostics, and dispatch remain generated;
- lexer-only and listener/visitor-disabled consumers do not acquire unrelated
  generated surfaces merely because support exists in the runtime.

This extraction is not a structured render-model optimization. It changes
generated dependencies and the generated-source/runtime contract, so it belongs
to stage R with an explicit global API revision and consumer measurements. R0.5
must validate the candidate boundary in disposable code before any
`context_v1` path becomes a supported runtime contract.

### 4.6 Optimization stages and legality

[Issue #125](https://github.com/ophi-dev/antlr-rust-runtime/issues/125) and
follow-ups `#128`-`#131` require optimizations at different representations.
They must not share one mutation API:

| Stage | Allowed work | Forbidden work | Current/future examples |
| --- | --- | --- | --- |
| Integrated grammar model | Proven structural source rewrites before final numbering and ATN construction | Guessing around actions, predicates, labels, precedence, lexer priority, or target intent | precedence-ladder collapse; #129 factoring; #130 inlining; #131 subsumption |
| Semantic grammar | Mandatory language semantics and checks | User-selectable performance rewrites hidden inside semantic analysis | immediate/mutual left-recursion handling |
| Mutable ATN graph | Semantics-preserving canonicalization before final packing | Tree/API-changing source rewrites or untracked state removal | set collapse and tail-epsilon removal |
| Analyzed parser decisions | Prediction specialization with an explicit faithful fallback | Changing alternative order, recovery, predicate timing, or context semantics | LL(1), fixed lookahead, bounded experiments such as rejected #281 |
| Generated parser IR | Validated structured control-flow transformations | Editing rendered Rust or bypassing semantic/recovery steps | future rule/step deduplication or specialization |
| Routing plan | Selecting generated versus interpreted execution from analyzed costs/capabilities | Rewriting grammar, ATN, or IR | adaptive-ATN preference and generated-only checks |
| Structured render model | Layout/deduplication that is observationally inert | Recognition, recovery, diagnostics, tree/API, or fallback changes | import/literal/table layout only |

Each selectable pass has a stable descriptor:

```rust
struct PassDescriptor {
    id: PassId,
    stage: OptimizationStage,
    safety: SafetyClass,
    canonical_order: usize,
    prerequisites: &'static [PassId],
    conflicts: &'static [PassId],
}

struct OptimizationConfig {
    mode: OptimizationMode, // disabled, apply, or report-only
    safety_ceiling: SafetyClass,
    enabled: Vec<ConfiguredPass>,
}
```

At minimum, `SafetyClass` retains the two existing compatibility contracts:

- **Tree/API preserving** keeps the accepted token language, generated
  rule/token surfaces, labels, listener/visitor events, parse-tree shape,
  semantic timing, and tested error/recovery behavior compatible.
- **Recognition preserving** guarantees recognition parity for complete valid
  inputs but may change tree/API or malformed-input behavior. Every affected
  surface must be reported, and both the pass and this policy level require
  explicit opt-in.

The class is a compatibility claim, not a profitability score, and it cannot be
inferred from the stage where the pass runs. All optional passes remain off by
default.

The exact representation can differ, but these properties are required:

- Pass IDs and order are stable and independent of CLI argument order.
- Unknown, duplicate, conflicting, or unsatisfied pass selections fail before
  compilation.
- Grammar passes mutate an owned integrated copy. Loaded/authored syntax stays
  immutable for diagnostics, audit output, and differential comparison.
- A typed grammar-pass result keeps the transformed model, provenance/source
  map, report events, and optional transformed-source audit artifact together.
  Semantic analysis and ATN construction consume that result; callers cannot
  pair a transformed grammar with stale pre-transform artifacts.
- Recognition-preserving passes require policy-level opt-in; enabling one pass
  must not silently authorize all passes in that safety class.
- Per-pass budgets and options belong to `ConfiguredPass`, not fields added to
  `driver.rs`.
- Existing dedicated flags may map into `OptimizationConfig` during migration,
  but the pipeline must not gain one driver boolean and `if` block per pass.
- After the explicit O0 reporting transition, report-only mode runs a
  transformed shadow artifact through downstream semantic, ATN, decision, IR,
  and render-model analysis so projected cost effects are real. Stage A only
  establishes the non-committing API and preserves the current report command's
  early return, diagnostics, exit status, output, and generation work.
- Passes never write files. Deterministic transformed `.g4` and source-map
  artifacts, when requested, flow through `GeneratedArtifacts` and cannot
  overwrite an authored input path.
- Pass implementations live in stage-owned modules. A small central built-in
  registration table is acceptable; pass-specific logic and report DTOs are
  not.

### 4.7 Analysis invalidation

Analysis caching is stage-local. There is no global analysis object spanning
grammar, ATN, and parser IR.

For the grammar transform stage:

- analyses are requested through methods rather than read as public fields;
- the editable model has a monotonically increasing revision;
- cached facts are tagged with the model revision and their dependency set;
- a pass conservatively declares analyses it preserves; mutation invalidates
  everything else plus dependent analyses;
- model validation runs after every changed pass and before another pass can
  observe the model;
- final semantic analysis is always built from the transformed model.

The existing `AnalysisInvalidation` mask can remain as a compatibility adapter
while modules move, but the target contract must make stale reads impossible.
Debug/test builds should compare selected cached results with a clean full
recomputation. Similar stage-local managers may be added for ATN or IR only
when repeated analyses justify them.

### 4.8 Cross-phase reporting

Reporting is a neutral compiler service, not a grammar-transform serializer or
a parser-render side effect.

`CompilationReport` accumulates deterministic events keyed by stable pass,
grammar, recognizer, decision, and IR-node IDs. Provenance and tombstones carry
authored origins through grammar, ATN, and parser IR stages. The common pass
envelope records:

- descriptor, stage, safety class, mode, and effective parameters;
- applied/eligible/declined status and a reason;
- authored/transformed coordinates, source-map IDs, and affected generated
  API/tree surfaces;
- before/after structural, ATN, decision-tier, IR, and artifact-size metrics;
- links to downstream decisions or IR nodes affected by a rewrite.

Pass-specific proof evidence is owned by the pass behind a private deterministic
reporting interface, including its normalized proof and replacement
model/text. The shared report model must not grow fields such as `rungs`,
`boundaryRule`, or `groupingChanges` for one pass. This does not require serde;
the existing deterministic manual JSON policy can use a small internal
report-value/writer abstraction.

After O0, assemble `optimizations.json` only after downstream plans and
in-memory artifacts are available. During stage A, its compatibility writer
continues at the current early-return point with byte-identical content. Keep
`decisions.json` as the detailed inventory of all parser decisions, but link its
rows to optimization pass IDs where applicable. Wall-clock timings, RSS, and
benchmark samples are nondeterministic and belong in an optional trace or
published benchmark result, not the deterministic manifest.

## 5. Proposed source tree

Names may adjust during implementation, but responsibility and dependency
direction should remain as follows:

```text
src/
  generated/
    mod.rs                          GrammarMetadata and generated recognizer traits
    support/
      mod.rs                        doc-hidden, no unversioned ABI re-exports
      context_v1.rs                 independently revisioned context support

src/bin/
  antlr4-rust-gen.rs              process entry only

src/bin_support/
  rust_names.rs                   shared generator/testsuite identifier helpers
  optimization/
    mod.rs                        neutral optimization/reporting facade
    config.rs                     profiles, pass selection, safety policy
    descriptor.rs                 stable pass IDs, stages, dependencies/order
    report.rs                     CompilationReport and deterministic manifest
    metrics.rs                    stage-neutral named metric snapshots

  grammar/
    mod.rs                        source-compiler facade
    integration.rs                imports/combined split/mandatory integration
    validation.rs                 post-mutation model invariants
    transform/
      mod.rs                      GrammarTransform stage contract
      registry.rs                 canonical selection/order and execution
      analysis.rs                 revisioned transform-stage analyses
      artifact.rs                 transformed source/source-map audit output
      passes/
        precedence_ladder.rs      existing optional pass
        prune_unreachable.rs      existing optional pass
        factoring.rs              future #129
        inline_rules.rs           future #130
        subsumed_alternatives.rs  future #131
    atn/
      optimize.rs                 mandatory ATN canonicalization only
    ...                           existing frontend/model/semantic/ATN modules

  codegen/
    mod.rs                        private facade and top-level exports
    cli.rs                        Args, CliCommand, parse_from, usage
    driver.rs                     config -> pipeline -> artifact commit
    pipeline.rs                   typed stage sequencing and validation
    artifact.rs                   output set, collision checks, filesystem write
    model.rs                      typed common/lexer/parser codegen inputs
    rust_output.rs                syntax-only module frame and Rust literals

    structural/
      mod.rs                      structural action/predicate/rule-call inventory
      contexts.rs                 grammar alternatives -> child/ref cardinalities

    embedded/
      mod.rs                      embedded-action facade
      model.rs                    AttrDecl, ElementRef, AltModel, RuleModel
      members.rs                  @members classification
      translate.rs                $-attribute/body translation
      antlr4rust/
        mod.rs                    compatibility lowering facade
        aliases.rs                token/member aliases and shadowing
        scopes.rs                 lexical bindings and cfg-aware fallbacks
        macros.rs                 macro opacity and format captures
      rust_syntax/
        mod.rs                    grammar-backed Rust syntax queries
        generated/                checked-in generated lexer/parser artifacts

    semantics/
      mod.rs                      facade
      model.rs                    policies, kinds, dispositions, template enums
      patterns.rs                 sem-pattern file parser and matching
      inventory.rs                collect/enforce coordinates and grammar options
      templates.rs                built-in action/predicate/rule-arg recognition
      hooks.rs                    typed hook mappings and adapter rendering
      semir.rs                    generated SemIR tables and member initializers
      manifest.rs                 semantics.json DTO rendering
      stack_member.rs             existing stack-member lowering
      template_syntax.rs          existing target-template parsing helpers

    lexer/
      mod.rs                      build/render facade
      render_model.rs             complete lexer emission input
      render.rs                   generated lexer module assembly

    parser/
      mod.rs                      staged parser-backend facade
      ir/
        mod.rs                    GeneratedParserRule and GeneratedParserStep
        lower.rs                  packed ParserAtn -> generated parser IR
        optimize.rs               future stage-local IR pass registry
      routing.rs                  generated/interpreted routing and call graphs
      decision.rs                 LL(1), fixed lookahead, report rows
      manifest.rs                 decisions.json rendering
      render_model.rs             final structured parser emission input
      render/
        mod.rs                    parser module assembly
        rules.rs                  rule and step rendering
        decisions.rs              decision/predicate rendering
        loops.rs                  star/plus/left-recursive loop rendering
        fallback.rs               interpreted fallback and error routing
      surface/
        mod.rs                    surface facade and shared model
        model.rs                  ParserSurfaceModel and context descriptors
        names.rs                  collision-free context/listener names
        accessors.rs              labels, cardinality, typed child accessors
        support_abi.rs            global API -> runtime support-contract bindings
        contexts.rs               context/view/validation type rendering
        traversal.rs              listener, visitor, and walkers
        facade.rs                 parser constructors and convenience APIs

    test_support.rs               cfg(test) ATN and fixture builders
```

This is not a requirement to create every empty file up front. Create a file
when its responsibility is extracted, and merge two proposed files if their
combined implementation remains cohesive and comfortably reviewable.

The `src/generated/` directory is a stage R target. Stage A may leave the
existing `src/generated.rs` file intact; do not mix that runtime file move with
the byte-preserving generator extraction.

Conversely, `parser/render.rs` or `surface.rs` must not become renamed
5,000-line replacements for the original file. As a review guideline:

- facade and `mod.rs` files should generally stay below 300 production lines;
- ordinary modules should generally stay below 1,200 production lines;
- algorithm-heavy modules may approach 1,800 lines when splitting would divide
  one algorithm;
- a codegen module above 2,000 production lines requires an explicit ownership
  justification or another split.

These are review triggers, not formatting targets. Checked-in generated
recognizers are exempt from line thresholds, but they must remain isolated,
clearly generated, and reproducible by a documented updater.

## 6. Ownership of current code

### 6.1 Process and common output

`cli.rs` owns:

- `Args`, `CliCommand`, `Args::parse`, `next_arg`, and `usage`;
- parsing from an injected iterator (`parse_from`) so tests do not mutate
  process state;
- validation of mutually exclusive and range-limited flags.

`driver.rs` owns:

- converting a validated CLI command to `CompilerConfig`;
- invoking `pipeline::run`;
- warning/pruning diagnostics;
- committing or reporting the returned artifacts.

It does not select concrete passes, push them into registries, classify
decisions, or encode report-only control flow.

`pipeline.rs` owns:

- the typed compiler/backend stage sequence;
- deriving stage-specific configuration from `CompilerConfig`;
- invoking grammar compilation, lexer/parser backend stages, and renderers;
- validation between stages;
- root lexer/parser deduplication;
- completing `CompilationReport` after in-memory artifacts exist.

`artifact.rs` owns:

- `GeneratedArtifacts`;
- normalized module path collision checks;
- stale `optimizations.json` removal;
- creating the output directory and committing files.

`rust_output.rs` owns:

- syntax-only generated module framing;
- token, rule, channel, mode, ATN, and metadata literal rendering;
- JSON string escaping only if it remains shared by both manifest writers;
- constant-name and array rendering, using `rust_names` for identifier rules.

It does not choose the generated-code API revision, runtime support path, or
context implementation strategy. `parser::surface::support_abi` owns those
decisions and supplies the compatibility-check fragment to module assembly.

The generated header still uses explicit generator build metadata rather than
implicitly assuming that `env!("CARGO_PKG_NAME")` is the runtime package. This
can be introduced while preserving current rendered bytes and prevents the
later crate move from causing accidental output churn.

### 6.2 Compiler optimization infrastructure

`optimization/` is neutral shared infrastructure. It owns configuration,
descriptors, report envelopes, and metrics, but no grammar/ATN/IR mutation.
Stage modules register implementations against it.

Split the current `grammar/transform.rs` by responsibility:

- `integration.rs` keeps mandatory import/combined-grammar integration;
- `transform::registry` executes optional grammar-model passes;
- `transform::analysis` owns stage-local cached facts and invalidation;
- `validation.rs` checks the editable model after every mutation;
- `transform::artifact` materializes optional transformed-source and source-map
  audit artifacts but returns values instead of writing files;
- each pass owns candidate discovery, proof, mutation, and evidence under
  `transform/passes/`;
- neutral reporting serializes pass-owned evidence after all downstream stages.

This keeps #129 factoring, #130 inlining, and #131 subsumption in separate files
and prevents each from adding fields to one precedence-ladder report struct.
`grammar::atn::optimize` remains mandatory ANTLR-compatible canonicalization;
do not route it through the opt-in grammar transform registry.

### 6.3 Structural projection

`structural/` owns the bridge from `SemanticGrammar`, finalized ATN graph, and
provenance into codegen-oriented coordinates:

- `StructuralAction`, `StructuralPredicate`, and `StructuralRuleCall`;
- state/source coordinate mapping;
- embedded rule alternatives and element references;
- token-set expansion;
- child cardinality and branch coexistence analysis.

It must consume ANTLR model metadata only. Language-specific helper names or
grammar identities belong in pattern files, typed hooks, docs, or language
tests.

### 6.4 Embedded and Rust compatibility

The current `src/bin_support/embedded.rs` should be split as part of this work.
Its production half contains at least five independent concerns:

- `embedded::model` owns attributes, element references, alternatives, rules,
  and child-cardinality facts;
- `embedded::members` owns `@members` item/field classification;
- `embedded::translate` owns ANTLR `$`-attribute resolution and body
  translation;
- `embedded::antlr4rust` owns target compatibility lowering, split further by
  aliases, lexical scopes, and macro handling;
- `embedded::rust_syntax` owns grammar-backed Rust syntax queries used by that
  compatibility lowerer.

The generated Rust lexer/parser under `rust_syntax/generated/` are build
artifacts, not hand-maintained modules. Keep them behind the hand-written query
facade and regenerate them only through `tools/rust-syntax/update-generated.sh`.
Target compatibility code may depend on generic embedded models, but generic
grammar, structural, semantic, and parser-IR modules must not depend on
antlr4rust-specific lowering.

### 6.5 Semantic handling

`semantics::model` owns:

- `SemUnknownPolicy`;
- coordinate and grammar-option kinds/dispositions;
- `ActionTemplate`, `PredicateTemplate`, and `RuleArgTemplate`;
- helper-call literals and typed-hook mapping DTOs.

`semantics::patterns` owns:

- `SemPatternFile` and its rule types;
- the small TOML-subset parser;
- exact-body/helper/coordinate matching;
- member inventory assignment.

`semantics::inventory` owns:

- lexer/parser coordinate collection;
- grammar-option collection;
- `--sem-unknown` and `--require-full-semantics` enforcement;
- source line/column attribution.

`semantics::templates` owns recognition of supported portable shapes. It must
remain grammar-agnostic: support is based on action/predicate structure or
declared pattern metadata, never a grammar name, language rule name, or source
file extension.

`semantics::hooks` and `semantics::semir` own code generation for the already
classified semantic model. They must not reclassify source text.

`semantics::manifest` serializes the inventory it is given. It must not inspect
the grammar or ATN.

### 6.6 Lexer generation

`lexer::render_model` owns all pre-render decisions:

- compiled or supplied lexer DFA words;
- structural and embedded lexer actions/predicates;
- unknown-policy behavior;
- typed lifecycle/semantic hooks;
- member-state initialization;
- lexer superclass/options surface.

`lexer::render` owns source assembly only. The internal facade should be
close to:

```rust
fn build_lexer_render_model(
    input: LexerCodegenData<'_>,
    options: &LexerOptions,
) -> io::Result<LexerRenderModel<'_>>;

fn render_lexer(model: &LexerRenderModel<'_>) -> String;
```

### 6.7 Parser lowering and decisions

`parser::ir` owns the generated recursive-descent representation and no
rendering strings:

- `GeneratedParserRule`;
- `GeneratedParserStep`;
- `GeneratedRuleCallPrecedence`;
- fast-path/complete-LL(1) arm DTOs that lowering produces.

`parser::lower` owns:

- ATN state/path compilation;
- block, star, plus, and left-recursive lowering;
- transition-to-step conversion;
- FIRST-set helpers used specifically to construct IR fast paths;
- rejecting unsupported generated shapes by returning `None`.

`parser::ir::optimize` owns optional transformations over
`LoweredParserIr`. It validates every changed result and returns
`OptimizedParserIr`; it cannot inspect CLI state or rendered source. The module
may initially contain only the stage facade and validator rather than an empty
generic pass framework.

`parser::decision` owns:

- tool-parity LL(1) classification;
- fixed-lookahead walks, budgets, rectangles, and tries;
- sync-no-op restriction;
- `DecisionClassification` and `DecisionReportRow`.

Do not duplicate FIRST-set logic across `lower` and `decision`. If both need
the same implementation, extract a private `parser::look` module with a typed
API rather than importing renderer internals.

`parser::routing` owns:

- generated-rule enablement;
- rules that must use the interpreted ATN;
- caller/reachability graphs;
- adaptive-ATN preference heuristics and indexed slots;
- generated-only validation.

Routing consumes validated IR plus decision analysis and returns `RoutingPlan`.
Building `ParserRenderModel` is a separate final join with semantics and the
already-built `ParserSurfaceModel`. This separates five frequently changed
concerns: generated API shape, what can be lowered, how IR may be optimized,
how a decision predicts, and which engine a call uses.

### 6.8 Parser rendering and typed surfaces

`parser::render` consumes `ParserRenderModel`. It owns emitted control flow but
does not decide classifier tiers, optimize IR, select engines, or recompute
graph reachability. Its rule/decision/loop renderers do not receive
`ParserSurfaceModel`. Render-model passes may alter only observationally inert
layout; they never receive generated Rust text as optimization input.

`parser::surface::model` owns the grammar-specific user-facing API shape:

- context type and callback name allocation;
- alternative dispatch;
- label and child accessor selection;
- attribute presence and fields;
- validation requirements;
- listener/visitor callback and dispatch descriptors;
- enabled optional surfaces.

The surface model may depend on `embedded::model`, but the embedded body
translator must not depend on surface renderers. Shared cardinality and
element-reference types should live in `embedded::model` or a neutral
`surface::model`, not be copied.

Surface renderers consume only `ParserSurfaceModel` plus
`GeneratedSupportBindings`. In stage A those bindings select the current inline
implementation so output remains byte-for-byte stable. In stage R they select
global codegen revision `N+1` and the `context_v1` runtime support contract.
Neither mode may inspect parser IR, routing, or decision analysis.

`parser::surface::support_abi` owns:

- the emitted `__antlr4_rust_require_codegen_api!` revision;
- the explicit mapping from each emitted global codegen revision to its
  independently versioned, fully-qualified runtime support paths;
- thin per-context type and trait bindings;
- the choice between inline stage-A mechanics and revisioned stage-R support;
- compile-time capability checks needed by generated source.

It does not own generic Rust literal formatting, public surface naming,
recognition control flow, or runtime implementation. Keep per-context
declarations separate from once-per-recognizer validation, traversal, and
facade glue so either duplication layer can be evaluated independently.

### 6.9 Runtime generated support

Stage R extends the runtime's existing generated-recognizer boundary in
`src/generated.rs` and reuses the flat-tree operations in `src/tree.rs`.
Recognizer-independent implementations live under
`generated::support::context_v1`; generated grammar data remains in each
recognizer module.

The runtime support module may own the non-generic stored/active context core,
named iterator implementations, invocation-state operations, and neutral child
scans/events proven by R0.5. It operates on runtime-owned token/tree storage and
scalar indexes, but never codegen AST/IR types, grammar-specific descriptors, or
names. Public generated context newtypes, validated types/errors,
`__RuleAttrsN` types and traits, typed accessors, callback traits and dispatch,
recursive-descent methods, predicates, and semantic hooks remain generated.

Do not create a third published "generated support" crate. Every generated
recognizer already depends on `antlr4_runtime`; another package would add
version coordination without creating a useful dependency direction. Likewise,
do not move hot generic parser/lexer wrappers merely to shorten source:
specialization, inlining, feature isolation, and monomorphization effects must
be measured before changing their ownership.

## 7. Test layout

### 7.1 Unit tests

Move tests with each implementation in the same extraction commit. The current
test block naturally separates into:

| Current baseline lines | Destination |
| --- | --- |
| 16955-17195 | common output/model tests |
| 17196-18435 | parser IR, lowering, routing, and generated-rule rendering |
| 18436-19598 | embedded compatibility and typed-surface tests |
| 19599-20562 | parser staged-artifact/render/semantic-decision tests |
| 20563-20860 | semantic template and lexer semantic tests |
| 20861-21517 | `codegen::test_support` ATN and fixture builders |
| 21518-21740 | decision classifier/fixed-lookahead tests |
| 21741-23132 | pattern, policy, hook, manifest, and lexer-policy tests |

Every test module using Insta keeps:

```rust
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
```

Snapshot module paths will change when tests move. Rename snapshot files with
the tests, run `cargo insta test`, and review that snapshot bodies are
unchanged. Do not accept a mass snapshot content rewrite as expected refactor
noise.

Move the 57 tests currently in `embedded.rs` with their model, members,
translation, antlr4rust, and Rust-syntax owners. Do not leave a large facade
test module that recreates the same conflict surface after production code
moves.

`codegen::test_support` should provide:

- minimal parser/lexer fixture compilation;
- reusable ATN builders;
- compact constructors for generated steps/rules;
- repository fixture path resolution.

It must be `#[cfg(test)]` and must not become a backdoor production dependency.

### 7.2 CLI integration tests

Keep one integration-test target so Cargo does not compile and link the large
test harness repeatedly, but split its source:

```text
tests/antlr4_rust_gen_cli.rs
tests/antlr4_rust_gen_cli/
  support.rs
  cli.rs
  diagnostics.rs
  transforms.rs
  optimizations.rs
  lexer.rs
  parser.rs
  semantics.rs
  typed_tree.rs
  compatibility.rs
  multi_recognizer.rs
```

The root file should contain module declarations only. Shared temporary-crate,
generator invocation, normalization, and generated-project helpers belong in
`support.rs`.

### 7.3 Behavioral equivalence fixture

Before the first extraction, define a deterministic representative generation
set containing:

- lexer-only, parser-only, split, and combined grammars;
- embedded and template action modes;
- semantic patterns and typed hooks;
- listener/visitor combinations;
- LL(1), adaptive, and fixed-lookahead decisions;
- context labels, validated trees, and left recursion;
- optimization report-only and pruning runs.

Generate the set before and after each extraction and recursively compare:

- every generated `.rs` module;
- `semantics.json`;
- `decisions.json`;
- `optimizations.json` when present;
- stdout, stderr, and exit status for failing cases.

Existing unit and CLI tests remain authoritative. This comparison is an
additional refactor gate that catches accidental changes to unasserted
whitespace, ordering, or generated comments.

### 7.4 Optimization contract tests

Before adding #129-#131 or another backend optimization, the owning stage must
have reusable contract tests for:

- deterministic output and report ordering;
- deterministic transformed-source/source-map audit output and proof that
  authored inputs are never overwritten;
- idempotence of each pass;
- validation after every changed pass;
- canonical ordering, prerequisites, conflicts, and pairwise pass interaction;
- analysis invalidation, including cached-versus-clean recomputation checks;
- provenance/tombstones from authored nodes through ATN and parser IR;
- apply versus report-only agreement on projected downstream metrics;
- explicit declined candidates and stable reasons;
- valid and invalid differential parsing, consumed input, diagnostics, and
  recovery;
- parse-tree, typed-context, listener, visitor, and public-rule checks required
  by the declared safety class;
- same-machine interleaved A/B benchmarks over multiple protected grammar
  families, including generated bytes, packed ATN size, decision tiers, parse
  time, and peak memory.

These tests are pass/stage tests, not conditions embedded in generic rendering
tests. A shorter grammar or generated module is not sufficient evidence of an
optimization win.

### 7.5 Multi-recognizer and generated-support contracts

Before stage R, add two fixture layers. The controlled scaling matrix contains
one fixed lexer and zero, one, two, and three renamed copies of the same
medium-sized parser, generated from identical grammar input and options. Run
every count in both layouts:

- all generated modules in one consumer crate;
- one crate per generated parser, converging in one final binary.

The zero-parser case isolates fixed runtime, lexer, and binary overhead.
Report both absolute values and the marginal delta from `k` to `k+1`; changing
the grammar at each count would confound duplication with grammar complexity.
Each included parser must parse an input and exercise a typed context so
dead-code elimination cannot erase it.

The external-validity layer contains this repository's two-parser/one-lexer
dogfood shape and a Mehen fixture with its independent Java and Kotlin parser
crates plus the third parser when available. Record parser, lexer, generated
module, context, and crate counts explicitly; do not call a parser/lexer pair
one "recognizer." At least one fixture must also exercise recovered and
validated trees, direct terminals, attributes, labels, listener callbacks,
visitor dispatch, and invocation-state display.

The compatibility matrix must include:

- checked-in source for every previously accepted global revision compiling and
  running on the new runtime;
- global revision `N+1` source using `context_v1` and running on the new runtime;
- revision `N+1` source against a runtime that accepts only through `N` failing
  with the intended generated-code API diagnostic;
- one older accepted revision and revision `N+1` linked and executed in the same
  binary;
- every literal accepted arm in
  `__antlr4_rust_require_codegen_api!`, not only the current revision and a
  synthetic unsupported number.

Retain an older global revision only while the runtime still provides every API
its generated source needs. A `#[doc(hidden)]` support module is not exempt from
this contract. `context_v1` remains compatible for as long as any accepted
global revision maps to it; an incompatible context-support change uses
`context_v2` and a new global revision rather than silently changing
`context_v1`.

Generated public-API compatibility needs more than text snapshots. For plain
rules, labeled alternatives, labels, attributes, recovered/active/stored/
validated contexts, listeners, and visitors:

- compile downstream consumer snippets that name the generated nominal types
  and traits, including empty and non-empty `__RuleAttrsN` APIs;
- compare rustdoc or another public-API inventory before and after R2, allowing
  only the new hidden support dependency and explicitly approved changes;
- compile assertions for observable trait implementations, associated types,
  conversions, lifetimes, and auto traits;
- exercise the supported generator flag combinations independently, including
  listener/visitor-disabled and lexer-only output, so the runtime binding does
  not collapse distinct feature surfaces;
- compile representative same-crate and separate-crate consumers, not only the
  generated modules in isolation.

Stage R records four independent measurement families:

1. **Generated source:** lines and bytes per module and in aggregate, including
   attrs, common context blocks, and once-per-recognizer glue. This is the only
   result a macro-only extraction may claim without further evidence.
2. **Compile cost:** clean `cargo check` and release build wall/user time and
   peak RSS for every controlled count and layout. Use dedicated target
   directories, `CARGO_INCREMENTAL=0`, fixed linker/job-count/profile settings,
   and repeated runs with medians. Run compatibility and clean-build gates on
   the declared MSRV, Rust 1.95, as well as the normal CI toolchain. Also
   measure an incremental edit to one generated crate in the separate-crate
   layout and the Mehen fixture.
3. **Linked artifact:** stripped executable bytes plus `.text`, `.rodata`, and
   writable-data sections, with a linker map or symbol attribution showing
   whether common mechanics have one implementation. Report every controlled
   count with LTO disabled and with the project's normal LTO setting, including
   the marginal size of each added parser. Generic helpers and exported macros
   count as shared only when the linked symbols demonstrate it.
4. **Runtime behavior:** first-parse latency, steady-state throughput, and peak
   memory per grammar, plus focused context accessor, listener, visitor, and
   validation workloads. Predeclare regression tolerances and use
   same-machine interleaved A/B runs.

Do not combine these into one percentage or infer compile-time, binary-size, or
runtime improvement from generated line count. Keep the raw commands, profiles,
toolchain, linker, fixture revisions, and samples in the issue/PR evidence.

## 8. Migration sequence

Each PR labeled stage A is behavior-preserving. No feature work or opportunistic
cleanup belongs in those PRs. New internal report structures may collect more
facts, but compatibility writers preserve the active generated files and
manifest bytes. All stage-A steps compare with the active repository baseline.
If the stage-R binding change exceptionally lands before A11, record its new
`N+1` output baseline once and make the remaining stage-A steps byte-preserving
against it.

### A0. Establish the equivalence gate

1. Record baseline command/output hashes for the representative generation
   set.
2. Add any missing deterministic integration assertion needed to reproduce it.
3. Confirm the current unit, integration, conformance, and parity baselines.
4. Document that `__ANTLR4_RUST_CODEGEN_API` remains unchanged.

### A1. Establish a mechanical process shell

1. Add `codegen::{cli,driver,artifact,rust_output}`.
2. Change CLI parsing to `parse_from` while retaining an `env::args` adapter.
3. Return an artifact set before writing it.
4. Reduce `src/bin/antlr4-rust-gen.rs` to module wiring and process exit.
5. Move common-output and CLI tests.

This gives later modules a stable facade and removes filesystem/process state
from rendering tests.

### A2. Establish optimization configuration, pipeline, and reporting contracts

1. Add `CompilerConfig` and `codegen::pipeline` without changing stage order.
2. Introduce canonical pass descriptors, `OptimizationConfig`, and central
   selection/order validation.
3. Map existing dedicated flags into that configuration; add no new CLI
   syntax in this PR.
4. Introduce the neutral `CompilationReport` envelope and adapt existing
   deterministic manifest writers without changing their bytes.
5. Make the driver perform only `config -> pipeline -> artifact commit`.
6. Add configuration-order, conflict, and report-envelope unit tests.

This creates one place for pass policy without creating a universal mutation
trait. Stage-specific implementations remain in their owning modules.

### A3. Split grammar integration, transformation, and reporting

1. Move mandatory import and combined-grammar integration out of
   `grammar/transform.rs`.
2. Move editable-model validation and transform-stage analysis into dedicated
   modules.
3. Put each existing optional grammar pass in its own pass-owned file.
4. Replace the shared pass-specific candidate DTO with a neutral envelope and
   private pass evidence while preserving `optimizations.json`.
5. Key transform analyses by model revision, conservatively invalidate them
   after mutation, and validate after every changed pass.
6. Return one typed result that binds the transformed model, provenance/source
   map, report events, and optional audit artifacts.
7. Keep mandatory ATN canonicalization outside the optional transform
   registry.

This prevents `grammar/transform.rs` from becoming the next merge hotspot and
gives future issues `#129`-`#131` separate implementation ownership.

### A4. Extract semantic domain and policy

1. Move semantic enums and DTOs first.
2. Move pattern parsing and stack-member/template helpers under `semantics/`.
3. Move inventory/enforcement and `semantics.json` rendering.
4. Move hook and SemIR rendering without changing classification behavior.
5. Move the corresponding tests and snapshots.

Moving domain types first avoids circular imports caused by leaving
`PredicateTemplate` below parser rendering.

### A5. Introduce typed inputs and structural projection

1. Move `CodegenData` unchanged to `codegen::model`.
2. Extract structural coordinate and context/cardinality logic.
3. Split `embedded.rs` into model, members, `$` translation, and antlr4rust
   compatibility modules.
4. Keep Rust-syntax analysis behind its own facade and its generated
   recognizers in an isolated generated directory.
5. Replace `CodegenData` with `LexerCodegenData` and `ParserCodegenData`.
6. Keep allocation and diagnostic order unchanged.

The typed input conversion is the first intentional internal redesign and
should be its own commit after the moves compile.

### A6. Extract the parser surface model

1. Move context and callback naming/allocation first.
2. Move accessor selection, cardinality, attribute-presence, and validation
   descriptors into `ParserSurfaceModel`.
3. Separate per-context declarations from once-per-recognizer validation,
   traversal, and facade glue.
4. Add `GeneratedSupportBindings` and `surface::support_abi`, selecting the
   current global revision's inline implementation without changing one output
   byte.
5. Make surface rendering consume only the surface model and bindings, not
   parser IR, decision analysis, or routing.
6. Move context, validation, listener, visitor, walker, and facade tests and
   snapshots.

This is the earliest prerequisite for stage R. It intentionally models the
runtime-extraction boundary before changing generated source.

### R0. Establish the multi-recognizer baseline

1. Add the controlled zero/one/two/three-parser matrix in same-crate and
   separate-crate layouts, followed by the repository and Mehen fixtures from
   section 7.5.
2. Record source, compile, linked-artifact, and runtime measurements with exact
   commands and profiles.
3. Inventory every surface item as runtime mechanics, grammar-specific data/API,
   or generated specialization.
4. Freeze public generated API snapshots for plain rules, labeled alternatives,
   labels, attrs, active predicates, recovered trees, validated trees,
   listeners, and visitors.

### R0.5. Run a disposable context-support ABI spike

1. Prototype at least a non-generic/type-erased core and the best generic or
   macro-based alternative against the same medium parser.
2. Exercise stored and active contexts, named child/terminal iterators,
   invocation-state operations, and neutral scan/event callbacks without moving
   grammar-branded validation or callback dispatch into the runtime.
3. Compile the zero/one/two/three controlled matrix and inspect generated
   source, compile RSS/time, monomorphized symbols, linked sections, and focused
   runtime performance with and without normal LTO.
4. Compile consumer probes for nominal context types, all `__RuleAttrsN` APIs,
   trait bounds, auto traits, listeners, visitors, and supported flag
   combinations.
5. Write an ABI decision record naming the operations that earned runtime
   ownership and those that remain generated.
6. Discard the spike implementation. Do not publish its module path or treat
   its API as compatibility-constrained input to R1.

R0 and R0.5 may proceed on a focused branch while A7-A11 continue. Their
measurements inform R1, but they do not block unrelated extraction work.

### R1. Add the revisioned runtime context-support contract

1. Introduce
   `antlr4_runtime::generated::support::context_v1` without a root re-export.
2. Implement only the non-generic stored/active context operations, named
   iterators, invocation-state operations, and neutral scan/event primitives
   accepted by the R0.5 decision record.
3. Keep common machine-code candidates non-generic or type-erased where
   practical; use exported macros only for thin declarations and impl plumbing.
4. Add direct runtime unit tests and `context_v1` support compile fixtures.
5. Keep all previously accepted global revisions because this step removes none
   of their runtime surfaces.

### R2. Bind generated surfaces to `context_v1`

1. Capture the then-current global codegen revision as `N`; make
   `surface::support_abi` map new global revision `N+1` to `context_v1`.
2. Preserve every public generated nominal type, name, signature, trait
   implementation, and callback surface. Delegate mechanically identical
   per-context bodies to runtime operations and remove redundant empty-attrs
   storage/lookups without removing public `__RuleAttrsN` APIs.
3. Set `__ANTLR4_RUST_CODEGEN_API` to `N+1` and update accepted arms,
   diagnostics, compatibility docs, integration snapshots, and checked-in
   recognizers. Do not assume `N+1` is the number `2`.
4. Add older-on-new, `N+1`-on-`N`, and mixed-old/`N+1`-in-one-binary tests,
   plus the public-API and flag matrix from section 7.5.
5. Regenerate all checked-in recognizers and run the full parity/conformance
   matrix.
6. Repeat all section 7.5 measurements and publish deltas independently.

R1 and R2 are separate design checkpoints but should normally ship in one
release/PR series so no published generator emits source before its runtime
support exists. This stage is source compaction and ownership cleanup, not a
public generated API redesign.

### R3. Evaluate once-per-recognizer glue

Review ATN/DFA cache setup, terminal/error wrappers, validation infrastructure,
traversal bridges, parser/lexer facade delegation, and generic adapters one
family at a time. Do not broaden `context_v1` merely because another family is
repetitive: a new runtime-owned family needs its own disposable design spike,
compatibility decision, and multi-recognizer evidence. Grammar-branded
validation/errors, callback dispatch, grammar data, and hot generic code remain
generated. This step may conclude that some source duplication is the correct
tradeoff.

The recommended landing order is A0-A6, then two parallel tracks: A7-A11 on the
main decomposition track and R0-R0.5 on the issue-276 research track. Prefer
landing R1-R2 after A11 so the supported ABI change does not disrupt mechanical
moves; do it earlier only when the active branches coordinate one explicit
rebaseline. R3 is not a workspace prerequisite.

### A7. Extract the lexer render model and renderer

1. Move lexer-only classification and rendering helpers.
2. Build a validated `LexerRenderModel` before source assembly.
3. Make final lexer rendering consume only that model.
4. Move lexer tests and snapshots.

At this point lexer feature work no longer touches parser files.

### A8. Extract parser IR and lowering

1. Move parser IR DTOs without behavior changes.
2. Move ATN-to-IR lowering and its validation.
3. Return `LoweredParserIr` with stable source/ATN provenance.
4. Move focused tests and shared builders.
5. Preserve unsupported-shape fallback and environment-controlled paths
   exactly.

Do not move routing into lowering merely because both currently feed the same
renderer.

### A9. Extract parser decision analysis

1. Move LL(1) and fixed-lookahead analysis as one cohesive algorithm.
2. Return `ParserDecisionAnalysis` and move decision report DTOs.
3. Move `decisions.json` rendering and fixed-lookahead snapshots.
4. Verify classifier output and generated source byte-for-byte.

This module should be independently changeable by parser-performance work.

### A10. Establish staged parser routing and rendering

1. Add the validated `OptimizedParserIr` stage as an identity boundary; do not
   invent a generic pass trait before a real IR pass exists.
2. Extract call-graph analysis and generated/interpreted selection into
   `RoutingPlan`.
3. Join optimized IR, decision analysis, routing, semantics, and the independent
   `ParserSurfaceModel` into `ParserRenderModel`.
4. Split rule, decision, loop, fallback, and top-level module rendering.
5. Remove all analysis and pass selection from render functions.
6. Finalize `CompilationReport` only after downstream models and artifacts
   exist.
7. Expose the non-committing downstream-analysis entry point needed by O0, but
   leave the current report-only early return, diagnostics, exit status,
   manifest bytes, and generation work unchanged.
8. Preserve rendered fragment order and existing manifest bytes exactly.

This is the milestone that makes later parser-IR, decision, routing, and
render-layout optimization work independently reviewable.

### A11. Split integration tests and enforce boundaries

1. Split the CLI integration source while retaining one test target.
2. Remove obsolete re-exports introduced during extraction.
3. Narrow `pub(crate)` items to `pub(super)` or private.
4. Add a dependency-boundary note to `codegen/mod.rs`.
5. Run the complete validation matrix.

### O0. Deepen report-only optimization evidence

This is an intentional command-behavior and performance change in its own PR
after A11, not part of the mechanical stage-A extraction:

1. Record current report-only manifest bytes, stdout/stderr, exit status, wall
   time, and peak RSS for successful and failing representative grammars.
2. Route the transformed shadow through semantic, ATN, decision, IR, routing,
   and render-model analysis without committing recognizer artifacts.
3. Keep unrelated generation policies non-enforcing in report-only mode.
   Invalid transformed candidates become deterministic declined/error evidence;
   any new top-level diagnostic or failure requires an explicit documented
   contract decision and fixture.
4. Add projected downstream metrics to the report compatibility model and
   deliberately accept the resulting `optimizations.json` snapshot changes.
5. Rebaseline report-only timing and memory separately; normal generation
   remains subject to the existing performance-equivalence gate.
6. Verify apply/report-only agreement for downstream metrics and run the full
   optimization contract suite.

### 8.1 Landing and parallel-work strategy

The extraction series will itself conflict with active changes to the giant
file. Treat A1-A11 as a short mechanical campaign while R0-R0.5 gathers evidence
on a focused branch:

1. Start from a freshly fetched `origin/main`.
2. Land existing feature PRs that deeply modify the generator before A1 when
   practical.
3. Keep extraction PRs mechanical and merge them in order without long gaps.
   R2 is the one intentional output/API transition and gets its own review,
   preferably after A11.
4. Ask remaining branches to rebase after the module containing their work has
   landed.
5. After A3, grammar pass implementations can proceed in separate pass-owned
   files without editing integration or reporting.
6. After A6, R0-R0.5 can proceed without touching parser IR, decisions, or
   routing; typed-surface work has its own files.
7. After A7, grammar optimization, lexer, semantics, embedded actions, and typed
   surfaces can proceed mostly independently.
8. After A10, parser lowering, decisions, routing, rendering, and typed surfaces
   have separate ownership. A11 removes the remaining test hotspot.

Do not run repository-wide formatting in extraction commits. Format only moved
or edited files so code movement remains reviewable.

## 9. Workspace evaluation

### 9.1 What a workspace would improve

The generator has crossed the threshold from an incidental binary to a
separate compiler product:

- it has more than 50,000 lines when the grammar compiler and embedded support
  are included;
- it owns ICU, `intl`, and graph-analysis dependencies that runtime-only users
  do not need;
- it has a large unit/integration/conformance test surface;
- it has an independent installation workflow and CLI;
- it consumes the runtime but the runtime library must not consume it.

The direct-compiler plan in `docs/issue-141-direct-g4-codegen-plan.md`
deliberately deferred a workspace while the Rust-owned frontend was being
established. That was the right initial constraint. This plan supersedes only
that packaging deferral: the frontend now exists, the generator has grown from
the 16,306-line baseline recorded there to 23,132 lines, and its internal
module boundaries can be established before they become package boundaries.

A separate package would provide:

- runtime packages and docs that do not contain compiler implementation/tests;
- no runtime `codegen` feature whose purpose is only to enable a binary;
- focused `cargo test -p ...` and `cargo clippy -p ...` commands;
- a compile-time package boundary preventing codegen concerns from leaking into
  the runtime;
- clearer dependency ownership and faster runtime-only CI/local checks;
- room for a small programmatic codegen API later, if a real consumer appears.

The boundary remains one-way even after issue #276: codegen selects and emits
bindings to the runtime's generated-support ABI; the runtime never imports
surface models or renderer code.

### 9.2 Costs

The current single package provides useful coupling:

- `cargo install antlr-rust-runtime --features codegen --bin antlr4-rust-gen`
  installs the generator;
- runtime and generator always have one package version;
- generated headers use current package metadata;
- release-please and the publish workflow publish one crate;
- scripts and evidence maps select one package and one `codegen` feature;
- many tests use `env!("CARGO_MANIFEST_DIR")` as the repository root;
- conformance coverage assumes the harness and generator are targets of the
  same package.

A workspace conversion must intentionally update all of those contracts. It
also creates partial-release risk: the runtime can publish successfully before
the generator publication fails. Stage R already requires coordinated
runtime/generator compatibility, so the workspace must preserve that contract
rather than replace it with package-version equality.

### 9.3 Alternatives considered

**Remain one package permanently.** This is viable after the module split and
has the simplest release UX. It does not provide a package dependency boundary,
and the runtime crate continues to ship compiler source and codegen-only
feature/dependency metadata.

**Split every compiler phase into a crate.** Reject. It would force unstable
grammar/ATN/codegen models into public cross-crate APIs and make routine
compiler changes version several crates.

**Create the workspace before splitting the file.** Reject. It changes paths,
features, release automation, and package metadata while leaving the actual
merge hotspot intact.

**Keep a compatibility generator binary in the runtime package.** Reject as a
long-term design. The codegen crate must depend on the runtime, so making the
runtime package depend back on codegen creates a package cycle. Duplicating the
compiler to preserve the old install command is worse.

**Create a third generated-support crate.** Reject. Generated recognizers
already depend on the runtime, and support operates on runtime-owned tree,
parser, and token types. A third package adds version coordination and either
duplicates those types or depends back on the runtime without improving
ownership.

### 9.4 Recommendation

Adopt a workspace after A11 and R2, with two published packages and one optional
non-published tool package:

```text
Cargo.toml                         antlr-rust-runtime package + workspace root
src/                               runtime library and generated/support/context_v1

crates/antlr-rust-codegen/
  Cargo.toml                       published, same version as runtime
  src/bin/antlr4-rust-gen.rs
  src/optimization/
  src/codegen/
  src/grammar/
  tests/

tools/antlr4-runtime-testsuite/
  Cargo.toml                       publish = false
  src/
```

Keep the existing runtime package at the repository root. Do not move all
runtime source into `crates/` merely for visual symmetry; that creates path
churn for no ownership benefit. The revisioned generated-support ABI stays in
this package.

The non-published testsuite package is optional. It is appropriate if moving
the harness lets the runtime package contain only runtime targets. It must not
block extracting the published codegen package.

Do not create a separately published grammar-frontend crate. Keep `grammar/`
private to `antlr-rust-codegen` until an actual second consumer can justify a
stable API. Keep optimization descriptors, stage artifacts, pass registries,
and report evidence crate-private as well; a package boundary does not make
them public extension points.

## 10. Workspace migration plan

Stage B begins only after A11 and the issue-276 R2 transition are merged and
green. R3 may continue afterward because broader once-per-recognizer extraction
is evidence-driven rather than a packaging prerequisite.

### B1. Prepare workspace-safe paths and metadata

1. Add one test helper that resolves the workspace root.
2. Replace scattered codegen-test `CARGO_MANIFEST_DIR` assumptions with that
   helper; do not add repeated `../..` joins.
3. Make generator name/version/repository metadata explicit inputs to generated
   headers.
4. Update scripts to select targets by package and binary, even while both are
   still the root package.

### B2. Move the codegen package

1. Add a non-virtual workspace to the root manifest.
2. Move `optimization/`, `codegen/`, `grammar/`, embedded/Rust-syntax support,
   and generator tests into `crates/antlr-rust-codegen`.
3. Give the codegen package a dependency on the root runtime package.
4. Keep `generated::support::context_v1` and its compatibility tests in the
   runtime; do not move it with codegen's
   `parser::surface::support_abi`.
5. Move ICU, `intl`, and codegen graph dependencies out of runtime features and
   into the codegen package.
6. Remove the runtime package's `codegen` feature after all callers migrate.
7. Keep the binary name `antlr4-rust-gen`.
8. Give both published packages explicit package-content rules. Verify with
   `cargo package --list` that the runtime archive excludes compiler fixtures
   and that the codegen archive includes generated grammar frontends,
   Rust-syntax recognizers, and binary data such as
   `unicode_decomposition.bin`.

Initially make the codegen package binary-oriented. Do not promise a broad
public Rust compiler API merely because Cargo supports a library target.

### B3. Preserve the compatibility contract

Use the same release version for both packages. The published codegen package
should depend on the exact matching runtime version initially:

```toml
antlr-rust-runtime = { version = "=0.25.0", path = "../.." }
```

This can be relaxed later only with evidence that independently versioned
generator builds are useful. The exact dependency is a release/build policy:
it ensures this generator was compiled against the runtime support it emits.
The generated-code API macro is the separate source-compatibility contract for
recognizers that may outlive their generator package or coexist with older
generated modules.

After R2, the runtime continues accepting each older global revision and `N+1`
only while it implements every surface each revision requires. Workspace tests
must keep the mixed-revision binary and compile every accepted-revision fixture
against the packaged runtime, not only against the local source tree. The
global-revision fixtures also verify their expected support-contract mapping;
the workspace move must not infer that mapping from package versions.

The crate move must not modify `__ANTLR4_RUST_CODEGEN_API`. If generated output
changes only because package metadata changed, either preserve the old bytes
through explicit build metadata or isolate and review the header-only change.

### B4. Update commands and automation

Repository commands become explicit:

```bash
cargo run -p antlr-rust-codegen --bin antlr4-rust-gen -- ...
cargo test -p antlr-rust-codegen
cargo test -p antlr-rust-runtime
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The user install command becomes:

```bash
cargo install antlr-rust-codegen --bin antlr4-rust-gen
```

Update, in one migration PR:

- README and language-build docs;
- `AGENTS.md` and `CLAUDE.md`;
- parity and benchmark scripts;
- grammar-frontend updater and generated evidence commands;
- runtime-testsuite prebuild target discovery;
- CI and LLVM coverage object discovery;
- release-please configuration;
- publish workflow and dry runs;
- any package-name normalization in snapshots.

If generated evidence JSON stores executable commands, regenerate it with the
new package selectors instead of leaving stale instructions.

### B5. Publish in dependency order

The release job must:

1. verify both package versions equal the release tag;
2. fully test the workspace and build codegen against the exact local runtime;
3. package, inspect, and dry-run `antlr-rust-runtime`;
4. publish `antlr-rust-runtime`;
5. wait until that exact version is resolvable from Cargo/crates.io;
6. package and inspect `antlr-rust-codegen`, then run its full registry-backed
   dry run;
7. publish `antlr-rust-codegen`;
8. be safely rerunnable when the runtime is already published.

Both codegen packaging and its dry run must wait for the runtime. Cargo
normalizes the codegen manifest by removing the runtime dependency's local
`path` and resolves the exact registry version while preparing the archive;
`--no-verify` skips building archive contents but does not bypass dependency
resolution. CI and the pre-publish workspace build still verify the codegen
crate against the exact local runtime before either publication.

Use one release tag and one changelog. Independent release trains would conflict
with the project's recommended matching-version workflow and add no current
value.

### B6. Decide testsuite placement

If the harness moves to `tools/antlr4-runtime-testsuite`:

- mark it `publish = false`;
- make its runtime/codegen package selection explicit;
- keep its subprocess smoke crates pointed at the workspace runtime path;
- update coverage to include generator, harness, and stripe binaries;
- avoid creating a shared "misc utilities" crate solely for Rust name helpers.

A small stable naming helper may be exposed by codegen if it is genuinely part
of artifact naming. Otherwise keep harness-specific naming local and test it
against generated module filenames.

## 11. Validation gates

Run focused checks after every extraction:

```bash
cargo fmt --check
cargo test --locked --features codegen --bin antlr4-rust-gen
cargo test --locked --features codegen --test antlr4_rust_gen_cli
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Run the behavioral-equivalence generation comparison on every stage A PR
against the active global-revision baseline before and after R2.

The fresh-checkout instructions in `AGENTS.md` initially sparse-check out only
the Kotlin grammar. Before running the complete parity matrix below, expand that
same pinned checkout:

```bash
git -C /tmp/antlr-cleanroom/grammars-v4 sparse-checkout set \
  kotlin/kotlin javascript/javascript javascript/typescript
```

A full checkout at the documented pinned commit is also valid. Do not present
all three parity commands as one reproducible gate while only the Kotlin path is
materialized.

Run these after A3, A5, A6, R2, A7, A10, A11, O0, and after the workspace move:

```bash
cargo llvm-cov --locked --all-features --workspace --lcov --output-path lcov.info
cargo run --release --quiet --bin antlr4-runtime-testsuite
tests/kotlin-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
tests/javascript-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
tests/typescript-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
```

After B2, use the corresponding `-p` selectors. Validate workflow edits with
`actionlint`.

Performance-sensitive parser/lexer phases should also run the existing parity
and parse benchmarks before and after their extraction. Module movement should
not alter generated code, so any measurable generated-parser or generation-time
change in a stage-A-only PR is a regression to investigate, not expected noise.

O0 has a separate report-only baseline. Its review must show old and new
manifest bytes, diagnostics, exit status, wall time, and peak RSS, and must
confirm that normal generation remains unchanged.

R0 records the section 7.5 controlled and external-validity baselines. R0.5
records the disposable ABI alternatives and symbol evidence, passes public-API
consumer probes on Rust 1.95, and ends with a written keep/discard decision for
each candidate operation. R2 must run the complete compatibility and supported
flag matrices, controlled and real-consumer builds, linked-section/symbol
analysis, focused typed-context workloads, parse benchmarks, conformance sweep,
all parity suites, coverage, and CI-parity clippy. Review source, compile,
linked-size, and runtime results as separate gates.

## 12. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Broad `pub(crate)` surface recreates global coupling | default private/`pub(super)`; audit exports in A11 |
| Circular conceptual dependencies | move neutral DTOs to `model`; enforce one-way module graph |
| Mechanical moves change output order | byte comparison and existing snapshots on every PR |
| Insta paths create a misleading mass diff | rename snapshot files, verify bodies unchanged |
| One universal pass trait erases stage legality | use distinct grammar, ATN, decision, IR, routing, and render-model contracts |
| Pass booleans and ordering recreate a driver hotspot | canonical descriptors/configuration own selection; the driver only invokes the pipeline |
| A shared report DTO accumulates pass-specific fields | neutral event envelope plus private pass-owned evidence |
| A pass reads stale analysis after mutation | revision-key caches, declared preservation, conservative invalidation, and clean-recompute tests |
| A transformed grammar is paired with stale analysis or ATN data | one typed grammar-pass result owns the transformed model, provenance, report, and audit artifacts |
| Parser analysis is accidentally recomputed by rendering | staged artifacts own analysis results; renderers accept only render models |
| `ParserRenderModel` recouples surface/runtime work to IR and routing | build `ParserSurfaceModel` independently and give surface renderers only surface data plus support bindings |
| An optimization rewrites generated Rust text | permit render optimization only over a validated structured render model |
| Deep report-only projection silently changes extraction behavior or cost | keep the early return byte-for-byte in stage A; make O0 an explicit behavior/performance transition with diagnostics, exit-status, manifest, timing, and RSS baselines |
| Error ordering changes when inventories are unified | keep duplicate passes during moves; deduplicate only in a later focused PR |
| Generic codegen gains fixture/language special cases | enforce metadata/ATN modeling and keep mappings in pattern files/tests |
| Generated source shrinks but every recognizer still has machine-code copies | measure linked sections/symbols; use non-generic runtime operations for code intended to be shared |
| A generic runtime helper monomorphizes per context, parser, hook, or listener | inspect monomorphized symbols and retain generated specialization when sharing is not demonstrated |
| An exported macro is presented as binary deduplication | classify it as source compaction unless linked evidence proves folding |
| A plausible runtime boundary is published before its costs are known | run and discard R0.5, then admit only operations supported by its consumer, symbol, and runtime evidence |
| The doc-hidden generated-support ABI changes silently | independently namespace support contracts, test the global-revision mapping, and compile every accepted-revision fixture |
| Mixed old/new generated modules fail in one consumer binary | retain supported revision surfaces and execute an older plus `N+1` fixture |
| Runtime support becomes a dumping ground for grammar data | admit only the R0.5-proven neutral core over runtime-owned types; keep names, nominal context/attrs/validation types, accessors, and dispatch generated |
| Removing empty attrs storage accidentally removes public `__RuleAttrsN` APIs | compile nominal API/trait probes and diff the generated public API before accepting R2 |
| Runtime delegation changes generated auto traits or feature surfaces | compile trait assertions across supported generator flag combinations and same-/separate-crate layouts |
| Lexer-only or minimal consumers pay for parser surfaces | keep optional surfaces generated and inspect runtime package/code-size deltas |
| `rust_output.rs` couples syntax formatting to runtime ABI policy | keep revision/path selection in `parser::surface::support_abi` |
| Runtime extraction is hidden inside a mechanical move | stage A remains byte-preserving; stage R owns the API bump, regeneration, and measurements |
| Issue #276 stalls the conflict-reducing source split | run R0-R0.5 beside A7-A11 and prefer the supported R1-R2 transition after A11 |
| Crate move changes generated header through `env!` | explicit generator build metadata before B2 |
| Fixture paths break in a nested package | one workspace-root test helper |
| Runtime/codegen versions drift | inherited/lockstep versions plus release verification |
| Partial two-crate publication | dependency-order, visibility wait, rerunnable workflow |
| Coverage loses subprocess objects | update object collection and compare Codecov source counts |
| Generated Rust-syntax source obscures hand-written module growth | isolate it under `generated/`, exclude it from line thresholds, and verify its updater |
| Workspace adds crates without useful boundaries | publish only runtime/codegen; keep harness `publish = false` |

## 13. Completion criteria

Stage A is complete when:

- `src/bin/antlr4-rust-gen.rs` contains only module wiring and process entry,
  preferably fewer than 50 lines;
- no codegen production module exceeds 2,000 lines without a documented reason;
- lexer, parser lowering, decision analysis, typed surfaces, semantics, and CLI
  changes normally touch disjoint files;
- embedded models, member parsing, `$` translation, antlr4rust lowering, and
  Rust-syntax queries have separate hand-written ownership;
- checked-in generated recognizers remain isolated and updater-reproducible;
- `grammar/transform.rs` has been replaced by separate integration, validation,
  analysis, registry, and pass-owned modules;
- the grammar-pass result binds the transformed model to its provenance,
  report events, and optional audit artifacts before semantic/ATN construction;
- `CodegenData` has been replaced by typed lexer/parser inputs;
- `ParserSurfaceModel` is independent of parser IR, decisions, and routing, and
  separates per-context declarations from once-per-recognizer glue;
- `GeneratedSupportBindings` and `surface::support_abi` own revision/path
  selection without leaking into `rust_output.rs`;
- lexer and parser renderers consume `LexerRenderModel` and
  `ParserRenderModel`, not process/CLI state;
- parser lowering, optional IR transformation, decision analysis, routing, and
  rendering exchange distinct validated artifacts;
- selectable passes use canonical descriptors and `OptimizationConfig`;
  concrete pass types and order do not appear in the CLI driver;
- mutable-stage analyses are revision-keyed, invalidated conservatively, and
  covered by cached-versus-clean recomputation tests;
- `CompilationReport` is finalized after downstream analysis and artifact
  construction, and its shared envelope has no pass-specific fields;
- the pipeline exposes a non-committing downstream-analysis entry point while
  the existing report-only command retains its early return and behavior;
- optimization tests cover determinism, idempotence, composition, invalidation,
  differential behavior, and benchmark evidence;
- unit tests are colocated and the CLI integration suite is split by concern;
- generated modules and manifests match the active equivalence baseline
  byte-for-byte;
- all CI, conformance, and parity gates pass;
- no stage-A change increments `__ANTLR4_RUST_CODEGEN_API`; stage R owns any
  intervening revision change.

O0 is complete when:

- report-only mode analyzes transformed shadows through downstream stages
  without writing recognizer artifacts;
- unrelated generation policies remain non-enforcing unless an explicitly
  reviewed contract change says otherwise;
- manifest, diagnostic, exit-status, timing, and RSS changes are recorded
  against the pre-O0 baseline;
- apply and report-only projections agree on deterministic downstream metrics;
- normal generation still passes the stage-A equivalence and performance gates.

Stage R is complete when:

- R0.5 has a written decision record and its provisional implementation has
  been discarded before the supported contract is introduced;
- `generated::support::context_v1` contains only spike-proven neutral
  operations, is doc-hidden, independently revisioned, absent from root
  re-exports, and has no codegen dependency;
- mechanically identical eligible context operations delegate to the runtime
  core rather than being emitted once per context;
- generated contexts retain public nominal types, names, signatures,
  cardinalities, `__RuleAttrsN` types and traits, observable trait behavior,
  recovered/active/stored/validated behavior, errors, and callbacks;
- attribute-free rules retain their public attrs APIs while redundant internal
  payload storage and lookups are removed where compatible;
- `__ANTLR4_RUST_CODEGEN_API` is set to implementation-time revision `N+1`,
  maps explicitly to `context_v1`, and older global revisions remain accepted
  only with their complete runtime surfaces; all diagnostics, docs, snapshots,
  and checked-in recognizers are updated;
- older, `N+1`, rejected-new-on-old, every-accepted-arm, and mixed-revision
  compatibility fixtures compile or fail as specified and execute every linked
  recognizer;
- rustdoc/public-API comparison and downstream compile probes pass for nominal
  types, traits, auto traits, and supported generator flag combinations;
- controlled zero/one/two/three-parser fixtures in same- and separate-crate
  layouts, followed by repository and Mehen fixtures, report generated
  lines/bytes, clean and incremental compile cost and peak RSS, stripped
  section sizes/symbols with normal and disabled LTO, first-parse and
  steady-state performance, and memory;
- compatibility and clean-build gates pass on the Rust 1.95 MSRV;
- any claim of one implementation per binary is supported by linked symbol
  evidence rather than source reduction;
- predefined runtime regression tolerances pass for parsing, context accessors,
  validation, listeners, and visitors;
- all CI, conformance, parity, coverage, and clippy gates pass.

Stage B is complete when:

- runtime and codegen are separate workspace packages;
- runtime-only builds have no compiler dependencies or `codegen` feature;
- the generator remains installable as `antlr4-rust-gen`;
- both crates publish from one tag at the same version in dependency order;
- docs, scripts, evidence maps, coverage, and package paths use explicit
  workspace package selectors;
- the codegen crate still keeps grammar frontend and renderer internals private;
- generated support stays in the runtime package and surface ABI lowering stays
  in codegen;
- packaged-runtime tests compile every accepted generated revision and the
  mixed-revision consumer;
- a clean checkout passes the full stage A and R validation matrices.

The practical success test is organizational: a lexer PR, fixed-lookahead PR,
typed-context PR, and semantic-pattern PR should be able to proceed in parallel
without all four editing the same production source file. The consumer success
test is that a third parser can be added without another copy of
recognizer-independent context mechanics, with measured compile, link, and
runtime effects.

## 14. Adversarial review disposition

| Review finding | Incorporated action |
| --- | --- |
| A renderer-only split does not support a growing optimizer | Model the complete pipeline and migrate it through A2-A10 |
| `grammar/transform.rs` is the next conflict hotspot | Split integration, validation, analysis, registry, and pass-owned files in A3 |
| The shared transform report is precedence-specific | Use a neutral report envelope with private pass evidence |
| One parser plan would recreate `CodegenData` later in the pipeline | Use `LoweredParserIr`, `OptimizedParserIr`, `ParserDecisionAnalysis`, `RoutingPlan`, and `ParserRenderModel` |
| Driver booleans and pass order will not scale | Centralize stable descriptors, configuration, validation, and canonical order |
| Analysis invalidation can permit stale reads | Use revision-keyed stage-local caches and clean-recompute tests |
| Source transform reports cannot explain downstream effects | Finalize reports after ATN, decision, IR, routing, and artifact analysis |
| Optimization legality differs by representation | Define separate contracts and forbidden behavior for every stage |
| Existing tests do not prove pass composition or performance value | Add composition, idempotence, invalidation, differential, and benchmark contracts |
| The decomposition could complete while leaving issue #276 untouched | Add the explicit R0-R3 generated-support stage before the workspace move |
| One `ParserRenderModel` couples surface extraction to IR and routing | Build `ParserSurfaceModel` and `GeneratedSupportBindings` independently |
| Macros or generic helpers can shrink source without sharing linked code | Require non-generic runtime operations where sharing is intended and inspect linked symbols |
| The proposed runtime ownership was broader than a stable reusable core | Limit candidates to neutral stored/active operations, named iterators, invocation-state operations, and scans/events; keep grammar-branded types and dispatch generated |
| A support ABI could become permanent before alternatives are measured | Add disposable R0.5 and require a keep/discard decision before publishing `context_v1` |
| Empty attrs optimization could break public nominal APIs | Preserve every `__RuleAttrsN` type and trait while removing only redundant internal storage/lookups |
| Coupling a support path to the next global revision number prevents independent evolution | Map implementation-time global revision `N+1` to independently versioned `context_v1` |
| Current compatibility tests miss old/new recognizers and public trait changes | Add every-accepted-arm, rejected-new-on-old, mixed-revision, public-API, auto-trait, flag, and downstream-consumer fixtures |
| Different grammars at each parser count hide marginal duplication cost | Add controlled 0/1/2/3 copies of one renamed parser with one fixed lexer in same- and separate-crate layouts before repository and Mehen validation |
| Compile and size claims can depend on toolchain or LTO | Record peak RSS and marginal size on Rust 1.95, with LTO disabled and with normal project LTO |
| Issue #276 could block the main conflict-reduction campaign | Run R0-R0.5 beside A7-A11 and prefer landing incompatible R1-R2 after A11 |
| `rust_output.rs` could become an ABI-policy bucket | Keep it syntax-only and put support revision/path lowering in `surface::support_abi` |
| A workspace does not itself deduplicate generated mechanics | Keep support in the runtime, codegen lowering private, and reject a third support crate |
| Deep report-only analysis is not behavior-preserving under the current early-return contract | Keep A10 byte-preserving and move downstream projection plus diagnostics/timing rebaselining to O0 |
| Codegen packaging cannot resolve an unpublished exact runtime version | Publish and await the runtime before packaging or dry-running codegen; verify the local pair in the workspace first |
| The documented Kotlin-only sparse checkout cannot run JavaScript/TypeScript parity | Expand the pinned checkout to all three grammar paths before the complete validation matrix |
