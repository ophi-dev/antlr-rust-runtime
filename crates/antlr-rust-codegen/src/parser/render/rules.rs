pub(crate) fn render_generated_rule_method(
    out: &mut String,
    rule: &GeneratedParserRule,
    step_render_context: GeneratedStepRenderContext<'_>,
) {
    let index = rule.rule_index;
    if rule.left_recursive {
        writeln!(
            out,
            "\n    #[allow(dead_code)]\n    fn parse_generated_rule_{index}(&mut self, allow_fallback: bool) -> Result<antlr4_runtime::ParseTree, GeneratedRuleError> {{"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "        self.parse_generated_rule_{index}_precedence(0, allow_fallback)"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "    }}").expect("writing to a string cannot fail");
        writeln!(
            out,
            "\n    #[allow(dead_code)]\n    fn parse_generated_rule_{index}_precedence(&mut self, __precedence: i32, allow_fallback: bool) -> Result<antlr4_runtime::ParseTree, GeneratedRuleError> {{"
        )
        .expect("writing to a string cannot fail");
    } else {
        writeln!(
            out,
            "\n    #[allow(dead_code)]\n    fn parse_generated_rule_{index}(&mut self, __precedence: i32, allow_fallback: bool) -> Result<antlr4_runtime::ParseTree, GeneratedRuleError> {{"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "        let _ = __precedence;").expect("writing to a string cannot fail");
    }
    render_generated_rule_lifecycle(out, rule, step_render_context);
    writeln!(out, "    }}").expect("writing to a string cannot fail");
}

fn render_generated_rule_lifecycle(
    out: &mut String,
    rule: &GeneratedParserRule,
    step_render_context: GeneratedStepRenderContext<'_>,
) {
    let index = rule.rule_index;
    let entry_state = rule.entry_state;
    writeln!(
        out,
        "        antlr4_runtime::__antlr4_rust_generated_rule! {{"
    )
    .expect("writing to a string cannot fail");
    if rule.left_recursive {
        writeln!(
            out,
            "            recursive self, {entry_state}isize, {index}, __precedence, allow_fallback, atn(), GeneratedRuleError::Fatal;"
        )
        .expect("writing to a string cannot fail");
    } else {
        writeln!(
            out,
            "            ordinary self, {entry_state}isize, {index}, allow_fallback, atn(), GeneratedRuleError::Fatal;"
        )
        .expect("writing to a string cannot fail");
    }
    if step_render_context
        .adaptive_atn_preferred_rule_slots
        .iter()
        .any(Option::is_some)
    {
        writeln!(
            out,
            "            retry [self.adaptive_atn_retry_slot.is_some() => GeneratedRuleError::AdaptiveRetry];"
        )
        .expect("writing to a string cannot fail");
    } else {
        writeln!(out, "            retry [none];").expect("writing to a string cannot fail");
    }
    writeln!(
        out,
        "            bind (__ctx, __rule_start, __consumed_eof, __sync_error);"
    )
    .expect("writing to a string cannot fail");
    let mut setup = String::new();
    render_portable_local_declarations(&mut setup, index, step_render_context, 4);
    render_embedded_attrs_local(&mut setup, index, step_render_context, 4);
    render_embedded_init_entry(&mut setup, index, step_render_context, 4);
    render_generated_rule_section(out, "setup", &setup);
    writeln!(out, "            body {{").expect("writing to a string cannot fail");
    render_generated_steps(out, &rule.steps, 4, step_render_context);
    writeln!(out, "            }}").expect("writing to a string cannot fail");
    let mut success = String::new();
    render_embedded_after_and_seal(&mut success, index, step_render_context, true, 4);
    render_generated_rule_section(out, "success", &success);
    let mut recovery = String::new();
    render_embedded_after_and_seal(&mut recovery, index, step_render_context, false, 4);
    render_generated_rule_section(out, "recovery", &recovery);
    writeln!(out, "        }}").expect("writing to a string cannot fail");
}

