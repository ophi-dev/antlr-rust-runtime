use std::io;

use crate::parser::parser_public_rule_method_names;
use crate::rust_output::{module_name, replace_all, rust_type_name};

pub(crate) const LEXER_START_RULE: &str = "tokens";
pub(crate) const MAIN_PATH: &str = "__antlr4_rust_testrig/main.rs";

#[derive(Debug)]
pub(crate) struct TestRigLexer {
    grammar_name: String,
}

impl TestRigLexer {
    pub(crate) const fn new(grammar_name: String) -> Self {
        Self { grammar_name }
    }
}

#[derive(Debug)]
pub(crate) struct TestRigParser {
    grammar_name: String,
    rule_names: Vec<String>,
    rule_methods: Vec<String>,
}

impl TestRigParser {
    pub(crate) fn new(grammar_name: String, rule_names: Vec<String>) -> Self {
        let rule_methods = parser_public_rule_method_names(&rule_names);
        Self {
            grammar_name,
            rule_names,
            rule_methods,
        }
    }
}

pub(crate) fn render_test_rig(
    start_rule: &str,
    lexers: &[TestRigLexer],
    parsers: &[TestRigParser],
) -> io::Result<String> {
    if start_rule == LEXER_START_RULE {
        let lexer = select_lexer(lexers, None)?;
        return Ok(render_lexer_runner(lexer));
    }

    let (parser, rule_index) = select_parser(parsers, start_rule)?;
    let lexer = select_lexer(lexers, Some(parser))?;
    let rule_method = parser
        .rule_methods
        .get(rule_index)
        .expect("parser rule names and rendered methods have equal lengths");
    Ok(render_parser_runner(lexer, parser, rule_method))
}

