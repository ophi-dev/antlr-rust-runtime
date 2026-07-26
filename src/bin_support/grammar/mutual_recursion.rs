//! Mutual (indirect) left-recursion elimination — issue #151.
//!
//! ANTLR 4 rewrites *direct* left recursion (`e : e '+' e | INT`) into a
//! precedence-climbing rule, but rejects *mutual* (indirect) left recursion — a
//! left-corner cycle through two or more rules — with `error(119)`. This pass
//! accepts the tractable subclass of those cycles by rewriting them, on the
//! model, into an equivalent grammar that uses only *direct* left recursion, so
//! the existing [`rewrite_immediate_left_recursion`](super::left_recursion)
//! machinery (and, as a differential oracle, ANTLR itself) can handle them.
//!
//! The rewrite is **left-corner substitution** ("hub inlining"): for each
//! left-corner cycle, one member is chosen as the *hub*; every other member (a
//! *satellite*) reachable in left-corner position from the hub has its
//! alternatives inlined into the hub until the hub is directly left-recursive.
//! Satellites referenced only from within the cycle are then removed; a
//! satellite referenced from outside the cycle is retained unchanged (its body
//! already calls the hub, which is now the precedence rule).
//!
//! The pass is deliberately conservative. It commits a rewrite only when the
//! rebuilt hub is a shape the direct-recursion classifier accepts (every
//! alternative Primary/Prefix/Binary/Suffix). Any cycle it cannot reduce to
//! that shape — argument-bearing recursive calls, no token-consuming base case,
//! non-convergent substitution — is left **untouched and silent**, so the
//! downstream ATN-level `G4A005` detector reports it exactly as before.
//! Correctness is therefore guaranteed by construction: the pass either emits a
//! grammar the existing (conformance-verified) direct-recursion path accepts,
//! or it changes nothing.
//!
//! See `docs/issue-151-mutual-left-recursion-plan.md` for the full design and
//! the empirical validation against `dotnet/roslyn`'s `CSharp.Generated.g4`.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use super::model::{
    Alternative, AlternativeId, Block, Element, ElementKind, GrammarUnit, ModelIdAllocator,
    ModelNodeId, Quantifier, Rule, RuleCall, RuleId,
};
use super::provenance::{Origin, ProvenanceIndex, SyntheticReason};

/// Rewrite the tractable mutual-left-recursion cycles in `units` into direct
/// left recursion, in place. Returns `true` when at least one cycle was
/// rewritten (the model changed). Cycles that cannot be reduced are left
/// untouched — the ATN-level `G4A005` detector reports them downstream.
pub(crate) fn eliminate_mutual_left_recursion(
    units: &mut [GrammarUnit],
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> bool {
    let mut changed = false;
    for unit in units.iter_mut() {
        changed |= eliminate_in_unit(unit, ids, provenance);
    }
    changed
}

/// The rules of one left-corner strongly-connected component, in a
/// deterministic order (ascending `RuleId`).
type Cycle = Vec<RuleId>;

/// Read-only view of one grammar unit's rule analysis, threaded through the
/// left-corner walks: rule-name lookup and the nullable-rule set.
#[derive(Clone, Copy)]
struct Grammar<'a> {
    names: &'a BTreeMap<String, RuleId>,
    nullable: &'a BTreeSet<RuleId>,
}

