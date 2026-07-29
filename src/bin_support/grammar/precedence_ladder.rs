use std::collections::{BTreeMap, BTreeSet};

use super::action::{ActionReferenceKind, action_references};
use super::char_support::get_string_from_grammar_string_literal;
use super::diagnostic::Diagnostic;
use super::model::{
    Alternative, Authored, Block, Element, ElementKind, GrammarUnit, Label, ModelIdAllocator,
    ModelNodeId, OptionDecl, Quantifier, Rule, RuleCall, RuleId, RuleKind, SetElement, Terminal,
};
use super::provenance::{Origin, ProvenanceIndex, Tombstone};
use super::transform::{
    GrammarTransform, SafetyClass, TransformAlternativeMapping, TransformCandidateReport,
    TransformCandidateStatus, TransformContext, TransformGrammar, TransformLabelMapping,
    TransformProjection, TransformReport,
};
use super::transform_analysis::AnalysisInvalidation;

pub(crate) struct CollapsePrecedenceLadders;

impl GrammarTransform for CollapsePrecedenceLadders {
    fn name(&self) -> &'static str {
        "collapse-precedence-ladders"
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
        for unit in &mut grammar.units {
            let discovery = discover_ladders(unit);
            for declined in discovery.declined {
                report
                    .candidates
                    .push(declined_report(input, unit, declined));
            }
            for plan in discovery.plans {
                let candidate = apply_plan(input, unit, plan, ids, &mut grammar.provenance);
                report.candidates.push(candidate);
                changed = true;
            }
        }
        Ok(changed)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Fixity {
    Prefix,
    Infix,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum OperatorSymbol {
    Literal(String),
    Token(String),
}

#[derive(Clone, Debug)]
struct OperatorSignature {
    fixity: Fixity,
    symbols: BTreeSet<OperatorSymbol>,
}

#[derive(Clone, Debug)]
enum RungKind {
    Delegation,
    Star { loop_element: usize },
    Direct { recursive_alternatives: Vec<usize> },
    RightTail { tail_element: usize },
    Prefix { prefix_alternatives: Vec<usize> },
}

#[derive(Clone, Debug)]
struct Rung {
    rule: Rule,
    base_rule: String,
    base_alternative: usize,
    kind: RungKind,
    operators: Vec<OperatorSignature>,
}

impl Rung {
    const fn decision_count(&self) -> usize {
        match self.kind {
            RungKind::Delegation => 0,
            RungKind::Star { .. }
            | RungKind::Direct { .. }
            | RungKind::RightTail { .. }
            | RungKind::Prefix { .. } => 1,
        }
    }

    const fn changes_grouping(&self) -> bool {
        matches!(self.kind, RungKind::Star { .. })
    }
}

#[derive(Debug)]
struct LadderPlan {
    rungs: Vec<Rung>,
    boundary_rule: String,
}

#[derive(Debug)]
struct DeclinedLadder {
    entry_rule: Rule,
    rungs: Vec<String>,
    boundary_rule: Option<String>,
    reason: String,
}

#[derive(Debug, Default)]
struct LadderDiscovery {
    plans: Vec<LadderPlan>,
    declined: Vec<DeclinedLadder>,
}

fn discover_ladders(unit: &GrammarUnit) -> LadderDiscovery {
    let rules_by_name = unit
        .rules
        .iter()
        .map(|rule| (rule.name.clone(), rule.id))
        .collect::<BTreeMap<_, _>>();
    let mut rungs = BTreeMap::new();
    let mut declined_rules = BTreeMap::new();
    for rule in &unit.rules {
        match classify_rung(rule) {
            Ok(Some(rung)) => {
                rungs.insert(rule.id, rung);
            }
            Err(reason) => {
                declined_rules.insert(rule.id, reason);
            }
            Ok(None) => {}
        }
    }

    let incoming = incoming_callers(unit, &rules_by_name);
    let observed = observed_rule_contexts(unit, &rules_by_name);
    let parents = rung_parents(&rungs, &rules_by_name);
    let tops = rungs
        .keys()
        .copied()
        .filter(|rule| unique_removable_parent(*rule, &parents, &incoming, &observed).is_none())
        .collect::<Vec<_>>();

    let mut discovery = LadderDiscovery::default();
    let declined_context = DeclinedTailContext {
        unit,
        rules_by_name: &rules_by_name,
        declined_rules: &declined_rules,
        rungs: &rungs,
    };
    for top in tops {
        let (chain, boundary) =
            follow_chain(top, &rungs, &rules_by_name, &parents, &incoming, &observed);
        if rules_by_name
            .get(&boundary)
            .is_some_and(|boundary| chain.contains(boundary))
        {
            let entry = rungs[&top].rule.clone();
            discovery.declined.push(DeclinedLadder {
                entry_rule: entry,
                rungs: chain
                    .iter()
                    .map(|rule| rungs[rule].rule.name.clone())
                    .collect(),
                boundary_rule: Some(boundary),
                reason: "ladder delegates through a mutual-recursion cycle".to_owned(),
            });
            continue;
        }
        if chain.len() < 2 {
            maybe_report_declined_tail(
                &declined_context,
                &chain,
                &boundary,
                &mut discovery.declined,
            );
            continue;
        }
        let selected = chain
            .iter()
            .filter_map(|rule| rungs.get(rule).cloned())
            .collect::<Vec<_>>();
        if let Err(reason) = prove_operator_totality(&selected) {
            let entry = selected[0].rule.clone();
            discovery.declined.push(DeclinedLadder {
                entry_rule: entry,
                rungs: selected.iter().map(|rung| rung.rule.name.clone()).collect(),
                boundary_rule: Some(boundary),
                reason,
            });
            continue;
        }
        discovery.plans.push(LadderPlan {
            rungs: selected,
            boundary_rule: boundary,
        });
    }
    discovery
}

struct DeclinedTailContext<'a> {
    unit: &'a GrammarUnit,
    rules_by_name: &'a BTreeMap<String, RuleId>,
    declined_rules: &'a BTreeMap<RuleId, String>,
    rungs: &'a BTreeMap<RuleId, Rung>,
}

fn maybe_report_declined_tail(
    context: &DeclinedTailContext<'_>,
    chain: &[RuleId],
    boundary: &str,
    declined: &mut Vec<DeclinedLadder>,
) {
    let Some(entry_id) = chain.first() else {
        return;
    };
    let Some(boundary_id) = context.rules_by_name.get(boundary) else {
        return;
    };
    let Some(reason) = context.declined_rules.get(boundary_id) else {
        return;
    };
    let Some(entry) = context.unit.rules.iter().find(|rule| rule.id == *entry_id) else {
        return;
    };
    let mut names = chain
        .iter()
        .filter_map(|rule| context.rungs.get(rule).map(|rung| rung.rule.name.clone()))
        .collect::<Vec<_>>();
    names.push(boundary.to_owned());
    declined.push(DeclinedLadder {
        entry_rule: entry.clone(),
        rungs: names,
        boundary_rule: Some(boundary.to_owned()),
        reason: format!("rung {boundary} was declined: {reason}"),
    });
}

fn rung_parents(
    rungs: &BTreeMap<RuleId, Rung>,
    rules_by_name: &BTreeMap<String, RuleId>,
) -> BTreeMap<RuleId, Vec<RuleId>> {
    let mut parents = BTreeMap::<RuleId, Vec<RuleId>>::new();
    for (rule, rung) in rungs {
        if let Some(target) = rules_by_name.get(&rung.base_rule) {
            parents.entry(*target).or_default().push(*rule);
        }
    }
    parents
}

fn unique_removable_parent(
    rule: RuleId,
    parents: &BTreeMap<RuleId, Vec<RuleId>>,
    incoming: &BTreeMap<RuleId, BTreeSet<RuleId>>,
    observed: &BTreeSet<RuleId>,
) -> Option<RuleId> {
    if observed.contains(&rule) {
        return None;
    }
    let [parent] = parents.get(&rule)?.as_slice() else {
        return None;
    };
    let callers = incoming.get(&rule)?;
    (callers.len() == 1 && callers.contains(parent)).then_some(*parent)
}

fn follow_chain(
    top: RuleId,
    rungs: &BTreeMap<RuleId, Rung>,
    rules_by_name: &BTreeMap<String, RuleId>,
    parents: &BTreeMap<RuleId, Vec<RuleId>>,
    incoming: &BTreeMap<RuleId, BTreeSet<RuleId>>,
    observed: &BTreeSet<RuleId>,
) -> (Vec<RuleId>, String) {
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = top;
    loop {
        if !seen.insert(current) {
            return (chain, rungs[&current].base_rule.clone());
        }
        chain.push(current);
        let base = rungs[&current].base_rule.clone();
        let Some(next) = rules_by_name.get(&base).copied() else {
            return (chain, base);
        };
        let removable = unique_removable_parent(next, parents, incoming, observed) == Some(current);
        if removable && rungs.contains_key(&next) {
            current = next;
        } else {
            return (chain, base);
        }
    }
}

fn prove_operator_totality(rungs: &[Rung]) -> Result<(), String> {
    let operators = rungs
        .iter()
        .flat_map(|rung| rung.operators.iter())
        .collect::<Vec<_>>();
    for (index, left) in operators.iter().enumerate() {
        for right in &operators[index + 1..] {
            if left.fixity != right.fixity {
                continue;
            }
            match operator_sets_overlap(&left.symbols, &right.symbols) {
                Some(false) => {}
                Some(true) => {
                    return Err("operator token sets overlap across precedence levels".to_owned());
                }
                None => {
                    return Err("a symbolic-token/literal overlap cannot be disproved".to_owned());
                }
            }
        }
    }
    Ok(())
}

fn operator_sets_overlap(
    left: &BTreeSet<OperatorSymbol>,
    right: &BTreeSet<OperatorSymbol>,
) -> Option<bool> {
    let mut unknown = false;
    for left_symbol in left {
        for right_symbol in right {
            match (left_symbol, right_symbol) {
                (OperatorSymbol::Literal(left), OperatorSymbol::Literal(right))
                | (OperatorSymbol::Token(left), OperatorSymbol::Token(right)) => {
                    if left == right {
                        return Some(true);
                    }
                }
                _ => unknown = true,
            }
        }
    }
    (!unknown).then_some(false)
}

fn classify_rung(rule: &Rule) -> Result<Option<Rung>, String> {
    if rule.kind != RuleKind::Parser {
        return Ok(None);
    }
    let Some(rough_base) = rough_base_rule(rule) else {
        return Ok(None);
    };
    validate_rule_surface(rule)?;

    if let Some(rung) = classify_delegation(rule, &rough_base)? {
        return Ok(Some(rung));
    }
    if let Some(rung) = classify_star_or_tail(rule, &rough_base)? {
        return Ok(Some(rung));
    }
    if let Some(rung) = classify_direct_or_prefix(rule)? {
        return Ok(Some(rung));
    }
    Ok(None)
}

fn rough_base_rule(rule: &Rule) -> Option<String> {
    rule.block
        .alternatives
        .iter()
        .flat_map(|alternative| alternative.elements.iter())
        .find_map(|element| match &element.kind {
            ElementKind::RuleCall(call) if call.name != rule.name => Some(call.name.clone()),
            _ => None,
        })
}

fn validate_rule_surface(rule: &Rule) -> Result<(), String> {
    if !rule.modifiers.is_empty()
        || rule.arguments.is_some()
        || rule.returns.is_some()
        || rule.locals.is_some()
        || !rule.throws.is_empty()
        || !rule.options.is_empty()
        || !rule.actions.is_empty()
        || !rule.catches.is_empty()
        || rule.finally_action.is_some()
    {
        return Err(
            "rule-level attributes, actions, options, or exceptions are observable".to_owned(),
        );
    }
    if !rule.block.options.is_empty() {
        return Err("block options are not supported by the ladder proof".to_owned());
    }
    validate_block_elements(&rule.block)
}

fn validate_block_elements(block: &Block) -> Result<(), String> {
    for alternative in &block.alternatives {
        if !alternative.commands.is_empty() {
            return Err("lexer commands are not valid in a parser ladder".to_owned());
        }
        for element in &alternative.elements {
            if !element.options.is_empty() {
                return Err("element options are not supported by the ladder proof".to_owned());
            }
            if matches!(
                element.quantifier,
                Quantifier::Optional { greedy: false }
                    | Quantifier::ZeroOrMore { greedy: false }
                    | Quantifier::OneOrMore { greedy: false }
            ) {
                return Err("nongreedy repetition is observable".to_owned());
            }
            match &element.kind {
                ElementKind::Action { .. } | ElementKind::Predicate { .. } => {
                    return Err("actions and predicates are observable".to_owned());
                }
                ElementKind::RuleCall(call)
                    if call.arguments.is_some() || call.precedence.is_some() =>
                {
                    return Err("rule-call arguments or precedence are observable".to_owned());
                }
                ElementKind::Block(nested) => validate_block_elements(nested)?,
                ElementKind::Terminal(_)
                | ElementKind::RuleCall(_)
                | ElementKind::Range(..)
                | ElementKind::Set { .. }
                | ElementKind::Epsilon => {}
            }
        }
    }
    Ok(())
}

fn classify_delegation(rule: &Rule, base: &str) -> Result<Option<Rung>, String> {
    let [alternative] = rule.block.alternatives.as_slice() else {
        return Ok(None);
    };
    let [element] = alternative.elements.as_slice() else {
        return Ok(None);
    };
    if !is_plain_call(element, base) {
        return Ok(None);
    }
    if !alternative.options.is_empty() {
        return Err("delegating alternatives with options are not supported".to_owned());
    }
    Ok(Some(Rung {
        rule: rule.clone(),
        base_rule: base.to_owned(),
        base_alternative: 0,
        kind: RungKind::Delegation,
        operators: Vec::new(),
    }))
}

fn classify_star_or_tail(rule: &Rule, base: &str) -> Result<Option<Rung>, String> {
    let [alternative] = rule.block.alternatives.as_slice() else {
        return Ok(None);
    };
    if !alternative.options.is_empty() {
        return Ok(None);
    }
    let [first, tail] = alternative.elements.as_slice() else {
        return Ok(None);
    };
    if !is_plain_call(first, base) || tail.label.is_some() || !tail.options.is_empty() {
        return Ok(None);
    }
    let ElementKind::Block(block) = &tail.kind else {
        return Ok(None);
    };
    let [body] = block.alternatives.as_slice() else {
        return Ok(None);
    };
    if !body.options.is_empty() || body.label.is_some() || !body.commands.is_empty() {
        return Ok(None);
    }
    match tail.quantifier {
        Quantifier::ZeroOrMore { greedy: true } => {
            let [operator, operand] = body.elements.as_slice() else {
                return Ok(None);
            };
            if !is_plain_call(operand, base) {
                return Ok(None);
            }
            let symbols = operator_symbols(operator, Quantifier::One)?;
            Ok(Some(Rung {
                rule: rule.clone(),
                base_rule: base.to_owned(),
                base_alternative: 0,
                kind: RungKind::Star { loop_element: 1 },
                operators: vec![OperatorSignature {
                    fixity: Fixity::Infix,
                    symbols,
                }],
            }))
        }
        Quantifier::Optional { greedy: true } => {
            let operators = right_tail_operators(rule, base, body)?;
            if operators.is_empty() {
                return Ok(None);
            }
            Ok(Some(Rung {
                rule: rule.clone(),
                base_rule: base.to_owned(),
                base_alternative: 0,
                kind: RungKind::RightTail { tail_element: 1 },
                operators,
            }))
        }
        Quantifier::One
        | Quantifier::Optional { greedy: false }
        | Quantifier::ZeroOrMore { greedy: false }
        | Quantifier::OneOrMore { .. } => Ok(None),
    }
}

fn right_tail_operators(
    rule: &Rule,
    base: &str,
    body: &Alternative,
) -> Result<Vec<OperatorSignature>, String> {
    match body.elements.as_slice() {
        [operator, recursive] if is_plain_call(recursive, &rule.name) => {
            let symbols = operator_symbols(operator, Quantifier::One)?;
            Ok(vec![OperatorSignature {
                fixity: Fixity::Infix,
                symbols,
            }])
        }
        [first_operator, middle, second_operator, recursive]
            if is_plain_call(middle, base) && is_plain_call(recursive, &rule.name) =>
        {
            let mut symbols = operator_symbols(first_operator, Quantifier::One)?;
            symbols.extend(operator_symbols(second_operator, Quantifier::One)?);
            Ok(vec![OperatorSignature {
                fixity: Fixity::Infix,
                symbols,
            }])
        }
        _ => Ok(Vec::new()),
    }
}

fn classify_direct_or_prefix(rule: &Rule) -> Result<Option<Rung>, String> {
    let base_alternatives = rule
        .block
        .alternatives
        .iter()
        .enumerate()
        .filter_map(|(index, alternative)| {
            let [element] = alternative.elements.as_slice() else {
                return None;
            };
            let ElementKind::RuleCall(call) = &element.kind else {
                return None;
            };
            (is_plain_call(element, &call.name) && call.name != rule.name)
                .then_some((index, call.name.clone()))
        })
        .collect::<Vec<_>>();
    let [(base_alternative, base_rule)] = base_alternatives.as_slice() else {
        return Ok(None);
    };
    if !rule.block.alternatives[*base_alternative]
        .options
        .is_empty()
    {
        return Err("base alternatives with options are not supported".to_owned());
    }
    let other = (0..rule.block.alternatives.len())
        .filter(|index| index != base_alternative)
        .collect::<Vec<_>>();
    if other.is_empty() {
        return Ok(None);
    }

    if other.iter().all(|index| {
        direct_recursive_shape(&rule.block.alternatives[*index], &rule.name, base_rule)
    }) {
        let mut operators = Vec::new();
        for index in &other {
            let alternative = &rule.block.alternatives[*index];
            validate_association_options(alternative)?;
            if association_is_right(alternative)
                && is_plain_call(&alternative.elements[2], base_rule)
            {
                return Err(
                    "<assoc=right> requires a recursive right operand in a direct-LR rung"
                        .to_owned(),
                );
            }
            operators.push(OperatorSignature {
                fixity: Fixity::Infix,
                symbols: operator_symbols(&alternative.elements[1], Quantifier::One)?,
            });
        }
        return Ok(Some(Rung {
            rule: rule.clone(),
            base_rule: base_rule.clone(),
            base_alternative: *base_alternative,
            kind: RungKind::Direct {
                recursive_alternatives: other,
            },
            operators,
        }));
    }

    if other
        .iter()
        .all(|index| prefix_shape(&rule.block.alternatives[*index], base_rule))
    {
        let mut operators = Vec::new();
        for index in &other {
            let alternative = &rule.block.alternatives[*index];
            if !alternative.options.is_empty() {
                return Err("prefix alternatives with options are not supported".to_owned());
            }
            operators.push(OperatorSignature {
                fixity: Fixity::Prefix,
                symbols: operator_symbols(
                    &alternative.elements[0],
                    Quantifier::OneOrMore { greedy: true },
                )?,
            });
        }
        return Ok(Some(Rung {
            rule: rule.clone(),
            base_rule: base_rule.clone(),
            base_alternative: *base_alternative,
            kind: RungKind::Prefix {
                prefix_alternatives: other,
            },
            operators,
        }));
    }
    Ok(None)
}

fn direct_recursive_shape(alternative: &Alternative, rule: &str, base: &str) -> bool {
    let [left, operator, right] = alternative.elements.as_slice() else {
        return false;
    };
    is_plain_call(left, rule)
        && is_operator_outline(operator)
        && (is_plain_call(right, rule) || is_plain_call(right, base))
}

fn prefix_shape(alternative: &Alternative, base: &str) -> bool {
    let [operator, operand] = alternative.elements.as_slice() else {
        return false;
    };
    is_operator_outline(operator) && is_plain_call(operand, base)
}

fn validate_association_options(alternative: &Alternative) -> Result<(), String> {
    if alternative.options.is_empty()
        || (alternative.options.len() == 1
            && alternative.options[0].name.value == "assoc"
            && alternative.options[0].value.value == "right")
    {
        Ok(())
    } else {
        Err("only <assoc=right> is supported on a ladder operator".to_owned())
    }
}

const fn is_operator_outline(element: &Element) -> bool {
    matches!(
        element.kind,
        ElementKind::Terminal(_) | ElementKind::Set { .. } | ElementKind::Block(_)
    )
}

fn is_plain_call(element: &Element, name: &str) -> bool {
    matches!(
        &element.kind,
        ElementKind::RuleCall(RuleCall {
            name: target,
            arguments: None,
            precedence: None,
        }) if target == name && element.quantifier == Quantifier::One
    )
}

fn operator_symbols(
    element: &Element,
    quantifier: Quantifier,
) -> Result<BTreeSet<OperatorSymbol>, String> {
    if element.quantifier != quantifier {
        return Err("operator repetition does not match a canonical ladder shape".to_owned());
    }
    match &element.kind {
        ElementKind::Block(block) => {
            if !block.options.is_empty() {
                return Err("options on an operator group are not supported".to_owned());
            }
            if block.alternatives.is_empty() {
                return Err("operator group must not be empty".to_owned());
            }
            let mut symbols = BTreeSet::new();
            for alternative in &block.alternatives {
                if !alternative.options.is_empty()
                    || alternative.label.is_some()
                    || !alternative.commands.is_empty()
                {
                    return Err("operator alternatives must be plain token sets".to_owned());
                }
                let [operator] = alternative.elements.as_slice() else {
                    return Err("operator alternative must contain one token set".to_owned());
                };
                if operator.quantifier != Quantifier::One {
                    return Err("nested operator repetition is not supported".to_owned());
                }
                symbols.extend(operator_atom_symbols(&operator.kind)?);
            }
            Ok(symbols)
        }
        kind => operator_atom_symbols(kind),
    }
}

fn operator_atom_symbols(kind: &ElementKind) -> Result<BTreeSet<OperatorSymbol>, String> {
    match kind {
        ElementKind::Terminal(terminal) => {
            let symbol = terminal_symbol(terminal)?;
            Ok(BTreeSet::from([symbol]))
        }
        ElementKind::Set {
            inverted: false,
            elements,
        } => elements
            .iter()
            .map(|element| match element {
                SetElement::Terminal { value, options, .. } if options.is_empty() => {
                    terminal_symbol(value)
                }
                SetElement::Terminal { .. } | SetElement::Range { .. } => Err(
                    "operator sets may contain only plain token references or literals".to_owned(),
                ),
            })
            .collect(),
        ElementKind::Set { inverted: true, .. } => {
            Err("inverted operator sets are not finite".to_owned())
        }
        ElementKind::RuleCall(_)
        | ElementKind::Range(..)
        | ElementKind::Block(_)
        | ElementKind::Action { .. }
        | ElementKind::Predicate { .. }
        | ElementKind::Epsilon => Err("operator is not a finite token set".to_owned()),
    }
}

fn terminal_symbol(terminal: &Terminal) -> Result<OperatorSymbol, String> {
    match terminal {
        Terminal::Literal(literal) => get_string_from_grammar_string_literal(literal)
            .map(OperatorSymbol::Literal)
            .ok_or_else(|| format!("invalid operator literal {literal}")),
        Terminal::Token(token) => Ok(OperatorSymbol::Token(token.clone())),
        Terminal::LexerCharSet(_) | Terminal::Wildcard | Terminal::Eof => {
            Err("operator is not a parser token or literal".to_owned())
        }
    }
}

fn incoming_callers(
    unit: &GrammarUnit,
    rules_by_name: &BTreeMap<String, RuleId>,
) -> BTreeMap<RuleId, BTreeSet<RuleId>> {
    let mut incoming = BTreeMap::<RuleId, BTreeSet<RuleId>>::new();
    for rule in &unit.rules {
        visit_elements(&rule.block, &mut |element| {
            if let ElementKind::RuleCall(call) = &element.kind
                && let Some(target) = rules_by_name.get(&call.name)
                && *target != rule.id
            {
                incoming.entry(*target).or_default().insert(rule.id);
            }
        });
    }
    incoming
}

fn observed_rule_contexts(
    unit: &GrammarUnit,
    rules_by_name: &BTreeMap<String, RuleId>,
) -> BTreeSet<RuleId> {
    let mut observed = BTreeSet::new();
    for action in &unit.actions {
        collect_action_rule_references(&action.body, rules_by_name, &mut observed);
    }
    for rule in &unit.rules {
        for action in &rule.actions {
            collect_action_rule_references(&action.body, rules_by_name, &mut observed);
        }
        for handler in &rule.catches {
            collect_action_rule_references(&handler.body, rules_by_name, &mut observed);
        }
        if let Some(action) = &rule.finally_action {
            collect_action_rule_references(&action.body, rules_by_name, &mut observed);
        }
        visit_elements(&rule.block, &mut |element| match &element.kind {
            ElementKind::RuleCall(call) => {
                if let Some(arguments) = &call.arguments {
                    collect_action_rule_references(arguments, rules_by_name, &mut observed);
                }
            }
            ElementKind::Action { body, .. } => {
                collect_action_rule_references(body, rules_by_name, &mut observed);
            }
            ElementKind::Predicate { body, fail, .. } => {
                collect_action_rule_references(body, rules_by_name, &mut observed);
                if let Some(fail) = fail {
                    collect_action_rule_references(fail, rules_by_name, &mut observed);
                }
            }
            ElementKind::Terminal(_)
            | ElementKind::Range(..)
            | ElementKind::Set { .. }
            | ElementKind::Block(_)
            | ElementKind::Epsilon => {}
        });
    }
    observed
}

fn collect_action_rule_references(
    body: &str,
    rules_by_name: &BTreeMap<String, RuleId>,
    observed: &mut BTreeSet<RuleId>,
) {
    for reference in action_references(body) {
        let name = match reference.kind {
            ActionReferenceKind::Attribute { name, .. }
            | ActionReferenceKind::Qualified { name, .. } => Some(name),
            ActionReferenceKind::NonLocal { rule, .. } => Some(rule),
        };
        if let Some(rule) = name.and_then(|name| rules_by_name.get(name)) {
            observed.insert(*rule);
        }
    }
}

fn visit_elements(block: &Block, visitor: &mut impl FnMut(&Element)) {
    for alternative in &block.alternatives {
        for element in &alternative.elements {
            visitor(element);
            if let ElementKind::Block(nested) = &element.kind {
                visit_elements(nested, visitor);
            }
        }
    }
}

#[derive(Debug)]
struct OutputAlternative {
    source_rule: RuleId,
    source_rule_name: String,
    source_alternative: usize,
    source: Alternative,
    elements: Vec<Element>,
    right_associative: bool,
    tighter_operand: Option<usize>,
    label_hint: String,
}

fn apply_plan(
    input: &TransformContext<'_>,
    unit: &mut GrammarUnit,
    plan: LadderPlan,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> TransformCandidateReport {
    let hub = plan.rungs[0].rule.clone();
    let included_names = plan
        .rungs
        .iter()
        .map(|rung| rung.rule.name.clone())
        .collect::<BTreeSet<_>>();
    let mut outputs = output_alternatives(&plan);
    set_tighter_operand_precedences(&mut outputs, &included_names);
    let mut label_allocator = AlternativeLabelAllocator::new(unit, &plan, &hub.name);
    let mut cloner = TransformCloner {
        ids,
        provenance,
        pass: input.id,
    };
    let mut target_alternatives = BTreeMap::<(RuleId, usize), Vec<String>>::new();
    let mut label_mappings = Vec::new();
    let mut rewritten = Vec::with_capacity(outputs.len());
    for output in &mut outputs {
        let (label, renamed) = label_allocator.allocate(output);
        if let Some((source_label, target_label)) = renamed {
            label_mappings.push(TransformLabelMapping {
                source_rule: output.source_rule_name.clone(),
                source_label,
                source_span: output
                    .source
                    .label
                    .as_ref()
                    .map_or_else(|| output.source.span.clone(), |label| label.span.clone()),
                target_label,
            });
        }
        target_alternatives
            .entry((output.source_rule, output.source_alternative))
            .or_default()
            .push(label.value.clone());
        let mut alternative = cloner.alternative(output, label);
        replace_ladder_calls(&mut alternative.elements, &included_names, &hub.name);
        rewritten.push(alternative);
    }
    map_delegating_alternatives(&plan, &rewritten[0], &mut target_alternatives);

    let rewritten_ids = rewritten
        .iter()
        .map(|alternative| ModelNodeId::Alternative(alternative.id))
        .collect::<Vec<_>>();
    record_plan_provenance(provenance, input.id, &plan, hub.id, &rewritten_ids);
    let alternative_mappings = alternative_mappings(&plan, &target_alternatives);
    let projection = projection(&plan);
    let grouping_changes = plan
        .rungs
        .iter()
        .filter(|rung| rung.changes_grouping())
        .map(|rung| rung.rule.name.clone())
        .collect::<Vec<_>>();
    let removed_rules = plan.rungs[1..]
        .iter()
        .map(|rung| rung.rule.name.clone())
        .collect::<Vec<_>>();

    let removed_ids = plan.rungs[1..]
        .iter()
        .map(|rung| rung.rule.id)
        .collect::<BTreeSet<_>>();
    let top = unit
        .rules
        .iter_mut()
        .find(|rule| rule.id == hub.id)
        .expect("discovered ladder entry remains in its grammar");
    top.block.alternatives = rewritten;
    unit.rules.retain(|rule| !removed_ids.contains(&rule.id));

    TransformCandidateReport {
        pass: input.id,
        grammar: unit.name.clone(),
        entry_rule: hub.name.clone(),
        source_span: hub.span,
        status: if input.report_only {
            TransformCandidateStatus::Eligible
        } else {
            TransformCandidateStatus::Applied
        },
        reason: format!(
            "collapsed {} precedence rungs into {}",
            plan.rungs.len(),
            hub.name
        ),
        rungs: plan
            .rungs
            .iter()
            .map(|rung| rung.rule.name.clone())
            .collect(),
        boundary_rule: Some(plan.boundary_rule),
        projection: Some(projection),
        removed_rules,
        alternatives: alternative_mappings,
        labels: label_mappings,
        grouping_changes,
    }
}

fn output_alternatives(plan: &LadderPlan) -> Vec<OutputAlternative> {
    let bottom = plan
        .rungs
        .last()
        .expect("a ladder contains at least two rungs");
    let base = &bottom.rule.block.alternatives[bottom.base_alternative];
    let mut outputs = vec![OutputAlternative {
        source_rule: bottom.rule.id,
        source_rule_name: bottom.rule.name.clone(),
        source_alternative: bottom.base_alternative,
        source: base.clone(),
        elements: vec![base.elements[0].clone()],
        right_associative: false,
        tighter_operand: None,
        label_hint: "base".to_owned(),
    }];
    for rung in plan.rungs.iter().rev() {
        append_rung_outputs(rung, &mut outputs);
    }
    outputs
}

fn append_rung_outputs(rung: &Rung, outputs: &mut Vec<OutputAlternative>) {
    match &rung.kind {
        RungKind::Delegation => {}
        RungKind::Star { loop_element } => {
            let source = &rung.rule.block.alternatives[0];
            let ElementKind::Block(loop_block) = &source.elements[*loop_element].kind else {
                unreachable!("star rung was structurally classified");
            };
            let mut leading = source.elements[0].clone();
            let ElementKind::RuleCall(call) = &mut leading.kind else {
                unreachable!("star rung starts with its base-rule call");
            };
            call.name.clone_from(&rung.rule.name);
            let mut elements = vec![leading];
            elements.extend(loop_block.alternatives[0].elements.clone());
            outputs.push(output_from_source(rung, 0, elements, false, None, "loop"));
        }
        RungKind::Direct {
            recursive_alternatives,
        } => {
            for index in recursive_alternatives {
                let source = &rung.rule.block.alternatives[*index];
                outputs.push(output_from_source(
                    rung,
                    *index,
                    source.elements.clone(),
                    association_is_right(source),
                    None,
                    "operator",
                ));
            }
        }
        RungKind::RightTail { tail_element } => {
            let source = &rung.rule.block.alternatives[0];
            let ElementKind::Block(tail) = &source.elements[*tail_element].kind else {
                unreachable!("right tail was structurally classified");
            };
            let mut elements = vec![source.elements[0].clone()];
            elements.extend(tail.alternatives[0].elements.clone());
            let tighter_operand = (tail.alternatives[0].elements.len() == 4).then_some(2);
            outputs.push(output_from_source(
                rung,
                0,
                elements,
                true,
                tighter_operand,
                "right",
            ));
        }
        RungKind::Prefix {
            prefix_alternatives,
        } => {
            for index in prefix_alternatives {
                let source = &rung.rule.block.alternatives[*index];
                outputs.push(output_from_source(
                    rung,
                    *index,
                    source.elements.clone(),
                    false,
                    None,
                    "prefix",
                ));
            }
        }
    }
}

fn output_from_source(
    rung: &Rung,
    source_alternative: usize,
    elements: Vec<Element>,
    right_associative: bool,
    tighter_operand: Option<usize>,
    label_hint: &str,
) -> OutputAlternative {
    let source = &rung.rule.block.alternatives[source_alternative];
    OutputAlternative {
        source_rule: rung.rule.id,
        source_rule_name: rung.rule.name.clone(),
        source_alternative,
        source: source.clone(),
        elements,
        right_associative,
        tighter_operand,
        label_hint: label_hint.to_owned(),
    }
}

fn set_tighter_operand_precedences(
    outputs: &mut [OutputAlternative],
    included_names: &BTreeSet<String>,
) {
    let alternative_count = outputs.len();
    for (index, output) in outputs.iter_mut().enumerate() {
        let Some(element_index) = output.tighter_operand else {
            continue;
        };
        let ElementKind::RuleCall(call) = &mut output.elements[element_index].kind else {
            unreachable!("a ternary middle operand is a rule call");
        };
        if included_names.contains(&call.name) {
            let operator_precedence = u32::try_from(alternative_count - index)
                .expect("precedence ladder alternative count exceeds u32");
            call.precedence = Some(
                operator_precedence
                    .checked_add(1)
                    .expect("precedence ladder precedence overflow"),
            );
        }
    }
}

fn association_is_right(alternative: &Alternative) -> bool {
    alternative
        .options
        .iter()
        .any(|option| option.name.value == "assoc" && option.value.value == "right")
}

fn map_delegating_alternatives(
    plan: &LadderPlan,
    primary: &Alternative,
    mappings: &mut BTreeMap<(RuleId, usize), Vec<String>>,
) {
    let primary_label = primary
        .label
        .as_ref()
        .expect("all collapsed alternatives are labeled")
        .value
        .clone();
    for rung in &plan.rungs {
        let labels = mappings
            .entry((rung.rule.id, rung.base_alternative))
            .or_default();
        if !labels.contains(&primary_label) {
            labels.push(primary_label.clone());
        }
    }
}

fn replace_ladder_calls(elements: &mut [Element], names: &BTreeSet<String>, hub: &str) {
    for element in elements {
        match &mut element.kind {
            ElementKind::RuleCall(call) if names.contains(&call.name) => {
                call.name.clear();
                call.name.push_str(hub);
            }
            ElementKind::Block(block) => {
                for alternative in &mut block.alternatives {
                    replace_ladder_calls(&mut alternative.elements, names, hub);
                }
            }
            ElementKind::Terminal(_)
            | ElementKind::RuleCall(_)
            | ElementKind::Range(..)
            | ElementKind::Set { .. }
            | ElementKind::Action { .. }
            | ElementKind::Predicate { .. }
            | ElementKind::Epsilon => {}
        }
    }
}

struct TransformCloner<'a> {
    ids: &'a mut ModelIdAllocator,
    provenance: &'a mut ProvenanceIndex,
    pass: super::model::TransformId,
}

impl TransformCloner<'_> {
    fn alternative(&mut self, output: &OutputAlternative, label: Authored<String>) -> Alternative {
        let id = self.ids.alternative();
        self.record(
            ModelNodeId::Alternative(id),
            ModelNodeId::Alternative(output.source.id),
        );
        let options = if output.right_associative {
            vec![right_association_option(&output.source)]
        } else {
            output.source.options.clone()
        };
        Alternative {
            id,
            elements: output
                .elements
                .iter()
                .map(|element| self.element(element))
                .collect(),
            label: Some(label),
            options,
            commands: Vec::new(),
            syntax: output.source.syntax,
            span: output.source.span.clone(),
        }
    }

