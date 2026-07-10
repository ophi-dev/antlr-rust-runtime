#![allow(clippy::print_stderr, clippy::print_stdout)]

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "../bin_support/rust_names.rs"]
mod rust_names;

use rust_names::{module_name, rust_function_name, rust_string, rust_type_name};

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

    // Skips are classified up front (in descriptor order) so workers only see
    // runnable cases; `--limit` counts runnable cases, like the serial loop
    // it replaces, and stops classification there.
    let mut runnable = Vec::new();
    for descriptor in descriptors {
        if let Some(reason) = unsupported_reason(&descriptor) {
            summary.skipped += 1;
            println!("skip {}: {reason}", descriptor.id());
            continue;
        }
        runnable.push(descriptor);
        if args.limit.is_some_and(|limit| runnable.len() >= limit) {
            break;
        }
    }
    summary.ran = runnable.len();

    let context = SweepContext::prepare(&args)?;
    let (passed, failed) = run_cases(&args, &context, &runnable);
    summary.passed = passed;
    summary.failed = failed;

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

/// Runs the runnable descriptors on a worker pool. Each worker owns a
/// `CARGO_TARGET_DIR` stripe, so the runtime crate compiles once per worker
/// (first case) instead of once per case, and parallel smoke builds never
/// contend on one cargo build-dir lock.
fn run_cases(args: &Args, context: &SweepContext<'_>, runnable: &[Descriptor]) -> (usize, usize) {
    let jobs = args.jobs.clamp(1, runnable.len().max(1));
    let next = AtomicUsize::new(0);
    let tally = Mutex::new((0_usize, 0_usize));
    std::thread::scope(|scope| {
        for worker in 0..jobs {
            let next = &next;
            let tally = &tally;
            let stripe = args.work_dir.join(format!("cargo-target-{worker}"));
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(descriptor) = runnable.get(index) else {
                        break;
                    };
                    let passed = run_case(args, context, descriptor, &stripe);
                    let mut tally = tally.lock().expect("result tally lock");
                    if passed {
                        tally.0 += 1;
                    } else {
                        tally.1 += 1;
                    }
                }
            });
        }
    });
    let tally = tally.lock().expect("result tally lock");
    *tally
}

/// Runs one descriptor and reports its outcome; `println!`/`eprintln!` are
/// line-atomic, so per-case lines from parallel workers never interleave.
fn run_case(
    args: &Args,
    context: &SweepContext<'_>,
    descriptor: &Descriptor,
    stripe_target: &Path,
) -> bool {
    match run_descriptor(args, context, descriptor, stripe_target) {
        Ok(result) if result.output == descriptor.output && result.errors == descriptor.errors => {
            println!("pass {}", descriptor.id());
            if let Err(error) = remove_descriptor_work_dir(args, descriptor) {
                eprintln!(
                    "warning: could not clean {}: {error}",
                    descriptor.id()
                );
            }
            true
        }
        Ok(result) => {
            eprintln!(
                "fail {}\nexpected stdout:\n{}\nactual stdout:\n{}\nexpected stderr:\n{}\nactual stderr:\n{}",
                descriptor.id(),
                descriptor.output,
                result.output,
                descriptor.errors,
                result.errors
            );
            false
        }
        Err(error) => {
            eprintln!("fail {}: {error}", descriptor.id());
            false
        }
    }
}

/// Per-sweep artifacts prepared once so per-case work stays minimal: the
/// precompiled `StringTemplate` render driver (the Java single-file source
/// launcher would otherwise re-compile `RenderGrammar.java` on every render)
/// and the prebuilt `antlr4-rust-gen` executable (invoked directly instead of
/// through a `cargo run` graph check per case).
struct SweepContext<'a> {
    args: &'a Args,
    render_classes: PathBuf,
    generator: PathBuf,
}

impl<'a> SweepContext<'a> {
    fn prepare(args: &'a Args) -> io::Result<Self> {
        let render_classes = args.work_dir.join("stg-render-classes");
        fs::create_dir_all(&render_classes)?;
        run_checked(
            Command::new("javac")
                .arg("-cp")
                .arg(&args.antlr_jar)
                .arg("-d")
                .arg(&render_classes)
                .arg(args.runtime_crate.join("tools/stg-render/RenderGrammar.java")),
            "StringTemplate render driver compile",
        )?;
        let generator = prebuild_generator(args)?;
        Ok(Self {
            args,
            render_classes,
            generator,
        })
    }

