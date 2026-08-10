fn render_validated_tree_support(surface_name: &str) -> String {
    let validated_tree = format!("{surface_name}ValidatedTree");
    let validation_error = format!("{surface_name}ValidationError");
    format!(
        r#"/// Marker carried by generated contexts whose required-child
/// invariants were checked after a syntax-clean parse.
///
/// This marker stays module-local (unlike the runtime-owned support items)
/// so rustc can prove it never implements the runtime's
/// `__RecoveryContextState`, keeping the recovery-oriented and validated
/// accessor impls coherent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedTreeContext {{
    __private: (),
}}

/// A completed, syntax-clean parse tree whose generated child cardinalities
/// have been structurally validated.
///
/// Alias of the grammar-agnostic `antlr4_runtime::ValidatedTree`; the
/// validated-parse types of every generated parser are interchangeable.
pub type {validated_tree} = antlr4_runtime::ValidatedTree;

/// Failure to recognize or validate a strict generated parse.
///
/// Alias of the grammar-agnostic `antlr4_runtime::ValidationError`; the
/// validated-parse types of every generated parser are interchangeable.
pub type {validation_error} = antlr4_runtime::ValidationError;

"#
    )
}

fn render_context_listener_surface(
    context_names: &ContextSurfaceNames,
    listener_trait: &str,
    tree_walker: &str,
    validated_listener_trait: &str,
    validated_tree_walker: &str,
) -> String {
    let mut out = String::new();
    let mut trait_methods = String::new();
    let mut enter_arms = String::new();
    let mut exit_arms = String::new();
    let mut validated_trait_methods = String::new();
    let mut validated_enter_arms = String::new();
    let mut validated_exit_arms = String::new();
    for (kind_id, view) in context_names.views.iter().enumerate() {
        let ContextSurfaceName {
            context_type,
            listener_method,
            ..
        } = &view.surface;
        let _ = writeln!(
            trait_methods,
            "    fn enter_{listener_method}(&mut self, _ctx: &{context_type}) -> Result<(), E> {{ Ok(()) }}\n    fn exit_{listener_method}(&mut self, _ctx: &{context_type}) -> Result<(), E> {{ Ok(()) }}"
        );
        let _ = writeln!(
            enter_arms,
            "                        {kind_id} => listener.enter_{listener_method}(&{context_type}::__from_listener_node(context, invocation_states.as_deref()))?,"
        );
        let _ = writeln!(
            exit_arms,
            "                    {kind_id} => listener.exit_{listener_method}(&{context_type}::__from_listener_node(context, invocation_states.as_deref()))?,"
        );
        let _ = writeln!(
            validated_trait_methods,
            "    fn enter_{listener_method}(&mut self, _ctx: &{context_type}<ValidatedTreeContext>) -> Result<(), E> {{ Ok(()) }}\n    fn exit_{listener_method}(&mut self, _ctx: &{context_type}<ValidatedTreeContext>) -> Result<(), E> {{ Ok(()) }}"
        );
        let _ = writeln!(
            validated_enter_arms,
            "                        {kind_id} => listener.enter_{listener_method}(&{context_type}::<ValidatedTreeContext>::__from_validated_listener_node(context, invocation_states.as_deref()))?,"
        );
        let _ = writeln!(
            validated_exit_arms,
            "                    {kind_id} => listener.exit_{listener_method}(&{context_type}::<ValidatedTreeContext>::__from_validated_listener_node(context, invocation_states.as_deref()))?,"
        );
    }
    let _ = writeln!(
        out,
        "#[allow(dead_code, unused_variables)]\npub trait {listener_trait}<E = std::convert::Infallible> {{\n    fn walk(&mut self, tree: antlr4_runtime::Node<'_>) -> Result<(), E>\n    where\n        Self: Sized,\n    {{\n        {tree_walker}::walk(self, tree)\n    }}\n\n    fn enter_every_rule(&mut self, _ctx: RuleNodeView<'_>) -> Result<(), E> {{ Ok(()) }}\n    fn exit_every_rule(&mut self, _ctx: RuleNodeView<'_>) -> Result<(), E> {{ Ok(()) }}\n\n{trait_methods}    fn visit_terminal(&mut self, _node: &TerminalNode) -> Result<(), E> {{ Ok(()) }}\n    fn visit_error_node(&mut self, _node: &ErrorNode) -> Result<(), E> {{ Ok(()) }}\n    fn output(&mut self) -> std::io::Stdout {{ std::io::stdout() }}\n}}\n"
    );

    let _ = writeln!(
        out,
        r#"#[allow(dead_code)]
pub struct {tree_walker};

#[allow(dead_code)]
impl {tree_walker} {{
    pub fn walk<E, T: {listener_trait}<E>>(
        listener: &mut T,
        tree: antlr4_runtime::Node<'_>,
    ) -> Result<(), E> {{
        Self::__walk(listener, tree, None)
    }}

    pub fn walk_with_invocation_states<E, T: {listener_trait}<E>>(
        listener: &mut T,
        tree: antlr4_runtime::Node<'_>,
        parent_invocation_states: Vec<isize>,
    ) -> Result<(), E> {{
        Self::__walk(listener, tree, Some(parent_invocation_states))
    }}

    fn __walk<E, T: {listener_trait}<E>>(
        listener: &mut T,
        tree: antlr4_runtime::Node<'_>,
        mut invocation_states: Option<Vec<isize>>,
    ) -> Result<(), E> {{
        enum Event<'tree> {{
            Enter(antlr4_runtime::Node<'tree>),
            Exit(RuleNodeView<'tree>),
        }}

        let mut stack = vec![Event::Enter(tree)];
        while let Some(event) = stack.pop() {{
            match event {{
                Event::Enter(node) => match node.kind() {{
                    antlr4_runtime::NodeKind::Rule => {{
                        let context = node.as_rule().expect("rule node kind checked");
                        if let Some(states) = &mut invocation_states {{
                            states.insert(0, context.invoking_state());
                        }}
                        listener.enter_every_rule(context)?;
                        match __context_kind(context) {{
{enter_arms}                            _ => {{}}
                        }}
                        stack.push(Event::Exit(context));
                        stack.extend(context.children().rev().map(Event::Enter));
                    }}
                    antlr4_runtime::NodeKind::Terminal => {{
                        listener.visit_terminal(&TerminalNode::new(
                            node.as_terminal().expect("terminal node kind checked"),
                        ))?;
                    }}
                    antlr4_runtime::NodeKind::Error => {{
                        listener.visit_error_node(&ErrorNode::new(
                            node.as_error().expect("error node kind checked"),
                        ))?;
                    }}
                }},
                Event::Exit(context) => {{
                    match __context_kind(context) {{
{exit_arms}                        _ => {{}}
                    }}
                    listener.exit_every_rule(context)?;
                    if let Some(states) = &mut invocation_states {{
                        states.remove(0);
                    }}
                }}
            }}
        }}
        Ok(())
    }}
}}

pub type ParseTreeWalker = {tree_walker};
"#
    );
    let _ = writeln!(
        out,
        r#"#[allow(dead_code, unused_variables)]
pub trait {validated_listener_trait}<E = std::convert::Infallible> {{
    fn walk(&mut self, tree: ValidatedRuleNode<'_>) -> Result<(), E>
    where
        Self: Sized,
    {{
        {validated_tree_walker}::walk(self, tree)
    }}

    fn enter_every_rule(&mut self, _ctx: ValidatedRuleNode<'_>) -> Result<(), E> {{ Ok(()) }}
    fn exit_every_rule(&mut self, _ctx: ValidatedRuleNode<'_>) -> Result<(), E> {{ Ok(()) }}

{validated_trait_methods}    fn visit_terminal(&mut self, _node: &TerminalNode) -> Result<(), E> {{ Ok(()) }}
    fn output(&mut self) -> std::io::Stdout {{ std::io::stdout() }}
}}

#[allow(dead_code)]
pub struct {validated_tree_walker};

#[allow(dead_code)]
impl {validated_tree_walker} {{
    pub fn walk<E, T: {validated_listener_trait}<E>>(
        listener: &mut T,
        tree: ValidatedRuleNode<'_>,
    ) -> Result<(), E> {{
        Self::__walk(listener, tree.node(), None)
    }}

    pub fn walk_with_invocation_states<E, T: {validated_listener_trait}<E>>(
        listener: &mut T,
        tree: ValidatedRuleNode<'_>,
        parent_invocation_states: Vec<isize>,
    ) -> Result<(), E> {{
        Self::__walk(listener, tree.node(), Some(parent_invocation_states))
    }}

    fn __walk<E, T: {validated_listener_trait}<E>>(
        listener: &mut T,
        tree: antlr4_runtime::Node<'_>,
        mut invocation_states: Option<Vec<isize>>,
    ) -> Result<(), E> {{
        enum Event<'tree> {{
            Enter(antlr4_runtime::Node<'tree>),
            Exit(RuleNodeView<'tree>),
        }}

        let mut stack = vec![Event::Enter(tree)];
        while let Some(event) = stack.pop() {{
            match event {{
                Event::Enter(node) => match node.kind() {{
                    antlr4_runtime::NodeKind::Rule => {{
                        let context = node.as_rule().expect("rule node kind checked");
                        if let Some(states) = &mut invocation_states {{
                            states.insert(0, context.invoking_state());
                        }}
                        listener.enter_every_rule(ValidatedRuleNode::__new(context))?;
                        match __context_kind(context) {{
{validated_enter_arms}                            _ => {{}}
                        }}
                        stack.push(Event::Exit(context));
                        stack.extend(context.children().rev().map(Event::Enter));
                    }}
                    antlr4_runtime::NodeKind::Terminal => {{
                        listener.visit_terminal(&TerminalNode::new(
                            node.as_terminal().expect("terminal node kind checked"),
                        ))?;
                    }}
                    antlr4_runtime::NodeKind::Error => {{
                        unreachable!("validated parse tree contains an error node")
                    }}
                }},
                Event::Exit(context) => {{
                    match __context_kind(context) {{
{validated_exit_arms}                        _ => {{}}
                    }}
                    listener.exit_every_rule(ValidatedRuleNode::__new(context))?;
                    if let Some(states) = &mut invocation_states {{
                        states.remove(0);
                    }}
                }}
            }}
        }}
        Ok(())
    }}
}}

pub type ValidatedParseTreeWalker = {validated_tree_walker};
"#
    );
    out
}

fn render_context_visitor_surface(
    context_names: &ContextSurfaceNames,
    visitor_trait: &str,
    visitable_trait: &str,
    validated_visitor_trait: &str,
    validated_visitable_trait: &str,
) -> String {
    let mut out = String::new();
    let mut visitor_methods = String::new();
    let mut visitor_arms = String::new();
    let mut validated_visitor_methods = String::new();
    let mut validated_visitor_arms = String::new();
    for (kind_id, view) in context_names.views.iter().enumerate() {
        let ContextSurfaceName {
            context_type,
            visitor_method,
            ..
        } = &view.surface;
        let _ = writeln!(
            visitor_methods,
            "    fn visit_{visitor_method}(&mut self, ctx: &{context_type}) -> Self::Result {{\n        self.visit_children(ctx)\n    }}"
        );
        let _ = writeln!(
            visitor_arms,
            "            {kind_id} => {visitor_trait}::visit_{visitor_method}(self.0, &{context_type}::__from_listener_node(context, None)),"
        );
        let _ = writeln!(
            validated_visitor_methods,
            "    fn visit_{visitor_method}(&mut self, ctx: &{context_type}<ValidatedTreeContext>) -> Self::Result {{\n        self.visit_children(ctx)\n    }}"
        );
        let _ = writeln!(
            validated_visitor_arms,
            "            {kind_id} => {validated_visitor_trait}::visit_{visitor_method}(self.0, &{context_type}::<ValidatedTreeContext>::__from_validated_listener_node(context, None)),"
        );
    }
    let _ = writeln!(
        out,
        r#"#[allow(dead_code, unused_variables)]
pub trait {visitor_trait}: Sized {{
    type Result;

    fn default_result(&mut self) -> Self::Result;

    fn visit<'tree, T>(&mut self, tree: T) -> Self::Result
    where
        T: {visitable_trait}<'tree>,
    {{
        let tree = {visitable_trait}::into_parse_tree_node(tree);
        let mut bridge = __VisitorBridge(self);
        antlr4_runtime::ParseTreeVisitor::visit(&mut bridge, tree)
    }}

    fn visit_children<'tree, T>(&mut self, context: T) -> Self::Result
    where
        T: {visitable_trait}<'tree>,
    {{
        let tree = {visitable_trait}::into_parse_tree_node(context);
        let context = tree.as_rule().expect("visit_children requires a rule context");
        let mut bridge = __VisitorBridge(self);
        antlr4_runtime::ParseTreeVisitor::visit_children(&mut bridge, context)
    }}

    fn aggregate_result(
        &mut self,
        _aggregate: Self::Result,
        next_result: Self::Result,
    ) -> Self::Result {{
        next_result
    }}

    fn should_visit_next_child(
        &mut self,
        _context: RuleNodeView<'_>,
        _current_result: &Self::Result,
    ) -> bool {{
        true
    }}

    fn visit_terminal(&mut self, _node: &TerminalNode) -> Self::Result {{
        self.default_result()
    }}

    fn visit_error_node(&mut self, _node: &ErrorNode) -> Self::Result {{
        self.default_result()
    }}

{visitor_methods}}}

#[allow(dead_code)]
struct __VisitorBridge<'a, T: {visitor_trait}>(&'a mut T);

impl<T: {visitor_trait}> antlr4_runtime::ParseTreeVisitor for __VisitorBridge<'_, T> {{
    type Result = T::Result;

    fn visit_rule(&mut self, context: RuleNodeView<'_>) -> Self::Result {{
        match __context_kind(context) {{
{visitor_arms}            _ => {visitor_trait}::default_result(self.0),
        }}
    }}

    fn visit_terminal(&mut self, node: RuntimeTerminalNode<'_>) -> Self::Result {{
        {visitor_trait}::visit_terminal(self.0, &TerminalNode::new(node))
    }}

    fn visit_error_node(&mut self, node: RuntimeErrorNode<'_>) -> Self::Result {{
        {visitor_trait}::visit_error_node(self.0, &ErrorNode::new(node))
    }}

    fn default_result(&mut self) -> Self::Result {{
        {visitor_trait}::default_result(self.0)
    }}

    fn aggregate_result(
        &mut self,
        aggregate: Self::Result,
        next_result: Self::Result,
    ) -> Self::Result {{
        {visitor_trait}::aggregate_result(self.0, aggregate, next_result)
    }}

    fn should_visit_next_child(
        &mut self,
        context: RuleNodeView<'_>,
        current_result: &Self::Result,
    ) -> bool {{
        {visitor_trait}::should_visit_next_child(self.0, context, current_result)
    }}
}}
"#
    );
    let _ = writeln!(
        out,
        r#"#[allow(dead_code, unused_variables)]
pub trait {validated_visitor_trait}: Sized {{
    type Result;

    fn default_result(&mut self) -> Self::Result;

    fn visit<'tree, T>(&mut self, tree: T) -> Self::Result
    where
        T: {validated_visitable_trait}<'tree>,
    {{
        let tree = {validated_visitable_trait}::into_validated_parse_tree_node(tree);
        let mut bridge = __ValidatedVisitorBridge(self);
        antlr4_runtime::ParseTreeVisitor::visit(&mut bridge, tree)
    }}

    fn visit_children<'tree, T>(&mut self, context: T) -> Self::Result
    where
        T: {validated_visitable_trait}<'tree>,
    {{
        let tree = {validated_visitable_trait}::into_validated_parse_tree_node(context);
        let context = tree.as_rule().expect("visit_children requires a rule context");
        let mut bridge = __ValidatedVisitorBridge(self);
        antlr4_runtime::ParseTreeVisitor::visit_children(&mut bridge, context)
    }}

    fn aggregate_result(
        &mut self,
        _aggregate: Self::Result,
        next_result: Self::Result,
    ) -> Self::Result {{
        next_result
    }}

    fn should_visit_next_child(
        &mut self,
        _context: ValidatedRuleNode<'_>,
        _current_result: &Self::Result,
    ) -> bool {{
        true
    }}

    fn visit_terminal(&mut self, _node: &TerminalNode) -> Self::Result {{
        self.default_result()
    }}

{validated_visitor_methods}}}

#[allow(dead_code)]
struct __ValidatedVisitorBridge<'a, T: {validated_visitor_trait}>(&'a mut T);

