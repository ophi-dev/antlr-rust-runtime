use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::hash::BuildHasherDefault;
use std::rc::Rc;

use crate::atn::LexerAtn;
use crate::char_stream::{CharStream, TextInterval};
use crate::int_stream::EOF;
use crate::prediction::PredictionFxHasher;
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
#[derive(Clone, Debug, Default)]
struct LexerDfaCache {
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
    pub(crate) stack: Vec<usize>,
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
        stack: Vec<usize>,
        actions: Vec<LexerDfaActionKey>,
    ) -> Self {
        Self {
            state,
            alt_rule_index,
            consumed_eof,
            passed_non_greedy,
            stack,
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

    /// Rewinds or advances the input cursor to a token accept boundary.
    ///
    /// Some generated lexers intentionally accept a longer path to disambiguate
    /// a token, then emit only the prefix and leave the suffix for the next
    /// token. Recomputing line/column from `token_start` keeps the visible lexer
    /// position consistent after moving the cursor backwards.
    pub fn reset_accept_position(&mut self, index: usize) {
        let target = index.max(self.token_start);
        self.input.seek(self.token_start);
        self.line = self.token_start_line;
        self.column = self.token_start_column;
        while self.input.index() < target && self.input.la(1) != EOF {
            self.consume_char();
        }
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
}
