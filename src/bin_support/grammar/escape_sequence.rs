use antlr4_runtime::atn::IntervalSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum EscapeSequenceResult {
    Invalid,
    CodePoint {
        value: i32,
        start: usize,
        stop: usize,
    },
    Property {
        code_points: IntervalSet,
        start: usize,
        stop: usize,
    },
}

pub(super) fn parse_escape(_text: &str, _start: usize) -> EscapeSequenceResult {
    EscapeSequenceResult::Invalid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_point(value: i32, stop: usize) -> EscapeSequenceResult {
        EscapeSequenceResult::CodePoint {
            value,
            start: 0,
            stop,
        }
    }

    fn property(code_points: IntervalSet, stop: usize) -> EscapeSequenceResult {
        EscapeSequenceResult::Property {
            code_points,
            start: 0,
            stop,
        }
    }

    #[test]
    fn parse_empty_matches_java() {
        assert_eq!(parse_escape("", 0), EscapeSequenceResult::Invalid);
    }

    #[test]
    fn parse_just_backslash_matches_java() {
        assert_eq!(parse_escape("\\", 0), EscapeSequenceResult::Invalid);
    }

    #[test]
    fn parse_invalid_escape_matches_java() {
        assert_eq!(parse_escape("\\z", 0), EscapeSequenceResult::Invalid);
    }

    #[test]
    fn parse_newline_matches_java() {
        assert_eq!(parse_escape("\\n", 0), code_point(i32::from(b'\n'), 2));
    }

    #[test]
    fn parse_tab_matches_java() {
        assert_eq!(parse_escape("\\t", 0), code_point(i32::from(b'\t'), 2));
    }

    #[test]
    fn parse_unicode_too_short_matches_java() {
        assert_eq!(parse_escape("\\uABC", 0), EscapeSequenceResult::Invalid);
    }

    #[test]
    fn parse_unicode_bmp_matches_java() {
        assert_eq!(parse_escape("\\uABCD", 0), code_point(0xabcd, 6));
    }

    #[test]
    fn parse_unicode_smp_too_short_matches_java() {
        assert_eq!(parse_escape("\\u{}", 0), EscapeSequenceResult::Invalid);
    }

    #[test]
    fn parse_unicode_smp_missing_close_brace_matches_java() {
        assert_eq!(parse_escape("\\u{12345", 0), EscapeSequenceResult::Invalid);
    }

    #[test]
    fn parse_unicode_too_big_matches_java() {
        assert_eq!(
            parse_escape("\\u{110000}", 0),
            EscapeSequenceResult::Invalid
        );
    }

    #[test]
    fn parse_unicode_smp_matches_java() {
        assert_eq!(parse_escape("\\u{10ABCD}", 0), code_point(0x10_abcd, 10));
    }

    #[test]
    fn parse_unicode_property_too_short_matches_java() {
        assert_eq!(parse_escape("\\p{}", 0), EscapeSequenceResult::Invalid);
    }

    #[test]
    fn parse_unicode_property_missing_close_brace_matches_java() {
        assert_eq!(parse_escape("\\p{1234", 0), EscapeSequenceResult::Invalid);
    }

    #[test]
    fn parse_unicode_property_matches_java() {
        assert_eq!(
            parse_escape("\\p{Deseret}", 0),
            property(IntervalSet::from_range(66_560, 66_639), 11)
        );
    }

    #[test]
    fn parse_unicode_property_inverted_too_short_matches_java() {
        assert_eq!(parse_escape("\\P{}", 0), EscapeSequenceResult::Invalid);
    }

    #[test]
    fn parse_unicode_property_inverted_missing_close_brace_matches_java() {
        assert_eq!(
            parse_escape("\\P{Deseret", 0),
            EscapeSequenceResult::Invalid
        );
    }

    #[test]
    fn parse_unicode_property_inverted_matches_java() {
        let mut expected = IntervalSet::from_range(0, 66_559);
        expected.add_range(66_640, 0x10_ffff);
        assert_eq!(parse_escape("\\P{Deseret}", 0), property(expected, 11));
    }
}
