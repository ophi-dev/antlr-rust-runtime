use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{
    SafetyClass, StructuralMetrics, TransformAlternativeMapping, TransformCandidateReport,
    TransformCandidateStatus, TransformLabelMapping, TransformProjection, TransformReport,
};
use crate::grammar::frontend::{SourceId, SourceSpan};

pub(crate) fn render_optimization_manifest(
    report: &TransformReport,
    sources: &crate::grammar::source::SourceSet,
    report_only: bool,
) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"version\": 1,\n");
    let _ = writeln!(out, "  \"reportOnly\": {report_only},");
    out.push_str("  \"passes\": [");
    for (position, entry) in report.entries.iter().enumerate() {
        if position > 0 {
            out.push(',');
        }
        out.push_str("\n    {\n");
        let _ = writeln!(out, "      \"id\": {},", entry.id.index());
        let _ = writeln!(out, "      \"name\": {},", json_string(entry.name));
        let _ = writeln!(
            out,
            "      \"safetyClass\": {},",
            json_string(safety_name(entry.safety))
        );
        let _ = writeln!(out, "      \"changed\": {},", entry.changed);
        out.push_str("      \"metrics\": {\"before\": ");
        write_metrics(&mut out, &entry.before);
        out.push_str(", \"after\": ");
        write_metrics(&mut out, &entry.after);
        out.push_str("},\n");
        out.push_str("      \"candidates\": [");
        let candidates = report
            .candidates
            .iter()
            .filter(|candidate| candidate.pass == entry.id)
            .collect::<Vec<_>>();
        for (candidate_position, candidate) in candidates.iter().enumerate() {
            if candidate_position > 0 {
                out.push(',');
            }
            out.push_str("\n        ");
            write_candidate(&mut out, candidate, sources);
        }
        if candidates.is_empty() {
            out.push_str("]\n    }");
        } else {
            out.push_str("\n      ]\n    }");
        }
    }
    if report.entries.is_empty() {
        out.push_str("]\n}\n");
    } else {
        out.push_str("\n  ]\n}\n");
    }
    out
}

fn write_metrics(out: &mut String, metrics: &StructuralMetrics) {
    let _ = write!(
        out,
        "{{\"rules\": {}, \"alternatives\": {}, \"elements\": {}}}",
        metrics.rules, metrics.alternatives, metrics.elements
    );
}

fn write_candidate(
    out: &mut String,
    candidate: &TransformCandidateReport,
    sources: &crate::grammar::source::SourceSet,
) {
    out.push_str("{\n");
    let _ = writeln!(
        out,
        "          \"grammar\": {},",
        json_string(&candidate.grammar)
    );
    let _ = writeln!(
        out,
        "          \"entryRule\": {},",
        json_string(&candidate.entry_rule)
    );
    out.push_str("          \"source\": ");
    write_source_span(out, &candidate.source_span, sources);
    out.push_str(",\n");
    let _ = writeln!(
        out,
        "          \"status\": {},",
        json_string(candidate_status_name(candidate.status))
    );
    let _ = writeln!(
        out,
        "          \"reason\": {},",
        json_string(&candidate.reason)
    );
    out.push_str("          \"rungs\": ");
    write_string_array(out, &candidate.rungs);
    out.push_str(",\n");
    let _ = writeln!(
        out,
        "          \"boundaryRule\": {},",
        json_optional_string(candidate.boundary_rule.as_deref())
    );
    out.push_str("          \"projected\": ");
    if let Some(projection) = &candidate.projection {
        write_projection(out, projection);
    } else {
        out.push_str("null");
    }
    out.push_str(",\n");
    out.push_str("          \"removedRules\": [");
    for (position, rule) in candidate.removed_rules.iter().enumerate() {
        if position > 0 {
            out.push_str(", ");
        }
        let _ = write!(
            out,
            "{{\"rule\": {}, \"targetRule\": {}}}",
            json_string(rule),
            json_string(&candidate.entry_rule)
        );
    }
    out.push_str("],\n");
    out.push_str("          \"alternatives\": [");
    for (position, mapping) in candidate.alternatives.iter().enumerate() {
        if position > 0 {
            out.push(',');
        }
        out.push_str("\n            ");
        write_alternative_mapping(out, mapping, sources);
    }
    if candidate.alternatives.is_empty() {
        out.push_str("],\n");
    } else {
        out.push_str("\n          ],\n");
    }
    out.push_str("          \"labelRenames\": [");
    for (position, mapping) in candidate.labels.iter().enumerate() {
        if position > 0 {
            out.push(',');
        }
        out.push_str("\n            ");
        write_label_mapping(out, mapping, sources);
    }
    if candidate.labels.is_empty() {
        out.push_str("],\n");
    } else {
        out.push_str("\n          ],\n");
    }
    out.push_str("          \"groupingChanges\": [");
    for (position, rule) in candidate.grouping_changes.iter().enumerate() {
        if position > 0 {
            out.push_str(", ");
        }
        let _ = write!(
            out,
            "{{\"rule\": {}, \"from\": \"flat-loop\", \"to\": \"left-recursive-nesting\"}}",
            json_string(rule)
        );
    }
    out.push_str("]\n        }");
}

