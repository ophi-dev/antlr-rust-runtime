pub(crate) fn render_generated_decision(
    out: &mut String,
    decision_info: DecisionRender<'_>,
    indent: usize,
    render_context: GeneratedStepRenderContext<'_>,
) {
    let DecisionRender {
        state,
        decision,
        track_alt_number,
        allow_semantic_context,
        force_context,
        fast_path,
        alts,
    } = decision_info;
    let pad = "    ".repeat(indent);
    // Opt-in `--fixed-lookahead` static dispatch replaces the prediction
    // block entirely; its fall-through arm renders the decision's regular
    // adaptive body, so unproven lookahead behaves exactly as untiered.
    let static_table = (!allow_semantic_context && !force_context)
        .then(|| {
            render_context
                .decision_routing
                .static_dispatch_table(decision)
        })
        .flatten();
    // A tool-LL(1) decision dispatches on the tool's complete LOOK table
    // (exit alternatives included), like Java's switch compilation.
    let ResolvedDecisionDispatch {
        complete_ll1_dispatch,
        fast_path,
    } = resolve_decision_dispatch(render_context, decision, fast_path);
    if let Some(table) = static_table {
        render_generated_fixed_lookahead_prediction(
            out,
            &pad,
            state,
            decision,
            table,
            render_context,
            "false",
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
            "{pad}let mut __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
        )
        .expect("writing to a string cannot fail");
        writeln!(out, "{pad}let __prediction = match self.base.la(1) {{")
            .expect("writing to a string cannot fail");
        render_generated_fast_prediction_arms(out, &pad, fast_path);
        writeln!(out, "{pad}    _ => {{").expect("writing to a string cannot fail");
        // A non-loop block/optional decision is never a loop-back: ANTLR syncs it
        // like BLOCK_START (single-token deletion), so pass `false`.
        render_generated_sync_decision(out, &format!("{pad}        "), state, "false");
        writeln!(
            out,
            "{pad}        __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
        )
        .expect("writing to a string cannot fail");
        render_generated_post_sync_prediction(
            out,
            &format!("{pad}        "),
            state,
            decision,
            render_context,
            complete_ll1_dispatch,
            false,
        );
        writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
        writeln!(out, "{pad}}};").expect("writing to a string cannot fail");
    } else {
        if !allow_semantic_context {
            render_generated_sync_decision(out, &pad, state, "false");
        }
        writeln!(
            out,
            "{pad}let __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
        )
        .expect("writing to a string cannot fail");
        let force_adaptive = render_context
            .embedded
            .is_some_and(|embedded| embedded.adaptive_decision(decision));
        if allow_semantic_context || force_context {
            render_generated_adaptive_prediction(out, &pad, decision);
        } else {
            let _ = force_adaptive;
            render_generated_post_sync_prediction(
                out,
                &pad,
                state,
                decision,
                render_context,
                complete_ll1_dispatch,
                true,
            );
        }
    }
    if allow_semantic_context {
        render_generated_semantic_prediction_filter(
            out,
            &pad,
            alts,
            render_context.embedded,
            render_context.portable_locals,
        );
        render_generated_decision_diagnostic_report(
            out,
            &pad,
            state,
            alts,
            render_context.embedded,
            render_context.portable_locals,
        );
    } else {
        writeln!(
            out,
            "{pad}self.base.record_generated_prediction_diagnostic(atn(), {state}, &__prediction);"
        )
        .expect("writing to a string cannot fail");
    }
    writeln!(out, "{pad}match __prediction.alt {{").expect("writing to a string cannot fail");
    for (index, steps) in alts.iter().enumerate() {
        let alt = index + 1;
        writeln!(out, "{pad}    {alt} => {{").expect("writing to a string cannot fail");
        render_generated_alt_number_assignments(
            out,
            &format!("{pad}        "),
            alt,
            render_context.track_alt_numbers && track_alt_number,
            render_context.track_context_alt_numbers && track_alt_number,
        );
        render_generated_steps(out, steps, indent + 2, render_context);
        writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    }
    writeln!(
        out,
        "{pad}    _ => return Err(self.base.no_viable_alternative_error(__decision_start)),"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
}

fn render_generated_fast_prediction_arms(
    out: &mut String,
    pad: &str,
    fast_path: &GeneratedDecisionFastPath,
) {
    for arm in &fast_path.arms {
        let patterns = render_i32_match_patterns(&arm.intervals);
        let alt = arm.alt;
        writeln!(
            out,
            "{pad}    {patterns} => antlr4_runtime::ParserAtnPrediction {{ alt: {alt}, requires_full_context: false, has_semantic_context: false, diagnostic: None }},"
        )
        .expect("writing to a string cannot fail");
    }
}

fn render_generated_complete_ll1_prediction(
    out: &mut String,
    pad: &str,
    dispatch: &CompleteLl1Dispatch,
    assign: bool,
) {
    let prefix = if assign { "let __prediction = " } else { "" };
    let suffix = if assign { ";" } else { "" };
    writeln!(out, "{pad}{prefix}match self.base.la(1) {{")
        .expect("writing to a string cannot fail");
    render_generated_fast_prediction_arms(out, pad, &dispatch.fast_path);
    if let Some(default_alt) = dispatch.default_alt {
        writeln!(
            out,
            "{pad}    _ => antlr4_runtime::ParserAtnPrediction {{ alt: {default_alt}, requires_full_context: false, has_semantic_context: false, diagnostic: None }},"
        )
        .expect("writing to a string cannot fail");
    } else {
        writeln!(
            out,
            "{pad}    _ => return Err(self.base.no_viable_alternative_error(__decision_start)),"
        )
        .expect("writing to a string cannot fail");
    }
    writeln!(out, "{pad}}}{suffix}").expect("writing to a string cannot fail");
}

/// Two-stage adaptive prediction without the LL(1) shortcut — Java's plain
/// `adaptivePredict`: the SLL probe resolves or flags a full-context
/// conflict, and the retry with real outer context only runs when the
/// parser's prediction mode allows it (never in SLL mode).
fn render_generated_two_stage_adaptive_assignment(out: &mut String, pad: &str, decision: usize) {
    writeln!(out, "{pad}let __prediction = {{").expect("writing to a string cannot fail");
    render_generated_sll_then_context_prediction_with_indent(out, pad, decision, 1);
    writeln!(out, "{pad}}};").expect("writing to a string cannot fail");
}

fn render_generated_ll1_then_adaptive_prediction(
    out: &mut String,
    pad: &str,
    state: usize,
    decision: usize,
    assign: bool,
) {
    let prefix = if assign { "let __prediction = " } else { "" };
    let suffix = if assign { ";" } else { "" };
    writeln!(
        out,
        "{pad}{prefix}if let Some(__prediction) = self.base.ll1_decision_prediction(atn(), {state}) {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    __prediction").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}} else {{").expect("writing to a string cannot fail");
    render_generated_sll_then_context_prediction_with_indent(out, pad, decision, 1);
    writeln!(out, "{pad}}}{suffix}").expect("writing to a string cannot fail");
}

#[allow(clippy::too_many_arguments)]
fn render_generated_post_sync_prediction(
    out: &mut String,
    pad: &str,
    state: usize,
    decision: usize,
    render_context: GeneratedStepRenderContext<'_>,
    complete_ll1_dispatch: Option<&CompleteLl1Dispatch>,
    assign: bool,
) {
    let plans = render_context
        .decision_routing
        .shared_descent_plans(decision);
    if plans.is_empty() {
        render_generated_post_sync_fallback(
            out,
            pad,
            (state, decision),
            render_context,
            complete_ll1_dispatch,
            assign,
        );
    } else {
        render_generated_shared_descent_prediction(
            out,
            pad,
            state,
            decision,
            plans,
            render_context,
            complete_ll1_dispatch,
            assign,
        );
    }
}

fn render_generated_post_sync_fallback(
    out: &mut String,
    pad: &str,
    decision: (usize, usize),
    render_context: GeneratedStepRenderContext<'_>,
    complete_ll1_dispatch: Option<&CompleteLl1Dispatch>,
    assign: bool,
) {
    let (state, decision) = decision;
    if render_context
        .embedded
        .is_some_and(|embedded| embedded.adaptive_decision(decision))
    {
        if assign {
            render_generated_two_stage_adaptive_assignment(out, pad, decision);
        } else {
            render_generated_sll_then_context_prediction_with_indent(out, pad, decision, 0);
        }
    } else if let Some(dispatch) = complete_ll1_dispatch {
        render_generated_complete_ll1_prediction(out, pad, dispatch, assign);
    } else {
        render_generated_ll1_then_adaptive_prediction(out, pad, state, decision, assign);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_generated_shared_descent_prediction(
    out: &mut String,
    pad: &str,
    state: usize,
    decision: usize,
    plans: &[SharedDescentPlan],
    render_context: GeneratedStepRenderContext<'_>,
    complete_ll1_dispatch: Option<&CompleteLl1Dispatch>,
    assign: bool,
) {
    let prefix = if assign { "let __prediction = " } else { "" };
    let suffix = if assign { ";" } else { "" };
    writeln!(out, "{pad}{prefix}{{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    let __shared_descent_prediction = '__shared_descent_{decision}: {{"
    )
    .expect("writing to a string cannot fail");
    for plan in plans {
        render_shared_descent_attempt(out, &format!("{pad}        "), plan);
    }
    writeln!(out, "{pad}        break '__shared_descent_{decision} None;")
        .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }};").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    if let Some(__shared_descent_prediction) = __shared_descent_prediction {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}        __shared_descent_prediction")
        .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }} else {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        self.base.record_shared_descent_adaptive_fallback();"
    )
    .expect("writing to a string cannot fail");
    render_generated_post_sync_fallback(
        out,
        &format!("{pad}        "),
        (state, decision),
        render_context,
        complete_ll1_dispatch,
        false,
    );
    writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}}{suffix}").expect("writing to a string cannot fail");
}

