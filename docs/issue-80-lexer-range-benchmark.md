# Issue 80 ASCII range-scan benchmark record

Measurements were taken on 2026-07-19 on an Apple M3 Pro with Rust 1.96.0.
The baseline was `origin/main` at `5c7affddb`; generated lexers were rebuilt
with the matching generator/runtime for each revision. The grammars-v4
checkout was `284602b3f23ca54dc30778204ab7ae9e969145e9`.

## Retained scope

The compiled lexer DFA now represents an exact ASCII self-loop set as up to
four canonical, coalesced inclusive ranges when the existing `Until1` through
`Until3` forms do not apply. Unsupported shapes keep the ordinary DFA walk.
The retained scanner is a portable scalar implementation. Range descriptors
also participate in compiled-DFA serialization and malformed-stream
validation.

The final implementation contains no architecture-specific code or `unsafe`.
AVX2 and NEON candidates were developed and tested, then removed under issue
#80's decision gate because the measured backends did not justify their
dispatch, threshold, and maintenance costs.

## Scalar results

The Kotlin, C#, Java, and Trino lex-only suite used 20 warmups and 200 measured
lexes for each fixture. Ratios below one are faster.

| Fixtures | Count | Scalar ranges vs main |
|---|---:|---:|
| Kotlin | 6 | 0.9843x |
| C# | 4 | 0.9702x |
| Java | 4 | 0.9610x |
| Trino SQL | 5 | 0.9969x |
| **All shared fixtures** | **19** | **0.9796x** |

The aggregate-time ratio was `0.9854x`. The largest per-fixture ratio was
`1.0275x` on the short Unicode fallback fixture, below the repository's
`1.15x` regression guard.

Two Java fixtures isolate the range-heavy paths. Already-built baseline and
current runners were measured with 100 warmups and 2,000 timed lexes:

| Fixture | Main | Scalar ranges | Ratio |
|---|---:|---:|---:|
| Long identifiers and mixed ASCII ranges | 23.865 us | 21.411 us | 0.8972x |
| Long numbers and whitespace | 20.077 us | 17.196 us | 0.8565x |

## Architecture-backend decision

Before removal, the same generated runner was built with a forced scalar
scanner, normal AArch64 runtime dispatch with a 32-byte NEON threshold, and a
64-byte NEON threshold. Each process used 200 warmups and 5,000 timed lexes.
The table reports candidate/scalar ratios, so values above one are slower.

| Fixture | NEON at 32 bytes | NEON at 64 bytes |
|---|---:|---:|
| Long identifiers and mixed ASCII ranges | 1.0010x | 1.0196x |
| Long numbers and whitespace | 1.0200x | 1.0061x |
| Bazel Java | 1.0088x | 1.0129x |
| Trino Java | 1.0215x | 1.0312x |

Neither threshold improved a representative real fixture. The AVX2 candidate
cross-built and passed the x86_64 test suite under Rosetta, but that
environment does not provide representative native AVX2 benchmark evidence.
Shipping either backend would therefore fail the issue's decision gate.

## Diagnostics

An untimed `perf-counters` run over the long-identifier fixture generated 37
range descriptors: 32 one-range, one two-range, three three-range, and one
four-range descriptor. One lex recorded 43 scalar range scans and 705 skipped
bytes, attributed as 360 identifier, 288 number, and 57 whitespace bytes.
The small inventory did not justify a pooled descriptor indirection.

Correctness validation included randomized descriptor/scanner differential
tests, accelerated-versus-ordinary DFA token-stream comparisons, malformed
serialization tests, all nine Kotlin parity snippets, and the rendered ANTLR
runtime testsuite (`357 passed, 0 failed, 0 skipped`).
