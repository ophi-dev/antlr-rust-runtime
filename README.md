# ANTLR4 Runtime for Rust

[![Crates.io Version](https://img.shields.io/crates/v/antlr-rust-runtime)](https://crates.io/crates/antlr-rust-runtime)
[![ANTLR Runtime Testsuite](https://github.com/ophi-dev/antlr-rust-runtime/actions/workflows/antlr-runtime-testsuite.yml/badge.svg)](https://github.com/ophi-dev/antlr-rust-runtime/actions/workflows/antlr-runtime-testsuite.yml)
[![codecov](https://codecov.io/github/ophi-dev/antlr-rust-runtime/graph/badge.svg?token=QzgT4jB57u)](https://codecov.io/github/ophi-dev/antlr-rust-runtime)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://app.codspeed.io/ophi-dev/antlr-rust-runtime?utm_source=badge)

`antlr-rust-runtime` is a pure Rust runtime for ANTLR v4 lexers and parsers.
The companion `antlr-rust-codegen` package provides the generator library and
`antlr4-rust-gen` command. The implementation is written from scratch from the
public ANTLR runtime contract; it does not vendor or fork an older Rust ANTLR
runtime.

## First Steps

### 1. Get an ANTLR4 grammar

Use your own `.g4` files or a grammar from
[`antlr/grammars-v4`](https://github.com/antlr/grammars-v4). Rust generation
does not require Java, Node.js, the ANTLR tool jar, or an intermediate
`.interp` file. The repository's differential tests use ANTLR `4.13.2` only as
an explicit compatibility oracle.

Grammar sources are read as UTF-8. A leading byte order mark is skipped like
Java ANTLR's `UnicodeBOM` rule, and `\n`, `\r\n`, and lone `\r` line endings are
all accepted, so grammars saved by Windows editors need no conversion. The byte
order mark still occupies a column, which keeps reported positions identical to
ANTLR's.

### 2. Install the Rust ANTLR runtime tools

Each ANTLR target language needs a runtime package used by generated parsers.
For Rust projects, add the runtime crate:

<!-- x-release-please-start-version -->

```toml
[dependencies]
antlr-rust-runtime = "0.35.0"

[build-dependencies]
antlr-rust-codegen = "0.35.0"
```

<!-- x-release-please-end -->

The library crate is imported as `antlr4_runtime`:

```rust
use antlr4_runtime::{CommonTokenStream, InputStream};
```

Install the companion generator binary:

```bash
cargo install antlr-rust-codegen --bin antlr4-rust-gen
```

This installs `antlr4-rust-gen`, which compiles ANTLR `.g4` source into Rust
lexer and parser modules. During generation it also compiles the lexer's DFA
ahead of time and embeds the tables in the generated lexer, so tokenization
runs at full speed from the first character with no per-process warmup.

The former `antlr-rust-runtime` `codegen` feature and generator binary have
moved to `antlr-rust-codegen`. This packaging change does not alter the
generated-source/runtime API revision.

### 3. Generate at build time

Build scripts can generate directly into Cargo's `OUT_DIR`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let generation = antlr_rust_codegen::Builder::new()
        .grammar("grammar/MyGrammarLexer.g4")
        .grammar("grammar/MyGrammarParser.g4")
        .library_directory("grammar")
        .out_dir(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"))
        .generate()?;
    generation.emit_rerun_if_changed();
    Ok(())
}
```

`Generation::inputs()` contains every resolved root, import, and token
vocabulary. `Generation::outputs()` exposes the written artifacts,
`Generation::diagnostics()` exposes structured compiler warnings with exact
source byte spans, and `Generation::warnings()` preserves CLI-form compiler and
generator messages. Projects that commit generated source can omit the build
dependency and use the command instead.

### 4. Generate from the command line

Pass one or more root grammars directly. Imports and `tokenVocab` dependencies
are resolved from each root's directory and any additional `--lib`/`-I`
directories.

For a split lexer/parser grammar:

```bash
antlr4-rust-gen \
  MyGrammarLexer.g4 \
  MyGrammarParser.g4 \
  --lib . \
  --out-dir src/generated
```

Use multiple roots when a build should emit several independent recognizers in
one deterministic source-set compilation.

### Run a grammar with TestRig

TestRig can run directly from a grammar-only project. The project does not need
a `Cargo.toml`, generated Rust sources, or a runtime dependency; it only needs a
Rust toolchain because TestRig invokes Cargo internally. Install the companion
command from the codegen package:

```bash
cargo install antlr-rust-codegen --bin antlr4-rust-testrig
```

From the grammar project, pass a combined grammar, its parser start rule, and
zero or more UTF-8 inputs. With no input files the command reads stdin:

```bash
antlr4-rust-testrig JSON.g4 json --tokens --tree examples/*.json
echo '{"ok":true}' | antlr4-rust-testrig JSON json --tree
```

For a lexer grammar, use the special start rule `tokens`. Pair split grammars
with `--lexer-grammar` and the same import directories used for generation:

```bash
antlr4-rust-testrig MyGrammarParser.g4 start \
  --lexer-grammar MyGrammarLexer.g4 \
  --lib grammar \
  tests/valid/*.txt
```

`--trace`, `--diagnostics`, and `--sll` expose the corresponding parser modes;
exact-ambiguity diagnostics and SLL prediction are mutually exclusive.
Each invocation generates the recognizer and a grammar-specific runner in a
temporary Cargo package, selects the matching `antlr-rust-runtime` release, and
removes the generated package afterward. Cargo build artifacts are reused, so
the first invocation can take longer while dependencies and the runner compile.
The shared target directory lives in the current user's cache directory; set
`ANTLR4_RUST_TESTRIG_TARGET_DIR` to override it.

TestRig processes every named input and returns a non-zero status when
generation, temporary runner compilation, input reading, lexing, or parsing
fails. Recovered lexer and parser syntax errors therefore fail CI even when a
parse tree was produced.

### Generated-source/runtime compatibility

Every newly generated lexer and parser contains a compile-time generated-code
API check. The check records both the API revision and the
`antlr4-rust-gen` package version that produced the module. A mismatch reports
the generated file directly and asks you to either regenerate it with a
compatible generator or select a compatible `antlr-rust-runtime` dependency,
provided the selected runtime implements this check. A runtime released before
the check was introduced instead reports that the generated-code API macro is
missing; upgrade that runtime or regenerate with its matching older generator.

The generated-code API revision tracks the Rust source interface between
generated recognizers and the runtime. It is independent of package SemVer and
the serialized ATN/DFA format versions: compatible package releases may share
one revision, while an incompatible source-contract change increments it.
Using the same package release for `antlr4-rust-gen` and
`antlr-rust-runtime` remains the recommended workflow, but matching the
generated-code API is the compile-time requirement.

The bundled generator currently emits revision 16. Rules with an authored
`catch [...]` or `finally { ... }` clause now expand through two additional
generated-rule lifecycle sections: an `exception` slot that either keeps the
default report-and-recover handler or replaces it with the authored handler
(bound to the recognition error, matching ANTLR's generated catch
replacement), and a `propagate` slot that still runs the authored `finally`
body when a fatal error abandons the generated attempt. Rules without
exception clauses emit the unchanged four-section form. Alongside the
lifecycle change, every authored target-code section (named actions and
exception clauses) now receives a deterministic disposition in the
`semantics.json` `sections` rows, `--require-full-semantics` rejects
unsupported sections, and embedded generation emits `@header` bodies at the
top of the generated module and `@definitions` bodies at module scope.

Revision 15 generated recognizers
embed their static data tables — the ahead-of-time compiled lexer DFA, the
packed parser ATN, and the serialized lexer ATN inside `GrammarMetadata` — as
versioned encoded blobs defined by the runtime's `encoded` module: LEB128
varints (zigzag-mapped for signed values) armored as canonical unpadded base64
string literals. Each blob carries a magic, format version, element kind, and
checked element count, and decoding rejects corrupt, truncated, overflowing,
and unsupported data with targeted diagnostics. The decoded integer streams
are byte-identical to the previous decimal arrays, so recognizer behavior and
the inner serialized ATN/DFA format versions are unchanged, while generated
source shrinks materially and rustc no longer parses hundreds of thousands of
integer expressions per large grammar.

Revision 14 generated parsers embed
packed parser ATN format 3, whose rule-transition tags carry validated
grammar-agnostic tail-call markers. Parser and lexer prediction reuse the
existing caller context when every continuation from a rule call's follow state
is a plain epsilon path to the enclosing rule stop; full-context construction
also omits the same redundant frames. SLL accuracy is preserved by default, and
the reduced-accuracy parser mode is available only through an explicit
simulator constructor.

Revision 12 to 15 generated recognizers remain accepted because the runtime
still provides their source API — the four-section generated-rule form,
integer-array `GrammarMetadata` ATN data,
`CompiledLexerDfa::from_serialized`, and `ParserAtn::from_static` — and reads
packed parser ATN formats 1 through 3. Regenerate them with revision 16 to
execute authored `catch`/`finally` clauses and emit the compact encoded
representation.

Revision 13 moved the iterative generated listener tree-walk engine into
`antlr4_runtime::generated::walk_generated`. Generated parsers retain their
public plain and validated walker surfaces and emit only grammar-specific
listener callback adapters; traversal order, invocation-state threading, and
error propagation remain runtime-owned and shared.
Revision 12 generated recognizers remain accepted because this change does not
remove any runtime API they use; regenerate them to adopt the shared walker
engine and reduce generated source.
Revision 12 replaced every
per-rule `parse_generated_rule_N_dispatch` wrapper and the ordinary
rule-routing match arms with one generated function-pointer table. The
runtime-owned `generated::dispatch_generated_rule` function applies the
depth-cap, parse-listener, stack-growth, and balanced-exit guard around the
selected body; generated code retains explicit ATN-preference and
adaptive-routing exceptions.
Revision 11 stopped re-declaring the parse-driver core and entry-point
scaffolding in every generated parser:
the driver methods (`parse_rule*`, `parse_interpreted_rule*`) expand from the
runtime's `__antlr4_rust_parser_driver!` macro with the module's interpreted
fallback supplied as a binder block, the `parse*` / `parse_stream*` functions
and the `<Grammar>ParserParseOutput` alias of the runtime's
`GeneratedParseOutput` expand from `__antlr4_rust_parser_entry_points!`,
`GeneratedRuleError` is the runtime's grammar-agnostic type (its
`AdaptiveRetry` variant always exists), the adaptive-ATN retry state is the
runtime's `AdaptiveAtnRetryState<N>` sized by a const knob (0 for grammars
without residual adaptive routing), and the lexer `lex` / `lex_stream`
functions are re-exports of runtime generics. Revision 10 stopped re-declaring
the validated-parse surface in every generated parser: `<Grammar>ValidatedTree`
is a type alias of the runtime's `ValidatedTree` branded with the module-local
`ValidatedTreeContext` marker (likewise the module's `ValidatedRuleNode`
alias), `FromValidatedRuleNode` is a re-export of the runtime trait, and
`<Grammar>ValidationError` is an unbranded alias of the runtime's shared
`ValidationError` — so validated trees of different grammars stay distinct
types while all grammars share one error type (per-grammar trait impls on the
error name must collapse into one; see `docs/migration.md`). Revision 9
similarly imports the grammar-independent generated support surface (the typed
terminal/error-node wrappers, context child-iteration helpers, and
embedded-action input facade) from the runtime's `generated` module. Revisions
12, 13, and 14 are accepted generated-code contracts; recognizers from revision
11 or earlier must be regenerated when upgrading to this release.

Generated modules created before this check was introduced cannot be detected
retroactively. When first upgrading to a release that includes the check,
regenerate every committed lexer and parser once. Thereafter, normal
`cargo check` builds enforce compatibility for every compiled generated
module.

## Complete Example

Suppose you are using `JSON.g4` from `antlr/grammars-v4/json`. Generate both
recognizers directly from the combined grammar:

```bash
antlr4-rust-gen \
  JSON.g4 \
  --lib . \
  --out-dir src/generated
```

Declare the generated modules in your crate:

```rust
mod generated {
    #![allow(dead_code)]

    pub mod json_lexer;
    pub mod json_parser;
}
```

### Typed listeners and visitors

Parser generation emits a typed `<Grammar>Listener` and
`<Grammar>TreeWalker` by default. A listener can start a grammar-typed walk
directly. Listener callbacks return `Result<(), E>`, where `E` defaults to
`Infallible`, so domain errors can stop traversal without a side channel:

```rust
listener.walk(parsed.tree())?;
```

Use `--no-listener` to omit that surface. Add `--visitor` to emit a typed
`<Grammar>Visitor`; visitors choose an associated `Result` type, define its
initial value with `default_result()`, and drive recursion explicitly:

```rust
type Result = Result<i64, MissingChildError>;

fn default_result(&mut self) -> Self::Result {
    Ok(0)
}

fn visit_add_label(&mut self, ctx: &AddLabelContext) -> Self::Result {
    let left = self.visit(ctx.left()?)?;
    let right = self.visit(ctx.right()?)?;
    Ok(left + right)
}
```

Generated child accessors follow grammar cardinality. Required children return
`Result<T, MissingChildError>`, optional children return `Option<T>`, and
repeated children are lazy iterators. Rule labels keep their grammar names
(`left()`), while token accessors use snake_case names such as `int_token()` and
`comma_tokens()`. Every typed context also exposes `direct_terminals()`, which
iterates only terminals owned directly by that context. It is the stable
fallback for anonymous literal tokens and does not descend into nested rules.
On recovered trees, the iterator includes error nodes such as synthetic
`<missing ...>` tokens through the same `TerminalNode` surface.
`TerminalNode::is_error()` reports both inserted and deleted recovery nodes,
while `TerminalNode::is_missing()` identifies inserted synthetic tokens.
Recovery callbacks expose the same discriminator as `ErrorNode::is_missing()`.
`direct_terminals` is a reserved accessor name, so grammar rules or labels
that normalize to it use collision fallbacks such as
`direct_terminals_rule_child()` or `direct_terminals_label()`.

Call `parse_validated` when the application rejects recovered parses. It checks
lexer and parser syntax-error counts, recovered error nodes, and every generated
required-child invariant before returning a `<Grammar>ValidatedTree`:

```rust
use generated::json_lexer::JsonLexer;
use generated::json_parser::{
    self, JsonContext, JsonParser, ValidatedTreeContext,
};

fn main() -> Result<(), json_parser::JsonValidationError> {
    let validated = json_parser::parse_validated(
        r#"{"a":1}"#,
        JsonLexer::new,
        JsonParser::json,
    )?;
    let json = validated
        .tree()
        .downcast_ref::<JsonContext<'_, ValidatedTreeContext>>()
        .expect("the selected entry rule is json");

    // `value` and `EOF` are required by `json : value EOF`.
    println!("{}{}", json.value().text(), json.eof_token());
    Ok(())
}
```

Required accessors on contexts carrying `ValidatedTreeContext` return the child
directly. Optional children remain `Option<T>`, and repeated children remain
iterators. `parse_with_parser(...).validate()` provides the same boundary when
the caller needs the parser first. Generated `<Grammar>ValidatedListener` and,
with `--visitor`, `<Grammar>ValidatedVisitor` traits traverse this surface
without `MissingChildError` plumbing. The original listener, visitor, and
`ParsedFile` context APIs remain recovery-oriented and fallible.

`--no-visitor` disables visitor generation.

Call the generated parser helper for the compact path:

```rust
use generated::json_lexer::JsonLexer;
use generated::json_parser::{self, JsonParser};

fn main() -> Result<(), antlr4_runtime::AntlrError> {
    let parsed =
        json_parser::parse(r#"{"a":1}"#, JsonLexer::new, JsonParser::json)?;

    println!("{}", parsed.tree().text());
    Ok(())
}
```

Use `parse_with_parser` when you want the compact setup path and also need the
parser afterward for diagnostics:

```rust
use antlr4_runtime::Parser;
use generated::json_lexer::JsonLexer;
use generated::json_parser::{self, JsonParser};

fn main() -> Result<(), antlr4_runtime::AntlrError> {
    let output = json_parser::parse_with_parser(
        r#"{"a":1}"#,
        JsonLexer::new,
        JsonParser::json,
    )?;
    let syntax_errors = output.parser.number_of_syntax_errors();
    let json_parser::JsonParserParseOutput {
        result: tree,
        parser,
    } = output;
    let parsed = parser.into_parsed_file(tree);

    println!(
        "{} errors across {} tokens",
        syntax_errors,
        parsed.tokens().len()
    );
    println!("{}", parsed.tree().text());
    Ok(())
}
```

When the parser needs typed semantic hooks, use
`parse_with_parser_constructor`. The parser constructor receives the buffered
`CommonTokenStream`, so superclass-style hooks and any other constructor-time
configuration stay inside the same generated driver:

```rust
use antlr4_runtime::Parser;
use generated::java_script_lexer::JavaScriptLexer;
use generated::java_script_parser::{self, JavaScriptParser};
use javascript_lexer_base::JavaScriptLexerBase;
use javascript_parser_base::JavaScriptParserBase;

fn main() -> Result<(), antlr4_runtime::AntlrError> {
    let output = java_script_parser::parse_with_parser_constructor(
        "class Example { static value = /x+/; }",
        |input| {
            JavaScriptLexer::with_typed_hooks(
                input,
                JavaScriptLexerBase::with_strict_default(false),
            )
        },
        |tokens| JavaScriptParser::with_typed_hooks(tokens, JavaScriptParserBase),
        JavaScriptParser::program,
    )?;
    assert_eq!(output.parser.number_of_syntax_errors(), 0);
    let parsed = output.into_parsed_file();

    println!("{}", parsed.tree().text());
    Ok(())
}
```

See the [JavaScript build guide](docs/javascript-build.md) for a complete
typed-lexer and typed-parser base setup.

Call `.validate()` instead of `.into_parsed_file()` when the application wants
the generated validated-tree boundary. The lexer constructor can likewise be a
closure that calls `Lexer::with_typed_hooks`. Use
`parse_stream_with_parser_constructor` for a caller-provided `CharStream`.

Use `parse_stream` with a preconstructed `CharStream` when parsing a file,
preserving its source name, or supplying a custom stream implementation:

```rust
use std::fs::File;
use antlr4_runtime::InputStream;
use generated::json_lexer::JsonLexer;
use generated::json_parser::{self, JsonParser};

let input = InputStream::from_reader_with_source_name(
    File::open(path)?,
    path.display().to_string(),
)?;
let parsed = json_parser::parse_stream(input, JsonLexer::new, JsonParser::json)?;
```

`InputStream::from_reader` accepts any `std::io::Read` and validates UTF-8.
`parse_stream_with_parser` is the corresponding stream-based helper when the
caller also needs the parser afterward.

Construct each layer explicitly when you need parser options or custom error
handling before invoking the entry rule:

```rust
use antlr4_runtime::{CommonTokenStream, InputStream};
use generated::json_lexer::JsonLexer;
use generated::json_parser::JsonParser;

fn main() -> Result<(), antlr4_runtime::AntlrError> {
    let mut lexer = JsonLexer::new(InputStream::new(r#"{"a":1}"#));
    lexer.remove_error_listeners();
    let tokens = CommonTokenStream::new(lexer);
    let mut parser = JsonParser::new(tokens);
    parser.remove_error_listeners();
    let tree = parser.json()?;

    println!("{}", parser.node(tree).text());
    Ok(())
}
```

Generated recognizers install a `ConsoleErrorListener` by default. Remove it
from both the lexer and parser to suppress recovery output, as above, or call
`add_error_listener` after removal to redirect diagnostics to a replacement.
`ErrorListener::syntax_error` receives a `SyntaxErrorEvent`; its `span` is the
resolved half-open UTF-8 byte range for parser tokens and lexer failures, when
the input stream can provide byte offsets.

### Inspecting tokens

Generated lexer modules expose `lex` for token-only workflows. The returned
stream is already filled and retains EOF plus tokens on hidden or custom
channels:

```rust
use antlr4_runtime::Token as _;
use generated::json_lexer::{self, JsonLexer};

let tokens = json_lexer::lex(r#"{"a":1}"#, JsonLexer::new);
let vocabulary = json_lexer::metadata().vocabulary();
for token in tokens.tokens() {
    println!(
        "type={} channel={} text={:?}",
        vocabulary.display_name(token.token_type()),
        token.channel(),
        token.text(),
    );
}
```

The vocabulary resolves symbolic or literal token names, and the explicit
channel value includes the default channel. Rules using `skip` do not emit
tokens. `number_of_source_errors()` reports buffered lexer diagnostics; after
iterating `tokens()`, call `drain_source_errors()` to retrieve them.

Use `lex_stream` to supply a named `InputStream`, `ByteStream`, or custom
`CharStream`. Like `CommonTokenStream::new`, both helpers panic if token
buffering returns `TokenStoreError`; construct the lexer and use
`CommonTokenStream::try_new` when that error must be handled.

### Reusing Recognizers

Generated recognizers can be re-fed without reconstructing the lexer or parser.
The token stream owns its lexer, so use the mutable accessors to update that
nested source and rebuild the token buffer:

```rust
let lexer = JsonLexer::new(InputStream::new(""));
let tokens = CommonTokenStream::new(lexer);
let mut parser = JsonParser::new(tokens);

for input in [r#"{"a":1}"#, r#"{"b":2}"#] {
    let tokens = parser.token_stream_mut();
    tokens
        .token_source_mut()
        .set_input_stream(InputStream::new(input));
    tokens.refill();
    parser.reset();

    let tree = parser.json()?;
    println!("{}", parser.node(tree).text());
}
```

`CommonTokenStream::set_token_source` and generated
`Parser::set_token_stream` replace whole layers when ownership is already
available. `clear_dfa()` on generated lexers and parsers drops learned fallback
and decision DFA state for cold measurements or memory control; immutable
ahead-of-time lexer DFA tables remain embedded generated data.

### Choosing Parser Entry Rules

Generated parsers expose one public method per grammar rule. Call the method
that matches the grammar's intended top-level rule for the input. The generated
parser rustdoc lists likely entry methods first, followed by all rule methods,
using the same call-graph and `EOF` inference as the `G4S078` unreachable-rule
diagnostic. Use `--entry-rule` to declare callable non-`EOF` forms before
enabling `--prune-unreachable`; the generator cannot infer the semantic choice
between multiple public rule methods.

For the JSON grammar above, `json()` is the natural entry. Larger grammars may
have several top-level forms, so confirm the intended entry rule against that
grammar's documentation. Calling the wrong rule can still recover and return a
parse tree with error nodes, so check parser diagnostics when adding a new input
form.

### Parse-Tree Pattern Matching

Generated parsers expose `compile_parse_tree_pattern`, the analog of ANTLR's
`Parser.compileParseTreePattern`. A *tree pattern* is grammar input with `<tag>`
placeholders: literals must match exactly, `<expr>` matches any `expr` subtree,
`<ID>` matches any `ID` token, and `<lhs:expr>` binds the match to a label.

```rust
let pattern = parser.compile_parse_tree_pattern(
    "<ID> = <expr>;",
    RULE_STAT,
    MyGrammarLexer::new, // lexes the pattern's literal chunks
)?;

let m = pattern.match_tree(subtree);
if m.succeeded() {
    println!("assigns to {}", m.get("ID").unwrap().text());
}
```

Compilation interprets the pattern over a rule-bypass ATN
(`ParserAtn::with_bypass_alternatives`), the same mechanism the reference
runtimes use; matching walks the subject and pattern trees in lockstep. The
generated method caches the compiler per process, so compiling many patterns is
cheap; to change the `<`/`>`/`\` delimiters, use `ParseTreePatternMatcher`
directly.

## Technical Notes

- Pure Rust runtime implementation.
- Written from scratch as a clean-room implementation.
- Compiles ANTLR grammar source into lexer ATNs and packed parser runtime
  tables without an external generator.
- Supports lexer and parser execution through generated Rust wrappers.
- Supports real split lexer/parser grammars, including Kotlin smoke builds.
- Passes every upstream ANTLR runtime-testsuite descriptor discovered by the
  harness: `357 passed, 0 failed, 0 skipped, 357 run`.
- Licensed under BSD-3-Clause for compatibility with ANTLR's runtime licensing
  pattern and downstream open-source applications.

The runtime contains:

- `IntStream` and `CharStream`
- UTF-8 input as Unicode scalar values
- compact `TokenId`/`TokenView` access, `TokenSource`, and one canonical
  `TokenStore`
- buffered, channel-aware `CommonTokenStream`
- `Vocabulary`
- recognizer metadata and error listener plumbing
- parse tree node types, rule contexts, terminal nodes, error nodes, and walkers
- parse-tree XPath queries on par with the official ANTLR runtimes; XPath's
  checked-in lexer intentionally remains in the runtime because XPath is a
  runtime API, and extracting only its generated recognizer would introduce a
  runtime dependency cycle
- parse-tree pattern matching (`compileParseTreePattern` / `ParseTreePattern` /
  `ParseTreeMatch`) with rule/token tags, labels, and rule-bypass ATNs
- ANTLR v4 serialized lexer ATN deserialization
- lexer ATN recognition with longest-match/rule-priority behavior and lexer
  actions
- ahead-of-time compiled lexer DFA tables, built by `antlr4-rust-gen` and
  embedded in generated lexers, with per-token escape to ATN interpretation
  for constructs a finite DFA cannot represent (semantic predicates,
  recursive lexer rules)
- versioned, packed parser ATN tables embedded directly in generated parsers,
  with rule recognition over borrowing state/transition views
- canonical `ContextId` prediction graphs pooled with learned parser DFA state

The workspace also contains `antlr4-rust-gen`, the source-only generator in
`antlr-rust-codegen`, and the unpublished `antlr4-runtime-testsuite` conformance
harness.

See [docs/kotlin-build.md](docs/kotlin-build.md) for the Kotlin smoke workflow.
See [docs/runtime-testsuite.md](docs/runtime-testsuite.md) for the upstream
runtime-testsuite harness.

### Semantic Predicates and Actions: the Compatibility Boundary

ANTLR grammars may embed **target-language** semantic predicates and actions
(`{isTypeName()}?`, `{this.count++;}`). The direct compiler preserves their
structural owner, source span, and finalized ATN coordinate, but cannot make
arbitrary code written for another target language executable as Rust. The
boundary is:

- **Target-agnostic grammars** — no embedded code, or only built-in lexer
  commands (`skip`, `channel(...)`, `mode(...)`, `type(...)`) — are fully
  supported.
- **Recognized predicate/action shapes** — a library of common idioms
  (constant predicates, lookahead text/type checks, integer member counters,
  column predicates, and the upstream testsuite's action templates) — are
  translated into SemIR by `antlr4-rust-gen`.
- **User pattern files** — `--sem-patterns file.toml` can add exact predicate
  rewrites, helper-call rewrites, grammar-declared member state (see
  [Inline `@members` State](#inline-members-state-scalars-and-stacks) below),
  and per-coordinate `hook` / `assume-true` / `assume-false` / `error`
  dispositions without changing the generator.
- **Everything else is not silently guessed.** Each generator run writes a
  `semantics.json` manifest next to the generated modules listing every
  predicate/action coordinate with its grammar source span, body, and
  disposition (`translated`, `hooked`, `assume-true`, `assume-false`,
  `ignored`, `synthetic`, or `error`). A `synthetic` action is one ANTLR
  inserts itself (e.g. during left-recursion elimination); it has no
  grammar-author source, is a runtime no-op, and is exempt from the `error`
  gate — only actions the author actually wrote in the grammar can fail it.

Semantic pattern files are parsed by the checked-in TOML recognizer in
`antlr-rust-toml-parser`, generated by this project's own generator from the
pinned grammars-v4 TOML grammar. The generated recognizer provides the strict
validated-tree boundary and typed listener; codegen separately validates the
`[[pattern]]`, `[[helper]]`, `[[coordinate]]`, and `[[member]]` schema. Invalid
TOML, duplicate or unknown fields, incorrect value types, and unsupported
sections fail generation instead of being ignored.

This is a migration from the former permissive scalar reader: pattern files
must now be valid TOML, string-valued fields must be quoted, and unknown fields
are errors. The internal TOML facade exposes flat syntax/value items; codegen's
semantic-pattern schema remains responsible for duplicate and table semantics,
so the implementation crate is not a standalone semantic TOML API.

The same manifest inventories top-level grammar options. Options implemented
by the source compiler (`tokenVocab` and `caseInsensitive`) are recorded
without a warning. Target extension options such as `superClass` and
`contextSuperClass` warn because the Rust backend cannot inherit their
target-language implementation automatically. If caller-owned Rust hooks
provide that behavior, acknowledge the exact option:

```bash
antlr4-rust-gen L.g4 \
    --option-hook superClass=MyLexerBase --out-dir src/generated
```

Acknowledged options have the `hooked` disposition. Unacknowledged target
options have the `unsupported` disposition and make
`--require-full-semantics` fail.

The manifest also inventories every source-owned target-code *section* under
each grammar's `sections` array: grammar-level named actions (`@header`,
`@definitions`, `@members`, scoped variants), rule-level named actions
(`@init`, `@after`), and rule exception clauses (`catch [...] { ... }`,
`finally { ... }`). Sections have no ATN coordinate, so these rows are what
makes them auditable. Each row carries the section kind, scope, owning rule,
source position, body, and a disposition: `embedded` (the body is spliced
into generated Rust), `translated` (the section's behavior lowers into
generated metadata without splicing the body, e.g. `@members` state owned by
`[[member]]` declarations in `--sem-patterns`), or `unsupported`. Declaring
`[[member]]` slots for a recognizer is the caller's explicit acknowledgment
that the pattern file owns that recognizer's `@members` state: the
target-language body is replaced wholesale, not parsed or partially matched,
exactly like `--option-hook` acknowledges an option's target behavior.
Unsupported sections warn on every run and fail generation under
`--require-full-semantics` with a source-positioned diagnostic.

Under `--actions embedded`, supported sections execute:

- `@header` bodies are emitted once at the top of the generated module,
  before the generated imports; `@definitions` bodies are emitted once at
  module scope after the `@members` module items. Both are translated with
  the same token-alias machinery as `@members`. In a combined grammar the
  unscoped sections belong to the parser module; `@lexer::`-scoped sections
  are currently unsupported.
- A single `catch [name] { ... }` clause per rule replaces the default
  report-and-recover handler, with the recognition error bound to `name`
  (a plain or raw Rust identifier). Typed clauses such as
  `catch [RecognitionException e]` are rejected: the handler receives every
  recognition error, and this backend models no target-language exception
  types. The rule then completes normally, like ANTLR's generated catch
  replacement. Multiple catch clauses are unsupported.
- `finally { ... }` runs exactly once on every completed path — after
  `@after` on success, after default recovery or an authored catch handler,
  and before a fatal propagated error abandons the rule — and never during
  speculative adaptive-retry unwinds, whose re-entry runs the rule from the
  top. `@after` remains success-only.

Unknown coordinates are governed by `--sem-unknown`:

```bash
antlr4-rust-gen L.g4 P.g4 --lib . \
    --out-dir src/generated --sem-unknown error
```

- `assume-true` (current default, deprecated): unknown predicates pass,
  unknown actions are no-ops — the historical behavior. A future minor
  release changes the default to `error`.
- `hook`: unknown parser predicates are routed to `SemanticHooks` and fail if
  the hook does not handle them.
- `assume-false`: unknown predicates fail, removing the guarded alternatives.
- `error`: generation fails, naming each coordinate:

  ```text
  unsupported semantic predicate: rule=s(0) pred_index=0 at 2:4: {isTypeName()}
  ```

At runtime the same policy exists as
`ParserRuntimeOptions::unknown_predicate_policy`
(`UnknownSemanticPolicy::{AssumeTrue, AssumeFalse, Error}`); under `Error`,
evaluating an unknown predicate coordinate fails the parse with
`AntlrError::Unsupported` instead of producing a tree whose shape silently
depended on a guess.

Generated parsers also expose a parser-side hook escape hatch:
`MyParser::with_hooks(tokens, hooks)`, where `hooks` implements
`SemanticHooks`. Unknown parser predicates are offered to
`SemanticHooks::sempred` before the fallback policy is applied, and unhandled
parser action events are offered to `SemanticHooks::action` at their grammar
position after the containing path is committed. Predicate hooks may run
speculatively during prediction, so they must be replay-safe; action hooks never
run on speculative or losing paths.

For helper-call predicates and actions written as `helper()`, `this.helper()`,
or `self.helper()`, generated parsers also emit a typed hook adapter
(`MyParserHooks` plus `MyParserTypedHooks<T>`) that maps stable manifest
coordinates to named Rust methods. A `[[helper]]` pattern can opt into one
additional receiver spelling with `receiver = "..."`. For example, an
antlr4rust grammar using its `recog.helper()` convention can retain that source
spelling during migration:

```toml
[[helper]]
kind = "parser-predicate"
name = "helper"
receiver = "recog"
returns = "bool"
lower = "hook"
```

Use `kind = "parser-action"` and `returns = "unit"` for an action helper. Its
typed method runs exactly once on the committed path, before any later
predicate or nested-rule event.

Lexer callers can use `LexerSemCtx` with
`atn::lexer::next_token_with_semantic_hooks` or the
compiled-DFA variant to route lexer predicates/actions through the same
`SemanticHooks` trait. On the committed action path, `LexerSemCtx` exposes the
pending token type/channel, character lookahead and consumption, and mode
mutators. Actions can also queue a prefix token and advance the current token
start, allowing one lexer match to return multiple tokens while each
`TokenSource::next_token` call still appends exactly one token.

Lexer behavior that has no ATN action/predicate coordinate uses
`LexerLifecycleCtx`. A hook may implement `lexer_before_token`,
`lexer_after_accept`, `lexer_reset`, and the existing
`lexer_token_emitted` observer. The post-accept callback runs after portable
and custom actions but before the token span is emitted, including for rules
with no semantic transition. Generated lexers expose `reset()` to clear
runtime-owned pending tokens and invoke extension-owned cleanup. Lexers built
with `new()` retain the direct compiled-DFA path; `with_hooks()` opts into the
lifecycle dispatch path.

Generated lexers also own optional hook state and emit typed lexer adapters
when a semantic pattern maps lexer helper calls to hooks. The official
grammars-v4 JavaScript and TypeScript grammars are complete examples, including
checked-in Rust lexer/parser base modules and strict build commands; see
[`docs/javascript-build.md`](docs/javascript-build.md) and
[`docs/typescript-build.md`](docs/typescript-build.md).

Use `--require-full-semantics` in CI when every coordinate and target extension
option must be either translated, metadata-backed, or explicitly hooked;
policy fallbacks and unsupported options fail generation.

#### Inline `@members` State: Scalars and Stacks

Many published grammars keep their lexer state **inline** in `@lexer::members`
rather than externalizing it into a `superClass` — a nesting counter plus a
stack or two, mutated from inline actions and read from inline predicates. The
C# interpolated-string lexers are the motivating case, but the shape is common
(JavaScript/TypeScript template literals, any grammar with a mode stack).

SemIR models this state directly, so such a grammar needs **no hand-written
hooks**. A pattern file declares the slot inventory and maps each inline body:

```toml
[[member]]
name = "interpolatedStringLevel"
kind = "int"                       # `bool` is an alias: slots store integers

[[member]]
name = "verbatium"
kind = "bool"
init = true                        # the grammar's `bool verbatium = true;`

[[member]]
name = "interpolatedVerbatiums"
kind = "stack"

[[pattern]]
match = "interpolatedStringLevel++; interpolatedVerbatiums.Push(true); verbatium = true;"
lower = "seq(add_member(interpolatedStringLevel, int(1)), push_member(interpolatedVerbatiums, bool(true)), set_member(verbatium, bool(true)))"

[[pattern]]
match = "!verbatium"
lower = "not(member(verbatium))"
```

Declaring the slots keeps codegen out of the business of parsing host-language
fragments: the generator still only matches whole declared bodies, and the
*mapping* is user-owned data. Declaration order fixes slot numbering, and
scalar and stack slots are numbered in separate namespaces. An unknown or
mistyped slot name is a codegen error naming the slot, never a silently
mis-numbered one.

`init` carries a scalar slot's **declared initial value**. Slots otherwise start
at 0, so a grammar writing `private bool enabled = true;` and guarding a rule
with `{ enabled }?` would reject input it should accept — while the manifest
still reported the coordinate as `translated`. Declaring `init` seeds the slot
at construction (on the lexer *and* the parser) and restores it on every reset.
It is metadata rather than something parsed out of the host-language
declaration; stacks cannot take one (their initial contents are not a single
scalar) and are rejected if they try.

`scope = "lexer" | "parser" | "both"` (default `both`) selects which
recognizer's inventory a declaration joins. A combined grammar may legally
declare independent `@lexer::members` and `@parser::members`, including
same-named ones with different kinds and initializers; each recognizer numbers
its own slots and holds its own member state at runtime, so the two never
collide. Duplicates are still rejected *within* one recognizer's inventory.

Two `[[pattern]]` entries matching the same body is an error naming both IDs,
for predicates and actions alike — otherwise reordering the pattern file would
silently change runtime behavior.

The `lower` DSL is a constructor syntax for the IR, not an expression language:
`member(N)`, `member_top(N)`, `member_len(N)`, `int(N)`, `bool(b)`, `not(e)`,
`set_member`, `add_member`, `push_member`, `pop_member`, and `seq(...)` for the
compound bodies real grammars write. (A bare `lower = "bool(false)"` remains the
constant-false predicate template it has always been; literals mean member
state only inside a member expression or statement.)

**Empty-stack semantics are defined, not errors** — grammars rely on this:

| Operation | On an empty or never-pushed stack |
|---|---|
| `member_top(s)` | `Null`, which is **falsy** — exactly the `Count > 0 ? Peek() : false` idiom, so the guard needs no explicit depth check |
| `member_len(s)` | `0`, not Null: a never-used stack is empty, not absent |
| `pop_member(s)` | a no-op — an unbalanced pop is a grammar bug, and panicking inside speculative prediction would turn it into a crash |

Lexer member state is cleared by `reset()` and by replacing the input stream, so
a reused lexer never carries interpolation depth into the next input. Parser
member state is path-local and participates in the memo key, so speculative
paths never observe each other's mutations.

A worked example — the C# interpolated-string lexer, generated with
`--sem-unknown error --require-full-semantics` (i.e. zero hooks and zero policy
fallbacks) and validated token-for-token against an ANTLR 4.13.2 **Java** lexer
built from the same grammar — lives in
[`crates/antlr-rust-codegen/tests/fixtures/antlr4-rust-gen/stack-member-lexer/`](crates/antlr-rust-codegen/tests/fixtures/antlr4-rust-gen/stack-member-lexer/)
with its integration test (`inline_lexer_member_stacks_generate_without_hooks`
in [parser.rs](crates/antlr-rust-codegen/tests/antlr4_rust_gen_cli/parser.rs)).

#### Embedded target-language actions are not portable — including in official ANTLR

A grammar that embeds a **target-language** action (a `{ ... }` block of
Java/C#/etc. code, rather than a portable lexer command) is only usable with
the language it was written for. This is a limitation of ANTLR itself, not of
this runtime: **the official ANTLR tool does not translate embedded actions
between targets — it copies the source text verbatim into the generated code.**

For example, the official
[Kotlin/kotlin-spec](https://github.com/Kotlin/kotlin-spec) `KotlinLexer.g4`
contains a Java-only action:

```antlr
RCURL: '}' { if (!_modeStack.isEmpty()) { popMode(); } };
```

Generating a **Go** parser from it with the official tool
(`antlr4 -Dlanguage=Go KotlinLexer.g4`) emits the Java verbatim:

```go
func (l *KotlinLexer) RCURL_Action(localctx antlr.RuleContext, actionIndex int) {
	switch actionIndex {
	case 0:
		if !_modeStack.isEmpty() { // undefined in Go — does not compile
			popMode()              // undefined in Go — does not compile
		}
	}
}
```

The generated Go **fails to compile** (`undefined: _modeStack`, `undefined:
popMode`), and ANTLR offers no supported way to fix it beyond hand-editing the
grammar — the grammar even carries a comment telling non-Java users to replace
the snippet manually. Every non-Java ANTLR target has this gap.

This runtime does better in two ways:

1. It recognizes a **library of common embedded idioms** (e.g. the guarded
   `popMode()` above) and maps them to the equivalent portable operation, so
   many real grammars generate as-is.
2. For anything it does not recognize, `--sem-unknown=error` fails **loudly**
   at generation time, naming the coordinate, instead of silently emitting
   uncompilable or no-op code. The fix is to express the action as a portable
   lexer command (`-> popMode`, `-> pushMode(X)`, `-> type(X)`,
   `-> channel(HIDDEN)`), add a `--sem-patterns` rewrite, or route it through a
   `SemanticHooks` implementation.

Portable lexer commands and the recognized idioms are the target-agnostic
subset; prefer them when authoring grammars intended for multiple runtimes.

Grammars whose `{ ... }` blocks are already **Rust** can skip translation
entirely: `antlr4-rust-gen Foo.g4 --actions embedded` splices the
bodies verbatim (after `$`-attribute translation) into the generated parser,
inline at their ATN action/predicate coordinates. This is the mode the
conformance harness uses after rendering descriptor grammars through
`Rust.test.stg` (see below).

In embedded mode, bodies produced by the antlr4rust transforms in
`antlr/grammars-v4` need no additional flags. The generator lowers their
observed `recog.input.la(...)` / `lt(...)` token access, token-view getters,
generated parser token aliases, and `_localctx` child accessors onto native
runtime APIs. Unsupported `recog` or `_localctx` shapes fail at generation
with the owning grammar coordinate instead of becoming generated-crate errors;
those parser-only receivers are rejected in lexer bodies as well. On context
types reached by compatibility `_localctx` access, a legacy child getter takes
its source spelling and a colliding native helper is exposed as `context_*`.
Labeled alternatives currently materialize `_localctx` through the base rule
context, so alternative-only children remain outside this compatibility surface.

The CLI also discovers the current grammars-v4 C and Java
`Rust/transformGrammar.py` files next to grammar roots. It fingerprints the
`Rust/` folder and requires explicit trust before running the transform with
bundled RustPython in a child over a disposable grammar-tree copy. Interactive
choices can trust once, the exact revision, or the repository; non-interactive
invocations fail with the exact `--trust-rust-support sha256:...` argument.
This trusted-source path does not claim to sandbox the host. It stages inputs
to preserve the checkout and automatically emits transformed grammars and
shipped Rust support modules under `antlr-rust-support/`, plus a
`rust-support.json` audit manifest.

Python compatibility is intentionally pinned to the imports and filesystem
operations used by those two transforms. Other or future sibling scripts must
be evaluated explicitly before they are considered supported.

### Decision Tiers and `--fixed-lookahead`

ANTLR always generates an adaptive `ALL(*)` recognizer: every decision point
carries prediction machinery and a learned DFA cache, even when one token of
lookahead already settles it. Like the Java tool, `antlr4-rust-gen`
classifies every parser decision and ports Java's tiering: decisions whose
alternatives' LOOK(1) sets are pairwise disjoint compile to plain token
switches, and only the rest run adaptive prediction.

Every generation writes a **`decisions.json`** manifest next to
`semantics.json` reporting the tier of each decision — `ll1`, `fixed`
(see below), or `adaptive` with the reason it needs the simulator
(`non-greedy`, `precedence`, `predicate`, `empty-look`, `not-disjoint`,
`budget-exceeded`, `sync-bound`). Manifest version 2 also reports
`canDefer`: complete LL(1) dispatches are `false`, while partial fixed
tables and adaptive decisions are `true`, so the report agrees with whether
the emitted path can reach adaptive prediction:

```json
{"decision": 4, "rule": "namespace_", "state": 113, "canDefer": true, "tier": "fixed", "lookahead": 2}
```

The opt-in **`--fixed-lookahead <k>`** flag (off by default) goes one tier
further than Java: decisions that are not LL(1) but whose lookahead
languages are pairwise disjoint within `k` tokens compile into a static
nested `match` over `la(1) .. la(k)` — no simulator, no DFA warming, no
per-decision cache on a table hit. In plain (non-embedded) mode the flag also
compiles the Java-parity LL(1) switches statically. Behavior is preserved by
construction: dispatch arms are restricted to lookahead for which the
decision's recovery sync is a provable no-op. A complete LL(1) miss runs
sync and reuses the proven-total LL(1) dispatch without constructing a
simulator; fixed-LL(k) and adaptive decisions can defer through their regular
sync + adaptive body. The classification is deterministic and idempotent;
predicate-guarded, non-greedy, and precedence decisions always stay
adaptive.

`grammars-v4` examples: Thrift classifies 53 of 54 decisions LL(1) and the
last (`namespace_`) fixed-LL(2) — with `--fixed-lookahead 2` the whole
parser runs prediction-free. Rego adds two fixed-LL(2) tables (`regoElse`,
and `object_`'s trailing-comma list loop) over 27 LL(1) decisions, with the
10 genuinely ambiguous decisions staying adaptive.

### Unreachable parser rules

The grammar frontend reports `warning[G4S078]` for parser rules that no entry
rule can reach. Source call-graph components whose paths reach an explicit
`EOF` terminal are inferred as entries. Without an explicit entry selection for
a parser, rules that no other rule calls are also inferred; this preserves
callable top-level forms that do not consume `EOF`. The first parser rule is the
fallback only when neither inference finds an entry.

Repeated `--entry-rule NAME` options declare non-`EOF` entry rules explicitly.
Names are bare and apply to every generated parser in the invocation that
defines `NAME`. Once at least one name matches a parser, its other non-`EOF`
top-level rules are no longer inferred, while `EOF` entries remain automatic.
`G4S079` rejects a name not defined by any generated parser. Lexer rules,
including fragments and mode-scoped rules, and grammars loaded only as
`tokenVocab` sources are outside this parser analysis.

Emitting `G4S078` does not remove code. The opt-in `--prune-unreachable` pass
removes the same unreachable set before ATN construction and logs every
removed qualified rule:

```bash
antlr4-rust-gen Grammar.g4 \
  --entry-rule alternateTopLevel \
  --prune-unreachable \
  --out-dir src/generated
```

Pruning is recognition preserving only from the inferred and configured entry
rules. It removes generated rule methods, context types, and listener/visitor
callbacks, so consumers that invoke other rules directly must declare them with
`--entry-rule` or leave pruning disabled.

### Trivial-rule inlining

`--inline-trivial-rules` is an explicit, off-by-default optimization that
inlines two classes of pure parser rules into their call sites before ATN
construction, so the caller's decision sees the actual tokens instead of a
rule transition:

- **token-set rules** — a rule whose body is nothing but an alternation of
  single terminals (keyword lists, operator names). Every reference is
  replaced by the flattened token set, however many call sites exist;
  expansion is bounded by construction at one element per site.
- **single-use pure sequences** — a single-alternative rule referenced
  exactly once whose body carries no observable surface. Its body moves into
  the call site as a parenthesized block.

Inlined rules are removed, so this pass is **recognition preserving**, not
tree/API preserving: the callee's rule method, context type, and
listener/visitor callbacks disappear, its parse-tree level vanishes, and its
recovery boundary moves to the caller. The accepted language and consumed
input for valid text are unchanged.

Candidates are inlined all-or-nothing and fail closed. A candidate is
declined — with a reason recorded in the manifest — when it is a configured or
inferred entry rule, recursive, nullable, referenced by grammar target code,
carries labels, attributes, actions, predicates, options, or exception
clauses, or when any call site binds a label, passes arguments, or pins
precedence. Discovery re-runs after every accepted rewrite, so alias chains
(`a : b ; b : X | Y ;`) collapse in one invocation while every application
removes exactly one rule.

Every applied run writes `optimizations.json` recording each candidate's
status, reason, removed rule, and rewritten call sites with original source
spans. Inspect the same deterministic report without generating or changing a
parser with:

```bash
antlr4-rust-gen Grammar.g4 \
  --report-trivial-rules \
  --out-dir target/grammar-report
```

Report mode writes only `optimizations.json`. Apply reviewed candidates with:

```bash
antlr4-rust-gen Grammar.g4 \
  --inline-trivial-rules \
  --out-dir src/generated
```

### Precedence-ladder optimization

`--optimize-precedence-ladders` is an explicit, off-by-default source
optimization for grammars that express operator precedence as a linear chain
of parser rules. It recognizes pure delegation, binary star loops, direct
left recursion, right-associative optional tails, and prefix-unary levels,
then combines a proven chain into one left-recursive rule before ATN
construction.

This pass is **recognition preserving**, not tree/API preserving. It removes
intermediate rule methods and context/listener/visitor types, changes tree
depth, and turns flat star-loop children into nested left-recursive operator
contexts. Existing consumers keyed to those surfaces should not enable it.
Removed rules are also removed recovery boundaries, so malformed-input
diagnostic counts, error-listener calls, recovered trees, and stopping positions
are not preserved. The guarantee is that complete inputs parse without syntax
errors under the optimized grammar exactly when they do under the authored
grammar; consumers requiring recovery parity should not enable this pass.
Configured `--entry-rule` rules and externally referenced middle rules are
retained as collapse boundaries. Actions, predicates, rule attributes,
unsupported shapes, and overlapping same-fixity operator sets are declined. A
prefix level that is looser than another collapsed operator level is also
declined: ANTLR precedence parameters constrain recursive operators, not entry
into primary/prefix alternatives, so collapsing that order would admit the
looser prefix inside a tighter operand.

Every applied run writes `optimizations.json` beside `semantics.json` and
`decisions.json`. The manifest records the safety class, original source
spans, removed-rule and alternative-label migrations, grouping changes, and
projected context/decision reductions. Inspect the same deterministic report
without generating or changing a parser with:

```bash
antlr4-rust-gen Grammar.g4 \
  --report-precedence-ladders \
  --out-dir target/grammar-report
```

Report mode writes only `optimizations.json`. Apply a reviewed candidate with:

```bash
antlr4-rust-gen Grammar.g4 \
  --optimize-precedence-ladders \
  --out-dir src/generated
```

### Binary and Byte-Oriented Parsing

ANTLR grammars can parse binary formats, not just text. The convention the
reference runtimes use is to treat each byte as a codepoint in
`U+0000..=U+00FF` and write lexer rules over that range
(`BYTE : '\u0000' .. '\u00FF';`). This runtime ships
[`ByteStream`](crates/antlr-rust-runtime/src/byte_stream.rs) for exactly that: a `CharStream` backed by
raw bytes where the stream index is the byte offset and lookahead returns the
byte value (`0..=255`). It is generic over the
backing store — `ByteStream::new(vec)` owns, `ByteStream::new(&buf[..])` borrows
a network buffer zero-copy, and `ByteStream::from_reader(file)?` drains any
`std::io::Read`. Because the bytes are not text, `text()` renders a matched span
as lowercase hex. Generated parser modules accept it through `parse_stream`, so
binary inputs retain the same compact lexer/token-stream/parser setup as text:

```rust
let input = ByteStream::from_reader(file)?;
let parsed = midi_parser::parse_stream(input, MidiLexer::new, MidiParser::file)?;
```

Length-prefixed formats ("read N, then consume N bytes") are data-dependent, so
a pure grammar cannot frame them alone — the same constraint ANTLR's `bencoding`
grammar solves with a lexer `superClass`. Here that role is filled by a
[`SemanticHooks`](crates/antlr-rust-runtime/src/parser.rs) implementation: `LexerSemCtx`/`LexerLifecycleCtx`
expose `push_mode`/`pop_mode`, `enqueue_token` (to synthesize framing tokens),
and raw `la()` lookbehind, so a small hook struct can count down a declared
chunk length and emit an end-of-chunk token. A bare `{helper();}` lexer action
lowers to a typed hook method via a `--sem-patterns` `[[helper]]` entry with
`kind = "lexer-action"`, `lower = "hook"`.

A complete worked example — a Standard MIDI File grammar (MThd/MTrk chunks,
variable-length delta-times, note and meta events) with a chunk-framing hook,
parsed over a `ByteStream` from a real `.mid` fixture — lives in
[`crates/antlr-rust-codegen/tests/fixtures/antlr4-rust-gen/midi-binary/`](crates/antlr-rust-codegen/tests/fixtures/antlr4-rust-gen/midi-binary/)
and its integration test (`midi_binary_grammar_parses_standard_midi_file_over_byte_stream`
in [parser.rs](crates/antlr-rust-codegen/tests/antlr4_rust_gen_cli/parser.rs)). The grammar is
adapted from [milnet2/midi-grammar](https://github.com/milnet2/midi-grammar)
(Tobias Blaschke, BSD-3-Clause).

## Runtime Testsuite

Keep the ANTLR jar and upstream runtime-testsuite under
`target/antlr-cleanroom`. The repository ignores this directory, and unlike an operating-system
temporary directory it is not periodically purged. `cargo clean` removes it.

Run the full sweep with:

```bash
cargo run --release --quiet -p antlr-rust-runtime-testsuite --bin antlr4-runtime-testsuite
```

The harness runs descriptors the way every official ANTLR target does: each
descriptor grammar is rendered through
`tests/antlr-rust-runtime-testsuite/templates/Rust.test.stg` with the real
StringTemplate engine, so its actions and predicates become real Rust code.
The rendered `.g4` source graph is then compiled directly by
`antlr4-rust-gen`, and the resulting code is executed inline.

Run a specific descriptor:

```bash
cargo run -p antlr-rust-runtime-testsuite --bin antlr4-runtime-testsuite -- \
  --antlr-jar path/to/antlr-4.13.2-complete.jar \
  --descriptors path/to/antlr4/runtime-testsuite \
  --case LexerExec/KeywordID
```

## Performance

`tools/parse-bench/` benchmarks parse throughput of the generated Rust parsers
against the upstream Go runtime (`github.com/antlr4-go/antlr/v4`) — and
optionally the reference Python runtime and tree-sitter — on real-world Kotlin,
C#, Java, and Trino SQL fixtures. See
[`tools/parse-bench/README.md`](tools/parse-bench/README.md) for setup (the
ANTLR jar, the `grammars-v4` sparse checkout, and the Python dependencies).

Run the Rust-vs-Go comparison across all fixture languages:

```bash
python3 tools/parse-bench/run.py \
  --languages kotlin,csharp,java,trino \
  --runtimes rust-antlr,go-antlr \
  --iters 10 \
  --warmups 2 \
  --json target/parse-bench/results.json \
  --markdown target/parse-bench/results.md
```

Add `--ast-check` to require byte-identical Rust/Go parse trees (no error nodes)
before timing. Prefer that gate for fair comparisons; some C# Mono fixtures still
diverge today (see `tools/parse-bench/README.md`). For a clean smoke:

```bash
python3 tools/parse-bench/run.py \
  --languages kotlin,trino \
  --runtimes rust-antlr,go-antlr \
  --ast-check \
  --quick
```

The report prints `min`/`avg` parse time and a ratio against `rust-antlr` for
every fixture. Use `--quick` for a 3-iteration/1-warmup smoke run, or adjust
`--iters`/`--warmups` for longer, lower-variance runs; add
`--runtimes rust-antlr,go-antlr,python-antlr,tree-sitter` to include the other
runtimes.

### Current results

Relative parse speed of this runtime versus the Go runtime, summarized as the
geometric mean of the per-fixture `go ÷ rust` parse-time ratios in each language
group (**> 1.0** means Rust is faster than Go; **< 1.0** means slower):

| Language | Fixtures | Rust vs Go (parse time) |
|----------|---------:|-------------------------|
| Kotlin   | 4        | 30.336x                 |
| Java     | 4        | 3.166x                  |
| C#       | 4        | 2.177x                  |
| Trino SQL | 5       | 3.311x                  |

## Useful Information

[**Awesome ANTLR**](https://github.com/ophi-dev/awesome-antlr) — our curated
list of ANTLR resources: the tool and its documentation, runtimes for every
target language, editor and build tooling, grammar collections, and the research
behind adaptive LL(*) parsing.
