use crate::prediction::{AtnConfigSet, ContextArena, ContextId, PredictionFxHasher};
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::mem::size_of;

type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<PredictionFxHasher>>;

const NO_EDGE_INDEX: u32 = u32::MAX;
const NO_PREDICTION: u32 = u32::MAX;
const ACCEPT_STATE: u8 = 1 << 0;
const REQUIRES_FULL_CONTEXT: u8 = 1 << 1;
const HAS_SEMANTIC_CONTEXT: u8 = 1 << 2;

// Across the protected Kotlin, C#, Java, and Trino fixtures, rows with fewer
// than eight edges were overwhelmingly sparse. Above that point, dense rows
// won once at least one eighth of a bounded vocabulary was populated.
const DENSE_MAX_ROW_WIDTH: u32 = 512;
const DENSE_MIN_EDGES: u32 = 8;
const DENSE_DENSITY_DENOMINATOR: u32 = 8;

fn compact_index(index: usize, message: &'static str) -> u32 {
    u32::try_from(index)
        .ok()
        .filter(|value| *value != u32::MAX)
        .expect(message)
}

/// Compact identity for one learned parser-DFA state.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DfaStateId(u32);

pub(crate) const NO_DFA_STATE: DfaStateId = DfaStateId(u32::MAX);

impl DfaStateId {
    fn from_index(index: usize) -> Self {
        Self(compact_index(
            index,
            "parser DFA state count must fit below the u32 sentinel",
        ))
    }

    /// Returns this state's zero-based diagnostic index.
    pub fn index(self) -> usize {
        usize::try_from(self.0).expect("u32 DFA state ID fits in usize")
    }
}

/// Read-only transition exposed by parser-DFA diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DfaTransition {
    pub source: DfaStateId,
    pub symbol: i32,
    pub target: DfaStateId,
}

/// Storage and learning measurements for one parser DFA.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ParserDfaStats {
    pub states: usize,
    pub transitions: usize,
    pub max_row_width: usize,
    pub dense_rows: usize,
    pub sparse_rows: usize,
    pub empty_rows: usize,
    pub dense_slots: usize,
    pub sparse_entries: usize,
    /// Widths <=64, <=128, <=256, <=512, and >512.
    pub row_width_histogram: [usize; 5],
    /// Populated-edge counts 0, 1, 2-3, 4-7, 8-15, and >=16.
    pub populated_edge_histogram: [usize; 6],
    /// Empty, <=1%, <=5%, <=12.5%, <=25%, and >25% populated rows.
    pub edge_density_histogram: [usize; 6],
    pub hot_bytes: usize,
    pub cold_bytes: usize,
    pub states_created: usize,
    pub states_deduplicated: usize,
    pub fingerprint_candidates: usize,
    pub fingerprint_collisions: usize,
}

impl ParserDfaStats {
    pub(crate) fn add_assign(&mut self, other: Self) {
        self.states = self.states.saturating_add(other.states);
        self.transitions = self.transitions.saturating_add(other.transitions);
        self.max_row_width = self.max_row_width.max(other.max_row_width);
        self.dense_rows = self.dense_rows.saturating_add(other.dense_rows);
        self.sparse_rows = self.sparse_rows.saturating_add(other.sparse_rows);
        self.empty_rows = self.empty_rows.saturating_add(other.empty_rows);
        self.dense_slots = self.dense_slots.saturating_add(other.dense_slots);
        self.sparse_entries = self.sparse_entries.saturating_add(other.sparse_entries);
        for (total, value) in self
            .row_width_histogram
            .iter_mut()
            .zip(other.row_width_histogram)
        {
            *total = total.saturating_add(value);
        }
        for (total, value) in self
            .populated_edge_histogram
            .iter_mut()
            .zip(other.populated_edge_histogram)
        {
            *total = total.saturating_add(value);
        }
        for (total, value) in self
            .edge_density_histogram
            .iter_mut()
            .zip(other.edge_density_histogram)
        {
            *total = total.saturating_add(value);
        }
        self.hot_bytes = self.hot_bytes.saturating_add(other.hot_bytes);
        self.cold_bytes = self.cold_bytes.saturating_add(other.cold_bytes);
        self.states_created = self.states_created.saturating_add(other.states_created);
        self.states_deduplicated = self
            .states_deduplicated
            .saturating_add(other.states_deduplicated);
        self.fingerprint_candidates = self
            .fingerprint_candidates
            .saturating_add(other.fingerprint_candidates);
        self.fingerprint_collisions = self
            .fingerprint_collisions
            .saturating_add(other.fingerprint_collisions);
    }
}

/// Opaque learned parser DFA with split hot transition and cold ATN-config data.
#[derive(Debug)]
pub struct ParserDfa {
    decision: usize,
    atn_start_state: usize,
    max_token_type: i32,
    hot: DfaHotTables,
    cold: DfaColdStore,
    interner: DfaStateInterner,
    start_state: DfaStateId,
    precedence_start_states: Vec<DfaStateId>,
    precedence_mode: bool,
    learning: DfaLearningCounters,
}

