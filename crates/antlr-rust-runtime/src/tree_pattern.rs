//! ANTLR parse-tree pattern matching.
//!
//! A *tree pattern* is a string of ordinary grammar input with embedded
//! `<tag>` placeholders — for example `<ID> = <expr>;` matched as the rule
//! `stat`. Literals must match exactly; a `<rule>` tag matches any subtree of
//! that parser rule and a `<TOKEN>` tag matches any token of that type. Tags
//! may carry a label, `<lhs:ID>`, so matched nodes can be looked up by name.
//!
//! This mirrors ANTLR's `org.antlr.v4.runtime.tree.pattern` package. Compiling
//! a pattern lexes its literal chunks with the real lexer, converts each tag
//! into a synthetic rule/token tag token (ANTLR's `RuleTagToken` /
//! `TokenTagToken`), and interprets that hybrid token stream over a rule-bypass
//! ATN (see [`crate::atn::parser_atn::ParserAtn::with_bypass_alternatives`]) to
//! build a *pattern tree*. [`ParseTreePattern::match_tree`] then walks a
//! subject tree and the pattern tree in lockstep, binding tag labels.
//!
//! The ergonomic entry point is the `compile_parse_tree_pattern` method every
//! generated parser exposes; [`ParseTreePatternMatcher`] is the lower-level,
//! reusable compiler behind it.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::atn::parser_atn::ParserAtn;
use crate::recognizer::{Recognizer, RecognizerData};
use crate::token::{Token, TokenId, TokenSink, TokenSource, TokenSpec, TokenStoreError};
use crate::tree::{Node, NodeKind};
use crate::{BaseParser, CommonTokenStream, TOKEN_EOF};

const MATCH_STACK_RED_ZONE: usize = 1024 * 1024;
const MATCH_STACK_SIZE: usize = 4 * 1024 * 1024;

/// A tag's disposition: a reference to a parser rule or to a token type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TagKind {
    /// `<expr>` — matches an entire subtree produced by the named parser rule.
    /// Carries the rule index and the imaginary bypass token type used to drive
    /// the interpreter.
    Rule { rule_index: usize, bypass_type: i32 },
    /// `<ID>` — matches a single token of the named type.
    Token { token_type: i32 },
}

/// Identity of a synthetic tag token, tracked out of band keyed by the
/// [`TokenId`] the tag occupies in the pattern token store.
///
/// The compact [`crate::token::TokenStore`] has no room for the rule/token
/// name and label a tag carries, so — like the store's own sparse
/// `explicit_text` side table — the matcher keeps this data beside the store
/// rather than inside it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TagInfo {
    kind: TagKind,
    /// The rule or token name the tag references (e.g. `"expr"`, `"ID"`).
    name: String,
    /// The explicit label, if the tag was written `<label:name>`.
    label: Option<String>,
}

impl TagInfo {
    /// The names a matched node is filed under: always the referenced rule or
    /// token name, plus the explicit label when present. Mirrors the dual
    /// `labels.map(name, ...)` / `labels.map(label, ...)` calls in ANTLR's
    /// `matchImpl`.
    fn label_keys(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str()).chain(self.label.as_deref())
    }
}

/// A raw fragment of a split pattern: either literal text or a `<tag>`.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Chunk {
    /// Literal input text, with escape sequences already stripped.
    Text(String),
    /// A `<tag>` island: the referenced rule/token name and optional label.
    Tag { name: String, label: Option<String> },
}

/// An invalid tree pattern or a failure while compiling one.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ParseTreePatternError {
    /// A start delimiter was seen without a matching stop delimiter.
    #[error("unterminated tag in pattern: {pattern}")]
    UnterminatedTag { pattern: String },
    /// A stop delimiter was seen without a preceding start delimiter.
    #[error("missing start tag in pattern: {pattern}")]
    MissingStartTag { pattern: String },
    /// A stop delimiter appeared at or before its start delimiter.
    #[error("tag delimiters out of order in pattern: {pattern}")]
    DelimitersOutOfOrder { pattern: String },
    /// A tag was empty (`<>` or `<label:>`).
    #[error("empty tag in pattern: {pattern}")]
    EmptyTag { pattern: String },
    /// A `<TOKEN>` tag named a token the grammar does not define.
    #[error("unknown token {name} in pattern: {pattern}")]
    UnknownToken { name: String, pattern: String },
    /// A `<rule>` tag named a parser rule the grammar does not define.
    #[error("unknown rule {name} in pattern: {pattern}")]
    UnknownRule { name: String, pattern: String },
    /// A tag started with neither an upper- nor lower-case letter, so it could
    /// not be classified as a token or rule reference.
    #[error("invalid tag {tag} in pattern: {pattern}")]
    InvalidTag { tag: String, pattern: String },
    /// Lexing a literal chunk with the real lexer failed.
    #[error("could not tokenize pattern chunk: {message}")]
    Tokenization { message: String },
    /// The start rule did not consume the whole pattern (ANTLR issue #413).
    #[error("start rule did not consume the full pattern: {pattern}")]
    StartRuleDoesNotConsumeFullPattern { pattern: String },
    /// Interpreting the pattern token stream failed.
    #[error("could not interpret pattern as rule {rule_index}: {message}")]
    CannotInvokeStartRule { rule_index: usize, message: String },
    /// The rule-bypass transform of the grammar ATN failed.
    #[error("could not build rule-bypass ATN: {message}")]
    BypassAtn { message: String },
    /// [`ParseTreePatternMatcher::set_delimiters`] was given an empty start or
    /// stop delimiter.
    #[error("{which} delimiter cannot be empty")]
    EmptyDelimiter { which: &'static str },
}

/// Tag delimiters and escape string used to split a pattern.
///
/// Defaults to `<`, `>`, and `\`, matching ANTLR. Grammars that use `<...>` in
/// their own concrete syntax (e.g. Java generics) can pick different delimiters
/// via [`ParseTreePatternMatcher::set_delimiters`].
#[derive(Clone, Debug, Eq, PartialEq)]
struct Delimiters {
    start: String,
    stop: String,
    escape: String,
}

impl Default for Delimiters {
    fn default() -> Self {
        Self {
            start: "<".to_owned(),
            stop: ">".to_owned(),
            escape: "\\".to_owned(),
        }
    }
}

