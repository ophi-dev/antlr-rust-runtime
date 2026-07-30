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

use super::action::{ActionReferenceKind, action_references};
use super::model::{
    Alternative, AlternativeId, Block, Element, ElementKind, GrammarKind, GrammarUnit,
    ModelIdAllocator, ModelNodeId, OptionDecl, Quantifier, Rule, RuleCall, RuleId, RuleKind,
    Terminal,
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
    preserved_rules: &BTreeSet<RuleId>,
) -> bool {
    let mut changed = false;
    for unit in units.iter_mut() {
        // Precedence rewriting is a parser-rule construct: a lexer rule cycle
        // must not be routed through it (the injected precedence predicates
        // become "unsupported embedded lexer action" much later, far from the
        // cause).
        if unit.kind == GrammarKind::Parser {
            changed |= eliminate_in_unit(unit, ids, provenance, preserved_rules);
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
    preserved_rules: &BTreeSet<RuleId>,
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
            .find_map(|cycle| plan_cycle(unit, cycle, grammar, preserved_rules))
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
    /// The original hub alternative this one descends from. Its `#label` and
    /// commands are authored API naming this alternative *position* in the hub,
    /// so the result inherits them — dropping a caller's `#ViaSatellite` would
    /// silently delete its generated context and listener callbacks.
    label_from: AlternativeId,
    /// Alternative options accumulated along the splice chain: the hub
    /// alternative's own options unioned with those of every satellite
    /// alternative spliced into this position. `<assoc=right>` rides on the
    /// alternative that declares the operator, which may sit behind an
    /// alias-shaped satellite (`a : <assoc=right> b '^' e ; b : e ;`), so no
    /// single source alternative can stand in for the set. Conflicting
    /// declarations decline the cycle in [`splice_satellite`], keeping the
    /// union unambiguous by construction.
    options: Vec<OptionDecl>,
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
/// * `UnparameterizedHub` — the hub declares no arguments: every in-cycle
///   corner is bare by `BareCorner`, so it necessarily omits a parameterized
///   hub's required arguments, and the splice would launder that
///   missing-argument error into silently default-initialized attributes.
/// * `BareCorner` — every corner we substitute is an unquantified, unlabelled,
///   argument-free call, preceded only by actions/predicates/epsilon. This is
///   the single notion of "the left corner": one index, computed once, used for
///   both the decision and the splice.
/// * `InlinableSatellite` — satellites carry no rule-level attributes that
///   inlining would drop, no alternative labels that would collide, and no
///   embedded actions or predicates at all: semantic bodies are owned by their
///   rule and alternative (`$ctx` means the satellite's context; `$`-references
///   resolve against the enclosing alternative), and that ownership does not
///   survive transplantation into the hub.
/// * `NoRebinding` — merging the caller's and satellite's element lists must
///   not rebind anything: explicit label scopes must not overlap, and no
///   action on either side may reference a token, rule or label name the other
///   side introduces (`$ID` binds by occurrence within its alternative).
/// * `DirectlyRewritable` — the resulting hub is Primary/Prefix/Binary/Suffix
///   throughout, as [`super::left_recursion`] requires, and every recursive
///   alternative's tail can consume input (a nullable tail is a
///   left-recursive alternative followed by the empty string, which the
///   direct rewriter rejects).
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
                return if element.quantifier == Quantifier::One && bare_reference(element, call) {
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

/// The single definition of a "bare" rule reference — the only kind either
/// corner derivation may substitute or split. Shared by [`classify_corner`] and
/// [`planned_corner`] so the two cannot drift apart: a label would dangle in
/// actions once the element is gone, arguments have nowhere to go once the
/// callee is inlined, and element options would be silently discarded.
const fn bare_reference(element: &Element, call: &RuleCall) -> bool {
    element.label.is_none() && call.arguments.is_none() && element.options.is_empty()
}

/// Decide the whole rewrite for one cycle, or decline. Mutates nothing.
fn plan_cycle(
    unit: &GrammarUnit,
    cycle: &Cycle,
    grammar: Grammar<'_>,
    preserved_rules: &BTreeSet<RuleId>,
) -> Option<CyclePlan> {
    let rules = rules_by_id(unit);
    if cycle.iter().any(|member| !rules.contains_key(member)) {
        return None;
    }
    let cycle_set: BTreeSet<RuleId> = cycle.iter().copied().collect();
    let hub_id = choose_hub(unit, cycle, &cycle_set, grammar, preserved_rules)?;
    // Note: a *nullable* hub is fine — Roslyn's `pattern` is one
    // (`recursive_pattern` is all-optional) and ANTLR accepts the collapsed
    // grammar; the ill-founded shapes nullability could smuggle in (an
    // epsilon-only alternative, a token-free self-loop) are declined by
    // `planned_hub_is_directly_rewritable` on the planned result instead.
    //
    // Requirements::UnparameterizedHub — semantic call validation runs after
    // this pass, so rewriting a parameterized hub would delete the very
    // argument-less corner calls that validation should reject.
    if rules[&hub_id].arguments.is_some() {
        return None;
    }

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
            label_from: alternative.id,
            options: alternative.options.clone(),
            origin: alternative.id,
            elements: alternative.elements.clone(),
            verbatim: true,
        })
        .collect();

    let budget = substitution_budget(&cycle_set, &rules);
    let mut steps: usize = 0;
    while let Some(position) = planned.iter().position(|candidate| {
        !matches!(
            planned_corner(candidate, hub_id, &cycle_set, grammar),
            PlannedCorner::Settled
        )
    }) {
        steps += 1;
        // Requirements::Converges
        if steps > budget {
            return None;
        }
        let replacement = match planned_corner(&planned[position], hub_id, &cycle_set, grammar) {
            PlannedCorner::Optional { index, target } => {
                // The absent branch deletes the element; any surviving action
                // that names the corner's rule (`$e.text`) would dangle.
                if remaining_actions_reference(
                    &planned[position].elements,
                    index,
                    &rules[&target].name,
                ) {
                    return None;
                }
                split_optional(&planned[position], index)
            }
            PlannedCorner::Satellite { index, target } => {
                splice_satellite(&planned[position], index, rules[&target])?
            }
            // The corner enters the cycle but cannot be substituted or split
            // (quantified, labelled, argument- or option-bearing): the cycle
            // is out of the tractable subclass.
            PlannedCorner::Blocked => return None,
            PlannedCorner::Settled => unreachable!("position was found to need work"),
        };
        // Splice in place: the expansions occupy the slot of the alternative
        // they came from, so declared alternative order — and therefore
        // precedence — is preserved.
        planned.splice(position..=position, replacement);
    }

    // A plan that did no work would be reapplied verbatim on every iteration of
    // the driver loop — the cycle lives somewhere this pass cannot reach (for
    // example inside a nested block), so decline it.
    if steps == 0 {
        return None;
    }

    // Requirements::DirectlyRewritable
    if !planned_hub_is_directly_rewritable(&planned, hub_id, &cycle_set, grammar) {
        return None;
    }

    let removable = removable_satellites(
        unit,
        cycle,
        hub_id,
        &planned,
        grammar.names,
        preserved_rules,
    );
    Some(CyclePlan {
        hub: hub_id,
        alternatives: planned,
        removable,
    })
}

/// What still needs doing to a planned alternative before the hub is direct.
enum PlannedCorner {
    /// A leading optional call to cycle member `target` at `index`, to be split.
    Optional { index: usize, target: RuleId },
    /// A bare call to satellite `target` at `index`, to be spliced.
    Satellite { index: usize, target: RuleId },
    /// The corner enters the cycle but is not substitutable: quantified with
    /// `*`/`+`, nongreedy-optional, labelled, argument- or option-bearing.
    Blocked,
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
                // A leading greedy `X?` where X is in the cycle: split it into
                // present and absent branches (union-preserving) so the present
                // branch becomes a well-formed recursive corner. This applies to
                // the hub itself — C#'s `expr? '..' expr?` is exactly that
                // shape — so it is checked before the self-reference test below.
                // A nongreedy `X??` prefers the absent branch, which the split
                // order would invert, so it is Blocked instead; a labelled or
                // option-bearing optional would lose those in the absent branch.
                if matches!(element.quantifier, Quantifier::Optional { greedy: true })
                    && bare_reference(element, call)
                {
                    return PlannedCorner::Optional { index, target };
                }
                // A plain hub self-reference is the goal state, not something
                // to substitute: recursing on it would never terminate. The
                // direct rewriter keys on the call itself (labels included, via
                // its deleted-label machinery), so mirror that.
                if target == hub_id && element.quantifier == Quantifier::One {
                    return PlannedCorner::Settled;
                }
                if element.quantifier == Quantifier::One && bare_reference(element, call) {
                    return PlannedCorner::Satellite { index, target };
                }
                return PlannedCorner::Blocked;
            }
            _ => return PlannedCorner::Settled,
        }
    }
    PlannedCorner::Settled
}

