use std::collections::{BTreeMap, BTreeSet};
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
    let adjusts_accept_position = grammar_source.is_some_and(uses_position_adjusting_lexer);
    let action_method = render_lexer_action_method(&actions);
    let predicate_method = render_lexer_predicate_method(&predicates);
    let accept_adjust_method = if adjusts_accept_position {
        render_position_adjusting_lexer_methods()
    } else {
        String::new()
    };
    let next_token_call = match (
        actions.is_empty(),
        predicates.is_empty(),
        adjusts_accept_position,
    ) {
        (true, true, false) => {
            "antlr4_runtime::atn::lexer::next_token(&mut self.base, atn())".to_owned()
        }
        (false, true, false) => {
            "antlr4_runtime::atn::lexer::next_token_with_actions(&mut self.base, atn(), Self::run_action)"
                .to_owned()
        }
        (true, false, false) => {
            "antlr4_runtime::atn::lexer::next_token_with_actions_and_predicates(&mut self.base, atn(), |_, _| {}, Self::run_predicate)"
                .to_owned()
        }
        (false, false, false) => {
            "antlr4_runtime::atn::lexer::next_token_with_actions_and_predicates(&mut self.base, atn(), Self::run_action, Self::run_predicate)"
                .to_owned()
        }
        (true, true, true) => {
            "antlr4_runtime::atn::lexer::next_token_with_accept_adjuster(&mut self.base, atn(), Self::adjust_accept_position)"
                .to_owned()
        }
        (false, true, true) => {
            "antlr4_runtime::atn::lexer::next_token_with_hooks(&mut self.base, atn(), Self::run_action, |_, _| true, Self::adjust_accept_position)"
                .to_owned()
        }
        (true, false, true) => {
            "antlr4_runtime::atn::lexer::next_token_with_hooks(&mut self.base, atn(), |_, _| {}, Self::run_predicate, Self::adjust_accept_position)"
                .to_owned()
        }
        (false, false, true) => {
            "antlr4_runtime::atn::lexer::next_token_with_hooks(&mut self.base, atn(), Self::run_action, Self::run_predicate, Self::adjust_accept_position)"
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
{accept_adjust_method}
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
    fn drain_errors(&mut self) -> Vec<antlr4_runtime::token::TokenSourceError> {{
        self.base.drain_errors()
    }}
    fn lexer_dfa_string(&self) -> String {{
        self.base.lexer_dfa_string()
    }}
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
        || Ok(vec![Vec::new(); data.rule_names.len()]),
        |grammar| parser_after_action_templates(data, grammar),
    )?;
    let init_actions = grammar_source.map_or_else(
        || Ok(vec![None; data.rule_names.len()]),
        |grammar| parser_init_action_templates(data, grammar),
    )?;
    let predicates = grammar_source.map_or_else(
        || Ok(Vec::new()),
        |grammar| parser_predicate_templates(data, grammar),
    )?;
    let rule_args =
        grammar_source.map_or_else(|| Ok(Vec::new()), |grammar| parser_rule_args(data, grammar))?;
    let int_members = grammar_source.map_or_else(Vec::new, parser_int_members);
    let member_actions = parser_member_actions(&actions, &int_members)?;
    let return_actions = parser_return_actions(&actions);
    let has_init_actions = init_actions.iter().any(Option::is_some);
    let has_action_dispatch = !actions.is_empty() || has_init_actions;
    let has_predicate_dispatch = !predicates.is_empty();
    let has_return_actions = !return_actions.is_empty();
    let track_alt_numbers = grammar_source.is_some_and(uses_alt_number_contexts);
    let init_action_rules = init_actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| action.as_ref().map(|_| index))
        .collect::<Vec<_>>();
    let action_method = render_parser_action_method(&actions, &init_actions, &int_members)?;
    let base_initialization = render_parser_base_initialization(&int_members);
    let mut rule_methods = String::new();
    for (index, rule) in data.rule_names.iter().enumerate() {
        let after_action = after_actions.get(index).map_or(&[][..], Vec::as_slice);
        let uses_after_interval = after_action.iter().any(ActionTemplate::uses_rule_interval);
        let needs_slow_path = has_action_dispatch
            || track_alt_numbers
            || has_predicate_dispatch
            || has_return_actions
            || after_action.iter().any(ActionTemplate::needs_nested_tree);
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
        if !needs_slow_path && after_action.is_empty() {
            writeln!(
                rule_methods,
                "        self.base.parse_atn_rule(atn(), {index})"
            )
            .expect("writing to a string cannot fail");
        } else {
            if needs_slow_path {
                if has_predicate_dispatch || has_return_actions {
                    writeln!(
                        rule_methods,
                        "        let (tree, actions) = self.base.parse_atn_rule_with_runtime_options(atn(), {index}, antlr4_runtime::ParserRuntimeOptions {{ init_action_rules: &{}, track_alt_numbers: {track_alt_numbers}, predicates: &{}, rule_args: &{}, member_actions: &{}, return_actions: &{} }})?;",
                        render_usize_array(&init_action_rules),
                        render_parser_predicate_array(&predicates, data, &int_members)?,
                        render_parser_rule_arg_array(&rule_args),
                        render_parser_member_action_array(&member_actions),
                        render_parser_return_action_array(&return_actions, data)?
                    )
                    .expect("writing to a string cannot fail");
                } else if track_alt_numbers {
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
            if !after_action.is_empty() {
                if uses_after_interval {
                    writeln!(
                        rule_methods,
                        "        let stop_index = antlr4_runtime::IntStream::index(self.base.input()).checked_sub(1);"
                    )
                    .expect("writing to a string cannot fail");
                }
                for template in after_action {
                    writeln!(
                        rule_methods,
                        "        {}",
                        render_parser_after_action_statement(template, index)
                    )
                    .expect("writing to a string cannot fail");
                }
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
{base_initialization}
        Self {{ base }}
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
    fn report_diagnostic_errors(&self) -> bool {{ self.base.report_diagnostic_errors() }}
    fn set_report_diagnostic_errors(&mut self, report: bool) {{ self.base.set_report_diagnostic_errors(report); }}
    fn prediction_mode(&self) -> antlr4_runtime::PredictionMode {{ self.base.prediction_mode() }}
    fn set_prediction_mode(&mut self, mode: antlr4_runtime::PredictionMode) {{ self.base.set_prediction_mode(mode); }}
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
    ListenerWalk {
        target: StringTreeTarget,
        kind: ListenerKind,
    },
    RuleValue {
        rule_name: String,
        kind: RuleValueKind,
        newline: bool,
    },
    RuleReturnValue {
        rule_name: String,
        value_name: String,
        newline: bool,
    },
    SetIntReturn {
        name: String,
        value: i64,
    },
    TokenText {
        source: TokenTextSource,
        newline: bool,
    },
    TokenTextWithPrefix {
        prefix: String,
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
    AddMember {
        member: String,
        value: i64,
    },
    MemberValue {
        member: String,
        newline: bool,
    },
    Sequence(Vec<Self>),
}

impl ActionTemplate {
    /// Reports whether an `@after` action needs the rule's input interval
    /// captured before and after parsing.
    fn uses_rule_interval(&self) -> bool {
        matches!(
            self,
            Self::Text { .. }
                | Self::TextWithPrefix { .. }
                | Self::TokenText { .. }
                | Self::TokenTextWithPrefix { .. }
                | Self::TokenDisplay { .. }
        ) || matches!(self, Self::Sequence(actions) if actions.iter().any(Self::uses_rule_interval))
    }

    /// Reports whether rendering the action requires a nested parse tree
    /// instead of the faster flat rule tree.
    fn needs_nested_tree(&self) -> bool {
        matches!(
            self,
            Self::StringTree { .. }
                | Self::RuleInvocationStack { .. }
                | Self::ListenerWalk { .. }
                | Self::RuleValue { .. }
                | Self::RuleReturnValue { .. }
        ) || matches!(self, Self::Sequence(actions) if actions.iter().any(Self::needs_nested_tree))
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
    FalseWithMessage {
        message: String,
    },
    Invoke {
        value: bool,
    },
    LocalIntEquals {
        value: i64,
    },
    MemberModuloEquals {
        member: String,
        modulus: i64,
        value: i64,
        equals: bool,
    },
    LookaheadTextEquals {
        offset: isize,
        text: String,
    },
    TextEquals(String),
    TokenStartColumnEquals(usize),
    ColumnLessThan(usize),
    ColumnGreaterOrEqual(usize),
    LookaheadNotEquals {
        offset: isize,
        token_name: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StringTreeTarget {
    Current,
    Label(String),
    Rule(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ListenerKind {
    Basic,
    TokenGetter,
    RuleGetter,
    LeftRecursive,
    LeftRecursiveWithLabels,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuleValueKind {
    Int,
    String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuleArgTemplate {
    Literal(i64),
    InheritLocal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IntMemberTemplate {
    name: String,
    initial_value: i64,
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
    if actions.len() == templates.len() {
        return Ok(actions.into_iter().zip(templates).collect());
    }

    let filtered_templates =
        extract_supported_rule_action_templates(grammar_source, &data.rule_names)?;
    if actions.len() != filtered_templates.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "grammar has {} supported action template(s), but lexer ATN has {} custom action(s)",
                filtered_templates.len(),
                actions.len()
            ),
        ));
    }
    Ok(actions.into_iter().zip(filtered_templates).collect())
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

/// Pairs supported parser semantic predicates with serialized predicate
/// coordinates from the parser ATN.
fn parser_predicate_templates(
    data: &InterpData,
    grammar_source: &str,
) -> io::Result<Vec<((usize, usize), PredicateTemplate)>> {
    let predicates = lexer_predicate_transitions(data)?;
    let mut mapped = Vec::new();
    let mut offset = 0;
    let mut predicate_index = 0;
    while let Some(block) = next_predicate_action_block(grammar_source, offset) {
        offset = block.after_brace;
        if let Some(template) = parse_predicate_template(block.body) {
            let template = match predicate_fail_message(grammar_source, block.after_brace) {
                Some(message) => predicate_template_with_fail_message(template, message),
                None => template,
            };
            let Some(coordinates) = predicates.get(predicate_index).copied() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "grammar predicate template <{}> has no parser ATN predicate transition",
                        block.body
                    ),
                ));
            };
            mapped.push((coordinates, template));
        }
        predicate_index += 1;
    }
    Ok(mapped)
}

/// Attaches ANTLR's fail option to predicates whose false result is modeled by
/// the metadata runtime.
fn predicate_template_with_fail_message(
    template: PredicateTemplate,
    message: String,
) -> PredicateTemplate {
    match template {
        PredicateTemplate::False => PredicateTemplate::FalseWithMessage { message },
        _ => template,
    }
}

/// Pairs supported target-template actions with parser ATN action source states.
fn parser_action_templates(
    data: &InterpData,
    grammar_source: &str,
) -> io::Result<Vec<(usize, ActionTemplate)>> {
    let templates = extract_supported_action_templates(grammar_source)?;
    match parser_action_templates_from_templates(data, templates) {
        Ok(actions) => Ok(actions),
        Err(unfiltered_error) => {
            let templates =
                extract_supported_rule_action_templates(grammar_source, &data.rule_names)?;
            parser_action_templates_from_templates(data, templates).map_err(|_| unfiltered_error)
        }
    }
}

fn parser_action_templates_from_templates(
    data: &InterpData,
    templates: Vec<ActionTemplate>,
) -> io::Result<Vec<(usize, ActionTemplate)>> {
    if templates.is_empty() {
        return Ok(Vec::new());
    }
    let states = parser_action_states(data)?;
    if states.len() > templates.len() {
        // Return-value print helpers appear before raw return-assignment
        // actions in these descriptors, so source-order pairing selects the
        // user-visible print action instead of a later raw assignment action.
        if templates
            .iter()
            .any(|template| matches!(template, ActionTemplate::RuleValue { .. }))
        {
            return Ok(states.into_iter().zip(templates).collect());
        }
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
) -> io::Result<Vec<Vec<ActionTemplate>>> {
    let mut actions = vec![Vec::new(); data.rule_names.len()];
    let listener_kind = listener_template_kind(grammar_source);
    for block in named_action_templates(grammar_source, "@after") {
        let Some(rule_name) = after_action_rule_name(grammar_source, block.open_brace) else {
            continue;
        };
        let Some(rule_index) = data.rule_names.iter().position(|name| name == rule_name) else {
            continue;
        };
        let Some(template) = parse_after_action_template(block.body, listener_kind) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported @after target action template <{}>", block.body),
            ));
        };
        actions[rule_index].push(resolve_after_action_template(
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
        if matches!(
            body,
            "BuildParseTrees()" | "BailErrorStrategy()" | "LL_EXACT_AMBIG_DETECTION()"
        ) {
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
    extract_supported_action_templates_filtered(grammar_source, None)
}

/// Extracts only action templates owned by rules present in the active `.interp`
/// metadata, which keeps combined grammars from feeding parser actions to lexer
/// generation and vice versa.
fn extract_supported_rule_action_templates(
    grammar_source: &str,
    rule_names: &[String],
) -> io::Result<Vec<ActionTemplate>> {
    extract_supported_action_templates_filtered(grammar_source, Some(rule_names))
}

fn extract_supported_action_templates_filtered(
    grammar_source: &str,
    rule_names: Option<&[String]>,
) -> io::Result<Vec<ActionTemplate>> {
    let mut templates = Vec::new();
    let mut offset = 0;
    loop {
        let block = next_parser_action_block(grammar_source, offset);
        let signature = next_signature_template(grammar_source, offset);
        match (block, signature) {
            (None, None) => break,
            (Some(block), Some(signature)) if signature.open_angle < block.open_brace => {
                offset = signature.after_template;
                if !rule_action_included(grammar_source, signature.open_angle, rule_names) {
                    continue;
                }
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
                if !rule_action_included(grammar_source, block.open_brace, rule_names) {
                    continue;
                }
                if block.predicate
                    || is_after_action(grammar_source, block.open_brace)
                    || is_init_action(grammar_source, block.open_brace)
                    || is_definitions_action(grammar_source, block.open_brace)
                    || is_members_action(grammar_source, block.open_brace)
                    || is_options_block(grammar_source, block.open_brace)
                {
                    continue;
                }
                let Some(template) = parse_action_block_template(block.body) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported target action template <{}>", block.body),
                    ));
                };
                templates.push(resolve_action_template_labels(
                    template,
                    grammar_source,
                    block.open_brace,
                ));
            }
            (None, Some(signature)) => {
                offset = signature.after_template;
                if !rule_action_included(grammar_source, signature.open_angle, rule_names) {
                    continue;
                }
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

/// Applies an optional rule-name filter to an action or signature position.
fn rule_action_included(source: &str, position: usize, rule_names: Option<&[String]>) -> bool {
    let Some(header) = statement_rule_header(source, position) else {
        return rule_names.is_none();
    };
    rule_names.is_none_or(|names| names.iter().any(|name| name == header.name))
        && !has_prior_rule_definition(source, header.name, header.start)
}

/// Finds grammar predicate templates in the same order as ANTLR serializes
/// predicate transitions.
fn extract_supported_predicate_templates(
    grammar_source: &str,
) -> io::Result<Vec<PredicateTemplate>> {
    let mut templates = Vec::new();
    let mut offset = 0;
    while let Some(block) = next_predicate_action_block(grammar_source, offset) {
        offset = block.after_brace;
        if let Some(template) = parse_predicate_template(block.body) {
            templates.push(template);
        } else if block.body.contains('<') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported target predicate template <{}>", block.body),
            ));
        }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NamedActionTemplate<'a> {
    open_brace: usize,
    body: &'a str,
}

/// Finds all target templates inside a rule-level named action body, including
/// multi-template blocks such as the listener-suite `@after` actions.
fn named_action_templates<'a>(source: &'a str, marker: &str) -> Vec<NamedActionTemplate<'a>> {
    let mut templates = Vec::new();
    let mut offset = 0;
    while let Some(marker_start) = source[offset..].find(marker).map(|index| offset + index) {
        let Some(open_brace) = source[marker_start..]
            .find('{')
            .map(|index| marker_start + index)
        else {
            break;
        };
        let Some(close_brace) = matching_action_brace(source, open_brace + 1) else {
            break;
        };
        let mut cursor = open_brace + 1;
        while cursor < close_brace {
            let Some(open_angle) = source[cursor..close_brace]
                .find('<')
                .map(|index| cursor + index)
            else {
                break;
            };
            let Some(close_angle) = matching_template_close(source, open_angle + 1) else {
                break;
            };
            if close_angle > close_brace {
                break;
            }
            templates.push(NamedActionTemplate {
                open_brace,
                body: &source[open_angle + 1..close_angle],
            });
            cursor = close_angle + 1;
        }
        offset = close_brace + 1;
    }
    templates
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

/// Finds the next semantic-predicate action block, including expressions that
/// combine target-template calls with target-language comparison operators.
fn next_predicate_action_block(source: &str, offset: usize) -> Option<TemplateBlock<'_>> {
    let mut cursor = offset;
    while let Some(open_rel) = source[cursor..].find('{') {
        let open_brace = cursor + open_rel;
        let close_brace = matching_action_brace(source, open_brace + 1)?;
        let after_brace = close_brace + 1;
        if source[after_brace..].trim_start().starts_with('?') {
            return Some(TemplateBlock {
                open_brace,
                body: &source[open_brace + 1..close_brace],
                after_brace,
                predicate: true,
            });
        }
        cursor = open_brace + 1;
    }
    None
}

/// Parses an ANTLR semantic-predicate fail option following the predicate `?`.
fn predicate_fail_message(source: &str, after_brace: usize) -> Option<String> {
    let rest = source[after_brace..].trim_start();
    let rest = rest.strip_prefix('?')?.trim_start();
    let rest = rest.strip_prefix("<fail=")?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let body_start = quote.len_utf8();
    let body_end = rest[body_start..].find(quote)? + body_start;
    let after_quote = body_end + quote.len_utf8();
    if !rest[after_quote..].trim_start().starts_with('>') {
        return None;
    }
    Some(rest[body_start..body_end].to_owned())
}

/// Finds the next parser action block, including empty actions serialized as
/// no-op ATN action transitions.
fn next_parser_action_block(source: &str, offset: usize) -> Option<TemplateBlock<'_>> {
    let mut cursor = offset;
    while let Some(open_rel) = source[cursor..].find('{') {
        let open_brace = cursor + open_rel;
        let close_brace = matching_action_brace(source, open_brace + 1)?;
        let body = &source[open_brace + 1..close_brace];
        if body.trim().is_empty()
            || template_sequence_bodies(body).is_some()
            || parse_int_return_assignment(body).is_some()
        {
            let after_brace = close_brace + 1;
            return Some(TemplateBlock {
                open_brace,
                body,
                after_brace,
                predicate: source[after_brace..].trim_start().starts_with('?'),
            });
        }
        cursor = open_brace + 1;
    }
    None
}

/// Splits a body made only of adjacent target-template expressions.
fn template_sequence_bodies(body: &str) -> Option<Vec<&str>> {
    let mut templates = Vec::new();
    let mut cursor = 0;
    while cursor < body.len() {
        cursor = skip_ascii_whitespace(body, cursor);
        if cursor == body.len() {
            break;
        }
        if body.as_bytes().get(cursor) != Some(&b'<') {
            return None;
        }
        let close_angle = matching_template_close(body, cursor + 1)?;
        templates.push(&body[cursor + 1..close_angle]);
        cursor = close_angle + 1;
    }
    (!templates.is_empty()).then_some(templates)
}

/// Finds the closing brace for a named ANTLR action block while ignoring braces
/// inside string literals.
fn matching_action_brace(source: &str, mut index: usize) -> Option<usize> {
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
            '{' if !quoted => nested += 1,
            '}' if !quoted && nested == 0 => return Some(index),
            '}' if !quoted => nested = nested.saturating_sub(1),
            _ => {}
        }
        index += ch.len_utf8();
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuleHeader<'a> {
    name: &'a str,
    start: usize,
}

/// Returns the grammar rule that owns an action or signature position by reading
/// the current rule header before the first colon in the statement.
fn statement_rule_header(source: &str, position: usize) -> Option<RuleHeader<'_>> {
    let prefix = source.get(..position)?;
    let (start, header) = prefix.rfind(':').map_or_else(
        || {
            let header_start = prefix.rfind([';', '}']).map_or(0, |index| index + 1);
            (header_start, &prefix[header_start..])
        },
        |colon| {
            let header_start = source[..colon]
                .rfind([';', '}'])
                .map_or(0, |index| index + 1);
            (header_start, &source[header_start..colon])
        },
    );
    let name = leading_rule_name(header)?;
    Some(RuleHeader { name, start })
}

/// Reports whether an earlier rule with the same name already owns the active
/// definition, matching ANTLR's import override rules for composite grammars.
fn has_prior_rule_definition(source: &str, name: &str, before: usize) -> bool {
    let mut offset = 0;
    while let Some(colon) = source[offset..before].find(':').map(|index| offset + index) {
        let header_start = source[..colon]
            .rfind([';', '}'])
            .map_or(0, |index| index + 1);
        if leading_rule_name(&source[header_start..colon]) == Some(name) {
            return true;
        }
        offset = colon + 1;
    }
    false
}

/// Reads the first ANTLR identifier from a rule header, allowing the optional
/// `fragment` prefix used by lexer rules.
fn leading_rule_name(header: &str) -> Option<&str> {
    let header = trim_leading_non_rule_lines(header);
    let header = header
        .strip_prefix("fragment")
        .map_or(header, str::trim_start);
    let end = header
        .char_indices()
        .find_map(|(index, ch)| (!(ch == '_' || ch.is_ascii_alphanumeric())).then_some(index))
        .unwrap_or(header.len());
    let name = &header[..end];
    (!name.is_empty()).then_some(name)
}

/// Drops standalone comment and preamble-template lines that can sit between
/// grammar-level metadata and the next rule header.
fn trim_leading_non_rule_lines(mut header: &str) -> &str {
    loop {
        header = header.trim_start();
        if header.starts_with("//") {
            let Some(newline) = header.find('\n') else {
                return "";
            };
            header = &header[newline + 1..];
            continue;
        }
        if header.starts_with('<') {
            let Some(close) = header.find('>') else {
                return header;
            };
            if header[close + 1..]
                .chars()
                .next()
                .is_none_or(|ch| ch == '\r' || ch == '\n')
            {
                header = &header[close + 1..];
                continue;
            }
        }
        return header;
    }
}

/// Detects member-action blocks whose target code is compile-time scaffolding
/// rather than an ATN semantic action.
fn is_members_action(source: &str, open_brace: usize) -> bool {
    let prefix = source[..open_brace].trim_end();
    prefix.ends_with("@members") || prefix.ends_with("@parser::members")
}

fn is_definitions_action(source: &str, open_brace: usize) -> bool {
    source[..open_brace].trim_end().ends_with("@definitions")
}

/// ANTLR `options { ... }` blocks are grammar metadata, not semantic actions,
/// even though their braces look like empty action transitions to a text scan.
fn is_options_block(source: &str, open_brace: usize) -> bool {
    source[..open_brace].trim_end().ends_with("options")
}

fn uses_alt_number_contexts(source: &str) -> bool {
    source.contains("<TreeNodeWithAltNumField") || source.contains("contextSuperClass")
}

/// Identifies the descriptor listener helper declared in the file-scope
/// preamble; these helpers are test templates, not ANTLR grammar syntax.
fn listener_template_kind(source: &str) -> Option<ListenerKind> {
    source.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with("<BasicListener(") {
            Some(ListenerKind::Basic)
        } else if trimmed.starts_with("<TokenGetterListener(") {
            Some(ListenerKind::TokenGetter)
        } else if trimmed.starts_with("<RuleGetterListener(") {
            Some(ListenerKind::RuleGetter)
        } else if trimmed.starts_with("<LRListener(") {
            Some(ListenerKind::LeftRecursive)
        } else if trimmed.starts_with("<LRWithLabelsListener(") {
            Some(ListenerKind::LeftRecursiveWithLabels)
        } else {
            None
        }
    })
}

