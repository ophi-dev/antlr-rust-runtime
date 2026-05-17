use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse()?;
    fs::create_dir_all(&args.out_dir)?;

    if let Some(lexer) = args.lexer {
        let data = InterpData::parse(&fs::read_to_string(&lexer)?)?;
        let grammar_name = args
            .lexer_name
            .clone()
            .unwrap_or_else(|| grammar_name_from_path(&lexer));
        let module = render_lexer(&grammar_name, &data);
        fs::write(
            args.out_dir
                .join(format!("{}.rs", module_name(&grammar_name))),
            module,
        )?;
    }

    if let Some(parser) = args.parser {
        let data = InterpData::parse(&fs::read_to_string(&parser)?)?;
        let grammar_name = args
            .parser_name
            .clone()
            .unwrap_or_else(|| grammar_name_from_path(&parser));
        let module = render_parser(&grammar_name, &data);
        fs::write(
            args.out_dir
                .join(format!("{}.rs", module_name(&grammar_name))),
            module,
        )?;
    }

    Ok(())
}

#[derive(Debug)]
struct Args {
    lexer: Option<PathBuf>,
    parser: Option<PathBuf>,
    lexer_name: Option<String>,
    parser_name: Option<String>,
    out_dir: PathBuf,
}

impl Args {
    /// Parses the small generator CLI surface without pulling in a command-line
    /// dependency.
    ///
    /// This binary is intended to stay easy to vendor into build pipelines, so
    /// the parser deliberately accepts only the flags the runtime target needs
    /// today: lexer/parser `.interp` inputs, optional grammar names, and an
    /// output directory.
    fn parse() -> Result<Self, String> {
        let mut lexer = None;
        let mut parser = None;
        let mut lexer_name = None;
        let mut parser_name = None;
        let mut out_dir = None;

        let mut iter = env::args().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--lexer" => lexer = Some(PathBuf::from(next_arg(&mut iter, "--lexer")?)),
                "--parser" => parser = Some(PathBuf::from(next_arg(&mut iter, "--parser")?)),
                "--lexer-name" => lexer_name = Some(next_arg(&mut iter, "--lexer-name")?),
                "--parser-name" => parser_name = Some(next_arg(&mut iter, "--parser-name")?),
                "--out-dir" => out_dir = Some(PathBuf::from(next_arg(&mut iter, "--out-dir")?)),
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unknown argument {other}\n\n{}", usage())),
            }
        }

        if lexer.is_none() && parser.is_none() {
            return Err(format!(
                "at least one of --lexer or --parser is required\n\n{}",
                usage()
            ));
        }

        Ok(Self {
            lexer,
            parser,
            lexer_name,
            parser_name,
            out_dir: out_dir.unwrap_or_else(|| PathBuf::from(".")),
        })
    }
}

fn next_arg(iter: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value\n\n{}", usage()))
}

fn usage() -> String {
    "usage: antlr4-rust-gen [--lexer Lexer.interp] [--parser Parser.interp] [--out-dir DIR]"
        .to_owned()
}

#[derive(Clone, Debug, Default)]
struct InterpData {
    literal_names: Vec<Option<String>>,
    symbolic_names: Vec<Option<String>>,
    rule_names: Vec<String>,
    channel_names: Vec<String>,
    mode_names: Vec<String>,
    atn: Vec<i32>,
}

