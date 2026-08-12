// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
/// Renders parser predicate metadata shared by generated predicate checks.
#[allow(dead_code)]
fn render_parser_predicate_constant(
    predicates: &[((usize, usize), PredicateTemplate)],
    data: &RecognizerCodegenData<'_>,
) -> io::Result<String> {
    let predicates = render_parser_predicate_array(predicates, data)?;
    Ok(format!(
        "#[allow(dead_code)]\nconst PARSER_PREDICATES: &[(usize, usize, antlr4_runtime::ParserPredicate)] = &{predicates};\n"
    ))
}

pub(crate) fn render_parser_semantics_function(
    predicates: &[((usize, usize), PredicateTemplate)],
    data: &RecognizerCodegenData<'_>,
) -> io::Result<String> {
    let predicate_builders = render_parser_semir_predicate_builders(predicates, data)?;
    Ok(format!(
        r#"fn parser_semantics() -> &'static antlr4_runtime::ParserSemantics {{
    static SEMANTICS_CELL: OnceLock<antlr4_runtime::ParserSemantics> = OnceLock::new();
    SEMANTICS_CELL.get_or_init(|| {{
        let mut ir = antlr4_runtime::semir::SemIr::new();
        let mut predicates = Vec::new();
{predicate_builders}
        let actions = Vec::new();
        antlr4_runtime::ParserSemantics {{ ir, predicates, actions }}
    }})
}}
"#
    ))
}

/// Renders the declared member initial values as a `[(slot, value), ...]`
/// literal, or an empty string when the inventory declares none (which keeps
/// every other grammar's generated output byte-identical).
///
/// Both recognizers seed from this: the lexer through
/// `BaseLexer::with_initial_members`, the parser through
/// `BaseParser::set_initial_members`. A parser predicate reading a slot that
/// silently started at 0 would reject input the source grammar accepts, exactly
/// as on the lexer side.
pub(crate) fn render_member_init_seeds(
    patterns: &SemPatternFile,
    recognizer: stack_member::MemberScope,
) -> io::Result<String> {
    let slots = patterns.member_slots_for(recognizer)?;
    let seeds = slots
        .scalar_inits()
        .map(|(slot, value)| format!("({slot}, {value})"))
        .collect::<Vec<_>>();
    if seeds.is_empty() {
        return Ok(String::new());
    }
    Ok(format!("[{}]", seeds.join(", ")))
}

/// Whether any coordinate lowered into the lexer `SemIR` table.
fn has_lexer_semantics(
    predicates: &[((usize, usize), PredicateTemplate)],
    actions: &[((i32, i32), ActionTemplate)],
) -> bool {
    predicates
        .iter()
        .any(|(_, template)| matches!(template, PredicateTemplate::MemberExpr(_)))
        || actions
            .iter()
            .any(|(_, template)| matches!(template, ActionTemplate::MemberStmt(_)))
}

/// Renders the generated lexer's `SemIR` table for member-state coordinates
/// (issue #206), mirroring `parser_semantics()`.
pub(crate) fn render_lexer_semantics_function(
    predicates: &[((usize, usize), PredicateTemplate)],
    actions: &[((i32, i32), ActionTemplate)],
) -> String {
    if !has_lexer_semantics(predicates, actions) {
        return String::new();
    }
    let mut body = String::new();
    let mut next = 0;
    for ((rule_index, pred_index), template) in predicates {
        let PredicateTemplate::MemberExpr(expr) = template else {
            continue;
        };
        let root = stack_member::render_member_expr(expr, &mut body, &mut next);
        writeln!(
            body,
            "        predicates.push(antlr4_runtime::LexerSemanticPredicate {{ rule_index: {rule_index}, pred_index: {pred_index}, expr: {root} }});"
        )
        .expect("writing to a string cannot fail");
    }
    for ((rule_index, action_index), template) in actions {
        let ActionTemplate::MemberStmt(stmt) = template else {
            continue;
        };
        let root = stack_member::render_member_stmt(stmt, &mut body, &mut next);
        writeln!(
            body,
            "        actions.push(antlr4_runtime::LexerSemanticAction {{ rule_index: {rule_index}, action_index: {action_index}, stmt: {root} }});"
        )
        .expect("writing to a string cannot fail");
    }
    // Owns its surrounding blank lines so an empty render leaves the module
    // byte-identical to one generated before this table existed.
    format!(
        r#"
fn lexer_semantics() -> &'static antlr4_runtime::LexerSemantics {{
    static SEMANTICS_CELL: OnceLock<antlr4_runtime::LexerSemantics> = OnceLock::new();
    SEMANTICS_CELL.get_or_init(|| {{
        let mut ir = antlr4_runtime::semir::SemIr::new();
        let mut predicates = Vec::new();
        let mut actions = Vec::new();
{body}        antlr4_runtime::LexerSemantics {{ ir, predicates, actions }}
    }})
}}
"#
    )
}

