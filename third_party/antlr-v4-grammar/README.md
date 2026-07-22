# ANTLR v4 grammar frontend

These files are pinned from
`https://github.com/mike-lischke/antlr-ng.git` at commit
`1f68422ae4bfc62f93343769e144d01f305487b1`.

The upstream per-file BSD notices and repository `License.txt` are retained.
The source changes are:

- removal of the TypeScript-only `@header` block from `ANTLRv4Lexer.g4`; the
  Rust frontend supplies `LexerAdaptor` behavior in
  `src/bin_support/grammar/lexer_adaptor.rs`;
- acceptance of a bare `RULE_REF` in `lexerAtom`, matching Java ANTLR's
  `ANTLRParser.g`, so the semantic pipeline can issue
  `PARSER_RULE_REF_IN_LEXER_RULE`.

The checked-in frontend is a direct self-hosting fixed point. Regenerate and
verify it without Java or Node.js:

```text
tools/grammar-frontend/update-stage0.sh --check
```

The script:

1. builds the checked-in frontend as Stage 0;
2. compiles these grammars into Stage 1 with the source-only generator;
3. builds Stage 1 in an isolated source copy and compiles Stage 2 there;
4. requires Stage 1 and Stage 2, including `semantics.json`, to be
   byte-identical without header normalization;
5. runs the pinned nine-file frontend corpus and malformed fail-closed cases
   against Stage 1; and
6. compares Stage 1 with the checked-in files.

The direct generator invocation includes:

```text
third_party/antlr-v4-grammar/ANTLRv4Lexer.g4
third_party/antlr-v4-grammar/ANTLRv4Parser.g4
--lib third_party/antlr-v4-grammar
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

Their hashes, the intermediate `.interp` hashes, the seed JDK, and the exact
legacy commands are retained in `stage0-manifest.json` as the initial bootstrap
record. They are not part of normal regeneration. Current source and generated
hashes are recorded in `self-hosted.sha256`. Use `--update` instead of `--check`
only when intentionally accepting a new tested fixed point.
