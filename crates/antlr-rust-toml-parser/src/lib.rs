// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
//! Checked-in TOML recognizers and a decoded syntax facade.
//!
//! This crate is a lockstep implementation dependency of
//! `antlr-rust-codegen`, not a standalone TOML compatibility promise.

mod ast;
mod decode;
mod string;

#[doc(hidden)]
pub mod generated {
    #[doc(hidden)]
    pub mod toml_lexer;
    #[doc(hidden)]
    pub mod toml_parser;
}

pub use ast::{Assignment, Document, Item, Key, TableHeader, Value};
pub use decode::{Error, parse};

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
mod tests {
    use super::parse;

    #[test]
    fn typed_walker_decodes_toml_document_in_source_order() {
        let document = parse(
            r#"
title = "TOML \u0052ules # inside"
literal = 'a\nb'
answer = 0x2a
enabled = true
when = 1979-05-27T07:32:00Z
values = [1, "two", false, { first = "a", second = 2 }]
multiline = """call("x")"""
multiline_pair = """a""b"""
basic_quote_tail = """abc""""
basic_two_quote_tail = """abc"""""
literal_quote_tail = '''abc''''
literal_two_quote_tail = '''abc'''''
lowercase_utc = 1979-05-27T07:32:00z
commented_array = [1 # first
, 2]
float = 1_000.5
owner.name = "Ada"

[standard.table]
value = "kept"

[[helper]]
name = "one"

[[helper]]
name = "two"
"#,
        )
        .expect("valid TOML should parse");

        insta::assert_debug_snapshot!("decoded_toml_document", document);
    }

    #[test]
    fn invalid_toml_reports_a_structured_error_without_recovery() {
        let error = parse("version = 1 trailing\n")
            .expect_err("trailing tokens must not be accepted through parser recovery");

        insta::assert_snapshot!("invalid_toml_diagnostic", error);
    }

    #[test]
    fn invalid_basic_string_escape_is_rejected() {
        let error = parse("value = \"bad\\/escape\"\n")
            .expect_err("JSON-only slash escapes are not valid TOML");

        insta::assert_snapshot!("invalid_toml_escape", error);
    }

    #[test]
    fn drained_lexer_error_still_fails_validated_parse() {
        // The lexer skips `@`; without the retained source-error count, the
        // remaining tokens would form a valid document and validation could
        // incorrectly succeed.
        let error = parse("version = 1\n@\n")
            .expect_err("draining a lexer diagnostic must not make validation succeed");

        insta::assert_snapshot!("invalid_toml_lexer_diagnostic", error);
    }

    #[test]
    fn forbidden_control_character_in_comment_is_rejected() {
        let error = parse("version = 1 # bad\u{0000}\n")
            .expect_err("TOML comments must reject forbidden control characters");

        // A snapshot would embed the raw U+0000 diagnostic byte in the file.
        assert!(
            error.to_string().contains("token recognition error"),
            "unexpected diagnostic: {error}"
        );
    }

    #[test]
    fn multiline_continuation_allows_pre_newline_whitespace() {
        let document = parse("continued = \"\"\"abc\\   \n def\"\"\"\n")
            .expect("continuation whitespace is valid TOML");

        insta::assert_debug_snapshot!("multiline_continuation_whitespace", document);
    }

    #[test]
    fn bare_carriage_return_after_continuation_is_rejected() {
        let error = parse("value = \"\"\"abc\\\n\rdef\"\"\"\n")
            .expect_err("a bare carriage return is not TOML whitespace");

        insta::assert_snapshot!("invalid_multiline_continuation_cr", error);
    }

    #[test]
    fn invalid_date_time_components_are_rejected() {
        let error = parse("when = 1979-13-40T25:61:61+01:99\n")
            .expect_err("date-time components must satisfy TOML ranges");

        insta::assert_snapshot!("invalid_toml_date_time", error);

        for value in [
            "1979-02-30",
            "1979-05-27T24:00:00Z",
            "1979-05-27T23:60:00Z",
            "1979-05-27T23:59:61Z",
            "1979-05-27T23:59:59+01:99",
        ] {
            assert!(
                parse(&format!("when = {value}\n")).is_err(),
                "{value} must be rejected"
            );
        }
    }

    #[test]
    fn toml_1_1_extensions_decode_through_the_generated_parser() {
        let document = parse(concat!(
            "\u{feff}",
            r#"
escaped = "\e\x41"
local_time = 13:37
offset_date_time = 1979-05-27 07:32z
inline = {
    key = 1,
}
"#,
            "basic_zwnbsp = \"x\u{feff}y\"\n",
            "literal_zwnbsp = 'x\u{feff}y'\n",
            "multiline_zwnbsp = \"\"\"x\u{feff}y\"\"\"\n",
            "commented_zwnbsp = 1 # x\u{feff}y\n",
        ))
        .expect("TOML 1.1 extensions should parse");

        insta::assert_debug_snapshot!("toml_1_1_extensions", document);
    }

    #[test]
    fn interior_utf8_bom_is_rejected() {
        let error = parse("first = 1\n\u{feff}second = 2\n")
            .expect_err("UTF-8 BOM is valid only at the start");

        insta::assert_snapshot!("invalid_interior_utf8_bom", error);
    }

    #[test]
    fn hard_parse_failure_preserves_the_lexer_diagnostic() {
        let error = parse("a = 1\rb = 2\n").expect_err("a bare carriage return must fail lexing");

        insta::assert_snapshot!("hard_failure_lexer_diagnostic", error);
    }

    #[test]
    fn hard_parse_failure_preserves_the_parser_diagnostic() {
        let error = parse("a = 1 b = 2\n").expect_err("two pairs on one line must fail");

        insta::assert_snapshot!("hard_failure_parser_diagnostic", error);
    }

    #[test]
    fn float_payload_is_directly_parseable() {
        let document = parse("value = 1_000.5\n").expect("float should parse");
        let [crate::Item::Assignment(assignment)] = document.items() else {
            panic!("expected one assignment");
        };
        let crate::Value::Float(value) = assignment.value() else {
            panic!("expected a float");
        };

        insta::assert_snapshot!("normalized_float_payload", value);
        value.parse::<f64>().expect("normalized float should parse");
    }
}
