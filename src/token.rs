use crate::char_stream::TextInterval;
use std::fmt;
use std::ops::Range;
use std::rc::Rc;

pub const TOKEN_EOF: i32 = -1;
pub const INVALID_TOKEN_TYPE: i32 = 0;
pub const DEFAULT_CHANNEL: i32 = 0;
pub const HIDDEN_CHANNEL: i32 = 1;

/// Largest source or location offset accepted by the compact token store.
///
/// `u32::MAX` is reserved for ANTLR's synthetic `-1` source boundary.
pub const MAX_TOKEN_OFFSET: usize = (u32::MAX - 1) as usize;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TokenId(u32);

impl TokenId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<usize> for TokenId {
    type Error = TokenStoreError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        u32::try_from(value)
            .map(Self)
            .map_err(|_| TokenStoreError::overflow("index", value, u32::MAX as usize))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenChannel {
    Default,
    Hidden,
    Custom(i32),
}

impl TokenChannel {
    pub const fn value(self) -> i32 {
        match self {
            Self::Default => DEFAULT_CHANNEL,
            Self::Hidden => HIDDEN_CHANNEL,
            Self::Custom(channel) => channel,
        }
    }
}

impl From<i32> for TokenChannel {
    fn from(value: i32) -> Self {
        match value {
            DEFAULT_CHANNEL => Self::Default,
            HIDDEN_CHANNEL => Self::Hidden,
            other => Self::Custom(other),
        }
    }
}

pub trait Token: fmt::Debug {
    fn token_id(&self) -> TokenId;
    fn token_type(&self) -> i32;
    fn channel(&self) -> i32;
    /// Zero-based absolute start index measured in Unicode scalar values.
    fn start(&self) -> usize;
    /// Zero-based absolute inclusive stop index measured in Unicode scalar
    /// values.
    fn stop(&self) -> usize;
    /// One-based source line where the token starts.
    fn line(&self) -> usize;
    /// Zero-based source column where the token starts, measured in Unicode
    /// scalar values from the start of `line`.
    fn column(&self) -> usize;
    fn text(&self) -> Option<&str>;
    fn source_name(&self) -> &str;

    fn interval(&self) -> TextInterval {
        TextInterval::new(self.start(), self.stop())
    }

    /// Zero-based absolute start offset measured in UTF-8 bytes.
    fn start_byte(&self) -> usize;

    /// Zero-based exclusive end offset measured in UTF-8 bytes.
    fn stop_byte(&self) -> usize;

    /// Zero-based UTF-8 byte span for the token text.
    fn byte_span(&self) -> Range<usize> {
        self.start_byte()..self.stop_byte()
    }
}

impl<T: Token + ?Sized> Token for &T {
    fn token_id(&self) -> TokenId {
        (**self).token_id()
    }

    fn token_type(&self) -> i32 {
        (**self).token_type()
    }

    fn channel(&self) -> i32 {
        (**self).channel()
    }

    fn start(&self) -> usize {
        (**self).start()
    }

    fn stop(&self) -> usize {
        (**self).stop()
    }

    fn line(&self) -> usize {
        (**self).line()
    }

    fn column(&self) -> usize {
        (**self).column()
    }

    fn text(&self) -> Option<&str> {
        (**self).text()
    }

    fn source_name(&self) -> &str {
        (**self).source_name()
    }

    fn start_byte(&self) -> usize {
        (**self).start_byte()
    }

    fn stop_byte(&self) -> usize {
        (**self).stop_byte()
    }
}

/// The fields emitted for one token.
///
/// This is transient sink input, not an owned token representation. Source
/// text and the source name live once in [`TokenStore`].
#[derive(Clone, Debug)]
pub struct TokenSpec {
    pub token_type: i32,
    pub channel: i32,
    pub start: usize,
    pub stop: usize,
    pub start_byte: usize,
    pub stop_byte: usize,
    pub line: usize,
    pub column: usize,
    pub text: Option<String>,
    pub source_backed: bool,
}

impl TokenSpec {
    #[must_use]
    pub fn explicit(token_type: i32, text: impl Into<String>) -> Self {
        Self {
            token_type,
            channel: DEFAULT_CHANNEL,
            start: 0,
            stop: 0,
            start_byte: 0,
            stop_byte: 1,
            line: 1,
            column: 0,
            text: Some(text.into()),
            source_backed: false,
        }
    }