fn select_parser<'a>(
    parsers: &'a [TestRigParser],
    start_rule: &str,
) -> io::Result<(&'a TestRigParser, usize)> {
    let matches = parsers
        .iter()
        .filter_map(|parser| {
            parser
                .rule_names
                .iter()
                .position(|rule| rule == start_rule)
                .map(|index| (parser, index))
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [selected] => Ok(*selected),
        [] => {
            let available = parsers
                .iter()
                .flat_map(|parser| {
                    parser
                        .rule_names
                        .iter()
                        .map(|rule| format!("{}.{}", parser.grammar_name, rule))
                })
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if available.is_empty() {
                "no parser was generated".to_owned()
            } else {
                format!("available rules: {available}")
            };
            Err(invalid_input(format!(
                "test rig start rule `{start_rule}` was not found; {suffix}"
            )))
        }
        _ => {
            let owners = matches
                .iter()
                .map(|(parser, _)| parser.grammar_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(invalid_input(format!(
                "test rig start rule `{start_rule}` is ambiguous across parsers: {owners}"
            )))
        }
    }
}

fn select_lexer<'a>(
    lexers: &'a [TestRigLexer],
    parser: Option<&TestRigParser>,
) -> io::Result<&'a TestRigLexer> {
    match lexers {
        [lexer] => return Ok(lexer),
        [] => {
            return Err(invalid_input(
                "test rig requires a generated lexer; use a combined grammar or pass \
                 --lexer-grammar for a split grammar",
            ));
        }
        _ => {}
    }

    if let Some(parser) = parser {
        let parser_stem = parser
            .grammar_name
            .strip_suffix("Parser")
            .unwrap_or(&parser.grammar_name);
        let matches = lexers
            .iter()
            .filter(|lexer| {
                lexer
                    .grammar_name
                    .strip_suffix("Lexer")
                    .unwrap_or(&lexer.grammar_name)
                    == parser_stem
            })
            .collect::<Vec<_>>();
        if let [lexer] = matches.as_slice() {
            return Ok(*lexer);
        }
    }

    let names = lexers
        .iter()
        .map(|lexer| lexer.grammar_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(invalid_input(format!(
        "test rig cannot choose a lexer from multiple generated lexers: {names}"
    )))
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn render_lexer_runner(lexer: &TestRigLexer) -> String {
    let mut rendered = assemble_runner(LEXER_RUNNER_TEMPLATE);
    for (placeholder, value) in [
        ("__LEXER_MODULE__", module_name(&lexer.grammar_name)),
        ("__LEXER_TYPE__", rust_type_name(&lexer.grammar_name)),
    ] {
        rendered = replace_all(&rendered, placeholder, &value);
    }
    rendered
}

fn render_parser_runner(lexer: &TestRigLexer, parser: &TestRigParser, rule_method: &str) -> String {
    let mut rendered = assemble_runner(PARSER_RUNNER_TEMPLATE);
    for (placeholder, value) in [
        ("__LEXER_MODULE__", module_name(&lexer.grammar_name)),
        ("__LEXER_TYPE__", rust_type_name(&lexer.grammar_name)),
        ("__PARSER_MODULE__", module_name(&parser.grammar_name)),
        ("__PARSER_TYPE__", rust_type_name(&parser.grammar_name)),
        ("__RULE_METHOD__", rule_method.to_owned()),
    ] {
        rendered = replace_all(&rendered, placeholder, &value);
    }
    rendered
}

const RUNNER_PREAMBLE: &str = r#"use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use antlr4_runtime::{
    CharStream as _, CommonTokenStream, InputStream, IntStream as _,
};
use miette::{
    Context as _, Diagnostic, IntoDiagnostic as _, LabeledSpan, NamedSource, SourceCode,
};

const LEXER_DIAGNOSTIC_CODE: &str = "antlr4_rust_testrig::lexer";

type SharedSource = Arc<NamedSource<Arc<str>>>;

#[allow(dead_code)] // Parser-only flags remain accepted for lexer-only TestRig invocations.
#[derive(Clone, Copy, Debug, Default)]
struct Options {
    tokens: bool,
    tree: bool,
    trace: bool,
    diagnostics: bool,
    sll: bool,
}

fn main() -> miette::Result<ExitCode> {
    Ok(if run()? {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn run() -> miette::Result<bool> {
    let (options, input_files) = options_and_inputs()
        .into_diagnostic()
        .wrap_err("invalid TestRig runner arguments")?;
    if input_files.is_empty() {
        let stdin = io::stdin();
        let input = InputStream::from_reader_with_source_name(stdin.lock(), "<stdin>")
            .into_diagnostic()
            .wrap_err("failed to read standard input")?;
        return process(input, options);
    }

    let show_names = input_files.len() > 1;
    let mut success = true;
    for input_file in input_files {
        let path = PathBuf::from(input_file);
        if show_names {
            eprintln!("{}", path.display());
        }
        match process_file(&path, options) {
            Ok(input_success) => success &= input_success,
            Err(error) => {
                eprintln!("Error: {error:?}");
                success = false;
            }
        }
    }
    Ok(success)
}

fn process_file(path: &std::path::Path, options: Options) -> miette::Result<bool> {
    let file = File::open(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to open input {}", path.display()))?;
    let input =
        InputStream::from_reader_with_source_name(file, path.display().to_string())
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to read input {}", path.display()))?;
    process(input, options)
}

fn options_and_inputs() -> io::Result<(Options, Vec<OsString>)> {
    let mut options = Options::default();
    let mut inputs = Vec::new();
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--tokens") => options.tokens = true,
            Some("--tree") => options.tree = true,
            Some("--trace") => options.trace = true,
            Some("--diagnostics") => options.diagnostics = true,
            Some("--sll") => options.sll = true,
            Some("--") => {
                inputs.extend(arguments);
                break;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unexpected runner argument {}", argument.to_string_lossy()),
                ));
            }
        }
    }
    Ok((options, inputs))
}

#[derive(Debug)]
struct SyntaxDiagnostic {
    code: &'static str,
    message: String,
    source: SharedSource,
    span: Option<Range<usize>>,
}

impl SyntaxDiagnostic {
    fn new(
        code: &'static str,
        source: SharedSource,
        line: usize,
        column: usize,
        span: Option<Range<usize>>,
        message: String,
    ) -> Self {
        let span = span
            .filter(|span| span.start <= span.end && span.end <= source.inner().len())
            .or_else(|| position_span(source.inner(), line, column));
        let message = if span.is_some() {
            message
        } else {
            format!("{}:{line}:{column}: {message}", source.name())
        };
        Self {
            code,
            message,
            source,
            span,
        }
    }
}

impl fmt::Display for SyntaxDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SyntaxDiagnostic {}

impl Diagnostic for SyntaxDiagnostic {
    fn code(&self) -> Option<Box<dyn fmt::Display + '_>> {
        Some(Box::new(self.code))
    }

    fn source_code(&self) -> Option<&dyn SourceCode> {
        Some(self.source.as_ref())
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        self.span.as_ref().map(|span| {
            let label = if span.is_empty() {
                LabeledSpan::at_offset(span.start, "here")
            } else {
                LabeledSpan::underline(span.clone())
            };
            let labels: Box<dyn Iterator<Item = LabeledSpan>> =
                Box::new(std::iter::once(label));
            labels
        })
    }
}