/// Whether any action or predicate among `elements` (other than the corner at
/// `skip` itself, and descending into nested blocks) references `rule_name` —
/// e.g. `$s.text` after the `s` element has been spliced away.
fn remaining_actions_reference(elements: &[Element], skip: usize, rule_name: &str) -> bool {
    elements.iter().enumerate().any(|(index, element)| {
        if index == skip {
            return false;
        }
        element_actions_reference(element, rule_name)
    })
}

fn element_actions_reference(element: &Element, rule_name: &str) -> bool {
    let mut bodies: Vec<&str> = Vec::new();
    match &element.kind {
        ElementKind::Action { body, .. } => bodies.push(body),
        ElementKind::Predicate { body, fail, .. } => {
            bodies.push(body);
            if let Some(fail) = fail.as_deref() {
                bodies.push(fail);
            }
        }
        ElementKind::Block(block) => {
            return block.alternatives.iter().any(|alternative| {
                alternative
                    .elements
                    .iter()
                    .any(|nested| element_actions_reference(nested, rule_name))
            });
        }
        _ => {}
    }
    bodies.into_iter().any(|body| {
        action_references(body)
            .iter()
            .any(|reference| match reference.kind {
                ActionReferenceKind::Attribute { name, .. } => name == rule_name,
                ActionReferenceKind::Qualified { name, .. } => name == rule_name,
                ActionReferenceKind::NonLocal { rule, .. } => rule == rule_name,
            })
    })
}

