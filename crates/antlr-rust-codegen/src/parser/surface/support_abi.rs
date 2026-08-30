// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
/// Runtime support surface selected for the generated-source API revision.
///
/// Keeping this selection in the parser surface layer makes a future ABI
/// revision an explicit mapping instead of a collection of renderer literals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GeneratedSupportBindings {
    codegen_api: u32,
}

impl GeneratedSupportBindings {
    pub(crate) const fn current() -> Self {
        Self {
            codegen_api: antlr4_runtime::__ANTLR4_RUST_CODEGEN_API,
        }
    }

    pub(crate) fn module_header(self) -> String {
        debug_assert_eq!(self.codegen_api, antlr4_runtime::__ANTLR4_RUST_CODEGEN_API);
        generated_module_header()
    }

    pub(crate) const fn module_footer(self) -> &'static str {
        GENERATED_MODULE_FOOTER
    }
}

#[derive(Debug)]
struct Antlr4RustTokenAliasInventory {
    names: BTreeSet<String>,
    values: BTreeMap<String, i32>,
}

fn antlr4rust_token_alias_inventory(
    data: &RecognizerCodegenData<'_>,
    root_type_name: &str,
    source: SourceId,
) -> Antlr4RustTokenAliasInventory {
    let owner_type = data
        .sources
        .and_then(|sources| sources.get(source))
        .map(grammar::parse_loader_unit)
        .map_or_else(
            || root_type_name.to_owned(),
            |parsed| {
                let mut name = parsed.header.name.value;
                if parsed.header.kind == GrammarKind::Combined {
                    name.push_str("Parser");
                }
                name
            },
        );
    let values = antlr4rust_token_alias_values(&owner_type, data, source);
    let names = values.keys().cloned().collect();
    Antlr4RustTokenAliasInventory { names, values }
}

fn record_antlr4rust_translation(
    translated: &embedded::ParserBodyTranslation,
    aliases: &Antlr4RustTokenAliasInventory,
    rule_index: usize,
    uses_input: &mut bool,
    context_roots: &mut BTreeSet<usize>,
    aliases_to_emit: &mut BTreeMap<String, i32>,
) {
    *uses_input |= translated.uses_input;
    if translated.uses_local_context {
        context_roots.insert(rule_index);
    }
    aliases_to_emit.extend(
        translated
            .token_aliases
            .iter()
            .filter_map(|name| aliases.values.get(name).map(|value| (name.clone(), *value))),
    );
}

