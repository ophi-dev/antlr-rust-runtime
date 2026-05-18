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
- parser precedence predicates in metadata-driven recognition,
- lexer and parser target-template actions for the currently supported stdout
  helpers,
- parser token-label text actions such as `$TOKEN.text` and `$label.text`,
- `StringTemplate` backslash rendering for descriptor grammars,
- official ANTLR `.interp` generation,
- Rust module generation and execution through Cargo.

Not wired yet:

- composite grammars,
- target-template semantic actions beyond the currently supported stdout helpers,
- parser error recovery diagnostics,
- runtime diagnostic/profile/DFA flags.

The harness reports unsupported descriptors as skipped and treats output mismatches
as failures.

Current validated groups:

- full descriptor sweep: `142 passed, 0 failed, 215 skipped, 142 run`
- `LexerExec`: `41 passed, 0 failed, 1 skipped, 41 run`
- `LexerErrors`: `12 passed, 0 failed, 0 skipped, 12 run`
- `LeftRecursion`: `7 passed, 0 failed, 91 skipped, 7 run`
- `ParserExec`: `35 passed, 0 failed, 15 skipped, 35 run`
- `ParserErrors`: `4 passed, 0 failed, 30 skipped, 4 run`
- `Performance`: `7 passed, 0 failed, 0 skipped, 7 run`
- `SemPredEvalLexer`: `1 passed, 0 failed, 7 skipped, 1 run`
- `SemPredEvalParser`: `7 passed, 0 failed, 19 skipped, 7 run`
- `Sets`: `28 passed, 0 failed, 3 skipped, 28 run`

The remaining target-action skips are descriptors that depend on templates the
Rust harness does not render yet, such as target members, listener hooks,
diagnostic helpers, or semantic predicates that need generated context methods.
