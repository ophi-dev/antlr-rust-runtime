pub(crate) mod action;
pub(crate) mod atn;
mod char_support;
pub(crate) mod compiler;
pub(crate) mod diagnostic;
mod escape_sequence;
pub(crate) mod integration;
mod left_recursion;
pub(crate) mod loader;
pub(crate) mod model;
mod mutual_recursion;
pub(crate) mod provenance;
pub(crate) mod rule_reachability;
mod semantics;
pub(crate) mod source;
mod syntax;
pub(crate) mod transform;
mod unicode;
mod unicode_escape;
pub(crate) mod validation;

pub(crate) mod frontend {
    pub(crate) use antlr_rust_g4_parser::{
        Cst, FrontendError, SourceFile, SourceId, SourceSpan, SyntaxId, SyntaxNode, SyntaxNodeKind,
        SyntaxToken, parse_input_stream, parse_input_stream_recovering,
    };

    #[cfg(test)]
    pub(crate) use antlr_rust_g4_parser::parse_source;
}

pub(crate) mod generated {
    pub(crate) use antlr_rust_g4_parser::generated::antlr_v4_parser;
}

pub(crate) use syntax::parse_loader_unit;

#[cfg(test)]
pub(crate) use semantics::{
    ParsedAttributeDeclaration as ScopeDecl, parse_attribute_declarations as parse_scope_decls,
};