impl InterpData {
    /// Parses ANTLR `.interp` files emitted next to generated grammars.
    ///
    /// The `.interp` format is line-oriented metadata followed by one serialized
    /// ATN integer array. We use it as the clean-room bridge from the official
    /// ANTLR tool to generated Rust metadata without reading or translating
    /// another target's generated source.
    fn parse(input: &str) -> Result<Self, io::Error> {
        let mut data = Self::default();
        let mut section = Section::None;
        let mut atn_text = String::new();

        for line in input.lines() {
            let trimmed = line.trim();
            section = match trimmed {
                "token literal names:" => Section::LiteralNames,
                "token symbolic names:" => Section::SymbolicNames,
                "rule names:" => Section::RuleNames,
                "channel names:" => Section::ChannelNames,
                "mode names:" => Section::ModeNames,
                "atn:" => Section::Atn,
                _ => section,
            };

            if matches!(
                trimmed,
                "token literal names:"
                    | "token symbolic names:"
                    | "rule names:"
                    | "channel names:"
                    | "mode names:"
                    | "atn:"
            ) {
                continue;
            }

            match section {
                Section::None => {}
                Section::LiteralNames => data.literal_names.push(parse_optional_name(trimmed)),
                Section::SymbolicNames => data.symbolic_names.push(parse_optional_name(trimmed)),
                Section::RuleNames => {
                    if !trimmed.is_empty() {
                        data.rule_names.push(trimmed.to_owned());
                    }
                }
                Section::ChannelNames => {
                    if !trimmed.is_empty() {
                        data.channel_names.push(trimmed.to_owned());
                    }
                }
                Section::ModeNames => {
                    if !trimmed.is_empty() {
                        data.mode_names.push(trimmed.to_owned());
                    }
                }
                Section::Atn => atn_text.push_str(trimmed),
            }
        }

        data.atn = parse_atn_values(&atn_text)?;
        Ok(data)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Section {
    None,
    LiteralNames,
    SymbolicNames,
    RuleNames,
    ChannelNames,
    ModeNames,
    Atn,
}

fn parse_optional_name(value: &str) -> Option<String> {
    match value {
        "" | "null" => None,
        other => Some(other.to_owned()),
    }
}

/// Parses the bracketed serialized ATN integer array from an `.interp` file.
fn parse_atn_values(value: &str) -> Result<Vec<i32>, io::Error> {
    let body = value.trim().trim_start_matches('[').trim_end_matches(']');
    if body.is_empty() {
        return Ok(Vec::new());
    }
    body.split(',')
        .map(|part| {
            part.trim().parse::<i32>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid ATN integer {:?}: {error}", part.trim()),
                )
            })
        })
        .collect()
}

/// Renders a Rust lexer module that delegates token recognition to the shared
/// ATN interpreter.
///
/// The emitted lexer owns only generated metadata and a `BaseLexer`. Keeping
/// recognition in the runtime avoids emitting thousands of lines of
/// grammar-specific Rust control flow for the first target implementation.
fn render_lexer(grammar_name: &str, data: &InterpData) -> String {
    let type_name = rust_type_name(grammar_name);
    let metadata = render_metadata(grammar_name, data);
    let token_constants = render_token_constants(data);

    format!(
        r#"use antlr4_runtime::char_stream::CharStream;
use antlr4_runtime::recognizer::RecognizerData;
use antlr4_runtime::token::{{CommonToken, TokenSource}};
use antlr4_runtime::atn::Atn;
use antlr4_runtime::atn::serialized::AtnDeserializer;
use antlr4_runtime::{{BaseLexer, GeneratedLexer, GrammarMetadata, Lexer, Recognizer}};
use std::sync::OnceLock;

{token_constants}
{metadata}

static ATN: OnceLock<Atn> = OnceLock::new();

/// Deserializes and caches the grammar ATN for all lexer instances.
fn atn() -> &'static Atn {{
    ATN.get_or_init(|| {{
        let serialized = METADATA.serialized_atn();
        AtnDeserializer::new(&serialized)
            .deserialize()
            .expect("generated lexer contains a valid ANTLR serialized ATN")
    }})
}}

#[derive(Clone, Debug)]
pub struct {type_name}<I>
where
    I: CharStream,
{{
    base: BaseLexer<I>,
}}

impl<I> {type_name}<I>
where
    I: CharStream,
{{
    pub fn new(input: I) -> Self {{
        let metadata = Self::metadata();
        let data = RecognizerData::new(metadata.grammar_file_name(), metadata.vocabulary())
            .with_rule_names(metadata.rule_names().iter().copied())
            .with_channel_names(metadata.channel_names().iter().copied())
            .with_mode_names(metadata.mode_names().iter().copied());
        Self {{ base: BaseLexer::new(input, data) }}
    }}

    pub fn metadata() -> &'static GrammarMetadata {{
        &METADATA
    }}
}}

