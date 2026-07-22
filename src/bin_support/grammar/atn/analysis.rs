use std::collections::{BTreeMap, BTreeSet};

use antlr4_runtime::atn::AtnStateKind;
use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use super::super::diagnostic::{CompilationError, Diagnostic, Severity};
use super::super::model::{BuildStateId, RuleId, SemanticGrammar};
use super::build::{FinalizedAtnGraph, FinalizedTransition, FinalizedTransitionKind};

const EOF_TOKEN_TYPE: i32 = -1;
const MIN_USER_TOKEN_TYPE: i32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DecisionLookahead {
    pub(crate) state: usize,
    pub(crate) alternatives: Vec<Option<Vec<(i32, i32)>>>,
    pub(crate) disjoint: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ParserAnalysis {
    pub(crate) decision_lookahead: Vec<DecisionLookahead>,
    pub(crate) nullable_rules: BTreeSet<RuleId>,
    pub(crate) recursive_components: Vec<Vec<RuleId>>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ParserCheckSite {
    pub(super) rule: RuleId,
    pub(super) start: usize,
    pub(super) stop: usize,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ParserCheckSites {
    pub(super) closures: Vec<ParserCheckSite>,
    pub(super) optionals: Vec<ParserCheckSite>,
}

impl ParserCheckSites {
    pub(super) fn compact(
        graph: &FinalizedAtnGraph,
        closures: impl IntoIterator<Item = (RuleId, BuildStateId, BuildStateId)>,
        optionals: impl IntoIterator<Item = (RuleId, BuildStateId, BuildStateId)>,
    ) -> Self {
        Self {
            closures: compact_sites(graph, closures),
            optionals: compact_sites(graph, optionals),
        }
    }
}

fn compact_sites(
    graph: &FinalizedAtnGraph,
    sites: impl IntoIterator<Item = (RuleId, BuildStateId, BuildStateId)>,
) -> Vec<ParserCheckSite> {
    sites
        .into_iter()
        .filter_map(|(rule, start, stop)| {
            Some(ParserCheckSite {
                rule,
                start: *graph.state_map.get(&start)?,
                stop: *graph.state_map.get(&stop)?,
            })
        })
        .collect()
}

pub(crate) fn analyze_parser(
    grammar: &SemanticGrammar,
    graph: &FinalizedAtnGraph,
    check_sites: &ParserCheckSites,
) -> Result<ParserAnalysis, CompilationError> {
    let analyzer = LookAnalyzer::new(graph);
    let mut diagnostics = epsilon_diagnostics(grammar, check_sites, &analyzer);
    if has_errors(&diagnostics) {
        return Err(CompilationError::new(diagnostics));
    }

    let nullable_rule_indices = nullable_rule_indices(graph);
    let recursive_components = indirect_left_recursive_components(graph, &nullable_rule_indices);
    diagnostics.extend(recursion_diagnostics(grammar, &recursive_components));
    if has_errors(&diagnostics) {
        return Err(CompilationError::new(diagnostics));
    }

    let nullable_rules = nullable_rule_indices
        .iter()
        .filter_map(|index| {
            graph
                .rule_starts
                .get(*index)
                .and_then(|state| graph.states[*state].rule)
        })
        .collect();
    let decision_lookahead = decision_lookahead(graph, &analyzer);
    Ok(ParserAnalysis {
        decision_lookahead,
        nullable_rules,
        recursive_components,
        diagnostics,
    })
}

fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
}

fn epsilon_diagnostics(
    grammar: &SemanticGrammar,
    check_sites: &ParserCheckSites,
    analyzer: &LookAnalyzer<'_>,
) -> Vec<Diagnostic> {
    let rules = grammar
        .unit
        .rules
        .iter()
        .map(|rule| (rule.id, rule))
        .collect::<BTreeMap<_, _>>();
    let mut diagnostics = Vec::new();
    for site in &check_sites.closures {
        let look = analyzer.look(site.start, Some(site.stop), true, true);
        let rule = rules[&site.rule];
        if look.epsilon {
            let (code, message) = if rule.left_recursion.is_some() {
                (
                    "G4A002",
                    format!(
                        "left recursive rule {} contains a left recursive alternative which can be followed by the empty string",
                        rule.name
                    ),
                )
            } else {
                (
                    "G4A001",
                    format!(
                        "rule {} contains a closure whose body can match an empty string",
                        rule.name
                    ),
                )
            };
            diagnostics.push(Diagnostic::error(code, rule.span.clone(), message));
        }
        if look.contains(EOF_TOKEN_TYPE) {
            diagnostics.push(Diagnostic::error(
                "G4A003",
                rule.span.clone(),
                format!(
                    "rule {} contains a closure whose body can match EOF",
                    rule.name
                ),
            ));
        }
    }

    for site in &check_sites.optionals {
        let Some(state) = analyzer.graph.states.get(site.start) else {
            continue;
        };
        let mut bypasses = 0;
        let mut warned = false;
        for transition in analyzer.transitions_from(state) {
            if transition.target == site.stop {
                bypasses += 1;
                continue;
            }
            let look = analyzer.look(transition.target, Some(site.stop), true, true);
            if look.epsilon {
                let rule = rules[&site.rule];
                diagnostics.push(Diagnostic::warning(
                    "G4A004",
                    rule.span.clone(),
                    format!(
                        "rule {} contains an optional block with an alternative that can match an empty string",
                        rule.name
                    ),
                ));
                warned = true;
                break;
            }
        }
        if !warned {
            debug_assert_eq!(
                bypasses,
                1,
                "optional block must have one bypass: site={site:?}, targets={:?}",
                analyzer
                    .transitions_from(state)
                    .map(|transition| transition.target)
                    .collect::<Vec<_>>()
            );
        }
    }
    diagnostics
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
                kind if kind.is_epsilon() => pending.push(transition.target),
                _ => {}
            }
        }
    }
    false
}

