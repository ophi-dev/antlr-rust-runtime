use crate::int_stream::{EOF, IntStream, UNKNOWN_SOURCE_NAME};
use std::rc::Rc;

use crate::token::{
    CommonToken, DEFAULT_CHANNEL, TOKEN_EOF, Token, TokenRef, TokenSource, TokenSourceError,
};

#[derive(Debug)]
pub struct CommonTokenStream<S> {
    source: S,
    tokens: Vec<TokenRef>,
    public_tokens: Vec<CommonToken>,
    next_visible_after: Vec<usize>,
    cursor: usize,
    fetched_eof: bool,
    channel: i32,
    source_errors: Vec<TokenSourceError>,
}

const UNKNOWN_NEXT_VISIBLE: usize = usize::MAX;

impl<S> CommonTokenStream<S>
where
    S: TokenSource,
{
    /// Creates a token stream that filters lookahead to the default channel.
    pub const fn new(source: S) -> Self {
        Self::with_channel(source, DEFAULT_CHANNEL)
    }

    /// Creates a token stream whose `LT/LA` operations see only `channel`.
    pub const fn with_channel(source: S, channel: i32) -> Self {
        Self {
            source,
            tokens: Vec::new(),
            public_tokens: Vec::new(),
            next_visible_after: Vec::new(),
            cursor: 0,
            fetched_eof: false,
            channel,
            source_errors: Vec::new(),
        }
    }

    /// Reads tokens from the source until EOF is buffered.
    pub fn fill(&mut self) {
        while !self.fetched_eof {
            self.fetch_one();
        }
        self.cursor = self.adjust_seek_index(self.cursor);
    }

    /// Returns the token at an absolute buffered index, fetching from the source
    /// as needed.
    pub fn get(&mut self, index: usize) -> Option<&CommonToken> {
        self.sync(index);
        self.tokens.get(index).map(Rc::as_ref)
    }

    /// Returns a shared reference-counted token at an absolute buffered index.
    pub fn get_ref(&mut self, index: usize) -> Option<TokenRef> {
        self.sync(index);
        self.tokens.get(index).map(Rc::clone)
    }

    /// Returns the token at one-based lookahead/lookbehind offset, skipping
    /// tokens outside the configured channel for positive offsets.
    pub fn lt(&mut self, offset: isize) -> Option<&CommonToken> {
        if offset == 0 {
            return None;
        }
        if offset < 0 {
            return offset
                .checked_neg()
                .map(isize::cast_unsigned)
                .and_then(|offset| self.lb(offset));
        }

        let mut index = self.next_token_on_channel(self.cursor, self.channel);
        let mut remaining = offset;
        while remaining > 1 {
            index = self.next_token_on_channel(index + 1, self.channel);
            remaining -= 1;
        }
        self.sync(index);
        self.tokens.get(index).map(Rc::as_ref)
    }

    /// Returns the token at one-based lookahead/lookbehind offset as a shared
    /// reference-counted token.
    pub fn lt_ref(&mut self, offset: isize) -> Option<TokenRef> {
        if offset == 0 {
            return None;
        }
        if offset < 0 {
            return offset
                .checked_neg()
                .map(isize::cast_unsigned)
                .and_then(|offset| self.lb_ref(offset));
        }

        let mut index = self.next_token_on_channel(self.cursor, self.channel);
        let mut remaining = offset;
        while remaining > 1 {
            index = self.next_token_on_channel(index + 1, self.channel);
            remaining -= 1;
        }
        self.sync(index);
        self.tokens.get(index).map(Rc::clone)
    }

    pub fn lb(&self, offset: usize) -> Option<&CommonToken> {
        if offset == 0 || self.cursor == 0 {
            return None;
        }
        let mut index = self.cursor;
        let mut remaining = offset;
        while remaining > 0 {
            index = self.previous_token_on_channel(index, self.channel)?;
            remaining -= 1;
        }
        self.tokens.get(index).map(Rc::as_ref)
    }

    fn lb_ref(&self, offset: usize) -> Option<TokenRef> {
        if offset == 0 || self.cursor == 0 {
            return None;
        }
        let mut index = self.cursor;
        let mut remaining = offset;
        while remaining > 0 {
            index = self.previous_token_on_channel(index, self.channel)?;
            remaining -= 1;
        }
        self.tokens.get(index).map(Rc::clone)
    }

    pub const fn token_source(&self) -> &S {
        &self.source
    }

    pub fn tokens(&self) -> &[CommonToken] {
        &self.public_tokens
    }

    /// Ensures the buffer contains `index`, unless EOF has already been fetched.
    fn sync(&mut self, index: usize) -> bool {
        if index < self.tokens.len() {
            return true;
        }
        let needed = index + 1 - self.tokens.len();
        self.fetch(needed) >= needed
    }

    /// Fetches up to `count` more tokens, stopping early at EOF.
    fn fetch(&mut self, count: usize) -> usize {
        let mut fetched = 0;
        while fetched < count && !self.fetched_eof {
            self.fetch_one();
            fetched += 1;
        }
        fetched
    }

    fn fetch_one(&mut self) {
        let mut token = self.source.next_token();
        self.source_errors.extend(self.source.drain_errors());
        let token_index = isize::try_from(self.tokens.len()).unwrap_or(isize::MAX);
        token.set_token_index(token_index);
        self.fetched_eof = token.token_type() == TOKEN_EOF;
        self.tokens.push(Rc::new(token.clone()));
        self.public_tokens.push(token);
        self.next_visible_after.push(UNKNOWN_NEXT_VISIBLE);
    }

    /// Moves a raw token index to the next token visible on this stream's
    /// channel.
    fn adjust_seek_index(&mut self, index: usize) -> usize {
        self.next_token_on_channel(index, self.channel)
    }

    /// Finds the next buffered token on `channel`, fetching as needed.
    fn next_token_on_channel(&mut self, mut index: usize, channel: i32) -> usize {
        self.sync(index);
        while let Some(token) = self.tokens.get(index) {
            if token.token_type() == TOKEN_EOF || token.channel() == channel {
                return index;
            }
            index += 1;
            self.sync(index);
        }
        index
    }

    /// Finds the previous buffered token on `channel`.
    fn previous_token_on_channel(&self, mut index: usize, channel: i32) -> Option<usize> {
        while index > 0 {
            index -= 1;
            let token = self.tokens.get(index)?;
            if token.token_type() == TOKEN_EOF || token.channel() == channel {
                return Some(index);
            }
        }
        None
    }

    /// Finds the previous buffered token visible to this stream before
    /// `index`.
    ///
    /// Parser rule intervals and `$text` actions are defined in terms of
    /// visible tokens, but their rendered source text still includes hidden
    /// tokens between the visible start and stop. Returning the previous token
    /// on the stream channel avoids accidentally using trailing hidden
    /// whitespace as the stop token.
    pub fn previous_visible_token_index(&mut self, index: usize) -> Option<usize> {
        if index > 0 {
            self.sync(index - 1);
        }
        self.previous_token_on_channel(index, self.channel)
    }
}

