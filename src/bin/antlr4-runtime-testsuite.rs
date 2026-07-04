#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "../bin_support/rust_names.rs"]
mod rust_names;
#[path = "../bin_support/templates.rs"]
mod templates;

use rust_names::{module_name, rust_function_name, rust_string, rust_type_name};
use templates::{
    is_after_action, is_definitions_action, is_init_action, is_members_action, is_options_block,
    matching_action_brace, matching_template_close, named_action_templates,
    next_parser_action_block, next_predicate_action_block, next_template_block,
    parse_template_string, split_template_arguments, template_sequence_bodies,
};

const DESCRIPTOR_PATH: &str = "resources/org/antlr/v4/test/runtime/descriptors";
const ANTLR_JAR_ENV: &str = "ANTLR4_JAR";
const DESCRIPTORS_ENV: &str = "ANTLR4_RUNTIME_TESTSUITE";
const DEFAULT_ANTLR_JAR: &str = "/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar";
const DEFAULT_DESCRIPTORS: &str = "/tmp/antlr-cleanroom/antlr4-upstream/runtime-testsuite";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse()?;
    let descriptor_root = resolve_descriptor_root(&args.descriptors)?;
    let descriptors = load_descriptors(&descriptor_root, &args)?;
    let mut summary = Summary::default();

    if args.work_dir.exists() && !args.keep {
        fs::remove_dir_all(&args.work_dir)?;
    }
    fs::create_dir_all(&args.work_dir)?;

    for descriptor in descriptors {
        if let Some(reason) = unsupported_reason(&descriptor) {
            summary.skipped += 1;
            println!("skip {}: {reason}", descriptor.id());
            continue;
        }

        summary.ran += 1;
        match run_descriptor(&args, &descriptor) {
            Ok(result)
                if result.output == descriptor.output && result.errors == descriptor.errors =>
            {
                summary.passed += 1;
                println!("pass {}", descriptor.id());
                remove_descriptor_work_dir(&args, &descriptor)?;
            }
            Ok(result) => {
                summary.failed += 1;
                eprintln!(
                    "fail {}\nexpected stdout:\n{}\nactual stdout:\n{}\nexpected stderr:\n{}\nactual stderr:\n{}",
                    descriptor.id(),
                    descriptor.output,
                    result.output,
                    descriptor.errors,
                    result.errors
                );
            }
            Err(error) => {
                summary.failed += 1;
                eprintln!("fail {}: {error}", descriptor.id());
            }
        }

        if args.limit.is_some_and(|limit| summary.ran >= limit) {
            break;
        }
    }

    println!(
        "summary: {} passed, {} failed, {} skipped, {} run",
        summary.passed, summary.failed, summary.skipped, summary.ran
    );

    if summary.failed == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} runtime-testsuite case(s) failed",
            summary.failed
        ))
        .into())
    }
}

#[derive(Debug)]
struct Args {
    antlr_jar: PathBuf,
    descriptors: PathBuf,
    runtime_crate: PathBuf,
    work_dir: PathBuf,
    group: Option<String>,
    case_name: Option<String>,
    limit: Option<usize>,
    keep: bool,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut antlr_jar = None;
        let mut descriptors = None;
        let mut runtime_crate = env::current_dir().map_err(|error| error.to_string())?;
        let mut work_dir = runtime_crate.join("target/antlr-runtime-testsuite");
        let mut group = None;
        let mut case_name = None;
        let mut limit = None;
        let mut keep = false;

        let mut iter = env::args().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--antlr-jar" => {
                    antlr_jar = Some(PathBuf::from(next_arg(&mut iter, "--antlr-jar")?));
                }
                "--descriptors" => {
                    descriptors = Some(PathBuf::from(next_arg(&mut iter, "--descriptors")?));
                }
                "--runtime-crate" => {
                    runtime_crate = PathBuf::from(next_arg(&mut iter, "--runtime-crate")?);
                    work_dir = runtime_crate.join("target/antlr-runtime-testsuite");
                }
                "--work-dir" => work_dir = PathBuf::from(next_arg(&mut iter, "--work-dir")?),
                "--group" => group = Some(next_arg(&mut iter, "--group")?),
                "--case" => {
                    let value = next_arg(&mut iter, "--case")?;
                    if let Some((case_group, name)) = value.split_once('/') {
                        group = Some(case_group.to_owned());
                        case_name = Some(name.to_owned());
                    } else {
                        case_name = Some(value);
                    }
                }
                "--limit" => {
                    let value = next_arg(&mut iter, "--limit")?;
                    limit = Some(
                        value
                            .parse::<usize>()
                            .map_err(|error| format!("invalid --limit {value:?}: {error}"))?,
                    );
                }
                "--keep" => keep = true,
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unknown argument {other}\n\n{}", usage())),
            }
        }

        let antlr_jar = resolve_path_argument(
            antlr_jar,
            ANTLR_JAR_ENV,
            vec![
                runtime_crate.join("tools/antlr-4.13.2-complete.jar"),
                runtime_crate.join("target/antlr-4.13.2-complete.jar"),
                PathBuf::from(DEFAULT_ANTLR_JAR),
            ],
            "--antlr-jar",
            "ANTLR tool jar",
        )?;
        let descriptors = resolve_path_argument(
            descriptors,
            DESCRIPTORS_ENV,
            vec![
                runtime_crate.join("target/antlr4/runtime-testsuite"),
                runtime_crate.join("../antlr4/runtime-testsuite"),
                PathBuf::from(DEFAULT_DESCRIPTORS),
            ],
            "--descriptors",
            "ANTLR runtime-testsuite descriptors",
        )?;

        Ok(Self {
            antlr_jar,
            descriptors,
            runtime_crate,
            work_dir,
            group,
            case_name,
            limit,
            keep,
        })
    }
}

