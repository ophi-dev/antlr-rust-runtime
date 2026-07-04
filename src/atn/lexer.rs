use std::collections::{BTreeSet, HashSet};
use std::hash::BuildHasherDefault;

use crate::atn::lexer_dfa::{CompiledLexerAccept, CompiledLexerDfa, DEAD_STATE, ESCAPE_STATE};
use crate::atn::{Atn, AtnStateKind, LexerAction, Transition};
use crate::char_stream::{CharStream, TextInterval};
use crate::int_stream::EOF;
use crate::lexer::{
    BaseLexer, Lexer, LexerCustomAction, LexerDfaActionKey, LexerDfaCachedAccept,
    LexerDfaCachedState, LexerDfaCachedTransition, LexerDfaConfigKey, LexerDfaKey, LexerPredicate,
};
use crate::prediction::PredictionFxHasher;
use crate::token::{CommonToken, DEFAULT_CHANNEL, INVALID_TOKEN_TYPE, TokenFactory};

#[allow(clippy::disallowed_types)]
type FxHashSet<K> = HashSet<K, BuildHasherDefault<PredictionFxHasher>>;

const MIN_CHAR_VALUE: i32 = 0;
const MAX_CHAR_VALUE: i32 = 0x0010_FFFF;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct LexerConfig {
    pub(super) state: usize,
    pub(super) position: usize,
    pub(super) consumed_eof: bool,
    pub(super) alt_rule_index: Option<usize>,
    pub(super) passed_non_greedy: bool,
    pub(super) stack: Vec<usize>,
    pub(super) actions: Vec<LexerActionTrace>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct LexerActionTrace {
    pub(super) action_index: usize,
    pub(super) position: usize,
    /// Lexer rule that the action transition belonged to. ANTLR suppresses
    /// commands of nested non-fragment rule references, so the dispatcher
    /// must compare this against the accepted rule before applying side
    /// effects like `pushMode` / `popMode`.
    pub(super) rule_index: usize,
}

#[derive(Clone, Debug)]
pub(super) struct AcceptState {
    pub(super) position: usize,
    pub(super) rule_index: usize,
    pub(super) consumed_eof: bool,
    pub(super) actions: Vec<LexerActionTrace>,
}

#[derive(Clone, Debug)]
enum MatchResult {
    Accept(AcceptState),
    NoViableAlt { stop: usize },
}

#[derive(Clone, Debug)]
pub(super) struct ClosureResult {
    pub(super) configs: Vec<LexerConfig>,
    pub(super) has_semantic_context: bool,
}

/// Mutable emission state produced by executing lexer actions for one token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LexerActionResult {
    token_type: i32,
    channel: i32,
    skip: bool,
    more: bool,
}

impl LexerActionResult {
    /// Starts action execution with the token type chosen by the accepted rule
    /// and the default channel.
    const fn new(token_type: i32, channel: i32) -> Self {
        Self {
            token_type,
            channel,
            skip: false,
            more: false,
        }
    }

    /// Applies one deserialized lexer action to this token emission result and
    /// to the lexer mode stack when the action changes modes.
    fn apply<I, F>(&mut self, action: &LexerAction, lexer: &mut BaseLexer<I, F>)
    where
        I: CharStream,
        F: TokenFactory,
    {
        match action {
            LexerAction::Channel(channel) => self.channel = *channel,
            LexerAction::Custom { .. } => {}
            LexerAction::Mode(mode) => lexer.set_mode(*mode),
            LexerAction::More => self.more = true,
            LexerAction::PopMode => {
                lexer.pop_mode();
            }
            LexerAction::PushMode(mode) => lexer.push_mode(*mode),
            LexerAction::Skip => self.skip = true,
            LexerAction::Type(token_type) => self.token_type = *token_type,
        }
    }
}

/// Accumulates one epsilon-closure expansion, including whether predicate
/// evaluation made the closure input-position-sensitive.
struct ClosureState {
    seen: FxHashSet<LexerConfig>,
    closed: Vec<LexerConfig>,
    has_semantic_context: bool,
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
    next_token_with_cache(lexer, atn, |_, _| {}, |_, _| true, |_, _, _| {})
}

