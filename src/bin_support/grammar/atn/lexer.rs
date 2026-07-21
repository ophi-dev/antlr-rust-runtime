use std::collections::{BTreeMap, BTreeSet};

use antlr4_runtime::atn::lexer_dfa::CompiledLexerDfa;
use antlr4_runtime::atn::serialized::SERIALIZED_VERSION;
use antlr4_runtime::atn::{
    AtnStateKind, IntervalSet, LexerAction, LexerAtn, LexerAtnState, LexerTransition,
};
use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use super::super::diagnostic::{CompilationError, Diagnostic, Severity};
use super::super::frontend::SourceSpan;
use super::super::model::{
    Alternative, Block, BuildStateId, Element, ElementKind, ModelNodeId, Quantifier,
    ResolvedLexerCommand, Rule, RuleId, SemanticGrammar, SetElement, Terminal,
};
use super::super::provenance::{Origin, ProvenanceIndex, SyntheticReason};
use super::super::unicode::{property_ranges, simple_lowercase, simple_uppercase};
use super::build::{
    BuildGraph, BuildTransitionKind, BuildTransitionSpec, FinalizedAtnGraph, FinalizedTransition,
    FinalizedTransitionKind,
};
use super::optimize::{collapse_lexer_sets, remove_tail_epsilons};