fn render_parser_semir_predicate_builders(
    predicates: &[((usize, usize), PredicateTemplate)],
    data: &RecognizerCodegenData<'_>,
) -> io::Result<String> {
    let mut out = String::new();
    for ((rule_index, pred_index), predicate) in predicates {
        let expr = render_parser_semir_predicate_expr(predicate, data)?;
        // Carry the `<fail=...>` message for ANY predicate that supplies one, not
        // only a constant-false one — a hook/lookahead/member predicate that
        // returns false at runtime should surface the grammar's fail text too.
        let failure_message = predicate_template_fail_message(predicate).map_or_else(
            || "None".to_owned(),
            |message| format!("Some(\"{}\")", rust_string(message)),
        );
        writeln!(
            out,
            "        let __expr = {expr};\n        predicates.push(antlr4_runtime::ParserSemanticPredicate {{ rule_index: {rule_index}, pred_index: {pred_index}, expr: __expr, failure_message: {failure_message} }});"
        )
        .expect("writing to a string cannot fail");
    }
    Ok(out)
}

#[allow(clippy::too_many_lines)]
fn render_parser_semir_predicate_expr(
    predicate: &PredicateTemplate,
    data: &RecognizerCodegenData<'_>,
) -> io::Result<String> {
    match predicate {
        // A `<fail=...>` wrapper is transparent to evaluation; lower its inner.
        PredicateTemplate::WithFailMessage { inner, .. } => {
            render_parser_semir_predicate_expr(inner, data)
        }
        PredicateTemplate::Hook
        | PredicateTemplate::UnknownWithFailMessage { .. }
        | PredicateTemplate::Unknown => Ok(
            "ir.expr(antlr4_runtime::semir::PExpr::Hook(antlr4_runtime::semir::HookId::new(0)))"
                .to_owned(),
        ),
        PredicateTemplate::True => {
            Ok("ir.expr(antlr4_runtime::semir::PExpr::Bool(true))".to_owned())
        }
        PredicateTemplate::False | PredicateTemplate::FalseWithMessage { .. } => {
            Ok("ir.expr(antlr4_runtime::semir::PExpr::Bool(false))".to_owned())
        }
        PredicateTemplate::Invoke { value } => Ok(format!(
            "ir.expr(antlr4_runtime::semir::PExpr::EvalTrace({value}))"
        )),
        PredicateTemplate::LocalIntEquals { value } => Ok(render_local_arg_semir_cmp("Eq", *value)),
        PredicateTemplate::LocalIntLessOrEqual { value } => {
            Ok(render_local_arg_semir_cmp("Le", *value))
        }
        PredicateTemplate::LookaheadTextEquals { offset, text } => Ok(format!(
            "{{ let __actual = ir.expr(antlr4_runtime::semir::PExpr::TokenText({offset})); let __text = ir.intern(\"{}\"); let __expected = ir.expr(antlr4_runtime::semir::PExpr::Str(__text)); ir.expr(antlr4_runtime::semir::PExpr::Cmp(antlr4_runtime::semir::CmpOp::Eq, __actual, __expected)) }}",
            rust_string(text)
        )),
        PredicateTemplate::LookaheadNotEquals { offset, token_name } => {
            let token_type = token_type_for_name(data, token_name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown predicate token {token_name}"),
                )
            })?;
            Ok(format!(
                "{{ let __actual = ir.expr(antlr4_runtime::semir::PExpr::La({offset})); let __expected = ir.expr(antlr4_runtime::semir::PExpr::Int({token_type})); ir.expr(antlr4_runtime::semir::PExpr::Cmp(antlr4_runtime::semir::CmpOp::Ne, __actual, __expected)) }}"
            ))
        }
        PredicateTemplate::TokenPairAdjacent => {
            Ok("ir.expr(antlr4_runtime::semir::PExpr::TokenIndexAdjacent)".to_owned())
        }
        PredicateTemplate::ContextChildRuleTextNotEquals { rule_name, text } => {
            let rule_index = data
                .rule_names
                .iter()
                .position(|name| name == rule_name)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unknown predicate rule {rule_name}"),
                    )
                })?;
            Ok(format!(
                "{{ let __actual = ir.expr(antlr4_runtime::semir::PExpr::CtxRuleText({rule_index})); let __text = ir.intern(\"{}\"); let __expected = ir.expr(antlr4_runtime::semir::PExpr::Str(__text)); ir.expr(antlr4_runtime::semir::PExpr::Cmp(antlr4_runtime::semir::CmpOp::Ne, __actual, __expected)) }}",
                rust_string(text)
            ))
        }
        // Parsers declare `@members` too, so member-state predicates lower
        // here as well as in the lexer (issue #206).
        PredicateTemplate::MemberExpr(expr) => {
            let mut body = String::new();
            let mut next = 0;
            let root = stack_member::render_member_expr(expr, &mut body, &mut next);
            // The renderer emits `let` bindings, so wrap them in a block
            // expression that evaluates to the root id.
            Ok(format!("{{ {} {root} }}", body.trim()))
        }
        PredicateTemplate::TextEquals(_)
        | PredicateTemplate::TokenStartColumnEquals(_)
        | PredicateTemplate::ColumnLessThan(_)
        | PredicateTemplate::ColumnGreaterOrEqual(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "lexer-only predicate cannot be lowered for parser SemIR",
        )),
    }
}

