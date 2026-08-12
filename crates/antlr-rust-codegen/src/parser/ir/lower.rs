// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
pub(crate) fn compile_generated_parser_rule(
    context: &GeneratedParserCompileContext<'_>,
    rule_index: usize,
) -> Option<GeneratedParserRule> {
    let entry_state = context.atn.rule_to_start_state().get(rule_index)?;
    let stop_state = context.atn.rule_to_stop_state().get(rule_index)?;
    let start = context.atn.state(entry_state)?;
    if start.left_recursive_rule() {
        return compile_generated_left_recursive_parser_rule(
            context,
            rule_index,
            entry_state,
            stop_state,
        );
    }
    let mut visited = BTreeSet::new();
    let steps = compile_generated_parser_path(context, entry_state, stop_state, &mut visited)?;
    Some(GeneratedParserRule {
        rule_index,
        entry_state,
        left_recursive: false,
        steps,
    })
}

fn compile_generated_left_recursive_parser_rule(
    context: &GeneratedParserCompileContext<'_>,
    rule_index: usize,
    entry_state: usize,
    stop_state: usize,
) -> Option<GeneratedParserRule> {
    let loop_entry = find_left_recursive_loop_entry(context, rule_index)?;
    let mut visited = BTreeSet::new();
    let mut steps = compile_generated_parser_path(context, entry_state, loop_entry, &mut visited)?;
    let loop_state = context.atn.state(loop_entry)?;
    let decision = context
        .decision_by_state
        .get(loop_entry)
        .copied()
        .flatten()?;
    let (loop_step, exit_target) = compile_generated_left_recursive_loop(
        context,
        rule_index,
        entry_state,
        loop_state,
        decision,
    )?;
    steps.push(loop_step);
    steps.extend(compile_generated_parser_path(
        context,
        exit_target,
        stop_state,
        &mut BTreeSet::new(),
    )?);
    Some(GeneratedParserRule {
        rule_index,
        entry_state,
        left_recursive: true,
        steps,
    })
}

fn find_left_recursive_loop_entry(
    context: &GeneratedParserCompileContext<'_>,
    rule_index: usize,
) -> Option<usize> {
    context.atn.states().find_map(|state| {
        (state.rule_index() == Some(rule_index)
            && state.kind() == AtnStateKind::StarLoopEntry
            && state.precedence_rule_decision())
        .then_some(state.state_number())
    })
}

fn compile_generated_left_recursive_loop(
    context: &GeneratedParserCompileContext<'_>,
    rule_index: usize,
    entry_state: usize,
    state: ParserAtnState<'_>,
    decision: usize,
) -> Option<(GeneratedParserStep, usize)> {
    let mut enter = None;
    let mut exit = None;
    for (index, transition) in state.transitions().iter().enumerate() {
        let alt = index + 1;
        let target = transition.target();
        let target_state = context.atn.state(target)?;
        if target_state.kind() == AtnStateKind::LoopEnd {
            exit = Some((alt, transition, target, target_state.loop_back_state()?));
        } else {
            enter = Some((alt, transition));
        }
    }

    let (enter_alt, enter_transition) = enter?;
    let (exit_alt, exit_transition, exit_target, loop_back_state) = exit?;
    let (enter_step, enter_target) = compile_generated_parser_transition(
        state.state_number(),
        context.rule_args,
        enter_transition,
        generated_action_state_sets(context),
        generated_predicate_coordinate_sets(context),
    )?;
    let mut body = enter_step.into_iter().collect::<Vec<_>>();
    body.extend(compile_generated_parser_path(
        context,
        enter_target,
        loop_back_state,
        &mut BTreeSet::new(),
    )?);
    allow_semantic_context_in_decisions(&mut body);
    if !steps_may_consume(&body) {
        return None;
    }

    let (exit_step, _) = compile_generated_parser_transition(
        state.state_number(),
        context.rule_args,
        exit_transition,
        generated_action_state_sets(context),
        generated_predicate_coordinate_sets(context),
    )?;
    if exit_step.is_some() {
        return None;
    }

    Some((
        GeneratedParserStep::LeftRecursiveLoop {
            state: state.state_number(),
            decision,
            enter_alt,
            exit_alt,
            rule_index,
            entry_state,
            body,
        },
        exit_target,
    ))
}