fn diagnostic_source(input: &InputStream) -> SharedSource {
    let contents: Arc<str> = input
        .source_text()
        .map_or_else(|| Arc::from(""), |contents| Arc::from(contents.as_ref()));
    Arc::new(NamedSource::new(input.source_name(), contents))
}

fn position_span(source: &str, line: usize, column: usize) -> Option<Range<usize>> {
    let line_index = line.checked_sub(1)?;
    let mut line_start = 0;
    for _ in 0..line_index {
        line_start += source.get(line_start..)?.find('\n')? + 1;
    }
    let remainder = source.get(line_start..)?;
    let line_length = remainder.find('\n').unwrap_or(remainder.len());
    let line_text = remainder.get(..line_length)?;
    let relative = line_text
        .char_indices()
        .nth(column)
        .map(|(offset, _)| offset)
        .or_else(|| (line_text.chars().count() == column).then_some(line_text.len()))?;
    let length = line_text
        .get(relative..)?
        .chars()
        .next()
        .map_or(0, char::len_utf8);
    let start = line_start + relative;
    Some(start..start + length)
}

fn report_diagnostic(diagnostic: SyntaxDiagnostic) {
    eprintln!("Error: {:?}", miette::Report::new(diagnostic));
}

fn report_lexer_errors<L: antlr4_runtime::TokenSource>(
    tokens: &mut CommonTokenStream<L>,
    source: &SharedSource,
) -> usize {
    tokens.fill();
    let errors = tokens.drain_source_errors();
    let count = errors.len();
    for error in errors {
        report_diagnostic(SyntaxDiagnostic::new(
            LEXER_DIAGNOSTIC_CODE,
            Arc::clone(source),
            error.line,
            error.column,
            error.span,
            error.message,
        ));
    }
    count
}
"#;

const LEXER_RUNNER_TEMPLATE: &str = r#"#![allow(clippy::print_stderr, clippy::print_stdout)]

#[path = "../__LEXER_MODULE__.rs"]
mod generated_lexer;

use generated_lexer::__LEXER_TYPE__;

__RUNNER_PREAMBLE__

fn process(
    input: InputStream,
    options: Options,
) -> miette::Result<bool> {
    let source = diagnostic_source(&input);
    let lexer = __LEXER_TYPE__::new(input);
    let mut tokens = CommonTokenStream::try_new(lexer)
        .into_diagnostic()
        .wrap_err("failed to buffer lexer tokens")?;
    let lexer_errors = report_lexer_errors(&mut tokens, &source);
    if options.tokens {
        for token in tokens.tokens() {
            println!("{token}");
        }
    }
    Ok(lexer_errors == 0)
}
"#;

const PARSER_RUNNER_TEMPLATE: &str = r#"#![allow(clippy::print_stderr, clippy::print_stdout)]

