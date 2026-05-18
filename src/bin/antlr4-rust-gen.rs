use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use antlr4_runtime::atn::serialized::{AtnDeserializer, SerializedAtn};
use antlr4_runtime::atn::{LexerAction, Transition};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse()?;
    fs::create_dir_all(&args.out_dir)?;
    let grammar_source = args
        .grammar
        .as_deref()
        .map(fs::read_to_string)
        .transpose()?;

    if let Some(lexer) = args.lexer {
        let data = InterpData::parse(&fs::read_to_string(&lexer)?)?;
        let grammar_name = args
            .lexer_name
            .clone()
            .unwrap_or_else(|| grammar_name_from_path(&lexer));
        let module = render_lexer(&grammar_name, &data, grammar_source.as_deref())?;
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
        let module = render_parser(&grammar_name, &data, grammar_source.as_deref())?;
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
    grammar: Option<PathBuf>,
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
        let mut grammar = None;
        let mut out_dir = None;

        let mut iter = env::args().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--lexer" => lexer = Some(PathBuf::from(next_arg(&mut iter, "--lexer")?)),
                "--parser" => parser = Some(PathBuf::from(next_arg(&mut iter, "--parser")?)),
                "--lexer-name" => lexer_name = Some(next_arg(&mut iter, "--lexer-name")?),
                "--parser-name" => parser_name = Some(next_arg(&mut iter, "--parser-name")?),
                "--grammar" => grammar = Some(PathBuf::from(next_arg(&mut iter, "--grammar")?)),
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
            grammar,
            out_dir: out_dir.unwrap_or_else(|| PathBuf::from(".")),
        })
    }
}

fn next_arg(iter: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value\n\n{}", usage()))
}

fn usage() -> String {
    "usage: antlr4-rust-gen [--lexer Lexer.interp] [--parser Parser.interp] [--grammar Grammar.g4] [--out-dir DIR]"
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
fn render_lexer(
    grammar_name: &str,
    data: &InterpData,
    grammar_source: Option<&str>,
) -> io::Result<String> {
    let type_name = rust_type_name(grammar_name);
    let metadata = render_metadata(grammar_name, data);
    let token_constants = render_token_constants(data);
    let actions = grammar_source.map_or_else(
        || Ok(Vec::new()),
        |source| lexer_action_templates(data, source),
    )?;
    let predicates = grammar_source.map_or_else(
        || Ok(Vec::new()),
        |source| lexer_predicate_templates(data, source),
    )?;
    let action_method = render_lexer_action_method(&actions);
    let predicate_method = render_lexer_predicate_method(&predicates);
    let next_token_call = match (actions.is_empty(), predicates.is_empty()) {
        (true, true) => "antlr4_runtime::atn::lexer::next_token(&mut self.base, atn())".to_owned(),
        (false, true) => {
            "antlr4_runtime::atn::lexer::next_token_with_actions(&mut self.base, atn(), Self::run_action)"
                .to_owned()
        }
        (true, false) => {
            "antlr4_runtime::atn::lexer::next_token_with_actions_and_predicates(&mut self.base, atn(), |_, _| {}, Self::run_predicate)"
                .to_owned()
        }
        (false, false) => {
            "antlr4_runtime::atn::lexer::next_token_with_actions_and_predicates(&mut self.base, atn(), Self::run_action, Self::run_predicate)"
                .to_owned()
        }
    };

    Ok(format!(
        r#"use antlr4_runtime::char_stream::CharStream;
use antlr4_runtime::recognizer::RecognizerData;
use antlr4_runtime::token::{{CommonToken, TokenSource}};
use antlr4_runtime::atn::Atn;
use antlr4_runtime::atn::serialized::AtnDeserializer;
use antlr4_runtime::{{BaseLexer, GeneratedLexer, GrammarMetadata, Lexer, Recognizer}};
use std::sync::OnceLock;

{token_constants}
{metadata}

static ATN_CELL: OnceLock<Atn> = OnceLock::new();

/// Deserializes and caches the grammar ATN for all lexer instances.
fn atn() -> &'static Atn {{
    ATN_CELL.get_or_init(|| {{
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

{action_method}
{predicate_method}
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
        {next_token_call}
    }}

    fn line(&self) -> usize {{ self.base.line() }}
    fn column(&self) -> usize {{ self.base.column() }}
    fn source_name(&self) -> &str {{ self.base.source_name() }}
}}
"#
    ))
}