    #[must_use]
    pub fn eof(index: usize, byte_offset: usize, line: usize, column: usize) -> Self {
        Self {
            token_type: TOKEN_EOF,
            channel: DEFAULT_CHANNEL,
            start: index,
            stop: index.checked_sub(1).unwrap_or(usize::MAX),
            start_byte: byte_offset,
            stop_byte: byte_offset,
            line,
            column,
            text: Some("<EOF>".to_owned()),
            source_backed: false,
        }
    }

    #[must_use]
    pub const fn with_channel(mut self, channel: i32) -> Self {
        self.channel = channel;
        self
    }

    #[must_use]
    pub const fn with_span(mut self, start: usize, stop: usize) -> Self {
        self.start = start;
        self.stop = stop;
        self.start_byte = start;
        self.stop_byte = default_stop_byte(start, stop);
        self
    }

    #[must_use]
    pub const fn with_byte_span(mut self, start_byte: usize, stop_byte: usize) -> Self {
        self.start_byte = start_byte;
        self.stop_byte = stop_byte;
        self
    }

    #[must_use]
    pub const fn with_position(mut self, line: usize, column: usize) -> Self {
        self.line = line;
        self.column = column;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenStoreError(TokenStoreErrorKind);

impl TokenStoreError {
    const fn overflow(field: &'static str, value: usize, limit: usize) -> Self {
        Self(TokenStoreErrorKind::Overflow {
            field,
            value,
            limit,
        })
    }

    const fn invalid_source_boundary(offset: usize, source_len: usize) -> Self {
        Self(TokenStoreErrorKind::InvalidSourceBoundary { offset, source_len })
    }

    pub(crate) const fn invalid_source_output(
        expected_id: usize,
        returned_id: usize,
        appended: usize,
    ) -> Self {
        Self(TokenStoreErrorKind::InvalidSourceOutput {
            expected_id,
            returned_id,
            appended,
        })
    }
}

impl fmt::Display for TokenStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            TokenStoreErrorKind::Overflow {
                field,
                value,
                limit,
            } => write!(
                f,
                "token {field} {value} exceeds the supported limit {limit}"
            ),
            TokenStoreErrorKind::InvalidSourceBoundary { offset, source_len } => write!(
                f,
                "token source byte offset {offset} is not a UTF-8 character boundary \
                 for source length {source_len}"
            ),
            TokenStoreErrorKind::InvalidSourceOutput {
                expected_id,
                returned_id,
                appended,
            } => write!(
                f,
                "token source must append exactly one token and return ID {expected_id}, \
                 but appended {appended} and returned ID {returned_id}"
            ),
        }
    }
}

impl std::error::Error for TokenStoreError {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TokenStoreErrorKind {
    Overflow {
        field: &'static str,
        value: usize,
        limit: usize,
    },
    InvalidSourceBoundary {
        offset: usize,
        source_len: usize,
    },
    InvalidSourceOutput {
        expected_id: usize,
        returned_id: usize,
        appended: usize,
    },
}

/// Canonical compact storage for every token associated with one token stream.
#[derive(Debug)]
pub struct TokenStore {
    source: Option<Rc<str>>,
    source_name: Rc<str>,
    token_types: Vec<i32>,
    channels: Vec<i32>,
    scalar_starts: Vec<u32>,
    scalar_stops: Vec<u32>,
    byte_starts: Vec<u32>,
    byte_stops: Vec<u32>,
    lines: Vec<u32>,
    columns: Vec<u32>,
    source_backed: Vec<bool>,
    explicit_text: Vec<(TokenId, Rc<str>)>,
}

impl TokenStore {
    pub(crate) fn new(source: Option<Rc<str>>, source_name: impl Into<Rc<str>>) -> Self {
        Self {
            source,
            source_name: source_name.into(),
            token_types: Vec::new(),
            channels: Vec::new(),
            scalar_starts: Vec::new(),
            scalar_stops: Vec::new(),
            byte_starts: Vec::new(),
            byte_stops: Vec::new(),
            lines: Vec::new(),
            columns: Vec::new(),
            source_backed: Vec::new(),
            explicit_text: Vec::new(),
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.token_types.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.token_types.is_empty()
    }

    pub(crate) fn push(&mut self, spec: TokenSpec) -> Result<TokenId, TokenStoreError> {
        let raw_id = u32::try_from(self.len())
            .map_err(|_| TokenStoreError::overflow("count", self.len(), u32::MAX as usize))?;
        let id = TokenId(raw_id);
        let scalar_start = compact_boundary("start offset", spec.start)?;
        let scalar_stop = compact_boundary("stop offset", spec.stop)?;
        let byte_start = compact_offset("start byte", spec.start_byte)?;
        let byte_stop = compact_offset("stop byte", spec.stop_byte)?;
        let line = compact_offset("line", spec.line)?;
        let column = compact_offset("column", spec.column)?;

        if spec.source_backed {
            let Some(source) = self.source.as_ref() else {
                return Err(TokenStoreError::overflow("source text", 1, 0));
            };
            if spec.start_byte > spec.stop_byte || spec.stop_byte > source.len() {
                return Err(TokenStoreError::overflow(
                    "source byte span",
                    spec.stop_byte,
                    source.len(),
                ));
            }
            if !source.is_char_boundary(spec.start_byte) {
                return Err(TokenStoreError::invalid_source_boundary(
                    spec.start_byte,
                    source.len(),
                ));
            }
            if !source.is_char_boundary(spec.stop_byte) {
                return Err(TokenStoreError::invalid_source_boundary(
                    spec.stop_byte,
                    source.len(),
                ));
            }
        }

        self.token_types.push(spec.token_type);
        self.channels.push(spec.channel);
        self.scalar_starts.push(scalar_start);
        self.scalar_stops.push(scalar_stop);
        self.byte_starts.push(byte_start);
        self.byte_stops.push(byte_stop);
        self.lines.push(line);
        self.columns.push(column);
        self.source_backed.push(spec.source_backed);
        if let Some(text) = spec.text {
            self.explicit_text.push((id, Rc::from(text)));
        }
        Ok(id)
    }

    const fn contains(&self, id: TokenId) -> bool {
        id.index() < self.len()
    }

    /// Returns a borrowing view of one token record.
    #[must_use]
    pub fn view(&self, id: TokenId) -> Option<TokenView<'_>> {
        self.contains(id).then_some(TokenView { store: self, id })
    }

