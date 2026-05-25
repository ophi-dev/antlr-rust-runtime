use crate::atn::{Atn, AtnState, Transition};
use crate::dfa::{Dfa, DfaState};
use crate::prediction::{
    AtnConfig, AtnConfigSet, EMPTY_RETURN_STATE, PredictionContext, PredictionContextMergeCache,
    SemanticContext, has_sll_conflict_terminating_prediction,
};
use std::collections::BTreeSet;
use std::rc::Rc;

#[derive(Debug)]
pub struct ParserAtnSimulator<'a> {
    atn: &'a Atn,
    decision_to_dfa: Vec<Dfa>,
}

impl<'a> ParserAtnSimulator<'a> {
    pub fn new(atn: &'a Atn) -> Self {
        let decision_to_dfa = atn
            .decision_to_state()
            .iter()
            .copied()
            .enumerate()
            .map(|(decision, state)| {
                Dfa::with_max_token_type(state, decision, atn.max_token_type())
            })
            .collect();
        Self {
            atn,
            decision_to_dfa,
        }
    }

    pub fn decision_dfas(&self) -> &[Dfa] {
        &self.decision_to_dfa
    }

    pub fn adaptive_predict(
        &mut self,
        decision: usize,
        lookahead: impl IntoIterator<Item = i32>,
    ) -> Result<usize, ParserAtnSimulatorError> {
        let Some(&decision_state) = self.atn.decision_to_state().get(decision) else {
            return Err(ParserAtnSimulatorError::UnknownDecision(decision));
        };
        let mut state_number = self.ensure_start_state(decision, decision_state)?;
        if let Some(prediction) = self.dfa_prediction(decision, state_number) {
            return Ok(prediction);
        }
        for symbol in lookahead {
            if let Some(target) = self
                .decision_to_dfa
                .get(decision)
                .and_then(|dfa| dfa.state(state_number))
                .and_then(|state| state.edge(symbol))
            {
                state_number = target;
            } else {
                let configs = self
                    .decision_to_dfa
                    .get(decision)
                    .and_then(|dfa| dfa.state(state_number))
                    .map(|state| state.configs.clone())
                    .ok_or(ParserAtnSimulatorError::MissingDfaState(state_number))?;
                let target = self.compute_target_state(decision, state_number, &configs, symbol)?;
                state_number = target;
            }
            if let Some(prediction) = self.dfa_prediction(decision, state_number) {
                return Ok(prediction);
            }
        }
        Err(ParserAtnSimulatorError::PredictionRequiresMoreLookahead)
    }

    fn ensure_start_state(
        &mut self,
        decision: usize,
        decision_state: usize,
    ) -> Result<usize, ParserAtnSimulatorError> {
        if let Some(start) = self.decision_to_dfa[decision].start_state() {
            return Ok(start);
        }
        let decision_state = self
            .atn
            .state(decision_state)
            .ok_or(ParserAtnSimulatorError::MissingAtnState(decision_state))?;
        let configs = self.compute_start_state(decision_state)?;
        let state_number = self.decision_to_dfa[decision].add_state(DfaState::new(configs));
        self.decision_to_dfa[decision].set_start_state(state_number);
        Ok(state_number)
    }

    fn compute_start_state(
        &self,
        decision_state: &AtnState,
    ) -> Result<AtnConfigSet, ParserAtnSimulatorError> {
        let mut configs = AtnConfigSet::new();
        let mut merge_cache = PredictionContextMergeCache::new();
        for (index, transition) in decision_state.transitions.iter().enumerate() {
            let alt = index + 1;
            let config = AtnConfig::new(transition.target(), alt, PredictionContext::empty());
            self.closure(config, &mut configs, &mut merge_cache)?;
        }
        Ok(configs)
    }

