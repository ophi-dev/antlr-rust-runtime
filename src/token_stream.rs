use crate::int_stream::{EOF, IntStream, UNKNOWN_SOURCE_NAME};
use crate::token::{CommonToken, DEFAULT_CHANNEL, TOKEN_EOF, Token, TokenSource};

#[derive(Debug)]
pub struct CommonTokenStream<S> {
    source: S,
    tokens: Vec<CommonToken>,
    cursor: usize,
    fetched_eof: bool,
    channel: i32,
}

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
            cursor: 0,
            fetched_eof: false,
            channel,
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
        self.tokens.get(index)
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

        let mut index = self.cursor;
        let mut remaining = offset;
        while remaining > 1 {
            index = self.next_token_on_channel(index + 1, self.channel);
            remaining -= 1;
        }
        self.sync(index);
        self.tokens.get(index)
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
        self.tokens.get(index)
    }

    pub const fn token_source(&self) -> &S {
        &self.source
    }

    pub fn tokens(&self) -> &[CommonToken] {
        &self.tokens
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
        let token_index = isize::try_from(self.tokens.len()).unwrap_or(isize::MAX);
        token.set_token_index(token_index);
        self.fetched_eof = token.token_type() == TOKEN_EOF;
        self.tokens.push(token);
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
        self.cursor = self.adjust_seek_index(self.cursor + 1);
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

    pub fn text(&mut self, start: usize, stop: usize) -> String {
        self.sync(stop);
        if start > stop {
            return String::new();
        }
        self.tokens[start..=stop.min(self.tokens.len().saturating_sub(1))]
            .iter()
            .filter_map(Token::text)
            .collect::<Vec<_>>()
            .join("")
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
}
