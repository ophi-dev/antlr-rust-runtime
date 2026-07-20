use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;

use crate::atn::lexer_dfa::{
    CompiledLexerAccept, CompiledLexerContext, CompiledLexerContinuation, CompiledLexerDfa,
    DEAD_STATE, ESCAPE_STATE,
};
use crate::atn::{AtnStateKind, LexerAction, LexerAtn, LexerTransition};
use crate::char_stream::{CharStream, TextInterval};
use crate::int_stream::EOF;
use crate::lexer::{
    BaseLexer, EMPTY_LEXER_CONTEXT, Lexer, LexerContextArena, LexerContextId, LexerContextNode,
    LexerCustomAction, LexerDfaActionKey, LexerDfaCachedAccept, LexerDfaCachedState,
    LexerDfaCachedTransition, LexerDfaConfigKey, LexerDfaKey, LexerLifecycleCtx, LexerPredicate,
    LexerSemCtx,
};
use crate::parser::{SemanticHooks, UnknownSemanticPolicy};
use crate::prediction::{PredictionFxHasher, PredictionWorkspace};
use crate::token::{INVALID_TOKEN_TYPE, TokenId, TokenSink, TokenStoreError};

#[allow(clippy::disallowed_types)]
type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<PredictionFxHasher>>;

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
    pub(super) context: LexerContextId,
    pub(super) actions: Vec<LexerActionTrace>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct LexerConfigKey {
    state: usize,
    position: usize,
    consumed_eof: bool,
    alt_rule_index: Option<usize>,
    passed_non_greedy: bool,
    actions: Vec<LexerActionTrace>,
}

impl From<&LexerConfig> for LexerConfigKey {
    fn from(config: &LexerConfig) -> Self {
        Self {
            state: config.state,
            position: config.position,
            consumed_eof: config.consumed_eof,
            alt_rule_index: config.alt_rule_index,
            passed_non_greedy: config.passed_non_greedy,
            actions: config.actions.clone(),
        }
    }
}

/// Ordered lexer configurations with graph-structured caller contexts.
///
/// Configurations which differ only by their caller path are behaviorally
/// equivalent until a rule stop pops that path, so retaining one config with
/// the union of those paths avoids materializing every concrete call stack.
#[derive(Debug, Default)]
struct LexerConfigSet {
    configs: Vec<LexerConfig>,
    config_index: FxHashMap<LexerConfigKey, usize>,
}

impl LexerConfigSet {
    fn add(
        &mut self,
        config: LexerConfig,
        contexts: &mut LexerContextArena,
        workspace: &mut PredictionWorkspace,
    ) {
        let key = LexerConfigKey::from(&config);
        if let Some(&index) = self.config_index.get(&key) {
            let existing = self.configs[index].context;
            self.configs[index].context = contexts.merge(existing, config.context, workspace);
            return;
        }
        self.config_index.insert(key, self.configs.len());
        self.configs.push(config);
    }

