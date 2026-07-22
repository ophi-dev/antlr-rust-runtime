# Build the official JavaScript grammar for Rust

The official grammars-v4 JavaScript lexer and parser use grammar-specific
stateful actions and predicates. This repository supports them through typed
Rust hook modules, following the same base-class model used by the official Go,
Python, C#, C++, Java, and JavaScript targets.

## Prerequisites

- Rust 1.95 or newer
- `antlr/grammars-v4` at the pinned parity commit
- Java 17 and ANTLR 4.13.2 only when running the Python parity proof

```bash
# The jar is an oracle dependency for the parity harness, not Rust generation.
ANTLR4_JAR=/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar
ANTLR_JAR_SHA256=eae2dfa119a64327444672aff63e9ec35a20180dc5b8090b7a6ab85125df4d76
mkdir -p /tmp/antlr-cleanroom/tools
curl -fLo "$ANTLR4_JAR" \
  https://www.antlr.org/download/antlr-4.13.2-complete.jar
echo "${ANTLR_JAR_SHA256}  ${ANTLR4_JAR}" | shasum -a 256 -c -

git clone --filter=blob:none --no-checkout \
  https://github.com/antlr/grammars-v4.git \
  /tmp/antlr-cleanroom/grammars-v4
git -C /tmp/antlr-cleanroom/grammars-v4 sparse-checkout init --cone
git -C /tmp/antlr-cleanroom/grammars-v4 sparse-checkout set javascript/javascript
git -C /tmp/antlr-cleanroom/grammars-v4 checkout \
  284602b3f23ca54dc30778204ab7ae9e969145e9
```

## Generate strict Rust modules

From this repository's root:

```bash
GRAMMAR=/tmp/antlr-cleanroom/grammars-v4/javascript/javascript
BUILD=/tmp/antlr-cleanroom/javascript-rust
mkdir -p "$BUILD/generated"

cargo run --locked --release --features codegen --bin antlr4-rust-gen -- \
  "$GRAMMAR/JavaScriptLexer.g4" \
  "$GRAMMAR/JavaScriptParser.g4" \
  --lib "$GRAMMAR" \
  --sem-patterns patterns/javascript.toml \
  --option-hook superClass=JavaScriptLexerBase \
  --option-hook superClass=JavaScriptParserBase \
  --sem-unknown error \
  --require-full-semantics \
  --out-dir "$BUILD/generated"
```

This deliberately does not use `--allow-unsupported-lexer-actions`: every
authored coordinate is translated or routed to a typed Rust hook. It also does
not use `--require-generated-parser`; rules without generated
recursive-descent bodies use the faithful runtime ATN interpreter, including
the same semantic hooks.
The `--option-hook` acknowledgements record that the checked-in Rust base-hook
modules supply the grammars' superclass behavior.

Copy these files into an application crate:

- `$BUILD/generated/java_script_lexer.rs`
- `$BUILD/generated/java_script_parser.rs`
- `tests/javascript-parity/dumper/src/javascript_lexer_base.rs`
- `tests/javascript-parity/dumper/src/javascript_parser_base.rs`

The base files are examples rather than runtime modules. Adjust their module
paths if the generated files do not live under `generated` in the application.

## Construct the typed lexer and parser

```rust
use antlr4_runtime::{CommonTokenStream, InputStream, Parser};
use generated::java_script_lexer::JavaScriptLexer;
use generated::java_script_parser::JavaScriptParser;
use javascript_lexer_base::JavaScriptLexerBase;
use javascript_parser_base::JavaScriptParserBase;

let source = "class Example { static value = /x+/; }";
let lexer = JavaScriptLexer::with_typed_hooks(
    InputStream::new(source),
    JavaScriptLexerBase::with_strict_default(false),
);
let tokens = CommonTokenStream::new(lexer);
let mut parser = JavaScriptParser::with_typed_hooks(tokens, JavaScriptParserBase);
let tree = parser.program().expect("JavaScript parses");
assert_eq!(parser.number_of_syntax_errors(), 0);
assert!(!tree.text().is_empty());
```

`program()` is the compilation-unit entry rule. The lexer base tracks the last
default-channel token, strict scopes, brace depth, and template depth. The
parser base supplies automatic-semicolon-insertion and contextual lookahead
helpers.

For lower-level diagnostics, fill a `CommonTokenStream` and call
`drain_source_errors()` before parsing, or inspect
`Parser::number_of_syntax_errors()` after the entry rule.

## Run the repository proof

Install the Python reference runtime and run the parity harness:

```bash
python3 -m pip install antlr4-python3-runtime==4.13.2
tests/javascript-parity/run.sh \
  --antlr-jar "$ANTLR4_JAR" \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
```

The harness regenerates both targets and compares tokens and parse trees for
all fixtures under `tests/javascript-parity/snippets/`.
