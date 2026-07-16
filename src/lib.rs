//! Clean-room ANTLR v4 runtime foundation for Rust.

pub mod atn;
pub mod char_stream;
pub mod dfa;
pub mod errors;
pub mod generated;
pub mod int_stream;
pub mod lexer;
pub mod parser;
#[cfg(feature = "perf-counters")]
pub mod perf;
pub mod prediction;
pub mod recognizer;
pub mod semir;
pub mod token;
pub mod token_stream;
pub mod tree;
pub mod vocabulary;

pub use atn::parser::{ParserAtnPrediction, ParserAtnSimulator, ParserAtnSimulatorError};
pub use char_stream::{CharStream, InputStream, TextInterval};
pub use dfa::{Dfa, DfaState};
pub use errors::{AntlrError, ConsoleErrorListener, ErrorListener};
pub use generated::{GeneratedLexer, GeneratedParser, GrammarMetadata};
pub use int_stream::{EOF, IntStream, UNKNOWN_SOURCE_NAME};
pub use lexer::{BaseLexer, Lexer, LexerCustomAction, LexerMode, LexerPredicate, LexerSemCtx};
pub use parser::{
    BailErrorStrategy, BaseParser, ExpectedTokenSet, NoSemanticHooks, Parser, ParserAction,
    ParserMemberAction, ParserPredicate, ParserReturnAction, ParserRuleArg, ParserRuntimeOptions,
    ParserSemCtx, ParserSemanticAction, ParserSemanticPredicate, ParserSemantics, PredictionMode,
    RecognitionArenaStats, SemanticHooks, UnknownSemanticPolicy,
};
#[cfg(feature = "perf-counters")]
pub use perf::{dump as dump_prediction_perf_counters, reset as reset_prediction_perf_counters};
pub use prediction::{
    AtnConfig, AtnConfigSet, PredictionContext, PredictionContextMergeCache, SemanticContext,
};
pub use recognizer::{Recognizer, RecognizerData};
pub use token::{
    DEFAULT_CHANNEL, HIDDEN_CHANNEL, INVALID_TOKEN_TYPE, MAX_TOKEN_OFFSET, TOKEN_EOF, Token,
    TokenChannel, TokenId, TokenSink, TokenSource, TokenSpec, TokenStore, TokenStoreError,
    TokenView,
};
pub use token_stream::CommonTokenStream;
pub use tree::{
    ErrorNodeView, FromRuleNode, GeneratedAttrs, Node, NodeChildren, NodeId, NodeKind, ParseTree,
    ParseTreeDescendants, ParseTreeListener, ParseTreeStats, ParseTreeStorage, ParseTreeWalker,
    ParsedFile, ParserRuleContext, RuleNodeView, TerminalNodeView,
};
pub use vocabulary::Vocabulary;

/// Formats a slice the way Java's `List.toString` does: `[a, b, c]`.
///
/// ANTLR's runtime-testsuite descriptors byte-compare output produced by
/// Java's list rendering (`getRuleInvocationStack()`, token-getter lists).
/// Rust's `Vec` `Debug` quotes elements, so — like Go's
/// `antlr.PrintArrayJavaStyle` and Python's `str_list` — the Rust target
/// exposes a dedicated formatter for generated test actions.
pub fn java_style_list<T: std::fmt::Display>(items: &[T]) -> String {
    let mut out = String::from("[");
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        use std::fmt::Write;
        write!(out, "{item}").expect("writing to a string cannot fail");
    }
    out.push(']');
    out
}
