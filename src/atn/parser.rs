use crate::atn::{Atn, AtnState, AtnStateKind, Transition};
use crate::dfa::{Dfa, DfaState};
use crate::int_stream::IntStream;
use crate::prediction::{
    AtnConfig, AtnConfigSet, EMPTY_RETURN_STATE, PredictionContext, PredictionContextCache,
    PredictionContextMergeCache, PredictionFxHasher, SemanticContext,
    has_sll_conflict_terminating_prediction, resolves_to_just_one_viable_alt,
};
use crate::token::TOKEN_EOF;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;
use std::rc::Rc;

type FxHashSet<T> = HashSet<T, BuildHasherDefault<PredictionFxHasher>>;

#[derive(Debug)]
pub struct ParserAtnSimulator<'a> {
    atn: &'a Atn,
    decision_to_dfa: Vec<Dfa>,
    shared_cache_key: Option<usize>,
    context_cache: Rc<RefCell<PredictionContextCache>>,
}

thread_local! {
    static SHARED_DECISION_DFAS: RefCell<HashMap<usize, Vec<Dfa>>> = RefCell::new(HashMap::new());
    static SHARED_CONTEXT_CACHES: RefCell<HashMap<usize, Rc<RefCell<PredictionContextCache>>>> =
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParserAtnPredictionDiagnosticKind {
    Ambiguity,
    ContextSensitivity,
}

#[derive(Clone, Copy)]
struct PredictionCheck<'a> {
    decision: usize,
    decision_state: usize,
    state_number: usize,
    start_index: usize,
    precedence: i32,
    outer_context: &'a Rc<PredictionContext>,
    force_full_context_retry: bool,
}

#[derive(Clone, Copy)]
struct AdaptivePredictRequest<'a> {
    decision: usize,
    precedence: usize,
    outer_context: &'a Rc<PredictionContext>,
    force_full_context_retry: bool,
}

