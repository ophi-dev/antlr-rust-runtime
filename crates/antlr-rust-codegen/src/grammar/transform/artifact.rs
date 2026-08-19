// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::Serialize;

use super::{
    SafetyClass, StructuralMetrics, TransformAlternativeMapping, TransformCallSite,
    TransformCandidateReport, TransformCandidateStatus, TransformLabelMapping, TransformProjection,
    TransformReport, TransformReportEntry,
};
use crate::grammar::frontend::{SourceId, SourceSpan};
use crate::grammar::source::SourceSet;
use crate::json::to_pretty_json;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OptimizationManifest<'a> {
    version: u8,
    report_only: bool,
    passes: Vec<OptimizationPassManifest<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OptimizationPassManifest<'a> {
    id: usize,
    name: &'a str,
    safety_class: &'static str,
    changed: bool,
    metrics: MetricsChangeManifest,
    candidates: Vec<TransformCandidateManifest<'a>>,
}

#[derive(Serialize)]
struct MetricsChangeManifest {
    before: StructuralMetricsManifest,
    after: StructuralMetricsManifest,
}

#[derive(Serialize)]
struct StructuralMetricsManifest {
    rules: usize,
    alternatives: usize,
    elements: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransformCandidateManifest<'a> {
    grammar: &'a str,
    entry_rule: &'a str,
    source: SourceManifest<'a>,
    status: &'static str,
    reason: &'a str,
    rungs: &'a [String],
    boundary_rule: Option<&'a str>,
    projected: Option<TransformProjectionManifest>,
    removed_rules: Vec<RemovedRuleManifest<'a>>,
    alternatives: Vec<AlternativeMappingManifest<'a>>,
    label_renames: Vec<LabelMappingManifest<'a>>,
    grouping_changes: Vec<GroupingChangeManifest<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    inlined_call_sites: Vec<CallSiteManifest<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceManifest<'a> {
    path: Option<Cow<'a, str>>,
    byte_start: u32,
    byte_end: u32,
    start: Option<LineColumnManifest>,
    end: Option<LineColumnManifest>,
}

#[derive(Clone, Copy, Serialize)]
struct LineColumnManifest {
    line: usize,
    column: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TransformProjectionManifest {
    contexts_per_operand: ReductionManifest,
    precedence_decisions: ReductionManifest,
}

#[derive(Serialize)]
struct ReductionManifest {
    before: usize,
    after: usize,
    reduction: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemovedRuleManifest<'a> {
    rule: &'a str,
    target_rule: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AlternativeMappingManifest<'a> {
    source_rule: &'a str,
    source_alternative: usize,
    source: SourceManifest<'a>,
    target_rule: &'a str,
    target_alt_labels: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LabelMappingManifest<'a> {
    source_rule: &'a str,
    source_label: &'a str,
    source: SourceManifest<'a>,
    target_label: &'a str,
}

#[derive(Serialize)]
struct GroupingChangeManifest<'a> {
    rule: &'a str,
    from: &'static str,
    to: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CallSiteManifest<'a> {
    caller: &'a str,
    alternative: usize,
    source: SourceManifest<'a>,
}

pub(crate) fn render_optimization_manifest(
    report: &TransformReport,
    sources: &SourceSet,
    report_only: bool,
) -> String {
    let passes = report
        .entries
        .iter()
        .map(|entry| optimization_pass_manifest(entry, report, sources))
        .collect();
    to_pretty_json(&OptimizationManifest {
        version: 1,
        report_only,
        passes,
    })
}

fn optimization_pass_manifest<'a>(
    entry: &'a TransformReportEntry,
    report: &'a TransformReport,
    sources: &'a SourceSet,
) -> OptimizationPassManifest<'a> {
    let candidates = report
        .candidates
        .iter()
        .filter(|candidate| candidate.pass == entry.id)
        .map(|candidate| transform_candidate_manifest(candidate, sources))
        .collect();
    OptimizationPassManifest {
        id: entry.id.index(),
        name: entry.name,
        safety_class: safety_name(entry.safety),
        changed: entry.changed,
        metrics: MetricsChangeManifest {
            before: structural_metrics_manifest(&entry.before),
            after: structural_metrics_manifest(&entry.after),
        },
        candidates,
    }
}

const fn structural_metrics_manifest(metrics: &StructuralMetrics) -> StructuralMetricsManifest {
    StructuralMetricsManifest {
        rules: metrics.rules,
        alternatives: metrics.alternatives,
        elements: metrics.elements,
    }
}

