use antlr_rust_toml_parser::{Item as TomlItem, Value as TomlValue};

impl SemPatternFile {
    pub(crate) fn predicate_template(
        &self,
        kind: SemanticsKind,
        body: &str,
    ) -> io::Result<Option<PredicateTemplate>> {
        let body = body.trim();
        let mut matches = Vec::new();
        matches.extend(
            self.patterns
                .iter()
                .filter(|pattern| pattern.match_body.trim() == body)
                .map(|pattern| (pattern.id.as_str(), pattern.lower.as_str())),
        );
        matches.extend(
            self.helpers
                .iter()
                .filter(|helper| semantic_helper_kind_matches(helper, kind))
                .filter_map(|helper| {
                    parse_semantic_helper_call(body, kind, helper.receiver.as_deref())
                        .filter(|call| helper_call_matches(call, helper))
                        .map(|_| helper)
                })
                .map(|helper| (helper.name.as_str(), helper.lower.as_str())),
        );
        if matches.len() > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "ambiguous semantic patterns for {body:?}: {}",
                    matches
                        .iter()
                        .map(|(id, _)| *id)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        let Some((_, lower)) = matches.first() else {
            return Ok(None);
        };
        // Member lowerings resolve slot names against the `[[member]]`
        // inventory for *this* recognizer, so they are tried before the
        // slot-free built-in shapes.
        let slots = self.member_slots_for(member_scope_for_kind(kind))?;
        if let Some(expr) = stack_member::parse_member_expr(lower, &slots) {
            return Ok(Some(PredicateTemplate::MemberExpr(expr?)));
        }
        parse_pattern_lower(lower).map(Some)
    }

