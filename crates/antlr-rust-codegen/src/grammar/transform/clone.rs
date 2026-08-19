// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
//! Shared cloning and tombstoning helpers for optional grammar transforms.
//!
//! Every cloned model node receives a fresh ID and a provenance record that
//! chains the source node with an [`Origin::OptionalTransform`] entry, so
//! [`crate::grammar::validation::validate_model`] invariants hold after any
//! structural rewrite.

use crate::grammar::frontend::SyntaxId;
use crate::grammar::model::{
    Alternative, Block, Element, ElementKind, Label, ModelIdAllocator, ModelNodeId, Rule,
    TransformId,
};
use crate::grammar::provenance::{Origin, ProvenanceIndex, Tombstone};

pub(crate) struct TransformCloner<'a> {
    pub(crate) ids: &'a mut ModelIdAllocator,
    pub(crate) provenance: &'a mut ProvenanceIndex,
    pub(crate) pass: TransformId,
}

impl TransformCloner<'_> {
    pub(crate) fn block(&mut self, source: &Block) -> Block {
        Block {
            alternatives: source
                .alternatives
                .iter()
                .map(|alternative| self.alternative(alternative))
                .collect(),
            options: source.options.clone(),
            syntax: source.syntax,
            span: source.span.clone(),
        }
    }

    pub(crate) fn alternative(&mut self, source: &Alternative) -> Alternative {
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

    pub(crate) fn element(&mut self, source: &Element) -> Element {
        let mut cloned = source.clone();
        cloned.id = self.ids.element();
        cloned.label = source.label.as_ref().map(|label| self.label(label));
        cloned.kind = match &source.kind {
            ElementKind::Block(block) => ElementKind::Block(self.block(block)),
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

    pub(crate) fn label(&mut self, source: &Label) -> Label {
        let mut cloned = source.clone();
        cloned.id = self.ids.label();
        self.record(ModelNodeId::Label(cloned.id), ModelNodeId::Label(source.id));
        cloned
    }

    pub(crate) fn record(&mut self, destination: ModelNodeId, source: ModelNodeId) {
        let mut origins = self.provenance.origins(source).to_vec();
        origins.push(Origin::OptionalTransform {
            pass: self.pass,
            inputs: Box::new([source]),
        });
        self.provenance.record_model(destination, origins);
    }
}

pub(crate) fn tombstone_rule(
    provenance: &mut ProvenanceIndex,
    rule: &Rule,
    reason: &'static str,
    replacements: &[ModelNodeId],
) {
    tombstone(provenance, rule.syntax, reason, replacements);
    for action in &rule.actions {
        tombstone(provenance, action.syntax, reason, replacements);
    }
    for handler in &rule.catches {
        tombstone(provenance, handler.syntax, reason, replacements);
    }
    if let Some(action) = &rule.finally_action {
        tombstone(provenance, action.syntax, reason, replacements);
    }
    tombstone_block(provenance, &rule.block, reason, replacements);
}

pub(crate) fn tombstone_block(
    provenance: &mut ProvenanceIndex,
    block: &Block,
    reason: &'static str,
    replacements: &[ModelNodeId],
) {
    for alternative in &block.alternatives {
        tombstone(provenance, alternative.syntax, reason, replacements);
        if let Some(label) = &alternative.label {
            tombstone(provenance, label.syntax, reason, replacements);
        }
        for element in &alternative.elements {
            tombstone(provenance, element.syntax, reason, replacements);
            if let Some(label) = &element.label {
                tombstone(provenance, label.syntax, reason, replacements);
            }
            if let ElementKind::Block(nested) = &element.kind {
                tombstone_block(provenance, nested, reason, replacements);
            }
        }
    }
}

fn tombstone(
    provenance: &mut ProvenanceIndex,
    syntax: SyntaxId,
    reason: &'static str,
    replacements: &[ModelNodeId],
) {
    provenance.tombstone(
        syntax,
        Tombstone {
            phase: "optional-transform",
            reason,
            replacements: replacements.to_vec().into_boxed_slice(),
        },
    );
}
