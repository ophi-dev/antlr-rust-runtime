// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
//! Inline trivial pure parser rules into their call sites.
//!
//! Two candidate classes are supported, both recognition preserving and
//! therefore opt-in (issue #130):
//!
//! - **token-set rules** — a parser rule whose body is nothing but an
//!   alternation of single terminals (keyword lists, operator names). Every
//!   reference is replaced by the flattened set, so the caller's decision
//!   sees the actual tokens instead of a rule transition. Expansion is
//!   bounded by construction: one element per call site.
//! - **single-use pure sequences** — a rule with exactly one alternative,
//!   referenced exactly once, whose body carries no observable surface
//!   (labels, actions, predicates, attributes, options, exceptions). Its
//!   body moves into the call site as a parenthesized block, removing the
//!   rule transition and context allocation without moving any decision.
//!
//! Candidates are inlined all-or-nothing: if any reference is ineligible the
//! whole candidate is declined with a reason. Applied callees are removed,
//! so the pass is idempotent, and iteration follows authored rule order, so
//! it is deterministic. Discovery re-runs after every accepted rewrite,
//! letting alias chains (`a : b ; b : X | Y ;`) collapse without composed
//! growth: every application removes exactly one rule.

use std::collections::{BTreeMap, BTreeSet};

use crate::grammar::diagnostic::Diagnostic;
use crate::grammar::frontend::SourceSpan;
use crate::grammar::model::{
    Block, Element, ElementKind, GrammarKind, GrammarUnit, ModelIdAllocator, ModelNodeId,
    Quantifier, Rule, RuleId, RuleKind, SetElement, Terminal, TransformId,
};
use crate::grammar::provenance::{Origin, ProvenanceIndex};
use crate::grammar::rule_reachability::{EntryRuleConfig, analyze};
use crate::grammar::transform::analysis::{
    AnalysisInvalidation, TransformAnalysis, observed_rule_contexts, rule_surface_is_observable,
};
use crate::grammar::transform::clone::{TransformCloner, tombstone_rule};
use crate::grammar::transform::{
    GrammarTransform, SafetyClass, TransformCallSite, TransformCandidateReport,
    TransformCandidateStatus, TransformContext, TransformGrammar, TransformReport,
    TransformRuleRemoval,
};

pub(crate) struct InlineTrivialRules {
    entries: EntryRuleConfig,
}

impl InlineTrivialRules {
    pub(crate) const NAME: &'static str = "inline-trivial-rules";

    pub(crate) const fn new(entries: EntryRuleConfig) -> Self {
        Self { entries }
    }
}

impl GrammarTransform for InlineTrivialRules {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::RecognitionPreserving
    }

    fn invalidates(&self) -> AnalysisInvalidation {
        AnalysisInvalidation::ALL
    }

    fn apply(
        &self,
        input: &TransformContext<'_>,
        grammar: &mut TransformGrammar,
        ids: &mut ModelIdAllocator,
        report: &mut TransformReport,
    ) -> Result<bool, Diagnostic> {
        let mut changed = false;
        let TransformGrammar {
            units,
            target_units,
            preserved_rules,
            provenance,
        } = grammar;
        for unit in units {
            if !target_units.contains(&unit.id) || unit.kind != GrammarKind::Parser {
                continue;
            }
            loop {
                let InlineDiscovery { plans, declined } =
                    discover(unit, &self.entries, preserved_rules, input);
                let Some(plan) = plans.into_iter().next() else {
                    report.candidates.extend(
                        declined
                            .into_iter()
                            .map(|declined| declined_report(input, &unit.name, declined)),
                    );
                    break;
                };
                let candidate = apply_plan(input, unit, &plan, ids, provenance);
                report.rule_removals.push(TransformRuleRemoval {
                    pass: input.id,
                    grammar: unit.name.clone(),
                    rule: plan.callee_name.clone(),
                    source_span: plan.name_span.clone(),
                });
                report.candidates.push(candidate);
                changed = true;
            }
        }
        Ok(changed)
    }
}

