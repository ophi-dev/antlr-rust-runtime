# ANTLR v4 grammar frontend seed

These files are pinned from
`https://github.com/mike-lischke/antlr-ng.git` at commit
`1f68422ae4bfc62f93343769e144d01f305487b1`.

The upstream per-file BSD notices and repository `License.txt` are retained.
The only source change is removal of the TypeScript-only `@header` block from
`ANTLRv4Lexer.g4`; the Rust frontend supplies `LexerAdaptor` behavior in
`src/bin_support/grammar/lexer_adaptor.rs`.

Stage 0 is seeded with ANTLR 4.13.2 and the legacy Rust generator:

```text
tools/grammar-frontend/update-stage0.sh --check
```

The generator invocations include:

```text
--sem-patterns third_party/antlr-v4-grammar/antlr-v4.toml
--option-hook superClass=LexerAdaptor
--sem-unknown error
--require-full-semantics
--require-generated-parser
```

Expected generated files:

```text
src/bin_support/grammar/generated/antlr_v4_lexer.rs
src/bin_support/grammar/generated/antlr_v4_parser.rs
```

Their hashes are recorded by `tools/grammar-frontend/update-stage0.sh` in
`stage0-manifest.json`.