fn uses_position_adjusting_lexer(source: &str) -> bool {
    source.contains("<PositionAdjustingLexer()")
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
    let (label, rebuild) = match template {
        ActionTemplate::StringTree {
            target: StringTreeTarget::Label(label),
            newline,
        } => (label, ResolvedAfterAction::StringTree { newline }),
        ActionTemplate::ListenerWalk {
            target: StringTreeTarget::Label(label),
            kind,
        } => (label, ResolvedAfterAction::ListenerWalk { kind }),
        other => return Ok(other),
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
    Ok(rebuild.into_action(rule_index))
}

/// Resolves `$label.return` action templates against `label=rule` occurrences
/// in the owning rule before generated code loses source-level labels.
fn resolve_action_template_labels(
    template: ActionTemplate,
    source: &str,
    open_brace: usize,
) -> ActionTemplate {
    match template {
        ActionTemplate::RuleReturnValue {
            rule_name,
            value_name,
            newline,
        } => {
            let resolved = labeled_rule_name(source, open_brace, &rule_name)
                .unwrap_or(&rule_name)
                .to_owned();
            ActionTemplate::RuleReturnValue {
                rule_name: resolved,
                value_name,
                newline,
            }
        }
        ActionTemplate::Sequence(actions) => ActionTemplate::Sequence(
            actions
                .into_iter()
                .map(|action| resolve_action_template_labels(action, source, open_brace))
                .collect(),
        ),
        other => other,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolvedAfterAction {
    StringTree { newline: bool },
    ListenerWalk { kind: ListenerKind },
}

impl ResolvedAfterAction {
    /// Rebuilds a label-based `@after` action after resolving the label to the
    /// rule index stored in generated parse-tree nodes.
    const fn into_action(self, rule_index: usize) -> ActionTemplate {
        match self {
            Self::StringTree { newline } => ActionTemplate::StringTree {
                target: StringTreeTarget::Rule(rule_index),
                newline,
            },
            Self::ListenerWalk { kind } => ActionTemplate::ListenerWalk {
                target: StringTreeTarget::Rule(rule_index),
                kind,
            },
        }
    }
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
fn parse_action_block_template(body: &str) -> Option<ActionTemplate> {
    if body.trim().is_empty() {
        return Some(ActionTemplate::Noop);
    }
    parse_action_template_sequence(body).or_else(|| parse_int_return_assignment(body))
}

fn parse_action_template_sequence(body: &str) -> Option<ActionTemplate> {
    let parts = template_sequence_bodies(body)?;
    let mut actions = Vec::with_capacity(parts.len());
    for part in parts {
        actions.push(parse_action_template(part)?);
    }
    match actions.as_slice() {
        [action] => Some(action.clone()),
        _ => Some(ActionTemplate::Sequence(actions)),
    }
}

fn parse_action_template(body: &str) -> Option<ActionTemplate> {
    let body = body.trim();
    match body {
        "Pass()" | "LL_EXACT_AMBIG_DETECTION()" => Some(ActionTemplate::Noop),
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
        "Invoke_foo()" => Some(ActionTemplate::Literal {
            value: "foo".to_owned(),
            newline: true,
        }),
        _ => parse_plus_text(body)
            .or_else(|| parse_string_tree(body))
            .or_else(|| parse_rule_invocation_stack(body))
            .or_else(|| parse_append_str_token_text(body))
            .or_else(|| parse_rule_value(body))
            .or_else(|| parse_token_text(body))
            .or_else(|| parse_token_display(body))
            .or_else(|| parse_add_member(body))
            .or_else(|| parse_member_value(body))
            .or_else(|| parse_noop_action(body))
            .or_else(|| parse_write_literal(body)),
    }
}

fn parse_init_int_member(body: &str) -> Option<IntMemberTemplate> {
    let arguments = body
        .strip_prefix("InitIntMember(")
        .and_then(|value| value.strip_suffix(')'))
        .map(split_template_arguments)?;
    let [name, value] = arguments.as_slice() else {
        return None;
    };
    Some(IntMemberTemplate {
        name: parse_template_string(name)?,
        initial_value: parse_template_string(value)?.parse::<i64>().ok()?,
    })
}

fn parse_add_member(body: &str) -> Option<ActionTemplate> {
    let arguments = body
        .strip_prefix("AddMember(")
        .and_then(|value| value.strip_suffix(')'))
        .map(split_template_arguments)?;
    let [member, value] = arguments.as_slice() else {
        return None;
    };
    Some(ActionTemplate::AddMember {
        member: parse_template_string(member)?,
        value: parse_template_string(value)?.parse::<i64>().ok()?,
    })
}

fn parse_member_value(body: &str) -> Option<ActionTemplate> {
    let (newline, argument) = if let Some(argument) = body
        .strip_prefix("writeln(GetMember(")
        .and_then(|value| value.strip_suffix("))"))
    {
        (true, argument)
    } else {
        (
            false,
            body.strip_prefix("write(GetMember(")
                .and_then(|value| value.strip_suffix("))"))?,
        )
    };
    Some(ActionTemplate::MemberValue {
        member: parse_template_string(argument)?,
        newline,
    })
}

/// Parses rule-level `@after` helpers, including listener-suite wrappers that
/// are meaningful only after the selected parse tree is available.
fn parse_after_action_template(
    body: &str,
    listener_kind: Option<ListenerKind>,
) -> Option<ActionTemplate> {
    parse_context_member_string_tree(body)
        .or_else(|| parse_context_member_walk_listener(body, listener_kind?))
        .or_else(|| parse_action_template(body))
}

fn parse_predicate_template(body: &str) -> Option<PredicateTemplate> {
    let body = body.trim();
    if let Some(inner) = single_template_body(body) {
        return parse_predicate_template(inner);
    }
    match body {
        "True()" => Some(PredicateTemplate::True),
        "False()" => Some(PredicateTemplate::False),
        _ => parse_text_equals_predicate(body)
            .or_else(|| parse_token_start_column_equals_predicate(body))
            .or_else(|| parse_column_compare_predicate(body))
            .or_else(|| parse_invoke_predicate(body))
            .or_else(|| parse_val_equals_predicate(body))
            .or_else(|| parse_mod_member_predicate(body))
            .or_else(|| parse_boolean_member_not_predicate(body))
            .or_else(|| parse_lt_equals_predicate(body))
            .or_else(|| parse_la_not_equals_predicate(body)),
    }
}

/// Returns the call body for an action made of exactly one target template.
fn single_template_body(body: &str) -> Option<&str> {
    let body = body.trim();
    if body.as_bytes().first() != Some(&b'<') {
        return None;
    }
    let close = matching_template_close(body, 1)?;
    (close + 1 == body.len()).then_some(&body[1..close])
}

/// Parses `GetMember("name"):Not()` for the runtime testsuite boolean-member
/// fixture, where `name` is initialized to `True()` in `@parser::members`.
fn parse_boolean_member_not_predicate(body: &str) -> Option<PredicateTemplate> {
    let argument = body
        .strip_prefix("GetMember(")
        .and_then(|value| value.strip_suffix("):Not()"))?;
    parse_template_string(argument).map(|_| PredicateTemplate::False)
}

/// Parses integer member modulo predicates such as
/// `ModMemberEquals("i","2","0")`.
fn parse_mod_member_predicate(body: &str) -> Option<PredicateTemplate> {
    let (equals, arguments) = if let Some(arguments) = body
        .strip_prefix("ModMemberEquals(")
        .and_then(|value| value.strip_suffix(')'))
    {
        (true, arguments)
    } else {
        (
            false,
            body.strip_prefix("ModMemberNotEquals(")
                .and_then(|value| value.strip_suffix(')'))?,
        )
    };
    let arguments = split_template_arguments(arguments);
    let [member, modulus, value] = arguments.as_slice() else {
        return None;
    };
    Some(PredicateTemplate::MemberModuloEquals {
        member: parse_template_string(member)?,
        modulus: parse_template_string(modulus)?.parse::<i64>().ok()?,
        value: parse_template_string(value)?.parse::<i64>().ok()?,
        equals,
    })
}

/// Parses simple local integer argument predicates such as
/// `ValEquals("$i","2")`.
fn parse_val_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let arguments = body
        .strip_prefix("ValEquals(")
        .and_then(|value| value.strip_suffix(')'))
        .map(split_template_arguments)?;
    let [local, value] = arguments.as_slice() else {
        return None;
    };
    if parse_template_string(local)? != "$i" {
        return None;
    }
    Some(PredicateTemplate::LocalIntEquals {
        value: parse_template_string(value)?.parse::<i64>().ok()?,
    })
}

/// Parses the runtime-testsuite helper that prints when a predicate is
/// evaluated before returning the wrapped boolean value.
fn parse_invoke_predicate(body: &str) -> Option<PredicateTemplate> {
    let value = body.strip_suffix(":Invoke_pred()")?;
    match value {
        "True()" => Some(PredicateTemplate::Invoke { value: true }),
        "False()" => Some(PredicateTemplate::Invoke { value: false }),
        r#"ValEquals("$i","99")"# => Some(PredicateTemplate::Invoke { value: true }),
        _ => None,
    }
}

fn parse_text_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let argument = body
        .strip_prefix("TextEquals(")
        .and_then(|value| value.strip_suffix(')'))?;
    Some(PredicateTemplate::TextEquals(parse_template_string(
        argument,
    )?))
}

fn parse_token_start_column_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let argument = body
        .strip_prefix("TokenStartColumnEquals(")
        .and_then(|value| value.strip_suffix(')'))?;
    Some(PredicateTemplate::TokenStartColumnEquals(
        parse_template_string(argument)?.parse().ok()?,
    ))
}

/// Parses lexer column predicates serialized by upstream templates as
/// `<Column()> \< 2` or `<Column()> >= 2`.
fn parse_column_compare_predicate(body: &str) -> Option<PredicateTemplate> {
    let rest = body
        .trim()
        .strip_prefix("<Column()>")
        .or_else(|| body.trim().strip_prefix("Column()"))?
        .trim_start();
    let rest = rest.strip_prefix('\\').unwrap_or(rest).trim_start();
    if let Some(value) = rest.strip_prefix('<') {
        return Some(PredicateTemplate::ColumnLessThan(
            value.trim().parse().ok()?,
        ));
    }
    Some(PredicateTemplate::ColumnGreaterOrEqual(
        rest.strip_prefix(">=")?.trim().parse().ok()?,
    ))
}

fn parse_la_not_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let arguments = body
        .strip_prefix("LANotEquals(")
        .and_then(|value| value.strip_suffix(')'))
        .map(split_template_arguments)?;
    let [offset, token] = arguments.as_slice() else {
        return None;
    };
    let offset = parse_template_string(offset)?.parse::<isize>().ok()?;
    let token_name = parse_parser_token_argument(token)?;
    Some(PredicateTemplate::LookaheadNotEquals { offset, token_name })
}

