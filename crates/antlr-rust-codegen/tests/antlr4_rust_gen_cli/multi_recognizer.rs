#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.

use std::collections::BTreeMap;

use antlr_rust_codegen::{Builder, ErrorKind};

#[allow(clippy::wildcard_imports)]
use super::support::*;

fn flat_directory_contents(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(directory)
        .expect("artifact directory should be readable")
        .map(|entry| {
            let entry = entry.expect("artifact entry should be readable");
            let name = entry
                .file_name()
                .into_string()
                .expect("artifact name should be UTF-8");
            let contents = fs::read(entry.path()).expect("artifact should be readable");
            (name, contents)
        })
        .collect()
}

#[test]
fn builder_and_cli_share_artifacts_warnings_and_structured_diagnostics() {
    let temp = temporary_directory("builder-cli-parity");
    let grammar = temp.path().join("Parity.g4");
    let library_out = temp.path().join("generated");
    let cli_out = temp.path().join("cli");
    fs::write(
        &grammar,
        "grammar Parity;\nstart: ID EOF;\nID: [a-z]+;\nWS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("parity grammar should be writable");

    let generation = Builder::new()
        .grammar(&grammar)
        .out_dir(&library_out)
        .generate()
        .expect("library generation should succeed");
    let cli = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        cli_out.as_os_str(),
    ]);
    assert!(
        cli.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&cli.stdout),
        utf8(&cli.stderr)
    );
    assert_eq!(
        flat_directory_contents(&library_out),
        flat_directory_contents(&cli_out)
    );
    assert_eq!(
        generation.inputs(),
        [grammar.canonicalize().expect("grammar")]
    );
    assert!(generation.warnings().is_empty());
    assert_generated_project(
        temp.path(),
        &["parity_lexer.rs", "parity_parser.rs"],
        r#"
#[cfg(test)]
mod generated_source_without_codegen_dependency {
    use super::parity_lexer::ParityLexer;
    use super::parity_parser::ParityParser;
    use antlr4_runtime::{CommonTokenStream, InputStream};

    #[test]
    fn parses_without_linking_the_generator() {
        let lexer = ParityLexer::new(InputStream::new("value"));
        let mut parser = ParityParser::new(CommonTokenStream::new(lexer));
        parser.start().expect("fixture input should parse");
    }
}
"#,
    );
    let consumer_manifest =
        fs::read_to_string(temp.path().join("compile-generated").join("Cargo.toml"))
            .expect("consumer manifest should be readable");
    assert!(!consumer_manifest.contains("antlr-rust-codegen"));

    let options = temp.path().join("Options.g4");
    let options_out = temp.path().join("options-library");
    fs::write(
        &options,
        "lexer grammar Options;\noptions { superClass=CustomLexer; }\nA: 'a';\n",
    )
    .expect("options grammar should be writable");
    let generation = Builder::new()
        .grammar(&options)
        .out_dir(&options_out)
        .generate()
        .expect("unsupported options should remain warnings");
    let cli_options_out = temp.path().join("options-cli");
    let cli = run_antlr4_rust_gen(&[
        options.as_os_str(),
        OsStr::new("--out-dir"),
        cli_options_out.as_os_str(),
    ]);
    assert!(cli.status.success(), "stderr: {}", utf8(&cli.stderr));
    assert_eq!(
        utf8(&cli.stderr).trim_end(),
        generation.warnings().join("\n")
    );

    let malformed = temp.path().join("Malformed.g4");
    fs::write(&malformed, "lexer grammar Malformed;\nA: 'a'\n")
        .expect("malformed grammar should be writable");
    let error = Builder::new()
        .grammar(&malformed)
        .out_dir(temp.path().join("malformed-library"))
        .generate()
        .expect_err("malformed grammar should fail");
    assert_eq!(error.kind(), ErrorKind::Compilation);
    assert!(!error.diagnostics().is_empty());
    let cli = run_antlr4_rust_gen(&[
        malformed.as_os_str(),
        OsStr::new("--out-dir"),
        temp.path().join("malformed-cli").as_os_str(),
    ]);
    assert!(!cli.status.success());
    let stderr = utf8(&cli.stderr);
    for diagnostic in error.diagnostics() {
        assert!(stderr.contains(diagnostic.code()), "{stderr}");
        assert!(stderr.contains(diagnostic.message()), "{stderr}");
    }
}

#[test]
fn builder_tracks_token_vocabulary_and_semantic_pattern_inputs() {
    let temp = temporary_directory("builder-external-inputs");
    let grammar = temp.path().join("ExternalParser.g4");
    let tokens = temp.path().join("ExternalLexer.tokens");
    let patterns = temp.path().join("semantics.toml");
    fs::write(
        &grammar,
        "parser grammar ExternalParser;\n\
         options { tokenVocab=ExternalLexer; }\n\
         start: ID EOF;\n",
    )
    .expect("parser grammar should be writable");
    fs::write(&tokens, "ID=1\n").expect("token vocabulary should be writable");
    fs::write(&patterns, "").expect("semantic patterns should be writable");

    let generation = Builder::new()
        .grammar(&grammar)
        .semantic_patterns(&patterns)
        .out_dir(temp.path().join("generated"))
        .generate()
        .expect("generation with external inputs should succeed");
    let canonical_temp = temp.path().canonicalize().expect("temporary directory");
    let inputs = generation
        .inputs()
        .iter()
        .map(|path| {
            path.strip_prefix(&canonical_temp)
                .expect("input should be below the temporary directory")
                .display()
                .to_string()
        })
        .collect::<Vec<_>>();

    insta::assert_debug_snapshot!("builder_external_inputs", inputs);
}

