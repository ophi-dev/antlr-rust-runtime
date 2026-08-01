use std::collections::BTreeSet;
use std::io;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use antlr4_runtime::{
    CommonTokenStream, ErrorListener, InputStream, Node, Parser as _, Recognizer, SyntaxErrorEvent,
    TokenId, TokenStore,
};

mod generated {
    pub(crate) mod lexer {
        include!("generated/rust_lexer.rs");
    }

    pub(crate) mod parser {
        include!("generated/rust_parser.rs");
    }
}

use generated::lexer::{IDENT, LIFETIME, RAW_IDENTIFIER, RustLexer};
use generated::parser::{
    RULE_ASSOCIATED_CONST_DECL, RULE_ASSOCIATED_STATIC_DECL, RULE_ATTR, RULE_BLOCK,
    RULE_BLOCK_WITH_INNER_ATTRS, RULE_CLOSURE_PARAM, RULE_CLOSURE_PARAMS, RULE_CLOSURE_TAIL,
    RULE_CONST_DECL, RULE_ENUM_DECL, RULE_ENUM_VARIANT_MAIN, RULE_EXPR, RULE_EXTERN_CRATE,
    RULE_FIELD, RULE_FIELD_NAME, RULE_FN_DECL, RULE_FN_HEAD, RULE_FOREIGN_FN_DECL,
    RULE_FOREIGN_ITEM_TAIL, RULE_IDENT, RULE_IMPL_BLOCK, RULE_IMPL_ITEM_TAIL, RULE_INNER_ATTR,
    RULE_LIFETIME, RULE_MACRO_DECL, RULE_MACRO_INVOCATION, RULE_MACRO_INVOCATION_SEMI,
    RULE_MACRO_RULES_DEFINITION, RULE_MACRO_TAIL, RULE_METHOD_DECL, RULE_METHOD_PARAM_LIST,
    RULE_MOD_DECL, RULE_MOD_DECL_SHORT, RULE_PARAM, RULE_PARAM_LIST, RULE_PATTERN,
    RULE_PATTERN_NO_TOP_ALT, RULE_PRIM_EXPR_NO_STRUCT, RULE_STATIC_DECL, RULE_STRUCT_DECL,
    RULE_STRUCT_TAIL, RULE_TRAIT_ALIAS, RULE_TRAIT_DECL, RULE_TRAIT_ITEM, RULE_TRAIT_METHOD_DECL,
    RULE_TRAIT_METHOD_PARAM, RULE_TRAIT_METHOD_PARAM_LIST, RULE_TYPE_DECL, RULE_TYPE_PARAMETER,
    RULE_TYPE_PATH_MAIN, RULE_UNION_DECL, RULE_VARIADIC_PARAM_LIST, RustParser,
};

const WRAPPER_PREFIX: &str = "{\n";

#[derive(Debug, Default)]
pub(crate) struct RustSyntax {
    type_identifier_byte_starts: BTreeSet<usize>,
    declaration_identifier_byte_starts: BTreeSet<usize>,
    non_value_identifier_byte_starts: BTreeSet<usize>,
    opaque_macro_identifier_byte_starts: BTreeSet<usize>,
    struct_field_shorthand_byte_starts: BTreeSet<usize>,
    pattern_field_shorthand_byte_starts: BTreeSet<usize>,
    value_binding_byte_starts: BTreeSet<usize>,
    scoped_value_bindings: Vec<ScopedValueBinding>,
    function_bindings: Vec<FunctionBinding>,
    closure_bindings: Vec<ClosureBinding>,
}

#[derive(Debug)]
pub(crate) struct ScopedValueBinding {
    pub(crate) declaration_start: usize,
    pub(crate) scope: Range<usize>,
}

#[derive(Debug)]
pub(crate) struct ClosureBinding {
    pub(crate) parameter_ranges: Vec<Range<usize>>,
    pub(crate) scope: Range<usize>,
}

#[derive(Debug)]
pub(crate) struct FunctionBinding {
    pub(crate) parameter_ranges: Vec<Range<usize>>,
    pub(crate) scope: Range<usize>,
}

#[derive(Clone, Debug)]
struct RustSyntaxDiagnostic {
    line: usize,
    column: usize,
    message: String,
}

