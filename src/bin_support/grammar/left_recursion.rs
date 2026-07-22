use std::collections::BTreeSet;

use super::diagnostic::Diagnostic;
use super::model::{
    Alternative, AlternativeId, Block, Element, ElementKind, GrammarUnit, LeftRecursionInfo,
    LeftRecursiveAlternativeKind, ModelIdAllocator, ModelNodeId, Quantifier,
    RemovedLeftRecursiveLabel, Rule, RuleId,
};
use super::provenance::{LeftRecursionRole, Origin, ProvenanceIndex, Tombstone};

pub(crate) fn rewrite_immediate_left_recursion(
    units: &mut [GrammarUnit],
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut rewritten_names = BTreeSet::new();
    for unit in units.iter_mut() {
        for rule in &mut unit.rules {
            if matches!(classify_rule(rule), RuleClassification::NotLeftRecursive) {
                continue;
            }
            match rewrite_rule(rule, ids, provenance) {
                Ok(()) => {
                    rewritten_names.insert(rule.name.clone());
                }
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
        }
    }
    if diagnostics.is_empty() {
        for unit in units.iter_mut() {
            for rule in &mut unit.rules {
                update_external_calls(&mut rule.block, &rewritten_names);
            }
        }
    }
    diagnostics
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AlternativeClass {
    Primary,
    Prefix,
    Binary,
    Suffix,
    Nonconforming,
}

#[derive(Debug)]
enum RuleClassification {
    NotLeftRecursive,
    LeftRecursive(Vec<AlternativeClass>),
}

fn classify_rule(rule: &Rule) -> RuleClassification {
    let has_immediate = rule.block.alternatives.iter().any(|alternative| {
        alternative
            .elements
            .first()
            .is_some_and(|element| is_self_call(element, &rule.name))
    });
    if !has_immediate {
        return RuleClassification::NotLeftRecursive;
    }
    RuleClassification::LeftRecursive(
        rule.block
            .alternatives
            .iter()
            .map(|alternative| classify_alternative(alternative, &rule.name))
            .collect(),
    )
}

fn classify_alternative(alternative: &Alternative, rule_name: &str) -> AlternativeClass {
    let Some(first) = alternative.elements.first() else {
        return AlternativeClass::Primary;
    };
    if alternative
        .elements
        .iter()
        .filter_map(|element| self_call(element, rule_name))
        .any(|call| call.arguments.is_some())
    {
        return AlternativeClass::Nonconforming;
    }
    let first_recursive = is_self_call(first, rule_name);
    let last_significant = alternative
        .elements
        .iter()
        .rposition(|element| !is_epsilon_element(element));
    let last_recursive = last_significant
        .and_then(|index| alternative.elements.get(index))
        .is_some_and(|element| is_self_call(element, rule_name));
    if first_recursive {
        let Some(last) = last_significant else {
            return AlternativeClass::Nonconforming;
        };
        if last == 0 {
            return AlternativeClass::Nonconforming;
        }
        if last_recursive {
            AlternativeClass::Binary
        } else {
            AlternativeClass::Suffix
        }
    } else if last_recursive {
        AlternativeClass::Prefix
    } else {
        AlternativeClass::Primary
    }
}

fn rewrite_rule(
    rule: &mut Rule,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> Result<(), Diagnostic> {
    let RuleClassification::LeftRecursive(classes) = classify_rule(rule) else {
        return Ok(());
    };
    if classes.contains(&AlternativeClass::Nonconforming) {
        return Err(Diagnostic::error(
            "G4R001",
            rule.span.clone(),
            format!(
                "rule {} is left recursive but doesn't conform to a pattern ANTLR can handle",
                rule.name
            ),
        ));
    }
    if !classes
        .iter()
        .any(|class| matches!(class, AlternativeClass::Primary | AlternativeClass::Prefix))
    {
        return Err(Diagnostic::error(
            "G4R002",
            rule.span.clone(),
            format!(
                "left recursive rule {} must contain an alternative which is not left recursive",
                rule.name
            ),
        ));
    }

    let original_alternatives = std::mem::take(&mut rule.block.alternatives);
    let alternative_count = original_alternatives.len();
    let mut info = LeftRecursionInfo::default();
    let mut primary = Vec::new();
    let mut operators = Vec::new();
    for (index, (source, class)) in original_alternatives
        .iter()
        .zip(classes.iter().copied())
        .enumerate()
    {
        let alt_number = index + 1;
        let precedence = u32::try_from(alternative_count - index)
            .expect("left-recursive alternative count exceeds u32");
        let rewritten = match class {
            AlternativeClass::Primary | AlternativeClass::Prefix => {
                let mut cloner = LeftRecursionCloner {
                    ids,
                    provenance,
                    rule: rule.id,
                    original_alt: source.id,
                    role: LeftRecursionRole::Primary,
                };
                let mut alternative = cloner.alternative(source, 0);
                if class == AlternativeClass::Prefix {
                    set_rightmost_precedence(&mut alternative.elements, &rule.name, precedence);
                }
                if primary.is_empty() {
                    prepend_result_action(&mut alternative, rule.id, source.id, ids, provenance);
                }
                primary.push(alternative);
                primary.last().expect("just pushed").id
            }
            AlternativeClass::Binary | AlternativeClass::Suffix => {
                if let Some(label) = source.elements[0].label.as_ref() {
                    let ElementKind::RuleCall(call) = &source.elements[0].kind else {
                        unreachable!("left-recursive operator starts with a rule call");
                    };
                    info.deleted_labels.insert(
                        label.id,
                        RemovedLeftRecursiveLabel {
                            original_alternative: source.id,
                            label: label.clone(),
                            target: call.name.clone(),
                        },
                    );
                    provenance.tombstone(
                        label.syntax,
                        Tombstone {
                            phase: "left-recursion",
                            reason: "label belonged to the removed leading recursive call",
                            replacements: Box::new([]),
                        },
                    );
                }
                let mut cloner = LeftRecursionCloner {
                    ids,
                    provenance,
                    rule: rule.id,
                    original_alt: source.id,
                    role: LeftRecursionRole::Operator,
                };
                let mut alternative = cloner.alternative(source, 1);
                if class == AlternativeClass::Binary {
                    let right_associative =
                        association(source).is_some_and(|value| value == "right");
                    let next_precedence = if right_associative {
                        precedence
                    } else {
                        precedence
                            .checked_add(1)
                            .expect("left-recursion precedence overflow")
                    };
                    set_rightmost_precedence(
                        &mut alternative.elements,
                        &rule.name,
                        next_precedence,
                    );
                }
                prepend_precedence_predicate(
                    &mut alternative,
                    rule.id,
                    source.id,
                    precedence,
                    ids,
                    provenance,
                );
                operators.push(alternative);
                operators.last().expect("just pushed").id
            }
            AlternativeClass::Nonconforming => unreachable!("checked above"),
        };
        info.original_to_rewritten.insert(source.id, rewritten);
        info.alternative_kinds.insert(
            source.id,
            match class {
                AlternativeClass::Primary => LeftRecursiveAlternativeKind::Primary,
                AlternativeClass::Prefix => LeftRecursiveAlternativeKind::Prefix,
                AlternativeClass::Binary => LeftRecursiveAlternativeKind::Binary,
                AlternativeClass::Suffix => LeftRecursiveAlternativeKind::Suffix,
                AlternativeClass::Nonconforming => unreachable!("checked above"),
            },
        );
        provenance.tombstone(
            source.syntax,
            Tombstone {
                phase: "left-recursion",
                reason: "alternative structurally rewritten",
                replacements: Box::new([ModelNodeId::Alternative(rewritten)]),
            },
        );
        let _ = alt_number;
    }

    let primary_block = Block {
        alternatives: primary,
        options: Vec::new(),
        syntax: rule.block.syntax,
        span: rule.block.span.clone(),
    };
    let operator_block = Block {
        alternatives: operators,
        options: Vec::new(),
        syntax: rule.block.syntax,
        span: rule.block.span.clone(),
    };
    let outer_alt = ids.alternative();
    let primary_element = ids.element();
    let operator_element = ids.element();
    let source_nodes = original_alternatives
        .iter()
        .map(|alternative| ModelNodeId::Alternative(alternative.id))
        .collect::<Vec<_>>();
    record_lr_synthetic(
        provenance,
        ModelNodeId::Alternative(outer_alt),
        rule.id,
        original_alternatives[0].id,
        LeftRecursionRole::Primary,
        &source_nodes,
    );
    record_lr_synthetic(
        provenance,
        ModelNodeId::Element(primary_element),
        rule.id,
        original_alternatives[0].id,
        LeftRecursionRole::Primary,
        &source_nodes,
    );
    record_lr_synthetic(
        provenance,
        ModelNodeId::Element(operator_element),
        rule.id,
        original_alternatives[0].id,
        LeftRecursionRole::Operator,
        &source_nodes,
    );
    rule.block.alternatives = vec![Alternative {
        id: outer_alt,
        elements: vec![
            Element {
                id: primary_element,
                kind: ElementKind::Block(primary_block),
                quantifier: Quantifier::One,
                label: None,
                options: Vec::new(),
                syntax: rule.syntax,
                span: rule.span.clone(),
                enclosing_span: rule.span.clone(),
            },
            Element {
                id: operator_element,
                kind: ElementKind::Block(operator_block),
                quantifier: Quantifier::ZeroOrMore { greedy: true },
                label: None,
                options: Vec::new(),
                syntax: rule.syntax,
                span: rule.span.clone(),
                enclosing_span: rule.span.clone(),
            },
        ],
        label: None,
        options: Vec::new(),
        commands: Vec::new(),
        syntax: rule.syntax,
        span: rule.span.clone(),
    }];
    provenance.record_model(
        ModelNodeId::Rule(rule.id),
        [Origin::LeftRecursion {
            rule: rule.id,
            original_alt: original_alternatives[0].id,
            role: LeftRecursionRole::Primary,
        }],
    );
    rule.left_recursion = Some(info);
    Ok(())
}

fn association(alternative: &Alternative) -> Option<&str> {
    alternative
        .options
        .iter()
        .find(|option| option.name.value == "assoc")
        .map(|option| option.value.value.as_str())
}

fn prepend_result_action(
    alternative: &mut Alternative,
    rule: RuleId,
    original_alt: AlternativeId,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) {
    let action = ids.action();
    let element = ids.element();
    record_lr_synthetic(
        provenance,
        ModelNodeId::Action(action),
        rule,
        original_alt,
        LeftRecursionRole::Primary,
        &[],
    );
    record_lr_synthetic(
        provenance,
        ModelNodeId::Element(element),
        rule,
        original_alt,
        LeftRecursionRole::Primary,
        &[],
    );
    alternative.elements.insert(
        0,
        Element {
            id: element,
            kind: ElementKind::Action {
                id: action,
                body: String::new(),
            },
            quantifier: Quantifier::One,
            label: None,
            options: Vec::new(),
            syntax: alternative.syntax,
            span: alternative.span.clone(),
            enclosing_span: alternative.span.clone(),
        },
    );
}

fn prepend_precedence_predicate(
    alternative: &mut Alternative,
    rule: RuleId,
    original_alt: AlternativeId,
    precedence: u32,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) {
    let predicate = ids.predicate();
    let element = ids.element();
    record_lr_synthetic(
        provenance,
        ModelNodeId::Predicate(predicate),
        rule,
        original_alt,
        LeftRecursionRole::PrecedencePredicate,
        &[],
    );
    record_lr_synthetic(
        provenance,
        ModelNodeId::Element(element),
        rule,
        original_alt,
        LeftRecursionRole::PrecedencePredicate,
        &[],
    );
    alternative.elements.insert(
        0,
        Element {
            id: element,
            kind: ElementKind::Predicate {
                id: predicate,
                body: String::new(),
                fail: None,
                precedence: Some(precedence),
            },
            quantifier: Quantifier::One,
            label: None,
            options: Vec::new(),
            syntax: alternative.syntax,
            span: alternative.span.clone(),
            enclosing_span: alternative.span.clone(),
        },
    );
}

fn set_rightmost_precedence(elements: &mut [Element], rule_name: &str, precedence: u32) {
    for element in elements.iter_mut().rev() {
        if let ElementKind::RuleCall(call) = &mut element.kind {
            if call.name == rule_name {
                call.precedence = Some(precedence);
                return;
            }
        }
    }
}

fn update_external_calls(block: &mut Block, rewritten_names: &BTreeSet<String>) {
    for alternative in &mut block.alternatives {
        for element in &mut alternative.elements {
            match &mut element.kind {
                ElementKind::RuleCall(call)
                    if rewritten_names.contains(&call.name) && call.precedence.is_none() =>
                {
                    call.precedence = Some(0);
                }
                ElementKind::Block(nested) => update_external_calls(nested, rewritten_names),
                _ => {}
            }
        }
    }
}

fn is_self_call(element: &Element, rule_name: &str) -> bool {
    self_call(element, rule_name).is_some()
}

fn self_call<'a>(element: &'a Element, rule_name: &str) -> Option<&'a super::model::RuleCall> {
    match &element.kind {
        ElementKind::RuleCall(call)
            if element.quantifier == Quantifier::One && call.name == rule_name =>
        {
            Some(call)
        }
        _ => None,
    }
}

const fn is_epsilon_element(element: &Element) -> bool {
    matches!(
        element.kind,
        ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon
    )
}

fn record_lr_synthetic(
    provenance: &mut ProvenanceIndex,
    destination: ModelNodeId,
    rule: RuleId,
    original_alt: AlternativeId,
    role: LeftRecursionRole,
    inputs: &[ModelNodeId],
) {
    let mut origins = inputs
        .iter()
        .flat_map(|input| provenance.origins(*input).iter().cloned())
        .collect::<Vec<_>>();
    origins.push(Origin::LeftRecursion {
        rule,
        original_alt,
        role,
    });
    provenance.record_model(destination, origins);
}

struct LeftRecursionCloner<'a> {
    ids: &'a mut ModelIdAllocator,
    provenance: &'a mut ProvenanceIndex,
    rule: RuleId,
    original_alt: AlternativeId,
    role: LeftRecursionRole,
}