    fn element(&mut self, source: &Element) -> Element {
        let mut cloned = source.clone();
        cloned.id = self.ids.element();
        cloned.label = source.label.as_ref().map(|label| self.label(label));
        cloned.kind = match &source.kind {
            ElementKind::Block(block) => ElementKind::Block(Block {
                alternatives: block
                    .alternatives
                    .iter()
                    .map(|alternative| self.nested_alternative(alternative))
                    .collect(),
                options: block.options.clone(),
                syntax: block.syntax,
                span: block.span.clone(),
            }),
            ElementKind::Action { id, body } => {
                let cloned_id = self.ids.action();
                self.record(ModelNodeId::Action(cloned_id), ModelNodeId::Action(*id));
                ElementKind::Action {
                    id: cloned_id,
                    body: body.clone(),
                }
            }
            ElementKind::Predicate {
                id,
                body,
                fail,
                precedence,
            } => {
                let cloned_id = self.ids.predicate();
                self.record(
                    ModelNodeId::Predicate(cloned_id),
                    ModelNodeId::Predicate(*id),
                );
                ElementKind::Predicate {
                    id: cloned_id,
                    body: body.clone(),
                    fail: fail.clone(),
                    precedence: *precedence,
                }
            }
            kind => kind.clone(),
        };
        self.record(
            ModelNodeId::Element(cloned.id),
            ModelNodeId::Element(source.id),
        );
        cloned
    }

