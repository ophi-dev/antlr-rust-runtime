use std::collections::BTreeSet;

use crate::atn::{Atn, AtnStateKind, LexerActionResult, Transition};
use crate::char_stream::{CharStream, TextInterval};
use crate::int_stream::EOF;
use crate::lexer::{BaseLexer, Lexer};
use crate::token::{CommonToken, DEFAULT_CHANNEL, INVALID_TOKEN_TYPE, TokenFactory};

const MIN_CHAR_VALUE: i32 = 0;
const MAX_CHAR_VALUE: i32 = 0x0010_FFFF;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LexerConfig {
    state: usize,
    position: usize,
    consumed_eof: bool,
    alt_rule_index: Option<usize>,
    stack: Vec<usize>,
    actions: Vec<usize>,
}

#[derive(Clone, Debug)]
struct AcceptState {
    position: usize,
    rule_index: usize,
    consumed_eof: bool,
    actions: Vec<usize>,
}

#[derive(Clone, Debug)]
enum MatchResult {
    Accept(AcceptState),
    NoViableAlt { stop: usize },
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
        if lexer.hit_eof() {
            return lexer.eof_token();
        }

        if !continuing_more {
            lexer.begin_token();
        }
        let mode = lexer.mode();
        let start = lexer.input().index();
        let accept = match match_token(lexer, atn, mode, start) {
            MatchResult::Accept(accept) => accept,
            MatchResult::NoViableAlt { stop } => {
                lexer.input_mut().seek(start);
                if lexer.input_mut().la(1) == EOF {
                    lexer.set_hit_eof(true);
                    return lexer.eof_token();
                }
                report_token_recognition_error(lexer, start, stop);
                while lexer.input().index() < stop {
                    lexer.consume_char();
                }
                continuing_more = false;
                continue;
            }
        };

        lexer.input_mut().seek(start);
        while lexer.input().index() < accept.position {
            lexer.consume_char();
        }
        if accept.consumed_eof {
            lexer.set_hit_eof(true);
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

        let stop = accept.position.checked_sub(1).unwrap_or(usize::MAX);
        let text = if accept.consumed_eof && start == accept.position {
            Some("<EOF>".to_owned())
        } else {
            None
        };
        return lexer.emit_with_stop(result.token_type, result.channel, stop, text);
    }
}

/// Simulates all lexer paths reachable from the current mode start state and
/// returns the best accepting rule path for the input slice beginning at
/// `start`.
///
/// This is intentionally an ATN simulation, not generated Rust code for each
/// rule. The generated lexer carries the serialized ATN and this interpreter
/// supplies matching semantics shared by all generated grammars.
fn match_token<I, F>(lexer: &mut BaseLexer<I, F>, atn: &Atn, mode: i32, start: usize) -> MatchResult
where
    I: CharStream,
    F: TokenFactory,
{
    let Some(mode_index) = usize::try_from(mode).ok() else {
        return MatchResult::NoViableAlt { stop: start };
    };
    let Some(start_state) = atn.mode_to_start_state().get(mode_index).copied() else {
        return MatchResult::NoViableAlt { stop: start };
    };
    let mut active = prune_after_accepts(
        atn,
        epsilon_closure(
            atn,
            [LexerConfig {
                state: start_state,
                position: start,
                consumed_eof: false,
                alt_rule_index: None,
                stack: Vec::new(),
                actions: Vec::new(),
            }],
        ),
    );

    let mut best = best_accept(atn, &active);
    let mut error_stop = start;
    while !active.is_empty() {
        let mut next = Vec::new();
        for config in active {
            let symbol = symbol_at(lexer, config.position);
            if symbol != EOF {
                error_stop = error_stop.max(config.position.saturating_add(1));
            }
            let Some(state) = atn.state(config.state) else {
                continue;
            };
            for transition in &state.transitions {
                if !transition.matches(symbol, MIN_CHAR_VALUE, MAX_CHAR_VALUE) {
                    continue;
                }
                let mut advanced = config.clone();
                set_config_state(atn, &mut advanced, transition.target());
                if symbol == EOF {
                    advanced.consumed_eof = true;
                } else {
                    advanced.position += 1;
                }
                next.push(advanced);
            }
        }

        active = prune_after_accepts(atn, epsilon_closure(atn, next));
        if let Some(accept) = best_accept(atn, &active) {
            if best.as_ref().is_none_or(|current| {
                accept.position > current.position
                    || (accept.position == current.position
                        && accept.rule_index < current.rule_index)
            }) {
                best = Some(accept);
            }
        }
    }

    best.map_or(
        MatchResult::NoViableAlt { stop: error_stop },
        MatchResult::Accept,
    )
}

