# Issue #141: direct `.g4` codegen implementation plan

Status: revised for an intentionally breaking v0.14.x cutover
Prepared: 2026-07-21
Repository baseline: `7b27c7f59677664906e019ca66ae003cedebb904`
(`v0.14.2`, identical to `origin/main`)
Issue: <https://github.com/ophi-dev/antlr-rust-runtime/issues/141>

## 1. Executive decision

Implement one Rust-owned compiler pipeline:

```text
one or more .g4 roots
  -> lossless source set and dependency graph
  -> import integration, combined split, and preliminary analysis
  -> optional source transforms with analysis invalidation
  -> rule collection/basic checks and structural left-recursion rewrite
  -> full symbols, numbering, and semantic checks
  -> lexer/parser ATN construction and post-build analysis
  -> Rust emission
```

The final production command accepts `.g4` roots directly. It does not invoke
Java, Node, antlr-ng, or another ANTLR implementation, and it does not read,
write, or round-trip through `.interp`.

The implementation will:

- keep all grammar frontend and tool-model code private to the generator under
  `src/bin_support/grammar/`;
- vendor the pinned ANTLR-v4 meta-grammar and check in its generated Rust
  frontend with BSD attribution;
- treat Java ANTLR 4.13.2 as the compatibility oracle and pinned antlr-ng as an
  implementation blueprint, not as production dependencies;
- replace `InterpData` with one provenance-rich `Compilation` shared by all
  emitters and analyses;
- construct a parser ATN in memory and lower it directly once into
  `ParserAtnBuilder`'s packed representation;
- construct a lexer ATN once, compile its DFA from that graph, and serialize
  lexer runtime data only at the final artifact boundary while the runtime
  still requires it;
- use checked-in Java `.interp` files only as test fixtures and serializer
  oracles; production never emits or consumes them after the cutover;
- expose a first-class transform boundary for #128 without implementing
  #129-#131 in this issue;
- retain labels and typed rule structure needed by #138 without implementing
  the visitor API in this issue.

This is a codegen correctness, reproducibility, latency, and architecture
project. It does not claim parser runtime speedups. Any runtime claim requires
separate parse benchmarks and a separately identified artifact change.

This is an early `0.14.x` project. Breaking the CLI, generated names, generated
source shape, and runtime-version expectations is allowed. There is no
deprecation window, preview release, compatibility adapter, or staged consumer
migration. Delivery is split only to keep individual code reviews tractable.

## 2. Grounded current state

### 2.1 Live issue and repository state

- At plan preparation, issue #141 was open, had no comments, and had no linked
  implementation PR.
- No implementation PR existed; the pull request carrying this document is
  planning-only.
- Local `main`, `origin/main`, and tag `v0.14.2` all point to
  `7b27c7f59677664906e019ca66ae003cedebb904`.
- The worktree was clean before this plan was added.

The issue's final-state contract is unambiguous:

- `.g4` roots are sufficient production input;
- the Rust path owns imports, combined splitting, `tokenVocab`, numbering,
  semantic checks, left recursion, ATN construction, optimization, and
  provenance;
- `.interp` is checked-in test-oracle data, not a second compatibility
  architecture or release input;
- public `--lexer` and `--parser` input are removed in the wiring cutover;
- parser ATNs are packed directly;
- current source/ATN positional correlation must disappear;
- the full zero-skip runtime testsuite and all real-grammar gates must use the
  direct path.

GitHub search found only two external repositories, `ophi-dev/mehen` and
`ophi-dev/gremlin-rs`. Both are same-organization projects with pinned runtime
versions. They are integration playgrounds for the cutover, not compatibility
constraints on this design.

### 2.2 Current production path

`src/bin/antlr4-rust-gen.rs` is a 16,306-line binary module. At this baseline it
contains:

| Indicator | Current count |
| --- | ---: |
| textual `grammar_source` references | 98 |
| textual `InterpData` references | 75 |
| explicit `AtnDeserializer::new` calls | 14 |

The concrete coupling is:

- `src/bin/antlr4-rust-gen.rs:58-170` reads the optional source as one string,
  but first parses each required `.interp` into `InterpData`.
- `src/bin/antlr4-rust-gen.rs:1854-1991` defines the CLI and rejects an
  invocation without `--lexer` or `--parser`; `--grammar` is singular and
  supplementary.
- `src/bin/antlr4-rust-gen.rs:1993-2101` defines the line-oriented
  `InterpData` parser and its literal, symbolic, rule, channel, mode, and
  serialized-ATN fields.
- `src/bin/antlr4-rust-gen.rs:7508-7513` deserializes and repacks the parser
  ATN. The generated parser then embeds the packed `u32` stream at
  `src/bin/antlr4-rust-gen.rs:7759-7767`.
- `src/bin/antlr4-rust-gen.rs:2343-2352` emits a serialized lexer ATN that every
  generated process deserializes once at runtime. Lexer DFA compilation also
  starts from a deserialized copy during codegen.
- Helpers independently deserialize the same ATN for generated-rule
  compilation, caller reachability, decision analysis, action/predicate
  inventories, rule argument pairing, and entry-rule inference.

`GrammarMetadata` already avoids embedding duplicate serialized parser data:
`render_parser_metadata` passes an empty ATN slice at
`src/bin/antlr4-rust-gen.rs:10014-10018`. This makes `ParserAtnBuilder`
(`src/atn/parser_atn.rs:1182-1348`) the established final parser artifact
rather than a new format to design.

### 2.3 Positional machinery that must be removed

The current split authority forces source scans to be paired back to ATN
coordinates by order and count:

- `embedded_rule_call_args` scans `rule[expr]` text and greedily pairs calls to
  same-callee rule transitions (`src/bin/antlr4-rust-gen.rs:6864-6941`).
- `embedded_lexer_actions` zips source-ordered bodies with deserialized custom
  action coordinates and errors on a count mismatch
  (`src/bin/antlr4-rust-gen.rs:8189-8221`).
- `action_slot_state_offset` assumes extra per-rule action states are
  synthesized ahead of authored slots
  (`src/bin/antlr4-rust-gen.rs:8696-8731`).
- `synthetic_parser_action_states` infers authored versus synthesized actions
  from block counts, attributed slots, and residual state order
  (`src/bin/antlr4-rust-gen.rs:9493-9593`).
- `parser_rule_args` scans source order and greedily consumes ATN rule
  transitions (`src/bin/antlr4-rust-gen.rs:9613-9653`).
- `likely_parser_entry_rule_indices` deserializes again to reconstruct a call
  graph (`src/bin/antlr4-rust-gen.rs:9914-9941`).

These functions are deletion targets, not algorithms to transplant into the
new frontend.

### 2.4 Runtime-testsuite coupling

The harness currently:

- renders each upstream descriptor through the real StringTemplate test group;
- writes the root and slave `.g4` files;
- concatenates rendered root/import sources into a synthetic
  `<name>.source.g4` at `src/bin/antlr4-runtime-testsuite.rs:793-911`;
- invokes Java ANTLR to create `.interp` at
  `src/bin/antlr4-runtime-testsuite.rs:913-924`;
- passes those files plus the synthetic source into the generator at
  `src/bin/antlr4-runtime-testsuite.rs:965-997`.

The synthetic concatenation exists only to make positional source scans line up
with an ATN already constructed elsewhere. The direct path must pass the real
root and import files to the source-set loader instead.

The harness still needs Java/StringTemplate to render upstream target templates
for this test suite. That is test infrastructure, not production codegen. The
ANTLR jar must stop being used to build metadata for the Rust side.

### 2.5 Current validation baseline

The baseline was rerun on the exact commit above:

```text
cargo test --locked
  247 runtime unit tests passed
    2 runtime-testsuite unit tests passed
  164 generator unit tests passed
    7 generator CLI integration tests passed
  420 total passed; 0 failed
```

Using a clean ANTLR 4.13.2 checkout at release commit
`cc82115a4e7f53d71d9d905caa2c2dfa4da58899`:

```text
cargo run --release --locked --quiet --bin antlr4-runtime-testsuite -- \
  --descriptors /tmp/antlr-cleanroom/antlr4-4.13.2-runtime/runtime-testsuite

summary: 357 passed, 0 failed, 0 skipped, 357 run
```

The current host reports Homebrew OpenJDK `26.0.1`. That ambient installation
is adequate for the baseline run but is not a reproducible oracle lock; the
fixture updater records an exact JDK vendor/build before regenerating checked-in
Java `.interp` or diagnostic fixtures.

Every implementation phase must preserve these gates. The direct path is not
done merely because it can compile a few grammars.

### 2.6 Upstream correspondence

Use these exact references:

- Java compatibility oracle: ANTLR 4.13.2 release commit
  `cc82115a4e7f53d71d9d905caa2c2dfa4da58899`.
- TypeScript blueprint and meta-grammar source: antlr-ng commit
  `1f68422ae4bfc62f93343769e144d01f305487b1`.
- Existing bootstrap proof: issue #128 comment
  <https://github.com/ophi-dev/antlr-rust-runtime/issues/128#issuecomment-5017282066>.

Java `Tool.process`, Java `SemanticPipeline.process`, and antlr-ng
`Tool.process` establish the required order:

```text
load imports
-> integrate imports / mandatory grammar transforms
-> split an implicit lexer from a combined grammar
-> collect rules and run basic checks
-> rewrite immediate left recursion
-> collect symbols, import vocabulary, and assign numbers
-> lexer or parser ATN factory
-> ATN optimization and post-build analysis
-> interpreter serialization / target codegen
```

The optional #128 transform boundary is a Rust insertion after import
integration and preliminary name/call/vocabulary analysis, but before the
final semantic pipeline. Every accepted pass invalidates and recomputes the
preliminary analyses it can affect. It does not move final numbering ahead of
left-recursion rewriting.

The Rust implementation ends the shared frontend work after ATN analysis and
feeds its existing Rust emitter. It does not port antlr-ng's target
StringTemplate emitter.

Porting policy:

- every port record names pinned `primary_test_source` and
  `alternate_test_source` identities; shared Java/antlr-ng cases default to
  Java first and antlr-ng second, while documented single-source or external
  assertions may name a different pair;
- port and lock the record's primary test/fixture first, in a clean context
  that has not inspected either candidate implementation;
- use the pinned antlr-ng TypeScript implementation as the primary
  implementation source, in a separate commit that may not edit the locked
  test;
- port one Rust test for a shared logical case and record both upstream
  locations rather than duplicating Java and TypeScript variants eagerly;
- on failure, independently port the record's alternate test before consulting
  or debugging the Rust implementation; if the required test oracles still
  fail, port the smallest corresponding Java implementation unit;
- if Java 4.13.2 and antlr-ng disagree, retain both observations in the test
  map, but Java 4.13.2 is the compatibility verdict unless an intentional
  difference is approved;
- debug or reinterpret the test only after the alternate test and alternate
  implementation ports have failed, following section 11.6.

The primary upstream algorithm correspondences are:

| Rust responsibility | Java 4.13.2 reference | antlr-ng reference |
| --- | --- | --- |
| orchestration/order | `org.antlr.v4.Tool` | `src/Tool.ts` |
| imports/combined split | `GrammarTransformPipeline` | `src/tool/GrammarTransformPipeline.ts` |
| semantic stages/numbering | `SemanticPipeline` | `src/semantics/SemanticPipeline.ts` |
| immediate left recursion | `LeftRecursiveRuleTransformer` and `LeftRecursiveRuleAnalyzer` | `src/analysis/LeftRecursiveRule*.ts` |
| parser ATN | `ParserATNFactory` | `src/automata/ParserATNFactory.ts` |
| lexer ATN | `LexerATNFactory` | `src/automata/LexerATNFactory.ts` |
| ATN optimization | `ATNOptimizer` | `src/automata/ATNOptimizer.ts` |
| post-build checks/LL(1) | `AnalysisPipeline` | `src/analysis/AnalysisPipeline.ts` |
| token vocabulary files | `TokenVocabParser` | `src/parse/TokenVocabParser.ts` |

### 2.7 Supplemental `vscode-antlr4` fixture evidence

Use `mike-lischke/vscode-antlr4` commit
`3e9469d1d490c71b3e3b909edf1235582a3f8db8` as a pinned third source of
grammar inputs and test ideas. It is not an implementation source and does not
change the Java/antlr-ng precedence above. The repository is MIT-licensed;
the three ANTLR-v4 meta-grammar files retain their BSD notices, and
`CPP14.g4` retains Camilo Sanchez's embedded MIT notice.

The pinned tree has exactly 12 tracked `.g4` files: three frontend grammars and
nine backend test-data grammars. Its focused symbol, reference-graph, and bug
tests passed locally (`11/11`). Cross-running every relevant fixture with Java
4.13.2 and pinned antlr-ng established these roles:

