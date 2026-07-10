use crate::char_stream::TextInterval;
use std::fmt;
use std::ops::Range;
use std::rc::Rc;

pub const TOKEN_EOF: i32 = -1;
pub const INVALID_TOKEN_TYPE: i32 = 0;
pub const DEFAULT_CHANNEL: i32 = 0;
pub const HIDDEN_CHANNEL: i32 = 1;

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
    fn token_type(&self) -> i32;
    fn channel(&self) -> i32;
    /// Zero-based absolute start index measured in Unicode scalar values.
    fn start(&self) -> usize;
    /// Zero-based absolute inclusive stop index measured in Unicode scalar
    /// values.
    fn stop(&self) -> usize;
    fn token_index(&self) -> isize;
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
    ///
    /// The default implementation treats the character index as a byte offset,
    /// which is exact for ASCII and preserves compatibility for token
    /// implementations that do not expose source byte bounds.
    fn start_byte(&self) -> usize {
        self.start()
    }

    /// Zero-based exclusive end offset measured in UTF-8 bytes.
    ///
    /// Unlike [`Self::stop`], this is exclusive so
    /// `token.start_byte()..token.stop_byte()` can slice the original UTF-8
    /// source when the token carries source byte bounds. The default
    /// implementation treats character indices as byte offsets.
    fn stop_byte(&self) -> usize {
        default_stop_byte(self.start(), self.stop())
    }

    /// Zero-based UTF-8 byte span for the token text.
    fn byte_span(&self) -> Range<usize> {
        self.start_byte()..self.stop_byte()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommonToken {
    token_type: i32,
    channel: i32,
    start: usize,
    stop: usize,
    token_index: isize,
    line: usize,
    column: usize,
    text: Option<TokenText>,
    byte_span: Option<TokenByteSpan>,
    source_name: Rc<str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TokenByteSpan {
    start_byte: u32,
    stop_byte: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TokenText {
    Explicit(Rc<str>),
    Source {
        input: Rc<str>,
        start_byte: u32,
        stop_byte: u32,
    },
}

impl TokenText {
    fn as_str(&self) -> &str {
        match self {
            Self::Explicit(text) => text.as_ref(),
            Self::Source {
                input,
                start_byte,
                stop_byte,
            } => &input[*start_byte as usize..*stop_byte as usize],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenSourceText {
    pub input: Rc<str>,
    pub start_byte: u32,
    pub stop_byte: u32,
}

#[derive(Debug)]
pub struct TokenSpec<'a> {
    pub token_type: i32,
    pub channel: i32,
    pub start: usize,
    pub stop: usize,
    pub line: usize,
    pub column: usize,
    pub text: Option<String>,
    pub source_text: Option<TokenSourceText>,
    pub source_name: &'a str,
}

impl CommonToken {
    pub fn new(token_type: i32) -> Self {
        Self {
            token_type,
            channel: DEFAULT_CHANNEL,
            start: 0,
            stop: 0,
            token_index: -1,
            line: 1,
            column: 0,
            text: None,
            byte_span: None,
            source_name: Rc::from(""),
        }
    }

    pub fn eof(source_name: impl Into<Rc<str>>, index: usize, line: usize, column: usize) -> Self {
        Self {
            token_type: TOKEN_EOF,
            channel: DEFAULT_CHANNEL,
            start: index,
            stop: index.checked_sub(1).unwrap_or(usize::MAX),
            token_index: -1,
            line,
            column,
            text: Some(TokenText::Explicit(Rc::from("<EOF>"))),
            byte_span: None,
            source_name: source_name.into(),
        }
    }

    #[must_use]
    pub fn with_text(mut self, text: impl Into<Rc<str>>) -> Self {
        self.text = Some(TokenText::Explicit(text.into()));
        self
    }

    #[must_use]
    pub fn with_source_text(mut self, input: Rc<str>, start_byte: u32, stop_byte: u32) -> Self {
        debug_assert!(
            start_byte <= stop_byte && stop_byte as usize <= input.len(),
            "invalid token source-text bounds: start={start_byte}, stop={stop_byte}, len={}",
            input.len()
        );
        self.text = Some(TokenText::Source {
            input,
            start_byte,
            stop_byte,
        });
        self.byte_span = Some(TokenByteSpan {
            start_byte,
            stop_byte,
        });
        self
    }

    #[must_use]
    pub(crate) fn with_byte_span(mut self, start_byte: u32, stop_byte: u32) -> Self {
        debug_assert!(
            start_byte <= stop_byte,
            "invalid token byte span: start={start_byte}, stop={stop_byte}"
        );
        self.byte_span = Some(TokenByteSpan {
            start_byte,
            stop_byte,
        });
        self
    }

    #[must_use]
    pub const fn with_span(mut self, start: usize, stop: usize) -> Self {
        self.start = start;
        self.stop = stop;
        self
    }

    #[must_use]
    pub const fn with_position(mut self, line: usize, column: usize) -> Self {
        self.line = line;
        self.column = column;
        self
    }

    #[must_use]
    pub const fn with_channel(mut self, channel: i32) -> Self {
        self.channel = channel;
        self
    }

    #[must_use]
    pub fn with_source_name(mut self, source_name: impl Into<Rc<str>>) -> Self {
        self.source_name = source_name.into();
        self
    }

    pub const fn set_token_index(&mut self, token_index: isize) {
        self.token_index = token_index;
    }

    const fn source_byte_span(&self) -> Option<Range<usize>> {
        match self.byte_span {
            Some(TokenByteSpan {
                start_byte,
                stop_byte,
            }) => Some(start_byte as usize..stop_byte as usize),
            None => None,
        }
    }
}

impl Token for CommonToken {
    fn token_type(&self) -> i32 {
        self.token_type
    }

    fn channel(&self) -> i32 {
        self.channel
    }

    fn start(&self) -> usize {
        self.start
    }

    fn stop(&self) -> usize {
        self.stop
    }

    fn token_index(&self) -> isize {
        self.token_index
    }

    fn line(&self) -> usize {
        self.line
    }

    fn column(&self) -> usize {
        self.column
    }

    fn text(&self) -> Option<&str> {
        self.text.as_ref().map(TokenText::as_str)
    }

    fn source_name(&self) -> &str {
        self.source_name.as_ref()
    }

    fn start_byte(&self) -> usize {
        self.source_byte_span()
            .map_or(self.start, |byte_span| byte_span.start)
    }

    fn stop_byte(&self) -> usize {
        self.source_byte_span()
            .map_or_else(|| default_stop_byte(self.start, self.stop), |span| span.end)
    }
}

impl CommonToken {
    /// The token's text, empty when unset — ANTLR's `getText()` shape, which
    /// generated test actions print directly (`token.text()` in `{}`).
    ///
    /// This inherent method intentionally shadows the Option-returning
    /// [`Token::text`] trait method on concrete `CommonToken` values; generic
    /// code keeps the trait signature.
    #[must_use]
    pub fn text(&self) -> &str {
        self.text.as_ref().map_or("", TokenText::as_str)
    }
}

impl fmt::Display for CommonToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = CommonToken::text(self);
        let channel = if self.channel() == DEFAULT_CHANNEL {
            String::new()
        } else {
            format!(",channel={}", self.channel())
        };
        write!(
            f,
            "[@{},{}:{}='{}',<{}>{},{}:{}]",
            self.token_index(),
            display_token_boundary(self.start()),
            display_token_boundary(self.stop()),
            display_text(text),
            self.token_type(),
            channel,
            self.line(),
            self.column()
        )
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

const fn default_stop_byte(start: usize, stop: usize) -> usize {
    match stop.checked_add(1) {
        Some(end) if end >= start => end,
        Some(_) | None => start,
    }
}

/// Escapes token text the way ANTLR's token display format expects.
///
/// Debug escaping is close but not identical: ANTLR leaves ordinary
/// backslashes and quotes unescaped, and only normalizes control characters
/// that would otherwise disrupt the one-line token representation.
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

pub type TokenRef = Rc<CommonToken>;

pub trait TokenFactory {
    fn create(&self, spec: TokenSpec<'_>) -> CommonToken;
}

#[derive(Clone, Debug, Default)]
pub struct CommonTokenFactory;

impl TokenFactory for CommonTokenFactory {
    fn create(&self, spec: TokenSpec<'_>) -> CommonToken {
        let mut token = CommonToken::new(spec.token_type)
            .with_channel(spec.channel)
            .with_span(spec.start, spec.stop)
            .with_position(spec.line, spec.column)
            .with_source_name(spec.source_name);
        if let Some(text) = spec.text {
            token = token.with_text(text);
        } else if let Some(source_text) = spec.source_text {
            token = token.with_source_text(
                source_text.input,
                source_text.start_byte,
                source_text.stop_byte,
            );
        }
        token
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
    fn next_token(&mut self) -> CommonToken;
    fn line(&self) -> usize;
    fn column(&self) -> usize;
    fn source_name(&self) -> &str;
    /// Returns and clears diagnostics emitted while fetching tokens.
    fn drain_errors(&mut self) -> Vec<TokenSourceError> {
        Vec::new()
    }

    /// Serializes lexer DFA cache state when the token source exposes one.
    fn lexer_dfa_string(&self) -> String {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_token_display_matches_antlr_shape() {
        let mut token = CommonToken::new(7)
            .with_text("abc")
            .with_span(2, 4)
            .with_position(3, 9);
        token.set_token_index(5);
        assert_eq!(token.to_string(), "[@5,2:4='abc',<7>,3:9]");
    }

    #[test]
    fn common_token_display_matches_antlr_escaping() {
        let quote = CommonToken::new(1).with_text("\"");
        assert_eq!(quote.to_string(), "[@-1,0:0='\"',<1>,1:0]");

        let newline = CommonToken::new(1).with_text("\n");
        assert_eq!(newline.to_string(), "[@-1,0:0='\\n',<1>,1:0]");

        let backslash = CommonToken::new(1).with_text("\\");
        assert_eq!(backslash.to_string(), "[@-1,0:0='\\',<1>,1:0]");
    }

    #[test]
    fn common_token_display_includes_non_default_channel() {
        let token = CommonToken::new(2).with_text("b").with_channel(2);
        assert_eq!(token.to_string(), "[@-1,0:0='b',<2>,channel=2,1:0]");
    }

    #[test]
    fn eof_display_uses_antlr_empty_input_stop_index() {
        let token = CommonToken::eof("", 0, 1, 0);
        assert_eq!(token.to_string(), "[@-1,0:-1='<EOF>',<-1>,1:0]");
    }

    #[test]
    fn source_backed_token_exposes_utf8_byte_span() {
        let source: Rc<str> = Rc::from("éβz");
        let token = CommonToken::new(1)
            .with_span(1, 1)
            .with_source_text(source, 2, 4);

        assert_eq!(token.start(), 1);
        assert_eq!(token.stop(), 1);
        assert_eq!(token.start_byte(), 2);
        assert_eq!(token.stop_byte(), 4);
        assert_eq!(token.byte_span(), 2..4);
        assert_eq!(token.text(), "β");
    }

    #[test]
    fn explicit_text_byte_span_falls_back_to_character_span() {
        let token = CommonToken::new(1).with_text("β").with_span(3, 3);

        assert_eq!(token.byte_span(), 3..4);
    }

    #[test]
    fn explicit_text_can_carry_utf8_byte_span() {
        let token = CommonToken::new(TOKEN_EOF)
            .with_text("<EOF>")
            .with_span(1, 0)
            .with_byte_span(2, 2);

        assert_eq!(token.text(), "<EOF>");
        assert_eq!(token.byte_span(), 2..2);
    }
}
