use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn run_antlr4_rust_gen(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_antlr4-rust-gen"))
        .args(args)
        .output()
        .expect("antlr4-rust-gen should run")
}

fn utf8(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("process output should be UTF-8")
}

fn temporary_grammar_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "antlr4-rust-gen-options-{}-{nonce}.g4",
        std::process::id()
    ))
}

#[test]
fn long_help_exits_successfully_on_stdout() {
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
        stdout.starts_with("Usage: antlr4-rust-gen [OPTIONS]\n"),
        "{stdout}"
    );
    assert!(stdout.contains("  --lexer Lexer.interp"), "{stdout}");
    assert!(stdout.contains("  --option-hook KEY=VALUE"), "{stdout}");
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
    let output = run_antlr4_rust_gen(&["--lexer-name", "--help"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("at least one of --lexer or --parser is required"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn missing_inputs_still_report_usage_on_stderr() {
    let output = run_antlr4_rust_gen(&[]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("at least one of --lexer or --parser is required"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn unknown_arguments_still_report_usage_on_stderr() {
    let output = run_antlr4_rust_gen(&["--bogus"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("unknown argument --bogus"));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
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
fn unsupported_grammar_options_warn_and_exact_hooks_acknowledge_them() {
    let grammar = temporary_grammar_path();
    fs::write(
        &grammar,
        "lexer grammar L;\noptions { superClass = MyLexerBase; }\nA: 'a';\n",
    )
    .expect("temporary grammar should be writable");
    let missing_interp = grammar.with_extension("missing.interp");
    let grammar = grammar
        .to_str()
        .expect("temporary grammar path should be UTF-8");
    let missing_interp = missing_interp
        .to_str()
        .expect("temporary interp path should be UTF-8");

    let unsupported = run_antlr4_rust_gen(&[
        "--lexer",
        missing_interp,
        "--grammar",
        grammar,
        "--require-full-semantics",
    ]);
    assert!(!unsupported.status.success());
    let stderr = utf8(&unsupported.stderr);
    assert!(
        stderr.contains("warning: unsupported grammar option: superClass=MyLexerBase at 2:10"),
        "{stderr}"
    );
    assert!(stderr.contains("--option-hook KEY=VALUE"), "{stderr}");

    let acknowledged = run_antlr4_rust_gen(&[
        "--lexer",
        missing_interp,
        "--grammar",
        grammar,
        "--option-hook",
        "superClass=MyLexerBase",
        "--require-full-semantics",
    ]);
    assert!(!acknowledged.status.success());
    let stderr = utf8(&acknowledged.stderr);
    assert!(!stderr.contains("unsupported grammar option"), "{stderr}");
    assert!(
        !stderr.contains("require caller-owned target behavior"),
        "{stderr}"
    );

    fs::remove_file(grammar).expect("temporary grammar should be removable");
}
