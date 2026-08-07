#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GeneratedParserRule {
    pub(crate) rule_index: usize,
    pub(crate) entry_state: usize,
    pub(crate) left_recursive: bool,
    pub(crate) steps: Vec<GeneratedParserStep>,
}

/// Parser ATN lowered into generated control-flow steps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoweredParserIr {
    rules: Vec<Option<GeneratedParserRule>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GeneratedParserStep {
    MatchToken {
        token_type: i32,
        follow_state: usize,
    },
    MatchSet {
        token_set: Option<usize>,
        intervals: Vec<(i32, i32)>,
        follow_state: usize,
    },
    MatchNotSet {
        token_set: Option<usize>,
        intervals: Vec<(i32, i32)>,
        follow_state: usize,
    },
    MatchWildcard {
        follow_state: usize,
    },
    Precedence(i32),
    Predicate {
        rule_index: usize,
        pred_index: usize,
    },
    Action {
        source_state: usize,
        rule_index: usize,
        action_index: Option<usize>,
    },
    CallRule {
        source_state: usize,
        rule_index: usize,
        precedence: GeneratedRuleCallPrecedence,
    },
    Decision {
        state: usize,
        decision: usize,
        track_alt_number: bool,
        allow_semantic_context: bool,
        force_context: bool,
        fast_path: Option<GeneratedDecisionFastPath>,
        alts: Vec<Vec<Self>>,
    },
    StarLoop {
        state: usize,
        decision: usize,
        enter_alt: usize,
        exit_alt: usize,
        track_alt_number: bool,
        allow_semantic_context: bool,
        force_context: bool,
        /// `true` for a `+` (one-or-more) loop, `false` for a `*` (zero-or-more)
        /// loop. A `+` loop's mandatory first element is iteration 1, so its first
        /// loop-back sync recovers like ANTLR's `STAR_LOOP_BACK`/`PLUS_LOOP_BACK`
        /// (multi-token `consumeUntil`); a `*` loop's first sync is at the entry,
        /// which recovers like `STAR_LOOP_ENTRY` (single-token deletion).
        plus_loop: bool,
        fast_path: Option<GeneratedDecisionFastPath>,
        body: Vec<Self>,
    },
    LeftRecursiveLoop {
        state: usize,
        decision: usize,
        enter_alt: usize,
        exit_alt: usize,
        rule_index: usize,
        entry_state: usize,
        body: Vec<Self>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GeneratedRuleCallPrecedence {
    Literal(i32),
    InheritLocal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GeneratedDecisionFastPath {
    pub(crate) arms: Vec<GeneratedDecisionFastArm>,
}

/// Complete tool LOOK(1) dispatch for one LL(1) decision.
///
/// `default_alt` is the sole alternative that can reach the owning rule's
/// stop state without consuming input. ANTLR's generated optional/loop code
/// selects that alternative when synchronization observes EPSILON and the
/// current token is outside every explicit LOOK arm.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompleteLl1Dispatch {
    pub(crate) fast_path: GeneratedDecisionFastPath,
    pub(crate) default_alt: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GeneratedDecisionFastArm {
    pub(crate) alt: usize,
    pub(crate) intervals: Vec<(i32, i32)>,
}

#[derive(Clone, Copy)]
pub(crate) struct DecisionRender<'a> {
    pub(crate) state: usize,
    pub(crate) decision: usize,
    pub(crate) track_alt_number: bool,
    pub(crate) allow_semantic_context: bool,
    pub(crate) force_context: bool,
    pub(crate) fast_path: Option<&'a GeneratedDecisionFastPath>,
    pub(crate) alts: &'a [Vec<GeneratedParserStep>],
}

#[derive(Clone, Copy)]
pub(crate) struct StarLoopRender<'a> {
    pub(crate) state: usize,
    pub(crate) decision: usize,
    pub(crate) alts: (usize, usize),
    pub(crate) track_alt_number: bool,
    pub(crate) allow_semantic_context: bool,
    pub(crate) force_context: bool,
    pub(crate) plus_loop: bool,
    pub(crate) fast_path: Option<&'a GeneratedDecisionFastPath>,
    pub(crate) body: &'a [GeneratedParserStep],
}

#[derive(Clone, Copy)]
pub(crate) struct LeftRecursiveLoopRender<'a> {
    pub(crate) state: usize,
    pub(crate) decision: usize,
    pub(crate) alts: (usize, usize),
    pub(crate) rule: (usize, usize),
    pub(crate) body: &'a [GeneratedParserStep],
}

/// Embedded-mode data consulted while rendering rule bodies: verbatim
/// action/predicate expressions plus per-rule `@init` / `@after` bodies.
#[derive(Clone, Copy)]
pub(crate) struct EmbeddedStepRender<'a> {
    /// Keep every decision on the adaptive simulator (no LL(1)/fast-path
    /// shortcuts) regardless of the tool classification.
    pub(crate) force_adaptive: bool,
    /// Tool-classified non-LL(1) decisions ([`tool_decision_analysis`]).
    pub(crate) adaptive_decisions: &'a BTreeSet<usize>,
    /// Complete tool LOOK(1) dispatches for LL(1)-disjoint decisions.
    pub(crate) complete_ll1_dispatches: &'a BTreeMap<usize, CompleteLl1Dispatch>,
    pub(crate) predicates: &'a BTreeMap<(usize, usize), (String, Option<String>)>,
    pub(crate) rule_has_attrs: &'a [bool],
    pub(crate) init_entry: &'a BTreeMap<usize, String>,
    pub(crate) after: &'a BTreeMap<usize, String>,
    pub(crate) call_args: &'a BTreeMap<usize, String>,
    pub(crate) rule_arg0: &'a [Option<String>],
}

impl<'a> EmbeddedStepRender<'a> {
    /// Java routes this decision through `adaptivePredict` — only there are
    /// DFA states learned and full-context diagnostics emitted — so the
    /// generated code must skip its LL(1)/fast-path shortcuts for it.
    pub(crate) fn adaptive_decision(&self, decision: usize) -> bool {
        self.force_adaptive || self.adaptive_decisions.contains(&decision)
    }

