#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UnicodeEscapeStyle {
    Utf16CodeUnits,
    FixedWidthScalar,
    BracedScalar,
}

pub(super) fn escape_code_point(code_point: i32, style: UnicodeEscapeStyle) -> String {
    assert!(
        (0..=0x10_ffff).contains(&code_point),
        "Unicode code point is out of range"
    );
    match style {
        UnicodeEscapeStyle::Utf16CodeUnits if code_point > 0xffff => {
            let supplementary = code_point - 0x1_0000;
            let high = 0xd800 + (supplementary >> 10);
            let low = 0xdc00 + (supplementary & 0x3ff);
            format!("\\u{high:04X}\\u{low:04X}")
        }
        UnicodeEscapeStyle::Utf16CodeUnits => format!("\\u{code_point:04X}"),
        UnicodeEscapeStyle::FixedWidthScalar if code_point > 0xffff => {
            format!("\\U{code_point:08X}")
        }
        UnicodeEscapeStyle::FixedWidthScalar => format!("\\u{code_point:04X}"),
        UnicodeEscapeStyle::BracedScalar => format!("\\u{{{code_point:04X}}}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_java_escape_matches_java() {
        assert_eq!(
            escape_code_point(0x0061, UnicodeEscapeStyle::Utf16CodeUnits),
            "\\u0061"
        );
    }

    #[test]
    fn latin_python_escape_matches_java() {
        assert_eq!(
            escape_code_point(0x0061, UnicodeEscapeStyle::FixedWidthScalar),
            "\\u0061"
        );
    }

    #[test]
    fn latin_swift_escape_matches_java() {
        assert_eq!(
            escape_code_point(0x0061, UnicodeEscapeStyle::BracedScalar),
            "\\u{0061}"
        );
    }

    #[test]
    fn bmp_java_escape_matches_java() {
        assert_eq!(
            escape_code_point(0xabcd, UnicodeEscapeStyle::Utf16CodeUnits),
            "\\uABCD"
        );
    }

    #[test]
    fn bmp_python_escape_matches_java() {
        assert_eq!(
            escape_code_point(0xabcd, UnicodeEscapeStyle::FixedWidthScalar),
            "\\uABCD"
        );
    }

    #[test]
    fn bmp_swift_escape_matches_java() {
        assert_eq!(
            escape_code_point(0xabcd, UnicodeEscapeStyle::BracedScalar),
            "\\u{ABCD}"
        );
    }

    #[test]
    fn smp_java_escape_matches_java() {
        assert_eq!(
            escape_code_point(0x1f4a9, UnicodeEscapeStyle::Utf16CodeUnits),
            "\\uD83D\\uDCA9"
        );
    }

    #[test]
    fn smp_python_escape_matches_java() {
        assert_eq!(
            escape_code_point(0x1f4a9, UnicodeEscapeStyle::FixedWidthScalar),
            "\\U0001F4A9"
        );
    }

    #[test]
    fn smp_swift_escape_matches_java() {
        assert_eq!(
            escape_code_point(0x1f4a9, UnicodeEscapeStyle::BracedScalar),
            "\\u{1F4A9}"
        );
    }
}
