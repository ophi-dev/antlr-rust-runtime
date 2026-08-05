#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/rust-support")
        .join(name)
}

fn untrusted_fingerprint(stderr: &str) -> &str {
    let marker = "fingerprint: sha256:";
    let start = stderr
        .find(marker)
        .expect("trust diagnostic should include a fingerprint")
        + "fingerprint: ".len();
    let end = stderr[start..]
        .find(char::is_whitespace)
        .map_or(stderr.len(), |offset| start + offset);
    &stderr[start..end]
}

fn run_with_trust_store(
    grammars: &[&Path],
    output: &Path,
    trust_store: &Path,
    fingerprint: Option<&str>,
) -> Output {
    let mut command = cli_command(env!("CARGO_BIN_EXE_antlr4-rust-gen"));
    command
        .env("ANTLR4_RUST_TRUST_STORE", trust_store)
        .args(grammars)
        .arg("--out-dir")
        .arg(output);
    if let Some(fingerprint) = fingerprint {
        command.arg("--trust-rust-support").arg(fingerprint);
    }
    command.output().expect("antlr4-rust-gen should run")
}

#[test]
fn noninteractive_bundle_requires_its_exact_fingerprint() {
    let temp = temporary_directory("rust-support-trust");
    let grammar = fixture("java/JavaParser.g4");
    let generated = temp.path().join("generated");
    let trust_store = temp.path().join("trust.json");

    let output = run_with_trust_store(&[&grammar], &generated, &trust_store, None);

    assert!(
        !output.status.success(),
        "untrusted support unexpectedly ran"
    );
    assert_eq!(utf8(&output.stdout), "");
    let stderr = normalize_cli_snapshot(utf8(&output.stderr));
    let fingerprint = untrusted_fingerprint(&stderr);
    assert!(fingerprint.starts_with("sha256:"));
    assert!(stderr.contains("--trust-rust-support sha256:"));
    insta::assert_snapshot!("untrusted_rust_support_diagnostic", stderr);
}

#[test]
fn noninteractive_bundle_rejects_a_different_fingerprint() {
    let temp = temporary_directory("rust-support-mismatched-trust");
    let grammar = fixture("java/JavaParser.g4");
    let generated = temp.path().join("generated");
    let trust_store = temp.path().join("trust.json");
    let fingerprint = format!("sha256:{}", "0".repeat(64));

    let output = run_with_trust_store(&[&grammar], &generated, &trust_store, Some(&fingerprint));

    assert!(
        !output.status.success(),
        "mismatched support fingerprint unexpectedly ran"
    );
    let stderr = normalize_cli_snapshot(utf8(&output.stderr));
    assert!(stderr.contains("Rust target support is not trusted"));
    assert!(!generated.exists());
}

#[test]
fn exact_fingerprint_does_not_require_a_readable_trust_store() {
    let temp = temporary_directory("rust-support-explicit-trust");
    let grammar = fixture("java/JavaParser.g4");
    let lexer = fixture("java/JavaLexer.g4");
    let generated = temp.path().join("generated");
    let probe_store = temp.path().join("probe-trust.json");
    let untrusted = run_with_trust_store(&[&lexer, &grammar], &generated, &probe_store, None);
    assert!(!untrusted.status.success());
    let stderr = normalize_cli_snapshot(utf8(&untrusted.stderr));
    let fingerprint = untrusted_fingerprint(&stderr).to_owned();

    let corrupt_store = temp.path().join("corrupt-trust.json");
    fs::write(&corrupt_store, "not valid JSON = [")
        .expect("corrupt trust store should be writable");
    let output = run_with_trust_store(
        &[&lexer, &grammar],
        &generated,
        &corrupt_store,
        Some(&fingerprint),
    );

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
}

#[test]
fn current_java_transform_runs_from_the_staged_tree() {
    let temp = temporary_directory("rust-support-java");
    let grammar = fixture("java/JavaParser.g4");
    let lexer = fixture("java/JavaLexer.g4");
    let original = fs::read_to_string(&grammar).expect("fixture grammar should be readable");
    let generated = temp.path().join("generated");
    let trust_store = temp.path().join("trust.json");

    let untrusted = run_with_trust_store(&[&lexer, &grammar], &generated, &trust_store, None);
    assert!(!untrusted.status.success());
    let untrusted_stderr = normalize_cli_snapshot(utf8(&untrusted.stderr));
    let fingerprint = untrusted_fingerprint(&untrusted_stderr).to_owned();
    let output = run_with_trust_store(
        &[&lexer, &grammar],
        &generated,
        &trust_store,
        Some(&fingerprint),
    );

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&grammar).expect("fixture grammar should remain readable"),
        original,
        "the source checkout must not be transformed in place"
    );
    let transformed = fs::read_to_string(
        generated
            .join("antlr-rust-support")
            .read_dir()
            .expect("support artifact directory")
            .next()
            .expect("one support bundle")
            .expect("support bundle entry")
            .path()
            .join("transformed/JavaParser.g4"),
    )
    .expect("transformed Java grammar should be emitted");
    assert!(!transformed.contains("this.IsNotIdentifierAssign()"));
    assert!(transformed.contains("recog.input.la(1)"));
    let semantics =
        fs::read_to_string(generated.join("semantics.json")).expect("semantics manifest");
    let manifest: serde_json::Value =
        serde_json::from_str(&semantics).expect("semantics manifest should be valid JSON");
    assert_eq!(manifest["policy"], "error");
    assert!(
        manifest["options"]
            .as_array()
            .expect("manifest options should be an array")
            .iter()
            .any(|option| option["disposition"] == "hooked")
    );
    assert_generated_project(
        temp.path(),
        &["java_lexer.rs", "java_parser.rs"],
        r#"
#[cfg(test)]
mod rust_support_java_tests {
    use super::java_lexer::JavaLexer;
    use super::java_parser::JavaParser;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    #[test]
    fn transformed_predicate_executes() {
        let lexer = JavaLexer::new(InputStream::new("identifier"));
        let mut parser = JavaParser::new(CommonTokenStream::new(lexer));
        assert!(parser.compilation_unit().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }
}
"#,
    );
}

