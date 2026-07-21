mod atn;
mod compiler;
mod diagnostic;
pub(crate) mod frontend;
mod generated {
    pub(super) mod antlr_v4_lexer;
    pub(super) mod antlr_v4_parser;
}
mod left_recursion;
mod lexer_adaptor;
mod loader;
mod model;
mod provenance;
mod semantics;
mod source;
mod syntax;
mod transform;
mod transform_analysis;
mod unicode;

#[cfg(test)]
mod ported_tests;
