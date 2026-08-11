#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

#[test]
fn embedded_parser_semantics_satisfy_strict_manifest_checks() {
    let temp = temporary_directory("embedded-parser-semantics");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/embedded-parser-semantics/T.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
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
    assert_eq!(
        manifest.matches("\"disposition\": \"translated\"").count(),
        2
    );
    assert_eq!(manifest.matches("\"template\": \"Embedded\"").count(), 2);
    let parser = fs::read_to_string(out.join("t_parser.rs")).expect("parser should be emitted");
    assert!(
        !parser.contains("__Antlr4RustInput"),
        "native embedded bodies should not emit the antlr4rust facade"
    );
    assert_generated_modules_compile(temp.path(), &["t_lexer.rs", "t_parser.rs"]);
}
/// Issue #241: antlr4rust grammars use `recog` as the parser-predicate receiver.
/// Supporting that convention keeps migrations source-compatible by routing it
/// through the same typed helper hook as bare, `this.`, and `self.` calls.
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn recog_receiver_parser_predicate_routes_to_typed_hook() {
    let temp = temporary_directory("recog-parser-hook");
    let grammar = temp.path().join("RecogPredicate.g4");
    let patterns = temp.path().join("patterns.toml");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        r#"grammar RecogPredicate;

options {
    superClass = ParserBase;
}

shl
    : {recog.IsOk()}? LT LT EOF
    ;

LT: '<';
WS: [ \t\r\n]+ -> skip;
"#,
    )
    .expect("grammar should be writable");
    fs::write(
        &patterns,
        r#"version = 1

[[helper]]
kind = "parser-predicate"
name = "IsOk"
receiver = "recog"
returns = "bool"
lower = "hook"
"#,
    )
    .expect("semantic patterns should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--sem-patterns"),
        patterns.as_os_str(),
        OsStr::new("--option-hook"),
        OsStr::new("superClass=ParserBase"),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
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
    insta::assert_snapshot!("recog_receiver_semantics_manifest", manifest);
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest).expect("manifest should contain valid JSON");
    let predicate = manifest["grammars"]
        .as_array()
        .expect("manifest grammars should be an array")
        .iter()
        .flat_map(|grammar| {
            grammar["coordinates"]
                .as_array()
                .expect("grammar coordinates should be an array")
        })
        .find(|coordinate| coordinate["kind"] == "parser-predicate" && coordinate["rule"] == "shl")
        .expect("parser predicate should be inventoried");
    assert_eq!(predicate["body"], "recog.IsOk()");
    assert_eq!(predicate["template"], "Hook");

    let parser = fs::read_to_string(out.join("recog_predicate_parser.rs"))
        .expect("parser should be emitted");
    let (_, adapter) = parser
        .split_once("pub trait RecogPredicateParserHooks")
        .expect("typed hook adapter should be emitted");
    let (adapter, _) = adapter
        .split_once("\n\n/// Marker carried by generated contexts whose required-child")
        .expect("the validated-tree support should follow the typed hook adapter");
    insta::assert_snapshot!(
        "recog_receiver_typed_hook_adapter",
        format!("pub trait RecogPredicateParserHooks{adapter}")
    );

    let test_source = r####"
#[cfg(test)]
mod recog_receiver_tests {
    use super::recog_predicate_lexer::RecogPredicateLexer;
    use super::recog_predicate_parser::{
        RecogPredicateParser, RecogPredicateParserHooks,
    };
    use antlr4_runtime::{
        CommonTokenStream, InputStream, Parser as _, ParserSemCtx, TokenSource,
    };
    use std::cell::Cell;
    use std::rc::Rc;

    struct Hooks {
        calls: Rc<Cell<usize>>,
    }

