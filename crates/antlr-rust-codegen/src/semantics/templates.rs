// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
/// Emits the generated lexer action dispatcher for structurally bound custom
/// actions in the compiled lexer.
pub(crate) fn render_lexer_action_method(
    actions: &[((i32, i32), ActionTemplate)],
) -> String {
    if actions.is_empty() {
        return String::new();
    }
    let mut comments = String::new();
    for (_, template) in actions {
        if let ActionTemplate::UnsupportedLexerAction { rule_name, body } = template {
            writeln!(
                comments,
                "    {}",
                render_unsupported_lexer_action_comment(rule_name, body)
            )
            .expect("writing to a string cannot fail");
        }
    }
    if !lexer_actions_need_dispatch(actions) {
        return comments;
    }
    let mut arms = String::new();
    for ((rule_index, action_index), template) in actions {
        if !lexer_action_template_needs_dispatch(template) {
            continue;
        }
        let statement = render_lexer_action_statement(template);
        writeln!(
            arms,
            "            ({rule_index}, {action_index}) => {{ {statement} true }}"
        )
        .expect("writing to a string cannot fail");
    }
    arms.push_str("            _ => false,\n");
    format!(
        "{comments}    fn run_action(_base: &mut BaseLexer<I>, action: antlr4_runtime::LexerCustomAction) -> bool {{\n        match (action.rule_index(), action.action_index()) {{\n{arms}        }}\n    }}\n"
    )
}

pub(crate) fn lexer_actions_need_dispatch(actions: &[((i32, i32), ActionTemplate)]) -> bool {
    actions
        .iter()
        .any(|(_, template)| lexer_action_template_needs_dispatch(template))
}

fn lexer_action_template_needs_dispatch(template: &ActionTemplate) -> bool {
    match template {
        ActionTemplate::Hook(_) | ActionTemplate::UnsupportedLexerAction { .. } => false,
        ActionTemplate::LexerPopMode | ActionTemplate::MemberStmt(_) => {
            !render_lexer_action_statement(template).is_empty()
        }
    }
}

/// Renders one supported lexer target-template action as Rust code.
pub(crate) fn render_lexer_action_statement(template: &ActionTemplate) -> String {
    match template {
        ActionTemplate::LexerPopMode => "_base.pop_mode();".to_owned(),
        // Member mutations live in the `SemIR` table, so the dispatch arm just
        // executes the table entry for this coordinate.
        ActionTemplate::MemberStmt(_) => {
            "let _ = lexer_semantics().exec_action(_base, action);".to_owned()
        }
        ActionTemplate::Hook(_) => String::new(),
        ActionTemplate::UnsupportedLexerAction { rule_name, body } => {
            render_unsupported_lexer_action_comment(rule_name, body)
        }
    }
}

fn render_unsupported_lexer_action_comment(rule_name: &str, body: &str) -> String {
    format!(
        "/* TODO unsupported embedded lexer action in rule {}: {{{}}}; rewrite target-specific actions as portable lexer commands where possible */",
        rust_block_comment_text(rule_name),
        rust_block_comment_text(body)
    )
}

/// Emits the generated lexer predicate dispatcher for structurally bound
/// predicate coordinates in the compiled lexer.
pub(crate) fn render_lexer_predicate_method(
    predicates: &[((usize, usize), PredicateTemplate)],
    sem_unknown: SemUnknownPolicy,
) -> String {
    if predicates.is_empty() {
        return String::new();
    }
    let mut arms = String::new();
    for ((rule_index, pred_index), template) in predicates {
        let statement = if matches!(template, PredicateTemplate::Hook) {
            "None".to_owned()
        } else {
            format!("Some({})", render_lexer_predicate_expression(template))
        };
        writeln!(
            arms,
            "            ({rule_index}, {pred_index}) => {{ {statement} }}"
        )
        .expect("writing to a string cannot fail");
    }
    // The catch-all arm is the unknown-lexer-predicate handler for any
    // coordinate this grammar left untranslated. `--sem-unknown=assume-false`
    // must flip it to `false` here too; otherwise a mixed lexer (one translated
    // predicate plus an uncovered coordinate) keeps the uncovered guard viable
    // even though the manifest promised assume-false.
    let default_arm = if sem_unknown == SemUnknownPolicy::AssumeFalse {
        "            _ => Some(false),\n"
    } else if sem_unknown == SemUnknownPolicy::Hook {
        "            _ => None,\n"
    } else {
        "            _ => Some(true),\n"
    };
    arms.push_str(default_arm);
    format!(
        "    fn run_predicate(_base: &BaseLexer<I>, predicate: antlr4_runtime::LexerPredicate) -> Option<bool> {{\n        match (predicate.rule_index(), predicate.pred_index()) {{\n{arms}        }}\n    }}\n"
    )
}

