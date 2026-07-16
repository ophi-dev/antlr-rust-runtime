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

## Prediction Memory Counters

Set `ANTLR_PERF_DUMP=1` to build the Rust runner with prediction counters and
print context-store measurements:

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
hits/misses. Store statistics are collected in a separate untimed parse after
the benchmark loop, so walking the arena does not affect reported timings.

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
