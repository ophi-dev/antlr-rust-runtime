use super::{
    BTreeSet, Cow, FormatMacroCaptures, Range, RustDelimiterMap, RustLexeme, RustLexemeKind,
    RustReplacement, io, is_turbofish_open, is_unqualified_identifier, lex_rust_body, lexeme_text,
    next_significant, raw_identifier_end, rust_identifier_end, rust_identifier_name,
    unsupported_antlr4rust,
};

pub(crate) fn format_macro_capture_candidates(
    body: &str,
    lexemes: &[RustLexeme],
    delimiters: &RustDelimiterMap,
    token_aliases: &BTreeSet<String>,
) -> Vec<FormatMacroCaptures> {
    let mut captures = Vec::new();
    for (position, lexeme) in lexemes.iter().enumerate() {
        if lexeme.kind != RustLexemeKind::Identifier {
            continue;
        }
        let Some(format_argument) = format_string_argument_index(lexeme_text(body, *lexeme)) else {
            continue;
        };
        let Some(bang) = next_significant(lexemes, position) else {
            continue;
        };
        let Some(open) = next_significant(lexemes, bang) else {
            continue;
        };
        if lexemes[bang].kind != RustLexemeKind::Punctuation(b'!')
            || !matches!(
                lexemes[open].kind,
                RustLexemeKind::Punctuation(b'(' | b'[' | b'{')
            )
        {
            continue;
        }
        let Some(close) = delimiters.pairs[open] else {
            continue;
        };
        let arguments = macro_argument_ranges(lexemes, open, close, delimiters);
        let Some(format_range) = arguments.get(format_argument) else {
            continue;
        };
        let Some(format_literal) = first_significant_in(lexemes, format_range.clone()) else {
            continue;
        };
        if lexemes[format_literal].kind != RustLexemeKind::Literal
            || next_significant(&lexemes[..format_range.end], format_literal).is_some()
        {
            continue;
        }
        let Some(content) = rust_format_literal_content(body, lexemes[format_literal]) else {
            continue;
        };
        let mut aliases = format_capture_aliases(content.as_ref(), token_aliases);
        for argument in arguments.iter().skip(format_argument + 1) {
            let Some(name) = first_significant_in(lexemes, argument.clone()) else {
                continue;
            };
            let Some(equal) = next_significant(&lexemes[..argument.end], name) else {
                continue;
            };
            if lexemes[name].kind == RustLexemeKind::Identifier
                && lexemes[equal].kind == RustLexemeKind::Punctuation(b'=')
            {
                aliases.remove(rust_identifier_name(lexeme_text(body, lexemes[name])));
            }
        }
        if !aliases.is_empty() {
            captures.push(FormatMacroCaptures {
                format_literal,
                close,
                aliases,
            });
        }
    }
    captures
}

pub(crate) fn format_string_argument_index(macro_name: &str) -> Option<usize> {
    match macro_name {
        "format" | "format_args" | "eprint" | "eprintln" | "panic" | "print" | "println"
        | "todo" | "unreachable" => Some(0),
        "assert" | "debug_assert" | "write" | "writeln" => Some(1),
        "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne" => Some(2),
        _ => None,
    }
}

pub(crate) fn macro_argument_ranges(
    lexemes: &[RustLexeme],
    open: usize,
    close: usize,
    delimiters: &RustDelimiterMap,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = open + 1;
    let mut position = start;
    let mut generic_depth = 0_usize;
    while position < close {
        if matches!(
            lexemes[position].kind,
            RustLexemeKind::Punctuation(b'(' | b'[' | b'{')
        ) && let Some(nested_close) = delimiters.pairs[position]
            && nested_close < close
        {
            position = nested_close + 1;
            continue;
        }
        match lexemes[position].kind {
            RustLexemeKind::Punctuation(b'<')
                if generic_depth > 0 || is_turbofish_open(lexemes, position) =>
            {
                generic_depth += 1;
            }
            RustLexemeKind::Punctuation(b'>') if generic_depth > 0 => {
                generic_depth -= 1;
            }
            RustLexemeKind::Punctuation(b',') if generic_depth == 0 => {
                ranges.push(start..position);
                start = position + 1;
            }
            _ => {}
        }
        position += 1;
    }
    if start < close {
        ranges.push(start..close);
    }
    ranges
}

pub(crate) fn first_significant_in(
    lexemes: &[RustLexeme],
    mut range: Range<usize>,
) -> Option<usize> {
    range.find(|position| lexemes[*position].kind != RustLexemeKind::Trivia)
}

