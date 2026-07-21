#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UnicodeEscapeStyle {
    Utf16CodeUnits,
    FixedWidthScalar,
    BracedScalar,
}

pub(super) fn escape_code_point(_code_point: i32, _style: UnicodeEscapeStyle) -> String {
    String::new()
}
