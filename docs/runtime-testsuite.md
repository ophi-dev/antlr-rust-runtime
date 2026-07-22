# ANTLR Runtime Testsuite

ANTLR maintains a shared runtime conformance suite in `antlr/antlr4/runtime-testsuite`.
This repo includes `antlr4-runtime-testsuite`, a Rust-side harness that consumes those
upstream descriptor files without vendoring them.

## Why a Rust Harness Exists

The upstream Java/JUnit harness assumes each target can be generated directly with
`-Dlanguage=<target>` and that target-specific grammar action templates are available.
The Rust harness keeps the upstream descriptor/template contract but uses the
direct source compiler:

1. the ANTLR jar's StringTemplate engine renders each descriptor through
   `Rust.test.stg`,
2. `antlr4-rust-gen` compiles the rendered `.g4` root and import graph directly,
3. the generated modules run against `antlr4_runtime`.

The jar is used only as the upstream StringTemplate implementation. It does
not generate Rust metadata or recognizers.

## Run Full Sweep

On the maintainer checkout, where the ANTLR jar and upstream runtime-testsuite
live under `/tmp/antlr-cleanroom`, the full Rust sweep is:

```bash
cargo run --quiet --bin antlr4-runtime-testsuite
```

In other environments, pass explicit paths or set `ANTLR4_JAR` and
`ANTLR4_RUNTIME_TESTSUITE`.

## Run One Descriptor

```bash
cargo run --bin antlr4-runtime-testsuite -- \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --descriptors /tmp/antlr-cleanroom/antlr4-upstream/runtime-testsuite \
  --case LexerExec/KeywordID
```

`--descriptors` may point either at the upstream `runtime-testsuite` directory or
directly at its `resources/org/antlr/v4/test/runtime/descriptors` directory.

## Run a Group Sample

```bash
cargo run --bin antlr4-runtime-testsuite -- \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --descriptors /tmp/antlr-cleanroom/antlr4-upstream/runtime-testsuite \
  --group LexerExec \
  --limit 20
```

The harness creates temporary Cargo crates under `target/antlr-runtime-testsuite`.
Pass `--keep` to retain those directories for debugging.

## Current Scope

Supported now:

- lexer descriptors,
- parser descriptors with empty stdout/stderr expectations,
- single and imported/composite grammar source graphs,
- descriptor stdout/stderr comparison,
- grouped lexer recovery diagnostics,
- farthest-token parser mismatch diagnostics for supported non-recovery
  failures,
- parser single-token insertion/deletion recovery diagnostics for supported
  descriptors,
- parser mismatched-token recovery diagnostics and error-node parse trees for
  supported descriptors,
- parser extraneous-input diagnostics and error-node parse trees for supported
  single-token deletion descriptors,
- parser precedence predicates in ATN recognition,
- lexer and parser target-template actions for the currently supported stdout
  helpers,
- parser token-label text actions such as `$TOKEN.text` and `$label.text`,
- parser `AppendStr(..., "$TOKEN.text")` stdout actions for supported
  semantic-predicate descriptors,
- parser token-display actions such as `Append(..., "$label")` and
  `Append(..., "$rule.stop")` for recovered-token descriptors,
- parser rule-level `@after` actions for the currently supported stdout helpers,
- parser `$text` action intervals that stop at the previous visible token,
  including the greedy and non-greedy if/else binding descriptors,
- parser decision-order tie breaking for clean action-bearing ambiguities such
  as optional `else` binding and assignment-vs-wildcard alternatives,
- parser rule-level `@init {<GetExpectedTokenNames():writeln()>}` actions,
- nested parser tree construction for action-bearing rules and direct
  `ToStringTree("$ctx")` stdout actions,
- lexer semantic predicates for the currently supported `True()`, `False()`,
  `TextEquals(...)`, token-start-column, and current-column templates,
- lexer DFA dump output for the predicate-sensitive `SemPredEvalLexer`
  `showDFA` fixtures,
- lexer accept-position adjustment for the upstream `PositionAdjustingLexer`
  through the same generic post-accept lifecycle hook available to callers,
- parser `@init {<BuildParseTrees()>}` and `notBuildParseTree` descriptors,
- parser `predictionMode=LL` and `predictionMode=SLL` descriptors modeled by
  the metadata recognizer,
