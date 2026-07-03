# Issue #15 — closing the Rust vs Go C# gap: profile-based reframing and plan

Date: 2026-07-02. Method: macOS `sample` over the warm `parse-bench` rust-runner
(6,300+ samples per fixture), inclusive attribution per subsystem (recursion
deduplicated). Baselines: C# `dotnet-wpf-datagrid-column.cs` 51 ms vs Go 19.4 ms;
Trino `tpcds-q47.sql` 1.6 ms vs Go 0.83 ms.

## TL;DR

The ATN prediction/closure machinery — issue #15's declared frontier — is only
~21% of C# wall time (`closure` itself 8%). The dominant costs are the **lexer
(36%)** and the **`sync_decision` error-sync helper (21%)**, both doing per-char
or per-decision work Go never does on the happy path. The prior campaign could
not see this: the `perf-counters` feature only instruments prediction internals.

## Measured wall-time budget (warm parses)

| Subsystem | C# `wpf` (51 ms; Go 19.4 ms) | Trino `q47` (1.6 ms; Go 0.83 ms) |
|---|---:|---:|
| Lexer (`next_token_with_cache`) | **36.0%** | **33.7%** |
| `adaptive_predict` total | 21.2% | 31.2% |
| — of which `closure` | 8.1% | 10.2% |
| `sync_decision` total | **20.9%** | **15.3%** |
| — of which expected-token stack walk | 7.9% | 13.4% |
| Parser-DFA clone-in (`new_shared`) | 2.6% | **10.7%** |
| Parse-tree drop | 2.2% | 1.0% |

Arithmetic check: Go-like lexing (~4 ms) plus O(1) sync takes wpf from 51 ms to
~27 ms before touching prediction. Trino reaches rough parity. The remaining
~7 ms C# delta is the genuine prediction-layer gap where the issue's existing
closure/merge analysis applies — as phase 4, not phase 1.

Why the language pattern looked paradoxical: Rust wins Kotlin 1.5–1.9× because
Kotlin makes Go's *parser* slow (prediction-heavy), masking our lexer/sync
overheads. C#/Trino are cheap for Go's parser, so our fixed per-char and
per-decision overheads dominate; tiny Trino files additionally expose a fixed
~0.2 ms/parse parser-DFA clone tax.

## Root causes

### RC1 — lexer re-learns its DFA every parse and traces every character

- The learned lexer DFA lives in `BaseLexer.lexer_dfa` (`src/lexer.rs:115`),
  **per instance** — a fresh lexer per parse rebuilds the whole DFA via ATN
  simulation (`close_config`/`epsilon_closure` appear throughout warm
  iterations in the profile).
- The warm hot loop (`src/atn/lexer.rs:524-531`) calls `record_lexer_dfa_edge`
  — a `BTreeSet<LexerDfaEdge>` insert, the hottest non-malloc frame in the
  profile — **on every input character even on cache hits**, purely to support
  the runtime-testsuite's `showDFA` output (`lexer_dfa_string`, consumed only
  by `src/bin/antlr4-runtime-testsuite.rs`).
- Edge lookup is an `FxHashMap<(usize, i32)>` probe per char; Go indexes a
  per-state array (`edges[t-MinDFAEdge]`) on a DFA shared statically across
  lexer instances.

### RC2 — `sync_decision` walks the whole rule stack on every nullable exit

Generated parsers call `sync_decision` (`src/parser.rs:3282`) before
optional/loop decisions. When the decision is nullable and the token does not
start an alternative — the completely normal "exit the `*` loop" case —
`src/parser.rs:3320` computes the full context-expected token set by
recursively walking every rule-stack frame and unioning per-state bitsets.
Java/Go's `DefaultErrorStrategy.sync` returns immediately on nullable
decisions via memoized `atn.nextTokens(s)` — O(1). C#'s deep expression
nesting (15+ frames per expression) makes the O(depth) walk brutal. The
per-state caches feeding it (`state_expected_token_cache`,
`rule_stop_reach_cache`, `src/parser.rs:6526-6551`) are also per-instance.

### RC3 — parser DFA is cloned in and cloned back per parser instance