fn antlr4rust_compatibility_rules(
    model: &embedded::EmbeddedModel,
    roots: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    let rule_indices = model
        .rules
        .iter()
        .enumerate()
        .map(|(index, rule)| (rule.name.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut reachable = roots.clone();
    let mut pending = roots.iter().copied().collect::<Vec<_>>();
    while let Some(rule_index) = pending.pop() {
        for child_name in context_child_cardinalities(&model.rules[rule_index], None).keys() {
            let Some(&child_index) = rule_indices.get(child_name.as_str()) else {
                continue;
            };
            if reachable.insert(child_index) {
                pending.push(child_index);
            }
        }
    }
    reachable
}

/// Builds the embedded translation of every action, predicate, `@init` and
/// `@after` body in the rendered grammar, plus the members model.
pub(crate) fn build_embedded_parser_data(
    data: &RecognizerCodegenData<'_>,
    type_name: &str,
    grammar_name: &str,
    options: ParserRenderOptions<'_>,
) -> io::Result<ParserSurfaceBindings> {
    let model = structural_embedded_model(data, true)?;
    let antlr4rust_token_alias_module =
        antlr4rust_token_alias_module_name(&model.parser_members.module_symbols);
    let antlr4rust_context_wrapper =
        antlr4rust_context_wrapper_name(&model.parser_members.module_symbols);
    let antlr4rust_input_facade =
        antlr4rust_input_facade_name(&model.parser_members.module_symbols);
    let antlr4rust_token_view = antlr4rust_token_view_name(&model.parser_members.module_symbols);
    let antlr4rust_names = embedded::Antlr4RustNames {
        token_alias_module: &antlr4rust_token_alias_module,
        context_wrapper: &antlr4rust_context_wrapper,
        input_facade: &antlr4rust_input_facade,
    };
    let context_names = context_surface_names(&model);
    let token_types: BTreeMap<String, i32> = data
        .symbolic_names
        .iter()
        .enumerate()
        .filter_map(|(token_type, name)| {
            let name = name.as_ref()?;
            i32::try_from(token_type)
                .ok()
                .map(|token_type| (name.clone(), token_type))
        })
        .collect();
    let finish_body = |body: &str, translated: &str| -> String {
        post_process_embedded(body, translated, type_name)
    };

    let mut out = ParserSurfaceBindings {
        rule_has_attrs: model
            .rules
            .iter()
            .map(embedded::RuleModel::has_attrs)
            .collect(),
        ..ParserSurfaceBindings::default()
    };
    let mut uses_antlr4rust_input = false;
    let mut antlr4rust_context_roots = BTreeSet::new();
    let mut antlr4rust_token_aliases = BTreeMap::new();
    let mut antlr4rust_direct_alias_imports = BTreeSet::new();
    let mut antlr4rust_alias_inventory_cache = BTreeMap::new();

    for action in structural_actions(data)? {
        if action.body.trim().is_empty() {
            out.inline_actions.insert(action.state, String::new());
            continue;
        }
        let ctx = embedded::TranslationCtx {
            model: &model,
            rule_index: action.rule_index,
            body_offset: Some(
                usize::try_from(action.span.bytes.start).expect("source offset exceeds usize"),
            ),
            site: embedded::ActionSite::Body,
            token_types: &token_types,
        };
        let aliases = antlr4rust_alias_inventory_cache
            .entry(action.span.source)
            .or_insert_with(|| {
                antlr4rust_token_alias_inventory(data, type_name, action.span.source)
            });
        let translated = embedded::translate_parser_body_with_alias_module(
            &action.body,
            &ctx,
            &context_names.rules[action.rule_index].context_type,
            &aliases.names,
            antlr4rust_names,
            embedded::ParserBodyKind::Action,
        )
        .map_err(|error| {
            embedded_body_translation_error(
                data,
                &action.span,
                "parser action",
                action.rule_index,
                action.action_index,
                &error,
            )
        })?;
        record_antlr4rust_translation(
            &translated,
            aliases,
            action.rule_index,
            &mut uses_antlr4rust_input,
            &mut antlr4rust_context_roots,
            &mut antlr4rust_token_aliases,
        );
        out.inline_actions
            .insert(action.state, finish_body(&action.body, &translated.source));
    }

    for predicate in structural_predicates(data)? {
        let ctx = embedded::TranslationCtx {
            model: &model,
            rule_index: predicate.rule_index,
            body_offset: Some(
                usize::try_from(predicate.span.bytes.start).expect("source offset exceeds usize"),
            ),
            site: embedded::ActionSite::Body,
            token_types: &token_types,
        };
        let aliases = antlr4rust_alias_inventory_cache
            .entry(predicate.span.source)
            .or_insert_with(|| {
                antlr4rust_token_alias_inventory(data, type_name, predicate.span.source)
            });
        let translated = embedded::translate_parser_body_with_alias_module(
            predicate.body.trim(),
            &ctx,
            &context_names.rules[predicate.rule_index].context_type,
            &aliases.names,
            antlr4rust_names,
            embedded::ParserBodyKind::Predicate,
        )
        .map_err(|error| {
            embedded_body_translation_error(
                data,
                &predicate.span,
                "parser predicate",
                predicate.rule_index,
                predicate.predicate_index,
                &error,
            )
        })?;
        record_antlr4rust_translation(
            &translated,
            aliases,
            predicate.rule_index,
            &mut uses_antlr4rust_input,
            &mut antlr4rust_context_roots,
            &mut antlr4rust_token_aliases,
        );
        out.predicates.insert(
            (predicate.rule_index, predicate.predicate_index),
            (
                finish_body(&predicate.body, &translated.source),
                predicate.fail.clone(),
            ),
        );
    }

    // Rule-header sections: `@init`, `@after`, and the rule exception
    // clauses (`catch` / `finally`) all translate identically; only the
    // action site and the diagnostic label differ.
    let semantic = data
        .semantic
        .expect("embedded parser data has semantic grammar");
    let mut translate_rule_section = |rule_index: usize,
                                      rule_name: &str,
                                      kind: &'static str,
                                      site: embedded::ActionSite,
                                      body: &str|
     -> io::Result<String> {
        let semantic_rule = semantic.unit.rules.iter().find(|semantic_rule| {
            semantic.recognizer.rule_numbers.get(&semantic_rule.id) == Some(&rule_index)
        });
        let rule_source = semantic_rule.map_or(semantic.unit.source, |semantic_rule| {
            semantic_rule.span.source
        });
        let aliases = antlr4rust_alias_inventory_cache
            .entry(rule_source)
            .or_insert_with(|| antlr4rust_token_alias_inventory(data, type_name, rule_source));
        let ctx = embedded::TranslationCtx {
            model: &model,
            rule_index,
            body_offset: None,
            site,
            token_types: &token_types,
        };
        let translated = embedded::translate_parser_body_with_alias_module(
            body,
            &ctx,
            &context_names.rules[rule_index].context_type,
            &aliases.names,
            antlr4rust_names,
            embedded::ParserBodyKind::Action,
        )
        .map_err(|error| {
            embedded_rule_action_translation_error(
                data,
                semantic_rule,
                kind,
                rule_index,
                rule_name,
                &error,
            )
        })?;
        record_antlr4rust_translation(
            &translated,
            aliases,
            rule_index,
            &mut uses_antlr4rust_input,
            &mut antlr4rust_context_roots,
            &mut antlr4rust_token_aliases,
        );
        Ok(post_process_embedded(body, &translated.source, type_name))
    };
    for (rule_index, rule) in model.rules.iter().enumerate() {
        if let Some(body) = &rule.init_body {
            let translated = translate_rule_section(
                rule_index,
                &rule.name,
                "init",
                embedded::ActionSite::Init,
                body,
            )?;
            out.init_entry.insert(rule_index, translated);
        }
        if let Some(body) = &rule.after_body {
            let translated = translate_rule_section(
                rule_index,
                &rule.name,
                "after",
                embedded::ActionSite::After,
                body,
            )?;
            out.after.insert(rule_index, translated);
        }
        if let Some((binding, body)) = &rule.catch_clause {
            let translated = translate_rule_section(
                rule_index,
                &rule.name,
                "catch",
                embedded::ActionSite::After,
                body,
            )?;
            out.catch_clauses
                .insert(rule_index, (binding.clone(), translated));
        }
        if let Some(body) = &rule.finally_body {
            let translated = translate_rule_section(
                rule_index,
                &rule.name,
                "finally",
                embedded::ActionSite::After,
                body,
            )?;
            out.finally_bodies.insert(rule_index, translated);
        }
    }

    // Per-rule attrs structs exist only when generated actions need a payload.
    for (rule_index, rule) in model.rules.iter().enumerate() {
        if !rule.has_attrs() {
            continue;
        }
        let struct_name = embedded::attrs_struct_name(rule_index);
        let mut fields = String::new();
        for attr in &rule.attrs {
            let _ = writeln!(
                fields,
                "    pub {}: {},",
                embedded::escape_keyword(&attr.name),
                attr.ty
            );
        }
        let _ = writeln!(
            out.attrs_structs,
            "#[derive(Clone, Debug, Default)]\n#[allow(non_snake_case, dead_code)]\npub struct {struct_name} {{\n{fields}}}\n"
        );
    }

    // Members: fields, impl items, module items.
    out.struct_fields
        .push_str("    __embedded_pending_arg: Option<i64>,\n");
    out.field_inits
        .push_str("            __embedded_pending_arg: None,\n");
    for field in &model.parser_members.fields {
        let aliases = antlr4rust_alias_inventory_cache
            .entry(field.source)
            .or_insert_with(|| antlr4rust_token_alias_inventory(data, type_name, field.source));
        let translated_type = embedded::translate_member_field_type_token_aliases(
            &field.ty,
            &aliases.names,
            &antlr4rust_token_alias_module,
        )
        .map_err(|error| {
            embedded_member_translation_error(
                data,
                field.source,
                "parser member field type",
                &error,
            )
        })?;
        let translated_init = embedded::translate_member_token_aliases(
            &field.init,
            &aliases.names,
            &antlr4rust_token_alias_module,
        )
        .map_err(|error| {
            embedded_member_translation_error(
                data,
                field.source,
                "parser member field initializer",
                &error,
            )
        })?;
        for translated in [&translated_type, &translated_init] {
            antlr4rust_token_aliases.extend(
                translated.token_aliases.iter().filter_map(|name| {
                    aliases.values.get(name).map(|value| (name.clone(), *value))
                }),
            );
        }
        for attribute in field.attributes.lines() {
            let _ = writeln!(out.struct_fields, "    {attribute}");
        }
        for attribute in embedded::member_field_initializer_attributes(&field.attributes).lines() {
            let _ = writeln!(out.field_inits, "            {attribute}");
        }
        let _ = writeln!(
            out.struct_fields,
            "    {}: {},",
            field.name, translated_type.source
        );
        let _ = writeln!(
            out.field_inits,
            "            {}: {},",
            field.name, translated_init.source
        );
    }
    for item in &model.parser_members.impl_items {
        let aliases = antlr4rust_alias_inventory_cache
            .entry(item.source)
            .or_insert_with(|| antlr4rust_token_alias_inventory(data, type_name, item.source));
        let translated = embedded::translate_member_token_aliases(
            &item.body,
            &aliases.names,
            &antlr4rust_token_alias_module,
        )
        .map_err(|error| {
            embedded_member_translation_error(data, item.source, "parser member impl item", &error)
        })?;
        antlr4rust_token_aliases.extend(
            translated
                .token_aliases
                .iter()
                .filter_map(|name| aliases.values.get(name).map(|value| (name.clone(), *value))),
        );
        let item = post_process_embedded(&item.body, &translated.source, type_name);
        let mut indented = String::with_capacity(item.len());
        for (line_index, line) in item.lines().enumerate() {
            if line_index > 0 {
                indented.push_str("\n    ");
            }
            indented.push_str(line);
        }
        let _ = writeln!(out.impl_items, "    {indented}\n");
    }
    for item in &model.parser_members.module_items {
        let aliases = antlr4rust_alias_inventory_cache
            .entry(item.source)
            .or_insert_with(|| antlr4rust_token_alias_inventory(data, type_name, item.source));
        let translated = embedded::translate_member_token_aliases(
            &item.body,
            &aliases.names,
            &antlr4rust_token_alias_module,
        )
        .map_err(|error| {
            embedded_member_translation_error(
                data,
                item.source,
                "parser member module item",
                &error,
            )
        })?;
        antlr4rust_direct_alias_imports.extend(translated.direct_alias_imports.iter().cloned());
        antlr4rust_token_aliases.extend(
            translated
                .token_aliases
                .iter()
                .filter_map(|name| aliases.values.get(name).map(|value| (name.clone(), *value))),
        );
        let item = post_process_embedded(&item.body, &translated.source, type_name);
        let _ = writeln!(out.module_items, "{item}\n");
    }

    for (items, out_slot, kind) in [
        (
            &model.header_items,
            &mut out.header_items,
            "parser @header item",
        ),
        (
            &model.definitions_items,
            &mut out.definitions_items,
            "parser @definitions item",
        ),
    ] {
        for item in items {
            let aliases = antlr4rust_alias_inventory_cache
                .entry(item.source)
                .or_insert_with(|| antlr4rust_token_alias_inventory(data, type_name, item.source));
            let translated = embedded::translate_member_token_aliases(
                &item.body,
                &aliases.names,
                &antlr4rust_token_alias_module,
            )
            .map_err(|error| embedded_member_translation_error(data, item.source, kind, &error))?;
            antlr4rust_direct_alias_imports.extend(translated.direct_alias_imports.iter().cloned());
            antlr4rust_token_aliases.extend(
                translated.token_aliases.iter().filter_map(|name| {
                    aliases.values.get(name).map(|value| (name.clone(), *value))
                }),
            );
            // No `post_process_embedded`: its `TParser::` -> `Self::` rewrite
            // targets bodies inside the generated impl, and `Self` does not
            // exist at module scope where these items are emitted.
            let _ = writeln!(out_slot, "{}\n", translated.source);
        }
    }

    // Rule-call argument expressions attach to the exact finalized transition
    // produced from each structural call element.
    out.call_args = structural_embedded_rule_call_args(data)?;
    out.rule_arg0 = model
        .rules
        .iter()
        .map(|rule| {
            rule.arg_names
                .first()
                .map(|name| embedded::escape_keyword(name))
        })
        .collect();

    // Associated token constants: rendered bodies reference tokens as
    // `TParser::NL` (post-processed to `Self::NL`).
    for (name, token_type) in &token_types {
        let _ = writeln!(
            out.impl_items,
            "    #[allow(dead_code)]\n    pub const {name}: i32 = {token_type};"
        );
    }
    out.impl_items.push('\n');

    // Recognizer-surface facades the rendered bodies call.
    out.impl_items.push_str(&embedded_parser_facades());
    if uses_antlr4rust_input {
        out.module_items.push_str(&render_antlr4rust_input_facade(
            &antlr4rust_input_facade,
            &antlr4rust_token_view,
        ));
    }
    if !antlr4rust_token_aliases.is_empty() {
        out.module_items.push_str(&render_antlr4rust_token_aliases(
            &antlr4rust_token_aliases,
            &model.parser_members.module_symbol_cfgs,
            &model.parser_members.module_import_cfgs,
            &antlr4rust_direct_alias_imports,
            &antlr4rust_token_alias_module,
        ));
    }
    let antlr4rust_context_rules =
        antlr4rust_compatibility_rules(&model, &antlr4rust_context_roots);
    out.module_items.push_str(&render_embedded_context_types(
        grammar_name,
        data,
        &model,
        options,
        &antlr4rust_context_rules,
        &antlr4rust_context_wrapper,
    )?);
    Ok(out)
}

fn embedded_member_translation_error(
    data: &RecognizerCodegenData<'_>,
    source: SourceId,
    kind: &str,
    error: &io::Error,
) -> io::Error {
    let path = data
        .sources
        .and_then(|sources| sources.logical_path(source))
        .map_or_else(|| "<grammar>".to_owned(), |path| path.display().to_string());
    io::Error::new(
        error.kind(),
        format!("{path}: cannot lower embedded {kind}: {error}"),
    )
}

fn embedded_named_body_translation_error(
    data: &RecognizerCodegenData<'_>,
    span: &SourceSpan,
    kind: &str,
    rule_index: usize,
    error: &io::Error,
) -> io::Error {
    let path = data
        .sources
        .and_then(|sources| sources.logical_path(span.source))
        .map_or_else(|| "<grammar>".to_owned(), |path| path.display().to_string());
    let (line, column) = structural_line_column(data, span);
    let rule = data
        .rule_names
        .get(rule_index)
        .map_or("<unknown>", String::as_str);
    io::Error::new(
        error.kind(),
        format!(
            "{path}:{line}:{column}: cannot lower embedded {kind} in rule {rule} \
             ({rule_index}): {error}"
        ),
    )
}

fn embedded_rule_action_translation_error(
    data: &RecognizerCodegenData<'_>,
    semantic_rule: Option<&Rule>,
    action_name: &str,
    rule_index: usize,
    rule_name: &str,
    error: &io::Error,
) -> io::Error {
    let label = match action_name {
        "catch" => "parser catch clause".to_owned(),
        "finally" => "parser finally clause".to_owned(),
        other => format!("parser @{other}"),
    };
    embedded_rule_section_span(semantic_rule, action_name).map_or_else(
        || {
            io::Error::new(
                error.kind(),
                format!(
                    "cannot lower embedded {label} body for parser rule {rule_name} \
                     ({rule_index}): {error}"
                ),
            )
        },
        |span| embedded_named_body_translation_error(data, span, &label, rule_index, error),
    )
}

/// The authored source span of one rule-header section: a named action
/// (`@init` / `@after`), the rule's `catch` clause, or its `finally` clause.
fn embedded_rule_section_span<'r>(
    semantic_rule: Option<&'r Rule>,
    action_name: &str,
) -> Option<&'r SourceSpan> {
    let rule = semantic_rule?;
    match action_name {
        "catch" => rule.catches.first().map(|handler| &handler.body_span),
        "finally" => rule
            .finally_action
            .as_ref()
            .map(|action| &action.body_span),
        _ => rule
            .actions
            .iter()
            .find(|action| action.name == action_name)
            .map(|action| &action.body_span),
    }
}

