/// LOOK(1) of one decision alternative, plus whether the walk crossed a
/// predicate — Java nulls such alternatives, forcing the decision adaptive.
#[derive(Debug, Default)]
struct DecisionAltLook {
    symbols: BTreeSet<i32>,
    hit_pred: bool,
    /// This alternative can reach the owning rule's stop state without
    /// consuming input, so generated optional/loop dispatch may select it as
    /// the default after nullable synchronization.
    nullable: bool,
}

/// Upper bound accepted by `--fixed-lookahead`. Dispatch-table size and
/// analysis cost grow with depth; beyond a few tokens the tier stops paying
/// for itself, so cap the flag rather than let a typo explode generation.
pub(crate) const MAX_FIXED_LOOKAHEAD_FLAG: u8 = 8;
/// Per-alternative cap on lookahead rectangles gathered by the fixed-LL(k)
/// walk. This is an *analysis* budget: generous enough that ordinary
/// decisions get an honest disjoint / not-disjoint verdict (fan-outy
/// recursive regions need thousands of rectangles at depth 2-3), while the
/// emitted code size is bounded separately by the table-arm budget.
const FIXED_LOOKAHEAD_RECTANGLE_BUDGET: usize = 8192;
/// Per-alternative cap on total walk steps (closure visits), guarding
/// against pathological ATN regions; deterministic by construction.
const FIXED_LOOKAHEAD_STEP_BUDGET: usize = 200_000;
/// Cap on total dispatch-trie arms actually emitted for one decision. A
/// disjointness proof over a huge rectangle set would compile into an
/// unreasonably large `match`; past this size the adaptive simulator's
/// learned DFA is the better engine, so decline the table.
const FIXED_LOOKAHEAD_TABLE_ARM_BUDGET: usize = 256;