| Fixture | Plan role | Grounded result |
| --- | --- | --- |
| `ANTLRv4Lexer.g4`, `ANTLRv4Parser.g4`, `LexBasic.g4` | Phase A alternate meta-grammar/import/adaptor corpus; Phase B import and serializer fixture | parser and `LexBasic` `.interp` are byte-identical; lexer serialized ATN is identical, while Java writes `null` channel holes and antlr-ng writes blank lines |
| `TParser.g4` + `TLexer.g4` | principal Phase A syntax/span corpus; Java-verdict Phase B split-grammar case | covers scoped named actions, args/returns/locals, init/after, catch/finally, labels, predicates/actions, nongreedy EBNF, wildcard, immediate LR, `tokenVocab`, tokens/channels/modes, and lexer commands; Java accepts it, while antlr-ng rejects `t = .` at line 119 with error 64 |
| `CPP14.g4` | large Phase A combined-grammar input and Phase B parser/lexer ATN fixture | Java/antlr-ng `.interp` files are byte-identical: 96,060-byte parser and 51,824-byte lexer fixtures |
| `OddExpr.g4` | Phase A scale input and Phase B serializer/lexer-ATN stress fixture | Java/antlr-ng `.interp` files are byte-identical; the 118,094-byte grammar produces a 3,811,286-byte lexer `.interp`, 3,745 token types, and 101,246 lexer ATN states |
| `sentences.g4` | Unicode-property, direct-LR, and serializer divergence fixture | parser `.interp` is byte-identical; lexer ATNs differ because the pinned tools embed different Unicode property tables, with antlr-ng pinned to Unicode 16.0 data; Java is normative |
| `t.g4` | Phase A source-span/symbol input and Phase B mixed warning/error fixture | Java reports warning 125 for implicit `ZZ` and error 177 for unknown channel `BLAH`; antlr-ng reports corresponding codes 61 and 106 |
| `t2.g4` | Phase B indirect-LR diagnostic fixture | Java reports error 119 with `[a, c, b]`; antlr-ng error 55 repeats members as `[a, c, b, b, c]`; preserve both raw outcomes and use Java's verdict |
| `TParser2.g4` + `TLexer2.g4` | Phase B missing-`tokenVocab` diagnostic fixture | both reject the missing vocabulary; Java reports error 114 and antlr-ng error 54, while the rendered path text depends on the recorded working directory and command |

The extension's `OddExpr` exception test contains assertions only inside its
`catch`, so its passing result does not prove an exception occurred. A direct
run with the test's exact flags and bundled `antlr4-4.13.2-SNAPSHOT` jar
(SHA-256
`156a885bdaf847601d4b0350185dfaa5c4aee36c9a9c4c1632c31b0381360d39`)
exits successfully, as do the official Java 4.13.2 release jar and pinned
antlr-ng. Do not port the catch-only expected failure. Preserve it only as the
historical reason this valuable stress grammar entered the extension.
The current legacy Rust generator also consumed the official release fixtures
successfully and emitted 5,688,181 bytes of Rust/manifest output, so this is a
practical end-to-end scale case rather than a serializer-only synthetic.
Do not treat editor-only symbol diagnostics, target-file existence checks,
random sentence generation, or UI behavior as compiler compatibility
requirements.

## 3. Scope and non-negotiable invariants

### 3.1 In scope

- Lossless parsing of lexer, parser, combined, and imported ANTLR 4 grammars.
- Multi-root source loading, deterministic import lookup, and `tokenVocab`.
- Mandatory import integration and combined-grammar splitting.
- Semantic checks needed to construct a correct grammar model and ATN.
- Exact token, channel, mode, rule, action, and predicate numbering.
- Structural immediate-left-recursion rewriting and recursion validation.
- Lexer and parser ATN construction, optimization, and analysis.
- Stable source/model/ATN provenance.
- Conversion of every in-generator grammar-source reader to the shared model.
- Checked-in Java `.interp` fixtures plus a test-only Rust serializer.
- The pinned 12-file `vscode-antlr4` source corpus with explicit provenance,
  license, oracle, and phase roles.
- Same-org downstream validation using mehen and gremlin-rs.
- Final deletion of `.interp` production input and positional correlation.

### 3.2 Out of scope

- Implementing the optimization passes in #129-#131.
- Implementing the visitor API in #138.
- Translating arbitrary Java, TypeScript, C++, Python, or other target-language
  actions or superclass code into Rust.
- Porting another target's StringTemplate code generator.
- Supporting source-less `.interp` distributions in the new tool.
- General incremental compilation or a persistent cache. The model must permit
  it later, but parity comes first.
- Claiming parser throughput improvement from a codegen-only architecture
  change.

### 3.3 Invariants enforced in code and tests

1. Generic frontend, semantic, ATN, and emitter paths contain no grammar names,
   source-language names, benchmark rule names, or language file extensions.
2. A lexer or parser syntax error is fatal. A recovered meta-parser tree is
   never transformed, semantically analyzed, or emitted.
3. A semantic error is fatal and emits no partial Rust modules or manifest.
4. Actions and predicates retain opaque source bodies and structural owners.
   Only existing body translators/patterns interpret their contents.
5. Every authored and synthesized model node has provenance. Every ATN state
   and transition that represents grammar structure links back to that
   provenance.
6. State and alternative order are explicit. Hash-map iteration and directory
   traversal never decide numbering or emitted order.
7. Each grammar is parsed once, each dependency is resolved once, and each ATN
   is constructed/analyzed once per invocation.
8. The direct production path has no fallback to Java, antlr-ng, `.interp`, or
   the old frontend. Unsupported input produces a source diagnostic.
9. `.interp` parsing/serialization retained after the cutover is test-only;
   production has no legacy frontend or fallback.
10. Optional optimization passes are opt-in, deterministic, idempotent, and
    operate after preliminary import/name/call/vocabulary analysis but before
    final semantic numbering, left-recursion rewriting, and ATN construction.
    Each pass declares invalidated analyses, which are recomputed before the
    next pass or compiler stage.

## 4. Final CLI and source-set contract

### 4.1 Final command shape

Use positional roots and repeatable library paths:

```text
antlr4-rust-gen [OPTIONS] ROOT.g4...

Inputs:
  ROOT.g4...                       One or more explicit grammar roots
  -I, --lib DIR                    Add an import/token-vocabulary lookup directory

Outputs and existing policies:
  --out-dir DIR
  --actions embedded|templates
  --sem-patterns FILE
  --sem-unknown error|hook|assume-true|assume-false
  --option-hook KEY=VALUE
  --require-full-semantics
  --require-generated-parser
  --allow-unsupported-lexer-actions
```

`--lexer`, `--parser`, `--lexer-name`, `--parser-name`, and the old
supplementary meaning of `--grammar` do not exist in the final CLI.

Phases A and B expose no public direct-mode CLI. Phase C changes the command
once, updates its tests/docs, and removes the old flags in the same review.

### 4.2 Root and dependency semantics

- An explicit root produces output. A grammar loaded only as an import is
  merged according to ANTLR semantics and is not emitted separately unless it
  is also an explicit root.
- Paths are canonicalized for identity, but diagnostics retain the spelling
  supplied by the user and emitted metadata uses a deterministic logical name.
- A grammar name maps to exactly one source file in an invocation. Two
  different files declaring the same grammar name are an error even if their
  contents match.
- Imports resolve relative to the importing file first, then repeatable
  `--lib` directories in command order. The first existing candidate wins,
  matching an explicitly ordered search rather than filesystem iteration;
  lookup provenance records the selected path and any shadowed candidates.
  A selected file whose declaration does not match the imported grammar name
  is an error.
- Import cycles, incompatible grammar-type imports, and missing imports are
  source diagnostics with the complete dependency chain.
- `tokenVocab=Foo` first binds to an explicit or discovered source-backed
  vocabulary producer named `Foo`: a lexer grammar or the implicit lexer of a
  combined grammar. That producer is compiled first in memory. A parser
  grammar cannot be a vocabulary producer, and a dependency-only producer
  emits no recognizer unless it is also an explicit root.
- Source discovery for `Foo` checks `Foo.g4` beside the referring grammar and
  then each `--lib` directory in order, applying the same declaration/name and
  shadow-provenance rules as imports.
- Source-backed vocabulary precedence is an intentional direct-compiler
  extension to Java's file-only `TokenVocabParser` path. When no source
  producer is in the invocation graph, standard `Foo.tokens` lookup follows
  the Java-compatible tiers: ordered `--lib` directories, an earlier staged
  output from the same invocation, then the importing grammar's directory.
  Within a tier, command order is authoritative and shadowed files are
  recorded rather than treated as an unordered ambiguity.
- Auxiliary `.tokens` files provide only names and assigned numbers. They are
  never grammar, source, or ATN authorities and cannot cause lexer code to be
  emitted.
- Token-vocabulary dependencies are topologically ordered independently of
  textual imports. Cycles and conflicting assigned numbers are errors.
- The loader caches by canonical path, so a diamond import is parsed once while
  each import edge retains its own source span.
- A failed invocation emits no new generated outputs. Output replacement keeps
  the generator's current behavior; redesigning output ownership or stale-file
  cleanup is outside this issue.

### 4.3 Grammar kind and output naming

- `lexer grammar L;` emits recognizer/module `L`.
- `parser grammar P;` emits recognizer/module `P`.
- `grammar G;` emits the conventional implicit recognizers `GLexer` and
  `GParser`.
- Names come from declarations, not filenames. A declaration/filename mismatch
  follows the pinned Java diagnostic contract.
- Module filenames continue to use `rust_names::module_name`.
- Output collisions after Rust-name normalization are diagnosed before writing.
- A combined grammar's parser is named `GParser` even if existing generated
  users referred to `G`. The direct frontend does not retain a naming override
  solely to emulate filename-derived `.interp` behavior.

## 5. Target architecture

### 5.1 Ownership boundary

The grammar compiler is codegen implementation detail:

```text
src/bin/antlr4-rust-gen.rs
  thin CLI/orchestration
          |
          v
src/bin_support/grammar/
  source -> syntax -> loader -> transforms -> semantics -> ATN -> Compilation
          |
          v
existing Rust emitter modules
```

Do not place the frontend in `src/lib.rs`:

- runtime users should not compile or see a grammar compiler API;
- exposing tool-model types would prematurely freeze them;
- the runtime's deliberately small public dependency surface should not absorb
  codegen-only parsing/Unicode/diagnostic concerns.

Do not introduce a separately published workspace crate initially. It would
complicate `cargo install antlr-rust-runtime --bin antlr4-rust-gen` and package
publication before there is a second real consumer. Private modules can be
extracted later if that need appears.

Only narrow, representation-level runtime changes are allowed:

- additions needed by `ParserAtnBuilder` to lower a completed graph;
- additions needed to construct/serialize `LexerAtn`;
- metadata constructor cleanup that distinguishes packed parser data from
  serialized lexer data.

### 5.2 Pipeline layers

```text
SourceSet
  source text, line index, lossless token stream, trivia, parsed CST
       |
       v
LoadedGrammarSet
  explicit roots, imports, tokenVocab edges, lookup provenance
       |
       v
IntegratedGrammarSet
  imports integrated, combined grammars split, stable IDs, editable units
       |
       v
TransformAnalysis
  name/import binding, call graph, nullability, side effects, vocabulary facts
       |
       v
TransformGrammar
  copied units + optional pass registry + invalidation + source map
       |
       v
CollectedGrammar
  rule declarations and alternatives + basic semantic checks
       |
       v
RewrittenGrammar
  immediate-LR rewrite with authored/synthetic provenance
       |
       v
SemanticGrammar
  symbols, numbering, options, attributes, labels, actions/predicates
       |
       +----------------------+
       |                      |
       v                      v
LexerAtnBuild             ParserAtnBuild
       |                      |
       v                      v
LexerAtn + DFA       finalized graph -> packed ParserAtn
       +----------+-----------+
                  v
        PostBuildAnalysis
  indirect-LR, epsilon, decision, LL(1), provenance coverage
                  |
                  v
             Compilation
                  |
                  v
       Rust modules + semantics.json
```

### 5.3 Stable identities and source data

Use dense typed newtypes rather than names or raw indexes:

```rust
SourceId
SyntaxId
GrammarId
RuleId
AlternativeId
ElementId
LabelId
ActionId
PredicateId
ModeId
TokenSymbolId
ChannelId
TransformId
BuildStateId
BuildTransitionId
```

Every authored syntax node stores:

```rust
SourceSpan {
    source: SourceId,
    bytes: Range<u32>,
}
```

`SourceFile` owns:

- immutable UTF-8 source text;
- a line-start index for byte-to-line/column diagnostics;
- every token, including off-channel whitespace/comments;
- token-to-byte spans and token channel;
- the lossless CST and syntax diagnostics.

Byte spans are canonical. Token indexes and line/column are derived views, not
competing coordinates.

### 5.4 Provenance

Model and ATN provenance must be structural:

```rust
enum Origin {
    Authored { syntax: SyntaxId },
    Imported { edge: ImportId, original: ModelNodeId },
    ImplicitLexer { combined: GrammarId, original: ModelNodeId },
    MandatoryTransform { kind: MandatoryTransform, inputs: Box<[ModelNodeId]> },
    OptionalTransform { pass: TransformId, inputs: Box<[ModelNodeId]> },
    LeftRecursion {
        rule: RuleId,
        original_alt: AlternativeId,
        role: LeftRecursionRole,
    },
    Synthetic { reason: SyntheticReason, owner: ModelNodeId },
}
```

The ATN factory records an `AtnOrigin` for every state and transition. When the
parser build graph is compacted into `ParserAtnBuilder`, it returns a build-ID
to packed-state mapping and remaps provenance once.

`ProvenanceIndex` is bidirectional:

- model/ATN/packed IDs map to one or more authored or synthetic origins;
- each authored `SyntaxId` maps to every surviving model and ATN node;
- a removed node has an explicit tombstone with phase, reason, and replacement
  IDs rather than silently disappearing;
- an optimizer merge unions all input origins, while a split records the same
  source origin on every result;
- build-state to packed-state and packed-state to build-state mappings are
  retained for diagnostics and differential snapshots.

Coverage invariants run after mandatory transforms, after each optional pass,
after ATN optimization, and after packing. Diamond imports and imported named
actions retain each import-edge origin, including multi-origin merges.

Consequences:

- an action transition is created from its `ActionId`; no source/ATN zip is
  needed;
- an LR precedence predicate is explicitly synthetic and points at its
  original alternative;
- rule-call arguments travel on `RuleCall` elements into the exact
  `RuleTransition`;
- labels and typed contexts remain available after ATN construction;
- diagnostics can report the original source even after imports and transforms.

### 5.5 Shared compilation artifact

The emitter receives typed artifacts, not a bag shaped like `.interp`:

