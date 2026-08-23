// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use super::*;

/// One `name: type` attribute declared in a rule's `[...]` args clause or
/// `returns [...]` / `locals [...]` clauses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttrDecl {
    pub(crate) name: String,
    /// Rust type after mapping (Java `int` -> `i32`, `boolean` -> `bool`, …).
    pub(crate) ty: String,
}

/// Number of children with one grammar target that an alternative can emit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ChildCardinality {
    pub(crate) min: usize,
    /// `None` denotes an unbounded maximum.
    pub(crate) max: Option<usize>,
}

impl ChildCardinality {
    pub(crate) const ZERO: Self = Self {
        min: 0,
        max: Some(0),
    };
    pub(crate) const ONE: Self = Self {
        min: 1,
        max: Some(1),
    };

    pub(crate) const fn is_required_single(self) -> bool {
        self.min == 1 && matches!(self.max, Some(1))
    }

    pub(crate) const fn is_repeated(self) -> bool {
        match self.max {
            Some(max) => max > 1,
            None => true,
        }
    }
}

/// Suffix marking a label that was temporarily renamed while resolving a *sibling*
/// declaration of the same label in isolation. Grammar labels are identifiers, so
/// this cannot collide with a real one.
pub(crate) const SIBLING_DECLARATION_SUFFIX: &str = " (sibling declaration)";

/// One enclosing block: its byte extent, and whether its own quantifier relaxes
/// the lower bound of the elements inside it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GroupSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// `true` for `(…)?` / `(…)*` — the group may contribute nothing.
    pub(crate) optional: bool,
    /// `true` for `(…)*` / `(…)+` — the group may run more than once, so the number
    /// of children it contributes is not fixed even when it is known to have run.
    pub(crate) repeated: bool,
}

