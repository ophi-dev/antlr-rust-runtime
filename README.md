# ANTLR4 Runtime for Rust

`antlr-rust-runtime` is a pure Rust runtime and metadata generator for ANTLR v4
lexers and parsers. It is a clean-room implementation written from scratch from
the public ANTLR runtime contract; it does not vendor or fork an older Rust
ANTLR runtime.

## First Steps

### 1. Install ANTLR4

Follow the ANTLR getting-started guide and install the ANTLR tool jar. The
runtime tests currently validate against ANTLR `4.13.2`.

### 2. Install the Rust ANTLR runtime tools

Each ANTLR target language needs a runtime package used by generated parsers.
For Rust projects, add the runtime crate:

```toml
[dependencies]
antlr-rust-runtime = "0.1"
```

The library crate is imported as `antlr4_runtime`:

```rust
use antlr4_runtime::{CommonTokenStream, InputStream};
```

Install the companion generator binary:

```bash
cargo install antlr-rust-runtime
```

This installs `antlr4-rust-gen`, which turns ANTLR `.interp` metadata into Rust
lexer and parser modules.

### 3. Generate your parser

The current release uses a metadata-first generation path:

1. run the official ANTLR tool to produce `.interp` files,
2. run `antlr4-rust-gen` to emit Rust modules,
3. compile those modules against `antlr4_runtime`.

For a split lexer/parser grammar:

```bash
antlr4 MyGrammarLexer.g4 MyGrammarParser.g4

antlr4-rust-gen \
  --lexer MyGrammarLexer.interp \
  --parser MyGrammarParser.interp \
  --out-dir src/generated
```

The checked-in ANTLR `RustTarget`/StringTemplate shell is kept in `tool/` and
will be expanded around the same runtime contracts.

## Complete Example

Suppose you are using the JSON grammar from `antlr/grammars-v4/json`.

Fetch or copy `JSON.g4`, then generate ANTLR metadata:

```bash
antlr4 JSON.g4
```

Generate Rust modules:

```bash
antlr4-rust-gen \
  --lexer JSONLexer.interp \
  --parser JSONParser.interp \
  --out-dir src/generated
```

Declare the generated modules in your crate:

```rust
mod generated {
    pub mod json_lexer;
    pub mod json_parser;
}
```

Call the generated lexer and parser:

```rust
use antlr4_runtime::{CommonTokenStream, InputStream};
use generated::json_lexer::JSONLexer;
use generated::json_parser::JSONParser;

fn main() -> Result<(), antlr4_runtime::AntlrError> {
    let lexer = JSONLexer::new(InputStream::new(r#"{"a":1}"#));
    let tokens = CommonTokenStream::new(lexer);
    let mut parser = JSONParser::new(tokens);
    let tree = parser.json()?;

    println!("{}", tree.text());
    Ok(())
}
```

## Technical Notes

- Pure Rust runtime implementation.
- Written from scratch as a clean-room implementation.
- Supports ANTLR serialized ATN deserialization.
- Supports lexer and parser execution through generated Rust wrappers.
- Supports real split lexer/parser grammars, including Kotlin smoke builds.
- Passes every upstream ANTLR runtime-testsuite descriptor discovered by the
  harness: `357 passed, 0 failed, 0 skipped, 357 run`.
- Licensed under BSD-3-Clause for compatibility with ANTLR's runtime licensing
  pattern and downstream open-source applications.

The runtime contains:

- `IntStream` and `CharStream`
- UTF-8 input as Unicode scalar values
- `Token`, `CommonToken`, token factories, and `TokenSource`
- buffered, channel-aware `CommonTokenStream`
- `Vocabulary`
- recognizer metadata and error listener plumbing
- parse tree node types, rule contexts, terminal nodes, error nodes, and walkers
- ANTLR v4 serialized ATN deserialization
- lexer ATN recognition with longest-match/rule-priority behavior and lexer
  actions
- parser ATN rule recognition with backtracking over token stream indices
- `antlr4-rust-gen`, a Rust generator that consumes ANTLR `.interp` metadata and
  emits Rust modules
- `antlr4-runtime-testsuite`, a harness for running upstream ANTLR
  runtime-test descriptors through the Rust metadata path

See [docs/kotlin-build.md](docs/kotlin-build.md) for the Kotlin smoke workflow.
See [docs/runtime-testsuite.md](docs/runtime-testsuite.md) for the upstream
runtime-testsuite harness.

## Runtime Testsuite

On the maintainer checkout, where the ANTLR jar and upstream runtime-testsuite
live under `/tmp/antlr-cleanroom`, run the full sweep with:

```bash
cargo run --quiet --bin antlr4-runtime-testsuite
```

Run a specific descriptor:

```bash
cargo run --bin antlr4-runtime-testsuite -- \
  --antlr-jar path/to/antlr-4.13.2-complete.jar \
  --descriptors path/to/antlr4/runtime-testsuite \
  --case LexerExec/KeywordID
```

## Useful Information

- ANTLR: <https://www.antlr.org/>
- ANTLR documentation: <https://github.com/antlr/antlr4/blob/dev/doc/index.md>
- Grammars v4: <https://github.com/antlr/grammars-v4>
