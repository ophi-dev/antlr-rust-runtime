// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use crate::grammar::transform::SafetyClass;
use crate::grammar::transform::passes::inline_trivial::InlineTrivialRules;
use crate::grammar::transform::passes::precedence_ladder::CollapsePrecedenceLadders;
use crate::grammar::transform::passes::prune_unreachable::PruneUnreachableRules;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OptimizationStage {
    IntegratedGrammar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PassDescriptor {
    pub(super) id: &'static str,
    pub(super) stage: OptimizationStage,
    pub(super) safety: SafetyClass,
    pub(super) canonical_order: u16,
    pub(super) prerequisites: &'static [&'static str],
    pub(super) conflicts: &'static [&'static str],
}

pub(super) const PRUNE_UNREACHABLE_RULES: PassDescriptor = PassDescriptor {
    id: PruneUnreachableRules::NAME,
    stage: OptimizationStage::IntegratedGrammar,
    safety: SafetyClass::RecognitionPreserving,
    canonical_order: 100,
    prerequisites: &[],
    conflicts: &[],
};

pub(super) const INLINE_TRIVIAL_RULES: PassDescriptor = PassDescriptor {
    id: InlineTrivialRules::NAME,
    stage: OptimizationStage::IntegratedGrammar,
    safety: SafetyClass::RecognitionPreserving,
    canonical_order: 150,
    prerequisites: &[],
    conflicts: &[],
};

pub(super) const COLLAPSE_PRECEDENCE_LADDERS: PassDescriptor = PassDescriptor {
    id: CollapsePrecedenceLadders::NAME,
    stage: OptimizationStage::IntegratedGrammar,
    safety: SafetyClass::RecognitionPreserving,
    canonical_order: 200,
    prerequisites: &[],
    conflicts: &[],
};
