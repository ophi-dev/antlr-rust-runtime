# Issue 87 packed parser-ATN benchmark record

Measurements were taken on 2026-07-17 on an Apple M3 Pro. The baseline was
`origin/main` at `e01e5e8c5`; generated parsers were rebuilt with the matching
generator/runtime for each revision. The grammars-v4 checkout was
`284602b3f23ca54dc30778204ab7ae9e969145e9`.

## Protected parser suites

The Kotlin, C#, Java, and Trino suite used 10 warmups and 50 measured
end-to-end parses for each of 17 fixtures. Compared with the object-graph
baseline, the packed ATN was 0.86% faster by geometric mean and 3.65% faster by
aggregate time.

Short-fixture ratios near the 2% limit were checked with interleaved processes
and higher iteration counts. Trino q47 measured `1.0120x`, the Java filter
fixture `0.9969x`, and C# anonymous types `0.9905x`. No confirmed fixture
regressed by more than 2%.

DFA-cold performance used seven interleaved process pairs per fixture, with no
warmups and one timed parse in each fresh process. The geometric ratio was
`0.9919x` and the aggregate ratio was `0.9965x`. The initial seven-pair Java
Mojang sample measured `1.0217x`; expanding it to 31 pairs stabilized at
`1.0086x`.

## Construction and resident storage

A counting global allocator was reset immediately before the first parser-ATN
initialization in a fresh process. Allocation bytes are cumulative requested
bytes, including temporary construction storage. Baseline resident bytes are
the serialized static stream plus retained object-graph heap; packed resident
bytes are the validated static word stream, with no retained parser-ATN heap.

| Grammar | States | Transitions | Baseline allocations | Baseline allocated | Baseline resident | Packed allocations | Packed resident | Resident change |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Kotlin | 2,854 | 3,582 | 3,037 | 1,224,384 | 937,496 | 0 | 155,188 | -83.4% |
| C# | 2,629 | 3,504 | 2,931 | 1,241,648 | 904,648 | 0 | 147,804 | -83.7% |
| Java | 1,841 | 2,461 | 2,038 | 713,616 | 561,500 | 0 | 103,364 | -81.6% |
| Trino | 3,278 | 4,404 | 3,696 | 1,394,152 | 1,047,020 | 0 | 184,088 | -82.4% |
| **Total** | **10,602** | **13,951** | **11,702** | **4,573,800** | **3,450,664** | **0** | **590,444** | **-82.9%** |

Validation time was the median of 21 interleaved fresh-process pairs:

| Grammar | Baseline construction | Packed validation | Ratio |
|---|---:|---:|---:|
| Kotlin | 159.2 us | 41.5 us | 0.261x |
| C# | 171.5 us | 43.5 us | 0.254x |
| Java | 121.8 us | 31.0 us | 0.255x |
| Trino | 200.4 us | 57.6 us | 0.287x |

## Artifact size

Packing stores more precomputed information in the generated artifact so the
runtime does not rebuild links, transition vectors, or interval sets:

| Measurement | Baseline | Packed | Change |
|---|---:|---:|---:|
| Static parser-ATN data | 391,008 | 590,444 | +51.0% |
| Generated parser source | 8,123,871 | 8,455,102 | +4.1% |
| Four-grammar benchmark executable | 8,467,424 | 8,668,352 | +2.4% |

The static-data increase is offset by removing 3,059,656 retained heap bytes
and all 11,702 cold construction allocation operations across the four
grammars.
