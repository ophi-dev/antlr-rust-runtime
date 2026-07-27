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
//! alternatives spliced into the hub, in place, until the hub is directly
//! left-recursive. Satellites referenced only from within the cycle are then
//! removed; a satellite referenced from outside the cycle is retained unchanged
//! (its body already calls the hub, which is now the precedence rule).
//!
//! # Structure: decide, then act
//!
//! The pass is organised so that **every** decision is made against the
//! untouched model, before anything is mutated:
//!
//! 1. [`plan_cycle`] proves the whole rewrite is admissible and returns a
//!    [`CyclePlan`], or `None`. It allocates nothing and mutates nothing.
//! 2. [`apply_plan`] performs the planned splice. It is mechanical and cannot
//!    fail.
//!
//! That ordering is what makes the safety claim real: an inadmissible cycle is
//! left **bit-for-bit untouched and silent** — no IDs consumed, no provenance
//! written — so the downstream ATN-level `G4A005` detector reports it exactly as
//! it does today. The alternative (mutate, then check) previously lost
//! `<assoc=right>`, element labels, rule arguments and `*`-quantified corners by
//! dropping them mid-splice.
//!
//! # What is required of a cycle
//!
//! [`Requirements`] spells out the preconditions. In brief: the grammar must be
//! a parser grammar; every substituted left corner must be a *bare* call to a
//! cycle member — no quantifier, no label, no arguments, and nothing
//! token-consuming or nullable before it; satellites must carry no rule-level
//! attributes (arguments, returns, locals, `@init`/`@after`, `catch`/`finally`)
//! that inlining would silently drop; and the resulting hub must be a shape the
//! direct-recursion classifier already accepts. Anything else is declined.
//!
//! See `docs/issue-151-mutual-left-recursion-plan.md` for the full design and
//! the empirical validation against `dotnet/roslyn`'s `CSharp.Generated.g4`.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use super::model::{
    Alternative, AlternativeId, Block, Element, ElementKind, GrammarKind, GrammarUnit,
    ModelIdAllocator, ModelNodeId, Quantifier, Rule, RuleCall, RuleId, RuleKind,
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
        // Precedence rewriting is a parser-rule construct: a lexer rule cycle
        // must not be routed through it (the injected precedence predicates
        // become "unsupported embedded lexer action" much later, far from the
        // cause).
        if unit.kind == GrammarKind::Parser {
            changed |= eliminate_in_unit(unit, ids, provenance);
        }
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

impl Grammar<'_> {
    fn target(self, call: &RuleCall) -> Option<RuleId> {
        self.names.get(&call.name).copied()
    }
}

