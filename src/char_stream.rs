use crate::int_stream::{EOF, IntStream, UNKNOWN_SOURCE_NAME};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextInterval {
    pub start: usize,
    pub stop: usize,
}

impl TextInterval {
    pub const fn new(start: usize, stop: usize) -> Self {
        Self { start, stop }
    }

    pub const fn empty() -> Self {
        Self { start: 1, stop: 0 }
    }

    pub const fn is_empty(self) -> bool {
        self.start > self.stop
    }
}

/// Line/column effect of consuming a half-open character span.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PositionSummary {
    /// Number of newline characters in the span.
    pub line_breaks: usize,
    /// Number of characters after the final newline, or the complete span
    /// length when no newline is present.
    pub trailing_columns: usize,
}

impl PositionSummary {
    /// Applies this summary to an existing one-based line and zero-based column.
    pub const fn apply(self, line: usize, column: usize) -> (usize, usize) {
        let line = line.saturating_add(self.line_breaks);
        let column = if self.line_breaks == 0 {
            column.saturating_add(self.trailing_columns)
        } else {
            self.trailing_columns
        };
        (line, column)
    }
}

pub trait CharStream: IntStream {
    fn text(&self, interval: TextInterval) -> String;

    /// Reads one Unicode scalar at an absolute character index without moving
    /// the stream cursor.
    ///
    /// Returning `None` leaves callers on the compatible `seek` + `la`
    /// fallback. Implementations that support immutable access return
    /// [`EOF`] when `index` is outside the input.
    fn symbol_at(&self, _index: usize) -> Option<i32> {
        None
    }

    /// Returns the complete input as ASCII bytes when character and byte
    /// indexes are identical.
    fn contiguous_ascii(&self) -> Option<&[u8]> {
        None
    }

    /// Summarizes source-position changes for the half-open character interval
    /// `[start, end)` without moving the stream cursor.
    ///
    /// Implementations may clamp `end` to the input size. Returning `None`
    /// leaves callers on scalar replay.
    fn position_summary(&self, _start: usize, _end: usize) -> Option<PositionSummary> {
        None
    }

    /// Returns the complete backing UTF-8 source when it can be shared with a
    /// token store.
    ///
    /// Implementations that return `None` remain supported, but their token
    /// text is copied into the store's sparse explicit-text pool.
    fn source_text(&self) -> Option<Rc<str>> {
        None
    }

    fn byte_interval(&self, interval: TextInterval) -> Option<(usize, usize)> {
        self.text_source_interval(interval)
            .map(|(_, start, stop)| (start, stop))
    }

    fn text_source_interval(&self, _interval: TextInterval) -> Option<(Rc<str>, usize, usize)> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct InputStream {
    source: Rc<str>,
    data: InputData,
    cursor: usize,
    source_name: String,
}

#[derive(Clone, Debug)]
enum InputData {
    Ascii,
    Unicode {
        chars: Vec<char>,
        byte_offsets: Vec<usize>,
    },
}

impl InputData {
    fn new(input: &str) -> Self {
        if input.is_ascii() {
            Self::Ascii
        } else {
            Self::Unicode {
                chars: input.chars().collect(),
                byte_offsets: input.char_indices().map(|(index, _)| index).collect(),
            }
        }
    }

    const fn len(&self, source: &str) -> usize {
        match self {
            Self::Ascii => source.len(),
            Self::Unicode { chars, .. } => chars.len(),
        }
    }

    fn get(&self, source: &str, index: usize) -> Option<char> {
        match self {
            Self::Ascii => source.as_bytes().get(index).map(|byte| char::from(*byte)),
            Self::Unicode { chars, .. } => chars.get(index).copied(),
        }
    }

    fn byte_bounds(&self, source: &str, start: usize, stop: usize) -> Option<(usize, usize)> {
        match self {
            Self::Ascii => Some((start, stop + 1)),
            Self::Unicode { byte_offsets, .. } => {
                let start_byte = *byte_offsets.get(start)?;
                let stop_byte = byte_offsets.get(stop + 1).copied().unwrap_or(source.len());
                Some((start_byte, stop_byte))
            }
        }
    }
}

impl InputStream {
    /// Creates a character stream from UTF-8 text using ANTLR's unknown source
    /// name placeholder.
    pub fn new(input: impl AsRef<str>) -> Self {
        Self::with_source_name(input, UNKNOWN_SOURCE_NAME)
    }

    /// Creates a character stream with an explicit source name for tokens and
    /// diagnostics.
    pub fn with_source_name(input: impl AsRef<str>, source_name: impl Into<String>) -> Self {
        let input = input.as_ref();
        Self {
            source: Rc::from(input),
            data: InputData::new(input),
            cursor: 0,
            source_name: source_name.into(),
        }
    }

    /// Returns true when the cursor has reached or passed the end of input.
    pub fn is_eof(&self) -> bool {
        self.cursor >= self.data.len(&self.source)
    }
}

impl IntStream for InputStream {
    fn consume(&mut self) {
        if !self.is_eof() {
            self.cursor += 1;
        }
    }