fn render_shared_descent_attempt(out: &mut String, pad: &str, plan: &SharedDescentPlan) {
    let prefix_checks = plan
        .prefix_tokens
        .iter()
        .enumerate()
        .map(|(index, token)| format!("self.base.la({}) == {token}", index + 1));
    let trigger_patterns = render_i32_match_patterns(&plan.trigger_intervals);
    let trigger_depth = plan.prefix_tokens.len() + 1;
    let condition = prefix_checks
        .chain(std::iter::once(format!(
            "matches!(self.base.la({trigger_depth}), {trigger_patterns})"
        )))
        .collect::<Vec<_>>()
        .join(" && ");
    writeln!(out, "{pad}if {condition} {{").expect("writing to a string cannot fail");
    let preview = render_shared_descent_preview(plan);
    let transaction_pad = preview.as_ref().map_or_else(
        || pad.to_owned(),
        |preview| {
            writeln!(
                out,
                "{pad}    let __shared_descent_preview_alt: Option<usize> = {preview};"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}    if let Some(__shared_descent_preview_alt) = __shared_descent_preview_alt {{"
            )
            .expect("writing to a string cannot fail");
            format!("{pad}    ")
        },
    );
    writeln!(
        out,
        "{transaction_pad}    if let Some(__shared_descent_marker) = self.base.begin_shared_descent({}) {{",
        plan.prefix_tokens.len()
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{transaction_pad}        let __shared_invoking_marker = self.base.push_invoking_state({}isize);",
        plan.call_site
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{transaction_pad}        let __shared_descent_result = self.parse_generated_rule_{}_dispatch(0, false);",
        plan.common_rule
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{transaction_pad}        self.base.discard_invoking_state(__shared_invoking_marker);"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{transaction_pad}        match __shared_descent_result {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{transaction_pad}            Ok(__shared_descent_node) if self.base.shared_descent_parse_is_clean(&__shared_descent_marker) => {{"
    )
    .expect("writing to a string cannot fail");
    let success_pad = format!("{transaction_pad}                ");
    if preview.is_some() {
        render_shared_descent_commit(out, &success_pad, plan, "__shared_descent_preview_alt");
    } else {
        writeln!(
            out,
            "{success_pad}let __shared_descent_tail = self.base.la(1);"
        )
        .expect("writing to a string cannot fail");
        writeln!(
            out,
            "{success_pad}let __shared_descent_alt = match __shared_descent_tail {{"
        )
        .expect("writing to a string cannot fail");
        render_shared_descent_tail_arms(out, &format!("{success_pad}    "), plan, false);
        writeln!(out, "{success_pad}}};").expect("writing to a string cannot fail");
        writeln!(
            out,
            "{success_pad}if let Some(__shared_descent_alt) = __shared_descent_alt {{"
        )
        .expect("writing to a string cannot fail");
        render_shared_descent_commit(
            out,
            &format!("{success_pad}    "),
            plan,
            "__shared_descent_alt",
        );
        writeln!(out, "{success_pad}}}").expect("writing to a string cannot fail");
        writeln!(
            out,
            "{success_pad}self.base.rollback_shared_descent(&__shared_descent_marker, false);"
        )
        .expect("writing to a string cannot fail");
    }
    writeln!(out, "{transaction_pad}            }}").expect("writing to a string cannot fail");
    writeln!(out, "{transaction_pad}            _ => {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{transaction_pad}                self.base.rollback_shared_descent(&__shared_descent_marker, true);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{transaction_pad}            }}").expect("writing to a string cannot fail");
    writeln!(out, "{transaction_pad}        }}").expect("writing to a string cannot fail");
    writeln!(out, "{transaction_pad}    }}").expect("writing to a string cannot fail");
    if preview.is_some() {
        writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    }
    writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
}

fn render_shared_descent_preview(plan: &SharedDescentPlan) -> Option<String> {
    let token_length = plan.common_token_length?;
    let tail_depth = plan
        .prefix_tokens
        .len()
        .checked_add(token_length)?
        .checked_add(1)?;
    let mut out = format!(
        "{{ let __shared_descent_preview_tail = self.base.la({tail_depth}); match __shared_descent_preview_tail {{ "
    );
    render_shared_descent_tail_arms(&mut out, "", plan, true);
    out.push_str("} }");
    Some(out)
}

fn render_shared_descent_tail_arms(
    out: &mut String,
    pad: &str,
    plan: &SharedDescentPlan,
    preview: bool,
) {
    for tail in &plan.tails {
        let patterns = render_i32_match_patterns(&tail.intervals);
        if tail.guard_against_follow {
            let tail_name = if preview {
                "__shared_descent_preview_tail"
            } else {
                "__shared_descent_tail"
            };
            writeln!(
                out,
                "{pad}{patterns} if self.base.shared_descent_guard_allows(atn(), {tail_name}) => Some({}),",
                tail.alt
            )
            .expect("writing to a string cannot fail");
        } else {
            writeln!(out, "{pad}{patterns} => Some({}),", tail.alt)
                .expect("writing to a string cannot fail");
        }
    }
    if let Some(default_alt) = plan.default_alt {
        let tail_name = if preview {
            "__shared_descent_preview_tail"
        } else {
            "__shared_descent_tail"
        };
        let exclusion = if plan.default_excluded_intervals.is_empty() {
            String::new()
        } else {
            let patterns = render_i32_match_patterns(&plan.default_excluded_intervals);
            format!("!matches!({tail_name}, {patterns}) && ")
        };
        writeln!(
            out,
            "{pad}_ if {exclusion}self.base.shared_descent_follow_contains(atn(), {tail_name}) => Some({default_alt}),"
        )
        .expect("writing to a string cannot fail");
    }
    writeln!(out, "{pad}_ => None,").expect("writing to a string cannot fail");
}

fn render_shared_descent_commit(out: &mut String, pad: &str, plan: &SharedDescentPlan, alt: &str) {
    let call_sites = plan
        .resume_call_sites
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(
        out,
        "{pad}self.base.commit_shared_descent(&__shared_descent_marker, __shared_descent_node, {}, &[{call_sites}]);",
        plan.common_rule
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}break '__shared_descent_{} Some(antlr4_runtime::ParserAtnPrediction {{ alt: {alt}, requires_full_context: false, has_semantic_context: false, diagnostic: None }});",
        plan.decision
    )
    .expect("writing to a string cannot fail");
}