/// `α X? β` becomes `α X β | α β`, preserving order (present branch first, as
/// the authored greedy `?` prefers matching). Both products keep the caller's
/// `#label` — ANTLR permits the same label on multiple alternatives (they share
/// one context class), which is the faithful reading of a split.
fn split_optional(candidate: &PlannedAlternative, index: usize) -> Vec<PlannedAlternative> {
    let mut present = candidate.elements.clone();
    if let Some(element) = present.get_mut(index) {
        element.quantifier = Quantifier::One;
    }
    let mut absent = candidate.elements.clone();
    absent.remove(index);
    vec![
        PlannedAlternative {
            label_from: candidate.label_from,
            options: candidate.options.clone(),
            origin: candidate.origin,
            elements: present,
            verbatim: false,
        },
        PlannedAlternative {
            label_from: candidate.label_from,
            options: candidate.options.clone(),
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
    // Deleting the corner element severs any `$satellite.attr` reference an
    // action in the surviving prefix/suffix makes by rule name.
    if remaining_actions_reference(&candidate.elements, index, &satellite.name) {
        return None;
    }
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
        // Requirements::NoRebinding — implicit references bind by occurrence
        // within the alternative: a caller action's `$ID` means the caller's
        // own `ID` occurrence, and a satellite body arriving in front of it
        // with another `ID` would capture the reference (and vice versa).
        // Decline when either side's actions name anything the other side
        // introduces.
        if implicit_bindings_collide(prefix, suffix, &source.elements) {
            return None;
        }
        // The options of every alternative merged into this position apply to
        // the flattened result: an `<assoc=right>` declared on an operator
        // alternative must survive a later alias splice (`b : e`). Two
        // alternatives declaring the same option with different values is
        // genuinely ambiguous, so decline rather than pick one.
        let mut options = candidate.options.clone();
        for option in &source.options {
            match options
                .iter()
                .find(|existing| existing.name.value == option.name.value)
            {
                Some(existing) if existing.value.value != option.value.value => return None,
                Some(_) => {}
                None => options.push(option.clone()),
            }
        }
        let mut elements = Vec::with_capacity(prefix.len() + source.elements.len() + suffix.len());
        elements.extend(prefix.iter().cloned());
        elements.extend(source.elements.iter().cloned());
        elements.extend(suffix.iter().cloned());
        expansions.push(PlannedAlternative {
            // The caller's `#label` names this hub position and survives.
            label_from: candidate.label_from,
            options,
            origin: source.id,
            elements,
            verbatim: false,
        });
    }
    Some(expansions)
}

fn labels_collide(prefix: &[Element], suffix: &[Element], spliced: &[Element]) -> bool {
    let mut caller = BTreeSet::new();
    collect_labels(prefix, &mut caller);
    collect_labels(suffix, &mut caller);
    let mut satellite = BTreeSet::new();
    collect_labels(spliced, &mut satellite);
    !caller.is_disjoint(&satellite)
}

/// Every label name bound in `elements`, descending into nested blocks —
/// separately-valid label scopes become one scope after a splice, so collisions
/// anywhere in either tree matter.
fn collect_labels<'a>(elements: &'a [Element], out: &mut BTreeSet<&'a str>) {
    for element in elements {
        if let Some(label) = &element.label {
            out.insert(label.name.as_str());
        }
        if let ElementKind::Block(nested) = &element.kind {
            for alternative in &nested.alternatives {
                collect_labels(&alternative.elements, out);
            }
        }
    }
}

