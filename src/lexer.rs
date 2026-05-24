use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::char_stream::{CharStream, TextInterval};
use crate::int_stream::EOF;
use crate::recognizer::{Recognizer, RecognizerData};
use crate::token::{CommonToken, CommonTokenFactory, TokenFactory, TokenSourceError, TokenSpec};

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

pub trait Lexer: Recognizer {
    fn mode(&self) -> i32;
    fn set_mode(&mut self, mode: i32);
    fn push_mode(&mut self, mode: i32);
    fn pop_mode(&mut self) -> Option<i32>;
}

#[derive(Clone, Debug)]
pub struct BaseLexer<I, F = CommonTokenFactory> {
    input: I,
    data: RecognizerData,
    factory: F,
    mode: i32,
    mode_stack: Vec<i32>,
    token_start: usize,
    token_start_line: usize,
    token_start_column: usize,
    line: usize,
    column: usize,
    hit_eof: bool,
    errors: Vec<TokenSourceError>,
    lexer_dfa: LexerDfaTrace,
}

/// Compact observation log for the default-mode lexer DFA printed by `showDFA`
/// runtime-suite descriptors.
#[derive(Clone, Debug, Default)]
struct LexerDfaTrace {
    state_numbers: BTreeMap<LexerDfaKey, usize>,
    accept_predictions: BTreeMap<usize, i32>,
    edges: BTreeSet<LexerDfaEdge>,
    cached_states: BTreeMap<usize, Rc<LexerDfaCachedState>>,
    transitions: BTreeMap<(usize, i32), LexerDfaCachedTransition>,
    mode_starts: BTreeMap<i32, usize>,
}

impl LexerDfaTrace {
    const fn new() -> Self {
        Self {
            state_numbers: BTreeMap::new(),
            accept_predictions: BTreeMap::new(),
            edges: BTreeSet::new(),
            cached_states: BTreeMap::new(),
            transitions: BTreeMap::new(),
            mode_starts: BTreeMap::new(),
        }
    }
}

/// Normalized lexer ATN config-set identity used for observed DFA traces.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
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
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct LexerDfaConfigKey {
    pub(crate) state: usize,
    pub(crate) alt_rule_index: Option<usize>,
    pub(crate) consumed_eof: bool,
    pub(crate) passed_non_greedy: bool,
    pub(crate) stack: Vec<usize>,
    pub(crate) actions: Vec<LexerDfaActionKey>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

#[derive(Clone, Debug)]
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
    /// Creates a lexer base using `CommonTokenFactory`.
    pub const fn new(input: I, data: RecognizerData) -> Self {
        Self::with_factory(input, data, CommonTokenFactory)
    }
}

