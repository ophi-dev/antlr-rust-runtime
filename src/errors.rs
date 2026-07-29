use std::ops::Range;

use crate::recognizer::Recognizer;
use crate::token::{TokenId, TokenSourceError, TokenView};
use thiserror::Error;

#[derive(Debug, Error, Clone, Eq, PartialEq)]
pub enum AntlrError {
    #[error("mismatched input: expected {expected}, found {found}")]
    MismatchedInput { expected: String, found: String },
    #[error("no viable alternative at input {input}")]
    NoViableAlternative { input: String },
    #[error("lexer error at {line}:{column}: {message}")]
    LexerError {
        line: usize,
        column: usize,
        message: String,
    },
    #[error("parser error at {line}:{column}: {message}")]
    ParserError {
        line: usize,
        column: usize,
        message: String,
        /// Token the error is anchored to, when one exists. The anchor must
        /// be captured where the error is built: prediction restores the
        /// input cursor, so the current lookahead at reporting time is not
        /// necessarily the offending token (`no viable alternative` anchors
        /// at the error index while the cursor sits at the decision start).
        offending: Option<TokenId>,
    },
    #[error("unsupported runtime feature: {0}")]
    Unsupported(String),
}

/// Structured context for one recognizer diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxErrorEvent<'a> {
    /// Token the diagnostic is anchored to, when one exists.
    ///
    /// Lexer errors have no offending token because the failed match did not
    /// produce one.
    pub offending: Option<TokenView<'a>>,
    /// One-based input line where the diagnostic starts.
    pub line: usize,
    /// Zero-based column within `line` where the diagnostic starts.
    pub column: usize,
    /// Half-open UTF-8 byte span of the offending source text.
    ///
    /// Custom streams and token sources that cannot resolve byte offsets leave
    /// this as `None`.
    pub span: Option<Range<usize>>,
    /// ANTLR-compatible diagnostic message without the leading line/column.
    pub message: &'a str,
    /// Recognition error that caused the diagnostic, when one exists.
    pub error: Option<&'a AntlrError>,
}

impl<'a> From<&'a TokenSourceError> for SyntaxErrorEvent<'a> {
    fn from(error: &'a TokenSourceError) -> Self {
        Self {
            offending: None,
            line: error.line,
            column: error.column,
            span: error.span.clone(),
            message: &error.message,
            error: None,
        }
    }
}

/// Receives recognizer diagnostics.
///
/// Listeners registered through [`Recognizer::add_error_listener`] must be
/// [`Send`] and work with every recognizer type. Implement the trait
/// generically, as [`ConsoleErrorListener`] does, when a listener will be
/// registered.
pub trait ErrorListener<R: Recognizer + ?Sized> {
    /// Receives one diagnostic with its ANTLR position and resolved byte span.
    fn syntax_error(&mut self, recognizer: &R, event: &SyntaxErrorEvent<'_>);
}

#[derive(Debug, Default)]
pub struct ConsoleErrorListener;

impl<R: Recognizer + ?Sized> ErrorListener<R> for ConsoleErrorListener {
    #[allow(clippy::print_stderr)]
    fn syntax_error(&mut self, _recognizer: &R, event: &SyntaxErrorEvent<'_>) {
        eprintln!("line {}:{} {}", event.line, event.column, event.message);
    }
}