/// Ports the ANTLR tool's `AnalysisPipeline` classification: the decisions
/// whose alternatives' LOOK(1) sets are not pairwise disjoint (or hit a
/// predicate, or come up empty). Java compiles LL(1)-disjoint decisions to
/// plain token switches — no simulator, no learned DFA — and routes every
/// other decision through `adaptivePredict`, the only place DFA states are
/// learned and full-context diagnostics fire. `dumpDFA` and diagnostic
/// output therefore only match Java when the generated routing agrees.
///
/// On top of that Java-parity split, `--fixed-lookahead k` (k >= 2) probes
/// the residual `adaptivePredict` decisions with a bounded LOOK(k) walk and
/// compiles the ones whose k-token lookahead languages are pairwise disjoint
/// into static dispatch tables ([`FixedLookaheadTable`]). Every tier verdict
/// is recorded in [`ParserDecisionAnalysis::report_rows`] for the
/// `decisions.json` manifest.
pub(crate) fn classify_decisions(
    data: &ParserCodegenData<'_>,
    fixed_lookahead: Option<usize>,
) -> ParserDecisionAnalysis {
    let atn = data.parser_atn();
    let max_lookahead = fixed_lookahead.unwrap_or(1);
    let mut classification = ParserDecisionAnalysis::default();
    for (decision, state_number) in atn.decision_to_state().iter().enumerate() {
        let Some(state) = atn.state(state_number) else {
            continue;
        };
        let report = |tier: DecisionTierReport| DecisionReportRow {
            decision,
            state: state_number,
            rule_index: state.rule_index(),
            fallback: tier.fallback_capability(),
            tier,
        };
        // The tool never LL(1)-compiles non-greedy or left-recursion
        // precedence decisions, disjoint LOOK or not — Java always emits
        // `adaptivePredict` for them (a token switch would make a
        // non-greedy loop greedy).
        if state.non_greedy() {
            classification.adaptive_decisions.insert(decision);
            classification
                .report_rows
                .push(report(DecisionTierReport::adaptive(
                    AdaptiveReason::NonGreedy,
                    0,
                )));
            continue;
        }
        if state.precedence_rule_decision() {
            classification.adaptive_decisions.insert(decision);
            classification
                .report_rows
                .push(report(DecisionTierReport::adaptive(
                    AdaptiveReason::Precedence,
                    0,
                )));
            continue;
        }
        let looks: Vec<DecisionAltLook> = state
            .transitions()
            .iter()
            .map(|transition| {
                let mut look = DecisionAltLook::default();
                let mut walk = DecisionLookWalk {
                    atn,
                    owning_rule_stop: state
                        .rule_index()
                        .and_then(|rule_index| atn.rule_to_stop_state().get(rule_index)),
                    busy: BTreeSet::new(),
                    called_rules: vec![false; atn.rule_to_start_state().len()],
                };
                walk.walk(transition.target(), &mut Vec::new(), &mut look);
                look
            })
            .collect();
        if looks.iter().any(|look| look.hit_pred) {
            classification.adaptive_decisions.insert(decision);
            classification
                .report_rows
                .push(report(DecisionTierReport::adaptive(
                    AdaptiveReason::Predicate,
                    1,
                )));
            continue;
        }
        if looks.iter().any(|look| look.symbols.is_empty()) {
            classification.adaptive_decisions.insert(decision);
            classification
                .report_rows
                .push(report(DecisionTierReport::adaptive(
                    AdaptiveReason::EmptyLook,
                    1,
                )));
            continue;
        }
        if decision_alt_looks_disjoint(&looks) {
            let arms: Vec<Vec<(i32, i32)>> = looks
                .iter()
                .map(|look| symbol_intervals(&look.symbols))
                .collect();
            let default_alt = looks
                .iter()
                .enumerate()
                .find_map(|(index, look)| look.nullable.then_some(index + 1));
            // With the opt-in flag, plain mode compiles the switch
            // statically. Only arms over sync-no-op lookahead commit
            // without the decision's recovery synchronization.
            if fixed_lookahead.is_some() {
                let root = FixedLookaheadNode::Probe(
                    arms.iter()
                        .enumerate()
                        .filter(|(_, intervals)| !intervals.is_empty())
                        .map(|(index, intervals)| {
                            (intervals.clone(), FixedLookaheadNode::Alt(index + 1))
                        })
                        .collect(),
                );
                if let Some(root) = restrict_dispatch_to_sync_noop(atn, state, root) {
                    classification
                        .ll1_dispatch_tables
                        .insert(decision, FixedLookaheadTable { lookahead: 1, root });
                }
            }
            classification.complete_ll1_dispatches.insert(
                decision,
                CompleteLl1Dispatch {
                    fast_path: GeneratedDecisionFastPath {
                        arms: arms
                            .into_iter()
                            .enumerate()
                            .map(|(index, intervals)| GeneratedDecisionFastArm {
                                alt: index + 1,
                                intervals,
                            })
                            .collect(),
                    },
                    default_alt,
                },
            );
            classification
                .report_rows
                .push(report(DecisionTierReport::Ll1));
            continue;
        }
        // Not LL(1): Java parity keeps the decision adaptive. With the
        // opt-in flag, probe increasing fixed depths before giving up.
        classification.adaptive_decisions.insert(decision);
        let mut tier = DecisionTierReport::adaptive(AdaptiveReason::NotDisjoint, max_lookahead);
        for depth in 2..=max_lookahead {
            match fixed_lookahead_table(atn, state, depth) {
                FixedLookaheadOutcome::Table(table) => {
                    match restrict_dispatch_to_sync_noop(atn, state, table.root) {
                        Some(root) => {
                            classification.fixed_lookahead_tables.insert(
                                decision,
                                FixedLookaheadTable {
                                    lookahead: table.lookahead,
                                    root,
                                },
                            );
                            tier = DecisionTierReport::Fixed { lookahead: depth };
                        }
                        None => {
                            tier = DecisionTierReport::adaptive(AdaptiveReason::SyncBound, depth);
                        }
                    }
                    break;
                }
                FixedLookaheadOutcome::NotDisjoint => {}
                FixedLookaheadOutcome::Predicate => {
                    tier = DecisionTierReport::adaptive(AdaptiveReason::Predicate, depth);
                    break;
                }
                FixedLookaheadOutcome::Budget => {
                    tier = DecisionTierReport::adaptive(AdaptiveReason::Budget, depth);
                    break;
                }
            }
        }
        classification.report_rows.push(report(tier));
    }
    classification
}

