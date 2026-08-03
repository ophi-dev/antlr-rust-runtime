use crate::generator::prelude::*;
use crate::semantics::PredicateTemplate;

#[derive(Clone, Copy, Debug)]
struct StructuralElement<'a> {
    element: &'a Element,
}

include!("contexts.rs");

#[derive(Clone, Debug)]
pub(crate) struct StructuralAction {
    pub(crate) rule_index: usize,
    pub(crate) action_index: usize,
    pub(crate) state: usize,
    pub(crate) body: String,
    pub(crate) span: SourceSpan,
    pub(crate) authored: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct StructuralPredicate {
    pub(crate) rule_index: usize,
    pub(crate) predicate_index: usize,
    pub(crate) body: String,
    pub(crate) fail: Option<String>,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug)]
pub(crate) struct StructuralRuleCall {
    pub(crate) target_rule_index: usize,
    pub(crate) state: usize,
    pub(crate) arguments: Option<String>,
    pub(crate) caller_first_argument: Option<String>,
}

fn structural_elements<'model>(
    data: &RecognizerCodegenData<'model>,
) -> io::Result<Vec<StructuralElement<'model>>> {
    let semantic = data.semantic.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "structural grammar model is unavailable",
        )
    })?;
    let mut elements = Vec::new();
    for rule in &semantic.unit.rules {
        for alternative in &rule.block.alternatives {
            collect_structural_elements(alternative, &mut elements);
        }
    }
    Ok(elements)
}

fn collect_structural_elements<'a>(
    alternative: &'a Alternative,
    elements: &mut Vec<StructuralElement<'a>>,
) {
    for element in &alternative.elements {
        elements.push(StructuralElement { element });
        if let ElementKind::Block(block) = &element.kind {
            for nested in &block.alternatives {
                collect_structural_elements(nested, elements);
            }
        }
    }
}

pub(crate) fn structural_actions(
    data: &RecognizerCodegenData<'_>,
) -> io::Result<Vec<StructuralAction>> {
    let semantic = data.semantic.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "structural grammar model is unavailable",
        )
    })?;
    let graph = data.graph.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "finalized ATN graph is unavailable",
        )
    })?;
    let provenance = data
        .provenance
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "provenance is unavailable"))?;
    let mut actions = Vec::new();
    for item in structural_elements(data)? {
        let ElementKind::Action { id, body } = &item.element.kind else {
            continue;
        };
        let binding = semantic
            .bindings
            .actions
            .get(id)
            .expect("semantic action binding exists");
        let authored = provenance
            .origins(ModelNodeId::Element(item.element.id))
            .iter()
            .any(|origin| matches!(origin, Origin::Authored { .. }));
        for transition in graph.transitions_for_model(ModelNodeId::Element(item.element.id)) {
            let FinalizedTransitionKind::Action { rule_index, .. } = transition.kind else {
                continue;
            };
            actions.push(StructuralAction {
                rule_index,
                action_index: binding.index,
                state: transition.source,
                body: body.clone(),
                span: item.element.span.clone(),
                authored,
            });
        }
    }
    actions.sort_by_key(|action| (action.state, action.rule_index, action.action_index));
    actions.dedup_by_key(|action| action.state);
    Ok(actions)
}

pub(crate) fn structural_predicates(
    data: &RecognizerCodegenData<'_>,
) -> io::Result<Vec<StructuralPredicate>> {
    let semantic = data.semantic.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "structural grammar model is unavailable",
        )
    })?;
    let graph = data.graph.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "finalized ATN graph is unavailable",
        )
    })?;
    let mut predicates = Vec::new();
    for item in structural_elements(data)? {
        let ElementKind::Predicate { id, body, fail, .. } = &item.element.kind else {
            continue;
        };
        let binding = semantic
            .bindings
            .predicates
            .get(id)
            .expect("semantic predicate binding exists");
        for transition in graph.transitions_for_model(ModelNodeId::Element(item.element.id)) {
            let FinalizedTransitionKind::Predicate {
                rule_index,
                predicate_index,
                ..
            } = transition.kind
            else {
                continue;
            };
            debug_assert_eq!(binding.index, predicate_index);
            predicates.push(StructuralPredicate {
                rule_index,
                predicate_index,
                body: body.clone(),
                fail: fail.clone(),
                span: item.element.span.clone(),
            });
        }
    }
    predicates.sort_by_key(|predicate| (predicate.rule_index, predicate.predicate_index));
    predicates.dedup_by_key(|predicate| (predicate.rule_index, predicate.predicate_index));
    Ok(predicates)
}

