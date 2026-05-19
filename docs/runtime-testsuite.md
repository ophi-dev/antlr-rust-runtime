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
- parser mismatched-token recovery diagnostics and error-node parse trees for
  supported descriptors,
- parser extraneous-input diagnostics and error-node parse trees for supported
  single-token deletion descriptors,
- parser precedence predicates in metadata-driven recognition,
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
  and `TextEquals(...)` templates,
- lexer accept-position adjustment for the upstream `PositionAdjustingLexer`
  target template,
- parser `@init {<BuildParseTrees()>}` and `notBuildParseTree` descriptors,
- parser rule-level `@after {<ToStringTree("$label.ctx")>}` actions for simple
  rule labels,
- parser semantic predicates for `LANotEquals(...)` and `LTEquals(...)`
  lookahead target templates,
- parser rule-argument predicates for supported `ValEquals("$i", "...")`
  target templates, including literal integer calls and `VarRef("i")`
  forwarding,
- parser integer-member target templates for semantic-predicate fixtures,
  including `AddMember`, `GetMember`, `ModMemberEquals`, and
  `ModMemberNotEquals`,
- multi-template parser action blocks and empty regular actions that must stay
  aligned with serialized ATN action states,
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
- official ANTLR `.interp` generation,
- Rust module generation and execution through Cargo.

Not wired yet:

- composite grammar override/member/mixed-action shapes beyond the currently
  supported import metadata cases,
- target-template semantic actions beyond the currently supported stdout helpers
  and no-op compile checks,
- parser error recovery diagnostics beyond the currently supported mismatch,
  no-viable, extraneous-input, and token recovery cases,
- runtime diagnostic/profile/DFA flags.

The harness reports unsupported descriptors as skipped and treats output mismatches
as failures.

Current validated groups:

- full descriptor sweep: `319 passed, 0 failed, 38 skipped, 319 run`
- `CompositeLexers`: `1 passed, 0 failed, 1 skipped, 1 run`
- `CompositeParsers`: `8 passed, 0 failed, 7 skipped, 8 run`
- `LexerExec`: `42 passed, 0 failed, 0 skipped, 42 run`
- `LexerErrors`: `12 passed, 0 failed, 0 skipped, 12 run`
- `LeftRecursion`: `97 passed, 0 failed, 1 skipped, 97 run`
- `Listeners`: `7 passed, 0 failed, 0 skipped, 7 run`
- `ParseTrees`: `10 passed, 0 failed, 0 skipped, 10 run`
- `ParserExec`: `48 passed, 0 failed, 2 skipped, 48 run`
- `ParserErrors`: `34 passed, 0 failed, 0 skipped, 34 run`
- `Performance`: `7 passed, 0 failed, 0 skipped, 7 run`
- `SemPredEvalLexer`: `2 passed, 0 failed, 6 skipped, 2 run`
- `SemPredEvalParser`: `20 passed, 0 failed, 6 skipped, 20 run`
- `Sets`: `31 passed, 0 failed, 0 skipped, 31 run`

The remaining skips are now dominated by diagnostic/profile flags, remaining
composite grammar shapes, and parser recovery diagnostics beyond the currently
modeled cases.
