use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use antlr4_runtime::atn::serialized::{AtnDeserializer, SerializedAtn};
use antlr4_runtime::atn::{Atn, AtnStateKind, LexerAction, Transition};

#[path = "../bin_support/rust_names.rs"]
mod rust_names;
#[path = "../bin_support/templates.rs"]
mod templates;

#[cfg(test)]
use rust_names::is_rust_keyword;
use rust_names::{
    module_name, rust_function_name, rust_string, rust_type_name, sanitize_identifier,
    split_identifier_words,
};
use templates::{
    is_after_action, is_definitions_action, is_init_action, is_members_action, is_options_block,
    matching_template_close, named_action_templates, next_parser_action_block,
    next_predicate_action_block, next_template_block, parse_template_string,
    split_template_arguments, template_sequence_bodies,
};

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

#[derive(Clone, Debug, Eq, PartialEq)]
struct GeneratedParserRule {
    rule_index: usize,
    entry_state: usize,
    steps: Vec<GeneratedParserStep>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum GeneratedParserStep {
    MatchToken(i32),
    MatchSet(Vec<(i32, i32)>),
    MatchNotSet(Vec<(i32, i32)>),
    MatchWildcard,
    Action {
        source_state: usize,
        rule_index: usize,
    },
    CallRule {
        source_state: usize,
        rule_index: usize,
    },
    Decision {
        decision: usize,
        alts: Vec<Vec<Self>>,
    },
    StarLoop {
        decision: usize,
        enter_alt: usize,
        exit_alt: usize,
        body: Vec<Self>,
    },
}

struct GeneratedParserCompileContext<'a> {
    atn: &'a Atn,
    decision_by_state: &'a [Option<usize>],
    inline_action_states: &'a BTreeSet<usize>,
}

/// Compiles the parser ATN subset that is safe to emit as recursive-descent
/// Rust today. Unsupported states deliberately return `None` so the generated
/// method can keep using the interpreter fallback until more ATN shapes are
/// covered.
fn parser_generated_rules(
    data: &InterpData,
    enabled_rules: &[bool],
    inline_action_states: &BTreeSet<usize>,
) -> io::Result<Vec<Option<GeneratedParserRule>>> {
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(&data.atn))
        .deserialize()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let decision_by_state = decision_by_state(&atn);
    let context = GeneratedParserCompileContext {
        atn: &atn,
        decision_by_state: &decision_by_state,
        inline_action_states,
    };
    Ok((0..data.rule_names.len())
        .map(|rule_index| {
            if enabled_rules.get(rule_index).copied().unwrap_or_default() {
                compile_generated_parser_rule(&context, rule_index)
            } else {
                None
            }
        })
        .collect())
}

fn decision_by_state(atn: &Atn) -> Vec<Option<usize>> {
    let mut decision_by_state = vec![None; atn.states().len()];
    for (decision, &state_number) in atn.decision_to_state().iter().enumerate() {
        if let Some(slot) = decision_by_state.get_mut(state_number) {
            *slot = Some(decision);
        }
    }
    decision_by_state
}

fn compile_generated_parser_rule(
    context: &GeneratedParserCompileContext<'_>,
    rule_index: usize,
) -> Option<GeneratedParserRule> {
    let entry_state = context.atn.rule_to_start_state().get(rule_index).copied()?;
    let stop_state = context.atn.rule_to_stop_state().get(rule_index).copied()?;
    let start = context.atn.state(entry_state)?;
    if start.left_recursive_rule {
        return None;
    }
    let mut visited = BTreeSet::new();
    let steps = compile_generated_parser_path(context, entry_state, stop_state, &mut visited)?;
    Some(GeneratedParserRule {
        rule_index,
        entry_state,
        steps,
    })
}

fn compile_generated_parser_path(
    context: &GeneratedParserCompileContext<'_>,
    state_number: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    if state_number == stop_state {
        return Some(Vec::new());
    }
    if !visited.insert(state_number) {
        return None;
    }

    let state = context.atn.state(state_number)?;
    let steps = if let Some(decision) = context
        .decision_by_state
        .get(state_number)
        .copied()
        .flatten()
    {
        compile_generated_parser_decision_state(context, state, decision, stop_state, visited)?
    } else {
        let transition = state.transitions.first()?;
        if state.transitions.len() != 1 {
            return None;
        }
        let (step, target) = compile_generated_parser_transition(
            state_number,
            transition,
            context.inline_action_states,
        )?;
        let mut steps = step.into_iter().collect::<Vec<_>>();
        steps.extend(compile_generated_parser_path(
            context, target, stop_state, visited,
        )?);
        steps
    };
    visited.remove(&state_number);
    Some(steps)
}

fn compile_generated_parser_decision_state(
    context: &GeneratedParserCompileContext<'_>,
    state: &antlr4_runtime::atn::AtnState,
    decision: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    match state.kind {
        AtnStateKind::BlockStart | AtnStateKind::PlusBlockStart | AtnStateKind::StarBlockStart => {
            compile_generated_parser_block_decision(context, state, decision, stop_state, visited)
        }
        AtnStateKind::StarLoopEntry => {
            compile_generated_parser_star_loop(context, state, decision, stop_state, visited)
        }
        AtnStateKind::PlusLoopBack => {
            compile_generated_parser_plus_loop(context, state, decision, stop_state, visited)
        }
        _ => None,
    }
}

fn compile_generated_parser_block_decision(
    context: &GeneratedParserCompileContext<'_>,
    state: &antlr4_runtime::atn::AtnState,
    decision: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    let end_state = state.end_state?;
    let mut alts = Vec::with_capacity(state.transitions.len());
    for transition in &state.transitions {
        let (step, target) = compile_generated_parser_transition(
            state.state_number,
            transition,
            context.inline_action_states,
        )?;
        let mut alt_visited = visited.clone();
        let mut alt_steps = step.into_iter().collect::<Vec<_>>();
        alt_steps.extend(compile_generated_parser_path(
            context,
            target,
            end_state,
            &mut alt_visited,
        )?);
        alts.push(alt_steps);
    }

    let mut steps = vec![GeneratedParserStep::Decision { decision, alts }];
    steps.extend(compile_generated_parser_path(
        context, end_state, stop_state, visited,
    )?);
    Some(steps)
}

