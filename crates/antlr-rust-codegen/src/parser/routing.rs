// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
#[cfg(test)]
pub(crate) fn render_generated_rule_dispatch(
    rules: &[Option<GeneratedParserRule>],
    direct_generated_rule_calls: &[bool],
    inline_action_statements: &BTreeMap<usize, String>,
    track_alt_numbers: bool,
) -> String {
    render_generated_rule_dispatch_with_rule_names(
        rules,
        direct_generated_rule_calls,
        &[],
        inline_action_statements,
        track_alt_numbers,
        false,
        None,
        None,
        DecisionRoutingRender::default(),
    )
}

fn generated_force_generated_rules(
    rules: &[Option<GeneratedParserRule>],
    embedded: bool,
    portable_required_generated_rules: Option<&BTreeSet<usize>>,
) -> BTreeSet<usize> {
    if embedded {
        rules.iter().flatten().map(|rule| rule.rule_index).collect()
    } else {
        portable_required_generated_rules.map_or_else(BTreeSet::new, |required| {
            generated_rule_callers_reaching(rules, required)
        })
    }
}

/// Generated/interpreted engine selection derived from optimized parser IR.
#[derive(Debug)]
pub(crate) struct RoutingPlan {
    direct_generated_rule_calls: Vec<bool>,
    atn_preferred_rule_calls: Vec<bool>,
    adaptive_atn_preferred_rule_slots: Vec<Option<usize>>,
    adaptive_atn_probe_rule_slots: Vec<Vec<usize>>,
    adaptive_atn_preferred_rule_count: usize,
}

pub(crate) fn build_routing_plan(
    rules: &[Option<GeneratedParserRule>],
    rule_names: &[String],
    inline_action_statements: &BTreeMap<usize, String>,
    embedded: bool,
    portable_required_generated_rules: Option<&BTreeSet<usize>>,
) -> RoutingPlan {
    let direct_generated_rule_calls = rules.iter().map(Option::is_some).collect::<Vec<_>>();
    let force_generated_rules =
        generated_force_generated_rules(rules, embedded, portable_required_generated_rules);
    let atn_preferred_rule_calls =
        generated_atn_preferred_rule_calls_excluding(rules, rule_names, &force_generated_rules);
    let effectful_action_states = inline_action_statements
        .iter()
        .filter_map(|(state, statement)| (!statement.trim().is_empty()).then_some(*state))
        .collect::<BTreeSet<_>>();
    let adaptive_atn_routing = generated_adaptive_atn_routing_excluding(
        rules,
        &force_generated_rules,
        &effectful_action_states,
    );
    let adaptive_atn_preferred_rule_count = adaptive_atn_routing
        .candidates
        .iter()
        .filter(|preferred| **preferred)
        .count();
    let adaptive_atn_preferred_rule_slots = indexed_rule_slots(&adaptive_atn_routing.candidates);
    let adaptive_atn_probe_rule_slots = indexed_probe_slots(
        &adaptive_atn_routing.probe_candidate_rules,
        &adaptive_atn_preferred_rule_slots,
    );
    RoutingPlan {
        direct_generated_rule_calls,
        atn_preferred_rule_calls,
        adaptive_atn_preferred_rule_slots,
        adaptive_atn_probe_rule_slots,
        adaptive_atn_preferred_rule_count,
    }
}