/// `--fixed-lookahead`: a static `la(1)..la(k)` dispatch trie whose hits
/// commit an alternative without touching the simulator.
///
/// A hit skips the decision's recovery synchronization, which is safe
/// because every first-token arm is pre-restricted to the decision's
/// within-rule lookahead — exactly the set for which `sync_decision`
/// early-returns without doing recovery work (the same shape the default
/// partial fast path ships today). Every other token — including
/// context-dependent loop exits fall through to synchronization. A complete
/// LL(1) decision then reuses its proven-total dispatch; fixed/adaptive
/// decisions retain their regular adaptive body.
#[allow(clippy::too_many_arguments)]
fn render_generated_fixed_lookahead_prediction(
    out: &mut String,
    pad: &str,
    state: usize,
    decision: usize,
    table: &FixedLookaheadTable,
    render_context: GeneratedStepRenderContext<'_>,
    loop_sync_flag: &str,
    complete_ll1_dispatch: Option<&CompleteLl1Dispatch>,
) {
    writeln!(
        out,
        "{pad}let mut __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
    )
    .expect("writing to a string cannot fail");
    write!(out, "{pad}let __fixed_lookahead_alt: Option<usize> = ")
        .expect("writing to a string cannot fail");
    render_fixed_lookahead_node(out, pad, &table.root, 1);
    writeln!(out, ";").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}let __prediction = if let Some(__fixed_lookahead_alt) = __fixed_lookahead_alt {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    antlr4_runtime::ParserAtnPrediction {{ alt: __fixed_lookahead_alt, requires_full_context: false, has_semantic_context: false, diagnostic: None }}"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}}} else {{").expect("writing to a string cannot fail");
    let inner_pad = format!("{pad}    ");
    render_generated_sync_decision(out, &inner_pad, state, loop_sync_flag);
    writeln!(
        out,
        "{inner_pad}__decision_start = antlr4_runtime::IntStream::index(self.base.input());"
    )
    .expect("writing to a string cannot fail");
    if let Some(dispatch) = complete_ll1_dispatch {
        render_generated_complete_ll1_prediction(out, &inner_pad, dispatch, false);
    } else if render_context
        .embedded
        .is_some_and(|embedded| embedded.adaptive_decision(decision))
    {
        render_generated_sll_then_context_prediction_with_indent(out, pad, decision, 1);
    } else {
        render_generated_ll1_then_adaptive_prediction(out, &inner_pad, state, decision, false);
    }
    writeln!(out, "{pad}}};").expect("writing to a string cannot fail");
}

