/// Renders a Rust lexer module that delegates token recognition to the shared
/// ATN interpreter.
///
/// The emitted lexer owns only generated metadata and a `BaseLexer`. Keeping
/// recognition in the runtime avoids emitting thousands of lines of
/// grammar-specific Rust control flow for the first target implementation.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn render_lexer(
    grammar_name: &str,
    data: &LexerCodegenData<'_>,
    allow_unsupported_lexer_actions: bool,
    sem_unknown: SemUnknownPolicy,
    patterns: &SemPatternFile,
    embedded: bool,
) -> io::Result<String> {
    render_lexer_model(&LexerRenderModel::new(
        grammar_name,
        data,
        allow_unsupported_lexer_actions,
        sem_unknown,
        patterns,
        embedded,
    ))
}

/// Emits Rust from a complete lexer render model.
pub(crate) fn render_lexer_model(model: &LexerRenderModel<'_>) -> io::Result<String> {
    let grammar_name = model.grammar_name;
    let data = model.data;
    let allow_unsupported_lexer_actions = model.allow_unsupported_lexer_actions;
    let sem_unknown = model.sem_unknown;
    let patterns = model.patterns;
    let embedded = model.embedded;
    let type_name = rust_type_name(grammar_name);
    let metadata = render_lexer_metadata(grammar_name, data);
    let token_constants = render_lexer_token_constants(data);
    let lexer_state_constants = render_lexer_state_constants(data);
    // Embedded mode: lexer action/predicate bodies are verbatim Rust from the
    // rendered grammar; translate the recognizer surface textually and skip
    // the template machinery entirely.
    let embedded_lexer_actions = if embedded {
        structural_embedded_lexer_actions(data)?
    } else {
        Vec::new()
    };
    let embedded_lexer_predicates = if embedded {
        structural_embedded_lexer_predicates(data)?
    } else {
        Vec::new()
    };
    let mut actions = if embedded {
        Vec::new()
    } else {
        let actions = structural_lexer_action_templates(data, patterns)?;
        reject_unsupported_lexer_action_templates(
            &actions
                .iter()
                .map(|(_, template)| template.clone())
                .collect::<Vec<_>>(),
            allow_unsupported_lexer_actions,
        )?;
        actions
    };
    // Any per-coordinate override replaces a translated action. Hook overrides
    // fall through to the semantic hook object; assume-* overrides are no-ops.
    actions.retain(|((rule_index, action_index), _)| {
        let rule_name = usize::try_from(*rule_index)
            .ok()
            .and_then(|rule| data.rule_names.get(rule).map(String::as_str));
        patterns
            .coordinate_override(
                SemanticsKind::LexerAction,
                rule_name,
                usize::try_from(*action_index).ok(),
                None,
            )
            .is_none()
    });
    let mut predicates = if embedded {
        Vec::new()
    } else {
        structural_predicate_templates(data, SemanticsKind::LexerPredicate, patterns)?
    };
    // Apply per-coordinate overrides to every lexer predicate transition. Hook
    // coordinates are represented explicitly so generated dispatch can fall
    // through to the caller-owned semantic hook object.
    for coordinate in lexer_predicate_transitions(data) {
        let (rule_index, pred_index) = coordinate;
        let dispose = patterns
            .coordinate_override(
                SemanticsKind::LexerPredicate,
                data.rule_names.get(rule_index).map(String::as_str),
                Some(pred_index),
                None,
            )
            .map(|override_| override_.dispose);
        let covered = predicates.iter().any(|(pred, _)| *pred == coordinate);
        match dispose {
            Some(CoordinateDispose::Hook) => {
                set_lexer_predicate_template(&mut predicates, coordinate, PredicateTemplate::Hook);
            }
            Some(CoordinateDispose::Error) => {}
            Some(CoordinateDispose::AssumeFalse) => {
                set_lexer_predicate_template(&mut predicates, coordinate, PredicateTemplate::False);
            }
            Some(CoordinateDispose::AssumeTrue) => {
                set_lexer_predicate_template(&mut predicates, coordinate, PredicateTemplate::True);
            }
            None => {
                if !covered && sem_unknown == SemUnknownPolicy::Hook {
                    set_lexer_predicate_template(
                        &mut predicates,
                        coordinate,
                        PredicateTemplate::Hook,
                    );
                }
            }
        }
    }
    // Predicate coordinates with no translated template normally fall back to
    // always-true; `--sem-unknown=assume-false` flips that fallback, which
    // requires the hook-taking token path even when no dispatch is generated.
    let unknown_predicates_assume_false = sem_unknown == SemUnknownPolicy::AssumeFalse
        && predicates.is_empty()
        && !lexer_predicate_transitions(data).is_empty();
    let lexer_dfa_data = compiled_lexer_dfa_words(data);
    let lexer_typed_hook_mappings = if embedded {
        Vec::new()
    } else {
        lexer_typed_hook_mappings(data, patterns, &actions)?
    };
    let needs_typed_hook_adapter =
        !lexer_typed_hook_mappings.is_empty() || uses_lexer_superclass(data);
    let typed_hook_adapter = if needs_typed_hook_adapter {
        render_lexer_typed_hook_adapter(&type_name, &lexer_typed_hook_mappings)
    } else {
        String::new()
    };
    let typed_lexer_constructor = if needs_typed_hook_adapter {
        let trait_name = format!("{type_name}Hooks");
        let adapter_name = format!("{type_name}TypedHooks");
        format!(
            "impl<I, T> {type_name}<I, {adapter_name}<T>>\nwhere\n    I: CharStream,\n    T: {trait_name},\n{{\n    pub fn with_typed_hooks(input: I, hooks: T) -> Self {{\n        Self::with_hooks(input, {adapter_name}::new(hooks))\n    }}\n}}\n"
        )
    } else {
        String::new()
    };
    let action_coordinates = lexer_custom_actions(data);
    let has_hook_disposed_actions = lexer_actions_require_semantic_hooks(
        &action_coordinates,
        &data.rule_names,
        &actions,
        patterns,
        sem_unknown,
    );
    let has_semantic_hooks = has_hook_disposed_actions
        || !lexer_typed_hook_mappings.is_empty()
        || actions
            .iter()
            .any(|(_, template)| matches!(template, ActionTemplate::Hook(_)))
        || predicates
            .iter()
            .any(|(_, template)| matches!(template, PredicateTemplate::Hook));
    let has_action_dispatch = if embedded {
        embedded_lexer_actions
            .iter()
            .any(|(_, statement)| !statement.trim().is_empty())
    } else {
        lexer_actions_need_dispatch(&actions)
    };
    let action_method = if embedded {
        render_embedded_lexer_action_method(&embedded_lexer_actions)
    } else {
        render_lexer_action_method(&actions)
    };
    let predicate_method = if embedded {
        render_embedded_lexer_predicate_method(&embedded_lexer_predicates)
    } else {
        render_lexer_predicate_method(&predicates, sem_unknown)
    };
    // Member-state coordinates (issue #206) evaluate from a `SemIR` table
    // rather than inline Rust, so they need no hand-written hook.
    let lexer_semantics_function = if embedded {
        String::new()
    } else {
        render_lexer_semantics_function(&predicates, &actions)
    };
    // A grammar-declared initializer (`private bool verbatium = true;`) must
    // seed its slot, or a predicate reading it would reject input the source
    // grammar accepts. `with_initial_members` also retains the values so every
    // reset restores them instead of zeroing.
    let lexer_member_inits =
        match render_member_init_seeds(patterns, stack_member::MemberScope::Lexer)? {
            seeds if embedded || seeds.is_empty() => String::new(),
            seeds => format!(".with_initial_members({seeds})"),
        };
    let has_predicate_dispatch = if embedded {
        !embedded_lexer_predicates.is_empty()
    } else {
        !predicates.is_empty()
    };
    let lexer_unknown_policy = match sem_unknown {
        SemUnknownPolicy::AssumeTrue => "antlr4_runtime::UnknownSemanticPolicy::AssumeTrue",
        SemUnknownPolicy::AssumeFalse => "antlr4_runtime::UnknownSemanticPolicy::AssumeFalse",
        SemUnknownPolicy::Hook | SemUnknownPolicy::Error => {
            "antlr4_runtime::UnknownSemanticPolicy::Error"
        }
    };
    let semantic_action = if has_action_dispatch {
        "Self::run_action"
    } else {
        "|_, _| false"
    };
    let semantic_predicate = if has_predicate_dispatch {
        "Self::run_predicate"
    } else {
        "|_, _| None"
    };
    let lifecycle_next_token_call = format!(
        "antlr4_runtime::atn::lexer::next_token_compiled_with_semantic_dispatch(&mut lexer.base, sink, atn(), lexer_dfa(), &mut lexer.hooks, {semantic_action}, {semantic_predicate}, {lexer_unknown_policy}, |_, _, _| {{}})"
    );
    let next_token_call = if has_semantic_hooks {
        lifecycle_next_token_call.clone()
    } else if !has_action_dispatch && !has_predicate_dispatch && !unknown_predicates_assume_false {
        "antlr4_runtime::atn::lexer::next_token_compiled(&mut lexer.base, sink, atn(), lexer_dfa())"
            .to_owned()
    } else {
        let action = if has_action_dispatch {
            "|base, action| { let _ = Self::run_action(base, action); }"
        } else {
            "|_, _| {}"
        };
        let predicate = if has_predicate_dispatch {
            "|base, predicate| Self::run_predicate(base, predicate).unwrap_or(true)"
        } else if unknown_predicates_assume_false {
            "|_, _| false"
        } else {
            "|_, _| true"
        };
        format!(
            "antlr4_runtime::atn::lexer::next_token_compiled_with_hooks(&mut lexer.base, sink, atn(), lexer_dfa(), {action}, {predicate}, |_, _, _| {{}})"
        )
    };
    let next_token_call = if has_semantic_hooks {
        next_token_call
    } else {
        format!(
            "if H::ENABLES_LEXER_LIFECYCLE {{\n            {lifecycle_next_token_call}\n        }} else {{\n            {next_token_call}\n        }}"
        )
    };
    let generated_header = generated_module_header();
    let generated_footer = GENERATED_MODULE_FOOTER;

    Ok(format!(
        r#"{generated_header}use antlr4_runtime::char_stream::CharStream;
use antlr4_runtime::atn::LexerAtn;
use antlr4_runtime::atn::lexer_dfa::CompiledLexerDfa;
use antlr4_runtime::atn::serialized::AtnDeserializer;
use antlr4_runtime::{{BaseLexer, GrammarMetadata, Lexer}};
use std::sync::OnceLock;

{token_constants}
{lexer_state_constants}
{metadata}
{typed_hook_adapter}

static ATN_CELL: OnceLock<LexerAtn> = OnceLock::new();

/// Deserializes and caches the grammar ATN for all lexer instances.
fn atn() -> &'static LexerAtn {{
    ATN_CELL.get_or_init(|| {{
        let serialized = metadata().serialized_atn();
        AtnDeserializer::new(&serialized)
            .deserialize()
            .expect("generated lexer contains a valid ANTLR serialized ATN")
    }})
}}

static LEXER_DFA_DATA: &[u32] = &[{lexer_dfa_data}];

static LEXER_DFA_CELL: OnceLock<CompiledLexerDfa> = OnceLock::new();

/// Ahead-of-time lexer DFA tables compiled by antlr4-rust-gen, embedded so
/// runtime startup only deserializes them. Rebuilt from the ATN instead when
/// the embedded stream comes from a different runtime version.
fn lexer_dfa() -> &'static CompiledLexerDfa {{
    LEXER_DFA_CELL.get_or_init(|| {{
        CompiledLexerDfa::from_serialized(LEXER_DFA_DATA)
            .unwrap_or_else(|| CompiledLexerDfa::compile(atn()))
    }})
}}
{lexer_semantics_function}
#[derive(Clone, Debug)]
pub struct {type_name}<I, H = antlr4_runtime::NoSemanticHooks>
where
    I: CharStream,
    H: antlr4_runtime::SemanticHooks,
{{
    base: BaseLexer<I>,
    hooks: H,
}}