fn compile_generated_parser_path(
    context: &GeneratedParserCompileContext<'_>,
    state_number: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    if state_number == stop_state {
        return Some(Vec::new());
    }
    if !visited.insert(state_number) {
        return None;
    }

    let state = context.atn.state(state_number)?;
    let steps = if let Some(decision) = context
        .decision_by_state
        .get(state_number)
        .copied()
        .flatten()
    {
        compile_generated_parser_decision_state(context, state, decision, stop_state, visited)?
    } else {
        let transition = state.transitions().first()?;
        if state.transitions().len() != 1 {
            return None;
        }
        let (step, target) = compile_generated_parser_transition(
            state_number,
            context.rule_args,
            transition,
            generated_action_state_sets(context),
            generated_predicate_coordinate_sets(context),
        )?;
        let mut steps = step.into_iter().collect::<Vec<_>>();
        steps.extend(compile_generated_parser_path(
            context, target, stop_state, visited,
        )?);
        steps
    };
    visited.remove(&state_number);
    Some(steps)
}

fn compile_generated_parser_decision_state(
    context: &GeneratedParserCompileContext<'_>,
    state: ParserAtnState<'_>,
    decision: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    match state.kind() {
        AtnStateKind::BlockStart | AtnStateKind::PlusBlockStart | AtnStateKind::StarBlockStart => {
            compile_generated_parser_block_decision(context, state, decision, stop_state, visited)
        }
        AtnStateKind::StarLoopEntry => {
            compile_generated_parser_star_loop(context, state, decision, stop_state, visited)
        }
        AtnStateKind::PlusLoopBack => {
            compile_generated_parser_plus_loop(context, state, decision, stop_state, visited)
        }
        _ => None,
    }
}

fn compile_generated_parser_block_decision(
    context: &GeneratedParserCompileContext<'_>,
    state: ParserAtnState<'_>,
    decision: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    let end_state = state.end_state()?;
    let mut alts = Vec::with_capacity(state.transitions().len());
    for transition in state.transitions() {
        let (step, target) = compile_generated_parser_transition(
            state.state_number(),
            context.rule_args,
            transition,
            generated_action_state_sets(context),
            generated_predicate_coordinate_sets(context),
        )?;
        let mut alt_visited = visited.clone();
        let mut alt_steps = step.into_iter().collect::<Vec<_>>();
        alt_steps.extend(compile_generated_parser_path(
            context,
            target,
            end_state,
            &mut alt_visited,
        )?);
        alts.push(alt_steps);
    }

    let mut steps = vec![GeneratedParserStep::Decision {
        state: state.state_number(),
        decision,
        track_alt_number: state_tracks_alt_number(state),
        allow_semantic_context: alts.iter().any(|alt| steps_contain_predicate(alt)),
        force_context: state.non_greedy(),
        fast_path: generated_decision_fast_path(
            context,
            state,
            alts.iter()
                .enumerate()
                .map(|(index, alt)| (index + 1, alt.as_slice())),
        ),
        alts,
    }];
    steps.extend(compile_generated_parser_path(
        context, end_state, stop_state, visited,
    )?);
    Some(steps)
}

