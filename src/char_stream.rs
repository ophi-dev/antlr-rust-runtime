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

pub trait CharStream: IntStream {
    fn text(&self, interval: TextInterval) -> String;

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
}