/// Splits a pattern into interleaved literal text and `<tag>` chunks.
///
/// Faithful port of ANTLR's `ParseTreePatternMatcher.split`: it scans for the
/// escaped and unescaped delimiters, validates that starts and stops are
/// balanced and ordered, slices the chunks, then strips escape sequences from
/// the text chunks only (never from tags). Operates on `char` boundaries so
/// multi-byte delimiters and Unicode input are handled correctly.
fn split(pattern: &str, delimiters: &Delimiters) -> Result<Vec<Chunk>, ParseTreePatternError> {
    let chars: Vec<char> = pattern.chars().collect();
    let start: Vec<char> = delimiters.start.chars().collect();
    let stop: Vec<char> = delimiters.stop.chars().collect();
    let escape: Vec<char> = delimiters.escape.chars().collect();

    let matches_at = |at: usize, needle: &[char]| -> bool {
        !needle.is_empty() && chars[at..].starts_with(needle)
    };

    // Pass 1: locate every unescaped start/stop delimiter, by char index.
    let mut starts = Vec::new();
    let mut stops = Vec::new();
    let mut position = 0;
    while position < chars.len() {
        if matches_at(position, &escape) && matches_at(position + escape.len(), &start) {
            position += escape.len() + start.len();
        } else if matches_at(position, &escape) && matches_at(position + escape.len(), &stop) {
            position += escape.len() + stop.len();
        } else if matches_at(position, &start) {
            starts.push(position);
            position += start.len();
        } else if matches_at(position, &stop) {
            stops.push(position);
            position += stop.len();
        } else {
            position += 1;
        }
    }

    if starts.len() > stops.len() {
        return Err(ParseTreePatternError::UnterminatedTag {
            pattern: pattern.to_owned(),
        });
    }
    if starts.len() < stops.len() {
        return Err(ParseTreePatternError::MissingStartTag {
            pattern: pattern.to_owned(),
        });
    }
    for (open, close) in starts.iter().zip(&stops) {
        if open >= close {
            return Err(ParseTreePatternError::DelimitersOutOfOrder {
                pattern: pattern.to_owned(),
            });
        }
    }
    // Tags must also not overlap each other (e.g. `<a<b>>` pairs 0/4 and 2/5):
    // each close must come before the next open, or the inter-tag text slice
    // below would be an inverted range. Upstream reaches the same shape and
    // throws from `String.substring`; returning the structured error is safer.
    for (close, next_open) in stops.iter().zip(starts.iter().skip(1)) {
        if close + stop.len() > *next_open {
            return Err(ParseTreePatternError::DelimitersOutOfOrder {
                pattern: pattern.to_owned(),
            });
        }
    }

    let slice = |from: usize, to: usize| -> String { chars[from..to].iter().collect() };

    // Pass 2: collect chunks between the located delimiters.
    let ntags = starts.len();
    let mut chunks = Vec::new();
    if ntags == 0 {
        chunks.push(Chunk::Text(slice(0, chars.len())));
    } else if starts[0] > 0 {
        chunks.push(Chunk::Text(slice(0, starts[0])));
    }
    for index in 0..ntags {
        let tag = slice(starts[index] + start.len(), stops[index]);
        chunks.push(parse_tag(&tag, pattern)?);
        if index + 1 < ntags {
            chunks.push(Chunk::Text(slice(
                stops[index] + stop.len(),
                starts[index + 1],
            )));
        }
    }
    if ntags > 0 {
        let after_last = stops[ntags - 1] + stop.len();
        if after_last < chars.len() {
            chunks.push(Chunk::Text(slice(after_last, chars.len())));
        }
    }

    // Strip escape sequences from text chunks (tags are left untouched).
    if !delimiters.escape.is_empty() {
        for chunk in &mut chunks {
            if let Chunk::Text(text) = chunk {
                *text = strip_escape(text, &delimiters.escape);
            }
        }
    }

    Ok(chunks)
}

/// Removes every occurrence of `escape` from `text`, non-overlapping and
/// left to right — the escape-stripping ANTLR does with `String.replace`.
fn strip_escape(text: &str, escape: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(escape) {
        out.push_str(&rest[..at]);
        rest = &rest[at + escape.len()..];
    }
    out.push_str(rest);
    out
}

/// Parses the inside of a `<...>` into a tag chunk, splitting an optional
/// `label:` prefix. Empty tags (`<>`, `<label:>`) are rejected.
fn parse_tag(tag: &str, pattern: &str) -> Result<Chunk, ParseTreePatternError> {
    let (label, name) = tag.find(':').map_or((None, tag), |colon| {
        (Some(tag[..colon].to_owned()), &tag[colon + 1..])
    });
    if name.is_empty() {
        return Err(ParseTreePatternError::EmptyTag {
            pattern: pattern.to_owned(),
        });
    }
    Ok(Chunk::Tag {
        name: name.to_owned(),
        label,
    })
}

/// Lexes one literal pattern chunk into the token specs it produces.
///
/// The matcher owns all split/tag/interpret logic; this trait is the single
/// grammar-specific hook, supplying the real lexer's output for a run of
/// concrete input text. The trailing EOF must be excluded; off-default-channel
/// tokens (whitespace, comments) may be returned on their own channel and are
/// skipped by the interpreter exactly as in a normal parse, matching ANTLR's
/// `tokenize`. Implemented for
/// `FnMut(&str) -> Result<Vec<TokenSpec>, ParseTreePatternError>` so a closure
/// suffices.
pub trait PatternLexer {
    /// Tokenizes `text` into token specs (no EOF), each on its original channel.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseTreePatternError::Tokenization`] if the lexer rejects
    /// the chunk.
    fn tokenize_chunk(&mut self, text: &str) -> Result<Vec<TokenSpec>, ParseTreePatternError>;
}

impl<F> PatternLexer for F
where
    F: FnMut(&str) -> Result<Vec<TokenSpec>, ParseTreePatternError>,
{
    fn tokenize_chunk(&mut self, text: &str) -> Result<Vec<TokenSpec>, ParseTreePatternError> {
        self(text)
    }
}

/// Runs a lexer over one chunk of pattern text and returns its token specs
/// (every non-EOF token, on its original channel), suitable for a
/// [`PatternLexer`].
///
/// This is the bridge generated parsers use to satisfy [`PatternLexer`] from
/// their concrete lexer: `make_lexer` builds a fresh lexer over the chunk's
/// [`InputStream`](crate::InputStream), the tokens are buffered, and each
/// non-EOF token becomes a `TokenSpec::explicit(type, text)` carrying its
/// channel. Like ANTLR's `tokenize`, hidden-channel tokens (whitespace,
/// comments) are preserved on their channel so the interpreter skips them the
/// same way it does during a normal parse. Positions are dropped because
/// pattern trees compare by type and text, not span.
///
/// # Errors
///
/// Returns [`ParseTreePatternError::Tokenization`] if the lexer reports a
/// tokenization error for the chunk.
pub fn lex_pattern_chunk<L>(
    text: &str,
    make_lexer: impl FnOnce(crate::InputStream) -> L,
) -> Result<Vec<TokenSpec>, ParseTreePatternError>
where
    L: TokenSource,
{
    let lexer = make_lexer(crate::InputStream::new(text));
    let mut stream =
        CommonTokenStream::try_new(lexer).map_err(|error| ParseTreePatternError::Tokenization {
            message: error.to_string(),
        })?;
    stream.fill();
    if let Some(error) = stream.drain_source_errors().into_iter().next() {
        return Err(ParseTreePatternError::Tokenization {
            message: format!("lexer error at {}:{}", error.line, error.column),
        });
    }
    Ok(stream
        .tokens()
        .filter(|token| token.token_type() != TOKEN_EOF)
        .map(|token| {
            TokenSpec::explicit(token.token_type(), token.text_or_empty())
                .with_channel(token.channel())
        })
        .collect())
}

/// Compiles tree patterns for one grammar.
///
/// Holds the grammar's rule-bypass ATN and recognizer metadata; each
/// [`Self::compile`] call lexes the pattern's literal chunks (via the supplied
/// [`PatternLexer`]), converts tags into synthetic tokens, and interprets the
/// hybrid stream to build a reusable [`ParseTreePattern`].
///
/// Most callers reach this through
/// the `compile_parse_tree_pattern` method on generated parsers; construct one directly to
/// reuse the bypass ATN across many patterns or to customize delimiters.
#[derive(Debug)]
pub struct ParseTreePatternMatcher<'a> {
    bypass_atn: ParserAtn,
    data: &'a RecognizerData,
    delimiters: Delimiters,
}

impl<'a> ParseTreePatternMatcher<'a> {
    /// Creates a matcher for a grammar's parser ATN and recognizer metadata.
    ///
    /// The bypass ATN is derived from `atn` once here and reused by every
    /// compile. `data` supplies rule and token names for resolving tags.
    ///
    /// # Errors
    ///
    /// Returns [`ParseTreePatternError::BypassAtn`] if the rule-bypass
    /// transform of `atn` fails (e.g. an unrecognizable left-recursive
    /// precedence prefix).
    pub fn new(atn: &ParserAtn, data: &'a RecognizerData) -> Result<Self, ParseTreePatternError> {
        let bypass_atn =
            atn.with_bypass_alternatives()
                .map_err(|error| ParseTreePatternError::BypassAtn {
                    message: error.to_string(),
                })?;
        Ok(Self {
            bypass_atn,
            data,
            delimiters: Delimiters::default(),
        })
    }

