/// Complete, immutable input to lexer Rust emission.
///
/// Constructing this value before calling the renderer keeps CLI/configuration
/// concerns out of lexer emission and gives future render-model passes one
/// stage-owned input.
#[derive(Clone, Copy)]
pub(crate) struct LexerRenderModel<'a> {
    pub(crate) grammar_name: &'a str,
    pub(crate) data: &'a LexerCodegenData<'a>,
    pub(crate) allow_unsupported_lexer_actions: bool,
    pub(crate) sem_unknown: SemUnknownPolicy,
    pub(crate) patterns: &'a SemPatternFile,
    pub(crate) embedded: bool,
}

impl<'a> LexerRenderModel<'a> {
    pub(crate) const fn new(
        grammar_name: &'a str,
        data: &'a LexerCodegenData<'a>,
        allow_unsupported_lexer_actions: bool,
        sem_unknown: SemUnknownPolicy,
        patterns: &'a SemPatternFile,
        embedded: bool,
    ) -> Self {
        Self {
            grammar_name,
            data,
            allow_unsupported_lexer_actions,
            sem_unknown,
            patterns,
            embedded,
        }
    }
}

/// Renders lexer-module conveniences that buffer text or a caller-provided
/// character stream without constructing a parser.
pub(crate) fn render_lexer_lex_convenience(type_name: &str) -> String {
    format!(
        r#"/// Lexes UTF-8 text into an eagerly filled token stream without constructing
/// a parser.
///
/// The stream retains every emitted token, including EOF and tokens on hidden
/// or custom channels. Lexer rules using `skip` do not emit tokens.
///
/// With `use antlr4_runtime::Token as _;`, print each token's vocabulary name,
/// numeric channel, and text:
/// `let vocabulary = metadata().vocabulary(); for token in lex(src, {type_name}::new).tokens() {{ println!("type={{}} channel={{}} text={{:?}}", vocabulary.display_name(token.token_type()), token.channel(), token.text()); }}`
///
/// `number_of_source_errors()` reports buffered lexer diagnostics. After
/// iterating `tokens()`, `drain_source_errors()` retrieves those diagnostics.
///
/// # Panics
///
/// Panics if buffering returns an [`antlr4_runtime::TokenStoreError`]. Construct
/// the lexer and call [`antlr4_runtime::CommonTokenStream::try_new`] to handle
/// that error instead.
pub fn lex<L: antlr4_runtime::TokenSource>(
    input: impl AsRef<str>,
    lexer: impl FnOnce(antlr4_runtime::InputStream) -> L,
) -> antlr4_runtime::CommonTokenStream<L> {{
    lex_stream(antlr4_runtime::InputStream::new(input.as_ref()), lexer)
}}

/// Lexes a caller-provided character stream without constructing a parser.
///
/// Unlike [`lex`], this accepts any [`antlr4_runtime::CharStream`], including a
/// named [`antlr4_runtime::InputStream`] or a byte-oriented
/// [`antlr4_runtime::ByteStream`].
///
/// `number_of_source_errors()` reports buffered lexer diagnostics. After
/// iterating `tokens()`, `drain_source_errors()` retrieves those diagnostics.
///
/// # Panics
///
/// Panics if buffering returns an [`antlr4_runtime::TokenStoreError`]. Call
/// [`antlr4_runtime::CommonTokenStream::try_new`] with the constructed lexer to
/// handle that error instead.
pub fn lex_stream<I: antlr4_runtime::CharStream, L: antlr4_runtime::TokenSource>(
    input: I,
    lexer: impl FnOnce(I) -> L,
) -> antlr4_runtime::CommonTokenStream<L> {{
    antlr4_runtime::CommonTokenStream::new(lexer(input))
}}"#
    )
}

/// Renders the lexer-owned grammar metadata table.
pub(crate) fn render_lexer_metadata(grammar_name: &str, data: &LexerCodegenData<'_>) -> String {
    format!(
        "pub static METADATA: GrammarMetadata = GrammarMetadata::new(\n    \"{}\",\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n);\n\npub fn metadata() -> &'static GrammarMetadata {{\n    &METADATA\n}}\n\npub fn rule_names() -> &'static [&'static str] {{\n    METADATA.rule_names()\n}}\n",
        rust_string(grammar_name),
        render_lexer_str_slice(&data.rule_names),
        render_lexer_option_str_slice(&data.literal_names),
        render_lexer_option_str_slice(&data.symbolic_names),
        render_lexer_empty_option_str_slice(max_len(
            &data.literal_names,
            &data.symbolic_names
        )),
        render_lexer_str_slice(&data.channel_names),
        render_lexer_str_slice(&data.mode_names),
        render_lexer_i32_slice(&data.lexer_atn_words)
    )
}

/// Renders `CHANNEL_*` / `MODE_*` constants for one prefix.
///
/// Identifiers are allocated in declaration order (channel/mode number), so
/// the first-declared name keeps the canonical identifier and a `_2` suffix
/// never reads like a channel or mode number, while emission keeps the
/// name-ordered layout so non-colliding grammars render byte-identically.
fn render_lexer_prefixed_constants<N: Copy + Ord + std::fmt::Display>(
    prefix: &str,
    numbers: &BTreeMap<String, N>,
    used: &mut BTreeSet<String>,
) -> String {
    let mut declaration_order: Vec<(&String, N)> =
        numbers.iter().map(|(name, number)| (name, *number)).collect();
    declaration_order.sort_by_key(|&(_, number)| number);
    let idents: BTreeMap<&String, String> = declaration_order
        .into_iter()
        .map(|(name, _)| (name, allocate_const_name(prefix, name, used)))
        .collect();
    let mut out = String::new();
    for (name, number) in numbers {
        writeln!(out, "pub const {}: i32 = {number};", idents[name])
            .expect("writing to a string cannot fail");
    }
    out
}

/// Renders lexer channel and mode constants. Both prefixes share the
/// generated module's `used` identifier set with the token constants, since
/// a token named `ChannelFoo` or `ModeBar` would otherwise collide with a
/// channel `foo` or mode `bar`.
pub(crate) fn render_lexer_state_constants(
    data: &LexerCodegenData<'_>,
    used: &mut BTreeSet<String>,
) -> String {
    let mut out = render_lexer_prefixed_constants("CHANNEL_", &data.channel_numbers, used);
    out.push_str(&render_lexer_prefixed_constants(
        "MODE_",
        &data.mode_numbers,
        used,
    ));
    out
}

fn render_lexer_option_str_slice(values: &[Option<String>]) -> String {
    let items = values
        .iter()
        .map(|value| {
            value.as_ref().map_or_else(
                || "None".to_owned(),
                |value| format!("Some(\"{}\")", rust_string(value)),
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

fn render_lexer_empty_option_str_slice(len: usize) -> String {
    let items = (0..len).map(|_| "None").collect::<Vec<_>>().join(", ");
    format!("[{items}]")
}

fn render_lexer_str_slice(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| format!("\"{}\"", rust_string(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

fn render_lexer_i32_slice(values: &[i32]) -> String {
    let items = values
        .iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}
