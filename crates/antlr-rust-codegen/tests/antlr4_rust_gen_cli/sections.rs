// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
//! Issue #355: authored target-code sections (named actions and rule
//! exception clauses) must execute, be routed through a documented
//! disposition, or fail generation — never disappear silently.
#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

fn generate_embedded_strict(grammar: &Path, out: &Path) -> Output {
    run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--require-generated-parser"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ])
}

/// The full ANTLR rule lifecycle for ordinary rules: `@after` runs on the
/// committed path only, an authored `catch` replaces the default
/// report-and-recover handler, and `finally` runs exactly once on the
/// success, recovered-error, and caught-error paths.
#[test]
fn catch_finally_and_after_follow_antlr_lifecycle_ordering() {
    let temp = temporary_directory("section-lifecycle");
    let grammar = temp.path().join("SectionLifecycle.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        r#"grammar SectionLifecycle;

start
@init { crate::record_event("init"); }
@after { crate::record_event("after"); }
    : A B EOF
    ;
finally {
    crate::record_event("finally");
    return;
}

caught
    : A B EOF
    ;
catch [error] {
    let _ = &error;
    crate::record_event("catch");
}

rawIdent
    : A B EOF
    ;
catch [r#type] {
    let _ = &r#type;
    crate::record_event("raw-identifier-catch");
}

caughtWithFinally
    : A B EOF
    ;
catch [error] {
    let _ = &error;
    crate::record_event("catch-before-finally");
    return;
}
finally {
    crate::record_event("finally-after-catch");
}

A: 'a';
B: 'b';
WS: [ \t\r\n]+ -> skip;
"#,
    )
    .expect("grammar should be writable");

    let output = generate_embedded_strict(&grammar, &out);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    insta::assert_snapshot!("section_lifecycle_semantics_manifest", manifest);

    let parser = fs::read_to_string(out.join("section_lifecycle_parser.rs"))
        .expect("parser should be emitted");
    // The propagated-failure slot carries the authored `finally`, and the
    // authored catch replaces the default handler.
    assert!(
        parser.contains("propagate {"),
        "missing propagate section:\n{parser}"
    );
    assert!(
        parser.contains("exception (|error| {"),
        "missing authored catch handler:\n{parser}"
    );
    assert!(
        parser.contains("exception (|r#type| {"),
        "raw-identifier catch argument should bind verbatim:\n{parser}"
    );

    let test_source = r####"
use std::cell::RefCell;

thread_local! {
    static EVENTS: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

pub fn record_event(event: &str) {
    EVENTS.with(|events| events.borrow_mut().push(event.to_owned()));
}

#[cfg(test)]
fn take_events() -> Vec<String> {
    EVENTS.with(|events| events.borrow_mut().drain(..).collect())
}

#[cfg(test)]
mod section_lifecycle_tests {
    use super::section_lifecycle_lexer::SectionLifecycleLexer;
    use super::section_lifecycle_parser::SectionLifecycleParser;
    use super::take_events;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    macro_rules! parser {
        ($input:expr) => {
            SectionLifecycleParser::new(CommonTokenStream::new(SectionLifecycleLexer::new(
                InputStream::new($input),
            )))
        };
    }

    #[test]
    fn success_runs_init_after_then_finally() {
        let mut parser = parser!("ab");
        parser.start().expect("clean input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        assert_eq!(take_events(), ["init", "after", "finally"]);
    }

    #[test]
    fn recovered_error_skips_after_but_runs_finally_once() {
        let mut parser = parser!("aa");
        parser
            .start()
            .expect("default recovery should still produce a tree");
        assert!(parser.number_of_syntax_errors() > 0);
        assert_eq!(take_events(), ["init", "finally"]);
    }

    #[test]
    fn authored_catch_replaces_default_recovery() {
        let mut parser = parser!("aa");
        parser.caught().expect("caught rule completes normally");
        // ANTLR emits the authored clause instead of the default
        // report-and-recover handler, so no syntax error is recorded.
        assert_eq!(parser.number_of_syntax_errors(), 0);
        assert_eq!(take_events(), ["catch"]);
    }

    #[test]
    fn authored_catch_does_not_run_on_success() {
        let mut parser = parser!("ab");
        parser.caught().expect("clean input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        assert!(take_events().is_empty());
    }

    #[test]
    fn raw_identifier_catch_argument_binds_verbatim() {
        let mut parser = parser!("aa");
        parser.raw_ident().expect("caught rule completes normally");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        assert_eq!(take_events(), ["raw-identifier-catch"]);
    }

    #[test]
    fn finally_runs_after_catch_even_when_the_handler_returns_early() {
        // Java runs `finally` after the catch clause, including when the
        // catch body returns; an authored `return` exits only the handler.
        let mut parser = parser!("aa");
        parser
            .caught_with_finally()
            .expect("caught rule completes normally");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        assert_eq!(
            take_events(),
            ["catch-before-finally", "finally-after-catch"]
        );
    }

    #[test]
    fn finally_still_runs_on_success_for_catch_rules() {
        let mut parser = parser!("ab");
        parser
            .caught_with_finally()
            .expect("clean input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        assert_eq!(take_events(), ["finally-after-catch"]);
    }
}
"####;
    assert_generated_project(
        temp.path(),
        &["section_lifecycle_lexer.rs", "section_lifecycle_parser.rs"],
        test_source,
    );
}

/// Ordinary and left-recursive rules share one lifecycle contract: a
/// left-recursive rule's operator loop is one rule invocation, so its
/// `finally` runs exactly once however many expansions the loop takes.
#[test]
fn finally_runs_once_per_left_recursive_rule_invocation() {
    let temp = temporary_directory("section-left-recursion");
    let grammar = temp.path().join("SectionExpr.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        r#"grammar SectionExpr;

start
    : expr EOF
    ;

expr
    : expr PLUS expr
    | ID
    ;
finally {
    crate::record_event("expr-finally");
}

ID: [a-z]+;
PLUS: '+';
WS: [ \t\r\n]+ -> skip;
"#,
    )
    .expect("grammar should be writable");

    let output = generate_embedded_strict(&grammar, &out);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let test_source = r####"
use std::cell::RefCell;

thread_local! {
    static EVENTS: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

pub fn record_event(event: &str) {
    EVENTS.with(|events| events.borrow_mut().push(event.to_owned()));
}

#[cfg(test)]
mod left_recursion_tests {
    use super::EVENTS;
    use super::section_expr_lexer::SectionExprLexer;
    use super::section_expr_parser::SectionExprParser;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    fn finally_count(input: &str) -> usize {
        let mut parser = SectionExprParser::new(CommonTokenStream::new(SectionExprLexer::new(
            InputStream::new(input),
        )));
        parser.start().expect("input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        EVENTS.with(|events| events.borrow_mut().drain(..).count())
    }

    #[test]
    fn operator_chain_is_one_rule_invocation() {
        // `a` enters `expr` once. `a+a+a` enters the rule function three
        // times: once for the top-level invocation whose operator loop
        // expands twice, plus once per right operand — never once per loop
        // pass beyond those entries.
        assert_eq!(finally_count("a"), 1);
        assert_eq!(finally_count("a+a+a"), 3);
    }
}
"####;
    assert_generated_project(
        temp.path(),
        &["section_expr_lexer.rs", "section_expr_parser.rs"],
        test_source,
    );
}

/// Supported `@header` / `@definitions` bodies appear exactly once at their
/// documented module positions: `@header` before the generated imports,
/// `@definitions` at module scope after the `@members` module items.
#[test]
fn header_and_definitions_emit_once_at_documented_positions() {
    let temp = temporary_directory("section-header");
    let grammar = temp.path().join("Header.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        r#"grammar Header;

@header {
    const HEADER_SENTINEL: i32 = 7;
}

@parser::definitions {
    fn definition_sentinel() -> i32 { HEADER_SENTINEL }
}

@parser::members {
    fn use_definitions(&mut self) -> i32 { definition_sentinel() }
}

start: A EOF;
A: 'a';
"#,
    )
    .expect("grammar should be writable");

    let output = generate_embedded_strict(&grammar, &out);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let parser =
        fs::read_to_string(out.join("header_parser.rs")).expect("parser should be emitted");
    assert_eq!(
        parser.matches("const HEADER_SENTINEL: i32 = 7;").count(),
        1,
        "@header must be emitted exactly once:\n{parser}"
    );
    assert_eq!(
        parser.matches("fn definition_sentinel").count(),
        1,
        "@definitions must be emitted exactly once:\n{parser}"
    );
    let header_position = parser
        .find("const HEADER_SENTINEL")
        .expect("header body is present");
    let first_import = parser.find("use antlr4_runtime").expect("imports exist");
    assert!(
        header_position < first_import,
        "@header must precede the generated imports:\n{parser}"
    );
    // The unscoped @header belongs to the parser module of a combined
    // grammar; the lexer module must not duplicate it.
    let lexer = fs::read_to_string(out.join("header_lexer.rs")).expect("lexer should be emitted");
    assert!(
        !lexer.contains("HEADER_SENTINEL"),
        "@header must not be duplicated into the lexer module:\n{lexer}"
    );

    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    for expected in [
        r#""name": "header""#,
        r#""name": "definitions""#,
        r#""disposition": "embedded""#,
    ] {
        assert!(
            manifest.contains(expected),
            "missing {expected} in manifest:\n{manifest}"
        );
    }

    assert_generated_modules_compile(temp.path(), &["header_lexer.rs", "header_parser.rs"]);
}

fn write_section_audit_grammar(path: &Path) {
    fs::write(
        path,
        r#"grammar SectionAudit;

@header {
    const AUDIT: i32 = 1;
}

@lexer::members {
    fn lexer_helper(&mut self) {}
}

@parser::tokenfactory {
    unknown_section();
}

start
@init { let _ = AUDIT; }
    : A EOF
    ;
catch [not a valid binding list] {
    let _ = ();
}

multi: A EOF;
catch [first] { let _ = &first; }
catch [second] { let _ = &second; }

narrowed: A EOF;
catch [FailedPredicateException fpe] {
    let _ = &fpe;
}

A: 'a';
"#,
    )
    .expect("grammar should be writable");
}

/// Every authored section receives a manifest row: supported ones are
/// `embedded`, everything else is `unsupported` and warns without failing a
/// default (non-strict) run.
#[test]
fn unsupported_sections_warn_and_stay_manifest_visible() {
    let temp = temporary_directory("section-audit");
    let grammar = temp.path().join("SectionAudit.g4");
    let out = temp.path().join("generated");
    write_section_audit_grammar(&grammar);

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
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
    let stderr = utf8(&output.stderr);
    for expected in [
        "warning: unsupported target-code section: @lexer::members",
        "warning: unsupported target-code section: @parser::tokenfactory",
        "warning: unsupported target-code section: rule start(0) catch[...]",
        "warning: unsupported target-code section: rule multi(1) catch[...]",
    ] {
        assert!(expected.is_ascii());
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in {stderr}"
        );
    }

    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    insta::assert_snapshot!("section_audit_semantics_manifest", manifest);
}

/// `--require-full-semantics` rejects every unsupported authored section with
/// a source-positioned diagnostic and remediation guidance.
#[test]
fn require_full_semantics_rejects_unsupported_sections() {
    let temp = temporary_directory("section-audit-strict");
    let grammar = temp.path().join("SectionAudit.g4");
    let out = temp.path().join("generated");
    write_section_audit_grammar(&grammar);

    let output = generate_embedded_strict(&grammar, &out);
    assert!(
        !output.status.success(),
        "unsupported sections must fail strict generation\nstdout: {}",
        utf8(&output.stdout)
    );
    let stderr = utf8(&output.stderr);
    // Source path, line, column, section kind, body, and remediation — for
    // every unsupported section across both recognizers in one run.
    for expected in [
        "unsupported target-code section: @lexer::members at ",
        "SectionAudit.g4:7:",
        "lexer-scoped sections are not implemented in embedded mode",
        "unsupported target-code section: @parser::tokenfactory at ",
        "unsupported target-code section: rule start(0) catch[...] at ",
        "catch argument must be a single Rust identifier",
        "unsupported target-code section: rule multi(1) catch[...] at ",
        "multiple catch clauses are not supported",
        // Typed clauses cannot narrow: the generated handler receives every
        // recognition error, so they are rejected rather than mistranslated.
        "unsupported target-code section: rule narrowed(2) catch[...] at ",
        "--require-full-semantics: 6 target-code section(s) would be silently dropped",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in {stderr}"
        );
    }
}

/// Templates mode executes no authored target code, so the strict flag must
/// reject the sections it would silently drop; the default run keeps
/// succeeding with warnings.
#[test]
fn templates_mode_rejects_authored_sections_under_strict_flag() {
    let temp = temporary_directory("section-templates");
    let grammar = temp.path().join("TemplateSections.g4");
    fs::write(
        &grammar,
        r#"grammar TemplateSections;

@header {
    package com.example;
}

start
@init { setup(); }
    : A EOF
    ;
finally {
    cleanup();
}

A: 'a';
"#,
    )
    .expect("grammar should be writable");

    let lenient = temp.path().join("generated-lenient");
    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        lenient.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let stderr = utf8(&output.stderr);
    for expected in [
        "warning: unsupported target-code section: @header",
        "warning: unsupported target-code section: rule start(0) @init",
        "warning: unsupported target-code section: rule start(0) finally",
        "templates mode does not execute authored target-code sections",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in {stderr}"
        );
    }

    let strict = temp.path().join("generated-strict");
    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        strict.as_os_str(),
    ]);
    assert!(
        !output.status.success(),
        "templates mode must reject dropped sections under the strict flag"
    );
    let stderr = utf8(&output.stderr);
    for expected in [
        "unsupported target-code section: @header at ",
        "TemplateSections.g4:3:",
        "--require-full-semantics: 3 target-code section(s) would be silently dropped",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in {stderr}"
        );
    }
}

