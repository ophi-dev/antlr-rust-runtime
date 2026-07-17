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

The generated lexer caches its deserialized lexer ATN with `OnceLock`. The
generated parser embeds a versioned packed parser ATN, validates it once without
rebuilding an object graph, and delegates recognition to `antlr4_runtime`.

## Choose the Kotlin Entry Rule

`antlr4-rust-gen` emits one public parser method for every grammar rule and
lists those methods in the generated parser rustdoc. Choose the method that
matches the Kotlin input shape rather than assuming the first rule is always
right:

- `.kt` compilation units use `parser.kotlin_file()`.
- `.kts` script-style input uses `parser.script()`.

ANTLR recovery can produce a parse tree even when the wrong entry rule is used,
but the tree will contain recovered error nodes and diagnostics. When adding a
new Kotlin input form, confirm the entry rule against the upstream grammar and
check parser diagnostics.

## Smoke Crate

Create any Rust crate that depends on this runtime:

```toml
[dependencies]
antlr-rust-runtime = { path = "../path/to/runtime-crate" }
```

Replace the path with the relative path from the smoke crate to this checkout.

Then include the generated modules and parse a Kotlin sample:

```rust
use generated::kotlin_lexer::KotlinLexer;
use generated::kotlin_parser::{self, KotlinParser};

let tree = kotlin_parser::parse("fun main() {}", KotlinLexer::new, KotlinParser::kotlin_file)
    .expect("entry rule parses");
assert!(tree.text().contains("fun"));
```

Use `parse_with_parser` when a caller also needs parser state after the entry
rule, such as syntax diagnostics or the token stream:

```rust
use antlr4_runtime::Parser;
use generated::kotlin_lexer::KotlinLexer;
use generated::kotlin_parser::{self, KotlinParser};

let output =
    kotlin_parser::parse_with_parser("fun main() {}", KotlinLexer::new, KotlinParser::kotlin_file)
        .expect("entry rule parses");
let syntax_errors = output.parser.number_of_syntax_errors();
let tree = output.result;
let tokens = output.parser.into_token_stream();

assert_eq!(syntax_errors, 0);
assert!(tree.text().contains("fun"));
assert!(!tokens.tokens().is_empty());
```

The generated helper is additive. The explicit path is still available when the
caller needs to name the input source, adjust parser options, or attach custom
error handling before the entry rule:

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