/// One element reference inside an alternative: a rule ref, token ref, or a
/// labeled sub-block, in source order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ElementRef {
    pub(crate) label: Option<String>,
    /// Referenced rule or token spelling; empty for token sets and wildcards.
    pub(crate) target: String,
    /// Token types matched by this element. Empty for rule references.
    pub(crate) token_types: Vec<i32>,
    pub(crate) is_block: bool,
    /// `label+=ref` list label.
    pub(crate) is_list: bool,
    /// Cardinality of this element after its direct EBNF suffix.
    pub(crate) cardinality: ChildCardinality,
    /// Whether source-order occurrence lookup is unambiguous for a generated
    /// label accessor. Single-alternative EBNF groups preserve it; choices opt
    /// out because their flattened CST children do not retain the chosen path.
    pub(crate) stable_accessor: bool,
    /// `(choice id, alternative index)` for every enclosing *multi*-alternative
    /// block, outermost first. Two refs that share a choice id but sit in
    /// different alternatives of it are mutually exclusive: no parse contains
    /// both. Empty means the ref is on the rule's own sequential path.
    ///
    /// The whole ancestry is kept, not just the innermost choice: for
    /// `((x=e | f) | e)` the labeled `x` and the trailing `e` are separated by
    /// the *outer* choice, which an innermost-only tag would lose.
    pub(crate) choice_branch: Vec<(usize, usize)>,
    /// Alternative count of each choice named in `choice_branch`, in the same
    /// order. Recorded at collection time because an *empty* alternative emits no
    /// ref at all, so the branch count cannot be recovered from the refs alone —
    /// `(a=A | )` would otherwise look like a one-branch choice that always
    /// yields an `A`.
    pub(crate) choice_arity: Vec<usize>,
    /// Byte span of each enclosing choice *block*, in the same order as
    /// `choice_branch`. An action lies inside a branch only when its offset falls
    /// within the block's span — refs alone cannot tell `(A | xs+=A) {…}` (action
    /// after the group) from `(A x=A {…} | B)` (action inside it), since both put
    /// branch refs on either side of the action.
    pub(crate) choice_spans: Vec<(usize, usize)>,
    /// Cardinality with every *enclosing* quantifier and choice split treated as
    /// satisfied — only this element's own EBNF suffix applies. An action inside
    /// `(A x=A {…})?` runs only when the group matched, so on that path the
    /// preceding `A` is exactly-once even though both `cardinality` and
    /// `branch_local_cardinality` report `0..1` from the group's `?`.
    pub(crate) group_local_cardinality: ChildCardinality,
    /// Byte span and lower-bound-relaxing flag of *every* enclosing block, including
    /// single-alternative groups that `choice_spans` omits. The flag says whether
    /// that group's own quantifier is what made this element optional (`(…)?` or
    /// `(…)*`), so a group known taken can have *its* contribution removed without
    /// disturbing the others: in `((q) x=q {…})?` the outer `?` is satisfied when the
    /// action runs while the inner `(q)` is mandatory and already closed.
    pub(crate) group_spans: Vec<GroupSpan>,
    /// Byte span of each enclosing choice *alternative* (the branch itself), in the
    /// same order as `choice_branch`. Lets an action be attributed to the branch
    /// whose text contains it, including a branch holding only actions or
    /// predicates — such a branch emits no `ElementRef` at all, so ref spans alone
    /// would attribute the action to a neighbouring branch.
    pub(crate) branch_spans: Vec<(usize, usize)>,
    /// Whether no terminal can precede this element on its own parse path. Only
    /// then do a block read (which indexes every terminal child) and a token read
    /// (which indexes only same-type children) provably agree, so this is what
    /// makes a mixed-mode merge sound — occurrence zero alone is not enough, since
    /// a token-mode zero can still sit at a non-zero terminal position.
    pub(crate) leading_terminal: bool,
    /// Byte span of the element in the grammar source, when known. A mid-rule
    /// action executes at *its* source position, so only refs that start before
    /// the action's offset have been matched when its body runs.
    pub(crate) span: Option<(usize, usize)>,
    /// Cardinality this element would have if every enclosing choice took the
    /// branch containing it — i.e. with only the *quantifiers* applied, not the
    /// `min: 0` that `choice_branch` membership imposes.
    ///
    /// `(a=A | b=A) x=A` and `(a=A | b=A)? x=A` give their branch refs the same
    /// `cardinality` (`0..1`), yet the first choice always yields one `A` and the
    /// second may yield none. Only this field separates them.
    pub(crate) branch_local_cardinality: ChildCardinality,
}

impl ElementRef {
    /// Whether `self` and `other` can both appear in one parse. They cannot when
    /// any choice encloses both in *different* alternatives.
    pub(crate) fn can_coexist_with(&self, other: &Self) -> bool {
        !self.choice_branch.iter().any(|(choice, branch)| {
            other
                .choice_branch
                .iter()
                .any(|(other_choice, other_branch)| {
                    choice == other_choice && branch != other_branch
                })
        })
    }

    /// Drops the choices `discard` selects, keeping every parallel choice array
    /// aligned with `choice_branch`.
    ///
    /// `choice_arity`, `choice_spans`, and `branch_spans` are indexed *by position*
    /// in `choice_branch`, so removing an entry must remove the same position from
    /// each. Truncating to the surviving length instead keeps the outermost
    /// entries — which is wrong whenever an *outer* choice is the one dropped:
    /// `((q | q | b) x=q | c)` would then read the outer choice's arity of 2
    /// against the surviving inner choice, mistaking a three-way choice for an
    /// exhaustive two-way one.
    pub(crate) fn retain_choices(&mut self, mut keep: impl FnMut(usize) -> bool) {
        let mask = self
            .choice_branch
            .iter()
            .map(|&(choice, _)| keep(choice))
            .collect::<Vec<_>>();
        fn retain_by_mask<T>(list: &mut Vec<T>, mask: &[bool]) {
            let mut index = 0;
            list.retain(|_| {
                let kept = mask.get(index).copied().unwrap_or(true);
                index += 1;
                kept
            });
        }
        retain_by_mask(&mut self.choice_branch, &mask);
        retain_by_mask(&mut self.choice_arity, &mask);
        retain_by_mask(&mut self.choice_spans, &mask);
        retain_by_mask(&mut self.branch_spans, &mask);
    }
}

