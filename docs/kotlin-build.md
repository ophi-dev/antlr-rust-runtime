# Kotlin Grammar Smoke Build

This is the current clean-room path for proving that the runtime can build Rust modules from the ANTLR Kotlin grammar.

## Inputs

- Official ANTLR tool jar, tested with `antlr-4.13.2-complete.jar`.
- Kotlin grammar from `antlr/grammars-v4`, directory `kotlin/kotlin`.

## Generate ANTLR Metadata

```bash
java -jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  -o /tmp/antlr-cleanroom/kotlin-java \
  -Xexact-output-dir \
  KotlinLexer.g4 KotlinParser.g4
```

Run this from the Kotlin grammar directory. The files consumed by this repo are:

- `KotlinLexer.interp`
- `KotlinParser.interp`

## Generate Rust Modules

```bash
cargo run --bin antlr4-rust-gen -- \
  --lexer /tmp/antlr-cleanroom/kotlin-java/KotlinLexer.interp \
  --parser /tmp/antlr-cleanroom/kotlin-java/KotlinParser.interp \
  --out-dir /tmp/antlr-cleanroom/kotlin-rust
```

This emits:

- `kotlin_lexer.rs`
- `kotlin_parser.rs`

The generated lexer and parser cache a deserialized ATN with `OnceLock` and delegate recognition to `antlr4_runtime`.

## Smoke Crate

Create any Rust crate that depends on this runtime:

```toml
[dependencies]
antlr-rust-runtime = { path = "../path/to/runtime-crate" }
```

Replace the path with the relative path from the smoke crate to this checkout.

Then include the generated modules and parse a Kotlin sample:

```rust
use antlr4_runtime::{CommonTokenStream, InputStream};
use generated::kotlin_lexer::KotlinLexer;
use generated::kotlin_parser::KotlinParser;

let lexer = KotlinLexer::new(InputStream::new("fun main() {}"));
let tokens = CommonTokenStream::new(lexer);
let mut parser = KotlinParser::new(tokens);
let tree = parser.kotlin_file().expect("entry rule parses");
assert!(tree.text().contains("fun"));
```

Validated locally: the generated Kotlin lexer emits real tokens and the generated parser recognizes the `parser.kotlin_file()` entry rule for `fun main() {}`.
