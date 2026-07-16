use crate::atn::{Atn, AtnState, AtnStateKind, Transition};
use crate::dfa::{
    DfaStateBuilder, DfaStateId, NO_DFA_STATE, ParserDfa, ParserDfaStateView, ParserDfaStats,
};
use crate::int_stream::IntStream;
use crate::prediction::{
    AtnConfig, AtnConfigSet, ContextArena, ContextId, EMPTY_CONTEXT, EMPTY_RETURN_STATE,
    PredictionContextStats, PredictionFxHasher, PredictionWorkspace, SemanticContext,
    all_subsets_conflict, all_subsets_equal, conflicting_alt_subsets,
    has_sll_conflict_terminating_prediction, single_viable_alt,
};
use crate::token::TOKEN_EOF;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;

type FxHashSet<T> = HashSet<T, BuildHasherDefault<PredictionFxHasher>>;

#[derive(Debug)]
pub struct ParserAtnSimulator<'a> {
    atn: &'a Atn,
    store: PredictionStore,
    workspace: PredictionWorkspace,
    outer_context_cache: Option<CachedOuterContext>,
    outer_context_cache_hits: usize,
    outer_context_cache_misses: usize,
    shared_cache_key: Option<usize>,
    /// Java's `LL_EXACT_AMBIG_DETECTION`: the full-context loop keeps
    /// consuming past "resolves to one viable alt" conflicts until every
    /// `(state, context)` subset conflicts over the same alt set.
    exact_ambig_detection: bool,
}

#[derive(Clone, Copy, Debug)]
struct CachedOuterContext {
    rule_context_version: usize,
    context: ContextId,
}

#[derive(Debug, Default)]
struct PredictionStore {
    contexts: ContextArena,
    decision_to_dfa: Vec<ParserDfa>,
}

impl PredictionStore {
    fn new(atn: &Atn) -> Self {
        Self {
            contexts: ContextArena::new(),
            decision_to_dfa: initial_decision_dfas(atn),
        }
    }
}