/// Restricts a dispatch trie's first-token arms to the decision's
/// within-rule lookahead — the exact set for which the runtime's
/// `sync_decision` early-returns without recovery work — so a table hit can
/// commit without running the synchronization the untiered parser would
/// no-op through, while every other token falls through to the full
/// sync + adaptive body. Returns `None` when no arm survives.
fn restrict_dispatch_to_sync_noop(
    atn: &ParserAtn,
    state: ParserAtnState<'_>,
    root: FixedLookaheadNode,
) -> Option<FixedLookaheadNode> {
    let allowed = sync_noop_symbol_intervals(atn, state);
    let FixedLookaheadNode::Probe(arms) = root else {
        // A rootless commit would skip synchronization for every token;
        // decline (decision states always probe, so this is defensive).
        return None;
    };
    let arms: Vec<(Vec<(i32, i32)>, FixedLookaheadNode)> = arms
        .into_iter()
        .filter_map(|(intervals, child)| {
            let restricted = intersect_interval_sets(&intervals, &allowed);
            (!restricted.is_empty()).then_some((restricted, child))
        })
        .collect();
    (!arms.is_empty()).then_some(FixedLookaheadNode::Probe(arms))
}

/// The decision state's within-rule lookahead: the union of every
/// alternative's FIRST set bounded at the owning rule's stop state,
/// mirroring the runtime's `transition_first_set`, whose per-transition
/// sets are exactly `sync_decision`'s early-return condition.
pub(crate) fn sync_noop_symbol_intervals(
    atn: &ParserAtn,
    state: ParserAtnState<'_>,
) -> Vec<(i32, i32)> {
    let Some(rule_index) = state.rule_index() else {
        return Vec::new();
    };
    let Some(rule_stop) = atn.rule_to_stop_state().get(rule_index) else {
        return Vec::new();
    };
    let mut ctx = GeneratedFirstSetCtx::default();
    let mut symbols = BTreeSet::new();
    for transition in state.transitions() {
        let direct = generated_transition_symbols(transition, atn.max_token_type());
        if !direct.is_empty() {
            symbols.extend(direct);
            continue;
        }
        match transition.data() {
            ParserTransitionData::Rule {
                target,
                rule_index: child_rule,
                follow_state,
                ..
            } => {
                let Some(child_stop) = atn.rule_to_stop_state().get(child_rule) else {
                    continue;
                };
                let child = generated_rule_first_set(atn, target, child_stop, &mut ctx);
                symbols.extend(child.symbols.iter().copied());
                if child.nullable {
                    let follow = generated_rule_first_set(atn, follow_state, rule_stop, &mut ctx);
                    symbols.extend(follow.symbols.iter().copied());
                }
            }
            ParserTransitionData::Epsilon { target }
            | ParserTransitionData::Action { target, .. }
            | ParserTransitionData::Predicate { target, .. }
            | ParserTransitionData::Precedence { target, .. } => {
                let first = generated_rule_first_set(atn, target, rule_stop, &mut ctx);
                symbols.extend(first.symbols.iter().copied());
            }
            _ => {}
        }
    }
    symbol_intervals(&symbols)
}

/// Intersection of two sorted disjoint interval sets.
pub(crate) fn intersect_interval_sets(
    left: &[(i32, i32)],
    right: &[(i32, i32)],
) -> Vec<(i32, i32)> {
    let mut result = Vec::new();
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() && right_index < right.len() {
        let (left_start, left_stop) = left[left_index];
        let (right_start, right_stop) = right[right_index];
        let start = left_start.max(right_start);
        let stop = left_stop.min(right_stop);
        if start <= stop {
            result.push((start, stop));
        }
        if left_stop < right_stop {
            left_index += 1;
        } else {
            right_index += 1;
        }
    }
    result
}

/// Validated prediction facts for every parser decision.
///
/// This is the decision-analysis stage artifact consumed by IR routing and
/// reporting. It contains no rendered Rust and does not select parser
/// surfaces.
#[derive(Debug, Default)]
pub(crate) struct ParserDecisionAnalysis {
    /// Decisions Java compiles to `adaptivePredict` calls. Decisions that
    /// additionally earned a [`FixedLookaheadTable`] stay in this set: the
    /// table's fall-through branch must render the same adaptive body the
    /// decision would get without the table.
    pub(crate) adaptive_decisions: BTreeSet<usize>,
    /// Complete LOOK(1) dispatches for LL(1)-disjoint decisions — exit
    /// alternatives included, unlike the within-rule fast-path/LL(1)
    /// analyses. A nullable default alternative makes recovery misses total
    /// without deferring to the simulator.
    pub(crate) complete_ll1_dispatches: BTreeMap<usize, CompleteLl1Dispatch>,
    /// `--fixed-lookahead` in plain mode: depth-1 static dispatch for the
    /// LL(1) decisions above, restricted to sync-no-op lookahead.
    pub(crate) ll1_dispatch_tables: BTreeMap<usize, FixedLookaheadTable>,
    /// `--fixed-lookahead`: static dispatch tables for decisions whose
    /// LOOK(k) languages are pairwise disjoint at some 2 <= k <= flag,
    /// restricted to sync-no-op lookahead.
    pub(crate) fixed_lookahead_tables: BTreeMap<usize, FixedLookaheadTable>,
    /// Per-decision tier verdicts for the `decisions.json` manifest.
    pub(crate) report_rows: Vec<DecisionReportRow>,
}