    /// Resolves an inline lexer-action body to a lowered member statement.
    ///
    /// `None` means no `[[pattern]]` entry matched this body, leaving the
    /// caller's existing helper-hook and unsupported-action handling in place.
    ///
    /// Two entries matching one body is an error, not a first-wins pick: they
    /// lower to different mutations, so silently taking the first would make
    /// merely reordering the pattern file change runtime behavior. This mirrors
    /// [`Self::predicate_template`]'s ambiguity check.
    pub(crate) fn member_action_stmt(
        &self,
        kind: SemanticsKind,
        body: &str,
    ) -> io::Result<Option<stack_member::MemberStmt>> {
        let body = normalize_action_match_body(body);
        let matches = self
            .patterns
            .iter()
            .filter(|pattern| normalize_action_match_body(&pattern.match_body) == body)
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "ambiguous semantic patterns for {body:?}: {}",
                    matches
                        .iter()
                        .map(|pattern| pattern.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        let Some(pattern) = matches.first() else {
            return Ok(None);
        };
        let slots = self.member_slots_for(member_scope_for_kind(kind))?;
        stack_member::parse_member_stmt(&pattern.lower, &slots).transpose()
    }

    pub(crate) fn hook_helper_call(
        &self,
        kind: SemanticsKind,
        body: &str,
    ) -> io::Result<Option<SemanticHelperCall>> {
        let matches = self
            .helpers
            .iter()
            .filter(|helper| {
                semantic_helper_kind_matches(helper, kind) && helper.lower.trim() == "hook"
            })
            .filter_map(|helper| {
                parse_semantic_helper_call(body, kind, helper.receiver.as_deref())
                    .filter(|call| helper_call_matches(call, helper))
                    .map(|call| (helper, call))
            })
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "ambiguous semantic helper patterns for {body:?}: {}",
                    matches
                        .iter()
                        .map(|(helper, _)| helper.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        Ok(matches.into_iter().next().map(|(_, call)| call))
    }

    pub(crate) fn coordinate_disposition(
        &self,
        kind: SemanticsKind,
        rule: Option<&str>,
        index: Option<usize>,
        atn_state: Option<usize>,
    ) -> Option<SemanticsDisposition> {
        self.coordinate_override(kind, rule, index, atn_state)
            .map(|override_| override_.dispose.disposition())
    }

    #[allow(clippy::option_option)]
    pub(crate) fn coordinate_predicate_template(
        &self,
        kind: SemanticsKind,
        rule: Option<&str>,
        index: Option<usize>,
    ) -> Option<Option<PredicateTemplate>> {
        self.coordinate_override(kind, rule, index, None)
            .map(|override_| override_.dispose.predicate_template())
    }

    pub(crate) fn coordinate_override(
        &self,
        kind: SemanticsKind,
        rule: Option<&str>,
        index: Option<usize>,
        atn_state: Option<usize>,
    ) -> Option<&SemCoordinateOverride> {
        self.coordinates.iter().find(|override_| {
            override_.kind == kind
                && override_
                    .rule
                    .as_deref()
                    .is_none_or(|expected| rule == Some(expected))
                && override_
                    .index
                    .is_none_or(|expected| index == Some(expected))
                && override_
                    .atn_state
                    .is_none_or(|expected| atn_state == Some(expected))
        })
    }
}

/// Which member inventory a coordinate kind reads.
///
/// A combined grammar's lexer and parser members are independent, so each
/// recognizer's coordinates resolve slot names against its own inventory.
const fn member_scope_for_kind(kind: SemanticsKind) -> stack_member::MemberScope {
    match kind {
        SemanticsKind::LexerPredicate | SemanticsKind::LexerAction => {
            stack_member::MemberScope::Lexer
        }
        SemanticsKind::ParserPredicate | SemanticsKind::ParserAction => {
            stack_member::MemberScope::Parser
        }
    }
}

/// Canonical form of an action `match` body.
///
/// Actions carry the grammar's statement terminator, so one optional trailing
/// `;` is not part of the body — this matches what
/// [`parse_semantic_helper_call`] has always done for action bodies, so the two
/// matchers agree on whether a pattern applies.
///
/// Exactly one `;` is removed, not a run of them: stripping repeatedly would
/// collapse `x;` and `x;;` onto one body, merging two distinct declared
/// patterns (or manufacturing an ambiguity error between them). Any other
/// spelling difference stays significant, so matching remains whole-body.
fn normalize_action_match_body(body: &str) -> &str {
    let body = body.trim();
    body.strip_suffix(';').unwrap_or(body).trim_end()
}

fn semantic_helper_kind_matches(helper: &SemHelperRule, kind: SemanticsKind) -> bool {
    helper.kind == Some(kind)
        || (helper.kind.is_none()
            && matches!(
                kind,
                SemanticsKind::LexerPredicate | SemanticsKind::ParserPredicate
            ))
}

fn helper_call_matches(call: &SemanticHelperCall, helper: &SemHelperRule) -> bool {
    call.name == helper.name
        && call.arguments.len() == helper.arguments.len()
        && call
            .arguments
            .iter()
            .zip(&helper.arguments)
            .all(|(literal, kind)| {
                matches!(
                    (literal, kind),
                    (SemanticLiteral::String(_), SemanticLiteralKind::String)
                        | (SemanticLiteral::Bool(_), SemanticLiteralKind::Bool)
                        | (SemanticLiteral::Integer(_), SemanticLiteralKind::Integer)
                )
            })
}

pub(crate) fn parse_semantic_helper_call(
    body: &str,
    kind: SemanticsKind,
    receiver: Option<&str>,
) -> Option<SemanticHelperCall> {
    let mut body = body.trim();
    if matches!(
        kind,
        SemanticsKind::LexerAction | SemanticsKind::ParserAction
    ) {
        body = body.strip_suffix(';').unwrap_or(body).trim_end();
    }
    let negated = matches!(
        kind,
        SemanticsKind::LexerPredicate | SemanticsKind::ParserPredicate
    ) && body.starts_with('!');
    if negated {
        body = body[1..].trim_start();
    }
    body = body
        .strip_prefix("this.")
        .or_else(|| body.strip_prefix("self."))
        .or_else(|| receiver.and_then(|receiver| body.strip_prefix(receiver)?.strip_prefix('.')))
        .unwrap_or(body);
    let open = body.find('(')?;
    let name = body[..open].trim();
    if !is_semantic_helper_identifier(name) || !body.ends_with(')') {
        return None;
    }
    let arguments = parse_semantic_literals(&body[open + 1..body.len() - 1])?;
    Some(SemanticHelperCall {
        name: name.to_owned(),
        arguments,
        negated,
    })
}

fn is_semantic_helper_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|first| {
        (first == '_' || first.is_ascii_alphabetic())
            && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    })
}

