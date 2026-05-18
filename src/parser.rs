use std::collections::{BTreeMap, BTreeSet};

use crate::atn::{Atn, AtnState, AtnStateKind, Transition};
use crate::errors::AntlrError;
use crate::int_stream::IntStream;
use crate::recognizer::{Recognizer, RecognizerData};
use crate::token::{TOKEN_EOF, Token, TokenSource};
use crate::token_stream::CommonTokenStream;
use crate::tree::{ParseTree, ParserRuleContext, RuleNode, TerminalNode};
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
/// such as `$text`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ParserAction {
    source_state: usize,
    rule_index: usize,
    start_index: usize,
    stop_index: Option<usize>,
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
    actions: Vec<ParserAction>,
    nodes: Vec<RecognizedNode>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RecognizedNode {
    Token {
        index: usize,
    },
    Rule {
        rule_index: usize,
        children: Vec<Self>,
    },
    LeftRecursiveBoundary {
        rule_index: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FastRecognizeOutcome {
    index: usize,
    consumed_eof: bool,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecognizeRequest {
    state_number: usize,
    stop_state: usize,
    index: usize,
    rule_start_index: usize,
    /// Current left-recursive precedence threshold, matching ANTLR's
    /// `precpred(_ctx, k)` check for generated precedence rules.
    precedence: i32,
    depth: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecognizeKey {
    state_number: usize,
    stop_state: usize,
    index: usize,
    precedence: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FastRecognizeRequest {
    state_number: usize,
    stop_state: usize,
    index: usize,
    precedence: i32,
    depth: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FastRecognizeKey {
    state_number: usize,
    stop_state: usize,
    index: usize,
    precedence: i32,
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
            },
            &mut visiting,
            &mut memo,
            &mut expected,
        );
        let Some(outcome) = select_best_fast_outcome(outcomes.into_iter()) else {
            return Err(self.recognition_error(rule_index, &expected));
        };

        let mut context = ParserRuleContext::new(rule_index, self.state());
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
        let outcomes = self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: start_state,
                stop_state,
                index: start_index,
                rule_start_index: start_index,
                precedence: 0,
                depth: 0,
            },
            &mut visiting,
            &mut memo,
            &mut expected,
        );
        let Some(outcome) = select_best_outcome(outcomes.into_iter()) else {
            return Err(self.recognition_error(rule_index, &expected));
        };

        let mut context = ParserRuleContext::new(rule_index, self.state());
        if self.build_parse_trees {
            let nodes = fold_left_recursive_boundaries(outcome.nodes);
            for node in &nodes {
                context.add_child(self.recognized_node_tree(node)?);
            }
        }
        self.input.seek(outcome.index);

        Ok((self.rule_node(context), outcome.actions))
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
        let items = symbols
            .iter()
            .map(|symbol| expected_symbol_display(*symbol, self.vocabulary()))
            .collect::<Vec<_>>();
        if let [single] = items.as_slice() {
            return single.clone();
        }
        format!("{{{}}}", items.join(", "))
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
        } = request;
        if depth > RECOGNITION_DEPTH_LIMIT {
            return Vec::new();
        }
        if state_number == stop_state {
            return vec![FastRecognizeOutcome {
                index,
                consumed_eof: false,
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
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                outcome.consumed_eof |= child.consumed_eof;
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
                        expected.record_transition(index, transition, atn.max_token_type());
                    }
                }
            }
        }

        visiting.remove(&(state_number, stop_state, index, precedence));
        dedupe_fast_outcomes(&mut outcomes);
        memo.insert(key, outcomes.clone());
        outcomes
    }

    /// Attempts to reach `stop_state` and carries semantic actions for the
    /// selected parser path.
    fn recognize_state(
        &mut self,
        atn: &Atn,
        request: RecognizeRequest,
        visiting: &mut BTreeSet<(usize, usize, usize, i32)>,
        memo: &mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
        expected: &mut ExpectedTokens,
    ) -> Vec<RecognizeOutcome> {
        let RecognizeRequest {
            state_number,
            stop_state,
            index,
            rule_start_index,
            precedence,
            depth,
        } = request;
        if depth > RECOGNITION_DEPTH_LIMIT {
            return Vec::new();
        }
        if state_number == stop_state {
            return vec![RecognizeOutcome {
                index,
                consumed_eof: false,
                actions: Vec::new(),
                nodes: Vec::new(),
            }];
        }
        let key = RecognizeKey {
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
        let mut outcomes = Vec::new();
        for transition in &state.transitions {
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
                            index.checked_sub(1),
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
                                precedence,
                                depth: depth + 1,
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
                                precedence,
                                depth: depth + 1,
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
                            precedence: *rule_precedence,
                            depth: depth + 1,
                        },
                        visiting,
                        memo,
                        expected,
                    );
                    for child in children {
                        let child_node = RecognizedNode::Rule {
                            rule_index: *rule_index,
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
                                    precedence,
                                    depth: depth + 1,
                                },
                                visiting,
                                memo,
                                expected,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                outcome.consumed_eof |= child.consumed_eof;
                                let mut actions = child.actions.clone();
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
                                    precedence,
                                    depth: depth + 1,
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
                        expected.record_transition(index, transition, atn.max_token_type());
                    }
                }
            }
        }

        visiting.remove(&(state_number, stop_state, index, precedence));
        dedupe_outcomes(&mut outcomes);
        memo.insert(key, outcomes.clone());
        outcomes
    }

    /// Reads the token type at an absolute token-stream index.
    fn token_type_at(&mut self, index: usize) -> i32 {
        self.input.seek(index);
        self.input.la_token(1)
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

    /// Converts a recognized internal node into a public parse-tree node.
    fn recognized_node_tree(&mut self, node: &RecognizedNode) -> Result<ParseTree, AntlrError> {
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
            RecognizedNode::Rule {
                rule_index,
                children,
            } => {
                let mut context = ParserRuleContext::new(*rule_index, self.state());
                for child in children {
                    context.add_child(self.recognized_node_tree(child)?);
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

/// Folds boundary markers emitted at precedence-loop entries into nested rule
/// nodes, matching ANTLR's recursive-context parse-tree shape.
fn fold_left_recursive_boundaries(nodes: Vec<RecognizedNode>) -> Vec<RecognizedNode> {
    let mut folded = Vec::new();
    for node in nodes {
        match node {
            RecognizedNode::LeftRecursiveBoundary { rule_index } => {
                if !folded.is_empty() {
                    let children = std::mem::take(&mut folded);
                    folded.push(RecognizedNode::Rule {
                        rule_index,
                        children,
                    });
                }
            }
            node => folded.push(node),
        }
    }
    folded
}

fn token_input_display(token: &impl Token) -> String {
    format!("'{}'", token.text().unwrap_or("<EOF>"))
}

fn expected_symbol_display(symbol: i32, vocabulary: &Vocabulary) -> String {
    if symbol == TOKEN_EOF {
        return "<EOF>".to_owned();
    }
    vocabulary.display_name(symbol)
}

/// Chooses the outermost parse result that consumed the most input.
///
/// The recognizer intentionally keeps shorter endpoints available while walking
/// nested rule transitions so callers can satisfy following tokens such as
/// `expr 'and' expr`. Only the public rule entry commits to one endpoint.
fn select_best_fast_outcome(
    outcomes: impl Iterator<Item = FastRecognizeOutcome>,
) -> Option<FastRecognizeOutcome> {
    outcomes.max_by_key(|outcome| (outcome.index, outcome.consumed_eof))
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
        if outcome_position > best_position
            || (!prefer_first_tie
                && outcome_position == best_position
                && outcome.actions.len() >= best.actions.len())
        {
            return outcome;
        }
        best
    })
}

/// Reports whether a candidate contains recursive tree structure where ANTLR's
/// first viable candidate preserves the correct left-recursive context shape.
fn nodes_need_stable_tie(nodes: &[RecognizedNode]) -> bool {
    nodes.iter().any(|node| node_needs_stable_tie(node, &[]))
}

fn node_needs_stable_tie(node: &RecognizedNode, ancestors: &[usize]) -> bool {
    match node {
        RecognizedNode::Token { .. } => false,
        RecognizedNode::LeftRecursiveBoundary { .. } => true,
        RecognizedNode::Rule {
            rule_index,
            children,
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
            children: vec![RecognizedNode::Rule {
                rule_index: 1,
                children: vec![RecognizedNode::Token { index: 0 }],
            }],
        }];
        let first = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            actions: vec![ParserAction::new(1, 0, 0, None)],
            nodes: recursive_nodes.clone(),
        };
        let second = RecognizeOutcome {
            index: 1,
            consumed_eof: false,
            actions: vec![ParserAction::new(2, 0, 0, None)],
            nodes: recursive_nodes,
        };

        let selected = select_best_outcome([first, second].into_iter())
            .expect("one outcome should be selected");
        assert_eq!(selected.actions[0].source_state(), 1);
    }
}