fn instantiate_parser_fixture(workspace: &Path, package: &str, grammar: &str, module: &str) {
    let template =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/build-dependency/parser-crate-template");
    let package_directory = workspace.join(package);
    for (relative, target) in [
        ("Cargo.toml.in", package_directory.join("Cargo.toml")),
        ("build.rs.in", package_directory.join("build.rs")),
        (
            "grammar/Lexer.g4.in",
            package_directory.join(format!("grammar/{grammar}Lexer.g4")),
        ),
        (
            "grammar/Parser.g4.in",
            package_directory.join(format!("grammar/{grammar}Parser.g4")),
        ),
        (
            "grammar/Shared.g4.in",
            package_directory.join(format!("grammar/{grammar}Shared.g4")),
        ),
        ("src/lib.rs.in", package_directory.join("src/lib.rs")),
    ] {
        let source = template.join(relative);
        let contents = fs::read_to_string(source)
            .expect("build-dependency fixture template should be readable")
            .replace("@PACKAGE@", package)
            .replace("@GRAMMAR@", grammar)
            .replace("@MODULE@", module);
        fs::create_dir_all(
            target
                .parent()
                .expect("fixture target should have a parent"),
        )
        .expect("fixture directory should be writable");
        fs::write(target, contents).expect("fixture should be writable");
    }
}

fn run_fixture_workspace(workspace: &Path) -> Output {
    Command::new(env!("CARGO"))
        .args(["test", "--workspace", "--offline"])
        .current_dir(workspace)
        .env("CARGO_TARGET_DIR", workspace.join("target"))
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .expect("fixture workspace should run")
}

fn assert_fixture_success(output: &Output) {
    assert!(
        output.status.success(),
        "fixture workspace failed\nstdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
}

fn build_run_count(workspace: &Path, package: &str) -> usize {
    fs::read_to_string(workspace.join(package).join("build-runs.txt"))
        .expect("build run marker should exist")
        .parse()
        .expect("build run marker should be numeric")
}

fn find_named_files(directory: &Path, name: &str, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("fixture directory should be readable") {
        let entry = entry.expect("fixture entry should be readable");
        let path = entry.path();
        if path.is_dir() {
            find_named_files(&path, name, found);
        } else if entry.file_name() == name {
            found.push(path);
        }
    }
}

#[test]
fn three_build_dependency_consumers_share_codegen_and_track_only_resolved_inputs() {
    let temp = temporary_directory("build-dependency-workspace");
    let workspace = temp.path();
    let fixtures = [
        ("parser-one", "FixtureOne", "fixture_one"),
        ("parser-two", "FixtureTwo", "fixture_two"),
        ("parser-three", "FixtureThree", "fixture_three"),
    ];
    for (package, grammar, module) in fixtures {
        instantiate_parser_fixture(workspace, package, grammar, module);
    }
    let root = workspace_root();
    fs::write(
        workspace.join("Cargo.toml"),
        format!(
            "[workspace]\n\
             members = [\"parser-one\", \"parser-two\", \"parser-three\"]\n\
             resolver = \"3\"\n\
             \n\
             [workspace.dependencies]\n\
             antlr-rust-runtime = {{ path = {:?} }}\n\
             antlr-rust-codegen = {{ path = {:?} }}\n",
            root,
            root.join("crates/antlr-rust-codegen")
        ),
    )
    .expect("fixture workspace manifest should be writable");

    let first = run_fixture_workspace(workspace);
    assert_fixture_success(&first);
    assert_eq!(
        utf8(&first.stderr)
            .matches("Compiling antlr-rust-codegen v")
            .count(),
        1,
        "all consumers should share one host codegen build:\n{}",
        utf8(&first.stderr)
    );
    for (package, _, _) in fixtures {
        assert_eq!(build_run_count(workspace, package), 1);
    }

    let second = run_fixture_workspace(workspace);
    assert_fixture_success(&second);
    for (package, _, _) in fixtures {
        assert_eq!(build_run_count(workspace, package), 1);
    }

    let unrelated = workspace.join("parser-one/src/lib.rs");
    let mut source = fs::read_to_string(&unrelated).expect("fixture source should be readable");
    source.push_str("\n// Unrelated target-source change.\n");
    fs::write(unrelated, source).expect("fixture source should be writable");
    assert_fixture_success(&run_fixture_workspace(workspace));
    for (package, _, _) in fixtures {
        assert_eq!(build_run_count(workspace, package), 1);
    }

    let imported = workspace.join("parser-one/grammar/FixtureOneShared.g4");
    let mut source = fs::read_to_string(&imported).expect("imported grammar should be readable");
    source.push_str("\n// Imported grammar change.\n");
    fs::write(imported, source).expect("imported grammar should be writable");
    assert_fixture_success(&run_fixture_workspace(workspace));
    assert_eq!(build_run_count(workspace, "parser-one"), 2);
    assert_eq!(build_run_count(workspace, "parser-two"), 1);
    assert_eq!(build_run_count(workspace, "parser-three"), 1);

    let root_grammar = workspace.join("parser-one/grammar/FixtureOneParser.g4");
    let mut source = fs::read_to_string(&root_grammar).expect("root grammar should be readable");
    source.push_str("\n// Root grammar change.\n");
    fs::write(root_grammar, source).expect("root grammar should be writable");
    assert_fixture_success(&run_fixture_workspace(workspace));
    assert_eq!(build_run_count(workspace, "parser-one"), 3);

    let mut host_target_files = Vec::new();
    find_named_files(
        &workspace.join("target"),
        "host-target.txt",
        &mut host_target_files,
    );
    assert_eq!(host_target_files.len(), 3);
    for path in host_target_files {
        let contents = fs::read_to_string(path).expect("host-target marker should be readable");
        let mut lines = contents.lines();
        assert_eq!(lines.next(), lines.next());
    }
}
