use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RustLexemeKind {
    Trivia,
    Identifier,
    Literal,
    Punctuation(u8),
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RustLexeme {
    kind: RustLexemeKind,
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct RustDelimiterMap {
    pairs: Vec<Option<usize>>,
}

impl RustDelimiterMap {
    fn new(lexemes: &[RustLexeme]) -> Self {
        let mut pairs = vec![None; lexemes.len()];
        let mut stack = Vec::new();
        for (position, lexeme) in lexemes.iter().enumerate() {
            match lexeme.kind {
                RustLexemeKind::Punctuation(open @ (b'(' | b'[' | b'{')) => {
                    let close = match open {
                        b'(' => b')',
                        b'[' => b']',
                        b'{' => b'}',
                        _ => unreachable!("matched opening delimiter"),
                    };
                    stack.push((position, close));
                }
                RustLexemeKind::Punctuation(close @ (b')' | b']' | b'}')) => {
                    if let Some((open, expected)) = stack.pop()
                        && close == expected
                    {
                        pairs[open] = Some(position);
                        pairs[position] = Some(open);
                    }
                }
                _ => {}
            }
        }
        Self { pairs }
    }
}

#[doc(hidden)]
pub fn member_cfg_predicates(attributes: &str) -> Vec<String> {
    let lexemes = lex_rust_body(attributes);
    let delimiters = RustDelimiterMap::new(&lexemes);
    let mut predicates = Vec::new();
    let mut position = 0;
    while position < lexemes.len() {
        if lexemes[position].kind != RustLexemeKind::Punctuation(b'#') {
            position += 1;
            continue;
        }
        let Some(attribute_open) = next_significant(&lexemes, position) else {
            break;
        };
        if lexemes[attribute_open].kind != RustLexemeKind::Punctuation(b'[') {
            position += 1;
            continue;
        }
        let Some(attribute_close) = delimiters.pairs[attribute_open] else {
            position += 1;
            continue;
        };
        position = attribute_close + 1;

        let Some(name) = next_significant(&lexemes, attribute_open) else {
            continue;
        };
        if let Some(predicate) =
            cfg_meta_predicate(attributes, &lexemes, name..attribute_close, &delimiters)
        {
            predicates.push(predicate);
        }
    }
    predicates
}

fn cfg_meta_predicate(
    source: &str,
    lexemes: &[RustLexeme],
    range: Range<usize>,
    delimiters: &RustDelimiterMap,
) -> Option<String> {
    let name = range
        .clone()
        .find(|position| lexemes[*position].kind != RustLexemeKind::Trivia)?;
    if lexemes[name].kind != RustLexemeKind::Identifier {
        return None;
    }
    let open = next_significant(lexemes, name)?;
    if open >= range.end || lexemes[open].kind != RustLexemeKind::Punctuation(b'(') {
        return None;
    }
    let close = delimiters.pairs.get(open).copied().flatten()?;
    if close >= range.end {
        return None;
    }
    match lexeme_text(source, lexemes[name]) {
        "cfg" => cfg_meta_source(source, lexemes, open + 1..close),
        "cfg_attr" => {
            let arguments = split_cfg_meta_arguments(lexemes, open + 1..close, delimiters);
            let condition = cfg_meta_source(source, lexemes, arguments.first()?.clone())?;
            let applied = arguments
                .into_iter()
                .skip(1)
                .filter_map(|argument| cfg_meta_predicate(source, lexemes, argument, delimiters))
                .collect::<Vec<_>>();
            let applied = cfg_all_predicate(&applied)?;
            Some(format!("any(not({condition}), {applied})"))
        }
        _ => None,
    }
}

fn cfg_meta_source(source: &str, lexemes: &[RustLexeme], range: Range<usize>) -> Option<String> {
    let start = range
        .clone()
        .find(|position| lexemes[*position].kind != RustLexemeKind::Trivia)?;
    let end = range
        .rev()
        .find(|position| lexemes[*position].kind != RustLexemeKind::Trivia)?;
    Some(
        source[lexemes[start].start..lexemes[end].end]
            .trim()
            .to_owned(),
    )
}

fn split_cfg_meta_arguments(
    lexemes: &[RustLexeme],
    range: Range<usize>,
    delimiters: &RustDelimiterMap,
) -> Vec<Range<usize>> {
    let mut arguments = Vec::new();
    let mut start = range.start;
    let mut position = range.start;
    while position < range.end {
        if matches!(
            lexemes[position].kind,
            RustLexemeKind::Punctuation(b'(' | b'[' | b'{')
        ) && let Some(close) = delimiters.pairs[position]
            && close < range.end
        {
            position = close + 1;
            continue;
        }
        if lexemes[position].kind == RustLexemeKind::Punctuation(b',') {
            arguments.push(start..position);
            start = position + 1;
        }
        position += 1;
    }
    arguments.push(start..range.end);
    arguments
}

#[doc(hidden)]
pub fn cfg_all_predicate(predicates: &[String]) -> Option<String> {
    match predicates {
        [] => None,
        [predicate] => Some(predicate.clone()),
        predicates => Some(format!("all({})", predicates.join(", "))),
    }
}

fn next_significant(lexemes: &[RustLexeme], position: usize) -> Option<usize> {
    (position + 1..lexemes.len()).find(|&index| lexemes[index].kind != RustLexemeKind::Trivia)
}

fn lexeme_text(body: &str, lexeme: RustLexeme) -> &str {
    &body[lexeme.start..lexeme.end]
}

fn lex_rust_body(body: &str) -> Vec<RustLexeme> {
    let bytes = body.as_bytes();
    let mut lexemes = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let start = offset;
        let kind = if bytes[offset].is_ascii_whitespace() {
            offset = skip_while(bytes, offset, u8::is_ascii_whitespace);
            RustLexemeKind::Trivia
        } else if body[offset..].starts_with("//") {
            offset = body[offset..]
                .find('\n')
                .map_or(body.len(), |newline| offset + newline);
            RustLexemeKind::Trivia
        } else if body[offset..].starts_with("/*") {
            offset = block_comment_end(body, offset);
            RustLexemeKind::Trivia
        } else if let Some(end) = raw_literal_end(body, offset) {
            offset = end;
            RustLexemeKind::Literal
        } else if let Some(end) = quoted_literal_end(body, offset) {
            offset = end;
            RustLexemeKind::Literal
        } else if let Some(end) = raw_identifier_end(body, offset) {
            offset = end;
            RustLexemeKind::Identifier
        } else if let Some(end) = rust_identifier_end(body, offset) {
            offset = end;
            RustLexemeKind::Identifier
        } else if bytes[offset].is_ascii_punctuation() {
            offset += 1;
            RustLexemeKind::Punctuation(bytes[offset - 1])
        } else {
            offset += body[offset..]
                .chars()
                .next()
                .expect("offset is within the string")
                .len_utf8();
            RustLexemeKind::Other
        };
        lexemes.push(RustLexeme {
            kind,
            start,
            end: offset,
        });
    }
    lexemes
}

fn skip_while(bytes: &[u8], mut offset: usize, predicate: fn(&u8) -> bool) -> usize {
    while bytes.get(offset).is_some_and(predicate) {
        offset += 1;
    }
    offset
}

fn block_comment_end(body: &str, start: usize) -> usize {
    let bytes = body.as_bytes();
    let mut depth = 1;
    let mut offset = start + 2;
    while offset + 1 < bytes.len() {
        match &bytes[offset..offset + 2] {
            b"/*" => {
                depth += 1;
                offset += 2;
            }
            b"*/" => {
                depth -= 1;
                offset += 2;
                if depth == 0 {
                    return offset;
                }
            }
            _ => offset += 1,
        }
    }
    body.len()
}

fn raw_literal_end(body: &str, start: usize) -> Option<usize> {
    let rest = &body[start..];
    let prefix = ["br", "cr", "r"]
        .into_iter()
        .find(|prefix| rest.starts_with(prefix))?;
    let mut quote = start + prefix.len();
    while body.as_bytes().get(quote) == Some(&b'#') {
        quote += 1;
    }
    if body.as_bytes().get(quote) != Some(&b'"') {
        return None;
    }
    let hashes = quote - start - prefix.len();
    let closing = format!("\"{}", "#".repeat(hashes));
    let content = quote + 1;
    Some(
        body[content..]
            .find(&closing)
            .map_or(body.len(), |end| content + end + closing.len()),
    )
}

fn raw_identifier_end(body: &str, start: usize) -> Option<usize> {
    if body.as_bytes().get(start..start + 2) != Some(b"r#") {
        return None;
    }
    rust_identifier_end(body, start + 2)
}

fn rust_identifier_end(body: &str, start: usize) -> Option<usize> {
    let mut chars = body.get(start..)?.char_indices();
    let (_, first) = chars.next()?;
    if first != '_' && !first.is_alphabetic() {
        return None;
    }
    let mut end = start + first.len_utf8();
    for (relative, ch) in chars {
        if ch != '_' && !ch.is_alphanumeric() {
            break;
        }
        end = start + relative + ch.len_utf8();
    }
    Some(end)
}

fn quoted_literal_end(body: &str, start: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let (quote, content) = match bytes.get(start..start + 2) {
        Some([b'b' | b'c', b'"']) => (b'"', start + 2),
        Some([b'b', b'\'']) => (b'\'', start + 2),
        _ if bytes[start] == b'"' => (b'"', start + 1),
        _ if bytes[start] == b'\'' => (b'\'', start + 1),
        _ => return None,
    };
    if quote == b'\'' {
        return char_literal_end(body, content);
    }

    let mut offset = content;
    let mut escaped = false;
    while offset < bytes.len() {
        if escaped {
            escaped = false;
        } else if bytes[offset] == b'\\' {
            escaped = true;
        } else if bytes[offset] == quote {
            return Some(offset + 1);
        }
        offset += 1;
    }
    Some(body.len())
}

fn char_literal_end(body: &str, content: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let end = if bytes.get(content) == Some(&b'\\') {
        match bytes.get(content + 1).copied()? {
            b'x' => content.checked_add(4)?,
            b'u' if bytes.get(content + 2) == Some(&b'{') => {
                content + 3 + body[content + 3..].find('}')? + 1
            }
            _ => content.checked_add(2)?,
        }
    } else {
        content
            + body[content..]
                .chars()
                .next()
                .filter(|ch| *ch != '\'' && *ch != '\n' && *ch != '\r')?
                .len_utf8()
    };
    (bytes.get(end) == Some(&b'\'')).then_some(end + 1)
}