```rust
struct Compilation {
    sources: SourceSet,
    roots: Vec<CompiledRoot>,
    diagnostics: Vec<Diagnostic>,
    transform_report: TransformReport,
}

struct CompiledLexer {
    recognizer: RecognizerModel,
    grammar: SemanticGrammarId,
    atn: LexerAtn,
    dfa: CompiledLexerDfa,
    runtime_artifact: LexerRuntimeArtifact,
    semantics: SemanticBindings,
    provenance: LexerAtnProvenance,
}

struct CompiledParser {
    recognizer: RecognizerModel,
    grammar: SemanticGrammarId,
    atn: ParserAtn,
    analysis: ParserAnalysis,
    semantics: SemanticBindings,
    provenance: ParserAtnProvenance,
}
```

`LexerRuntimeArtifact` is an output value containing the encoded words and
format required by generated lexer startup. It is not a compiler input or
semantic model. Phase B constructs it from the direct lexer graph and checks it
against the lexer ATN section in the committed `.interp` fixture. Phase C gives
that artifact to the existing renderer. The direct path never decodes its own
artifact.

`RecognizerModel` owns rule/name/vocabulary/channel/mode tables and typed rule
structure. It is constructed from semantic symbols, not parsed from text
sections.

Consumers use the narrowest view:

- metadata/constants renderer: `RecognizerModel`;
- lexer renderer: `CompiledLexer`;
- parser generated-rule compiler: `CompiledParser`;
- action/predicate/SemIR/manifest code: `SemanticBindings` plus source spans;
- typed contexts/listeners: typed rule/alternative/label model;
- entry-rule documentation: semantic rule-call graph;
- #128 reporting: transform report, source map, and before/after structural
  metrics.

No helper deserializes an ATN.

## 6. Bootstrap frontend

### 6.1 Vendored inputs

Vendor from antlr-ng commit `1f68422...`:

```text
third_party/antlr-v4-grammar/
  ANTLRv4Lexer.g4
  ANTLRv4Parser.g4
  predefined.tokens
  antlr-v4.toml
  LICENSE.txt
  README.md
```

`README.md` records:

- upstream repository and exact commit;
- local changes, including removal/replacement of the TypeScript-only
  `@header`;
- the bootstrap/regeneration command, including
  `--sem-patterns antlr-v4.toml`,
  `--option-hook superClass=LexerAdaptor`, `--sem-unknown error`, and the
  strict full-semantics/generated-parser flags established by #128;
- license provenance;
- expected generated-file hashes.

Keep the original per-file BSD notices. Do not silently copy antlr-ng's full
tool implementation or generated target code.

### 6.2 Checked-in generated Rust

Check in the generated frontend under:

```text
src/bin_support/grammar/generated/
  antlr_v4_lexer.rs
  antlr_v4_parser.rs
```

Add `frontend.rs` and `lexer_adaptor.rs` beside it. The adaptor implements the
grammar-specific behavior demonstrated in the #128 spike:

- classify `ID` as `TOKEN_REF` or `RULE_REF`;
- track whether lexing is outside a rule, in a lexer/parser rule, in a prequel
  construct, in rule options, or in a named action;
- choose `Argument` versus `LexerCharSet` mode at `[`;
- handle nested argument end behavior;
- reset adaptor state on lexer reset;
- forward generated typed action/predicate and lexer lifecycle callbacks.

This behavior is specific to parsing ANTLR grammar files and must not appear in
generic runtime or generic generator logic.

### 6.3 Fail-closed parsing

The frontend may use normal ANTLR recovery to collect useful diagnostics, but
the API returns no usable syntax tree if:

- the lexer reports any error;
- the parser reports any error;
- an action/argument/character-set token is unterminated;
- a token span cannot be mapped back to valid UTF-8 byte boundaries.

This is required by the #128 bootstrap result: nine valid grammar files had
byte-identical tokens and canonical trees against antlr-ng, while malformed
recovery trees differed. Transforming a recovered tree is forbidden.

### 6.4 Reproducible self-hosting

The final regeneration flow has three stages:

1. Stage 0 is the checked-in frontend.
2. Stage 0 compiles the vendored meta-grammars through the direct compiler and
   writes candidate Stage 1 generated Rust.
3. A temporary binary using Stage 1 recompiles the same inputs to Stage 2.

The update command requires Stage 1 and Stage 2 to be byte-identical after the
normal generated header. It then runs the frontend differential corpus before
updating checked-in files.

Phase A lands and validates Stage 0 only. Stage 1/2 cannot run until loading,
semantics, LR rewriting, both ATN factories, and structural code emission are
complete; the fixed-point gate runs at the end of Phase C.

For the initial bootstrap only, record and retain the pinned Java 4.13.2 output
used to seed Stage 0. Normal production and normal regeneration do not require
Java once self-hosting is established.

## 7. Required semantic and ATN behavior

### 7.1 Mandatory grammar transforms

These are compiler semantics, not optional #128 optimizations:

- load and type-check all imported grammars;
- integrate root/import token declarations, channels, named actions, modes,
  and rules in ANTLR's deterministic order;
- preserve root-rule override semantics and diagnose invalid duplicate
  definitions;
- reduce eligible blocks to sets exactly where ANTLR does;
- split a combined grammar into an implicit lexer and parser;
- copy only lexer-valid options/actions to the implicit lexer;
- synthesize implicit lexer rules for parser string literals in source order;
- process the implicit lexer first and import its vocabulary into the parser.

Mandatory transforms copy/construct model nodes with provenance. They do not
rewrite source text and reparse it.

### 7.2 Optional transform boundary for #128

Add a registry but no optimization passes:

```rust
trait GrammarTransform {
    fn name(&self) -> &'static str;
    fn safety_class(&self) -> SafetyClass;
    fn apply(
        &self,
        input: &TransformContext<'_>,
        grammar: &mut TransformGrammar,
        report: &mut TransformReport,
    ) -> Result<(), Diagnostic>;
}
```

The boundary runs after import integration, combined splitting, and a
preliminary `TransformAnalysis`, but before rule collection/basic checks,
left-recursion rewriting, final semantic numbering, and ATN construction.
The editable model keeps per-source structure and import-edge provenance even
though names and call boundaries are resolved.

`TransformAnalysis` exposes the facts needed by #129-#131 without implementing
those passes: resolved imports and references, the rule-call graph, direct and
indirect recursion components, nullability, action/predicate/attribute
side-effect inventory, labels/API surface, and vocabulary/mode facts. A pass
declares which facts it may invalidate. After every accepted mutation, model
validation runs and the affected analyses are recomputed before another pass
can inspect them. Final semantic analysis is always rebuilt from the
transformed copy; stale IDs, numbers, and call graphs cannot cross the
boundary.

The infrastructure must support:

- transform-a-copy;
- pass selection and deterministic ordering;
- report-only mode;
- tree/API-preserving versus recognition-preserving safety classes;
- node provenance and an original-source map;
- token-preserving transformed `.g4` rendering for audit;
- before/after structural metrics;
- idempotence checks;
- explicit analysis dependencies and invalidation/recomputation tests.

Actual factoring, inlining, and subsumption remain in #129-#131.

### 7.3 Semantic pipeline

Mirror Java/ANTLR ordering rather than combining checks opportunistically:

1. Collect rule declarations and alternatives.
2. Run basic grammar/type/option checks.
3. Classify and structurally rewrite immediate left recursion.
4. Define rules in stable source/import order.
5. Collect symbols, labels, attributes, actions, predicates, rule calls,
   terminals, modes, and commands.
6. Check name collisions, references, arguments, labels, modes, commands, and
   imported/qualified references.
7. Import or construct token vocabularies.
8. Assign token, literal, channel, mode, rule, action, and predicate numbers.
9. Validate attribute expressions sufficiently to preserve current
   semantic-hook/embedded-action behavior.
10. Build the semantic rule-call graph and identify entry rules.

The preliminary transform analysis is not a substitute for these stages.
Final symbol and token indexes are never assigned before the LR rewrite. After
ATN construction, the post-build analysis performs indirect-left-recursion,
epsilon, decision, and LL(1) checks before emission.

Numbering compatibility includes:

- `EOF` and invalid-token conventions;
- token definitions and literal aliases;
- non-fragment lexer rules, excluding `type(...)`/`more` cases as ANTLR does;
- conflicting literal aliases across modes;
- parser implicit-token/string diagnostics;
- default/user channels and modes;
- declaration/import order;
- per-rule action and predicate traversal order;
- deduplicated lexer action table order.

### 7.4 Structural left-recursion rewrite

Implement the algorithm over model nodes; do not render a target-language
StringTemplate rule and parse it back.

The rewrite must:

- detect immediate recursive references, including labeled references;
- classify primary/other, prefix, binary, ternary, and suffix alternatives;
- preserve original alternative order and labels;
- validate and retain left/right associativity;
- compute ANTLR-compatible precedence and next-precedence values;
- add precedence arguments to the correct rightmost recursive calls;
- create precedence predicates and required synthetic actions explicitly;
- retain returns/locals/arguments and action ownership;
- add `[0]` precedence to external calls of rewritten rules;
- report nonconforming recursion and no-primary-alternative cases;
- attach every synthesized node to the original rule/alternative provenance.

After ATN construction, detect indirect/mutual left-recursion cycles and stop
before emission.

### 7.5 Parser ATN

Use a generator-private mutable build graph because construction and
optimization require paired-state links, transition rewrites, and state
removal. This graph is not a serialized representation.

Implement:

- rule start/stop states and left-recursive markers;
- atoms, ranges, sets, negated sets, wildcard, EOF;
- rule calls with follow states and precedence;
- actions, semantic predicates, precedence predicates;
- alternatives, optional, plus, star, greedy/nongreedy ordering;
- labels as model/provenance data without changing transition semantics;
- rule-follow links and EOF links for entry rules;
- tail-epsilon/state compaction equivalent to the pinned tool;
- epsilon-closure/optional diagnostics;
- decision registration and LL(1) lookahead metadata.

Then lower once:

```text
ParserAtnBuild::finish
  -> FinalizedParserAtnGraph
  -> ParserAtnBuilder
  -> ParserAtn
```

There is no `Vec<i32>`, `SerializedAtn`, `.interp`, or deserialization in this
production path. Post-build analysis borrows the finalized graph; Phase C
generated-rule compilation borrows the one packed `ParserAtn` plus structural
actions, predicates, arguments, and labels. It may not call legacy positional
source/ATN correlation helpers.

Under test configuration only, the `.interp` serializer may borrow
`FinalizedParserAtnGraph` before lowering. Production has no serializer edge:
the same finalized graph goes directly to `ParserAtnBuilder`.

### 7.6 Lexer ATN

Implement:

- one decision start state per mode;
- source-ordered rule start/stop states and mode links;
- fragment behavior and rule-to-token mapping;
- Unicode scalar literals, ranges, sets, negation, wildcard, and EOF;
- ANTLR escape syntax and Unicode property escapes;
- grammar/rule `caseInsensitive` semantics and collision diagnostics;
- rule calls, actions, predicates, and lexer priority;
- standard commands: `skip`, `more`, `popMode`, `mode`, `pushMode`, `type`,
  and `channel`;
- Java-compatible warning diagnostics for duplicate/incompatible commands;
- custom action coordinates bound directly to `ActionId`;
- set collapse, tail epsilon removal, state compaction, epsilon-token checks.

ANTLR's Unicode property/alias data must be pinned and reproducibly generated
or vendored with provenance. Rust's standard library is not a substitute for
ANTLR's exact Unicode tables.

Unknown target-specific lexer commands do not invoke another target's
StringTemplate templates. They produce a Rust-target diagnostic unless an
explicit existing hook/pattern contract handles them.

The completed `LexerAtn` feeds:

- `CompiledLexerDfa::compile` once during codegen;
- semantic inventory and diagnostics;
- an output-only lexer ATN encoder while generated lexers still consume
  serialized runtime data.

The production compiler never serializes and then deserializes that data.
Replacing generated-lexer startup deserialization with a packed/static format
is a separately measured follow-up, not a prerequisite.

## 8. Review delivery plan

There are three review phases. They are review-size boundaries, not
compatibility stages or release milestones. Phases A and B add private code and
tests while leaving the current generator untouched. Phase C wires the new
compiler, breaks the CLI deliberately, and deletes production `.interp`
support in the same review.

### Phase A: check in the generated ANTLR grammar frontend

Deliver:

- Vendor the two meta-grammars, `predefined.tokens`, the two-entry
  `antlr-v4.toml` semantic-pattern configuration, licenses, and provenance.
- Check in the generated Rust lexer/parser.
- Implement the grammar-specific lexer adaptor and lifecycle forwarding.
- Produce `SourceFile`, lossless tokens/trivia, CST, source spans, and syntax
  diagnostics.
- Add the Stage 0 update harness and record the seed command, including
  `--sem-patterns`, `--option-hook superClass=LexerAdaptor`, and strict semantic
  flags.
- Capture the nine valid antlr-ng bootstrap grammars plus malformed fail-closed
  cases from #128 as frontend tests.
- Mirror the 12 tracked `.g4` files from pinned `vscode-antlr4`, preserving
  per-file notices and repository-level MIT attribution. Record every source
  hash and selected upstream assertion in
  `tests/codegen-direct/external-fixture-map.json`; do not copy generated
  TypeScript or the extension's historical `.interp` files.
- Generate `tests/codegen-direct/external-source-inventory.json` from a
  temporary checkout of the exact extension commit. Its required-artifact set
  is exactly `License.txt` plus the 12 tracked `.g4` files returned by
  `git ls-tree`; record a stable source ID, path, mode, Git blob ID, and SHA-256
  for each, plus repository URL and commit. The fixture map consumes that set
  as an exact partition: its top-level repository-license record owns the
  `License.txt` source ID, and its fixture rows own every `.g4` source ID
  exactly once. The pin-refresh command fetches and verifies the exact commit;
  ordinary CI compares the checked-in mirror/map to this manifest offline.
