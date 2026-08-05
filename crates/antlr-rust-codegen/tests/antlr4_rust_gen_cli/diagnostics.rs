#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

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

/// Issue #236: lexer left-corner cycles are grammar errors, not parser
/// precedence rules. Diagnose direct and mutual cycles before DFA construction;
/// ordinary recursion after a consuming transition remains covered by the
/// `lexer-recursion` ATN parity fixture.
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn lexer_left_recursion_reports_each_cycle_without_partial_outputs() {
    let temp = temporary_directory("lexer-left-recursion");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/lexer-left-recursion/LexerLeftRecursion.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    assert_eq!(utf8(&output.stdout), "");
    let stderr = replace_miette_path(
        utf8(&output.stderr),
        grammar
            .to_str()
            .expect("fixture path should be valid Unicode"),
        "<grammar>",
    );
    insta::assert_snapshot!(
        "lexer_left_recursion_diagnostics",
        normalize_cli_snapshot(&stderr)
    );
    assert!(!out.exists(), "failed compilation emitted output");
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
    let missing_semicolon = stderr
        .split_once("Error: G4F003")
        .and_then(|(_, remainder)| remainder.split("\nError:").next())
        .expect("missing-semicolon diagnostic should be rendered");
    assert!(
        missing_semicolon.contains(&fixture.join("Delegate.g4").display().to_string()),
        "{stderr}"
    );
    assert!(
        !missing_semicolon.contains(&format!("{}:", root.display())),
        "{stderr}"
    );
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

/// Issue #151, decline path: a cycle the transform must *not* rewrite still
/// reports the pre-existing `G4A005` mutual-left-recursion diagnostic, naming
/// both original rules. This is the guard that the transform is additive — it
/// either produces a grammar the verified direct-recursion path accepts, or it
/// changes nothing observable.
#[test]
fn undecidable_mutual_left_recursion_still_reports_the_cycle() {
    let temp = temporary_directory("mutual-left-recursion-declined");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/mutual-left-recursion/DeclinedCycle.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        !output.status.success(),
        "a declined cycle must not generate a parser\nstdout: {}",
        utf8(&output.stdout)
    );
    let stderr = utf8(&output.stderr);
    assert!(
        stderr.contains("G4A005"),
        "declining must fall through to the cycle detector: {stderr}"
    );
    // Both cycle members are still present and named, i.e. nothing was inlined
    // or deleted on the way to the diagnostic.
    assert!(
        stderr.contains("mutually left-recursive rules: [a, b]"),
        "the diagnostic must name the original rule set: {stderr}"
    );
    assert!(
        !out.join("declined_cycle_parser.rs").exists(),
        "no parser artifact should be emitted for a declined cycle"
    );
}

#[test]
fn symbol_conflicts_are_reported_against_the_authored_grammar() {
    // The mutual-left-recursion rewrite deletes hub-only satellites, and a
    // return value named after one (`e returns [i32 s]` vs rule `s`) must
    // still be reported: symbol validation reads a snapshot taken before the
    // rewrite runs.
    let temp = temporary_directory("mutual-left-recursion-symbol-clash");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/mutual-left-recursion/ReturnsClash.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        !output.status.success(),
        "a symbol conflict must fail generation even when the conflicting rule \
         is a deletable cycle satellite\nstdout: {}",
        utf8(&output.stdout)
    );
    let stderr = utf8(&output.stderr);
    assert!(
        stderr.contains("G4S057"),
        "the return-value/rule-name conflict must be diagnosed: {stderr}"
    );
}
