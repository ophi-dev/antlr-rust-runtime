use crate::atn::AtnStateKind;
use crate::atn::parser_atn::{
    ParserAtn as Atn, ParserAtnState as AtnState, ParserTransition, ParserTransitionKind,
};
#[cfg(test)]
use crate::atn::parser_atn::{ParserAtnBuilder, ParserTransitionSpec};
use crate::dfa::{
    DfaStateBuilder, DfaStateId, NO_DFA_STATE, ParserDfa, ParserDfaStateView, ParserDfaStats,
};
use crate::int_stream::IntStream;
use crate::prediction::{
    AtnConfig, AtnConfigSet, ContextArena, ContextId, EMPTY_CONTEXT, EMPTY_RETURN_STATE,
    PredictionContextStats, PredictionFxHasher, PredictionPredicateCall,
    PredictionSemanticProvenanceArena, PredictionSemanticProvenanceId, PredictionWorkspace,
    SemanticContext, all_subsets_conflict, all_subsets_equal, conflicting_alt_subsets,
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
    /// Accept states treated as provisional by the latest direct prediction.
    /// Generated SLL still uses their stored accept metadata.
    deferred_accept_states: FxHashSet<(usize, DfaStateId)>,
    shared_cache_key: Option<usize>,
    shared_cache_generation: u64,
    has_trained_decision: bool,
    measure_adaptive_work: bool,
    adaptive_calls: usize,
    adaptive_closure_work: usize,
    /// Java's `LL_EXACT_AMBIG_DETECTION`: the full-context loop keeps
    /// consuming past "resolves to one viable alt" conflicts until every
    /// `(state, context)` subset conflicts over the same alt set.
    exact_ambig_detection: bool,
    /// Memoized full-context resolutions. Upstream re-runs the LL simulation
    /// on every visit to a `requires_full_context` DFA state; grammars with
    /// keyword/identifier-style true ambiguities (Avro IDL's `nullableType`,
    /// SQL non-reserved keywords) pay that on every occurrence. Under the
    /// memo gate — no predicate transitions in the ATN — the LL result is a
    /// pure function of the decision, precedence, interned caller context,
    /// and the token window the loop read, so identical occurrences replay
    /// the recorded resolution. The gate carries that purity claim: general
    /// ANTLR full-context closure can also consult parser state through
    /// `predTransition`, which is exactly what the gate excludes.
    full_context_memo: HashMap<
        FullContextMemoKey,
        Vec<FullContextMemoEntry>,
        BuildHasherDefault<PredictionFxHasher>,
    >,
    full_context_memo_len: usize,
    /// Whether the memo is sound for this ATN: predicates make prediction
    /// outcomes depend on caller-side evaluation, so any semantic transition
    /// disables memoization entirely. Computed lazily on first retry.
    full_context_memo_gate: Option<bool>,
    /// Semantic configurations that survived the most recent prediction.
    ///
    /// The simulator defers predicate evaluation to the parser because hooks
    /// need live parser state. Keeping the surviving alternative/context pairs
    /// lets the committed parser evaluate only simulator-viable paths.
    prediction_semantic_candidates: Vec<CompactParserSemanticCandidate>,
    /// Whether ATN configs retain rule-call paths for parameterized predicates.
    ///
    /// This is enabled only by the committed parser when generated rule
    /// argument metadata exists, keeping ordinary prediction configs compact.
    track_prediction_rule_calls: bool,
    semantic_provenance: Option<Box<PredictionSemanticProvenanceArena>>,
}

#[derive(Clone, Copy, Debug)]
struct CachedOuterContext {
    rule_context_version: usize,
    context: ContextId,
}

/// Lookup key for one memoized full-context resolution.
///
/// The first window token joins the key so a probe is one hash lookup in
/// the common case (distinct keyword per resolution) instead of a scan.
///
/// `outer_context` is the interned FULL caller stack, which bounds the hit
/// rate: the same construct at rule-nesting depth 3 and depth 4 has
/// different `ContextId`s and misses. Flat-ish grammars (Avro IDL: ~155
/// unique contexts against 2,100 retries) hit almost always; deeply
/// recursive grammars re-derive once per distinct nesting shape.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FullContextMemoKey {
    decision: usize,
    precedence: i32,
    outer_context: ContextId,
    first_symbol: i32,
}

/// One memoized full-context resolution.
///
/// `window_tail` is the visible-token sequence the recorded LL loop read
/// after the keyed first symbol, so a hit replays only when the upcoming
/// input matches token-for-token — identical decision + precedence +
/// interned caller context + read window is literally the same computation.
///
/// `prediction.stop_index` holds the RECORDING run's absolute index and is
/// stale for any other occurrence — the probe unconditionally overwrites it
/// from the live cursor before returning a replay. Do not read it directly.
#[derive(Clone, Debug)]
struct FullContextMemoEntry {
    window_tail: Vec<i32>,
    prediction: FullContextPrediction,
}

/// Memoized windows above this many visible tokens are not recorded: long
/// ambiguous prefixes are rare, and verifying a hit costs a token compare
/// per window token.
const FULL_CONTEXT_MEMO_MAX_WINDOW: usize = 16;
/// Total memo entries per simulator. Contexts are interned per parse and
/// real grammars produce a few hundred; the bound only guards adversarial
/// context churn.
const FULL_CONTEXT_MEMO_MAX_ENTRIES: usize = 4096;

/// ATN-static memo gate, cached per thread by ATN identity.
///
/// The gate is a property of the ATN alone, but generated parsers build a
/// simulator per parser instance, so a per-simulator cache would rescan the
/// ATN on the first LL retry of every parse — pure cost for grammars the
/// gate turns off. Keyed like `SHARED_PREDICTION_STORES`.
fn atn_has_predicate_transition(atn: &Atn) -> bool {
    thread_local! {
        static GATES: RefCell<HashMap<usize, bool, BuildHasherDefault<PredictionFxHasher>>> =
            RefCell::new(HashMap::default());
    }
    let ptr: *const Atn = atn;
    let key = ptr as usize;
    GATES.with(|gates| {
        *gates.borrow_mut().entry(key).or_insert_with(|| {
            (0..atn.state_count()).any(|state_number| {
                atn.state(state_number).is_some_and(|state| {
                    state
                        .transitions()
                        .into_iter()
                        .any(|transition| transition.kind() == ParserTransitionKind::Predicate)
                })
            })
        })
    })
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

#[derive(Debug, Default)]
struct SharedPredictionStore {
    generation: u64,
    store: Option<PredictionStore>,
}

thread_local! {
    static SHARED_PREDICTION_STORES: RefCell<HashMap<usize, SharedPredictionStore>> =
        RefCell::new(HashMap::new());
}

fn clear_shared_prediction_store(key: usize) -> u64 {
    SHARED_PREDICTION_STORES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let shared = cache.entry(key).or_default();
        shared.generation = shared.generation.wrapping_add(1);
        shared.store = None;
        shared.generation
    })
}

const ADAPTIVE_ATN_PREFERENCE_MIN_CALLS: usize = 32;
const ADAPTIVE_ATN_PREFERENCE_MIN_CLOSURE_WORK_PER_CALL: usize = 256;
const ADAPTIVE_ATN_PREFERENCE_DECISIVE_CLOSURE_WORK_PER_CALL: usize = 512;

