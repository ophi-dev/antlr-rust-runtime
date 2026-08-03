# `antlr4-rust-gen` workspace-first refactoring plan

Status: proposed

Prepared: 2026-08-03

Repository baseline: `bd6710eaedbd79e652681b7b102e03680fd96fbd`

Revision: 2. This revision supersedes the module-first packaging sequence in
Revision 1. It retains the optimization-pipeline requirements from issues
[#125](https://github.com/ophi-dev/antlr-rust-runtime/issues/125) and
`#128`-`#131`, and the multi-recognizer generated-support requirements from
[#276](https://github.com/ophi-dev/antlr-rust-runtime/issues/276).

## 1. Decision summary

Introduce the Cargo workspace and complete the source decomposition in one
atomic PR. Preserve generator CLI syntax, generated/runtime APIs, diagnostics,
and output behavior while intentionally replacing package-selection/install
commands and adding the `Builder` library API. Move each responsibility
directly from its current location to its final package and module. Do not
create intermediate modules under `src/bin_support/`, merge a package-only
state, or require follow-up extraction PRs.

That PR produces:

| Package | Role | Published |
| --- | --- | --- |
| `antlr-rust-runtime` | Runtime and generated-code support contract | yes |
| `antlr-rust-g4-parser` | Checked-in ANTLRv4 lexer/parser and source-syntax facade | yes, implementation detail |
| `antlr-rust-rs-parser` | Checked-in Rust lexer/parser and Rust-syntax query facade | yes, implementation detail |
| `antlr-rust-codegen` | `.g4` compiler, public generator library, `antlr4-rust-gen`, optimization pipeline, and Rust rendering | yes |
| `antlr-rust-runtime-testsuite` | Upstream conformance harness | no |

All packages required to compile `antlr-rust-codegen` must be published because
crates.io does not permit a published package to retain unpublished path-only
or Git dependencies. Consumers still name only the runtime and codegen
packages; Cargo resolves the implementation packages transitively.

The intended Rust consumer experience is:

```toml
[dependencies]
antlr-rust-runtime = "0.25"

[build-dependencies]
antlr-rust-codegen = "0.25"
```

`antlr-rust-codegen` must expose a narrow library API suitable for `build.rs`
and continue to install the `antlr4-rust-gen` binary:

```bash
cargo install antlr-rust-codegen --bin antlr4-rust-gen
```

This avoids an independently downloaded generator for normal Rust builds.
`Cargo.lock`, lockstep releases, exact internal dependency edges, and the
generated-code API check jointly prevent silent generator/runtime drift.

In the same PR, split the 23,000-line generator directly into stage-owned
modules under `crates/antlr-rust-codegen/`, including a dedicated `grammar`
module tree, and isolate both codegen bootstrap frontends in their parser
packages. This gives parallel work the final package and file boundaries
immediately while avoiding intermediate compatibility layers and two rounds of
source movement.

Issue #276 remains a separate behavior-changing track. It moves only proven
recognizer-independent mechanics into the runtime and must measure one-, two-,
and three-parser consumers. Workspace extraction alone neither deduplicates
generated code nor justifies a wider runtime API.

The structural PR must not increment
`__ANTLR4_RUST_CODEGEN_API`. A package name in a generated provenance comment
may change from `antlr-rust-runtime` to `antlr-rust-codegen`; that is an
explicitly reviewed header-only difference, not a runtime contract change.
Any later generated source that requires new runtime support, including the
issue-276 transition, must follow the repository's codegen API revision policy.

## 2. Baseline and constraints

At the baseline above:

| Area | Size or count |
| --- | ---: |
| `src/bin/antlr4-rust-gen.rs` | 23,132 lines |
| Production portion of that file | 16,939 lines |
| In-file unit-test portion | 6,193 lines |
| In-file `#[test]` functions | 161 |
| `tests/antlr4_rust_gen_cli.rs` | 6,668 lines, 129 tests |
| `src/bin_support/embedded.rs` | 8,107 lines |
| `src/bin_support/rust_syntax/mod.rs` | 2,775 hand-written lines |
| Generated Rust-syntax recognizers | 92,784 generated lines |
| Generated ANTLRv4 recognizers | 26,446 generated lines |
| Checked-in generated parser contexts | 282 |
| Existing grammar compiler under `src/bin_support/grammar/` | already partly split by phase |

The root manifest is currently one package. Its optional `codegen` feature
pulls in ICU, `intl`, and graph dependencies, and its `antlr4-rust-gen` binary
reaches support code through `#[path]` modules. The current publish workflow
runs one `cargo publish`, and release-please updates one package version.

### 2.1 Why the first crate cut is atomic

The generated parser crates depend on `antlr-rust-runtime`. If the runtime
package retained the generator binary while depending on either new parser
crate, Cargo would see this cycle:

```text
antlr-rust-runtime
  -> generated parser package
       -> antlr-rust-runtime
```

Optional features do not make package dependency cycles valid. A published
runtime package also cannot use an optional Git dependency to escape the
cycle: crates.io rejects Git-only normal dependencies even when optional.

Therefore the first compilable package split must move all of these together:

1. the generator binary out of the runtime package;
2. the ANTLRv4 generated frontend into `antlr-rust-g4-parser`;
3. the Rust generated frontend and query facade into
   `antlr-rust-rs-parser`;
4. the grammar compiler and remaining generator implementation into
   `antlr-rust-codegen`.

This makes the structural PR large, but it establishes the acyclic final
dependency graph, completes the merge-conflict reduction, and moves each source
family once. Existing unit, snapshot, CLI, conformance, parity, and coverage
tests are the primary safety net. New algorithms and behavior-changing
optimizations still wait for separate work.

### 2.2 Existing boundaries to preserve

- `src/bin_support/grammar/` already owns `.g4` source loading, syntax,
  transforms, semantic analysis, provenance, and ATN construction.
- `src/bin_support/grammar/generated/` contains the checked-in ANTLRv4
  recognizers. `frontend.rs` and `lexer_adaptor.rs` are their hand-written
  source-syntax facade.
- `src/bin_support/rust_syntax/` contains the checked-in Rust recognizers and
  hand-written syntax queries used by antlr4rust compatibility lowering.
- `src/bin_support/embedded.rs` owns embedded-action models, `$`-attribute
  translation, and antlr4rust compatibility behavior.
- `src/bin_support/stack_member.rs`, `templates.rs`, and `rust_names.rs` are
  small codegen services with clear ownership.
- Runtime APIs stay in the root package. Grammar, optimization, and renderer
  models must not move into the runtime for import convenience.

The package split should expose narrow facades around these boundaries. It must
not make every former `pub(crate)` item an advertised public API.

## 3. Goals and non-goals

### 3.1 Goals

1. Establish the final workspace, package dependency direction, and source
   decomposition together.
2. Put each codegen bootstrap frontend in a dedicated package, matching the
   repository layout used by multi-parser consumers such as Mehen.
3. Remove compiler dependencies and the `codegen` feature from the published
   runtime package.
4. Let Rust consumers run generation as a Cargo build dependency without a
   separately managed executable.
5. Keep the CLI available from the same codegen release and implementation.
6. Publish every transitive codegen package in one lockstep release with
   explicit dependency-order automation.
7. Move each production source family once, directly to its final package.
8. Split lexer, parser prediction, typed surfaces, embedded compatibility,
   semantic lowering, grammar transforms, optimization reporting, and CLI work
   into independently owned files after the crate cut.
9. Give grammar, ATN, parser IR, routing, and render-model optimizations
   stage-specific inputs, legality rules, validation, and reporting.
10. Keep issue #276 aligned with consumers that link multiple independently
    generated parser crates.
11. Preserve generator CLI behavior, generated behavior, diagnostics, and
    manifest ordering throughout structural changes.
12. Leave the project fully releasable when the single structural PR merges.

### 3.2 Non-goals

- Do not split the runtime into a new `runtime-core` package merely to move the
  XPath lexer.
- Do not make the runtime depend on codegen, directly or through a feature.
- Do not create a package for every optimizer pass or renderer file.
- Do not claim that published implementation crates have stable standalone
  APIs. Only their narrow codegen-facing facades are compatibility-managed.
- Do not put grammar names, rule names, language extensions, or fixture-specific
  workarounds in generic runtime or codegen paths.
- Do not change parser routing, semantic policy, generated API, optimization
  behavior, or performance policy while relocating and decomposing their
  implementations.
- Do not rewrite generated Rust text as an optimization representation.
- Do not infer linked-code deduplication from fewer generated lines.
- Do not use `build.rs` to write into a consumer's source tree.
- Do not maintain separate release trains for the lockstep implementation
  packages initially.

## 4. Target workspace

Keep the runtime package at the repository root to minimize unrelated path
churn:

```text
Cargo.toml
src/                                      antlr-rust-runtime

crates/
  antlr-rust-g4-parser/
    Cargo.toml
    src/
      lib.rs                              narrow source-syntax facade
      lexer_adaptor.rs
      frontend.rs
      generated/
        antlr_v4_lexer.rs
        antlr_v4_parser.rs

  antlr-rust-rs-parser/
    Cargo.toml
    src/
      lib.rs                              RustSyntax query facade
      generated/
        rust_lexer.rs
        rust_parser.rs
        semantics.json
        decisions.json

  antlr-rust-codegen/
    Cargo.toml
    src/
      lib.rs                              Builder and generator facade
      bin/
        antlr4-rust-gen.rs                process entry only
      cli.rs
      driver.rs
      pipeline.rs
      grammar/
        atn/
        transform/
        ...
      optimization/
      embedded/
      semantics/
      lexer/
      parser/
      ...
    tests/
      antlr4_rust_gen_cli.rs
      antlr4_rust_gen_cli/

tools/
  antlr-rust-runtime-testsuite/
    Cargo.toml                            publish = false
    src/
```

The final dependency graph is one-way:

```text
antlr-rust-codegen
  -> antlr-rust-runtime
  -> antlr-rust-g4-parser
       -> antlr-rust-runtime
  -> antlr-rust-rs-parser
       -> antlr-rust-runtime

antlr-rust-runtime-testsuite depends on runtime and codegen, and is unpublished.
```

No arrow may point from the runtime toward a codegen-side package.

| Package | Direct workspace dependencies |
| --- | --- |
| `antlr-rust-runtime` | none |
| `antlr-rust-g4-parser` | runtime |
| `antlr-rust-rs-parser` | runtime |
| `antlr-rust-codegen` | runtime, G4 parser, Rust parser |
| `antlr-rust-runtime-testsuite` | runtime, codegen |

### 4.1 Runtime package

`antlr-rust-runtime` retains:

- token, stream, ATN, DFA, prediction, parser, lexer, and tree APIs;
- generated recognizer traits and the codegen API compatibility macro;
- current XPath public APIs and its generated lexer;
- future issue-276 generated-support contracts.

After the cut it has no `codegen` feature, generator binary, ICU/codegen graph
dependencies, or generator tests.

XPath is the intentional generated-artifact exception. Moving only its lexer to
a package that depends on the runtime would create `runtime -> xpath-syntax ->
runtime`. Moving all XPath APIs would break the runtime's public API because
the runtime could not re-export a crate that depends back on it. Revisit that
only if a separately justified runtime-core split or XPath package migration is
worth the compatibility cost.

### 4.2 G4 parser package

`antlr-rust-g4-parser` owns the generated ANTLRv4 lexer/parser, lexer adaptor, and
source text to recovered-CST facade. It should expose source-syntax values and
diagnostics needed by `antlr_rust_codegen::grammar`, not its entire
implementation.

Move source IDs/spans and CST types with the frontend when doing so avoids a
dependency back from the parser package to codegen. Codegen can re-export
selected coordinate types internally for migration compatibility. Generated
recognizer modules may remain `#[doc(hidden)] pub` where the hand-written
facade needs cross-package access.

The checked-in generated source is the bootstrap input. This package must not
run the generator from its own `build.rs`.

The developer updater may invoke the workspace generator and must retain its
existing stage-0/stage-1 fixed-point check. That updater relationship is not a
Cargo dependency and therefore does not create a package or publication cycle.

### 4.3 Rust parser package

`antlr-rust-rs-parser` owns the generated Rust lexer/parser and the
hand-written `RustSyntax` query result. Its public facade should be limited to
analysis entry points and query/value types consumed by embedded compatibility
lowering.

The current query module calls cfg and member-item helpers through `super`.
Move syntax-owned helpers into this package or pass their neutral results
through the facade during the atomic cut. Do not create a dependency from this
package back to codegen.

As with the G4 frontend, checked-in generated source is packaged directly
and regenerated only by the repository updater.

The Rust parser updater likewise invokes codegen only as repository tooling.
Published parser source never runs codegen while being packaged or compiled.

Keep the G4 and Rust parser packages separate. They are independently generated
from different grammars, have different adapters and update workflows, and
serve different compiler layers: G4 parsing is required by grammar compilation,
while Rust parsing supports embedded-action compatibility. Separate packages
let Cargo compile them in parallel and keep regeneration or incremental changes
to one from rebuilding the other. Combining them would create a generic
generated-parser bucket without a shared domain API.

### 4.4 Codegen package

`antlr-rust-codegen` owns:

- the public `Builder`/configuration API;
- CLI parsing and the `antlr4-rust-gen` binary;
- source-set loading and import resolution;
- integrated/semantic grammar models, transforms, validation, and provenance;
- lexer and parser ATN construction and packing;
- compiler pipeline orchestration;
- embedded actions and antlr4rust compatibility;
- semantic patterns, inventories, hooks, and SemIR rendering;
- lexer and parser lowering, planning, and Rust rendering;
- optimization selection and cross-stage reporting;
- generated artifacts and filesystem commit policy.

The binary is a thin adapter over the library. Tests must exercise the library
API and the installed CLI surface without maintaining two generation paths.

Keeping the grammar compiler in codegen avoids publishing a broad model/ATN API
with no second consumer. Its `grammar` module remains a strong internal
boundary: it may depend on `antlr-rust-g4-parser` and runtime, but not on Rust
syntax, embedded target compatibility, renderers, CLI arguments, or output
paths.

### 4.5 Testsuite package

Move the runtime testsuite to its own package and mark it `publish = false`.
Its smoke crates continue to use the local runtime path, while the harness
invokes the workspace codegen binary explicitly.

Do not introduce a shared miscellaneous package just for `rust_names`. Either
give the testsuite a local equivalent or expose a genuinely stable artifact
naming function from codegen.

### 4.6 Workspace API policy

Use three API classes:

| Class | Examples | Policy |
| --- | --- | --- |
| Consumer API | runtime APIs; codegen `Builder`, configuration, errors | documented and SemVer-managed |
| Generated-source ABI | runtime support referenced by emitted source | doc-hidden but revisioned and compatibility-tested |
| Lockstep implementation API | generated parser facades consumed by codegen | doc-hidden, exact-versioned, may change in any coordinated release |

Publishing a package makes its `pub` items technically usable by anyone.
`#[doc(hidden)]` is not privacy. Exact internal dependencies and clear package
descriptions prevent accidental compatibility promises while still allowing
the final packages to be installed from crates.io.

## 5. Consumer and contributor experience

### 5.1 Build-time generation

Generation invoked by `build.rs` belongs in `[build-dependencies]`, not
`[dev-dependencies]`:

```toml
[dependencies]
antlr-rust-runtime = "0.25"

[build-dependencies]
antlr-rust-codegen = "0.25"
```

The target API should be close to:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    antlr_rust_codegen::Builder::new()
        .grammar("grammar/MyLexer.g4")
        .grammar("grammar/MyParser.g4")
        .library_directory("grammar")
        .out_dir(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"))
        .generate()?;
    Ok(())
}
```

The builder should:

- accept multiple roots and the same semantic/optimization options as the CLI;
- write only under the requested output directory;
- return the resolved input inventory so a build helper can emit precise
  `cargo::rerun-if-changed` directives;
- return structured errors rather than exiting the process;
- avoid reading CLI state or global environment except through explicit
  defaults;
- expose generator version and codegen API metadata for diagnostics.

Multiple parser crates in one workspace can use the same build dependency.
Cargo compiles one host copy of a given codegen version/profile and each parser
crate runs its own deterministic generation step. Cross-compilation naturally
builds codegen for the host and runtime/generated parsers for the target.

Build-time generation increases clean-build time and the host dependency graph.
Projects that commit generated source should use an `xtask` or the CLI instead
of forcing every downstream build to compile codegen.

### 5.2 CLI generation

Manual and non-Cargo workflows remain:

```bash
cargo install antlr-rust-codegen --bin antlr4-rust-gen
antlr4-rust-gen MyLexer.g4 MyParser.g4 --lib . --out-dir src/generated
```

Repository contributors use:

```bash
cargo run -p antlr-rust-codegen --bin antlr4-rust-gen -- ...
```

The CLI and library call the same pipeline and artifact writer. A CLI feature
must not fork generation behavior from `Builder`.

### 5.3 Package migration

The existing installation command is intentionally replaced:

```text
old: cargo install antlr-rust-runtime --features codegen --bin antlr4-rust-gen
new: cargo install antlr-rust-codegen --bin antlr4-rust-gen
```

Repository commands likewise gain explicit `-p` selectors, and projects moving
generation into `build.rs` add `antlr-rust-codegen` under
`[build-dependencies]`.

Removing the runtime's `codegen` feature and binary target is a package-level
breaking change and must be called out in the changelog, README, and migration
guide. It does not by itself increment `__ANTLR4_RUST_CODEGEN_API`: that
revision tracks compatibility between generated Rust source and runtime APIs,
not how the generator executable/library is packaged.

### 5.4 Version synchronization

Use one version for every published workspace package initially. Internal
package edges use an exact requirement plus a local path:

```toml
antlr-rust-runtime = { version = "=0.25.0", path = "../.." }
```

The actual relative path differs by package and belongs in
`[workspace.dependencies]`.

Synchronization has four layers:

1. one workspace release version and one release tag;
2. exact versions on implementation edges;
3. the consumer's `Cargo.lock`;
4. `__antlr4_rust_require_codegen_api!` in every generated module.

The first three select a coherent toolchain. The fourth remains the source
contract when generated files are committed, copied between projects, or
compiled against a later compatible runtime. Package equality must not replace
the generated-code API revision.

Do not add a runtime `codegen` feature that depends on the codegen package.
Besides recreating the package cycle, a normal dependency does not make its
binary available to `cargo run`.

## 6. Final internal decomposition

The single structural PR establishes package ownership and file-level
parallelism together. Create the following modules directly under the final
packages.

### 6.1 Grammar compiler module tree

```text
crates/antlr-rust-codegen/src/grammar/
  mod.rs                         compiler and model facades
  integration.rs                 imports and combined-grammar integration
  validation.rs                  model invariants after mutation
  transform/
    mod.rs                       grammar-stage pass contract
    registry.rs                  canonical selection and ordering
    analysis.rs                  revisioned stage-local analyses
    artifact.rs                  transformed source/source-map values
    passes/
      precedence_ladder.rs
      prune_unreachable.rs
      factoring.rs               future #129
      inline_rules.rs            future #130
      subsumed_alternatives.rs   future #131
  atn/
    optimize.rs                  mandatory ATN canonicalization
  ...                            loader, model, semantics, provenance, packing
```

Split the current `transform.rs`; it must not become the replacement hotspot.
Mandatory integration and ATN canonicalization are compiler semantics, not
optional optimizer passes.

### 6.2 Codegen package tree

```text
crates/antlr-rust-codegen/src/
  lib.rs                         public Builder facade
  bin/antlr4-rust-gen.rs         argument/process adapter
  cli.rs                         parse injected arguments and usage
  driver.rs                      config -> pipeline -> artifact commit
  pipeline.rs                    typed stage sequencing
  artifact.rs                    output set and collision-safe writes
  rust_output.rs                 syntax-only Rust literals/module framing

  optimization/
    config.rs                    profiles, pass selection, safety policy
    descriptor.rs                stable IDs and ordering
    report.rs                    neutral cross-stage report
    metrics.rs                   deterministic metric snapshots

  structural/
    mod.rs                       source/ATN coordinate projection
    contexts.rs                  child/ref cardinality

  embedded/
    model.rs
    members.rs
    translate.rs
    antlr4rust/
      aliases.rs
      scopes.rs
      macros.rs

  semantics/
    model.rs
    patterns.rs
    inventory.rs
    templates.rs
    hooks.rs
    semir.rs
    manifest.rs
    stack_member.rs
    template_syntax.rs

  lexer/
    render_model.rs
    render.rs

  parser/
    ir/
      mod.rs
      lower.rs
      optimize.rs
    routing.rs
    decision.rs
    manifest.rs
    render_model.rs
    render/
      rules.rs
      decisions.rs
      loops.rs
      fallback.rs
    surface/
      model.rs
      names.rs
      accessors.rs
      support_abi.rs
      contexts.rs
      traversal.rs
      facade.rs

  test_support.rs
```

Create a file when its responsibility moves. Do not add empty pass frameworks.
As review triggers:

- facade and `mod.rs` files should normally stay below 300 production lines;
- ordinary hand-written modules should normally stay below 1,200 lines;
- algorithm-heavy modules may approach 1,800 lines;
- a hand-written module above 2,000 lines needs an ownership justification or
  another split;
- checked-in generated recognizers are exempt but stay isolated and
  updater-reproducible.

### 6.3 Typed pipeline

Replace the all-purpose `CodegenData` with stage-specific artifacts:

```rust
struct LexerCodegenData<'a> { /* grammar, ATN, DFA, semantics */ }
struct ParserCodegenData<'a> { /* grammar, ATN, semantics */ }
struct LexerRenderModel<'a> { /* complete lexer emission input */ }
struct LoweredParserIr<'a> { /* ATN lowered to generated steps */ }
struct OptimizedParserIr<'a> { /* validated optional IR result */ }
struct ParserDecisionAnalysis { /* LL(1), fixed, adaptive facts */ }
struct RoutingPlan { /* generated/interpreted selection */ }
struct ParserSurfaceModel<'a> { /* public grammar-specific API shape */ }
struct GeneratedSupportBindings { /* codegen revision -> runtime support */ }
struct ParserRenderModel<'a> { /* final parser assembly input */ }
struct GeneratedArtifacts { /* normalized path -> bytes */ }
```

The CLI converts arguments to `CompilerConfig`; the driver invokes the
pipeline; the pipeline constructs typed stages; renderers return artifacts;
the artifact layer performs filesystem writes. Renderers do not read source
files, CLI state, or environment variables.

Lexer modules must not import parser render modules. Parser surface rendering
must not receive parser IR or decision internals. Runtime support binding
selection belongs in `parser::surface::support_abi`, not generic Rust
formatting.

### 6.4 Optimization boundaries

Future optimization work must use representation-specific contracts:

| Stage | Allowed work | Forbidden work |
| --- | --- | --- |
| Integrated grammar | Proven structural rewrites before numbering/ATN construction | Guessing around actions, labels, precedence, or lexer priority |
| Semantic grammar | Mandatory semantics and checks | Hidden opt-in performance rewrites |
| Mutable ATN | Semantics-preserving canonicalization | Tree/API-changing source rewrites |
| Decision analysis | Faithful prediction specialization with fallback | Changing alternative order, recovery, or predicate timing |
| Parser IR | Validated control-flow transformations | Editing rendered Rust strings |
| Routing plan | Engine selection from analyzed facts | Rewriting grammar, ATN, or IR |
| Structured render model | Observationally inert layout changes | Recognition, recovery, tree, API, or diagnostic changes |

Each selectable pass has a stable ID, stage, safety class, canonical order,
prerequisites, conflicts, and effective configuration. Pass implementations
remain in stage-owned modules. The CLI driver must not accumulate one boolean
and ordering branch per pass.

Mutable-stage analyses are revision-keyed and conservatively invalidated after
mutation. Validation runs after every changed pass. A transformed grammar,
its provenance, report events, and audit artifacts travel as one typed result
so callers cannot combine transformed models with stale analysis.

`CompilationReport` uses a neutral envelope. Pass-specific proof evidence
remains private to each pass. Complete reports only after downstream decision,
IR, routing, and artifact metrics exist; do not infer effects from source
rewrites alone.

## 7. Single-PR implementation plan

The complete structural refactoring lands in one PR and one squash commit. It
includes the workspace, final package boundaries, full source decomposition,
test relocation, consumer build API, documentation, CI, and publishing
automation. There are no mergeable intermediate layouts and no promised
follow-up extraction series.

Issue #276 runtime deduplication, deeper report-only behavior, and new
optimization implementations remain outside this PR because they intentionally
change generated/runtime behavior. The structural PR creates their final
extension points but does not implement those changes.

### 7.1 Scope checklist

The PR is incomplete until all of these are present:

1. Add the workspace root, shared package metadata, exact path/version
   dependencies, workspace lints, and resolver.
2. Create all three published codegen-side package manifests and the
   unpublished testsuite manifest.
3. Move ANTLRv4 generated recognizers, frontend, lexer adaptor, syntax tests,
   and snapshots directly to `antlr-rust-g4-parser`.
4. Move Rust generated recognizers, syntax query facade, tests, and snapshots
   directly to `antlr-rust-rs-parser`.
5. Move grammar loading, model, integration, transforms, validation, semantics,
   provenance, ATN compiler, Unicode data, tests, and snapshots directly to
   `antlr_rust_codegen::grammar`, already split into the final modules from
   section 6.1.
6. Move generator orchestration, embedded compatibility, semantic rendering,
   lexer/parser planning, decisions, routing, surfaces, output, tests,
   snapshots, and fixtures directly into the final
   `antlr-rust-codegen` modules from section 6.2.
7. Replace `CodegenData` with typed lexer/parser inputs and establish the typed
   pipeline artifacts from section 6.3.
8. Add narrow cross-crate facades and update imports without exposing generated
   frontend internals as supported consumer APIs.
9. Add the public `Builder` entry point and make the CLI a thin adapter over
   the same pipeline.
10. Remove the root package's `codegen` feature, generator binary/test targets,
    and codegen-only dependencies.
11. Move and split the large CLI integration test while retaining one Cargo
    test target.
12. Update all repository commands, scripts, updater paths, CI jobs, coverage
    object discovery, docs, and package selectors.
13. Replace the one-package publish workflow with the complete topological,
    rerunnable workflow from section 8.
14. Compare generated artifacts and diagnostics, permitting only the reviewed
    generated-provenance package-name line to differ.
15. Pass every validation gate in section 9.

### 7.2 Working order inside the branch

The implementation can use local checkpoints to keep the branch buildable, but
those checkpoints are not architectural stages and are never merged
independently. A practical working order is bottom-up:

1. Capture the pre-change equivalence outputs and test baselines.
2. Add final manifests and package skeletons.
3. Move generated parser frontends and establish their facades.
4. Move and fully decompose the grammar compiler under codegen.
5. Move and fully decompose codegen, using temporary local imports only while
   the branch is in progress.
6. Establish `Builder`/CLI parity and relocate integration tests.
7. Remove all temporary paths, aliases, compatibility modules, and root
   codegen targets.
8. Update tooling, documentation, release metadata, and publish automation.
9. Run focused tests during work, then the full clean validation matrix.

The final diff contains only final paths and APIs. Do not leave forwarding
modules under `src/bin_support`, duplicate old/new implementations, or TODOs
that defer a planned module split.

### 7.3 Review and merge strategy

The PR should be rebased on a freshly fetched `origin/main` immediately before
the move. Organize commits or review views by final destination package so
large generated files remain recognizable as renames, but rely on the final
combined test result rather than treating intermediate commits as releasable.

Review in this order:

1. manifests and dependency direction;
2. generated frontend relocations and facade boundaries;
3. grammar decomposition;
4. codegen pipeline and renderer decomposition;
5. build API and CLI parity;
6. tests, scripts, CI, and coverage;
7. release and package verification.

The single PR is intentionally broad. The risk is bounded by byte-level
generation comparison, existing snapshots and integration tests, the upstream
conformance sweep, three language parity suites, package smoke tests, and
CI-parity clippy. Do not reduce scope by merging a temporary architecture.

After merge, ordinary work lands directly in the final owner:

- grammar passes change `antlr_rust_codegen::grammar`;
- Rust compatibility changes `antlr-rust-rs-parser` and codegen embedded
  modules;
- lexer and parser work changes disjoint codegen modules;
- generated frontend updates touch their own packages;
- runtime support changes runtime plus the narrow support binding module.

## 8. Publishing and release changes

### 8.1 Manifest policy

The root manifest becomes both the runtime package and workspace root:

```toml
[workspace]
members = [
    "crates/antlr-rust-g4-parser",
    "crates/antlr-rust-rs-parser",
    "crates/antlr-rust-codegen",
    "tools/antlr-rust-runtime-testsuite",
]
resolver = "3"

[workspace.package]
version = "0.25.0"
edition = "2024"
rust-version = "1.95"
license = "BSD-3-Clause"
repository = "https://github.com/ophi-dev/antlr-rust-runtime"
```

Published members inherit package metadata and declare explicit descriptions,
readmes or package-level documentation, and package contents. The testsuite has
`publish = false`.

Central dependency entries include both local paths and exact registry
versions:

```toml
[workspace.dependencies]
antlr-rust-runtime = { path = ".", version = "=0.25.0" }
antlr-rust-g4-parser = { path = "crates/antlr-rust-g4-parser", version = "=0.25.0" }
antlr-rust-rs-parser = { path = "crates/antlr-rust-rs-parser", version = "=0.25.0" }
```

Use the implementation-time release version rather than copying this example.
Cargo removes local paths from normalized published manifests and resolves the
exact versions from the registry.

### 8.2 Publish graph

Publish in this order:

```text
1. antlr-rust-runtime
2. antlr-rust-g4-parser
3. antlr-rust-rs-parser
4. antlr-rust-codegen
```

Steps 2 and 3 are graph peers but may remain sequential for simpler recovery.
Codegen waits for runtime and both parser packages. Codegen availability is the
marker that a release set is complete.

For each package, the workflow must:

1. verify its resolved version equals the release tag;
2. run `cargo package --list -p <package>` and enforce expected contents;
3. run local workspace tests before any publish;
4. publish or detect that the exact version already exists;
5. poll the sparse registry until that exact package/version resolves;
6. package and verify the next dependent crate against registry dependencies,
   not only local paths.

The job must be rerunnable after any partial success. Published versions are
immutable; a retry skips an already published exact version and continues.
Never use `--no-verify` as the normal solution to dependency propagation.

### 8.3 Release-please

Keep one release component, one tag, and one changelog. Update
`release-please-config.json` and its manifest so a release PR changes:

- `[workspace.package].version`;
- every exact internal version in `[workspace.dependencies]`;
- README dependency and install examples;
- any checked release metadata consumed by generated headers.

Do not assume the Rust updater understands inherited workspace versions and
exact dependency strings. Add explicit extra-file update markers or a checked
release script, then test the generated release diff. CI must fail when
`cargo metadata` reports publishable package versions that differ or when an
internal exact requirement points at another release.

The release commit message and changelog remain repository-wide. Internal
packages do not receive independent release notes.

### 8.4 Crates.io name availability

Checked against the crates.io API on 2026-08-03:

| Proposed package | Exact name | Underscore-normalized lookup |
| --- | --- | --- |
| Runtime | existing `antlr-rust-runtime` 0.25.0 | resolves to the same existing crate |
| Codegen | `antlr-rust-codegen` unclaimed (`404`) | `antlr_rust_codegen` unclaimed (`404`) |
| G4 parser | `antlr-rust-g4-parser` unclaimed (`404`) | `antlr_rust_g4_parser` unclaimed (`404`) |
| Rust parser | `antlr-rust-rs-parser` unclaimed (`404`) | `antlr_rust_rs_parser` unclaimed (`404`) |

`cargo search antlr-rust --limit 100` also showed no package using any of the
three proposed new names. All names satisfy Cargo's package-name syntax and
length limits.

crates.io has no owner-controlled namespaces: owning `antlr-rust-runtime` does
not reserve the `antlr-rust-*` prefix. A `404` is only point-in-time
availability, not a reservation. Recheck exact and normalized names immediately
before merge and first publication. If any name has been claimed, stop and
choose a coherent replacement set rather than silently adding inconsistent
suffixes. Claim the names only by publishing functional packages in accordance
with crates.io policy; do not upload placeholders for name squatting.

### 8.5 Publish workflow and credentials

Replace the current single `cargo publish --dry-run`/`cargo publish` pair in
`.github/workflows/publish.yml` with the ordered loop above. Keep trusted
publishing through the `release` GitHub environment.

Before the first multi-package release:

1. recheck all three exact and underscore-normalized crate names;
2. publish the functional packages and assign the same owners/owner team as
   `antlr-rust-runtime`;
3. configure the repository, workflow, and environment as trusted publishers
   for every crate;
4. document and test the one-time bootstrap if crates.io requires an initial
   owner-token publication before trusted publishing;
5. confirm none of the names redirect to unrelated packages.

Before merge, complete the availability recheck and verify that the initial
publication credential path is ready. Complete ownership and trusted-publisher
setup during the first functional publication.

### 8.6 Package contents

Package checks must prove:

- runtime excludes generator source, fixtures, and codegen-only data;
- G4 parser includes both generated recognizers and required facade
  source;
- Rust parser includes generated recognizers and generation manifests needed
  for audit/reproduction;
- codegen includes the grammar compiler, `unicode_decomposition.bin`,
  CLI/library source, and any built-in semantic data;
- no package relies on files outside its archive;
- no published package contains benchmark corpora or conformance scratch data.

Run a registry-backed smoke project after publishing codegen:

```toml
[dependencies]
antlr-rust-runtime = "=<released-version>"

[build-dependencies]
antlr-rust-codegen = "=<released-version>"
```

The smoke project generates, compiles, and executes at least one lexer/parser
pair without workspace path patches.

### 8.7 Partial publication risk

crates.io has no multi-package transaction. A failure can expose runtime or
implementation packages before codegen. Mitigate this by:

- validating every package archive and the complete local workspace first;
- publishing only immutable, already tested tag contents;
- using exact dependencies;
- waiting for registry visibility at each edge;
- making the workflow idempotent;
- publishing codegen last;
- announcing the release set as complete only after the registry smoke test.

Do not solve partial publication by loosening internal versions or by retaining
Git fallbacks in published manifests.

## 9. Tests, CI, and repository tooling

### 9.1 Workspace commands

After the structural PR, canonical commands become:

```bash
cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo run -p antlr-rust-codegen --bin antlr4-rust-gen -- ...
cargo test -p antlr-rust-codegen
cargo test -p antlr-rust-g4-parser
cargo test -p antlr-rust-rs-parser
cargo test -p antlr-rust-runtime
```

Use `cargo fmt --check --all` if supported by the pinned toolchain, and format
only files touched by the structural PR.

### 9.2 Test relocation

Move tests and snapshots with their production owner in the structural PR.
Keep generated frontend snapshots in their parser packages and
grammar/renderer snapshots in codegen. Every test module using Insta retains
the required `clippy::disallowed_methods` allowance.

Rename external snapshot files with their test modules and update source-path
metadata, but verify that every snapshot payload remains unchanged unless it
contains the explicitly approved generator provenance line.

Split the large CLI integration suite into source modules while keeping one
integration target:

```text
crates/antlr-rust-codegen/tests/antlr4_rust_gen_cli.rs
crates/antlr-rust-codegen/tests/antlr4_rust_gen_cli/
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

The support module owns workspace-root resolution, temporary consumer crates,
generator invocation, and normalization.

### 9.3 Equivalence gates

At the start of the structural branch, record a representative deterministic
generation set. Compare before and after:

- generated `.rs` files;
- `semantics.json`, `decisions.json`, and `optimizations.json`;
- stdout, stderr, and exit status for failing inputs;
- package and binary `--version` output;
- updater-generated checked-in recognizers.

Normalize only the intentionally changed generator package name in the
generated provenance line. All other differences require explanation. A
source-only move never changes `__ANTLR4_RUST_CODEGEN_API`.

### 9.4 Build-dependency contract

Add fixtures for:

- generation from `build.rs` into `OUT_DIR`;
- precise rerun behavior after grammar and imported grammar changes;
- no rerun after unrelated source changes;
- cross-compilation host/target separation where CI supports it;
- two and three parser crates sharing one codegen build dependency;
- structured library errors matching CLI diagnostics;
- a committed-generated-source project with no codegen dependency.

The build API and CLI must generate identical files for the same explicit
configuration.

### 9.5 Scripts and workflows

Update in the structural PR:

- README and language build guides;
- `AGENTS.md` and `CLAUDE.md`;
- grammar frontend and Rust-syntax updaters;
- Kotlin, JavaScript, and TypeScript parity scripts;
- fixed-lookahead and parse benchmarks;
- runtime testsuite prebuild/discovery;
- CI target selection;
- LLVM coverage object discovery;
- release-please and publish workflows;
- generated evidence that records executable commands.

Replace repeated `../../..` assumptions with one workspace-root mechanism.
Validate workflow changes with `actionlint`.

Coverage must still include runtime, both generated parsers, codegen, testsuite,
and nested smoke binaries. Compare source-file inventories and percentages
before and after the structural PR so package relocation does not silently
drop instrumented objects.

### 9.6 Package validation

For every published package:

```bash
cargo package --locked -p <package>
cargo package --list -p <package>
```

After each upstream package is visible, verify downstream archives using the
registry-normalized manifest. Test the declared Rust 1.95 MSRV as well as the
normal stable CI toolchain.

### 9.7 Single-PR acceptance gate

The structural PR is not mergeable until one clean checkout passes:

```bash
cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo llvm-cov --locked --workspace --all-features \
  --lcov --output-path lcov.info

cargo run --release --quiet \
  -p antlr-rust-runtime-testsuite --bin antlr4-runtime-testsuite

tests/kotlin-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
tests/javascript-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
tests/typescript-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4

actionlint
```

Expand the pinned `grammars-v4` sparse checkout to all three language paths
before running the parity commands. Run the behavioral-equivalence fixture,
package/archive checks, build-dependency fixtures, and registry-normalized
smoke preparation in the same gate. Compare coverage source inventories as
well as percentages.

## 10. Multi-recognizer generated support

The workspace shape improves compilation ownership for consumers with several
parser crates, but repeated generated mechanics remain until issue #276 moves a
proven subset into runtime support.

Classify generated items before moving them:

| Ownership | Examples | Treatment |
| --- | --- | --- |
| Candidate runtime core | non-generic context operations over runtime nodes, named iterators, invocation-state lookup, neutral scans/events | Prototype and measure; move only proven neutral operations |
| Grammar-specific API/data | metadata, ATN/DFA words, rule/token IDs, context and attrs types, accessors, callbacks | Keep generated |
| Generated specialization | rule bodies, prediction, hooks, validation policy, parser/lexer adapters | Keep generated unless measurements prove otherwise |

Use these stages:

1. **R0 baseline:** zero, one, two, and three copies of one parser in same-crate
   and one-parser-per-crate layouts, followed by repository and Mehen shapes.
2. **R0.5 disposable spike:** compare type-erased/non-generic support with the
   best generic or macro alternative. Inspect compile cost, symbols, linked
   sections, and runtime behavior. Discard the spike.
3. **R1 runtime contract:** add only accepted operations under a versioned,
   doc-hidden path such as
   `antlr4_runtime::generated::support::context_v1`.
4. **R2 generated binding:** allocate implementation-time codegen revision
   `N+1`, map it explicitly to `context_v1`, regenerate checked-in artifacts,
   and preserve older accepted revisions while their full runtime surfaces
   remain.
5. **R3 further families:** evaluate once-per-recognizer cache, traversal,
   validation, and facade glue independently.

Start R0/R0.5 after the structural PR has established
`ParserSurfaceModel`. Land R1/R2 later so an intentional generated/runtime ABI
change does not obscure the behavior-preserving refactor.

Measure four families separately:

- generated lines and bytes;
- clean/incremental compile time and peak RSS;
- stripped binary and `.text`/`.rodata`/writable section sizes with normal and
  disabled LTO, including marginal parser cost;
- first-parse latency, steady-state throughput, and memory.

Macros and generic helpers count as source compaction unless linked symbols
show one implementation. Preserve nominal contexts, every `__RuleAttrsN` API,
labels, accessors, listeners, visitors, validation behavior, and observable
traits unless a separate compatibility change explicitly says otherwise.

## 11. Optimization readiness

The workspace boundaries align with future optimization work:

- source grammar transforms and ATN canonicalization live in
  `antlr_rust_codegen::grammar`;
- decision, parser-IR, routing, and render-model optimization live in
  `antlr-rust-codegen`;
- runtime hot-path work stays in `antlr-rust-runtime`;
- generated frontend packages remain bootstrap inputs, not optimizer hosts.

Before adding another pass, tests must cover deterministic output/reporting,
idempotence, canonical ordering, conflicts, analysis invalidation,
cached-versus-clean recomputation, provenance, apply/report-only agreement,
explicit declined reasons, valid/invalid differential parsing, diagnostics,
recovery, tree/API behavior required by the safety class, and same-machine
interleaved performance evidence.

No central optimizer crate is proposed. The neutral descriptor/reporting
module belongs in codegen because it orchestrates cross-stage evidence, while
each mutation implementation stays with the representation it owns.

## 12. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| The single structural PR is too large to review conventionally | organize review by final package, preserve generated files as renames, forbid behavior changes, and rely on the complete equivalence/conformance matrix |
| A generated frontend is extracted without moving the runtime binary and creates a package cycle | include runtime binary removal and every final codegen-side package in the same PR |
| Former `pub(crate)` items become accidental public APIs | narrow facades, doc-hidden implementation modules, exact lockstep edges, and package descriptions that deny standalone stability |
| Published package count burdens releases | one version/tag/changelog, automated topological publishing, and codegen-last completion |
| Partial crates.io publication | preflight all archives, exact dependencies, visibility polling, idempotent retries, and final registry smoke test |
| Release-please misses inherited or exact versions | explicit update markers/script plus metadata consistency CI |
| First publish lacks trusted-publisher authorization | recheck names before merge and prepare the initial owner-token/trusted-publisher bootstrap before release |
| Build-time codegen slows clean builds | keep it opt-in as a build dependency and document committed-source/xtask workflow |
| Build dependency and target runtime resolve incoherently | lockstep versions, exact internal runtime edge, Cargo.lock, and generated-code API compile check |
| Existing users invoke the runtime package's `codegen` feature/binary | treat removal as a package-level breaking migration and document the replacement install/build-dependency commands |
| Runtime regains a codegen feature for convenience | enforce one-way package graph and test runtime package metadata |
| XPath extraction creates a cycle or API break | keep XPath in runtime until a separately justified runtime-core/API migration |
| Generated headers change from `env!` package metadata | inject explicit generator identity and approve only the provenance-line change |
| Fixture paths break after relocation | one tested workspace-root helper |
| Coverage loses subprocess objects | update object collection and compare instrumented source inventory |
| Generated source dominates review | isolate generated renames and verify updater output separately |
| A renderer-only split blocks new optimization stages | preserve grammar, ATN, decision, IR, routing, surface, and render-model boundaries |
| Shared report DTO becomes pass-specific | neutral envelope plus private pass evidence |
| Mutable analysis becomes stale | revision-keyed caches, conservative invalidation, validation, and clean recomputation tests |
| Workspace is mistaken for generated-code deduplication | keep issue #276 evidence and runtime-support stages separate |
| Generic runtime support multiplies machine code | inspect monomorphized symbols and retain generated specialization when sharing is not proven |
| Internal crates attract direct consumers | clear naming/docs and no stability promise; accept that publishing cannot technically prevent direct use |

## 13. Completion criteria

The single structural PR is complete when:

- all final packages exist with the dependency graph in section 4;
- both codegen bootstrap frontends are in their dedicated parser packages;
- XPath remains an explicit documented runtime exception;
- generator and grammar source have moved directly to final packages;
- runtime has no codegen feature, generator target, or codegen-only dependency;
- `antlr-rust-codegen` exposes one pipeline through both `Builder` and the CLI;
- changelog, README, and migration docs replace the old runtime-package install
  command and explain the build-dependency workflow;
- a build-dependency consumer generates and runs a parser;
- all publishable packages share one version and exact internal edges;
- release automation publishes all four packages in order and can resume after
  partial success;
- package archives are complete and registry-backed smoke generation passes;
- generated behavior is unchanged apart from the approved provenance line;
- `__ANTLR4_RUST_CODEGEN_API` is unchanged;
- the binary entry point contains only argument/process adaptation;
- no hand-written production module exceeds 2,000 lines without a documented
  reason;
- grammar integration, validation, analyses, registry, and passes are separate;
- embedded model/member/translation/compatibility concerns are separate;
- `CodegenData` is replaced by typed lexer/parser inputs;
- lexer, parser IR, decisions, routing, surfaces, and rendering exchange
  validated stage artifacts;
- renderers do not read CLI, filesystem, or environment state;
- tests and snapshots are colocated with their owners;
- optimization descriptors and reporting scale without driver booleans or
  pass-specific shared fields;
- normal generation remains byte- and behavior-equivalent;
- CI, clippy, coverage, conformance, parity, and package gates pass.

The multi-recognizer track is complete only when:

- the disposable ABI spike has a recorded keep/discard decision;
- runtime support contains only measured neutral operations;
- the generated-code revision transition follows the repository policy;
- older, new, rejected-new-on-old, and mixed-revision fixtures behave as
  specified;
- same- and separate-crate zero/one/two/three-parser fixtures report source,
  compile, linked-size, and runtime measurements;
- Mehen and repository dogfood shapes validate the result;
- any claim of one shared implementation is backed by linked-symbol evidence.

The organizational success test is that grammar transform, lexer, parser
decision, typed-surface, embedded compatibility, and semantic-pattern PRs can
proceed in parallel without sharing a production source file. The consumer
success test is that a workspace with several parser crates pins and builds one
coherent codegen toolchain through Cargo without separately installing a
binary.

## 14. Reference projects and review disposition

The structure combines:

- [rust-analyzer](https://github.com/rust-lang/rust-analyzer) for a large,
  explicit workspace dependency graph and release-built tooling;
- [Tree-sitter](https://github.com/tree-sitter/tree-sitter) for generated
  grammar ownership in dedicated packages;
- [Biome](https://github.com/biomejs/biome) for parser/compiler stage
  boundaries;
- [LALRPOP](https://github.com/lalrpop/lalrpop) and
  [prost-build](https://github.com/tokio-rs/prost) for the Rust pattern of a
  runtime dependency paired with a build-time generator library.

Revision 2 incorporates these review conclusions:

| Finding | Action |
| --- | --- |
| A module-first then crate-first migration moves the same code twice | Introduce the workspace first and move directly to final packages |
| One codegen crate leaves generated frontends as hidden coupling | Give G4 and Rust generated frontends dedicated parser packages and facades |
| Extracting a parser package while the binary remains in runtime creates a cycle | Put the complete workspace, generator move, and dependency-DAG cut in one structural PR |
| The grammar compiler package has one consumer and a broad unstable API | Keep it as `antlr_rust_codegen::grammar`; publish only independently generated parser boundaries |
| One published runtime forces users to manage a separate generator binary | Publish the complete codegen DAG and provide a build-dependency API |
| A dev dependency does not make a dependency's binary executable | Use a library API in `[build-dependencies]`; retain the CLI in the same package |
| Published internal crates create API and release burden | Use narrow/doc-hidden facades, exact lockstep versions, one release, and automated topological publication |
| crates.io has no protected project namespace | Check exact and normalized names, record current availability, recheck before merge, and publish functional packages promptly |
| crates.io publication is not atomic | Validate first, publish in dependency order, poll visibility, resume safely, and publish codegen last |
| Staged structural PRs add temporary paths and compatibility work | Land workspace conversion and the complete source decomposition in one tested PR |
| Multiple parsers per binary amplify repeated generated helpers | Keep the issue-276 runtime-support track and require one/two/three-parser measurements |
| Workspace boundaries alone do not support future optimizer stages | Preserve distinct grammar, ATN, decision, IR, routing, and render-model contracts |
| XPath cannot follow the same generated-crate rule without a cycle or API break | Keep it in runtime as an explicit exception pending separate architectural evidence |