impl LeftRecursionCloner<'_> {
    fn alternative(&mut self, source: &Alternative, skip: usize) -> Alternative {
        let mut cloned = source.clone();
        cloned.id = self.ids.alternative();
        cloned.elements = source.elements[skip..]
            .iter()
            .map(|element| self.element(element))
            .collect();
        self.record(
            ModelNodeId::Alternative(cloned.id),
            ModelNodeId::Alternative(source.id),
        );
        cloned
    }

    fn element(&mut self, source: &Element) -> Element {
        let mut cloned = source.clone();
        cloned.id = self.ids.element();
        cloned.label = source.label.as_ref().map(|label| {
            let mut cloned = label.clone();
            cloned.id = self.ids.label();
            self.record(ModelNodeId::Label(cloned.id), ModelNodeId::Label(label.id));
            cloned
        });
        cloned.kind = match &source.kind {
            ElementKind::Block(block) => ElementKind::Block(Block {
                alternatives: block
                    .alternatives
                    .iter()
                    .map(|alternative| self.alternative(alternative, 0))
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

    fn record(&mut self, destination: ModelNodeId, source: ModelNodeId) {
        let mut origins = self.provenance.origins(source).to_vec();
        origins.push(Origin::LeftRecursion {
            rule: self.rule,
            original_alt: self.original_alt,
            role: self.role,
        });
        self.provenance.record_model(destination, origins);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::frontend::{SourceId, parse_source};
    use crate::grammar::model::{GrammarId, GrammarKind};
    use crate::grammar::syntax::parse_grammar_unit;

    #[test]
    fn structurally_rewrites_binary_prefix_suffix_and_right_assoc_alternatives() {
        let text = r#"
parser grammar P;
expr
    : <assoc=right> expr '?' expr ':' expr # Conditional
    | expr '*' expr                       # Multiply
    | '-' expr                            # Negate
    | expr '++'                           # Suffix
    | INT                                 # Atom
    ;
"#;
        let file = parse_source(SourceId::new(0), "P.g4", text).expect("valid grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let mut unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);
        assert_eq!(unit.kind, GrammarKind::Parser);
        let diagnostics = rewrite_immediate_left_recursion(
            std::slice::from_mut(&mut unit),
            &mut ids,
            &mut provenance,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let rule = &unit.rules[0];
        let info = rule.left_recursion.as_ref().expect("rewrite metadata");
        assert_eq!(info.original_to_rewritten.len(), 5);
        let outer = &rule.block.alternatives[0];
        let ElementKind::Block(primary) = &outer.elements[0].kind else {
            panic!("primary block");
        };
        let ElementKind::Block(operators) = &outer.elements[1].kind else {
            panic!("operator block");
        };
        assert_eq!(primary.alternatives.len(), 2);
        assert_eq!(operators.alternatives.len(), 3);
        assert!(matches!(
            operators.alternatives[0].elements[0].kind,
            ElementKind::Predicate {
                precedence: Some(5),
                ..
            }
        ));
        let conditional_call = operators.alternatives[0]
            .elements
            .iter()
            .rev()
            .find_map(|element| match &element.kind {
                ElementKind::RuleCall(call) => Some(call),
                _ => None,
            })
            .expect("conditional recursive call");
        assert_eq!(conditional_call.precedence, Some(5));
        let multiply_call = operators.alternatives[1]
            .elements
            .iter()
            .rev()
            .find_map(|element| match &element.kind {
                ElementKind::RuleCall(call) => Some(call),
                _ => None,
            })
            .expect("multiply recursive call");
        assert_eq!(multiply_call.precedence, Some(5));
        assert!(matches!(
            outer.elements[1].quantifier,
            Quantifier::ZeroOrMore { greedy: true }
        ));
    }

    #[test]
    fn rejects_left_recursion_without_a_primary_alternative() {
        let text = "parser grammar P; expr : expr '+' expr;";
        let file = parse_source(SourceId::new(0), "P.g4", text).expect("valid grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let mut unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);
        let diagnostics = rewrite_immediate_left_recursion(
            std::slice::from_mut(&mut unit),
            &mut ids,
            &mut provenance,
        );
        assert_eq!(diagnostics[0].code, "G4R002");
    }

    #[test]
    fn enclosed_recursive_call_remains_a_primary_alternative() {
        let text = "parser grammar P; expr : expr '+' expr | '(' expr ')' | INT;";
        let file = parse_source(SourceId::new(0), "P.g4", text).expect("valid grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let mut unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);
        let diagnostics = rewrite_immediate_left_recursion(
            std::slice::from_mut(&mut unit),
            &mut ids,
            &mut provenance,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");

        let outer = &unit.rules[0].block.alternatives[0];
        let ElementKind::Block(primary) = &outer.elements[0].kind else {
            panic!("primary block");
        };
        assert_eq!(primary.alternatives.len(), 2);
    }
}
