// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use icu_properties::{CodePointSetData, CodePointSetDataBorrowed, props};

/// Returns the byte end of a Rust identifier beginning at `start`.
#[allow(dead_code)] // This shared module is also compiled by the conformance harness.
pub(crate) fn rust_identifier_end(value: &str, start: usize) -> Option<usize> {
    static XID_START: CodePointSetDataBorrowed<'static> =
        CodePointSetData::new::<props::XidStart>();
    static XID_CONTINUE: CodePointSetDataBorrowed<'static> =
        CodePointSetData::new::<props::XidContinue>();

    let mut chars = value.get(start..)?.char_indices();
    let (_, first) = chars.next()?;
    if first != '_' && !XID_START.contains(first) {
        return None;
    }
    let mut end = start + first.len_utf8();
    for (relative, ch) in chars {
        if !XID_CONTINUE.contains(ch) {
            break;
        }
        end = start + relative + ch.len_utf8();
    }
    Some(end)
}

/// Converts a grammar type name into a snake-case module file name.
pub(crate) fn module_name(name: &str) -> String {
    split_identifier_words(name).join("_")
}

/// Converts an ANTLR grammar name into a Rust type name.
pub(crate) fn rust_type_name(name: &str) -> String {
    split_identifier_words(name)
        .into_iter()
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                let mut out = String::with_capacity(part.len());
                out.push(first.to_ascii_uppercase());
                out.push_str(chars.as_str());
                out
            })
        })
        .collect()
}

/// Converts an ANTLR rule name into a snake-case Rust method name.
pub(crate) fn rust_function_name(name: &str) -> String {
    let words = split_identifier_words(name);
    let ident = if words.is_empty() {
        "rule".to_owned()
    } else {
        words.join("_")
    };
    rust_identifier(&ident)
}

/// Escapes a Rust string literal using explicit ASCII escape forms.
pub(crate) fn rust_string(value: &str) -> String {
    value.escape_default().to_string()
}

/// Replaces every non-overlapping occurrence without relying on the
/// allocation-hiding `str::replace` helper prohibited by the workspace lints.
pub(crate) fn replace_all(text: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(needle) {
        out.push_str(&rest[..index]);
        out.push_str(replacement);
        rest = &rest[index + needle.len()..];
    }
    out.push_str(rest);
    out
}

/// Splits mixed-case, snake-case, and punctuation-heavy grammar identifiers
/// into words for Rust identifier rendering.
pub(crate) fn split_identifier_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = name.chars().collect();
    for (index, ch) in chars.iter().copied().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(ascii_lowercase(&current));
                current.clear();
            }
            continue;
        }

        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        let starts_new_word = !current.is_empty()
            && ch.is_ascii_uppercase()
            && (previous.is_some_and(|prev| prev.is_ascii_lowercase() || prev.is_ascii_digit())
                || (previous.is_some_and(|prev| prev.is_ascii_uppercase())
                    && next.is_some_and(|next| next.is_ascii_lowercase())));

        if starts_new_word {
            words.push(ascii_lowercase(&current));
            current.clear();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(ascii_lowercase(&current));
    }
    words
}

/// Produces a legal Rust identifier and leaves keyword handling to callers that
/// know whether raw identifiers are valid at the target position.
pub(crate) fn sanitize_identifier(value: &str) -> String {
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            if index == 0 && ch.is_ascii_digit() {
                out.push('_');
            }
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "_".to_owned() } else { out }
}

/// Produces a legal Rust identifier, using raw syntax only for keywords that
/// permit it. Path keywords cannot be raw identifiers, so suffix those names.
pub(crate) fn rust_identifier(value: &str) -> String {
    let identifier = sanitize_identifier(value);
    if matches!(identifier.as_str(), "crate" | "self" | "Self" | "super") {
        format!("{identifier}_")
    } else if is_rust_keyword(&identifier) {
        format!("r#{identifier}")
    } else {
        identifier
    }
}

/// Returns true for Rust reserved and contextual keywords that cannot be used
/// directly as generated identifiers.
pub(crate) fn is_rust_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "gen"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "Self"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "try"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
    )
}

/// Converts ASCII letters to lower case without using allocation-hiding string
/// case helpers disallowed by the strict Clippy policy.
fn ascii_lowercase(value: &str) -> String {
    value.chars().map(|ch| ch.to_ascii_lowercase()).collect()
}
