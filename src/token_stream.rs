use crate::int_stream::{EOF, IntStream, UNKNOWN_SOURCE_NAME};

use crate::token::{
    DEFAULT_CHANNEL, TOKEN_EOF, Token, TokenId, TokenSink, TokenSource, TokenSourceError,
    TokenSpec, TokenStore, TokenStoreError, TokenView,
};

#[derive(Debug)]
pub struct CommonTokenStream<S> {
    source: S,
    store: TokenStore,
    source_token_count: usize,
    next_visible_after: Vec<usize>,
    cursor: usize,
    channel: i32,
    source_errors: Vec<TokenSourceError>,
}

const UNKNOWN_NEXT_VISIBLE: usize = usize::MAX;

impl<S> CommonTokenStream<S>
where
    S: TokenSource,
{
    /// Creates and fills a token stream that filters lookahead to the default
    /// channel.
    ///
    /// Use [`Self::try_new`] when token/source limit errors should be handled
    /// instead of reported as a construction panic.
    pub fn new(source: S) -> Self {
        Self::try_new(source).unwrap_or_else(|error| panic!("failed to buffer tokens: {error}"))
    }

    pub fn try_new(source: S) -> Result<Self, TokenStoreError> {
        Self::try_with_channel(source, DEFAULT_CHANNEL)
    }

    /// Creates and fills a token stream whose `LT/LA` operations see only
    /// `channel`.
    pub fn with_channel(source: S, channel: i32) -> Self {
        Self::try_with_channel(source, channel)
            .unwrap_or_else(|error| panic!("failed to buffer tokens: {error}"))
    }

    pub fn try_with_channel(mut source: S, channel: i32) -> Result<Self, TokenStoreError> {
        let source_name = source.source_name().to_owned();
        let mut store = TokenStore::new(source.source_text(), source_name);
        let mut source_errors = Vec::new();
        loop {
            let mut sink = TokenSink::new(&mut store);
            let id = source.next_token(&mut sink)?;
            source_errors.extend(source.drain_errors());
            let token = sink
                .view(id)
                .expect("token source returned an ID it did not emit");
            if token.token_type() == TOKEN_EOF {
                break;
            }
        }
        let source_token_count = store.len();
        let mut stream = Self {
            source,
            store,
            source_token_count,
            next_visible_after: vec![UNKNOWN_NEXT_VISIBLE; source_token_count],
            cursor: 0,
            channel,
            source_errors,
        };
        stream.cursor = stream.adjust_seek_index(0);
        Ok(stream)
    }

    /// Idempotent eager-buffering operation. Construction already buffers
    /// through EOF so the store can be shared with CST nodes.
    pub fn fill(&mut self) {
        self.cursor = self.adjust_seek_index(self.cursor);
    }

    /// Returns a borrowing view of the token at an absolute buffered index.
    pub fn get(&self, index: usize) -> Option<TokenView<'_>> {
        (index < self.source_token_count)
            .then(|| TokenId::try_from(index).ok())
            .flatten()
            .and_then(|id| self.store.view(id))
    }

    /// Returns the compact ID at an absolute buffered index.
    pub fn get_id(&self, index: usize) -> Option<TokenId> {
        (index < self.source_token_count)
            .then(|| TokenId::try_from(index).ok())
            .flatten()
    }

    /// Returns the token at one-based lookahead/lookbehind offset, skipping
    /// tokens outside the configured channel for positive offsets.
    pub fn lt(&self, offset: isize) -> Option<TokenView<'_>> {
        self.lt_id(offset).and_then(|id| self.store.view(id))
    }

    /// Returns the compact token ID at one-based lookahead/lookbehind offset.
    pub fn lt_id(&self, offset: isize) -> Option<TokenId> {
        if offset == 0 {
            return None;
        }
        if offset < 0 {
            return offset
                .checked_neg()
                .map(isize::cast_unsigned)
                .and_then(|offset| self.lb_id(offset));
        }

        let mut index = self.next_token_on_channel(self.cursor, self.channel);
        let mut remaining = offset;
        while remaining > 1 {
            index = self.next_token_on_channel(index + 1, self.channel);
            remaining -= 1;
        }
        self.get_id(index)
    }

    pub fn lb(&self, offset: usize) -> Option<TokenView<'_>> {
        self.lb_id(offset).and_then(|id| self.store.view(id))
    }

    fn lb_id(&self, offset: usize) -> Option<TokenId> {
        if offset == 0 || self.cursor == 0 {
            return None;
        }
        let mut index = self.cursor;
        let mut remaining = offset;
        while remaining > 0 {
            index = self.previous_token_on_channel(index, self.channel)?;
            remaining -= 1;
        }
        self.get_id(index)
    }

    pub const fn token_source(&self) -> &S {
        &self.source
    }

    /// Iterates borrowing views of the original buffered token sequence.
    pub const fn tokens(&self) -> TokenIter<'_> {
        TokenIter {
            store: &self.store,
            next: 0,
            stop: self.source_token_count,
        }
    }

    pub const fn token_count(&self) -> usize {
        self.source_token_count
    }

    /// Returns the canonical token store owned by this stream.
    #[must_use]
    pub const fn token_store(&self) -> &TokenStore {
        &self.store
    }

    /// Consumes the stream and returns its canonical token store.
    #[must_use]
    pub fn into_token_store(self) -> TokenStore {
        self.store
    }

    pub(crate) fn token_view(&self, id: TokenId) -> Option<TokenView<'_>> {
        self.store.view(id)
    }

    pub(crate) fn insert(&mut self, spec: TokenSpec) -> Result<TokenId, TokenStoreError> {
        self.store.push(spec)
    }

    /// Moves a raw token index to the next token visible on this stream's
    /// channel.
    fn adjust_seek_index(&self, index: usize) -> usize {
        self.next_token_on_channel(index, self.channel)
    }

    /// Finds the next buffered token on `channel`.
    fn next_token_on_channel(&self, mut index: usize, channel: i32) -> usize {
        while let Some(id) = self.get_id(index) {
            if self.store.token_type(id) == Some(TOKEN_EOF)
                || self.store.channel(id) == Some(channel)
            {
                return index;
            }
            index += 1;
        }
        index
    }

    /// Finds the previous buffered token on `channel`.
    fn previous_token_on_channel(&self, mut index: usize, channel: i32) -> Option<usize> {
        while index > 0 {
            index -= 1;
            let id = self.get_id(index)?;
            if self.store.token_type(id) == Some(TOKEN_EOF)
                || self.store.channel(id) == Some(channel)
            {
                return Some(index);
            }
        }
        None
    }

    /// Finds the previous buffered token visible to this stream before
    /// `index`.
    pub fn previous_visible_token_index(&self, index: usize) -> Option<usize> {
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
        self.source_token_count
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
    pub fn la_token(&self, offset: isize) -> i32 {
        self.lt_id(offset)
            .and_then(|id| self.store.token_type(id))
            .unwrap_or(TOKEN_EOF)
    }

    /// Returns the token type at a buffered absolute index. Past-EOF reads are
    /// reported as `TOKEN_EOF`.
    pub fn token_type_at_index(&self, index: usize) -> i32 {
        self.get_id(index)
            .and_then(|id| self.store.token_type(id))
            .unwrap_or(TOKEN_EOF)
    }

    /// Returns the token channel visible to `LT/LA` operations.
    pub const fn channel(&self) -> i32 {
        self.channel
    }

    /// Returns the next parser-visible token index after consuming the token
    /// at `index`, skipping hidden-channel tokens.
    pub fn next_visible_after(&mut self, index: usize) -> usize {
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
            match self.get_id(next) {
                Some(id)
                    if self.store.token_type(id) != Some(TOKEN_EOF)
                        && self.store.channel(id) != Some(self.channel) =>
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

    pub fn text(&self, start: usize, stop: usize) -> String {
        if start > stop || start >= self.source_token_count {
            return String::new();
        }
        (start..=stop.min(self.source_token_count.saturating_sub(1)))
            .filter_map(|index| self.get(index))
            .take_while(|token| token.token_type() != TOKEN_EOF)
            .map(|token| token.text().to_owned())
            .collect()
    }

    /// Concatenated text of every buffered token except EOF.
    pub fn text_all(&self) -> String {
        self.tokens()
            .filter(|token| token.token_type() != TOKEN_EOF)
            .map(|token| token.text().to_owned())
            .collect()
    }

    /// Returns and clears diagnostics emitted by the underlying token source.
    pub fn drain_source_errors(&mut self) -> Vec<TokenSourceError> {
        std::mem::take(&mut self.source_errors)
    }

    pub const fn is_filled(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct TokenIter<'a> {
    store: &'a TokenStore,
    next: usize,
    stop: usize,
}

impl<'a> Iterator for TokenIter<'a> {
    type Item = TokenView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.stop {
            return None;
        }
        let id = TokenId::try_from(self.next).ok()?;
        self.next += 1;
        self.store.view(id)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.stop - self.next;
        (remaining, Some(remaining))
    }
}

impl DoubleEndedIterator for TokenIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.next >= self.stop {
            return None;
        }
        self.stop -= 1;
        let id = TokenId::try_from(self.stop).ok()?;
        self.store.view(id)
    }
}

