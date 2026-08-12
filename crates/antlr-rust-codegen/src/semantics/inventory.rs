// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
/// One semantic predicate/action coordinate inventoried for the manifest.
///
/// Optional fields reflect the coordinate kind or the absence of authored
/// source for a synthetic action. Authored bodies and spans come from the same
/// structural element that owns the finalized ATN transition.
#[derive(Clone, Debug)]
pub(crate) struct SemanticsEntry {
    pub(crate) kind: SemanticsKind,
    pub(crate) rule_index: Option<usize>,
    pub(crate) rule_name: Option<String>,
    pub(crate) index: Option<usize>,
    pub(crate) atn_state: Option<usize>,
    pub(crate) line: Option<usize>,
    pub(crate) column: Option<usize>,
    pub(crate) body: Option<String>,
    pub(crate) disposition: SemanticsDisposition,
    pub(crate) template: Option<String>,
}

impl SemanticsEntry {
    /// Renders the fail-loud error line for this coordinate, matching the
    /// shape documented in the compatibility-boundary docs.
    fn describe_unsupported(&self) -> String {
        let mut message = String::from(self.kind.error_label());
        message.push(':');
        match (&self.rule_name, self.rule_index) {
            (Some(name), Some(rule_index)) => {
                let _ = write!(message, " rule={name}({rule_index})");
            }
            (None, Some(rule_index)) => {
                let _ = write!(message, " rule_index={rule_index}");
            }
            _ => {}
        }
        if let Some(index) = self.index {
            let label = match self.kind {
                SemanticsKind::LexerPredicate | SemanticsKind::ParserPredicate => "pred_index",
                SemanticsKind::LexerAction | SemanticsKind::ParserAction => "action_index",
            };
            let _ = write!(message, " {label}={index}");
        }
        if let Some(atn_state) = self.atn_state {
            let _ = write!(message, " atn_state={atn_state}");
        }
        if let (Some(line), Some(column)) = (self.line, self.column) {
            let _ = write!(message, " at {line}:{column}");
        }
        if let Some(body) = &self.body {
            let _ = write!(message, ": {{{body}}}");
        }
        message
    }
}

/// Fails generation when coordinates must be rejected at codegen: either the
/// global `--sem-unknown=error` policy is active (every unimplemented
/// coordinate), or a per-coordinate `dispose = "error"` override rejects a
/// specific coordinate regardless of the global policy.
///
/// A per-coordinate error override otherwise lowers to no `SemIR` entry and
/// does not escalate the runtime policy, so without this it would silently fall
/// back to the global default (e.g. `AssumeTrue`) instead of failing.
pub(crate) fn enforce_sem_unknown(
    policy: SemUnknownPolicy,
    entries: &[SemanticsEntry],
) -> io::Result<()> {
    let unsupported = entries
        .iter()
        .filter(|entry| {
            // A per-coordinate `dispose = "error"` override is always fatal.
            if entry.disposition == SemanticsDisposition::Error {
                return true;
            }
            // Under the global Error policy, any unimplemented coordinate is
            // fatal — except a `Synthetic` action ANTLR inserted itself (no
            // author intent to implement).
            policy == SemUnknownPolicy::Error
                && !matches!(
                    entry.disposition,
                    SemanticsDisposition::Translated
                        | SemanticsDisposition::Hooked
                        | SemanticsDisposition::Synthetic
                )
        })
        .collect::<Vec<_>>();
    if unsupported.is_empty() {
        return Ok(());
    }
    let mut message = String::new();
    for entry in &unsupported {
        message.push_str(&entry.describe_unsupported());
        message.push('\n');
    }
    let _ = write!(
        message,
        "--sem-unknown=error: {} semantic coordinate(s) have no Rust implementation and are \
         configured to fail; add a semantic pattern, adjust a coordinate's `dispose`, or accept \
         a documented fallback with \
         --sem-unknown=assume-true / assume-false",
        unsupported.len()
    );
    Err(io::Error::new(io::ErrorKind::InvalidData, message))
}