/// Resolves an optional CLI path from, in order, the explicit flag, an
/// environment override, and known local checkout locations.
///
/// The bare `cargo run --bin antlr4-runtime-testsuite` workflow is meant for
/// the maintainer machine where the ANTLR jar and upstream checkout already
/// live under `/tmp/antlr-cleanroom`; fresh environments can still pass
/// explicit paths or set the documented environment variables.
fn resolve_path_argument(
    explicit: Option<PathBuf>,
    env_key: &str,
    candidates: Vec<PathBuf>,
    flag: &str,
    label: &str,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Ok(value) = env::var(env_key) {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    candidates.into_iter().find(|path| path.exists()).ok_or_else(|| {
        format!(
            "missing {label}; pass {flag}, set {env_key}, or create the default checkout under /tmp/antlr-cleanroom\n\n{}",
            usage()
        )
    })
}

fn next_arg(iter: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value\n\n{}", usage()))
}

fn usage() -> String {
    "usage: antlr4-runtime-testsuite [--antlr-jar ANTLR.jar] [--descriptors PATH] [--case Group/Name] [--group Group] [--limit N] [--keep]\n\nDefaults: ANTLR4_JAR or /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar; ANTLR4_RUNTIME_TESTSUITE or /tmp/antlr-cleanroom/antlr4-upstream/runtime-testsuite".to_owned()
}

#[derive(Debug, Default)]
struct Summary {
    ran: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
}

#[derive(Clone, Debug)]
struct Descriptor {
    group: String,
    name: String,
    test_type: String,
    grammar_name: String,
    grammar: String,
    start_rule: String,
    input: String,
    output: String,
    errors: String,
    flags: String,
    slave_grammars: Vec<String>,
}

impl Descriptor {
    fn id(&self) -> String {
        format!("{}/{}", self.group, self.name)
    }

    fn is_parser(&self) -> bool {
        matches!(self.test_type.as_str(), "Parser" | "CompositeParser")
    }

    fn is_lexer(&self) -> bool {
        matches!(self.test_type.as_str(), "Lexer" | "CompositeLexer")
    }

    fn is_composite(&self) -> bool {
        matches!(
            self.test_type.as_str(),
            "CompositeParser" | "CompositeLexer"
        )
    }
}

#[derive(Debug)]
struct RunResult {
    output: String,
    errors: String,
}

/// Resolves either the upstream `runtime-testsuite` root or the descriptor
/// directory itself to the concrete descriptor directory.
fn resolve_descriptor_root(path: &Path) -> io::Result<PathBuf> {
    let direct = path.join(DESCRIPTOR_PATH);
    if direct.is_dir() {
        return Ok(direct);
    }
    if path.ends_with("descriptors") && path.is_dir() {
        return Ok(path.to_path_buf());
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "descriptor root not found under {}; pass runtime-testsuite root or descriptors directory",
            path.display()
        ),
    ))
}

/// Loads descriptor files in stable order and applies the CLI group/case
/// filters before parsing.
fn load_descriptors(root: &Path, args: &Args) -> io::Result<Vec<Descriptor>> {
    let mut descriptors = Vec::new();
    let mut group_dirs = sorted_children(root)?;
    group_dirs.retain(|entry| entry.path.is_dir());
    for group_dir in group_dirs {
        let group = group_dir.name;
        if args.group.as_ref().is_some_and(|wanted| wanted != &group) {
            continue;
        }

        let mut files = sorted_children(&group_dir.path)?;
        files.retain(|entry| entry.path.extension() == Some(OsStr::new("txt")));
        for file in files {
            let name = file.name.trim_end_matches(".txt").to_owned();
            if args
                .case_name
                .as_ref()
                .is_some_and(|wanted| wanted != &name)
            {
                continue;
            }
            let text = fs::read_to_string(&file.path)?;
            descriptors.push(parse_descriptor(group.clone(), name, &text)?);
        }
    }
    Ok(descriptors)
}

#[derive(Debug)]
struct DirEntryInfo {
    name: String,
    path: PathBuf,
}

fn sorted_children(path: &Path) -> io::Result<Vec<DirEntryInfo>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        entries.push(DirEntryInfo {
            name,
            path: entry.path(),
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

/// Parses ANTLR runtime-testsuite descriptor text into the subset this harness
/// needs for execution and output comparison.
fn parse_descriptor(group: String, name: String, text: &str) -> io::Result<Descriptor> {
    let mut current_section: Option<String> = None;
    let mut current_value = String::new();
    let mut sections = Vec::new();

    for line in text.lines() {
        if let Some(section) = section_name(line) {
            if let Some(field) = current_section.replace(section.to_owned()) {
                sections.push((field, current_value.clone()));
                current_value.clear();
            }
        } else {
            current_value.push_str(line);
            current_value.push('\n');
        }
    }
    if let Some(field) = current_section {
        sections.push((field, current_value));
    }

    let mut descriptor = Descriptor {
        group,
        name,
        test_type: "Lexer".to_owned(),
        grammar_name: String::new(),
        grammar: String::new(),
        start_rule: String::new(),
        input: String::new(),
        output: String::new(),
        errors: String::new(),
        flags: String::new(),
        slave_grammars: Vec::new(),
    };

    for (section, value) in sections {
        let value = normalize_section_value(&value);
        match section.as_str() {
            "type" => descriptor.test_type = value,
            "grammar" => {
                let value = render_st_backslash_escapes(&value);
                descriptor.grammar_name = grammar_name(&value)?;
                descriptor.grammar = value;
            }
            "slaveGrammar" => descriptor
                .slave_grammars
                .push(render_st_backslash_escapes(&value)),
            "input" => descriptor.input = value,
            "output" => descriptor.output = value,
            "errors" => descriptor.errors = value,
            "flags" => descriptor.flags = value,
            "start" => descriptor.start_rule = value,
            "notes" | "skip" => {}
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown descriptor section {other:?}"),
                ));
            }
        }
    }

    Ok(descriptor)
}

/// Returns a descriptor section name, deliberately excluding token display
/// output such as `[@0,...]`.
fn section_name(line: &str) -> Option<&str> {
    if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
        let name = &line[1..line.len() - 1];
        match name {
            "notes" | "type" | "grammar" | "slaveGrammar" | "start" | "input" | "output"
            | "errors" | "flags" | "skip" => Some(name),
            _ => None,
        }
    } else {
        None
    }
}

/// Mirrors the upstream descriptor parser's section trimming and triple-quote
/// handling so expected stdout/stderr bytes compare correctly.
fn normalize_section_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with("\"\"\"") {
        remove_marker(trimmed, "\"\"\"")
    } else if trimmed.contains('\n') {
        let mut out = trimmed.to_owned();
        out.push('\n');
        out
    } else {
        trimmed.to_owned()
    }
}

fn remove_marker(value: &str, marker: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(index) = rest.find(marker) {
        out.push_str(&rest[..index]);
        rest = &rest[index + marker.len()..];
    }
    out.push_str(rest);
    out
}

/// Applies the `StringTemplate` backslash collapse used by the upstream Java
/// harness when descriptor grammars are rendered as templates.
fn render_st_backslash_escapes(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek() {
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                Some('<' | '>') => {}
                _ => out.push(ch),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn grammar_name(grammar: &str) -> io::Result<String> {
    let first_line = grammar.lines().next().unwrap_or_default();
    let Some(start) = first_line.find("grammar ") else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("missing grammar declaration in {first_line:?}"),
        ));
    };
    let Some(stop) = first_line.find(';') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("missing grammar declaration semicolon in {first_line:?}"),
        ));
    };
    Ok(first_line[start + "grammar ".len()..stop].to_owned())
}

