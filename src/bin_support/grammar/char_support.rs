use antlr4_runtime::atn::IntervalSet;

use super::unicode::simple_uppercase;

const MAX_CODE_POINT: i32 = 0x10_FFFF;

pub(super) fn get_antlr_char_literal_for_char(code_point: i32) -> String {
    let escaped = match code_point {
        0x08 => Some("\\b"),
        0x09 => Some("\\t"),
        0x0a => Some("\\n"),
        0x0c => Some("\\f"),
        0x0d => Some("\\r"),
        0x5c => Some("\\\\"),
        _ => None,
    };
    let body = if code_point < 0 {
        "<INVALID>".to_owned()
    } else if let Some(escaped) = escaped {
        escaped.to_owned()
    } else if code_point <= 0x7f
        && !u8::try_from(code_point).is_ok_and(|value| value.is_ascii_control())
    {
        match code_point {
            0x27 => "\\'".to_owned(),
            _ => char::from_u32(
                u32::try_from(code_point).expect("Basic Latin code point is nonnegative"),
            )
            .expect("Basic Latin code point is valid")
            .to_string(),
        }
    } else if code_point <= 0xffff {
        format!("\\u{code_point:04X}")
    } else {
        format!("\\u{{{code_point:06X}}}")
    };
    format!("'{body}'")
}

pub(super) fn get_char_value_from_grammar_char_literal(literal: Option<&str>) -> i32 {
    let Some(literal) = literal.filter(|literal| literal.len() >= 3) else {
        return -1;
    };
    literal
        .get(1..literal.len() - 1)
        .map_or(-1, get_char_value_from_char_in_grammar_literal)
}

pub(super) fn get_string_from_grammar_string_literal(literal: &str) -> Option<String> {
    if literal.len() < 2 {
        return Some(String::new());
    }
    let body = literal.get(1..literal.len() - 1)?;
    let values = decode_literal_body(body).ok()?;
    values
        .into_iter()
        .map(|value| u32::try_from(value).ok().and_then(char::from_u32))
        .collect()
}

pub(super) fn get_char_value_from_char_in_grammar_literal(text: &str) -> i32 {
    let mut characters = text.chars();
    if let Some(value) = characters.next()
        && characters.next().is_none()
    {
        return value as i32;
    }
    if !text.starts_with('\\') {
        return -1;
    }
    parse_code_point_escape(text, 0, false).map_or(-1, |(value, consumed)| {
        if consumed == text.len() { value } else { -1 }
    })
}

pub(super) fn parse_hex_value(text: &str, start: isize, end: isize) -> i32 {
    let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
        return -1;
    };
    let Some(digits) = text.get(start..end) else {
        return -1;
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return -1;
    }
    i32::from_str_radix(digits, 16).unwrap_or(-1)
}

pub(super) fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    let mapped = simple_uppercase(first as i32);
    let first = u32::try_from(mapped)
        .ok()
        .and_then(char::from_u32)
        .unwrap_or(first);
    let mut result = String::with_capacity(value.len());
    result.push(first);
    result.push_str(characters.as_str());
    result
}

pub(super) fn get_interval_set_escaped_string(intervals: &IntervalSet) -> String {
    intervals
        .ranges()
        .iter()
        .map(|&(start, stop)| get_range_escaped_string(start, stop))
        .collect::<Vec<_>>()
        .join(" | ")
}

pub(super) fn get_range_escaped_string(start: i32, stop: i32) -> String {
    if start == stop {
        get_antlr_char_literal_for_char(start)
    } else {
        format!(
            "{}..{}",
            get_antlr_char_literal_for_char(start),
            get_antlr_char_literal_for_char(stop)
        )
    }
}

pub(super) fn decode_string_literal(literal: &str) -> Result<Vec<i32>, String> {
    let body = literal
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .ok_or_else(|| format!("invalid lexer string literal {literal}"))?;
    decode_literal_body(body)
}

