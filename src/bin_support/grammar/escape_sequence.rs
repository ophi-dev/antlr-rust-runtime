use antlr4_runtime::atn::IntervalSet;

use super::unicode::property_ranges;

const MAX_CODE_POINT: i32 = 0x10_ffff;

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

pub(super) fn parse_escape(text: &str, start: usize) -> EscapeSequenceResult {
    let Some(tail) = text.get(start..) else {
        return EscapeSequenceResult::Invalid;
    };
    let mut characters = tail.char_indices();
    if characters.next().is_none_or(|(_, value)| value != '\\') {
        return EscapeSequenceResult::Invalid;
    }
    let Some((escaped_start, escaped)) = characters.next() else {
        return EscapeSequenceResult::Invalid;
    };
    let cursor = escaped_start + escaped.len_utf8();
    match escaped {
        'u' => parse_unicode_escape(tail, start, cursor),
        'p' | 'P' => parse_property_escape(tail, start, cursor, escaped == 'P'),
        _ => simple_escape(escaped).map_or(EscapeSequenceResult::Invalid, |value| {
            EscapeSequenceResult::CodePoint {
                value,
                start,
                stop: start + cursor,
            }
        }),
    }
}

fn parse_unicode_escape(tail: &str, start: usize, cursor: usize) -> EscapeSequenceResult {
    if cursor + 3 > tail.len() {
        return EscapeSequenceResult::Invalid;
    }
    let (digits, stop) = if tail.as_bytes().get(cursor) == Some(&b'{') {
        let digits_start = cursor + 1;
        let Some(close) = tail[digits_start..].find('}') else {
            return EscapeSequenceResult::Invalid;
        };
        let close = digits_start + close;
        (&tail[digits_start..close], close + 1)
    } else {
        let Some(digits) = tail.get(cursor..cursor + 4) else {
            return EscapeSequenceResult::Invalid;
        };
        (digits, cursor + 4)
    };
    let Ok(value) = i32::from_str_radix(digits, 16) else {
        return EscapeSequenceResult::Invalid;
    };
    if value > MAX_CODE_POINT {
        return EscapeSequenceResult::Invalid;
    }
    EscapeSequenceResult::CodePoint {
        value,
        start,
        stop: start + stop,
    }
}

fn parse_property_escape(
    tail: &str,
    start: usize,
    cursor: usize,
    inverted: bool,
) -> EscapeSequenceResult {
    if cursor + 3 > tail.len() || tail.as_bytes().get(cursor) != Some(&b'{') {
        return EscapeSequenceResult::Invalid;
    }
    let name_start = cursor + 1;
    let Some(close) = tail[name_start..].find('}') else {
        return EscapeSequenceResult::Invalid;
    };
    let close = name_start + close;
    let Some(ranges) =
        property_ranges(&tail[name_start..close]).filter(|ranges| !ranges.is_empty())
    else {
        return EscapeSequenceResult::Invalid;
    };
    let code_points = if inverted {
        complement(ranges)
    } else {
        interval_set(ranges)
    };
    EscapeSequenceResult::Property {
        code_points,
        start,
        stop: start + close + 1,
    }
}

fn simple_escape(escaped: char) -> Option<i32> {
    match escaped {
        'n' => Some(i32::from(b'\n')),
        'r' => Some(i32::from(b'\r')),
        't' => Some(i32::from(b'\t')),
        'b' => Some(i32::from(b'\x08')),
        'f' => Some(i32::from(b'\x0c')),
        '\\' => Some(i32::from(b'\\')),
        ']' => Some(i32::from(b']')),
        '-' => Some(i32::from(b'-')),
        _ => None,
    }
}

fn interval_set(ranges: &[i32]) -> IntervalSet {
    let mut result = IntervalSet::new();
    for range in ranges.chunks_exact(2) {
        result.add_range(range[0], range[1]);
    }
    result
}

fn complement(ranges: &[i32]) -> IntervalSet {
    let mut result = IntervalSet::new();
    let mut next = 0;
    for range in ranges.chunks_exact(2) {
        if next < range[0] {
            result.add_range(next, range[0] - 1);
        }
        next = range[1] + 1;
    }
    if next <= MAX_CODE_POINT {
        result.add_range(next, MAX_CODE_POINT);
    }
    result
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