enum InlineBody {
    TokenSet(Vec<SetElement>),
    SingleUse(Block),
}

struct InlinePlan {
    callee: RuleId,
    callee_name: String,
    rule_span: SourceSpan,
    name_span: SourceSpan,
    body: InlineBody,
    call_sites: Vec<TransformCallSite>,
}

struct DeclinedInline {
    rule: String,
    span: SourceSpan,
    reason: String,
    call_sites: Vec<TransformCallSite>,
}

#[derive(Default)]
struct InlineDiscovery {
    plans: Vec<InlinePlan>,
    declined: Vec<DeclinedInline>,
}

struct ScannedSite {
    site: TransformCallSite,
    problem: Option<&'static str>,
}

struct DiscoveryContext {
    analysis: TransformAnalysis,
    entry_rules: BTreeSet<RuleId>,
    observed: BTreeSet<RuleId>,
    preserved: BTreeSet<RuleId>,
}

fn discover(
    unit: &GrammarUnit,
    entries: &EntryRuleConfig,
    preserved_rules: &BTreeSet<RuleId>,
    input: &TransformContext<'_>,
) -> InlineDiscovery {
    let analysis = TransformAnalysis::compute(std::slice::from_ref(unit));
    let context = DiscoveryContext {
        entry_rules: analyze(unit, entries).entry_rules.into_iter().collect(),
        observed: observed_rule_contexts(
            unit,
            &analysis.rules_by_name,
            input.action_reference_parser,
        ),
        preserved: preserved_rules.clone(),
        analysis,
    };
    let sites_by_callee = scan_call_sites(unit, &context.analysis.rules_by_name);
    let mut discovery = InlineDiscovery::default();
    for rule in &unit.rules {
        if rule.kind != RuleKind::Parser {
            continue;
        }
        let Some(sites) = sites_by_callee
            .get(&rule.id)
            .filter(|sites| !sites.is_empty())
        else {
            continue;
        };
        evaluate_candidate(rule, sites, &context, &mut discovery);
    }
    discovery
}

/// Collects every reference to a same-unit parser rule, keyed by callee,
/// in authored order.
fn scan_call_sites(
    unit: &GrammarUnit,
    rules_by_name: &BTreeMap<String, RuleId>,
) -> BTreeMap<RuleId, Vec<ScannedSite>> {
    let mut sites = BTreeMap::<RuleId, Vec<ScannedSite>>::new();
    for rule in &unit.rules {
        if rule.kind != RuleKind::Parser {
            continue;
        }
        for (index, alternative) in rule.block.alternatives.iter().enumerate() {
            scan_alternative_elements(
                &alternative.elements,
                rules_by_name,
                &mut sites,
                rule,
                index + 1,
            );
        }
    }
    sites
}

fn scan_alternative_elements(
    elements: &[Element],
    rules_by_name: &BTreeMap<String, RuleId>,
    sites: &mut BTreeMap<RuleId, Vec<ScannedSite>>,
    caller: &Rule,
    alternative: usize,
) {
    for element in elements {
        match &element.kind {
            ElementKind::RuleCall(call) => {
                let Some(target) = rules_by_name.get(&call.name) else {
                    continue;
                };
                let problem = if element.label.is_some() {
                    Some("a call site binds a label to the rule context")
                } else if !element.options.is_empty() {
                    Some("a call site carries element options")
                } else if call.arguments.is_some() {
                    Some("a call site passes rule arguments")
                } else if call.precedence.is_some() {
                    Some("a call site pins left-recursive precedence")
                } else {
                    None
                };
                sites.entry(*target).or_default().push(ScannedSite {
                    site: TransformCallSite {
                        caller: caller.name.clone(),
                        alternative,
                        source_span: element.span.clone(),
                    },
                    problem,
                });
            }
            ElementKind::Block(nested) => {
                for nested_alternative in &nested.alternatives {
                    scan_alternative_elements(
                        &nested_alternative.elements,
                        rules_by_name,
                        sites,
                        caller,
                        alternative,
                    );
                }
            }
            ElementKind::Terminal(_)
            | ElementKind::Range(..)
            | ElementKind::Set { .. }
            | ElementKind::Action { .. }
            | ElementKind::Predicate { .. }
            | ElementKind::Epsilon => {}
        }
    }
}

