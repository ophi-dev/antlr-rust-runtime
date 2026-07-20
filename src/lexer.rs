use std::cell::{RefCell, RefMut};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::hash::BuildHasherDefault;
use std::rc::Rc;

use crate::atn::LexerAtn;
use crate::char_stream::{CharStream, TextInterval};
use crate::int_stream::EOF;
use crate::prediction::{
    ContextArena, ContextId, EMPTY_CONTEXT, PredictionFxHasher, PredictionWorkspace,
};
use crate::recognizer::{Recognizer, RecognizerData};
use crate::token::{
    DEFAULT_CHANNEL, INVALID_TOKEN_TYPE, TokenId, TokenSink, TokenSourceError, TokenSpec,
    TokenStoreError,
};

#[allow(clippy::disallowed_types)]
type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<PredictionFxHasher>>;

pub const SKIP: i32 = -3;
pub const MORE: i32 = -2;
pub const DEFAULT_MODE: i32 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LexerMode(pub i32);

/// Grammar-specific lexer action reached on the accepted ATN path.
///
/// ANTLR serializes embedded lexer actions as `(rule_index, action_index)`
/// pairs. The runtime also records the input position where the action was
/// reached so generated code can evaluate templates such as `Text()` at the
/// same point as a generated ANTLR lexer, not only at the token end.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LexerCustomAction {
    rule_index: i32,
    action_index: i32,
    position: usize,
}

impl LexerCustomAction {
    /// Creates a custom lexer action event from serialized ATN metadata.
    pub const fn new(rule_index: i32, action_index: i32, position: usize) -> Self {
        Self {
            rule_index,
            action_index,
            position,
        }
    }

    /// Lexer rule index that owns the embedded action.
    pub const fn rule_index(self) -> i32 {
        self.rule_index
    }

    /// Per-rule action index assigned by ANTLR serialization.
    pub const fn action_index(self) -> i32 {
        self.action_index
    }

    /// Character-stream position at which the action transition was reached.
    pub const fn position(self) -> usize {
        self.position
    }
}

/// Grammar-specific lexer predicate reached while exploring an ATN path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LexerPredicate {
    rule_index: usize,
    pred_index: usize,
    position: usize,
}

impl LexerPredicate {
    /// Creates a lexer predicate event from serialized ATN metadata.
    pub const fn new(rule_index: usize, pred_index: usize, position: usize) -> Self {
        Self {
            rule_index,
            pred_index,
            position,
        }
    }

    /// Lexer rule index that owns the predicate transition.
    pub const fn rule_index(self) -> usize {
        self.rule_index
    }

    /// Per-rule predicate index assigned by ANTLR serialization.
    pub const fn pred_index(self) -> usize {
        self.pred_index
    }

    /// Character-stream position at which the predicate is evaluated.
    pub const fn position(self) -> usize {
        self.position
    }
}

