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

pub(crate) struct AdaptiveAtnParserRenderSlots {
    pub(crate) struct_field: String,
    pub(crate) field_init: String,
    pub(crate) reset: &'static str,
    pub(crate) retry_variant: &'static str,
    pub(crate) retry_into_error: &'static str,
}

pub(crate) fn adaptive_atn_parser_render_slots(
    preferred_rule_count: usize,
) -> AdaptiveAtnParserRenderSlots {
    if preferred_rule_count == 0 {
        return AdaptiveAtnParserRenderSlots {
            struct_field: String::new(),
            field_init: String::new(),
            reset: "",
            retry_variant: "",
            retry_into_error: "",
        };
    }
    AdaptiveAtnParserRenderSlots {
        struct_field: format!(
            "    adaptive_atn_preferred_rules: [bool; {preferred_rule_count}],\n    adaptive_atn_preference_depths: [usize; {preferred_rule_count}],\n    adaptive_atn_preference_starts: [(usize, usize); {preferred_rule_count}],\n    adaptive_atn_syntax_error_starts: [usize; {preferred_rule_count}],\n    adaptive_atn_retry_slot: Option<usize>,\n"
        ),
        field_init: format!(
            "            adaptive_atn_preferred_rules: [false; {preferred_rule_count}],\n            adaptive_atn_preference_depths: [0; {preferred_rule_count}],\n            adaptive_atn_preference_starts: [(0, 0); {preferred_rule_count}],\n            adaptive_atn_syntax_error_starts: [0; {preferred_rule_count}],\n            adaptive_atn_retry_slot: None,\n"
        ),
        reset: "        parser.adaptive_atn_preferred_rules.fill(false);\n        parser.adaptive_atn_preference_depths.fill(0);\n        parser.adaptive_atn_preference_starts.fill((0, 0));\n        parser.adaptive_atn_syntax_error_starts.fill(0);\n        parser.adaptive_atn_retry_slot = None;\n",
        retry_variant: "    AdaptiveRetry,\n",
        retry_into_error: "            Self::AdaptiveRetry => antlr4_runtime::AntlrError::Unsupported(\"internal adaptive ATN retry escaped its routing boundary\".to_owned()),\n",
    }
}