/// One top-level alternative of a parser rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AltModel {
    /// `# altLabel`, if present.
    pub(crate) label: Option<String>,
    /// Byte span of the alternative inside the grammar source.
    pub(crate) span: (usize, usize),
    pub(crate) refs: Vec<ElementRef>,
    /// Aggregate child cardinality by referenced rule or symbolic token.
    pub(crate) children: BTreeMap<String, ChildCardinality>,
    /// Target of the first syntactic element when it is a bare (possibly
    /// labeled) rule/token reference; `None` for a leading literal, set,
    /// block, or action. ANTLR's left-recursion transformer only treats an
    /// alternative as an operator alternative when the recursion is the
    /// first element, so `'(' e ')'` stays primary even though its first
    /// *reference* is the rule itself.
    pub(crate) leading_target: Option<String>,
}

impl AltModel {
    /// Whether this is a left-recursive operator alternative of `rule_name`.
    pub(crate) fn is_lr_operator(&self, rule_name: &str) -> bool {
        self.leading_target.as_deref() == Some(rule_name)
    }
}

/// Structural model of one compiled parser rule.
#[derive(Clone, Debug, Default)]
pub(crate) struct RuleModel {
    pub(crate) name: String,
    /// Args, returns and locals, flattened (names are unique per rule in the
    /// runtime testsuite corpus).
    pub(crate) attrs: Vec<AttrDecl>,
    /// Names declared specifically by the rule's `locals [...]` clause.
    pub(crate) local_names: Vec<String>,
    /// Names of the attrs that come from the `[...]` args clause, in order —
    /// call sites initialize these positionally (`a[2]`).
    pub(crate) arg_names: Vec<String>,
    pub(crate) init_body: Option<String>,
    pub(crate) after_body: Option<String>,
    /// Supported authored `catch [...] { ... }` clause: the Rust binding name
    /// derived from the argument, plus the handler body. `None` when the rule
    /// has no catch clause or the clause is unsupported (the section
    /// inventory reports the latter).
    pub(crate) catch_clause: Option<(String, String)>,
    /// Authored `finally { ... }` body (non-empty).
    pub(crate) finally_body: Option<String>,
    pub(crate) alts: Vec<AltModel>,
}

impl RuleModel {
    pub(crate) const fn has_attrs(&self) -> bool {
        !self.attrs.is_empty()
    }

    pub(crate) fn attr(&self, name: &str) -> Option<&AttrDecl> {
        self.attrs.iter().find(|attr| attr.name == name)
    }

    /// The alternative whose span contains `offset`, if any.
    pub(crate) fn alt_at(&self, offset: usize) -> Option<&AltModel> {
        self.alts
            .iter()
            .find(|alt| alt.span.0 <= offset && offset < alt.span.1)
    }
}

/// One member field declared through the target's field-with-initializer
/// members convention (`i: i32 = 0;`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemberField {
    pub(crate) source: SourceId,
    pub(crate) attributes: String,
    pub(crate) name: String,
    pub(crate) ty: String,
    pub(crate) init: String,
}

/// One source-owned item from a grammar-level `@members` block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemberItem {
    pub(crate) source: SourceId,
    pub(crate) body: String,
}

