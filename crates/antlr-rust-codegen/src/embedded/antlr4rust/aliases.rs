use super::{Range, RustDelimiterMap, RustLexeme, RustLexemeKind, RustReplacement, io};

pub(crate) fn recog_input_replacement(
    body: &str,
    lexemes: &[RustLexeme],
    position: usize,
    input_facade: &str,
) -> io::Result<RustReplacement> {
    let Some(dot) = next_significant(lexemes, position) else {
        return Err(unsupported_antlr4rust("unsupported bare `recog` reference"));
    };
    if lexemes[dot].kind != RustLexemeKind::Punctuation(b'.') {
        return Err(unsupported_antlr4rust("unsupported bare `recog` reference"));
    }
    let Some(member) = next_significant(lexemes, dot) else {
        return Err(unsupported_antlr4rust("incomplete `recog` member access"));
    };
    if lexeme_text(body, lexemes[member]) != "input" {
        return Err(unsupported_antlr4rust(&format!(
            "unsupported `recog.{}` member",
            lexeme_text(body, lexemes[member])
        )));
    }
    let accessor = required_member_call(body, lexemes, member, "recog.input")?;
    let accessor_name = lexeme_text(body, lexemes[accessor]);
    if !matches!(accessor_name, "la" | "lt") {
        return Err(unsupported_antlr4rust(&format!(
            "unsupported `recog.input.{accessor_name}` accessor"
        )));
    }
    require_nonempty_call_argument(body, lexemes, accessor, "recog.input")?;
    Ok(RustReplacement {
        range: lexemes[position].start..lexemes[member].end,
        text: format!("{input_facade}(self.base.token_stream())"),
    })
}

pub(crate) fn local_context_replacement(
    body: &str,
    lexemes: &[RustLexeme],
    position: usize,
    local_context_expression: &str,
) -> io::Result<RustReplacement> {
    let method = required_member_call(body, lexemes, position, "_localctx")?;
    let method_name = lexeme_text(body, lexemes[method]);
    if method_name != "as_deref" {
        return Err(unsupported_antlr4rust(&format!(
            "unsupported `_localctx.{method_name}` accessor"
        )));
    }
    let close = require_empty_call_arguments(body, lexemes, method, "_localctx")?;
    Ok(RustReplacement {
        range: lexemes[position].start..lexemes[close].end,
        text: local_context_expression.to_owned(),
    })
}

pub(crate) fn required_member_call(
    body: &str,
    lexemes: &[RustLexeme],
    receiver: usize,
    display_receiver: &str,
) -> io::Result<usize> {
    let dot = next_significant(lexemes, receiver)
        .filter(|&index| lexemes[index].kind == RustLexemeKind::Punctuation(b'.'))
        .ok_or_else(|| {
            unsupported_antlr4rust(&format!("unsupported `{display_receiver}` shape"))
        })?;
    let method = next_significant(lexemes, dot)
        .filter(|&index| lexemes[index].kind == RustLexemeKind::Identifier)
        .ok_or_else(|| {
            unsupported_antlr4rust(&format!("unsupported `{display_receiver}` shape"))
        })?;
    next_significant(lexemes, method)
        .filter(|&index| lexemes[index].kind == RustLexemeKind::Punctuation(b'('))
        .ok_or_else(|| {
            unsupported_antlr4rust(&format!(
                "`{display_receiver}.{}` must be a method call",
                lexeme_text(body, lexemes[method])
            ))
        })?;
    Ok(method)
}

pub(crate) fn require_empty_call_arguments(
    body: &str,
    lexemes: &[RustLexeme],
    method: usize,
    display_receiver: &str,
) -> io::Result<usize> {
    let method_name = lexeme_text(body, lexemes[method]);
    let open = next_significant(lexemes, method)
        .filter(|&index| lexemes[index].kind == RustLexemeKind::Punctuation(b'('))
        .expect("required_member_call already checked the opening parenthesis");
    next_significant(lexemes, open)
        .filter(|&index| lexemes[index].kind == RustLexemeKind::Punctuation(b')'))
        .ok_or_else(|| {
            unsupported_antlr4rust(&format!(
                "`{display_receiver}.{method_name}()` does not accept arguments"
            ))
        })
}

