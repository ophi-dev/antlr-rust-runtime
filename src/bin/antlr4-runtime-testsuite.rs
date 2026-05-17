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
            "notes" | "skip" | "start" => {}
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
        out.push(ch);
        if ch == '\\' && chars.peek() == Some(&'\\') {
            chars.next();
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
    if descriptor.test_type != "Lexer" {
        return Some("metadata harness currently executes lexer descriptors only");
    }
    if !descriptor.slave_grammars.is_empty() {
        return Some("composite grammars are not wired into the metadata harness yet");
    }
    if !descriptor.flags.is_empty() {
        return Some("diagnostic/profile/DFA flags are not implemented in the Rust harness yet");
    }
    if descriptor.grammar.contains("{<")
        || descriptor.grammar.contains("<writeln")
        || descriptor.grammar.contains("@members")
        || descriptor.grammar.contains("@definitions")
    {
        return Some("target-template semantic actions are not rendered by this harness yet");
    }
    None
}

/// Runs one descriptor through ANTLR metadata generation, Rust code generation,
/// a temporary Cargo crate, and process output capture.
fn run_descriptor(args: &Args, descriptor: &Descriptor) -> io::Result<RunResult> {
    let case_dir = args.work_dir.join(safe_case_dir(&descriptor.id()));
    if case_dir.exists() {
        fs::remove_dir_all(&case_dir)?;
    }
    fs::create_dir_all(&case_dir)?;

    let grammar_path = case_dir.join(format!("{}.g4", descriptor.grammar_name));
    fs::write(&grammar_path, &descriptor.grammar)?;

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

    let rust_dir = case_dir.join("generated");
    fs::create_dir_all(&rust_dir)?;
    let interp_path = java_dir.join(format!("{}.interp", descriptor.grammar_name));
    run_checked(
        Command::new("cargo")
            .arg("run")
            .arg("--quiet")
            .arg("--manifest-path")
            .arg(args.runtime_crate.join("Cargo.toml"))
            .arg("--bin")
            .arg("antlr4-rust-gen")
            .arg("--")
            .arg("--lexer")
            .arg(&interp_path)
            .arg("--out-dir")
            .arg(&rust_dir),
        "Rust metadata generator",
    )?;

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
    let module_name = module_name(&descriptor.grammar_name);
    fs::copy(
        rust_dir.join(format!("{module_name}.rs")),
        smoke_dir.join(format!("src/generated/{module_name}.rs")),
    )?;
    fs::write(
        smoke_dir.join("Cargo.toml"),
        smoke_cargo_toml(&args.runtime_crate),
    )?;
    fs::write(
        smoke_dir.join("src/main.rs"),
        smoke_main(&descriptor.grammar_name, &descriptor.input),
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

/// Builds a small executable that lexes the descriptor input and prints every
/// buffered token using `CommonToken`'s ANTLR-compatible display format.
fn smoke_main(grammar_name: &str, input: &str) -> String {
    let module_name = module_name(grammar_name);
    let type_name = rust_type_name(grammar_name);
    format!(
        "pub mod generated {{\n    pub mod {module_name};\n}}\n\nuse antlr4_runtime::{{CommonTokenStream, InputStream}};\nuse generated::{module_name}::{type_name};\n\nfn main() {{\n    let lexer = {type_name}::new(InputStream::new(\"{}\"));\n    let mut tokens = CommonTokenStream::new(lexer);\n    tokens.fill();\n    for token in tokens.tokens() {{\n        println!(\"{{token}}\");\n    }}\n}}\n",
        rust_string(input)
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

fn rust_string(value: &str) -> String {
    value.escape_default().to_string()
}

fn toml_string(value: &str) -> String {
    rust_string(value)
}