#[derive(Clone, Debug, Default)]
struct RustSyntaxDiagnosticCollector(Arc<Mutex<Vec<RustSyntaxDiagnostic>>>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnalysisRoot {
    Body,
    MemberItem,
}

#[derive(Debug)]
struct ScopedMacroBinding {
    name: String,
    scope: Range<usize>,
}

impl RustSyntaxDiagnosticCollector {
    fn take(&self) -> Vec<RustSyntaxDiagnostic> {
        std::mem::take(
            &mut *self
                .0
                .lock()
                .expect("Rust syntax diagnostic collector mutex poisoned"),
        )
    }
}

impl<R> ErrorListener<R> for RustSyntaxDiagnosticCollector
where
    R: Recognizer + ?Sized,
{
    fn syntax_error(&mut self, _recognizer: &R, event: &SyntaxErrorEvent<'_>) {
        self.0
            .lock()
            .expect("Rust syntax diagnostic collector mutex poisoned")
            .push(RustSyntaxDiagnostic {
                line: event.line,
                column: event.column,
                message: event.message.to_owned(),
            });
    }
}

impl RustSyntax {
    pub(crate) fn is_type_identifier(&self, byte_start: usize) -> bool {
        self.type_identifier_byte_starts.contains(&byte_start)
    }

    pub(crate) fn is_declaration_identifier(&self, byte_start: usize) -> bool {
        self.declaration_identifier_byte_starts
            .contains(&byte_start)
    }

    pub(crate) fn is_non_value_identifier(&self, byte_start: usize) -> bool {
        self.non_value_identifier_byte_starts.contains(&byte_start)
    }

    pub(crate) fn is_opaque_macro_identifier(&self, byte_start: usize) -> bool {
        self.opaque_macro_identifier_byte_starts
            .contains(&byte_start)
    }

    pub(crate) fn is_struct_field_shorthand(&self, byte_start: usize) -> bool {
        self.struct_field_shorthand_byte_starts
            .contains(&byte_start)
    }

    pub(crate) fn is_pattern_field_shorthand(&self, byte_start: usize) -> bool {
        self.pattern_field_shorthand_byte_starts
            .contains(&byte_start)
    }

    pub(crate) fn value_binding_byte_starts(&self) -> impl Iterator<Item = usize> + '_ {
        self.value_binding_byte_starts.iter().copied()
    }

    pub(crate) fn scoped_value_bindings(&self) -> &[ScopedValueBinding] {
        &self.scoped_value_bindings
    }

    pub(crate) fn function_bindings(&self) -> &[FunctionBinding] {
        &self.function_bindings
    }

    pub(crate) fn closure_bindings(&self) -> &[ClosureBinding] {
        &self.closure_bindings
    }
}

pub(crate) fn analyze(body: &str) -> io::Result<RustSyntax> {
    analyze_with_root(body, AnalysisRoot::Body)
}

pub(crate) fn analyze_member_item(body: &str) -> io::Result<RustSyntax> {
    analyze_with_root(body, AnalysisRoot::MemberItem)
}

fn analyze_with_root(body: &str, analysis_root: AnalysisRoot) -> io::Result<RustSyntax> {
    let wrapped = format!("{WRAPPER_PREFIX}{body}\n}}");
    let mut lexer = RustLexer::new(InputStream::new(&wrapped));
    lexer.remove_error_listeners();
    let diagnostics = RustSyntaxDiagnosticCollector::default();
    let mut parser = RustParser::new(CommonTokenStream::new(lexer));
    parser.remove_error_listeners();
    parser.add_error_listener(diagnostics.clone());
    let root = parser.block();
    let parser_error_count = parser.number_of_syntax_errors();
    let lexer_error_count = parser.token_stream().number_of_source_errors();
    let lexer_errors = parser.token_stream_mut().drain_source_errors();
    let diagnostics = diagnostics.take();
    if lexer_error_count > 0 {
        return Err(lexer_errors.first().map_or_else(
            || {
                rust_syntax_error(&format!(
                    "lexer reported {lexer_error_count} syntax error(s)"
                ))
            },
            |error| rust_syntax_diagnostic(error.line, error.column, &error.message),
        ));
    }
    if parser_error_count > 0 {
        return Err(diagnostics.first().map_or_else(
            || {
                rust_syntax_error(&format!(
                    "parser reported {parser_error_count} syntax error(s)"
                ))
            },
            |diagnostic| {
                rust_syntax_diagnostic(diagnostic.line, diagnostic.column, &diagnostic.message)
            },
        ));
    }
    let root = root.map_err(|error| rust_syntax_error(&error.to_string()))?;
    let parsed = parser.into_parsed_file(root);
    let mut syntax = RustSyntax::default();
    let macro_bindings = collect_scoped_macro_bindings(parsed.tree(), parsed.tokens(), body.len());

    for node in parsed.tree().descendants() {
        let Some(rule) = node.as_rule() else {
            continue;
        };
        match rule.rule_index() {
            RULE_TYPE_PATH_MAIN => {
                collect_identifier_starts_excluding_rules(
                    rule.node(),
                    parsed.tokens(),
                    body.len(),
                    &[RULE_EXPR],
                    &mut syntax.type_identifier_byte_starts,
                );
            }
            RULE_TYPE_PARAMETER => {
                if let Some(identifier) = rule.child_rule(RULE_IDENT) {
                    collect_identifier_starts(
                        identifier.node(),
                        parsed.tokens(),
                        body.len(),
                        &mut syntax.type_identifier_byte_starts,
                    );
                }
                collect_const_generic_binding(rule, parsed.tokens(), body.len(), &mut syntax);
            }
            RULE_TYPE_DECL | RULE_ENUM_DECL | RULE_UNION_DECL | RULE_TRAIT_DECL
            | RULE_TRAIT_ALIAS | RULE_MOD_DECL | RULE_MOD_DECL_SHORT | RULE_EXTERN_CRATE
            | RULE_MACRO_DECL => {
                collect_direct_identifier_start(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.type_identifier_byte_starts,
                );
            }
            RULE_STRUCT_DECL => {
                collect_direct_identifier_start(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.type_identifier_byte_starts,
                );
                if parsed_struct_has_value_constructor(rule) {
                    collect_direct_identifier_start(
                        rule,
                        parsed.tokens(),
                        body.len(),
                        &mut syntax.value_binding_byte_starts,
                    );
                }
            }
            RULE_ENUM_VARIANT_MAIN => {
                collect_direct_identifier_start(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.declaration_identifier_byte_starts,
                );
            }
            RULE_FN_DECL | RULE_METHOD_DECL | RULE_TRAIT_METHOD_DECL | RULE_FOREIGN_FN_DECL => {
                collect_function_binding(rule, parsed.tokens(), body.len(), &mut syntax);
            }
            RULE_FIELD if rule.child_rule(RULE_FIELD_NAME).is_none() => {
                collect_direct_identifier_start(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.struct_field_shorthand_byte_starts,
                );
            }
            RULE_FN_HEAD => {
                collect_direct_identifier_start(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.declaration_identifier_byte_starts,
                );
                if fn_head_introduces_unqualified_value(rule, analysis_root) {
                    collect_direct_identifier_start(
                        rule,
                        parsed.tokens(),
                        body.len(),
                        &mut syntax.value_binding_byte_starts,
                    );
                }
            }
            RULE_STATIC_DECL | RULE_CONST_DECL => {
                let starts = if item_introduces_unqualified_value(rule) {
                    &mut syntax.value_binding_byte_starts
                } else {
                    &mut syntax.declaration_identifier_byte_starts
                };
                collect_direct_identifier_start(rule, parsed.tokens(), body.len(), starts);
            }
            RULE_ASSOCIATED_STATIC_DECL | RULE_ASSOCIATED_CONST_DECL => {
                collect_direct_identifier_start(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.declaration_identifier_byte_starts,
                );
            }
            RULE_PRIM_EXPR_NO_STRUCT => {
                collect_closure_binding(rule, parsed.tokens(), body.len(), &mut syntax);
            }
            RULE_ATTR | RULE_INNER_ATTR | RULE_MACRO_RULES_DEFINITION => {
                collect_identifier_starts(
                    rule.node(),
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.opaque_macro_identifier_byte_starts,
                );
            }
            RULE_MACRO_TAIL => {
                collect_opaque_macro_identifiers(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &macro_bindings,
                    &mut syntax,
                );
            }
            RULE_MACRO_INVOCATION | RULE_MACRO_INVOCATION_SEMI => {
                collect_opaque_macro_invocation_identifiers(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &macro_bindings,
                    &mut syntax,
                );
            }
            RULE_LIFETIME => {
                collect_lifetime_identifier_start(rule, parsed.tokens(), body.len(), &mut syntax);
            }
            _ => {}
        }
    }
    collect_pattern_field_shorthands(parsed.tree(), parsed.tokens(), body.len(), &mut syntax);
    Ok(syntax)
}