pub(crate) fn require_nonempty_call_argument(
    body: &str,
    lexemes: &[RustLexeme],
    method: usize,
    display_receiver: &str,
) -> io::Result<()> {
    let method_name = lexeme_text(body, lexemes[method]);
    let open = next_significant(lexemes, method)
        .filter(|&index| lexemes[index].kind == RustLexemeKind::Punctuation(b'('))
        .expect("required_member_call already checked the opening parenthesis");
    let first = next_significant(lexemes, open).ok_or_else(|| {
        unsupported_antlr4rust(&format!(
            "unterminated `{display_receiver}.{method_name}` call"
        ))
    })?;
    if lexemes[first].kind == RustLexemeKind::Punctuation(b')') {
        return Err(unsupported_antlr4rust(&format!(
            "`{display_receiver}.{method_name}` requires one offset argument"
        )));
    }

    let mut delimiters = Vec::new();
    for (position, lexeme) in lexemes.iter().enumerate().skip(open + 1) {
        match lexeme.kind {
            RustLexemeKind::Punctuation(b'(') => delimiters.push(b')'),
            RustLexemeKind::Punctuation(b'[') => delimiters.push(b']'),
            RustLexemeKind::Punctuation(b'{') => delimiters.push(b'}'),
            RustLexemeKind::Punctuation(b'<')
                if is_turbofish_open(lexemes, position)
                    || delimiters.last().is_some_and(|close| *close == b'>') =>
            {
                delimiters.push(b'>');
            }
            RustLexemeKind::Punctuation(b'>')
                if delimiters.last().is_some_and(|close| *close == b'>') =>
            {
                delimiters.pop();
            }
            RustLexemeKind::Punctuation(close @ (b')' | b']' | b'}')) => {
                if let Some(expected) = delimiters.pop() {
                    if close != expected {
                        return Err(unsupported_antlr4rust(&format!(
                            "unterminated `{display_receiver}.{method_name}` call"
                        )));
                    }
                } else if close == b')' {
                    return Ok(());
                } else {
                    return Err(unsupported_antlr4rust(&format!(
                        "unterminated `{display_receiver}.{method_name}` call"
                    )));
                }
            }
            RustLexemeKind::Punctuation(b',') if delimiters.is_empty() => {
                let trailing = next_significant(lexemes, position)
                    .is_some_and(|next| lexemes[next].kind == RustLexemeKind::Punctuation(b')'));
                if !trailing {
                    return Err(unsupported_antlr4rust(&format!(
                        "`{display_receiver}.{method_name}` accepts exactly one offset argument"
                    )));
                }
            }
            _ => {}
        }
    }
    Err(unsupported_antlr4rust(&format!(
        "unterminated `{display_receiver}.{method_name}` call"
    )))
}

pub(crate) fn is_turbofish_open(lexemes: &[RustLexeme], position: usize) -> bool {
    previous_significant(lexemes, position).is_some_and(|previous| {
        lexemes[previous].kind == RustLexemeKind::Punctuation(b':')
            && previous_significant(lexemes, previous)
                .is_some_and(|before| lexemes[before].kind == RustLexemeKind::Punctuation(b':'))
    })
}

pub(crate) fn qualified_path_start(lexemes: &[RustLexeme], position: usize) -> usize {
    let mut start = position;
    while let Some(second_colon) = previous_significant(lexemes, start)
        && lexemes[second_colon].kind == RustLexemeKind::Punctuation(b':')
        && let Some(first_colon) = previous_significant(lexemes, second_colon)
        && lexemes[first_colon].kind == RustLexemeKind::Punctuation(b':')
    {
        let Some(segment) = previous_significant(lexemes, first_colon) else {
            return first_colon;
        };
        if lexemes[segment].kind != RustLexemeKind::Identifier {
            break;
        }
        start = segment;
    }
    start
}

pub(crate) fn opaque_macro_invocation_range(
    lexemes: &[RustLexeme],
    identifier: usize,
    delimiters: &RustDelimiterMap,
) -> Option<Range<usize>> {
    let (bang, close) = (0..identifier).rev().find_map(|candidate| {
        if lexemes[candidate].kind != RustLexemeKind::Punctuation(b'!') {
            return None;
        }
        let open = next_significant(lexemes, candidate)?;
        if !matches!(
            lexemes[open].kind,
            RustLexemeKind::Punctuation(b'(' | b'[' | b'{')
        ) {
            return None;
        }
        let close = delimiters.pairs[open]?;
        (identifier < close).then_some((candidate, close))
    })?;
    let mut start = previous_significant(lexemes, bang)?;
    if lexemes[start].kind != RustLexemeKind::Identifier {
        return None;
    }
    while let Some(second_colon) = previous_significant(lexemes, start) {
        let Some(first_colon) = previous_significant(lexemes, second_colon) else {
            break;
        };
        let Some(segment) = previous_significant(lexemes, first_colon) else {
            break;
        };
        if lexemes[second_colon].kind != RustLexemeKind::Punctuation(b':')
            || lexemes[first_colon].kind != RustLexemeKind::Punctuation(b':')
            || lexemes[segment].kind != RustLexemeKind::Identifier
        {
            break;
        }
        start = segment;
    }
    Some(lexemes[start].start..lexemes[close].end)
}

