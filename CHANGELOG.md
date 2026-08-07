# Changelog

## Unreleased

### Breaking Changes

* Bump the generated/runtime API contract to revision 7 for the neutral-parse
  transaction and resume methods used by opt-in shared-descent dispatch.
* Move `antlr4-rust-gen` and the removed runtime `codegen` feature to the
  `antlr-rust-codegen` package. Install the binary from that package or add it
  as a build dependency and use `Builder`; that package move did not itself
  change generated/runtime API revision 3.
* Replace `antlr4-rust-gen`'s handwritten argument parser with `clap` and remove
  the ANTLR-style `-listener`, `-no-listener`, `-visitor`, and `-no-visitor`
  spellings. Use the corresponding double-dash options.

### Features

* Add opt-in `--shared-descent` parser dispatch with deterministic candidate
  reporting, neutral-parse rollback/resume support, and runtime counters.

## [0.31.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.30.0...v0.31.0) (2026-08-06)


### ⚠ BREAKING CHANGES

* **codegen:** semantic pattern files must now be valid TOML. String-valued fields require quotes, and malformed sections, trailing junk, and unknown fields fail generation instead of being accepted by the former permissive scalar reader.

### Features

* **codegen:** parse semantic patterns with generated TOML ([#313](https://github.com/ophi-dev/antlr-rust-runtime/issues/313)) ([e9b259a](https://github.com/ophi-dev/antlr-rust-runtime/commit/e9b259a5aeee57a0e9ced09b9c50efa155d1a6a4))
* **codegen:** run trusted Rust support bundles ([#304](https://github.com/ophi-dev/antlr-rust-runtime/issues/304)) ([debc33d](https://github.com/ophi-dev/antlr-rust-runtime/commit/debc33d72d5cad5b638cb7468bccad7759406729))


### Bug Fixes

* **deps:** update rust crate toml to v1 ([c210f81](https://github.com/ophi-dev/antlr-rust-runtime/commit/c210f819d37f81574fa7e89c3c6c767b3151855f))

## [0.30.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.29.0...v0.30.0) (2026-08-05)


### Features

* **codegen:** add TestRig command ([#302](https://github.com/ophi-dev/antlr-rust-runtime/issues/302)) ([e03e75c](https://github.com/ophi-dev/antlr-rust-runtime/commit/e03e75c87fdab5e1644a2e3ade2a46924bbe3f75))

## [0.29.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.28.0...v0.29.0) (2026-08-04)


### ⚠ BREAKING CHANGES

* **codegen:** centralize generated rule lifecycle ([#301](https://github.com/ophi-dev/antlr-rust-runtime/issues/301))
* **codegen:** centralize recognizer facades ([#300](https://github.com/ophi-dev/antlr-rust-runtime/issues/300))
* **codegen:** centralize typed context mechanics ([#299](https://github.com/ophi-dev/antlr-rust-runtime/issues/299))
* **codegen:** parse CLI with clap ([#297](https://github.com/ophi-dev/antlr-rust-runtime/issues/297))

### refactor

* **codegen:** parse CLI with clap ([#297](https://github.com/ophi-dev/antlr-rust-runtime/issues/297)) ([4bdb7e2](https://github.com/ophi-dev/antlr-rust-runtime/commit/4bdb7e2839eb2c11e84a3c6687bb3f51a8601653))


### Features

* **codegen:** centralize generated rule lifecycle ([#301](https://github.com/ophi-dev/antlr-rust-runtime/issues/301)) ([2c4b985](https://github.com/ophi-dev/antlr-rust-runtime/commit/2c4b98574570617f5366f9b1a398fe3f7c0d5e5e))
* **codegen:** centralize recognizer facades ([#300](https://github.com/ophi-dev/antlr-rust-runtime/issues/300)) ([a50acb6](https://github.com/ophi-dev/antlr-rust-runtime/commit/a50acb66791ecafaf2d301bd2061376bbc92c278))
* **codegen:** centralize typed context mechanics ([#299](https://github.com/ophi-dev/antlr-rust-runtime/issues/299)) ([920bb67](https://github.com/ophi-dev/antlr-rust-runtime/commit/920bb67f41109193a999adc50e000748cf6f06b3))

## [0.28.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.27.0...v0.28.0) (2026-08-04)


### Features

* **codegen:** expose structured generation diagnostics ([#294](https://github.com/ophi-dev/antlr-rust-runtime/issues/294)) ([ef1bdd0](https://github.com/ophi-dev/antlr-rust-runtime/commit/ef1bdd025fdb5fbcfdd6296dce329e9ab65859b3))

## [0.27.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.26.0...v0.27.0) (2026-08-03)


### ⚠ BREAKING CHANGES

* antlr4-rust-gen and the runtime codegen feature move to the antlr-rust-codegen package.

### refactor

* split codegen into workspace packages ([#290](https://github.com/ophi-dev/antlr-rust-runtime/issues/290)) ([a44f645](https://github.com/ophi-dev/antlr-rust-runtime/commit/a44f6459d2642ac0db340004c89ed5c5ff899e57))

## [0.26.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.25.0...v0.26.0) (2026-08-03)


### Features

* **codegen:** enforce generated runtime compatibility ([#274](https://github.com/ophi-dev/antlr-rust-runtime/issues/274)) ([39a09a8](https://github.com/ophi-dev/antlr-rust-runtime/commit/39a09a8acc41406ba6c3e460b9f54d4a32bad3a4))
* **codegen:** run named parser actions at committed positions ([#287](https://github.com/ophi-dev/antlr-rust-runtime/issues/287)) ([dee1944](https://github.com/ophi-dev/antlr-rust-runtime/commit/dee1944d728287baf2f793122ed2326bf31336d5))
* **codegen:** support antlr4rust embedded parser surface ([#270](https://github.com/ophi-dev/antlr-rust-runtime/issues/270)) ([f85c971](https://github.com/ophi-dev/antlr-rust-runtime/commit/f85c971d913d9f544f0b6609c4d55a3a97755b4c))


### Bug Fixes

* **deps:** update rust crate intl to 0.6.0 ([#268](https://github.com/ophi-dev/antlr-rust-runtime/issues/268)) ([5bae7bf](https://github.com/ophi-dev/antlr-rust-runtime/commit/5bae7bfa2d73a4509052847f9de674f0afa943f1))


### Performance Improvements

* **codegen:** omit adaptive fallback from complete LL(1) dispatch ([#285](https://github.com/ophi-dev/antlr-rust-runtime/issues/285)) ([90a5052](https://github.com/ophi-dev/antlr-rust-runtime/commit/90a50520b2395fafe765096d1edac66eb67d6bcc))

## [0.25.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.24.0...v0.25.0) (2026-08-01)


### Features

* **codegen:** expose direct context terminals ([#271](https://github.com/ophi-dev/antlr-rust-runtime/issues/271)) ([d090dd5](https://github.com/ophi-dev/antlr-rust-runtime/commit/d090dd563a574d4bd71daf952175234fb36ce641))

## [0.24.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.23.0...v0.24.0) (2026-07-31)


### Features

* **codegen:** add validated parse tree surface ([#260](https://github.com/ophi-dev/antlr-rust-runtime/issues/260)) ([bb9a3ca](https://github.com/ophi-dev/antlr-rust-runtime/commit/bb9a3ca75356481e6afbcc7158bb815f633220f1))
* **codegen:** diagnose and prune unreachable rules ([#264](https://github.com/ophi-dev/antlr-rust-runtime/issues/264)) ([4c4df20](https://github.com/ophi-dev/antlr-rust-runtime/commit/4c4df205fe4726c0bd22aaaaaf509a89b160dd54))

## [0.23.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.22.0...v0.23.0) (2026-07-29)


### ⚠ BREAKING CHANGES

* **runtime:** ErrorListener::syntax_error now receives SyntaxErrorEvent; Token byte accessors return Option values; TokenSourceError includes span and diagnostic structs are non-exhaustive.

### Features

* **codegen:** collapse linear precedence ladders ([#252](https://github.com/ophi-dev/antlr-rust-runtime/issues/252)) ([5e109c8](https://github.com/ophi-dev/antlr-rust-runtime/commit/5e109c893b2f90ab91dc9b36f88fb7e903d426f9))
* **runtime:** expose resolved byte spans to error listeners ([#257](https://github.com/ophi-dev/antlr-rust-runtime/issues/257)) ([b91b4ca](https://github.com/ophi-dev/antlr-rust-runtime/commit/b91b4ca55aaf0f097d5fc195fcafd1e8a19474c6))
* support antlr4rust recog predicate receiver ([#249](https://github.com/ophi-dev/antlr-rust-runtime/issues/249)) ([45c56f4](https://github.com/ophi-dev/antlr-rust-runtime/commit/45c56f42428c0cc0464da72afbfdf490a110a4a6))

## [0.22.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.21.0...v0.22.0) (2026-07-29)


### Features

* **codegen:** per-decision tiers, decisions.json report, opt-in --fixed-lookahead static dispatch ([#150](https://github.com/ophi-dev/antlr-rust-runtime/issues/150)) ([#247](https://github.com/ophi-dev/antlr-rust-runtime/issues/247)) ([e54db40](https://github.com/ophi-dev/antlr-rust-runtime/commit/e54db40454bd960abe7acd6b182ec51ca33dd269))


### Bug Fixes

* remove max turns and challenge ([5709d03](https://github.com/ophi-dev/antlr-rust-runtime/commit/5709d038044249ac5ce1e64231e1d0b40460cc03))
* restore official Claude Code review action ([bd743f5](https://github.com/ophi-dev/antlr-rust-runtime/commit/bd743f5dccf1a07cd6784cbfb892cfe0cde20b39))

## [0.21.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.20.1...v0.21.0) (2026-07-28)


### ⚠ BREAKING CHANGES

* **codegen:** accept mutual (indirect) left recursion via hub inlining ([#151](https://github.com/ophi-dev/antlr-rust-runtime/issues/151)) (#221)

### Features

* **codegen:** accept mutual (indirect) left recursion via hub inlining ([#151](https://github.com/ophi-dev/antlr-rust-runtime/issues/151)) ([#221](https://github.com/ophi-dev/antlr-rust-runtime/issues/221)) ([61930e9](https://github.com/ophi-dev/antlr-rust-runtime/commit/61930e9ad05d04b7d801b5650b516c235a01ddd0))


### Bug Fixes

* clamp max turns for code review prompt ([f5e2f85](https://github.com/ophi-dev/antlr-rust-runtime/commit/f5e2f852941c7f6a1a45016e4d2bfa9a77c4acdc))
* **codegen:** diagnose left-recursive lexer rules ([#243](https://github.com/ophi-dev/antlr-rust-runtime/issues/243)) ([e562413](https://github.com/ophi-dev/antlr-rust-runtime/commit/e562413ca6fc9babe67f852365ae2296d65781ad)), closes [#236](https://github.com/ophi-dev/antlr-rust-runtime/issues/236)
* **codegen:** retain safe shared-label accessors ([#240](https://github.com/ophi-dev/antlr-rust-runtime/issues/240)) ([78506d1](https://github.com/ophi-dev/antlr-rust-runtime/commit/78506d1c094ea119d9a6d3dc5558062130626399))
* unrelated extends removed ([9454ab9](https://github.com/ophi-dev/antlr-rust-runtime/commit/9454ab9fea7e83928fc3d8b81a30a87b2a06e2b7))

## [0.20.1](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.19.1...v0.20.1) (2026-07-27)


### chore

* release 0.20.1 ([09cf717](https://github.com/ophi-dev/antlr-rust-runtime/commit/09cf7179b8b66dec040be6a413b3be814a24158f))


### Features

* **semir:** support stack-valued member state so inline [@lexer](https://github.com/lexer)::members grammars need no hooks ([#226](https://github.com/ophi-dev/antlr-rust-runtime/issues/226)) ([8f961ff](https://github.com/ophi-dev/antlr-rust-runtime/commit/8f961ff71055e20809f18876cc6aeb19d95ca685)), closes [#206](https://github.com/ophi-dev/antlr-rust-runtime/issues/206)


### Bug Fixes

* **codegen:** expose combined literal token constants ([#228](https://github.com/ophi-dev/antlr-rust-runtime/issues/228)) ([f25c79b](https://github.com/ophi-dev/antlr-rust-runtime/commit/f25c79b6b5acb3e571623218abe4f31b49cb4daa))
* **codegen:** keep token labels stable through recovery ([#235](https://github.com/ophi-dev/antlr-rust-runtime/issues/235)) ([d08d62f](https://github.com/ophi-dev/antlr-rust-runtime/commit/d08d62f5213c0446bba0aa39c1067671e958397a)), closes [#213](https://github.com/ophi-dev/antlr-rust-runtime/issues/213)
* **parser:** notify listeners for fatal entry errors ([#234](https://github.com/ophi-dev/antlr-rust-runtime/issues/234)) ([332251a](https://github.com/ophi-dev/antlr-rust-runtime/commit/332251a101cf95264db5ef6bcc42766d4d9415d3))


### Performance Improvements

* **codegen:** derive stored invocation states lazily ([#227](https://github.com/ophi-dev/antlr-rust-runtime/issues/227)) ([541311e](https://github.com/ophi-dev/antlr-rust-runtime/commit/541311ef08d5b3c8dd30acb0c2ea865c5aa34a61))
* **codegen:** lower untranslated parser predicates as generatable Unknown templates ([#218](https://github.com/ophi-dev/antlr-rust-runtime/issues/218)) ([84fda7d](https://github.com/ophi-dev/antlr-rust-runtime/commit/84fda7db278b66ca8936d09cfc3f55fbf2549236))
* **codegen:** route costly left-recursive parses through the ATN ([#231](https://github.com/ophi-dev/antlr-rust-runtime/issues/231)) ([32bf468](https://github.com/ophi-dev/antlr-rust-runtime/commit/32bf46832117fe54e835e9d463022228397e3d89))
* **runtime:** share generated recognizer metadata ([#220](https://github.com/ophi-dev/antlr-rust-runtime/issues/220)) ([1098f6f](https://github.com/ophi-dev/antlr-rust-runtime/commit/1098f6f0809c2f9c2510d2cdd2018a1bc3337a34))

## [0.19.1](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.19.0...v0.19.1) (2026-07-26)


### Bug Fixes

* **codegen:** union token sets for a group label shared across alternatives ([#211](https://github.com/ophi-dev/antlr-rust-runtime/issues/211)) ([d55e447](https://github.com/ophi-dev/antlr-rust-runtime/commit/d55e447cba54ecbe4e8b434422e7ba49cbe3b40a))
* **frontend:** accept a UTF-8 BOM in grammar sources ([#215](https://github.com/ophi-dev/antlr-rust-runtime/issues/215)) ([c987350](https://github.com/ophi-dev/antlr-rust-runtime/commit/c987350d1a26390bdbdc8b9d0527537359f229cd)), closes [#212](https://github.com/ophi-dev/antlr-rust-runtime/issues/212)


### Performance Improvements

* **prediction:** memoize full-context LL resolutions by interned caller context ([#208](https://github.com/ophi-dev/antlr-rust-runtime/issues/208)) ([7eb9307](https://github.com/ophi-dev/antlr-rust-runtime/commit/7eb93072c4fd551db8ce7550e43c35f080f197c8))

## [0.19.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.18.0...v0.19.0) (2026-07-26)


### Features

* **parser:** configurable max rule-nesting depth to bound adversarial input ([#199](https://github.com/ophi-dev/antlr-rust-runtime/issues/199)) ([66ebc82](https://github.com/ophi-dev/antlr-rust-runtime/commit/66ebc8228ddad1097e8ee524149dd2c29b84a597))
* **runtime:** add_parse_listener — parse-time rule enter/exit events (ANTLR parity) ([#204](https://github.com/ophi-dev/antlr-rust-runtime/issues/204)) ([e823538](https://github.com/ophi-dev/antlr-rust-runtime/commit/e823538eed1784676ff10022221aafdf813870e2))
* **runtime:** pass the offending token to ErrorListener::syntax_error ([#196](https://github.com/ophi-dev/antlr-rust-runtime/issues/196)) ([dde6bf0](https://github.com/ophi-dev/antlr-rust-runtime/commit/dde6bf053f9e15ffde7809b6177eead1f7b0e60b))


### Bug Fixes

* **codegen:** guard generated rule dispatch against native stack overflow ([3634641](https://github.com/ophi-dev/antlr-rust-runtime/commit/36346412fd19f89032a84536285694c932a8c5c0)), closes [#193](https://github.com/ophi-dev/antlr-rust-runtime/issues/193)

## [0.18.0](https://github.com/ophi-dev/antlr-rust-runtime/compare/v0.17.0...v0.18.0) (2026-07-25)


### Features

* support parse-tree pattern matching ([#192](https://github.com/ophi-dev/antlr-rust-runtime/issues/192)) ([619ebcb](https://github.com/ophi-dev/antlr-rust-runtime/commit/619ebcbe242b11080809f189f9d5935bef5294de))


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
