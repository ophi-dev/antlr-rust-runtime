//! ANTLR v4 grammar compiler and Rust source generator.

mod artifact;
mod builder;
mod cli;
mod cli_report;
mod config;
mod driver;
pub(crate) mod embedded;
mod error;
#[path = "pipeline.rs"]
mod generator;
#[allow(dead_code)]
pub(crate) mod grammar;
mod json;
mod lexer;
mod optimization;
mod parser;
mod rust_output;
mod rust_support;
mod semantics;
mod structural;
mod test_rig;
mod testrig_cli;

pub use artifact::Generation;
pub use builder::{ActionMode, Builder, UnknownSemanticPolicy};
pub use error::{Diagnostic, Error, ErrorKind, Severity};

/// Version of this generator package.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Generated-source/runtime API revision emitted by this generator.
pub const CODEGEN_API: u32 = antlr4_runtime::__ANTLR4_RUST_CODEGEN_API;

#[cfg(test)]
pub(crate) use generator::ParserCodegenData;
#[cfg(test)]
pub(crate) use parser::render_parser;
#[cfg(test)]
pub(crate) use rust_output::rust_string;
#[cfg(test)]
pub(crate) use semantics::collect_structural_grammar_options;
#[cfg(test)]
pub(crate) use structural::{structural_embedded_model, structural_predicates};

/// Runs the `antlr4-rust-gen` command using the process arguments and streams.
///
/// The public [`Builder`] API is the preferred integration point for build
/// scripts. This function exists for the package's binary adapter.
#[doc(hidden)]
pub fn run_cli() -> miette::Result<()> {
    cli_report::install_handler_from_env();
    cli::run_cli()
}

/// Runs the `antlr4-rust-testrig` command using the process arguments.
#[doc(hidden)]
pub fn run_testrig_cli() -> miette::Result<std::process::ExitCode> {
    cli_report::install_handler_from_env();
    testrig_cli::run_cli()
}
