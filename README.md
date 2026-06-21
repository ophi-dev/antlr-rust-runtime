# ANTLR4 Runtime for Rust

![Crates.io Version](https://img.shields.io/crates/v/antlr-rust-runtime)
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
antlr-rust-runtime = "0.4"
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
lexer and parser modules.

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
cannot infer that semantic choice from `.interp` metadata. The generated parser
rustdoc lists the available rule methods.

For the JSON grammar above, `json()` is the natural entry. Larger grammars may
have several top-level forms: with the Kotlin grammar, `.kt` compilation units
typically use `kotlin_file()`, while script-style `.kts` input uses `script()`.
Calling the wrong rule can still recover and return a parse tree with error
nodes, so check parser diagnostics when adding a new input form.

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
- parser ATN rule recognition with backtracking over token stream indices
- `antlr4-rust-gen`, a Rust generator that consumes ANTLR `.interp` metadata and
  emits Rust modules
- `antlr4-runtime-testsuite`, a harness for running upstream ANTLR
  runtime-test descriptors through the Rust metadata path

See [docs/kotlin-build.md](docs/kotlin-build.md) for the Kotlin smoke workflow.
See [docs/runtime-testsuite.md](docs/runtime-testsuite.md) for the upstream
runtime-testsuite harness.

## Runtime Testsuite

On the maintainer checkout, where the ANTLR jar and upstream runtime-testsuite
live under `/tmp/antlr-cleanroom`, run the full sweep with:

```bash
cargo run --quiet --bin antlr4-runtime-testsuite
```

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
| Kotlin   | 4        | ~10× faster             |
| Java     | 4        | ~0.9× (roughly on par)  |
| C#       | 4        | ~0.45× (Go ~2.2× faster)|
| Trino SQL| 5        | ~0.4× (Go ~2.6× faster) |

Rust is dramatically faster on Kotlin (expression-ladder memoization in the
generated walker) and near parity on Java; C# and Trino remain ahead for Go and
are the focus of ongoing prediction/closure optimization. Numbers are quick-mode
(`--quick`, best-of-min) on an Apple M3 Pro and are indicative — re-run the
benchmark on your own hardware for authoritative figures.

## Useful Information

- ANTLR: <https://www.antlr.org/>
- ANTLR documentation: <https://github.com/antlr/antlr4/blob/dev/doc/index.md>
- Grammars v4: <https://github.com/antlr/grammars-v4>