    fn nested_alternative(&mut self, source: &Alternative) -> Alternative {
        let id = self.ids.alternative();
        self.record(
            ModelNodeId::Alternative(id),
            ModelNodeId::Alternative(source.id),
        );
        Alternative {
            id,
            elements: source
                .elements
                .iter()
                .map(|element| self.element(element))
                .collect(),
            label: source.label.clone(),
            options: source.options.clone(),
            commands: source.commands.clone(),
            syntax: source.syntax,
            span: source.span.clone(),
        }
    }

    fn label(&mut self, source: &Label) -> Label {
        let mut cloned = source.clone();
        cloned.id = self.ids.label();
        self.record(ModelNodeId::Label(cloned.id), ModelNodeId::Label(source.id));
        cloned
    }

    fn record(&mut self, destination: ModelNodeId, source: ModelNodeId) {
        let mut origins = self.provenance.origins(source).to_vec();
        origins.push(Origin::OptionalTransform {
            pass: self.pass,
            inputs: Box::new([source]),
        });
        self.provenance.record_model(destination, origins);
    }
}

fn right_association_option(source: &Alternative) -> OptionDecl {
    OptionDecl {
        name: Authored {
            value: "assoc".to_owned(),
            syntax: source.syntax,
            span: source.span.clone(),
        },
        value: Authored {
            value: "right".to_owned(),
            syntax: source.syntax,
            span: source.span.clone(),
        },
    }
}