fn embedded_context_accessor_translation_error(
    data: &RecognizerCodegenData<'_>,
    rule_index: usize,
    error: &io::Error,
) -> io::Error {
    let semantic_rule = data.semantic.and_then(|semantic| {
        semantic
            .unit
            .rules
            .iter()
            .find(|rule| semantic.recognizer.rule_numbers.get(&rule.id) == Some(&rule_index))
    });
    semantic_rule.map_or_else(
        || {
            let rule_name = data
                .rule_names
                .get(rule_index)
                .map_or("<unknown>", String::as_str);
            io::Error::new(
                error.kind(),
                format!(
                    "cannot lower embedded antlr4rust compatibility accessors in rule \
                     {rule_name} ({rule_index}): {error}"
                ),
            )
        },
        |rule| {
            embedded_named_body_translation_error(
                data,
                &rule.name_span,
                "antlr4rust compatibility accessors",
                rule_index,
                error,
            )
        },
    )
}

/// Parser modules carry the versioned packed ATN separately, so retaining the
/// legacy serialized integer stream in metadata would duplicate the artifact.
pub(crate) fn render_parser_metadata(grammar_name: &str, data: &ParserCodegenData<'_>) -> String {
    render_metadata_with_atn(grammar_name, data, &[], &[], &[])
}

