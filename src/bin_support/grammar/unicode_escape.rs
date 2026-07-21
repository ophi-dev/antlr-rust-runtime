#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UnicodeEscapeStyle {
    Utf16CodeUnits,
    FixedWidthScalar,
    BracedScalar,
}

pub(super) fn escape_code_point(_code_point: i32, _style: UnicodeEscapeStyle) -> String {
    String::new()
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