fn eliminate_in_unit(
    unit: &mut GrammarUnit,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> bool {
    let names = rule_names(unit);
    let nullable = nullable_rules(unit, &names);
    let cycles = left_corner_cycles(
        unit,
        Grammar {
            names: &names,
            nullable: &nullable,
        },
    );
    if cycles.is_empty() {
        return false;
    }

    let mut changed = false;
    for cycle in cycles {
        // Recompute names each iteration: a prior cycle may have removed rules.
        let names = rule_names(unit);
        let nullable = nullable_rules(unit, &names);
        let grammar = Grammar {
            names: &names,
            nullable: &nullable,
        };
        changed |= eliminate_cycle(unit, &cycle, grammar, ids, provenance);
    }
    changed
}

fn eliminate_cycle(
    unit: &mut GrammarUnit,
    cycle: &Cycle,
    grammar: Grammar<'_>,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> bool {
    // A cycle can be invalidated by an earlier rewrite in the same unit; ignore
    // members that no longer exist.
    if cycle
        .iter()
        .any(|member| !unit.rules.iter().any(|rule| rule.id == *member))
    {
        return false;
    }
    let cycle_set = cycle.iter().copied().collect::<BTreeSet<_>>();
    let Some(hub_id) = choose_hub(unit, cycle, &cycle_set, grammar) else {
        // No cycle member has a token-consuming base alternative: the language
        // is ill-founded. Leave it for the ATN detector's G4A005.
        return false;
    };

    // Work on clones so a mid-rewrite bail-out leaves the model pristine.
    let mut rules_by_id = unit
        .rules
        .iter()
        .map(|rule| (rule.id, rule.clone()))
        .collect::<BTreeMap<_, _>>();

    // Expand leading-optional recursive corners (`X? rest` -> `X rest | rest`)
    // across every cycle member before substituting.
    for &member in cycle {
        if let Some(rule) = rules_by_id.get_mut(&member) {
            expand_leading_optionals(rule, &cycle_set, grammar, ids, provenance);
        }
    }

    // Substitute satellites into the hub's left-corner positions until every
    // hub alternative's left corner is either non-recursive or the hub itself.
    let Some(hub_block) =
        substitute_into_hub(hub_id, &cycle_set, &rules_by_id, grammar, ids, provenance)
    else {
        return false;
    };

    // Gate: the rebuilt hub must be a shape the direct-recursion classifier
    // accepts. Otherwise decline silently and let G4A005 fire downstream.
    if !hub_is_directly_rewritable(&hub_block, hub_id, grammar.names) {
        return false;
    }

    // Commit. Install the rebuilt hub, then drop satellites that no retained
    // rule still references.
    if let Some(hub) = rules_by_id.get_mut(&hub_id) {
        hub.block = hub_block;
    }
    let removable = removable_satellites(&rules_by_id, cycle, hub_id, grammar.names);
    record_hub_provenance(provenance, hub_id, cycle);

    unit.rules.retain(|rule| !removable.contains(&rule.id));
    for rule in &mut unit.rules {
        if let Some(rebuilt) = rules_by_id.remove(&rule.id) {
            *rule = rebuilt;
        }
    }
    true
}

/// Choose the hub of a cycle: prefer a member with a token-consuming base
/// alternative that is also referenced from outside the cycle (the public
/// entry). Ties break on lowest `RuleId` for determinism. Returns `None` when
/// no member has a base case (ill-founded cycle we must not touch).
fn choose_hub(
    unit: &GrammarUnit,
    cycle: &Cycle,
    cycle_set: &BTreeSet<RuleId>,
    grammar: Grammar<'_>,
) -> Option<RuleId> {
    let external = externally_referenced(unit, cycle_set, grammar.names);
    let rules_by_id = unit
        .rules
        .iter()
        .map(|rule| (rule.id, rule))
        .collect::<BTreeMap<_, _>>();
    let has_base = |id: &RuleId| -> bool {
        rules_by_id.get(id).is_some_and(|rule| {
            rule.block
                .alternatives
                .iter()
                .any(|alternative| !left_corner_is_cycle_member(alternative, cycle_set, grammar))
        })
    };
    let candidates = cycle.iter().copied().filter(has_base).collect::<Vec<_>>();
    candidates
        .iter()
        .copied()
        .filter(|id| external.contains(id))
        .min()
        .or_else(|| candidates.into_iter().min())
}

/// Set of cycle members referenced by a rule that is *not* in the cycle.
fn externally_referenced(
    unit: &GrammarUnit,
    cycle_set: &BTreeSet<RuleId>,
    names: &BTreeMap<String, RuleId>,
) -> BTreeSet<RuleId> {
    let mut external = BTreeSet::new();
    for rule in &unit.rules {
        if cycle_set.contains(&rule.id) {
            continue;
        }
        collect_calls_into(&rule.block, names, &mut |target| {
            if cycle_set.contains(&target) {
                external.insert(target);
            }
        });
    }
    external
}

/// Satellites (cycle members other than the hub) that can be safely removed:
/// no rule that will be *retained* references them. Computed to a fixpoint so a
/// retained satellite pulls its own dependencies back in.
fn removable_satellites(
    rules_by_id: &BTreeMap<RuleId, Rule>,
    cycle: &Cycle,
    hub_id: RuleId,
    names: &BTreeMap<String, RuleId>,
) -> BTreeSet<RuleId> {
    let mut removable = cycle
        .iter()
        .copied()
        .filter(|member| *member != hub_id)
        .collect::<BTreeSet<_>>();
    loop {
        let mut referenced_by_retained = BTreeSet::new();
        for rule in rules_by_id.values() {
            if removable.contains(&rule.id) {
                continue;
            }
            collect_calls_into(&rule.block, names, &mut |target| {
                if removable.contains(&target) {
                    referenced_by_retained.insert(target);
                }
            });
        }
        if referenced_by_retained.is_empty() {
            return removable;
        }
        for target in referenced_by_retained {
            removable.remove(&target);
        }
    }
}

/// Expand every alternative whose left corner is an *optional* reference to a
/// cycle member: `X? rest` becomes two alternatives `X rest | rest`. This is
/// union-preserving and lets a leading-optional recursion (C#'s
/// `expression? '..' expression?`) reduce to a well-formed direct form.
fn expand_leading_optionals(
    rule: &mut Rule,
    cycle_set: &BTreeSet<RuleId>,
    grammar: Grammar<'_>,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) {
    let mut expanded = Vec::with_capacity(rule.block.alternatives.len());
    for alternative in std::mem::take(&mut rule.block.alternatives) {
        if let Some((with_corner, without_corner)) =
            split_leading_optional(&alternative, cycle_set, grammar)
        {
            expanded.push(renumber_alternative(
                with_corner,
                ids,
                provenance,
                alternative.id,
            ));
            expanded.push(renumber_alternative(
                without_corner,
                ids,
                provenance,
                alternative.id,
            ));
        } else {
            expanded.push(alternative);
        }
    }
    rule.block.alternatives = expanded;
}

/// If `alternative` begins with an optional reference to a cycle member (after
/// skipping leading epsilon elements), produce the `X rest` / `rest` pair.
fn split_leading_optional(
    alternative: &Alternative,
    cycle_set: &BTreeSet<RuleId>,
    grammar: Grammar<'_>,
) -> Option<(Alternative, Alternative)> {
    let index = first_significant_index(&alternative.elements)?;
    let element = &alternative.elements[index];
    let ElementKind::RuleCall(call) = &element.kind else {
        return None;
    };
    if !matches!(element.quantifier, Quantifier::Optional { .. }) {
        return None;
    }
    let target = grammar.names.get(&call.name)?;
    if !cycle_set.contains(target) {
        return None;
    }
    // Only split a genuine *left* corner: nothing token-consuming precedes it.
    if alternative.elements[..index]
        .iter()
        .any(|prefix| !is_epsilon_element(prefix, grammar.names, grammar.nullable))
    {
        return None;
    }

    let mut with_corner = alternative.clone();
    if let Some(element) = with_corner.elements.get_mut(index) {
        element.quantifier = Quantifier::One;
    }
    let mut without_corner = alternative.clone();
    without_corner.elements.remove(index);
    Some((with_corner, without_corner))
}

/// Build the hub's rewritten block by inlining satellite bodies into every
/// left-corner-satellite alternative, transitively, until each alternative's
/// left corner is non-recursive or the hub itself. Returns `None` if
/// substitution fails to converge within a generous bound.
fn substitute_into_hub(
    hub_id: RuleId,
    cycle_set: &BTreeSet<RuleId>,
    rules_by_id: &BTreeMap<RuleId, Rule>,
    grammar: Grammar<'_>,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> Option<Block> {
    let hub = rules_by_id.get(&hub_id)?;
    let mut worklist = hub.block.alternatives.clone();
    let mut result = Vec::new();
    let budget = substitution_budget(cycle_set, rules_by_id);
    let mut steps: usize = 0;

    while let Some(alternative) = pop_front(&mut worklist) {
        match left_corner_target(&alternative, grammar) {
            Some(target) if target == hub_id => result.push(alternative),
            Some(target) if cycle_set.contains(&target) => {
                steps += 1;
                if steps > budget {
                    return None;
                }
                let satellite = rules_by_id.get(&target)?;
                for expansion in inline_satellite(&alternative, satellite, ids, provenance) {
                    worklist.push(expansion);
                }
            }
            _ => result.push(alternative),
        }
    }

    Some(Block {
        alternatives: result,
        options: hub.block.options.clone(),
        syntax: hub.block.syntax,
        span: hub.block.span.clone(),
    })
}

/// Replace the leading satellite reference in `alternative` with each of the
/// satellite's alternatives, concatenating the satellite body with the trailing
/// elements of the original alternative. One input alternative yields one
/// output per satellite alternative.
fn inline_satellite(
    alternative: &Alternative,
    satellite: &Rule,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> Vec<Alternative> {
    let Some(corner_index) = alternative
        .elements
        .iter()
        .position(|element| matches!(element.kind, ElementKind::RuleCall(_)))
    else {
        return vec![alternative.clone()];
    };
    let prefix = &alternative.elements[..corner_index];
    let suffix = &alternative.elements[corner_index + 1..];

    satellite
        .block
        .alternatives
        .iter()
        .map(|sat_alt| {
            let mut elements =
                Vec::with_capacity(prefix.len() + sat_alt.elements.len() + suffix.len());
            elements.extend(prefix.iter().cloned());
            elements.extend(sat_alt.elements.iter().cloned());
            elements.extend(suffix.iter().cloned());
            let id = ids.alternative();
            provenance.record_model(
                ModelNodeId::Alternative(id),
                [Origin::Synthetic {
                    reason: SyntheticReason::RuleBoundary,
                    owner: ModelNodeId::Alternative(sat_alt.id),
                }],
            );
            Alternative {
                id,
                elements: renumber_elements(elements, ids),
                label: alternative.label.clone(),
                options: alternative.options.clone(),
                commands: alternative.commands.clone(),
                syntax: alternative.syntax,
                span: alternative.span.clone(),
            }
        })
        .collect()
}

/// Whether the rebuilt hub is a shape [`rewrite_immediate_left_recursion`]
/// accepts: at least one non-left-recursive (primary/prefix) alternative, and
/// every left-recursive alternative starts with a bare hub reference followed
/// by at least one more element — never a bare self-loop, never carrying
/// arguments.
fn hub_is_directly_rewritable(
    block: &Block,
    hub_id: RuleId,
    names: &BTreeMap<String, RuleId>,
) -> bool {
    let mut has_primary = false;
    let mut has_recursive = false;
    for alternative in &block.alternatives {
        // Any argument-bearing self-reference is nonconforming.
        if alternative.elements.iter().any(|element| {
            self_call(element, hub_id, names).is_some_and(|call| call.arguments.is_some())
        }) {
            return false;
        }
        let first_recursive = alternative
            .elements
            .first()
            .is_some_and(|element| is_hub_call(element, hub_id, names));
        let last_significant = alternative.elements.iter().rposition(|element| {
            !matches!(
                element.kind,
                ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon
            )
        });
        let last_recursive = last_significant
            .and_then(|index| alternative.elements.get(index))
            .is_some_and(|element| is_hub_call(element, hub_id, names));
        if first_recursive {
            match last_significant {
                // A bare `hub` with nothing after it is a nonconforming
                // self-loop (matches the direct rewriter's rejection).
                Some(0) | None => return false,
                Some(_) => has_recursive = true,
            }
        } else if last_recursive {
            has_recursive = true;
        } else {
            has_primary = true;
        }
    }
    has_primary && has_recursive
}

fn is_hub_call(element: &Element, hub_id: RuleId, names: &BTreeMap<String, RuleId>) -> bool {
    self_call(element, hub_id, names).is_some()
}

fn self_call<'a>(
    element: &'a Element,
    hub_id: RuleId,
    names: &BTreeMap<String, RuleId>,
) -> Option<&'a RuleCall> {
    match &element.kind {
        ElementKind::RuleCall(call)
            if element.quantifier == Quantifier::One && names.get(&call.name) == Some(&hub_id) =>
        {
            Some(call)
        }
        _ => None,
    }
}