    impl RecogPredicateParserHooks for Hooks {
        fn is_ok<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>) -> bool
        where
            L: TokenSource,
        {
            self.calls.set(self.calls.get() + 1);
            true
        }
    }

    #[test]
    fn typed_hook_accepts_the_predicated_alternative() {
        let calls = Rc::new(Cell::new(0));
        let lexer = RecogPredicateLexer::new(InputStream::new("<<"));
        let mut parser = RecogPredicateParser::with_typed_hooks(
            CommonTokenStream::new(lexer),
            Hooks {
                calls: Rc::clone(&calls),
            },
        );
        let _ = parser.shl().expect("predicated rule should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        assert!(calls.get() > 0, "typed predicate hook was not invoked");
    }
}
"####;

    assert_generated_project(
        temp.path(),
        &["recog_predicate_lexer.rs", "recog_predicate_parser.rs"],
        test_source,
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn named_parser_actions_run_at_committed_positions_on_both_parser_paths() {
    let temp = temporary_directory("named-parser-actions");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/parser-action-hooks");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        fixture.join("ActionTiming.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        fixture.join("patterns.toml").as_os_str(),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
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
    insta::assert_snapshot!("named_parser_actions_semantics_manifest", manifest);

    let parser =
        fs::read_to_string(out.join("action_timing_parser.rs")).expect("parser should be emitted");
    for expected in [
        "pub trait ActionTimingParserHooks",
        "fn enter<L>",
        "fn enter_scope<L>",
        "fn exit_scope<L>",
        "fn tick<L>",
        "fn seed<L>",
        "fn reduce<L>",
        "fn middle<L>",
        "match (action.rule_index(), action.action_index())",
        "parser_action_at_current_indexed",
        "parser_action_hook_with_context",
        "parser_action_hook_with_context_and_local",
        "action_indices: &[(",
    ] {
        assert!(parser.contains(expected), "missing {expected:?}\n{parser}");
    }

    let test_source = r####"
#[cfg(test)]
mod named_action_tests {
    use super::action_timing_lexer::ActionTimingLexer;
    use super::action_timing_parser::{
        ActionTimingParser, ActionTimingParserHooks, ActionTimingParserTypedHooks,
    };
    use antlr4_runtime::{
        AntlrError, CommonTokenStream, InputStream, Parser as _, ParserSemCtx, TokenSource,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Hooks {
        entered: usize,
        events: Rc<RefCell<Vec<String>>>,
    }

    impl ActionTimingParserHooks for Hooks {
        fn enter<L>(
            &mut self,
            _ctx: &mut ParserSemCtx<'_, L>,
            name: &str,
            level: i64,
            enabled: bool,
        ) where
            L: TokenSource,
        {
            self.entered += 1;
            self.events
                .borrow_mut()
                .push(format!("enter:{name}:{level}:{enabled}"));
        }

        fn is_entered<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>) -> bool
        where
            L: TokenSource,
        {
            let result = self.entered > 0;
            self.events.borrow_mut().push(format!("predicate:{result}"));
            result
        }

        fn enter_scope<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>)
        where
            L: TokenSource,
        {
            self.events.borrow_mut().push("scope+".to_owned());
        }

        fn exit_scope<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>)
        where
            L: TokenSource,
        {
            self.events.borrow_mut().push("scope-".to_owned());
        }

        fn tick<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>, value: i64)
        where
            L: TokenSource,
        {
            self.events.borrow_mut().push(format!("tick:{value}"));
        }

        fn lose<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>)
        where
            L: TokenSource,
        {
            self.events.borrow_mut().push("lose".to_owned());
        }

        fn seed<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>)
        where
            L: TokenSource,
        {
            self.events.borrow_mut().push("seed".to_owned());
        }

        fn reduce<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>)
        where
            L: TokenSource,
        {
            self.events.borrow_mut().push("reduce".to_owned());
        }

        fn exit<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>, name: &str)
        where
            L: TokenSource,
        {
            self.entered -= 1;
            self.events.borrow_mut().push(format!("exit:{name}"));
        }

        fn middle<L>(&mut self, _ctx: &mut ParserSemCtx<'_, L>, name: &str)
        where
            L: TokenSource,
        {
            self.events.borrow_mut().push(name.to_owned());
        }

        fn observe_argument<L>(&mut self, ctx: &mut ParserSemCtx<'_, L>)
        where
            L: TokenSource,
        {
            let value = ctx
                .local_int_arg()
                .expect("the parameterized rule argument should be visible");
            self.events.borrow_mut().push(format!("argument:{value}"));
        }
    }

    type TestParser = ActionTimingParser<
        ActionTimingLexer<InputStream>,
        ActionTimingParserTypedHooks<Hooks>,
    >;
    type Entry = fn(&mut TestParser) -> Result<antlr4_runtime::ParseTree, AntlrError>;

    #[derive(Debug, Eq, PartialEq)]
    struct Outcome {
        events: Vec<String>,
        syntax_errors: usize,
        text: String,
    }

    fn run(input: &str, entry: Entry) -> Outcome {
        let events = Rc::new(RefCell::new(Vec::new()));
        let lexer = ActionTimingLexer::new(InputStream::new(input));
        let mut parser = ActionTimingParser::with_typed_hooks(
            CommonTokenStream::new(lexer),
            Hooks {
                entered: 0,
                events: Rc::clone(&events),
            },
        );
        parser.remove_error_listeners();
        let root = entry(&mut parser).expect("fixture input should parse");
        let outcome = Outcome {
            events: events.borrow().clone(),
            syntax_errors: parser.number_of_syntax_errors(),
            text: parser.node(root).text(),
        };
        outcome
    }

    #[test]
    fn generated_and_interpreted_order_match() {
        let input = "nest item more b x+y+z";
        let generated = run(input, TestParser::generated);
        let interpreted = run(input, TestParser::interpreted);

        assert_eq!(generated, interpreted);
        assert_eq!(
            generated.events,
            [
                "enter:outer:1:true",
                "predicate:true",
                "scope+",
                "scope+",
                "scope-",
                "scope-",
                "tick:7",
                "tick:7",
                "seed",
                "reduce",
                "reduce",
                "exit:outer",
            ]
        );
        assert_eq!(generated.syntax_errors, 0);
        assert_eq!(generated.text, "nestitemmorebx+y+z<EOF>");
        assert!(
            !generated.events.iter().any(|event| event == "lose"),
            "the action in the losing alternative must not run"
        );
    }

    #[test]
    fn parameterized_rule_arguments_reach_both_action_paths() {
        let generated = run("", TestParser::generated_argument);
        let interpreted = run("", TestParser::interpreted_argument);

        assert_eq!(generated.events, ["argument:17"]);
        assert_eq!(interpreted.events, ["argument:23"]);
        assert_eq!(generated.text, "<EOF>");
        assert_eq!(interpreted.text, "<EOF>");
    }

    #[test]
    fn recovery_before_and_after_an_action_matches() {
        for input in ["c a b c", "a b a c"] {
            let generated = run(input, TestParser::recover_generated);
            let interpreted = run(input, TestParser::recover_interpreted);

            assert_eq!(generated, interpreted, "input {input:?}");
            assert_eq!(generated.events, ["middle"], "input {input:?}");
            assert_eq!(generated.syntax_errors, 1, "input {input:?}");
        }
    }
}
"####;

    assert_generated_project(
        temp.path(),
        &["action_timing_lexer.rs", "action_timing_parser.rs"],
        test_source,
    );
}

