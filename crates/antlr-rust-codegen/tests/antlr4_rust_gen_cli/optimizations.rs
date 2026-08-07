#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

#[test]
fn adaptive_atn_routing_generated_path_compiles() {
    let temp = temporary_directory("adaptive-atn-routing");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/adaptive-atn-routing/AdaptiveRouting.g4");
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

    let parser = fs::read_to_string(out.join("adaptive_routing_parser.rs"))
        .expect("parser should be emitted");
    assert!(
        parser.contains("adaptive_atn_preferred_rules"),
        "structural candidate should emit adaptive ATN routing"
    );
    assert!(
        parser.contains("_adaptive_probe_dispatch"),
        "left-recursive seed should probe its enclosing candidate"
    );
    assert_generated_modules_compile(
        temp.path(),
        &["adaptive_routing_lexer.rs", "adaptive_routing_parser.rs"],
    );
}

#[test]
fn shared_descent_matches_baseline_for_valid_invalid_and_observed_parses() {
    let temp = temporary_directory("shared-descent");
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/antlr4-rust-gen/shared-descent");
    let out = temp.path().join("generated");

    for (grammar, shared) in [("Baseline.g4", false), ("Shared.g4", true)] {
        let grammar = fixtures.join(grammar);
        let mut args = vec![
            grammar.as_os_str(),
            OsStr::new("--actions"),
            OsStr::new("embedded"),
            OsStr::new("--require-generated-parser"),
            OsStr::new("--out-dir"),
            out.as_os_str(),
        ];
        if shared {
            args.push(OsStr::new("--shared-descent"));
        }
        let output = run_antlr4_rust_gen(&args);
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            utf8(&output.stdout),
            utf8(&output.stderr)
        );
    }

    let manifest =
        fs::read_to_string(out.join("decisions.json")).expect("manifest should be emitted");
    assert!(manifest.contains("\"status\": \"selected\""));
    assert!(manifest.contains("\"reason\": \"semantic-descent\""));
    insta::assert_snapshot!("shared_descent_decisions_manifest", manifest);

    let baseline_parser =
        fs::read_to_string(out.join("baseline_parser.rs")).expect("baseline parser should exist");
    let shared_parser =
        fs::read_to_string(out.join("shared_parser.rs")).expect("shared parser should exist");
    assert!(!baseline_parser.contains("begin_shared_descent("));
    assert!(!baseline_parser.contains("take_shared_descent_resume("));
    assert!(shared_parser.contains("begin_shared_descent("));
    assert!(shared_parser.contains("take_shared_descent_resume("));

    assert_generated_project(
        temp.path(),
        &[
            "baseline_lexer.rs",
            "baseline_parser.rs",
            "shared_lexer.rs",
            "shared_parser.rs",
        ],
        r##"
#[cfg(test)]
mod shared_descent_tests {
    use std::sync::{Arc, Mutex};

    use antlr4_runtime::{
        AntlrError, CommonTokenStream, ErrorListener, InputStream, IntStream as _, Node,
        NodeKind, ParseListener, Parser as _, Recognizer, SharedDescentStats, SyntaxErrorEvent,
    };

    #[derive(Clone, Copy, Debug)]
    enum Entry {
        Start,
        LeftStart,
        Prefixed,
        Wrapped,
        Failed,
        Semantic,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Diagnostic {
        line: usize,
        column: usize,
        message: String,
        offending: Option<String>,
    }

    #[derive(Clone, Debug)]
    struct DiagnosticListener {
        events: Arc<Mutex<Vec<Diagnostic>>>,
    }

    impl<R> ErrorListener<R> for DiagnosticListener
    where
        R: Recognizer + ?Sized,
    {
        fn syntax_error(&mut self, _recognizer: &R, event: &SyntaxErrorEvent<'_>) {
            self.events.lock().expect("diagnostic lock").push(Diagnostic {
                line: event.line,
                column: event.column,
                message: event.message.to_owned(),
                offending: event
                    .offending
                    .and_then(|token| token.text().map(str::to_owned)),
            });
        }
    }

    struct TraceListener {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl ParseListener for TraceListener {
        fn enter_every_rule(
            &mut self,
            event: &antlr4_runtime::EnterRuleEvent<'_>,
        ) -> Result<(), AntlrError> {
            self.events
                .lock()
                .expect("trace lock")
                .push(format!("enter:{}", event.rule_index));
            Ok(())
        }

        fn exit_every_rule(&mut self, rule_index: usize) {
            self.events
                .lock()
                .expect("trace lock")
                .push(format!("exit:{rule_index}"));
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Outcome {
        status: String,
        syntax_errors: usize,
        token_index: usize,
        diagnostics: Vec<Diagnostic>,
        trace: Vec<String>,
        tree: Option<String>,
    }

    #[derive(Clone, Debug)]
    struct Run {
        outcome: Outcome,
        stats: SharedDescentStats,
    }

    fn lisp(node: Node<'_>, names: &[&str], out: &mut String) {
        match node.kind() {
            NodeKind::Rule => {
                let rule = node.as_rule().expect("rule node");
                out.push('(');
                out.push_str(names.get(rule.rule_index()).copied().unwrap_or("?"));
                for child in rule.children() {
                    out.push(' ');
                    lisp(child, names, out);
                }
                out.push(')');
            }
            NodeKind::Terminal => {
                out.push_str(&node.as_terminal().expect("terminal node").text());
            }
            NodeKind::Error => {
                out.push_str("<error:");
                out.push_str(&node.as_error().expect("error node").text());
                out.push('>');
            }
        }
    }

    macro_rules! define_runner {
        ($name:ident, $lexer_mod:ident, $lexer:ident, $parser_mod:ident, $parser:ident) => {
            fn $name(input: &str, entry: Entry, observed: bool) -> Run {
                let lexer = super::$lexer_mod::$lexer::new(InputStream::new(input));
                let mut parser =
                    super::$parser_mod::$parser::new(CommonTokenStream::new(lexer));
                parser.remove_error_listeners();
                let diagnostics = Arc::new(Mutex::new(Vec::new()));
                parser.add_error_listener(DiagnosticListener {
                    events: Arc::clone(&diagnostics),
                });
                let trace = Arc::new(Mutex::new(Vec::new()));
                if observed {
                    parser.add_parse_listener(TraceListener {
                        events: Arc::clone(&trace),
                    });
                }
                let result = match entry {
                    Entry::Start => parser.start(),
                    Entry::LeftStart => parser.left_start(),
                    Entry::Prefixed => parser.prefixed(),
                    Entry::Wrapped => parser.wrapped(),
                    Entry::Failed => parser.failed(),
                    Entry::Semantic => parser.semantic(),
                };
                let syntax_errors = parser.number_of_syntax_errors();
                let token_index = parser.token_stream_mut().index();
                let tree = result.as_ref().ok().map(|root| {
                    let mut out = String::new();
                    lisp(
                        parser.node(*root),
                        super::$parser_mod::rule_names(),
                        &mut out,
                    );
                    out
                });
                Run {
                    outcome: Outcome {
                        status: result
                            .as_ref()
                            .map(|_| "ok".to_owned())
                            .unwrap_or_else(|error| format!("error:{error:?}")),
                        syntax_errors,
                        token_index,
                        diagnostics: diagnostics.lock().expect("diagnostic lock").clone(),
                        trace: trace.lock().expect("trace lock").clone(),
                        tree,
                    },
                    stats: parser.shared_descent_stats(),
                }
            }
        };
    }

    define_runner!(
        run_baseline,
        baseline_lexer,
        BaselineLexer,
        baseline_parser,
        BaselineParser
    );
    define_runner!(
        run_shared,
        shared_lexer,
        SharedLexer,
        shared_parser,
        SharedParser
    );

    #[test]
    fn differential_corpus() {
        let cases = [
            (Entry::Start, "foo"),
            (Entry::Start, "kw"),
            (Entry::Start, "foo over"),
            (Entry::Start, "foo -> bar"),
            (Entry::Start, "foo 'x'"),
            (Entry::Start, "foo(*)"),
            (Entry::Start, "foo()"),
            (Entry::Start, "distinct foo()"),
            (Entry::Start, "foo.bar()"),
            (Entry::Start, "foo + bar"),
            (Entry::Start, "foo("),
            (Entry::Start, "foo ->"),
            (Entry::Start, "foo +"),
            (Entry::LeftStart, "foo + bar"),
            (Entry::LeftStart, "foo +"),
            (Entry::Prefixed, "@ foo :"),
            (Entry::Prefixed, "@ foo ;"),
            (Entry::Prefixed, "@ foo ?"),
            (Entry::Wrapped, "foo :"),
            (Entry::Wrapped, "foo ;"),
            (Entry::Wrapped, "# foo :"),
            (Entry::Wrapped, "# ?"),
            (Entry::Failed, "foo ! :"),
            (Entry::Failed, "foo ! ;"),
            (Entry::Failed, "foo ?"),
            (Entry::Failed, "foo ? :"),
            (Entry::Semantic, "foo :"),
            (Entry::Semantic, "foo ;"),
        ];
        for (entry, input) in cases {
            let baseline = run_baseline(input, entry, false);
            let shared = run_shared(input, entry, false);
            assert_eq!(
                shared.outcome, baseline.outcome,
                "shared descent changed {entry:?} for {input:?}"
            );
        }
    }

    #[test]
    fn counters_and_observer_fallbacks_are_explicit() {
        let direct = run_shared("foo", Entry::Start, false);
        assert!(direct.stats.attempts > 0);
        assert!(direct.stats.commits > 0);

        let collision = run_shared("foo()", Entry::Start, false);
        assert!(collision.stats.adaptive_fallbacks > 0);

        let wrapped = run_shared("foo :", Entry::Wrapped, false);
        assert!(wrapped.stats.commits > 0);

        let failed = run_shared("foo ? :", Entry::Failed, false);
        assert!(failed.stats.failed_neutral_parses > 0);

        let baseline_observed = run_baseline("foo over", Entry::Start, true);
        let shared_observed = run_shared("foo over", Entry::Start, true);
        assert_eq!(shared_observed.outcome, baseline_observed.outcome);
        assert_eq!(shared_observed.stats.attempts, 0);
        assert!(shared_observed.stats.adaptive_fallbacks > 0);

        let semantic = run_shared("foo :", Entry::Semantic, false);
        assert_eq!(semantic.stats.attempts, 0);
    }
}
"##,
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn complete_ll1_recovery_matches_java_without_adaptive_fallback() {
    let temp = temporary_directory("complete-ll1-recovery");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/ll1-no-fallback/T.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--require-generated-parser"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let parser = fs::read_to_string(out.join("t_parser.rs")).expect("parser should be emitted");
    assert_eq!(
        parser
            .matches("let __prediction = match self.base.la(1) {")
            .count(),
        7,
        "all complete LL(1) decisions should emit direct lookahead dispatch"
    );
    assert!(
        !parser.contains("adaptive_predict_stream_info_sll_probe("),
        "complete LL(1) decisions should not emit SLL fallback"
    );
    assert!(
        !parser.contains("adaptive_predict_stream_info_with_context("),
        "complete LL(1) decisions should not emit full-context fallback"
    );
    let manifest =
        fs::read_to_string(out.join("decisions.json")).expect("manifest should be emitted");
    insta::assert_snapshot!("ll1_no_fallback_decisions_manifest", manifest);

    assert_generated_project(
        temp.path(),
        &["t_lexer.rs", "t_parser.rs"],
        r#"
#[cfg(test)]
mod complete_ll1_recovery_tests {
    use std::sync::{Arc, Mutex};

    use super::t_lexer::TLexer;
    use super::t_parser::{self, TParser};
    use antlr4_runtime::{
        AntlrError, CommonTokenStream, ErrorListener, InputStream, IntStream as _, Node,
        NodeKind, Parser as _, Recognizer, SyntaxErrorEvent,
    };

    type ParserType = TParser<TLexer<InputStream>>;
    type Entry = fn(&mut ParserType) -> Result<antlr4_runtime::ParseTree, AntlrError>;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Event {
        offending: Option<String>,
        line: usize,
        column: usize,
        span: Option<std::ops::Range<usize>>,
        message: String,
    }

    #[derive(Clone, Debug)]
    struct RecordingListener {
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl<R> ErrorListener<R> for RecordingListener
    where
        R: Recognizer + ?Sized,
    {
        fn syntax_error(&mut self, _recognizer: &R, event: &SyntaxErrorEvent<'_>) {
            self.events.lock().expect("events lock").push(Event {
                offending: event
                    .offending
                    .and_then(|token| token.text().map(str::to_owned)),
                line: event.line,
                column: event.column,
                span: event.span.clone(),
                message: event.message.to_owned(),
            });
        }
    }

    #[derive(Debug)]
    struct Outcome {
        case: &'static str,
        status: String,
        syntax_errors: usize,
        token_index: usize,
        events: Vec<Event>,
        tree: Option<String>,
    }

    fn lisp(node: Node<'_>, names: &[&str], out: &mut String) {
        match node.kind() {
            NodeKind::Rule => {
                let rule = node.as_rule().expect("rule node");
                out.push('(');
                out.push_str(names.get(rule.rule_index()).copied().unwrap_or("?"));
                for child in rule.children() {
                    out.push(' ');
                    lisp(child, names, out);
                }
                out.push(')');
            }
            NodeKind::Terminal => {
                out.push_str(&node.as_terminal().expect("terminal node").text());
            }
            NodeKind::Error => {
                out.push_str("<error:");
                out.push_str(&node.as_error().expect("error node").text());
                out.push('>');
            }
        }
    }

    fn parse(case: &'static str, input: &str, entry: Entry) -> Outcome {
        let lexer = TLexer::new(InputStream::new(input));
        let mut parser = TParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(usize::MAX));
        parser.remove_error_listeners();
        let events = Arc::new(Mutex::new(Vec::new()));
        parser.add_error_listener(RecordingListener {
            events: Arc::clone(&events),
        });

        let result = entry(&mut parser);
        let syntax_errors = parser.number_of_syntax_errors();
        let token_index = parser.token_stream_mut().index();
        let events = events.lock().expect("events lock").clone();
        match result {
            Ok(root) => {
                let parsed = parser.into_parsed_file(root);
                let mut tree = String::new();
                lisp(parsed.tree(), t_parser::rule_names(), &mut tree);
                Outcome {
                    case,
                    status: "ok".to_owned(),
                    syntax_errors,
                    token_index,
                    events,
                    tree: Some(tree),
                }
            }
            Err(error) => Outcome {
                case,
                status: format!("{error:?}"),
                syntax_errors,
                token_index,
                events,
                tree: None,
            },
        }
    }

    #[test]
    fn records_java_parity_recovery_cases() {
        let outcomes = [
            parse("required-valid", "aa", TParser::required),
            parse("required-invalid-first", "c", TParser::required),
            parse("required-single-deletion", "caa", TParser::required),
            parse("required-multiple-unexpected", "ccaa", TParser::required),
            parse("direct-optional-eof", "", TParser::direct_optional),
            parse("direct-star-single-deletion", "ca", TParser::direct_star),
            parse("direct-star-multiple-unexpected", "cca", TParser::direct_star),
            parse(
                "nested-optional-caller-exit",
                "ca",
                TParser::nested_optional_entry,
            ),
            parse("nested-star-eof", "", TParser::nested_star_entry),
            parse(
                "nested-star-after-iteration",
                "aca",
                TParser::nested_star_entry,
            ),
            parse(
                "nested-plus-after-iteration",
                "aca",
                TParser::nested_plus_entry,
            ),
        ];
        std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/ll1-recovery.txt"),
            format!("{outcomes:#?}\n"),
        )
        .expect("recovery snapshot should be writable");
    }
}
"#,
    );

    let recovery = fs::read_to_string(temp.path().join("compile-generated/ll1-recovery.txt"))
        .expect("recovery snapshot should be emitted");
    insta::assert_snapshot!("complete_ll1_recovery_java_parity", recovery);
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn unrecovered_generated_entry_errors_notify_listeners_once() {
    let temp = temporary_directory("fatal-error-listener");
    let grammar = temp.path().join("Fatal.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar Fatal;\nfatal: A (B B | C C);\nstart: child EOF;\nmixed: child A (B B | C C);\nsemantic_mixed: child semantic_child A (B B | C C);\nsemantic_child: {unsupported()}?;\nclean: A;\nchild: A (B B | C C);\nA: 'a';\nB: 'b';\nC: 'c';\nD: 'd';\n",
    )
    .expect("grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("templates"),
        OsStr::new("--sem-unknown"),
        OsStr::new("hook"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let test_source = r####"
#[cfg(test)]
mod fatal_error_listener_tests {
    use std::sync::{Arc, Mutex};

    use super::fatal_lexer::FatalLexer;
    use super::fatal_parser::FatalParser;
    use antlr4_runtime::{
        AntlrError, CommonTokenStream, ErrorListener, InputStream, Parser as _, Recognizer,
        SyntaxErrorEvent,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Event {
        offending_text: Option<String>,
        line: usize,
        column: usize,
        span: Option<std::ops::Range<usize>>,
        message: String,
        error: Option<AntlrError>,
    }

    #[allow(dead_code)]
    #[derive(Debug)]
    struct EntrySnapshot<'a> {
        returned_error: &'a AntlrError,
        syntax_errors: usize,
        events: &'a [Event],
    }

    #[derive(Clone, Debug)]
    struct RecordingListener {
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl<R> ErrorListener<R> for RecordingListener
    where
        R: Recognizer + ?Sized,
    {
        fn syntax_error(&mut self, _recognizer: &R, event: &SyntaxErrorEvent<'_>) {
            self.events.lock().expect("events lock").push(Event {
                offending_text: event
                    .offending
                    .and_then(|token| token.text().map(str::to_owned)),
                line: event.line,
                column: event.column,
                span: event.span.clone(),
                message: event.message.to_owned(),
                error: event.error.cloned(),
            });
        }
    }

    fn parser(
        input: &str,
    ) -> (
        FatalParser<FatalLexer<InputStream>>,
        Arc<Mutex<Vec<Event>>>,
    ) {
        let lexer = FatalLexer::new(InputStream::new(input));
        let mut parser = FatalParser::new(CommonTokenStream::new(lexer));
        // A configured (effectively unbounded) cap selects generated bodies
        // for rules the normal performance routing prefers to interpret.
        parser.set_max_rule_depth(Some(usize::MAX));
        parser.remove_error_listeners();
        let events = Arc::new(Mutex::new(Vec::new()));
        parser.add_error_listener(RecordingListener {
            events: Arc::clone(&events),
        });
        (parser, events)
    }

    #[test]
    fn fatal_public_entry_reports_the_returned_error() {
        let (mut parser, events) = parser("ad");

        let error = parser
            .fatal()
            .expect_err("invalid first token should remain fatal");

        assert_eq!(parser.number_of_syntax_errors(), 1);
        let events = events.lock().expect("events lock");
        assert_eq!(events.len(), 1, "fatal error must be reported exactly once");
        let event = &events[0];
        assert_eq!(event.offending_text.as_deref(), Some("d"));
        let AntlrError::ParserError {
            line,
            column,
            message,
            ..
        } = &error
        else {
            panic!("expected a positioned parser error, got {error:?}");
        };
        assert_eq!((event.line, event.column), (*line, *column));
        assert_eq!(&event.message, message);
        assert_eq!(event.error.as_ref(), Some(&error));
    }

    #[test]
    fn recovered_nested_error_is_not_reported_twice() {
        let (mut parser, events) = parser("ad");

        parser
            .start()
            .expect("the parent should recover the nested child error");

        assert_eq!(parser.number_of_syntax_errors(), 1);
        let events = events.lock().expect("events lock");
        assert_eq!(events.len(), 1, "recovery must report the error exactly once");
        assert_eq!(events[0].offending_text.as_deref(), Some("d"));
    }

    #[test]
    fn fatal_entry_preserves_prior_recovery_diagnostics() {
        let (mut parser, events) = parser("adad");

        let error = parser
            .mixed()
            .expect_err("the entry should fail after the child recovery");

        let events = events.lock().expect("events lock");
        let snapshot = EntrySnapshot {
            returned_error: &error,
            syntax_errors: parser.number_of_syntax_errors(),
            events: &events,
        };
        std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/fatal-entry-events.txt"),
            format!("{snapshot:#?}\n"),
        )
        .expect("fatal entry snapshot should be writable");
    }

    #[test]
    fn semantic_override_does_not_leak_prior_recovery_diagnostics() {
        let (mut parser, events) = parser("adad");

        let error = parser
            .semantic_mixed()
            .expect_err("the semantic override should win over the fatal parser error");
        assert!(
            matches!(&error, AntlrError::Unsupported(_)),
            "expected the configured fail-loud semantic error, got {error:?}"
        );

        let reported_before_reuse = events.lock().expect("events lock").len();
        parser
            .clean()
            .expect("the clean entry should succeed on the rewound input");
        let events = events.lock().expect("events lock");
        assert_eq!(
            events.len(),
            reported_before_reuse,
            "the clean entry must not emit diagnostics retained by the failed entry"
        );

        let snapshot = EntrySnapshot {
            returned_error: &error,
            syntax_errors: parser.number_of_syntax_errors(),
            events: &events,
        };
        std::fs::write(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/semantic-override-events.txt"
            ),
            format!("{snapshot:#?}\n"),
        )
        .expect("semantic override snapshot should be writable");
    }
}
"####;

    assert_generated_project(
        temp.path(),
        &["fatal_lexer.rs", "fatal_parser.rs"],
        test_source,
    );
    let fatal_entry =
        fs::read_to_string(temp.path().join("compile-generated/fatal-entry-events.txt"))
            .expect("fatal entry snapshot should be emitted");
    insta::assert_snapshot!(
        "fatal_entry_preserves_prior_recovery_diagnostics",
        fatal_entry
    );
    let semantic_override = fs::read_to_string(
        temp.path()
            .join("compile-generated/semantic-override-events.txt"),
    )
    .expect("semantic override snapshot should be emitted");
    insta::assert_snapshot!(
        "semantic_override_does_not_leak_prior_recovery_diagnostics",
        semantic_override
    );
}
