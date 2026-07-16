# Issue 86 parser-DFA benchmark record

Measurements were taken on 2026-07-16 on an Apple M3 Pro. The baseline was
`origin/main` at `16e904796`; generated parsers were rebuilt with the matching
generator/runtime for each revision.

## Protected parser suites

The Kotlin, C#, Java, and Trino suite used 5 warmups and 20 measured parses for
each of 17 fixtures. Compared with the baseline, the compact DFA was 2.39%
faster by geometric mean and 2.51% faster by aggregate time. It improved 16 of
17 fixtures; the remaining Java fixture was 0.17% slower. The repository's 2%
regression check passed for every fixture.

An untimed instrumentation parse of each fixture produced these aggregate
store measurements:

| Measurement | Value |
|---|---:|
| states | 20,396 |
| transitions | 24,355 |
| empty rows | 2,766 |
| one-edge inline rows | 13,862 |
| rows with 2-3 edges | 3,201 |
| rows with 4-7 edges | 461 |
| rows with 8-15 edges | 85 |
| rows with at least 16 edges | 21 |
| dense rows | 4 |
| fingerprint candidates checked | 4,725 |
| structural fingerprint collisions | 0 |
| retained hot bytes | 1,253,968 (61.5/state) |
| retained cold bytes | 157,249,424 (7,709.8/state) |

All fixture vocabulary rows were 131-342 slots wide. Of the states, 18,150
had width at most 256 and 2,246 had width at most 512. Cache import took 0.280
ms total across 17 processes; publication took 0.029 ms for 20,396 states.

## Dense threshold selection

Three policies were measured over the same fixtures with 10 iterations:

| Policy versus selected hybrid | Geometric time | Aggregate time | Wins |
|---|---:|---:|---:|
| sparse only | 1.0055x | 1.0073x | 5/17 |
| aggressive dense promotion | 1.0402x | 1.0306x | 0/17 |
| selected hybrid | 1.0000x | 1.0000x | 17/17 |

The selected policy keeps empty and one-edge rows allocation-free, and promotes
a sparse row only when its vocabulary width is at most 512, it contains at
least 8 edges, and its density is at least 12.5%.

## Solidity and Go

The issue #76 Solidity fixtures used five independently warmed processes with
80 measured parses per process. The table reports the median of those process
medians. Go used `antlr4-go/antlr` v4.13.1 and the same ANTLR 4.13.2 grammar.

| Fixture | Bytes | Current Rust | Main Rust | Go | Rust over Go |
|---|---:|---:|---:|---:|---:|
| Ownable | 3,102 | 241 us | 241 us | 718 us | 2.98x |
| ERC20 | 10,800 | 699 us | 695 us | 2,096 us | 3.00x |
| ERC721 | 16,201 | 1,075 us | 1,065 us | 4,034 us | 3.75x |
| Governor | 32,182 | 1,837 us | 1,827 us | 6,826 us | 3.72x |

The largest current-versus-main difference was 0.94%. Repeating Governor to
64, 129, and 257 KB produced Rust median ranges of 3.62-3.68 ms, 7.24-7.68 ms,
and 14.55-14.61 ms respectively. Go took 13.66-14.32 ms, 27.27-27.61 ms, and
55.11-55.56 ms. Current and main produced identical parse-tree hashes for all
four source fixtures.

The counting allocator measured one cold full-tree parse:

| Fixture | Current allocated | Main allocated | Change | Current peak | Main peak | Change |
|---|---:|---:|---:|---:|---:|---:|
| Ownable | 4,876,494 | 5,294,082 | -7.9% | 1,795,172 | 2,178,804 | -17.6% |
| ERC20 | 8,066,082 | 8,596,250 | -6.2% | 1,954,632 | 2,436,324 | -19.8% |
| ERC721 | 10,474,874 | 11,077,706 | -5.4% | 2,128,788 | 2,665,308 | -20.1% |
| Governor | 15,388,610 | 16,462,362 | -6.5% | 2,574,336 | 3,541,292 | -27.3% |

These are whole-parse allocation totals, measured with otherwise identical
generated parsers and harnesses; the branch difference is the parser-DFA
storage change.