fn parse_semantic_literals(body: &str) -> Option<Vec<SemanticLiteral>> {
    let mut body = body.trim();
    if body.is_empty() {
        return Some(Vec::new());
    }
    let mut literals = Vec::new();
    while !body.is_empty() {
        if body.starts_with('"') || body.starts_with('\'') {
            let quote = *body.as_bytes().first()?;
            let mut escaped = false;
            let mut end = None;
            for (index, byte) in body.as_bytes().iter().copied().enumerate().skip(1) {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == quote {
                    end = Some(index);
                    break;
                }
            }
            let end = end?;
            let raw = &body[1..end];
            let value = unescape_semantic_string(raw)?;
            literals.push(SemanticLiteral::String(value));
            body = body[end + 1..].trim_start();
        } else {
            let end = body.find(',').unwrap_or(body.len());
            let raw = body[..end].trim();
            let literal = match raw {
                "true" => SemanticLiteral::Bool(true),
                "false" => SemanticLiteral::Bool(false),
                _ => SemanticLiteral::Integer(raw.parse().ok()?),
            };
            literals.push(literal);
            body = body[end..].trim_start();
        }
        if body.is_empty() {
            break;
        }
        body = body.strip_prefix(',')?.trim_start();
        if body.is_empty() {
            return None;
        }
    }
    Some(literals)
}

fn unescape_semantic_string(value: &str) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next()? {
            '"' => out.push('"'),
            '\'' => out.push('\''),
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Some(out)
}

fn parse_pattern_lower(lower: &str) -> io::Result<PredicateTemplate> {
    let lower = lower.trim();
    match lower {
        "true" | "bool(true)" => return Ok(PredicateTemplate::True),
        "false" | "bool(false)" => return Ok(PredicateTemplate::False),
        "hook" => return Ok(PredicateTemplate::Hook),
        "token_index_adjacent" => return Ok(PredicateTemplate::TokenPairAdjacent),
        _ => {}
    }
    parse_pattern_lt_text(lower)
        .or_else(|| parse_pattern_la_not_equals(lower))
        .or_else(|| parse_pattern_ctx_rule_text_not_equals(lower))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported semantic pattern lower expression {lower:?}"),
            )
        })
}

fn parse_pattern_lt_text(lower: &str) -> Option<PredicateTemplate> {
    let body = lower.strip_prefix("cmp(eq, token_text(")?;
    let (offset, rest) = body.split_once("), str(\"")?;
    let text = rest.strip_suffix("\"))")?;
    Some(PredicateTemplate::LookaheadTextEquals {
        offset: offset.trim().parse().ok()?,
        text: text.to_owned(),
    })
}

fn parse_pattern_la_not_equals(lower: &str) -> Option<PredicateTemplate> {
    let body = lower.strip_prefix("cmp(ne, la(")?;
    let (offset, rest) = body.split_once("), token(")?;
    let token_name = rest.strip_suffix("))")?;
    Some(PredicateTemplate::LookaheadNotEquals {
        offset: offset.trim().parse().ok()?,
        token_name: token_name.trim().to_owned(),
    })
}

fn parse_pattern_ctx_rule_text_not_equals(lower: &str) -> Option<PredicateTemplate> {
    let body = lower.strip_prefix("cmp(ne, ctx_rule_text(")?;
    let (rule_name, rest) = body.split_once("), str(\"")?;
    let text = rest.strip_suffix("\"))")?;
    Some(PredicateTemplate::ContextChildRuleTextNotEquals {
        rule_name: rule_name.trim().to_owned(),
        text: text.to_owned(),
    })
}

pub(crate) fn load_sem_patterns(path: &Path) -> io::Result<SemPatternFile> {
    parse_sem_patterns(&fs::read_to_string(path)?)
}

pub(crate) fn parse_sem_patterns(input: &str) -> io::Result<SemPatternFile> {
    let document =
        antlr_rust_toml_parser::parse(input).map_err(|error| invalid_toml(&error))?;
    let mut file = SemPatternFile::default();
    let mut section: Option<PatternSection> = None;
    let mut root_fields = BTreeMap::<String, TomlValue>::new();
    let mut fields = BTreeMap::<String, TomlValue>::new();
    for item in document.into_items() {
        match item {
            TomlItem::Assignment(assignment) => {
                let (key, value) = assignment.into_parts();
                let name = single_schema_key(&key)?;
                let target = if section.is_some() {
                    &mut fields
                } else {
                    &mut root_fields
                };
                if target.insert(name.clone(), value).is_some() {
                    return Err(invalid_semantic_pattern(format!(
                        "duplicate semantic pattern field {name:?}"
                    )));
                }
            }
            TomlItem::Table(header) => {
                flush_pattern_section(&mut file, section.take(), &mut fields)?;
                if !header.is_array() {
                    return Err(invalid_semantic_pattern(
                        "semantic pattern sections must use TOML array tables",
                    ));
                }
                section = Some(parse_pattern_section(&single_schema_key(header.key())?)?);
            }
        }
    }
    flush_pattern_section(&mut file, section, &mut fields)?;
    validate_root_fields(&mut root_fields)?;
    Ok(file)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PatternSection {
    Pattern,
    Helper,
    Coordinate,
    Member,
}

impl PatternSection {
    const fn name(self) -> &'static str {
        match self {
            Self::Pattern => "pattern",
            Self::Helper => "helper",
            Self::Coordinate => "coordinate",
            Self::Member => "member",
        }
    }
}