/// Renders a Rust parser module with one public method per grammar rule.
///
/// Parser methods currently route through the runtime parser interpreter entry
/// point. As the parser ATN simulator matures, the generated surface can remain
/// stable while the interpreter becomes semantically complete.
fn render_parser(
    grammar_name: &str,
    data: &InterpData,
    grammar_source: Option<&str>,
) -> io::Result<String> {
    let type_name = rust_type_name(grammar_name);
    let metadata = render_metadata(grammar_name, data);
    let token_constants = render_token_constants(data);
    let rule_constants = render_rule_constants(data);
    let actions = grammar_source.map_or_else(
        || Ok(Vec::new()),
        |grammar| parser_action_templates(data, grammar),
    )?;
    let after_actions = grammar_source.map_or_else(
        || Ok(vec![None; data.rule_names.len()]),
        |grammar| parser_after_action_templates(data, grammar),
    )?;
    let init_actions = grammar_source.map_or_else(
        || Ok(vec![None; data.rule_names.len()]),
        |grammar| parser_init_action_templates(data, grammar),
    )?;
    let has_init_actions = init_actions.iter().any(Option::is_some);
    let has_action_dispatch = !actions.is_empty() || has_init_actions;
    let track_alt_numbers = grammar_source.is_some_and(uses_alt_number_contexts);
    let init_action_rules = init_actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| action.as_ref().map(|_| index))
        .collect::<Vec<_>>();
    let action_method = render_parser_action_method(&actions, &init_actions);
    let mut rule_methods = String::new();
    for (index, rule) in data.rule_names.iter().enumerate() {
        let after_action = after_actions.get(index).and_then(Option::as_ref);
        let uses_after_interval = after_action.is_some_and(ActionTemplate::uses_rule_interval);
        let needs_slow_path = has_action_dispatch
            || track_alt_numbers
            || after_action.is_some_and(ActionTemplate::needs_nested_tree);
        writeln!(
            rule_methods,
            "    pub fn {}(&mut self) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{",
            rust_function_name(rule)
        )
        .expect("writing to a string cannot fail");
        if uses_after_interval {
            writeln!(
                rule_methods,
                "        let start_index = antlr4_runtime::IntStream::index(self.base.input());"
            )
            .expect("writing to a string cannot fail");
        }
        if !needs_slow_path && after_action.is_none() {
            writeln!(
                rule_methods,
                "        self.base.parse_atn_rule(atn(), {index})"
            )
            .expect("writing to a string cannot fail");
        } else {
            if needs_slow_path {
                if track_alt_numbers {
                    writeln!(
                        rule_methods,
                        "        let (tree, actions) = self.base.parse_atn_rule_with_action_options(atn(), {index}, &{}, true)?;",
                        render_usize_array(&init_action_rules)
                    )
                    .expect("writing to a string cannot fail");
                } else if has_init_actions {
                    writeln!(
                        rule_methods,
                        "        let (tree, actions) = self.base.parse_atn_rule_with_action_inits(atn(), {index}, &{})?;",
                        render_usize_array(&init_action_rules)
                    )
                    .expect("writing to a string cannot fail");
                } else {
                    writeln!(
                        rule_methods,
                        "        let (tree, actions) = self.base.parse_atn_rule_with_actions(atn(), {index})?;"
                    )
                    .expect("writing to a string cannot fail");
                }
                if has_action_dispatch {
                    writeln!(
                        rule_methods,
                        "        for action in actions {{ self.run_action(action, &tree); }}"
                    )
                    .expect("writing to a string cannot fail");
                } else {
                    writeln!(rule_methods, "        let _ = actions;")
                        .expect("writing to a string cannot fail");
                }
            } else {
                writeln!(
                    rule_methods,
                    "        let tree = self.base.parse_atn_rule(atn(), {index})?;"
                )
                .expect("writing to a string cannot fail");
            }
            if let Some(template) = after_action {
                if uses_after_interval {
                    writeln!(
                        rule_methods,
                        "        let stop_index = antlr4_runtime::IntStream::index(self.base.input()).checked_sub(1);"
                    )
                    .expect("writing to a string cannot fail");
                }
                writeln!(
                    rule_methods,
                    "        {}",
                    render_parser_after_action_statement(template, index)
                )
                .expect("writing to a string cannot fail");
            }
            writeln!(rule_methods, "        Ok(tree)").expect("writing to a string cannot fail");
        }
        writeln!(rule_methods, "    }}").expect("writing to a string cannot fail");
    }

    Ok(format!(
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

static ATN_CELL: OnceLock<Atn> = OnceLock::new();

/// Deserializes and caches the grammar ATN for all parser instances.
fn atn() -> &'static Atn {{
    ATN_CELL.get_or_init(|| {{
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

{action_method}
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
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ActionTemplate {
    Noop,
    Text {
        newline: bool,
    },
    TextWithPrefix {
        prefix: String,
        newline: bool,
    },
    StringTree {
        target: StringTreeTarget,
        newline: bool,
    },
    RuleInvocationStack {
        newline: bool,
    },
    TokenText {
        source: TokenTextSource,
        newline: bool,
    },
    TokenDisplay {
        prefix: String,
        source: TokenDisplaySource,
        newline: bool,
    },
    ExpectedTokenNames {
        newline: bool,
    },
    Literal {
        value: String,
        newline: bool,
    },
}

impl ActionTemplate {
    /// Reports whether an `@after` action needs the rule's input interval
    /// captured before and after parsing.
    const fn uses_rule_interval(&self) -> bool {
        matches!(
            self,
            Self::Text { .. }
                | Self::TextWithPrefix { .. }
                | Self::TokenText { .. }
                | Self::TokenDisplay { .. }
        )
    }

    /// Reports whether rendering the action requires a nested parse tree
    /// instead of the faster flat rule tree.
    const fn needs_nested_tree(&self) -> bool {
        matches!(
            self,
            Self::StringTree { .. } | Self::RuleInvocationStack { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenTextSource {
    RuleStart,
    ActionStop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TokenDisplaySource {
    FirstErrorOrActionStop,
    RuleStop(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PredicateTemplate {
    True,
    False,
    TextEquals(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StringTreeTarget {
    Current,
    Label(String),
    Rule(usize),
}

/// Pairs supported lexer target-template actions with serialized custom-action
/// coordinates from the lexer ATN.
fn lexer_action_templates(
    data: &InterpData,
    grammar_source: &str,
) -> io::Result<Vec<((i32, i32), ActionTemplate)>> {
    let templates = extract_supported_action_templates(grammar_source)?;
    if templates.is_empty() {
        return Ok(Vec::new());
    }
    let actions = lexer_custom_actions(data)?;
    if actions.is_empty() {
        return Ok(Vec::new());
    }
    if actions.len() != templates.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "grammar has {} supported action template(s), but lexer ATN has {} custom action(s)",
                templates.len(),
                actions.len()
            ),
        ));
    }
    Ok(actions.into_iter().zip(templates).collect())
}

/// Pairs supported lexer semantic predicates with serialized predicate
/// coordinates from the lexer ATN.
fn lexer_predicate_templates(
    data: &InterpData,
    grammar_source: &str,
) -> io::Result<Vec<((usize, usize), PredicateTemplate)>> {
    let predicates = lexer_predicate_transitions(data)?;
    if predicates.is_empty() {
        return Ok(Vec::new());
    }
    let templates = extract_supported_predicate_templates(grammar_source)?;
    if templates.is_empty() {
        return Ok(Vec::new());
    }
    if predicates.len() != templates.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "grammar has {} supported predicate template(s), but lexer ATN has {} predicate transition(s)",
                templates.len(),
                predicates.len()
            ),
        ));
    }
    Ok(predicates.into_iter().zip(templates).collect())
}

/// Pairs supported target-template actions with parser ATN action source states.
fn parser_action_templates(
    data: &InterpData,
    grammar_source: &str,
) -> io::Result<Vec<(usize, ActionTemplate)>> {
    let templates = extract_supported_action_templates(grammar_source)?;
    if templates.is_empty() {
        return Ok(Vec::new());
    }
    let states = parser_action_states(data)?;
    if states.len() > templates.len() {
        let skip = states.len() - templates.len();
        return Ok(states.into_iter().skip(skip).zip(templates).collect());
    }
    if states.len() != templates.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "grammar has {} supported action template(s), but parser ATN has {} action transition(s)",
                templates.len(),
                states.len()
            ),
        ));
    }
    Ok(states.into_iter().zip(templates).collect())
}

/// Extracts rule-level `@after` target templates keyed by generated rule
/// index.
fn parser_after_action_templates(
    data: &InterpData,
    grammar_source: &str,
) -> io::Result<Vec<Option<ActionTemplate>>> {
    let mut actions = vec![None; data.rule_names.len()];
    let mut offset = 0;
    while let Some(block) = next_template_block(grammar_source, offset) {
        offset = block.after_brace;
        if block.predicate || !is_after_action(grammar_source, block.open_brace) {
            continue;
        }
        let Some(rule_name) = after_action_rule_name(grammar_source, block.open_brace) else {
            continue;
        };
        let Some(rule_index) = data.rule_names.iter().position(|name| name == rule_name) else {
            continue;
        };
        let Some(template) = parse_action_template(block.body) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported @after target action template <{}>", block.body),
            ));
        };
        actions[rule_index] = Some(resolve_after_action_template(
            template,
            grammar_source,
            block.open_brace,
            data,
        )?);
    }
    Ok(actions)
}

