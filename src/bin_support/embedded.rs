//! Embedded-action grammar model and `$`-attribute translator.
//!
//! In embedded mode the generator receives a grammar whose actions and
//! predicates are already **real Rust code** — rendered by the conformance
//! harness through `Rust.test.stg`, exactly like every official ANTLR target
//! renders its `.test.stg` — and splices those bodies verbatim into the
//! generated recognizer. The only rewriting applied is ANTLR's own
//! `$attribute` reference translation (the Rust analog of ANTLR's
//! `ActionTranslator`): `$text`, `$ctx`, `$_p`, rule/token/label references,
//! and rule attribute (`args`/`returns`/`locals`) reads and writes.
//!
//! This module consumes the structural grammar model needed for that
//! translation: per-rule attribute declarations, per-alternative element
//! references with labels (for `$label.attr` occurrence resolution), and
//! `@members` bodies split into struct fields, impl items, and module items.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;

use crate::templates::{matching_action_brace, skip_ascii_whitespace};

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
    pub(crate) alts: Vec<AltModel>,
}

impl RuleModel {
    pub(crate) const fn has_attrs(&self) -> bool {
        !self.attrs.is_empty()
    }

    fn attr(&self, name: &str) -> Option<&AttrDecl> {
        self.attrs.iter().find(|attr| attr.name == name)
    }

    /// The alternative whose span contains `offset`, if any.
    fn alt_at(&self, offset: usize) -> Option<&AltModel> {
        self.alts
            .iter()
            .find(|alt| alt.span.0 <= offset && offset < alt.span.1)
    }
}

/// One member field declared through the target's field-with-initializer
/// members convention (`i: i32 = 0;`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemberField {
    pub(crate) name: String,
    pub(crate) ty: String,
    pub(crate) init: String,
}

/// `@members` content split by item kind.
#[derive(Clone, Debug, Default)]
pub(crate) struct MembersModel {
    /// Field declarations lowered onto the recognizer struct.
    pub(crate) fields: Vec<MemberField>,
    /// `fn` items spliced into the recognizer's inherent `impl` block.
    pub(crate) impl_items: Vec<String>,
    /// `struct` / `impl` / attribute-prefixed items emitted at module level
    /// (test listeners, custom nodes, …).
    pub(crate) module_items: Vec<String>,
}

/// Full grammar model for embedded translation.
#[derive(Clone, Debug, Default)]
pub(crate) struct EmbeddedModel {
    /// Parser rules keyed by parser rule index (grammar order).
    pub(crate) rules: Vec<RuleModel>,
    pub(crate) parser_members: MembersModel,
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
fn split_name_colon_type(part: &str) -> Option<(&str, &str)> {
    let colon = part.find(':')?;
    if part[colon..].starts_with("::") {
        return None;
    }
    let name = part[..colon].trim();
    let ty = part[colon + 1..].trim();
    (is_identifier(name) && !ty.is_empty()).then_some((name, ty))
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Splits a members body into field declarations, impl items, and module
/// items.
pub(crate) fn classify_members(body: &str, members: &mut MembersModel) -> io::Result<()> {
    let mut offset = 0;
    let mut pending_attrs = String::new();
    while offset < body.len() {
        offset = skip_ascii_whitespace(body, offset);
        if offset >= body.len() {
            break;
        }
        let rest = &body[offset..];
        if rest.starts_with("//") {
            offset += rest.find('\n').map_or(rest.len(), |nl| nl + 1);
        } else if rest.starts_with('#') {
            // `#[derive(..)]` / `#[allow(..)]` — attaches to the next item.
            let Some(close) = rest.find(']') else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unterminated attribute in @members block",
                ));
            };
            pending_attrs.push_str(&rest[..=close]);
            pending_attrs.push('\n');
            offset += close + 1;
        } else if rest.starts_with("fn ") {
            let item_end = item_end_from(body, offset)?;
            let mut item = std::mem::take(&mut pending_attrs);
            item.push_str(body[offset..item_end].trim());
            members.impl_items.push(item);
            offset = item_end;
        } else if rest.starts_with("struct ")
            || rest.starts_with("impl ")
            || rest.starts_with("use ")
        {
            let item_end = item_end_from(body, offset)?;
            let mut item = std::mem::take(&mut pending_attrs);
            item.push_str(body[offset..item_end].trim());
            members.module_items.push(item);
            offset = item_end;
        } else if let Some(field) = parse_member_field(&body[offset..]) {
            let (field, consumed) = field;
            members.fields.push(field);
            offset += consumed;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported @members item starting at: {}",
                    &rest[..rest.len().min(60)]
                ),
            ));
        }
    }
    Ok(())
}

/// Finds the end of an item: the matching `}` of its first top-level brace
/// block, or the terminating `;` for braceless items (`use x;`).
fn item_end_from(body: &str, offset: usize) -> io::Result<usize> {
    let mut quoted = false;
    let mut escaped = false;
    let mut index = offset;
    while let Some(ch) = body[index..].chars().next() {
        if escaped {
            escaped = false;
            index += ch.len_utf8();
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '{' if !quoted => {
                let close = matching_action_brace(body, index + 1).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unterminated brace in @members item",
                    )
                })?;
                return Ok(close + 1);
            }
            ';' if !quoted => return Ok(index + 1),
            _ => {}
        }
        index += ch.len_utf8();
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "unterminated @members item",
    ))
}

/// Parses one `name: type = init;` member-field declaration; returns the
/// field and the number of bytes consumed.
fn parse_member_field(rest: &str) -> Option<(MemberField, usize)> {
    let semicolon = rest.find(';')?;
    let decl = &rest[..semicolon];
    if decl.contains('{') || decl.contains('(') {
        return None;
    }
    let (name_ty, init) = decl.split_once('=')?;
    let (name, ty) = split_name_colon_type(name_ty.trim())?;
    Some((
        MemberField {
            name: name.to_owned(),
            ty: ty.to_owned(),
            init: init.trim().to_owned(),
        },
        semicolon + 1,
    ))
}

/// Context for translating one action/predicate body.
pub(crate) struct TranslationCtx<'a> {
    pub(crate) model: &'a EmbeddedModel,
    /// Rule containing the body.
    pub(crate) rule_index: usize,
    /// Byte offset of the body inside the grammar source, used to pick the
    /// enclosing alternative for label resolution. `None` for `@init` /
    /// `@after` bodies (labels resolve across all alternatives there).
    pub(crate) body_offset: Option<usize>,
    pub(crate) site: ActionSite,
    /// Token name -> token type, from the compiled recognizer metadata.
    pub(crate) token_types: &'a BTreeMap<String, i32>,
}