fn parse_pattern_section(name: &str) -> io::Result<PatternSection> {
    match name {
        "pattern" => Ok(PatternSection::Pattern),
        "helper" => Ok(PatternSection::Helper),
        "coordinate" => Ok(PatternSection::Coordinate),
        "member" => Ok(PatternSection::Member),
        _ => Err(invalid_semantic_pattern(format!(
            "unknown semantic pattern section {name:?}"
        ))),
    }
}

fn flush_pattern_section(
    file: &mut SemPatternFile,
    section: Option<PatternSection>,
    fields: &mut BTreeMap<String, TomlValue>,
) -> io::Result<()> {
    let Some(section) = section else {
        return Ok(());
    };
    match section {
        PatternSection::Pattern => {
            let match_body = take_required_string_field(fields, "match")?;
            let lower = take_required_string_field(fields, "lower")?;
            let id = fields
                .remove("id")
                .map(|value| expect_string_field("id", value))
                .transpose()?
                .unwrap_or_else(|| format!("pattern:{}", file.patterns.len()));
            file.patterns.push(SemPatternRule {
                id,
                match_body,
                lower,
            });
        }
        PatternSection::Helper => {
            let kind = fields
                .remove("kind")
                .map(|value| expect_string_field("kind", value))
                .transpose()?
                .map(|value| parse_coordinate_kind(&value))
                .transpose()?;
            let receiver = fields
                .remove("receiver")
                .map(|value| expect_string_field("receiver", value))
                .transpose()?
                .map(|receiver| {
                    if is_semantic_helper_identifier(&receiver) {
                        Ok(receiver)
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "semantic helper receiver must be an identifier, got {receiver:?}"
                            ),
                        ))
                    }
                })
                .transpose()?;
            let arguments = fields
                .remove("arguments")
                .map(|value| expect_string_field("arguments", value))
                .transpose()?
                .map_or_else(|| Ok(Vec::new()), |value| parse_helper_arguments(&value))?;
            // `returns` is documentation in existing pattern files. Validate
            // its shape when present, but the semantic kind determines whether
            // the generated method returns bool or unit.
            if let Some(returns) = fields
                .remove("returns")
                .map(|value| expect_string_field("returns", value))
                .transpose()?
            {
                let expected = if kind.is_none_or(|kind| {
                    matches!(
                        kind,
                        SemanticsKind::LexerPredicate | SemanticsKind::ParserPredicate
                    )
                }) {
                    "bool"
                } else {
                    "unit"
                };
                if returns != expected {
                    let kind = kind.map_or("predicate", SemanticsKind::manifest_name);
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "semantic helper kind {kind} requires returns = {expected:?}, got {returns:?}"
                        ),
                    ));
                }
            }
            file.helpers.push(SemHelperRule {
                kind,
                receiver,
                name: take_required_string_field(fields, "name")?,
                arguments,
                lower: take_required_string_field(fields, "lower")?,
            });
        }
        PatternSection::Member => {
            file.members.push(stack_member::MemberDeclaration {
                name: take_required_string_field(fields, "name")?,
                kind: stack_member::MemberKind::parse(&take_required_string_field(
                    fields, "kind",
                )?)?,
                // Defaults to `both`, so single-recognizer grammars (and every
                // pattern file written before scoping existed) need no `scope`.
                scope: fields
                    .remove("scope")
                    .map(|value| expect_string_field("scope", value))
                    .transpose()?
                    .map_or(Ok(stack_member::MemberScope::Both), |value| {
                        stack_member::MemberScope::parse(&value)
                    })?,
                // A grammar's declared initializer (`bool verbatium = true;`)
                // is metadata here rather than something parsed out of the
                // host-language declaration.
                init: fields
                    .remove("init")
                    .map(parse_member_init_field)
                    .transpose()?,
            });
        }
        PatternSection::Coordinate => {
            file.coordinates.push(SemCoordinateOverride {
                kind: parse_coordinate_kind(&take_required_string_field(fields, "kind")?)?,
                rule: fields
                    .remove("rule")
                    .map(|value| expect_string_field("rule", value))
                    .transpose()?,
                index: fields
                    .remove("index")
                    .map(|value| parse_usize_field("index", &value))
                    .transpose()?,
                atn_state: fields
                    .remove("atn_state")
                    .map(|value| parse_usize_field("atn_state", &value))
                    .transpose()?,
                dispose: CoordinateDispose::parse(&take_required_string_field(
                    fields, "dispose",
                )?)?,
            });
        }
    }
    if let Some(name) = fields.keys().next() {
        return Err(invalid_semantic_pattern(format!(
            "unknown field {name:?} in [[{}]]",
            section.name()
        )));
    }
    fields.clear();
    Ok(())
}