    /// Returns the token type for `id`.
    #[must_use]
    pub fn token_type(&self, id: TokenId) -> Option<i32> {
        self.token_types.get(id.index()).copied()
    }

    /// Returns the token channel for `id`.
    #[must_use]
    pub fn channel(&self, id: TokenId) -> Option<i32> {
        self.channels.get(id.index()).copied()
    }

    /// Returns the token's zero-based scalar start offset.
    #[must_use]
    pub fn start(&self, id: TokenId) -> Option<usize> {
        self.scalar_starts
            .get(id.index())
            .copied()
            .map(expand_boundary)
    }

    /// Returns the token's zero-based inclusive scalar stop offset.
    #[must_use]
    pub fn stop(&self, id: TokenId) -> Option<usize> {
        self.scalar_stops
            .get(id.index())
            .copied()
            .map(expand_boundary)
    }

    /// Returns the token's one-based source line.
    #[must_use]
    pub fn line(&self, id: TokenId) -> Option<usize> {
        self.lines.get(id.index()).map(|line| *line as usize)
    }

    /// Returns the token's zero-based source column.
    #[must_use]
    pub fn column(&self, id: TokenId) -> Option<usize> {
        self.columns.get(id.index()).map(|column| *column as usize)
    }

    /// Returns the token's zero-based UTF-8 byte start offset.
    #[must_use]
    pub fn start_byte(&self, id: TokenId) -> Option<usize> {
        self.byte_starts
            .get(id.index())
            .map(|offset| *offset as usize)
    }

    /// Returns the token's zero-based exclusive UTF-8 byte stop offset.
    #[must_use]
    pub fn stop_byte(&self, id: TokenId) -> Option<usize> {
        self.byte_stops
            .get(id.index())
            .map(|offset| *offset as usize)
    }

    fn explicit_text(&self, id: TokenId) -> Option<&str> {
        self.explicit_text
            .binary_search_by_key(&id, |(token_id, _)| *token_id)
            .ok()
            .map(|index| self.explicit_text[index].1.as_ref())
    }

