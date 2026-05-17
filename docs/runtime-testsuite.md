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
- `StringTemplate` backslash rendering for descriptor grammars,
- official ANTLR `.interp` generation,
- Rust module generation and execution through Cargo.

Not wired yet:

- composite grammars,
- target-template semantic actions such as `<writeln(...)>`,
- parser target actions/listeners that produce expected stdout,
- parser error recovery diagnostics,
- runtime diagnostic/profile/DFA flags.

The harness reports unsupported descriptors as skipped and treats output mismatches
as failures.

Current validated groups:

- `LexerExec`: `29 passed, 0 failed, 13 skipped, 29 run`
- `LexerErrors`: `12 passed, 0 failed, 0 skipped, 12 run`
- `ParserExec`: `10 passed, 0 failed, 40 skipped, 10 run`
- `ParserErrors`: `4 passed, 0 failed, 30 skipped, 4 run`

The `LexerExec` skips are descriptors that depend on target-specific action or
member templates. Those should become runnable when the Rust target action
surface is generated instead of represented only as `.interp` metadata.