#[test]
fn forwarded_parser_rule_arguments_reach_named_actions() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/parser-action-forwarded-args");
    let temp = temporary_directory("parser-action-forwarded-args");
    let out = temp.path().join("generated");
    let output = run_antlr4_rust_gen(&[
        fixture.join("Forwarded.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        fixture.join("patterns.toml").as_os_str(),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
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

    let parser =
        fs::read_to_string(out.join("forwarded_parser.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("parser_action_hook_with_context_and_local"),
        "{parser}"
    );
    assert!(parser.contains("inherit_local: true"), "{parser}");

    let test_source = r####"
#[cfg(test)]
mod forwarded_argument_tests {
    use super::forwarded_lexer::ForwardedLexer;
    use super::forwarded_parser::{
        ForwardedParser, ForwardedParserHooks, ForwardedParserTypedHooks,
    };
    use antlr4_runtime::{
        AntlrError, CommonTokenStream, InputStream, ParserSemCtx, TokenSource,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Hooks {
        values: Rc<RefCell<Vec<i64>>>,
    }

    impl ForwardedParserHooks for Hooks {
        fn observe_argument<L>(&mut self, ctx: &mut ParserSemCtx<'_, L>)
        where
            L: TokenSource,
        {
            self.values.borrow_mut().push(
                ctx.local_int_arg()
                    .expect("forwarded parser argument should be visible"),
            );
        }
    }

    type TestParser =
        ForwardedParser<ForwardedLexer<InputStream>, ForwardedParserTypedHooks<Hooks>>;
    type Entry = fn(&mut TestParser) -> Result<antlr4_runtime::ParseTree, AntlrError>;

    fn run(entry: Entry) -> Vec<i64> {
        let values = Rc::new(RefCell::new(Vec::new()));
        let lexer = ForwardedLexer::new(InputStream::new(""));
        let mut parser = ForwardedParser::with_typed_hooks(
            CommonTokenStream::new(lexer),
            Hooks {
                values: Rc::clone(&values),
            },
        );
        entry(&mut parser).expect("fixture input should parse");
        let result = values.borrow().clone();
        result
    }

    #[test]
    fn generated_and_interpreted_paths_forward_arguments() {
        assert_eq!(run(TestParser::generated), [29]);
        assert_eq!(run(TestParser::interpreted), [31]);
    }
}
"####;

    assert_generated_project(
        temp.path(),
        &["forwarded_lexer.rs", "forwarded_parser.rs"],
        test_source,
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn parser_action_hook_signatures_reject_normalized_conflicts() {
    let temp = temporary_directory("parser-action-signature-conflict");
    let grammar = temp.path().join("Conflict.g4");
    let patterns = temp.path().join("patterns.toml");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar Conflict;\n\
         start: {this.Mark(\"x\");} {this.Mark(1);} A EOF;\n\
         A: 'a';\n",
    )
    .expect("grammar should be writable");
    fs::write(
        &patterns,
        "version = 1\n\
         [[helper]]\n\
         kind = \"parser-action\"\n\
         name = \"Mark\"\n\
         arguments = \"string\"\n\
         returns = \"unit\"\n\
         lower = \"hook\"\n\
         [[helper]]\n\
         kind = \"parser-action\"\n\
         name = \"Mark\"\n\
         arguments = \"integer\"\n\
         returns = \"unit\"\n\
         lower = \"hook\"\n",
    )
    .expect("semantic patterns should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--sem-patterns"),
        patterns.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(!output.status.success(), "conflicting hooks should fail");
    let stderr = utf8(&output.stderr);
    assert!(
        !stderr.contains(&temp.path().display().to_string()),
        "diagnostic must not expose temporary paths: {stderr}"
    );
    insta::assert_snapshot!(
        "parser_action_hook_signature_conflict_diagnostic",
        normalize_current_package_version(stderr)
    );
    assert!(
        !out.exists(),
        "failed generation must not leave partial output"
    );
}

#[test]
fn imported_lexer_action_generates_typed_hook_from_structural_body() {
    let temp = temporary_directory("imported-lexer-hook");
    let root = temp.path().join("RootLexer.g4");
    let delegate = temp.path().join("DelegateLexer.g4");
    let patterns = temp.path().join("patterns.toml");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "lexer grammar RootLexer;\nimport DelegateLexer;\nB: 'b';\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "lexer grammar DelegateLexer;\nA: 'a' {this.handle(\"a\");};\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(
        &patterns,
        "version = 1\n\
         [[helper]]\n\
         kind = \"lexer-action\"\n\
         name = \"handle\"\n\
         arguments = \"string\"\n\
         returns = \"unit\"\n\
         lower = \"hook\"\n",
    )
    .expect("semantic patterns should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--sem-patterns"),
        patterns.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let lexer = fs::read_to_string(out.join("root_lexer.rs")).expect("lexer should be emitted");
    assert!(lexer.contains("pub trait RootLexerHooks"), "{lexer}");
    assert!(lexer.contains("fn handle"), "{lexer}");
    assert!(lexer.contains("self.0.handle(ctx, \"a\")"), "{lexer}");
}

#[test]
fn imported_rule_arguments_and_locals_use_structural_call_owners() {
    let temp = temporary_directory("imported-rule-arguments");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: outer EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\n\
         outer locals [boolean seen=false]\n\
             : {$seen=true;} {$seen}? target[true]\n\
             ;\n\
         target[boolean enabled]: ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(&tokens, "lexer grammar Tokens;\nID: [a-z]+;\n")
        .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("let mut __antlr_local_seen = false;"),
        "{parser}"
    );
    assert!(
        parser.contains("dispatch_generated_rule(2, 1, false)"),
        "{parser}"
    );
}

#[test]
fn imported_embedded_action_uses_structural_rule_and_transition_owner() {
    let temp = temporary_directory("imported-embedded-action");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\n\
         delegated: {writeln!(self.output(), \"delegated\").unwrap();} ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(&tokens, "lexer grammar Tokens;\nID: [a-z]+;\n")
        .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
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
    let parser = fs::read_to_string(out.join("root.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("writeln!(self.output(), \"delegated\").unwrap();"),
        "{parser}"
    );
}
