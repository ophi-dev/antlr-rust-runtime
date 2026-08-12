// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
//! Checked-in `ANTLRv4` recognizers and recovered source-syntax facade.
//!
//! This crate is a lockstep implementation dependency of
//! `antlr-rust-codegen`. Its exported facade is intentionally narrow and is
//! not a standalone compatibility promise.

mod frontend;
#[doc(hidden)]
pub mod generated {
    #[doc(hidden)]
    pub mod antlr_v4_lexer;
    #[doc(hidden)]
    pub mod antlr_v4_parser;
}
mod lexer_adaptor;

#[doc(hidden)]
pub use frontend::{
    Cst, CstDescendants, DiagnosticStage, FrontendError, RecoveredSource, SourceFile, SourceId,
    SourceSpan, SyntaxDiagnostic, SyntaxId, SyntaxNode, SyntaxNodeKind, SyntaxToken,
    parse_input_stream, parse_input_stream_recovering, parse_source, parse_source_recovering,
};