- Port only frontend-relevant extension assertions: exact source spans for
  named action, rule, and argument blocks, plus the `a::` malformed in-memory
  edit from `symbol-info.spec.ts`. Re-express its expected
  token/tree/diagnostic observables against the pinned antlr-ng frontend rather
  than copying editor-internal symbol kinds. The second edit, `a: b | c`,
  is syntactically valid: Phase A must return its tree, while Phase B asserts
  the two undefined-`b` semantic diagnostics. Split-grammar dependency
  ownership also starts in Phase B with the source-set loader.
- Mechanically generate
  `tests/codegen-direct/upstream-case-inventory.json` from the pinned Java and
  antlr-ng revisions by reconciling source extraction with pinned JUnit/Vitest
  runner discovery. Include enabled, disabled/skipped, inherited,
  parameterized, dynamic, loop/table-driven, and `it.each` instances with
  stable IDs, suite/name/parameters, status, source location, and source hash.
- Create `tests/codegen-direct/upstream-test-map.json` whose referenced
  upstream source-case ID sets form an exact partition of that inventory. A
  shared logical row may group multiple equivalent Java and antlr-ng source
  cases. Assign every logical case a Rust test/fixture, later phase, existing
  coverage, consult-only, or out-of-scope disposition; every non-port
  disposition records a rationale, evidence/covering test, and reviewer.
- Add the test-map validator and durable per-case port-evidence format described
  in sections 11.5 and 11.6.
- Port the Phase A rows: `TestASTStructure`, grammar-syntax cases from
  `TestToolSyntaxErrors`, source-frontend cases from antlr-ng
  `bugs/General.spec.ts`, and the bootstrap grammar corpus.
- For every Phase A `port` row, land the reviewed test-only commit and record
  its demonstrated red run before landing the corresponding frontend
  implementation commit. Test and implementation ports may be batched by a
  coherent frontend unit, but their commit boundary and per-case evidence must
  remain visible.

Gate:

- Complete token streams `(type, channel, byte span, text)` and canonical parse
  trees match pinned antlr-ng for all nine bootstrap files.
- All 12 pinned `vscode-antlr4` grammars match pinned antlr-ng token/tree
  snapshots even when later semantic analysis is expected to fail; only the
  `a::` edit fails closed, while the undefined-`b` edit yields a usable tree.
- Both meta-grammars self-parse.
- Malformed inputs always abort and never yield a transformable tree.
- Stage 0 frontend tests run with Java and Node absent from `PATH`.
- The checked-in generated files reproduce from the recorded seed command.
- Every active Phase A port record has reached `green` through
  the section 11.6 protocol, including a locked test-port commit and red
  evidence.
- Every mapped case has an explicit disposition and Phase A/B/C owner; later
  phase `port` rows need not be green at the Phase A gate.
- The test-map validator proves exact source-case coverage of the pinned
  upstream inventory, valid dispositions/evidence, allowed states, and valid
  closure hashes.
- The external-fixture validator proves an exact partition of all 13 pinned
  `vscode-antlr4` inventory artifacts (12 grammar sources plus
  `License.txt`), exact blob/source hashes, applicable license records, TDD
  owners, and explicit phase ownership.
- No Phase A implementation commit edits its locked oracle closure.
- Existing `cargo test --locked`, clippy, and `357/357/0` conformance remain
  green.

Stage 0 is intentionally seeded with the current Java/legacy toolchain.
Self-hosting is not claimed until Phase C can emit Rust from the direct ATN.

### Phase B: implement the complete source-to-ATN compiler

This phase owns everything from loaded `.g4` files through optimized lexer and
parser ATNs, typed semantic bindings, analyses, provenance, packed parser data,
and the lexer runtime artifact. It does not call the production renderers or
change the public CLI.

The primary oracle is a checked-in fixture set:

```text
tests/codegen-direct/fixtures/<case>/
  *.g4                       root and imported sources
  *.tokens                   auxiliary input/output where applicable
  *.interp                   Java 4.13.2 expected recognizer metadata
  diagnostics.json           expected errors/warnings where applicable
  fixture.json               roots, upstream test IDs, tool versions, hashes
```

The `vscode-antlr4` inputs use that same format. Expected `.tokens`, `.interp`,
and compiler diagnostics are regenerated with Java 4.13.2; they are not copied
from the extension. `fixture.json` additionally records the extension commit,
source path/hash, applicable license, selected upstream test location, raw
antlr-ng outcome, and whether the two oracles agree. The three alternate
meta-grammar files, `CPP14`, `OddExpr`, the split pair, `sentences`, `t`, `t2`,
and the missing-`tokenVocab` pair are mandatory Phase B rows.

A test-only `.interp` serializer accepts `RecognizerModel` plus the finalized
parser graph or `LexerAtn` and emits the standard sections and serialized ATN.
Routine tests need no Java:

1. compile the fixture `.g4` directly;
2. serialize the direct result in test code;
3. compare literal/symbolic/rule/channel/mode tables and ATN integers with the
   committed Java `.interp`;
4. lower the same parser graph directly through `ParserAtnBuilder` and compare
   it with the packed result obtained from the committed `.interp`;
5. encode the same lexer graph into `LexerRuntimeArtifact`, deserialize only
   inside the test, and compare token behavior/DFA fallback.

`.interp` proves vocabulary, numbering, ATN structure, and serialization, but
it does not contain authored body text, source spans, labels, or provenance.
Focused model snapshots assert those structural bindings separately.

The serializer is never a production intermediate. Java is used only by an
explicit fixture-update command pinned to ANTLR 4.13.2 and an exact JDK.
Reviewed state-renumbering differences may use normalized graph comparison,
but exact fixture equality remains the default.

The headings below are implementation order inside Phase B, not separate
merge, release, or compatibility phases. Phase B is reviewed and accepted as
one source-to-ATN unit with the single end gate below.

#### B.1 Source-set loader and dependency graph

Changes:

- Implement positional roots, repeatable `--lib`, canonical file identity, and
  deterministic lookup in a non-default direct test entry point.
- Parse each source once and build import plus `tokenVocab` graphs.
- Implement source-backed vocabulary producers and Java-compatible fallback
  `.tokens` lookup tiers without admitting `.interp`.
- Diagnose missing imports, cycles, duplicate grammar names,
  declaration/filename mismatch, and incompatible grammar kinds.

Checks:

- Root/import/tokenVocab fixture matrices match the defined lookup contract,
  Java ordering where applicable, and diagnostic source spans.
- The alternate `ANTLRv4Lexer.g4` imports `LexBasic.g4` exactly once, and
  `ANTLRv4Parser.g4` binds its lexer vocabulary from the resolved source graph
  without reading the extension's checked-in `.tokens` or `.interp`.
- The valid undefined-`b` edit from `symbol-info.spec.ts` reaches semantics
  with a usable CST and reports both original-source reference spans.
- Source-vs-`.tokens`, repeated-`--lib`, shadowed-candidate, parser-as-producer,
  and dependency-only producer cases have explicit expected results.
- Diamond imports parse shared files once.
- Reordering directory entries cannot change model order or output hashes.

#### B.2 Integrated model, transform analysis, and provenance

Changes:

- Convert CST nodes into typed grammar units with stable IDs.
- Integrate imports with root override semantics.
- Implement block-to-set reduction and combined-grammar splitting.
- Synthesize implicit literal lexer rules.
- Build preliminary import/name binding, call graph, recursion, nullability,
  side-effect/API, and vocabulary facts over the editable integrated model.
- Add the no-op optional transform registry, declared analysis invalidation,
  source map, transform report, and token-preserving transformed-source
  renderer.
- Implement bidirectional provenance, optimizer-ready merge/split rules,
  tombstones, and model validation.

Checks:

- Parser, lexer, combined, import, override, mode, named-action, and literal-split
  models match pinned Java/antlr-ng canonical fixtures.
- No model node lacks forward provenance, and every authored node has a
  surviving reverse mapping or an explained tombstone.
- Diamond imports and merged imported named actions retain all edge origins.
- A no-op transform is deterministic and idempotent.
- A test-only mutation pass proves declared analyses are invalidated and
  recomputed before another pass observes the model.
- Re-rendered no-op source is byte-identical.

#### B.3 Rule collection and basic semantic checks

Changes:

- Collect rule declarations and alternatives in stable source/import order.
- Run Java-compatible basic grammar, rule, type, option, and prequel checks.
- Stop this pipeline on errors before LR rewriting, symbol numbering, or ATN
  construction.

Checks:

- Applicable basic-error fixtures match Java severity/category and primary
  source construct; exact prose may differ only where documented.
- Warning-only fixtures continue to the next stage and retain their warning.
- No ATN is constructed after a semantic error.

#### B.4 Structural left-recursion rewrite

Changes:

- Implement LR classification, associativity, precedence, rewrite, synthetic
  nodes, and call-site precedence.
- Store original-to-rewritten alternative and label maps.
- Replace synthetic-action inference in direct-mode semantic inventory with
  explicit provenance.

Checks:

- Canonical rewritten models match Java/antlr-ng across binary, ternary, suffix,
  prefix, labeled, attributed, actionful, and right-associative cases.
- Nonconforming immediate-recursion and no-primary cases stop with diagnostics.
- Existing `LeftRecursion/*` conformance cases stay green through the legacy
  path; action-free direct fixtures match their rewritten structure.

#### B.5 Full symbols, semantics, and deterministic numbering

Changes:

- Define rewritten rules, then collect symbols, labels, attributes, actions,
  predicates, rule calls, terminals, modes, channels, and commands.
- Resolve references and run symbol/attribute/action checks.
- Import source-backed or auxiliary vocabularies and process the implicit
  lexer before its combined parser.
- Assign token, literal, channel, mode, rule, action, and predicate numbers in
  Java-compatible order.
- Build final structural `SemanticBindings` and the semantic rule-call graph.

Checks:

- `.tokens` text and every `.interp` name section emitted by test-only oracle
  serialization match Java exactly for the applicable matrix.
- Applicable errors and warnings match Java category and severity; Rust-only
  target-body policy failures are identified separately.
- Actions, predicates, calls, labels, and attributes have stable structural
  owners; no count/offset inference is used in direct semantic output.
- No ATN is constructed after a semantic error.

#### B.6 Direct parser ATN factory, packed lowering, and analysis

Changes:

- Implement parser build graph, factory, optimizer, indirect-recursion checks,
  epsilon checks, LL(1) analysis, and bidirectional provenance.
- Lower once through `ParserAtnBuilder`.
- Store the finished direct ATN and structural
  rule/action/predicate/argument/label bindings in `CompiledParser` for Phase C.
- Add only minimal builder/runtime APIs proven necessary by the lowering.

Checks:

- Canonical states, transition order/data, decisions, side tables, and LL(1)
  lookahead match Java for focused and real grammar fixtures.
- `CPP14`, `OddExpr`, and the Java-accepted `TParser` fixture match their
  committed parser `.interp` exactly before any normalized exception is
  considered.
- The alternate `ANTLRv4Parser.g4` serializer matches its Java parser
  `.interp` exactly.
- Test-only serialized parser ATN values match Java where exact state ordering
  is expected; any non-byte-identical case must have an equivalent normalized
  graph and a reviewed reason.
- Packed words match legacy only when build-state order is equal. An approved
  state-renumbering case must instead prove normalized graph equivalence,
  deterministic valid packed data, and identical behavior; normalization may
  not be used to claim exact-word equality.
- Indirect/mutual LR is diagnosed only after the graph needed by the
  post-build analysis exists.
- The direct parser host path has no `SerializedAtn` or deserializer call.
- Phase B parser tests call no positional source/ATN correlation helper.

#### B.7 Direct lexer ATN factory, artifact encoding, and analysis

Changes:

- Implement lexer factory, Unicode/escape/case-insensitive handling, commands,
  optimization, checks, and provenance.
- Compile the lexer DFA from the direct graph.
- Construct `LexerRuntimeArtifact` with an output-only encoder for the
  generated runtime's current lexer ATN representation.
- Store custom actions/predicates/commands as structural bindings for Phase C.

Checks:

- Lexer states, transition order, modes, actions, token mapping, and serialized
  values match Java across focused fixtures.
- `OddExpr` constructs and serializes all 101,246 lexer states without a
  16-bit target-code limit, and its 3,811,286-byte Java fixture compares
  exactly without an abbreviated or hash-only assertion.
- `CPP14` and `TLexer` cover ordinary large and command/mode/channel lexers.
  The serializer must reproduce Java's explicit `null` channel holes even
  though antlr-ng's textual writer leaves those entries blank.
- `LexBasic` and the alternate `ANTLRv4Lexer` match their Java `.interp`
  fixtures exactly; the latter explicitly gates the same Java-`null` versus
  antlr-ng-blank channel-slot divergence.
- `sentences` uses the Java 4.13.2 Unicode-property expansion as the expected
  ATN. The antlr-ng Unicode-16 ATN is retained as divergence evidence, not
  accepted as a normalized equivalent.
- Token streams match for valid and invalid Unicode, modes, commands,
  priority, predicates, and action cases.
- Warning-only nullable-token and duplicate/incompatible-command fixtures emit
  artifacts with Java-compatible warning severities.
- Direct codegen builds the DFA without encoding/decoding its own ATN.
- Generated-lexer interpreted fallback and compiled DFA remain equivalent.
- Direct lexer generation calls no positional source/ATN correlation helper.

Phase B end gate:

- Every valid fixture serializes to its committed Java `.interp` exactly, or
  has one reviewed normalized-state exception with identical behavior.
- Every invalid/warning fixture matches the expected diagnostic category,
  severity, and source span.
- Direct parser packing and lexer artifact encoding are tested from the same
  in-memory ATNs; neither production artifact is produced by reading the
  serialized test form.
- Every active Phase B port record has completed the section
  11.6 protocol and the focused grammar matrix in section 11 is green;
  escalated rows include the independently ported alternate test and, when
  required, alternate implementation evidence.