fn evaluate_candidate(
    rule: &Rule,
    sites: &[ScannedSite],
    context: &DiscoveryContext,
    discovery: &mut InlineDiscovery,
) {
    let token_set = token_set_body(rule);
    let single_use_sequence = sites.len() == 1 && rule.block.alternatives.len() == 1;
    if token_set.is_none() && !single_use_sequence {
        return;
    }
    let call_sites = sites.iter().map(|scanned| scanned.site.clone()).collect();
    match eligibility(rule, sites, token_set, context) {
        Ok(body) => discovery.plans.push(InlinePlan {
            callee: rule.id,
            callee_name: rule.name.clone(),
            rule_span: rule.span.clone(),
            name_span: rule.name_span.clone(),
            body,
            call_sites,
        }),
        Err(reason) => discovery.declined.push(DeclinedInline {
            rule: rule.name.clone(),
            span: rule.span.clone(),
            reason,
            call_sites,
        }),
    }
}

fn eligibility(
    rule: &Rule,
    sites: &[ScannedSite],
    token_set: Option<Vec<SetElement>>,
    context: &DiscoveryContext,
) -> Result<InlineBody, String> {
    if context.preserved.contains(&rule.id) {
        return Err("configured parser entry rules keep their generated API".to_owned());
    }
    if context.entry_rules.contains(&rule.id) {
        return Err("inferred parser entry rules keep their generated API".to_owned());
    }
    if context
        .analysis
        .recursive_components
        .iter()
        .any(|component| component.contains(&rule.id))
    {
        return Err("recursive rules cannot be inlined".to_owned());
    }
    if context.observed.contains(&rule.id) {
        return Err("grammar target code observes the rule context".to_owned());
    }
    if rule_surface_is_observable(rule) {
        return Err(
            "rule-level attributes, actions, options, or exceptions are observable".to_owned(),
        );
    }
    if let Some(problem) = sites.iter().find_map(|scanned| {
        scanned
            .problem
            .map(|problem| (problem, scanned.site.caller.clone()))
    }) {
        return Err(format!("{} in rule {}", problem.0, problem.1));
    }
    if let Some(members) = token_set {
        return Ok(InlineBody::TokenSet(members));
    }
    single_use_purity(rule, &context.analysis)?;
    Ok(InlineBody::SingleUse(rule.block.clone()))
}

/// A body consisting solely of single-terminal alternatives, flattened into
/// prospective set members. `EOF`, wildcard, and any observable surface
/// disqualify the shape.
fn token_set_body(rule: &Rule) -> Option<Vec<SetElement>> {
    if !rule.block.options.is_empty() {
        return None;
    }
    let mut members = Vec::new();
    for alternative in &rule.block.alternatives {
        if alternative.label.is_some()
            || !alternative.options.is_empty()
            || !alternative.commands.is_empty()
            || alternative.elements.len() != 1
        {
            return None;
        }
        let element = &alternative.elements[0];
        if element.quantifier != Quantifier::One
            || element.label.is_some()
            || !element.options.is_empty()
        {
            return None;
        }
        match &element.kind {
            ElementKind::Terminal(terminal) => members.push(SetElement::Terminal {
                source: element.id,
                value: inlinable_terminal(terminal)?.clone(),
                span: element.span.clone(),
                options: Vec::new(),
            }),
            ElementKind::Set {
                inverted: false,
                elements,
            } => {
                for member in elements {
                    members.push(inlinable_set_member(member)?.clone());
                }
            }
            _ => return None,
        }
    }
    if members.is_empty() {
        return None;
    }
    let mut seen: Vec<Terminal> = Vec::new();
    members.retain(|member| match member {
        SetElement::Terminal { value, .. } => {
            if seen.contains(value) {
                false
            } else {
                seen.push(value.clone());
                true
            }
        }
        SetElement::Range { .. } => true,
    });
    Some(members)
}