impl ParserDfa {
    pub fn new(atn_start_state: usize, decision: usize) -> Self {
        Self::with_max_token_type(atn_start_state, decision, 0)
    }

    pub fn with_max_token_type(
        atn_start_state: usize,
        decision: usize,
        max_token_type: i32,
    ) -> Self {
        Self {
            decision,
            atn_start_state,
            max_token_type,
            hot: DfaHotTables::new(max_token_type),
            cold: DfaColdStore::default(),
            interner: DfaStateInterner::default(),
            start_state: NO_DFA_STATE,
            precedence_start_states: Vec::new(),
            precedence_mode: false,
            learning: DfaLearningCounters::default(),
        }
    }

    pub const fn decision(&self) -> usize {
        self.decision
    }

    pub const fn atn_start_state(&self) -> usize {
        self.atn_start_state
    }

    pub const fn max_token_type(&self) -> i32 {
        self.max_token_type
    }

    pub const fn state_count(&self) -> usize {
        self.hot.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.hot.is_empty()
    }

    pub fn states(&self) -> impl ExactSizeIterator<Item = ParserDfaStateView<'_>> {
        (0..self.state_count()).map(|index| ParserDfaStateView {
            dfa: self,
            id: DfaStateId::from_index(index),
        })
    }

    pub fn state(&self, id: DfaStateId) -> Option<ParserDfaStateView<'_>> {
        (id.index() < self.state_count()).then_some(ParserDfaStateView { dfa: self, id })
    }

    pub fn transitions(&self) -> impl Iterator<Item = DfaTransition> + '_ {
        self.states().flat_map(ParserDfaStateView::transitions)
    }

    pub fn start_state(&self) -> Option<DfaStateId> {
        (self.start_state != NO_DFA_STATE).then_some(self.start_state)
    }

    pub(crate) fn set_start_state(&mut self, state: DfaStateId) {
        self.assert_valid_state(state);
        self.start_state = state;
    }

    pub const fn is_precedence_dfa(&self) -> bool {
        self.precedence_mode
    }

    pub(crate) fn set_precedence_dfa(&mut self, precedence_dfa: bool) {
        if self.precedence_mode == precedence_dfa {
            return;
        }
        self.hot.clear();
        self.cold.clear();
        self.interner.clear();
        self.start_state = NO_DFA_STATE;
        self.precedence_start_states.clear();
        self.precedence_mode = precedence_dfa;
        if precedence_dfa {
            let state = self.add_state(DfaStateBuilder::new(AtnConfigSet::new()));
            self.start_state = state;
        }
    }

    pub fn precedence_start_state(&self, precedence: usize) -> Option<DfaStateId> {
        self.precedence_start_states
            .get(precedence)
            .copied()
            .filter(|state| *state != NO_DFA_STATE)
    }

    pub(crate) fn precedence_start_states(&self) -> &[DfaStateId] {
        &self.precedence_start_states
    }

    pub(crate) fn set_precedence_start_state(&mut self, precedence: usize, state: DfaStateId) {
        self.assert_valid_state(state);
        if precedence >= self.precedence_start_states.len() {
            self.precedence_start_states
                .resize(precedence + 1, NO_DFA_STATE);
        }
        self.precedence_start_states[precedence] = state;
    }

    pub fn stats(&self) -> ParserDfaStats {
        let edge_stats = self.hot.edges.stats();
        ParserDfaStats {
            states: self.state_count(),
            transitions: edge_stats.transitions,
            max_row_width: edge_stats.max_row_width,
            dense_rows: edge_stats.dense_rows,
            sparse_rows: edge_stats.sparse_rows,
            empty_rows: edge_stats.empty_rows,
            dense_slots: edge_stats.dense_slots,
            sparse_entries: edge_stats.sparse_entries,
            row_width_histogram: edge_stats.row_width_histogram,
            populated_edge_histogram: edge_stats.populated_edge_histogram,
            edge_density_histogram: edge_stats.edge_density_histogram,
            hot_bytes: self.hot.retained_bytes()
                + self.interner.retained_bytes()
                + self.precedence_start_states.capacity() * size_of::<DfaStateId>(),
            cold_bytes: self.cold.retained_bytes(),
            states_created: self.learning.states_created,
            states_deduplicated: self.learning.states_deduplicated,
            fingerprint_candidates: self.learning.fingerprint_candidates,
            fingerprint_collisions: self.learning.fingerprint_collisions,
        }
    }

    pub(crate) fn add_state(&mut self, state: DfaStateBuilder) -> DfaStateId {
        let fingerprint = state.configs.fingerprint();
        if let Some(existing) = self.find_state(fingerprint, &state.configs) {
            self.learning.states_deduplicated = self.learning.states_deduplicated.saturating_add(1);
            #[cfg(feature = "perf-counters")]
            crate::perf::record_dfa_state_deduplicated();
            return existing;
        }
        self.insert_state_with_fingerprint(state, fingerprint)
    }

    pub(crate) fn insert_state(&mut self, state: DfaStateBuilder) -> DfaStateId {
        let fingerprint = state.configs.fingerprint();
        self.insert_state_with_fingerprint(state, fingerprint)
    }

    fn insert_state_with_fingerprint(
        &mut self,
        state: DfaStateBuilder,
        fingerprint: u64,
    ) -> DfaStateId {
        let id = DfaStateId::from_index(self.state_count());
        let DfaStateBuilder {
            mut configs,
            prediction,
            requires_full_context,
            conflicting_alts,
            has_semantic_context_for_alt,
        } = state;
        configs.set_readonly(true);
        self.hot.push_state(
            prediction,
            requires_full_context,
            has_semantic_context_for_alt,
        );
        self.cold.push(configs, conflicting_alts);
        self.interner.insert(fingerprint, id);
        self.learning.states_created = self.learning.states_created.saturating_add(1);
        #[cfg(feature = "perf-counters")]
        crate::perf::record_dfa_state_created();
        id
    }

    pub(crate) fn state_id_for_configs(&mut self, configs: &AtnConfigSet) -> Option<DfaStateId> {
        let state = self.find_state(configs.fingerprint(), configs);
        if state.is_some() {
            self.learning.states_deduplicated = self.learning.states_deduplicated.saturating_add(1);
            #[cfg(feature = "perf-counters")]
            crate::perf::record_dfa_state_deduplicated();
        }
        state
    }

    fn find_state(&mut self, fingerprint: u64, configs: &AtnConfigSet) -> Option<DfaStateId> {
        let mut candidate = self.interner.head(fingerprint);
        while candidate != NO_DFA_STATE {
            self.learning.fingerprint_candidates =
                self.learning.fingerprint_candidates.saturating_add(1);
            #[cfg(feature = "perf-counters")]
            crate::perf::record_dfa_fingerprint_candidate();
            if self.configs(candidate) == configs {
                return Some(candidate);
            }
            self.learning.fingerprint_collisions =
                self.learning.fingerprint_collisions.saturating_add(1);
            #[cfg(feature = "perf-counters")]
            crate::perf::record_dfa_fingerprint_collision();
            candidate = self.interner.next(candidate);
        }
        None
    }

    pub(crate) fn edge(&self, source: DfaStateId, symbol: i32) -> Option<DfaStateId> {
        self.hot.edges.target(source, symbol)
    }

    pub(crate) fn add_edge(&mut self, source: DfaStateId, symbol: i32, target: DfaStateId) {
        self.assert_valid_state(source);
        self.assert_valid_state(target);
        self.hot.edges.add(source, symbol, target);
    }

    pub(crate) fn configs(&self, state: DfaStateId) -> &AtnConfigSet {
        &self.cold.configs[state.index()]
    }

    pub(crate) fn conflicting_alts(&self, state: DfaStateId) -> &[usize] {
        &self.cold.extras[state.index()].conflicting_alts
    }

    pub(crate) fn clone_state_without_edges(&self, state: DfaStateId) -> DfaStateBuilder {
        let index = state.index();
        DfaStateBuilder {
            configs: self.cold.configs[index].clone(),
            prediction: self.hot.prediction(state),
            requires_full_context: self.hot.has_flag(state, REQUIRES_FULL_CONTEXT),
            conflicting_alts: self.cold.extras[index].conflicting_alts.clone(),
            has_semantic_context_for_alt: self.hot.has_flag(state, HAS_SEMANTIC_CONTEXT),
        }
    }

    pub(crate) fn remap_contexts(&mut self, remap: &[ContextId], arena: &ContextArena) {
        for configs in &mut self.cold.configs {
            configs.remap_contexts(remap, arena);
        }
        self.interner.rebuild(&self.cold.configs);
    }

    fn assert_valid_state(&self, state: DfaStateId) {
        assert_ne!(state, NO_DFA_STATE, "DFA state ID cannot be the sentinel");
        assert!(
            state.index() < self.state_count(),
            "DFA state ID must index aligned hot/cold storage"
        );
    }
}

