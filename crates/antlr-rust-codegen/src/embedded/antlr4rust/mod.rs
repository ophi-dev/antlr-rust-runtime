mod aliases;
mod macros;
mod scopes;

use super::*;
pub(crate) use aliases::*;
pub(crate) use macros::*;
pub(crate) use scopes::*;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ParserBodyTranslation {
    pub(crate) source: String,
    pub(crate) uses_input: bool,
    pub(crate) uses_local_context: bool,
    pub(crate) token_aliases: BTreeSet<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct LoweredAntlr4RustBody {
    pub(crate) source: String,
    pub(crate) uses_input: bool,
    pub(crate) uses_local_context: bool,
    pub(crate) token_aliases: BTreeSet<String>,
    pub(crate) direct_alias_imports: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RustLexemeKind {
    Trivia,
    Identifier,
    Literal,
    Punctuation(u8),
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RustLexeme {
    pub(crate) kind: RustLexemeKind,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RustReplacement {
    pub(crate) range: Range<usize>,
    pub(crate) text: String,
}

#[derive(Debug)]
pub(crate) struct FormatMacroCaptures {
    pub(crate) format_literal: usize,
    pub(crate) close: usize,
    pub(crate) aliases: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Antlr4RustSourceKind {
    Body,
    MemberItem,
}

pub(crate) fn lower_antlr4rust_surface(
    body: &str,
    token_aliases: &BTreeSet<String>,
    token_alias_module: &str,
    input_facade: &str,
    local_context_expression: Option<&str>,
    source_kind: Antlr4RustSourceKind,
) -> io::Result<LoweredAntlr4RustBody> {
    let lexemes = lex_rust_body(body);
    let delimiters = RustDelimiterMap::new(&lexemes);
    let format_captures =
        format_macro_capture_candidates(body, &lexemes, &delimiters, token_aliases);
    let has_token_alias_identifier = lexemes.iter().any(|lexeme| {
        lexeme.kind == RustLexemeKind::Identifier
            && token_aliases.contains(rust_identifier_name(lexeme_text(body, *lexeme)))
    });
    let needs_alias_analysis = has_token_alias_identifier || !format_captures.is_empty();
    let has_compatibility_receiver = lexemes.iter().any(|lexeme| {
        lexeme.kind == RustLexemeKind::Identifier
            && matches!(lexeme_text(body, *lexeme), "recog" | "_localctx")
    });
    let needs_syntax_analysis = needs_alias_analysis || has_compatibility_receiver;
    let syntax = if needs_syntax_analysis {
        if source_kind == Antlr4RustSourceKind::MemberItem {
            rust_syntax::analyze_member_item(body)?
        } else {
            rust_syntax::analyze(body)?
        }
    } else {
        rust_syntax::RustSyntax::default()
    };
    let local_bindings = if needs_alias_analysis {
        local_antlr4rust_alias_bindings(body, &lexemes, token_aliases, token_alias_module, &syntax)?
    } else {
        AliasBindingScopes::default()
    };
    let mut replacements = Vec::new();
    let mut opaque_macro_aliases = BTreeMap::<(usize, usize), BTreeSet<String>>::new();
    let mut opaque_non_expression_aliases = BTreeMap::<(usize, String), BTreeSet<String>>::new();
    let mut conditional_macro_alias_fallbacks =
        BTreeMap::<(usize, String, String), BTreeSet<String>>::new();
    let mut uses_input = false;
    let mut uses_local_context = false;
    let mut used_token_aliases = local_bindings.use_target_aliases.clone();
    replacements.extend(local_bindings.use_target_replacements.iter().cloned());
    for capture in format_captures {
        let aliases = capture
            .aliases
            .into_iter()
            .filter(|alias| !local_bindings.is_bound(alias, capture.format_literal))
            .collect::<BTreeSet<_>>();
        if aliases.is_empty() {
            continue;
        }
        if syntax.is_opaque_macro_byte(lexemes[capture.format_literal].start) {
            if let Some((insertion, active)) =
                syntax.conditional_macro_fallback(lexemes[capture.format_literal].start)
            {
                used_token_aliases.extend(aliases.iter().cloned());
                let mut alias_module_path = "super::"
                    .repeat(syntax.inline_module_depth(lexemes[capture.format_literal].start));
                alias_module_path.push_str(token_alias_module);
                conditional_macro_alias_fallbacks
                    .entry((insertion, active.to_owned(), alias_module_path))
                    .or_default()
                    .extend(aliases);
            }
            continue;
        }
        used_token_aliases.extend(aliases.iter().cloned());
        let trailing_comma = previous_significant(&lexemes, capture.close)
            .is_some_and(|previous| lexemes[previous].kind == RustLexemeKind::Punctuation(b','));
        let mut alias_module_path =
            "super::".repeat(syntax.inline_module_depth(lexemes[capture.format_literal].start));
        alias_module_path.push_str(token_alias_module);
        let mut text = if trailing_comma {
            " ".to_owned()
        } else {
            ", ".to_owned()
        };
        text.push_str(
            &aliases
                .iter()
                .map(|alias| format!("{alias} = {alias_module_path}::{alias}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        replacements.push(RustReplacement {
            range: lexemes[capture.close].start..lexemes[capture.close].start,
            text,
        });
    }
    replacements.extend(conditional_macro_alias_fallbacks.into_iter().map(
        |((insertion, active, alias_module_path), aliases)| RustReplacement {
            range: insertion..insertion,
            text: format!(
                "#[cfg(not({active}))]\n#[allow(unused_imports)]\n\
                 use {alias_module_path}::{{{}}};\n",
                aliases.into_iter().collect::<Vec<_>>().join(", ")
            ),
        },
    ));
    for (position, lexeme) in lexemes.iter().enumerate() {
        if lexeme.kind != RustLexemeKind::Identifier {
            continue;
        }
        let source_identifier = lexeme_text(body, *lexeme);
        let alias_identifier = rust_identifier_name(source_identifier);
        let unqualified_alias = is_unqualified_identifier(&lexemes, position);
        let generated_module_alias = relative_alias_module_path(body, &lexemes, position)
            .is_some_and(|path| {
                path.targets_generated_module(syntax.inline_module_depth(lexeme.start))
            });
        if source_kind == Antlr4RustSourceKind::Body
            && token_aliases.contains(alias_identifier)
            && syntax.is_opaque_macro_identifier(lexeme.start)
            && !local_bindings.binding_positions.contains(&position)
            && !local_bindings.is_bound(alias_identifier, position)
            && let Some(range) = opaque_macro_invocation_range(&lexemes, position, &delimiters)
        {
            used_token_aliases.insert(alias_identifier.to_owned());
            if syntax.opaque_macro_accepts_expression_fallback(lexeme.start) {
                opaque_macro_aliases
                    .entry((range.start, range.end))
                    .or_default()
                    .insert(alias_identifier.to_owned());
            } else {
                let invocation = lexemes.partition_point(|candidate| candidate.end <= range.start);
                let mut block = delimiters.enclosing_block(invocation);
                if syntax.opaque_macro_requires_parent_block_fallback(lexeme.start)
                    && let Some(open) = block.start.checked_sub(1)
                {
                    block = delimiters.enclosing_block(open);
                }
                if let Some(insertion) = block_cfg_fallback_insertion(&lexemes, &block, &delimiters)
                {
                    let mut module_path =
                        "super::".repeat(syntax.inline_module_depth(lexeme.start));
                    module_path.push_str(token_alias_module);
                    opaque_non_expression_aliases
                        .entry((insertion, module_path))
                        .or_default()
                        .insert(alias_identifier.to_owned());
                }
            }
        }
        if token_aliases.contains(alias_identifier)
            && !local_bindings.binding_positions.contains(&position)
            && !local_bindings.is_bound(alias_identifier, position)
            && !syntax.is_type_identifier(lexeme.start)
            && !syntax.is_declaration_identifier(lexeme.start)
            && !syntax.is_non_value_identifier(lexeme.start)
            && !syntax.is_opaque_macro_identifier(lexeme.start)
            && !local_bindings
                .use_target_replacements
                .iter()
                .any(|replacement| {
                    replacement.range.start == lexeme.start && replacement.range.end == lexeme.end
                })
            && (unqualified_alias || generated_module_alias)
            && !is_token_alias_path_or_field(body, &lexemes, position, &delimiters)
        {
            used_token_aliases.insert(alias_identifier.to_owned());
            let qualified = format!("{token_alias_module}::{alias_identifier}");
            let text = if syntax.is_struct_field_shorthand(lexeme.start) {
                format!("{source_identifier}: {qualified}")
            } else {
                qualified
            };
            replacements.push(RustReplacement {
                range: lexeme.start..lexeme.end,
                text,
            });
        }
        // Standalone `recog` and `_localctx` are reserved compatibility
        // receivers. Unknown shapes fail here so diagnostics retain the
        // owning grammar coordinate instead of surfacing from rustc later.
        match source_identifier {
            _ if source_kind == Antlr4RustSourceKind::MemberItem => {}
            _ if syntax.is_opaque_macro_identifier(lexeme.start) => {}
            "recog" if is_standalone_identifier(&lexemes, position) => {
                replacements.push(recog_input_replacement(
                    body,
                    &lexemes,
                    position,
                    input_facade,
                )?);
                uses_input = true;
            }
            "_localctx" if is_standalone_identifier(&lexemes, position) => {
                replacements.push(local_context_replacement(
                    body,
                    &lexemes,
                    position,
                    local_context_expression
                        .expect("parser receiver lowering provides a context expression"),
                )?);
                uses_local_context = true;
            }
            _ => {}
        }
    }
    replacements.extend(opaque_non_expression_aliases.into_iter().map(
        |((insertion, module_path), aliases)| RustReplacement {
            range: insertion..insertion,
            text: format!(
                "#[allow(unused_imports)]\nuse {module_path}::{{{}}};\n",
                aliases.into_iter().collect::<Vec<_>>().join(", ")
            ),
        },
    ));
    for ((start, end), aliases) in opaque_macro_aliases {
        let fallback_module = format!("__antlr4rust_opaque_aliases_{start}");
        let aliases = aliases.into_iter().collect::<Vec<_>>().join(", ");
        replacements.push(RustReplacement {
            range: start..start,
            text: format!(
                "{{\nmod {fallback_module} {{\n    #[allow(unused_imports)]\n    \
                 pub(super) use super::{token_alias_module}::{{{aliases}}};\n}}\n\
                 #[allow(unused_imports)]\nuse {fallback_module}::*;\n"
            ),
        });
        replacements.push(RustReplacement {
            range: end..end,
            text: "\n}".to_owned(),
        });
    }
    replacements.sort_by_key(|replacement| replacement.range.start);
    Ok(LoweredAntlr4RustBody {
        source: apply_rust_replacements(body, &replacements)?,
        uses_input,
        uses_local_context,
        token_aliases: used_token_aliases,
        direct_alias_imports: local_bindings.direct_alias_imports,
    })
}

pub(crate) fn validate_lexer_body_compatibility_receivers(body: &str) -> io::Result<()> {
    let lowered = lower_antlr4rust_surface(
        body,
        &BTreeSet::new(),
        ANTLR4RUST_TOKEN_ALIAS_MODULE,
        ANTLR4RUST_INPUT_FACADE,
        Some("__antlr4rust_lexer_context"),
        Antlr4RustSourceKind::Body,
    )?;
    if lowered.uses_input {
        return Err(unsupported_antlr4rust(
            "`recog.input` is only supported in embedded parser bodies",
        ));
    }
    if lowered.uses_local_context {
        return Err(unsupported_antlr4rust(
            "`_localctx` is only supported in embedded parser bodies",
        ));
    }
    Ok(())
}

#[derive(Debug, Default)]
pub(crate) struct AliasBindingScopes {
    binding_positions: BTreeSet<usize>,
    scopes: BTreeMap<String, Vec<Range<usize>>>,
    use_target_aliases: BTreeSet<String>,
    use_target_replacements: Vec<RustReplacement>,
    direct_alias_imports: BTreeSet<String>,
    local_cfg_alias_fallbacks: BTreeMap<(usize, usize, String), LocalCfgAliasFallback>,
}

#[derive(Debug)]
pub(crate) struct LocalCfgAliasFallback {
    block_insertion: usize,
    earliest_lexical: Option<LexicalCfgAliasFallback>,
    item_insertion: Option<usize>,
    item_active_predicates: BTreeSet<String>,
}

#[derive(Debug)]
pub(crate) struct LexicalCfgAliasFallback {
    insertion: usize,
    active_predicates: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CfgAliasBindingKind {
    Lexical,
    Item,
}

pub(crate) struct CfgAliasFallbackSite<'a> {
    block: Range<usize>,
    block_insertion: usize,
    binding_insertion: usize,
    active: &'a str,
    kind: CfgAliasBindingKind,
}