/// Parses `LTEquals` predicates that compare lookahead token text.
///
/// The runtime-testsuite passes the expected text as a quoted target-language
/// string literal, so the decoded `StringTemplate` argument may still contain
/// one nested quote pair.
fn parse_lt_equals_predicate(body: &str) -> Option<PredicateTemplate> {
    let arguments = body
        .strip_prefix("LTEquals(")
        .and_then(|value| value.strip_suffix(')'))
        .map(split_template_arguments)?;
    let [offset, text] = arguments.as_slice() else {
        return None;
    };
    let offset = parse_template_string(offset)?.parse::<isize>().ok()?;
    let text = parse_template_string(text)?;
    let text = text
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(&text)
        .to_owned();
    Some(PredicateTemplate::LookaheadTextEquals { offset, text })
}

fn parse_parser_token_argument(argument: &str) -> Option<String> {
    let body = argument
        .trim()
        .strip_prefix("{T<ParserToken(")?
        .strip_suffix(")>}")?;
    let parts = split_template_arguments(body);
    let [_, token_name] = parts.as_slice() else {
        return None;
    };
    parse_template_string(token_name)
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

/// Parses `ContextMember("$ctx", "label"):ToStringTree():write[ln]()` from the
/// listener descriptors into the same label-resolution path as `$label.ctx`.
fn parse_context_member_string_tree(body: &str) -> Option<ActionTemplate> {
    let (newline, label) = if let Some(arguments) = body
        .strip_prefix("ContextMember(")
        .and_then(|value| value.strip_suffix("):ToStringTree():writeln()"))
    {
        (true, parse_context_member_label(arguments)?)
    } else {
        let arguments = body
            .strip_prefix("ContextMember(")
            .and_then(|value| value.strip_suffix("):ToStringTree():write()"))?;
        (false, parse_context_member_label(arguments)?)
    };
    Some(ActionTemplate::StringTree {
        target: StringTreeTarget::Label(label),
        newline,
    })
}

/// Parses `ContextMember("$ctx", "label"):WalkListener()` and attaches the
/// file-scope listener template selected by the descriptor.
fn parse_context_member_walk_listener(body: &str, kind: ListenerKind) -> Option<ActionTemplate> {
    let arguments = body
        .strip_prefix("ContextMember(")
        .and_then(|value| value.strip_suffix("):WalkListener()"))?;
    Some(ActionTemplate::ListenerWalk {
        target: StringTreeTarget::Label(parse_context_member_label(arguments)?),
        kind,
    })
}

/// Extracts the rule label from `ContextMember("$ctx", "...")`; the first
/// argument is fixed by the upstream templates and identifies the current ctx.
fn parse_context_member_label(arguments: &str) -> Option<String> {
    let arguments = split_template_arguments(arguments);
    let [ctx, label] = arguments.as_slice() else {
        return None;
    };
    (parse_template_string(ctx)? == "$ctx").then(|| parse_template_string(label))?
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
        || body.starts_with("InitIntVar(")
        || body.starts_with("IntArg(")
        || body.starts_with("Production(")
        || body.starts_with("Result(")
        || body.starts_with("SetMember("))
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

/// Parses direct `$label.text` print helpers and maps token-looking labels to
/// the action stop token while rule-looking labels read from the rule start.
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

/// Parses return-value print helpers such as `writeln("$e.v")` from the
/// left-recursion descriptors into parse-tree evaluation actions.
fn parse_rule_value(body: &str) -> Option<ActionTemplate> {
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
    let (rule_name, value_name) = value.strip_prefix('$')?.split_once('.')?;
    if !is_antlr_identifier(rule_name) || !is_antlr_identifier(value_name) {
        return None;
    }
    match value_name {
        "v" => Some(ActionTemplate::RuleValue {
            rule_name: rule_name.to_owned(),
            kind: RuleValueKind::Int,
            newline,
        }),
        "result" => Some(ActionTemplate::RuleValue {
            rule_name: rule_name.to_owned(),
            kind: RuleValueKind::String,
            newline,
        }),
        "text" => None,
        _ => Some(ActionTemplate::RuleReturnValue {
            rule_name: rule_name.to_owned(),
            value_name: value_name.to_owned(),
            newline,
        }),
    }
}

/// Parses simple raw return assignments such as `$y=1000;` into metadata that
/// the runtime can attach to the selected rule context.
fn parse_int_return_assignment(body: &str) -> Option<ActionTemplate> {
    let (name, value) = body
        .trim()
        .strip_prefix('$')?
        .strip_suffix(';')?
        .split_once('=')?;
    let name = name.trim();
    let value = value.trim().parse::<i64>().ok()?;
    is_antlr_identifier(name).then(|| ActionTemplate::SetIntReturn {
        name: name.to_owned(),
        value,
    })
}

/// Parses `AppendStr("prefix", "$text")` and `$TOKEN.text` variants used by
/// parser action descriptors.
fn parse_append_str_token_text(body: &str) -> Option<ActionTemplate> {
    let (newline, arguments) = append_str_arguments(body)?;
    let arguments = split_template_arguments(arguments);
    let [prefix_argument, value_argument] = arguments.as_slice() else {
        return None;
    };
    let prefix = parse_template_string(prefix_argument)?;
    let prefix = prefix
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(&prefix)
        .to_owned();
    let value = parse_template_string(value_argument)?;
    if value == "$text" {
        return Some(ActionTemplate::TextWithPrefix { prefix, newline });
    }
    let label = value.strip_prefix('$')?.strip_suffix(".text")?;
    let source = label
        .chars()
        .next()
        .filter(char::is_ascii_uppercase)
        .map_or(TokenTextSource::RuleStart, |_| TokenTextSource::ActionStop);
    Some(ActionTemplate::TokenTextWithPrefix {
        prefix,
        source,
        newline,
    })
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

/// Extracts the comma-separated arguments from the fluent
/// `AppendStr(...):write[ln]()` forms used by runtime descriptors.
fn append_str_arguments(body: &str) -> Option<(bool, &str)> {
    if let Some(arguments) = body
        .strip_prefix("AppendStr(")
        .and_then(|value| value.strip_suffix("):writeln()"))
    {
        return Some((true, arguments));
    }
    body.strip_prefix("AppendStr(")
        .and_then(|value| value.strip_suffix("):write()"))
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

/// Reads the parser ATN action transitions keyed by source state.
fn parser_action_state_rules(data: &InterpData) -> io::Result<BTreeMap<usize, usize>> {
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(data.atn.clone()))
        .deserialize()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut states = BTreeMap::new();
    for state in atn.states() {
        for transition in &state.transitions {
            if let Transition::Action { rule_index, .. } = transition {
                states.insert(state.state_number, *rule_index);
            }
        }
    }
    Ok(states)
}

/// Pairs supported rule-call arguments from grammar source with the ATN
/// rule-transition source states that carry those calls at runtime.
///
/// Runtime-test templates encode rule arguments in the original grammar text,
/// but the generated `.interp` data only preserves rule-transition structure.
/// Source order is stable for the covered fixtures, so matching grammar calls
/// to same-rule ATN transitions lets the generated parser expose local
/// predicate values without depending on ANTLR's Java code generator.
fn parser_rule_args(
    data: &InterpData,
    grammar_source: &str,
) -> io::Result<Vec<(usize, usize, RuleArgTemplate)>> {
    let calls = literal_rule_arg_calls(data, grammar_source);
    if calls.is_empty() {
        return Ok(Vec::new());
    }
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(data.atn.clone()))
        .deserialize()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut rule_transitions = Vec::new();
    for state in atn.states() {
        for transition in &state.transitions {
            if let Transition::Rule { rule_index, .. } = transition {
                rule_transitions.push((state.state_number, *rule_index));
            }
        }
    }

    let mut used = vec![false; rule_transitions.len()];
    let mut args = Vec::new();
    for (rule_index, value) in calls {
        if let Some((index, (source_state, _))) = rule_transitions
            .iter()
            .enumerate()
            .find(|(index, (_, transition_rule))| !used[*index] && *transition_rule == rule_index)
        {
            used[index] = true;
            args.push((*source_state, rule_index, value));
        }
    }
    Ok(args)
}

/// Extracts calls like `a[2]` and `a[<VarRef("i")>]` while ignoring rule
/// declarations and target templates whose bracket contents are unsupported.
fn literal_rule_arg_calls(
    data: &InterpData,
    grammar_source: &str,
) -> Vec<(usize, RuleArgTemplate)> {
    let mut calls = Vec::new();
    for (rule_index, rule_name) in data.rule_names.iter().enumerate() {
        let pattern = format!("{rule_name}[");
        let mut offset = 0;
        while let Some(start) = grammar_source[offset..]
            .find(&pattern)
            .map(|index| offset + index)
        {
            let value_start = start + pattern.len();
            let Some(value_stop) = grammar_source[value_start..]
                .find(']')
                .map(|index| value_start + index)
            else {
                break;
            };
            if start == 0
                || grammar_source[..start]
                    .chars()
                    .next_back()
                    .is_none_or(|ch| !(ch == '_' || ch.is_ascii_alphanumeric()))
            {
                let value = grammar_source[value_start..value_stop].trim();
                if let Ok(value) = value.parse::<i64>() {
                    calls.push((start, rule_index, RuleArgTemplate::Literal(value)));
                } else if value == r#"<VarRef("i")>"# {
                    calls.push((start, rule_index, RuleArgTemplate::InheritLocal));
                }
            }
            offset = value_stop + 1;
        }
    }
    calls.sort_by_key(|(start, _, _)| *start);
    calls
        .into_iter()
        .map(|(_, rule_index, value)| (rule_index, value))
        .collect()
}

