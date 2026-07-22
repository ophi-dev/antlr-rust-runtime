use std::collections::BTreeMap;

use antlr4_runtime::atn::AtnStateKind;

use super::super::model::{BuildStateId, BuildTransitionId, ModelNodeId, RuleId};
use super::super::provenance::{Origin, ProvenanceIndex, SyntheticReason};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BuildTransitionKind {
    Epsilon,
    Atom(i32),
    Range {
        start: i32,
        stop: i32,
    },
    Set(Vec<(i32, i32)>),
    NotSet(Vec<(i32, i32)>),
    Wildcard,
    Rule {
        rule: RuleId,
        rule_index: usize,
        follow: BuildStateId,
        precedence: i32,
    },
    Predicate {
        rule_index: usize,
        predicate_index: usize,
        context_dependent: bool,
    },
    Action {
        rule_index: usize,
        action_index: Option<usize>,
        context_dependent: bool,
    },
    Precedence(i32),
}

impl BuildTransitionKind {
    pub(crate) const fn is_epsilon(&self) -> bool {
        matches!(
            self,
            Self::Epsilon
                | Self::Rule { .. }
                | Self::Predicate { .. }
                | Self::Action { .. }
                | Self::Precedence(_)
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BuildTransition {
    pub(crate) id: BuildTransitionId,
    pub(crate) source: BuildStateId,
    pub(crate) target: BuildStateId,
    pub(crate) kind: BuildTransitionKind,
}

pub(crate) struct BuildTransitionSpec {
    pub(crate) source: BuildStateId,
    pub(crate) target: BuildStateId,
    pub(crate) kind: BuildTransitionKind,
    pub(crate) prepend: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildState {
    pub(crate) id: BuildStateId,
    pub(crate) kind: AtnStateKind,
    pub(crate) rule: Option<RuleId>,
    pub(crate) rule_index: Option<usize>,
    pub(crate) end_state: Option<BuildStateId>,
    pub(crate) loop_back_state: Option<BuildStateId>,
    pub(crate) non_greedy: bool,
    pub(crate) left_recursive_rule: bool,
    pub(crate) transitions: Vec<BuildTransitionId>,
    pub(crate) active: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildGraph {
    pub(crate) max_token_type: i32,
    pub(crate) states: Vec<BuildState>,
    pub(crate) transitions: Vec<BuildTransition>,
    pub(crate) decisions: Vec<BuildStateId>,
    pub(crate) rule_starts: Vec<BuildStateId>,
    pub(crate) rule_stops: Vec<BuildStateId>,
}

impl BuildGraph {
    pub(crate) const fn new(max_token_type: i32) -> Self {
        Self {
            max_token_type,
            states: Vec::new(),
            transitions: Vec::new(),
            decisions: Vec::new(),
            rule_starts: Vec::new(),
            rule_stops: Vec::new(),
        }
    }

    pub(crate) fn add_state(
        &mut self,
        kind: AtnStateKind,
        rule: Option<(RuleId, usize)>,
        origins: impl IntoIterator<Item = Origin>,
        provenance: &mut ProvenanceIndex,
    ) -> BuildStateId {
        let id = BuildStateId::new(
            u32::try_from(self.states.len()).expect("ATN state count exceeds u32"),
        );
        let (rule, rule_index) =
            rule.map_or((None, None), |(rule, index)| (Some(rule), Some(index)));
        self.states.push(BuildState {
            id,
            kind,
            rule,
            rule_index,
            end_state: None,
            loop_back_state: None,
            non_greedy: false,
            left_recursive_rule: false,
            transitions: Vec::new(),
            active: true,
        });
        provenance.record_state(id, origins);
        id
    }

    pub(crate) fn add_synthetic_state(
        &mut self,
        kind: AtnStateKind,
        rule: Option<(RuleId, usize)>,
        reason: SyntheticReason,
        owner: ModelNodeId,
        provenance: &mut ProvenanceIndex,
    ) -> BuildStateId {
        let mut origins = provenance.origins(owner).to_vec();
        origins.push(Origin::Synthetic { reason, owner });
        self.add_state(kind, rule, origins, provenance)
    }

    pub(crate) fn add_transition(
        &mut self,
        spec: BuildTransitionSpec,
        origins: impl IntoIterator<Item = Origin>,
        provenance: &mut ProvenanceIndex,
    ) -> BuildTransitionId {
        let BuildTransitionSpec {
            source,
            target,
            kind,
            prepend,
        } = spec;
        if let Some(existing) = self.state(source).transitions.iter().copied().find(|id| {
            let transition = self.transition(*id);
            transition.target == target && transition.kind == kind
        }) {
            provenance.record_transition(existing, origins);
            return existing;
        }
        let id = BuildTransitionId::new(
            u32::try_from(self.transitions.len()).expect("ATN transition count exceeds u32"),
        );
        self.transitions.push(BuildTransition {
            id,
            source,
            target,
            kind,
        });
        let transitions = &mut self.state_mut(source).transitions;
        if prepend {
            transitions.insert(0, id);
        } else {
            transitions.push(id);
        }
        provenance.record_transition(id, origins);
        id
    }

    pub(crate) fn add_synthetic_transition(
        &mut self,
        spec: BuildTransitionSpec,
        reason: SyntheticReason,
        owner: ModelNodeId,
        provenance: &mut ProvenanceIndex,
    ) -> BuildTransitionId {
        let mut origins = provenance.origins(owner).to_vec();
        origins.push(Origin::Synthetic { reason, owner });
        self.add_transition(spec, origins, provenance)
    }

    pub(crate) fn state(&self, id: BuildStateId) -> &BuildState {
        &self.states[id.index()]
    }

    pub(crate) fn state_mut(&mut self, id: BuildStateId) -> &mut BuildState {
        &mut self.states[id.index()]
    }

    pub(crate) fn transition(&self, id: BuildTransitionId) -> &BuildTransition {
        &self.transitions[id.index()]
    }

    pub(crate) fn transition_mut(&mut self, id: BuildTransitionId) -> &mut BuildTransition {
        &mut self.transitions[id.index()]
    }

    pub(crate) fn remove_state(&mut self, id: BuildStateId) {
        self.state_mut(id).active = false;
    }

    pub(crate) fn add_decision(&mut self, state: BuildStateId) {
        self.decisions.push(state);
    }

    pub(crate) fn finalize(self) -> FinalizedAtnGraph {
        let mut state_map = BTreeMap::new();
        for state in self.states.iter().filter(|state| state.active) {
            let compact = state_map.len();
            state_map.insert(state.id, compact);
        }

        let states = self
            .states
            .into_iter()
            .filter(|state| state.active)
            .map(|state| {
                let transitions = state
                    .transitions
                    .into_iter()
                    .filter(|transition| {
                        let transition = &self.transitions[transition.index()];
                        state_map.contains_key(&transition.target)
                    })
                    .collect();
                FinalizedState {
                    original: state.id,
                    kind: state.kind,
                    rule: state.rule,
                    rule_index: state.rule_index,
                    end_state: state
                        .end_state
                        .and_then(|target| state_map.get(&target).copied()),
                    loop_back_state: state
                        .loop_back_state
                        .and_then(|target| state_map.get(&target).copied()),
                    non_greedy: state.non_greedy,
                    left_recursive_rule: state.left_recursive_rule,
                    transitions,
                }
            })
            .collect();
        let transitions = self
            .transitions
            .into_iter()
            .filter(|transition| {
                state_map.contains_key(&transition.source)
                    && state_map.contains_key(&transition.target)
            })
            .map(|transition| FinalizedTransition {
                original: transition.id,
                source: state_map[&transition.source],
                target: state_map[&transition.target],
                kind: finalize_transition_kind(transition.kind, &state_map),
            })
            .collect();
        FinalizedAtnGraph {
            max_token_type: self.max_token_type,
            states,
            transitions,
            decisions: self
                .decisions
                .into_iter()
                .filter_map(|state| state_map.get(&state).copied())
                .collect(),
            rule_starts: self
                .rule_starts
                .into_iter()
                .map(|state| state_map[&state])
                .collect(),
            rule_stops: self
                .rule_stops
                .into_iter()
                .map(|state| state_map[&state])
                .collect(),
            state_map,
        }
    }
}

fn finalize_transition_kind(
    kind: BuildTransitionKind,
    states: &BTreeMap<BuildStateId, usize>,
) -> FinalizedTransitionKind {
    match kind {
        BuildTransitionKind::Epsilon => FinalizedTransitionKind::Epsilon,
        BuildTransitionKind::Atom(label) => FinalizedTransitionKind::Atom(label),
        BuildTransitionKind::Range { start, stop } => {
            FinalizedTransitionKind::Range { start, stop }
        }
        BuildTransitionKind::Set(ranges) => FinalizedTransitionKind::Set(ranges),
        BuildTransitionKind::NotSet(ranges) => FinalizedTransitionKind::NotSet(ranges),
        BuildTransitionKind::Wildcard => FinalizedTransitionKind::Wildcard,
        BuildTransitionKind::Rule {
            rule,
            rule_index,
            follow,
            precedence,
        } => FinalizedTransitionKind::Rule {
            rule,
            rule_index,
            follow: states[&follow],
            precedence,
        },
        BuildTransitionKind::Predicate {
            rule_index,
            predicate_index,
            context_dependent,
        } => FinalizedTransitionKind::Predicate {
            rule_index,
            predicate_index,
            context_dependent,
        },
        BuildTransitionKind::Action {
            rule_index,
            action_index,
            context_dependent,
        } => FinalizedTransitionKind::Action {
            rule_index,
            action_index,
            context_dependent,
        },
        BuildTransitionKind::Precedence(precedence) => {
            FinalizedTransitionKind::Precedence(precedence)
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FinalizedAtnGraph {
    pub(crate) max_token_type: i32,
    pub(crate) states: Vec<FinalizedState>,
    pub(crate) transitions: Vec<FinalizedTransition>,
    pub(crate) decisions: Vec<usize>,
    pub(crate) rule_starts: Vec<usize>,
    pub(crate) rule_stops: Vec<usize>,
    pub(crate) state_map: BTreeMap<BuildStateId, usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct FinalizedState {
    pub(crate) original: BuildStateId,
    pub(crate) kind: AtnStateKind,
    pub(crate) rule: Option<RuleId>,
    pub(crate) rule_index: Option<usize>,
    pub(crate) end_state: Option<usize>,
    pub(crate) loop_back_state: Option<usize>,
    pub(crate) non_greedy: bool,
    pub(crate) left_recursive_rule: bool,
    pub(crate) transitions: Vec<BuildTransitionId>,
}

#[derive(Clone, Debug)]
pub(crate) struct FinalizedTransition {
    pub(crate) original: BuildTransitionId,
    pub(crate) source: usize,
    pub(crate) target: usize,
    pub(crate) kind: FinalizedTransitionKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FinalizedTransitionKind {
    Epsilon,
    Atom(i32),
    Range {
        start: i32,
        stop: i32,
    },
    Set(Vec<(i32, i32)>),
    NotSet(Vec<(i32, i32)>),
    Wildcard,
    Rule {
        rule: RuleId,
        rule_index: usize,
        follow: usize,
        precedence: i32,
    },
    Predicate {
        rule_index: usize,
        predicate_index: usize,
        context_dependent: bool,
    },
    Action {
        rule_index: usize,
        action_index: Option<usize>,
        context_dependent: bool,
    },
    Precedence(i32),
}
