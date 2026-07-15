# ANTLR4 Runtime for Rust

[![Crates.io Version](https://img.shields.io/crates/v/antlr-rust-runtime)](https://crates.io/crates/antlr-rust-runtime)
[![ANTLR Runtime Testsuite](https://github.com/ophi-dev/antlr-rust-runtime/actions/workflows/antlr-runtime-testsuite.yml/badge.svg)](https://github.com/ophi-dev/antlr-rust-runtime/actions/workflows/antlr-runtime-testsuite.yml)

`antlr-rust-runtime` is a pure Rust runtime and metadata generator for ANTLR v4
lexers and parsers. It is a clean-room implementation written from scratch from
the public ANTLR runtime contract; it does not vendor or fork an older Rust
ANTLR runtime.

## First Steps

### 1. Install ANTLR4

Follow the ANTLR getting-started guide and install the ANTLR tool jar. The
runtime tests currently validate against ANTLR `4.13.2`.

### 2. Install the Rust ANTLR runtime tools

Each ANTLR target language needs a runtime package used by generated parsers.
For Rust projects, add the runtime crate:

```toml
[dependencies]
antlr-rust-runtime = "0.9"
```

The library crate is imported as `antlr4_runtime`:

```rust
use antlr4_runtime::{CommonTokenStream, InputStream};
```

Install the companion generator binary:

```bash
cargo install antlr-rust-runtime
```

This installs `antlr4-rust-gen`, which turns ANTLR `.interp` metadata into Rust
lexer and parser modules. During generation it also compiles the lexer's DFA
ahead of time and embeds the tables in the generated lexer, so tokenization
runs at full speed from the first character with no per-process warmup.

### 3. Generate your parser

The current release uses a metadata-first generation path:

1. run the official ANTLR tool to produce `.interp` files,
2. run `antlr4-rust-gen` to emit Rust modules,
3. compile those modules against `antlr4_runtime`.

For a split lexer/parser grammar:

```bash
antlr4 MyGrammarLexer.g4 MyGrammarParser.g4

antlr4-rust-gen \
  --lexer MyGrammarLexer.interp \
  --parser MyGrammarParser.interp \
  --out-dir src/generated
```

The checked-in ANTLR `RustTarget`/StringTemplate shell is kept in `tool/` and
will be expanded around the same runtime contracts.

### Alternative: Generate metadata with antlr-ng

[`antlr-ng`](https://www.antlr-ng.org/introduction.html) is a TypeScript/npm
parser generator based on ANTLR 4.13.2. It does not currently ship a Rust
target, but it can produce the same `.interp` metadata that `antlr4-rust-gen`
uses.

Install it with npm or run it through `npx`:

```bash
npx -y antlr-ng -Dlanguage=Java -o build/antlr --exact-output-dir true JSON.g4
```

The `-Dlanguage=Java` option selects one of antlr-ng's bundled code-generation
targets only so the tool emits grammar artifacts, including `JSONLexer.interp`
and `JSON.interp`. The Java files can be ignored; Rust code still comes from
`antlr4-rust-gen`:

```bash
antlr4-rust-gen \
  --lexer build/antlr/JSONLexer.interp \
  --parser build/antlr/JSON.interp \
  --out-dir src/generated
```

For local tooling, antlr-ng requires Node.js 20 or newer. See the
[antlr-ng getting-started guide](https://www.antlr-ng.org/getting-started.html)
for CLI installation and option details.

## Complete Example

Suppose you are using the JSON grammar from `antlr/grammars-v4/json`.

Fetch or copy `JSON.g4`, then generate ANTLR metadata:

```bash
antlr4 JSON.g4
```

Generate Rust modules:

```bash
antlr4-rust-gen \
  --lexer JSONLexer.interp \
  --parser JSON.interp \
  --out-dir src/generated
```

Declare the generated modules in your crate:

```rust
mod generated {
    #![allow(dead_code)]

    pub mod json;
    pub mod json_lexer;
}
```

Call the generated parser helper for the compact path:

```rust
use generated::json::{self, Json};
use generated::json_lexer::JsonLexer;

fn main() -> Result<(), antlr4_runtime::AntlrError> {
    let tree = json::parse(r#"{"a":1}"#, JsonLexer::new, Json::json)?;

    println!("{}", tree.text());
    Ok(())
}
```

Use `parse_with_parser` when you want the compact setup path and also need the
parser afterward for diagnostics or the owned token stream:

```rust
use antlr4_runtime::Parser;
use generated::json::{self, Json};
use generated::json_lexer::JsonLexer;

fn main() -> Result<(), antlr4_runtime::AntlrError> {
    let output = json::parse_with_parser(r#"{"a":1}"#, JsonLexer::new, Json::json)?;
    let syntax_errors = output.parser.number_of_syntax_errors();
    let tree = output.result;
    let tokens = output.parser.into_token_stream();

    println!("{} errors across {} tokens", syntax_errors, tokens.tokens().len());
    println!("{}", tree.text());
    Ok(())
}
```

Or construct each layer explicitly when you need to set source names, parser
options, or custom error handling before invoking the entry rule:

```rust
use antlr4_runtime::{CommonTokenStream, InputStream};
use generated::json::Json;
use generated::json_lexer::JsonLexer;

fn main() -> Result<(), antlr4_runtime::AntlrError> {
    let lexer = JsonLexer::new(InputStream::new(r#"{"a":1}"#));
    let tokens = CommonTokenStream::new(lexer);
    let mut parser = Json::new(tokens);
    let tree = parser.json()?;

    println!("{}", tree.text());
    Ok(())
}
```

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
- Supports ANTLR serialized ATN deserialization.
- Supports lexer and parser execution through generated Rust wrappers.
- Supports real split lexer/parser grammars, including Kotlin smoke builds.
- Passes every upstream ANTLR runtime-testsuite descriptor discovered by the
  harness: `357 passed, 0 failed, 0 skipped, 357 run`.
- Licensed under BSD-3-Clause for compatibility with ANTLR's runtime licensing
  pattern and downstream open-source applications.

The runtime contains:

- `IntStream` and `CharStream`
- UTF-8 input as Unicode scalar values
- `Token`, `CommonToken`, token factories, and `TokenSource`
- buffered, channel-aware `CommonTokenStream`
- `Vocabulary`
- recognizer metadata and error listener plumbing
- parse tree node types, rule contexts, terminal nodes, error nodes, and walkers
- ANTLR v4 serialized ATN deserialization
- lexer ATN recognition with longest-match/rule-priority behavior and lexer
  actions
- ahead-of-time compiled lexer DFA tables, built by `antlr4-rust-gen` and
  embedded in generated lexers, with per-token escape to ATN interpretation
  for constructs a finite DFA cannot represent (semantic predicates,
  recursive lexer rules)
- parser ATN rule recognition with backtracking over token stream indices
- `antlr4-rust-gen`, a Rust generator that consumes ANTLR `.interp` metadata and
  emits Rust modules
- `antlr4-runtime-testsuite`, a harness for running upstream ANTLR
  runtime-test descriptors through the Rust metadata path

See [docs/kotlin-build.md](docs/kotlin-build.md) for the Kotlin smoke workflow.
See [docs/runtime-testsuite.md](docs/runtime-testsuite.md) for the upstream
runtime-testsuite harness.

### Semantic Predicates and Actions: the Compatibility Boundary

ANTLR grammars may embed **target-language** semantic predicates and actions
(`{isTypeName()}?`, `{this.count++;}`). The serialized ATN records *where*
they occur, but not executable code, so a metadata-first runtime cannot run
arbitrary grammar-embedded snippets. The boundary is:

- **Target-agnostic grammars** — no embedded code, or only built-in lexer
  commands (`skip`, `channel(...)`, `mode(...)`, `type(...)`) — are fully
  supported.
- **Recognized predicate/action shapes** — a library of common idioms
  (constant predicates, lookahead text/type checks, integer member counters,
  column predicates, and the upstream testsuite's action templates) — are
  translated into SemIR by `antlr4-rust-gen` when the grammar source is
  passed via `--grammar`.
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

Unknown coordinates are governed by `--sem-unknown`:

```bash
antlr4-rust-gen --lexer L.interp --parser P.interp --grammar G.g4 \
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
`SemanticHooks` trait.

Generated lexers also own optional hook state and emit typed lexer adapters
when a semantic pattern maps lexer helper calls to hooks. The official
grammars-v4 JavaScript and TypeScript grammars are complete examples, including
checked-in Rust lexer/parser base modules and strict build commands; see
[`docs/javascript-build.md`](docs/javascript-build.md) and
[`docs/typescript-build.md`](docs/typescript-build.md).

Use `--require-full-semantics` in CI when every coordinate must be either
translated or explicitly hooked; policy fallbacks fail generation.

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
entirely: `antlr4-rust-gen --actions embedded --grammar Foo.g4` splices the
bodies verbatim (after `$`-attribute translation) into the generated parser,
inline at their ATN action/predicate coordinates. This is the mode the
conformance harness uses after rendering descriptor grammars through
`Rust.test.stg` (see below).

## Runtime Testsuite

On the maintainer checkout, where the ANTLR jar and upstream runtime-testsuite
live under `/tmp/antlr-cleanroom`, run the full sweep with:

```bash
cargo run --release --quiet --bin antlr4-runtime-testsuite
```

The harness runs descriptors the way every official ANTLR target does: each
descriptor grammar is rendered through `.conformance-review/Rust.test.stg`
with the real StringTemplate engine, so its actions and predicates become real
Rust code that is compiled and executed inline.

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
  --quick \
  --json target/parse-bench/results.json \
  --markdown target/parse-bench/results.md
```

The report prints `min`/`avg` parse time and a ratio against `rust-antlr` for
every fixture. Drop `--quick` (or add `--iters`/`--warmups`) for longer, lower
variance runs; add `--runtimes rust-antlr,go-antlr,python-antlr,tree-sitter` to
include the other runtimes.

### Current results

Relative parse speed of this runtime versus the Go runtime, summarized as the
geometric mean of the per-fixture `go ÷ rust` parse-time ratios in each language
group (**> 1.0** means Rust is faster than Go; **< 1.0** means slower):

| Language | Fixtures | Rust vs Go (parse time) |
|----------|---------:|-------------------------|
| Kotlin   | 4        | ~18.5× faster           |
| Java     | 4        | ~2.5× faster            |
| C#       | 4        | ~1.7× faster            |
| Trino SQL| 5        | ~2.2× faster            |

Rust is faster than Go on average in all four language groups, with
Kotlin leading dramatically (expression-ladder memoization in the generated
walker). Lexer DFAs are compiled at generation time and embedded in the
generated lexer, so tokenization needs no warmup at all; learned parser
decision DFAs are shared across parser instances, so repeated parses of the
same grammar — the common case for a CLI tool or language server — skip
relearning entirely. Shared grammar-level lookahead caches likewise amortize
left-recursive loop prediction across parses. Numbers are warm-parse minimums
from 10 measured iterations after two warmups on an Apple M3 Pro and are
indicative — re-run the benchmark on your own hardware for authoritative
figures.

## Useful Information

- ANTLR: <https://www.antlr.org/>
- ANTLR documentation: <https://github.com/antlr/antlr4/blob/dev/doc/index.md>
- Grammars v4: <https://github.com/antlr/grammars-v4>