/// Runs one lexer-token match and invokes `custom_action` for embedded
/// grammar-specific lexer actions on the accepted path.
///
/// The callback receives the base lexer plus the serialized custom-action
/// coordinates. It is used by generated lexers to replay target templates while
/// keeping all ATN path exploration in the shared runtime.
pub fn next_token_with_actions<I, F, A>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    custom_action: A,
) -> CommonToken
where
    I: CharStream,
    F: TokenFactory,
    A: FnMut(&mut BaseLexer<I, F>, LexerCustomAction),
{
    next_token_with_hooks(lexer, atn, custom_action, |_, _| true, |_, _, _| {})
}

/// Runs one lexer-token match and lets generated code adjust the final accept
/// position before the token is emitted.
///
/// ANTLR target templates such as `PositionAdjustingLexer` use this to accept
/// a long disambiguating token path but emit only the prefix, leaving the
/// remaining characters for the next token.
pub fn next_token_with_accept_adjuster<I, F, E>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    accept_adjuster: E,
) -> CommonToken
where
    I: CharStream,
    F: TokenFactory,
    E: FnMut(&mut BaseLexer<I, F>, i32, usize),
{
    next_token_with_hooks(lexer, atn, |_, _| {}, |_, _| true, accept_adjuster)
}

/// Runs one lexer-token match with grammar-specific actions and predicates.
///
/// Predicates are evaluated during ATN closure construction so non-viable
/// paths are rejected before longest-match and lexer-rule priority selection.
pub fn next_token_with_actions_and_predicates<I, F, A, P>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    mut custom_action: A,
    mut semantic_predicate: P,
) -> CommonToken
where
    I: CharStream,
    F: TokenFactory,
    A: FnMut(&mut BaseLexer<I, F>, LexerCustomAction),
    P: FnMut(&BaseLexer<I, F>, LexerPredicate) -> bool,
{
    next_token_with_hooks(
        lexer,
        atn,
        &mut custom_action,
        &mut semantic_predicate,
        |_, _, _| {},
    )
}

/// Runs one lexer-token match with all generated extension hooks.
///
/// Custom actions and predicates correspond to serialized ATN edges. The
/// accept adjuster runs after lexer commands but before `emit`, matching target
/// runtimes that override emission to split a longest-match token.
pub fn next_token_with_hooks<I, F, A, P, E>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    mut custom_action: A,
    mut semantic_predicate: P,
    mut accept_adjuster: E,
) -> CommonToken
where
    I: CharStream,
    F: TokenFactory,
    A: FnMut(&mut BaseLexer<I, F>, LexerCustomAction),
    P: FnMut(&BaseLexer<I, F>, LexerPredicate) -> bool,
    E: FnMut(&mut BaseLexer<I, F>, i32, usize),
{
    next_token_with_hooks_impl(
        lexer,
        atn,
        &mut custom_action,
        &mut semantic_predicate,
        &mut accept_adjuster,
        LexerMatchStrategy {
            compiled: None,
            use_cache: false,
        },
    )
}

/// Runs one lexer-token match against an ahead-of-time compiled lexer DFA.
///
/// Tokens starting in a compiled mode are matched by walking static tables;
/// modes the compiler left dynamic fall back to cached ATN interpretation per
/// token, so behavior always matches [`next_token`].
pub fn next_token_compiled<I, F>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    dfa: &CompiledLexerDfa,
) -> CommonToken
where
    I: CharStream,
    F: TokenFactory,
{
    next_token_with_hooks_impl(
        lexer,
        atn,
        &mut |_, _| {},
        &mut |_, _| true,
        &mut |_, _, _| {},
        LexerMatchStrategy {
            compiled: Some(dfa),
            use_cache: true,
        },
    )
}

/// Runs one compiled-DFA lexer-token match with all generated extension hooks.
///
/// Compiled modes never contain semantic predicates, so hook grammars still
/// take the table walk for their static modes; predicate-bearing modes re-run
/// the ATN interpreter exactly like [`next_token_with_hooks`].
pub fn next_token_compiled_with_hooks<I, F, A, P, E>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    dfa: &CompiledLexerDfa,
    mut custom_action: A,
    mut semantic_predicate: P,
    mut accept_adjuster: E,
) -> CommonToken
where
    I: CharStream,
    F: TokenFactory,
    A: FnMut(&mut BaseLexer<I, F>, LexerCustomAction),
    P: FnMut(&BaseLexer<I, F>, LexerPredicate) -> bool,
    E: FnMut(&mut BaseLexer<I, F>, i32, usize),
{
    next_token_with_hooks_impl(
        lexer,
        atn,
        &mut custom_action,
        &mut semantic_predicate,
        &mut accept_adjuster,
        LexerMatchStrategy {
            compiled: Some(dfa),
            use_cache: false,
        },
    )
}