const EOF_CODE_POINT: i32 = -1;
const MAX_CODE_POINT: i32 = 0x10_FFFF;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LexerRuntimeArtifact {
    pub(crate) atn_words: Vec<i32>,
    pub(crate) dfa_words: Vec<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LexerAnalysis {
    pub(crate) nullable_rules: BTreeSet<RuleId>,
    pub(crate) recursive_components: Vec<Vec<RuleId>>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
pub(crate) struct CompiledLexer {
    pub(crate) semantic: SemanticGrammar,
    pub(crate) graph: FinalizedAtnGraph,
    pub(crate) atn: LexerAtn,
    pub(crate) dfa: CompiledLexerDfa,
    pub(crate) runtime_artifact: LexerRuntimeArtifact,
    pub(crate) analysis: LexerAnalysis,
    pub(crate) provenance: ProvenanceIndex,
}

pub(crate) fn compile_lexer(
    grammar: SemanticGrammar,
    mut provenance: ProvenanceIndex,
) -> Result<CompiledLexer, CompilationError> {
    let mut factory = LexerFactory::new(&grammar, &mut provenance);
    factory.build();
    if has_errors(&factory.diagnostics) {
        return Err(CompilationError::new(factory.diagnostics));
    }
    collapse_lexer_sets(&mut factory.graph, factory.provenance);

    let closure_sites = std::mem::take(&mut factory.epsilon_closures);
    let mut diagnostics = std::mem::take(&mut factory.diagnostics);
    let mode_starts = std::mem::take(&mut factory.mode_starts);
    let lexer_actions = std::mem::take(&mut factory.lexer_actions);
    let graph = factory.graph.finalize();
    let analysis = analyze_lexer(&grammar, &graph, closure_sites, &mut diagnostics)?;
    let atn = lower(&grammar, &graph, &mode_starts, lexer_actions);
    let dfa = CompiledLexerDfa::compile(&atn);
    let runtime_artifact = LexerRuntimeArtifact {
        atn_words: encode_lexer_atn(&atn),
        dfa_words: dfa.serialize(),
    };

    Ok(CompiledLexer {
        semantic: grammar,
        graph,
        atn,
        dfa,
        runtime_artifact,
        analysis,
        provenance,
    })
}

fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
}

#[derive(Clone, Copy, Debug)]
struct StatePair {
    left: BuildStateId,
    right: BuildStateId,
}

struct LexerFactory<'a> {
    grammar: &'a SemanticGrammar,
    graph: BuildGraph,
    provenance: &'a mut ProvenanceIndex,
    current_rule: Option<(&'a Rule, usize)>,
    mode_starts: Vec<BuildStateId>,
    lexer_actions: Vec<LexerAction>,
    epsilon_closures: Vec<(RuleId, BuildStateId, BuildStateId)>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> LexerFactory<'a> {
    fn new(grammar: &'a SemanticGrammar, provenance: &'a mut ProvenanceIndex) -> Self {
        Self {
            grammar,
            graph: BuildGraph::new(grammar.recognizer.vocabulary.max_token_type()),
            provenance,
            current_rule: None,
            mode_starts: Vec::new(),
            lexer_actions: Vec::new(),
            epsilon_closures: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn build(&mut self) {
        self.create_mode_starts();
        self.create_rule_boundaries();
        for (rule_index, rule) in self.grammar.unit.rules.iter().enumerate() {
            self.current_rule = Some((rule, rule_index));
            let pair = self.block(&rule.block, Quantifier::One, ModelNodeId::Rule(rule.id));
            let start = self.graph.rule_starts[rule_index];
            let stop = self.graph.rule_stops[rule_index];
            self.synthetic_epsilon(
                start,
                pair.left,
                SyntheticReason::RuleBoundary,
                ModelNodeId::Rule(rule.id),
                false,
            );
            self.synthetic_epsilon(
                pair.right,
                stop,
                SyntheticReason::RuleBoundary,
                ModelNodeId::Rule(rule.id),
                false,
            );
        }
        self.current_rule = None;
        self.link_modes();
    }

    fn create_mode_starts(&mut self) {
        for (index, name) in self.grammar.recognizer.mode_names.iter().enumerate() {
            let owner = if index == 0 {
                ModelNodeId::Grammar(self.grammar.unit.id)
            } else {
                let mode = self
                    .grammar
                    .unit
                    .modes
                    .get(index - 1)
                    .expect("semantic mode table matches grammar modes");
                debug_assert_eq!(&mode.name, name);
                ModelNodeId::Mode(mode.id)
            };
            let start = self.graph.add_synthetic_state(
                AtnStateKind::TokenStart,
                None,
                SyntheticReason::LexerModeStart,
                owner,
                self.provenance,
            );
            self.graph.add_decision(start);
            self.mode_starts.push(start);
        }
    }

    fn create_rule_boundaries(&mut self) {
        for (rule_index, rule) in self.grammar.unit.rules.iter().enumerate() {
            let owner = ModelNodeId::Rule(rule.id);
            let start = self.graph.add_synthetic_state(
                AtnStateKind::RuleStart,
                Some((rule.id, rule_index)),
                SyntheticReason::RuleBoundary,
                owner,
                self.provenance,
            );
            let stop = self.graph.add_synthetic_state(
                AtnStateKind::RuleStop,
                Some((rule.id, rule_index)),
                SyntheticReason::RuleBoundary,
                owner,
                self.provenance,
            );
            self.graph.rule_starts.push(start);
            self.graph.rule_stops.push(stop);
        }
    }

    fn link_modes(&mut self) {
        let rules = self
            .grammar
            .unit
            .rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| !rule.fragment)
            .map(|(index, rule)| {
                let mode = rule.mode.map_or(0, |mode| {
                    self.grammar
                        .unit
                        .modes
                        .iter()
                        .position(|candidate| candidate.id == mode)
                        .map_or(0, |index| index + 1)
                });
                (index, rule.id, mode)
            })
            .collect::<Vec<_>>();
        for (rule_index, rule, mode) in rules {
            self.synthetic_epsilon(
                self.mode_starts[mode],
                self.graph.rule_starts[rule_index],
                SyntheticReason::LexerModeStart,
                ModelNodeId::Rule(rule),
                false,
            );
        }
    }

    fn block(&mut self, block: &Block, quantifier: Quantifier, owner: ModelNodeId) -> StatePair {
        let alternatives = block
            .alternatives
            .iter()
            .map(|alternative| self.alternative(alternative))
            .collect::<Vec<_>>();
        if quantifier == Quantifier::One && alternatives.len() == 1 {
            return alternatives[0];
        }

        let start_kind = match quantifier {
            Quantifier::One | Quantifier::Optional { .. } => AtnStateKind::BlockStart,
            Quantifier::ZeroOrMore { .. } => AtnStateKind::StarBlockStart,
            Quantifier::OneOrMore { .. } => AtnStateKind::PlusBlockStart,
        };
        let start = self.synthetic_state(start_kind, SyntheticReason::BlockBoundary, owner);
        if (quantifier == Quantifier::One && alternatives.len() > 1)
            || matches!(quantifier, Quantifier::Optional { .. })
            || (!matches!(quantifier, Quantifier::One) && alternatives.len() > 1)
        {
            self.graph.add_decision(start);
        }
        let pair = self.make_block(start, &alternatives, owner);
        match quantifier {
            Quantifier::One => pair,
            Quantifier::Optional { greedy } => self.optional(pair, greedy, owner),
            Quantifier::ZeroOrMore { greedy } => self.star(pair, greedy, owner),
            Quantifier::OneOrMore { greedy } => self.plus(pair, greedy, owner),
        }
    }

    fn alternative(&mut self, alternative: &Alternative) -> StatePair {
        let owner = ModelNodeId::Alternative(alternative.id);
        let mut body_elements = alternative
            .elements
            .iter()
            .map(|element| self.element(element))
            .collect::<Vec<_>>();
        if body_elements.is_empty() {
            body_elements.push(self.epsilon_pair(owner));
        }
        let body = self.element_list(&body_elements, owner);
        if alternative.commands.is_empty() {
            return body;
        }

        let mut commands = Vec::with_capacity(alternative.commands.len());
        for (index, command) in alternative.commands.iter().enumerate() {
            let Some(binding) = self.grammar.bindings.commands.get(&(alternative.id, index)) else {
                continue;
            };
            let action = command_action(binding.command);
            let action_index = self.lexer_action_index(action);
            let pair = self.basic_pair(owner);
            self.graph.add_transition(
                BuildTransitionSpec {
                    source: pair.left,
                    target: pair.right,
                    kind: BuildTransitionKind::Action {
                        rule_index: self.current_rule_index(),
                        action_index: Some(action_index),
                        context_dependent: false,
                    },
                    prepend: false,
                },
                [Origin::Authored {
                    syntax: command.syntax,
                    span: command.span.clone(),
                }],
                self.provenance,
            );
            commands.push(pair);
        }
        if commands.is_empty() {
            return body;
        }
        let commands = self.element_list(&commands, owner);
        self.synthetic_epsilon(
            body.right,
            commands.left,
            SyntheticReason::BlockBoundary,
            owner,
            false,
        );
        StatePair {
            left: body.left,
            right: commands.right,
        }
    }

    fn element(&mut self, element: &Element) -> StatePair {
        let owner = ModelNodeId::Element(element.id);
        let base = match &element.kind {
            ElementKind::Terminal(terminal) => self.terminal(element, terminal),
            ElementKind::RuleCall(_) => self.rule_call(element),
            ElementKind::Range(start, stop) => self.range_pair(owner, start, stop, &element.span),
            ElementKind::Set { inverted, elements } => {
                self.set_pair(owner, *inverted, elements, &element.span)
            }
            ElementKind::Block(block) => {
                return self.block(block, element.quantifier, owner);
            }
            ElementKind::Action { id, .. } => self.custom_action_pair(owner, *id),
            ElementKind::Predicate { id, .. } => {
                let binding = self
                    .grammar
                    .bindings
                    .predicates
                    .get(id)
                    .expect("semantic predicate binding exists");
                let pair = self.basic_pair(owner);
                self.authored_transition(
                    pair.left,
                    pair.right,
                    BuildTransitionKind::Predicate {
                        rule_index: self.current_rule_index(),
                        predicate_index: binding.index,
                        context_dependent: binding.context_dependent,
                    },
                    owner,
                );
                pair
            }
            ElementKind::Epsilon => self.epsilon_pair(owner),
        };

        if element.quantifier == Quantifier::One {
            base
        } else {
            self.quantified_atom(base, element.quantifier, owner)
        }
    }

    fn terminal(&mut self, element: &Element, terminal: &Terminal) -> StatePair {
        let owner = ModelNodeId::Element(element.id);
        match terminal {
            Terminal::Wildcard => {
                let pair = self.basic_pair(owner);
                self.authored_transition(
                    pair.left,
                    pair.right,
                    BuildTransitionKind::Wildcard,
                    owner,
                );
                pair
            }
            Terminal::Eof => self.atom_pair(owner, EOF_CODE_POINT),
            Terminal::Literal(literal) => {
                let values = match decode_string_literal(literal) {
                    Ok(values) if !values.is_empty() => values,
                    Ok(_) => {
                        self.diagnostics.push(Diagnostic::error(
                            "G4L001",
                            element.span.clone(),
                            "lexer string literal cannot be empty",
                        ));
                        return self.epsilon_pair(owner);
                    }
                    Err(message) => {
                        self.diagnostics.push(Diagnostic::error(
                            "G4L002",
                            element.span.clone(),
                            message,
                        ));
                        return self.epsilon_pair(owner);
                    }
                };
                let pairs = values
                    .into_iter()
                    .map(|value| self.character_pair(owner, value, value))
                    .collect::<Vec<_>>();
                self.element_list(&pairs, owner)
            }
            Terminal::LexerCharSet(text) => {
                let char_set = match parse_char_set(text) {
                    Ok(char_set) if !char_set.is_empty() => char_set,
                    Ok(_) => {
                        self.diagnostics.push(Diagnostic::error(
                            "G4L001",
                            element.span.clone(),
                            "lexer character set cannot be empty",
                        ));
                        ParsedCharSet::default()
                    }
                    Err(message) => {
                        self.diagnostics.push(Diagnostic::error(
                            "G4L002",
                            element.span.clone(),
                            message,
                        ));
                        ParsedCharSet::default()
                    }
                };
                let ranges = self.finalize_char_set(char_set);
                let pair = self.basic_pair(owner);
                self.authored_transition(
                    pair.left,
                    pair.right,
                    BuildTransitionKind::Set(ranges),
                    owner,
                );
                pair
            }
            Terminal::Token(name) => {
                self.diagnostics.push(Diagnostic::error(
                    "G4L003",
                    element.span.clone(),
                    format!("token reference {name} is not valid as a lexer character"),
                ));
                self.epsilon_pair(owner)
            }
        }
    }

    fn custom_action_pair(
        &mut self,
        owner: ModelNodeId,
        id: super::super::model::ActionId,
    ) -> StatePair {
        let binding = self
            .grammar
            .bindings
            .actions
            .get(&id)
            .expect("semantic action binding exists");
        let action = LexerAction::Custom {
            rule_index: i32::try_from(self.current_rule_index()).expect("rule index exceeds i32"),
            action_index: i32::try_from(binding.index).expect("action index exceeds i32"),
        };
        let action_index = self.lexer_action_index(action);
        let pair = self.basic_pair(owner);
        self.authored_transition(
            pair.left,
            pair.right,
            BuildTransitionKind::Action {
                rule_index: self.current_rule_index(),
                action_index: Some(action_index),
                context_dependent: false,
            },
            owner,
        );
        pair
    }

    fn rule_call(&mut self, element: &Element) -> StatePair {
        let owner = ModelNodeId::Element(element.id);
        let binding = self
            .grammar
            .bindings
            .rule_calls
            .get(&element.id)
            .expect("semantic rule-call binding exists");
        let pair = self.basic_pair(owner);
        let target_index = self.grammar.recognizer.rule_numbers[&binding.target];
        self.authored_transition(
            pair.left,
            self.graph.rule_starts[target_index],
            BuildTransitionKind::Rule {
                rule: binding.target,
                rule_index: target_index,
                follow: pair.right,
                precedence: 0,
            },
            owner,
        );
        pair
    }

    fn range_pair(
        &mut self,
        owner: ModelNodeId,
        start: &str,
        stop: &str,
        span: &SourceSpan,
    ) -> StatePair {
        let start = decode_character_literal(start);
        let stop = decode_character_literal(stop);
        match (start, stop) {
            (Ok(start), Ok(stop)) if start <= stop => self.character_pair(owner, start, stop),
            (Ok(start), Ok(stop)) => {
                self.diagnostics.push(Diagnostic::error(
                    "G4L001",
                    span.clone(),
                    format!("empty character range {start:#x}..{stop:#x}"),
                ));
                self.epsilon_pair(owner)
            }
            (Err(message), _) | (_, Err(message)) => {
                self.diagnostics
                    .push(Diagnostic::error("G4L002", span.clone(), message));
                self.epsilon_pair(owner)
            }
        }
    }

    fn set_pair(
        &mut self,
        owner: ModelNodeId,
        inverted: bool,
        elements: &[SetElement],
        span: &SourceSpan,
    ) -> StatePair {
        let mut char_set = ParsedCharSet::default();
        for element in elements {
            match element {
                SetElement::Terminal {
                    value,
                    span: member_span,
                    ..
                } => match value {
                    Terminal::Literal(literal) => match decode_character_literal(literal) {
                        Ok(value) => char_set.explicit.push((value, value)),
                        Err(_) => self.diagnostics.push(Diagnostic::error(
                            "G4S066",
                            member_span.clone(),
                            format!(
                                "multi-character literals are not allowed in lexer sets: {literal}"
                            ),
                        )),
                    },
                    Terminal::LexerCharSet(text) => match parse_char_set(text) {
                        Ok(parsed) => char_set.extend(parsed),
                        Err(message) => self.diagnostics.push(Diagnostic::error(
                            "G4L002",
                            member_span.clone(),
                            message,
                        )),
                    },
                    Terminal::Token(name) => self.diagnostics.push(Diagnostic::error(
                        "G4S065",
                        member_span.clone(),
                        format!("rule reference {name} is not currently supported in a set"),
                    )),
                    Terminal::Eof => {
                        char_set.explicit.push((EOF_CODE_POINT, EOF_CODE_POINT));
                    }
                    Terminal::Wildcard => {}
                },
                SetElement::Range { start, stop, .. } => {
                    match (
                        decode_character_literal(start),
                        decode_character_literal(stop),
                    ) {
                        (Ok(start), Ok(stop)) if start <= stop => {
                            char_set.explicit.push((start, stop));
                        }
                        (Ok(start), Ok(stop)) => self.diagnostics.push(Diagnostic::error(
                            "G4L001",
                            span.clone(),
                            format!("empty character range {start:#x}..{stop:#x}"),
                        )),
                        (Err(message), _) | (_, Err(message)) => {
                            self.diagnostics.push(Diagnostic::error(
                                "G4L002",
                                span.clone(),
                                message,
                            ));
                        }
                    }
                }
            }
        }
        let ranges = self.finalize_char_set(char_set);
        let pair = self.basic_pair(owner);
        let kind = if inverted {
            BuildTransitionKind::NotSet(ranges)
        } else {
            collapsed_set_kind(ranges)
        };
        self.authored_transition(pair.left, pair.right, kind, owner);
        pair
    }

    fn finalize_char_set(&self, char_set: ParsedCharSet) -> Vec<(i32, i32)> {
        let mut ranges = self.case_fold_ranges(&char_set.explicit);
        ranges.extend(char_set.properties);
        normalize_ranges(&ranges)
    }

    fn character_pair(&mut self, owner: ModelNodeId, start: i32, stop: i32) -> StatePair {
        let (ranges, case_expanded) = self.case_fold_range(start, stop);
        let ranges = normalize_ranges(&ranges);
        let pair = self.basic_pair(owner);
        let kind = if case_expanded {
            BuildTransitionKind::Set(ranges)
        } else {
            collapsed_set_kind(ranges)
        };
        self.authored_transition(pair.left, pair.right, kind, owner);
        pair
    }

    fn case_fold_ranges(&self, ranges: &[(i32, i32)]) -> Vec<(i32, i32)> {
        if !self.current_case_insensitive() {
            return ranges.to_vec();
        }
        let mut result = Vec::new();
        for &(start, stop) in ranges {
            result.extend(self.case_fold_range(start, stop).0);
        }
        result
    }

    fn case_fold_range(&self, start: i32, stop: i32) -> (Vec<(i32, i32)>, bool) {
        if !self.current_case_insensitive() || start < 0 || stop < 0 {
            return (vec![(start, stop)], false);
        }
        let lower_start = simple_lowercase(start);
        let upper_start = simple_uppercase(start);
        let lower_stop = simple_lowercase(stop);
        let upper_stop = simple_uppercase(stop);
        let mixed = (lower_start == start) != (lower_stop == stop);
        if (lower_start == upper_start && lower_stop == upper_stop)
            || mixed
            || lower_stop - lower_start != upper_stop - upper_start
        {
            (vec![(start, stop)], false)
        } else {
            (
                vec![(lower_start, lower_stop), (upper_start, upper_stop)],
                true,
            )
        }
    }

    fn current_case_insensitive(&self) -> bool {
        let (rule, _) = self.current_rule.expect("building a lexer rule");
        rule.case_insensitive.unwrap_or_else(|| {
            self.grammar
                .unit
                .options
                .iter()
                .find(|option| option.name.value == "caseInsensitive")
                .is_some_and(|option| option.value.value == "true")
        })
    }

    fn lexer_action_index(&mut self, action: LexerAction) -> usize {
        if let Some(index) = self
            .lexer_actions
            .iter()
            .position(|candidate| candidate == &action)
        {
            index
        } else {
            let index = self.lexer_actions.len();
            self.lexer_actions.push(action);
            index
        }
    }

    fn quantified_atom(
        &mut self,
        base: StatePair,
        quantifier: Quantifier,
        owner: ModelNodeId,
    ) -> StatePair {
        let kind = match quantifier {
            Quantifier::Optional { .. } => AtnStateKind::BlockStart,
            Quantifier::ZeroOrMore { .. } => AtnStateKind::StarBlockStart,
            Quantifier::OneOrMore { .. } => AtnStateKind::PlusBlockStart,
            Quantifier::One => return base,
        };
        let start = self.synthetic_state(kind, SyntheticReason::BlockBoundary, owner);
        if matches!(quantifier, Quantifier::Optional { .. }) {
            self.graph.add_decision(start);
        }
        let pair = self.make_block(start, &[base], owner);
        match quantifier {
            Quantifier::Optional { greedy } => self.optional(pair, greedy, owner),
            Quantifier::ZeroOrMore { greedy } => self.star(pair, greedy, owner),
            Quantifier::OneOrMore { greedy } => self.plus(pair, greedy, owner),
            Quantifier::One => unreachable!("handled above"),
        }
    }

    fn make_block(
        &mut self,
        start: BuildStateId,
        alternatives: &[StatePair],
        owner: ModelNodeId,
    ) -> StatePair {
        let end = self.synthetic_state(
            AtnStateKind::BlockEnd,
            SyntheticReason::BlockBoundary,
            owner,
        );
        self.graph.state_mut(start).end_state = Some(end);
        for alternative in alternatives {
            self.synthetic_epsilon(
                start,
                alternative.left,
                SyntheticReason::BlockBoundary,
                owner,
                false,
            );
            self.synthetic_epsilon(
                alternative.right,
                end,
                SyntheticReason::BlockBoundary,
                owner,
                false,
            );
            remove_tail_epsilons(&mut self.graph, alternative.left);
        }
        StatePair {
            left: start,
            right: end,
        }
    }

    fn optional(&mut self, pair: StatePair, greedy: bool, owner: ModelNodeId) -> StatePair {
        self.graph.state_mut(pair.left).non_greedy = !greedy;
        self.synthetic_epsilon(
            pair.left,
            pair.right,
            SyntheticReason::LoopBoundary,
            owner,
            !greedy,
        );
        pair
    }

    fn plus(&mut self, pair: StatePair, greedy: bool, owner: ModelNodeId) -> StatePair {
        let (rule, _) = self.current_rule.expect("building a lexer rule");
        self.epsilon_closures.push((rule.id, pair.left, pair.right));
        let loop_state = self.synthetic_state(
            AtnStateKind::PlusLoopBack,
            SyntheticReason::LoopBoundary,
            owner,
        );
        self.graph.state_mut(loop_state).non_greedy = !greedy;
        self.graph.add_decision(loop_state);
        let end = self.synthetic_state(AtnStateKind::LoopEnd, SyntheticReason::LoopBoundary, owner);
        self.graph.state_mut(pair.left).loop_back_state = Some(loop_state);
        self.graph.state_mut(end).loop_back_state = Some(loop_state);
        self.synthetic_epsilon(
            pair.right,
            loop_state,
            SyntheticReason::LoopBoundary,
            owner,
            false,
        );
        for target in if greedy {
            [pair.left, end]
        } else {
            [end, pair.left]
        } {
            self.synthetic_epsilon(
                loop_state,
                target,
                SyntheticReason::LoopBoundary,
                owner,
                false,
            );
        }
        StatePair {
            left: pair.left,
            right: end,
        }
    }

    fn star(&mut self, pair: StatePair, greedy: bool, owner: ModelNodeId) -> StatePair {
        let (rule, _) = self.current_rule.expect("building a lexer rule");
        self.epsilon_closures.push((rule.id, pair.left, pair.right));
        let entry = self.synthetic_state(
            AtnStateKind::StarLoopEntry,
            SyntheticReason::LoopBoundary,
            owner,
        );
        self.graph.state_mut(entry).non_greedy = !greedy;
        self.graph.add_decision(entry);
        let end = self.synthetic_state(AtnStateKind::LoopEnd, SyntheticReason::LoopBoundary, owner);
        let loop_state = self.synthetic_state(
            AtnStateKind::StarLoopBack,
            SyntheticReason::LoopBoundary,
            owner,
        );
        self.graph.state_mut(end).loop_back_state = Some(loop_state);
        for target in if greedy {
            [pair.left, end]
        } else {
            [end, pair.left]
        } {
            self.synthetic_epsilon(entry, target, SyntheticReason::LoopBoundary, owner, false);
        }
        self.synthetic_epsilon(
            pair.right,
            loop_state,
            SyntheticReason::LoopBoundary,
            owner,
            false,
        );
        self.synthetic_epsilon(
            loop_state,
            entry,
            SyntheticReason::LoopBoundary,
            owner,
            false,
        );
        StatePair {
            left: entry,
            right: end,
        }
    }

    fn element_list(&mut self, elements: &[StatePair], owner: ModelNodeId) -> StatePair {
        for pair in elements.windows(2) {
            let element = pair[0];
            let next = pair[1];
            let state = self.graph.state(element.left);
            let transition = (state.kind == AtnStateKind::Basic
                && self.graph.state(element.right).kind == AtnStateKind::Basic
                && state.transitions.len() == 1)
                .then(|| state.transitions[0]);
            let can_inline = transition.is_some_and(|transition| {
                let transition = self.graph.transition(transition);
                match &transition.kind {
                    BuildTransitionKind::Rule { follow, .. } => *follow == element.right,
                    _ => transition.target == element.right,
                }
            });
            if can_inline {
                let transition = transition.expect("checked above");
                match &mut self.graph.transition_mut(transition).kind {
                    BuildTransitionKind::Rule { follow, .. } => *follow = next.left,
                    _ => self.graph.transition_mut(transition).target = next.left,
                }
                self.graph.remove_state(element.right);
            } else {
                self.synthetic_epsilon(
                    element.right,
                    next.left,
                    SyntheticReason::BlockBoundary,
                    owner,
                    false,
                );
            }
        }
        StatePair {
            left: elements[0].left,
            right: elements[elements.len() - 1].right,
        }
    }

    const fn current_rule_index(&self) -> usize {
        self.current_rule.expect("building a lexer rule").1
    }

    fn basic_pair(&mut self, owner: ModelNodeId) -> StatePair {
        StatePair {
            left: self.authored_state(AtnStateKind::Basic, owner),
            right: self.authored_state(AtnStateKind::Basic, owner),
        }
    }

    fn epsilon_pair(&mut self, owner: ModelNodeId) -> StatePair {
        let pair = self.basic_pair(owner);
        self.authored_transition(pair.left, pair.right, BuildTransitionKind::Epsilon, owner);
        pair
    }

    fn atom_pair(&mut self, owner: ModelNodeId, label: i32) -> StatePair {
        let pair = self.basic_pair(owner);
        self.authored_transition(
            pair.left,
            pair.right,
            BuildTransitionKind::Atom(label),
            owner,
        );
        pair
    }

    fn authored_state(&mut self, kind: AtnStateKind, owner: ModelNodeId) -> BuildStateId {
        let origins = self.provenance.origins(owner).to_vec();
        self.graph.add_state(
            kind,
            self.current_rule.map(|(rule, index)| (rule.id, index)),
            origins,
            self.provenance,
        )
    }

    fn synthetic_state(
        &mut self,
        kind: AtnStateKind,
        reason: SyntheticReason,
        owner: ModelNodeId,
    ) -> BuildStateId {
        self.graph.add_synthetic_state(
            kind,
            self.current_rule.map(|(rule, index)| (rule.id, index)),
            reason,
            owner,
            self.provenance,
        )
    }

    fn authored_transition(
        &mut self,
        source: BuildStateId,
        target: BuildStateId,
        kind: BuildTransitionKind,
        owner: ModelNodeId,
    ) {
        let origins = self.provenance.origins(owner).to_vec();
        self.graph.add_transition(
            BuildTransitionSpec {
                source,
                target,
                kind,
                prepend: false,
            },
            origins,
            self.provenance,
        );
    }

    fn synthetic_epsilon(
        &mut self,
        source: BuildStateId,
        target: BuildStateId,
        reason: SyntheticReason,
        owner: ModelNodeId,
        prepend: bool,
    ) {
        self.graph.add_synthetic_transition(
            BuildTransitionSpec {
                source,
                target,
                kind: BuildTransitionKind::Epsilon,
                prepend,
            },
            reason,
            owner,
            self.provenance,
        );
    }
}

fn command_action(command: ResolvedLexerCommand) -> LexerAction {
    match command {
        ResolvedLexerCommand::Skip => LexerAction::Skip,
        ResolvedLexerCommand::More => LexerAction::More,
        ResolvedLexerCommand::PopMode => LexerAction::PopMode,
        ResolvedLexerCommand::Mode(mode) => {
            LexerAction::Mode(i32::try_from(mode).expect("mode index exceeds i32"))
        }
        ResolvedLexerCommand::PushMode(mode) => {
            LexerAction::PushMode(i32::try_from(mode).expect("mode index exceeds i32"))
        }
        ResolvedLexerCommand::Type(token_type) => LexerAction::Type(token_type),
        ResolvedLexerCommand::Channel(channel) => LexerAction::Channel(channel),
    }
}

fn collapsed_set_kind(ranges: Vec<(i32, i32)>) -> BuildTransitionKind {
    match ranges.as_slice() {
        [(start, stop)] if start == stop => BuildTransitionKind::Atom(*start),
        [(start, stop)] => BuildTransitionKind::Range {
            start: *start,
            stop: *stop,
        },
        _ => BuildTransitionKind::Set(ranges),
    }
}

fn lower(
    grammar: &SemanticGrammar,
    graph: &FinalizedAtnGraph,
    mode_starts: &[BuildStateId],
    lexer_actions: Vec<LexerAction>,
) -> LexerAtn {
    let mut atn = LexerAtn::new(graph.max_token_type);
    for (state_number, state) in graph.states.iter().enumerate() {
        let mut runtime = LexerAtnState::new(state_number, state.kind);
        if let Some(rule_index) = state.rule_index {
            runtime = runtime.with_rule_index(rule_index);
        }
        if matches!(
            state.kind,
            AtnStateKind::BlockStart | AtnStateKind::PlusBlockStart | AtnStateKind::StarBlockStart
        ) {
            runtime.end_state = state.end_state;
        }
        if state.kind == AtnStateKind::LoopEnd {
            runtime.loop_back_state = state.loop_back_state;
        }
        runtime.non_greedy = state.non_greedy;
        runtime.left_recursive_rule = state.left_recursive_rule;
        atn.add_state(runtime);
    }
    atn.set_rule_to_start_state(graph.rule_starts.clone());
    atn.set_rule_to_stop_state(graph.rule_stops.clone());
    atn.set_rule_to_token_type(
        grammar
            .unit
            .rules
            .iter()
            .map(|rule| {
                grammar
                    .recognizer
                    .vocabulary
                    .by_name
                    .get(&rule.name)
                    .copied()
                    .unwrap_or(0)
            })
            .collect(),
    );
    for mode in mode_starts {
        atn.add_mode_start_state(graph.state_map[mode]);
    }
    for decision in &graph.decisions {
        atn.add_decision_state(*decision);
    }
    atn.set_lexer_actions(lexer_actions);

    let transitions = graph
        .transitions
        .iter()
        .map(|transition| (transition.original, transition))
        .collect::<BTreeMap<_, _>>();
    for (source, state) in graph.states.iter().enumerate() {
        for transition_id in &state.transitions {
            let Some(transition) = transitions.get(transition_id).copied() else {
                continue;
            };
            let transition = lower_transition(transition);
            atn.state_mut(source)
                .expect("direct state exists")
                .add_transition(transition);
        }
    }
    add_rule_return_edges(&mut atn);
    atn
}

fn lower_transition(transition: &FinalizedTransition) -> LexerTransition {
    match &transition.kind {
        FinalizedTransitionKind::Epsilon => LexerTransition::Epsilon {
            target: transition.target,
        },
        FinalizedTransitionKind::Atom(label) => LexerTransition::Atom {
            target: transition.target,
            label: *label,
        },
        FinalizedTransitionKind::Range { start, stop } => LexerTransition::Range {
            target: transition.target,
            start: *start,
            stop: *stop,
        },
        FinalizedTransitionKind::Set(ranges) => LexerTransition::Set {
            target: transition.target,
            set: runtime_set(ranges),
        },
        FinalizedTransitionKind::NotSet(ranges) => LexerTransition::NotSet {
            target: transition.target,
            set: runtime_set(ranges),
        },
        FinalizedTransitionKind::Wildcard => LexerTransition::Wildcard {
            target: transition.target,
        },
        FinalizedTransitionKind::Rule {
            rule_index,
            follow,
            precedence,
            ..
        } => LexerTransition::Rule {
            target: transition.target,
            rule_index: *rule_index,
            follow_state: *follow,
            precedence: *precedence,
        },
        FinalizedTransitionKind::Predicate {
            rule_index,
            predicate_index,
            context_dependent,
        } => LexerTransition::Predicate {
            target: transition.target,
            rule_index: *rule_index,
            pred_index: *predicate_index,
            context_dependent: *context_dependent,
        },
        FinalizedTransitionKind::Action {
            rule_index,
            action_index,
            context_dependent,
        } => LexerTransition::Action {
            target: transition.target,
            rule_index: *rule_index,
            action_index: *action_index,
            context_dependent: *context_dependent,
        },
        FinalizedTransitionKind::Precedence(precedence) => LexerTransition::Precedence {
            target: transition.target,
            precedence: *precedence,
        },
    }
}

fn runtime_set(ranges: &[(i32, i32)]) -> IntervalSet {
    let mut set = IntervalSet::new();
    for &(start, stop) in ranges {
        set.add_range(start, stop);
    }
    set
}

fn add_rule_return_edges(atn: &mut LexerAtn) {
    let mut return_edges = Vec::new();
    for state in atn.states() {
        for transition in &state.transitions {
            let LexerTransition::Rule {
                target,
                follow_state,
                ..
            } = transition
            else {
                continue;
            };
            let rule_index = atn
                .state(*target)
                .and_then(|state| state.rule_index)
                .expect("rule transition targets a rule start");
            return_edges.push((atn.rule_to_stop_state()[rule_index], *follow_state));
        }
    }
    for (stop, follow) in return_edges {
        atn.state_mut(stop)
            .expect("rule stop state exists")
            .add_transition(LexerTransition::Epsilon { target: follow });
    }
}

fn analyze_lexer(
    grammar: &SemanticGrammar,
    graph: &FinalizedAtnGraph,
    closure_sites: Vec<(RuleId, BuildStateId, BuildStateId)>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<LexerAnalysis, CompilationError> {
    let nullable_indices = nullable_rule_indices(graph);
    let recursive_components = recursive_rule_components(grammar);
    let transitions = transitions_by_id(graph);
    for (rule, start, stop) in closure_sites {
        let (Some(&start), Some(&stop)) = (graph.state_map.get(&start), graph.state_map.get(&stop))
        else {
            continue;
        };
        if epsilon_reaches(graph, &transitions, start, stop, &nullable_indices) {
            let rule = grammar
                .unit
                .rules
                .iter()
                .find(|candidate| candidate.id == rule)
                .expect("closure rule belongs to grammar");
            diagnostics.push(Diagnostic::error(
                "G4A001",
                rule.span.clone(),
                format!(
                    "rule {} contains a closure whose body can match an empty string",
                    rule.name
                ),
            ));
        }
    }
    if has_errors(diagnostics) {
        return Err(CompilationError::new(std::mem::take(diagnostics)));
    }

    for (index, rule) in grammar.unit.rules.iter().enumerate() {
        if !rule.fragment && nullable_indices.contains(&index) {
            diagnostics.push(Diagnostic::warning(
                "G4A006",
                rule.span.clone(),
                format!(
                    "non-fragment lexer rule {} can match the empty string",
                    rule.name
                ),
            ));
        }
    }
    let nullable_rules = nullable_indices
        .iter()
        .map(|index| grammar.unit.rules[*index].id)
        .collect();
    Ok(LexerAnalysis {
        nullable_rules,
        recursive_components,
        diagnostics: diagnostics.clone(),
    })
}

fn recursive_rule_components(grammar: &SemanticGrammar) -> Vec<Vec<RuleId>> {
    let mut graph = DiGraph::<RuleId, ()>::new();
    let nodes = grammar
        .unit
        .rules
        .iter()
        .map(|rule| (rule.id, graph.add_node(rule.id)))
        .collect::<BTreeMap<_, _>>();
    for (source, targets) in &grammar.call_graph {
        for target in targets {
            graph.add_edge(nodes[source], nodes[target], ());
        }
    }
    let mut components = tarjan_scc(&graph)
        .into_iter()
        .filter_map(|component| {
            let cyclic = component.len() > 1
                || component
                    .first()
                    .is_some_and(|node| graph.find_edge(*node, *node).is_some());
            cyclic.then(|| {
                let mut rules = component
                    .into_iter()
                    .map(|node| graph[node])
                    .collect::<Vec<_>>();
                rules.sort_unstable();
                rules
            })
        })
        .collect::<Vec<_>>();
    components.sort();
    components
}

fn nullable_rule_indices(graph: &FinalizedAtnGraph) -> BTreeSet<usize> {
    let transitions = transitions_by_id(graph);
    let mut nullable = BTreeSet::new();
    loop {
        let previous = nullable.len();
        for (rule, (&start, &stop)) in graph.rule_starts.iter().zip(&graph.rule_stops).enumerate() {
            if epsilon_reaches(graph, &transitions, start, stop, &nullable) {
                nullable.insert(rule);
            }
        }
        if nullable.len() == previous {
            return nullable;
        }
    }
}

fn epsilon_reaches(
    graph: &FinalizedAtnGraph,
    transitions: &BTreeMap<super::super::model::BuildTransitionId, &FinalizedTransition>,
    start: usize,
    stop: usize,
    nullable_rules: &BTreeSet<usize>,
) -> bool {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(state) = pending.pop() {
        if state == stop {
            return true;
        }
        if !visited.insert(state) {
            continue;
        }
        for transition in graph.states[state]
            .transitions
            .iter()
            .filter_map(|transition| transitions.get(transition).copied())
        {
            match &transition.kind {
                FinalizedTransitionKind::Rule {
                    rule_index, follow, ..
                } if nullable_rules.contains(rule_index) => pending.push(*follow),
                kind if transition_is_epsilon(kind) => pending.push(transition.target),
                _ => {}
            }
        }
    }
    false
}

const fn transition_is_epsilon(kind: &FinalizedTransitionKind) -> bool {
    matches!(
        kind,
        FinalizedTransitionKind::Epsilon
            | FinalizedTransitionKind::Predicate { .. }
            | FinalizedTransitionKind::Action { .. }
            | FinalizedTransitionKind::Precedence(_)
    )
}

fn transitions_by_id(
    graph: &FinalizedAtnGraph,
) -> BTreeMap<super::super::model::BuildTransitionId, &FinalizedTransition> {
    graph
        .transitions
        .iter()
        .map(|transition| (transition.original, transition))
        .collect()
}

fn decode_string_literal(literal: &str) -> Result<Vec<i32>, String> {
    let body = literal
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .ok_or_else(|| format!("invalid lexer string literal {literal}"))?;
    let mut values = Vec::new();
    let mut cursor = 0;
    while cursor < body.len() {
        let character = body[cursor..]
            .chars()
            .next()
            .expect("cursor is on a character boundary");
        if character == '\\' {
            let (value, consumed) = parse_code_point_escape(body, cursor, false)?;
            values.push(value);
            cursor += consumed;
        } else {
            values.push(character as i32);
            cursor += character.len_utf8();
        }
    }
    Ok(values)
}

fn decode_character_literal(literal: &str) -> Result<i32, String> {
    let values = decode_string_literal(literal)?;
    match values.as_slice() {
        [value] => Ok(*value),
        _ => Err(format!(
            "lexer character literal {literal} must contain exactly one Unicode scalar"
        )),
    }
}

fn parse_code_point_escape(text: &str, start: usize, in_set: bool) -> Result<(i32, usize), String> {
    let tail = text
        .get(start..)
        .ok_or_else(|| "escape starts outside source text".to_owned())?;
    let mut characters = tail.char_indices();
    let (_, slash) = characters
        .next()
        .ok_or_else(|| "unterminated escape sequence".to_owned())?;
    if slash != '\\' {
        return Err("escape sequence does not start with a backslash".to_owned());
    }
    let (escaped_offset, escaped) = characters
        .next()
        .ok_or_else(|| "unterminated escape sequence".to_owned())?;
    let simple = match escaped {
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'b' => Some('\u{0008}'),
        'f' => Some('\u{000c}'),
        '\\' => Some('\\'),
        '\'' => Some('\''),
        ']' | '-' if in_set => Some(escaped),
        _ => None,
    };
    if let Some(value) = simple {
        return Ok((value as i32, escaped_offset + escaped.len_utf8()));
    }
    if escaped != 'u' {
        return Err(format!("invalid escape sequence \\{escaped}"));
    }

    let digits_start = escaped_offset + escaped.len_utf8();
    let unicode = &tail[digits_start..];
    let (digits, consumed) = if let Some(rest) = unicode.strip_prefix('{') {
        let close = rest
            .find('}')
            .ok_or_else(|| "unterminated braced Unicode escape".to_owned())?;
        (&rest[..close], digits_start + 1 + close + 1)
    } else {
        if unicode.len() < 4 {
            return Err("Unicode escape must contain four hexadecimal digits".to_owned());
        }
        (&unicode[..4], digits_start + 4)
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid Unicode escape \\u{{{digits}}}"));
    }
    let value = u32::from_str_radix(digits, 16)
        .map_err(|_| format!("invalid Unicode escape \\u{{{digits}}}"))?;
    if value > MAX_CODE_POINT as u32 || char::from_u32(value).is_none() {
        return Err(format!("Unicode escape is not a scalar value: {value:#x}"));
    }
    Ok((
        i32::try_from(value).expect("Unicode scalar fits i32"),
        consumed,
    ))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ParsedCharSet {
    explicit: Vec<(i32, i32)>,
    properties: Vec<(i32, i32)>,
}

impl ParsedCharSet {
    const fn is_empty(&self) -> bool {
        self.explicit.is_empty() && self.properties.is_empty()
    }

    fn extend(&mut self, other: Self) {
        self.explicit.extend(other.explicit);
        self.properties.extend(other.properties);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CharSetAtom {
    CodePoint(i32),
    Property(Vec<(i32, i32)>),
}

fn parse_char_set(text: &str) -> Result<ParsedCharSet, String> {
    let body = text
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| format!("invalid lexer character set {text}"))?;
    let mut result = ParsedCharSet::default();
    let mut pending = None;
    let mut in_range = false;
    let mut cursor = 0;
    while cursor < body.len() {
        let character = body[cursor..]
            .chars()
            .next()
            .expect("cursor is on a character boundary");
        if character == '-'
            && cursor != 0
            && cursor + character.len_utf8() != body.len()
            && pending.is_some()
            && !in_range
        {
            if matches!(pending.as_ref(), Some(CharSetAtom::Property(_))) {
                return Err(format!(
                    "Unicode property is not allowed as a range boundary in [{body}]"
                ));
            }
            in_range = true;
            cursor += character.len_utf8();
            continue;
        }

        let (atom, consumed) = if character == '\\' {
            parse_char_set_escape(body, cursor)?
        } else {
            (
                CharSetAtom::CodePoint(character as i32),
                character.len_utf8(),
            )
        };
        cursor += consumed;

        match atom {
            CharSetAtom::CodePoint(stop) if in_range => {
                let Some(CharSetAtom::CodePoint(start)) = pending.take() else {
                    return Err(format!(
                        "Unicode property is not allowed as a range boundary in [{body}]"
                    ));
                };
                if start > stop {
                    return Err(format!("empty character range {start:#x}..{stop:#x}"));
                }
                result.explicit.push((start, stop));
                in_range = false;
            }
            _ if in_range => {
                return Err(format!(
                    "Unicode property is not allowed as a range boundary in [{body}]"
                ));
            }
            atom => {
                append_char_set_atom(&mut result, pending.take());
                pending = Some(atom);
            }
        }
    }
    append_char_set_atom(&mut result, pending);
    result.explicit = normalize_ranges(&result.explicit);
    result.properties = normalize_ranges(&result.properties);
    Ok(result)
}

fn parse_char_set_escape(text: &str, start: usize) -> Result<(CharSetAtom, usize), String> {
    let tail = text
        .get(start..)
        .ok_or_else(|| "escape starts outside source text".to_owned())?;
    let escaped = tail
        .strip_prefix('\\')
        .and_then(|value| value.chars().next())
        .ok_or_else(|| "unterminated escape sequence".to_owned())?;
    if matches!(escaped, 'p' | 'P') {
        parse_property_escape(tail, escaped == 'P')
    } else {
        let (value, consumed) = parse_code_point_escape(text, start, true)?;
        Ok((CharSetAtom::CodePoint(value), consumed))
    }
}

fn parse_property_escape(tail: &str, inverted: bool) -> Result<(CharSetAtom, usize), String> {
    let property = tail
        .get(3..)
        .and_then(|value| value.split_once('}'))
        .ok_or_else(|| "unterminated Unicode property escape".to_owned())?;
    if tail.as_bytes().get(2).is_none_or(|byte| *byte != b'{') {
        return Err("Unicode property escape must use braces".to_owned());
    }
    let (name, remainder) = property;
    let consumed = tail.len() - remainder.len();
    let raw_ranges = property_ranges(name)
        .filter(|ranges| !ranges.is_empty())
        .ok_or_else(|| format!("unknown or empty Unicode property {name}"))?;
    let ranges = raw_ranges
        .chunks_exact(2)
        .map(|range| (range[0], range[1]))
        .collect::<Vec<_>>();
    let ranges = if inverted {
        complement_ranges(&ranges)
    } else {
        ranges
    };
    Ok((CharSetAtom::Property(ranges), consumed))
}

fn append_char_set_atom(result: &mut ParsedCharSet, atom: Option<CharSetAtom>) {
    match atom {
        Some(CharSetAtom::CodePoint(value)) => result.explicit.push((value, value)),
        Some(CharSetAtom::Property(ranges)) => result.properties.extend(ranges),
        None => {}
    }
}

fn complement_ranges(ranges: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut complement = Vec::new();
    let mut next = 0;
    for &(start, stop) in ranges {
        if next < start {
            complement.push((next, start - 1));
        }
        next = stop.saturating_add(1);
    }
    if next <= MAX_CODE_POINT {
        complement.push((next, MAX_CODE_POINT));
    }
    complement
}

fn normalize_ranges(ranges: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut ranges = ranges.to_vec();
    ranges.sort_unstable();
    let mut normalized: Vec<(i32, i32)> = Vec::with_capacity(ranges.len());
    for (start, stop) in ranges {
        if let Some((_, previous_stop)) = normalized.last_mut()
            && start <= previous_stop.saturating_add(1)
        {
            *previous_stop = (*previous_stop).max(stop);
        } else {
            normalized.push((start, stop));
        }
    }
    normalized
}

fn encode_lexer_atn(atn: &LexerAtn) -> Vec<i32> {
    let sets = collect_runtime_sets(atn);
    let mut data = vec![
        SERIALIZED_VERSION,
        0,
        atn.max_token_type(),
        usize_to_i32(atn.states().len()),
    ];
    for state in atn.states() {
        data.push(state_type(state.kind));
        if state.kind == AtnStateKind::Invalid {
            continue;
        }
        data.push(state.rule_index.map_or(-1, usize_to_i32));
        match state.kind {
            AtnStateKind::LoopEnd => data.push(usize_to_i32(
                state
                    .loop_back_state
                    .expect("loop-end state has loop-back state"),
            )),
            AtnStateKind::BlockStart
            | AtnStateKind::PlusBlockStart
            | AtnStateKind::StarBlockStart => data.push(usize_to_i32(
                state.end_state.expect("block-start state has end state"),
            )),
            _ => {}
        }
    }
    append_state_list(
        &mut data,
        atn.states()
            .iter()
            .filter(|state| state.non_greedy)
            .map(|state| state.state_number),
    );
    append_state_list(
        &mut data,
        atn.states()
            .iter()
            .filter(|state| state.kind == AtnStateKind::RuleStart && state.left_recursive_rule)
            .map(|state| state.state_number),
    );

    data.push(usize_to_i32(atn.rule_to_start_state().len()));
    for (&start, &token_type) in atn
        .rule_to_start_state()
        .iter()
        .zip(atn.rule_to_token_type())
    {
        data.push(usize_to_i32(start));
        data.push(token_type);
    }
    data.push(usize_to_i32(atn.mode_to_start_state().len()));
    data.extend(atn.mode_to_start_state().iter().copied().map(usize_to_i32));
    serialize_sets(&mut data, &sets);
    serialize_runtime_edges(&mut data, atn, &sets);
    append_state_list(&mut data, atn.decision_to_state().iter().copied());
    serialize_lexer_actions(&mut data, atn.lexer_actions());
    data
}

fn collect_runtime_sets(atn: &LexerAtn) -> Vec<Vec<(i32, i32)>> {
    let mut sets = Vec::new();
    for state in atn.states() {
        if state.kind == AtnStateKind::RuleStop {
            continue;
        }
        for transition in &state.transitions {
            let ranges = match transition {
                LexerTransition::Set { set, .. } | LexerTransition::NotSet { set, .. } => {
                    set.ranges()
                }
                _ => continue,
            };
            let ranges = ranges.to_vec();
            if !sets.contains(&ranges) {
                sets.push(ranges);
            }
        }
    }
    sets
}

fn append_state_list(data: &mut Vec<i32>, values: impl IntoIterator<Item = usize>) {
    let values = values.into_iter().collect::<Vec<_>>();
    data.push(usize_to_i32(values.len()));
    data.extend(values.into_iter().map(usize_to_i32));
}

fn serialize_sets(data: &mut Vec<i32>, sets: &[Vec<(i32, i32)>]) {
    data.push(usize_to_i32(sets.len()));
    for set in sets {
        let eof = set
            .iter()
            .any(|(start, stop)| *start <= EOF_CODE_POINT && EOF_CODE_POINT <= *stop);
        let eof_singleton = set
            .first()
            .is_some_and(|range| *range == (EOF_CODE_POINT, EOF_CODE_POINT));
        data.push(usize_to_i32(set.len() - usize::from(eof_singleton)));
        data.push(i32::from(eof));
        for &(start, stop) in set {
            if (start, stop) == (EOF_CODE_POINT, EOF_CODE_POINT) {
                continue;
            }
            data.push(if start == EOF_CODE_POINT { 0 } else { start });
            data.push(stop);
        }
    }
}

fn serialize_runtime_edges(data: &mut Vec<i32>, atn: &LexerAtn, sets: &[Vec<(i32, i32)>]) {
    let count = atn
        .states()
        .iter()
        .filter(|state| state.kind != AtnStateKind::RuleStop)
        .map(|state| state.transitions.len())
        .sum();
    data.push(usize_to_i32(count));
    for state in atn.states() {
        if state.kind == AtnStateKind::RuleStop {
            continue;
        }
        for transition in &state.transitions {
            let (target, kind, arg1, arg2, arg3) = runtime_edge(transition, sets);
            data.extend([
                usize_to_i32(state.state_number),
                usize_to_i32(target),
                kind,
                arg1,
                arg2,
                arg3,
            ]);
        }
    }
}

fn runtime_edge(
    transition: &LexerTransition,
    sets: &[Vec<(i32, i32)>],
) -> (usize, i32, i32, i32, i32) {
    match transition {
        LexerTransition::Epsilon { target } => (*target, 1, 0, 0, 0),
        LexerTransition::Range {
            target,
            start,
            stop,
        } => (
            *target,
            2,
            if *start == EOF_CODE_POINT { 0 } else { *start },
            *stop,
            i32::from(*start == EOF_CODE_POINT),
        ),
        LexerTransition::Rule {
            target,
            rule_index,
            follow_state,
            precedence,
        } => (
            *follow_state,
            3,
            usize_to_i32(*target),
            usize_to_i32(*rule_index),
            *precedence,
        ),
        LexerTransition::Predicate {
            target,
            rule_index,
            pred_index,
            context_dependent,
        } => (
            *target,
            4,
            usize_to_i32(*rule_index),
            usize_to_i32(*pred_index),
            i32::from(*context_dependent),
        ),
        LexerTransition::Atom { target, label } => (
            *target,
            5,
            if *label == EOF_CODE_POINT { 0 } else { *label },
            0,
            i32::from(*label == EOF_CODE_POINT),
        ),
        LexerTransition::Action {
            target,
            rule_index,
            action_index,
            context_dependent,
        } => (
            *target,
            6,
            usize_to_i32(*rule_index),
            action_index.map_or(-1, usize_to_i32),
            i32::from(*context_dependent),
        ),
        LexerTransition::Set { target, set } => (*target, 7, runtime_set_index(sets, set), 0, 0),
        LexerTransition::NotSet { target, set } => (*target, 8, runtime_set_index(sets, set), 0, 0),
        LexerTransition::Wildcard { target } => (*target, 9, 0, 0, 0),
        LexerTransition::Precedence { target, precedence } => (*target, 10, *precedence, 0, 0),
    }
}

fn runtime_set_index(sets: &[Vec<(i32, i32)>], set: &IntervalSet) -> i32 {
    usize_to_i32(
        sets.iter()
            .position(|candidate| candidate.as_slice() == set.ranges())
            .expect("transition set was collected"),
    )
}

fn serialize_lexer_actions(data: &mut Vec<i32>, actions: &[LexerAction]) {
    data.push(usize_to_i32(actions.len()));
    for action in actions {
        let (kind, first, second) = match action {
            LexerAction::Channel(channel) => (0, *channel, 0),
            LexerAction::Custom {
                rule_index,
                action_index,
            } => (1, *rule_index, *action_index),
            LexerAction::Mode(mode) => (2, *mode, 0),
            LexerAction::More => (3, 0, 0),
            LexerAction::PopMode => (4, 0, 0),
            LexerAction::PushMode(mode) => (5, *mode, 0),
            LexerAction::Skip => (6, 0, 0),
            LexerAction::Type(token_type) => (7, *token_type, 0),
        };
        data.extend(<[i32; 3]>::from((kind, first, second)));
    }
}

const fn state_type(kind: AtnStateKind) -> i32 {
    match kind {
        AtnStateKind::Invalid => 0,
        AtnStateKind::Basic => 1,
        AtnStateKind::RuleStart => 2,
        AtnStateKind::BlockStart => 3,
        AtnStateKind::PlusBlockStart => 4,
        AtnStateKind::StarBlockStart => 5,
        AtnStateKind::TokenStart => 6,
        AtnStateKind::RuleStop => 7,
        AtnStateKind::BlockEnd => 8,
        AtnStateKind::StarLoopBack => 9,
        AtnStateKind::StarLoopEntry => 10,
        AtnStateKind::PlusLoopBack => 11,
        AtnStateKind::LoopEnd => 12,
    }
}

fn usize_to_i32(value: usize) -> i32 {
    i32::try_from(value).expect("ATN value exceeds i32")
}