/// Extracts rule-level `@init` templates that must be replayed when a rule is
/// entered on the selected parser path.
fn parser_init_action_templates(
    data: &InterpData,
    grammar_source: &str,
) -> io::Result<Vec<Option<ActionTemplate>>> {
    let mut actions = vec![None; data.rule_names.len()];
    let mut offset = 0;
    while let Some(block) = next_template_block(grammar_source, offset) {
        offset = block.after_brace;
        if block.predicate || !is_init_action(grammar_source, block.open_brace) {
            continue;
        }
        let body = block.body.trim();
        if matches!(body, "BuildParseTrees()" | "BailErrorStrategy()") {
            continue;
        }
        let Some(rule_name) = init_action_rule_name(grammar_source, block.open_brace) else {
            continue;
        };
        let Some(rule_index) = data.rule_names.iter().position(|name| name == rule_name) else {
            continue;
        };
        let Some(template) = parse_action_template(body) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported @init target action template <{}>", block.body),
            ));
        };
        actions[rule_index] = Some(template);
    }
    Ok(actions)
}

/// Finds grammar action templates in the same order as ANTLR serializes action
/// transitions, while ignoring semantic predicates that are control-flow guards.
fn extract_supported_action_templates(grammar_source: &str) -> io::Result<Vec<ActionTemplate>> {
    let mut templates = Vec::new();
    let mut offset = 0;
    loop {
        let block = next_template_block(grammar_source, offset);
        let signature = next_signature_template(grammar_source, offset);
        match (block, signature) {
            (None, None) => break,
            (Some(block), Some(signature)) if signature.open_angle < block.open_brace => {
                offset = signature.after_template;
                let Some(template) = parse_action_template(signature.body) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported signature target template <{}>", signature.body),
                    ));
                };
                templates.push(template);
            }
            (Some(block), _) => {
                offset = block.after_brace;
                if block.predicate
                    || is_after_action(grammar_source, block.open_brace)
                    || is_init_action(grammar_source, block.open_brace)
                    || is_members_action(grammar_source, block.open_brace)
                {
                    continue;
                }
                let Some(template) = parse_action_template(block.body) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported target action template <{}>", block.body),
                    ));
                };
                templates.push(template);
            }
            (None, Some(signature)) => {
                offset = signature.after_template;
                let Some(template) = parse_action_template(signature.body) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported signature target template <{}>", signature.body),
                    ));
                };
                templates.push(template);
            }
        }
    }
    Ok(templates)
}

/// Finds grammar predicate templates in the same order as ANTLR serializes
/// predicate transitions.
fn extract_supported_predicate_templates(
    grammar_source: &str,
) -> io::Result<Vec<PredicateTemplate>> {
    let mut templates = Vec::new();
    let mut offset = 0;
    while let Some(block) = next_template_block(grammar_source, offset) {
        offset = block.after_brace;
        if !block.predicate {
            continue;
        }
        let Some(template) = parse_predicate_template(block.body) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported target predicate template <{}>", block.body),
            ));
        };
        templates.push(template);
    }
    Ok(templates)
}

/// Finds the next supported return-value target template that ANTLR lowers into
/// an action transition even though the metadata runtime treats it as a no-op.
fn next_signature_template(source: &str, offset: usize) -> Option<SignatureTemplate<'_>> {
    find_signature_template(source, offset, "returns [<")
}