#[derive(Clone, Copy)]
struct DfaEdge {
    decision: usize,
    source_state: usize,
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
    ambiguity_alts: Option<Vec<usize>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ClosureConfigKey {
    state: usize,
    alt: usize,
    context: Rc<PredictionContext>,
    semantic_context: SemanticContext,
    precedence_filter_suppressed: bool,
}

impl From<&AtnConfig> for ClosureConfigKey {
    fn from(config: &AtnConfig) -> Self {
        Self {
            state: config.state,
            alt: config.alt,
            context: Rc::clone(&config.context),
            semantic_context: config.semantic_context.clone(),
            precedence_filter_suppressed: config.precedence_filter_suppressed,
        }
    }
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

fn initial_decision_dfas(atn: &Atn) -> Vec<Dfa> {
    atn.decision_to_state()
        .iter()
        .copied()
        .enumerate()
        .map(|(decision, state)| {
            let mut dfa = Dfa::with_max_token_type(state, decision, atn.max_token_type());
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

fn merge_shared_decision_dfas(shared: &mut Vec<Dfa>, local: &[Dfa]) {
    if shared.len() != local.len() {
        *shared = local.to_vec();
        return;
    }
    for (shared_dfa, local_dfa) in shared.iter_mut().zip(local) {
        // State numbers are stable for a DFA cloned from the shared cache and
        // then extended locally. If this local DFA has at least as many states,
        // it is a complete valid replacement for the shared one. If it is
        // smaller, it may be a stale parser instance that predates another
        // parser's cache update, so do not merge edges by numeric state id.
        if local_dfa.states().len() >= shared_dfa.states().len() {
            *shared_dfa = local_dfa.clone();
        }
    }
}

impl Drop for ParserAtnSimulator<'_> {
    fn drop(&mut self) {
        let Some(key) = self.shared_cache_key else {
            return;
        };
        SHARED_DECISION_DFAS.with(|cache| {
            let mut cache = cache.borrow_mut();
            if let Some(shared) = cache.get_mut(&key) {
                merge_shared_decision_dfas(shared, &self.decision_to_dfa);
            } else {
                cache.insert(key, self.decision_to_dfa.clone());
            }
        });
    }
}

impl<'a> ParserAtnSimulator<'a> {
    pub fn new(atn: &'a Atn) -> Self {
        Self {
            atn,
            decision_to_dfa: initial_decision_dfas(atn),
            shared_cache_key: None,
            context_cache: Rc::new(RefCell::new(PredictionContextCache::new())),
        }
    }

    /// Creates a simulator that starts from, and publishes back into, a
    /// thread-local DFA cache keyed by a generated parser's static ATN.
    ///
    /// Generated parsers usually create a fresh parser object per parse. Without
    /// this cache every parse relearns the same adaptive DFA; with it, later
    /// parser instances reuse the SLL cache learned by earlier instances while
    /// still keeping mutable simulator state local to the parser during a parse.
    pub fn new_shared(atn: &'static Atn) -> Self {
        let ptr: *const Atn = atn;
        let key = ptr as usize;
        let decision_to_dfa = SHARED_DECISION_DFAS
            .with(|cache| cache.borrow().get(&key).cloned())
            .unwrap_or_else(|| initial_decision_dfas(atn));
        let context_cache = SHARED_CONTEXT_CACHES.with(|cache| {
            Rc::clone(
                cache
                    .borrow_mut()
                    .entry(key)
                    .or_insert_with(|| Rc::new(RefCell::new(PredictionContextCache::new()))),
            )
        });
        Self {
            atn,
            decision_to_dfa,
            shared_cache_key: Some(key),
            context_cache,
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
        let empty = PredictionContext::empty();
        let marker = input.mark();
        let index = input.index();
        let mut merge_cache = PredictionContextMergeCache::new();
        let result = self.adaptive_predict_stream_inner(
            AdaptivePredictRequest {
                decision,
                precedence,
                outer_context: &empty,
                force_full_context_retry: false,
            },
            input,
            &mut merge_cache,
        );
        input.seek(index);
        input.release(marker);
        result
    }

    pub fn adaptive_predict_stream_info_with_context<T: IntStream>(
        &mut self,
        decision: usize,
        precedence: usize,
        input: &mut T,
        outer_context: &Rc<PredictionContext>,
    ) -> Result<ParserAtnPrediction, ParserAtnSimulatorError> {
        let marker = input.mark();
        let index = input.index();
        let mut merge_cache = PredictionContextMergeCache::new();
        let result = self.adaptive_predict_stream_inner(
            AdaptivePredictRequest {
                decision,
                precedence,
                outer_context,
                force_full_context_retry: true,
            },
            input,
            &mut merge_cache,
        );
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
        request: AdaptivePredictRequest<'_>,
        input: &mut T,
        merge_cache: &mut PredictionContextMergeCache,
    ) -> Result<ParserAtnPrediction, ParserAtnSimulatorError> {
        let AdaptivePredictRequest {
            decision,
            precedence,
            outer_context,
            force_full_context_retry,
        } = request;
        let Some(&decision_state) = self.atn.decision_to_state().get(decision) else {
            return Err(ParserAtnSimulatorError::UnknownDecision(decision));
        };
        let start_index = input.index();
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
            },
            merge_cache,
        )? {
            return Ok(prediction);
        }
        loop {
            let symbol = input.la(1);
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
                },
                merge_cache,
            )? {
                return Ok(prediction);
            }
            if symbol == TOKEN_EOF {
                return Err(ParserAtnSimulatorError::PredictionRequiresMoreLookahead);
            }
            input.consume();
        }
    }