pub(crate) fn unsupported_antlr4rust(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("antlr4rust compatibility lowering: {message}"),
    )
}

pub(crate) fn is_standalone_identifier(lexemes: &[RustLexeme], position: usize) -> bool {
    let Some(previous) = previous_significant(lexemes, position) else {
        return true;
    };
    match lexemes[previous].kind {
        RustLexemeKind::Punctuation(b'.') => false,
        RustLexemeKind::Punctuation(b':') => previous_significant(lexemes, previous)
            .is_none_or(|before| lexemes[before].kind != RustLexemeKind::Punctuation(b':')),
        _ => true,
    }
}

pub(crate) fn is_unqualified_identifier(lexemes: &[RustLexeme], position: usize) -> bool {
    is_standalone_identifier(lexemes, position)
        && next_significant(lexemes, position).is_none_or(|next| {
            lexemes[next].kind != RustLexemeKind::Punctuation(b':')
                || next_significant(lexemes, next)
                    .is_none_or(|after| lexemes[after].kind != RustLexemeKind::Punctuation(b':'))
        })
}

#[derive(Clone, Copy)]
pub(crate) enum RelativeAliasModulePath {
    SelfModule,
    Super(usize),
}

impl RelativeAliasModulePath {
    pub(crate) const fn targets_generated_module(self, inline_module_depth: usize) -> bool {
        match self {
            Self::SelfModule => inline_module_depth == 0,
            Self::Super(levels) => levels == inline_module_depth && levels > 0,
        }
    }
}

pub(crate) fn relative_alias_module_path(
    body: &str,
    lexemes: &[RustLexeme],
    position: usize,
) -> Option<RelativeAliasModulePath> {
    if next_significant(lexemes, position).is_some_and(|next| {
        lexemes[next].kind == RustLexemeKind::Punctuation(b':')
            && next_significant(lexemes, next)
                .is_some_and(|after| lexemes[after].kind == RustLexemeKind::Punctuation(b':'))
    }) {
        return None;
    }
    let mut segment = previous_path_segment(lexemes, position)?;
    match lexeme_text(body, lexemes[segment]) {
        "self" if previous_path_segment(lexemes, segment).is_none() => {
            Some(RelativeAliasModulePath::SelfModule)
        }
        "super" => {
            let mut levels = 1;
            while let Some(previous) = previous_path_segment(lexemes, segment) {
                if lexeme_text(body, lexemes[previous]) != "super" {
                    return None;
                }
                levels += 1;
                segment = previous;
            }
            Some(RelativeAliasModulePath::Super(levels))
        }
        _ => None,
    }
}

pub(crate) fn previous_path_segment(lexemes: &[RustLexeme], position: usize) -> Option<usize> {
    let second_colon = previous_significant(lexemes, position)?;
    let first_colon = previous_significant(lexemes, second_colon)?;
    if lexemes[second_colon].kind != RustLexemeKind::Punctuation(b':')
        || lexemes[first_colon].kind != RustLexemeKind::Punctuation(b':')
    {
        return None;
    }
    previous_significant(lexemes, first_colon)
        .filter(|segment| lexemes[*segment].kind == RustLexemeKind::Identifier)
}

pub(crate) fn next_significant(lexemes: &[RustLexeme], position: usize) -> Option<usize> {
    (position + 1..lexemes.len()).find(|&index| lexemes[index].kind != RustLexemeKind::Trivia)
}

pub(crate) fn previous_significant(lexemes: &[RustLexeme], position: usize) -> Option<usize> {
    (0..position)
        .rev()
        .find(|&index| lexemes[index].kind != RustLexemeKind::Trivia)
}

pub(crate) fn lexeme_text(body: &str, lexeme: RustLexeme) -> &str {
    &body[lexeme.start..lexeme.end]
}

pub(crate) fn rust_identifier_name(identifier: &str) -> &str {
    identifier.strip_prefix("r#").unwrap_or(identifier)
}