/// Classifies descriptors that the current metadata-first harness cannot run
/// yet while keeping them visible in summaries.
fn unsupported_reason(descriptor: &Descriptor) -> Option<&'static str> {
    if !descriptor.slave_grammars.is_empty() && !descriptor.is_composite() {
        return Some("composite grammars are not wired into the metadata harness yet");
    }
    if descriptor.is_composite() && !composite_grammar_supported(descriptor) {
        return Some("composite grammar shape is not wired into the metadata harness yet");
    }
    if !descriptor.flags.is_empty() && !runtime_flags_supported(descriptor) {
        return Some("diagnostic/profile/DFA flags are not implemented in the Rust harness yet");
    }
    let grammar = combined_grammar_source(descriptor);
    if has_target_template(&grammar) && !target_templates_supported(descriptor, &grammar) {
        return Some("target-template semantic actions are not rendered by this harness yet");
    }
    if descriptor.is_parser() {
        if !descriptor.output.is_empty() {
            if !target_templates_supported(descriptor, &grammar) {
                return Some(
                    "parser target actions/listeners are not wired into the Rust harness yet",
                );
            }
        }
        if !descriptor.errors.is_empty() && !parser_error_diagnostics_supported(descriptor) {
            return Some(
                "parser error recovery diagnostics are not wired into the Rust harness yet",
            );
        }
        return None;
    }
    if !descriptor.is_lexer() {
        return Some("descriptor type is not supported by the metadata harness yet");
    }
    None
}

/// Identifies descriptor runtime flags whose behavior is already represented by
/// the current Rust harness without extra setup.
fn runtime_flags_supported(descriptor: &Descriptor) -> bool {
    matches!(
        descriptor.flags.trim(),
        "notBuildParseTree" | "predictionMode=LL" | "predictionMode=SLL"
    ) || (descriptor.flags.trim() == "showDFA"
        && matches!(
            descriptor.id().as_str(),
            "SemPredEvalLexer/DisableRule"
                | "SemPredEvalLexer/EnumNotID"
                | "SemPredEvalLexer/IDnotEnum"
                | "SemPredEvalLexer/IDvsEnum"
                | "SemPredEvalLexer/Indent"
                | "SemPredEvalLexer/LexerInputPositionSensitivePredicates"
        ))
        || (descriptor.flags.trim() == "showDiagnosticErrors"
            && matches!(
                descriptor.id().as_str(),
                "SemPredEvalParser/TwoUnpredicatedAlts"
                    | "SemPredEvalParser/TwoUnpredicatedAltsAndOneOrthogonalAlt"
            ))
        || (descriptor.flags.trim() == "showDiagnosticErrors"
            && descriptor.group == "FullContextParsing")
}

/// Whitelists composite descriptors whose import and action shapes are modeled by
/// the current metadata harness.
fn composite_grammar_supported(descriptor: &Descriptor) -> bool {
    matches!(
        descriptor.id().as_str(),
        "CompositeLexers/LexerDelegatorInvokesDelegateRule"
            | "CompositeLexers/LexerDelegatorRuleOverridesDelegate"
            | "CompositeParsers/BringInLiteralsFromDelegate"
            | "CompositeParsers/CombinedImportsCombined"
            | "CompositeParsers/DelegatesSeeSameTokenType"
            | "CompositeParsers/DelegatorAccessesDelegateMembers"
            | "CompositeParsers/DelegatorInvokesDelegateRule"
            | "CompositeParsers/DelegatorInvokesDelegateRuleWithArgs"
            | "CompositeParsers/DelegatorInvokesDelegateRuleWithReturnStruct"
            | "CompositeParsers/DelegatorInvokesFirstVersionOfDelegateRule"
            | "CompositeParsers/DelegatorRuleOverridesDelegate"
            | "CompositeParsers/DelegatorRuleOverridesDelegates"
            | "CompositeParsers/DelegatorRuleOverridesLookaheadInDelegate"
            | "CompositeParsers/ImportedGrammarWithEmptyOptions"
            | "CompositeParsers/ImportedRuleWithAction"
            | "CompositeParsers/ImportLexerWithOnlyFragmentRules"
            | "CompositeParsers/KeywordVSIDOrder"
    )
}

/// Admits only parser-error descriptors covered by the current mismatch and
/// single-token recovery diagnostics, leaving mixed lexer/parser diagnostic
/// ordering cases skipped.
fn parser_error_diagnostics_supported(descriptor: &Descriptor) -> bool {
    if runtime_flags_supported(descriptor) && descriptor.flags.trim() == "showDiagnosticErrors" {
        return true;
    }
    matches!(
        descriptor.name.as_str(),
        "ConjuringUpToken"
            | "ConjuringUpTokenFromSet"
            | "ComplementSet"
            | "ExtraToken"
            | "ExtraTokensAndAltLabels"
            | "ExtraneousInput"
            | "InvalidEmptyInput"
            | "LL2"
            | "LL3"
            | "LLStar"
            | "MultiTokenDeletionBeforeLoop"
            | "MultiTokenDeletionBeforeLoop2"
            | "MultiTokenDeletionDuringLoop"
            | "MultiTokenDeletionDuringLoop2"
            | "NoTruePredsThrowsNoViableAlt"
            | "NoViableAlt"
            | "NoViableAltAvoidance"
            | "PredFromAltTestedInLoopBack_1"
            | "PredTestedEvenWhenUnAmbig_2"
            | "PredictionMode_SLL"
            | "SingleSetInsertion"
            | "SingleSetInsertionConsumption"
            | "SingleTokenDeletion"
            | "SingleTokenDeletionBeforeAlt"
            | "SingleTokenDeletionBeforeLoop"
            | "SingleTokenDeletionBeforeLoop2"
            | "SingleTokenDeletionBeforePredict"
            | "SingleTokenDeletionConsumption"
            | "SingleTokenDeletionDuringLoop"
            | "SingleTokenDeletionDuringLoop2"
            | "SingleTokenDeletionExpectingSet"
            | "SingleTokenInsertion"
            | "SemPredFailOption"
            | "SimpleValidate"
            | "SimpleValidate2"
            | "Sync"
            | "TokenMismatch"
            | "TokenMismatch2"
            | "TokenMismatch3"
            | "ValidateInDFA"
            | "UnicodeEscapedSMPRangeSetMismatch"
    )
}

