#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActionReference<'a> {
    pub(crate) kind: ActionReferenceKind<'a>,
    pub(crate) expression: &'a str,
    pub(crate) name_offset: usize,
    pub(crate) attribute_offset: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionReferenceKind<'a> {
    Attribute { name: &'a str, assignment: bool },
    Qualified { name: &'a str, attribute: &'a str },
    NonLocal { rule: &'a str, attribute: &'a str },
}

pub(crate) fn action_references(body: &str) -> Vec<ActionReference<'_>> {
    let mut references = Vec::new();
    collect_references(body, 0, &mut references);
    references
}

fn collect_references<'a>(
    body: &'a str,
    base_offset: usize,
    references: &mut Vec<ActionReference<'a>>,
) {
    let bytes = body.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = macro_rules_definition_end(body, index) {
            index = end;
            continue;
        }
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = body[index + 2..]
                    .find('\n')
                    .map_or(bytes.len(), |newline| index + 2 + newline + 1);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(body, index);
            }
            b'\\' => {
                index += 1;
                if index < bytes.len() {
                    index += next_char_len(body, index);
                }
            }
            b'$' if bytes
                .get(index + 1)
                .is_some_and(|byte| is_identifier_start(*byte)) =>
            {
                index += parse_reference(body, index, base_offset, references);
            }
            _ => index += next_char_len(body, index),
        }
    }
}

fn macro_rules_definition_end(body: &str, start: usize) -> Option<usize> {
    const PREFIX: &str = "macro_rules";
    if !body[start..].starts_with(PREFIX)
        || start
            .checked_sub(1)
            .and_then(|before| body.as_bytes().get(before))
            .is_some_and(|byte| is_identifier_continue(*byte))
        || body
            .as_bytes()
            .get(start + PREFIX.len())
            .is_some_and(|byte| is_identifier_continue(*byte))
    {
        return None;
    }
    let bytes = body.as_bytes();
    let mut cursor = skip_rust_trivia(body, start + PREFIX.len());
    if bytes.get(cursor) != Some(&b'!') {
        return None;
    }
    cursor = skip_rust_trivia(body, cursor + 1);
    let name_start = if bytes.get(cursor..cursor + 2) == Some(b"r#") {
        cursor + 2
    } else {
        cursor
    };
    cursor = skip_rust_trivia(
        body,
        crate::rust_names::rust_identifier_end(body, name_start)?,
    );
    let expected = match bytes.get(cursor)? {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        _ => return None,
    };
    balanced_token_tree_end(body, cursor, expected)
}

fn balanced_token_tree_end(body: &str, open: usize, expected: u8) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut stack = vec![expected];
    let mut index = open + 1;
    while index < bytes.len() {
        if let Some(end) = raw_string_end(body, index) {
            index = end;
            continue;
        }
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = body[index + 2..]
                    .find('\n')
                    .map_or(bytes.len(), |newline| index + 2 + newline + 1);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = block_comment_end(body, index);
            }
            b'\'' => {
                index = char_literal_end(body, index).unwrap_or(index + 1);
            }
            quote @ (b'"' | b'`') => {
                index = quoted_end(body, index, quote);
            }
            b'(' => {
                stack.push(b')');
                index += 1;
            }
            b'[' => {
                stack.push(b']');
                index += 1;
            }
            b'{' => {
                stack.push(b'}');
                index += 1;
            }
            close @ (b')' | b']' | b'}') if stack.last() == Some(&close) => {
                stack.pop();
                index += 1;
                if stack.is_empty() {
                    return Some(index);
                }
            }
            _ => index += next_char_len(body, index),
        }
    }
    None
}

