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

/// Renders lexer token constants without coupling lexer emission to parser
/// surface rendering.
pub(crate) fn render_lexer_token_constants(data: &LexerCodegenData<'_>) -> String {
    let mut out = String::from("pub const EOF: i32 = antlr4_runtime::TOKEN_EOF;\n");
    let mut seen = BTreeSet::new();
    if let Some(semantic) = data.semantic {
        let vocabulary = &semantic.recognizer.vocabulary;
        for name in vocabulary
            .name_order
            .iter()
            .filter(|name| name.starts_with("T__"))
        {
            let ident = sanitize_identifier(name);
            let _ = seen.insert(ident.clone());
            let token_type = vocabulary.by_name[name];
            writeln!(out, "pub const {ident}: i32 = {token_type};")
                .expect("writing to a string cannot fail");
        }
    }
    for (index, name) in data.symbolic_names.iter().enumerate() {
        let Some(name) = name else { continue };
        let ident = rust_const_name(name);
        if ident == "EOF" || !seen.insert(ident.clone()) {
            continue;
        }
        writeln!(out, "pub const {ident}: i32 = {index};")
            .expect("writing to a string cannot fail");
    }
    out
}

pub(crate) fn render_lexer_state_constants(data: &LexerCodegenData<'_>) -> String {
    let mut out = String::new();
    for (name, number) in &data.channel_numbers {
        writeln!(
            out,
            "pub const CHANNEL_{}: i32 = {number};",
            rust_const_name(name)
        )
        .expect("writing to a string cannot fail");
    }
    for (name, number) in &data.mode_numbers {
        writeln!(
            out,
            "pub const MODE_{}: i32 = {number};",
            rust_const_name(name)
        )
        .expect("writing to a string cannot fail");
    }
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