/// Renders one dispatch-trie node as an expression. `node_pad` is the
/// indentation of the node's closing brace; `depth` is the 1-based
/// lookahead position this node probes.
fn render_fixed_lookahead_node(
    out: &mut String,
    node_pad: &str,
    node: &FixedLookaheadNode,
    depth: usize,
) {
    match node {
        FixedLookaheadNode::Alt(alt) => {
            write!(out, "Some({alt})").expect("writing to a string cannot fail");
        }
        FixedLookaheadNode::Probe(arms) => {
            writeln!(out, "match self.base.la({depth}) {{")
                .expect("writing to a string cannot fail");
            let arm_pad = format!("{node_pad}    ");
            for (intervals, child) in arms {
                let patterns = render_i32_match_patterns(intervals);
                write!(out, "{arm_pad}{patterns} => ").expect("writing to a string cannot fail");
                render_fixed_lookahead_node(out, &arm_pad, child, depth + 1);
                writeln!(out, ",").expect("writing to a string cannot fail");
            }
            writeln!(out, "{arm_pad}_ => None,").expect("writing to a string cannot fail");
            write!(out, "{node_pad}}}").expect("writing to a string cannot fail");
        }
    }
}

fn render_generated_decision_diagnostic_report(
    out: &mut String,
    pad: &str,
    state: usize,
    alts: &[Vec<GeneratedParserStep>],
    embedded: Option<EmbeddedStepRender<'_>>,
    portable: Option<PortableLocalStepRender<'_>>,
) {
    let alt_conditions = alts
        .iter()
        .map(|steps| {
            semantic_alt_candidate_condition_with_la(steps, "__diagnostic_la", embedded, portable)
        })
        .collect::<Vec<_>>();
    if alt_conditions
        .iter()
        .any(|condition| condition == "true" || condition == "false")
    {
        return;
    }
    writeln!(out, "{pad}if self.base.report_diagnostic_errors() {{")
        .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    let __diagnostic_la = self.base.la(1);")
        .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    let mut __diagnostic_alts = Vec::new();")
        .expect("writing to a string cannot fail");
    for (index, condition) in alt_conditions.iter().enumerate() {
        let alt = index + 1;
        writeln!(out, "{pad}    if {condition} {{").expect("writing to a string cannot fail");
        writeln!(out, "{pad}        __diagnostic_alts.push({alt});")
            .expect("writing to a string cannot fail");
        writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    }
    writeln!(
        out,
        "{pad}    self.base.record_generated_ambiguity_diagnostic(atn(), {state}, __decision_start, __decision_start, &__diagnostic_alts);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
}

fn render_generated_semantic_prediction_filter(
    out: &mut String,
    pad: &str,
    alts: &[Vec<GeneratedParserStep>],
    embedded: Option<EmbeddedStepRender<'_>>,
    portable: Option<PortableLocalStepRender<'_>>,
) {
    let alt_has_predicates = alts
        .iter()
        .map(|steps| !leading_predicates(steps, portable).is_empty())
        .collect::<Vec<_>>();
    if !alt_has_predicates
        .iter()
        .any(|has_predicate| *has_predicate)
    {
        return;
    }
    let alt_conditions = alts
        .iter()
        .map(|steps| semantic_alt_candidate_condition(steps, embedded, portable))
        .collect::<Vec<_>>();
    writeln!(
        out,
        "{pad}let __prediction = if __prediction.has_semantic_context {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    let __semantic_la = self.base.la(1);")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    let __semantic_alt = match __prediction.alt {{"
    )
    .expect("writing to a string cannot fail");
    for (index, condition) in alt_conditions.iter().enumerate() {
        if !alt_has_predicates[index] {
            continue;
        }
        let alt = index + 1;
        writeln!(out, "{pad}        {alt} if {condition} => Some({alt}),")
            .expect("writing to a string cannot fail");
        writeln!(out, "{pad}        {alt} => {{").expect("writing to a string cannot fail");
        render_semantic_alt_search(out, pad, &alt_conditions, alts, portable);
        writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
    }
    writeln!(out, "{pad}        _ => Some(__prediction.alt),")
        .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }};").expect("writing to a string cannot fail");
    writeln!(out, "{pad}    match __semantic_alt {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        Some(__alt) => antlr4_runtime::ParserAtnPrediction {{ alt: __alt, ..__prediction }},"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}        None => {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}            let __error = self.base.no_viable_alternative_error(__decision_start);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}            return Err(__error);")
        .expect("writing to a string cannot fail");
    writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}} else {{").expect("writing to a string cannot fail");
    writeln!(out, "{pad}    __prediction").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}};").expect("writing to a string cannot fail");
}