pub(super) fn decode_character_literal(literal: &str) -> Result<i32, String> {
    let values = decode_string_literal(literal)?;
    match values.as_slice() {
        [value] => Ok(*value),
        _ => Err(format!(
            "lexer character literal {literal} must contain exactly one Unicode scalar"
        )),
    }
}

fn decode_literal_body(body: &str) -> Result<Vec<i32>, String> {
    let mut values = Vec::new();
    let mut cursor = 0;
    while cursor < body.len() {
        let character = body[cursor..]
            .chars()
            .next()
            .expect("cursor is on a character boundary");
        if character == '\\' {
            let (value, consumed) = parse_code_point_escape(body, cursor, false)?;
            push_code_point(&mut values, value);
            cursor += consumed;
        } else {
            push_code_point(&mut values, character as i32);
            cursor += character.len_utf8();
        }
    }
    Ok(values)
}

fn push_code_point(values: &mut Vec<i32>, value: i32) {
    let Some(&high) = values.last() else {
        values.push(value);
        return;
    };
    if (0xD800..=0xDBFF).contains(&high) && (0xDC00..=0xDFFF).contains(&value) {
        let supplementary = 0x1_0000 + ((high - 0xD800) << 10) + value - 0xDC00;
        *values.last_mut().expect("last value exists") = supplementary;
    } else {
        values.push(value);
    }
}

pub(super) fn parse_code_point_escape(
    text: &str,
    start: usize,
    in_set: bool,
) -> Result<(i32, usize), String> {
    let tail = text
        .get(start..)
        .ok_or_else(|| "escape starts outside source text".to_owned())?;
    let mut characters = tail.char_indices();
    let (_, slash) = characters
        .next()
        .ok_or_else(|| "unterminated escape sequence".to_owned())?;
    if slash != '\\' {
        return Err("escape sequence does not start with a backslash".to_owned());
    }
    let (escaped_offset, escaped) = characters
        .next()
        .ok_or_else(|| "unterminated escape sequence".to_owned())?;
    let simple = match escaped {
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'b' => Some('\u{0008}'),
        'f' => Some('\u{000c}'),
        '\\' => Some('\\'),
        '\'' => Some('\''),
        ']' | '-' if in_set => Some(escaped),
        _ => None,
    };
    if let Some(value) = simple {
        return Ok((value as i32, escaped_offset + escaped.len_utf8()));
    }
    if escaped != 'u' {
        return Err(format!("invalid escape sequence \\{escaped}"));
    }

    let digits_start = escaped_offset + escaped.len_utf8();
    let unicode = &tail[digits_start..];
    let (digits, consumed) = if let Some(rest) = unicode.strip_prefix('{') {
        let close = rest
            .find('}')
            .ok_or_else(|| "unterminated braced Unicode escape".to_owned())?;
        (&rest[..close], digits_start + 1 + close + 1)
    } else {
        if unicode.len() < 4 {
            return Err("Unicode escape must contain four hexadecimal digits".to_owned());
        }
        (&unicode[..4], digits_start + 4)
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid Unicode escape \\u{{{digits}}}"));
    }
    let value = u32::from_str_radix(digits, 16)
        .map_err(|_| format!("invalid Unicode escape \\u{{{digits}}}"))?;
    if value > MAX_CODE_POINT as u32 {
        return Err(format!("Unicode escape is out of range: {value:#x}"));
    }
    Ok((
        i32::try_from(value).expect("Unicode code point fits i32"),
        consumed,
    ))
}

#[cfg(test)]
mod tests {
    use antlr4_runtime::atn::IntervalSet;

    use super::*;

    #[test]
    fn antlr_char_literal_for_char_matches_java() {
        assert_eq!(get_antlr_char_literal_for_char(-1), "'<INVALID>'");
        assert_eq!(get_antlr_char_literal_for_char(i32::from(b'\n')), "'\\n'");
        assert_eq!(get_antlr_char_literal_for_char(i32::from(b'\\')), "'\\\\'");
        assert_eq!(get_antlr_char_literal_for_char(i32::from(b'\'')), "'\\''");
        assert_eq!(get_antlr_char_literal_for_char(i32::from(b'b')), "'b'");
        assert_eq!(get_antlr_char_literal_for_char(0xffff), "'\\uFFFF'");
        assert_eq!(get_antlr_char_literal_for_char(0x10_ffff), "'\\u{10FFFF}'");
    }