fn transform_candidate_manifest<'a>(
    candidate: &'a TransformCandidateReport,
    sources: &'a SourceSet,
) -> TransformCandidateManifest<'a> {
    TransformCandidateManifest {
        grammar: &candidate.grammar,
        entry_rule: &candidate.entry_rule,
        source: source_manifest(&candidate.source_span, sources),
        status: candidate_status_name(candidate.status),
        reason: &candidate.reason,
        rungs: &candidate.rungs,
        boundary_rule: candidate.boundary_rule.as_deref(),
        projected: candidate
            .projection
            .as_ref()
            .map(transform_projection_manifest),
        removed_rules: candidate
            .removed_rules
            .iter()
            .map(|rule| RemovedRuleManifest {
                rule,
                target_rule: &candidate.entry_rule,
            })
            .collect(),
        alternatives: candidate
            .alternatives
            .iter()
            .map(|mapping| alternative_mapping_manifest(mapping, sources))
            .collect(),
        label_renames: candidate
            .labels
            .iter()
            .map(|mapping| label_mapping_manifest(mapping, sources))
            .collect(),
        grouping_changes: candidate
            .grouping_changes
            .iter()
            .map(|rule| GroupingChangeManifest {
                rule,
                from: "flat-loop",
                to: "left-recursive-nesting",
            })
            .collect(),
        inlined_call_sites: candidate
            .call_sites
            .iter()
            .map(|site| call_site_manifest(site, sources))
            .collect(),
    }
}

fn call_site_manifest<'a>(
    site: &'a TransformCallSite,
    sources: &'a SourceSet,
) -> CallSiteManifest<'a> {
    CallSiteManifest {
        caller: &site.caller,
        alternative: site.alternative,
        source: source_manifest(&site.source_span, sources),
    }
}

const fn transform_projection_manifest(
    projection: &TransformProjection,
) -> TransformProjectionManifest {
    TransformProjectionManifest {
        contexts_per_operand: ReductionManifest {
            before: projection.contexts_per_operand_before,
            after: projection.contexts_per_operand_after,
            reduction: projection
                .contexts_per_operand_before
                .saturating_sub(projection.contexts_per_operand_after),
        },
        precedence_decisions: ReductionManifest {
            before: projection.precedence_decisions_before,
            after: projection.precedence_decisions_after,
            reduction: projection
                .precedence_decisions_before
                .saturating_sub(projection.precedence_decisions_after),
        },
    }
}

fn alternative_mapping_manifest<'a>(
    mapping: &'a TransformAlternativeMapping,
    sources: &'a SourceSet,
) -> AlternativeMappingManifest<'a> {
    AlternativeMappingManifest {
        source_rule: &mapping.source_rule,
        source_alternative: mapping.source_alternative,
        source: source_manifest(&mapping.source_span, sources),
        target_rule: &mapping.target_rule,
        target_alt_labels: &mapping.target_alternatives,
    }
}

fn label_mapping_manifest<'a>(
    mapping: &'a TransformLabelMapping,
    sources: &'a SourceSet,
) -> LabelMappingManifest<'a> {
    LabelMappingManifest {
        source_rule: &mapping.source_rule,
        source_label: &mapping.source_label,
        source: source_manifest(&mapping.source_span, sources),
        target_label: &mapping.target_label,
    }
}

fn source_manifest<'a>(span: &SourceSpan, sources: &'a SourceSet) -> SourceManifest<'a> {
    SourceManifest {
        path: sources
            .logical_path(span.source)
            .map(std::path::Path::to_string_lossy),
        byte_start: span.bytes.start,
        byte_end: span.bytes.end,
        start: sources
            .line_column(span.source, span.bytes.start)
            .map(line_column_manifest),
        end: sources
            .line_column(span.source, span.bytes.end)
            .map(line_column_manifest),
    }
}

const fn line_column_manifest((line, column): (usize, usize)) -> LineColumnManifest {
    LineColumnManifest { line, column }
}

const fn safety_name(safety: SafetyClass) -> &'static str {
    match safety {
        SafetyClass::TreeAndApiPreserving => "tree-and-api-preserving",
        SafetyClass::RecognitionPreserving => "recognition-preserving",
    }
}

const fn candidate_status_name(status: TransformCandidateStatus) -> &'static str {
    match status {
        TransformCandidateStatus::Applied => "applied",
        TransformCandidateStatus::Eligible => "eligible",
        TransformCandidateStatus::Declined => "declined",
    }
}

pub(crate) fn render_unmodified_sources(sources: &SourceSet) -> BTreeMap<SourceId, String> {
    sources
        .iter()
        .map(|source| (source.id(), source.text().to_owned()))
        .collect()
}
