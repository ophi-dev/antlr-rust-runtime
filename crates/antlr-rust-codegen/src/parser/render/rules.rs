pub(crate) fn render_generated_rule_method(
    out: &mut String,
    rule: &GeneratedParserRule,
    step_render_context: GeneratedStepRenderContext<'_>,
) {
    if rule.left_recursive {
        render_generated_left_recursive_rule_method(out, rule, step_render_context);
        return;
    }
    let index = rule.rule_index;
    let entry_state = rule.entry_state;
    writeln!(
        out,
        "\n    #[allow(dead_code)]\n    fn parse_generated_rule_{index}(&mut self, __precedence: i32, allow_fallback: bool) -> Result<antlr4_runtime::ParseTree, GeneratedRuleError> {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "        let _ = __precedence;").expect("writing to a string cannot fail");
    writeln!(out, "        let _ = allow_fallback;").expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let __generated_diagnostic_marker = self.base.generated_diagnostics_checkpoint();"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let mut __ctx = self.base.enter_rule({entry_state}isize, {index});"
    )
    .expect("writing to a string cannot fail");
    // Capture the rule start AFTER `enter_rule`, which advances the cursor past any
    // leading hidden-channel tokens to the first visible token. Capturing before
    // would make `$start`/`$text` in generated actions include a leading hidden
    // prefix (e.g. whitespace), diverging from ANTLR and the rule context start.
    writeln!(
        out,
        "        let __rule_start = antlr4_runtime::IntStream::index(self.base.input());"
    )
    .expect("writing to a string cannot fail");
    render_portable_local_declarations(out, index, step_render_context);
    render_embedded_attrs_local(out, index, step_render_context);
    render_embedded_init_entry(out, index, step_render_context);
    writeln!(out, "        let mut __consumed_eof = false;")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let mut __sync_error: Option<antlr4_runtime::AntlrError> = None;"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let __result = (|| -> Result<(), antlr4_runtime::AntlrError> {{"
    )
    .expect("writing to a string cannot fail");
    render_generated_steps(out, &rule.steps, 3, step_render_context);
    writeln!(out, "            Ok(())").expect("writing to a string cannot fail");
    writeln!(out, "        }})();").expect("writing to a string cannot fail");
    writeln!(out, "        match __result {{").expect("writing to a string cannot fail");
    writeln!(out, "            Ok(()) => {{").expect("writing to a string cannot fail");
    render_embedded_after_and_seal(out, index, step_render_context, true);
    writeln!(
        out,
        "                let __tree = self.base.finish_rule(__ctx, __consumed_eof);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                Ok(__tree)").expect("writing to a string cannot fail");
    writeln!(out, "            }}").expect("writing to a string cannot fail");
    writeln!(out, "            Err(__error) => {{").expect("writing to a string cannot fail");
    render_generated_adaptive_retry_unwind(out, step_render_context, false);
    // A rule's own `sync_decision` failure (`__sync_error`) is fatal ONLY at the
    // top-level public entry (`allow_fallback`). When this rule is a nested child
    // (`!allow_fallback`), ANTLR recovers the mismatch INSIDE the child and returns
    // a partial subtree to the parent — it never propagates the sync failure up. So
    // for a nested child, recover locally like any other body error (a `Fatal`
    // escaping here would make the parent recover on ITS context, dropping the
    // child subtree). Only the true top-level keeps the `Fatal` abort (preserving
    // antlr#6 `InvalidEmptyInput`-style start-rule errors).
    writeln!(
        out,
        "                if let Some(__error) = __sync_error {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                    if allow_fallback {{")
        .expect("writing to a string cannot fail");
    writeln!(out, "                        self.base.exit_rule();")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "                        self.base.rollback_generated_tree(__generated_diagnostic_marker);"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "                        self.base.record_generated_syntax_error();"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "                        return Err(GeneratedRuleError::Fatal(__error));"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                    }}").expect("writing to a string cannot fail");
    writeln!(
        out,
        "                    self.base.recover_generated_rule(&mut __ctx, atn(), __error);"
    )
    .expect("writing to a string cannot fail");
    render_embedded_after_and_seal(out, index, step_render_context, false);
    writeln!(
        out,
        "                    let __tree = self.base.finish_rule(__ctx, __consumed_eof);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                    return Ok(__tree);")
        .expect("writing to a string cannot fail");
    writeln!(out, "                }}").expect("writing to a string cannot fail");
    writeln!(
        out,
        "                self.base.recover_generated_rule(&mut __ctx, atn(), __error);"
    )
    .expect("writing to a string cannot fail");
    render_embedded_after_and_seal(out, index, step_render_context, false);
    writeln!(
        out,
        "                let __tree = self.base.finish_rule(__ctx, __consumed_eof);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                Ok(__tree)").expect("writing to a string cannot fail");
    writeln!(out, "            }}").expect("writing to a string cannot fail");
    writeln!(out, "        }}").expect("writing to a string cannot fail");
    writeln!(out, "    }}").expect("writing to a string cannot fail");
}

