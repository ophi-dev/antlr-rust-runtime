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
//! This module owns the grammar source model needed for that translation:
//! per-rule attribute declarations, per-alternative element references with
//! labels (for `$label.attr` occurrence resolution), and `@members` blocks
//! split into struct fields, impl items, and module items.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;

use crate::templates::{
    GrammarSourceCursor, matching_action_brace, next_parser_action_block, skip_ascii_whitespace,
};

/// One `name: type` attribute declared in a rule's `[...]` args clause or
/// `returns [...]` / `locals [...]` clauses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttrDecl {
    pub(crate) name: String,
    /// Rust type after mapping (Java `int` -> `i32`, `boolean` -> `bool`, …).
    pub(crate) ty: String,
}

/// One element reference inside an alternative: a rule ref, token ref, or a
/// labeled sub-block, in source order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ElementRef {
    pub(crate) label: Option<String>,
    /// Referenced rule or token name; empty for a labeled `(...)` block,
    /// `~set`, or string literal.
    pub(crate) target: String,
    pub(crate) is_block: bool,
    /// `label+=ref` list label.
    pub(crate) is_list: bool,
}

/// One top-level alternative of a parser rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AltModel {
    /// `# altLabel`, if present.
    pub(crate) label: Option<String>,
    /// Byte span of the alternative inside the grammar source.
    pub(crate) span: (usize, usize),
    pub(crate) refs: Vec<ElementRef>,
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

/// Parsed model of one parser rule from the rendered grammar source.
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
    /// Byte span of the rule body (between `:` and `;`).
    pub(crate) body_span: (usize, usize),
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
fn map_attr_type(raw: &str) -> String {
    match raw.trim() {
        "int" => "i32".to_owned(),
        "boolean" => "bool".to_owned(),
        "float" | "double" => "f64".to_owned(),
        other => other.to_owned(),
    }
}

/// Parses `[a: i32, int b, String s]`-style attribute declarations, accepting
/// both the rendered Rust `name: type` form and raw `type name` descriptors.
fn parse_attr_decls(clause: &str) -> Vec<AttrDecl> {
    let mut decls = Vec::new();
    for part in split_top_level(clause, ',') {
        let part = strip_default_initializer(part.trim());
        if part.is_empty() {
            continue;
        }
        if let Some((name, ty)) = split_name_colon_type(part) {
            decls.push(AttrDecl {
                name: name.to_owned(),
                ty: map_attr_type(ty),
            });
        } else if let Some((ty, name)) = part.rsplit_once(char::is_whitespace) {
            decls.push(AttrDecl {
                name: name.trim().to_owned(),
                ty: map_attr_type(ty),
            });
        }
    }
    decls
}

