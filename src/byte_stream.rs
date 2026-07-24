//! A byte-oriented [`CharStream`] for parsing binary formats.
//!
//! ANTLR grammars normally consume Unicode text, but many real-world formats
//! are raw bytes: chunk containers (RIFF/WAV), fixed-width records (tar), and
//! self-describing tag streams (CBOR, Standard MIDI). The reference runtimes
//! parse these by treating each byte as a codepoint in `U+0000..=U+00FF` (a
//! "Latin-1" view) and writing lexer rules over `' '..'ÿ'`.
//!
//! [`InputStream`](crate::InputStream) can do this too, but only after decoding
//! the bytes into a `String`: any byte `>= 0x80` is not valid UTF-8 on its own,
//! so the whole input takes the non-ASCII path and is materialized into a
//! `Vec<char>` plus a byte-offset table — roughly 12 bytes of heap per input
//! byte, and the compiled-DFA ASCII scanner is disabled.
//!
//! `ByteStream` avoids all of that. It is generic over any `AsRef<[u8]>`
//! backing store, so stream index equals byte offset, lookahead is a single
//! array read, and there is no transcoding or auxiliary allocation.
//!
//! # Mapping to Rust IO primitives
//!
//! ANTLR parsing needs random access — the lexer and parser `seek`, look
//! behind with `la(-1)`, and `mark`/`release` for prediction — so the bytes
//! must live fully in memory; `ByteStream` cannot lazily pull from a socket
//! mid-parse. The design instead meets the two IO shapes that matter:
//!
//! - **Bytes you already hold** (a network read buffer, an `mmap`, a slice of a
//!   larger frame): borrow them zero-copy with `ByteStream::new(&buf[..])`.
//!   Nothing is copied; the stream lives as long as the borrow.
//! - **A reader** (`File`, `TcpStream`, `Stdin`, `Cursor`): drain it into an
//!   owned buffer with [`ByteStream::from_reader`], which is just a thin
//!   wrapper over [`std::io::Read::read_to_end`].
//! - **An owned `Vec<u8>`**: hand it over with `ByteStream::new(vec)` and the
//!   stream takes ownership without copying.
//!
//! ```ignore
//! // From a file:
//! let stream = ByteStream::from_reader(std::fs::File::open(path)?)?;
//! // Zero-copy from an in-memory buffer (e.g. bytes read off a socket):
//! let stream = ByteStream::new(&packet[..]);
//!
//! let lexer = MidiLexer::new(stream);
//! let tokens = CommonTokenStream::new(lexer);
//! let mut parser = MidiParser::new(tokens);
//! let tree = parser.file()?;
//! ```
//!
//! Write lexer rules against the byte range, e.g. `BYTE : ' ' .. 'ÿ';`.
//!
//! # Token text is hex
//!
//! Because the bytes are not text, [`CharStream::text`] renders the matched
//! span as a lowercase hex string with no separators (`[0xDE, 0xAD]` becomes
//! `"dead"`). Token *positions* are still exact byte offsets; use
//! [`IntStream::index`](crate::IntStream::index) or a token's byte span when you
//! need to slice the original bytes.

use std::io;

use crate::char_stream::{CharStream, TextInterval};
use crate::int_stream::{EOF, IntStream, UNKNOWN_SOURCE_NAME};

/// A [`CharStream`] backed by raw bytes, where each byte is one symbol in
/// `0..=255` and the stream index is the byte offset.
///
/// Generic over the backing store `B: AsRef<[u8]>`: use `Vec<u8>` for owned
/// bytes or `&[u8]` to borrow an existing buffer zero-copy. See the
/// [module documentation](self) for how this maps onto Rust IO primitives.
#[derive(Clone, Debug)]
pub struct ByteStream<B = Vec<u8>> {
    bytes: B,
    cursor: usize,
    source_name: String,
}

impl ByteStream<Vec<u8>> {
    /// Creates a byte stream by draining a [`std::io::Read`] into an owned
    /// buffer — the bridge for files, sockets, stdin, and [`std::io::Cursor`].
    ///
    /// # Errors
    ///
    /// Returns any error produced while reading `reader` to end.
    pub fn from_reader(mut reader: impl io::Read) -> io::Result<Self> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(Self::new(bytes))
    }
}

impl<B: AsRef<[u8]>> ByteStream<B> {
    /// Creates a byte stream over `bytes`, using ANTLR's unknown source-name
    /// placeholder.
    ///
    /// `bytes` may be an owned `Vec<u8>` or a borrowed `&[u8]` (zero-copy).
    pub fn new(bytes: B) -> Self {
        Self::with_source_name(bytes, UNKNOWN_SOURCE_NAME)
    }

    /// Creates a byte stream with an explicit source name for tokens and
    /// diagnostics.
    pub fn with_source_name(bytes: B, source_name: impl Into<String>) -> Self {
        Self {
            bytes,
            cursor: 0,
            source_name: source_name.into(),
        }
    }

    /// Returns the backing bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// Returns true when the cursor has reached or passed the end of input.
    #[must_use]
    pub fn is_eof(&self) -> bool {
        self.cursor >= self.bytes.as_ref().len()
    }
}

impl<B: AsRef<[u8]>> IntStream for ByteStream<B> {
    fn consume(&mut self) {
        if !self.is_eof() {
            self.cursor += 1;
        }
    }