    fn compute_target_state(
        &mut self,
        decision: usize,
        source_state: usize,
        configs: &AtnConfigSet,
        symbol: i32,
    ) -> Result<usize, ParserAtnSimulatorError> {
        let mut reach = AtnConfigSet::new();
        let mut merge_cache = PredictionContextMergeCache::new();
        for config in configs.configs() {
            let Some(state) = self.atn.state(config.state) else {
                continue;
            };
            for transition in &state.transitions {
                if transition.matches(symbol, 1, self.atn.max_token_type()) {
                    let target = AtnConfig {
                        state: transition.target(),
                        alt: config.alt,
                        context: Rc::clone(&config.context),
                        semantic_context: config.semantic_context.clone(),
                        reaches_into_outer_context: config.reaches_into_outer_context,
                        precedence_filter_suppressed: config.precedence_filter_suppressed,
                    };
                    self.closure(target, &mut reach, &mut merge_cache)?;
                }
            }
        }
        if reach.is_empty() {
            return Err(ParserAtnSimulatorError::NoViableAlt { symbol });
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
        let mut dfa_state = DfaState::new(reach);
        if let Some(prediction) = conflict_prediction {
            dfa_state.mark_accept(prediction);
            dfa_state.requires_full_context = requires_full_context;
        }
        let target_state = self.decision_to_dfa[decision].add_state(dfa_state);
        if let Some(source) = self.decision_to_dfa[decision].state_mut(source_state) {
            source.add_edge(symbol, target_state);
        }
        Ok(target_state)
    }

    fn closure(
        &self,
        config: AtnConfig,
        configs: &mut AtnConfigSet,
        merge_cache: &mut PredictionContextMergeCache,
    ) -> Result<(), ParserAtnSimulatorError> {
        let mut stack = vec![config];
        let mut visited = BTreeSet::new();
        while let Some(config) = stack.pop() {
            if !visited.insert(config.clone()) {
                continue;
            }
            let Some(state) = self.atn.state(config.state) else {
                continue;
            };
            if state.is_rule_stop() {
                self.closure_at_rule_stop(config, configs, merge_cache)?;
                continue;
            }
            if state.transitions.iter().any(Transition::is_epsilon) {
                for transition in &state.transitions {
                    if transition.is_epsilon() {
                        stack.push(self.epsilon_target_config(&config, transition));
                    }
                }
            } else {
                configs.add_with_merge_cache(config, Some(merge_cache));
            }
        }
        Ok(())
    }

    fn closure_at_rule_stop(
        &self,
        config: AtnConfig,
        configs: &mut AtnConfigSet,
        merge_cache: &mut PredictionContextMergeCache,
    ) -> Result<(), ParserAtnSimulatorError> {
        if config.context.is_empty() {
            configs.add_with_merge_cache(config, Some(merge_cache));
            return Ok(());
        }
        for index in 0..config.context.len() {
            let Some(return_state) = config.context.return_state(index) else {
                continue;
            };
            if return_state == EMPTY_RETURN_STATE {
                configs.add_with_merge_cache(config.clone(), Some(merge_cache));
                continue;
            }
            let parent = config
                .context
                .parent(index)
                .unwrap_or_else(PredictionContext::empty);
            let next = AtnConfig {
                state: return_state,
                alt: config.alt,
                context: parent,
                semantic_context: config.semantic_context.clone(),
                reaches_into_outer_context: config.reaches_into_outer_context,
                precedence_filter_suppressed: config.precedence_filter_suppressed,
            };
            self.closure(next, configs, merge_cache)?;
        }
        Ok(())
    }

    fn epsilon_target_config(&self, config: &AtnConfig, transition: &Transition) -> AtnConfig {
        let semantic_context = match transition {
            Transition::Predicate {
                rule_index,
                pred_index,
                context_dependent,
                ..
            } => SemanticContext::and(
                config.semantic_context.clone(),
                SemanticContext::Predicate {
                    rule_index: *rule_index,
                    pred_index: *pred_index,
                    context_dependent: *context_dependent,
                },
            ),
            Transition::Precedence { precedence, .. } => SemanticContext::and(
                config.semantic_context.clone(),
                SemanticContext::Precedence {
                    precedence: *precedence,
                },
            ),
            _ => config.semantic_context.clone(),
        };
        let context = match transition {
            Transition::Rule { follow_state, .. } => {
                PredictionContext::singleton(Rc::clone(&config.context), *follow_state)
            }
            _ => Rc::clone(&config.context),
        };
        AtnConfig {
            state: transition.target(),
            alt: config.alt,
            context,
            semantic_context,
            reaches_into_outer_context: config.reaches_into_outer_context,
            precedence_filter_suppressed: config.precedence_filter_suppressed,
        }
    }

    fn dfa_prediction(&self, decision: usize, state_number: usize) -> Option<usize> {
        self.decision_to_dfa
            .get(decision)
            .and_then(|dfa| dfa.state(state_number))
            .and_then(|state| state.prediction)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParserAtnSimulatorError {
    MissingAtnState(usize),
    MissingDfaState(usize),
    NoViableAlt { symbol: i32 },
    PredictionRequiresMoreLookahead,
    UnknownDecision(usize),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atn::{AtnStateKind, AtnType};

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
    fn adaptive_predict_reports_no_viable_alt() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(
            simulator.adaptive_predict(0, [4]),
            Err(ParserAtnSimulatorError::NoViableAlt { symbol: 4 })
        );
    }

    #[test]
    fn adaptive_predict_marks_sll_conflict_for_full_context() {
        let atn = ambiguous_single_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);

        assert_eq!(simulator.adaptive_predict(0, [1]), Ok(1));

        let dfa = &simulator.decision_dfas()[0];
        let start = dfa.start_state().expect("start state");
        let target = dfa
            .state(start)
            .and_then(|state| state.edge(1))
            .expect("edge for token 1");
        let state = dfa.state(target).expect("target state");
        assert!(state.is_accept_state);
        assert!(state.requires_full_context);
        assert_eq!(state.prediction, Some(1));
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

    fn add_state(atn: &mut Atn, state_number: usize, kind: AtnStateKind) {
        atn.add_state(AtnState::new(state_number, kind).with_rule_index(0));
    }
}