fn inlinable_terminal(terminal: &Terminal) -> Option<&Terminal> {
    match terminal {
        Terminal::Token(name) if name != "EOF" => Some(terminal),
        Terminal::Literal(_) => Some(terminal),
        Terminal::Token(_) | Terminal::LexerCharSet(_) | Terminal::Wildcard | Terminal::Eof => None,
    }
}

fn inlinable_set_member(member: &SetElement) -> Option<&SetElement> {
    match member {
        SetElement::Terminal { value, options, .. }
            if options.is_empty() && inlinable_terminal(value).is_some() =>
        {
            Some(member)
        }
        SetElement::Terminal { .. } | SetElement::Range { .. } => None,
    }
}

fn single_use_purity(rule: &Rule, analysis: &TransformAnalysis) -> Result<(), String> {
    if analysis.side_effecting.contains(&rule.id) {
        return Err("embedded actions or predicates are observable".to_owned());
    }
    if analysis.nullable.contains(&rule.id) {
        return Err("a nullable body can change decision ownership at the call site".to_owned());
    }
    validate_pure_block(&rule.block)
}

fn validate_pure_block(block: &Block) -> Result<(), String> {
    if !block.options.is_empty() {
        return Err("block options are observable".to_owned());
    }
    for alternative in &block.alternatives {
        if alternative.label.is_some() {
            return Err("alternative labels define generated context types".to_owned());
        }
        if !alternative.options.is_empty() || !alternative.commands.is_empty() {
            return Err("alternative options or commands are observable".to_owned());
        }
        for element in &alternative.elements {
            validate_pure_element(element)?;
        }
    }
    Ok(())
}

fn validate_pure_element(element: &Element) -> Result<(), String> {
    if element.label.is_some() {
        return Err("element labels bind generated accessors".to_owned());
    }
    if !element.options.is_empty() {
        return Err("element options are observable".to_owned());
    }
    match &element.kind {
        ElementKind::Action { .. } | ElementKind::Predicate { .. } => {
            Err("embedded actions or predicates are observable".to_owned())
        }
        ElementKind::RuleCall(call) if call.arguments.is_some() || call.precedence.is_some() => {
            Err("nested rule calls pass arguments or pin precedence".to_owned())
        }
        ElementKind::Range(..) => Err("token ranges are not valid in parser rules".to_owned()),
        ElementKind::Block(nested) => validate_pure_block(nested),
        ElementKind::RuleCall(_)
        | ElementKind::Terminal(_)
        | ElementKind::Set { .. }
        | ElementKind::Epsilon => Ok(()),
    }
}