    fn prediction_or_full_context<T: IntStream>(
        &self,
        input: &mut T,
        check: PredictionCheck<'_>,
        merge_cache: &mut PredictionContextMergeCache,
    ) -> Result<Option<ParserAtnPrediction>, ParserAtnSimulatorError> {
        let PredictionCheck {
            decision,
            decision_state,
            state_number,
            start_index,
            precedence,
            outer_context,
            force_full_context_retry,
        } = check;
        if outer_context.is_empty()
            && let Some(prediction) =
                self.non_greedy_exit_prediction(decision, decision_state, state_number)
        {
            return Ok(Some(prediction));
        }
        let Some(info) = self.dfa_prediction_info(decision, state_number) else {
            return Ok(None);
        };
        let prediction = info.prediction;
        if prediction.requires_full_context
            && (force_full_context_retry || !prediction.has_semantic_context)
        {
            let sll_stop_index = input.index();
            input.seek(start_index);
            let full_context = self.adaptive_predict_full_context(
                decision_state,
                input,
                precedence,
                outer_context,
                merge_cache,
            )?;
            let (kind, conflicting_alts) = if let Some(ambiguity_alts) = full_context.ambiguity_alts
            {
                (ParserAtnPredictionDiagnosticKind::Ambiguity, ambiguity_alts)
            } else {
                (
                    ParserAtnPredictionDiagnosticKind::ContextSensitivity,
                    info.conflicting_alts,
                )
            };
            let mut prediction = full_context.prediction;
            if conflicting_alts.len() > 1 {
                prediction.diagnostic = Some(ParserAtnPredictionDiagnostic {
                    kind,
                    start_index,
                    sll_stop_index,
                    ll_stop_index: full_context.stop_index,
                    conflicting_alts,
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
        state_number: usize,
    ) -> Option<ParserAtnPrediction> {
        if !self
            .atn
            .state(decision_state)
            .is_some_and(|state| state.non_greedy)
        {
            return None;
        }
        let configs = &self
            .decision_to_dfa
            .get(decision)?
            .state(state_number)?
            .configs;
        let alt = configs
            .configs()
            .iter()
            .filter(|config| {
                self.atn
                    .state(config.state)
                    .is_some_and(AtnState::is_rule_stop)
                    && config.context.has_empty_path()
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
        merge_cache: &mut PredictionContextMergeCache,
    ) -> Result<usize, ParserAtnSimulatorError> {
        if self.decision_to_dfa[decision].is_precedence_dfa() {
            let precedence_key = usize::try_from(precedence.max(0)).unwrap_or_default();
            if let Some(start) =
                self.decision_to_dfa[decision].precedence_start_state(precedence_key)
            {
                return Ok(start);
            }
        } else if let Some(start) = self.decision_to_dfa[decision].start_state() {
            return Ok(start);
        }
        let decision_state = self
            .atn
            .state(decision_state)
            .ok_or(ParserAtnSimulatorError::MissingAtnState(decision_state))?;
        let configs = self.compute_start_state(decision_state, precedence, merge_cache);
        let state_number = self.add_dfa_state(decision, DfaState::new(configs));
        if self.decision_to_dfa[decision].is_precedence_dfa() {
            let precedence_key = usize::try_from(precedence.max(0)).unwrap_or_default();
            self.decision_to_dfa[decision].set_precedence_start_state(precedence_key, state_number);
        } else {
            self.decision_to_dfa[decision].set_start_state(state_number);
        }
        Ok(state_number)
    }

    fn add_dfa_state(&mut self, decision: usize, mut state: DfaState) -> usize {
        if state.configs.is_readonly() {
            return self.decision_to_dfa[decision].add_state(state);
        }
        if let Some(existing) =
            self.decision_to_dfa[decision].state_number_for_configs(&state.configs)
        {
            return existing;
        }
        state
            .configs
            .optimize_contexts(&mut self.context_cache.borrow_mut());
        self.decision_to_dfa[decision].insert_state(state)
    }

    fn compute_start_state(
        &self,
        decision_state: &AtnState,
        precedence: i32,
        merge_cache: &mut PredictionContextMergeCache,
    ) -> AtnConfigSet {
        let empty = PredictionContext::empty();
        self.compute_start_state_with_context(
            decision_state,
            false,
            &empty,
            precedence,
            merge_cache,
        )
    }

    fn compute_start_state_with_context(
        &self,
        decision_state: &AtnState,
        full_context: bool,
        initial_context: &Rc<PredictionContext>,
        precedence: i32,
        merge_cache: &mut PredictionContextMergeCache,
    ) -> AtnConfigSet {
        let mut configs = AtnConfigSet::new_full_context(full_context);
        for (index, transition) in decision_state.transitions.iter().enumerate() {
            let alt = index + 1;
            let config = AtnConfig::new(transition.target(), alt, Rc::clone(initial_context));
            self.closure(config, &mut configs, merge_cache, precedence, false);
        }
        configs
    }

    fn adaptive_predict_full_context<T: IntStream>(
        &self,
        decision_state: usize,
        input: &mut T,
        precedence: i32,
        outer_context: &Rc<PredictionContext>,
        merge_cache: &mut PredictionContextMergeCache,
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
        loop {
            if let Some(alt) = configs.unique_alt() {
                return Ok(FullContextPrediction {
                    prediction: ParserAtnPrediction {
                        alt,
                        requires_full_context: true,
                        has_semantic_context: configs_have_semantic_context_for_alt(&configs, alt),
                        diagnostic: None,
                    },
                    stop_index: input.index(),
                    ambiguity_alts: None,
                });
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
                return Ok(FullContextPrediction {
                    prediction: ParserAtnPrediction {
                        alt,
                        requires_full_context: true,
                        has_semantic_context: configs_have_semantic_context_for_alt(&configs, alt),
                        diagnostic: None,
                    },
                    stop_index: input.index(),
                    ambiguity_alts: None,
                });
            }
            if symbol == TOKEN_EOF || self.configs_all_reached_rule_stop(&configs) {
                let alts = configs.alts();
                let alt = alts
                    .iter()
                    .next()
                    .copied()
                    .ok_or(ParserAtnSimulatorError::PredictionRequiresMoreLookahead)?;
                return Ok(FullContextPrediction {
                    prediction: ParserAtnPrediction {
                        alt,
                        requires_full_context: true,
                        has_semantic_context: configs_have_semantic_context_for_alt(&configs, alt),
                        diagnostic: None,
                    },
                    stop_index: input.index(),
                    ambiguity_alts: (alts.len() > 1).then(|| alts.into_iter().collect()),
                });
            }
            if !configs.has_semantic_context()
                && let Some(alt) = resolves_to_just_one_viable_alt(configs.configs())
            {
                return Ok(FullContextPrediction {
                    prediction: ParserAtnPrediction {
                        alt,
                        requires_full_context: true,
                        has_semantic_context: configs_have_semantic_context_for_alt(&configs, alt),
                        diagnostic: None,
                    },
                    stop_index: input.index(),
                    ambiguity_alts: None,
                });
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
        merge_cache: &mut PredictionContextMergeCache,
    ) -> Result<usize, ParserAtnSimulatorError> {
        let mut reach = self.compute_reach_set(configs, symbol, false, precedence, merge_cache);
        if reach.is_empty() {
            if let Some(prediction) = self.alt_that_finished_decision_entry_rule(configs) {
                let mut dfa_state = DfaState::new(configs.clone());
                dfa_state.mark_accept(prediction);
                let target_state = self.add_dfa_state(edge.decision, dfa_state);
                if let Some(source) =
                    self.decision_to_dfa[edge.decision].state_mut(edge.source_state)
                {
                    source.add_edge(symbol, target_state);
                }
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
        let conflicting_alts = if requires_full_context {
            let alts = reach.conflicting_alts();
            if alts.is_empty() { reach.alts() } else { alts }
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        let mut dfa_state = DfaState::new(reach);
        if let Some(prediction) = conflict_prediction {
            dfa_state.mark_accept(prediction);
            dfa_state.requires_full_context = requires_full_context;
            dfa_state.conflicting_alts = conflicting_alts;
        }
        let target_state = self.add_dfa_state(edge.decision, dfa_state);
        if let Some(source) = self.decision_to_dfa[edge.decision].state_mut(edge.source_state) {
            source.add_edge(symbol, target_state);
        }
        Ok(target_state)
    }

    fn compute_reach_set(
        &self,
        configs: &AtnConfigSet,
        symbol: i32,
        full_context: bool,
        precedence: i32,
        merge_cache: &mut PredictionContextMergeCache,
    ) -> AtnConfigSet {
        let mut reach = AtnConfigSet::new_full_context(full_context);
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
                    let target = AtnConfig {
                        state: transition.target(),
                        alt: config.alt,
                        context: Rc::clone(&config.context),
                        semantic_context: config.semantic_context.clone(),
                        reaches_into_outer_context: config.reaches_into_outer_context,
                        precedence_filter_suppressed: config.precedence_filter_suppressed,
                    };
                    self.closure(
                        target,
                        &mut reach,
                        merge_cache,
                        precedence,
                        symbol == TOKEN_EOF,
                    );
                }
            }
        }
        if symbol == TOKEN_EOF {
            reach = self.rule_stop_configs(reach, merge_cache);
        }
        if !full_context || !self.configs_contain_rule_stop(&reach) {
            for config in skipped_stop_states {
                reach.add_with_merge_cache(config, Some(merge_cache));
            }
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
                        && config.context.has_empty_path()
            })
            .map(|config| config.alt)
            .min()
    }

    fn rule_stop_configs(
        &self,
        configs: AtnConfigSet,
        merge_cache: &mut PredictionContextMergeCache,
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
            result.add_with_merge_cache(config.clone(), Some(merge_cache));
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
        &self,
        config: AtnConfig,
        configs: &mut AtnConfigSet,
        merge_cache: &mut PredictionContextMergeCache,
        precedence: i32,
        treat_eof_as_epsilon: bool,
    ) {
        let mut stack = vec![config];
        let mut visited = FxHashSet::<ClosureConfigKey>::default();
        while let Some(config) = stack.pop() {
            if !visited.insert(ClosureConfigKey::from(&config)) {
                continue;
            }
            let Some(state) = self.atn.state(config.state) else {
                continue;
            };
            let at_rule_stop = state.is_rule_stop();
            if at_rule_stop
                && self.closure_at_rule_stop(config.clone(), configs, merge_cache, &mut stack)
            {
                continue;
            }
            let epsilon_only = !state.transitions.is_empty()
                && state.transitions.iter().all(Transition::is_epsilon);
            if !epsilon_only {
                configs.add_with_merge_cache(config.clone(), Some(merge_cache));
            }
            for (index, transition) in state.transitions.iter().enumerate() {
                if index == 0
                    && can_drop_left_recursive_loop_entry_edge(self.atn, state, &config.context)
                {
                    continue;
                }
                if transition.is_epsilon() {
                    if let Some(mut target) =
                        self.epsilon_target_config(&config, transition, precedence)
                    {
                        if at_rule_stop {
                            target.reaches_into_outer_context =
                                target.reaches_into_outer_context.saturating_add(1);
                        }
                        stack.push(target);
                    }
                } else if treat_eof_as_epsilon
                    && transition.matches(TOKEN_EOF, 1, self.atn.max_token_type())
                {
                    stack.push(AtnConfig {
                        state: transition.target(),
                        alt: config.alt,
                        context: Rc::clone(&config.context),
                        semantic_context: config.semantic_context.clone(),
                        reaches_into_outer_context: config.reaches_into_outer_context,
                        precedence_filter_suppressed: config.precedence_filter_suppressed,
                    });
                }
            }
        }
    }

    fn closure_at_rule_stop(
        &self,
        config: AtnConfig,
        configs: &mut AtnConfigSet,
        merge_cache: &mut PredictionContextMergeCache,
        stack: &mut Vec<AtnConfig>,
    ) -> bool {
        if config.context.is_empty() {
            if configs.full_context() {
                configs.add_with_merge_cache(config, Some(merge_cache));
                return true;
            }
            return false;
        }
        let mut handled_all_paths = true;
        for index in 0..config.context.len() {
            let Some(return_state) = config.context.return_state(index) else {
                continue;
            };
            if return_state == EMPTY_RETURN_STATE {
                if configs.full_context() {
                    let mut empty_context_config = config.clone();
                    empty_context_config.context = PredictionContext::empty();
                    configs.add_with_merge_cache(empty_context_config, Some(merge_cache));
                } else {
                    handled_all_paths = false;
                }
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
            stack.push(next);
        }
        handled_all_paths
    }

    fn epsilon_target_config(
        &self,
        config: &AtnConfig,
        transition: &Transition,
        precedence: i32,
    ) -> Option<AtnConfig> {
        if matches!(
            transition,
            Transition::Precedence {
                precedence: transition_precedence,
                ..
            } if *transition_precedence < precedence
        ) {
            return None;
        }
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
        Some(AtnConfig {
            state: transition.target(),
            alt: config.alt,
            context,
            semantic_context,
            reaches_into_outer_context: config.reaches_into_outer_context,
            precedence_filter_suppressed: config.precedence_filter_suppressed,
        })
    }

    fn dfa_prediction_info(
        &self,
        decision: usize,
        state_number: usize,
    ) -> Option<DfaPredictionInfo> {
        self.decision_to_dfa
            .get(decision)
            .and_then(|dfa| dfa.state(state_number))
            .and_then(|state| {
                state.prediction.map(|alt| {
                    let conflicting_alts = if state.requires_full_context {
                        if state.conflicting_alts.is_empty() {
                            state.configs.alts().into_iter().collect()
                        } else {
                            state.conflicting_alts.clone()
                        }
                    } else {
                        Vec::new()
                    };
                    DfaPredictionInfo {
                        prediction: ParserAtnPrediction {
                            alt,
                            requires_full_context: state.requires_full_context,
                            has_semantic_context: configs_have_semantic_context_for_alt(
                                &state.configs,
                                alt,
                            ),
                            diagnostic: None,
                        },
                        conflicting_alts,
                    }
                })
            })
    }
}

/// Reports whether closure should skip the loop-entry branch for a
/// left-recursive rule under the current caller context.
pub(crate) fn can_drop_left_recursive_loop_entry_edge(
    atn: &Atn,
    state: &AtnState,
    context: &PredictionContext,
) -> bool {
    if state.kind != AtnStateKind::StarLoopEntry
        || !state.precedence_rule_decision
        || context.is_empty()
        || context.has_empty_path()
    {
        return false;
    }
    let Some(rule_index) = state.rule_index else {
        return false;
    };
    for index in 0..context.len() {
        let Some(return_state_number) = context.return_state(index) else {
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
    for index in 0..context.len() {
        let return_state_number = context
            .return_state(index)
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
    MissingDfaState(usize),
    NoViableAlt { symbol: i32, index: usize },
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
        assert!(state.is_accept_state);
        assert!(state.requires_full_context);
        assert_eq!(state.prediction, Some(1));
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
                }),
            }
        );
        assert_eq!(input.index(), 0);
    }

    #[test]
    fn context_prediction_reports_context_sensitivity_for_dfa_conflict() {
        let atn = two_token_decision_atn();
        let mut simulator = ParserAtnSimulator::new(&atn);
        let empty = PredictionContext::empty();
        let mut start_configs = AtnConfigSet::new();
        start_configs.add(AtnConfig::new(2, 1, Rc::clone(&empty)));
        let start = simulator.decision_to_dfa[0].add_state(DfaState::new(start_configs));
        simulator.decision_to_dfa[0].set_start_state(start);

        let mut accept_configs = AtnConfigSet::new();
        accept_configs.add(
            AtnConfig::new(3, 1, Rc::clone(&empty)).with_semantic_context(
                SemanticContext::Predicate {
                    rule_index: 0,
                    pred_index: 0,
                    context_dependent: false,
                },
            ),
        );
        let mut accept_state = DfaState::new(accept_configs);
        accept_state.mark_accept(1);
        accept_state.requires_full_context = true;
        accept_state.conflicting_alts = vec![1, 2];
        let accept = simulator.decision_to_dfa[0].add_state(accept_state);
        simulator.decision_to_dfa[0]
            .state_mut(start)
            .expect("start state")
            .add_edge(1, accept);

        let mut input = VecIntStream::new(vec![1, 3, TOKEN_EOF]);
        let prediction = simulator
            .adaptive_predict_stream_info_with_context(0, 0, &mut input, &empty)
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
                }),
            }
        );
        assert_eq!(input.index(), 0);
    }

    #[test]
    fn full_context_reach_prefers_longer_match_over_skipped_stop_state() {
        let atn = prefix_alt_decision_atn();
        let simulator = ParserAtnSimulator::new(&atn);
        let empty = PredictionContext::empty();
        let mut configs = AtnConfigSet::new_full_context(true);
        configs.add(AtnConfig::new(2, 1, Rc::clone(&empty)));
        configs.add(AtnConfig::new(1, 2, empty));
        let mut merge_cache = PredictionContextMergeCache::new();

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

        let simulator = ParserAtnSimulator::new(&atn);
        let mut configs = AtnConfigSet::new_full_context(false);
        let mut merge_cache = PredictionContextMergeCache::new();
        simulator.closure(
            AtnConfig::new(0, 2, PredictionContext::empty()),
            &mut configs,
            &mut merge_cache,
            0,
            false,
        );

        assert_eq!(configs.len(), 1);
        let config = &configs.configs()[0];
        assert_eq!(config.state, 1);
        assert_eq!(config.alt, 2);
        assert_eq!(config.reaches_into_outer_context, 1);
    }

    #[test]
    fn semantic_context_flag_is_scoped_to_predicted_alt() {
        let empty = PredictionContext::empty();
        let mut configs = AtnConfigSet::new();
        configs.add(AtnConfig::new(1, 1, Rc::clone(&empty)));
        configs.add(AtnConfig::new(2, 2, empty).with_semantic_context(
            SemanticContext::Predicate {
                rule_index: 0,
                pred_index: 0,
                context_dependent: false,
            },
        ));

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
        let same_rule_context = PredictionContext::singleton(PredictionContext::empty(), 4);
        let other_rule_context = PredictionContext::singleton(PredictionContext::empty(), 5);

        assert!(can_drop_left_recursive_loop_entry_edge(
            &atn,
            loop_entry,
            &same_rule_context
        ));
        assert!(!can_drop_left_recursive_loop_entry_edge(
            &atn,
            loop_entry,
            &other_rule_context
        ));
        assert!(!can_drop_left_recursive_loop_entry_edge(
            &atn,
            loop_entry,
            &PredictionContext::empty()
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
