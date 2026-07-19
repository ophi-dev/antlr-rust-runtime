# Issue 81 adaptive parser token-set benchmark record

Measurements were taken on 2026-07-19 on an Apple M3 Pro with Rust 1.96.0.
The baseline was `main` at `2b826537d`; each runner was generated and built
from its matching generator/runtime. The grammars-v4 checkout was
`284602b3f23ca54dc30778204ab7ae9e969145e9`.

## Applicable scope

Current `main` already uses a dense `TokenBitSet` for parser FIRST/lookahead
caches, and parser ATNs are immutable packed word streams. The remaining hot
set probes were:

- packed `Set` and `NotSet` transition matching;
- generated dense set/not-set recognition; and
- generated leading-lookahead guards for those dense sets.

The change attaches an adaptive lookup representation to each packed parser
interval set. Sets use two inline words through token type 127, a bounded dense
table when its cost and density justify one, or the existing normalized
interval search. Lexer Unicode sets are unchanged. Dense payloads have a hard
64 KiB per-set limit. Generated direct recognition keeps literal interval
checks for inline and sparse sets; routing every small generated set through
packed metadata was not supported by the isolated parser measurements below.

## End-to-end parse results

The ordinary release runners used 10 warmups and 50 measured end-to-end parses
for each of 17 fixtures. Compiler-level options were disabled so these results
isolate the token-set representation from native CPU tuning, ThinLTO, and PGO.
The benchmark harness exposes each of those modes independently, including
separate PGO profile-generation and profile-use builds. Ratios below one are
faster.

| Fixtures | Count | Geometric ratio vs main | Aggregate ratio vs main |
|---|---:|---:|---:|
| Kotlin | 4 | 1.0120x | 0.9915x |
| C# | 4 | 0.9705x | 0.9716x |
| Java | 4 | 0.9752x | 0.9940x |
| Trino SQL | 5 | 0.9506x | 0.9391x |
| **All** | **17** | **0.9753x** | **0.9875x** |

The single-pass Kotlin coroutines result was noisy at `1.0959x`. Seven
alternating process pairs, each with 5 warmups and 20 measured parses,
produced a `0.9475x` median ratio. The other one-pass ratios above one were C#
codegen (`1.0063x`) and Bazel Java (`1.0257x`); their paired medians were
`0.9815x` and `0.9884x`. No fixture had a confirmed regression.

## Parser-only Kotlin results

The Kotlin parity dumper eagerly buffers tokens before starting its parser
stopwatch. Seven alternating process pairs ran 100 in-process parses per
fixture with ordinary release settings:

| Fixture | Median paired ratio vs main | Pair range |
|---|---:|---:|
| Kotlin lazy bodies | 0.9964x | 0.9933x..1.0372x |
| Coroutines flow limit | 1.0019x | 0.9911x..1.0115x |
| Ktor describe route | 1.0046x | 0.9979x..1.0117x |
| Ktor security inference | 1.0128x | 0.9745x..1.0205x |
| **Geometric ratio** | **1.0039x** | |

Every range crossed `1.0`; the isolated parser result is effectively flat.
An earlier experiment routed every generated inline set through packed
metadata and was rejected. The final policy keeps the indexed path for dense
generated sets, where it replaces longer interval checks, while packed ATN
prediction uses the selected representation for every set.

## Representation counters

Each row is one instrumented parse. "Bitset probes" is the combined inline and
dense count; those probes replace interval membership. Interval probes retain
the normalized binary search.

| Grammar fixture | Inline sets | Dense sets | Interval sets | Bitset bytes | Bitset probes | Interval probes |
|---|---:|---:|---:|---:|---:|---:|
| Kotlin coroutines | 21 | 1 | 3 | 360 | 17,846 | 710 |
| C# codegen | 14 | 0 | 5 | 224 | 1,870 | 1,112 |
| Java Closure | 13 | 2 | 0 | 256 | 641 | 0 |
| Trino TPCH q21 | 5 | 3 | 29 | 200 | 13 | 22 |

The Trino distribution demonstrates the sparse fallback: most of its sets stay
as intervals, while dense or small sets still receive indexed membership.

## Packed table size

The original normalized intervals remain available for deterministic
diagnostics and iteration. The delta therefore includes three metadata words
per set plus the selected bitset payload.

| Grammar | Main packed ATN | Adaptive packed ATN | Bitset payload | Total change |
|---|---:|---:|---:|---:|
| Kotlin | 155,188 B | 155,860 B | 360 B | +672 B (+0.43%) |
| C# | 147,804 B | 148,268 B | 224 B | +464 B (+0.31%) |
| Java | 103,364 B | 103,812 B | 256 B | +448 B (+0.43%) |
| Trino | 184,088 B | 184,744 B | 200 B | +656 B (+0.36%) |
| **Total** | **590,444 B** | **592,684 B** | **1,040 B** | **+2,240 B (+0.38%)** |

Focused tests compare adaptive membership with normalized intervals over
randomized sets and values, exercise EOF and negative values, verify exact
`NotSet` vocabulary bounds, validate the legacy packed format, reject
inconsistent packed bit tables, and lock the 64 KiB dense boundary.
