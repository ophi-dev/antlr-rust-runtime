# ANTLR Runtime Testsuite

ANTLR maintains a shared runtime conformance suite in `antlr/antlr4/runtime-testsuite`.
This repo includes `antlr4-runtime-testsuite`, a Rust-side harness that consumes those
upstream descriptor files without vendoring them.

## Why a Rust Harness Exists

The upstream Java/JUnit harness assumes each target can be generated directly with
`-Dlanguage=<target>` and that target-specific grammar action templates are available.
This runtime currently uses a clean-room metadata path:

1. the official ANTLR tool emits `.interp` metadata,
2. `antlr4-rust-gen` emits Rust modules from that metadata,
3. the generated modules run against `antlr4_runtime`.

The harness follows that path while still using the upstream descriptor grammar,
input, stdout, and stderr expectations.

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
- single-grammar descriptors,
- descriptor stdout/stderr comparison,
- grouped lexer recovery diagnostics,
- farthest-token parser mismatch diagnostics for supported non-recovery
  failures,
- parser single-token insertion/deletion recovery diagnostics for supported
  descriptors,
- parser precedence predicates in metadata-driven recognition,
- lexer and parser target-template actions for the currently supported stdout
  helpers,
- parser token-label text actions such as `$TOKEN.text` and `$label.text`,
- parser token-display actions such as `Append(..., "$label")` and
  `Append(..., "$rule.stop")` for recovered-token descriptors,
- parser rule-level `@after` actions for the currently supported stdout helpers,
- parser rule-level `@init {<GetExpectedTokenNames():writeln()>}` actions,
- nested parser tree construction for action-bearing rules and direct
  `ToStringTree("$ctx")` stdout actions,
- lexer semantic predicates for the currently supported `True()`, `False()`,
  and `TextEquals(...)` templates,
- lexer accept-position adjustment for the upstream `PositionAdjustingLexer`
  target template,
- parser `@init {<BuildParseTrees()>}` and `notBuildParseTree` descriptors,
- parser rule-level `@after {<ToStringTree("$label.ctx")>}` actions for simple
  rule labels,
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
- `StringTemplate` backslash rendering for descriptor grammars,
- official ANTLR `.interp` generation,
- Rust module generation and execution through Cargo.

Not wired yet:

- composite grammars,
- target-template semantic actions beyond the currently supported stdout helpers
  and no-op compile checks,
- parser error recovery diagnostics beyond the currently supported mismatch and
  single-token recovery cases,
- runtime diagnostic/profile/DFA flags.

The harness reports unsupported descriptors as skipped and treats output mismatches
as failures.

Current validated groups:

- full descriptor sweep: `251 passed, 0 failed, 106 skipped, 251 run`
- `LexerExec`: `42 passed, 0 failed, 0 skipped, 42 run`
- `LexerErrors`: `12 passed, 0 failed, 0 skipped, 12 run`
- `LeftRecursion`: `81 passed, 0 failed, 17 skipped, 81 run`
- `ParseTrees`: `6 passed, 0 failed, 4 skipped, 6 run`
- `ParserExec`: `43 passed, 0 failed, 7 skipped, 43 run`
- `ParserErrors`: `22 passed, 0 failed, 12 skipped, 22 run`
- `Performance`: `7 passed, 0 failed, 0 skipped, 7 run`
- `SemPredEvalLexer`: `2 passed, 0 failed, 6 skipped, 2 run`
- `SemPredEvalParser`: `7 passed, 0 failed, 19 skipped, 7 run`
- `Sets`: `29 passed, 0 failed, 2 skipped, 29 run`

The remaining target-action skips are descriptors that depend on templates the
Rust harness does not render yet, such as target members, listener hooks,
diagnostic helpers, return-value evaluation, parser predicates that need
generated context methods, or listener hooks.