    fn into_configs(self) -> Vec<LexerConfig> {
        self.configs
    }
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

/// Applies one deserialized lexer action to the shared in-progress token state.
///
/// Keeping type and channel on [`BaseLexer`] gives portable commands and custom
/// hooks the same mutation surface, matching ANTLR's `Lexer._type` /
/// `Lexer._channel` model.
fn apply_lexer_action<I>(action: &LexerAction, lexer: &mut BaseLexer<I>)
where
    I: CharStream,
{
    match action {
        LexerAction::Channel(channel) => lexer.set_channel(*channel),
        LexerAction::Custom { .. } => {}
        LexerAction::Mode(mode) => lexer.set_mode(*mode),
        LexerAction::More => lexer.more(),
        LexerAction::PopMode => {
            lexer.pop_mode();
        }
        LexerAction::PushMode(mode) => lexer.push_mode(*mode),
        LexerAction::Skip => lexer.skip(),
        LexerAction::Type(token_type) => lexer.set_type(*token_type),
    }
}

fn refresh_hit_eof<I>(lexer: &mut BaseLexer<I>) -> bool
where
    I: CharStream,
{
    let hit_eof = lexer.input().index() >= lexer.input().size();
    lexer.set_hit_eof(hit_eof);
    hit_eof
}

/// Accumulates one epsilon-closure expansion, including whether predicate
/// evaluation made the closure input-position-sensitive.
struct ClosureState {
    expanded: FxHashMap<LexerConfigKey, LexerContextId>,
    closed: LexerConfigSet,
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
pub fn next_token<I>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
{
    next_token_with_cache(lexer, sink, atn, |_, _| {}, |_, _| true, |_, _, _| {})
}

/// Runs one lexer-token match and invokes `custom_action` for embedded
/// grammar-specific lexer actions on the accepted path.
///
/// The callback receives the base lexer plus the serialized custom-action
/// coordinates. It is used by generated lexers to replay target templates while
/// keeping all ATN path exploration in the shared runtime.
pub fn next_token_with_actions<I, A>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    custom_action: A,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    A: FnMut(&mut BaseLexer<I>, LexerCustomAction),
{
    next_token_with_hooks(lexer, sink, atn, custom_action, |_, _| true, |_, _, _| {})
}

/// Runs one lexer-token match and lets generated code adjust the final accept
/// position before the token is emitted.
///
/// Generated lexer extensions use this to accept a long disambiguating token
/// path but emit only a prefix, leaving the remaining characters for the next
/// token.
pub fn next_token_with_accept_adjuster<I, E>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    accept_adjuster: E,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    E: FnMut(&mut BaseLexer<I>, i32, usize),
{
    next_token_with_hooks(lexer, sink, atn, |_, _| {}, |_, _| true, accept_adjuster)
}

/// Runs one lexer-token match with grammar-specific actions and predicates.
///
/// Predicates are evaluated during ATN closure construction so non-viable
/// paths are rejected before longest-match and lexer-rule priority selection.
pub fn next_token_with_actions_and_predicates<I, A, P>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    mut custom_action: A,
    mut semantic_predicate: P,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    A: FnMut(&mut BaseLexer<I>, LexerCustomAction),
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
{
    next_token_with_hooks(
        lexer,
        sink,
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
pub fn next_token_with_hooks<I, A, P, E>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    mut custom_action: A,
    mut semantic_predicate: P,
    mut accept_adjuster: E,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    A: FnMut(&mut BaseLexer<I>, LexerCustomAction),
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
    E: FnMut(&mut BaseLexer<I>, i32, usize),
{
    next_token_with_hooks_impl(
        lexer,
        sink,
        atn,
        &mut custom_action,
        &mut semantic_predicate,
        &mut |_| {},
        &mut accept_adjuster,
        &mut |_, _| {},
        LexerMatchStrategy {
            compiled: None,
            use_cache: false,
        },
    )
}

/// Dispatches one lexer custom action to a shared [`SemanticHooks`] object.
///
/// Both `next_token_*_with_semantic_hooks` entry points wire the interpreter's
/// action callback to this, so the `RefCell` borrow + [`LexerSemCtx`] plumbing
/// lives in one place instead of being copied per entry point.
fn dispatch_lexer_action_hook<I, H>(
    hooks: &RefCell<&mut H>,
    lexer: &mut BaseLexer<I>,
    action: LexerCustomAction,
) -> bool
where
    I: CharStream,
    H: SemanticHooks,
{
    let Ok(rule_index) = usize::try_from(action.rule_index()) else {
        return false;
    };
    let Ok(action_index) = usize::try_from(action.action_index()) else {
        return false;
    };
    // A custom action runs on the committed path, so it gets a mutable lexer
    // borrow: a `lexer_action` hook can change the mode stack
    // (`push_mode`/`pop_mode`/`set_mode`), matching the closure-based
    // `custom_action` API that also receives `&mut BaseLexer`.
    let mut ctx = LexerSemCtx::new_mut(lexer, rule_index, action_index, action.position());
    hooks.borrow_mut().lexer_action(&mut ctx, action)
}

/// Dispatches one lexer semantic predicate to a shared [`SemanticHooks`]
/// object, defaulting unknown coordinates to `true` (the historical closure
/// default) when the hook declines with `None`.
fn dispatch_lexer_predicate_hook<I, H>(
    hooks: &RefCell<&mut H>,
    lexer: &BaseLexer<I>,
    predicate: LexerPredicate,
) -> Option<bool>
where
    I: CharStream,
    H: SemanticHooks,
{
    let mut ctx = LexerSemCtx::new(
        lexer,
        predicate.rule_index(),
        predicate.pred_index(),
        predicate.position(),
    );
    hooks
        .borrow_mut()
        .lexer_sempred(&mut ctx, predicate.rule_index(), predicate.pred_index())
}

fn dispatch_lexer_before_token_hook<I, H>(hooks: &RefCell<&mut H>, lexer: &mut BaseLexer<I>)
where
    I: CharStream,
    H: SemanticHooks,
{
    let mut ctx = LexerLifecycleCtx::new(lexer, None);
    hooks.borrow_mut().lexer_before_token(&mut ctx);
}

fn dispatch_lexer_after_accept_hook<I, H>(
    hooks: &RefCell<&mut H>,
    lexer: &mut BaseLexer<I>,
    accept_position: usize,
) where
    I: CharStream,
    H: SemanticHooks,
{
    let mut ctx = LexerLifecycleCtx::new(lexer, Some(accept_position));
    hooks.borrow_mut().lexer_after_accept(&mut ctx);
}

/// Resets a base lexer and its caller-owned lifecycle state for reuse.
pub fn reset_with_semantic_hooks<I, H>(lexer: &mut BaseLexer<I>, hooks: &mut H)
where
    I: CharStream,
    H: SemanticHooks,
{
    lexer.reset();
    let mut ctx = LexerLifecycleCtx::new(lexer, None);
    hooks.lexer_reset(&mut ctx);
}

/// Replaces a base lexer's input and resets caller-owned lifecycle state.
pub fn set_input_stream_with_semantic_hooks<I, H>(lexer: &mut BaseLexer<I>, hooks: &mut H, input: I)
where
    I: CharStream,
    H: SemanticHooks,
{
    lexer.set_input_stream(input);
    let mut ctx = LexerLifecycleCtx::new(lexer, None);
    hooks.lexer_reset(&mut ctx);
}

/// Runs one lexer-token match with a shared [`SemanticHooks`] object.
///
/// This is the trait-based facade over the historical lexer closure hooks.
/// Unknown lexer predicates default to `true` when the hook returns `None`,
/// matching the existing closure default; callers that need fail-loud behavior
/// can implement the hook to record and reject coordinates explicitly.
pub fn next_token_with_semantic_hooks<I, H>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    hooks: &mut H,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    H: SemanticHooks,
{
    let hooks = RefCell::new(hooks);
    let token = next_token_with_hooks_impl(
        lexer,
        sink,
        atn,
        &mut |lexer, action| {
            let _ = dispatch_lexer_action_hook(&hooks, lexer, action);
        },
        &mut |lexer, predicate| {
            dispatch_lexer_predicate_hook(&hooks, lexer, predicate).unwrap_or(true)
        },
        &mut |lexer| dispatch_lexer_before_token_hook(&hooks, lexer),
        &mut |_, _, _| {},
        &mut |lexer, accept_position| {
            dispatch_lexer_after_accept_hook(&hooks, lexer, accept_position);
        },
        LexerMatchStrategy {
            compiled: None,
            use_cache: false,
        },
    );
    let token = token?;
    hooks.borrow_mut().lexer_token_emitted(
        sink.view(token)
            .expect("lexer hook token should be present in its sink"),
    );
    Ok(token)
}

/// Runs one lexer-token match against an ahead-of-time compiled lexer DFA.
///
/// Tokens starting in a compiled mode are matched by walking static tables;
/// modes the compiler left dynamic fall back to cached ATN interpretation per
/// token, so behavior always matches [`next_token`].
pub fn next_token_compiled<I>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    dfa: &CompiledLexerDfa,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
{
    next_token_with_hooks_impl(
        lexer,
        sink,
        atn,
        &mut |_, _| {},
        &mut |_, _| true,
        &mut |_| {},
        &mut |_, _, _| {},
        &mut |_, _| {},
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
#[allow(clippy::too_many_arguments)]
pub fn next_token_compiled_with_hooks<I, A, P, E>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    dfa: &CompiledLexerDfa,
    mut custom_action: A,
    mut semantic_predicate: P,
    mut accept_adjuster: E,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    A: FnMut(&mut BaseLexer<I>, LexerCustomAction),
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
    E: FnMut(&mut BaseLexer<I>, i32, usize),
{
    next_token_with_hooks_impl(
        lexer,
        sink,
        atn,
        &mut custom_action,
        &mut semantic_predicate,
        &mut |_| {},
        &mut accept_adjuster,
        &mut |_, _| {},
        LexerMatchStrategy {
            compiled: Some(dfa),
            use_cache: false,
        },
    )
}

/// Runs one compiled-DFA lexer-token match with a shared [`SemanticHooks`]
/// object for dynamic predicate/action modes.
pub fn next_token_compiled_with_semantic_hooks<I, H>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    dfa: &CompiledLexerDfa,
    hooks: &mut H,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    H: SemanticHooks,
{
    let hooks = RefCell::new(hooks);
    let token = next_token_with_hooks_impl(
        lexer,
        sink,
        atn,
        &mut |lexer, action| {
            let _ = dispatch_lexer_action_hook(&hooks, lexer, action);
        },
        &mut |lexer, predicate| {
            dispatch_lexer_predicate_hook(&hooks, lexer, predicate).unwrap_or(true)
        },
        &mut |lexer| dispatch_lexer_before_token_hook(&hooks, lexer),
        &mut |_, _, _| {},
        &mut |lexer, accept_position| {
            dispatch_lexer_after_accept_hook(&hooks, lexer, accept_position);
        },
        LexerMatchStrategy {
            compiled: Some(dfa),
            use_cache: false,
        },
    );
    let token = token?;
    hooks.borrow_mut().lexer_token_emitted(
        sink.view(token)
            .expect("lexer hook token should be present in its sink"),
    );
    Ok(token)
}

/// Runs one interpreted lexer-token match by composing generated translations
/// with a caller-owned semantic hook object.
///
/// A generated action returning `true` owns the coordinate; otherwise it is
/// offered to [`SemanticHooks::lexer_action`]. A generated predicate returning
/// `Some` owns the coordinate; `None` falls through to
/// [`SemanticHooks::lexer_sempred`] and finally the selected unknown policy.
#[allow(clippy::too_many_arguments)]
pub fn next_token_with_semantic_dispatch<I, H, A, P, E>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    hooks: &mut H,
    mut generated_action: A,
    mut generated_predicate: P,
    unknown_policy: UnknownSemanticPolicy,
    mut accept_adjuster: E,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    H: SemanticHooks,
    A: FnMut(&mut BaseLexer<I>, LexerCustomAction) -> bool,
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> Option<bool>,
    E: FnMut(&mut BaseLexer<I>, i32, usize),
{
    let hooks = RefCell::new(hooks);
    let token = next_token_with_hooks_impl(
        lexer,
        sink,
        atn,
        &mut |lexer, action| {
            if !generated_action(lexer, action)
                && !dispatch_lexer_action_hook(&hooks, lexer, action)
                && unknown_policy == UnknownSemanticPolicy::Error
                && let (Ok(rule), Ok(index)) = (
                    usize::try_from(action.rule_index()),
                    usize::try_from(action.action_index()),
                )
            {
                lexer.record_semantic_error(true, rule, index);
            }
        },
        &mut |lexer, predicate| {
            generated_predicate(lexer, predicate)
                .or_else(|| dispatch_lexer_predicate_hook(&hooks, lexer, predicate))
                .unwrap_or_else(|| match unknown_policy {
                    UnknownSemanticPolicy::AssumeTrue => true,
                    UnknownSemanticPolicy::AssumeFalse => false,
                    UnknownSemanticPolicy::Error => {
                        lexer.record_semantic_error(
                            false,
                            predicate.rule_index(),
                            predicate.pred_index(),
                        );
                        false
                    }
                })
        },
        &mut |lexer| dispatch_lexer_before_token_hook(&hooks, lexer),
        &mut accept_adjuster,
        &mut |lexer, accept_position| {
            dispatch_lexer_after_accept_hook(&hooks, lexer, accept_position);
        },
        LexerMatchStrategy {
            compiled: None,
            use_cache: false,
        },
    );
    let token = token?;
    hooks.borrow_mut().lexer_token_emitted(
        sink.view(token)
            .expect("lexer hook token should be present in its sink"),
    );
    Ok(token)
}

/// Compiled-DFA counterpart of [`next_token_with_semantic_dispatch`].
#[allow(clippy::too_many_arguments)]
pub fn next_token_compiled_with_semantic_dispatch<I, H, A, P, E>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    dfa: &CompiledLexerDfa,
    hooks: &mut H,
    mut generated_action: A,
    mut generated_predicate: P,
    unknown_policy: UnknownSemanticPolicy,
    mut accept_adjuster: E,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    H: SemanticHooks,
    A: FnMut(&mut BaseLexer<I>, LexerCustomAction) -> bool,
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> Option<bool>,
    E: FnMut(&mut BaseLexer<I>, i32, usize),
{
    let hooks = RefCell::new(hooks);
    let token = next_token_with_hooks_impl(
        lexer,
        sink,
        atn,
        &mut |lexer, action| {
            if !generated_action(lexer, action)
                && !dispatch_lexer_action_hook(&hooks, lexer, action)
                && unknown_policy == UnknownSemanticPolicy::Error
                && let (Ok(rule), Ok(index)) = (
                    usize::try_from(action.rule_index()),
                    usize::try_from(action.action_index()),
                )
            {
                lexer.record_semantic_error(true, rule, index);
            }
        },
        &mut |lexer, predicate| {
            generated_predicate(lexer, predicate)
                .or_else(|| dispatch_lexer_predicate_hook(&hooks, lexer, predicate))
                .unwrap_or_else(|| match unknown_policy {
                    UnknownSemanticPolicy::AssumeTrue => true,
                    UnknownSemanticPolicy::AssumeFalse => false,
                    UnknownSemanticPolicy::Error => {
                        lexer.record_semantic_error(
                            false,
                            predicate.rule_index(),
                            predicate.pred_index(),
                        );
                        false
                    }
                })
        },
        &mut |lexer| dispatch_lexer_before_token_hook(&hooks, lexer),
        &mut accept_adjuster,
        &mut |lexer, accept_position| {
            dispatch_lexer_after_accept_hook(&hooks, lexer, accept_position);
        },
        LexerMatchStrategy {
            compiled: Some(dfa),
            use_cache: false,
        },
    );
    let token = token?;
    hooks.borrow_mut().lexer_token_emitted(
        sink.view(token)
            .expect("lexer hook token should be present in its sink"),
    );
    Ok(token)
}

fn next_token_with_cache<I, A, P, E>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    mut custom_action: A,
    mut semantic_predicate: P,
    mut accept_adjuster: E,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    A: FnMut(&mut BaseLexer<I>, LexerCustomAction),
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
    E: FnMut(&mut BaseLexer<I>, i32, usize),
{
    next_token_with_hooks_impl(
        lexer,
        sink,
        atn,
        &mut custom_action,
        &mut semantic_predicate,
        &mut |_| {},
        &mut accept_adjuster,
        &mut |_, _| {},
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
fn match_token_with_strategy<I, P>(
    lexer: &mut BaseLexer<I>,
    atn: &LexerAtn,
    mode: i32,
    start: usize,
    semantic_predicate: &mut P,
    strategy: LexerMatchStrategy<'_>,
) -> MatchResult
where
    I: CharStream,
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
{
    lexer.reset_lexer_prediction_workspace();
    if let Some(dfa) = strategy.compiled
        && !lexer.force_interpreted()
        && let Some(start_state) = dfa.mode_start(mode)
    {
        match match_token_compiled(lexer, dfa, start_state, start) {
            CompiledMatch::Complete(result) => return result,
            CompiledMatch::Resume(resume) => {
                if compiled_resume_matches_atn(atn, start, &resume) {
                    return match_token_from_continuation(
                        lexer,
                        atn,
                        mode,
                        start,
                        resume,
                        semantic_predicate,
                    );
                }
            }
            CompiledMatch::Restart => {}
        }
    }
    if strategy.use_cache {
        match_token_cached(lexer, atn, mode, start, semantic_predicate)
    } else {
        match_token(lexer, atn, mode, start, semantic_predicate)
    }
}

#[allow(clippy::too_many_arguments)]
fn next_token_with_hooks_impl<I, A, P, B, E, L>(
    lexer: &mut BaseLexer<I>,
    sink: &mut TokenSink<'_>,
    atn: &LexerAtn,
    custom_action: &mut A,
    semantic_predicate: &mut P,
    before_token: &mut B,
    accept_adjuster: &mut E,
    after_accept: &mut L,
    strategy: LexerMatchStrategy<'_>,
) -> Result<TokenId, TokenStoreError>
where
    I: CharStream,
    A: FnMut(&mut BaseLexer<I>, LexerCustomAction),
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
    B: FnMut(&mut BaseLexer<I>),
    E: FnMut(&mut BaseLexer<I>, i32, usize),
    L: FnMut(&mut BaseLexer<I>, usize),
{
    let mut continuing_more = false;
    loop {
        before_token(lexer);
        if !continuing_more {
            if let Some(token) = lexer.emit_pending_token(sink)? {
                return Ok(token);
            }
        }

        if lexer.hit_eof() {
            return lexer.emit_eof_or_pending(sink);
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
                lexer.commit_position(start, start);
                if lexer.input_mut().la(1) == EOF {
                    lexer.set_hit_eof(true);
                    return lexer.emit_eof_or_pending(sink);
                }
                record_token_recognition_error(lexer, start, stop);
                lexer.commit_position(start, stop);
                continuing_more = false;
                continue;
            }
        };

        lexer.commit_position(start, accept.position);

        let token_type = atn
            .rule_to_token_type()
            .get(accept.rule_index)
            .copied()
            .unwrap_or(INVALID_TOKEN_TYPE);
        lexer.set_type(token_type);
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
                    other => apply_lexer_action(other, lexer),
                }
            }
        }

        let action_token_type = lexer.token_type();
        if action_token_type == crate::lexer::SKIP || action_token_type == crate::lexer::MORE {
            after_accept(lexer, accept.position);
            let lifecycle_token_type = lexer.token_type();
            if lifecycle_token_type != crate::lexer::SKIP
                && lifecycle_token_type != crate::lexer::MORE
            {
                accept_adjuster(lexer, lifecycle_token_type, accept.position);
            }
        } else {
            accept_adjuster(lexer, action_token_type, accept.position);
            after_accept(lexer, accept.position);
        }

        let token_type = lexer.token_type();
        if token_type == crate::lexer::SKIP || token_type == crate::lexer::MORE {
            if accept.consumed_eof || lexer.input().index() != accept.position {
                refresh_hit_eof(lexer);
            }
            continuing_more = token_type == crate::lexer::MORE;
            continue;
        }
        let emit_position = lexer.input().index();
        let hit_eof = if accept.consumed_eof || emit_position != accept.position {
            refresh_hit_eof(lexer)
        } else {
            false
        };
        let stop = emit_position.checked_sub(1).unwrap_or(usize::MAX);
        let text = if hit_eof && accept.consumed_eof && start == emit_position {
            Some("<EOF>".to_owned())
        } else {
            None
        };
        return lexer.emit_or_enqueue_with_stop(sink, stop, text);
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
    atn: &LexerAtn,
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
fn match_token<I, P>(
    lexer: &mut BaseLexer<I>,
    atn: &LexerAtn,
    mode: i32,
    start: usize,
    semantic_predicate: &mut P,
) -> MatchResult
where
    I: CharStream,
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
{
    let Some(mode_index) = usize::try_from(mode).ok() else {
        return MatchResult::NoViableAlt { stop: start };
    };
    let Some(start_state) = atn.mode_to_start_state().get(mode_index).copied() else {
        return MatchResult::NoViableAlt { stop: start };
    };
    let start_closure = epsilon_closure_with_lexer(
        lexer,
        atn,
        [LexerConfig {
            state: start_state,
            position: start,
            consumed_eof: false,
            alt_rule_index: None,
            passed_non_greedy: false,
            context: EMPTY_LEXER_CONTEXT,
            actions: Vec::new(),
        }],
        semantic_predicate,
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
        let position = active[0].position;
        debug_assert!(
            active.iter().all(|config| config.position == position),
            "lexer ATN configs must advance through the input in lockstep"
        );
        let symbol = symbol_at(lexer, position);
        if symbol != EOF {
            error_stop = error_stop.max(position.saturating_add(1));
        }
        let mut next = Vec::new();
        let source_dfa_state = dfa_state;
        let source_has_semantic_context = dfa_state_has_semantic_context;
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

        let closure = epsilon_closure_with_lexer(lexer, atn, next, semantic_predicate);
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
                if symbol != EOF {
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

fn match_token_cached<I, P>(
    lexer: &mut BaseLexer<I>,
    atn: &LexerAtn,
    mode: i32,
    start: usize,
    semantic_predicate: &mut P,
) -> MatchResult
where
    I: CharStream,
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
{
    let Some((dfa_state, mode_start_has_semantic_context)) =
        cached_mode_start_state(lexer, atn, mode, start, semantic_predicate)
    else {
        return MatchResult::NoViableAlt { stop: start };
    };
    if mode_start_has_semantic_context {
        return match_token(lexer, atn, mode, start, semantic_predicate);
    }

    match_token_cached_from_state(
        lexer,
        atn,
        mode,
        start,
        dfa_state,
        start,
        None,
        start,
        semantic_predicate,
    )
}

#[allow(clippy::too_many_arguments)]
fn match_token_cached_from_state<I, P>(
    lexer: &mut BaseLexer<I>,
    atn: &LexerAtn,
    mode: i32,
    start: usize,
    mut dfa_state: usize,
    mut position: usize,
    mut best: Option<AcceptState>,
    mut error_stop: usize,
    semantic_predicate: &mut P,
) -> MatchResult
where
    I: CharStream,
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
{
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

        let closure = epsilon_closure_with_lexer(lexer, atn, next, semantic_predicate);
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

enum CompiledMatch<'a> {
    Complete(MatchResult),
    Resume(CompiledResume<'a>),
    Restart,
}

struct CompiledResume<'a> {
    continuation: &'a CompiledLexerContinuation,
    position: usize,
    best: Option<AcceptState>,
    error_stop: usize,
}

fn compiled_resume_matches_atn(atn: &LexerAtn, start: usize, resume: &CompiledResume<'_>) -> bool {
    let Some(consumed) = resume.position.checked_sub(start) else {
        return false;
    };
    let rule_count = atn.rule_to_token_type().len();
    let contexts_valid = resume
        .continuation
        .contexts
        .iter()
        .enumerate()
        .all(|(index, context)| {
            let local_id = u32::try_from(index + 1).ok();
            local_id.is_some_and(|local_id| match context {
                CompiledLexerContext::Singleton {
                    parent,
                    return_state,
                } => *parent < local_id && atn.state(*return_state).is_some(),
                CompiledLexerContext::Union { left, right } => {
                    *left < local_id && *right < local_id
                }
            })
        });
    contexts_valid
        && !resume.continuation.configs.is_empty()
        && resume.continuation.configs.iter().all(|config| {
            atn.state(config.state).is_some()
                && config
                    .alt_rule_index
                    .is_some_and(|rule_index| rule_index < rule_count)
                && usize::try_from(config.context)
                    .is_ok_and(|context| context <= resume.continuation.contexts.len())
                && config.actions.iter().all(|action| {
                    action.action_index < atn.lexer_actions().len()
                        && action.rule_index < rule_count
                        && action.behind <= consumed
                })
        })
}

/// Matches one token by walking the ahead-of-time compiled lexer DFA.
///
/// The walk reproduces the interpreter's longest-match selection: remember
/// the best accept seen so far, advance until the table has no transition,
/// then return the remembered accept — or a recognition error spanning every
/// character the walk looked at, exactly like `match_token`. Reaching an
/// escape edge resumes the interpreter from its narrowed configs when present;
/// budget-only escapes restart from the token boundary.
fn match_token_compiled<'a, I>(
    lexer: &mut BaseLexer<I>,
    dfa: &'a CompiledLexerDfa,
    start_state: u16,
    start: usize,
) -> CompiledMatch<'a>
where
    I: CharStream,
{
    if let Some(input) = lexer.input().contiguous_ascii() {
        return match_token_compiled_ascii(input, dfa, start_state, start);
    }
    match_token_compiled_generic(lexer, dfa, start_state, start)
}

fn match_token_compiled_ascii<'a>(
    input: &[u8],
    dfa: &'a CompiledLexerDfa,
    start_state: u16,
    start: usize,
) -> CompiledMatch<'a> {
    const MIN_RUN_SCAN_PREFIX: usize = 8;

    let mut state = start_state;
    let mut position = start;
    let mut best: Option<AcceptState> = None;
    let mut error_stop = start;
    let mut eof_edges = 0_u32;
    let mut self_loop_prefix = 0;
    #[cfg(feature = "perf-counters")]
    let mut direct_chars = 0;
    #[cfg(feature = "perf-counters")]
    let mut scalar_chars = 0;
    let result = loop {
        if let Some(accept) = dfa.accept(state) {
            record_compiled_accept(accept, position, &mut best);
        }
        if position < input.len() && self_loop_prefix == MIN_RUN_SCAN_PREFIX {
            if let Some(scan) = dfa.scan_ascii_run(state, &input[position..]) {
                #[cfg(feature = "perf-counters")]
                {
                    direct_chars += scan.bytes;
                    crate::perf::record_lexer_run_scan(scan.bytes, scan.found_exit);
                    if let Some(range) = scan.range {
                        crate::perf::record_lexer_range_scan(range.class(), scan.bytes);
                    }
                }
                position += scan.bytes;
                error_stop = error_stop.max(position);
                if scan.bytes > 0
                    && let Some(accept) = dfa.accept(state)
                {
                    record_compiled_accept(accept, position, &mut best);
                }
            } else {
                self_loop_prefix += 1;
                #[cfg(feature = "perf-counters")]
                crate::perf::record_lexer_run_rejected_state();
            }
        }
        let (target, at_eof) = if position < input.len() {
            let symbol = input[position];
            #[cfg(feature = "perf-counters")]
            {
                direct_chars += 1;
                scalar_chars += 1;
            }
            error_stop = error_stop.max(position.saturating_add(1));
            (dfa.ascii_target(state, symbol), false)
        } else {
            eof_edges += 1;
            if eof_edges > MAX_COMPILED_EOF_EDGES {
                break CompiledMatch::Restart;
            }
            (dfa.eof_target(state), true)
        };
        if target == DEAD_STATE {
            break CompiledMatch::Complete(best.map_or(
                MatchResult::NoViableAlt { stop: error_stop },
                MatchResult::Accept,
            ));
        }
        if target == ESCAPE_STATE {
            let continuation = if at_eof {
                dfa.eof_continuation(state)
            } else {
                dfa.char_continuation(state, i32::from(input[position]))
            };
            break continuation.map_or(CompiledMatch::Restart, |continuation| {
                CompiledMatch::Resume(CompiledResume {
                    continuation,
                    position: position + usize::from(!at_eof),
                    best,
                    error_stop,
                })
            });
        }
        if !at_eof {
            position += 1;
            if target == state {
                if self_loop_prefix < MIN_RUN_SCAN_PREFIX {
                    self_loop_prefix += 1;
                }
            } else {
                self_loop_prefix = 0;
            }
        }
        state = target;
    };
    #[cfg(feature = "perf-counters")]
    {
        crate::perf::record_lexer_direct_ascii(direct_chars);
        crate::perf::record_lexer_compiled_scalar_ascii(scalar_chars);
    }
    result
}

fn match_token_compiled_generic<'a, I>(
    lexer: &mut BaseLexer<I>,
    dfa: &'a CompiledLexerDfa,
    start_state: u16,
    start: usize,
) -> CompiledMatch<'a>
where
    I: CharStream,
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
                return CompiledMatch::Restart;
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
            let continuation = if symbol == EOF {
                dfa.eof_continuation(state)
            } else {
                dfa.char_continuation(state, symbol)
            };
            return continuation.map_or(CompiledMatch::Restart, |continuation| {
                CompiledMatch::Resume(CompiledResume {
                    continuation,
                    position: position + usize::from(symbol != EOF),
                    best,
                    error_stop,
                })
            });
        }
        if symbol != EOF {
            position += 1;
        }
        state = target;
    }
    CompiledMatch::Complete(best.map_or(
        MatchResult::NoViableAlt { stop: error_stop },
        MatchResult::Accept,
    ))
}

fn match_token_from_continuation<I, P>(
    lexer: &mut BaseLexer<I>,
    atn: &LexerAtn,
    mode: i32,
    start: usize,
    resume: CompiledResume<'_>,
    semantic_predicate: &mut P,
) -> MatchResult
where
    I: CharStream,
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
{
    let CompiledResume {
        continuation,
        position,
        mut best,
        error_stop,
    } = resume;
    let moved = {
        let mut prediction = lexer.lexer_prediction_store();
        let prediction = &mut *prediction;
        let mut contexts = Vec::with_capacity(continuation.contexts.len() + 1);
        contexts.push(EMPTY_LEXER_CONTEXT);
        for compiled in &continuation.contexts {
            let imported = match *compiled {
                CompiledLexerContext::Singleton {
                    parent,
                    return_state,
                } => prediction
                    .contexts
                    .singleton(contexts[parent as usize], return_state),
                CompiledLexerContext::Union { left, right } => prediction.contexts.merge(
                    contexts[left as usize],
                    contexts[right as usize],
                    &mut prediction.workspace,
                ),
            };
            contexts.push(imported);
        }
        continuation
            .configs
            .iter()
            .map(|config| LexerConfig {
                state: config.state,
                position,
                consumed_eof: config.consumed_eof,
                alt_rule_index: config.alt_rule_index,
                passed_non_greedy: config.passed_non_greedy,
                context: contexts[config.context as usize],
                actions: config
                    .actions
                    .iter()
                    .map(|action| LexerActionTrace {
                        action_index: action.action_index,
                        position: position.saturating_sub(action.behind),
                        rule_index: action.rule_index,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>()
    };
    let closure = epsilon_closure_with_lexer(lexer, atn, moved, semantic_predicate);
    if closure.has_semantic_context {
        return match_token(lexer, atn, mode, start, semantic_predicate);
    }
    let active = prune_after_accepts(atn, closure.configs);
    update_best_accept(atn, &active, &mut best);
    if active.is_empty() {
        return best.map_or(
            MatchResult::NoViableAlt { stop: error_stop },
            MatchResult::Accept,
        );
    }
    let Some(position) = shared_config_position(&active) else {
        return match_token(lexer, atn, mode, start, semantic_predicate);
    };
    let dfa_state = cache_dfa_state(lexer, atn, &active, false, start, position);
    match_token_cached_from_state(
        lexer,
        atn,
        mode,
        start,
        dfa_state,
        position,
        best,
        error_stop,
        semantic_predicate,
    )
}

fn update_best_accept(atn: &LexerAtn, active: &[LexerConfig], best: &mut Option<AcceptState>) {
    let Some(accept) = best_accept(atn, active) else {
        return;
    };
    if best.as_ref().is_none_or(|current| {
        accept.position > current.position
            || (accept.position == current.position && accept.rule_index < current.rule_index)
    }) {
        *best = Some(accept);
    }
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

fn cached_mode_start_state<I, P>(
    lexer: &BaseLexer<I>,
    atn: &LexerAtn,
    mode: i32,
    start: usize,
    semantic_predicate: &mut P,
) -> Option<(usize, bool)>
where
    I: CharStream,
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
{
    if let Some(state) = lexer.cached_lexer_mode_start(mode) {
        return Some((state, false));
    }

    let mode_index = usize::try_from(mode).ok()?;
    let start_state = atn.mode_to_start_state().get(mode_index).copied()?;
    let start_closure = epsilon_closure_with_lexer(
        lexer,
        atn,
        [LexerConfig {
            state: start_state,
            position: start,
            consumed_eof: false,
            alt_rule_index: None,
            passed_non_greedy: false,
            context: EMPTY_LEXER_CONTEXT,
            actions: Vec::new(),
        }],
        semantic_predicate,
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

fn cache_dfa_state<I>(
    lexer: &BaseLexer<I>,
    atn: &LexerAtn,
    active: &[LexerConfig],
    has_semantic_context: bool,
    token_start: usize,
    position: usize,
) -> usize
where
    I: CharStream,
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

fn epsilon_closure_with_lexer<I, P>(
    lexer: &BaseLexer<I>,
    atn: &LexerAtn,
    configs: impl IntoIterator<Item = LexerConfig>,
    semantic_predicate: &mut P,
) -> ClosureResult
where
    I: CharStream,
    P: FnMut(&BaseLexer<I>, LexerPredicate) -> bool,
{
    let mut prediction = lexer.lexer_prediction_store();
    let prediction = &mut *prediction;
    epsilon_closure(
        atn,
        configs,
        &mut prediction.contexts,
        &mut prediction.workspace,
        &mut |predicate| semantic_predicate(lexer, predicate),
    )
}

/// Expands epsilon, rule-call, predicate, precedence, and action transitions
/// without consuming input.
///
/// Lexer rule calls use graph-structured caller contexts. Equivalent configs
/// merge their contexts in the returned ordered set, retaining all return
/// paths without cloning every concrete stack.
pub(super) fn epsilon_closure<P>(
    atn: &LexerAtn,
    configs: impl IntoIterator<Item = LexerConfig>,
    contexts: &mut LexerContextArena,
    workspace: &mut PredictionWorkspace,
    semantic_predicate: &mut P,
) -> ClosureResult
where
    P: FnMut(LexerPredicate) -> bool,
{
    let mut state = ClosureState {
        expanded: FxHashMap::default(),
        closed: LexerConfigSet::default(),
        has_semantic_context: false,
    };

    for config in configs {
        close_config(
            atn,
            config,
            contexts,
            workspace,
            &mut state,
            semantic_predicate,
        );
    }

    ClosureResult {
        configs: state.closed.into_configs(),
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
    atn: &LexerAtn,
    config: LexerConfig,
    contexts: &mut LexerContextArena,
    workspace: &mut PredictionWorkspace,
    closure: &mut ClosureState,
    semantic_predicate: &mut P,
) where
    P: FnMut(LexerPredicate) -> bool,
{
    let key = LexerConfigKey::from(&config);
    if let Some(existing) = closure.expanded.get(&key).copied() {
        let merged = contexts.merge(existing, config.context, workspace);
        if merged == existing {
            return;
        }
        closure.expanded.insert(key, merged);
        // `config` is the newly discovered ordered delta. Re-expanding the
        // merged prefix would duplicate earlier paths ahead of this one.
    } else {
        closure.expanded.insert(key, config.context);
    }

    let Some(state) = atn.state(config.state) else {
        return;
    };

    if state.kind == AtnStateKind::RuleStop {
        let mut pending = vec![config.context];
        let mut visited = FxHashSet::default();
        while let Some(context) = pending.pop() {
            if !visited.insert(context) {
                continue;
            }
            match contexts.node(context) {
                LexerContextNode::Empty => {
                    let mut accepted = config.clone();
                    accepted.context = EMPTY_LEXER_CONTEXT;
                    closure.closed.add(accepted, contexts, workspace);
                }
                LexerContextNode::Singleton {
                    parent,
                    return_state,
                } => {
                    let mut returned = config.clone();
                    set_config_state(atn, &mut returned, return_state);
                    returned.context = parent;
                    close_config(
                        atn,
                        returned,
                        contexts,
                        workspace,
                        closure,
                        semantic_predicate,
                    );
                }
                LexerContextNode::Union { left, right } => {
                    pending.push(right);
                    pending.push(left);
                }
            }
        }
        return;
    }

    for transition in &state.transitions {
        match transition {
            LexerTransition::Epsilon { target } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                next.passed_non_greedy |= state.non_greedy;
                close_config(atn, next, contexts, workspace, closure, semantic_predicate);
            }
            LexerTransition::Rule {
                target,
                follow_state,
                ..
            } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                next.passed_non_greedy |= state.non_greedy;
                next.context = contexts.singleton(config.context, *follow_state);
                close_config(atn, next, contexts, workspace, closure, semantic_predicate);
            }
            LexerTransition::Predicate {
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
                    close_config(atn, next, contexts, workspace, closure, semantic_predicate);
                }
            }
            LexerTransition::Precedence { target, .. } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                next.passed_non_greedy |= state.non_greedy;
                close_config(atn, next, contexts, workspace, closure, semantic_predicate);
            }
            LexerTransition::Action {
                target,
                action_index,
                rule_index,
                ..
            } => {
                let mut next = config.clone();
                set_config_state(atn, &mut next, *target);
                next.passed_non_greedy |= state.non_greedy;
                if let Some(action_index) = action_index {
                    let trace = LexerActionTrace {
                        action_index: *action_index,
                        position: config.position,
                        rule_index: *rule_index,
                    };
                    let keep = next.alt_rule_index.is_none_or(|accept_rule| {
                        lexer_action_belongs_to_accept(atn, accept_rule, *rule_index)
                    });
                    if keep {
                        append_lexer_action_trace(atn, &mut next.actions, trace);
                    }
                }
                close_config(atn, next, contexts, workspace, closure, semantic_predicate);
            }
            LexerTransition::Atom { .. }
            | LexerTransition::Range { .. }
            | LexerTransition::Set { .. }
            | LexerTransition::NotSet { .. }
            | LexerTransition::Wildcard { .. } => {}
        }
    }

    if state
        .transitions
        .iter()
        .any(|transition| !transition.is_epsilon())
    {
        closure.closed.add(config, contexts, workspace);
    }
}

/// Appends one executable action while removing an earlier setter whose
/// result cannot be observed before this one overwrites it.
fn append_lexer_action_trace(
    atn: &LexerAtn,
    actions: &mut Vec<LexerActionTrace>,
    trace: LexerActionTrace,
) {
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Setter {
        Channel,
        Mode,
        TokenType,
    }

    const fn setter(action: &LexerAction) -> Option<Setter> {
        match action {
            LexerAction::Channel(_) => Some(Setter::Channel),
            LexerAction::Mode(_) => Some(Setter::Mode),
            LexerAction::More | LexerAction::Skip | LexerAction::Type(_) => Some(Setter::TokenType),
            LexerAction::Custom { .. } | LexerAction::PopMode | LexerAction::PushMode(_) => None,
        }
    }

    let Some(action) = atn.lexer_actions().get(trace.action_index) else {
        actions.push(trace);
        return;
    };
    let Some(slot) = setter(action) else {
        actions.push(trace);
        return;
    };
    for index in (0..actions.len()).rev() {
        let Some(previous) = atn.lexer_actions().get(actions[index].action_index) else {
            break;
        };
        if matches!(previous, LexerAction::Custom { .. })
            || (slot == Setter::Mode && matches!(previous, LexerAction::PushMode(_)))
        {
            break;
        }
        if setter(previous) == Some(slot) {
            actions.remove(index);
            break;
        }
    }
    actions.push(trace);
}

/// Removes lower-priority non-greedy configs ordered after a top-level accept
/// for the same lexer rule.
///
/// Non-greedy decisions serialize their exit path before their continuing path.
/// Once any earlier path reaches the rule stop state, later same-rule configs
/// that passed through a non-greedy decision must not continue to grow into a
/// longer token. Paths that never crossed a non-greedy decision remain
/// available so ordinary lexer longest-match selection can still win.
pub(super) fn prune_after_accepts(atn: &LexerAtn, configs: Vec<LexerConfig>) -> Vec<LexerConfig> {
    let mut accepted_rules = Vec::new();
    let mut pruned = Vec::with_capacity(configs.len());
    for config in configs {
        let Some(rule_index) = config.alt_rule_index else {
            pruned.push(config);
            continue;
        };
        if config.passed_non_greedy && accepted_rules.contains(&rule_index) {
            continue;
        }
        let is_top_level_accept = config.context == EMPTY_LEXER_CONTEXT
            && atn
                .state(config.state)
                .is_some_and(crate::atn::LexerAtnState::is_rule_stop);
        if is_top_level_accept && !accepted_rules.contains(&rule_index) {
            accepted_rules.push(rule_index);
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
pub(super) fn best_accept(atn: &LexerAtn, configs: &[LexerConfig]) -> Option<AcceptState> {
    configs
        .iter()
        .filter_map(|config| {
            let state = atn.state(config.state)?;
            if !state.is_rule_stop() || config.context != EMPTY_LEXER_CONTEXT {
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
fn accept_prediction(atn: &LexerAtn, configs: &[LexerConfig]) -> Option<i32> {
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
        config.context,
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
            context: config.context,
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
pub(super) fn set_config_state(atn: &LexerAtn, config: &mut LexerConfig, state_number: usize) {
    config.state = state_number;
    if config.alt_rule_index.is_none() {
        config.alt_rule_index = atn.state(state_number).and_then(|state| state.rule_index);
    }
}

/// Buffers ANTLR's default diagnostic for one unmatchable input span.
fn record_token_recognition_error<I>(lexer: &BaseLexer<I>, start: usize, stop: usize)
where
    I: CharStream,
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
/// Streams with immutable random access avoid touching their committed cursor;
/// custom streams retain the compatible seek-and-lookahead path.
fn symbol_at<I>(lexer: &mut BaseLexer<I>, position: usize) -> i32
where
    I: CharStream,
{
    let symbol = lexer.input().symbol_at(position).unwrap_or_else(|| {
        lexer.input_mut().seek(position);
        lexer.input_mut().la(1)
    });
    #[cfg(feature = "perf-counters")]
    if symbol != EOF {
        crate::perf::record_lexer_generic_char();
    }
    symbol
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atn::lexer_dfa::{
        CompiledLexerActionTrace, CompiledLexerConfig, CompiledLexerContext,
    };
    use crate::atn::serialized::{AtnDeserializer, SerializedAtn};
    use crate::atn::{LexerAtnState, LexerTransition};
    use crate::char_stream::InputStream;
    use crate::recognizer::RecognizerData;
    use crate::token::{DEFAULT_CHANNEL, HIDDEN_CHANNEL, TOKEN_EOF, Token, TokenStore, TokenView};
    use crate::vocabulary::Vocabulary;

    #[derive(Debug)]
    struct TokenSnapshot {
        token_type: i32,
        start: usize,
        stop: usize,
        text: String,
    }

    fn recognizer_data() -> RecognizerData {
        RecognizerData::new(
            "T",
            Vocabulary::new([None, Some("T")], [None, Some("T")], [None::<&str>, None]),
        )
    }

    // `BLOCK_COMMENT: ('/**/' | '/*' ~[!] .*? '*/'); OTHER: .;`
    //
    // `#[rustfmt::skip]`: this serialized ATN is a positional stream emitted by
    // ANTLR 4.13.2. Keeping it intact makes the regression exercise the exact
    // shared-prefix/non-greedy topology reported in issue #106.
    #[rustfmt::skip]
    fn shared_prefix_non_greedy_atn() -> LexerAtn {
        AtnDeserializer::new(&SerializedAtn::from_i32(&[
            4, 0, 2, 25, 6, -1, 2, 0, 7, 0, 2, 1, 7, 1, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 5, 0, 15, 8, 0, 10, 0, 12, 0, 18, 9, 0, 1, 0, 1, 0, 3, 0, 22, 8, 0, 1, 1, 1, 1, 1, 16, 0, 2, 1, 1, 3, 2, 1, 0, 1, 1, 0, 33, 33, 26, 0, 1, 1, 0, 0, 0, 0, 3, 1, 0, 0, 0, 1, 21, 1, 0, 0, 0, 3, 23, 1, 0, 0, 0, 5, 6, 5, 47, 0, 0, 6, 7, 5, 42, 0, 0, 7, 8, 5, 42, 0, 0, 8, 22, 5, 47, 0, 0, 9, 10, 5, 47, 0, 0, 10, 11, 5, 42, 0, 0, 11, 12, 1, 0, 0, 0, 12, 16, 8, 0, 0, 0, 13, 15, 9, 0, 0, 0, 14, 13, 1, 0, 0, 0, 15, 18, 1, 0, 0, 0, 16, 17, 1, 0, 0, 0, 16, 14, 1, 0, 0, 0, 17, 19, 1, 0, 0, 0, 18, 16, 1, 0, 0, 0, 19, 20, 5, 42, 0, 0, 20, 22, 5, 47, 0, 0, 21, 5, 1, 0, 0, 0, 21, 9, 1, 0, 0, 0, 22, 2, 1, 0, 0, 0, 23, 24, 9, 0, 0, 0, 24, 4, 1, 0, 0, 0, 3, 0, 16, 21, 0,
        ]))
        .deserialize()
        .expect("issue #106 lexer ATN should deserialize")
    }

    // `COMMENT: '/*' (COMMENT | .)*? '*/' -> channel(HIDDEN);`
    // `fragment Hidden: COMMENT; AT_PRE: Hidden '@'; OTHER: .;`
    //
    // This is the reduced issue #135 topology: recursive non-greedy comments
    // remain speculative through another token rule, and the referenced token
    // rule carries a built-in action.
    #[rustfmt::skip]
    fn recursive_comment_channel_atn() -> LexerAtn {
        AtnDeserializer::new(&SerializedAtn::from_i32(&[
            4, 0, 3, 31, 6, -1, 2, 0, 7, 0, 2, 1, 7, 1, 2, 2, 7, 2, 2, 3, 7, 3,
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 5, 0, 15, 8, 0, 10, 0, 12, 0, 18, 9, 0,
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 2, 1, 2, 1, 2, 1, 3,
            1, 3, 1, 16, 0, 4, 1, 1, 3, 0, 5, 2, 7, 3, 1, 0, 0, 31, 0, 1, 1, 0,
            0, 0, 0, 5, 1, 0, 0, 0, 0, 7, 1, 0, 0, 0, 1, 9, 1, 0, 0, 0, 3, 24,
            1, 0, 0, 0, 5, 26, 1, 0, 0, 0, 7, 29, 1, 0, 0, 0, 9, 10, 5, 47, 0,
            0, 10, 11, 5, 42, 0, 0, 11, 16, 1, 0, 0, 0, 12, 15, 3, 1, 0, 0, 13,
            15, 9, 0, 0, 0, 14, 12, 1, 0, 0, 0, 14, 13, 1, 0, 0, 0, 15, 18, 1,
            0, 0, 0, 16, 17, 1, 0, 0, 0, 16, 14, 1, 0, 0, 0, 17, 19, 1, 0, 0,
            0, 18, 16, 1, 0, 0, 0, 19, 20, 5, 42, 0, 0, 20, 21, 5, 47, 0, 0, 21,
            22, 1, 0, 0, 0, 22, 23, 6, 0, 0, 0, 23, 2, 1, 0, 0, 0, 24, 25, 3, 1,
            0, 0, 25, 4, 1, 0, 0, 0, 26, 27, 3, 3, 1, 0, 27, 28, 5, 64, 0, 0, 28,
            6, 1, 0, 0, 0, 29, 30, 9, 0, 0, 0, 30, 8, 1, 0, 0, 0, 3, 0, 14, 16,
            1, 0, 1, 0,
        ]))
        .deserialize()
        .expect("recursive-comment lexer ATN should deserialize")
    }

    #[derive(Clone, Copy, Debug)]
    enum TestMatchStrategy {
        Interpreted,
        Cached,
        Compiled,
    }

    fn lex_issue_106_comment(strategy: TestMatchStrategy) -> TokenSnapshot {
        let atn = shared_prefix_non_greedy_atn();
        let dfa = CompiledLexerDfa::compile(&atn);
        let data = RecognizerData::new(
            "Issue106Lexer",
            Vocabulary::new(
                [None::<&str>, None, None],
                [None, Some("BLOCK_COMMENT"), Some("OTHER")],
                [None::<&str>, None, None],
            ),
        );
        let mut lexer = BaseLexer::new(InputStream::new("/**/5 /*   */"), data);
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let token = match strategy {
            TestMatchStrategy::Interpreted => next_token_with_hooks(
                &mut lexer,
                &mut sink,
                &atn,
                |_, _| {},
                |_, _| true,
                |_, _, _| {},
            ),
            TestMatchStrategy::Cached => {
                next_token(&mut lexer, &mut sink, &atn)
                    .expect("first pass should populate the learned DFA");
                lexer.reset();
                next_token(&mut lexer, &mut sink, &atn)
            }
            TestMatchStrategy::Compiled => next_token_compiled(&mut lexer, &mut sink, &atn, &dfa),
        }
        .expect("comment token should fit");
        let token = sink.view(token).expect("comment token should exist");
        TokenSnapshot {
            token_type: token.token_type(),
            start: token.start(),
            stop: token.stop(),
            text: token.text().to_owned(),
        }
    }

    fn trailing_action_atn(
        labels: &[char],
        token_type: i32,
        actions: Vec<LexerAction>,
    ) -> LexerAtn {
        assert!(!labels.is_empty());
        let stop = labels.len() + actions.len() + 1;
        let mut atn = LexerAtn::new(token_type);
        for state in 0..=stop {
            let kind = match state {
                0 => AtnStateKind::TokenStart,
                1 => AtnStateKind::RuleStart,
                value if value == stop => AtnStateKind::RuleStop,
                _ => AtnStateKind::Basic,
            };
            let state = if state == 0 {
                LexerAtnState::new(state, kind)
            } else {
                LexerAtnState::new(state, kind).with_rule_index(0)
            };
            atn.add_state(state);
        }
        atn.state_mut(0)
            .expect("token start")
            .add_transition(LexerTransition::Epsilon { target: 1 });
        for (index, label) in labels.iter().enumerate() {
            atn.state_mut(index + 1)
                .expect("label source")
                .add_transition(LexerTransition::Atom {
                    target: index + 2,
                    label: u32::from(*label).cast_signed(),
                });
        }
        for index in 0..actions.len() {
            atn.state_mut(labels.len() + index + 1)
                .expect("action source")
                .add_transition(LexerTransition::Action {
                    target: labels.len() + index + 2,
                    rule_index: 0,
                    action_index: Some(index),
                    context_dependent: false,
                });
        }
        atn.set_rule_to_start_state(vec![1]);
        atn.set_rule_to_stop_state(vec![stop]);
        atn.set_rule_to_token_type(vec![token_type]);
        atn.add_mode_start_state(0);
        atn.add_decision_state(0);
        atn.set_lexer_actions(actions);
        atn
    }

    fn eof_rewind_action_atn() -> LexerAtn {
        let mut atn = LexerAtn::new(2);
        for (state_number, kind, rule_index) in [
            (0, AtnStateKind::TokenStart, None),
            (1, AtnStateKind::RuleStart, Some(0)),
            (2, AtnStateKind::Basic, Some(0)),
            (3, AtnStateKind::Basic, Some(0)),
            (4, AtnStateKind::Basic, Some(0)),
            (5, AtnStateKind::RuleStop, Some(0)),
            (6, AtnStateKind::RuleStart, Some(1)),
            (7, AtnStateKind::RuleStop, Some(1)),
        ] {
            let mut state = LexerAtnState::new(state_number, kind);
            if let Some(rule_index) = rule_index {
                state = state.with_rule_index(rule_index);
            }
            atn.add_state(state);
        }
        atn.state_mut(0)
            .expect("token start")
            .add_transition(LexerTransition::Epsilon { target: 1 });
        atn.state_mut(0)
            .expect("token start")
            .add_transition(LexerTransition::Epsilon { target: 6 });
        atn.state_mut(1)
            .expect("prefix rule start")
            .add_transition(LexerTransition::Atom {
                target: 2,
                label: 'a' as i32,
            });
        atn.state_mut(2)
            .expect("prefix rule body")
            .add_transition(LexerTransition::Atom {
                target: 3,
                label: 'b' as i32,
            });
        atn.state_mut(3)
            .expect("prefix rule EOF")
            .add_transition(LexerTransition::Atom {
                target: 4,
                label: EOF,
            });
        atn.state_mut(4)
            .expect("prefix rule action")
            .add_transition(LexerTransition::Action {
                target: 5,
                rule_index: 0,
                action_index: Some(0),
                context_dependent: false,
            });
        atn.state_mut(6)
            .expect("suffix rule start")
            .add_transition(LexerTransition::Atom {
                target: 7,
                label: 'b' as i32,
            });
        atn.set_rule_to_start_state(vec![1, 6]);
        atn.set_rule_to_stop_state(vec![5, 7]);
        atn.set_rule_to_token_type(vec![1, 2]);
        atn.add_mode_start_state(0);
        atn.add_decision_state(0);
        atn.set_lexer_actions(vec![LexerAction::Custom {
            rule_index: 0,
            action_index: 0,
        }]);
        atn
    }

    fn lifecycle_rewind_atn() -> LexerAtn {
        let mut atn = LexerAtn::new(2);
        for (state_number, kind, rule_index) in [
            (0, AtnStateKind::TokenStart, None),
            (1, AtnStateKind::RuleStart, Some(0)),
            (2, AtnStateKind::Basic, Some(0)),
            (3, AtnStateKind::RuleStop, Some(0)),
            (4, AtnStateKind::RuleStart, Some(1)),
            (5, AtnStateKind::RuleStop, Some(1)),
        ] {
            let mut state = LexerAtnState::new(state_number, kind);
            if let Some(rule_index) = rule_index {
                state = state.with_rule_index(rule_index);
            }
            atn.add_state(state);
        }
        atn.state_mut(0)
            .expect("token start")
            .add_transition(LexerTransition::Epsilon { target: 1 });
        atn.state_mut(0)
            .expect("token start")
            .add_transition(LexerTransition::Epsilon { target: 4 });
        atn.state_mut(1)
            .expect("long rule start")
            .add_transition(LexerTransition::Atom {
                target: 2,
                label: 'a' as i32,
            });
        atn.state_mut(2)
            .expect("long rule body")
            .add_transition(LexerTransition::Atom {
                target: 3,
                label: 'b' as i32,
            });
        atn.state_mut(4)
            .expect("suffix rule start")
            .add_transition(LexerTransition::Atom {
                target: 5,
                label: 'b' as i32,
            });
        atn.set_rule_to_start_state(vec![1, 4]);
        atn.set_rule_to_stop_state(vec![3, 5]);
        atn.set_rule_to_token_type(vec![1, 2]);
        atn.add_mode_start_state(0);
        atn.add_decision_state(0);
        atn
    }

    fn custom_control_action_atn(control_action: LexerAction) -> LexerAtn {
        let mut atn = LexerAtn::new(8);
        for (state_number, kind, rule_index) in [
            (0, AtnStateKind::TokenStart, None),
            (1, AtnStateKind::RuleStart, Some(0)),
            (2, AtnStateKind::Basic, Some(0)),
            (3, AtnStateKind::Basic, Some(0)),
            (4, AtnStateKind::RuleStop, Some(0)),
            (5, AtnStateKind::RuleStart, Some(1)),
            (6, AtnStateKind::RuleStop, Some(1)),
        ] {
            let mut state = LexerAtnState::new(state_number, kind);
            if let Some(rule_index) = rule_index {
                state = state.with_rule_index(rule_index);
            }
            atn.add_state(state);
        }
        atn.state_mut(0)
            .expect("token start")
            .add_transition(LexerTransition::Epsilon { target: 1 });
        atn.state_mut(0)
            .expect("token start")
            .add_transition(LexerTransition::Epsilon { target: 5 });
        atn.state_mut(1)
            .expect("first rule start")
            .add_transition(LexerTransition::Atom {
                target: 2,
                label: 'a' as i32,
            });
        atn.state_mut(2)
            .expect("custom action")
            .add_transition(LexerTransition::Action {
                target: 3,
                rule_index: 0,
                action_index: Some(0),
                context_dependent: false,
            });
        atn.state_mut(3)
            .expect("control action")
            .add_transition(LexerTransition::Action {
                target: 4,
                rule_index: 0,
                action_index: Some(1),
                context_dependent: false,
            });
        atn.state_mut(5)
            .expect("suffix rule start")
            .add_transition(LexerTransition::Atom {
                target: 6,
                label: 'b' as i32,
            });
        atn.set_rule_to_start_state(vec![1, 5]);
        atn.set_rule_to_stop_state(vec![4, 6]);
        atn.set_rule_to_token_type(vec![1, 2]);
        atn.add_mode_start_state(0);
        atn.add_decision_state(0);
        atn.set_lexer_actions(vec![
            LexerAction::Custom {
                rule_index: 0,
                action_index: 0,
            },
            control_action,
        ]);
        atn
    }

    fn lex_one<I: CharStream>(lexer: &mut BaseLexer<I>, atn: &LexerAtn) -> TokenSnapshot {
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let id = next_token(lexer, &mut sink, atn).expect("test token should fit");
        let token = sink.view(id).expect("emitted token should exist");
        TokenSnapshot {
            token_type: token.token_type(),
            start: token.start(),
            stop: token.stop(),
            text: token.text().to_owned(),
        }
    }

    #[derive(Debug, Default)]
    struct FunctionTokenHooks {
        accepted: Vec<(i32, i32, String)>,
        emitted: Vec<(i32, i32, String)>,
    }

    impl SemanticHooks for FunctionTokenHooks {
        fn lexer_action<I>(
            &mut self,
            ctx: &mut LexerSemCtx<'_, I>,
            _action: LexerCustomAction,
        ) -> bool
        where
            I: CharStream,
        {
            assert_eq!(ctx.text_so_far(), "count");
            while matches!(ctx.la(1), value if value == ' ' as i32 || value == '\t' as i32) {
                assert!(ctx.consume());
                assert!(ctx.set_channel(HIDDEN_CHANNEL));
            }
            assert_eq!(ctx.la(1), '(' as i32);
            assert!(ctx.set_type(7));
            true
        }

        fn lexer_after_accept<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: CharStream,
        {
            self.accepted
                .push((ctx.token_type(), ctx.channel(), ctx.token_text()));
        }

        fn lexer_token_emitted(&mut self, token: TokenView<'_>) {
            self.emitted
                .push((token.token_type(), token.channel(), token.text().to_owned()));
        }
    }

    #[test]
    fn lexer_action_can_override_pending_type_and_channel() {
        let atn = trailing_action_atn(
            &['c', 'o', 'u', 'n', 't'],
            1,
            vec![LexerAction::Custom {
                rule_index: 0,
                action_index: 0,
            }],
        );
        let mut lexer = BaseLexer::new(InputStream::new("count \t("), recognizer_data());
        let mut hooks = FunctionTokenHooks::default();
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);

        let id = next_token_with_semantic_hooks(&mut lexer, &mut sink, &atn, &mut hooks)
            .expect("dynamic token should fit");
        let token = sink.view(id).expect("dynamic token should exist");
        assert_eq!(token.token_type(), 7);
        assert_eq!(token.channel(), HIDDEN_CHANNEL);
        assert_eq!(token.text(), "count \t");
        assert_eq!(lexer.la(1), '(' as i32);
        assert_eq!(hooks.accepted, [(7, HIDDEN_CHANNEL, "count \t".to_owned())]);
        assert_eq!(hooks.emitted, [(7, HIDDEN_CHANNEL, "count \t".to_owned())]);
    }

    #[derive(Debug, Default)]
    struct DotSplitHooks {
        before_pending_counts: Vec<usize>,
        emitted_types: Vec<i32>,
    }

    impl SemanticHooks for DotSplitHooks {
        fn lexer_before_token<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: CharStream,
        {
            self.before_pending_counts.push(ctx.pending_token_count());
        }

        fn lexer_action<I>(
            &mut self,
            ctx: &mut LexerSemCtx<'_, I>,
            _action: LexerCustomAction,
        ) -> bool
        where
            I: CharStream,
        {
            assert_eq!(ctx.text_so_far(), ".β");
            let dot = ctx.token_start();
            assert!(ctx.enqueue_token(1, dot));
            assert!(ctx.set_token_start(dot + 1));
            true
        }

        fn lexer_token_emitted(&mut self, token: TokenView<'_>) {
            self.emitted_types.push(token.token_type());
        }
    }

    #[test]
    fn lexer_action_can_queue_prefix_before_automatic_token() {
        let atn = trailing_action_atn(
            &['.', 'β'],
            3,
            vec![
                LexerAction::Custom {
                    rule_index: 0,
                    action_index: 0,
                },
                LexerAction::Type(2),
            ],
        );
        let dfa = CompiledLexerDfa::compile(&atn);
        let mut lexer = BaseLexer::new(InputStream::new(".β"), recognizer_data());
        let mut hooks = DotSplitHooks::default();
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);

        let dot =
            next_token_compiled_with_semantic_hooks(&mut lexer, &mut sink, &atn, &dfa, &mut hooks)
                .expect("dot token should fit");
        assert_eq!(sink.token_count(), 1);
        let identifier =
            next_token_compiled_with_semantic_hooks(&mut lexer, &mut sink, &atn, &dfa, &mut hooks)
                .expect("identifier token should fit");
        assert_eq!(sink.token_count(), 2);
        let eof =
            next_token_compiled_with_semantic_hooks(&mut lexer, &mut sink, &atn, &dfa, &mut hooks)
                .expect("EOF token should fit");
        assert_eq!(sink.token_count(), 3);

        let dot = sink.view(dot).expect("dot token should exist");
        assert_eq!(dot.token_type(), 1);
        assert_eq!(dot.channel(), DEFAULT_CHANNEL);
        assert_eq!(dot.text(), ".");
        assert_eq!(dot.start(), 0);
        assert_eq!(dot.stop(), 0);
        assert_eq!(dot.byte_span(), 0..1);
        assert_eq!((dot.line(), dot.column()), (1, 0));

        let identifier = sink
            .view(identifier)
            .expect("identifier token should exist");
        assert_eq!(identifier.token_type(), 2);
        assert_eq!(identifier.text(), "β");
        assert_eq!(identifier.start(), 1);
        assert_eq!(identifier.stop(), 1);
        assert_eq!(identifier.byte_span(), 1..3);
        assert_eq!((identifier.line(), identifier.column()), (1, 1));

        assert_eq!(
            sink.view(eof).expect("EOF token should exist").token_type(),
            TOKEN_EOF
        );
        assert_eq!(hooks.before_pending_counts, [0, 1, 0]);
        assert_eq!(hooks.emitted_types, [1, 2, TOKEN_EOF]);
    }

    #[derive(Debug, Default)]
    struct RewindAtEofHooks {
        action_count: usize,
    }

    impl SemanticHooks for RewindAtEofHooks {
        fn lexer_action<I>(
            &mut self,
            ctx: &mut LexerSemCtx<'_, I>,
            _action: LexerCustomAction,
        ) -> bool
        where
            I: CharStream,
        {
            assert_eq!(ctx.la(1), EOF);
            let suffix_start = ctx.token_start() + 1;
            assert!(ctx.reset_accept_position(suffix_start));
            self.action_count += 1;
            true
        }
    }

    #[test]
    fn lexer_action_rewind_from_eof_preserves_suffix() {
        let atn = eof_rewind_action_atn();
        let dfa = CompiledLexerDfa::compile(&atn);
        let mut lexer = BaseLexer::new(InputStream::new("ab"), recognizer_data());
        let mut hooks = RewindAtEofHooks::default();
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);

        let prefix =
            next_token_compiled_with_semantic_hooks(&mut lexer, &mut sink, &atn, &dfa, &mut hooks)
                .expect("prefix token should fit");
        assert!(!lexer.hit_eof());
        let suffix =
            next_token_compiled_with_semantic_hooks(&mut lexer, &mut sink, &atn, &dfa, &mut hooks)
                .expect("suffix token should fit");
        let eof =
            next_token_compiled_with_semantic_hooks(&mut lexer, &mut sink, &atn, &dfa, &mut hooks)
                .expect("EOF token should fit");

        let prefix = sink.view(prefix).expect("prefix token should exist");
        assert_eq!((prefix.token_type(), prefix.text()), (1, "a"));
        let suffix = sink.view(suffix).expect("suffix token should exist");
        assert_eq!((suffix.token_type(), suffix.text()), (2, "b"));
        assert_eq!(
            sink.view(eof).expect("EOF token should exist").token_type(),
            TOKEN_EOF
        );
        assert_eq!(hooks.action_count, 1);
    }

    #[derive(Debug, Default)]
    struct LifecycleRecordingHooks {
        events: Vec<String>,
    }

    impl SemanticHooks for LifecycleRecordingHooks {
        fn lexer_before_token<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: CharStream,
        {
            self.events.push(format!(
                "before:{}:{}",
                ctx.input_position(),
                ctx.pending_token_count()
            ));
        }

        fn lexer_after_accept<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: CharStream,
        {
            self.events.push(format!(
                "accept:{}:{}",
                ctx.token_type(),
                ctx.accepted_text().expect("accepted callback has text")
            ));
            if ctx.token_type() == 1 {
                ctx.reset_accept_position(ctx.token_start() + 1);
            }
        }

        fn lexer_token_emitted(&mut self, token: TokenView<'_>) {
            self.events
                .push(format!("emit:{}:{}", token.token_type(), token.text()));
        }
    }

    fn lifecycle_tokens(compiled: bool) -> (Vec<(i32, String)>, Vec<String>) {
        let atn = lifecycle_rewind_atn();
        let dfa = CompiledLexerDfa::compile(&atn);
        let mut lexer = BaseLexer::new(InputStream::new("ab"), recognizer_data());
        let mut hooks = LifecycleRecordingHooks::default();
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let mut ids = Vec::new();
        for _ in 0..3 {
            let id = if compiled {
                next_token_compiled_with_semantic_hooks(
                    &mut lexer, &mut sink, &atn, &dfa, &mut hooks,
                )
            } else {
                next_token_with_semantic_hooks(&mut lexer, &mut sink, &atn, &mut hooks)
            }
            .expect("lifecycle token should fit");
            ids.push(id);
        }
        let tokens = ids
            .into_iter()
            .map(|id| {
                let token = sink.view(id).expect("lifecycle token should exist");
                (token.token_type(), token.text().to_owned())
            })
            .collect();
        (tokens, hooks.events)
    }

    #[test]
    fn lifecycle_post_accept_runs_without_actions_and_matches_compiled_order() {
        let interpreted = lifecycle_tokens(false);
        let compiled = lifecycle_tokens(true);

        assert_eq!(interpreted, compiled);
        assert_eq!(
            interpreted.0,
            [
                (1, "a".to_owned()),
                (2, "b".to_owned()),
                (TOKEN_EOF, "<EOF>".to_owned()),
            ]
        );
        assert_eq!(
            interpreted.1,
            [
                "before:0:0",
                "accept:1:ab",
                "emit:1:a",
                "before:1:0",
                "accept:2:b",
                "emit:2:b",
                "before:2:0",
                "emit:-1:<EOF>",
            ]
        );
    }

    #[derive(Debug, Default)]
    struct MoreAfterAcceptHooks {
        events: Vec<String>,
    }

    impl SemanticHooks for MoreAfterAcceptHooks {
        fn lexer_before_token<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: CharStream,
        {
            self.events.push(format!("before:{}", ctx.input_position()));
        }

        fn lexer_after_accept<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: CharStream,
        {
            self.events.push(format!(
                "accept:{}:{}",
                ctx.token_type(),
                ctx.accepted_text().expect("accepted callback has text")
            ));
            if ctx.token_type() == 1 {
                ctx.more();
            }
        }

        fn lexer_token_emitted(&mut self, token: TokenView<'_>) {
            self.events
                .push(format!("emit:{}:{}", token.token_type(), token.text()));
        }
    }

    fn lifecycle_more_token(compiled: bool) -> ((i32, String), Vec<String>) {
        let atn = lifecycle_rewind_atn();
        let dfa = CompiledLexerDfa::compile(&atn);
        let mut lexer = BaseLexer::new(InputStream::new("abb"), recognizer_data());
        let mut hooks = MoreAfterAcceptHooks::default();
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let id = if compiled {
            next_token_compiled_with_semantic_hooks(&mut lexer, &mut sink, &atn, &dfa, &mut hooks)
        } else {
            next_token_with_semantic_hooks(&mut lexer, &mut sink, &atn, &mut hooks)
        }
        .expect("combined lifecycle token should fit");
        let token = sink
            .view(id)
            .expect("combined lifecycle token should exist");
        ((token.token_type(), token.text().to_owned()), hooks.events)
    }

    #[test]
    fn lifecycle_post_accept_more_composes_with_the_next_match() {
        let interpreted = lifecycle_more_token(false);
        let compiled = lifecycle_more_token(true);

        assert_eq!(interpreted, compiled);
        assert_eq!(interpreted.0, (2, "abb".to_owned()));
        assert_eq!(
            interpreted.1,
            [
                "before:0",
                "accept:1:ab",
                "before:2",
                "accept:2:abb",
                "emit:2:abb",
            ]
        );
    }

    #[derive(Debug, Default)]
    struct QueueDuringMoreHooks {
        before: Vec<(usize, usize)>,
    }

    type QueuedMoreToken = (i32, usize, usize, String);
    type BeforeTokenState = (usize, usize);

    impl SemanticHooks for QueueDuringMoreHooks {
        fn lexer_before_token<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: CharStream,
        {
            self.before
                .push((ctx.input_position(), ctx.pending_token_count()));
        }

        fn lexer_action<I>(
            &mut self,
            ctx: &mut LexerSemCtx<'_, I>,
            _action: LexerCustomAction,
        ) -> bool
        where
            I: CharStream,
        {
            assert_eq!(ctx.text_so_far(), "a");
            assert!(ctx.enqueue_token(7, ctx.token_start()));
            true
        }
    }

    fn queued_more_tokens(compiled: bool) -> (Vec<QueuedMoreToken>, Vec<BeforeTokenState>) {
        let atn = custom_control_action_atn(LexerAction::More);
        let dfa = CompiledLexerDfa::compile(&atn);
        let mut lexer = BaseLexer::new(InputStream::new("ab"), recognizer_data());
        let mut hooks = QueueDuringMoreHooks::default();
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let mut ids = Vec::new();
        for _ in 0..2 {
            let id = if compiled {
                next_token_compiled_with_semantic_hooks(
                    &mut lexer, &mut sink, &atn, &dfa, &mut hooks,
                )
            } else {
                next_token_with_semantic_hooks(&mut lexer, &mut sink, &atn, &mut hooks)
            }
            .expect("queued MORE token should fit");
            ids.push(id);
        }
        let tokens = ids
            .into_iter()
            .map(|id| {
                let token = sink.view(id).expect("queued MORE token should exist");
                (
                    token.token_type(),
                    token.start(),
                    token.stop(),
                    token.text().to_owned(),
                )
            })
            .collect();
        (tokens, hooks.before)
    }

    #[test]
    fn queued_token_waits_for_more_chain_to_finish() {
        let interpreted = queued_more_tokens(false);
        let compiled = queued_more_tokens(true);

        assert_eq!(interpreted, compiled);
        assert_eq!(
            interpreted.0,
            [(7, 0, 0, "a".to_owned()), (2, 0, 1, "ab".to_owned()),]
        );
        assert_eq!(interpreted.1, [(0, 0), (1, 1), (2, 1)]);
    }

    #[derive(Debug, Default)]
    struct OverrideControlActionHooks {
        events: Vec<String>,
    }

    impl SemanticHooks for OverrideControlActionHooks {
        fn lexer_action<I>(
            &mut self,
            ctx: &mut LexerSemCtx<'_, I>,
            _action: LexerCustomAction,
        ) -> bool
        where
            I: CharStream,
        {
            self.events.push(format!("action:{}", ctx.text_so_far()));
            true
        }

        fn lexer_after_accept<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: CharStream,
        {
            self.events.push(format!(
                "accept:{}:{}",
                ctx.token_type(),
                ctx.accepted_text().expect("accepted callback has text")
            ));
            if ctx.token_type() == crate::lexer::SKIP || ctx.token_type() == crate::lexer::MORE {
                ctx.set_type(8);
            }
        }

        fn lexer_token_emitted(&mut self, token: TokenView<'_>) {
            self.events
                .push(format!("emit:{}:{}", token.token_type(), token.text()));
        }
    }

    fn overridden_control_action_tokens(
        control_action: LexerAction,
        compiled: bool,
    ) -> (Vec<(i32, String)>, Vec<String>) {
        let atn = custom_control_action_atn(control_action);
        let dfa = CompiledLexerDfa::compile(&atn);
        let mut lexer = BaseLexer::new(InputStream::new("ab"), recognizer_data());
        let mut hooks = OverrideControlActionHooks::default();
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let mut ids = Vec::new();
        for _ in 0..3 {
            let id = if compiled {
                next_token_compiled_with_semantic_hooks(
                    &mut lexer, &mut sink, &atn, &dfa, &mut hooks,
                )
            } else {
                next_token_with_semantic_hooks(&mut lexer, &mut sink, &atn, &mut hooks)
            }
            .expect("overridden control-action token should fit");
            ids.push(id);
        }
        let tokens = ids
            .into_iter()
            .map(|id| {
                let token = sink
                    .view(id)
                    .expect("overridden control-action token should exist");
                (token.token_type(), token.text().to_owned())
            })
            .collect();
        (tokens, hooks.events)
    }

    #[test]
    fn lifecycle_post_accept_can_override_skip_and_more_actions() {
        for (control_action, control_type) in [
            (LexerAction::Skip, crate::lexer::SKIP),
            (LexerAction::More, crate::lexer::MORE),
        ] {
            let interpreted = overridden_control_action_tokens(control_action.clone(), false);
            let compiled = overridden_control_action_tokens(control_action, true);

            assert_eq!(interpreted, compiled);
            assert_eq!(
                interpreted.0,
                [
                    (8, "a".to_owned()),
                    (2, "b".to_owned()),
                    (TOKEN_EOF, "<EOF>".to_owned()),
                ]
            );
            assert_eq!(
                interpreted.1,
                [
                    "action:a".to_owned(),
                    format!("accept:{control_type}:a"),
                    "emit:8:a".to_owned(),
                    "accept:2:b".to_owned(),
                    "emit:2:b".to_owned(),
                    "emit:-1:<EOF>".to_owned(),
                ]
            );
        }
    }

    #[derive(Debug, Default)]
    struct ResettableSplitHooks {
        transient: bool,
        reset_count: usize,
    }

    impl SemanticHooks for ResettableSplitHooks {
        fn lexer_action<I>(
            &mut self,
            ctx: &mut LexerSemCtx<'_, I>,
            _action: LexerCustomAction,
        ) -> bool
        where
            I: CharStream,
        {
            let start = ctx.token_start();
            assert!(ctx.enqueue_token(1, start));
            assert!(ctx.set_token_start(start + 1));
            self.transient = true;
            true
        }

        fn lexer_reset<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: CharStream,
        {
            assert_eq!(ctx.input_position(), 0);
            assert_eq!(ctx.pending_token_count(), 0);
            assert_eq!(ctx.mode(), crate::lexer::DEFAULT_MODE);
            assert_eq!(ctx.token_type(), INVALID_TOKEN_TYPE);
            assert_eq!(ctx.channel(), DEFAULT_CHANNEL);
            assert_eq!((ctx.line(), ctx.column()), (1, 0));
            assert_eq!(ctx.token_start(), 0);
            self.transient = false;
            self.reset_count += 1;
        }
    }

    #[test]
    fn lifecycle_reset_clears_pending_tokens_and_extension_state() {
        let atn = trailing_action_atn(
            &['.', 'x'],
            2,
            vec![LexerAction::Custom {
                rule_index: 0,
                action_index: 0,
            }],
        );
        let dfa = CompiledLexerDfa::compile(&atn);
        let mut lexer = BaseLexer::new(InputStream::new(".x"), recognizer_data());
        let mut hooks = ResettableSplitHooks::default();
        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);

        let first =
            next_token_compiled_with_semantic_hooks(&mut lexer, &mut sink, &atn, &dfa, &mut hooks)
                .expect("queued prefix should fit");
        assert_eq!(sink.view(first).expect("prefix exists").text(), ".");
        assert!(hooks.transient);

        lexer.push_mode(7);
        lexer.set_channel(HIDDEN_CHANNEL);
        lexer.set_hit_eof(true);
        reset_with_semantic_hooks(&mut lexer, &mut hooks);
        assert!(!hooks.transient);
        assert_eq!(hooks.reset_count, 1);

        set_input_stream_with_semantic_hooks(
            &mut lexer,
            &mut hooks,
            InputStream::with_source_name(".x", "replacement"),
        );
        assert_eq!(hooks.reset_count, 2);
        assert_eq!(lexer.source_name(), "replacement");

        let after_reset =
            next_token_compiled_with_semantic_hooks(&mut lexer, &mut sink, &atn, &dfa, &mut hooks)
                .expect("prefix after reset should fit");
        assert_eq!(
            sink.view(after_reset)
                .expect("post-reset prefix exists")
                .text(),
            ".",
            "the stale queued suffix must not survive reset"
        );
    }

    #[test]
    fn predicate_sensitive_lexer_state_is_not_replay_cached() {
        let atn = LexerAtn::new(1);
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
    fn lexer_action_hook_context_can_change_mode() {
        // A custom-action hook receives a mutable lexer borrow, so it can push /
        // pop / set the lexer mode (matching the closure `custom_action` API). A
        // speculative predicate context receives a shared borrow, so the same
        // mutators are inert there.
        let data = RecognizerData::new(
            "T",
            Vocabulary::new([None, Some("T")], [None, Some("T")], [None::<&str>, None]),
        );
        let mut lexer = BaseLexer::new(InputStream::new(""), data);

        // Action (mutable) context: mode mutations apply.
        {
            let mut ctx = LexerSemCtx::new_mut(&mut lexer, 0, 0, 0);
            assert_eq!(ctx.mode(), 0);
            assert!(ctx.push_mode(2), "action context applies push_mode");
            assert_eq!(ctx.mode(), 2);
            assert!(ctx.set_mode(3), "action context applies set_mode");
            assert_eq!(ctx.mode(), 3);
            assert_eq!(ctx.pop_mode(), Some(0), "pop restores the pushed-from mode");
            assert_eq!(ctx.mode(), 0);
        }
        assert_eq!(lexer.mode(), 0, "mutations went through to the lexer");

        // Predicate (shared) context: mutators are inert and report not-applied.
        {
            let mut ctx = LexerSemCtx::new(&lexer, 0, 0, 0);
            assert!(!ctx.push_mode(2), "predicate context does not mutate");
            assert!(!ctx.set_mode(3), "predicate context does not mutate");
            assert_eq!(ctx.pop_mode(), None, "predicate context does not mutate");
        }
        assert_eq!(
            lexer.mode(),
            0,
            "shared-context calls left the lexer unchanged"
        );
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

        let token = lex_one(&mut lexer, &atn);
        assert_eq!(token.token_type, 1);
        assert_eq!(token.text, "ab");
        assert_eq!(lex_one(&mut lexer, &atn).token_type, TOKEN_EOF);
    }

    #[test]
    fn fixed_literal_accept_stops_later_non_greedy_same_rule_paths() {
        for strategy in [
            TestMatchStrategy::Interpreted,
            TestMatchStrategy::Cached,
            TestMatchStrategy::Compiled,
        ] {
            let token = lex_issue_106_comment(strategy);
            assert_eq!(token.token_type, 1, "{strategy:?}");
            assert_eq!((token.start, token.stop), (0, 3), "{strategy:?}");
            assert_eq!(token.text, "/**/", "{strategy:?}");
        }
    }

    #[test]
    fn recursive_comment_contexts_and_actions_stay_bounded_on_all_paths() {
        let atn = recursive_comment_channel_atn();
        let compiled = CompiledLexerDfa::compile(&atn);
        let serialized = compiled.serialize();
        let compiled =
            CompiledLexerDfa::from_serialized(&serialized).expect("compiled DFA round trip");
        let first = "/* first /* nested */ tail */";
        let mut source = format!("{first}x");
        for _ in 0..40 {
            source.push_str("/* comment */x");
        }

        for strategy in [
            TestMatchStrategy::Interpreted,
            TestMatchStrategy::Cached,
            TestMatchStrategy::Compiled,
        ] {
            let data = RecognizerData::new(
                "RecursiveComment",
                Vocabulary::new(
                    [None::<&str>, None, None, None],
                    [None, Some("COMMENT"), Some("AT_PRE"), Some("OTHER")],
                    [None::<&str>, None, None, None],
                ),
            );
            let mut lexer = BaseLexer::new(InputStream::new(source.clone()), data);
            let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
            let mut sink = TokenSink::new(&mut store);
            let mut token_count = 0;
            let mut contexts_after_warmup = 0;
            loop {
                let token = match strategy {
                    TestMatchStrategy::Interpreted => next_token_with_hooks(
                        &mut lexer,
                        &mut sink,
                        &atn,
                        |_, _| {},
                        |_, _| true,
                        |_, _, _| {},
                    ),
                    TestMatchStrategy::Cached => next_token(&mut lexer, &mut sink, &atn),
                    TestMatchStrategy::Compiled => {
                        next_token_compiled(&mut lexer, &mut sink, &atn, &compiled)
                    }
                }
                .expect("recursive comment source should lex");
                let token = sink.view(token).expect("token should exist");
                if token_count == 0 {
                    assert_eq!(token.token_type(), 1, "{strategy:?}");
                    assert_eq!(token.text(), first, "{strategy:?}");
                    assert_eq!(token.channel(), HIDDEN_CHANNEL, "{strategy:?}");
                } else if token_count == 3 {
                    contexts_after_warmup = lexer.lexer_dfa_cache_shape().3;
                }
                token_count += 1;
                if token.token_type() == TOKEN_EOF {
                    break;
                }
            }
            assert_eq!(token_count, 83, "{strategy:?}");

            let (cached_states, cached_transitions, max_configs, contexts) =
                lexer.lexer_dfa_cache_shape();
            assert!(max_configs <= 16, "{strategy:?}: {max_configs} configs");
            assert!(contexts <= 1024, "{strategy:?}: {contexts} contexts");
            assert!(
                contexts <= contexts_after_warmup + 16,
                "{strategy:?}: contexts grew from {contexts_after_warmup} to {contexts}"
            );
            if !matches!(strategy, TestMatchStrategy::Interpreted) {
                assert!(cached_states > 0, "{strategy:?}");
                assert!(cached_transitions > 0, "{strategy:?}");
            }
        }
    }

    #[test]
    fn compiled_resume_rejects_untrusted_continuation_payloads() {
        let atn = trailing_action_atn(&['a'], 1, vec![LexerAction::Skip]);
        let valid_config = CompiledLexerConfig {
            state: 2,
            consumed_eof: false,
            alt_rule_index: Some(0),
            passed_non_greedy: false,
            context: 0,
            actions: vec![CompiledLexerActionTrace {
                action_index: 0,
                rule_index: 0,
                behind: 0,
            }],
        };
        let valid = |config| CompiledLexerContinuation {
            contexts: Vec::new(),
            configs: vec![config],
        };
        let matches = |continuation: &CompiledLexerContinuation| {
            compiled_resume_matches_atn(
                &atn,
                4,
                &CompiledResume {
                    continuation,
                    position: 5,
                    best: None,
                    error_stop: 5,
                },
            )
        };

        assert!(matches(&valid(valid_config.clone())));
        assert!(!matches(&CompiledLexerContinuation {
            contexts: Vec::new(),
            configs: Vec::new(),
        }));

        let mut invalid = valid_config.clone();
        invalid.state = usize::MAX;
        assert!(!matches(&valid(invalid)));

        let mut invalid = valid_config.clone();
        invalid.alt_rule_index = None;
        assert!(!matches(&valid(invalid)));

        let mut invalid = valid_config.clone();
        invalid.alt_rule_index = Some(1);
        assert!(!matches(&valid(invalid)));

        let mut invalid = valid_config.clone();
        invalid.context = u32::MAX;
        assert!(!matches(&valid(invalid)));

        assert!(!matches(&CompiledLexerContinuation {
            contexts: vec![CompiledLexerContext::Singleton {
                parent: 1,
                return_state: 2,
            }],
            configs: vec![valid_config.clone()],
        }));

        let mut invalid = valid_config.clone();
        invalid.actions[0].action_index = 1;
        assert!(!matches(&valid(invalid)));

        let mut invalid = valid_config.clone();
        invalid.actions[0].rule_index = 1;
        assert!(!matches(&valid(invalid)));

        let mut invalid = valid_config;
        invalid.actions[0].behind = 2;
        assert!(!matches(&valid(invalid)));
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

        let token = lex_one(&mut lexer, &atn);
        assert_eq!(token.token_type, 1);
        assert_eq!(token.start, 0);
        assert_eq!(token.stop, 1);
        assert_eq!(token.text, "ab");
    }
}