#[path = "../__LEXER_MODULE__.rs"]
mod generated_lexer;
#[path = "../__PARSER_MODULE__.rs"]
mod generated_parser;

use antlr4_runtime::{
    AntlrError, EnterRuleEvent, ErrorListener, ParseListener, Parser as _, PredictionMode,
    Recognizer, SyntaxErrorEvent,
};
use std::sync::Mutex;
use generated_lexer::__LEXER_TYPE__;
use generated_parser::__PARSER_TYPE__;

__RUNNER_PREAMBLE__

const PARSER_DIAGNOSTIC_CODE: &str = "antlr4_rust_testrig::parser";

#[derive(Clone, Debug)]
struct DiagnosticCollector {
    code: &'static str,
    source: SharedSource,
    diagnostics: Arc<Mutex<Vec<SyntaxDiagnostic>>>,
}

impl DiagnosticCollector {
    fn new(code: &'static str, source: SharedSource) -> Self {
        Self {
            code,
            source,
            diagnostics: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn report(&self) {
        let diagnostics = {
            let mut diagnostics = self
                .diagnostics
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *diagnostics)
        };
        for diagnostic in diagnostics {
            report_diagnostic(diagnostic);
        }
    }
}

impl<R> ErrorListener<R> for DiagnosticCollector
where
    R: Recognizer + ?Sized,
{
    fn syntax_error(&mut self, _recognizer: &R, event: &SyntaxErrorEvent<'_>) {
        self.diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(SyntaxDiagnostic::new(
                self.code,
                Arc::clone(&self.source),
                event.line,
                event.column,
                event.span.clone(),
                event.message.to_owned(),
            ));
    }
}

#[derive(Debug)]
struct TraceListener {
    rule_names: &'static [&'static str],
    depth: usize,
}

impl TraceListener {
    const fn new(rule_names: &'static [&'static str]) -> Self {
        Self {
            rule_names,
            depth: 0,
        }
    }

    fn rule_name(&self, rule_index: usize) -> &str {
        self.rule_names
            .get(rule_index)
            .copied()
            .unwrap_or("<unknown>")
    }
}

impl ParseListener for TraceListener {
    fn enter_every_rule(&mut self, event: &EnterRuleEvent<'_>) -> Result<(), AntlrError> {
        let lookahead = event
            .current
            .map_or_else(|| "<EOF>".to_owned(), |token| token.to_string());
        eprintln!(
            "{}enter   {}, LT(1)={lookahead}",
            "  ".repeat(self.depth),
            self.rule_name(event.rule_index),
        );
        self.depth += 1;
        Ok(())
    }

    fn exit_every_rule(&mut self, rule_index: usize) {
        self.depth = self.depth.saturating_sub(1);
        eprintln!(
            "{}exit    {}",
            "  ".repeat(self.depth),
            self.rule_name(rule_index),
        );
    }
}

fn process(
    input: InputStream,
    options: Options,
) -> miette::Result<bool> {
    let source = diagnostic_source(&input);
    let lexer = __LEXER_TYPE__::new(input);
    let mut tokens = CommonTokenStream::try_new(lexer)
        .into_diagnostic()
        .wrap_err("failed to buffer lexer tokens")?;
    let lexer_errors = report_lexer_errors(&mut tokens, &source);
    if options.tokens {
        for token in tokens.tokens() {
            println!("{token}");
        }
    }

    let mut parser = __PARSER_TYPE__::new(tokens);
    let parser_diagnostics =
        DiagnosticCollector::new(PARSER_DIAGNOSTIC_CODE, Arc::clone(&source));
    parser.remove_error_listeners();
    parser.add_error_listener(parser_diagnostics.clone());
    if options.tree {
        parser.set_build_parse_trees(true);
    }
    if options.diagnostics {
        parser.set_report_diagnostic_errors(true);
        parser.set_prediction_mode(PredictionMode::LlExactAmbigDetection);
    }
    if options.sll {
        parser.set_prediction_mode(PredictionMode::Sll);
    }
    if options.trace {
        parser.add_parse_listener(TraceListener::new(
            generated_parser::rule_names(),
        ));
    }

    let result = parser.__RULE_METHOD__();
    let parser_errors = parser.number_of_syntax_errors();
    let root = match result {
        Ok(root) => Some(root),
        Err(error) => {
            if !matches!(error, AntlrError::ParserError { .. }) {
                eprintln!("Error: {:?}", miette::Report::msg(error.to_string()));
            }
            None
        }
    };
    parser_diagnostics.report();
    if options.tree && let Some(root) = root {
        println!(
            "{}",
            parser
                .node(root)
                .to_string_tree(Some(&parser), parser.token_store()),
        );
    }
    Ok(lexer_errors == 0 && parser_errors == 0 && root.is_some())
}