impl<I> {type_name}<I, antlr4_runtime::NoSemanticHooks>
where
    I: CharStream,
{{
    pub fn new(input: I) -> Self {{
        Self::with_hooks(input, antlr4_runtime::NoSemanticHooks)
    }}
}}

impl<I, H> {type_name}<I, H>
where
    I: CharStream,
    H: antlr4_runtime::SemanticHooks,
{{
    pub fn with_hooks(input: I, hooks: H) -> Self {{
        let grammar_metadata = metadata();
        let data = grammar_metadata.recognizer_data();
        Self {{ base: BaseLexer::new(input, data).with_shared_dfa(atn()){lexer_member_inits}, hooks }}
    }}

{action_method}
{predicate_method}
}}

{typed_lexer_constructor}

antlr4_runtime::__antlr4_rust_lexer_facade! {{
    type: {type_name}<I, H>,
    fields: {{
        base: base,
        hooks: hooks,
    }},
    metadata: metadata,
    next_token(lexer, sink) {{
        {next_token_call}
    }}
}}
{generated_footer}"#
    ))
}

pub(crate) fn lexer_actions_require_semantic_hooks(
    coordinates: &[(i32, i32)],
    rule_names: &[String],
    actions: &[((i32, i32), ActionTemplate)],
    patterns: &SemPatternFile,
    sem_unknown: SemUnknownPolicy,
) -> bool {
    coordinates.iter().any(|coordinate| {
        let (rule_index, action_index) = *coordinate;
        let rule_name = usize::try_from(rule_index)
            .ok()
            .and_then(|rule| rule_names.get(rule).map(String::as_str));
        if let Some(override_) = patterns.coordinate_override(
            SemanticsKind::LexerAction,
            rule_name,
            usize::try_from(action_index).ok(),
            None,
        ) {
            return override_.dispose == CoordinateDispose::Hook;
        }
        if sem_unknown != SemUnknownPolicy::Hook {
            return false;
        }
        !matches!(
            actions
                .iter()
                .find(|(candidate, _)| candidate == coordinate)
                .map(|(_, template)| template),
            Some(ActionTemplate::LexerPopMode)
        )
    })
}