/// `@members` content split by item kind.
#[derive(Clone, Debug, Default)]
pub(crate) struct MembersModel {
    /// Field declarations lowered onto the recognizer struct.
    pub(crate) fields: Vec<MemberField>,
    /// `fn` items spliced into the recognizer's inherent `impl` block.
    pub(crate) impl_items: Vec<MemberItem>,
    /// `struct` / `impl` / attribute-prefixed items emitted at module level
    /// (test listeners, custom nodes, …).
    pub(crate) module_items: Vec<MemberItem>,
    /// Names introduced into the generated module's value/type namespaces.
    pub(crate) module_symbols: BTreeSet<String>,
    /// Activation predicates for declarations that occupy the value namespace.
    /// An empty predicate list denotes an unconditional declaration. Braced
    /// structs are type-only, while tuple/unit constructors occupy both the
    /// type and value namespaces.
    pub(crate) module_symbol_cfgs: BTreeMap<String, Vec<Vec<String>>>,
    /// Activation predicates for imports whose target namespace is resolved by
    /// Rust. Rendering keeps a token-value fallback alongside the import so a
    /// type-only target does not hide the compatibility alias.
    pub(crate) module_import_cfgs: BTreeMap<String, Vec<Vec<String>>>,
}

/// Full grammar model for embedded translation.
#[derive(Clone, Debug, Default)]
pub(crate) struct EmbeddedModel {
    /// Parser rules keyed by parser rule index (grammar order).
    pub(crate) rules: Vec<RuleModel>,
    pub(crate) parser_members: MembersModel,
    /// `@header` bodies emitted verbatim (after token-alias translation) at
    /// the top of the generated module, before generated imports.
    pub(crate) header_items: Vec<MemberItem>,
    /// `@definitions` bodies emitted at module scope after the embedded
    /// `@members` module items.
    pub(crate) definitions_items: Vec<MemberItem>,
}

/// Where an action body executes, which changes how `$text` translates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionSite {
    /// Mid-rule action: an `action` local minted by
    /// `parser_action_at_current` is in scope.
    Body,
    /// Rule `@after`: runs after the body, before `finish_rule`.
    After,
    /// Rule `@init`: runs at rule entry.
    Init,
}

/// How a translated parser body is embedded into generated Rust.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParserBodyKind {
    Action,
    Predicate,
}

pub(crate) const ANTLR4RUST_TOKEN_ALIAS_MODULE: &str = "__antlr4rust_token_aliases";
pub(crate) const ANTLR4RUST_CONTEXT_WRAPPER: &str = "__Antlr4RustContext";
pub(crate) const ANTLR4RUST_INPUT_FACADE: &str = "__Antlr4RustInput";
pub(crate) const ANTLR4RUST_TOKEN_VIEW: &str = "__Antlr4RustTokenView";

#[derive(Clone, Copy)]
pub(crate) struct Antlr4RustNames<'a> {
    pub(crate) token_alias_module: &'a str,
    pub(crate) context_wrapper: &'a str,
    pub(crate) input_facade: &'a str,
}

/// Maps a grammar attribute type (possibly Java-flavored, possibly already
/// Rust from the rendered templates) onto the Rust type the generated attrs
/// struct uses.
pub(crate) fn map_attr_type(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(inner) = raw
        .strip_prefix("List")
        .map(str::trim_start)
        .and_then(|rest| rest.strip_prefix('<'))
        .and_then(|inner| inner.strip_suffix('>'))
        .map(str::trim)
        .filter(|inner| !inner.is_empty())
    {
        return format!("Vec<{}>", map_attr_type(inner));
    }
    match raw {
        "int" | "Integer" => "i32".to_owned(),
        "boolean" => "bool".to_owned(),
        "float" | "double" => "f64".to_owned(),
        other => other.to_owned(),
    }
}

/// Splits `name: type`, tolerating generic types containing `:` (`Vec<T>` has
/// none today, but `::` paths do appear, e.g. `std::string::String`).
pub(crate) fn split_name_colon_type(part: &str) -> Option<(&str, &str)> {
    let colon = part.find(':')?;
    if part[colon..].starts_with("::") {
        return None;
    }
    let name = part[..colon].trim();
    let ty = part[colon + 1..].trim();
    (is_identifier(name) && !ty.is_empty()).then_some((name, ty))
}

pub(crate) fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
