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
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = body[index + 2..]
                    .find('\n')
                    .map_or(bytes.len(), |newline| index + 2 + newline + 1);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = body[index + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |close| index + 2 + close + 2);
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
}