    /// Complete dispatch table for a tool-LL(1) decision, exit alternatives
    /// included — Java's switch compilation. Legit input never reaches the
    /// simulator through this, so no DFA state is ever learned for the
    /// decision (matching the dump).
    fn tool_ll1_dispatch(&self, decision: usize) -> Option<&'a CompleteLl1Dispatch> {
        self.complete_ll1_dispatches.get(&decision)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PortableLocalStepRender<'a> {
    pub(crate) declarations: &'a [Vec<String>],
    pub(crate) predicates: &'a BTreeMap<(usize, usize), (String, Option<String>)>,
    pub(crate) required_generated_rules: &'a BTreeSet<usize>,
}

/// Mode-independent decision routing produced by [`classify_decisions`].
///
/// Embedded mode reads its Java-parity LL(1) tables through
/// [`EmbeddedStepRender`]. `complete_ll1_dispatches` carries the same
/// classifier result into plain-mode recovery misses; the other fields carry
/// opt-in `--fixed-lookahead` routing: restricted LL(1) switch tables for
/// plain mode and fixed-LL(k) dispatch tables for both modes.
#[derive(Clone, Copy, Default)]
pub(crate) struct DecisionRoutingRender<'a> {
    /// Complete tool LOOK(1) dispatches used by plain-mode recovery misses.
    /// Embedded mode carries the same data through [`EmbeddedStepRender`].
    pub(crate) complete_ll1_dispatches: Option<&'a BTreeMap<usize, CompleteLl1Dispatch>>,
    pub(crate) ll1_dispatch_tables: Option<&'a BTreeMap<usize, FixedLookaheadTable>>,
    pub(crate) fixed_lookahead_tables: Option<&'a BTreeMap<usize, FixedLookaheadTable>>,
    pub(crate) shared_descent_plans: Option<&'a BTreeMap<usize, Vec<SharedDescentPlan>>>,
}

impl<'a> DecisionRoutingRender<'a> {
    fn complete_ll1_dispatch(self, decision: usize) -> Option<&'a CompleteLl1Dispatch> {
        self.complete_ll1_dispatches
            .and_then(|dispatches| dispatches.get(&decision))
    }

    /// Static dispatch table for a decision: the fixed-LL(k) trie when the
    /// probe proved one, else the depth-1 table from the tool's complete
    /// LOOK(1) arms (plain mode only; embedded mode renders its Java-parity
    /// switch through [`EmbeddedStepRender`]). Both are pre-restricted to
    /// sync-no-op lookahead and render through the same dispatch shape.
    pub(crate) fn static_dispatch_table(self, decision: usize) -> Option<&'a FixedLookaheadTable> {
        self.fixed_lookahead_tables
            .and_then(|tables| tables.get(&decision))
            .or_else(|| {
                self.ll1_dispatch_tables
                    .and_then(|tables| tables.get(&decision))
            })
    }

    pub(crate) fn shared_descent_plans(self, decision: usize) -> &'a [SharedDescentPlan] {
        self.shared_descent_plans
            .and_then(|plans| plans.get(&decision))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(crate) fn shared_descent_resume_call(self, rule_index: usize, source_state: usize) -> bool {
        self.shared_descent_plans.is_some_and(|plans| {
            plans.values().flatten().any(|plan| {
                plan.common_rule == rule_index && plan.resume_call_sites.contains(&source_state)
            })
        })
    }

    pub(crate) fn has_shared_descent_plans(self) -> bool {
        self.shared_descent_plans
            .is_some_and(|plans| plans.values().any(|plans| !plans.is_empty()))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct GeneratedStepRenderContext<'a> {
    pub(crate) current_rule_index: usize,
    /// `Some` in embedded mode: actions/predicates are verbatim Rust.
    pub(crate) embedded: Option<EmbeddedStepRender<'a>>,
    /// Portable raw-grammar boolean locals translated without embedded mode.
    pub(crate) portable_locals: Option<PortableLocalStepRender<'a>>,
    /// Opt-in `--fixed-lookahead` static dispatch routing (both modes).
    pub(crate) decision_routing: DecisionRoutingRender<'a>,
    pub(crate) inline_action_statements: &'a BTreeMap<usize, String>,
    pub(crate) track_alt_numbers: bool,
    pub(crate) track_context_alt_numbers: bool,
    pub(crate) direct_generated_rule_calls: &'a [bool],
    pub(crate) atn_preferred_rule_calls: &'a [bool],
    pub(crate) adaptive_atn_preferred_rule_slots: &'a [Option<usize>],
    pub(crate) adaptive_atn_probe_rule_slots: &'a [Vec<usize>],
}

pub(crate) struct ResolvedDecisionDispatch<'a> {
    pub(crate) complete_ll1_dispatch: Option<&'a CompleteLl1Dispatch>,
    pub(crate) fast_path: Option<&'a GeneratedDecisionFastPath>,
}

pub(crate) fn resolve_decision_dispatch<'a>(
    render_context: GeneratedStepRenderContext<'a>,
    decision: usize,
    fallback_fast_path: Option<&'a GeneratedDecisionFastPath>,
) -> ResolvedDecisionDispatch<'a> {
    let tool_dispatch = render_context
        .embedded
        .and_then(|embedded| embedded.tool_ll1_dispatch(decision));
    ResolvedDecisionDispatch {
        complete_ll1_dispatch: tool_dispatch.or_else(|| {
            render_context
                .decision_routing
                .complete_ll1_dispatch(decision)
        }),
        fast_path: tool_dispatch
            .map(|dispatch| &dispatch.fast_path)
            .or(fallback_fast_path),
    }
}