    /// Overrides the tag delimiters and escape string (defaults `<`, `>`, `\`).
    ///
    /// Useful for grammars whose concrete syntax already uses `<...>`. Unlike
    /// upstream, an empty `escape` is accepted and simply disables escaping
    /// (Java's `indexOf`-based scan misbehaves on an empty escape string).
    ///
    /// # Errors
    ///
    /// Returns [`ParseTreePatternError::EmptyDelimiter`] when `start` or `stop`
    /// is empty, mirroring upstream's `IllegalArgumentException` — an empty
    /// delimiter would silently collapse every pattern into one text chunk.
    pub fn set_delimiters(
        &mut self,
        start: impl Into<String>,
        stop: impl Into<String>,
        escape: impl Into<String>,
    ) -> Result<(), ParseTreePatternError> {
        let start = start.into();
        let stop = stop.into();
        if start.is_empty() {
            return Err(ParseTreePatternError::EmptyDelimiter { which: "start" });
        }
        if stop.is_empty() {
            return Err(ParseTreePatternError::EmptyDelimiter { which: "stop" });
        }
        self.delimiters = Delimiters {
            start,
            stop,
            escape: escape.into(),
        };
        Ok(())
    }

    /// Compiles `pattern`, rooted at parser rule `rule_index`, into a reusable
    /// [`ParseTreePattern`].
    ///
    /// `lexer` tokenizes the pattern's literal chunks; tags become synthetic
    /// rule/token tokens, and the hybrid stream is interpreted over the bypass
    /// ATN starting at `rule_index`.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseTreePatternError`] for a malformed pattern, an unknown
    /// rule/token tag, a lexer failure, an interpretation failure, or a pattern
    /// the start rule does not fully consume.
    pub fn compile(
        &self,
        pattern: &str,
        rule_index: usize,
        lexer: impl PatternLexer,
    ) -> Result<ParseTreePattern, ParseTreePatternError> {
        let chunks = split(pattern, &self.delimiters)?;
        let (specs, tags_by_index) = self.tokenize(&chunks, pattern, lexer)?;
        let tree = self.interpret(specs, &tags_by_index, rule_index, pattern)?;
        Ok(ParseTreePattern {
            pattern: pattern.to_owned(),
            pattern_rule_index: rule_index,
            tree,
        })
    }

    /// Converts chunks into a flat token-spec list, recording which flat indices
    /// are tags. Mirrors ANTLR's `tokenize`: upper-case tags are token
    /// references, lower-case tags are rule references, literals are lexed.
    fn tokenize(
        &self,
        chunks: &[Chunk],
        pattern: &str,
        mut lexer: impl PatternLexer,
    ) -> Result<(Vec<TokenSpec>, BTreeMap<usize, TagInfo>), ParseTreePatternError> {
        let mut specs = Vec::new();
        let mut tags_by_index = BTreeMap::new();
        for chunk in chunks {
            match chunk {
                Chunk::Tag { name, label } => {
                    let (spec, tag) = self.tag_token(name, label.clone(), pattern)?;
                    tags_by_index.insert(specs.len(), tag);
                    specs.push(spec);
                }
                Chunk::Text(text) => {
                    specs.extend(lexer.tokenize_chunk(text)?);
                }
            }
        }
        // An EOF-typed token (an `<EOF>` tag, or a stray EOF from a custom
        // lexer) terminates the buffered token stream, so anything after it
        // would be dropped before the full-consumption check could see it.
        // A trailing EOF is legitimate for rules that end in `EOF`; anywhere
        // else the pattern is broken and must fail loudly instead of silently
        // truncating (upstream ANTLR silently ignores the suffix here).
        if let Some(at) = specs
            .iter()
            .position(|spec| spec.token_type == TOKEN_EOF)
            .filter(|at| at + 1 < specs.len())
        {
            return Err(ParseTreePatternError::Tokenization {
                message: format!(
                    "EOF at pattern token {at} terminates the stream; {} following token(s) \
                     would be ignored",
                    specs.len() - at - 1
                ),
            });
        }
        Ok((specs, tags_by_index))
    }

    /// Builds the synthetic token and tag record for one `<tag>`.
    ///
    /// An upper-case initial classifies a token reference (`<ID>`), a lower-case
    /// initial a rule reference (`<expr>`). Names resolve to token types via the
    /// vocabulary and to rule indices via the rule-name list.
    fn tag_token(
        &self,
        name: &str,
        label: Option<String>,
        pattern: &str,
    ) -> Result<(TokenSpec, TagInfo), ParseTreePatternError> {
        let display = tag_display(name, label.as_deref());
        let first = name
            .chars()
            .next()
            .ok_or_else(|| ParseTreePatternError::InvalidTag {
                tag: name.to_owned(),
                pattern: pattern.to_owned(),
            })?;
        if first.is_uppercase() {
            let token_type = self.data.vocabulary().token_type(name).ok_or_else(|| {
                ParseTreePatternError::UnknownToken {
                    name: name.to_owned(),
                    pattern: pattern.to_owned(),
                }
            })?;
            let spec = TokenSpec::explicit(token_type, display);
            let tag = TagInfo {
                kind: TagKind::Token { token_type },
                name: name.to_owned(),
                label,
            };
            Ok((spec, tag))
        } else if first.is_lowercase() {
            let rule_index =
                self.rule_index(name)
                    .ok_or_else(|| ParseTreePatternError::UnknownRule {
                        name: name.to_owned(),
                        pattern: pattern.to_owned(),
                    })?;
            // The bypass ATN owns the imaginary-type formula, so the matcher
            // and the ATN's bypass `Atom` edges can never disagree.
            let bypass_type = self
                .bypass_atn
                .bypass_token_type(rule_index)
                .map_err(|error| ParseTreePatternError::BypassAtn {
                    message: error.to_string(),
                })?;
            let spec = TokenSpec::explicit(bypass_type, display);
            let tag = TagInfo {
                kind: TagKind::Rule {
                    rule_index,
                    bypass_type,
                },
                name: name.to_owned(),
                label,
            };
            Ok((spec, tag))
        } else {
            Err(ParseTreePatternError::InvalidTag {
                tag: name.to_owned(),
                pattern: pattern.to_owned(),
            })
        }
    }

    /// Resolves a parser rule name to its index (last wins, like the runtime's
    /// other name lookups).
    fn rule_index(&self, name: &str) -> Option<usize> {
        self.data.rule_names().iter().rposition(|rule| rule == name)
    }

    /// Interprets the hybrid token specs over the bypass ATN, producing the
    /// pattern tree and re-keying the tag table by the tokens' final store IDs.
    fn interpret(
        &self,
        specs: Vec<TokenSpec>,
        tags_by_index: &BTreeMap<usize, TagInfo>,
        rule_index: usize,
        pattern: &str,
    ) -> Result<PatternTree, ParseTreePatternError> {
        let trailing_eof = specs
            .last()
            .is_some_and(|spec| spec.token_type == TOKEN_EOF);
        let source = PatternTokenSource { specs, index: 0 };
        let mut parser = BaseParser::new(CommonTokenStream::new(source), self.data.clone());
        // ANTLR installs a BailErrorStrategy for the pattern parse: a pattern
        // the grammar only accepts through error recovery must fail loudly, not
        // bake `<missing ...>` error nodes into the pattern tree (which would
        // then match nothing). Recovery diagnostics are checked below; the
        // default console listener is removed so a rejected pattern does not
        // also print to stderr.
        parser.remove_error_listeners();
        let root = parser
            .parse_atn_rule(&self.bypass_atn, rule_index)
            .map_err(|error| ParseTreePatternError::CannotInvokeStartRule {
                rule_index,
                message: error.to_string(),
            })?;
        if parser.number_of_syntax_errors() > 0 {
            return Err(ParseTreePatternError::CannotInvokeStartRule {
                rule_index,
                message: format!(
                    "pattern is not valid for the rule: {} syntax error(s) during pattern parse",
                    parser.number_of_syntax_errors()
                ),
            });
        }

        // The start rule must consume the whole pattern (ANTLR issue #413):
        // the next visible token after the parse must be EOF.
        if parser.token_stream().la_token(1) != TOKEN_EOF {
            return Err(ParseTreePatternError::StartRuleDoesNotConsumeFullPattern {
                pattern: pattern.to_owned(),
            });
        }

        let file = parser.into_parsed_file(root);
        // A trailing `<EOF>` tag doubles as the stream terminator, so the
        // lookahead check above cannot tell "the rule matched EOF" from "the
        // tag was silently ignored". Rules that end in `EOF` put an EOF
        // terminal in the tree; require it, and reject the pattern otherwise
        // (upstream silently drops the tag here).
        if trailing_eof
            && !file.tree().descendants().any(|node| {
                node.as_terminal()
                    .is_some_and(|terminal| terminal.symbol().token_type() == TOKEN_EOF)
            })
        {
            return Err(ParseTreePatternError::StartRuleDoesNotConsumeFullPattern {
                pattern: pattern.to_owned(),
            });
        }
        let tags = rekey_tags_by_token_id(tags_by_index);
        Ok(PatternTree { file, tags })
    }
}