fn raw_string_end(body: &str, start: usize) -> Option<usize> {
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

fn block_comment_end(body: &str, open: usize) -> usize {
    let bytes = body.as_bytes();
    let mut depth = 1_usize;
    let mut index = open + 2;
    while index + 1 < bytes.len() {
        match &bytes[index..index + 2] {
            b"/*" => {
                depth += 1;
                index += 2;
            }
            b"*/" => {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }
    bytes.len()
}

fn char_literal_end(body: &str, open: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let content = open + 1;
    let end = match bytes.get(content)? {
        b'\\' => match bytes.get(content + 1)? {
            b'x' => content + 4,
            b'u' if bytes.get(content + 2) == Some(&b'{') => {
                let close = body[content + 3..].find('}')? + content + 3;
                close + 1
            }
            _ => content + 1 + next_char_len(body, content + 1),
        },
        _ => content + next_char_len(body, content),
    };
    (bytes.get(end) == Some(&b'\'')).then_some(end + 1)
}

fn quoted_end(body: &str, open: usize, quote: u8) -> usize {
    let bytes = body.as_bytes();
    let mut index = open + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = (index + 2).min(bytes.len()),
            byte if byte == quote => return index + 1,
            _ => index += next_char_len(body, index),
        }
    }
    bytes.len()
}

fn parse_reference<'a>(
    body: &'a str,
    dollar: usize,
    base_offset: usize,
    references: &mut Vec<ActionReference<'a>>,
) -> usize {
    let name_start = dollar + 1;
    let name_end = identifier_end(body.as_bytes(), name_start);
    let name = &body[name_start..name_end];

    if body[name_end..].starts_with("::") {
        let attribute_start = name_end + 2;
        if body
            .as_bytes()
            .get(attribute_start)
            .is_some_and(|byte| is_identifier_start(*byte))
        {
            let attribute_end = identifier_end(body.as_bytes(), attribute_start);
            let expression_end =
                assignment(body, attribute_end).map_or(attribute_end, |value| value.end);
            references.push(ActionReference {
                kind: ActionReferenceKind::NonLocal {
                    rule: name,
                    attribute: &body[attribute_start..attribute_end],
                },
                expression: &body[dollar..expression_end],
                name_offset: base_offset + name_start,
                attribute_offset: Some(base_offset + attribute_start),
            });
            return expression_end - dollar;
        }
    }

    if body[name_end..].starts_with('.') {
        let attribute_start = name_end + 1;
        if body
            .as_bytes()
            .get(attribute_start)
            .is_some_and(|byte| is_identifier_start(*byte))
        {
            let attribute_end = identifier_end(body.as_bytes(), attribute_start);
            if body.as_bytes().get(attribute_end) != Some(&b'(') {
                references.push(ActionReference {
                    kind: ActionReferenceKind::Qualified {
                        name,
                        attribute: &body[attribute_start..attribute_end],
                    },
                    expression: &body[dollar..attribute_end],
                    name_offset: base_offset + name_start,
                    attribute_offset: Some(base_offset + attribute_start),
                });
                return attribute_end - dollar;
            }
        }
    }

    if let Some(assignment) = assignment(body, name_end) {
        references.push(ActionReference {
            kind: ActionReferenceKind::Attribute {
                name,
                assignment: true,
            },
            expression: &body[dollar..assignment.end],
            name_offset: base_offset + name_start,
            attribute_offset: None,
        });
        collect_references(
            &body[assignment.rhs_start..assignment.rhs_end],
            base_offset + assignment.rhs_start,
            references,
        );
        return assignment.end - dollar;
    }

    references.push(ActionReference {
        kind: ActionReferenceKind::Attribute {
            name,
            assignment: false,
        },
        expression: &body[dollar..name_end],
        name_offset: base_offset + name_start,
        attribute_offset: None,
    });
    name_end - dollar
}

#[derive(Clone, Copy)]
struct Assignment {
    rhs_start: usize,
    rhs_end: usize,
    end: usize,
}

fn assignment(body: &str, operand_end: usize) -> Option<Assignment> {
    let bytes = body.as_bytes();
    let equals = skip_whitespace(bytes, operand_end);
    if bytes.get(equals) != Some(&b'=') || bytes.get(equals + 1) == Some(&b'=') {
        return None;
    }
    let rhs_start = equals + 1;
    let rhs_end = body[rhs_start..]
        .char_indices()
        .skip(1)
        .find_map(|(offset, character)| (character == ';').then_some(rhs_start + offset))?;
    Some(Assignment {
        rhs_start,
        rhs_end,
        end: rhs_end + 1,
    })
}

fn identifier_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start + 1;
    while bytes
        .get(end)
        .is_some_and(|byte| is_identifier_continue(*byte))
    {
        end += 1;
    }
    end
}

const fn is_identifier_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

const fn is_identifier_continue(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    index
}

fn skip_rust_trivia(body: &str, mut index: usize) -> usize {
    let bytes = body.as_bytes();
    loop {
        index = skip_whitespace(bytes, index);
        if bytes.get(index..index + 2) == Some(b"//") {
            index = body[index + 2..]
                .find('\n')
                .map_or(bytes.len(), |newline| index + 2 + newline + 1);
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            index = block_comment_end(body, index);
        } else {
            return index;
        }
    }
}

