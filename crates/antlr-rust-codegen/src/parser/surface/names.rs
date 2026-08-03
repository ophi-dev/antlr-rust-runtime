pub(crate) fn build_structural_parser_surface(
    data: &RecognizerCodegenData<'_>,
    grammar_name: &str,
    options: ParserRenderOptions<'_>,
) -> io::Result<ParserSurfaceBindings> {
    let model = structural_embedded_model(data, false)?;
    let mut out = ParserSurfaceBindings {
        rule_has_attrs: model
            .rules
            .iter()
            .map(embedded::RuleModel::has_attrs)
            .collect(),
        ..ParserSurfaceBindings::default()
    };
    for (rule_index, rule) in model.rules.iter().enumerate() {
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
    out.module_items.push_str(EMBEDDED_INPUT_FACADE);
    out.module_items.push_str(&render_embedded_context_types(
        grammar_name,
        data,
        &model,
        options,
        &BTreeSet::new(),
        embedded::ANTLR4RUST_CONTEXT_WRAPPER,
    )?);
    Ok(out)
}

/// Lowers argument text from each structural `rule[expr]` call onto that
/// element's finalized rule-transition state. Supports integer literals and
/// single identifiers (translated to the caller's `__attrs` field).
fn structural_embedded_rule_call_args(
    data: &RecognizerCodegenData<'_>,
) -> io::Result<BTreeMap<usize, String>> {
    Ok(structural_rule_calls(data)?
        .into_iter()
        .filter_map(|call| {
            let expression = embedded_rule_call_expression(call.arguments.as_deref()?)?;
            Some((call.state, expression))
        })
        .collect())
}

fn embedded_rule_call_expression(value: &str) -> Option<String> {
    let value = value.trim();
    if value.parse::<i64>().is_ok() {
        Some(value.to_owned())
    } else if value
        .chars()
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && value
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
    {
        // A bare identifier is a caller attribute (arg/local/return).
        Some(format!("__attrs.{}", embedded::escape_keyword(value)))
    } else {
        None
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextSurfaceName {
    pub(crate) context_type: String,
    pub(crate) listener_method: String,
    pub(crate) visitor_method: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextViewName {
    pub(crate) surface: ContextSurfaceName,
    pub(crate) rule_index: usize,
    pub(crate) alternative_label: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ContextSurfaceNames {
    pub(crate) rules: Vec<ContextSurfaceName>,
    pub(crate) views: Vec<ContextViewName>,
}

impl ContextSurfaceNames {
    fn kind_id(&self, rule_index: usize, alternative_label: Option<&str>) -> usize {
        self.views
            .iter()
            .position(|view| {
                view.rule_index == rule_index
                    && view.alternative_label.as_deref() == alternative_label
            })
            .expect("context view has an allocated dispatch identity")
    }
}

/// Reserves canonical rule names before allocating labels. Alternative labels
/// always use a `Label`/`_label` suffix so their generated surfaces cannot be
/// confused with rule surfaces.
pub(crate) fn context_surface_names(model: &embedded::EmbeddedModel) -> ContextSurfaceNames {
    let mut used_context_types = BTreeSet::from([
        "StoredTreeContext".to_owned(),
        "ValidatedTreeContext".to_owned(),
    ]);
    let mut used_listener_methods = BTreeSet::from(["every_rule".to_owned()]);
    let mut used_visitor_methods = BTreeSet::from([
        "children".to_owned(),
        "error_node".to_owned(),
        "terminal".to_owned(),
    ]);
    let rules = model
        .rules
        .iter()
        .map(|rule| ContextSurfaceName {
            context_type: allocate_rule_context_type(&rule.name, &mut used_context_types),
            listener_method: allocate_rule_listener_method(&rule.name, &mut used_listener_methods),
            visitor_method: allocate_rule_listener_method(&rule.name, &mut used_visitor_methods),
        })
        .collect::<Vec<_>>();

    let mut alternatives = (0..model.rules.len())
        .map(|_| BTreeMap::new())
        .collect::<Vec<_>>();
    let mut views = Vec::new();
    for (rule_index, rule) in model.rules.iter().enumerate() {
        views.push(ContextViewName {
            surface: rules[rule_index].clone(),
            rule_index,
            alternative_label: None,
        });
        for alternative in &rule.alts {
            let Some(label) = &alternative.label else {
                continue;
            };
            if let Entry::Vacant(entry) = alternatives[rule_index].entry(label.clone()) {
                let surface = ContextSurfaceName {
                    context_type: allocate_label_context_type(label, &mut used_context_types),
                    listener_method: allocate_label_listener_method(
                        label,
                        &mut used_listener_methods,
                    ),
                    visitor_method: allocate_label_listener_method(
                        label,
                        &mut used_visitor_methods,
                    ),
                };
                entry.insert(surface.clone());
                views.push(ContextViewName {
                    surface,
                    rule_index,
                    alternative_label: Some(label.clone()),
                });
            }
        }
    }

    ContextSurfaceNames { rules, views }
}

fn allocate_rule_context_type(source_name: &str, used: &mut BTreeSet<String>) -> String {
    let base = rust_type_name(source_name);
    let canonical = format!("{base}Context");
    if used.insert(canonical.clone()) {
        return canonical;
    }

    allocate_numbered_context_type(&format!("{base}Rule"), used)
}

fn allocate_label_context_type(source_name: &str, used: &mut BTreeSet<String>) -> String {
    allocate_numbered_context_type(&format!("{}Label", rust_type_name(source_name)), used)
}

fn allocate_numbered_context_type(stem: &str, used: &mut BTreeSet<String>) -> String {
    let mut candidate = format!("{stem}Context");
    let mut suffix = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{stem}{suffix}Context");
        suffix += 1;
    }
    candidate
}

fn allocate_rule_listener_method(source_name: &str, used: &mut BTreeSet<String>) -> String {
    let canonical = rust_function_name(source_name)
        .trim_start_matches("r#")
        .to_owned();
    if used.insert(canonical.clone()) {
        return canonical;
    }

    allocate_numbered_listener_method(&format!("{canonical}_rule"), used)
}

fn allocate_label_listener_method(source_name: &str, used: &mut BTreeSet<String>) -> String {
    let canonical = rust_function_name(source_name);
    let stem = format!("{}_label", canonical.trim_start_matches("r#"));
    allocate_numbered_listener_method(&stem, used)
}

fn allocate_numbered_listener_method(stem: &str, used: &mut BTreeSet<String>) -> String {
    let mut candidate = stem.to_owned();
    let mut suffix = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{stem}_{suffix}");
        suffix += 1;
    }
    candidate
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ContextAlternativeDispatch {
    runtime_alt_number: usize,
    kind_id: usize,
    operator: bool,
}

fn context_alternative_dispatch(
    rule_index: usize,
    rule: &embedded::RuleModel,
    names: &ContextSurfaceNames,
) -> (bool, Vec<ContextAlternativeDispatch>) {
    let left_recursive = rule
        .alts
        .iter()
        .any(|alternative| alternative.is_lr_operator(&rule.name));
    let mut primary_alt_number = 0;
    let mut operator_alt_number = 0;
    let alternatives = rule
        .alts
        .iter()
        .enumerate()
        .map(|(authored_alt_index, alternative)| {
            let operator = left_recursive && alternative.is_lr_operator(&rule.name);
            let runtime_alt_number = if operator {
                operator_alt_number += 1;
                operator_alt_number
            } else if left_recursive {
                primary_alt_number += 1;
                primary_alt_number
            } else {
                authored_alt_index + 1
            };
            ContextAlternativeDispatch {
                runtime_alt_number,
                kind_id: names.kind_id(rule_index, alternative.label.as_deref()),
                operator,
            }
        })
        .collect();
    (left_recursive, alternatives)
}

fn render_context_alt_kind_match(
    alternatives: &[ContextAlternativeDispatch],
    fallback_kind: usize,
    alt_number: &str,
) -> String {
    if alternatives.is_empty()
        || alternatives
            .iter()
            .all(|alternative| alternative.kind_id == fallback_kind)
    {
        return fallback_kind.to_string();
    }
    let mut arms = String::new();
    for alternative in alternatives {
        let _ = writeln!(
            arms,
            "                    {} => {},",
            alternative.runtime_alt_number, alternative.kind_id
        );
    }
    let distinct_kinds = alternatives
        .iter()
        .map(|alternative| alternative.kind_id)
        .collect::<BTreeSet<_>>();
    if distinct_kinds.len() == 1 {
        let only_kind = distinct_kinds
            .first()
            .copied()
            .expect("non-empty alternatives have one context kind");
        let _ = writeln!(arms, "                    0 => {only_kind},");
    }
    format!(
        "match {alt_number} {{\n{arms}                    _ => {fallback_kind},\n                }}"
    )
}

fn render_context_kind_functions(
    model: &embedded::EmbeddedModel,
    names: &ContextSurfaceNames,
) -> String {
    if names
        .views
        .iter()
        .all(|view| view.alternative_label.is_none())
    {
        return r#"#[allow(dead_code)]
fn __context_kind(context: RuleNodeView<'_>) -> usize {
    context.rule_index()
}

#[allow(dead_code)]
fn __active_context_kind(
    context: &antlr4_runtime::ParserRuleContext,
    _storage: &antlr4_runtime::ParseTreeStorage,
    _tokens: &antlr4_runtime::TokenStore,
) -> usize {
    context.rule_index()
}

"#
        .to_owned();
    }

    let mut stored_arms = String::new();
    let mut active_arms = String::new();
    for (rule_index, rule) in model.rules.iter().enumerate() {
        let fallback_kind = names.kind_id(rule_index, None);
        let (left_recursive, alternatives) = context_alternative_dispatch(rule_index, rule, names);
        if !left_recursive {
            let matcher = render_context_alt_kind_match(
                &alternatives,
                fallback_kind,
                "context.context_alt_number()",
            );
            let _ = writeln!(
                stored_arms,
                "        {rule_index} => {{\n            {matcher}\n        }},"
            );
            let _ = writeln!(
                active_arms,
                "        {rule_index} => {{\n            {matcher}\n        }},"
            );
            continue;
        }

        let primary = alternatives
            .iter()
            .copied()
            .filter(|alternative| !alternative.operator)
            .collect::<Vec<_>>();
        let operators = alternatives
            .iter()
            .copied()
            .filter(|alternative| alternative.operator)
            .collect::<Vec<_>>();
        let primary_match =
            render_context_alt_kind_match(&primary, fallback_kind, "context.context_alt_number()");
        let operator_match = render_context_alt_kind_match(
            &operators,
            fallback_kind,
            "context.context_alt_number()",
        );
        let _ = writeln!(
            stored_arms,
            "        {rule_index} => {{\n            let operator = context.children().next().and_then(antlr4_runtime::Node::as_rule).is_some_and(|child| child.rule_index() == {rule_index});\n            if operator {{\n                {operator_match}\n            }} else {{\n                {primary_match}\n            }}\n        }},"
        );
        let _ = writeln!(
            active_arms,
            "        {rule_index} => {{\n            let operator = context.child_nodes(storage, tokens).next().and_then(antlr4_runtime::Node::as_rule).is_some_and(|child| child.rule_index() == {rule_index});\n            if operator {{\n                {operator_match}\n            }} else {{\n                {primary_match}\n            }}\n        }},"
        );
    }

    format!(
        r#"#[allow(dead_code)]
fn __context_kind(context: RuleNodeView<'_>) -> usize {{
    match context.rule_index() {{
{stored_arms}        _ => usize::MAX,
    }}
}}

#[allow(dead_code)]
fn __active_context_kind(
    context: &antlr4_runtime::ParserRuleContext,
    storage: &antlr4_runtime::ParseTreeStorage,
    tokens: &antlr4_runtime::TokenStore,
) -> usize {{
    match context.rule_index() {{
{active_arms}        _ => usize::MAX,
    }}
}}

"#
    )
}

fn context_alternatives<'a>(
    rule: &'a embedded::RuleModel,
    alternative_label: Option<&str>,
) -> Vec<&'a embedded::AltModel> {
    rule.alts
        .iter()
        .filter(|alternative| {
            alternative_label.is_none_or(|label| alternative.label.as_deref() == Some(label))
        })
        .collect()
}

fn context_child_cardinalities(
    rule: &embedded::RuleModel,
    alternative_label: Option<&str>,
) -> BTreeMap<String, embedded::ChildCardinality> {
    choice_child_cardinalities(
        context_alternatives(rule, alternative_label)
            .into_iter()
            .map(|alternative| alternative.children.clone()),
    )
}
