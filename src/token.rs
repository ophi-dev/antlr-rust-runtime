use crate::char_stream::TextInterval;
use std::fmt;
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
    fn start(&self) -> usize;
    fn stop(&self) -> usize;
    fn token_index(&self) -> isize;
    fn line(&self) -> usize;
    fn column(&self) -> usize;
    fn text(&self) -> Option<&str>;
    fn source_name(&self) -> &str;

    fn interval(&self) -> TextInterval {
        TextInterval::new(self.start(), self.stop())
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
    text: Option<String>,
    source_name: String,
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
    pub source_name: &'a str,
}

impl CommonToken {
    pub const fn new(token_type: i32) -> Self {
        Self {
            token_type,
            channel: DEFAULT_CHANNEL,
            start: 0,
            stop: 0,
            token_index: -1,
            line: 1,
            column: 0,
            text: None,
            source_name: String::new(),
        }
    }

    pub fn eof(source_name: impl Into<String>, index: usize, line: usize, column: usize) -> Self {
        Self {
            token_type: TOKEN_EOF,
            channel: DEFAULT_CHANNEL,
            start: index,
            stop: index.saturating_sub(1),
            token_index: -1,
            line,
            column,
            text: Some("<EOF>".to_owned()),
            source_name: source_name.into(),
        }
    }

    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
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
    pub fn with_source_name(mut self, source_name: impl Into<String>) -> Self {
        self.source_name = source_name.into();
        self
    }

    pub const fn set_token_index(&mut self, token_index: isize) {
        self.token_index = token_index;
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
        self.text.as_deref()
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }
}

impl fmt::Display for CommonToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = self.text().unwrap_or("");
        let stop = if self.token_type() == TOKEN_EOF && self.start() == 0 {
            "-1".to_owned()
        } else {
            self.stop().to_string()
        };
        write!(
            f,
            "[@{},{}:{}='{}',<{}>,{}:{}]",
            self.token_index(),
            self.start(),
            stop,
            display_text(text),
            self.token_type(),
            self.line(),
            self.column()
        )
    }
}

/// Escapes token text the way ANTLR's token display format expects.
///
/// Debug escaping is close but not identical: ANTLR leaves double quotes
/// unescaped because token text is wrapped in single quotes.
fn display_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
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
        }
        token
    }
}

pub trait TokenSource {
    fn next_token(&mut self) -> CommonToken;
    fn line(&self) -> usize;
    fn column(&self) -> usize;
    fn source_name(&self) -> &str;
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
    }

    #[test]
    fn eof_display_uses_antlr_empty_input_stop_index() {
        let token = CommonToken::eof("", 0, 1, 0);
        assert_eq!(token.to_string(), "[@-1,0:-1='<EOF>',<-1>,1:0]");
    }
}