/// Whether `alternative`'s left corner is a cycle member.
fn left_corner_is_cycle_member(
    alternative: &Alternative,
    cycle_set: &BTreeSet<RuleId>,
    grammar: Grammar<'_>,
) -> bool {
    left_corner_target(alternative, grammar).is_some_and(|target| cycle_set.contains(&target))
}

/// The target rule of `alternative`'s left corner, if the left corner is a rule
/// call. Walks through leading epsilon-only and nullable-rule prefix elements
/// (matching the left-corner relation the ATN detector uses).
fn left_corner_target(alternative: &Alternative, grammar: Grammar<'_>) -> Option<RuleId> {
    for element in &alternative.elements {
        match &element.kind {
            ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon => {}
            ElementKind::RuleCall(call) => {
                let target = grammar.names.get(&call.name).copied();
                let skippable = matches!(
                    element.quantifier,
                    Quantifier::Optional { .. } | Quantifier::ZeroOrMore { .. }
                ) || target
                    .is_some_and(|target| grammar.nullable.contains(&target));
                if !skippable {
                    return target;
                }
                // Skippable call that is *not* a cycle-relevant nullable: it
                // could still be the left corner, so report it.
                if target.is_none_or(|target| !grammar.nullable.contains(&target)) {
                    return target;
                }
            }
            _ => return None,
        }
    }
    None
}

