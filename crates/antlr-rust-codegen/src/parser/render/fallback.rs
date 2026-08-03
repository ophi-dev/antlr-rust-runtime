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
    let mut out = String::new();
    let action_indices = render_parser_action_index_array(action_indices);
    let rule_args = render_parser_rule_arg_array(rule_args);
    if has_predicate_dispatch || unknown_policy_literal.is_some() {
        writeln!(
            out,
            "let (tree, actions) = self.base.parse_atn_rule_with_runtime_options_and_precedence(atn(), rule_index, precedence, antlr4_runtime::ParserRuntimeOptions {{ action_indices: &{action_indices}, track_alt_numbers: {track_alt_numbers}, track_context_alt_numbers: {track_context_alt_numbers}, predicates: &[], semantics: Some(parser_semantics()), rule_args: &{rule_args}, member_actions: &[], return_actions: &[], unknown_predicate_policy: {} , ..antlr4_runtime::ParserRuntimeOptions::default() }})?;",
            unknown_policy_literal
                .unwrap_or("antlr4_runtime::UnknownSemanticPolicy::AssumeTrue")
        )
        .expect("writing to a string cannot fail");
    } else if track_alt_numbers || track_context_alt_numbers {
        writeln!(
            out,
            "let (tree, actions) = self.base.parse_atn_rule_with_runtime_options_and_precedence(atn(), rule_index, precedence, antlr4_runtime::ParserRuntimeOptions {{ action_indices: &{action_indices}, track_alt_numbers: {track_alt_numbers}, track_context_alt_numbers: {track_context_alt_numbers}, rule_args: &{rule_args}, ..antlr4_runtime::ParserRuntimeOptions::default() }})?;"
        )
        .expect("writing to a string cannot fail");
    } else if has_action_dispatch {
        writeln!(
            out,
            "let (tree, actions) = self.base.parse_atn_rule_with_runtime_options_and_precedence(atn(), rule_index, precedence, antlr4_runtime::ParserRuntimeOptions {{ action_indices: &{action_indices}, rule_args: &{rule_args}, ..antlr4_runtime::ParserRuntimeOptions::default() }})?;"
        )
        .expect("writing to a string cannot fail");
    } else {
        return "self.base.parse_atn_rule_with_precedence(atn(), rule_index, precedence)"
            .to_owned();
    }

    if has_action_dispatch {
        writeln!(
            out,
            "for action in actions {{ self.run_action(action, tree); }}"
        )
        .expect("writing to a string cannot fail");
    } else {
        writeln!(out, "let _ = actions;").expect("writing to a string cannot fail");
    }
    writeln!(out, "Ok(tree)").expect("writing to a string cannot fail");
    out.lines()
        .map(|line| format!("        {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
