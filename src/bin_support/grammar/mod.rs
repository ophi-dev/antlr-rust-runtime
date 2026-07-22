mod action;
pub(crate) mod atn;
mod char_support;
pub(crate) mod compiler;
pub(crate) mod diagnostic;
mod escape_sequence;
pub(crate) mod frontend;
mod generated {
    pub(super) mod antlr_v4_lexer;
    pub(super) mod antlr_v4_parser;
}
mod left_recursion;
mod lexer_adaptor;
pub(crate) mod loader;
pub(crate) mod model;
pub(crate) mod provenance;
mod semantics;
pub(crate) mod source;
mod syntax;
mod transform;
mod transform_analysis;
mod unicode;
mod unicode_escape;

#[cfg(test)]
pub(crate) use semantics::{
    ParsedAttributeDeclaration as ScopeDecl, parse_attribute_declarations as parse_scope_decls,
};

#[cfg(test)]
mod ported_tests;