pub(crate) fn structural_rule_calls(
    data: &RecognizerCodegenData<'_>,
) -> io::Result<Vec<StructuralRuleCall>> {
    let semantic = data.semantic.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "structural grammar model is unavailable",
        )
    })?;
    let graph = data.graph.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "finalized ATN graph is unavailable",
        )
    })?;
    let mut calls = Vec::new();
    for item in structural_elements(data)? {
        let ElementKind::RuleCall(call) = &item.element.kind else {
            continue;
        };
        let binding = semantic
            .bindings
            .rule_calls
            .get(&item.element.id)
            .expect("semantic rule-call binding exists");
        for transition in graph.transitions_for_model(ModelNodeId::Element(item.element.id)) {
            let FinalizedTransitionKind::Rule { rule_index, .. } = transition.kind else {
                continue;
            };
            debug_assert_eq!(
                semantic.recognizer.rule_numbers[&binding.target],
                rule_index
            );
            calls.push(StructuralRuleCall {
                target_rule_index: rule_index,
                state: transition.source,
                arguments: call.arguments.clone(),
                caller_first_argument: semantic
                    .bindings
                    .attributes
                    .get(&binding.caller)
                    .and_then(|attributes| attributes.arguments.first())
                    .map(|argument| argument.name.clone()),
            });
        }
    }
    calls.sort_by_key(|call| (call.state, call.target_rule_index));
    calls.dedup_by_key(|call| call.state);
    Ok(calls)
}

pub(crate) fn structural_line_column(
    data: &RecognizerCodegenData<'_>,
    span: &SourceSpan,
) -> (usize, usize) {
    data.sources
        .and_then(|sources| sources.get(span.source))
        .and_then(|source| source.line_column(span.bytes.start))
        .unwrap_or((0, 0))
}

pub(crate) fn embedded_body_translation_error(
    data: &RecognizerCodegenData<'_>,
    span: &SourceSpan,
    kind: &str,
    rule_index: usize,
    coordinate_index: usize,
    error: &io::Error,
) -> io::Error {
    let path = data
        .sources
        .and_then(|sources| sources.logical_path(span.source))
        .map_or_else(|| "<grammar>".to_owned(), |path| path.display().to_string());
    let (line, column) = structural_line_column(data, span);
    let rule = data
        .rule_names
        .get(rule_index)
        .map_or("<unknown>", String::as_str);
    io::Error::new(
        error.kind(),
        format!(
            "{path}:{line}:{column}: cannot lower embedded {kind} coordinate \
             ({rule_index}, {coordinate_index}) in rule {rule}: {error}"
        ),
    )
}

pub(crate) fn structural_embedded_model(
    data: &RecognizerCodegenData<'_>,
    include_members: bool,
) -> io::Result<embedded::EmbeddedModel> {
    let semantic = data.semantic.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "structural grammar model is unavailable",
        )
    })?;
    let mut rules = data
        .rule_names
        .iter()
        .map(|name| embedded::RuleModel {
            name: name.clone(),
            ..embedded::RuleModel::default()
        })
        .collect::<Vec<_>>();

    for rule in &semantic.unit.rules {
        let Some(&rule_index) = semantic.recognizer.rule_numbers.get(&rule.id) else {
            continue;
        };
        let attributes = semantic
            .bindings
            .attributes
            .get(&rule.id)
            .cloned()
            .unwrap_or_default();
        let attrs = attributes
            .arguments
            .iter()
            .chain(&attributes.returns)
            .chain(&attributes.locals)
            .map(structural_attr_decl)
            .collect();
        let init_body = rule
            .actions
            .iter()
            .find(|action| action.name == "init")
            .map(|action| action.body.clone());
        let after_body = rule
            .actions
            .iter()
            .find(|action| action.name == "after")
            .map(|action| action.body.clone());
        rules[rule_index] = embedded::RuleModel {
            name: rule.name.clone(),
            attrs,
            local_names: attributes
                .locals
                .iter()
                .map(|attribute| attribute.name.clone())
                .collect(),
            arg_names: attributes
                .arguments
                .iter()
                .map(|attribute| attribute.name.clone())
                .collect(),
            init_body,
            after_body,
            alts: structural_rule_alternatives(rule, &semantic.recognizer.vocabulary),
        };
    }

    let mut parser_members = embedded::MembersModel::default();
    if include_members {
        for action in &semantic.unit.actions {
            if action.name == "members"
                && action
                    .scope
                    .as_deref()
                    .is_none_or(|scope| scope == "parser")
            {
                embedded::classify_members(
                    &action.body,
                    action.body_span.source,
                    &mut parser_members,
                )?;
            }
        }
    }

    Ok(embedded::EmbeddedModel {
        rules,
        parser_members,
    })
}
