const MAX_SHARED_DESCENT_PREFIX: usize = 2;
const MAX_SHARED_DESCENT_PREFIX_FANOUT: usize = 16;
const SHARED_DESCENT_WALK_BUDGET: usize = 200_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SharedDescentPlan {
    pub(crate) decision: usize,
    pub(crate) common_rule: usize,
    pub(crate) prefix_tokens: Vec<i32>,
    pub(crate) conditional: bool,
    pub(crate) trigger_intervals: Vec<(i32, i32)>,
    pub(crate) common_token_length: Option<usize>,
    pub(crate) call_site: usize,
    pub(crate) resume_call_sites: Vec<usize>,
    pub(crate) alternatives: Vec<usize>,
    pub(crate) tails: Vec<SharedDescentTail>,
    pub(crate) default_alt: Option<usize>,
    pub(crate) default_excluded_intervals: Vec<(i32, i32)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SharedDescentTail {
    pub(crate) alt: usize,
    pub(crate) intervals: Vec<(i32, i32)>,
    pub(crate) guard_against_follow: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SharedDescentGroupReport {
    pub(crate) common_rule: usize,
    pub(crate) prefix_tokens: Vec<i32>,
    pub(crate) alternatives: Vec<usize>,
    pub(crate) outcome: SharedDescentGroupOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SharedDescentGroupOutcome {
    Selected,
    Declined(SharedDescentDeclineReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SharedDescentDeclineReason {
    ConditionalHeadCoversFirst,
    ContextSensitiveRule,
    DominatedGroup,
    NoSafeTrigger,
    NoTailDispatch,
    SemanticDescent,
}

impl SharedDescentDeclineReason {
    pub(crate) const fn manifest_name(self) -> &'static str {
        match self {
            Self::ConditionalHeadCoversFirst => "conditional-head-covers-first",
            Self::ContextSensitiveRule => "context-sensitive-rule",
            Self::DominatedGroup => "dominated-group",
            Self::NoSafeTrigger => "no-safe-trigger",
            Self::NoTailDispatch => "no-tail-dispatch",
            Self::SemanticDescent => "semantic-descent",
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct SharedDescentAnalysis {
    pub(crate) plans: BTreeMap<usize, Vec<SharedDescentPlan>>,
    pub(crate) reports: BTreeMap<usize, Vec<SharedDescentGroupReport>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SharedDescentGroupKey {
    common_rule: usize,
    prefix_tokens: Vec<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DescentCall {
    source_state: usize,
    rule_index: usize,
    follow_state: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AltPath {
    alt: usize,
    common_rule: usize,
    prefix_tokens: Vec<i32>,
    chain: Vec<DescentCall>,
    head_guard: BTreeSet<i32>,
    semantic_descent: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GeneratedDecisionShape {
    state: usize,
    owning_rule: usize,
    allow_semantic_context: bool,
    force_context: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DescentVerdict {
    Always,
    Sometimes,
    NoCleanPath,
    Semantic,
    Veto,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TailLook {
    symbols: BTreeSet<i32>,
    nullable: bool,
}

struct TailDispatch {
    tails: Vec<SharedDescentTail>,
    default_alt: Option<usize>,
    default_excluded_intervals: Vec<(i32, i32)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExactTokenLength {
    Unknown,
    Inexact,
    Exact(usize),
}

pub(crate) fn analyze_shared_descent(
    atn: &ParserAtn,
    rules: &[Option<GeneratedParserRule>],
    decision_rows: &[DecisionReportRow],
    semantic_rules: &BTreeSet<usize>,
) -> SharedDescentAnalysis {
    let shapes = generated_decision_shapes(rules);
    let context_free_decisions = decision_rows
        .iter()
        .filter_map(|row| {
            matches!(
                row.tier,
                DecisionTierReport::Ll1 | DecisionTierReport::Fixed { .. }
            )
            .then_some(row.decision)
        })
        .collect::<BTreeSet<_>>();
    let mut neutral_safety = vec![None; rules.len()];
    let mut exact_lengths = vec![ExactTokenLength::Unknown; rules.len()];
    let mut analysis = SharedDescentAnalysis::default();

    for (decision, shape) in shapes {
        let Some(state) = atn.state(shape.state) else {
            continue;
        };
        if !matches!(
            decision_rows.get(decision).map(|row| row.tier),
            Some(DecisionTierReport::Adaptive { .. })
        ) {
            continue;
        }
        let groups = candidate_groups(atn, state);
        for (key, mut paths) in groups {
            let unsupported_chain = paths.iter().any(|path| {
                path.chain.iter().any(|call| {
                    rules
                        .get(call.rule_index)
                        .and_then(Option::as_ref)
                        .is_none_or(|rule| rule.left_recursive)
                })
            });
            if shape.allow_semantic_context {
                for path in &mut paths {
                    path.semantic_descent = true;
                }
            }
            for path in &mut paths {
                path.semantic_descent |= path
                    .chain
                    .iter()
                    .any(|call| semantic_rules.contains(&call.rule_index));
            }
            let alternatives = paths
                .iter()
                .map(|path| path.alt)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let mut report = SharedDescentGroupReport {
                common_rule: key.common_rule,
                prefix_tokens: key.prefix_tokens.clone(),
                alternatives: alternatives.clone(),
                outcome: SharedDescentGroupOutcome::Selected,
            };
            if shape.force_context || unsupported_chain {
                report.outcome = SharedDescentGroupOutcome::Declined(
                    SharedDescentDeclineReason::ContextSensitiveRule,
                );
                analysis.reports.entry(decision).or_default().push(report);
                continue;
            }
            if paths.iter().all(|path| path.semantic_descent) {
                report.outcome = SharedDescentGroupOutcome::Declined(
                    SharedDescentDeclineReason::SemanticDescent,
                );
                analysis.reports.entry(decision).or_default().push(report);
                continue;
            }
            paths.retain(|path| !path.semantic_descent);
            if !neutral_rule_is_context_free(
                key.common_rule,
                rules,
                semantic_rules,
                &context_free_decisions,
                &mut neutral_safety,
                &mut BTreeSet::new(),
            ) {
                report.outcome = SharedDescentGroupOutcome::Declined(
                    SharedDescentDeclineReason::ContextSensitiveRule,
                );
                analysis.reports.entry(decision).or_default().push(report);
                continue;
            }
            let first = rule_first_symbols(atn, key.common_rule);
            let head_guard = paths
                .iter()
                .flat_map(|path| path.head_guard.iter().copied())
                .collect::<BTreeSet<_>>();
            let mut trigger = first
                .difference(&head_guard)
                .copied()
                .collect::<BTreeSet<_>>();
            if trigger.is_empty() {
                report.outcome = SharedDescentGroupOutcome::Declined(
                    SharedDescentDeclineReason::ConditionalHeadCoversFirst,
                );
                analysis.reports.entry(decision).or_default().push(report);
                continue;
            }
            restrict_trigger_against_nonmembers(
                atn,
                state,
                shape.owning_rule,
                &key.prefix_tokens,
                &alternatives,
                &mut trigger,
            );
            if trigger.is_empty() {
                report.outcome =
                    SharedDescentGroupOutcome::Declined(SharedDescentDeclineReason::NoSafeTrigger);
                analysis.reports.entry(decision).or_default().push(report);
                continue;
            }
            let Some(TailDispatch {
                tails,
                default_alt,
                default_excluded_intervals,
            }) = tail_dispatch(atn, shape.owning_rule, &alternatives, &paths)
            else {
                report.outcome =
                    SharedDescentGroupOutcome::Declined(SharedDescentDeclineReason::NoTailDispatch);
                analysis.reports.entry(decision).or_default().push(report);
                continue;
            };
            let resume_call_sites = paths
                .iter()
                .filter_map(|path| path.chain.last().map(|call| call.source_state))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let call_site = resume_call_sites
                .first()
                .copied()
                .unwrap_or_else(|| state.state_number());
            let plan = SharedDescentPlan {
                decision,
                common_rule: key.common_rule,
                prefix_tokens: key.prefix_tokens,
                conditional: !head_guard.is_empty(),
                trigger_intervals: symbols_to_ranges(trigger),
                common_token_length: exact_rule_token_length(
                    key.common_rule,
                    rules,
                    &mut exact_lengths,
                    &mut BTreeSet::new(),
                ),
                call_site,
                resume_call_sites,
                alternatives,
                tails,
                default_alt,
                default_excluded_intervals,
            };
            let plans = analysis.plans.entry(decision).or_default();
            if plans
                .iter()
                .any(|existing| plan_is_dominated(&plan, existing))
            {
                report.outcome =
                    SharedDescentGroupOutcome::Declined(SharedDescentDeclineReason::DominatedGroup);
                analysis.reports.entry(decision).or_default().push(report);
                continue;
            }
            plans.push(plan);
            analysis.reports.entry(decision).or_default().push(report);
        }
    }

    for plans in analysis.plans.values_mut() {
        plans.sort_by(|left, right| {
            left.conditional
                .cmp(&right.conditional)
                .then_with(|| left.prefix_tokens.len().cmp(&right.prefix_tokens.len()))
                .then_with(|| left.common_rule.cmp(&right.common_rule))
                .then_with(|| left.alternatives.cmp(&right.alternatives))
        });
    }
    analysis
}

fn exact_rule_token_length(
    rule_index: usize,
    rules: &[Option<GeneratedParserRule>],
    memo: &mut [ExactTokenLength],
    active: &mut BTreeSet<usize>,
) -> Option<usize> {
    match memo.get(rule_index).copied()? {
        ExactTokenLength::Exact(length) => return Some(length),
        ExactTokenLength::Inexact => return None,
        ExactTokenLength::Unknown => {}
    }
    if !active.insert(rule_index) {
        return None;
    }
    let result = rules
        .get(rule_index)
        .and_then(Option::as_ref)
        .filter(|rule| !rule.left_recursive)
        .and_then(|rule| exact_steps_token_length(&rule.steps, rules, memo, active));
    active.remove(&rule_index);
    if let Some(slot) = memo.get_mut(rule_index) {
        *slot = result.map_or(ExactTokenLength::Inexact, ExactTokenLength::Exact);
    }
    result
}

fn exact_steps_token_length(
    steps: &[GeneratedParserStep],
    rules: &[Option<GeneratedParserRule>],
    memo: &mut [ExactTokenLength],
    active: &mut BTreeSet<usize>,
) -> Option<usize> {
    let mut total = 0_usize;
    for step in steps {
        let length = match step {
            GeneratedParserStep::MatchToken { .. }
            | GeneratedParserStep::MatchSet { .. }
            | GeneratedParserStep::MatchNotSet { .. }
            | GeneratedParserStep::MatchWildcard { .. } => 1,
            GeneratedParserStep::Precedence(_)
            | GeneratedParserStep::Predicate { .. }
            | GeneratedParserStep::Action { .. } => 0,
            GeneratedParserStep::CallRule { rule_index, .. } => {
                exact_rule_token_length(*rule_index, rules, memo, active)?
            }
            GeneratedParserStep::Decision { alts, .. } => {
                let mut lengths = alts
                    .iter()
                    .map(|alt| exact_steps_token_length(alt, rules, memo, active));
                let first = lengths.next()??;
                if lengths.any(|length| length != Some(first)) {
                    return None;
                }
                first
            }
            GeneratedParserStep::StarLoop { .. }
            | GeneratedParserStep::LeftRecursiveLoop { .. } => return None,
        };
        total = total.checked_add(length)?;
    }
    Some(total)
}

fn plan_is_dominated(plan: &SharedDescentPlan, existing: &SharedDescentPlan) -> bool {
    plan.conditional == existing.conditional
        && plan.prefix_tokens == existing.prefix_tokens
        && plan.alternatives == existing.alternatives
        && plan.tails == existing.tails
        && plan.default_alt == existing.default_alt
        && plan.default_excluded_intervals == existing.default_excluded_intervals
        && preview_plan_dominates(existing, plan)
        && intervals_cover(&existing.trigger_intervals, &plan.trigger_intervals)
}

const fn preview_plan_dominates(existing: &SharedDescentPlan, plan: &SharedDescentPlan) -> bool {
    !matches!(
        (existing.common_token_length, plan.common_token_length),
        (None, Some(_))
    )
}

fn intervals_cover(outer: &[(i32, i32)], inner: &[(i32, i32)]) -> bool {
    inner.iter().all(|(inner_start, inner_stop)| {
        outer
            .iter()
            .any(|(outer_start, outer_stop)| outer_start <= inner_start && outer_stop >= inner_stop)
    })
}

fn generated_decision_shapes(
    rules: &[Option<GeneratedParserRule>],
) -> BTreeMap<usize, GeneratedDecisionShape> {
    let mut shapes = BTreeMap::new();
    for rule in rules.iter().flatten() {
        collect_generated_decision_shapes(&rule.steps, rule.rule_index, &mut shapes);
    }
    shapes
}

fn collect_generated_decision_shapes(
    steps: &[GeneratedParserStep],
    owning_rule: usize,
    shapes: &mut BTreeMap<usize, GeneratedDecisionShape>,
) {
    for step in steps {
        match step {
            GeneratedParserStep::Decision {
                state,
                decision,
                allow_semantic_context,
                force_context,
                alts,
                ..
            } => {
                shapes.insert(
                    *decision,
                    GeneratedDecisionShape {
                        state: *state,
                        owning_rule,
                        allow_semantic_context: *allow_semantic_context,
                        force_context: *force_context,
                    },
                );
                for alt in alts {
                    collect_generated_decision_shapes(alt, owning_rule, shapes);
                }
            }
            GeneratedParserStep::StarLoop { body, .. }
            | GeneratedParserStep::LeftRecursiveLoop { body, .. } => {
                collect_generated_decision_shapes(body, owning_rule, shapes);
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

fn candidate_groups(
    atn: &ParserAtn,
    state: ParserAtnState<'_>,
) -> BTreeMap<SharedDescentGroupKey, Vec<AltPath>> {
    let mut grouped = BTreeMap::<SharedDescentGroupKey, Vec<AltPath>>::new();
    for (index, transition) in state.transitions().iter().enumerate() {
        let alt = index + 1;
        for mut path in alt_paths(atn, transition.target(), alt) {
            let starts = post_prefix_states(atn, transition.target(), &path.prefix_tokens);
            let mut found = false;
            let mut semantic = path.semantic_descent;
            let mut head_guard = BTreeSet::new();
            for start in starts {
                let mut walk = DescentWalk::new(atn, path.common_rule);
                match walk.check(start) {
                    DescentVerdict::Always | DescentVerdict::Sometimes => {
                        found = true;
                        head_guard.extend(walk.head_guard);
                    }
                    DescentVerdict::NoCleanPath => {}
                    DescentVerdict::Semantic => {
                        semantic = true;
                        found = true;
                    }
                    DescentVerdict::Veto => {
                        found = false;
                        break;
                    }
                }
            }
            if !found {
                continue;
            }
            path.head_guard.extend(head_guard);
            path.semantic_descent = semantic;
            grouped
                .entry(SharedDescentGroupKey {
                    common_rule: path.common_rule,
                    prefix_tokens: path.prefix_tokens.clone(),
                })
                .or_default()
                .push(path);
        }
    }
    grouped.retain(|_, paths| {
        paths
            .iter()
            .map(|path| path.alt)
            .collect::<BTreeSet<_>>()
            .len()
            >= 2
    });
    grouped
}

fn alt_paths(atn: &ParserAtn, start: usize, alt: usize) -> Vec<AltPath> {
    let mut out = Vec::new();
    let mut work = vec![(
        start,
        Vec::<i32>::new(),
        false,
        Vec::<DescentCall>::new(),
        false,
    )];
    let mut seen = BTreeSet::new();
    while let Some((state_number, prefix, in_rule, chain, semantic)) = work.pop() {
        if !seen.insert((state_number, prefix.clone(), in_rule)) {
            continue;
        }
        let Some(state) = atn.state(state_number) else {
            continue;
        };
        if state.kind() == AtnStateKind::RuleStop {
            continue;
        }
        for transition in state.transitions() {
            match transition.data() {
                ParserTransitionData::Rule {
                    target,
                    rule_index,
                    follow_state,
                    ..
                } => {
                    let mut next_chain = chain.clone();
                    next_chain.push(DescentCall {
                        source_state: state_number,
                        rule_index,
                        follow_state,
                    });
                    out.push(AltPath {
                        alt,
                        common_rule: rule_index,
                        prefix_tokens: prefix.clone(),
                        chain: next_chain.clone(),
                        head_guard: BTreeSet::new(),
                        semantic_descent: semantic,
                    });
                    work.push((target, prefix.clone(), true, next_chain, semantic));
                    if rule_is_nullable(atn, rule_index) {
                        work.push((
                            follow_state,
                            prefix.clone(),
                            in_rule,
                            chain.clone(),
                            semantic,
                        ));
                    }
                }
                ParserTransitionData::Epsilon { target } => {
                    work.push((target, prefix.clone(), in_rule, chain.clone(), semantic));
                }
                ParserTransitionData::Action { target, .. }
                | ParserTransitionData::Predicate { target, .. }
                | ParserTransitionData::Precedence { target, .. } => {
                    work.push((target, prefix.clone(), in_rule, chain.clone(), true));
                }
                _ if !in_rule && prefix.len() < MAX_SHARED_DESCENT_PREFIX => {
                    let symbols = generated_transition_symbols(transition, atn.max_token_type());
                    if symbols.len() <= MAX_SHARED_DESCENT_PREFIX_FANOUT {
                        for symbol in symbols {
                            let mut next = prefix.clone();
                            next.push(symbol);
                            work.push((transition.target(), next, false, chain.clone(), semantic));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn post_prefix_states(atn: &ParserAtn, start: usize, prefix: &[i32]) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    let mut work = vec![(start, 0_usize)];
    let mut seen = BTreeSet::new();
    while let Some((state_number, consumed)) = work.pop() {
        if !seen.insert((state_number, consumed)) {
            continue;
        }
        if consumed == prefix.len() {
            out.insert(state_number);
            continue;
        }
        let Some(state) = atn.state(state_number) else {
            continue;
        };
        if state.kind() == AtnStateKind::RuleStop {
            continue;
        }
        for transition in state.transitions() {
            match transition.data() {
                ParserTransitionData::Rule {
                    target,
                    rule_index,
                    follow_state,
                    ..
                } => {
                    work.push((target, consumed));
                    if rule_is_nullable(atn, rule_index) {
                        work.push((follow_state, consumed));
                    }
                }
                ParserTransitionData::Epsilon { target }
                | ParserTransitionData::Action { target, .. }
                | ParserTransitionData::Predicate { target, .. }
                | ParserTransitionData::Precedence { target, .. } => {
                    work.push((target, consumed));
                }
                _ => {
                    let symbols = generated_transition_symbols(transition, atn.max_token_type());
                    if symbols.contains(&prefix[consumed]) {
                        work.push((transition.target(), consumed + 1));
                    }
                }
            }
        }
    }
    out
}

struct DescentWalk<'a> {
    atn: &'a ParserAtn,
    common_rule: usize,
    first: BTreeSet<i32>,
    head_guard: BTreeSet<i32>,
    active: BTreeSet<(usize, usize)>,
    steps: usize,
}

impl<'a> DescentWalk<'a> {
    fn new(atn: &'a ParserAtn, common_rule: usize) -> Self {
        Self {
            atn,
            common_rule,
            first: rule_first_symbols(atn, common_rule),
            head_guard: BTreeSet::new(),
            active: BTreeSet::new(),
            steps: 0,
        }
    }

    fn check(&mut self, start: usize) -> DescentVerdict {
        let Some(rule_index) = self.atn.state(start).and_then(ParserAtnState::rule_index) else {
            return DescentVerdict::Veto;
        };
        let key = (rule_index, start);
        if !self.active.insert(key) {
            return DescentVerdict::Veto;
        }
        let verdict = self.check_inner(start);
        self.active.remove(&key);
        verdict
    }

    fn check_inner(&mut self, start: usize) -> DescentVerdict {
        let mut work = vec![start];
        let mut seen = BTreeSet::new();
        let (mut found, mut r_less) = (false, false);
        while let Some(state_number) = work.pop() {
            self.steps += 1;
            if self.steps > SHARED_DESCENT_WALK_BUDGET {
                return DescentVerdict::Veto;
            }
            if !seen.insert(state_number) {
                continue;
            }
            let Some(state) = self.atn.state(state_number) else {
                return DescentVerdict::Veto;
            };
            if state.kind() == AtnStateKind::RuleStop {
                r_less = true;
                continue;
            }
            for transition in state.transitions() {
                match transition.data() {
                    ParserTransitionData::Rule {
                        target,
                        rule_index,
                        follow_state,
                        ..
                    } => {
                        if rule_index == self.common_rule {
                            found = true;
                            continue;
                        }
                        let child_first = rule_first_symbols(self.atn, rule_index);
                        let overlap = child_first
                            .intersection(&self.first)
                            .copied()
                            .collect::<BTreeSet<_>>();
                        if overlap.is_empty() {
                            if rule_is_nullable(self.atn, rule_index) {
                                work.push(follow_state);
                            }
                            continue;
                        }
                        match self.check(target) {
                            DescentVerdict::Always => found = true,
                            DescentVerdict::Sometimes => {
                                found = true;
                                work.push(follow_state);
                            }
                            DescentVerdict::NoCleanPath | DescentVerdict::Veto => {
                                self.head_guard.extend(overlap);
                                if rule_is_nullable(self.atn, rule_index) {
                                    work.push(follow_state);
                                }
                            }
                            DescentVerdict::Semantic => return DescentVerdict::Semantic,
                        }
                    }
                    ParserTransitionData::Epsilon { target } => work.push(target),
                    ParserTransitionData::Action { .. }
                    | ParserTransitionData::Predicate { .. }
                    | ParserTransitionData::Precedence { .. } => {
                        return DescentVerdict::Semantic;
                    }
                    _ => {
                        let symbols =
                            generated_transition_symbols(transition, self.atn.max_token_type());
                        self.head_guard
                            .extend(symbols.intersection(&self.first).copied());
                    }
                }
            }
        }
        match (found, r_less) {
            (true, true) => DescentVerdict::Sometimes,
            (true, false) => DescentVerdict::Always,
            (false, _) => DescentVerdict::NoCleanPath,
        }
    }
}

fn neutral_rule_is_context_free(
    rule_index: usize,
    rules: &[Option<GeneratedParserRule>],
    semantic_rules: &BTreeSet<usize>,
    context_free_decisions: &BTreeSet<usize>,
    memo: &mut [Option<bool>],
    active: &mut BTreeSet<usize>,
) -> bool {
    if let Some(result) = memo.get(rule_index).copied().flatten() {
        return result;
    }
    if semantic_rules.contains(&rule_index) || !active.insert(rule_index) {
        return false;
    }
    let result = rules
        .get(rule_index)
        .and_then(Option::as_ref)
        .is_some_and(|rule| {
            !rule.left_recursive
                && neutral_steps_are_context_free(
                    &rule.steps,
                    rules,
                    semantic_rules,
                    context_free_decisions,
                    memo,
                    active,
                )
        });
    active.remove(&rule_index);
    if let Some(slot) = memo.get_mut(rule_index) {
        *slot = Some(result);
    }
    result
}

fn neutral_steps_are_context_free(
    steps: &[GeneratedParserStep],
    rules: &[Option<GeneratedParserRule>],
    semantic_rules: &BTreeSet<usize>,
    context_free_decisions: &BTreeSet<usize>,
    memo: &mut [Option<bool>],
    active: &mut BTreeSet<usize>,
) -> bool {
    steps.iter().all(|step| match step {
        GeneratedParserStep::MatchToken { .. }
        | GeneratedParserStep::MatchSet { .. }
        | GeneratedParserStep::MatchNotSet { .. }
        | GeneratedParserStep::MatchWildcard { .. } => true,
        GeneratedParserStep::Precedence(_)
        | GeneratedParserStep::Predicate { .. }
        | GeneratedParserStep::Action { .. }
        | GeneratedParserStep::LeftRecursiveLoop { .. } => false,
        GeneratedParserStep::CallRule { rule_index, .. } => neutral_rule_is_context_free(
            *rule_index,
            rules,
            semantic_rules,
            context_free_decisions,
            memo,
            active,
        ),
        GeneratedParserStep::Decision { decision, alts, .. } => {
            context_free_decisions.contains(decision)
                && alts.iter().all(|alt| {
                    neutral_steps_are_context_free(
                        alt,
                        rules,
                        semantic_rules,
                        context_free_decisions,
                        memo,
                        active,
                    )
                })
        }
        GeneratedParserStep::StarLoop { decision, body, .. } => {
            context_free_decisions.contains(decision)
                && neutral_steps_are_context_free(
                    body,
                    rules,
                    semantic_rules,
                    context_free_decisions,
                    memo,
                    active,
                )
        }
    })
}

fn restrict_trigger_against_nonmembers(
    atn: &ParserAtn,
    state: ParserAtnState<'_>,
    owning_rule: usize,
    prefix: &[i32],
    alternatives: &[usize],
    trigger: &mut BTreeSet<i32>,
) {
    let members = alternatives.iter().copied().collect::<BTreeSet<_>>();
    let max_member = alternatives.iter().copied().max().unwrap_or_default();
    let Some(rule_stop) = atn.rule_to_stop_state().get(owning_rule) else {
        trigger.clear();
        return;
    };
    for (index, transition) in state.transitions().iter().enumerate() {
        let alt = index + 1;
        if members.contains(&alt) || alt > max_member {
            continue;
        }
        let mut first = BTreeSet::new();
        for post in post_prefix_states(atn, transition.target(), prefix) {
            let mut ctx = GeneratedFirstSetCtx::default();
            first.extend(generated_rule_first_set(atn, post, rule_stop, &mut ctx).symbols);
        }
        for symbol in first {
            trigger.remove(&symbol);
        }
    }
}

fn tail_dispatch(
    atn: &ParserAtn,
    owning_rule: usize,
    alternatives: &[usize],
    paths: &[AltPath],
) -> Option<TailDispatch> {
    let mut looks = BTreeMap::<usize, TailLook>::new();
    for alt in alternatives {
        let mut look = TailLook::default();
        for path in paths.iter().filter(|path| path.alt == *alt) {
            let chain = tail_look_for_chain(atn, owning_rule, &path.chain);
            look.symbols.extend(chain.symbols);
            look.nullable |= chain.nullable;
        }
        looks.insert(*alt, look);
    }
    let nullable = alternatives
        .iter()
        .copied()
        .filter(|alt| looks.get(alt).is_some_and(|look| look.nullable))
        .collect::<Vec<_>>();
    let default_alt = (nullable.len() == 1).then(|| nullable[0]);
    let default_excluded_intervals = default_alt.map_or_else(Vec::new, |default_alt| {
        let symbols = alternatives
            .iter()
            .filter(|alt| **alt != default_alt)
            .filter_map(|alt| looks.get(alt))
            .flat_map(|look| look.symbols.iter().copied())
            .collect::<BTreeSet<_>>();
        symbols_to_ranges(symbols)
    });
    let mut tails = Vec::new();
    for alt in alternatives {
        let look = looks.get(alt)?;
        let mut unique = look.symbols.clone();
        for other in alternatives.iter().filter(|other| *other != alt) {
            if let Some(other) = looks.get(other) {
                unique.retain(|symbol| !other.symbols.contains(symbol));
            }
        }
        if !unique.is_empty() {
            tails.push(SharedDescentTail {
                alt: *alt,
                intervals: symbols_to_ranges(unique),
                guard_against_follow: !look.nullable && !nullable.is_empty(),
            });
        }
    }
    (!tails.is_empty() || default_alt.is_some()).then_some(TailDispatch {
        tails,
        default_alt,
        default_excluded_intervals,
    })
}

fn tail_look_for_chain(atn: &ParserAtn, owning_rule: usize, chain: &[DescentCall]) -> TailLook {
    let Some(last) = chain.last() else {
        return TailLook::default();
    };
    let mut look = TailLook::default();
    let mut work = vec![last.follow_state];
    let mut seen = BTreeSet::new();
    while let Some(state_number) = work.pop() {
        if !seen.insert(state_number) {
            continue;
        }
        let Some(state) = atn.state(state_number) else {
            continue;
        };
        if state.kind() == AtnStateKind::RuleStop {
            let frame = chain.iter().rposition(|call| {
                atn.state(call.source_state)
                    .and_then(ParserAtnState::rule_index)
                    == state.rule_index()
            });
            match frame {
                Some(index) if index > 0 => work.push(chain[index - 1].follow_state),
                _ if state.rule_index() == Some(owning_rule) => look.nullable = true,
                _ => {}
            }
            continue;
        }
        for transition in state.transitions() {
            let symbols = generated_transition_symbols(transition, atn.max_token_type());
            if !symbols.is_empty() {
                look.symbols.extend(symbols);
                continue;
            }
            match transition.data() {
                ParserTransitionData::Rule {
                    target,
                    rule_index,
                    follow_state,
                    ..
                } => {
                    let Some(child_stop) = atn.rule_to_stop_state().get(rule_index) else {
                        continue;
                    };
                    let mut ctx = GeneratedFirstSetCtx::default();
                    let child = generated_rule_first_set(atn, target, child_stop, &mut ctx);
                    look.symbols.extend(child.symbols);
                    if child.nullable {
                        work.push(follow_state);
                    }
                }
                ParserTransitionData::Epsilon { target }
                | ParserTransitionData::Action { target, .. }
                | ParserTransitionData::Predicate { target, .. }
                | ParserTransitionData::Precedence { target, .. } => work.push(target),
                ParserTransitionData::Atom { .. }
                | ParserTransitionData::Range { .. }
                | ParserTransitionData::Set { .. }
                | ParserTransitionData::NotSet { .. }
                | ParserTransitionData::Wildcard { .. } => {}
            }
        }
    }
    look
}

fn rule_first_symbols(atn: &ParserAtn, rule_index: usize) -> BTreeSet<i32> {
    let Some(start) = atn.rule_to_start_state().get(rule_index) else {
        return BTreeSet::new();
    };
    let Some(stop) = atn.rule_to_stop_state().get(rule_index) else {
        return BTreeSet::new();
    };
    generated_rule_first_set(atn, start, stop, &mut GeneratedFirstSetCtx::default()).symbols
}

fn rule_is_nullable(atn: &ParserAtn, rule_index: usize) -> bool {
    let Some(start) = atn.rule_to_start_state().get(rule_index) else {
        return false;
    };
    let Some(stop) = atn.rule_to_stop_state().get(rule_index) else {
        return false;
    };
    generated_rule_first_set(atn, start, stop, &mut GeneratedFirstSetCtx::default()).nullable
}

fn symbols_to_ranges(symbols: BTreeSet<i32>) -> Vec<(i32, i32)> {
    let mut ranges = Vec::new();
    for symbol in symbols {
        match ranges.last_mut() {
            Some((_, stop)) if *stop + 1 == symbol => *stop = symbol,
            _ => ranges.push((symbol, symbol)),
        }
    }
    ranges
}
