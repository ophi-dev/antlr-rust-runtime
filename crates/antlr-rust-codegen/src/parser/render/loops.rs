pub(crate) fn render_generated_star_loop(
    out: &mut String,
    loop_info: StarLoopRender<'_>,
    indent: usize,
    render_context: GeneratedStepRenderContext<'_>,
) {
    let StarLoopRender {
        state,
        decision,
        alts,
        track_alt_number,
        allow_semantic_context,
        force_context,
        plus_loop,
        fast_path,
        body,
    } = loop_info;
    let (enter_alt, exit_alt) = alts;
    let pad = "    ".repeat(indent);
    // Opt-in `--fixed-lookahead` static dispatch (see
    // `render_generated_decision`); the miss arm renders the loop's regular
    // adaptive body.
    let static_table = (!allow_semantic_context && !force_context)
        .then(|| {
            render_context
                .decision_routing
                .static_dispatch_table(decision)
        })
        .flatten();
    // A tool-LL(1) loop decision dispatches on the tool's complete LOOK
    // table (exit alternative included), like Java's switch-driven loops.
    let ResolvedDecisionDispatch {
        complete_ll1_dispatch,
        fast_path,
    } = resolve_decision_dispatch(render_context, decision, fast_path);
    // Per-loop "iteration started" flag, threaded into `sync_decision` so it
    // recovers like ANTLR: a `*` loop's first sync is at the loop ENTRY
    // (single-token deletion), every later sync is a loop-BACK (multi-token
    // `consumeUntil`). A `+` loop's mandatory first element is iteration 1, so it
    // is already on the loop-back side at its first sync (init `true`).
    let loop_iter = format!("__loop_iter_{state}");
    writeln!(out, "{pad}let mut {loop_iter} = {plus_loop};")
        .expect("writing to a string cannot fail");
    writeln!(out, "{pad}loop {{").expect("writing to a string cannot fail");
    let inner_pad = format!("{pad}    ");
    if let Some(table) = static_table {
        render_generated_fixed_lookahead_prediction(
            out,
            &inner_pad,
            state,
            decision,
            table,
            render_context,
            &loop_iter,
            complete_ll1_dispatch,
        );
    } else if let Some(fast_path) = fast_path.filter(|_| {
        !allow_semantic_context
            && !force_context
            && !render_context
                .embedded
                .is_some_and(|embedded| embedded.adaptive_decision(decision))
    }) {
        writeln!(
            out,
            "{pad}    let mut __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "{pad}    let __prediction = match self.base.la(1) {{")
            .expect("writing to a string cannot fail");
        render_generated_fast_prediction_arms(out, &inner_pad, fast_path);
        writeln!(out, "{pad}        _ => {{").expect("writing to a string cannot fail");
        render_generated_sync_decision(out, &format!("{pad}            "), state, &loop_iter);
        writeln!(
            out,
            "{pad}            __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
        )
        .expect("writing to a string cannot fail");
        if let Some(dispatch) = complete_ll1_dispatch {
            render_generated_complete_ll1_prediction(
                out,
                &format!("{pad}            "),
                dispatch,
                false,
            );
        } else {
            render_generated_ll1_then_adaptive_prediction(
                out,
                &format!("{pad}            "),
                state,
                decision,
                false,
            );
        }
        writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
        writeln!(out, "{pad}    }};").expect("writing to a string cannot fail");
    } else {
        render_generated_sync_decision(out, &inner_pad, state, &loop_iter);
        writeln!(
            out,
            "{pad}    let __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
        )
        .expect("writing to a string cannot fail");
        let force_adaptive = render_context
            .embedded
            .is_some_and(|embedded| embedded.adaptive_decision(decision));
        if allow_semantic_context || force_context {
            render_generated_adaptive_prediction(out, &inner_pad, decision);
        } else if force_adaptive {
            render_generated_two_stage_adaptive_assignment(out, &inner_pad, decision);
        } else if let Some(dispatch) = complete_ll1_dispatch {
            render_generated_complete_ll1_prediction(out, &inner_pad, dispatch, true);
        } else {
            render_generated_ll1_then_adaptive_prediction(out, &inner_pad, state, decision, true);
        }
    }
    render_generated_loop_semantic_prediction_filter(
        out,
        &format!("{pad}    "),
        enter_alt,
        exit_alt,
        body,
        render_context.embedded,
        render_context.portable_locals,
    );
    writeln!(
        out,
        "{pad}    self.base.record_generated_prediction_diagnostic(atn(), {state}, &__prediction);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    match __prediction.alt {{").expect("writing to a string cannot fail");
    writeln!(out, "{pad}        {enter_alt} => {{").expect("writing to a string cannot fail");
    // Once an iteration is taken, every subsequent sync is a loop-back.
    writeln!(out, "{pad}            {loop_iter} = true;").expect("writing to a string cannot fail");
    render_generated_alt_number_assignments(
        out,
        &format!("{pad}            "),
        enter_alt,
        render_context.track_alt_numbers && track_alt_number,
        render_context.track_context_alt_numbers && track_alt_number,
    );
    render_generated_steps(out, body, indent + 3, render_context);
    writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}        {exit_alt} => {{").expect("writing to a string cannot fail");
    render_generated_alt_number_assignments(
        out,
        &format!("{pad}            "),
        exit_alt,
        render_context.track_alt_numbers && track_alt_number,
        render_context.track_context_alt_numbers && track_alt_number,
    );
    writeln!(out, "{pad}            break;").expect("writing to a string cannot fail");
    writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        _ => return Err(self.base.no_viable_alternative_error(__decision_start)),"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
}

fn render_generated_left_recursive_loop(
    out: &mut String,
    loop_info: LeftRecursiveLoopRender<'_>,
    indent: usize,
    render_context: GeneratedStepRenderContext<'_>,
) {
    let LeftRecursiveLoopRender {
        state,
        decision,
        alts,
        rule,
        body,
    } = loop_info;
    let (rule_index, entry_state) = rule;
    let (enter_alt, exit_alt) = alts;
    let pad = "    ".repeat(indent);
    writeln!(out, "{pad}loop {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    let __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
    )
    .expect("writing to a string cannot fail");
    if render_context.embedded.is_some() {
        writeln!(
            out,
            "{pad}    let __prediction_precedence = if __precedence <= 0 {{ 0 }} else {{ __precedence as usize }};"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "{pad}    let __prediction = match {{")
            .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}        let __simulator = self.simulator.get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new_shared(atn()));"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}        let __prediction_context = __simulator.intern_prediction_context(self.base.rule_context_version(), self.base.prediction_context_return_states(atn()));"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}        __simulator.set_exact_ambig_detection(self.base.prediction_mode() == antlr4_runtime::PredictionMode::LlExactAmbigDetection);"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}        __simulator.adaptive_predict_stream_info_with_context({decision}, __prediction_precedence, self.base.input(), __prediction_context)"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "{pad}    }} {{").expect("writing to a string cannot fail");
        writeln!(out, "{pad}        Ok(__prediction) => __prediction,")
            .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}        Err(antlr4_runtime::ParserAtnSimulatorError::NoViableAlt {{ .. }}) if self.base.left_recursive_loop_enter_matches(atn(), {state}, __precedence) => {{"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}            antlr4_runtime::ParserAtnPrediction {{ alt: {enter_alt}, requires_full_context: true, has_semantic_context: true, diagnostic: None }}"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}        Err(antlr4_runtime::ParserAtnSimulatorError::NoViableAlt {{ .. }}) => {{"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}            antlr4_runtime::ParserAtnPrediction {{ alt: {exit_alt}, requires_full_context: true, has_semantic_context: false, diagnostic: None }}"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}        Err(_) => return Err(self.base.no_viable_alternative_error(__decision_start)),"
        )
        .expect("writing to a string cannot fail");
    } else {
        writeln!(
            out,
            "{pad}    let __prediction = match self.base.left_recursive_loop_enter_prediction(atn(), {state}, __precedence) {{"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}        Some(true) => antlr4_runtime::ParserAtnPrediction {{ alt: {enter_alt}, requires_full_context: false, has_semantic_context: true, diagnostic: None }},"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}        Some(false) => antlr4_runtime::ParserAtnPrediction {{ alt: {exit_alt}, requires_full_context: false, has_semantic_context: false, diagnostic: None }},"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "{pad}        None => {{").expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}            let __prediction_precedence = if __precedence <= 0 {{ 0 }} else {{ __precedence as usize }};"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}            let __simulator = self.simulator.get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new_shared(atn()));"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}            let __prediction_context = __simulator.intern_prediction_context(self.base.rule_context_version(), self.base.prediction_context_return_states(atn()));"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}            __simulator.set_exact_ambig_detection(self.base.prediction_mode() == antlr4_runtime::PredictionMode::LlExactAmbigDetection);"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}            match __simulator.adaptive_predict_stream_info_with_context({decision}, __prediction_precedence, self.base.input(), __prediction_context) {{"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}                Ok(__prediction) => __prediction,"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}                Err(antlr4_runtime::ParserAtnSimulatorError::NoViableAlt {{ .. }}) => antlr4_runtime::ParserAtnPrediction {{ alt: {exit_alt}, requires_full_context: true, has_semantic_context: false, diagnostic: None }},"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{pad}                Err(_) => return Err(self.base.no_viable_alternative_error(__decision_start)),"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "{pad}            }}").expect("writing to a string cannot fail");
        writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
    }
    writeln!(out, "{pad}    }};").expect("writing to a string cannot fail");
    render_generated_loop_semantic_prediction_filter(
        out,
        &format!("{pad}    "),
        enter_alt,
        exit_alt,
        body,
        render_context.embedded,
        render_context.portable_locals,
    );
    writeln!(
        out,
        "{pad}    self.base.record_generated_prediction_diagnostic(atn(), {state}, &__prediction);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    match __prediction.alt {{").expect("writing to a string cannot fail");
    writeln!(out, "{pad}        {enter_alt} => {{").expect("writing to a string cannot fail");
    if render_context.embedded.is_some_and(|embedded| {
        embedded
            .rule_has_attrs
            .get(rule_index)
            .copied()
            .unwrap_or(false)
    }) {
        // Java's `_prevctx`: the outgoing iteration context becomes the new
        // context's left child, so it must carry the attrs accumulated so
        // far — operator-alt actions and typed views read them through the
        // child context, not the live `__attrs` local.
        writeln!(
            out,
            "{pad}            __ctx.set_generated_attrs(antlr4_runtime::GeneratedAttrs::new(__attrs.clone()));"
        )
        .expect("writing to a string cannot fail");
    }
    // Upstream fires `triggerExitRuleEvent` at the TOP of each operator-loop
    // pass (`recRuleSetPrevCtx`, a StarBlock iteration op) — the outgoing
    // iteration's context exits before the next expansion enters, so live
    // listener depth never accumulates across a flat operator chain. The
    // dispatch wrapper's single exit then plays `unrollRecursionContexts`'
    // one-link walk when the rule finishes.
    writeln!(
        out,
        "{pad}            self.base.parse_listener_exit_rule({rule_index});"
    )
    .expect("writing to a string cannot fail");
    // Each operator iteration deepens the tree without a rule frame; probe
    // the depth cap BEFORE the expansion push so the boundary matches the
    // dispatch site (which checks before its rule-frame push): frames and
    // expansions are both admitted up to the cap, and token-only operator
    // alternatives (no nested rule dispatch) still abort promptly instead
    // of at the top-level drain.
    writeln!(
        out,
        "{pad}            if let Some(__depth_error) = self.base.rule_depth_cap_violation() {{\n\
         {pad}                return Err(__depth_error);\n\
         {pad}            }}"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}            self.base.push_new_recursion_context_with_previous({entry_state}isize, {rule_index}, &mut __ctx);"
    )
    .expect("writing to a string cannot fail");
    // The listener enter probe fires AFTER the expansion push, mirroring
    // upstream (`pushNewRecursionContext` assigns `_ctx` before
    // `triggerEnterRuleEvent`): an abort here propagates through the rule's
    // error paths, and the dispatch wrapper's exit keeps the pair balanced.
    writeln!(
        out,
        "{pad}            if let Some(__listener_error) = self.base.parse_listener_enter_rule({rule_index}) {{\n\
         {pad}                return Err(__listener_error);\n\
         {pad}            }}"
    )
    .expect("writing to a string cannot fail");
    render_generated_steps(out, body, indent + 3, render_context);
    writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}        {exit_alt} => break,").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        _ => return Err(self.base.no_viable_alternative_error(__decision_start)),"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
}