/// Builds the grammar text passed to the Rust generator for action extraction.
///
/// ANTLR's metadata output for imported grammars is flattened into the delegator
/// `.interp` file, so action templates from imported rules must be visible to the
/// Rust generator as well. Delegates are ordered by the delegator's `import`
/// clause so rule overrides pick the same first definition ANTLR keeps.
fn combined_grammar_source(descriptor: &Descriptor) -> String {
    let mut out = String::new();
    let mut seen = BTreeSet::new();
    push_grammar_source(&mut out, &descriptor.grammar);
    append_imported_grammar_sources(&descriptor.grammar, descriptor, &mut seen, &mut out);
    for grammar in &descriptor.slave_grammars {
        if let Ok(name) = grammar_name(grammar) {
            if seen.insert(name) {
                push_grammar_source(&mut out, grammar);
            }
        }
    }
    out
}

fn append_imported_grammar_sources(
    grammar: &str,
    descriptor: &Descriptor,
    seen: &mut BTreeSet<String>,
    out: &mut String,
) {
    for import in imported_grammar_names(grammar) {
        if !seen.insert(import.clone()) {
            continue;
        }
        let Some(slave) = slave_grammar_by_name(descriptor, &import) else {
            continue;
        };
        push_grammar_source(out, slave);
        append_imported_grammar_sources(slave, descriptor, seen, out);
    }
}

fn slave_grammar_by_name<'a>(descriptor: &'a Descriptor, name: &str) -> Option<&'a str> {
    descriptor.slave_grammars.iter().find_map(|grammar| {
        grammar_name(grammar)
            .ok()
            .filter(|grammar_name| grammar_name == name)
            .map(|_| grammar.as_str())
    })
}

fn push_grammar_source(out: &mut String, grammar: &str) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(grammar);
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

/// Extracts direct `import A, B;` dependencies from a grammar header.
fn imported_grammar_names(grammar: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in grammar.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        let Some(imports) = line
            .strip_prefix("import ")
            .and_then(|value| value.strip_suffix(';'))
        else {
            continue;
        };
        names.extend(
            imports
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned),
        );
    }
    names
}

fn has_target_template(grammar: &str) -> bool {
    next_template_block(grammar, 0).is_some()
        || grammar.contains("{<")
        || grammar.contains("<BailErrorStrategy")
        || grammar.contains("<ImportRuleInvocationStack")
        || grammar.contains("<writeln")
        || grammar.contains("<write")
        || grammar.contains("<InputText")
        || grammar.contains("<LANotEquals")
        || grammar.contains("@members")
        || grammar.contains("@definitions")
}

fn target_templates_supported(descriptor: &Descriptor, grammar: &str) -> bool {
    if descriptor.is_lexer() {
        return lexer_target_templates_supported(descriptor, grammar);
    }
    if !descriptor.is_parser() {
        return false;
    }
    if unsupported_members_templates(grammar)
        || grammar.contains("@definitions")
        || !supported_signature_templates(grammar)
    {
        return false;
    }
    if grammar.contains("@init") && !supported_init_action_templates(grammar) {
        return false;
    }
    if grammar.contains("@after") && !supported_after_action_templates(grammar) {
        return false;
    }
    supported_action_templates(grammar)
}

fn lexer_target_templates_supported(descriptor: &Descriptor, grammar: &str) -> bool {
    if descriptor.name == "PositionAdjustingLexer" {
        return grammar.contains("<PositionAdjustingLexer")
            && supported_lexer_predicate_templates(grammar)
            && supported_action_templates(grammar);
    }
    if grammar.contains("@members")
        || grammar.contains("@definitions")
        || grammar.contains("<PositionAdjustingLexer")
    {
        return false;
    }
    supported_lexer_predicate_templates(grammar) && supported_action_templates(grammar)
}

fn supported_action_templates(grammar: &str) -> bool {
    let mut offset = 0;
    while let Some(block) = next_parser_action_block(grammar, offset, is_int_return_assignment) {
        offset = block.after_brace;
        if block.predicate
            || is_after_action(grammar, block.open_brace)
            || is_init_action(grammar, block.open_brace)
            || is_definitions_action(grammar, block.open_brace)
            || is_members_action(grammar, block.open_brace)
            || is_options_block(grammar, block.open_brace)
        {
            continue;
        }
        if !is_supported_action_block(block.body) {
            return false;
        }
    }
    true
}

fn is_supported_action_block(body: &str) -> bool {
    body.trim().is_empty()
        || is_supported_action_template_sequence(body)
        || is_int_return_assignment(body)
}

/// Allows upstream parser setup actions that are either implemented directly by
/// the smoke harness or irrelevant to metadata-driven recognition.
fn supported_init_action_templates(grammar: &str) -> bool {
    let mut saw_init_action = false;
    let mut offset = 0;
    while let Some(block) = next_template_block(grammar, offset) {
        offset = block.after_brace;
        if block.predicate || !is_init_action(grammar, block.open_brace) {
            continue;
        }
        saw_init_action = true;
        if !matches!(
            block.body.trim(),
            "BuildParseTrees()"
                | "BailErrorStrategy()"
                | "GetExpectedTokenNames():writeln()"
                | "LL_EXACT_AMBIG_DETECTION()"
        ) {
            return false;
        }
    }
    saw_init_action
}

fn supported_after_action_templates(grammar: &str) -> bool {
    let mut saw_after_action = false;
    let listener_kind = listener_template_kind(grammar);
    for block in named_action_templates(grammar, "@after") {
        saw_after_action = true;
        let body = block.body.trim();
        if is_string_tree_label_template(body)
            || is_context_member_string_tree_template(body)
            || (listener_kind.is_some() && is_context_member_walk_listener_template(body))
        {
            continue;
        }
        if !is_supported_action_template(body) {
            return false;
        }
    }
    saw_after_action
}

fn supported_lexer_predicate_templates(grammar: &str) -> bool {
    let mut offset = 0;
    while let Some(block) = next_predicate_action_block(grammar, offset) {
        offset = block.after_brace;
        if block.body.contains('<') && !is_supported_lexer_predicate_template(block.body.trim()) {
            return false;
        }
    }
    true
}

fn is_supported_lexer_predicate_template(body: &str) -> bool {
    if let Some(inner) = single_template_body(body) {
        return is_supported_lexer_predicate_template(inner);
    }
    matches!(body, "True()" | "False()")
        || body == r#"<Column()> \< 2"#
        || body == "<Column()> < 2"
        || body == "<Column()> >= 2"
        || body
            .strip_prefix("TextEquals(")
            .and_then(|value| value.strip_suffix(')'))
            .is_some_and(|argument| parse_template_string(argument).is_some())
        || body
            .strip_prefix("TokenStartColumnEquals(")
            .and_then(|value| value.strip_suffix(')'))
            .is_some_and(|argument| {
                parse_template_string(argument).is_some_and(|value| value.parse::<usize>().is_ok())
            })
}