/// Index of the first element that is not epsilon-only (action/predicate/eps).
fn first_significant_index(elements: &[Element]) -> Option<usize> {
    elements.iter().position(|element| {
        !matches!(
            element.kind,
            ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon
        )
    })
}

fn is_epsilon_element(
    element: &Element,
    names: &BTreeMap<String, RuleId>,
    nullable: &BTreeSet<RuleId>,
) -> bool {
    if matches!(
        element.quantifier,
        Quantifier::Optional { .. } | Quantifier::ZeroOrMore { .. }
    ) {
        return true;
    }
    match &element.kind {
        ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon => true,
        ElementKind::RuleCall(call) => names
            .get(&call.name)
            .is_some_and(|target| nullable.contains(target)),
        _ => false,
    }
}

/// A generous upper bound on substitution steps (squared alternative count),
/// guarding against non-convergence without rejecting deep legitimate chains.
fn substitution_budget(
    cycle_set: &BTreeSet<RuleId>,
    rules_by_id: &BTreeMap<RuleId, Rule>,
) -> usize {
    let alt_count: usize = cycle_set
        .iter()
        .filter_map(|id| rules_by_id.get(id))
        .map(|rule| rule.block.alternatives.len())
        .sum();
    alt_count.saturating_mul(alt_count).max(64)
}

