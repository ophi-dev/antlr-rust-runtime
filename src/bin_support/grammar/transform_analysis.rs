use std::collections::{BTreeMap, BTreeSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use super::model::{ElementKind, GrammarUnit, RuleId};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AnalysisInvalidation(u16);

impl AnalysisInvalidation {
    pub(crate) const NAMES: Self = Self(1 << 0);
    pub(crate) const CALLS: Self = Self(1 << 1);
    pub(crate) const NULLABILITY: Self = Self(1 << 2);
    pub(crate) const SIDE_EFFECTS: Self = Self(1 << 3);
    pub(crate) const VOCABULARY: Self = Self(1 << 4);
    pub(crate) const ALL: Self = Self(u16::MAX);

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TransformAnalysis {
    pub(crate) rules_by_name: BTreeMap<String, RuleId>,
    pub(crate) call_graph: BTreeMap<RuleId, Vec<RuleId>>,
    pub(crate) nullable: BTreeSet<RuleId>,
    pub(crate) recursive_components: Vec<Vec<RuleId>>,
    pub(crate) side_effecting: BTreeSet<RuleId>,
    valid: AnalysisInvalidation,
}

impl TransformAnalysis {
    pub(crate) fn compute(units: &[GrammarUnit]) -> Self {
        let mut analysis = Self::default();
        analysis.recompute(units);
        analysis
    }

    pub(crate) fn invalidate(&mut self, invalidation: AnalysisInvalidation) {
        if invalidation.intersects(AnalysisInvalidation::NAMES) {
            self.rules_by_name.clear();
        }
        if invalidation.intersects(AnalysisInvalidation::CALLS) {
            self.call_graph.clear();
            self.recursive_components.clear();
        }
        if invalidation.intersects(AnalysisInvalidation::NULLABILITY) {
            self.nullable.clear();
        }
        if invalidation.intersects(AnalysisInvalidation::SIDE_EFFECTS) {
            self.side_effecting.clear();
        }
        self.valid.0 &= !invalidation.0;
    }

    pub(crate) fn recompute(&mut self, units: &[GrammarUnit]) {
        self.rules_by_name.clear();
        for unit in units {
            for rule in &unit.rules {
                self.rules_by_name.insert(rule.name.clone(), rule.id);
            }
        }
        self.call_graph.clear();
        self.side_effecting.clear();
        for unit in units {
            for rule in &unit.rules {
                let mut calls = Vec::new();
                collect_calls(
                    &rule.block,
                    &self.rules_by_name,
                    &mut calls,
                    &mut self.side_effecting,
                    rule.id,
                );
                calls.sort_unstable();
                calls.dedup();
                self.call_graph.insert(rule.id, calls);
            }
        }
        self.nullable = compute_nullable(units, &self.rules_by_name);
        self.recursive_components = recursive_components(&self.call_graph);
        self.valid = AnalysisInvalidation::ALL;
    }
}

fn collect_calls(
    block: &super::model::Block,
    names: &BTreeMap<String, RuleId>,
    calls: &mut Vec<RuleId>,
    side_effecting: &mut BTreeSet<RuleId>,
    owner: RuleId,
) {
    for alternative in &block.alternatives {
        for element in &alternative.elements {
            match &element.kind {
                ElementKind::RuleCall(call) => {
                    if let Some(target) = names.get(&call.name) {
                        calls.push(*target);
                    }
                    if call.arguments.is_some() {
                        side_effecting.insert(owner);
                    }
                }
                ElementKind::Block(nested) => {
                    collect_calls(nested, names, calls, side_effecting, owner);
                }
                ElementKind::Action { .. } | ElementKind::Predicate { .. } => {
                    side_effecting.insert(owner);
                }
                ElementKind::Terminal(_)
                | ElementKind::Range(..)
                | ElementKind::Set { .. }
                | ElementKind::Epsilon => {}
            }
        }
    }
}

fn compute_nullable(units: &[GrammarUnit], names: &BTreeMap<String, RuleId>) -> BTreeSet<RuleId> {
    let rules = units
        .iter()
        .flat_map(|unit| unit.rules.iter())
        .map(|rule| (rule.id, rule))
        .collect::<BTreeMap<_, _>>();
    let mut nullable = BTreeSet::new();
    loop {
        let previous = nullable.len();
        for (id, rule) in &rules {
            if rule.block.alternatives.iter().any(|alternative| {
                alternative
                    .elements
                    .iter()
                    .all(|element| element_nullable(element, names, &nullable))
            }) {
                nullable.insert(*id);
            }
        }
        if nullable.len() == previous {
            return nullable;
        }
    }
}

fn element_nullable(
    element: &super::model::Element,
    names: &BTreeMap<String, RuleId>,
    nullable: &BTreeSet<RuleId>,
) -> bool {
    if matches!(
        element.quantifier,
        super::model::Quantifier::Optional { .. } | super::model::Quantifier::ZeroOrMore { .. }
    ) {
        return true;
    }
    match &element.kind {
        ElementKind::Epsilon | ElementKind::Action { .. } | ElementKind::Predicate { .. } => true,
        ElementKind::RuleCall(call) => names
            .get(&call.name)
            .is_some_and(|target| nullable.contains(target)),
        ElementKind::Block(block) => block.alternatives.iter().any(|alternative| {
            alternative
                .elements
                .iter()
                .all(|nested| element_nullable(nested, names, nullable))
        }),
        ElementKind::Terminal(_) | ElementKind::Range(..) | ElementKind::Set { .. } => false,
    }
}

fn recursive_components(call_graph: &BTreeMap<RuleId, Vec<RuleId>>) -> Vec<Vec<RuleId>> {
    let mut graph = DiGraph::<RuleId, ()>::new();
    let indices = call_graph
        .keys()
        .map(|rule| (*rule, graph.add_node(*rule)))
        .collect::<BTreeMap<_, _>>();
    for (source, targets) in call_graph {
        for target in targets {
            graph.add_edge(indices[source], indices[target], ());
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
                let mut members = component
                    .into_iter()
                    .map(|node| graph[node])
                    .collect::<Vec<_>>();
                members.sort_unstable();
                members
            })
        })
        .collect::<Vec<_>>();
    components.sort();
    components
}
