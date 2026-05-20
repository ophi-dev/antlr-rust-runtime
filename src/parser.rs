use std::collections::{BTreeMap, BTreeSet};

use crate::atn::{Atn, AtnState, AtnStateKind, Transition};
use crate::errors::AntlrError;
use crate::int_stream::IntStream;
use crate::recognizer::{Recognizer, RecognizerData};
use crate::token::{CommonToken, TOKEN_EOF, Token, TokenSource, TokenSourceError};
use crate::token_stream::CommonTokenStream;
use crate::tree::{ErrorNode, ParseTree, ParserRuleContext, RuleNode, TerminalNode};
use crate::vocabulary::Vocabulary;

/// Upper bound for the recursive metadata recognizer before it treats a path as
/// non-viable. Long expression-regression descriptors legitimately walk tens
/// of thousands of ATN edges.
const RECOGNITION_DEPTH_LIMIT: usize = 100_000;

/// Parser semantic action reached while recognizing one ATN path.
///
/// Generated parsers use `source_state` to dispatch back to the grammar action
/// rendered for that ATN action transition. The token interval is the current
/// rule's input span at the action site, which covers common target templates
/// such as `$text`. Rule-init actions do not have an ATN action source state,
/// so they are marked separately and may carry an ATN state for expected-token
/// rendering.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParserAction {
    source_state: usize,
    rule_index: usize,
    start_index: usize,
    stop_index: Option<usize>,
    rule_init: bool,
    expected_state: Option<usize>,
}

impl ParserAction {
    /// Creates an action event for a recognized parser path.
    pub const fn new(
        source_state: usize,
        rule_index: usize,
        start_index: usize,
        stop_index: Option<usize>,
    ) -> Self {
        Self {
            source_state,
            rule_index,
            start_index,
            stop_index,
            rule_init: false,
            expected_state: None,
        }
    }

    /// Creates an action event for a rule-level `@init` action.
    pub const fn new_rule_init(
        rule_index: usize,
        start_index: usize,
        expected_state: Option<usize>,
    ) -> Self {
        Self {
            source_state: usize::MAX,
            rule_index,
            start_index,
            stop_index: None,
            rule_init: true,
            expected_state,
        }
    }

    /// ATN state that owns the semantic-action transition.
    pub const fn source_state(&self) -> usize {
        self.source_state
    }

    /// Grammar rule index recorded by the serialized ATN action transition.
    pub const fn rule_index(&self) -> usize {
        self.rule_index
    }

    /// Token-stream index where the active rule began.
    pub const fn start_index(&self) -> usize {
        self.start_index
    }

    /// Last token-stream index consumed before the action was reached.
    pub const fn stop_index(&self) -> Option<usize> {
        self.stop_index
    }

    /// Reports whether this event represents a rule-level `@init` action.
    pub const fn is_rule_init(&self) -> bool {
        self.rule_init
    }

    /// ATN state used to compute expected-token display for this action.
    pub const fn expected_state(&self) -> Option<usize> {
        self.expected_state
    }
}

/// Parser semantic predicate rendered from a supported target template.
///
/// The metadata recognizer evaluates these at the token-stream index where the
/// predicate transition is reached. Unsupported or absent predicate templates
/// remain unconditional so existing generated parsers keep their previous
/// behavior unless the generator opts into this table.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ParserPredicate {
    True,
    False,
    /// Target-template test helper that reports predicate evaluation before
    /// returning the wrapped boolean value.
    Invoke {
        value: bool,
    },
    LookaheadTextEquals {
        offset: isize,
        text: &'static str,
    },
    LookaheadNotEquals {
        offset: isize,
        token_type: i32,
    },
    /// Compares the current rule invocation's integer argument with a literal
    /// value from a supported `ValEquals("$i", "...")` target template.
    LocalIntEquals {
        value: i64,
    },
    /// Compares a generated parser integer member modulo a literal value.
    MemberModuloEquals {
        member: usize,
        modulus: i64,
        value: i64,
        equals: bool,
    },
}

/// Prediction strategy requested by generated parser harnesses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PredictionMode {
    /// Prefer the clean full-context outcome when alternatives reach the same
    /// input position.
    Ll,
    /// Preserve SLL's first-viable alternative bias at a decision, even when a
    /// later full-context alternative could avoid recovery.
    Sll,
}

/// Integer argument metadata for a generated parser rule invocation.
///
/// ANTLR's serialized ATN does not retain Rust-target rule argument values, so
/// the generator records the rule-transition source state and the value that
/// should be visible to semantic predicates inside the callee.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParserRuleArg {
    /// ATN state containing the rule transition that receives this argument.
    pub source_state: usize,
    /// Callee rule index for the transition.
    pub rule_index: usize,
    /// Literal fallback value to expose in the callee.
    pub value: i64,
    /// Whether the callee should inherit the caller's current integer argument.
    pub inherit_local: bool,
}

/// Integer member mutation attached to an ATN action transition.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParserMemberAction {
    /// ATN state containing the action transition.
    pub source_state: usize,
    /// Generator-assigned integer member id.
    pub member: usize,
    /// Delta applied when the action is reached on one speculative path.
    pub delta: i64,
}

/// Integer return-value assignment attached to an ATN action transition.
///
/// Generated parsers use this metadata when target actions assign a simple
/// return field such as `$y=1000;`. The interpreter applies it while selecting
/// the recognized path so the finished parse tree can answer later
/// `$label.y` action templates.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParserReturnAction {
    /// ATN state containing the action transition.
    pub source_state: usize,
    /// Rule index recorded by the serialized action transition.
    pub rule_index: usize,
    /// Return-field name as it appears in the grammar.
    pub name: &'static str,
    /// Literal integer value assigned by the action.
    pub value: i64,
}

/// Optional generated-runtime metadata for metadata-driven parser execution.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParserRuntimeOptions<'a> {
    /// Rule indexes whose `@init` actions should be replayed.
    pub init_action_rules: &'a [usize],
    /// Whether generated parse-tree contexts should retain alternative numbers.
    pub track_alt_numbers: bool,
    /// Semantic predicate table keyed by serialized `(rule_index, pred_index)`.
    pub predicates: &'a [(usize, usize, ParserPredicate)],
    /// Rule-call integer argument table keyed by ATN source state.
    pub rule_args: &'a [ParserRuleArg],
    /// Integer member mutations keyed by ATN action source state.
    pub member_actions: &'a [ParserMemberAction],
    /// Integer return assignments keyed by ATN action source state.
    pub return_actions: &'a [ParserReturnAction],
}

pub trait Parser: Recognizer {
    /// Reports whether generated parser rules should build parse-tree nodes
    /// while recognizing input.
    fn build_parse_trees(&self) -> bool;

    /// Enables or disables parse-tree construction for subsequent rule calls.
    fn set_build_parse_trees(&mut self, build: bool);

    /// Reports whether prediction diagnostic-listener messages are emitted
    /// during parser ATN recognition.
    fn report_diagnostic_errors(&self) -> bool {
        false
    }

    /// Enables or disables ANTLR-style prediction diagnostics for subsequent
    /// rule calls.
    fn set_report_diagnostic_errors(&mut self, _report: bool) {}

    /// Reports the prediction strategy used when selecting among alternatives.
    fn prediction_mode(&self) -> PredictionMode {
        PredictionMode::Ll
    }

    /// Sets the prediction strategy for subsequent rule calls.
    fn set_prediction_mode(&mut self, _mode: PredictionMode) {}
}