pub(crate) fn enforce_require_full_semantics(
    require: bool,
    entries: &[SemanticsEntry],
) -> io::Result<()> {
    if !require {
        return Ok(());
    }
    let fallback = entries
        .iter()
        .filter(|entry| {
            // A `Synthetic` action is an ANTLR internal, not a missing author
            // semantic, so it does not count as an unimplemented fallback.
            !matches!(
                entry.disposition,
                SemanticsDisposition::Translated
                    | SemanticsDisposition::Hooked
                    | SemanticsDisposition::Synthetic
            )
        })
        .collect::<Vec<_>>();
    if fallback.is_empty() {
        return Ok(());
    }
    let mut message = String::new();
    for entry in &fallback {
        message.push_str(&entry.describe_unsupported());
        message.push('\n');
    }
    let _ = write!(
        message,
        "--require-full-semantics: {} semantic coordinate(s) use policy fallback dispositions",
        fallback.len()
    );
    Err(io::Error::new(io::ErrorKind::InvalidData, message))
}

pub(crate) fn grammar_option_warning_messages(entries: &[GrammarOptionEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.disposition == GrammarOptionDisposition::Unsupported)
        .map(|entry| format!("warning: {}", entry.describe_unsupported()))
        .collect()
}

pub(crate) fn enforce_require_full_options(
    require: bool,
    entries: &[GrammarOptionEntry],
) -> io::Result<()> {
    if !require {
        return Ok(());
    }
    let unsupported = entries
        .iter()
        .filter(|entry| entry.disposition == GrammarOptionDisposition::Unsupported)
        .collect::<Vec<_>>();
    if unsupported.is_empty() {
        return Ok(());
    }
    let mut message = String::new();
    for entry in &unsupported {
        message.push_str(&entry.describe_unsupported());
        message.push('\n');
    }
    let _ = write!(
        message,
        "--require-full-semantics: {} grammar option(s) require caller-owned target behavior; \
         acknowledge implemented behavior with --option-hook KEY=VALUE",
        unsupported.len()
    );
    Err(io::Error::new(io::ErrorKind::InvalidData, message))
}

pub(crate) fn normalize_option_hook(value: &str) -> Result<String, String> {
    let Some((key, option_value)) = value.split_once('=') else {
        return Err(format!("--option-hook requires KEY=VALUE; got {value:?}"));
    };
    let key = key.trim();
    let option_value = option_value.trim();
    let mut key_chars = key.chars();
    if !key_chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        || !key_chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        || option_value.is_empty()
    {
        return Err(format!("--option-hook requires KEY=VALUE; got {value:?}"));
    }
    Ok(format!("{key}={option_value}"))
}

/// Classifies every structurally bound custom-action and predicate coordinate
/// so manifest dispositions match what the generated module will do.
/// Manifest disposition for a predicate coordinate given its covering
/// template: hook-routed coordinates report `hooked` (the user trait owns
/// them), any other template is a real translation, and uncovered
/// coordinates fall back to the unknown policy.
pub(crate) const fn predicate_template_disposition(
    template: Option<&PredicateTemplate>,
    policy: SemUnknownPolicy,
) -> SemanticsDisposition {
    match template {
        // Unwrap a `<fail=...>` wrapper: disposition follows the inner template
        // (a wrapped hook is still `Hooked`, not `Translated`).
        Some(PredicateTemplate::WithFailMessage { inner, .. }) => {
            predicate_template_disposition(Some(inner), policy)
        }
        Some(PredicateTemplate::Hook) => SemanticsDisposition::Hooked,
        Some(PredicateTemplate::UnknownWithFailMessage { .. } | PredicateTemplate::Unknown) => {
            policy.unknown_predicate_disposition()
        }
        Some(_) => SemanticsDisposition::Translated,
        None => policy.unknown_predicate_disposition(),
    }
}

