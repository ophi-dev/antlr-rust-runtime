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

    out.push_str(
        r#"#[allow(dead_code)]
#[derive(Clone)]
pub struct TerminalNode<'a> {
    __node: RuntimeTerminalNode<'a>,
}

#[allow(dead_code)]
impl<'a> TerminalNode<'a> {
    fn new(node: RuntimeTerminalNode<'a>) -> Self {
        Self { __node: node }
    }

    pub fn symbol(&self) -> antlr4_runtime::TokenView<'a> {
        self.__node.symbol()
    }

    pub fn is_error(&self) -> bool {
        matches!(
            self.__node.node().kind(),
            antlr4_runtime::NodeKind::Error
        )
    }

    pub fn is_missing(&self) -> bool {
        self.symbol().is_synthetic()
    }
}

impl std::fmt::Display for TerminalNode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.__node.text())
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct ErrorNode<'a> {
    __node: RuntimeErrorNode<'a>,
}

#[allow(dead_code)]
impl<'a> ErrorNode<'a> {
    fn new(node: RuntimeErrorNode<'a>) -> Self {
        Self { __node: node }
    }

    pub fn symbol(&self) -> antlr4_runtime::TokenView<'a> {
        self.__node.symbol()
    }

    pub fn is_missing(&self) -> bool {
        self.symbol().is_synthetic()
    }
}

impl std::fmt::Display for ErrorNode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.__node.text())
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
enum __GeneratedRuleContext<'a> {
    Stored(RuleNodeView<'a>),
    Active {
        context: &'a antlr4_runtime::ParserRuleContext,
        storage: &'a antlr4_runtime::ParseTreeStorage,
        tokens: &'a antlr4_runtime::TokenStore,
    },
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct StoredTreeContext;

#[derive(Clone, Copy, Debug)]
struct __ActiveParserContext;

#[allow(dead_code)]
fn __context_children<'a>(
    source: __GeneratedRuleContext<'a>,
) -> impl Iterator<Item = antlr4_runtime::Node<'a>> + 'a {
    let mut stored = match source {
        __GeneratedRuleContext::Stored(node) => Some(node.children()),
        __GeneratedRuleContext::Active { .. } => None,
    };
    let mut active = match source {
        __GeneratedRuleContext::Stored(_) => None,
        __GeneratedRuleContext::Active {
            context,
            storage,
            tokens,
        } => Some(context.child_nodes(storage, tokens)),
    };
    std::iter::from_fn(move || {
        stored
            .as_mut()
            .and_then(Iterator::next)
            .or_else(|| active.as_mut().and_then(Iterator::next))
    })
}

#[allow(dead_code)]
fn __rule_children<'a>(
    source: __GeneratedRuleContext<'a>,
    rule_index: usize,
) -> impl Iterator<Item = RuleNodeView<'a>> + 'a {
    __context_children(source).filter_map(move |child| {
        let rule = child.as_rule()?;
        (rule.rule_index() == rule_index).then_some(rule)
    })
}

// Keep this triage aligned with runtime `RuleNodeView::terminal_children()` and
// `ParserRuleContext::terminal_children()`.
fn __terminal_children<'a>(
    source: __GeneratedRuleContext<'a>,
) -> impl Iterator<Item = RuntimeTerminalNode<'a>> + 'a {
    __context_children(source).filter_map(|child| match child.kind() {
        antlr4_runtime::NodeKind::Terminal => child.as_terminal(),
        antlr4_runtime::NodeKind::Error => {
            child.as_error().map(antlr4_runtime::ErrorNodeView::terminal)
        }
        antlr4_runtime::NodeKind::Rule => None,
    })
}

#[allow(dead_code)]
fn __token_children<'a>(
    source: __GeneratedRuleContext<'a>,
    token_type: i32,
) -> impl Iterator<Item = RuntimeTerminalNode<'a>> + 'a {
    __terminal_children(source)
        .filter(move |terminal| terminal.symbol().token_type() == token_type)
}

#[allow(dead_code)]
fn __token_children_matching<'a>(
    source: __GeneratedRuleContext<'a>,
    token_types: &'static [i32],
) -> impl Iterator<Item = RuntimeTerminalNode<'a>> + 'a {
    __terminal_children(source)
        .filter(move |terminal| token_types.contains(&terminal.symbol().token_type()))
}