/// Extracts integer parser members declared through supported member templates.
fn parser_int_members(grammar_source: &str) -> Vec<IntMemberTemplate> {
    let mut members = Vec::new();
    for marker in ["@members", "@parser::members"] {
        for block in named_action_templates(grammar_source, marker) {
            if let Some(member) = parse_init_int_member(block.body.trim())
                && !members
                    .iter()
                    .any(|existing: &IntMemberTemplate| existing.name == member.name)
            {
                members.push(member);
            }
        }
    }
    members
}

/// Maps generated action templates that mutate parser members to ATN states.
fn parser_member_actions(
    actions: &[(usize, ActionTemplate)],
    members: &[IntMemberTemplate],
) -> io::Result<Vec<(usize, usize, i64)>> {
    let mut member_actions = Vec::new();
    for (source_state, action) in actions {
        collect_member_actions(*source_state, action, members, &mut member_actions)?;
    }
    Ok(member_actions)
}

/// Maps generated return assignments to ATN action states so the interpreter
/// can attach them to the selected rule context during recognition.
fn parser_return_actions(actions: &[(usize, ActionTemplate)]) -> Vec<(usize, String, i64)> {
    let mut return_actions = Vec::new();
    for (source_state, action) in actions {
        collect_return_actions(*source_state, action, &mut return_actions);
    }
    return_actions
}