thread_local! {
    static SHARED_PREDICTION_STORES: RefCell<HashMap<usize, PredictionStore>> =
        RefCell::new(HashMap::new());
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParserAtnPrediction {
    pub alt: usize,
    pub requires_full_context: bool,
    pub has_semantic_context: bool,
    pub diagnostic: Option<ParserAtnPredictionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParserAtnPredictionDiagnostic {
    pub kind: ParserAtnPredictionDiagnosticKind,
    pub start_index: usize,
    pub sll_stop_index: usize,
    pub ll_stop_index: usize,
    pub conflicting_alts: Vec<usize>,
    /// For [`ParserAtnPredictionDiagnosticKind::Ambiguity`]: whether the
    /// full-context loop proved an exact ambiguity (Java's `exact` flag —
    /// the default `DiagnosticErrorListener` only reports exact ones).
    pub exact: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParserAtnPredictionDiagnosticKind {
    Ambiguity,
    ContextSensitivity,
}

#[derive(Clone, Copy)]
struct PredictionCheck {
    decision: usize,
    decision_state: usize,
    state_number: DfaStateId,
    start_index: usize,
    precedence: i32,
    outer_context: ContextId,
    force_full_context_retry: bool,
    sll_probe_only: bool,
}

#[derive(Clone, Copy)]
struct AdaptivePredictRequest {
    decision: usize,
    precedence: usize,
    outer_context: ContextId,
    force_full_context_retry: bool,
    /// When set, the SLL walk stops at the first full-context-requiring conflict
    /// and returns the SLL prediction (carrying `requires_full_context = true`)
    /// WITHOUT running the expensive full-context LL loop. The generated
    /// two-stage prediction uses only that boolean to decide whether to re-run
    /// with the real outer context, so the empty-context LL pass this skips is
    /// discarded work. Mirrors Go's execATN, which returns "needs LL" from the
    /// SLL stage rather than computing LL twice.
    sll_probe_only: bool,
}

#[derive(Clone, Copy)]
struct DfaEdge {
    decision: usize,
    source_state: DfaStateId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DfaPredictionInfo {
    prediction: ParserAtnPrediction,
    conflicting_alts: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FullContextPrediction {
    prediction: ParserAtnPrediction,
    stop_index: usize,
    resolution: FullContextResolution,
}

/// How the full-context loop settled, mirroring the two exits of Java's
/// `execATNWithFullContext`: a truly unique alt (reported as context
/// sensitivity) or a conflict resolution (reported as ambiguity, exact or
/// not).
#[derive(Clone, Debug, Eq, PartialEq)]
enum FullContextResolution {
    Unique,
    Ambiguous { exact: bool, alts: Vec<usize> },
}

fn full_context_prediction(
    alt: usize,
    configs: &AtnConfigSet,
    stop_index: usize,
    resolution: FullContextResolution,
) -> FullContextPrediction {
    FullContextPrediction {
        prediction: ParserAtnPrediction {
            alt,
            requires_full_context: true,
            has_semantic_context: configs_have_semantic_context_for_alt(configs, alt),
            diagnostic: None,
        },
        stop_index,
        resolution,
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ClosureConfigKey {
    state: usize,
    alt: usize,
    context: ContextId,
    semantic_context: SemanticContext,
    precedence_filter_suppressed: bool,
}

impl From<&AtnConfig> for ClosureConfigKey {
    fn from(config: &AtnConfig) -> Self {
        Self {
            state: config.state,
            alt: config.alt,
            context: config.context,
            semantic_context: config.semantic_context.clone(),
            precedence_filter_suppressed: config.precedence_filter_suppressed,
        }
    }
}

/// Reusable scratch buffers for `closure`. ANTLR's reference runtimes allocate a
/// fresh work stack and "closure busy" visited set per `closure` call (millions
/// of allocations on large parses); reusing one buffer across the per-config
/// calls of a single reach/start-state computation removes that churn. Each
/// `closure` call clears the buffers first, so the visited scope stays per-call
/// — behaviour-identical to allocating fresh sets.
#[derive(Default)]
struct ClosureScratch {
    /// Work stack of `(config, collect_predicates)`. The per-config
    /// `collect_predicates` flag mirrors ANTLR's
    /// `continueCollecting = collectPredicates && !ActionTransition`: once an
    /// action edge is crossed, predicates on the far side are NOT collected into
    /// the config's semantic context, so they are deferred to parse time rather
    /// than evaluated during prediction (the "action hides predicates" rule).
    stack: Vec<(AtnConfig, bool)>,
    visited: FxHashSet<ClosureConfigKey>,
}

/// Per-closure-tree invariants, grouped so `closure` stays within Clippy's
/// argument-count budget while threading the reusable [`ClosureScratch`].
#[derive(Clone, Copy)]
struct ClosureParams {
    precedence: i32,
    collect_predicates: bool,
    treat_eof_as_epsilon: bool,
}

#[derive(Debug)]
struct LookaheadIntStream {
    symbols: Vec<i32>,
    index: usize,
}

impl LookaheadIntStream {
    const fn new(symbols: Vec<i32>) -> Self {
        Self { symbols, index: 0 }
    }
}

impl IntStream for LookaheadIntStream {
    fn consume(&mut self) {
        if self.la(1) != TOKEN_EOF {
            self.index += 1;
        }
    }

    fn la(&mut self, offset: isize) -> i32 {
        if offset <= 0 {
            return 0;
        }
        let offset = offset.cast_unsigned() - 1;
        self.symbols
            .get(self.index + offset)
            .copied()
            .unwrap_or(TOKEN_EOF)
    }

    fn index(&self) -> usize {
        self.index
    }

    fn seek(&mut self, index: usize) {
        self.index = index.min(self.symbols.len());
    }

    fn size(&self) -> usize {
        self.symbols.len()
    }
}

fn initial_decision_dfas(atn: &Atn) -> Vec<ParserDfa> {
    atn.decision_to_state()
        .iter()
        .copied()
        .enumerate()
        .map(|(decision, state)| {
            let mut dfa = ParserDfa::with_max_token_type(state, decision, atn.max_token_type());
            if atn
                .state(state)
                .is_some_and(|state| state.precedence_rule_decision)
            {
                dfa.set_precedence_dfa(true);
            }
            dfa
        })
        .collect()
}

/// Merges a dropping simulator's DFAs into tables that another simulator
/// checked in first, losslessly. The two evolved independently (the
/// later-constructed one started cold), so numeric state ids are not
/// comparable — but DFA states ARE comparable by their ATN config set, the
/// same identity `ParserDfa::add_state` dedups on. Re-keying `local`'s states into
/// `shared`'s numbering and unioning edges/starts means overlapping
/// simulators never lose learned coverage, however it is distributed.
/// Walking every state is fine here: this only runs on the rare
/// overlapping-simulators drop path.
fn union_decision_dfas(shared: &mut Vec<ParserDfa>, local: Vec<ParserDfa>) {
    if shared.len() != local.len() {
        *shared = local;
        return;
    }
    for (shared_dfa, local_dfa) in shared.iter_mut().zip(local) {
        union_decision_dfa(shared_dfa, local_dfa);
    }
}

fn union_prediction_stores(
    shared: &mut PredictionStore,
    mut local: PredictionStore,
    workspace: &mut PredictionWorkspace,
) {
    let remap = shared.contexts.import_all(&local.contexts, workspace);
    for dfa in &mut local.decision_to_dfa {
        dfa.remap_contexts(&remap, &shared.contexts);
    }
    union_decision_dfas(&mut shared.decision_to_dfa, local.decision_to_dfa);
}

fn union_decision_dfa(shared: &mut ParserDfa, local: ParserDfa) {
    if shared.is_precedence_dfa() != local.is_precedence_dfa() {
        // A mode flip resets the tables (`set_precedence_dfa`), so the two are
        // not unionable; keep whichever learned more states.
        if local.state_count() > shared.state_count() {
            *shared = local;
        }
        return;
    }
    // Pass 1: map every local state number to a shared state number by
    // config-set identity, inserting the states shared has not learned.
    // Their edges reference local numbering, so they are cleared here and
    // re-added in pass 2 under the shared numbering.
    let mut renumber = Vec::with_capacity(local.state_count());
    for state in local.states() {
        let configs = local.configs(state.id());
        let number = shared.state_id_for_configs(configs).unwrap_or_else(|| {
            let missing = local.clone_state_without_edges(state.id());
            shared.insert_state(missing)
        });
        renumber.push(number);
    }
    // Pass 2: union edges, translating targets into shared numbering. The
    // incumbent's entries win; only gaps are filled. Accept metadata needs no
    // reconciliation: it is a pure function of the config set, and equal
    // config sets produced it through the same accept-time computation.
    for state in local.states() {
        let mapped = renumber[state.id().index()];
        for transition in state.transitions() {
            let Some(&mapped_target) = renumber.get(transition.target.index()) else {
                continue;
            };
            if shared.edge(mapped, transition.symbol).is_none() {
                shared.add_edge(mapped, transition.symbol, mapped_target);
            }
        }
    }
    if shared.start_state().is_none()
        && let Some(start) = local.start_state()
        && let Some(&mapped) = renumber.get(start.index())
    {
        shared.set_start_state(mapped);
    }
    for (precedence, start) in local.precedence_start_states().iter().copied().enumerate() {
        if start == NO_DFA_STATE {
            continue;
        }
        if shared.precedence_start_state(precedence).is_none()
            && let Some(&mapped) = renumber.get(start.index())
        {
            shared.set_precedence_start_state(precedence, mapped);
        }
    }
}

impl Drop for ParserAtnSimulator<'_> {
    fn drop(&mut self) {
        let Some(key) = self.shared_cache_key else {
            return;
        };
        #[cfg(feature = "perf-counters")]
        let publication_started = std::time::Instant::now();
        #[cfg(feature = "perf-counters")]
        let published_states = self
            .store
            .decision_to_dfa
            .iter()
            .map(ParserDfa::state_count)
            .sum();
        // Check the DFAs back IN by move. The slot is normally vacant because
        // `new_shared` checked them out; it is occupied only when another
        // simulator for the same ATN was created while this one was alive
        // (that one started cold and checked its copy in first) — then union
        // the two by config-set identity so neither side's learning is lost.
        let store = std::mem::take(&mut self.store);
        SHARED_PREDICTION_STORES.with(|cache| {
            let mut cache = cache.borrow_mut();
            if let Some(shared) = cache.get_mut(&key) {
                union_prediction_stores(shared, store, &mut self.workspace);
            } else {
                cache.insert(key, store);
            }
        });
        #[cfg(feature = "perf-counters")]
        crate::perf::record_dfa_cache_publication(
            publication_started.elapsed().as_nanos(),
            published_states,
        );
    }
}

impl<'a> ParserAtnSimulator<'a> {
    pub fn new(atn: &'a Atn) -> Self {
        Self {
            atn,
            store: PredictionStore::new(atn),
            workspace: PredictionWorkspace::default(),
            outer_context_cache: None,
            outer_context_cache_hits: 0,
            outer_context_cache_misses: 0,
            shared_cache_key: None,
            exact_ambig_detection: false,
        }
    }

    /// Switches the full-context resolution strategy (Java's
    /// `LL_EXACT_AMBIG_DETECTION` versus plain `LL`).
    pub const fn set_exact_ambig_detection(&mut self, exact: bool) {
        self.exact_ambig_detection = exact;
    }

    /// Creates a simulator that starts from, and publishes back into, a
    /// thread-local DFA cache keyed by a generated parser's static ATN.
    ///
    /// Generated parsers usually create a fresh parser object per parse. Without
    /// this cache every parse relearns the same adaptive DFA; with it, later
    /// parser instances reuse the SLL cache learned by earlier instances while
    /// still keeping mutable simulator state local to the parser during a parse.
    ///
    /// The DFAs are checked OUT of the cache by move (and back in on drop):
    /// cloning a warm DFA per parser instance costs O(learned states) — ~10%
    /// of a small parse. A second simulator created for the same ATN while one
    /// is alive finds the slot empty and starts cold; the drop-time check-in
    /// then remaps its context IDs and unions both independently learned stores.
    /// Renders every non-empty learned decision DFA in the format of Java's
    /// `Parser.dumpDFA()` / `DFASerializer` — `Decision N:` headers followed
    /// by `s0-'else'->:s1^=>1` edge lines — which the runtime testsuite's
    /// `showDFA` descriptors byte-compare.
    pub fn dump_dfa_java_style(&self, vocabulary: &crate::vocabulary::Vocabulary) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let mut seen_one = false;
        for dfa in &self.store.decision_to_dfa {
            if dfa.is_empty() {
                continue;
            }
            if seen_one {
                out.push('\n');
            }
            seen_one = true;
            let _ = writeln!(out, "Decision {}:", dfa.decision());
            for state in dfa.states() {
                let source = dfa_state_display(state);
                for transition in state.transitions() {
                    let Some(target_state) = dfa.state(transition.target) else {
                        continue;
                    };
                    let label = vocabulary.display_name(transition.symbol);
                    let _ = writeln!(out, "{source}-{label}->{}", dfa_state_display(target_state));
                }
            }
        }
        out
    }

    pub fn new_shared(atn: &'static Atn) -> Self {
        let ptr: *const Atn = atn;
        let key = ptr as usize;
        #[cfg(feature = "perf-counters")]
        let import_started = std::time::Instant::now();
        let store = SHARED_PREDICTION_STORES
            .with(|cache| cache.borrow_mut().remove(&key))
            .unwrap_or_else(|| PredictionStore::new(atn));
        #[cfg(feature = "perf-counters")]
        crate::perf::record_dfa_cache_import(
            import_started.elapsed().as_nanos(),
            store
                .decision_to_dfa
                .iter()
                .map(ParserDfa::state_count)
                .sum(),
        );
        Self {
            atn,
            store,
            workspace: PredictionWorkspace::default(),
            outer_context_cache: None,
            outer_context_cache_hits: 0,
            outer_context_cache_misses: 0,
            shared_cache_key: Some(key),
            exact_ambig_detection: false,
        }
    }

    pub fn decision_dfas(&self) -> &[ParserDfa] {
        &self.store.decision_to_dfa
    }

    /// Returns aggregate learned parser-DFA storage and interning measurements.
    pub fn parser_dfa_stats(&self) -> ParserDfaStats {
        let mut stats = ParserDfaStats::default();
        for dfa in &self.store.decision_to_dfa {
            stats.add_assign(dfa.stats());
        }
        stats
    }

    /// Returns compact prediction-context allocation and interning totals for
    /// this simulator's learned store.
    pub fn prediction_context_stats(&self) -> PredictionContextStats {
        let mut stats = self.store.contexts.stats();
        stats.retained_bytes += self.workspace.retained_bytes();
        stats.workspace_merge_cache_entries = self.workspace.merge_cache_len();
        stats.workspace_merge_cache_capacity = self.workspace.merge_cache_capacity();
        stats.workspace_entry_capacity = self.workspace.entry_capacity();
        stats.outer_context_cache_hits = self.outer_context_cache_hits;
        stats.outer_context_cache_misses = self.outer_context_cache_misses;
        stats
    }

    /// Interns a generated parser's outer call stack in this simulator's
    /// context arena. Return states must be supplied outermost to innermost,
    /// and `rule_context_version` must change whenever that stack changes.
    pub fn intern_prediction_context(
        &mut self,
        rule_context_version: usize,
        return_states: impl IntoIterator<Item = usize>,
    ) -> ContextId {
        if let Some(cached) = self.outer_context_cache
            && cached.rule_context_version == rule_context_version
        {
            self.outer_context_cache_hits = self.outer_context_cache_hits.saturating_add(1);
            return cached.context;
        }
        self.outer_context_cache_misses = self.outer_context_cache_misses.saturating_add(1);
        let mut context = EMPTY_CONTEXT;
        for return_state in return_states {
            context = self.store.contexts.singleton(context, return_state);
        }
        self.outer_context_cache = Some(CachedOuterContext {
            rule_context_version,
            context,
        });
        context
    }

    pub fn adaptive_predict(
        &mut self,
        decision: usize,
        lookahead: impl IntoIterator<Item = i32>,
    ) -> Result<usize, ParserAtnSimulatorError> {
        self.adaptive_predict_with_precedence(decision, 0, lookahead)
    }

    pub fn adaptive_predict_stream<T: IntStream>(
        &mut self,
        decision: usize,
        input: &mut T,
    ) -> Result<usize, ParserAtnSimulatorError> {
        self.adaptive_predict_stream_with_precedence(decision, 0, input)
    }

    pub fn adaptive_predict_stream_with_precedence<T: IntStream>(
        &mut self,
        decision: usize,
        precedence: usize,
        input: &mut T,
    ) -> Result<usize, ParserAtnSimulatorError> {
        self.adaptive_predict_stream_info_with_precedence(decision, precedence, input)
            .map(|prediction| prediction.alt)
    }

    pub fn adaptive_predict_stream_info_with_precedence<T: IntStream>(
        &mut self,
        decision: usize,
        precedence: usize,
        input: &mut T,
    ) -> Result<ParserAtnPrediction, ParserAtnSimulatorError> {
        let marker = input.mark();
        let index = input.index();
        let mut workspace = std::mem::take(&mut self.workspace);
        workspace.reset();
        let result = self.adaptive_predict_stream_inner(
            AdaptivePredictRequest {
                decision,
                precedence,
                outer_context: EMPTY_CONTEXT,
                force_full_context_retry: false,
                sll_probe_only: false,
            },
            input,
            &mut workspace,
        );
        self.workspace = workspace;
        input.seek(index);
        input.release(marker);
        result
    }

    /// SLL-probe variant of [`Self::adaptive_predict_stream_info_with_precedence`].
    ///
    /// Identical to the precedence entry except that, when the SLL walk reaches
    /// a conflict state requiring full context, it returns the SLL prediction
    /// (carrying `requires_full_context = true`) WITHOUT running the
    /// full-context LL loop. The generated two-stage prediction calls this for
    /// stage 1 and only consults `requires_full_context` to decide whether to
    /// re-run with the real outer context, so the empty-context LL pass this
    /// skips would be discarded anyway. Avoids the double LL pass per escalation.
    pub fn adaptive_predict_stream_info_sll_probe<T: IntStream>(
        &mut self,
        decision: usize,
        precedence: usize,
        input: &mut T,
    ) -> Result<ParserAtnPrediction, ParserAtnSimulatorError> {
        let marker = input.mark();
        let index = input.index();
        let mut workspace = std::mem::take(&mut self.workspace);
        workspace.reset();
        let result = self.adaptive_predict_stream_inner(
            AdaptivePredictRequest {
                decision,
                precedence,
                outer_context: EMPTY_CONTEXT,
                force_full_context_retry: false,
                sll_probe_only: true,
            },
            input,
            &mut workspace,
        );
        self.workspace = workspace;
        input.seek(index);
        input.release(marker);
        result
    }

    pub fn adaptive_predict_stream_info_with_context<T: IntStream>(
        &mut self,
        decision: usize,
        precedence: usize,
        input: &mut T,
        outer_context: ContextId,
    ) -> Result<ParserAtnPrediction, ParserAtnSimulatorError> {
        self.store.contexts.assert_valid(outer_context);
        let marker = input.mark();
        let index = input.index();
        let mut workspace = std::mem::take(&mut self.workspace);
        workspace.reset();
        let result = self.adaptive_predict_stream_inner(
            AdaptivePredictRequest {
                decision,
                precedence,
                outer_context,
                force_full_context_retry: true,
                sll_probe_only: false,
            },
            input,
            &mut workspace,
        );
        self.workspace = workspace;
        input.seek(index);
        input.release(marker);
        result
    }

    pub fn adaptive_predict_with_precedence(
        &mut self,
        decision: usize,
        precedence: usize,
        lookahead: impl IntoIterator<Item = i32>,
    ) -> Result<usize, ParserAtnSimulatorError> {
        self.adaptive_predict_info_with_precedence(decision, precedence, lookahead)
            .map(|prediction| prediction.alt)
    }

    pub fn adaptive_predict_info_with_precedence(
        &mut self,
        decision: usize,
        precedence: usize,
        lookahead: impl IntoIterator<Item = i32>,
    ) -> Result<ParserAtnPrediction, ParserAtnSimulatorError> {
        let mut input = LookaheadIntStream::new(lookahead.into_iter().collect());
        self.adaptive_predict_stream_info_with_precedence(decision, precedence, &mut input)
    }

    fn adaptive_predict_stream_inner<T: IntStream>(
        &mut self,
        request: AdaptivePredictRequest,
        input: &mut T,
        merge_cache: &mut PredictionWorkspace,
    ) -> Result<ParserAtnPrediction, ParserAtnSimulatorError> {
        let AdaptivePredictRequest {
            decision,
            precedence,
            outer_context,
            force_full_context_retry,
            sll_probe_only,
        } = request;
        #[cfg(feature = "perf-counters")]
        crate::perf::record_adaptive_call(decision, force_full_context_retry);
        let Some(&decision_state) = self.atn.decision_to_state().get(decision) else {
            return Err(ParserAtnSimulatorError::UnknownDecision(decision));
        };
        let start_index = input.index();
        // Precedence originates from the parser's precedence stack (rule nesting
        // depth), so it is always small in practice. A value above `i32::MAX`
        // would be clamped here; the clamp only ever affects pathological inputs
        // and at worst over-filters precedence transitions, never miscomputing a
        // real parse.
        let precedence = i32::try_from(precedence).unwrap_or(i32::MAX);
        let mut state_number =
            self.ensure_start_state(decision, decision_state, precedence, merge_cache)?;
        if let Some(prediction) = self.prediction_or_full_context(
            input,
            PredictionCheck {
                decision,
                decision_state,
                state_number,
                start_index,
                precedence,
                outer_context,
                force_full_context_retry,
                sll_probe_only,
            },
            merge_cache,
        )? {
            return Ok(prediction);
        }
        loop {
            let symbol = input.la(1);
            let target = self
                .store
                .decision_to_dfa
                .get(decision)
                .and_then(|dfa| dfa.edge(state_number, symbol));
            #[cfg(feature = "perf-counters")]
            crate::perf::record_dfa_edge_lookup(target.is_some());
            if let Some(target) = target {
                state_number = target;
            } else {
                let configs = self
                    .store
                    .decision_to_dfa
                    .get(decision)
                    .map(|dfa| dfa.configs(state_number).clone())
                    .ok_or(ParserAtnSimulatorError::MissingDfaState(state_number))?;
                let target = match self.compute_target_state(
                    DfaEdge {
                        decision,
                        source_state: state_number,
                    },
                    &configs,
                    symbol,
                    precedence,
                    merge_cache,
                ) {
                    Ok(target) => target,
                    Err(ParserAtnSimulatorError::NoViableAlt { symbol, .. }) => {
                        return Err(ParserAtnSimulatorError::NoViableAlt {
                            symbol,
                            index: input.index(),
                        });
                    }
                    Err(error) => return Err(error),
                };
                state_number = target;
            }
            if let Some(prediction) = self.prediction_or_full_context(
                input,
                PredictionCheck {
                    decision,
                    decision_state,
                    state_number,
                    start_index,
                    precedence,
                    outer_context,
                    force_full_context_retry,
                    sll_probe_only,
                },
                merge_cache,
            )? {
                return Ok(prediction);
            }
            if symbol == TOKEN_EOF {
                // We ran out of input while still inside the decision and the
                // current state is not a clean accept. ANTLR's execATN takes one
                // more step on EOF, reaches an empty reach set, and falls back to
                // getSynValidOrSemInvalidAltThatFinishedDecisionEntryRule: any alt
                // whose configs already reached the decision's rule-stop (i.e. an
                // exit alt of a `(...)*`/`(...)+`/precedence loop) is a valid
                // prediction, not a syntax error. Mirror that fallback here so we
                // exit the loop cleanly instead of reporting a spurious
                // "no viable alternative at input '<EOF>'".
                if let Some(configs) = self
                    .store
                    .decision_to_dfa
                    .get(decision)
                    .map(|dfa| dfa.configs(state_number).clone())
                    && let Some(alt) = self.alt_that_finished_decision_entry_rule(&configs)
                {
                    return Ok(ParserAtnPrediction {
                        alt,
                        requires_full_context: false,
                        has_semantic_context: configs_have_semantic_context_for_alt(&configs, alt),
                        diagnostic: None,
                    });
                }
                return Err(ParserAtnSimulatorError::PredictionRequiresMoreLookahead);
            }
            input.consume();
        }
    }

    fn prediction_or_full_context<T: IntStream>(
        &mut self,
        input: &mut T,
        check: PredictionCheck,
        merge_cache: &mut PredictionWorkspace,
    ) -> Result<Option<ParserAtnPrediction>, ParserAtnSimulatorError> {
        let PredictionCheck {
            decision,
            decision_state,
            state_number,
            start_index,
            precedence,
            outer_context,
            force_full_context_retry,
            sll_probe_only,
        } = check;
        if self.store.contexts.is_empty(outer_context)
            && let Some(prediction) =
                self.non_greedy_exit_prediction(decision, decision_state, state_number)
        {
            return Ok(Some(prediction));
        }
        let Some(info) = self.dfa_prediction_info(decision, state_number) else {
            return Ok(None);
        };
        let prediction = info.prediction;
        // SLL-probe stage: the caller only needs to know that this conflict
        // requires full context; it will re-run with the real outer context.
        // Returning the SLL prediction here (with requires_full_context set)
        // avoids running the full-context LL loop with the empty probe context,
        // whose result the generated two-stage code discards. Mirrors Go's
        // execATN, which signals "needs LL" instead of computing LL twice.
        if sll_probe_only && prediction.requires_full_context {
            return Ok(Some(prediction));
        }
        if prediction.requires_full_context
            && (force_full_context_retry || !prediction.has_semantic_context)
        {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_full_context_retry(decision);
            let sll_stop_index = input.index();
            input.seek(start_index);
            let full_context = self.adaptive_predict_full_context(
                decision_state,
                input,
                precedence,
                outer_context,
                merge_cache,
            )?;
            let (kind, exact, conflicting_alts) = match full_context.resolution {
                FullContextResolution::Ambiguous { exact, ref alts } => (
                    ParserAtnPredictionDiagnosticKind::Ambiguity,
                    exact,
                    alts.clone(),
                ),
                // A unique full-context alt after an SLL conflict is Java's
                // reportContextSensitivity; the SLL state's conflicting alts
                // describe the conflict that forced the retry.
                FullContextResolution::Unique => (
                    ParserAtnPredictionDiagnosticKind::ContextSensitivity,
                    false,
                    info.conflicting_alts,
                ),
            };
            let mut prediction = full_context.prediction;
            if conflicting_alts.len() > 1 {
                prediction.diagnostic = Some(ParserAtnPredictionDiagnostic {
                    kind,
                    start_index,
                    sll_stop_index,
                    ll_stop_index: full_context.stop_index,
                    conflicting_alts,
                    exact,
                });
            }
            return Ok(Some(prediction));
        }
        Ok(Some(prediction))
    }

    fn non_greedy_exit_prediction(
        &self,
        decision: usize,
        decision_state: usize,
        state_number: DfaStateId,
    ) -> Option<ParserAtnPrediction> {
        if !self
            .atn
            .state(decision_state)
            .is_some_and(|state| state.non_greedy)
        {
            return None;
        }
        let configs = &self
            .store
            .decision_to_dfa
            .get(decision)?
            .configs(state_number);
        let alt = configs
            .configs()
            .iter()
            .filter(|config| {
                self.atn
                    .state(config.state)
                    .is_some_and(AtnState::is_rule_stop)
                    && self.store.contexts.has_empty_path(config.context)
            })
            .map(|config| config.alt)
            .min()?;
        Some(ParserAtnPrediction {
            alt,
            requires_full_context: false,
            has_semantic_context: configs_have_semantic_context_for_alt(configs, alt),
            diagnostic: None,
        })
    }

    fn ensure_start_state(
        &mut self,
        decision: usize,
        decision_state: usize,
        precedence: i32,
        merge_cache: &mut PredictionWorkspace,
    ) -> Result<DfaStateId, ParserAtnSimulatorError> {
        if self.store.decision_to_dfa[decision].is_precedence_dfa() {
            let precedence_key = usize::try_from(precedence.max(0)).unwrap_or_default();
            if let Some(start) =
                self.store.decision_to_dfa[decision].precedence_start_state(precedence_key)
            {
                return Ok(start);
            }
        } else if let Some(start) = self.store.decision_to_dfa[decision].start_state() {
            return Ok(start);
        }
        let decision_state = self
            .atn
            .state(decision_state)
            .ok_or(ParserAtnSimulatorError::MissingAtnState(decision_state))?;
        let configs = self.compute_start_state(decision_state, precedence, merge_cache);
        let state_number = self.add_dfa_state(decision, DfaStateBuilder::new(configs));
        if self.store.decision_to_dfa[decision].is_precedence_dfa() {
            let precedence_key = usize::try_from(precedence.max(0)).unwrap_or_default();
            self.store.decision_to_dfa[decision]
                .set_precedence_start_state(precedence_key, state_number);
        } else {
            self.store.decision_to_dfa[decision].set_start_state(state_number);
        }
        Ok(state_number)
    }

    fn add_dfa_state(&mut self, decision: usize, state: DfaStateBuilder) -> DfaStateId {
        self.store.decision_to_dfa[decision].add_state(state)
    }

    fn compute_start_state(
        &mut self,
        decision_state: &AtnState,
        precedence: i32,
        merge_cache: &mut PredictionWorkspace,
    ) -> AtnConfigSet {
        self.compute_start_state_with_context(
            decision_state,
            false,
            EMPTY_CONTEXT,
            precedence,
            merge_cache,
        )
    }

    fn compute_start_state_with_context(
        &mut self,
        decision_state: &AtnState,
        full_context: bool,
        initial_context: ContextId,
        precedence: i32,
        merge_cache: &mut PredictionWorkspace,
    ) -> AtnConfigSet {
        let mut configs = AtnConfigSet::new_full_context(full_context);
        let mut scratch = ClosureScratch::default();
        let params = ClosureParams {
            precedence,
            collect_predicates: true,
            treat_eof_as_epsilon: false,
        };
        for (index, transition) in decision_state.transitions.iter().enumerate() {
            let alt = index + 1;
            let config = AtnConfig::new(
                transition.target(),
                alt,
                initial_context,
                &self.store.contexts,
            );
            self.closure(config, &mut configs, merge_cache, &mut scratch, params);
        }
        configs
    }

    fn adaptive_predict_full_context<T: IntStream>(
        &mut self,
        decision_state: usize,
        input: &mut T,
        precedence: i32,
        outer_context: ContextId,
        merge_cache: &mut PredictionWorkspace,
    ) -> Result<FullContextPrediction, ParserAtnSimulatorError> {
        let decision_state = self
            .atn
            .state(decision_state)
            .ok_or(ParserAtnSimulatorError::MissingAtnState(decision_state))?;
        let mut configs = self.compute_start_state_with_context(
            decision_state,
            true,
            outer_context,
            precedence,
            merge_cache,
        );
        // Java's `execATNWithFullContext`: after each reach set a truly
        // unique alt resolves as context sensitivity. Otherwise default LL
        // mode stops at the first "resolves to just one viable alt" conflict
        // — reported as a NON-exact ambiguity, which the exactOnly listener
        // suppresses — while LL_EXACT_AMBIG_DETECTION keeps consuming until
        // every (state, context) subset conflicts over the same alt set: an
        // exact ambiguity.
        loop {
            if let Some(alt) = configs.unique_alt() {
                return Ok(full_context_prediction(
                    alt,
                    &configs,
                    input.index(),
                    FullContextResolution::Unique,
                ));
            }
            let symbol = input.la(1);
            let reach = self.compute_reach_set(&configs, symbol, true, precedence, merge_cache);
            if reach.is_empty() {
                return Err(ParserAtnSimulatorError::NoViableAlt {
                    symbol,
                    index: input.index(),
                });
            }
            configs = reach;
            if let Some(alt) = configs.unique_alt() {
                return Ok(full_context_prediction(
                    alt,
                    &configs,
                    input.index(),
                    FullContextResolution::Unique,
                ));
            }
            if !configs.has_semantic_context() {
                let subsets = conflicting_alt_subsets(configs.configs());
                if self.exact_ambig_detection {
                    let alts: Vec<usize> = configs.alts().into_iter().collect();
                    // Both subset checks hold vacuously for an empty list; a
                    // real exact ambiguity always carries alternatives, so
                    // guard the pick instead of indexing.
                    if all_subsets_conflict(&subsets)
                        && all_subsets_equal(&subsets)
                        && let Some(&alt) = alts.first()
                    {
                        return Ok(full_context_prediction(
                            alt,
                            &configs,
                            input.index(),
                            FullContextResolution::Ambiguous { exact: true, alts },
                        ));
                    }
                } else if let Some(alt) = single_viable_alt(&subsets) {
                    let alts: Vec<usize> = configs.alts().into_iter().collect();
                    return Ok(full_context_prediction(
                        alt,
                        &configs,
                        input.index(),
                        FullContextResolution::Ambiguous { exact: false, alts },
                    ));
                }
            }
            if symbol == TOKEN_EOF || self.configs_all_reached_rule_stop(&configs) {
                // Safety net Java reaches implicitly: at EOF every surviving
                // path sits in a rule-stop config, so the checks above
                // resolve; guard against pathological sets instead of
                // spinning on an unconsumable EOF.
                let alts: Vec<usize> = configs.alts().into_iter().collect();
                let alt = *alts
                    .first()
                    .ok_or(ParserAtnSimulatorError::PredictionRequiresMoreLookahead)?;
                let resolution = if alts.len() > 1 {
                    FullContextResolution::Ambiguous {
                        exact: self.exact_ambig_detection,
                        alts,
                    }
                } else {
                    FullContextResolution::Unique
                };
                return Ok(full_context_prediction(
                    alt,
                    &configs,
                    input.index(),
                    resolution,
                ));
            }
            input.consume();
        }
    }

    fn compute_target_state(
        &mut self,
        edge: DfaEdge,
        configs: &AtnConfigSet,
        symbol: i32,
        precedence: i32,
        merge_cache: &mut PredictionWorkspace,
    ) -> Result<DfaStateId, ParserAtnSimulatorError> {
        let mut reach = self.compute_reach_set(configs, symbol, false, precedence, merge_cache);
        if reach.is_empty() {
            if let Some(prediction) = self.alt_that_finished_decision_entry_rule(configs) {
                let mut dfa_state = DfaStateBuilder::new(configs.clone());
                dfa_state.mark_accept(prediction);
                // The set-wide flag gates the per-alt scan: if no config in the
                // set carries a semantic context, no alt can either.
                dfa_state.set_has_semantic_context_for_alt(
                    configs.has_semantic_context()
                        && configs_have_semantic_context_for_alt(configs, prediction),
                );
                let target_state = self.add_dfa_state(edge.decision, dfa_state);
                self.store.decision_to_dfa[edge.decision].add_edge(
                    edge.source_state,
                    symbol,
                    target_state,
                );
                return Ok(target_state);
            }
            return Err(ParserAtnSimulatorError::NoViableAlt { symbol, index: 0 });
        }
        let prediction = reach.unique_alt();
        let conflict_prediction = prediction.or_else(|| {
            if !has_sll_conflict_terminating_prediction(&reach, |state| {
                self.atn.state(state).is_some_and(AtnState::is_rule_stop)
            }) {
                return None;
            }
            reach
                .conflicting_alts()
                .into_iter()
                .next()
                .or_else(|| reach.alts().into_iter().next())
        });
        let requires_full_context = prediction.is_none() && conflict_prediction.is_some();
        #[cfg(feature = "perf-counters")]
        if requires_full_context {
            crate::perf::record_sll_conflict(edge.decision);
        }
        let conflicting_alts = if requires_full_context {
            let alts = reach.conflicting_alts();
            if alts.is_empty() { reach.alts() } else { alts }
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        let mut dfa_state = DfaStateBuilder::new(reach);
        if let Some(prediction) = conflict_prediction {
            dfa_state.mark_accept(prediction);
            dfa_state.set_requires_full_context(requires_full_context);
            dfa_state.set_conflicting_alts(conflicting_alts);
            // The set-wide flag gates the per-alt scan: if no config in the set
            // carries a semantic context, no alt can either.
            dfa_state.set_has_semantic_context_for_alt(
                dfa_state.configs.has_semantic_context()
                    && configs_have_semantic_context_for_alt(&dfa_state.configs, prediction),
            );
        }
        let target_state = self.add_dfa_state(edge.decision, dfa_state);
        self.store.decision_to_dfa[edge.decision].add_edge(edge.source_state, symbol, target_state);
        Ok(target_state)
    }

    fn compute_reach_set(
        &mut self,
        configs: &AtnConfigSet,
        symbol: i32,
        full_context: bool,
        precedence: i32,
        merge_cache: &mut PredictionWorkspace,
    ) -> AtnConfigSet {
        let mut intermediate = AtnConfigSet::new_full_context(full_context);
        let mut skipped_stop_states = Vec::new();
        for config in configs.configs() {
            let Some(state) = self.atn.state(config.state) else {
                continue;
            };
            if state.is_rule_stop() {
                if full_context || symbol == TOKEN_EOF {
                    skipped_stop_states.push(config.clone());
                }
                continue;
            }
            for transition in &state.transitions {
                if transition.matches(symbol, 1, self.atn.max_token_type()) {
                    let target =
                        config.moved_to(transition.target(), config.context, &self.store.contexts);
                    intermediate.add(target, &mut self.store.contexts, merge_cache);
                }
            }
        }
        let mut reach = if skipped_stop_states.is_empty() && symbol != TOKEN_EOF {
            if intermediate.len() == 1 || intermediate.unique_alt().is_some() {
                intermediate
            } else {
                self.close_intermediate_reach_set(
                    intermediate,
                    full_context,
                    precedence,
                    symbol,
                    merge_cache,
                )
            }
        } else {
            self.close_intermediate_reach_set(
                intermediate,
                full_context,
                precedence,
                symbol,
                merge_cache,
            )
        };
        if symbol == TOKEN_EOF {
            reach = self.rule_stop_configs(reach, merge_cache);
        }
        if !full_context || !self.configs_contain_rule_stop(&reach) {
            for config in skipped_stop_states {
                reach.add(config, &mut self.store.contexts, merge_cache);
            }
        }
        #[cfg(feature = "perf-counters")]
        crate::perf::record_reach_set(full_context, configs.len(), reach.len());
        reach
    }

    fn close_intermediate_reach_set(
        &mut self,
        intermediate: AtnConfigSet,
        full_context: bool,
        precedence: i32,
        symbol: i32,
        merge_cache: &mut PredictionWorkspace,
    ) -> AtnConfigSet {
        let mut reach = AtnConfigSet::new_full_context(full_context);
        let mut scratch = ClosureScratch::default();
        let params = ClosureParams {
            precedence,
            collect_predicates: false,
            treat_eof_as_epsilon: symbol == TOKEN_EOF,
        };
        // `closure` takes `AtnConfig` by value, so drain the intermediate set by
        // move instead of cloning each config.
        for config in intermediate.into_configs() {
            self.closure(config, &mut reach, merge_cache, &mut scratch, params);
        }
        reach
    }

    fn alt_that_finished_decision_entry_rule(&self, configs: &AtnConfigSet) -> Option<usize> {
        configs
            .configs()
            .iter()
            .filter(|config| {
                config.reaches_into_outer_context > 0
                    || self
                        .atn
                        .state(config.state)
                        .is_some_and(AtnState::is_rule_stop)
                        && self.store.contexts.has_empty_path(config.context)
            })
            .map(|config| config.alt)
            .min()
    }

    fn rule_stop_configs(
        &mut self,
        configs: AtnConfigSet,
        merge_cache: &mut PredictionWorkspace,
    ) -> AtnConfigSet {
        if configs.configs().iter().all(|config| {
            self.atn
                .state(config.state)
                .is_some_and(AtnState::is_rule_stop)
        }) {
            return configs;
        }
        let mut result = AtnConfigSet::new_full_context(configs.full_context());
        for config in configs.configs().iter().filter(|config| {
            self.atn
                .state(config.state)
                .is_some_and(AtnState::is_rule_stop)
        }) {
            result.add(config.clone(), &mut self.store.contexts, merge_cache);
        }
        result
    }

    fn configs_all_reached_rule_stop(&self, configs: &AtnConfigSet) -> bool {
        configs.configs().iter().all(|config| {
            self.atn
                .state(config.state)
                .is_some_and(AtnState::is_rule_stop)
        })
    }

    fn configs_contain_rule_stop(&self, configs: &AtnConfigSet) -> bool {
        configs.configs().iter().any(|config| {
            self.atn
                .state(config.state)
                .is_some_and(AtnState::is_rule_stop)
        })
    }

    fn closure(
        &mut self,
        config: AtnConfig,
        configs: &mut AtnConfigSet,
        merge_cache: &mut PredictionWorkspace,
        scratch: &mut ClosureScratch,
        params: ClosureParams,
    ) {
        let ClosureParams {
            precedence,
            collect_predicates,
            treat_eof_as_epsilon,
        } = params;
        scratch.stack.clear();
        scratch.visited.clear();
        scratch.stack.push((config, collect_predicates));
        while let Some((config, collect_predicates)) = scratch.stack.pop() {
            if !scratch.visited.insert(ClosureConfigKey::from(&config)) {
                continue;
            }
            let Some(state) = self.atn.state(config.state) else {
                continue;
            };
            let at_rule_stop = state.is_rule_stop();
            if at_rule_stop
                && self.closure_at_rule_stop(
                    config.clone(),
                    collect_predicates,
                    configs,
                    merge_cache,
                    &mut scratch.stack,
                )
            {
                continue;
            }
            let epsilon_only = !state.transitions.is_empty()
                && state.transitions.iter().all(Transition::is_epsilon);
            if !epsilon_only {
                configs.add(config.clone(), &mut self.store.contexts, merge_cache);
            }
            for (index, transition) in state.transitions.iter().enumerate() {
                if index == 0
                    && can_drop_left_recursive_loop_entry_edge(
                        self.atn,
                        state,
                        &self.store.contexts,
                        config.context,
                    )
                {
                    continue;
                }
                if transition.is_epsilon() {
                    if let Some(mut target) = self.epsilon_target_config(
                        &config,
                        transition,
                        precedence,
                        collect_predicates,
                        configs.full_context(),
                    ) {
                        if at_rule_stop {
                            target.reaches_into_outer_context =
                                target.reaches_into_outer_context.saturating_add(1);
                        }
                        // ANTLR: stop collecting predicates once an action edge is
                        // crossed, so a predicate after an action is deferred to
                        // parse time rather than evaluated during prediction.
                        let target_collect_predicates =
                            collect_predicates && !matches!(transition, Transition::Action { .. });
                        scratch.stack.push((target, target_collect_predicates));
                    }
                } else if treat_eof_as_epsilon
                    && transition.matches(TOKEN_EOF, 1, self.atn.max_token_type())
                {
                    scratch.stack.push((
                        config.moved_to(transition.target(), config.context, &self.store.contexts),
                        collect_predicates,
                    ));
                }
            }
        }
        #[cfg(feature = "perf-counters")]
        crate::perf::record_closure(scratch.visited.len());
    }

    fn closure_at_rule_stop(
        &mut self,
        config: AtnConfig,
        collect_predicates: bool,
        configs: &mut AtnConfigSet,
        merge_cache: &mut PredictionWorkspace,
        stack: &mut Vec<(AtnConfig, bool)>,
    ) -> bool {
        if self.store.contexts.is_empty(config.context) {
            if configs.full_context() {
                configs.add(config, &mut self.store.contexts, merge_cache);
                return true;
            }
            return false;
        }
        let mut handled_all_paths = true;
        for index in 0..self.store.contexts.len(config.context) {
            let Some(return_state) = self.store.contexts.return_state(config.context, index) else {
                continue;
            };
            if return_state == EMPTY_RETURN_STATE {
                if configs.full_context() {
                    let mut empty_context_config = config.clone();
                    empty_context_config.set_context(EMPTY_CONTEXT, &self.store.contexts);
                    configs.add(empty_context_config, &mut self.store.contexts, merge_cache);
                } else {
                    handled_all_paths = false;
                }
                continue;
            }
            let parent = self
                .store
                .contexts
                .parent(config.context, index)
                .unwrap_or(EMPTY_CONTEXT);
            let next = config.moved_to(return_state, parent, &self.store.contexts);
            stack.push((next, collect_predicates));
        }
        handled_all_paths
    }

    fn epsilon_target_config(
        &mut self,
        config: &AtnConfig,
        transition: &Transition,
        precedence: i32,
        collect_predicates: bool,
        full_context: bool,
    ) -> Option<AtnConfig> {
        let semantic_context = match transition {
            Transition::Predicate {
                rule_index,
                pred_index,
                context_dependent,
                ..
            } if collect_predicates => SemanticContext::and(
                config.semantic_context.clone(),
                SemanticContext::Predicate {
                    rule_index: *rule_index,
                    pred_index: *pred_index,
                    context_dependent: *context_dependent,
                },
            ),
            Transition::Precedence {
                precedence: transition_precedence,
                ..
            } if collect_predicates && *transition_precedence < precedence => return None,
            Transition::Precedence { precedence, .. } if collect_predicates && !full_context => {
                SemanticContext::and(
                    config.semantic_context.clone(),
                    SemanticContext::Precedence {
                        precedence: *precedence,
                    },
                )
            }
            _ => config.semantic_context.clone(),
        };
        let context = match transition {
            Transition::Rule { follow_state, .. } => {
                self.store.contexts.singleton(config.context, *follow_state)
            }
            _ => config.context,
        };
        let mut target = config.moved_to(transition.target(), context, &self.store.contexts);
        target.semantic_context = semantic_context;
        Some(target)
    }

    fn dfa_prediction_info(
        &self,
        decision: usize,
        state_number: DfaStateId,
    ) -> Option<DfaPredictionInfo> {
        let dfa = self.store.decision_to_dfa.get(decision)?;
        let state = dfa.state(state_number)?;
        let alt = state.prediction()?;
        let requires_full_context = state.requires_full_context();
        let conflicting_alts = if requires_full_context {
            let stored = dfa.conflicting_alts(state_number);
            if stored.is_empty() {
                dfa.configs(state_number).alts().into_iter().collect()
            } else {
                stored.to_vec()
            }
        } else {
            Vec::new()
        };
        Some(DfaPredictionInfo {
            prediction: ParserAtnPrediction {
                alt,
                requires_full_context,
                // Precomputed at accept time (see compute_target_state) so
                // warm accept lookup does not rescan the cold config set.
                has_semantic_context: state.has_semantic_context(),
                diagnostic: None,
            },
            conflicting_alts,
        })
    }
}

/// Reports whether closure should skip the loop-entry branch for a
/// left-recursive rule under the current caller context.
pub(crate) fn can_drop_left_recursive_loop_entry_edge(
    atn: &Atn,
    state: &AtnState,
    contexts: &ContextArena,
    context: ContextId,
) -> bool {
    if state.kind != AtnStateKind::StarLoopEntry
        || !state.precedence_rule_decision
        || contexts.is_empty(context)
        || contexts.has_empty_path(context)
    {
        return false;
    }
    let Some(rule_index) = state.rule_index else {
        return false;
    };
    for index in 0..contexts.len(context) {
        let Some(return_state_number) = contexts.return_state(context, index) else {
            return false;
        };
        let Some(return_state) = atn.state(return_state_number) else {
            return false;
        };
        if return_state.rule_index != Some(rule_index) {
            return false;
        }
    }
    let Some(block_end_state_number) = state
        .transitions
        .first()
        .and_then(|transition| atn.state(transition.target()))
        .and_then(|decision_start| decision_start.end_state)
    else {
        return false;
    };
    for index in 0..contexts.len(context) {
        let return_state_number = contexts
            .return_state(context, index)
            .expect("return state checked above");
        let return_state = atn
            .state(return_state_number)
            .expect("return state checked above");
        if return_state.state_number == block_end_state_number {
            continue;
        }
        if return_state.transitions.len() != 1 || !return_state.transitions[0].is_epsilon() {
            return false;
        }
        let return_target = return_state.transitions[0].target();
        if return_state.kind == AtnStateKind::BlockEnd && return_target == state.state_number {
            continue;
        }
        if return_target == block_end_state_number {
            continue;
        }
        let Some(return_target_state) = atn.state(return_target) else {
            return false;
        };
        if return_target_state.kind == AtnStateKind::BlockEnd
            && return_target_state.transitions.len() == 1
            && return_target_state.transitions[0].is_epsilon()
            && return_target_state.transitions[0].target() == state.state_number
        {
            continue;
        }
        return false;
    }
    true
}

fn configs_have_semantic_context_for_alt(configs: &AtnConfigSet, alt: usize) -> bool {
    configs
        .configs()
        .iter()
        .any(|config| config.alt == alt && !config.semantic_context.is_none())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParserAtnSimulatorError {
    MissingAtnState(usize),
    MissingDfaState(DfaStateId),
    NoViableAlt { symbol: i32, index: usize },
    PredictionRequiresMoreLookahead,
    UnknownDecision(usize),
}

/// Java `DFASerializer.getStateString`: `:sN^=>alt` for accept states.
fn dfa_state_display(state: ParserDfaStateView<'_>) -> String {
    let mut out = String::new();
    if state.is_accept_state() {
        out.push(':');
    }
    out.push('s');
    out.push_str(&state.id().index().to_string());
    if state.requires_full_context() {
        out.push('^');
    }
    if state.is_accept_state() {
        out.push_str("=>");
        out.push_str(
            &state
                .prediction()
                .map(|prediction| prediction.to_string())
                .unwrap_or_default(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atn::{AtnStateKind, AtnType};

    #[test]
    fn union_decision_dfa_preserves_disjoint_coverage() {
        fn configs(
            atn_state: usize,
            arena: &mut ContextArena,
            workspace: &mut PredictionWorkspace,
        ) -> AtnConfigSet {
            let mut set = AtnConfigSet::new();
            set.add(
                AtnConfig::new(atn_state, 1, EMPTY_CONTEXT, arena),
                arena,
                workspace,
            );
            set
        }
        fn state(
            atn_state: usize,
            arena: &mut ContextArena,
            workspace: &mut PredictionWorkspace,
        ) -> DfaStateBuilder {
            DfaStateBuilder::new(configs(atn_state, arena, workspace))
        }
        let mut arena = ContextArena::new();
        let mut workspace = PredictionWorkspace::default();

        // Two DFAs that evolved independently from the same grammar: equal
        // state/edge counts, but disjoint transitions and different state
        // numbering for the shared successor.
        let mut shared = ParserDfa::with_max_token_type(0, 0, 8);
        let shared_root = shared.add_state(state(10, &mut arena, &mut workspace));
        let shared_a = shared.add_state(state(11, &mut arena, &mut workspace));
        shared.add_edge(shared_root, 1, shared_a);
        shared.set_start_state(shared_root);

        let mut local = ParserDfa::with_max_token_type(0, 0, 8);
        let local_b = local.add_state(state(12, &mut arena, &mut workspace));
        let local_root = local.add_state(state(10, &mut arena, &mut workspace));
        local.add_edge(local_root, 2, local_b);
        local.set_precedence_start_state(3, local_root);

        union_decision_dfa(&mut shared, local);

        // The root (same config set) gained local's edge without losing its
        // own, with the target re-keyed into shared numbering.
        assert_eq!(shared.edge(shared_root, 1), Some(shared_a));
        let merged_b = shared
            .state_id_for_configs(&configs(12, &mut arena, &mut workspace))
            .expect("local-only state adopted");
        assert_eq!(shared.edge(shared_root, 2), Some(merged_b));
        assert_eq!(shared.states().len(), 3);
        // Start-state gaps fill from local; incumbents are kept.
        assert_eq!(shared.start_state(), Some(shared_root));
        assert_eq!(shared.precedence_start_state(3), Some(shared_root));
    }

    #[test]
    fn union_prediction_stores_remaps_context_ids_before_dfa_union() {
        let atn = two_token_decision_atn();
        let mut shared = PredictionStore::new(&atn);
        let mut local = PredictionStore::new(&atn);
        let mut workspace = PredictionWorkspace::default();

        let distracting = shared.contexts.singleton(EMPTY_CONTEXT, 99);
        let local_context = local.contexts.singleton(EMPTY_CONTEXT, 7);
        assert_eq!(distracting, local_context, "both stores allocate ID 1");

        let mut configs = AtnConfigSet::new();
        configs.add(
            AtnConfig::new(42, 1, local_context, &local.contexts),
            &mut local.contexts,
            &mut workspace,
        );
        local.decision_to_dfa[0].add_state(DfaStateBuilder::new(configs));

        union_prediction_stores(&mut shared, local, &mut workspace);

        let imported = shared.decision_to_dfa[0]
            .states()
            .flat_map(|state| shared.decision_to_dfa[0].configs(state.id()).configs())
            .find(|config| config.state == 42)
            .expect("local DFA config imported");
        assert_ne!(imported.context, local_context);
        assert_eq!(shared.contexts.return_state(imported.context, 0), Some(7));
        imported.assert_store(&shared.contexts);
    }

    #[test]
    fn outer_context_cache_invalidates_with_rule_context_version() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        let first = simulator.intern_prediction_context(1, [7]);
        let cached = simulator.intern_prediction_context(1, [99]);
        let refreshed = simulator.intern_prediction_context(2, [99]);

        assert_eq!(cached, first);
        assert_ne!(refreshed, first);
        assert_eq!(
            simulator.store.contexts.return_state(refreshed, 0),
            Some(99)
        );
        let stats = simulator.prediction_context_stats();
        assert_eq!(stats.outer_context_cache_hits, 1);
        assert_eq!(stats.outer_context_cache_misses, 2);
    }

    #[test]
    fn outer_context_cache_is_simulator_local() {
        let atn = two_token_decision_atn();
        let mut first = ParserAtnSimulator::new(&atn);
        let mut second = ParserAtnSimulator::new(&atn);

        let first_context = first.intern_prediction_context(1, [7]);
        let second_context = second.intern_prediction_context(1, [99]);

        assert_eq!(first.store.contexts.return_state(first_context, 0), Some(7));
        assert_eq!(
            second.store.contexts.return_state(second_context, 0),
            Some(99)
        );
    }

    #[test]
    fn adaptive_predict_reuses_dense_dfa_edges() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));
        assert_eq!(simulator.adaptive_predict(0, [1, 3]), Ok(2));

        let dfa = &simulator.decision_dfas()[0];
        let start = dfa.start_state().expect("start state");
        let after_first = dfa.state(start).and_then(|state| state.edge(1));
        assert!(after_first.is_some());
    }

    #[test]
    fn shared_simulator_reuses_learned_dfa_states() {
        let atn = Box::leak(Box::new(two_token_decision_atn()));
        let learned_states = {
            let mut simulator = ParserAtnSimulator::new_shared(atn);
            assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));
            simulator.decision_dfas()[0].states().len()
        };

        let simulator = ParserAtnSimulator::new_shared(atn);
        assert_eq!(simulator.decision_dfas()[0].states().len(), learned_states);
    }

    #[test]
    fn adaptive_predict_reports_no_viable_alt() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(
            simulator.adaptive_predict(0, [4]),
            Err(ParserAtnSimulatorError::NoViableAlt {
                symbol: 4,
                index: 0
            })
        );
    }

    #[test]
    fn adaptive_predict_marks_sll_conflict_for_full_context() {
        let atn = ambiguous_single_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_predict(0, [1]), Ok(1));
        let prediction = simulator
            .adaptive_predict_info_with_precedence(0, 0, [1])
            .expect("prediction");
        assert_eq!(
            prediction,
            ParserAtnPrediction {
                alt: 1,
                requires_full_context: true,
                has_semantic_context: false,
                diagnostic: Some(ParserAtnPredictionDiagnostic {
                    kind: ParserAtnPredictionDiagnosticKind::Ambiguity,
                    start_index: 0,
                    sll_stop_index: 0,
                    ll_stop_index: 0,
                    conflicting_alts: vec![1, 2],
                    exact: false,
                }),
            }
        );

        let dfa = &simulator.decision_dfas()[0];
        let start = dfa.start_state().expect("start state");
        let target = dfa
            .state(start)
            .and_then(|state| state.edge(1))
            .expect("edge for token 1");
        let state = dfa.state(target).expect("target state");
        assert!(state.is_accept_state());
        assert!(state.requires_full_context());
        assert_eq!(state.prediction(), Some(1));
    }

    #[test]
    fn adaptive_predict_keeps_rule_stop_configs_at_eof() {
        let atn = optional_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_predict(0, [TOKEN_EOF]), Ok(2));
    }

    #[test]
    fn adaptive_predict_treats_repeated_eof_as_epsilon_after_first_eof() {
        let atn = multiple_eof_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_predict(0, [1, TOKEN_EOF]), Ok(1));
    }

    #[test]
    fn adaptive_predict_uses_finished_entry_rule_alt_on_error_edge() {
        let atn = prefix_alt_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_predict(0, [1, 3]), Ok(1));
    }

    #[test]
    fn adaptive_predict_uses_precedence_dfa_start_states() {
        let mut atn = two_token_decision_atn();
        atn.state_mut(1)
            .expect("decision state")
            .precedence_rule_decision = true;
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(
            simulator.adaptive_predict_with_precedence(0, 3, [1, 2]),
            Ok(1)
        );
        assert_eq!(
            simulator.adaptive_predict_with_precedence(0, 7, [1, 3]),
            Ok(2)
        );

        let dfa = &simulator.decision_dfas()[0];
        assert!(dfa.is_precedence_dfa());
        assert!(dfa.precedence_start_state(3).is_some());
        assert!(dfa.precedence_start_state(7).is_some());
    }

    #[test]
    fn adaptive_predict_stream_restores_input_position() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut input = VecIntStream::new(vec![1, 3, TOKEN_EOF]);

        assert_eq!(simulator.adaptive_predict_stream(0, &mut input), Ok(2));
        assert_eq!(input.index(), 0);
        assert_eq!(input.la(1), 1);
    }

    #[test]
    fn adaptive_predict_stream_retries_full_context_conflict() {
        let atn = ambiguous_single_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut input = VecIntStream::new(vec![1, TOKEN_EOF]);

        let prediction = simulator
            .adaptive_predict_stream_info_with_precedence(0, 0, &mut input)
            .expect("prediction");

        assert_eq!(
            prediction,
            ParserAtnPrediction {
                alt: 1,
                requires_full_context: true,
                has_semantic_context: false,
                diagnostic: Some(ParserAtnPredictionDiagnostic {
                    kind: ParserAtnPredictionDiagnosticKind::Ambiguity,
                    start_index: 0,
                    sll_stop_index: 0,
                    ll_stop_index: 0,
                    conflicting_alts: vec![1, 2],
                    exact: false,
                }),
            }
        );
        assert_eq!(input.index(), 0);
    }

    #[test]
    fn context_prediction_reports_context_sensitivity_for_dfa_conflict() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut workspace = PredictionWorkspace::default();
        let mut start_configs = AtnConfigSet::new();
        start_configs.add(
            AtnConfig::new(2, 1, EMPTY_CONTEXT, &simulator.store.contexts),
            &mut simulator.store.contexts,
            &mut workspace,
        );
        let start =
            simulator.store.decision_to_dfa[0].add_state(DfaStateBuilder::new(start_configs));
        simulator.store.decision_to_dfa[0].set_start_state(start);

        let mut accept_configs = AtnConfigSet::new();
        accept_configs.add(
            AtnConfig::new(3, 1, EMPTY_CONTEXT, &simulator.store.contexts).with_semantic_context(
                SemanticContext::Predicate {
                    rule_index: 0,
                    pred_index: 0,
                    context_dependent: false,
                },
            ),
            &mut simulator.store.contexts,
            &mut workspace,
        );
        let mut accept_state = DfaStateBuilder::new(accept_configs);
        accept_state.mark_accept(1);
        accept_state.set_requires_full_context(true);
        accept_state.set_conflicting_alts(vec![1, 2]);
        let accept = simulator.store.decision_to_dfa[0].add_state(accept_state);
        simulator.store.decision_to_dfa[0].add_edge(start, 1, accept);

        let mut input = VecIntStream::new(vec![1, 3, TOKEN_EOF]);
        let prediction = simulator
            .adaptive_predict_stream_info_with_context(0, 0, &mut input, EMPTY_CONTEXT)
            .expect("prediction");

        assert_eq!(
            prediction,
            ParserAtnPrediction {
                alt: 2,
                requires_full_context: true,
                has_semantic_context: false,
                diagnostic: Some(ParserAtnPredictionDiagnostic {
                    kind: ParserAtnPredictionDiagnosticKind::ContextSensitivity,
                    start_index: 0,
                    sll_stop_index: 0,
                    ll_stop_index: 1,
                    conflicting_alts: vec![1, 2],
                    exact: false,
                }),
            }
        );
        assert_eq!(input.index(), 0);
    }

    #[test]
    fn full_context_reach_prefers_longer_match_over_skipped_stop_state() {
        let atn = prefix_alt_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut configs = AtnConfigSet::new_full_context(true);
        let mut merge_cache = PredictionWorkspace::default();
        configs.add(
            AtnConfig::new(2, 1, EMPTY_CONTEXT, &simulator.store.contexts),
            &mut simulator.store.contexts,
            &mut merge_cache,
        );
        configs.add(
            AtnConfig::new(1, 2, EMPTY_CONTEXT, &simulator.store.contexts),
            &mut simulator.store.contexts,
            &mut merge_cache,
        );

        let reach = simulator.compute_reach_set(&configs, 2, true, 0, &mut merge_cache);

        assert_eq!(reach.alts(), std::iter::once(2).collect());
        assert!(simulator.configs_all_reached_rule_stop(&reach));
    }

    #[test]
    fn sll_closure_follows_empty_context_rule_stop_exits() {
        let mut atn = Atn::new(AtnType::Parser, 1);
        add_state(&mut atn, 0, AtnStateKind::RuleStop);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Atom {
                target: 2,
                label: 1,
            });

        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut configs = AtnConfigSet::new_full_context(false);
        let mut merge_cache = PredictionWorkspace::default();
        let mut scratch = ClosureScratch::default();
        let config = AtnConfig::new(0, 2, EMPTY_CONTEXT, &simulator.store.contexts);
        simulator.closure(
            config,
            &mut configs,
            &mut merge_cache,
            &mut scratch,
            ClosureParams {
                precedence: 0,
                collect_predicates: true,
                treat_eof_as_epsilon: false,
            },
        );

        assert_eq!(configs.len(), 1);
        let config = &configs.configs()[0];
        assert_eq!(config.state, 1);
        assert_eq!(config.alt, 2);
        assert_eq!(config.reaches_into_outer_context, 1);
    }

    #[test]
    fn precedence_contexts_are_collected_only_for_start_closure() {
        let mut atn = Atn::new(AtnType::Parser, 1);
        add_state(&mut atn, 0, AtnStateKind::Basic);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        let mut simulator = ParserAtnSimulator::new(&atn);
        let transition = Transition::Precedence {
            target: 1,
            precedence: 2,
        };
        let config = AtnConfig::new(0, 1, EMPTY_CONTEXT, &simulator.store.contexts);

        let sll_start = simulator
            .epsilon_target_config(&config, &transition, 1, true, false)
            .expect("sll start transition");
        assert!(matches!(
            sll_start.semantic_context,
            SemanticContext::Precedence { precedence: 2 }
        ));

        let full_context_start = simulator
            .epsilon_target_config(&config, &transition, 1, true, true)
            .expect("full-context start transition");
        assert!(full_context_start.semantic_context.is_none());

        let reach = simulator
            .epsilon_target_config(&config, &transition, 3, false, false)
            .expect("reach transition");
        assert!(reach.semantic_context.is_none());

        assert!(
            simulator
                .epsilon_target_config(&config, &transition, 3, true, false)
                .is_none()
        );
    }

    #[test]
    fn closure_stops_collecting_predicates_after_action_edge() {
        // ANTLR's `closure_` sets
        // `continueCollecting = collectPredicates && !ActionTransition`, so a
        // predicate reached *after* an action edge is NOT folded into the
        // config's semantic context — it is deferred to parse time (the
        // "action hides predicates" rule). Build `0 -Action-> 1 -Pred-> 2` and
        // assert the closure config carries NO semantic context.
        let mut atn = Atn::new(AtnType::Parser, 1);
        add_state(&mut atn, 0, AtnStateKind::Basic);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        add_state(&mut atn, 3, AtnStateKind::Basic);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Action {
                target: 1,
                rule_index: 0,
                action_index: Some(0),
                context_dependent: false,
            });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Predicate {
                target: 2,
                rule_index: 0,
                pred_index: 0,
                context_dependent: false,
            });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 3,
                label: 1,
            });

        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut configs = AtnConfigSet::new();
        let mut merge_cache = PredictionWorkspace::default();
        let mut scratch = ClosureScratch::default();
        let config = AtnConfig::new(0, 1, EMPTY_CONTEXT, &simulator.store.contexts);
        simulator.closure(
            config,
            &mut configs,
            &mut merge_cache,
            &mut scratch,
            ClosureParams {
                precedence: 0,
                collect_predicates: true,
                treat_eof_as_epsilon: false,
            },
        );

        // The config that stops at state 2 (post-predicate, awaiting the atom)
        // must NOT carry the predicate — the action edge turned collection off.
        let at_two = configs
            .configs()
            .iter()
            .find(|config| config.state == 2)
            .expect("config at state 2");
        assert!(
            at_two.semantic_context.is_none(),
            "predicate after an action edge must not be collected during prediction"
        );

        // Control: the SAME predicate reached WITHOUT an intervening action edge
        // IS collected (so the assertion above is about the action edge, not a
        // blanket failure to collect predicates).
        let direct_config = AtnConfig::new(1, 1, EMPTY_CONTEXT, &simulator.store.contexts);
        let direct = simulator
            .epsilon_target_config(
                &direct_config,
                &Transition::Predicate {
                    target: 2,
                    rule_index: 0,
                    pred_index: 0,
                    context_dependent: false,
                },
                0,
                true,
                false,
            )
            .expect("predicate transition");
        assert!(matches!(
            direct.semantic_context,
            SemanticContext::Predicate { pred_index: 0, .. }
        ));
    }

    #[test]
    fn reach_set_skips_closure_for_unique_intermediate_alt() {
        let mut atn = Atn::new(AtnType::Parser, 1);
        add_state(&mut atn, 0, AtnStateKind::Basic);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Atom {
                target: 1,
                label: 7,
            });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });

        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut configs = AtnConfigSet::new_full_context(false);
        let mut merge_cache = PredictionWorkspace::default();
        configs.add(
            AtnConfig::new(0, 1, EMPTY_CONTEXT, &simulator.store.contexts),
            &mut simulator.store.contexts,
            &mut merge_cache,
        );

        let reach = simulator.compute_reach_set(&configs, 7, false, 0, &mut merge_cache);

        assert_eq!(reach.len(), 1);
        assert_eq!(reach.configs()[0].state, 1);
    }

    #[test]
    fn semantic_context_flag_is_scoped_to_predicted_alt() {
        let mut arena = ContextArena::new();
        let mut workspace = PredictionWorkspace::default();
        let mut configs = AtnConfigSet::new();
        configs.add(
            AtnConfig::new(1, 1, EMPTY_CONTEXT, &arena),
            &mut arena,
            &mut workspace,
        );
        configs.add(
            AtnConfig::new(2, 2, EMPTY_CONTEXT, &arena).with_semantic_context(
                SemanticContext::Predicate {
                    rule_index: 0,
                    pred_index: 0,
                    context_dependent: false,
                },
            ),
            &mut arena,
            &mut workspace,
        );

        assert!(!configs_have_semantic_context_for_alt(&configs, 1));
        assert!(configs_have_semantic_context_for_alt(&configs, 2));
    }

    #[test]
    fn adaptive_predict_prefers_non_greedy_exit_before_consuming() {
        let atn = non_greedy_optional_exit_first_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_predict(0, [1, TOKEN_EOF]), Ok(1));
    }

    #[test]
    fn left_recursive_loop_entry_drop_requires_same_rule_return() {
        let atn = left_recursive_loop_entry_atn();
        let loop_entry = atn.state(1).expect("loop entry");
        let mut contexts = ContextArena::new();
        let same_rule_context = contexts.singleton(EMPTY_CONTEXT, 4);
        let other_rule_context = contexts.singleton(EMPTY_CONTEXT, 5);

        assert!(can_drop_left_recursive_loop_entry_edge(
            &atn,
            loop_entry,
            &contexts,
            same_rule_context
        ));
        assert!(!can_drop_left_recursive_loop_entry_edge(
            &atn,
            loop_entry,
            &contexts,
            other_rule_context
        ));
        assert!(!can_drop_left_recursive_loop_entry_edge(
            &atn,
            loop_entry,
            &contexts,
            EMPTY_CONTEXT
        ));
    }

    fn two_token_decision_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 3);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::BlockStart);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        add_state(&mut atn, 3, AtnStateKind::Basic);
        add_state(&mut atn, 4, AtnStateKind::Basic);
        add_state(&mut atn, 5, AtnStateKind::Basic);
        add_state(&mut atn, 6, AtnStateKind::BlockEnd);
        add_state(&mut atn, 7, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![7]);
        atn.add_decision_state(1);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 4 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 3,
                label: 1,
            });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Atom {
                target: 6,
                label: 2,
            });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Atom {
                target: 5,
                label: 1,
            });
        atn.state_mut(5)
            .expect("state 5")
            .add_transition(Transition::Atom {
                target: 6,
                label: 3,
            });
        atn.state_mut(6)
            .expect("state 6")
            .add_transition(Transition::Epsilon { target: 7 });
        atn
    }

    fn optional_token_decision_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 1);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::BlockStart);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        add_state(&mut atn, 3, AtnStateKind::BlockEnd);
        add_state(&mut atn, 4, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![4]);
        atn.add_decision_state(1);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 3 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 3,
                label: 1,
            });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Epsilon { target: 4 });
        atn
    }

    fn non_greedy_optional_exit_first_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 1);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::BlockStart);
        add_state(&mut atn, 2, AtnStateKind::BlockEnd);
        add_state(&mut atn, 3, AtnStateKind::Basic);
        add_state(&mut atn, 4, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![4]);
        atn.add_decision_state(1);
        atn.state_mut(1).expect("state 1").non_greedy = true;
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 3 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Epsilon { target: 4 });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Atom {
                target: 4,
                label: 1,
            });
        atn
    }

    fn ambiguous_single_token_decision_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 1);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::BlockStart);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        add_state(&mut atn, 3, AtnStateKind::Basic);
        add_state(&mut atn, 4, AtnStateKind::Basic);
        add_state(&mut atn, 5, AtnStateKind::Basic);
        add_state(&mut atn, 6, AtnStateKind::BlockEnd);
        add_state(&mut atn, 7, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![7]);
        atn.add_decision_state(1);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 4 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 3,
                label: 1,
            });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Epsilon { target: 6 });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Atom {
                target: 5,
                label: 1,
            });
        atn.state_mut(5)
            .expect("state 5")
            .add_transition(Transition::Epsilon { target: 6 });
        atn.state_mut(6)
            .expect("state 6")
            .add_transition(Transition::Epsilon { target: 7 });
        atn
    }

    fn prefix_alt_decision_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 3);
        add_state(&mut atn, 0, AtnStateKind::BlockStart);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        add_state(&mut atn, 2, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![2]);
        atn.add_decision_state(0);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Atom {
                target: 2,
                label: 1,
            });
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Atom {
                target: 1,
                label: 1,
            });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Atom {
                target: 2,
                label: 2,
            });
        atn
    }

    fn multiple_eof_decision_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 2);
        for state_number in 0..=10 {
            let kind = match state_number {
                0 => AtnStateKind::RuleStart,
                1 => AtnStateKind::BlockStart,
                7 => AtnStateKind::BlockEnd,
                10 => AtnStateKind::RuleStop,
                _ => AtnStateKind::Basic,
            };
            add_state(&mut atn, state_number, kind);
        }
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![10]);
        atn.add_decision_state(1);
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 4 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 3,
                label: 1,
            });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Epsilon { target: 7 });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Atom {
                target: 5,
                label: 1,
            });
        atn.state_mut(5)
            .expect("state 5")
            .add_transition(Transition::Atom {
                target: 6,
                label: 2,
            });
        atn.state_mut(6)
            .expect("state 6")
            .add_transition(Transition::Epsilon { target: 7 });
        atn.state_mut(7)
            .expect("state 7")
            .add_transition(Transition::Epsilon { target: 8 });
        atn.state_mut(8)
            .expect("state 8")
            .add_transition(Transition::Atom {
                target: 9,
                label: TOKEN_EOF,
            });
        atn.state_mut(9)
            .expect("state 9")
            .add_transition(Transition::Atom {
                target: 10,
                label: TOKEN_EOF,
            });
        atn
    }

    fn left_recursive_loop_entry_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 1);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::StarLoopEntry);
        add_state(&mut atn, 2, AtnStateKind::BlockStart);
        add_state(&mut atn, 3, AtnStateKind::BlockEnd);
        add_state(&mut atn, 4, AtnStateKind::Basic);
        atn.add_state(AtnState::new(5, AtnStateKind::Basic).with_rule_index(1));
        add_state(&mut atn, 6, AtnStateKind::LoopEnd);
        add_state(&mut atn, 7, AtnStateKind::RuleStop);
        atn.state_mut(1)
            .expect("loop entry")
            .precedence_rule_decision = true;
        atn.state_mut(2).expect("block start").end_state = Some(3);
        atn.state_mut(1)
            .expect("loop entry")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(1)
            .expect("loop entry")
            .add_transition(Transition::Epsilon { target: 6 });
        atn.state_mut(4)
            .expect("same-rule return")
            .add_transition(Transition::Epsilon { target: 3 });
        atn.state_mut(5)
            .expect("other-rule return")
            .add_transition(Transition::Epsilon { target: 3 });
        atn
    }

    fn add_state(atn: &mut Atn, state_number: usize, kind: AtnStateKind) {
        atn.add_state(AtnState::new(state_number, kind).with_rule_index(0));
    }

    #[derive(Debug)]
    struct VecIntStream {
        symbols: Vec<i32>,
        index: usize,
    }

    impl VecIntStream {
        fn new(symbols: Vec<i32>) -> Self {
            Self { symbols, index: 0 }
        }
    }

    impl IntStream for VecIntStream {
        fn consume(&mut self) {
            if self.la(1) != TOKEN_EOF {
                self.index += 1;
            }
        }

        fn la(&mut self, offset: isize) -> i32 {
            if offset <= 0 {
                return 0;
            }
            let offset = offset.cast_unsigned() - 1;
            self.symbols
                .get(self.index + offset)
                .copied()
                .unwrap_or(TOKEN_EOF)
        }

        fn index(&self) -> usize {
            self.index
        }

        fn seek(&mut self, index: usize) {
            self.index = index;
        }

        fn size(&self) -> usize {
            self.symbols.len()
        }
    }
}