#[allow(clippy::too_many_arguments)]
fn render_generated_loop_semantic_prediction_filter(
    out: &mut String,
    pad: &str,
    enter_alt: usize,
    exit_alt: usize,
    body: &[GeneratedParserStep],
    embedded: Option<EmbeddedStepRender<'_>>,
    portable: Option<PortableLocalStepRender<'_>>,
) {
    let Some(condition) = loop_entry_condition(body, embedded, portable) else {
        return;
    };
    writeln!(
        out,
        "{pad}let __prediction = if __prediction.alt == {enter_alt} {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    let __semantic_la = self.base.la(1);")
        .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    if {condition} {{").expect("writing to a string cannot fail");
    writeln!(out, "{pad}        __prediction").expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }} else {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        antlr4_runtime::ParserAtnPrediction {{ alt: {exit_alt}, ..__prediction }}"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}} else {{").expect("writing to a string cannot fail");
    writeln!(out, "{pad}    __prediction").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}};").expect("writing to a string cannot fail");
}

fn loop_entry_condition(
    body: &[GeneratedParserStep],
    embedded: Option<EmbeddedStepRender<'_>>,
    portable: Option<PortableLocalStepRender<'_>>,
) -> Option<String> {
    let step = body.first()?;
    match step {
        GeneratedParserStep::Predicate { .. } | GeneratedParserStep::Precedence(_) => {
            Some(semantic_alt_candidate_condition(body, embedded, portable))
        }
        GeneratedParserStep::Decision { alts, .. } => {
            if !alts.iter().any(|alt| steps_contain_predicate(alt)) {
                return None;
            }
            Some(
                alts.iter()
                    .map(|alt| {
                        format!(
                            "({})",
                            semantic_alt_candidate_condition(alt, embedded, portable)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" || "),
            )
        }
        GeneratedParserStep::Action { .. }
        | GeneratedParserStep::MatchToken { .. }
        | GeneratedParserStep::MatchSet { .. }
        | GeneratedParserStep::MatchNotSet { .. }
        | GeneratedParserStep::MatchWildcard { .. }
        | GeneratedParserStep::CallRule { .. }
        | GeneratedParserStep::StarLoop { .. }
        | GeneratedParserStep::LeftRecursiveLoop { .. } => None,
    }
}