/// One decision's verdict for the `decisions.json` manifest.
#[derive(Clone, Debug)]
pub(crate) struct DecisionReportRow {
    pub(crate) decision: usize,
    pub(crate) state: usize,
    pub(crate) rule_index: Option<usize>,
    pub(crate) fallback: DecisionFallbackCapability,
    pub(crate) tier: DecisionTierReport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DecisionTierReport {
    /// LOOK(1)-disjoint: Java compiles a token switch; no prediction runs.
    Ll1,
    /// Disjoint at a fixed `lookahead` >= 2; static dispatch table emitted
    /// (only reported when `--fixed-lookahead` enabled the probe).
    Fixed { lookahead: usize },
    /// Stays on adaptive prediction. `probed_lookahead` is the deepest
    /// lookahead the classifier examined before settling on the reason.
    Adaptive {
        reason: AdaptiveReason,
        probed_lookahead: usize,
    },
}

impl DecisionTierReport {
    const fn adaptive(reason: AdaptiveReason, probed_lookahead: usize) -> Self {
        Self::Adaptive {
            reason,
            probed_lookahead,
        }
    }

    const fn fallback_capability(self) -> DecisionFallbackCapability {
        match self {
            Self::Ll1 => DecisionFallbackCapability::None,
            Self::Fixed { .. } | Self::Adaptive { .. } => DecisionFallbackCapability::CanDefer,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DecisionFallbackCapability {
    None,
    CanDefer,
}

impl DecisionFallbackCapability {
    pub(crate) const fn can_defer(self) -> bool {
        matches!(self, Self::CanDefer)
    }
}

/// Why a decision keeps `adaptivePredict` routing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdaptiveReason {
    /// Non-greedy loop entry: a dispatch table would make it greedy.
    NonGreedy,
    /// Left-recursion precedence decision: gated on the runtime
    /// precedence stack, invisible to token lookahead.
    Precedence,
    /// A semantic predicate guards lookahead-reachable paths; only the
    /// simulator evaluates predicates during prediction.
    Predicate,
    /// Some alternative's lookahead set came up empty (Java nulls the
    /// whole decision in `getDecisionLookahead`).
    EmptyLook,
    /// Lookahead languages stay overlapping at every probed depth.
    NotDisjoint,
    /// The fixed-lookahead walk exceeded its rectangle or step budget.
    Budget,
    /// Disjointness was proven, but every first-token dispatch symbol lies
    /// outside the decision's within-rule lookahead — the region where
    /// recovery synchronization is a provable no-op — so no arm survives
    /// the sync-safety restriction.
    SyncBound,
}

impl AdaptiveReason {
    pub(crate) const fn manifest_name(self) -> &'static str {
        match self {
            Self::NonGreedy => "non-greedy",
            Self::Precedence => "precedence",
            Self::Predicate => "predicate",
            Self::EmptyLook => "empty-look",
            Self::NotDisjoint => "not-disjoint",
            Self::Budget => "budget-exceeded",
            Self::SyncBound => "sync-bound",
        }
    }
}

/// One k-token lookahead "rectangle": dimension `d` holds the interval set
/// of tokens an ATN path can match at lookahead position `d + 1`. A
/// rectangle denotes the cross product of its dimensions, and the union of
/// an alternative's rectangles is exactly its LOOK(k) language: every path
/// through the ATN consuming k terminal edges contributes the cross product
/// of the sets those edges match, and every LOOK(k) word arises from such a
/// path. Paths that reach end-of-input early pad the remaining dimensions
/// with `{EOF}`, matching a token stream's behavior of returning EOF for
/// every position past the end.
type LookaheadRectangle = Vec<Vec<(i32, i32)>>;

/// Static dispatch table for one fixed-LL(k) decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FixedLookaheadTable {
    pub(crate) lookahead: usize,
    pub(crate) root: FixedLookaheadNode,
}

/// Dispatch trie over `la(1) .. la(k)`. Arms at each probe level are
/// pairwise disjoint interval sets; lookahead words outside every arm fall
/// through to synchronization and the decision's tier-specific miss path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FixedLookaheadNode {
    /// Every surviving rectangle belongs to this 1-based alternative; no
    /// further lookahead is read.
    Alt(usize),
    /// Probe the next lookahead token and descend.
    Probe(Vec<(Vec<(i32, i32)>, Self)>),
}

