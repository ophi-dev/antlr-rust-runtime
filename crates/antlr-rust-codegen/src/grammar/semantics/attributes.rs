use std::collections::{BTreeMap, BTreeSet};

use crate::grammar::diagnostic::Diagnostic;
use crate::grammar::frontend::SourceSpan;
use crate::grammar::model::{
    Alternative, AttributeClause, AttributeSymbol, ElementKind, GrammarUnit, Label, LabelKind,
    Quantifier, Rule, RuleAttributes, Vocabulary,
};

pub(super) fn check_symbol_conflicts(
    unit: &GrammarUnit,
    vocabulary: &Vocabulary,
) -> Vec<Diagnostic> {
    let rule_names = unit
        .rules
        .iter()
        .map(|rule| rule.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();
    for rule in &unit.rules {
        let attributes = rule_attributes(rule);
        check_attribute_names(
            &attributes.arguments,
            &rule_names,
            vocabulary,
            ("parameter", "G4S056"),
            rule,
            &mut diagnostics,
        );
        check_attribute_names(
            &attributes.returns,
            &rule_names,
            vocabulary,
            ("return value", "G4S057"),
            rule,
            &mut diagnostics,
        );
        check_attribute_names(
            &attributes.locals,
            &rule_names,
            vocabulary,
            ("local", "G4S058"),
            rule,
            &mut diagnostics,
        );
        check_attribute_overlap(
            &attributes.returns,
            &attributes.arguments,
            "return value",
            "parameter",
            "G4S059",
            &mut diagnostics,
        );
        check_attribute_overlap(
            &attributes.locals,
            &attributes.arguments,
            "local",
            "parameter",
            "G4S060",
            &mut diagnostics,
        );
        check_attribute_overlap(
            &attributes.locals,
            &attributes.returns,
            "local",
            "return value",
            "G4S061",
            &mut diagnostics,
        );
        check_rule_labels(rule, &rule_names, vocabulary, &attributes, &mut diagnostics);
    }
    diagnostics
}

pub(super) fn rule_attributes(rule: &Rule) -> RuleAttributes {
    RuleAttributes {
        arguments: rule
            .arguments
            .as_ref()
            .map(attribute_symbols)
            .unwrap_or_default(),
        returns: rule
            .returns
            .as_ref()
            .map(attribute_symbols)
            .unwrap_or_default(),
        locals: rule
            .locals
            .as_ref()
            .map(attribute_symbols)
            .unwrap_or_default(),
    }
}

fn check_attribute_names(
    attributes: &[AttributeSymbol],
    rule_names: &BTreeSet<&str>,
    vocabulary: &Vocabulary,
    conflict: (&str, &'static str),
    rule: &Rule,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let (kind, rule_code) = conflict;
    for attribute in attributes {
        if rule_names.contains(attribute.name.as_str()) {
            diagnostics.push(Diagnostic::error(
                rule_code,
                attribute.span.clone(),
                format!(
                    "{kind} {} conflicts with rule with same name",
                    attribute.name
                ),
            ));
        }
    }
    for attribute in attributes {
        if vocabulary.by_name.contains_key(&attribute.name) {
            diagnostics.push(Diagnostic::error(
                "G4S037",
                attribute.span.clone(),
                format!(
                    "{kind} {} conflicts with token with same name in rule {}",
                    attribute.name, rule.name
                ),
            ));
        }
    }
}

fn check_attribute_overlap(
    attributes: &[AttributeSymbol],
    reference: &[AttributeSymbol],
    kind: &str,
    reference_kind: &str,
    code: &'static str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for attribute in attributes {
        if reference
            .iter()
            .any(|candidate| candidate.name == attribute.name)
        {
            diagnostics.push(Diagnostic::error(
                code,
                attribute.span.clone(),
                format!(
                    "{kind} {} conflicts with {reference_kind} with same name",
                    attribute.name
                ),
            ));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LabelValueKind {
    Token,
    Rule,
    Other,
}

#[derive(Clone, Debug)]
struct LabelSignature {
    assignment: LabelKind,
    value_kind: LabelValueKind,
    target: Option<String>,
    span: SourceSpan,
}

fn check_rule_labels(
    rule: &Rule,
    rule_names: &BTreeSet<&str>,
    vocabulary: &Vocabulary,
    attributes: &RuleAttributes,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let context_scoped = rule
        .block
        .alternatives
        .iter()
        .any(|alternative| alternative.label.is_some());
    let left_recursive = rule.block.alternatives.iter().any(|alternative| {
        alternative.elements.first().is_some_and(|element| {
            matches!(
                &element.kind,
                ElementKind::RuleCall(call)
                    if element.quantifier == Quantifier::One && call.name == rule.name
            )
        })
    });
    let mut namespaces = BTreeMap::<String, BTreeMap<String, LabelSignature>>::new();
    for alternative in &rule.block.alternatives {
        let context = if context_scoped {
            alternative
                .label
                .as_ref()
                .map_or_else(String::new, |label| label.value.clone())
        } else {
            String::new()
        };
        check_alternative_labels(
            rule,
            alternative,
            &context,
            left_recursive,
            rule_names,
            vocabulary,
            attributes,
            &mut namespaces,
            diagnostics,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn check_alternative_labels(
    rule: &Rule,
    alternative: &Alternative,
    context: &str,
    left_recursive: bool,
    rule_names: &BTreeSet<&str>,
    vocabulary: &Vocabulary,
    attributes: &RuleAttributes,
    namespaces: &mut BTreeMap<String, BTreeMap<String, LabelSignature>>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for element in &alternative.elements {
        if let Some(label) = &element.label {
            check_label_name(label, rule_names, vocabulary, attributes, diagnostics);
            let signature = label_signature(label, &element.kind);
            let namespace = namespaces.entry(context.to_owned()).or_default();
            if let Some(previous) = namespace.get(&label.name) {
                let primary = if left_recursive {
                    rule.span.clone()
                } else {
                    label.span.clone()
                };
                if previous.assignment != signature.assignment
                    || previous.value_kind != signature.value_kind
                {
                    diagnostics.push(
                        Diagnostic::error(
                            "G4S041",
                            primary.clone(),
                            format!("label {} has a conflicting type", label.name),
                        )
                        .with_related(previous.span.clone(), "first label is here"),
                    );
                }
                if previous.value_kind == LabelValueKind::Rule
                    && signature.value_kind == LabelValueKind::Rule
                    && previous.target != signature.target
                {
                    diagnostics.push(
                        Diagnostic::error(
                            "G4S041",
                            primary,
                            format!("label {} refers to different rules", label.name),
                        )
                        .with_related(previous.span.clone(), "first label is here"),
                    );
                }
            } else {
                namespace.insert(label.name.clone(), signature);
            }
        }
        if let ElementKind::Block(block) = &element.kind {
            for nested in &block.alternatives {
                check_alternative_labels(
                    rule,
                    nested,
                    context,
                    left_recursive,
                    rule_names,
                    vocabulary,
                    attributes,
                    namespaces,
                    diagnostics,
                );
            }
        }
    }
}

fn check_label_name(
    label: &Label,
    rule_names: &BTreeSet<&str>,
    vocabulary: &Vocabulary,
    attributes: &RuleAttributes,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if rule_names.contains(label.name.as_str()) {
        diagnostics.push(Diagnostic::error(
            "G4S038",
            label.span.clone(),
            format!("label {} conflicts with rule with same name", label.name),
        ));
    }
    if vocabulary.by_name.contains_key(&label.name) {
        diagnostics.push(Diagnostic::error(
            "G4S039",
            label.span.clone(),
            format!("label {} conflicts with token with same name", label.name),
        ));
    }
    for (symbols, code, kind) in [
        (&attributes.arguments, "G4S062", "parameter"),
        (&attributes.returns, "G4S063", "return value"),
        (&attributes.locals, "G4S064", "local"),
    ] {
        if symbols.iter().any(|attribute| attribute.name == label.name) {
            diagnostics.push(Diagnostic::error(
                code,
                label.span.clone(),
                format!("label {} conflicts with {kind} with same name", label.name),
            ));
        }
    }
}

fn label_signature(label: &Label, kind: &ElementKind) -> LabelSignature {
    let (value_kind, target) = match kind {
        ElementKind::RuleCall(call) => (LabelValueKind::Rule, Some(call.name.clone())),
        ElementKind::Terminal(_) | ElementKind::Range(..) | ElementKind::Set { .. } => {
            (LabelValueKind::Token, None)
        }
        ElementKind::Block(_)
        | ElementKind::Action { .. }
        | ElementKind::Predicate { .. }
        | ElementKind::Epsilon => (LabelValueKind::Other, None),
    };
    LabelSignature {
        assignment: label.kind,
        value_kind,
        target,
        span: label.span.clone(),
    }
}

fn attribute_symbols(clause: &AttributeClause) -> Vec<AttributeSymbol> {
    parse_attribute_declarations(&clause.text)
        .into_iter()
        .map(|declaration| {
            let offset =
                u32::try_from(declaration.name_offset).expect("attribute name offset exceeds u32");
            let length =
                u32::try_from(declaration.name.len()).expect("attribute name length exceeds u32");
            let start = clause
                .span
                .bytes
                .start
                .checked_add(1)
                .and_then(|start| start.checked_add(offset))
                .expect("attribute name span exceeds u32");
            let end = start
                .checked_add(length)
                .expect("attribute name span exceeds u32");
            AttributeSymbol {
                name: declaration.name,
                ty: declaration.ty.unwrap_or_default(),
                span: SourceSpan {
                    source: clause.span.source,
                    bytes: start..end,
                },
            }
        })
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParsedAttributeDeclaration {
    pub(crate) name: String,
    pub(crate) ty: Option<String>,
    pub(crate) initializer: Option<String>,
    pub(crate) name_offset: usize,
}

pub(crate) fn parse_attribute_declarations(clause: &str) -> Vec<ParsedAttributeDeclaration> {
    split_top_level(clause, ',')
        .into_iter()
        .filter_map(|(raw_offset, raw_part)| {
            let leading = raw_part.len() - raw_part.trim_start().len();
            let part_offset = raw_offset + leading;
            let part = raw_part.trim();
            if part.is_empty() {
                return None;
            }

            let (declarator, initializer) =
                part.find('=')
                    .filter(|index| *index > 0)
                    .map_or((part, None), |equals| {
                        (
                            part[..equals].trim_end(),
                            Some(part[equals + 1..].trim().to_owned()),
                        )
                    });
            let (name, ty, name_offset) = if let Some(colon) = postfix_type_colon(declarator) {
                parse_postfix_attribute_declaration(declarator, colon)?
            } else {
                parse_prefix_attribute_declaration(declarator)?
            };
            Some(ParsedAttributeDeclaration {
                name,
                ty,
                initializer,
                name_offset: part_offset + name_offset,
            })
        })
        .collect()
}

fn postfix_type_colon(declarator: &str) -> Option<usize> {
    declarator
        .char_indices()
        .find(|(index, character)| {
            *character == ':'
                && !declarator[..*index].ends_with(':')
                && !declarator[*index + 1..].starts_with(':')
        })
        .map(|(index, _)| index)
}

fn parse_prefix_attribute_declaration(declarator: &str) -> Option<(String, Option<String>, usize)> {
    let mut in_identifier = false;
    let mut start = None;
    for (index, character) in declarator.char_indices().rev() {
        if !in_identifier && is_identifier_character(character) {
            in_identifier = true;
        } else if in_identifier && !is_identifier_character(character) {
            start = Some(index + character.len_utf8());
            break;
        }
    }
    let start = start.or_else(|| in_identifier.then_some(0))?;
    let stop = declarator[start..]
        .char_indices()
        .find(|(_, character)| !is_identifier_character(*character))
        .map_or(declarator.len(), |(offset, _)| start + offset);
    let name = declarator[start..stop].to_owned();
    let ty = format!("{}{}", &declarator[..start], &declarator[stop..]);
    Some((name, nonempty_trimmed(&ty), start))
}

fn parse_postfix_attribute_declaration(
    declarator: &str,
    colon: usize,
) -> Option<(String, Option<String>, usize)> {
    let name_part = &declarator[..colon];
    let start = name_part
        .char_indices()
        .find(|(_, character)| is_identifier_character(*character))
        .map(|(index, _)| index)?;
    let stop = name_part[start..]
        .char_indices()
        .find(|(_, character)| !is_identifier_character(*character))
        .map_or(name_part.len(), |(offset, _)| start + offset);
    let name = name_part[start..stop].to_owned();
    let ty = nonempty_trimmed(&declarator[colon + 1..]);
    Some((name, ty, start))
}

fn split_top_level(text: &str, separator: char) -> Vec<(usize, &str)> {
    let mut parts = Vec::new();
    let mut delimiter_depth = 0_usize;
    let mut angle_depth = 0_usize;
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut in_initializer = false;
    for (index, character) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '(' | '[' | '{' if !quoted => delimiter_depth += 1,
            ')' | ']' | '}' if !quoted => {
                delimiter_depth = delimiter_depth.saturating_sub(1);
            }
            '<' if !quoted && !in_initializer => angle_depth += 1,
            '>' if !quoted && !in_initializer => {
                angle_depth = angle_depth.saturating_sub(1);
            }
            '=' if !quoted
                && delimiter_depth == 0
                && angle_depth == 0
                && is_assignment_operator(text, index) =>
            {
                in_initializer = true;
            }
            _ if character == separator
                && !quoted
                && delimiter_depth == 0
                && (in_initializer || angle_depth == 0) =>
            {
                parts.push((start, &text[start..index]));
                start = index + character.len_utf8();
                angle_depth = 0;
                in_initializer = false;
            }
            _ => {}
        }
    }
    parts.push((start, &text[start..]));
    parts
}

fn is_assignment_operator(text: &str, index: usize) -> bool {
    let previous = text[..index].chars().next_back();
    let next = text[index + '='.len_utf8()..].chars().next();
    !matches!(previous, Some('=' | '!' | '<' | '>')) && next != Some('=')
}

fn nonempty_trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn is_identifier_character(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}