impl RoutingPlan {
    pub(crate) const fn adaptive_atn_preferred_rule_count(&self) -> usize {
        self.adaptive_atn_preferred_rule_count
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_routing_plan(
    plan: &RoutingPlan,
    rules: &[Option<GeneratedParserRule>],
    inline_action_statements: &BTreeMap<usize, String>,
    track_alt_numbers: bool,
    track_context_alt_numbers: bool,
    embedded: Option<EmbeddedStepRender<'_>>,
    portable_locals: Option<PortableLocalStepRender<'_>>,
    decision_routing: DecisionRoutingRender<'_>,
) -> String {
    let mut out = String::new();
    let direct_generated_rule_calls = &plan.direct_generated_rule_calls;
    let atn_preferred_rule_calls = &plan.atn_preferred_rule_calls;
    let adaptive_atn_preferred_rule_slots = &plan.adaptive_atn_preferred_rule_slots;
    let adaptive_atn_probe_rule_slots = &plan.adaptive_atn_probe_rule_slots;
    writeln!(
        out,
        "    const __GENERATED_RULE_BODIES: [Option<antlr4_runtime::generated::GeneratedRuleBody<Self>>; {}] = [",
        rules.len()
    )
    .expect("writing to a string cannot fail");
    for rule in rules {
        match rule {
            Some(rule) if rule.left_recursive => writeln!(
                out,
                "        Some(Self::parse_generated_rule_{}_precedence),",
                rule.rule_index
            ),
            Some(rule) => writeln!(
                out,
                "        Some(Self::parse_generated_rule_{}),",
                rule.rule_index
            ),
            None => writeln!(out, "        None,"),
        }
        .expect("writing to a string cannot fail");
    }
    writeln!(out, "    ];").expect("writing to a string cannot fail");
    writeln!(
        out,
        "\n    #[allow(dead_code)]\n    #[inline(always)]\n    fn dispatch_generated_rule(&mut self, rule_index: usize, precedence: i32, allow_fallback: bool) -> Result<antlr4_runtime::ParseTree, GeneratedRuleError> {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let body = Self::__GENERATED_RULE_BODIES.get(rule_index).copied().flatten().expect(\"generated rule dispatch target\");"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        antlr4_runtime::generated::dispatch_generated_rule(self, rule_index, precedence, allow_fallback, body)"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "    }}").expect("writing to a string cannot fail");
    writeln!(
        out,
        "\n    #[allow(dead_code)]\n    fn parse_generated_rule(&mut self, rule_index: usize, precedence: i32, allow_fallback: bool) -> Option<Result<antlr4_runtime::ParseTree, GeneratedRuleError>> {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let _body = Self::__GENERATED_RULE_BODIES.get(rule_index).copied().flatten()?;"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "        match rule_index {{").expect("writing to a string cannot fail");
    for rule in rules.iter().flatten() {
        let index = rule.rule_index;
        if atn_preferred_rule_calls
            .get(index)
            .copied()
            .unwrap_or_default()
        {
            // The interpreted fast path never consults the depth cap nor
            // fires parse-listener events, so either feature overrides the
            // ATN preference: correctness of the resource limit and listener
            // coverage beat the long-call-chain optimization.
            writeln!(
                out,
                "            {index} if self.generated_only() || self.base.has_rule_depth_cap() || self.base.has_parse_listeners() => Some(self.dispatch_generated_rule({index}, precedence, allow_fallback)),"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "            {index} => None,").expect("writing to a string cannot fail");
        } else if let Some(slot) = adaptive_atn_preferred_rule_slots
            .get(index)
            .copied()
            .flatten()
        {
            // Expensive left-recursive regions start generated and switch only
            // after warmed adaptive-prediction work identifies a costly input.
            // Features implemented solely by generated bodies, and hooks whose
            // decision overrides differ between generated and interpreted
            // parsing, still override the adaptive ATN preference.
            writeln!(
                out,
                "            {index} if self.generated_only() || self.base.has_rule_depth_cap() || self.base.has_parse_listeners() || self.base.observes_parser_decisions() => Some(self.dispatch_generated_rule({index}, precedence, allow_fallback)),"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "            {index} if !self.adaptive_atn.preferred_rules[{slot}] => Some(self.parse_generated_rule_{index}_adaptive_dispatch(precedence, allow_fallback, None)),"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "            {index} => None,").expect("writing to a string cannot fail");
        }
    }
    writeln!(
        out,
        "            _ => Some(self.dispatch_generated_rule(rule_index, precedence, allow_fallback)),"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "        }}").expect("writing to a string cannot fail");
    writeln!(out, "    }}").expect("writing to a string cannot fail");
    let step_render_context = GeneratedStepRenderContext {
        current_rule_index: usize::MAX,
        embedded,
        portable_locals,
        decision_routing,
        inline_action_statements,
        track_alt_numbers,
        track_context_alt_numbers,
        direct_generated_rule_calls,
        atn_preferred_rule_calls,
        adaptive_atn_preferred_rule_slots,
        adaptive_atn_probe_rule_slots,
    };
    for rule in rules.iter().flatten() {
        let index = rule.rule_index;
        if let Some(probe_slots) = adaptive_atn_probe_rule_slots
            .get(index)
            .filter(|slots| !slots.is_empty())
        {
            writeln!(
                out,
                "\n    #[allow(dead_code)]\n    fn parse_generated_rule_{index}_adaptive_probe_dispatch(&mut self, precedence: i32, allow_fallback: bool) -> Result<antlr4_runtime::ParseTree, GeneratedRuleError> {{"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "        let __result = self.dispatch_generated_rule({index}, precedence, allow_fallback);"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "        if __result.is_ok() && self.adaptive_atn.retry_slot.is_none() {{\n            \
                 if let Some(__adaptive_after) = self.simulator\n                \
                     .as_ref()\n                \
                     .and_then(antlr4_runtime::ParserAtnSimulator::adaptive_prediction_work)\n            \
                 {{"
            )
            .expect("writing to a string cannot fail");
            for slot in probe_slots {
                writeln!(
                out,
                "                if self.adaptive_atn.preference_depths[{slot}] != 0\n                    \
                     && !self.adaptive_atn.preferred_rules[{slot}]\n                    \
                     && self.base.number_of_syntax_errors() == self.adaptive_atn.syntax_error_starts[{slot}]\n                    \
                     && antlr4_runtime::ParserAtnSimulator::adaptive_prediction_delta_is_decisive(self.adaptive_atn.preference_starts[{slot}], __adaptive_after)\n                \
                     {{\n                    \
                         self.adaptive_atn.preferred_rules[{slot}] = true;\n                    \
                         self.adaptive_atn.retry_slot = Some({slot});\n                    \
                         return Err(GeneratedRuleError::AdaptiveRetry);\n                \
                     }}"
                )
                .expect("writing to a string cannot fail");
            }
            writeln!(out, "            }}\n        }}\n        __result")
                .expect("writing to a string cannot fail");
            writeln!(out, "    }}").expect("writing to a string cannot fail");
        }
        if let Some(slot) = adaptive_atn_preferred_rule_slots
            .get(index)
            .copied()
            .flatten()
        {
            writeln!(
                out,
                "\n    #[allow(dead_code)]\n    fn parse_generated_rule_{index}_adaptive_dispatch(&mut self, precedence: i32, allow_fallback: bool, invoking_state: Option<isize>) -> Result<antlr4_runtime::ParseTree, GeneratedRuleError> {{"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "        if self.generated_only() || self.base.has_rule_depth_cap() || self.base.has_parse_listeners() || self.base.observes_parser_decisions() {{\n            \
                 return self.dispatch_generated_rule({index}, precedence, allow_fallback);\n        \
                 }}\n        \
                 let __adaptive_outermost = self.adaptive_atn.preference_depths[{slot}] == 0;\n        \
                 let __adaptive_rule_start = antlr4_runtime::IntStream::index(self.base.input());\n        \
                 let __adaptive_parser_state = self.base.state();\n        \
                 let __adaptive_diagnostic_marker = self.base.generated_diagnostics_checkpoint();"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "        if __adaptive_outermost {{\n            \
                 self.adaptive_atn.preference_starts[{slot}] = self.simulator\n                \
                     .as_ref()\n                \
                     .and_then(antlr4_runtime::ParserAtnSimulator::adaptive_prediction_work)\n                \
                     .unwrap_or((0, 0));\n        \
                 self.adaptive_atn.syntax_error_starts[{slot}] = self.base.number_of_syntax_errors();\n        \
                 }}\n        \
                 self.adaptive_atn.preference_depths[{slot}] += 1;\n        \
                 let mut __result = self.dispatch_generated_rule({index}, precedence, allow_fallback);\n        \
                 self.adaptive_atn.preference_depths[{slot}] -= 1;"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "        if !self.adaptive_atn.preferred_rules[{slot}] {{\n            \
                 if let Some(__adaptive_after) = self.simulator\n                \
                     .as_ref()\n                \
                     .and_then(antlr4_runtime::ParserAtnSimulator::adaptive_prediction_work)\n            \
                 {{\n                \
                     let __adaptive_expensive = __result.is_ok()\n                        \
                         && self.base.number_of_syntax_errors() == self.adaptive_atn.syntax_error_starts[{slot}]\n                        \
                         && antlr4_runtime::ParserAtnSimulator::adaptive_prediction_delta_is_expensive(self.adaptive_atn.preference_starts[{slot}], __adaptive_after);\n                \
                     self.adaptive_atn.preferred_rules[{slot}] = __adaptive_expensive;\n                \
                     if __adaptive_expensive {{\n                    \
                         self.adaptive_atn.retry_slot = Some({slot});\n                    \
                         __result = Err(GeneratedRuleError::AdaptiveRetry);\n                \
                     }}\n            \
                 }}\n        \
                 }}\n        \
                 if __adaptive_outermost\n            \
                     && self.adaptive_atn.retry_slot == Some({slot})\n            \
                     && matches!(&__result, Err(GeneratedRuleError::AdaptiveRetry))\n        \
                 {{\n            \
                     self.adaptive_atn.retry_slot = None;\n            \
                     self.base.restore_generated_diagnostics(__adaptive_diagnostic_marker);\n            \
                     antlr4_runtime::IntStream::seek(self.base.input(), __adaptive_rule_start);\n            \
                     self.base.set_state(__adaptive_parser_state);\n            \
                     if let Some(invoking_state) = invoking_state {{\n                \
                         self.base.push_invoking_state(invoking_state);\n            \
                     }}\n            \
                     return self.parse_rule_precedence_from_generated({index}, precedence).map_err(GeneratedRuleError::Interpreted);\n        \
                 }}"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "        __result").expect("writing to a string cannot fail");
            writeln!(out, "    }}").expect("writing to a string cannot fail");
        }
        render_generated_rule_method(
            &mut out,
            rule,
            GeneratedStepRenderContext {
                current_rule_index: index,
                ..step_render_context
            },
        );
    }
    out
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn render_generated_rule_dispatch_with_rule_names(
    rules: &[Option<GeneratedParserRule>],
    direct_generated_rule_calls: &[bool],
    rule_names: &[String],
    inline_action_statements: &BTreeMap<usize, String>,
    track_alt_numbers: bool,
    track_context_alt_numbers: bool,
    embedded: Option<EmbeddedStepRender<'_>>,
    portable_locals: Option<PortableLocalStepRender<'_>>,
    decision_routing: DecisionRoutingRender<'_>,
) -> String {
    let mut plan = build_routing_plan(
        rules,
        rule_names,
        inline_action_statements,
        embedded.is_some(),
        portable_locals.map(|portable| portable.required_generated_rules),
    );
    plan.direct_generated_rule_calls = direct_generated_rule_calls.to_vec();
    render_routing_plan(
        &plan,
        rules,
        inline_action_statements,
        track_alt_numbers,
        track_context_alt_numbers,
        embedded,
        portable_locals,
        decision_routing,
    )
}

pub(crate) fn render_portable_local_declarations(
    out: &mut String,
    rule_index: usize,
    step_render_context: GeneratedStepRenderContext<'_>,
    indent: usize,
) {
    let Some(portable) = step_render_context.portable_locals else {
        return;
    };
    let Some(declarations) = portable.declarations.get(rule_index) else {
        return;
    };
    let pad = "    ".repeat(indent);
    for declaration in declarations {
        writeln!(out, "{pad}#[allow(unused_mut)]\n{pad}{declaration}")
            .expect("writing to a string cannot fail");
    }
}

/// Declares the embedded per-rule attrs local (`__attrs`) on rule entry.
pub(crate) fn render_embedded_attrs_local(
    out: &mut String,
    rule_index: usize,
    step_render_context: GeneratedStepRenderContext<'_>,
    indent: usize,
) {
    let Some(embedded) = step_render_context.embedded else {
        return;
    };
    if !embedded
        .rule_has_attrs
        .get(rule_index)
        .copied()
        .unwrap_or_default()
    {
        return;
    }
    let pad = "    ".repeat(indent);
    let attrs_struct = embedded::attrs_struct_name(rule_index);
    writeln!(
        out,
        "{pad}#[allow(unused_mut)]\n{pad}let mut __attrs = {attrs_struct}::default();"
    )
    .expect("writing to a string cannot fail");
    if let Some(arg0) = embedded
        .rule_arg0
        .get(rule_index)
        .and_then(Option::as_deref)
    {
        writeln!(
            out,
            "{pad}if let Some(__arg) = self.__embedded_pending_arg.take() {{ __attrs.{arg0} = __arg as _; }}"
        )
        .expect("writing to a string cannot fail");
    }
}

/// Runs the embedded `@init` body at rule entry, after context/attrs setup and
/// before matching the rule body.
pub(crate) fn render_embedded_init_entry(
    out: &mut String,
    rule_index: usize,
    step_render_context: GeneratedStepRenderContext<'_>,
    indent: usize,
) {
    let Some(embedded) = step_render_context.embedded else {
        return;
    };
    if let Some(init) = embedded.init_entry.get(&rule_index) {
        let pad = "    ".repeat(indent);
        writeln!(out, "{pad}{init}").expect("writing to a string cannot fail");
    }
}

/// Runs the embedded `@after` body (committed path only — ANTLR's caught-error
/// path skips `@after`), then the authored `finally` body (every completed
/// path, matching ANTLR's try/finally ordering), and seals the attrs snapshot
/// before `finish_rule`.
pub(crate) fn render_embedded_after_and_seal(
    out: &mut String,
    rule_index: usize,
    step_render_context: GeneratedStepRenderContext<'_>,
    run_after: bool,
    indent: usize,
) {
    let Some(embedded) = step_render_context.embedded else {
        return;
    };
    let pad = "    ".repeat(indent);
    if run_after {
        if let Some(after) = embedded.after.get(&rule_index) {
            if embedded.finally_bodies.contains_key(&rule_index) {
                // With an authored `finally` following, `@after` runs behind
                // its own boundary so an early `return` in the body cannot
                // skip the finally body, the attrs seal, or rule
                // finalization (Java's try/finally ordering).
                writeln!(out, "{pad}(|| {{").expect("writing to a string cannot fail");
                writeln!(out, "{pad}{after}").expect("writing to a string cannot fail");
                writeln!(out, "{pad}}})();").expect("writing to a string cannot fail");
            } else {
                writeln!(out, "{pad}{after}").expect("writing to a string cannot fail");
            }
        }
    }
    if let Some(finally_body) = embedded.finally_bodies.get(&rule_index) {
        writeln!(out, "{pad}{finally_body}").expect("writing to a string cannot fail");
    }
    if embedded
        .rule_has_attrs
        .get(rule_index)
        .copied()
        .unwrap_or_default()
    {
        writeln!(
            out,
            "{pad}__ctx.set_generated_attrs(antlr4_runtime::GeneratedAttrs::new(__attrs.clone()));"
        )
        .expect("writing to a string cannot fail");
    }
}