fn compile_generated_parser_star_loop(
    context: &GeneratedParserCompileContext<'_>,
    state: ParserAtnState<'_>,
    decision: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    let mut enter = None;
    let mut exit = None;
    for (index, transition) in state.transitions().iter().enumerate() {
        let alt = index + 1;
        let target = transition.target();
        let target_state = context.atn.state(target)?;
        let target_kind = target_state.kind();
        if target_kind == AtnStateKind::LoopEnd {
            exit = Some((alt, transition, target_state.loop_back_state()?));
        } else {
            enter = Some((alt, transition));
        }
    }

    let (enter_alt, enter_transition) = enter?;
    let (exit_alt, exit_transition, loop_back_state) = exit?;
    let (enter_step, enter_target) = compile_generated_parser_transition(
        state.state_number(),
        context.rule_args,
        enter_transition,
        generated_action_state_sets(context),
        generated_predicate_coordinate_sets(context),
    )?;
    let mut body_visited = BTreeSet::new();
    let mut body = enter_step.into_iter().collect::<Vec<_>>();
    body.extend(compile_generated_parser_path(
        context,
        enter_target,
        loop_back_state,
        &mut body_visited,
    )?);
    if !steps_may_consume(&body) {
        return None;
    }

    let (exit_step, exit_target) = compile_generated_parser_transition(
        state.state_number(),
        context.rule_args,
        exit_transition,
        generated_action_state_sets(context),
        generated_predicate_coordinate_sets(context),
    )?;
    if exit_step.is_some() {
        return None;
    }

    let mut steps = vec![GeneratedParserStep::StarLoop {
        state: state.state_number(),
        decision,
        enter_alt,
        exit_alt,
        track_alt_number: state_tracks_alt_number(state),
        allow_semantic_context: steps_contain_predicate(&body),
        force_context: state.non_greedy(),
        plus_loop: false,
        fast_path: None,
        body,
    }];
    steps.extend(compile_generated_parser_path(
        context,
        exit_target,
        stop_state,
        visited,
    )?);
    Some(steps)
}

fn compile_generated_parser_plus_loop(
    context: &GeneratedParserCompileContext<'_>,
    state: ParserAtnState<'_>,
    decision: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    let mut enter = None;
    let mut exit = None;
    for (index, transition) in state.transitions().iter().enumerate() {
        let alt = index + 1;
        let target = transition.target();
        let target_state = context.atn.state(target)?;
        if target_state.kind() == AtnStateKind::LoopEnd {
            exit = Some((alt, transition));
        } else {
            enter = Some((alt, transition));
        }
    }

    let (enter_alt, enter_transition) = enter?;
    let (enter_step, enter_target) = compile_generated_parser_transition(
        state.state_number(),
        context.rule_args,
        enter_transition,
        generated_action_state_sets(context),
        generated_predicate_coordinate_sets(context),
    )?;
    let mut body_visited = BTreeSet::new();
    let mut body = enter_step.into_iter().collect::<Vec<_>>();
    body.extend(compile_generated_parser_path(
        context,
        enter_target,
        state.state_number(),
        &mut body_visited,
    )?);
    if !steps_may_consume(&body) {
        return None;
    }

    let (exit_alt, exit_transition) = exit?;
    let (exit_step, exit_target) = compile_generated_parser_transition(
        state.state_number(),
        context.rule_args,
        exit_transition,
        generated_action_state_sets(context),
        generated_predicate_coordinate_sets(context),
    )?;
    if exit_step.is_some() {
        return None;
    }

    let mut steps = vec![GeneratedParserStep::StarLoop {
        state: state.state_number(),
        decision,
        enter_alt,
        exit_alt,
        track_alt_number: state_tracks_alt_number(state),
        allow_semantic_context: steps_contain_predicate(&body),
        force_context: state.non_greedy(),
        plus_loop: true,
        fast_path: None,
        body,
    }];
    steps.extend(compile_generated_parser_path(
        context,
        exit_target,
        stop_state,
        visited,
    )?);
    Some(steps)
}

fn steps_may_consume(steps: &[GeneratedParserStep]) -> bool {
    steps.iter().any(|step| match step {
        GeneratedParserStep::MatchToken { .. }
        | GeneratedParserStep::MatchSet { .. }
        | GeneratedParserStep::MatchNotSet { .. }
        | GeneratedParserStep::MatchWildcard { .. }
        | GeneratedParserStep::CallRule { .. } => true,
        GeneratedParserStep::Action { .. }
        | GeneratedParserStep::Precedence(_)
        | GeneratedParserStep::Predicate { .. } => false,
        GeneratedParserStep::Decision { alts, .. } => alts.iter().any(|alt| steps_may_consume(alt)),
        GeneratedParserStep::StarLoop { body, .. }
        | GeneratedParserStep::LeftRecursiveLoop { body, .. } => steps_may_consume(body),
    })
}