/// Lexer reference held by [`LexerSemCtx`]. A semantic *predicate* is evaluated
/// speculatively and gets a shared borrow; a *custom action* runs on the
/// committed path and gets a mutable borrow so a hook can change lexer state
/// and pending token emission, matching the closure-based `custom_action` API.
#[derive(Debug)]
enum LexerRef<'a, I>
where
    I: CharStream,
{
    Shared(&'a BaseLexer<I>),
    Mut(&'a mut BaseLexer<I>),
}

impl<I> LexerRef<'_, I>
where
    I: CharStream,
{
    const fn get(&self) -> &BaseLexer<I> {
        match self {
            LexerRef::Shared(lexer) => lexer,
            LexerRef::Mut(lexer) => lexer,
        }
    }
}

/// Runtime view passed to lexer semantic hooks.
#[derive(Debug)]
pub struct LexerSemCtx<'a, I>
where
    I: CharStream,
{
    lexer: LexerRef<'a, I>,
    rule_index: usize,
    coordinate_index: usize,
    position: usize,
}

impl<'a, I> LexerSemCtx<'a, I>
where
    I: CharStream,
{
    pub(crate) const fn new(
        lexer: &'a BaseLexer<I>,
        rule_index: usize,
        coordinate_index: usize,
        position: usize,
    ) -> Self {
        Self {
            lexer: LexerRef::Shared(lexer),
            rule_index,
            coordinate_index,
            position,
        }
    }

    /// Builds a context with a mutable lexer borrow, for a custom-action hook
    /// that may change lexer and pending-token state.
    pub(crate) const fn new_mut(
        lexer: &'a mut BaseLexer<I>,
        rule_index: usize,
        coordinate_index: usize,
        position: usize,
    ) -> Self {
        Self {
            lexer: LexerRef::Mut(lexer),
            rule_index,
            coordinate_index,
            position,
        }
    }

    /// Lexer rule index that owns the predicate/action coordinate.
    #[must_use]
    pub const fn rule_index(&self) -> usize {
        self.rule_index
    }

    /// Predicate/action index inside the owning lexer rule.
    #[must_use]
    pub const fn coordinate_index(&self) -> usize {
        self.coordinate_index
    }

    /// Absolute input position where the predicate/action transition fired.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// Lexer mode at this coordinate.
    #[must_use]
    pub fn mode(&self) -> i32 {
        self.lexer.get().mode()
    }

    /// Current source column.
    #[must_use]
    pub const fn column(&self) -> usize {
        self.lexer.get().column()
    }

    /// Source column at [`Self::position`].
    #[must_use]
    pub fn position_column(&self) -> usize {
        self.lexer.get().column_at(self.position)
    }

    /// Column captured at the current token start.
    #[must_use]
    pub const fn token_start_column(&self) -> usize {
        self.lexer.get().token_start_column()
    }

    /// Text matched from token start to this coordinate.
    #[must_use]
    pub fn text_so_far(&self) -> String {
        self.lexer.get().token_text_until(self.position)
    }

    /// Character at a one-based lookahead/lookbehind offset.
    ///
    /// Predicates read relative to their speculative ATN coordinate. Actions
    /// read relative to the committed input cursor, including characters
    /// consumed by an earlier action.
    pub fn la(&mut self, offset: isize) -> i32 {
        match &mut self.lexer {
            LexerRef::Shared(lexer) => lexer.lookahead_at(self.position, offset),
            LexerRef::Mut(lexer) => lexer.input_mut().la(offset),
        }
    }

    /// Absolute source index where the current token begins.
    #[must_use]
    pub const fn token_start(&self) -> usize {
        self.lexer.get().token_start()
    }

    /// Pending type of the token being matched.
    #[must_use]
    pub const fn token_type(&self) -> i32 {
        self.lexer.get().token_type()
    }

    /// Pending channel of the token being matched.
    #[must_use]
    pub const fn channel(&self) -> i32 {
        self.lexer.get().channel()
    }

    /// Sets the pending emitted token type. Action context only; see
    /// [`Self::set_mode`] for the return value.
    pub const fn set_type(&mut self, token_type: i32) -> bool {
        match &mut self.lexer {
            LexerRef::Mut(lexer) => {
                lexer.set_type(token_type);
                true
            }
            LexerRef::Shared(_) => false,
        }
    }

    /// Sets the pending emitted token channel. Action context only; see
    /// [`Self::set_mode`] for the return value.
    pub const fn set_channel(&mut self, channel: i32) -> bool {
        match &mut self.lexer {
            LexerRef::Mut(lexer) => {
                lexer.set_channel(channel);
                true
            }
            LexerRef::Shared(_) => false,
        }
    }

    /// Consumes one input character and updates source position tracking.
    /// Action context only; returns whether the operation was available.
    pub fn consume(&mut self) -> bool {
        match &mut self.lexer {
            LexerRef::Mut(lexer) => {
                lexer.consume_char();
                true
            }
            LexerRef::Shared(_) => false,
        }
    }

    /// Marks the current match as skipped. Action context only.
    pub const fn skip(&mut self) -> bool {
        self.set_type(SKIP)
    }

    /// Extends the current token with another lexer-rule match. Action context
    /// only.
    pub const fn more(&mut self) -> bool {
        self.set_type(MORE)
    }

    /// Repositions the committed accept cursor. Action context only.
    pub fn reset_accept_position(&mut self, index: usize) -> bool {
        match &mut self.lexer {
            LexerRef::Mut(lexer) => {
                lexer.reset_accept_position(index);
                true
            }
            LexerRef::Shared(_) => false,
        }
    }

    /// Moves the current token start forward within the committed match.
    ///
    /// This is used after queueing a prefix token so automatic emission covers
    /// only the remaining suffix. Returns `false` for predicate contexts or an
    /// index outside the current token span.
    pub fn set_token_start(&mut self, index: usize) -> bool {
        match &mut self.lexer {
            LexerRef::Mut(lexer) => lexer.set_token_start(index),
            LexerRef::Shared(_) => false,
        }
    }

    /// Queues an additional token on the current channel.
    ///
    /// The queued token spans the current token start through `stop`
    /// (inclusive) and is returned before the match's automatically emitted
    /// token. Action context only.
    pub fn enqueue_token(&mut self, token_type: i32, stop: usize) -> bool {
        let channel = self.channel();
        self.enqueue_token_with_channel(token_type, channel, stop)
    }

    /// Queues an additional token on an explicit channel. See
    /// [`Self::enqueue_token`].
    pub fn enqueue_token_with_channel(
        &mut self,
        token_type: i32,
        channel: i32,
        stop: usize,
    ) -> bool {
        match &mut self.lexer {
            LexerRef::Mut(lexer) => {
                lexer.enqueue_token(token_type, channel, stop, None);
                true
            }
            LexerRef::Shared(_) => false,
        }
    }

    /// Sets the current lexer mode. Available only from a custom-action hook
    /// (the mutable-borrow context); a no-op with a warning path for the
    /// speculative predicate context, where mutating lexer state is invalid.
    ///
    /// Returns `true` if the mutation was applied (action context), `false` if
    /// it was ignored (predicate context).
    pub fn set_mode(&mut self, mode: i32) -> bool {
        match &mut self.lexer {
            LexerRef::Mut(lexer) => {
                lexer.set_mode(mode);
                true
            }
            LexerRef::Shared(_) => false,
        }
    }

    /// Pushes the current mode and switches to `mode`. Action context only; see
    /// [`Self::set_mode`] for the return value.
    pub fn push_mode(&mut self, mode: i32) -> bool {
        match &mut self.lexer {
            LexerRef::Mut(lexer) => {
                lexer.push_mode(mode);
                true
            }
            LexerRef::Shared(_) => false,
        }
    }

    /// Pops the mode stack, restoring the previous mode. Action context only;
    /// returns the popped mode (`None` if the stack was empty or this is a
    /// predicate context).
    pub fn pop_mode(&mut self) -> Option<i32> {
        match &mut self.lexer {
            LexerRef::Mut(lexer) => lexer.pop_mode(),
            LexerRef::Shared(_) => None,
        }
    }
}

/// Mutable lexer state exposed at lifecycle boundaries that have no ATN
/// semantic coordinate.
///
/// The context is used before a token request starts matching, after an
/// accepted path has applied its actions but before emission, and while a
/// lexer is reset for reuse. [`Self::accept_position`] is present only at the
/// post-accept boundary.
#[derive(Debug)]
pub struct LexerLifecycleCtx<'a, I>
where
    I: CharStream,
{
    lexer: &'a mut BaseLexer<I>,
    accept_position: Option<usize>,
}

impl<'a, I> LexerLifecycleCtx<'a, I>
where
    I: CharStream,
{
    pub(crate) const fn new(lexer: &'a mut BaseLexer<I>, accept_position: Option<usize>) -> Self {
        Self {
            lexer,
            accept_position,
        }
    }

    /// Original input boundary selected by the accepted ATN path.
    ///
    /// A post-accept hook may move the committed cursor away from this
    /// boundary with [`Self::reset_accept_position`].
    #[must_use]
    pub const fn accept_position(&self) -> Option<usize> {
        self.accept_position
    }

    /// Current committed input position.
    #[must_use]
    pub fn input_position(&self) -> usize {
        self.lexer.input().index()
    }

    /// Current lexer mode.
    #[must_use]
    pub const fn mode(&self) -> i32 {
        self.lexer.mode
    }

    /// Current source line.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.lexer.line()
    }

    /// Current source column.
    #[must_use]
    pub const fn column(&self) -> usize {
        self.lexer.column()
    }

    /// Absolute source index where the current token begins.
    #[must_use]
    pub const fn token_start(&self) -> usize {
        self.lexer.token_start()
    }

    /// Source line captured at the current token start.
    #[must_use]
    pub const fn token_start_line(&self) -> usize {
        self.lexer.token_start_line()
    }

    /// Source column captured at the current token start.
    #[must_use]
    pub const fn token_start_column(&self) -> usize {
        self.lexer.token_start_column()
    }

    /// Pending type of the token being matched.
    #[must_use]
    pub const fn token_type(&self) -> i32 {
        self.lexer.token_type()
    }

    /// Pending channel of the token being matched.
    #[must_use]
    pub const fn channel(&self) -> i32 {
        self.lexer.channel()
    }

    /// Number of tokens waiting to be returned before another ATN match.
    #[must_use]
    pub fn pending_token_count(&self) -> usize {
        self.lexer.pending_tokens.len()
    }

    /// Text from the current token start through the committed input cursor.
    #[must_use]
    pub fn token_text(&self) -> String {
        self.lexer.token_text()
    }

    /// Text selected by the original accepted ATN path.
    ///
    /// Returns `None` outside the post-accept callback.
    #[must_use]
    pub fn accepted_text(&self) -> Option<String> {
        self.accept_position
            .map(|position| self.lexer.token_text_until(position))
    }

    /// Character at a one-based lookahead/lookbehind offset from the
    /// committed input cursor.
    pub fn la(&mut self, offset: isize) -> i32 {
        self.lexer.la(offset)
    }

    /// Consumes one input character and updates source position tracking.
    pub fn consume(&mut self) {
        self.lexer.consume_char();
    }

    /// Overrides the pending emitted token type.
    pub const fn set_type(&mut self, token_type: i32) {
        self.lexer.set_type(token_type);
    }

    /// Overrides the pending emitted token channel.
    pub const fn set_channel(&mut self, channel: i32) {
        self.lexer.set_channel(channel);
    }

    /// Marks the current match as skipped.
    pub const fn skip(&mut self) {
        self.lexer.skip();
    }

    /// Extends the current token with another lexer-rule match.
    pub const fn more(&mut self) {
        self.lexer.more();
    }

    /// Repositions the committed accept cursor.
    pub fn reset_accept_position(&mut self, index: usize) {
        self.lexer.reset_accept_position(index);
    }

    /// Moves the current token start forward within the committed match.
    pub fn set_token_start(&mut self, index: usize) -> bool {
        self.lexer.set_token_start(index)
    }

    /// Queues an additional token on the current channel.
    pub fn enqueue_token(&mut self, token_type: i32, stop: usize) {
        self.enqueue_token_with_channel(token_type, self.channel(), stop);
    }

    /// Queues an additional token on an explicit channel.
    pub fn enqueue_token_with_channel(&mut self, token_type: i32, channel: i32, stop: usize) {
        self.lexer.enqueue_token(token_type, channel, stop, None);
    }

    /// Sets the current lexer mode.
    pub fn set_mode(&mut self, mode: i32) {
        self.lexer.set_mode(mode);
    }

    /// Pushes the current mode and switches to `mode`.
    pub fn push_mode(&mut self, mode: i32) {
        self.lexer.push_mode(mode);
    }

    /// Pops the mode stack, restoring the previous mode.
    pub fn pop_mode(&mut self) -> Option<i32> {
        self.lexer.pop_mode()
    }
}

pub trait Lexer: Recognizer {
    fn mode(&self) -> i32;
    fn set_mode(&mut self, mode: i32);
    fn push_mode(&mut self, mode: i32);
    fn pop_mode(&mut self) -> Option<i32>;
}

#[derive(Clone, Debug)]
pub struct BaseLexer<I> {
    input: I,
    data: RecognizerData,
    has_source_text: bool,
    mode: i32,
    mode_stack: Vec<i32>,
    token_type: i32,
    channel: i32,
    token_start: usize,
    token_start_line: usize,
    token_start_column: usize,
    line: usize,
    column: usize,
    hit_eof: bool,
    force_interpreted: bool,
    errors: RefCell<Vec<TokenSourceError>>,
    semantic_error_coordinates: RefCell<BTreeSet<(u8, usize, usize, usize)>>,
    pending_tokens: VecDeque<TokenSpec>,
    dfa_cache: Rc<RefCell<LexerDfaCache>>,
}

/// Learned lexer DFA: the input-independent state/transition tables built up
/// by ATN simulation.
///
/// Semantic-predicate-dependent states are stored flagged and every consumer
/// re-simulates them instead of trusting their cached data, so the cache can
/// be shared across lexer instances (and inputs) for the same ATN — see
/// [`BaseLexer::with_shared_dfa`].
#[derive(Debug, Default)]
struct LexerDfaCache {
    prediction: LexerPredictionStore,
    state_numbers: FxHashMap<LexerDfaKey, usize>,
    accept_predictions: FxHashMap<usize, i32>,
    /// `showDFA` edge trace. Lives with the tables it describes, so a lexer
    /// on a shared cache reports the accumulated DFA — matching the reference
    /// runtimes, whose static shared DFA is what `showDFA` prints.
    edges: BTreeSet<LexerDfaEdge>,
    /// Dense by DFA state number (states are numbered contiguously from 0).
    cached_states: Vec<Option<Rc<LexerDfaCachedState>>>,
    /// Per-source-state edge rows for symbols in `0..DENSE_EDGE_SYMBOLS`,
    /// allocated lazily on the first cached transition out of a state. The
    /// per-character lookup is then one bounds check and an array index —
    /// the same scheme as Go's `edges[t-MinDFAEdge]`.
    dense_edges: Vec<Option<Box<DenseEdgeRow>>>,
    /// Transitions on symbols outside the dense range (supplementary planes).
    sparse_edges: FxHashMap<(usize, i32), LexerDfaCachedTransition>,
    mode_starts: FxHashMap<i32, usize>,
}

/// Canonical caller contexts paired with the learned lexer DFA that stores
/// their IDs.
#[derive(Debug, Default)]
pub(crate) struct LexerPredictionStore {
    pub(crate) contexts: LexerContextArena,
    pub(crate) workspace: PredictionWorkspace,
}

/// Store-local identity for one ordered lexer caller-context node.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct LexerContextId(u32);

pub(crate) const EMPTY_LEXER_CONTEXT: LexerContextId = LexerContextId(0);

/// One node in an ordered graph of lexer caller stacks.
///
/// `Union` preserves ATN traversal priority. The paired unordered prediction
/// context detects when a later union adds no stack paths, which keeps cyclic
/// lexer closures finite without flattening their priority order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum LexerContextNode {
    Empty,
    Singleton {
        parent: LexerContextId,
        return_state: usize,
    },
    Union {
        left: LexerContextId,
        right: LexerContextId,
    },
}