#[test]
fn support_strictness_does_not_leak_to_an_unrelated_root() {
    let temp = temporary_directory("rust-support-mixed-roots");
    let grammar = fixture("java/JavaParser.g4");
    let lexer = fixture("java/JavaLexer.g4");
    let unrelated = temp.path().join("Unrelated.g4");
    fs::write(
        &unrelated,
        "grammar Unrelated;\n\
         options { superClass = UnrelatedBase; }\n\
         root: ID EOF;\n\
         ID: [a-z]+;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("unrelated grammar should be writable");
    let generated = temp.path().join("generated");
    let trust_store = temp.path().join("trust.json");

    let untrusted = run_with_trust_store(
        &[&lexer, &grammar, &unrelated],
        &generated,
        &trust_store,
        None,
    );
    assert!(!untrusted.status.success());
    let untrusted_stderr = normalize_cli_snapshot(utf8(&untrusted.stderr));
    let fingerprint = untrusted_fingerprint(&untrusted_stderr).to_owned();
    let output = run_with_trust_store(
        &[&lexer, &grammar, &unrelated],
        &generated,
        &trust_store,
        Some(&fingerprint),
    );

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert!(
        utf8(&output.stderr)
            .contains("warning: unsupported grammar option: superClass=UnrelatedBase")
    );
    let semantics =
        fs::read_to_string(generated.join("semantics.json")).expect("semantics manifest");
    let manifest: serde_json::Value =
        serde_json::from_str(&semantics).expect("semantics manifest should be valid JSON");
    assert_eq!(manifest["policy"], "assume-true");
    let option = manifest["options"]
        .as_array()
        .expect("manifest options should be an array")
        .iter()
        .find(|option| option["name"] == "superClass" && option["value"] == "UnrelatedBase")
        .expect("unrelated superClass option should be reported");
    insta::assert_snapshot!(
        "unrelated_super_class_option",
        serde_json::to_string_pretty(option).expect("option should serialize")
    );
}

#[test]
fn current_c_transform_wires_its_rust_support_module() {
    let temp = temporary_directory("rust-support-c");
    let grammar = fixture("c/CParser.g4");
    let generated = temp.path().join("generated");
    let trust_store = temp.path().join("trust.json");

    let untrusted = run_with_trust_store(&[&grammar], &generated, &trust_store, None);
    assert!(!untrusted.status.success());
    let untrusted_stderr = normalize_cli_snapshot(utf8(&untrusted.stderr));
    let fingerprint = untrusted_fingerprint(&untrusted_stderr).to_owned();
    let output = run_with_trust_store(&[&grammar], &generated, &trust_store, Some(&fingerprint));

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(generated.join("c_parser_parser.rs")).expect("C parser module");
    assert!(parser.starts_with("#[doc(hidden)]\n#[path = \"antlr-rust-support/"));
    assert!(parser.contains("super::__antlr_rust_support_"));
    assert!(parser.contains("pub use self::__antlr_rust_support_"));
    assert!(parser.contains(" as rust_support;"));
    assert!(!parser.contains("crate::c_parser_base"));
    assert_generated_project(
        temp.path(),
        &["c_parser_lexer.rs", "c_parser_parser.rs"],
        r#"
#[cfg(test)]
mod rust_support_c_tests {
    use super::c_parser_lexer::CParserLexer;
    use super::c_parser_parser::CParserParser;
    use super::c_parser_parser::rust_support;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    #[test]
    fn support_actions_run_before_the_transformed_predicate() {
        rust_support::c_parser_base::reset_symbol_table();
        let lexer = CParserLexer::new(InputStream::new("name"));
        let mut parser = CParserParser::new(CommonTokenStream::new(lexer));
        assert!(parser.translation_unit().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }
}
"#,
    );
}