/// Compiles the lexer DFA at generation time and flattens it for embedding.
///
/// An empty stream makes the generated lexer fall back to compiling the DFA
/// from its ATN at first use, so generation never fails on this step.
fn compiled_lexer_dfa_words(data: &LexerCodegenData<'_>) -> String {
    if !data.lexer_dfa_words.is_empty() {
        return data
            .lexer_dfa_words
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
    }
    let atn = data.lexer_atn();
    let words = CompiledLexerDfa::compile(atn).serialize();
    let rendered: Vec<String> = words.iter().map(u32::to_string).collect();
    rendered.join(",")
}
/// Translates the lexer recognizer surface used by rendered test bodies onto
/// the `BaseLexer` hooks: `self.text()` / column accessors become position
/// -aware `_base` calls, `self.output()` becomes stdout.
fn translate_embedded_lexer_body(body: &str, position_expr: &str) -> io::Result<String> {
    embedded::validate_lexer_body_compatibility_receivers(body)?;
    let mut out = replace_all(body.trim(), "self.output()", "std::io::stdout()");
    out = replace_all(
        &out,
        "self.text()",
        &format!("_base.token_text_until({position_expr})"),
    );
    out = replace_all(
        &out,
        "self.char_position_in_line()",
        &format!("_base.column_at({position_expr})"),
    );
    out = replace_all(
        &out,
        "self.token_start_char_position_in_line()",
        "_base.token_start_column()",
    );
    if out.contains("self.") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported recognizer surface in embedded lexer body: {body}"),
        ));
    }
    Ok(out)
}