impl TranslationCtx<'_> {
    fn rule(&self) -> &RuleModel {
        &self.model.rules[self.rule_index]
    }

    fn rule_index_by_name(&self, name: &str) -> Option<usize> {
        self.model.rules.iter().position(|rule| rule.name == name)
    }

    /// Resolves a label to `(ref, occurrence-among-same-target-in-alt)`.
    ///
    /// Every read `translate_element_read` can emit is a *positional* query over
    /// the flattened CST children — `nth(i)` for a single label, "all children of
    /// this target" for a list label, "the last terminal child" for a block
    /// label. None of those retain which grammar branch built a child, so a
    /// label only resolves when its read provably selects the label's own
    /// element and nothing else. When it cannot, this returns `None` and the
    /// caller fails loudly rather than translating to a silently wrong read.
    ///
    /// The conditions that make a read unfaithful, by label kind:
    ///
    /// * **single** — a preceding ref with inexact cardinality (sibling branches
    ///   of a choice are mutually exclusive and report `min: 0`, so counting
    ///   them indexes past what the parse built), or an *optional* label with a
    ///   following same-target child that slides into its position when absent;
    /// * **list** — any same-target child outside the label, since the read
    ///   cannot exclude it;
    /// * **block** — a following terminal, which would become the `last()` the
    ///   read takes;
    /// * **any kind** — a second declaration of the same label that this one
    ///   read cannot also serve.
    fn resolve_label(&self, label: &str) -> Option<(ElementRef, usize)> {
        let rule = self.rule();
        if let Some(alt) = self.body_offset.and_then(|offset| rule.alt_at(offset)) {
            // A mid-rule action is confined to the branch it is written in, so a
            // sibling branch's children can be excluded.
            return Self::resolve_label_in_alt(alt, label, true);
        }
        // `@after` / `@init` bodies are not scoped to an alternative, so the
        // label may be declared in several. One read has to serve whichever
        // alternative the parse took: taking the first match would emit that
        // alternative's lookup and silently yield a default on the others.
        let mut resolved: Option<(ElementRef, usize)> = None;
        let mut non_declaring = Vec::new();
        for alt in &rule.alts {
            let declares = alt
                .refs
                .iter()
                .any(|element| element.label.as_deref() == Some(label));
            if !declares {
                non_declaring.push(alt);
                continue;
            }
            // An `@after` / `@init` body runs whichever branch the parse took, so
            // a sibling branch's match *can* be the child present when the read
            // executes — sibling exclusion does not apply here.
            let candidate = Self::resolve_label_in_alt(alt, label, false)?;
            if resolved
                .as_ref()
                .is_some_and(|existing| !Self::same_label_read(existing, &candidate))
            {
                return None;
            }
            resolved = Some(candidate);
        }
        let (element, occurrence) = resolved?;
        // An alternative that never declares the label leaves it unset, so the
        // read must come up empty there. It will not if that alternative happens
        // to build a child the read would select anyway (`r : x=A | A`), which
        // would report a value for a label the parse never bound.
        for alt in non_declaring {
            if Self::alt_can_satisfy_read(alt, &element, occurrence) {
                return None;
            }
        }
        Some((element, occurrence))
    }

    /// Whether `alt` builds a child that the read for `element` would select,
    /// even though `alt` does not declare the label.
    fn alt_can_satisfy_read(alt: &AltModel, element: &ElementRef, occurrence: usize) -> bool {
        if element.target.is_empty() {
            // Block reads take the last terminal child, so any terminal matches.
            return alt.refs.iter().any(|candidate| {
                !candidate.token_types.is_empty() && candidate.cardinality.max != Some(0)
            });
        }
        let available = alt
            .refs
            .iter()
            .filter(|candidate| candidate.target == element.target)
            .try_fold(0_usize, |total, candidate| {
                Some(total.saturating_add(candidate.cardinality.max?))
            });
        // A list read selects any same-target child; a positional read needs one
        // at `occurrence`. An unbounded count can always reach either.
        available.is_none_or(|available| available > occurrence)
    }

    /// Whether two per-alternative resolutions lower to the same read, so one
    /// translation can stand for both. The fields compared are exactly those
    /// `translate_element_read` consumes to pick a read: list mode, block mode,
    /// and the target it queries. Two block labels are equivalent regardless of
    /// their token sets, because the block read ignores them.
    fn same_label_read(left: &(ElementRef, usize), right: &(ElementRef, usize)) -> bool {
        if left.0.is_list != right.0.is_list || left.0.is_block != right.0.is_block {
            return false;
        }
        if left.0.is_block && left.0.target.is_empty() && right.0.target.is_empty() {
            return true;
        }
        left.1 == right.1 && left.0.target == right.0.target
    }

    fn resolve_label_in_alt(
        alt: &AltModel,
        label: &str,
        exclude_sibling_branches: bool,
    ) -> Option<(ElementRef, usize)> {
        let declarations = alt
            .refs
            .iter()
            .filter(|element| element.label.as_deref() == Some(label))
            .collect::<Vec<_>>();
        let element = *declarations.first()?;
        let declares_label = |candidate: &ElementRef| candidate.label.as_deref() == Some(label);
        // The generated read queries by rule index or *token type*, so a
        // differently-spelled terminal with the same type is the same child as
        // far as the read is concerned (`A : 'a';` makes `A` and `'a'` aliases).
        let same_target = |candidate: &ElementRef| {
            if candidate.target.is_empty() || candidate.cardinality.max == Some(0) {
                return false;
            }
            if element.token_types.is_empty() || candidate.token_types.is_empty() {
                return candidate.target == element.target;
            }
            candidate
                .token_types
                .iter()
                .any(|token_type| element.token_types.contains(token_type))
        };

        if element.is_list {
            // `translate_element_read` lowers a list label to a per-target child
            // iterator, which needs a rule or token *target*. A list over a token
            // group (`xs+=(A | B)`) has none, so the read would fall through to
            // the scalar block path and emit `.last()…collect()` — code that does
            // not compile. Leave it unresolved instead.
            if element.target.is_empty() {
                return None;
            }
            // A list read yields every child of *one* target, so repeated
            // declarations are the normal idiom (`xs+=e (op xs+=e)+`) only while
            // they all name that same target. `xs+=A xs+=B` would iterate `A`
            // alone and silently drop every `B`.
            if declarations
                .iter()
                .any(|candidate| candidate.target != element.target || !candidate.is_list)
            {
                return None;
            }
            // What the read cannot express is exclusion, so the label resolves
            // only when no same-target element sits *outside* it.
            let exclusive = alt
                .refs
                .iter()
                .all(|candidate| declares_label(candidate) || !same_target(candidate));
            return exclusive.then(|| (element.clone(), 0));
        }
        // A single label read is one positional lookup, so a second declaration
        // (`(x=A | x=B)`) cannot be served: picking the first silently yields an
        // empty value whenever the parse took the other branch.
        if declarations.len() > 1 {
            return None;
        }
        let position = alt
            .refs
            .iter()
            .position(|candidate| std::ptr::eq(candidate, element))?;
        let (before, after) = (&alt.refs[..position], &alt.refs[position + 1..]);
        if element.target.is_empty() {
            // Block/wildcard labels read the most recent terminal child. That is
            // the block's own token exactly when no other terminal has been
            // matched between the block and the action — and because a mid-rule
            // action executes at its own source position, a terminal written
            // *after* the action never interferes. ANTLR's `t=~'x' 'z' {$t.text}`
            // descriptors depend on that (`Sets/ParserNotTokenWithLabel`).
            //
            // `ElementRef` carries no source span, so the action's position
            // relative to these refs is not recoverable here and the read is
            // accepted as-is. A label read *across* an intervening terminal
            // (`((x=(A | B))) C {$x.text}` reads `C`) is therefore still wrong;
            // fixing it needs element spans in the model (issue #233) rather
            // than a guard that would reject the conformance shapes above.
            return Some((element.clone(), 0));
        }

        // A repeated single label (`(x=A)+`) is overwritten on every iteration,
        // so ANTLR exposes the *latest* match. The read here is a fixed
        // `nth(i)`, which would pin the first one; only the accessor path can
        // express `.last()`. Leave it unresolved rather than read the wrong
        // iteration.
        if element.cardinality.is_repeated() {
            return None;
        }
        // Single label: count the same-target children ahead of it, bailing as
        // soon as one contributes an unfixed number.
        let occurrence = before
            .iter()
            .filter(|candidate| candidate.target == element.target)
            .try_fold(0_usize, |total, candidate| {
                let max = candidate.cardinality.max?;
                (candidate.cardinality.min == max).then(|| total.saturating_add(max))
            })?;
        // An optional label is displaced by a *following* same-target child that
        // can slide into its position. Sequential followers can, whether they are
        // mandatory (`x=A? A`) or optional (`(pred x=A)? A?` — the follower may
        // consume the only token). A ref from a sibling branch of the same choice
        // cannot, since no parse contains both, so counting it would reject valid
        // mid-rule actions like `r : (x=A {$x.text} B | A C)`.
        let shadowed_when_absent = element.cardinality.min == 0
            && after.iter().any(|candidate| {
                same_target(candidate)
                    && (!exclude_sibling_branches || candidate.can_coexist_with(element))
            });
        (!shadowed_when_absent).then_some((element.clone(), occurrence))
    }
}