fn render_metadata_with_atn(
    grammar_name: &str,
    data: &RecognizerCodegenData<'_>,
    channel_names: &[String],
    mode_names: &[String],
    serialized_atn: &[i32],
) -> String {
    format!(
        "pub static METADATA: GrammarMetadata = GrammarMetadata::new(\n    \"{}\",\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n);\n\npub fn metadata() -> &'static GrammarMetadata {{\n    &METADATA\n}}\n\npub fn rule_names() -> &'static [&'static str] {{\n    METADATA.rule_names()\n}}\n",
        rust_string(grammar_name),
        render_str_slice(&data.rule_names),
        render_option_str_slice(&data.literal_names),
        render_option_str_slice(&data.symbolic_names),
        render_empty_option_str_slice(max_len(&data.literal_names, &data.symbolic_names)),
        render_str_slice(channel_names),
        render_str_slice(mode_names),
        render_i32_slice(serialized_atn)
    )
}

/// Metadata-derived `<GeneratedParserType>_<TOKEN>` aliases used by
/// antlr4rust embedded bodies.
fn antlr4rust_token_alias_name(type_name: &str, token_name: &str) -> String {
    sanitize_identifier(&format!("{type_name}_{token_name}"))
}

fn antlr4rust_implicit_token_aliases(
    type_name: &str,
    data: &RecognizerCodegenData<'_>,
    source: SourceId,
) -> Vec<(String, i32)> {
    let Some(semantic) = data.semantic else {
        return Vec::new();
    };
    let vocabulary = &semantic.recognizer.vocabulary;
    let source_literals = if source == semantic.unit.source {
        None
    } else {
        data.sources
            .and_then(|sources| sources.get(source))
            .and_then(source_implicit_token_literals)
    };
    if let Some(literals) = source_literals {
        return literals
            .iter()
            .enumerate()
            .filter_map(|(index, literal)| {
                let token_type = vocabulary.by_literal.get(literal)?;
                Some((
                    antlr4rust_token_alias_name(type_name, &format!("T__{index}")),
                    *token_type,
                ))
            })
            .collect();
    }
    vocabulary
        .name_order
        .iter()
        .filter(|name| name.starts_with("T__"))
        .map(|name| {
            (
                antlr4rust_token_alias_name(type_name, name),
                vocabulary.by_name[name],
            )
        })
        .collect()
}