impl<I, F> BaseLexer<I, F>
where
    I: CharStream,
    F: TokenFactory,
{
    /// Creates a lexer base with a custom token factory.
    pub const fn with_factory(input: I, data: RecognizerData, factory: F) -> Self {
        Self {
            input,
            data,
            factory,
            mode: DEFAULT_MODE,
            mode_stack: Vec::new(),
            token_start: 0,
            token_start_line: 1,
            token_start_column: 0,
            line: 1,
            column: 0,
            hit_eof: false,
            errors: Vec::new(),
            lexer_dfa: LexerDfaTrace::new(),
        }
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

    /// Builds a token spanning from the current token start to the character
    /// before the input cursor.
    ///
    /// When generated or interpreted lexer code does not supply explicit text,
    /// the base lexer captures the matched source interval so downstream token
    /// streams and parse trees can render token text without retaining a source
    /// pair object.
    pub fn emit(&self, token_type: i32, channel: i32, text: Option<String>) -> CommonToken {
        let stop = self.input.index().checked_sub(1).unwrap_or(usize::MAX);
        self.emit_with_stop(token_type, channel, stop, text)
    }

    /// Builds a token with an explicit stop index.
    ///
    /// EOF-matching lexer rules do not consume a Unicode scalar value, so their
    /// stop index can be one before the current input index. The caller passes
    /// `usize::MAX` to represent ANTLR's `-1` stop index at empty input.
    pub fn emit_with_stop(
        &self,
        token_type: i32,
        channel: i32,
        stop: usize,
        text: Option<String>,
    ) -> CommonToken {
        let text = text.or_else(|| {
            if stop == usize::MAX {
                Some("<EOF>".to_owned())
            } else {
                Some(self.input.text(TextInterval::new(self.token_start, stop)))
            }
        });
        self.factory.create(TokenSpec {
            token_type,
            channel,
            start: self.token_start,
            stop,
            line: self.token_start_line,
            column: self.token_start_column,
            text,
            source_name: self.input.source_name(),
        })
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
        let mut column = self.token_start_column;
        if position <= self.token_start {
            return column;
        }
        for ch in self
            .input
            .text(TextInterval::new(self.token_start, position - 1))
            .chars()
        {
            if ch == '\n' {
                column = 0;
            } else {
                column += 1;
            }
        }
        column
    }

    /// Builds the synthetic EOF token at the current input cursor.
    pub fn eof_token(&self) -> CommonToken {
        CommonToken::eof(
            self.input.source_name(),
            self.input.index(),
            self.line,
            self.column,
        )
    }
}

impl<I, F> Recognizer for BaseLexer<I, F>
where
    I: CharStream,
    F: TokenFactory,
{
    fn data(&self) -> &RecognizerData {
        &self.data
    }

    fn data_mut(&mut self) -> &mut RecognizerData {
        &mut self.data
    }
}

impl<I, F> Lexer for BaseLexer<I, F>
where
    I: CharStream,
    F: TokenFactory,
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

impl<I, F> BaseLexer<I, F>
where
    I: CharStream,
    F: TokenFactory,
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

    pub const fn hit_eof(&self) -> bool {
        self.hit_eof
    }

    pub const fn set_hit_eof(&mut self, hit_eof: bool) {
        self.hit_eof = hit_eof;
    }

    /// Buffers a lexer diagnostic until the token stream consumer is ready to
    /// emit errors in parser-compatible order.
    pub fn record_error(&mut self, line: usize, column: usize, message: impl Into<String>) {
        self.errors
            .push(TokenSourceError::new(line, column, message));
    }

    /// Returns and clears lexer diagnostics produced while fetching tokens.
    pub fn drain_errors(&mut self) -> Vec<TokenSourceError> {
        std::mem::take(&mut self.errors)
    }

    /// Returns the stable state number for a normalized lexer DFA config set,
    /// creating one if this input path has not reached it before.
    pub(crate) fn lexer_dfa_state(
        &mut self,
        key: LexerDfaKey,
        accept_prediction: Option<i32>,
    ) -> usize {
        let next = self.lexer_dfa.state_numbers.len();
        let state = *self.lexer_dfa.state_numbers.entry(key).or_insert(next);
        if let Some(prediction) = accept_prediction {
            self.lexer_dfa.accept_predictions.insert(state, prediction);
        }
        state
    }

    /// Records a visible lexer DFA edge unless it was already observed.
    pub fn record_lexer_dfa_edge(&mut self, from: usize, symbol: i32, to: usize) {
        self.lexer_dfa
            .edges
            .insert(LexerDfaEdge { from, symbol, to });
    }

    pub(crate) fn cached_lexer_dfa_transition(
        &self,
        state: usize,
        symbol: i32,
    ) -> Option<LexerDfaCachedTransition> {
        self.lexer_dfa.transitions.get(&(state, symbol)).cloned()
    }

    pub(crate) fn cache_lexer_dfa_transition(
        &mut self,
        state: usize,
        symbol: i32,
        transition: LexerDfaCachedTransition,
    ) {
        self.lexer_dfa
            .transitions
            .entry((state, symbol))
            .or_insert(transition);
    }

    pub(crate) fn cached_lexer_dfa_state(&self, state: usize) -> Option<Rc<LexerDfaCachedState>> {
        self.lexer_dfa.cached_states.get(&state).cloned()
    }

    pub(crate) fn cache_lexer_dfa_state(
        &mut self,
        state: usize,
        cached_state: LexerDfaCachedState,
    ) {
        self.lexer_dfa
            .cached_states
            .entry(state)
            .or_insert_with(|| Rc::new(cached_state));
    }

    pub(crate) fn cached_lexer_mode_start(&self, mode: i32) -> Option<usize> {
        self.lexer_dfa.mode_starts.get(&mode).copied()
    }

    pub(crate) fn cache_lexer_mode_start(&mut self, mode: i32, state: usize) {
        self.lexer_dfa.mode_starts.entry(mode).or_insert(state);
    }

    /// Serializes the observed default-mode lexer DFA in ANTLR's text shape.
    pub fn lexer_dfa_string(&self) -> String {
        let mut out = String::new();
        for edge in &self.lexer_dfa.edges {
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
        self.lexer_dfa.accept_predictions.get(&state).map_or_else(
            || format!("s{state}"),
            |prediction| format!(":s{state}=>{prediction}"),
        )
    }
}

fn lexer_dfa_edge_label(symbol: i32) -> Option<String> {
    char::from_u32(symbol.cast_unsigned()).map(|ch| format!("'{ch}'"))
}