fn collect_return_actions(
    source_state: usize,
    action: &ActionTemplate,
    out: &mut Vec<(usize, String, i64)>,
) {
    match action {
        ActionTemplate::SetIntReturn { name, value } => {
            out.push((source_state, name.clone(), *value));
        }
        ActionTemplate::Sequence(actions) => {
            for action in actions {
                collect_return_actions(source_state, action, out);
            }
        }
        ActionTemplate::Noop
        | ActionTemplate::Text { .. }
        | ActionTemplate::TextWithPrefix { .. }
        | ActionTemplate::StringTree { .. }
        | ActionTemplate::RuleInvocationStack { .. }
        | ActionTemplate::ListenerWalk { .. }
        | ActionTemplate::RuleValue { .. }
        | ActionTemplate::RuleReturnValue { .. }
        | ActionTemplate::TokenText { .. }
        | ActionTemplate::TokenTextWithPrefix { .. }
        | ActionTemplate::TokenDisplay { .. }
        | ActionTemplate::ExpectedTokenNames { .. }
        | ActionTemplate::Literal { .. }
        | ActionTemplate::AddMember { .. }
        | ActionTemplate::MemberValue { .. } => {}
    }
}

fn collect_member_actions(
    source_state: usize,
    action: &ActionTemplate,
    members: &[IntMemberTemplate],
    out: &mut Vec<(usize, usize, i64)>,
) -> io::Result<()> {
    match action {
        ActionTemplate::AddMember { member, value } => {
            let member = member_id(members, member)?;
            out.push((source_state, member, *value));
        }
        ActionTemplate::Sequence(actions) => {
            for action in actions {
                collect_member_actions(source_state, action, members, out)?;
            }
        }
        ActionTemplate::Noop
        | ActionTemplate::Text { .. }
        | ActionTemplate::TextWithPrefix { .. }
        | ActionTemplate::StringTree { .. }
        | ActionTemplate::RuleInvocationStack { .. }
        | ActionTemplate::ListenerWalk { .. }
        | ActionTemplate::RuleValue { .. }
        | ActionTemplate::RuleReturnValue { .. }
        | ActionTemplate::SetIntReturn { .. }
        | ActionTemplate::TokenText { .. }
        | ActionTemplate::TokenTextWithPrefix { .. }
        | ActionTemplate::TokenDisplay { .. }
        | ActionTemplate::ExpectedTokenNames { .. }
        | ActionTemplate::Literal { .. }
        | ActionTemplate::MemberValue { .. } => {}
    }
    Ok(())
}