- parser `showDiagnosticErrors` ambiguity diagnostics for the currently modeled
  exact-ambiguity semantic-predicate descriptors,
- parser `DumpDFA()` output for the currently modeled full-context diagnostics
  descriptors,
- parser rule-level `@after {<ToStringTree("$label.ctx")>}` actions for simple
  rule labels,
- parser semantic predicates for `LANotEquals(...)` and `LTEquals(...)`
  lookahead target templates,
- parser rule-argument predicates for supported `ValEquals("$i", "...")`
  target templates, including literal integer calls and `VarRef("i")`
  forwarding,
- parser boolean-member predicates for the runtime-testsuite
  `GetMember(...):Not()` fixture,
- parser integer-member target templates for semantic-predicate fixtures,
  including `AddMember`, `GetMember`, `ModMemberEquals`, and
  `ModMemberNotEquals`,
- multi-template parser action blocks and empty regular actions bound to their
  finalized ATN action states,
- parser supported-predicate decision ordering for action-bearing alternatives,
- listener-suite target templates for `BasicListener`, token/rule getter
  listeners, and the left-recursive listener fixtures,
- simple left-recursive return-value stdout helpers such as `$e.v` and
  `$e.result`,
- common-label left-recursion compile-check templates such as `Production(...)`
  and `Result(...)`,
- integer/member predicate scaffolding used by selected semantic-predicate
  descriptors, including `InitIntMember`, `SetMember`, and `Invoke_pred`,
- alt-numbered parse-tree contexts for grammars using
  `TreeNodeWithAltNumField`/`contextSuperClass`,
- `RuleInvocationStack()` stdout helper actions,
- `BailErrorStrategy()` descriptors as no-ops while the default Rust error
  handling still matches the covered outputs,
- compile-time-only target templates such as `IntArg`, `AssignLocal`,
  `AssertIsList`, `Pass`, parser property helpers, and supported member
  scaffolding as no-ops,
- nested `StringTemplate` action parsing for supported no-op wrappers,
- `StringTemplate` comments in descriptor grammars,
- ANTLR recursive-context tree rewrites for left-recursive parse-tree output,
- ANTLR whitespace escaping for terminal text in `ToStringTree(...)` output,
- `StringTemplate` backslash rendering for descriptor grammars,
- direct Rust module generation from rendered grammar source and execution
  through Cargo.

Not wired yet:

- composite grammar override/member/mixed-action shapes beyond the currently
  covered descriptor cases,
- target-template semantic actions beyond the currently supported stdout helpers
  and no-op compile checks,
- parser error recovery diagnostics beyond the currently supported mismatch,
  no-viable, extraneous-input, semantic-predicate fail options, EOF unwind, and
  token recovery cases,
- runtime diagnostic/profile/DFA flags beyond the currently modeled ambiguity
  diagnostics and non-default prediction modes.

The harness reports unsupported descriptors as skipped, and the sweep fails if
any descriptor fails or is skipped.

Current validated groups:

- full descriptor sweep: `357 passed, 0 failed, 0 skipped, 357 run`
- `CompositeLexers`: `2 passed, 0 failed, 0 skipped, 2 run`
- `CompositeParsers`: `15 passed, 0 failed, 0 skipped, 15 run`
- `FullContextParsing`: `15 passed, 0 failed, 0 skipped, 15 run`
- `LexerExec`: `42 passed, 0 failed, 0 skipped, 42 run`
- `LexerErrors`: `12 passed, 0 failed, 0 skipped, 12 run`
- `LeftRecursion`: `98 passed, 0 failed, 0 skipped, 98 run`
- `Listeners`: `7 passed, 0 failed, 0 skipped, 7 run`
- `ParseTrees`: `10 passed, 0 failed, 0 skipped, 10 run`
- `ParserExec`: `50 passed, 0 failed, 0 skipped, 50 run`
- `ParserErrors`: `34 passed, 0 failed, 0 skipped, 34 run`
- `Performance`: `7 passed, 0 failed, 0 skipped, 7 run`
- `SemPredEvalLexer`: `8 passed, 0 failed, 0 skipped, 8 run`
- `SemPredEvalParser`: `26 passed, 0 failed, 0 skipped, 26 run`
- `Sets`: `31 passed, 0 failed, 0 skipped, 31 run`