/// Finds one signature template introduced by a specific rule-element marker.
fn find_signature_template<'a>(
    source: &'a str,
    offset: usize,
    marker: &str,
) -> Option<SignatureTemplate<'a>> {
    let marker_start = offset + source[offset..].find(marker)?;
    let open_angle = marker_start + marker.len() - 1;
    let body_start = open_angle + 1;
    let close_rel = source[body_start..].find(">]")?;
    let close_angle = body_start + close_rel;
    Some(SignatureTemplate {
        open_angle,
        body: &source[body_start..close_angle],
        after_template: close_angle + 2,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SignatureTemplate<'a> {
    open_angle: usize,
    body: &'a str,
    after_template: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TemplateBlock<'a> {
    open_brace: usize,
    body: &'a str,
    after_brace: usize,
    predicate: bool,
}

/// Finds the next target-template block while allowing whitespace inside the
/// ANTLR action braces, for example `{ <writeln("$text")> }`.
fn next_template_block(source: &str, offset: usize) -> Option<TemplateBlock<'_>> {
    let mut cursor = offset;
    while let Some(open_rel) = source[cursor..].find('{') {
        let open = cursor + open_rel;
        let template_start = skip_ascii_whitespace(source, open + 1);
        if source.as_bytes().get(template_start) != Some(&b'<') {
            cursor = open + 1;
            continue;
        }
        let close_angle = matching_template_close(source, template_start + 1)?;
        let close_brace = skip_ascii_whitespace(source, close_angle + 1);
        if source.as_bytes().get(close_brace) != Some(&b'}') {
            cursor = open + 1;
            continue;
        }
        let after_brace = close_brace + 1;
        return Some(TemplateBlock {
            open_brace: open,
            body: &source[template_start + 1..close_angle],
            after_brace,
            predicate: source[after_brace..].trim_start().starts_with('?'),
        });
    }
    None
}

/// Finds the matching `>` for a `StringTemplate` expression, allowing nested
/// template expressions inside arguments such as `<Assert({<Inner()>})>`.
fn matching_template_close(source: &str, mut index: usize) -> Option<usize> {
    let mut nested = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    while let Some(ch) = source[index..].chars().next() {
        if escaped {
            escaped = false;
            index += ch.len_utf8();
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => nested += 1,
            '>' if !quoted && nested == 0 => return Some(index),
            '>' if !quoted => nested = nested.saturating_sub(1),
            _ => {}
        }
        index += ch.len_utf8();
    }
    None
}

fn skip_ascii_whitespace(source: &str, mut index: usize) -> usize {
    while source
        .as_bytes()
        .get(index)
        .is_some_and(u8::is_ascii_whitespace)
    {
        index += 1;
    }
    index
}

fn is_after_action(source: &str, open_brace: usize) -> bool {
    is_rule_named_action(source, open_brace, "@after")
}

fn is_init_action(source: &str, open_brace: usize) -> bool {
    is_rule_named_action(source, open_brace, "@init")
}

fn is_rule_named_action(source: &str, open_brace: usize, marker: &str) -> bool {
    let prefix = &source[..open_brace];
    let statement_start = prefix.rfind(';').map_or(0, |index| index + 1);
    prefix[statement_start..].trim_end().ends_with(marker)
}

/// Detects member-action blocks whose target code is compile-time scaffolding
/// rather than an ATN semantic action.
fn is_members_action(source: &str, open_brace: usize) -> bool {
    let prefix = &source[..open_brace];
    let statement_start = prefix.rfind(';').map_or(0, |index| index + 1);
    matches!(
        prefix[statement_start..].trim(),
        "@members" | "@parser::members"
    )
}

fn uses_alt_number_contexts(source: &str) -> bool {
    source.contains("<TreeNodeWithAltNumField") || source.contains("contextSuperClass")
}

fn after_action_rule_name(source: &str, open_brace: usize) -> Option<&str> {
    named_action_rule_name(source, open_brace, "@after")
}

fn init_action_rule_name(source: &str, open_brace: usize) -> Option<&str> {
    named_action_rule_name(source, open_brace, "@init")
}

fn named_action_rule_name<'a>(source: &'a str, open_brace: usize, marker: &str) -> Option<&'a str> {
    let prefix = &source[..open_brace];
    let statement_start = prefix.rfind(';').map_or(0, |index| index + 1);
    let rule_preamble = prefix[statement_start..]
        .split(marker)
        .next()?
        .split('@')
        .next()?;
    rule_preamble
        .lines()
        .filter(|line| !line.trim_start().starts_with('<'))
        .flat_map(|line| line.split(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric())))
        .rfind(|name| !name.is_empty())
}

/// Resolves `$label.ctx` in a rule-level `@after` action to the referenced
/// rule index so generated code does not need to preserve source-level labels.
fn resolve_after_action_template(
    template: ActionTemplate,
    source: &str,
    open_brace: usize,
    data: &InterpData,
) -> io::Result<ActionTemplate> {
    let ActionTemplate::StringTree {
        target: StringTreeTarget::Label(label),
        newline,
    } = template
    else {
        return Ok(template);
    };
    let Some(rule_name) = labeled_rule_name(source, open_brace, &label) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("could not resolve label {label} for @after ToStringTree action"),
        ));
    };
    let Some(rule_index) = data.rule_names.iter().position(|name| name == rule_name) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("label {label} references unknown rule {rule_name}"),
        ));
    };
    Ok(ActionTemplate::StringTree {
        target: StringTreeTarget::Rule(rule_index),
        newline,
    })
}