/// A section scoped to a recognizer this invocation never generates from the
/// grammar (here: `@parser::members` in a standalone lexer grammar) cannot
/// silently vanish — it is inventoried as unsupported by the recognizer that
/// carries it.
#[test]
fn cross_scoped_sections_without_a_counterpart_are_unsupported() {
    let temp = temporary_directory("section-no-counterpart");
    let grammar = temp.path().join("SoloLexer.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "lexer grammar SoloLexer;\n\n@parser::members {\n    fn orphaned(&mut self) {}\n}\n\nA: 'a';\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
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
    let stderr = utf8(&output.stderr);
    for expected in [
        "warning: unsupported target-code section: @parser::members at ",
        "scoped to a recognizer this invocation does not generate",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in {stderr}"
        );
    }

    let strict_out = temp.path().join("generated-strict");
    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        strict_out.as_os_str(),
    ]);
    assert!(
        !output.status.success(),
        "orphaned cross-scoped sections must fail strict generation"
    );
    assert!(
        utf8(&output.stderr).contains("unsupported target-code section: @parser::members at "),
        "stderr: {}",
        utf8(&output.stderr)
    );
}

/// Sections survive import resolution: an imported grammar's `@members` and a
/// cloned rule's `finally` clause stay inventoried (and executed) in the
/// importing recognizer, attributed to the imported source file.
#[test]
fn imported_grammar_sections_are_inventoried() {
    let temp = temporary_directory("section-imports");
    let root = temp.path().join("Root.g4");
    let sub = temp.path().join("Sub.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        r#"grammar Root;
import Sub;

start: item EOF;

A: 'a';
"#,
    )
    .expect("root grammar should be writable");
    fs::write(
        &sub,
        r#"parser grammar Sub;

@members {
    fn imported_helper(&mut self) {}
}

item: A;
finally {
    self.imported_helper();
}
"#,
    )
    .expect("imported grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        OsStr::new("--lib"),
        temp.path().as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--require-generated-parser"),
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
    insta::assert_snapshot!("imported_sections_semantics_manifest", manifest);

    let parser = fs::read_to_string(out.join("root_parser.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("self.imported_helper();"),
        "imported finally body must execute:\n{parser}"
    );
    assert_generated_modules_compile(temp.path(), &["root_lexer.rs", "root_parser.rs"]);
}
