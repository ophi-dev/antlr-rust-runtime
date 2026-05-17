use std::collections::{BTreeSet, VecDeque};

use crate::atn::{Atn, AtnStateKind, LexerActionResult, Transition};
use crate::char_stream::CharStream;
use crate::int_stream::EOF;
use crate::lexer::{BaseLexer, Lexer};
use crate::token::{CommonToken, DEFAULT_CHANNEL, INVALID_TOKEN_TYPE, TokenFactory};

const MIN_CHAR_VALUE: i32 = 0;
const MAX_CHAR_VALUE: i32 = 0x0010_FFFF;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LexerConfig {
    state: usize,
    position: usize,
    stack: Vec<usize>,
    actions: Vec<usize>,
}

#[derive(Clone, Debug)]
struct AcceptState {
    position: usize,
    rule_index: usize,
    actions: Vec<usize>,
}

/// Runs one lexer-token match against an ANTLR ATN and returns the emitted
/// token.
///
/// The function implements ANTLR's lexer rule priority at the token level:
/// choose the longest viable match from the current mode, then choose the
/// earliest lexer rule when two matches end at the same input position. Lexer
/// actions collected on the accepted path are applied after the input cursor is
/// moved to the accepted token boundary, so mode changes and token type/channel
/// rewrites happen at the same point generated ANTLR lexers perform them.
pub fn next_token<I, F>(lexer: &mut BaseLexer<I, F>, atn: &Atn) -> CommonToken
where
    I: CharStream,
    F: TokenFactory,
{
    let mut continuing_more = false;
    loop {
        if lexer.input_mut().la(1) == EOF {
            return lexer.eof_token();
        }

        if !continuing_more {
            lexer.begin_token();
        }
        let mode = lexer.mode();
        let start = lexer.input().index();
        let Some(accept) = match_token(lexer, atn, mode, start) else {
            lexer.consume_char();
            return lexer.emit(INVALID_TOKEN_TYPE, DEFAULT_CHANNEL, None);
        };

        lexer.input_mut().seek(start);
        while lexer.input().index() < accept.position {
            lexer.consume_char();
        }

        let token_type = atn
            .rule_to_token_type()
            .get(accept.rule_index)
            .copied()
            .unwrap_or(INVALID_TOKEN_TYPE);
        let mut result = LexerActionResult::new(token_type, DEFAULT_CHANNEL);
        for action_index in accept.actions {
            if let Some(action) = atn.lexer_actions().get(action_index) {
                result.apply(action, lexer);
            }
        }

        if result.skip {
            continuing_more = false;
            continue;
        }
        if result.more {
            continuing_more = true;
            continue;
        }

        return lexer.emit(result.token_type, result.channel, None);
    }
}

/// Simulates all lexer paths reachable from the current mode start state and
/// returns the best accepting rule path for the input slice beginning at
/// `start`.
///
/// This is intentionally an ATN simulation, not generated Rust code for each
/// rule. The generated lexer carries the serialized ATN and this interpreter
/// supplies matching semantics shared by all generated grammars.
fn match_token<I, F>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    mode: i32,
    start: usize,
) -> Option<AcceptState>
where
    I: CharStream,
    F: TokenFactory,
{
    let mode_index = usize::try_from(mode).ok()?;
    let start_state = *atn.mode_to_start_state().get(mode_index)?;
    let mut active = epsilon_closure(
        atn,
        [LexerConfig {
            state: start_state,
            position: start,
            stack: Vec::new(),
            actions: Vec::new(),
        }],
    );

    let mut best = best_accept(atn, &active);
    while !active.is_empty() {
        let mut next = Vec::new();
        for config in active {
            let symbol = symbol_at(lexer, config.position);
            if symbol == EOF {
                continue;
            }
            let Some(state) = atn.state(config.state) else {
                continue;
            };
            for transition in &state.transitions {
                if !transition.matches(symbol, MIN_CHAR_VALUE, MAX_CHAR_VALUE) {
                    continue;
                }
                let mut advanced = config.clone();
                advanced.state = transition.target();
                advanced.position += 1;
                next.push(advanced);
            }
        }

        active = epsilon_closure(atn, next);
        if let Some(accept) = best_accept(atn, &active) {
            if best
                .as_ref()
                .is_none_or(|current| accept.position > current.position)
                || best.as_ref().is_some_and(|current| {
                    accept.position == current.position && accept.rule_index < current.rule_index
                })
            {
                best = Some(accept);
            }
        }
    }

    best
}

