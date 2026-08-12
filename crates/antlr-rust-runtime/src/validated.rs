// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
//! Shared validated-parse surface for generated recognizers.
//!
//! Every generated parser exposes a strict parsing mode that rejects syntax
//! errors, recovered error nodes, and missing generated required children
//! before handing out a tree whose typed accessors are infallible. The types
//! backing that surface are grammar-agnostic — all grammar-specific
//! information they carry (context and child names) arrives as data from the
//! generated `validate_tree_structure` — so they are defined once here and
//! aliased by generated modules.
//!
//! [`ValidatedTree`] and [`ValidatedRuleNode`] are branded with a `Grammar`
//! type parameter. Each generated module instantiates them with its
//! module-local `ValidatedTreeContext` marker
//! (`pub type TomlValidatedTree = antlr4_runtime::ValidatedTree<ValidatedTreeContext>;`),
//! so trees and nodes of different grammars remain distinct types and
//! [`ValidatedRuleNode::downcast_ref`] cannot resolve a node against another
//! grammar's contexts, whose rule indexes and context kinds are grammar-local
//! numbers. [`ValidationError`] is deliberately unbranded: a binary linking
//! several generated parsers handles one error type and compiles one copy of
//! its `Display`/`Error`/`From` machinery.

use std::marker::PhantomData;

use thiserror::Error;

use crate::errors::AntlrError;
use crate::tree::{MissingChildError, Node, ParsedFile, RuleNodeView};

/// A completed, syntax-clean parse tree whose generated child cardinalities
/// have been structurally validated.
///
/// Constructed only by a generated parser's `validate()` /
/// `parse_validated()` conveniences after `validate_tree_structure` proved
/// the required-child invariants, so [`ValidatedTree::tree`] and the
/// validated context accessors never observe a violated invariant. `Grammar`
/// is the generated module's `ValidatedTreeContext` marker; it keeps the
/// validated trees of different grammars nominally distinct.
pub struct ValidatedTree<Grammar> {
    parsed: ParsedFile,
    grammar: PhantomData<Grammar>,
}

impl<Grammar> ValidatedTree<Grammar> {
    /// Wraps a parse whose structure was already validated.
    ///
    /// This is a doc-hidden contract for generated code, not a sealed
    /// boundary: it is technically callable from any crate, and wrapping a
    /// parse that did not pass the grammar's `validate_tree_structure` makes
    /// later infallible validated accessors panic via `unreachable!`. (In
    /// generated-code API revisions 9 and earlier the equivalent constructor
    /// was private to the generated module.)
    #[doc(hidden)]
    #[must_use]
    pub const fn __new(parsed: ParsedFile) -> Self {
        Self {
            parsed,
            grammar: PhantomData,
        }
    }

    /// Returns the validated entry-rule root.
    #[must_use]
    pub fn tree(&self) -> ValidatedRuleNode<'_, Grammar> {
        let Some(rule) = self.parsed.tree().as_rule() else {
            unreachable!("validated parse root was checked as a rule node")
        };
        ValidatedRuleNode {
            node: rule,
            grammar: PhantomData,
        }
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

impl<Grammar> std::fmt::Debug for ValidatedTree<Grammar> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedTree")
            .field("parsed", &self.parsed)
            .finish()
    }
}

/// A rule node borrowed from a [`ValidatedTree`] with the same `Grammar`
/// brand.
pub struct ValidatedRuleNode<'a, Grammar> {
    node: RuleNodeView<'a>,
    grammar: PhantomData<Grammar>,
}

impl<'a, Grammar> ValidatedRuleNode<'a, Grammar> {
    /// Wraps a rule node that belongs to an already-validated tree.
    ///
    /// This is a doc-hidden contract for generated code (validated walkers
    /// and visitor bridges), not a sealed boundary: it is technically
    /// callable from any crate, and minting a validated node over an
    /// unvalidated tree makes later infallible validated accessors panic via
    /// `unreachable!`. (In generated-code API revisions 9 and earlier the
    /// node's field was private to the generated module.)
    #[doc(hidden)]
    #[must_use]
    pub const fn __new(node: RuleNodeView<'a>) -> Self {
        Self {
            node,
            grammar: PhantomData,
        }
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

    /// Views this node as one of the grammar's validated context types.
    ///
    /// The `Grammar` brand ties candidates to the grammar that produced the
    /// node, so contexts of other generated parsers do not satisfy the bound.
    #[must_use]
    pub fn downcast_ref<T: FromValidatedRuleNode<'a, Grammar = Grammar>>(self) -> Option<T> {
        T::from_validated_rule_node(self)
    }
}

impl<Grammar> std::fmt::Debug for ValidatedRuleNode<'_, Grammar> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedRuleNode")
            .field("node", &self.node)
            .finish()
    }
}

impl<Grammar> Clone for ValidatedRuleNode<'_, Grammar> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Grammar> Copy for ValidatedRuleNode<'_, Grammar> {}

/// Constructs a generated validated context from a validated rule node of
/// the same grammar.
pub trait FromValidatedRuleNode<'a>: Sized {
    /// The generated module's `ValidatedTreeContext` marker.
    type Grammar;

    fn from_validated_rule_node(node: ValidatedRuleNode<'a, Self::Grammar>) -> Option<Self>;
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

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
mod tests {
    use std::error::Error as _;

    use super::*;

    fn every_variant() -> Vec<ValidationError> {
        vec![
            ValidationError::Recognition(AntlrError::LexerError {
                line: 3,
                column: 7,
                message: "token recognition error at: '#'".to_owned(),
            }),
            ValidationError::SyntaxErrors {
                lexer: 1,
                parser: 2,
            },
            ValidationError::MissingChild(MissingChildError::new("StartContext", "atom")),
            ValidationError::InvalidChildCount {
                context: "StartContext",
                child: "atom",
                minimum: 2,
                actual: 1,
            },
            ValidationError::RecoveredErrorNode {
                line: 4,
                column: 9,
                text: "<missing ';'>".to_owned(),
            },
            ValidationError::InvalidRoot,
            ValidationError::UnknownRule { rule_index: 41 },
        ]
    }

    #[test]
    fn validation_error_display_texts() {
        let rendered = every_variant()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!("validation_error_display_texts", rendered);
    }

    #[test]
    fn validation_error_sources() {
        for error in every_variant() {
            let expects_source = matches!(
                error,
                ValidationError::Recognition(_) | ValidationError::MissingChild(_)
            );
            assert_eq!(
                error.source().is_some(),
                expects_source,
                "source() mismatch for {error:?}"
            );
        }
    }

    #[test]
    fn validation_error_from_conversions() {
        let recognition = AntlrError::LexerError {
            line: 1,
            column: 0,
            message: "boom".to_owned(),
        };
        assert_eq!(
            ValidationError::from(recognition.clone()),
            ValidationError::Recognition(recognition)
        );

        let missing = MissingChildError::new("StartContext", "atom");
        assert_eq!(
            ValidationError::from(missing),
            ValidationError::MissingChild(missing)
        );
    }

    #[test]
    fn require_min_count_accepts_satisfied_minimums() {
        assert_eq!(require_min_count(2, 2, "StartContext", "atom"), Ok(()));
        assert_eq!(require_min_count(3, 0, "StartContext", "atom"), Ok(()));
    }

    #[test]
    fn require_min_count_reports_the_violated_site() {
        assert_eq!(
            require_min_count(1, 2, "StartContext", "atom"),
            Err(ValidationError::InvalidChildCount {
                context: "StartContext",
                child: "atom",
                minimum: 2,
                actual: 1,
            })
        );
    }
}