pub(crate) fn render_generated_rule_error(
    retry_variant: &str,
    retry_into_error: &str,
) -> String {
    format!(
        r#"#[allow(dead_code)]
#[derive(Debug)]
enum GeneratedRuleError {{
    Fatal(antlr4_runtime::AntlrError),
    Interpreted(antlr4_runtime::AntlrError),
{retry_variant}}}

impl GeneratedRuleError {{
    fn into_error(self) -> antlr4_runtime::AntlrError {{
        match self {{
            Self::Fatal(error) | Self::Interpreted(error) => error,
{retry_into_error}        }}
    }}
}}"#
    )
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
        "    #[allow(dead_code)]\n    fn parse_generated_rule(&mut self, rule_index: usize, precedence: i32, allow_fallback: bool) -> Option<Result<antlr4_runtime::ParseTree, GeneratedRuleError>> {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "        let _ = precedence;").expect("writing to a string cannot fail");
    writeln!(out, "        let _ = allow_fallback;").expect("writing to a string cannot fail");
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
                "            {index} if self.generated_only() || self.base.has_rule_depth_cap() || self.base.has_parse_listeners() => Some(self.parse_generated_rule_{index}_dispatch(precedence, allow_fallback)),"
            )
            .expect("writing to a string cannot fail");
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
                "            {index} if self.generated_only() || self.base.has_rule_depth_cap() || self.base.has_parse_listeners() || self.base.observes_parser_decisions() => Some(self.parse_generated_rule_{index}_dispatch(precedence, allow_fallback)),"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "            {index} if !self.adaptive_atn_preferred_rules[{slot}] => Some(self.parse_generated_rule_{index}_adaptive_dispatch(precedence, allow_fallback, None)),"
            )
            .expect("writing to a string cannot fail");
        } else {
            writeln!(
                out,
                "            {index} => Some(self.parse_generated_rule_{index}_dispatch(precedence, allow_fallback)),"
            )
            .expect("writing to a string cannot fail");
        }
    }
    writeln!(out, "            _ => None,").expect("writing to a string cannot fail");
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
                "        let __result = self.parse_generated_rule_{index}_dispatch(precedence, allow_fallback);"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "        if __result.is_ok() && self.adaptive_atn_retry_slot.is_none() {{\n            \
                 if let Some(__adaptive_after) = self.simulator\n                \
                     .as_ref()\n                \
                     .and_then(antlr4_runtime::ParserAtnSimulator::adaptive_prediction_work)\n            \
                 {{"
            )
            .expect("writing to a string cannot fail");
            for slot in probe_slots {
                writeln!(
                out,
                "                if self.adaptive_atn_preference_depths[{slot}] != 0\n                    \
                     && !self.adaptive_atn_preferred_rules[{slot}]\n                    \
                     && self.base.number_of_syntax_errors() == self.adaptive_atn_syntax_error_starts[{slot}]\n                    \
                     && antlr4_runtime::ParserAtnSimulator::adaptive_prediction_delta_is_decisive(self.adaptive_atn_preference_starts[{slot}], __adaptive_after)\n                \
                     {{\n                    \
                         self.adaptive_atn_preferred_rules[{slot}] = true;\n                    \
                         self.adaptive_atn_retry_slot = Some({slot});\n                    \
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
                 return self.parse_generated_rule_{index}_dispatch(precedence, allow_fallback);\n        \
                 }}\n        \
                 let __adaptive_outermost = self.adaptive_atn_preference_depths[{slot}] == 0;\n        \
                 let __adaptive_rule_start = antlr4_runtime::IntStream::index(self.base.input());\n        \
                 let __adaptive_parser_state = self.base.state();\n        \
                 let __adaptive_diagnostic_marker = self.base.generated_diagnostics_checkpoint();"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "        if __adaptive_outermost {{\n            \
                 self.adaptive_atn_preference_starts[{slot}] = self.simulator\n                \
                     .as_ref()\n                \
                     .and_then(antlr4_runtime::ParserAtnSimulator::adaptive_prediction_work)\n                \
                     .unwrap_or((0, 0));\n        \
                 self.adaptive_atn_syntax_error_starts[{slot}] = self.base.number_of_syntax_errors();\n        \
                 }}\n        \
                 self.adaptive_atn_preference_depths[{slot}] += 1;\n        \
                 let mut __result = self.parse_generated_rule_{index}_dispatch(precedence, allow_fallback);\n        \
                 self.adaptive_atn_preference_depths[{slot}] -= 1;"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "        if !self.adaptive_atn_preferred_rules[{slot}] {{\n            \
                 if let Some(__adaptive_after) = self.simulator\n                \
                     .as_ref()\n                \
                     .and_then(antlr4_runtime::ParserAtnSimulator::adaptive_prediction_work)\n            \
                 {{\n                \
                     let __adaptive_expensive = __result.is_ok()\n                        \
                         && self.base.number_of_syntax_errors() == self.adaptive_atn_syntax_error_starts[{slot}]\n                        \
                         && antlr4_runtime::ParserAtnSimulator::adaptive_prediction_delta_is_expensive(self.adaptive_atn_preference_starts[{slot}], __adaptive_after);\n                \
                     self.adaptive_atn_preferred_rules[{slot}] = __adaptive_expensive;\n                \
                     if __adaptive_expensive {{\n                    \
                         self.adaptive_atn_retry_slot = Some({slot});\n                    \
                         __result = Err(GeneratedRuleError::AdaptiveRetry);\n                \
                     }}\n            \
                 }}\n        \
                 }}\n        \
                 if __adaptive_outermost\n            \
                     && self.adaptive_atn_retry_slot == Some({slot})\n            \
                     && matches!(&__result, Err(GeneratedRuleError::AdaptiveRetry))\n        \
                 {{\n            \
                     self.adaptive_atn_retry_slot = None;\n            \
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
        writeln!(
            out,
            "\n    #[allow(dead_code)]\n    fn parse_generated_rule_{index}_dispatch(&mut self, precedence: i32, allow_fallback: bool) -> Result<antlr4_runtime::ParseTree, GeneratedRuleError> {{"
        )
        .expect("writing to a string cannot fail");
        let target_call = if rule.left_recursive {
            format!("self.parse_generated_rule_{index}_precedence(precedence, allow_fallback)")
        } else {
            writeln!(out, "        let _ = precedence;").expect("writing to a string cannot fail");
            format!("self.parse_generated_rule_{index}(precedence, allow_fallback)")
        };
        // Rule nesting maps onto native call depth; sample remaining stack
        // capacity at the shared dispatch boundary so deeply nested input
        // grows onto a segmented stack instead of aborting the process. The
        // optional depth-cap and parse-listener probes stay inline-cheap
        // (one `Option`/emptiness check each when unused) and their errors,
        // while absorbed by rule-level recovery like any rule failure, stay
        // sticky until the top-level entry drains them — that pairing is
        // what actually enforces the abort. The matching listener exit event
        // fires from the rule body's exit paths (`finish_rule`/recovery), so
        // enter/exit stay balanced. Plain `if let` keeps generated output
        // edition-2021 compatible.
        writeln!(
            out,
            "        if let Some(error) = self.base.rule_depth_cap_violation() {{\n            \
             return Err(GeneratedRuleError::Fatal(error));\n        \
             }}\n        \
             if let Some(error) = self.base.parse_listener_enter_rule({index}) {{\n            \
             return Err(GeneratedRuleError::Fatal(error));\n        \
             }}\n        \
             let __listener_result = if self.base.generated_rule_stack_check_due() {{\n            \
             antlr4_runtime::grow_generated_rule_stack(|| {target_call})\n        \
             }} else {{\n            {target_call}\n        }};\n        \
             self.base.parse_listener_exit_rule({index});\n        \
             __listener_result"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "    }}").expect("writing to a string cannot fail");
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
) {
    let Some(portable) = step_render_context.portable_locals else {
        return;
    };
    let Some(declarations) = portable.declarations.get(rule_index) else {
        return;
    };
    for declaration in declarations {
        writeln!(out, "        #[allow(unused_mut)]\n        {declaration}")
            .expect("writing to a string cannot fail");
    }
}

/// Declares the embedded per-rule attrs local (`__attrs`) on rule entry.
pub(crate) fn render_embedded_attrs_local(
    out: &mut String,
    rule_index: usize,
    step_render_context: GeneratedStepRenderContext<'_>,
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
    let attrs_struct = embedded::attrs_struct_name(rule_index);
    writeln!(
        out,
        "        #[allow(unused_mut)]\n        let mut __attrs = {attrs_struct}::default();"
    )
    .expect("writing to a string cannot fail");
    if let Some(arg0) = embedded
        .rule_arg0
        .get(rule_index)
        .and_then(Option::as_deref)
    {
        writeln!(
            out,
            "        if let Some(__arg) = self.__embedded_pending_arg.take() {{ __attrs.{arg0} = __arg as _; }}"
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
) {
    let Some(embedded) = step_render_context.embedded else {
        return;
    };
    if let Some(init) = embedded.init_entry.get(&rule_index) {
        writeln!(out, "        {init}").expect("writing to a string cannot fail");
    }
}

/// Runs the embedded `@after` body (committed path only — ANTLR's caught-error
/// path skips `@after`) and seals the attrs snapshot before `finish_rule`.
pub(crate) fn render_embedded_after_and_seal(
    out: &mut String,
    rule_index: usize,
    step_render_context: GeneratedStepRenderContext<'_>,
    run_after: bool,
) {
    let Some(embedded) = step_render_context.embedded else {
        return;
    };
    if run_after {
        if let Some(after) = embedded.after.get(&rule_index) {
            writeln!(out, "                {after}").expect("writing to a string cannot fail");
        }
    }
    if embedded
        .rule_has_attrs
        .get(rule_index)
        .copied()
        .unwrap_or_default()
    {
        writeln!(
            out,
            "                __ctx.set_generated_attrs(antlr4_runtime::GeneratedAttrs::new(__attrs.clone()));"
        )
        .expect("writing to a string cannot fail");
    }
}

pub(crate) fn render_generated_adaptive_retry_unwind(
    out: &mut String,
    step_render_context: GeneratedStepRenderContext<'_>,
    left_recursive: bool,
) {
    if !step_render_context
        .adaptive_atn_preferred_rule_slots
        .iter()
        .any(Option::is_some)
    {
        return;
    }
    let exit_rule = if left_recursive {
        "self.base.unroll_recursion_context();"
    } else {
        "self.base.exit_rule();"
    };
    writeln!(
        out,
        "                if self.adaptive_atn_retry_slot.is_some() {{\n                    \
         {exit_rule}\n                    \
         self.base.restore_generated_diagnostics(__generated_diagnostic_marker);\n                    \
         return Err(GeneratedRuleError::AdaptiveRetry);\n                \
         }}"
    )
    .expect("writing to a string cannot fail");
}
