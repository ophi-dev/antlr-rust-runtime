//! Lightweight counters for lexer and prediction performance investigations.

#![allow(clippy::missing_const_for_thread_local)]

use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Debug, Default)]
struct Counters {
    adaptive_calls: u64,
    forced_full_context_calls: u64,
    full_context_retries: u64,
    sll_conflicts: u64,
    reach_sll_calls: u64,
    reach_full_context_calls: u64,
    reach_input_configs: u64,
    reach_output_configs: u64,
    reach_max_input_configs: u64,
    reach_max_output_configs: u64,
    closure_calls: u64,
    closure_visited_total: u64,
    closure_visited_max: u64,
    config_add_calls: u64,
    config_inserts: u64,
    config_merges: u64,
    config_max_size: u64,
    context_merge_calls: u64,
    context_merge_identical: u64,
    context_merge_cache_hits: u64,
    context_merge_cache_misses: u64,
    context_merge_uncached: u64,
    context_cache_calls: u64,
    context_cache_hits: u64,
    context_cache_misses: u64,
    context_cache_inserts: u64,
    dfa_edge_lookups: u64,
    dfa_edge_hits: u64,
    dfa_edge_misses: u64,
    dfa_atn_fallbacks: u64,
    dfa_states_created: u64,
    dfa_states_deduplicated: u64,
    dfa_fingerprint_candidates: u64,
    dfa_fingerprint_collisions: u64,
    dfa_cache_imports: u64,
    dfa_cache_import_nanos: u64,
    dfa_cache_import_states: u64,
    dfa_cache_publications: u64,
    dfa_cache_publication_nanos: u64,
    dfa_cache_publication_states: u64,
    lexer_direct_ascii_chars: u64,
    lexer_generic_chars: u64,
    lexer_scalar_replay_chars: u64,
    lexer_bulk_committed_chars: u64,
    decisions: BTreeMap<usize, DecisionCounters>,
}

#[derive(Debug, Default)]
struct DecisionCounters {
    adaptive_calls: u64,
    forced_full_context_calls: u64,
    full_context_retries: u64,
    sll_conflicts: u64,
}

thread_local! {
    static COUNTERS: RefCell<Counters> = RefCell::new(Counters::default());
}

fn with_counters(update: impl FnOnce(&mut Counters)) {
    COUNTERS.with(|counters| update(&mut counters.borrow_mut()));
}

fn add_len(total: &mut u64, max: &mut u64, len: usize) {
    let len = u64::try_from(len).unwrap_or(u64::MAX);
    *total = total.saturating_add(len);
    *max = (*max).max(len);
}

pub(crate) fn record_adaptive_call(decision: usize, forced_full_context: bool) {
    with_counters(|counters| {
        counters.adaptive_calls = counters.adaptive_calls.saturating_add(1);
        let decision_counters = counters.decisions.entry(decision).or_default();
        decision_counters.adaptive_calls = decision_counters.adaptive_calls.saturating_add(1);
        if forced_full_context {
            counters.forced_full_context_calls =
                counters.forced_full_context_calls.saturating_add(1);
            decision_counters.forced_full_context_calls = decision_counters
                .forced_full_context_calls
                .saturating_add(1);
        }
    });
}

pub(crate) fn record_full_context_retry(decision: usize) {
    with_counters(|counters| {
        counters.full_context_retries = counters.full_context_retries.saturating_add(1);
        let decision_counters = counters.decisions.entry(decision).or_default();
        decision_counters.full_context_retries =
            decision_counters.full_context_retries.saturating_add(1);
    });
}

pub(crate) fn record_sll_conflict(decision: usize) {
    with_counters(|counters| {
        counters.sll_conflicts = counters.sll_conflicts.saturating_add(1);
        let decision_counters = counters.decisions.entry(decision).or_default();
        decision_counters.sll_conflicts = decision_counters.sll_conflicts.saturating_add(1);
    });
}

pub(crate) fn record_reach_set(full_context: bool, input_configs: usize, output_configs: usize) {
    with_counters(|counters| {
        if full_context {
            counters.reach_full_context_calls = counters.reach_full_context_calls.saturating_add(1);
        } else {
            counters.reach_sll_calls = counters.reach_sll_calls.saturating_add(1);
        }
        add_len(
            &mut counters.reach_input_configs,
            &mut counters.reach_max_input_configs,
            input_configs,
        );
        add_len(
            &mut counters.reach_output_configs,
            &mut counters.reach_max_output_configs,
            output_configs,
        );
    });
}

pub(crate) fn record_closure(visited_configs: usize) {
    with_counters(|counters| {
        counters.closure_calls = counters.closure_calls.saturating_add(1);
        add_len(
            &mut counters.closure_visited_total,
            &mut counters.closure_visited_max,
            visited_configs,
        );
    });
}

pub(crate) fn record_config_add_call() {
    with_counters(|counters| {
        counters.config_add_calls = counters.config_add_calls.saturating_add(1);
    });
}

