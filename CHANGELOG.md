# Changelog

## [0.17.1](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.17.0...v0.17.1) (2026-07-24)


### Bug Fixes

* **codegen:** support capitalized lexer command aliases ([e0f032b](https://github.com/ophi-dev/antlr-rust-runtime/commit/e0f032b590fe33f32f5a32607e2ad2589d3dba2b))

## [0.17.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.16.0...v0.17.0) (2026-07-24)


### Features

* **runtime:** add ByteStream for binary parsing + MIDI example ([#188](https://github.com/ophi-dev/antlr-rust-runtime/issues/188)) ([60a92be](https://github.com/ophi-dev/antlr-rust-runtime/commit/60a92be0e721e5f97a57c2130a3c68caecf74d49))
* support parse-tree XPath queries ([#186](https://github.com/ophi-dev/antlr-rust-runtime/issues/186)) ([5969f40](https://github.com/ophi-dev/antlr-rust-runtime/commit/5969f40d7ff87920981d0715dbebed32fc1bb737))

## [0.16.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.15.2...v0.16.0) (2026-07-24)


### Features

* add grammar-driven C# parity support ([#182](https://github.com/ophi-dev/antlr-rust-runtime/issues/182)) ([37be5d7](https://github.com/ophi-dev/antlr-rust-runtime/commit/37be5d72314f1ed36336305bb01dc11d47c711f5))


### Bug Fixes

* **runtime:** isolate interpreter prefix fallback ([8dbde5c](https://github.com/ophi-dev/antlr-rust-runtime/commit/8dbde5c729588112600df7201f111481d55b96b7))
* **runtime:** match Phase C ANTLR behavior ([1b6e4db](https://github.com/ophi-dev/antlr-rust-runtime/commit/1b6e4db8a92f11cb0f700aba3138ba91e743366c))

## [0.15.2](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.15.1...v0.15.2) (2026-07-23)


### Bug Fixes

* grouped token accessors in typed contexts ([#178](https://github.com/ophi-dev/antlr-rust-runtime/issues/178)) ([f549f7c](https://github.com/ophi-dev/antlr-rust-runtime/commit/f549f7ce74774639022a347eccfb843a59aae26a))

## [0.15.1](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.15.0...v0.15.1) (2026-07-23)


### Bug Fixes

* restore fast Java parsing with typed contexts ([#175](https://github.com/ophi-dev/antlr-rust-runtime/issues/175)) ([865ac8f](https://github.com/ophi-dev/antlr-rust-runtime/commit/865ac8f768656f9a0308399fc8d9fbd1e277fad1))

## [0.15.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.14.2...v0.15.0) (2026-07-23)


### Features

* add mehen ([9567df6](https://github.com/ophi-dev/antlr-rust-runtime/commit/9567df6ee7da12c2e4340a9ad5d7e94c9c6dfbd5))
* add typed listeners, visitors, and traversal ([#165](https://github.com/ophi-dev/antlr-rust-runtime/issues/165)) ([c240714](https://github.com/ophi-dev/antlr-rust-runtime/commit/c2407142a92ced988b7984852eb5b67e99e30d89))
* **codegen:** make .g4 the sole production input ([#163](https://github.com/ophi-dev/antlr-rust-runtime/issues/163)) ([0a250ab](https://github.com/ophi-dev/antlr-rust-runtime/commit/0a250ab7d3eb17191890bafb20e039063fc65170))
* implement Phase A direct .g4 grammar frontend ([#152](https://github.com/ophi-dev/antlr-rust-runtime/issues/152)) ([40a4560](https://github.com/ophi-dev/antlr-rust-runtime/commit/40a4560a499ab4984a9249b15e5de2e2f37a83f9))
* implement Phase B direct .g4 source-to-ATN compiler ([#157](https://github.com/ophi-dev/antlr-rust-runtime/issues/157)) ([993dd48](https://github.com/ophi-dev/antlr-rust-runtime/commit/993dd48c99254e146df6c677d7938779ed91ef80))
* make TokenStore iterable ([#166](https://github.com/ophi-dev/antlr-rust-runtime/issues/166)) ([2ee56fa](https://github.com/ophi-dev/antlr-rust-runtime/commit/2ee56fad866619d2683a75d2f04c27abc9a8799c))


### Bug Fixes

* add fetch-depth: 0 ([ddc701a](https://github.com/ophi-dev/antlr-rust-runtime/commit/ddc701a44301d9f9917274b4e45add863d373940))
* CC plugin name ([32f2829](https://github.com/ophi-dev/antlr-rust-runtime/commit/32f2829f2ad1b0a2184f51c3fd0410bf14ebb8b4))
* cc review action ([bd24370](https://github.com/ophi-dev/antlr-rust-runtime/commit/bd243709d84f4efb3dfa6e5cfcb0d09bf3fc08c0))
* **parser:** force progress after repeated recovery ([#155](https://github.com/ophi-dev/antlr-rust-runtime/issues/155)) ([b785f96](https://github.com/ophi-dev/antlr-rust-runtime/commit/b785f96a1d4f35ac8da937630a8c9d936c167b3d))
* remove Github Token ([c908162](https://github.com/ophi-dev/antlr-rust-runtime/commit/c908162dbfb52aac6a73bd2e143b3291c7325433))
* **tokens:** align TokenView text semantics ([#167](https://github.com/ophi-dev/antlr-rust-runtime/issues/167)) ([c56a8c4](https://github.com/ophi-dev/antlr-rust-runtime/commit/c56a8c457e88276d4dcad1a686d3133c76a444e9))

## [0.14.2](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.14.1...v0.14.2) (2026-07-21)


### Bug Fixes

* **parser:** bound speculative recognition stack ([#147](https://github.com/ophi-dev/antlr-rust-runtime/issues/147)) ([84f365e](https://github.com/ophi-dev/antlr-rust-runtime/commit/84f365e9d1a675f6d92e37a18afd3231a2883035))

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