fn eliminate_in_unit(
    unit: &mut GrammarUnit,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> bool {
    let mut changed = false;
    // Re-derive the cycle set after each successful rewrite: splicing a
    // satellite body into the hub imports the satellite's own left corners, so
    // the remaining cycles are a function of the *current* model, not the
    // original one. `progress` bounds the loop — one rewrite per pass at most,
    // and each rewrite strictly reduces the number of cycle members.
    loop {
        let names = rule_names(unit);
        let nullable = nullable_rules(unit, &names);
        let grammar = Grammar {
            names: &names,
            nullable: &nullable,
        };
        let Some(plan) = left_corner_cycles(unit, grammar)
            .iter()
            .find_map(|cycle| plan_cycle(unit, cycle, grammar))
        else {
            return changed;
        };
        apply_plan(unit, &plan, ids, provenance);
        changed = true;
    }
}

/// A fully-decided rewrite: which hub absorbs which satellite alternatives, at
/// which element index, and which satellites may then be dropped. Produced only
/// from an untouched model, and only when every [`Requirements`] precondition
/// holds, so applying it cannot fail.
#[derive(Debug)]
struct CyclePlan {
    hub: RuleId,
    /// Rewritten hub alternatives, in final order, as (source alternative that
    /// supplied the elements, element list). The source is retained so
    /// provenance and alternative options can be attributed correctly.
    alternatives: Vec<PlannedAlternative>,
    /// Satellites safe to delete: in the cycle, not the hub, and unreferenced by
    /// every rule that survives.
    removable: BTreeSet<RuleId>,
}

#[derive(Debug)]
struct PlannedAlternative {
    /// Alternative whose *options*, label and commands the result inherits. For
    /// a spliced alternative this is the satellite's alternative, because that
    /// is where `<assoc=right>` and friends live.
    attributes_from: AlternativeId,
    /// Alternative the elements were last taken from, for provenance.
    origin: AlternativeId,
    elements: Vec<Element>,
    /// Set when this alternative is a verbatim copy of an original hub
    /// alternative, so `apply_plan` can reuse it untouched.
    verbatim: bool,
}

/// The preconditions a cycle must satisfy. Documented as a type so the decline
/// reasons stay enumerated in one place and testable one at a time.
///
/// * `ParserGrammarOnly` — lexer rules never reach here.
/// * `HasTokenConsumingBase` — some member has an alternative whose left corner
///   leaves the cycle, else the language is empty.
/// * `BareCorner` — every corner we substitute is an unquantified, unlabelled,
///   argument-free call, preceded only by actions/predicates/epsilon. This is
///   the single notion of "the left corner": one index, computed once, used for
///   both the decision and the splice.
/// * `InlinableSatellite` — satellites carry no rule-level attributes that
///   inlining would drop, and no alternative labels that would collide.
/// * `DirectlyRewritable` — the resulting hub is Primary/Prefix/Binary/Suffix
///   throughout, as [`super::left_recursion`] requires.
/// * `Converges` — substitution terminates within a bound.
struct Requirements;

/// Position of an alternative's left corner: the index of a bare call to a
/// cycle member, or a reason it is unusable.
enum Corner {
    /// A bare call to `target` at `index`, safe to substitute.
    Bare { index: usize, target: RuleId },
    /// The left corner does not enter the cycle: the alternative is a base case.
    OutsideCycle,
    /// The left corner reaches the cycle but is not substitutable (quantified,
    /// labelled, argument-bearing, or behind a nullable/optional prefix).
    Unusable,
}

/// Classify `alternative`'s left corner with respect to `cycle_set`.
///
/// This is the *only* place the corner position is derived. Everything
/// downstream consumes the returned index, which removes the class of bug where
/// the decision walked past a nullable prefix while the splice replaced "the
/// first rule call".
fn classify_corner(
    alternative: &Alternative,
    cycle_set: &BTreeSet<RuleId>,
    grammar: Grammar<'_>,
) -> Corner {
    for (index, element) in alternative.elements.iter().enumerate() {
        match &element.kind {
            // Epsilon-only elements cannot consume input, so keep scanning.
            ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon => {}
            ElementKind::RuleCall(call) => {
                let Some(target) = grammar.target(call) else {
                    return Corner::OutsideCycle;
                };
                if !cycle_set.contains(&target) {
                    // A nullable non-cycle call could be skipped, leaving a
                    // cycle member as the real corner. Substituting past it is
                    // not something we model, so decline rather than guess.
                    return if grammar.nullable.contains(&target) {
                        Corner::Unusable
                    } else {
                        Corner::OutsideCycle
                    };
                }
                // The corner is a cycle member: it must be bare to be spliced.
                let bare = element.quantifier == Quantifier::One
                    && element.label.is_none()
                    && call.arguments.is_none()
                    && element.options.is_empty();
                return if bare {
                    Corner::Bare { index, target }
                } else {
                    Corner::Unusable
                };
            }
            // A token, set, range or block consumes input (or is a structure we
            // do not descend into): the corner is settled here.
            _ => return Corner::OutsideCycle,
        }
    }
    Corner::OutsideCycle
}

/// Whether `alternative`'s left corner enters the cycle at all (usable or not).
fn corner_enters_cycle(
    alternative: &Alternative,
    cycle_set: &BTreeSet<RuleId>,
    grammar: Grammar<'_>,
) -> bool {
    !matches!(
        classify_corner(alternative, cycle_set, grammar),
        Corner::OutsideCycle
    )
}

/// Decide the whole rewrite for one cycle, or decline. Mutates nothing.
fn plan_cycle(unit: &GrammarUnit, cycle: &Cycle, grammar: Grammar<'_>) -> Option<CyclePlan> {
    let rules = rules_by_id(unit);
    if cycle.iter().any(|member| !rules.contains_key(member)) {
        return None;
    }
    let cycle_set: BTreeSet<RuleId> = cycle.iter().copied().collect();
    let hub_id = choose_hub(unit, cycle, &cycle_set, grammar)?;

    // Requirements::InlinableSatellite — check before planning any splice, so a
    // satellite carrying behaviour we would drop declines the whole cycle.
    for member in cycle.iter().filter(|member| **member != hub_id) {
        if !satellite_is_inlinable(rules[member]) {
            return None;
        }
    }

    // Expand leading-optional corners to a fixpoint, then splice, preserving
    // alternative order throughout (order *is* precedence).
    let mut planned: Vec<PlannedAlternative> = rules[&hub_id]
        .block
        .alternatives
        .iter()
        .map(|alternative| PlannedAlternative {
            attributes_from: alternative.id,
            origin: alternative.id,
            elements: alternative.elements.clone(),
            verbatim: true,
        })
        .collect();

    let budget = substitution_budget(&cycle_set, &rules);
    let mut steps: usize = 0;
    while let Some(position) = planned.iter().position(|candidate| {
        matches!(
            planned_corner(candidate, hub_id, &cycle_set, grammar),
            PlannedCorner::Optional { .. } | PlannedCorner::Satellite { .. }
        )
    }) {
        steps += 1;
        // Requirements::Converges
        if steps > budget {
            return None;
        }
        let replacement = match planned_corner(&planned[position], hub_id, &cycle_set, grammar) {
            PlannedCorner::Optional { index } => split_optional(&planned[position], index),
            PlannedCorner::Satellite { index, target } => {
                splice_satellite(&planned[position], index, rules[&target])?
            }
            PlannedCorner::Settled => unreachable!("position was found to need work"),
        };
        // Splice in place: the expansions occupy the slot of the alternative
        // they came from, so declared alternative order — and therefore
        // precedence — is preserved.
        planned.splice(position..=position, replacement);
    }

    // Requirements::DirectlyRewritable
    if !planned_hub_is_directly_rewritable(&planned, hub_id, grammar) {
        return None;
    }

    let removable = removable_satellites(unit, cycle, hub_id, grammar.names);
    Some(CyclePlan {
        hub: hub_id,
        alternatives: planned,
        removable,
    })
}

/// What still needs doing to a planned alternative before the hub is direct.
enum PlannedCorner {
    /// A leading optional call to a cycle member at `index`, to be split.
    Optional { index: usize },
    /// A bare call to satellite `target` at `index`, to be spliced.
    Satellite { index: usize, target: RuleId },
    /// Nothing to do: base case, or already a hub self-reference.
    Settled,
}

fn planned_corner(
    candidate: &PlannedAlternative,
    hub_id: RuleId,
    cycle_set: &BTreeSet<RuleId>,
    grammar: Grammar<'_>,
) -> PlannedCorner {
    for (index, element) in candidate.elements.iter().enumerate() {
        match &element.kind {
            ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon => {}
            ElementKind::RuleCall(call) => {
                let Some(target) = grammar.target(call) else {
                    return PlannedCorner::Settled;
                };
                if !cycle_set.contains(&target) {
                    return PlannedCorner::Settled;
                }
                // A leading `X?` where X is in the cycle: split it into the
                // present and absent branches (union-preserving) so the present
                // branch becomes a well-formed recursive corner. This applies to
                // the hub itself too — C#'s `expr? '..' expr?` is exactly that
                // shape — so it is checked before the self-reference test below.
                // Only a bare, unlabelled optional qualifies: a labelled one
                // would leave `$label` dangling in the absent branch.
                if matches!(element.quantifier, Quantifier::Optional { .. })
                    && element.label.is_none()
                    && call.arguments.is_none()
                {
                    return PlannedCorner::Optional { index };
                }
                // A non-optional hub self-reference is the goal state, not
                // something to substitute: recursing on it would never
                // terminate.
                if target == hub_id {
                    return PlannedCorner::Settled;
                }
                if element.quantifier != Quantifier::One {
                    // `*`/`+`/labelled/argument corners are not substitutable;
                    // planning already vetted the originals, but a spliced body
                    // can introduce one, so settle and let the final
                    // directly-rewritable gate decline.
                    return PlannedCorner::Settled;
                }
                if element.label.is_some() || call.arguments.is_some() {
                    return PlannedCorner::Settled;
                }
                return PlannedCorner::Satellite { index, target };
            }
            _ => return PlannedCorner::Settled,
        }
    }
    PlannedCorner::Settled
}

/// `α X? β` becomes `α X β | α β`, preserving order (present branch first, as
/// the authored greedy `?` prefers matching).
fn split_optional(candidate: &PlannedAlternative, index: usize) -> Vec<PlannedAlternative> {
    let mut present = candidate.elements.clone();
    if let Some(element) = present.get_mut(index) {
        element.quantifier = Quantifier::One;
    }
    let mut absent = candidate.elements.clone();
    absent.remove(index);
    vec![
        PlannedAlternative {
            attributes_from: candidate.attributes_from,
            origin: candidate.origin,
            elements: present,
            verbatim: false,
        },
        PlannedAlternative {
            attributes_from: candidate.attributes_from,
            origin: candidate.origin,
            elements: absent,
            verbatim: false,
        },
    ]
}

/// Replace the bare satellite call at `index` with each satellite alternative,
/// keeping the surrounding prefix and suffix. Returns `None` if any satellite
/// alternative cannot be inlined at this position.
fn splice_satellite(
    candidate: &PlannedAlternative,
    index: usize,
    satellite: &Rule,
) -> Option<Vec<PlannedAlternative>> {
    // Requirements::BareCorner already established that `index` holds a bare
    // call; the caller's remaining elements are kept verbatim around it.
    let prefix = &candidate.elements[..index];
    let suffix = &candidate.elements[index + 1..];
    let mut expansions = Vec::with_capacity(satellite.block.alternatives.len());
    for source in &satellite.block.alternatives {
        // Merging two element lists merges their label scopes. If both sides
        // bind the same name, caller actions would silently rebind to the
        // satellite's element, so decline instead.
        if labels_collide(prefix, suffix, &source.elements) {
            return None;
        }
        let mut elements = Vec::with_capacity(prefix.len() + source.elements.len() + suffix.len());
        elements.extend(prefix.iter().cloned());
        elements.extend(source.elements.iter().cloned());
        elements.extend(suffix.iter().cloned());
        expansions.push(PlannedAlternative {
            // Take options from the satellite alternative: `<assoc=right>` is
            // declared there and drives the direct rewriter's associativity.
            attributes_from: source.id,
            origin: source.id,
            elements,
            verbatim: false,
        });
    }
    Some(expansions)
}

fn labels_collide(prefix: &[Element], suffix: &[Element], spliced: &[Element]) -> bool {
    let caller: BTreeSet<&str> = prefix
        .iter()
        .chain(suffix)
        .filter_map(|element| element.label.as_ref())
        .map(|label| label.name.as_str())
        .collect();
    spliced
        .iter()
        .filter_map(|element| element.label.as_ref())
        .any(|label| caller.contains(label.name.as_str()))
}

/// A satellite may only be inlined if nothing rule-level would be lost. Rule
/// arguments/returns/locals, `@init`/`@after` actions, `catch`/`finally`
/// handlers and `#`-labelled alternatives all attach to the *rule*, and vanish
/// when the rule does.
fn satellite_is_inlinable(satellite: &Rule) -> bool {
    satellite.kind == RuleKind::Parser
        && satellite.arguments.is_none()
        && satellite.returns.is_none()
        && satellite.locals.is_none()
        && satellite.throws.is_empty()
        && satellite.actions.is_empty()
        && satellite.catches.is_empty()
        && satellite.finally_action.is_none()
        && satellite.options.is_empty()
        && satellite
            .block
            .alternatives
            .iter()
            .all(|alternative| alternative.label.is_none())
}

/// Whether the planned hub is a shape [`super::left_recursion`] accepts: at
/// least one primary and one recursive alternative, every recursive alternative
/// a bare hub reference with something after it, and no argument-bearing
/// self-reference.
fn planned_hub_is_directly_rewritable(
    planned: &[PlannedAlternative],
    hub_id: RuleId,
    grammar: Grammar<'_>,
) -> bool {
    let mut has_primary = false;
    let mut has_recursive = false;
    for candidate in planned {
        let elements = &candidate.elements;
        if elements.iter().any(|element| {
            hub_call(element, hub_id, grammar).is_some_and(|call| call.arguments.is_some())
        }) {
            return false;
        }
        let first_significant = elements
            .iter()
            .position(|element| !is_epsilon_only(element))
            .filter(|index| is_hub_call(&elements[*index], hub_id, grammar));
        let last_significant = elements
            .iter()
            .rposition(|element| !is_epsilon_only(element));
        let last_recursive = last_significant
            .and_then(|index| elements.get(index))
            .is_some_and(|element| is_hub_call(element, hub_id, grammar));
        match (first_significant, last_significant) {
            // A bare `hub` with nothing significant after it is a nonconforming
            // self-loop, exactly as the direct rewriter treats it.
            (Some(first), Some(last)) if first == last => return false,
            (Some(_), Some(_)) => has_recursive = true,
            (Some(_), None) => return false,
            (None, _) if last_recursive => has_recursive = true,
            (None, _) => has_primary = true,
        }
    }
    has_primary && has_recursive
}

const fn is_epsilon_only(element: &Element) -> bool {
    matches!(
        element.kind,
        ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon
    )
}

fn is_hub_call(element: &Element, hub_id: RuleId, grammar: Grammar<'_>) -> bool {
    hub_call(element, hub_id, grammar).is_some()
}

fn hub_call<'a>(
    element: &'a Element,
    hub_id: RuleId,
    grammar: Grammar<'_>,
) -> Option<&'a RuleCall> {
    match &element.kind {
        ElementKind::RuleCall(call)
            if element.quantifier == Quantifier::One && grammar.target(call) == Some(hub_id) =>
        {
            Some(call)
        }
        _ => None,
    }
}