/// Whether merging the caller's surviving elements with a satellite
/// alternative would capture an *implicit* action reference: an action on one
/// side names a token, rule or label that the other side introduces as an
/// element. Only names actually referenced by an action matter — inert
/// duplicate occurrences (`e : s s`) are fine.
fn implicit_bindings_collide(prefix: &[Element], suffix: &[Element], spliced: &[Element]) -> bool {
    let mut caller_refs = BTreeSet::new();
    action_reference_names(prefix, &mut caller_refs);
    action_reference_names(suffix, &mut caller_refs);
    let mut satellite_intro = BTreeSet::new();
    bindable_names(spliced, &mut satellite_intro);
    if !caller_refs.is_disjoint(&satellite_intro) {
        return true;
    }
    let mut satellite_refs = BTreeSet::new();
    action_reference_names(spliced, &mut satellite_refs);
    let mut caller_intro = BTreeSet::new();
    bindable_names(prefix, &mut caller_intro);
    bindable_names(suffix, &mut caller_intro);
    !satellite_refs.is_disjoint(&caller_intro)
}

/// Every simple `$name` / `$name.attr` reference made by actions and
/// predicates among `elements`, descending nested blocks.
fn action_reference_names<'a>(elements: &'a [Element], out: &mut BTreeSet<&'a str>) {
    for element in elements {
        let mut bodies: Vec<&str> = Vec::new();
        match &element.kind {
            ElementKind::Action { body, .. } => bodies.push(body),
            ElementKind::Predicate { body, fail, .. } => {
                bodies.push(body);
                if let Some(fail) = fail.as_deref() {
                    bodies.push(fail);
                }
            }
            ElementKind::Block(nested) => {
                for alternative in &nested.alternatives {
                    action_reference_names(&alternative.elements, out);
                }
            }
            _ => {}
        }
        for body in bodies {
            for reference in action_references(body) {
                match reference.kind {
                    ActionReferenceKind::Attribute { name, .. }
                    | ActionReferenceKind::Qualified { name, .. } => {
                        out.insert(name);
                    }
                    ActionReferenceKind::NonLocal { .. } => {}
                }
            }
        }
    }
}

