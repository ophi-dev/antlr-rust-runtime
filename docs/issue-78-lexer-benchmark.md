# Issue 78 lexer fast-path benchmark record

Measurements were taken on 2026-07-17 on an Apple M3 Pro with Rust 1.96.0.
The baseline was `origin/main` at `73f33a407`; generated lexers and parsers
were rebuilt with the matching generator/runtime for each revision. The
grammars-v4 checkout was
`284602b3f23ca54dc30778204ab7ae9e969145e9`.

## Current-main finding

Ahead-of-time DFA compilation already removes ATN closure, hashing, and config
allocation from ordinary lexer matching. Issue #78 still applied after that
work in three places:

- each compiled-DFA symbol read still changed the shared cursor with
  `seek(position)` followed by `la(1)`;
- each accepted or recovered span was replayed through `consume_char()` to
  rebuild line and column;
- position queries and accept rewinds still used higher-level text or stream
  operations.

The scalar change therefore keeps the compiled DFA and lexer lifecycle model
intact. It adds optional immutable-access and position-summary methods to
`CharStream`, specializes the compiled ASCII walk, and centralizes accepted
position commits. Streams that do not implement the optional methods retain
the original scalar fallback.

## Lex-only results

The four configurations were built with the lex-only benchmark runner:

1. `main`;
2. the scalar fast paths with the ordinary release profile;
3. the scalar fast paths plus `-C target-cpu=native`;
4. the scalar fast paths plus ThinLTO and one codegen unit.

After all builds completed, the already-built runners were measured in eight
rotating, alternating process rounds per fixture. Each process used 20 warmups
and 100 timed lexes. The table reports the median process average across all 19
fixtures; ratios below one are faster.

| Configuration | Geometric ratio vs main | Aggregate ratio vs main |
|---|---:|---:|
| scalar release | 0.8853x | 0.8642x |
| scalar + native CPU | 0.8772x | 0.8671x |
| scalar + ThinLTO / one codegen unit | 0.7725x | 0.7450x |

Native CPU tuning was effectively neutral relative to the ordinary scalar
build (`0.9908x` geometric, `1.0034x` aggregate). ThinLTO and one codegen unit
improved on the ordinary scalar build by a further `0.8725x` geometric and
`0.8621x` aggregate.

The ordinary scalar build produced these per-language geometric ratios:

| Fixtures | Count | Scalar vs main |
|---|---:|---:|
| Kotlin, including two lexer stress fixtures | 6 | 0.9173x |
| C# | 4 | 0.8505x |
| Java | 4 | 0.8598x |
| Trino SQL | 5 | 0.8970x |

Every source-derived fixture improved. The short Unicode stress fixture had
overlapping 35-41 microsecond samples in the broad run, so it was repeated in
15 alternating process pairs with 100 warmups and 5,000 timed lexes. Its
median process average was 34.1 microseconds for the scalar build and 34.8
microseconds for main (`0.9805x`). The ASCII stress fixture measured `0.8872x`
in the broad run.

## End-to-end parse results

The same ordinary baseline and scalar binaries were measured over the 17
source-derived fixtures in six alternating process rounds, each with 5
warmups and 20 timed parses.

| Fixtures | Scalar vs main |
|---|---:|
| Kotlin | 0.9977x |
| C# | 0.9946x |
| Java | 1.0025x |
| Trino SQL | 0.9900x |
| **Geometric mean** | **0.9958x** |
| **Aggregate time** | **0.9961x** |

The largest fixture ratio was `1.0182x` on the Java Trino filter fixture, so
all 17 fixtures remained within the 2% regression threshold.

## Fast-path counters

A three-iteration instrumentation run demonstrated that the two synthetic
fixtures use the intended paths:

| Fixture | Direct ASCII reads | Generic reads | Scalar replay | Bulk committed |
|---|---:|---:|---:|---:|
| ASCII stress | 6,057 | 0 | 0 | 5,193 |
| Unicode fallback | 0 | 2,181 | 0 | 1,845 |

The Unicode stream remains indexed by scalar value. The generic count records
immutable scalar lookups; it does not indicate cursor mutation or replay.