fn render_lexer_predicate_expression(template: &PredicateTemplate) -> String {
    match template {
        // A `<fail=...>` wrapper is transparent to evaluation; render its inner.
        PredicateTemplate::WithFailMessage { inner, .. } => {
            render_lexer_predicate_expression(inner)
        }
        PredicateTemplate::True => "true".to_owned(),
        PredicateTemplate::False => "false".to_owned(),
        PredicateTemplate::TextEquals(value) => format!(
            "_base.token_text_until(predicate.position()) == \"{}\"",
            rust_string(value)
        ),
        PredicateTemplate::TokenStartColumnEquals(value) => {
            format!("_base.token_start_column() == {value}")
        }
        PredicateTemplate::ColumnLessThan(value) => {
            format!("_base.column_at(predicate.position()) < {value}")
        }
        PredicateTemplate::ColumnGreaterOrEqual(value) => {
            format!("_base.column_at(predicate.position()) >= {value}")
        }
        // Member predicates evaluate from the `SemIR` table. The coordinate is
        // present there by construction, so `unwrap_or(false)` is unreachable
        // rather than a silent default.
        PredicateTemplate::MemberExpr(_) => {
            "lexer_semantics().eval_predicate(_base, predicate).unwrap_or(false)".to_owned()
        }
        PredicateTemplate::Hook
        | PredicateTemplate::UnknownWithFailMessage { .. }
        | PredicateTemplate::Unknown
        | PredicateTemplate::Invoke { .. }
        | PredicateTemplate::FalseWithMessage { .. }
        | PredicateTemplate::LocalIntEquals { .. }
        | PredicateTemplate::LocalIntLessOrEqual { .. }
        | PredicateTemplate::LookaheadTextEquals { .. }
        | PredicateTemplate::LookaheadNotEquals { .. }
        | PredicateTemplate::TokenPairAdjacent
        | PredicateTemplate::ContextChildRuleTextNotEquals { .. } => {
            unreachable!("lookahead parser predicates are not lexer predicates")
        }
    }
}

/// Reports whether a parser action source state carries an `assume-true` /
/// `assume-false` override. Such a coordinate is a documented silent no-op:
/// it must NOT fall through to the `parser_action_hook` catch-all (which fails
/// loud under the Error policy or runs a user side effect). It gets an explicit
/// empty arm instead. A `hook` (or `error`) override is excluded here so it
/// still routes to the hook.
pub(crate) fn parser_action_assume_overridden(
    patterns: &SemPatternFile,
    data: &RecognizerCodegenData<'_>,
    action_state_coordinates: &BTreeMap<usize, (usize, Option<usize>)>,
    state: usize,
) -> bool {
    let (rule_index, action_index) = action_state_coordinates
        .get(&state)
        .copied()
        .unwrap_or((usize::MAX, None));
    let rule_name = data.rule_names.get(rule_index).map(String::as_str);
    patterns
        .coordinate_override(
            SemanticsKind::ParserAction,
            rule_name,
            action_index,
            Some(state),
        )
        .is_some_and(|override_| {
            matches!(
                override_.dispose,
                CoordinateDispose::AssumeTrue | CoordinateDispose::AssumeFalse
            )
        })
}