/// Every name an action can bind by occurrence within an alternative: element
/// labels, token names, and rule-call names, descending nested blocks.
fn bindable_names<'a>(elements: &'a [Element], out: &mut BTreeSet<&'a str>) {
    for element in elements {
        if let Some(label) = &element.label {
            out.insert(label.name.as_str());
        }
        match &element.kind {
            ElementKind::RuleCall(call) => {
                out.insert(call.name.as_str());
            }
            ElementKind::Terminal(Terminal::Token(token)) => {
                out.insert(token.as_str());
            }
            ElementKind::Block(nested) => {
                for alternative in &nested.alternatives {
                    bindable_names(&alternative.elements, out);
                }
            }
            _ => {}
        }
    }
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
        && !satellite_has_embedded_semantics(satellite)
}

/// Whether the satellite carries any embedded action or predicate at all.
/// Semantic bodies are *owned* by their rule and alternative: `$ctx` (and the
/// semantic-context parameter itself) means the satellite's context, and the
/// embedded-action pipeline resolves `$`-references against the enclosing
/// alternative's source span — none of which survives transplantation into the
/// hub. Rather than pattern-match the body for the references that happen to
/// break (a target-language-specific and inherently incomplete test), any
/// satellite with inline semantics declines.
fn satellite_has_embedded_semantics(satellite: &Rule) -> bool {
    fn elements_have_semantics(elements: &[Element]) -> bool {
        elements.iter().any(|element| match &element.kind {
            ElementKind::Action { .. } | ElementKind::Predicate { .. } => true,
            ElementKind::Block(nested) => nested
                .alternatives
                .iter()
                .any(|alternative| elements_have_semantics(&alternative.elements)),
            _ => false,
        })
    }
    satellite
        .block
        .alternatives
        .iter()
        .any(|alternative| elements_have_semantics(&alternative.elements))
}