#[derive(Debug)]
pub struct BaseParser<S> {
    input: CommonTokenStream<S>,
    data: RecognizerData,
    build_parse_trees: bool,
    report_diagnostic_errors: bool,
    prediction_mode: PredictionMode,
    prediction_diagnostics: Vec<ParserDiagnostic>,
    reported_prediction_diagnostics: BTreeSet<(usize, usize, String)>,
    int_members: BTreeMap<usize, i64>,
    /// Predicate side effects are observable in a few target-template tests;
    /// speculative recognition may revisit the same coordinate, so replay it
    /// once per parser instance.
    invoked_predicates: Vec<(usize, usize)>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecognizeOutcome {
    index: usize,
    consumed_eof: bool,
    alt_number: usize,
    member_values: BTreeMap<usize, i64>,
    return_values: BTreeMap<String, i64>,
    diagnostics: Vec<ParserDiagnostic>,
    decisions: Vec<usize>,
    actions: Vec<ParserAction>,
    nodes: Vec<RecognizedNode>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RecognizedNode {
    Token {
        index: usize,
    },
    ErrorToken {
        index: usize,
    },
    MissingToken {
        token_type: i32,
        at_index: usize,
        text: String,
    },
    Rule {
        rule_index: usize,
        invoking_state: isize,
        alt_number: usize,
        start_index: usize,
        stop_index: Option<usize>,
        return_values: BTreeMap<String, i64>,
        children: Vec<Self>,
    },
    LeftRecursiveBoundary {
        rule_index: usize,
    },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FastRecognizeOutcome {
    index: usize,
    consumed_eof: bool,
    diagnostics: Vec<ParserDiagnostic>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ParserDiagnostic {
    line: usize,
    column: usize,
    message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ExpectedTokens {
    index: Option<usize>,
    symbols: BTreeSet<i32>,
    no_viable: Option<NoViableAlternative>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NoViableAlternative {
    start_index: usize,
    error_index: usize,
}

impl ExpectedTokens {
    /// Records the expected symbols for the farthest token index reached by any
    /// failed ATN path.
    fn record_transition(&mut self, index: usize, transition: &Transition, max_token_type: i32) {
        let symbols = transition_expected_symbols(transition, max_token_type);
        match self.index {
            Some(current) if index < current => {}
            Some(current) if index == current => self.symbols.extend(symbols),
            _ => {
                self.index = Some(index);
                self.symbols = symbols;
            }
        }
    }

    /// Records an ambiguous decision that failed after consuming a shared
    /// prefix, which ANTLR reports as `no viable alternative`.
    const fn record_no_viable(&mut self, start_index: usize, error_index: usize) {
        match self.no_viable {
            Some(current) if error_index < current.error_index => {}
            _ => {
                self.no_viable = Some(NoViableAlternative {
                    start_index,
                    error_index,
                });
            }
        }
    }
}

/// Converts one consuming transition into the token types that would satisfy it
/// for diagnostic reporting.
fn transition_expected_symbols(transition: &Transition, max_token_type: i32) -> BTreeSet<i32> {
    let mut symbols = BTreeSet::new();
    match transition {
        Transition::Atom { label, .. } => {
            symbols.insert(*label);
        }
        Transition::Range { start, stop, .. } => {
            symbols.extend(*start..=*stop);
        }
        Transition::Set { set, .. } => {
            for (start, stop) in set.ranges() {
                symbols.extend(*start..=*stop);
            }
        }
        Transition::NotSet { set, .. } => {
            symbols.extend((1..=max_token_type).filter(|symbol| !set.contains(*symbol)));
        }
        Transition::Wildcard { .. } => {
            symbols.extend(1..=max_token_type);
        }
        Transition::Epsilon { .. }
        | Transition::Rule { .. }
        | Transition::Predicate { .. }
        | Transition::Action { .. }
        | Transition::Precedence { .. } => {}
    }
    symbols
}

/// Returns the consuming-token expectations reachable from an ATN state through
/// epsilon transitions. Recovery diagnostics need this closure so alternatives
/// and loop exits report the same expectation set ANTLR users see.
fn state_expected_symbols(atn: &Atn, state_number: usize) -> BTreeSet<i32> {
    let mut symbols = BTreeSet::new();
    let mut stack = vec![state_number];
    let mut visited = BTreeSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current) {
            continue;
        }
        let Some(state) = atn.state(current) else {
            continue;
        };
        for transition in &state.transitions {
            let transition_symbols = transition_expected_symbols(transition, atn.max_token_type());
            if transition_symbols.is_empty() {
                if transition.is_epsilon() {
                    stack.push(transition.target());
                }
            } else {
                symbols.extend(transition_symbols);
            }
        }
    }
    symbols
}

/// Returns token types that can resume parsing from `state_number` after a
/// failed child rule, following rule calls as well as epsilon transitions.
fn state_sync_symbols(atn: &Atn, state_number: usize, stop_state: usize) -> BTreeSet<i32> {
    let mut symbols = BTreeSet::new();
    state_sync_symbols_inner(
        atn,
        state_number,
        stop_state,
        &mut BTreeSet::new(),
        &mut symbols,
    );
    symbols
}

/// Walks epsilon-like continuations from a parent follow state until it finds
/// consuming tokens that can anchor recovery, or EOF if the parent rule can end.
fn state_sync_symbols_inner(
    atn: &Atn,
    state_number: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
    symbols: &mut BTreeSet<i32>,
) {
    if !visited.insert(state_number) {
        return;
    }
    if state_number == stop_state {
        symbols.insert(TOKEN_EOF);
        return;
    }
    let Some(state) = atn.state(state_number) else {
        return;
    };
    for transition in &state.transitions {
        let transition_symbols = transition_expected_symbols(transition, atn.max_token_type());
        if transition_symbols.is_empty() {
            match transition {
                Transition::Rule { target, .. }
                | Transition::Epsilon { target }
                | Transition::Action { target, .. }
                | Transition::Predicate { target, .. }
                | Transition::Precedence { target, .. } => {
                    state_sync_symbols_inner(atn, *target, stop_state, visited, symbols);
                }
                Transition::Atom { .. }
                | Transition::Range { .. }
                | Transition::Set { .. }
                | Transition::NotSet { .. }
                | Transition::Wildcard { .. } => {}
            }
        } else {
            symbols.extend(transition_symbols);
        }
    }
}

/// Carries recovery expectations and their restart state through epsilon-only
/// paths. ANTLR can report and repair at the decision state even when the
/// failed consuming transition is nested under block or loop epsilon edges.
fn next_recovery_context(
    atn: &Atn,
    state: &AtnState,
    inherited: &BTreeSet<i32>,
    inherited_state: Option<usize>,
) -> (BTreeSet<i32>, Option<usize>) {
    let state_symbols = state_expected_symbols(atn, state.state_number);
    if state.transitions.len() > 1 && !state_symbols.is_empty() {
        let mut symbols = state_symbols;
        symbols.extend(inherited.iter().copied());
        return (symbols, Some(state.state_number));
    }
    (inherited.clone(), inherited_state)
}

fn recovery_expected_symbols(
    atn: &Atn,
    state_number: usize,
    inherited: &BTreeSet<i32>,
) -> BTreeSet<i32> {
    let mut symbols = state_expected_symbols(atn, state_number);
    symbols.extend(inherited.iter().copied());
    symbols
}

/// Applies generated integer-member side effects to one speculative path.
fn apply_member_actions(
    source_state: usize,
    actions: &[ParserMemberAction],
    values: &mut BTreeMap<usize, i64>,
) {
    for action in actions
        .iter()
        .filter(|action| action.source_state == source_state)
    {
        *values.entry(action.member).or_default() += action.delta;
    }
}

/// Returns the speculative member state after replaying one ATN action state.
fn member_values_after_action(
    source_state: usize,
    actions: &[ParserMemberAction],
    values: &BTreeMap<usize, i64>,
) -> BTreeMap<usize, i64> {
    let mut values = values.clone();
    apply_member_actions(source_state, actions, &mut values);
    values
}

/// Returns the speculative rule-return state after replaying one ATN action.
fn return_values_after_action(
    source_state: usize,
    rule_index: usize,
    actions: &[ParserReturnAction],
    values: &BTreeMap<String, i64>,
) -> BTreeMap<String, i64> {
    let mut values = values.clone();
    for action in actions
        .iter()
        .filter(|action| action.source_state == source_state && action.rule_index == rule_index)
    {
        values.insert(action.name.to_owned(), action.value);
    }
    values
}

/// Resolves the integer argument visible to a child rule invocation.
fn rule_local_int_arg(
    rule_args: &[ParserRuleArg],
    source_state: usize,
    rule_index: usize,
    local_int_arg: Option<(usize, i64)>,
) -> Option<(usize, i64)> {
    rule_args
        .iter()
        .find(|arg| arg.source_state == source_state && arg.rule_index == rule_index)
        .map(|arg| {
            let value = if arg.inherit_local {
                local_int_arg.map_or(arg.value, |(_, value)| value)
            } else {
                arg.value
            };
            (rule_index, value)
        })
}

/// Builds the terminal recognition outcome for a path that reached its stop
/// state.
fn stop_outcome(
    index: usize,
    rule_alt_number: usize,
    member_values: BTreeMap<usize, i64>,
    return_values: BTreeMap<String, i64>,
) -> Vec<RecognizeOutcome> {
    vec![RecognizeOutcome {
        index,
        consumed_eof: false,
        alt_number: rule_alt_number,
        member_values,
        return_values,
        diagnostics: Vec::new(),
        decisions: Vec::new(),
        actions: Vec::new(),
        nodes: Vec::new(),
    }]
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecognizeRequest<'a> {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    decision_start_index: Option<usize>,
    init_action_rules: &'a BTreeSet<usize>,
    predicates: &'a [(usize, usize, ParserPredicate)],
    rule_args: &'a [ParserRuleArg],
    member_actions: &'a [ParserMemberAction],
    return_actions: &'a [ParserReturnAction],
    local_int_arg: Option<(usize, i64)>,
    member_values: BTreeMap<usize, i64>,
    return_values: BTreeMap<String, i64>,
    rule_alt_number: usize,
    track_alt_numbers: bool,
    /// Current left-recursive precedence threshold, matching ANTLR's
    /// `precpred(_ctx, k)` check for generated precedence rules.
    precedence: i32,
    depth: usize,
    recovery_symbols: BTreeSet<i32>,
    recovery_state: Option<usize>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecognizeKey {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    decision_start_index: Option<usize>,
    local_int_arg: Option<(usize, i64)>,
    member_values: BTreeMap<usize, i64>,
    return_values: BTreeMap<String, i64>,
    rule_alt_number: usize,
    track_alt_numbers: bool,
    precedence: i32,
    recovery_symbols: BTreeSet<i32>,
    recovery_state: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EpsilonActionStep {
    source_state: usize,
    target: usize,
    action_rule_index: Option<usize>,
    left_recursive_boundary: Option<usize>,
    decision: Option<usize>,
    decision_start_index: Option<usize>,
    alt_number: usize,
    recovery_symbols: BTreeSet<i32>,
    recovery_state: Option<usize>,
}

struct RecognizeScratch<'a> {
    visiting: &'a mut BTreeSet<RecognizeKey>,
    memo: &'a mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
    expected: &'a mut ExpectedTokens,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FastRecognizeRequest {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    decision_start_index: Option<usize>,
    precedence: i32,
    depth: usize,
    recovery_symbols: BTreeSet<i32>,
    recovery_state: Option<usize>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FastRecognizeKey {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    decision_start_index: Option<usize>,
    precedence: i32,
    recovery_symbols: BTreeSet<i32>,
    recovery_state: Option<usize>,
}

struct FastRecoveryRequest<'a, 'b> {
    atn: &'a Atn,
    transition: &'a Transition,
    expected_symbols: BTreeSet<i32>,
    target: usize,
    request: FastRecognizeRequest,
    visiting: &'b mut BTreeSet<FastRecognizeKey>,
    memo: &'b mut BTreeMap<FastRecognizeKey, Vec<FastRecognizeOutcome>>,
    expected: &'b mut ExpectedTokens,
}

struct FastCurrentTokenDeletionRequest<'a, 'b> {
    atn: &'a Atn,
    expected_symbols: BTreeSet<i32>,
    request: FastRecognizeRequest,
    visiting: &'b mut BTreeSet<FastRecognizeKey>,
    memo: &'b mut BTreeMap<FastRecognizeKey, Vec<FastRecognizeOutcome>>,
    expected: &'b mut ExpectedTokens,
}

struct RecoveryRequest<'a, 'b> {
    atn: &'a Atn,
    transition: &'a Transition,
    expected_symbols: BTreeSet<i32>,
    target: usize,
    request: RecognizeRequest<'a>,
    visiting: &'b mut BTreeSet<RecognizeKey>,
    memo: &'b mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
    expected: &'b mut ExpectedTokens,
}

struct CurrentTokenDeletionRequest<'a, 'b> {
    atn: &'a Atn,
    expected_symbols: BTreeSet<i32>,
    request: RecognizeRequest<'a>,
    visiting: &'b mut BTreeSet<RecognizeKey>,
    memo: &'b mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
    expected: &'b mut ExpectedTokens,
}

/// Carries the state needed after the normal token-recovery strategies fail
/// for a consuming transition.
struct ConsumingFailureFallback<'a> {
    atn: &'a Atn,
    target: usize,
    request: RecognizeRequest<'a>,
    symbol: i32,
    expected_symbols: BTreeSet<i32>,
    decision_start_index: Option<usize>,
    decision: Option<usize>,
}

/// Captures the parent-rule context needed when a called rule fails before it
/// can produce a normal outcome.
struct ChildRuleFailureRecovery<'a> {
    atn: &'a Atn,
    rule_index: usize,
    start_index: usize,
    follow_state: usize,
    stop_state: usize,
    member_values: BTreeMap<usize, i64>,
    expected: &'a ExpectedTokens,
}

/// Bundles the context needed to evaluate one semantic predicate transition.
#[derive(Clone, Copy, Debug)]
struct PredicateEval<'a> {
    index: usize,
    rule_index: usize,
    pred_index: usize,
    predicates: &'a [(usize, usize, ParserPredicate)],
    local_int_arg: Option<(usize, i64)>,
    member_values: &'a BTreeMap<usize, i64>,
}

impl<S> BaseParser<S>
where
    S: TokenSource,
{
    /// Creates a parser base over a buffered token stream and recognizer
    /// metadata.
    pub const fn new(input: CommonTokenStream<S>, data: RecognizerData) -> Self {
        Self {
            input,
            data,
            build_parse_trees: true,
            report_diagnostic_errors: false,
            prediction_mode: PredictionMode::Ll,
            prediction_diagnostics: Vec::new(),
            reported_prediction_diagnostics: BTreeSet::new(),
            int_members: BTreeMap::new(),
            invoked_predicates: Vec::new(),
        }
    }

    pub const fn input(&mut self) -> &mut CommonTokenStream<S> {
        &mut self.input
    }

    pub fn la(&mut self, offset: isize) -> i32 {
        self.input.la_token(offset)
    }

    pub fn consume(&mut self) {
        IntStream::consume(&mut self.input);
    }

    /// Sets a generated integer member value used by target-template tests.
    pub fn set_int_member(&mut self, member: usize, value: i64) {
        self.int_members.insert(member, value);
    }

    /// Reads a generated integer member value.
    pub fn int_member(&self, member: usize) -> Option<i64> {
        self.int_members.get(&member).copied()
    }

    /// Adds `delta` to a generated integer member and returns the new value.
    pub fn add_int_member(&mut self, member: usize, delta: i64) -> i64 {
        let value = self.int_members.entry(member).or_default();
        *value += delta;
        *value
    }

    /// Matches and consumes the current token when it has the expected token
    /// type.
    ///
    /// On success the consumed token is wrapped as a terminal parse-tree node.
    /// On mismatch the error carries vocabulary display names so diagnostics are
    /// stable across literal and symbolic token naming.
    pub fn match_token(&mut self, token_type: i32) -> Result<ParseTree, AntlrError> {
        let current = self
            .input
            .lt(1)
            .cloned()
            .ok_or_else(|| AntlrError::ParserError {
                line: 0,
                column: 0,
                message: "missing current token".to_owned(),
            })?;
        if current.token_type() == token_type {
            self.consume();
            Ok(ParseTree::Terminal(TerminalNode::new(current)))
        } else {
            Err(AntlrError::MismatchedInput {
                expected: self.vocabulary().display_name(token_type),
                found: self.vocabulary().display_name(current.token_type()),
            })
        }
    }

    pub fn match_eof(&mut self) -> Result<ParseTree, AntlrError> {
        self.match_token(TOKEN_EOF)
    }

    pub const fn rule_node(&self, context: ParserRuleContext) -> ParseTree {
        ParseTree::Rule(RuleNode::new(context))
    }

    /// Parses a generated rule by interpreting the parser ATN from the rule's
    /// start state to its stop state.
    ///
    /// The recognizer backtracks across alternatives and loop exits using token
    /// stream indices instead of committing to input consumption immediately.
    /// Once a viable ATN path is found, the parser consumes the accepted token
    /// interval and returns a rule node. The initial tree is intentionally flat;
    /// nested rule-node construction will be layered on top of the same
    /// recognition routine.
    pub fn parse_atn_rule(
        &mut self,
        atn: &Atn,
        rule_index: usize,
    ) -> Result<ParseTree, AntlrError> {
        let start_state = atn
            .rule_to_start_state()
            .get(rule_index)
            .copied()
            .ok_or_else(|| {
                AntlrError::Unsupported(format!("rule {rule_index} has no start state"))
            })?;
        let stop_state = atn
            .rule_to_stop_state()
            .get(rule_index)
            .copied()
            .filter(|state| *state != usize::MAX)
            .ok_or_else(|| {
                AntlrError::Unsupported(format!("rule {rule_index} has no stop state"))
            })?;

        let start_index = self.input.index();
        self.clear_prediction_diagnostics();
        let mut visiting = BTreeSet::new();
        let mut memo = BTreeMap::new();
        let mut expected = ExpectedTokens::default();
        let outcomes = self.recognize_state_fast(
            atn,
            FastRecognizeRequest {
                state_number: start_state,
                stop_state,
                index: start_index,
                rule_start_index: start_index,
                decision_start_index: None,
                precedence: 0,
                depth: 0,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            &mut visiting,
            &mut memo,
            &mut expected,
        );
        let Some(outcome) = select_best_fast_outcome(outcomes.into_iter()) else {
            let error = self.recognition_error(rule_index, start_index, &expected);
            report_token_source_errors(&self.input.drain_source_errors());
            return Err(error);
        };

        report_parser_diagnostics(&self.prediction_diagnostics);
        report_parser_diagnostics(&outcome.diagnostics);
        report_token_source_errors(&self.input.drain_source_errors());
        let mut context = ParserRuleContext::new(rule_index, self.state());
        if let Some(token) = self.token_at(start_index) {
            context.set_start(token);
        }
        if let Some(token) = self
            .previous_token_index(outcome.index)
            .and_then(|index| self.token_at(index))
        {
            context.set_stop(token);
        }
        self.input.seek(start_index);
        while self.input.index() < outcome.index {
            let token_type = self.la(1);
            let child = self.match_token(token_type)?;
            if self.build_parse_trees {
                context.add_child(child);
            }
        }
        if outcome.consumed_eof && self.la(1) == TOKEN_EOF && self.build_parse_trees {
            context.add_child(self.match_eof()?);
        }

        Ok(self.rule_node(context))
    }

    /// Parses a generated rule and returns semantic actions reached on the
    /// selected ATN path.
    ///
    /// This slower path preserves action ordering and token intervals for
    /// generated code that replays target-specific action templates after the
    /// recognizer has chosen one viable parse path.
    pub fn parse_atn_rule_with_actions(
        &mut self,
        atn: &Atn,
        rule_index: usize,
    ) -> Result<(ParseTree, Vec<ParserAction>), AntlrError> {
        self.parse_atn_rule_with_action_options(atn, rule_index, &[], false)
    }

    /// Parses a generated rule and emits ATN actions plus selected rule-init
    /// actions reached on the chosen path.
    ///
    /// Generated parsers use this when a grammar contains rule-level `@init`
    /// templates that must run for nested rule invocations. The runtime keeps
    /// the action list path-sensitive, so init templates are replayed only for
    /// rules that were actually entered by the selected parse.
    pub fn parse_atn_rule_with_action_inits(
        &mut self,
        atn: &Atn,
        rule_index: usize,
        init_action_rules: &[usize],
    ) -> Result<(ParseTree, Vec<ParserAction>), AntlrError> {
        self.parse_atn_rule_with_action_options(atn, rule_index, init_action_rules, false)
    }

    /// Parses a generated rule with optional semantic-action replay features.
    ///
    /// `track_alt_numbers` is used by grammars that opt into ANTLR's
    /// alt-numbered context behavior. It keeps ordinary parse-tree rendering
    /// unchanged for grammars that do not request that target template.
    pub fn parse_atn_rule_with_action_options(
        &mut self,
        atn: &Atn,
        rule_index: usize,
        init_action_rules: &[usize],
        track_alt_numbers: bool,
    ) -> Result<(ParseTree, Vec<ParserAction>), AntlrError> {
        self.parse_atn_rule_with_runtime_options(
            atn,
            rule_index,
            ParserRuntimeOptions {
                init_action_rules,
                track_alt_numbers,
                ..ParserRuntimeOptions::default()
            },
        )
    }

    /// Parses a generated rule with action replay and parser predicate support.
    ///
    /// `predicates` maps serialized `(rule_index, pred_index)` coordinates to
    /// target-template predicate semantics emitted by the generator. Missing
    /// entries are treated as true so unsupported predicate-free grammars keep
    /// the previous unconditional transition behavior.
    pub fn parse_atn_rule_with_runtime_options(
        &mut self,
        atn: &Atn,
        rule_index: usize,
        options: ParserRuntimeOptions<'_>,
    ) -> Result<(ParseTree, Vec<ParserAction>), AntlrError> {
        let ParserRuntimeOptions {
            init_action_rules,
            track_alt_numbers,
            predicates,
            rule_args,
            member_actions,
            return_actions,
        } = options;
        let start_state = atn
            .rule_to_start_state()
            .get(rule_index)
            .copied()
            .ok_or_else(|| {
                AntlrError::Unsupported(format!("rule {rule_index} has no start state"))
            })?;
        let stop_state = atn
            .rule_to_stop_state()
            .get(rule_index)
            .copied()
            .filter(|state| *state != usize::MAX)
            .ok_or_else(|| {
                AntlrError::Unsupported(format!("rule {rule_index} has no stop state"))
            })?;

        let start_index = self.input.index();
        self.clear_prediction_diagnostics();
        let init_action_rules = init_action_rules.iter().copied().collect::<BTreeSet<_>>();
        let mut visiting = BTreeSet::new();
        let mut memo = BTreeMap::new();
        let mut expected = ExpectedTokens::default();
        let member_values = self.int_members.clone();
        let return_values = BTreeMap::new();
        let outcomes = self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: start_state,
                stop_state,
                index: start_index,
                rule_start_index: start_index,
                decision_start_index: None,
                init_action_rules: &init_action_rules,
                predicates,
                rule_args,
                member_actions,
                return_actions,
                local_int_arg: None,
                member_values,
                return_values,
                rule_alt_number: 0,
                track_alt_numbers,
                precedence: 0,
                depth: 0,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            &mut visiting,
            &mut memo,
            &mut expected,
        );
        let Some(outcome) = select_best_outcome(outcomes.into_iter(), self.prediction_mode) else {
            let error = self.recognition_error(rule_index, start_index, &expected);
            report_token_source_errors(&self.input.drain_source_errors());
            return Err(error);
        };

        report_parser_diagnostics(&self.prediction_diagnostics);
        report_parser_diagnostics(&outcome.diagnostics);
        report_token_source_errors(&self.input.drain_source_errors());
        let mut actions = outcome.actions;
        if init_action_rules.contains(&rule_index) {
            actions.insert(
                0,
                ParserAction::new_rule_init(rule_index, start_index, Some(start_state)),
            );
        }
        let mut context = ParserRuleContext::new(rule_index, self.state());
        if track_alt_numbers {
            context.set_alt_number(outcome.alt_number);
        }
        for (name, value) in outcome.return_values {
            context.set_int_return(name, value);
        }
        if let Some(token) = self.token_at(start_index) {
            context.set_start(token);
        }
        if let Some(token) = self
            .previous_token_index(outcome.index)
            .and_then(|index| self.token_at(index))
        {
            context.set_stop(token);
        }
        if self.build_parse_trees {
            let nodes = fold_left_recursive_boundaries(outcome.nodes);
            for node in &nodes {
                context.add_child(self.recognized_node_tree(node, track_alt_numbers)?);
            }
        }
        self.input.seek(outcome.index);

        Ok((self.rule_node(context), actions))
    }

    /// Temporary parser entry used by generated parser methods while the parser
    /// ATN simulator is being implemented.
    ///
    /// This keeps generated parser crates buildable and gives us a stable method
    /// surface for every grammar rule. It intentionally accepts all remaining
    /// tokens into one rule context; it is not the final parser semantics.
    pub fn parse_interpreted_rule(&mut self, rule_index: usize) -> Result<ParseTree, AntlrError> {
        let mut context = ParserRuleContext::new(rule_index, self.state());
        while self.la(1) != TOKEN_EOF {
            let token_type = self.la(1);
            let child = self.match_token(token_type)?;
            if self.build_parse_trees {
                context.add_child(child);
            }
        }
        if self.build_parse_trees {
            context.add_child(self.match_eof()?);
        }
        Ok(self.rule_node(context))
    }

    /// Builds the parser error reported when no ATN path can reach the active
    /// rule stop state.
    fn recognition_error(
        &mut self,
        rule_index: usize,
        start_index: usize,
        expected: &ExpectedTokens,
    ) -> AntlrError {
        let (index, message) = self.expected_error_message(rule_index, start_index, expected);
        self.input.seek(index);
        let current = self.input.lt(1).cloned();
        let line = current.as_ref().map(Token::line).unwrap_or_default();
        let column = current.as_ref().map(Token::column).unwrap_or_default();
        AntlrError::ParserError {
            line,
            column,
            message,
        }
    }

    /// Builds the token index and ANTLR-compatible message for a failed rule.
    fn expected_error_message(
        &mut self,
        rule_index: usize,
        start_index: usize,
        expected: &ExpectedTokens,
    ) -> (usize, String) {
        let index = expected
            .index
            .or_else(|| expected.no_viable.map(|no_viable| no_viable.error_index))
            .unwrap_or_else(|| self.input.index());
        self.input.seek(index);
        let current = self.input.lt(1).cloned();
        let message = if expected
            .no_viable
            .as_ref()
            .is_some_and(|no_viable| no_viable.error_index == index)
        {
            let start = expected
                .no_viable
                .as_ref()
                .map_or(start_index, |no_viable| no_viable.start_index);
            let text = display_input_text(&self.input.text(start, index));
            format!("no viable alternative at input '{text}'")
        } else if expected.symbols.is_empty() {
            if expected.index.is_some() {
                format!(
                    "missing {} at {}",
                    self.expected_symbols_display(&expected.symbols),
                    current
                        .as_ref()
                        .map_or_else(|| "'<EOF>'".to_owned(), token_input_display)
                )
            } else {
                format!("no viable alternative while parsing rule {rule_index}")
            }
        } else {
            format!(
                "mismatched input {} expecting {}",
                current
                    .as_ref()
                    .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
                self.expected_symbols_display(&expected.symbols)
            )
        };
        (index, message)
    }

    /// Converts a failed child rule into a recovered outcome so the parent can
    /// continue after reporting the child diagnostic.
    fn child_rule_failure_recovery(
        &mut self,
        rule_index: usize,
        start_index: usize,
        sync_symbols: &BTreeSet<i32>,
        member_values: BTreeMap<usize, i64>,
        expected: &ExpectedTokens,
    ) -> Option<RecognizeOutcome> {
        let (error_index, message) = self.expected_error_message(rule_index, start_index, expected);
        let token = self.token_at(error_index);
        let mut next_index = error_index;
        loop {
            let symbol = self.token_type_at(next_index);
            if sync_symbols.contains(&symbol) {
                if next_index == error_index {
                    return None;
                }
                break;
            }
            if symbol == TOKEN_EOF {
                break;
            }
            let after = self.consume_index(next_index, symbol);
            if after == next_index {
                break;
            }
            next_index = after;
        }
        Some(RecognizeOutcome {
            index: next_index,
            consumed_eof: false,
            alt_number: 0,
            member_values,
            return_values: BTreeMap::new(),
            diagnostics: vec![diagnostic_for_token(token.as_ref(), message)],
            decisions: Vec::new(),
            actions: Vec::new(),
            nodes: vec![RecognizedNode::ErrorToken { index: error_index }],
        })
    }

    /// Adapts the optional recovery result to the normal outcome list used by
    /// rule-call transitions.
    fn child_rule_failure_recovery_outcomes(
        &mut self,
        request: ChildRuleFailureRecovery<'_>,
    ) -> Vec<RecognizeOutcome> {
        let sync_symbols =
            state_sync_symbols(request.atn, request.follow_state, request.stop_state);
        self.child_rule_failure_recovery(
            request.rule_index,
            request.start_index,
            &sync_symbols,
            request.member_values,
            request.expected,
        )
        .into_iter()
        .collect()
    }

    /// Formats expected token types using ANTLR's single-token or set syntax.
    fn expected_symbols_display(&self, symbols: &BTreeSet<i32>) -> String {
        expected_symbols_display(symbols, self.vocabulary())
    }

    /// Returns the single-token deletion repair if the token after `index`
    /// satisfies the failed consuming transition.
    fn single_token_deletion(
        &mut self,
        transition: &Transition,
        index: usize,
        max_token_type: i32,
        expected_symbols: &BTreeSet<i32>,
    ) -> Option<(ParserDiagnostic, usize, i32)> {
        let current_symbol = self.token_type_at(index);
        if current_symbol == TOKEN_EOF {
            return None;
        }
        let next_index = self.consume_index(index, current_symbol);
        if next_index == index {
            return None;
        }
        let next_symbol = self.token_type_at(next_index);
        if !transition.matches(next_symbol, 1, max_token_type) {
            return None;
        }
        let transition_expected = transition_expected_symbols(transition, max_token_type);
        let expected_display = self.expected_symbols_display(if expected_symbols.is_empty() {
            &transition_expected
        } else {
            expected_symbols
        });
        let current = self.token_at(index);
        let message = format!(
            "extraneous input {} expecting {expected_display}",
            current
                .as_ref()
                .map_or_else(|| "'<EOF>'".to_owned(), token_input_display)
        );
        Some((
            diagnostic_for_token(current.as_ref(), message),
            next_index,
            next_symbol,
        ))
    }

    /// Returns the repair used when deleting the current token lets a recovery
    /// state continue with the following token.
    fn current_token_deletion(
        &mut self,
        index: usize,
        expected_symbols: &BTreeSet<i32>,
    ) -> Option<(ParserDiagnostic, usize, Vec<usize>)> {
        if expected_symbols.is_empty() {
            return None;
        }
        let current_symbol = self.token_type_at(index);
        if current_symbol == TOKEN_EOF {
            return None;
        }
        let current = self.token_at(index);
        let message = format!(
            "extraneous input {} expecting {}",
            current
                .as_ref()
                .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
            self.expected_symbols_display(expected_symbols)
        );
        let diagnostic = diagnostic_for_token(current.as_ref(), message);
        let mut skipped = Vec::new();
        let mut cursor = index;
        loop {
            let symbol = self.token_type_at(cursor);
            if symbol == TOKEN_EOF {
                return None;
            }
            skipped.push(cursor);
            let next_index = self.consume_index(cursor, symbol);
            if next_index == cursor {
                return None;
            }
            let next_symbol = self.token_type_at(next_index);
            if expected_symbols.contains(&next_symbol) {
                return Some((diagnostic, next_index, skipped));
            }
            cursor = next_index;
        }
    }

    /// Returns the single-token insertion repair for a failed consuming
    /// transition. The caller validates the repair by continuing from the
    /// transition target at the same input index.
    fn single_token_insertion(
        &mut self,
        transition: &Transition,
        index: usize,
        max_token_type: i32,
        expected_symbols: &BTreeSet<i32>,
        follow_symbols: &BTreeSet<i32>,
    ) -> Option<(ParserDiagnostic, i32, String)> {
        let current_symbol = self.token_type_at(index);
        if !follow_symbols.contains(&current_symbol) {
            return None;
        }
        let transition_expected = transition_expected_symbols(transition, max_token_type);
        let token_type = transition_expected.iter().next().copied()?;
        let expected_display = self.expected_symbols_display(if expected_symbols.is_empty() {
            &transition_expected
        } else {
            expected_symbols
        });
        let mut token_symbols = BTreeSet::new();
        token_symbols.insert(token_type);
        let missing_token_display = self.expected_symbols_display(&token_symbols);
        let current = self.token_at(index);
        let message = format!(
            "missing {expected_display} at {}",
            current
                .as_ref()
                .map_or_else(|| "'<EOF>'".to_owned(), token_input_display)
        );
        let text = format!("<missing {missing_token_display}>");
        Some((
            diagnostic_for_token(current.as_ref(), message),
            token_type,
            text,
        ))
    }

    /// Explores ANTLR's single-token deletion recovery for the fast recognizer:
    /// skip the unexpected current token when the following token satisfies the
    /// transition that failed.
    fn fast_single_token_deletion_recovery(
        &mut self,
        recovery: FastRecoveryRequest<'_, '_>,
    ) -> Vec<FastRecognizeOutcome> {
        let FastRecoveryRequest {
            atn,
            transition,
            expected_symbols,
            target,
            request,
            visiting,
            memo,
            expected,
        } = recovery;
        let FastRecognizeRequest {
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            precedence,
            depth,
            ..
        } = request;
        let Some((diagnostic, next_index, next_symbol)) =
            self.single_token_deletion(transition, index, atn.max_token_type(), &expected_symbols)
        else {
            return Vec::new();
        };
        let after_next = self.consume_index(next_index, next_symbol);
        self.recognize_state_fast(
            atn,
            FastRecognizeRequest {
                state_number: target,
                stop_state,
                index: after_next,
                rule_start_index,
                decision_start_index,
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            outcome.consumed_eof |= next_symbol == TOKEN_EOF;
            outcome.diagnostics.insert(0, diagnostic.clone());
            outcome
        })
        .collect()
    }

    /// Explores ANTLR's single-token insertion recovery for the fast recognizer:
    /// pretend the expected transition token was present and continue without
    /// consuming the current token.
    fn fast_single_token_insertion_recovery(
        &mut self,
        recovery: FastRecoveryRequest<'_, '_>,
    ) -> Vec<FastRecognizeOutcome> {
        let FastRecoveryRequest {
            atn,
            transition,
            expected_symbols,
            target,
            request,
            visiting,
            memo,
            expected,
        } = recovery;
        let FastRecognizeRequest {
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            precedence,
            depth,
            ..
        } = request;
        let follow_symbols = state_expected_symbols(atn, transition.target());
        let Some((diagnostic, _token_type, _text)) = self.single_token_insertion(
            transition,
            index,
            atn.max_token_type(),
            &expected_symbols,
            &follow_symbols,
        ) else {
            return Vec::new();
        };
        self.recognize_state_fast(
            atn,
            FastRecognizeRequest {
                state_number: target,
                stop_state,
                index,
                rule_start_index,
                decision_start_index,
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            outcome.diagnostics.insert(0, diagnostic.clone());
            outcome
        })
        .collect()
    }

    /// Retries the current fast-recognition state after deleting one
    /// unexpected token that precedes a valid loop or block continuation.
    fn fast_current_token_deletion_recovery(
        &mut self,
        recovery: FastCurrentTokenDeletionRequest<'_, '_>,
    ) -> Vec<FastRecognizeOutcome> {
        let FastCurrentTokenDeletionRequest {
            atn,
            expected_symbols,
            mut request,
            visiting,
            memo,
            expected,
        } = recovery;
        if request.index == request.rule_start_index {
            return Vec::new();
        }
        let Some((diagnostic, next_index, _skipped)) =
            self.current_token_deletion(request.index, &expected_symbols)
        else {
            return Vec::new();
        };
        request.state_number = request.recovery_state.unwrap_or(request.state_number);
        request.index = next_index;
        request.depth += 1;
        request.recovery_state = None;
        self.recognize_state_fast(atn, request, visiting, memo, expected)
            .into_iter()
            .map(|mut outcome| {
                outcome.diagnostics.insert(0, diagnostic.clone());
                outcome
            })
            .collect()
    }

    /// Attempts to reach `stop_state` from `state_number` without committing
    /// token consumption to the parser's public stream position.
    fn recognize_state_fast(
        &mut self,
        atn: &Atn,
        request: FastRecognizeRequest,
        visiting: &mut BTreeSet<FastRecognizeKey>,
        memo: &mut BTreeMap<FastRecognizeKey, Vec<FastRecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<FastRecognizeOutcome> {
        let FastRecognizeRequest {
            state_number,
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            precedence,
            depth,
            recovery_symbols,
            recovery_state,
        } = request;
        if depth > RECOGNITION_DEPTH_LIMIT {
            return Vec::new();
        }
        if state_number == stop_state {
            return vec![FastRecognizeOutcome {
                index,
                consumed_eof: false,
                diagnostics: Vec::new(),
            }];
        }
        let key = FastRecognizeKey {
            state_number,
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            precedence,
            recovery_symbols: recovery_symbols.clone(),
            recovery_state,
        };
        if let Some(outcomes) = memo.get(&key) {
            return outcomes.clone();
        }

        let visit_key = key.clone();
        if !visiting.insert(visit_key.clone()) {
            return Vec::new();
        }

        let Some(state) = atn.state(state_number) else {
            visiting.remove(&visit_key);
            return Vec::new();
        };
        let next_decision_start_index = if starts_prediction_decision(state) {
            Some(index)
        } else {
            decision_start_index
        };
        let (epsilon_recovery_symbols, epsilon_recovery_state) =
            next_recovery_context(atn, state, &recovery_symbols, recovery_state);
        let mut outcomes = Vec::new();
        for transition in &state.transitions {
            match transition {
                Transition::Epsilon { target }
                | Transition::Predicate { target, .. }
                | Transition::Action { target, .. } => {
                    outcomes.extend(self.recognize_state_fast(
                        atn,
                        FastRecognizeRequest {
                            state_number: *target,
                            stop_state,
                            index,
                            rule_start_index,
                            decision_start_index: next_decision_start_index,
                            precedence,
                            depth: depth + 1,
                            recovery_symbols: epsilon_recovery_symbols.clone(),
                            recovery_state: epsilon_recovery_state,
                        },
                        visiting,
                        memo,
                        expected,
                    ));
                }
                Transition::Precedence {
                    target,
                    precedence: transition_precedence,
                } => {
                    if *transition_precedence >= precedence {
                        outcomes.extend(self.recognize_state_fast(
                            atn,
                            FastRecognizeRequest {
                                state_number: *target,
                                stop_state,
                                index,
                                rule_start_index,
                                decision_start_index: next_decision_start_index,
                                precedence,
                                depth: depth + 1,
                                recovery_symbols: epsilon_recovery_symbols.clone(),
                                recovery_state: epsilon_recovery_state,
                            },
                            visiting,
                            memo,
                            expected,
                        ));
                    }
                }
                Transition::Rule {
                    target,
                    rule_index,
                    follow_state,
                    precedence: rule_precedence,
                    ..
                } => {
                    let Some(child_stop) = atn.rule_to_stop_state().get(*rule_index).copied()
                    else {
                        continue;
                    };
                    let expected_before_child = expected.clone();
                    let children = self.recognize_state_fast(
                        atn,
                        FastRecognizeRequest {
                            state_number: *target,
                            stop_state: child_stop,
                            index,
                            rule_start_index: index,
                            decision_start_index: None,
                            precedence: *rule_precedence,
                            depth: depth + 1,
                            recovery_symbols: epsilon_recovery_symbols.clone(),
                            recovery_state: epsilon_recovery_state,
                        },
                        visiting,
                        memo,
                        expected,
                    );
                    if children
                        .iter()
                        .any(|child| child.diagnostics.is_empty() && child.index > index)
                    {
                        *expected = expected_before_child;
                    }
                    for child in children {
                        outcomes.extend(
                            self.recognize_state_fast(
                                atn,
                                FastRecognizeRequest {
                                    state_number: *follow_state,
                                    stop_state,
                                    index: child.index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
                                    recovery_state: None,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                outcome.consumed_eof |= child.consumed_eof;
                                let mut diagnostics = child.diagnostics.clone();
                                diagnostics.append(&mut outcome.diagnostics);
                                outcome.diagnostics = diagnostics;
                                outcome
                            }),
                        );
                    }
                }
                Transition::Atom { target, .. }
                | Transition::Range { target, .. }
                | Transition::Set { target, .. }
                | Transition::NotSet { target, .. }
                | Transition::Wildcard { target, .. } => {
                    let symbol = self.token_type_at(index);
                    if transition.matches(symbol, 1, atn.max_token_type()) {
                        let next_index = self.consume_index(index, symbol);
                        outcomes.extend(
                            self.recognize_state_fast(
                                atn,
                                FastRecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index: next_index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
                                    recovery_state: None,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                outcome.consumed_eof |= symbol == TOKEN_EOF;
                                outcome
                            }),
                        );
                    } else {
                        let expected_symbols =
                            recovery_expected_symbols(atn, state.state_number, &recovery_symbols);
                        if expected_symbols.contains(&symbol) {
                            continue;
                        }
                        expected.record_transition(index, transition, atn.max_token_type());
                        record_no_viable_if_ambiguous(expected, next_decision_start_index, index);
                        outcomes.extend(self.fast_single_token_deletion_recovery(
                            FastRecoveryRequest {
                                atn,
                                transition,
                                expected_symbols: expected_symbols.clone(),
                                target: *target,
                                request: FastRecognizeRequest {
                                    state_number,
                                    stop_state,
                                    index,
                                    rule_start_index,
                                    decision_start_index,
                                    precedence,
                                    depth,
                                    recovery_symbols: recovery_symbols.clone(),
                                    recovery_state,
                                },
                                visiting,
                                memo,
                                expected,
                            },
                        ));
                        if !state_is_left_recursive_rule(atn, state) {
                            outcomes.extend(self.fast_single_token_insertion_recovery(
                                FastRecoveryRequest {
                                    atn,
                                    transition,
                                    expected_symbols: expected_symbols.clone(),
                                    target: *target,
                                    request: FastRecognizeRequest {
                                        state_number,
                                        stop_state,
                                        index,
                                        rule_start_index,
                                        decision_start_index,
                                        precedence,
                                        depth,
                                        recovery_symbols: recovery_symbols.clone(),
                                        recovery_state,
                                    },
                                    visiting,
                                    memo,
                                    expected,
                                },
                            ));
                        }
                        outcomes.extend(self.fast_current_token_deletion_recovery(
                            FastCurrentTokenDeletionRequest {
                                atn,
                                expected_symbols,
                                request: FastRecognizeRequest {
                                    state_number,
                                    stop_state,
                                    index,
                                    rule_start_index,
                                    decision_start_index,
                                    precedence,
                                    depth,
                                    recovery_symbols: recovery_symbols.clone(),
                                    recovery_state,
                                },
                                visiting,
                                memo,
                                expected,
                            },
                        ));
                    }
                }
            }
        }

        visiting.remove(&visit_key);
        if self.prediction_mode == PredictionMode::Ll {
            discard_recovered_fast_outcomes_if_clean_path_exists(&mut outcomes);
        }
        dedupe_fast_outcomes(&mut outcomes);
        memo.insert(key, outcomes.clone());
        outcomes
    }

    /// Explores single-token deletion recovery while preserving the matched
    /// token and skipped error token in the selected parse tree path.
    fn single_token_deletion_recovery(
        &mut self,
        recovery: RecoveryRequest<'_, '_>,
    ) -> Vec<RecognizeOutcome> {
        let RecoveryRequest {
            atn,
            transition,
            expected_symbols,
            target,
            request,
            visiting,
            memo,
            expected,
        } = recovery;
        let RecognizeRequest {
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            init_action_rules,
            predicates,
            rule_args,
            member_actions,
            return_actions,
            local_int_arg,
            member_values,
            return_values,
            rule_alt_number,
            track_alt_numbers,
            precedence,
            depth,
            ..
        } = request;
        let Some((diagnostic, next_index, next_symbol)) =
            self.single_token_deletion(transition, index, atn.max_token_type(), &expected_symbols)
        else {
            return Vec::new();
        };
        let after_next = self.consume_index(next_index, next_symbol);
        self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: target,
                stop_state,
                index: after_next,
                rule_start_index,
                decision_start_index,
                init_action_rules,
                predicates,
                rule_args,
                member_actions,
                return_actions,
                local_int_arg,
                member_values,
                return_values,
                rule_alt_number,
                track_alt_numbers,
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            outcome.consumed_eof |= next_symbol == TOKEN_EOF;
            outcome.diagnostics.insert(0, diagnostic.clone());
            outcome
                .nodes
                .insert(0, RecognizedNode::Token { index: next_index });
            outcome
                .nodes
                .insert(0, RecognizedNode::ErrorToken { index });
            outcome
        })
        .collect()
    }

    /// Retries the current recognition state after deleting one unexpected
    /// token, preserving the deleted token as an error node in the parse tree.
    fn current_token_deletion_recovery(
        &mut self,
        recovery: CurrentTokenDeletionRequest<'_, '_>,
    ) -> Vec<RecognizeOutcome> {
        let CurrentTokenDeletionRequest {
            atn,
            expected_symbols,
            mut request,
            visiting,
            memo,
            expected,
        } = recovery;
        let error_index = request.index;
        if error_index == request.rule_start_index {
            return Vec::new();
        }
        let Some((diagnostic, next_index, skipped)) =
            self.current_token_deletion(error_index, &expected_symbols)
        else {
            return Vec::new();
        };
        request.state_number = request.recovery_state.unwrap_or(request.state_number);
        request.index = next_index;
        request.depth += 1;
        request.recovery_state = None;
        self.recognize_state(atn, request, visiting, memo, expected)
            .into_iter()
            .map(|mut outcome| {
                outcome.diagnostics.insert(0, diagnostic.clone());
                for index in skipped.iter().rev() {
                    outcome
                        .nodes
                        .insert(0, RecognizedNode::ErrorToken { index: *index });
                }
                outcome
            })
            .collect()
    }

    /// Falls back after deletion/insertion repairs cannot continue from a
    /// failed consuming transition.
    fn consuming_failure_fallback(
        &mut self,
        fallback: ConsumingFailureFallback<'_>,
        visiting: &mut BTreeSet<RecognizeKey>,
        memo: &mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        if fallback.expected_symbols.is_empty() {
            return Vec::new();
        }
        if fallback.symbol == TOKEN_EOF {
            return self.eof_consuming_failure_fallback(fallback, expected);
        }
        self.non_eof_consuming_failure_fallback(fallback, visiting, memo, expected)
    }

    /// Keeps unexpected non-EOF input visible as an error node when no repair
    /// path can otherwise reach the transition target.
    fn non_eof_consuming_failure_fallback(
        &mut self,
        fallback: ConsumingFailureFallback<'_>,
        visiting: &mut BTreeSet<RecognizeKey>,
        memo: &mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        let ConsumingFailureFallback {
            atn,
            target,
            request,
            symbol,
            expected_symbols,
            decision_start_index,
            decision,
        } = fallback;
        let error_index = request.index;
        let diagnostic =
            self.recovery_failure_diagnostic(error_index, decision_start_index, &expected_symbols);
        let next_index = self.consume_index(error_index, symbol);
        self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: target,
                stop_state: request.stop_state,
                index: next_index,
                rule_start_index: request.rule_start_index,
                decision_start_index,
                init_action_rules: request.init_action_rules,
                predicates: request.predicates,
                rule_args: request.rule_args,
                member_actions: request.member_actions,
                return_actions: request.return_actions,
                local_int_arg: request.local_int_arg,
                member_values: request.member_values,
                return_values: request.return_values,
                rule_alt_number: request.rule_alt_number,
                track_alt_numbers: request.track_alt_numbers,
                precedence: request.precedence,
                depth: request.depth + 1,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            prepend_decision(&mut outcome, decision);
            outcome.diagnostics.insert(0, diagnostic.clone());
            outcome
                .nodes
                .insert(0, RecognizedNode::ErrorToken { index: error_index });
            outcome
        })
        .collect()
    }

    /// Stops the current rule at EOF after a nested failure, matching ANTLR's
    /// behavior of unwinding instead of inserting caller tokens at EOF.
    fn eof_consuming_failure_fallback(
        &mut self,
        fallback: ConsumingFailureFallback<'_>,
        expected: &ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        let request = fallback.request;
        if request.index == request.rule_start_index {
            return Vec::new();
        }
        let diagnostic =
            self.eof_rule_recovery_diagnostic(request.index, &fallback.expected_symbols, expected);
        vec![RecognizeOutcome {
            index: request.index,
            consumed_eof: false,
            alt_number: request.rule_alt_number,
            member_values: request.member_values,
            return_values: request.return_values,
            diagnostics: vec![diagnostic],
            decisions: Vec::new(),
            actions: Vec::new(),
            nodes: Vec::new(),
        }]
    }

    /// Explores single-token insertion recovery while adding a conjured
    /// missing-token error node to the selected parse tree path.
    fn single_token_insertion_recovery(
        &mut self,
        recovery: RecoveryRequest<'_, '_>,
    ) -> Vec<RecognizeOutcome> {
        let RecoveryRequest {
            atn,
            transition,
            expected_symbols,
            target,
            request,
            visiting,
            memo,
            expected,
        } = recovery;
        let RecognizeRequest {
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            init_action_rules,
            predicates,
            rule_args,
            member_actions,
            return_actions,
            local_int_arg,
            member_values,
            return_values,
            rule_alt_number,
            track_alt_numbers,
            precedence,
            depth,
            ..
        } = request;
        let follow_symbols = state_expected_symbols(atn, transition.target());
        let Some((diagnostic, token_type, text)) = self.single_token_insertion(
            transition,
            index,
            atn.max_token_type(),
            &expected_symbols,
            &follow_symbols,
        ) else {
            return Vec::new();
        };
        self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: target,
                stop_state,
                index,
                rule_start_index,
                decision_start_index,
                init_action_rules,
                predicates,
                rule_args,
                member_actions,
                return_actions,
                local_int_arg,
                member_values,
                return_values,
                rule_alt_number,
                track_alt_numbers,
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
                recovery_state: None,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            outcome.diagnostics.insert(0, diagnostic.clone());
            outcome.nodes.insert(
                0,
                RecognizedNode::MissingToken {
                    token_type,
                    at_index: index,
                    text: text.clone(),
                },
            );
            outcome
        })
        .collect()
    }

    /// Attempts to reach `stop_state` and carries semantic actions for the
    /// selected parser path.
    fn recognize_state(
        &mut self,
        atn: &Atn,
        request: RecognizeRequest<'_>,
        visiting: &mut BTreeSet<RecognizeKey>,
        memo: &mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        let request_template = request.clone();
        let RecognizeRequest {
            state_number,
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            init_action_rules,
            predicates,
            rule_args,
            member_actions,
            return_actions,
            local_int_arg,
            member_values,
            return_values,
            rule_alt_number,
            track_alt_numbers,
            precedence,
            depth,
            recovery_symbols,
            recovery_state,
        } = request;
        if depth > RECOGNITION_DEPTH_LIMIT {
            return Vec::new();
        }
        if state_number == stop_state {
            return stop_outcome(index, rule_alt_number, member_values, return_values);
        }
        let key = RecognizeKey {
            state_number,
            stop_state,
            index,
            rule_start_index,
            decision_start_index,
            local_int_arg,
            member_values: member_values.clone(),
            return_values: return_values.clone(),
            rule_alt_number,
            track_alt_numbers,
            precedence,
            recovery_symbols: recovery_symbols.clone(),
            recovery_state,
        };
        if let Some(outcomes) = memo.get(&key) {
            return outcomes.clone();
        }

        let visit_key = key.clone();
        if !visiting.insert(visit_key.clone()) {
            return Vec::new();
        }

        let Some(state) = atn.state(state_number) else {
            visiting.remove(&visit_key);
            return Vec::new();
        };
        let next_decision_start_index = if starts_prediction_decision(state) {
            Some(index)
        } else {
            decision_start_index
        };
        let (epsilon_recovery_symbols, epsilon_recovery_state) =
            next_recovery_context(atn, state, &recovery_symbols, recovery_state);
        let mut outcomes = Vec::new();
        for (transition_index, transition) in state.transitions.iter().enumerate() {
            let decision = transition_decision(atn, state, transition_index, predicates);
            let next_alt_number =
                next_alt_number(state, transition_index, rule_alt_number, track_alt_numbers);
            match transition {
                Transition::Epsilon { target } | Transition::Action { target, .. } => {
                    let action_rule_index = match transition {
                        Transition::Action { rule_index, .. } => Some(*rule_index),
                        _ => None,
                    };
                    outcomes.extend(self.recognize_epsilon_or_action_step(
                        atn,
                        &request_template,
                        EpsilonActionStep {
                            source_state: state_number,
                            target: *target,
                            action_rule_index,
                            left_recursive_boundary: left_recursive_boundary(atn, state, *target),
                            decision,
                            decision_start_index: next_decision_start_index,
                            alt_number: next_alt_number,
                            recovery_symbols: epsilon_recovery_symbols.clone(),
                            recovery_state: epsilon_recovery_state,
                        },
                        RecognizeScratch {
                            visiting,
                            memo,
                            expected,
                        },
                    ));
                }
                Transition::Predicate {
                    target,
                    rule_index,
                    pred_index,
                    ..
                } => {
                    if self.parser_predicate_matches(PredicateEval {
                        index,
                        rule_index: *rule_index,
                        pred_index: *pred_index,
                        predicates,
                        local_int_arg,
                        member_values: &member_values,
                    }) {
                        let left_recursive_boundary = left_recursive_boundary(atn, state, *target);
                        outcomes.extend(
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    init_action_rules,
                                    predicates,
                                    rule_args,
                                    member_actions,
                                    return_actions,
                                    local_int_arg,
                                    member_values: member_values.clone(),
                                    return_values: return_values.clone(),
                                    rule_alt_number: next_alt_number,
                                    track_alt_numbers,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: epsilon_recovery_symbols.clone(),
                                    recovery_state: epsilon_recovery_state,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                prepend_decision(&mut outcome, decision);
                                if let Some(rule_index) = left_recursive_boundary {
                                    outcome.nodes.insert(
                                        0,
                                        RecognizedNode::LeftRecursiveBoundary { rule_index },
                                    );
                                }
                                outcome
                            }),
                        );
                    } else {
                        record_predicate_no_viable(expected, next_decision_start_index, index);
                    }
                }
                Transition::Precedence {
                    target,
                    precedence: transition_precedence,
                } => {
                    if *transition_precedence >= precedence {
                        outcomes.extend(
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    init_action_rules,
                                    predicates,
                                    rule_args,
                                    member_actions,
                                    return_actions,
                                    local_int_arg,
                                    member_values: member_values.clone(),
                                    return_values: return_values.clone(),
                                    rule_alt_number: next_alt_number,
                                    track_alt_numbers,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: epsilon_recovery_symbols.clone(),
                                    recovery_state: epsilon_recovery_state,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                prepend_decision(&mut outcome, decision);
                                outcome
                            }),
                        );
                    }
                }
                Transition::Rule {
                    target,
                    rule_index,
                    follow_state,
                    precedence: rule_precedence,
                    ..
                } => {
                    let Some(child_stop) = atn.rule_to_stop_state().get(*rule_index).copied()
                    else {
                        continue;
                    };
                    let child_local_int_arg =
                        rule_local_int_arg(rule_args, state_number, *rule_index, local_int_arg);
                    let expected_before_child = expected.clone();
                    let children = self.recognize_state(
                        atn,
                        RecognizeRequest {
                            state_number: *target,
                            stop_state: child_stop,
                            index,
                            rule_start_index: index,
                            decision_start_index: None,
                            init_action_rules,
                            predicates,
                            rule_args,
                            member_actions,
                            return_actions,
                            local_int_arg: child_local_int_arg,
                            member_values: member_values.clone(),
                            return_values: BTreeMap::new(),
                            rule_alt_number: 0,
                            track_alt_numbers,
                            precedence: *rule_precedence,
                            depth: depth + 1,
                            recovery_symbols: epsilon_recovery_symbols.clone(),
                            recovery_state: epsilon_recovery_state,
                        },
                        visiting,
                        memo,
                        expected,
                    );
                    let children = if children.is_empty() {
                        self.child_rule_failure_recovery_outcomes(ChildRuleFailureRecovery {
                            atn,
                            rule_index: *rule_index,
                            start_index: index,
                            follow_state: *follow_state,
                            stop_state,
                            member_values: member_values.clone(),
                            expected,
                        })
                    } else {
                        children
                    };
                    let preserve_child_expected =
                        self.child_expected_reaches_clean_eof(&children, expected);
                    restore_expected(
                        &children,
                        index,
                        expected,
                        expected_before_child,
                        preserve_child_expected,
                    );
                    for child in children {
                        let child_node = RecognizedNode::Rule {
                            rule_index: *rule_index,
                            invoking_state: invoking_state_number(state_number),
                            alt_number: child.alt_number,
                            start_index: index,
                            stop_index: self.previous_token_index(child.index),
                            return_values: child.return_values.clone(),
                            children: fold_left_recursive_boundaries(child.nodes.clone()),
                        };
                        outcomes.extend(
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *follow_state,
                                    stop_state,
                                    index: child.index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    init_action_rules,
                                    predicates,
                                    rule_args,
                                    member_actions,
                                    return_actions,
                                    local_int_arg,
                                    member_values: child.member_values.clone(),
                                    return_values: return_values.clone(),
                                    rule_alt_number,
                                    track_alt_numbers,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
                                    recovery_state: None,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                outcome.consumed_eof |= child.consumed_eof;
                                let mut diagnostics = child.diagnostics.clone();
                                diagnostics.append(&mut outcome.diagnostics);
                                outcome.diagnostics = diagnostics;
                                let mut decisions = child.decisions.clone();
                                decisions.append(&mut outcome.decisions);
                                outcome.decisions = decisions;
                                prepend_decision(&mut outcome, decision);
                                let mut actions = child.actions.clone();
                                if init_action_rules.contains(rule_index) {
                                    actions.insert(
                                        0,
                                        ParserAction::new_rule_init(
                                            *rule_index,
                                            index,
                                            Some(*follow_state),
                                        ),
                                    );
                                }
                                actions.append(&mut outcome.actions);
                                outcome.actions = actions;
                                outcome.nodes.insert(0, child_node.clone());
                                outcome
                            }),
                        );
                    }
                }
                Transition::Atom { target, .. }
                | Transition::Range { target, .. }
                | Transition::Set { target, .. }
                | Transition::NotSet { target, .. }
                | Transition::Wildcard { target, .. } => {
                    let symbol = self.token_type_at(index);
                    if transition.matches(symbol, 1, atn.max_token_type()) {
                        let next_index = self.consume_index(index, symbol);
                        outcomes.extend(
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index: next_index,
                                    rule_start_index,
                                    decision_start_index: next_decision_start_index,
                                    init_action_rules,
                                    predicates,
                                    rule_args,
                                    member_actions,
                                    return_actions,
                                    local_int_arg,
                                    member_values: member_values.clone(),
                                    return_values: return_values.clone(),
                                    rule_alt_number: next_alt_number,
                                    track_alt_numbers,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
                                    recovery_state: None,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                prepend_decision(&mut outcome, decision);
                                outcome.consumed_eof |= symbol == TOKEN_EOF;
                                outcome.nodes.insert(0, RecognizedNode::Token { index });
                                outcome
                            }),
                        );
                    } else {
                        let expected_symbols =
                            recovery_expected_symbols(atn, state.state_number, &recovery_symbols);
                        if expected_symbols.contains(&symbol) {
                            continue;
                        }
                        expected.record_transition(index, transition, atn.max_token_type());
                        record_no_viable_if_ambiguous(expected, next_decision_start_index, index);
                        let before_recovery = outcomes.len();
                        let recovery_request = request_template.clone();
                        outcomes.extend(
                            self.single_token_deletion_recovery(RecoveryRequest {
                                atn,
                                transition,
                                expected_symbols: expected_symbols.clone(),
                                target: *target,
                                request: recovery_request.clone(),
                                visiting,
                                memo,
                                expected,
                            })
                            .into_iter()
                            .map(|mut outcome| {
                                prepend_decision(&mut outcome, decision);
                                outcome
                            }),
                        );
                        if !state_is_left_recursive_rule(atn, state) {
                            outcomes.extend(
                                self.single_token_insertion_recovery(RecoveryRequest {
                                    atn,
                                    transition,
                                    expected_symbols: expected_symbols.clone(),
                                    target: *target,
                                    request: recovery_request.clone(),
                                    visiting,
                                    memo,
                                    expected,
                                })
                                .into_iter()
                                .map(|mut outcome| {
                                    prepend_decision(&mut outcome, decision);
                                    outcome
                                }),
                            );
                        }
                        outcomes.extend(self.current_token_deletion_recovery(
                            CurrentTokenDeletionRequest {
                                atn,
                                expected_symbols: expected_symbols.clone(),
                                request: recovery_request.clone(),
                                visiting,
                                memo,
                                expected,
                            },
                        ));
                        if outcomes.len() == before_recovery {
                            outcomes.extend(self.consuming_failure_fallback(
                                ConsumingFailureFallback {
                                    atn,
                                    target: *target,
                                    request: recovery_request,
                                    symbol,
                                    expected_symbols,
                                    decision_start_index: next_decision_start_index,
                                    decision,
                                },
                                visiting,
                                memo,
                                expected,
                            ));
                        }
                    }
                }
            }
        }

        visiting.remove(&visit_key);
        self.record_prediction_diagnostics(atn, state, index, &outcomes);
        if self.prediction_mode == PredictionMode::Ll {
            discard_recovered_outcomes_if_clean_path_exists(&mut outcomes);
        }
        dedupe_outcomes(&mut outcomes);
        memo.insert(key, outcomes.clone());
        outcomes
    }

    /// Follows an epsilon or semantic-action transition while preserving the
    /// path-local side effects that may later become generated action output.
    fn recognize_epsilon_or_action_step(
        &mut self,
        atn: &Atn,
        request: &RecognizeRequest<'_>,
        step: EpsilonActionStep,
        scratch: RecognizeScratch<'_>,
    ) -> Vec<RecognizeOutcome> {
        let RecognizeScratch {
            visiting,
            memo,
            expected,
        } = scratch;
        let action = step.action_rule_index.map(|rule_index| {
            ParserAction::new(
                step.source_state,
                rule_index,
                request.rule_start_index,
                self.previous_token_index(request.index),
            )
        });
        let next_member_values = if action.is_some() {
            member_values_after_action(
                step.source_state,
                request.member_actions,
                &request.member_values,
            )
        } else {
            request.member_values.clone()
        };
        let next_return_values = action.map_or_else(
            || request.return_values.clone(),
            |action| {
                return_values_after_action(
                    step.source_state,
                    action.rule_index(),
                    request.return_actions,
                    &request.return_values,
                )
            },
        );

        self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: step.target,
                stop_state: request.stop_state,
                index: request.index,
                rule_start_index: request.rule_start_index,
                decision_start_index: step.decision_start_index,
                init_action_rules: request.init_action_rules,
                predicates: request.predicates,
                rule_args: request.rule_args,
                member_actions: request.member_actions,
                return_actions: request.return_actions,
                local_int_arg: request.local_int_arg,
                member_values: next_member_values,
                return_values: next_return_values,
                rule_alt_number: step.alt_number,
                track_alt_numbers: request.track_alt_numbers,
                precedence: request.precedence,
                depth: request.depth + 1,
                recovery_symbols: step.recovery_symbols,
                recovery_state: step.recovery_state,
            },
            visiting,
            memo,
            expected,
        )
        .into_iter()
        .map(|mut outcome| {
            prepend_decision(&mut outcome, step.decision);
            if let Some(rule_index) = step.left_recursive_boundary {
                outcome
                    .nodes
                    .insert(0, RecognizedNode::LeftRecursiveBoundary { rule_index });
            }
            if let Some(action) = action {
                outcome.actions.insert(0, action);
            }
            outcome
        })
        .collect()
    }

    /// Reads the token type at an absolute token-stream index.
    fn token_type_at(&mut self, index: usize) -> i32 {
        self.input.seek(index);
        self.input.la_token(1)
    }

    /// Clones the visible token at an absolute token-stream index.
    fn token_at(&mut self, index: usize) -> Option<CommonToken> {
        self.input.get(index).cloned()
    }

    /// Reports whether a child rule reached EOF cleanly while also recording
    /// an EOF expectation from a longer path inside that child.
    fn child_expected_reaches_clean_eof(
        &mut self,
        children: &[RecognizeOutcome],
        expected: &ExpectedTokens,
    ) -> bool {
        let Some(index) = expected.index else {
            return false;
        };
        self.token_type_at(index) == TOKEN_EOF
            && children
                .iter()
                .any(|child| child.diagnostics.is_empty() && child.index == index)
    }

    /// Finds the previous token visible to the parser before `index`.
    ///
    /// The token stream cursor skips hidden-channel tokens, so subtracting one
    /// from a visible-token index can point at whitespace. Parser intervals use
    /// this helper to stop at the previous visible token while preserving hidden
    /// text inside the rendered interval.
    fn previous_token_index(&mut self, index: usize) -> Option<usize> {
        self.input.previous_visible_token_index(index)
    }

    /// Evaluates a supported parser predicate at a speculative input index.
    ///
    /// Parser ATN simulation is index-based, so predicate evaluation seeks to
    /// the candidate index before applying lookahead. A missing predicate entry
    /// means the generator did not opt into runtime evaluation for that
    /// coordinate and the transition remains viable.
    fn parser_predicate_matches(&mut self, eval: PredicateEval<'_>) -> bool {
        let PredicateEval {
            index,
            rule_index,
            pred_index,
            predicates,
            local_int_arg,
            member_values,
        } = eval;
        let Some((_, _, predicate)) = predicates
            .iter()
            .find(|(rule, pred, _)| *rule == rule_index && *pred == pred_index)
        else {
            return true;
        };
        self.input.seek(index);
        match predicate {
            ParserPredicate::True => true,
            ParserPredicate::False => false,
            ParserPredicate::Invoke { value } => {
                let key = (rule_index, pred_index);
                if !self.invoked_predicates.contains(&key) {
                    self.invoked_predicates.push(key);
                    use std::io::Write as _;
                    let mut stdout = std::io::stdout().lock();
                    let _ = writeln!(stdout, "eval={value}");
                }
                *value
            }
            ParserPredicate::LookaheadTextEquals { offset, text } => {
                self.input.lt(*offset).and_then(Token::text) == Some(*text)
            }
            ParserPredicate::LookaheadNotEquals { offset, token_type } => {
                self.la(*offset) != *token_type
            }
            ParserPredicate::LocalIntEquals { value } => {
                local_int_arg.is_none_or(|(_, actual)| actual == *value)
            }
            ParserPredicate::MemberModuloEquals {
                member,
                modulus,
                value,
                equals,
            } => {
                if *modulus == 0 {
                    return false;
                }
                let actual = member_values.get(member).copied().unwrap_or_default() % *modulus;
                (actual == *value) == *equals
            }
        }
    }

    /// Returns the token-stream index after consuming `symbol` at `index`.
    ///
    /// EOF is not advanced by ANTLR token streams, so EOF transitions keep the
    /// index stable and rely on `consumed_eof` to record that EOF was matched.
    fn consume_index(&mut self, index: usize, symbol: i32) -> usize {
        self.input.seek(index);
        if symbol != TOKEN_EOF {
            self.consume();
        }
        self.input.index()
    }

    /// Builds ANTLR's no-viable-alternative diagnostic for an ambiguous
    /// decision that failed after consuming a shared prefix.
    fn no_viable_alternative(
        &mut self,
        start_index: usize,
        error_index: usize,
    ) -> ParserDiagnostic {
        let text = display_input_text(&self.input.text(start_index, error_index));
        diagnostic_for_token(
            self.token_at(error_index).as_ref(),
            format!("no viable alternative at input '{text}'"),
        )
    }

    /// Selects the diagnostic for a failed consuming transition after all
    /// recovery repairs have been ruled out.
    fn recovery_failure_diagnostic(
        &mut self,
        index: usize,
        decision_start_index: Option<usize>,
        expected_symbols: &BTreeSet<i32>,
    ) -> ParserDiagnostic {
        if expected_symbols.len() > 1 {
            if let Some(decision_start) = no_viable_decision_start(decision_start_index, index) {
                return self.no_viable_alternative(decision_start, index);
            }
        }
        diagnostic_for_token(
            self.token_at(index).as_ref(),
            format!(
                "mismatched input {} expecting {}",
                self.token_at(index)
                    .as_ref()
                    .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
                self.expected_symbols_display(expected_symbols)
            ),
        )
    }

    /// Builds the EOF diagnostic used when ANTLR unwinds a failed nested rule
    /// instead of inserting missing tokens in the caller.
    fn eof_rule_recovery_diagnostic(
        &mut self,
        index: usize,
        expected_symbols: &BTreeSet<i32>,
        expected: &ExpectedTokens,
    ) -> ParserDiagnostic {
        let symbols = if expected.index == Some(index) && !expected.symbols.is_empty() {
            &expected.symbols
        } else {
            expected_symbols
        };
        diagnostic_for_token(
            self.token_at(index).as_ref(),
            format!(
                "mismatched input {} expecting {}",
                self.token_at(index)
                    .as_ref()
                    .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
                self.expected_symbols_display(symbols)
            ),
        )
    }

    /// Returns token text for a buffered token interval.
    pub fn text_interval(&mut self, start: usize, stop: Option<usize>) -> String {
        stop.map_or_else(String::new, |stop| self.input.text(start, stop))
    }

    /// Resets per-parse prediction diagnostics while keeping the parser-level
    /// reporting flag configured by generated harness code.
    fn clear_prediction_diagnostics(&mut self) {
        self.prediction_diagnostics.clear();
        self.reported_prediction_diagnostics.clear();
    }

    /// Buffers ANTLR-style diagnostic-listener messages for decision states
    /// where multiple clean alternatives survive full-context recognition.
    fn record_prediction_diagnostics(
        &mut self,
        atn: &Atn,
        state: &AtnState,
        start_index: usize,
        outcomes: &[RecognizeOutcome],
    ) {
        if !self.report_diagnostic_errors || state.transitions.len() < 2 {
            return;
        }
        let Some(decision) = atn
            .decision_to_state()
            .iter()
            .position(|state_number| *state_number == state.state_number)
        else {
            return;
        };
        let Some(rule_index) = state.rule_index else {
            return;
        };
        let mut alts_by_end = BTreeMap::<usize, BTreeSet<usize>>::new();
        for outcome in outcomes
            .iter()
            .filter(|outcome| outcome.diagnostics.is_empty())
        {
            let Some(alt) = outcome.decisions.first() else {
                continue;
            };
            alts_by_end
                .entry(outcome.index)
                .or_default()
                .insert(alt + 1);
        }
        let Some((&end_index, ambig_alts)) = alts_by_end
            .iter()
            .filter(|(_, alts)| alts.len() > 1)
            .max_by_key(|(end, _)| *end)
        else {
            return;
        };
        let rule_name = self
            .rule_names()
            .get(rule_index)
            .map_or_else(|| "<unknown>".to_owned(), Clone::clone);
        let stop_index = self.previous_token_index(end_index).unwrap_or(start_index);
        let input = display_input_text(&self.input.text(start_index, stop_index));
        let alts = ambig_alts
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let key = (decision, start_index, format!("{alts}:{input}"));
        if !self.reported_prediction_diagnostics.insert(key) {
            return;
        }
        let start_token = self.token_at(start_index);
        let stop_token = self.token_at(stop_index);
        self.prediction_diagnostics.push(diagnostic_for_token(
            start_token.as_ref(),
            format!("reportAttemptingFullContext d={decision} ({rule_name}), input='{input}'"),
        ));
        self.prediction_diagnostics.push(diagnostic_for_token(
            stop_token.as_ref(),
            format!(
                "reportAmbiguity d={decision} ({rule_name}): ambigAlts={{{alts}}}, input='{input}'"
            ),
        ));
    }

    /// Formats the tokens expected from an ATN state using ANTLR display names.
    pub fn expected_tokens_at_state(&self, atn: &Atn, state_number: usize) -> String {
        expected_symbols_display(
            &state_expected_symbols(atn, state_number),
            self.vocabulary(),
        )
    }

    /// Formats a buffered token in ANTLR's diagnostic token display form.
    pub fn token_display_at(&mut self, index: usize) -> Option<String> {
        self.token_at(index).map(|token| format!("{token}"))
    }

    /// Converts a recognized internal node into a public parse-tree node.
    fn recognized_node_tree(
        &mut self,
        node: &RecognizedNode,
        track_alt_numbers: bool,
    ) -> Result<ParseTree, AntlrError> {
        match node {
            RecognizedNode::Token { index } => {
                let token =
                    self.input
                        .get(*index)
                        .cloned()
                        .ok_or_else(|| AntlrError::ParserError {
                            line: 0,
                            column: 0,
                            message: format!("missing token at index {index}"),
                        })?;
                Ok(ParseTree::Terminal(TerminalNode::new(token)))
            }
            RecognizedNode::ErrorToken { index } => {
                let token =
                    self.input
                        .get(*index)
                        .cloned()
                        .ok_or_else(|| AntlrError::ParserError {
                            line: 0,
                            column: 0,
                            message: format!("missing error token at index {index}"),
                        })?;
                Ok(ParseTree::Error(ErrorNode::new(token)))
            }
            RecognizedNode::MissingToken {
                token_type,
                at_index,
                text,
            } => {
                let current = self.token_at(*at_index);
                let token = CommonToken::new(*token_type)
                    .with_text(text)
                    .with_span(usize::MAX, usize::MAX)
                    .with_position(
                        current.as_ref().map(Token::line).unwrap_or_default(),
                        current.as_ref().map(Token::column).unwrap_or_default(),
                    );
                Ok(ParseTree::Error(ErrorNode::new(token)))
            }
            RecognizedNode::Rule {
                rule_index,
                invoking_state,
                alt_number,
                start_index,
                stop_index,
                return_values,
                children,
            } => {
                let mut context = ParserRuleContext::new(*rule_index, *invoking_state);
                if track_alt_numbers {
                    context.set_alt_number(*alt_number);
                }
                for (name, value) in return_values {
                    context.set_int_return(name.clone(), *value);
                }
                if let Some(token) = self.token_at(*start_index) {
                    context.set_start(token);
                }
                if let Some(token) = stop_index.and_then(|index| self.token_at(index)) {
                    context.set_stop(token);
                }
                for child in children {
                    context.add_child(self.recognized_node_tree(child, track_alt_numbers)?);
                }
                Ok(self.rule_node(context))
            }
            RecognizedNode::LeftRecursiveBoundary { rule_index } => Err(AntlrError::Unsupported(
                format!("unfolded left-recursive boundary for rule {rule_index}"),
            )),
        }
    }
}