fn write_projection(out: &mut String, projection: &TransformProjection) {
    let context_reduction = projection
        .contexts_per_operand_before
        .saturating_sub(projection.contexts_per_operand_after);
    let decision_reduction = projection
        .precedence_decisions_before
        .saturating_sub(projection.precedence_decisions_after);
    let _ = write!(
        out,
        "{{\"contextsPerOperand\": {{\"before\": {}, \"after\": {}, \"reduction\": {context_reduction}}}, \
         \"precedenceDecisions\": {{\"before\": {}, \"after\": {}, \"reduction\": {decision_reduction}}}}}",
        projection.contexts_per_operand_before,
        projection.contexts_per_operand_after,
        projection.precedence_decisions_before,
        projection.precedence_decisions_after
    );
}

fn write_alternative_mapping(
    out: &mut String,
    mapping: &TransformAlternativeMapping,
    sources: &crate::grammar::source::SourceSet,
) {
    out.push('{');
    let _ = write!(
        out,
        "\"sourceRule\": {}, \"sourceAlternative\": {}, \"source\": ",
        json_string(&mapping.source_rule),
        mapping.source_alternative
    );
    write_source_span(out, &mapping.source_span, sources);
    let _ = write!(
        out,
        ", \"targetRule\": {}, \"targetAltLabels\": ",
        json_string(&mapping.target_rule)
    );
    write_string_array(out, &mapping.target_alternatives);
    out.push('}');
}

fn write_label_mapping(
    out: &mut String,
    mapping: &TransformLabelMapping,
    sources: &crate::grammar::source::SourceSet,
) {
    out.push('{');
    let _ = write!(
        out,
        "\"sourceRule\": {}, \"sourceLabel\": {}, \"source\": ",
        json_string(&mapping.source_rule),
        json_string(&mapping.source_label)
    );
    write_source_span(out, &mapping.source_span, sources);
    let _ = write!(
        out,
        ", \"targetLabel\": {}",
        json_string(&mapping.target_label)
    );
    out.push('}');
}

fn write_source_span(
    out: &mut String,
    span: &SourceSpan,
    sources: &crate::grammar::source::SourceSet,
) {
    let path = sources
        .logical_path(span.source)
        .map(|path| path.to_string_lossy().into_owned());
    let start = sources.line_column(span.source, span.bytes.start);
    let end = sources.line_column(span.source, span.bytes.end);
    out.push('{');
    let _ = write!(
        out,
        "\"path\": {}, \"byteStart\": {}, \"byteEnd\": {}, \"start\": ",
        json_optional_string(path.as_deref()),
        span.bytes.start,
        span.bytes.end
    );
    write_line_column(out, start);
    out.push_str(", \"end\": ");
    write_line_column(out, end);
    out.push('}');
}

fn write_line_column(out: &mut String, coordinate: Option<(usize, usize)>) {
    if let Some((line, column)) = coordinate {
        let _ = write!(out, "{{\"line\": {line}, \"column\": {column}}}");
    } else {
        out.push_str("null");
    }
}

fn write_string_array(out: &mut String, values: &[String]) {
    out.push('[');
    for (position, value) in values.iter().enumerate() {
        if position > 0 {
            out.push_str(", ");
        }
        out.push_str(&json_string(value));
    }
    out.push(']');
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

fn json_optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control <= '\u{1f}' => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

pub(crate) fn render_unmodified_sources(
    sources: &crate::grammar::source::SourceSet,
) -> BTreeMap<SourceId, String> {
    sources
        .iter()
        .map(|source| (source.id(), source.text().to_owned()))
        .collect()
}