enum FixedLookaheadOutcome {
    Table(FixedLookaheadTable),
    NotDisjoint,
    Predicate,
    Budget,
}

/// Probes one decision at exactly `depth` tokens of lookahead: walks every
/// alternative's LOOK(depth) rectangles, requires the alternatives'
/// languages to be pairwise disjoint, and compiles the dispatch trie.
fn fixed_lookahead_table(
    atn: &ParserAtn,
    state: ParserAtnState<'_>,
    depth: usize,
) -> FixedLookaheadOutcome {
    let mut alt_rectangles: Vec<Vec<LookaheadRectangle>> = Vec::new();
    for transition in state.transitions() {
        let mut walk = FixedLookaheadWalk {
            atn,
            depth_limit: depth,
            steps: 0,
        };
        match walk.alt_rectangles(transition.target()) {
            Ok(rectangles) if rectangles.is_empty() => {
                // No k-token word reaches here (only possible through walk
                // pruning); without a complete language the table would be
                // unsound, so keep the decision adaptive.
                return FixedLookaheadOutcome::NotDisjoint;
            }
            Ok(rectangles) => alt_rectangles.push(rectangles),
            Err(FixedWalkBail::Predicate) => return FixedLookaheadOutcome::Predicate,
            Err(FixedWalkBail::Budget) => return FixedLookaheadOutcome::Budget,
        }
    }
    for (left, left_rectangles) in alt_rectangles.iter().enumerate() {
        for right_rectangles in alt_rectangles.iter().skip(left + 1) {
            let overlap = left_rectangles.iter().any(|left_rectangle| {
                right_rectangles
                    .iter()
                    .any(|right_rectangle| rectangles_overlap(left_rectangle, right_rectangle))
            });
            if overlap {
                return FixedLookaheadOutcome::NotDisjoint;
            }
        }
    }
    let items: Vec<(usize, &LookaheadRectangle)> = alt_rectangles
        .iter()
        .enumerate()
        .flat_map(|(index, rectangles)| {
            rectangles
                .iter()
                .map(move |rectangle| (index + 1, rectangle))
        })
        .collect();
    match build_fixed_lookahead_node(&items, 0, depth) {
        Some(root) => {
            // A valid proof can still compile into an unreasonably large
            // `match`; past the arm budget the simulator's learned DFA is
            // the better engine.
            if fixed_lookahead_node_arm_count(&root) > FIXED_LOOKAHEAD_TABLE_ARM_BUDGET {
                return FixedLookaheadOutcome::Budget;
            }
            FixedLookaheadOutcome::Table(FixedLookaheadTable {
                lookahead: depth,
                root,
            })
        }
        // Unreachable when disjointness holds; decline defensively rather
        // than emit a table the proof does not cover.
        None => FixedLookaheadOutcome::NotDisjoint,
    }
}

/// Total dispatch arms across the trie (leaves plus probe arms), the size
/// proxy for the emitted `match` code.
fn fixed_lookahead_node_arm_count(node: &FixedLookaheadNode) -> usize {
    match node {
        FixedLookaheadNode::Alt(_) => 1,
        FixedLookaheadNode::Probe(arms) => arms
            .iter()
            .map(|(_, child)| 1 + fixed_lookahead_node_arm_count(child))
            .sum(),
    }
}

/// Two k-dimensional rectangles overlap iff every dimension's interval
/// sets intersect.
fn rectangles_overlap(left: &LookaheadRectangle, right: &LookaheadRectangle) -> bool {
    left.iter()
        .zip(right.iter())
        .all(|(left_set, right_set)| interval_sets_intersect(left_set, right_set))
}