/// Borrowing diagnostic view over one parser-DFA state.
#[derive(Clone, Copy)]
pub struct ParserDfaStateView<'a> {
    dfa: &'a ParserDfa,
    id: DfaStateId,
}

impl std::fmt::Debug for ParserDfaStateView<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParserDfaStateView")
            .field("id", &self.id)
            .field("is_accept_state", &self.is_accept_state())
            .field("prediction", &self.prediction())
            .field("requires_full_context", &self.requires_full_context())
            .finish_non_exhaustive()
    }
}

impl<'a> ParserDfaStateView<'a> {
    pub const fn id(self) -> DfaStateId {
        self.id
    }

    pub fn is_accept_state(self) -> bool {
        self.dfa.hot.has_flag(self.id, ACCEPT_STATE)
    }

    pub fn prediction(self) -> Option<usize> {
        self.dfa.hot.prediction(self.id)
    }

    pub fn requires_full_context(self) -> bool {
        self.dfa.hot.has_flag(self.id, REQUIRES_FULL_CONTEXT)
    }

    pub fn has_semantic_context(self) -> bool {
        self.dfa.hot.has_flag(self.id, HAS_SEMANTIC_CONTEXT)
    }

    pub fn edge(self, symbol: i32) -> Option<DfaStateId> {
        self.dfa.edge(self.id, symbol)
    }