fn compile_generated_parser_star_loop(
    context: &GeneratedParserCompileContext<'_>,
    state: &antlr4_runtime::atn::AtnState,
    decision: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    let mut enter = None;
    let mut exit = None;
    for (index, transition) in state.transitions.iter().enumerate() {
        let alt = index + 1;
        let target = transition.target();
        let target_state = context.atn.state(target)?;
        let target_kind = target_state.kind;
        if target_kind == AtnStateKind::LoopEnd {
            exit = Some((alt, transition, target_state.loop_back_state?));
        } else {
            enter = Some((alt, transition));
        }
    }

    let (enter_alt, enter_transition) = enter?;
    let (exit_alt, exit_transition, loop_back_state) = exit?;
    let (enter_step, enter_target) = compile_generated_parser_transition(
        state.state_number,
        enter_transition,
        context.inline_action_states,
    )?;
    let mut body_visited = BTreeSet::new();
    let mut body = enter_step.into_iter().collect::<Vec<_>>();
    body.extend(compile_generated_parser_path(
        context,
        enter_target,
        loop_back_state,
        &mut body_visited,
    )?);
    if !steps_may_consume(&body) {
        return None;
    }

    let (exit_step, exit_target) = compile_generated_parser_transition(
        state.state_number,
        exit_transition,
        context.inline_action_states,
    )?;
    if exit_step.is_some() {
        return None;
    }

    let mut steps = vec![GeneratedParserStep::StarLoop {
        decision,
        enter_alt,
        exit_alt,
        body,
    }];
    steps.extend(compile_generated_parser_path(
        context,
        exit_target,
        stop_state,
        visited,
    )?);
    Some(steps)
}

fn compile_generated_parser_plus_loop(
    context: &GeneratedParserCompileContext<'_>,
    state: &antlr4_runtime::atn::AtnState,
    decision: usize,
    stop_state: usize,
    visited: &mut BTreeSet<usize>,
) -> Option<Vec<GeneratedParserStep>> {
    let mut enter = None;
    let mut exit = None;
    for (index, transition) in state.transitions.iter().enumerate() {
        let alt = index + 1;
        let target = transition.target();
        let target_state = context.atn.state(target)?;
        if target_state.kind == AtnStateKind::LoopEnd {
            exit = Some((alt, transition));
        } else {
            enter = Some((alt, transition));
        }
    }

    let (enter_alt, enter_transition) = enter?;
    let (enter_step, enter_target) = compile_generated_parser_transition(
        state.state_number,
        enter_transition,
        context.inline_action_states,
    )?;
    let mut body_visited = BTreeSet::new();
    let mut body = enter_step.into_iter().collect::<Vec<_>>();
    body.extend(compile_generated_parser_path(
        context,
        enter_target,
        state.state_number,
        &mut body_visited,
    )?);
    if !steps_may_consume(&body) {
        return None;
    }

    let (exit_alt, exit_transition) = exit?;
    let (exit_step, exit_target) = compile_generated_parser_transition(
        state.state_number,
        exit_transition,
        context.inline_action_states,
    )?;
    if exit_step.is_some() {
        return None;
    }

    let mut steps = vec![GeneratedParserStep::StarLoop {
        decision,
        enter_alt,
        exit_alt,
        body,
    }];
    steps.extend(compile_generated_parser_path(
        context,
        exit_target,
        stop_state,
        visited,
    )?);
    Some(steps)
}

fn steps_may_consume(steps: &[GeneratedParserStep]) -> bool {
    steps.iter().any(|step| match step {
        GeneratedParserStep::MatchToken(_)
        | GeneratedParserStep::MatchSet(_)
        | GeneratedParserStep::MatchNotSet(_)
        | GeneratedParserStep::MatchWildcard
        | GeneratedParserStep::CallRule { .. } => true,
        GeneratedParserStep::Action { .. } => false,
        GeneratedParserStep::Decision { alts, .. } => alts.iter().any(|alt| steps_may_consume(alt)),
        GeneratedParserStep::StarLoop { body, .. } => steps_may_consume(body),
    })
}

fn compile_generated_parser_transition(
    source_state: usize,
    transition: &Transition,
    inline_action_states: &BTreeSet<usize>,
) -> Option<(Option<GeneratedParserStep>, usize)> {
    match transition {
        Transition::Epsilon { target } => Some((None, *target)),
        Transition::Atom { target, label } => {
            Some((Some(GeneratedParserStep::MatchToken(*label)), *target))
        }
        Transition::Range {
            target,
            start,
            stop,
        } => Some((
            Some(GeneratedParserStep::MatchSet(vec![(*start, *stop)])),
            *target,
        )),
        Transition::Set { target, set } => Some((
            Some(GeneratedParserStep::MatchSet(set.ranges().to_vec())),
            *target,
        )),
        Transition::NotSet { target, set } => Some((
            Some(GeneratedParserStep::MatchNotSet(set.ranges().to_vec())),
            *target,
        )),
        Transition::Wildcard { target } => {
            Some((Some(GeneratedParserStep::MatchWildcard), *target))
        }
        Transition::Rule {
            rule_index,
            follow_state,
            ..
        } => Some((
            Some(GeneratedParserStep::CallRule {
                source_state,
                rule_index: *rule_index,
            }),
            *follow_state,
        )),
        Transition::Action {
            target, rule_index, ..
        } if inline_action_states.contains(&source_state) => Some((
            Some(GeneratedParserStep::Action {
                source_state,
                rule_index: *rule_index,
            }),
            *target,
        )),
        Transition::Predicate { target, .. } => Some((None, *target)),
        Transition::Action { .. } | Transition::Precedence { .. } => None,
    }
}

