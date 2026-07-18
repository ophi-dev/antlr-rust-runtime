# Migration Notes

`antlr-rust-runtime` is pre-1.0. Minor releases may include breaking runtime and
generator changes. Generated lexers and parsers must use the same release of
`antlr4-rust-gen` as the runtime.

## Recognizer Reuse Method Names

Generated parsers now reserve `reset`, `set_token_stream`,
`token_stream_mut`, and `clear_dfa` for recognizer reuse. Grammar rules that
normalize to one of those Rust names gain the usual `_rule` suffix after
regeneration, such as `reset_rule()`.

## Compact Token, Tree, and Prediction Stores

The compact token, flat CST, and prediction-context stores replace the previous
pointer-owned APIs. Code generated against the older token or recursive tree
APIs does not compile against this runtime and must be regenerated.

`CommonToken`, `TokenRef`, and token factories are removed. Custom token sources
now append a `TokenSpec` directly to the supplied `TokenSink` and return its
`TokenId`. Buffered-token consumers use borrowing `TokenView` values from
`get`, `lt`, or the `tokens()` iterator. Custom `CharStream` implementations
should provide `source_text()` when the complete UTF-8 input can be shared;
otherwise token text is stored explicitly in the sparse side pool.

`CommonTokenStream` owns its `TokenStore` directly. `BaseParser` owns one
`ParseTreeStorage`: nodes are addressed by `NodeId`, every rule child list is a
range in one shared edge pool, and terminal/error records contain only
`TokenId`. `Node`, `RuleNodeView`, and terminal/error views borrow the stores;
there is no recursive `ParserRuleContext` ownership graph or legacy
materializer.

Generated `parse()` returns `ParsedFile`, which owns the token store, flat CST,
and root ID. Access the root through `tree()`, inspect storage metrics through
`storage().stats()`, or resolve another ID through `node()`. Direct rule calls
return `NodeId`; use `parser.node(id)` while the parser is alive, or consume the
parser with `into_parsed_file(id)`.

Parser prediction contexts are compact and store-local. `ContextId` replaces
the exported recursive `PredictionContext` graph; singleton records live
directly in a shared arena and array payloads use shared parent and return-state
pools. Each `ParserAtnSimulator` owns that arena together with its learned
parser DFAs, and remaps context IDs before combining independently learned
stores. `prediction_context_stats()` exposes arena allocation and interner
totals, retained capacities, workspace usage, and outer-context cache activity
for measurement.

Learned parser DFAs are also opaque, compact stores. `Dfa` and the mutable
field-oriented `DfaState` API are removed. Use `ParserDfa::state_count`,
`ParserDfa::states`, `ParserDfa::transitions`, and borrowing
`ParserDfaStateView` values for diagnostics. State and transition targets are
identified by `DfaStateId`; ATN configuration sets remain internal cold data.
`ParserAtnSimulator::parser_dfa_stats()` reports dense/sparse row distribution,
hot/cold retained bytes, and state-interner measurements.

## Packed Parser ATNs

Parser ATNs now use `ParserAtn`, a validated packed word stream with checked
compact IDs, contiguous transition ranges, and pooled interval data. Generated
parsers embed this versioned stream directly and borrow it without rebuilding
an object graph. `ParserAtn::from_static` rejects bad magic, byte order,
versions, section lengths, offsets, and indices; it never falls back to the old
representation.

The old parser-facing `Atn`, `AtnState`, and `Transition` graph APIs are
removed. The graph retained for lexer simulation is now explicitly named
`LexerAtn`, `LexerAtnState`, and `LexerTransition`. Borrow parser diagnostics
through `ParserAtnState`, `ParserTransition`, and their iterators instead of
materializing owned records.

Parser `GrammarMetadata::serialized_atn()` is empty because the generated
module carries `PARSER_ATN_DATA` as its single parser-ATN artifact. Code that
needs parser ATN diagnostics must use the module's `parser_atn()` function (or
`GeneratedParser::parser_atn()`) and the runtime borrowing views rather than
re-deserializing metadata.

Regenerate lexers and parsers with the matching `antlr4-rust-gen` release.
Older generated parsers do not contain the packed parser format and are
intentionally source- and data-incompatible with this runtime. A format
mismatch reports both the generated version and the runtime-supported range;
there is no compatibility repacker.

Token IDs cover indices through `u32::MAX`. Source scalar/byte offsets, line
numbers, and columns are limited to `u32::MAX - 1` (4,294,967,294);
`u32::MAX` is reserved for ANTLR's synthetic `-1` boundary. All conversions are
checked. Use `CommonTokenStream::try_new` or `try_with_channel` to handle limit
errors; `new` and `with_channel` panic with the same error.
