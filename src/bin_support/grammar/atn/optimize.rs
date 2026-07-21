use std::collections::BTreeSet;

use antlr4_runtime::atn::AtnStateKind;

use super::super::model::{BuildStateId, BuildTransitionId};
use super::super::provenance::ProvenanceIndex;
use super::build::{BuildGraph, BuildTransitionKind};

pub(crate) fn collapse_lexer_sets(graph: &mut BuildGraph, provenance: &mut ProvenanceIndex) {
    for decision in graph.decisions.clone() {
        collapse_decision_sets(graph, decision, provenance);
    }
}

fn collapse_decision_sets(
    graph: &mut BuildGraph,
    decision: BuildStateId,
    provenance: &mut ProvenanceIndex,
) {
    let branches = graph.state(decision).transitions.clone();
    let mut run = Vec::new();
    for branch in branches {
        if let Some(alternative) = collapsible_set_alternative(graph, branch) {
            run.push(alternative);
        } else {
            collapse_set_run(graph, &run, provenance);
            run.clear();
        }
    }
    collapse_set_run(graph, &run, provenance);
}

#[derive(Clone, Debug)]
struct SetAlternative {
    branch_state: BuildStateId,
    match_transition: BuildTransitionId,
    block_end: BuildStateId,
    ranges: Vec<(i32, i32)>,
}

fn collapsible_set_alternative(
    graph: &BuildGraph,
    branch: BuildTransitionId,
) -> Option<SetAlternative> {
    let branch = graph.transition(branch);
    if branch.kind != BuildTransitionKind::Epsilon || !graph.state(branch.target).active {
        return None;
    }
    let branch_state = graph.state(branch.target);
    let [match_transition] = branch_state.transitions.as_slice() else {
        return None;
    };
    let transition = graph.transition(*match_transition);
    if graph.state(transition.target).kind != AtnStateKind::BlockEnd {
        return None;
    }
    let ranges = match &transition.kind {
        BuildTransitionKind::Atom(value) => vec![(*value, *value)],
        BuildTransitionKind::Range { start, stop } => vec![(*start, *stop)],
        BuildTransitionKind::Set(ranges) => ranges.clone(),
        _ => return None,
    };
    Some(SetAlternative {
        branch_state: branch.target,
        match_transition: *match_transition,
        block_end: transition.target,
        ranges,
    })
}

fn collapse_set_run(
    graph: &mut BuildGraph,
    alternatives: &[SetAlternative],
    provenance: &mut ProvenanceIndex,
) {
    if alternatives.len() < 2 {
        return;
    }
    let block_end = alternatives[0].block_end;
    if alternatives
        .iter()
        .any(|alternative| alternative.block_end != block_end)
    {
        return;
    }

    let first = &alternatives[0];
    let ranges = normalize_ranges(
        alternatives
            .iter()
            .flat_map(|alternative| alternative.ranges.iter().copied()),
    );
    let origins = alternatives
        .iter()
        .flat_map(|alternative| {
            provenance
                .transition_origins(alternative.match_transition)
                .iter()
                .cloned()
        })
        .collect::<Vec<_>>();
    let transition = graph.transition_mut(first.match_transition);
    transition.target = block_end;
    transition.kind = collapsed_set_kind(ranges);
    provenance.record_transition(first.match_transition, origins);

    for alternative in &alternatives[1..] {
        graph.remove_state(alternative.branch_state);
    }
}

fn collapsed_set_kind(ranges: Vec<(i32, i32)>) -> BuildTransitionKind {
    match ranges.as_slice() {
        [(start, stop)] if start == stop => BuildTransitionKind::Atom(*start),
        [(start, stop)] => BuildTransitionKind::Range {
            start: *start,
            stop: *stop,
        },
        _ => BuildTransitionKind::Set(ranges),
    }
}

fn normalize_ranges(ranges: impl IntoIterator<Item = (i32, i32)>) -> Vec<(i32, i32)> {
    let mut ranges = ranges.into_iter().collect::<Vec<_>>();
    ranges.sort_unstable();
    let mut normalized: Vec<(i32, i32)> = Vec::with_capacity(ranges.len());
    for (start, stop) in ranges {
        if let Some((_, previous_stop)) = normalized.last_mut()
            && start <= previous_stop.saturating_add(1)
        {
            *previous_stop = (*previous_stop).max(stop);
        } else {
            normalized.push((start, stop));
        }
    }
    normalized
}

pub(crate) fn remove_tail_epsilons(graph: &mut BuildGraph, start: BuildStateId) {
    visit(graph, start, &mut BTreeSet::new());
}

fn visit(graph: &mut BuildGraph, state: BuildStateId, visited: &mut BTreeSet<BuildStateId>) {
    if !visited.insert(state) || !graph.state(state).active {
        return;
    }
    remove_tail_epsilon(graph, state);
    let targets = graph
        .state(state)
        .transitions
        .iter()
        .map(|transition| graph.transition(*transition).target)
        .collect::<Vec<_>>();
    for target in targets {
        visit(graph, target, visited);
    }
}

fn remove_tail_epsilon(graph: &mut BuildGraph, state: BuildStateId) {
    let source = graph.state(state);
    if source.kind != AtnStateKind::Basic || source.transitions.len() != 1 {
        return;
    }
    let transition_id = source.transitions[0];
    let transition = graph.transition(transition_id);
    let candidate = match &transition.kind {
        BuildTransitionKind::Rule { follow, .. } => *follow,
        _ => transition.target,
    };
    let candidate_state = graph.state(candidate);
    if candidate_state.kind != AtnStateKind::Basic || candidate_state.transitions.len() != 1 {
        return;
    }
    let epsilon = graph.transition(candidate_state.transitions[0]);
    if epsilon.kind != BuildTransitionKind::Epsilon {
        return;
    }
    if !matches!(
        graph.state(epsilon.target).kind,
        AtnStateKind::BlockEnd | AtnStateKind::PlusLoopBack | AtnStateKind::StarLoopBack
    ) {
        return;
    }
    let target = epsilon.target;
    match &mut graph.transition_mut(transition_id).kind {
        BuildTransitionKind::Rule { follow, .. } => *follow = target,
        _ => graph.transition_mut(transition_id).target = target,
    }
    graph.remove_state(candidate);
}
