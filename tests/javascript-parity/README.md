# JavaScript lexer/parser parity

This smoke test builds the unmodified official
`antlr/grammars-v4/javascript/javascript` grammar for Rust and compares its
tokens and parse trees byte-for-byte with the official Python target.

The checked-in `dumper/src/javascript_lexer_base.rs` and
`dumper/src/javascript_parser_base.rs` are the Rust equivalents of the
grammar-specific base classes shipped by grammars-v4. Generated recognizer
modules are copied into `dumper/src/generated/` by `run.sh` and are not
committed.

Run:

```bash
python3 -m pip install antlr4-python3-runtime==4.13.2
tests/javascript-parity/run.sh \
  --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
  --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
```

The fixtures cover hashbang start-of-file handling, regex/division
disambiguation, strict scopes, nested template braces, hidden-channel line
terminators, and the argument-taking `n("static"|"get"|"set")` parser helper.