struct AlternativeLabelAllocator {
    reserved: BTreeSet<String>,
    used: BTreeSet<String>,
    rule_names: BTreeSet<String>,
    hub: String,
    serial: usize,
}

impl AlternativeLabelAllocator {
    fn new(unit: &GrammarUnit, plan: &LadderPlan, hub: &str) -> Self {
        let removed = plan
            .rungs
            .iter()
            .map(|rung| rung.rule.id)
            .collect::<BTreeSet<_>>();
        let reserved = unit
            .rules
            .iter()
            .filter(|rule| !removed.contains(&rule.id))
            .flat_map(|rule| &rule.block.alternatives)
            .filter_map(|alternative| alternative.label.as_ref())
            .map(|label| ascii_lowercase(&label.value))
            .collect();
        Self {
            reserved,
            used: BTreeSet::new(),
            rule_names: unit.rules.iter().map(|rule| rule.name.clone()).collect(),
            hub: hub.to_owned(),
            serial: 0,
        }
    }

    fn allocate(
        &mut self,
        output: &OutputAlternative,
    ) -> (Authored<String>, Option<(String, String)>) {
        if let Some(source) = &output.source.label
            && self.available(&source.value)
        {
            self.reserve(&source.value);
            return (source.clone(), None);
        }
        let source_label = output
            .source
            .label
            .as_ref()
            .map(|label| label.value.clone());
        loop {
            self.serial += 1;
            let candidate = format!(
                "{}{}{}{}",
                pascal_identifier(&self.hub),
                pascal_identifier(&output.source_rule_name),
                pascal_identifier(&output.label_hint),
                self.serial
            );
            if self.available(&candidate) {
                self.reserve(&candidate);
                let label = Authored {
                    value: candidate.clone(),
                    syntax: output.source.syntax,
                    span: output.source.span.clone(),
                };
                return (label, source_label.map(|source| (source, candidate)));
            }
        }
    }