/// Perform a planned rewrite. Mechanical: allocates the IDs and provenance the
/// plan implies, installs the hub block, and drops the removable satellites.
fn apply_plan(
    unit: &mut GrammarUnit,
    plan: &CyclePlan,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) {
    let hub_index = unit
        .rules
        .iter()
        .position(|rule| rule.id == plan.hub)
        .expect("planned hub exists");
    let attributes = collect_alternative_attributes(unit);
    let template = unit.rules[hub_index].block.alternatives.first().cloned();

    let alternatives = plan
        .alternatives
        .iter()
        .enumerate()
        .map(|(index, planned)| {
            let source = attributes
                .get(&planned.attributes_from)
                .or(template.as_ref())
                .expect("hub has at least one alternative");
            let id = if planned.verbatim {
                // An untouched hub alternative keeps its identity, so unrelated
                // provenance and label bindings stay valid.
                planned.origin
            } else {
                let fresh = ids.alternative();
                provenance.record_model(
                    ModelNodeId::Alternative(fresh),
                    [Origin::Synthetic {
                        reason: SyntheticReason::RuleBoundary,
                        owner: ModelNodeId::Alternative(planned.origin),
                    }],
                );
                fresh
            };
            let elements = if planned.verbatim {
                planned.elements.clone()
            } else {
                renumber_elements(planned.elements.clone(), ids, provenance)
            };
            let _ = index;
            Alternative {
                id,
                elements,
                label: source.label.clone(),
                options: source.options.clone(),
                commands: source.commands.clone(),
                syntax: source.syntax,
                span: source.span.clone(),
            }
        })
        .collect::<Vec<_>>();

    let hub = &mut unit.rules[hub_index];
    hub.block = Block {
        alternatives,
        options: hub.block.options.clone(),
        syntax: hub.block.syntax,
        span: hub.block.span.clone(),
    };
    provenance.record_model(
        ModelNodeId::Rule(plan.hub),
        [Origin::Synthetic {
            reason: SyntheticReason::RuleBoundary,
            owner: ModelNodeId::Rule(plan.hub),
        }],
    );
    unit.rules.retain(|rule| !plan.removable.contains(&rule.id));
}

