pub(crate) mod analysis;
pub(crate) mod artifact;
mod registry;

pub(crate) mod passes {
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
    pub(crate) removed_rules: Vec<String>,
    pub(crate) alternatives: Vec<TransformAlternativeMapping>,
    pub(crate) labels: Vec<TransformLabelMapping>,
    pub(crate) grouping_changes: Vec<String>,
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