fn allow_semantic_context_in_decisions(steps: &mut [GeneratedParserStep]) {
    for step in steps {
        match step {
            GeneratedParserStep::Decision {
                allow_semantic_context,
                fast_path,
                alts,
                ..
            } => {
                *allow_semantic_context = true;
                *fast_path = None;
                for alt in alts {
                    allow_semantic_context_in_decisions(alt);
                }
            }
            GeneratedParserStep::StarLoop {
                allow_semantic_context,
                fast_path,
                body,
                ..
            } => {
                *allow_semantic_context = true;
                *fast_path = None;
                allow_semantic_context_in_decisions(body);
            }
            GeneratedParserStep::LeftRecursiveLoop { body, .. } => {
                allow_semantic_context_in_decisions(body);
            }
            GeneratedParserStep::MatchToken { .. }
            | GeneratedParserStep::MatchSet { .. }
            | GeneratedParserStep::MatchNotSet { .. }
            | GeneratedParserStep::MatchWildcard { .. }
            | GeneratedParserStep::Precedence(_)
            | GeneratedParserStep::Predicate { .. }
            | GeneratedParserStep::Action { .. }
            | GeneratedParserStep::CallRule { .. } => {}
        }
    }
}

/// Applies generated-rule rendering constraints to classifier report rows.
///
/// A LOOK(1)-disjoint decision nested in a left-recursive operator body is
/// still classified `ll1`, but [`allow_semantic_context_in_decisions`] forces
/// its emitted path through full-context adaptive prediction. Keep the tier as
/// the tool verdict while reporting that the rendered path can defer.
pub(crate) fn rendered_decision_report_rows(
    rows: &[DecisionReportRow],
    rules: &[Option<GeneratedParserRule>],
) -> Vec<DecisionReportRow> {
    let mut forced_adaptive = BTreeSet::new();
    for rule in rules.iter().flatten() {
        collect_render_forced_adaptive_decisions(&rule.steps, &mut forced_adaptive);
    }
    rows.iter()
        .cloned()
        .map(|mut row| {
            if forced_adaptive.contains(&row.decision) {
                row.fallback = DecisionFallbackCapability::CanDefer;
            }
            row
        })
        .collect()
}

fn collect_render_forced_adaptive_decisions(
    steps: &[GeneratedParserStep],
    decisions: &mut BTreeSet<usize>,
) {
    for step in steps {
        match step {
            GeneratedParserStep::Decision {
                decision,
                allow_semantic_context,
                force_context,
                alts,
                ..
            } => {
                if *allow_semantic_context || *force_context {
                    decisions.insert(*decision);
                }
                for alt in alts {
                    collect_render_forced_adaptive_decisions(alt, decisions);
                }
            }
            GeneratedParserStep::StarLoop {
                decision,
                allow_semantic_context,
                force_context,
                body,
                ..
            } => {
                if *allow_semantic_context || *force_context {
                    decisions.insert(*decision);
                }
                collect_render_forced_adaptive_decisions(body, decisions);
            }
            GeneratedParserStep::LeftRecursiveLoop { decision, body, .. } => {
                decisions.insert(*decision);
                collect_render_forced_adaptive_decisions(body, decisions);
            }
            GeneratedParserStep::MatchToken { .. }
            | GeneratedParserStep::MatchSet { .. }
            | GeneratedParserStep::MatchNotSet { .. }
            | GeneratedParserStep::MatchWildcard { .. }
            | GeneratedParserStep::Precedence(_)
            | GeneratedParserStep::Predicate { .. }
            | GeneratedParserStep::Action { .. }
            | GeneratedParserStep::CallRule { .. } => {}
        }
    }
}

pub(crate) fn steps_contain_predicate(steps: &[GeneratedParserStep]) -> bool {
    steps.iter().any(|step| match step {
        GeneratedParserStep::Predicate { .. } => true,
        GeneratedParserStep::Decision { alts, .. } => {
            alts.iter().any(|alt| steps_contain_predicate(alt))
        }
        GeneratedParserStep::StarLoop { body, .. }
        | GeneratedParserStep::LeftRecursiveLoop { body, .. } => steps_contain_predicate(body),
        GeneratedParserStep::MatchToken { .. }
        | GeneratedParserStep::MatchSet { .. }
        | GeneratedParserStep::MatchNotSet { .. }
        | GeneratedParserStep::MatchWildcard { .. }
        | GeneratedParserStep::Precedence(_)
        | GeneratedParserStep::Action { .. }
        | GeneratedParserStep::CallRule { .. } => false,
    })
}