/// Finds the rule name on the right side of `label=ruleName` inside the rule
/// that owns an `@after` action block.
fn labeled_rule_name<'a>(source: &'a str, open_brace: usize, label: &str) -> Option<&'a str> {
    let statement_start = source[..open_brace].rfind(';').map_or(0, |index| index + 1);
    let statement_end = source[open_brace..]
        .find(';')
        .map_or(source.len(), |index| open_brace + index);
    let rule = &source[statement_start..statement_end];
    let assignment = format!("{label}=");
    let after_label = rule.split(&assignment).nth(1)?;
    let mut chars = after_label.trim_start().chars();
    let mut end = 0;
    for ch in chars.by_ref() {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            end += ch.len_utf8();
        } else {
            break;
        }
    }
    let name = after_label.trim_start().get(..end)?;
    (!name.is_empty()).then_some(name)
}

/// Converts the subset of upstream `StringTemplate` actions the Rust generator
/// can replay today into concrete output actions.
fn parse_action_template(body: &str) -> Option<ActionTemplate> {
    let body = body.trim();
    match body {
        "Pass()" => Some(ActionTemplate::Noop),
        r#"writeln("$text")"# | "InputText():writeln()" | "Text():writeln()" => {
            Some(ActionTemplate::Text { newline: true })
        }
        r#"write("$text")"# | "Text():write()" => Some(ActionTemplate::Text { newline: false }),
        r#"ToStringTree("$ctx"):writeln()"# => Some(ActionTemplate::StringTree {
            target: StringTreeTarget::Current,
            newline: true,
        }),
        r#"ToStringTree("$ctx"):write()"# => Some(ActionTemplate::StringTree {
            target: StringTreeTarget::Current,
            newline: false,
        }),
        "GetExpectedTokenNames():writeln()" => {
            Some(ActionTemplate::ExpectedTokenNames { newline: true })
        }
        "GetExpectedTokenNames():write()" => {
            Some(ActionTemplate::ExpectedTokenNames { newline: false })
        }
        _ => parse_plus_text(body)
            .or_else(|| parse_string_tree(body))
            .or_else(|| parse_rule_invocation_stack(body))
            .or_else(|| parse_token_text(body))
            .or_else(|| parse_token_display(body))
            .or_else(|| parse_noop_action(body))
            .or_else(|| parse_write_literal(body)),
    }
}

fn parse_predicate_template(body: &str) -> Option<PredicateTemplate> {
    let body = body.trim();
    match body {
        "True()" => Some(PredicateTemplate::True),
        "False()" => Some(PredicateTemplate::False),
        _ => {
            let argument = body
                .strip_prefix("TextEquals(")
                .and_then(|value| value.strip_suffix(')'))?;
            Some(PredicateTemplate::TextEquals(parse_template_string(
                argument,
            )?))
        }
    }
}

/// Parses `ToStringTree("$label.ctx")` target templates into a label-bearing
/// tree action that can later be resolved against the owning rule.
fn parse_string_tree(body: &str) -> Option<ActionTemplate> {
    let (newline, argument) = if let Some(argument) = body
        .strip_prefix("ToStringTree(")
        .and_then(|value| value.strip_suffix("):writeln()"))
    {
        (true, argument)
    } else {
        let argument = body
            .strip_prefix("ToStringTree(")
            .and_then(|value| value.strip_suffix("):write()"))?;
        (false, argument)
    };
    let value = parse_template_string(argument)?;
    let label = value.strip_prefix('$')?.strip_suffix(".ctx")?;
    Some(ActionTemplate::StringTree {
        target: StringTreeTarget::Label(label.to_owned()),
        newline,
    })
}

/// Parses the runtime-testsuite helper that prints the active rule invocation
/// stack for a parser action site.
fn parse_rule_invocation_stack(body: &str) -> Option<ActionTemplate> {
    match body {
        "RuleInvocationStack():writeln()" => {
            Some(ActionTemplate::RuleInvocationStack { newline: true })
        }
        "RuleInvocationStack():write()" => {
            Some(ActionTemplate::RuleInvocationStack { newline: false })
        }
        _ => None,
    }
}

/// Recognizes target templates whose only purpose is compile-time API coverage
/// in the upstream descriptors.
fn parse_noop_action(body: &str) -> Option<ActionTemplate> {
    if (body.starts_with("AssignLocal(")
        || body.starts_with("AssertIsList(")
        || body.starts_with("IntArg("))
        && body.ends_with(')')
    {
        return Some(ActionTemplate::Noop);
    }
    None
}

fn parse_plus_text(body: &str) -> Option<ActionTemplate> {
    let (newline, argument) = if let Some(argument) = body
        .strip_prefix("PlusText(")
        .and_then(|value| value.strip_suffix("):writeln()"))
    {
        (true, argument)
    } else {
        let argument = body
            .strip_prefix("PlusText(")
            .and_then(|value| value.strip_suffix("):write()"))?;
        (false, argument)
    };
    let prefix = parse_template_string(argument)?;
    Some(ActionTemplate::TextWithPrefix { prefix, newline })
}

fn parse_token_text(body: &str) -> Option<ActionTemplate> {
    let (newline, argument) = if let Some(argument) = body
        .strip_prefix("writeln(")
        .and_then(|value| value.strip_suffix(')'))
    {
        (true, argument)
    } else {
        let argument = body
            .strip_prefix("write(")
            .and_then(|value| value.strip_suffix(')'))?;
        (false, argument)
    };
    let value = parse_template_string(argument)?;
    let label = value.strip_prefix('$')?.strip_suffix(".text")?;
    let source = label
        .chars()
        .next()
        .filter(char::is_ascii_uppercase)
        .map_or(TokenTextSource::RuleStart, |_| TokenTextSource::ActionStop);
    Some(ActionTemplate::TokenText { source, newline })
}

