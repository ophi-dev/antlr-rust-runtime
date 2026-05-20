/// A brace-delimited ANTLR action block recognized by the metadata generator
/// and runtime-testsuite harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TemplateBlock<'a> {
    pub(crate) open_brace: usize,
    pub(crate) body: &'a str,
    pub(crate) after_brace: usize,
    pub(crate) predicate: bool,
}

/// One target-template expression nested inside a named ANTLR action block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NamedActionTemplate<'a> {
    pub(crate) open_brace: usize,
    pub(crate) body: &'a str,
}

/// Finds all target templates inside a rule-level named action body, including
/// multi-template blocks such as the listener-suite `@after` actions.
pub(crate) fn named_action_templates<'a>(
    source: &'a str,
    marker: &str,
) -> Vec<NamedActionTemplate<'a>> {
    let mut templates = Vec::new();
    let mut offset = 0;
    while let Some(marker_start) = source[offset..].find(marker).map(|index| offset + index) {
        let Some(open_brace) = source[marker_start..]
            .find('{')
            .map(|index| marker_start + index)
        else {
            break;
        };
        let Some(close_brace) = matching_action_brace(source, open_brace + 1) else {
            break;
        };
        let mut cursor = open_brace + 1;
        while cursor < close_brace {
            let Some(open_angle) = source[cursor..close_brace]
                .find('<')
                .map(|index| cursor + index)
            else {
                break;
            };
            let Some(close_angle) = matching_template_close(source, open_angle + 1) else {
                break;
            };
            if close_angle > close_brace {
                break;
            }
            templates.push(NamedActionTemplate {
                open_brace,
                body: &source[open_angle + 1..close_angle],
            });
            cursor = close_angle + 1;
        }
        offset = close_brace + 1;
    }
    templates
}

/// Finds the next target-template block while allowing whitespace inside the
/// ANTLR action braces, for example `{ <writeln("$text")> }`.
pub(crate) fn next_template_block(source: &str, offset: usize) -> Option<TemplateBlock<'_>> {
    let mut cursor = offset;
    while let Some(open_rel) = source[cursor..].find('{') {
        let open_brace = cursor + open_rel;
        let template_start = skip_ascii_whitespace(source, open_brace + 1);
        if source.as_bytes().get(template_start) != Some(&b'<') {
            cursor = open_brace + 1;
            continue;
        }
        let close_angle = matching_template_close(source, template_start + 1)?;
        let close_brace = skip_ascii_whitespace(source, close_angle + 1);
        if source.as_bytes().get(close_brace) != Some(&b'}') {
            cursor = open_brace + 1;
            continue;
        }
        let after_brace = close_brace + 1;
        return Some(TemplateBlock {
            open_brace,
            body: &source[template_start + 1..close_angle],
            after_brace,
            predicate: source[after_brace..].trim_start().starts_with('?'),
        });
    }
    None
}

/// Finds one semantic-predicate action block, including expression predicates
/// whose target-template call is only part of the action body.
pub(crate) fn next_predicate_action_block(
    source: &str,
    offset: usize,
) -> Option<TemplateBlock<'_>> {
    let mut cursor = offset;
    while let Some(open_rel) = source[cursor..].find('{') {
        let open_brace = cursor + open_rel;
        let close_brace = matching_action_brace(source, open_brace + 1)?;
        let after_brace = close_brace + 1;
        if source[after_brace..].trim_start().starts_with('?') {
            return Some(TemplateBlock {
                open_brace,
                body: &source[open_brace + 1..close_brace],
                after_brace,
                predicate: true,
            });
        }
        cursor = open_brace + 1;
    }
    None
}

/// Finds the next parser action block, including empty actions serialized as
/// no-op ATN action transitions.
pub(crate) fn next_parser_action_block(
    source: &str,
    offset: usize,
    is_regular_action_body: impl Fn(&str) -> bool,
) -> Option<TemplateBlock<'_>> {
    let mut cursor = offset;
    while let Some(open_rel) = source[cursor..].find('{') {
        let open_brace = cursor + open_rel;
        let close_brace = matching_action_brace(source, open_brace + 1)?;
        let body = &source[open_brace + 1..close_brace];
        if body.trim().is_empty()
            || template_sequence_bodies(body).is_some()
            || is_regular_action_body(body)
        {
            let after_brace = close_brace + 1;
            return Some(TemplateBlock {
                open_brace,
                body,
                after_brace,
                predicate: source[after_brace..].trim_start().starts_with('?'),
            });
        }
        cursor = open_brace + 1;
    }
    None
}

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

/// Returns true when an action block belongs to a rule-level `@after` action.
pub(crate) fn is_after_action(source: &str, open_brace: usize) -> bool {
    is_rule_named_action(source, open_brace, "@after")
}

/// Returns true when an action block belongs to a rule-level `@init` action.
pub(crate) fn is_init_action(source: &str, open_brace: usize) -> bool {
    is_rule_named_action(source, open_brace, "@init")
}

/// Returns true when an action block belongs to the named ANTLR rule action
/// immediately preceding `open_brace`.
pub(crate) fn is_rule_named_action(source: &str, open_brace: usize, marker: &str) -> bool {
    let prefix = &source[..open_brace];
    let statement_start = prefix.rfind(';').map_or(0, |index| index + 1);
    prefix[statement_start..].trim_end().ends_with(marker)
}

/// Detects target member blocks that are compile-time scaffolding for other
/// runtimes and should not be counted as parser action transitions.
pub(crate) fn is_members_action(source: &str, open_brace: usize) -> bool {
    let prefix = source[..open_brace].trim_end();
    prefix.ends_with("@members") || prefix.ends_with("@parser::members")
}

/// Returns true for target `@definitions` action blocks.
pub(crate) fn is_definitions_action(source: &str, open_brace: usize) -> bool {
    source[..open_brace].trim_end().ends_with("@definitions")
}

/// ANTLR `options { ... }` blocks are grammar metadata, not semantic actions,
/// even though their braces look like empty action transitions to a text scan.
pub(crate) fn is_options_block(source: &str, open_brace: usize) -> bool {
    source[..open_brace].trim_end().ends_with("options")
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
    use super::{matching_action_brace, matching_template_close};

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
