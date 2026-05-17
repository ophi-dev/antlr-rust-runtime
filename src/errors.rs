use crate::recognizer::Recognizer;
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
    },
    #[error("unsupported runtime feature: {0}")]
    Unsupported(String),
}

pub trait ErrorListener<R: Recognizer> {
    fn syntax_error(
        &mut self,
        recognizer: &R,
        line: usize,
        column: usize,
        message: &str,
        error: Option<&AntlrError>,
    );
}

#[derive(Debug, Default)]
pub struct ConsoleErrorListener;

impl<R: Recognizer> ErrorListener<R> for ConsoleErrorListener {
    #[allow(clippy::print_stderr)]
    fn syntax_error(
        &mut self,
        _recognizer: &R,
        line: usize,
        column: usize,
        message: &str,
        _error: Option<&AntlrError>,
    ) {
        eprintln!("line {line}:{column} {message}");
    }
}