/// Emits the helper methods for ANTLR's `PositionAdjustingLexer` runtime-test
/// target template.
///
/// The template accepts a longer lexer path for keywords and labels, then emits
/// only the keyword or identifier prefix. Resetting the accept position leaves
/// delimiters such as `{`, `=`, and `+=` available for the next token.
fn render_position_adjusting_lexer_methods() -> String {
    r#"
    fn adjust_accept_position(base: &mut BaseLexer<I>, token_type: i32, accept_position: usize) {
        match token_type {
            TOKENS => Self::adjust_accept_position_for_keyword(base, accept_position, "tokens"),
            LABEL => Self::adjust_accept_position_for_identifier(base, accept_position),
            _ => {}
        }
    }

    fn adjust_accept_position_for_identifier(base: &mut BaseLexer<I>, accept_position: usize) {
        let identifier_length = base
            .token_text_until(accept_position)
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .count();
        Self::reset_accept_position_after_prefix(base, accept_position, identifier_length);
    }

    fn adjust_accept_position_for_keyword(
        base: &mut BaseLexer<I>,
        accept_position: usize,
        keyword: &str,
    ) {
        Self::reset_accept_position_after_prefix(
            base,
            accept_position,
            keyword.chars().count(),
        );
    }

    fn reset_accept_position_after_prefix(
        base: &mut BaseLexer<I>,
        accept_position: usize,
        prefix_length: usize,
    ) {
        let target = base.token_start().saturating_add(prefix_length);
        if accept_position > target {
            base.reset_accept_position(target);
        }
    }
"#
    .to_owned()
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
        ActionTemplate::TokenTextWithPrefix {
            prefix, newline, ..
        } => {
            let write = if *newline { "println!" } else { "print!" };
            format!(
                "let text = _base.token_text_until(action.position()); {write}(\"{}{{}}\", text);",
                rust_string(prefix)
            )
        }
        ActionTemplate::TokenDisplay { .. } => String::new(),
        ActionTemplate::ExpectedTokenNames { .. } => String::new(),
        ActionTemplate::StringTree { .. } => String::new(),
        ActionTemplate::RuleInvocationStack { .. } => String::new(),
        ActionTemplate::ListenerWalk { .. } => String::new(),
        ActionTemplate::RuleValue { .. } => String::new(),
        ActionTemplate::RuleReturnValue { .. } => String::new(),
        ActionTemplate::SetIntReturn { .. } => String::new(),
        ActionTemplate::AddMember { .. } => String::new(),
        ActionTemplate::MemberValue { .. } => String::new(),
        ActionTemplate::Sequence(actions) => actions
            .iter()
            .map(render_lexer_action_statement)
            .collect::<Vec<_>>()
            .join(" "),
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
        PredicateTemplate::TokenStartColumnEquals(value) => {
            format!("_base.token_start_column() == {value}")
        }
        PredicateTemplate::ColumnLessThan(value) => {
            format!("_base.column_at(predicate.position()) < {value}")
        }
        PredicateTemplate::ColumnGreaterOrEqual(value) => {
            format!("_base.column_at(predicate.position()) >= {value}")
        }
        PredicateTemplate::Invoke { .. }
        | PredicateTemplate::FalseWithMessage { .. }
        | PredicateTemplate::LocalIntEquals { .. }
        | PredicateTemplate::MemberModuloEquals { .. }
        | PredicateTemplate::LookaheadTextEquals { .. }
        | PredicateTemplate::LookaheadNotEquals { .. } => {
            unreachable!("lookahead parser predicates are not lexer predicates")
        }
    }
}