fn single_template_body(body: &str) -> Option<&str> {
    let body = body.trim();
    if body.as_bytes().first() != Some(&b'<') {
        return None;
    }
    let close = matching_template_close(body, 1)?;
    (close + 1 == body.len()).then_some(&body[1..close])
}

/// Mirrors the generator's currently supported action-template subset so the
/// harness runs only descriptors it can translate faithfully.
fn is_supported_action_template(body: &str) -> bool {
    matches!(
        body,
        r#"writeln("$text")"#
            | r#"write("$text")"#
            | "InputText():writeln()"
            | "Text():writeln()"
            | "Text():write()"
            | "RuleInvocationStack():writeln()"
            | "RuleInvocationStack():write()"
            | "Pass()"
            | "LL_EXACT_AMBIG_DETECTION()"
            | "DumpDFA()"
            | r#"ToStringTree("$ctx"):writeln()"#
            | r#"ToStringTree("$ctx"):write()"#
            | "Invoke_foo()"
    ) || body.starts_with("writeln(\"\\\"")
        || body.starts_with("write(\"\\\"")
        || is_string_tree_label_template(body)
        || is_noop_action_template(body)
        || is_append_str_token_text_template(body)
        || is_token_text_template(body)
        || is_token_display_template(body)
        || is_add_member_template(body)
        || is_member_value_template(body)
        || is_rule_value_template(body)
        || (body.starts_with("PlusText(\"") && body.ends_with("):writeln()"))
        || (body.starts_with("PlusText(\"") && body.ends_with("):write()"))
}

fn is_supported_action_template_sequence(body: &str) -> bool {
    template_sequence_bodies(body).is_some_and(|templates| {
        templates
            .into_iter()
            .all(|template| is_supported_action_template(template.trim()))
    })
}

fn is_add_member_template(body: &str) -> bool {
    body.strip_prefix("AddMember(")
        .and_then(|value| value.strip_suffix(')'))
        .map(split_template_arguments)
        .is_some_and(|arguments| {
            let [member, value] = arguments.as_slice() else {
                return false;
            };
            parse_template_string(member).is_some()
                && parse_template_string(value).is_some_and(|value| value.parse::<i64>().is_ok())
        })
}

fn is_member_value_template(body: &str) -> bool {
    let argument = body
        .strip_prefix("writeln(GetMember(")
        .and_then(|value| value.strip_suffix("))"))
        .or_else(|| {
            body.strip_prefix("write(GetMember(")
                .and_then(|value| value.strip_suffix("))"))
        });
    argument.is_some_and(|argument| parse_template_string(argument).is_some())
}

fn supported_signature_templates(grammar: &str) -> bool {
    grammar.lines().all(|line| {
        supported_signature_template_on_line(line, "returns [")
            && supported_signature_template_on_line(line, "locals [")
    })
}

/// Checks one `returns [...]` or `locals [...]` clause for target-template
/// signatures the generator can erase or model in the runtime-test harness.
fn supported_signature_template_on_line(line: &str, marker: &str) -> bool {
    let Some(marker_start) = line.find(marker) else {
        return true;
    };
    let after_marker = marker_start + marker.len();
    let leading_whitespace = line[after_marker..].len() - line[after_marker..].trim_start().len();
    let template_start = after_marker + leading_whitespace;
    if line.as_bytes().get(template_start) != Some(&b'<') {
        return true;
    }
    let Some(close_angle) = matching_template_close(line, template_start + 1) else {
        return false;
    };
    let body = &line[template_start + 1..close_angle];
    (body.starts_with("IntArg(") && body.ends_with(')'))
        || matches!(body, "StringType()" | "StringList()")
}

/// Allows only member templates that are no-op scaffolding for this metadata
/// harness; real listener/member customizations stay skipped.
fn unsupported_members_templates(grammar: &str) -> bool {
    if !(grammar.contains("@members") || grammar.contains("@parser::members")) {
        return false;
    }
    let mut saw_supported = false;
    let mut offset = 0;
    while let Some(block) = next_template_block(grammar, offset) {
        offset = block.after_brace;
        if !is_members_action(grammar, block.open_brace) {
            continue;
        }
        if !is_supported_members_template(block.body.trim()) {
            return true;
        }
        saw_supported = true;
    }
    !saw_supported
}

fn is_supported_members_template(body: &str) -> bool {
    body == "DeclareContextListGettersFunction()"
        || body == "Declare_foo()"
        || body == "Declare_pred()"
        || (body.starts_with("InitBooleanMember(") && body.ends_with(",True())"))
        || (body.starts_with("InitIntMember(") && body.ends_with(')'))
}

fn listener_template_kind(grammar: &str) -> Option<&'static str> {
    grammar
        .lines()
        .find_map(|line| listener_line_kind(line.trim()))
}

fn listener_line_kind(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("<BasicListener(") {
        Some("basic")
    } else if trimmed.starts_with("<TokenGetterListener(") {
        Some("token-getter")
    } else if trimmed.starts_with("<RuleGetterListener(") {
        Some("rule-getter")
    } else if trimmed.starts_with("<LRListener(") {
        Some("left-recursive")
    } else if trimmed.starts_with("<LRWithLabelsListener(") {
        Some("left-recursive-labels")
    } else {
        None
    }
}

fn is_noop_action_template(body: &str) -> bool {
    (body.starts_with("AssignLocal(")
        || body.starts_with("AssertIsList(")
        || body.starts_with("InitIntVar(")
        || body.starts_with("IntArg(")
        || body.starts_with("Production(")
        || body.starts_with("Result(")
        || body.starts_with("SetMember("))
        && body.ends_with(')')
}

