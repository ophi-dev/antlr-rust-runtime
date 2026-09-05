# Migration Notes

`antlr-rust-runtime` is pre-1.0. Minor releases may include breaking runtime and
generator changes. Using the same release of `antlr4-rust-gen` and
`antlr-rust-runtime` remains recommended. Newly generated modules also carry a
generated-code API revision that is checked against the selected runtime at
compile time, so releases that deliberately preserve the source contract can
remain compatible without exact SemVer equality.

The current generator emits revision 16. Rules with an authored `catch [...]`
or `finally { ... }` clause now expand through two additional
`__antlr4_rust_generated_rule!` lifecycle sections: `exception (...)` (either
`none` or an authored handler `|name| { ... }` that replaces the default
report-and-recover behavior, matching ANTLR's generated catch replacement;
`name` is the single Rust identifier from the catch argument) and
`propagate { ... }` (the authored `finally` body for the propagated-failure
unwind; the success and recovery paths carry the `finally` body inline).
Neither section runs on the adaptive-retry unwind, whose re-entry executes the
rule from the top. Rules without exception clauses emit the unchanged
four-section form, and the runtime normalizes it to the same defaults, so
revision 12 to 15 recognizers remain compatible with this runtime. Regenerate
with revision 16 to execute authored `catch`/`finally` clauses; earlier
revisions silently dropped them.

