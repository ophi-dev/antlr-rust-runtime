# TypeScript lexer/parser parity

This smoke test builds the unmodified official
`antlr/grammars-v4/javascript/typescript` grammar for Rust and compares its
tokens and parse trees byte-for-byte with the official Java target.

The checked-in `dumper/src/typescript_lexer_base.rs` and
`dumper/src/typescript_parser_base.rs` files are Rust equivalents of the
grammar-specific Java base classes shipped by grammars-v4. Generated recognizer
modules are copied into `dumper/src/generated/` by `run.sh` and are not
committed.

Run:

```bash
tests/typescript-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
```

The fixtures cover TypeScript declarations, the argument-taking `p("of")` and
`n("get"|"set")` parser helpers, nested template state, regex disambiguation,
strict scopes, and automatic semicolon insertion.
