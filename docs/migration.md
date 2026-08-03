# Migration Notes

`antlr-rust-runtime` is pre-1.0. Minor releases may include breaking runtime and
generator changes. Using the same release of `antlr4-rust-gen` and
`antlr-rust-runtime` remains recommended. Newly generated modules also carry a
generated-code API revision that is checked against the selected runtime at
compile time, so releases that deliberately preserve the source contract can
remain compatible without exact SemVer equality.

The current generator emits revision 3 so generated parser-action hooks can
observe parameterized rule arguments. This runtime continues to accept revision
1 and 2 generated modules because their required APIs remain supported.

Generated modules created before the compatibility check was introduced carry
no enforceable revision. Regenerate every committed lexer and parser once when
first adopting a release with this mechanism. If a later build reports a
generated-code API mismatch, either regenerate the named module with a
compatible generator or select a runtime that accepts its requested revision.
When new generated source is compiled against a runtime that predates the
check, Rust reports the missing generated-code API macro instead; upgrade that
runtime or regenerate with its matching older generator.
`antlr4-rust-gen --version` reports the generator package version for auditing
and reproducible regeneration.

## Generator Package and Build Scripts

`antlr4-rust-gen` no longer ships from the runtime package or behind its
removed `codegen` feature. Install the command from its companion package:

```bash
cargo install antlr-rust-codegen --bin antlr4-rust-gen
```

Rust build scripts can instead use the same generation pipeline as a library:

<!-- x-release-please-start-version -->

```toml
[dependencies]
antlr-rust-runtime = "0.27.0"

[build-dependencies]
antlr-rust-codegen = "0.27.0"
```

<!-- x-release-please-end -->

Generate only into Cargo's `OUT_DIR` with `antlr_rust_codegen::Builder`, then
call `Generation::emit_rerun_if_changed()` to track the resolved roots,
imports, and token vocabularies. Projects that commit generated modules need
only the runtime dependency and can keep generation in an `xtask` or the CLI.
This package move does not change generated-code API revision 3.

## Structured Syntax Error Events and Byte Spans

`ErrorListener::syntax_error` now receives one `&SyntaxErrorEvent<'_>` instead
of separate offending-token, line, column, message, and error arguments:

```rust
// Before
fn syntax_error(
    &mut self,
    recognizer: &R,
    offending: Option<TokenView<'_>>,
    line: usize,
    column: usize,
    message: &str,
    error: Option<&AntlrError>,
);

// After
fn syntax_error(&mut self, recognizer: &R, event: &SyntaxErrorEvent<'_>);
```

Read `event.span` for the resolved half-open UTF-8 byte range. Lexer failures
and parser diagnostics use the same event shape; streams and token sources that
cannot resolve byte offsets leave the span as `None`.

`Token::start_byte()` and `stop_byte()` now return `Option<usize>`, while
`byte_span()` returns `Option<Range<usize>>`. `None` means the token source
could not resolve exact byte offsets. Custom token sources must set
Unicode-scalar and UTF-8 byte positions independently:

```rust
TokenSpec::explicit(token_type, text)
    .with_span(scalar_start, scalar_stop)
    .with_byte_span(byte_start, byte_end)
```

`TokenSpec::with_span` no longer assumes scalar indexes are byte offsets.
Omit `with_byte_span` when no exact mapping exists.

`TokenSourceError` gained an optional `span` and, like `SyntaxErrorEvent`, is
non-exhaustive. Construct token-source diagnostics with
`TokenSourceError::new(...).with_span(...)` instead of a struct literal.

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

`TokenView::text()` now returns `Option<&str>`, matching `Token::text()` for
both concrete and generic receivers. Code that intentionally treats missing
token text as empty can use `TokenView::text_or_empty()`; otherwise handle the
`None` case explicitly.

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
parser with `into_parsed_file(id)`. Iterate every retained token, including
hidden-channel tokens and EOF, with `parsed.tokens().iter()` or
`for token in parsed.tokens()`.

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