/// Parses token-display templates such as `Append("prefix","$x")` and
/// `writeln(Append("", "$rule.stop"))`.
fn parse_token_display(body: &str) -> Option<ActionTemplate> {
    let (newline, arguments) = append_arguments(body)?;
    let arguments = split_template_arguments(arguments);
    let [prefix_argument, value_argument] = arguments.as_slice() else {
        return None;
    };
    let prefix = parse_template_string(prefix_argument)?;
    let value = parse_template_string(value_argument)?;
    let source = if let Some(rule_name) = value.strip_prefix('$').and_then(|name| {
        name.strip_suffix(".stop")
            .filter(|name| is_antlr_identifier(name))
    }) {
        TokenDisplaySource::RuleStop(rule_name.to_owned())
    } else if value.strip_prefix('$').is_some_and(is_antlr_identifier) {
        TokenDisplaySource::FirstErrorOrActionStop
    } else {
        return None;
    };
    Some(ActionTemplate::TokenDisplay {
        prefix,
        source,
        newline,
    })
}

fn append_arguments(body: &str) -> Option<(bool, &str)> {
    if let Some(arguments) = body
        .strip_prefix("Append(")
        .and_then(|value| value.strip_suffix("):writeln()"))
    {
        return Some((true, arguments));
    }
    if let Some(arguments) = body
        .strip_prefix("Append(")
        .and_then(|value| value.strip_suffix("):write()"))
    {
        return Some((false, arguments));
    }
    if let Some(arguments) = body
        .strip_prefix("writeln(Append(")
        .and_then(|value| value.strip_suffix("))"))
    {
        return Some((true, arguments));
    }
    body.strip_prefix("write(Append(")
        .and_then(|value| value.strip_suffix("))"))
        .map(|arguments| (false, arguments))
}

/// Splits a `StringTemplate` argument list while ignoring commas inside quoted
/// strings or nested template/function calls.
fn split_template_arguments(arguments: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    let mut paren_depth = 0_usize;
    let mut angle_depth = 0_usize;
    let mut brace_depth = 0_usize;
    for (index, ch) in arguments.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '(' if !quoted => paren_depth += 1,
            ')' if !quoted => paren_depth = paren_depth.saturating_sub(1),
            '<' if !quoted => angle_depth += 1,
            '>' if !quoted => angle_depth = angle_depth.saturating_sub(1),
            '{' if !quoted => brace_depth += 1,
            '}' if !quoted => brace_depth = brace_depth.saturating_sub(1),
            ',' if !quoted && paren_depth == 0 && angle_depth == 0 && brace_depth == 0 => {
                parts.push(arguments[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(arguments[start..].trim());
    parts
}

fn is_antlr_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn parse_write_literal(body: &str) -> Option<ActionTemplate> {
    let (newline, argument) = if let Some(argument) = body
        .strip_prefix("writeln(")
        .and_then(|value| value.strip_suffix(')'))
    {
        (true, argument)
    } else {
        let argument = body
            .strip_prefix("write(")
            .and_then(|value| value.strip_suffix(')'))?;
        (false, argument)
    };
    let value = parse_template_string(argument)?;
    Some(ActionTemplate::Literal { value, newline })
}

/// Decodes the descriptor's quoted `StringTemplate` argument into the Rust
/// string literal payload that generated parser code should print.
fn parse_template_string(argument: &str) -> Option<String> {
    let mut value = argument.trim();
    value = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(ch);
        }
    }
    if out.starts_with('"') && out.ends_with('"') && out.len() >= 2 {
        out = out[1..out.len() - 1].to_owned();
    }
    Some(out)
}

/// Reads the lexer ATN to locate serialized custom action coordinates.
fn lexer_custom_actions(data: &InterpData) -> io::Result<Vec<(i32, i32)>> {
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(data.atn.clone()))
        .deserialize()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(atn
        .lexer_actions()
        .iter()
        .filter_map(|action| match action {
            LexerAction::Custom {
                rule_index,
                action_index,
            } => Some((*rule_index, *action_index)),
            _ => None,
        })
        .collect())
}

/// Reads the lexer ATN to locate semantic predicate coordinates.
fn lexer_predicate_transitions(data: &InterpData) -> io::Result<Vec<(usize, usize)>> {
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(data.atn.clone()))
        .deserialize()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut predicates = Vec::new();
    for state in atn.states() {
        for transition in &state.transitions {
            if let Transition::Predicate {
                rule_index,
                pred_index,
                ..
            } = transition
            {
                predicates.push((*rule_index, *pred_index));
            }
        }
    }
    Ok(predicates)
}

/// Reads the parser ATN to locate action-transition source states.
fn parser_action_states(data: &InterpData) -> io::Result<Vec<usize>> {
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(data.atn.clone()))
        .deserialize()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut states = Vec::new();
    for state in atn.states() {
        if state
            .transitions
            .iter()
            .any(|transition| matches!(transition, Transition::Action { .. }))
        {
            states.push(state.state_number);
        }
    }
    Ok(states)
}

/// Emits the generated lexer action dispatcher for grammar-specific custom
/// lexer actions discovered from the serialized ATN.
fn render_lexer_action_method(actions: &[((i32, i32), ActionTemplate)]) -> String {
    if actions.is_empty() {
        return String::new();
    }
    let mut arms = String::new();
    for ((rule_index, action_index), template) in actions {
        let statement = render_lexer_action_statement(template);
        writeln!(
            arms,
            "            ({rule_index}, {action_index}) => {{ {statement} }}"
        )
        .expect("writing to a string cannot fail");
    }
    arms.push_str("            _ => {}\n");
    format!(
        "    fn run_action(_base: &mut BaseLexer<I>, action: antlr4_runtime::LexerCustomAction) {{\n        match (action.rule_index(), action.action_index()) {{\n{arms}        }}\n    }}\n"
    )
}