/// Generated attrs struct name for a rule.
pub(crate) fn attrs_struct_name(rule_index: usize) -> String {
    format!("__RuleAttrs{rule_index}")
}

/// Translates every `$…` reference in an embedded body to Rust.
pub(crate) fn translate_body(body: &str, ctx: &TranslationCtx<'_>) -> io::Result<String> {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(dollar) = find_dollar(rest) {
        out.push_str(&rest[..dollar]);
        let after = &rest[dollar + 1..];
        let name_len = after
            .find(|ch: char| ch != '_' && !ch.is_ascii_alphanumeric())
            .unwrap_or(after.len());
        if name_len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("stray $ in embedded action: {body}"),
            ));
        }
        let name = &after[..name_len];
        // Optional `.suffix`.
        let mut consumed = name_len;
        let mut suffix: Option<&str> = None;
        if after[name_len..].starts_with('.') {
            let suffix_text = &after[name_len + 1..];
            let suffix_len = suffix_text
                .find(|ch: char| ch != '_' && !ch.is_ascii_alphanumeric())
                .unwrap_or(suffix_text.len());
            if suffix_len > 0 {
                // Only treat it as an attribute suffix when it is not a
                // method call — `$ctx.to_string_tree(...)` keeps its call.
                let after_suffix = suffix_text[suffix_len..].trim_start();
                let is_call = after_suffix.starts_with('(');
                if !is_call {
                    suffix = Some(&suffix_text[..suffix_len]);
                    consumed = name_len + 1 + suffix_len;
                } else if name == "ctx"
                    && (suffix_text[..suffix_len].ends_with("_children")
                        || suffix_text[..suffix_len].ends_with("_all"))
                    && after_suffix.starts_with("()")
                {
                    // `$ctx.<rule>_children()` (or the legacy `_all()` form) is
                    // an active-context collection read. Consume the empty
                    // parens along with the suffix.
                    suffix = Some(&suffix_text[..suffix_len]);
                    let call_end = suffix_text[suffix_len..]
                        .find(')')
                        .map_or(suffix_len, |close| suffix_len + close + 1);
                    consumed = name_len + 1 + call_end;
                }
            }
        }
        let translated = translate_reference(name, suffix, ctx, body)?;
        out.push_str(&translated);
        rest = &rest[dollar + 1 + consumed..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Finds the next `$` that is outside a string literal.
fn find_dollar(text: &str) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '$' if !quoted => return Some(index),
            _ => {}
        }
    }
    None
}

fn translate_reference(
    name: &str,
    suffix: Option<&str>,
    ctx: &TranslationCtx<'_>,
    body: &str,
) -> io::Result<String> {
    // Special names first.
    match (name, suffix) {
        ("ctx", None) => return Ok("(&__ctx)".to_owned()),
        ("ctx", Some(member)) => return translate_ctx_member(member, ctx, body),
        ("text", None) => return Ok(text_expression(ctx)),
        ("_p", None) => return Ok("__precedence".to_owned()),
        ("parser", None) => return Ok("self".to_owned()),
        ("start", None) => {
            return Ok("__ctx.start(self.base.token_store())".to_owned());
        }
        _ => {}
    }
    let rule = ctx.rule();
    // Labels shadow attrs; attrs shadow rule/token names.
    if let Some((element, occurrence)) = ctx.resolve_label(name) {
        return translate_element_read(&element, occurrence, suffix, ctx, body);
    }
    if rule.attr(name).is_some() {
        let mut expr = format!("__attrs.{}", escape_keyword(name));
        if let Some(suffix) = suffix {
            let _ = write!(expr, ".{suffix}");
        }
        return Ok(expr);
    }
    if let Some(target_rule) = ctx.rule_index_by_name(name) {
        let element = ElementRef {
            label: None,
            target: name.to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality::ONE,
            stable_accessor: false,
            choice_branch: Vec::new(),
        };
        let _ = target_rule;
        return translate_element_read(&element, usize::MAX, suffix, ctx, body);
    }
    if ctx.token_types.contains_key(name) {
        let element = ElementRef {
            label: None,
            target: name.to_owned(),
            token_types: vec![ctx.token_types[name]],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality::ONE,
            stable_accessor: false,
            choice_branch: Vec::new(),
        };
        return translate_element_read(&element, usize::MAX, suffix, ctx, body);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("cannot translate ${name} in embedded action: {body}"),
    ))
}

/// `$text` — text matched so far for the current rule.
fn text_expression(ctx: &TranslationCtx<'_>) -> String {
    match ctx.site {
        ActionSite::Body => {
            "self.base.text_interval(action.start_index(), action.stop_index())".to_owned()
        }
        ActionSite::After | ActionSite::Init => {
            "{ let __stop = self.base.rule_stop_token_index(antlr4_runtime::IntStream::index(self.base.input()), __consumed_eof); self.base.text_interval(__rule_start, __stop) }"
                .to_owned()
        }
    }
}

