/// Generates the typed context views, listener trait, and walker for the
/// embedded `.test.stg` surface:
///
/// * one `<Rule>Context` view per parser rule (plus one per labeled
///   alternative) with cardinality-aware rule, token, and label accessors,
///   `child_count()`, `direct_terminals()`, `start()`, `text()`, public attribute
///   fields, and a `FromRuleNode` impl backing
///   `ctx.downcast_ref::<XContext>()`;
/// * the `<Grammar>Listener` trait with defaulted `enter_/exit_<rule>` (and
///   per-labeled-alternative) callbacks plus terminal/error-node visitors;
/// * a module-local `ParseTreeWalker` whose bridge dispatches the runtime
///   walker onto the typed listener callbacks, threading the invoking-state
///   chain Java's `RuleContext.toString` renders (`[13 6]`).
pub(crate) fn parser_surface_name(grammar_name: &str) -> &str {
    grammar_name.strip_suffix("Parser").unwrap_or(grammar_name)
}

fn render_embedded_context_types(
    grammar_name: &str,
    data: &RecognizerCodegenData<'_>,
    model: &embedded::EmbeddedModel,
    options: ParserRenderOptions<'_>,
    antlr4rust_context_rules: &BTreeSet<usize>,
    antlr4rust_context_wrapper: &str,
) -> io::Result<String> {
    let mut out = String::new();
    let context_names = context_surface_names(model);
    let surface_name = parser_surface_name(grammar_name);
    let listener_trait = format!("{surface_name}Listener");
    let visitor_trait = format!("{surface_name}Visitor");
    let visitable_trait = format!("{surface_name}Visitable");
    let tree_walker = format!("{surface_name}TreeWalker");
    let validation_error = format!("{surface_name}ValidationError");
    let validated_listener_trait = format!("{surface_name}ValidatedListener");
    let validated_visitor_trait = format!("{surface_name}ValidatedVisitor");
    let validated_visitable_trait = format!("{surface_name}ValidatedVisitable");
    let validated_tree_walker = format!("{surface_name}ValidatedTreeWalker");
    let token_accessors = std::iter::once(("EOF".to_owned(), TOKEN_EOF))
        .chain(
            data.symbolic_names
                .iter()
                .enumerate()
                .filter_map(|(token_type, name)| {
                    let name = name.as_ref()?;
                    i32::try_from(token_type).ok().map(|ty| (name.clone(), ty))
                }),
        )
        .collect::<Vec<_>>();

    if options.generate_visitor {
        let _ = writeln!(
            out,
            "#[allow(dead_code)]\npub trait {visitable_trait}<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a>;\n}}\n\nimpl<'a> {visitable_trait}<'a> for antlr4_runtime::Node<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self }}\n}}\n\nimpl<'a> {visitable_trait}<'a> for RuleNodeView<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.node() }}\n}}\n\n#[allow(dead_code)]\npub trait {validated_visitable_trait}<'a> {{\n    fn into_validated_parse_tree_node(self) -> antlr4_runtime::Node<'a>;\n}}\n\nimpl<'a> {validated_visitable_trait}<'a> for ValidatedRuleNode<'a> {{\n    fn into_validated_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.node() }}\n}}\n\nimpl<'a> {validated_visitable_trait}<'a> for &ValidatedRuleNode<'a> {{\n    fn into_validated_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.node() }}\n}}\n"
        );
    }

    if !antlr4rust_context_rules.is_empty() {
        let _ = writeln!(
            out,
            "#[allow(dead_code)]
struct {antlr4rust_context_wrapper}<T>(T);

impl<T> std::ops::Deref for {antlr4rust_context_wrapper}<T> {{
    type Target = T;

    fn deref(&self) -> &Self::Target {{
        &self.0
    }}
}}
"
        );
    }
    out.push_str(&render_validated_tree_support(surface_name));
    if options.generate_visitor {
        let _ = writeln!(
            out,
            "impl<'a> {visitable_trait}<'a> for TerminalNode<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.node() }}\n}}\n\nimpl<'a> {visitable_trait}<'a> for &TerminalNode<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.node() }}\n}}\n\nimpl<'a> {visitable_trait}<'a> for ErrorNode<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.node() }}\n}}\n\nimpl<'a> {visitable_trait}<'a> for &ErrorNode<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.node() }}\n}}\n\nimpl<'a> {validated_visitable_trait}<'a> for TerminalNode<'a> {{\n    fn into_validated_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.node() }}\n}}\n\nimpl<'a> {validated_visitable_trait}<'a> for &TerminalNode<'a> {{\n    fn into_validated_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.node() }}\n}}\n"
        );
    }
    out.push_str(&render_context_kind_functions(model, &context_names));
    let mut validation_arms = String::new();

    for ContextViewName {
        surface,
        rule_index,
        alternative_label,
    } in &context_names.views
    {
        let view_name = &surface.context_type;
        let rule = &model.rules[*rule_index];
        let context_kind = context_names.kind_id(*rule_index, alternative_label.as_deref());
        let context_kind_match = alternative_label
            .as_ref()
            .map_or_else(|| "any".to_owned(), |_| format!("exact({context_kind})"));
        let child_cardinalities = context_child_cardinalities(rule, alternative_label.as_deref());
        let label_accessors = context_label_accessors(rule, alternative_label.as_deref());
        let antlr4rust_compat =
            alternative_label.is_none() && antlr4rust_context_rules.contains(rule_index);
        let compatibility_methods = if antlr4rust_compat {
            antlr4rust_compat_method_names(view_name, model, &token_accessors, &child_cardinalities)
                .map_err(|error| {
                    embedded_context_accessor_translation_error(data, *rule_index, &error)
                })?
        } else {
            BTreeSet::new()
        };
        let common_methods = context_common_method_names(&compatibility_methods);
        let rule_node_method = &common_methods.rule_node;
        let attrs_struct = embedded::attrs_struct_name(*rule_index);
        let mut attributes = String::new();
        if !rule.attrs.is_empty() {
            let _ = writeln!(attributes, "            {attrs_struct} {{");
            for attr in &rule.attrs {
                let name = embedded::escape_keyword(&attr.name);
                let _ = writeln!(attributes, "                {name}: {},", attr.ty);
            }
            attributes.push_str("            }\n");
        }
        let _ = writeln!(
            out,
            "antlr4_runtime::__antlr4_rust_context! {{\n    pub struct {view_name} {{\n        rule_index: {rule_index},\n        context_kind: {context_kind_match},\n        validated_downcast: branded,\n        attributes: {{\n{attributes}        }},\n        methods: {{\n            rule_node: {rule_node_method},\n            child_count: {child_count},\n            direct_terminals: {direct_terminals},\n            start: {start},\n            text: {text},\n        }}\n    }}\n}}\n",
            child_count = common_methods.child_count,
            direct_terminals = common_methods.direct_terminals,
            start = common_methods.start,
            text = common_methods.text,
        );
        let rendered_accessors = render_context_child_accessors(ContextAccessorsRender {
            view_name,
            model,
            context_names: &context_names,
            token_accessors: &token_accessors,
            child_cardinalities: &child_cardinalities,
            label_accessors: &label_accessors,
            antlr4rust_compat,
            antlr4rust_context_wrapper,
            common_methods: &common_methods,
        });
        let compatibility_impl = (!rendered_accessors.compatibility.is_empty()).then(|| {
            format!(
                "\n#[allow(dead_code, private_bounds, clippy::all)]\nimpl<'a, State: __RecoveryContextState> {antlr4rust_context_wrapper}<{view_name}<'a, State>> {{\n{compatibility}}}\n",
                compatibility = rendered_accessors.compatibility,
            )
        });
        let _ = writeln!(
            out,
            "antlr4_runtime::__antlr4_rust_context_accessors! {{\n    {view_name} {{\n{declarations}    }}\n}}\n{compatibility_impl}",
            declarations = rendered_accessors.declarations,
            compatibility_impl = compatibility_impl.unwrap_or_default(),
        );
        let validation_body = if rendered_accessors.validation.is_empty() {
            String::new()
        } else {
            format!(
                "                    let context = {view_name}::__from_listener_node(context, None);\n{}",
                rendered_accessors.validation
            )
        };
        let _ = writeln!(
            validation_arms,
            "                {context_kind} => {{\n{validation_body}                }},"
        );
        if options.generate_visitor {
            let _ = writeln!(
                out,
                "impl<'a> {visitable_trait}<'a> for {view_name}<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.{rule_node_method}().node() }}\n}}\n\nimpl<'a> {visitable_trait}<'a> for &{view_name}<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.{rule_node_method}().node() }}\n}}\n"
            );
            let _ = writeln!(
                out,
                "impl<'a> {validated_visitable_trait}<'a> for {view_name}<'a, ValidatedTreeContext> {{\n    fn into_validated_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.{rule_node_method}().node() }}\n}}\n\nimpl<'a> {validated_visitable_trait}<'a> for &{view_name}<'a, ValidatedTreeContext> {{\n    fn into_validated_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.{rule_node_method}().node() }}\n}}\n"
            );
        }
    }

    let _ = writeln!(
        out,
        r#"/// Checks generated required-child invariants without changing the
/// recovery-oriented tree's type.
///
/// Strict parsing calls this after proving that lexer and parser syntax-error
/// counts are both zero. It is public so structural runtime/codegen invariant
/// failures can be diagnosed independently.
pub fn validate_tree_structure(
    parsed: &antlr4_runtime::ParsedFile,
) -> Result<(), {validation_error}> {{
    let tree = parsed.tree();
    if tree.as_rule().is_none() {{
        return Err({validation_error}::InvalidRoot);
    }}
    for node in tree.descendants() {{
        match node.kind() {{
            antlr4_runtime::NodeKind::Terminal => {{}}
            antlr4_runtime::NodeKind::Error => {{
                let symbol = node
                    .as_error()
                    .expect("error node kind checked")
                    .symbol();
                return Err({validation_error}::RecoveredErrorNode {{
                    line: symbol.line(),
                    column: symbol.column(),
                    text: symbol.text_or_empty().to_owned(),
                }});
            }}
            antlr4_runtime::NodeKind::Rule => {{
                let context = node.as_rule().expect("rule node kind checked");
                match __context_kind(context) {{
{validation_arms}                    _ => {{
                        return Err({validation_error}::UnknownRule {{
                            rule_index: context.rule_index(),
                        }});
                    }}
                }}
            }}
        }}
    }}
    Ok(())
}}
"#
    );

    if options.generate_listener {
        out.push_str(&render_context_listener_surface(
            &context_names,
            &listener_trait,
            &tree_walker,
            &validated_listener_trait,
            &validated_tree_walker,
        ));
    }

    if options.generate_visitor {
        out.push_str(&render_context_visitor_surface(
            &context_names,
            &visitor_trait,
            &visitable_trait,
            &validated_visitor_trait,
            &validated_visitable_trait,
        ));
    }
    Ok(out)
}