fn next_token_with_cache<I, F, A, P, E>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    mut custom_action: A,
    mut semantic_predicate: P,
    mut accept_adjuster: E,
) -> CommonToken
where
    I: CharStream,
    F: TokenFactory,
    A: FnMut(&mut BaseLexer<I, F>, LexerCustomAction),
    P: FnMut(&BaseLexer<I, F>, LexerPredicate) -> bool,
    E: FnMut(&mut BaseLexer<I, F>, i32, usize),
{
    next_token_with_hooks_impl(
        lexer,
        atn,
        &mut custom_action,
        &mut semantic_predicate,
        &mut accept_adjuster,
        LexerMatchStrategy {
            compiled: None,
            use_cache: true,
        },
    )
}

/// Token-matching backend chosen by a lexer entry point: an optional
/// ahead-of-time compiled DFA, and whether ATN interpretation (used directly
/// or as the compiled path's per-mode fallback) may replay the learned-DFA
/// cache. Hook entry points disable the cache, matching their interpreted
/// counterparts.
#[derive(Clone, Copy)]
struct LexerMatchStrategy<'a> {
    compiled: Option<&'a CompiledLexerDfa>,
    use_cache: bool,
}

/// Dispatches one token match to the strategy's backend.
fn match_token_with_strategy<I, F, P>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    mode: i32,
    start: usize,
    semantic_predicate: &mut P,
    strategy: LexerMatchStrategy<'_>,
) -> MatchResult
where
    I: CharStream,
    F: TokenFactory,
    P: FnMut(&BaseLexer<I, F>, LexerPredicate) -> bool,
{
    if let Some(dfa) = strategy.compiled
        && !lexer.force_interpreted()
        && let Some(start_state) = dfa.mode_start(mode)
        && let Some(result) = match_token_compiled(lexer, dfa, start_state, start)
    {
        return result;
    }
    if strategy.use_cache {
        match_token_cached(lexer, atn, mode, start, semantic_predicate)
    } else {
        match_token(lexer, atn, mode, start, semantic_predicate)
    }
}

