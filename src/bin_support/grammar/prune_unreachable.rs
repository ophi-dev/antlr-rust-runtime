use std::collections::BTreeSet;

use super::diagnostic::Diagnostic;
use super::model::{Block, ElementKind, ModelIdAllocator, Rule, RuleId};
use super::provenance::{ProvenanceIndex, Tombstone};
use super::rule_reachability::{EntryRuleConfig, analyze};
use super::transform::{
    GrammarTransform, SafetyClass, TransformContext, TransformGrammar, TransformReport,
    TransformRuleRemoval,
};
use super::transform_analysis::AnalysisInvalidation;

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
                tombstone_rule(&mut grammar.provenance, rule);
            }
            unit.rules.retain(|rule| !unreachable.contains(&rule.id));
            remove_mode_rule_references(unit, &unreachable);
            changed = true;
        }
        Ok(changed)
    }
}

fn remove_mode_rule_references(unit: &mut super::model::GrammarUnit, removed: &BTreeSet<RuleId>) {
    for mode in &mut unit.modes {
        mode.rules.retain(|rule| !removed.contains(rule));
    }
}

fn tombstone_rule(provenance: &mut ProvenanceIndex, rule: &Rule) {
    tombstone(provenance, rule.syntax);
    for action in &rule.actions {
        tombstone(provenance, action.syntax);
    }
    for handler in &rule.catches {
        tombstone(provenance, handler.syntax);
    }
    if let Some(action) = &rule.finally_action {
        tombstone(provenance, action.syntax);
    }
    tombstone_block(provenance, &rule.block);
}

fn tombstone_block(provenance: &mut ProvenanceIndex, block: &Block) {
    for alternative in &block.alternatives {
        tombstone(provenance, alternative.syntax);
        if let Some(label) = &alternative.label {
            tombstone(provenance, label.syntax);
        }
        for element in &alternative.elements {
            tombstone(provenance, element.syntax);
            if let Some(label) = &element.label {
                tombstone(provenance, label.syntax);
            }
            if let ElementKind::Block(nested) = &element.kind {
                tombstone_block(provenance, nested);
            }
        }
    }
}

fn tombstone(provenance: &mut ProvenanceIndex, syntax: super::frontend::SyntaxId) {
    provenance.tombstone(
        syntax,
        Tombstone {
            phase: "optional-transform",
            reason: "unreachable parser rule pruned",
            replacements: Box::new([]),
        },
    );
}
