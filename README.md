# ANTLR4 Runtime for Rust

[![Crates.io Version](https://img.shields.io/crates/v/antlr-rust-runtime)](https://crates.io/crates/antlr-rust-runtime)
[![ANTLR Runtime Testsuite](https://github.com/ophi-dev/antlr-rust-runtime/actions/workflows/antlr-runtime-testsuite.yml/badge.svg)](https://github.com/ophi-dev/antlr-rust-runtime/actions/workflows/antlr-runtime-testsuite.yml)
[![codecov](https://codecov.io/github/ophi-dev/antlr-rust-runtime/graph/badge.svg?token=QzgT4jB57u)](https://codecov.io/github/ophi-dev/antlr-rust-runtime)

`antlr-rust-runtime` is a pure Rust runtime and source generator for ANTLR v4
lexers and parsers. It is a clean-room implementation written from scratch from
the public ANTLR runtime contract; it does not vendor or fork an older Rust
ANTLR runtime.

## First Steps

### 1. Get an ANTLR4 grammar

Use your own `.g4` files or a grammar from
[`antlr/grammars-v4`](https://github.com/antlr/grammars-v4). Rust generation
does not require Java, Node.js, the ANTLR tool jar, or an intermediate
`.interp` file. The repository's differential tests use ANTLR `4.13.2` only as
an explicit compatibility oracle.

### 2. Install the Rust ANTLR runtime tools

Each ANTLR target language needs a runtime package used by generated parsers.
For Rust projects, add the runtime crate:

<!-- x-release-please-start-version -->

```toml
[dependencies]
antlr-rust-runtime = "0.16.0"
```

<!-- x-release-please-end -->

The library crate is imported as `antlr4_runtime`:

```rust
use antlr4_runtime::{CommonTokenStream, InputStream};
```

Install the companion generator binary:

```bash
cargo install antlr-rust-runtime --features codegen --bin antlr4-rust-gen
```

This installs `antlr4-rust-gen`, which compiles ANTLR `.g4` source into Rust
lexer and parser modules. During generation it also compiles the lexer's DFA
ahead of time and embeds the tables in the generated lexer, so tokenization
runs at full speed from the first character with no per-process warmup.

### 3. Generate your parser

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
`comma_tokens()`.

`--no-visitor` disables visitor generation. The generator also accepts ANTLR's
single-dash spellings (`-listener`, `-no-listener`, `-visitor`,
`-no-visitor`).

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

Or construct each layer explicitly when you need to set source names, parser
options, or custom error handling before invoking the entry rule:

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
that matches the grammar's intended top-level rule for the input; the generator
can identify rules that are not called by other rules, but it cannot infer the
semantic choice between multiple top-level forms. The generated parser rustdoc
lists likely entry methods first, followed by all rule methods.

For the JSON grammar above, `json()` is the natural entry. Larger grammars may
have several top-level forms, so confirm the intended entry rule against that
grammar's documentation. Calling the wrong rule can still recover and return a
parse tree with error nodes, so check parser diagnostics when adding a new input
form.

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
- parse-tree XPath queries on par with the official ANTLR runtimes
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
- `antlr4-rust-gen`, a source-only Rust generator that compiles `.g4` roots and
  their import graph into Rust modules
- `antlr4-runtime-testsuite`, a harness for running upstream ANTLR
  runtime-test descriptors through the direct Rust source compiler

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
  rewrites, helper-call rewrites, and per-coordinate `hook` /
  `assume-true` / `assume-false` / `error` dispositions without changing the
  generator.
- **Everything else is not silently guessed.** Each generator run writes a
  `semantics.json` manifest next to the generated modules listing every
  predicate/action coordinate with its grammar source span, body, and
  disposition (`translated`, `hooked`, `assume-true`, `assume-false`,
  `ignored`, `synthetic`, or `error`). A `synthetic` action is one ANTLR
  inserts itself (e.g. during left-recursion elimination); it has no
  grammar-author source, is a runtime no-op, and is exempt from the `error`
  gate — only actions the author actually wrote in the grammar can fail it.

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
parser action events are offered to `SemanticHooks::action` after the committed
parse path is selected. Predicate hooks may run speculatively during
prediction, so they must be replay-safe.

For bare helper-call predicates, generated parsers also emit a typed hook
adapter (`MyParserHooks` plus `MyParserTypedHooks<T>`) that maps stable
manifest coordinates to named Rust methods. Lexer callers can use
`LexerSemCtx` with `atn::lexer::next_token_with_semantic_hooks` or the
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

### Binary and Byte-Oriented Parsing

ANTLR grammars can parse binary formats, not just text. The convention the
reference runtimes use is to treat each byte as a codepoint in
`U+0000..=U+00FF` and write lexer rules over that range
(`BYTE : '\u0000' .. '\u00FF';`). This runtime ships
[`ByteStream`](src/byte_stream.rs) for exactly that: a `CharStream` backed by
raw bytes where the stream index is the byte offset and lookahead returns the
byte value (`0..=255`). It is generic over the
backing store — `ByteStream::new(vec)` owns, `ByteStream::new(&buf[..])` borrows
a network buffer zero-copy, and `ByteStream::from_reader(file)?` drains any
`std::io::Read`. Because the bytes are not text, `text()` renders a matched span
as lowercase hex.

Length-prefixed formats ("read N, then consume N bytes") are data-dependent, so
a pure grammar cannot frame them alone — the same constraint ANTLR's `bencoding`
grammar solves with a lexer `superClass`. Here that role is filled by a
[`SemanticHooks`](src/parser.rs) implementation: `LexerSemCtx`/`LexerLifecycleCtx`
expose `push_mode`/`pop_mode`, `enqueue_token` (to synthesize framing tokens),
and raw `la()` lookbehind, so a small hook struct can count down a declared
chunk length and emit an end-of-chunk token. A bare `{helper();}` lexer action
lowers to a typed hook method via a `--sem-patterns` `[[helper]]` entry with
`kind = "lexer-action"`, `lower = "hook"`.

A complete worked example — a Standard MIDI File grammar (MThd/MTrk chunks,
variable-length delta-times, note and meta events) with a chunk-framing hook,
parsed over a `ByteStream` from a real `.mid` fixture — lives in
[`tests/fixtures/antlr4-rust-gen/midi-binary/`](tests/fixtures/antlr4-rust-gen/midi-binary/)
and its integration test (`midi_binary_grammar_parses_standard_midi_file_over_byte_stream`
in [tests/antlr4_rust_gen_cli.rs](tests/antlr4_rust_gen_cli.rs)). The grammar is
adapted from [milnet2/midi-grammar](https://github.com/milnet2/midi-grammar)
(Tobias Blaschke, BSD-3-Clause).

## Runtime Testsuite

On the maintainer checkout, where the ANTLR jar and upstream runtime-testsuite
live under `/tmp/antlr-cleanroom`, run the full sweep with:

```bash
cargo run --release --quiet --bin antlr4-runtime-testsuite
```

The harness runs descriptors the way every official ANTLR target does: each
descriptor grammar is rendered through `.conformance-review/Rust.test.stg`
with the real StringTemplate engine, so its actions and predicates become real
Rust code. The rendered `.g4` source graph is then compiled directly by
`antlr4-rust-gen`, and the resulting code is executed inline.

Run a specific descriptor:

```bash
cargo run --bin antlr4-runtime-testsuite -- \
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

- ANTLR: <https://www.antlr.org/>
- ANTLR documentation: <https://github.com/antlr/antlr4/blob/dev/doc/index.md>
- Grammars v4: <https://github.com/antlr/grammars-v4>
