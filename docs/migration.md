# Migration Notes

`antlr-rust-runtime` is pre-1.0. Minor releases may include breaking runtime and
generator changes. Generated lexers and parsers must use the same release of
`antlr4-rust-gen` as the runtime.

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
totals for measurement.

Token IDs cover indices through `u32::MAX`. Source scalar/byte offsets, line
numbers, and columns are limited to `u32::MAX - 1` (4,294,967,294);
`u32::MAX` is reserved for ANTLR's synthetic `-1` boundary. All conversions are
checked. Use `CommonTokenStream::try_new` or `try_with_channel` to handle limit
errors; `new` and `with_channel` panic with the same error.