fn record_hub_provenance(provenance: &mut ProvenanceIndex, hub_id: RuleId, cycle: &Cycle) {
    let _ = cycle;
    provenance.record_model(
        ModelNodeId::Rule(hub_id),
        [Origin::Synthetic {
            reason: SyntheticReason::RuleBoundary,
            owner: ModelNodeId::Rule(hub_id),
        }],
    );
}

/// Compute left-corner strongly-connected components (size > 1) over the parser
/// rules of one unit, each returned as an ascending list of `RuleId`. Single
/// directly-left-recursive rules are excluded — those belong to
/// [`rewrite_immediate_left_recursion`] and must not be touched here.
fn left_corner_cycles(unit: &GrammarUnit, grammar: Grammar<'_>) -> Vec<Cycle> {
    let mut graph = DiGraph::<RuleId, ()>::new();
    let nodes = unit
        .rules
        .iter()
        .map(|rule| (rule.id, graph.add_node(rule.id)))
        .collect::<BTreeMap<_, _>>();
    for rule in &unit.rules {
        let mut corners = BTreeSet::new();
        for alternative in &rule.block.alternatives {
            collect_left_corner_calls(alternative, grammar, &mut corners);
        }
        for target in corners {
            if let (Some(source), Some(target)) = (nodes.get(&rule.id), nodes.get(&target)) {
                graph.add_edge(*source, *target, ());
            }
        }
    }

    let mut cycles = tarjan_scc(&graph)
        .into_iter()
        .filter_map(|component| {
            (component.len() > 1).then(|| {
                let mut rules = component
                    .into_iter()
                    .map(|node| graph[node])
                    .collect::<Cycle>();
                rules.sort_unstable();
                rules
            })
        })
        .collect::<Vec<_>>();
    cycles.sort();
    cycles
}