- Both upstream and external map validators pass at the Phase B head.
- No implementation/debugging commit edits a locked oracle closure without the
  oracle-review re-port and new red proof required by section 11.6.
- No Phase B module is reachable from the public generator CLI or production
  renderer yet.

### Phase C: wire the compiler and remove the old path

Phase C is one intentional breaking cutover. It connects `Compilation` to the
existing Rust emitters, switches the repository harnesses, removes the old
input flags and positional source correlation, and finishes self-hosting.
There is no preview mode or compatibility release.

#### C.0 Lock wiring and generated-behavior tests

Before changing a renderer, source reader, harness, or CLI:

- Port every active Phase C-owned port record assigned to the
  unit being wired, whether it is owned by `upstream-test-map.json` or an
  external assertion, using the section 11.6 isolation protocol.
- Add direct `.g4` CLI, renderer, generated-source, and generated-code execution
  tests for issue #141 behavior not represented by a port record.
- Run each test against the Phase B `Compilation` plus the unwired production
  boundary and record the expected failure fingerprint.
- Review and lock the complete test closure in a test-only commit before its
  C.1/C.2/C.4/C.6 implementation commit.

Phase C may batch coherent test-only commits, but implementation for a batch
does not start until every case in that batch is locked and demonstrably red.
This is commit ordering inside the Phase C review, not another delivery phase.
Both map validators run before each batch starts and after its evidence is
recorded.

#### C.1 Wire every renderer/source reader to `Compilation`

Changes:

- Split CLI/orchestration from rendering in the 16K-line binary.
- Make renderer entry points accept `CompiledLexer`/`CompiledParser` from the
  shared `Compilation`.
- Grammar options and warnings consume option nodes/spans.
- `semantics.json` consumes structural actions/predicates and provenance.
- SemIR and semantic-pattern matching receive opaque body text plus owner/span,
  rather than scan the whole grammar.
- Embedded Rust receives rule/attribute/action models.
- Rule-call arguments attach to exact call elements/transitions.
- Typed contexts/listeners receive labels and element structure outside
  embedded-only mode.
- Entry-rule docs consume the semantic call graph.
- Retire grammar-structural parsing from `embedded.rs`; retain only
  target-body translation that is genuinely needed.
- Retire grammar-wide action/predicate scans from `templates.rs`; retain only
  StringTemplate/body parsing helpers.

Checks:

- Every row in issue #141's current `--grammar` inventory has a direct model
  owner and focused test.
- Direct mode contains no positional source/ATN pairing.
- Core parser/lexer emission uses structural binding from Phase B;
  this phase removes the remaining source scanners rather than postponing that
  prerequisite.
- Authored, empty, unknown, and LR-synthetic actions have correct manifest
  dispositions without count inference.
- Typed label/context snapshots are available in standard mode for #138.

#### C.2 Establish the self-hosting fixed point

Changes:

- Use the completed direct compiler and recorded Stage 0 configuration to
  compile the vendored meta-grammars into candidate Stage 1.
- Build a temporary Stage 1 frontend and regenerate Stage 2.
- Normalize only the standard generated header; compare every other byte.
- Run the nine-file frontend corpus and malformed fail-closed cases with the
  candidate frontend before replacing checked-in files.

Checks:

- Stage 1 equals Stage 2 after the documented header normalization.
- The checked-in frontend equals the accepted fixed point.
- Regeneration succeeds with Java and Node absent from `PATH`.
- `antlr-v4.toml`, `superClass=LexerAdaptor`, semantic strictness, and
  lifecycle forwarding are all represented in the reproducible command.

#### C.3 Run the final differential before deletion

Changes:

- Run direct and legacy frontends over the same fixture/source set in an
  internal differential harness.
- Compare all intermediate layers, not only final Rust text.
- Add normalized approved-difference records for intentional naming or
  representation changes.
- Run behavioral token/parse/error differentials against Java-generated
  oracles.

Checks:

- The complete matrix in section 11 is green.
- Every active Phase C port record has completed the section
  11.6 protocol, including behavioral cases and generated-code execution; no
  row remains in a mapped, red, escalation, debugging, or blocked state.
- The test-map validator passes at the Phase C head.
- The external-fixture validator passes at the Phase C head.
- No unexplained model, number, ATN, artifact, token, tree, or error delta
  remains.
- Direct output is deterministic across two clean runs and different absolute
  checkout paths.
- Production input is switched only after this check is green in the same
  review.

#### C.4 Switch repository harnesses to direct `.g4`

Runtime testsuite:

- Keep StringTemplate rendering of descriptor `.g4` test input.
- Stop invoking Java ANTLR for Rust metadata.
- Pass the real root file and import directory to `antlr4-rust-gen`.
- Remove synthetic grammar concatenation and import-name text scanning.
- Remove `unsupported_reason`/composite whitelisting once all descriptors use
  the resolved graph.
- Make any skipped descriptor a harness failure.

Parity scripts:

- Kotlin Rust generation consumes `KotlinLexer.g4`,
  `UnicodeClasses.g4`, and `KotlinParser.g4` directly.
- JavaScript and TypeScript Rust generation consumes their lexer/parser roots
  directly.
- The ANTLR jar may remain only to generate Python/Java comparison targets, in
  a separate oracle directory.
- Assert the Rust-generation command has no `.interp` input.

Parse benchmark:

- Split `tools/parse-bench/run.py:277-366` so Java/Go/Python oracle generation
  and Rust generation are independent.
- Rust receives grammar roots and no `interp_dir`.
- Keep parser timing methodology unchanged so the codegen cutover does not reset
  runtime baselines.

Checks:

- Direct path: `357 passed, 0 failed, 0 skipped`.
- Kotlin, JavaScript, and TypeScript tree parity are byte-identical.
- Parse-bench AST parity passes for Kotlin, Java, and Trino.
- Quick parser benchmark has no protected runtime regression beyond the
  existing 1.15 threshold.
- CI jobs use the direct path for Rust generation.

#### C.5 Use same-org repositories as pre-acceptance integration tests

Run the Phase C branch against the two repositories found by GitHub search:

- mehen exercises split Kotlin/Java grammars, imports such as
  `UnicodeClasses.g4`, semantic hooks, and checked-in generated drift;
- gremlin-rs exercises a large combined grammar and the intentional
  `Gremlin` to `GremlinParser` generated-name break.

Before C.6 is accepted, author its complete source-only CLI/deletion candidate
commit on the Phase C branch without merging it. Prepare concrete downstream
commits or PR heads that pin that exact candidate SHA and update their generator
invocations and generated sources. Run each downstream's tests and record both
SHAs plus command/result in this review. Any change to the candidate SHA,
including a fix discovered by either downstream, invalidates both runs and
requires refreshed downstream heads and reruns. Only an exact candidate with
both recorded green results may pass C.5 and be accepted as C.6. This satisfies
the issue's known-consumer integration criterion before the cutover is
accepted.

Their current APIs, runtime versions, and MSRVs are not compatibility
requirements. A failure is evidence of a compiler or test-coverage gap to fix;
it is not a reason to preserve the old CLI or add an adapter. The prepared
downstream commits need no coordinated release order and may merge later
because their existing exact pins continue to select the old runtime.

#### C.6 Switch the CLI and delete production `.interp`

Author the complete change below as the candidate consumed by C.5. Do not
accept or merge it until both downstream heads pass against that exact commit;
after any candidate edit, repeat C.5. This is an integration gate inside the
single Phase C review, not a preview release or coordinated merge sequence.

In-repository:

- Rewrite README, Kotlin/JavaScript/TypeScript build docs, runtime-testsuite
  docs, and examples to show `.g4` roots only.
- Update CLI integration tests for positional roots, multiple roots, `--lib`,
  diagnostics, and help.

Delete:

- `Args.lexer`, `Args.parser`, `Args.grammar`, lexer/parser name overrides, and
  corresponding help/tests;
- `InterpData`, `Section`, `parse_atn_values`, and production `.interp` reads;
- all source/ATN zip, count, offset, greedy-call matching, and synthetic-action
  inference functions;
- host-side generator imports of `SerializedAtn`/`AtnDeserializer`;
- runtime-testsuite synthetic source concatenation and Java metadata stage;
- `.interp` production documentation and examples.

Retain only:

- the test-only `.interp` fixture reader/serializer used by Phase B;
- generated-lexer runtime deserialization while that artifact format remains;
- fixture-update tooling that invokes pinned Java explicitly.

Final static gates:

- no `InterpData` in production code;
- no production parser of `.interp`;
- no public `--lexer`/`--parser` input;
- no generator helper that accepts unrelated source text plus an ATN and pairs
  them positionally;
- no Java/antlr-ng process launch in normal `antlr4-rust-gen`;
- no current docs describing `.interp` as supported production input.

Final dynamic gates:

- full clippy and test suite;
- direct `357/357/0` conformance;
- all parity and parse-bench AST gates;
- all pinned `vscode-antlr4` rows at their Phase C expected success/failure
  outcomes;
- self-host regeneration fixed point;
- mehen and gremlin-rs integration runs against the Phase C branch;
- measurements from section 13.

## 9. Proposed file ownership

Exact names can adjust during implementation, but ownership must remain this
clear:

```text
third_party/antlr-v4-grammar/
  ANTLRv4Lexer.g4
  ANTLRv4Parser.g4
  predefined.tokens
  antlr-v4.toml
  LICENSE.txt
  README.md

src/bin_support/grammar/
  mod.rs                 pipeline facade
  source.rs              files, text, tokens, trivia, spans, line index
  diagnostic.rs          stable diagnostic codes and source rendering
  syntax.rs              lossless CST facade and typed syntax access
  frontend.rs            generated parser invocation and fail-closed boundary
  lexer_adaptor.rs       ANTLRv4Lexer-specific state/lifecycle hooks
  loader.rs              roots, imports, --lib, tokenVocab graph
  model.rs               typed grammar/tool model and IDs
  provenance.rs          origin chains and source maps
  transform.rs           mandatory/optional transform framework
  transform_analysis.rs  names, calls, recursion, nullability, side effects
  semantics.rs           symbols, checks, numbering, call graph
  left_recursion.rs      structural rewrite
  unicode.rs             pinned ANTLR-compatible property/alias data facade
  generated/
    antlr_v4_lexer.rs
    antlr_v4_parser.rs
  atn/
    mod.rs
    build.rs             private mutable graph and handles
    parser.rs
    lexer.rs
    optimize.rs
    analysis.rs
    lexer_encode.rs
    interp_test.rs        test-only .interp serializer/fixture reader

src/bin_support/codegen/
  mod.rs                 renderer facade over Compilation
  metadata.rs
  lexer.rs
  parser.rs
  semantics.rs
  typed_tree.rs

tests/codegen-direct/
  fixtures/<case>/
    *.g4
    *.tokens
    *.interp
    diagnostics.json
    fixture.json
  port-evidence/<case>/
    index.json
    revisions/<revision-id>/
      manifest.json
      oracle-results/
      matrix-results/
  upstream-case-inventory.json
  upstream-test-map.json
  external/vscode-antlr4/
    License.txt
    <mirrored .g4 sources>
  external-source-inventory.json
  external-fixture-map.json
  approved-differences.json

tools/grammar-frontend/
  update.sh or equivalent reproducible updater
  update-interp-fixtures.sh
  inventory-upstream-tests
  validate-upstream-test-map
  update-vscode-antlr4-fixtures
  validate-external-fixture-map
  oracle/               pinned fixture regeneration, never production
```

Primary existing-file changes:

| File | Planned change |
| --- | --- |
| `src/bin/antlr4-rust-gen.rs` | shrink to source-only CLI, compile, and emit |
| `src/bin_support/embedded.rs` | consume structural model; keep only embedded-Rust body lowering |
| `src/bin_support/templates.rs` | keep body/template parsing; remove grammar-wide structural scans |
| `src/atn/parser_atn.rs` | minimal lowering APIs if the private build graph proves they are needed |
| `src/atn/mod.rs` | minimal lexer construction/encoding support |
| `src/generated.rs` | distinguish parser metadata from `LexerRuntimeArtifact` cleanly |
| `src/bin/antlr4-runtime-testsuite.rs` | direct roots/imports, no metadata Java stage or concatenation |
| `tests/*-parity/run.sh` | direct Rust generation |
| `tools/parse-bench/run.py` | direct Rust generation independent of oracle targets |
| `.github/workflows/*.yml` | keep pinned oracles, assert direct Rust path |
| `README.md`, `docs/*.md` | final `.g4`-only usage |

## 10. Checked-in `.interp` oracle design

### 10.1 Fixture generation

Java ANTLR 4.13.2 under the exact recorded JDK is normative for grammar
semantics, numbering, ATN shape, Unicode behavior, and diagnostic severity.
`tools/grammar-frontend/update-interp-fixtures.sh` regenerates a named fixture
directory from its `.g4` roots, writes `.tokens`/`.interp` outputs and expected
diagnostics, and updates `fixture.json` with:

- root arguments and library paths;
- logical IDs from `upstream-test-map.json`;
- ANTLR jar SHA-256 and release commit;
- JDK vendor/full build;
- source and generated-file hashes;
- external source repository/commit/path, applicable license, and source hash;
- raw antlr-ng command/outcome plus an agreement/divergence classification;
- raw stdout, stderr, and exit status for every oracle command;
- the exact regeneration command.

Generated fixtures are reviewed like source. CI never regenerates them and
does not need Java for Phase B unit tests. Pinned antlr-ng remains a readable
implementation reference and the Phase A token/tree oracle.

### 10.2 Test-only serializer contract

The serializer consumes `RecognizerModel` plus `FinalizedParserAtnGraph` or
`LexerAtn` and writes standard ANTLR `.interp` sections:

1. token literal names;
2. token symbolic names;
3. rule names;
4. lexer channels and modes where applicable;
5. serialized ATN integers.

It exists only under test/test-support configuration. Production parser
generation lowers the graph directly to `ParserAtnBuilder`; production lexer
generation encodes `LexerRuntimeArtifact` directly. Neither path serializes and
then deserializes `.interp`.