pub(crate) fn render_semantic_alt_search(
    out: &mut String,
    pad: &str,
    alt_conditions: &[String],
    alts: &[Vec<GeneratedParserStep>],
    portable: Option<PortableLocalStepRender<'_>>,
) {
    // The predicted alt's predicate failed; pick another alt whose candidate
    // condition holds. This runs in TWO passes so an alt whose viability is not
    // locally checkable (no predicate and no computable lookahead — its first
    // consuming step is a rule call/decision/loop, giving condition `"true"`)
    // neither shadows a concretely-guarded alt nor becomes unreachable:
    //
    //   Pass 1 — alts with a resolved guard (predicate and/or lookahead), in
    //     order. A later token-led alt is reachable even if an earlier
    //     unresolved rule-call alt exists (`{p()}? 'a' | x | 'a'` on input `a`
    //     picks the `'a'` alt, not rule-call `x`).
    //   Pass 2 — the remaining unresolved alts, in order, as a last resort. So
    //     an unresolved alt is still tried when no resolved alt matched
    //     (`{p()}? 'a' | x` on input in FIRST(x) selects `x` instead of
    //     reporting NoViableAlt).
    //
    // Scoped to the fallback search only; the shared
    // `semantic_alt_candidate_condition` is unchanged, so left-recursion
    // loop-entry and diagnostic paths keep their behavior.
    let unresolved = alts
        .iter()
        .map(|steps| semantic_alt_guard_is_unresolved(steps, portable))
        .collect::<Vec<_>>();
    for (index, condition) in alt_conditions.iter().enumerate() {
        if unresolved.get(index).copied().unwrap_or(false) {
            continue;
        }
        let alt = index + 1;
        writeln!(out, "{pad}            if {condition} {{")
            .expect("writing to a string cannot fail");
        writeln!(out, "{pad}                Some({alt})").expect("writing to a string cannot fail");
        writeln!(out, "{pad}            }} else").expect("writing to a string cannot fail");
    }
    // Last-resort pass: unresolved alts keep their real condition (typically
    // `true`), so they are tried only after every resolved alt missed.
    for (index, condition) in alt_conditions.iter().enumerate() {
        if !unresolved.get(index).copied().unwrap_or(false) {
            continue;
        }
        let alt = index + 1;
        writeln!(out, "{pad}            if {condition} {{")
            .expect("writing to a string cannot fail");
        writeln!(out, "{pad}                Some({alt})").expect("writing to a string cannot fail");
        writeln!(out, "{pad}            }} else").expect("writing to a string cannot fail");
    }
    writeln!(out, "{pad}            {{ None }}").expect("writing to a string cannot fail");
}