`ParserAtnSimulator::new_shared` (`src/atn/parser.rs:266`) deep-clones the
warm shared DFA vector out of a thread-local on every parser construction;
`Drop` clones dirty DFAs back (`merge_shared_decision_dfas`,
`src/atn/parser.rs:210`). Go/Java mutate one shared DFA in place — zero
copies. Cost ∝ warm-DFA size: 2.6% on a 51 ms C# parse, 10.7% on a 1.6 ms
Trino parse.

### RC4 — prediction-layer gap is real but smaller than framed

Verified code-level: our merge keeps equal-return-state entries as separate
array entries (`merge_two_context_entries`, `src/prediction.rs:315-350`)
where Go recursively merges parents; Go's `closureBusy` spans a whole
`computeReachSet` across all seeds but is consulted at only two points
(`closureWork`), while our `scratch.visited` is per-seed and gates every pop.
Two hypotheses killed by reading Go v4.13.1: its merge cache is *also*
per-prediction (`p.mergeCache = nil` after each predict), and we already have
the shared-context-cache/`optimizeConfigs` equivalent. Within the ~21% slice,
issue #15's closure-driver analysis stands, including its discarded-paths list.

## Plan

Every phase gates on: `cargo test --locked`, clippy `-D warnings`, 357/357
conformance, kotlin-parity byte-identical, interleaved A/B across **all**
fixtures (Kotlin/Java within noise). Phases 1–3 strictly remove work for every
language, so Kotlin should improve, not regress.

### Phase 1 — lexer (biggest lever, ~30%+ of C# time)

1. Make edge-trace recording opt-in: `record_dfa_trace: bool` (default off) on
   `BaseLexer`; skip `record_lexer_dfa_edge` at `src/atn/lexer.rs:529`/`:580`
   unless set. Only the testsuite `showDFA` template opts in.
2. Share the learned lexer DFA across instances: thread-local keyed by ATN
   pointer holding `Rc<RefCell<...>>` with the mutable learned state
   (`state_numbers`, `cached_states`, `transitions`, `mode_starts`) moved out
   of the per-instance trace. Keep the trace-only `edges` per instance
   (showDFA expects per-run observations).
3. Dense per-state edge arrays for ASCII (Go's `MinDFAEdge..MaxDFAEdge`
   scheme), hashmap fallback above. Re-profile after 1–2 first.

### Phase 2 — `sync_decision` O(1) happy path (~15–20%)

1. Replace the eager full-union build (`src/parser.rs:3320`) with an
   early-exit membership walk over rule-stack frames (outermost-in) against
   cached per-state bitsets; build the full union only on the actual
   mismatch/deletion/error path. Same semantics — membership in the union
   equals membership in any member.
2. Walk the stack directly instead of materializing a `PredictionContext`.
3. Hoist `state_expected_token_cache` / `rule_stop_reach_cache` into
   `with_shared_atn_caches` (pattern already used by
   `cached_decision_lookahead`, `src/parser.rs:6601`).

### Phase 3 — parser DFA shared in place (~3% C#, ~11% Trino)

Thread-local `Rc<RefCell<Vec<Dfa>>>`; simulator keeps the `Rc`, short
`borrow_mut`s at existing mutation points. Delete `merge_shared_decision_dfas`,
the `Drop` impl, dirty-tracking. Debug-assert non-reentrancy. Also fixes the
stale-instance discard wart at `src/atn/parser.rs:222-229`.

### Phase 4 — prediction layer (original frontier, correctly sized)

Only after 1–3 land and a fresh profile shows `adaptive_predict` dominant:

1. Differential instrumentation: patch the vendored Go runtime with counters
   equivalent to ours (closure invocations, config adds, merges, DFA states,
   LL retries); compare per fixture. Decides algorithmic vs per-op cost.
2. If per-op: `ClosureConfigKey` clones `SemanticContext` per pop
   (`src/atn/parser.rs:116`), config clones at `:1083`/`:1095`; consider
   `Rc`-ing `SemanticContext`.
3. If algorithmic: test Go's busy-set scope (one set per `compute_reach_set`
   spanning all seeds, consulted only at Go's two points), measured against
   the mojang explosion with the recursive collapse temporarily enabled.
   Issue #15's discarded-paths list stays binding.
4. Add wall-time bucket counters (lex/sync/predict/other) behind
   `perf-counters` so future campaigns see the subsystem split.

## Validation additions

Run the profile-attribution script before/after each phase so claimed savings
are verified against the subsystem budget, not just end-to-end ms.
