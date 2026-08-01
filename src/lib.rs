//! Clean-room ANTLR v4 runtime foundation for Rust.

extern crate self as antlr4_runtime;

/// Current generated-source/runtime contract revision emitted by the bundled generator.
#[doc(hidden)]
pub const __ANTLR4_RUST_CODEGEN_API: u32 = 1;

/// Verifies that generated source is compatible with the selected runtime.
#[doc(hidden)]
#[macro_export]
macro_rules! __antlr4_rust_require_codegen_api {
    (1, $generator_version:literal) => {};
    ($requested:literal, $generator_version:literal) => {
        compile_error!(concat!(
            "antlr4-rust generated-code API mismatch: antlr4-rust-gen v",
            $generator_version,
            " emitted generated-code API revision ",
            stringify!($requested),
            ", but the selected antlr-rust-runtime supports revision 1; regenerate this \
             recognizer with a compatible antlr4-rust-gen or select a compatible \
             antlr-rust-runtime dependency"
        ));
    };
}

pub mod atn;
pub mod byte_stream;
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
pub mod tree_pattern;
pub mod vocabulary;
pub mod xpath;

pub use atn::parser::{ParserAtnPrediction, ParserAtnSimulator, ParserAtnSimulatorError};
pub use byte_stream::ByteStream;
pub use char_stream::{CharStream, InputStream, PositionSummary, TextInterval};
pub use dfa::{DfaStateId, DfaTransition, ParserDfa, ParserDfaStateView, ParserDfaStats};
pub use errors::{AntlrError, ConsoleErrorListener, ErrorListener, SyntaxErrorEvent};
pub use generated::{GeneratedLexer, GeneratedParser, GrammarMetadata};
pub use int_stream::{EOF, IntStream, UNKNOWN_SOURCE_NAME};
pub use lexer::{
    BaseLexer, Lexer, LexerCustomAction, LexerLifecycleCtx, LexerMode, LexerPredicate, LexerSemCtx,
    LexerSemIrCtx, LexerSemanticAction, LexerSemanticPredicate, LexerSemantics,
};
pub use parser::{
    BailErrorStrategy, BaseParser, EnterRuleEvent, ExpectedTokenSet, NoSemanticHooks,
    ParseListener, Parser, ParserAction, ParserMemberAction, ParserPredicate, ParserReturnAction,
    ParserRuleArg, ParserRuntimeOptions, ParserSemCtx, ParserSemanticAction,
    ParserSemanticPredicate, ParserSemantics, PredictionMode, RecognitionArenaStats, SemanticHooks,
    UnknownSemanticPolicy, grow_generated_rule_stack,
};
#[cfg(feature = "perf-counters")]
pub use perf::{dump as dump_prediction_perf_counters, reset as reset_prediction_perf_counters};
pub use prediction::{ContextId, PredictionContextStats, SemanticContext};
pub use recognizer::{Recognizer, RecognizerData};
pub use token::{
    DEFAULT_CHANNEL, HIDDEN_CHANNEL, INVALID_TOKEN_TYPE, MAX_TOKEN_OFFSET, TOKEN_EOF, Token,
    TokenChannel, TokenId, TokenIter, TokenSink, TokenSource, TokenSpec, TokenStore,
    TokenStoreError, TokenView,
};
pub use token_stream::CommonTokenStream;
pub use tree::{
    AsRuleNode, ErrorNodeView, FromRuleNode, GeneratedAttrs, MissingChildError, Node, NodeChildren,
    NodeId, NodeKind, ParseTree, ParseTreeDescendants, ParseTreeListener, ParseTreeStats,
    ParseTreeStorage, ParseTreeVisitor, ParseTreeWalker, ParsedFile, ParserRuleContext,
    RuleNodeView, TerminalNodeView,
};
pub use tree_pattern::{
    ParseTreeMatch, ParseTreePattern, ParseTreePatternError, ParseTreePatternMatcher, PatternLexer,
    lex_pattern_chunk,
};
pub use vocabulary::Vocabulary;
pub use xpath::{XPath, XPathError};

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