/// Expands epsilon, rule-call, predicate, precedence, and action transitions
/// without consuming input.
///
/// Lexer rule calls use an explicit return-state stack in `LexerConfig` because
/// fragment rules and nested lexer constructs compile to rule transitions in the
/// serialized ATN. Predicates currently pass through; semantic predicate hooks
/// will be wired here when grammar-specific semantic predicates are generated.
fn epsilon_closure(atn: &Atn, configs: impl IntoIterator<Item = LexerConfig>) -> Vec<LexerConfig> {
    let mut queue: VecDeque<LexerConfig> = configs.into_iter().collect();
    let mut seen = BTreeSet::new();
    let mut closed = Vec::new();

    while let Some(config) = queue.pop_front() {
        if !seen.insert(config.clone()) {
            continue;
        }

        let Some(state) = atn.state(config.state) else {
            continue;
        };

        if state.kind == AtnStateKind::RuleStop {
            if let Some((&follow_state, rest)) = config.stack.split_last() {
                let mut returned = config.clone();
                returned.state = follow_state;
                returned.stack = rest.to_vec();
                queue.push_back(returned);
            }
            closed.push(config);
            continue;
        }

        let mut expanded = false;
        for transition in &state.transitions {
            match transition {
                Transition::Epsilon { target } => {
                    let mut next = config.clone();
                    next.state = *target;
                    queue.push_back(next);
                    expanded = true;
                }
                Transition::Rule {
                    target,
                    follow_state,
                    ..
                } => {
                    let mut next = config.clone();
                    next.state = *target;
                    next.stack.push(*follow_state);
                    queue.push_back(next);
                    expanded = true;
                }
                Transition::Predicate { target, .. } | Transition::Precedence { target, .. } => {
                    let mut next = config.clone();
                    next.state = *target;
                    queue.push_back(next);
                    expanded = true;
                }
                Transition::Action {
                    target,
                    action_index,
                    ..
                } => {
                    let mut next = config.clone();
                    next.state = *target;
                    if let Some(action_index) = action_index {
                        next.actions.push(*action_index);
                    }
                    queue.push_back(next);
                    expanded = true;
                }
                Transition::Atom { .. }
                | Transition::Range { .. }
                | Transition::Set { .. }
                | Transition::NotSet { .. }
                | Transition::Wildcard { .. } => {}
            }
        }

        if !expanded
            || state
                .transitions
                .iter()
                .any(|transition| !transition.is_epsilon())
        {
            closed.push(config);
        }
    }

    closed
}

/// Selects the highest-priority accept configuration from a closure set.
///
/// ANTLR lexer priority is encoded by rule order. `match_token` already handles
/// longest-match selection across input positions; within a single position the
/// lower rule index wins.
fn best_accept(atn: &Atn, configs: &[LexerConfig]) -> Option<AcceptState> {
    configs
        .iter()
        .filter_map(|config| {
            let state = atn.state(config.state)?;
            if !state.is_rule_stop() || !config.stack.is_empty() {
                return None;
            }
            Some(AcceptState {
                position: config.position,
                rule_index: state.rule_index?,
                actions: config.actions.clone(),
            })
        })
        .min_by_key(|accept| accept.rule_index)
}