/// Removes a raw grammar initializer when it is the type's Rust `Default`.
///
/// Embedded attrs are initialized through `Default::default()`, so retaining
/// explicit `false` / zero initializers would only prevent the declaration
/// parser from recognizing otherwise portable ANTLR rule locals.
fn strip_default_initializer(part: &str) -> &str {
    let Some(index) = part
        .as_bytes()
        .iter()
        .enumerate()
        .find_map(|(index, byte)| {
            if *byte != b'='
                || part.as_bytes().get(index + 1) == Some(&b'=')
                || part
                    .as_bytes()
                    .get(index.wrapping_sub(1))
                    .is_some_and(|byte| matches!(*byte, b'!' | b'<' | b'>' | b'='))
            {
                return None;
            }
            Some(index)
        })
    else {
        return part;
    };
    let (declaration, value) = part.split_at(index);
    let value = &value[1..];
    matches!(value.trim(), "false" | "0")
        .then_some(declaration.trim_end())
        .unwrap_or(part)
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

/// Splits on `separator` at zero bracket/paren/angle/brace depth outside
/// string literals.
fn split_top_level(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
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
            '(' | '[' | '{' | '<' if !quoted => depth += 1,
            ')' | ']' | '}' | '>' if !quoted => depth -= 1,
            _ if ch == separator && !quoted && depth == 0 => {
                parts.push(&text[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Parses the rendered grammar into the embedded model.
///
/// `parser_rule_names` come from the parser `.interp` metadata and define
/// both which rules matter and their indices.
pub(crate) fn parse_embedded_model(
    source: &str,
    parser_rule_names: &[String],
) -> io::Result<EmbeddedModel> {
    let mut model = parse_embedded_rules_model(source, parser_rule_names);
    model.parser_members = parse_members_blocks(source, &["@parser::members", "@members"])?;
    Ok(model)
}

/// Parses only parser rule structure from the rendered grammar.
///
/// This is enough for action-state attribution in non-embedded generation,
/// where target-specific `@members` bodies are irrelevant and may be Java,
/// Python, or another runtime's syntax.
pub(crate) fn parse_embedded_rules_model(
    source: &str,
    parser_rule_names: &[String],
) -> EmbeddedModel {
    let mut rules: Vec<RuleModel> = parser_rule_names
        .iter()
        .map(|name| RuleModel {
            name: name.clone(),
            ..RuleModel::default()
        })
        .collect();

    for (name, header_start, colon, semicolon) in rule_definitions(source) {
        let Some(rule_index) = parser_rule_names.iter().position(|rule| *rule == name) else {
            continue;
        };
        let rule = &mut rules[rule_index];
        // Composite grammars can override an imported rule; the first
        // (delegator) definition wins, matching ANTLR's import semantics.
        if rule.body_span != (0, 0) {
            continue;
        }
        parse_rule_header_clauses(source, header_start, colon, rule);
        rule.body_span = (colon + 1, semicolon);
        rule.alts = parse_alternatives(source, colon + 1, semicolon);
    }

    EmbeddedModel {
        rules,
        parser_members: MembersModel::default(),
    }
}

/// Yields `(rule_name, header_start, colon_offset, semicolon_offset)` for
/// each rule definition in the grammar.
fn rule_definitions(source: &str) -> Vec<(String, usize, usize, usize)> {
    let mut definitions = Vec::new();
    let mut cursor = GrammarSourceCursor::new(source, 0);
    let mut statement_start: Option<usize> = None;
    let mut first_identifier: Option<(usize, usize)> = None;
    while let Some((index, ch)) = cursor.next_significant() {
        match ch {
            '@' => {
                // A named action (`@parser::members {...}` at grammar level,
                // `@init`/`@after` inside a rule header): skip its brace
                // block wholesale so its `::`, `;` and `:` never reach the
                // statement scan. Rule-header state is preserved, so a rule's
                // own `@init` keeps the pending rule identifier.
                if let Some(brace) = source[index..].find('{').map(|found| index + found) {
                    if let Some(close) = matching_action_brace(source, brace + 1) {
                        cursor.seek(close + 1);
                    }
                }
            }
            '{' | '[' => {
                // Skip named-action bodies / arg clauses wholesale so `;` and
                // `:` inside them cannot desynchronize the statement scan.
                let close = if ch == '{' {
                    matching_action_brace(source, index + 1)
                } else {
                    matching_arg_bracket(source, index + 1)
                };
                if let Some(close) = close {
                    cursor.seek(close + 1);
                }
                // A grammar-level `options {...}` / `tokens {...}` statement
                // ends with its block; clear it so the next rule's name is
                // not mistaken for a continuation of this statement.
                if matches!(
                    first_identifier.map(|(start, end)| &source[start..end]),
                    Some("options" | "tokens")
                ) {
                    statement_start = None;
                    first_identifier = None;
                }
            }
            ':' if first_identifier.is_some() => {
                // `::` inside e.g. `@parser::members` never reaches here (the
                // brace skip above consumes those blocks before their colon).
                let (id_start, id_end) = first_identifier.expect("checked above");
                let name = &source[id_start..id_end];
                let header_start = statement_start.unwrap_or(id_start);
                // Find the closing `;` from here, skipping action braces.
                let mut end_cursor = GrammarSourceCursor::new(source, index + 1);
                let mut semicolon = None;
                while let Some((end_index, end_ch)) = end_cursor.next_significant() {
                    match end_ch {
                        '{' => {
                            if let Some(close) = matching_action_brace(source, end_index + 1) {
                                end_cursor.seek(close + 1);
                            }
                        }
                        '[' => {
                            if let Some(close) = matching_arg_bracket(source, end_index + 1) {
                                end_cursor.seek(close + 1);
                            }
                        }
                        ';' => {
                            semicolon = Some(end_index);
                            break;
                        }
                        _ => {}
                    }
                }
                if let Some(semicolon) = semicolon {
                    definitions.push((name.to_owned(), header_start, index, semicolon));
                    cursor.seek(semicolon + 1);
                }
                statement_start = None;
                first_identifier = None;
            }
            ';' => {
                statement_start = None;
                first_identifier = None;
            }
            _ if ch == '_' || ch.is_ascii_alphanumeric() => {
                if statement_start.is_none() {
                    statement_start = Some(index);
                }
                if first_identifier.is_none() {
                    let mut end = index + ch.len_utf8();
                    while source[end..]
                        .chars()
                        .next()
                        .is_some_and(|next| next == '_' || next.is_ascii_alphanumeric())
                    {
                        end += 1;
                    }
                    // `grammar X;`, `import Y;`, `mode M;` headers and option
                    // keywords are filtered by the parser-rule-name lookup in
                    // the caller; here we just record the identifier.
                    first_identifier = Some((index, end));
                    cursor.seek(end);
                }
            }
            _ => {}
        }
    }
    definitions
}

/// `[` matcher for arg clauses: unlike lexer char sets these nest `[...]`
/// rarely, but strings may contain `]`.
fn matching_arg_bracket(source: &str, mut index: usize) -> Option<usize> {
    let mut nested = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    while let Some(ch) = source[index..].chars().next() {
        if escaped {
            escaped = false;
            index += ch.len_utf8();
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            '[' if !quoted => nested += 1,
            ']' if !quoted && nested == 0 => return Some(index),
            ']' if !quoted => nested -= 1,
            _ => {}
        }
        index += ch.len_utf8();
    }
    None
}

/// Parses the clauses between a rule's name and its `:`: args `[...]`,
/// `returns [...]`, `locals [...]`, `@init {...}`, `@after {...}`.
fn parse_rule_header_clauses(
    source: &str,
    header_start: usize,
    colon: usize,
    rule: &mut RuleModel,
) {
    let header = &source[header_start..colon];
    let mut offset = 0;
    // The rule name itself.
    offset = skip_ascii_whitespace(header, offset);
    while offset < header.len()
        && header[offset..]
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        offset += 1;
    }
    let mut pending_keyword: Option<&str> = None;
    while offset < header.len() {
        offset = skip_ascii_whitespace(header, offset);
        if offset >= header.len() {
            break;
        }
        let rest = &header[offset..];
        if rest.starts_with('[') {
            let Some(close) = matching_arg_bracket(header, offset + 1) else {
                break;
            };
            let clause = &header[offset + 1..close];
            let decls = parse_attr_decls(clause);
            match pending_keyword.take() {
                Some("locals") => {
                    rule.local_names
                        .extend(decls.iter().map(|decl| decl.name.clone()));
                    rule.attrs.extend(decls);
                }
                Some("returns") => rule.attrs.extend(decls),
                _ => {
                    for decl in &decls {
                        rule.arg_names.push(decl.name.clone());
                    }
                    rule.attrs.extend(decls);
                }
            }
            offset = close + 1;
        } else if rest.starts_with("returns") {
            pending_keyword = Some("returns");
            offset += "returns".len();
        } else if rest.starts_with("locals") {
            pending_keyword = Some("locals");
            offset += "locals".len();
        } else if rest.starts_with("@init") || rest.starts_with("@after") {
            let is_init = rest.starts_with("@init");
            let Some(brace) = header[offset..].find('{').map(|found| offset + found) else {
                break;
            };
            let Some(close) = matching_action_brace(header, brace + 1) else {
                break;
            };
            let body = header[brace + 1..close].trim().to_owned();
            if !body.is_empty() {
                if is_init {
                    rule.init_body = Some(body);
                } else {
                    rule.after_body = Some(body);
                }
            }
            offset = close + 1;
        } else if rest.starts_with("options") {
            let Some(brace) = header[offset..].find('{').map(|found| offset + found) else {
                break;
            };
            let Some(close) = matching_action_brace(header, brace + 1) else {
                break;
            };
            offset = close + 1;
        } else {
            offset += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
}

/// Splits a rule body into top-level alternatives and scans each one's
/// element references (labels, rule refs, token refs) in source order.
fn parse_alternatives(source: &str, body_start: usize, body_end: usize) -> Vec<AltModel> {
    let mut alts = Vec::new();
    let mut alt_start = body_start;
    let mut cursor = GrammarSourceCursor::new(source, body_start);
    let mut depth = 0_i32;
    while let Some((index, ch)) = cursor.next_significant() {
        if index >= body_end {
            break;
        }
        match ch {
            '{' => {
                if let Some(close) = matching_action_brace(source, index + 1) {
                    cursor.seek(close + 1);
                }
            }
            '[' => {
                if let Some(close) = matching_arg_bracket(source, index + 1) {
                    cursor.seek(close + 1);
                }
            }
            '(' => depth += 1,
            ')' => depth -= 1,
            '|' if depth == 0 => {
                alts.push(parse_alt(source, alt_start, index));
                alt_start = index + 1;
            }
            _ => {}
        }
    }
    alts.push(parse_alt(source, alt_start, body_end));
    alts
}

/// Scans one alternative's labels and references.
fn parse_alt(source: &str, start: usize, end: usize) -> AltModel {
    let mut refs = Vec::new();
    let mut label: Option<String> = None;
    let mut pending_label: Option<String> = None;
    let mut pending_list = false;
    let mut cursor = GrammarSourceCursor::new(source, start);
    while let Some((index, ch)) = cursor.next_significant() {
        if index >= end {
            break;
        }
        match ch {
            '{' => {
                if let Some(close) = matching_action_brace(source, index + 1) {
                    cursor.seek(close + 1);
                }
            }
            '[' => {
                if let Some(close) = matching_arg_bracket(source, index + 1) {
                    cursor.seek(close + 1);
                }
            }
            '<' => {
                // Element options such as `<assoc=right>`.
                if let Some(close) = source[index..end].find('>') {
                    cursor.seek(index + close + 1);
                }
            }
            '#' => {
                // Alternative label.
                let rest = source[index + 1..end].trim();
                let name: String = rest
                    .chars()
                    .take_while(|ch| *ch == '_' || ch.is_ascii_alphanumeric())
                    .collect();
                if !name.is_empty() {
                    label = Some(name);
                }
                // Nothing after the label matters for refs.
                break;
            }
            '(' | '~' => {
                if let Some(block_label) = pending_label.take() {
                    refs.push(ElementRef {
                        label: Some(block_label),
                        target: String::new(),
                        is_block: true,
                        is_list: std::mem::take(&mut pending_list),
                    });
                }
            }
            _ if ch == '_' || ch.is_ascii_alphabetic() => {
                let mut word_end = index + ch.len_utf8();
                while source[word_end..]
                    .chars()
                    .next()
                    .is_some_and(|next| next == '_' || next.is_ascii_alphanumeric())
                {
                    word_end += 1;
                }
                let word = &source[index..word_end];
                cursor.seek(word_end);
                // Label assignment? `x=ref` / `x+=ref` (but not `==`).
                let after = skip_ascii_whitespace(source, word_end);
                let bytes = source.as_bytes();
                let is_label = match bytes.get(after) {
                    Some(b'=') => bytes.get(after + 1) != Some(&b'='),
                    Some(b'+') => bytes.get(after + 1) == Some(&b'='),
                    _ => false,
                };
                if is_label {
                    pending_label = Some(word.to_owned());
                    pending_list = bytes.get(after) == Some(&b'+');
                    let skip = if pending_list { 2 } else { 1 };
                    let value_start = skip_ascii_whitespace(source, after + skip);
                    // A label directly on a string literal (`label='y'`): the
                    // cursor skips quoted text, so record the ref here.
                    if bytes.get(value_start) == Some(&b'\'') {
                        refs.push(ElementRef {
                            label: pending_label.take(),
                            target: String::new(),
                            is_block: true,
                            is_list: std::mem::take(&mut pending_list),
                        });
                    }
                    cursor.seek(after + skip);
                } else if word == "EOF" {
                    pending_label = None;
                } else {
                    refs.push(ElementRef {
                        label: pending_label.take(),
                        target: word.to_owned(),
                        is_block: false,
                        is_list: std::mem::take(&mut pending_list),
                    });
                }
            }
            _ => {}
        }
    }
    AltModel {
        label,
        span: (start, end),
        refs,
        leading_target: leading_element_target(source, start, end),
    }
}

/// Scans the raw alternative text for its first syntactic element and
/// returns the referenced name when that element is a bare identifier,
/// allowing `label=` / `label+=` prefixes and `<assoc=right>`-style element
/// options before it. Anything else — a string literal, `(...)` block,
/// `~set`, or `{action}` — yields `None`.
fn leading_element_target(source: &str, start: usize, end: usize) -> Option<String> {
    let bytes = source.as_bytes();
    let mut index = start;
    loop {
        index = skip_ascii_whitespace(source, index);
        if index >= end {
            return None;
        }
        match bytes[index] {
            b'<' => {
                let close = source[index..end].find('>')?;
                index += close + 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let newline = source[index..end].find('\n')?;
                index += newline + 1;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let close = source[index..end].find("*/")?;
                index += close + 2;
            }
            byte if byte == b'_' || byte.is_ascii_alphabetic() => {
                let mut word_end = index + 1;
                while word_end < end
                    && (bytes[word_end] == b'_' || bytes[word_end].is_ascii_alphanumeric())
                {
                    word_end += 1;
                }
                let word = &source[index..word_end];
                let after = skip_ascii_whitespace(source, word_end);
                let is_label = match bytes.get(after) {
                    Some(b'=') => bytes.get(after + 1) != Some(&b'='),
                    Some(b'+') => bytes.get(after + 1) == Some(&b'='),
                    _ => false,
                };
                if is_label {
                    // The labeled element follows; keep scanning.
                    index = after + if bytes[after] == b'+' { 2 } else { 1 };
                    continue;
                }
                return Some(word.to_owned());
            }
            _ => return None,
        }
    }
}

/// Parses `@parser::members {...}` / `@members {...}` blocks into the
/// members model. Multiple blocks accumulate.
fn parse_members_blocks(source: &str, markers: &[&str]) -> io::Result<MembersModel> {
    let mut members = MembersModel::default();
    let mut offset = 0;
    while let Some(block) = next_parser_action_block(source, offset, |_| true) {
        offset = block.after_brace;
        let prefix = source[..block.open_brace].trim_end();
        if !markers.iter().any(|marker| prefix.ends_with(marker)) {
            continue;
        }
        // `@lexer::members` also ends with `@members`? No — markers are
        // matched exactly against the trailing token, and `@lexer::members`
        // ends with `members`, not `@members`.
        if prefix.ends_with("@lexer::members") && !markers.contains(&"@lexer::members") {
            continue;
        }
        classify_members(block.body, &mut members)?;
    }
    Ok(members)
}

/// Splits a members body into field declarations, impl items, and module
/// items.
fn classify_members(body: &str, members: &mut MembersModel) -> io::Result<()> {
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
    /// Token name -> token type, from the `.interp` metadata.
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
                    && suffix_text[..suffix_len].ends_with("_all")
                    && after_suffix.starts_with("()")
                {
                    // `$ctx.<rule>_all()` — a generated list accessor call;
                    // consume the empty parens along with the suffix.
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
            is_block: false,
            is_list: false,
        };
        let _ = target_rule;
        return translate_element_read(&element, usize::MAX, suffix, ctx, body);
    }
    if ctx.token_types.contains_key(name) {
        let element = ElementRef {
            label: None,
            target: name.to_owned(),
            is_block: false,
            is_list: false,
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

/// `$ctx.member` — a labeled element read (`$ctx.r`) or a generated list
/// accessor (`$ctx.elseIfStatement_all`).
fn translate_ctx_member(member: &str, ctx: &TranslationCtx<'_>, body: &str) -> io::Result<String> {
    if let Some((element, occurrence)) = ctx.resolve_label(member) {
        // `$ctx.r` denotes the labeled child's subtree (Java field of the
        // context); translate like `$r.ctx`.
        return translate_element_read(&element, occurrence, Some("ctx"), ctx, body);
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
        // `label+=x`: the label denotes the list of every `x` child.
        if let Some(rule_index) = ctx.rule_index_by_name(&element.target) {
            return match suffix {
                None | Some("ctx") => Ok(format!(
                    "__ctx.child_rule_trees(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).collect::<Vec<_>>()"
                )),
                Some(other) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsupported list-label read .{other} in embedded action: {body}"),
                )),
            };
        }
        if let Some(token_type) = ctx.token_types.get(&element.target) {
            return Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type}).collect::<Vec<_>>()"
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

    fn model(source: &str, rules: &[&str]) -> EmbeddedModel {
        parse_embedded_model(
            source,
            &rules.iter().map(|&r| r.to_owned()).collect::<Vec<_>>(),
        )
        .expect("model parses")
    }

    fn tokens(pairs: &[(&str, i32)]) -> BTreeMap<String, i32> {
        pairs
            .iter()
            .map(|(name, ty)| ((*name).to_owned(), *ty))
            .collect()
    }

    #[test]
    fn parses_rule_attrs_and_alts() {
        let source = "grammar T;\n\
            s : e {writeln!(self.output(), \"{}\", $e.v);} ;\n\
            e returns [v: i32] : a=e '*' b=e {$v = 1;} | INT {$v = 2;} ;\n\
            INT : [0-9]+ ;\n";
        let m = model(source, &["s", "e"]);
        assert_eq!(
            m.rules[1].attrs,
            vec![AttrDecl {
                name: "v".into(),
                ty: "i32".into()
            }]
        );
        assert_eq!(m.rules[1].alts.len(), 2);
        assert_eq!(m.rules[1].alts[0].refs.len(), 2);
        assert_eq!(m.rules[1].alts[0].refs[0].label.as_deref(), Some("a"));
        assert_eq!(m.rules[1].alts[0].refs[1].label.as_deref(), Some("b"));
    }

    #[test]
    fn parses_raw_default_valued_rule_locals() {
        let model = parse_embedded_rules_model(
            "parser grammar T;\ns locals [boolean seen=false, int count = 0] : ;",
            &["s".to_owned()],
        );

        assert_eq!(
            model.rules[0].attrs,
            [
                AttrDecl {
                    name: "seen".to_owned(),
                    ty: "bool".to_owned(),
                },
                AttrDecl {
                    name: "count".to_owned(),
                    ty: "i32".to_owned(),
                },
            ]
        );
        assert_eq!(model.rules[0].local_names, ["seen", "count"]);
    }

    #[test]
    fn preserves_non_default_local_comparison_initializers() {
        assert_eq!(
            strip_default_initializer("boolean seen = other == false"),
            "boolean seen = other == false"
        );
        assert_eq!(
            strip_default_initializer("int count = other == 0"),
            "int count = other == 0"
        );
        assert_eq!(
            strip_default_initializer("boolean seen=false"),
            "boolean seen"
        );
        assert_eq!(strip_default_initializer("int count = 0"), "int count");
    }

    #[test]
    fn parses_java_style_attr_decls() {
        let decls = parse_attr_decls("int v, String s");
        assert_eq!(
            decls[0],
            AttrDecl {
                name: "v".into(),
                ty: "i32".into()
            }
        );
        assert_eq!(
            decls[1],
            AttrDecl {
                name: "s".into(),
                ty: "String".into()
            }
        );
    }

    #[test]
    fn translates_attr_and_rule_reads() {
        let source = "grammar T;\n\
            s : e {writeln!(self.output(), \"{}\", $e.v);} ;\n\
            e returns [v: i32] : INT {$v = $INT.int;} ;\n";
        let m = model(source, &["s", "e"]);
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
    fn translates_ctx_and_text() {
        let source = "grammar T;\ns : ID {writeln!(self.output(), \"{}\", $text);} ;\n";
        let m = model(source, &["s"]);
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
    fn classifies_member_blocks() {
        let source = "grammar T;\n\
            @parser::members {\n\
            i: i32 = 0;\n\
            #[allow(non_snake_case)]\n\
            fn Property(&self) -> bool {\n    true\n}\n\
            struct LeafListener;\n\
            }\n\
            s : ID ;\n";
        let m = model(source, &["s"]);
        assert_eq!(m.parser_members.fields.len(), 1);
        assert_eq!(m.parser_members.fields[0].name, "i");
        assert_eq!(m.parser_members.fields[0].init, "0");
        assert_eq!(m.parser_members.impl_items.len(), 1);
        assert!(m.parser_members.impl_items[0].contains("fn Property"));
        assert_eq!(m.parser_members.module_items.len(), 1);
    }

    #[test]
    fn dollar_inside_strings_is_left_alone() {
        let source = "grammar T;\ns : ID ;\n";
        let m = model(source, &["s"]);
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