/// Detects the loop edge where ANTLR would call `pushNewRecursionContext` for a
/// transformed left-recursive rule.
fn left_recursive_boundary(atn: &Atn, state: &AtnState, target: usize) -> Option<usize> {
    if !state.precedence_rule_decision {
        return None;
    }
    let target_state = atn.state(target)?;
    if target_state.kind == AtnStateKind::LoopEnd {
        return None;
    }
    state.rule_index
}

/// Selects the first outer alternative observed for a rule path.
///
/// ANTLR's alt-numbered tree contexts store the rule alternative chosen at the
/// outer decision. The metadata recognizer only needs this when a generated
/// grammar opts into that target template; otherwise the value remains `0` and
/// parse-tree rendering is unchanged.
const fn next_alt_number(
    state: &AtnState,
    transition_index: usize,
    current_alt_number: usize,
    track_alt_numbers: bool,
) -> usize {
    if !track_alt_numbers || current_alt_number != 0 || state.transitions.len() <= 1 {
        return current_alt_number;
    }
    if matches!(
        state.kind,
        AtnStateKind::Basic
            | AtnStateKind::BlockStart
            | AtnStateKind::PlusBlockStart
            | AtnStateKind::StarBlockStart
            | AtnStateKind::StarLoopEntry
    ) && !state.precedence_rule_decision
    {
        return transition_index + 1;
    }
    current_alt_number
}