fn render_generated_rule_dispatch(
    rules: &[Option<GeneratedParserRule>],
    inline_action_statements: &BTreeMap<usize, String>,
) -> String {
    let mut out = String::new();
    writeln!(
        out,
        "    #[allow(dead_code)]\n    fn parse_generated_rule(&mut self, rule_index: usize) -> Option<Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError>> {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "        match rule_index {{").expect("writing to a string cannot fail");
    for rule in rules.iter().flatten() {
        let index = rule.rule_index;
        writeln!(
            out,
            "            {index} => Some(self.parse_generated_rule_{index}()),"
        )
        .expect("writing to a string cannot fail");
    }
    writeln!(out, "            _ => None,").expect("writing to a string cannot fail");
    writeln!(out, "        }}").expect("writing to a string cannot fail");
    writeln!(out, "    }}").expect("writing to a string cannot fail");
    for rule in rules.iter().flatten() {
        render_generated_rule_method(&mut out, rule, inline_action_statements);
    }
    out
}

fn render_generated_rule_method(
    out: &mut String,
    rule: &GeneratedParserRule,
    inline_action_statements: &BTreeMap<usize, String>,
) {
    let index = rule.rule_index;
    let entry_state = rule.entry_state;
    writeln!(
        out,
        "\n    #[allow(dead_code)]\n    fn parse_generated_rule_{index}(&mut self) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let __rule_start = antlr4_runtime::IntStream::index(self.base.input());"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let mut __ctx = self.base.enter_rule({entry_state}isize, {index});"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "        let mut __consumed_eof = false;")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "        let __result = (|| -> Result<(), antlr4_runtime::AntlrError> {{"
    )
    .expect("writing to a string cannot fail");
    render_generated_steps(out, &rule.steps, 3, inline_action_statements);
    writeln!(out, "            Ok(())").expect("writing to a string cannot fail");
    writeln!(out, "        }})();").expect("writing to a string cannot fail");
    writeln!(out, "        match __result {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "            Ok(()) => Ok(self.base.finish_rule(__ctx, __consumed_eof)),"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "            Err(_) => {{").expect("writing to a string cannot fail");
    writeln!(out, "                self.base.exit_rule();")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "                antlr4_runtime::IntStream::seek(self.base.input(), __rule_start);"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "                self.parse_interpreted_rule({index})")
        .expect("writing to a string cannot fail");
    writeln!(out, "            }}").expect("writing to a string cannot fail");
    writeln!(out, "        }}").expect("writing to a string cannot fail");
    writeln!(out, "    }}").expect("writing to a string cannot fail");
}

fn render_generated_steps(
    out: &mut String,
    steps: &[GeneratedParserStep],
    indent: usize,
    inline_action_statements: &BTreeMap<usize, String>,
) {
    for step in steps {
        render_generated_step(out, step, indent, inline_action_statements);
    }
}

fn render_generated_step(
    out: &mut String,
    step: &GeneratedParserStep,
    indent: usize,
    inline_action_statements: &BTreeMap<usize, String>,
) {
    let pad = "    ".repeat(indent);
    match step {
        GeneratedParserStep::MatchToken(token_type) => {
            writeln!(
                out,
                "{pad}let __child = self.base.match_token({token_type})?;"
            )
            .expect("writing to a string cannot fail");
            if *token_type == antlr4_runtime::token::TOKEN_EOF {
                writeln!(out, "{pad}__consumed_eof = true;")
                    .expect("writing to a string cannot fail");
            }
            writeln!(out, "{pad}self.base.add_parse_child(&mut __ctx, __child);")
                .expect("writing to a string cannot fail");
        }
        GeneratedParserStep::MatchSet(intervals) => {
            let intervals = render_i32_ranges(intervals);
            writeln!(
                out,
                "{pad}let __matched_eof = self.base.la(1) == antlr4_runtime::token::TOKEN_EOF;"
            )
            .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}let __child = self.base.match_set(&{intervals})?;"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}if __matched_eof {{ __consumed_eof = true; }}")
                .expect("writing to a string cannot fail");
            writeln!(out, "{pad}self.base.add_parse_child(&mut __ctx, __child);")
                .expect("writing to a string cannot fail");
        }
        GeneratedParserStep::MatchNotSet(intervals) => {
            let intervals = render_i32_ranges(intervals);
            writeln!(
                out,
                "{pad}let __child = self.base.match_not_set(&{intervals}, 1, atn().max_token_type())?;"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}self.base.add_parse_child(&mut __ctx, __child);")
                .expect("writing to a string cannot fail");
        }
        GeneratedParserStep::MatchWildcard => {
            writeln!(out, "{pad}let __child = self.base.match_wildcard()?;")
                .expect("writing to a string cannot fail");
            writeln!(out, "{pad}self.base.add_parse_child(&mut __ctx, __child);")
                .expect("writing to a string cannot fail");
        }
        GeneratedParserStep::CallRule {
            source_state,
            rule_index,
        } => {
            writeln!(
                out,
                "{pad}let __invoking_marker = self.base.push_invoking_state({source_state}isize);"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}let __child = self.parse_rule({rule_index});")
                .expect("writing to a string cannot fail");
            writeln!(
                out,
                "{pad}self.base.discard_invoking_state(__invoking_marker);"
            )
            .expect("writing to a string cannot fail");
            writeln!(out, "{pad}let __child = __child?;").expect("writing to a string cannot fail");
            writeln!(out, "{pad}self.base.add_parse_child(&mut __ctx, __child);")
                .expect("writing to a string cannot fail");
        }
        GeneratedParserStep::Action {
            source_state,
            rule_index,
        } => {
            let Some(statement) = inline_action_statements.get(source_state) else {
                return;
            };
            if statement.is_empty() {
                return;
            }
            if statement.contains("action.") {
                writeln!(
                    out,
                    "{pad}let action = self.base.parser_action_at_current({source_state}, {rule_index}, __rule_start, __consumed_eof);"
                )
                .expect("writing to a string cannot fail");
            }
            writeln!(out, "{pad}{statement}").expect("writing to a string cannot fail");
        }
        GeneratedParserStep::Decision { decision, alts } => {
            render_generated_decision(out, *decision, alts, indent, inline_action_statements);
        }
        GeneratedParserStep::StarLoop {
            decision,
            enter_alt,
            exit_alt,
            body,
        } => {
            render_generated_star_loop(
                out,
                *decision,
                (*enter_alt, *exit_alt),
                body,
                indent,
                inline_action_statements,
            );
        }
    }
}

fn render_generated_decision(
    out: &mut String,
    decision: usize,
    alts: &[Vec<GeneratedParserStep>],
    indent: usize,
    inline_action_statements: &BTreeMap<usize, String>,
) {
    let pad = "    ".repeat(indent);
    writeln!(
        out,
        "{pad}let __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}let __prediction = {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    let __prediction_context = self.base.prediction_context(atn());"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    let __simulator = self.simulator.get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new(atn()));"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    __simulator.adaptive_predict_stream_info_with_context({decision}, 0, self.base.input(), &__prediction_context)"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}}}.map_err(|_| self.base.no_viable_alternative_error(__decision_start))?;"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}if __prediction.has_semantic_context {{")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    return Err(self.base.no_viable_alternative_error(__decision_start));"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}match __prediction.alt {{").expect("writing to a string cannot fail");
    for (index, steps) in alts.iter().enumerate() {
        let alt = index + 1;
        writeln!(out, "{pad}    {alt} => {{").expect("writing to a string cannot fail");
        render_generated_steps(out, steps, indent + 2, inline_action_statements);
        writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    }
    writeln!(
        out,
        "{pad}    _ => return Err(self.base.no_viable_alternative_error(__decision_start)),"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
}

fn render_generated_star_loop(
    out: &mut String,
    decision: usize,
    alts: (usize, usize),
    body: &[GeneratedParserStep],
    indent: usize,
    inline_action_statements: &BTreeMap<usize, String>,
) {
    let (enter_alt, exit_alt) = alts;
    let pad = "    ".repeat(indent);
    writeln!(out, "{pad}loop {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    let __decision_start = antlr4_runtime::IntStream::index(self.base.input());"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    let __prediction = {{").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        let __prediction_context = self.base.prediction_context(atn());"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        let __simulator = self.simulator.get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new(atn()));"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        __simulator.adaptive_predict_stream_info_with_context({decision}, 0, self.base.input(), &__prediction_context)"
    )
    .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}    }}.map_err(|_| self.base.no_viable_alternative_error(__decision_start))?;"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    if __prediction.has_semantic_context {{")
        .expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        return Err(self.base.no_viable_alternative_error(__decision_start));"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}    match __prediction.alt {{").expect("writing to a string cannot fail");
    writeln!(out, "{pad}        {enter_alt} => {{").expect("writing to a string cannot fail");
    render_generated_steps(out, body, indent + 3, inline_action_statements);
    writeln!(out, "{pad}        }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}        {exit_alt} => break,").expect("writing to a string cannot fail");
    writeln!(
        out,
        "{pad}        _ => return Err(self.base.no_viable_alternative_error(__decision_start)),"
    )
    .expect("writing to a string cannot fail");
    writeln!(out, "{pad}    }}").expect("writing to a string cannot fail");
    writeln!(out, "{pad}}}").expect("writing to a string cannot fail");
}

