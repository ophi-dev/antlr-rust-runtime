use crate::recognizer::Recognizer;
use crate::token::{TokenId, TokenView};
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

/// Receives recognizer diagnostics.
///
/// Listeners registered through [`Recognizer::add_error_listener`] must be
/// [`Send`] and work with every recognizer type. Implement the trait
/// generically, as [`ConsoleErrorListener`] does, when a listener will be
/// registered.
pub trait ErrorListener<R: Recognizer + ?Sized> {
    /// `offending` carries the token the diagnostic points at, matching
    /// ANTLR's `syntaxError(recognizer, offendingSymbol, ...)`. It is `None`
    /// for lexer errors (no token was produced) and for diagnostics that are
    /// not anchored to a specific token.
    #[allow(clippy::too_many_arguments)] // mirrors ANTLR's canonical syntaxError signature
    fn syntax_error(
        &mut self,
        recognizer: &R,
        offending: Option<TokenView<'_>>,
        line: usize,
        column: usize,
        message: &str,
        error: Option<&AntlrError>,
    );
}

#[derive(Debug, Default)]
pub struct ConsoleErrorListener;

impl<R: Recognizer + ?Sized> ErrorListener<R> for ConsoleErrorListener {
    #[allow(clippy::print_stderr)]
    fn syntax_error(
        &mut self,
        _recognizer: &R,
        _offending: Option<TokenView<'_>>,
        line: usize,
        column: usize,
        message: &str,
        _error: Option<&AntlrError>,
    ) {
        eprintln!("line {line}:{column} {message}");
    }
}
