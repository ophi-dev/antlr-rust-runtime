#![allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

fn write_combined_grammar(directory: &Path) -> PathBuf {
    let grammar = directory.join("TestRig.g4");
    fs::write(
        &grammar,
        "grammar TestRig;\n\
         start : ID EOF ;\n\
         ID : [a-z]+ ;\n\
         WS : [ \\t\\r\\n]+ -> skip ;\n",
    )
    .expect("grammar should be writable");
    grammar
}

fn normalize_test_directory(value: &str, directory: &Path) -> String {
    normalize_cli_snapshot(&value.replace(&directory.display().to_string(), "<test-directory>"))
}

#[test]
fn help_describes_test_runner_inputs_and_modes() {
    let output = run_antlr4_rust_testrig(&["--help"]);

    assert!(output.status.success(), "stderr: {}", utf8(&output.stderr));
    assert_eq!(utf8(&output.stderr), "");
    insta::assert_snapshot!("antlr4_rust_testrig_help", utf8(&output.stdout));
}

#[test]
fn parser_runner_prints_requested_views_and_fails_on_each_error_source() {
    let directory = temporary_directory("testrig-parser");
    let grammar = write_combined_grammar(directory.path());
    let grammar_without_extension = grammar.with_extension("");
    let valid = directory.path().join("valid.txt");
    let parser_error = directory.path().join("parser-error.txt");
    let lexer_error = directory.path().join("lexer-error.txt");
    fs::write(&valid, "hello\n").expect("valid input should be writable");
    fs::write(&parser_error, "hello world\n").expect("parser-error input should be writable");
    fs::write(&lexer_error, "@\n").expect("lexer-error input should be writable");

    let valid_output = run_antlr4_rust_testrig(&[
        grammar_without_extension.as_os_str(),
        OsStr::new("start"),
        OsStr::new("--tokens"),
        OsStr::new("--tree"),
        OsStr::new("--trace"),
        OsStr::new("--diagnostics"),
        OsStr::new("--sll"),
        valid.as_os_str(),
    ]);
    assert!(
        valid_output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&valid_output.stdout),
        utf8(&valid_output.stderr)
    );
    insta::assert_snapshot!("parser_runner_stdout", utf8(&valid_output.stdout));
    insta::assert_snapshot!("parser_runner_trace", utf8(&valid_output.stderr));

    let parser_output = run_antlr4_rust_testrig(&[
        grammar.as_os_str(),
        OsStr::new("start"),
        parser_error.as_os_str(),
    ]);
    assert!(
        !parser_output.status.success(),
        "parser syntax errors must fail the runner"
    );
    insta::assert_snapshot!(
        "parser_syntax_error",
        normalize_test_directory(utf8(&parser_output.stderr), directory.path())
    );

    let lexer_output = run_antlr4_rust_testrig(&[
        grammar.as_os_str(),
        OsStr::new("start"),
        lexer_error.as_os_str(),
    ]);
    assert!(
        !lexer_output.status.success(),
        "lexer syntax errors must fail the runner"
    );
    insta::assert_snapshot!(
        "lexer_syntax_error",
        normalize_test_directory(utf8(&lexer_output.stderr), directory.path())
    );
}

#[test]
fn stdin_and_split_grammars_are_supported() {
    let directory = temporary_directory("testrig-split");
    let lexer = directory.path().join("SplitLexer.g4");
    let parser = directory.path().join("SplitParser.g4");
    fs::write(
        &lexer,
        "lexer grammar SplitLexer;\n\
         ID : [a-z]+ ;\n\
         WS : [ \\t\\r\\n]+ -> skip ;\n",
    )
    .expect("lexer grammar should be writable");
    fs::write(
        &parser,
        "parser grammar SplitParser;\n\
         options { tokenVocab = SplitLexer; }\n\
         start : ID EOF ;\n",
    )
    .expect("parser grammar should be writable");

    let output = run_antlr4_rust_testrig_with_stdin(
        &[
            parser.as_os_str(),
            OsStr::new("start"),
            OsStr::new("--lexer-grammar"),
            lexer.as_os_str(),
            OsStr::new("--lib"),
            directory.path().as_os_str(),
            OsStr::new("--tree"),
        ],
        b"split\n",
    );

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    insta::assert_snapshot!("split_parser_tree", utf8(&output.stdout));
    assert_eq!(utf8(&output.stderr), "");
}

#[test]
fn lexer_only_and_unknown_rule_failures_are_nonzero() {
    let directory = temporary_directory("testrig-lexer");
    let lexer = directory.path().join("Main.g4");
    fs::write(
        &lexer,
        "lexer grammar Main;\n\
         ID : [a-z]+ ;\n\
         WS : [ \\t\\r\\n]+ -> skip ;\n",
    )
    .expect("lexer grammar should be writable");

    let lexer_output =
        run_antlr4_rust_testrig_with_stdin(&[lexer.as_os_str(), OsStr::new("tokens")], b"@\n");
    assert!(!lexer_output.status.success());
    insta::assert_snapshot!(
        "lexer_only_syntax_error",
        normalize_cli_snapshot(utf8(&lexer_output.stderr))
    );

    let grammar = write_combined_grammar(directory.path());
    let rule_output = run_antlr4_rust_testrig(&[grammar.as_os_str(), OsStr::new("missing")]);
    assert!(!rule_output.status.success());
    insta::assert_snapshot!(
        "unknown_start_rule_diagnostic",
        normalize_cli_snapshot(utf8(&rule_output.stderr))
    );
}

#[test]
fn multiple_inputs_continue_after_an_error_and_return_failure() {
    let directory = temporary_directory("testrig-multiple");
    let grammar = write_combined_grammar(directory.path());
    let first = directory.path().join("first.txt");
    let invalid = directory.path().join("invalid.txt");
    let missing = directory.path().join("missing.txt");
    let last = directory.path().join("last.txt");
    fs::write(&first, "first\n").expect("first input should be writable");
    fs::write(&invalid, "two words\n").expect("invalid input should be writable");
    fs::write(&last, "last\n").expect("last input should be writable");

    let output = run_antlr4_rust_testrig(&[
        grammar.as_os_str(),
        OsStr::new("start"),
        OsStr::new("--tree"),
        first.as_os_str(),
        invalid.as_os_str(),
        missing.as_os_str(),
        last.as_os_str(),
    ]);

    assert!(!output.status.success());
    let stdout = utf8(&output.stdout);
    assert!(stdout.contains("(start first <EOF>)"));
    assert!(stdout.contains("(start two words <EOF>)"));
    assert!(stdout.contains("(start last <EOF>)"));
    let stderr = utf8(&output.stderr);
    assert!(stderr.contains(&first.display().to_string()));
    assert!(stderr.contains(&invalid.display().to_string()));
    assert!(stderr.contains(&missing.display().to_string()));
    assert!(stderr.contains(&last.display().to_string()));
    assert!(stderr.contains("extraneous input 'words' expecting <EOF>"));
    assert!(stderr.contains("failed to open input"), "{stderr}");
}