pub(crate) struct GeneratedParserCompileContext<'a> {
    pub(crate) atn: &'a ParserAtn,
    pub(crate) decision_by_state: &'a [Option<usize>],
    pub(crate) rule_args: &'a [(usize, usize, RuleArgTemplate)],
    pub(crate) inline_action_states: &'a BTreeSet<usize>,
    pub(crate) action_states: &'a BTreeSet<usize>,
    pub(crate) generated_action_states: &'a BTreeSet<usize>,
    pub(crate) action_indices: &'a BTreeMap<usize, usize>,
    pub(crate) predicate_coordinates: &'a BTreeSet<(usize, usize)>,
    pub(crate) generated_predicate_coordinates: &'a BTreeSet<(usize, usize)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TypedHookMapping {
    pub(crate) rule_index: usize,
    pub(crate) coordinate_index: usize,
    pub(crate) kind: ParserTypedHookKind,
    pub(crate) method_name: String,
    pub(crate) call: SemanticHelperCall,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ParserTypedHookKind {
    Predicate,
    Action,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum LexerTypedHookKind {
    Predicate,
    Action,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LexerTypedHookMapping {
    pub(crate) rule_index: usize,
    pub(crate) coordinate_index: usize,
    pub(crate) kind: LexerTypedHookKind,
    pub(crate) method_name: String,
    pub(crate) call: SemanticHelperCall,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParserRenderOptions<'a> {
    pub(crate) require_generated_parser: bool,
    /// Splice verbatim Rust action/predicate bodies from the grammar
    /// (`--actions embedded`).
    pub(crate) embedded: bool,
    pub(crate) generate_listener: bool,
    pub(crate) generate_visitor: bool,
    pub(crate) sem_unknown: SemUnknownPolicy,
    pub(crate) patterns: Option<&'a SemPatternFile>,
    /// `--fixed-lookahead <k>`: compile decisions provable within `k`
    /// tokens of lookahead into static dispatch tables.
    pub(crate) fixed_lookahead: Option<usize>,
    /// `--shared-descent`: parse proven common first-consuming rules once and
    /// resume through the ordinary selected alternative.
    pub(crate) shared_descent: bool,
}

impl Default for ParserRenderOptions<'_> {
    fn default() -> Self {
        Self {
            require_generated_parser: false,
            embedded: false,
            generate_listener: true,
            generate_visitor: false,
            sem_unknown: SemUnknownPolicy::default(),
            patterns: None,
            fixed_lookahead: None,
            shared_descent: false,
        }
    }
}

/// A non-default policy must reach the interpreter through the emitted runtime
/// options, so its literal forces the options-carrying call shape.
///
/// A `hook`-disposed coordinate falls through to the configured policy when its
/// hook is unimplemented. It does not escalate the global policy, because that
/// would flip unrelated `assume-true` coordinates in the same grammar to
/// fail-loud.
pub(crate) const fn parser_unknown_policy_literal(
    policy: SemUnknownPolicy,
) -> Option<&'static str> {
    match policy {
        SemUnknownPolicy::AssumeTrue => None,
        SemUnknownPolicy::AssumeFalse => Some("antlr4_runtime::UnknownSemanticPolicy::AssumeFalse"),
        SemUnknownPolicy::Hook | SemUnknownPolicy::Error => {
            Some("antlr4_runtime::UnknownSemanticPolicy::Error")
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ActionStateSets<'a> {
    pub(crate) all: &'a BTreeSet<usize>,
    pub(crate) generated: &'a BTreeSet<usize>,
    pub(crate) inline: &'a BTreeSet<usize>,
    pub(crate) indices: &'a BTreeMap<usize, usize>,
}

#[derive(Clone, Copy)]
pub(crate) struct PredicateCoordinateSets<'a> {
    pub(crate) all: &'a BTreeSet<(usize, usize)>,
    pub(crate) generated: &'a BTreeSet<(usize, usize)>,
}

const fn generated_action_state_sets<'a>(
    context: &GeneratedParserCompileContext<'a>,
) -> ActionStateSets<'a> {
    ActionStateSets {
        all: context.action_states,
        generated: context.generated_action_states,
        inline: context.inline_action_states,
        indices: context.action_indices,
    }
}

const fn generated_predicate_coordinate_sets<'a>(
    context: &GeneratedParserCompileContext<'a>,
) -> PredicateCoordinateSets<'a> {
    PredicateCoordinateSets {
        all: context.predicate_coordinates,
        generated: context.generated_predicate_coordinates,
    }
}

