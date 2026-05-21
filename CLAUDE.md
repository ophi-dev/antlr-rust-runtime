# Development notes

## Kotlin parser parity perf benchmark

Reproduces the timings against the Kotlin grammar from `antlr/grammars-v4`.

### One-time setup (fresh checkout)

```bash
# 1. ANTLR jar (any path; pin v4.13.2)
mkdir -p /tmp/antlr-cleanroom/tools
curl -fLo /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
    https://www.antlr.org/download/antlr-4.13.2-complete.jar

# 2. grammars-v4 checkout (sparse, just the kotlin grammar)
mkdir -p /tmp/antlr-cleanroom/grammars-v4
git -C /tmp/antlr-cleanroom/grammars-v4 init -q
git -C /tmp/antlr-cleanroom/grammars-v4 remote add origin https://github.com/antlr/grammars-v4.git
git -C /tmp/antlr-cleanroom/grammars-v4 sparse-checkout init --cone
git -C /tmp/antlr-cleanroom/grammars-v4 sparse-checkout set kotlin/kotlin
git -C /tmp/antlr-cleanroom/grammars-v4 fetch --depth 1 origin 284602b3f23ca54dc30778204ab7ae9e969145e9
git -C /tmp/antlr-cleanroom/grammars-v4 checkout FETCH_HEAD
```

### Run the parity smoke + dumper build

```bash
tests/kotlin-parity/run.sh \
    --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
    --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
```

That regenerates the Rust parser from the Kotlin grammar `.interp`, builds `tests/kotlin-parity/dumper`, and asserts the parse trees match `antlr4-python3-runtime` byte-for-byte.

### Measure parse-only timings

The dumper has a built-in parse-only stopwatch so process startup (~10 ms) is excluded:

```bash
DUMPER=tests/kotlin-parity/dumper/target/release/kotlin-parity-dumper
for snippet in tests/kotlin-parity/snippets/*.kt; do
    echo "=== $(basename "$snippet") ==="
    "$DUMPER" --input "$snippet" --output /tmp/dump.txt --iters 5 --time
done
```

`--iters N` repeats parse N times within one process; `--time` prints `min`/`avg` to stderr.

## ANTLR runtime testsuite

Validates the Rust runtime against ANTLR's upstream conformance descriptors.

### One-time setup

```bash
git clone --depth 1 https://github.com/antlr/antlr4 /tmp/antlr-cleanroom/antlr4-upstream
```

The harness reads `antlr4-upstream/runtime-testsuite` and the same ANTLR jar fetched above.

### Run the full sweep

```bash
cargo run --release --quiet --bin antlr4-runtime-testsuite
```

Defaults to `ANTLR4_JAR=/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar` and `ANTLR4_RUNTIME_TESTSUITE=/tmp/antlr-cleanroom/antlr4-upstream/runtime-testsuite`. Override with `--antlr-jar`/`--descriptors` or env vars. Expected outcome: `summary: 357 passed, 0 failed, 0 skipped, 357 run`. Wall-clock ≈ 10–15 minutes on Apple Silicon, ≈ 30 minutes on the GitHub Linux runner.

### Run a subset while iterating

```bash
# One descriptor:
cargo run --release --quiet --bin antlr4-runtime-testsuite -- --case LexerExec/KeywordID

# One group (e.g. while debugging left-recursion):
cargo run --release --quiet --bin antlr4-runtime-testsuite -- --group LeftRecursion --limit 20

# Keep the per-case temp crates for inspection:
cargo run --release --quiet --bin antlr4-runtime-testsuite -- --case ParserErrors/SingleSetInsertion --keep
```

Per-case scratch crates land under `target/antlr-runtime-testsuite/<case>/`. Stale dirs from a killed run can fail a re-run with `Os { code: 66, ... DirectoryNotEmpty }` — `rm -rf target/antlr-runtime-testsuite/*` to recover.

## CI parity

CI runs `cargo clippy --locked --all-targets --all-features -- -D warnings`, so reproduce locally with the same flags before pushing — `clippy::excessive-nesting`, `clippy::disallowed_types`, and similar nursery/pedantic lints all promote to errors there.