fn parse_helper_arguments(value: &str) -> io::Result<Vec<SemanticLiteralKind>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|part| match part.trim() {
            "string" | "str" => Ok(SemanticLiteralKind::String),
            "bool" | "boolean" => Ok(SemanticLiteralKind::Bool),
            "int" | "integer" => Ok(SemanticLiteralKind::Integer),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown semantic helper argument kind {other:?}"),
            )),
        })
        .collect()
}

fn take_required_string_field(
    fields: &mut BTreeMap<String, TomlValue>,
    name: &str,
) -> io::Result<String> {
    let value = fields.remove(name).ok_or_else(|| {
        invalid_semantic_pattern(format!("semantic pattern section missing {name}"))
    })?;
    expect_string_field(name, value)
}

fn expect_string_field(name: &str, value: TomlValue) -> io::Result<String> {
    match value {
        TomlValue::String(value) => Ok(value),
        other => Err(invalid_semantic_pattern(format!(
            "semantic pattern field {name:?} must be a string, got {}",
            toml_value_kind(&other)
        ))),
    }
}

fn parse_member_init_field(value: TomlValue) -> io::Result<i64> {
    match value {
        TomlValue::Boolean(value) => Ok(i64::from(value)),
        TomlValue::Integer(value) => Ok(value),
        other => Err(invalid_semantic_pattern(format!(
            "member init must be an integer or boolean, got {}",
            toml_value_kind(&other)
        ))),
    }
}

fn parse_usize_field(name: &str, value: &TomlValue) -> io::Result<usize> {
    let TomlValue::Integer(value) = value else {
        return Err(invalid_semantic_pattern(format!(
            "{name} must be an integer, got {}",
            toml_value_kind(value)
        )));
    };
    usize::try_from(*value).map_err(|error| {
        invalid_semantic_pattern(format!("invalid {name} value {value}: {error}"))
    })
}

fn validate_root_fields(fields: &mut BTreeMap<String, TomlValue>) -> io::Result<()> {
    if let Some(version) = fields.remove("version") {
        if version != TomlValue::Integer(1) {
            return Err(invalid_semantic_pattern(
                "semantic pattern version must be the integer 1",
            ));
        }
    }
    if let Some(name) = fields.keys().next() {
        return Err(invalid_semantic_pattern(format!(
            "unknown top-level semantic pattern field {name:?}"
        )));
    }
    Ok(())
}

fn single_schema_key(key: &antlr_rust_toml_parser::Key) -> io::Result<String> {
    key.as_single().map(str::to_owned).ok_or_else(|| {
        invalid_semantic_pattern("semantic pattern schema does not accept dotted keys")
    })
}

const fn toml_value_kind(value: &TomlValue) -> &'static str {
    match value {
        TomlValue::String(_) => "string",
        TomlValue::Integer(_) => "integer",
        TomlValue::Float(_) => "float",
        TomlValue::Boolean(_) => "boolean",
        TomlValue::DateTime(_) => "date-time",
        TomlValue::Array(_) => "array",
        TomlValue::InlineTable(_) => "inline table",
    }
}

fn invalid_toml(error: &antlr_rust_toml_parser::Error) -> io::Error {
    invalid_semantic_pattern(error.to_string())
}

fn invalid_semantic_pattern(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn parse_coordinate_kind(value: &str) -> io::Result<SemanticsKind> {
    match value {
        "lexer-action" => Ok(SemanticsKind::LexerAction),
        "lexer-predicate" => Ok(SemanticsKind::LexerPredicate),
        "parser-predicate" | "predicate" => Ok(SemanticsKind::ParserPredicate),
        "parser-action" | "action" => Ok(SemanticsKind::ParserAction),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown semantic coordinate kind {other}"),
        )),
    }
}

