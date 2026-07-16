# Changelog

## Unreleased

### Breaking

- Buffered tokens now live once in a compact `TokenStore` and are addressed by
  `TokenId`; public access uses borrowing `TokenView` values.
- `CommonTokenStream` owns its `TokenStore` directly, parse-tree nodes store
  only `TokenId`, and token-dependent tree APIs take `&TokenStore`.
- Generated `parse()` helpers return `ParsedFile<R>` so completed trees retain
  ownership of their canonical token store.
- `CommonToken`, `TokenRef`, and token factories are removed. `TokenSource`
  implementations write directly to `TokenSink`.
- Speculative parser nodes, child sequences, recovery diagnostics, and uncommon
  payloads now live in one parser-owned, index-addressed recognition arena.
  `RecognitionArenaStats` reports total/live/dead records and retained
  capacities for the latest interpreted rule parse.
- Generated lexers and parsers must be regenerated with the matching
  `antlr4-rust-gen` release.
