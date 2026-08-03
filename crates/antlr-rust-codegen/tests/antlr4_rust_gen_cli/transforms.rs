#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

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

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn unreachable_parser_rules_warn_without_pruning() {
    let temp = temporary_directory("unreachable-rules-warning");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/unreachable-rules/Reachability.g4");
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
    let stderr = utf8(&output.stderr).replace(
        grammar
            .to_str()
            .expect("fixture path should be valid Unicode"),
        "<grammar>",
    );
    insta::assert_snapshot!("unreachable_rule_warning_diagnostics", stderr);

    let parser = fs::read_to_string(out.join("reachability_parser.rs"))
        .expect("baseline parser should be emitted");
    insta::assert_debug_snapshot!(
        "unreachable_rule_default_generated_api",
        generated_parser_api(&parser)
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn prune_unreachable_is_transitive_loud_and_preserves_explicit_entries() {
    let temp = temporary_directory("unreachable-rules-pruned");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/unreachable-rules/Reachability.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--entry-rule"),
        OsStr::new("manual"),
        OsStr::new("--visitor"),
        OsStr::new("--prune-unreachable"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let stderr = utf8(&output.stderr).replace(
        grammar
            .to_str()
            .expect("fixture path should be valid Unicode"),
        "<grammar>",
    );
    insta::assert_snapshot!("pruned_unreachable_rule_diagnostics", stderr);

    let parser = fs::read_to_string(out.join("reachability_parser.rs"))
        .expect("pruned parser should be emitted");
    insta::assert_debug_snapshot!(
        "pruned_unreachable_rule_generated_api",
        generated_parser_api(&parser)
    );

    let lexer =
        fs::read_to_string(out.join("reachability_lexer.rs")).expect("lexer should be emitted");
    assert!(lexer.contains("\"LETTER\""));
    assert_generated_modules_compile(
        temp.path(),
        &["reachability_lexer.rs", "reachability_parser.rs"],
    );
}

#[test]
fn independent_top_level_rules_without_eof_are_inferred() {
    let temp = temporary_directory("no-eof-entry-rules");
    let grammar = temp.path().join("NoEof.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar NoEof;\n\
         parseA : expr ;\n\
         parseB : stmt ;\n\
         expr : ID ;\n\
         stmt : ID SEMI ;\n\
         ID : [a-z]+ ;\n\
         SEMI : ';' ;\n\
         WS : [ \\t\\r\\n]+ -> skip ;\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--prune-unreachable"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert_eq!(utf8(&output.stderr), "");
    assert_generated_modules_compile(temp.path(), &["no_eof_lexer.rs", "no_eof_parser.rs"]);
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn eof_reaching_recursive_source_component_is_inferred_as_entries() {
    let temp = temporary_directory("recursive-entry-component");
    let grammar = temp.path().join("RecursiveEntries.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar RecursiveEntries;\n\
         junk : ID ;\n\
         a : 'a' b | EOF ;\n\
         b : 'b' a ;\n\
         ID : [a-z]+ ;\n\
         WS : [ \\t\\r\\n]+ -> skip ;\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--visitor"),
        OsStr::new("--prune-unreachable"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("recursive_entries_parser.rs"))
        .expect("parser should be emitted");
    let api = generated_parser_api(&parser);
    assert!(
        api.iter().any(|symbol| symbol == "fn a"),
        "recursive entry a should survive pruning"
    );
    assert!(
        api.iter().any(|symbol| symbol == "fn b"),
        "recursive entry b should survive pruning"
    );
    assert!(
        api.iter().any(|symbol| symbol == "fn junk"),
        "independent top-level entry junk should survive pruning"
    );
    insta::assert_debug_snapshot!("recursive_eof_entry_component_generated_api", api);
    assert_generated_modules_compile(
        temp.path(),
        &["recursive_entries_lexer.rs", "recursive_entries_parser.rs"],
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn inferred_mutual_entries_survive_left_recursion_rewrite() {
    let temp = temporary_directory("inferred-mutual-entries");
    let grammar = temp.path().join("RecursiveLeftEntries.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar RecursiveLeftEntries;\n\
         a : b 'a' | EOF ;\n\
         b : a 'b' | ID ;\n\
         ID : [a-z]+ ;\n\
         WS : [ \\t\\r\\n]+ -> skip ;\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--visitor"),
        OsStr::new("--prune-unreachable"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("recursive_left_entries_parser.rs"))
        .expect("parser should be emitted");
    let api = generated_parser_api(&parser);
    assert!(
        api.iter().any(|symbol| symbol == "fn a"),
        "inferred entry a should survive rewriting"
    );
    assert!(
        api.iter().any(|symbol| symbol == "fn b"),
        "inferred entry b should survive rewriting"
    );
    insta::assert_debug_snapshot!("inferred_mutual_entries_generated_api", api);
    assert_generated_modules_compile(
        temp.path(),
        &[
            "recursive_left_entries_lexer.rs",
            "recursive_left_entries_parser.rs",
        ],
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn configured_entry_survives_mutual_recursion_rewrite() {
    let temp = temporary_directory("configured-mutual-entry");
    let grammar = temp.path().join("ConfiguredMutualEntry.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar ConfiguredMutualEntry;\n\
         e : s | ID ;\n\
         s : e '+' ID ;\n\
         ID : [a-z]+ ;\n\
         WS : [ \\t\\r\\n]+ -> skip ;\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--entry-rule"),
        OsStr::new("s"),
        OsStr::new("--visitor"),
        OsStr::new("--prune-unreachable"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("configured_mutual_entry_parser.rs"))
        .expect("parser should be emitted");
    let api = generated_parser_api(&parser);
    assert!(
        api.iter().any(|symbol| symbol == "fn s"),
        "configured entry s should survive rewriting"
    );
    insta::assert_debug_snapshot!("configured_mutual_entry_generated_api", api);
    assert_generated_modules_compile(
        temp.path(),
        &[
            "configured_mutual_entry_lexer.rs",
            "configured_mutual_entry_parser.rs",
        ],
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn configured_entry_survives_precedence_ladder_optimization() {
    let temp = temporary_directory("configured-precedence-entry");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/precedence-ladder/Ladder.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--entry-rule"),
        OsStr::new("conditionalOr"),
        OsStr::new("--visitor"),
        OsStr::new("--optimize-precedence-ladders"),
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
        fs::read_to_string(out.join("ladder_parser.rs")).expect("parser should be emitted");
    let api = generated_parser_api(&parser);
    assert!(
        api.iter().any(|symbol| symbol == "fn conditional_or"),
        "configured entry conditionalOr should survive optimization"
    );
    insta::assert_debug_snapshot!("configured_precedence_entry_generated_api", api);
    assert_generated_modules_compile(temp.path(), &["ladder_lexer.rs", "ladder_parser.rs"]);
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn configured_entry_rule_must_name_a_parser_rule() {
    let temp = temporary_directory("unknown-entry-rule");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/unreachable-rules/Reachability.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--entry-rule"),
        OsStr::new("missing"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(!output.status.success(), "stdout: {}", utf8(&output.stdout));
    let stderr = utf8(&output.stderr).replace(
        grammar
            .to_str()
            .expect("fixture path should be valid Unicode"),
        "<grammar>",
    );
    insta::assert_snapshot!("configured_entry_rule_not_found", stderr);
    assert!(!out.exists(), "invalid entry selection emitted output");
}

#[test]
fn token_vocab_source_parser_is_not_diagnosed_or_pruned() {
    let temp = temporary_directory("token-vocab-reachability");
    let vocabulary = temp.path().join("Vocab.g4");
    let grammar = temp.path().join("User.g4");
    let out = temp.path().join("generated");
    fs::write(
        &vocabulary,
        "grammar Vocab;\n\
         vstart : A EOF ;\n\
         vcycle : A vhelper ;\n\
         vhelper : B vcycle | B ;\n\
         A : 'a' ;\n\
         B : 'b' ;\n",
    )
    .expect("vocabulary grammar should be writable");
    fs::write(
        &grammar,
        "parser grammar User;\n\
         options { tokenVocab=Vocab; }\n\
         root : A EOF ;\n",
    )
    .expect("root grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--lib"),
        temp.path().as_os_str(),
        OsStr::new("--prune-unreachable"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert_eq!(utf8(&output.stderr), "");
    let mut output_files = fs::read_dir(&out)
        .expect("output directory should exist")
        .map(|entry| {
            entry
                .expect("output entry should be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    output_files.sort();
    assert_eq!(
        output_files,
        ["decisions.json", "semantics.json", "user.rs"]
    );
}

#[test]
fn lexer_fragments_and_mode_rules_are_not_parser_reachability_candidates() {
    let temp = temporary_directory("lexer-reachability-exclusions");
    let grammar = temp.path().join("Modes.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "lexer grammar Modes;\n\
         A : 'a';\n\
         fragment UNUSED : 'u';\n\
         mode OTHER;\n\
         B : 'b';\n\
         fragment MODE_UNUSED : 'x';\n",
    )
    .expect("lexer grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--prune-unreachable"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    assert!(!utf8(&output.stderr).contains("G4S078"));
    let lexer = fs::read_to_string(out.join("modes.rs")).expect("lexer should be emitted");
    assert!(lexer.contains("\"UNUSED\""));
    assert!(lexer.contains("\"MODE_UNUSED\""));
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn precedence_ladder_optimization_is_explicit_auditable_and_recognition_preserving() {
    let temp = temporary_directory("precedence-ladder");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/precedence-ladder/Ladder.g4");
    let baseline = temp.path().join("baseline");
    let optimized = temp.path().join("optimized");
    let report = temp.path().join("report");

    for (out, extra) in [
        (&baseline, None),
        (&optimized, Some("--optimize-precedence-ladders")),
        (&report, Some("--report-precedence-ladders")),
    ] {
        let mut args = vec![
            grammar.as_os_str(),
            OsStr::new("--out-dir"),
            out.as_os_str(),
        ];
        if let Some(flag) = extra {
            args.push(OsStr::new(flag));
        }
        let output = run_antlr4_rust_gen(&args);
        assert!(
            output.status.success(),
            "{extra:?} failed\nstdout: {}\nstderr: {}",
            utf8(&output.stdout),
            utf8(&output.stderr)
        );
    }

    assert!(!baseline.join("optimizations.json").exists());
    let report_files = fs::read_dir(&report)
        .expect("report directory should exist")
        .map(|entry| {
            entry
                .expect("report entry should be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(report_files, ["optimizations.json"]);

    let manifest = fs::read_to_string(optimized.join("optimizations.json"))
        .expect("applied optimization manifest should be emitted");
    let stable_manifest = manifest.replace(env!("CARGO_MANIFEST_DIR"), "$CARGO_MANIFEST_DIR");
    insta::assert_snapshot!("precedence_ladder_optimization_manifest", stable_manifest);
    let dry_run_manifest = fs::read_to_string(report.join("optimizations.json"))
        .expect("dry-run optimization manifest should be emitted");
    assert!(dry_run_manifest.contains("\"reportOnly\": true"));
    assert!(dry_run_manifest.contains("\"status\": \"eligible\""));
    assert!(!dry_run_manifest.contains("\"status\": \"applied\""));

    let baseline_parser = fs::read_to_string(baseline.join("ladder_parser.rs"))
        .expect("baseline parser should be emitted");
    let optimized_parser = fs::read_to_string(optimized.join("ladder_parser.rs"))
        .expect("optimized parser should be emitted");
    assert!(baseline_parser.contains("pub fn conditional_or("));
    assert!(!optimized_parser.contains("pub fn conditional_or("));
    assert!(optimized_parser.contains("pub fn expr("));
    assert!(optimized_parser.contains("pub fn atom("));

    let differential = temp.path().join("differential");
    let generated = differential.join("generated");
    fs::create_dir_all(&generated).expect("differential source directory");
    for (source, target) in [
        (
            baseline.join("ladder_lexer.rs"),
            generated.join("baseline_ladder_lexer.rs"),
        ),
        (
            baseline.join("ladder_parser.rs"),
            generated.join("baseline_ladder_parser.rs"),
        ),
        (
            optimized.join("ladder_lexer.rs"),
            generated.join("optimized_ladder_lexer.rs"),
        ),
        (
            optimized.join("ladder_parser.rs"),
            generated.join("optimized_ladder_parser.rs"),
        ),
    ] {
        fs::copy(source, target).expect("generated differential module should be copied");
    }
    assert_generated_project(
        &differential,
        &[
            "baseline_ladder_lexer.rs",
            "baseline_ladder_parser.rs",
            "optimized_ladder_lexer.rs",
            "optimized_ladder_parser.rs",
        ],
        r#"
#[cfg(test)]
mod precedence_ladder_differential {
    use super::{
        baseline_ladder_lexer, baseline_ladder_parser, optimized_ladder_lexer,
        optimized_ladder_parser,
    };
    use antlr4_runtime::{FromRuleNode, IntStream as _, Parser as _};

    #[derive(Debug)]
    struct ParseOutcome {
        completed: bool,
        syntax_errors: usize,
        token_index: usize,
    }

    impl ParseOutcome {
        fn accepted(&self) -> bool {
            self.completed && self.syntax_errors == 0
        }
    }

    macro_rules! parse_result {
        ($name:ident, $lexer:ident, $parser:ident, $entry:ident) => {
            fn $name(input: &str) -> ParseOutcome {
                match $parser::parse_with_parser(
                    input,
                    $lexer::LadderLexer::new,
                    $parser::LadderParser::$entry,
                ) {
                    Ok(output) => {
                        let syntax_errors = output.parser.number_of_syntax_errors();
                        let token_index = output.parser.into_token_stream().index();
                        ParseOutcome {
                            completed: true,
                            syntax_errors,
                            token_index,
                        }
                    }
                    Err(_) => ParseOutcome {
                        completed: false,
                        syntax_errors: usize::MAX,
                        token_index: 0,
                    },
                }
            }
        };
    }

    fn assert_same_recognition(
        input: &str,
        baseline: ParseOutcome,
        optimized: ParseOutcome,
    ) {
        assert_eq!(
            optimized.accepted(),
            baseline.accepted(),
            "recognition diverged for {input:?}: baseline={baseline:?}, optimized={optimized:?}"
        );
        if baseline.accepted() {
            assert_eq!(
                optimized.token_index, baseline.token_index,
                "valid-input consumption diverged for {input:?}: \
                 baseline={baseline:?}, optimized={optimized:?}"
            );
        }
    }

    parse_result!(
        baseline,
        baseline_ladder_lexer,
        baseline_ladder_parser,
        start
    );
    parse_result!(
        optimized,
        optimized_ladder_lexer,
        optimized_ladder_parser,
        start
    );
    parse_result!(
        baseline_star,
        baseline_ladder_lexer,
        baseline_ladder_parser,
        star_start
    );
    parse_result!(
        optimized_star,
        optimized_ladder_lexer,
        optimized_ladder_parser,
        star_start
    );
    parse_result!(
        baseline_direct,
        baseline_ladder_lexer,
        baseline_ladder_parser,
        direct_start
    );
    parse_result!(
        optimized_direct,
        optimized_ladder_lexer,
        optimized_ladder_parser,
        direct_start
    );
    parse_result!(
        baseline_mixed,
        baseline_ladder_lexer,
        baseline_ladder_parser,
        mixed_start
    );
    parse_result!(
        optimized_mixed,
        optimized_ladder_lexer,
        optimized_ladder_parser,
        mixed_start
    );

    #[test]
    fn valid_and_invalid_inputs_keep_the_authored_language() {
        for input in [
            "1",
            "1 + 2 * 3",
            "-1 + 2",
            "!!1 || 2 && 3",
            "1 < 2 == 3",
            "1 ? 2 : 3 ? 4 : 5",
            "(1 + 2) * 3",
        ] {
            let baseline = baseline(input);
            assert!(baseline.accepted(), "baseline should accept {input:?}: {baseline:?}");
            assert_same_recognition(input, baseline, optimized(input));
        }
        for input in [
            "+",
            "1 ? 2 ? 3 : 4 : 5",
            "1 +",
            "? 1 : 2",
            "1 ? 2",
            "1 && || 2",
            "((1)",
        ] {
            let baseline = baseline(input);
            assert!(
                !baseline.accepted(),
                "baseline should reject {input:?}: {baseline:?}"
            );
            assert_same_recognition(input, baseline, optimized(input));
        }
    }

    #[test]
    fn precedence_and_right_associativity_survive_the_rewrite() {
        let parsed = optimized_ladder_parser::parse(
            "1 + 2 * 3",
            optimized_ladder_lexer::LadderLexer::new,
            optimized_ladder_parser::LadderParser::start,
        )
        .expect("valid arithmetic expression");
        let start =
            optimized_ladder_parser::StartContext::from_rule_node(
                parsed.tree().as_rule().expect("rule root")
            )
                .expect("start context");
        let expression = start.expr().expect("entry expression");
        assert!(
            optimized_ladder_parser::ExprCalcOperator2LabelContext::from_rule_node(
                expression.rule_node()
            )
            .is_some(),
            "addition should remain the root operator"
        );
        let children = expression.expr_children().collect::<Vec<_>>();
        assert!(
            optimized_ladder_parser::ExprCalcOperator1LabelContext::from_rule_node(
                children[1].rule_node()
            )
            .is_some(),
            "multiplication should bind inside the right operand"
        );

        let parsed = optimized_ladder_parser::parse(
            "1 ? 2 : 3 ? 4 : 5",
            optimized_ladder_lexer::LadderLexer::new,
            optimized_ladder_parser::LadderParser::start,
        )
        .expect("valid conditional expression");
        let start =
            optimized_ladder_parser::StartContext::from_rule_node(
                parsed.tree().as_rule().expect("rule root")
            )
                .expect("start context");
        let expression = start.expr().expect("entry expression");
        assert!(
            optimized_ladder_parser::ExprExprRight6LabelContext::from_rule_node(
                expression.rule_node()
            )
            .is_some(),
            "outer conditional context"
        );
        let children = expression.expr_children().collect::<Vec<_>>();
        assert!(
            optimized_ladder_parser::ExprExprRight6LabelContext::from_rule_node(
                children[2].rule_node()
            )
            .is_some(),
            "conditional tail should remain right associative"
        );
    }

    #[test]
    fn lowest_star_rung_remains_repeatable_at_the_ladder_boundary() {
        for input in [
            "1",
            "1 * 2 * 3",
            "1 + 2 * 3 + 4 * 5 * 6",
            "1 *",
            "* 1",
        ] {
            assert_same_recognition(input, baseline_star(input), optimized_star(input));
        }
    }

    #[test]
    fn direct_base_rhs_and_binary_right_tail_match_the_original() {
        for input in [
            "1",
            "1 ^ 2 ^ 3",
            "1 ~ 2 ~ 3",
            "1 ~ 2 ^ 3 ~ 4",
            "1 ^",
            "~ 1",
        ] {
            assert_same_recognition(input, baseline_direct(input), optimized_direct(input));
        }
    }

    #[test]
    fn looser_prefix_stays_out_of_a_tighter_right_tail_operand() {
        for input in ["1", "!1", "!!1 ^ 2", "1 ^ 2 ^ 3"] {
            assert_same_recognition(input, baseline_mixed(input), optimized_mixed(input));
        }

        let invalid = baseline_mixed("1 ^ !1");
        assert!(
            !invalid.accepted(),
            "baseline should reject the loose prefix: {invalid:?}"
        );
        assert_same_recognition("1 ^ !1", invalid, optimized_mixed("1 ^ !1"));
    }
}
"#,
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn precedence_ladder_modes_reject_authored_semantic_errors_before_transforming() {
    let temp = temporary_directory("precedence-ladder-authored-errors");
    let cases = [
        (
            "DuplicateRule",
            r#"grammar DuplicateRule;
start : high EOF ;
high : low ('+' low)* ;
low : atom ('*' atom)* ;
low : atom ;
atom : INT ;
INT : [0-9]+ ;
WS : [ \t\r\n]+ -> skip ;
"#,
            "G4S002",
            "rule low is redefined",
        ),
        (
            "DuplicateLabel",
            r#"grammar DuplicateLabel;
start : high EOF ;
high : low ('+' low)* # Shared ;
low : atom ('*' atom)* # Shared ;
atom : INT ;
INT : [0-9]+ ;
WS : [ \t\r\n]+ -> skip ;
"#,
            "G4S013",
            "alternative label Shared is redefined",
        ),
    ];
    let mut diagnostics = Vec::new();

    for (grammar_name, source, code, message) in cases {
        let grammar_dir = temp.path().join(grammar_name);
        fs::create_dir_all(&grammar_dir).expect("grammar directory should be writable");
        let grammar = grammar_dir.join(format!("{grammar_name}.g4"));
        fs::write(&grammar, source).expect("invalid grammar should be writable");
        for flag in [
            "--optimize-precedence-ladders",
            "--report-precedence-ladders",
        ] {
            let out = grammar_dir.join(flag.trim_start_matches("--"));
            let output = run_antlr4_rust_gen(&[
                grammar.as_os_str(),
                OsStr::new(flag),
                OsStr::new("--out-dir"),
                out.as_os_str(),
            ]);
            assert!(
                !output.status.success(),
                "{grammar_name} unexpectedly succeeded with {flag}"
            );
            let stderr = utf8(&output.stderr);
            assert!(stderr.contains(code), "{stderr}");
            assert!(stderr.contains(message), "{stderr}");
            diagnostics.push((
                grammar_name,
                flag,
                stderr.replace(
                    temp.path()
                        .to_str()
                        .expect("temporary path should be UTF-8"),
                    "$TMP",
                ),
            ));
        }
    }

    insta::assert_debug_snapshot!("precedence_ladder_authored_semantic_errors", diagnostics);
}

#[test]
fn unoptimized_regeneration_removes_stale_precedence_manifest() {
    let temp = temporary_directory("precedence-ladder-stale-manifest");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/precedence-ladder/Ladder.g4");
    let out = temp.path().join("generated");

    let optimized = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--optimize-precedence-ladders"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        optimized.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&optimized.stdout),
        utf8(&optimized.stderr)
    );
    assert!(out.join("optimizations.json").is_file());

    let baseline = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        baseline.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&baseline.stdout),
        utf8(&baseline.stderr)
    );
    assert!(!out.join("optimizations.json").exists());
    let parser = fs::read_to_string(out.join("ladder_parser.rs"))
        .expect("unoptimized parser should be emitted");
    assert!(parser.contains("pub fn conditional_or("));
}

#[test]
fn embedded_actions_keep_potentially_referenced_ladder_rules() {
    let temp = temporary_directory("precedence-ladder-embedded-actions");
    let grammar = temp.path().join("ActionLadder.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar ActionLadder;\n\
         start: high { let _ = self.low(); } EOF;\n\
         high: low ('+' low)*;\n\
         low: atom ('*' atom)*;\n\
         atom: INT;\n\
         INT: [0-9]+;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("embedded-action grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--optimize-precedence-ladders"),
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
        fs::read_to_string(out.join("action_ladder_parser.rs")).expect("parser should be emitted");
    assert!(parser.contains("pub fn low("));
    let manifest = fs::read_to_string(out.join("optimizations.json"))
        .expect("optimization manifest should be emitted");
    assert!(manifest.contains("\"changed\": false"), "{manifest}");
    assert_generated_modules_compile(
        temp.path(),
        &["action_ladder_lexer.rs", "action_ladder_parser.rs"],
    );
}

#[test]
fn embedded_rule_attributes_keep_potentially_referenced_ladder_contexts() {
    let temp = temporary_directory("precedence-ladder-embedded-rule-attributes");
    let grammar = temp.path().join("AttributeLadder.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar AttributeLadder;\n\
         start returns [std::marker::PhantomData<fn() -> LowContext<'static>> saved]\n\
             : high EOF;\n\
         high: low ('+' low)*;\n\
         low: atom ('*' atom)*;\n\
         atom: INT;\n\
         INT: [0-9]+;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("embedded-rule-attribute grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--optimize-precedence-ladders"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("attribute_ladder_parser.rs"))
        .expect("parser should be emitted");
    assert!(parser.contains("pub fn low("));
    let manifest = fs::read_to_string(out.join("optimizations.json"))
        .expect("optimization manifest should be emitted");
    assert!(manifest.contains("\"changed\": false"), "{manifest}");
    assert_generated_modules_compile(
        temp.path(),
        &["attribute_ladder_lexer.rs", "attribute_ladder_parser.rs"],
    );
}
