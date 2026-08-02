pub(crate) mod action;
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
mod mutual_recursion;
pub(crate) mod precedence_ladder;
pub(crate) mod provenance;
pub(crate) mod prune_unreachable;
pub(crate) mod rule_reachability;
mod semantics;
pub(crate) mod source;
mod syntax;
pub(crate) mod transform;
mod transform_analysis;
mod unicode;
mod unicode_escape;

pub(crate) use syntax::parse_loader_unit;

#[cfg(test)]
pub(crate) use semantics::{
    ParsedAttributeDeclaration as ScopeDecl, parse_attribute_declarations as parse_scope_decls,
};

#[cfg(test)]
mod ported_tests;