/// Emits the generated parser action dispatcher for the grammar-specific action
/// source states discovered from the serialized ATN.
fn render_parser_action_method(
    actions: &[(usize, ActionTemplate)],
    init_actions: &[Option<ActionTemplate>],
    members: &[IntMemberTemplate],
) -> io::Result<String> {
    let has_init_actions = init_actions.iter().any(Option::is_some);
    if actions.is_empty() && !has_init_actions {
        return Ok(String::new());
    }
    let mut init_arms = String::new();
    for (rule_index, template) in init_actions.iter().enumerate() {
        let Some(template) = template else {
            continue;
        };
        let statement = render_action_statement(template, members)?;
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
        let statement = render_action_statement(template, members)?;
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
    Ok(format!(
        "    fn run_action(&mut self, action: antlr4_runtime::ParserAction, _tree: &antlr4_runtime::ParseTree) {{\n{init_dispatch}        match action.source_state() {{\n{arms}        }}\n    }}\n"
    ))
}

/// Renders one supported target-template action as Rust code.
fn render_action_statement(
    template: &ActionTemplate,
    members: &[IntMemberTemplate],
) -> io::Result<String> {
    match template {
        ActionTemplate::Noop => Ok(String::new()),
        ActionTemplate::Text { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(format!(
                "let text = self.base.text_interval(action.start_index(), action.stop_index()); {write}(\"{{}}\", text);"
            ))
        }
        ActionTemplate::TextWithPrefix { prefix, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(format!(
                "let text = self.base.text_interval(action.start_index(), action.stop_index()); {write}(\"{}{{}}\", text);",
                rust_string(prefix)
            ))
        }
        ActionTemplate::TokenText { source, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(match source {
                TokenTextSource::RuleStart => format!(
                    "let text = self.base.text_interval(action.start_index(), Some(action.start_index())); {write}(\"{{}}\", text);"
                ),
                TokenTextSource::ActionStop => format!(
                    "let text = action.stop_index().map_or_else(String::new, |index| self.base.text_interval(index, Some(index))); {write}(\"{{}}\", text);"
                ),
            })
        }
        ActionTemplate::TokenTextWithPrefix {
            prefix,
            source,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            let prefix = rust_string(prefix);
            Ok(match source {
                TokenTextSource::RuleStart => format!(
                    "let text = self.base.text_interval(action.start_index(), Some(action.start_index())); {write}(\"{prefix}{{}}\", text);"
                ),
                TokenTextSource::ActionStop => format!(
                    "let text = action.stop_index().map_or_else(String::new, |index| self.base.text_interval(index, Some(index))); {write}(\"{prefix}{{}}\", text);"
                ),
            })
        }
        ActionTemplate::TokenDisplay {
            prefix,
            source,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(render_token_display_write(
                write, "_tree", "action", prefix, source,
            ))
        }
        ActionTemplate::ExpectedTokenNames { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(format!(
                "let text = action.expected_state().map_or_else(String::new, |state| self.base.expected_tokens_at_state(atn(), state)); {write}(\"{{}}\", text);"
            ))
        }
        ActionTemplate::StringTree { target, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(render_string_tree_write(write, "_tree", target))
        }
        ActionTemplate::RuleInvocationStack { newline } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(render_rule_invocation_stack_write(
                write,
                "_tree",
                "action.rule_index()",
            ))
        }
        ActionTemplate::ListenerWalk { .. } => Ok(String::new()),
        ActionTemplate::RuleValue {
            rule_name,
            kind,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(render_rule_value_write(write, "_tree", rule_name, *kind))
        }
        ActionTemplate::RuleReturnValue {
            rule_name,
            value_name,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(render_rule_return_value_write(
                write, "_tree", rule_name, value_name,
            ))
        }
        ActionTemplate::SetIntReturn { .. } => Ok(String::new()),
        ActionTemplate::Literal { value, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(format!("{write}(\"{}\");", rust_string(value)))
        }
        ActionTemplate::AddMember { member, value } => {
            let member = member_id(members, member)?;
            Ok(format!("self.base.add_int_member({member}, {value});"))
        }
        ActionTemplate::MemberValue { member, newline } => {
            let member = member_id(members, member)?;
            let write = if *newline { "println!" } else { "print!" };
            Ok(format!(
                "{write}(\"{{}}\", self.base.int_member({member}).unwrap_or_default());"
            ))
        }
        ActionTemplate::Sequence(actions) => {
            let mut rendered = Vec::with_capacity(actions.len());
            for action in actions {
                rendered.push(render_action_statement(action, members)?);
            }
            Ok(rendered.join(" "))
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
        ActionTemplate::TokenTextWithPrefix {
            prefix,
            source,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            let prefix = rust_string(prefix);
            match source {
                TokenTextSource::RuleStart => format!(
                    "let text = self.base.text_interval(start_index, Some(start_index)); {write}(\"{prefix}{{}}\", text);"
                ),
                TokenTextSource::ActionStop => format!(
                    "let text = stop_index.map_or_else(String::new, |index| self.base.text_interval(index, Some(index))); {write}(\"{prefix}{{}}\", text);"
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
        ActionTemplate::ListenerWalk { target, kind } => render_listener_walk(target, *kind),
        ActionTemplate::RuleValue {
            rule_name,
            kind,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            render_rule_value_write(write, "tree", rule_name, *kind)
        }
        ActionTemplate::RuleReturnValue {
            rule_name,
            value_name,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            render_rule_return_value_write(write, "tree", rule_name, value_name)
        }
        ActionTemplate::Literal { value, newline } => {
            let write = if *newline { "println!" } else { "print!" };
            format!("{write}(\"{}\");", rust_string(value))
        }
        ActionTemplate::SetIntReturn { .. }
        | ActionTemplate::AddMember { .. }
        | ActionTemplate::MemberValue { .. } => String::new(),
        ActionTemplate::Sequence(actions) => actions
            .iter()
            .map(|action| render_parser_after_action_statement(action, rule_index))
            .collect::<Vec<_>>()
            .join(" "),
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

/// Emits a rule-return print helper backed by return slots captured on the
/// generated parse tree during metadata-driven recognition.
fn render_rule_return_value_write(
    write: &str,
    tree_expr: &str,
    rule_name: &str,
    value_name: &str,
) -> String {
    let rule_name = rust_string(rule_name);
    let value_name = rust_string(value_name);
    format!(
        "let text = METADATA.rule_names().iter().position(|name| *name == \"{rule_name}\").and_then(|rule_index| {tree_expr}.first_rule_int_return(rule_index, \"{value_name}\")).map_or_else(String::new, |value| value.to_string()); {write}(\"{{}}\", text);"
    )
}

/// Emits a return-value print helper for the left-recursion descriptors by
/// evaluating the selected rule's token text from the generated parse tree.
fn render_rule_value_write(
    write: &str,
    tree_expr: &str,
    rule_name: &str,
    kind: RuleValueKind,
) -> String {
    let rule_name = rust_string(rule_name);
    let evaluator = match kind {
        RuleValueKind::Int => {
            r#"
fn parse_primary(chars: &[char], index: &mut usize) -> i64 {
    if chars.get(*index) == Some(&'(') {
        *index += 1;
        let value = parse_sum(chars, index);
        if chars.get(*index) == Some(&')') {
            *index += 1;
        }
        return value;
    }
    if chars.get(*index).is_some_and(|ch| ch.is_ascii_alphabetic()) {
        while chars.get(*index).is_some_and(|ch| ch.is_ascii_alphabetic()) {
            *index += 1;
        }
        let mut value = 3;
        while *index + 1 < chars.len() && chars[*index] == '+' && chars[*index + 1] == '+' {
            *index += 2;
            value += 1;
        }
        while *index + 1 < chars.len() && chars[*index] == '-' && chars[*index + 1] == '-' {
            *index += 2;
            value -= 1;
        }
        return value;
    }
    let start = *index;
    while chars.get(*index).is_some_and(|ch| ch.is_ascii_digit()) {
        *index += 1;
    }
    chars[start..*index]
        .iter()
        .collect::<String>()
        .parse::<i64>()
        .unwrap_or_default()
}
fn parse_product(chars: &[char], index: &mut usize) -> i64 {
    let mut value = parse_primary(chars, index);
    while chars.get(*index) == Some(&'*') {
        *index += 1;
        value *= parse_primary(chars, index);
    }
    value
}
fn parse_sum(chars: &[char], index: &mut usize) -> i64 {
    let mut value = parse_product(chars, index);
    while chars.get(*index) == Some(&'+') {
        *index += 1;
        value += parse_product(chars, index);
    }
    value
}
fn eval_rule_value(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0;
    parse_sum(&chars, &mut index).to_string()
}
"#
        }
        RuleValueKind::String => {
            r#"
fn find_top_level_plus(chars: &[char]) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, ch) in chars.iter().enumerate().rev() {
        match ch {
            ')' => depth += 1,
            '(' => depth = depth.saturating_sub(1),
            '+' if depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
}
fn eval_string_value(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if let Some(index) = find_top_level_plus(&chars) {
        let left = eval_string_value(&text[..index]);
        let right = eval_string_value(&text[index + 1..]);
        return format!("({left}+{right})");
    }
    if let Some(index) = text.find('=') {
        let left = &text[..index];
        let right = eval_string_value(&text[index + 1..]);
        return format!("({left}={right})");
    }
    text.to_owned()
}
fn eval_rule_value(text: &str) -> String {
    eval_string_value(text)
}
"#
        }
    };
    format!(
        "{evaluator}
let text = METADATA
    .rule_names()
    .iter()
    .position(|name| *name == \"{rule_name}\")
    .and_then(|rule_index| {tree_expr}.first_rule(rule_index))
    .map_or_else(|| eval_rule_value(&{tree_expr}.text()), |node| eval_rule_value(&node.text()));
{write}(\"{{}}\", text);"
    )
}

/// Emits the small listener bodies used by the upstream listener descriptors.
/// These are target-template test fixtures, so the generated code mirrors their
/// observable callbacks without exposing them as a stable listener API.
fn render_listener_walk(target: &StringTreeTarget, kind: ListenerKind) -> String {
    let StringTreeTarget::Rule(rule_index) = target else {
        return String::new();
    };
    let template = match kind {
        ListenerKind::Basic => {
            r#"
fn visit_listener_node(node: &antlr4_runtime::ParseTree) {
    match node {
        antlr4_runtime::ParseTree::Rule(rule) => {
            for child in rule.context().children() {
                visit_listener_node(child);
            }
        }
        antlr4_runtime::ParseTree::Terminal(node) => {
            println!("{}", antlr4_runtime::Token::text(node.symbol()).unwrap_or(""));
        }
        antlr4_runtime::ParseTree::Error(node) => {
            println!("{}", antlr4_runtime::Token::text(node.symbol()).unwrap_or(""));
        }
    }
}
if let Some(node) = tree.first_rule(__TARGET_RULE__) {
    visit_listener_node(node);
}
"#
        }
        ListenerKind::TokenGetter => {
            r#"
fn terminal_tokens<'a>(
    ctx: &'a antlr4_runtime::ParserRuleContext,
) -> Vec<&'a antlr4_runtime::CommonToken> {
    ctx.children()
        .iter()
        .filter_map(|child| match child {
            antlr4_runtime::ParseTree::Terminal(node) => Some(node.symbol()),
            antlr4_runtime::ParseTree::Error(node) => Some(node.symbol()),
            antlr4_runtime::ParseTree::Rule(_) => None,
        })
        .collect()
}
fn token_text(token: &antlr4_runtime::CommonToken) -> &str {
    antlr4_runtime::Token::text(token).unwrap_or("")
}
if let Some(antlr4_runtime::ParseTree::Rule(rule)) = tree.first_rule(__TARGET_RULE__) {
    let tokens = terminal_tokens(rule.context());
    match tokens.as_slice() {
        [first, second] => {
            let list = tokens
                .iter()
                .map(|token| token_text(token).to_owned())
                .collect::<Vec<_>>()
                .join(", ");
            println!("{} {} [{}]", token_text(first), token_text(second), list);
        }
        [token] => println!("{}", *token),
        _ => {}
    }
}
"#
        }
        ListenerKind::RuleGetter => {
            r#"
fn rule_children<'a>(
    ctx: &'a antlr4_runtime::ParserRuleContext,
    rule_index: usize,
) -> Vec<&'a antlr4_runtime::ParserRuleContext> {
    ctx.children()
        .iter()
        .filter_map(|child| match child {
            antlr4_runtime::ParseTree::Rule(rule)
                if rule.context().rule_index() == rule_index =>
            {
                Some(rule.context())
            }
            _ => None,
        })
        .collect()
}
fn start_text(ctx: &antlr4_runtime::ParserRuleContext) -> &str {
    ctx.start().and_then(antlr4_runtime::Token::text).unwrap_or("")
}
let b_rule = METADATA
    .rule_names()
    .iter()
    .position(|name| *name == "b")
    .unwrap_or(usize::MAX);
if let Some(antlr4_runtime::ParseTree::Rule(rule)) = tree.first_rule(__TARGET_RULE__) {
    let rules = rule_children(rule.context(), b_rule);
    match rules.as_slice() {
        [first, second] => println!(
            "{} {} {}",
            start_text(first),
            start_text(second),
            start_text(first)
        ),
        [only] => println!("{}", start_text(only)),
        _ => {}
    }
}
"#
        }
        ListenerKind::LeftRecursive => {
            r#"
fn rule_children<'a>(
    ctx: &'a antlr4_runtime::ParserRuleContext,
    rule_index: usize,
) -> Vec<&'a antlr4_runtime::ParserRuleContext> {
    ctx.children()
        .iter()
        .filter_map(|child| match child {
            antlr4_runtime::ParseTree::Rule(rule)
                if rule.context().rule_index() == rule_index =>
            {
                Some(rule.context())
            }
            _ => None,
        })
        .collect()
}
fn start_text(ctx: &antlr4_runtime::ParserRuleContext) -> &str {
    ctx.start().and_then(antlr4_runtime::Token::text).unwrap_or("")
}
fn first_terminal_text(ctx: &antlr4_runtime::ParserRuleContext) -> Option<&str> {
    ctx.children().iter().find_map(|child| match child {
        antlr4_runtime::ParseTree::Terminal(node) => antlr4_runtime::Token::text(node.symbol()),
        antlr4_runtime::ParseTree::Error(node) => antlr4_runtime::Token::text(node.symbol()),
        antlr4_runtime::ParseTree::Rule(_) => None,
    })
}
fn walk_lr(node: &antlr4_runtime::ParseTree, e_rule: usize) {
    if let antlr4_runtime::ParseTree::Rule(rule) = node {
        for child in rule.context().children() {
            walk_lr(child, e_rule);
        }
        let ctx = rule.context();
        if ctx.rule_index() == e_rule {
            if ctx.children().len() == 3 {
                let rules = rule_children(ctx, e_rule);
                if rules.len() >= 2 {
                    println!(
                        "{} {} {}",
                        start_text(rules[0]),
                        start_text(rules[1]),
                        start_text(rules[0])
                    );
                }
            } else if let Some(text) = first_terminal_text(ctx) {
                println!("{text}");
            }
        }
    }
}
let e_rule = METADATA
    .rule_names()
    .iter()
    .position(|name| *name == "e")
    .unwrap_or(usize::MAX);
if let Some(node) = tree.first_rule(__TARGET_RULE__) {
    walk_lr(node, e_rule);
}
"#
        }
        ListenerKind::LeftRecursiveWithLabels => {
            r#"
fn rule_children<'a>(
    ctx: &'a antlr4_runtime::ParserRuleContext,
    rule_index: usize,
) -> Vec<&'a antlr4_runtime::ParserRuleContext> {
    ctx.children()
        .iter()
        .filter_map(|child| match child {
            antlr4_runtime::ParseTree::Rule(rule)
                if rule.context().rule_index() == rule_index =>
            {
                Some(rule.context())
            }
            _ => None,
        })
        .collect()
}
fn first_rule_child(
    ctx: &antlr4_runtime::ParserRuleContext,
    rule_index: usize,
) -> Option<&antlr4_runtime::ParserRuleContext> {
    ctx.children().iter().find_map(|child| match child {
        antlr4_runtime::ParseTree::Rule(rule) if rule.context().rule_index() == rule_index => {
            Some(rule.context())
        }
        _ => None,
    })
}
fn start_text(ctx: &antlr4_runtime::ParserRuleContext) -> &str {
    ctx.start().and_then(antlr4_runtime::Token::text).unwrap_or("")
}
fn first_terminal_text(ctx: &antlr4_runtime::ParserRuleContext) -> Option<&str> {
    ctx.children().iter().find_map(|child| match child {
        antlr4_runtime::ParseTree::Terminal(node) => antlr4_runtime::Token::text(node.symbol()),
        antlr4_runtime::ParseTree::Error(node) => antlr4_runtime::Token::text(node.symbol()),
        antlr4_runtime::ParseTree::Rule(_) => None,
    })
}
fn walk_lr_labels(node: &antlr4_runtime::ParseTree, e_rule: usize, e_list_rule: usize) {
    if let antlr4_runtime::ParseTree::Rule(rule) = node {
        for child in rule.context().children() {
            walk_lr_labels(child, e_rule, e_list_rule);
        }
        let ctx = rule.context();
        if ctx.rule_index() == e_rule {
            if let Some(e_list_ctx) = first_rule_child(ctx, e_list_rule) {
                let e_children = rule_children(ctx, e_rule);
                let callee = e_children.first().map_or("", |child| start_text(child));
                println!(
                    "{} [{} {}]",
                    callee,
                    e_list_ctx.invoking_state(),
                    ctx.invoking_state()
                );
            } else if let Some(text) = first_terminal_text(ctx) {
                println!("{text}");
            }
        }
    }
}
let e_rule = METADATA
    .rule_names()
    .iter()
    .position(|name| *name == "e")
    .unwrap_or(usize::MAX);
let e_list_rule = METADATA
    .rule_names()
    .iter()
    .position(|name| *name == "eList")
    .unwrap_or(usize::MAX);
if let Some(node) = tree.first_rule(__TARGET_RULE__) {
    walk_lr_labels(node, e_rule, e_list_rule);
}
"#
        }
    };
    render_with_target_rule(template, *rule_index)
}

