use crate::prediction::AtnConfigSet;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dfa {
    decision: usize,
    atn_start_state: usize,
    max_token_type: i32,
    states: Vec<DfaState>,
    state_index: BTreeMap<AtnConfigSet, usize>,
    start_state: Option<usize>,
    precedence_start_states: Vec<Option<usize>>,
    precedence_mode: bool,
}

impl Dfa {
    pub const fn new(atn_start_state: usize, decision: usize) -> Self {
        Self::with_max_token_type(atn_start_state, decision, 0)
    }

    pub const fn with_max_token_type(
        atn_start_state: usize,
        decision: usize,
        max_token_type: i32,
    ) -> Self {
        Self {
            decision,
            atn_start_state,
            max_token_type,
            states: Vec::new(),
            state_index: BTreeMap::new(),
            start_state: None,
            precedence_start_states: Vec::new(),
            precedence_mode: false,
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

    pub fn states(&self) -> &[DfaState] {
        &self.states
    }

    pub const fn start_state(&self) -> Option<usize> {
        self.start_state
    }

    pub const fn set_start_state(&mut self, state_number: usize) {
        self.start_state = Some(state_number);
    }

    pub const fn is_precedence_dfa(&self) -> bool {
        self.precedence_mode
    }

    pub fn set_precedence_dfa(&mut self, precedence_dfa: bool) {
        if self.precedence_mode == precedence_dfa {
            return;
        }
        self.states.clear();
        self.state_index.clear();
        self.start_state = None;
        self.precedence_start_states.clear();
        self.precedence_mode = precedence_dfa;
        if precedence_dfa {
            self.start_state = Some(self.add_state(DfaState::new(AtnConfigSet::new())));
        }
    }

    pub fn precedence_start_state(&self, precedence: usize) -> Option<usize> {
        self.precedence_start_states
            .get(precedence)
            .and_then(|state| *state)
    }

    pub fn set_precedence_start_state(&mut self, precedence: usize, state_number: usize) {
        if precedence >= self.precedence_start_states.len() {
            self.precedence_start_states.resize(precedence + 1, None);
        }
        self.precedence_start_states[precedence] = Some(state_number);
    }

    /// Inserts a DFA state or returns the existing state number for an
    /// equivalent ATN configuration set.
    pub fn add_state(&mut self, state: DfaState) -> usize {
        if let Some(existing) = self.state_number_for_configs(&state.configs) {
            return existing;
        }
        self.insert_state(state)
    }

    pub(crate) fn insert_state(&mut self, mut state: DfaState) -> usize {
        let state_number = self.states.len();
        state.state_number = state_number;
        state.ensure_edge_capacity(self.max_token_type);
        let state_key = state.configs.clone();
        state.configs.set_readonly(true);
        self.state_index.insert(state_key, state_number);
        self.states.push(state);
        state_number
    }

    pub(crate) fn state_number_for_configs(&self, configs: &AtnConfigSet) -> Option<usize> {
        self.state_index.get(configs).copied()
    }

    pub fn state(&self, state_number: usize) -> Option<&DfaState> {
        self.states.get(state_number)
    }

    pub fn state_mut(&mut self, state_number: usize) -> Option<&mut DfaState> {
        self.states.get_mut(state_number)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DfaState {
    pub state_number: usize,
    pub configs: AtnConfigSet,
    pub edges: Vec<Option<usize>>,
    pub is_accept_state: bool,
    pub prediction: Option<usize>,
    pub requires_full_context: bool,
    pub conflicting_alts: Vec<usize>,
}

impl DfaState {
    pub const fn new(configs: AtnConfigSet) -> Self {
        Self {
            state_number: usize::MAX,
            configs,
            edges: Vec::new(),
            is_accept_state: false,
            prediction: None,
            requires_full_context: false,
            conflicting_alts: Vec::new(),
        }
    }

    pub fn add_edge(&mut self, symbol: i32, target_state: usize) {
        let Some(index) = edge_index(symbol) else {
            return;
        };
        if index >= self.edges.len() {
            self.edges.resize(index + 1, None);
        }
        self.edges[index] = Some(target_state);
    }

    pub fn edge(&self, symbol: i32) -> Option<usize> {
        edge_index(symbol)
            .and_then(|index| self.edges.get(index))
            .and_then(|state| *state)
    }

    pub const fn mark_accept(&mut self, prediction: usize) {
        self.is_accept_state = true;
        self.prediction = Some(prediction);
    }

    fn ensure_edge_capacity(&mut self, max_token_type: i32) {
        let Ok(max) = usize::try_from(max_token_type) else {
            return;
        };
        if self.edges.len() < max + 2 {
            self.edges.resize(max + 2, None);
        }
    }
}

fn edge_index(symbol: i32) -> Option<usize> {
    if symbol < -1 {
        None
    } else {
        usize::try_from(symbol + 1).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prediction::{AtnConfig, AtnConfigSet, PredictionContext};

    #[test]
    fn dfa_reuses_equal_config_sets() {
        let mut configs = AtnConfigSet::new();
        configs.add(AtnConfig::new(1, 1, PredictionContext::empty()));
        let state = DfaState::new(configs.clone());
        let mut dfa = Dfa::with_max_token_type(0, 0, 16);
        assert_eq!(dfa.add_state(state), 0);
        assert_eq!(dfa.add_state(DfaState::new(configs)), 0);
    }

    #[test]
    fn dfa_edges_are_dense_by_token_type() {
        let mut state = DfaState::new(AtnConfigSet::new());
        state.add_edge(-1, 3);
        state.add_edge(5, 7);

        assert_eq!(state.edge(-1), Some(3));
        assert_eq!(state.edge(5), Some(7));
        assert_eq!(state.edge(4), None);
    }

    #[test]
    fn precedence_dfa_tracks_start_states_by_precedence() {
        let mut dfa = Dfa::new(10, 2);
        dfa.set_precedence_dfa(true);
        dfa.set_precedence_start_state(4, 9);

        assert!(dfa.is_precedence_dfa());
        assert_eq!(dfa.precedence_start_state(4), Some(9));
        assert_eq!(dfa.precedence_start_state(3), None);
    }
}