pub(crate) fn collect_lexer_semantics(
    data: &LexerCodegenData<'_>,
    embedded: bool,
    allow_unsupported_lexer_actions: bool,
    policy: SemUnknownPolicy,
    patterns: &SemPatternFile,
) -> io::Result<Vec<SemanticsEntry>> {
    let actions = if embedded {
        Vec::new()
    } else {
        let actions = structural_lexer_action_templates(data, patterns)?;
        reject_unsupported_lexer_action_templates(
            &actions
                .iter()
                .map(|(_, template)| template.clone())
                .collect::<Vec<_>>(),
            allow_unsupported_lexer_actions,
        )?;
        actions
    };
    let predicates = if embedded {
        Vec::new()
    } else {
        structural_predicate_templates(data, SemanticsKind::LexerPredicate, patterns)?
    };
    let mut entries = Vec::new();
    for action in structural_actions(data)? {
        let coordinate = (
            i32::try_from(action.rule_index).expect("rule index exceeds i32"),
            i32::try_from(action.action_index).expect("action index exceeds i32"),
        );
        let template = actions
            .iter()
            .find(|(covered, _)| *covered == coordinate)
            .map(|(_, template)| template);
        let (line, column) = structural_line_column(data, &action.span);
        entries.push(SemanticsEntry {
            kind: SemanticsKind::LexerAction,
            rule_index: Some(action.rule_index),
            rule_name: data.rule_names.get(action.rule_index).cloned(),
            index: Some(action.action_index),
            atn_state: None,
            line: Some(line),
            column: Some(column),
            body: Some(one_line_action_body(&action.body)),
            disposition: if embedded {
                SemanticsDisposition::Translated
            } else {
                patterns
                    .coordinate_disposition(
                        SemanticsKind::LexerAction,
                        data.rule_names.get(action.rule_index).map(String::as_str),
                        Some(action.action_index),
                        None,
                    )
                    .unwrap_or_else(|| match template {
                        Some(ActionTemplate::Hook(_)) => SemanticsDisposition::Hooked,
                        // A lowered member mutation is fully translated: it
                        // becomes a `LexerSemantics` table entry, so it needs
                        // neither a hook nor a policy fallback.
                        Some(ActionTemplate::LexerPopMode | ActionTemplate::MemberStmt(_)) => {
                            SemanticsDisposition::Translated
                        }
                        Some(ActionTemplate::UnsupportedLexerAction { .. }) | None => {
                            policy.unknown_action_disposition()
                        }
                    })
            },
            template: if embedded {
                Some("Embedded".to_owned())
            } else {
                matches!(
                    template,
                    Some(ActionTemplate::LexerPopMode | ActionTemplate::MemberStmt(_))
                )
                .then(|| format!("{:?}", template.expect("matched template")))
            },
        });
    }
    for predicate in structural_predicates(data)? {
        let coordinate = (predicate.rule_index, predicate.predicate_index);
        let template = predicates
            .iter()
            .find(|(covered, _)| *covered == coordinate)
            .map(|(_, template)| template);
        let (line, column) = structural_line_column(data, &predicate.span);
        entries.push(SemanticsEntry {
            kind: SemanticsKind::LexerPredicate,
            rule_index: Some(predicate.rule_index),
            rule_name: data.rule_names.get(predicate.rule_index).cloned(),
            index: Some(predicate.predicate_index),
            atn_state: None,
            line: Some(line),
            column: Some(column),
            body: Some(one_line_action_body(&predicate.body)),
            disposition: if embedded {
                SemanticsDisposition::Translated
            } else {
                patterns
                    .coordinate_disposition(
                        SemanticsKind::LexerPredicate,
                        data.rule_names
                            .get(predicate.rule_index)
                            .map(String::as_str),
                        Some(predicate.predicate_index),
                        None,
                    )
                    .unwrap_or_else(|| predicate_template_disposition(template, policy))
            },
            template: if embedded {
                Some("Embedded".to_owned())
            } else {
                template.map(|template| format!("{template:?}"))
            },
        });
    }
    Ok(entries)
}