fn fn_head_introduces_unqualified_value(
    fn_head: antlr4_runtime::RuleNodeView<'_>,
    analysis_root: AnalysisRoot,
) -> bool {
    let Some(owner) = enclosing_function_declaration(fn_head) else {
        return false;
    };
    match owner.rule_index() {
        RULE_FN_DECL => {
            analysis_root == AnalysisRoot::Body || enclosing_function_declaration(owner).is_some()
        }
        RULE_FOREIGN_FN_DECL => true,
        RULE_METHOD_DECL | RULE_TRAIT_METHOD_DECL => false,
        _ => false,
    }
}

fn item_introduces_unqualified_value(rule: antlr4_runtime::RuleNodeView<'_>) -> bool {
    let mut node = rule.node().parent();
    while let Some(parent) = node {
        if let Some(parent) = parent.as_rule() {
            match parent.rule_index() {
                RULE_BLOCK | RULE_BLOCK_WITH_INNER_ATTRS => return true,
                RULE_IMPL_ITEM_TAIL | RULE_TRAIT_ITEM => return false,
                _ => {}
            }
        }
        node = parent.parent();
    }
    true
}

fn enclosing_function_declaration(
    rule: antlr4_runtime::RuleNodeView<'_>,
) -> Option<antlr4_runtime::RuleNodeView<'_>> {
    let mut node = rule.node().parent();
    while let Some(parent) = node {
        let parent_rule = parent.as_rule();
        if parent_rule.is_some_and(|parent| {
            matches!(
                parent.rule_index(),
                RULE_FN_DECL | RULE_METHOD_DECL | RULE_TRAIT_METHOD_DECL | RULE_FOREIGN_FN_DECL
            )
        }) {
            return parent_rule;
        }
        node = parent.parent();
    }
    None
}

fn rust_syntax_diagnostic(line: usize, column: usize, message: &str) -> io::Error {
    let body_line = line.saturating_sub(1).max(1);
    rust_syntax_error(&format!("at {body_line}:{column}: {message}"))
}

fn rust_syntax_error(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("cannot classify embedded Rust syntax: {message}"),
    )
}