/// A token source over pre-lexed pattern specs, terminated by EOF — the runtime
/// analog of ANTLR's `ListTokenSource`.
#[derive(Debug)]
struct PatternTokenSource {
    specs: Vec<TokenSpec>,
    index: usize,
}

impl TokenSource for PatternTokenSource {
    fn next_token(&mut self, sink: &mut TokenSink<'_>) -> Result<TokenId, TokenStoreError> {
        let spec = self
            .specs
            .get(self.index)
            .cloned()
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
        "tree-pattern"
    }
}

/// Re-keys the tag table from flat spec indices to the [`TokenId`]s the tokens
/// occupy in the finished store.
///
/// [`PatternTokenSource`] pushes specs in order from index 0, and
/// `buffer_token_source` asserts each pushed token lands at its expected index,
/// so a tag recorded at flat index `i` occupies exactly `TokenId(i)`.
fn rekey_tags_by_token_id(tags_by_index: &BTreeMap<usize, TagInfo>) -> BTreeMap<TokenId, TagInfo> {
    tags_by_index
        .iter()
        .filter_map(|(&index, tag)| Some((TokenId::try_from(index).ok()?, tag.clone())))
        .collect()
}

/// Formats a tag for a synthetic token's text, e.g. `<expr>` or `<e:expr>`.
fn tag_display(name: &str, label: Option<&str>) -> String {
    label.map_or_else(|| format!("<{name}>"), |label| format!("<{label}:{name}>"))
}

/// The compiled pattern tree plus the tag side table describing its tag leaves.
///
/// The tree is owned as a [`ParsedFile`](crate::tree::ParsedFile); `tags` maps
/// each tag leaf's [`TokenId`] to the rule/token it stands in for. Both are
/// produced once by [`ParseTreePatternMatcher::compile`] and shared by every
/// match.
#[derive(Debug)]
struct PatternTree {
    file: crate::tree::ParsedFile,
    tags: BTreeMap<TokenId, TagInfo>,
}

/// A pattern like `<ID> = <expr>;` compiled to a reusable tree.
///
/// Created by [`ParseTreePatternMatcher::compile`] or
/// a generated parser's `compile_parse_tree_pattern`. Match a subject tree with
/// [`Self::match_tree`] (full result) or [`Self::matches`] (boolean).
#[derive(Debug)]
pub struct ParseTreePattern {
    pattern: String,
    pattern_rule_index: usize,
    tree: PatternTree,
}

impl ParseTreePattern {
    /// The tree-pattern source string this was compiled from.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The parser rule index that roots the pattern.
    #[must_use]
    pub const fn pattern_rule_index(&self) -> usize {
        self.pattern_rule_index
    }

    /// The compiled pattern as a parse tree, with tags present as terminal
    /// leaves (a rule tag is a rule node whose single child carries the
    /// imaginary bypass token).
    ///
    /// Mirrors ANTLR's `ParseTreePattern.getPatternTree`; useful for inspecting
    /// why a pattern that compiled does not match a subject —
    /// `pattern_tree().text()` renders the tag placeholders inline.
    #[must_use]
    pub fn pattern_tree(&self) -> Node<'_> {
        self.tree.file.tree()
    }

    /// Matches `tree` against this pattern, returning the full result including
    /// bound labels and the first mismatched node (if any).
    #[must_use]
    pub fn match_tree<'subject>(&self, tree: Node<'subject>) -> ParseTreeMatch<'subject> {
        let mut labels: BTreeMap<String, Vec<Node<'subject>>> = BTreeMap::new();
        let pattern_root = self.tree.file.tree();
        let mismatched = match_impl(tree, pattern_root, &self.tree.tags, &mut labels);
        ParseTreeMatch {
            tree,
            labels,
            mismatched_node: mismatched,
        }
    }

    /// Returns whether `tree` matches this pattern.
    #[must_use]
    pub fn matches(&self, tree: Node<'_>) -> bool {
        self.match_tree(tree).succeeded()
    }

    /// Finds nodes under `tree` with an `XPath` expression, then returns the
    /// successful matches of this pattern against those subtrees.
    ///
    /// Mirrors ANTLR's `ParseTreePattern.findAll`: unsuccessful matches are
    /// omitted, whatever the reason for the failure. `recognizer` resolves the
    /// rule and token names in `xpath`, exactly as in
    /// [`XPath::find_all`](crate::XPath::find_all).
    ///
    /// # Errors
    ///
    /// Returns [`XPathError`](crate::XPathError) when `xpath` is not a valid
    /// parse-tree path expression.
    pub fn find_all<'subject, R>(
        &self,
        tree: Node<'subject>,
        xpath: &str,
        recognizer: &R,
    ) -> Result<Vec<ParseTreeMatch<'subject>>, crate::XPathError>
    where
        R: Recognizer + ?Sized,
    {
        Ok(crate::XPath::find_all(tree, xpath, recognizer)?
            .into_iter()
            .map(|subtree| self.match_tree(subtree))
            .filter(ParseTreeMatch::succeeded)
            .collect())
    }
}

/// The result of matching a subject tree against a [`ParseTreePattern`].
///
/// Holds the label bindings discovered during the match and, on failure, the
/// first subject node that did not match. Borrows the subject tree.
#[derive(Clone, Debug)]
pub struct ParseTreeMatch<'subject> {
    tree: Node<'subject>,
    labels: BTreeMap<String, Vec<Node<'subject>>>,
    mismatched_node: Option<Node<'subject>>,
}

impl<'subject> ParseTreeMatch<'subject> {
    /// Returns whether the match succeeded (no node mismatched).
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        self.mismatched_node.is_none()
    }

    /// The subject tree this match was computed against.
    #[must_use]
    pub const fn tree(&self) -> Node<'subject> {
        self.tree
    }

    /// The first subject node that failed to match, or `None` on success.
    #[must_use]
    pub const fn mismatched_node(&self) -> Option<Node<'subject>> {
        self.mismatched_node
    }

    /// The last node bound to `label`, or `None` if nothing matched it.
    ///
    /// Unlabeled tags `<ID>`/`<expr>` are filed under their rule/token name, so
    /// `get("expr")` returns a node matched by `<expr>`.
    #[must_use]
    pub fn get(&self, label: &str) -> Option<Node<'subject>> {
        self.labels
            .get(label)
            .and_then(|nodes| nodes.last().copied())
    }

    /// Every node bound to `label`, in match order.
    #[must_use]
    pub fn get_all(&self, label: &str) -> &[Node<'subject>] {
        self.labels.get(label).map_or(&[], Vec::as_slice)
    }

    /// All label bindings, keyed by label name.
    #[must_use]
    pub const fn labels(&self) -> &BTreeMap<String, Vec<Node<'subject>>> {
        &self.labels
    }
}