#[cfg(test)]
pub(crate) fn collect_parser_semantics(
    data: &RecognizerCodegenData<'_>,
    policy: SemUnknownPolicy,
    patterns: &SemPatternFile,
) -> io::Result<Vec<SemanticsEntry>> {
    collect_parser_semantics_for_mode(data, false, policy, patterns)
}

pub(crate) fn collect_parser_semantics_for_mode(
    data: &RecognizerCodegenData<'_>,
    embedded: bool,
    policy: SemUnknownPolicy,
    patterns: &SemPatternFile,
) -> io::Result<Vec<SemanticsEntry>> {
    let portable_locals = (!embedded)
        .then(|| build_structural_portable_local_data(data, patterns))
        .transpose()?
        .unwrap_or_default();
    let predicates = if embedded {
        Vec::new()
    } else {
        structural_predicate_templates(data, SemanticsKind::ParserPredicate, patterns)?
    };
    let mut entries = Vec::new();
    for predicate in structural_predicates(data)? {
        let coordinate = (predicate.rule_index, predicate.predicate_index);
        let template = predicates
            .iter()
            .find(|(covered, _)| *covered == coordinate)
            .map(|(_, template)| template);
        let (line, column) = structural_line_column(data, &predicate.span);
        entries.push(SemanticsEntry {
            kind: SemanticsKind::ParserPredicate,
            rule_index: Some(predicate.rule_index),
            rule_name: data.rule_names.get(predicate.rule_index).cloned(),
            index: Some(predicate.predicate_index),
            atn_state: None,
            line: Some(line),
            column: Some(column),
            body: Some(one_line_action_body(&predicate.body)),
            disposition: if embedded {
                SemanticsDisposition::Translated
            } else {
                patterns
                    .coordinate_disposition(
                        SemanticsKind::ParserPredicate,
                        data.rule_names
                            .get(predicate.rule_index)
                            .map(String::as_str),
                        Some(predicate.predicate_index),
                        None,
                    )
                    .unwrap_or_else(|| {
                        if portable_locals.predicates.contains_key(&coordinate) {
                            SemanticsDisposition::Translated
                        } else {
                            predicate_template_disposition(template, policy)
                        }
                    })
            },
            template: if embedded {
                Some("Embedded".to_owned())
            } else {
                portable_locals
                    .predicates
                    .contains_key(&coordinate)
                    .then(|| "PortableBooleanLocal".to_owned())
                    .or_else(|| {
                        template
                            // `Unknown` is an internal lowering of an
                            // untranslated body, not a translation; the
                            // manifest keeps reporting `template: null` so the
                            // disposition remains the user-facing signal.
                            .filter(|template| !matches!(template, PredicateTemplate::Unknown))
                            .map(|template| format!("{template:?}"))
                    })
            },
        });
    }
    for action in structural_actions(data)? {
        let (line, column) = structural_line_column(data, &action.span);
        let hook_call = if embedded || !action.authored || action.body.trim().is_empty() {
            None
        } else {
            patterns.hook_helper_call(SemanticsKind::ParserAction, &action.body)?
        };
        entries.push(SemanticsEntry {
            kind: SemanticsKind::ParserAction,
            rule_index: Some(action.rule_index),
            rule_name: data.rule_names.get(action.rule_index).cloned(),
            index: Some(action.action_index),
            atn_state: Some(action.state),
            line: action.authored.then_some(line),
            column: action.authored.then_some(column),
            body: action.authored.then(|| one_line_action_body(&action.body)),
            disposition: if embedded && action.authored {
                SemanticsDisposition::Translated
            } else if embedded {
                SemanticsDisposition::Synthetic
            } else {
                patterns
                    .coordinate_disposition(
                        SemanticsKind::ParserAction,
                        data.rule_names.get(action.rule_index).map(String::as_str),
                        Some(action.action_index),
                        Some(action.state),
                    )
                    .unwrap_or_else(|| {
                        if portable_locals.inline_actions.contains_key(&action.state) {
                            SemanticsDisposition::Translated
                        } else if !action.authored || action.body.trim().is_empty() {
                            SemanticsDisposition::Synthetic
                        } else if hook_call.is_some() {
                            SemanticsDisposition::Hooked
                        } else {
                            policy.unknown_action_disposition()
                        }
                    })
            },
            template: if embedded && action.authored {
                Some("Embedded".to_owned())
            } else if embedded {
                None
            } else {
                portable_locals
                    .inline_actions
                    .contains_key(&action.state)
                    .then(|| "PortableBooleanLocal".to_owned())
                    .or_else(|| {
                        hook_call
                            .as_ref()
                            .map(|call| format!("Hook({})", rust_function_name(&call.name)))
                    })
            },
        });
    }
    entries.sort_by_key(|entry| {
        (
            entry.rule_index,
            matches!(entry.kind, SemanticsKind::ParserAction),
            entry.index,
            entry.atn_state,
        )
    });
    Ok(entries)
}

