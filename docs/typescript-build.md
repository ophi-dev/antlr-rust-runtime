# Build the official TypeScript grammar for Rust

The official grammars-v4 TypeScript lexer and parser use grammar-specific
stateful actions and predicates, including the argument-taking `p("of")` and
`n("get"|"set")` helpers. This repository routes those calls through generated
typed Rust hooks and keeps the target-specific state in copyable base modules.

## Prerequisites

- Rust 1.95 or newer
- `antlr/grammars-v4` at the pinned parity commit
- Java 17 and ANTLR 4.13.2 only when running the Java parity proof

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
git -C /tmp/antlr-cleanroom/grammars-v4 sparse-checkout set javascript/typescript
git -C /tmp/antlr-cleanroom/grammars-v4 checkout \
  284602b3f23ca54dc30778204ab7ae9e969145e9
```

## Generate strict Rust modules

From this repository's root:

```bash
GRAMMAR=/tmp/antlr-cleanroom/grammars-v4/javascript/typescript
BUILD=/tmp/antlr-cleanroom/typescript-rust
mkdir -p "$BUILD/generated"

cargo run --locked --release --features codegen --bin antlr4-rust-gen -- \
  "$GRAMMAR/TypeScriptLexer.g4" \
  "$GRAMMAR/TypeScriptParser.g4" \
  --lib "$GRAMMAR" \
  --sem-patterns patterns/javascript.toml \
  --option-hook superClass=TypeScriptLexerBase \
  --option-hook superClass=TypeScriptParserBase \
  --sem-unknown error \
  --require-full-semantics \
  --out-dir "$BUILD/generated"
```

Every authored action and predicate is either translated or routed to a typed
hook. Do not pass `--allow-unsupported-lexer-actions`: the TypeScript base needs
all template, brace, strict-mode, regex, and token-lookaround helpers to retain
the official grammar's behavior.
The `--option-hook` acknowledgements record that those Rust hooks supply the
otherwise target-specific superclass behavior.

Copy these files into an application crate:

- `$BUILD/generated/type_script_lexer.rs`
- `$BUILD/generated/type_script_parser.rs`
- `tests/typescript-parity/dumper/src/typescript_lexer_base.rs`
- `tests/typescript-parity/dumper/src/typescript_parser_base.rs`

The base files are examples rather than runtime modules. Adjust their module
paths if the generated files do not live under `generated` in the application.

## Construct the typed lexer and parser

```rust
use antlr4_runtime::{CommonTokenStream, InputStream, Parser};
use generated::type_script_lexer::TypeScriptLexer;
use generated::type_script_parser::TypeScriptParser;
use typescript_lexer_base::TypeScriptLexerBase;
use typescript_parser_base::TypeScriptParserBase;

let source = "interface Box<T> { value: T }";
let lexer = TypeScriptLexer::with_typed_hooks(
    InputStream::new(source),
    TypeScriptLexerBase::with_strict_default(false),
);
let tokens = CommonTokenStream::new(lexer);
let mut parser = TypeScriptParser::with_typed_hooks(tokens, TypeScriptParserBase);
let tree = parser.program().expect("TypeScript parses");
assert_eq!(parser.number_of_syntax_errors(), 0);
assert!(!tree.text().is_empty());
```

`program()` is the compilation-unit entry rule. The lexer base tracks the last
default-channel token, strict scopes, brace depth, and nested template depth.
The parser base implements visible lookahead/lookbehind for `n(...)` and
`p(...)`, automatic-semicolon-insertion checks, and the TypeScript
open-brace/function/interface guard.

For lower-level diagnostics, fill a `CommonTokenStream` and call
`drain_source_errors()` before parsing, or inspect
`Parser::number_of_syntax_errors()` after the entry rule.

The generator intentionally omits `--require-generated-parser`. Rules outside
the generated recursive-descent subset use the faithful runtime ATN
interpreter, which receives the same typed hooks.

## Run the repository proof

```bash
tests/typescript-parity/run.sh \
  --antlr-jar "$ANTLR4_JAR" \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
```

The harness regenerates the Rust recognizers and the official Java target, then
compares tokens and parse trees for every fixture under
`tests/typescript-parity/snippets/`.
