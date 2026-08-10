//! Shared validated-parse surface for generated recognizers.
//!
//! Every generated parser exposes a strict parsing mode that rejects syntax
//! errors, recovered error nodes, and missing generated required children
//! before handing out a tree whose typed accessors are infallible. The types
//! backing that surface are grammar-agnostic — all grammar-specific
//! information they carry (context and child names) arrives as data from the
//! generated `validate_tree_structure` — so they are defined once here and
//! aliased by generated modules as `<Grammar>ValidatedTree` /
//! `<Grammar>ValidationError`.
//!
//! Because the generated names are plain type aliases, the validated-parse
//! types of different grammars are deliberately interchangeable: a binary
//! linking several generated parsers handles one [`ValidationError`] type and
//! compiles one copy of its `Display`/`Error`/`From` machinery.

use thiserror::Error;

use crate::errors::AntlrError;
use crate::tree::{MissingChildError, Node, ParsedFile, RuleNodeView};

/// A completed, syntax-clean parse tree whose generated child cardinalities
/// have been structurally validated.
///
/// Constructed only by a generated parser's `validate()` /
/// `parse_validated()` conveniences after `validate_tree_structure` proved
/// the required-child invariants, so [`ValidatedTree::tree`] and the
/// validated context accessors never observe a violated invariant.
#[derive(Debug)]
pub struct ValidatedTree {
    parsed: ParsedFile,
}

impl ValidatedTree {
    /// Wraps a parse whose structure was already validated.
    ///
    /// Only generated code may construct the validated-tree type boundary;
    /// calling this with an unvalidated parse breaks the surface's
    /// infallibility guarantees.
    #[doc(hidden)]
    #[must_use]
    pub const fn __new(parsed: ParsedFile) -> Self {
        Self { parsed }
    }

    /// Returns the validated entry-rule root.
    #[must_use]
    pub fn tree(&self) -> ValidatedRuleNode<'_> {
        let Some(rule) = self.parsed.tree().as_rule() else {
            unreachable!("validated parse root was checked as a rule node")
        };
        ValidatedRuleNode { node: rule }
    }

    /// Borrows the underlying recovery-oriented parsed file.
    #[must_use]
    pub const fn parsed_file(&self) -> &ParsedFile {
        &self.parsed
    }

    /// Drops the validation type boundary and returns the underlying parsed
    /// file.
    #[must_use]
    pub fn into_parsed_file(self) -> ParsedFile {
        self.parsed
    }
}

/// A rule node borrowed from a [`ValidatedTree`].
#[derive(Clone, Copy, Debug)]
pub struct ValidatedRuleNode<'a> {
    node: RuleNodeView<'a>,
}

impl<'a> ValidatedRuleNode<'a> {
    /// Wraps a rule node that belongs to an already-validated tree.
    ///
    /// Only generated code (validated walkers and visitor bridges) may mint
    /// validated rule nodes.
    #[doc(hidden)]
    #[must_use]
    pub const fn __new(node: RuleNodeView<'a>) -> Self {
        Self { node }
    }

    #[must_use]
    pub const fn rule_node(self) -> RuleNodeView<'a> {
        self.node
    }

    #[must_use]
    pub const fn node(self) -> Node<'a> {
        self.node.node()
    }

    #[must_use]
    pub fn rule_index(self) -> usize {
        self.node.rule_index()
    }

    #[must_use]
    pub fn text(self) -> String {
        self.node.text()
    }

    #[must_use]
    pub fn downcast_ref<T: FromValidatedRuleNode<'a>>(self) -> Option<T> {
        T::from_validated_rule_node(self)
    }
}

/// Constructs a generated validated context from a validated rule node.
pub trait FromValidatedRuleNode<'a>: Sized {
    fn from_validated_rule_node(node: ValidatedRuleNode<'a>) -> Option<Self>;
}

/// Failure to recognize or validate a strict generated parse.
///
/// Grammar-specific detail (context and child names) is carried as variant
/// data supplied by the generated `validate_tree_structure`, so one error
/// type serves every generated parser.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum ValidationError {
    #[error("parse failed: {0}")]
    Recognition(#[from] AntlrError),
    #[error("parse produced {lexer} lexer and {parser} parser syntax errors")]
    SyntaxErrors { lexer: usize, parser: usize },
    #[error("{0}")]
    MissingChild(#[from] MissingChildError),
    #[error(
        "required child {child} occurs {actual} times in {context}; expected at least {minimum}"
    )]
    InvalidChildCount {
        context: &'static str,
        child: &'static str,
        minimum: usize,
        actual: usize,
    },
    #[error("recovered error node at {line}:{column}: {text}")]
    RecoveredErrorNode {
        line: usize,
        column: usize,
        text: String,
    },
    #[error("validated parse root is not a rule node")]
    InvalidRoot,
    #[error("parse tree contains unknown rule index {rule_index}")]
    UnknownRule { rule_index: usize },
}

/// Checks one generated repeated-child minimum-cardinality invariant.
///
/// Generated `validate_tree_structure` implementations call this once per
/// required list child; `context` and `child` are grammar data supplied by
/// the generated caller.
///
/// # Errors
///
/// Returns [`ValidationError::InvalidChildCount`] when `actual < minimum`.
pub const fn require_min_count(
    actual: usize,
    minimum: usize,
    context: &'static str,
    child: &'static str,
) -> Result<(), ValidationError> {
    if actual < minimum {
        return Err(ValidationError::InvalidChildCount {
            context,
            child,
            minimum,
            actual,
        });
    }
    Ok(())
}
