use std::process::{Command, Output};

fn run_antlr4_rust_gen(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_antlr4-rust-gen"))
        .args(args)
        .output()
        .expect("antlr4-rust-gen should run")
}

fn utf8(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("process output should be UTF-8")
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