/// Collect the cycle-candidate left corners of one alternative: every rule
/// reachable in left-corner position (through leading epsilon/nullable
/// elements). Yields *all* nullable-prefix corners so the SCC graph captures
/// the full left-corner relation.
fn collect_left_corner_calls(
    alternative: &Alternative,
    grammar: Grammar<'_>,
    result: &mut BTreeSet<RuleId>,
) {
    for element in &alternative.elements {
        match &element.kind {
            ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon => {}
            ElementKind::RuleCall(call) => {
                let Some(target) = grammar.names.get(&call.name) else {
                    return;
                };
                result.insert(*target);
                let optional = matches!(
                    element.quantifier,
                    Quantifier::Optional { .. } | Quantifier::ZeroOrMore { .. }
                );
                if !optional && !grammar.nullable.contains(target) {
                    return;
                }
            }
            ElementKind::Block(block) => {
                for nested in &block.alternatives {
                    collect_left_corner_calls(nested, grammar, result);
                }
                let optional = matches!(
                    element.quantifier,
                    Quantifier::Optional { .. } | Quantifier::ZeroOrMore { .. }
                );
                if !optional && !block_is_nullable(block, grammar.names, grammar.nullable) {
                    return;
                }
            }
            _ => return,
        }
    }
}

fn collect_calls_into(
    block: &Block,
    names: &BTreeMap<String, RuleId>,
    sink: &mut impl FnMut(RuleId),
) {
    for alternative in &block.alternatives {
        for element in &alternative.elements {
            match &element.kind {
                ElementKind::RuleCall(call) => {
                    if let Some(target) = names.get(&call.name) {
                        sink(*target);
                    }
                }
                ElementKind::Block(nested) => collect_calls_into(nested, names, sink),
                _ => {}
            }
        }
    }
}

fn rule_names(unit: &GrammarUnit) -> BTreeMap<String, RuleId> {
    unit.rules
        .iter()
        .map(|rule| (rule.name.clone(), rule.id))
        .collect()
}

/// Rules that can derive the empty string (model-level, matching
/// `transform_analysis::compute_nullable`).
fn nullable_rules(unit: &GrammarUnit, names: &BTreeMap<String, RuleId>) -> BTreeSet<RuleId> {
    let rules = unit
        .rules
        .iter()
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
    element: &Element,
    names: &BTreeMap<String, RuleId>,
    nullable: &BTreeSet<RuleId>,
) -> bool {
    if matches!(
        element.quantifier,
        Quantifier::Optional { .. } | Quantifier::ZeroOrMore { .. }
    ) {
        return true;
    }
    match &element.kind {
        ElementKind::Epsilon | ElementKind::Action { .. } | ElementKind::Predicate { .. } => true,
        ElementKind::RuleCall(call) => names
            .get(&call.name)
            .is_some_and(|target| nullable.contains(target)),
        ElementKind::Block(block) => block_is_nullable(block, names, nullable),
        _ => false,
    }
}

fn block_is_nullable(
    block: &Block,
    names: &BTreeMap<String, RuleId>,
    nullable: &BTreeSet<RuleId>,
) -> bool {
    block.alternatives.iter().any(|alternative| {
        alternative
            .elements
            .iter()
            .all(|element| element_nullable(element, names, nullable))
    })
}

fn pop_front<T>(items: &mut Vec<T>) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(items.remove(0))
    }
}

