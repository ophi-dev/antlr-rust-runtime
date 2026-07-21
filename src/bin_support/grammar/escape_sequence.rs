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
