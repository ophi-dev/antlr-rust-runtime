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
    RULE_FOREIGN_ITEM, RULE_FOREIGN_ITEM_TAIL, RULE_IDENT, RULE_IMPL_BLOCK, RULE_IMPL_ITEM_TAIL,
    RULE_INNER_ATTR, RULE_ITEM, RULE_LIFETIME, RULE_MACRO_DECL, RULE_MACRO_INVOCATION,
    RULE_MACRO_INVOCATION_SEMI, RULE_MACRO_RULES_DEFINITION, RULE_MACRO_TAIL, RULE_METHOD_DECL,
    RULE_METHOD_PARAM_LIST, RULE_MOD_DECL, RULE_MOD_DECL_SHORT, RULE_PARAM, RULE_PARAM_LIST,
    RULE_PATTERN, RULE_PATTERN_NO_TOP_ALT, RULE_PATTERN_WITHOUT_MUT, RULE_PRIM_EXPR_NO_STRUCT,
    RULE_RENAME, RULE_STATIC_DECL, RULE_STMT, RULE_STRUCT_DECL, RULE_STRUCT_TAIL, RULE_TRAIT_ALIAS,
    RULE_TRAIT_DECL, RULE_TRAIT_ITEM, RULE_TRAIT_METHOD_DECL, RULE_TRAIT_METHOD_PARAM,
    RULE_TRAIT_METHOD_PARAM_LIST, RULE_TYPE_DECL, RULE_TYPE_NO_BOUNDS, RULE_TYPE_PARAMETER,
    RULE_TYPE_PATH_MAIN, RULE_UNION_DECL, RULE_USE_DECL, RULE_USE_ITEM, RULE_USE_ITEM_LIST,
    RULE_USE_PATH, RULE_USE_SUFFIX, RULE_VARIADIC_PARAM_LIST, RustParser,
};

const WRAPPER_PREFIX: &str = "{\n";

#[derive(Debug, Default)]
pub(crate) struct RustSyntax {
    type_identifier_byte_starts: BTreeSet<usize>,
    declaration_identifier_byte_starts: BTreeSet<usize>,
    non_value_identifier_byte_starts: BTreeSet<usize>,
    opaque_macro_identifier_byte_starts: BTreeSet<usize>,
    opaque_macro_byte_ranges: Vec<Range<usize>>,
    opaque_expression_macro_byte_ranges: Vec<Range<usize>>,
    opaque_parent_block_macro_byte_ranges: Vec<Range<usize>>,
    conditional_macro_shadows: Vec<ConditionalMacroShadow>,
    struct_field_shorthand_byte_starts: BTreeSet<usize>,
    pattern_field_shorthand_byte_starts: BTreeSet<usize>,
    conditional_pattern_binding_ranges: Vec<ConditionalPatternBinding>,
    inline_module_ranges: Vec<Range<usize>>,
    value_binding_byte_starts: BTreeSet<usize>,
    conditional_value_bindings: Vec<ConditionalValueBinding>,
    scoped_value_bindings: Vec<ScopedValueBinding>,
    function_bindings: Vec<FunctionBinding>,
    closure_bindings: Vec<ClosureBinding>,
}

#[derive(Debug)]
pub(crate) struct ConditionalBindingFallback {
    pub(crate) insertion: usize,
    pub(crate) active_predicate: String,
}

#[derive(Debug)]
struct ConditionalValueBinding {
    declaration_start: usize,
    fallback: ConditionalBindingFallback,
}

#[derive(Debug)]
pub(crate) struct ScopedValueBinding {
    pub(crate) declaration_start: usize,
    pub(crate) scope: Range<usize>,
    pub(crate) cfg_fallback: Option<ConditionalBindingFallback>,
}

#[derive(Debug)]
pub(crate) struct ClosureBinding {
    pub(crate) parameter_ranges: Vec<Range<usize>>,
    pub(crate) cfg_parameter_predicates: Vec<(Range<usize>, String)>,
    pub(crate) scope: Range<usize>,
}

#[derive(Debug)]
pub(crate) struct FunctionBinding {
    pub(crate) parameter_ranges: Vec<Range<usize>>,
    pub(crate) cfg_parameter_predicates: Vec<(Range<usize>, String)>,
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
    activation: MacroBindingActivation,
}

#[derive(Debug)]
enum MacroBindingActivation {
    Always,
    Conditional { insertion: usize, predicate: String },
}

#[derive(Debug)]
struct ConditionalMacroShadow {
    range: Range<usize>,
    insertion: usize,
    active_predicate: String,
}

#[derive(Debug)]
struct ConditionalPatternBinding {
    range: Range<usize>,
    active_predicate: String,
}

#[derive(Debug)]
enum LocalMacroShadow {
    None,
    Always,
    Conditional {
        insertion: usize,
        active_predicate: String,
    },
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

    pub(crate) fn is_opaque_macro_byte(&self, byte_start: usize) -> bool {
        self.opaque_macro_byte_ranges
            .iter()
            .any(|range| range.contains(&byte_start))
    }

    pub(crate) fn opaque_macro_accepts_expression_fallback(&self, byte_start: usize) -> bool {
        self.opaque_expression_macro_byte_ranges
            .iter()
            .any(|range| range.contains(&byte_start))
    }