    /// Returns explicit or source-backed text for `id`.
    #[must_use]
    pub fn text(&self, id: TokenId) -> Option<&str> {
        if let Some(text) = self.explicit_text(id) {
            return Some(text);
        }
        if !self.source_backed.get(id.index()).copied().unwrap_or(false) {
            return None;
        }
        let source = self.source.as_deref()?;
        let start = self.byte_starts[id.index()] as usize;
        let stop = self.byte_stops[id.index()] as usize;
        source.get(start..stop)
    }
}

const fn default_stop_byte(start: usize, stop: usize) -> usize {
    match stop.checked_add(1) {
        Some(end) if end >= start => end,
        Some(_) | None => start,
    }
}

const fn compact_boundary(field: &'static str, value: usize) -> Result<u32, TokenStoreError> {
    if value == usize::MAX {
        return Ok(u32::MAX);
    }
    compact_offset(field, value)
}

const fn compact_offset(field: &'static str, value: usize) -> Result<u32, TokenStoreError> {
    if value > MAX_TOKEN_OFFSET {
        return Err(TokenStoreError::overflow(field, value, MAX_TOKEN_OFFSET));
    }
    Ok(value as u32)
}

/// Borrowing public view of one canonical token-store record.
#[derive(Clone, Copy)]
pub struct TokenView<'a> {
    store: &'a TokenStore,
    id: TokenId,
}

impl<'a> TokenView<'a> {
    /// The token's text, empty when no explicit or source-backed text exists.
    #[must_use]
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn text(&self) -> &'a str {
        self.store.text(self.id).unwrap_or("")
    }
}

impl fmt::Debug for TokenView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenView")
            .field("id", &self.id)
            .field("token_type", &self.token_type())
            .field("channel", &self.channel())
            .field("text", &self.text())
            .finish()
    }
}

impl PartialEq for TokenView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.token_type() == other.token_type()
            && self.channel() == other.channel()
            && self.start() == other.start()
            && self.stop() == other.stop()
            && self.line() == other.line()
            && self.column() == other.column()
            && self.text() == other.text()
            && self.source_name() == other.source_name()
    }
}

impl Eq for TokenView<'_> {}

impl Token for TokenView<'_> {
    fn token_id(&self) -> TokenId {
        self.id
    }

    fn token_type(&self) -> i32 {
        self.store.token_types[self.id.index()]
    }

    fn channel(&self) -> i32 {
        self.store.channels[self.id.index()]
    }

    fn start(&self) -> usize {
        expand_boundary(self.store.scalar_starts[self.id.index()])
    }

    fn stop(&self) -> usize {
        expand_boundary(self.store.scalar_stops[self.id.index()])
    }

    fn line(&self) -> usize {
        self.store.lines[self.id.index()] as usize
    }

    fn column(&self) -> usize {
        self.store.columns[self.id.index()] as usize
    }

    fn text(&self) -> Option<&str> {
        self.store.text(self.id)
    }

    fn source_name(&self) -> &str {
        self.store.source_name.as_ref()
    }

    fn start_byte(&self) -> usize {
        self.store.byte_starts[self.id.index()] as usize
    }

    fn stop_byte(&self) -> usize {
        self.store.byte_stops[self.id.index()] as usize
    }
}

impl fmt::Display for TokenView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let channel = if self.channel() == DEFAULT_CHANNEL {
            String::new()
        } else {
            format!(",channel={}", self.channel())
        };
        write!(
            f,
            "[@{},{}:{}='{}',<{}>{},{}:{}]",
            display_token_index(self),
            display_token_boundary(self.start()),
            display_token_boundary(self.stop()),
            display_text(self.text()),
            self.token_type(),
            channel,
            self.line(),
            self.column()
        )
    }
}

impl AsRef<str> for TokenView<'_> {
    fn as_ref(&self) -> &str {
        self.text()
    }
}

const fn expand_boundary(value: u32) -> usize {
    if value == u32::MAX {
        usize::MAX
    } else {
        value as usize
    }
}

/// Mutable append-only view used by a token source.
#[derive(Debug)]
pub struct TokenSink<'a> {
    store: &'a mut TokenStore,
}

impl<'a> TokenSink<'a> {
    pub(crate) const fn new(store: &'a mut TokenStore) -> Self {
        Self { store }
    }

    pub fn push(&mut self, spec: TokenSpec) -> Result<TokenId, TokenStoreError> {
        self.store.push(spec)
    }

    pub fn view(&self, id: TokenId) -> Option<TokenView<'_>> {
        self.store.view(id)
    }

    pub(crate) const fn token_count(&self) -> usize {
        self.store.len()
    }
}

/// A diagnostic buffered by a token source while it was producing tokens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenSourceError {
    /// One-based input line where the diagnostic starts.
    pub line: usize,
    /// Zero-based column within `line` where the diagnostic starts.
    pub column: usize,
    /// ANTLR-compatible diagnostic message without the leading line/column.
    pub message: String,
}