fn indirect_left_recursive_components(
    graph: &FinalizedAtnGraph,
    nullable_rules: &BTreeSet<usize>,
) -> Vec<Vec<RuleId>> {
    let transitions = transitions_by_id(graph);
    let mut left_corners = vec![BTreeSet::new(); graph.rule_starts.len()];
    for (rule, start) in graph.rule_starts.iter().copied().enumerate() {
        collect_left_corners(
            graph,
            &transitions,
            start,
            nullable_rules,
            &mut left_corners[rule],
        );
    }

    let mut petgraph = DiGraph::<RuleId, ()>::new();
    let nodes = graph
        .rule_starts
        .iter()
        .map(|state| {
            let rule = graph.states[*state]
                .rule
                .expect("rule-start state has a rule");
            (rule, petgraph.add_node(rule))
        })
        .collect::<Vec<_>>();
    for (source, targets) in left_corners.iter().enumerate() {
        for target in targets {
            petgraph.add_edge(nodes[source].1, nodes[*target].1, ());
        }
    }

    let mut components = tarjan_scc(&petgraph)
        .into_iter()
        .filter_map(|component| {
            let cyclic = component.len() > 1
                || component
                    .first()
                    .is_some_and(|node| petgraph.find_edge(*node, *node).is_some());
            cyclic.then(|| {
                let mut rules = component
                    .into_iter()
                    .map(|node| petgraph[node])
                    .collect::<Vec<_>>();
                let first = rules
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, rule)| **rule)
                    .map_or(0, |(index, _)| index);
                rules.rotate_left(first);
                rules
            })
        })
        .collect::<Vec<_>>();
    components.sort();
    components
}

fn collect_left_corners(
    graph: &FinalizedAtnGraph,
    transitions: &BTreeMap<super::super::model::BuildTransitionId, &FinalizedTransition>,
    start: usize,
    nullable_rules: &BTreeSet<usize>,
    result: &mut BTreeSet<usize>,
) {
    let mut pending = vec![start];
    let mut visited = BTreeSet::new();
    while let Some(state) = pending.pop() {
        if !visited.insert(state) || graph.states[state].kind == AtnStateKind::RuleStop {
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
                } => {
                    result.insert(*rule_index);
                    if nullable_rules.contains(rule_index) {
                        pending.push(*follow);
                    }
                }
                kind if kind.is_epsilon() => pending.push(transition.target),
                _ => {}
            }
        }
    }
}