pub(crate) fn reject_unsupported_lexer_action_templates(
    actions: &[ActionTemplate],
    allow_unsupported_only: bool,
) -> io::Result<()> {
    if let Some(ActionTemplate::UnsupportedLexerAction { rule_name, body }) = actions
        .iter()
        .find(|action| matches!(action, ActionTemplate::UnsupportedLexerAction { .. }))
    {
        let has_supported_dispatch = actions.iter().any(lexer_action_template_needs_dispatch);
        if allow_unsupported_only && !has_supported_dispatch {
            return Ok(());
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported embedded lexer action in rule {rule_name}: {{{body}}}; \
                 rewrite target-specific actions as portable lexer commands where possible"
            ),
        ));
    }
    Ok(())
}

fn parse_lexer_action_block_template(body: &str) -> Option<ActionTemplate> {
    parse_lexer_pop_mode_action(body)
}

pub(crate) fn parse_lexer_pop_mode_action(body: &str) -> Option<ActionTemplate> {
    let body = body
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace() && *ch != ';')
        .collect::<String>();
    matches!(
        body.as_str(),
        "popMode()"
            | "this.popMode()"
            | "if(!_modeStack.isEmpty()){popMode()}"
            | "if(!this._modeStack.isEmpty()){popMode()}"
            | "if(!_modeStack.isEmpty())popMode()"
            | "if(!this._modeStack.isEmpty())popMode()"
    )
    .then_some(ActionTemplate::LexerPopMode)
}

pub(crate) fn one_line_action_body(body: &str) -> String {
    const ACTION_SUMMARY_LIMIT: usize = 96;

    let mut out = String::new();
    for (index, part) in body.split_whitespace().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(part);
        if out.len() > ACTION_SUMMARY_LIMIT {
            let mut limit = ACTION_SUMMARY_LIMIT;
            while !out.is_char_boundary(limit) {
                limit -= 1;
            }
            out.truncate(limit);
            out.push_str("...");
            break;
        }
    }
    out
}

pub(crate) fn rust_block_comment_text(value: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    while let Some(relative_index) = value[cursor..].find("*/") {
        let index = cursor + relative_index;
        out.push_str(&value[cursor..index]);
        out.push_str("* /");
        cursor = index + 2;
    }
    if cursor == 0 {
        value.to_owned()
    } else {
        out.push_str(&value[cursor..]);
        out
    }
}

/// Attaches ANTLR's `<fail=...>` option to a predicate template so the runtime
/// can surface the grammar-supplied message when the predicate fails at runtime.
///
/// A constant-false predicate folds into `FalseWithMessage` (its own dedicated
/// variant). Any *other* template — a hook, lookahead, member, or local-int
/// predicate that can also return false at runtime — is wrapped in
/// `WithFailMessage` so the message is preserved rather than discarded; the
/// wrapper is transparent to evaluation (see `predicate_effective_template`) and
/// only contributes the failure message.
pub(crate) fn predicate_template_with_fail_message(
    template: PredicateTemplate,
    message: String,
) -> PredicateTemplate {
    match template {
        PredicateTemplate::False => PredicateTemplate::FalseWithMessage { message },
        // Already message-carrying: replace the message (a later `<fail=...>`
        // wins) rather than nesting wrappers.
        PredicateTemplate::FalseWithMessage { .. } => {
            PredicateTemplate::FalseWithMessage { message }
        }
        PredicateTemplate::WithFailMessage { inner, .. } => {
            PredicateTemplate::WithFailMessage { inner, message }
        }
        other => PredicateTemplate::WithFailMessage {
            inner: Box::new(other),
            message,
        },
    }
}

/// The evaluation-relevant template, unwrapping a `WithFailMessage` wrapper.
/// The wrapper only carries a failure message; its runtime truth value and
/// codegen come from the inner template.
pub(crate) fn predicate_effective_template(template: &PredicateTemplate) -> &PredicateTemplate {
    match template {
        PredicateTemplate::WithFailMessage { inner, .. } => inner,
        other => other,
    }
}

/// The grammar-supplied `<fail=...>` message a template carries, if any.
pub(crate) fn predicate_template_fail_message(template: &PredicateTemplate) -> Option<&str> {
    match template {
        PredicateTemplate::FalseWithMessage { message }
        | PredicateTemplate::UnknownWithFailMessage { message }
        | PredicateTemplate::WithFailMessage { message, .. } => Some(message),
        _ => None,
    }
}