impl TokenSourceError {
    /// Creates a token-source diagnostic at the given input position.
    pub fn new(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            column,
            message: message.into(),
        }
    }
}

pub trait TokenSource {
    fn next_token(&mut self, sink: &mut TokenSink<'_>) -> Result<TokenId, TokenStoreError>;
    fn line(&self) -> usize;
    fn column(&self) -> usize;
    fn source_name(&self) -> &str;

    /// Returns the source buffer once for ownership by the token store.
    fn source_text(&self) -> Option<Rc<str>> {
        None
    }

    /// Returns and clears diagnostics emitted while fetching tokens.
    fn drain_errors(&mut self) -> Vec<TokenSourceError> {
        Vec::new()
    }

    /// Reports a buffered diagnostic through source-owned listeners.
    ///
    /// Returns `true` when the source owns diagnostic reporting. The parser
    /// uses its own listeners as a fallback for token sources that return
    /// `false`.
    fn report_error(&self, _error: &TokenSourceError) -> bool {
        false
    }

    /// Serializes lexer DFA cache state when the token source exposes one.
    fn lexer_dfa_string(&self) -> String {
        String::new()
    }
}

fn display_token_index(token: &impl Token) -> String {
    if token.start() == usize::MAX && token.stop() == usize::MAX {
        "-1".to_owned()
    } else {
        token.token_id().index().to_string()
    }
}

/// Formats synthetic-token boundaries with ANTLR's `-1` sentinel.
fn display_token_boundary(value: usize) -> String {
    if value == usize::MAX {
        "-1".to_owned()
    } else {
        value.to_string()
    }
}

/// Escapes token text the way ANTLR's token display format expects.
fn display_text(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn one_token(spec: TokenSpec) -> TokenStore {
        let mut store = TokenStore::new(None, "");
        store.push(spec).expect("test token should fit");
        store
    }

    #[test]
    fn token_view_display_matches_antlr_shape() {
        let store = one_token(
            TokenSpec::explicit(7, "abc")
                .with_span(2, 4)
                .with_position(3, 9),
        );
        assert_eq!(
            store.view(TokenId(0)).expect("token").to_string(),
            "[@0,2:4='abc',<7>,3:9]"
        );
    }

    #[test]
    fn synthetic_token_display_uses_antlr_negative_index() {
        let store = one_token(
            TokenSpec::explicit(7, "<missing X>")
                .with_span(usize::MAX, usize::MAX)
                .with_byte_span(0, 0)
                .with_position(3, 9),
        );
        assert_eq!(
            store.view(TokenId(0)).expect("token").to_string(),
            "[@-1,-1:-1='<missing X>',<7>,3:9]"
        );
    }

    #[test]
    fn source_backed_token_exposes_utf8_byte_span() {
        let mut store = TokenStore::new(Some(Rc::from("éβz")), "");
        let id = store
            .push(TokenSpec {
                token_type: 1,
                channel: DEFAULT_CHANNEL,
                start: 1,
                stop: 1,
                start_byte: 2,
                stop_byte: 4,
                line: 1,
                column: 1,
                text: None,
                source_backed: true,
            })
            .expect("token should fit");
        let token = TokenView { store: &store, id };

        assert_eq!(token.start(), 1);
        assert_eq!(token.stop(), 1);
        assert_eq!(token.byte_span(), 2..4);
        assert_eq!(token.text(), "β");
    }

    #[test]
    fn source_backed_token_rejects_non_utf8_boundaries() {
        for (start_byte, stop_byte) in [(1, 2), (0, 1)] {
            let mut store = TokenStore::new(Some(Rc::from("éz")), "");
            let error = store
                .push(TokenSpec {
                    token_type: 1,
                    channel: DEFAULT_CHANNEL,
                    start: 0,
                    stop: 0,
                    start_byte,
                    stop_byte,
                    line: 1,
                    column: 0,
                    text: None,
                    source_backed: true,
                })
                .expect_err("spans that split UTF-8 code points must fail");

            assert!(error.to_string().contains("UTF-8 character boundary"));
            assert!(store.is_empty());
        }
    }

    #[test]
    fn overlarge_offset_is_rejected() {
        let mut store = TokenStore::new(None, "");
        let error = store
            .push(TokenSpec::explicit(1, "x").with_span(MAX_TOKEN_OFFSET + 1, 0))
            .expect_err("overlarge offsets must fail");
        assert!(error.to_string().contains("supported limit"));
    }
}
