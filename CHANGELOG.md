# Changelog

## Unreleased

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
- Generated lexers and parsers must be regenerated with the matching
  `antlr4-rust-gen` release.