pub(crate) fn lower_parser_ir(
    data: &ParserCodegenData<'_>,
    enabled_rules: &[bool],
    rule_args: &[(usize, usize, RuleArgTemplate)],
    action_states: ActionStateSets<'_>,
    predicate_coordinates: PredicateCoordinateSets<'_>,
) -> LoweredParserIr {
    let atn = data.parser_atn();
    let decision_by_state = decision_by_state(atn);
    let context = GeneratedParserCompileContext {
        atn,
        decision_by_state: &decision_by_state,
        rule_args,
        inline_action_states: action_states.inline,
        action_states: action_states.all,
        generated_action_states: action_states.generated,
        action_indices: action_states.indices,
        predicate_coordinates: predicate_coordinates.all,
        generated_predicate_coordinates: predicate_coordinates.generated,
    };
    let rules = (0..data.rule_names.len())
        .map(|rule_index| {
            if enabled_rules.get(rule_index).copied().unwrap_or_default() {
                compile_generated_parser_rule(&context, rule_index)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    LoweredParserIr { rules }
}

pub(crate) const ATN_PREFERRED_LEADING_CALL_CHAIN_MIN: usize = 8;
const ATN_PREFERRED_CHAIN_MIN_DECISION_DENSITY_NUMERATOR: usize = 2;
pub(crate) const ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST: usize = 8;
pub(crate) const ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS: usize = 8;
const ATN_PREFERRED_LEFT_RECURSIVE_WRAPPER_MIN_DECISION_COST: usize = 8;
const ATN_PREFERRED_WRAPPER_MIN_DECISION_COST: usize = 2;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct GeneratedRuleShape {
    decision_cost: usize,
    action_or_predicate_count: usize,
    left_recursive_operator_alts: usize,
}

impl AddAssign for GeneratedRuleShape {
    fn add_assign(&mut self, rhs: Self) {
        self.decision_cost += rhs.decision_cost;
        self.action_or_predicate_count += rhs.action_or_predicate_count;
        self.left_recursive_operator_alts = self
            .left_recursive_operator_alts
            .max(rhs.left_recursive_operator_alts);
    }
}

#[cfg(test)]
pub(crate) fn generated_atn_preferred_rule_calls(
    rules: &[Option<GeneratedParserRule>],
    rule_names: &[String],
) -> Vec<bool> {
    generated_atn_preferred_rule_calls_excluding(rules, rule_names, &BTreeSet::new())
}

#[cfg(test)]
pub(crate) fn generated_adaptive_atn_preferred_rule_calls(
    rules: &[Option<GeneratedParserRule>],
) -> Vec<bool> {
    generated_adaptive_atn_preferred_rule_calls_excluding(rules, &BTreeSet::new(), &BTreeSet::new())
}

pub(crate) struct GeneratedAdaptiveAtnRouting {
    pub(crate) candidates: Vec<bool>,
    pub(crate) probe_candidate_rules: Vec<Vec<usize>>,
}

pub(crate) fn generated_atn_preferred_rule_calls_excluding(
    rules: &[Option<GeneratedParserRule>],
    _rule_names: &[String],
    force_generated: &BTreeSet<usize>,
) -> Vec<bool> {
    let leading_rule_calls = rules
        .iter()
        .map(|rule| {
            rule.as_ref()
                .and_then(|rule| generated_steps_leading_mandatory_rule_call(&rule.steps))
        })
        .collect::<Vec<_>>();
    let shapes = generated_rule_shapes(rules);
    let mut preferred = vec![false; rules.len()];

    for start in 0..rules.len() {
        if rules[start].is_none() {
            continue;
        }
        let mut chain = Vec::new();
        let mut seen = vec![false; rules.len()];
        let mut current = start;

        loop {
            if current >= rules.len() || rules[current].is_none() || seen[current] {
                break;
            }
            seen[current] = true;
            chain.push(current);
            let Some(next) = leading_rule_calls[current] else {
                break;
            };
            current = next;
        }

        if chain.len() >= ATN_PREFERRED_LEADING_CALL_CHAIN_MIN
            && generated_atn_preferred_chain_is_expensive(&chain, &shapes)
        {
            for rule_index in chain {
                preferred[rule_index] = true;
            }
        }
    }
    propagate_atn_preferred_wrappers(rules, &shapes, &mut preferred);
    exclude_forced_generated_rules(&mut preferred, force_generated);

    preferred
}

#[cfg(test)]
pub(crate) fn generated_adaptive_atn_preferred_rule_calls_excluding(
    rules: &[Option<GeneratedParserRule>],
    force_generated: &BTreeSet<usize>,
    effectful_action_states: &BTreeSet<usize>,
) -> Vec<bool> {
    generated_adaptive_atn_routing_excluding(rules, force_generated, effectful_action_states)
        .candidates
}

pub(crate) fn generated_adaptive_atn_routing_excluding(
    rules: &[Option<GeneratedParserRule>],
    force_generated: &BTreeSet<usize>,
    effectful_action_states: &BTreeSet<usize>,
) -> GeneratedAdaptiveAtnRouting {
    let shapes = generated_rule_shapes(rules);
    let action_rules = rules
        .iter()
        .flatten()
        .filter(|rule| generated_steps_have_actions(&rule.steps, effectful_action_states))
        .map(|rule| rule.rule_index)
        .collect::<BTreeSet<_>>();
    let action_rule_callers = generated_rule_callers_reaching(rules, &action_rules);
    let seeds = rules
        .iter()
        .flatten()
        .filter(|rule| {
            generated_rule_is_expensive_left_recursive(
                rule,
                shapes.get(rule.rule_index).copied().unwrap_or_default(),
            )
        })
        .map(|rule| rule.rule_index)
        .collect::<Vec<_>>();
    let mut candidates = vec![false; rules.len()];
    let mut probe_candidate_rules = vec![BTreeSet::new(); rules.len()];

    for seed in seeds {
        // Keep the seed eligible when it is entered directly or through a
        // cheaper caller. Calls from an eligible wrapper use the probe mapping
        // below instead, so the wrapper remains the retry boundary there.
        candidates[seed] = true;
        let mut region = vec![false; rules.len()];
        region[seed] = true;
        let wrappers = rules
            .iter()
            .enumerate()
            .filter_map(|(rule_index, rule)| {
                rule.as_ref()
                    .filter(|rule| {
                        generated_rule_is_atn_preferred_wrapper(
                            rule,
                            &shapes,
                            &region,
                            ATN_PREFERRED_LEFT_RECURSIVE_WRAPPER_MIN_DECISION_COST,
                        )
                    })
                    .map(|_| rule_index)
            })
            .collect::<Vec<_>>();
        for wrapper in wrappers {
            candidates[wrapper] = true;
            probe_candidate_rules[seed].insert(wrapper);
        }
    }
    exclude_forced_generated_rules(&mut candidates, force_generated);
    // Adaptive retry rewinds and reparses the candidate through the committed
    // interpreter. Exclude every generated caller that could already have run
    // an action before that retry boundary.
    exclude_forced_generated_rules(&mut candidates, &action_rule_callers);
    let probe_candidate_rules = probe_candidate_rules
        .into_iter()
        .map(|candidates_for_probe| {
            candidates_for_probe
                .into_iter()
                .filter(|candidate| candidates.get(*candidate).copied().unwrap_or_default())
                .collect()
        })
        .collect();

    GeneratedAdaptiveAtnRouting {
        candidates,
        probe_candidate_rules,
    }
}

pub(crate) fn indexed_rule_slots(selected: &[bool]) -> Vec<Option<usize>> {
    let mut next_slot = 0;
    selected
        .iter()
        .map(|selected| {
            if *selected {
                let slot = next_slot;
                next_slot += 1;
                Some(slot)
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn indexed_probe_slots(
    probe_candidate_rules: &[Vec<usize>],
    candidate_slots: &[Option<usize>],
) -> Vec<Vec<usize>> {
    probe_candidate_rules
        .iter()
        .map(|candidate_rules| {
            candidate_rules
                .iter()
                .filter_map(|candidate| candidate_slots.get(*candidate).copied().flatten())
                .collect()
        })
        .collect()
}

fn generated_rule_shapes(rules: &[Option<GeneratedParserRule>]) -> Vec<GeneratedRuleShape> {
    rules
        .iter()
        .map(|rule| {
            rule.as_ref()
                .map_or_else(GeneratedRuleShape::default, generated_rule_shape)
        })
        .collect()
}

fn exclude_forced_generated_rules(preferred: &mut [bool], force_generated: &BTreeSet<usize>) {
    for rule_index in force_generated {
        if let Some(entry) = preferred.get_mut(*rule_index) {
            *entry = false;
        }
    }
}

const fn generated_rule_is_expensive_left_recursive(
    rule: &GeneratedParserRule,
    shape: GeneratedRuleShape,
) -> bool {
    rule.left_recursive
        && shape.decision_cost >= ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST
        && shape.left_recursive_operator_alts >= ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS
}

pub(crate) fn generated_rule_callers_reaching(
    rules: &[Option<GeneratedParserRule>],
    target_rules: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    let mut graph = DiGraph::new();
    let nodes = (0..rules.len())
        .map(|rule_index| graph.add_node(rule_index))
        .collect::<Vec<_>>();
    for rule in rules.iter().flatten() {
        let mut callees = BTreeSet::new();
        collect_generated_step_callees(&rule.steps, &mut callees);
        for callee in callees {
            if let Some(target) = nodes.get(callee) {
                graph.add_edge(nodes[rule.rule_index], *target, ());
            }
        }
    }
    graph_nodes_reaching(&graph, target_rules)
}

pub(crate) fn parser_rule_callers_reaching(
    data: &ParserCodegenData<'_>,
    target_rules: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    if target_rules.is_empty() {
        return BTreeSet::new();
    }
    if let Some(semantic) = data.semantic {
        let mut graph = DiGraph::new();
        let nodes = (0..data.rule_names.len())
            .map(|rule_index| graph.add_node(rule_index))
            .collect::<Vec<_>>();
        for (caller, callees) in &semantic.call_graph {
            let caller = semantic.recognizer.rule_numbers[caller];
            for callee in callees {
                graph.add_edge(
                    nodes[caller],
                    nodes[semantic.recognizer.rule_numbers[callee]],
                    (),
                );
            }
        }
        return graph_nodes_reaching(&graph, target_rules);
    }
    let atn = data.parser_atn();
    atn_rule_callers_reaching(atn, target_rules, data.rule_names.len())
}

pub(crate) fn atn_rule_callers_reaching(
    atn: &ParserAtn,
    target_rules: &BTreeSet<usize>,
    rule_count: usize,
) -> BTreeSet<usize> {
    let mut reaching = target_rules.clone();
    loop {
        let mut changed = false;
        for state in atn.states() {
            let Some(caller_rule) = state.rule_index().filter(|index| *index < rule_count) else {
                continue;
            };
            if reaching.contains(&caller_rule) {
                continue;
            }
            let calls_reaching_rule = state.transitions().iter().any(|transition| {
                matches!(
                    transition.data(),
                    ParserTransitionData::Rule { rule_index, .. }
                        if reaching.contains(&rule_index)
                )
            });
            if calls_reaching_rule {
                changed |= reaching.insert(caller_rule);
            }
        }
        if !changed {
            return reaching;
        }
    }
}

fn collect_generated_step_callees(steps: &[GeneratedParserStep], callees: &mut BTreeSet<usize>) {
    for step in steps {
        match step {
            GeneratedParserStep::CallRule { rule_index, .. } => {
                callees.insert(*rule_index);
            }
            GeneratedParserStep::Decision { alts, .. } => {
                for alternative in alts {
                    collect_generated_step_callees(alternative, callees);
                }
            }
            GeneratedParserStep::StarLoop { body, .. }
            | GeneratedParserStep::LeftRecursiveLoop { body, .. } => {
                collect_generated_step_callees(body, callees);
            }
            GeneratedParserStep::MatchToken { .. }
            | GeneratedParserStep::MatchSet { .. }
            | GeneratedParserStep::MatchNotSet { .. }
            | GeneratedParserStep::MatchWildcard { .. }
            | GeneratedParserStep::Precedence(_)
            | GeneratedParserStep::Predicate { .. }
            | GeneratedParserStep::Action { .. } => {}
        }
    }
}

pub(crate) fn graph_nodes_reaching(
    graph: &DiGraph<usize, ()>,
    targets: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    let reversed = Reversed(graph);
    let mut traversal = Dfs::empty(reversed);
    let mut reaching = targets.clone();
    for target in graph
        .node_indices()
        .filter(|node| targets.contains(&graph[*node]))
    {
        traversal.move_to(target);
        while let Some(node) = traversal.next(reversed) {
            reaching.insert(graph[node]);
        }
    }
    reaching
}

fn generated_atn_preferred_chain_is_expensive(
    chain: &[usize],
    shapes: &[GeneratedRuleShape],
) -> bool {
    let decision_cost = chain
        .iter()
        .filter_map(|rule_index| shapes.get(*rule_index))
        .map(|shape| shape.decision_cost)
        .sum::<usize>();
    decision_cost >= chain.len() * ATN_PREFERRED_CHAIN_MIN_DECISION_DENSITY_NUMERATOR
}

fn propagate_atn_preferred_wrappers(
    rules: &[Option<GeneratedParserRule>],
    shapes: &[GeneratedRuleShape],
    preferred: &mut [bool],
) {
    propagate_atn_preferred_wrappers_with_min_decision_cost(
        rules,
        shapes,
        preferred,
        ATN_PREFERRED_WRAPPER_MIN_DECISION_COST,
    );
}

fn propagate_atn_preferred_wrappers_with_min_decision_cost(
    rules: &[Option<GeneratedParserRule>],
    shapes: &[GeneratedRuleShape],
    preferred: &mut [bool],
    min_decision_cost: usize,
) {
    loop {
        let mut changed = false;
        for (rule_index, rule) in rules.iter().enumerate() {
            if preferred.get(rule_index).copied().unwrap_or_default() {
                continue;
            }
            let Some(rule) = rule else {
                continue;
            };
            if !generated_rule_is_atn_preferred_wrapper(rule, shapes, preferred, min_decision_cost)
            {
                continue;
            }
            preferred[rule_index] = true;
            changed = true;
        }
        if !changed {
            return;
        }
    }
}

fn generated_rule_is_atn_preferred_wrapper(
    rule: &GeneratedParserRule,
    shapes: &[GeneratedRuleShape],
    preferred: &[bool],
    min_decision_cost: usize,
) -> bool {
    if rule.left_recursive {
        return false;
    }
    let shape = shapes.get(rule.rule_index).copied().unwrap_or_default();
    shape.action_or_predicate_count == 0
        && shape.decision_cost >= min_decision_cost
        && generated_steps_call_atn_preferred_rule(&rule.steps, preferred)
}

fn generated_rule_shape(rule: &GeneratedParserRule) -> GeneratedRuleShape {
    generated_steps_shape(&rule.steps)
}

fn generated_steps_shape(steps: &[GeneratedParserStep]) -> GeneratedRuleShape {
    let mut shape = GeneratedRuleShape::default();
    for step in steps {
        shape += generated_step_shape(step);
    }
    shape
}

fn generated_steps_have_actions(
    steps: &[GeneratedParserStep],
    effectful_action_states: &BTreeSet<usize>,
) -> bool {
    steps.iter().any(|step| match step {
        GeneratedParserStep::Action { source_state, .. } => {
            effectful_action_states.contains(source_state)
        }
        GeneratedParserStep::Decision { alts, .. } => alts
            .iter()
            .any(|alt| generated_steps_have_actions(alt, effectful_action_states)),
        GeneratedParserStep::StarLoop { body, .. }
        | GeneratedParserStep::LeftRecursiveLoop { body, .. } => {
            generated_steps_have_actions(body, effectful_action_states)
        }
        GeneratedParserStep::MatchToken { .. }
        | GeneratedParserStep::MatchSet { .. }
        | GeneratedParserStep::MatchNotSet { .. }
        | GeneratedParserStep::MatchWildcard { .. }
        | GeneratedParserStep::Precedence(_)
        | GeneratedParserStep::Predicate { .. }
        | GeneratedParserStep::CallRule { .. } => false,
    })
}

fn generated_step_shape(step: &GeneratedParserStep) -> GeneratedRuleShape {
    match step {
        GeneratedParserStep::Decision {
            allow_semantic_context,
            force_context,
            fast_path,
            alts,
            ..
        } => {
            let mut shape = GeneratedRuleShape {
                decision_cost: usize::from(
                    fast_path.is_none() || *allow_semantic_context || *force_context,
                ),
                action_or_predicate_count: 0,
                left_recursive_operator_alts: 0,
            };
            for alt in alts {
                shape += generated_steps_shape(alt);
            }
            shape
        }
        GeneratedParserStep::StarLoop {
            allow_semantic_context,
            force_context,
            fast_path,
            body,
            ..
        } => {
            let mut shape = GeneratedRuleShape {
                decision_cost: usize::from(
                    fast_path.is_none() || *allow_semantic_context || *force_context,
                ),
                action_or_predicate_count: 0,
                left_recursive_operator_alts: 0,
            };
            shape += generated_steps_shape(body);
            shape
        }
        GeneratedParserStep::LeftRecursiveLoop { body, .. } => {
            let mut shape = GeneratedRuleShape {
                decision_cost: 1,
                action_or_predicate_count: 0,
                left_recursive_operator_alts: generated_steps_direct_max_alt_count(body),
            };
            shape += generated_steps_shape(body);
            shape
        }
        GeneratedParserStep::Predicate { .. } | GeneratedParserStep::Action { .. } => {
            GeneratedRuleShape {
                decision_cost: 0,
                action_or_predicate_count: 1,
                left_recursive_operator_alts: 0,
            }
        }
        GeneratedParserStep::MatchToken { .. }
        | GeneratedParserStep::MatchSet { .. }
        | GeneratedParserStep::MatchNotSet { .. }
        | GeneratedParserStep::MatchWildcard { .. }
        | GeneratedParserStep::Precedence(_)
        | GeneratedParserStep::CallRule { .. } => GeneratedRuleShape::default(),
    }
}

fn generated_steps_direct_max_alt_count(steps: &[GeneratedParserStep]) -> usize {
    steps
        .iter()
        .filter_map(|step| match step {
            GeneratedParserStep::Decision { alts, .. } => Some(alts.len()),
            GeneratedParserStep::MatchToken { .. }
            | GeneratedParserStep::MatchSet { .. }
            | GeneratedParserStep::MatchNotSet { .. }
            | GeneratedParserStep::MatchWildcard { .. }
            | GeneratedParserStep::Precedence(_)
            | GeneratedParserStep::Predicate { .. }
            | GeneratedParserStep::Action { .. }
            | GeneratedParserStep::CallRule { .. }
            | GeneratedParserStep::StarLoop { .. }
            | GeneratedParserStep::LeftRecursiveLoop { .. } => None,
        })
        .max()
        .unwrap_or_default()
}

fn generated_steps_call_atn_preferred_rule(
    steps: &[GeneratedParserStep],
    preferred: &[bool],
) -> bool {
    steps.iter().any(|step| match step {
        GeneratedParserStep::CallRule { rule_index, .. } => {
            preferred.get(*rule_index).copied().unwrap_or_default()
        }
        GeneratedParserStep::Decision { alts, .. } => alts
            .iter()
            .any(|alt| generated_steps_call_atn_preferred_rule(alt, preferred)),
        GeneratedParserStep::StarLoop { body, .. }
        | GeneratedParserStep::LeftRecursiveLoop { body, .. } => {
            generated_steps_call_atn_preferred_rule(body, preferred)
        }
        GeneratedParserStep::MatchToken { .. }
        | GeneratedParserStep::MatchSet { .. }
        | GeneratedParserStep::MatchNotSet { .. }
        | GeneratedParserStep::MatchWildcard { .. }
        | GeneratedParserStep::Precedence(_)
        | GeneratedParserStep::Predicate { .. }
        | GeneratedParserStep::Action { .. } => false,
    })
}

fn generated_steps_leading_mandatory_rule_call(steps: &[GeneratedParserStep]) -> Option<usize> {
    for step in steps {
        match step {
            GeneratedParserStep::CallRule { rule_index, .. } => return Some(*rule_index),
            GeneratedParserStep::Decision { alts, .. } if generated_alts_are_nullable(alts) => {}
            GeneratedParserStep::Decision { alts, .. } => {
                return generated_alts_common_leading_mandatory_rule_call(alts);
            }
            GeneratedParserStep::StarLoop { .. }
            | GeneratedParserStep::LeftRecursiveLoop { .. }
            | GeneratedParserStep::Precedence(_)
            | GeneratedParserStep::Predicate { .. }
            | GeneratedParserStep::Action { .. } => {}
            GeneratedParserStep::MatchToken { .. }
            | GeneratedParserStep::MatchSet { .. }
            | GeneratedParserStep::MatchNotSet { .. }
            | GeneratedParserStep::MatchWildcard { .. } => return None,
        }
    }
    None
}

fn generated_alts_common_leading_mandatory_rule_call(
    alts: &[Vec<GeneratedParserStep>],
) -> Option<usize> {
    let mut common = None;
    for alt in alts {
        let rule_index = generated_steps_leading_mandatory_rule_call(alt)?;
        match common {
            Some(common_rule_index) if common_rule_index != rule_index => return None,
            Some(_) => {}
            None => common = Some(rule_index),
        }
    }
    common
}

fn generated_alts_are_nullable(alts: &[Vec<GeneratedParserStep>]) -> bool {
    alts.iter().any(|alt| generated_steps_are_nullable(alt))
}

fn generated_steps_are_nullable(steps: &[GeneratedParserStep]) -> bool {
    steps.iter().all(generated_step_is_nullable)
}

fn generated_step_is_nullable(step: &GeneratedParserStep) -> bool {
    match step {
        GeneratedParserStep::Precedence(_)
        | GeneratedParserStep::Predicate { .. }
        | GeneratedParserStep::Action { .. }
        | GeneratedParserStep::StarLoop { .. }
        | GeneratedParserStep::LeftRecursiveLoop { .. } => true,
        GeneratedParserStep::Decision { alts, .. } => generated_alts_are_nullable(alts),
        GeneratedParserStep::MatchToken { .. }
        | GeneratedParserStep::MatchSet { .. }
        | GeneratedParserStep::MatchNotSet { .. }
        | GeneratedParserStep::MatchWildcard { .. }
        | GeneratedParserStep::CallRule { .. } => false,
    }
}

pub(crate) fn require_all_parser_rules_generated(
    rules: &[Option<GeneratedParserRule>],
    data: &RecognizerCodegenData<'_>,
) -> io::Result<()> {
    let missing = rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule.is_none())
        .map(|(index, _)| {
            data.rule_names
                .get(index)
                .map_or_else(|| index.to_string(), Clone::clone)
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "generated parser did not emit {} rule(s): {}",
            missing.len(),
            missing.join(", ")
        ),
    ))
}

pub(crate) fn require_portable_local_rules_generated(
    rules: &[Option<GeneratedParserRule>],
    required: &BTreeSet<usize>,
    data: &RecognizerCodegenData<'_>,
) -> io::Result<()> {
    let missing = required
        .iter()
        .filter(|rule_index| rules.get(**rule_index).is_none_or(Option::is_none))
        .map(|rule_index| {
            data.rule_names
                .get(*rule_index)
                .map_or_else(|| rule_index.to_string(), Clone::clone)
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "portable local semantics require {} generated parser rule(s): {}",
            missing.len(),
            missing.join(", ")
        ),
    ))
}

fn generated_steps_call_disabled_rule(steps: &[GeneratedParserStep], enabled: &[bool]) -> bool {
    steps.iter().any(|step| match step {
        GeneratedParserStep::CallRule { rule_index, .. } => {
            !enabled.get(*rule_index).copied().unwrap_or_default()
        }
        GeneratedParserStep::Decision { alts, .. } => alts
            .iter()
            .any(|alt| generated_steps_call_disabled_rule(alt, enabled)),
        GeneratedParserStep::StarLoop { body, .. }
        | GeneratedParserStep::LeftRecursiveLoop { body, .. } => {
            generated_steps_call_disabled_rule(body, enabled)
        }
        GeneratedParserStep::MatchToken { .. }
        | GeneratedParserStep::MatchSet { .. }
        | GeneratedParserStep::MatchNotSet { .. }
        | GeneratedParserStep::MatchWildcard { .. }
        | GeneratedParserStep::Precedence(_)
        | GeneratedParserStep::Predicate { .. }
        | GeneratedParserStep::Action { .. } => false,
    })
}

pub(crate) fn decision_by_state(atn: &ParserAtn) -> Vec<Option<usize>> {
    let mut decision_by_state = vec![None; atn.state_count()];
    for (decision, state_number) in atn.decision_to_state().iter().enumerate() {
        if let Some(slot) = decision_by_state.get_mut(state_number) {
            *slot = Some(decision);
        }
    }
    decision_by_state
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GeneratedLookSet {
    pub(crate) symbols: BTreeSet<i32>,
    pub(crate) nullable: bool,
}

#[derive(Default)]
pub(crate) struct GeneratedFirstSetCtx {
    cache: BTreeMap<(usize, usize), GeneratedLookSet>,
    in_progress: BTreeSet<(usize, usize)>,
    hit_cycle: bool,
}

fn generated_decision_fast_path<'a>(
    context: &GeneratedParserCompileContext<'_>,
    state: ParserAtnState<'_>,
    alts: impl IntoIterator<Item = (usize, &'a [GeneratedParserStep])>,
) -> Option<GeneratedDecisionFastPath> {
    if state.precedence_rule_decision() || state.non_greedy() {
        return None;
    }
    let mut first_ctx = GeneratedFirstSetCtx::default();
    let mut symbol_alts = BTreeMap::<i32, Option<usize>>::new();
    for (alt, steps) in alts {
        let look = generated_steps_first_set(context.atn, steps, &mut first_ctx);
        if look.nullable {
            return None;
        }
        for symbol in look.symbols {
            match symbol_alts.get(&symbol).copied().flatten() {
                None if symbol_alts.contains_key(&symbol) => {}
                None => {
                    symbol_alts.insert(symbol, Some(alt));
                }
                Some(existing) if existing == alt => {}
                Some(_) => {
                    symbol_alts.insert(symbol, None);
                }
            }
        }
    }

    let mut symbols_by_alt = BTreeMap::<usize, BTreeSet<i32>>::new();
    for (symbol, alt) in symbol_alts {
        if let Some(alt) = alt {
            symbols_by_alt.entry(alt).or_default().insert(symbol);
        }
    }
    let arms = symbols_by_alt
        .into_iter()
        .map(|(alt, symbols)| GeneratedDecisionFastArm {
            alt,
            intervals: symbols_to_ranges(symbols),
        })
        .filter(|arm| !arm.intervals.is_empty())
        .collect::<Vec<_>>();
    (!arms.is_empty()).then_some(GeneratedDecisionFastPath { arms })
}

fn generated_steps_first_set(
    atn: &ParserAtn,
    steps: &[GeneratedParserStep],
    ctx: &mut GeneratedFirstSetCtx,
) -> GeneratedLookSet {
    let mut first = GeneratedLookSet::default();
    for step in steps {
        match step {
            GeneratedParserStep::MatchToken { token_type, .. } => {
                first.symbols.insert(*token_type);
                first.nullable = false;
                return first;
            }
            GeneratedParserStep::MatchSet { intervals, .. } => {
                for (start, stop) in intervals {
                    first.symbols.extend(*start..=*stop);
                }
                first.nullable = false;
                return first;
            }
            GeneratedParserStep::MatchNotSet { intervals, .. } => {
                first.symbols.extend(1..=atn.max_token_type());
                for (start, stop) in intervals {
                    for symbol in *start..=*stop {
                        first.symbols.remove(&symbol);
                    }
                }
                first.nullable = false;
                return first;
            }
            GeneratedParserStep::MatchWildcard { .. } => {
                first.symbols.extend(1..=atn.max_token_type());
                first.nullable = false;
                return first;
            }
            GeneratedParserStep::CallRule { rule_index, .. } => {
                let Some(start) = atn.rule_to_start_state().get(*rule_index) else {
                    return GeneratedLookSet::default();
                };
                let Some(stop) = atn.rule_to_stop_state().get(*rule_index) else {
                    return GeneratedLookSet::default();
                };
                let child = generated_rule_first_set(atn, start, stop, ctx);
                first.symbols.extend(child.symbols);
                if !child.nullable {
                    first.nullable = false;
                    return first;
                }
            }
            GeneratedParserStep::Decision { alts, .. } => {
                let nested = generated_alt_steps_first_set(atn, alts, ctx);
                first.symbols.extend(nested.symbols);
                if !nested.nullable {
                    first.nullable = false;
                    return first;
                }
            }
            GeneratedParserStep::StarLoop { body, .. }
            | GeneratedParserStep::LeftRecursiveLoop { body, .. } => {
                let nested = generated_steps_first_set(atn, body, ctx);
                first.symbols.extend(nested.symbols);
            }
            GeneratedParserStep::Precedence(_)
            | GeneratedParserStep::Predicate { .. }
            | GeneratedParserStep::Action { .. } => {}
        }
    }
    first.nullable = true;
    first
}

fn generated_alt_steps_first_set(
    atn: &ParserAtn,
    alts: &[Vec<GeneratedParserStep>],
    ctx: &mut GeneratedFirstSetCtx,
) -> GeneratedLookSet {
    let mut first = GeneratedLookSet::default();
    for alt in alts {
        let alt_first = generated_steps_first_set(atn, alt, ctx);
        first.symbols.extend(alt_first.symbols);
        first.nullable |= alt_first.nullable;
    }
    first
}

pub(crate) fn generated_rule_first_set(
    atn: &ParserAtn,
    state_number: usize,
    rule_stop_state: usize,
    ctx: &mut GeneratedFirstSetCtx,
) -> GeneratedLookSet {
    let key = (state_number, rule_stop_state);
    if let Some(cached) = ctx.cache.get(&key) {
        return cached.clone();
    }
    if !ctx.in_progress.insert(key) {
        return GeneratedLookSet::default();
    }
    let saved_hit_cycle = ctx.hit_cycle;
    ctx.hit_cycle = false;
    let mut first = GeneratedLookSet::default();
    generated_rule_first_set_inner(
        atn,
        state_number,
        rule_stop_state,
        ctx,
        &mut BTreeSet::new(),
        &mut first,
    );
    ctx.in_progress.remove(&key);
    if !ctx.hit_cycle {
        ctx.cache.insert(key, first.clone());
    }
    ctx.hit_cycle = saved_hit_cycle || ctx.hit_cycle;
    first
}

fn generated_rule_first_set_inner(
    atn: &ParserAtn,
    state_number: usize,
    rule_stop_state: usize,
    ctx: &mut GeneratedFirstSetCtx,
    visited: &mut BTreeSet<usize>,
    first: &mut GeneratedLookSet,
) {
    if !visited.insert(state_number) {
        return;
    }
    if state_number == rule_stop_state {
        first.nullable = true;
        return;
    }
    let Some(state) = atn.state(state_number) else {
        return;
    };
    for transition in state.transitions() {
        let symbols = generated_transition_symbols(transition, atn.max_token_type());
        if !symbols.is_empty() {
            first.symbols.extend(symbols);
            continue;
        }
        match transition.data() {
            ParserTransitionData::Epsilon { target }
            | ParserTransitionData::Action { target, .. }
            | ParserTransitionData::Predicate { target, .. }
            | ParserTransitionData::Precedence { target, .. } => {
                generated_rule_first_set_inner(atn, target, rule_stop_state, ctx, visited, first);
            }
            ParserTransitionData::Rule {
                target,
                rule_index,
                follow_state,
                ..
            } => {
                let Some(child_stop) = atn.rule_to_stop_state().get(rule_index) else {
                    continue;
                };
                let child_key = (target, child_stop);
                if ctx.in_progress.contains(&child_key) && !ctx.cache.contains_key(&child_key) {
                    ctx.hit_cycle = true;
                }
                let child = generated_rule_first_set(atn, target, child_stop, ctx);
                first.symbols.extend(child.symbols);
                if child.nullable {
                    generated_rule_first_set_inner(
                        atn,
                        follow_state,
                        rule_stop_state,
                        ctx,
                        visited,
                        first,
                    );
                }
            }
            ParserTransitionData::Atom { .. }
            | ParserTransitionData::Range { .. }
            | ParserTransitionData::Set { .. }
            | ParserTransitionData::NotSet { .. }
            | ParserTransitionData::Wildcard { .. } => {}
        }
    }
}

pub(crate) fn generated_transition_symbols(
    transition: ParserTransition<'_>,
    max_token_type: i32,
) -> BTreeSet<i32> {
    let mut symbols = BTreeSet::new();
    match transition.data() {
        ParserTransitionData::Atom { label, .. } => {
            symbols.insert(label);
        }
        ParserTransitionData::Range { start, stop, .. } => {
            symbols.extend(start..=stop);
        }
        ParserTransitionData::Set { set, .. } => {
            for (start, stop) in set.ranges() {
                symbols.extend(start..=stop);
            }
        }
        ParserTransitionData::NotSet { set, .. } => {
            symbols.extend((1..=max_token_type).filter(|symbol| !set.contains(*symbol)));
        }
        ParserTransitionData::Wildcard { .. } => {
            symbols.extend(1..=max_token_type);
        }
        ParserTransitionData::Epsilon { .. }
        | ParserTransitionData::Rule { .. }
        | ParserTransitionData::Predicate { .. }
        | ParserTransitionData::Action { .. }
        | ParserTransitionData::Precedence { .. } => {}
    }
    symbols
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

fn state_tracks_alt_number(state: ParserAtnState<'_>) -> bool {
    matches!(
        state.kind(),
        AtnStateKind::Basic
            | AtnStateKind::BlockStart
            | AtnStateKind::PlusBlockStart
            | AtnStateKind::StarBlockStart
            | AtnStateKind::StarLoopEntry
    ) && !state.precedence_rule_decision()
        && state.transitions().len() > 1
}
