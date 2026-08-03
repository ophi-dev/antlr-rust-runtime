use std::collections::BTreeSet;

use crate::cli::CompilerConfig;
use crate::error::Error;
use crate::grammar::compiler::Compilation;
use crate::grammar::rule_reachability::EntryRuleConfig;
use crate::grammar::transform::passes::precedence_ladder::CollapsePrecedenceLadders;
use crate::grammar::transform::passes::prune_unreachable::PruneUnreachableRules;
use crate::grammar::transform::{GrammarTransform, TransformRegistry};

use super::descriptor::{
    COLLAPSE_PRECEDENCE_LADDERS, OptimizationStage, PRUNE_UNREACHABLE_RULES, PassDescriptor,
};
use super::report::pruned_unreachable_rule_messages;

type MessageRenderer = fn(&Compilation, &PassDescriptor) -> Vec<String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PassExecution {
    Apply,
    ReportOnly,
}

struct ConfiguredGrammarPass {
    transform: Box<dyn GrammarTransform>,
    execution: PassExecution,
    emit_manifest: bool,
    messages: Option<MessageRenderer>,
}

struct PassRegistration {
    descriptor: &'static PassDescriptor,
    configure: fn(&SelectionSettings) -> Option<ConfiguredGrammarPass>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SelectionSettings {
    entry_rules: EntryRuleConfig,
    prune_unreachable: bool,
    precedence_ladders: Option<PassExecution>,
}

impl SelectionSettings {
    fn from_compiler_config(config: &CompilerConfig) -> Self {
        let precedence_ladders = if config.report_precedence_ladders {
            Some(PassExecution::ReportOnly)
        } else if config.optimize_precedence_ladders {
            Some(PassExecution::Apply)
        } else {
            None
        };
        Self {
            entry_rules: EntryRuleConfig::new(config.entry_rules.iter().cloned()),
            prune_unreachable: config.prune_unreachable,
            precedence_ladders,
        }
    }
}

pub(super) struct SelectedPass {
    pub(super) descriptor: &'static PassDescriptor,
    pub(super) execution: PassExecution,
    pub(super) emit_manifest: bool,
    pub(super) messages: Option<MessageRenderer>,
}

pub(crate) struct OptimizationPlan {
    pub(super) entry_rules: EntryRuleConfig,
    pub(super) transforms: TransformRegistry,
    pub(super) selected: Vec<SelectedPass>,
    pub(super) report_only: bool,
}

const PASS_REGISTRY: &[PassRegistration] = &[
    PassRegistration {
        descriptor: &PRUNE_UNREACHABLE_RULES,
        configure: configure_prune_unreachable,
    },
    PassRegistration {
        descriptor: &COLLAPSE_PRECEDENCE_LADDERS,
        configure: configure_precedence_ladders,
    },
];

impl OptimizationPlan {
    pub(crate) fn from_compiler_config(config: &CompilerConfig) -> Result<Self, Error> {
        Self::from_settings(SelectionSettings::from_compiler_config(config))
    }

    fn from_settings(settings: SelectionSettings) -> Result<Self, Error> {
        let mut registrations = PASS_REGISTRY.iter().collect::<Vec<_>>();
        registrations.sort_by_key(|registration| registration.descriptor.canonical_order);
        validate_registry(&registrations)?;

        let mut transforms = TransformRegistry::default();
        let mut selected = Vec::new();
        for registration in registrations {
            let Some(configured) = (registration.configure)(&settings) else {
                continue;
            };
            validate_configured_pass(registration.descriptor, configured.transform.as_ref())?;
            transforms.push_boxed(configured.transform);
            selected.push(SelectedPass {
                descriptor: registration.descriptor,
                execution: configured.execution,
                emit_manifest: configured.emit_manifest,
                messages: configured.messages,
            });
        }
        validate_selection(&selected)?;
        let report_only = selected
            .iter()
            .any(|pass| pass.execution == PassExecution::ReportOnly);
        Ok(Self {
            entry_rules: settings.entry_rules,
            transforms,
            selected,
            report_only,
        })
    }

    pub(crate) const fn entry_rules(&self) -> &EntryRuleConfig {
        &self.entry_rules
    }

    pub(crate) const fn transforms(&self) -> &TransformRegistry {
        &self.transforms
    }

