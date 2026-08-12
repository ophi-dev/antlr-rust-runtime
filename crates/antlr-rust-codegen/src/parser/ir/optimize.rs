// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
/// Validated parser IR after optional control-flow cleanup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OptimizedParserIr {
    rules: Vec<Option<GeneratedParserRule>>,
}

impl OptimizedParserIr {
    pub(crate) fn rules(&self) -> &[Option<GeneratedParserRule>] {
        &self.rules
    }
}

/// Applies the existing generated-callee closure and validates rule identity.
pub(crate) fn optimize_parser_ir(
    lowered: LoweredParserIr,
    require_generated_callees: bool,
) -> io::Result<OptimizedParserIr> {
    let mut rules = lowered.rules;
    if require_generated_callees {
        drop_rules_calling_disabled_rules(&mut rules);
    }
    for (rule_index, rule) in rules.iter().enumerate() {
        if rule
            .as_ref()
            .is_some_and(|rule| rule.rule_index != rule_index)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("lowered parser rule {rule_index} has a mismatched identity"),
            ));
        }
    }
    Ok(OptimizedParserIr { rules })
}

pub(crate) fn drop_rules_calling_disabled_rules(rules: &mut [Option<GeneratedParserRule>]) {
    loop {
        let enabled = rules.iter().map(Option::is_some).collect::<Vec<_>>();
        let drop_index = rules.iter().filter_map(Option::as_ref).find_map(|rule| {
            generated_steps_call_disabled_rule(&rule.steps, &enabled).then_some(rule.rule_index)
        });
        let Some(rule_index) = drop_index else {
            return;
        };
        rules[rule_index] = None;
    }
}