fn collect_const_generic_binding(
    parameter: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
    syntax: &mut RustSyntax,
) {
    let is_const = parameter
        .children()
        .filter_map(Node::as_terminal)
        .any(|terminal| terminal.text() == "const");
    if !is_const {
        return;
    }
    let Some(declaration_start) = direct_identifier_byte_start(parameter, tokens, body_len) else {
        return;
    };
    let Some(owner) = enclosing_const_generic_owner(parameter) else {
        return;
    };
    let Some(mut scope) = body_byte_range(owner, tokens, body_len) else {
        return;
    };
    scope.start = declaration_start;
    syntax.scoped_value_bindings.push(ScopedValueBinding {
        declaration_start,
        scope,
    });
}

fn enclosing_const_generic_owner(
    rule: antlr4_runtime::RuleNodeView<'_>,
) -> Option<antlr4_runtime::RuleNodeView<'_>> {
    let mut node = rule.node().parent();
    while let Some(parent) = node {
        let parent_rule = parent.as_rule();
        if parent_rule.is_some_and(|parent| {
            matches!(
                parent.rule_index(),
                RULE_FN_DECL
                    | RULE_METHOD_DECL
                    | RULE_TRAIT_METHOD_DECL
                    | RULE_FOREIGN_FN_DECL
                    | RULE_TYPE_DECL
                    | RULE_STRUCT_DECL
                    | RULE_ENUM_DECL
                    | RULE_UNION_DECL
                    | RULE_TRAIT_DECL
                    | RULE_IMPL_BLOCK
                    | RULE_TRAIT_ITEM
                    | RULE_IMPL_ITEM_TAIL
                    | RULE_FOREIGN_ITEM_TAIL
                    | RULE_MACRO_DECL
            )
        }) {
            return parent_rule;
        }
        node = parent.parent();
    }
    None
}

fn collect_opaque_macro_identifiers(
    macro_tail: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
    macro_bindings: &[ScopedMacroBinding],
    syntax: &mut RustSyntax,
) {
    let Some(parent) = macro_tail.node().parent().and_then(Node::as_rule) else {
        return;
    };
    let Some((macro_name, unqualified)) = macro_invocation_name(parent, tokens) else {
        return;
    };
    if !macro_allows_value_alias_lowering(&macro_name)
        || unqualified && local_macro_shadows(&macro_name, parent, tokens, body_len, macro_bindings)
    {
        collect_identifier_starts(
            macro_tail.node(),
            tokens,
            body_len,
            &mut syntax.opaque_macro_identifier_byte_starts,
        );
    }
}

fn macro_allows_value_alias_lowering(name: &str) -> bool {
    matches!(
        name,
        "assert"
            | "assert_eq"
            | "assert_ne"
            | "dbg"
            | "debug_assert"
            | "debug_assert_eq"
            | "debug_assert_ne"
            | "eprint"
            | "eprintln"
            | "format"
            | "format_args"
            | "matches"
            | "panic"
            | "print"
            | "println"
            | "todo"
            | "unreachable"
            | "vec"
            | "write"
            | "writeln"
    )
}

fn collect_opaque_macro_invocation_identifiers(
    invocation: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
    macro_bindings: &[ScopedMacroBinding],
    syntax: &mut RustSyntax,
) {
    let Some((macro_name, unqualified)) = macro_invocation_name(invocation, tokens) else {
        return;
    };
    if !macro_allows_value_alias_lowering(&macro_name)
        || unqualified
            && local_macro_shadows(&macro_name, invocation, tokens, body_len, macro_bindings)
    {
        collect_identifier_starts(
            invocation.node(),
            tokens,
            body_len,
            &mut syntax.opaque_macro_identifier_byte_starts,
        );
    }
}

fn macro_invocation_name(
    invocation: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
) -> Option<(String, bool)> {
    let mut macro_name = None;
    let mut qualified = false;
    for terminal in invocation
        .node()
        .descendants()
        .filter_map(Node::as_terminal)
    {
        if terminal.text() == "!" {
            break;
        }
        if terminal.text() == ":" {
            qualified = true;
        }
        if matches!(
            tokens.token_type(terminal.token_id()),
            Some(IDENT | RAW_IDENTIFIER)
        ) {
            macro_name = Some(
                terminal
                    .text()
                    .strip_prefix("r#")
                    .unwrap_or_else(|| terminal.text())
                    .to_owned(),
            );
        }
    }
    macro_name.map(|name| (name, !qualified))
}

fn local_macro_shadows(
    name: &str,
    invocation: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
    bindings: &[ScopedMacroBinding],
) -> bool {
    let Some(start) = invocation
        .start_id()
        .and_then(|token| body_byte_start(tokens, token, body_len))
    else {
        return false;
    };
    bindings
        .iter()
        .any(|binding| binding.name == name && binding.scope.contains(&start))
}

fn collect_scoped_macro_bindings(
    root: Node<'_>,
    tokens: &TokenStore,
    body_len: usize,
) -> Vec<ScopedMacroBinding> {
    root.descendants()
        .filter_map(Node::as_rule)
        .filter(|rule| rule.rule_index() == RULE_MACRO_RULES_DEFINITION)
        .filter_map(|definition| {
            let name = macro_rules_name(definition, tokens)?;
            let declaration = body_byte_range(definition, tokens, body_len)?;
            let mut scope = enclosing_block(definition)
                .and_then(|block| body_byte_range(block, tokens, body_len))
                .unwrap_or(0..body_len);
            scope.start = declaration.end.min(scope.end);
            Some(ScopedMacroBinding { name, scope })
        })
        .collect()
}