/// Expands the target-rule placeholder without using `str::replace`, which is
/// disallowed by the repository Clippy policy because it hides allocation.
fn render_with_target_rule(template: &str, rule_index: usize) -> String {
    const PLACEHOLDER: &str = "__TARGET_RULE__";
    let rule_index = rule_index.to_string();
    let mut out = String::with_capacity(template.len() + rule_index.len());
    let mut rest = template;
    while let Some(index) = rest.find(PLACEHOLDER) {
        out.push_str(&rest[..index]);
        out.push_str(&rule_index);
        rest = &rest[index + PLACEHOLDER.len()..];
    }
    out.push_str(rest);
    out
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

/// Renders parser predicate metadata as an inline slice consumed by the runtime
/// parser interpreter.
fn render_parser_predicate_array(
    predicates: &[((usize, usize), PredicateTemplate)],
    data: &InterpData,
    members: &[IntMemberTemplate],
) -> io::Result<String> {
    let mut items = Vec::new();
    for ((rule_index, pred_index), predicate) in predicates {
        let expression = match predicate {
            PredicateTemplate::True => "antlr4_runtime::ParserPredicate::True".to_owned(),
            PredicateTemplate::False => "antlr4_runtime::ParserPredicate::False".to_owned(),
            PredicateTemplate::FalseWithMessage { message } => {
                format!(
                    "antlr4_runtime::ParserPredicate::FalseWithMessage {{ message: \"{}\" }}",
                    rust_string(message)
                )
            }
            PredicateTemplate::Invoke { value } => {
                format!("antlr4_runtime::ParserPredicate::Invoke {{ value: {value} }}")
            }
            PredicateTemplate::LocalIntEquals { value } => {
                format!("antlr4_runtime::ParserPredicate::LocalIntEquals {{ value: {value} }}")
            }
            PredicateTemplate::MemberModuloEquals {
                member,
                modulus,
                value,
                equals,
            } => {
                let member = member_id(members, member)?;
                format!(
                    "antlr4_runtime::ParserPredicate::MemberModuloEquals {{ member: {member}, modulus: {modulus}, value: {value}, equals: {equals} }}"
                )
            }
            PredicateTemplate::TextEquals(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TextEquals is only supported for lexer predicates",
                ));
            }
            PredicateTemplate::TokenStartColumnEquals(_)
            | PredicateTemplate::ColumnLessThan(_)
            | PredicateTemplate::ColumnGreaterOrEqual(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "column predicates are only supported for lexer predicates",
                ));
            }
            PredicateTemplate::LookaheadTextEquals { offset, text } => {
                format!(
                    "antlr4_runtime::ParserPredicate::LookaheadTextEquals {{ offset: {offset}, text: \"{}\" }}",
                    rust_string(text)
                )
            }
            PredicateTemplate::LookaheadNotEquals { offset, token_name } => {
                let token_type = token_type_for_name(data, token_name).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unknown predicate token {token_name}"),
                    )
                })?;
                format!(
                    "antlr4_runtime::ParserPredicate::LookaheadNotEquals {{ offset: {offset}, token_type: {token_type} }}"
                )
            }
        };
        items.push(format!("({rule_index}, {pred_index}, {expression})"));
    }
    Ok(format!("[{}]", items.join(", ")))
}

/// Renders parser rule-argument metadata for generated calls into the runtime.
fn render_parser_rule_arg_array(args: &[(usize, usize, RuleArgTemplate)]) -> String {
    let items = args
        .iter()
        .map(|(source_state, rule_index, value)| {
            let (value, inherit_local) = match value {
                RuleArgTemplate::Literal(value) => (*value, false),
                RuleArgTemplate::InheritLocal => (0, true),
            };
            format!(
                "antlr4_runtime::ParserRuleArg {{ source_state: {source_state}, rule_index: {rule_index}, value: {value}, inherit_local: {inherit_local} }}"
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

/// Renders parser member-action metadata for speculative predicate evaluation.
fn render_parser_member_action_array(args: &[(usize, usize, i64)]) -> String {
    let items = args
        .iter()
        .map(|(source_state, member, delta)| {
            format!(
                "antlr4_runtime::ParserMemberAction {{ source_state: {source_state}, member: {member}, delta: {delta} }}"
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

/// Renders parser return-assignment metadata keyed by ATN action state.
fn render_parser_return_action_array(
    args: &[(usize, String, i64)],
    data: &InterpData,
) -> io::Result<String> {
    if args.is_empty() {
        return Ok("[]".to_owned());
    }
    let action_rules = parser_action_state_rules(data)?;
    let mut items = Vec::new();
    for (source_state, name, value) in args {
        let rule_index = action_rules.get(source_state).copied().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("return assignment has no action transition at state {source_state}"),
            )
        })?;
        items.push(format!(
            "antlr4_runtime::ParserReturnAction {{ source_state: {source_state}, rule_index: {rule_index}, name: \"{}\", value: {value} }}",
            rust_string(name)
        ));
    }
    Ok(format!("[{}]", items.join(", ")))
}

/// Renders the generated parser base construction and member initialization.
fn render_parser_base_initialization(members: &[IntMemberTemplate]) -> String {
    let mut out = if members.is_empty() {
        "        let base = BaseParser::new(input, data);".to_owned()
    } else {
        "        let mut base = BaseParser::new(input, data);".to_owned()
    };
    let initializers = members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let value = member.initial_value;
            format!("        base.set_int_member({index}, {value});")
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !initializers.is_empty() {
        out.push('\n');
        out.push_str(&initializers);
    }
    out
}

fn member_id(members: &[IntMemberTemplate], name: &str) -> io::Result<usize> {
    members
        .iter()
        .position(|member| member.name == name)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown parser member {name}"),
            )
        })
}

fn token_type_for_name(data: &InterpData, token_name: &str) -> Option<usize> {
    data.symbolic_names
        .iter()
        .position(|name| name.as_deref() == Some(token_name))
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
    fn parses_column_predicate_templates() {
        assert_eq!(
            parse_predicate_template(r#"<TokenStartColumnEquals("0")>"#),
            Some(PredicateTemplate::TokenStartColumnEquals(0))
        );
        assert_eq!(
            parse_predicate_template(r#"<Column()> \< 2"#),
            Some(PredicateTemplate::ColumnLessThan(2))
        );
        assert_eq!(
            parse_predicate_template("<Column()> >= 2"),
            Some(PredicateTemplate::ColumnGreaterOrEqual(2))
        );
    }

    #[test]
    fn extracts_predicate_expression_blocks() {
        let templates = extract_supported_predicate_templates(
            r#"fragment ID1 : { <Column()> \< 2 }? [a-zA-Z];
fragment ID2 : { <Column()> >= 2 }? [a-zA-Z];"#,
        )
        .expect("supported predicate expressions should extract");

        assert_eq!(
            templates,
            [
                PredicateTemplate::ColumnLessThan(2),
                PredicateTemplate::ColumnGreaterOrEqual(2)
            ]
        );
    }

    #[test]
    fn parses_predicate_fail_option_message() {
        let grammar = "a : a ID {<False()>}?<fail='custom message'> | ID ;";
        let block =
            next_predicate_action_block(grammar, 0).expect("predicate block should be present");

        assert_eq!(
            predicate_fail_message(grammar, block.after_brace),
            Some("custom message".to_owned())
        );
        assert_eq!(
            predicate_template_with_fail_message(
                PredicateTemplate::False,
                "custom message".to_owned(),
            ),
            PredicateTemplate::FalseWithMessage {
                message: "custom message".to_owned()
            }
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

    #[test]
    fn parses_rule_value_print_template() {
        let template = parse_action_template(r#"writeln("$e.result")"#)
            .expect("rule value print helper should parse");

        assert!(matches!(
            template,
            ActionTemplate::RuleValue {
                rule_name,
                kind: RuleValueKind::String,
                newline: true,
            } if rule_name == "e"
        ));
    }

    #[test]
    fn parses_rule_return_assignment_and_label_read() {
        assert!(matches!(
            parse_action_block_template("$y=1000;"),
            Some(ActionTemplate::SetIntReturn { name, value }) if name == "y" && value == 1000
        ));

        let template = parse_action_template(r#"writeln("$label.y")"#)
            .expect("rule return print helper should parse");
        let resolved = resolve_action_template_labels(
            template,
            "s : label=a[3] {<writeln(\"$label.y\")>} ;",
            15,
        );

        assert!(matches!(
            resolved,
            ActionTemplate::RuleReturnValue {
                rule_name,
                value_name,
                newline: true,
            } if rule_name == "a" && value_name == "y"
        ));
    }

    #[test]
    fn parses_common_label_compile_check_templates_as_noops() {
        assert!(matches!(
            parse_action_template(r#"Production("e")"#),
            Some(ActionTemplate::Noop)
        ));
        assert!(matches!(
            parse_action_template(r#"Result("v")"#),
            Some(ActionTemplate::Noop)
        ));
    }

    #[test]
    fn parses_member_scaffolding_templates() {
        assert!(matches!(
            parse_action_template(r#"SetMember("i","1")"#),
            Some(ActionTemplate::Noop)
        ));
        assert_eq!(
            parse_invoke_predicate(r#"True():Invoke_pred()"#),
            Some(PredicateTemplate::Invoke { value: true })
        );
        assert_eq!(
            parse_invoke_predicate(r#"False():Invoke_pred()"#),
            Some(PredicateTemplate::Invoke { value: false })
        );
        assert_eq!(
            parse_val_equals_predicate(r#"ValEquals("$i","2")"#),
            Some(PredicateTemplate::LocalIntEquals { value: 2 })
        );
        assert_eq!(
            parse_boolean_member_not_predicate(r#"GetMember("enumKeyword"):Not()"#),
            Some(PredicateTemplate::False)
        );
    }
}