/// Whether two sorted disjoint interval sets share any symbol.
fn interval_sets_intersect(left: &[(i32, i32)], right: &[(i32, i32)]) -> bool {
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() && right_index < right.len() {
        let (left_start, left_stop) = left[left_index];
        let (right_start, right_stop) = right[right_index];
        if left_stop < right_start {
            left_index += 1;
        } else if right_stop < left_start {
            right_index += 1;
        } else {
            return true;
        }
    }
    false
}

/// Builds the dispatch trie for `items` (1-based alt, rectangle) from
/// dimension `dim`. Returns `None` only if two alternatives still share a
/// full lookahead word — impossible once pairwise disjointness holds, but
/// declining beats emitting a table the proof does not cover.
fn build_fixed_lookahead_node(
    items: &[(usize, &LookaheadRectangle)],
    dim: usize,
    depth: usize,
) -> Option<FixedLookaheadNode> {
    let first_alt = items.first().map(|(alt, _)| *alt)?;
    if items.iter().all(|(alt, _)| *alt == first_alt) {
        return Some(FixedLookaheadNode::Alt(first_alt));
    }
    if dim >= depth {
        return None;
    }
    // Atomize this dimension: split the token space at every interval
    // boundary so each atom is covered by a fixed subset of rectangles.
    let mut bounds = BTreeSet::new();
    for (_, rectangle) in items {
        for (start, stop) in &rectangle[dim] {
            bounds.insert(*start);
            bounds.insert(stop.checked_add(1)?);
        }
    }
    let bounds: Vec<i32> = bounds.into_iter().collect();
    let mut arms: Vec<(Vec<(i32, i32)>, FixedLookaheadNode)> = Vec::new();
    for window in bounds.windows(2) {
        let (atom_start, atom_stop) = (window[0], window[1] - 1);
        let covering: Vec<(usize, &LookaheadRectangle)> = items
            .iter()
            .filter(|(_, rectangle)| {
                interval_sets_intersect(&rectangle[dim], &[(atom_start, atom_start)])
            })
            .copied()
            .collect();
        if covering.is_empty() {
            continue;
        }
        let child = build_fixed_lookahead_node(&covering, dim + 1, depth)?;
        match arms.iter_mut().find(|(_, existing)| *existing == child) {
            Some((intervals, _)) => push_coalesced_interval(intervals, atom_start, atom_stop),
            None => arms.push((vec![(atom_start, atom_stop)], child)),
        }
    }
    Some(FixedLookaheadNode::Probe(arms))
}

/// Appends `[start, stop]` to a sorted interval list, merging with the
/// previous interval when adjacent (atoms arrive in ascending order).
fn push_coalesced_interval(intervals: &mut Vec<(i32, i32)>, start: i32, stop: i32) {
    match intervals.last_mut() {
        Some((_, last_stop)) if *last_stop + 1 == start => *last_stop = stop,
        _ => intervals.push((start, stop)),
    }
}

enum FixedWalkBail {
    Predicate,
    Budget,
}

/// Bounded LOOK(k) enumeration for one decision alternative.
///
/// Follows [`DecisionLookWalk`]'s structure — the same rule-stop return
/// handling, empty-context FOLLOW fallthrough, and per-closure recursion
/// guards — but keeps walking through consuming edges until `depth_limit`
/// tokens accumulate. The busy set and called-rule guard reset at every
/// consumed token (they cut epsilon cycles inside one closure segment;
/// re-entering a rule after consuming input is legitimate recursion), while
/// the simulated call stack carries across segments. Predicate or
/// precedence edges abort the alternative: only the simulator can evaluate
/// them during prediction, so any decision they guard stays adaptive.
struct FixedLookaheadWalk<'a> {
    atn: &'a ParserAtn,
    depth_limit: usize,
    steps: usize,
}

