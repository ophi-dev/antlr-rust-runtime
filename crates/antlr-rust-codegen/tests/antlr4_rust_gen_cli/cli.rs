#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

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

    insta::assert_snapshot!("antlr4_rust_gen_help", utf8(&output.stdout));
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

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn generated_modules_enforce_codegen_api_compatibility() {
    let temp = temporary_directory("codegen-api-compatibility");
    let grammar = temp.path().join("CodegenApi.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar CodegenApi;\n\
         root : ID EOF ;\n\
         ID : [a-z]+ ;\n\
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

    let lexer_path = out.join("codegen_api_lexer.rs");
    let parser_path = out.join("codegen_api_parser.rs");
    let lexer = fs::read_to_string(&lexer_path).expect("lexer should be emitted");
    let parser = fs::read_to_string(&parser_path).expect("parser should be emitted");
    let lexer_check = lexer
        .lines()
        .find(|line| line.contains("__antlr4_rust_require_codegen_api"))
        .expect("lexer should require a generated-code API");
    let parser_check = parser
        .lines()
        .find(|line| line.contains("__antlr4_rust_require_codegen_api"))
        .expect("parser should require a generated-code API");
    let checks = format!("lexer: {lexer_check}\nparser: {parser_check}");
    insta::assert_snapshot!(
        "generated_codegen_api_checks",
        normalize_current_package_version(&checks)
    );

    let modules = ["codegen_api_lexer.rs", "codegen_api_parser.rs"];
    assert_generated_modules_compile(temp.path(), &modules);

    let current = format!(
        "__antlr4_rust_require_codegen_api!({},",
        antlr4_runtime::__ANTLR4_RUST_CODEGEN_API
    );
    let unsupported = "__antlr4_rust_require_codegen_api!(11,";
    let mut incompatible_parser = parser;
    let check_start = incompatible_parser
        .find(&current)
        .expect("parser check should contain the current API revision");
    incompatible_parser.replace_range(check_start..check_start + current.len(), unsupported);
    fs::write(parser_path, incompatible_parser).expect("parser should be writable");

    let output = run_generated_project(temp.path(), &modules, "");
    assert!(
        !output.status.success(),
        "previous generated-code API unexpectedly compiled"
    );
    assert_eq!(utf8(&output.stdout), "");
    let diagnostic = utf8(&output.stderr)
        .lines()
        .skip_while(|line| !line.starts_with("error:"))
        .take(2)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        diagnostic.contains("supports revision 12"),
        "diagnostic should name the supported revision: {diagnostic}"
    );
    insta::assert_snapshot!(
        "generated_codegen_api_mismatch_diagnostic",
        normalize_current_package_version(&diagnostic)
    );
}

#[test]
fn help_flag_as_option_value_is_not_intercepted() {
    let output = run_antlr4_rust_gen(&["--option-hook", "--help"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("--option-hook requires KEY=VALUE"));
    assert!(stderr.contains("For more information, try '--help'."));
}

#[test]
fn version_flags_as_option_values_are_not_intercepted() {
    for flag in ["--version", "-V"] {
        let output = run_antlr4_rust_gen(&["--option-hook", flag]);

        assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
        assert_eq!(utf8(&output.stdout), "");

        let stderr = utf8(&output.stderr);
        assert!(stderr.contains("--option-hook requires KEY=VALUE"));
        assert!(stderr.contains("For more information, try '--help'."));
    }
}

#[test]
fn missing_roots_report_usage_on_stderr() {
    let args: [&str; 0] = [];
    let output = run_antlr4_rust_gen(&args);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("required arguments were not provided"));
    assert!(stderr.contains("<ROOT.g4>..."));
    assert!(stderr.contains("Usage: antlr4-rust-gen"));
}

#[test]
fn unknown_arguments_report_usage_on_stderr() {
    let output = run_antlr4_rust_gen(&["--bogus"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");

    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("unexpected argument '--bogus'"));
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
            stderr.contains(&format!("unexpected argument '{flag}'")),
            "{stderr}"
        );
    }
}

#[test]
fn antlr_single_dash_generation_flags_are_rejected() {
    for flag in ["-listener", "-no-listener", "-visitor", "-no-visitor"] {
        let output = run_antlr4_rust_gen(&[flag, "T.g4"]);

        assert!(!output.status.success(), "{flag} unexpectedly succeeded");
        let stderr = utf8(&output.stderr);
        assert!(stderr.contains("unexpected argument"), "{stderr}");
        assert!(stderr.contains("Usage: antlr4-rust-gen"), "{stderr}");
    }
}

#[test]
fn option_hook_requires_a_key_value_assignment() {
    let output = run_antlr4_rust_gen(&["--option-hook", "superClass"]);

    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");
    let stderr = utf8(&output.stderr);
    assert!(stderr.contains("--option-hook requires KEY=VALUE"));
    assert!(stderr.contains("For more information, try '--help'."));
}