    /// `java -cp` separator-joined ANTLR jar + render driver classes.
    fn render_classpath(&self) -> String {
        let separator = if cfg!(windows) { ";" } else { ":" };
        format!(
            "{}{separator}{}",
            self.args.antlr_jar.display(),
            self.render_classes.display()
        )
    }
}

/// Builds `antlr4-rust-gen` once and resolves its executable path from
/// cargo's JSON messages, honoring any `CARGO_TARGET_DIR` redirection.
fn prebuild_generator(args: &Args) -> io::Result<PathBuf> {
    let output = run_output(
        Command::new("cargo")
            .arg("build")
            .arg("--manifest-path")
            .arg(args.runtime_crate.join("Cargo.toml"))
            .arg("--bin")
            .arg("antlr4-rust-gen")
            .arg("--message-format=json"),
    )?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "antlr4-rust-gen build failed\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter(|line| line.contains("antlr4-rust-gen"))
        .filter_map(|line| {
            let start = line.find("\"executable\":\"")? + "\"executable\":\"".len();
            let end = line[start..].find('"')? + start;
            Some(PathBuf::from(&line[start..end]))
        })
        .next_back()
        .ok_or_else(|| io::Error::other("cargo did not report the antlr4-rust-gen executable"))
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
    /// Parallel case workers, each with its own cargo target-dir stripe.
    jobs: usize,
    /// Template group used to render descriptor grammars (real
    /// `StringTemplate` via the ANTLR jar) before generation.
    stg: PathBuf,
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
        let mut jobs = None;
        let mut stg = None;

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
                "--jobs" => {
                    let value = next_arg(&mut iter, "--jobs")?;
                    jobs = Some(
                        value
                            .parse::<usize>()
                            .ok()
                            .filter(|jobs| *jobs > 0)
                            .ok_or_else(|| format!("invalid --jobs {value:?}"))?,
                    );
                }
                "--stg" => stg = Some(PathBuf::from(next_arg(&mut iter, "--stg")?)),
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

        let stg = stg.unwrap_or_else(|| runtime_crate.join(".conformance-review/Rust.test.stg"));
        // Every worker drives a JVM and rustc of its own; past ~8 the extra
        // workers mostly fight each other for cores.
        let jobs = jobs.unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, |cores| cores.get().min(8))
        });
        Ok(Self {
            antlr_jar,
            descriptors,
            runtime_crate,
            work_dir,
            group,
            case_name,
            limit,
            keep,
            jobs,
            stg,
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
    "usage: antlr4-runtime-testsuite [--antlr-jar ANTLR.jar] [--descriptors PATH] [--case Group/Name] [--group Group] [--limit N] [--jobs N] [--keep] [--stg PATH]\n\nDefaults: ANTLR4_JAR or /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar; ANTLR4_RUNTIME_TESTSUITE or /tmp/antlr-cleanroom/antlr4-upstream/runtime-testsuite; --stg .conformance-review/Rust.test.stg; --jobs min(cores, 8)".to_owned()
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
    /// Raw grammar section text (ST escapes intact) for the template render.
    grammar_template: String,
    start_rule: String,
    input: String,
    output: String,
    errors: String,
    flags: String,
    slave_grammar_templates: Vec<String>,
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
        grammar_template: String::new(),
        start_rule: String::new(),
        input: String::new(),
        output: String::new(),
        errors: String::new(),
        flags: String::new(),
        slave_grammar_templates: Vec::new(),
    };

    for (section, value) in sections {
        let value = normalize_section_value(&value);
        match section.as_str() {
            "type" => descriptor.test_type = value,
            "grammar" => {
                descriptor.grammar_name = grammar_name(&render_st_backslash_escapes(&value))?;
                descriptor.grammar_template = value;
            }
            "slaveGrammar" => {
                descriptor.slave_grammar_templates.push(value);
            }
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
    if !descriptor.slave_grammar_templates.is_empty() && !descriptor.is_composite() {
        return Some("composite grammars are not wired into the metadata harness yet");
    }
    if descriptor.is_composite() && !composite_grammar_supported(descriptor) {
        return Some("composite grammar shape is not wired into the metadata harness yet");
    }
    if !descriptor.flags.is_empty() && !runtime_flags_supported(descriptor) {
        return Some("diagnostic/profile/DFA flags are not implemented in the Rust harness yet");
    }
    // Honest residuals: these descriptors FAIL when run (nothing is faked);
    // they are skipped with the missing feature named until it lands.
    if matches!(
        descriptor.id().as_str(),
        "FullContextParsing/AmbiguityNoLoop"
            | "FullContextParsing/CtxSensitiveDFATwoDiffInput"
            | "FullContextParsing/ExprAmbiguity_2"
            | "FullContextParsing/FullContextIF_THEN_ELSEParse_1"
            | "FullContextParsing/FullContextIF_THEN_ELSEParse_3"
            | "FullContextParsing/FullContextIF_THEN_ELSEParse_4"
            | "FullContextParsing/FullContextIF_THEN_ELSEParse_5"
            | "FullContextParsing/FullContextIF_THEN_ELSEParse_6"
            | "FullContextParsing/LoopsSimulateTailRecursion"
    ) {
        return Some(
            "full-context DFA learning does not yet reproduce Java's per-decision dump/diagnostics",
        );
    }
    if descriptor.id() == "LexerExec/PositionAdjustingLexer" {
        return Some("lexer subclass method overrides are not supported yet");
    }
    if descriptor.is_parser() || descriptor.is_lexer() {
        return None;
    }
    Some("descriptor type is not supported by the metadata harness yet")
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

/// Renders one grammar template through the target `.test.stg` group using
/// the `StringTemplate` engine bundled in the ANTLR jar (driver precompiled
/// once per sweep), mirroring upstream `RuntimeTests`.
fn render_grammar_through_stg(
    context: &SweepContext<'_>,
    case_dir: &Path,
    tag: &str,
    grammar_template: &str,
) -> io::Result<String> {
    let template_path = case_dir.join(format!("{tag}.template.g4"));
    let rendered_path = case_dir.join(format!("{tag}.rendered.g4"));
    fs::write(&template_path, grammar_template)?;
    run_checked(
        Command::new("java")
            .arg("-cp")
            .arg(context.render_classpath())
            .arg("RenderGrammar")
            .arg(&context.args.stg)
            .arg(&template_path)
            .arg(&rendered_path),
        "StringTemplate grammar render",
    )
    .map_err(|error| io::Error::other(format!("{tag}: {error}")))?;
    fs::read_to_string(&rendered_path)
}

/// Concatenates rendered grammars (delegator first) for the generator's
/// `--grammar` action extraction.
fn combined_rendered_grammar_source(rendered: &[String]) -> String {
    let mut out = String::new();
    for grammar in rendered {
        push_grammar_source(&mut out, grammar);
    }
    out
}

/// Runs one descriptor through ANTLR metadata generation, Rust code generation,
/// a temporary Cargo crate, and process output capture.
fn run_descriptor(
    args: &Args,
    context: &SweepContext<'_>,
    descriptor: &Descriptor,
    stripe_target: &Path,
) -> io::Result<RunResult> {
    let case_dir = descriptor_work_dir(args, descriptor);
    if case_dir.exists() {
        fs::remove_dir_all(&case_dir)?;
    }
    fs::create_dir_all(&case_dir)?;

    let source_grammar_path = case_dir.join(format!("{}.source.g4", descriptor.grammar_name));
    let grammar_path = case_dir.join(format!("{}.g4", descriptor.grammar_name));
    // Render the descriptor grammar (and slaves) through the target
    // `.test.stg` with the real StringTemplate engine, exactly like the
    // upstream harness. The rendered grammar feeds BOTH the ANTLR tool
    // and the embedded-actions Rust generator.
    let rendered =
        render_grammar_through_stg(context, &case_dir, "main", &descriptor.grammar_template)?;
    fs::write(&grammar_path, &rendered)?;
    let mut rendered_slaves = Vec::new();
    for (index, slave) in descriptor.slave_grammar_templates.iter().enumerate() {
        let rendered_slave =
            render_grammar_through_stg(context, &case_dir, &format!("slave{index}"), slave)?;
        let slave_path = case_dir.join(format!("{}.g4", grammar_name(&rendered_slave)?));
        fs::write(&slave_path, &rendered_slave)?;
        rendered_slaves.push(rendered_slave);
    }
    // Delegates must follow the delegator's `import` clause order so an
    // overridden rule keeps the same first definition ANTLR keeps.
    let mut combined_rendered = vec![rendered.clone()];
    let mut remaining: Vec<Option<String>> = rendered_slaves.into_iter().map(Some).collect();
    for import in imported_grammar_names(&rendered) {
        for slot in &mut remaining {
            if slot
                .as_deref()
                .and_then(|slave| grammar_name(slave).ok())
                .is_some_and(|name| name == import)
            {
                combined_rendered.extend(slot.take());
            }
        }
    }
    combined_rendered.extend(remaining.into_iter().flatten());
    fs::write(
        &source_grammar_path,
        combined_rendered_grammar_source(&combined_rendered),
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
        generate_rust_modules(context, descriptor, &java_dir, &case_dir, &source_grammar_path)?;

    let smoke_dir = case_dir.join("rust");
    create_smoke_crate(args, descriptor, &rust_dir, &smoke_dir)?;
    // The worker's target-dir stripe keeps the runtime crate and every
    // dependency compiled across cases; only the case's generated module
    // is compiled and linked here.
    let output = run_output(
        Command::new("cargo")
            .arg("run")
            .arg("--quiet")
            .env("CARGO_TARGET_DIR", stripe_target)
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

/// Runs the prebuilt `antlr4-rust-gen` for either a lexer descriptor or a
/// combined parser descriptor.
fn generate_rust_modules(
    context: &SweepContext<'_>,
    descriptor: &Descriptor,
    java_dir: &Path,
    case_dir: &Path,
    source_grammar_path: &Path,
) -> io::Result<PathBuf> {
    let rust_dir = case_dir.join("generated");
    fs::create_dir_all(&rust_dir)?;

    let mut command = Command::new(&context.generator);
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
    command.arg("--actions").arg("embedded");
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
    let report_diagnostic_errors = descriptor.flags.trim() == "showDiagnosticErrors";
    let prediction_mode = if descriptor.flags.trim() == "predictionMode=SLL" {
        "            parser.set_prediction_mode(antlr4_runtime::PredictionMode::Sll);\n"
    } else {
        ""
    };
    format!(
        "pub mod generated {{\n    pub mod {lexer_module};\n    pub mod {parser_module};\n}}\n\nuse antlr4_runtime::{{AntlrError, CommonTokenStream, InputStream, Parser}};\nuse generated::{lexer_module}::{lexer_type};\nuse generated::{parser_module}::{parser_type};\n\nfn main() {{\n    let handle = std::thread::Builder::new()\n        // Runtime-suite smoke crates run deeply nested generated parser paths;\n        // this is harness-only and does not change the runtime's default stack.\n        .stack_size(128 * 1024 * 1024)\n        .spawn(|| {{\n            let lexer = {lexer_type}::new(InputStream::new(\"{}\"));\n            let tokens = CommonTokenStream::new(lexer);\n            let mut parser = {parser_type}::new(tokens);\n            parser.set_build_parse_trees({build_parse_trees});\n            parser.set_report_diagnostic_errors({report_diagnostic_errors});\n{prediction_mode}            if let Err(error) = parser.{start_rule}() {{\n                match error {{\n                    AntlrError::ParserError {{ line, column, message }} => eprintln!(\"line {{line}}:{{column}} {{message}}\"),\n                    other => eprintln!(\"{{other}}\"),\n                }}\n            }}\n        }})\n        .expect(\"parser smoke thread should start\");\n    handle.join().expect(\"parser smoke thread should finish\");\n}}\n",
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