    pub fn transitions(self) -> impl Iterator<Item = DfaTransition> + 'a {
        self.dfa
            .hot
            .edges
            .transitions(self.id)
            .map(move |(symbol, target)| DfaTransition {
                source: self.id,
                symbol,
                target,
            })
    }
}

#[derive(Debug)]
pub(crate) struct DfaStateBuilder {
    pub(crate) configs: AtnConfigSet,
    prediction: Option<usize>,
    requires_full_context: bool,
    conflicting_alts: Vec<usize>,
    has_semantic_context_for_alt: bool,
}

impl DfaStateBuilder {
    pub(crate) const fn new(configs: AtnConfigSet) -> Self {
        Self {
            configs,
            prediction: None,
            requires_full_context: false,
            conflicting_alts: Vec::new(),
            has_semantic_context_for_alt: false,
        }
    }

    pub(crate) const fn mark_accept(&mut self, prediction: usize) {
        self.prediction = Some(prediction);
    }

    pub(crate) const fn set_requires_full_context(&mut self, required: bool) {
        self.requires_full_context = required;
    }

    pub(crate) fn set_conflicting_alts(&mut self, conflicting_alts: Vec<usize>) {
        self.conflicting_alts = conflicting_alts;
    }

    pub(crate) const fn set_has_semantic_context_for_alt(&mut self, has_semantic: bool) {
        self.has_semantic_context_for_alt = has_semantic;
    }
}

#[derive(Debug)]
struct DfaHotTables {
    edges: EdgeTable,
    accept_predictions: Vec<u32>,
    flags: Vec<u8>,
}

impl DfaHotTables {
    fn new(max_token_type: i32) -> Self {
        Self {
            edges: EdgeTable::new(max_token_type),
            accept_predictions: Vec::new(),
            flags: Vec::new(),
        }
    }

    fn push_state(
        &mut self,
        prediction: Option<usize>,
        requires_full_context: bool,
        has_semantic_context: bool,
    ) {
        self.edges.push_row();
        let prediction = prediction.map_or(NO_PREDICTION, |prediction| {
            compact_index(prediction, "DFA prediction must fit below the u32 sentinel")
        });
        self.accept_predictions.push(prediction);
        let mut flags = 0;
        if prediction != NO_PREDICTION {
            flags |= ACCEPT_STATE;
        }
        if requires_full_context {
            flags |= REQUIRES_FULL_CONTEXT;
        }
        if has_semantic_context {
            flags |= HAS_SEMANTIC_CONTEXT;
        }
        self.flags.push(flags);
        debug_assert_eq!(self.edges.len(), self.accept_predictions.len());
        debug_assert_eq!(self.edges.len(), self.flags.len());
    }

    fn prediction(&self, state: DfaStateId) -> Option<usize> {
        let prediction = self.accept_predictions[state.index()];
        (prediction != NO_PREDICTION)
            .then(|| usize::try_from(prediction).expect("u32 DFA prediction fits in usize"))
    }

    fn has_flag(&self, state: DfaStateId, flag: u8) -> bool {
        self.flags[state.index()] & flag != 0
    }

    const fn len(&self) -> usize {
        self.flags.len()
    }

    const fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    fn clear(&mut self) {
        self.edges.clear();
        self.accept_predictions.clear();
        self.flags.clear();
    }

    const fn retained_bytes(&self) -> usize {
        self.edges.retained_bytes()
            + self.accept_predictions.capacity() * size_of::<u32>()
            + self.flags.capacity() * size_of::<u8>()
    }
}

#[derive(Debug, Default)]
struct DfaColdStore {
    configs: Vec<AtnConfigSet>,
    extras: Vec<DfaColdExtras>,
}

impl DfaColdStore {
    fn push(&mut self, configs: AtnConfigSet, conflicting_alts: Vec<usize>) {
        self.configs.push(configs);
        self.extras.push(DfaColdExtras { conflicting_alts });
        debug_assert_eq!(self.configs.len(), self.extras.len());
    }

    fn clear(&mut self) {
        self.configs.clear();
        self.extras.clear();
    }

    fn retained_bytes(&self) -> usize {
        self.configs.capacity() * size_of::<AtnConfigSet>()
            + self.extras.capacity() * size_of::<DfaColdExtras>()
            + self
                .configs
                .iter()
                .map(AtnConfigSet::retained_bytes)
                .sum::<usize>()
            + self
                .extras
                .iter()
                .map(|extra| extra.conflicting_alts.capacity() * size_of::<usize>())
                .sum::<usize>()
    }
}

#[derive(Debug, Default)]
struct DfaColdExtras {
    conflicting_alts: Vec<usize>,
}

#[derive(Debug, Default)]
struct DfaStateInterner {
    heads: FxHashMap<u64, DfaStateId>,
    next: Vec<DfaStateId>,
}

