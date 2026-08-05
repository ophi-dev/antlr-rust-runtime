use super::Error;

pub(crate) fn decode_basic(value: &str, multiline: bool) -> Result<String, Error> {
    let delimiter = if multiline { "\"\"\"" } else { "\"" };
    let body = strip_delimiters(value, delimiter)?;
    let body = if multiline {
        strip_initial_newline(body)
    } else {
        body
    };
    let mut chars = body.chars().peekable();
    let mut decoded = String::with_capacity(body.len());
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            decode_escape(&mut chars, multiline, &mut decoded)?;
        } else {
            push_source_char(ch, multiline, &mut chars, &mut decoded)?;
        }
    }
    Ok(decoded)
}

pub(crate) fn decode_literal(value: &str, multiline: bool) -> Result<String, Error> {
    let delimiter = if multiline { "'''" } else { "'" };
    let body = strip_delimiters(value, delimiter)?;
    let body = if multiline {
        strip_initial_newline(body)
    } else {
        body
    };
    let mut chars = body.chars().peekable();
    let mut decoded = String::with_capacity(body.len());
    while let Some(ch) = chars.next() {
        push_source_char(ch, multiline, &mut chars, &mut decoded)?;
    }
    Ok(decoded)
}

fn strip_delimiters<'a>(value: &'a str, delimiter: &str) -> Result<&'a str, Error> {
    value
        .strip_prefix(delimiter)
        .and_then(|body| body.strip_suffix(delimiter))
        .ok_or_else(|| Error::invalid(format!("invalid TOML string token {value:?}")))
}

fn strip_initial_newline(value: &str) -> &str {
    value
        .strip_prefix("\r\n")
        .or_else(|| value.strip_prefix('\n'))
        .unwrap_or(value)
}

fn decode_escape(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    multiline: bool,
    decoded: &mut String,
) -> Result<(), Error> {
    let escaped = chars
        .next()
        .ok_or_else(|| Error::invalid("unterminated TOML escape"))?;
    match escaped {
        '"' => decoded.push('"'),
        '\\' => decoded.push('\\'),
        'b' => decoded.push('\u{0008}'),
        't' => decoded.push('\t'),
        'n' => decoded.push('\n'),
        'f' => decoded.push('\u{000c}'),
        'r' => decoded.push('\r'),
        'u' => decoded.push(decode_unicode_escape(chars, 4)?),
        'U' => decoded.push(decode_unicode_escape(chars, 8)?),
        '\n' if multiline => trim_multiline_continuation(chars),
        '\r' if multiline && chars.next_if_eq(&'\n').is_some() => {
            trim_multiline_continuation(chars);
        }
        other => {
            return Err(Error::invalid(format!(
                "unsupported TOML escape sequence \\{other}"
            )));
        }
    }
    Ok(())
}

fn decode_unicode_escape(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    digits: usize,
) -> Result<char, Error> {
    let mut value = 0_u32;
    for _ in 0..digits {
        let digit = chars
            .next()
            .and_then(|ch| ch.to_digit(16))
            .ok_or_else(|| Error::invalid("invalid TOML Unicode escape"))?;
        value = value * 16 + digit;
    }
    char::from_u32(value)
        .ok_or_else(|| Error::invalid(format!("invalid TOML Unicode scalar U+{value:04X}")))
}

fn trim_multiline_continuation(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while chars
        .next_if(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r'))
        .is_some()
    {}
}

fn push_source_char(
    ch: char,
    multiline: bool,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    decoded: &mut String,
) -> Result<(), Error> {
    if ch == '\r' && multiline && chars.next_if_eq(&'\n').is_some() {
        decoded.push('\n');
        return Ok(());
    }
    let forbidden = matches!(ch, '\u{0000}'..='\u{0008}' | '\u{000b}'..='\u{001f}' | '\u{007f}')
        || (!multiline && matches!(ch, '\n' | '\r'));
    if forbidden {
        return Err(Error::invalid(format!(
            "forbidden control character U+{:04X} in TOML string",
            u32::from(ch)
        )));
    }
    decoded.push(ch);
    Ok(())
}
