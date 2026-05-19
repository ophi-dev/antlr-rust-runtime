//! Clean-room ANTLR v4 runtime foundation for Rust.

pub mod atn;
pub mod char_stream;
pub mod dfa;
pub mod errors;
pub mod generated;
pub mod int_stream;
pub mod lexer;
pub mod parser;
pub mod prediction;
pub mod recognizer;
pub mod token;
pub mod token_stream;
pub mod tree;
pub mod vocabulary;

pub use char_stream::{CharStream, InputStream, TextInterval};
pub use dfa::{Dfa, DfaState};
pub use errors::{AntlrError, ConsoleErrorListener, ErrorListener};
pub use generated::{GeneratedLexer, GeneratedParser, GrammarMetadata};
pub use int_stream::{EOF, IntStream, UNKNOWN_SOURCE_NAME};
pub use lexer::{BaseLexer, Lexer, LexerCustomAction, LexerMode, LexerPredicate};
pub use parser::{
    BaseParser, Parser, ParserAction, ParserMemberAction, ParserPredicate, ParserRuleArg,
    ParserRuntimeOptions,
};
pub use prediction::{AtnConfig, AtnConfigSet, PredictionContext};
pub use recognizer::{Recognizer, RecognizerData};
pub use token::{
    CommonToken, CommonTokenFactory, DEFAULT_CHANNEL, HIDDEN_CHANNEL, INVALID_TOKEN_TYPE,
    TOKEN_EOF, Token, TokenChannel, TokenFactory, TokenSource,
};
pub use token_stream::CommonTokenStream;
pub use tree::{
    ErrorNode, ParseTree, ParseTreeListener, ParseTreeWalker, ParserRuleContext, RuleNode,
    TerminalNode,
};
pub use vocabulary::Vocabulary;
