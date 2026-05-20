# ANTLR v4 Runtime for Rust

This repository is a clean-room Rust implementation of the ANTLR v4 runtime and target support.

No third-party Rust runtime or target implementation is vendored here. The implementation is built from the public ANTLR runtime contract: streams, tokens, token sources, token streams, recognizers, lexers, parsers, parse trees, error listeners/strategies, ATN metadata, and generated-code integration.

## Goals

- Generate Rust lexers and parsers from ANTLR v4 grammars with `-Dlanguage=Rust`.
- Support real-world grammars, including split lexer/parser grammars and large grammars such as Kotlin.
- Keep generated Rust code idiomatic, explicit, and stable across crate releases.
- Keep runtime behavior compatible with ANTLR v4 semantics while using Rust ownership and errors directly.

## Current Status

The crate now contains a working clean-room runtime core and metadata-based generator:

- `IntStream` and `CharStream`
- UTF-8 input as Unicode scalar values
- `Token`, `CommonToken`, token factories, and `TokenSource`
- buffered, channel-aware `CommonTokenStream`
- `Vocabulary`
- recognizer metadata and error listener plumbing
- parse tree node types, rule contexts, terminal nodes, error nodes, and walkers
- ANTLR v4 serialized ATN deserialization
- lexer ATN recognition with longest-match/rule-priority behavior and lexer actions
- parser ATN rule recognition with backtracking over token stream indices
- generated lexer/parser wrappers over the runtime base types
- `antlr4-rust-gen`, a Rust generator that consumes ANTLR `.interp` metadata and emits Rust modules
- `antlr4-runtime-testsuite`, a harness for running upstream ANTLR runtime-test descriptors through the Rust metadata path

The current generator path is intentionally metadata-first: run the official ANTLR tool to produce `.interp` files from grammars, then run `antlr4-rust-gen` to emit Rust. The checked-in Java `RustTarget`/StringTemplate files are still the direct `-Dlanguage=Rust` integration shell and will be expanded around the same runtime contracts.

The current parser builds and recognizes Kotlin's `kotlinFile` entry rule for a smoke sample. Parse tree shape is still basic: parser recognition is ATN-backed, but nested rule-node construction and full ANTLR error recovery are still in progress.

See [docs/kotlin-build.md](docs/kotlin-build.md) for the Kotlin smoke workflow.
See [docs/runtime-testsuite.md](docs/runtime-testsuite.md) for the upstream runtime-testsuite harness.

## Development

```bash
cargo test
```

Generate Rust modules from ANTLR `.interp` metadata:

```bash
cargo run --bin antlr4-rust-gen -- \
  --lexer path/to/KotlinLexer.interp \
  --parser path/to/KotlinParser.interp \
  --out-dir target/generated/kotlin
```

Run one upstream runtime-testsuite descriptor:

```bash
cargo run --quiet --bin antlr4-runtime-testsuite

cargo run --bin antlr4-runtime-testsuite -- \
  --antlr-jar path/to/antlr-4.13.2-complete.jar \
  --descriptors path/to/antlr4/runtime-testsuite \
  --case LexerExec/KeywordID
```

## Clean-Room Notes

The implementation does not copy code from an existing Rust ANTLR runtime. Requirements are derived from ANTLR's public runtime APIs and documented behavior, then implemented independently in Rust.