/// Folds boundary markers emitted at precedence-loop entries into nested rule
/// nodes, matching ANTLR's recursive-context parse-tree shape.
fn fold_left_recursive_boundaries(nodes: Vec<RecognizedNode>) -> Vec<RecognizedNode> {
    let mut folded = Vec::new();
    for node in nodes {
        match node {
            RecognizedNode::LeftRecursiveBoundary { rule_index } => {
                if !folded.is_empty() {
                    let children = std::mem::take(&mut folded);
                    let start_index = recognized_nodes_start_index(&children).unwrap_or_default();
                    let stop_index = recognized_nodes_stop_index(&children);
                    folded.push(RecognizedNode::Rule {
                        rule_index,
                        invoking_state: -1,
                        alt_number: 0,
                        start_index,
                        stop_index,
                        return_values: BTreeMap::new(),
                        children,
                    });
                }
            }
            node => folded.push(node),
        }
    }
    folded
}

fn recognized_nodes_start_index(nodes: &[RecognizedNode]) -> Option<usize> {
    nodes.iter().find_map(recognized_node_start_index)
}

const fn recognized_node_start_index(node: &RecognizedNode) -> Option<usize> {
    match node {
        RecognizedNode::Token { index } | RecognizedNode::ErrorToken { index } => Some(*index),
        RecognizedNode::MissingToken { at_index, .. } => Some(*at_index),
        RecognizedNode::Rule { start_index, .. } => Some(*start_index),
        RecognizedNode::LeftRecursiveBoundary { .. } => None,
    }
}