    fn available(&self, value: &str) -> bool {
        let normalized = ascii_lowercase(value);
        !self.reserved.contains(&normalized)
            && !self.used.contains(&normalized)
            && !self.rule_names.contains(&decapitalize(value))
    }

    fn reserve(&mut self, value: &str) {
        self.used.insert(ascii_lowercase(value));
    }
}

fn pascal_identifier(value: &str) -> String {
    let mut output = String::new();
    let mut capitalize = true;
    for character in value.chars() {
        if character == '_' {
            capitalize = true;
        } else if capitalize {
            output.extend(character.to_uppercase());
            capitalize = false;
        } else {
            output.push(character);
        }
    }
    output
}

fn decapitalize(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    first.to_lowercase().chain(characters).collect::<String>()
}

fn ascii_lowercase(value: &str) -> String {
    value
        .bytes()
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

fn record_plan_provenance(
    provenance: &mut ProvenanceIndex,
    pass: super::model::TransformId,
    plan: &LadderPlan,
    hub: RuleId,
    replacements: &[ModelNodeId],
) {
    let inputs = plan
        .rungs
        .iter()
        .map(|rung| ModelNodeId::Rule(rung.rule.id))
        .collect::<Vec<_>>();
    let mut origins = inputs
        .iter()
        .flat_map(|input| provenance.origins(*input).iter().cloned())
        .collect::<Vec<_>>();
    origins.push(Origin::OptionalTransform {
        pass,
        inputs: inputs.into_boxed_slice(),
    });
    provenance.record_model(ModelNodeId::Rule(hub), origins);
    for rung in &plan.rungs {
        if rung.rule.id != hub {
            provenance.tombstone(
                rung.rule.syntax,
                Tombstone {
                    phase: "optional-transform",
                    reason: "precedence ladder rule collapsed into its entry rule",
                    replacements: Box::new([ModelNodeId::Rule(hub)]),
                },
            );
        }
        for alternative in &rung.rule.block.alternatives {
            provenance.tombstone(
                alternative.syntax,
                Tombstone {
                    phase: "optional-transform",
                    reason: "precedence ladder alternative migrated",
                    replacements: replacements.to_vec().into_boxed_slice(),
                },
            );
        }
    }
}

fn alternative_mappings(
    plan: &LadderPlan,
    targets: &BTreeMap<(RuleId, usize), Vec<String>>,
) -> Vec<TransformAlternativeMapping> {
    let hub = &plan.rungs[0].rule.name;
    let mut mappings = Vec::new();
    for rung in &plan.rungs {
        for (index, alternative) in rung.rule.block.alternatives.iter().enumerate() {
            let mut target_alternatives = targets
                .get(&(rung.rule.id, index))
                .cloned()
                .unwrap_or_default();
            target_alternatives.sort();
            target_alternatives.dedup();
            mappings.push(TransformAlternativeMapping {
                source_rule: rung.rule.name.clone(),
                source_alternative: index + 1,
                source_span: alternative.span.clone(),
                target_rule: hub.clone(),
                target_alternatives,
            });
        }
    }
    mappings
}

fn projection(plan: &LadderPlan) -> TransformProjection {
    let decisions = plan.rungs.iter().map(Rung::decision_count).sum::<usize>();
    TransformProjection {
        contexts_per_operand_before: plan.rungs.len() + 1,
        contexts_per_operand_after: 2,
        precedence_decisions_before: decisions,
        precedence_decisions_after: usize::from(decisions > 0),
    }
}

fn declined_report(
    input: &TransformContext<'_>,
    unit: &GrammarUnit,
    declined: DeclinedLadder,
) -> TransformCandidateReport {
    TransformCandidateReport {
        pass: input.id,
        grammar: unit.name.clone(),
        entry_rule: declined.entry_rule.name,
        source_span: declined.entry_rule.span,
        status: TransformCandidateStatus::Declined,
        reason: declined.reason,
        rungs: declined.rungs,
        boundary_rule: declined.boundary_rule,
        projection: None,
        removed_rules: Vec::new(),
        alternatives: Vec::new(),
        labels: Vec::new(),
        grouping_changes: Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;
    use crate::grammar::frontend::{SourceId, parse_source};
    use crate::grammar::model::GrammarId;
    use crate::grammar::syntax::parse_grammar_unit;
    use crate::grammar::transform::{TransformGrammar, TransformRegistry};

    const CEL_LADDER: &str = r#"
parser grammar P;
expr
    : e=conditionalOr (op='?' e1=conditionalOr ':' e2=expr)?
    ;
conditionalOr
    : e=conditionalAnd (ops+='||' e1+=conditionalAnd)*
    ;
conditionalAnd
    : e=relation (ops+='&&' e1+=relation)*
    ;
relation
    : calc
    | relation op=('<'|'<='|'>='|'>'|'=='|'!='|'in') relation
    ;
calc
    : unary
    | calc op=('*'|'/'|'%') calc
    | calc op=('+'|'-') calc
    ;
unary
    : member             # MemberExpr
    | (ops+='!')+ member # LogicalNot
    | (ops+='-')+ member # Negate
    ;
member : INT ;
"#;

    #[test]
    fn collapses_all_canonical_cel_rungs_and_is_idempotent() {
        let (mut grammar, mut ids) = fixture(CEL_LADDER);
        let mut registry = TransformRegistry::default();
        registry.push(CollapsePrecedenceLadders);
        let report = registry
            .run(&mut grammar, &mut ids, false)
            .expect("CEL ladder should collapse");

        insta::assert_debug_snapshot!(
            "cel_ladder_collapse",
            (
                summarize(&grammar.units[0]),
                &report.entries,
                &report.candidates,
            )
        );
        assert_eq!(
            grammar.units[0]
                .rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect::<Vec<_>>(),
            ["expr", "member"]
        );
        assert_eq!(
            report.candidates[0].grouping_changes,
            ["conditionalOr", "conditionalAnd"]
        );
        let ternary = grammar.units[0].rules[0]
            .block
            .alternatives
            .iter()
            .find(|alternative| association_is_right(alternative))
            .expect("collapsed ternary alternative");
        let recursive_calls = ternary
            .elements
            .iter()
            .filter_map(|element| match &element.kind {
                ElementKind::RuleCall(call) if call.name == "expr" => Some(call.precedence),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(recursive_calls, [None, Some(2), None]);

        let second = registry
            .run(&mut grammar, &mut ids, false)
            .expect("a second pass should remain valid");
        assert!(!second.entries[0].changed);
        assert!(second.candidates.is_empty());
    }

    #[test]
    fn report_only_projects_the_rewrite_without_mutating_the_grammar() {
        let (mut grammar, mut ids) = fixture(CEL_LADDER);
        let before = grammar.units.clone();
        let mut registry = TransformRegistry::default();
        registry.push(CollapsePrecedenceLadders);
        let report = registry
            .run(&mut grammar, &mut ids, true)
            .expect("dry-run should analyze the ladder");

        assert_eq!(grammar.units, before);
        assert_eq!(
            report.candidates[0].status,
            TransformCandidateStatus::Eligible
        );
        assert_eq!(report.entries[0].before.rules, 7);
        assert_eq!(report.entries[0].after.rules, 2);
    }

    #[test]
    fn external_middle_reference_creates_a_compatibility_boundary() {
        let (mut grammar, mut ids) = fixture(
            r#"
parser grammar P;
top : middle ;
other : middle ;
middle : low ('+' low)* ;
low : atom ('*' atom)* ;
atom : INT ;
"#,
        );
        let mut registry = TransformRegistry::default();
        registry.push(CollapsePrecedenceLadders);
        let report = registry
            .run(&mut grammar, &mut ids, false)
            .expect("the lower segment should collapse");

        assert_eq!(
            grammar.units[0]
                .rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect::<Vec<_>>(),
            ["top", "other", "middle", "atom"]
        );
        assert_eq!(report.candidates[0].entry_rule, "middle");
        assert_eq!(report.candidates[0].removed_rules, ["low"]);
        let middle = grammar.units[0]
            .rules
            .iter()
            .find(|rule| rule.name == "middle")
            .expect("collapsed segment entry remains");
        let calls = middle
            .block
            .alternatives
            .iter()
            .map(|alternative| {
                let mut calls = Vec::new();
                collect_calls(&alternative.elements, &mut calls);
                calls
            })
            .collect::<Vec<_>>();
        assert_eq!(
            calls,
            [
                vec!["atom"],
                vec!["middle", "atom"],
                vec!["middle", "middle"],
            ]
        );
    }

    #[test]
    fn supports_delegation_base_rhs_and_binary_right_tail_rungs() {
        let (mut grammar, mut ids) = fixture(
            r#"
parser grammar P;
entry : forwarded ;
forwarded : direct ;
direct
    : tail
    | direct '+' tail
    ;
tail : atom ('^' tail)? ;
atom : INT ;
"#,
        );
        let mut registry = TransformRegistry::default();
        registry.push(CollapsePrecedenceLadders);
        let report = registry
            .run(&mut grammar, &mut ids, false)
            .expect("all canonical rungs should collapse");

        assert_eq!(
            report.candidates[0].rungs,
            ["entry", "forwarded", "direct", "tail"]
        );
        assert_eq!(
            grammar.units[0]
                .rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect::<Vec<_>>(),
            ["entry", "atom"]
        );
        let alternatives = &grammar.units[0].rules[0].block.alternatives;
        assert_eq!(alternatives.len(), 3);
        assert!(
            alternatives[1]
                .options
                .iter()
                .any(|option| { option.name.value == "assoc" && option.value.value == "right" })
        );
        assert!(alternatives[2].options.is_empty());
    }

    #[test]
    fn overlapping_infix_sets_decline_the_whole_candidate() {
        let (mut grammar, mut ids) = fixture(
            r#"
parser grammar P;
high : low ('+' low)* ;
low : atom ('+' atom)* ;
atom : INT ;
"#,
        );
        let before = grammar.units.clone();
        let mut registry = TransformRegistry::default();
        registry.push(CollapsePrecedenceLadders);
        let report = registry
            .run(&mut grammar, &mut ids, false)
            .expect("declining a candidate is not a compilation error");

        assert_eq!(grammar.units, before);
        assert!(!report.entries[0].changed);
        assert_eq!(
            report.candidates[0].status,
            TransformCandidateStatus::Declined
        );
        assert!(
            report.candidates[0]
                .reason
                .contains("operator token sets overlap")
        );
    }

    #[test]
    fn mixed_literal_and_symbolic_operator_sets_fail_closed() {
        let (mut grammar, mut ids) = fixture(
            r#"
parser grammar P;
high : low (PLUS low)* ;
low : atom ('+' atom)* ;
atom : INT ;
"#,
        );
        let before = grammar.units.clone();
        let mut registry = TransformRegistry::default();
        registry.push(CollapsePrecedenceLadders);
        let report = registry
            .run(&mut grammar, &mut ids, false)
            .expect("declining an ambiguous candidate is not a compilation error");

        assert_eq!(grammar.units, before);
        assert_eq!(
            report.candidates[0].status,
            TransformCandidateStatus::Declined
        );
        assert!(
            report.candidates[0]
                .reason
                .contains("symbolic-token/literal overlap cannot be disproved")
        );
    }

    #[test]
    fn mutual_recursion_with_an_external_entry_is_not_a_linear_ladder() {
        let (mut grammar, mut ids) = fixture(
            r#"
parser grammar P;
start : a EOF ;
a : b ('+' b)* ;
b : a ('*' a)* ;
"#,
        );
        let before = grammar.units.clone();
        let mut registry = TransformRegistry::default();
        registry.push(CollapsePrecedenceLadders);
        let report = registry
            .run(&mut grammar, &mut ids, false)
            .expect("declining mutual recursion is not a compilation error");

        assert_eq!(grammar.units, before);
        assert_eq!(
            report.candidates[0].status,
            TransformCandidateStatus::Declined
        );
        assert!(
            report.candidates[0]
                .reason
                .contains("mutual-recursion cycle")
        );
    }

    #[test]
    fn right_association_on_a_nonrecursive_rhs_is_declined() {
        let (mut grammar, mut ids) = fixture(
            r#"
parser grammar P;
entry : power ;
power
    : atom
    | <assoc=right> power '^' atom
    ;
atom : INT ;
"#,
        );
        let before = grammar.units.clone();
        let mut registry = TransformRegistry::default();
        registry.push(CollapsePrecedenceLadders);
        let report = registry
            .run(&mut grammar, &mut ids, false)
            .expect("declining ambiguous associativity is not a compilation error");

        assert_eq!(grammar.units, before);
        assert_eq!(
            report.candidates[0].status,
            TransformCandidateStatus::Declined
        );
        assert!(
            report.candidates[0]
                .reason
                .contains("requires a recursive right operand")
        );
    }

    #[test]
    fn colliding_alternative_labels_are_renamed_and_reported() {
        let (mut grammar, mut ids) = fixture(
            r#"
parser grammar P;
high : low ('+' low)* # Shared ;
low : atom ('*' atom)* # Shared ;
atom : INT ;
"#,
        );
        let mut registry = TransformRegistry::default();
        registry.push(CollapsePrecedenceLadders);
        let report = registry
            .run(&mut grammar, &mut ids, false)
            .expect("label collisions should be migrated");

        let labels = grammar.units[0].rules[0]
            .block
            .alternatives
            .iter()
            .map(|alternative| {
                alternative
                    .label
                    .as_ref()
                    .expect("collapsed alternatives are labeled")
                    .value
                    .clone()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(labels.len(), 3);
        assert!(labels.contains("Shared"));
        assert_eq!(report.candidates[0].labels.len(), 2);
        assert!(
            report.candidates[0]
                .labels
                .iter()
                .all(|mapping| mapping.source_label == "Shared")
        );
    }

    #[test]
    fn action_and_rule_argument_references_observe_rule_contexts() {
        let (grammar, _) = fixture(
            r#"
parser grammar P;
@members { let _ = $actionRule; }
caller : sink[$argumentRule.text] ;
sink[boolean enabled] : INT ;
actionRule : low ;
argumentRule : low ;
low : INT ;
"#,
        );
        let unit = &grammar.units[0];
        let rules_by_name = unit
            .rules
            .iter()
            .map(|rule| (rule.name.clone(), rule.id))
            .collect::<BTreeMap<_, _>>();
        let observed = observed_rule_contexts(unit, &rules_by_name);

        assert!(observed.contains(&rules_by_name["actionRule"]));
        assert!(observed.contains(&rules_by_name["argumentRule"]));
    }

    fn fixture(text: &str) -> (TransformGrammar, ModelIdAllocator) {
        let file = parse_source(SourceId::new(0), "P.g4", text).expect("valid grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);
        (
            TransformGrammar {
                units: vec![unit],
                provenance,
            },
            ids,
        )
    }

    fn summarize(unit: &GrammarUnit) -> Vec<(String, Vec<(String, Vec<String>)>)> {
        unit.rules
            .iter()
            .map(|rule| {
                (
                    rule.name.clone(),
                    rule.block
                        .alternatives
                        .iter()
                        .map(|alternative| {
                            let mut calls = Vec::new();
                            collect_calls(&alternative.elements, &mut calls);
                            (
                                alternative
                                    .label
                                    .as_ref()
                                    .map_or_else(String::new, |label| label.value.clone()),
                                calls,
                            )
                        })
                        .collect(),
                )
            })
            .collect()
    }

    fn collect_calls(elements: &[Element], calls: &mut Vec<String>) {
        for element in elements {
            match &element.kind {
                ElementKind::RuleCall(call) => calls.push(call.name.clone()),
                ElementKind::Block(block) => {
                    for alternative in &block.alternatives {
                        collect_calls(&alternative.elements, calls);
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
}