/// Renders one supported lexer target-template action as Rust code.
fn render_lexer_action_statement(template: &ActionTemplate) -> String {
    match template {
        ActionTemplate::Noop => String::new(),
        ActionTemplate::Text { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!(
                "let text = _base.token_text_until(action.position()); {write}(\"{{}}\", text);"
            )
        }
        ActionTemplate::TextWithPrefix { prefix, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!(
                "let text = _base.token_text_until(action.position()); {write}(\"{}{{}}\", text);",
                rust_string(prefix)
            )
        }
        ActionTemplate::TokenText { newline, .. } => {
            let write = if *newline { "println!" } else { "print!" };
            format!(
                "let text = _base.token_text_until(action.position()); {write}(\"{{}}\", text);"
            )
        }
        ActionTemplate::TokenDisplay { .. } => String::new(),
        ActionTemplate::ExpectedTokenNames { .. } => String::new(),
        ActionTemplate::StringTree { .. } => String::new(),
        ActionTemplate::RuleInvocationStack { .. } => String::new(),
        ActionTemplate::Literal { value, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!("{write}(\"{}\");", rust_string(value))
        }
    }
}

/// Emits the generated lexer predicate dispatcher for grammar-specific
/// predicate coordinates discovered from the serialized ATN.
fn render_lexer_predicate_method(predicates: &[((usize, usize), PredicateTemplate)]) -> String {
    if predicates.is_empty() {
        return String::new();
    }
    let mut arms = String::new();
    for ((rule_index, pred_index), template) in predicates {
        let statement = render_lexer_predicate_expression(template);
        writeln!(
            arms,
            "            ({rule_index}, {pred_index}) => {{ {statement} }}"
        )
        .expect("writing to a string cannot fail");
    }
    arms.push_str("            _ => true,\n");
    format!(
        "    fn run_predicate(_base: &BaseLexer<I>, predicate: antlr4_runtime::LexerPredicate) -> bool {{\n        match (predicate.rule_index(), predicate.pred_index()) {{\n{arms}        }}\n    }}\n"
    )
}

fn render_lexer_predicate_expression(template: &PredicateTemplate) -> String {
    match template {
        PredicateTemplate::True => "true".to_owned(),
        PredicateTemplate::False => "false".to_owned(),
        PredicateTemplate::TextEquals(value) => format!(
            "_base.token_text_until(predicate.position()) == \"{}\"",
            rust_string(value)
        ),
    }
}

/// Emits the generated parser action dispatcher for the grammar-specific action
/// source states discovered from the serialized ATN.
fn render_parser_action_method(
    actions: &[(usize, ActionTemplate)],
    init_actions: &[Option<ActionTemplate>],
) -> String {
    let has_init_actions = init_actions.iter().any(Option::is_some);
    if actions.is_empty() && !has_init_actions {
        return String::new();
    }
    let mut init_arms = String::new();
    for (rule_index, template) in init_actions.iter().enumerate() {
        let Some(template) = template else {
            continue;
        };
        let statement = render_action_statement(template);
        writeln!(
            init_arms,
            "                {rule_index} => {{ {statement} }}"
        )
        .expect("writing to a string cannot fail");
    }
    if has_init_actions {
        init_arms.push_str("                _ => {}\n");
    }
    let mut arms = String::new();
    for (state, template) in actions {
        let statement = render_action_statement(template);
        writeln!(arms, "            {state} => {{ {statement} }}")
            .expect("writing to a string cannot fail");
    }
    arms.push_str("            _ => {}\n");
    let init_dispatch = if has_init_actions {
        format!(
            "        if action.is_rule_init() {{\n            match action.rule_index() {{\n{init_arms}            }}\n            return;\n        }}\n"
        )
    } else {
        String::new()
    };
    format!(
        "    fn run_action(&mut self, action: antlr4_runtime::ParserAction, _tree: &antlr4_runtime::ParseTree) {{\n{init_dispatch}        match action.source_state() {{\n{arms}        }}\n    }}\n"
    )
}

/// Renders one supported target-template action as Rust code.
fn render_action_statement(template: &ActionTemplate) -> String {
    match template {
        ActionTemplate::Noop => String::new(),
        ActionTemplate::Text { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!(
                "let text = self.base.text_interval(action.start_index(), action.stop_index()); {write}(\"{{}}\", text);"
            )
        }
        ActionTemplate::TextWithPrefix { prefix, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!(
                "let text = self.base.text_interval(action.start_index(), action.stop_index()); {write}(\"{}{{}}\", text);",
                rust_string(prefix)
            )
        }
        ActionTemplate::TokenText { source, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            match source {
                TokenTextSource::RuleStart => format!(
                    "let text = self.base.text_interval(action.start_index(), Some(action.start_index())); {write}(\"{{}}\", text);"
                ),
                TokenTextSource::ActionStop => format!(
                    "let text = action.stop_index().map_or_else(String::new, |index| self.base.text_interval(index, Some(index))); {write}(\"{{}}\", text);"
                ),
            }
        }
        ActionTemplate::TokenDisplay {
            prefix,
            source,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            render_token_display_write(write, "_tree", "action", prefix, source)
        }
        ActionTemplate::ExpectedTokenNames { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!(
                "let text = action.expected_state().map_or_else(String::new, |state| self.base.expected_tokens_at_state(atn(), state)); {write}(\"{{}}\", text);"
            )
        }
        ActionTemplate::StringTree { target, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            render_string_tree_write(write, "_tree", target)
        }
        ActionTemplate::RuleInvocationStack { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            render_rule_invocation_stack_write(write, "_tree", "action.rule_index()")
        }
        ActionTemplate::Literal { value, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!("{write}(\"{}\");", rust_string(value))
        }
    }
}