fn antlr4rust_token_alias_values(
    type_name: &str,
    data: &RecognizerCodegenData<'_>,
    source: SourceId,
) -> BTreeMap<String, i32> {
    let eof = antlr4rust_token_alias_name(type_name, "EOF");
    let mut aliases = BTreeMap::from([(eof, TOKEN_EOF)]);
    for (alias, token_type) in antlr4rust_implicit_token_aliases(type_name, data, source) {
        aliases.entry(alias).or_insert(token_type);
    }
    for (token_type, name) in data.symbolic_names.iter().enumerate() {
        let Some(name) = name else { continue };
        let alias = antlr4rust_token_alias_name(type_name, name);
        let token_type = i32::try_from(token_type).expect("token type exceeds i32");
        aliases.entry(alias).or_insert(token_type);
    }
    aliases
}

fn antlr4rust_token_alias_module_name(member_symbols: &BTreeSet<String>) -> String {
    antlr4rust_compatibility_symbol_name(embedded::ANTLR4RUST_TOKEN_ALIAS_MODULE, member_symbols)
}

fn antlr4rust_context_wrapper_name(member_symbols: &BTreeSet<String>) -> String {
    antlr4rust_compatibility_symbol_name(embedded::ANTLR4RUST_CONTEXT_WRAPPER, member_symbols)
}

