use std::collections::{BTreeMap, BTreeSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use super::model::{
    Block, ElementKind, GrammarKind, GrammarUnit, Rule, RuleId, RuleKind, SetElement, Terminal,
};
use super::mutual_recursion::collect_calls_into;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct EntryRuleConfig {
    names: BTreeSet<String>,
}

impl EntryRuleConfig {
    pub(crate) fn new(names: impl IntoIterator<Item = String>) -> Self {
        Self {
            names: names.into_iter().collect(),
        }
    }

    pub(crate) fn unknown_names(&self, units: &[GrammarUnit]) -> Vec<String> {
        let parser_rules = units
            .iter()
            .flat_map(|unit| &unit.rules)
            .filter(|rule| rule.kind == RuleKind::Parser)
            .map(|rule| rule.name.as_str())
            .collect::<BTreeSet<_>>();
        self.names
            .iter()
            .filter(|name| !parser_rules.contains(name.as_str()))
            .cloned()
            .collect()
    }

    pub(crate) fn matching_rule_ids(&self, units: &[GrammarUnit]) -> BTreeSet<RuleId> {
        units
            .iter()
            .flat_map(|unit| &unit.rules)
            .filter(|rule| rule.kind == RuleKind::Parser && self.names.contains(&rule.name))
            .map(|rule| rule.id)
            .collect()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RuleReachability {
    pub(crate) entry_rules: Vec<RuleId>,
    pub(crate) unreachable_rules: Vec<RuleId>,
}

pub(crate) fn analyze(unit: &GrammarUnit, configured: &EntryRuleConfig) -> RuleReachability {
    if unit.kind != GrammarKind::Parser {
        return RuleReachability::default();
    }
    let parser_rules = unit
        .rules
        .iter()
        .filter(|rule| rule.kind == RuleKind::Parser)
        .collect::<Vec<_>>();
    let names = parser_rules
        .iter()
        .map(|rule| (rule.name.clone(), rule.id))
        .collect::<BTreeMap<_, _>>();
    let call_graph = parser_rules
        .iter()
        .map(|rule| {
            let mut calls = BTreeSet::new();
            collect_calls_into(&rule.block, &names, &mut |target| {
                calls.insert(target);
            });
            (rule.id, calls)
        })
        .collect::<BTreeMap<_, _>>();
    let source_component_rules = source_component_rules(&call_graph);
    let mut reaches_eof = parser_rules
        .iter()
        .filter(|rule| rule_contains_eof(rule))
        .map(|rule| rule.id)
        .collect::<BTreeSet<_>>();
    loop {
        let previous = reaches_eof.len();
        for (caller, targets) in &call_graph {
            if targets.iter().any(|target| reaches_eof.contains(target)) {
                reaches_eof.insert(*caller);
            }
        }
        if reaches_eof.len() == previous {
            break;
        }
    }

    let mut entry_rules = parser_rules
        .iter()
        .filter(|rule| {
            configured.names.contains(&rule.name)
                || (reaches_eof.contains(&rule.id) && source_component_rules.contains(&rule.id))
        })
        .map(|rule| rule.id)
        .collect::<Vec<_>>();
    if entry_rules.is_empty()
        && let Some(first) = parser_rules.first()
    {
        entry_rules.push(first.id);
    }

    let mut reachable = BTreeSet::new();
    let mut pending = entry_rules.clone();
    while let Some(rule) = pending.pop() {
        if !reachable.insert(rule) {
            continue;
        }
        if let Some(calls) = call_graph.get(&rule) {
            pending.extend(calls.iter().copied());
        }
    }
    let unreachable_rules = parser_rules
        .into_iter()
        .filter_map(|rule| (!reachable.contains(&rule.id)).then_some(rule.id))
        .collect();
    RuleReachability {
        entry_rules,
        unreachable_rules,
    }
}

fn source_component_rules(call_graph: &BTreeMap<RuleId, BTreeSet<RuleId>>) -> BTreeSet<RuleId> {
    let mut graph = DiGraph::<RuleId, ()>::new();
    let nodes = call_graph
        .keys()
        .map(|rule| (*rule, graph.add_node(*rule)))
        .collect::<BTreeMap<_, _>>();
    for (source, targets) in call_graph {
        for target in targets {
            graph.add_edge(nodes[source], nodes[target], ());
        }
    }

    let components = tarjan_scc(&graph);
    let mut component_by_rule = BTreeMap::new();
    for (component_index, component) in components.iter().enumerate() {
        for node in component {
            component_by_rule.insert(graph[*node], component_index);
        }
    }
    let mut has_external_incoming = vec![false; components.len()];
    for (source, targets) in call_graph {
        let source_component = component_by_rule[source];
        for target in targets {
            let target_component = component_by_rule[target];
            if source_component != target_component {
                has_external_incoming[target_component] = true;
            }
        }
    }

    call_graph
        .keys()
        .filter(|rule| !has_external_incoming[component_by_rule[rule]])
        .copied()
        .collect()
}

fn rule_contains_eof(rule: &Rule) -> bool {
    block_contains_eof(&rule.block)
}

fn block_contains_eof(block: &Block) -> bool {
    block.alternatives.iter().any(|alternative| {
        alternative
            .elements
            .iter()
            .any(|element| match &element.kind {
                ElementKind::Terminal(terminal) => terminal_is_eof(terminal),
                ElementKind::Set {
                    inverted: false,
                    elements,
                } => elements.iter().any(|element| {
                    matches!(
                        element,
                        SetElement::Terminal { value, .. } if terminal_is_eof(value)
                    )
                }),
                ElementKind::Block(nested) => block_contains_eof(nested),
                _ => false,
            })
    })
}

fn terminal_is_eof(terminal: &Terminal) -> bool {
    matches!(terminal, Terminal::Eof) || matches!(terminal, Terminal::Token(name) if name == "EOF")
}