impl FixedLookaheadWalk<'_> {
    fn alt_rectangles(&mut self, start: usize) -> Result<Vec<LookaheadRectangle>, FixedWalkBail> {
        let mut rectangles = Vec::new();
        let rule_count = self.atn.rule_to_start_state().len();
        self.segment(
            start,
            &mut Vec::new(),
            &mut BTreeSet::new(),
            &mut vec![false; rule_count],
            &[],
            &mut rectangles,
        )?;
        Ok(rectangles)
    }

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn segment(
        &mut self,
        state_number: usize,
        ctx: &mut Vec<usize>,
        busy: &mut BTreeSet<(usize, Vec<usize>)>,
        called_rules: &mut Vec<bool>,
        prefix: &[Vec<(i32, i32)>],
        out: &mut Vec<LookaheadRectangle>,
    ) -> Result<(), FixedWalkBail> {
        self.steps += 1;
        if self.steps > FIXED_LOOKAHEAD_STEP_BUDGET {
            return Err(FixedWalkBail::Budget);
        }
        if !busy.insert((state_number, ctx.clone())) {
            return Ok(());
        }
        let Some(state) = self.atn.state(state_number) else {
            return Ok(());
        };
        if state.kind() == AtnStateKind::RuleStop {
            if let Some(return_state) = ctx.pop() {
                let cleared = state
                    .rule_index()
                    .map(|rule| std::mem::replace(&mut called_rules[rule], false));
                let result = self.segment(return_state, ctx, busy, called_rules, prefix, out);
                if let (Some(rule), Some(flag)) = (state.rule_index(), cleared) {
                    called_rules[rule] = flag;
                }
                ctx.push(return_state);
                return result;
            }
            if state.transitions().is_empty() {
                // Escaped the start rule's stop state: every remaining
                // lookahead position reads EOF.
                self.emit_eof_padded(prefix.to_vec(), out)?;
                return Ok(());
            }
        }
        for transition in state.transitions() {
            match transition.data() {
                ParserTransitionData::Rule {
                    target,
                    rule_index,
                    follow_state,
                    ..
                } => {
                    if called_rules.get(rule_index).copied().unwrap_or(true) {
                        continue;
                    }
                    called_rules[rule_index] = true;
                    ctx.push(follow_state);
                    let result = self.segment(target, ctx, busy, called_rules, prefix, out);
                    ctx.pop();
                    called_rules[rule_index] = false;
                    result?;
                }
                ParserTransitionData::Predicate { .. }
                | ParserTransitionData::Precedence { .. } => {
                    return Err(FixedWalkBail::Predicate);
                }
                ParserTransitionData::Epsilon { target }
                | ParserTransitionData::Action { target, .. } => {
                    self.segment(target, ctx, busy, called_rules, prefix, out)?;
                }
                _ => {
                    // Terminal edge: expand the transition's own
                    // label/range/set data — the same expansion the
                    // within-rule FIRST walks use, and identical to the
                    // simulator's `matches` for every terminal kind —
                    // instead of probing the whole vocabulary per visit
                    // (this walk revisits edges across segments and
                    // probed depths, so a vocabulary scan multiplies).
                    let mut symbols =
                        generated_transition_symbols(transition, self.atn.max_token_type());
                    // A consumed EOF pins every later position to EOF, so
                    // split it out of the deeper walk.
                    if symbols.remove(&TOKEN_EOF) {
                        let mut rectangle = prefix.to_vec();
                        rectangle.push(vec![(TOKEN_EOF, TOKEN_EOF)]);
                        self.emit_eof_padded(rectangle, out)?;
                    }
                    if symbols.is_empty() {
                        continue;
                    }
                    let mut extended = prefix.to_vec();
                    extended.push(symbol_intervals(&symbols));
                    if extended.len() == self.depth_limit {
                        push_rectangle(out, extended)?;
                    } else {
                        // Token consumed: fresh closure guards, same stack.
                        let rule_count = called_rules.len();
                        self.segment(
                            transition.target(),
                            ctx,
                            &mut BTreeSet::new(),
                            &mut vec![false; rule_count],
                            &extended,
                            out,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn emit_eof_padded(
        &self,
        mut rectangle: LookaheadRectangle,
        out: &mut Vec<LookaheadRectangle>,
    ) -> Result<(), FixedWalkBail> {
        while rectangle.len() < self.depth_limit {
            rectangle.push(vec![(TOKEN_EOF, TOKEN_EOF)]);
        }
        push_rectangle(out, rectangle)
    }
}

fn push_rectangle(
    out: &mut Vec<LookaheadRectangle>,
    rectangle: LookaheadRectangle,
) -> Result<(), FixedWalkBail> {
    if out.len() >= FIXED_LOOKAHEAD_RECTANGLE_BUDGET {
        return Err(FixedWalkBail::Budget);
    }
    out.push(rectangle);
    Ok(())
}

/// Collapses a sorted symbol set into inclusive intervals.
fn symbol_intervals(symbols: &BTreeSet<i32>) -> Vec<(i32, i32)> {
    let mut intervals: Vec<(i32, i32)> = Vec::new();
    for &symbol in symbols {
        match intervals.last_mut() {
            Some((_, stop)) if *stop + 1 == symbol => *stop = symbol,
            _ => intervals.push((symbol, symbol)),
        }
    }
    intervals
}

/// `AnalysisPipeline.disjoint`: pairwise-disjoint alt LOOK sets, with
/// Java's `getDecisionLookahead` nulling (empty or predicate-hitting sets)
/// folded in as an immediate non-LL(1) verdict.
fn decision_alt_looks_disjoint(looks: &[DecisionAltLook]) -> bool {
    let mut combined = BTreeSet::new();
    for look in looks {
        if look.hit_pred || look.symbols.is_empty() {
            return false;
        }
        if look.symbols.intersection(&combined).next().is_some() {
            return false;
        }
        combined.extend(look.symbols.iter().copied());
    }
    true
}

struct DecisionLookWalk<'a> {
    atn: &'a ParserAtn,
    /// Stop state of the rule that owns the decision. Reaching it with an
    /// empty simulated call stack is ANTLR's EPSILON result for this alt.
    owning_rule_stop: Option<usize>,
    /// Java's `lookBusy`: (state, calling context) pairs already expanded.
    busy: BTreeSet<(usize, Vec<usize>)>,
    /// Java's `calledRuleStack`: rules on the walk's invocation path, the
    /// recursion guard that bounds the context stack.
    called_rules: Vec<bool>,
}

impl DecisionLookWalk<'_> {
    /// `LL1Analyzer._LOOK` with an initially empty context: nested rule
    /// invocations return precisely through `ctx`; the decision's own rule
    /// boundary falls through the rule-stop state's return edges — the
    /// deserializer materializes one per call site, which is exactly the
    /// context-free FOLLOW Java walks there.
    fn walk(&mut self, state_number: usize, ctx: &mut Vec<usize>, look: &mut DecisionAltLook) {
        if !self.busy.insert((state_number, ctx.clone())) {
            return;
        }
        if self.owning_rule_stop == Some(state_number) && ctx.is_empty() {
            look.nullable = true;
        }
        let Some(state) = self.atn.state(state_number) else {
            return;
        };
        if state.kind() == AtnStateKind::RuleStop {
            if let Some(return_state) = ctx.pop() {
                let cleared = state
                    .rule_index()
                    .map(|rule| std::mem::replace(&mut self.called_rules[rule], false));
                self.walk(return_state, ctx, look);
                if let (Some(rule), Some(flag)) = (state.rule_index(), cleared) {
                    self.called_rules[rule] = flag;
                }
                ctx.push(return_state);
                return;
            }
            if state.transitions().is_empty() {
                // The walk escaped through a stop state with no call sites —
                // the start rule's end, where the only lookahead is EOF.
                look.symbols.insert(TOKEN_EOF);
                return;
            }
        }
        for transition in state.transitions() {
            match transition.data() {
                ParserTransitionData::Rule {
                    target,
                    rule_index,
                    follow_state,
                    ..
                } => {
                    if self.called_rules.get(rule_index).copied().unwrap_or(true) {
                        continue;
                    }
                    self.called_rules[rule_index] = true;
                    ctx.push(follow_state);
                    self.walk(target, ctx, look);
                    ctx.pop();
                    self.called_rules[rule_index] = false;
                }
                ParserTransitionData::Predicate { .. }
                | ParserTransitionData::Precedence { .. } => {
                    look.hit_pred = true;
                }
                ParserTransitionData::Epsilon { target }
                | ParserTransitionData::Action { target, .. } => {
                    self.walk(target, ctx, look);
                }
                _ => {
                    // Terminal edge: enumerate the vocabulary through the
                    // shared `matches` so atoms, ranges, sets, negations and
                    // wildcards agree with the simulator exactly.
                    for symbol in TOKEN_EOF..=self.atn.max_token_type() {
                        if transition.matches(symbol, 1, self.atn.max_token_type()) {
                            look.symbols.insert(symbol);
                        }
                    }
                }
            }
        }
    }
}