    #[test]
    fn char_value_from_grammar_char_literal_matches_java() {
        assert_eq!(get_char_value_from_grammar_char_literal(None), -1);
        assert_eq!(get_char_value_from_grammar_char_literal(Some("")), -1);
        assert_eq!(get_char_value_from_grammar_char_literal(Some("b")), -1);
        assert_eq!(get_char_value_from_grammar_char_literal(Some("foo")), 111);
    }

    #[test]
    fn string_from_grammar_string_literal_matches_java() {
        assert_eq!(get_string_from_grammar_string_literal("foo\\u{bbb"), None);
        assert_eq!(get_string_from_grammar_string_literal("foo\\u{[]bb"), None);
        assert_eq!(get_string_from_grammar_string_literal("foo\\u[]bb"), None);
        assert_eq!(get_string_from_grammar_string_literal("foo\\ubb"), None);
        assert_eq!(
            get_string_from_grammar_string_literal("foo\\u{bb}bb"),
            Some("oo\u{bb}b".to_owned())
        );
        assert_eq!(
            get_string_from_grammar_string_literal("'\\uD83C\\uDF0D'"),
            Some("\u{1f30d}".to_owned())
        );
        assert_eq!(
            decode_string_literal("'\\uD83C\\uDF0D'"),
            Ok(vec![0x1_f30d])
        );
        assert_eq!(decode_string_literal("'\\uD83C'"), Ok(vec![0xd83c]));
    }

    #[test]
    fn char_value_from_char_in_grammar_literal_matches_java() {
        assert_eq!(get_char_value_from_char_in_grammar_literal("f"), 102);
        assert_eq!(get_char_value_from_char_in_grammar_literal("' "), -1);
        assert_eq!(get_char_value_from_char_in_grammar_literal("\\ "), -1);
        assert_eq!(get_char_value_from_char_in_grammar_literal("\\'"), 39);
        assert_eq!(get_char_value_from_char_in_grammar_literal("\\n"), 10);
        assert_eq!(get_char_value_from_char_in_grammar_literal("foobar"), -1);
        assert_eq!(get_char_value_from_char_in_grammar_literal("\\u1234"), 4660);
        assert_eq!(get_char_value_from_char_in_grammar_literal("\\u{12}"), 18);
        assert_eq!(get_char_value_from_char_in_grammar_literal("\\u{"), -1);
        assert_eq!(get_char_value_from_char_in_grammar_literal("foo"), -1);
    }

    #[test]
    fn parse_hex_value_matches_java() {
        assert_eq!(parse_hex_value("foobar", -1, 3), -1);
        assert_eq!(parse_hex_value("foobar", 1, -1), -1);
        assert_eq!(parse_hex_value("foobar", 1, 3), -1);
        assert_eq!(parse_hex_value("123456", 1, 3), 35);
    }

    #[test]
    fn capitalize_matches_java() {
        assert_eq!(capitalize("foo"), "Foo");
    }

    #[test]
    fn interval_set_escaped_string_matches_java() {
        assert_eq!(get_interval_set_escaped_string(&IntervalSet::new()), "");
        assert_eq!(
            get_interval_set_escaped_string(&IntervalSet::from_range(0, 0)),
            "'\\u0000'"
        );
        let mut set = IntervalSet::new();
        set.add(3);
        set.add(1);
        set.add(2);
        assert_eq!(
            get_interval_set_escaped_string(&set),
            "'\\u0001'..'\\u0003'"
        );
    }

    #[test]
    fn range_escaped_string_matches_java() {
        assert_eq!(get_range_escaped_string(2, 4), "'\\u0002'..'\\u0004'");
        assert_eq!(get_range_escaped_string(2, 2), "'\\u0002'");
    }
}