"#,
    );
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
    out.push_str(LABELED_TOKEN_CHILD_HELPERS);
    out.push_str(
        r#"#[allow(dead_code)]
trait __FromActiveRuleContext<'a>: Sized {
    fn __from_active(
        context: &'a antlr4_runtime::ParserRuleContext,
        live_attrs: Option<&dyn std::any::Any>,
        invocation_states: Vec<isize>,
        storage: &'a antlr4_runtime::ParseTreeStorage,
        tokens: &'a antlr4_runtime::TokenStore,
    ) -> Option<Self>;
}

#[allow(dead_code)]
fn __active_context_view<'a, T: __FromActiveRuleContext<'a>>(
    context: &'a antlr4_runtime::ParserRuleContext,
    invocation_states: Vec<isize>,
    storage: &'a antlr4_runtime::ParseTreeStorage,
    tokens: &'a antlr4_runtime::TokenStore,
) -> Option<T> {
    T::__from_active(context, None, invocation_states, storage, tokens)
}

#[allow(dead_code)]
fn __active_context_view_with_attrs<'a, T: __FromActiveRuleContext<'a>>(
    context: &'a antlr4_runtime::ParserRuleContext,
    live_attrs: &dyn std::any::Any,
    invocation_states: Vec<isize>,
    storage: &'a antlr4_runtime::ParseTreeStorage,
    tokens: &'a antlr4_runtime::TokenStore,
) -> Option<T> {
    T::__from_active(
        context,
        Some(live_attrs),
        invocation_states,
        storage,
        tokens,
    )
}

#[allow(dead_code)]
fn __write_invocation_states(
    f: &mut std::fmt::Formatter<'_>,
    states: impl Iterator<Item = isize>,
) -> std::fmt::Result {
    f.write_str("[")?;
    let mut separator = "";
    for state in states {
        write!(f, "{separator}{state}")?;
        separator = " ";
    }
    f.write_str("]")
}

