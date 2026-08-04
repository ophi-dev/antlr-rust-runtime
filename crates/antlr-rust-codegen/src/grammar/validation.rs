use std::collections::BTreeSet;

use super::diagnostic::Diagnostic;
use super::frontend::SourceSpan;
use super::model::{Block, ElementKind, ModelNodeId};
use super::transform::TransformGrammar;

pub(crate) fn validate_model(grammar: &TransformGrammar) -> Result<(), Diagnostic> {
    let mut nodes = BTreeSet::new();
    for unit in &grammar.units {
        insert_model_node(&mut nodes, ModelNodeId::Grammar(unit.id), unit.span.clone())?;
        for token in &unit.tokens {
            insert_model_node(
                &mut nodes,
                ModelNodeId::Token(token.id),
                token.name.span.clone(),
            )?;
        }
        for channel in &unit.channels {
            insert_model_node(
                &mut nodes,
                ModelNodeId::Channel(channel.id),
                channel.name.span.clone(),
            )?;
        }
        for action in &unit.actions {
            insert_model_node(
                &mut nodes,
                ModelNodeId::Action(action.id),
                action.span.clone(),
            )?;
        }
        let modes = unit
            .modes
            .iter()
            .map(|mode| mode.id)
            .collect::<BTreeSet<_>>();
        let rules = unit
            .rules
            .iter()
            .map(|rule| rule.id)
            .collect::<BTreeSet<_>>();
        for mode in &unit.modes {
            insert_model_node(&mut nodes, ModelNodeId::Mode(mode.id), mode.span.clone())?;
            if let Some(missing) = mode.rules.iter().find(|rule| !rules.contains(rule)) {
                return Err(Diagnostic::error(
                    "G4T902",
                    mode.span.clone(),
                    format!(
                        "mode {} references missing rule ID {}",
                        mode.name,
                        missing.index()
                    ),
                ));
            }
        }
        for rule in &unit.rules {
            insert_model_node(&mut nodes, ModelNodeId::Rule(rule.id), rule.span.clone())?;
            if rule.mode.is_some_and(|mode| !modes.contains(&mode)) {
                return Err(Diagnostic::error(
                    "G4T903",
                    rule.span.clone(),
                    format!("rule {} references a missing mode", rule.name),
                ));
            }
            for action in &rule.actions {
                insert_model_node(
                    &mut nodes,
                    ModelNodeId::Action(action.id),
                    action.span.clone(),
                )?;
            }
            if let Some(action) = &rule.finally_action {
                insert_model_node(
                    &mut nodes,
                    ModelNodeId::Action(action.id),
                    action.span.clone(),
                )?;
            }
            collect_block_nodes(&rule.block, &mut nodes)?;
        }
    }
    if let Some(node) = nodes
        .iter()
        .find(|node| grammar.provenance.origins(**node).is_empty())
    {
        let span = grammar.units.first().map(|unit| unit.span.clone());
        return Err(Diagnostic::error_with_optional_span(
            "G4T904",
            span,
            format!("model node {node:?} has no provenance"),
        ));
    }
    Ok(())
}

fn collect_block_nodes(block: &Block, nodes: &mut BTreeSet<ModelNodeId>) -> Result<(), Diagnostic> {
    for alternative in &block.alternatives {
        insert_model_node(
            nodes,
            ModelNodeId::Alternative(alternative.id),
            alternative.span.clone(),
        )?;
        for element in &alternative.elements {
            insert_model_node(
                nodes,
                ModelNodeId::Element(element.id),
                element.enclosing_span.clone(),
            )?;
            if let Some(label) = &element.label {
                insert_model_node(nodes, ModelNodeId::Label(label.id), label.span.clone())?;
            }
            match &element.kind {
                ElementKind::Action { id, .. } => {
                    insert_model_node(nodes, ModelNodeId::Action(*id), element.span.clone())?;
                }
                ElementKind::Predicate { id, .. } => {
                    insert_model_node(nodes, ModelNodeId::Predicate(*id), element.span.clone())?;
                }
                ElementKind::Block(nested) => collect_block_nodes(nested, nodes)?,
                _ => {}
            }
        }
    }
    Ok(())
}

fn insert_model_node(
    nodes: &mut BTreeSet<ModelNodeId>,
    node: ModelNodeId,
    span: SourceSpan,
) -> Result<(), Diagnostic> {
    if nodes.insert(node) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "G4T901",
            span,
            format!("model node ID {node:?} is used more than once"),
        ))
    }
}