/// Whether the planned hub is a shape [`super::left_recursion`] accepts.
///
/// The downstream classifier keys recursion on the **literal first element**
/// (`classify_rule` uses `.first()`), so this gate mirrors that exactly rather
/// than skipping leading actions: an alternative like `{pred} hub '+' ID` is
/// *not* recognisably recursive downstream, would land in the primary block
/// still left-recursive, and must therefore decline here — which the
/// corner-closure check below does uniformly for every non-recursive
/// alternative, covering epsilon prefixes, nested blocks and any leftover
/// cycle-member corner in one place.
fn planned_hub_is_directly_rewritable(
    planned: &[PlannedAlternative],
    hub_id: RuleId,
    cycle_set: &BTreeSet<RuleId>,
    grammar: Grammar<'_>,
) -> bool {
    let mut has_primary = false;
    let mut has_recursive = false;
    for candidate in planned {
        let elements = &candidate.elements;
        // No argument-bearing self-reference anywhere (mirrors G4R001).
        if elements.iter().any(|element| {
            hub_call(element, hub_id, grammar).is_some_and(|call| call.arguments.is_some())
        }) {
            return false;
        }
        let Some(last_significant) = elements.iter().rposition(|e| !is_epsilon_only(e)) else {
            // An epsilon-only alternative would make the precedence hub
            // nullable; the original hub was not.
            return false;
        };
        if elements
            .first()
            .is_some_and(|element| is_hub_call(element, hub_id, grammar))
        {
            // Recursive (Binary/Suffix) form. A bare `hub` with nothing
            // significant after it is a nonconforming self-loop, exactly as
            // the direct rewriter treats it.
            if last_significant == 0 {
                return false;
            }
            // The recursive remainder must consume input: a nullable tail
            // (`e : e n | ID` with `n : ;`) would commit a rewrite the direct
            // pass then rejects ("can be followed by the empty string"),
            // reporting against the transformed rule instead of the authored
            // cycle.
            if elements[1..]
                .iter()
                .all(|element| element_nullable(element, grammar.names, grammar.nullable))
            {
                return false;
            }
            has_recursive = true;
        } else {
            // Primary/Prefix bucket. Its left-corner closure must not re-enter
            // the cycle: the downstream rewriter would file it as a primary
            // alternative and the committed hub would still be left-recursive,
            // failing later with a diagnostic naming the wrong rule set.
            let mut corners = BTreeSet::new();
            collect_left_corner_calls(elements, grammar, &mut corners);
            if corners.iter().any(|corner| cycle_set.contains(corner)) {
                return false;
            }
            has_primary = true;
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
        .map(|planned| {
            // The caller's alternative supplies the `#label`, commands and
            // position identity; the options were accumulated across the whole
            // splice chain by `plan_cycle` (`<assoc=right>` describes the
            // operator that was inlined, wherever in the chain it was
            // declared).
            let label_source = attributes
                .get(&planned.label_from)
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
            Alternative {
                id,
                elements,
                label: label_source.label.clone(),
                options: planned.options.clone(),
                commands: label_source.commands.clone(),
                syntax: label_source.syntax,
                span: label_source.span.clone(),
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
    preserved_rules: &BTreeSet<RuleId>,
) -> Option<RuleId> {
    let mut external = externally_referenced(unit, cycle_set, grammar.names);
    external.extend(preserved_rules.iter().copied());
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
/// references them. The hub's contribution is taken from the **planned**
/// alternatives, not its current body — verbatim alternatives and spliced
/// suffixes can keep satellite calls alive (`t : arr | t '?' arr`,
/// `e : s s | ID`), and deleting such a satellite would leave a dangling rule
/// reference. Computed to a fixpoint so a retained satellite pulls its own
/// dependencies back in.
fn removable_satellites(
    unit: &GrammarUnit,
    cycle: &Cycle,
    hub_id: RuleId,
    planned: &[PlannedAlternative],
    names: &BTreeMap<String, RuleId>,
    preserved_rules: &BTreeSet<RuleId>,
) -> BTreeSet<RuleId> {
    let mut removable = cycle
        .iter()
        .copied()
        .filter(|member| *member != hub_id && !preserved_rules.contains(member))
        .collect::<BTreeSet<_>>();
    loop {
        let mut referenced = BTreeSet::new();
        {
            let mut sink = |target: RuleId| {
                if removable.contains(&target) {
                    referenced.insert(target);
                }
            };
            for candidate in planned {
                collect_calls_in_elements(&candidate.elements, names, &mut sink);
            }
            for rule in &unit.rules {
                if removable.contains(&rule.id) || rule.id == hub_id {
                    continue;
                }
                collect_calls_into(&rule.block, names, &mut sink);
            }
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
            collect_left_corner_calls(&alternative.elements, grammar, &mut corners);
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

/// Collect every rule reachable in left-corner position from `elements`
/// (through leading epsilon/nullable/optional elements), so the SCC graph
/// captures the full left-corner relation the ATN detector uses.
fn collect_left_corner_calls(
    elements: &[Element],
    grammar: Grammar<'_>,
    result: &mut BTreeSet<RuleId>,
) {
    for element in elements {
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
                    collect_left_corner_calls(&nested.elements, grammar, result);
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

pub(crate) fn collect_calls_into(
    block: &Block,
    names: &BTreeMap<String, RuleId>,
    sink: &mut impl FnMut(RuleId),
) {
    for alternative in &block.alternatives {
        collect_calls_in_elements(&alternative.elements, names, sink);
    }
}

pub(crate) fn collect_calls_in_elements(
    elements: &[Element],
    names: &BTreeMap<String, RuleId>,
    sink: &mut impl FnMut(RuleId),
) {
    for element in elements {
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
            &BTreeSet::new(),
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
                &BTreeSet::new(),
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
                if let Some(label) = &alternative.label {
                    use std::fmt::Write as _;
                    let _ = write!(out, " #{}", label.value);
                }
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
            &BTreeSet::new(),
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
            &BTreeSet::new(),
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
    fn chained_alias_splice_carries_operator_options() {
        // `<assoc=right>` is declared on `a`'s operator alternative, but the
        // final splice in the chain is the alias `b : e`. Options accumulate
        // across the whole chain, so the option must survive to the collapsed
        // alternative — a later alias splice must not overwrite it.
        let unit = rewritten(
            "parser grammar P; \
             e : a | ID ; \
             a : <assoc=right> b '^' e ; \
             b : e ;",
        );
        insta::assert_snapshot!("chained_assoc_carried", render(&unit));
    }

    #[test]
    fn declines_conflicting_options_along_a_splice_chain() {
        // Two alternatives in one chain declare the same option with different
        // values; flattening them into one alternative would have to pick a
        // winner, so the cycle declines instead.
        assert_declined(
            "parser grammar P; \
             e : a | ID ; \
             a : <assoc=right> b '^' e ; \
             b : <assoc=left> e ;",
        );
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
    fn terminates_and_declines_when_no_corner_is_reducible() {
        // The only cycle-entering corner is a block, which is never spliced. A
        // plan that makes no step must decline rather than spin: this test
        // hangs (and times out) if the zero-step guard regresses.
        assert_declined(
            "parser grammar P; \
             e : (s | ID) | e '+' e ; \
             s : e '*' e ;",
        );
    }

    #[test]
    fn retains_satellite_still_referenced_by_the_planned_hub() {
        // Only the *corner* occurrence of `s` is consumed by the splice; the
        // second `s` survives in the planned suffix, so `s` must be retained
        // even though no rule outside the cycle references it.
        let unit = rewritten(
            "parser grammar P; \
             e : s s | ID ; \
             s : e '+' ID ;",
        );
        assert!(
            unit.rules.iter().any(|rule| rule.name == "s"),
            "satellite referenced by the planned hub body is retained"
        );
        insta::assert_snapshot!("suffix_satellite_retained", render(&unit));
    }

    #[test]
    fn retains_satellite_referenced_from_an_unspliced_alternative() {
        // The hub's second alternative keeps its `arr` reference verbatim (it
        // is not a left corner), so deleting `arr` would leave a dangling call.
        let unit = rewritten(
            "parser grammar P; \
             t : arr | t '?' arr | ID ; \
             arr : t '[' ']' ;",
        );
        assert!(
            unit.rules.iter().any(|rule| rule.name == "arr"),
            "satellite referenced by an unspliced alternative is retained"
        );
        insta::assert_snapshot!("verbatim_alt_satellite_retained", render(&unit));
    }

    #[test]
    fn preserves_the_caller_alternative_label() {
        // The `#ViaSatellite` label names the *hub's* alternative — authored
        // API surface that must survive the splice (the satellite has no say).
        let unit = rewritten(
            "parser grammar P; \
             e : s # ViaSatellite | ID # Atom ; \
             s : e '+' ID ;",
        );
        insta::assert_snapshot!("caller_alt_label_preserved", render(&unit));
    }

    #[test]
    fn splitting_an_optional_keeps_the_label_on_both_products() {
        // ANTLR accepts the same `#label` on multiple alternatives (they share
        // one context class), so both split products keep the caller's label.
        let unit = rewritten(
            "parser grammar P; \
             e : e '+' e # Add | r # Range | ID # Atom ; \
             r : e? '..' ;",
        );
        insta::assert_snapshot!("split_label_on_both_products", render(&unit));
    }

    #[test]
    fn declines_predicate_prefixed_satellite_alternative() {
        // Splicing would give the hub `{pred}? e '+' ID`, whose literal first
        // element is a predicate — the direct rewriter files that under
        // *primary*, leaving the recursion undetected. The gate must mirror
        // that reading and decline before anything is touched.
        assert_declined(
            "parser grammar P; \
             e : s | ID ; \
             s : {true}? e '+' ID ;",
        );
    }

    #[test]
    fn declines_nongreedy_optional_corner() {
        // `e??` prefers the absent branch; the greedy split `e rest | rest`
        // would invert that preference, so only greedy optionals are split.
        assert_declined(
            "parser grammar P; \
             e : r | ID ; \
             r : e?? '..' ;",
        );
    }

    #[test]
    fn declines_when_a_surviving_action_references_the_satellite() {
        // `$s.text` resolves against the corner element by rule name; deleting
        // the corner would leave the reference dangling.
        assert_declined(
            "parser grammar P; \
             e : s { let _x = $s.text; } | ID ; \
             s : e '+' ID ;",
        );
    }

    #[test]
    fn declines_parameterized_hub() {
        // Every in-cycle corner is bare, so it omits the hub's required
        // arguments; rewriting would delete the argument-less call before
        // semantic call validation could reject it, leaving the parameter
        // silently default-initialized.
        assert_declined(
            "parser grammar P; \
             e[i32 x] : s | ID ; \
             s : e '+' ID ;",
        );
    }

    #[test]
    fn declines_satellite_action_bound_to_its_rule_context() {
        // `$ctx` inside the satellite means the satellite's context; spliced
        // into the hub it would silently mean the hub's instead.
        assert_declined(
            "parser grammar P; \
             e : s | ID ; \
             s : e '+' ID { let _r = $ctx; } ;",
        );
    }

    #[test]
    fn declines_when_a_splice_would_capture_an_implicit_reference() {
        // The caller's `$ID` names its own trailing `ID` occurrence; the
        // satellite body arriving in front of the action introduces another
        // `ID` that would capture the reference.
        assert_declined(
            "parser grammar P; \
             e : s { let _t = $ID.text; } ID | INT ; \
             s : e '+' ID ;",
        );
    }

    #[test]
    fn declines_any_satellite_with_embedded_semantics() {
        // Even an action bound only to the satellite's own labelled element
        // does not survive transplantation: the embedded-action pipeline
        // resolves `$i` against the enclosing alternative's source span, and
        // the spliced alternative carries the hub's. Semantic bodies are owned
        // by their rule, so any satellite with inline semantics declines.
        assert_declined(
            "parser grammar P; \
             e : s | ID ; \
             s : e '+' i=ID { let _t = $i.text; } ;",
        );
    }

    #[test]
    fn declines_nullable_recursive_tail() {
        // After splicing, the hub alternative would be `e n` with nullable
        // `n` — a left-recursive alternative that can be followed by the
        // empty string, which the direct rewriter rejects. Declining keeps
        // the diagnostic on the authored cycle instead of the rewritten rule.
        assert_declined(
            "parser grammar P; \
             e : s | ID ; \
             s : e n ; \
             n : ;",
        );
    }

    #[test]
    fn declines_when_a_split_absent_branch_is_a_bare_self_loop() {
        // `s : e?` splits into `e` (a token-free self-loop) and epsilon; the
        // direct rewriter accepts neither, so the plan declines up front.
        assert_declined(
            "parser grammar P; \
             e : s | ID ; \
             s : e? ;",
        );
    }

    #[test]
    fn leaves_lexer_grammars_untouched() {
        // Precedence rewriting is a parser-rule construct. Routing a lexer SCC
        // through it produced an "unsupported embedded lexer action" naming an
        // action the grammar never declared. Left-recursive lexer rules are
        // invalid in ANTLR regardless (error(119)); lexer ATN analysis reports
        // their left-corner cycles as G4A005 (issue #236).
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
            &BTreeSet::new(),
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
            &BTreeSet::new(),
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
