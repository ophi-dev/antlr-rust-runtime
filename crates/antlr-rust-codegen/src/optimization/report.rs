// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use std::collections::BTreeSet;

use crate::grammar::compiler::Compilation;
use crate::grammar::transform::render_optimization_manifest;

use super::config::OptimizationPlan;
use super::descriptor::PassDescriptor;

impl OptimizationPlan {
    pub(crate) fn diagnostic_messages(&self, compilation: &Compilation) -> Vec<String> {
        if self.report_only {
            return Vec::new();
        }
        self.selected
            .iter()
            .filter_map(|pass| {
                pass.messages
                    .map(|render| render(compilation, pass.descriptor))
            })
            .flatten()
            .collect()
    }

    pub(crate) fn render_manifest(&self, compilation: &Compilation) -> Option<String> {
        self.selected
            .iter()
            .any(|pass| pass.emit_manifest)
            .then(|| {
                render_optimization_manifest(
                    &compilation.transform_report,
                    &compilation.sources,
                    self.report_only,
                )
            })
    }
}

pub(super) fn pruned_unreachable_rule_messages(
    compilation: &Compilation,
    descriptor: &PassDescriptor,
) -> Vec<String> {
    let pass_ids = compilation
        .transform_report
        .entries
        .iter()
        .filter(|entry| entry.name == descriptor.id)
        .map(|entry| entry.id)
        .collect::<BTreeSet<_>>();
    compilation
        .transform_report
        .rule_removals
        .iter()
        .filter(|removal| pass_ids.contains(&removal.pass))
        .map(|removal| {
            let source = compilation.sources.get(removal.source_span.source);
            let path = source.map_or_else(
                || "<grammar>".to_owned(),
                |source| source.logical_path().display().to_string(),
            );
            let position = source
                .and_then(|source| source.line_column(removal.source_span.bytes.start))
                .map_or_else(String::new, |(line, column)| format!(":{line}:{column}"));
            format!(
                "pruned: {path}{position}: unreachable parser rule {}.{}",
                removal.grammar, removal.rule
            )
        })
        .collect()
}