fn render_generated_left_recursive_rule_method(
    out: &mut String,
    rule: &GeneratedParserRule,
    step_render_context: GeneratedStepRenderContext<'_>,
) {
    let index = rule.rule_index;
    let entry_state = rule.entry_state;
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
    writeln!(out, "        let _ = allow_fallback;").expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let __generated_diagnostic_marker = self.base.generated_diagnostics_checkpoint();"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let mut __ctx = self.base.enter_recursion_rule({entry_state}isize, {index}, __precedence);"
    )
    .expect("writing to a string cannot fail");
    // Capture the rule start AFTER `enter_recursion_rule`, which (via `enter_rule`)
    // advances the cursor past any leading hidden-channel tokens to the first
    // visible token. Capturing before would make `$start`/`$text` in generated
    // actions include a leading hidden prefix, diverging from ANTLR.
    writeln!(
        out,
        "        let __rule_start = antlr4_runtime::IntStream::index(self.base.input());"
    )
    .expect("writing to a string cannot fail");
    render_portable_local_declarations(out, index, step_render_context);
    render_embedded_attrs_local(out, index, step_render_context);
    render_embedded_init_entry(out, index, step_render_context);
    writeln!(out, "        let mut __consumed_eof = false;")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let mut __sync_error: Option<antlr4_runtime::AntlrError> = None;"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let __result = (|| -> Result<(), antlr4_runtime::AntlrError> {{"
    )
    .expect("writing to a string cannot fail");
    render_generated_steps(out, &rule.steps, 3, step_render_context);
    writeln!(out, "            Ok(())").expect("writing to a string cannot fail");
    writeln!(out, "        }})();").expect("writing to a string cannot fail");
    writeln!(out, "        match __result {{").expect("writing to a string cannot fail");
    writeln!(out, "            Ok(()) => {{").expect("writing to a string cannot fail");
    render_embedded_after_and_seal(out, index, step_render_context, true);
    writeln!(
        out,
        "                let __tree = self.base.finish_recursion_rule(__ctx, __consumed_eof);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                Ok(__tree)").expect("writing to a string cannot fail");
    writeln!(out, "            }}").expect("writing to a string cannot fail");
    writeln!(out, "            Err(__error) => {{").expect("writing to a string cannot fail");
    render_generated_adaptive_retry_unwind(out, step_render_context, true);
    // Same as the non-left-recursive case: a nested child (`!allow_fallback`)
    // recovers its own sync failure internally and returns a partial subtree; only
    // the top-level entry propagates `Fatal`. Use `finish_recursion_rule` (which
    // unrolls the recursion context) in the recover branch — do NOT also call
    // `unroll_recursion_context` (that would double-unroll).
    writeln!(
        out,
        "                if let Some(__error) = __sync_error {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                    if allow_fallback {{")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "                        self.base.unroll_recursion_context();"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "                        self.base.rollback_generated_tree(__generated_diagnostic_marker);"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "                        self.base.record_generated_syntax_error();"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "                        return Err(GeneratedRuleError::Fatal(__error));"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                    }}").expect("writing to a string cannot fail");
    writeln!(
        out,
        "                    self.base.recover_generated_rule(&mut __ctx, atn(), __error);"
    )
    .expect("writing to a string cannot fail");
    render_embedded_after_and_seal(out, index, step_render_context, false);
    writeln!(
        out,
        "                    let __tree = self.base.finish_recursion_rule(__ctx, __consumed_eof);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                    return Ok(__tree);")
        .expect("writing to a string cannot fail");
    writeln!(out, "                }}").expect("writing to a string cannot fail");
    writeln!(
        out,
        "                self.base.recover_generated_rule(&mut __ctx, atn(), __error);"
    )
    .expect("writing to a string cannot fail");
    render_embedded_after_and_seal(out, index, step_render_context, false);
    writeln!(
        out,
        "                let __tree = self.base.finish_recursion_rule(__ctx, __consumed_eof);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                Ok(__tree)").expect("writing to a string cannot fail");
    writeln!(out, "            }}").expect("writing to a string cannot fail");
    writeln!(out, "        }}").expect("writing to a string cannot fail");
    writeln!(out, "    }}").expect("writing to a string cannot fail");
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
                "{pad}let __match = self.base.match_token_recovering({token_type}, {follow_state}, atn())?;"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}__consumed_eof |= __match.consumed_eof();")
                .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}for __child in __match.into_child_iter() {{ self.base.add_parse_child(&mut __ctx, __child); }}"
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
                    "{pad}let __match = self.base.match_token_set_recovering(atn().token_set({token_set}).expect(\"generated parser token-set index\"), {follow_state}, atn())?;"
                )
                .expect("writing to a string cannot fail");
            } else {
                let intervals = render_i32_ranges(intervals);
                writeln!(
                    out,
                    "{pad}let __match = self.base.match_set_recovering(&{intervals}, {follow_state}, atn())?;"
                )
                .expect("writing to a string cannot fail");
            }
            writeln!(out, "{pad}__consumed_eof |= __match.consumed_eof();")
                .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}for __child in __match.into_child_iter() {{ self.base.add_parse_child(&mut __ctx, __child); }}"
            )
            .expect("writing to a string cannot fail");
        }
        GeneratedParserStep::MatchNotSet {
            token_set,
            intervals,
            follow_state,
        } => {
            if let Some(token_set) = token_set {
                writeln!(
                    out,
                    "{pad}let __match = self.base.match_not_token_set_recovering(atn().token_set({token_set}).expect(\"generated parser token-set index\"), 1, atn().max_token_type(), {follow_state}, atn())?;"
                )
                .expect("writing to a string cannot fail");
            } else {
                let intervals = render_i32_ranges(intervals);
                writeln!(
                    out,
                    "{pad}let __match = self.base.match_not_set_recovering(&{intervals}, 1, atn().max_token_type(), {follow_state}, atn())?;"
                )
                .expect("writing to a string cannot fail");
            }
            writeln!(out, "{pad}__consumed_eof |= __match.consumed_eof();")
                .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}for __child in __match.into_child_iter() {{ self.base.add_parse_child(&mut __ctx, __child); }}"
            )
            .expect("writing to a string cannot fail");
        }
        GeneratedParserStep::MatchWildcard { follow_state } => {
            // A wildcard matches any single token. Model it as a not-set with an
            // empty exclusion set (every token in 1..=max), reusing the recovering
            // match so a wildcard at EOF performs ANTLR's single-token insertion
            // (`<missing ...>` error node) and lets the rule continue, instead of
            // aborting the remaining steps.
            writeln!(
                out,
                "{pad}let __match = self.base.match_not_set_recovering(&[], 1, atn().max_token_type(), {follow_state}, atn())?;"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}__consumed_eof |= __match.consumed_eof();")
                .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}for __child in __match.into_child_iter() {{ self.base.add_parse_child(&mut __ctx, __child); }}"
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
            writeln!(
                out,
                "{pad}let __invoking_marker = self.base.push_invoking_state({source_state}isize);"
            )
            .expect("writing to a string cannot fail");
            if let Some(embedded) = render_context.embedded {
                if let Some(expression) = embedded.call_args.get(source_state) {
                    writeln!(
                        out,
                        "{pad}self.__embedded_pending_arg = Some(i64::from({expression}));"
                    )
                    .expect("writing to a string cannot fail");
                }
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
                // ATN-preferred child: route through `parse_rule_precedence_from_generated`.
                // The rule's `parse_generated_rule` dispatch arm is guarded by
                // `generated_only() || has_rule_depth_cap() || has_parse_listeners()`:
                // in the default configuration the generated probe returns `None`
                // and the wrapper parses the child on the INTERPRETED path
                // (preserving the ATN-preferred optimization); a configured depth
                // cap or a registered parse listener flips it to the generated
                // body, the only path that enforces the cap and fires events.
                from_generated_call
            } else if probes_enclosing_candidate {
                // The child is also a candidate for direct entry paths, but
                // this call belongs to an enclosing wrapper candidate. Probe
                // that wrapper and keep it as the sole retry boundary.
                generated_child_call
            } else if let Some(slot) = render_context
                .adaptive_atn_preferred_rule_slots
                .get(*rule_index)
                .copied()
                .flatten()
            {
                // Keep the direct generated call while prediction remains
                // cheap. Once the warmed simulator crosses the cost threshold,
                // enter through the wrapper so its guarded dispatch can select
                // the interpreted rule without reordering buffered actions.
                format!(
                    "if self.adaptive_atn_preferred_rules[{slot}] {{ {from_generated_call} }} else {{ self.parse_generated_rule_{rule_index}_adaptive_dispatch({precedence}, false, Some({source_state}isize)).map_err(GeneratedRuleError::into_error) }}"
                )
            } else {
                generated_child_call
            };
            writeln!(out, "{pad}let __child = {child_call};")
                .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}self.base.discard_invoking_state(__invoking_marker);"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}let __child = __child?;").expect("writing to a string cannot fail");
            writeln!(out, "{pad}self.base.add_parse_child(&mut __ctx, __child);")
                .expect("writing to a string cannot fail");
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
