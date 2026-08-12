// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
pub(crate) fn lexer_typed_hook_mappings(
    data: &RecognizerCodegenData<'_>,
    patterns: &SemPatternFile,
    actions: &[((i32, i32), ActionTemplate)],
) -> io::Result<Vec<LexerTypedHookMapping>> {
    let mut mappings = actions
        .iter()
        .filter_map(|((rule_index, action_index), template)| {
            let ActionTemplate::Hook(call) = template else {
                return None;
            };
            Some(LexerTypedHookMapping {
                rule_index: usize::try_from(*rule_index).ok()?,
                coordinate_index: usize::try_from(*action_index).ok()?,
                kind: LexerTypedHookKind::Action,
                method_name: rust_function_name(&call.name),
                call: call.clone(),
            })
        })
        .collect::<Vec<_>>();

    for predicate in structural_predicates(data)? {
        if let Some(call) =
            patterns.hook_helper_call(SemanticsKind::LexerPredicate, &predicate.body)?
        {
            mappings.push(LexerTypedHookMapping {
                rule_index: predicate.rule_index,
                coordinate_index: predicate.predicate_index,
                kind: LexerTypedHookKind::Predicate,
                method_name: rust_function_name(&call.name),
                call,
            });
        }
    }

    let predicate_names = mappings
        .iter()
        .filter(|mapping| mapping.kind == LexerTypedHookKind::Predicate)
        .map(|mapping| mapping.method_name.clone())
        .collect::<BTreeSet<_>>();
    let action_names = mappings
        .iter()
        .filter(|mapping| mapping.kind == LexerTypedHookKind::Action)
        .map(|mapping| mapping.method_name.clone())
        .collect::<BTreeSet<_>>();
    const RESERVED_METHODS: [&str; 4] = [
        "lexer_reset",
        "lexer_before_token",
        "lexer_after_accept",
        "token_emitted",
    ];
    for mapping in &mut mappings {
        if RESERVED_METHODS.contains(&mapping.method_name.as_str())
            || (predicate_names.contains(&mapping.method_name)
                && action_names.contains(&mapping.method_name))
        {
            mapping.method_name.push_str(match mapping.kind {
                LexerTypedHookKind::Predicate => "_pred",
                LexerTypedHookKind::Action => "_action",
            });
        }
    }
    validate_lexer_typed_hook_signatures(&mappings)?;
    mappings.sort_by_key(|mapping| {
        (
            mapping.rule_index,
            mapping.coordinate_index,
            matches!(mapping.kind, LexerTypedHookKind::Action),
        )
    });
    mappings.dedup();
    Ok(mappings)
}