/// Expands epsilon, rule-call, predicate, precedence, and action transitions
/// without consuming input.
///
/// Lexer rule calls use an explicit return-state stack in `LexerConfig` because
/// fragment rules and nested lexer constructs compile to rule transitions in the
/// serialized ATN. Predicates currently pass through; semantic predicate hooks
/// will be wired here when grammar-specific semantic predicates are generated.
fn epsilon_closure(atn: &Atn, configs: impl IntoIterator<Item = LexerConfig>) -> Vec<LexerConfig> {
    let mut seen = BTreeSet::new();
    let mut closed = Vec::new();

    for config in configs {
        close_config(atn, config, &mut seen, &mut closed);
    }

    closed
}

/// Recursively expands one config's epsilon reachability in serialized
/// transition order.
///
/// Ordered DFS matters for lexer greediness: greedy loop entries serialize the
/// loop path before the exit path, while non-greedy entries serialize the exit
/// path first. The later accept-pruning step relies on this order.
fn close_config(
    atn: &Atn,
    config: LexerConfig,
    seen: &mut BTreeSet<LexerConfig>,
    closed: &mut Vec<LexerConfig>,
) {
    if !seen.insert(config.clone()) {
        return;
    }

    let Some(state) = atn.state(config.state) else {
        return;
    };

    if state.kind == AtnStateKind::RuleStop {
        if let Some((&follow_state, rest)) = config.stack.split_last() {
            let mut returned = config.clone();
            set_config_state(atn, &mut returned, follow_state);
            returned.stack = rest.to_vec();
            close_config(atn, returned, seen, closed);
        }
        closed.push(config);
        return;
    }

    let mut expanded = false;
    for transition in &state.transitions {
        match transition {
            Transition::Epsilon { target } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                close_config(atn, next, seen, closed);
                expanded = true;
            }
            Transition::Rule {
                target,
                follow_state,
                ..
            } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                next.stack.push(*follow_state);
                close_config(atn, next, seen, closed);
                expanded = true;
            }
            Transition::Predicate { target, .. } | Transition::Precedence { target, .. } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                close_config(atn, next, seen, closed);
                expanded = true;
            }
            Transition::Action {
                target,
                action_index,
                ..
            } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                if let Some(action_index) = action_index {
                    next.actions.push(*action_index);
                }
                close_config(atn, next, seen, closed);
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

/// Removes configs ordered after a top-level accept for the same lexer rule.
///
/// ANTLR's lexer simulator preserves ATN transition order and skips later
/// configs for a rule once an earlier config reaches that rule's stop state.
/// This is what makes non-greedy loops stop early while greedy loops can still
/// place their continuing path before the stop path.
fn prune_after_accepts(atn: &Atn, configs: Vec<LexerConfig>) -> Vec<LexerConfig> {
    let mut accepted_rules = BTreeSet::new();
    let mut pruned = Vec::with_capacity(configs.len());
    for config in configs {
        let Some(rule_index) = config.alt_rule_index else {
            pruned.push(config);
            continue;
        };
        if accepted_rules.contains(&rule_index) {
            continue;
        }
        let is_top_level_accept = config.stack.is_empty()
            && atn
                .state(config.state)
                .is_some_and(crate::atn::AtnState::is_rule_stop);
        if is_top_level_accept {
            accepted_rules.insert(rule_index);
        }
        pruned.push(config);
    }
    pruned
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
                rule_index: config.alt_rule_index.or(state.rule_index)?,
                consumed_eof: config.consumed_eof,
                actions: config.actions.clone(),
            })
        })
        .min_by_key(|accept| accept.rule_index)
}

/// Moves a lexer config to `state_number` and records the top-level lexer rule
/// once the config leaves a mode start state.
fn set_config_state(atn: &Atn, config: &mut LexerConfig, state_number: usize) {
    config.state = state_number;
    if config.alt_rule_index.is_none() {
        config.alt_rule_index = atn.state(state_number).and_then(|state| state.rule_index);
    }
}

/// Reports and skips a single unmatchable character using ANTLR's default lexer
/// diagnostic text.
#[allow(clippy::print_stderr)]
fn report_token_recognition_error<I, F>(lexer: &BaseLexer<I, F>, start: usize, stop: usize)
where
    I: CharStream,
    F: TokenFactory,
{
    let stop = stop.saturating_sub(1);
    let text = display_error_text(&lexer.input().text(TextInterval::new(start, stop)));
    eprintln!(
        "line {}:{} token recognition error at: '{}'",
        lexer.line(),
        lexer.column(),
        text
    );
}

fn display_error_text(text: &str) -> String {
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