fn recursion_diagnostics(grammar: &SemanticGrammar, components: &[Vec<RuleId>]) -> Vec<Diagnostic> {
    let rules = grammar
        .unit
        .rules
        .iter()
        .map(|rule| (rule.id, rule))
        .collect::<BTreeMap<_, _>>();
    components
        .iter()
        .filter_map(|component| {
            let first = rules.get(component.first()?)?;
            let names = component
                .iter()
                .filter_map(|rule| rules.get(rule).map(|rule| rule.name.as_str()))
                .collect::<Vec<_>>()
                .join(", ");
            let mut diagnostic = Diagnostic::error(
                "G4A005",
                first.span.clone(),
                format!("mutually left-recursive rules: [{names}]"),
            );
            for rule in component.iter().skip(1).filter_map(|rule| rules.get(rule)) {
                diagnostic =
                    diagnostic.with_related(rule.span.clone(), "cycle member is declared here");
            }
            Some(diagnostic)
        })
        .collect()
}

fn decision_lookahead(
    graph: &FinalizedAtnGraph,
    analyzer: &LookAnalyzer<'_>,
) -> Vec<DecisionLookahead> {
    graph
        .decisions
        .iter()
        .copied()
        .map(|state_number| {
            let state = &graph.states[state_number];
            let transitions = analyzer.transitions_from(state).collect::<Vec<_>>();
            let alternatives = if state.non_greedy {
                vec![None; transitions.len() + 1]
            } else {
                transitions
                    .iter()
                    .map(|transition| {
                        let look = analyzer.look(transition.target, None, false, false);
                        (!look.hit_predicate && !look.ranges.is_empty()).then_some(look.ranges)
                    })
                    .collect()
            };
            let disjoint = alternatives_are_disjoint(&alternatives);
            DecisionLookahead {
                state: state_number,
                alternatives,
                disjoint,
            }
        })
        .collect()
}

fn alternatives_are_disjoint(alternatives: &[Option<Vec<(i32, i32)>>]) -> bool {
    let mut combined = Vec::new();
    for alternative in alternatives {
        let Some(alternative) = alternative else {
            return false;
        };
        if alternative
            .iter()
            .any(|range| range_overlaps_any(*range, &combined))
        {
            return false;
        }
        combined = normalize_ranges(
            &combined
                .into_iter()
                .chain(alternative.iter().copied())
                .collect::<Vec<_>>(),
        );
    }
    true
}

fn range_overlaps_any(range: (i32, i32), ranges: &[(i32, i32)]) -> bool {
    ranges
        .iter()
        .any(|other| range.0 <= other.1 && other.0 <= range.1)
}

struct LookAnalyzer<'a> {
    graph: &'a FinalizedAtnGraph,
    transitions: BTreeMap<super::super::model::BuildTransitionId, &'a FinalizedTransition>,
}

impl<'a> LookAnalyzer<'a> {
    fn new(graph: &'a FinalizedAtnGraph) -> Self {
        Self {
            graph,
            transitions: transitions_by_id(graph),
        }
    }

    fn transitions_from(
        &self,
        state: &'a super::build::FinalizedState,
    ) -> impl Iterator<Item = &'a FinalizedTransition> + '_ {
        state
            .transitions
            .iter()
            .filter_map(|transition| self.transitions.get(transition).copied())
    }

    fn look(
        &self,
        start: usize,
        stop: Option<usize>,
        context_none: bool,
        see_through_predicates: bool,
    ) -> LookResult {
        let mut result = LookResult::default();
        let mut pending = vec![LookWork {
            state: start,
            returns: Vec::new(),
            called_rules: BTreeSet::new(),
        }];
        let mut visited = BTreeSet::new();
        while let Some(work) = pending.pop() {
            let key = work.key();
            if !visited.insert(key) {
                continue;
            }
            if stop == Some(work.state) && work.returns.is_empty() && context_none {
                result.epsilon = true;
                continue;
            }

            let state = &self.graph.states[work.state];
            if state.kind == AtnStateKind::RuleStop {
                if let Some(frame) = work.returns.last() {
                    let mut next = work.clone();
                    next.state = frame.follow;
                    next.called_rules.remove(&frame.rule);
                    next.returns.pop();
                    pending.push(next);
                } else if context_none {
                    result.epsilon = true;
                }
                continue;
            }
            for transition in self.transitions_from(state) {
                self.follow_transition(
                    transition,
                    &work,
                    see_through_predicates,
                    &mut pending,
                    &mut result,
                );
            }
        }
        result.ranges = normalize_ranges(&result.ranges);
        result
    }