impl DfaStateInterner {
    fn head(&self, fingerprint: u64) -> DfaStateId {
        self.heads
            .get(&fingerprint)
            .copied()
            .unwrap_or(NO_DFA_STATE)
    }

    fn next(&self, state: DfaStateId) -> DfaStateId {
        self.next[state.index()]
    }

    fn insert(&mut self, fingerprint: u64, state: DfaStateId) {
        debug_assert_eq!(state.index(), self.next.len());
        let previous = self
            .heads
            .insert(fingerprint, state)
            .unwrap_or(NO_DFA_STATE);
        self.next.push(previous);
    }

    fn rebuild(&mut self, configs: &[AtnConfigSet]) {
        self.clear();
        self.next.reserve(configs.len());
        for (index, configs) in configs.iter().enumerate() {
            self.insert(configs.fingerprint(), DfaStateId::from_index(index));
        }
    }

    fn clear(&mut self) {
        self.heads.clear();
        self.next.clear();
    }

    fn retained_bytes(&self) -> usize {
        self.heads.capacity() * size_of::<(u64, DfaStateId)>()
            + self.next.capacity() * size_of::<DfaStateId>()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct DfaLearningCounters {
    states_created: usize,
    states_deduplicated: usize,
    fingerprint_candidates: usize,
    fingerprint_collisions: usize,
}

#[derive(Clone, Copy, Debug, Default)]
enum EdgeRow {
    #[default]
    Empty,
    Inline {
        symbol: i32,
        target: DfaStateId,
    },
    Sparse {
        head: u32,
        len: u32,
    },
    Dense {
        start: u32,
        populated: u32,
    },
}

#[derive(Clone, Copy, Debug)]
struct SparseEdge {
    symbol: i32,
    target: DfaStateId,
    next: u32,
}

#[derive(Debug)]
struct EdgeTable {
    width: u32,
    rows: Vec<EdgeRow>,
    dense_targets: Vec<DfaStateId>,
    sparse_edges: Vec<SparseEdge>,
}

impl EdgeTable {
    fn new(max_token_type: i32) -> Self {
        let width = i64::from(max_token_type)
            .checked_add(2)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        Self {
            width,
            rows: Vec::new(),
            dense_targets: Vec::new(),
            sparse_edges: Vec::new(),
        }
    }

    fn push_row(&mut self) {
        self.rows.push(EdgeRow::default());
    }

    const fn len(&self) -> usize {
        self.rows.len()
    }

    fn target(&self, state: DfaStateId, symbol: i32) -> Option<DfaStateId> {
        let slot = self.slot(symbol)?;
        match *self.rows.get(state.index())? {
            EdgeRow::Empty => None,
            EdgeRow::Inline {
                symbol: stored,
                target,
            } => (stored == symbol).then_some(target),
            EdgeRow::Dense { start, .. } => {
                let index = usize::try_from(start.checked_add(slot)?).ok()?;
                self.dense_targets
                    .get(index)
                    .copied()
                    .filter(|target| *target != NO_DFA_STATE)
            }
            EdgeRow::Sparse { mut head, .. } => {
                while head != NO_EDGE_INDEX {
                    let edge = self.sparse_edges[usize::try_from(head).ok()?];
                    match edge.symbol.cmp(&symbol) {
                        std::cmp::Ordering::Less => head = edge.next,
                        std::cmp::Ordering::Equal => return Some(edge.target),
                        std::cmp::Ordering::Greater => return None,
                    }
                }
                None
            }
        }
    }

    fn add(&mut self, state: DfaStateId, symbol: i32, target: DfaStateId) -> bool {
        let Some(slot) = self.slot(symbol) else {
            return false;
        };
        match self.rows[state.index()] {
            EdgeRow::Empty => {
                self.rows[state.index()] = EdgeRow::Inline { symbol, target };
                true
            }
            EdgeRow::Inline {
                symbol: stored_symbol,
                target: stored_target,
            } => {
                if stored_symbol == symbol {
                    let changed = stored_target != target;
                    self.rows[state.index()] = EdgeRow::Inline { symbol, target };
                    return changed;
                }
                self.promote_inline_to_sparse(state, stored_symbol, stored_target, symbol, target);
                true
            }
            EdgeRow::Dense { start, populated } => {
                let index = usize::try_from(start.checked_add(slot).expect("dense slot overflow"))
                    .expect("u32 dense slot fits in usize");
                let changed = self.dense_targets[index] != target;
                if self.dense_targets[index] == NO_DFA_STATE {
                    self.rows[state.index()] = EdgeRow::Dense {
                        start,
                        populated: populated
                            .checked_add(1)
                            .expect("dense edge count must fit in u32"),
                    };
                }
                self.dense_targets[index] = target;
                changed
            }
            EdgeRow::Sparse { head, len } => {
                if let Some(changed) = self.update_sparse(head, symbol, target) {
                    return changed;
                }
                self.insert_sparse(state, head, len, symbol, target);
                true
            }
        }
    }

    fn promote_inline_to_sparse(
        &mut self,
        state: DfaStateId,
        first_symbol: i32,
        first_target: DfaStateId,
        second_symbol: i32,
        second_target: DfaStateId,
    ) {
        let (lower_symbol, lower_target, upper_symbol, upper_target) =
            if first_symbol < second_symbol {
                (first_symbol, first_target, second_symbol, second_target)
            } else {
                (second_symbol, second_target, first_symbol, first_target)
            };
        let upper_index = compact_index(
            self.sparse_edges.len(),
            "sparse edge pool must fit below the u32 sentinel",
        );
        self.sparse_edges.push(SparseEdge {
            symbol: upper_symbol,
            target: upper_target,
            next: NO_EDGE_INDEX,
        });
        let lower_index = compact_index(
            self.sparse_edges.len(),
            "sparse edge pool must fit below the u32 sentinel",
        );
        self.sparse_edges.push(SparseEdge {
            symbol: lower_symbol,
            target: lower_target,
            next: upper_index,
        });
        self.rows[state.index()] = EdgeRow::Sparse {
            head: lower_index,
            len: 2,
        };
    }

    fn update_sparse(
        &mut self,
        mut edge_index: u32,
        symbol: i32,
        target: DfaStateId,
    ) -> Option<bool> {
        while edge_index != NO_EDGE_INDEX {
            let edge = &mut self.sparse_edges
                [usize::try_from(edge_index).expect("u32 sparse edge index fits in usize")];
            match edge.symbol.cmp(&symbol) {
                std::cmp::Ordering::Less => edge_index = edge.next,
                std::cmp::Ordering::Equal => {
                    let changed = edge.target != target;
                    edge.target = target;
                    return Some(changed);
                }
                std::cmp::Ordering::Greater => return None,
            }
        }
        None
    }

    fn insert_sparse(
        &mut self,
        state: DfaStateId,
        head: u32,
        len: u32,
        symbol: i32,
        target: DfaStateId,
    ) {
        let new_index = compact_index(
            self.sparse_edges.len(),
            "sparse edge pool must fit below the u32 sentinel",
        );
        let mut previous = NO_EDGE_INDEX;
        let mut current = head;
        while current != NO_EDGE_INDEX {
            let edge = self.sparse_edges
                [usize::try_from(current).expect("u32 sparse edge index fits in usize")];
            if edge.symbol > symbol {
                break;
            }
            previous = current;
            current = edge.next;
        }
        self.sparse_edges.push(SparseEdge {
            symbol,
            target,
            next: current,
        });
        let new_len = len
            .checked_add(1)
            .expect("sparse edge count must fit in u32");
        if previous == NO_EDGE_INDEX {
            self.rows[state.index()] = EdgeRow::Sparse {
                head: new_index,
                len: new_len,
            };
        } else {
            self.sparse_edges
                [usize::try_from(previous).expect("u32 sparse edge index fits in usize")]
            .next = new_index;
            self.rows[state.index()] = EdgeRow::Sparse { head, len: new_len };
        }
        if self.should_promote(new_len) {
            self.promote(state);
        }
    }

    const fn should_promote(&self, populated: u32) -> bool {
        self.width <= DENSE_MAX_ROW_WIDTH
            && populated >= DENSE_MIN_EDGES
            && populated.saturating_mul(DENSE_DENSITY_DENOMINATOR) >= self.width
    }

    fn promote(&mut self, state: DfaStateId) {
        let EdgeRow::Sparse {
            mut head,
            len: populated,
        } = self.rows[state.index()]
        else {
            return;
        };
        let start = compact_index(
            self.dense_targets.len(),
            "dense edge pool must fit below the u32 sentinel",
        );
        let new_len = self
            .dense_targets
            .len()
            .checked_add(usize::try_from(self.width).expect("u32 row width fits in usize"))
            .expect("dense edge pool length overflow");
        self.dense_targets.resize(new_len, NO_DFA_STATE);
        while head != NO_EDGE_INDEX {
            let edge = self.sparse_edges
                [usize::try_from(head).expect("u32 sparse edge index fits in usize")];
            let slot = self.slot(edge.symbol).expect("stored symbol fits row");
            let index = usize::try_from(start.checked_add(slot).expect("dense slot overflow"))
                .expect("u32 dense slot fits in usize");
            self.dense_targets[index] = edge.target;
            head = edge.next;
        }
        self.rows[state.index()] = EdgeRow::Dense { start, populated };
    }

    fn transitions(&self, state: DfaStateId) -> EdgeTransitions<'_> {
        match self.rows[state.index()] {
            EdgeRow::Empty => EdgeTransitions::Empty,
            EdgeRow::Inline { symbol, target } => EdgeTransitions::Inline {
                edge: Some((symbol, target)),
            },
            EdgeRow::Sparse { head, .. } => EdgeTransitions::Sparse {
                table: self,
                next: head,
            },
            EdgeRow::Dense { start, .. } => EdgeTransitions::Dense {
                table: self,
                start,
                slot: 0,
            },
        }
    }

    fn slot(&self, symbol: i32) -> Option<u32> {
        let slot = symbol
            .checked_add(1)
            .and_then(|value| u32::try_from(value).ok())?;
        (slot < self.width).then_some(slot)
    }

    fn stats(&self) -> EdgeTableStats {
        let mut stats = EdgeTableStats {
            max_row_width: usize::try_from(self.width).expect("u32 row width fits in usize"),
            dense_slots: self.dense_targets.len(),
            sparse_entries: self.sparse_edges.len(),
            ..EdgeTableStats::default()
        };
        for row in &self.rows {
            let populated = match *row {
                EdgeRow::Empty => {
                    stats.empty_rows += 1;
                    0
                }
                EdgeRow::Inline { .. } => {
                    stats.sparse_rows += 1;
                    1
                }
                EdgeRow::Dense { populated, .. } => {
                    stats.dense_rows += 1;
                    populated
                }
                EdgeRow::Sparse { len, .. } => {
                    stats.sparse_rows += 1;
                    len
                }
            };
            stats.transitions = stats
                .transitions
                .saturating_add(usize::try_from(populated).expect("u32 edge count fits in usize"));
            stats.row_width_histogram[row_width_bucket(self.width)] += 1;
            stats.populated_edge_histogram[populated_edge_bucket(populated)] += 1;
            let bucket = density_bucket(populated, self.width);
            stats.edge_density_histogram[bucket] += 1;
        }
        stats
    }

    const fn retained_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<EdgeRow>()
            + self.dense_targets.capacity() * size_of::<DfaStateId>()
            + self.sparse_edges.capacity() * size_of::<SparseEdge>()
    }