"#;

fn assemble_runner(template: &str) -> String {
    replace_all(template, "__RUNNER_PREAMBLE__", RUNNER_PREAMBLE)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;

    fn lexer(name: &str) -> TestRigLexer {
        TestRigLexer::new(name.to_owned())
    }

    fn parser(name: &str, rules: &[&str]) -> TestRigParser {
        TestRigParser::new(
            name.to_owned(),
            rules.iter().map(|rule| (*rule).to_owned()).collect(),
        )
    }

    #[test]
    fn renders_parser_runner_with_generated_method_name() {
        let rendered = render_test_rig(
            "reset",
            &[lexer("DemoLexer")],
            &[parser("DemoParser", &["start", "reset"])],
        )
        .expect("test rig should render");

        insta::assert_snapshot!("parser_runner_rendered", rendered);
    }

    #[test]
    fn renders_lexer_runner_for_tokens_start_rule() {
        let rendered = render_test_rig("tokens", &[lexer("DemoLexer")], &[])
            .expect("lexer test rig should render");

        insta::assert_snapshot!("lexer_runner_rendered", rendered);
    }

    #[test]
    fn selects_lexer_by_parser_stem_among_multiple_candidates() {
        let lexers = [lexer("OtherLexer"), lexer("DemoLexer")];
        let parser = parser("DemoParser", &["start"]);

        let selected =
            select_lexer(&lexers, Some(&parser)).expect("matching lexer should be selected");

        insta::assert_snapshot!(&selected.grammar_name, @"DemoLexer");
    }

    #[test]
    fn rejects_missing_and_unmatched_lexers() {
        let missing = select_lexer(&[], None).expect_err("missing lexer should fail");
        insta::assert_snapshot!(
            missing.to_string(),
            @"test rig requires a generated lexer; use a combined grammar or pass --lexer-grammar for a split grammar"
        );

        let lexers = [lexer("FirstLexer"), lexer("SecondLexer")];
        let parser = parser("DemoParser", &["start"]);
        let unmatched = select_lexer(&lexers, Some(&parser))
            .expect_err("unmatched lexer candidates should fail");
        insta::assert_snapshot!(
            unmatched.to_string(),
            @"test rig cannot choose a lexer from multiple generated lexers: FirstLexer, SecondLexer"
        );
    }

    #[test]
    fn rejects_unknown_and_ambiguous_rules() {
        let missing = render_test_rig(
            "missing",
            &[lexer("DemoLexer")],
            &[parser("DemoParser", &["start"])],
        )
        .expect_err("missing start rule should fail");
        insta::assert_snapshot!(
            missing.to_string(),
            @"test rig start rule `missing` was not found; available rules: DemoParser.start"
        );

        let ambiguous = render_test_rig(
            "start",
            &[lexer("DemoLexer")],
            &[
                parser("FirstParser", &["start"]),
                parser("SecondParser", &["start"]),
            ],
        )
        .expect_err("ambiguous start rule should fail");
        insta::assert_snapshot!(
            ambiguous.to_string(),
            @"test rig start rule `start` is ambiguous across parsers: FirstParser, SecondParser"
        );
    }
}