"#,
    );
    out.push_str(&render_validated_tree_support(surface_name));
    if options.generate_visitor {
        let _ = writeln!(
            out,
            "impl<'a> {visitable_trait}<'a> for TerminalNode<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.__node.node() }}\n}}\n\nimpl<'a> {visitable_trait}<'a> for &TerminalNode<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.__node.node() }}\n}}\n\nimpl<'a> {visitable_trait}<'a> for ErrorNode<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.__node.node() }}\n}}\n\nimpl<'a> {visitable_trait}<'a> for &ErrorNode<'a> {{\n    fn into_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.__node.node() }}\n}}\n\nimpl<'a> {validated_visitable_trait}<'a> for TerminalNode<'a> {{\n    fn into_validated_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.__node.node() }}\n}}\n\nimpl<'a> {validated_visitable_trait}<'a> for &TerminalNode<'a> {{\n    fn into_validated_parse_tree_node(self) -> antlr4_runtime::Node<'a> {{ self.__node.node() }}\n}}\n"
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
        let stored_kind_guard = alternative_label.as_ref().map_or(String::new(), |_| {
            format!(" || __context_kind(node) != {context_kind}")
        });
        let active_kind_guard = alternative_label.as_ref().map_or(String::new(), |_| {
            format!(" || __active_context_kind(context, storage, tokens) != {context_kind}")
        });
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
        let common_accessors = render_context_common_accessors(&common_methods);
        let rule_node_method = &common_methods.rule_node;
        let attrs_struct = embedded::attrs_struct_name(*rule_index);
        let mut fields = String::new();
        let mut field_inits = String::new();
        for attr in &rule.attrs {
            let name = embedded::escape_keyword(&attr.name);
            let _ = writeln!(fields, "    pub {name}: {ty},", ty = attr.ty);
            let _ = writeln!(field_inits, "            {name}: __attrs.{name}.clone(),");
        }
        let attrs_bindings = |source: &str| {
            if rule.attrs.is_empty() {
                String::new()
            } else {
                format!(
                    "        let __default = {attrs_struct}::default();\n        let __attrs = {source}.generated_attrs::<{attrs_struct}>().unwrap_or(&__default);\n"
                )
            }
        };
        let active_attrs_bindings = if rule.attrs.is_empty() {
            String::new()
        } else {
            format!(
                "        let __default = {attrs_struct}::default();\n        let __attrs = match live_attrs {{\n            Some(live_attrs) => live_attrs.downcast_ref::<{attrs_struct}>().expect(\"active context attributes match the parser rule\"),\n            None => context.generated_attrs::<{attrs_struct}>().unwrap_or(&__default),\n        }};\n"
            )
        };
        let stored_attrs_bindings = attrs_bindings("node");
        let _ = writeln!(
            out,
            "#[allow(non_camel_case_types, dead_code)]\n#[derive(Clone)]\npub struct {view_name}<'a, State = StoredTreeContext> {{\n    __node: __GeneratedRuleContext<'a>,\n    __invocation_states: Option<Vec<isize>>,\n    __state: std::marker::PhantomData<State>,\n{fields}}}\n"
        );
        let _ = writeln!(
            out,
            "impl<'a> FromRuleNode<'a> for {view_name}<'a> {{\n    fn from_rule_node(node: RuleNodeView<'a>) -> Option<Self> {{\n        if node.rule_index() != {rule_index}{stored_kind_guard} {{ return None; }}\n        Some(Self::__from_node(node))\n    }}\n}}\n\nimpl<'a> AsRuleNode<'a> for {view_name}<'a> {{\n    fn as_rule_node(&self) -> RuleNodeView<'a> {{ self.{rule_node_method}() }}\n}}\n\nimpl<'a> {view_name}<'a> {{\n    pub fn {rule_node_method}(&self) -> RuleNodeView<'a> {{\n        match self.__node {{\n            __GeneratedRuleContext::Stored(node) => node,\n            __GeneratedRuleContext::Active {{ .. }} => unreachable!(\"stored context type contains an active parser context\"),\n        }}\n    }}\n}}\n\nimpl<'a> __FromActiveRuleContext<'a> for {view_name}<'a, __ActiveParserContext> {{\n    fn __from_active(\n        context: &'a antlr4_runtime::ParserRuleContext,\n        live_attrs: Option<&dyn std::any::Any>,\n        invocation_states: Vec<isize>,\n        storage: &'a antlr4_runtime::ParseTreeStorage,\n        tokens: &'a antlr4_runtime::TokenStore,\n    ) -> Option<Self> {{\n        if context.rule_index() != {rule_index}{active_kind_guard} {{ return None; }}\n{active_attrs_bindings}        Some(Self {{\n            __node: __GeneratedRuleContext::Active {{ context, storage, tokens }},\n            __invocation_states: Some(invocation_states),\n            __state: std::marker::PhantomData,\n{field_inits}        }})\n    }}\n}}\n"
        );
        let _ = writeln!(
            out,
            "impl<'a> FromValidatedRuleNode<'a> for {view_name}<'a, ValidatedTreeContext> {{\n    fn from_validated_rule_node(node: ValidatedRuleNode<'a>) -> Option<Self> {{\n        let node = node.rule_node();\n        if node.rule_index() != {rule_index}{stored_kind_guard} {{ return None; }}\n        Some(Self::__from_validated_node(node))\n    }}\n}}\n\nimpl<'a> AsRuleNode<'a> for {view_name}<'a, ValidatedTreeContext> {{\n    fn as_rule_node(&self) -> RuleNodeView<'a> {{ self.{rule_node_method}() }}\n}}\n"
        );
        let mut accessors = String::new();
        let _ = writeln!(
            accessors,
            "    fn __from_node(node: RuleNodeView<'a>) -> Self {{\n        Self::__from_node_with_invocation_states(node, None)\n    }}\n\n    fn __from_child_node(\n        node: RuleNodeView<'a>,\n        parent_invocation_states: Option<&[isize]>,\n    ) -> Self {{\n        let invocation_states = parent_invocation_states.map(|states| {{\n            let mut invocation_states = Vec::with_capacity(states.len() + 1);\n            invocation_states.push(node.invoking_state());\n            invocation_states.extend_from_slice(states);\n            invocation_states\n        }});\n        Self::__from_node_with_invocation_states(node, invocation_states)\n    }}\n\n    fn __from_listener_node(node: RuleNodeView<'a>, invocation_states: Option<&[isize]>) -> Self {{\n        Self::__from_node_with_invocation_states(\n            node,\n            invocation_states.map(|states| states.to_vec()),\n        )\n    }}\n\n    fn __from_node_with_invocation_states(\n        node: RuleNodeView<'a>,\n        invocation_states: Option<Vec<isize>>,\n    ) -> Self {{\n{stored_attrs_bindings}        Self {{\n            __node: __GeneratedRuleContext::Stored(node),\n            __invocation_states: invocation_states,\n            __state: std::marker::PhantomData,\n{field_inits}        }}\n    }}\n"
        );
        let mut validated_constructors = String::new();
        let _ = writeln!(
            validated_constructors,
            "    fn __from_validated_node(node: RuleNodeView<'a>) -> Self {{\n        Self::__from_validated_node_with_invocation_states(node, None)\n    }}\n\n    fn __from_validated_child_node(\n        node: RuleNodeView<'a>,\n        parent_invocation_states: Option<&[isize]>,\n    ) -> Self {{\n        let invocation_states = parent_invocation_states.map(|states| {{\n            let mut invocation_states = Vec::with_capacity(states.len() + 1);\n            invocation_states.push(node.invoking_state());\n            invocation_states.extend_from_slice(states);\n            invocation_states\n        }});\n        Self::__from_validated_node_with_invocation_states(node, invocation_states)\n    }}\n\n    fn __from_validated_listener_node(\n        node: RuleNodeView<'a>,\n        invocation_states: Option<&[isize]>,\n    ) -> Self {{\n        Self::__from_validated_node_with_invocation_states(\n            node,\n            invocation_states.map(|states| states.to_vec()),\n        )\n    }}\n\n    fn __from_validated_node_with_invocation_states(\n        node: RuleNodeView<'a>,\n        invocation_states: Option<Vec<isize>>,\n    ) -> Self {{\n{stored_attrs_bindings}        Self {{\n            __node: __GeneratedRuleContext::Stored(node),\n            __invocation_states: invocation_states,\n            __state: std::marker::PhantomData,\n{field_inits}        }}\n    }}\n\n    pub fn {rule_node_method}(&self) -> RuleNodeView<'a> {{\n        match self.__node {{\n            __GeneratedRuleContext::Stored(node) => node,\n            __GeneratedRuleContext::Active {{ .. }} => unreachable!(\"validated context contains an active parser context\"),\n        }}\n    }}\n"
        );
        let rendered_accessors = render_context_child_accessors(ContextAccessorsRender {
            view_name,
            model,
            context_names: &context_names,
            token_accessors: &token_accessors,
            child_cardinalities: &child_cardinalities,
            label_accessors: &label_accessors,
            validation_error_name: &validation_error,
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
            "#[allow(dead_code, clippy::all)]\nimpl<'a> {view_name}<'a> {{\n{accessors}}}\n\n#[allow(dead_code, clippy::all)]\nimpl<'a, State> {view_name}<'a, State> {{\n{common_accessors}}}\n\n#[allow(dead_code, private_bounds, clippy::all)]\nimpl<'a, State: __RecoveryContextState> {view_name}<'a, State> {{\n{recovered}}}\n\n#[allow(dead_code, clippy::all)]\nimpl<'a> {view_name}<'a, ValidatedTreeContext> {{\n{validated_constructors}{validated}}}\n{compatibility_impl}",
            recovered = rendered_accessors.recovered,
            validated = rendered_accessors.validated,
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
        // Java's RuleContext.toString(): bracketed invoking-state chain from
        // this context to the root, the root's sentinel excluded.
        let _ = writeln!(
            out,
            "impl<State> std::fmt::Display for {view_name}<'_, State> {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        match &self.__invocation_states {{\n            Some(states) => __write_invocation_states(f, states.iter().copied()),\n            None => match self.__node {{\n                __GeneratedRuleContext::Stored(node) => {{\n                    __write_invocation_states(f, node.invocation_states())\n                }}\n                __GeneratedRuleContext::Active {{ .. }} => {{\n                    unreachable!(\"active context is missing invocation states\")\n                }}\n            }},\n        }}\n    }}\n}}\n"
        );
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