pub(crate) fn structural_lexer_action_templates(
    data: &RecognizerCodegenData<'_>,
    patterns: &SemPatternFile,
) -> io::Result<Vec<((i32, i32), ActionTemplate)>> {
    let mut templates = structural_actions(data)?
        .into_iter()
        .map(|action| {
            let rule_name = data
                .rule_names
                .get(action.rule_index)
                .map_or("<unknown>", String::as_str);
            let template = match parse_lexer_action_block_template(&action.body) {
                Some(template) => template,
                // A `stack_member` pattern lowers the body into SemIR, so the
                // grammar needs no hand-written hook for it.
                None => {
                    match patterns.member_action_stmt(SemanticsKind::LexerAction, &action.body)? {
                        Some(stmt) => ActionTemplate::MemberStmt(stmt),
                        None => patterns
                            .hook_helper_call(SemanticsKind::LexerAction, &action.body)?
                            .map_or_else(
                                || ActionTemplate::UnsupportedLexerAction {
                                    rule_name: rule_name.to_owned(),
                                    body: one_line_action_body(&action.body),
                                },
                                ActionTemplate::Hook,
                            ),
                    }
                }
            };
            Ok((
                (
                    i32::try_from(action.rule_index).expect("rule index exceeds i32"),
                    i32::try_from(action.action_index).expect("action index exceeds i32"),
                ),
                template,
            ))
        })
        .collect::<io::Result<Vec<_>>>()?;
    templates.sort_by_key(|(coordinate, _)| *coordinate);
    templates.dedup_by_key(|(coordinate, _)| *coordinate);
    Ok(templates)
}

pub(crate) fn structural_predicate_templates(
    data: &RecognizerCodegenData<'_>,
    kind: SemanticsKind,
    patterns: &SemPatternFile,
) -> io::Result<Vec<((usize, usize), PredicateTemplate)>> {
    let mut templates = Vec::new();
    for predicate in structural_predicates(data)? {
        let coordinate = (predicate.rule_index, predicate.predicate_index);
        let rule_name = data
            .rule_names
            .get(predicate.rule_index)
            .map(String::as_str);
        let (template, parsed_body) = match patterns.coordinate_predicate_template(
            kind,
            rule_name,
            Some(predicate.predicate_index),
        ) {
            Some(template) => (template, false),
            None => (
                parse_predicate_template_with_patterns_kind(&predicate.body, patterns, kind)?,
                true,
            ),
        };
        if parsed_body && template.is_none() && is_unsupported_string_template_body(&predicate.body)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported target predicate template <{}>", predicate.body),
            ));
        }
        match (template, predicate.fail) {
            (Some(template), Some(message)) => {
                templates.push((
                    coordinate,
                    predicate_template_with_fail_message(template, message),
                ));
            }
            (Some(template), None) => templates.push((coordinate, template)),
            (None, Some(message)) if kind == SemanticsKind::ParserPredicate => {
                templates.push((
                    coordinate,
                    PredicateTemplate::UnknownWithFailMessage { message },
                ));
            }
            // An untranslated parser predicate still lowers (hook -> policy
            // evaluator) so its rule keeps a generated body; an uncovered
            // coordinate would cascade every calling rule onto the interpreter
            // (issue #209). `parsed_body` keeps a `dispose = "error"` coordinate
            // override uncovered: that override documents "no SemIR entry" and
            // is enforced fatal before rendering.
            (None, None) if parsed_body && kind == SemanticsKind::ParserPredicate => {
                templates.push((coordinate, PredicateTemplate::Unknown));
            }
            (None, _) => {}
        }
    }
    Ok(templates)
}