fn next_token_with_hooks_impl<I, F, A, P, E>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    custom_action: &mut A,
    semantic_predicate: &mut P,
    accept_adjuster: &mut E,
    strategy: LexerMatchStrategy<'_>,
) -> CommonToken
where
    I: CharStream,
    F: TokenFactory,
    A: FnMut(&mut BaseLexer<I, F>, LexerCustomAction),
    P: FnMut(&BaseLexer<I, F>, LexerPredicate) -> bool,
    E: FnMut(&mut BaseLexer<I, F>, i32, usize),
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
        let token_match =
            match_token_with_strategy(lexer, atn, mode, start, semantic_predicate, strategy);
        let accept = match token_match {
            MatchResult::Accept(accept) => accept,
            MatchResult::NoViableAlt { stop } => {
                lexer.input_mut().seek(start);
                if lexer.input_mut().la(1) == EOF {
                    lexer.set_hit_eof(true);
                    return lexer.eof_token();
                }
                record_token_recognition_error(lexer, start, stop);
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
        for trace in accept.actions {
            if !lexer_action_belongs_to_accept(atn, accept.rule_index, trace.rule_index) {
                continue;
            }
            if let Some(action) = atn.lexer_actions().get(trace.action_index) {
                match action {
                    LexerAction::Custom {
                        rule_index,
                        action_index,
                    } => {
                        custom_action(
                            lexer,
                            LexerCustomAction::new(*rule_index, *action_index, trace.position),
                        );
                    }
                    other => result.apply(other, lexer),
                }
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

        accept_adjuster(lexer, result.token_type, accept.position);
        let emit_position = lexer.input().index();
        let stop = emit_position.checked_sub(1).unwrap_or(usize::MAX);
        let text = if accept.consumed_eof && start == emit_position {
            Some("<EOF>".to_owned())
        } else {
            None
        };
        return lexer.emit_with_stop(result.token_type, result.channel, stop, text);
    }
}

/// Reports whether a custom lexer action should fire for the accepted token.
///
/// ANTLR treats token-rule references inside another token rule like inlined
/// matching logic for action ownership: the referenced token rule can help match
/// text, but its embedded action does not run unless that rule itself accepts
/// the token. Fragment-rule actions remain eligible because fragments have no
/// token type of their own.
pub(super) fn lexer_action_belongs_to_accept(
    atn: &Atn,
    accept_rule: usize,
    action_rule: usize,
) -> bool {
    action_rule == accept_rule
        || atn
            .rule_to_token_type()
            .get(action_rule)
            .copied()
            .unwrap_or(INVALID_TOKEN_TYPE)
            == INVALID_TOKEN_TYPE
}

/// Simulates all lexer paths reachable from the current mode start state and
/// returns the best accepting rule path for the input slice beginning at
/// `start`.
///
/// This is intentionally an ATN simulation, not generated Rust code for each
/// rule. The generated lexer carries the serialized ATN and this interpreter
/// supplies matching semantics shared by all generated grammars.
fn match_token<I, F, P>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    mode: i32,
    start: usize,
    semantic_predicate: &mut P,
) -> MatchResult
where
    I: CharStream,
    F: TokenFactory,
    P: FnMut(&BaseLexer<I, F>, LexerPredicate) -> bool,
{
    let Some(mode_index) = usize::try_from(mode).ok() else {
        return MatchResult::NoViableAlt { stop: start };
    };
    let Some(start_state) = atn.mode_to_start_state().get(mode_index).copied() else {
        return MatchResult::NoViableAlt { stop: start };
    };
    let start_closure = epsilon_closure(
        atn,
        [LexerConfig {
            state: start_state,
            position: start,
            consumed_eof: false,
            alt_rule_index: None,
            passed_non_greedy: false,
            stack: Vec::new(),
            actions: Vec::new(),
        }],
        &mut |predicate| semantic_predicate(lexer, predicate),
    );
    let mut active = prune_after_accepts(atn, start_closure.configs);
    let mut dfa_state = lexer.lexer_dfa_state(
        lexer_dfa_key(&active, start),
        accept_prediction(atn, &active),
    );
    let mut dfa_state_has_semantic_context = start_closure.has_semantic_context;

    let mut best = best_accept(atn, &active);
    let mut error_stop = start;
    while !active.is_empty() {
        let mut next = Vec::new();
        let source_dfa_state = dfa_state;
        let source_has_semantic_context = dfa_state_has_semantic_context;
        let mut edge_symbol = None;
        for config in active {
            let symbol = symbol_at(lexer, config.position);
            if symbol != EOF {
                error_stop = error_stop.max(config.position.saturating_add(1));
                edge_symbol = Some(symbol);
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

        let closure = epsilon_closure(atn, next, &mut |predicate| {
            semantic_predicate(lexer, predicate)
        });
        let target_has_semantic_context = closure.has_semantic_context;
        let suppress_edge = source_has_semantic_context || target_has_semantic_context;
        active = prune_after_accepts(atn, closure.configs);
        if !active.is_empty() {
            dfa_state = lexer.lexer_dfa_state(
                lexer_dfa_key(&active, start),
                accept_prediction(atn, &active),
            );
            dfa_state_has_semantic_context = target_has_semantic_context;
            if !suppress_edge {
                if let Some(symbol) = edge_symbol {
                    lexer.record_lexer_dfa_edge(source_dfa_state, symbol, dfa_state);
                }
            }
        }
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

fn match_token_cached<I, F, P>(
    lexer: &mut BaseLexer<I, F>,
    atn: &Atn,
    mode: i32,
    start: usize,
    semantic_predicate: &mut P,
) -> MatchResult
where
    I: CharStream,
    F: TokenFactory,
    P: FnMut(&BaseLexer<I, F>, LexerPredicate) -> bool,
{
    let Some((mut dfa_state, mode_start_has_semantic_context)) =
        cached_mode_start_state(lexer, atn, mode, start, semantic_predicate)
    else {
        return MatchResult::NoViableAlt { stop: start };
    };
    if mode_start_has_semantic_context {
        return match_token(lexer, atn, mode, start, semantic_predicate);
    }

    let mut position = start;
    let mut best = None;
    let mut error_stop = start;
    loop {
        let Some(cached_state) = lexer.cached_lexer_dfa_state(dfa_state) else {
            return match_token(lexer, atn, mode, start, semantic_predicate);
        };
        if cached_state.has_semantic_context {
            return match_token(lexer, atn, mode, start, semantic_predicate);
        }
        if let Some(accept) = cached_state.accept.as_ref() {
            let accept = cached_accept_state(accept, start, position);
            if best.as_ref().is_none_or(|current: &AcceptState| {
                accept.position > current.position
                    || (accept.position == current.position
                        && accept.rule_index < current.rule_index)
            }) {
                best = Some(accept);
            }
        }

        let symbol = symbol_at(lexer, position);
        if symbol != EOF {
            error_stop = error_stop.max(position.saturating_add(1));
        }

        if !cached_state.has_semantic_context {
            if let Some(cached) = lexer.cached_lexer_dfa_transition(dfa_state, symbol) {
                // A cached transition implies its DFA edge was already recorded
                // when the transition was cached (the sole
                // `cache_lexer_dfa_transition` site records it first, under the
                // same suppress-edge gate), so re-recording here would be a
                // per-character BTreeSet re-insert of a duplicate.
                dfa_state = cached.target_state;
                position += cached.position_delta;
                continue;
            }
        }

        let source_dfa_state = dfa_state;
        let source_has_semantic_context = cached_state.has_semantic_context;
        let active = cached_configs_to_configs(&cached_state.configs, start, position);
        let mut next = Vec::new();
        for config in active {
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

        let closure = epsilon_closure(atn, next, &mut |predicate| {
            semantic_predicate(lexer, predicate)
        });
        let target_has_semantic_context = closure.has_semantic_context;
        if target_has_semantic_context {
            return match_token(lexer, atn, mode, start, semantic_predicate);
        }
        let suppress_edge = source_has_semantic_context || target_has_semantic_context;
        let active = prune_after_accepts(atn, closure.configs);
        if active.is_empty() {
            break;
        }
        let Some(target_position) = shared_config_position(&active) else {
            return match_token(lexer, atn, mode, start, semantic_predicate);
        };
        dfa_state = cache_dfa_state(
            lexer,
            atn,
            &active,
            target_has_semantic_context,
            start,
            target_position,
        );
        if !suppress_edge && symbol != EOF {
            lexer.record_lexer_dfa_edge(source_dfa_state, symbol, dfa_state);
            lexer.cache_lexer_dfa_transition(
                source_dfa_state,
                symbol,
                LexerDfaCachedTransition {
                    target_state: dfa_state,
                    position_delta: target_position.saturating_sub(position),
                },
            );
        }
        position = target_position;
    }

    best.map_or(
        MatchResult::NoViableAlt { stop: error_stop },
        MatchResult::Accept,
    )
}

/// Bounds EOF-edge traversals in the compiled walk. EOF transitions do not
/// advance the cursor, so past this bound the walk stops guessing and escapes
/// to the ATN interpreter, which owns the semantics of longer EOF chains
/// (including grammars whose chains never terminate).
const MAX_COMPILED_EOF_EDGES: u32 = 8;

/// Matches one token by walking the ahead-of-time compiled lexer DFA.
///
/// The walk reproduces the interpreter's longest-match selection: remember
/// the best accept seen so far, advance until the table has no transition,
/// then return the remembered accept — or a recognition error spanning every
/// character the walk looked at, exactly like `match_token`. Reaching an
/// escape edge (semantic predicate, recursive lexer rule, state budget)
/// returns `None`, and the caller re-matches the token with the interpreter.
fn match_token_compiled<I, F>(
    lexer: &mut BaseLexer<I, F>,
    dfa: &CompiledLexerDfa,
    start_state: u16,
    start: usize,
) -> Option<MatchResult>
where
    I: CharStream,
    F: TokenFactory,
{
    let mut state = start_state;
    let mut position = start;
    let mut best: Option<AcceptState> = None;
    let mut error_stop = start;
    let mut eof_edges = 0_u32;
    loop {
        if let Some(accept) = dfa.accept(state) {
            record_compiled_accept(accept, position, &mut best);
        }
        let symbol = symbol_at(lexer, position);
        let target = if symbol == EOF {
            eof_edges += 1;
            if eof_edges > MAX_COMPILED_EOF_EDGES {
                return None;
            }
            dfa.eof_target(state)
        } else {
            error_stop = error_stop.max(position.saturating_add(1));
            dfa.char_target(state, symbol)
        };
        if target == DEAD_STATE {
            break;
        }
        if target == ESCAPE_STATE {
            return None;
        }
        if symbol != EOF {
            position += 1;
        }
        state = target;
    }
    Some(best.map_or(
        MatchResult::NoViableAlt { stop: error_stop },
        MatchResult::Accept,
    ))
}

/// Applies the interpreter's longest-match / lowest-rule preference to one
/// compiled accept state, materializing its action traces at absolute input
/// positions.
fn record_compiled_accept(
    accept: &CompiledLexerAccept,
    position: usize,
    best: &mut Option<AcceptState>,
) {
    let replaces = best.as_ref().is_none_or(|current| {
        position > current.position
            || (position == current.position && accept.rule_index < current.rule_index)
    });
    if !replaces {
        return;
    }
    *best = Some(AcceptState {
        position,
        rule_index: accept.rule_index,
        consumed_eof: accept.consumed_eof,
        actions: accept
            .actions
            .iter()
            .map(|trace| LexerActionTrace {
                action_index: trace.action_index,
                position: position.saturating_sub(trace.behind),
                rule_index: trace.rule_index,
            })
            .collect(),
    });
}

fn cached_mode_start_state<I, F, P>(
    lexer: &BaseLexer<I, F>,
    atn: &Atn,
    mode: i32,
    start: usize,
    semantic_predicate: &mut P,
) -> Option<(usize, bool)>
where
    I: CharStream,
    F: TokenFactory,
    P: FnMut(&BaseLexer<I, F>, LexerPredicate) -> bool,
{
    if let Some(state) = lexer.cached_lexer_mode_start(mode) {
        return Some((state, false));
    }

    let mode_index = usize::try_from(mode).ok()?;
    let start_state = atn.mode_to_start_state().get(mode_index).copied()?;
    let start_closure = epsilon_closure(
        atn,
        [LexerConfig {
            state: start_state,
            position: start,
            consumed_eof: false,
            alt_rule_index: None,
            passed_non_greedy: false,
            stack: Vec::new(),
            actions: Vec::new(),
        }],
        &mut |predicate| semantic_predicate(lexer, predicate),
    );
    let active = prune_after_accepts(atn, start_closure.configs);
    let state = cache_dfa_state(
        lexer,
        atn,
        &active,
        start_closure.has_semantic_context,
        start,
        start,
    );
    if !start_closure.has_semantic_context {
        lexer.cache_lexer_mode_start(mode, state);
    }
    Some((state, start_closure.has_semantic_context))
}

fn cache_dfa_state<I, F>(
    lexer: &BaseLexer<I, F>,
    atn: &Atn,
    active: &[LexerConfig],
    has_semantic_context: bool,
    token_start: usize,
    position: usize,
) -> usize
where
    I: CharStream,
    F: TokenFactory,
{
    let state = lexer.lexer_dfa_state(
        lexer_dfa_key(active, token_start),
        accept_prediction(atn, active),
    );
    if !has_semantic_context {
        lexer.cache_lexer_dfa_state(
            state,
            LexerDfaCachedState {
                has_semantic_context,
                configs: active
                    .iter()
                    .map(|config| normalized_config_key(config, token_start))
                    .collect(),
                accept: best_accept(atn, active).map(|accept| LexerDfaCachedAccept {
                    position_delta: accept.position.saturating_sub(position),
                    rule_index: accept.rule_index,
                    consumed_eof: accept.consumed_eof,
                    actions: accept
                        .actions
                        .iter()
                        .map(|action| LexerDfaActionKey {
                            action_index: action.action_index,
                            position_delta: action.position.saturating_sub(token_start),
                            rule_index: action.rule_index,
                        })
                        .collect(),
                }),
            },
        );
    }
    state
}

fn cached_accept_state(
    accept: &LexerDfaCachedAccept,
    token_start: usize,
    state_position: usize,
) -> AcceptState {
    let position = state_position + accept.position_delta;
    AcceptState {
        position,
        rule_index: accept.rule_index,
        consumed_eof: accept.consumed_eof,
        actions: accept
            .actions
            .iter()
            .map(|action| LexerActionTrace {
                action_index: action.action_index,
                position: token_start + action.position_delta,
                rule_index: action.rule_index,
            })
            .collect(),
    }
}

/// Expands epsilon, rule-call, predicate, precedence, and action transitions
/// without consuming input.
///
/// Lexer rule calls use an explicit return-state stack in `LexerConfig` because
/// fragment rules and nested lexer constructs compile to rule transitions in the
/// serialized ATN.
pub(super) fn epsilon_closure<P>(
    atn: &Atn,
    configs: impl IntoIterator<Item = LexerConfig>,
    semantic_predicate: &mut P,
) -> ClosureResult
where
    P: FnMut(LexerPredicate) -> bool,
{
    let mut state = ClosureState {
        seen: FxHashSet::default(),
        closed: Vec::new(),
        has_semantic_context: false,
    };

    for config in configs {
        close_config(atn, config, &mut state, semantic_predicate);
    }

    ClosureResult {
        configs: state.closed,
        has_semantic_context: state.has_semantic_context,
    }
}

/// Recursively expands one config's epsilon reachability in serialized
/// transition order.
///
/// Ordered DFS matters for lexer greediness: greedy loop entries serialize the
/// loop path before the exit path, while non-greedy entries serialize the exit
/// path first. The later accept-pruning step relies on this order.
fn close_config<P>(
    atn: &Atn,
    config: LexerConfig,
    closure: &mut ClosureState,
    semantic_predicate: &mut P,
) where
    P: FnMut(LexerPredicate) -> bool,
{
    if !closure.seen.insert(config.clone()) {
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
            close_config(atn, returned, closure, semantic_predicate);
        }
        closure.closed.push(config);
        return;
    }

    for transition in &state.transitions {
        match transition {
            Transition::Epsilon { target } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                next.passed_non_greedy |= state.non_greedy;
                close_config(atn, next, closure, semantic_predicate);
            }
            Transition::Rule {
                target,
                follow_state,
                ..
            } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                next.passed_non_greedy |= state.non_greedy;
                next.stack.push(*follow_state);
                close_config(atn, next, closure, semantic_predicate);
            }
            Transition::Predicate {
                target,
                rule_index,
                pred_index,
                ..
            } => {
                closure.has_semantic_context = true;
                if semantic_predicate(LexerPredicate::new(
                    *rule_index,
                    *pred_index,
                    config.position,
                )) {
                    let mut next = config.clone();
                    set_config_state(atn, &mut next, *target);
                    next.passed_non_greedy |= state.non_greedy;
                    close_config(atn, next, closure, semantic_predicate);
                }
            }
            Transition::Precedence { target, .. } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                next.passed_non_greedy |= state.non_greedy;
                close_config(atn, next, closure, semantic_predicate);
            }
            Transition::Action {
                target,
                action_index,
                rule_index,
                ..
            } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                next.passed_non_greedy |= state.non_greedy;
                if let Some(action_index) = action_index {
                    next.actions.push(LexerActionTrace {
                        action_index: *action_index,
                        position: config.position,
                        rule_index: *rule_index,
                    });
                }
                close_config(atn, next, closure, semantic_predicate);
            }
            Transition::Atom { .. }
            | Transition::Range { .. }
            | Transition::Set { .. }
            | Transition::NotSet { .. }
            | Transition::Wildcard { .. } => {}
        }
    }

    if state
        .transitions
        .iter()
        .any(|transition| !transition.is_epsilon())
    {
        closure.closed.push(config);
    }
}

/// Removes configs ordered after a non-greedy top-level accept for the same
/// lexer rule.
///
/// Non-greedy decisions serialize their exit path before their continuing path.
/// Once such a path reaches the rule stop state, later same-rule configs should
/// not continue to grow into a longer token. Greedy decisions still need all
/// paths to remain available so longest-match selection can win.
pub(super) fn prune_after_accepts(atn: &Atn, configs: Vec<LexerConfig>) -> Vec<LexerConfig> {
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
        if is_top_level_accept && config.passed_non_greedy {
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
pub(super) fn best_accept(atn: &Atn, configs: &[LexerConfig]) -> Option<AcceptState> {
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

/// Returns the token type predicted by an accepting lexer config set, if any.
fn accept_prediction(atn: &Atn, configs: &[LexerConfig]) -> Option<i32> {
    best_accept(atn, configs)
        .and_then(|accept| atn.rule_to_token_type().get(accept.rule_index).copied())
}

/// Builds a stable DFA state identity from a lexer closure while ignoring the
/// absolute input position, matching ANTLR's cache shape rather than one input
/// occurrence.
fn lexer_dfa_key(configs: &[LexerConfig], token_start: usize) -> LexerDfaKey {
    LexerDfaKey::new(
        configs
            .iter()
            .map(|config| normalized_config_key(config, token_start))
            .collect::<Vec<_>>(),
    )
}

/// Normalizes a config for DFA-state identity without embedding its absolute
/// character offset in the current input.
fn normalized_config_key(config: &LexerConfig, token_start: usize) -> LexerDfaConfigKey {
    LexerDfaConfigKey::new(
        config.state,
        config.alt_rule_index,
        config.consumed_eof,
        config.passed_non_greedy,
        config.stack.clone(),
        config
            .actions
            .iter()
            .map(|action| {
                debug_assert!(
                    action.position >= token_start,
                    "lexer DFA action position must be relative to the current token"
                );
                LexerDfaActionKey {
                    action_index: action.action_index,
                    position_delta: action.position.saturating_sub(token_start),
                    rule_index: action.rule_index,
                }
            })
            .collect(),
    )
}

fn shared_config_position(configs: &[LexerConfig]) -> Option<usize> {
    let position = configs.first()?.position;
    configs
        .iter()
        .all(|config| config.position == position)
        .then_some(position)
}

fn cached_configs_to_configs(
    configs: &[LexerDfaConfigKey],
    token_start: usize,
    position: usize,
) -> Vec<LexerConfig> {
    configs
        .iter()
        .map(|config| LexerConfig {
            state: config.state,
            position,
            consumed_eof: config.consumed_eof,
            alt_rule_index: config.alt_rule_index,
            passed_non_greedy: config.passed_non_greedy,
            stack: config.stack.clone(),
            actions: config
                .actions
                .iter()
                .map(|action| LexerActionTrace {
                    action_index: action.action_index,
                    position: token_start + action.position_delta,
                    rule_index: action.rule_index,
                })
                .collect(),
        })
        .collect()
}

/// Moves a lexer config to `state_number` and records the top-level lexer rule
/// once the config leaves a mode start state.
pub(super) fn set_config_state(atn: &Atn, config: &mut LexerConfig, state_number: usize) {
    config.state = state_number;
    if config.alt_rule_index.is_none() {
        config.alt_rule_index = atn.state(state_number).and_then(|state| state.rule_index);
    }
}

/// Buffers ANTLR's default diagnostic for one unmatchable input span.
fn record_token_recognition_error<I, F>(lexer: &mut BaseLexer<I, F>, start: usize, stop: usize)
where
    I: CharStream,
    F: TokenFactory,
{
    let stop = stop.saturating_sub(1);
    let text = display_error_text(&lexer.input().text(TextInterval::new(start, stop)));
    lexer.record_error(
        lexer.line(),
        lexer.column(),
        format!("token recognition error at: '{text}'"),
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
    use crate::atn::AtnType;
    use crate::atn::serialized::{AtnDeserializer, SerializedAtn};
    use crate::char_stream::InputStream;
    use crate::recognizer::RecognizerData;
    use crate::token::{TOKEN_EOF, Token};
    use crate::vocabulary::Vocabulary;

    #[test]
    fn predicate_sensitive_lexer_state_is_not_replay_cached() {
        let atn = Atn::new(AtnType::Lexer, 1);
        let data = RecognizerData::new(
            "T",
            Vocabulary::new([None, Some("T")], [None, Some("T")], [None::<&str>, None]),
        );
        let lexer = BaseLexer::new(InputStream::new(""), data);

        let predicate_state = cache_dfa_state(&lexer, &atn, &[], true, 0, 0);
        assert!(lexer.cached_lexer_dfa_state(predicate_state).is_none());

        let plain_state = cache_dfa_state(&lexer, &atn, &[], false, 0, 0);
        assert_eq!(predicate_state, plain_state);
        assert!(lexer.cached_lexer_dfa_state(plain_state).is_some());
    }

    #[test]
    fn lexer_matches_longest_token_and_skips() {
        let atn = AtnDeserializer::new(&SerializedAtn::from_i32(&[
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
        let atn = AtnDeserializer::new(&SerializedAtn::from_i32(&[
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
