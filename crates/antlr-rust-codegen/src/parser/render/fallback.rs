#[derive(Clone, Copy)]
pub(crate) struct ParserFallbackRender<'a> {
    pub(crate) track_alt_numbers: bool,
    pub(crate) track_context_alt_numbers: bool,
    pub(crate) rule_args: &'a [(usize, usize, RuleArgTemplate)],
    pub(crate) action_indices: &'a [(usize, usize)],
    pub(crate) has_action_dispatch: bool,
    pub(crate) has_predicate_dispatch: bool,
    pub(crate) unknown_policy_literal: Option<&'a str>,
}

/// Renders the interpreted-fallback binder block for the runtime's
/// `__antlr4_rust_parser_driver!` macro.
///
/// The block composes the grammar's `ParserRuntimeOptions` (semantics table,
/// action indices, alt-number tracking, unknown-coordinate policy) and must
/// evaluate to `Result<(ParseTree, Vec<ParserAction>), AntlrError>`; the
/// driver macro owns the surrounding action dispatch, which is uniform
/// because every generated parser defines `run_action` (an empty method when
/// the grammar has no action states).
pub(crate) fn render_parser_parse_rule_fallback(options: ParserFallbackRender<'_>) -> String {
    let ParserFallbackRender {
        track_alt_numbers,
        track_context_alt_numbers,
        rule_args,
        action_indices,
        has_action_dispatch,
        has_predicate_dispatch,
        unknown_policy_literal,
    } = options;
    let action_indices = render_parser_action_index_array(action_indices);
    let rule_args = render_parser_rule_arg_array(rule_args);
    let call = if has_predicate_dispatch || unknown_policy_literal.is_some() {
        format!(
            "parser.base.parse_atn_rule_with_runtime_options_and_precedence(atn(), rule_index, precedence, antlr4_runtime::ParserRuntimeOptions {{ action_indices: &{action_indices}, track_alt_numbers: {track_alt_numbers}, track_context_alt_numbers: {track_context_alt_numbers}, predicates: &[], semantics: Some(parser_semantics()), rule_args: &{rule_args}, member_actions: &[], return_actions: &[], unknown_predicate_policy: {} , ..antlr4_runtime::ParserRuntimeOptions::default() }})",
            unknown_policy_literal.unwrap_or("antlr4_runtime::UnknownSemanticPolicy::AssumeTrue")
        )
    } else if track_alt_numbers || track_context_alt_numbers {
        format!(
            "parser.base.parse_atn_rule_with_runtime_options_and_precedence(atn(), rule_index, precedence, antlr4_runtime::ParserRuntimeOptions {{ action_indices: &{action_indices}, track_alt_numbers: {track_alt_numbers}, track_context_alt_numbers: {track_context_alt_numbers}, rule_args: &{rule_args}, ..antlr4_runtime::ParserRuntimeOptions::default() }})"
        )
    } else if has_action_dispatch {
        format!(
            "parser.base.parse_atn_rule_with_runtime_options_and_precedence(atn(), rule_index, precedence, antlr4_runtime::ParserRuntimeOptions {{ action_indices: &{action_indices}, rule_args: &{rule_args}, ..antlr4_runtime::ParserRuntimeOptions::default() }})"
        )
    } else {
        // The plain path returns only a tree; adapt it to the driver's
        // uniform `(tree, actions)` contract without allocating.
        "parser.base.parse_atn_rule_with_precedence(atn(), rule_index, precedence).map(|tree| (tree, Vec::new()))"
            .to_owned()
    };
    format!("        {call}")
}