    pub(crate) fn opaque_macro_requires_parent_block_fallback(&self, byte_start: usize) -> bool {
        self.opaque_parent_block_macro_byte_ranges
            .iter()
            .any(|range| range.contains(&byte_start))
    }

    pub(crate) fn conditional_macro_fallback(&self, byte_start: usize) -> Option<(usize, &str)> {
        self.conditional_macro_shadows
            .iter()
            .find(|shadow| shadow.range.contains(&byte_start))
            .map(|shadow| (shadow.insertion, shadow.active_predicate.as_str()))
    }

    pub(crate) fn is_struct_field_shorthand(&self, byte_start: usize) -> bool {
        self.struct_field_shorthand_byte_starts
            .contains(&byte_start)
    }

    pub(crate) fn is_pattern_field_shorthand(&self, byte_start: usize) -> bool {
        self.pattern_field_shorthand_byte_starts
            .contains(&byte_start)
    }

    pub(crate) fn pattern_binding_cfg_predicate(&self, byte_start: usize) -> Option<String> {
        let predicates = self
            .conditional_pattern_binding_ranges
            .iter()
            .filter(|binding| binding.range.contains(&byte_start))
            .map(|binding| binding.active_predicate.clone())
            .collect::<Vec<_>>();
        super::cfg_all_predicate(&predicates)
    }

    pub(crate) fn inline_module_depth(&self, byte_start: usize) -> usize {
        self.inline_module_ranges
            .iter()
            .filter(|range| range.contains(&byte_start))
            .count()
    }