fn apply_plan(
    input: &TransformContext<'_>,
    unit: &mut GrammarUnit,
    plan: &InlinePlan,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> TransformCandidateReport {
    let mut rewritten = Vec::new();
    for rule in &mut unit.rules {
        if rule.id != plan.callee {
            rewrite_block(
                &mut rule.block,
                plan,
                input.id,
                ids,
                provenance,
                &mut rewritten,
            );
        }
    }
    if let Some(callee) = unit.rules.iter().find(|rule| rule.id == plan.callee) {
        tombstone_rule(
            provenance,
            callee,
            "trivial rule inlined into its call sites",
            &rewritten,
        );
    }
    unit.rules.retain(|rule| rule.id != plan.callee);
    applied_report(input, &unit.name, plan)
}

fn rewrite_block(
    block: &mut Block,
    plan: &InlinePlan,
    pass: TransformId,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
    rewritten: &mut Vec<ModelNodeId>,
) {
    for alternative in &mut block.alternatives {
        for element in &mut alternative.elements {
            match &mut element.kind {
                ElementKind::Block(nested) => {
                    rewrite_block(nested, plan, pass, ids, provenance, rewritten);
                }
                ElementKind::RuleCall(call) if call.name == plan.callee_name => {
                    element.kind = replacement_kind(&plan.body, pass, ids, provenance);
                    let node = ModelNodeId::Element(element.id);
                    let mut origins = provenance.origins(node).to_vec();
                    origins.push(Origin::OptionalTransform {
                        pass,
                        inputs: Box::new([ModelNodeId::Rule(plan.callee)]),
                    });
                    provenance.record_model(node, origins);
                    rewritten.push(node);
                }
                _ => {}
            }
        }
    }
}

fn replacement_kind(
    body: &InlineBody,
    pass: TransformId,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> ElementKind {
    match body {
        InlineBody::TokenSet(members) => {
            if let [SetElement::Terminal { value, .. }] = members.as_slice() {
                ElementKind::Terminal(value.clone())
            } else {
                ElementKind::Set {
                    inverted: false,
                    elements: members.clone(),
                }
            }
        }
        InlineBody::SingleUse(callee_block) => {
            let mut cloner = TransformCloner {
                ids,
                provenance,
                pass,
            };
            ElementKind::Block(cloner.block(callee_block))
        }
    }
}

fn applied_report(
    input: &TransformContext<'_>,
    unit_name: &str,
    plan: &InlinePlan,
) -> TransformCandidateReport {
    let reason = match &plan.body {
        InlineBody::TokenSet(members) => format!(
            "inlined token-set rule {} ({} members) into {} call site{}",
            plan.callee_name,
            members.len(),
            plan.call_sites.len(),
            if plan.call_sites.len() == 1 { "" } else { "s" }
        ),
        InlineBody::SingleUse(_) => format!(
            "inlined single-use rule {} into its call site in {}",
            plan.callee_name, plan.call_sites[0].caller
        ),
    };
    TransformCandidateReport {
        pass: input.id,
        grammar: unit_name.to_owned(),
        entry_rule: plan.callee_name.clone(),
        source_span: plan.rule_span.clone(),
        status: if input.report_only {
            TransformCandidateStatus::Eligible
        } else {
            TransformCandidateStatus::Applied
        },
        reason,
        rungs: Vec::new(),
        boundary_rule: None,
        projection: None,
        removed_rules: vec![plan.callee_name.clone()],
        alternatives: Vec::new(),
        labels: Vec::new(),
        grouping_changes: Vec::new(),
        call_sites: plan.call_sites.clone(),
    }
}

fn declined_report(
    input: &TransformContext<'_>,
    unit_name: &str,
    declined: DeclinedInline,
) -> TransformCandidateReport {
    TransformCandidateReport {
        pass: input.id,
        grammar: unit_name.to_owned(),
        entry_rule: declined.rule,
        source_span: declined.span,
        status: TransformCandidateStatus::Declined,
        reason: declined.reason,
        rungs: Vec::new(),
        boundary_rule: None,
        projection: None,
        removed_rules: Vec::new(),
        alternatives: Vec::new(),
        labels: Vec::new(),
        grouping_changes: Vec::new(),
        call_sites: declined.call_sites,
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;
    use crate::grammar::transform::TransformRegistry;
    use crate::grammar::transform::test_support::single_unit_fixture as fixture;

    fn registry(entries: EntryRuleConfig) -> TransformRegistry {
        let mut registry = TransformRegistry::default();
        registry.push(InlineTrivialRules::new(entries));
        registry
    }

    fn shape(unit: &GrammarUnit) -> Vec<String> {
        unit.rules
            .iter()
            .map(|rule| format!("{} : {}", rule.name, block_shape(&rule.block)))
            .collect()
    }

    fn block_shape(block: &Block) -> String {
        block
            .alternatives
            .iter()
            .map(|alternative| {
                alternative
                    .elements
                    .iter()
                    .map(element_shape)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    fn element_shape(element: &Element) -> String {
        let label = element
            .label
            .as_ref()
            .map_or_else(String::new, |label| format!("{}=", label.name));
        let kind = match &element.kind {
            ElementKind::Terminal(terminal) => terminal_shape(terminal),
            ElementKind::RuleCall(call) => call.name.clone(),
            ElementKind::Set { inverted, elements } => format!(
                "{}{{{}}}",
                if *inverted { "~" } else { "" },
                elements
                    .iter()
                    .map(|member| match member {
                        SetElement::Terminal { value, .. } => terminal_shape(value),
                        SetElement::Range { start, stop, .. } => format!("{start}..{stop}"),
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            ElementKind::Block(nested) => format!("({})", block_shape(nested)),
            ElementKind::Range(start, stop, _) => format!("{}..{}", start.value, stop.value),
            ElementKind::Action { .. } => "{action}".to_owned(),
            ElementKind::Predicate { .. } => "{predicate}?".to_owned(),
            ElementKind::Epsilon => "ε".to_owned(),
        };
        let quantifier = match element.quantifier {
            Quantifier::One => "",
            Quantifier::Optional { .. } => "?",
            Quantifier::ZeroOrMore { .. } => "*",
            Quantifier::OneOrMore { .. } => "+",
        };
        format!("{label}{kind}{quantifier}")
    }

    fn terminal_shape(terminal: &Terminal) -> String {
        match terminal {
            Terminal::Token(name) => name.clone(),
            Terminal::Literal(literal) | Terminal::LexerCharSet(literal) => literal.clone(),
            Terminal::Wildcard => ".".to_owned(),
            Terminal::Eof => "EOF".to_owned(),
        }
    }

    fn candidate_lines(report: &TransformReport) -> Vec<String> {
        report
            .candidates
            .iter()
            .map(|candidate| {
                format!(
                    "{:?} {} [{}]: {}",
                    candidate.status,
                    candidate.entry_rule,
                    candidate
                        .call_sites
                        .iter()
                        .map(|site| format!("{}#{}", site.caller, site.alternative))
                        .collect::<Vec<_>>()
                        .join(", "),
                    candidate.reason
                )
            })
            .collect()
    }

    #[test]
    fn inlines_multi_use_token_set_rules_and_is_idempotent() {
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
start : stmt+ EOF ;
stmt : kw ID SEMI | ID kw* SEMI ;
kw : 'select' | 'from' | KWTOK ;
",
        );
        let registry = registry(EntryRuleConfig::default());
        let report = registry
            .run(&mut grammar, &mut ids, false)
            .expect("token-set inlining should apply");

        insta::assert_debug_snapshot!(
            "token_set_inline_shapes_and_candidates",
            (shape(&grammar.units[0]), candidate_lines(&report))
        );
        assert_eq!(report.rule_removals.len(), 1);
        assert_eq!(report.rule_removals[0].rule, "kw");
        assert!(report.entries[0].changed);
        assert_eq!(report.entries[0].before.rules, 3);
        assert_eq!(report.entries[0].after.rules, 2);

        let second = registry
            .run(&mut grammar, &mut ids, false)
            .expect("a second run should remain valid");
        assert!(!second.entries[0].changed);
        assert!(second.candidates.is_empty());
    }

    #[test]
    fn single_use_pure_sequence_is_inlined_as_a_block() {
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
start : item (COMMA item)* EOF ;
item : prefix ID ;
prefix : AT AT? ;
",
        );
        let report = registry(EntryRuleConfig::default())
            .run(&mut grammar, &mut ids, false)
            .expect("single-use inlining should apply");

        insta::assert_debug_snapshot!(
            "single_use_inline_shapes_and_candidates",
            (shape(&grammar.units[0]), candidate_lines(&report))
        );
        assert_eq!(
            grammar.units[0]
                .rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect::<Vec<_>>(),
            ["start", "item"]
        );
    }

    #[test]
    fn alias_chains_collapse_through_the_fixpoint() {
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
start : a EOF ;
a : b ;
b : X | Y ;
",
        );
        let report = registry(EntryRuleConfig::default())
            .run(&mut grammar, &mut ids, false)
            .expect("alias chain should collapse");

        insta::assert_debug_snapshot!(
            "alias_chain_shapes_and_candidates",
            (shape(&grammar.units[0]), candidate_lines(&report))
        );
        assert_eq!(grammar.units[0].rules.len(), 1);
    }

    #[test]
    fn labeled_call_sites_decline_the_whole_candidate() {
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
start : x=kw kw ID EOF ;
kw : A | B ;
",
        );
        let report = registry(EntryRuleConfig::default())
            .run(&mut grammar, &mut ids, false)
            .expect("declined candidates should not fail the pass");

        assert!(!report.entries[0].changed);
        assert_eq!(grammar.units[0].rules.len(), 2, "kw must be retained");
        insta::assert_debug_snapshot!("labeled_call_site_declines", candidate_lines(&report));
    }

    #[test]
    fn nullable_and_recursive_single_use_rules_are_declined() {
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
start : wrap nul EOF ;
wrap : rec | X ;
rec : LP wrap RP ;
nul : AT? ;
",
        );
        let report = registry(EntryRuleConfig::default())
            .run(&mut grammar, &mut ids, false)
            .expect("declined candidates should not fail the pass");

        assert!(!report.entries[0].changed);
        insta::assert_debug_snapshot!("nullable_and_recursive_declines", candidate_lines(&report));
    }

    #[test]
    fn configured_entry_rules_keep_their_api() {
        let entries = EntryRuleConfig::new(["kw".to_owned()]);
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
start : kw ID EOF ;
kw : A | B ;
",
        );
        grammar.preserved_rules = entries.matching_rule_ids(&grammar.units, &grammar.target_units);
        let report = registry(entries)
            .run(&mut grammar, &mut ids, false)
            .expect("declined candidates should not fail the pass");

        assert!(!report.entries[0].changed);
        insta::assert_debug_snapshot!("configured_entry_declines", candidate_lines(&report));
    }

    #[test]
    fn opaque_target_code_declines_every_candidate() {
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
@members { int depth; }
start : kw ID EOF ;
kw : A | B ;
",
        );
        let report = registry(EntryRuleConfig::default())
            .run(&mut grammar, &mut ids, false)
            .expect("declined candidates should not fail the pass");

        assert!(!report.entries[0].changed);
        insta::assert_debug_snapshot!("opaque_target_code_declines", candidate_lines(&report));
    }

    #[test]
    fn rule_level_options_decline_token_set_candidates() {
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
start : kw ID EOF ;
kw options { caseInsensitive=true; } : A | B ;
",
        );
        let report = registry(EntryRuleConfig::default())
            .run(&mut grammar, &mut ids, false)
            .expect("declined candidates should not fail the pass");

        assert!(!report.entries[0].changed);
        assert_eq!(grammar.units[0].rules.len(), 2, "kw must be retained");
        insta::assert_debug_snapshot!("rule_level_option_declines", candidate_lines(&report));
    }

    #[test]
    fn duplicate_token_set_members_are_deduplicated() {
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
start : kw ID EOF ;
kw : A | B | A ;
",
        );
        let report = registry(EntryRuleConfig::default())
            .run(&mut grammar, &mut ids, false)
            .expect("duplicate members should still inline");

        insta::assert_debug_snapshot!(
            "duplicate_member_dedup_shapes_and_candidates",
            (shape(&grammar.units[0]), candidate_lines(&report))
        );
    }

    #[test]
    fn report_only_projects_the_rewrite_without_mutating_the_grammar() {
        let (mut grammar, mut ids) = fixture(
            r"
parser grammar P;
start : stmt+ EOF ;
stmt : kw ID SEMI | ID kw* SEMI ;
kw : 'select' | 'from' | KWTOK ;
",
        );
        let before = grammar.units.clone();
        let report = registry(EntryRuleConfig::default())
            .run(&mut grammar, &mut ids, true)
            .expect("dry-run should analyze candidates");

        assert_eq!(grammar.units, before);
        assert_eq!(
            report.candidates[0].status,
            TransformCandidateStatus::Eligible
        );
        assert_eq!(report.entries[0].before.rules, 3);
        assert_eq!(report.entries[0].after.rules, 2);
    }
}