/// Reads the Unicode scalar value at an absolute character-stream index.
///
/// The interpreter explores many paths at different input offsets, so it seeks
/// the shared input stream before each lookahead instead of cloning the stream.
fn symbol_at<I, F>(lexer: &mut BaseLexer<I, F>, position: usize) -> i32
where
    I: CharStream,
    F: TokenFactory,
{
    lexer.input_mut().seek(position);
    lexer.input_mut().la(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atn::serialized::{AtnDeserializer, SerializedAtn};
    use crate::char_stream::InputStream;
    use crate::recognizer::RecognizerData;
    use crate::token::{TOKEN_EOF, Token};
    use crate::vocabulary::Vocabulary;

    #[test]
    fn lexer_matches_longest_token_and_skips() {
        let atn = AtnDeserializer::new(&SerializedAtn::from_i32([
            4, 0, 2, // version, lexer, max token type
            9, // states
            6, -1, // 0 token start
            2, 0, // 1 rule 0 start
            1, 0, // 2
            1, 0, // 3
            7, 0, // 4 rule 0 stop
            2, 1, // 5 rule 1 start
            1, 1, // 6
            1, 1, // 7
            7, 1, // 8 rule 1 stop
            0, // non-greedy
            0, // precedence
            2, // rules
            1, 1, // rule 0 starts at 1, token type 1
            5, 2, // rule 1 starts at 5, token type 2
            1, // modes
            0, // default mode starts at 0
            0, // sets
            8, // edges
            0, 1, 1, 0, 0, 0, // start -> rule 0
            0, 5, 1, 0, 0, 0, // start -> rule 1
            1, 2, 5, 'a' as i32, 0, 0, 2, 3, 5, 'b' as i32, 0, 0, 3, 4, 1, 0, 0, 0, 5, 6, 5,
            ' ' as i32, 0, 0, 6, 7, 1, 0, 0, 0, 7, 8, 6, 1, 0, 0, // action 0, then stop
            1, // decisions
            0, 1, // lexer actions
            6, 0, 0, // skip
        ]))
        .deserialize()
        .expect("artificial lexer ATN should deserialize");
        let data = RecognizerData::new(
            "T",
            Vocabulary::new(
                [None, Some("'ab'"), Some("' '")],
                [None, Some("AB"), Some("WS")],
                [None::<&str>, None, None],
            ),
        );
        let mut lexer = BaseLexer::new(InputStream::new(" ab"), data);

        let token = next_token(&mut lexer, &atn);
        assert_eq!(token.token_type(), 1);
        assert_eq!(token.text(), Some("ab"));
        assert_eq!(next_token(&mut lexer, &atn).token_type(), TOKEN_EOF);
    }

    #[test]
    fn lexer_more_extends_original_token_start() {
        let atn = AtnDeserializer::new(&SerializedAtn::from_i32([
            4, 0, 1, // version, lexer, max token type
            8, // states
            6, -1, // 0 token start
            2, 0, // 1 rule 0 start
            1, 0, // 2
            1, 0, // 3
            7, 0, // 4 rule 0 stop
            2, 1, // 5 rule 1 start
            1, 1, // 6
            7, 1, // 7 rule 1 stop
            0, // non-greedy
            0, // precedence
            2, // rules
            1, 1, // rule 0 starts at 1, token type 1
            5, 1, // rule 1 starts at 5, token type 1
            1, // modes
            0, // default mode starts at 0
            0, // sets
            6, // edges
            0, 1, 1, 0, 0, 0, // start -> rule 0
            0, 5, 1, 0, 0, 0, // start -> rule 1
            1, 2, 5, 'a' as i32, 0, 0, 2, 4, 6, 0, 0, 0, // more action, then stop
            5, 6, 5, 'b' as i32, 0, 0, 6, 7, 1, 0, 0, 0, 1, // decisions
            0, 1, // lexer actions
            3, 0, 0, // more
        ]))
        .deserialize()
        .expect("artificial lexer ATN with more action should deserialize");
        let data = RecognizerData::new(
            "T",
            Vocabulary::new([None, Some("AB")], [None, Some("AB")], [None::<&str>, None]),
        );
        let mut lexer = BaseLexer::new(InputStream::new("ab"), data);

        let token = next_token(&mut lexer, &atn);
        assert_eq!(token.token_type(), 1);
        assert_eq!(token.start(), 0);
        assert_eq!(token.stop(), 1);
        assert_eq!(token.text(), Some("ab"));
    }
}