/// Looks up the verbatim predicate expression (and optional `<fail=…>`
/// message) for a coordinate in embedded mode. A coordinate with no embedded
/// body evaluates true, matching ANTLR's treatment of a missing predicate.
fn embedded_predicate_condition_and_message(
    embedded: &EmbeddedStepRender<'_>,
    rule_index: usize,
    pred_index: usize,
) -> (String, Option<String>) {
    embedded
        .predicates
        .get(&(rule_index, pred_index))
        .map_or_else(
            || ("true".to_owned(), None),
            |(expression, message)| (expression.clone(), message.clone()),
        )
}

pub(crate) fn semantic_alt_candidate_condition(
    steps: &[GeneratedParserStep],
    embedded: Option<EmbeddedStepRender<'_>>,
    portable: Option<PortableLocalStepRender<'_>>,
) -> String {
    semantic_alt_candidate_condition_with_la(steps, "__semantic_la", embedded, portable)
}

fn semantic_alt_candidate_condition_with_la(
    steps: &[GeneratedParserStep],
    la_symbol: &str,
    embedded: Option<EmbeddedStepRender<'_>>,
    portable: Option<PortableLocalStepRender<'_>>,
) -> String {
    // Order matters: the lookahead guard comes FIRST so `&&` short-circuits on it
    // before any predicate hook runs. Otherwise, searching alternatives in a
    // semantic decision would evaluate an alternative's leading hook/unknown
    // predicate even when its first token cannot match the current lookahead —
    // recording a spurious fail-loud `Unsupported` hit under
    // `--sem-unknown=hook`/`error` and rejecting a later syntactically viable
    // alternative. This also matches ANTLR, which only evaluates a predicate for
    // a lookahead-viable alternative.
    let mut conditions = Vec::new();
    if let Some(lookahead) = leading_lookahead_condition(steps, la_symbol) {
        conditions.push(lookahead);
    }
    conditions.extend(leading_predicates(steps, portable).into_iter().map(
        |(rule_index, pred_index)| {
            if let Some((condition, _)) =
                portable.and_then(|portable| portable.predicates.get(&(rule_index, pred_index)))
            {
                return format!("({condition})");
            }
            embedded.map_or_else(
                || {
                    format!(
                        "self.base.parser_semantic_ir_predicate_matches_with_context_and_local(parser_semantics(), {rule_index}, {pred_index}, &__ctx, __precedence)"
                    )
                },
                |embedded| {
                    let (condition, _) = embedded_predicate_condition_and_message(
                        &embedded, rule_index, pred_index,
                    );
                    format!("({condition})")
                },
            )
        },
    ));
    if conditions.is_empty() {
        "true".to_owned()
    } else {
        conditions.join(" && ")
    }
}

