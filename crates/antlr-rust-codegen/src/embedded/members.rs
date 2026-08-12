// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use super::{
    ANTLR4RUST_INPUT_FACADE, Antlr4RustSourceKind, BTreeSet, MemberField, MemberItem, MembersModel,
    Range, RustDelimiterMap, RustLexeme, RustLexemeKind, SourceId, io, lex_rust_body, lexeme_text,
    lower_antlr4rust_surface, matching_action_brace, member_cfg_predicates, next_significant,
    rust_identifier_name, rust_syntax, skip_ascii_whitespace, split_name_colon_type,
};

/// Splits a members body into field declarations, impl items, and module
/// items.
pub(crate) fn classify_members(
    body: &str,
    source: SourceId,
    members: &mut MembersModel,
) -> io::Result<()> {
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
            let end = member_attribute_end(rest)?;
            pending_attrs.push_str(&rest[..end]);
            pending_attrs.push('\n');
            offset += end;
        } else if rest.starts_with("fn ") {
            let item_end = item_end_from(body, offset)?;
            let mut item = std::mem::take(&mut pending_attrs);
            item.push_str(body[offset..item_end].trim());
            members.impl_items.push(MemberItem { source, body: item });
            offset = item_end;
        } else if rest.starts_with("struct ")
            || rest.starts_with("impl ")
            || rest.starts_with("impl<")
        {
            let item_end = item_end_from(body, offset)?;
            record_member_module_symbols(
                &body[offset..item_end],
                &member_cfg_predicates(&pending_attrs),
                members,
            )?;
            let mut item = std::mem::take(&mut pending_attrs);
            item.push_str(body[offset..item_end].trim());
            members.module_items.push(MemberItem { source, body: item });
            offset = item_end;
        } else if rest.starts_with("use ") {
            let item_end = use_item_end_from(body, offset)?;
            record_member_module_symbols(
                &body[offset..item_end],
                &member_cfg_predicates(&pending_attrs),
                members,
            )?;
            let mut item = std::mem::take(&mut pending_attrs);
            item.push_str(body[offset..item_end].trim());
            members.module_items.push(MemberItem { source, body: item });
            offset = item_end;
        } else if let Some(field) = parse_member_field(&body[offset..], source) {
            let (mut field, consumed) = field;
            field.attributes = std::mem::take(&mut pending_attrs);
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

pub(crate) fn member_attribute_end(rest: &str) -> io::Result<usize> {
    let lexemes = lex_rust_body(rest);
    let delimiters = RustDelimiterMap::new(&lexemes);
    let Some(hash) = lexemes
        .iter()
        .position(|lexeme| lexeme.kind != RustLexemeKind::Trivia)
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unterminated attribute in @members block",
        ));
    };
    let mut open = next_significant(&lexemes, hash);
    if open.is_some_and(|position| lexemes[position].kind == RustLexemeKind::Punctuation(b'!')) {
        open = open.and_then(|position| next_significant(&lexemes, position));
    }
    let close = open
        .filter(|position| lexemes[*position].kind == RustLexemeKind::Punctuation(b'['))
        .and_then(|position| delimiters.pairs[position])
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "unterminated attribute in @members block",
            )
        })?;
    Ok(lexemes[close].end)
}