    fn clear(&mut self) {
        self.rows.clear();
        self.dense_targets.clear();
        self.sparse_edges.clear();
    }
}

enum EdgeTransitions<'a> {
    Empty,
    Inline {
        edge: Option<(i32, DfaStateId)>,
    },
    Sparse {
        table: &'a EdgeTable,
        next: u32,
    },
    Dense {
        table: &'a EdgeTable,
        start: u32,
        slot: u32,
    },
}

impl Iterator for EdgeTransitions<'_> {
    type Item = (i32, DfaStateId);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Empty => None,
            Self::Inline { edge } => edge.take(),
            Self::Sparse { table, next } => {
                if *next == NO_EDGE_INDEX {
                    return None;
                }
                let edge = table.sparse_edges
                    [usize::try_from(*next).expect("u32 sparse edge index fits in usize")];
                *next = edge.next;
                Some((edge.symbol, edge.target))
            }
            Self::Dense { table, start, slot } => {
                while *slot < table.width {
                    let current = *slot;
                    *slot += 1;
                    let index =
                        usize::try_from(start.checked_add(current).expect("dense slot overflow"))
                            .expect("u32 dense slot fits in usize");
                    let target = table.dense_targets[index];
                    if target != NO_DFA_STATE {
                        let symbol =
                            i32::try_from(current).expect("bounded dense row slot fits in i32") - 1;
                        return Some((symbol, target));
                    }
                }
                None
            }
        }
    }
}