Each fixture test compares complete sections and ATN integers. It also compares
the direct packed parser against packing the committed Java ATN, and compares
direct lexer token behavior against the committed Java artifact. Separate
expected diagnostic files cover grammars for which Java emits no `.interp`.

### 10.3 Differences

Exact equality is the default. A state-renumbering exception must name the
fixture, show the old/new state mapping, prove normalized graph and runtime
behavior equivalence, and be recorded in `approved-differences.json`.
Normalization never substitutes for exact packed-word equality when state
numbering is equal.

The file also records intentional contract changes such as source-backed
`tokenVocab`, repeatable `--lib`, combined-parser naming, or diagnostic text.
An unlisted difference fails the Phase B or C gate.

Known third-source disagreements are inputs to this process, not pre-approved
Rust differences. In particular, `null` versus blank `.interp` channel slots,
the `t = .` semantic verdict, Unicode-property expansions, indirect-LR member
lists, and missing-`tokenVocab` wording are recorded with both raw upstream
outcomes. The direct compiler still matches Java 4.13.2 unless a separate
intentional difference is reviewed.

## 11. Test matrix

### 11.1 Grammar surface

| Area | Required cases |
| --- | --- |
| kinds | lexer, parser, combined, imported/delegate |
| prequel | options, imports, tokens, channels, named actions |
| rules | modifiers, args, returns, locals, throws, catch/finally |
| alternatives | labels, element/list labels, empty, nested groups |
| EBNF | optional/star/plus, greedy/nongreedy, nullable boundaries |
| atoms | token/rule refs, literals, ranges, sets, not-set, wildcard, EOF |
| lexer | modes, fragments, commands, actions, predicates, recursion |
| options | tokenVocab, caseInsensitive, contextSuperClass, superClass, target options |
| actions | init/after/members, rule actions, predicates with fail messages |
| Unicode | BMP/SMP literals, escapes, properties, negation, case folding |
| LR | primary/prefix/binary/ternary/suffix, left/right assoc, labels/actions/attrs |

### 11.2 Dependency graph

- sibling import and `--lib` import;
- ordered multiple imports;
- first-match and shadowed-candidate behavior across repeated `--lib`;
- alias import syntax;
- root override and two delegates defining the same rule;
- diamond import;
- direct and indirect cycles;
- missing import and declaration/name mismatch;
- invalid grammar-kind combinations;
- explicit, discovered, and dependency-only source `tokenVocab` producers;
- combined implicit lexer as a vocabulary producer;
- parser grammar rejected as a vocabulary producer;
- source-vs-auxiliary `.tokens` precedence;
- auxiliary `.tokens` lookup in library, staged-output, and importer tiers;
- conflicting token numbers;
- multiple explicit roots sharing dependencies;
- combined grammar plus imported lexer/parser grammars.

### 11.3 Error-producing grammars

- malformed declaration and missing semicolon/delimiter;
- unterminated string, action, argument, comment, and character set;
- duplicate/undefined rule and parser-rule reference from a lexer rule;
- token, channel, mode, label, and attribute conflicts classified as errors by
  the pinned matrix;
- missing/cyclic `tokenVocab`;
- invalid lexer command names/forms classified as errors by Java;
- invalid escape/range/property and empty set;
- epsilon closure errors;
- nonconforming immediate LR, no primary LR alt, and indirect LR cycle;
- target action body that the existing strict semantic policy cannot handle.

Every error case asserts:

- stable diagnostic code;
- Java-compatible error severity where the case is part of ANTLR semantics;
- primary original-source span;
- relevant secondary import/definition span;
- no emitted partial artifacts;
- no recovered-tree continuation.

### 11.4 Warning-only and target-policy cases

At minimum, preserve Java 4.13.2's warning behavior for:

- nullable non-fragment lexer rules (`EPSILON_TOKEN`);
- optional blocks containing an epsilon-capable alternative
  (`EPSILON_OPTIONAL`);
- unsupported option names/values (`ILLEGAL_OPTION`,
  `ILLEGAL_OPTION_VALUE`);
- duplicate lexer commands (`DUPLICATED_COMMAND`);
- incompatible lexer commands (`INCOMPATIBLE_COMMANDS`);
- every additional warning classified as applicable by
  `upstream-test-map.json`.

Each case asserts the diagnostic code and warning severity, continued ATN/code
generation, and a usable artifact. Rust target-body strictness failures are
tested in a separate target-policy table; they must not be reported as Java
grammar errors. If a future warnings-as-errors mode is added, it is explicit
opt-in and does not change parity defaults.

### 11.5 Upstream test port map

At the pinned revisions, Java has 46 `Test*.java` tool-test classes and
antlr-ng has same-named Vitest suites for 42 of them. The Java-only classes are
`TestFastQueue`, `TestPerformance`, `TestUnbufferedCharStream`, and
`TestUnbufferedTokenStream`. antlr-ng additionally has
`tests/bugs/General.spec.ts` and its separate performance harness.

Phase A's inventory generator reconciles source extraction against pinned
JUnit/Vitest runner discovery rather than counting only class/file names. It
emits stable IDs and hashes for enabled, disabled/skipped, inherited,
parameterized, dynamic, loop/table-driven, and `it.each` cases. A discrepancy
between source and runner discovery fails inventory generation until its
tool-specific expansion is modeled; it cannot become a silent map omission.
The checked-in inventory changes only through that generator with an explicit
source-pin update.

The map contains only the active projection: exactly one row per stable logical
case, with one `active_revision_id`. Historical revisions live only in the
append-only evidence ledger described in section 11.6; they are not map rows.
The validator requires the upstream source-case ID sets consumed by active map
rows to form an exact partition of the inventory. A shared logical row may
consume multiple Java and antlr-ng IDs that assert one observable, but no ID
may be missing, duplicated, or unknown; duplicate active logical IDs, wildcard
dispositions, and suite-level placeholders are also forbidden.

The 42 shared suites are one logical corpus, not two eager ports.
`upstream-test-map.json` is case-level and records:

- pinned Java and antlr-ng commits;
- stable logical case ID;
- unique active revision ID, resolved through the evidence ledger;
- Java file and test method, when present;
- antlr-ng file and test name, when present;
- owning Phase A/B/C or existing Rust suite;
- disposition: `port`, `consult`, `covered-existing`, or `out-of-scope`;
- Rust test or fixture path;
- primary/alternate test and implementation sources;
- locked primary and, when escalated, alternate test-port commits;
- demonstrated-red command/result, primary and alternate implementation
  commits, and current TDD state;
- prerequisites, unit under test, expected red-failure fingerprint, and the
  observable behavior that makes the primary and alternate tests equivalent;
- oracle-closure hash and isolated base/branch commits used for each port;
- whether upstream expectations agree and, if not, both expectations plus the
  Java-compatibility verdict and final resolution;
- for every `consult`, `covered-existing`, or `out-of-scope` row, a
  case-specific rationale, covering test/evidence where applicable, and
  approving reviewer.

Default ownership is:

- **Phase A, port:** `TestASTStructure`; grammar-lexer/parser cases from
  `TestToolSyntaxErrors`; source-frontend cases from antlr-ng
  `bugs/General.spec.ts`; the nine-file bootstrap corpus.
- **Phase B, port:** `TestATNConstruction`, `TestATNSerialization`,
  `TestBasicSemanticErrors`, `TestSymbolIssues`, `TestAttributeChecks`,
  `TestCompositeGrammars`, `TestTokenTypeAssignment`,
  `TestLeftRecursionToolIssues`, `TestErrorSets`, structural cases from
  `TestLexerActions`, remaining tool/semantic cases from
  `TestToolSyntaxErrors`, applicable compiler/ATN regressions from antlr-ng
  `bugs/General.spec.ts`, `TestScopeParsing`, `TestTokenPositionOptions`,
  `TestCharSupport`, `TestEscapeSequenceParsing`, `TestUnicodeData`,
  `TestUnicodeEscapes`, `TestUnicodeGrammar`, `TestVocabulary`,
  `TestTopologicalSort`, `TestGraphNodes`, and `TestLookaheadTrees`.
- **Phase C, port applicable behavior:** `TestAmbigParseTrees`,
  `TestATNInterpreter`, `TestATNLexerInterpreter`,
  `TestATNParserPrediction`, `TestGrammarParserInterpreter`,
  `TestParserInterpreter`, `TestParserExec`, behavioral lexer-action cases,
  and target-neutral expectations from `TestCodeGeneration`.
- **Consult or existing coverage:** `TestActionSplitter`,
  `TestActionTranslation`, `TestDollarParser`, `TestATNDeserialization`, and
  upstream target-source snapshots. Existing Rust action/body/runtime tests
  remain authoritative for Rust-specific behavior.
- **Out of scope as direct ports:** token-stream/container/parser-profiler/tree
  utility suites (`TestBufferedTokenStream`, `TestCommonTokenStream`,
  `TestIntervalSet`, `TestParseTreeMatcher`, `TestParserProfiler`,
  `TestXPath`, `TestUtils`), the Java-only queue/unbuffered-stream classes, and
  both upstream performance suites. Existing Rust runtime tests and section 13
  benchmarks cover those concerns.

A suite may split across phases at case granularity, especially
`TestToolSyntaxErrors`, `TestLexerActions`, and `TestCodeGeneration`. No suite
is considered covered merely because its file appears in the map: every case
must have a disposition. The 357 runtime descriptors remain a separate
behavioral gate, not a substitute for this source/tool test map.

### 11.6 Cross-source TDD and failure resolution

A **port record** is the revision named by `active_revision_id` from either a
`port` row in `upstream-test-map.json` or an external-only assertion's `tdd`
object in `external-fixture-map.json`. Historical revisions are ledger entries,
not map rows and not inputs to source-inventory partitioning. Every active port
record uses the same state, closure, evidence, and validation schema. It names
exact `primary_test_source` and `alternate_test_source` identities plus fixed
`primary_implementation_source: antlr-ng@1f68422...` and
`alternate_implementation_source: java-antlr@cc82115...` identities. The
ordinary shared-case test order is Java then antlr-ng. Any override must be one
of the explicitly allowed extension/single-source forms, state why the default
source cannot express that observable, and retain Java's compatibility verdict
where Java exposes it. Every active revision follows this state machine:

```text
mapped
  -> primary-test-ported
     -> verified-covered-existing -> done
     -> primary-test-locked-red
        -> primary-implementation-ported
           -> green -> done
           -> failed
              -> alternate-test-locked-red
              -> declared-oracle-outcomes-recorded
              -> primary-implementation-rerun
                 -> green -> done
                 -> failed
                    -> alternate-implementation-ported
                    -> full-matrix-recorded
                       -> either-port-green -> done
                       -> neither-port-green
                          -> last-resort-debugging
                             -> oracle-review
                                -> oracle-relocked
                                -> declared-oracle-outcomes-refreshed
                                -> unchanged-ports-rerun
                                   -> either-port-green -> done
                                   -> neither-port-green
                                      -> implementation-debugging
                             -> validated-oracles
                                -> implementation-debugging
                             -> unresolved-oracle-ambiguity -> blocked
                             -> implementation-debugging
                                -> debugged-green -> done
                                -> blocked
```

Isolation and commit rules:

1. **Port the test first.** The test-port task receives only the record's
   pinned `primary_test_source`, grammar inputs/expected outputs, and the local
   test-harness API, but neither implementation source nor candidate Rust
   implementation code. Java is the default primary source, not a hard-coded
   task input.
2. **Prove red before implementation.** The ported test must execute and fail
   at the declared unit under test with the recorded failure fingerprint while
   all case prerequisites remain green. A setup failure, unrelated panic, or
   generic `unimplemented` panic is not evidence for the missing behavior. A
   compile-fail red is allowed only when the missing API shape is itself the
   behavior under test and the expected diagnostic is recorded exactly;
   otherwise add a behavior-free scaffold that returns a typed unsupported
   result so the assertion reaches the intended boundary.
3. **Review and lock the test.** Confirm its inputs/assertions against the
   original declared-source execution or immutable generated fixture, commit
   it separately, and record the commit plus the exact failing command/result
   in the owning map record. A test-only commit is expected to be red and is
   not independently mergeable.
4. **Port the primary implementation.** A separate task/commit ports the
   smallest relevant unit from antlr-ng. It may read the locked test but may
   not edit it.
5. **Green ends the case.** Record the implementation source and passing Rust
   test. Do not port the alternate copies merely for duplication.

The clean-context boundary is an input contract, not just a request to ignore
already-read code:

- the primary-test task receives only the pinned source named by
  `primary_test_source`, its inputs/expected results, and the local harness API
  needed to express it;
- the primary-implementation task receives only the pinned antlr-ng
  implementation unit, the public Rust boundary, and the locked primary test;
- the alternate-test task receives only the pinned source named by
  `alternate_test_source`, its inputs/expected results, and the same local
  harness API, not the first Rust test or either candidate implementation;
- the alternate-implementation task receives only the corresponding pinned
  Java implementation unit, the public Rust boundary, and both locked tests,
  not the antlr-ng implementation port.

Use isolated worktrees/branches rooted at one recorded, behavior-free scaffold
commit:

```text
scaffold
  |-- primary-test              record.primary_test_source port only
  |-- alternate-test            record.alternate_test_source port only
  |-- primary-implementation    primary-test + antlr-ng implementation
  `-- alternate-implementation  both locked tests + Java implementation
