// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
/// Grammar-specific parser API and embedded-body bindings.
///
/// This model is built from structural grammar metadata only. Decision and IR
/// internals are deliberately absent so public surface construction cannot
/// accidentally depend on prediction routing.
#[derive(Debug, Default)]
pub(crate) struct ParserSurfaceBindings {
    /// ATN action state -> translated inline Rust statements.
    pub(crate) inline_actions: BTreeMap<usize, String>,
    /// (rule, pred) -> (translated expr, optional `<fail=...>` message).
    pub(crate) predicates: BTreeMap<(usize, usize), (String, Option<String>)>,
    /// rule -> translated `@init` statements (run at rule entry).
    pub(crate) init_entry: BTreeMap<usize, String>,
    /// rule -> translated `@after` statements (run before `finish_rule`).
    pub(crate) after: BTreeMap<usize, String>,
    /// rule -> whether the rule declares args/returns/locals.
    pub(crate) rule_has_attrs: Vec<bool>,
    /// Rule-transition source state -> translated caller arg expression.
    pub(crate) call_args: BTreeMap<usize, String>,
    /// rule -> escaped name of its first declared arg, if any.
    pub(crate) rule_arg0: Vec<Option<String>>,
    /// Rendered `__RuleAttrsN` struct definitions.
    pub(crate) attrs_structs: String,
    /// Member fields lowered onto the parser struct.
    pub(crate) struct_fields: String,
    pub(crate) field_inits: String,
    /// `@members` fn items + generated facades for the parser impl block.
    pub(crate) impl_items: String,
    /// `@members` structs/impls and generated support types.
    pub(crate) module_items: String,
    /// `@header` items for the top of the module, before generated imports.
    pub(crate) header_items: String,
    /// `@definitions` items for module scope, after the `@members` module
    /// items.
    pub(crate) definitions_items: String,
    /// rule -> (binding name, translated authored `catch` handler body).
    pub(crate) catch_clauses: BTreeMap<usize, (String, String)>,
    /// rule -> translated authored `finally` body.
    pub(crate) finally_bodies: BTreeMap<usize, String>,
}

/// Mode-selected parser surface stage artifact.
#[derive(Debug, Default)]
pub(crate) struct ParserSurfaceModel {
    embedded: Option<ParserSurfaceBindings>,
    structural: Option<ParserSurfaceBindings>,
}

impl ParserSurfaceModel {
    pub(crate) const fn embedded(bindings: ParserSurfaceBindings) -> Self {
        Self {
            embedded: Some(bindings),
            structural: None,
        }
    }

    pub(crate) const fn structural(bindings: ParserSurfaceBindings) -> Self {
        Self {
            embedded: None,
            structural: Some(bindings),
        }
    }

    pub(crate) const fn embedded_bindings(&self) -> Option<&ParserSurfaceBindings> {
        self.embedded.as_ref()
    }

    pub(crate) const fn structural_bindings(&self) -> Option<&ParserSurfaceBindings> {
        self.structural.as_ref()
    }

    pub(crate) fn bindings(&self) -> Option<&ParserSurfaceBindings> {
        self.embedded.as_ref().or(self.structural.as_ref())
    }
}
