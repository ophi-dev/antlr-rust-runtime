use crate::prediction::AtnConfigSet;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dfa {
    decision: usize,
    atn_start_state: usize,
    states: Vec<DfaState>,
}

impl Dfa {
    pub const fn new(atn_start_state: usize, decision: usize) -> Self {
        Self {
            decision,
            atn_start_state,
            states: Vec::new(),
        }
    }

    pub const fn decision(&self) -> usize {
        self.decision
    }

    pub const fn atn_start_state(&self) -> usize {
        self.atn_start_state
    }

    pub fn states(&self) -> &[DfaState] {
        &self.states
    }

    /// Inserts a DFA state or returns the existing state number for an
    /// equivalent ATN configuration set.
    pub fn add_state(&mut self, mut state: DfaState) -> usize {
        if let Some(existing) = self
            .states
            .iter()
            .find(|candidate| candidate.configs == state.configs)
        {
            return existing.state_number;
        }
        let state_number = self.states.len();
        state.state_number = state_number;
        self.states.push(state);
        state_number
    }

    pub fn state_mut(&mut self, state_number: usize) -> Option<&mut DfaState> {
        self.states.get_mut(state_number)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DfaState {
    pub state_number: usize,
    pub configs: AtnConfigSet,
    pub edges: BTreeMap<i32, usize>,
    pub is_accept_state: bool,
    pub prediction: Option<usize>,
    pub requires_full_context: bool,
}

impl DfaState {
    pub const fn new(configs: AtnConfigSet) -> Self {
        Self {
            state_number: usize::MAX,
            configs,
            edges: BTreeMap::new(),
            is_accept_state: false,
            prediction: None,
            requires_full_context: false,
        }
    }

    pub fn add_edge(&mut self, symbol: i32, target_state: usize) {
        self.edges.insert(symbol, target_state);
    }

    pub fn edge(&self, symbol: i32) -> Option<usize> {
        self.edges.get(&symbol).copied()
    }

    pub const fn mark_accept(&mut self, prediction: usize) {
        self.is_accept_state = true;
        self.prediction = Some(prediction);
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
        let mut dfa = Dfa::new(0, 0);
        assert_eq!(dfa.add_state(state), 0);
        assert_eq!(dfa.add_state(DfaState::new(configs)), 0);
    }
}
