/// A brace-delimited ANTLR action block recognized by the metadata generator
/// and runtime-testsuite harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TemplateBlock<'a> {
    pub(crate) open_brace: usize,
    pub(crate) body: &'a str,
    pub(crate) after_brace: usize,
    pub(crate) predicate: bool,
}

/// Finds the next target-template block while allowing whitespace inside the
/// ANTLR action braces, for example `{ <writeln("$text")> }`.
#[cfg(test)]
pub(crate) fn next_template_block(source: &str, offset: usize) -> Option<TemplateBlock<'_>> {
    let mut cursor = offset;
    while let Some(open_brace) = find_significant_open_brace(source, cursor) {
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
    while let Some(open_brace) = find_significant_open_brace(source, cursor) {
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
    while let Some(open_brace) = find_significant_open_brace(source, cursor) {
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

/// Finds the next `{` that is structurally significant grammar text.
///
/// Grammar rules routinely reference brace TOKENS as quoted literals
/// (`'{' statementList? '}'`) and mention braces in comments; a naive
/// `find('{')` desynchronizes every action/predicate walk on such grammars.
/// All block walkers locate their opening brace through this scanner so the
/// walk only ever starts at real action-block braces.
pub(crate) fn find_significant_open_brace(source: &str, offset: usize) -> Option<usize> {
    let mut cursor = GrammarSourceCursor::new(source, offset);
    while let Some((index, ch)) = cursor.next_significant() {
        if ch == '{' {
            return Some(index);
        }
    }
    None
}

/// Lexical cursor over ANTLR grammar source that skips line and block
/// comments, string literals, and `[...]` character sets, yielding only
/// characters that are significant to grammar structure.
///
/// Action extraction, predicate extraction, and rule-header scanning need the
/// same skip rules; sharing one state machine keeps them from drifting apart.
pub(crate) struct GrammarSourceCursor<'a> {
    source: &'a str,
    index: usize,
    single_quoted: bool,
    double_quoted: bool,
    escaped: bool,
    line_comment: bool,
    block_comment: bool,
    char_set: bool,
}

impl<'a> GrammarSourceCursor<'a> {
    pub(crate) const fn new(source: &'a str, offset: usize) -> Self {
        Self {
            source,
            index: offset,
            single_quoted: false,
            double_quoted: false,
            escaped: false,
            line_comment: false,
            block_comment: false,
            char_set: false,
        }
    }

    /// Moves the cursor to `index`, which must be a char boundary outside any
    /// comment, string literal, or character set.
    ///
    /// Only the generator's rule-header scanner needs this; the testsuite
    /// harness compiles this module too, so the method is allowed to be
    /// unused there.
    #[allow(dead_code)]
    pub(crate) const fn seek(&mut self, index: usize) {
        self.index = index;
    }

    /// Returns the next structurally significant character with its byte
    /// offset, consuming it.
    pub(crate) fn next_significant(&mut self) -> Option<(usize, char)> {
        while let Some(ch) = self.source[self.index..].chars().next() {
            let index = self.index;
            let size = ch.len_utf8();
            if self.consume_skipped(ch, size) {
                continue;
            }
            match ch {
                '/' if self.source.as_bytes().get(index..index + 2) == Some(b"//") => {
                    self.line_comment = true;
                    self.index += 2;
                }
                '/' if self.source.as_bytes().get(index..index + 2) == Some(b"/*") => {
                    self.block_comment = true;
                    self.index += 2;
                }
                '\'' => {
                    self.single_quoted = true;
                    self.index += size;
                }
                '"' => {
                    self.double_quoted = true;
                    self.index += size;
                }
                '[' => {
                    self.char_set = true;
                    self.index += size;
                }
                _ => {
                    self.index += size;
                    return Some((index, ch));
                }
            }
        }
        None
    }

    /// Consumes one character belonging to an active comment, string, or
    /// character-set region; false when the cursor is at top level.
    fn consume_skipped(&mut self, ch: char, size: usize) -> bool {
        if self.line_comment {
            self.line_comment = ch != '\n';
            self.index += size;
            return true;
        }
        if self.block_comment {
            if self.source.as_bytes().get(self.index..self.index + 2) == Some(b"*/") {
                self.block_comment = false;
                self.index += 2;
            } else {
                self.index += size;
            }
            return true;
        }
        if self.char_set {
            match ch {
                _ if self.escaped => self.escaped = false,
                '\\' => self.escaped = true,
                ']' => self.char_set = false,
                _ => {}
            }
            self.index += size;
            return true;
        }
        if self.escaped {
            self.escaped = false;
            self.index += size;
            return true;
        }
        if self.single_quoted {
            match ch {
                '\\' => self.escaped = true,
                '\'' => self.single_quoted = false,
                _ => {}
            }
            self.index += size;
            return true;
        }
        if self.double_quoted {
            match ch {
                '\\' => self.escaped = true,
                '"' => self.double_quoted = false,
                _ => {}
            }
            self.index += size;
            return true;
        }
        false
    }
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
    use super::{
        matching_action_brace, matching_template_close, next_predicate_action_block,
        next_template_block,
    };

    #[test]
    fn predicate_block_found_after_quoted_brace_literals() {
        // Mirrors grammars-v4 JavaScriptParser: brace TOKENS appear as quoted
        // literals before the first semantic predicate.
        let source = "block : '{' statementList? '}' ;\n\
                      stmt : {this.notLineTerminator()}? expr ;\n";

        let block = next_predicate_action_block(source, 0).expect("predicate block is found");
        assert_eq!(block.body, "this.notLineTerminator()");
        assert!(block.predicate);
        assert!(
            next_predicate_action_block(source, block.after_brace).is_none(),
            "quoted brace literals must not produce phantom blocks"
        );
    }

    #[test]
    fn predicate_block_found_after_commented_brace() {
        let source = "// a { comment with a brace\n\
                      /* and { another } one */\n\
                      stmt : {this.closeBrace()}? expr ;\n";

        let block = next_predicate_action_block(source, 0).expect("predicate block is found");
        assert_eq!(block.body, "this.closeBrace()");
    }

    #[test]
    fn template_block_skips_quoted_brace_literals() {
        let source = "a : '{' {<True()>}? 'b' ;";

        let block = next_template_block(source, 0).expect("template block is found");
        assert_eq!(block.body, "True()");
        assert!(block.predicate);
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