fn recognized_nodes_stop_index(nodes: &[RecognizedNode]) -> Option<usize> {
    nodes.iter().rev().find_map(recognized_node_stop_index)
}

/// Converts an ATN state number into the signed invoking-state slot used by
/// ANTLR parse-tree contexts, saturating only for impossible platform widths.
fn invoking_state_number(state_number: usize) -> isize {
    isize::try_from(state_number).unwrap_or(isize::MAX)
}

const fn recognized_node_stop_index(node: &RecognizedNode) -> Option<usize> {
    match node {
        RecognizedNode::Token { index } | RecognizedNode::ErrorToken { index } => Some(*index),
        RecognizedNode::MissingToken { at_index, .. } => at_index.checked_sub(1),
        RecognizedNode::Rule { stop_index, .. } => *stop_index,
        RecognizedNode::LeftRecursiveBoundary { .. } => None,
    }
}

fn token_input_display(token: &impl Token) -> String {
    format!("'{}'", token.text().unwrap_or("<EOF>"))
}

fn display_input_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

fn diagnostic_for_token(token: Option<&impl Token>, message: String) -> ParserDiagnostic {
    ParserDiagnostic {
        line: token.map(Token::line).unwrap_or_default(),
        column: token.map(Token::column).unwrap_or_default(),
        message,
    }
}