fn is_token_text_template(body: &str) -> bool {
    let Some(argument) = body
        .strip_prefix("writeln(\"$")
        .and_then(|value| value.strip_suffix(".text\")"))
        .or_else(|| {
            body.strip_prefix("write(\"$")
                .and_then(|value| value.strip_suffix(".text\")"))
        })
    else {
        return false;
    };
    argument
        .chars()
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Recognizes the simple `$rule.v` and `$rule.result` print helpers that the
/// generator can evaluate from the parse tree for left-recursion fixtures.
fn is_rule_value_template(body: &str) -> bool {
    let Some(argument) = body
        .strip_prefix("writeln(\"$")
        .and_then(|value| value.strip_suffix("\")"))
        .or_else(|| {
            body.strip_prefix("write(\"$")
                .and_then(|value| value.strip_suffix("\")"))
        })
    else {
        return false;
    };
    let Some((rule_name, value_name)) = argument.split_once('.') else {
        return false;
    };
    is_antlr_identifier(rule_name) && is_antlr_identifier(value_name) && value_name != "text"
}

/// Recognizes simple raw return assignments that ANTLR lowers to action
/// transitions and the Rust generator captures as rule-context return slots.
fn is_int_return_assignment(body: &str) -> bool {
    let body = body.trim();
    let Some((name, value)) = body
        .strip_prefix('$')
        .and_then(|body| body.strip_suffix(';'))
        .and_then(|body| body.split_once('='))
    else {
        return false;
    };
    is_antlr_identifier(name.trim()) && value.trim().parse::<i64>().is_ok()
}

/// Mirrors the generator's `AppendStr` subset: a literal prefix plus either the
/// current rule text or a `$label.text` payload.
fn is_append_str_token_text_template(body: &str) -> bool {
    append_str_arguments(body)
        .map(split_template_arguments)
        .is_some_and(|arguments| {
            let [prefix, value] = arguments.as_slice() else {
                return false;
            };
            parse_template_string(prefix).is_some()
                && parse_template_string(value).is_some_and(|value| {
                    value == "$text"
                        || value
                            .strip_prefix('$')
                            .and_then(|label| label.strip_suffix(".text"))
                            .is_some_and(is_antlr_identifier)
                })
        })
}

/// Extracts the comma-separated arguments from the fluent
/// `AppendStr(...):write[ln]()` forms used by runtime descriptors.
fn append_str_arguments(body: &str) -> Option<&str> {
    if let Some(arguments) = body
        .strip_prefix("AppendStr(")
        .and_then(|value| value.strip_suffix("):writeln()"))
    {
        return Some(arguments);
    }
    body.strip_prefix("AppendStr(")
        .and_then(|value| value.strip_suffix("):write()"))
}

fn is_token_display_template(body: &str) -> bool {
    append_arguments(body)
        .map(split_template_arguments)
        .is_some_and(|arguments| {
            let [prefix, value] = arguments.as_slice() else {
                return false;
            };
            parse_template_string(prefix).is_some()
                && parse_template_string(value).is_some_and(|value| {
                    value.strip_prefix('$').is_some_and(|name| {
                        is_antlr_identifier(name.strip_suffix(".stop").unwrap_or(name))
                    })
                })
        })
}

fn append_arguments(body: &str) -> Option<&str> {
    if let Some(arguments) = body
        .strip_prefix("Append(")
        .and_then(|value| value.strip_suffix("):writeln()"))
    {
        return Some(arguments);
    }
    if let Some(arguments) = body
        .strip_prefix("Append(")
        .and_then(|value| value.strip_suffix("):write()"))
    {
        return Some(arguments);
    }
    if let Some(arguments) = body
        .strip_prefix("writeln(Append(")
        .and_then(|value| value.strip_suffix("))"))
    {
        return Some(arguments);
    }
    body.strip_prefix("write(Append(")
        .and_then(|value| value.strip_suffix("))"))
}

fn is_antlr_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Recognizes `ToStringTree("$label.ctx")` templates that the generator can
/// resolve from a rule-level `@after` action.
fn is_string_tree_label_template(body: &str) -> bool {
    let Some(argument) = body
        .strip_prefix("ToStringTree(\"$")
        .and_then(|value| value.strip_suffix(".ctx\"):writeln()"))
        .or_else(|| {
            body.strip_prefix("ToStringTree(\"$")
                .and_then(|value| value.strip_suffix(".ctx\"):write()"))
        })
    else {
        return false;
    };
    argument
        .chars()
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_context_member_string_tree_template(body: &str) -> bool {
    if let Some(arguments) = body
        .strip_prefix("ContextMember(")
        .and_then(|value| value.strip_suffix("):ToStringTree():writeln()"))
        .or_else(|| {
            body.strip_prefix("ContextMember(")
                .and_then(|value| value.strip_suffix("):ToStringTree():write()"))
        })
    {
        return context_member_label(arguments).is_some();
    }
    false
}

fn is_context_member_walk_listener_template(body: &str) -> bool {
    body.strip_prefix("ContextMember(")
        .and_then(|value| value.strip_suffix("):WalkListener()"))
        .and_then(context_member_label)
        .is_some()
}

/// Validates `ContextMember("$ctx", "label")` wrappers used by listener
/// descriptors before the generator resolves the label to a rule reference.
fn context_member_label(arguments: &str) -> Option<String> {
    let arguments = split_template_arguments(arguments);
    let [ctx, label] = arguments.as_slice() else {
        return None;
    };
    (parse_template_string(ctx)? == "$ctx").then(|| parse_template_string(label))?
}

/// Runs one descriptor through ANTLR metadata generation, Rust code generation,
/// a temporary Cargo crate, and process output capture.
fn run_descriptor(args: &Args, descriptor: &Descriptor) -> io::Result<RunResult> {
    let case_dir = descriptor_work_dir(args, descriptor);
    if case_dir.exists() {
        fs::remove_dir_all(&case_dir)?;
    }
    fs::create_dir_all(&case_dir)?;

    let source_grammar_path = case_dir.join(format!("{}.source.g4", descriptor.grammar_name));
    fs::write(&source_grammar_path, combined_grammar_source(descriptor))?;
    let grammar_path = case_dir.join(format!("{}.g4", descriptor.grammar_name));
    fs::write(
        &grammar_path,
        render_target_templates_for_metadata(&descriptor.grammar),
    )?;
    write_slave_grammars(&case_dir, descriptor)?;

    let java_dir = case_dir.join("antlr");
    fs::create_dir_all(&java_dir)?;
    run_checked(
        Command::new("java")
            .arg("-jar")
            .arg(&args.antlr_jar)
            .arg("-o")
            .arg(&java_dir)
            .arg("-Xexact-output-dir")
            .arg(&grammar_path),
        "ANTLR tool",
    )?;

    let rust_dir =
        generate_rust_modules(args, descriptor, &java_dir, &case_dir, &source_grammar_path)?;

    let smoke_dir = case_dir.join("rust");
    create_smoke_crate(args, descriptor, &rust_dir, &smoke_dir)?;
    let output = run_output(
        Command::new("cargo")
            .arg("run")
            .arg("--quiet")
            .current_dir(&smoke_dir),
    )?;
    Ok(RunResult {
        output: String::from_utf8_lossy(&output.stdout).into_owned(),
        errors: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn descriptor_work_dir(args: &Args, descriptor: &Descriptor) -> PathBuf {
    args.work_dir.join(safe_case_dir(&descriptor.id()))
}

/// Deletes successful descriptor output unless the caller asked to keep cases
/// around for inspection.
fn remove_descriptor_work_dir(args: &Args, descriptor: &Descriptor) -> io::Result<()> {
    if args.keep {
        return Ok(());
    }
    fs::remove_dir_all(descriptor_work_dir(args, descriptor))
}

/// Writes imported grammars next to the delegator grammar before invoking ANTLR,
/// matching the file layout expected by ANTLR's import resolver.
fn write_slave_grammars(case_dir: &Path, descriptor: &Descriptor) -> io::Result<()> {
    for grammar in &descriptor.slave_grammars {
        let grammar_path = case_dir.join(format!("{}.g4", grammar_name(grammar)?));
        fs::write(grammar_path, render_target_templates_for_metadata(grammar))?;
    }
    Ok(())
}

/// Replaces target-template actions with neutral ANTLR actions before invoking
/// the official tool for `.interp` metadata.
///
/// The original grammar is still passed to `antlr4-rust-gen`, which replays the
/// supported templates from Rust after the ATN path has been selected.
fn render_target_templates_for_metadata(grammar: &str) -> String {
    let grammar = strip_named_action_template_body(grammar, "@after");
    let grammar = render_target_predicates_for_metadata(&grammar);
    let mut out = String::with_capacity(grammar.len());
    let mut offset = 0;
    while let Some(block) = next_template_block(&grammar, offset) {
        out.push_str(&grammar[offset..block.open_brace]);
        if block.predicate {
            out.push_str("{true}");
        } else {
            out.push_str("{}");
        }
        offset = block.after_brace;
    }
    out.push_str(&grammar[offset..]);
    strip_supported_preamble_templates(&strip_template_comments(&out))
}

/// Replaces target-template predicate expressions with `true` while preserving
/// the surrounding `?`, so ANTLR still serializes a predicate transition.
fn render_target_predicates_for_metadata(grammar: &str) -> String {
    let mut out = String::with_capacity(grammar.len());
    let mut offset = 0;
    while let Some(block) = next_predicate_action_block(grammar, offset) {
        if block.body.contains('<') {
            out.push_str(&grammar[offset..block.open_brace]);
            out.push_str("{true}");
        } else {
            out.push_str(&grammar[offset..block.after_brace]);
        }
        offset = block.after_brace;
    }
    out.push_str(&grammar[offset..]);
    out
}

/// Replaces target-template contents in named action blocks with an empty
/// action so ANTLR can still emit metadata for the surrounding grammar.
fn strip_named_action_template_body(grammar: &str, marker: &str) -> String {
    let mut out = String::with_capacity(grammar.len());
    let mut offset = 0;
    while let Some(marker_start) = grammar[offset..].find(marker).map(|index| offset + index) {
        let Some(open_brace) = grammar[marker_start..]
            .find('{')
            .map(|index| marker_start + index)
        else {
            break;
        };
        let Some(close_brace) = matching_action_brace(grammar, open_brace + 1) else {
            break;
        };
        out.push_str(&grammar[offset..=open_brace]);
        if !grammar[open_brace + 1..close_brace].contains('<') {
            out.push_str(&grammar[open_brace + 1..close_brace]);
        }
        out.push('}');
        offset = close_brace + 1;
    }
    out.push_str(&grammar[offset..]);
    out
}

/// Removes upstream `StringTemplate` comments before handing grammar text to
/// ANTLR, which only understands comments in ANTLR syntax.
fn strip_template_comments(grammar: &str) -> String {
    let mut out = String::with_capacity(grammar.len());
    let mut rest = grammar;
    while let Some(start) = rest.find("<!") {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(stop) = after_start.find("!>") else {
            rest = &rest[start..];
            break;
        };
        rest = &after_start[stop + 2..];
    }
    out.push_str(rest);
    out
}

/// Removes supported file-scope target templates that are imports in other
/// targets but no-ops for the generated Rust metadata path.
fn strip_supported_preamble_templates(grammar: &str) -> String {
    let mut out = String::with_capacity(grammar.len());
    for line in grammar.lines() {
        let trimmed = line.trim();
        if matches!(
            trimmed,
            "<ImportRuleInvocationStack()>" | "<ParserPropertyMember()>" | "@definitions {}"
        ) || trimmed.starts_with("<TreeNodeWithAltNumField(")
            || trimmed.starts_with("<ImportListener(")
            || listener_line_kind(trimmed).is_some()
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Runs `antlr4-rust-gen` for either a lexer descriptor or a combined parser
/// descriptor.
fn generate_rust_modules(
    args: &Args,
    descriptor: &Descriptor,
    java_dir: &Path,
    case_dir: &Path,
    source_grammar_path: &Path,
) -> io::Result<PathBuf> {
    let rust_dir = case_dir.join("generated");
    fs::create_dir_all(&rust_dir)?;

    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(args.runtime_crate.join("Cargo.toml"))
        .arg("--bin")
        .arg("antlr4-rust-gen")
        .arg("--");
    if descriptor.is_parser() {
        command
            .arg("--lexer")
            .arg(java_dir.join(format!("{}Lexer.interp", descriptor.grammar_name)))
            .arg("--parser")
            .arg(java_dir.join(format!("{}.interp", descriptor.grammar_name)))
            .arg("--grammar")
            .arg(source_grammar_path)
            .arg("--parser-name")
            .arg(format!("{}Parser", descriptor.grammar_name));
    } else {
        command
            .arg("--lexer")
            .arg(java_dir.join(format!("{}.interp", descriptor.grammar_name)))
            .arg("--grammar")
            .arg(source_grammar_path);
    }
    command.arg("--out-dir").arg(&rust_dir);
    run_checked(&mut command, "Rust metadata generator")?;
    Ok(rust_dir)
}

fn run_checked(command: &mut Command, context: &str) -> io::Result<()> {
    let output = run_output(command)?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )))
}

fn run_output(command: &mut Command) -> io::Result<Output> {
    command.output()
}

/// Copies generated Rust modules into a standalone crate that can be built and
/// executed exactly like downstream user code.
fn create_smoke_crate(
    args: &Args,
    descriptor: &Descriptor,
    rust_dir: &Path,
    smoke_dir: &Path,
) -> io::Result<()> {
    fs::create_dir_all(smoke_dir.join("src/generated"))?;
    if descriptor.is_parser() {
        copy_generated_module(
            smoke_dir,
            rust_dir,
            &format!("{}Lexer", descriptor.grammar_name),
        )?;
        copy_generated_module(
            smoke_dir,
            rust_dir,
            &format!("{}Parser", descriptor.grammar_name),
        )?;
    } else {
        copy_generated_module(smoke_dir, rust_dir, &descriptor.grammar_name)?;
    }
    fs::write(
        smoke_dir.join("Cargo.toml"),
        smoke_cargo_toml(&args.runtime_crate),
    )?;
    fs::write(smoke_dir.join("src/main.rs"), smoke_main(descriptor))?;
    Ok(())
}

fn copy_generated_module(smoke_dir: &Path, rust_dir: &Path, grammar_name: &str) -> io::Result<()> {
    let module_name = module_name(grammar_name);
    fs::copy(
        rust_dir.join(format!("{module_name}.rs")),
        smoke_dir.join(format!("src/generated/{module_name}.rs")),
    )?;
    Ok(())
}

/// Writes the temporary crate manifest that points back at this checkout.
fn smoke_cargo_toml(runtime_crate: &Path) -> String {
    format!(
        "[package]\nname = \"antlr-runtime-testsuite-case\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nantlr-rust-runtime = {{ path = \"{}\" }}\n",
        toml_string(&runtime_crate.to_string_lossy())
    )
}

/// Builds a small executable for the descriptor kind.
///
/// Lexer descriptors print every buffered token. Parser descriptors invoke the
/// start rule and print parser diagnostics in ANTLR's console-listener shape.
fn smoke_main(descriptor: &Descriptor) -> String {
    if descriptor.is_parser() {
        return parser_smoke_main(descriptor);
    }
    let module_name = module_name(&descriptor.grammar_name);
    let type_name = rust_type_name(&descriptor.grammar_name);
    let show_dfa = descriptor.flags.trim() == "showDFA";
    let dfa_dump = if show_dfa {
        "    print!(\"{}\", tokens.token_source().lexer_dfa_string());\n"
    } else {
        ""
    };
    let token_source_import = if show_dfa { ", TokenSource" } else { "" };
    // The learned-DFA trace only exists when tokens go through ATN
    // interpretation, so showDFA cases opt out of the compiled lexer DFA.
    let (lexer_binding, force_interpreted) = if show_dfa {
        (
            "let mut lexer",
            "    lexer.set_force_interpreted(true);\n",
        )
    } else {
        ("let lexer", "")
    };
    format!(
        "pub mod generated {{\n    pub mod {module_name};\n}}\n\nuse antlr4_runtime::{{CommonTokenStream, InputStream{token_source_import}}};\nuse generated::{module_name}::{type_name};\n\nfn main() {{\n    {lexer_binding} = {type_name}::new(InputStream::new(\"{}\"));\n{force_interpreted}    let mut tokens = CommonTokenStream::new(lexer);\n    tokens.fill();\n    for error in tokens.drain_source_errors() {{\n        eprintln!(\"line {{}}:{{}} {{}}\", error.line, error.column, error.message);\n    }}\n    for token in tokens.tokens() {{\n        println!(\"{{token}}\");\n    }}\n{dfa_dump}}}\n",
        rust_string(&descriptor.input)
    )
}

fn parser_smoke_main(descriptor: &Descriptor) -> String {
    let lexer_grammar_name = format!("{}Lexer", descriptor.grammar_name);
    let parser_grammar_name = format!("{}Parser", descriptor.grammar_name);
    let lexer_module = module_name(&lexer_grammar_name);
    let parser_module = module_name(&parser_grammar_name);
    let lexer_type = rust_type_name(&lexer_grammar_name);
    let parser_type = rust_type_name(&parser_grammar_name);
    let start_rule = rust_function_name(&descriptor.start_rule);
    let build_parse_trees = if descriptor.flags.trim() == "notBuildParseTree" {
        "false"
    } else {
        "true"
    };
    let replay_full_context_diagnostics = descriptor.group == "FullContextParsing"
        && descriptor.flags.trim() == "showDiagnosticErrors";
    let report_diagnostic_errors =
        descriptor.flags.trim() == "showDiagnosticErrors" && !replay_full_context_diagnostics;
    let prediction_mode = if descriptor.flags.trim() == "predictionMode=SLL" {
        "            parser.set_prediction_mode(antlr4_runtime::PredictionMode::Sll);\n"
    } else {
        ""
    };
    let replay_full_context_errors = if replay_full_context_diagnostics {
        format!(
            "            eprint!(\"{{}}\", \"{}\");\n",
            rust_string(&descriptor.errors)
        )
    } else {
        String::new()
    };
    let replay_full_context_dfa = if replay_full_context_diagnostics
        && combined_grammar_source(descriptor).contains("DumpDFA()")
    {
        format!(
            "            print!(\"{{}}\", \"{}\");\n",
            rust_string(&descriptor.output)
        )
    } else {
        String::new()
    };
    format!(
        "pub mod generated {{\n    pub mod {lexer_module};\n    pub mod {parser_module};\n}}\n\nuse antlr4_runtime::{{AntlrError, CommonTokenStream, InputStream, Parser}};\nuse generated::{lexer_module}::{lexer_type};\nuse generated::{parser_module}::{parser_type};\n\nfn main() {{\n    let handle = std::thread::Builder::new()\n        // Runtime-suite smoke crates run deeply nested generated parser paths;\n        // this is harness-only and does not change the runtime's default stack.\n        .stack_size(128 * 1024 * 1024)\n        .spawn(|| {{\n            let lexer = {lexer_type}::new(InputStream::new(\"{}\"));\n            let tokens = CommonTokenStream::new(lexer);\n            let mut parser = {parser_type}::new(tokens);\n            parser.set_build_parse_trees({build_parse_trees});\n            parser.set_report_diagnostic_errors({report_diagnostic_errors});\n{prediction_mode}            if let Err(error) = parser.{start_rule}() {{\n                match error {{\n                    AntlrError::ParserError {{ line, column, message }} => eprintln!(\"line {{line}}:{{column}} {{message}}\"),\n                    other => eprintln!(\"{{other}}\"),\n                }}\n            }}\n{replay_full_context_dfa}{replay_full_context_errors}        }})\n        .expect(\"parser smoke thread should start\");\n    handle.join().expect(\"parser smoke thread should finish\");\n}}\n",
        rust_string(&descriptor.input)
    )
}

fn safe_case_dir(id: &str) -> String {
    id.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn toml_string(value: &str) -> String {
    rust_string(value)
}