/// Index every alternative in the unit so a plan can recover the options,
/// label and commands of whichever alternative supplied its attributes.
fn collect_alternative_attributes(unit: &GrammarUnit) -> BTreeMap<AlternativeId, Alternative> {
    let mut index = BTreeMap::new();
    for rule in &unit.rules {
        collect_block_alternatives(&rule.block, &mut index);
    }
    index
}

fn collect_block_alternatives(block: &Block, index: &mut BTreeMap<AlternativeId, Alternative>) {
    for alternative in &block.alternatives {
        index.insert(alternative.id, alternative.clone());
        for element in &alternative.elements {
            if let ElementKind::Block(nested) = &element.kind {
                collect_block_alternatives(nested, index);
            }
        }
    }
}

/// Choose the hub of a cycle: prefer a member with a token-consuming base
/// alternative that is also referenced from outside the cycle (the public
/// entry). Ties break on lowest `RuleId` for determinism. Returns `None` when no
/// member has a base case (ill-founded cycle we must not touch).
fn choose_hub(
    unit: &GrammarUnit,
    cycle: &Cycle,
    cycle_set: &BTreeSet<RuleId>,
    grammar: Grammar<'_>,
) -> Option<RuleId> {
    let external = externally_referenced(unit, cycle_set, grammar.names);
    let rules = rules_by_id(unit);
    let candidates = cycle
        .iter()
        .copied()
        .filter(|id| {
            rules.get(id).is_some_and(|rule| {
                rule.block
                    .alternatives
                    .iter()
                    .any(|alternative| !corner_enters_cycle(alternative, cycle_set, grammar))
            })
        })
        .collect::<Vec<_>>();
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

/// Satellites that can be safely removed: no rule that will be *retained*
/// references them. Computed to a fixpoint so a retained satellite pulls its own
/// dependencies back in.
fn removable_satellites(
    unit: &GrammarUnit,
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
        let mut referenced = BTreeSet::new();
        for rule in &unit.rules {
            // The hub's own body is about to be replaced wholesale, so its
            // current references do not keep a satellite alive.
            if removable.contains(&rule.id) || rule.id == hub_id {
                continue;
            }
            collect_calls_into(&rule.block, names, &mut |target| {
                if removable.contains(&target) {
                    referenced.insert(target);
                }
            });
        }
        if referenced.is_empty() {
            return removable;
        }
        for target in referenced {
            removable.remove(&target);
        }
    }
}

