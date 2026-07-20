# Changelog

## [0.14.1](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.14.0...v0.14.1) (2026-07-20)


### Bug Fixes

* **parser:** prevent adaptive-set stack overflow ([405dde5](https://github.com/ophi-dev/antlr-rust-runtime/commit/405dde56716fcc94084d7a7c05ddff7861cac51e))

## [0.14.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.13.0...v0.14.0) (2026-07-20)


### Features

* scan compact ASCII lexer range classes ([#127](https://github.com/ophi-dev/antlr-rust-runtime/issues/127)) ([2b82653](https://github.com/ophi-dev/antlr-rust-runtime/commit/2b826537d392a3e94c828cae04c546ca66d7db0c))


### Bug Fixes

* make recovery diagnostics configurable ([#140](https://github.com/ophi-dev/antlr-rust-runtime/issues/140)) ([6f0d206](https://github.com/ophi-dev/antlr-rust-runtime/commit/6f0d2061600c4b41bed47b9df19911aae0cb901d))


### Performance Improvements

* **parser:** add adaptive token sets ([#133](https://github.com/ophi-dev/antlr-rust-runtime/issues/133)) ([12ca05c](https://github.com/ophi-dev/antlr-rust-runtime/commit/12ca05c4ad9fa8d97c5988c65478cd22ce6764a9))

## [0.13.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.12.0...v0.13.0) (2026-07-19)


### Features

* SIMD optimization ([#120](https://github.com/ophi-dev/antlr-rust-runtime/issues/120)) ([c8775c2](https://github.com/ophi-dev/antlr-rust-runtime/commit/c8775c2252d4b263588ed86f752bfa5c849010c9))

## [0.12.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.11.0...v0.12.0) (2026-07-18)

### Added

- Generated lexers and parsers now expose ANTLR-style recognizer reuse APIs:
  `set_input_stream`, `set_token_source`, `set_token_stream`, full `reset`,
  and `clear_dfa`. `CommonTokenStream::refill` supports re-feeding the lexer
  owned inside an existing parser without reconstructing either recognizer.

### Breaking

- Generated parser rules named `reset`, `setTokenStream`, `tokenStreamMut`, or
  `clearDfa` now gain a `_rule` suffix to avoid the recognizer reuse methods.

## [0.11.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.10.0...v0.11.0) (2026-07-18)

### Performance

- Compiled lexers read in-memory ASCII directly from their static DFA tables
  and commit accepted spans in bulk. Optional `CharStream` fast paths preserve
  scalar fallback behavior for custom streams and Unicode input.

### Breaking

- Buffered tokens now live once in a compact `TokenStore` and are addressed by
  `TokenId`; public access uses borrowing `TokenView` values.
- `CommonTokenStream` owns its `TokenStore` directly. `BaseParser` owns one flat
  `ParseTreeStorage`; `NodeId` addresses compact records, rule children are
  pooled ranges, and terminal/error records store only `TokenId`.
- Recursive owning `ParseTree`, `RuleNode`, `ParserRuleContext` children, and
  terminal token wrappers are removed. Public tree access uses borrowing
  `Node`, `RuleNodeView`, `TerminalNodeView`, and `ErrorNodeView` values.
- Generated typed contexts are borrowing views, and listener traversal runs
  iteratively over flat storage without recreating recursive context objects.
- Generated `parse()` helpers return `ParsedFile`, which owns the token store,
  flat CST storage, and root ID. Direct rule methods return `NodeId`.
- `CommonToken`, `TokenRef`, and token factories are removed. `TokenSource`
  implementations write directly to `TokenSink`.
- Speculative parser nodes, child sequences, recovery diagnostics, and uncommon
  payloads now live in one parser-owned, index-addressed recognition arena.
  `RecognitionArenaStats` reports total/live/dead records and retained
  capacities for the latest interpreted rule parse.
- Recursive `Rc<PredictionContext>` graphs and the exported
  `PredictionContext`/`AtnConfig` compatibility API are removed. Prediction
  contexts are canonical `ContextId` values in pooled storage owned together
  with learned parser DFAs; overlapping stores remap IDs before DFA union.
- Learned parser DFAs now use compact `DfaStateId` values, pooled dense/sparse
  edge rows, aligned hot accept tables, and a separate cold config store.
  Public `Dfa`/`DfaState` fields are removed in favor of opaque `ParserDfa`
  diagnostics and borrowing state views.
- Parser ATNs are versioned packed word streams with compact state/transition
  IDs, contiguous transition ranges, and pooled interval data. The parser
  object graph and its public `Atn`/`AtnState`/`Transition` types are removed;
  the remaining lexer graph is explicitly named `LexerAtn`,
  `LexerAtnState`, and `LexerTransition`.
- `ParserAtnSimulator::prediction_context_stats()` reports context creation,
  singleton/array distribution, pooled entries and bytes, and interner hits.
- `ParserAtnSimulator::parser_dfa_stats()` reports edge density, hot/cold
  retained bytes, and fingerprint-interner activity.
- Generated lexers and parsers must be regenerated with the matching
  `antlr4-rust-gen` release. Older generated parsers do not contain the packed
  parser format and are intentionally incompatible.