    fn follow_transition(
        &self,
        transition: &FinalizedTransition,
        work: &LookWork,
        see_through_predicates: bool,
        pending: &mut Vec<LookWork>,
        result: &mut LookResult,
    ) {
        match &transition.kind {
            FinalizedTransitionKind::Rule {
                rule_index, follow, ..
            } => {
                if work.called_rules.contains(rule_index) {
                    return;
                }
                let mut next = work.clone();
                next.state = transition.target;
                next.called_rules.insert(*rule_index);
                next.returns.push(ReturnFrame {
                    follow: *follow,
                    rule: *rule_index,
                });
                pending.push(next);
            }
            FinalizedTransitionKind::Predicate { .. } | FinalizedTransitionKind::Precedence(_) => {
                if see_through_predicates {
                    pending.push(work.at(transition.target));
                } else {
                    result.hit_predicate = true;
                }
            }
            FinalizedTransitionKind::Epsilon | FinalizedTransitionKind::Action { .. } => {
                pending.push(work.at(transition.target));
            }
            FinalizedTransitionKind::Atom(label) => {
                result.ranges.push((*label, *label));
            }
            FinalizedTransitionKind::Range { start, stop } => {
                result.ranges.push((*start, *stop));
            }
            FinalizedTransitionKind::Set(ranges) => {
                result.ranges.extend(ranges.iter().copied());
            }
            FinalizedTransitionKind::NotSet(ranges) => {
                result.ranges.extend(complement_ranges(
                    ranges,
                    MIN_USER_TOKEN_TYPE,
                    self.graph.max_token_type,
                ));
            }
            FinalizedTransitionKind::Wildcard => {
                result
                    .ranges
                    .push((MIN_USER_TOKEN_TYPE, self.graph.max_token_type));
            }
        }
    }
}

#[derive(Clone)]
struct LookWork {
    state: usize,
    returns: Vec<ReturnFrame>,
    called_rules: BTreeSet<usize>,
}

impl LookWork {
    fn at(&self, state: usize) -> Self {
        let mut next = self.clone();
        next.state = state;
        next
    }

    fn key(&self) -> (usize, Vec<(usize, usize)>, Vec<usize>) {
        (
            self.state,
            self.returns
                .iter()
                .map(|frame| (frame.follow, frame.rule))
                .collect(),
            self.called_rules.iter().copied().collect(),
        )
    }
}

#[derive(Clone, Copy)]
struct ReturnFrame {
    follow: usize,
    rule: usize,
}

#[derive(Default)]
struct LookResult {
    ranges: Vec<(i32, i32)>,
    epsilon: bool,
    hit_predicate: bool,
}

impl LookResult {
    fn contains(&self, symbol: i32) -> bool {
        self.ranges
            .iter()
            .any(|(start, stop)| *start <= symbol && symbol <= *stop)
    }
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

fn complement_ranges(ranges: &[(i32, i32)], minimum: i32, maximum: i32) -> Vec<(i32, i32)> {
    if maximum < minimum {
        return Vec::new();
    }
    let ranges = normalize_ranges(ranges);
    let mut complement = Vec::new();
    let mut cursor = minimum;
    for (start, stop) in ranges {
        if stop < minimum || start > maximum {
            continue;
        }
        let start = start.max(minimum);
        let stop = stop.min(maximum);
        if cursor < start {
            complement.push((cursor, start - 1));
        }
        cursor = stop.saturating_add(1);
        if cursor > maximum {
            return complement;
        }
    }
    if cursor <= maximum {
        complement.push((cursor, maximum));
    }
    complement
}

fn normalize_ranges(ranges: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut ranges = ranges.to_vec();
    ranges.sort_unstable();
    let mut normalized: Vec<(i32, i32)> = Vec::with_capacity(ranges.len());
    for (start, stop) in ranges {
        let (start, stop) = if start <= stop {
            (start, stop)
        } else {
            (stop, start)
        };
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

trait EpsilonTransition {
    fn is_epsilon(&self) -> bool;
}

impl EpsilonTransition for FinalizedTransitionKind {
    fn is_epsilon(&self) -> bool {
        matches!(
            self,
            Self::Epsilon | Self::Predicate { .. } | Self::Action { .. } | Self::Precedence(_)
        )
    }
}