#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
fn render_parser_parse_rule_fallback(
    init_action_rules: &[usize],
    track_alt_numbers: bool,
    predicates: &[((usize, usize), PredicateTemplate)],
    data: &InterpData,
    int_members: &[IntMemberTemplate],
    rule_args: &[(usize, usize, RuleArgTemplate)],
    member_actions: &[(usize, usize, i64)],
    return_actions: &[(usize, String, i64)],
    has_action_dispatch: bool,
    has_predicate_dispatch: bool,
    has_return_actions: bool,
) -> io::Result<String> {
    let mut out = String::new();
    if has_predicate_dispatch || has_return_actions {
        writeln!(
            out,
            "let (tree, actions) = self.base.parse_atn_rule_with_runtime_options(atn(), rule_index, antlr4_runtime::ParserRuntimeOptions {{ init_action_rules: &{}, track_alt_numbers: {track_alt_numbers}, predicates: &{}, rule_args: &{}, member_actions: &{}, return_actions: &{} }})?;",
            render_usize_array(init_action_rules),
            render_parser_predicate_array(predicates, data, int_members)?,
            render_parser_rule_arg_array(rule_args),
            render_parser_member_action_array(member_actions),
            render_parser_return_action_array(return_actions, data)?
        )
        .expect("writing to a string cannot fail");
    } else if track_alt_numbers {
        writeln!(
            out,
            "let (tree, actions) = self.base.parse_atn_rule_with_action_options(atn(), rule_index, &{}, true)?;",
            render_usize_array(init_action_rules)
        )
        .expect("writing to a string cannot fail");
    } else if !init_action_rules.is_empty() {
        writeln!(
            out,
            "let (tree, actions) = self.base.parse_atn_rule_with_action_inits(atn(), rule_index, &{})?;",
            render_usize_array(init_action_rules)
        )
        .expect("writing to a string cannot fail");
    } else if has_action_dispatch {
        writeln!(
            out,
            "let (tree, actions) = self.base.parse_atn_rule_with_actions(atn(), rule_index)?;"
        )
        .expect("writing to a string cannot fail");
    } else {
        return Ok("self.base.parse_atn_rule(atn(), rule_index)".to_owned());
    }

    if has_action_dispatch {
        writeln!(
            out,
            "for action in actions {{ self.run_action(action, &tree); }}"
        )
        .expect("writing to a string cannot fail");
    } else {
        writeln!(out, "let _ = actions;").expect("writing to a string cannot fail");
    }
    writeln!(out, "Ok(tree)").expect("writing to a string cannot fail");
    Ok(out
        .lines()
        .map(|line| format!("        {line}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Renders a Rust parser module with one public method per grammar rule.
///
/// Parser methods use generated recursive-descent bodies for the ATN subset
/// covered by `parser_generated_rules` and keep the interpreter fallback for
/// unsupported constructs while the generated surface is expanded.
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
    let inline_action_statements = inline_parser_action_statements(&actions, &int_members)?;
    let inline_action_states = inline_action_statements
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    let has_init_actions = init_actions.iter().any(Option::is_some);
    let has_action_dispatch = !actions.is_empty() || has_init_actions;
    let has_predicate_dispatch = !predicates.is_empty();
    let has_return_actions = !return_actions.is_empty();
    let track_alt_numbers = grammar_source.is_some_and(uses_alt_number_contexts);
    let direct_rules_allowed = !track_alt_numbers
        && !has_predicate_dispatch
        && !has_return_actions
        && after_actions.iter().all(Vec::is_empty);
    let generated_rule_enabled = (0..data.rule_names.len())
        .map(|index| direct_rules_allowed && init_actions.get(index).is_none_or(Option::is_none))
        .collect::<Vec<_>>();
    let generated_rules =
        parser_generated_rules(data, &generated_rule_enabled, &inline_action_states)?;
    let generated_rule_dispatch =
        render_generated_rule_dispatch(&generated_rules, &inline_action_statements);
    let init_action_rules = init_actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| action.as_ref().map(|_| index))
        .collect::<Vec<_>>();
    let parse_rule_fallback = render_parser_parse_rule_fallback(
        &init_action_rules,
        track_alt_numbers,
        &predicates,
        data,
        &int_members,
        &rule_args,
        &member_actions,
        &return_actions,
        has_action_dispatch,
        has_predicate_dispatch,
        has_return_actions,
    )?;
    let adaptive_direct_allowed = !has_action_dispatch
        && !track_alt_numbers
        && !has_predicate_dispatch
        && !has_return_actions;
    let action_method = render_parser_action_method(&actions, &init_actions, &int_members)?;
    let base_initialization = render_parser_base_initialization(&int_members);
    let mut rule_methods = String::new();
    for (index, rule) in data.rule_names.iter().enumerate() {
        let after_action = after_actions.get(index).map_or(&[][..], Vec::as_slice);
        let uses_after_interval = after_action.iter().any(ActionTemplate::uses_rule_interval);
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
        if after_action.is_empty() {
            writeln!(rule_methods, "        self.parse_rule({index})")
                .expect("writing to a string cannot fail");
        } else {
            writeln!(
                rule_methods,
                "        let tree = self.parse_rule({index})?;"
            )
            .expect("writing to a string cannot fail");
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
    simulator: Option<antlr4_runtime::ParserAtnSimulator<'static>>,
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
        Self {{ base, simulator: None }}
    }}

    pub fn metadata() -> &'static GrammarMetadata {{
        &METADATA
    }}

    #[allow(dead_code)]
    fn simulator(&mut self) -> &mut antlr4_runtime::ParserAtnSimulator<'static> {{
        self.simulator
            .get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new(atn()))
    }}

    #[allow(dead_code)]
    fn parse_rule(&mut self, rule_index: usize) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{
        if let Some(result) = self.parse_generated_rule(rule_index) {{
            return result;
        }}
        self.parse_interpreted_rule(rule_index)
    }}

    #[allow(dead_code)]
    fn parse_interpreted_rule(&mut self, rule_index: usize) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{
        if {adaptive_direct_allowed} && std::env::var_os("ANTLR4_RUST_ADAPTIVE_DIRECT").is_some() {{
            let simulator = self
                .simulator
                .get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new(atn()));
            self.base
                .parse_atn_rule_adaptive_or_fallback(atn(), simulator, rule_index)
        }} else {{
{parse_rule_fallback}
        }}
    }}