/// `$ctx.member` — a labeled element read (`$ctx.r`) or a generated child
/// iterator (`$ctx.elseIfStatement_children()`).
fn translate_ctx_member(member: &str, ctx: &TranslationCtx<'_>, body: &str) -> io::Result<String> {
    if let Some((element, occurrence)) = ctx.resolve_label(member) {
        // `$ctx.r` denotes the labeled child's subtree (Java field of the
        // context); translate like `$r.ctx`.
        return translate_element_read(&element, occurrence, Some("ctx"), ctx, body);
    }
    if let Some(rule_name) = member.strip_suffix("_children") {
        if let Some(rule_index) = ctx.rule_index_by_name(rule_name) {
            return Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index})"
            ));
        }
    }
    if let Some(rule_name) = member.strip_suffix("_all") {
        if let Some(rule_index) = ctx.rule_index_by_name(rule_name) {
            return Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).collect::<Vec<_>>()"
            ));
        }
    }
    if ctx.rule().attr(member).is_some() {
        return Ok(format!("__attrs.{}", escape_keyword(member)));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("cannot translate $ctx.{member} in embedded action: {body}"),
    ))
}

/// Reads a rule/token element reference with an optional attribute suffix.
///
/// `occurrence == usize::MAX` means "implicit reference": ANTLR resolves
/// `$e` to the most recent `e` match, i.e. the LAST matching child so far.
fn translate_element_read(
    element: &ElementRef,
    occurrence: usize,
    suffix: Option<&str>,
    ctx: &TranslationCtx<'_>,
    body: &str,
) -> io::Result<String> {
    if element.is_list {
        // `label+=x`: expose the matching children as a lazy Rust iterator.
        if let Some(rule_index) = ctx.rule_index_by_name(&element.target) {
            return match suffix {
                None | Some("ctx") => Ok(format!(
                    "__ctx.child_rule_trees(self.base.parse_tree_storage(), self.base.token_store(), {rule_index})"
                )),
                Some(other) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsupported list-label read .{other} in embedded action: {body}"),
                )),
            };
        }
        if let Some(token_type) = ctx.token_types.get(&element.target) {
            return Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type})"
            ));
        }
    }
    if element.is_block {
        // A labeled `(...)` block over tokens: `$myset.stop` / `$myset.text`
        // read the token the block matched — the most recent terminal child.
        // A bare `$myset` read denotes the Token object itself (Java prints
        // `Token.toString()`), which is the same rendering as start/stop.
        return match suffix {
            None | Some("stop" | "start") => Ok(
                "__ctx.terminal_children(self.base.parse_tree_storage(), self.base.token_store()).last().map(|__t| __t.symbol().to_string()).unwrap_or_default()"
                    .to_owned(),
            ),
            Some("text") => Ok(
                "__ctx.terminal_children(self.base.parse_tree_storage(), self.base.token_store()).last().map(|__t| __t.text().to_owned()).unwrap_or_default()"
                    .to_owned(),
            ),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported block-label read in embedded action: {body}"),
            )),
        };
    }
    if let Some(rule_index) = ctx.rule_index_by_name(&element.target) {
        let pick = if occurrence == usize::MAX {
            "last()".to_owned()
        } else {
            format!("nth({occurrence})")
        };
        return match suffix {
            Some("ctx") | None => Ok(format!(
                "__ctx.child_rule_trees(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.expect(\"labeled rule child\")"
            )),
            Some("text") => Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.map(|__c| __c.text()).unwrap_or_default()"
            )),
            Some("start") => Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.and_then(|__c| __c.start()).map(|__t| __t.to_string()).unwrap_or_default()"
            )),
            Some("stop") => Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.and_then(|__c| __c.stop()).map(|__t| __t.to_string()).unwrap_or_default()"
            )),
            Some(attr) => {
                let target_rule = &ctx.model.rules[rule_index];
                let Some(decl) = target_rule.attr(attr) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "rule {} has no attribute {attr} (embedded action: {body})",
                            element.target
                        ),
                    ));
                };
                let attrs_struct = attrs_struct_name(rule_index);
                let field = escape_keyword(&decl.name);
                Ok(format!(
                    "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.and_then(|__c| __c.generated_attrs::<{attrs_struct}>()).map(|__a| __a.{field}.clone()).unwrap_or_default()"
                ))
            }
        };
    }
    if let Some(token_type) = ctx.token_types.get(&element.target) {
        let pick = if occurrence == usize::MAX {
            "last()".to_owned()
        } else {
            format!("nth({occurrence})")
        };
        return match suffix {
            Some("text") => Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type}).{pick}.map(|__t| __t.text().to_owned()).unwrap_or_default()"
            )),
            Some("int") => Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type}).{pick}.map(|__t| __t.text().parse::<i32>().unwrap_or_default()).unwrap_or_default()"
            )),
            Some("line") => Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type}).{pick}.map(|__t| __t.symbol().line()).unwrap_or_default()"
            )),
            None | Some("stop" | "start") => Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type}).{pick}.map(|__t| __t.symbol().to_string()).unwrap_or_default()"
            )),
            Some(other) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported token attribute .{other} on ${} (embedded action: {body})",
                    element.target
                ),
            )),
        };
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "cannot resolve element ${} in embedded action: {body}",
            element.target
        ),
    ))
}

