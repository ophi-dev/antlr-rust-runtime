use std::collections::{BTreeMap, BTreeSet};

use crate::atn::{Atn, AtnState, AtnStateKind, Transition};
use crate::errors::AntlrError;
use crate::int_stream::IntStream;
use crate::recognizer::{Recognizer, RecognizerData};
use crate::token::{CommonToken, TOKEN_EOF, Token, TokenSource};
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

pub trait Parser: Recognizer {
    fn build_parse_trees(&self) -> bool;
    fn set_build_parse_trees(&mut self, build: bool);
}

#[derive(Debug)]
pub struct BaseParser<S> {
    input: CommonTokenStream<S>,
    data: RecognizerData,
    build_parse_trees: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecognizeOutcome {
    index: usize,
    consumed_eof: bool,
    alt_number: usize,
    diagnostics: Vec<ParserDiagnostic>,
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
        alt_number: usize,
        start_index: usize,
        stop_index: Option<usize>,
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
}

impl ExpectedTokens {
    /// Records the expected symbols for the farthest token index reached by any
    /// failed ATN path.
    fn record_transition(&mut self, index: usize, transition: &Transition, max_token_type: i32) {
        let symbols = transition_expected_symbols(transition, max_token_type);
        if symbols.is_empty() {
            return;
        }
        match self.index {
            Some(current) if index < current => {}
            Some(current) if index == current => self.symbols.extend(symbols),
            _ => {
                self.index = Some(index);
                self.symbols = symbols;
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

/// Carries recovery context through epsilon-only paths. ANTLR reports some
/// recovery diagnostics at the decision state even when the failed consuming
/// transition is nested under block or loop epsilon edges.
fn next_recovery_symbols(atn: &Atn, state: &AtnState, inherited: &BTreeSet<i32>) -> BTreeSet<i32> {
    let state_symbols = state_expected_symbols(atn, state.state_number);
    if state.transitions.len() > 1 && !state_symbols.is_empty() {
        return state_symbols;
    }
    inherited.clone()
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecognizeRequest<'a> {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    init_action_rules: &'a BTreeSet<usize>,
    rule_alt_number: usize,
    track_alt_numbers: bool,
    /// Current left-recursive precedence threshold, matching ANTLR's
    /// `precpred(_ctx, k)` check for generated precedence rules.
    precedence: i32,
    depth: usize,
    recovery_symbols: BTreeSet<i32>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecognizeKey {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    rule_alt_number: usize,
    track_alt_numbers: bool,
    precedence: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FastRecognizeRequest {
    state_number: usize,
    stop_state: usize,
    index: usize,
    precedence: i32,
    depth: usize,
    recovery_symbols: BTreeSet<i32>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FastRecognizeKey {
    state_number: usize,
    stop_state: usize,
    index: usize,
    precedence: i32,
}

struct FastRecoveryRequest<'a, 'b> {
    atn: &'a Atn,
    transition: &'a Transition,
    expected_symbols: BTreeSet<i32>,
    target: usize,
    request: FastRecognizeRequest,
    visiting: &'b mut BTreeSet<(usize, usize, usize, i32)>,
    memo: &'b mut BTreeMap<FastRecognizeKey, Vec<FastRecognizeOutcome>>,
    expected: &'b mut ExpectedTokens,
}

struct RecoveryRequest<'a, 'b> {
    atn: &'a Atn,
    transition: &'a Transition,
    expected_symbols: BTreeSet<i32>,
    target: usize,
    request: RecognizeRequest<'a>,
    visiting: &'b mut BTreeSet<(usize, usize, usize, usize, i32)>,
    memo: &'b mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
    expected: &'b mut ExpectedTokens,
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
        let mut visiting = BTreeSet::new();
        let mut memo = BTreeMap::new();
        let mut expected = ExpectedTokens::default();
        let outcomes = self.recognize_state_fast(
            atn,
            FastRecognizeRequest {
                state_number: start_state,
                stop_state,
                index: start_index,
                precedence: 0,
                depth: 0,
                recovery_symbols: BTreeSet::new(),
            },
            &mut visiting,
            &mut memo,
            &mut expected,
        );
        let Some(outcome) = select_best_fast_outcome(outcomes.into_iter()) else {
            return Err(self.recognition_error(rule_index, &expected));
        };

        report_parser_diagnostics(&outcome.diagnostics);
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
        let init_action_rules = init_action_rules.iter().copied().collect::<BTreeSet<_>>();
        let mut visiting = BTreeSet::new();
        let mut memo = BTreeMap::new();
        let mut expected = ExpectedTokens::default();
        let outcomes = self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: start_state,
                stop_state,
                index: start_index,
                rule_start_index: start_index,
                init_action_rules: &init_action_rules,
                rule_alt_number: 0,
                track_alt_numbers,
                precedence: 0,
                depth: 0,
                recovery_symbols: BTreeSet::new(),
            },
            &mut visiting,
            &mut memo,
            &mut expected,
        );
        let Some(outcome) = select_best_outcome(outcomes.into_iter()) else {
            return Err(self.recognition_error(rule_index, &expected));
        };

        report_parser_diagnostics(&outcome.diagnostics);
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
    fn recognition_error(&mut self, rule_index: usize, expected: &ExpectedTokens) -> AntlrError {
        let index = expected.index.unwrap_or_else(|| self.input.index());
        self.input.seek(index);
        let current = self.input.lt(1).cloned();
        let line = current.as_ref().map(Token::line).unwrap_or_default();
        let column = current.as_ref().map(Token::column).unwrap_or_default();
        let message = if expected.symbols.is_empty() {
            format!("no viable alternative while parsing rule {rule_index}")
        } else {
            format!(
                "mismatched input {} expecting {}",
                current
                    .as_ref()
                    .map_or_else(|| "'<EOF>'".to_owned(), token_input_display),
                self.expected_symbols_display(&expected.symbols)
            )
        };
        AntlrError::ParserError {
            line,
            column,
            message,
        }
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
        if current_symbol == TOKEN_EOF || !follow_symbols.contains(&current_symbol) {
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
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
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
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
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

    /// Attempts to reach `stop_state` from `state_number` without committing
    /// token consumption to the parser's public stream position.
    fn recognize_state_fast(
        &mut self,
        atn: &Atn,
        request: FastRecognizeRequest,
        visiting: &mut BTreeSet<(usize, usize, usize, i32)>,
        memo: &mut BTreeMap<FastRecognizeKey, Vec<FastRecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<FastRecognizeOutcome> {
        let FastRecognizeRequest {
            state_number,
            stop_state,
            index,
            precedence,
            depth,
            recovery_symbols,
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
            precedence,
        };
        if let Some(outcomes) = memo.get(&key) {
            return outcomes.clone();
        }

        if !visiting.insert((state_number, stop_state, index, precedence)) {
            return Vec::new();
        }

        let Some(state) = atn.state(state_number) else {
            visiting.remove(&(state_number, stop_state, index, precedence));
            return Vec::new();
        };
        let epsilon_recovery_symbols = next_recovery_symbols(atn, state, &recovery_symbols);
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
                            precedence,
                            depth: depth + 1,
                            recovery_symbols: epsilon_recovery_symbols.clone(),
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
                                precedence,
                                depth: depth + 1,
                                recovery_symbols: epsilon_recovery_symbols.clone(),
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
                    let children = self.recognize_state_fast(
                        atn,
                        FastRecognizeRequest {
                            state_number: *target,
                            stop_state: child_stop,
                            index,
                            precedence: *rule_precedence,
                            depth: depth + 1,
                            recovery_symbols: epsilon_recovery_symbols.clone(),
                        },
                        visiting,
                        memo,
                        expected,
                    );
                    for child in children {
                        outcomes.extend(
                            self.recognize_state_fast(
                                atn,
                                FastRecognizeRequest {
                                    state_number: *follow_state,
                                    stop_state,
                                    index: child.index,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
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
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
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
                                    precedence,
                                    depth,
                                    recovery_symbols: recovery_symbols.clone(),
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
                                    expected_symbols,
                                    target: *target,
                                    request: FastRecognizeRequest {
                                        state_number,
                                        stop_state,
                                        index,
                                        precedence,
                                        depth,
                                        recovery_symbols: recovery_symbols.clone(),
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
        }

        visiting.remove(&(state_number, stop_state, index, precedence));
        discard_recovered_fast_outcomes_if_clean_path_exists(&mut outcomes);
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
            init_action_rules,
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
                init_action_rules,
                rule_alt_number,
                track_alt_numbers,
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
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
            init_action_rules,
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
                init_action_rules,
                rule_alt_number,
                track_alt_numbers,
                precedence,
                depth: depth + 1,
                recovery_symbols: BTreeSet::new(),
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
        visiting: &mut BTreeSet<(usize, usize, usize, usize, i32)>,
        memo: &mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        let RecognizeRequest {
            state_number,
            stop_state,
            index,
            rule_start_index,
            init_action_rules,
            rule_alt_number,
            track_alt_numbers,
            precedence,
            depth,
            recovery_symbols,
        } = request;
        if depth > RECOGNITION_DEPTH_LIMIT {
            return Vec::new();
        }
        if state_number == stop_state {
            return vec![RecognizeOutcome {
                index,
                consumed_eof: false,
                alt_number: rule_alt_number,
                diagnostics: Vec::new(),
                actions: Vec::new(),
                nodes: Vec::new(),
            }];
        }
        let key = RecognizeKey {
            state_number,
            stop_state,
            index,
            rule_start_index,
            rule_alt_number,
            track_alt_numbers,
            precedence,
        };
        if let Some(outcomes) = memo.get(&key) {
            return outcomes.clone();
        }

        let visit_key = (
            state_number,
            stop_state,
            index,
            rule_start_index,
            precedence,
        );
        if !visiting.insert(visit_key) {
            return Vec::new();
        }

        let Some(state) = atn.state(state_number) else {
            visiting.remove(&visit_key);
            return Vec::new();
        };
        let epsilon_recovery_symbols = next_recovery_symbols(atn, state, &recovery_symbols);
        let mut outcomes = Vec::new();
        for (transition_index, transition) in state.transitions.iter().enumerate() {
            let next_alt_number =
                next_alt_number(state, transition_index, rule_alt_number, track_alt_numbers);
            match transition {
                Transition::Epsilon { target }
                | Transition::Predicate { target, .. }
                | Transition::Action { target, .. } => {
                    let left_recursive_boundary = left_recursive_boundary(atn, state, *target);
                    let action = match transition {
                        Transition::Action { rule_index, .. } => Some(ParserAction::new(
                            state_number,
                            *rule_index,
                            rule_start_index,
                            self.previous_token_index(index),
                        )),
                        _ => None,
                    };
                    outcomes.extend(
                        self.recognize_state(
                            atn,
                            RecognizeRequest {
                                state_number: *target,
                                stop_state,
                                index,
                                rule_start_index,
                                init_action_rules,
                                rule_alt_number: next_alt_number,
                                track_alt_numbers,
                                precedence,
                                depth: depth + 1,
                                recovery_symbols: epsilon_recovery_symbols.clone(),
                            },
                            visiting,
                            memo,
                            expected,
                        )
                        .into_iter()
                        .map(|mut outcome| {
                            if let Some(rule_index) = left_recursive_boundary {
                                outcome.nodes.insert(
                                    0,
                                    RecognizedNode::LeftRecursiveBoundary { rule_index },
                                );
                            }
                            if let Some(action) = action {
                                outcome.actions.insert(0, action);
                            }
                            outcome
                        }),
                    );
                }
                Transition::Precedence {
                    target,
                    precedence: transition_precedence,
                } => {
                    if *transition_precedence >= precedence {
                        outcomes.extend(self.recognize_state(
                            atn,
                            RecognizeRequest {
                                state_number: *target,
                                stop_state,
                                index,
                                rule_start_index,
                                init_action_rules,
                                rule_alt_number: next_alt_number,
                                track_alt_numbers,
                                precedence,
                                depth: depth + 1,
                                recovery_symbols: epsilon_recovery_symbols.clone(),
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
                    let children = self.recognize_state(
                        atn,
                        RecognizeRequest {
                            state_number: *target,
                            stop_state: child_stop,
                            index,
                            rule_start_index: index,
                            init_action_rules,
                            rule_alt_number: 0,
                            track_alt_numbers,
                            precedence: *rule_precedence,
                            depth: depth + 1,
                            recovery_symbols: epsilon_recovery_symbols.clone(),
                        },
                        visiting,
                        memo,
                        expected,
                    );
                    for child in children {
                        let child_node = RecognizedNode::Rule {
                            rule_index: *rule_index,
                            alt_number: child.alt_number,
                            start_index: index,
                            stop_index: self.previous_token_index(child.index),
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
                                    init_action_rules,
                                    rule_alt_number,
                                    track_alt_numbers,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
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
                                    init_action_rules,
                                    rule_alt_number: next_alt_number,
                                    track_alt_numbers,
                                    precedence,
                                    depth: depth + 1,
                                    recovery_symbols: BTreeSet::new(),
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
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
                        outcomes.extend(self.single_token_deletion_recovery(RecoveryRequest {
                            atn,
                            transition,
                            expected_symbols: expected_symbols.clone(),
                            target: *target,
                            request: RecognizeRequest {
                                state_number,
                                stop_state,
                                index,
                                rule_start_index,
                                init_action_rules,
                                rule_alt_number,
                                track_alt_numbers,
                                precedence,
                                depth,
                                recovery_symbols: recovery_symbols.clone(),
                            },
                            visiting,
                            memo,
                            expected,
                        }));
                        if !state_is_left_recursive_rule(atn, state) {
                            outcomes.extend(self.single_token_insertion_recovery(
                                RecoveryRequest {
                                    atn,
                                    transition,
                                    expected_symbols,
                                    target: *target,
                                    request: RecognizeRequest {
                                        state_number,
                                        stop_state,
                                        index,
                                        rule_start_index,
                                        init_action_rules,
                                        rule_alt_number,
                                        track_alt_numbers,
                                        precedence,
                                        depth,
                                        recovery_symbols: recovery_symbols.clone(),
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
        }

        visiting.remove(&visit_key);
        discard_recovered_outcomes_if_clean_path_exists(&mut outcomes);
        dedupe_outcomes(&mut outcomes);
        memo.insert(key, outcomes.clone());
        outcomes
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

    /// Finds the previous token visible to the parser before `index`.
    ///
    /// The token stream cursor skips hidden-channel tokens, so subtracting one
    /// from a visible-token index can point at whitespace. Parser intervals use
    /// this helper to stop at the previous visible token while preserving hidden
    /// text inside the rendered interval.
    fn previous_token_index(&mut self, index: usize) -> Option<usize> {
        self.input.previous_visible_token_index(index)
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

    /// Returns token text for a buffered token interval.
    pub fn text_interval(&mut self, start: usize, stop: Option<usize>) -> String {
        stop.map_or_else(String::new, |stop| self.input.text(start, stop))
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
                alt_number,
                start_index,
                stop_index,
                children,
            } => {
                let mut context = ParserRuleContext::new(*rule_index, self.state());
                if track_alt_numbers {
                    context.set_alt_number(*alt_number);
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
                        alt_number: 0,
                        start_index,
                        stop_index,
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
            outcome.diagnostics.len(),
            (best.index, best.consumed_eof),
            best.diagnostics.len(),
        ) {
            return outcome;
        }
        best
    })
}

fn select_best_outcome(
    outcomes: impl Iterator<Item = RecognizeOutcome>,
) -> Option<RecognizeOutcome> {
    let outcomes = outcomes.collect::<Vec<_>>();
    let prefer_first_tie = outcomes
        .iter()
        .any(|outcome| nodes_need_stable_tie(&outcome.nodes));
    outcomes.into_iter().reduce(|best, outcome| {
        let outcome_position = (outcome.index, outcome.consumed_eof);
        let best_position = (best.index, best.consumed_eof);
        if outcome_is_better(
            outcome_position,
            outcome.diagnostics.len(),
            best_position,
            best.diagnostics.len(),
        ) || (!prefer_first_tie
            && outcome_position == best_position
            && outcome.diagnostics.len() == best.diagnostics.len()
            && outcome.actions.len() >= best.actions.len())
        {
            return outcome;
        }
        best
    })
}

fn outcome_is_better(
    outcome_position: (usize, bool),
    outcome_diagnostics: usize,
    best_position: (usize, bool),
    best_diagnostics: usize,
) -> bool {
    outcome_position > best_position
        || (outcome_position == best_position && outcome_diagnostics < best_diagnostics)
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
    nodes.iter().any(|node| node_needs_stable_tie(node, &[]))
}

fn node_needs_stable_tie(node: &RecognizedNode, ancestors: &[usize]) -> bool {
    match node {
        RecognizedNode::Token { .. }
        | RecognizedNode::ErrorToken { .. }
        | RecognizedNode::MissingToken { .. } => false,
        RecognizedNode::LeftRecursiveBoundary { .. } => true,
        RecognizedNode::Rule {
            rule_index,
            children,
            ..
        } => {
            ancestors.contains(rule_index) || {
                let mut child_ancestors = ancestors.to_vec();
                child_ancestors.push(*rule_index);
                children
                    .iter()
                    .any(|child| node_needs_stable_tie(child, &child_ancestors))
            }
        }
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
                    alt_number: 0,
                    start_index: 0,
                    stop_index: Some(0),
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
            diagnostics: Vec::new(),
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: vec![RecognizedNode::Token { index: 0 }],
        };
        let second = RecognizeOutcome {
            actions: vec![ParserAction::new(2, 0, 0, None)],
            ..first.clone()
        };

        let selected = select_best_outcome([first, second].into_iter())
            .expect("one outcome should be selected");
        assert_eq!(selected.actions[0].source_state(), 2);
    }

    #[test]
    fn outcome_ties_prefer_more_actions_for_non_recursive_paths() {
        let first = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            diagnostics: Vec::new(),
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

        let selected = select_best_outcome([second, first].into_iter())
            .expect("one outcome should be selected");
        assert_eq!(selected.actions.len(), 2);
    }

    #[test]
    fn outcome_ties_keep_first_recursive_tree_shape() {
        let recursive_nodes = vec![RecognizedNode::Rule {
            rule_index: 1,
            alt_number: 0,
            start_index: 0,
            stop_index: Some(0),
            children: vec![RecognizedNode::Rule {
                rule_index: 1,
                alt_number: 0,
                start_index: 0,
                stop_index: Some(0),
                children: vec![RecognizedNode::Token { index: 0 }],
            }],
        }];
        let first = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            diagnostics: Vec::new(),
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: recursive_nodes.clone(),
        };
        let second = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            alt_number: 0,
            diagnostics: Vec::new(),
            actions: vec![ParserAction::new(2, 0, 0, None)],
            nodes: recursive_nodes,
        };

        let selected = select_best_outcome([first, second].into_iter())
            .expect("one outcome should be selected");
        assert_eq!(selected.actions[0].source_state(), 1);
    }
}