impl ExactSizeIterator for TokenIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::HIDDEN_CHANNEL;
    use std::collections::VecDeque;

    #[derive(Debug)]
    struct VecTokenSource {
        tokens: VecDeque<TokenSpec>,
        index: usize,
    }

    impl TokenSource for VecTokenSource {
        fn next_token(&mut self, sink: &mut TokenSink<'_>) -> Result<TokenId, TokenStoreError> {
            let spec = self
                .tokens
                .pop_front()
                .unwrap_or_else(|| TokenSpec::eof(self.index, self.index, 1, self.index));
            self.index += 1;
            sink.push(spec)
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

    fn source(tokens: Vec<TokenSpec>) -> VecTokenSource {
        VecTokenSource {
            tokens: tokens.into(),
            index: 0,
        }
    }

    #[test]
    fn stream_skips_hidden_channel_for_lookahead() {
        let mut stream = CommonTokenStream::new(source(vec![
            TokenSpec::explicit(1, "a"),
            TokenSpec::explicit(2, " ").with_channel(HIDDEN_CHANNEL),
            TokenSpec::explicit(3, "b"),
            TokenSpec::eof(3, 3, 1, 3),
        ]));
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
    fn text_returns_empty_when_start_is_past_buffer() {
        let stream = CommonTokenStream::new(source(vec![
            TokenSpec::explicit(1, "a"),
            TokenSpec::eof(1, 1, 1, 1),
        ]));
        assert_eq!(stream.text(10, 12), "");
    }

    #[test]
    fn tokens_returns_borrowing_views() {
        let stream = CommonTokenStream::new(source(vec![
            TokenSpec::explicit(1, "a"),
            TokenSpec::explicit(2, "b"),
            TokenSpec::eof(2, 2, 1, 2),
        ]));
        assert_eq!(stream.tokens().len(), 3);
        assert_eq!(
            stream.tokens().next().map(|token| token.token_type()),
            Some(1)
        );
        assert_eq!(
            stream.tokens().next_back().map(|token| token.token_type()),
            Some(TOKEN_EOF)
        );
    }
}
