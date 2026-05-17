use crate::int_stream::{EOF, IntStream, UNKNOWN_SOURCE_NAME};

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

pub trait CharStream: IntStream {
    fn text(&self, interval: TextInterval) -> String;
}

#[derive(Clone, Debug)]
pub struct InputStream {
    data: Vec<char>,
    cursor: usize,
    source_name: String,
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
        Self {
            data: input.as_ref().chars().collect(),
            cursor: 0,
            source_name: source_name.into(),
        }
    }

    /// Returns true when the cursor has reached or passed the end of input.
    pub const fn is_eof(&self) -> bool {
        self.cursor >= self.data.len()
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
            .and_then(|index| self.data.get(index).copied())
            .map_or(EOF, |ch| ch as i32)
    }

    fn index(&self) -> usize {
        self.cursor
    }

    fn seek(&mut self, index: usize) {
        self.cursor = index.min(self.data.len());
    }

    fn size(&self) -> usize {
        self.data.len()
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }
}

impl CharStream for InputStream {
    /// Returns text for an inclusive interval of Unicode scalar indices.
    fn text(&self, interval: TextInterval) -> String {
        if interval.is_empty() || self.data.is_empty() {
            return String::new();
        }

        let start = interval.start.min(self.data.len());
        let stop = interval.stop.min(self.data.len().saturating_sub(1));
        if start > stop {
            return String::new();
        }

        self.data[start..=stop].iter().collect()
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
}