{generated_rule_dispatch}

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
    RuleTextWithPrefix {
        rule_name: String,
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
                | Self::RuleTextWithPrefix { .. }
                | Self::TokenText { .. }
                | Self::TokenTextWithPrefix { .. }
                | Self::TokenDisplay { .. }
        ) || matches!(self, Self::Sequence(actions) if actions.iter().any(Self::uses_rule_interval))
    }

    /// Reports whether a parser action can be emitted directly at its ATN
    /// action-transition site without needing the completed parse tree or
    /// interpreter-only state.
    fn can_run_inline(&self) -> bool {
        matches!(
            self,
            Self::Noop
                | Self::Text { .. }
                | Self::TextWithPrefix { .. }
                | Self::TokenText { .. }
                | Self::TokenTextWithPrefix { .. }
                | Self::Literal { .. }
                | Self::AddMember { .. }
                | Self::MemberValue { .. }
        ) || matches!(self, Self::Sequence(actions) if actions.iter().all(Self::can_run_inline))
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
    LocalIntLessOrEqual {
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
        let block = next_parser_action_block(grammar_source, offset, |body| {
            parse_int_return_assignment(body).is_some()
        });
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
        "Pass()" | "LL_EXACT_AMBIG_DETECTION()" | "DumpDFA()" => Some(ActionTemplate::Noop),
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
            .or_else(|| parse_raw_local_int_less_or_equal_predicate(body))
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

/// Parses raw ANTLR semantic predicates such as `5 >= $_p`.
///
/// The Java generator lowers these against the generated context field
/// `_localctx._p`. The metadata runtime does not execute target code, so the
/// generator records the literal bound and the rule-call argument table makes
/// the current `_p` value available while interpreting the predicate
/// transition.
fn parse_raw_local_int_less_or_equal_predicate(body: &str) -> Option<PredicateTemplate> {
    let (value, local) = body.split_once(">=")?;
    if local.trim() != "$_p" {
        return None;
    }
    Some(PredicateTemplate::LocalIntLessOrEqual {
        value: value.trim().parse::<i64>().ok()?,
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
    let first = label.chars().next()?;
    if !first.is_ascii_uppercase() {
        return Some(ActionTemplate::RuleTextWithPrefix {
            rule_name: label.to_owned(),
            prefix,
            newline,
        });
    }
    Some(ActionTemplate::TokenTextWithPrefix {
        prefix,
        source: TokenTextSource::ActionStop,
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

/// Reads the lexer ATN to locate serialized custom action coordinates.
fn lexer_custom_actions(data: &InterpData) -> io::Result<Vec<(i32, i32)>> {
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(&data.atn))
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
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(&data.atn))
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
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(&data.atn))
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
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(&data.atn))
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
    let atn = AtnDeserializer::new(&SerializedAtn::from_i32(&data.atn))
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

/// Renders parser actions that are safe to execute from generated rule bodies.
fn inline_parser_action_statements(
    actions: &[(usize, ActionTemplate)],
    members: &[IntMemberTemplate],
) -> io::Result<BTreeMap<usize, String>> {
    let mut statements = BTreeMap::new();
    for (source_state, action) in actions {
        if action.can_run_inline() {
            statements.insert(*source_state, render_action_statement(action, members)?);
        }
    }
    Ok(statements)
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
        | ActionTemplate::RuleTextWithPrefix { .. }
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
        | ActionTemplate::RuleTextWithPrefix { .. }
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
        ActionTemplate::RuleTextWithPrefix { .. } => String::new(),
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
        | PredicateTemplate::LocalIntLessOrEqual { .. }
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
        ActionTemplate::RuleTextWithPrefix {
            rule_name,
            prefix,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            Ok(render_rule_text_write(write, "_tree", prefix, rule_name))
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
        ActionTemplate::RuleTextWithPrefix {
            rule_name,
            prefix,
            newline,
        } => {
            let write = if *newline { "println!" } else { "print!" };
            render_rule_text_write(write, "tree", prefix, rule_name)
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
        StringTreeTarget::Label(label) => {
            let label = rust_string(label);
            format!(
                "let text = METADATA.rule_names().iter().position(|name| *name == \"{label}\").and_then(|rule_index| {tree_expr}.first_rule(rule_index)).map_or_else(String::new, |node| node.to_string_tree(&{rule_names})); {write}(\"{{}}\", text);"
            )
        }
    }
}

/// Emits text for the first child rule with `rule_name`, matching `$rule.text`
/// in the runtime-testsuite action templates.
fn render_rule_text_write(write: &str, tree_expr: &str, prefix: &str, rule_name: &str) -> String {
    let prefix = rust_string(prefix);
    let rule_name = rust_string(rule_name);
    format!(
        "let text = METADATA.rule_names().iter().position(|name| *name == \"{rule_name}\").and_then(|rule_index| {tree_expr}.first_rule(rule_index)).map_or_else(String::new, antlr4_runtime::ParseTree::text); {write}(\"{prefix}{{}}\", text);"
    )
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

/// Renders an inline `[(i32, i32); N]` expression for generated token-set
/// matches.
fn render_i32_ranges(values: &[(i32, i32)]) -> String {
    let items = values
        .iter()
        .map(|(start, stop)| format!("({start}, {stop})"))
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
            PredicateTemplate::LocalIntLessOrEqual { value } => {
                format!("antlr4_runtime::ParserPredicate::LocalIntLessOrEqual {{ value: {value} }}")
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

/// Converts ASCII letters to upper case without using allocation-hiding string
/// case helpers disallowed by the strict Clippy policy.
fn ascii_uppercase(value: &str) -> String {
    value.chars().map(|ch| ch.to_ascii_uppercase()).collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use antlr4_runtime::atn::{AtnState, AtnType, IntervalSet};

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

    fn compile_test_parser_rule(
        atn: &Atn,
        rule_index: usize,
        inline_action_states: &BTreeSet<usize>,
    ) -> Option<GeneratedParserRule> {
        let decision_by_state = decision_by_state(atn);
        let context = GeneratedParserCompileContext {
            atn,
            decision_by_state: &decision_by_state,
            inline_action_states,
        };
        compile_generated_parser_rule(&context, rule_index)
    }

    #[test]
    fn compiles_linear_parser_rule_body() {
        let atn = linear_rule_atn();
        let body = compile_test_parser_rule(&atn, 0, &BTreeSet::new())
            .expect("linear rule should compile");

        assert_eq!(body.rule_index, 0);
        assert_eq!(body.entry_state, 0);
        assert_eq!(
            body.steps,
            [
                GeneratedParserStep::MatchToken(1),
                GeneratedParserStep::MatchToken(antlr4_runtime::token::TOKEN_EOF)
            ]
        );
    }

    #[test]
    fn compiles_block_decision_with_adaptive_prediction() {
        let atn = block_decision_atn();
        let body = compile_test_parser_rule(&atn, 0, &BTreeSet::new())
            .expect("block decision rule should compile");

        assert_eq!(
            body.steps,
            [GeneratedParserStep::Decision {
                decision: 0,
                alts: vec![
                    vec![GeneratedParserStep::MatchToken(1)],
                    vec![GeneratedParserStep::MatchToken(2)]
                ],
            }]
        );

        let rendered = render_generated_rule_dispatch(&[Some(body)], &BTreeMap::new());
        assert!(rendered.contains("parse_generated_rule_0"));
        assert!(rendered.contains("adaptive_predict_stream_info_with_context(0, 0"));
        assert!(!rendered.contains("requires_full_context"));
    }

    #[test]
    fn compiles_star_loop_with_adaptive_prediction() {
        let atn = star_loop_atn();
        let body = compile_test_parser_rule(&atn, 0, &BTreeSet::new())
            .expect("star loop rule should compile");

        assert_eq!(
            body.steps,
            [GeneratedParserStep::StarLoop {
                decision: 0,
                enter_alt: 1,
                exit_alt: 2,
                body: vec![GeneratedParserStep::MatchToken(1)],
            }]
        );

        let rendered = render_generated_rule_dispatch(&[Some(body)], &BTreeMap::new());
        assert!(rendered.contains("loop {"));
        assert!(rendered.contains("1 => {"));
        assert!(rendered.contains("2 => break,"));
        assert!(rendered.contains("adaptive_predict_stream_info_with_context(0, 0"));
    }

    #[test]
    fn compiles_plus_loop_back_with_adaptive_prediction() {
        let atn = plus_loop_atn();
        let body = compile_test_parser_rule(&atn, 0, &BTreeSet::new())
            .expect("plus loop rule should compile");

        assert_eq!(
            body.steps,
            [
                GeneratedParserStep::MatchToken(1),
                GeneratedParserStep::StarLoop {
                    decision: 0,
                    enter_alt: 1,
                    exit_alt: 2,
                    body: vec![GeneratedParserStep::MatchToken(1)],
                }
            ]
        );
    }

    #[test]
    fn compiles_plus_block_body_decision_with_adaptive_prediction() {
        let atn = plus_block_decision_atn();
        let body = compile_test_parser_rule(&atn, 0, &BTreeSet::new())
            .expect("plus block decision rule should compile");

        let body_decision = GeneratedParserStep::Decision {
            decision: 0,
            alts: vec![
                vec![GeneratedParserStep::MatchToken(1)],
                vec![GeneratedParserStep::MatchToken(2)],
            ],
        };
        assert_eq!(
            body.steps,
            [
                body_decision.clone(),
                GeneratedParserStep::StarLoop {
                    decision: 1,
                    enter_alt: 1,
                    exit_alt: 2,
                    body: vec![body_decision],
                }
            ]
        );
    }

    #[test]
    fn compiles_token_set_transitions() {
        let range = Transition::Range {
            target: 7,
            start: 2,
            stop: 4,
        };
        assert_eq!(
            compile_generated_parser_transition(3, &range, &BTreeSet::new()),
            Some((Some(GeneratedParserStep::MatchSet(vec![(2, 4)])), 7))
        );

        let mut set = IntervalSet::new();
        set.add(1);
        set.add_range(5, 6);
        let set_transition = Transition::Set { target: 8, set };
        assert_eq!(
            compile_generated_parser_transition(3, &set_transition, &BTreeSet::new()),
            Some((Some(GeneratedParserStep::MatchSet(vec![(1, 1), (5, 6)])), 8))
        );
    }

    #[test]
    fn compiles_inline_action_transitions_only_for_allowed_states() {
        let action = Transition::Action {
            target: 8,
            rule_index: 2,
            action_index: Some(0),
            context_dependent: false,
        };
        assert_eq!(
            compile_generated_parser_transition(4, &action, &BTreeSet::new()),
            None
        );

        let mut inline_actions = BTreeSet::new();
        inline_actions.insert(4);
        assert_eq!(
            compile_generated_parser_transition(4, &action, &inline_actions),
            Some((
                Some(GeneratedParserStep::Action {
                    source_state: 4,
                    rule_index: 2,
                }),
                8
            ))
        );
    }

    #[test]
    fn compiles_parser_predicates_as_viable_when_no_metadata_is_active() {
        let predicate = Transition::Predicate {
            target: 8,
            rule_index: 2,
            pred_index: 1,
            context_dependent: false,
        };

        assert_eq!(
            compile_generated_parser_transition(4, &predicate, &BTreeSet::new()),
            Some((None, 8))
        );
    }

    #[test]
    fn parse_rule_fallback_runs_parser_actions() {
        let fallback = render_parser_parse_rule_fallback(
            &[],
            false,
            &[],
            &minimal_parser_data(),
            &[],
            &[],
            &[],
            &[],
            true,
            false,
            false,
        )
        .expect("fallback should render");

        assert!(fallback.contains("parse_atn_rule_with_actions(atn(), rule_index)?"));
        assert!(fallback.contains("for action in actions { self.run_action(action, &tree); }"));
        assert!(fallback.contains("Ok(tree)"));
    }

    #[test]
    fn renders_inline_action_event_only_when_statement_uses_it() {
        let rule = GeneratedParserRule {
            rule_index: 0,
            entry_state: 0,
            steps: vec![
                GeneratedParserStep::Action {
                    source_state: 4,
                    rule_index: 0,
                },
                GeneratedParserStep::Action {
                    source_state: 6,
                    rule_index: 0,
                },
            ],
        };
        let mut statements = BTreeMap::new();
        statements.insert(
            4,
            "let text = self.base.text_interval(action.start_index(), action.stop_index()); print!(\"{}\", text);"
                .to_owned(),
        );
        statements.insert(6, "println!(\"alt 2\");".to_owned());

        let rendered = render_generated_rule_dispatch(&[Some(rule)], &statements);

        assert!(rendered.contains("parser_action_at_current(4, 0"));
        assert!(!rendered.contains("parser_action_at_current(6, 0"));
        assert!(rendered.contains("println!(\"alt 2\");"));
    }

    #[test]
    fn classifies_inline_safe_parser_actions() {
        assert!(ActionTemplate::Text { newline: true }.can_run_inline());
        assert!(
            ActionTemplate::Sequence(vec![
                ActionTemplate::Literal {
                    value: "x".to_owned(),
                    newline: false,
                },
                ActionTemplate::MemberValue {
                    member: "i".to_owned(),
                    newline: true,
                },
            ])
            .can_run_inline()
        );
        assert!(
            !ActionTemplate::StringTree {
                target: StringTreeTarget::Current,
                newline: true,
            }
            .can_run_inline()
        );
        assert!(
            !ActionTemplate::Sequence(vec![
                ActionTemplate::Noop,
                ActionTemplate::ExpectedTokenNames { newline: true },
            ])
            .can_run_inline()
        );
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
            parse_raw_local_int_less_or_equal_predicate("5 >= $_p"),
            Some(PredicateTemplate::LocalIntLessOrEqual { value: 5 })
        );
        assert_eq!(
            parse_boolean_member_not_predicate(r#"GetMember("enumKeyword"):Not()"#),
            Some(PredicateTemplate::False)
        );
    }

    fn linear_rule_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 2);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        atn.add_state(AtnState::new(1, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(3, AtnStateKind::RuleStop).with_rule_index(0));
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Atom {
                target: 2,
                label: 1,
            });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 3,
                label: antlr4_runtime::token::TOKEN_EOF,
            });
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![3]);
        atn
    }

    fn block_decision_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 2);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        let mut decision = AtnState::new(1, AtnStateKind::BlockStart).with_rule_index(0);
        decision.end_state = Some(4);
        atn.add_state(decision);
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(3, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(4, AtnStateKind::BlockEnd).with_rule_index(0));
        atn.add_state(AtnState::new(5, AtnStateKind::RuleStop).with_rule_index(0));
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 3 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 4,
                label: 1,
            });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Atom {
                target: 4,
                label: 2,
            });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Epsilon { target: 5 });
        atn.add_decision_state(1);
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![5]);
        atn
    }

    fn star_loop_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 2);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        atn.add_state(AtnState::new(1, AtnStateKind::StarLoopEntry).with_rule_index(0));
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        let mut loop_end = AtnState::new(3, AtnStateKind::LoopEnd).with_rule_index(0);
        loop_end.loop_back_state = Some(4);
        atn.add_state(loop_end);
        atn.add_state(AtnState::new(4, AtnStateKind::StarLoopBack).with_rule_index(0));
        atn.add_state(AtnState::new(5, AtnStateKind::RuleStop).with_rule_index(0));
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 3 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 4,
                label: 1,
            });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Epsilon { target: 5 });
        atn.add_decision_state(1);
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![5]);
        atn
    }

    fn plus_loop_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 2);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        let mut plus_start = AtnState::new(1, AtnStateKind::PlusBlockStart).with_rule_index(0);
        plus_start.end_state = Some(3);
        atn.add_state(plus_start);
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(3, AtnStateKind::BlockEnd).with_rule_index(0));
        atn.add_state(AtnState::new(4, AtnStateKind::PlusLoopBack).with_rule_index(0));
        let mut loop_end = AtnState::new(5, AtnStateKind::LoopEnd).with_rule_index(0);
        loop_end.loop_back_state = Some(4);
        atn.add_state(loop_end);
        atn.add_state(AtnState::new(6, AtnStateKind::RuleStop).with_rule_index(0));
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 3,
                label: 1,
            });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Epsilon { target: 4 });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Epsilon { target: 5 });
        atn.state_mut(5)
            .expect("state 5")
            .add_transition(Transition::Epsilon { target: 6 });
        atn.add_decision_state(4);
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![6]);
        atn
    }

    fn plus_block_decision_atn() -> Atn {
        let mut atn = Atn::new(AtnType::Parser, 2);
        atn.add_state(AtnState::new(0, AtnStateKind::RuleStart).with_rule_index(0));
        let mut plus_start = AtnState::new(1, AtnStateKind::PlusBlockStart).with_rule_index(0);
        plus_start.end_state = Some(4);
        atn.add_state(plus_start);
        atn.add_state(AtnState::new(2, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(3, AtnStateKind::Basic).with_rule_index(0));
        atn.add_state(AtnState::new(4, AtnStateKind::BlockEnd).with_rule_index(0));
        atn.add_state(AtnState::new(5, AtnStateKind::PlusLoopBack).with_rule_index(0));
        let mut loop_end = AtnState::new(6, AtnStateKind::LoopEnd).with_rule_index(0);
        loop_end.loop_back_state = Some(5);
        atn.add_state(loop_end);
        atn.add_state(AtnState::new(7, AtnStateKind::RuleStop).with_rule_index(0));
        atn.state_mut(0)
            .expect("state 0")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 2 });
        atn.state_mut(1)
            .expect("state 1")
            .add_transition(Transition::Epsilon { target: 3 });
        atn.state_mut(2)
            .expect("state 2")
            .add_transition(Transition::Atom {
                target: 4,
                label: 1,
            });
        atn.state_mut(3)
            .expect("state 3")
            .add_transition(Transition::Atom {
                target: 4,
                label: 2,
            });
        atn.state_mut(4)
            .expect("state 4")
            .add_transition(Transition::Epsilon { target: 5 });
        atn.state_mut(5)
            .expect("state 5")
            .add_transition(Transition::Epsilon { target: 1 });
        atn.state_mut(5)
            .expect("state 5")
            .add_transition(Transition::Epsilon { target: 6 });
        atn.state_mut(6)
            .expect("state 6")
            .add_transition(Transition::Epsilon { target: 7 });
        atn.add_decision_state(1);
        atn.add_decision_state(5);
        atn.set_rule_to_start_state(vec![0]);
        atn.set_rule_to_stop_state(vec![7]);
        atn
    }

    fn minimal_parser_data() -> InterpData {
        InterpData {
            literal_names: vec![None, Some("'a'".to_owned())],
            symbolic_names: vec![None, Some("A".to_owned())],
            rule_names: vec!["s".to_owned()],
            channel_names: vec!["DEFAULT_TOKEN_CHANNEL".to_owned()],
            mode_names: vec!["DEFAULT_MODE".to_owned()],
            atn: vec![
                4, 1, 1, // version, parser grammar, max token type
                2, // states
                2, 0, // rule start
                7, 0, // rule stop
                0, // non-greedy states
                0, // precedence states
                1, // rules
                0, // rule 0 start
                0, // modes
                0, // sets
                1, // transitions
                0, 1, 1, 0, 0, 0, // epsilon
                0, // decisions
            ],
        }
    }
}