    fn la(&mut self, offset: isize) -> i32 {
        if offset == 0 {
            return 0;
        }

        let absolute = if offset > 0 {
            self.cursor.checked_add((offset - 1).cast_unsigned())
        } else {
            offset
                .checked_neg()
                .and_then(|distance| usize::try_from(distance).ok())
                .and_then(|distance| self.cursor.checked_sub(distance))
        };

        absolute
            .and_then(|index| self.data.get(&self.source, index))
            .map_or(EOF, |ch| ch as i32)
    }

    fn index(&self) -> usize {
        self.cursor
    }

    fn seek(&mut self, index: usize) {
        self.cursor = index.min(self.data.len(&self.source));
    }

    fn size(&self) -> usize {
        self.data.len(&self.source)
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }
}

impl CharStream for InputStream {
    /// Returns text for an inclusive interval of Unicode scalar indices.
    fn text(&self, interval: TextInterval) -> String {
        if let Some((source, start, stop)) = self.text_source_interval(interval) {
            return source[start..stop].to_owned();
        }
        String::new()
    }

    fn symbol_at(&self, index: usize) -> Option<i32> {
        Some(
            self.data
                .get(&self.source, index)
                .map_or(EOF, |ch| u32::from(ch).cast_signed()),
        )
    }

    fn contiguous_ascii(&self) -> Option<&[u8]> {
        matches!(self.data, InputData::Ascii).then(|| self.source.as_bytes())
    }

    fn position_summary(&self, start: usize, end: usize) -> Option<PositionSummary> {
        let len = self.data.len(&self.source);
        let start = start.min(len);
        let end = end.min(len);
        if start > end {
            return None;
        }

        let mut summary = PositionSummary::default();
        let mut note = |is_newline| {
            if is_newline {
                summary.line_breaks += 1;
                summary.trailing_columns = 0;
            } else {
                summary.trailing_columns += 1;
            }
        };
        match &self.data {
            InputData::Ascii => {
                for &byte in &self.source.as_bytes()[start..end] {
                    note(byte == b'\n');
                }
            }
            InputData::Unicode { chars, .. } => {
                for &ch in &chars[start..end] {
                    note(ch == '\n');
                }
            }
        }
        Some(summary)
    }

    fn text_source_interval(&self, interval: TextInterval) -> Option<(Rc<str>, usize, usize)> {
        let len = self.data.len(&self.source);
        if interval.is_empty() || len == 0 {
            return None;
        }

        let start = interval.start.min(len);
        let stop = interval.stop.min(len.saturating_sub(1));
        if start > stop {
            return None;
        }

        let (start_byte, stop_byte) = self.data.byte_bounds(&self.source, start, stop)?;
        Some((Rc::clone(&self.source), start_byte, stop_byte))
    }

    fn source_text(&self) -> Option<Rc<str>> {
        Some(Rc::clone(&self.source))
    }

    fn byte_interval(&self, interval: TextInterval) -> Option<(usize, usize)> {
        let len = self.data.len(&self.source);
        if interval.is_empty() || len == 0 {
            return None;
        }
        let start = interval.start.min(len);
        let stop = interval.stop.min(len.saturating_sub(1));
        (start <= stop)
            .then(|| self.data.byte_bounds(&self.source, start, stop))
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookahead_and_text_are_codepoint_indexed() {
        let mut input = InputStream::with_source_name("aβ\n", "sample");
        assert_eq!(input.source_name(), "sample");
        assert_eq!(input.size(), 3);
        assert_eq!(input.la(1), 'a' as i32);
        assert_eq!(input.la(2), 'β' as i32);
        assert_eq!(input.text(TextInterval::new(0, 1)), "aβ");
        input.consume();
        assert_eq!(input.index(), 1);
        assert_eq!(input.la(-1), 'a' as i32);
        assert_eq!(input.la(isize::MIN), EOF);
        input.seek(99);
        assert_eq!(input.la(1), EOF);
    }

    #[test]
    fn optional_fast_paths_preserve_scalar_indexes_and_positions() {
        let ascii = InputStream::new("ab\ncd");
        assert_eq!(ascii.contiguous_ascii(), Some(&b"ab\ncd"[..]));
        assert_eq!(ascii.symbol_at(2), Some('\n' as i32));
        assert_eq!(ascii.symbol_at(5), Some(EOF));
        assert_eq!(
            ascii.position_summary(1, 5),
            Some(PositionSummary {
                line_breaks: 1,
                trailing_columns: 2,
            })
        );
        assert_eq!(
            ascii.position_summary(5, 99),
            Some(PositionSummary::default())
        );
        assert_eq!(ascii.position_summary(4, 2), None);

        let unicode = InputStream::new("aβ\nγ");
        assert_eq!(unicode.contiguous_ascii(), None);
        assert_eq!(unicode.symbol_at(1), Some('β' as i32));
        assert_eq!(unicode.symbol_at(4), Some(EOF));
        assert_eq!(
            unicode.position_summary(1, 4),
            Some(PositionSummary {
                line_breaks: 1,
                trailing_columns: 1,
            })
        );
    }

    #[test]
    fn position_summary_applies_to_existing_coordinates() {
        assert_eq!(
            PositionSummary {
                line_breaks: 0,
                trailing_columns: 3,
            }
            .apply(4, 7),
            (4, 10)
        );
        assert_eq!(
            PositionSummary {
                line_breaks: 2,
                trailing_columns: 3,
            }
            .apply(4, 7),
            (6, 3)
        );
    }
}