/// Whether an alternative has no locally-checkable viability guard: no leading
/// predicate and no computable lookahead (its first consuming step is a
/// `CallRule` / nested decision / loop whose FIRST set is not computed here).
/// Such an alt's [`semantic_alt_candidate_condition`] is `"true"`, so in the
/// ordered semantic-alt fallback search it would shadow a later alt with a
/// concrete matching lookahead. The search treats these as last-resort
/// candidates instead. A genuine epsilon alt (no consuming step at all) is NOT
/// unguarded in this sense — it legitimately matches anything.
pub(crate) fn semantic_alt_guard_is_unresolved(
    steps: &[GeneratedParserStep],
    portable: Option<PortableLocalStepRender<'_>>,
) -> bool {
    if !leading_predicates(steps, portable).is_empty() {
        return false;
    }
    if leading_lookahead_condition(steps, "__semantic_la").is_some() {
        return false;
    }
    // No predicate and no computable lookahead: unresolved only if a consuming
    // step exists (rule call / decision / loop). Pure epsilon stays resolved.
    steps.iter().any(|step| {
        matches!(
            step,
            GeneratedParserStep::CallRule { .. }
                | GeneratedParserStep::Decision { .. }
                | GeneratedParserStep::StarLoop { .. }
                | GeneratedParserStep::LeftRecursiveLoop { .. }
        )
    })
}

fn leading_predicates(
    steps: &[GeneratedParserStep],
    _portable: Option<PortableLocalStepRender<'_>>,
) -> Vec<(usize, usize)> {
    let mut predicates = Vec::new();
    for step in steps {
        match step {
            GeneratedParserStep::Predicate {
                rule_index,
                pred_index,
            } => predicates.push((*rule_index, *pred_index)),
            // ANTLR stops collecting prediction-visible predicates at every
            // action boundary. The action runs only after the alternative is
            // committed, so a later predicate must observe its side effects.
            GeneratedParserStep::Action { .. } => break,
            GeneratedParserStep::Precedence(_) => {}
            GeneratedParserStep::MatchToken { .. }
            | GeneratedParserStep::MatchSet { .. }
            | GeneratedParserStep::MatchNotSet { .. }
            | GeneratedParserStep::MatchWildcard { .. }
            | GeneratedParserStep::CallRule { .. }
            | GeneratedParserStep::Decision { .. }
            | GeneratedParserStep::StarLoop { .. }
            | GeneratedParserStep::LeftRecursiveLoop { .. } => break,
        }
    }
    predicates
}

pub(crate) fn leading_lookahead_condition(
    steps: &[GeneratedParserStep],
    la_symbol: &str,
) -> Option<String> {
    for step in steps {
        match step {
            GeneratedParserStep::Predicate { .. }
            | GeneratedParserStep::Action { .. }
            | GeneratedParserStep::Precedence(_) => {}
            GeneratedParserStep::MatchToken { token_type, .. } => {
                return Some(format!("{la_symbol} == {token_type}"));
            }
            GeneratedParserStep::MatchSet {
                token_set,
                intervals,
                ..
            } => {
                return Some(token_set_condition(la_symbol, *token_set, intervals));
            }
            GeneratedParserStep::MatchNotSet {
                token_set,
                intervals,
                ..
            } => {
                let excluded = token_set_condition(la_symbol, *token_set, intervals);
                return Some(format!(
                    "(1..=atn().max_token_type()).contains(&{la_symbol}) && !({excluded})"
                ));
            }
            GeneratedParserStep::MatchWildcard { .. } => {
                return Some(format!("{la_symbol} != antlr4_runtime::TOKEN_EOF"));
            }
            GeneratedParserStep::CallRule { .. }
            | GeneratedParserStep::Decision { .. }
            | GeneratedParserStep::StarLoop { .. }
            | GeneratedParserStep::LeftRecursiveLoop { .. } => return None,
        }
    }
    None
}

fn token_set_condition(symbol: &str, token_set: Option<usize>, intervals: &[(i32, i32)]) -> String {
    token_set.map_or_else(
        || intervals_condition(symbol, intervals),
        |token_set| {
            format!(
                "atn().token_set({token_set}).expect(\"generated parser token-set index\").contains({symbol})"
            )
        },
    )
}

fn intervals_condition(symbol: &str, intervals: &[(i32, i32)]) -> String {
    if intervals.is_empty() {
        return "false".to_owned();
    }
    intervals
        .iter()
        .map(|(start, stop)| {
            if start == stop {
                format!("{symbol} == {start}")
            } else {
                format!("({start}..={stop}).contains(&{symbol})")
            }
        })
        .collect::<Vec<_>>()
        .join(" || ")
}