fn render_generated_rule_section(out: &mut String, name: &str, body: &str) {
    if body.is_empty() {
        writeln!(out, "            {name} {{}}").expect("writing to a string cannot fail");
        return;
    }
    writeln!(out, "            {name} {{").expect("writing to a string cannot fail");
    out.push_str(body);
    writeln!(out, "            }}").expect("writing to a string cannot fail");
}

fn render_generated_steps(
    out: &mut String,
    steps: &[GeneratedParserStep],
    indent: usize,
    render_context: GeneratedStepRenderContext<'_>,
) {
    for step in steps {
        render_generated_step(out, step, indent, render_context);
    }
}

pub(crate) fn render_generated_step(
    out: &mut String,
    step: &GeneratedParserStep,
    indent: usize,
    render_context: GeneratedStepRenderContext<'_>,
) {
    let pad = "    ".repeat(indent);
    match step {
        GeneratedParserStep::MatchToken {
            token_type,
            follow_state,
        } => {
            writeln!(
                out,
                "{pad}self.base.match_token_into({token_type}, {follow_state}, atn(), &mut __ctx, &mut __consumed_eof)?;"
            )
            .expect("writing to a string cannot fail");
        }
        GeneratedParserStep::MatchSet {
            token_set,
            intervals,
            follow_state,
        } => {
            if let Some(token_set) = token_set {
                writeln!(
                    out,
                    "{pad}self.base.match_token_set_into(atn().token_set({token_set}).expect(\"generated parser token-set index\"), {follow_state}, atn(), &mut __ctx, &mut __consumed_eof)?;"
                )
                .expect("writing to a string cannot fail");
            } else {
                let intervals = render_i32_ranges(intervals);
                writeln!(
                    out,
                    "{pad}self.base.match_set_into(&{intervals}, {follow_state}, atn(), &mut __ctx, &mut __consumed_eof)?;"
                )
                .expect("writing to a string cannot fail");
            }
        }
        GeneratedParserStep::MatchNotSet {
            token_set,
            intervals,
            follow_state,
        } => {
            if let Some(token_set) = token_set {
                writeln!(
                    out,
                    "{pad}self.base.match_not_token_set_into(atn().token_set({token_set}).expect(\"generated parser token-set index\"), 1, atn().max_token_type(), {follow_state}, atn(), &mut __ctx, &mut __consumed_eof)?;"
                )
                .expect("writing to a string cannot fail");
            } else {
                let intervals = render_i32_ranges(intervals);
                writeln!(
                    out,
                    "{pad}self.base.match_not_set_into(&{intervals}, 1, atn().max_token_type(), {follow_state}, atn(), &mut __ctx, &mut __consumed_eof)?;"
                )
                .expect("writing to a string cannot fail");
            }
        }
        GeneratedParserStep::MatchWildcard { follow_state } => {
            // A wildcard matches any single token. Model it as a not-set with an
            // empty exclusion set (every token in 1..=max), reusing the recovering
            // match so a wildcard at EOF performs ANTLR's single-token insertion
            // (`<missing ...>` error node) and lets the rule continue, instead of
            // aborting the remaining steps.
            writeln!(
                out,
                "{pad}self.base.match_not_set_into(&[], 1, atn().max_token_type(), {follow_state}, atn(), &mut __ctx, &mut __consumed_eof)?;"
            )
            .expect("writing to a string cannot fail");
        }
        GeneratedParserStep::Precedence(precedence) => {
            writeln!(out, "{pad}if !self.base.precpred({precedence}) {{")
                .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}    return Err(self.base.failed_predicate_error(\"precpred(_ctx, {precedence})\"));"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
        }
        GeneratedParserStep::Predicate {
            rule_index,
            pred_index,
        } => {
            if let Some((condition, message)) = render_context
                .portable_locals
                .and_then(|portable| portable.predicates.get(&(*rule_index, *pred_index)))
            {
                writeln!(out, "{pad}if !({condition}) {{")
                    .expect("writing to a string cannot fail");
                match message {
                    Some(message) => writeln!(
                        out,
                        "{pad}    return Err(self.base.failed_predicate_option_error({rule_index}, \"{}\".to_owned()));",
                        rust_string(message)
                    )
                    .expect("writing to a string cannot fail"),
                    None => writeln!(
                        out,
                        "{pad}    return Err(self.base.failed_predicate_error(\"semantic predicate\"));"
                    )
                    .expect("writing to a string cannot fail"),
                }
                writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
                return;
            }
            if let Some(embedded) = render_context.embedded {
                let (condition, message) =
                    embedded_predicate_condition_and_message(&embedded, *rule_index, *pred_index);
                writeln!(out, "{pad}if !({condition}) {{")
                    .expect("writing to a string cannot fail");
                match message {
                    Some(message) => writeln!(
                        out,
                        "{pad}    return Err(self.base.failed_predicate_option_error({rule_index}, \"{}\".to_owned()));",
                        rust_string(&message)
                    )
                    .expect("writing to a string cannot fail"),
                    None => writeln!(
                        out,
                        "{pad}    return Err(self.base.failed_predicate_error(\"semantic predicate\"));"
                    )
                    .expect("writing to a string cannot fail"),
                }
                writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
                return;
            }
            writeln!(
                out,
                "{pad}if !self.base.parser_semantic_ir_predicate_matches_with_context_and_local(parser_semantics(), {rule_index}, {pred_index}, &__ctx, __precedence) {{"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}    if let Some(__message) = self.base.parser_semantic_ir_predicate_failure_message({rule_index}, {pred_index}, parser_semantics()) {{"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}        return Err(self.base.failed_predicate_option_error({rule_index}, __message));"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}    return Err(self.base.failed_predicate_error(\"semantic predicate\"));"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
        }
        GeneratedParserStep::CallRule {
            source_state,
            rule_index,
            precedence,
        } => {
            let has_embedded_call_arg = render_context
                .embedded
                .and_then(|embedded| embedded.call_args.get(source_state))
                .is_some();
            if has_embedded_call_arg {
                writeln!(
                    out,
                    "{pad}let __invoking_marker = self.base.push_invoking_state({source_state}isize);"
                )
                .expect("writing to a string cannot fail");
                let expression = render_context
                    .embedded
                    .expect("has_embedded_call_arg guard ensures embedded is Some")
                    .call_args
                    .get(source_state)
                    .expect("has_embedded_call_arg guard ensures call_arg exists");
                writeln!(
                    out,
                    "{pad}self.__embedded_pending_arg = Some(i64::from({expression}));"
                )
                .expect("writing to a string cannot fail");
            }
            let precedence = match precedence {
                GeneratedRuleCallPrecedence::Literal(value) => value.to_string(),
                GeneratedRuleCallPrecedence::InheritLocal => "__precedence".to_owned(),
            };
            let from_generated_call =
                format!("self.parse_rule_precedence_from_generated({rule_index}, {precedence})");
            let mut probes_enclosing_candidate = false;
            let generated_child_call = if render_context
                .direct_generated_rule_calls
                .get(*rule_index)
                .copied()
                .unwrap_or_default()
            {
                let probe_slots = render_context
                    .adaptive_atn_probe_rule_slots
                    .get(*rule_index)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let caller_candidate_slot = render_context
                    .adaptive_atn_preferred_rule_slots
                    .get(render_context.current_rule_index)
                    .copied()
                    .flatten();
                probes_enclosing_candidate =
                    caller_candidate_slot.is_some_and(|slot| probe_slots.contains(&slot));
                let dispatch = if probes_enclosing_candidate {
                    "adaptive_probe_dispatch"
                } else {
                    "dispatch"
                };
                format!(
                    "self.parse_generated_rule_{rule_index}_{dispatch}({precedence}, false).map_err(GeneratedRuleError::into_error)"
                )
            } else {
                from_generated_call.clone()
            };
            let child_call = if render_context
                .atn_preferred_rule_calls
                .get(*rule_index)
                .copied()
                .unwrap_or_default()
            {
                from_generated_call
            } else if probes_enclosing_candidate {
                generated_child_call
            } else if let Some(slot) = render_context
                .adaptive_atn_preferred_rule_slots
                .get(*rule_index)
                .copied()
                .flatten()
            {
                format!(
                    "if self.adaptive_atn_preferred_rules[{slot}] {{ {from_generated_call} }} else {{ self.parse_generated_rule_{rule_index}_adaptive_dispatch({precedence}, false, Some({source_state}isize)).map_err(GeneratedRuleError::into_error) }}"
                )
            } else {
                generated_child_call
            };
            if has_embedded_call_arg {
                writeln!(out, "{pad}let __child = {child_call};")
                    .expect("writing to a string cannot fail");
                writeln!(
                    out,
                    "{pad}self.base.discard_invoking_state(__invoking_marker);"
                )
                .expect("writing to a string cannot fail");
                writeln!(out, "{pad}let __child = __child?;")
                    .expect("writing to a string cannot fail");
                writeln!(out, "{pad}self.base.add_parse_child(&mut __ctx, __child);")
                    .expect("writing to a string cannot fail");
            } else {
                writeln!(
                    out,
                    "{pad}antlr4_runtime::__antlr4_rust_invoke_subrule!(self, {source_state}isize, {child_call}, __ctx);"
                )
                .expect("writing to a string cannot fail");
            }
        }
        GeneratedParserStep::Action {
            source_state,
            rule_index,
            action_index,
        } => {
            if let Some(action_index) = action_index {
                writeln!(
                    out,
                    "{pad}let action = self.base.parser_action_at_current_indexed({source_state}, {rule_index}, {action_index}, __rule_start, __consumed_eof);"
                )
                .expect("writing to a string cannot fail");
            } else {
                writeln!(
                    out,
                    "{pad}let action = self.base.parser_action_at_current({source_state}, {rule_index}, __rule_start, __consumed_eof);"
                )
                .expect("writing to a string cannot fail");
            }
            if let Some(statement) = render_context.inline_action_statements.get(source_state) {
                if !statement.is_empty() {
                    writeln!(out, "{pad}{statement}").expect("writing to a string cannot fail");
                }
            }
            if render_context.embedded.is_some() {
                // Embedded actions executed inline just above, at their
                // ANTLR-correct point in the rule body.
                writeln!(out, "{pad}let _ = &action;").expect("writing to a string cannot fail");
                return;
            }
            writeln!(out, "{pad}let _ = action;").expect("writing to a string cannot fail");
        }
        GeneratedParserStep::Decision {
            state,
            decision,
            track_alt_number,
            allow_semantic_context,
            force_context,
            fast_path,
            alts,
        } => {
            render_generated_decision(
                out,
                DecisionRender {
                    state: *state,
                    decision: *decision,
                    track_alt_number: *track_alt_number,
                    allow_semantic_context: *allow_semantic_context,
                    force_context: *force_context,
                    fast_path: fast_path.as_ref(),
                    alts,
                },
                indent,
                render_context,
            );
        }
        GeneratedParserStep::StarLoop {
            state,
            decision,
            enter_alt,
            exit_alt,
            track_alt_number,
            allow_semantic_context,
            force_context,
            plus_loop,
            fast_path,
            body,
        } => {
            render_generated_star_loop(
                out,
                StarLoopRender {
                    state: *state,
                    decision: *decision,
                    alts: (*enter_alt, *exit_alt),
                    track_alt_number: *track_alt_number,
                    allow_semantic_context: *allow_semantic_context,
                    force_context: *force_context,
                    plus_loop: *plus_loop,
                    fast_path: fast_path.as_ref(),
                    body,
                },
                indent,
                render_context,
            );
        }
        GeneratedParserStep::LeftRecursiveLoop {
            state,
            decision,
            enter_alt,
            exit_alt,
            rule_index,
            entry_state,
            body,
            ..
        } => {
            render_generated_left_recursive_loop(
                out,
                LeftRecursiveLoopRender {
                    state: *state,
                    decision: *decision,
                    alts: (*enter_alt, *exit_alt),
                    rule: (*rule_index, *entry_state),
                    body,
                },
                indent,
                render_context,
            );
        }
    }
}