/// Emits parser diagnostics for the selected recovered parse path.
#[allow(clippy::print_stderr)]
fn report_parser_diagnostics(diagnostics: &[ParserDiagnostic]) {
    for diagnostic in diagnostics {
        eprintln!(
            "line {}:{} {}",
            diagnostic.line, diagnostic.column, diagnostic.message
        );
    }
}

/// Emits buffered token-source diagnostics after parser diagnostics that were
/// discovered while speculatively reading the same token stream.
#[allow(clippy::print_stderr)]
fn report_token_source_errors(errors: &[TokenSourceError]) {
    for error in errors {
        eprintln!("line {}:{} {}", error.line, error.column, error.message);
    }
}

fn expected_symbols_display(symbols: &BTreeSet<i32>, vocabulary: &Vocabulary) -> String {
    let items = symbols
        .iter()
        .map(|symbol| expected_symbol_display(*symbol, vocabulary))
        .collect::<Vec<_>>();
    if let [single] = items.as_slice() {
        return single.clone();
    }
    format!("{{{}}}", items.join(", "))
}

fn expected_symbol_display(symbol: i32, vocabulary: &Vocabulary) -> String {
    if symbol == TOKEN_EOF {
        return "<EOF>".to_owned();
    }
    vocabulary.display_name(symbol)
}