fn render_local_arg_semir_cmp(op: &str, value: i64) -> String {
    format!(
        "{{ let __local = ir.expr(antlr4_runtime::semir::PExpr::LocalArg); let __absent = ir.expr(antlr4_runtime::semir::PExpr::IsNull(__local)); let __expected = ir.expr(antlr4_runtime::semir::PExpr::Int({value})); let __comparison = ir.expr(antlr4_runtime::semir::PExpr::Cmp(antlr4_runtime::semir::CmpOp::{op}, __local, __expected)); ir.expr(antlr4_runtime::semir::PExpr::Or([__absent, __comparison].into())) }}"
    )
}

/// Renders parser predicate metadata as an inline slice consumed by the runtime
/// parser interpreter.
#[allow(dead_code)]
fn render_parser_predicate_array(
    predicates: &[((usize, usize), PredicateTemplate)],
    data: &RecognizerCodegenData<'_>,
) -> io::Result<String> {
    let mut items = Vec::new();
    for ((rule_index, pred_index), predicate) in predicates {
        // The deprecated `ParserPredicate` table (SemIR is the active path). A
        // `<fail=...>` wrapper on a non-constant-false predicate has no encoding
        // in this legacy enum, so render the transparent inner template; the
        // SemIR predicate builder carries the message on the active path.
        let predicate = predicate_effective_template(predicate);
        let expression = match predicate {
            PredicateTemplate::True => "antlr4_runtime::ParserPredicate::True".to_owned(),
            PredicateTemplate::Hook
            | PredicateTemplate::UnknownWithFailMessage { .. }
            | PredicateTemplate::Unknown => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "hook predicates lower only through parser SemIR",
                ));
            }
            PredicateTemplate::False => "antlr4_runtime::ParserPredicate::False".to_owned(),
            PredicateTemplate::FalseWithMessage { message } => {
                format!(
                    "antlr4_runtime::ParserPredicate::FalseWithMessage {{ message: \"{}\" }}",
                    rust_string(message)
                )
            }
            PredicateTemplate::Invoke { value } => {
                format!("antlr4_runtime::ParserPredicate::Invoke {{ value: {value} }}")
            }
            PredicateTemplate::LocalIntEquals { value } => {
                format!("antlr4_runtime::ParserPredicate::LocalIntEquals {{ value: {value} }}")
            }
            PredicateTemplate::LocalIntLessOrEqual { value } => {
                format!("antlr4_runtime::ParserPredicate::LocalIntLessOrEqual {{ value: {value} }}")
            }
            PredicateTemplate::TextEquals(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TextEquals is only supported for lexer predicates",
                ));
            }
            PredicateTemplate::TokenStartColumnEquals(_)
            | PredicateTemplate::ColumnLessThan(_)
            | PredicateTemplate::ColumnGreaterOrEqual(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "column predicates are only supported for lexer predicates",
                ));
            }
            // The closed legacy enum has no member-expression encoding; stack
            // members are a SemIR-only capability by design.
            PredicateTemplate::MemberExpr(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "member-state predicates lower only through SemIR",
                ));
            }
            PredicateTemplate::LookaheadTextEquals { offset, text } => {
                format!(
                    "antlr4_runtime::ParserPredicate::LookaheadTextEquals {{ offset: {offset}, text: \"{}\" }}",
                    rust_string(text)
                )
            }
            PredicateTemplate::LookaheadNotEquals { offset, token_name } => {
                let token_type = token_type_for_name(data, token_name).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unknown predicate token {token_name}"),
                    )
                })?;
                format!(
                    "antlr4_runtime::ParserPredicate::LookaheadNotEquals {{ offset: {offset}, token_type: {token_type} }}"
                )
            }
            PredicateTemplate::TokenPairAdjacent => {
                "antlr4_runtime::ParserPredicate::TokenPairAdjacent".to_owned()
            }
            PredicateTemplate::ContextChildRuleTextNotEquals { rule_name, text } => {
                let rule_index = data
                    .rule_names
                    .iter()
                    .position(|name| name == rule_name)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unknown predicate rule {rule_name}"),
                        )
                    })?;
                format!(
                    "antlr4_runtime::ParserPredicate::ContextChildRuleTextNotEquals {{ rule_index: {rule_index}, text: \"{}\" }}",
                    rust_string(text)
                )
            }
            // `predicate_effective_template` above already unwrapped any wrapper;
            // the constructor never nests, so this is unreachable.
            PredicateTemplate::WithFailMessage { .. } => {
                unreachable!("predicate_effective_template unwraps the fail-message wrapper")
            }
        };
        items.push(format!("({rule_index}, {pred_index}, {expression})"));
    }
    Ok(format!("[{}]", items.join(", ")))
}
