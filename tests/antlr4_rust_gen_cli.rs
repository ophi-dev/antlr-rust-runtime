use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn run_antlr4_rust_gen(args: &[impl AsRef<OsStr>]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_antlr4-rust-gen"))
        .args(args)
        .output()
        .expect("antlr4-rust-gen should run")
}

fn assert_generated_modules_compile(temp_dir: &Path, modules: &[&str]) {
    assert_generated_project(temp_dir, modules, "");
}

fn assert_generated_project(temp_dir: &Path, modules: &[&str], test_source: &str) {
    let project = temp_dir.join("compile-generated");
    let source = project.join("src");
    fs::create_dir_all(&source).expect("generated-module check should be writable");
    fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\n\
             name = \"compile-generated\"\n\
             version = \"0.0.0\"\n\
             edition = \"2024\"\n\
             \n\
             [dependencies]\n\
             antlr-rust-runtime = {{ path = {:?} }}\n",
            env!("CARGO_MANIFEST_DIR")
        ),
    )
    .expect("generated-module manifest should be writable");
    let declarations = modules
        .iter()
        .map(|module| {
            let module_name = module.strip_suffix(".rs").unwrap_or(module);
            format!("#[path = {module:?}]\nmod {module_name};")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        source.join("lib.rs"),
        format!("{declarations}\n{test_source}"),
    )
    .expect("generated-module crate root should be writable");
    for module in modules {
        fs::copy(temp_dir.join("generated").join(module), source.join(module))
            .expect("generated module should be copied into the check crate");
    }

    let output = Command::new(env!("CARGO"))
        .args([
            if test_source.is_empty() {
                "check"
            } else {
                "test"
            },
            "--quiet",
            "--offline",
            "--manifest-path",
            project
                .join("Cargo.toml")
                .to_str()
                .expect("temporary path should be UTF-8"),
        ])
        .env("CARGO_TARGET_DIR", project.join("target"))
        .output()
        .expect("cargo check should run");
    assert!(
        output.status.success(),
        "generated project failed\nstdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
}

fn utf8(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("process output should be UTF-8")
}

fn temporary_directory(label: &str) -> TempDirectory {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "antlr4-rust-gen-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temporary directory should be writable");
    TempDirectory(path)
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn long_help_describes_source_only_cli() {
    let output = run_antlr4_rust_gen(&["--help"]);

    assert!(
        output.status.success(),
        "status: {:?}\nstderr: {}",
        output.status.code(),
        utf8(&output.stderr)
    );
    assert_eq!(utf8(&output.stderr), "");

    let stdout = utf8(&output.stdout);
    assert!(
        stdout.starts_with("Usage: antlr4-rust-gen [OPTIONS] ROOT.g4...\n"),
        "{stdout}"
    );
    assert!(stdout.contains("  -I, --lib DIR"), "{stdout}");
    assert!(stdout.contains("  --option-hook KEY=VALUE"), "{stdout}");
    assert!(stdout.contains("  -listener, --listener"), "{stdout}");
    assert!(stdout.contains("  -no-listener, --no-listener"), "{stdout}");
    assert!(stdout.contains("  -visitor, --visitor"), "{stdout}");
    assert!(stdout.contains("  -no-visitor, --no-visitor"), "{stdout}");
    assert!(!stdout.contains("--lexer "), "{stdout}");
    assert!(!stdout.contains("--parser "), "{stdout}");
    assert!(!stdout.contains("--grammar "), "{stdout}");
    assert!(stdout.contains("  -V, --version"), "{stdout}");
    assert!(stdout.contains("  -h, --help"), "{stdout}");
}

#[test]
fn short_help_exits_successfully_on_stdout() {
    let output = run_antlr4_rust_gen(&["-h"]);

    assert!(output.status.success(), "stderr: {}", utf8(&output.stderr));
    assert_eq!(utf8(&output.stderr), "");
    assert!(utf8(&output.stdout).contains("Usage: antlr4-rust-gen"));
}

#[test]
fn long_and_short_version_exit_successfully_on_stdout() {
    for flag in ["--version", "-V"] {
        let output = run_antlr4_rust_gen(&[flag]);

        assert!(
            output.status.success(),
            "{flag} status: {:?}\nstderr: {}",
            output.status.code(),
            utf8(&output.stderr)
        );
        assert_eq!(utf8(&output.stderr), "");
        assert_eq!(
            utf8(&output.stdout),
            concat!("antlr4-rust-gen ", env!("CARGO_PKG_VERSION"), "\n")
        );
    }
}

#[test]
fn help_flag_as_option_value_is_not_intercepted() {
    let output = run_antlr4_rust_gen(&["--option-hook", "--help"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("--option-hook requires KEY=VALUE"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn version_flags_as_option_values_are_not_intercepted() {
    for flag in ["--version", "-V"] {
        let output = run_antlr4_rust_gen(&["--option-hook", flag]);

        assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
        assert_eq!(utf8(&output.stdout), "");

        let stderr = utf8(&output.stderr);
        assert!(stderr.contains("--option-hook requires KEY=VALUE"));
        assert!(stderr.contains("Usage: antlr4-rust-gen"));
    }
}

#[test]
fn missing_roots_report_usage_on_stderr() {
    let args: [&str; 0] = [];
    let output = run_antlr4_rust_gen(&args);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("at least one grammar root is required"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn unknown_arguments_report_usage_on_stderr() {
    let output = run_antlr4_rust_gen(&["--bogus"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("unknown argument --bogus"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn legacy_interp_flags_are_rejected() {
    for flag in [
        "--lexer",
        "--parser",
        "--grammar",
        "--lexer-name",
        "--parser-name",
    ] {
        let output = run_antlr4_rust_gen(&[flag, "Legacy.interp"]);
        assert!(!output.status.success(), "{flag} unexpectedly succeeded");
        let stderr = utf8(&output.stderr);
        assert!(
            stderr.contains(&format!("unknown argument {flag}")),
            "{stderr}"
        );
    }
}

#[test]
fn option_hook_requires_a_key_value_assignment() {
    let output = run_antlr4_rust_gen(&["--option-hook", "superClass"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");
    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("--option-hook requires KEY=VALUE"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn positional_lexer_root_emits_rust_and_manifest() {
    let temp = temporary_directory("lexer");
    let grammar = temp.path().join("Letters.g4");
    let out = temp.path().join("generated");
    fs::write(&grammar, "lexer grammar Letters;\nA: 'a';\n").expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert!(out.join("letters.rs").is_file());
    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    assert!(manifest.contains("\"name\": \"Letters\""), "{manifest}");
    assert!(manifest.contains("\"kind\": \"lexer\""), "{manifest}");
}

#[test]
fn adaptive_atn_routing_generated_path_compiles() {
    let temp = temporary_directory("adaptive-atn-routing");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/adaptive-atn-routing/AdaptiveRouting.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let parser = fs::read_to_string(out.join("adaptive_routing_parser.rs"))
        .expect("parser should be emitted");
    assert!(
        parser.contains("adaptive_atn_preferred_rules"),
        "structural candidate should emit adaptive ATN routing"
    );
    assert!(
        parser.contains("_adaptive_probe_dispatch"),
        "left-recursive seed should probe its enclosing candidate"
    );
    assert_generated_modules_compile(
        temp.path(),
        &["adaptive_routing_lexer.rs", "adaptive_routing_parser.rs"],
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn unrecovered_generated_entry_errors_notify_listeners_once() {
    let temp = temporary_directory("fatal-error-listener");
    let grammar = temp.path().join("Fatal.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar Fatal;\nfatal: A (B B | C C);\nstart: child EOF;\nmixed: child A (B B | C C);\nsemantic_mixed: child semantic_child A (B B | C C);\nsemantic_child: {unsupported()}?;\nclean: A;\nchild: A (B B | C C);\nA: 'a';\nB: 'b';\nC: 'c';\nD: 'd';\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("templates"),
        OsStr::new("--sem-unknown"),
        OsStr::new("hook"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let test_source = r####"
#[cfg(test)]
mod fatal_error_listener_tests {
    use std::sync::{Arc, Mutex};

    use super::fatal_lexer::FatalLexer;
    use super::fatal_parser::FatalParser;
    use antlr4_runtime::{
        AntlrError, CommonTokenStream, ErrorListener, InputStream, Parser as _, Recognizer,
        TokenView,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Event {
        offending_text: Option<String>,
        line: usize,
        column: usize,
        message: String,
        error: Option<AntlrError>,
    }

    #[allow(dead_code)]
    #[derive(Debug)]
    struct EntrySnapshot<'a> {
        returned_error: &'a AntlrError,
        syntax_errors: usize,
        events: &'a [Event],
    }

    #[derive(Clone, Debug)]
    struct RecordingListener {
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl<R> ErrorListener<R> for RecordingListener
    where
        R: Recognizer + ?Sized,
    {
        fn syntax_error(
            &mut self,
            _recognizer: &R,
            offending: Option<TokenView<'_>>,
            line: usize,
            column: usize,
            message: &str,
            error: Option<&AntlrError>,
        ) {
            self.events.lock().expect("events lock").push(Event {
                offending_text: offending.and_then(|token| token.text().map(str::to_owned)),
                line,
                column,
                message: message.to_owned(),
                error: error.cloned(),
            });
        }
    }

    fn parser(
        input: &str,
    ) -> (
        FatalParser<FatalLexer<InputStream>>,
        Arc<Mutex<Vec<Event>>>,
    ) {
        let lexer = FatalLexer::new(InputStream::new(input));
        let mut parser = FatalParser::new(CommonTokenStream::new(lexer));
        // A configured (effectively unbounded) cap selects generated bodies
        // for rules the normal performance routing prefers to interpret.
        parser.set_max_rule_depth(Some(usize::MAX));
        parser.remove_error_listeners();
        let events = Arc::new(Mutex::new(Vec::new()));
        parser.add_error_listener(RecordingListener {
            events: Arc::clone(&events),
        });
        (parser, events)
    }

    #[test]
    fn fatal_public_entry_reports_the_returned_error() {
        let (mut parser, events) = parser("ad");

        let error = parser
            .fatal()
            .expect_err("invalid first token should remain fatal");

        assert_eq!(parser.number_of_syntax_errors(), 1);
        let events = events.lock().expect("events lock");
        assert_eq!(events.len(), 1, "fatal error must be reported exactly once");
        let event = &events[0];
        assert_eq!(event.offending_text.as_deref(), Some("d"));
        let AntlrError::ParserError {
            line,
            column,
            message,
            ..
        } = &error
        else {
            panic!("expected a positioned parser error, got {error:?}");
        };
        assert_eq!((event.line, event.column), (*line, *column));
        assert_eq!(&event.message, message);
        assert_eq!(event.error.as_ref(), Some(&error));
    }

    #[test]
    fn recovered_nested_error_is_not_reported_twice() {
        let (mut parser, events) = parser("ad");

        parser
            .start()
            .expect("the parent should recover the nested child error");

        assert_eq!(parser.number_of_syntax_errors(), 1);
        let events = events.lock().expect("events lock");
        assert_eq!(events.len(), 1, "recovery must report the error exactly once");
        assert_eq!(events[0].offending_text.as_deref(), Some("d"));
    }

    #[test]
    fn fatal_entry_preserves_prior_recovery_diagnostics() {
        let (mut parser, events) = parser("adad");

        let error = parser
            .mixed()
            .expect_err("the entry should fail after the child recovery");

        let events = events.lock().expect("events lock");
        let snapshot = EntrySnapshot {
            returned_error: &error,
            syntax_errors: parser.number_of_syntax_errors(),
            events: &events,
        };
        std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/fatal-entry-events.txt"),
            format!("{snapshot:#?}\n"),
        )
        .expect("fatal entry snapshot should be writable");
    }

    #[test]
    fn semantic_override_does_not_leak_prior_recovery_diagnostics() {
        let (mut parser, events) = parser("adad");

        let error = parser
            .semantic_mixed()
            .expect_err("the semantic override should win over the fatal parser error");
        assert!(
            matches!(&error, AntlrError::Unsupported(_)),
            "expected the configured fail-loud semantic error, got {error:?}"
        );

        let reported_before_reuse = events.lock().expect("events lock").len();
        parser
            .clean()
            .expect("the clean entry should succeed on the rewound input");
        let events = events.lock().expect("events lock");
        assert_eq!(
            events.len(),
            reported_before_reuse,
            "the clean entry must not emit diagnostics retained by the failed entry"
        );

        let snapshot = EntrySnapshot {
            returned_error: &error,
            syntax_errors: parser.number_of_syntax_errors(),
            events: &events,
        };
        std::fs::write(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/semantic-override-events.txt"
            ),
            format!("{snapshot:#?}\n"),
        )
        .expect("semantic override snapshot should be writable");
    }
}
"####;

    assert_generated_project(
        temp.path(),
        &["fatal_lexer.rs", "fatal_parser.rs"],
        test_source,
    );
    let fatal_entry =
        fs::read_to_string(temp.path().join("compile-generated/fatal-entry-events.txt"))
            .expect("fatal entry snapshot should be emitted");
    insta::assert_snapshot!(
        "fatal_entry_preserves_prior_recovery_diagnostics",
        fatal_entry
    );
    let semantic_override = fs::read_to_string(
        temp.path()
            .join("compile-generated/semantic-override-events.txt"),
    )
    .expect("semantic override snapshot should be emitted");
    insta::assert_snapshot!(
        "semantic_override_does_not_leak_prior_recovery_diagnostics",
        semantic_override
    );
}

/// Editors on Windows commonly save `.g4` sources with a UTF-8 byte order mark
/// and CRLF line endings. Both must generate exactly like the plain spelling.
#[test]
fn byte_order_mark_and_crlf_grammars_generate_like_plain_sources() {
    let temp = temporary_directory("bom-crlf");
    let plain = "lexer grammar Letters;\nA: 'a';\nWS: [ \\t\\r\\n]+ -> skip;\n";
    let crlf = "lexer grammar Letters;\r\nA: 'a';\r\nWS: [ \\t\\r\\n]+ -> skip;\r\n";
    let cases = [
        ("plain", plain.to_owned()),
        ("bom", format!("\u{feff}{plain}")),
        ("crlf", crlf.to_owned()),
        ("bom-crlf", format!("\u{feff}{crlf}")),
    ];

    let mut generated = Vec::new();
    for (name, text) in cases {
        let case = temp.path().join(name);
        let grammar = case.join("Letters.g4");
        let out = case.join("generated");
        fs::create_dir_all(&case).expect("case directory should be writable");
        fs::write(&grammar, &text).expect("grammar should be writable");

        let output = run_antlr4_rust_gen(&[
            grammar.as_os_str(),
            OsStr::new("--out-dir"),
            out.as_os_str(),
        ]);
        assert!(
            output.status.success(),
            "{name}: stdout: {}\nstderr: {}",
            utf8(&output.stdout),
            utf8(&output.stderr)
        );
        generated.push((
            name,
            fs::read_to_string(out.join("letters.rs")).expect("lexer should be emitted"),
        ));
    }

    let (_, expected) = &generated[0];
    for (name, actual) in &generated[1..] {
        assert_eq!(
            actual, expected,
            "{name} output differs from the plain source"
        );
    }
    assert!(
        !expected.contains('\r'),
        "generated code should not carry carriage returns"
    );
}

/// A `.tokens` vocabulary is a generated sidecar parsed line by line, so it
/// never reaches the grammar lexer's byte order mark handling. Both a marked
/// and a CRLF sidecar must still supply the recorded token numbers.
#[test]
fn byte_order_mark_and_crlf_token_vocabularies_are_honored() {
    for (name, vocabulary) in [
        ("plain", "ID=1\nNUM=2\n".to_owned()),
        ("bom", "\u{feff}ID=1\nNUM=2\n".to_owned()),
        ("crlf", "ID=1\r\nNUM=2\r\n".to_owned()),
        ("bom-crlf", "\u{feff}ID=1\r\nNUM=2\r\n".to_owned()),
    ] {
        let temp = temporary_directory("vocab-bom");
        let grammar = temp.path().join("P.g4");
        let out = temp.path().join("generated");
        fs::write(temp.path().join("V.tokens"), &vocabulary)
            .expect("vocabulary should be writable");
        fs::write(
            &grammar,
            "parser grammar P;\n\
             options { tokenVocab=V; }\n\
             r: ID NUM;\n",
        )
        .expect("grammar should be writable");

        let output = run_antlr4_rust_gen(&[
            grammar.as_os_str(),
            OsStr::new("--lib"),
            temp.path().as_os_str(),
            OsStr::new("--out-dir"),
            out.as_os_str(),
        ]);
        assert!(
            output.status.success(),
            "{name}: stdout: {}\nstderr: {}",
            utf8(&output.stdout),
            utf8(&output.stderr)
        );
        let parser = fs::read_to_string(out.join("p.rs")).expect("parser should be emitted");
        assert!(
            parser.contains("ID: i32 = 1;") && parser.contains("NUM: i32 = 2;"),
            "{name}: vocabulary numbers were not imported"
        );
    }
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn combined_literal_tokens_are_public_and_lexable() {
    let temp = temporary_directory("combined-literal-tokens");
    let grammar = temp.path().join("T.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar T;\n\
         greeting : 'hello' NAME 'world' ;\n\
         NAME : [a-zA-Z]+ ;\n\
         WS : [ \\t\\r\\n]+ -> skip ;\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let constants = ["t_lexer.rs", "t_parser.rs"].map(|module| {
        let generated =
            fs::read_to_string(out.join(module)).expect("generated module should be readable");
        let (_, after_eof) = generated
            .split_once("pub const EOF: i32 = antlr4_runtime::TOKEN_EOF;")
            .expect("generated token constants should start with EOF");
        let (after_eof, _) = after_eof
            .split_once("\n\n")
            .expect("generated token constants should form their own block");
        (
            module,
            format!("pub const EOF: i32 = antlr4_runtime::TOKEN_EOF;{after_eof}"),
        )
    });
    insta::assert_debug_snapshot!("combined_literal_token_constants", constants);

    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod combined_literal_tests {
    use super::t_lexer::TLexer;
    use super::t_parser::TParser;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    #[test]
    fn recognizes_implicit_literal_rules() {
        let lexer = TLexer::new(InputStream::new("hello Alice world"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = TParser::new(tokens);
        parser.greeting().expect("literal input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }
}
"#,
    );
}

#[test]
fn combined_root_suffixes_alternative_contexts_and_listener_methods() {
    let temp = temporary_directory("combined-contexts");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/combined-contexts/Shapes.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert!(out.join("shapes_lexer.rs").is_file());
    let parser =
        fs::read_to_string(out.join("shapes_parser.rs")).expect("parser should be emitted");
    for expected in [
        "pub struct StartContext<'a, State = StoredTreeContext>",
        "pub struct SingleLabelContext<'a, State = StoredTreeContext>",
        "pub struct ManyLabelContext<'a, State = StoredTreeContext>",
        "pub trait ShapesListener<E = std::convert::Infallible>",
        "pub struct ShapesTreeWalker",
        "pub type ParseTreeWalker = ShapesTreeWalker",
        "fn enter_every_rule(&mut self",
        "fn enter_single_label(&mut self",
        "fn enter_many_label(&mut self",
        "pub fn atom_children(&self) -> impl Iterator<Item = AtomContext<'a>>",
        "pub fn first(&self) -> Result<AtomContext<'a>, MissingChildError>",
        "pub fn rest(&self) -> impl Iterator<Item = AtomContext<'a>>",
        "pub fn value(&self) -> Result<AtomContext<'a>, MissingChildError>",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }
    assert!(
        !parser.contains("_all(&self)"),
        "generated contexts must not expose allocating Java-style list accessors\n{parser}"
    );
    assert!(
        !parser.contains("antlr4_runtime::{{"),
        "generated imports must not contain redundant nested braces\n{parser}"
    );
    assert!(
        !parser.contains("pub trait ShapesVisitor"),
        "visitor generation must remain opt-in\n{parser}"
    );
    assert_generated_project(
        temp.path(),
        &["shapes_lexer.rs", "shapes_parser.rs"],
        r#"
#[cfg(test)]
mod typed_label_tests {
    use super::shapes_lexer::ShapesLexer;
    use super::shapes_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    #[test]
    fn list_and_repeated_single_labels_keep_antlr_semantics() {
        let lexer = ShapesLexer::new(InputStream::new("a,b,c"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = ShapesParser::new(tokens);
        let root = parser.start().expect("list input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        let many = parsed
            .tree()
            .as_rule()
            .expect("start rule")
            .downcast_ref::<ManyLabelContext>()
            .expect("comma-separated input uses the many alternative");
        assert_eq!(
            many
                .rest()
                .map(|atom| atom.rule_node().node().text())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );

        let lexer = ShapesLexer::new(InputStream::new("a b c"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = ShapesParser::new(tokens);
        let root = parser.latest().expect("repeated input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        let latest = parsed
            .tree()
            .as_rule()
            .expect("latest rule")
            .downcast_ref::<LatestContext>()
            .expect("latest context");
        assert_eq!(latest.atom_children().count(), 3);
        assert_eq!(
            latest
                .value()
                .expect("one or more atoms guarantees a value")
                .rule_node()
                .node()
                .text(),
            "c"
        );
    }
}
"#,
    );
}

#[test]
fn grouped_literal_tokens_are_exposed_on_typed_contexts() {
    let temp = temporary_directory("grouped-token-accessors");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/grouped-token-accessors/T.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod grouped_token_tests {
    use super::t_lexer::TLexer;
    use super::t_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    #[test]
    fn reads_grouped_operators_and_context_text() {
        let lexer = TLexer::new(InputStream::new("left<=right=+="));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = TParser::new(tokens);
        let root = parser.root().expect("operator input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        let root = parsed
            .tree()
            .as_rule()
            .expect("root rule")
            .downcast_ref::<RootContext>()
            .expect("typed root context");
        let expression = root.expression().expect("root expression");
        assert_eq!(expression.text(), "left<=right");
        assert_eq!(expression.bop().expect("operator label").to_string(), "<=");
        assert!(expression.le_token().is_some());
        assert!(expression.ge_token().is_none());
        assert!(expression.equal_token().is_none());
        assert!(expression.notequal_token().is_none());
        assert!(expression.assign_token().is_none());
        assert!(expression.add_assign_token().is_none());

        let identifier = expression
            .expression_children()
            .next()
            .expect("left expression")
            .identifier()
            .expect("left identifier");
        assert_eq!(identifier.text(), "left");

        let operators = root
            .operator_sequence()
            .expect("trailing operator sequence");
        assert_eq!(operators.assign_tokens().count(), 1);
        assert_eq!(operators.add_assign_tokens().count(), 1);
        assert_eq!(operators.le_tokens().count(), 0);

        let eof_choice = root.eof_choice().expect("EOF choice");
        assert!(eof_choice.eof_token().is_some());
        assert!(eof_choice.le_token().is_none());
    }
}
"#,
    );
}

#[test]
fn token_group_label_shared_across_alternatives_unions_the_sets() {
    let temp = temporary_directory("multi-alternative-label");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/multi-alternative-label/T.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod multi_alternative_label_tests {
    use super::t_lexer::TLexer;
    use super::t_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    fn parse(input: &str) -> antlr4_runtime::ParsedFile {
        let lexer = TLexer::new(InputStream::new(input));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = TParser::new(tokens);
        let root = parser.expr().expect("input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        parser.into_parsed_file(root)
    }

    #[test]
    fn op_label_is_available_on_every_alternative_group() {
        let parsed = parse("a * b + c < d");
        let expr = parsed
            .tree()
            .as_rule()
            .expect("expr rule")
            .downcast_ref::<ExprContext>()
            .expect("typed expr context");
        let relation = expr.relation().expect("relation child");
        assert_eq!(relation.op().expect("relation operator").to_string(), "<");

        let calc = relation
            .relation_children()
            .next()
            .expect("left relation")
            .calc()
            .expect("calc child");
        assert_eq!(calc.op().expect("additive operator").to_string(), "+");

        let product = calc.calc_children().next().expect("left calc");
        assert_eq!(
            product.op().expect("multiplicative operator").to_string(),
            "*"
        );

        let leaf = product.calc_children().next().expect("leaf calc");
        assert!(leaf.op().is_none(), "primary alternative carries no operator");
    }
}
"#,
    );
}

#[test]
fn token_label_accessors_distinguish_deleted_and_inserted_recovery_tokens() {
    let temp = temporary_directory("token-label-recovery");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/token-label-recovery/T.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod token_label_recovery_tests {
    use super::t_lexer::TLexer;
    use super::t_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _, Token as _};

    #[test]
    fn union_label_skips_a_deleted_token_from_another_alternative() {
        let lexer = TLexer::new(InputStream::new("x b a"));
        let mut parser = TParser::new(CommonTokenStream::new(lexer));
        let root = parser
            .different_types()
            .expect("single-token deletion should recover");
        assert_eq!(parser.number_of_syntax_errors(), 1);
        let parsed = parser.into_parsed_file(root);
        let context = parsed
            .tree()
            .as_rule()
            .expect("differentTypes rule")
            .downcast_ref::<DifferentTypesContext>()
            .expect("typed differentTypes context");

        assert_eq!(context.op().expect("operator label").to_string(), "a");
        assert_eq!(
            context
                .b_token()
                .expect("plain token accessors retain deleted tokens")
                .to_string(),
            "b"
        );
    }

    #[test]
    fn single_type_label_skips_a_deleted_token_of_the_same_type() {
        let lexer = TLexer::new(InputStream::new("x a y a"));
        let mut parser = TParser::new(CommonTokenStream::new(lexer));
        let root = parser
            .same_type()
            .expect("single-token deletion should recover");
        assert_eq!(parser.number_of_syntax_errors(), 1);
        let parsed = parser.into_parsed_file(root);
        let context = parsed
            .tree()
            .as_rule()
            .expect("sameType rule")
            .downcast_ref::<SameTypeContext>()
            .expect("typed sameType context");

        assert_eq!(context.op().expect("operator label").to_string(), "a");
        assert_eq!(
            context
                .a_token()
                .expect("plain token accessor retains the deleted token")
                .symbol()
                .column(),
            2
        );
        assert_eq!(context.op().expect("operator label").symbol().column(), 6);
    }

    #[test]
    fn label_keeps_a_synthesized_missing_token() {
        let lexer = TLexer::new(InputStream::new("x b"));
        let mut parser = TParser::new(CommonTokenStream::new(lexer));
        let root = parser
            .missing_token()
            .expect("single-token insertion should recover");
        assert_eq!(parser.number_of_syntax_errors(), 1);
        let parsed = parser.into_parsed_file(root);
        let context = parsed
            .tree()
            .as_rule()
            .expect("missingToken rule")
            .downcast_ref::<MissingTokenContext>()
            .expect("typed missingToken context");
        let label = context.op().expect("missing token remains assigned to label");

        assert_eq!(label.to_string(), "<missing 'a'>");
        assert_eq!(label.symbol().start(), usize::MAX);
        assert_eq!(
            label.symbol(),
            context.a_token().expect("plain token accessor").symbol()
        );
    }
}
"#,
    );
}

#[test]
fn compile_parse_tree_pattern_matches_and_binds_against_generated_parser() {
    let temp = temporary_directory("tree-pattern-compile");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/typed-tree-walkers/Calculator.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let parser =
        fs::read_to_string(out.join("calculator_parser.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("pub fn compile_parse_tree_pattern<PL>("),
        "generated parser must expose compile_parse_tree_pattern\n{parser}"
    );

    // End-to-end: compile a pattern rooted at the (left-recursive) `expression`
    // rule and match it against a real parse. Exercises the rule-bypass ATN's
    // left-recursive path through a genuinely generated grammar.
    assert_generated_project(
        temp.path(),
        &["calculator_lexer.rs", "calculator_parser.rs"],
        r#"
#[cfg(test)]
mod tree_pattern_tests {
    use super::calculator_lexer::CalculatorLexer;
    use super::calculator_parser::*;
    use antlr4_runtime::{CommonTokenStream, InputStream, Node, Parser as _};

    /// Parses `input` and returns the owned tree plus the top-level expression
    /// node id, for matching against a pattern.
    fn parse_top_expression(input: &'static str) -> antlr4_runtime::ParsedFile {
        let lexer = CalculatorLexer::new(InputStream::new(input));
        let mut parser = CalculatorParser::new(CommonTokenStream::new(lexer));
        let root = parser.start().expect("input parses");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        parser.into_parsed_file(root)
    }

    fn top_expression(parsed: &antlr4_runtime::ParsedFile) -> Node<'_> {
        parsed
            .tree()
            .as_rule()
            .expect("start rule")
            .child_rule(RULE_EXPRESSION)
            .expect("top-level expression")
            .node()
    }

    #[test]
    fn compiles_and_matches_expression_pattern() {
        let lexer = CalculatorLexer::new(InputStream::new(""));
        let parser = CalculatorParser::new(CommonTokenStream::new(lexer));
        // `<expr> + <expr>` rooted at the expression rule.
        let pattern = parser
            .compile_parse_tree_pattern(
                "<expression> + <expression>",
                RULE_EXPRESSION,
                CalculatorLexer::new,
            )
            .expect("pattern compiles");

        let parsed = parse_top_expression("2 + 8");
        let result = pattern.match_tree(top_expression(&parsed));
        assert!(result.succeeded(), "2 + 8 should match `<expr> + <expr>`");
        // Both operands bind under the rule name `expression`.
        let operands: Vec<_> = result
            .get_all("expression")
            .iter()
            .map(|node| node.text())
            .collect();
        assert_eq!(operands, vec!["2".to_owned(), "8".to_owned()]);
    }

    #[test]
    fn rejects_non_matching_structure() {
        let lexer = CalculatorLexer::new(InputStream::new(""));
        let parser = CalculatorParser::new(CommonTokenStream::new(lexer));
        let pattern = parser
            .compile_parse_tree_pattern(
                "<expression> * <expression>",
                RULE_EXPRESSION,
                CalculatorLexer::new,
            )
            .expect("pattern compiles");

        let parsed = parse_top_expression("2 + 8");
        // Addition must not match a multiplication pattern.
        assert!(!pattern.match_tree(top_expression(&parsed)).succeeded());
    }

    #[test]
    fn trailing_eof_tag_requires_a_rule_that_consumes_it() {
        let lexer = CalculatorLexer::new(InputStream::new(""));
        let parser = CalculatorParser::new(CommonTokenStream::new(lexer));
        // `start : expression EOF ;` consumes the tag: the pattern matches a
        // whole parse.
        let pattern = parser
            .compile_parse_tree_pattern(
                "<expression> <EOF>",
                RULE_START,
                CalculatorLexer::new,
            )
            .expect("EOF-consuming rule accepts a trailing <EOF> tag");
        let parsed = parse_top_expression("2 + 8");
        assert!(pattern.match_tree(parsed.tree()).succeeded());

        // `expression` never consumes EOF, so the tag would be silently
        // dropped from the pattern tree; that must be rejected.
        assert!(
            parser
                .compile_parse_tree_pattern(
                    "<expression> + <expression> <EOF>",
                    RULE_EXPRESSION,
                    CalculatorLexer::new,
                )
                .is_err(),
            "unconsumed trailing <EOF> tag must not compile"
        );
    }
}
"#,
    );
}

#[test]
fn visitor_and_typed_walk_dispatch_labeled_left_recursion() {
    let temp = temporary_directory("typed-tree-walkers");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/typed-tree-walkers/Calculator.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--visitor"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser =
        fs::read_to_string(out.join("calculator_parser.rs")).expect("parser should be emitted");
    for expected in [
        "pub trait CalculatorVisitor",
        "pub trait CalculatorVisitable",
        "pub trait CalculatorListener",
        "pub struct CalculatorTreeWalker",
        "fn visit_multiply_label(&mut self",
        "fn visit_add_label(&mut self",
        "fn visit_number_label(&mut self",
        "fn default_result(&mut self) -> Self::Result;",
        "pub trait CalculatorListener<E = std::convert::Infallible>",
        "pub fn expression_children(&self) -> impl Iterator<Item = ExpressionContext<'a>>",
        "pub fn left(&self) -> Result<ExpressionContext<'a>, MissingChildError>",
        "pub fn right(&self) -> Result<ExpressionContext<'a>, MissingChildError>",
        "pub fn star_token(&self) -> Option<TerminalNode<'a>>",
        "pub fn int_token(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn eof_token(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn literal(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn choice(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn other(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn wildcard(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn plus_token(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "pub fn star_token(&self) -> Result<TerminalNode<'a>, MissingChildError>",
        "__labeled_token_children_matching(self.__node",
        "track_context_alt_numbers: true",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }
    assert!(
        !parser.contains("pub fn INT(") && !parser.contains("_all(&self)"),
        "generated contexts must expose Rust-shaped token and collection accessors\n{parser}"
    );

    assert_generated_project(
        temp.path(),
        &["calculator_lexer.rs", "calculator_parser.rs"],
        r#"
#[cfg(test)]
mod allocation_tracking {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    std::thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    pub struct TrackingAllocator;

    unsafe impl GlobalAlloc for TrackingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let pointer = unsafe { System.alloc(layout) };
            record_allocation();
            pointer
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let pointer = unsafe { System.alloc_zeroed(layout) };
            record_allocation();
            pointer
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) };
        }

        unsafe fn realloc(
            &self,
            pointer: *mut u8,
            layout: Layout,
            new_size: usize,
        ) -> *mut u8 {
            let pointer = unsafe { System.realloc(pointer, layout, new_size) };
            record_allocation();
            pointer
        }
    }

    fn record_allocation() {
        ENABLED.with(|enabled| {
            if enabled.get() {
                ALLOCATIONS.with(|allocations| allocations.set(allocations.get() + 1));
            }
        });
    }

    pub fn count_allocations<T>(operation: impl FnOnce() -> T) -> (T, usize) {
        ALLOCATIONS.with(|allocations| allocations.set(0));
        ENABLED.with(|enabled| enabled.set(true));
        let value = operation();
        ENABLED.with(|enabled| enabled.set(false));
        let allocations = ALLOCATIONS.with(Cell::get);
        (value, allocations)
    }
}

#[cfg(test)]
#[global_allocator]
static ALLOCATOR: allocation_tracking::TrackingAllocator =
    allocation_tracking::TrackingAllocator;

#[cfg(test)]
mod typed_tree_tests {
    use super::calculator_lexer::CalculatorLexer;
    use super::calculator_parser::*;
    use super::allocation_tracking::count_allocations;
    use antlr4_runtime::{
        CommonTokenStream, InputStream, MissingChildError, Parser as _, RuleNodeView,
    };

    struct Eval;

    impl CalculatorVisitor for Eval {
        type Result = Result<i64, MissingChildError>;

        fn default_result(&mut self) -> Self::Result {
            Ok(0)
        }

        fn visit_start(&mut self, ctx: &StartContext) -> Self::Result {
            self.visit(ctx.expression()?)
        }

        fn visit_number_label(&mut self, ctx: &NumberLabelContext) -> Self::Result {
            Ok(ctx
                .int_token()?
                .to_string()
                .parse()
                .expect("integer token"))
        }

        fn visit_multiply_label(&mut self, ctx: &MultiplyLabelContext) -> Self::Result {
            let left = self.visit(ctx.left()?)?;
            let right = self.visit(ctx.right()?)?;
            if ctx.star_token().is_some() {
                Ok(left * right)
            } else {
                Ok(left / right)
            }
        }

        fn visit_add_label(&mut self, ctx: &AddLabelContext) -> Self::Result {
            let left = self.visit(ctx.left()?)?;
            let right = self.visit(ctx.right()?)?;
            if ctx.plus_token().is_some() {
                Ok(left + right)
            } else {
                Ok(left - right)
            }
        }
    }

    #[derive(Default)]
    struct Trace {
        events: Vec<&'static str>,
        entered_rules: usize,
        exited_rules: usize,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct TraceError;

    impl CalculatorListener<TraceError> for Trace {
        fn enter_every_rule(&mut self, _ctx: RuleNodeView<'_>) -> Result<(), TraceError> {
            self.entered_rules += 1;
            Ok(())
        }

        fn exit_every_rule(&mut self, _ctx: RuleNodeView<'_>) -> Result<(), TraceError> {
            self.exited_rules += 1;
            Ok(())
        }

        fn enter_multiply_label(
            &mut self,
            _ctx: &MultiplyLabelContext,
        ) -> Result<(), TraceError> {
            self.events.push("enter:multiply");
            Ok(())
        }

        fn exit_multiply_label(
            &mut self,
            _ctx: &MultiplyLabelContext,
        ) -> Result<(), TraceError> {
            self.events.push("exit:multiply");
            Ok(())
        }

        fn enter_add_label(&mut self, _ctx: &AddLabelContext) -> Result<(), TraceError> {
            self.events.push("enter:add");
            Ok(())
        }

        fn exit_add_label(&mut self, _ctx: &AddLabelContext) -> Result<(), TraceError> {
            self.events.push("exit:add");
            Ok(())
        }

        fn enter_number_label(
            &mut self,
            _ctx: &NumberLabelContext,
        ) -> Result<(), TraceError> {
            self.events.push("enter:number");
            Ok(())
        }

        fn exit_number_label(
            &mut self,
            _ctx: &NumberLabelContext,
        ) -> Result<(), TraceError> {
            self.events.push("exit:number");
            Ok(())
        }
    }

    struct FailingTrace;

    impl CalculatorListener<&'static str> for FailingTrace {
        fn enter_multiply_label(
            &mut self,
            _ctx: &MultiplyLabelContext,
        ) -> Result<(), &'static str> {
            Err("stop at multiply")
        }
    }

    #[test]
    fn evaluates_and_walks_exact_typed_alternatives() {
        let lexer = CalculatorLexer::new(InputStream::new("2 + 8 / 2"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = CalculatorParser::new(tokens);
        let root = parser.start().expect("calculator input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        assert!(
            parsed
                .tree()
                .descendants()
                .filter_map(antlr4_runtime::Node::as_rule)
                .all(|rule| rule.alt_number() == 0),
            "typed dispatch metadata must not become display-visible alt numbers"
        );
        let start = parsed
            .tree()
            .as_rule()
            .expect("start rule")
            .downcast_ref::<StartContext>()
            .expect("typed start context");
        assert_eq!(start.eof_token().expect("required EOF").to_string(), "<EOF>");

        assert_eq!(Eval.visit(parsed.tree()).expect("evaluation succeeds"), 6);

        let mut trace = Trace::default();
        trace.walk(parsed.tree()).expect("typed listener walk");
        assert_eq!(
            trace.events,
            [
                "enter:add",
                "enter:number",
                "exit:number",
                "enter:multiply",
                "enter:number",
                "exit:number",
                "enter:number",
                "exit:number",
                "exit:multiply",
                "exit:add",
            ]
        );
        assert_eq!(trace.entered_rules, 6);
        assert_eq!(trace.exited_rules, 6);

        assert_eq!(
            FailingTrace.walk(parsed.tree()),
            Err("stop at multiply"),
            "listener domain errors must stop and escape the generated walker"
        );

        let start = parsed.tree().as_rule().expect("start rule");
        let expression = start
            .child_rule(RULE_EXPRESSION)
            .expect("top-level expression");
        let add = expression
            .downcast_ref::<AddLabelContext>()
            .expect("top-level expression is addition");
        assert_eq!(add.rule_node().node().id(), expression.node().id());
        let expected_display = format!(
            "[{}]",
            expression
                .invocation_states()
                .map(|state| state.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(add.to_string(), expected_display);
        assert_eq!(add.expression_children().count(), 2);
        assert!(add.plus_token().is_some());
        assert!(add.minus_token().is_none());
        assert_eq!(
            add.left().expect("left expression").rule_node().node().id(),
            expression
                .child_rules(RULE_EXPRESSION)
                .next()
                .expect("left expression")
                .node()
                .id()
        );
        assert!(expression.downcast_ref::<MultiplyLabelContext>().is_none());

        let (child_ids, allocations) = count_allocations(|| {
            let add = expression
                .downcast_ref::<AddLabelContext>()
                .expect("top-level expression is addition");
            let left = add.left().expect("left expression");
            let right = add.right().expect("right expression");
            (
                add.rule_node().node().id(),
                left.rule_node().node().id(),
                right.rule_node().node().id(),
            )
        });
        assert_eq!(child_ids.0, expression.node().id());
        assert_eq!(
            allocations, 0,
            "stored typed contexts and child accessors must not allocate"
        );

        let right = expression
            .child_rules(RULE_EXPRESSION)
            .nth(1)
            .expect("right expression");
        assert!(right.downcast_ref::<MultiplyLabelContext>().is_some());
        assert!(right.downcast_ref::<AddLabelContext>().is_none());

        let lexer = CalculatorLexer::new(InputStream::new("+*1-"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = CalculatorParser::new(tokens);
        let root = parser
            .labeled_tokens()
            .expect("labeled token input should parse");
        let parsed = parser.into_parsed_file(root);
        let labeled = parsed
            .tree()
            .as_rule()
            .expect("labeledTokens rule")
            .downcast_ref::<LabeledTokensContext>()
            .expect("typed labeledTokens context");
        assert_eq!(labeled.literal().expect("literal label").to_string(), "+");
        assert_eq!(labeled.choice().expect("set label").to_string(), "*");
        assert_eq!(labeled.other().expect("not-set label").to_string(), "1");
        assert_eq!(labeled.wildcard().expect("wildcard label").to_string(), "-");

        let lexer = CalculatorLexer::new(InputStream::new("+*"));
        let tokens = CommonTokenStream::new(lexer);
        let mut parser = CalculatorParser::new(tokens);
        let root = parser
            .literal_tokens()
            .expect("literal token input should parse");
        let parsed = parser.into_parsed_file(root);
        let literal_tokens = parsed
            .tree()
            .as_rule()
            .expect("literalTokens rule")
            .downcast_ref::<LiteralTokensContext>()
            .expect("typed literalTokens context");
        assert_eq!(
            literal_tokens
                .plus_token()
                .expect("required literal PLUS")
                .to_string(),
            "+"
        );
        assert_eq!(
            literal_tokens
                .star_token()
                .expect("required literal STAR")
                .to_string(),
            "*"
        );
        assert_eq!(
            literal_tokens
                .eof_token()
                .expect("required literal EOF")
                .to_string(),
            "<EOF>"
        );
    }
}
"#,
    );
}

#[test]
fn listener_and_visitor_generation_can_be_disabled_independently() {
    let temp = temporary_directory("tree-walker-flags");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/combined-contexts/Shapes.g4");
    let visitor_only = temp.path().join("visitor-only");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("-no-listener"),
        OsStr::new("-visitor"),
        OsStr::new("--out-dir"),
        visitor_only.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(visitor_only.join("shapes_parser.rs"))
        .expect("parser should be emitted");
    assert!(parser.contains("pub trait ShapesVisitor"), "{parser}");
    assert!(!parser.contains("pub trait ShapesListener"), "{parser}");
    assert!(!parser.contains("pub struct ShapesTreeWalker"), "{parser}");
    assert!(!parser.contains("pub type ParseTreeWalker"), "{parser}");

    let neither = temp.path().join("neither");
    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--no-listener"),
        OsStr::new("--visitor"),
        OsStr::new("--no-visitor"),
        OsStr::new("--out-dir"),
        neither.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser =
        fs::read_to_string(neither.join("shapes_parser.rs")).expect("parser should be emitted");
    assert!(!parser.contains("pub trait ShapesVisitor"), "{parser}");
    assert!(!parser.contains("pub trait ShapesListener"), "{parser}");
}

#[test]
fn colliding_rule_and_alternative_label_context_names_compile() {
    let temp = temporary_directory("context-name-collision");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/context-name-collision/T.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("t.rs")).expect("parser should be emitted");
    for expected in [
        "pub struct ObjectCreationExpressionContext<'a, State = StoredTreeContext>",
        "pub struct ObjectCreationExpressionLabelContext<'a, State = StoredTreeContext>",
        "pub struct ParenthesizedLabelContext<'a, State = StoredTreeContext>",
        "fn enter_object_creation_expression(&mut self",
        "fn enter_object_creation_expression_label(&mut self",
        "fn enter_parenthesized_label(&mut self",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }
    assert_generated_modules_compile(temp.path(), &["t.rs"]);
}

#[test]
fn embedded_parser_semantics_satisfy_strict_manifest_checks() {
    let temp = temporary_directory("embedded-parser-semantics");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/embedded-parser-semantics/T.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    assert_eq!(
        manifest.matches("\"disposition\": \"translated\"").count(),
        2
    );
    assert_eq!(manifest.matches("\"template\": \"Embedded\"").count(), 2);
    assert_generated_modules_compile(temp.path(), &["t_lexer.rs", "t_parser.rs"]);
}

#[test]
fn imported_predicate_manifest_uses_its_structural_source_owner() {
    let temp = temporary_directory("imported-predicate");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\n\
         delegated: {featureEnabled()}? ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(
        &tokens,
        "lexer grammar Tokens;\n\
         ID: [a-z]+;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    assert!(manifest.contains("\"name\": \"Root\""), "{manifest}");
    assert!(
        manifest.contains("\"body\": \"featureEnabled()\""),
        "{manifest}"
    );
    assert!(manifest.contains("\"line\": 2"), "{manifest}");
}

#[test]
fn imported_parser_predicate_generates_typed_hook_from_structural_body() {
    let temp = temporary_directory("imported-parser-hook");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\ndelegated: {isTypeName()}? ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(&tokens, "lexer grammar Tokens;\nID: [a-z]+;\n")
        .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root.rs")).expect("parser should be emitted");
    assert!(parser.contains("pub trait RootHooks"), "{parser}");
    assert!(parser.contains("fn is_type_name"), "{parser}");
    assert!(
        parser.contains("(1, 0) => Some(self.0.is_type_name(ctx))"),
        "{parser}"
    );
}

#[test]
fn imported_lexer_action_generates_typed_hook_from_structural_body() {
    let temp = temporary_directory("imported-lexer-hook");
    let root = temp.path().join("RootLexer.g4");
    let delegate = temp.path().join("DelegateLexer.g4");
    let patterns = temp.path().join("patterns.toml");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "lexer grammar RootLexer;\nimport DelegateLexer;\nB: 'b';\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "lexer grammar DelegateLexer;\nA: 'a' {this.handle(\"a\");};\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(
        &patterns,
        "version = 1\n\
         [[helper]]\n\
         kind = \"lexer-action\"\n\
         name = \"handle\"\n\
         arguments = \"string\"\n\
         returns = \"unit\"\n\
         lower = \"hook\"\n",
    )
    .expect("semantic patterns should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--sem-patterns"),
        patterns.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let lexer = fs::read_to_string(out.join("root_lexer.rs")).expect("lexer should be emitted");
    assert!(lexer.contains("pub trait RootLexerHooks"), "{lexer}");
    assert!(lexer.contains("fn handle"), "{lexer}");
    assert!(lexer.contains("self.0.handle(ctx, \"a\")"), "{lexer}");
}

#[test]
fn imported_rule_arguments_and_locals_use_structural_call_owners() {
    let temp = temporary_directory("imported-rule-arguments");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: outer EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\n\
         outer locals [boolean seen=false]\n\
             : {$seen=true;} {$seen}? target[true]\n\
             ;\n\
         target[boolean enabled]: ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(&tokens, "lexer grammar Tokens;\nID: [a-z]+;\n")
        .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("let mut __antlr_local_seen = false;"),
        "{parser}"
    );
    assert!(
        parser.contains("parse_generated_rule_2_dispatch(1, false)"),
        "{parser}"
    );
}

#[test]
fn imported_embedded_action_uses_structural_rule_and_transition_owner() {
    let temp = temporary_directory("imported-embedded-action");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\n\
         delegated: {writeln!(self.output(), \"delegated\").unwrap();} ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(&tokens, "lexer grammar Tokens;\nID: [a-z]+;\n")
        .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("writeln!(self.output(), \"delegated\").unwrap();"),
        "{parser}"
    );
}

#[test]
fn multiple_roots_and_repeatable_library_paths_are_resolved() {
    let temp = temporary_directory("roots");
    let first_lib = temp.path().join("first-lib");
    let second_lib = temp.path().join("second-lib");
    let out = temp.path().join("generated");
    fs::create_dir_all(&first_lib).expect("first library directory should be writable");
    fs::create_dir_all(&second_lib).expect("second library directory should be writable");
    fs::write(
        first_lib.join("Shared.g4"),
        "lexer grammar Shared;\nA: 'a';\n",
    )
    .expect("import should be writable");
    let root = temp.path().join("Root.g4");
    let other = temp.path().join("Other.g4");
    fs::write(&root, "lexer grammar Root;\nimport Shared;\nB: 'b';\n")
        .expect("root should be writable");
    fs::write(&other, "lexer grammar Other;\nC: 'c';\n").expect("second root should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        other.as_os_str(),
        OsStr::new("-I"),
        first_lib.as_os_str(),
        OsStr::new("--lib"),
        second_lib.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert!(out.join("root.rs").is_file());
    assert!(out.join("other.rs").is_file());
    assert!(!out.join("shared.rs").exists());
}

#[test]
fn invalid_source_emits_diagnostics_without_partial_outputs() {
    let temp = temporary_directory("invalid");
    let grammar = temp.path().join("Broken.g4");
    let out = temp.path().join("generated");
    fs::write(&grammar, "lexer grammar Broken;\nA: 'unterminated;\n")
        .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");
    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("Broken.g4"), "{stderr}");
    assert!(stderr.contains("G4F002"), "{stderr}");
    assert!(!stderr.contains("unknown argument"), "{stderr}");
    assert!(
        !out.exists()
            || fs::read_dir(&out)
                .expect("output should be readable")
                .next()
                .is_none(),
        "failed compilation emitted partial output"
    );
}

#[test]
fn imported_source_diagnostics_report_the_import_path() {
    let temp = temporary_directory("imported-diagnostic");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/imported-diagnostic");
    let root = fixture.join("Root.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        OsStr::new("--lib"),
        fixture.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    let stderr = utf8(&output.stderr);
    let delegate_diagnostic = format!("error[G4F003]: {}", fixture.join("Delegate.g4").display());
    let wrong_root_diagnostic = format!("error[G4F003]: {}", root.display());
    assert!(stderr.contains(&delegate_diagnostic), "{stderr}");
    assert!(!stderr.contains(&wrong_root_diagnostic), "{stderr}");
    assert!(!out.exists(), "failed compilation emitted output");
}

#[test]
fn unsupported_grammar_options_warn_and_exact_hooks_acknowledge_them() {
    let temp = temporary_directory("options");
    let grammar = temp.path().join("OptionsLexer.g4");
    fs::write(
        &grammar,
        "lexer grammar OptionsLexer;\noptions { superClass = MyLexerBase; }\nA: 'a';\n",
    )
    .expect("grammar should be writable");

    let unsupported_out = temp.path().join("unsupported");
    let unsupported = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        unsupported_out.as_os_str(),
        OsStr::new("--require-full-semantics"),
    ]);
    assert!(!unsupported.status.success());
    let stderr = utf8(&unsupported.stderr);
    assert!(
        stderr.contains("warning: unsupported grammar option: superClass=MyLexerBase at 2:10"),
        "{stderr}"
    );
    assert!(stderr.contains("--option-hook KEY=VALUE"), "{stderr}");
    assert!(!unsupported_out.exists());

    let acknowledged_out = temp.path().join("acknowledged");
    let acknowledged = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        acknowledged_out.as_os_str(),
        OsStr::new("--option-hook"),
        OsStr::new("superClass=MyLexerBase"),
        OsStr::new("--require-full-semantics"),
    ]);
    assert!(
        acknowledged.status.success(),
        "stderr: {}",
        utf8(&acknowledged.stderr)
    );
    let stderr = utf8(&acknowledged.stderr);
    assert!(!stderr.contains("unsupported grammar option"), "{stderr}");
    assert!(
        !stderr.contains("require caller-owned target behavior"),
        "{stderr}"
    );
    assert!(acknowledged_out.join("options_lexer.rs").is_file());
}

/// End-to-end binary-parsing example: generate the MIDI recognizer from the
/// committed byte-oriented grammar, then parse a real Standard MIDI File
/// through a `ByteStream`. Exercises the whole binary path — raw high bytes as
/// codepoints, a `SemanticHooks` chunk-length superClass emitting synthesized
/// `END_OF_CHUNK` tokens (the `bencoding` pattern), and the generated parser.
#[test]
fn midi_binary_grammar_parses_standard_midi_file_over_byte_stream() {
    let temp = temporary_directory("midi-binary");
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/antlr4-rust-gen/midi-binary");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        dir.join("MidiLexer.g4").as_os_str(),
        dir.join("MidiParser.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        dir.join("patterns.toml").as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    // The bare `{beginChunk();}` lexer action lowers to a typed hook method.
    let lexer = fs::read_to_string(out.join("midi_lexer.rs")).expect("lexer should be emitted");
    assert!(lexer.contains("pub trait MidiLexerHooks"), "{lexer}");
    assert!(lexer.contains("fn begin_chunk"), "{lexer}");

    let fixture = dir.join("twinkle.mid");
    let fixture = fixture.to_str().expect("fixture path should be UTF-8");
    let test_source = format!(
        r####"
#[cfg(test)]
mod midi_tests {{
    use super::midi_lexer::{{MidiLexer, MidiLexerHooks, END_OF_CHUNK}};
    use super::midi_parser::MidiParser;
    use antlr4_runtime::{{
        ByteStream, CommonTokenStream, LexerLifecycleCtx, LexerSemCtx, Parser as _, Token as _,
    }};

    /// A minimal chunk-framing superClass: reads each MThd/MTrk chunk's declared
    /// byte length and synthesizes END_OF_CHUNK once the body is consumed — the
    /// "read N, then frame N bytes" pattern, in Rust, on plain `ByteStream`.
    #[derive(Default)]
    struct MidiHooks {{
        end_of_chunk: Option<usize>,
    }}

    impl MidiLexerHooks for MidiHooks {{
        fn begin_chunk<I>(&mut self, ctx: &mut LexerSemCtx<'_, I>)
        where
            I: antlr4_runtime::CharStream,
        {{
            // The header token just matched magic(4) + big-endian length(4).
            // Read the four length bytes RAW via lookbehind — `text_so_far()`
            // would return `ByteStream`'s hex rendering, not the bytes.
            let b3 = ctx.la(-4) as u32;
            let b2 = ctx.la(-3) as u32;
            let b1 = ctx.la(-2) as u32;
            let b0 = ctx.la(-1) as u32;
            let len = ((b3 << 24) | (b2 << 16) | (b1 << 8) | b0) as usize;
            self.end_of_chunk = Some(ctx.position() + len);
        }}

        fn lexer_before_token<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: antlr4_runtime::CharStream,
        {{
            // Fires after the previous body token was emitted and before the
            // next match — the clean point to close the chunk so END_OF_CHUNK
            // lands AFTER the last body token rather than inverting with it.
            if let Some(end) = self.end_of_chunk {{
                let pos = ctx.input_position();
                if pos >= end {{
                    self.end_of_chunk = None;
                    ctx.pop_mode();
                    ctx.enqueue_token(END_OF_CHUNK, pos.saturating_sub(1));
                }}
            }}
        }}
    }}

    fn parse(bytes: Vec<u8>) -> (Vec<i32>, usize) {{
        let lexer = MidiLexer::with_typed_hooks(ByteStream::new(bytes.clone()), MidiHooks::default());
        let mut stream = CommonTokenStream::new(lexer);
        stream.fill();
        let types: Vec<i32> = stream.tokens().map(|t| t.token_type()).collect();

        let lexer = MidiLexer::with_typed_hooks(ByteStream::new(bytes), MidiHooks::default());
        let mut parser = MidiParser::new(CommonTokenStream::new(lexer));
        parser.file().expect("well-formed MIDI parses");
        (types, parser.number_of_syntax_errors())
    }}

    #[test]
    fn parses_a_real_standard_midi_file() {{
        let bytes = include_bytes!({fixture:?}).to_vec();
        let (types, errors) = parse(bytes);

        // BEGIN_HEADER, six HDR_BYTE, END_OF_CHUNK; BEGIN_TRACK, four
        // (DELTA_TIME, event) pairs, END_OF_CHUNK; EOF (-1).
        assert_eq!(errors, 0, "no syntax errors on a well-formed file");
        assert_eq!(
            types,
            vec![
                2, // BEGIN_HEADER
                4, 4, 4, 4, 4, 4, // six HDR_BYTE (format, ntracks, division)
                1, // END_OF_CHUNK (MThd body framed by its length = 6)
                3, // BEGIN_TRACK
                5, 7, // delta, NOTE_ON
                5, 6, // delta, NOTE_OFF
                5, 9, // delta, META_SET_TEMPO
                5, 8, // delta, META_END_OF_TRACK
                1,  // END_OF_CHUNK (MTrk body framed by its length = 19)
                -1, // EOF
            ],
        );
    }}
}}
"####
    );

    assert_generated_project(
        temp.path(),
        &["midi_lexer.rs", "midi_parser.rs"],
        &test_source,
    );
}

#[test]
fn deeply_nested_input_parses_without_native_stack_overflow() {
    let temp = temporary_directory("deep-nesting");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/deep-nesting/Nest.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    // Every generated dispatch method must carry the stack guard; the rule
    // chain here multiplies each `[` into ~6 native rule frames (issue #193).
    let parser = fs::read_to_string(out.join("nest_parser.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("antlr4_runtime::grow_generated_rule_stack("),
        "generated dispatch must guard native stack growth\n{parser}"
    );

    assert_generated_project(
        temp.path(),
        &["nest_lexer.rs", "nest_parser.rs"],
        r#"
#[cfg(test)]
mod deep_nesting_tests {
    use super::nest_lexer::NestLexer;
    use super::nest_parser::{parse, NestParser};
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    fn nested(depth: usize) -> String {
        format!("{}a{}", "[".repeat(depth), "]".repeat(depth))
    }

    #[test]
    fn ten_thousand_levels_parse_on_the_default_test_stack() {
        // Rust test threads default to a 2 MiB stack; without segmented-stack
        // growth this depth aborted the process (issue #193).
        let parsed = parse(&nested(10_000), NestLexer::new, NestParser::s)
            .expect("deeply nested input should parse");
        assert!(parsed.tree().as_rule().is_some());
    }

    #[test]
    fn max_rule_depth_bounds_adversarial_nesting() {
        // Callers parsing untrusted input can cap rule nesting (issue #198):
        // shallow input parses, input past the cap fails with a positioned
        // error even though rule-level recovery would produce a tree, and the
        // violation does not leak into the parser's next parse. (No Nest rule
        // is ATN-preferred — the cap-overrides-ATN-preference guard is pinned
        // by the generator's dispatch-rendering unit test.)
        let lexer = NestLexer::new(InputStream::new(&nested(4)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        assert!(parser.s().is_ok(), "shallow input parses under the cap");

        let lexer = NestLexer::new(InputStream::new(&nested(1_000)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        let error = parser.s().expect_err("cap must reject deep nesting");
        assert!(
            error.to_string().contains("rule nesting depth limit of 64"),
            "unexpected error: {error}"
        );

        let lexer = NestLexer::new(InputStream::new(&nested(4)));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        assert!(
            parser.s().is_ok(),
            "reused parser starts clean after a depth violation"
        );
    }

    #[test]
    fn max_rule_depth_counts_left_recursive_expansions() {
        // `a+a+a+...` deepens the tree one level per operator without pushing
        // a rule frame. Upstream ANTLR fires a rule-entry listener event for
        // each expansion, so listener-based depth counters reject it — the
        // cap must too, or a 2000-term chain builds a 2000-deep tree under
        // any configured bound.
        let chain = vec!["a"; 2_000].join("+");
        let lexer = NestLexer::new(InputStream::new(&chain));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        let error = parser
            .s()
            .expect_err("operator expansions must count toward the cap");
        assert!(
            error.to_string().contains("rule nesting depth limit of 64"),
            "unexpected error: {error}"
        );

        // The same chain parses when uncapped, and a short chain fits.
        let lexer = NestLexer::new(InputStream::new(&chain));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        assert!(parser.s().is_ok(), "uncapped operator chain parses");
        let lexer = NestLexer::new(InputStream::new("a+a+a"));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        assert!(parser.s().is_ok(), "short operator chain fits the cap");
    }

    #[test]
    fn max_rule_depth_expansion_boundary_matches_frame_boundary() {
        // The expansion check runs BEFORE the expansion push, mirroring the
        // dispatch site's check before its rule-frame push: each extra
        // operator costs exactly one depth level. Pin that alignment by
        // finding the minimal cap admitting an N-operator chain and asserting
        // one more operator needs exactly one more level.
        fn parses_at(cap: usize, operators: usize) -> bool {
            let chain = vec!["a"; operators + 1].join("+");
            let lexer = NestLexer::new(InputStream::new(&chain));
            let mut parser = NestParser::new(CommonTokenStream::new(lexer));
            parser.set_max_rule_depth(Some(cap));
            parser.s().is_ok()
        }

        let minimal_cap = (1..128)
            .find(|cap| parses_at(*cap, 4))
            .expect("some cap admits a 4-operator chain");
        assert!(
            !parses_at(minimal_cap - 1, 4),
            "minimal cap should be exact"
        );
        assert!(
            !parses_at(minimal_cap, 5),
            "one more operator must exceed the same cap"
        );
        assert!(
            parses_at(minimal_cap + 1, 5),
            "one more operator must need exactly one more level"
        );
    }

    #[test]
    fn depth_violation_survives_recovery_and_does_not_poison_reuse() {
        // A violation absorbed by mid-tree recovery must still fail the parse
        // (the resource bound was hit), the reported error must be the depth
        // cap rather than a derived syntax error, and a second entry-rule
        // call on the same parser instance must not inherit the violation.
        let source = format!("{}a", "[".repeat(200));
        let lexer = NestLexer::new(InputStream::new(&source));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        let error = parser
            .s()
            .expect_err("depth violation must fail the parse even after recovery");
        assert!(
            error.to_string().contains("rule nesting depth limit of 64"),
            "depth cap must win over derived syntax errors: {error}"
        );

        let lexer = NestLexer::new(InputStream::new("a"));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        assert!(
            parser.expr().is_ok(),
            "a different entry rule on the same instance starts clean"
        );
    }

    /// The cel-rust `RecursionListener` shape: count `expr` nesting live,
    /// abort the parse past a limit — ported verbatim onto
    /// `add_parse_listener` (issue #202).
    struct RecursionListener {
        max: u16,
        depth: u16,
        high_water: std::sync::Arc<std::sync::atomic::AtomicU16>,
    }

    impl antlr4_runtime::ParseListener for RecursionListener {
        fn enter_every_rule(
            &mut self,
            event: &antlr4_runtime::EnterRuleEvent<'_>,
        ) -> Result<(), antlr4_runtime::AntlrError> {
            if event.rule_index == super::nest_parser::RULE_EXPR {
                self.depth += 1;
                self.high_water
                    .fetch_max(self.depth, std::sync::atomic::Ordering::Relaxed);
            }
            if self.depth > self.max {
                use antlr4_runtime::Token as _;
                let (line, column) = event
                    .current
                    .as_ref()
                    .map_or((0, 0), |token| (token.line(), token.column()));
                return Err(antlr4_runtime::AntlrError::ParserError {
                    line,
                    column,
                    message: format!("Recursion limit of {} exceeded", self.max),
                    offending: event.current.as_ref().map(antlr4_runtime::Token::token_id),
                });
            }
            Ok(())
        }

        fn exit_every_rule(&mut self, rule_index: usize) {
            if rule_index == super::nest_parser::RULE_EXPR {
                self.depth -= 1;
            }
        }
    }

    /// Records the event stream for order/balance assertions.
    struct TracingListener {
        tag: &'static str,
        events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl antlr4_runtime::ParseListener for TracingListener {
        fn enter_every_rule(
            &mut self,
            event: &antlr4_runtime::EnterRuleEvent<'_>,
        ) -> Result<(), antlr4_runtime::AntlrError> {
            self.events
                .lock()
                .expect("trace lock")
                .push(format!("enter{}:{}", self.tag, event.rule_index));
            Ok(())
        }

        fn exit_every_rule(&mut self, rule_index: usize) {
            self.events
                .lock()
                .expect("trace lock")
                .push(format!("exit{}:{}", self.tag, rule_index));
        }
    }

    #[test]
    fn parse_listener_counts_rules_and_aborts_past_a_limit() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU16, Ordering};

        // Under the limit: events fire, parse succeeds, enter/exit balance
        // (depth returns to zero, so high-water == max nesting seen).
        let high_water = Arc::new(AtomicU16::new(0));
        let lexer = NestLexer::new(InputStream::new(&nested(3)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 32,
            depth: 0,
            high_water: Arc::clone(&high_water),
        });
        assert!(parser.s().is_ok(), "shallow input parses under the limit");
        assert_eq!(
            high_water.load(Ordering::Relaxed),
            4,
            "one expr per bracket level plus the outermost expr"
        );

        // Successful left-recursive chain: the live depth counter returns to
        // its starting value. Proven through the public API: parse the same
        // under-limit chain twice with one listener instance — any residual
        // depth from parse one would raise parse two's high-water mark.
        let high_water = Arc::new(AtomicU16::new(0));
        let chain = vec!["a"; 20].join("+");
        let lexer = NestLexer::new(InputStream::new(&chain));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 1_000,
            depth: 0,
            high_water: Arc::clone(&high_water),
        });
        assert!(parser.s().is_ok(), "under-limit operator chain parses");
        let first_peak = high_water.load(Ordering::Relaxed);
        assert!(first_peak > 0, "the chain nests expr rules");
        let lexer = NestLexer::new(InputStream::new(&chain));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        assert!(parser.s().is_ok(), "same chain parses again");
        assert_eq!(
            high_water.load(Ordering::Relaxed),
            first_peak,
            "depth returned to zero after the successful LR parse"
        );

        // Past the limit: the listener aborts with its own positioned error,
        // sticky through recovery.
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 8,
            depth: 0,
            high_water: Arc::new(AtomicU16::new(0)),
        });
        let error = parser.s().expect_err("listener abort must fail the parse");
        assert!(
            error.to_string().contains("Recursion limit of 8 exceeded"),
            "unexpected error: {error}"
        );

        // Flat operator chains do NOT accumulate live listener depth: each
        // loop pass exits the outgoing iteration before the next expansion
        // enters (upstream recRuleSetPrevCtx), so a 40-term `a+a+...` chain
        // peaks at expr depth 2 in every ANTLR target — including this one —
        // and parses fine under a limit of 8.
        let chain = vec!["a"; 40].join("+");
        let high_water = Arc::new(AtomicU16::new(0));
        let lexer = NestLexer::new(InputStream::new(&chain));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 8,
            depth: 0,
            high_water: Arc::clone(&high_water),
        });
        assert!(
            parser.s().is_ok(),
            "flat operator chain stays at Java's live depth"
        );
        assert_eq!(
            high_water.load(Ordering::Relaxed),
            2,
            "operator chain peaks at depth 2, matching the Java oracle"
        );

        // The abort does not poison the instance: clearing listeners (which
        // also returns them and drops the sticky abort) and reusing the
        // parser parses clean input.
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 8,
            depth: 0,
            high_water: Arc::new(AtomicU16::new(0)),
        });
        let error = parser.s().expect_err("nested input exceeds the limit");
        assert!(
            error.to_string().contains("Recursion limit of 8 exceeded"),
            "unexpected error: {error}"
        );
        let lexer = NestLexer::new(InputStream::new("a"));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        let mut removed = parser.remove_parse_listeners();
        assert_eq!(removed.len(), 1, "removed listeners are handed back");
        assert!(parser.s().is_ok(), "reused parser starts clean");

        // Returned boxes re-register as-is (ParseListener is implemented for
        // Box<dyn ParseListener>), preserving accumulated listener state.
        let boxed = removed.pop().expect("one listener was removed");
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        parser.add_parse_listener(boxed);
        let error = parser
            .s()
            .expect_err("re-registered listener still enforces its limit");
        assert!(
            error.to_string().contains("Recursion limit of 8 exceeded"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parse_listener_event_order_matches_upstream() {
        use std::sync::{Arc, Mutex};

        // Two listeners: enters fire in registration order, exits in reverse
        // (upstream Parser.triggerExitRuleEvent walks back to front), and
        // pairs balance across recovery on malformed input.
        let events = Arc::new(Mutex::new(Vec::new()));
        let lexer = NestLexer::new(InputStream::new("[a]"));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(TracingListener {
            tag: "A",
            events: Arc::clone(&events),
        });
        parser.add_parse_listener(TracingListener {
            tag: "B",
            events: Arc::clone(&events),
        });
        assert!(parser.s().is_ok());
        let trace = events.lock().expect("trace lock").clone();
        let s_rule = super::nest_parser::RULE_S;
        assert_eq!(trace.first().map(String::as_str), Some(format!("enterA:{s_rule}").as_str()));
        assert_eq!(trace.get(1).map(String::as_str), Some(format!("enterB:{s_rule}").as_str()));
        // Last two events close the entry rule: B exits before A.
        assert_eq!(
            trace.last().map(String::as_str),
            Some(format!("exitA:{s_rule}").as_str())
        );
        assert_eq!(
            trace.get(trace.len() - 2).map(String::as_str),
            Some(format!("exitB:{s_rule}").as_str())
        );
        // Balance: every rule index enters exactly as often as it exits,
        // for both listeners.
        let count = |needle: &str| trace.iter().filter(|event| event.starts_with(needle)).count();
        assert_eq!(count("enterA:"), count("exitA:"));
        assert_eq!(count("enterB:"), count("exitB:"));

        // Recovery path: malformed input still balances.
        let events = Arc::new(Mutex::new(Vec::new()));
        let lexer = NestLexer::new(InputStream::new("[a"));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(TracingListener {
            tag: "R",
            events: Arc::clone(&events),
        });
        let _ = parser.s();
        let trace = events.lock().expect("trace lock").clone();
        let count = |needle: &str| trace.iter().filter(|event| event.starts_with(needle)).count();
        assert_eq!(
            count("enterR:"),
            count("exitR:"),
            "recovery keeps pairs balanced: {trace:?}"
        );

        // Successful left-recursive operator chain: each expansion fires a
        // simulated enter (upstream triggerEnterRuleEvent parity) and the
        // unroll fires the matching exits. `a+a+a+a` yields exactly 7
        // RULE_EXPR pairs: 1 rule dispatch + 3 expansions + 3 right-operand
        // dispatches.
        let events = Arc::new(Mutex::new(Vec::new()));
        let lexer = NestLexer::new(InputStream::new("a+a+a+a"));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(TracingListener {
            tag: "L",
            events: Arc::clone(&events),
        });
        assert!(parser.s().is_ok(), "operator chain parses");
        let trace = events.lock().expect("trace lock").clone();
        let expr_rule = super::nest_parser::RULE_EXPR;
        let count = |needle: String| trace.iter().filter(|event| **event == needle).count();
        assert_eq!(
            count(format!("enterL:{expr_rule}")),
            7,
            "expr enters = dispatch + expansions + operands: {trace:?}"
        );
        assert_eq!(
            count(format!("enterL:{expr_rule}")),
            count(format!("exitL:{expr_rule}")),
            "successful LR unroll balances expansion exits: {trace:?}"
        );
    }

    #[test]
    fn depth_cap_and_listener_abort_coexist() {
        // Whichever bound trips first surfaces; the other never fires because
        // the sticky abort stops rule entries (and thus stack growth). With a
        // tight listener limit the listener error wins the race...
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        parser.add_parse_listener(RecursionListener {
            max: 1,
            depth: 0,
            high_water: std::sync::Arc::new(std::sync::atomic::AtomicU16::new(0)),
        });
        let error = parser.s().expect_err("listener limit must fail the parse");
        assert!(
            error.to_string().contains("Recursion limit of 1 exceeded"),
            "listener abort surfaces when it trips first: {error}"
        );

        // ...and with a tight cap the depth violation wins the race.
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(8));
        parser.add_parse_listener(RecursionListener {
            max: 1_000,
            depth: 0,
            high_water: std::sync::Arc::new(std::sync::atomic::AtomicU16::new(0)),
        });
        let error = parser.s().expect_err("depth cap must fail the parse");
        assert!(
            error.to_string().contains("rule nesting depth limit of 8"),
            "depth-cap violation surfaces when it trips first: {error}"
        );

        let lexer = NestLexer::new(InputStream::new("a"));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(None);
        let _ = parser.remove_parse_listeners();
        assert!(parser.s().is_ok(), "instance is clean after either abort");
    }
}
"#,
    );
}

/// Issue #206: a grammar keeping its interpolation state inline in
/// `@lexer::members` — C# bodies, a nesting counter, and two stacks — generates
/// with **zero** hand-written hooks, purely from a `stack_member` pattern file.
///
/// The expected token streams below are not hand-derived: they were captured
/// from an ANTLR 4.13.2 **Java** lexer generated from the same grammar (bodies
/// mechanically ported to Java, semantics unchanged) and match byte-for-byte.
/// That is the differential validation the issue asks for — the stack state is
/// load-bearing in cases 4 and 5, where a scalar-only flag would leak across
/// adjacent strings.
#[test]
fn inline_lexer_member_stacks_generate_without_hooks() {
    let temp = temporary_directory("stack-member-lexer");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/stack-member-lexer");
    let out = temp.path().join("generated");

    // `--sem-unknown error` is the real assertion: generation fails if any
    // coordinate is left to a policy fallback or needs a hook.
    let output = run_antlr4_rust_gen(&[
        dir.join("CSharpInterpolation.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        dir.join("patterns.toml").as_os_str(),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let manifest = fs::read_to_string(out.join("semantics.json")).expect("manifest is emitted");
    assert!(
        !manifest.contains("\"hooked\""),
        "no coordinate may need a hook: {manifest}"
    );
    assert!(
        !manifest.contains("\"assume-true\"") && !manifest.contains("\"assume-false\""),
        "no coordinate may fall back to a policy: {manifest}"
    );

    // Member state lowers into the SemIR table, not inline Rust or a hook trait.
    let lexer =
        fs::read_to_string(out.join("c_sharp_interpolation.rs")).expect("lexer should be emitted");
    for expected in [
        "fn lexer_semantics()",
        "AStmt::PushMember",
        "AStmt::PopMember",
        "PExpr::MemberTop",
    ] {
        assert!(lexer.contains(expected), "missing {expected} in: {lexer}");
    }
    assert!(
        !lexer.contains("pub trait CSharpInterpolationHooks"),
        "grammar must need no hook trait: {lexer}"
    );

    let test_source = r####"
#[cfg(test)]
mod stack_member_tests {
    use super::c_sharp_interpolation::CSharpInterpolation;
    use antlr4_runtime::{CommonTokenStream, InputStream, IntStream as _, Token as _};

    fn lex(input: &str) -> String {
        let lexer = CSharpInterpolation::new(InputStream::new(input));
        let mut stream = CommonTokenStream::new(lexer);
        stream.fill();
        (0..stream.size())
            .filter_map(|index| stream.get(index))
            .map(|token| {
                format!("({}, {})", token.token_type(), token.text().unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Each expectation is the ANTLR 4.13.2 Java lexer's output for the same
    /// grammar and input.
    #[test]
    fn matches_the_java_oracle_token_stream() {
        // Regular string: `{ !verbatium }?` admits REGULAR_STRING_INSIDE (8).
        assert_eq!(
            lex(r#"$"abc""#),
            r#"(1, $") (8, abc) (7, ") (-1, <EOF>)"#
        );
        // Verbatim string: `{ verbatium }?` admits VERBATIUM_INSIDE_STRING (9)
        // and lets `""` lex as one token (6) instead of closing the string.
        assert_eq!(
            lex(r#"$@"a""b""#),
            r#"(2, $@") (9, a) (6, "") (9, b) (7, ") (-1, <EOF>)"#
        );
        // Interpolation hole: `{` pushes curlyLevels and DEFAULT_MODE.
        assert_eq!(
            lex(r#"$"a{x}b""#),
            r#"(1, $") (8, a) (3, x) (3, b) (-1, <EOF>)"#
        );
        // Verbatim then regular: popping must clear the flag, or `y` would
        // wrongly lex as VERBATIUM_INSIDE_STRING.
        assert_eq!(
            lex(r#"$@"x"$"y""#),
            r#"(2, $@") (9, x) (7, ") (1, $") (8, y) (7, ") (-1, <EOF>)"#
        );
        // Regular then verbatim: the second string's `""` must still be one
        // token, so the flag has to be restored per string, not left false.
        assert_eq!(
            lex(r#"$"p"$@"q""r""#),
            r#"(1, $") (8, p) (7, ") (2, $@") (9, q) (6, "") (9, r) (7, ") (-1, <EOF>)"#
        );
    }

    /// A reused lexer must not carry interpolation state across inputs.
    #[test]
    fn state_does_not_leak_between_inputs() {
        let first = lex(r#"$@"a""b""#);
        let second = lex(r#"$"c""#);
        assert_eq!(second, r#"(1, $") (8, c) (7, ") (-1, <EOF>)"#);
        assert_ne!(first, second);
    }
}
"####;

    assert_generated_project(temp.path(), &["c_sharp_interpolation.rs"], test_source);
}

/// Issue #206 review: a grammar whose inline member declares a non-default
/// initializer must honor it. `{ enabled }?` over `private bool enabled = true;`
/// has to pass on a fresh lexer; a slot silently starting at 0 would reject
/// input the source grammar accepts while still reporting itself `translated`.
///
/// The expected token stream is an ANTLR 4.13.2 **Java** lexer's output for the
/// same grammar (`(1, a) (2, b) (-1, <EOF>)`).
#[test]
fn declared_member_initializers_reach_the_generated_lexer() {
    let temp = temporary_directory("member-initializer");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/member-initializer");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        dir.join("L.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        dir.join("patterns.toml").as_os_str(),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let lexer = fs::read_to_string(out.join("l.rs")).expect("lexer should be emitted");
    assert!(
        lexer.contains(".with_initial_members([(0, 1)])"),
        "the declared initializer must seed the slot: {lexer}"
    );

    let test_source = r####"
#[cfg(test)]
mod member_initializer_tests {
    use super::l::L;
    use antlr4_runtime::{CommonTokenStream, InputStream, IntStream as _, Token as _};

    fn lex(input: &str) -> String {
        let lexer = L::new(InputStream::new(input));
        let mut stream = CommonTokenStream::new(lexer);
        stream.fill();
        (0..stream.size())
            .filter_map(|index| stream.get(index))
            .map(|token| {
                format!("({}, {})", token.token_type(), token.text().unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Matches the ANTLR 4.13.2 Java lexer for the same grammar and input.
    #[test]
    fn initialized_member_admits_its_guarded_rule() {
        assert_eq!(lex("ab"), "(1, a) (2, b) (-1, <EOF>)");
    }
}
"####;

    assert_generated_project(temp.path(), &["l.rs"], test_source);
}

/// Issue #206 review: a combined grammar's **parser** members need the same
/// initializer seeding the lexer got, and lexer/parser inventories must be
/// independent — a combined grammar may legally declare same-named members with
/// different kinds in each recognizer.
///
/// Expectations come from an ANTLR 4.13.2 **Java** parser built from the same
/// grammar: `"a"` parses with 0 syntax errors, `"b"` with 1.
#[test]
fn declared_parser_member_initializers_reach_the_generated_parser() {
    let temp = temporary_directory("parser-member-initializer");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/parser-member-initializer");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        dir.join("P.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        dir.join("patterns.toml").as_os_str(),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    // Each recognizer seeds its own slot 0 from its own inventory: the lexer's
    // `bool level = true` and the parser's `int level = 2`.
    let lexer = fs::read_to_string(out.join("p_lexer.rs")).expect("lexer should be emitted");
    assert!(
        lexer.contains(".with_initial_members([(0, 1)])"),
        "lexer must seed its own member: {lexer}"
    );
    let parser = fs::read_to_string(out.join("p_parser.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("base.set_initial_members([(0, 2)]);"),
        "parser must seed its own member: {parser}"
    );

    let test_source = r####"
#[cfg(test)]
mod parser_member_initializer_tests {
    use super::p_lexer::PLexer;
    use super::p_parser::PParser;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    fn syntax_errors(input: &str) -> usize {
        let lexer = PLexer::new(InputStream::new(input));
        let mut parser = PParser::new(CommonTokenStream::new(lexer));
        let _ = parser.s();
        parser.number_of_syntax_errors()
    }

    /// Matches the ANTLR 4.13.2 Java parser for the same grammar.
    ///
    /// `"a"` needs *both* declared initializers: the lexer's `level = true`
    /// admits `A`, and the parser's `level = 2` satisfies `{ level == 2 }?`.
    /// A slot silently starting at 0 on either side would fail it.
    #[test]
    fn both_recognizers_observe_their_own_declared_initial_values() {
        assert_eq!(syntax_errors("a"), 0);
        assert_eq!(syntax_errors("b"), 1);
    }
}
"####;

    assert_generated_project(temp.path(), &["p_lexer.rs", "p_parser.rs"], test_source);
}