pub(crate) fn member_field_initializer_attributes(attributes: &str) -> String {
    member_cfg_predicates(attributes)
        .into_iter()
        .map(|predicate| format!("#[cfg({predicate})]"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn record_member_module_symbols(
    item: &str,
    cfg_predicates: &[String],
    members: &mut MembersModel,
) -> io::Result<()> {
    let item = item.trim_start();
    if item.starts_with("struct ") {
        let lexemes = lex_rust_body(item);
        if let Some(keyword) = lexemes.iter().position(|lexeme| {
            lexeme.kind == RustLexemeKind::Identifier && lexeme_text(item, *lexeme) == "struct"
        }) && let Some(name) = next_significant(&lexemes, keyword)
            .filter(|position| lexemes[*position].kind == RustLexemeKind::Identifier)
            .map(|position| rust_identifier_name(lexeme_text(item, lexemes[position])))
        {
            members.module_symbols.insert(name.to_owned());
            if rust_syntax::struct_has_value_constructor(item)? {
                record_member_value_symbol(name, cfg_predicates, members);
            }
        }
        return Ok(());
    }
    let Some(rest) = item.strip_prefix("use ") else {
        return Ok(());
    };
    for name in use_tree_bindings(rest)? {
        members.module_symbols.insert(name.clone());
        members
            .module_import_cfgs
            .entry(name)
            .or_default()
            .push(cfg_predicates.to_vec());
    }
    Ok(())
}

pub(crate) fn record_member_value_symbol(
    name: &str,
    cfg_predicates: &[String],
    members: &mut MembersModel,
) {
    members
        .module_symbol_cfgs
        .entry(name.to_owned())
        .or_default()
        .push(cfg_predicates.to_vec());
}

pub(crate) fn use_tree_bindings(source: &str) -> io::Result<BTreeSet<String>> {
    Ok(analyze_use_tree(source)?.bindings)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UseTreeTarget {
    pub(crate) name: String,
    pub(crate) range: Range<usize>,
    pub(crate) local_module_leaf: bool,
    pub(crate) binding: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct UseTreeAnalysis {
    pub(crate) bindings: BTreeSet<String>,
    pub(crate) binding_ranges: Vec<(String, Range<usize>)>,
    pub(crate) targets: Vec<UseTreeTarget>,
    pub(crate) contains_glob: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct UsePathComponent {
    name: String,
    range: Range<usize>,
}

pub(crate) fn analyze_use_tree(source: &str) -> io::Result<UseTreeAnalysis> {
    let mut lexemes = lex_rust_body(source)
        .into_iter()
        .filter(|lexeme| lexeme.kind != RustLexemeKind::Trivia)
        .collect::<Vec<_>>();
    let Some(semicolon) = lexemes.pop() else {
        return Err(invalid_use_tree());
    };
    if semicolon.kind != RustLexemeKind::Punctuation(b';') {
        return Err(invalid_use_tree());
    }

    let mut parser = UseTreeBindingParser {
        source,
        lexemes: &lexemes,
        position: 0,
        analysis: UseTreeAnalysis::default(),
    };
    parser.parse_tree(&[], false)?;
    if parser.position != parser.lexemes.len() {
        return Err(invalid_use_tree());
    }
    Ok(parser.analysis)
}

pub(crate) struct UseTreeBindingParser<'a> {
    source: &'a str,
    lexemes: &'a [RustLexeme],
    position: usize,
    analysis: UseTreeAnalysis,
}

impl UseTreeBindingParser<'_> {
    fn parse_tree(&mut self, prefix: &[UsePathComponent], global: bool) -> io::Result<()> {
        let global = (prefix.is_empty() && self.consume_path_separator()) || global;
        if self.consume_punctuation(b'*') {
            self.analysis.contains_glob = true;
            return Ok(());
        }
        if self.consume_punctuation(b'{') {
            return self.parse_group(prefix, global);
        }
        let component = self.consume_identifier().ok_or_else(invalid_use_tree)?;
        let mut path = prefix.to_vec();
        path.push(component.clone());
        if self.peek_text() == Some("as") {
            self.position += 1;
            let alias = self.consume_identifier().ok_or_else(invalid_use_tree)?;
            let binding = (alias.name != "_").then_some(alias);
            self.record_target(&path, global, binding.as_ref());
            if let Some(alias) = binding {
                self.record_binding(alias);
            }
            return Ok(());
        }
        if self.consume_path_separator() {
            return self.parse_tree(&path, global);
        }
        if component.name == "self" {
            if let Some(parent) = prefix.last() {
                let binding = parent.clone();
                self.record_target(prefix, global, Some(&binding));
                self.record_binding(binding);
            }
        } else {
            let binding =
                (!matches!(component.name.as_str(), "crate" | "super")).then_some(component);
            self.record_target(&path, global, binding.as_ref());
            if let Some(binding) = binding {
                self.record_binding(binding);
            }
        }
        Ok(())
    }

    fn parse_group(&mut self, prefix: &[UsePathComponent], global: bool) -> io::Result<()> {
        loop {
            if self.consume_punctuation(b'}') {
                return Ok(());
            }
            self.parse_tree(prefix, global)?;
            if self.consume_punctuation(b',') {
                continue;
            }
            if self.consume_punctuation(b'}') {
                return Ok(());
            }
            return Err(invalid_use_tree());
        }
    }

    fn record_target(
        &mut self,
        path: &[UsePathComponent],
        global: bool,
        binding: Option<&UsePathComponent>,
    ) {
        let Some(target) = path.last() else {
            return;
        };
        self.analysis.targets.push(UseTreeTarget {
            name: target.name.clone(),
            range: target.range.clone(),
            local_module_leaf: !global
                && path.len() == 2
                && path
                    .first()
                    .is_some_and(|component| component.name == "self"),
            binding: binding.map(|binding| binding.name.clone()),
        });
    }

    fn record_binding(&mut self, binding: UsePathComponent) {
        self.analysis.bindings.insert(binding.name.clone());
        self.analysis
            .binding_ranges
            .push((binding.name, binding.range));
    }

    fn consume_identifier(&mut self) -> Option<UsePathComponent> {
        let lexeme = *self.lexemes.get(self.position)?;
        if lexeme.kind != RustLexemeKind::Identifier {
            return None;
        }
        self.position += 1;
        Some(UsePathComponent {
            name: lexeme_text(self.source, lexeme)
                .strip_prefix("r#")
                .unwrap_or_else(|| lexeme_text(self.source, lexeme))
                .to_owned(),
            range: lexeme.start..lexeme.end,
        })
    }

    fn consume_path_separator(&mut self) -> bool {
        if self
            .lexemes
            .get(self.position..self.position + 2)
            .is_some_and(|pair| {
                pair.iter()
                    .all(|lexeme| lexeme.kind == RustLexemeKind::Punctuation(b':'))
            })
        {
            self.position += 2;
            true
        } else {
            false
        }
    }

    fn consume_punctuation(&mut self, punctuation: u8) -> bool {
        if self
            .lexemes
            .get(self.position)
            .is_some_and(|lexeme| lexeme.kind == RustLexemeKind::Punctuation(punctuation))
        {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn peek_text(&self) -> Option<&str> {
        self.lexemes
            .get(self.position)
            .map(|lexeme| lexeme_text(self.source, *lexeme))
    }
}

pub(crate) fn invalid_use_tree() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "unsupported use tree in @members block",
    )
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct MemberTokenAliasTranslation {
    pub(crate) source: String,
    pub(crate) token_aliases: BTreeSet<String>,
    pub(crate) direct_alias_imports: BTreeSet<String>,
}

pub(crate) fn translate_member_token_aliases(
    item: &str,
    token_aliases: &BTreeSet<String>,
    token_alias_module: &str,
) -> io::Result<MemberTokenAliasTranslation> {
    let lowered = lower_antlr4rust_surface(
        item,
        token_aliases,
        token_alias_module,
        ANTLR4RUST_INPUT_FACADE,
        None,
        Antlr4RustSourceKind::MemberItem,
    )?;
    Ok(MemberTokenAliasTranslation {
        source: lowered.source,
        token_aliases: lowered.token_aliases,
        direct_alias_imports: lowered.direct_alias_imports,
    })
}

pub(crate) fn translate_member_field_type_token_aliases(
    ty: &str,
    token_aliases: &BTreeSet<String>,
    token_alias_module: &str,
) -> io::Result<MemberTokenAliasTranslation> {
    const PREFIX: &str = "type __Antlr4RustMemberField = ";
    let wrapped = format!("{PREFIX}{ty};");
    let translated = translate_member_token_aliases(&wrapped, token_aliases, token_alias_module)?;
    let source = translated
        .source
        .strip_prefix(PREFIX)
        .and_then(|source| source.strip_suffix(';'))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "member field type translation changed its synthetic declaration",
            )
        })?
        .to_owned();
    Ok(MemberTokenAliasTranslation {
        source,
        token_aliases: translated.token_aliases,
        direct_alias_imports: translated.direct_alias_imports,
    })
}

pub(crate) fn use_item_end_from(body: &str, offset: usize) -> io::Result<usize> {
    lex_rust_body(&body[offset..])
        .into_iter()
        .find(|lexeme| lexeme.kind == RustLexemeKind::Punctuation(b';'))
        .map(|lexeme| offset + lexeme.end)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "unterminated use item in @members block",
            )
        })
}