impl<I> GeneratedLexer for {type_name}<I>
where
    I: CharStream,
{{
    fn metadata() -> &'static GrammarMetadata {{
        &METADATA
    }}
}}

impl<I> Recognizer for {type_name}<I>
where
    I: CharStream,
{{
    fn data(&self) -> &antlr4_runtime::RecognizerData {{
        self.base.data()
    }}

    fn data_mut(&mut self) -> &mut antlr4_runtime::RecognizerData {{
        self.base.data_mut()
    }}
}}

impl<I> Lexer for {type_name}<I>
where
    I: CharStream,
{{
    fn mode(&self) -> i32 {{ self.base.mode() }}
    fn set_mode(&mut self, mode: i32) {{ self.base.set_mode(mode); }}
    fn push_mode(&mut self, mode: i32) {{ self.base.push_mode(mode); }}
    fn pop_mode(&mut self) -> Option<i32> {{ self.base.pop_mode() }}
}}

impl<I> TokenSource for {type_name}<I>
where
    I: CharStream,
{{
    fn next_token(&mut self) -> CommonToken {{
        antlr4_runtime::atn::lexer::next_token(&mut self.base, atn())
    }}

    fn line(&self) -> usize {{ self.base.line() }}
    fn column(&self) -> usize {{ self.base.column() }}
    fn source_name(&self) -> &str {{ self.base.source_name() }}
}}
"#
    )
}

/// Renders a Rust parser module with one public method per grammar rule.
///
/// Parser methods currently route through the runtime parser interpreter entry
/// point. As the parser ATN simulator matures, the generated surface can remain
/// stable while the interpreter becomes semantically complete.
fn render_parser(grammar_name: &str, data: &InterpData) -> String {
    let type_name = rust_type_name(grammar_name);
    let metadata = render_metadata(grammar_name, data);
    let token_constants = render_token_constants(data);
    let rule_constants = render_rule_constants(data);
    let mut rule_methods = String::new();
    for (index, rule) in data.rule_names.iter().enumerate() {
        writeln!(
            rule_methods,
            "    pub fn {}(&mut self) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{",
            rust_function_name(rule)
        )
        .expect("writing to a string cannot fail");
        writeln!(
            rule_methods,
            "        self.base.parse_atn_rule(atn(), {index})"
        )
        .expect("writing to a string cannot fail");
        writeln!(rule_methods, "    }}").expect("writing to a string cannot fail");
    }

    format!(
        r#"use antlr4_runtime::recognizer::RecognizerData;
use antlr4_runtime::token::TokenSource;
use antlr4_runtime::token_stream::CommonTokenStream;
use antlr4_runtime::atn::Atn;
use antlr4_runtime::atn::serialized::AtnDeserializer;
use antlr4_runtime::{{BaseParser, GeneratedParser, GrammarMetadata, Parser, Recognizer}};
use std::sync::OnceLock;

{token_constants}
{rule_constants}
{metadata}

static ATN: OnceLock<Atn> = OnceLock::new();

/// Deserializes and caches the grammar ATN for all parser instances.
fn atn() -> &'static Atn {{
    ATN.get_or_init(|| {{
        let serialized = METADATA.serialized_atn();
        AtnDeserializer::new(&serialized)
            .deserialize()
            .expect("generated parser contains a valid ANTLR serialized ATN")
    }})
}}

#[derive(Debug)]
pub struct {type_name}<S>
where
    S: TokenSource,
{{
    base: BaseParser<S>,
}}

impl<S> {type_name}<S>
where
    S: TokenSource,
{{
    pub fn new(input: CommonTokenStream<S>) -> Self {{
        let metadata = Self::metadata();
        let data = RecognizerData::new(metadata.grammar_file_name(), metadata.vocabulary())
            .with_rule_names(metadata.rule_names().iter().copied())
            .with_channel_names(metadata.channel_names().iter().copied())
            .with_mode_names(metadata.mode_names().iter().copied());
        Self {{ base: BaseParser::new(input, data) }}
    }}

    pub fn metadata() -> &'static GrammarMetadata {{
        &METADATA
    }}