    pub(crate) fn value_binding_byte_starts(&self) -> impl Iterator<Item = usize> + '_ {
        self.value_binding_byte_starts.iter().copied()
    }

    pub(crate) fn value_binding_cfg_fallback(
        &self,
        declaration_start: usize,
    ) -> Option<&ConditionalBindingFallback> {
        self.conditional_value_bindings
            .iter()
            .find(|binding| binding.declaration_start == declaration_start)
            .map(|binding| &binding.fallback)
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
    let macro_bindings =
        collect_scoped_macro_bindings(parsed.tree(), parsed.tokens(), body, body.len());

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
                collect_const_generic_binding(rule, parsed.tokens(), body, body.len(), &mut syntax);
            }
            RULE_TYPE_DECL | RULE_ENUM_DECL | RULE_UNION_DECL | RULE_TRAIT_DECL
            | RULE_TRAIT_ALIAS | RULE_MOD_DECL_SHORT | RULE_EXTERN_CRATE | RULE_MACRO_DECL => {
                collect_direct_identifier_start(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.type_identifier_byte_starts,
                );
            }
            RULE_MOD_DECL => {
                collect_direct_identifier_start(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.type_identifier_byte_starts,
                );
                if let Some(range) = body_byte_range(rule, parsed.tokens(), body.len()) {
                    syntax.inline_module_ranges.push(range);
                }
            }
            RULE_STRUCT_DECL => {
                collect_direct_identifier_start(
                    rule,
                    parsed.tokens(),
                    body.len(),
                    &mut syntax.type_identifier_byte_starts,
                );
                if parsed_struct_has_value_constructor(rule) {
                    collect_value_binding(rule, parsed.tokens(), body, body.len(), &mut syntax);
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
                collect_function_binding(rule, parsed.tokens(), body, body.len(), &mut syntax);
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
                    collect_value_binding(rule, parsed.tokens(), body, body.len(), &mut syntax);
                }
            }
            RULE_STATIC_DECL | RULE_CONST_DECL => {
                if item_introduces_unqualified_value(rule) {
                    collect_value_binding(rule, parsed.tokens(), body, body.len(), &mut syntax);
                } else {
                    collect_direct_identifier_start(
                        rule,
                        parsed.tokens(),
                        body.len(),
                        &mut syntax.declaration_identifier_byte_starts,
                    );
                }
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
                collect_closure_binding(rule, parsed.tokens(), body, body.len(), &mut syntax);
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
    collect_pattern_field_roles(
        parsed.tree(),
        parsed.tokens(),
        body,
        body.len(),
        &mut syntax,
    );
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

fn collect_value_binding(
    declaration: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body: &str,
    body_len: usize,
    syntax: &mut RustSyntax,
) {
    let Some(declaration_start) = direct_identifier_byte_start(declaration, tokens, body_len)
    else {
        return;
    };
    syntax.value_binding_byte_starts.insert(declaration_start);
    let Some(item) = enclosing_item(declaration) else {
        return;
    };
    let Some(item_range) = body_byte_range(item, tokens, body_len) else {
        return;
    };
    let Some(declaration_range) = body_byte_range(declaration, tokens, body_len) else {
        return;
    };
    let Some(active_predicate) = super::cfg_all_predicate(&super::member_cfg_predicates(
        &body[item_range.start..declaration_range.start],
    )) else {
        return;
    };
    syntax
        .conditional_value_bindings
        .push(ConditionalValueBinding {
            declaration_start,
            fallback: ConditionalBindingFallback {
                insertion: item_range.start,
                active_predicate,
            },
        });
}

fn collect_const_generic_binding(
    parameter: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body: &str,
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
    let cfg_fallback = body_byte_range(parameter, tokens, body_len)
        .and_then(|range| {
            super::cfg_all_predicate(&super::member_cfg_predicates(
                &body[range.start..declaration_start],
            ))
        })
        .and_then(|active_predicate| {
            let item = enclosing_item(parameter)?;
            let insertion = body_byte_range(item, tokens, body_len)?.start;
            Some(ConditionalBindingFallback {
                insertion,
                active_predicate,
            })
        });
    syntax.scoped_value_bindings.push(ScopedValueBinding {
        declaration_start,
        scope,
        cfg_fallback,
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
    let Some((macro_name, qualification)) = macro_invocation_name(parent, tokens) else {
        return;
    };
    let shadow = if qualification == MacroQualification::Unqualified {
        local_macro_shadow(&macro_name, parent, tokens, body_len, macro_bindings)
    } else {
        LocalMacroShadow::None
    };
    if qualification == MacroQualification::Other
        || !macro_allows_value_alias_lowering(&macro_name)
        || !matches!(&shadow, LocalMacroShadow::None)
    {
        if let Some(range) = body_byte_range(macro_tail, tokens, body_len) {
            if let LocalMacroShadow::Conditional {
                insertion,
                active_predicate,
            } = shadow
            {
                syntax
                    .conditional_macro_shadows
                    .push(ConditionalMacroShadow {
                        range: range.clone(),
                        insertion,
                        active_predicate,
                    });
            }
            match macro_fallback_kind(parent) {
                OpaqueMacroFallbackKind::Expression => syntax
                    .opaque_expression_macro_byte_ranges
                    .push(range.clone()),
                OpaqueMacroFallbackKind::ParentBlock => syntax
                    .opaque_parent_block_macro_byte_ranges
                    .push(range.clone()),
                OpaqueMacroFallbackKind::EnclosingBlock => {}
            }
            syntax.opaque_macro_byte_ranges.push(range);
        }
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
    let Some((macro_name, qualification)) = macro_invocation_name(invocation, tokens) else {
        return;
    };
    let shadow = if qualification == MacroQualification::Unqualified {
        local_macro_shadow(&macro_name, invocation, tokens, body_len, macro_bindings)
    } else {
        LocalMacroShadow::None
    };
    if qualification == MacroQualification::Other
        || !macro_allows_value_alias_lowering(&macro_name)
        || !matches!(&shadow, LocalMacroShadow::None)
    {
        if let Some(range) = body_byte_range(invocation, tokens, body_len) {
            if let LocalMacroShadow::Conditional {
                insertion,
                active_predicate,
            } = shadow
            {
                syntax
                    .conditional_macro_shadows
                    .push(ConditionalMacroShadow {
                        range: range.clone(),
                        insertion,
                        active_predicate,
                    });
            }
            match macro_fallback_kind(invocation) {
                OpaqueMacroFallbackKind::Expression => syntax
                    .opaque_expression_macro_byte_ranges
                    .push(range.clone()),
                OpaqueMacroFallbackKind::ParentBlock => syntax
                    .opaque_parent_block_macro_byte_ranges
                    .push(range.clone()),
                OpaqueMacroFallbackKind::EnclosingBlock => {}
            }
            syntax.opaque_macro_byte_ranges.push(range);
        }
        collect_identifier_starts(
            invocation.node(),
            tokens,
            body_len,
            &mut syntax.opaque_macro_identifier_byte_starts,
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpaqueMacroFallbackKind {
    Expression,
    EnclosingBlock,
    ParentBlock,
}

fn macro_fallback_kind(invocation: antlr4_runtime::RuleNodeView<'_>) -> OpaqueMacroFallbackKind {
    let mut semicolon_invocation = false;
    let mut node = Some(invocation.node());
    while let Some(current) = node {
        if let Some(rule) = current.as_rule() {
            match rule.rule_index() {
                RULE_PATTERN_WITHOUT_MUT | RULE_TYPE_NO_BOUNDS => {
                    return OpaqueMacroFallbackKind::EnclosingBlock;
                }
                RULE_PRIM_EXPR_NO_STRUCT => return OpaqueMacroFallbackKind::Expression,
                RULE_MACRO_INVOCATION_SEMI => semicolon_invocation = true,
                RULE_IMPL_ITEM_TAIL | RULE_TRAIT_ITEM | RULE_FOREIGN_ITEM
                    if semicolon_invocation =>
                {
                    return OpaqueMacroFallbackKind::ParentBlock;
                }
                RULE_ITEM if semicolon_invocation => {
                    let local_statement = current
                        .parent()
                        .and_then(Node::as_rule)
                        .is_some_and(|parent| parent.rule_index() == RULE_STMT);
                    return if local_statement {
                        OpaqueMacroFallbackKind::Expression
                    } else {
                        OpaqueMacroFallbackKind::EnclosingBlock
                    };
                }
                RULE_STMT if semicolon_invocation => {
                    return OpaqueMacroFallbackKind::Expression;
                }
                _ => {}
            }
        }
        node = current.parent();
    }
    OpaqueMacroFallbackKind::Expression
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MacroQualification {
    Unqualified,
    Standard,
    Other,
}

fn macro_invocation_name(
    invocation: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
) -> Option<(String, MacroQualification)> {
    let mut path = Vec::new();
    let mut qualified = false;
    for terminal in invocation
        .node()
        .descendants()
        .filter_map(Node::as_terminal)
    {
        if terminal.text() == "!" {
            break;
        }
        if matches!(terminal.text(), ":" | "::") {
            qualified = true;
        }
        if matches!(
            tokens.token_type(terminal.token_id()),
            Some(IDENT | RAW_IDENTIFIER)
        ) {
            path.push(
                terminal
                    .text()
                    .strip_prefix("r#")
                    .unwrap_or_else(|| terminal.text())
                    .to_owned(),
            );
        }
    }
    let name = path.pop()?;
    let qualification = if !qualified {
        MacroQualification::Unqualified
    } else if path.as_slice() == ["std"] || path.as_slice() == ["core"] {
        MacroQualification::Standard
    } else {
        MacroQualification::Other
    };
    Some((name, qualification))
}

fn local_macro_shadow(
    name: &str,
    invocation: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body_len: usize,
    bindings: &[ScopedMacroBinding],
) -> LocalMacroShadow {
    let Some(start) = invocation
        .start_id()
        .and_then(|token| body_byte_start(tokens, token, body_len))
    else {
        return LocalMacroShadow::None;
    };
    let mut insertion = usize::MAX;
    let mut predicates = BTreeSet::new();
    for binding in bindings
        .iter()
        .filter(|binding| binding.name == name && binding.scope.contains(&start))
    {
        match &binding.activation {
            MacroBindingActivation::Always => return LocalMacroShadow::Always,
            MacroBindingActivation::Conditional {
                insertion: binding_insertion,
                predicate,
            } => {
                insertion = insertion.min(*binding_insertion);
                predicates.insert(predicate.clone());
            }
        }
    }
    match predicates.len() {
        0 => LocalMacroShadow::None,
        1 => LocalMacroShadow::Conditional {
            insertion,
            active_predicate: predicates
                .into_iter()
                .next()
                .expect("checked one predicate"),
        },
        _ => LocalMacroShadow::Conditional {
            insertion,
            active_predicate: format!(
                "any({})",
                predicates.into_iter().collect::<Vec<_>>().join(", ")
            ),
        },
    }
}

fn collect_scoped_macro_bindings(
    root: Node<'_>,
    tokens: &TokenStore,
    body: &str,
    body_len: usize,
) -> Vec<ScopedMacroBinding> {
    let mut bindings = Vec::new();
    for rule in root.descendants().filter_map(Node::as_rule) {
        match rule.rule_index() {
            RULE_MACRO_RULES_DEFINITION => {
                let Some(name) = macro_rules_name(rule, tokens) else {
                    continue;
                };
                let Some(declaration) = body_byte_range(rule, tokens, body_len) else {
                    continue;
                };
                let mut scope = enclosing_block(rule)
                    .and_then(|block| body_byte_range(block, tokens, body_len))
                    .unwrap_or(0..body_len);
                scope.start = declaration.end.min(scope.end);
                bindings.push(ScopedMacroBinding {
                    name,
                    scope,
                    activation: macro_binding_activation(rule, tokens, body, body_len),
                });
            }
            RULE_USE_DECL => {
                let scope = enclosing_block(rule)
                    .and_then(|block| body_byte_range(block, tokens, body_len))
                    .unwrap_or(0..body_len);
                let activation = macro_binding_activation(rule, tokens, body, body_len);
                bindings.extend(
                    use_decl_binding_names(rule, tokens)
                        .into_iter()
                        .filter(|name| macro_allows_value_alias_lowering(name))
                        .map(|name| ScopedMacroBinding {
                            name,
                            scope: scope.clone(),
                            activation: match &activation {
                                MacroBindingActivation::Always => MacroBindingActivation::Always,
                                MacroBindingActivation::Conditional {
                                    insertion,
                                    predicate,
                                } => MacroBindingActivation::Conditional {
                                    insertion: *insertion,
                                    predicate: predicate.clone(),
                                },
                            },
                        }),
                );
            }
            _ => {}
        }
    }
    bindings
}

fn macro_binding_activation(
    declaration: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body: &str,
    body_len: usize,
) -> MacroBindingActivation {
    let Some(item) = enclosing_item(declaration) else {
        return MacroBindingActivation::Always;
    };
    let Some(range) = body_byte_range(item, tokens, body_len) else {
        return MacroBindingActivation::Always;
    };
    let Some(predicate) =
        super::cfg_all_predicate(&super::member_cfg_predicates(&body[range.clone()]))
    else {
        return MacroBindingActivation::Always;
    };
    MacroBindingActivation::Conditional {
        insertion: range.start,
        predicate,
    }
}

fn enclosing_item(
    rule: antlr4_runtime::RuleNodeView<'_>,
) -> Option<antlr4_runtime::RuleNodeView<'_>> {
    let mut node = Some(rule.node());
    while let Some(current) = node {
        if let Some(current_rule) = current.as_rule()
            && current_rule.rule_index() == RULE_ITEM
        {
            return Some(current_rule);
        }
        node = current.parent();
    }
    None
}

fn use_decl_binding_names(
    declaration: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(path) = declaration.child_rule(RULE_USE_PATH) {
        collect_use_path_binding_names(path, tokens, &mut names);
    }
    names
}

fn collect_use_path_binding_names(
    path: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    names: &mut Vec<String>,
) {
    if let Some(list) = path.child_rule(RULE_USE_ITEM_LIST) {
        collect_use_item_list_binding_names(list, tokens, names);
        return;
    }
    if let Some(suffix) = path.child_rule(RULE_USE_SUFFIX) {
        if let Some(rename) = suffix.child_rule(RULE_RENAME) {
            if let Some(name) = identifier_name(rename, tokens) {
                names.push(name);
            }
        } else if let Some(list) = suffix.child_rule(RULE_USE_ITEM_LIST) {
            collect_use_item_list_binding_names(list, tokens, names);
        }
        return;
    }
    if let Some(name) = path
        .children()
        .filter_map(Node::as_rule)
        .filter_map(|child| identifier_name(child, tokens))
        .next_back()
    {
        names.push(name);
    }
}

fn collect_use_item_list_binding_names(
    list: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    names: &mut Vec<String>,
) {
    for item in list
        .children()
        .filter_map(Node::as_rule)
        .filter(|child| child.rule_index() == RULE_USE_ITEM)
    {
        if let Some(rename) = item.child_rule(RULE_RENAME) {
            if let Some(name) = identifier_name(rename, tokens) {
                names.push(name);
            }
        } else if let Some(path) = item.child_rule(RULE_USE_PATH) {
            collect_use_path_binding_names(path, tokens, names);
        } else if let Some(name) = identifier_name(item, tokens) {
            names.push(name);
        }
    }
}

fn identifier_name(rule: antlr4_runtime::RuleNodeView<'_>, tokens: &TokenStore) -> Option<String> {
    rule.node()
        .descendants()
        .filter_map(Node::as_terminal)
        .find(|terminal| {
            matches!(
                tokens.token_type(terminal.token_id()),
                Some(IDENT | RAW_IDENTIFIER)
            )
        })
        .map(|terminal| {
            terminal
                .text()
                .strip_prefix("r#")
                .unwrap_or_else(|| terminal.text())
                .to_owned()
        })
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
                RULE_BLOCK | RULE_BLOCK_WITH_INNER_ATTRS | RULE_MOD_DECL
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

fn collect_pattern_field_roles(
    root: Node<'_>,
    tokens: &TokenStore,
    body: &str,
    body_len: usize,
    syntax: &mut RustSyntax,
) {
    for node in root.descendants() {
        let Some(rule) = node.as_rule() else {
            continue;
        };
        if rule.rule_index() != generated::parser::RULE_PAT_FIELD {
            continue;
        }
        if rule.child_rule(RULE_PATTERN).is_none() {
            collect_direct_identifier_start(
                rule,
                tokens,
                body_len,
                &mut syntax.pattern_field_shorthand_byte_starts,
            );
        }
        let Some(range) = body_byte_range(rule, tokens, body_len) else {
            continue;
        };
        let Some(active_predicate) =
            super::cfg_all_predicate(&super::member_cfg_predicates(&body[range.clone()]))
        else {
            continue;
        };
        syntax
            .conditional_pattern_binding_ranges
            .push(ConditionalPatternBinding {
                range,
                active_predicate,
            });
    }
}

fn collect_closure_binding(
    expression: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body: &str,
    body_len: usize,
    syntax: &mut RustSyntax,
) {
    let Some(parameters) = expression.child_rule(RULE_CLOSURE_PARAMS) else {
        return;
    };
    let Some(tail) = expression.child_rule(RULE_CLOSURE_TAIL) else {
        return;
    };
    let Some(body_rule) = tail
        .child_rule(RULE_BLOCK)
        .or_else(|| tail.child_rule(RULE_EXPR))
    else {
        return;
    };
    let Some(scope) = body_byte_range(body_rule, tokens, body_len) else {
        return;
    };
    let parameter_ranges = parameters
        .node()
        .descendants()
        .filter_map(Node::as_rule)
        .filter(|rule| rule.rule_index() == RULE_CLOSURE_PARAM)
        .filter_map(|parameter| {
            parameter.child_rule(RULE_PATTERN_NO_TOP_ALT)?;
            body_byte_range(parameter, tokens, body_len)
        })
        .collect::<Vec<_>>();
    if !parameter_ranges.is_empty() {
        let cfg_parameter_predicates = parameter_ranges
            .iter()
            .filter_map(|range| {
                super::cfg_all_predicate(&super::member_cfg_predicates(&body[range.clone()]))
                    .map(|predicate| (range.clone(), predicate))
            })
            .collect();
        syntax.closure_bindings.push(ClosureBinding {
            parameter_ranges,
            cfg_parameter_predicates,
            scope,
        });
    }
}

fn collect_function_binding(
    function: antlr4_runtime::RuleNodeView<'_>,
    tokens: &TokenStore,
    body: &str,
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
    let Some(body_rule) = function.child_rule(RULE_BLOCK_WITH_INNER_ATTRS) else {
        return;
    };
    let Some(scope) = body_byte_range(body_rule, tokens, body_len) else {
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
        let cfg_parameter_predicates = parameter_ranges
            .iter()
            .filter_map(|range| {
                super::cfg_all_predicate(&super::member_cfg_predicates(&body[range.clone()]))
                    .map(|predicate| (range.clone(), predicate))
            })
            .collect();
        syntax.function_bindings.push(FunctionBinding {
            parameter_ranges,
            cfg_parameter_predicates,
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
    fn rejects_genuinely_invalid_recovered_parse_errors() {
        let error = super::analyze(
            "let _broken = (;\n\
             let _: Option<Alias> = None;",
        )
        .expect_err("recovered Rust parses must not disable alias protections");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        insta::assert_snapshot!("recovered_rust_parse_error", error);
    }

    #[test]
    fn parses_chained_tuple_fields_and_pub_self_visibility() {
        let body = "mod nested { pub(self) fn helper() {} }\n\
                    let pair = ((0, 1), 2);\n\
                    let _index = pair.0.1;\n\
                    Alias == 1;";
        let syntax = analyze(body);

        assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 0)));
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
    fn records_cfg_gated_pattern_field_bindings() {
        let body = "let Fields {\n\
                        #[cfg(any())] Alias,\n\
                        #[cfg(feature = \"other\")] field: Other,\n\
                    } = value;";
        let syntax = analyze(body);

        assert_eq!(
            syntax.pattern_binding_cfg_predicate(occurrence(body, "Alias", 0)),
            Some("any()".to_owned())
        );
        assert_eq!(
            syntax.pattern_binding_cfg_predicate(occurrence(body, "Other", 0)),
            Some("feature = \"other\"".to_owned())
        );
    }

    #[test]
    fn records_cfg_gated_match_pattern_bindings() {
        let body = "match value {\n\
                        Fields {\n\
                            #[cfg(any())] Alias,\n\
                        } if Alias == 1 => true,\n\
                        _ => false,\n\
                    }";
        let syntax = analyze(body);

        assert_eq!(
            syntax.pattern_binding_cfg_predicate(occurrence(body, "Alias", 0)),
            Some("any()".to_owned())
        );
    }

    #[test]
    fn records_nested_closure_parameters_and_bodies() {
        let body = "let closure = |_outer| |#[cfg(any())] Alias: i32| Alias;";
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
        assert_eq!(
            inner
                .cfg_parameter_predicates
                .iter()
                .find(|(range, _)| range.contains(&alias_parameter))
                .map(|(_, predicate)| predicate.as_str()),
            Some("any()")
        );
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
    fn preserves_opaque_compatibility_receivers() {
        let body = "let names = (stringify!(recog), take_ident!(_localctx));\n\
                    #[cfg_attr(any(), cfg(recog, _localctx))]\n\
                    let marker = true;\n\
                    recog.input.la(1) == 1 && _localctx.is_some() && marker";
        let syntax = analyze(body);

        for (name, opaque_occurrences) in [("recog", 2), ("_localctx", 2)] {
            for occurrence_index in 0..opaque_occurrences {
                assert!(syntax.is_opaque_macro_identifier(occurrence(
                    body,
                    name,
                    occurrence_index
                )));
            }
            assert!(!syntax.is_opaque_macro_identifier(occurrence(body, name, opaque_occurrences)));
        }
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
    fn preserves_imported_macros_bound_to_standard_names() {
        let body = "use crate::macros::{custom as assert, matches};\n\
                    let renamed = assert!(RenamedAlias);\n\
                    let direct = matches!(value, DirectAlias);\n\
                    let standard = std::matches!(value, StandardAlias);";
        let syntax = analyze(body);

        assert!(syntax.is_opaque_macro_identifier(occurrence(body, "RenamedAlias", 0)));
        assert!(syntax.is_opaque_macro_identifier(occurrence(body, "DirectAlias", 0)));
        assert!(!syntax.is_opaque_macro_identifier(occurrence(body, "StandardAlias", 0)));
    }

    #[test]
    fn records_cfg_gated_imported_macro_shadowing() {
        let body = "#[cfg(any())]\n\
                    use missing::format;\n\
                    let rendered = format!(\"{Alias}\");";
        let syntax = analyze(body);
        let capture = occurrence(body, "Alias", 0);

        assert!(syntax.is_opaque_macro_byte(capture));
        assert_eq!(
            syntax.conditional_macro_fallback(capture),
            Some((0, "any()"))
        );
    }

    #[test]
    fn distinguishes_expression_macros_from_type_and_pattern_macros() {
        let body = "macro_rules! type_value { ($i:ident) => { i32 }; }\n\
                    macro_rules! pattern_value { ($i:ident) => { 1 }; }\n\
                    macro_rules! expr_value { ($i:ident) => { $i }; }\n\
                    let _: type_value!(TypeAlias) = 0;\n\
                    let pattern = if let pattern_value!(PatternAlias) = 1 { true } else { false };\n\
                    let expression = expr_value!(ExprAlias);";
        let syntax = analyze(body);

        for alias in ["TypeAlias", "PatternAlias", "ExprAlias"] {
            assert!(syntax.is_opaque_macro_identifier(occurrence(body, alias, 0)));
        }
        assert!(!syntax.opaque_macro_accepts_expression_fallback(occurrence(body, "TypeAlias", 0)));
        assert!(!syntax.opaque_macro_accepts_expression_fallback(occurrence(
            body,
            "PatternAlias",
            0
        )));
        assert!(syntax.opaque_macro_accepts_expression_fallback(occurrence(body, "ExprAlias", 0)));
    }

    #[test]
    fn distinguishes_item_macro_alias_scopes() {
        let body = "macro_rules! item_value { ($i:ident) => { const VALUE: i32 = $i; }; }\n\
                    mod nested { item_value!(ModuleItemAlias); }\n\
                    struct Local;\n\
                    impl Local { item_value!(ImplItemAlias); }\n\
                    fn local() { item_value!(LocalItemAlias); }";
        let syntax = analyze(body);
        let module_alias = occurrence(body, "ModuleItemAlias", 0);
        let impl_alias = occurrence(body, "ImplItemAlias", 0);
        let local_alias = occurrence(body, "LocalItemAlias", 0);

        assert!(!syntax.opaque_macro_accepts_expression_fallback(module_alias));
        assert!(!syntax.opaque_macro_requires_parent_block_fallback(module_alias));
        assert!(!syntax.opaque_macro_accepts_expression_fallback(impl_alias));
        assert!(syntax.opaque_macro_requires_parent_block_fallback(impl_alias));
        assert!(syntax.opaque_macro_accepts_expression_fallback(local_alias));
        assert!(!syntax.opaque_macro_requires_parent_block_fallback(local_alias));
    }

    #[test]
    fn parses_spaced_raw_reference_expressions() {
        let body = "let value = 1;\n\
                    let compact = &raw const value;\n\
                    let spaced = & raw const value;\n\
                    Alias == 1 && compact == spaced";
        let syntax = analyze(body);

        assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 0)));
    }

    #[test]
    fn parses_underscore_assignment_expressions() {
        let body = "let mut observed = 0;\n\
                    _ = { observed = 1; compute() };\n\
                    Alias == 1 && observed == 1";
        let syntax = analyze(body);

        assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 0)));
    }

    #[test]
    fn parses_reviewed_modern_rust_forms() {
        for body in [
            "let source = 1;\n\
             let value = unsafe { *&raw const source };\n\
             Alias == value;",
            "let value = 1e_10;\nAlias == value;",
            "let ranged = match 2 { ..=1 => false, 2.. => true };\nAlias == ranged;",
            "fn callback(_: for<#[cfg(all())] 'a> fn(&'a i32)) {}\nAlias == 1;",
            "struct Local<const N: usize = 3>;\nAlias == 1;",
            "fn safe(safe: i32) -> i32 { safe }\nAlias == safe(1);",
            r#"let text = "\u{00_E6}"; Alias == text;"#,
            "let matched = match 1 {\n\
                 #![allow(unused)]\n\
                 #![allow(dead_code)]\n\
                 1 => true,\n\
                 _ => false,\n\
             };\n\
             Alias == matched;",
            "let 𞤀 = 1;\nAlias == 𞤀;",
            "mod nested { pub(in self) fn helper() {} }\nAlias == 1;",
            "fn helper() {}\nhelper::<>();\nAlias == 1;",
            "fn helper() where {}\nAlias == 1;",
        ] {
            let syntax = super::analyze(body)
                .unwrap_or_else(|error| panic!("Rust syntax fixture {body:?} failed: {error}"));
            assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
            assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 0)));
        }
    }

    #[test]
    fn nested_module_macros_do_not_shadow_outer_invocations() {
        let body = "mod macros {\n\
                        macro_rules! matches { ($($tokens:tt)*) => { true }; }\n\
                    }\n\
                    let _ = matches!(value, Alias);";
        let syntax = analyze(body);

        assert!(!syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 0)));
    }

    #[test]
    fn tracks_inline_module_nesting_depth() {
        let body = "let root = self::Alias;\n\
                    mod outer {\n\
                        fn outer() { let _ = super::Alias; let _ = self::Alias; }\n\
                        mod inner { fn inner() { let _ = super::super::Alias; } }\n\
                    }";
        let syntax = analyze(body);

        assert_eq!(syntax.inline_module_depth(occurrence(body, "Alias", 0)), 0);
        assert_eq!(syntax.inline_module_depth(occurrence(body, "Alias", 1)), 1);
        assert_eq!(syntax.inline_module_depth(occurrence(body, "Alias", 2)), 1);
        assert_eq!(syntax.inline_module_depth(occurrence(body, "Alias", 3)), 2);
    }

    #[test]
    fn preserves_custom_qualified_macros_and_lowers_standard_paths() {
        let body = "let custom = my_macros::assert!(Alias);\n\
                    let std_macro = std::assert!(Alias == 1);\n\
                    let core_macro = core::matches!(value, Alias);\n\
                    let unqualified = assert!(Alias == 1);";
        let syntax = analyze(body);

        assert!(syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 1)));
        assert!(!syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 2)));
        assert!(!syntax.is_opaque_macro_identifier(occurrence(body, "Alias", 3)));
        assert!(syntax.is_opaque_macro_byte(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_opaque_macro_byte(occurrence(body, "Alias", 1)));
        assert!(!syntax.is_opaque_macro_byte(occurrence(body, "Alias", 2)));
        assert!(!syntax.is_opaque_macro_byte(occurrence(body, "Alias", 3)));
    }

    #[test]
    fn parses_edition_2024_unsafe_extern_blocks() {
        let body = "unsafe extern \"C\" { fn foreign(); }\nAlias == 1";
        let syntax = analyze(body);

        assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 0)));
    }

    #[test]
    fn parses_associated_type_bounds_and_nonleading_let_chains() {
        let body = "fn consume(_: impl Iterator<Item: Copy>) {}\n\
                    if ready && let Some(value) = input { let _ = value; }\n\
                    while ready && let Some(value) = input { let _ = value; }\n\
                    Alias == 1";
        let syntax = analyze(body);

        assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 0)));
    }

    #[test]
    fn parses_raw_lifetimes_and_safe_foreign_items() {
        let body = "fn borrow<'r#type>(value: &'r#type i32) -> &'r#type i32 { value }\n\
                    unsafe extern \"C\" { safe fn foreign(); safe static VALUE: i32; }\n\
                    Alias == 1";
        let syntax = analyze(body);

        assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 0)));
    }

    #[test]
    fn parses_unsafe_foreign_statics_and_attributed_variadics() {
        let body = "unsafe extern \"C\" {\n\
                        unsafe static VALUE: i32;\n\
                        fn call(#[allow(unused)] ...);\n\
                    }\n\
                    type Variadic = unsafe extern \"C\" fn(#[allow(unused)] ...);\n\
                    Alias == 1";
        let syntax = analyze(body);

        assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 0)));
    }

    #[test]
    fn parses_attributed_shorthand_fields_and_numeric_subpatterns() {
        let body = "struct Local { field: i32 }\n\
                    let field = 1;\n\
                    let _ = Local { #[allow(unused)] field };\n\
                    struct Tuple(Option<i32>, i32);\n\
                    let _ = match Tuple(Some(2), 3) {\n\
                        Tuple { 0: Some(value), 1: _ } => value,\n\
                        _ => 0,\n\
                    };\n\
                    Alias == 1";
        let syntax = analyze(body);

        assert!(syntax.is_struct_field_shorthand(occurrence(body, "field", 2)));
        assert!(!syntax.is_type_identifier(occurrence(body, "Alias", 0)));
        assert!(!syntax.is_declaration_identifier(occurrence(body, "Alias", 0)));
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
        let body = "fn value<#[cfg(any())] const Alias: usize>() -> usize { Alias }\nAlias == 7;";
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
        let fallback = binding
            .cfg_fallback
            .as_ref()
            .expect("cfg-gated const generic fallback");
        assert_eq!(fallback.insertion, 0);
        assert_eq!(fallback.active_predicate, "any()");
    }

    #[test]
    fn records_cfg_gated_block_value_items() {
        let body = "#[cfg(any())]\nconst Alias: i32 = 1;\nAlias == 7;";
        let syntax = analyze(body);
        let declaration = occurrence(body, "Alias", 0);
        let fallback = syntax
            .value_binding_cfg_fallback(declaration)
            .expect("cfg-gated value item fallback");

        assert_eq!(fallback.insertion, 0);
        assert_eq!(fallback.active_predicate, "any()");
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
                    fn value(#[cfg(any())] Alias: i32) -> Array<{ 1 }> {\n\
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
        assert_eq!(
            binding
                .cfg_parameter_predicates
                .iter()
                .find(|(range, _)| range.contains(&parameter))
                .map(|(_, predicate)| predicate.as_str()),
            Some("any()")
        );
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
    fn parses_c_string_literals() {
        let body = r##"let normal = c"value";
                       let escaped = c"\xE6";
                       let unicode = c"\u{00E6}";
                       let underscored = c"\u{0_0E6}";
                       let raw = cr#"raw value"#;
                       normal.to_bytes() == raw.to_bytes()
                           && escaped.to_bytes() == unicode.to_bytes()
                           && unicode.to_bytes() == underscored.to_bytes()
                           && Alias == 1;"##;
        let syntax = analyze(body);
        let alias = occurrence(body, "Alias", 0);

        assert!(!syntax.is_type_identifier(alias));
        assert!(!syntax.is_declaration_identifier(alias));
    }

    #[test]
    fn rejects_nul_and_carriage_return_in_c_string_literals() {
        for body in [
            r#"let _ = c"\0";"#,
            r#"let _ = c"\x00";"#,
            r#"let _ = c"\u{0}";"#,
            r#"let _ = c"\u{00}";"#,
            r#"let _ = c"\u{0000000}";"#,
            r#"let _ = c"\u{0_000000}";"#,
            r#"let _ = c"\u{1234567}";"#,
            r#"let _ = c"\u{1_234567}";"#,
            "let _ = c\"raw\0value\";",
            "let _ = cr\"raw\0value\";",
            "let _ = cr\"raw\rvalue\";",
        ] {
            let error = super::analyze(body)
                .expect_err("an invalid C string must not parse without an error");
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::InvalidData,
                "{body:?}: {error}"
            );
            if body == r#"let _ = c"\0";"# {
                insta::assert_snapshot!("invalid_c_string_literal_error", error);
            }
        }
    }

    #[test]
    fn parses_precise_capture_bounds() {
        let body = "fn capture<'a, T, const N: usize>()\n\
                    -> impl Copy + use<'a, T, N> { Alias }";
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