/// Reports whether a predicate body is an untranslated ANTLR `<...>`
/// `StringTemplate` (a single template wrapper or a sequence of them), as
/// opposed to a native target-language predicate that merely contains a `<`
/// operator.
fn is_unsupported_string_template_body(body: &str) -> bool {
    single_template_body(body).is_some() || template_sequence_bodies(body).is_some()
}

pub(crate) fn uses_alt_number_contexts(data: &RecognizerCodegenData<'_>) -> bool {
    let Some(semantic) = data.semantic else {
        return false;
    };
    semantic
        .unit
        .options
        .iter()
        .any(|option| option.name.value == "contextSuperClass")
}

pub(crate) fn uses_lexer_superclass(data: &RecognizerCodegenData<'_>) -> bool {
    data.semantic.is_some_and(|semantic| {
        semantic
            .unit
            .options
            .iter()
            .any(|option| option.name.value == "superClass")
    })
}

pub(crate) fn uses_structural_context_alt_numbers(
    data: &RecognizerCodegenData<'_>,
) -> io::Result<bool> {
    if data.semantic.is_none() {
        return Ok(false);
    }
    let model = structural_embedded_model(data, false)?;
    Ok(model.rules.iter().any(|rule| {
        let left_recursive = rule
            .alts
            .iter()
            .any(|alternative| alternative.is_lr_operator(&rule.name));
        if !left_recursive {
            return rule
                .alts
                .iter()
                .map(|alternative| alternative.label.as_deref())
                .collect::<BTreeSet<_>>()
                .len()
                > 1;
        }

        [false, true].into_iter().any(|operator| {
            rule.alts
                .iter()
                .filter(|alternative| alternative.is_lr_operator(&rule.name) == operator)
                .map(|alternative| alternative.label.as_deref())
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        })
    }))
}

pub(crate) fn parse_predicate_template(body: &str) -> Option<PredicateTemplate> {
    let body = body.trim();
    if let Some(inner) = single_template_body(body) {
        return parse_predicate_template(inner);
    }
    match body {
        "True()" => Some(PredicateTemplate::True),
        "False()" => Some(PredicateTemplate::False),
        r#"ParserPropertyCall({$parser}, "Property()")"# => Some(PredicateTemplate::True),
        _ => parse_raw_boolean_predicate(body)
            .or_else(|| parse_text_equals_predicate(body))
            .or_else(|| parse_token_start_column_equals_predicate(body))
            .or_else(|| parse_column_compare_predicate(body))
            .or_else(|| parse_invoke_predicate(body))
            .or_else(|| parse_val_equals_predicate(body))
            .or_else(|| parse_raw_local_int_less_or_equal_predicate(body))
            .or_else(|| parse_lt_equals_predicate(body))
            .or_else(|| parse_la_not_equals_predicate(body)),
    }
}

fn parse_predicate_template_with_patterns(
    body: &str,
    patterns: &SemPatternFile,
) -> io::Result<Option<PredicateTemplate>> {
    parse_predicate_template_with_patterns_kind(body, patterns, SemanticsKind::ParserPredicate)
}

fn parse_predicate_template_with_patterns_kind(
    body: &str,
    patterns: &SemPatternFile,
    kind: SemanticsKind,
) -> io::Result<Option<PredicateTemplate>> {
    Ok(match parse_predicate_template(body) {
        Some(template) => Some(template),
        None => patterns.predicate_template(kind, body)?,
    })
}

fn parse_raw_boolean_predicate(body: &str) -> Option<PredicateTemplate> {
    match body {
        "true" => return Some(PredicateTemplate::True),
        "false" => return Some(PredicateTemplate::False),
        _ => {}
    }
    let (equals, left, right) = if let Some((left, right)) = body.split_once("==") {
        (true, left, right)
    } else {
        let (left, right) = body.split_once("!=")?;
        (false, left, right)
    };
    let left = left.trim().parse::<i64>().ok()?;
    let right = right.trim().parse::<i64>().ok()?;
    let value = if equals { left == right } else { left != right };
    Some(if value {
        PredicateTemplate::True
    } else {
        PredicateTemplate::False
    })
}

/// Returns the call body for an action made of exactly one target template.
fn single_template_body(body: &str) -> Option<&str> {
    let body = body.trim();
    if body.as_bytes().first() != Some(&b'<') {
        return None;
    }
    let close = matching_template_close(body, 1)?;
    (close + 1 == body.len()).then_some(&body[1..close])
}

