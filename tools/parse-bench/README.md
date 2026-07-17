# Parse Benchmark

This benchmark compares parse throughput for generated ANTLR parsers and
tree-sitter parsers on Kotlin, C#, and Java fixtures, with an explicit Trino
SQL fixture set for SQL-dialect Rust vs Go checks.

The harness is intentionally a standalone script instead of `cargo bench`.
`cargo bench` is useful for in-process Rust-only measurements, but this check
has to generate ANTLR parsers, build a Go binary, run Python parsers, and load
tree-sitter language libraries. Keeping that orchestration outside Cargo makes
the same command usable locally and in CI.

## Setup

Use the same ANTLR jar and `grammars-v4` checkout described in `AGENTS.md`.
The benchmark defaults to:

- `ANTLR4_JAR=/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar`
- `GRAMMARS_V4=/tmp/antlr-cleanroom/grammars-v4`

The sparse checkout must include C#, the modern Java grammar, and Trino SQL in
addition to Kotlin:

```bash
git -C /tmp/antlr-cleanroom/grammars-v4 sparse-checkout set kotlin/kotlin csharp/v7 java/java sql/trino
```

Install the Python dependencies in the interpreter you will use to run the
benchmark:

```bash
python3 -m pip install -r tools/parse-bench/requirements.txt
```

## Run

Quick local smoke:

```bash
python3 tools/parse-bench/run.py --quick
```

SQL-only Rust vs Go smoke:

```bash
python3 tools/parse-bench/run.py \
  --languages trino \
  --runtimes rust-antlr,go-antlr \
  --quick
```

Longer local run with reports:

```bash
python3 tools/parse-bench/run.py \
  --iters 20 \
  --warmups 3 \
  --rust-generated-only \
  --json target/parse-bench/results.json \
  --markdown target/parse-bench/results.md
```

The script regenerates parsers into `target/parse-bench`, builds:

- a Rust runner using this runtime and generated `.interp` metadata,
- a Python ANTLR runner using `antlr4-python3-runtime`,
- a Go ANTLR runner using `github.com/antlr4-go/antlr/v4`,
- a tree-sitter runner using `tree-sitter-language-pack`.

The output table reports `min` and `avg` parse time per fixture and a relative
ratio against `rust-antlr` for the same fixture.
Use `--rust-generated-only` for Adaptive LL delivery evidence so the Rust
generator fails if any parser rule lacks a generated body and the Rust runner
fails if a generated parser path falls back to the interpreter.

### Lex-only measurements

Use `--phase lex` to time generated Rust lexing and token buffering without
constructing a parser:

```bash
python3 tools/parse-bench/run.py \
  --phase lex \
  --languages kotlin,csharp,java,trino \
  --runtimes rust-antlr \
  --iters 20 \
  --warmups 3
```

The source-derived fixtures cover ordinary Kotlin, C#, Java, and Trino input.
Two lex-only Kotlin fixtures add concentrated ASCII coverage for long
identifiers, strings, comments, whitespace, and punctuation, plus mixed-script
coverage for the Unicode fallback.

Use a detached checkout for same-machine baseline comparisons, and select the
compiler-level configurations explicitly:

```bash
python3 tools/parse-bench/run.py \
  --phase lex \
  --runtimes rust-antlr \
  --runtime-root /tmp/antlr-runtime-main \
  --rust-native \
  --rust-thin-lto
```

`--rust-native` adds `-C target-cpu=native`. `--rust-thin-lto` writes
`lto = "thin"` and `codegen-units = 1` in the generated benchmark workspace,
where Cargo profile settings control the final application and its
dependencies.

## Prediction Memory Counters

Set `ANTLR_PERF_DUMP=1` to build the Rust runner with performance counters.
Parse runs print prediction and context-store measurements; lex-only runs print
lexer direct-ASCII, generic-character, scalar-replay, and bulk-commit counts:

```bash
ANTLR_PERF_DUMP=1 python3 tools/parse-bench/run.py \
  --languages csharp \
  --runtimes rust-antlr \
  --iters 10 \
  --warmups 2 \
  --rust-generated-only
```

The dump includes canonical context counts, pooled and retained bytes, arena
and workspace capacities, merge-cache activity, and outer-context cache
hits/misses. It also reports learned parser-DFA warm hits/misses, ATN
fallbacks, state interning activity, dense/sparse row counts, edge-density
histograms, and hot/cold retained bytes. Store statistics are collected in a
separate untimed parse after the benchmark loop, so walking the stores does not
affect reported timings.

## PR Watchdog

For CI, run the benchmark on the base checkout and the PR checkout on the same
runner, then compare JSON reports:

```bash
python3 tools/parse-bench/compare.py \
  --baseline base-parse-bench.json \
  --current head-parse-bench.json \
  --max-regression 1.15
```

By default the comparator checks `rust-antlr` only. Repeat `--runtime` to add
other runtimes.

## Fixtures

Fixture metadata lives in `fixtures/manifest.json`. The fixture files are
source-referenced benchmark inputs that point at independent upstream parser
stress patterns:

- Kotlin: JetBrains Kotlin, kotlinx.coroutines, Ktor.
- C#: dotnet/wpf, Mono.
- Java: Mojang DataFixerUpper, Bazel, Google Closure Compiler, Trino.
- Trino SQL: Trino benchmark TPC-DS/TPC-H queries with benchmark placeholders
  normalized to identifiers, including a curated TPC-DS grammar-stress suite
  selected by CTE/window/UNION/EXISTS/CASE/grouping feature density.
