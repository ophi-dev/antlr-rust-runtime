use std::collections::{BTreeMap, BTreeSet};

use crate::atn::{Atn, Transition};
use crate::errors::AntlrError;
use crate::int_stream::IntStream;
use crate::recognizer::{Recognizer, RecognizerData};
use crate::token::{TOKEN_EOF, Token, TokenSource};
use crate::token_stream::CommonTokenStream;
use crate::tree::{ParseTree, ParserRuleContext, RuleNode, TerminalNode};

/// Upper bound for the recursive metadata recognizer before it treats a path as
/// non-viable. Long expression-regression descriptors legitimately walk tens
/// of thousands of ATN edges.
const RECOGNITION_DEPTH_LIMIT: usize = 100_000;

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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecognizeOutcome {
    index: usize,
    consumed_eof: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RecognizeRequest {
    state_number: usize,
    stop_state: usize,
    index: usize,
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
        let outcomes = self.recognize_state(
            atn,
            RecognizeRequest {
                state_number: start_state,
                stop_state,
                index: start_index,
                precedence: 0,
                depth: 0,
            },
            &mut visiting,
            &mut memo,
        );
        let Some(outcome) = select_best_outcome(outcomes.into_iter()) else {
            return Err(AntlrError::ParserError {
                line: self.input.lt(1).map(Token::line).unwrap_or_default(),
                column: self.input.lt(1).map(Token::column).unwrap_or_default(),
                message: format!("no viable alternative while parsing rule {rule_index}"),
            });
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

    /// Attempts to reach `stop_state` from `state_number` without committing
    /// token consumption to the parser's public stream position.
    fn recognize_state(
        &mut self,
        atn: &Atn,
        request: RecognizeRequest,
        visiting: &mut BTreeSet<(usize, usize, usize, i32)>,
        memo: &mut BTreeMap<RecognizeKey, Vec<RecognizeOutcome>>,
    ) -> Vec<RecognizeOutcome> {
        let RecognizeRequest {
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
            return vec![RecognizeOutcome {
                index,
                consumed_eof: false,
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
                    outcomes.extend(self.recognize_state(
                        atn,
                        RecognizeRequest {
                            state_number: *target,
                            stop_state,
                            index,
                            precedence,
                            depth: depth + 1,
                        },
                        visiting,
                        memo,
                    ));
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
                                precedence,
                                depth: depth + 1,
                            },
                            visiting,
                            memo,
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
                            precedence: *rule_precedence,
                            depth: depth + 1,
                        },
                        visiting,
                        memo,
                    );
                    for child in children {
                        outcomes.extend(
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *follow_state,
                                    stop_state,
                                    index: child.index,
                                    precedence,
                                    depth: depth + 1,
                                },
                                visiting,
                                memo,
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
                            self.recognize_state(
                                atn,
                                RecognizeRequest {
                                    state_number: *target,
                                    stop_state,
                                    index: next_index,
                                    precedence,
                                    depth: depth + 1,
                                },
                                visiting,
                                memo,
                            )
                            .into_iter()
                            .map(|mut outcome| {
                                outcome.consumed_eof |= symbol == TOKEN_EOF;
                                outcome
                            }),
                        );
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
}

/// Chooses the outermost parse result that consumed the most input.
///
/// The recognizer intentionally keeps shorter endpoints available while walking
/// nested rule transitions so callers can satisfy following tokens such as
/// `expr 'and' expr`. Only the public rule entry commits to one endpoint.
fn select_best_outcome(
    outcomes: impl Iterator<Item = RecognizeOutcome>,
) -> Option<RecognizeOutcome> {
    outcomes.max_by_key(|outcome| (outcome.index, outcome.consumed_eof))
}

/// Sorts and removes equivalent endpoints before memoizing a state result.
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
}
