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
- single-grammar descriptors,
- descriptor stdout/stderr comparison,
- official ANTLR `.interp` generation,
- Rust module generation and execution through Cargo.

Not wired yet:

- parser descriptors,
- composite grammars,
- target-template semantic actions such as `<writeln(...)>`,
- runtime diagnostic/profile/DFA flags.

The harness reports unsupported descriptors as skipped and treats output mismatches
as failures. The first passing upstream descriptors are:

- `LexerExec/KeywordID`
- `LexerExec/EOFSuffixInFirstRule_1`
- `LexerExec/QuoteTranslation`