/// Walks a subject node and a pattern node in lockstep, recording label
/// bindings and returning the first subject node that failed to match.
///
/// Faithful port of ANTLR's `ParseTreePatternMatcher.matchImpl`:
/// - two terminals match when their token types agree; if the pattern terminal
///   is a token tag it binds, else the texts must be equal;
/// - a rule node paired with a single-terminal rule-tag subtree binds if the
///   rule indices agree;
/// - otherwise two rule nodes must have equal child counts and matching
///   children;
/// - a shape mismatch (terminal vs rule) fails at the subject node.
fn match_impl<'subject>(
    tree: Node<'subject>,
    pattern: Node<'_>,
    tags: &BTreeMap<TokenId, TagInfo>,
    labels: &mut BTreeMap<String, Vec<Node<'subject>>>,
) -> Option<Node<'subject>> {
    // Grown like the runtime's other recursive tree descents
    // (`ParseTreeVisitor::visit_children`) so a deep subject/pattern pair
    // cannot overflow the native stack.
    stacker::maybe_grow(MATCH_STACK_RED_ZONE, MATCH_STACK_SIZE, || {
        match (leaf_kind(tree), leaf_kind(pattern)) {
            (Some(_), Some(_)) => match_terminals(tree, pattern, tags, labels),
            (None, None) => match_rules(tree, pattern, tags, labels),
            // One is a leaf and the other a rule: shape mismatch.
            _ => Some(tree),
        }
    })
}

/// Returns the token type of a leaf (terminal or error node), or `None` for a
/// rule node. Error nodes carry a symbol just like terminals, so they compare
/// by token type too.
fn leaf_kind(node: Node<'_>) -> Option<i32> {
    match node.kind() {
        NodeKind::Terminal => node.as_terminal().map(|t| t.symbol().token_type()),
        NodeKind::Error => node.as_error().map(|e| e.symbol().token_type()),
        NodeKind::Rule => None,
    }
}

fn match_terminals<'subject>(
    tree: Node<'subject>,
    pattern: Node<'_>,
    tags: &BTreeMap<TokenId, TagInfo>,
    labels: &mut BTreeMap<String, Vec<Node<'subject>>>,
) -> Option<Node<'subject>> {
    let tree_type = leaf_kind(tree);
    let pattern_type = leaf_kind(pattern);
    if tree_type != pattern_type {
        return Some(tree);
    }
    // A token tag binds; otherwise the concrete texts must be equal.
    match pattern_token_tag(pattern, tags) {
        Some(tag) => {
            bind(labels, tag, tree);
            None
        }
        None if leaf_text(tree) == leaf_text(pattern) => None,
        None => Some(tree),
    }
}

fn match_rules<'subject>(
    tree: Node<'subject>,
    pattern: Node<'_>,
    tags: &BTreeMap<TokenId, TagInfo>,
    labels: &mut BTreeMap<String, Vec<Node<'subject>>>,
) -> Option<Node<'subject>> {
    // `match_impl` only routes rule-kinded nodes here, so a failed view is a
    // structural inconsistency; fail the match rather than fail open.
    let (Some(tree_rule), Some(pattern_rule)) = (tree.as_rule(), pattern.as_rule()) else {
        return Some(tree);
    };

    // (expr ...) matched against a `<expr>` rule-tag subtree.
    if let Some((tag_rule_index, tag)) = rule_tag_of(pattern, tags) {
        return if tree_rule.rule_index() == tag_rule_index {
            bind(labels, tag, tree);
            None
        } else {
            Some(tree)
        };
    }

    if tree_rule.child_count() != pattern_rule.child_count() {
        return Some(tree);
    }
    for (tree_child, pattern_child) in tree.children().zip(pattern.children()) {
        if let Some(mismatch) = match_impl(tree_child, pattern_child, tags, labels) {
            return Some(mismatch);
        }
    }
    None
}

/// Returns the tag for a `<TOKEN>` terminal leaf, if this pattern leaf is one.
fn pattern_token_tag<'a>(
    pattern: Node<'_>,
    tags: &'a BTreeMap<TokenId, TagInfo>,
) -> Option<&'a TagInfo> {
    let token_id = pattern.as_terminal()?.token_id();
    let tag = tags.get(&token_id)?;
    matches!(tag.kind, TagKind::Token { .. }).then_some(tag)
}

/// Detects a `<rule>` tag subtree — a rule node with exactly one terminal child
/// whose symbol is a rule tag — returning the referenced rule index alongside
/// the tag. Mirrors ANTLR's `getRuleTagToken`.
fn rule_tag_of<'a>(
    pattern: Node<'_>,
    tags: &'a BTreeMap<TokenId, TagInfo>,
) -> Option<(usize, &'a TagInfo)> {
    let rule = pattern.as_rule()?;
    if rule.child_count() != 1 {
        return None;
    }
    let child = pattern.children().next()?;
    let token_id = child.as_terminal()?.token_id();
    let tag = tags.get(&token_id)?;
    match tag.kind {
        TagKind::Rule { rule_index, .. } => Some((rule_index, tag)),
        TagKind::Token { .. } => None,
    }
}

/// Files `node` under every label key the tag contributes.
fn bind<'subject>(
    labels: &mut BTreeMap<String, Vec<Node<'subject>>>,
    tag: &TagInfo,
    node: Node<'subject>,
) {
    for key in tag.label_keys() {
        labels.entry(key.to_owned()).or_default().push(node);
    }
}