    fn la(&mut self, offset: isize) -> i32 {
        if offset == 0 {
            return 0;
        }

        // Mirror `InputStream::la`: `+1` is the symbol under the cursor, and
        // negative offsets look behind. `checked_*` keeps `isize::MIN` and
        // out-of-range lookahead on the EOF path instead of panicking.
        let absolute = if offset > 0 {
            self.cursor.checked_add((offset - 1).cast_unsigned())
        } else {
            offset
                .checked_neg()
                .and_then(|distance| usize::try_from(distance).ok())
                .and_then(|distance| self.cursor.checked_sub(distance))
        };

        absolute.map_or(EOF, |index| self.symbol_at(index).unwrap_or(EOF))
    }

    fn index(&self) -> usize {
        self.cursor
    }

    fn seek(&mut self, index: usize) {
        self.cursor = index.min(self.bytes.as_ref().len());
    }

    fn size(&self) -> usize {
        self.bytes.as_ref().len()
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }
}

impl<B: AsRef<[u8]>> CharStream for ByteStream<B> {
    /// Renders the inclusive byte interval as a lowercase, separator-free hex
    /// string. See the [module documentation](self) for the rationale.
    fn text(&self, interval: TextInterval) -> String {
        let bytes = self.bytes.as_ref();
        if interval.is_empty() {
            return String::new();
        }
        let stop = (interval.stop + 1).min(bytes.len());
        let start = interval.start.min(stop);
        use std::fmt::Write as _;
        bytes[start..stop].iter().fold(
            String::with_capacity((stop - start) * 2),
            |mut acc, byte| {
                // Writing to a String is infallible.
                let _ = write!(acc, "{byte:02x}");
                acc
            },
        )
    }

    fn symbol_at(&self, index: usize) -> Option<i32> {
        Some(
            self.bytes
                .as_ref()
                .get(index)
                .map_or(EOF, |&byte| i32::from(byte)),
        )
    }

    // NOTE: `contiguous_ascii` is deliberately NOT implemented. That fast path
    // feeds bytes into a 128-wide ASCII DFA row (`ascii_target`), which is only
    // valid for 7-bit input; bytes `>= 0x80` route correctly through the
    // generic path's `wide_rows` instead.

    fn byte_interval(&self, interval: TextInterval) -> Option<(usize, usize)> {
        // Index == byte offset, so the byte span is exact.
        let len = self.bytes.as_ref().len();
        if interval.is_empty() {
            let at = self.cursor.min(len);
            return Some((at, at));
        }
        let stop = (interval.stop + 1).min(len);
        let start = interval.start.min(stop);
        Some((start, stop))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
mod tests {
    use super::*;

    #[test]
    fn lookahead_reads_bytes_including_high_bytes() {
        let mut stream = ByteStream::new(vec![0x00, 0x7F, 0x80, 0xFF]);
        assert_eq!(stream.la(0), 0, "la(0) is the ANTLR sentinel, not EOF");
        assert_eq!(stream.la(1), 0x00);
        assert_eq!(stream.la(2), 0x7F);
        assert_eq!(stream.la(3), 0x80, "high byte is 128, not sign-extended");
        assert_eq!(stream.la(4), 0xFF);
        assert_eq!(stream.la(5), EOF);
        stream.consume();
        assert_eq!(stream.index(), 1);
        assert_eq!(stream.la(-1), 0x00);
        assert_eq!(stream.la(isize::MIN), EOF, "no panic on extreme offset");
    }

    #[test]
    fn consume_stops_at_eof_and_seek_clamps() {
        let mut stream = ByteStream::new(vec![0x01, 0x02]);
        assert_eq!(stream.size(), 2);
        stream.consume();
        stream.consume();
        stream.consume(); // past EOF is a no-op
        assert_eq!(stream.index(), 2);
        assert!(stream.is_eof());
        stream.seek(99);
        assert_eq!(stream.index(), 2, "seek clamps to size");
        stream.seek(1);
        assert_eq!(stream.la(1), 0x02);
    }

    #[test]
    fn text_is_lowercase_hex_and_byte_interval_is_exact() {
        let stream = ByteStream::new(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(stream.text(TextInterval::new(0, 3)), "deadbeef");
        assert_eq!(stream.text(TextInterval::new(1, 2)), "adbe");
        assert_eq!(stream.text(TextInterval::empty()), "");
        // Inclusive char interval [1, 2] -> half-open byte span [1, 3).
        assert_eq!(stream.byte_interval(TextInterval::new(1, 2)), Some((1, 3)));
        assert_eq!(stream.symbol_at(0), Some(0xDE));
        assert_eq!(stream.symbol_at(4), Some(EOF));
    }

    #[test]
    fn borrows_bytes_zero_copy() {
        // The network-buffer case: parse a slice we already hold without
        // handing ownership to the stream.
        let buffer: [u8; 4] = [0xCA, 0xFE, 0xBA, 0xBE];
        let mut stream = ByteStream::new(&buffer[..]);
        assert_eq!(stream.la(1), 0xCA);
        assert_eq!(stream.size(), 4);
        // `buffer` is still ours afterwards.
        assert_eq!(buffer[0], 0xCA);
    }

    #[test]
    fn from_reader_drains_any_read() {
        // The file/socket case, exercised here with an in-memory Cursor that
        // implements the same `io::Read` contract as `File`/`TcpStream`.
        let source = io::Cursor::new(vec![0x4D, 0x54, 0x68, 0x64]); // "MThd"
        let mut stream = ByteStream::from_reader(source).expect("cursor read is infallible");
        assert_eq!(stream.size(), 4);
        assert_eq!(stream.la(1), 0x4D);
        assert_eq!(stream.text(TextInterval::new(0, 3)), "4d546864");
    }
}
