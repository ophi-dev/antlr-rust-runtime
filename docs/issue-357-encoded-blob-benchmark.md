# Issue 357 encoded-blob source representation benchmark record

Measurements were taken on 2026-08-20 on a 16-core Intel Xeon Platinum 8275CL
with Rust 1.97.1 on Linux. The baseline was `main` at `5caa7d3e` in a separate
git worktree with its own target directory; the candidate was the same machine
and toolchain with the encoded-blob change applied and all checked-in
recognizers regenerated. All timing samples were interleaved between the two
trees. The grammars-v4 checkout for the Kotlin recognizer was
`284602b3f23ca54dc30778204ab7ae9e969145e9`.

## Representation

Generated static data tables — the ahead-of-time compiled lexer DFA, the
packed parser ATN, and the serialized lexer ATN inside `GrammarMetadata` —
change from decimal Rust integer arrays to one string literal per artifact:
LEB128 varints (zigzag-mapped for `i32` values) armored as canonical unpadded
base64, with a magic, format version, element kind, and checked element count
(`antlr4_runtime::encoded`). The decoded integer streams are byte-identical to
the previous arrays, so the inner serialized-DFA/packed-ATN formats and all
recognizer behavior are unchanged.

## Generated source size and rustc input

Payload text and lexical tokens for each data table (old tokens count the
integer literals plus separating commas; each blob is one string-literal
token). The ANTLRv4 lexer previously carried a single 2.72 MB source line;
blobs wrap at 120 columns via `\`-newline continuations.

| Artifact | values | old payload | new payload | old tokens | new tokens |
| --- | ---: | ---: | ---: | ---: | ---: |
| ANTLRv4 lexer DFA | 401,141 | 2,717,143 B | 1,582,816 B | ~802,281 | 1 |
| ANTLRv4 parser packed ATN | 8,952 | 42,291 B | 20,460 B | ~17,903 | 1 |
| Rust lexer DFA | 37,983 | 270,781 B | 160,064 B | ~75,965 | 1 |
| Rust parser packed ATN | 50,389 | 248,282 B | 118,680 B | ~100,777 | 1 |
| TOML lexer DFA | 19,512 | 187,080 B | 113,656 B | ~39,023 | 1 |
| TOML parser packed ATN | 2,472 | 11,486 B | 5,375 B | ~4,943 | 1 |

Whole generated modules (the lexer files also include the encoded serialized
lexer ATN in metadata; sidecar `decisions.json`/`semantics.json` manifests are
unchanged, so total artifact bytes move with the `.rs` files):

| Module | old bytes | new bytes | change |
| --- | ---: | ---: | ---: |
| `antlr_v4_lexer.rs` | 2,749,090 | 1,683,997 | -38.7% |
| `antlr_v4_parser.rs` | 405,146 | 384,343 | -5.1% |
| `rust_lexer.rs` | 347,986 | 213,529 | -38.6% |
| `rust_parser.rs` | 2,156,554 | 2,032,888 | -5.7% |
| `toml_lexer.rs` | 217,306 | 137,843 | -36.6% |
| `toml_parser.rs` | 127,720 | 121,881 | -4.6% |
| `kotlin_lexer.rs` (grammars-v4) | 921,517 | 566,068 | -38.6% |
| `kotlin_parser.rs` (grammars-v4) | 1,702,212 | 1,607,071 | -5.6% |

## Compile time and memory

Touch-based recompiles of each recognizer crate with warm dependencies,
alternating old/new per round. Peak RSS is the child high-water mark reported
by `getrusage`, measured in a fresh process per sample.

Debug `cargo check` (three rounds each; spreads shown):

| Crate | old wall | new wall | old peak RSS | new peak RSS |
| --- | ---: | ---: | ---: | ---: |
| antlr-rust-g4-parser | 0.99–1.01 s | 0.66–0.68 s | 289–293 MB | 198–199 MB |
| antlr-rust-rs-parser | 1.73–1.79 s | 1.68–1.69 s | 426–427 MB | 400 MB |
| antlr-rust-toml-parser | 0.39 s | 0.38–0.39 s | 134 MB | 129–130 MB |

Release `cargo build` (two rounds each):

| Crate | old wall | new wall | old peak RSS | new peak RSS |
| --- | ---: | ---: | ---: | ---: |
| antlr-rust-g4-parser | 6.90 s | 6.08–6.13 s | 458 MB | 360–361 MB |
| antlr-rust-rs-parser | 36.79–36.96 s | 36.67–36.68 s | 929–933 MB | 903–904 MB |
| antlr-rust-toml-parser | 4.98 s | 4.93–4.96 s | 379–380 MB | 370 MB |

## Binary and library size

Release `.rlib` sizes for the recognizer crates and the runtime (which gains
the decoder), plus the Kotlin parity dumper binary:

| Artifact | old bytes | new bytes | change |
| --- | ---: | ---: | ---: |
| `libantlr_rust_g4_parser.rlib` | 10,774,386 | 10,493,968 | -2.6% |
| `libantlr_rust_rs_parser.rlib` | 21,562,290 | 21,268,396 | -1.4% |
| `libantlr_rust_toml_parser.rlib` | 5,054,972 | 5,088,736 | +0.7% |
| `libantlr4_runtime.rlib` | 10,312,432 | 10,405,236 | +0.9% |
| `kotlin-parity-dumper` (binary) | 3,512,120 | 3,429,432 | -2.4% |

## First-use initialization

Blob decoding is a one-time cost inside the existing `OnceLock` first-use
points. Stage timings and a counting global allocator on the largest
checked-in artifact set (ANTLRv4, release build):

| Stage | wall | allocations | allocated bytes |
| --- | ---: | ---: | ---: |
| lexer DFA blob decode (401,141 words) | 3.8 ms | 2 | 2,791,678 |
| lexer DFA `from_serialized` (old path) | 2.9 ms | 10,610 | 2,290,248 |
| lexer DFA `from_encoded` (new path, total) | 6.3 ms | 10,612 | 5,081,926 |
| parser ATN `from_encoded` | 72 µs | 2 | 51,155 |
| serialized lexer ATN decode (5,325 values) | 24 µs | 2 | 28,181 |

Decoding adds exactly two transient allocations per artifact (the byte buffer
and the integer vector; both are dropped after construction, except the parser
ATN which retains its word vector — the old path borrowed rodata instead).

Process-level cold/warm probe, five interleaved rounds (cold is the first
parse in a fresh process including all decodes; warm is the median of 20
subsequent parses):

| Recognizer / input | old cold | new cold | old warm | new warm |
| --- | ---: | ---: | ---: | ---: |
| ANTLRv4 parser on `ANTLRv4Parser.g4` | 4.2–5.2 ms | 8.2–8.9 ms | 878–882 µs | 790–792 µs |
| TOML parser on this repo's `Cargo.toml` | 1.3–1.4 ms | 1.6–1.7 ms | 518–524 µs | 518–526 µs |

## Parse performance

Kotlin parity dumper (`--iters 5 --time`, parse-only stopwatch, three
interleaved rounds; min excludes the first in-process iteration, so it is the
steady-state number):

| Snippet | old min | new min |
| --- | ---: | ---: |
| 01-nested-types.kt | 0.200–0.206 ms | 0.198–0.200 ms |
| 02-dataframe.kt | 1.144–1.150 ms | 1.142–1.147 ms |
| 03-string-templates.kt | 0.598–0.601 ms | 0.605–0.612 ms |

All steady-state deltas are within ±2%. The warm ANTLRv4 probe above was
about 10% faster with the change; treat that as same-or-better rather than a
claimed speedup, since the regenerated recognizers also pick up generator
changes landed since the checked-in baseline.

## Equivalence evidence

- Full ANTLR runtime-testsuite sweep: 357 passed, 0 failed (twice: after the
  format change and after the table-driven base64 decoder).
- Kotlin parity harness: all snippet parse trees byte-identical to
  `antlr4-python3-runtime` with both trees.
- `tools/rust-syntax/update-generated.sh --check` and
  `tools/toml-syntax/update-generated.sh --check` pass, confirming
  deterministic regeneration; `tools/grammar-frontend/update-stage0.sh`
  proved the Stage 1 → Stage 2 fixed point for the self-hosted frontend.
- Corrupt, truncated, overflowing, overlong, and unsupported blob inputs fail
  with targeted `EncodedBlobError` diagnostics (snapshot-tested); the lexer
  DFA keeps its documented rebuild-from-ATN fallback.