fn macro_rules_name(
    definition: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
) -> Option<String> {
    let mut after_bang = false;
    for terminal in definition
        .node()
        .descendants()
        .filter_map(Node::as_terminal)
    {
        if terminal.text() == "!" {
            after_bang = true;
            continue;
        }
        if after_bang
            && matches!(
                tokens.token_type(terminal.token_id()),
                Some(IDENT | RAW_IDENTIFIER)
            )
        {
            return Some(
                terminal
                    .text()
                    .strip_prefix("r#")
                    .unwrap_or_else(|| terminal.text())
                    .to_owned(),
            );
        }
    }
    None
}

fn enclosing_block(
    rule: antlr4_runtime::RuleNodeView<'_>,
) -> Option<antlr4_runtime::RuleNodeView<'_>> {
    let mut node = rule.node().parent();
    while let Some(parent) = node {
        let parent_rule = parent.as_rule();
        if parent_rule.is_some_and(|parent| {
            matches!(
                parent.rule_index(),
                RULE_BLOCK | RULE_BLOCK_WITH_INNER_ATTRS
            )
        }) {
            return parent_rule;
        }
        node = parent.parent();
    }
    None
}

fn collect_lifetime_identifier_start(
    lifetime: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
    syntax: &mut RustSyntax,
) {
    let token = lifetime
        .node()
        .descendants()
        .filter_map(Node::as_terminal)
        .map(antlr4_runtime::TerminalNodeView::token_id)
        .find(|token| tokens.token_type(*token) == Some(LIFETIME));
    if let Some(start) = token
        .and_then(|token| body_byte_start(tokens, token, body_len))
        .and_then(|start| start.checked_add(1))
        .filter(|start| *start < body_len)
    {
        syntax.non_value_identifier_byte_starts.insert(start);
    }
}

fn collect_pattern_field_shorthands(
    root: Node<'_>,
    tokens: &TokenStore,
    body_len: usize,
    syntax: &mut RustSyntax,
) {
    for node in root.descendants() {
        let Some(rule) = node.as_rule() else {
            continue;
        };
        if rule.rule_index() == generated::parser::RULE_PAT_FIELD
            && rule.child_rule(RULE_PATTERN).is_none()
        {
            collect_direct_identifier_start(
                rule,
                tokens,
                body_len,
                &mut syntax.pattern_field_shorthand_byte_starts,
            );
        }
    }
}

fn collect_closure_binding(
    expression: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
    syntax: &mut RustSyntax,
) {
    let Some(parameters) = expression.child_rule(RULE_CLOSURE_PARAMS) else {
        return;
    };
    let Some(tail) = expression.child_rule(RULE_CLOSURE_TAIL) else {
        return;
    };
    let Some(scope) = body_byte_range(tail, tokens, body_len) else {
        return;
    };
    let parameter_ranges = parameters
        .node()
        .descendants()
        .filter_map(Node::as_rule)
        .filter(|rule| rule.rule_index() == RULE_CLOSURE_PARAM)
        .filter_map(|parameter| {
            let pattern = parameter.child_rule(RULE_PATTERN_NO_TOP_ALT)?;
            let mut range = body_byte_range(pattern, tokens, body_len)?;
            range.end = body_byte_range(parameter, tokens, body_len)?.end;
            Some(range)
        })
        .collect::<Vec<_>>();
    if !parameter_ranges.is_empty() {
        syntax.closure_bindings.push(ClosureBinding {
            parameter_ranges,
            scope,
        });
    }
}

fn collect_function_binding(
    function: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
    syntax: &mut RustSyntax,
) {
    let parameters = [
        RULE_PARAM_LIST,
        RULE_METHOD_PARAM_LIST,
        RULE_TRAIT_METHOD_PARAM_LIST,
        RULE_VARIADIC_PARAM_LIST,
    ]
    .into_iter()
    .find_map(|rule| function.child_rule(rule));
    let Some(parameters) = parameters else {
        return;
    };
    let Some(body) = function.child_rule(RULE_BLOCK_WITH_INNER_ATTRS) else {
        return;
    };
    let Some(scope) = body_byte_range(body, tokens, body_len) else {
        return;
    };
    let parameter_ranges = parameters
        .node()
        .descendants()
        .filter_map(Node::as_rule)
        .filter(|rule| matches!(rule.rule_index(), RULE_PARAM | RULE_TRAIT_METHOD_PARAM))
        .filter_map(|parameter| body_byte_range(parameter, tokens, body_len))
        .collect::<Vec<_>>();
    if !parameter_ranges.is_empty() {
        syntax.function_bindings.push(FunctionBinding {
            parameter_ranges,
            scope,
        });
    }
}

pub(crate) fn struct_has_value_constructor(item: &str) -> io::Result<bool> {
    Ok(analyze(item)?.value_binding_byte_starts().next().is_some())
}

fn parsed_struct_has_value_constructor(rule: antlr4_runtime::RuleNodeView<'_>) -> bool {
    let Some(tail) = rule.child_rule(RULE_STRUCT_TAIL) else {
        return false;
    };
    tail.node()
        .children()
        .filter_map(Node::as_terminal)
        .map(antlr4_runtime::TerminalNodeView::text)
        .find(|text| matches!(*text, "(" | "{" | ";"))
        .is_some_and(|delimiter| delimiter != "{")
}