pub(crate) fn record_config_insert(size_after: usize) {
    with_counters(|counters| {
        counters.config_inserts = counters.config_inserts.saturating_add(1);
        counters.config_max_size = counters
            .config_max_size
            .max(u64::try_from(size_after).unwrap_or(u64::MAX));
    });
}

pub(crate) fn record_config_merge() {
    with_counters(|counters| {
        counters.config_merges = counters.config_merges.saturating_add(1);
    });
}

pub(crate) fn record_context_merge_call() {
    with_counters(|counters| {
        counters.context_merge_calls = counters.context_merge_calls.saturating_add(1);
    });
}

pub(crate) fn record_context_merge_identical() {
    with_counters(|counters| {
        counters.context_merge_identical = counters.context_merge_identical.saturating_add(1);
    });
}

pub(crate) fn record_context_merge_cache_hit() {
    with_counters(|counters| {
        counters.context_merge_cache_hits = counters.context_merge_cache_hits.saturating_add(1);
    });
}

pub(crate) fn record_context_merge_cache_miss() {
    with_counters(|counters| {
        counters.context_merge_cache_misses = counters.context_merge_cache_misses.saturating_add(1);
    });
}

pub(crate) fn record_context_merge_uncached() {
    with_counters(|counters| {
        counters.context_merge_uncached = counters.context_merge_uncached.saturating_add(1);
    });
}

pub(crate) fn record_context_cache_call() {
    with_counters(|counters| {
        counters.context_cache_calls = counters.context_cache_calls.saturating_add(1);
    });
}

pub(crate) fn record_context_cache_hit() {
    with_counters(|counters| {
        counters.context_cache_hits = counters.context_cache_hits.saturating_add(1);
    });
}

pub(crate) fn record_context_cache_miss() {
    with_counters(|counters| {
        counters.context_cache_misses = counters.context_cache_misses.saturating_add(1);
    });
}

pub(crate) fn record_context_cache_insert() {
    with_counters(|counters| {
        counters.context_cache_inserts = counters.context_cache_inserts.saturating_add(1);
    });
}

pub(crate) fn record_dfa_edge_lookup(hit: bool) {
    with_counters(|counters| {
        counters.dfa_edge_lookups = counters.dfa_edge_lookups.saturating_add(1);
        if hit {
            counters.dfa_edge_hits = counters.dfa_edge_hits.saturating_add(1);
        } else {
            counters.dfa_edge_misses = counters.dfa_edge_misses.saturating_add(1);
            counters.dfa_atn_fallbacks = counters.dfa_atn_fallbacks.saturating_add(1);
        }
    });
}

pub(crate) fn record_dfa_state_created() {
    with_counters(|counters| {
        counters.dfa_states_created = counters.dfa_states_created.saturating_add(1);
    });
}

pub(crate) fn record_dfa_state_deduplicated() {
    with_counters(|counters| {
        counters.dfa_states_deduplicated = counters.dfa_states_deduplicated.saturating_add(1);
    });
}

pub(crate) fn record_dfa_fingerprint_candidate() {
    with_counters(|counters| {
        counters.dfa_fingerprint_candidates = counters.dfa_fingerprint_candidates.saturating_add(1);
    });
}

pub(crate) fn record_dfa_fingerprint_collision() {
    with_counters(|counters| {
        counters.dfa_fingerprint_collisions = counters.dfa_fingerprint_collisions.saturating_add(1);
    });
}

pub(crate) fn record_dfa_cache_import(nanos: u128, states: usize) {
    with_counters(|counters| {
        counters.dfa_cache_imports = counters.dfa_cache_imports.saturating_add(1);
        counters.dfa_cache_import_nanos = counters
            .dfa_cache_import_nanos
            .saturating_add(u64::try_from(nanos).unwrap_or(u64::MAX));
        counters.dfa_cache_import_states = counters
            .dfa_cache_import_states
            .saturating_add(u64::try_from(states).unwrap_or(u64::MAX));
    });
}

pub(crate) fn record_dfa_cache_publication(nanos: u128, states: usize) {
    with_counters(|counters| {
        counters.dfa_cache_publications = counters.dfa_cache_publications.saturating_add(1);
        counters.dfa_cache_publication_nanos = counters
            .dfa_cache_publication_nanos
            .saturating_add(u64::try_from(nanos).unwrap_or(u64::MAX));
        counters.dfa_cache_publication_states = counters
            .dfa_cache_publication_states
            .saturating_add(u64::try_from(states).unwrap_or(u64::MAX));
    });
}

pub(crate) fn record_lexer_direct_ascii(count: usize) {
    with_counters(|counters| {
        counters.lexer_direct_ascii_chars = counters
            .lexer_direct_ascii_chars
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    });
}

pub(crate) fn record_lexer_generic_char() {
    with_counters(|counters| {
        counters.lexer_generic_chars = counters.lexer_generic_chars.saturating_add(1);
    });
}

pub(crate) fn record_lexer_scalar_replay(count: usize) {
    with_counters(|counters| {
        counters.lexer_scalar_replay_chars = counters
            .lexer_scalar_replay_chars
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    });
}

