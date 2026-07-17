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