pub(crate) fn collect_structural_grammar_options(
    data: &RecognizerCodegenData<'_>,
    option_hooks: &BTreeSet<String>,
) -> io::Result<Vec<GrammarOptionEntry>> {
    let semantic = data.semantic.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "structural grammar model is unavailable",
        )
    })?;
    Ok(semantic
        .unit
        .options
        .iter()
        .map(|option| {
            let (line, column) = structural_line_column(data, &option.name.span);
            let key = option.name.value.clone();
            let value = option.value.value.clone();
            let assignment = format!("{key}={value}");
            let disposition = match key.as_str() {
                "tokenVocab" | "caseInsensitive" => GrammarOptionDisposition::Metadata,
                "language" => GrammarOptionDisposition::ToolHandled,
                _ if option_hooks.contains(&assignment) => GrammarOptionDisposition::Hooked,
                _ => GrammarOptionDisposition::Unsupported,
            };
            GrammarOptionEntry {
                key,
                value,
                line,
                column,
                disposition,
            }
        })
        .collect())
}
/// Reads the lexer ATN to locate serialized custom action coordinates.
pub(crate) fn lexer_custom_actions(data: &LexerCodegenData<'_>) -> Vec<(i32, i32)> {
    let atn = data.lexer_atn();
    atn.lexer_actions()
        .iter()
        .filter_map(|action| match action {
            LexerAction::Custom {
                rule_index,
                action_index,
            } => Some((*rule_index, *action_index)),
            _ => None,
        })
        .collect()
}

/// Reads the lexer ATN to locate semantic predicate coordinates.
pub(crate) fn lexer_predicate_transitions(data: &LexerCodegenData<'_>) -> Vec<(usize, usize)> {
    let atn = data.lexer_atn();
    let mut predicates = Vec::new();
    for state in atn.states() {
        for transition in &state.transitions {
            if let LexerTransition::Predicate {
                rule_index,
                pred_index,
                ..
            } = transition
            {
                predicates.push((*rule_index, *pred_index));
            }
        }
    }
    predicates
}

/// Reads the packed parser ATN to locate semantic predicate coordinates.
pub(crate) fn parser_predicate_transitions(data: &ParserCodegenData<'_>) -> Vec<(usize, usize)> {
    let atn = data.parser_atn();
    let mut predicates = Vec::new();
    for state in atn.states() {
        for transition in state.transitions() {
            if let ParserTransitionData::Predicate {
                rule_index,
                pred_index,
                ..
            } = transition.data()
            {
                predicates.push((rule_index, pred_index));
            }
        }
    }
    predicates
}

/// Reads the parser ATN to locate action-transition source states.
pub(crate) fn parser_action_states(data: &ParserCodegenData<'_>) -> Vec<usize> {
    let atn = data.parser_atn();
    let mut states = Vec::new();
    for state in atn.states() {
        if state
            .transitions()
            .iter()
            .any(|transition| matches!(transition.data(), ParserTransitionData::Action { .. }))
        {
            states.push(state.state_number());
        }
    }
    states
}