/// Borrowed text of a leaf node (terminal or error), for literal comparison.
fn leaf_text(node: Node<'_>) -> &str {
    node.as_terminal()
        .map(crate::tree::TerminalNodeView::text)
        .or_else(|| node.as_error().map(crate::tree::ErrorNodeView::text))
        .unwrap_or("")
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;
    use crate::token::{TokenSpec, TokenStore};
    use crate::tree::{NodeId, ParseTreeStorage, ParsedFile, ParserRuleContext};

    // Fixture grammar: `stat : ID '=' expr ';' ;  expr : INT | ID ;`
    const RULE_STAT: usize = 0;
    const RULE_EXPR: usize = 1;
    const ASSIGN: i32 = 1;
    const SEMI: i32 = 2;
    const ID: i32 = 3;
    const INT: i32 = 4;
    // Imaginary bypass token types live above max_token_type (=4); rule 0's
    // would be 5, rule 1's is 6.
    const BYPASS_EXPR: i32 = 6;

    // ---- chunk splitting -------------------------------------------------

    fn split_default(pattern: &str) -> Result<Vec<Chunk>, ParseTreePatternError> {
        split(pattern, &Delimiters::default())
    }

    #[test]
    fn split_interleaves_text_and_tags() {
        let chunks = split_default("<ID> = <expr> ;").expect("valid pattern");
        insta::assert_debug_snapshot!("split_interleaves_text_and_tags", chunks);
    }

    #[test]
    fn split_parses_labeled_tags() {
        let chunks = split_default("<lhs:ID> = <e:expr>").expect("valid pattern");
        insta::assert_debug_snapshot!("split_parses_labeled_tags", chunks);
    }

    #[test]
    fn split_strips_escapes_from_text_only() {
        // Escaped delimiters are literal text; the tag survives.
        let chunks = split_default(r"a \< b <ID> c \> d").expect("valid pattern");
        insta::assert_debug_snapshot!("split_strips_escapes", chunks);
    }

    #[test]
    fn split_no_tags_is_single_text_chunk() {
        let chunks = split_default("a = 3 ;").expect("valid pattern");
        insta::assert_debug_snapshot!("split_no_tags", chunks);
    }

    #[test]
    fn split_rejects_malformed_patterns() {
        let cases = ["<ID", "ID>", "><", "<>", "<a:>", "<a<b>>"];
        let errors: Vec<_> = cases
            .into_iter()
            .map(|pattern| {
                (
                    pattern,
                    split_default(pattern).expect_err("invalid").to_string(),
                )
            })
            .collect();
        insta::assert_debug_snapshot!("split_rejects_malformed", errors);
    }

    #[test]
    fn split_honors_custom_delimiters() {
        let delimiters = Delimiters {
            start: "[[".to_owned(),
            stop: "]]".to_owned(),
            escape: "%".to_owned(),
        };
        let chunks = split("x [[expr]] y", &delimiters).expect("valid pattern");
        insta::assert_debug_snapshot!("split_custom_delimiters", chunks);
    }

    // ---- lockstep matching against hand-built pattern trees --------------

    /// Builds one tree node recursively, recording any tag leaves.
    enum Build {
        Rule(usize, Vec<Self>),
        /// A concrete terminal: token type + text.
        Token(i32, &'static str),
        /// A token tag `<name>` occupying an imaginary/real slot.
        TokenTag {
            token_type: i32,
            name: &'static str,
            label: Option<&'static str>,
        },
        /// A rule tag `<name>`: a single-terminal rule subtree.
        RuleTag {
            rule_index: usize,
            bypass_type: i32,
            name: &'static str,
            label: Option<&'static str>,
        },
    }

    struct TreeFactory {
        tokens: TokenStore,
        storage: ParseTreeStorage,
        tags: BTreeMap<TokenId, TagInfo>,
    }

    impl TreeFactory {
        fn new() -> Self {
            Self {
                tokens: TokenStore::new(None, "TreePattern"),
                storage: ParseTreeStorage::new(),
                tags: BTreeMap::new(),
            }
        }

        fn push_token(&mut self, token_type: i32, text: &str) -> TokenId {
            self.tokens
                .push(TokenSpec::explicit(token_type, text))
                .expect("test token fits")
        }

        fn build(&mut self, spec: &Build) -> NodeId {
            match spec {
                Build::Token(token_type, text) => {
                    let id = self.push_token(*token_type, text);
                    self.storage.terminal(id)
                }
                Build::TokenTag {
                    token_type,
                    name,
                    label,
                } => {
                    let id = self.push_token(*token_type, &format!("<{name}>"));
                    self.tags.insert(
                        id,
                        TagInfo {
                            kind: TagKind::Token {
                                token_type: *token_type,
                            },
                            name: (*name).to_owned(),
                            label: label.map(str::to_owned),
                        },
                    );
                    self.storage.terminal(id)
                }
                Build::RuleTag {
                    rule_index,
                    bypass_type,
                    name,
                    label,
                } => {
                    let id = self.push_token(*bypass_type, &format!("<{name}>"));
                    self.tags.insert(
                        id,
                        TagInfo {
                            kind: TagKind::Rule {
                                rule_index: *rule_index,
                                bypass_type: *bypass_type,
                            },
                            name: (*name).to_owned(),
                            label: label.map(str::to_owned),
                        },
                    );
                    // Rule tag renders as a single-terminal rule subtree.
                    let leaf = self.storage.terminal(id);
                    let mut context = ParserRuleContext::new(*rule_index, -1);
                    self.storage.add_child(&mut context, leaf);
                    self.storage.finish_rule(context)
                }
                Build::Rule(rule_index, children) => {
                    let child_ids: Vec<_> = children.iter().map(|c| self.build(c)).collect();
                    let mut context = ParserRuleContext::new(*rule_index, -1);
                    for child in child_ids {
                        self.storage.add_child(&mut context, child);
                    }
                    self.storage.finish_rule(context)
                }
            }
        }

        fn into_file(self, root: NodeId) -> (ParsedFile, BTreeMap<TokenId, TagInfo>) {
            (ParsedFile::new(self.tokens, self.storage, root), self.tags)
        }
    }

    /// Builds a subject tree (no tags expected).
    fn subject_tree(spec: &Build) -> ParsedFile {
        let mut factory = TreeFactory::new();
        let root = factory.build(spec);
        factory.into_file(root).0
    }

    /// Builds a pattern from a spec, wrapping it as a `ParseTreePattern`.
    fn pattern_from(rule_index: usize, spec: &Build) -> ParseTreePattern {
        let mut factory = TreeFactory::new();
        let root = factory.build(spec);
        let (file, tags) = factory.into_file(root);
        ParseTreePattern {
            pattern: "<test>".to_owned(),
            pattern_rule_index: rule_index,
            tree: PatternTree { file, tags },
        }
    }

    /// Subject `x = 3 ;` as `stat`.
    fn subject_x_eq_3() -> ParsedFile {
        subject_tree(&Build::Rule(
            RULE_STAT,
            vec![
                Build::Token(ID, "x"),
                Build::Token(ASSIGN, "="),
                Build::Rule(RULE_EXPR, vec![Build::Token(INT, "3")]),
                Build::Token(SEMI, ";"),
            ],
        ))
    }

    #[test]
    fn matches_rule_tag_and_binds_label() {
        // Pattern: `<ID> = <e:expr> ;`
        let pattern = pattern_from(
            RULE_STAT,
            &Build::Rule(
                RULE_STAT,
                vec![
                    Build::TokenTag {
                        token_type: ID,
                        name: "ID",
                        label: None,
                    },
                    Build::Token(ASSIGN, "="),
                    Build::RuleTag {
                        rule_index: RULE_EXPR,
                        bypass_type: BYPASS_EXPR,
                        name: "expr",
                        label: Some("e"),
                    },
                    Build::Token(SEMI, ";"),
                ],
            ),
        );
        let subject = subject_x_eq_3();
        let result = pattern.match_tree(subject.tree());

        assert!(result.succeeded(), "pattern should match");
        // Unlabeled <ID> files under "ID"; labeled <e:expr> under both "e" and "expr".
        assert_eq!(result.get("ID").map(Node::text), Some("x".to_owned()));
        assert_eq!(result.get("e").map(Node::text), Some("3".to_owned()));
        assert_eq!(result.get("expr").map(Node::text), Some("3".to_owned()));
        assert!(result.get("absent").is_none());
    }

    #[test]
    fn literal_mismatch_reports_first_bad_node() {
        // Pattern requires the identifier to be exactly `y`, subject has `x`.
        let pattern = pattern_from(
            RULE_STAT,
            &Build::Rule(
                RULE_STAT,
                vec![
                    Build::Token(ID, "y"),
                    Build::Token(ASSIGN, "="),
                    Build::RuleTag {
                        rule_index: RULE_EXPR,
                        bypass_type: BYPASS_EXPR,
                        name: "expr",
                        label: None,
                    },
                    Build::Token(SEMI, ";"),
                ],
            ),
        );
        let subject = subject_x_eq_3();
        let result = pattern.match_tree(subject.tree());

        assert!(!result.succeeded());
        assert_eq!(
            result.mismatched_node().map(Node::text),
            Some("x".to_owned())
        );
    }

    #[test]
    fn child_count_mismatch_fails_at_rule() {
        // Pattern `stat` with only 3 children vs subject's 4.
        let pattern = pattern_from(
            RULE_STAT,
            &Build::Rule(
                RULE_STAT,
                vec![
                    Build::TokenTag {
                        token_type: ID,
                        name: "ID",
                        label: None,
                    },
                    Build::Token(ASSIGN, "="),
                    Build::RuleTag {
                        rule_index: RULE_EXPR,
                        bypass_type: BYPASS_EXPR,
                        name: "expr",
                        label: None,
                    },
                ],
            ),
        );
        let subject = subject_x_eq_3();
        let result = pattern.match_tree(subject.tree());

        assert!(!result.succeeded());
        // The whole stat node mismatches on arity.
        assert!(result.mismatched_node().and_then(Node::as_rule).is_some());
    }

    #[test]
    fn rule_tag_type_mismatch_fails() {
        // A `<expr>` rule tag positioned where the subject has a `stat`.
        let pattern = pattern_from(
            RULE_STAT,
            &Build::RuleTag {
                rule_index: RULE_EXPR,
                bypass_type: BYPASS_EXPR,
                name: "expr",
                label: None,
            },
        );
        let subject = subject_x_eq_3(); // root is stat, not expr
        let result = pattern.match_tree(subject.tree());
        assert!(!result.succeeded());
    }

    #[test]
    fn get_all_returns_every_binding_in_order() {
        // Pattern `expr expr` (two INT tags) against subject with two exprs.
        let pattern = pattern_from(
            RULE_STAT,
            &Build::Rule(
                RULE_STAT,
                vec![
                    Build::RuleTag {
                        rule_index: RULE_EXPR,
                        bypass_type: BYPASS_EXPR,
                        name: "expr",
                        label: Some("operand"),
                    },
                    Build::RuleTag {
                        rule_index: RULE_EXPR,
                        bypass_type: BYPASS_EXPR,
                        name: "expr",
                        label: Some("operand"),
                    },
                ],
            ),
        );
        let subject = subject_tree(&Build::Rule(
            RULE_STAT,
            vec![
                Build::Rule(RULE_EXPR, vec![Build::Token(INT, "1")]),
                Build::Rule(RULE_EXPR, vec![Build::Token(INT, "2")]),
            ],
        ));
        let result = pattern.match_tree(subject.tree());

        assert!(result.succeeded());
        let operands: Vec<_> = result.get_all("operand").iter().map(|n| n.text()).collect();
        assert_eq!(operands, vec!["1".to_owned(), "2".to_owned()]);
        // Unlabeled rule name "expr" also collects both.
        assert_eq!(result.get_all("expr").len(), 2);
    }

    // ---- end-to-end compile() against a real ATN ------------------------

    use crate::atn::AtnStateKind;
    use crate::atn::parser_atn::{ParserAtn, ParserAtnBuilder, ParserTransitionSpec};
    use crate::vocabulary::Vocabulary;

    /// Real parser ATN for `stat : ID '=' expr ';' ;  expr : INT | ID ;`.
    ///
    /// Hand-built rather than generated so the test stays self-contained; the
    /// shape (rule start/stop, a two-alt block in `expr`, a rule-call from
    /// `stat`) exercises the bypass transform on genuine grammar structure.
    fn stat_expr_atn() -> ParserAtn {
        let mut atn = ParserAtnBuilder::new(4);
        // States: stat = 0..=6, expr = 7..=12.
        for (number, kind, rule) in [
            (0, AtnStateKind::RuleStart, 0),  // stat start
            (1, AtnStateKind::Basic, 0),      // after ID
            (2, AtnStateKind::Basic, 0),      // after '='
            (3, AtnStateKind::Basic, 0),      // after expr
            (4, AtnStateKind::RuleStop, 0),   // stat stop
            (5, AtnStateKind::RuleStart, 1),  // expr start
            (6, AtnStateKind::BlockStart, 1), // expr decision
            (7, AtnStateKind::Basic, 1),      // INT alt
            (8, AtnStateKind::Basic, 1),      // ID alt
            (9, AtnStateKind::BlockEnd, 1),   // expr block end
            (10, AtnStateKind::RuleStop, 1),  // expr stop
        ] {
            assert_eq!(
                atn.add_state(kind, Some(rule)).expect("state").index(),
                number
            );
        }
        atn.set_rule_to_start_state(vec![0, 5]).expect("starts");
        atn.set_rule_to_stop_state(vec![4, 10]).expect("stops");
        atn.set_end_state(6, 9).expect("expr block end");
        atn.add_decision_state(6).expect("decision");

        // stat : ID '=' expr ';' ;
        atn.add_transition(
            0,
            ParserTransitionSpec::Atom {
                target: 1,
                label: ID,
            },
        )
        .expect("edge");
        atn.add_transition(
            1,
            ParserTransitionSpec::Atom {
                target: 2,
                label: ASSIGN,
            },
        )
        .expect("edge");
        atn.add_transition(
            2,
            ParserTransitionSpec::Rule {
                target: 5,
                rule_index: 1,
                follow_state: 3,
                precedence: 0,
            },
        )
        .expect("edge");
        atn.add_transition(
            3,
            ParserTransitionSpec::Atom {
                target: 4,
                label: SEMI,
            },
        )
        .expect("edge");
        // Synthetic rule-return edge (expr stop -> stat follow), as a packed ATN
        // would already contain.
        atn.add_transition(10, ParserTransitionSpec::Epsilon { target: 3 })
            .expect("edge");

        // expr : INT | ID ;
        atn.add_transition(5, ParserTransitionSpec::Epsilon { target: 6 })
            .expect("edge");
        atn.add_transition(6, ParserTransitionSpec::Epsilon { target: 7 })
            .expect("edge");
        atn.add_transition(6, ParserTransitionSpec::Epsilon { target: 8 })
            .expect("edge");
        atn.add_transition(
            7,
            ParserTransitionSpec::Atom {
                target: 9,
                label: INT,
            },
        )
        .expect("edge");
        atn.add_transition(
            8,
            ParserTransitionSpec::Atom {
                target: 9,
                label: ID,
            },
        )
        .expect("edge");
        atn.add_transition(9, ParserTransitionSpec::Epsilon { target: 10 })
            .expect("edge");

        atn.finish().expect("valid stat/expr ATN")
    }

    fn stat_expr_data() -> RecognizerData {
        RecognizerData::new(
            "StatExpr.g4",
            Vocabulary::new(
                [None, Some("'='"), Some("';'"), None, None],
                [None, Some("ASSIGN"), Some("SEMI"), Some("ID"), Some("INT")],
                [None::<&str>, None],
            ),
        )
        .with_rule_names(["stat", "expr"])
    }

    /// A minimal whitespace-splitting chunk lexer for the fixture grammar.
    ///
    /// Stands in for a real generated lexer: it turns each whitespace-delimited
    /// word of a literal chunk into a token spec, classifying identifiers,
    /// integers, and the two punctuation tokens.
    fn stat_expr_chunk_lexer(text: &str) -> Result<Vec<TokenSpec>, ParseTreePatternError> {
        let mut specs = Vec::new();
        for word in text.split_whitespace() {
            let token_type = match word {
                "=" => ASSIGN,
                ";" => SEMI,
                _ if word.chars().all(|c| c.is_ascii_digit()) => INT,
                _ if word.chars().all(|c| c.is_ascii_alphanumeric()) => ID,
                other => {
                    return Err(ParseTreePatternError::Tokenization {
                        message: format!("unexpected chunk word {other:?}"),
                    });
                }
            };
            specs.push(TokenSpec::explicit(token_type, word));
        }
        Ok(specs)
    }

    fn stat_expr_matcher_and_data() -> (ParserAtn, RecognizerData) {
        (stat_expr_atn(), stat_expr_data())
    }

    #[test]
    fn compile_and_match_full_pattern() {
        let (atn, data) = stat_expr_matcher_and_data();
        let matcher = ParseTreePatternMatcher::new(&atn, &data).expect("matcher");
        let pattern = matcher
            .compile("<ID> = <e:expr> ;", RULE_STAT, stat_expr_chunk_lexer)
            .expect("compiles");

        // Subject `x = 3 ;` parsed by the same ATN (no tags).
        let mut parser = BaseParser::new(
            CommonTokenStream::new(stat_expr_subject("x = 3 ;")),
            data.clone(),
        );
        let root = parser
            .parse_atn_rule(&atn, RULE_STAT)
            .expect("subject parse");
        let subject = parser.into_parsed_file(root);

        let result = pattern.match_tree(subject.tree());
        assert!(result.succeeded(), "pattern should match `x = 3 ;`");
        assert_eq!(result.get("ID").map(Node::text), Some("x".to_owned()));
        assert_eq!(result.get("e").map(Node::text), Some("3".to_owned()));
    }

    #[test]
    fn compile_rejects_patterns_that_only_parse_via_recovery() {
        // Upstream installs a BailErrorStrategy for the pattern parse. Without
        // the syntax-error gate these all "compile" by error recovery, baking
        // `<missing ...>` error nodes into pattern trees that then match
        // nothing: missing '=', missing expr, missing leading ID.
        let (atn, data) = stat_expr_matcher_and_data();
        let matcher = ParseTreePatternMatcher::new(&atn, &data).expect("matcher");
        for pattern in ["<ID> <e:expr> ;", "<ID> = ;", "= <expr> ;", "x 3 ;"] {
            let error = matcher
                .compile(pattern, RULE_STAT, stat_expr_chunk_lexer)
                .expect_err("recovered pattern parse must be rejected");
            assert!(
                matches!(error, ParseTreePatternError::CannotInvokeStartRule { .. }),
                "unexpected error for {pattern:?}: {error}"
            );
        }
    }

    #[test]
    fn split_rejects_overlapping_tags_without_panicking() {
        // `<a<b>>` pairs starts [0, 2] with stops [4, 5]; the inter-tag text
        // slice would be inverted (5..2). Must surface as an error, not a panic.
        let error = split_default("<a<b>>").expect_err("overlapping tags");
        assert!(matches!(
            error,
            ParseTreePatternError::DelimitersOutOfOrder { .. }
        ));
    }

    #[test]
    fn compile_rejects_tokens_after_an_eof_tag() {
        // An EOF-typed token terminates the buffered stream, so a suffix after
        // `<EOF>` would silently vanish before the full-consumption check.
        let (atn, data) = stat_expr_matcher_and_data();
        let matcher = ParseTreePatternMatcher::new(&atn, &data).expect("matcher");
        let error = matcher
            .compile(
                "<ID> = <expr> ; <EOF> garbage",
                RULE_STAT,
                stat_expr_chunk_lexer,
            )
            .expect_err("tokens after an EOF tag must be rejected");
        assert!(
            matches!(error, ParseTreePatternError::Tokenization { .. }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn compile_rejects_unconsumed_trailing_eof_tag() {
        // `stat` does not end in EOF, so a trailing `<EOF>` tag can never be
        // consumed by the rule — it only terminates the token stream, and the
        // resulting tree is identical to the pattern without the tag. That
        // must be an error, not a silently dropped tag.
        let (atn, data) = stat_expr_matcher_and_data();
        let matcher = ParseTreePatternMatcher::new(&atn, &data).expect("matcher");
        let error = matcher
            .compile("<ID> = <expr> ; <EOF>", RULE_STAT, stat_expr_chunk_lexer)
            .expect_err("unconsumed trailing EOF tag must be rejected");
        assert!(
            matches!(
                error,
                ParseTreePatternError::StartRuleDoesNotConsumeFullPattern { .. }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn compile_rejects_partial_pattern() {
        // ANTLR issue #413: the start rule must consume the whole pattern. A
        // pattern that stops short of `;` leaves an unconsumed token.
        let (atn, data) = stat_expr_matcher_and_data();
        let matcher = ParseTreePatternMatcher::new(&atn, &data).expect("matcher");
        let error = matcher
            .compile("<ID> = <expr> ; extra", RULE_STAT, stat_expr_chunk_lexer)
            .expect_err("trailing token should be rejected");
        assert!(
            matches!(
                error,
                ParseTreePatternError::StartRuleDoesNotConsumeFullPattern { .. }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn compile_rejects_unknown_tag_names() {
        let (atn, data) = stat_expr_matcher_and_data();
        let matcher = ParseTreePatternMatcher::new(&atn, &data).expect("matcher");
        let unknown_token = matcher
            .compile("<NOPE> = <expr> ;", RULE_STAT, stat_expr_chunk_lexer)
            .expect_err("unknown token tag");
        assert!(matches!(
            unknown_token,
            ParseTreePatternError::UnknownToken { .. }
        ));
        let unknown_rule = matcher
            .compile("<ID> = <nope> ;", RULE_STAT, stat_expr_chunk_lexer)
            .expect_err("unknown rule tag");
        assert!(matches!(
            unknown_rule,
            ParseTreePatternError::UnknownRule { .. }
        ));
    }

    #[test]
    fn set_delimiters_validates_and_switches_tag_syntax() {
        let (atn, data) = stat_expr_matcher_and_data();
        let mut matcher = ParseTreePatternMatcher::new(&atn, &data).expect("matcher");

        // Empty start/stop are rejected like upstream's IllegalArgumentException.
        assert!(matches!(
            matcher.set_delimiters("", ">", "\\"),
            Err(ParseTreePatternError::EmptyDelimiter { which: "start" })
        ));
        assert!(matches!(
            matcher.set_delimiters("<", "", "\\"),
            Err(ParseTreePatternError::EmptyDelimiter { which: "stop" })
        ));

        // Custom delimiters compile end-to-end; the old `<...>` is now literal
        // text the chunk lexer rejects.
        matcher
            .set_delimiters("[[", "]]", "%")
            .expect("valid delimiters");
        matcher
            .compile("[[ID]] = [[e:expr]] ;", RULE_STAT, stat_expr_chunk_lexer)
            .expect("custom-delimiter pattern compiles");
        matcher
            .compile("<ID> = <expr> ;", RULE_STAT, stat_expr_chunk_lexer)
            .expect_err("old delimiters are literal text now");
    }

    #[test]
    fn compiled_pattern_does_not_match_different_structure() {
        let (atn, data) = stat_expr_matcher_and_data();
        let matcher = ParseTreePatternMatcher::new(&atn, &data).expect("matcher");
        // Pattern requires the literal identifier `y`.
        let pattern = matcher
            .compile("y = <expr> ;", RULE_STAT, stat_expr_chunk_lexer)
            .expect("compiles");

        let mut parser = BaseParser::new(
            CommonTokenStream::new(stat_expr_subject("x = 3 ;")),
            data.clone(),
        );
        let root = parser
            .parse_atn_rule(&atn, RULE_STAT)
            .expect("subject parse");
        let subject = parser.into_parsed_file(root);

        let result = pattern.match_tree(subject.tree());
        assert!(
            !result.succeeded(),
            "identifier `x` should not match literal `y`"
        );
    }

    /// Subject-side token source: lexes a whole input string like the chunk
    /// lexer, then appends EOF.
    fn stat_expr_subject(input: &str) -> PatternTokenSource {
        let specs = stat_expr_chunk_lexer(input).expect("valid subject input");
        PatternTokenSource { specs, index: 0 }
    }

    #[derive(Debug)]
    struct StatExprRecognizer {
        data: RecognizerData,
    }

    impl Recognizer for StatExprRecognizer {
        fn data(&self) -> &RecognizerData {
            &self.data
        }

        fn data_mut(&mut self) -> &mut RecognizerData {
            &mut self.data
        }
    }

    #[test]
    fn find_all_pairs_xpath_selection_with_pattern_matching() {
        let (atn, data) = stat_expr_matcher_and_data();
        let matcher = ParseTreePatternMatcher::new(&atn, &data).expect("matcher");
        // Matches only integer expressions.
        let pattern = matcher
            .compile("<INT>", RULE_EXPR, stat_expr_chunk_lexer)
            .expect("compiles");

        let mut parser = BaseParser::new(
            CommonTokenStream::new(stat_expr_subject("x = 3 ;")),
            data.clone(),
        );
        let root = parser
            .parse_atn_rule(&atn, RULE_STAT)
            .expect("subject parse");
        let subject = parser.into_parsed_file(root);
        let recognizer = StatExprRecognizer { data };

        // `//expr` selects the one expr subtree; the `<INT>` pattern matches it.
        let matches = pattern
            .find_all(subject.tree(), "//expr", &recognizer)
            .expect("valid xpath");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].tree().text(), "3");
        // A path selecting nothing that matches yields no results.
        let none = pattern
            .find_all(subject.tree(), "//stat", &recognizer)
            .expect("valid xpath");
        assert!(none.is_empty());
    }
}