const fn adaptive_prediction_has_work_density(
    calls: usize,
    closure_work: usize,
    minimum_closure_work_per_call: usize,
) -> bool {
    calls >= ADAPTIVE_ATN_PREFERENCE_MIN_CALLS
        && closure_work >= calls.saturating_mul(minimum_closure_work_per_call)
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ParserSemanticCandidate {
    pub(crate) alt: usize,
    pub(crate) context: SemanticContext,
    pub(crate) predicate_calls: Vec<PredictionPredicateCall>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompactParserSemanticCandidate {
    alt: usize,
    context: SemanticContext,
    semantic_provenance: PredictionSemanticProvenanceId,
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

#[derive(Clone, Debug)]
struct PreviousGoodAlt {
    alt: usize,
    configs: Vec<AtnConfig>,
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
    semantic_candidates: Vec<CompactParserSemanticCandidate>,
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
        semantic_candidates: semantic_prediction_candidates(configs),
    }
}

fn semantic_prediction_candidates(configs: &AtnConfigSet) -> Vec<CompactParserSemanticCandidate> {
    if !configs.has_semantic_context() {
        return Vec::new();
    }
    let mut candidates = configs
        .configs()
        .iter()
        .map(|config| CompactParserSemanticCandidate {
            alt: config.alt,
            context: config.semantic_context.clone(),
            semantic_provenance: config.semantic_provenance_id(),
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    candidates
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ClosureConfigKey {
    state: usize,
    alt: usize,
    semantic_context: SemanticContext,
    context_and_provenance: u64,
}

impl From<&AtnConfig> for ClosureConfigKey {
    fn from(config: &AtnConfig) -> Self {
        Self {
            state: config.state,
            alt: config.alt,
            semantic_context: config.semantic_context.clone(),
            context_and_provenance: u64::from(config.context.compact())
                | (u64::from(config.semantic_provenance_and_flags()) << 32),
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
        .enumerate()
        .map(|(decision, state)| {
            let mut dfa = ParserDfa::with_max_token_type(state, decision, atn.max_token_type());
            if atn
                .state(state)
                .is_some_and(AtnState::precedence_rule_decision)
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
        let published = SHARED_PREDICTION_STORES.with(|cache| {
            let mut cache = cache.borrow_mut();
            let shared = cache.entry(key).or_default();
            if shared.generation != self.shared_cache_generation {
                return false;
            }
            if let Some(shared_store) = shared.store.as_mut() {
                union_prediction_stores(shared_store, store, &mut self.workspace);
            } else {
                shared.store = Some(store);
            }
            true
        });
        #[cfg(feature = "perf-counters")]
        if published {
            crate::perf::record_dfa_cache_publication(
                publication_started.elapsed().as_nanos(),
                published_states,
            );
        }
        #[cfg(not(feature = "perf-counters"))]
        let _ = published;
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
            deferred_accept_states: FxHashSet::default(),
            shared_cache_key: None,
            shared_cache_generation: 0,
            has_trained_decision: false,
            measure_adaptive_work: false,
            adaptive_calls: 0,
            adaptive_closure_work: 0,
            exact_ambig_detection: false,
            full_context_memo: HashMap::default(),
            full_context_memo_len: 0,
            full_context_memo_gate: None,
            prediction_semantic_candidates: Vec::new(),
            track_prediction_rule_calls: false,
            semantic_provenance: None,
        }
    }

    /// Resets transient simulator state while retaining learned decision DFAs.
    pub fn reset(&mut self) {
        self.measure_adaptive_work = false;
        self.adaptive_calls = 0;
        self.adaptive_closure_work = 0;
        self.outer_context_cache = None;
        self.deferred_accept_states.clear();
        self.prediction_semantic_candidates.clear();
        self.workspace.reset();
    }

    /// Clears this simulator's learned decision DFAs.
    ///
    /// Shared simulators also invalidate the thread-local cache generation so
    /// an overlapping stale simulator cannot republish pre-clear states later.
    pub fn clear_dfa(&mut self) {
        self.store = PredictionStore::new(self.atn);
        if let Some(semantic_provenance) = self.semantic_provenance.as_mut() {
            **semantic_provenance = PredictionSemanticProvenanceArena::default();
        }
        // The memo keys entries by ContextId into the store's arena the
        // line above just replaced; stale IDs would alias fresh contexts.
        self.full_context_memo.clear();
        self.full_context_memo_len = 0;
        self.reset();
        self.has_trained_decision = false;
        if let Some(key) = self.shared_cache_key {
            self.shared_cache_generation = clear_shared_prediction_store(key);
        }
    }

    /// Clears the thread-local learned DFA store for a generated parser ATN.
    pub fn clear_shared_dfa(atn: &'static Atn) {
        let ptr: *const Atn = atn;
        clear_shared_prediction_store(ptr as usize);
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
                let source = dfa_state_display(
                    state,
                    self.deferred_accept_states
                        .contains(&(dfa.decision(), state.id())),
                );
                for transition in state.transitions() {
                    let Some(target_state) = dfa.state(transition.target) else {
                        continue;
                    };
                    let label = vocabulary.display_name(transition.symbol);
                    let target = dfa_state_display(
                        target_state,
                        self.deferred_accept_states
                            .contains(&(dfa.decision(), target_state.id())),
                    );
                    let _ = writeln!(out, "{source}-{label}->{target}");
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
        let (store, generation) = SHARED_PREDICTION_STORES.with(|cache| {
            let mut cache = cache.borrow_mut();
            let shared = cache.entry(key).or_default();
            (
                shared
                    .store
                    .take()
                    .unwrap_or_else(|| PredictionStore::new(atn)),
                shared.generation,
            )
        });
        let has_trained_decision = store.decision_to_dfa.iter().any(|dfa| !dfa.is_empty());
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
            deferred_accept_states: FxHashSet::default(),
            shared_cache_key: Some(key),
            shared_cache_generation: generation,
            has_trained_decision,
            measure_adaptive_work: false,
            adaptive_calls: 0,
            adaptive_closure_work: 0,
            exact_ambig_detection: false,
            full_context_memo: HashMap::default(),
            full_context_memo_len: 0,
            full_context_memo_gate: None,
            prediction_semantic_candidates: Vec::new(),
            track_prediction_rule_calls: false,
            semantic_provenance: None,
        }
    }

    pub fn decision_dfas(&self) -> &[ParserDfa] {
        &self.store.decision_to_dfa
    }

    pub(crate) fn prediction_semantic_candidates(&self) -> Vec<ParserSemanticCandidate> {
        self.prediction_semantic_candidates
            .iter()
            .map(|candidate| ParserSemanticCandidate {
                alt: candidate.alt,
                context: candidate.context.clone(),
                predicate_calls: self.semantic_provenance.as_deref().map_or_else(
                    Vec::new,
                    |arena| {
                        arena
                            .predicate_calls(candidate.semantic_provenance)
                            .to_vec()
                    },
                ),
            })
            .collect()
    }

    pub(crate) fn set_track_prediction_rule_calls(&mut self, track: bool) {
        assert!(
            self.shared_cache_key.is_none(),
            "shared prediction simulators use a fixed untracked rule-call mode"
        );
        if self.track_prediction_rule_calls != track {
            assert!(
                !self.has_trained_decision,
                "prediction rule-call tracking mode cannot change after DFA construction"
            );
        }
        self.track_prediction_rule_calls = track;
        if track {
            self.semantic_provenance
                .get_or_insert_with(|| Box::new(PredictionSemanticProvenanceArena::default()));
        } else {
            self.semantic_provenance = None;
        }
    }

    /// Returns adaptive-call and closure-work counters for stable decisions.
    ///
    /// A call contributes only when its decision DFA was already non-empty and
    /// did not learn states, edges, or start mappings during the call. This
    /// excludes both first and incremental population from steady-state work.
    #[doc(hidden)]
    pub const fn adaptive_prediction_work(&self) -> Option<(usize, usize)> {
        if self.has_trained_decision {
            Some((self.adaptive_calls, self.adaptive_closure_work))
        } else {
            None
        }
    }

    /// Reports whether the adaptive-prediction work between two snapshots is
    /// expensive enough to justify trying an ATN-recognizer route.
    #[doc(hidden)]
    pub const fn adaptive_prediction_delta_is_expensive(
        before: (usize, usize),
        after: (usize, usize),
    ) -> bool {
        adaptive_prediction_has_work_density(
            after.0.saturating_sub(before.0),
            after.1.saturating_sub(before.1),
            ADAPTIVE_ATN_PREFERENCE_MIN_CLOSURE_WORK_PER_CALL,
        )
    }

    /// Reports whether a partial adaptive-prediction window has enough work
    /// density to justify abandoning generated parsing before the enclosing
    /// rule invocation completes.
    #[doc(hidden)]
    pub const fn adaptive_prediction_delta_is_decisive(
        before: (usize, usize),
        after: (usize, usize),
    ) -> bool {
        adaptive_prediction_has_work_density(
            after.0.saturating_sub(before.0),
            after.1.saturating_sub(before.1),
            ADAPTIVE_ATN_PREFERENCE_DECISIVE_CLOSURE_WORK_PER_CALL,
        )
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
        self.prediction_semantic_candidates.clear();
        let decision = request.decision;
        let learning_revision = self
            .store
            .decision_to_dfa
            .get(decision)
            .filter(|dfa| !dfa.is_empty())
            .map(ParserDfa::learning_revision);
        let work_start = (self.adaptive_calls, self.adaptive_closure_work);
        self.measure_adaptive_work = learning_revision.is_some();
        if self.measure_adaptive_work {
            self.adaptive_calls = self.adaptive_calls.saturating_add(1);
        }
        let result = self.adaptive_predict_stream_inner_impl(request, input, merge_cache);
        self.measure_adaptive_work = false;
        if let Some(learning_revision) = learning_revision
            && self
                .store
                .decision_to_dfa
                .get(decision)
                .map(ParserDfa::learning_revision)
                != Some(learning_revision)
        {
            (self.adaptive_calls, self.adaptive_closure_work) = work_start;
        }
        self.has_trained_decision |= self
            .store
            .decision_to_dfa
            .get(decision)
            .is_some_and(|dfa| !dfa.is_empty());
        result
    }

    fn adaptive_predict_stream_inner_impl<T: IntStream>(
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
        self.deferred_accept_states
            .retain(|(stored_decision, _)| *stored_decision != decision);
        #[cfg(feature = "perf-counters")]
        crate::perf::record_adaptive_call(decision, force_full_context_retry);
        let Some(decision_state) = self.atn.decision_to_state().get(decision) else {
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
        // The direct interpreter API can continue past a completed prefix, but
        // generated parsers retain standard SLL early termination.
        let track_previous_good_alt = !force_full_context_retry && !sll_probe_only;
        let mut previous_good_alt = None;
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
            if track_previous_good_alt {
                let finished = self
                    .store
                    .decision_to_dfa
                    .get(decision)
                    .map(|dfa| dfa.configs(state_number))
                    .and_then(|configs| self.previous_good_alt(configs));
                if finished.is_some() {
                    previous_good_alt = finished;
                }
            }
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
                let edge = DfaEdge {
                    decision,
                    source_state: state_number,
                };
                let target = match self.compute_target_state(
                    edge,
                    &configs,
                    symbol,
                    precedence,
                    merge_cache,
                ) {
                    Ok(target) => target,
                    Err(ParserAtnSimulatorError::NoViableAlt { symbol, .. }) => {
                        if let Some(fallback) = previous_good_alt.as_ref() {
                            self.add_previous_good_alt_target(edge, symbol, fallback, merge_cache)
                        } else {
                            return Err(ParserAtnSimulatorError::NoViableAlt {
                                symbol,
                                index: input.index(),
                            });
                        }
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
                let defer_unique = track_previous_good_alt
                    && previous_good_alt.is_some()
                    && !prediction.requires_full_context
                    && !self.prediction_reached_decision_entry_rule_stop(
                        DfaEdge {
                            decision,
                            source_state: state_number,
                        },
                        prediction.alt,
                        precedence,
                        symbol,
                        merge_cache,
                    );
                if !defer_unique {
                    return Ok(prediction);
                }
                self.deferred_accept_states.insert((decision, state_number));
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
                    self.prediction_semantic_candidates = semantic_prediction_candidates(&configs);
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
            self.record_prediction_semantic_candidates(decision, state_number);
            return Ok(Some(prediction));
        }
        let Some(info) = self.dfa_prediction_info(decision, state_number) else {
            return Ok(None);
        };
        let prediction = info.prediction;
        let semantic_candidates = self
            .store
            .decision_to_dfa
            .get(decision)
            .map(|dfa| semantic_prediction_candidates(dfa.configs(state_number)))
            .unwrap_or_default();
        self.prediction_semantic_candidates = semantic_candidates;
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
            let memo_allowed = self.full_context_memo_allowed();
            let memo_key = FullContextMemoKey {
                decision,
                precedence,
                outer_context,
                first_symbol: 0,
            };
            // A memo hit leaves the cursor at the replayed stop index —
            // exactly where the fresh LL loop below would have left it.
            if memo_allowed
                && let Some(full_context) = self.probe_full_context_memo(memo_key, input)
            {
                #[cfg(feature = "perf-counters")]
                crate::perf::record_full_context_memo_hit(decision);
                return Ok(Some(self.full_context_retry_prediction(
                    full_context,
                    info.conflicting_alts,
                    start_index,
                    sll_stop_index,
                )));
            }
            let full_context = self.adaptive_predict_full_context(
                decision_state,
                input,
                precedence,
                outer_context,
                merge_cache,
            )?;
            if memo_allowed {
                self.record_full_context_memo(memo_key, start_index, input, &full_context);
            }
            return Ok(Some(self.full_context_retry_prediction(
                full_context,
                info.conflicting_alts,
                start_index,
                sll_stop_index,
            )));
        }
        Ok(Some(prediction))
    }

    /// Builds the prediction (and diagnostic) a full-context retry reports,
    /// shared by the fresh LL run and the memoized replay so both produce
    /// byte-identical diagnostics.
    fn full_context_retry_prediction(
        &mut self,
        full_context: FullContextPrediction,
        sll_conflicting_alts: Vec<usize>,
        start_index: usize,
        sll_stop_index: usize,
    ) -> ParserAtnPrediction {
        let FullContextPrediction {
            mut prediction,
            stop_index,
            resolution,
            semantic_candidates,
        } = full_context;
        self.prediction_semantic_candidates = semantic_candidates;
        let (kind, exact, conflicting_alts) = match resolution {
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
                sll_conflicting_alts,
            ),
        };
        prediction.has_semantic_context = self
            .prediction_semantic_candidates
            .iter()
            .any(|candidate| candidate.alt == prediction.alt && !candidate.context.is_none());
        if conflicting_alts.len() > 1 {
            prediction.diagnostic = Some(ParserAtnPredictionDiagnostic {
                kind,
                start_index,
                sll_stop_index,
                ll_stop_index: stop_index,
                conflicting_alts,
                exact,
            });
        }
        prediction
    }

    fn record_prediction_semantic_candidates(&mut self, decision: usize, state_number: DfaStateId) {
        self.prediction_semantic_candidates = self
            .store
            .decision_to_dfa
            .get(decision)
            .map(|dfa| semantic_prediction_candidates(dfa.configs(state_number)))
            .unwrap_or_default();
    }

    /// Whether full-context memoization is sound for this ATN.
    ///
    /// Predicates make prediction outcomes depend on caller-side evaluation
    /// state, so any predicate transition disables the memo. Action and
    /// precedence transitions do NOT: upstream `ActionTransition.isEpsilon()`
    /// is true ("we are to be ignored by analysis 'cept for predicates") and
    /// never contributes to an LL outcome, and a precedence transition in
    /// full-context mode resolves immediately against the passed precedence
    /// (see the `!full_context` guard in `epsilon_target_config`) — which is
    /// already part of the memo key. Gating on all semantic transitions would
    /// turn the memo off for every grammar with a left-recursive rule.
    ///
    /// Exact-ambiguity detection changes how far the LL loop consumes, so
    /// entries recorded under one mode must not replay under the other —
    /// rather than key the mode, the memo simply stays off in the diagnostic
    /// mode.
    fn full_context_memo_allowed(&mut self) -> bool {
        if self.exact_ambig_detection {
            return false;
        }
        let atn = self.atn;
        *self
            .full_context_memo_gate
            .get_or_insert_with(|| !atn_has_predicate_transition(atn))
    }

    /// Returns the memoized LL resolution whose recorded token window matches
    /// the upcoming input exactly, if any. The caller must have positioned
    /// `input` at the decision's start index.
    ///
    /// The compare walks the stream exactly as the recorded LL loop did
    /// (`la(1)` at each cursor position, `consume` between), so a hit is
    /// literally the same computation replayed: same decision, precedence,
    /// interned caller context, and token sequence. On a hit the cursor is
    /// left at the replayed stop index — where the fresh LL loop would have
    /// left it; on a miss it is restored to the start.
    fn probe_full_context_memo<T: IntStream>(
        &self,
        mut key: FullContextMemoKey,
        input: &mut T,
    ) -> Option<FullContextPrediction> {
        if self.full_context_memo.is_empty() {
            return None;
        }
        let start_index = input.index();
        key.first_symbol = input.la(1);
        let entries = self.full_context_memo.get(&key)?;
        // For the keyword-vs-identifier ambiguity shape, occurrences of the
        // same keyword share `first_symbol`, so a hot decision's entries can
        // funnel into one bucket — distinguished only by their windows. The
        // scan is first-match-wins, which is correct because windows under a
        // key are prefix-free: the LL loop's stop position is a function of
        // key + consumed prefix, so no recorded window can be a strict prefix
        // of another. Each rejected candidate costs its matched-prefix length
        // in `consume`/`la` calls before the cursor restore; the global entry
        // cap bounds the worst case.
        'candidates: for entry in entries {
            for &expected in &entry.window_tail {
                input.consume();
                if input.la(1) != expected {
                    input.seek(start_index);
                    continue 'candidates;
                }
            }
            let mut replay = entry.prediction.clone();
            replay.stop_index = input.index();
            return Some(replay);
        }
        None
    }

    /// Records a fresh full-context resolution for later replay.
    ///
    /// The recorded window re-walks the visible tokens the LL loop read
    /// (`start_index..=stop_index` in cursor positions); windows longer than
    /// [`FULL_CONTEXT_MEMO_MAX_WINDOW`] are skipped, as is everything once
    /// the memo holds [`FULL_CONTEXT_MEMO_MAX_ENTRIES`].
    fn record_full_context_memo<T: IntStream>(
        &mut self,
        mut key: FullContextMemoKey,
        start_index: usize,
        input: &mut T,
        full_context: &FullContextPrediction,
    ) {
        if self.full_context_memo_len >= FULL_CONTEXT_MEMO_MAX_ENTRIES {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_full_context_memo_declined(key.decision);
            return;
        }
        let current = input.index();
        input.seek(start_index);
        key.first_symbol = input.la(1);
        let mut window_tail = Vec::new();
        while input.index() < full_context.stop_index
            && window_tail.len() < FULL_CONTEXT_MEMO_MAX_WINDOW
        {
            input.consume();
            window_tail.push(input.la(1));
        }
        let complete = input.index() >= full_context.stop_index;
        input.seek(current);
        if !complete {
            #[cfg(feature = "perf-counters")]
            crate::perf::record_full_context_memo_declined(key.decision);
            return;
        }
        self.full_context_memo
            .entry(key)
            .or_default()
            .push(FullContextMemoEntry {
                window_tail,
                prediction: full_context.clone(),
            });
        self.full_context_memo_len += 1;
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
            .is_some_and(AtnState::non_greedy)
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
        decision_state: AtnState<'_>,
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
        decision_state: AtnState<'_>,
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
        for (index, transition) in decision_state.transitions().iter().enumerate() {
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
        let max_token_type = self.atn.max_token_type();
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
            for transition in &state.transitions() {
                if transition.matches(symbol, 1, max_token_type) {
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
            .filter(|config| self.config_finished_decision_entry_rule(config))
            .map(|config| config.alt)
            .min()
    }

    fn previous_good_alt(&self, configs: &AtnConfigSet) -> Option<PreviousGoodAlt> {
        let alt = self.alt_that_finished_decision_entry_rule(configs)?;
        let configs = configs
            .configs()
            .iter()
            .filter(|config| config.alt == alt && self.config_finished_decision_entry_rule(config))
            .cloned()
            .collect();
        Some(PreviousGoodAlt { alt, configs })
    }

    fn config_finished_decision_entry_rule(&self, config: &AtnConfig) -> bool {
        config.reaches_into_outer_context > 0
            || self
                .atn
                .state(config.state)
                .is_some_and(AtnState::is_rule_stop)
                && self.store.contexts.has_empty_path(config.context)
    }

    fn add_previous_good_alt_target(
        &mut self,
        edge: DfaEdge,
        symbol: i32,
        fallback: &PreviousGoodAlt,
        merge_cache: &mut PredictionWorkspace,
    ) -> DfaStateId {
        let mut configs = AtnConfigSet::new();
        for config in &fallback.configs {
            configs.add(config.clone(), &mut self.store.contexts, merge_cache);
        }
        let has_semantic_context = configs_have_semantic_context_for_alt(&configs, fallback.alt);
        let mut state = DfaStateBuilder::new(configs);
        state.mark_accept(fallback.alt);
        state.set_has_semantic_context_for_alt(has_semantic_context);
        let target = self.add_dfa_state(edge.decision, state);
        self.store.decision_to_dfa[edge.decision].add_edge(edge.source_state, symbol, target);
        target
    }

    fn prediction_reached_decision_entry_rule_stop(
        &mut self,
        edge: DfaEdge,
        alt: usize,
        precedence: i32,
        symbol: i32,
        merge_cache: &mut PredictionWorkspace,
    ) -> bool {
        let configs = self.store.decision_to_dfa[edge.decision]
            .configs(edge.source_state)
            .clone();
        if self.alt_that_finished_decision_entry_rule(&configs) == Some(alt) {
            return true;
        }
        let closed =
            self.close_intermediate_reach_set(configs, false, precedence, symbol, merge_cache);
        self.alt_that_finished_decision_entry_rule(&closed) == Some(alt)
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
        let max_token_type = self.atn.max_token_type();
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
            let epsilon_only = state.epsilon_only();
            if !epsilon_only {
                configs.add(config.clone(), &mut self.store.contexts, merge_cache);
            }
            for (index, transition) in state.transitions().iter().enumerate() {
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
                let transition_kind = transition.kind();
                if matches!(
                    transition_kind,
                    ParserTransitionKind::Epsilon
                        | ParserTransitionKind::Rule
                        | ParserTransitionKind::Predicate
                        | ParserTransitionKind::Action
                        | ParserTransitionKind::Precedence
                ) {
                    if let Some(mut target) = self.epsilon_target_config(
                        &config,
                        transition,
                        transition_kind,
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
                            collect_predicates && transition_kind != ParserTransitionKind::Action;
                        scratch.stack.push((target, target_collect_predicates));
                    }
                } else if treat_eof_as_epsilon
                    && transition.matches_kind(transition_kind, TOKEN_EOF, 1, max_token_type)
                {
                    scratch.stack.push((
                        config.moved_to(transition.target(), config.context, &self.store.contexts),
                        collect_predicates,
                    ));
                }
            }
        }
        let closure_work = scratch.visited.len();
        if self.measure_adaptive_work {
            self.adaptive_closure_work = self.adaptive_closure_work.saturating_add(closure_work);
        }
        #[cfg(feature = "perf-counters")]
        crate::perf::record_closure(closure_work);
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
            let mut next = config.moved_to(return_state, parent, &self.store.contexts);
            if self.track_prediction_rule_calls {
                next.exit_prediction_rule(
                    self.semantic_provenance
                        .as_deref_mut()
                        .expect("tracked prediction has a provenance arena"),
                );
            }
            stack.push((next, collect_predicates));
        }
        handled_all_paths
    }

    #[allow(clippy::too_many_arguments)]
    fn epsilon_target_config(
        &mut self,
        config: &AtnConfig,
        transition: ParserTransition<'_>,
        transition_kind: ParserTransitionKind,
        precedence: i32,
        collect_predicates: bool,
        full_context: bool,
    ) -> Option<AtnConfig> {
        let semantic_context = match transition_kind {
            ParserTransitionKind::Predicate if collect_predicates => SemanticContext::and(
                config.semantic_context.clone(),
                SemanticContext::Predicate {
                    rule_index: transition.arg0() as usize,
                    pred_index: transition.arg1() as usize,
                    context_dependent: transition.arg2() != 0,
                },
            ),
            ParserTransitionKind::Precedence
                if collect_predicates
                    && i32::from_le_bytes(transition.arg0().to_le_bytes()) < precedence =>
            {
                return None;
            }
            ParserTransitionKind::Precedence if collect_predicates && !full_context => {
                SemanticContext::and(
                    config.semantic_context.clone(),
                    SemanticContext::Precedence {
                        precedence: i32::from_le_bytes(transition.arg0().to_le_bytes()),
                    },
                )
            }
            _ => config.semantic_context.clone(),
        };
        let context = if transition_kind == ParserTransitionKind::Rule {
            self.store
                .contexts
                .singleton(config.context, transition.arg1() as usize)
        } else {
            config.context
        };
        let mut target = config.moved_to(transition.target(), context, &self.store.contexts);
        target.semantic_context = semantic_context;
        if self.track_prediction_rule_calls {
            match transition_kind {
                ParserTransitionKind::Rule => {
                    target.enter_prediction_rule(
                        self.semantic_provenance
                            .as_deref_mut()
                            .expect("tracked prediction has a provenance arena"),
                        config.state,
                        transition.arg0() as usize,
                    );
                }
                ParserTransitionKind::Predicate if collect_predicates => {
                    target.record_prediction_predicate(
                        self.semantic_provenance
                            .as_deref_mut()
                            .expect("tracked prediction has a provenance arena"),
                        transition.arg0() as usize,
                        transition.arg1() as usize,
                    );
                }
                _ => {}
            }
        }
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
    state: AtnState<'_>,
    contexts: &ContextArena,
    context: ContextId,
) -> bool {
    if state.kind() != AtnStateKind::StarLoopEntry
        || !state.precedence_rule_decision()
        || contexts.is_empty(context)
        || contexts.has_empty_path(context)
    {
        return false;
    }
    let Some(rule_index) = state.rule_index() else {
        return false;
    };
    for index in 0..contexts.len(context) {
        let Some(return_state_number) = contexts.return_state(context, index) else {
            return false;
        };
        let Some(return_state) = atn.state(return_state_number) else {
            return false;
        };
        if return_state.rule_index() != Some(rule_index) {
            return false;
        }
    }
    let Some(block_end_state_number) = state
        .transitions()
        .first()
        .and_then(|transition| atn.state(transition.target()))
        .and_then(AtnState::end_state)
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
        if return_state.state_number() == block_end_state_number {
            continue;
        }
        if return_state.transitions().len() != 1
            || !return_state
                .transitions()
                .first()
                .is_some_and(ParserTransition::is_epsilon)
        {
            return false;
        }
        let return_target = return_state
            .transitions()
            .first()
            .expect("single transition checked above")
            .target();
        if return_state.kind() == AtnStateKind::BlockEnd && return_target == state.state_number() {
            continue;
        }
        if return_target == block_end_state_number {
            continue;
        }
        let Some(return_target_state) = atn.state(return_target) else {
            return false;
        };
        if return_target_state.kind() == AtnStateKind::BlockEnd
            && return_target_state.transitions().len() == 1
            && return_target_state
                .transitions()
                .first()
                .is_some_and(ParserTransition::is_epsilon)
            && return_target_state
                .transitions()
                .first()
                .is_some_and(|transition| transition.target() == state.state_number())
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
fn dfa_state_display(state: ParserDfaStateView<'_>, deferred: bool) -> String {
    let mut out = String::new();
    let is_accept = state.is_accept_state() && !deferred;
    if is_accept {
        out.push(':');
    }
    out.push('s');
    out.push_str(&state.id().index().to_string());
    if state.requires_full_context() {
        out.push('^');
    }
    if is_accept {
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
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;
    use crate::atn::AtnStateKind;
    use std::mem::size_of;

    fn finish_atn(builder: ParserAtnBuilder) -> Atn {
        builder.finish().expect("valid packed parser ATN")
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn parser_prediction_hot_path_layouts_stay_compact() {
        assert!(size_of::<ClosureConfigKey>() <= 56);
        assert!(size_of::<CompactParserSemanticCandidate>() <= 48);
    }

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
    fn adaptive_atn_preference_requires_expensive_prediction_delta() {
        assert!(!ParserAtnSimulator::adaptive_prediction_delta_is_expensive(
            (0, 0),
            (ADAPTIVE_ATN_PREFERENCE_MIN_CALLS - 1, usize::MAX),
        ));
        assert!(!ParserAtnSimulator::adaptive_prediction_delta_is_expensive(
            (5, 7),
            (
                5 + ADAPTIVE_ATN_PREFERENCE_MIN_CALLS,
                7 + ADAPTIVE_ATN_PREFERENCE_MIN_CALLS
                    * ADAPTIVE_ATN_PREFERENCE_MIN_CLOSURE_WORK_PER_CALL
                    - 1,
            ),
        ));
        assert!(ParserAtnSimulator::adaptive_prediction_delta_is_expensive(
            (5, 7),
            (
                5 + ADAPTIVE_ATN_PREFERENCE_MIN_CALLS,
                7 + ADAPTIVE_ATN_PREFERENCE_MIN_CALLS
                    * ADAPTIVE_ATN_PREFERENCE_MIN_CLOSURE_WORK_PER_CALL,
            ),
        ));
        assert!(!ParserAtnSimulator::adaptive_prediction_delta_is_decisive(
            (5, 7),
            (
                5 + ADAPTIVE_ATN_PREFERENCE_MIN_CALLS,
                7 + ADAPTIVE_ATN_PREFERENCE_MIN_CALLS
                    * ADAPTIVE_ATN_PREFERENCE_DECISIVE_CLOSURE_WORK_PER_CALL
                    - 1,
            ),
        ));
        assert!(ParserAtnSimulator::adaptive_prediction_delta_is_decisive(
            (5, 7),
            (
                5 + ADAPTIVE_ATN_PREFERENCE_MIN_CALLS,
                7 + ADAPTIVE_ATN_PREFERENCE_MIN_CALLS
                    * ADAPTIVE_ATN_PREFERENCE_DECISIVE_CLOSURE_WORK_PER_CALL,
            ),
        ));
    }

    #[test]
    fn adaptive_atn_preference_excludes_first_population_per_decision() {
        let atn = two_independent_decisions_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_prediction_work(), None);
        assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));
        let after_first_decision = simulator
            .adaptive_prediction_work()
            .expect("one decision is trained");
        assert_eq!(after_first_decision, (0, 0));

        assert_eq!(simulator.adaptive_predict(1, [1, 2]), Ok(1));
        assert_eq!(
            simulator.adaptive_prediction_work(),
            Some(after_first_decision),
            "cold work for another decision must not enter the routing counters"
        );

        assert_eq!(simulator.adaptive_predict(1, [1, 2]), Ok(1));
        let after_warm_decision = simulator
            .adaptive_prediction_work()
            .expect("trained decision work is measurable");
        assert_eq!(after_warm_decision.0, after_first_decision.0 + 1);
    }

    #[test]
    fn adaptive_atn_preference_excludes_incremental_population_per_decision() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));
        let after_first_path = simulator
            .adaptive_prediction_work()
            .expect("the decision is partially trained");
        let transitions_after_first_path = simulator.decision_dfas()[0].stats().transitions;

        assert_eq!(simulator.adaptive_predict(0, [1, 3]), Ok(2));
        assert!(
            simulator.decision_dfas()[0].stats().transitions > transitions_after_first_path,
            "the second input must extend the partially populated DFA"
        );
        assert_eq!(
            simulator.adaptive_prediction_work(),
            Some(after_first_path),
            "incremental DFA construction must not enter the routing counters"
        );

        assert_eq!(simulator.adaptive_predict(0, [1, 3]), Ok(2));
        assert_eq!(
            simulator
                .adaptive_prediction_work()
                .expect("the repeated path is stable")
                .0,
            after_first_path.0 + 1
        );
    }

    #[test]
    fn reset_retains_adaptive_training_and_clear_dfa_cools_it() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        assert_eq!(simulator.adaptive_prediction_work(), None);
        assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));
        assert_eq!(simulator.adaptive_prediction_work(), Some((0, 0)));

        simulator.reset();
        assert_eq!(simulator.adaptive_calls, 0);
        assert_eq!(simulator.adaptive_closure_work, 0);
        assert_eq!(simulator.adaptive_prediction_work(), Some((0, 0)));

        assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));
        assert_eq!(
            simulator
                .adaptive_prediction_work()
                .expect("warmed counters")
                .0,
            1
        );

        simulator.clear_dfa();
        assert_eq!(simulator.adaptive_calls, 0);
        assert_eq!(simulator.adaptive_closure_work, 0);
        assert_eq!(simulator.adaptive_prediction_work(), None);
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
    #[should_panic(expected = "shared prediction simulators use a fixed untracked rule-call mode")]
    fn shared_simulator_rejects_rule_call_tracking_mode_changes() {
        let atn = Box::leak(Box::new(two_token_decision_atn()));
        let mut simulator = ParserAtnSimulator::new_shared(atn);

        simulator.set_track_prediction_rule_calls(true);
    }

    #[test]
    #[should_panic(
        expected = "prediction rule-call tracking mode cannot change after DFA construction"
    )]
    fn simulator_rejects_rule_call_tracking_mode_changes_after_learning() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));

        simulator.set_track_prediction_rule_calls(true);
    }

    #[test]
    fn shared_simulator_preserves_and_clears_prediction_training_state() {
        let atn = Box::leak(Box::new(two_token_decision_atn()));
        {
            let mut simulator = ParserAtnSimulator::new_shared(atn);
            assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));
        }

        {
            let simulator = ParserAtnSimulator::new_shared(atn);
            assert_eq!(simulator.adaptive_prediction_work(), Some((0, 0)));
        }

        ParserAtnSimulator::clear_shared_dfa(atn);
        let simulator = ParserAtnSimulator::new_shared(atn);
        assert_eq!(simulator.adaptive_prediction_work(), None);
    }

    #[test]
    fn overlapping_shared_simulator_treats_an_empty_store_as_cold() {
        let atn = Box::leak(Box::new(two_token_decision_atn()));
        {
            let mut simulator = ParserAtnSimulator::new_shared(atn);
            assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));
        }

        let warmed = ParserAtnSimulator::new_shared(atn);
        assert_eq!(warmed.adaptive_prediction_work(), Some((0, 0)));
        let overlapping = ParserAtnSimulator::new_shared(atn);
        assert_eq!(overlapping.adaptive_prediction_work(), None);
    }

    #[test]
    fn clear_shared_dfa_drops_learned_states() {
        let atn = Box::leak(Box::new(two_token_decision_atn()));
        {
            let mut simulator = ParserAtnSimulator::new_shared(atn);
            assert_eq!(simulator.adaptive_predict(0, [1, 2]), Ok(1));
            assert!(!simulator.decision_dfas()[0].is_empty());
        }

        ParserAtnSimulator::clear_shared_dfa(atn);

        let simulator = ParserAtnSimulator::new_shared(atn);
        assert!(simulator.decision_dfas()[0].is_empty());
    }

    #[test]
    fn clear_dfa_rejects_stale_overlapping_simulator_publication() {
        let atn = Box::leak(Box::new(two_token_decision_atn()));
        let mut current = ParserAtnSimulator::new_shared(atn);
        let mut stale = ParserAtnSimulator::new_shared(atn);
        assert_eq!(stale.adaptive_predict(0, [1, 2]), Ok(1));
        assert!(!stale.decision_dfas()[0].is_empty());

        current.clear_dfa();
        drop(stale);
        drop(current);

        let simulator = ParserAtnSimulator::new_shared(atn);
        assert!(simulator.decision_dfas()[0].is_empty());
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
        insta::assert_debug_snapshot!(
            "adaptive_predict_marks_sll_conflict_for_full_context",
            prediction
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
    fn adaptive_predict_keeps_prefix_alt_until_longer_alt_finishes() {
        let atn = three_token_prefix_alt_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_predict(0, [1, 2, TOKEN_EOF]), Ok(1));
        assert_eq!(simulator.adaptive_predict(0, [1, 2, 1, TOKEN_EOF]), Ok(2));
    }

    #[test]
    fn sll_probe_keeps_unique_alt_early_termination() {
        let atn = three_token_prefix_alt_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        let mut input = VecIntStream::new(vec![1, 2, TOKEN_EOF]);

        let prediction = simulator
            .adaptive_predict_stream_info_sll_probe(0, 0, &mut input)
            .expect("SLL prediction should succeed");

        assert_eq!(prediction.alt, 2);
    }

    #[test]
    fn adaptive_predict_uses_precedence_dfa_start_states() {
        let atn = two_token_decision_atn_with_precedence(true);
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

        insta::assert_debug_snapshot!(
            "adaptive_predict_stream_retries_full_context_conflict",
            prediction
        );
        assert_eq!(input.index(), 0);
    }

    #[test]
    fn full_context_memo_replays_identical_retries() {
        let atn = ambiguous_single_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        // First occurrence: SLL conflicts, the LL loop runs and its
        // resolution is recorded.
        let mut input = VecIntStream::new(vec![1, TOKEN_EOF]);
        let fresh = simulator
            .adaptive_predict_stream_info_with_context(0, 0, &mut input, EMPTY_CONTEXT)
            .expect("fresh prediction");
        assert_eq!(simulator.full_context_memo_len, 1);
        assert_eq!(input.index(), 0, "cursor restored after prediction");

        // Second identical occurrence: the memo replays without running the
        // LL loop, producing a byte-identical prediction (diagnostic
        // included) and identical cursor behavior.
        let replayed = simulator
            .adaptive_predict_stream_info_with_context(0, 0, &mut input, EMPTY_CONTEXT)
            .expect("memoized prediction");
        assert_eq!(replayed, fresh);
        assert_eq!(simulator.full_context_memo_len, 1, "no duplicate entry");
        assert_eq!(input.index(), 0);

        // A different upcoming token sequence misses the memo: a fresh LL
        // run happens (and records its own entry) instead of a stale replay.
        let mut other_input = VecIntStream::new(vec![2, TOKEN_EOF]);
        let other = simulator.adaptive_predict_stream_info_with_context(
            0,
            0,
            &mut other_input,
            EMPTY_CONTEXT,
        );
        // Token 2 has no viable alternative in this ATN — the memo must not
        // have answered for it.
        assert!(other.is_err(), "different window must not replay");

        // A different outer context misses the memo as well.
        let context = simulator.store.contexts.singleton(EMPTY_CONTEXT, 6);
        let mut input = VecIntStream::new(vec![1, TOKEN_EOF]);
        let _ = simulator
            .adaptive_predict_stream_info_with_context(0, 0, &mut input, context)
            .expect("prediction under a different context");
        assert_eq!(
            simulator.full_context_memo_len, 2,
            "distinct context records its own entry"
        );
    }

    #[test]
    fn full_context_memo_walks_multi_token_windows() {
        let atn = ambiguous_three_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        // Fresh LL run consumes the three-token window before resolving; the
        // recorded entry carries a non-empty window tail.
        let mut input = VecIntStream::new(vec![1, 2, 3, TOKEN_EOF]);
        let fresh = simulator
            .adaptive_predict_stream_info_with_context(0, 0, &mut input, EMPTY_CONTEXT)
            .expect("fresh prediction");
        assert_eq!(simulator.full_context_memo_len, 1);
        let recorded_window_len = simulator
            .full_context_memo
            .values()
            .next()
            .and_then(|entries| entries.first())
            .map(|entry| entry.window_tail.len())
            .expect("one recorded entry");
        assert!(
            recorded_window_len >= 1,
            "the LL loop consumed tokens, so the window tail must be non-empty"
        );

        // Identical occurrence replays through the token-for-token compare,
        // byte-identical (stop_index recomputed from the live cursor).
        let replayed = simulator
            .adaptive_predict_stream_info_with_context(0, 0, &mut input, EMPTY_CONTEXT)
            .expect("memoized prediction");
        assert_eq!(replayed, fresh);
        assert_eq!(input.index(), 0, "cursor restored by the caller wrapper");

        // Same first symbol, diverging mid-window: the compare must reject
        // the entry and restore the cursor to the decision start before the
        // fresh LL run happens. Token 9 has no viable alternative, so a
        // (wrong) replay would have returned Ok — the Err proves the miss;
        // the memo also records nothing for the failed occurrence.
        let mut diverging = VecIntStream::new(vec![1, 9, 9, TOKEN_EOF]);
        let result = simulator.adaptive_predict_stream_info_with_context(
            0,
            0,
            &mut diverging,
            EMPTY_CONTEXT,
        );
        assert!(result.is_err(), "mid-window divergence must not replay");
        assert_eq!(simulator.full_context_memo_len, 1);
    }

    #[test]
    fn full_context_memo_stays_off_for_predicated_atns_and_exact_mode() {
        // Exact-ambiguity detection changes how far the LL loop consumes, so
        // the memo must not record or replay in that mode.
        let atn = ambiguous_single_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        simulator.set_exact_ambig_detection(true);
        let mut input = VecIntStream::new(vec![1, TOKEN_EOF]);
        let _ = simulator
            .adaptive_predict_stream_info_with_context(0, 0, &mut input, EMPTY_CONTEXT)
            .expect("prediction");
        assert_eq!(simulator.full_context_memo_len, 0);

        // Predicates make outcomes depend on caller-side evaluation: any
        // semantic transition in the ATN disables the memo entirely.
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::Basic);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        atn.add_transition(
            0,
            ParserTransitionSpec::Predicate {
                target: 1,
                rule_index: 0,
                pred_index: 0,
                context_dependent: false,
            },
        )
        .expect("transition");
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![1])
            .expect("rule stop states");
        let atn = finish_atn(atn);
        let mut simulator = ParserAtnSimulator::new(&atn);
        assert!(!simulator.full_context_memo_allowed());
    }

    #[test]
    fn full_context_memo_allows_action_and_precedence_transitions() {
        // Actions never affect prediction (upstream ActionTransition is
        // epsilon for analysis), and precedence transitions resolve against
        // the precedence already in the memo key — neither disables the memo.
        // Gating on them would turn the memo off for every grammar with a
        // left-recursive rule.
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::Basic);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        atn.add_transition(
            0,
            ParserTransitionSpec::Action {
                target: 1,
                rule_index: 0,
                action_index: Some(0),
                context_dependent: false,
            },
        )
        .expect("transition");
        atn.add_transition(
            1,
            ParserTransitionSpec::Precedence {
                target: 2,
                precedence: 1,
            },
        )
        .expect("transition");
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![2])
            .expect("rule stop states");
        let atn = finish_atn(atn);
        let mut simulator = ParserAtnSimulator::new(&atn);
        assert!(simulator.full_context_memo_allowed());
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

        insta::assert_debug_snapshot!(
            "context_prediction_reports_context_sensitivity_for_dfa_conflict",
            prediction
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
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::RuleStop);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
            .expect("transition");
        atn.add_transition(
            1,
            ParserTransitionSpec::Atom {
                target: 2,
                label: 1,
            },
        )
        .expect("transition");
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![0])
            .expect("rule stop states");
        let atn = finish_atn(atn);

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
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::Basic);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![1])
            .expect("rule stop states");
        atn.add_transition(
            0,
            ParserTransitionSpec::Precedence {
                target: 1,
                precedence: 2,
            },
        )
        .expect("precedence transition");
        let atn = finish_atn(atn);
        let transition = atn
            .state(0)
            .expect("source state")
            .transitions()
            .first()
            .expect("precedence transition");
        let mut simulator = ParserAtnSimulator::new(&atn);
        let config = AtnConfig::new(0, 1, EMPTY_CONTEXT, &simulator.store.contexts);

        let sll_start = simulator
            .epsilon_target_config(&config, transition, transition.kind(), 1, true, false)
            .expect("sll start transition");
        assert!(matches!(
            sll_start.semantic_context,
            SemanticContext::Precedence { precedence: 2 }
        ));

        let full_context_start = simulator
            .epsilon_target_config(&config, transition, transition.kind(), 1, true, true)
            .expect("full-context start transition");
        assert!(full_context_start.semantic_context.is_none());

        let reach = simulator
            .epsilon_target_config(&config, transition, transition.kind(), 3, false, false)
            .expect("reach transition");
        assert!(reach.semantic_context.is_none());

        assert!(
            simulator
                .epsilon_target_config(&config, transition, transition.kind(), 3, true, false)
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
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::Basic);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        add_state(&mut atn, 3, AtnStateKind::Basic);
        atn.add_transition(
            0,
            ParserTransitionSpec::Action {
                target: 1,
                rule_index: 0,
                action_index: Some(0),
                context_dependent: false,
            },
        )
        .expect("transition");
        atn.add_transition(
            1,
            ParserTransitionSpec::Predicate {
                target: 2,
                rule_index: 0,
                pred_index: 0,
                context_dependent: false,
            },
        )
        .expect("transition");
        atn.add_transition(
            2,
            ParserTransitionSpec::Atom {
                target: 3,
                label: 1,
            },
        )
        .expect("transition");
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![3])
            .expect("rule stop states");
        let atn = finish_atn(atn);

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
        let direct_transition = atn
            .state(1)
            .expect("predicate source")
            .transitions()
            .first()
            .expect("predicate transition");
        let direct = simulator
            .epsilon_target_config(
                &direct_config,
                direct_transition,
                direct_transition.kind(),
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
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::Basic);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        atn.add_transition(
            0,
            ParserTransitionSpec::Atom {
                target: 1,
                label: 7,
            },
        )
        .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
            .expect("transition");
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![2])
            .expect("rule stop states");
        let atn = finish_atn(atn);

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
        two_token_decision_atn_with_precedence(false)
    }

    fn two_independent_decisions_atn() -> Atn {
        let mut atn = ParserAtnBuilder::new(3);
        add_two_token_decision_rule(&mut atn, 0, 0);
        add_two_token_decision_rule(&mut atn, 8, 1);
        atn.set_rule_to_start_state(vec![0, 8])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![7, 15])
            .expect("rule stop states");
        finish_atn(atn)
    }

    fn two_token_decision_atn_with_precedence(precedence: bool) -> Atn {
        let mut atn = ParserAtnBuilder::new(3);
        add_two_token_decision_rule(&mut atn, 0, 0);
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![7])
            .expect("rule stop states");
        if precedence {
            atn.set_precedence_rule_decision(1)
                .expect("precedence decision state");
        }
        finish_atn(atn)
    }

    fn add_two_token_decision_rule(atn: &mut ParserAtnBuilder, offset: usize, rule_index: usize) {
        assert_eq!(atn.state_count(), offset);
        for kind in [
            AtnStateKind::RuleStart,
            AtnStateKind::BlockStart,
            AtnStateKind::Basic,
            AtnStateKind::Basic,
            AtnStateKind::Basic,
            AtnStateKind::Basic,
            AtnStateKind::BlockEnd,
            AtnStateKind::RuleStop,
        ] {
            let expected = atn.state_count();
            assert_eq!(
                atn.add_state(kind, Some(rule_index))
                    .expect("state")
                    .index(),
                expected
            );
        }
        atn.add_decision_state(offset + 1).expect("decision state");
        atn.add_transition(offset, ParserTransitionSpec::Epsilon { target: offset + 1 })
            .expect("transition");
        atn.add_transition(
            offset + 1,
            ParserTransitionSpec::Epsilon { target: offset + 2 },
        )
        .expect("transition");
        atn.add_transition(
            offset + 1,
            ParserTransitionSpec::Epsilon { target: offset + 4 },
        )
        .expect("transition");
        atn.add_transition(
            offset + 2,
            ParserTransitionSpec::Atom {
                target: offset + 3,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(
            offset + 3,
            ParserTransitionSpec::Atom {
                target: offset + 6,
                label: 2,
            },
        )
        .expect("transition");
        atn.add_transition(
            offset + 4,
            ParserTransitionSpec::Atom {
                target: offset + 5,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(
            offset + 5,
            ParserTransitionSpec::Atom {
                target: offset + 6,
                label: 3,
            },
        )
        .expect("transition");
        atn.add_transition(
            offset + 6,
            ParserTransitionSpec::Epsilon { target: offset + 7 },
        )
        .expect("transition");
    }

    fn optional_token_decision_atn() -> Atn {
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::BlockStart);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        add_state(&mut atn, 3, AtnStateKind::BlockEnd);
        add_state(&mut atn, 4, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![4])
            .expect("rule stop states");
        atn.add_decision_state(1).expect("decision state");
        atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 3 })
            .expect("transition");
        atn.add_transition(
            2,
            ParserTransitionSpec::Atom {
                target: 3,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(3, ParserTransitionSpec::Epsilon { target: 4 })
            .expect("transition");
        finish_atn(atn)
    }

    fn non_greedy_optional_exit_first_atn() -> Atn {
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::BlockStart);
        add_state(&mut atn, 2, AtnStateKind::BlockEnd);
        add_state(&mut atn, 3, AtnStateKind::Basic);
        add_state(&mut atn, 4, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![4])
            .expect("rule stop states");
        atn.add_decision_state(1).expect("decision state");
        atn.set_non_greedy(1).expect("non-greedy state");
        atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 3 })
            .expect("transition");
        atn.add_transition(2, ParserTransitionSpec::Epsilon { target: 4 })
            .expect("transition");
        atn.add_transition(
            3,
            ParserTransitionSpec::Atom {
                target: 4,
                label: 1,
            },
        )
        .expect("transition");
        finish_atn(atn)
    }

    /// `s : A B C | A B C ;` — both alternatives match the same THREE-token
    /// sequence, so the SLL conflict's full-context retry consumes multiple
    /// tokens before resolving, exercising the memo's window walk (record and
    /// probe) rather than the empty-window fast case.
    fn ambiguous_three_token_decision_atn() -> Atn {
        let mut atn = ParserAtnBuilder::new(3);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::BlockStart);
        // Alternative 1: states 2 -A-> 3 -B-> 4 -C-> 5
        add_state(&mut atn, 2, AtnStateKind::Basic);
        add_state(&mut atn, 3, AtnStateKind::Basic);
        add_state(&mut atn, 4, AtnStateKind::Basic);
        add_state(&mut atn, 5, AtnStateKind::Basic);
        // Alternative 2: states 6 -A-> 7 -B-> 8 -C-> 9
        add_state(&mut atn, 6, AtnStateKind::Basic);
        add_state(&mut atn, 7, AtnStateKind::Basic);
        add_state(&mut atn, 8, AtnStateKind::Basic);
        add_state(&mut atn, 9, AtnStateKind::Basic);
        add_state(&mut atn, 10, AtnStateKind::BlockEnd);
        add_state(&mut atn, 11, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![11])
            .expect("rule stop states");
        atn.add_decision_state(1).expect("decision state");
        atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 6 })
            .expect("transition");
        for (source, target, label) in [
            (2, 3, 1),
            (3, 4, 2),
            (4, 5, 3),
            (6, 7, 1),
            (7, 8, 2),
            (8, 9, 3),
        ] {
            atn.add_transition(source, ParserTransitionSpec::Atom { target, label })
                .expect("transition");
        }
        atn.add_transition(5, ParserTransitionSpec::Epsilon { target: 10 })
            .expect("transition");
        atn.add_transition(9, ParserTransitionSpec::Epsilon { target: 10 })
            .expect("transition");
        atn.add_transition(10, ParserTransitionSpec::Epsilon { target: 11 })
            .expect("transition");
        finish_atn(atn)
    }

    fn ambiguous_single_token_decision_atn() -> Atn {
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::BlockStart);
        add_state(&mut atn, 2, AtnStateKind::Basic);
        add_state(&mut atn, 3, AtnStateKind::Basic);
        add_state(&mut atn, 4, AtnStateKind::Basic);
        add_state(&mut atn, 5, AtnStateKind::Basic);
        add_state(&mut atn, 6, AtnStateKind::BlockEnd);
        add_state(&mut atn, 7, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![7])
            .expect("rule stop states");
        atn.add_decision_state(1).expect("decision state");
        atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 4 })
            .expect("transition");
        atn.add_transition(
            2,
            ParserTransitionSpec::Atom {
                target: 3,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(3, ParserTransitionSpec::Epsilon { target: 6 })
            .expect("transition");
        atn.add_transition(
            4,
            ParserTransitionSpec::Atom {
                target: 5,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(5, ParserTransitionSpec::Epsilon { target: 6 })
            .expect("transition");
        atn.add_transition(6, ParserTransitionSpec::Epsilon { target: 7 })
            .expect("transition");
        finish_atn(atn)
    }

    fn prefix_alt_decision_atn() -> Atn {
        let mut atn = ParserAtnBuilder::new(3);
        add_state(&mut atn, 0, AtnStateKind::BlockStart);
        add_state(&mut atn, 1, AtnStateKind::Basic);
        add_state(&mut atn, 2, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![2])
            .expect("rule stop states");
        atn.add_decision_state(0).expect("decision state");
        atn.add_transition(
            0,
            ParserTransitionSpec::Atom {
                target: 2,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(
            0,
            ParserTransitionSpec::Atom {
                target: 1,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(
            1,
            ParserTransitionSpec::Atom {
                target: 2,
                label: 2,
            },
        )
        .expect("transition");
        finish_atn(atn)
    }

    fn three_token_prefix_alt_decision_atn() -> Atn {
        let mut atn = ParserAtnBuilder::new(2);
        for (state_number, kind) in [
            (0, AtnStateKind::BlockStart),
            (1, AtnStateKind::Basic),
            (2, AtnStateKind::Basic),
            (3, AtnStateKind::Basic),
            (4, AtnStateKind::Basic),
            (5, AtnStateKind::Basic),
            (6, AtnStateKind::RuleStop),
        ] {
            add_state(&mut atn, state_number, kind);
        }
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![6])
            .expect("rule stop states");
        atn.add_decision_state(0).expect("decision state");
        atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
            .expect("transition");
        atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 2 })
            .expect("transition");
        atn.add_transition(
            1,
            ParserTransitionSpec::Atom {
                target: 6,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(
            2,
            ParserTransitionSpec::Atom {
                target: 3,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(
            3,
            ParserTransitionSpec::Atom {
                target: 4,
                label: 2,
            },
        )
        .expect("transition");
        atn.add_transition(
            4,
            ParserTransitionSpec::Atom {
                target: 5,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(5, ParserTransitionSpec::Epsilon { target: 6 })
            .expect("transition");
        finish_atn(atn)
    }

    fn multiple_eof_decision_atn() -> Atn {
        let mut atn = ParserAtnBuilder::new(2);
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
        atn.set_rule_to_start_state(vec![0])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![10])
            .expect("rule stop states");
        atn.add_decision_state(1).expect("decision state");
        atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 4 })
            .expect("transition");
        atn.add_transition(
            2,
            ParserTransitionSpec::Atom {
                target: 3,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(3, ParserTransitionSpec::Epsilon { target: 7 })
            .expect("transition");
        atn.add_transition(
            4,
            ParserTransitionSpec::Atom {
                target: 5,
                label: 1,
            },
        )
        .expect("transition");
        atn.add_transition(
            5,
            ParserTransitionSpec::Atom {
                target: 6,
                label: 2,
            },
        )
        .expect("transition");
        atn.add_transition(6, ParserTransitionSpec::Epsilon { target: 7 })
            .expect("transition");
        atn.add_transition(7, ParserTransitionSpec::Epsilon { target: 8 })
            .expect("transition");
        atn.add_transition(
            8,
            ParserTransitionSpec::Atom {
                target: 9,
                label: TOKEN_EOF,
            },
        )
        .expect("transition");
        atn.add_transition(
            9,
            ParserTransitionSpec::Atom {
                target: 10,
                label: TOKEN_EOF,
            },
        )
        .expect("transition");
        finish_atn(atn)
    }

    fn left_recursive_loop_entry_atn() -> Atn {
        let mut atn = ParserAtnBuilder::new(1);
        add_state(&mut atn, 0, AtnStateKind::RuleStart);
        add_state(&mut atn, 1, AtnStateKind::StarLoopEntry);
        add_state(&mut atn, 2, AtnStateKind::BlockStart);
        add_state(&mut atn, 3, AtnStateKind::BlockEnd);
        add_state(&mut atn, 4, AtnStateKind::Basic);
        assert_eq!(
            atn.add_state(AtnStateKind::Basic, Some(1))
                .expect("state")
                .index(),
            5
        );
        add_state(&mut atn, 6, AtnStateKind::LoopEnd);
        add_state(&mut atn, 7, AtnStateKind::RuleStop);
        atn.set_rule_to_start_state(vec![0, 5])
            .expect("rule start states");
        atn.set_rule_to_stop_state(vec![7, 7])
            .expect("rule stop states");
        atn.set_precedence_rule_decision(1)
            .expect("precedence decision state");
        atn.set_end_state(2, 3).expect("block end state");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
            .expect("transition");
        atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 6 })
            .expect("transition");
        atn.add_transition(4, ParserTransitionSpec::Epsilon { target: 3 })
            .expect("transition");
        atn.add_transition(5, ParserTransitionSpec::Epsilon { target: 3 })
            .expect("transition");
        finish_atn(atn)
    }

    fn add_state(atn: &mut ParserAtnBuilder, state_number: usize, kind: AtnStateKind) {
        assert_eq!(
            atn.add_state(kind, Some(0)).expect("state").index(),
            state_number
        );
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
