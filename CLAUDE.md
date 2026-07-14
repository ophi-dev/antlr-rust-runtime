# Development notes

## Inner loop

```bash
cargo test --locked                                                 # unit tests
cargo clippy --locked --all-targets --all-features -- -D warnings   # what CI runs
```

CI's clippy runs with the same `-D warnings` and promotes nursery/pedantic lints
(`clippy::excessive-nesting`, `clippy::disallowed_types`, …) to errors — reproduce
locally before pushing.

Run `cargo fmt` on files you touched before committing so formatting-only churn
doesn't ride along with logic changes (and never bulk-`cargo fmt` unrelated files
in a logic commit). Hand-grouped data — e.g. the positional serialized-ATN
fixtures in `src/atn/lexer_dfa.rs`, laid out one record-per-line to mirror the
ANTLR layout — carries `#[rustfmt::skip]`; leave those attributes in place rather
than letting fmt explode the block to one element per line.

## Source layout

- `src/lib.rs` — public exports
- `src/lexer.rs`, `src/atn/lexer.rs` — `BaseLexer` + lexer ATN simulator
- `src/parser.rs` — `BaseParser` and the recursive `recognize_state_fast` walker
- `src/atn/`, `src/atn/serialized.rs` — ATN graph + ANTLR `.interp` deserializer
- `src/prediction.rs` — `PredictionContext`, `AtnConfig`, `PredictionFxHasher`
- `src/token.rs`, `src/token_stream.rs`, `src/char_stream.rs` — input + token plumbing
- `src/tree.rs` — public `ParseTree` / `ParserRuleContext`
- `src/bin/antlr4-rust-gen.rs` — `.interp` → Rust parser code generator
- `src/bin/antlr4-runtime-testsuite.rs` — conformance harness (see below)
- `tests/kotlin-parity/` — Kotlin parity dumper + snippets
- `tools/parse-bench/` — Python harness comparing rust/go/python/tree-sitter parse times

## Generated parser codegen

```bash
cargo run --release --bin antlr4-rust-gen -- \
    --lexer  path/to/Foo.interp \
    --parser path/to/FooParser.interp \
    --out-dir crates/foo/src/generated
```

The output crate must depend on this runtime (`antlr-rust-runtime = { path = ... }`).
Both the kotlin-parity dumper and the parse-bench runner are examples.

Every run also writes a `semantics.json` manifest into `--out-dir` listing each
semantic predicate/action coordinate and its disposition. `--sem-unknown
error|hook|assume-true|assume-false`, `--sem-patterns`, and
`--require-full-semantics` control untranslated coordinates (default
`assume-true`, deprecated; see the README "Semantic Predicates and Actions"
section and issue #9).
Generated parsers emit SemIR tables, `with_hooks(tokens, hooks)`, and typed
hook adapters for bare helper predicates; lexer callers can route closure hooks
through `LexerSemCtx` and the shared `SemanticHooks` trait.

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

Defaults to `ANTLR4_JAR=/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar` and `ANTLR4_RUNTIME_TESTSUITE=/tmp/antlr-cleanroom/antlr4-upstream/runtime-testsuite`. Override with `--antlr-jar`/`--descriptors` or env vars. Cases run on `--jobs` parallel workers (default `min(cores, 8)`), each with its own cargo target-dir stripe; the render driver and `antlr4-rust-gen` are prebuilt once per sweep. Wall-clock ≈ 2 minutes on Apple Silicon.

### The rendered (embedded-actions) pipeline

The harness runs descriptors the way every official ANTLR target does:
each descriptor grammar is rendered through
`.conformance-review/Rust.test.stg` with the real StringTemplate engine
(`tools/stg-render/RenderGrammar.java`, executed via the ANTLR jar and the
Java single-file source launcher), so its actions/predicates become real
Rust code. The rendered grammar feeds both the ANTLR tool and
`antlr4-rust-gen --actions embedded`, which splices the bodies verbatim
after `$`-attribute translation (`src/bin_support/embedded.rs`) and
generates typed context views, per-rule attrs structs, members
fields/methods, listener traits, and recognizer facades. `--stg PATH`
overrides the template group. (An earlier template-recognition pipeline,
which simulated action output instead of executing it, was replaced by
this one before ever shipping.)

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

## Parse benchmark (vs Go / Python / tree-sitter)

`tools/parse-bench/` runs ANTLR-generated Kotlin and C# parsers and reports
min/avg parse time per fixture. CI runs it on every PR.

The C# fixtures need an extra grammar checked out (Kotlin is in the one-time
setup above):

```bash
git -C /tmp/antlr-cleanroom/grammars-v4 sparse-checkout set kotlin/kotlin csharp/v7
python3 -m pip install -r tools/parse-bench/requirements.txt
python3 tools/parse-bench/run.py \
    --antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar \
    --grammars-v4 /tmp/antlr-cleanroom/grammars-v4
```

See `tools/parse-bench/README.md` for `--quick`, `--languages`, `--runtimes`,
JSON / Markdown output, and the per-runner build details.

## perf-counters feature

```bash
cargo build --release --features perf-counters
ANTLR_PERF_DUMP=1 ./your-parser-binary  # dumps RFS_CALLS, MEMO_HITS, OUTCOMES_RETURN_*, …
```

Opt-in counters compiled out by default; useful for "where did the new ms come
from?" investigations. Disabled in default builds so they don't tax the inner
loop.

## CI parity

CI runs `cargo clippy --locked --all-targets --all-features -- -D warnings`, so reproduce locally with the same flags before pushing — `clippy::excessive-nesting`, `clippy::disallowed_types`, and similar nursery/pedantic lints all promote to errors there.

`AGENTS.md` mirrors this file for Codex / generic agents — keep them in sync when adding sections.