/// Parses simple local integer argument predicates such as
/// `ValEquals("$i","2")`.
pub(crate) fn parse_val_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let arguments = body
        .strip_prefix("ValEquals(")
        .and_then(|value| value.strip_suffix(')'))
        .map(split_template_arguments)?;
    let [local, value] = arguments.as_slice() else {
        return None;
    };
    if parse_template_string(local)? != "$i" {
        return None;
    }
    Some(PredicateTemplate::LocalIntEquals {
        value: parse_template_string(value)?.parse::<i64>().ok()?,
    })
}

/// Parses raw ANTLR semantic predicates such as `5 >= $_p`.
///
/// The Java generator lowers these against the generated context field
/// `_localctx._p`. The metadata runtime does not execute target code, so the
/// generator records the literal bound and the rule-call argument table makes
/// the current `_p` value available while interpreting the predicate
/// transition.
pub(crate) fn parse_raw_local_int_less_or_equal_predicate(
    body: &str,
) -> Option<PredicateTemplate> {
    let (value, local) = body.split_once(">=")?;
    if local.trim() != "$_p" {
        return None;
    }
    Some(PredicateTemplate::LocalIntLessOrEqual {
        value: value.trim().parse::<i64>().ok()?,
    })
}

/// Parses the runtime-testsuite helper that prints when a predicate is
/// evaluated before returning the wrapped boolean value.
pub(crate) fn parse_invoke_predicate(body: &str) -> Option<PredicateTemplate> {
    let value = body.strip_suffix(":Invoke_pred()")?;
    match value {
        "True()" => Some(PredicateTemplate::Invoke { value: true }),
        "False()" => Some(PredicateTemplate::Invoke { value: false }),
        r#"ValEquals("$i","99")"# => Some(PredicateTemplate::Invoke { value: true }),
        _ => None,
    }
}

fn parse_text_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let argument = body
        .strip_prefix("TextEquals(")
        .and_then(|value| value.strip_suffix(')'))?;
    Some(PredicateTemplate::TextEquals(parse_template_string(
        argument,
    )?))
}

fn parse_token_start_column_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let argument = body
        .strip_prefix("TokenStartColumnEquals(")
        .and_then(|value| value.strip_suffix(')'))?;
    Some(PredicateTemplate::TokenStartColumnEquals(
        parse_template_string(argument)?.parse().ok()?,
    ))
}

/// Parses lexer column predicates serialized by upstream templates as
/// `<Column()> \< 2` or `<Column()> >= 2`.
fn parse_column_compare_predicate(body: &str) -> Option<PredicateTemplate> {
    let rest = body
        .trim()
        .strip_prefix("<Column()>")
        .or_else(|| body.trim().strip_prefix("Column()"))?
        .trim_start();
    let rest = rest.strip_prefix('\\').unwrap_or(rest).trim_start();
    if let Some(value) = rest.strip_prefix('<') {
        return Some(PredicateTemplate::ColumnLessThan(
            value.trim().parse().ok()?,
        ));
    }
    Some(PredicateTemplate::ColumnGreaterOrEqual(
        rest.strip_prefix(">=")?.trim().parse().ok()?,
    ))
}

fn parse_la_not_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let arguments = body
        .strip_prefix("LANotEquals(")
        .and_then(|value| value.strip_suffix(')'))
        .map(split_template_arguments)?;
    let [offset, token] = arguments.as_slice() else {
        return None;
    };
    let offset = parse_template_string(offset)?.parse::<isize>().ok()?;
    let token_name = parse_parser_token_argument(token)?;
    Some(PredicateTemplate::LookaheadNotEquals { offset, token_name })
}

/// Parses `LTEquals` predicates that compare lookahead token text.
///
/// The runtime-testsuite passes the expected text as a quoted target-language
/// string literal, so the decoded `StringTemplate` argument may still contain
/// one nested quote pair.
fn parse_lt_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let arguments = body
        .strip_prefix("LTEquals(")
        .and_then(|value| value.strip_suffix(')'))
        .map(split_template_arguments)?;
    let [offset, text] = arguments.as_slice() else {
        return None;
    };
    let offset = parse_template_string(offset)?.parse::<isize>().ok()?;
    let text = parse_template_string(text)?;
    let text = text
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(&text)
        .to_owned();
    Some(PredicateTemplate::LookaheadTextEquals { offset, text })
}

fn parse_parser_token_argument(argument: &str) -> Option<String> {
    let body = argument
        .trim()
        .strip_prefix("{T<ParserToken(")?
        .strip_suffix(")>}")?;
    let parts = split_template_arguments(body);
    let [_, token_name] = parts.as_slice() else {
        return None;
    };
    parse_template_string(token_name)
}