fn structural_embedded_lexer_actions(
    data: &RecognizerCodegenData<'_>,
) -> io::Result<Vec<((i32, i32), String)>> {
    let mut actions = structural_actions(data)?
        .into_iter()
        .map(|action| {
            let translated = translate_embedded_lexer_body(&action.body, "action.position()")
                .map_err(|error| {
                    embedded_body_translation_error(
                        data,
                        &action.span,
                        "lexer action",
                        action.rule_index,
                        action.action_index,
                        &error,
                    )
                })?;
            Ok((
                (
                    i32::try_from(action.rule_index).expect("rule index exceeds i32"),
                    i32::try_from(action.action_index).expect("action index exceeds i32"),
                ),
                translated,
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    actions.sort_by_key(|(coordinate, _)| *coordinate);
    actions.dedup_by_key(|(coordinate, _)| *coordinate);
    Ok(actions)
}

fn structural_embedded_lexer_predicates(
    data: &RecognizerCodegenData<'_>,
) -> io::Result<Vec<((usize, usize), String)>> {
    structural_predicates(data)?
        .into_iter()
        .map(|predicate| {
            let translated = translate_embedded_lexer_body(&predicate.body, "predicate.position()")
                .map_err(|error| {
                    embedded_body_translation_error(
                        data,
                        &predicate.span,
                        "lexer predicate",
                        predicate.rule_index,
                        predicate.predicate_index,
                        &error,
                    )
                })?;
            Ok((
                (predicate.rule_index, predicate.predicate_index),
                translated,
            ))
        })
        .collect()
}

fn render_embedded_lexer_action_method(actions: &[((i32, i32), String)]) -> String {
    if actions
        .iter()
        .all(|(_, statement)| statement.trim().is_empty())
    {
        return String::new();
    }
    let mut arms = String::new();
    for ((rule_index, action_index), statement) in actions {
        writeln!(
            arms,
            "            ({rule_index}, {action_index}) => {{ {statement} true }}"
        )
        .expect("writing to a string cannot fail");
    }
    arms.push_str("            _ => false,\n");
    format!(
        "    fn run_action(_base: &mut BaseLexer<I>, action: antlr4_runtime::LexerCustomAction) -> bool {{\n        #[allow(unused_imports)]\n        use std::io::Write as _;\n        match (action.rule_index(), action.action_index()) {{\n{arms}        }}\n    }}\n"
    )
}

fn render_embedded_lexer_predicate_method(predicates: &[((usize, usize), String)]) -> String {
    if predicates.is_empty() {
        return String::new();
    }
    let mut arms = String::new();
    for ((rule_index, pred_index), expression) in predicates {
        writeln!(
            arms,
            "            ({rule_index}, {pred_index}) => Some({{ {expression} }}),"
        )
        .expect("writing to a string cannot fail");
    }
    arms.push_str("            _ => None,\n");
    format!(
        "    fn run_predicate(_base: &BaseLexer<I>, predicate: antlr4_runtime::LexerPredicate) -> Option<bool> {{\n        match (predicate.rule_index(), predicate.pred_index()) {{\n{arms}        }}\n    }}\n"
    )
}
