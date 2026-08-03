/// Splits a body made only of adjacent target-template expressions.
pub(crate) fn template_sequence_bodies(body: &str) -> Option<Vec<&str>> {
    let mut templates = Vec::new();
    let mut cursor = 0;
    while cursor < body.len() {
        cursor = skip_ascii_whitespace(body, cursor);
        if cursor == body.len() {
            break;
        }
        if body.as_bytes().get(cursor) != Some(&b'<') {
            return None;
        }
        let close_angle = matching_template_close(body, cursor + 1)?;
        templates.push(&body[cursor + 1..close_angle]);
        cursor = close_angle + 1;
    }
    (!templates.is_empty()).then_some(templates)
}

/// Finds the closing brace for a named ANTLR action block while ignoring braces
/// inside string and character literals.
pub(crate) fn matching_action_brace(source: &str, mut index: usize) -> Option<usize> {
    let mut nested = 0_usize;
    let mut double_quoted = false;
    let mut escaped = false;
    while let Some(ch) = source[index..].chars().next() {
        if escaped {
            escaped = false;
            index += ch.len_utf8();
            continue;
        }
        match ch {
            '\\' if double_quoted => escaped = true,
            '"' => double_quoted = !double_quoted,
            '\'' if !double_quoted => {
                if let Some(next_index) = skip_char_literal(source, index) {
                    index = next_index;
                    continue;
                }
            }
            '{' if !double_quoted => nested += 1,
            '}' if !double_quoted && nested == 0 => return Some(index),
            '}' if !double_quoted => nested = nested.saturating_sub(1),
            _ => {}
        }
        index += ch.len_utf8();
    }
    None
}

/// Finds the matching `>` for a `StringTemplate` expression, allowing nested
/// template expressions inside arguments such as `<Assert({<Inner()>})>`.
pub(crate) fn matching_template_close(source: &str, mut index: usize) -> Option<usize> {
    let mut nested = 0_usize;
    let mut double_quoted = false;
    let mut escaped = false;
    while let Some(ch) = source[index..].chars().next() {
        if escaped {
            escaped = false;
            index += ch.len_utf8();
            continue;
        }
        match ch {
            '\\' if double_quoted => escaped = true,
            '"' => double_quoted = !double_quoted,
            '\'' if !double_quoted => {
                if let Some(next_index) = skip_char_literal(source, index) {
                    index = next_index;
                    continue;
                }
            }
            '<' if !double_quoted => nested += 1,
            '>' if !double_quoted && nested == 0 => return Some(index),
            '>' if !double_quoted => nested = nested.saturating_sub(1),
            _ => {}
        }
        index += ch.len_utf8();
    }
    None
}

/// Skips one Rust-style character literal starting at `index`, if present.
///
/// Lifetimes such as `&'a str` and `<'input>` are intentionally not skipped:
/// they do not contain a closing quote immediately after one character or one
/// escaped character.
fn skip_char_literal(source: &str, index: usize) -> Option<usize> {
    let mut cursor = index.checked_add('\''.len_utf8())?;
    let mut chars = source[cursor..].chars();
    let first = chars.next()?;
    cursor += first.len_utf8();
    if first == '\\' {
        let escaped = chars.next()?;
        cursor += escaped.len_utf8();
    }
    if source[cursor..].starts_with('\'') {
        Some(cursor + '\''.len_utf8())
    } else {
        None
    }
}

/// Advances past ASCII whitespace and returns the first non-whitespace byte
/// boundary at or after `index`.
pub(crate) fn skip_ascii_whitespace(source: &str, mut index: usize) -> usize {
    while source
        .as_bytes()
        .get(index)
        .is_some_and(u8::is_ascii_whitespace)
    {
        index += 1;
    }
    index
}

/// Splits a `StringTemplate` argument list while ignoring commas inside quoted
/// strings or nested template/function calls.
pub(crate) fn split_template_arguments(arguments: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut paren_depth = 0_usize;
    let mut angle_depth = 0_usize;
    let mut brace_depth = 0_usize;
    for (index, ch) in arguments.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '(' if !quoted => paren_depth += 1,
            ')' if !quoted => paren_depth = paren_depth.saturating_sub(1),
            '<' if !quoted => angle_depth += 1,
            '>' if !quoted => angle_depth = angle_depth.saturating_sub(1),
            '{' if !quoted => brace_depth += 1,
            '}' if !quoted => brace_depth = brace_depth.saturating_sub(1),
            ',' if !quoted && paren_depth == 0 && angle_depth == 0 && brace_depth == 0 => {
                parts.push(arguments[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(arguments[start..].trim());
    parts
}

/// Decodes a quoted `StringTemplate` argument into the payload that generated
/// Rust code should compare or print.
pub(crate) fn parse_template_string(argument: &str) -> Option<String> {
    let mut value = argument.trim();
    value = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(ch);
        }
    }
    if out.starts_with('"') && out.ends_with('"') && out.len() >= 2 {
        out = out[1..out.len() - 1].to_owned();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{matching_action_brace, matching_template_close, template_sequence_bodies};

    #[test]
    fn template_sequence_extracts_adjacent_bodies() {
        assert_eq!(
            template_sequence_bodies(" <Before()> \n <After(\"x\")> "),
            Some(vec!["Before()", "After(\"x\")"])
        );
        assert_eq!(template_sequence_bodies("<Before()> raw"), None);
    }

    #[test]
    fn action_brace_ignores_braces_inside_char_literals() {
        let source = "{ char close = '}'; char open = '{'; return close; } tail";

        assert_eq!(matching_action_brace(source, 1), source.find("} tail"));
    }

    #[test]
    fn action_brace_does_not_treat_lifetime_as_char_literal() {
        let source = "{ let value: &'a str = name; } tail";

        assert_eq!(matching_action_brace(source, 1), source.find("} tail"));
    }

    #[test]
    fn template_close_ignores_angles_inside_char_literals() {
        let source = "<Assert({ char close = '>'; char open = '<'; return close; })> tail";

        assert_eq!(matching_template_close(source, 1), source.find("> tail"));
    }

    #[test]
    fn template_close_does_not_treat_lifetime_as_char_literal() {
        let source = "<AssertType(<'input>())> tail";

        assert_eq!(matching_template_close(source, 1), source.find("> tail"));
    }
}
