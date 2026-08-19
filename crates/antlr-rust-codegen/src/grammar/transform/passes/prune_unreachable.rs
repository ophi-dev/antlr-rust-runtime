// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use std::collections::BTreeSet;

use crate::grammar::diagnostic::Diagnostic;
use crate::grammar::model::{ModelIdAllocator, RuleId};
use crate::grammar::rule_reachability::{EntryRuleConfig, analyze};
use crate::grammar::transform::analysis::AnalysisInvalidation;
use crate::grammar::transform::clone::tombstone_rule;
use crate::grammar::transform::{
    GrammarTransform, SafetyClass, TransformContext, TransformGrammar, TransformReport,
    TransformRuleRemoval,
};

pub(crate) struct PruneUnreachableRules {
    entries: EntryRuleConfig,
}

impl PruneUnreachableRules {
    pub(crate) const NAME: &'static str = "prune-unreachable-rules";

    pub(crate) const fn new(entries: EntryRuleConfig) -> Self {
        Self { entries }
    }
}

impl GrammarTransform for PruneUnreachableRules {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::RecognitionPreserving
    }

    fn invalidates(&self) -> AnalysisInvalidation {
        AnalysisInvalidation::ALL
    }

    fn apply(
        &self,
        input: &TransformContext<'_>,
        grammar: &mut TransformGrammar,
        _ids: &mut ModelIdAllocator,
        report: &mut TransformReport,
    ) -> Result<bool, Diagnostic> {
        let mut changed = false;
        for unit in &mut grammar.units {
            if !grammar.target_units.contains(&unit.id) {
                continue;
            }
            let unreachable = analyze(unit, &self.entries)
                .unreachable_rules
                .into_iter()
                .collect::<BTreeSet<_>>();
            if unreachable.is_empty() {
                continue;
            }
            for rule in unit
                .rules
                .iter()
                .filter(|rule| unreachable.contains(&rule.id))
            {
                report.rule_removals.push(TransformRuleRemoval {
                    pass: input.id,
                    grammar: unit.name.clone(),
                    rule: rule.name.clone(),
                    source_span: rule.name_span.clone(),
                });
                tombstone_rule(
                    &mut grammar.provenance,
                    rule,
                    "unreachable parser rule pruned",
                    &[],
                );
            }
            unit.rules.retain(|rule| !unreachable.contains(&rule.id));
            remove_mode_rule_references(unit, &unreachable);
            changed = true;
        }
        Ok(changed)
    }
}

fn remove_mode_rule_references(
    unit: &mut crate::grammar::model::GrammarUnit,
    removed: &BTreeSet<RuleId>,
) {
    for mode in &mut unit.modes {
        mode.rules.retain(|rule| !removed.contains(rule));
    }
}