/// Post-translation cleanups shared by all embedded bodies: `TParser::NL`
/// becomes `Self::NL` so token constants resolve inside the generic impl.
fn post_process_embedded(_original: &str, translated: &str, type_name: &str) -> String {
    let translated = replace_all(translated, &format!("{type_name}::"), "Self::");
    let translated = replace_all(
        &translated,
        "(&__ctx).to_string_tree(Some(self), self.base.token_store())",
        "(&__ctx).to_string_tree(Some(self), self.base.parse_tree_storage(), self.base.token_store())",
    );
    replace_all(
        &translated,
        "(&__ctx).to_string_tree(Some(self))",
        "(&__ctx).to_string_tree(Some(self), self.base.parse_tree_storage(), self.base.token_store())",
    )
}

/// Generated-recognizer surface used by rendered test actions.
fn embedded_parser_facades() -> String {
    r#"    #[allow(dead_code)]
    fn output(&mut self) -> std::io::Stdout {
        std::io::stdout()
    }

    #[allow(dead_code)]
    fn input(&mut self) -> __GeneratedInput<'_, L> {
        __GeneratedInput(self.base.input())
    }

    #[allow(dead_code)]
    fn interpreter(&mut self) -> &mut Self {
        self
    }

    #[allow(dead_code)]
    fn set_error_handler(&mut self, _strategy: antlr4_runtime::BailErrorStrategy) {
        self.base.set_bail_on_error(true);
    }

    #[allow(dead_code)]
    fn expected_tokens(&mut self) -> antlr4_runtime::ExpectedTokenSet {
        self.base.expected_tokens_current(atn())
    }

    #[allow(dead_code)]
    fn rule_invocation_stack(&self) -> Vec<String> {
        self.base.rule_invocation_stack()
    }

    #[allow(dead_code)]
    fn dump_dfa(&mut self) {
        if let Some(simulator) = self.simulator.as_ref() {
            print!("{}", simulator.dump_dfa_java_style(self.base.vocabulary()));
        }
    }

"#
    .to_owned()
}