fn generated_rule_call_precedence(
    rule_args: &[(usize, usize, RuleArgTemplate)],
    source_state: usize,
    rule_index: usize,
    transition_precedence: i32,
) -> Option<GeneratedRuleCallPrecedence> {
    let Some((_, _, arg)) = rule_args
        .iter()
        .find(|(arg_source, arg_rule, _)| *arg_source == source_state && *arg_rule == rule_index)
    else {
        return Some(GeneratedRuleCallPrecedence::Literal(transition_precedence));
    };
    match arg {
        RuleArgTemplate::Literal(value) => i32::try_from(*value)
            .ok()
            .map(GeneratedRuleCallPrecedence::Literal),
        RuleArgTemplate::InheritLocal => Some(GeneratedRuleCallPrecedence::InheritLocal),
    }
}

pub(crate) fn compile_generated_parser_transition(
    source_state: usize,
    rule_args: &[(usize, usize, RuleArgTemplate)],
    transition: ParserTransition<'_>,
    action_states: ActionStateSets<'_>,
    predicate_coordinates: PredicateCoordinateSets<'_>,
) -> Option<(Option<GeneratedParserStep>, usize)> {
    match transition.data() {
        ParserTransitionData::Epsilon { target } => Some((None, target)),
        ParserTransitionData::Atom { target, label } => Some((
            Some(GeneratedParserStep::MatchToken {
                token_type: label,
                follow_state: target,
            }),
            target,
        )),
        ParserTransitionData::Range {
            target,
            start,
            stop,
        } => Some((
            Some(GeneratedParserStep::MatchSet {
                token_set: None,
                intervals: vec![(start, stop)],
                follow_state: target,
            }),
            target,
        )),
        ParserTransitionData::Set { target, set } => Some((
            Some(GeneratedParserStep::MatchSet {
                token_set: (set.kind() == ParserTokenSetKind::Dense).then_some(set.index()),
                intervals: set.ranges().collect(),
                follow_state: target,
            }),
            target,
        )),
        ParserTransitionData::NotSet { target, set } => Some((
            Some(GeneratedParserStep::MatchNotSet {
                token_set: (set.kind() == ParserTokenSetKind::Dense).then_some(set.index()),
                intervals: set.ranges().collect(),
                follow_state: target,
            }),
            target,
        )),
        ParserTransitionData::Wildcard { target } => Some((
            Some(GeneratedParserStep::MatchWildcard {
                follow_state: target,
            }),
            target,
        )),
        ParserTransitionData::Rule {
            rule_index,
            follow_state,
            precedence,
            ..
        } => Some((
            Some(GeneratedParserStep::CallRule {
                source_state,
                rule_index,
                precedence: generated_rule_call_precedence(
                    rule_args,
                    source_state,
                    rule_index,
                    precedence,
                )?,
            }),
            follow_state,
        )),
        ParserTransitionData::Action {
            target,
            rule_index,
            action_index,
            ..
        } if action_states.generated.contains(&source_state) => Some((
            Some(GeneratedParserStep::Action {
                source_state,
                rule_index,
                action_index: action_states
                    .indices
                    .get(&source_state)
                    .copied()
                    .or(action_index),
            }),
            target,
        )),
        ParserTransitionData::Action {
            target,
            action_index: None,
            ..
        } if !action_states.all.contains(&source_state) => Some((None, target)),
        ParserTransitionData::Predicate {
            target,
            rule_index,
            pred_index,
            ..
        } if predicate_coordinates
            .generated
            .contains(&(rule_index, pred_index)) =>
        {
            Some((
                Some(GeneratedParserStep::Predicate {
                    rule_index,
                    pred_index,
                }),
                target,
            ))
        }
        ParserTransitionData::Predicate {
            rule_index,
            pred_index,
            ..
        } if predicate_coordinates
            .all
            .contains(&(rule_index, pred_index)) =>
        {
            None
        }
        ParserTransitionData::Predicate { target, .. } => Some((None, target)),
        ParserTransitionData::Precedence { target, precedence } => {
            Some((Some(GeneratedParserStep::Precedence(precedence)), target))
        }
        ParserTransitionData::Action { .. } => None,
    }
}