/// A generous upper bound on substitution steps, guarding against
/// non-convergence without rejecting deep legitimate chains.
fn substitution_budget(cycle_set: &BTreeSet<RuleId>, rules: &BTreeMap<RuleId, &Rule>) -> usize {
    let alternatives: usize = cycle_set
        .iter()
        .filter_map(|id| rules.get(id))
        .map(|rule| rule.block.alternatives.len())
        .sum();
    alternatives.saturating_mul(alternatives).max(64)
}

/// Compute left-corner strongly-connected components (size > 1) over the parser
/// rules of one unit, each returned as an ascending list of `RuleId`. Single
/// directly-left-recursive rules are excluded — those belong to
/// [`super::left_recursion`] and must not be touched here.
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

/// Collect every rule reachable in left-corner position from `alternative`
/// (through leading epsilon/nullable/optional elements), so the SCC graph
/// captures the full left-corner relation the ATN detector uses.
fn collect_left_corner_calls(
    alternative: &Alternative,
    grammar: Grammar<'_>,
    result: &mut BTreeSet<RuleId>,
) {
    for element in &alternative.elements {
        match &element.kind {
            ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon => {}
            ElementKind::RuleCall(call) => {
                let Some(target) = grammar.target(call) else {
                    return;
                };
                result.insert(target);
                let skippable = matches!(
                    element.quantifier,
                    Quantifier::Optional { .. } | Quantifier::ZeroOrMore { .. }
                ) || grammar.nullable.contains(&target);
                if !skippable {
                    return;
                }
            }
            ElementKind::Block(block) => {
                for nested in &block.alternatives {
                    collect_left_corner_calls(nested, grammar, result);
                }
                let skippable = matches!(
                    element.quantifier,
                    Quantifier::Optional { .. } | Quantifier::ZeroOrMore { .. }
                ) || block_is_nullable(block, grammar.names, grammar.nullable);
                if !skippable {
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

fn rules_by_id(unit: &GrammarUnit) -> BTreeMap<RuleId, &Rule> {
    unit.rules.iter().map(|rule| (rule.id, rule)).collect()
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
    let rules = rules_by_id(unit);
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

/// Assign fresh IDs to a spliced element list, recursing through nested blocks,
/// labels, actions and predicates so no ID is shared between two live nodes.
fn renumber_elements(
    elements: Vec<Element>,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> Vec<Element> {
    elements
        .into_iter()
        .map(|element| renumber_element(element, ids, provenance))
        .collect()
}

fn renumber_element(
    mut element: Element,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> Element {
    let original = element.id;
    element.id = ids.element();
    record_clone(
        provenance,
        ModelNodeId::Element(element.id),
        ModelNodeId::Element(original),
    );

    if let Some(label) = element.label.as_mut() {
        let previous = label.id;
        label.id = ids.label();
        record_clone(
            provenance,
            ModelNodeId::Label(label.id),
            ModelNodeId::Label(previous),
        );
    }

    element.kind = match element.kind {
        ElementKind::Block(block) => ElementKind::Block(Block {
            alternatives: block
                .alternatives
                .into_iter()
                .map(|mut alternative| {
                    let previous = alternative.id;
                    alternative.id = ids.alternative();
                    record_clone(
                        provenance,
                        ModelNodeId::Alternative(alternative.id),
                        ModelNodeId::Alternative(previous),
                    );
                    alternative.elements = renumber_elements(alternative.elements, ids, provenance);
                    alternative
                })
                .collect(),
            options: block.options,
            syntax: block.syntax,
            span: block.span,
        }),
        ElementKind::Action { id, body } => {
            let fresh = ids.action();
            record_clone(
                provenance,
                ModelNodeId::Action(fresh),
                ModelNodeId::Action(id),
            );
            ElementKind::Action { id: fresh, body }
        }
        ElementKind::Predicate {
            id,
            body,
            fail,
            precedence,
        } => {
            let fresh = ids.predicate();
            record_clone(
                provenance,
                ModelNodeId::Predicate(fresh),
                ModelNodeId::Predicate(id),
            );
            ElementKind::Predicate {
                id: fresh,
                body,
                fail,
                precedence,
            }
        }
        kind => kind,
    };
    element
}

fn record_clone(provenance: &mut ProvenanceIndex, fresh: ModelNodeId, original: ModelNodeId) {
    let mut origins = provenance.origins(original).to_vec();
    origins.push(Origin::Synthetic {
        reason: SyntheticReason::BlockBoundary,
        owner: original,
    });
    provenance.record_model(fresh, origins);
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
mod tests {
    use super::*;
    use crate::grammar::frontend::{SourceId, parse_source};
    use crate::grammar::left_recursion::rewrite_immediate_left_recursion;
    use crate::grammar::model::{GrammarId, Terminal};
    use crate::grammar::syntax::parse_grammar_unit;

    struct Fixture {
        unit: GrammarUnit,
        ids: ModelIdAllocator,
        provenance: ProvenanceIndex,
    }

    fn parse(text: &str) -> Fixture {
        let file = parse_source(SourceId::new(0), "P.g4", text).expect("valid grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);
        Fixture {
            unit,
            ids,
            provenance,
        }
    }

    /// Run the pass, returning the rendered model *before* and *after* plus
    /// whether it reported a change. Rendering before invoking the pass is what
    /// makes the "model untouched" assertions meaningful.
    fn run(text: &str) -> (String, String, bool) {
        let mut fixture = parse(text);
        let before = render(&fixture.unit);
        let changed = eliminate_mutual_left_recursion(
            std::slice::from_mut(&mut fixture.unit),
            &mut fixture.ids,
            &mut fixture.provenance,
        );
        let after = render(&fixture.unit);
        (before, after, changed)
    }

    fn rewritten(text: &str) -> GrammarUnit {
        let mut fixture = parse(text);
        assert!(
            eliminate_mutual_left_recursion(
                std::slice::from_mut(&mut fixture.unit),
                &mut fixture.ids,
                &mut fixture.provenance,
            ),
            "expected the cycle to be rewritten"
        );
        fixture.unit
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
                if let Some(assoc) = alternative
                    .options
                    .iter()
                    .find(|option| option.name.value == "assoc")
                {
                    use std::fmt::Write as _;
                    let _ = write!(out, "<assoc={}> ", assoc.value.value);
                }
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
        let quantifier = match element.quantifier {
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
        let label = element
            .label
            .as_ref()
            .map(|label| format!("{}=", label.name))
            .unwrap_or_default();
        format!("{label}{body}{quantifier}")
    }

    fn rule<'a>(unit: &'a GrammarUnit, name: &str) -> &'a Rule {
        unit.rules
            .iter()
            .find(|rule| rule.name == name)
            .unwrap_or_else(|| panic!("rule {name} exists"))
    }

    /// Assert the pass declined: it reported no change *and* left the model
    /// byte-identical to the pre-pass rendering.
    fn assert_declined(text: &str) {
        let (before, after, changed) = run(text);
        assert!(
            !changed,
            "expected a decline, but the pass reported a change"
        );
        assert_eq!(
            before, after,
            "a declined cycle must leave the model untouched"
        );
    }

    #[test]
    fn collapses_two_rule_name_cycle_into_the_hub() {
        let unit = rewritten(
            "parser grammar P; \
             name : qualified_name | simple_name ; \
             qualified_name : name '.' simple_name ; \
             simple_name : ID ;",
        );
        assert!(
            unit.rules.iter().all(|rule| rule.name != "qualified_name"),
            "hub-only satellite is removed"
        );
        insta::assert_snapshot!("name_cycle_collapsed", render(&unit));
    }

    #[test]
    fn collapsed_hub_is_then_rewritten_by_the_direct_pass() {
        let mut fixture = parse(
            "parser grammar P; \
             name : qualified_name | simple_name ; \
             qualified_name : name '.' simple_name ; \
             simple_name : ID ;",
        );
        assert!(eliminate_mutual_left_recursion(
            std::slice::from_mut(&mut fixture.unit),
            &mut fixture.ids,
            &mut fixture.provenance,
        ));
        let diagnostics = rewrite_immediate_left_recursion(
            std::slice::from_mut(&mut fixture.unit),
            &mut fixture.ids,
            &mut fixture.provenance,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(
            rule(&fixture.unit, "name").left_recursion.is_some(),
            "collapsed hub is now a direct-left-recursion precedence rule"
        );
    }

    #[test]
    fn splits_optional_from_inlined_satellite() {
        // C#'s range-operator shape: the leading-optional recursion lives in the
        // satellite body (`r : e? '..' e?`). After inlining `r` into `e`, the
        // leading `e?` splits into `e '..' e?` (left-recursive) and `'..' e?`
        // (primary), yielding a well-formed direct-recursion hub.
        let unit = rewritten(
            "parser grammar P; \
             e : e '+' e | r | ID ; \
             r : e? '..' e? ;",
        );
        insta::assert_snapshot!("optional_from_satellite", render(&unit));
    }

    #[test]
    fn expands_consecutive_leading_optionals_to_a_fixpoint() {
        // Two leading optional corners in one alternative: splitting once would
        // leave the absent branch still starting with an optional corner, which
        // substitution would then treat as mandatory.
        let unit = rewritten(
            "parser grammar P; \
             e : e '+' e | r | ID ; \
             r : e? e? '..' ;",
        );
        insta::assert_snapshot!("consecutive_optionals", render(&unit));
    }

    #[test]
    fn range_operator_hub_is_accepted_by_the_direct_pass() {
        let mut fixture = parse(
            "parser grammar P; \
             e : e '+' e | r | ID ; \
             r : e? '..' e? ;",
        );
        assert!(eliminate_mutual_left_recursion(
            std::slice::from_mut(&mut fixture.unit),
            &mut fixture.ids,
            &mut fixture.provenance,
        ));
        let diagnostics = rewrite_immediate_left_recursion(
            std::slice::from_mut(&mut fixture.unit),
            &mut fixture.ids,
            &mut fixture.provenance,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(rule(&fixture.unit, "e").left_recursion.is_some());
    }

    #[test]
    fn retains_satellite_referenced_from_outside_the_cycle() {
        // array_type-style: the satellite is left-recursive through the hub but
        // also called by a non-cycle rule, so it is kept (not removed).
        let unit = rewritten(
            "parser grammar P; \
             t : arr | t '?' | ID ; \
             arr : t '[' ']' ; \
             new_arr : 'new' arr ;",
        );
        assert!(
            unit.rules.iter().any(|rule| rule.name == "arr"),
            "externally-referenced satellite is retained"
        );
        insta::assert_snapshot!("external_satellite_retained", render(&unit));
    }

    #[test]
    fn preserves_satellite_alternative_associativity() {
        // `<assoc=right>` is declared on the satellite's alternative and drives
        // the direct rewriter's associativity, so the spliced alternative must
        // inherit the satellite's options, not the hub call site's.
        let unit = rewritten(
            "parser grammar P; \
             expr : power | ID ; \
             power : <assoc=right> expr '^' expr ;",
        );
        insta::assert_snapshot!("assoc_right_preserved", render(&unit));
    }

    #[test]
    fn preserves_declared_alternative_order() {
        // Alternative order is precedence. A satellite spliced from the *first*
        // hub alternative must land first, ahead of the surviving originals.
        let unit = rewritten(
            "parser grammar P; \
             e : s | e '+' e | ID ; \
             s : e '*' e ;",
        );
        insta::assert_snapshot!("declared_order_preserved", render(&unit));
    }

    #[test]
    fn ignores_grammar_without_mutual_recursion() {
        let (_, _, changed) = run("parser grammar P; \
             e : e '+' t | t ; \
             t : ID ;");
        assert!(
            !changed,
            "direct-only left recursion is left for the direct pass"
        );
    }

    #[test]
    fn declines_cycle_without_a_token_consuming_operator() {
        // a:b; b:c; c:a|X reduces to `a : a | X` — a bare self-loop the direct
        // rewriter rejects. Declining leaves G4A005 to report it downstream.
        assert_declined("parser grammar P; a : b ; b : c ; c : a | X ;");
    }

    #[test]
    fn declines_argument_bearing_recursion() {
        assert_declined(
            "parser grammar P; \
             e : s | ID ; \
             s : e '+' e[3] ;",
        );
    }

    #[test]
    fn declines_argument_bearing_satellite_call() {
        // The corner itself carries arguments: removing it would drop them, and
        // the satellite's parameter scope cannot travel with the body.
        assert_declined(
            "parser grammar P; \
             e : s[3] | ID ; \
             s[int x] : e '+' ID ;",
        );
    }

    #[test]
    fn declines_quantified_corner() {
        // `b*` is not one satellite occurrence: splicing a single body in its
        // place would silently drop the closure.
        assert_declined(
            "parser grammar P; \
             a : b* 'x' | 'a' ; \
             b : a 'b' ;",
        );
    }

    #[test]
    fn declines_corner_behind_a_nullable_prefix() {
        // `n` is nullable, so the real left corner is ambiguous between `n` and
        // `b`; substituting either is a guess.
        assert_declined(
            "parser grammar P; \
             a : n b | 'a' ; \
             b : a 'b' ; \
             n : ;",
        );
    }

    #[test]
    fn declines_labelled_corner() {
        // Removing a labelled corner leaves `$x` dangling in caller actions.
        assert_declined(
            "parser grammar P; \
             e : x=s | ID ; \
             s : e '+' ID ;",
        );
    }

    #[test]
    fn declines_satellite_with_rule_level_action() {
        // `@init` attaches to the rule; inlining the body would discard it.
        assert_declined(
            "parser grammar P; \
             e : s | ID ; \
             s @init { let _x = 1; } : e '+' ID ;",
        );
    }

    #[test]
    fn declines_satellite_with_labelled_alternatives() {
        // `#`-labels generate context types keyed by the satellite rule; the hub
        // cannot host them without colliding with its own labelling scheme.
        assert_declined(
            "parser grammar P; \
             e : s | ID ; \
             s : e '+' ID # Add ;",
        );
    }

    #[test]
    fn declines_when_caller_and_satellite_labels_collide() {
        // Both sides bind `x`; merging the element lists would rebind the
        // caller's action to the satellite's element.
        assert_declined(
            "parser grammar P; \
             e : s x=ID | ID ; \
             s : e '+' x=ID ;",
        );
    }

    #[test]
    fn leaves_lexer_grammars_untouched() {
        // Precedence rewriting is a parser-rule construct. Routing a lexer SCC
        // through it produced an "unsupported embedded lexer action" naming an
        // action the grammar never declared. Left-recursive lexer rules are
        // invalid in ANTLR regardless (error(119)); diagnosing them properly is
        // tracked separately as issue #236.
        let mut fixture = parse(
            "lexer grammar L; \
             A : B 'a' | 'x' ; \
             B : A 'b' ;",
        );
        let before = render(&fixture.unit);
        let changed = eliminate_mutual_left_recursion(
            std::slice::from_mut(&mut fixture.unit),
            &mut fixture.ids,
            &mut fixture.provenance,
        );
        assert!(!changed, "lexer grammars must not be rewritten");
        assert_eq!(before, render(&fixture.unit));
    }

    #[test]
    fn declining_consumes_no_ids_and_writes_no_provenance() {
        // The safety claim is that a decline is invisible downstream, so the
        // allocator and provenance index must be untouched too.
        let mut fixture = parse("parser grammar P; a : b ; b : c ; c : a | X ;");
        let ids_before = format!("{:?}", fixture.ids);
        let provenance_before = format!("{:?}", fixture.provenance);
        assert!(!eliminate_mutual_left_recursion(
            std::slice::from_mut(&mut fixture.unit),
            &mut fixture.ids,
            &mut fixture.provenance,
        ));
        assert_eq!(
            ids_before,
            format!("{:?}", fixture.ids),
            "a declined cycle must not consume model IDs"
        );
        assert_eq!(
            provenance_before,
            format!("{:?}", fixture.provenance),
            "a declined cycle must not record provenance"
        );
    }
}