    pub(crate) const fn report_only(&self) -> bool {
        self.report_only
    }
}

fn configure_prune_unreachable(settings: &SelectionSettings) -> Option<ConfiguredGrammarPass> {
    settings.prune_unreachable.then(|| ConfiguredGrammarPass {
        transform: Box::new(PruneUnreachableRules::new(settings.entry_rules.clone())),
        execution: PassExecution::Apply,
        emit_manifest: false,
        messages: Some(pruned_unreachable_rule_messages),
    })
}

fn configure_precedence_ladders(settings: &SelectionSettings) -> Option<ConfiguredGrammarPass> {
    settings
        .precedence_ladders
        .map(|execution| ConfiguredGrammarPass {
            transform: Box::new(CollapsePrecedenceLadders),
            execution,
            emit_manifest: true,
            messages: None,
        })
}

fn validate_registry(registrations: &[&PassRegistration]) -> Result<(), Error> {
    let mut ids = BTreeSet::new();
    let mut orders = BTreeSet::new();
    for registration in registrations {
        let descriptor = registration.descriptor;
        if descriptor.stage != OptimizationStage::IntegratedGrammar {
            return Err(invalid_registry(&format!(
                "pass {} is not an integrated-grammar pass",
                descriptor.id
            )));
        }
        if !ids.insert(descriptor.id) {
            return Err(invalid_registry(&format!(
                "duplicate stable pass ID {}",
                descriptor.id
            )));
        }
        if !orders.insert(descriptor.canonical_order) {
            return Err(invalid_registry(&format!(
                "duplicate canonical order {}",
                descriptor.canonical_order
            )));
        }
    }
    for registration in registrations {
        let descriptor = registration.descriptor;
        for related in descriptor.prerequisites.iter().chain(descriptor.conflicts) {
            if !ids.contains(related) {
                return Err(invalid_registry(&format!(
                    "pass {} references unknown pass {related}",
                    descriptor.id
                )));
            }
        }
    }
    Ok(())
}

fn validate_configured_pass(
    descriptor: &PassDescriptor,
    transform: &dyn GrammarTransform,
) -> Result<(), Error> {
    if descriptor.id != transform.name() {
        return Err(invalid_registry(&format!(
            "descriptor ID {} does not match transform ID {}",
            descriptor.id,
            transform.name()
        )));
    }
    if descriptor.safety != transform.safety_class() {
        return Err(invalid_registry(&format!(
            "descriptor safety for {} does not match its transform",
            descriptor.id
        )));
    }
    Ok(())
}

fn validate_selection(selected: &[SelectedPass]) -> Result<(), Error> {
    let ids = selected
        .iter()
        .map(|pass| pass.descriptor.id)
        .collect::<BTreeSet<_>>();
    for pass in selected {
        for prerequisite in pass.descriptor.prerequisites {
            if !ids.contains(prerequisite) {
                return Err(Error::configuration(format!(
                    "optimization pass {} requires {prerequisite}",
                    pass.descriptor.id
                )));
            }
        }
        for conflict in pass.descriptor.conflicts {
            if ids.contains(conflict) {
                return Err(Error::configuration(format!(
                    "optimization pass {} conflicts with {conflict}",
                    pass.descriptor.id
                )));
            }
        }
    }
    Ok(())
}

fn invalid_registry(message: &str) -> Error {
    Error::configuration(format!("invalid optimization pass registry: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_passes_follow_canonical_descriptor_order() {
        let plan = OptimizationPlan::from_settings(SelectionSettings {
            entry_rules: EntryRuleConfig::default(),
            prune_unreachable: true,
            precedence_ladders: Some(PassExecution::Apply),
        })
        .expect("registered optimization passes should be valid");

        assert_eq!(
            plan.selected
                .iter()
                .map(|pass| (pass.descriptor.id, pass.descriptor.canonical_order))
                .collect::<Vec<_>>(),
            [
                (PruneUnreachableRules::NAME, 100),
                (CollapsePrecedenceLadders::NAME, 200),
            ]
        );
        assert!(!plan.report_only());
    }

    #[test]
    fn report_only_selection_is_effective_for_the_whole_transform_projection() {
        let plan = OptimizationPlan::from_settings(SelectionSettings {
            entry_rules: EntryRuleConfig::default(),
            prune_unreachable: true,
            precedence_ladders: Some(PassExecution::ReportOnly),
        })
        .expect("registered optimization passes should be valid");

        assert!(plan.report_only());
        assert_eq!(plan.selected.len(), 2);
    }
}
