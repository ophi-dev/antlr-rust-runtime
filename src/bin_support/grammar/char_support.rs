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