#[derive(Clone, Copy, Debug)]
struct LexerContextRecord {
    node: LexerContextNode,
    path_set: ContextId,
}

/// Canonical ordered caller-context DAG for one learned or compiled lexer DFA.
#[derive(Debug)]
pub(crate) struct LexerContextArena {
    records: Vec<LexerContextRecord>,
    ids: FxHashMap<LexerContextNode, LexerContextId>,
    path_sets: ContextArena,
}

impl LexerContextArena {
    pub(crate) fn new() -> Self {
        let mut ids = FxHashMap::default();
        ids.insert(LexerContextNode::Empty, EMPTY_LEXER_CONTEXT);
        Self {
            records: vec![LexerContextRecord {
                node: LexerContextNode::Empty,
                path_set: EMPTY_CONTEXT,
            }],
            ids,
            path_sets: ContextArena::new(),
        }
    }

    pub(crate) fn singleton(
        &mut self,
        parent: LexerContextId,
        return_state: usize,
    ) -> LexerContextId {
        self.assert_valid(parent);
        let node = LexerContextNode::Singleton {
            parent,
            return_state,
        };
        if let Some(&context) = self.ids.get(&node) {
            return context;
        }
        let path_set = self
            .path_sets
            .singleton(self.record(parent).path_set, return_state);
        self.intern(node, path_set)
    }

    pub(crate) fn merge(
        &mut self,
        left: LexerContextId,
        right: LexerContextId,
        workspace: &mut PredictionWorkspace,
    ) -> LexerContextId {
        self.assert_valid(left);
        self.assert_valid(right);
        if left == right {
            return left;
        }
        let left_set = self.record(left).path_set;
        let right_set = self.record(right).path_set;
        let path_set = self.path_sets.merge(left_set, right_set, false, workspace);
        if path_set == left_set {
            return left;
        }
        let node = LexerContextNode::Union { left, right };
        if let Some(&context) = self.ids.get(&node) {
            return context;
        }
        self.intern(node, path_set)
    }

    pub(crate) fn node(&self, context: LexerContextId) -> LexerContextNode {
        self.record(context).node
    }

    #[cfg(test)]
    pub(crate) const fn len(&self) -> usize {
        self.records.len()
    }

    fn intern(&mut self, node: LexerContextNode, path_set: ContextId) -> LexerContextId {
        let context = LexerContextId(
            u32::try_from(self.records.len()).expect("lexer context arena must fit in u32"),
        );
        self.records.push(LexerContextRecord { node, path_set });
        self.ids.insert(node, context);
        context
    }