fn collect_direct_identifier_start(
    rule: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
    starts: &mut BTreeSet<usize>,
) {
    if let Some(start) = direct_identifier_byte_start(rule, tokens, body_len) {
        starts.insert(start);
    }
}

fn direct_identifier_byte_start(
    rule: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
) -> Option<usize> {
    let identifier = rule.child_rule(RULE_IDENT)?;
    let token = identifier
        .node()
        .descendants()
        .filter_map(Node::as_terminal)
        .map(antlr4_runtime::TerminalNodeView::token_id)
        .find(|token| matches!(tokens.token_type(*token), Some(IDENT | RAW_IDENTIFIER)))?;
    body_byte_start(tokens, token, body_len)
}

fn collect_identifier_starts(
    root: Node<'_>,
    tokens: &TokenStore,
    body_len: usize,
    starts: &mut BTreeSet<usize>,
) {
    collect_identifier_starts_excluding_rules(root, tokens, body_len, &[], starts);
}

fn collect_identifier_starts_excluding_rules(
    root: Node<'_>,
    tokens: &TokenStore,
    body_len: usize,
    excluded_rules: &[usize],
    starts: &mut BTreeSet<usize>,
) {
    for node in root.children() {
        if node
            .as_rule()
            .is_some_and(|rule| excluded_rules.contains(&rule.rule_index()))
        {
            continue;
        }
        let token = node
            .as_terminal()
            .map(antlr4_runtime::TerminalNodeView::token_id)
            .or_else(|| node.as_error().map(antlr4_runtime::ErrorNodeView::token_id));
        if let Some(token) =
            token.filter(|token| matches!(tokens.token_type(*token), Some(IDENT | RAW_IDENTIFIER)))
            && let Some(start) = body_byte_start(tokens, token, body_len)
        {
            starts.insert(start);
        }
        collect_identifier_starts_excluding_rules(node, tokens, body_len, excluded_rules, starts);
    }
}

fn body_byte_start(tokens: &TokenStore, token: TokenId, body_len: usize) -> Option<usize> {
    tokens
        .start_byte(token)?
        .checked_sub(WRAPPER_PREFIX.len())
        .filter(|start| *start < body_len)
}