fn antlr4rust_input_facade_name(member_symbols: &BTreeSet<String>) -> String {
    antlr4rust_compatibility_symbol_name(embedded::ANTLR4RUST_INPUT_FACADE, member_symbols)
}

fn antlr4rust_token_view_name(member_symbols: &BTreeSet<String>) -> String {
    antlr4rust_compatibility_symbol_name(embedded::ANTLR4RUST_TOKEN_VIEW, member_symbols)
}

fn antlr4rust_compatibility_symbol_name(stem: &str, member_symbols: &BTreeSet<String>) -> String {
    if !member_symbols.contains(stem) {
        return stem.to_owned();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{stem}_{suffix}");
        if !member_symbols.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn render_antlr4rust_token_aliases(
    aliases: &BTreeMap<String, i32>,
    member_symbol_cfgs: &BTreeMap<String, Vec<Vec<String>>>,
    member_import_cfgs: &BTreeMap<String, Vec<Vec<String>>>,
    direct_alias_imports: &BTreeSet<String>,
    module_name: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "#[allow(non_snake_case, dead_code, unused_imports)]\nmod {module_name} {{"
    );
    let imported_aliases = aliases
        .iter()
        .filter(|(alias, _)| {
            !direct_alias_imports.contains(*alias) && member_import_cfgs.contains_key(*alias)
        })
        .collect::<Vec<_>>();
    if !imported_aliases.is_empty() {
        out.push_str("    mod __fallback {\n");
        for (alias, token_type) in imported_aliases {
            let value = if *token_type == TOKEN_EOF {
                "antlr4_runtime::TOKEN_EOF".to_owned()
            } else {
                token_type.to_string()
            };
            let _ = writeln!(
                out,
                "        #[allow(non_upper_case_globals)]\n        \
                 pub(crate) const {alias}: i32 = {value};"
            );
        }
        out.push_str("    }\n    pub(super) use __fallback::*;\n");
    }
    for (alias, token_type) in aliases {
        let value = if *token_type == TOKEN_EOF {
            "antlr4_runtime::TOKEN_EOF".to_owned()
        } else {
            token_type.to_string()
        };
        let value_declarations = (!direct_alias_imports.contains(alias))
            .then(|| member_symbol_cfgs.get(alias))
            .flatten();
        let import_declarations = (!direct_alias_imports.contains(alias))
            .then(|| member_import_cfgs.get(alias))
            .flatten();
        let declarations = value_declarations
            .into_iter()
            .flatten()
            .chain(import_declarations.into_iter().flatten())
            .collect::<Vec<_>>();
        if declarations
            .iter()
            .any(|declaration| declaration.is_empty())
        {
            let _ = writeln!(out, "    pub(super) use super::{alias};");
            continue;
        }
        let conditions = declarations
            .iter()
            .map(|predicates| match predicates.as_slice() {
                [predicate] => predicate.clone(),
                predicates => format!("all({})", predicates.join(", ")),
            })
            .collect::<BTreeSet<_>>();
        for condition in &conditions {
            let _ = writeln!(
                out,
                "    #[cfg({condition})]\n    pub(super) use super::{alias};"
            );
        }
        if import_declarations.is_some() {
            continue;
        }
        if !conditions.is_empty() {
            let active = if conditions.len() == 1 {
                conditions.first().expect("non-empty condition set").clone()
            } else {
                format!(
                    "any({})",
                    conditions.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            };
            let _ = writeln!(out, "    #[cfg(not({active}))]");
        }
        let _ = writeln!(
            out,
            "    #[allow(non_upper_case_globals)]\n    pub(super) const {alias}: i32 = {value};"
        );
    }
    out.push_str("}\n\n");
    out
}

/// Renders rule-index constants from grammar rule names. Rule names are
/// unique in the grammar, but their upper-snake mangling is lossy and the
/// literal `RULE_` prefix can itself collide with a token constant (token
/// `RuleFoo` vs rule `foo`), so allocation shares the generated module's
/// `used` identifier set and colliders get a numbered suffix.
pub(crate) fn render_rule_constants(
    data: &RecognizerCodegenData<'_>,
    used: &mut BTreeSet<String>,
) -> String {
    let mut out = String::new();
    for (index, name) in data.rule_names.iter().enumerate() {
        writeln!(
            out,
            "pub const {}: usize = {index};",
            allocate_const_name("RULE_", name, used)
        )
        .expect("writing to a string cannot fail");
    }
    out
}

/// Renders an `&[Option<&str>]` expression for literal or symbolic names.
fn render_option_str_slice(values: &[Option<String>]) -> String {
    let items = values
        .iter()
        .map(|value| {
            value.as_ref().map_or_else(
                || "None".to_owned(),
                |value| format!("Some(\"{}\")", rust_string(value)),
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

/// Renders an empty optional string table with a fixed length.
fn render_empty_option_str_slice(len: usize) -> String {
    let items = (0..len).map(|_| "None").collect::<Vec<_>>().join(", ");
    format!("[{items}]")
}

/// Renders an `&[&str]` expression for rule/channel/mode names.
fn render_str_slice(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| format!("\"{}\"", rust_string(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

/// Renders a line-wrapped `&[i32]` expression for serialized ATN data.
fn render_i32_slice(values: &[i32]) -> String {
    let items = values
        .iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

/// Renders an inline `[(i32, i32); N]` expression for generated token-set
/// matches.
pub(crate) fn render_i32_ranges(values: &[(i32, i32)]) -> String {
    let items = values
        .iter()
        .map(|(start, stop)| format!("({start}, {stop})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

pub(crate) fn render_i32_match_patterns(values: &[(i32, i32)]) -> String {
    values
        .iter()
        .map(|(start, stop)| {
            if start == stop {
                start.to_string()
            } else {
                format!("{start}..={stop}")
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}