/// Returns whether `state` belongs to an ANTLR-transformed left-recursive rule.
/// Inline insertion in those precedence loops can synthesize a missing operand
/// before an operator and then block the legitimate loop-exit path.
fn state_is_left_recursive_rule(atn: &Atn, state: &AtnState) -> bool {
    let Some(rule_index) = state.rule_index else {
        return false;
    };
    atn.rule_to_start_state()
        .get(rule_index)
        .and_then(|state_number| atn.state(*state_number))
        .is_some_and(|rule_start| rule_start.left_recursive_rule)
}

/// Chooses the outermost parse result that consumed the most input.
///
/// The recognizer intentionally keeps shorter endpoints available while walking
/// nested rule transitions so callers can satisfy following tokens such as
/// `expr 'and' expr`. Only the public rule entry commits to one endpoint.
fn select_best_fast_outcome(
    outcomes: impl Iterator<Item = FastRecognizeOutcome>,
) -> Option<FastRecognizeOutcome> {
    outcomes.reduce(|best, outcome| {
        if outcome_is_better(
            (outcome.index, outcome.consumed_eof),
            &outcome.diagnostics,
            (best.index, best.consumed_eof),
            &best.diagnostics,
        ) {
            return outcome;
        }
        best
    })
}

fn select_best_outcome(
    outcomes: impl Iterator<Item = RecognizeOutcome>,
    prediction_mode: PredictionMode,
) -> Option<RecognizeOutcome> {
    let outcomes = outcomes.collect::<Vec<_>>();
    let prefer_first_tie = outcomes
        .iter()
        .any(|outcome| nodes_need_stable_tie(&outcome.nodes));
    outcomes.into_iter().reduce(|best, outcome| {
        let outcome_position = (outcome.index, outcome.consumed_eof);
        let best_position = (best.index, best.consumed_eof);
        let better = match prediction_mode {
            PredictionMode::Ll => {
                outcome_is_better(
                    outcome_position,
                    &outcome.diagnostics,
                    best_position,
                    &best.diagnostics,
                ) || (!prefer_first_tie
                    && outcome_position == best_position
                    && outcome.diagnostics.len() == best.diagnostics.len()
                    && diagnostic_recovery_rank(&outcome.diagnostics)
                        == diagnostic_recovery_rank(&best.diagnostics)
                    && (outcome.decisions < best.decisions
                        || (outcome.decisions == best.decisions && outcome.actions > best.actions)))
            }
            PredictionMode::Sll => {
                outcome_position > best_position
                    || (outcome_position == best_position
                        && !prefer_first_tie
                        && (outcome.decisions < best.decisions
                            || (outcome.decisions == best.decisions
                                && outcome_is_better(
                                    outcome_position,
                                    &outcome.diagnostics,
                                    best_position,
                                    &best.diagnostics,
                                ))))
            }
        };
        if better {
            return outcome;
        }
        best
    })
}

/// Records the serialized transition order at parser decision states.
///
/// When two clean paths consume the same input, ANTLR's adaptive prediction
/// chooses by alternative order. Keeping this compact trace lets the metadata
/// recognizer distinguish greedy and non-greedy optional blocks without a full
/// prediction simulator.
fn transition_decision(
    atn: &Atn,
    state: &AtnState,
    transition_index: usize,
    predicates: &[(usize, usize, ParserPredicate)],
) -> Option<usize> {
    if state.transitions.len() <= 1
        || state.precedence_rule_decision
        || decision_reaches_unsupported_predicate(atn, state, predicates)
    {
        return None;
    }
    Some(transition_index)
}

/// Reports whether a state should reset the active no-viable decision start.
///
/// Loop entry/back states are continuations of the surrounding adaptive
/// prediction; resetting at those states would turn LL-star failures back into
/// ordinary mismatches.
const fn starts_prediction_decision(state: &AtnState) -> bool {
    state.transitions.len() > 1
        && !matches!(
            state.kind,
            AtnStateKind::PlusLoopBack | AtnStateKind::StarLoopBack | AtnStateKind::StarLoopEntry
        )
}

/// Marks a farthest expected-token set as no-viable when multiple alternatives
/// failed after the active decision had already consumed input.
fn record_no_viable_if_ambiguous(
    expected: &mut ExpectedTokens,
    decision_start_index: Option<usize>,
    index: usize,
) {
    if expected.index == Some(index) && expected.symbols.len() > 1 {
        if let Some(decision_start) = no_viable_decision_start(decision_start_index, index) {
            expected.record_no_viable(decision_start, index);
        }
    }
}

/// Records a no-viable decision caused by a failed semantic predicate before
/// any consuming transition can contribute an expected-token set.
const fn record_predicate_no_viable(
    expected: &mut ExpectedTokens,
    decision_start_index: Option<usize>,
    index: usize,
) {
    if let Some(decision_start) = decision_start_index {
        expected.record_no_viable(decision_start, index);
    }
}

/// Returns the active decision start only when the error is past that start.
const fn no_viable_decision_start(
    decision_start_index: Option<usize>,
    index: usize,
) -> Option<usize> {
    match decision_start_index {
        Some(start) if index > start => Some(start),
        _ => None,
    }
}

/// Restores expected-token bookkeeping when a child rule found a clean
/// consuming path; failures in longer child alternatives should not pollute the
/// caller's final expectation set.
fn restore_expected(
    children: &[RecognizeOutcome],
    child_start_index: usize,
    expected: &mut ExpectedTokens,
    snapshot: ExpectedTokens,
    preserve_child_expected: bool,
) {
    if preserve_child_expected {
        return;
    }
    if children
        .iter()
        .any(|child| child.diagnostics.is_empty() && child.index > child_start_index)
    {
        *expected = snapshot;
    }
}