/// Assign fresh IDs to an alternative and its elements (used when splitting a
/// leading-optional alternative into two).
fn renumber_alternative(
    mut alternative: Alternative,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
    origin_alt: AlternativeId,
) -> Alternative {
    alternative.id = ids.alternative();
    provenance.record_model(
        ModelNodeId::Alternative(alternative.id),
        [Origin::Synthetic {
            reason: SyntheticReason::BlockBoundary,
            owner: ModelNodeId::Alternative(origin_alt),
        }],
    );
    alternative.elements = renumber_elements(alternative.elements, ids);
    alternative
}

fn renumber_elements(elements: Vec<Element>, ids: &mut ModelIdAllocator) -> Vec<Element> {
    elements
        .into_iter()
        .map(|mut element| {
            element.id = ids.element();
            element
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
mod tests {
    use super::*;
    use crate::grammar::frontend::{SourceId, parse_source};
    use crate::grammar::left_recursion::rewrite_immediate_left_recursion;
    use crate::grammar::model::{GrammarId, GrammarKind, Terminal};
    use crate::grammar::syntax::parse_grammar_unit;

    /// Parse one parser grammar into a unit, run the mutual-recursion pass, and
    /// return the unit plus whether anything changed.
    fn run(text: &str) -> (GrammarUnit, bool) {
        let file = parse_source(SourceId::new(0), "P.g4", text).expect("valid grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let mut unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);
        assert_eq!(unit.kind, GrammarKind::Parser);
        let changed = eliminate_mutual_left_recursion(
            std::slice::from_mut(&mut unit),
            &mut ids,
            &mut provenance,
        );
        (unit, changed)
    }

    /// Render a unit's rules as `rule: alt | alt ;` lines with a compact
    /// per-element notation, for observable snapshots.
    fn render(unit: &GrammarUnit) -> String {
        let mut out = String::new();
        for rule in &unit.rules {
            out.push_str(&rule.name);
            out.push_str(":\n");
            for alternative in &rule.block.alternatives {
                out.push_str("  | ");
                out.push_str(&render_elements(&alternative.elements));
                out.push('\n');
            }
        }
        out
    }

    fn render_elements(elements: &[Element]) -> String {
        elements
            .iter()
            .map(render_element)
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn render_element(element: &Element) -> String {
        let quant = match element.quantifier {
            Quantifier::One => "",
            Quantifier::Optional { .. } => "?",
            Quantifier::ZeroOrMore { .. } => "*",
            Quantifier::OneOrMore { .. } => "+",
        };
        let body = match &element.kind {
            ElementKind::RuleCall(call) => call.name.clone(),
            ElementKind::Terminal(Terminal::Literal(text)) => format!("'{text}'"),
            ElementKind::Terminal(Terminal::Token(name)) => name.clone(),
            ElementKind::Terminal(_) => "<terminal>".to_owned(),
            ElementKind::Set { .. } => "<set>".to_owned(),
            ElementKind::Block(_) => "<block>".to_owned(),
            ElementKind::Range(..) => "<range>".to_owned(),
            ElementKind::Action { .. } => "<action>".to_owned(),
            ElementKind::Predicate { .. } => "<pred>".to_owned(),
            ElementKind::Epsilon => "<eps>".to_owned(),
        };
        format!("{body}{quant}")
    }

    fn rule<'a>(unit: &'a GrammarUnit, name: &str) -> &'a Rule {
        unit.rules
            .iter()
            .find(|rule| rule.name == name)
            .unwrap_or_else(|| panic!("rule {name} exists"))
    }

    #[test]
    fn collapses_two_rule_name_cycle_into_the_hub() {
        // name <-> qualified_name: the classic 2-rule mutual cycle. qualified_name
        // is referenced only by name, so it collapses away entirely.
        let (unit, changed) = run("parser grammar P; \
             name : qualified_name | simple_name ; \
             qualified_name : name '.' simple_name ; \
             simple_name : ID ;");
        assert!(changed);
        assert!(
            unit.rules.iter().all(|rule| rule.name != "qualified_name"),
            "hub-only satellite is removed"
        );
        insta::assert_snapshot!("name_cycle_collapsed", render(&unit));
    }

    #[test]
    fn collapsed_hub_is_then_rewritten_by_the_direct_pass() {
        // End-to-end: after collapse, the existing direct-recursion rewrite must
        // accept the hub and produce the precedence-climbing shape.
        let file = parse_source(
            SourceId::new(0),
            "P.g4",
            "parser grammar P; \
             name : qualified_name | simple_name ; \
             qualified_name : name '.' simple_name ; \
             simple_name : ID ;",
        )
        .expect("valid grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let mut unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);
        assert!(eliminate_mutual_left_recursion(
            std::slice::from_mut(&mut unit),
            &mut ids,
            &mut provenance,
        ));
        let diagnostics = rewrite_immediate_left_recursion(
            std::slice::from_mut(&mut unit),
            &mut ids,
            &mut provenance,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(
            rule(&unit, "name").left_recursion.is_some(),
            "collapsed hub is now a direct-left-recursion precedence rule"
        );
    }

    #[test]
    fn splits_optional_from_inlined_satellite() {
        // C#'s range-operator shape: the leading-optional recursion lives in the
        // satellite body (`r : e? '..' e?`). After inlining `r` into `e`, the
        // leading `e?` splits into `e '..' e?` (left-recursive) and `'..' e?`
        // (primary), yielding a well-formed direct-recursion hub.
        let (unit, changed) = run("parser grammar P; \
             e : e '+' e | r | ID ; \
             r : e? '..' e? ;");
        assert!(changed);
        insta::assert_snapshot!("optional_from_satellite", render(&unit));
    }

    #[test]
    fn range_operator_hub_is_accepted_by_the_direct_pass() {
        // End-to-end proof that the leading-optional split achieves its purpose:
        // the collapsed hub must be one the direct-recursion rewrite accepts.
        let file = parse_source(
            SourceId::new(0),
            "P.g4",
            "parser grammar P; \
             e : e '+' e | r | ID ; \
             r : e? '..' e? ;",
        )
        .expect("valid grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let mut unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);
        assert!(eliminate_mutual_left_recursion(
            std::slice::from_mut(&mut unit),
            &mut ids,
            &mut provenance,
        ));
        let diagnostics = rewrite_immediate_left_recursion(
            std::slice::from_mut(&mut unit),
            &mut ids,
            &mut provenance,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(rule(&unit, "e").left_recursion.is_some());
    }

    #[test]
    fn retains_satellite_referenced_from_outside_the_cycle() {
        // array_type-style: the satellite is left-recursive through the hub but
        // also called by a non-cycle rule, so it is kept (not removed).
        let (unit, changed) = run("parser grammar P; \
             t : arr | t '?' | ID ; \
             arr : t '[' ']' ; \
             new_arr : 'new' arr ;");
        assert!(changed);
        assert!(
            unit.rules.iter().any(|rule| rule.name == "arr"),
            "externally-referenced satellite is retained"
        );
        insta::assert_snapshot!("external_satellite_retained", render(&unit));
    }

    #[test]
    fn declines_cycle_without_a_token_consuming_operator() {
        // a:b; b:c; c:a|X reduces to `a : a | X` — a bare self-loop the direct
        // rewriter rejects. The pass must decline (leave the model untouched) so
        // the ATN-level G4A005 detector fires downstream.
        let before = run("parser grammar P; a : b ; b : c ; c : a | X ; ").0;
        let (after, changed) = run("parser grammar P; a : b ; b : c ; c : a | X ; ");
        assert!(!changed, "intractable cycle is declined");
        assert_eq!(render(&before), render(&after), "model is untouched");
    }

    #[test]
    fn ignores_grammar_without_mutual_recursion() {
        let (_, changed) = run("parser grammar P; \
             e : e '+' t | t ; \
             t : ID ;");
        assert!(
            !changed,
            "direct-only left recursion is left for the direct pass"
        );
    }

    #[test]
    fn declines_argument_bearing_recursion() {
        // A recursive call carrying arguments is nonconforming even after
        // inlining; the pass declines rather than producing an invalid hub.
        let (_, changed) = run("parser grammar P; \
             e : s | ID ; \
             s : e '+' e[3] ;");
        assert!(!changed, "argument-bearing recursion is declined");
    }
}