    fn record(&self, context: LexerContextId) -> &LexerContextRecord {
        self.assert_valid(context);
        &self.records[usize::try_from(context.0).expect("u32 lexer context ID fits in usize")]
    }

    fn assert_valid(&self, context: LexerContextId) {
        assert!(
            usize::try_from(context.0).is_ok_and(|index| index < self.records.len()),
            "lexer context ID does not belong to this store"
        );
    }
}

impl Default for LexerContextArena {
    fn default() -> Self {
        Self::new()
    }
}

/// Dense-row width: ASCII, matching the reference runtimes' DFA edge arrays.
const DENSE_EDGE_SYMBOLS: usize = 128;

type DenseEdgeRow = [LexerDfaCachedTransition; DENSE_EDGE_SYMBOLS];

/// Sentinel for an empty dense-row slot; no real transition targets it
/// because DFA state numbers are assigned contiguously from 0.
const EMPTY_DENSE_EDGE: LexerDfaCachedTransition = LexerDfaCachedTransition {
    target_state: usize::MAX,
    position_delta: 0,
};

thread_local! {
    /// Learned lexer DFAs shared across lexer instances, keyed by a generated
    /// lexer's static ATN identity (mirrors the parser's shared decision DFAs).
    static SHARED_LEXER_DFA_CACHES: RefCell<HashMap<usize, Rc<RefCell<LexerDfaCache>>>> =
        RefCell::new(HashMap::new());
}

/// Normalized lexer ATN config-set identity used for observed DFA traces.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct LexerDfaKey {
    configs: Vec<LexerDfaConfigKey>,
}

impl LexerDfaKey {
    pub(crate) fn new(mut configs: Vec<LexerDfaConfigKey>) -> Self {
        configs.sort_unstable();
        Self { configs }
    }
}

/// One lexer ATN config identity with the absolute input position removed.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct LexerDfaConfigKey {
    pub(crate) state: usize,
    pub(crate) alt_rule_index: Option<usize>,
    pub(crate) consumed_eof: bool,
    pub(crate) passed_non_greedy: bool,
    pub(crate) context: LexerContextId,
    pub(crate) actions: Vec<LexerDfaActionKey>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct LexerDfaActionKey {
    pub(crate) action_index: usize,
    pub(crate) position_delta: usize,
    pub(crate) rule_index: usize,
}

