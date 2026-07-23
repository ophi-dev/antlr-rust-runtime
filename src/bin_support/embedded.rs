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
    match raw.trim() {
        "int" => "i32".to_owned(),
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
    fn resolve_label(&self, label: &str) -> Option<(ElementRef, usize)> {
        let rule = self.rule();
        let alts: Vec<&AltModel> = self
            .body_offset
            .and_then(|offset| rule.alt_at(offset))
            .map_or_else(|| rule.alts.iter().collect(), |alt| vec![alt]);
        for alt in alts {
            let mut occurrence_by_target: BTreeMap<&str, usize> = BTreeMap::new();
            for element in &alt.refs {
                let occurrence = occurrence_by_target
                    .entry(element.target.as_str())
                    .or_insert(0);
                let current = *occurrence;
                *occurrence += 1;
                if element.label.as_deref() == Some(label) {
                    return Some((element.clone(), current));
                }
            }
        }
        None
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
        assert_eq!(map_attr_type("boolean"), "bool");
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
                },
                ElementRef {
                    label: Some("right".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality::ONE,
                    stable_accessor: true,
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
                },
                ElementRef {
                    label: Some("ids".to_owned()),
                    target: "ID".to_owned(),
                    token_types: vec![1],
                    is_block: false,
                    is_list: true,
                    cardinality: ChildCardinality { min: 1, max: None },
                    stable_accessor: true,
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