/// Reads parser ATN action coordinates keyed by source state.
pub(crate) fn parser_action_state_coordinates(
    data: &ParserCodegenData<'_>,
) -> BTreeMap<usize, (usize, Option<usize>)> {
    let atn = data.parser_atn();
    let mut states = BTreeMap::new();
    for state in atn.states() {
        for transition in state.transitions() {
            if let ParserTransitionData::Action {
                rule_index,
                action_index,
                ..
            } = transition.data()
            {
                states.insert(state.state_number(), (rule_index, action_index));
            }
        }
    }
    states
}

/// The parser ATN action states that ANTLR *synthesized* (during left-recursion
/// elimination and similar rewrites) rather than the author writing them.
///
/// Provenance marks transformed action elements directly. Any action transition
/// not represented by a structural element is synthetic as well.
pub(crate) fn synthetic_parser_action_states(
    data: &ParserCodegenData<'_>,
) -> io::Result<BTreeSet<usize>> {
    let actions = structural_actions(data)?;
    let represented = actions
        .iter()
        .map(|action| action.state)
        .collect::<BTreeSet<_>>();
    let mut synthetic = actions
        .into_iter()
        .filter_map(|action| (!action.authored).then_some(action.state))
        .collect::<BTreeSet<_>>();
    synthetic.extend(
        parser_action_states(data)
            .into_iter()
            .filter(|state| !represented.contains(state)),
    );
    Ok(synthetic)
}

pub(crate) fn empty_parser_action_states(
    data: &ParserCodegenData<'_>,
) -> io::Result<BTreeSet<usize>> {
    Ok(structural_actions(data)?
        .into_iter()
        .filter_map(|action| {
            (action.authored && action.body.trim().is_empty()).then_some(action.state)
        })
        .collect())
}

pub(crate) fn structural_parser_rule_args(
    data: &RecognizerCodegenData<'_>,
) -> io::Result<Vec<(usize, usize, RuleArgTemplate)>> {
    let mut args = Vec::new();
    for call in structural_rule_calls(data)? {
        let Some(value) = call.arguments.as_deref() else {
            continue;
        };
        let Some(template) = parse_rule_arg_template(value, call.caller_first_argument.as_deref())
        else {
            let rule_name = data
                .rule_names
                .get(call.target_rule_index)
                .map_or("<unknown>", String::as_str);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported parser rule argument expression `{}` for rule `{rule_name}`; use an integer/boolean literal or forward the caller's first declared argument",
                    value.trim()
                ),
            ));
        };
        args.push((call.state, call.target_rule_index, template));
    }
    Ok(args)
}

pub(crate) fn structural_parameterized_parser_rules(
    data: &RecognizerCodegenData<'_>,
) -> io::Result<BTreeSet<usize>> {
    let semantic = data.semantic.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "structural grammar model is unavailable",
        )
    })?;
    Ok(semantic
        .recognizer
        .rule_numbers
        .iter()
        .filter_map(|(rule, rule_index)| {
            semantic
                .bindings
                .attributes
                .get(rule)
                .is_some_and(|attributes| !attributes.arguments.is_empty())
                .then_some(*rule_index)
        })
        .collect())
}

fn parse_rule_arg_template(
    value: &str,
    caller_first_argument: Option<&str>,
) -> Option<RuleArgTemplate> {
    let value = value.trim();
    value.parse::<i64>().map_or_else(
        |_| {
            if matches!(value, "true" | "false") {
                Some(RuleArgTemplate::Literal(i64::from(value == "true")))
            } else if caller_first_argument.is_some_and(|argument| {
                value == argument
                    || value.strip_prefix('$') == Some(argument)
                    || value == format!(r#"<VarRef("{argument}")>"#)
            }) {
                Some(RuleArgTemplate::InheritLocal)
            } else {
                None
            }
        },
        |value| Some(RuleArgTemplate::Literal(value)),
    )
}