fn body_byte_range(
    rule: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
) -> Option<Range<usize>> {
    let start = body_byte_start(tokens, rule.start_id()?, body_len)?;
    let stop = tokens
        .stop_byte(rule.stop_id()?)?
        .checked_sub(WRAPPER_PREFIX.len())?
        .min(body_len);
    (start < stop).then_some(start..stop)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
mod tests {
    fn analyze(body: &str) -> super::RustSyntax {
        super::analyze(body).expect("Rust syntax fixture should parse without recovery")
    }

    fn occurrence(body: &str, needle: &str, index: usize) -> usize {
        body.match_indices(needle)
            .nth(index)
            .map(|(start, _)| start)
            .expect("fixture occurrence")
    }

    #[test]
    fn classifies_type_paths_and_type_parameters() {
        let body = "fn preserve<Alias>(value: Option<Alias>) -> Option<Alias> {\n\
                    let _: Result<Alias, i32> = Self::make::<Alias>();\n\
                    Alias == 7;\n\
                    value\n\
                    }";
        let starts = analyze(body).type_identifier_byte_starts;
        let names = starts
            .into_iter()
            .map(|start| {
                body[start..]
                    .split(|ch: char| !(ch == '_' || ch.is_alphanumeric()))
                    .next()
                    .expect("classified offset should start an identifier")
            })
            .collect::<Vec<_>>();

        insta::assert_debug_snapshot!(names, @r#"
        [
            "Alias",
            "Option",
            "Alias",
            "Option",
            "Alias",
            "Result",
            "Alias",
            "i32",
            "Alias",
        ]
        "#);
    }

    #[test]
    fn excludes_const_generic_expressions_from_type_paths() {
        let body = "let _: Wrapper<{ Alias as usize }> = value;";
        let syntax = analyze(body);

        assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
        for name in ["Wrapper", "usize"] {
            assert!(
                syntax.is_type_identifier(occurrence(body, name, 0)),
                "missing type identifier {name}"
            );
        }
    }

    #[test]
    fn retains_type_nodes_after_recovered_modern_syntax() {
        let body = "let _ = if let Some(left) = Some(1)\n\
                    && let Some(right) = Some(2) { left == right } else { false };\n\
                    let _: Option<Alias> = None;\n\
                    Alias == 7;";
        let starts = analyze(body).type_identifier_byte_starts;
        let names = starts
            .into_iter()
            .map(|start| {
                body[start..]
                    .split(|ch: char| !(ch == '_' || ch.is_alphanumeric()))
                    .next()
                    .expect("classified offset should start an identifier")
            })
            .collect::<Vec<_>>();

        insta::assert_debug_snapshot!(names, @r#"
        [
            "Option",
            "Alias",
        ]
        "#);
    }

    #[test]
    fn rejects_recovered_parse_errors() {
        let error = super::analyze(
            "let _index = pair.0.1;\n\
             let _: Option<Alias> = None;",
        )
        .expect_err("recovered Rust parses must not disable alias protections");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        insta::assert_snapshot!("recovered_rust_parse_error", error);
    }

    #[test]
    fn distinguishes_struct_shorthands_from_block_tails() {
        let body = "let Alias = 1;\n\
                    let _ = Fields { Alias };\n\
                    fn tail() -> i32 { Alias }\n\
                    let _ = if true { Alias } else { Alias };\n\
                    let _ = (|_| -> i32 { Alias })(());";
        let syntax = analyze(body);

        assert!(syntax.is_struct_field_shorthand(occurrence(body, "Alias", 1)));
        for index in 2..=5 {
            assert!(
                !syntax.is_struct_field_shorthand(occurrence(body, "Alias", index)),
                "block-tail occurrence {index} was classified as a field"
            );
        }
    }

    #[test]
    fn distinguishes_pattern_shorthands_from_leading_or_patterns() {
        let body = "match value {\n\
                        Fields { Alias } => Alias,\n\
                        | Alias | Other => 0,\n\
                    }";
        let syntax = analyze(body);

        assert!(syntax.is_pattern_field_shorthand(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_pattern_field_shorthand(occurrence(body, "Alias", 1)));
        assert!(!syntax.is_pattern_field_shorthand(occurrence(body, "Alias", 2)));
        assert!(syntax.closure_bindings().is_empty());
    }

    #[test]
    fn records_nested_closure_parameters_and_bodies() {
        let body = "let closure = |_outer| |Alias: i32| Alias;";
        let syntax = analyze(body);
        let bindings = syntax.closure_bindings();

        assert_eq!(bindings.len(), 2);
        let alias_parameter = occurrence(body, "Alias", 0);
        let alias_read = occurrence(body, "Alias", 1);
        let inner = bindings
            .iter()
            .find(|binding| {
                binding
                    .parameter_ranges
                    .iter()
                    .any(|range| range.contains(&alias_parameter))
            })
            .expect("inner closure binding");
        assert!(inner.scope.contains(&alias_read));
    }

    #[test]
    fn records_async_closure_parameters_and_bodies() {
        let body = "let closure = async |Alias: i32| Alias;";
        let syntax = analyze(body);
        let alias_parameter = occurrence(body, "Alias", 0);
        let alias_read = occurrence(body, "Alias", 1);
        let binding = syntax
            .closure_bindings()
            .iter()
            .find(|binding| {
                binding
                    .parameter_ranges
                    .iter()
                    .any(|range| range.contains(&alias_parameter))
            })
            .expect("async closure binding");

        assert!(binding.scope.contains(&alias_read));
    }

    #[test]
    fn preserves_opaque_macro_tokens() {
        let body = "let raw = stringify!(Alias);\n\
                    let custom = take_ident!(Alias);\n\
                    matches!(value, Alias);";
        let syntax = analyze(body);

        assert!(syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 0)));
        assert!(syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 1)));
        assert!(!syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 2)));
    }

    #[test]
    fn preserves_shadowed_standard_macros_and_attribute_tokens() {
        let body = "macro_rules! assert { ($i:ident) => { true }; }\n\
                    let _ = assert!(Alias);\n\
                    let _ = matches!(value, Alias);\n\
                    #[cfg(Alias)] let guarded = 1;\n\
                    Alias == guarded;";
        let syntax = analyze(body);

        assert!(syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 1)));
        assert!(syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 2)));
        assert!(!syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 3)));
    }

    #[test]
    fn records_lifetime_and_loop_label_identifiers() {
        let body = "fn borrow<'Alias>(value: &'Alias i32) -> &'Alias i32 {\n\
                        'Alias: loop { break 'Alias value; }\n\
                    }";
        let syntax = analyze(body);

        for index in 0..5 {
            assert!(
                syntax.is_non_value_identifier(occurrence(body, "Alias", index)),
                "missing lifetime or label occurrence {index}"
            );
        }
    }

    #[test]
    fn records_enum_variant_declarations_without_hiding_later_values() {
        let body = "enum Local { Alias, Tuple(i32), Named { value: i32 } }\nAlias == 7;";
        let syntax = analyze(body);

        for name in ["Alias", "Tuple", "Named"] {
            assert!(
                syntax.is_declaration_identifier(occurrence(body, name, 0)),
                "missing enum variant declaration {name}"
            );
        }
        assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 1)));
    }

    #[test]
    fn records_const_generic_scope() {
        let body = "fn value<const Alias: usize>() -> usize { Alias }\nAlias == 7;";
        let syntax = analyze(body);
        let declaration = occurrence(body, "Alias", 0);
        let function_read = occurrence(body, "Alias", 1);
        let later_read = occurrence(body, "Alias", 2);
        let binding = syntax
            .scoped_value_bindings()
            .iter()
            .find(|binding| binding.declaration_start == declaration)
            .expect("const generic binding");

        assert!(binding.scope.contains(&function_read));
        assert!(!binding.scope.contains(&later_read));
    }

    #[test]
    fn records_const_generic_item_scopes() {
        let body = "struct Holder<const StructAlias: usize> {\n\
                        values: [u8; StructAlias],\n\
                    }\n\
                    impl<const ImplAlias: usize> Holder<ImplAlias> {\n\
                        fn value() -> usize { ImplAlias }\n\
                    }\n\
                    type Array<const TypeAlias: usize> = [u8; TypeAlias];\n\
                    ImplAlias == 7;";
        let syntax = analyze(body);
        for (name, read_index) in [("StructAlias", 1), ("ImplAlias", 2), ("TypeAlias", 1)] {
            let declaration = occurrence(body, name, 0);
            let read = occurrence(body, name, read_index);
            let binding = syntax
                .scoped_value_bindings()
                .iter()
                .find(|binding| binding.declaration_start == declaration)
                .unwrap_or_else(|| panic!("missing const-generic binding {name}"));
            assert!(binding.scope.contains(&read), "{name} read is out of scope");
        }
        let outside_impl_read = occurrence(body, "ImplAlias", 3);
        let impl_binding = syntax
            .scoped_value_bindings()
            .iter()
            .find(|binding| binding.declaration_start == occurrence(body, "ImplAlias", 0))
            .expect("impl const-generic binding");
        assert!(!impl_binding.scope.contains(&outside_impl_read));
    }

    #[test]
    fn records_function_parameters_against_the_parsed_body() {
        let body = "struct Array<const N: usize>;\n\
                    fn value(Alias: i32) -> Array<{ 1 }> {\n\
                        let _ = Alias;\n\
                        Array\n\
                    }\n\
                    Alias == 7;";
        let syntax = analyze(body);
        let parameter = occurrence(body, "Alias", 0);
        let body_read = occurrence(body, "Alias", 1);
        let later_read = occurrence(body, "Alias", 2);
        let binding = syntax
            .function_bindings()
            .iter()
            .find(|binding| {
                binding
                    .parameter_ranges
                    .iter()
                    .any(|range| range.contains(&parameter))
            })
            .expect("function parameter binding");

        assert!(binding.scope.contains(&body_read));
        assert!(!binding.scope.contains(&later_read));
    }

    #[test]
    fn member_method_names_do_not_bind_unqualified_body_reads() {
        let body = "fn Alias(&self) -> bool { Alias == Self::Alias }";
        let syntax = super::analyze_member_item(body).expect("member method fixture should parse");
        let declaration = occurrence(body, "Alias", 0);
        let unqualified_read = occurrence(body, "Alias", 1);

        assert!(syntax.is_declaration_identifier(declaration));
        assert!(
            !syntax
                .value_binding_byte_starts()
                .any(|start| start == declaration)
        );
        assert!(!syntax.is_declaration_identifier(unqualified_read));
    }

    #[test]
    fn associated_constants_do_not_bind_unqualified_impl_reads() {
        let body = "impl Holder {\n\
                        const Alias: i32 = 1;\n\
                        fn value() -> bool { Alias == Self::Alias }\n\
                    }";
        let syntax =
            super::analyze_member_item(body).expect("associated constant fixture should parse");
        let declaration = occurrence(body, "Alias", 0);
        let unqualified_read = occurrence(body, "Alias", 1);

        assert!(syntax.is_declaration_identifier(declaration));
        assert!(
            !syntax
                .value_binding_byte_starts()
                .any(|start| start == declaration)
        );
        assert!(!syntax.is_declaration_identifier(unqualified_read));
    }

    #[test]
    fn parses_inline_const_blocks() {
        let body = "let value = const { Alias };";
        let syntax = analyze(body);
        let alias = occurrence(body, "Alias", 0);

        assert!(!syntax.is_type_identifier(alias));
        assert!(!syntax.is_declaration_identifier(alias));
    }

    #[test]
    fn separates_type_declarations_from_value_constructors() {
        let body = "fn Function() {}\n\
                    const Constant: i32 = 1;\n\
                    struct Named { value: i32 }\n\
                    struct Tuple(i32);\n\
                    struct Unit;\n\
                    struct GenericNamed<T> { value: T }\n\
                    struct GenericTuple<T>(T);\n\
                    struct WhereNamed<T> where T: Copy { value: T }\n\
                    struct WhereTuple<T>(T) where T: Copy;\n\
                    type TypeAlias = i32;\n\
                    enum Enumeration { Variant }\n\
                    trait Trait {}\n\
                    mod Module {}";
        let syntax = analyze(body);

        for name in [
            "Function",
            "Constant",
            "Tuple",
            "Unit",
            "GenericTuple",
            "WhereTuple",
        ] {
            assert!(
                syntax
                    .value_binding_byte_starts()
                    .any(|start| start == occurrence(body, name, 0)),
                "missing value binding {name}"
            );
        }
        for name in [
            "Named",
            "GenericNamed",
            "WhereNamed",
            "TypeAlias",
            "Enumeration",
            "Trait",
            "Module",
        ] {
            let start = occurrence(body, name, 0);
            assert!(syntax.is_type_identifier(start), "missing type role {name}");
            assert!(
                !syntax
                    .value_binding_byte_starts()
                    .any(|candidate| candidate == start),
                "type-only declaration {name} was classified as a value"
            );
        }
    }
}