/// Renders a rule-level `@after` action using the parsed rule input span.
fn render_parser_after_action_statement(template: &ActionTemplate, rule_index: usize) -> String {
    match template {
        ActionTemplate::Noop => String::new(),
        ActionTemplate::Text { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!(
                "let text = self.base.text_interval(start_index, stop_index); {write}(\"{{}}\", text);"
            )
        }
        ActionTemplate::TextWithPrefix { prefix, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!(
                "let text = self.base.text_interval(start_index, stop_index); {write}(\"{}{{}}\", text);",
                rust_string(prefix)
            )
        }
        ActionTemplate::TokenText { source, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            match source {
                TokenTextSource::RuleStart => format!(
                    "let text = self.base.text_interval(start_index, Some(start_index)); {write}(\"{{}}\", text);"
                ),
                TokenTextSource::ActionStop => format!(
                    "let text = stop_index.map_or_else(String::new, |index| self.base.text_interval(index, Some(index))); {write}(\"{{}}\", text);"
                ),
            }
        }
        ActionTemplate::TokenDisplay {
            prefix,
            source,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            render_after_token_display_write(write, "tree", prefix, source)
        }
        ActionTemplate::ExpectedTokenNames { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!("{write}(\"\");")
        }
        ActionTemplate::StringTree { target, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            render_string_tree_write(write, "tree", target)
        }
        ActionTemplate::RuleInvocationStack { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            let rule_index = rule_index.to_string();
            render_rule_invocation_stack_write(write, "tree", &rule_index)
        }
        ActionTemplate::Literal { value, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!("{write}(\"{}\");", rust_string(value))
        }
    }
}

/// Emits the generated print statement for the first rule invocation stack
/// matching `rule_index_expr`.
fn render_rule_invocation_stack_write(
    write: &str,
    tree_expr: &str,
    rule_index_expr: &str,
) -> String {
    let rule_names =
        "METADATA.rule_names().iter().map(|name| (*name).to_owned()).collect::<Vec<_>>()";
    format!(
        "let stack = {tree_expr}.rule_invocation_stack({rule_index_expr}, &{rule_names}).unwrap_or_default().join(\", \"); {write}(\"[{{}}]\", stack);"
    )
}

/// Emits the generated print statement for token-display target templates.
fn render_token_display_write(
    write: &str,
    tree_expr: &str,
    action_expr: &str,
    prefix: &str,
    source: &TokenDisplaySource,
) -> String {
    let prefix = rust_string(prefix);
    match source {
        TokenDisplaySource::FirstErrorOrActionStop => format!(
            "let text = {tree_expr}.first_error_token().map_or_else(|| {action_expr}.stop_index().and_then(|index| self.base.token_display_at(index)).unwrap_or_default(), |token| format!(\"{{token}}\")); {write}(\"{prefix}{{}}\", text);"
        ),
        TokenDisplaySource::RuleStop(rule_name) => {
            let rule_name = rust_string(rule_name);
            format!(
                "let text = METADATA.rule_names().iter().position(|name| *name == \"{rule_name}\").and_then(|rule_index| {tree_expr}.first_rule_stop(rule_index)).map_or_else(String::new, |token| format!(\"{{token}}\")); {write}(\"{prefix}{{}}\", text);"
            )
        }
    }
}

/// Emits token-display target templates from rule-level actions where no
/// parser action event is available.
fn render_after_token_display_write(
    write: &str,
    tree_expr: &str,
    prefix: &str,
    source: &TokenDisplaySource,
) -> String {
    let prefix = rust_string(prefix);
    match source {
        TokenDisplaySource::FirstErrorOrActionStop => format!(
            "let text = stop_index.and_then(|index| self.base.token_display_at(index)).unwrap_or_default(); {write}(\"{prefix}{{}}\", text);"
        ),
        TokenDisplaySource::RuleStop(rule_name) => {
            let rule_name = rust_string(rule_name);
            format!(
                "let text = METADATA.rule_names().iter().position(|name| *name == \"{rule_name}\").and_then(|rule_index| {tree_expr}.first_rule_stop(rule_index)).map_or_else(String::new, |token| format!(\"{{token}}\")); {write}(\"{prefix}{{}}\", text);"
            )
        }
    }
}

/// Emits the generated print statement for either the current parse tree or a
/// selected child rule tree found inside it.
fn render_string_tree_write(write: &str, tree_expr: &str, target: &StringTreeTarget) -> String {
    let rule_names =
        "METADATA.rule_names().iter().map(|name| (*name).to_owned()).collect::<Vec<_>>()";
    match target {
        StringTreeTarget::Current => {
            format!("{write}(\"{{}}\", {tree_expr}.to_string_tree(&{rule_names}));")
        }
        StringTreeTarget::Rule(rule_index) => format!(
            "let text = {tree_expr}.first_rule({rule_index}).map_or_else(String::new, |node| node.to_string_tree(&{rule_names})); {write}(\"{{}}\", text);"
        ),
        StringTreeTarget::Label(_) => String::new(),
    }
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

/// Renders an inline `[usize; N]` expression for generated parser helpers.
fn render_usize_array(values: &[usize]) -> String {
    let items = values
        .iter()
        .map(usize::to_string)
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

    #[test]
    fn parses_nested_template_action_block() {
        let block = next_template_block(
            r#"s @after {<AssertIsList({<ContextListFunction("$ctx","x")>})>} : 'x' ;"#,
            0,
        )
        .expect("nested template block should parse");

        assert_eq!(
            block.body,
            r#"AssertIsList({<ContextListFunction("$ctx","x")>})"#
        );
    }

    #[test]
    fn extracts_return_noop_between_parser_actions() {
        let templates = extract_supported_action_templates(
            r#"root : {<write("$text")>} continue ;
continue returns [<IntArg("return")>] : {<AssignLocal("$return","0")>} ;"#,
        )
        .expect("supported templates should extract");

        assert_eq!(templates.len(), 3);
        assert!(matches!(templates[0], ActionTemplate::Text { .. }));
        assert!(matches!(templates[1], ActionTemplate::Noop));
        assert!(matches!(templates[2], ActionTemplate::Noop));
    }
}