impl LexerDfaConfigKey {
    pub(crate) const fn new(
        state: usize,
        alt_rule_index: Option<usize>,
        consumed_eof: bool,
        passed_non_greedy: bool,
        context: LexerContextId,
        actions: Vec<LexerDfaActionKey>,
    ) -> Self {
        Self {
            state,
            alt_rule_index,
            consumed_eof,
            passed_non_greedy,
            context,
            actions,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LexerDfaCachedTransition {
    pub(crate) target_state: usize,
    pub(crate) position_delta: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct LexerDfaCachedAccept {
    pub(crate) position_delta: usize,
    pub(crate) rule_index: usize,
    pub(crate) consumed_eof: bool,
    pub(crate) actions: Vec<LexerDfaActionKey>,
}

#[derive(Clone, Debug)]
pub(crate) struct LexerDfaCachedState {
    pub(crate) has_semantic_context: bool,
    pub(crate) configs: Vec<LexerDfaConfigKey>,
    pub(crate) accept: Option<LexerDfaCachedAccept>,
}

/// One printable lexer DFA edge keyed so repeated matches keep deterministic
/// output order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LexerDfaEdge {
    from: usize,
    symbol: i32,
    to: usize,
}

impl<I> BaseLexer<I>
where
    I: CharStream,
{
    pub fn new(input: I, data: RecognizerData) -> Self {
        let has_source_text = input.source_text().is_some();
        Self {
            input,
            data,
            has_source_text,
            mode: DEFAULT_MODE,
            mode_stack: Vec::new(),
            token_type: INVALID_TOKEN_TYPE,
            channel: DEFAULT_CHANNEL,
            token_start: 0,
            token_start_line: 1,
            token_start_column: 0,
            line: 1,
            column: 0,
            hit_eof: false,
            force_interpreted: false,
            errors: RefCell::new(Vec::new()),
            semantic_error_coordinates: RefCell::new(BTreeSet::new()),
            pending_tokens: VecDeque::new(),
            dfa_cache: Rc::new(RefCell::new(LexerDfaCache::default())),
        }
    }

    /// Resets runtime-owned lexer state so this instance can consume its input
    /// again from the beginning.
    ///
    /// Learned DFA tables and configuration such as forced interpretation are
    /// retained. Token-production state, diagnostics, pending tokens, modes,
    /// and source position are cleared.
    pub fn reset(&mut self) {
        self.input.seek(0);
        self.mode = DEFAULT_MODE;
        self.mode_stack.clear();
        self.token_type = INVALID_TOKEN_TYPE;
        self.channel = DEFAULT_CHANNEL;
        self.token_start = 0;
        self.token_start_line = 1;
        self.token_start_column = 0;
        self.line = 1;
        self.column = 0;
        self.hit_eof = false;
        self.errors.get_mut().clear();
        self.semantic_error_coordinates.get_mut().clear();
        self.pending_tokens.clear();
    }

    /// Replaces the character stream and fully resets lexer state for reuse.
    ///
    /// Learned DFA tables and configuration such as forced interpretation are
    /// retained. The new stream is always rewound to its beginning.
    pub fn set_input_stream(&mut self, input: I) {
        self.input = input;
        self.has_source_text = self.input.source_text().is_some();
        self.reset();
    }

    /// Switches this lexer to the thread-shared learned DFA for `atn`.
    ///
    /// Generated lexers create a fresh instance per parse; without sharing,
    /// every instance relearns the same DFA through ATN simulation. The shared
    /// cache is keyed by the generated lexer's `&'static LexerAtn` identity and
    /// holds only input-independent data, so it stays valid across inputs.
    /// The `showDFA` edge trace lives in the cache too, so it reports the
    /// accumulated DFA — the same view the reference runtimes print from
    /// their static shared DFA.
    #[must_use]
    pub fn with_shared_dfa(mut self, atn: &'static LexerAtn) -> Self {
        let ptr: *const LexerAtn = atn;
        let key = ptr as usize;
        self.dfa_cache = SHARED_LEXER_DFA_CACHES
            .with(|caches| Rc::clone(caches.borrow_mut().entry(key).or_insert_with(Rc::default)));
        self
    }

    /// Clears the learned lexer DFA shared by recognizers for this grammar.
    ///
    /// Ahead-of-time compiled DFA tables are immutable generated data and are
    /// unaffected. Any path that falls back to ATN interpretation relearns its
    /// dynamic DFA from an empty cache after this call.
    pub fn clear_dfa(&self) {
        let mut cache = self.dfa_cache.borrow_mut();
        // In-flight predicate evaluation may clear the DFA while its configs
        // still hold store-local context IDs.
        let prediction = std::mem::take(&mut cache.prediction);
        *cache = LexerDfaCache {
            prediction,
            ..LexerDfaCache::default()
        };
    }

    pub const fn input(&self) -> &I {
        &self.input
    }

    pub const fn input_mut(&mut self) -> &mut I {
        &mut self.input
    }

    /// Captures the input index and source position for the token currently
    /// being matched.
    pub fn begin_token(&mut self) {
        self.semantic_error_coordinates.get_mut().clear();
        self.token_type = INVALID_TOKEN_TYPE;
        self.channel = DEFAULT_CHANNEL;
        self.token_start = self.input.index();
        self.token_start_line = self.line;
        self.token_start_column = self.column;
    }

    /// Returns the absolute character index where the current token began.
    pub const fn token_start(&self) -> usize {
        self.token_start
    }

    /// Returns the source line captured at the start of the current token.
    pub const fn token_start_line(&self) -> usize {
        self.token_start_line
    }

    /// Returns the source column captured at the start of the current token.
    pub const fn token_start_column(&self) -> usize {
        self.token_start_column
    }

    /// Returns the pending type of the token being matched.
    pub const fn token_type(&self) -> i32 {
        self.token_type
    }

    /// Overrides the pending type of the token being matched.
    pub const fn set_type(&mut self, token_type: i32) {
        self.token_type = token_type;
    }

    /// Returns the pending channel of the token being matched.
    pub const fn channel(&self) -> i32 {
        self.channel
    }

    /// Overrides the pending channel of the token being matched.
    pub const fn set_channel(&mut self, channel: i32) {
        self.channel = channel;
    }

    /// Marks the current match as skipped.
    pub const fn skip(&mut self) {
        self.set_type(SKIP);
    }

    /// Extends the current token with another lexer-rule match.
    pub const fn more(&mut self) {
        self.set_type(MORE);
    }

    /// Reads a character at a one-based lookahead/lookbehind offset from the
    /// committed input cursor without moving it.
    pub fn la(&mut self, offset: isize) -> i32 {
        self.input.la(offset)
    }

    fn lookahead_at(&self, position: usize, offset: isize) -> i32 {
        if offset == 0 {
            return 0;
        }
        let absolute = if offset > 0 {
            position.checked_add((offset - 1).cast_unsigned())
        } else {
            offset
                .checked_neg()
                .and_then(|distance| usize::try_from(distance).ok())
                .and_then(|distance| position.checked_sub(distance))
        };
        let Some(index) = absolute.filter(|index| *index < self.input.size()) else {
            return EOF;
        };
        if let Some(symbol) = self.input.symbol_at(index) {
            return symbol;
        }
        self.input
            .text(TextInterval::new(index, index))
            .chars()
            .next()
            .map_or(EOF, |ch| u32::from(ch).cast_signed())
    }

    /// Consumes one character from the input stream and updates lexer line and
    /// column counters.
    ///
    /// The input stream is indexed by Unicode scalar values. Newline handling
    /// follows ANTLR's default convention of incrementing the line and resetting
    /// the column after `\n`.
    pub fn consume_char(&mut self) {
        let la = self.input.la(1);
        if la == EOF {
            return;
        }
        self.input.consume();
        if char::from_u32(la.cast_unsigned()) == Some('\n') {
            self.line += 1;
            self.column = 0;
        } else {
            self.column += 1;
        }
    }

    /// Commits a predicted input span while keeping the current line and column
    /// as the coordinates at `start`.
    pub(crate) fn commit_position(&mut self, start: usize, target: usize) {
        self.reposition_from(start, self.line, self.column, target);
    }

    fn reposition_from(&mut self, start: usize, line: usize, column: usize, target: usize) {
        let start = start.min(self.input.size());
        let target = target.max(start).min(self.input.size());
        if let Some(summary) = self.input.position_summary(start, target) {
            self.input.seek(target);
            (self.line, self.column) = summary.apply(line, column);
            #[cfg(feature = "perf-counters")]
            crate::perf::record_lexer_bulk_commit(target - start);
            return;
        }

        self.input.seek(start);
        self.line = line;
        self.column = column;
        #[cfg(feature = "perf-counters")]
        let before = self.input.index();
        while self.input.index() < target && self.input.la(1) != EOF {
            self.consume_char();
        }
        #[cfg(feature = "perf-counters")]
        crate::perf::record_lexer_scalar_replay(self.input.index().saturating_sub(before));
    }

    /// Rewinds or advances the input cursor to a token accept boundary.
    ///
    /// Some generated lexers intentionally accept a longer path to disambiguate
    /// a token, then emit only the prefix and leave the suffix for the next
    /// token. Recomputing line/column from `token_start` keeps the visible lexer
    /// position consistent after moving the cursor backwards.
    pub fn reset_accept_position(&mut self, index: usize) {
        let target = index.max(self.token_start);
        self.reposition_from(
            self.token_start,
            self.token_start_line,
            self.token_start_column,
            target,
        );
    }

    /// Moves the current token start forward within the consumed input span.
    ///
    /// Source line and column are advanced with the start, so a subsequently
    /// emitted suffix token carries the same coordinates it would have had if
    /// lexed independently.
    pub fn set_token_start(&mut self, index: usize) -> bool {
        if index < self.token_start || index > self.input.index() {
            return false;
        }
        let (line, column) = self.position_at(index);
        self.token_start = index;
        self.token_start_line = line;
        self.token_start_column = column;
        true
    }

    /// Builds a token spanning from the current token start to the character
    /// before the input cursor.
    ///
    /// When generated or interpreted lexer code does not supply explicit text,
    /// the base lexer captures the matched source interval so downstream token
    /// streams and parse trees can render token text without retaining a source
    /// pair object.
    pub fn emit(
        &self,
        sink: &mut TokenSink<'_>,
        token_type: i32,
        channel: i32,
        text: Option<String>,
    ) -> Result<TokenId, TokenStoreError> {
        let stop = self.input.index().checked_sub(1).unwrap_or(usize::MAX);
        self.emit_with_stop(sink, token_type, channel, stop, text)
    }

    /// Builds a token with an explicit stop index.
    ///
    /// EOF-matching lexer rules do not consume a Unicode scalar value, so their
    /// stop index can be one before the current input index. The caller passes
    /// `usize::MAX` to represent ANTLR's `-1` stop index at empty input.
    pub fn emit_with_stop(
        &self,
        sink: &mut TokenSink<'_>,
        token_type: i32,
        channel: i32,
        stop: usize,
        text: Option<String>,
    ) -> Result<TokenId, TokenStoreError> {
        sink.push(self.token_spec_with_stop(token_type, channel, stop, text))
    }

    fn token_spec_with_stop(
        &self,
        token_type: i32,
        channel: i32,
        stop: usize,
        text: Option<String>,
    ) -> TokenSpec {
        let text = text.or_else(|| {
            if stop == usize::MAX {
                Some("<EOF>".to_owned())
            } else {
                None
            }
        });
        let source_interval = if self.has_source_text
            && text.is_none()
            && stop != usize::MAX
            && self.token_start <= stop
        {
            self.input
                .byte_interval(TextInterval::new(self.token_start, stop))
        } else {
            None
        };
        let text = text.or_else(|| {
            source_interval
                .is_none()
                .then(|| self.input.text(TextInterval::new(self.token_start, stop)))
        });
        let (start_byte, stop_byte) = source_interval
            .or_else(|| self.token_byte_span(stop))
            .unwrap_or((self.token_start, self.token_start));
        TokenSpec {
            token_type,
            channel,
            start: self.token_start,
            stop,
            start_byte,
            stop_byte,
            line: self.token_start_line,
            column: self.token_start_column,
            text,
            source_backed: source_interval.is_some(),
        }
    }

    /// Queues an additional token to be returned before the current match's
    /// automatic token.
    ///
    /// The token spans the current token start through `stop` (inclusive).
    /// `text = None` keeps the token source-backed when the input supports it.
    pub fn enqueue_token(
        &mut self,
        token_type: i32,
        channel: i32,
        stop: usize,
        text: Option<String>,
    ) {
        let token = self.token_spec_with_stop(token_type, channel, stop, text);
        self.pending_tokens.push_back(token);
    }

    pub(crate) fn emit_pending_token(
        &mut self,
        sink: &mut TokenSink<'_>,
    ) -> Result<Option<TokenId>, TokenStoreError> {
        self.pending_tokens
            .pop_front()
            .map(|token| sink.push(token))
            .transpose()
    }

    pub(crate) fn emit_or_enqueue_with_stop(
        &mut self,
        sink: &mut TokenSink<'_>,
        stop: usize,
        text: Option<String>,
    ) -> Result<TokenId, TokenStoreError> {
        let token = self.token_spec_with_stop(self.token_type, self.channel, stop, text);
        self.emit_or_enqueue(sink, token)
    }

    fn emit_or_enqueue(
        &mut self,
        sink: &mut TokenSink<'_>,
        token: TokenSpec,
    ) -> Result<TokenId, TokenStoreError> {
        if self.pending_tokens.is_empty() {
            return sink.push(token);
        }
        self.pending_tokens.push_back(token);
        self.emit_pending_token(sink)?
            .ok_or_else(|| unreachable!("the pending-token queue was just populated"))
    }

    /// Returns the current token text from the token start through the input
    /// cursor.
    pub fn token_text(&self) -> String {
        self.token_text_until(self.input.index())
    }

    /// Returns the current token text from the token start through
    /// `stop_exclusive`.
    ///
    /// Lexer custom actions can occur before the accepted token is complete.
    /// The action event records the position where the transition fired, and
    /// generated action code uses this helper to render ANTLR's `Text()`
    /// template at that exact point.
    pub fn token_text_until(&self, stop_exclusive: usize) -> String {
        if stop_exclusive <= self.token_start {
            return String::new();
        }
        self.input
            .text(TextInterval::new(self.token_start, stop_exclusive - 1))
    }

    /// Computes the zero-based source column at an absolute input position
    /// reached during prediction of the current token.
    pub fn column_at(&self, position: usize) -> usize {
        self.position_at(position).1
    }

    fn position_at(&self, position: usize) -> (usize, usize) {
        let mut line = self.token_start_line;
        let mut column = self.token_start_column;
        if position <= self.token_start {
            return (line, column);
        }
        if let Some(summary) = self.input.position_summary(self.token_start, position) {
            return summary.apply(line, column);
        }
        for ch in self
            .input
            .text(TextInterval::new(self.token_start, position - 1))
            .chars()
        {
            if ch == '\n' {
                line += 1;
                column = 0;
            } else {
                column += 1;
            }
        }
        (line, column)
    }

    /// Builds the synthetic EOF token at the current input cursor.
    pub fn eof_token(&self, sink: &mut TokenSink<'_>) -> Result<TokenId, TokenStoreError> {
        sink.push(self.eof_token_spec())
    }

    pub(crate) fn emit_eof_or_pending(
        &mut self,
        sink: &mut TokenSink<'_>,
    ) -> Result<TokenId, TokenStoreError> {
        let token = self.eof_token_spec();
        self.emit_or_enqueue(sink, token)
    }

    fn eof_token_spec(&self) -> TokenSpec {
        let byte_offset = self.eof_byte_offset().unwrap_or_else(|| self.input.index());
        TokenSpec::eof(self.input.index(), byte_offset, self.line, self.column)
    }

    fn eof_byte_offset(&self) -> Option<usize> {
        self.byte_offset_at(self.input.index())
    }

    fn token_byte_span(&self, stop: usize) -> Option<(usize, usize)> {
        if stop != usize::MAX && self.token_start <= stop {
            let (start_byte, stop_byte) = self
                .input
                .byte_interval(TextInterval::new(self.token_start, stop))?;
            return Some((start_byte, stop_byte));
        }
        let byte_offset = self.byte_offset_at(self.token_start)?;
        Some((byte_offset, byte_offset))
    }

    fn byte_offset_at(&self, index: usize) -> Option<usize> {
        let byte_offset = if index == 0 {
            0
        } else {
            let previous = TextInterval::new(index - 1, index - 1);
            self.input.byte_interval(previous)?.1
        };
        Some(byte_offset)
    }
}

impl<I> Recognizer for BaseLexer<I>
where
    I: CharStream,
{
    fn data(&self) -> &RecognizerData {
        &self.data
    }

    fn data_mut(&mut self) -> &mut RecognizerData {
        &mut self.data
    }
}

impl<I> Lexer for BaseLexer<I>
where
    I: CharStream,
{
    fn mode(&self) -> i32 {
        self.mode
    }

    fn set_mode(&mut self, mode: i32) {
        self.mode = mode;
    }

    fn push_mode(&mut self, mode: i32) {
        self.mode_stack.push(self.mode);
        self.mode = mode;
    }

    fn pop_mode(&mut self) -> Option<i32> {
        let mode = self.mode_stack.pop()?;
        self.mode = mode;
        Some(mode)
    }
}

impl<I> BaseLexer<I>
where
    I: CharStream,
{
    pub const fn line(&self) -> usize {
        self.line
    }

    pub const fn column(&self) -> usize {
        self.column
    }

    pub fn source_name(&self) -> &str {
        self.input.source_name()
    }

    pub fn source_text(&self) -> Option<Rc<str>> {
        self.input.source_text()
    }

    pub const fn hit_eof(&self) -> bool {
        self.hit_eof
    }

    pub const fn set_hit_eof(&mut self, hit_eof: bool) {
        self.hit_eof = hit_eof;
    }

    /// Routes every token through ATN interpretation even when the generated
    /// lexer carries an ahead-of-time compiled DFA.
    ///
    /// Interpretation is what learns the replayable DFA that
    /// [`Self::lexer_dfa_string`] reports, so harnesses asserting on the
    /// observed-DFA trace (ANTLR's `showDFA` descriptors) enable this before
    /// lexing.
    pub const fn set_force_interpreted(&mut self, force_interpreted: bool) {
        self.force_interpreted = force_interpreted;
    }

    /// Whether compiled-DFA entry points must fall back to interpretation.
    pub const fn force_interpreted(&self) -> bool {
        self.force_interpreted
    }

    /// Buffers a lexer diagnostic until the token stream consumer is ready to
    /// emit errors in parser-compatible order.
    pub fn record_error(&self, line: usize, column: usize, message: impl Into<String>) {
        self.errors
            .borrow_mut()
            .push(TokenSourceError::new(line, column, message));
    }

    /// Records one fail-loud semantic-hook miss per coordinate and token start.
    pub fn record_semantic_error(&self, action: bool, rule_index: usize, coordinate_index: usize) {
        let kind = u8::from(action);
        if !self.semantic_error_coordinates.borrow_mut().insert((
            kind,
            rule_index,
            coordinate_index,
            self.token_start,
        )) {
            return;
        }
        let label = if action { "action" } else { "predicate" };
        self.record_error(
            self.token_start_line,
            self.token_start_column,
            format!("unhandled lexer semantic {label}: rule={rule_index} index={coordinate_index}"),
        );
    }

    /// Returns and clears lexer diagnostics produced while fetching tokens.
    pub fn drain_errors(&mut self) -> Vec<TokenSourceError> {
        std::mem::take(self.errors.get_mut())
    }

    /// Borrows the canonical caller-context store paired with this lexer's
    /// learned DFA.
    pub(crate) fn lexer_prediction_store(&self) -> RefMut<'_, LexerPredictionStore> {
        RefMut::map(self.dfa_cache.borrow_mut(), |cache| &mut cache.prediction)
    }

    /// Starts a fresh token prediction while retaining bounded scratch
    /// allocations for subsequent matches.
    pub(crate) fn reset_lexer_prediction_workspace(&self) {
        self.dfa_cache.borrow_mut().prediction.workspace.reset();
    }

    #[cfg(test)]
    pub(crate) fn lexer_dfa_cache_shape(&self) -> (usize, usize, usize, usize) {
        let cache = self.dfa_cache.borrow();
        let cached_states = cache.cached_states.iter().flatten().count();
        let cached_transitions = cache
            .dense_edges
            .iter()
            .flatten()
            .map(|row| {
                row.iter()
                    .filter(|transition| transition.target_state != usize::MAX)
                    .count()
            })
            .sum::<usize>()
            + cache.sparse_edges.len();
        let max_configs = cache
            .cached_states
            .iter()
            .flatten()
            .map(|state| state.configs.len())
            .max()
            .unwrap_or(0);
        let contexts = cache.prediction.contexts.len();
        (cached_states, cached_transitions, max_configs, contexts)
    }

    /// Returns the stable state number for a normalized lexer DFA config set,
    /// creating one if this input path has not reached it before.
    pub(crate) fn lexer_dfa_state(
        &self,
        key: LexerDfaKey,
        accept_prediction: Option<i32>,
    ) -> usize {
        let mut cache = self.dfa_cache.borrow_mut();
        let next = cache.state_numbers.len();
        let state = *cache.state_numbers.entry(key).or_insert(next);
        if let Some(prediction) = accept_prediction {
            cache.accept_predictions.insert(state, prediction);
        }
        state
    }

    /// Records a visible lexer DFA edge unless it was already observed.
    pub fn record_lexer_dfa_edge(&self, from: usize, symbol: i32, to: usize) {
        self.dfa_cache
            .borrow_mut()
            .edges
            .insert(LexerDfaEdge { from, symbol, to });
    }

    pub(crate) fn cached_lexer_dfa_transition(
        &self,
        state: usize,
        symbol: i32,
    ) -> Option<LexerDfaCachedTransition> {
        let cache = self.dfa_cache.borrow();
        if let Ok(sym) = usize::try_from(symbol)
            && sym < DENSE_EDGE_SYMBOLS
        {
            let transition = cache.dense_edges.get(state)?.as_ref()?[sym];
            return (transition.target_state != usize::MAX).then_some(transition);
        }
        cache.sparse_edges.get(&(state, symbol)).copied()
    }

    pub(crate) fn cache_lexer_dfa_transition(
        &self,
        state: usize,
        symbol: i32,
        transition: LexerDfaCachedTransition,
    ) {
        let mut cache = self.dfa_cache.borrow_mut();
        if let Ok(sym) = usize::try_from(symbol)
            && sym < DENSE_EDGE_SYMBOLS
        {
            if cache.dense_edges.len() <= state {
                cache.dense_edges.resize_with(state + 1, || None);
            }
            let row = cache.dense_edges[state]
                .get_or_insert_with(|| Box::new([EMPTY_DENSE_EDGE; DENSE_EDGE_SYMBOLS]));
            // First write wins, matching the previous map `entry().or_insert`.
            if row[sym].target_state == usize::MAX {
                row[sym] = transition;
            }
            return;
        }
        cache
            .sparse_edges
            .entry((state, symbol))
            .or_insert(transition);
    }

    pub(crate) fn cached_lexer_dfa_state(&self, state: usize) -> Option<Rc<LexerDfaCachedState>> {
        self.dfa_cache
            .borrow()
            .cached_states
            .get(state)
            .cloned()
            .flatten()
    }

    pub(crate) fn cache_lexer_dfa_state(&self, state: usize, cached_state: LexerDfaCachedState) {
        let mut cache = self.dfa_cache.borrow_mut();
        if cache.cached_states.len() <= state {
            cache.cached_states.resize_with(state + 1, || None);
        }
        cache.cached_states[state].get_or_insert_with(|| Rc::new(cached_state));
    }

    pub(crate) fn cached_lexer_mode_start(&self, mode: i32) -> Option<usize> {
        self.dfa_cache.borrow().mode_starts.get(&mode).copied()
    }

    pub(crate) fn cache_lexer_mode_start(&self, mode: i32, state: usize) {
        self.dfa_cache
            .borrow_mut()
            .mode_starts
            .entry(mode)
            .or_insert(state);
    }

    /// Serializes the observed default-mode lexer DFA in ANTLR's text shape.
    pub fn lexer_dfa_string(&self) -> String {
        let mut out = String::new();
        let cache = self.dfa_cache.borrow();
        for edge in &cache.edges {
            let Some(label) = lexer_dfa_edge_label(edge.symbol) else {
                continue;
            };
            out.push_str(&self.lexer_dfa_state_string(edge.from));
            out.push('-');
            out.push_str(&label);
            out.push_str("->");
            out.push_str(&self.lexer_dfa_state_string(edge.to));
            out.push('\n');
        }
        out
    }

    fn lexer_dfa_state_string(&self, state: usize) -> String {
        self.dfa_cache
            .borrow()
            .accept_predictions
            .get(&state)
            .map_or_else(
                || format!("s{state}"),
                |prediction| format!(":s{state}=>{prediction}"),
            )
    }
}

fn lexer_dfa_edge_label(symbol: i32) -> Option<String> {
    char::from_u32(symbol.cast_unsigned()).map(|ch| format!("'{ch}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::char_stream::InputStream;
    use crate::int_stream::IntStream;
    use crate::recognizer::RecognizerData;
    use crate::token::{DEFAULT_CHANNEL, Token, TokenStore};
    use crate::vocabulary::Vocabulary;

    #[derive(Clone, Debug)]
    struct UnsharedInput(InputStream);

    impl IntStream for UnsharedInput {
        fn consume(&mut self) {
            self.0.consume();
        }

        fn la(&mut self, offset: isize) -> i32 {
            self.0.la(offset)
        }

        fn index(&self) -> usize {
            self.0.index()
        }

        fn seek(&mut self, index: usize) {
            self.0.seek(index);
        }

        fn size(&self) -> usize {
            self.0.size()
        }

        fn source_name(&self) -> &str {
            self.0.source_name()
        }
    }

    impl CharStream for UnsharedInput {
        fn text(&self, interval: TextInterval) -> String {
            self.0.text(interval)
        }

        fn byte_interval(&self, interval: TextInterval) -> Option<(usize, usize)> {
            self.0.byte_interval(interval)
        }
    }

    #[test]
    fn eof_token_uses_utf8_byte_offset_after_non_ascii_input() {
        let data = RecognizerData::new(
            "T",
            Vocabulary::new(
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
            ),
        );
        let mut lexer = BaseLexer::new(InputStream::new("β"), data);
        lexer.consume_char();

        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let id = lexer.eof_token(&mut sink).expect("test token should fit");
        let token = sink.view(id).expect("emitted token should exist");

        assert_eq!(token.start(), 1);
        assert_eq!(token.stop(), 0);
        assert_eq!(token.text(), "<EOF>");
        assert_eq!(token.byte_span(), 2..2);
    }

    #[test]
    fn eof_rule_token_uses_utf8_byte_offset_after_non_ascii_input() {
        let data = RecognizerData::new(
            "T",
            Vocabulary::new(
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
            ),
        );
        let mut lexer = BaseLexer::new(InputStream::new("β"), data);
        lexer.consume_char();
        lexer.begin_token();

        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let id = lexer
            .emit_with_stop(&mut sink, 1, DEFAULT_CHANNEL, 0, Some("<EOF>".to_owned()))
            .expect("test token should fit");
        let token = sink.view(id).expect("emitted token should exist");

        assert_eq!(token.start(), 1);
        assert_eq!(token.stop(), 0);
        assert_eq!(token.text(), "<EOF>");
        assert_eq!(token.byte_span(), 2..2);
    }

    #[test]
    fn emit_implicit_text_uses_utf8_byte_span_for_non_ascii_input() {
        let data = RecognizerData::new(
            "T",
            Vocabulary::new(
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
            ),
        );
        let mut lexer = BaseLexer::new(InputStream::new("β"), data);
        lexer.begin_token();
        lexer.consume_char();

        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let id = lexer
            .emit(&mut sink, 1, DEFAULT_CHANNEL, None)
            .expect("test token should fit");
        let token = sink.view(id).expect("emitted token should exist");

        assert_eq!(token.start(), 0);
        assert_eq!(token.stop(), 0);
        assert_eq!(token.text(), "β");
        assert_eq!(token.byte_span(), 0..2);
    }

    #[test]
    fn emit_falls_back_to_explicit_text_without_shareable_source() {
        let data = RecognizerData::new(
            "T",
            Vocabulary::new(
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
            ),
        );
        let mut lexer = BaseLexer::new(UnsharedInput(InputStream::new("β")), data);
        lexer.begin_token();
        lexer.consume_char();

        let mut store = TokenStore::new(lexer.source_text(), lexer.source_name());
        let mut sink = TokenSink::new(&mut store);
        let id = lexer
            .emit(&mut sink, 1, DEFAULT_CHANNEL, None)
            .expect("unshared input should emit explicit token text");
        let token = sink.view(id).expect("emitted token should exist");

        assert_eq!(token.text(), "β");
        assert_eq!(token.byte_span(), 0..2);
    }

    #[test]
    fn position_commits_and_rewinds_preserve_line_and_column() {
        let data = RecognizerData::new(
            "T",
            Vocabulary::new(
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
            ),
        );
        let mut lexer = BaseLexer::new(InputStream::new("ab\nγd"), data);
        lexer.begin_token();

        lexer.commit_position(0, 5);
        assert_eq!(lexer.input().index(), 5);
        assert_eq!((lexer.line(), lexer.column()), (2, 2));
        assert_eq!(lexer.column_at(2), 2);
        assert_eq!(lexer.column_at(4), 1);

        lexer.reset_accept_position(3);
        assert_eq!(lexer.input().index(), 3);
        assert_eq!((lexer.line(), lexer.column()), (2, 0));
    }

    #[test]
    fn custom_stream_position_commit_replays_without_fast_path_methods() {
        let data = RecognizerData::new(
            "T",
            Vocabulary::new(
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
            ),
        );
        let mut lexer = BaseLexer::new(UnsharedInput(InputStream::new("a\nb")), data);
        lexer.begin_token();

        lexer.commit_position(0, 3);
        assert_eq!(lexer.input().index(), 3);
        assert_eq!((lexer.line(), lexer.column()), (2, 1));
    }

    #[test]
    fn semantic_hook_errors_are_deduplicated_per_token_coordinate() {
        let data = RecognizerData::new(
            "T",
            Vocabulary::new(
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
            ),
        );
        let mut lexer = BaseLexer::new(InputStream::new("a"), data);
        lexer.begin_token();
        lexer.record_semantic_error(false, 3, 7);
        lexer.record_semantic_error(false, 3, 7);

        let errors = lexer.drain_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "unhandled lexer semantic predicate: rule=3 index=7"
        );

        lexer.begin_token();
        lexer.record_semantic_error(false, 3, 7);
        assert_eq!(
            lexer.drain_errors().len(),
            1,
            "deduplication resets at every token boundary, even after rewinding"
        );
    }

    #[test]
    fn set_input_stream_replaces_input_and_resets_transient_state() {
        let data = RecognizerData::new(
            "T",
            Vocabulary::new(
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
                std::iter::empty::<Option<&str>>(),
            ),
        );
        let mut lexer = BaseLexer::new(InputStream::new("old"), data);
        lexer.consume_char();
        lexer.set_mode(7);
        lexer.push_mode(9);
        lexer.set_type(3);
        lexer.record_error(1, 0, "stale");

        lexer.set_input_stream(InputStream::with_source_name("new", "replacement"));

        assert_eq!(lexer.input().index(), 0);
        assert_eq!(lexer.input().size(), 3);
        assert_eq!(lexer.source_name(), "replacement");
        assert_eq!(lexer.source_text().as_deref(), Some("new"));
        assert_eq!(lexer.mode(), DEFAULT_MODE);
        assert_eq!(lexer.token_type(), INVALID_TOKEN_TYPE);
        assert_eq!((lexer.line(), lexer.column()), (1, 0));
        assert!(!lexer.hit_eof());
        assert!(lexer.drain_errors().is_empty());
        assert!(lexer.pop_mode().is_none());
    }

    #[test]
    fn clear_dfa_invalidates_all_lexers_sharing_the_cache() {
        let atn = Box::leak(Box::new(LexerAtn::new(1)));
        let data = || {
            RecognizerData::new(
                "T",
                Vocabulary::new(
                    std::iter::empty::<Option<&str>>(),
                    std::iter::empty::<Option<&str>>(),
                    std::iter::empty::<Option<&str>>(),
                ),
            )
        };
        let first = BaseLexer::new(InputStream::new("a"), data()).with_shared_dfa(atn);
        let second = BaseLexer::new(InputStream::new("a"), data()).with_shared_dfa(atn);
        let state = first.lexer_dfa_state(LexerDfaKey::new(Vec::new()), Some(1));
        first.record_lexer_dfa_edge(state, i32::from(b'a'), state);

        assert!(!second.lexer_dfa_string().is_empty());
        first.clear_dfa();
        assert!(first.lexer_dfa_string().is_empty());
        assert!(second.lexer_dfa_string().is_empty());
    }
}
