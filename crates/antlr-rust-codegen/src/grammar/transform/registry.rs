// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use super::analysis::TransformAnalysis;
use super::{
    GrammarTransform, TransformContext, TransformGrammar, TransformReport, TransformReportEntry,
};
use crate::grammar::action::{ActionReferenceParser, action_references};
use crate::grammar::diagnostic::Diagnostic;
use crate::grammar::model::{ModelIdAllocator, TransformId};
use crate::grammar::validation::validate_model;
use crate::optimization::metrics::integrated_grammar_metrics;

#[derive(Default)]
pub(crate) struct TransformRegistry {
    passes: Vec<Box<dyn GrammarTransform>>,
}

impl std::fmt::Debug for TransformRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransformRegistry")
            .field(
                "passes",
                &self
                    .passes
                    .iter()
                    .map(|pass| pass.name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl TransformRegistry {
    pub(crate) fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    pub(crate) fn push(&mut self, pass: impl GrammarTransform + 'static) {
        self.push_boxed(Box::new(pass));
    }

    pub(crate) fn push_boxed(&mut self, pass: Box<dyn GrammarTransform>) {
        self.passes.push(pass);
    }

    pub(crate) fn run(
        &self,
        grammar: &mut TransformGrammar,
        ids: &mut ModelIdAllocator,
        report_only: bool,
    ) -> Result<TransformReport, Diagnostic> {
        self.run_with_action_reference_parser(grammar, ids, report_only, action_references)
    }

    pub(crate) fn run_with_action_reference_parser(
        &self,
        grammar: &mut TransformGrammar,
        ids: &mut ModelIdAllocator,
        report_only: bool,
        action_reference_parser: ActionReferenceParser,
    ) -> Result<TransformReport, Diagnostic> {
        if report_only {
            let mut projected_grammar = grammar.clone();
            let mut projected_ids = ids.clone();
            return self.run_mutating(
                &mut projected_grammar,
                &mut projected_ids,
                true,
                action_reference_parser,
            );
        }
        self.run_mutating(grammar, ids, false, action_reference_parser)
    }

    fn run_mutating(
        &self,
        grammar: &mut TransformGrammar,
        ids: &mut ModelIdAllocator,
        report_only: bool,
        action_reference_parser: ActionReferenceParser,
    ) -> Result<TransformReport, Diagnostic> {
        let mut report = TransformReport::default();
        let mut analysis = TransformAnalysis::compute(&grammar.units);
        for (index, pass) in self.passes.iter().enumerate() {
            let id = TransformId::new(index as u32);
            let before = integrated_grammar_metrics(&grammar.units);
            let changed = pass.apply(
                &TransformContext {
                    id,
                    analysis: &analysis,
                    report_only,
                    action_reference_parser,
                },
                grammar,
                ids,
                &mut report,
            )?;
            if changed {
                validate_model(grammar)?;
                analysis.invalidate(pass.invalidates());
                analysis.recompute(&grammar.units);
            }
            report.entries.push(TransformReportEntry {
                id,
                name: pass.name(),
                safety: pass.safety_class(),
                before,
                after: integrated_grammar_metrics(&grammar.units),
                changed,
            });
        }
        Ok(report)
    }
}