{rule_methods}
}}

impl<S> GeneratedParser for {type_name}<S>
where
    S: TokenSource,
{{
    fn metadata() -> &'static GrammarMetadata {{
        &METADATA
    }}
}}

impl<S> Recognizer for {type_name}<S>
where
    S: TokenSource,
{{
    fn data(&self) -> &antlr4_runtime::RecognizerData {{
        self.base.data()
    }}

    fn data_mut(&mut self) -> &mut antlr4_runtime::RecognizerData {{
        self.base.data_mut()
    }}
}}

impl<S> Parser for {type_name}<S>
where
    S: TokenSource,
{{
    fn build_parse_trees(&self) -> bool {{ self.base.build_parse_trees() }}
    fn set_build_parse_trees(&mut self, build: bool) {{ self.base.set_build_parse_trees(build); }}
}}
"#
    )
}

/// Renders static grammar metadata shared by generated lexers and parsers.
fn render_metadata(grammar_name: &str, data: &InterpData) -> String {
    format!(
        "pub static METADATA: GrammarMetadata = GrammarMetadata::new(\n    \"{}\",\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n    &{},\n);\n",
        rust_string(grammar_name),
        render_str_slice(&data.rule_names),
        render_option_str_slice(&data.literal_names),
        render_option_str_slice(&data.symbolic_names),
        render_empty_option_str_slice(max_len(&data.literal_names, &data.symbolic_names)),
        render_str_slice(&data.channel_names),
        render_str_slice(&data.mode_names),
        render_i32_slice(&data.atn)
    )
}

/// Renders token constants from symbolic token names while avoiding duplicate
/// Rust identifiers after sanitization.
fn render_token_constants(data: &InterpData) -> String {
    let mut out = String::from("pub const EOF: i32 = antlr4_runtime::TOKEN_EOF;\n");
    let mut seen = BTreeSet::new();
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

/// Renders rule-index constants from grammar rule names.
fn render_rule_constants(data: &InterpData) -> String {
    let mut out = String::new();
    for (index, name) in data.rule_names.iter().enumerate() {
        writeln!(
            out,
            "pub const RULE_{}: usize = {index};",
            rust_const_name(name)
        )
        .expect("writing to a string cannot fail");
    }
    out
}

/// Renders an `&[Option<&str>]` expression for literal or symbolic names.
fn render_option_str_slice(values: &[Option<String>]) -> String {
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

/// Renders an empty optional string table with a fixed length.
fn render_empty_option_str_slice(len: usize) -> String {
    let items = (0..len).map(|_| "None").collect::<Vec<_>>().join(", ");
    format!("[{items}]")
}

/// Renders an `&[&str]` expression for rule/channel/mode names.
fn render_str_slice(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| format!("\"{}\"", rust_string(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

/// Renders a line-wrapped `&[i32]` expression for serialized ATN data.
fn render_i32_slice(values: &[i32]) -> String {
    let items = values
        .iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

fn max_len(left: &[Option<String>], right: &[Option<String>]) -> usize {
    left.len().max(right.len())
}

/// Derives a grammar name from an input file stem when the user does not pass
/// an explicit `--lexer-name` or `--parser-name`.
fn grammar_name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Grammar")
        .to_owned()
}

/// Converts a grammar type name into a snake-case module file name.
fn module_name(name: &str) -> String {
    split_identifier_words(name).join("_")
}

/// Converts an ANTLR grammar name into a Rust type name.
fn rust_type_name(name: &str) -> String {
    split_identifier_words(name)
        .into_iter()
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                let mut out = String::with_capacity(part.len());
                out.push(first.to_ascii_uppercase());
                out.push_str(chars.as_str());
                out
            })
        })
        .collect()
}

/// Converts an ANTLR token/rule name into an upper-snake Rust constant name.
fn rust_const_name(name: &str) -> String {
    let words = split_identifier_words(name);
    let ident = if words.is_empty() {
        "TOKEN".to_owned()
    } else {
        ascii_uppercase(&words.join("_"))
    };
    sanitize_identifier(&ident)
}

/// Converts an ANTLR rule name into a snake-case Rust method name.
fn rust_function_name(name: &str) -> String {
    let words = split_identifier_words(name);
    let ident = if words.is_empty() {
        "rule".to_owned()
    } else {
        words.join("_")
    };
    let ident = sanitize_identifier(&ident);
    if is_rust_keyword(&ident) {
        format!("r#{ident}")
    } else {
        ident
    }
}

/// Splits mixed-case, snake-case, and punctuation-heavy grammar identifiers
/// into words for Rust identifier rendering.
fn split_identifier_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = name.chars().collect();
    for (index, ch) in chars.iter().copied().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(ascii_lowercase(&current));
                current.clear();
            }
            continue;
        }

        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        let starts_new_word = !current.is_empty()
            && ch.is_ascii_uppercase()
            && (previous.is_some_and(|prev| prev.is_ascii_lowercase() || prev.is_ascii_digit())
                || (previous.is_some_and(|prev| prev.is_ascii_uppercase())
                    && next.is_some_and(|next| next.is_ascii_lowercase())));

        if starts_new_word {
            words.push(ascii_lowercase(&current));
            current.clear();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(ascii_lowercase(&current));
    }
    words
}

