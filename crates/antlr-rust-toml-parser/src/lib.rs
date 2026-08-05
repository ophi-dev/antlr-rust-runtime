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

        assert!(
            error.to_string().contains("token recognition error"),
            "unexpected diagnostic: {error}"
        );
    }
}