```

The alternate-test branch is created directly from the scaffold and cannot
contain the primary Rust test or either candidate implementation. The
alternate-implementation branch starts from the scaffold with only the two
locked test commits applied; it cannot contain the primary implementation.
A temporary validation branch combines the locked tests with each candidate
implementation, runs the complete two-tests-by-two-implementations matrix, and
records every result. If the declared tests are observably different, the
recorded Java compatibility verdict identifies which matrix cells are required
to pass where Java exposes the observable; otherwise the record's reviewed
equivalence contract does so. Only the selected passing implementation and
validated required test closure alter the phase integration tree.

Evidence is durable, not dependent on temporary worktrees or agent transcripts.
`port-evidence/<case>/index.json` records the stable logical ID, all unique
revision IDs, the map-selected active revision, and append-only supersession
edges. Each `revisions/<revision-id>/manifest.json` records allowed inputs and
hashes, optional immediate `supersedes_revision_id`, scaffold SHA, every
branch/port commit and ancestry, exact commands/toolchains, raw declared-source
outcomes, red fingerprints, and matrix results. After resolution, a
no-tree-change evidence merge makes every isolated port commit a reachable
parent of the phase branch without applying rejected trees. The map validator
checks those parents and the committed evidence hashes. Reviewing an existing
commit does not count as an independent port; a new clean-context task must
produce the alternate port.

If the primary implementation does not pass:

1. **Port the alternate test before debugging.** In a fresh context that has
   not seen the first Rust test or candidate implementation, port the
   source named by `alternate_test_source` against the same public test
   harness. It must independently reach the declared unit and demonstrate its
   own fingerprinted red against the scaffold before it is locked. If it is
   not observably equivalent, correct the source assignment before any
   implementation debugging or use the single-source rule; do not label a
   merely similar passing test as independent evidence.
2. **Run every declared oracle on every escalation.** Execute both declared
   test sources over the same canonical input and normalized observable.
   Execute Java 4.13.2 and antlr-ng as well whenever they expose that
   observable; otherwise record a reviewed `not-applicable` result explaining
   the missing API. Commit exact toolchains, commands, raw outputs/diagnostics,
   and normalization. This determines whether each untouched Rust test port
   faithfully represents its source and whether the sources agree. Do not
   correct either Rust test or any oracle closure during this reconciliation.
   If Java and antlr-ng genuinely disagree, retain both observations; Java
   4.13.2 supplies the compatibility verdict unless an approved difference
   says otherwise.
3. **Rerun the unchanged primary implementation against the required faithful
   oracle set.** If it passes, select it and the faithful test closure and end
   the case; preserve any suspect test port only as evidence. If it still fails,
   port the alternate implementation whether the declared oracles agree or
   disagree. In another clean context/commit, port the smallest corresponding
   Java implementation unit. Do not debug or blend either implementation port.
4. **Run the complete matrix and choose by result.** Run both untouched
   implementation ports against both untouched test ports plus immutable Java
   fixtures where applicable. If either implementation passes every required
   faithful oracle, select that implementation unchanged and end the case. Do
   not debug or reconcile the failed port.
5. **Debug only after both ports fail.** Only when neither independently ported
   implementation passes the unchanged required oracles may
   `last-resort-debugging` begin on a test port, harness, model boundary,
   integration, or misunderstood upstream assumption.

The locked test closure comprises the Rust test, fixtures and expected values,
shared harness helpers, serializer/normalizer behavior, approved-difference
entry, declared test-source identities, and the row's source identity,
disposition, owner, prerequisites, equivalence claim, and failure fingerprint.
For `upstream:<logical-id>` ownership, the upstream row records the exact
reverse-linked external assertion IDs, and its closure transitively hashes each
linked assertion's source/inventory hash, canonical input, oracle commands,
expected/raw outcomes, normalization, and phase ownership. The external map
records the same owner and transitive closure hash. A `green`/`done` closure is
immutable. An approved source-pin, fixture, or linked-evidence update creates a
new ledger revision with a globally unique revision ID and closure hash, names
the immediate predecessor in `supersedes_revision_id`, and starts at `mapped`.
The active map row keeps its stable logical ID but switches
`active_revision_id` to that new leaf; the predecessor remains an immutable
historical `done` revision and does not re-enter the state machine. The
replacement executes this protocol from the start (or proves
`verified-covered-existing`) before the phase can pass. Implementation and
debugging commits may not change any part of a locked closure. Workflow-state
and evidence fields remain append-only.

No oracle-closure correction is allowed before `neither-port-green`. During
last-resort debugging, a suspected test defect triggers a fresh-context
independent oracle review from the record's declared test sources and raw
outcomes, without either candidate implementation as input. Any correction
requires a separate oracle-review commit, a new closure hash, and a new
fingerprinted result.
Normalization and `approved-differences.json` follow the same rule; an
implementation may not relax either to pass.

After an oracle is re-locked, rerun and re-normalize every declared test source
against the canonical input under the new closure hash, plus Java and antlr-ng
where they expose the observable. Commit the refreshed raw outcomes, then
rerun both unchanged implementation ports. If either Rust port is now green,
select it without implementation debugging. Only if both still fail may an
implementation/debugging commit change candidate code. That commit cannot
change the re-locked closure. It either reaches `debugged-green` or the row
becomes `blocked`; a blocked row is a phase hard stop, not a reason to weaken
the oracle.

Add both map validators to CI. They validate the shared port-record schema,
source pins, exact source-case or external-inventory coverage, case-specific
non-port evidence, allowed state transitions, required evidence per state,
closure hashes, bidirectional external-owner links and transitive hashes,
reachable evidence commits, Phase A/B/C ownership, and the rule that only
oracle-review commits after `neither-port-green` may alter a locked closure.
An `oracle-relocked` record cannot advance until every declared source outcome,
plus applicable Java and antlr-ng outcomes, is committed under the new hash.
The validators apply logical-ID uniqueness and exact source partitions only to
active map rows. For each logical case, they require globally unique revision
IDs, an acyclic connected supersession chain, at most one direct successor per
revision, exactly one leaf, and equality between that leaf, the ledger's active
pointer, and the map row's `active_revision_id`. Historical manifests remain
immutable and never consume inventory IDs a second time.
Run both validators at every phase gate.

If a newly ported primary test is already green because an earlier
implementation unit supplies the complete behavior, reclassify the row as
`covered-existing` through the `verified-covered-existing` transition. Record
the covering implementation/test and prove that the test reaches the intended
unit with its expected positive fingerprint. Do not manufacture a failure or
claim that the row completed the `port` protocol without a real red state.

Special cases:

- Java-generated `.interp`, `.tokens`, and diagnostics fixtures are immutable
  test oracles. They change only through the pinned fixture updater and review,
  never to make a Rust implementation pass.
- For an antlr-ng-only regression, port its TypeScript test first and use a
  generated Java fixture/diagnostic as the alternate oracle where possible.
- For a genuinely single-source case, commission an independent second port
  from the same test source before debugging, then use the alternate
  implementation source if one exists.
- Keep test, primary implementation, alternate test, alternate implementation,
  and any last-resort debugging as distinct review commits until the case is
  resolved. A passing alternate implementation is selected as-is; cleanup or
  refactoring happens only after the case is green and is not part of failure
  resolution.

### 11.7 Behavioral corpus

- all 357 upstream descriptors;
- all Kotlin parity snippets and scripts;
- all JavaScript parity snippets;
- all TypeScript parity snippets;
- parse-bench Kotlin, Java, Trino, and protected lexer stress fixtures;
- mehen Kotlin and Java checked-in generated parsers;
- gremlin-rs combined grammar and parser tests;
- the nine bootstrap grammars;
- all 12 pinned `vscode-antlr4` grammar sources in their assigned Phase A/B/C
  roles;
- focused valid/invalid source pairs for each ATN construct.

### 11.8 Curated `vscode-antlr4` corpus

`tests/codegen-direct/external-fixture-map.json` pins
`3e9469d1d490c71b3e3b909edf1235582a3f8db8`. Its source IDs and top-level
repository-license source ID must be an exact partition of all 13 required
artifacts in `external-source-inventory.json`: all 12 `.g4` paths and
`License.txt`. This partition uses active fixture rows only; historical
assertion revisions remain in the evidence ledger. Each fixture row records
its owned source ID, applicable repository/per-file license records, selected
extension test location if any, owning phase, Java/antlr-ng commands and raw
outcomes, expected Rust test, and whether the row is syntax-only, valid
compiler input, or an expected semantic failure.

Every external assertion in every fixture row has exactly one `tdd_owner`; a
single-assertion row may store it at row level:

- `upstream:<logical-id>` means the extension contributes only an input or
  scale case. The stable ID resolves exactly one active
  `upstream-test-map.json` row and its `active_revision_id`; that revision owns
  the behavior, locked closure, and section 11.6 evidence. Both active rows
  record the ownership link, and the upstream closure transitively hashes this
  external assertion's immutable input and oracle evidence. The external row
  may add corpus outcomes but cannot create a second, weaker port record.
- `external:<assertion-id>` means the extension contributes a unique
  observable not owned by an upstream logical case. The row contains a full
  `tdd` object with a stable assertion ID and `active_revision_id`, plus the
  same state, prerequisites, unit-under-test, failure fingerprint, closure
  hash, isolated commits, raw outcomes, matrix evidence, and terminal-state
  rules as every other section 11.6 port record.

Split a fixture's assertions into separately identified map entries when one
source exercises multiple implementation units with different TDD owners; its
source ID remains owned once by the parent fixture record, while each child
assertion names its own owner. The validator rejects a missing owner, a dangling
or one-way upstream logical ID, a duplicated external assertion ID, a mismatch
between owner and linked transitive closure hashes, or an
`external:<assertion-id>` without the complete shared port-record schema.

Oracle and port precedence is observable-specific:

- For serializer, ATN, numbering, Unicode, and semantic-diagnostic assertions,
  the Java 4.13.2 fixture/raw outcome is the primary test oracle. An extension
  grammar with no unique expected observable is input coverage and delegates
  through `upstream:<logical-id>`.
- An extension assertion is primary only for a unique frontend source-span or
  CST observable, including the pinned malformed-edit locations. Such a port
  record sets `primary_test_source` to the exact extension test/assertion and
  `alternate_test_source` to the independently generated antlr-ng
  token/tree/diagnostic oracle. Java is additionally recorded when it exposes
  the same normalized observable.
- Regardless of which test is primary, the primary implementation port comes
  from antlr-ng TypeScript. If escalation reaches the alternate implementation,
  port the corresponding Java unit. Java remains the compatibility verdict
  whenever upstream outcomes disagree.

Required use is:

1. Phase A parses and snapshots all 12 files, including the semantically
   invalid files, because syntax validity and compiler validity are separate.
   It ports the malformed `a::` edit, verifies that the valid undefined-`b`
   edit still yields a CST, and ports exact source-span assertions for named
   actions, rule bodies, and argument blocks.
2. Phase B uses Java-regenerated fixtures for the alternate meta-grammar trio,
   `CPP14`, `OddExpr`, the split pair, `sentences`, `t`, `t2`, and the
   missing-`tokenVocab` pair. It stores antlr-ng output beside fixture metadata
   only as a second observation. `t`, `t2`, and the missing-vocabulary pair are
   diagnostic fixtures: any partial files emitted by an upstream tool are raw
   evidence, not expected direct-compiler output.
3. Phase C runs every Java-valid row through the final source-only CLI and
   verifies emitted Rust plus packed/parser and lexer artifacts. Expected
   failures must emit no partial output.

These rows are locked test data before the corresponding compiler unit is
ported. A failure enters the section 11.6 protocol using the same canonical
input and the row's declared `tdd_owner`. The extension supplies either input
coverage or a narrowly identified source-location/CST assertion; it is not
automatically an independent test oracle. Each record's declared test sources
supply its observable-specific oracles; antlr-ng and Java still supply the
primary and alternate implementation evidence in that order. No extension
renderer, debugger/UI behavior, generated target source, random sentence
expectation, or historical tool crash becomes part of the compiler contract.

## 12. Downstream policy

Only mehen and gremlin-rs were found, both same-org and exact-pinned. Before
C.6 is accepted, its complete candidate commit is authored and concrete
downstream commits or PR heads update their exact pins, invocations, and
generated sources against that exact SHA as described in C.5. A candidate edit
invalidates the runs. Those downstream changes may merge independently or later
because the existing exact pins continue to select the old crate. They create
no compatibility, release, or coordinated merge-order work for this
repository.

## 13. Measurement plan

### 13.1 Codegen measurements

Compare on the same machine with interleaved A/B runs:

- current end-to-end Java 4.13.2 plus `.interp` plus Rust generator;
- current Rust generator alone with prebuilt `.interp`;
- direct Rust generator from `.g4`;
- direct Rust generator with no-op transform reporting enabled.

Measure:

- wall, user, and system time;
- peak RSS;
- bytes read and written;
- temporary bytes/files;
- emitted Rust, semantics manifest, packed parser ATN, lexer runtime artifact,
  and compiled DFA sizes;
- ATN states, transitions, decisions, and generated adaptive decisions;
- generated-lexer first-instance/startup time.

Use Kotlin, Java, JavaScript, TypeScript, C#, Trino, Gremlin, `CPP14`, and
`OddExpr` where their semantic support is already valid. Treat `OddExpr`
primarily as a peak-memory/output-scale case; do not let its unusually large
literal vocabulary dominate aggregate timing claims. Build once before timed
repetitions and do not overlap timing-sensitive runs.

### 13.2 Runtime guardrails

Run the existing same-machine parse methodology unchanged:

- token/tree/AST parity first;
- parse-only timings with process startup excluded;
- interleaved base/head samples;
- peak memory where the harness supports it.

The expected parser runtime result is neutral unless final ATN/artifact bytes
change. Report codegen gains and parser measurements in separate tables. A
smaller generator pipeline is not evidence of faster parsing.

### 13.3 Reporting requirements

Publish:

- machine/toolchain/commit details;
- raw JSON;
- sample counts and summary statistics;
- exact grammar revisions;
- all protected regressions and neutral results, not only wins.

Do not claim a codegen or runtime performance improvement until this evidence
exists.

## 14. Deletion and acceptance checklist

| Issue #141 acceptance criterion | Plan phase and proof |
| --- | --- |
| `.g4` roots are sufficient | Phase C CLI integration tests |
| no external production ANTLR | Phase C run with Java/Node absent |
| imports/combined/tokenVocab/numbering/semantics/LR/ATNs | Phase B committed `.interp` and diagnostic fixtures |
| all source responsibilities use one model | Phase C source-reader inventory |
| positional matching and synthetic concatenation removed | Phase C static deletion checks |
| parser ATN packed directly | Phase B direct-pack versus fixture-pack test; Phase C has no serialized intermediate |
| ATNs built/analyzed once | Phase B shared `Compilation`; Phase C renderer integration |
| #128 transform boundary | Phase B analysis/invalidation/source-map tests |
| labels/typed structure retained for #138 | Phase B model fixtures and Phase C generated snapshots |
| final CLI is source-only | Phase C CLI/help tests |
| no production `InterpData`/`.interp` | Phase C static and package audit |
| Java and antlr-ng source/tool tests | generated pinned inventory source IDs are consumed exactly once by the validated case-complete map across Phases A-C |
| pinned third-source fixture coverage | validated external fixture map accounts for all 12 `vscode-antlr4` grammar paths, licenses, hashes, oracle outcomes, and phase gates |
| test-first ports and ambiguity resolution | both map validators enforce the shared port-record schema for `upstream-test-map.json` ports and `external-fixture-map.json` external assertions: isolated bases, locked test closure/hash, failure fingerprint, red evidence, implementation commits, 2x2 escalation matrix when needed, and a valid terminal resolution |
| Java/antlr-ng/direct parity and zero-skip suites | Phase A frontend parity; Phase C conformance/parity |
| measured codegen effect, separate runtime claims | Phase C measurements and section 13 report |

## 15. Risks and controls

| Risk | Control |
| --- | --- |
| Scope becomes a wholesale antlr-ng port | Port only stages through ATN analysis; no target emitter, dependency generator, or unrelated CLI |
| Meta-grammar bootstrap is circular or drifts | Phase A lands Stage 0; Phase C proves Stage 1/2 only after direct emission exists; pin sources/config/hashes |
| Recovered grammar trees are subtly wrong | Any lexer/parser error invalidates the tree |
| Import or tokenVocab lookup changes rules/tokens | Explicit tier/order contract, lookup provenance, approved source-backed extension, and exact Java numbering snapshots |
| Combined split loses source identity | stable IDs, reverse provenance, tombstones, and explicit `ImplicitLexer` origins |
| LR rewrite changes labels/actions/precedence | structural algorithm plus rewritten-model and conformance differentials |
| Unicode behavior differs by Rust/toolchain/JDK version | pin ANTLR-compatible data and exact oracle JDK; test BMP/SMP/property cases |
| Direct ATN is behaviorally close but structurally different | committed Java `.interp` fixtures, test-only serializer, direct-pack comparison, then runtime tests |
| Existing body heuristics leak into the frontend | frontend treats bodies as opaque; existing SemIR/embedded consumers get owned body nodes |
| Temporary dual paths become permanent | Phases A/B do not expose a second CLI; Phase C wires and deletes the legacy production path together |
| Runtime crate absorbs compiler internals | keep frontend under `bin_support`; require evidence for each runtime API change |
| Combined recognizer naming breaks downstream code | accept the break; use gremlin-rs to validate the new generated API |
| Test serializer becomes production architecture | compile it only for tests/tooling and statically prohibit `.interp` use in the production generator |
| Optimizer removes source accountability | require reverse provenance, merge unions, tombstones, and coverage checks at every boundary |
| Optional transforms consume stale analysis | declare dependencies/invalidations and recompute after every accepted pass |
| Oracle availability makes CI flaky | pin checked-in snapshots; regenerate oracles in a separate job |
| A test and implementation repeat the same mistranslation from one source | isolated scaffold branches, the record's independently ported alternate test on failure, Java alternate implementation, then a recorded 2x2 matrix before debugging |
| A passing implementation weakens a harness or normalized exception | hash the complete oracle closure, validate map transitions in CI, and require a separate oracle-review commit plus new red proof |
| Rejected independent ports disappear with temporary branches | committed raw evidence plus a no-tree-change evidence merge keeps every port commit and ancestry reachable |
| Cases disappear behind broad `consult`/`out-of-scope` labels | generated pinned case inventory, exact set equality, and case-specific rationale/evidence/reviewer |
| Codegen benchmark is sold as runtime speedup | separate metrics/tables and require parse evidence for runtime claims |

## 16. Rollback rules and hard stop gates

- Phases A and B are private modules/tests. A failed review is revised or
  reverted without changing production behavior.
- Phase C is atomic from the user's perspective: source-only CLI in, legacy
  production path out. It does not ship a fallback or deprecation mode.
- A post-merge regression is fixed in the direct path or the Phase C commit is
  reverted. Same-org downstream repositories remain isolated by exact pins.
- Optional transforms remain off by default and can be disabled independently.
- Do not complete any phase while one of its active port
  records, including an external-owned assertion, is short of `green` or lacks
  its locked test-closure hash, demonstrated-red evidence, implementation
  commit, and required isolated-branch/matrix records.
- Do not complete a phase with a `blocked` row or a test-map/inventory validator
  failure.
- No part of a locked oracle closure, including harness/normalizer helpers,
  approved differences, or case disposition, can change from an implementation
  or debugging commit. A suspected bad test triggers the alternate
  clean-context test port; a persistent failure triggers the alternate
  clean-context implementation port and matrix before ordinary debugging is
  allowed.
- Do not complete Phase B while any unexplained `.interp`, diagnostic, packed
  parser, or lexer behavior delta exists.
- Do not merge Phase C by adding conformance skips, reclassifying an upstream
  warning as a fatal default error, weakening Rust target-body strictness, or
  allowing recovered syntax trees.
- Do not add grammar/language-specific workarounds to generic codegen/runtime
  paths to get a real-grammar gate green.

## 17. Adversarial review

An independent adversarial review of the completed first draft returned
`requires structural rewrite`. Its technical findings were incorporated; the
downstream-compatibility concern was resolved by the later project-policy
clarification rather than by preserving compatibility:

| Finding | Resolution in this plan |
| --- | --- |
| numbering preceded LR in architecture/phases; indirect LR was gated pre-ATN | Phase B orders basic checks, immediate LR, symbols/numbering, then ATN/post-build analysis |
| #128 transform boundary lacked the facts needed by #129-#131 | added preliminary import/name/call/nullability/side-effect/vocabulary analysis plus declared invalidation/recomputation |
| the frontend phase required an impossible self-host fixed point | Phase A checks in Stage 0; Stage 1/2 runs in Phase C after direct emission exists |
| Java warning-only grammars were classified as fatal | split error and warning matrices and required continued generation for Java warnings |
| parser/lexer behavioral gates preceded structural source binding | Phase B ATNs carry structural bindings; Phase C renderers consume them without positional matching |
| 357 runtime descriptors could not prove compiler semantics | the case-level dual-source test map assigns Java/antlr-ng cases to Phase A/B/C fixtures or explicit dispositions |
| `tokenVocab`/lookup precedence was contradictory | defined ordered import lookup, source-backed producer semantics, Java-compatible `.tokens` fallback tiers, and approved deviations |
| provenance was one-way and lost optimized/eliminated nodes | added reverse indexes, merge/split rules, tombstones, packed mappings, and boundary coverage gates |
| sibling runtime/API/MSRV compatibility was implicit | project policy explicitly accepts those breaks; mehen/gremlin-rs are Phase C test beds, not compatibility gates |
| the lexer model lacked its final runtime artifact | Phase B constructs `LexerRuntimeArtifact` directly and checks it against committed `.interp` fixtures |

The independent review of the explicit cross-source TDD protocol returned
`ready with minor fixes`. All findings were incorporated:

| Finding | Resolution in this plan |
| --- | --- |
| clean-context ports were impossible to prove on one linear branch | section 11.6 defines isolated scaffold branches, a validation branch, and a recorded two-test-by-two-implementation matrix |
| Phase C wiring could begin before its behavior tests were ported | C.0 locks each coherent test batch red before the corresponding implementation commit |
| any red failure could be mistaken for the intended missing behavior | each row declares prerequisites, unit under test, and an exact failure fingerprint |
| implementation commits could weaken helpers or normalized exceptions | the complete oracle closure is hashed; CI validates state transitions and only oracle-review commits may change it |
| a passing Java port still allowed premature reconciliation/debugging | the passing isolated port is selected unchanged; debugging starts only if neither port passes required oracles |
| the Phase A gate accidentally required later-phase ports to be non-port rows | Phase A requires ownership/disposition for all rows but `green` only for Phase A-owned ports |

The fresh follow-up review again returned `ready with minor fixes` and found one
remaining protocol blocker. It is also resolved:

| Finding | Resolution in this plan |
| --- | --- |
| a mistranslated test could be corrected before the alternate implementation port | reconciliation records outcomes without editing; no oracle closure changes until both isolated implementations fail unchanged required oracles |
| rejected port evidence depended on temporary branches/transcripts | committed input/result manifests plus a no-tree-change evidence merge retain every port and its ancestry |
| the state machine lacked covered, relock, debugging-exit, and blocked transitions | section 11.6 now names all of those states and makes `blocked` a phase hard stop |
| a claimed upstream disagreement did not always execute both pinned tools | every escalation runs both tools on one canonical input/observable and commits raw outcomes |
| suite counts did not mechanically prove case-complete dispositions | Phase A generates a hashed case inventory; the validator requires an exact source-ID partition and case-specific non-port evidence at every gate |

The final fresh-context review returned `ready with minor fixes` and explicitly
reported no blocking findings. Its two remaining corrections are incorporated:

| Finding | Resolution in this plan |
| --- | --- |
| source-only inventory could miss inherited, parameterized, skipped, dynamic, or table-driven runner cases | inventory generation reconciles source extraction with pinned JUnit/Vitest discovery and map rows partition source-case ID sets |
| oracle relock reran Rust ports without refreshing both upstream outcomes | `declared-oracle-outcomes-refreshed` is a validator-required state before unchanged Rust ports rerun |

The first adversarial pass over the supplemental `vscode-antlr4` corpus
returned `ready with blocking fixes`. Its findings are incorporated:

| Finding | Resolution in this plan |
| --- | --- |
| external fixtures had no unambiguous owner in the cross-source TDD protocol | every external assertion declares `upstream:<logical-id>` or `external:<assertion-id>`; external-owned assertions use the complete shared port-record schema |
| extension assertions, Java fixtures, and antlr-ng snapshots had no primary-test rule | Java is primary for serializer/ATN/diagnostic observables; the extension is primary only for unique frontend span/CST observables; antlr-ng is the alternate test oracle |
| adding a third test source risked changing implementation precedence | implementation ports remain antlr-ng TypeScript first and Java second, independent of the primary test source |
| the external inventory did not mechanically own the repository license | the 13-artifact inventory is partitioned by the fixture rows plus the top-level `License.txt` ownership record |
| alternate meta-grammar and diagnostic cases were described but not phase-gated | Phase A parses all 12 sources; Phase B owns alternate meta-grammar loading plus Java serializer/diagnostic fixtures; Phase C runs every final expected outcome |
| downstream updates could be deferred until after legacy deletion | the complete C.6 candidate is authored, then concrete downstream commits or PR heads pin and pass that exact SHA before C.6 is accepted; exact pins still permit independent merge timing |

The deep follow-up returned `not ready` and identified protocol contradictions.
They are resolved in this revision:

| Finding | Resolution in this plan |
| --- | --- |
| external-primary tests could not execute a Java-hard-coded TDD schema | every record names `primary_test_source` and `alternate_test_source`; all clean-context tasks, branches, escalation, relock, and evidence steps use those fields, while implementation order stays antlr-ng then Java |
| C.0 and the global hard stop covered only upstream `port` rows | both now name every phase-owned port record, including external assertions |
| delegated `upstream:<logical-id>` evidence could drift outside the owner's closure | ownership is bidirectional and the upstream closure transitively hashes linked external input, oracle, normalization, and outcome evidence |
| C.5 could test an incomplete pre-C.6 branch | the complete C.6 candidate is authored first, both downstream heads pin and pass its exact SHA, any candidate edit invalidates the runs, and only then is C.6 accepted |
| the corpus summary pluralized the one malformed edit and omitted meta-grammar Phase B fixtures | Phase A distinguishes malformed `a::` from valid undefined `b`; Phase B explicitly includes the alternate meta-grammar trio |

The next pass returned `ready with minor fixes` and no blocking findings. Both
minor findings are incorporated:

| Finding | Resolution in this plan |
| --- | --- |
| changing linked evidence after `done` had no legal relock transition | completed closures remain immutable; a pin/fixture/evidence update creates a new active `mapped` ledger revision while retaining the old `done` manifest |
| the corpus summary implied Java/antlr-ng always supplied both test oracles | it now refers to the record's declared test sources while retaining antlr-ng-then-Java implementation order |

The confirmation pass found one major issue introduced by that revision. It is
resolved:

| Finding | Resolution in this plan |
| --- | --- |
| retaining revisions as map rows would violate logical-ID uniqueness and exact inventory partitioning, and a one-replacement rule could not model A-to-B-to-C history | maps now contain only one active row per stable logical case; historical manifests live in a separate append-only ledger with unique revision IDs, a single-successor acyclic chain, and exactly one leaf selected by `active_revision_id`; only active rows consume inventory IDs |

The final targeted pass validated the active-row projection, A-to-B-to-C
revision chains, external partition, closure links, and phase gates and returned
`ready` with no remaining findings.

The review's technical residual risks remain covered by separate
exact-versus-normalized ATN claims, exact JDK fixture provenance, and
multi-origin imported named-action provenance. Output-ownership redesign and
downstream compatibility choreography were removed after the project policy
was clarified: this early version is expected to break, and the implementation
plan has only three code-review phases.