/// Produces a legal Rust identifier and appends an underscore for keywords.
fn sanitize_identifier(value: &str) -> String {
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            if index == 0 && ch.is_ascii_digit() {
                out.push('_');
            }
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "_".to_owned() } else { out }
}

/// Returns true for Rust reserved and contextual keywords that cannot be used
/// directly as generated identifiers.
fn is_rust_keyword(value: &str) -> bool {
    matches!(
        value,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "gen"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "Self"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "try"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
    )
}

/// Escapes a Rust string literal using explicit ASCII escape forms.
fn rust_string(value: &str) -> String {
    value.escape_default().to_string()
}

/// Converts ASCII letters to lower case without using allocation-hiding string
/// case helpers disallowed by the strict Clippy policy.
fn ascii_lowercase(value: &str) -> String {
    value.chars().map(|ch| ch.to_ascii_lowercase()).collect()
}

/// Converts ASCII letters to upper case without using allocation-hiding string
/// case helpers disallowed by the strict Clippy policy.
fn ascii_uppercase(value: &str) -> String {
    value.chars().map(|ch| ch.to_ascii_uppercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_interp_sections() {
        let data = InterpData::parse(
            r#"token literal names:
null
'x'
token symbolic names:
null
X
rule names:
file
channel names:
DEFAULT_TOKEN_CHANNEL
HIDDEN
mode names:
DEFAULT_MODE
atn:
[4, 1, 1, 0]
"#,
        )
        .expect("interp data should parse");
        assert_eq!(data.literal_names[1], Some("'x'".to_owned()));
        assert_eq!(data.symbolic_names[1], Some("X".to_owned()));
        assert_eq!(data.rule_names, ["file"]);
        assert_eq!(data.atn, [4, 1, 1, 0]);
    }

    #[test]
    fn converts_names_to_rust_identifiers() {
        assert_eq!(module_name("KotlinLexer"), "kotlin_lexer");
        assert_eq!(rust_function_name("kotlinFile"), "kotlin_file");
        assert_eq!(rust_const_name("LPAREN"), "LPAREN");
        assert_eq!(rust_const_name("Q_COLONCOLON"), "Q_COLONCOLON");
        assert_eq!(rust_const_name("LineStrExprStart"), "LINE_STR_EXPR_START");
        assert_eq!(rust_const_name("UnicodeClassLL"), "UNICODE_CLASS_LL");
        assert_eq!(rust_function_name("gen"), "r#gen");
        assert_eq!(rust_function_name("try"), "r#try");
        assert_eq!(rust_function_name("Self"), "r#self");
        assert!(is_rust_keyword("Self"));
    }
}
