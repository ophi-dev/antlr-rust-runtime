#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DESCRIPTOR_PATH: &str = "resources/org/antlr/v4/test/runtime/descriptors";

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

        Ok(Self {
            antlr_jar: antlr_jar.ok_or_else(usage)?,
            descriptors: descriptors.ok_or_else(usage)?,
            runtime_crate,
            work_dir,
            group,
            case_name,
            limit,
            keep,
        })
    }
}

fn next_arg(iter: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value\n\n{}", usage()))
}

fn usage() -> String {
    "usage: antlr4-runtime-testsuite --antlr-jar ANTLR.jar --descriptors PATH [--case Group/Name] [--group Group] [--limit N] [--keep]".to_owned()
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
    if !descriptor.slave_grammars.is_empty() {
        return Some("composite grammars are not wired into the metadata harness yet");
    }
    if !descriptor.flags.is_empty() && descriptor.flags.trim() != "notBuildParseTree" {
        return Some("diagnostic/profile/DFA flags are not implemented in the Rust harness yet");
    }
    if has_target_template(&descriptor.grammar) && !target_templates_supported(descriptor) {
        return Some("target-template semantic actions are not rendered by this harness yet");
    }
    if descriptor.test_type == "Parser" {
        if !descriptor.output.is_empty() {
            if !target_templates_supported(descriptor) {
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
    if descriptor.test_type != "Lexer" {
        return Some("descriptor type is not supported by the metadata harness yet");
    }
    None
}

/// Admits only parser-error descriptors covered by the current mismatch and
/// single-token recovery diagnostics, leaving mixed lexer/parser diagnostic
/// ordering cases skipped.
fn parser_error_diagnostics_supported(descriptor: &Descriptor) -> bool {
    matches!(
        descriptor.name.as_str(),
        "ConjuringUpToken"
            | "ConjuringUpTokenFromSet"
            | "InvalidEmptyInput"
            | "SingleSetInsertion"
            | "SingleSetInsertionConsumption"
            | "SingleTokenDeletion"
            | "SingleTokenDeletionBeforeAlt"
            | "SingleTokenDeletionBeforePredict"
            | "SingleTokenDeletionConsumption"
            | "SingleTokenDeletionDuringLoop"
            | "SingleTokenDeletionExpectingSet"
            | "SingleTokenInsertion"
            | "TokenMismatch"
            | "TokenMismatch2"
            | "TokenMismatch3"
    )
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

fn target_templates_supported(descriptor: &Descriptor) -> bool {
    if descriptor.test_type == "Lexer" {
        return lexer_target_templates_supported(descriptor);
    }
    if descriptor.test_type != "Parser" {
        return false;
    }
    if matches!(
        descriptor.name.as_str(),
        "IfIfElseGreedyBinding1" | "IfIfElseGreedyBinding2" | "Order" | "RewindBeforePredEval"
    ) {
        return false;
    }
    let grammar = &descriptor.grammar;
    if unsupported_members_templates(grammar)
        || grammar.contains("@definitions")
        || !supported_signature_templates(grammar)
        || grammar.contains("<LANotEquals")
        || grammar.contains("<AppendStr")
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

fn lexer_target_templates_supported(descriptor: &Descriptor) -> bool {
    let grammar = &descriptor.grammar;
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
    while let Some(block) = next_template_block(grammar, offset) {
        offset = block.after_brace;
        if block.predicate
            || is_after_action(grammar, block.open_brace)
            || is_init_action(grammar, block.open_brace)
            || is_definitions_action(grammar, block.open_brace)
            || is_members_action(grammar, block.open_brace)
        {
            continue;
        }
        if !is_supported_action_template(block.body.trim()) {
            return false;
        }
    }
    true
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
            "BuildParseTrees()" | "BailErrorStrategy()" | "GetExpectedTokenNames():writeln()"
        ) {
            return false;
        }
    }
    saw_init_action
}

fn supported_after_action_templates(grammar: &str) -> bool {
    let mut saw_after_action = false;
    let mut offset = 0;
    while let Some(block) = next_template_block(grammar, offset) {
        offset = block.after_brace;
        if block.predicate || !is_after_action(grammar, block.open_brace) {
            continue;
        }
        saw_after_action = true;
        let body = block.body.trim();
        if is_string_tree_label_template(body) {
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
    while let Some(block) = next_template_block(grammar, offset) {
        offset = block.after_brace;
        if block.predicate && !is_supported_lexer_predicate_template(block.body.trim()) {
            return false;
        }
    }
    true
}

fn is_supported_lexer_predicate_template(body: &str) -> bool {
    matches!(body, "True()" | "False()")
        || body
            .strip_prefix("TextEquals(")
            .and_then(|value| value.strip_suffix(')'))
            .is_some_and(|argument| parse_template_string(argument).is_some())
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
            | r#"ToStringTree("$ctx"):writeln()"#
            | r#"ToStringTree("$ctx"):write()"#
    ) || body.starts_with("writeln(\"\\\"")
        || body.starts_with("write(\"\\\"")
        || is_noop_action_template(body)
        || is_token_text_template(body)
        || is_token_display_template(body)
        || (body.starts_with("PlusText(\"") && body.ends_with("):writeln()"))
        || (body.starts_with("PlusText(\"") && body.ends_with("):write()"))
}

fn supported_signature_templates(grammar: &str) -> bool {
    grammar.lines().all(|line| {
        supported_signature_template_on_line(line, "returns [")
            && supported_signature_template_on_line(line, "locals [")
    })
}

fn supported_signature_template_on_line(line: &str, marker: &str) -> bool {
    let Some(marker_start) = line.find(marker) else {
        return true;
    };
    let template_start = marker_start + marker.len();
    let Some(template) = line[template_start..].trim().strip_prefix('<') else {
        return true;
    };
    template
        .strip_suffix(']')
        .and_then(|value| value.strip_suffix('>'))
        .is_some_and(|body| body.starts_with("IntArg(") && body.ends_with(')'))
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
        if block.body.trim() != "DeclareContextListGettersFunction()" {
            return true;
        }
        saw_supported = true;
    }
    !saw_supported
}

fn is_noop_action_template(body: &str) -> bool {
    (body.starts_with("AssignLocal(")
        || body.starts_with("AssertIsList(")
        || body.starts_with("IntArg("))
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

/// Splits a `StringTemplate` argument list while ignoring nested expressions.
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

/// Runs one descriptor through ANTLR metadata generation, Rust code generation,
/// a temporary Cargo crate, and process output capture.
fn run_descriptor(args: &Args, descriptor: &Descriptor) -> io::Result<RunResult> {
    let case_dir = args.work_dir.join(safe_case_dir(&descriptor.id()));
    if case_dir.exists() {
        fs::remove_dir_all(&case_dir)?;
    }
    fs::create_dir_all(&case_dir)?;

    let source_grammar_path = case_dir.join(format!("{}.source.g4", descriptor.grammar_name));
    fs::write(&source_grammar_path, &descriptor.grammar)?;
    let grammar_path = case_dir.join(format!("{}.g4", descriptor.grammar_name));
    fs::write(
        &grammar_path,
        render_target_templates_for_metadata(&descriptor.grammar),
    )?;

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

/// Replaces target-template actions with neutral ANTLR actions before invoking
/// the official tool for `.interp` metadata.
///
/// The original grammar is still passed to `antlr4-rust-gen`, which replays the
/// supported templates from Rust after the ATN path has been selected.
fn render_target_templates_for_metadata(grammar: &str) -> String {
    let mut out = String::with_capacity(grammar.len());
    let mut offset = 0;
    while let Some(block) = next_template_block(grammar, offset) {
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
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
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
        let open_brace = cursor + open_rel;
        let template_start = skip_ascii_whitespace(source, open_brace + 1);
        if source.as_bytes().get(template_start) != Some(&b'<') {
            cursor = open_brace + 1;
            continue;
        }
        let close_angle = matching_template_close(source, template_start + 1)?;
        let close_brace = skip_ascii_whitespace(source, close_angle + 1);
        if source.as_bytes().get(close_brace) != Some(&b'}') {
            cursor = open_brace + 1;
            continue;
        }
        let after_brace = close_brace + 1;
        return Some(TemplateBlock {
            open_brace,
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

/// Detects target member blocks that are compile-time scaffolding for other
/// runtimes and should not be counted as parser action transitions.
fn is_members_action(source: &str, open_brace: usize) -> bool {
    let prefix = source[..open_brace].trim_end();
    prefix.ends_with("@members") || prefix.ends_with("@parser::members")
}

fn is_definitions_action(source: &str, open_brace: usize) -> bool {
    source[..open_brace].trim_end().ends_with("@definitions")
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
    if descriptor.test_type == "Parser" {
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
    if descriptor.test_type == "Parser" {
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
        "[package]\nname = \"antlr-runtime-testsuite-case\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nantlr4-runtime-rs = {{ path = \"{}\" }}\n",
        toml_string(&runtime_crate.to_string_lossy())
    )
}

/// Builds a small executable for the descriptor kind.
///
/// Lexer descriptors print every buffered token. Parser descriptors invoke the
/// start rule and print parser diagnostics in ANTLR's console-listener shape.
fn smoke_main(descriptor: &Descriptor) -> String {
    if descriptor.test_type == "Parser" {
        return parser_smoke_main(descriptor);
    }
    let module_name = module_name(&descriptor.grammar_name);
    let type_name = rust_type_name(&descriptor.grammar_name);
    format!(
        "pub mod generated {{\n    pub mod {module_name};\n}}\n\nuse antlr4_runtime::{{CommonTokenStream, InputStream}};\nuse generated::{module_name}::{type_name};\n\nfn main() {{\n    let lexer = {type_name}::new(InputStream::new(\"{}\"));\n    let mut tokens = CommonTokenStream::new(lexer);\n    tokens.fill();\n    for token in tokens.tokens() {{\n        println!(\"{{token}}\");\n    }}\n}}\n",
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
    format!(
        "pub mod generated {{\n    pub mod {lexer_module};\n    pub mod {parser_module};\n}}\n\nuse antlr4_runtime::{{AntlrError, CommonTokenStream, InputStream, Parser}};\nuse generated::{lexer_module}::{lexer_type};\nuse generated::{parser_module}::{parser_type};\n\nfn main() {{\n    let handle = std::thread::Builder::new()\n        .stack_size(128 * 1024 * 1024)\n        .spawn(|| {{\n            let lexer = {lexer_type}::new(InputStream::new(\"{}\"));\n            let tokens = CommonTokenStream::new(lexer);\n            let mut parser = {parser_type}::new(tokens);\n            parser.set_build_parse_trees({build_parse_trees});\n            if let Err(error) = parser.{start_rule}() {{\n                match error {{\n                    AntlrError::ParserError {{ line, column, message }} => eprintln!(\"line {{line}}:{{column}} {{message}}\"),\n                    other => eprintln!(\"{{other}}\"),\n                }}\n            }}\n        }})\n        .expect(\"parser smoke thread should start\");\n    handle.join().expect(\"parser smoke thread should finish\");\n}}\n",
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

fn module_name(name: &str) -> String {
    split_identifier_words(name).join("_")
}

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

/// Splits grammar identifiers the same way the metadata generator does so the
/// harness imports the generated module and type names correctly.
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

fn ascii_lowercase(value: &str) -> String {
    value.chars().map(|ch| ch.to_ascii_lowercase()).collect()
}

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

fn rust_string(value: &str) -> String {
    value.escape_default().to_string()
}

fn toml_string(value: &str) -> String {
    rust_string(value)
}