/// Escapes attribute names that collide with Rust keywords (`$return`).
pub(crate) fn escape_keyword(name: &str) -> String {
    match name {
        "return" | "type" | "match" | "loop" | "move" | "ref" | "self" | "super" | "box"
        | "const" | "continue" | "crate" | "else" | "enum" | "extern" | "fn" | "for" | "if"
        | "impl" | "in" | "let" | "mod" | "mut" | "pub" | "static" | "struct" | "trait"
        | "unsafe" | "use" | "where" | "while" => format!("r#{name}"),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::{ScopeDecl, parse_scope_decls};

    fn model(rules: Vec<RuleModel>) -> EmbeddedModel {
        EmbeddedModel {
            rules,
            parser_members: MembersModel::default(),
        }
    }

    fn rule(name: &str) -> RuleModel {
        RuleModel {
            name: name.to_owned(),
            ..RuleModel::default()
        }
    }

    fn tokens(pairs: &[(&str, i32)]) -> BTreeMap<String, i32> {
        pairs
            .iter()
            .map(|(name, ty)| ((*name).to_owned(), *ty))
            .collect()
    }

    #[test]
    fn maps_attribute_types_for_generated_rust() {
        assert_eq!(map_attr_type("int"), "i32");
        assert_eq!(map_attr_type("Integer"), "i32");
        assert_eq!(map_attr_type("boolean"), "bool");
        assert_eq!(map_attr_type("List<Integer>"), "Vec<i32>");
        assert_eq!(map_attr_type("List < List<Integer> >"), "Vec<Vec<i32>>");
        assert_eq!(map_attr_type("std::string::String"), "std::string::String");
    }

    mod upstream_scope_parsing {
        use super::*;

        const CASES: &[(&str, &str)] = &[
            ("", ""),
            (" ", ""),
            ("int i", "i:int"),
            ("int[] i, int j[]", "i:int[], j:int []"),
            ("Map<A,B>[] i, int j[]", "i:Map<A,B>[], j:int []"),
            ("Map<A,List<B>>[] i", "i:Map<A,List<B>>[]"),
            (
                "int i = 34+a[3], int j[] = new int[34]",
                "i:int=34+a[3], j:int []=new int[34]",
            ),
            ("char *[3] foo = {1,2,3}", "foo:char *[3]={1,2,3}"),
            ("String[] headers", "headers:String[]"),
            ("std::vector<std::string> x", "x:std::vector<std::string>"),
            ("i", "i"),
            ("i,j", "i, j"),
            ("i\t,j, k", "i, j, k"),
            ("x: int", "x:int"),
            ("x :int", "x:int"),
            ("x:int", "x:int"),
            ("x:int=3", "x:int=3"),
            (
                "r:Rectangle=Rectangle(fromLength: 6, fromBreadth: 12)",
                "r:Rectangle=Rectangle(fromLength: 6, fromBreadth: 12)",
            ),
            ("p:pointer to int", "p:pointer to int"),
            ("a: array[3] of int", "a:array[3] of int"),
            ("a \t:\tfunc(array[3] of int)", "a:func(array[3] of int)"),
            ("x:int, y:float", "x:int, y:float"),
            (
                "x:T?, f:func(array[3] of int), y:int",
                "x:T?, f:func(array[3] of int), y:int",
            ),
            ("float64 x = 3", "x:float64=3"),
            ("map[string]int x", "x:map[string]int"),
        ];

        #[test]
        fn argument_declarations_match_java() {
            for &(input, expected) in CASES {
                let actual = parse_scope_decls(input)
                    .into_iter()
                    .map(render)
                    .collect::<Vec<_>>()
                    .join(", ");
                assert_eq!(actual, expected, "input {input:?}");
            }
        }

        fn render(declaration: ScopeDecl) -> String {
            let ty = declaration
                .ty
                .map_or_else(String::new, |ty| format!(":{ty}"));
            let initializer = declaration
                .initializer
                .map_or_else(String::new, |initializer| format!("={initializer}"));
            format!("{}{ty}{initializer}", declaration.name)
        }
    }

    #[test]
    fn translates_attr_and_rule_reads() {
        let mut expression = rule("e");
        expression.attrs.push(AttrDecl {
            name: "v".to_owned(),
            ty: "i32".to_owned(),
        });
        let m = model(vec![rule("s"), expression]);
        let toks = tokens(&[("INT", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 1,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let translated = translate_body("$v = $INT.int;", &ctx).expect("translates");
        assert!(translated.starts_with("__attrs.v = "), "{translated}");
        assert!(
            translated.contains(
                "child_tokens(self.base.parse_tree_storage(), self.base.token_store(), 1)"
            ),
            "{translated}"
        );

        let parent_ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let read = translate_body("writeln!(self.output(), \"{}\", $e.v);", &parent_ctx)
            .expect("translates");
        assert!(read.contains("generated_attrs::<__RuleAttrs1>"), "{read}");
    }

    #[test]
    fn resolves_structural_labels_within_the_owning_alternative() {
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                ElementRef {
                    label: Some("left".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality::ONE,
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
                ElementRef {
                    label: Some("right".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality::ONE,
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
            ],
            children: BTreeMap::from([(
                "e".to_owned(),
                ChildCardinality {
                    min: 2,
                    max: Some(2),
                },
            )]),
            leading_target: Some("e".to_owned()),
        });
        let mut expression = rule("e");
        expression.attrs.push(AttrDecl {
            name: "v".to_owned(),
            ty: "i32".to_owned(),
        });
        let m = model(vec![statement, expression]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$right.v", &ctx).expect("translates");
        assert!(translated.contains(".nth(1)"), "{translated}");
        assert!(
            translated.contains("generated_attrs::<__RuleAttrs1>"),
            "{translated}"
        );
    }

    /// A label preceded by a same-target ref from a *sibling* choice branch has
    /// no fixed CST position: `r : (e | x=e) {$x...}` builds one `e` child, so
    /// counting the flattened refs would emit `nth(1)` and silently read an
    /// element the parse never produced. Such a label must stay unresolved and
    /// surface as a translation error.
    #[test]
    fn inexact_preceding_refs_leave_labels_unresolved_instead_of_misindexing() {
        let mut statement = rule("s");
        let branch_ref = |label: Option<&str>| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: false,
            // Sibling branches of a choice are mutually exclusive.
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![branch_ref(None), branch_ref(Some("x"))],
            children: BTreeMap::from([(
                "e".to_owned(),
                ChildCardinality {
                    min: 1,
                    max: Some(1),
                },
            )]),
            leading_target: Some("e".to_owned()),
        });
        let m = model(vec![statement, rule("e")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$x.text", &ctx).expect_err("must not translate");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("cannot translate $x"), "{error}");
    }

    /// An *optional* label with a following same-target child has no fixed
    /// position either: in `r : ({false}? x=A)? A {$x...}` the mandatory `A`
    /// slides into `nth(0)` whenever the optional group is skipped, so the
    /// action would receive a value for an unset label.
    #[test]
    fn optional_labels_shadowed_by_a_following_child_stay_unresolved() {
        let mut statement = rule("s");
        let token_ref = |label: Option<&str>, min| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality { min, max: Some(1) },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            // `x=A?` then a mandatory `A`.
            refs: vec![token_ref(Some("x"), 0), token_ref(None, 1)],
            children: BTreeMap::new(),
            leading_target: Some("A".to_owned()),
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$x.text", &ctx).expect_err("must not translate");
        assert!(error.to_string().contains("cannot translate $x"), "{error}");
    }

    /// Token groups carry no target, so they must not share one occurrence
    /// bucket: an optional disjoint group ahead of a labeled group
    /// (`r : (A | B)? x=(C | D) {$x...}`) must not poison it. Block-label reads
    /// take the last terminal child and never consult the index at all.
    #[test]
    fn disjoint_token_groups_do_not_poison_a_later_block_label() {
        let mut statement = rule("s");
        let group_ref = |label: Option<&str>, token_types: Vec<i32>, min| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: String::new(),
            token_types,
            is_block: true,
            is_list: false,
            cardinality: ChildCardinality { min, max: Some(1) },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                group_ref(None, vec![1, 2], 0),
                group_ref(Some("x"), vec![3, 4], 1),
            ],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1), ("B", 2), ("C", 3), ("D", 4)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$x.text", &ctx).expect("translates");
        assert!(translated.contains("terminal_children"), "{translated}");
        assert!(translated.contains(".last()"), "{translated}");
    }

    /// A list label ahead of a same-target single label still contributes
    /// children, so it must go through the occurrence accounting rather than be
    /// skipped: `r : xs+=e name=e` puts one `e` before `name` (exact, countable
    /// → `nth(1)`), while `r : xs+=e+ name=e` puts an unbounded run there and
    /// leaves no fixed position at all.
    #[test]
    fn list_refs_ahead_of_a_single_label_are_counted_then_poison_when_unbounded() {
        let list_ref = |max| ElementRef {
            label: Some("xs".to_owned()),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: true,
            cardinality: ChildCardinality { min: 1, max },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let single_ref = ElementRef {
            label: Some("name".to_owned()),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let translate = |max| {
            let mut statement = rule("s");
            statement.alts.push(AltModel {
                label: None,
                span: (10, 20),
                refs: vec![list_ref(max), single_ref.clone()],
                children: BTreeMap::new(),
                leading_target: Some("e".to_owned()),
            });
            let m = model(vec![statement, rule("e")]);
            let toks = tokens(&[]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                body_offset: Some(15),
                site: ActionSite::Body,
                token_types: &toks,
            };
            translate_body("$name.text", &ctx).map_err(|error| error.to_string())
        };

        // Exactly one preceding `e`: the position is known.
        let exact = translate(Some(1)).expect("exact list count still resolves");
        assert!(exact.contains(".nth(1)"), "{exact}");

        // Unbounded run of `e` ahead of the label: no fixed index exists.
        let error = translate(None).expect_err("unbounded list must not resolve");
        assert!(error.contains("cannot translate $name"), "{error}");
    }

    /// A list read yields *every* same-target child, so it can only stand for
    /// the label when no same-target element sits outside it. In the `mixed`
    /// shape (`name=e ... errors+=e`) a `$errors` read would fold in `name`'s
    /// child, so the label must not resolve.
    #[test]
    fn list_labels_sharing_a_target_with_another_label_stay_unresolved() {
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                ElementRef {
                    label: Some("name".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality {
                        min: 1,
                        max: Some(1),
                    },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
                ElementRef {
                    label: Some("errors".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: true,
                    cardinality: ChildCardinality { min: 0, max: None },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
            ],
            children: BTreeMap::new(),
            leading_target: Some("e".to_owned()),
        });
        let m = model(vec![statement, rule("e")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$errors", &ctx).expect_err("must not translate");
        assert!(
            error.to_string().contains("cannot translate $errors"),
            "{error}"
        );

        // `name` is still resolvable: it precedes the list, so its own index is
        // fixed at 0 and the list contributes nothing ahead of it.
        let name = translate_body("$name.text", &ctx).expect("translates");
        assert!(name.contains(".nth(0)"), "{name}");
    }

    /// A block label reads the most recent terminal child, which is correct for
    /// ANTLR's `t=~'x' 'z' {$t.text}` conformance shapes because a mid-rule
    /// action runs at its own source position. `ElementRef` carries no span, so
    /// that ordering is not recoverable here and the read is accepted as-is —
    /// this test pins the resulting behaviour, including the known limitation
    /// that a read across an intervening terminal picks the later token
    /// (issue #233).
    #[test]
    fn block_labels_resolve_to_the_most_recent_terminal_read() {
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                ElementRef {
                    label: Some("x".to_owned()),
                    target: String::new(),
                    token_types: vec![1, 2],
                    is_block: true,
                    is_list: false,
                    cardinality: ChildCardinality {
                        min: 1,
                        max: Some(1),
                    },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
                ElementRef {
                    label: None,
                    target: "C".to_owned(),
                    token_types: vec![3],
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality {
                        min: 1,
                        max: Some(1),
                    },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
            ],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1), ("B", 2), ("C", 3)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$x.text", &ctx).expect("translates");
        assert!(translated.contains("terminal_children"), "{translated}");
        assert!(translated.contains(".last()"), "{translated}");
    }

    /// An alternative that does not declare the label leaves it unset, so the
    /// read has to come up empty there. `r : x=A | A` breaks that: the second
    /// alternative builds an `A` the read would select, reporting a value for a
    /// label the parse never bound. Conversely `r : x=A | B` is fine.
    #[test]
    fn unscoped_reads_reject_alternatives_that_would_satisfy_them_unbound() {
        let token_ref = |label: Option<&str>, target: &str, token_type| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: target.to_owned(),
            token_types: vec![token_type],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let translate = |second: ElementRef| {
            let mut statement = rule("s");
            for (index, refs) in [vec![token_ref(Some("x"), "A", 1)], vec![second]]
                .into_iter()
                .enumerate()
            {
                statement.alts.push(AltModel {
                    label: None,
                    span: (index * 10, index * 10 + 10),
                    refs,
                    children: BTreeMap::new(),
                    leading_target: None,
                });
            }
            let m = model(vec![statement]);
            let toks = tokens(&[("A", 1), ("B", 2)]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                body_offset: None,
                site: ActionSite::After,
                token_types: &toks,
            };
            translate_body("$x.text", &ctx).map_err(|error| error.to_string())
        };

        // `x=A | A`: the unlabeled `A` satisfies the read with `x` unset.
        let error = translate(token_ref(None, "A", 1))
            .expect_err("an unbound label must not read another alternative's child");
        assert!(error.contains("cannot translate $x"), "{error}");

        // `x=A | B`: nothing in the second alternative can be mistaken for `x`.
        let translated =
            translate(token_ref(None, "B", 2)).expect("disjoint alternative keeps the read");
        assert!(translated.contains(".nth(0)"), "{translated}");
    }

    /// A list read iterates one target, so every declaration of the label must
    /// name that target: `xs+=A xs+=B` would iterate `A` and drop every `B`.
    /// Equivalent *block* labels across alternatives (`x=(A | B) | x=(C | D)`)
    /// conversely stay resolvable, because the block read ignores token sets.
    #[test]
    fn list_declarations_must_share_a_target_while_block_reads_ignore_token_sets() {
        let list_ref = |target: &str, token_type| ElementRef {
            label: Some("xs".to_owned()),
            target: target.to_owned(),
            token_types: vec![token_type],
            is_block: false,
            is_list: true,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![list_ref("A", 1), list_ref("B", 2)],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1), ("B", 2)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };
        let error = translate_body("$xs", &ctx).expect_err("mixed list targets must not resolve");
        assert!(
            error.to_string().contains("cannot translate $xs"),
            "{error}"
        );

        // Two block labels over different token sets lower to the same read.
        let block_ref = |token_types: Vec<i32>| ElementRef {
            label: Some("x".to_owned()),
            target: String::new(),
            token_types,
            is_block: true,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let mut choice = rule("s");
        for (index, refs) in [vec![block_ref(vec![1, 2])], vec![block_ref(vec![3, 4])]]
            .into_iter()
            .enumerate()
        {
            choice.alts.push(AltModel {
                label: None,
                span: (index * 10, index * 10 + 10),
                refs,
                children: BTreeMap::new(),
                leading_target: None,
            });
        }
        let m = model(vec![choice]);
        let toks = tokens(&[("A", 1), ("B", 2), ("C", 3), ("D", 4)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::After,
            token_types: &toks,
        };
        let translated = translate_body("$x.text", &ctx).expect("equivalent block reads resolve");
        assert!(translated.contains("terminal_children"), "{translated}");
    }

    /// Reads that `translate_element_read` cannot express must stay unresolved
    /// rather than fall through to a different read: a list over a *token group*
    /// (`xs+=(A | B)`) has no target to iterate and would emit
    /// `.last()…collect()` — Rust that does not compile — and a *repeated* single
    /// label (`(x=A)+`) is overwritten each iteration, so a fixed `nth(0)` pins
    /// the first match where ANTLR exposes the latest.
    #[test]
    fn reads_the_translator_cannot_express_stay_unresolved() {
        let translate = |element: ElementRef, read: &str| {
            let mut statement = rule("s");
            statement.alts.push(AltModel {
                label: None,
                span: (10, 20),
                refs: vec![element],
                children: BTreeMap::new(),
                leading_target: None,
            });
            let m = model(vec![statement]);
            let toks = tokens(&[("A", 1), ("B", 2)]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                body_offset: Some(15),
                site: ActionSite::Body,
                token_types: &toks,
            };
            translate_body(read, &ctx).map_err(|error| error.to_string())
        };

        let group_list = ElementRef {
            label: Some("xs".to_owned()),
            target: String::new(),
            token_types: vec![1, 2],
            is_block: true,
            is_list: true,
            cardinality: ChildCardinality { min: 1, max: None },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let error = translate(group_list, "$xs").expect_err("no target to iterate");
        assert!(error.contains("cannot translate $xs"), "{error}");

        let repeated_single = ElementRef {
            label: Some("x".to_owned()),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality { min: 1, max: None },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let error = translate(repeated_single, "$x.text").expect_err("no last-occurrence read");
        assert!(error.contains("cannot translate $x"), "{error}");
    }

    /// Mutual exclusion needs the *whole* choice ancestry, not just the innermost
    /// choice. In `((x=e | f) | e)` the label and the trailing `e` are separated
    /// by the outer choice; keeping only the inner tag would make them look
    /// independent and reject a valid action.
    #[test]
    fn nested_choices_keep_their_outer_branch_ancestry() {
        let rule_ref = |label: Option<&str>, branches: Vec<(usize, usize)>| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: branches,
        };
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                // Outer choice 1 branch 0, then inner choice 2 branch 0.
                rule_ref(Some("x"), vec![(1, 0), (2, 0)]),
                // Outer choice 1 branch 1 — excluded by the *outer* choice alone.
                rule_ref(None, vec![(1, 1)]),
            ],
            children: BTreeMap::new(),
            leading_target: Some("e".to_owned()),
        });
        let m = model(vec![statement, rule("e")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$x.text", &ctx).expect("outer exclusion still applies");
        assert!(translated.contains(".nth(0)"), "{translated}");
    }

    /// Sibling exclusion is valid only for a *mid-rule* action, which is confined
    /// to the branch it is written in. An `@after` body runs whichever branch the
    /// parse took, so `r @after {$x.text} : (x=A | A) EOF;` must decline — the
    /// unlabeled branch's `A` is present when the read executes.
    #[test]
    fn unscoped_bodies_treat_sibling_matches_as_hazards() {
        let branch_ref = |label: Option<&str>, branch| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: vec![(3, branch)],
        };
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![branch_ref(Some("x"), 0), branch_ref(None, 1)],
            children: BTreeMap::new(),
            leading_target: Some("A".to_owned()),
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1)]);
        let after = TranslationCtx {
            model: &m,
            rule_index: 0,
            // `@after`: unscoped, so any branch may have run.
            body_offset: None,
            site: ActionSite::After,
            token_types: &toks,
        };
        let error = translate_body("$x.text", &after).expect_err("sibling match is a hazard here");
        assert!(error.to_string().contains("cannot translate $x"), "{error}");

        // The same refs read from a mid-rule action inside the labeled branch do
        // resolve, since that action cannot run on the sibling branch.
        let body = TranslationCtx {
            body_offset: Some(15),
            site: ActionSite::Body,
            ..after
        };
        let translated =
            translate_body("$x.text", &body).expect("mid-rule action excludes sibling");
        assert!(translated.contains(".nth(0)"), "{translated}");
    }

    /// A token label's read queries by token *type*, so a differently-spelled
    /// terminal with the same type is the same child: with `A : 'a';`,
    /// `r : (xs+=A)? 'a' {$xs}` must decline because the mandatory `'a'` would be
    /// iterated as an `xs` element.
    #[test]
    fn token_labels_compare_types_not_source_spelling() {
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                ElementRef {
                    label: Some("xs".to_owned()),
                    target: "A".to_owned(),
                    token_types: vec![1],
                    is_block: false,
                    is_list: true,
                    cardinality: ChildCardinality {
                        min: 0,
                        max: Some(1),
                    },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
                ElementRef {
                    label: None,
                    // Literal spelling differs; the token type is identical.
                    target: "'a'".to_owned(),
                    token_types: vec![1],
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality {
                        min: 1,
                        max: Some(1),
                    },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
            ],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$xs", &ctx).expect_err("aliased terminal is the same child");
        assert!(
            error.to_string().contains("cannot translate $xs"),
            "{error}"
        );
    }

    /// A same-target ref in a *sibling* choice branch never coexists with the
    /// label, so it cannot displace an optional one: `r : (x=A {$x.text} B | A C)`
    /// must still translate. Only a certainly-matched following child shadows.
    #[test]
    fn sibling_branch_children_do_not_shadow_an_optional_label() {
        let mut statement = rule("s");
        // Two branches of one choice: same choice id, different branch index.
        let branch_ref = |label: Option<&str>, branch| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            // Mutually exclusive branches both report `min: 0`.
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: vec![(7, branch)],
        };
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![branch_ref(Some("x"), 0), branch_ref(None, 1)],
            children: BTreeMap::new(),
            leading_target: Some("A".to_owned()),
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$x.text", &ctx).expect("translates");
        assert!(translated.contains(".nth(0)"), "{translated}");

        // The sequential counterpart — `(pred x=A)? A?`, both on the rule's own
        // path — *is* a hazard: the follower may consume the only token while the
        // label is unset. Cardinality alone cannot tell these two apart, which is
        // what `choice_branch` exists for.
        let mut sequential = rule("s");
        let sequential_ref = |label: Option<&str>| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        sequential.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![sequential_ref(Some("x")), sequential_ref(None)],
            children: BTreeMap::new(),
            leading_target: Some("A".to_owned()),
        });
        let m = model(vec![sequential]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };
        let error = translate_body("$x.text", &ctx).expect_err("sequential follower shadows");
        assert!(error.to_string().contains("cannot translate $x"), "{error}");
    }

    /// A list label repeated within one alternative is the ordinary
    /// comma-separated idiom (`xs+=e (op xs+=e)+`) — every declaration feeds the
    /// same iteration, so repeats must not be mistaken for a conflict. This is
    /// the shape of ANTLR's `ParserExec/ListLabelsOnRuleRefStartOfAlt`
    /// descriptor, read from `@after` across alternatives that declare it plus
    /// one that does not.
    #[test]
    fn repeated_list_declarations_across_alternatives_still_resolve() {
        let list_ref = || ElementRef {
            label: Some("args".to_owned()),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: true,
            cardinality: ChildCardinality { min: 1, max: None },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let token_ref = ElementRef {
            label: None,
            target: "ID".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let mut statement = rule("s");
        for (index, refs) in [
            // `args+=e (AND args+=e)+` — two declarations, one iteration.
            vec![list_ref(), list_ref()],
            // An alternative that never mentions the label at all.
            vec![token_ref],
        ]
        .into_iter()
        .enumerate()
        {
            statement.alts.push(AltModel {
                label: None,
                span: (index * 10, index * 10 + 10),
                refs,
                children: BTreeMap::new(),
                leading_target: None,
            });
        }
        let m = model(vec![statement, rule("e")]);
        let toks = tokens(&[("ID", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::After,
            token_types: &toks,
        };

        let translated = translate_body("$args", &ctx).expect("list label resolves");
        assert!(translated.contains("child_rule_trees"), "{translated}");
    }

    /// `@after` / `@init` bodies are not scoped to an alternative, so one read
    /// has to serve whichever alternative the parse took. `r : x=A | x=B` with an
    /// `@after` read of `$x` would emit the `A` lookup and yield a default on the
    /// `B` branch, while `r : x=A B | x=A C` resolves identically in both.
    #[test]
    fn unscoped_bodies_reject_labels_that_resolve_differently_per_alternative() {
        let token_ref = |label: Option<&str>, target: &str, token_type| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: target.to_owned(),
            token_types: vec![token_type],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        let translate = |second: Vec<ElementRef>| {
            let mut statement = rule("s");
            for (index, refs) in [vec![token_ref(Some("x"), "A", 1)], second]
                .into_iter()
                .enumerate()
            {
                statement.alts.push(AltModel {
                    label: None,
                    span: (index * 10, index * 10 + 10),
                    refs,
                    children: BTreeMap::new(),
                    leading_target: None,
                });
            }
            let m = model(vec![statement]);
            let toks = tokens(&[("A", 1), ("B", 2), ("C", 3)]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                // `@after`: no offset, so no single owning alternative.
                body_offset: None,
                site: ActionSite::After,
                token_types: &toks,
            };
            translate_body("$x.text", &ctx).map_err(|error| error.to_string())
        };

        // `x=A | x=B`: the two alternatives need different token lookups.
        let error = translate(vec![token_ref(Some("x"), "B", 2)])
            .expect_err("conflicting per-alternative reads must not translate");
        assert!(error.contains("cannot translate $x"), "{error}");

        // `x=A B | x=A C`: both resolve to the same `A` lookup at occurrence 0.
        let agreed = translate(vec![token_ref(Some("x"), "A", 1), token_ref(None, "C", 3)])
            .expect("agreeing per-alternative reads still translate");
        assert!(agreed.contains(".nth(0)"), "{agreed}");
    }

    /// One label declared over disjoint targets (`r : (x=A | x=B)`) cannot be
    /// served by a single positional read: picking the first declaration yields
    /// an empty value whenever the parse took the other branch.
    #[test]
    fn labels_repeated_over_disjoint_targets_stay_unresolved() {
        let mut statement = rule("s");
        let token_ref = |target: &str, token_type| ElementRef {
            label: Some("x".to_owned()),
            target: target.to_owned(),
            token_types: vec![token_type],
            is_block: false,
            is_list: false,
            // Mutually exclusive branches.
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
        };
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![token_ref("A", 1), token_ref("B", 2)],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1), ("B", 2)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$x.text", &ctx).expect_err("must not translate");
        assert!(error.to_string().contains("cannot translate $x"), "{error}");
    }

    #[test]
    fn translates_ctx_and_text() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[("ID", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let text = translate_body("$text", &ctx).expect("translates");
        assert!(
            text.contains("text_interval(action.start_index()"),
            "{text}"
        );
        let tree = translate_body("$ctx.to_string_tree(Some(self))", &ctx).expect("translates");
        assert_eq!(tree, "(&__ctx).to_string_tree(Some(self))");
    }

    #[test]
    fn translates_active_context_child_iterators() {
        let m = model(vec![rule("s"), rule("elseIfStatement")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated =
            translate_body("$ctx.elseIfStatement_children()", &ctx).expect("translates");
        assert_eq!(
            translated,
            "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), 1)"
        );
    }

    #[test]
    fn translates_list_labels_as_lazy_iterators() {
        let mut start = rule("s");
        start.alts.push(AltModel {
            label: None,
            span: (0, 10),
            refs: vec![
                ElementRef {
                    label: Some("args".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: true,
                    cardinality: ChildCardinality { min: 1, max: None },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
                ElementRef {
                    label: Some("ids".to_owned()),
                    target: "ID".to_owned(),
                    token_types: vec![1],
                    is_block: false,
                    is_list: true,
                    cardinality: ChildCardinality { min: 1, max: None },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                },
            ],
            children: BTreeMap::new(),
            leading_target: Some("e".to_owned()),
        });
        let m = model(vec![start, rule("e")]);
        let toks = tokens(&[("ID", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::After,
            token_types: &toks,
        };

        let rules = translate_body("let _: Vec<_> = $args.collect();", &ctx).expect("rule list");
        assert_eq!(rules.matches(".collect()").count(), 1, "{rules}");
        assert!(rules.contains("__ctx.child_rule_trees("), "{rules}");

        let tokens = translate_body("let _: Vec<_> = $ids.collect();", &ctx).expect("token list");
        assert_eq!(tokens.matches(".collect()").count(), 1, "{tokens}");
        assert!(tokens.contains("__ctx.child_tokens("), "{tokens}");
    }

    #[test]
    fn classifies_member_blocks() {
        let body = "i: i32 = 0;\n\
            #[allow(non_snake_case)]\n\
            fn Property(&self) -> bool {\n    true\n}\n\
            struct LeafListener;\n";
        let mut members = MembersModel::default();
        classify_members(body, &mut members).expect("members classify");

        assert_eq!(members.fields.len(), 1);
        assert_eq!(members.fields[0].name, "i");
        assert_eq!(members.fields[0].init, "0");
        assert_eq!(members.impl_items.len(), 1);
        assert!(members.impl_items[0].contains("fn Property"));
        assert_eq!(members.module_items.len(), 1);
    }

    #[test]
    fn dollar_inside_strings_is_left_alone() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[("ID", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let body = "writeln!(self.output(), \"{}\", \"$notaref\");";
        assert_eq!(translate_body(body, &ctx).expect("translates"), body);
    }
}