pub(crate) fn render_parser_action_method(
    has_action_states: bool,
    noop_states: &BTreeSet<usize>,
) -> String {
    if !has_action_states {
        return "    fn run_action(&mut self, _action: antlr4_runtime::ParserAction, _tree: antlr4_runtime::ParseTree) {}\n"
            .to_owned();
    }
    let mut arms = String::new();
    for state in noop_states {
        writeln!(arms, "            {state} => {{}}").expect("writing to a string cannot fail");
    }
    arms.push_str("            _ => { let _ = self.base.parser_action_hook(action, tree); }\n");
    format!(
        "    fn run_action(&mut self, action: antlr4_runtime::ParserAction, tree: antlr4_runtime::ParseTree) {{\n        match action.source_state() {{\n{arms}        }}\n    }}\n"
    )
}

pub(crate) fn likely_parser_entry_rule_indices(data: &ParserCodegenData<'_>) -> Vec<usize> {
    if let Some(semantic) = data.semantic {
        return semantic
            .entry_rules
            .iter()
            .map(|rule| semantic.recognizer.rule_numbers[rule])
            .collect();
    }
    let atn = data.parser_atn();
    likely_parser_entry_rule_indices_from_atn(
        atn,
        data.rule_names.len(),
    )
}

pub(crate) fn likely_parser_entry_rule_indices_from_atn(
    atn: &ParserAtn,
    rule_count: usize,
) -> Vec<usize> {
    let mut called_by_other_rule = vec![false; rule_count];
    for state in atn.states() {
        for transition in state.transitions() {
            let ParserTransitionData::Rule { rule_index, .. } = transition.data() else {
                continue;
            };
            if rule_index >= rule_count || state.rule_index() == Some(rule_index) {
                continue;
            }
            called_by_other_rule[rule_index] = true;
        }
    }
    called_by_other_rule
        .iter()
        .enumerate()
        .filter_map(|(index, called)| (!called).then_some(index))
        .collect()
}

/// Renders the generated parser type rustdoc that surfaces callable rule methods.
pub(crate) fn render_parser_rustdoc(
    public_rule_method_names: &[String],
    entry_rule_indices: &[usize],
) -> String {
    let all_method_capacity = public_rule_method_names
        .iter()
        .map(|method| method.len() + "/// - `()`\n".len())
        .sum::<usize>();
    let entry_method_capacity = entry_rule_indices
        .iter()
        .filter_map(|index| public_rule_method_names.get(*index))
        .map(|method| method.len() + "/// - `()`\n".len())
        .sum::<usize>();
    let mut out = String::with_capacity(384 + all_method_capacity + entry_method_capacity);
    writeln!(
        out,
        "/// Generated parser. Each grammar rule is exposed as a public method."
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "///").expect("writing to a string cannot fail");
    writeln!(
        out,
        "/// Pick an entry-rule method that matches the grammar's intended"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "/// top-level construct for the input being parsed. The generator can"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "/// infer entry candidates from call paths that reach explicit `EOF`"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "/// matches, from parser rules that no other rule calls, and from"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "/// configured entry rules. It cannot infer the semantic choice"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "/// between multiple candidates.").expect("writing to a string cannot fail");
    if !entry_rule_indices.is_empty() {
        writeln!(out, "///").expect("writing to a string cannot fail");
        writeln!(out, "/// Likely parser entry-rule methods:")
            .expect("writing to a string cannot fail");
        for index in entry_rule_indices {
            let Some(method_name) = public_rule_method_names.get(*index) else {
                continue;
            };
            writeln!(out, "/// - `{method_name}()`").expect("writing to a string cannot fail");
        }
    }
    if !public_rule_method_names.is_empty() {
        writeln!(out, "///").expect("writing to a string cannot fail");
        writeln!(out, "/// All parser rule methods:").expect("writing to a string cannot fail");
        for method_name in public_rule_method_names {
            writeln!(out, "/// - `{method_name}()`").expect("writing to a string cannot fail");
        }
    }
    out
}