Revision 16 also makes every authored target-code section accountable
(issue #355): `semantics.json` gains a per-grammar `sections` array covering
grammar-level and rule-level named actions plus `catch`/`finally` clauses,
each with a deterministic disposition (`embedded`, `translated`, or
`unsupported`). Unsupported sections warn by default and fail generation under
`--require-full-semantics` with a source-positioned diagnostic. Embedded
generation now also emits `@header` bodies at the top of the generated module
(before generated imports) and `@definitions` bodies at module scope (after
the `@members` module items), translated with the same token-alias machinery
as `@members`.

Revision 15 generated recognizers embed their
static data tables — the ahead-of-time compiled lexer DFA, the packed parser
ATN, and the serialized lexer ATN inside `GrammarMetadata` — as versioned
encoded blobs: LEB128 varints (zigzag-mapped for signed values) armored as
canonical unpadded base64 string literals, defined by the runtime's
`encoded` module. The blob carries a magic, format version, element kind, and a
checked element count; corrupt, truncated, overflowing, and unsupported data
fail with targeted diagnostics. This replaces the decimal integer arrays that
made rustc parse hundreds of thousands of integer expressions and produced
multi-megabyte single source lines for large grammars. The decoded integer
streams are byte-identical to the revision-14 arrays, so lexer/parser
behavior, diagnostics, and serialized metadata are unchanged; the embedded
lexer DFA keeps its documented fallback of rebuilding from the ATN when the
decoded stream comes from a different runtime version.

One hand-written-API consequence: `GrammarMetadata::serialized_atn` is no
longer a `const fn` (encoded-blob metadata decodes on first use, cached for
the metadata's lifetime). The generated-code API revision only gates generated
source, so hand-written downstream code calling it in a const context must
move that call to runtime.

Revision 12 to 14 generated recognizers remain compatible because the runtime
still implements their source APIs — `GrammarMetadata::new` with integer-array
ATN data, `CompiledLexerDfa::from_serialized`, and `ParserAtn::from_static` —
and reads packed parser ATN formats 1 through 3. Regenerate them with revision
15 or later to emit the compact encoded representation.

Revision 14 generated parsers embed packed parser
ATN format 3 with validated tail-call markers on rule transitions. Prediction
elides a marked return frame only when doing so preserves the configured SLL
accuracy policy, and committed full-context construction omits equivalent
caller frames before interning them. The default remains conservative; the
reduced-accuracy SLL policy requires an explicit simulator constructor.

The revision-14 runtime also extended the revision-11
`__antlr4_rust_parser_entry_points!` expansion with
`parse_with_parser_constructor` and
`parse_stream_with_parser_constructor`. These accept a caller-provided parser
constructor after token buffering, so generated typed semantic hooks can use the
shared parse driver. Their `GeneratedParseOutput` can be retained, converted
with `.into_parsed_file()`, or validated with `.validate()`. This is an additive
runtime-macro extension: generated source and the macro invocation are
unchanged, so the generated-code API remained revision 14 and existing revision
12-14 recognizers gain the helpers when linked against the updated runtime.
The generated alias widens compatibly to
`pub type TomlParserParseOutput<R, L, H = antlr4_runtime::NoSemanticHooks> =
antlr4_runtime::GeneratedParseOutput<R, TomlParser<L, H>>;`.

Revision 13 moved the iterative listener tree-walk engine into
`antlr4_runtime::generated::walk_generated`. Generated
parsers retain the same `<Grammar>TreeWalker`,
`<Grammar>ValidatedTreeWalker`, `ParseTreeWalker`, and listener APIs, but emit
only callback adapters for typed rule dispatch and terminal/error wrapping.
Traversal order, invocation-state push/pop behavior, and callback error
short-circuiting are unchanged.

Revision 12 generated recognizers remain compatible with this runtime because
their self-contained walkers use only runtime APIs that are still present.
Regenerate them with revision 13 to adopt the shared walker engine and reduce
generated source.

Revision 12 made generated parsers store their
uniform rule-body function pointers in one table and call the runtime-owned
`generated::dispatch_generated_rule` lifecycle guard. The per-rule
`parse_generated_rule_N_dispatch` wrappers are gone, ordinary routing is one
table lookup, and the exceptional ATN-preference/adaptive routes preserve their
existing conditions.

Revision 11 moved the parse-driver core and entry-point scaffolding every
generated parser used to re-declare into the runtime. The driver methods
(`parse_rule`, `parse_rule_precedence`,
`parse_rule_precedence_from_generated`, `parse_rule_precedence_inner`,
`parse_interpreted_rule`, `parse_interpreted_rule_precedence`) expand from the
runtime's `__antlr4_rust_parser_driver!` macro; the generated module supplies
only its interpreted-fallback call as a binder block, and action dispatch is
uniform — the driver always routes deferred actions through the module's
`run_action`, which is an empty method for grammars without action states.
The `parse` / `parse_validated` / `parse_with_parser` / `parse_stream` /
`parse_stream_validated` / `parse_stream_with_parser` functions and the
validation bridge expand from `__antlr4_rust_parser_entry_points!`, and
`<Grammar>ParserParseOutput` is now a type alias of the runtime's
`GeneratedParseOutput<R, P>` (`pub type TomlParserParseOutput<R, L> =
antlr4_runtime::GeneratedParseOutput<R, TomlParser<L>>;`) whose `validate()`
dispatches through the doc-hidden `__GeneratedParserValidate` trait the module
implements for its parser.
`GeneratedRuleError` moved to the runtime as a
grammar-agnostic type whose `AdaptiveRetry` variant always exists, the
adaptive-ATN retry state bundle became the runtime's
`AdaptiveAtnRetryState<const RULES: usize>` enabled by a const knob
(`AdaptiveAtnRetryState<0>` for grammars without residual adaptive routing;
its `retry_pending()` check constant-folds to `false`), and the generated
lexer `lex` / `lex_stream` functions are re-exports of the runtime's generic
functions. Public generated names, signatures, and behavior are preserved;
struct-literal construction and destructuring of `<Grammar>ParserParseOutput`
keep working through the alias.

Revision 10 moved the validated-parse surface every
generated parser used to re-declare (the validated-tree and validated-rule-node
types, the `FromValidatedRuleNode` trait, and the validation-error enum with
its `Display`/`Error`/`From` machinery) into the runtime as
`ValidatedTree<Grammar>`, `ValidatedRuleNode<'a, Grammar>`,
`FromValidatedRuleNode`, and `ValidationError`. Generated modules keep their
public names as type aliases branded with the module-local
`ValidatedTreeContext` marker
(`pub type TomlValidatedTree = antlr4_runtime::ValidatedTree<ValidatedTreeContext>;`),
plus a re-export of the `FromValidatedRuleNode` trait. Because of the brand,
validated trees and rule nodes of different grammars remain distinct types and
`downcast_ref` still cannot resolve a node against another grammar's contexts.

`ValidationError` is deliberately unbranded: every grammar's
`<Grammar>ValidationError` is now one shared runtime type, so a binary linking
several generated parsers compiles one copy of the error machinery and can
handle all their validation errors uniformly. The flip side: downstream code
that wrote one impl per grammar for these names — for example a `thiserror`
enum with separate `#[from]` variants for `TomlValidationError` and
`JavaValidationError`, or two `impl MyTrait for <Grammar>ValidationError`
blocks in one crate — now fails with `E0119` (conflicting implementations)
and must collapse those into a single impl of the shared type. Grammar-specific
detail (context and child names) stays in generated code as error-variant
data, and generated `validate_tree_structure` implementations delegate
repeated-child minimums to the runtime's `require_min_count` helper.

One known limitation of hoisting the types across the crate boundary: the
doc-hidden `ValidatedTree::__new`/`ValidatedRuleNode::__new` constructors used
by generated code are technically callable from any crate, whereas revision-9
modules kept them module-private. They are a contract, not a sealed boundary —
hand-constructing a validated value over an unvalidated parse makes the
infallible validated accessors panic.

Revision 9 moved the grammar-independent support preamble (the typed
`TerminalNode`/`ErrorNode` wrappers, the context child-iteration helpers, and
the embedded-action `__GeneratedInput` facade) into the runtime's `generated`
module the same way. Revisions 12, 13, and 14 are accepted generated-code
contracts; regenerate revision 11 and older lexer and parser modules when
upgrading to this release.

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
antlr-rust-runtime = "0.35.0"

[build-dependencies]
antlr-rust-codegen = "0.35.0"
```

<!-- x-release-please-end -->

Generate only into Cargo's `OUT_DIR` with `antlr_rust_codegen::Builder`, then
call `Generation::emit_rerun_if_changed()` to track the resolved roots,
imports, and token vocabularies. Projects that commit generated modules need
only the runtime dependency and can keep generation in an `xtask` or the CLI.
The earlier package move did not change generated-code API revision 3.

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
