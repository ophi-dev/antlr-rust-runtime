// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
pub(crate) mod analysis;
pub(crate) mod artifact;
pub(crate) mod clone;
mod registry;

pub(crate) mod passes {
    pub(crate) mod inline_trivial;
    pub(crate) mod precedence_ladder;
    pub(crate) mod prune_unreachable;
}

use std::collections::BTreeSet;

use super::action::ActionReferenceParser;
use super::diagnostic::Diagnostic;
use super::frontend::SourceSpan;
use super::model::{GrammarId, GrammarUnit, ModelIdAllocator, RuleId, TransformId};
use super::provenance::ProvenanceIndex;
use analysis::{AnalysisInvalidation, TransformAnalysis};

pub(crate) use crate::optimization::metrics::StructuralMetrics;
pub(crate) use artifact::render_optimization_manifest;
pub(crate) use registry::TransformRegistry;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SafetyClass {
    TreeAndApiPreserving,
    RecognitionPreserving,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformReportEntry {
    pub(crate) id: TransformId,
    pub(crate) name: &'static str,
    pub(crate) safety: SafetyClass,
    pub(crate) before: StructuralMetrics,
    pub(crate) after: StructuralMetrics,
    pub(crate) changed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransformCandidateStatus {
    Applied,
    Eligible,
    Declined,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformProjection {
    pub(crate) contexts_per_operand_before: usize,
    pub(crate) contexts_per_operand_after: usize,
    pub(crate) precedence_decisions_before: usize,
    pub(crate) precedence_decisions_after: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformAlternativeMapping {
    pub(crate) source_rule: String,
    pub(crate) source_alternative: usize,
    pub(crate) source_span: SourceSpan,
    pub(crate) target_rule: String,
    pub(crate) target_alternatives: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformLabelMapping {
    pub(crate) source_rule: String,
    pub(crate) source_label: String,
    pub(crate) source_span: SourceSpan,
    pub(crate) target_label: String,
}

/// One rewritten (or would-be rewritten) reference to an inlined rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformCallSite {
    pub(crate) caller: String,
    /// One-based top-level alternative ordinal within the caller.
    pub(crate) alternative: usize,
    pub(crate) source_span: SourceSpan,
}

/// One rule deleted by a transform, with the rule that absorbed it when a
/// single surviving target exists (inlined rules dissolve into their call
/// sites instead).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformRemovedRule {
    pub(crate) rule: String,
    pub(crate) target: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformCandidateReport {
    pub(crate) pass: TransformId,
    pub(crate) grammar: String,
    pub(crate) entry_rule: String,
    pub(crate) source_span: SourceSpan,
    pub(crate) status: TransformCandidateStatus,
    pub(crate) reason: String,
    pub(crate) rungs: Vec<String>,
    pub(crate) boundary_rule: Option<String>,
    pub(crate) projection: Option<TransformProjection>,
    pub(crate) removed_rules: Vec<TransformRemovedRule>,
    pub(crate) alternatives: Vec<TransformAlternativeMapping>,
    pub(crate) labels: Vec<TransformLabelMapping>,
    pub(crate) grouping_changes: Vec<String>,
    pub(crate) call_sites: Vec<TransformCallSite>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TransformReport {
    pub(crate) entries: Vec<TransformReportEntry>,
    pub(crate) candidates: Vec<TransformCandidateReport>,
    pub(crate) rule_removals: Vec<TransformRuleRemoval>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformRuleRemoval {
    pub(crate) pass: TransformId,
    pub(crate) grammar: String,
    pub(crate) rule: String,
    pub(crate) source_span: SourceSpan,
}

pub(crate) struct TransformContext<'a> {
    pub(crate) id: TransformId,
    pub(crate) analysis: &'a TransformAnalysis,
    pub(crate) report_only: bool,
    pub(crate) action_reference_parser: ActionReferenceParser,
}

#[derive(Clone, Debug)]
pub(crate) struct TransformGrammar {
    pub(crate) units: Vec<GrammarUnit>,
    pub(crate) target_units: BTreeSet<GrammarId>,
    pub(crate) preserved_rules: BTreeSet<RuleId>,
    pub(crate) provenance: ProvenanceIndex,
}

pub(crate) trait GrammarTransform {
    fn name(&self) -> &'static str;
    fn safety_class(&self) -> SafetyClass;
    fn invalidates(&self) -> AnalysisInvalidation;
    fn apply(
        &self,
        input: &TransformContext<'_>,
        grammar: &mut TransformGrammar,
        ids: &mut ModelIdAllocator,
        report: &mut TransformReport,
    ) -> Result<bool, Diagnostic>;
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::BTreeSet;

    use super::TransformGrammar;
    use crate::grammar::frontend::{SourceId, parse_source};
    use crate::grammar::model::{GrammarId, ModelIdAllocator};
    use crate::grammar::provenance::ProvenanceIndex;
    use crate::grammar::syntax::parse_grammar_unit;

    /// Parses one grammar source into a single-unit [`TransformGrammar`]
    /// targeted for optional transforms, without running integration.
    pub(crate) fn single_unit_fixture(text: &str) -> (TransformGrammar, ModelIdAllocator) {
        let file = parse_source(SourceId::new(0), "P.g4", text).expect("valid grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);
        (
            TransformGrammar {
                units: vec![unit],
                target_units: BTreeSet::from([GrammarId::new(0)]),
                preserved_rules: BTreeSet::new(),
                provenance,
            },
            ids,
        )
    }
}
