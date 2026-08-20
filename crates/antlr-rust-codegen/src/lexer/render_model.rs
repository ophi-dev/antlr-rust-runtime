// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
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
///
/// `lex` and `lex_stream` are grammar-independent generic functions owned by
/// the runtime's `generated` module; the lexer module re-exports them so the
/// established `my_lexer::lex(...)` call sites keep resolving.
pub(crate) fn render_lexer_lex_convenience() -> String {
    "pub use antlr4_runtime::generated::{lex, lex_stream};".to_owned()
}

/// Renders the lexer-owned grammar metadata table.
///
/// The serialized ATN travels as an encoded blob string literal (see
/// `antlr4_runtime::encoded`) instead of a decimal integer array.
pub(crate) fn render_lexer_metadata(grammar_name: &str, data: &LexerCodegenData<'_>) -> String {
    format!(
        "pub static METADATA: GrammarMetadata = GrammarMetadata::new_with_encoded_atn(\n    \"{}\",\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n    {},\n);\n\npub fn metadata() -> &'static GrammarMetadata {{\n    &METADATA\n}}\n\npub fn rule_names() -> &'static [&'static str] {{\n    METADATA.rule_names()\n}}\n",
        rust_string(grammar_name),
        render_lexer_str_slice(&data.rule_names),
        render_lexer_option_str_slice(&data.literal_names),
        render_lexer_option_str_slice(&data.symbolic_names),
        render_lexer_empty_option_str_slice(max_len(&data.literal_names, &data.symbolic_names)),
        render_lexer_str_slice(&data.channel_names),
        render_lexer_str_slice(&data.mode_names),
        rust_encoded_blob_literal(&antlr4_runtime::encoded::encode_i32_values(
            &data.lexer_atn_words
        ))
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
    let mut declaration_order: Vec<(&String, N)> = numbers
        .iter()
        .map(|(name, number)| (name, *number))
        .collect();
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
