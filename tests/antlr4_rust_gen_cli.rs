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
    fs::write(source.join("lib.rs"), declarations)
        .expect("generated-module crate root should be writable");
    for module in modules {
        fs::copy(temp_dir.join("generated").join(module), source.join(module))
            .expect("generated module should be copied into the check crate");
    }

    let output = Command::new(env!("CARGO"))
        .args([
            "check",
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
        "generated modules did not compile\nstdout: {}\nstderr: {}",
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
    assert!(!stdout.contains("--lexer "), "{stdout}");
    assert!(!stdout.contains("--parser "), "{stdout}");
    assert!(!stdout.contains("--grammar "), "{stdout}");
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
fn help_flag_as_option_value_is_not_intercepted() {
    let output = run_antlr4_rust_gen(&["--option-hook", "--help"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("--option-hook requires KEY=VALUE"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
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
fn combined_root_emits_standard_typed_contexts_and_listener() {
    let temp = temporary_directory("combined-contexts");
    let grammar = temp.path().join("Shapes.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar Shapes;\n\
         start: first=atom # Single | rest+=atom+ # Many;\n\
         atom: ID;\n\
         ID: [a-z]+;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
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
    assert!(out.join("shapes_lexer.rs").is_file());
    let parser =
        fs::read_to_string(out.join("shapes_parser.rs")).expect("parser should be emitted");
    for expected in [
        "pub struct StartContext<'a>",
        "pub struct SingleContext<'a>",
        "pub struct ManyContext<'a>",
        "pub trait ShapesListener",
        "fn enter_single(&mut self",
        "fn enter_many(&mut self",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }
    assert!(
        !parser.contains("antlr4_runtime::{{"),
        "generated imports must not contain redundant nested braces\n{parser}"
    );
    assert_generated_modules_compile(temp.path(), &["shapes_lexer.rs", "shapes_parser.rs"]);
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
        "pub struct ObjectCreationExpressionContext<'a>",
        "pub struct ObjectCreationExpressionLabelContext<'a>",
        "fn enter_object_creation_expression(&mut self",
        "fn enter_object_creation_expression_label(&mut self",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }
    assert_generated_modules_compile(temp.path(), &["t.rs"]);
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