fn render_generated_alt_number_assignments(
    out: &mut String,
    pad: &str,
    alt: usize,
    track_alt_number: bool,
    track_context_alt_number: bool,
) {
    if track_alt_number {
        writeln!(out, "{pad}if __ctx.alt_number() == 0 {{")
            .expect("writing to a string cannot fail");
        writeln!(out, "{pad}    __ctx.set_alt_number({alt});")
            .expect("writing to a string cannot fail");
        writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
    }
    if track_context_alt_number {
        writeln!(out, "{pad}if __ctx.context_alt_number() == 0 {{")
            .expect("writing to a string cannot fail");
        writeln!(out, "{pad}    __ctx.set_context_alt_number({alt});")
            .expect("writing to a string cannot fail");
        writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
    }
}

fn render_generated_sync_decision(out: &mut String, pad: &str, state: usize, loop_back_expr: &str) {
    writeln!(
        out,
        "{pad}match self.base.sync_decision(atn(), {state}, !__ctx.has_matched_child(), {loop_back_expr}) {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    Ok(__sync_children) => {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        for __child in __sync_children {{ self.base.add_parse_child(&mut __ctx, __child); }}"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}    Err(__error) => {{").expect("writing to a string cannot fail");
    writeln!(out, "{pad}        __sync_error = Some(__error.clone());")
        .expect("writing to a string cannot fail");
    writeln!(out, "{pad}        return Err(__error);").expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
}

fn render_generated_adaptive_prediction(out: &mut String, pad: &str, decision: usize) {
    writeln!(out, "{pad}let __prediction = {{").expect("writing to a string cannot fail");
    render_generated_adaptive_prediction_with_indent(out, pad, decision, 1);
    writeln!(out, "{pad}}};").expect("writing to a string cannot fail");
}

fn render_generated_adaptive_prediction_with_indent(
    out: &mut String,
    pad: &str,
    decision: usize,
    extra_indent: usize,
) {
    let nested = format!("{pad}{}", "    ".repeat(extra_indent));
    writeln!(
        out,
        "{nested}let __simulator = self.simulator.get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new_shared(atn()));"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{nested}let __prediction_context = __simulator.intern_prediction_context(self.base.rule_context_version(), self.base.prediction_context_return_states(atn()));"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{nested}__simulator.set_exact_ambig_detection(self.base.prediction_mode() == antlr4_runtime::PredictionMode::LlExactAmbigDetection);"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{nested}__simulator.adaptive_predict_stream_info_with_context({decision}, 0, self.base.input(), __prediction_context)"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{nested}    .map_err(|__error| match __error {{")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{nested}        antlr4_runtime::ParserAtnSimulatorError::NoViableAlt {{ index, .. }} => self.base.no_viable_alternative_error_at(__decision_start, index),"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{nested}        _ => self.base.no_viable_alternative_error(__decision_start),"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{nested}    }})?").expect("writing to a string cannot fail");
}

fn render_generated_sll_then_context_prediction_with_indent(
    out: &mut String,
    pad: &str,
    decision: usize,
    extra_indent: usize,
) {
    let nested = format!("{pad}{}", "    ".repeat(extra_indent));
    writeln!(out, "{nested}let __prediction = {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{nested}    let __simulator = self.simulator.get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new_shared(atn()));"
    )
    .expect("writing to a string cannot fail");
    // Stage 1 uses the SLL probe: on a full-context-requiring conflict it returns
    // requires_full_context WITHOUT running the LL loop (the result is discarded
    // here anyway — only the boolean gates the stage-2 re-run with real context).
    writeln!(
        out,
        "{nested}    __simulator.adaptive_predict_stream_info_sll_probe({decision}, 0, self.base.input())"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{nested}        .map_err(|__error| match __error {{")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{nested}            antlr4_runtime::ParserAtnSimulatorError::NoViableAlt {{ index, .. }} => self.base.no_viable_alternative_error_at(__decision_start, index),"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{nested}            _ => self.base.no_viable_alternative_error(__decision_start),"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{nested}        }})?").expect("writing to a string cannot fail");
    writeln!(out, "{nested}}};").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{nested}if __prediction.requires_full_context && self.base.prediction_mode() != antlr4_runtime::PredictionMode::Sll {{"
    )
    .expect("writing to a string cannot fail");
    render_generated_adaptive_prediction_with_indent(out, pad, decision, extra_indent + 1);
    writeln!(out, "{nested}}} else {{").expect("writing to a string cannot fail");
    writeln!(out, "{nested}    __prediction").expect("writing to a string cannot fail");
    writeln!(out, "{nested}}}").expect("writing to a string cannot fail");
}