impl<T: {validated_visitor_trait}> antlr4_runtime::ParseTreeVisitor
    for __ValidatedVisitorBridge<'_, T>
{{
    type Result = T::Result;

    fn visit_rule(&mut self, context: RuleNodeView<'_>) -> Self::Result {{
        match __context_kind(context) {{
{validated_visitor_arms}            _ => {validated_visitor_trait}::default_result(self.0),
        }}
    }}

    fn visit_terminal(&mut self, node: RuntimeTerminalNode<'_>) -> Self::Result {{
        {validated_visitor_trait}::visit_terminal(self.0, &TerminalNode::new(node))
    }}

    fn visit_error_node(&mut self, _node: RuntimeErrorNode<'_>) -> Self::Result {{
        unreachable!("validated parse tree contains an error node")
    }}

    fn default_result(&mut self) -> Self::Result {{
        {validated_visitor_trait}::default_result(self.0)
    }}

    fn aggregate_result(
        &mut self,
        aggregate: Self::Result,
        next_result: Self::Result,
    ) -> Self::Result {{
        {validated_visitor_trait}::aggregate_result(self.0, aggregate, next_result)
    }}

    fn should_visit_next_child(
        &mut self,
        context: RuleNodeView<'_>,
        current_result: &Self::Result,
    ) -> bool {{
        {validated_visitor_trait}::should_visit_next_child(
            self.0,
            ValidatedRuleNode::__new(context),
            current_result,
        )
    }}
}}
"#
    );
    out
}