impl<S> IntStream for CommonTokenStream<S>
where
    S: TokenSource,
{
    fn consume(&mut self) {
        if self.la(1) == EOF {
            return;
        }
        let current = self.next_token_on_channel(self.cursor, self.channel);
        self.cursor = self.adjust_seek_index(current + 1);
    }

    fn la(&mut self, offset: isize) -> i32 {
        self.la_token(offset)
    }

    fn index(&self) -> usize {
        self.cursor
    }

    fn seek(&mut self, index: usize) {
        self.cursor = self.adjust_seek_index(index);
    }

    fn size(&self) -> usize {
        self.tokens.len()
    }

    fn source_name(&self) -> &str {
        let source_name = self.source.source_name();
        if source_name.is_empty() {
            UNKNOWN_SOURCE_NAME
        } else {
            source_name
        }
    }
}

impl<S> CommonTokenStream<S>
where
    S: TokenSource,
{
    pub fn la_token(&mut self, offset: isize) -> i32 {
        self.lt(offset).map_or(TOKEN_EOF, Token::token_type)
    }

    /// Returns the token type at a buffered absolute index, fetching from the
    /// source on demand. Past-EOF reads are reported as `TOKEN_EOF` so the
    /// caller does not need to special-case the buffer's stop. The cursor is
    /// not modified, which lets hot speculative loops avoid the seek
    /// round-trip when they only need lookahead types.
    pub fn token_type_at_index(&mut self, index: usize) -> i32 {
        self.sync(index);
        self.tokens
            .get(index)
            .map_or(TOKEN_EOF, |token| token.token_type())
    }

    /// Returns the token channel visible to `LT/LA` operations.
    pub const fn channel(&self) -> i32 {
        self.channel
    }

    /// Returns the next parser-visible token index after consuming the token
    /// at `index`, skipping hidden-channel tokens. The parser's stream cursor
    /// is not modified. Used by speculative recognition that simulates token
    /// consumption thousands of times without committing it.
    pub fn next_visible_after(&mut self, index: usize) -> usize {
        self.sync(index);
        if let Some(cached) = self
            .next_visible_after
            .get(index)
            .copied()
            .filter(|cached| *cached != UNKNOWN_NEXT_VISIBLE)
        {
            return cached;
        }

        let mut next = index + 1;
        let found = loop {
            self.sync(next);
            match self.tokens.get(next) {
                Some(token)
                    if token.token_type() != TOKEN_EOF && token.channel() != self.channel =>
                {
                    next += 1;
                    continue;
                }
                _ => break next,
            }
        };
        if let Some(slot) = self.next_visible_after.get_mut(index) {
            *slot = found;
        }
        found
    }

    pub fn text(&mut self, start: usize, stop: usize) -> String {
        self.sync(stop);
        if start > stop || start >= self.tokens.len() {
            return String::new();
        }
        // Java's `BufferedTokenStream.getText(Interval)` stops at the first
        // EOF token, so an interval whose stop index lands on EOF renders
        // without a trailing `<EOF>` (diagnostics rely on this).
        self.tokens[start..=stop.min(self.tokens.len().saturating_sub(1))]
            .iter()
            .take_while(|token| token.token_type() != TOKEN_EOF)
            .map(|token| token.text())
            .collect::<Vec<_>>()
            .join("")
    }

    /// Concatenated text of every buffered token except EOF — ANTLR's
    /// `TokenStream.getText()`, the shape generated test actions read through
    /// `self.input().text()`.
    pub fn text_all(&mut self) -> String {
        self.fill();
        self.tokens
            .iter()
            .filter(|token| token.token_type() != TOKEN_EOF)
            .map(|token| token.text())
            .collect()
    }

    /// Returns and clears diagnostics emitted by the underlying token source
    /// while this stream was fetching tokens.
    pub fn drain_source_errors(&mut self) -> Vec<TokenSourceError> {
        std::mem::take(&mut self.source_errors)
    }

    pub const fn is_filled(&self) -> bool {
        self.fetched_eof
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::{CommonToken, HIDDEN_CHANNEL};

    #[derive(Debug)]
    struct VecTokenSource {
        tokens: Vec<CommonToken>,
        index: usize,
    }

    impl TokenSource for VecTokenSource {
        fn next_token(&mut self) -> CommonToken {
            let token = self
                .tokens
                .get(self.index)
                .cloned()
                .unwrap_or_else(|| CommonToken::eof("vec", self.index, 1, self.index));
            self.index += 1;
            token
        }

        fn line(&self) -> usize {
            1
        }

        fn column(&self) -> usize {
            self.index
        }

        fn source_name(&self) -> &'static str {
            "vec"
        }
    }

    #[test]
    fn stream_skips_hidden_channel_for_lookahead() {
        let source = VecTokenSource {
            tokens: vec![
                CommonToken::new(1).with_text("a"),
                CommonToken::new(2)
                    .with_text(" ")
                    .with_channel(HIDDEN_CHANNEL),
                CommonToken::new(3).with_text("b"),
                CommonToken::eof("vec", 3, 1, 3),
            ],
            index: 0,
        };
        let mut stream = CommonTokenStream::new(source);
        assert_eq!(stream.la_token(1), 1);
        stream.consume();
        assert_eq!(stream.la_token(1), 3);
        assert_eq!(
            stream
                .lt(-1)
                .expect("look-behind token should be buffered")
                .token_type(),
            1
        );
    }

    #[test]
    fn lookahead_skips_hidden_token_at_initial_cursor() {
        let source = VecTokenSource {
            tokens: vec![
                CommonToken::new(2)
                    .with_text(" ")
                    .with_channel(HIDDEN_CHANNEL),
                CommonToken::new(1).with_text("a"),
                CommonToken::eof("vec", 2, 1, 2),
            ],
            index: 0,
        };
        let mut stream = CommonTokenStream::new(source);

        assert_eq!(stream.la_token(1), 1);
        assert_eq!(stream.lt(1).and_then(Token::text), Some("a"));
        stream.consume();
        assert_eq!(stream.la_token(1), TOKEN_EOF);
    }

    #[test]
    fn text_returns_empty_when_start_is_past_buffer() {
        let source = VecTokenSource {
            tokens: vec![
                CommonToken::new(1).with_text("a"),
                CommonToken::eof("vec", 1, 1, 1),
            ],
            index: 0,
        };
        let mut stream = CommonTokenStream::new(source);

        assert_eq!(stream.text(10, 12), "");
    }

    #[test]
    fn tokens_returns_public_slice() {
        let source = VecTokenSource {
            tokens: vec![
                CommonToken::new(1).with_text("a"),
                CommonToken::new(2).with_text("b"),
                CommonToken::eof("vec", 2, 1, 2),
            ],
            index: 0,
        };
        let mut stream = CommonTokenStream::new(source);
        stream.fill();

        fn token_count(tokens: &[CommonToken]) -> usize {
            tokens.len()
        }

        let tokens = stream.tokens();
        assert_eq!(token_count(tokens), 3);
        assert_eq!(tokens[0].token_type(), 1);
        assert_eq!(tokens.first().map(Token::token_type), Some(1));
        assert_eq!(
            tokens.iter().next_back().map(Token::token_type),
            Some(TOKEN_EOF)
        );
    }
}