pub(crate) fn rust_format_literal_content(body: &str, literal: RustLexeme) -> Option<Cow<'_, str>> {
    let source = lexeme_text(body, literal);
    if source.starts_with('"') && source.ends_with('"') {
        let content = source.get(1..source.len().checked_sub(1)?)?;
        return decode_rust_string_content(content).map(Cow::Owned);
    }
    let hashes = source
        .strip_prefix('r')?
        .bytes()
        .take_while(|byte| *byte == b'#')
        .count();
    let content_start = 2 + hashes;
    let content_end = source.len().checked_sub(1 + hashes)?;
    source.get(content_start..content_end).map(Cow::Borrowed)
}

pub(crate) fn decode_rust_string_content(content: &str) -> Option<String> {
    let mut decoded = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }
        match chars.next()? {
            '0' => decoded.push('\0'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            '\'' => decoded.push('\''),
            '"' => decoded.push('"'),
            '\\' => decoded.push('\\'),
            'x' => {
                let high = chars.next()?.to_digit(16)?;
                let low = chars.next()?.to_digit(16)?;
                decoded.push(char::from_u32(high * 16 + low)?);
            }
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut value = 0_u32;
                let mut digits = 0_usize;
                let mut closed = false;
                for escaped in chars.by_ref() {
                    match escaped {
                        '}' if digits > 0 => {
                            closed = true;
                            break;
                        }
                        '_' => {}
                        _ => {
                            let digit = escaped.to_digit(16)?;
                            digits += 1;
                            if digits > 6 {
                                return None;
                            }
                            value = value.checked_mul(16)?.checked_add(digit)?;
                        }
                    }
                }
                if !closed {
                    return None;
                }
                decoded.push(char::from_u32(value)?);
            }
            '\n' => {
                while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                    chars.next();
                }
            }
            '\r' => {
                if chars.next()? != '\n' {
                    return None;
                }
                while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                    chars.next();
                }
            }
            _ => return None,
        }
    }
    Some(decoded)
}

pub(crate) fn format_capture_aliases(
    format_string: &str,
    token_aliases: &BTreeSet<String>,
) -> BTreeSet<String> {
    let bytes = format_string.as_bytes();
    let mut aliases = BTreeSet::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let Some(relative) = format_string[offset..].find('{') else {
            break;
        };
        let open = offset + relative;
        if bytes.get(open + 1) == Some(&b'{') {
            offset = open + 2;
            continue;
        }
        let mut identifier_start = open + 1;
        while bytes
            .get(identifier_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            identifier_start += 1;
        }
        let identifier_end = raw_identifier_end(format_string, identifier_start)
            .or_else(|| rust_identifier_end(format_string, identifier_start));
        if let Some(identifier_end) = identifier_end {
            let identifier = rust_identifier_name(&format_string[identifier_start..identifier_end]);
            if token_aliases.contains(identifier)
                && matches!(bytes.get(identifier_end), Some(b'}' | b':'))
            {
                aliases.insert(identifier.to_owned());
            }
        }
        offset = open + 1;
    }
    aliases
}

pub(crate) fn apply_rust_replacements(
    body: &str,
    replacements: &[RustReplacement],
) -> io::Result<String> {
    let mut out = String::with_capacity(body.len());
    let mut copied = 0;
    for replacement in replacements {
        if replacement.range.start < copied {
            return Err(unsupported_antlr4rust(
                "overlapping compatibility expressions",
            ));
        }
        out.push_str(&body[copied..replacement.range.start]);
        out.push_str(&replacement.text);
        copied = replacement.range.end;
    }
    out.push_str(&body[copied..]);
    Ok(out)
}

pub(crate) fn macro_rules_definition_ranges(body: &str) -> Vec<Range<usize>> {
    let lexemes = lex_rust_body(body);
    let delimiters = RustDelimiterMap::new(&lexemes);
    let mut ranges = Vec::new();
    for (position, lexeme) in lexemes.iter().enumerate() {
        if lexeme.kind != RustLexemeKind::Identifier
            || lexeme_text(body, *lexeme) != "macro_rules"
            || !is_unqualified_identifier(&lexemes, position)
        {
            continue;
        }
        let Some(bang) = next_significant(&lexemes, position) else {
            continue;
        };
        let Some(name) = next_significant(&lexemes, bang) else {
            continue;
        };
        let Some(open) = next_significant(&lexemes, name) else {
            continue;
        };
        if lexemes[bang].kind != RustLexemeKind::Punctuation(b'!')
            || lexemes[name].kind != RustLexemeKind::Identifier
            || !matches!(
                lexemes[open].kind,
                RustLexemeKind::Punctuation(b'(' | b'[' | b'{')
            )
        {
            continue;
        }
        if let Some(close) = delimiters.pairs[open] {
            ranges.push(lexeme.start..lexemes[close].end);
        }
    }
    ranges
}