pub(crate) fn record_lexer_bulk_commit(count: usize) {
    with_counters(|counters| {
        counters.lexer_bulk_committed_chars = counters
            .lexer_bulk_committed_chars
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    });
}

pub fn reset() {
    COUNTERS.with(|counters| *counters.borrow_mut() = Counters::default());
}

pub fn dump() {
    COUNTERS.with(|counters| {
        let counters = counters.borrow();
        dump_totals(&counters);
        dump_decisions(&counters);
    });
}

fn dump_totals(counters: &Counters) {
    for (name, value) in totals(counters) {
        print_counter(name, value);
    }
}

fn dump_decisions(counters: &Counters) {
    for (decision, counters) in &counters.decisions {
        print_decision_counter(*decision, "adaptive_calls", counters.adaptive_calls);
        print_decision_counter(
            *decision,
            "forced_full_context_calls",
            counters.forced_full_context_calls,
        );
        print_decision_counter(
            *decision,
            "full_context_retries",
            counters.full_context_retries,
        );
        print_decision_counter(*decision, "sll_conflicts", counters.sll_conflicts);
    }
}

const fn totals(counters: &Counters) -> [(&'static str, u64); 44] {
    [
        ("prediction.adaptive_calls", counters.adaptive_calls),
        (
            "prediction.forced_full_context_calls",
            counters.forced_full_context_calls,
        ),
        (
            "prediction.full_context_retries",
            counters.full_context_retries,
        ),
        ("prediction.sll_conflicts", counters.sll_conflicts),
        ("reach.sll_calls", counters.reach_sll_calls),
        (
            "reach.full_context_calls",
            counters.reach_full_context_calls,
        ),
        ("reach.input_configs", counters.reach_input_configs),
        ("reach.output_configs", counters.reach_output_configs),
        ("reach.max_input_configs", counters.reach_max_input_configs),
        (
            "reach.max_output_configs",
            counters.reach_max_output_configs,
        ),
        ("closure.calls", counters.closure_calls),
        ("closure.visited_total", counters.closure_visited_total),
        ("closure.visited_max", counters.closure_visited_max),
        ("config.add_calls", counters.config_add_calls),
        ("config.inserts", counters.config_inserts),
        ("config.merges", counters.config_merges),
        ("config.max_size", counters.config_max_size),
        ("context_merge.calls", counters.context_merge_calls),
        ("context_merge.identical", counters.context_merge_identical),
        (
            "context_merge.cache_hits",
            counters.context_merge_cache_hits,
        ),
        (
            "context_merge.cache_misses",
            counters.context_merge_cache_misses,
        ),
        ("context_merge.uncached", counters.context_merge_uncached),
        ("context_cache.calls", counters.context_cache_calls),
        ("context_cache.hits", counters.context_cache_hits),
        ("context_cache.misses", counters.context_cache_misses),
        ("context_cache.inserts", counters.context_cache_inserts),
        ("dfa.edge_lookups", counters.dfa_edge_lookups),
        ("dfa.warm_hits", counters.dfa_edge_hits),
        ("dfa.warm_misses", counters.dfa_edge_misses),
        ("dfa.atn_fallbacks", counters.dfa_atn_fallbacks),
        ("dfa.states_created", counters.dfa_states_created),
        ("dfa.states_deduplicated", counters.dfa_states_deduplicated),
        (
            "dfa.fingerprint_candidates",
            counters.dfa_fingerprint_candidates,
        ),
        (
            "dfa.fingerprint_collisions",
            counters.dfa_fingerprint_collisions,
        ),
        ("dfa_cache.imports", counters.dfa_cache_imports),
        ("dfa_cache.import_nanos", counters.dfa_cache_import_nanos),
        ("dfa_cache.import_states", counters.dfa_cache_import_states),
        ("dfa_cache.publications", counters.dfa_cache_publications),
        (
            "dfa_cache.publication_nanos",
            counters.dfa_cache_publication_nanos,
        ),
        (
            "dfa_cache.publication_states",
            counters.dfa_cache_publication_states,
        ),
        (
            "lexer.direct_ascii_chars",
            counters.lexer_direct_ascii_chars,
        ),
        ("lexer.generic_chars", counters.lexer_generic_chars),
        (
            "lexer.scalar_replay_chars",
            counters.lexer_scalar_replay_chars,
        ),
        (
            "lexer.bulk_committed_chars",
            counters.lexer_bulk_committed_chars,
        ),
    ]
}

#[cfg(test)]
pub(crate) fn lexer_snapshot() -> [u64; 4] {
    COUNTERS.with(|counters| {
        let counters = counters.borrow();
        [
            counters.lexer_direct_ascii_chars,
            counters.lexer_generic_chars,
            counters.lexer_scalar_replay_chars,
            counters.lexer_bulk_committed_chars,
        ]
    })
}

fn print_counter(name: &str, value: u64) {
    #[allow(clippy::print_stderr)]
    {
        eprintln!("perf {name}={value}");
    }
}

fn print_decision_counter(decision: usize, name: &str, value: u64) {
    #[allow(clippy::print_stderr)]
    {
        eprintln!("perf decision.{decision}.{name}={value}");
    }
}