pub(crate) fn validate_lexer_typed_hook_signatures(
    mappings: &[LexerTypedHookMapping],
) -> io::Result<()> {
    let mut signatures = BTreeMap::<(&str, LexerTypedHookKind), Vec<SemanticLiteralKind>>::new();
    for mapping in mappings {
        let signature = mapping
            .call
            .arguments
            .iter()
            .map(semantic_literal_kind)
            .collect::<Vec<_>>();
        match signatures.entry((&mapping.method_name, mapping.kind)) {
            Entry::Occupied(entry) if entry.get() != &signature => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "typed lexer semantic helper {} has conflicting literal signatures {:?} and {signature:?}",
                        mapping.call.name,
                        entry.get()
                    ),
                ));
            }
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                entry.insert(signature);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_lexer_typed_hook_adapter(
    type_name: &str,
    mappings: &[LexerTypedHookMapping],
) -> String {
    let trait_name = format!("{type_name}Hooks");
    let adapter_name = format!("{type_name}TypedHooks");
    let mut methods = BTreeMap::<(String, bool), Vec<SemanticLiteral>>::new();
    for mapping in mappings {
        methods
            .entry((
                mapping.method_name.clone(),
                mapping.kind == LexerTypedHookKind::Predicate,
            ))
            .or_insert_with(|| mapping.call.arguments.clone());
    }
    let method_decls = methods
        .iter()
        .map(|((method, predicate), arguments)| {
            let arguments = render_semantic_method_arguments(arguments);
            let separator = if arguments.is_empty() { "" } else { ", " };
            let result = if *predicate { " -> bool" } else { "" };
            format!(
                "    fn {method}<I>(&mut self, ctx: &mut antlr4_runtime::LexerSemCtx<'_, I>{separator}{arguments}){result}\n    where\n        I: antlr4_runtime::CharStream;"
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let predicate_arms = mappings
        .iter()
        .filter(|mapping| mapping.kind == LexerTypedHookKind::Predicate)
        .map(|mapping| {
            let rule = mapping.rule_index;
            let index = mapping.coordinate_index;
            let arguments = render_semantic_call_arguments(&mapping.call.arguments);
            let separator = if arguments.is_empty() { "" } else { ", " };
            let method = &mapping.method_name;
            let call = format!("self.0.{method}(ctx{separator}{arguments})");
            let call = if mapping.call.negated {
                format!("!{call}")
            } else {
                call
            };
            format!("            ({rule}, {index}) => Some({call}),")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let action_arms = mappings
        .iter()
        .filter(|mapping| mapping.kind == LexerTypedHookKind::Action)
        .map(|mapping| {
            let rule = mapping.rule_index;
            let index = mapping.coordinate_index;
            let arguments = render_semantic_call_arguments(&mapping.call.arguments);
            let separator = if arguments.is_empty() { "" } else { ", " };
            let method = &mapping.method_name;
            format!(
                "            ({rule}, {index}) => {{ self.0.{method}(ctx{separator}{arguments}); true }}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"pub trait {trait_name}: Sized {{
{method_decls}

    fn lexer_reset<I>(&mut self, _ctx: &mut antlr4_runtime::LexerLifecycleCtx<'_, I>)
    where
        I: antlr4_runtime::CharStream,
    {{}}

    fn lexer_before_token<I>(&mut self, _ctx: &mut antlr4_runtime::LexerLifecycleCtx<'_, I>)
    where
        I: antlr4_runtime::CharStream,
    {{}}

    fn lexer_after_accept<I>(&mut self, _ctx: &mut antlr4_runtime::LexerLifecycleCtx<'_, I>)
    where
        I: antlr4_runtime::CharStream,
    {{}}

    fn token_emitted(&mut self, _token: antlr4_runtime::TokenView<'_>) {{}}
}}

#[derive(Clone, Debug, Default)]
pub struct {adapter_name}<T>(pub T);

impl<T> {adapter_name}<T> {{
    pub const fn new(inner: T) -> Self {{ Self(inner) }}
}}

impl<T> antlr4_runtime::SemanticHooks for {adapter_name}<T>
where
    T: {trait_name},
{{
    fn lexer_sempred<I>(&mut self, ctx: &mut antlr4_runtime::LexerSemCtx<'_, I>, rule_index: usize, pred_index: usize) -> Option<bool>
    where
        I: antlr4_runtime::CharStream,
    {{
        match (rule_index, pred_index) {{
{predicate_arms}
            _ => None,
        }}
    }}

    fn lexer_action<I>(&mut self, ctx: &mut antlr4_runtime::LexerSemCtx<'_, I>, action: antlr4_runtime::LexerCustomAction) -> bool
    where
        I: antlr4_runtime::CharStream,
    {{
        let Ok(rule_index) = usize::try_from(action.rule_index()) else {{ return false; }};
        let Ok(action_index) = usize::try_from(action.action_index()) else {{ return false; }};
        match (rule_index, action_index) {{
{action_arms}
            _ => false,
        }}
    }}

    fn lexer_reset<I>(&mut self, ctx: &mut antlr4_runtime::LexerLifecycleCtx<'_, I>)
    where
        I: antlr4_runtime::CharStream,
    {{
        self.0.lexer_reset(ctx);
    }}

    fn lexer_before_token<I>(&mut self, ctx: &mut antlr4_runtime::LexerLifecycleCtx<'_, I>)
    where
        I: antlr4_runtime::CharStream,
    {{
        self.0.lexer_before_token(ctx);
    }}

    fn lexer_after_accept<I>(&mut self, ctx: &mut antlr4_runtime::LexerLifecycleCtx<'_, I>)
    where
        I: antlr4_runtime::CharStream,
    {{
        self.0.lexer_after_accept(ctx);
    }}

    fn lexer_token_emitted(&mut self, token: antlr4_runtime::TokenView<'_>) {{
        self.0.token_emitted(token);
    }}
}}
"#
    )
}

pub(crate) fn parser_typed_hook_mappings(
    data: &RecognizerCodegenData<'_>,
    patterns: &SemPatternFile,
) -> io::Result<Vec<TypedHookMapping>> {
    let mut mappings = Vec::new();
    for predicate in structural_predicates(data)? {
        push_typed_predicate_hook_mapping(
            data,
            patterns,
            predicate.rule_index,
            predicate.predicate_index,
            &predicate.body,
            &mut mappings,
        )?;
    }
    for action in structural_actions(data)?
        .into_iter()
        .filter(|action| action.authored && !action.body.trim().is_empty())
    {
        if let Some(call) = patterns.hook_helper_call(SemanticsKind::ParserAction, &action.body)? {
            mappings.push(TypedHookMapping {
                rule_index: action.rule_index,
                coordinate_index: action.action_index,
                kind: ParserTypedHookKind::Action,
                method_name: rust_function_name(&call.name),
                call,
            });
        }
    }
    disambiguate_parser_typed_hook_names(&mut mappings);
    mappings.sort_by_key(|mapping| (mapping.rule_index, mapping.coordinate_index, mapping.kind));
    mappings.dedup();
    validate_typed_hook_signatures(&mappings)?;
    Ok(mappings)
}

fn push_typed_predicate_hook_mapping(
    data: &RecognizerCodegenData<'_>,
    patterns: &SemPatternFile,
    rule_index: usize,
    pred_index: usize,
    body: &str,
    mappings: &mut Vec<TypedHookMapping>,
) -> io::Result<()> {
    let helper_call = match parse_semantic_helper_call(body, SemanticsKind::ParserPredicate, None) {
        Some(call) => Some(call),
        None => patterns.hook_helper_call(SemanticsKind::ParserPredicate, body)?,
    };
    let forced_hook = patterns
        .coordinate_predicate_template(
            SemanticsKind::ParserPredicate,
            data.rule_names.get(rule_index).map(String::as_str),
            Some(pred_index),
        )
        .is_some_and(|template| matches!(template, Some(PredicateTemplate::Hook)));
    let parsed = parse_predicate_template_with_patterns(body, patterns)?;
    if let Some(call) = helper_call
        && (forced_hook || parsed.is_none() || matches!(parsed, Some(PredicateTemplate::Hook)))
    {
        mappings.push(TypedHookMapping {
            rule_index,
            coordinate_index: pred_index,
            kind: ParserTypedHookKind::Predicate,
            method_name: rust_function_name(&call.name),
            call,
        });
    }
    Ok(())
}

const TYPED_HOOK_ACTION_METHOD: &str = "custom_action";

pub(crate) fn disambiguate_parser_typed_hook_names(mappings: &mut [TypedHookMapping]) {
    let predicate_names = mappings
        .iter()
        .filter(|mapping| mapping.kind == ParserTypedHookKind::Predicate)
        .map(|mapping| mapping.method_name.clone())
        .collect::<BTreeSet<_>>();
    let action_names = mappings
        .iter()
        .filter(|mapping| mapping.kind == ParserTypedHookKind::Action)
        .map(|mapping| mapping.method_name.clone())
        .collect::<BTreeSet<_>>();
    let mut allocated = BTreeMap::<(ParserTypedHookKind, String), String>::new();
    let mut used = BTreeSet::from([TYPED_HOOK_ACTION_METHOD.to_owned()]);
    for mapping in mappings {
        let helper = (mapping.kind, mapping.call.name.clone());
        if let Some(method_name) = allocated.get(&helper) {
            mapping.method_name.clone_from(method_name);
            continue;
        }
        if mapping.method_name == TYPED_HOOK_ACTION_METHOD
            || (predicate_names.contains(&mapping.method_name)
                && action_names.contains(&mapping.method_name))
        {
            mapping.method_name.push_str(match mapping.kind {
                ParserTypedHookKind::Predicate => "_pred",
                ParserTypedHookKind::Action => "_action",
            });
        }
        let method_name = unique_typed_hook_method_name(&mapping.method_name, &used);
        used.insert(method_name.clone());
        allocated.insert(helper, method_name.clone());
        mapping.method_name = method_name;
    }
}

fn unique_typed_hook_method_name(base: &str, used: &BTreeSet<String>) -> String {
    if !used.contains(base) {
        return base.to_owned();
    }
    let stem = base.strip_prefix("r#").unwrap_or(base);
    let mut suffix = 2;
    loop {
        let candidate = format!("{stem}_{suffix}");
        if !used.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

const fn semantic_literal_kind(literal: &SemanticLiteral) -> SemanticLiteralKind {
    match literal {
        SemanticLiteral::String(_) => SemanticLiteralKind::String,
        SemanticLiteral::Bool(_) => SemanticLiteralKind::Bool,
        SemanticLiteral::Integer(_) => SemanticLiteralKind::Integer,
    }
}

fn validate_typed_hook_signatures(mappings: &[TypedHookMapping]) -> io::Result<()> {
    let mut signatures = BTreeMap::<(&str, ParserTypedHookKind), Vec<SemanticLiteralKind>>::new();
    for mapping in mappings {
        let signature = mapping
            .call
            .arguments
            .iter()
            .map(semantic_literal_kind)
            .collect::<Vec<_>>();
        match signatures.entry((&mapping.method_name, mapping.kind)) {
            Entry::Occupied(entry) if entry.get() != &signature => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "typed semantic helper {} has conflicting literal signatures {:?} and {signature:?}",
                        mapping.call.name,
                        entry.get()
                    ),
                ));
            }
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                entry.insert(signature);
            }
        }
    }
    Ok(())
}

fn render_semantic_method_arguments(arguments: &[SemanticLiteral]) -> String {
    arguments
        .iter()
        .enumerate()
        .map(|(index, literal)| {
            let ty = match literal {
                SemanticLiteral::String(_) => "&str",
                SemanticLiteral::Bool(_) => "bool",
                SemanticLiteral::Integer(_) => "i64",
            };
            format!("arg{index}: {ty}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_semantic_call_arguments(arguments: &[SemanticLiteral]) -> String {
    arguments
        .iter()
        .map(|literal| match literal {
            SemanticLiteral::String(value) => format!("\"{}\"", rust_string(value)),
            SemanticLiteral::Bool(value) => value.to_string(),
            SemanticLiteral::Integer(value) => value.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn render_typed_hook_adapter(
    type_name: &str,
    mappings: &[TypedHookMapping],
) -> String {
    if mappings.is_empty() {
        return String::new();
    }
    let trait_name = format!("{type_name}Hooks");
    let adapter_name = format!("{type_name}TypedHooks");
    let mut methods = BTreeMap::<(String, ParserTypedHookKind), Vec<SemanticLiteral>>::new();
    for mapping in mappings {
        methods
            .entry((mapping.method_name.clone(), mapping.kind))
            .or_insert_with(|| mapping.call.arguments.clone());
    }
    let method_decls = methods
        .iter()
        .map(|((method, kind), arguments)| {
            let arguments = render_semantic_method_arguments(arguments);
            let separator = if arguments.is_empty() { "" } else { ", " };
            let result = if *kind == ParserTypedHookKind::Predicate {
                " -> bool"
            } else {
                ""
            };
            format!(
                "    fn {method}<L>(&mut self, ctx: &mut antlr4_runtime::ParserSemCtx<'_, L>{separator}{arguments}){result}\n    where\n        L: TokenSource;"
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let predicate_arms = mappings
        .iter()
        .filter(|mapping| mapping.kind == ParserTypedHookKind::Predicate)
        .map(|mapping| {
            let rule_index = mapping.rule_index;
            let pred_index = mapping.coordinate_index;
            let method = &mapping.method_name;
            let arguments = render_semantic_call_arguments(&mapping.call.arguments);
            let separator = if arguments.is_empty() { "" } else { ", " };
            let call = format!("self.0.{method}(ctx{separator}{arguments})");
            let call = if mapping.call.negated {
                format!("!{call}")
            } else {
                call
            };
            format!("            ({rule_index}, {pred_index}) => Some({call}),")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let action_arms = mappings
        .iter()
        .filter(|mapping| mapping.kind == ParserTypedHookKind::Action)
        .map(|mapping| {
            let rule_index = mapping.rule_index;
            let action_index = mapping.coordinate_index;
            let method = &mapping.method_name;
            let arguments = render_semantic_call_arguments(&mapping.call.arguments);
            let separator = if arguments.is_empty() { "" } else { ", " };
            format!(
                "            ({rule_index}, Some({action_index})) => {{ self.0.{method}(ctx{separator}{arguments}); true }},"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let action_dispatch = if action_arms.is_empty() {
        "        self.0.custom_action(ctx, action)".to_owned()
    } else {
        format!(
            "        match (action.rule_index(), action.action_index()) {{\n{action_arms}\n            _ => self.0.custom_action(ctx, action),\n        }}"
        )
    };
    format!(
        r#"pub trait {trait_name}: Sized {{
{method_decls}

    /// Handles a committed parser action routed to the typed hook. Return
    /// `true` when the action is handled so it satisfies a `hook`/`error`
    /// unknown-semantic policy; the default no-op returns `false` (unhandled),
    /// which fails loud under those policies.
    fn custom_action<L>(&mut self, _ctx: &mut antlr4_runtime::ParserSemCtx<'_, L>, _action: antlr4_runtime::ParserAction) -> bool
    where
        L: TokenSource,
    {{
        false
    }}
}}

#[derive(Clone, Copy, Debug, Default)]
pub struct {adapter_name}<T>(pub T);

impl<T> {adapter_name}<T> {{
    pub const fn new(inner: T) -> Self {{ Self(inner) }}
}}

impl<T> antlr4_runtime::SemanticHooks for {adapter_name}<T>
where
    T: {trait_name},
{{
    fn sempred<L>(&mut self, ctx: &mut antlr4_runtime::ParserSemCtx<'_, L>, rule_index: usize, pred_index: usize) -> Option<bool>
    where
        L: TokenSource,
    {{
        match (rule_index, pred_index) {{
{predicate_arms}
            _ => None,
        }}
    }}

    fn action<L>(&mut self, ctx: &mut antlr4_runtime::ParserSemCtx<'_, L>, action: antlr4_runtime::ParserAction) -> bool
    where
        L: TokenSource,
    {{
{action_dispatch}
    }}
}}
"#
    )
}