fn next_char_len(text: &str, index: usize) -> usize {
    text[index..].chars().next().map_or(1, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_references_and_tracks_identifier_offsets() {
        let body = "x $value $rule.result $scope::item";
        let references = action_references(body);
        assert_eq!(
            references,
            [
                ActionReference {
                    kind: ActionReferenceKind::Attribute {
                        name: "value",
                        assignment: false,
                    },
                    expression: "$value",
                    name_offset: 3,
                    attribute_offset: None,
                },
                ActionReference {
                    kind: ActionReferenceKind::Qualified {
                        name: "rule",
                        attribute: "result",
                    },
                    expression: "$rule.result",
                    name_offset: 10,
                    attribute_offset: Some(15),
                },
                ActionReference {
                    kind: ActionReferenceKind::NonLocal {
                        rule: "scope",
                        attribute: "item",
                    },
                    expression: "$scope::item",
                    name_offset: 23,
                    attribute_offset: Some(30),
                },
            ],
        );
    }

    #[test]
    fn assignments_follow_action_splitter_rhs_rules() {
        let references = action_references("$q = $blort; $S::j = $S::k; $S::i=$S::i");
        assert_eq!(
            references
                .iter()
                .map(|reference| reference.expression)
                .collect::<Vec<_>>(),
            ["$q = $blort;", "$blort", "$S::j = $S::k;", "$S::i", "$S::i",],
        );
        assert!(matches!(
            references[0].kind,
            ActionReferenceKind::Attribute {
                assignment: true,
                ..
            }
        ));
    }

    #[test]
    fn indexed_nonlocals_and_method_calls_start_as_simple_attributes() {
        let references = action_references("$Q[-1]::y $S[$S::y]::i $ID.getText()");
        assert_eq!(
            references
                .iter()
                .map(|reference| reference.expression)
                .collect::<Vec<_>>(),
            ["$Q", "$S", "$S::y", "$ID"],
        );
    }

    #[test]
    fn escaped_dollars_and_comments_are_text() {
        let references = action_references("\\$x /* $y */ // $z\n$ok");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].expression, "$ok");
    }

    #[test]
    fn macro_rules_metavariables_are_target_syntax() {
        let body = "macro_rules! value { ($i:ident) => { $i } }\n$actual";
        let references = action_references(body);

        assert_eq!(references.len(), 1);
        assert_eq!(references[0].expression, "$actual");
    }

    #[test]
    fn macro_rules_bodies_skip_lifetimes_and_quoted_delimiters() {
        for body in [
            "macro_rules! m { ($t:ty) => { fn f<'a>(v: &'a $t) {} } }\n$actual",
            "macro_rules! r#match { ($i:ident) => { $i } }\n$actual",
            "macro_rules! λ { ($i:ident) => { $i } }\n$actual",
            "macro_rules! r#λ { ($i:ident) => { $i } }\n$actual",
            "macro_rules /* keyword */ ! /* bang */ value /* name */ \
             { ($i:ident) => { $i } }\n$actual",
            r#"macro_rules! m { ($t:ty) => {{ let _ = "{ $t"; /* } $t */ }} }
$actual"#,
            r#"macro_rules! m { ($t:ty) => {{ /* outer /* inner */ } $ignored */ let _: $t; }} }
$actual"#,
            r"macro_rules! m { ($t:ty) => { let _ = '('; let _ = '\''; } }
$actual",
            r##"macro_rules! m { ($i:ident) => {{ let _ = r#""} $ignored"#; $i }} }
$actual"##,
            r##"macro_rules! m { ($i:ident) => {{ let _ = br#""} $ignored"#; $i }} }
$actual"##,
            r##"macro_rules! m { ($i:ident) => {{ let _ = cr#""} $ignored"#; $i }} }
$actual"##,
        ] {
            let references = action_references(body);
            assert_eq!(references.len(), 1, "{body}");
            assert_eq!(references[0].expression, "$actual", "{body}");
        }
    }

    #[test]
    fn unterminated_macro_rules_token_trees_are_not_treated_as_complete() {
        let body = "macro_rules! value { ($i:ident) => { $i }";

        assert_eq!(macro_rules_definition_end(body, 0), None);
    }
}