/// Reports whether a decision can reach a predicate the generator did not
/// translate. Static alternative order is unsafe for those context predicates.
fn decision_reaches_unsupported_predicate(
    atn: &Atn,
    state: &AtnState,
    predicates: &[(usize, usize, ParserPredicate)],
) -> bool {
    state.transitions.iter().any(|transition| {
        transition_reaches_unsupported_predicate(atn, transition, predicates, &mut BTreeSet::new())
    })
}

/// Walks epsilon-like edges from one transition to find unsupported predicates.
fn transition_reaches_unsupported_predicate(
    atn: &Atn,
    transition: &Transition,
    predicates: &[(usize, usize, ParserPredicate)],
    visited: &mut BTreeSet<usize>,
) -> bool {
    match transition {
        Transition::Predicate {
            rule_index,
            pred_index,
            ..
        } => !predicates
            .iter()
            .any(|(rule, pred, _)| rule == rule_index && pred == pred_index),
        Transition::Epsilon { target }
        | Transition::Action { target, .. }
        | Transition::Rule { target, .. } => {
            state_reaches_unsupported_predicate(atn, *target, predicates, visited)
        }
        Transition::Precedence { .. }
        | Transition::Atom { .. }
        | Transition::Range { .. }
        | Transition::Set { .. }
        | Transition::NotSet { .. }
        | Transition::Wildcard { .. } => false,
    }
}

/// Finds an unsupported predicate reachable before a consuming transition.
fn state_reaches_unsupported_predicate(
    atn: &Atn,
    state_number: usize,
    predicates: &[(usize, usize, ParserPredicate)],
    visited: &mut BTreeSet<usize>,
) -> bool {
    if !visited.insert(state_number) {
        return false;
    }
    let Some(state) = atn.state(state_number) else {
        return false;
    };
    state.transitions.iter().any(|transition| {
        transition_reaches_unsupported_predicate(atn, transition, predicates, visited)
    })
}

/// Adds a decision step to the front of an already-recognized suffix path.
fn prepend_decision(outcome: &mut RecognizeOutcome, decision: Option<usize>) {
    if let Some(decision) = decision {
        outcome.decisions.insert(0, decision);
    }
}

fn outcome_is_better(
    outcome_position: (usize, bool),
    outcome_diagnostics: &[ParserDiagnostic],
    best_position: (usize, bool),
    best_diagnostics: &[ParserDiagnostic],
) -> bool {
    outcome_position > best_position
        || (outcome_position == best_position
            && (outcome_diagnostics.len() < best_diagnostics.len()
                || (outcome_diagnostics.len() == best_diagnostics.len()
                    && diagnostic_recovery_rank(outcome_diagnostics)
                        < diagnostic_recovery_rank(best_diagnostics))))
}

/// Ranks concrete recovery repairs ahead of generic non-EOF mismatch fallbacks
/// when speculative paths otherwise consume the same input.
fn diagnostic_recovery_rank(diagnostics: &[ParserDiagnostic]) -> usize {
    diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.message.starts_with("mismatched input ")
                && !diagnostic.message.starts_with("mismatched input '<EOF>' ")
        })
        .count()
}

fn discard_recovered_fast_outcomes_if_clean_path_exists(outcomes: &mut Vec<FastRecognizeOutcome>) {
    if outcomes
        .iter()
        .any(|outcome| outcome.diagnostics.is_empty())
    {
        outcomes.retain(|outcome| outcome.diagnostics.is_empty());
    }
}

fn discard_recovered_outcomes_if_clean_path_exists(outcomes: &mut Vec<RecognizeOutcome>) {
    if outcomes
        .iter()
        .any(|outcome| outcome.diagnostics.is_empty())
    {
        outcomes.retain(|outcome| outcome.diagnostics.is_empty());
    }
}

/// Reports whether a candidate contains recursive tree structure where ANTLR's
/// first viable candidate preserves the correct left-recursive context shape.
fn nodes_need_stable_tie(nodes: &[RecognizedNode]) -> bool {
    nodes.iter().any(node_needs_stable_tie)
}

fn node_needs_stable_tie(node: &RecognizedNode) -> bool {
    match node {
        RecognizedNode::Token { .. }
        | RecognizedNode::ErrorToken { .. }
        | RecognizedNode::MissingToken { .. } => false,
        RecognizedNode::LeftRecursiveBoundary { .. } => true,
        RecognizedNode::Rule {
            rule_index,
            children,
            ..
        } => children.iter().any(|child| {
            matches!(
                child,
                RecognizedNode::Rule {
                    rule_index: child_rule,
                    ..
                } if child_rule == rule_index
            ) || node_needs_stable_tie(child)
        }),
    }
}

/// Sorts and removes equivalent endpoints before memoizing a state result.
fn dedupe_fast_outcomes(outcomes: &mut Vec<FastRecognizeOutcome>) {
    outcomes.sort_unstable();
    outcomes.dedup();
}

/// Sorts and removes equivalent endpoints, including their action traces.
fn dedupe_outcomes(outcomes: &mut Vec<RecognizeOutcome>) {
    outcomes.sort_unstable();
    outcomes.dedup();
}

impl<S> Recognizer for BaseParser<S>
where
    S: TokenSource,
{
    fn data(&self) -> &RecognizerData {
        &self.data
    }

    fn data_mut(&mut self) -> &mut RecognizerData {
        &mut self.data
    }
}

impl<S> Parser for BaseParser<S>
where
    S: TokenSource,
{
    fn build_parse_trees(&self) -> bool {
        self.build_parse_trees
    }

    fn set_build_parse_trees(&mut self, build: bool) {
        self.build_parse_trees = build;
    }

    fn report_diagnostic_errors(&self) -> bool {
        self.report_diagnostic_errors
    }

    fn set_report_diagnostic_errors(&mut self, report: bool) {
        self.report_diagnostic_errors = report;
    }

    fn prediction_mode(&self) -> PredictionMode {
        self.prediction_mode
    }

    fn set_prediction_mode(&mut self, mode: PredictionMode) {
        self.prediction_mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atn::serialized::{AtnDeserializer, SerializedAtn};
    use crate::token::CommonToken;
    use crate::token_stream::CommonTokenStream;
    use crate::vocabulary::Vocabulary;

    #[derive(Debug)]
    struct Source {
        tokens: Vec<CommonToken>,
        index: usize,
    }

    impl TokenSource for Source {
        fn next_token(&mut self) -> CommonToken {
            let token = self
                .tokens
                .get(self.index)
                .cloned()
                .unwrap_or_else(|| CommonToken::eof("parser-test", self.index, 1, self.index));
            self.index += 1;
            token
        }

        fn line(&self) -> usize {
            1
        }

        fn column(&self) -> usize {
            self.index
        }

        fn source_name(&self) -> &'static str {
            "parser-test"
        }
    }

    #[test]
    fn parser_matches_token_and_reports_mismatch() {
        let source = Source {
            tokens: vec![
                CommonToken::new(1).with_text("x"),
                CommonToken::eof("parser-test", 1, 1, 1),
            ],
            index: 0,
        };
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new([None, Some("'x'")], [None, Some("X")], [None::<&str>, None]),
        );
        let mut parser = BaseParser::new(CommonTokenStream::new(source), data);
        assert_eq!(
            parser.match_token(1).expect("token 1 should match").text(),
            "x"
        );
        assert!(parser.match_token(1).is_err());
    }

    #[test]
    fn parser_interprets_simple_atn_rule() {
        let atn = AtnDeserializer::new(&SerializedAtn::from_i32([
            4, 1, 2, // version, parser, max token type
            3, // states
            2, 0, // rule start
            1, 0, // basic
            7, 0, // rule stop
            0, // non-greedy states
            0, // precedence states
            1, // rules
            0, // rule 0 start
            0, // modes
            0, // sets
            2, // transitions
            0, 1, 5, 1, 0, 0, // match token 1
            1, 2, 5, -1, 0, 0, // match EOF
            0, // decisions
        ]))
        .deserialize()
        .expect("artificial parser ATN should deserialize");
        let source = Source {
            tokens: vec![
                CommonToken::new(1).with_text("x"),
                CommonToken::eof("parser-test", 1, 1, 1),
            ],
            index: 0,
        };
        let data = RecognizerData::new(
            "Mini.g4",
            Vocabulary::new([None, Some("'x'")], [None, Some("X")], [None::<&str>, None]),
        );
        let mut parser = BaseParser::new(CommonTokenStream::new(source), data);

        let tree = parser
            .parse_atn_rule(&atn, 0)
            .expect("artificial parser rule should parse");
        assert_eq!(tree.text(), "x<EOF>");
    }

    #[test]
    fn folds_left_recursive_boundary_into_rule_node() {
        let nodes = fold_left_recursive_boundaries(vec![
            RecognizedNode::Token { index: 0 },
            RecognizedNode::LeftRecursiveBoundary { rule_index: 1 },
            RecognizedNode::Token { index: 1 },
        ]);

        assert_eq!(
            nodes,
            vec![
                RecognizedNode::Rule {
                    rule_index: 1,
                    invoking_state: -1,
                    alt_number: 0,
                    start_index: 0,
                    stop_index: Some(0),
                    return_values: BTreeMap::new(),
                    children: vec![RecognizedNode::Token { index: 0 }],
                },
                RecognizedNode::Token { index: 1 },
            ]
        );
    }

    #[test]
    fn outcome_ties_keep_later_non_recursive_alternative() {
        let first = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: Vec::new(),
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: vec![RecognizedNode::Token { index: 0 }],
        };
        let second = RecognizeOutcome {
            actions: vec![ParserAction::new(2, 0, 0, None)],
            ..first.clone()
        };

        let selected = select_best_outcome([first, second].into_iter(), PredictionMode::Ll)
            .expect("one outcome should be selected");
        assert_eq!(selected.actions[0].source_state(), 2);
    }

    #[test]
    fn outcome_ties_prefer_more_actions_for_non_recursive_paths() {
        let first = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: Vec::new(),
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: vec![RecognizedNode::Token { index: 0 }],
        };
        let second = RecognizeOutcome {
            actions: vec![
                ParserAction::new(2, 0, 0, None),
                ParserAction::new(3, 0, 0, None),
            ],
            ..first.clone()
        };

        let selected = select_best_outcome([second, first].into_iter(), PredictionMode::Ll)
            .expect("one outcome should be selected");
        assert_eq!(selected.actions.len(), 2);
    }

    #[test]
    fn outcome_ties_prefer_later_action_stop_for_greedy_optional_paths() {
        let first = RecognizeOutcome {
            index: 7,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: vec![1, 0],
            actions: vec![
                ParserAction::new(23, 2, 2, Some(4)),
                ParserAction::new(23, 2, 0, Some(6)),
            ],
            nodes: vec![RecognizedNode::Token { index: 0 }],
        };
        let second = RecognizeOutcome {
            decisions: vec![0, 1],
            actions: vec![
                ParserAction::new(23, 2, 2, Some(6)),
                ParserAction::new(23, 2, 0, Some(6)),
            ],
            ..first.clone()
        };

        let selected = select_best_outcome([first, second].into_iter(), PredictionMode::Ll)
            .expect("one outcome should be selected");
        assert_eq!(selected.actions[0].stop_index(), Some(6));
    }

    #[test]
    fn outcome_ties_keep_first_recursive_tree_shape() {
        let recursive_nodes = vec![RecognizedNode::Rule {
            rule_index: 1,
            invoking_state: -1,
            alt_number: 0,
            start_index: 0,
            stop_index: Some(0),
            return_values: BTreeMap::new(),
            children: vec![RecognizedNode::Rule {
                rule_index: 1,
                invoking_state: -1,
                alt_number: 0,
                start_index: 0,
                stop_index: Some(0),
                return_values: BTreeMap::new(),
                children: vec![RecognizedNode::Token { index: 0 }],
            }],
        }];
        let first = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: Vec::new(),
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: recursive_nodes.clone(),
        };
        let second = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: Vec::new(),
            decisions: Vec::new(),
            actions: vec![ParserAction::new(2, 0, 0, None)],
            nodes: recursive_nodes,
        };

        let selected = select_best_outcome([first, second].into_iter(), PredictionMode::Ll)
            .expect("one outcome should be selected");
        assert_eq!(selected.actions[0].source_state(), 1);
    }

    #[test]
    fn sll_outcome_selection_keeps_earlier_recovered_alt() {
        let first_alt = RecognizeOutcome {
            index: 2,
            consumed_eof: true,
            alt_number: 0,
            member_values: BTreeMap::new(),
            return_values: BTreeMap::new(),
            diagnostics: vec![ParserDiagnostic {
                line: 1,
                column: 3,
                message: "missing 'Y' at '<EOF>'".to_owned(),
            }],
            decisions: vec![0],
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: vec![RecognizedNode::Token { index: 0 }],
        };
        let second_alt = RecognizeOutcome {
            diagnostics: Vec::new(),
            decisions: vec![1],
            actions: vec![ParserAction::new(2, 0, 0, None)],
            ..first_alt.clone()
        };

        let selected =
            select_best_outcome([second_alt, first_alt].into_iter(), PredictionMode::Sll)
                .expect("one outcome should be selected");
        assert_eq!(selected.diagnostics.len(), 1);
        assert_eq!(selected.decisions, [0]);
    }
}