/// Finds the end of an item: the matching `}` of its first top-level brace
/// block, or the terminating `;` for braceless items (`use x;`).
pub(crate) fn item_end_from(body: &str, offset: usize) -> io::Result<usize> {
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
pub(crate) fn parse_member_field(rest: &str, source: SourceId) -> Option<(MemberField, usize)> {
    let lexemes = lex_rust_body(rest);
    let delimiters = RustDelimiterMap::new(&lexemes);
    let mut separator = None;
    let mut terminator = None;
    let mut position = 0;
    while position < lexemes.len() {
        if matches!(
            lexemes[position].kind,
            RustLexemeKind::Punctuation(b'(' | b'[' | b'{')
        ) && let Some(close) = delimiters.pairs[position]
        {
            position = close + 1;
            continue;
        }
        match lexemes[position].kind {
            RustLexemeKind::Punctuation(b'=') if separator.is_none() => {
                separator = Some(position);
            }
            RustLexemeKind::Punctuation(b';') => {
                terminator = Some(position);
                break;
            }
            _ => {}
        }
        position += 1;
    }
    let separator = separator?;
    let terminator = terminator?;
    let name_ty = rest[..lexemes[separator].start].trim();
    let init = rest[lexemes[separator].end..lexemes[terminator].start].trim();
    let (name, ty) = split_name_colon_type(name_ty.trim())?;
    Some((
        MemberField {
            source,
            attributes: String::new(),
            name: name.to_owned(),
            ty: ty.to_owned(),
            init: init.to_owned(),
        },
        lexemes[terminator].end,
    ))
}