#[derive(Debug, Default)]
struct EdgeTableStats {
    transitions: usize,
    max_row_width: usize,
    dense_rows: usize,
    sparse_rows: usize,
    empty_rows: usize,
    dense_slots: usize,
    sparse_entries: usize,
    row_width_histogram: [usize; 5],
    populated_edge_histogram: [usize; 6],
    edge_density_histogram: [usize; 6],
}

const fn row_width_bucket(width: u32) -> usize {
    if width <= 64 {
        0
    } else if width <= 128 {
        1
    } else if width <= 256 {
        2
    } else if width <= 512 {
        3
    } else {
        4
    }
}

const fn populated_edge_bucket(populated: u32) -> usize {
    match populated {
        0 => 0,
        1 => 1,
        2..=3 => 2,
        4..=7 => 3,
        8..=15 => 4,
        _ => 5,
    }
}

const fn density_bucket(populated: u32, width: u32) -> usize {
    if populated == 0 || width == 0 {
        0
    } else if populated.saturating_mul(100) <= width {
        1
    } else if populated.saturating_mul(20) <= width {
        2
    } else if populated.saturating_mul(8) <= width {
        3
    } else if populated.saturating_mul(4) <= width {
        4
    } else {
        5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prediction::{AtnConfig, ContextArena, EMPTY_CONTEXT, PredictionWorkspace};
    use crate::token::TOKEN_EOF;

    fn configs(state: usize, arena: &mut ContextArena) -> AtnConfigSet {
        let mut workspace = PredictionWorkspace::default();
        let mut configs = AtnConfigSet::new();
        configs.add(
            AtnConfig::new(state, 1, EMPTY_CONTEXT, arena),
            arena,
            &mut workspace,
        );
        configs
    }

    #[test]
    fn dfa_reuses_equal_config_sets_without_key_clone() {
        let mut arena = ContextArena::new();
        let configs = configs(1, &mut arena);
        let mut dfa = ParserDfa::with_max_token_type(0, 0, 16);

        assert_eq!(
            dfa.add_state(DfaStateBuilder::new(configs.clone())).index(),
            0
        );
        assert_eq!(dfa.add_state(DfaStateBuilder::new(configs)).index(), 0);
        assert_eq!(dfa.state_count(), 1);
        assert_eq!(dfa.stats().states_deduplicated, 1);
    }

    #[test]
    fn fingerprint_collisions_are_verified_structurally() {
        let mut arena = ContextArena::new();
        let mut dfa = ParserDfa::with_max_token_type(0, 0, 16);
        let first = DfaStateBuilder::new(configs(1, &mut arena));
        let second = DfaStateBuilder::new(configs(2, &mut arena));
        let fingerprint = 7;

        let first_id = dfa.insert_state_with_fingerprint(first, fingerprint);
        let second_id = dfa.insert_state_with_fingerprint(second, fingerprint);
        let first_configs = configs(1, &mut arena);

        assert_eq!(dfa.find_state(fingerprint, &first_configs), Some(first_id));
        assert_ne!(first_id, second_id);
        assert_eq!(dfa.stats().fingerprint_collisions, 1);
    }

    #[test]
    fn sparse_edges_are_sorted_and_use_scalar_targets() {
        let mut arena = ContextArena::new();
        let mut dfa = ParserDfa::with_max_token_type(0, 0, 32);
        let source = dfa.add_state(DfaStateBuilder::new(configs(1, &mut arena)));
        let target = dfa.add_state(DfaStateBuilder::new(configs(2, &mut arena)));

        dfa.add_edge(source, 5, target);
        dfa.add_edge(source, -1, target);
        dfa.add_edge(source, 2, target);

        assert_eq!(dfa.edge(source, -1), Some(target));
        assert_eq!(dfa.edge(source, 4), None);
        assert_eq!(
            dfa.state(source)
                .expect("source")
                .transitions()
                .map(|edge| edge.symbol)
                .collect::<Vec<_>>(),
            [-1, 2, 5]
        );
    }

    #[test]
    fn populated_rows_promote_into_shared_dense_slab() {
        let mut arena = ContextArena::new();
        let mut dfa = ParserDfa::with_max_token_type(0, 0, 62);
        let source = dfa.add_state(DfaStateBuilder::new(configs(1, &mut arena)));
        let target = dfa.add_state(DfaStateBuilder::new(configs(2, &mut arena)));

        for symbol in 0..8 {
            dfa.add_edge(source, symbol, target);
        }

        assert_eq!(dfa.stats().dense_rows, 1);
        assert_eq!(dfa.edge(source, 7), Some(target));
        assert_eq!(dfa.edge(source, 8), None);
    }

    #[test]
    fn creating_states_does_not_allocate_edge_rows() {
        let mut arena = ContextArena::new();
        let mut dfa = ParserDfa::with_max_token_type(0, 0, 255);

        for state in 0..32 {
            dfa.add_state(DfaStateBuilder::new(configs(state, &mut arena)));
        }

        let stats = dfa.stats();
        assert_eq!(stats.dense_slots, 0);
        assert_eq!(stats.sparse_entries, 0);
        assert_eq!(stats.empty_rows, 32);
    }

    #[test]
    fn edge_updates_do_not_duplicate_sparse_entries() {
        let mut arena = ContextArena::new();
        let mut dfa = ParserDfa::with_max_token_type(0, 0, 32);
        let source = dfa.add_state(DfaStateBuilder::new(configs(1, &mut arena)));
        let first = dfa.add_state(DfaStateBuilder::new(configs(2, &mut arena)));
        let second = dfa.add_state(DfaStateBuilder::new(configs(3, &mut arena)));

        dfa.add_edge(source, TOKEN_EOF, first);
        dfa.add_edge(source, TOKEN_EOF, first);
        dfa.add_edge(source, TOKEN_EOF, second);

        assert_eq!(dfa.edge(source, TOKEN_EOF), Some(second));
        assert_eq!(dfa.state(source).expect("source").transitions().count(), 1);
    }

    #[test]
    fn out_of_vocabulary_edges_are_not_stored() {
        let mut arena = ContextArena::new();
        let mut dfa = ParserDfa::with_max_token_type(0, 0, 4);
        let source = dfa.add_state(DfaStateBuilder::new(configs(1, &mut arena)));
        let target = dfa.add_state(DfaStateBuilder::new(configs(2, &mut arena)));

        dfa.add_edge(source, -2, target);
        dfa.add_edge(source, 5, target);

        assert_eq!(dfa.edge(source, -2), None);
        assert_eq!(dfa.edge(source, 5), None);
        assert_eq!(dfa.stats().transitions, 0);
    }

    #[test]
    fn precedence_dfa_tracks_start_states_by_compact_id() {
        let mut dfa = ParserDfa::new(10, 2);
        dfa.set_precedence_dfa(true);
        let start = dfa.start_state().expect("precedence root");
        dfa.set_precedence_start_state(4, start);

        assert!(dfa.is_precedence_dfa());
        assert_eq!(dfa.precedence_start_state(4), Some(start));
        assert_eq!(dfa.precedence_start_state(3), None);
    }
}
