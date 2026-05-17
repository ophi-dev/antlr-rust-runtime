use crate::char_stream::{CharStream, TextInterval};
use crate::int_stream::EOF;
use crate::recognizer::{Recognizer, RecognizerData};
use crate::token::{CommonToken, CommonTokenFactory, TokenFactory, TokenSpec};

pub const SKIP: i32 = -3;
pub const MORE: i32 = -2;
pub const DEFAULT_MODE: i32 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LexerMode(pub i32);

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

    /// Builds a token spanning from the current token start to the character
    /// before the input cursor.
    ///
    /// When generated or interpreted lexer code does not supply explicit text,
    /// the base lexer captures the matched source interval so downstream token
    /// streams and parse trees can render token text without retaining a source
    /// pair object.
    pub fn emit(&self, token_type: i32, channel: i32, text: Option<String>) -> CommonToken {
        let stop = self.input.index().saturating_sub(1);
        let text =
            text.or_else(|| Some(self.input.text(TextInterval::new(self.token_start, stop))));
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
}
