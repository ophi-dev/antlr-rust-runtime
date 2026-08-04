#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

/// End-to-end binary-parsing example: generate the MIDI recognizer from the
/// committed byte-oriented grammar, then parse a real Standard MIDI File
/// through a `ByteStream`. Exercises the whole binary path — raw high bytes as
/// codepoints, a `SemanticHooks` chunk-length superClass emitting synthesized
/// `END_OF_CHUNK` tokens (the `bencoding` pattern), and the generated parser.
#[test]
fn midi_binary_grammar_parses_standard_midi_file_over_byte_stream() {
    let temp = temporary_directory("midi-binary");
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/antlr4-rust-gen/midi-binary");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        dir.join("MidiLexer.g4").as_os_str(),
        dir.join("MidiParser.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        dir.join("patterns.toml").as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    // The bare `{beginChunk();}` lexer action lowers to a typed hook method.
    let lexer = fs::read_to_string(out.join("midi_lexer.rs")).expect("lexer should be emitted");
    assert!(lexer.contains("pub trait MidiLexerHooks"), "{lexer}");
    assert!(lexer.contains("fn begin_chunk"), "{lexer}");

    let fixture = dir.join("twinkle.mid");
    let fixture = fixture.to_str().expect("fixture path should be UTF-8");
    let test_source = format!(
        r####"
#[cfg(test)]
mod midi_tests {{
    use super::midi_lexer::{{MidiLexer, MidiLexerHooks, END_OF_CHUNK}};
    use super::midi_parser::MidiParser;
    use antlr4_runtime::{{
        ByteStream, CommonTokenStream, LexerLifecycleCtx, LexerSemCtx, Parser as _, Token as _,
    }};

    /// A minimal chunk-framing superClass: reads each MThd/MTrk chunk's declared
    /// byte length and synthesizes END_OF_CHUNK once the body is consumed — the
    /// "read N, then frame N bytes" pattern, in Rust, on plain `ByteStream`.
    #[derive(Default)]
    struct MidiHooks {{
        end_of_chunk: Option<usize>,
    }}

    impl MidiLexerHooks for MidiHooks {{
        fn begin_chunk<I>(&mut self, ctx: &mut LexerSemCtx<'_, I>)
        where
            I: antlr4_runtime::CharStream,
        {{
            // The header token just matched magic(4) + big-endian length(4).
            // Read the four length bytes RAW via lookbehind — `text_so_far()`
            // would return `ByteStream`'s hex rendering, not the bytes.
            let b3 = ctx.la(-4) as u32;
            let b2 = ctx.la(-3) as u32;
            let b1 = ctx.la(-2) as u32;
            let b0 = ctx.la(-1) as u32;
            let len = ((b3 << 24) | (b2 << 16) | (b1 << 8) | b0) as usize;
            self.end_of_chunk = Some(ctx.position() + len);
        }}

        fn lexer_before_token<I>(&mut self, ctx: &mut LexerLifecycleCtx<'_, I>)
        where
            I: antlr4_runtime::CharStream,
        {{
            // Fires after the previous body token was emitted and before the
            // next match — the clean point to close the chunk so END_OF_CHUNK
            // lands AFTER the last body token rather than inverting with it.
            if let Some(end) = self.end_of_chunk {{
                let pos = ctx.input_position();
                if pos >= end {{
                    self.end_of_chunk = None;
                    ctx.pop_mode();
                    ctx.enqueue_token(END_OF_CHUNK, pos.saturating_sub(1));
                }}
            }}
        }}
    }}

    fn parse(bytes: Vec<u8>) -> (Vec<i32>, usize) {{
        let lexer = MidiLexer::with_typed_hooks(ByteStream::new(bytes.clone()), MidiHooks::default());
        let mut stream = CommonTokenStream::new(lexer);
        stream.fill();
        let types: Vec<i32> = stream.tokens().map(|t| t.token_type()).collect();

        let lexer = MidiLexer::with_typed_hooks(ByteStream::new(bytes), MidiHooks::default());
        let mut parser = MidiParser::new(CommonTokenStream::new(lexer));
        parser.file().expect("well-formed MIDI parses");
        (types, parser.number_of_syntax_errors())
    }}

    #[test]
    fn parses_a_real_standard_midi_file() {{
        let bytes = include_bytes!({fixture:?}).to_vec();
        let (types, errors) = parse(bytes);

        // BEGIN_HEADER, six HDR_BYTE, END_OF_CHUNK; BEGIN_TRACK, four
        // (DELTA_TIME, event) pairs, END_OF_CHUNK; EOF (-1).
        assert_eq!(errors, 0, "no syntax errors on a well-formed file");
        assert_eq!(
            types,
            vec![
                2, // BEGIN_HEADER
                4, 4, 4, 4, 4, 4, // six HDR_BYTE (format, ntracks, division)
                1, // END_OF_CHUNK (MThd body framed by its length = 6)
                3, // BEGIN_TRACK
                5, 7, // delta, NOTE_ON
                5, 6, // delta, NOTE_OFF
                5, 9, // delta, META_SET_TEMPO
                5, 8, // delta, META_END_OF_TRACK
                1,  // END_OF_CHUNK (MTrk body framed by its length = 19)
                -1, // EOF
            ],
        );
    }}
}}
"####
    );

    assert_generated_project(
        temp.path(),
        &["midi_lexer.rs", "midi_parser.rs"],
        &test_source,
    );
}

#[test]
fn deeply_nested_input_parses_without_native_stack_overflow() {
    let temp = temporary_directory("deep-nesting");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/deep-nesting/Nest.g4");
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

    // Every generated dispatch method must use the shared lifecycle contract;
    // the 10,000-level parse below proves its segmented-stack guard is active.
    let parser = fs::read_to_string(out.join("nest_parser.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("antlr4_runtime::__antlr4_rust_generated_rule!"),
        "generated dispatch must use the shared rule lifecycle\n{parser}"
    );

    assert_generated_project(
        temp.path(),
        &["nest_lexer.rs", "nest_parser.rs"],
        r#"
#[cfg(test)]
mod deep_nesting_tests {
    use super::nest_lexer::NestLexer;
    use super::nest_parser::{parse, NestParser};
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    fn nested(depth: usize) -> String {
        format!("{}a{}", "[".repeat(depth), "]".repeat(depth))
    }

    #[test]
    fn ten_thousand_levels_parse_on_the_default_test_stack() {
        // Rust test threads default to a 2 MiB stack; without segmented-stack
        // growth this depth aborted the process (issue #193).
        let parsed = parse(&nested(10_000), NestLexer::new, NestParser::s)
            .expect("deeply nested input should parse");
        assert!(parsed.tree().as_rule().is_some());
    }

    #[test]
    fn max_rule_depth_bounds_adversarial_nesting() {
        // Callers parsing untrusted input can cap rule nesting (issue #198):
        // shallow input parses, input past the cap fails with a positioned
        // error even though rule-level recovery would produce a tree, and the
        // violation does not leak into the parser's next parse. (No Nest rule
        // is ATN-preferred — the cap-overrides-ATN-preference guard is pinned
        // by the generator's dispatch-rendering unit test.)
        let lexer = NestLexer::new(InputStream::new(&nested(4)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        assert!(parser.s().is_ok(), "shallow input parses under the cap");

        let lexer = NestLexer::new(InputStream::new(&nested(1_000)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        let error = parser.s().expect_err("cap must reject deep nesting");
        assert!(
            error.to_string().contains("rule nesting depth limit of 64"),
            "unexpected error: {error}"
        );

        let lexer = NestLexer::new(InputStream::new(&nested(4)));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        assert!(
            parser.s().is_ok(),
            "reused parser starts clean after a depth violation"
        );
    }

    #[test]
    fn max_rule_depth_counts_left_recursive_expansions() {
        // `a+a+a+...` deepens the tree one level per operator without pushing
        // a rule frame. Upstream ANTLR fires a rule-entry listener event for
        // each expansion, so listener-based depth counters reject it — the
        // cap must too, or a 2000-term chain builds a 2000-deep tree under
        // any configured bound.
        let chain = vec!["a"; 2_000].join("+");
        let lexer = NestLexer::new(InputStream::new(&chain));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        let error = parser
            .s()
            .expect_err("operator expansions must count toward the cap");
        assert!(
            error.to_string().contains("rule nesting depth limit of 64"),
            "unexpected error: {error}"
        );

        // The same chain parses when uncapped, and a short chain fits.
        let lexer = NestLexer::new(InputStream::new(&chain));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        assert!(parser.s().is_ok(), "uncapped operator chain parses");
        let lexer = NestLexer::new(InputStream::new("a+a+a"));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        assert!(parser.s().is_ok(), "short operator chain fits the cap");
    }

    #[test]
    fn max_rule_depth_expansion_boundary_matches_frame_boundary() {
        // The expansion check runs BEFORE the expansion push, mirroring the
        // dispatch site's check before its rule-frame push: each extra
        // operator costs exactly one depth level. Pin that alignment by
        // finding the minimal cap admitting an N-operator chain and asserting
        // one more operator needs exactly one more level.
        fn parses_at(cap: usize, operators: usize) -> bool {
            let chain = vec!["a"; operators + 1].join("+");
            let lexer = NestLexer::new(InputStream::new(&chain));
            let mut parser = NestParser::new(CommonTokenStream::new(lexer));
            parser.set_max_rule_depth(Some(cap));
            parser.s().is_ok()
        }

        let minimal_cap = (1..128)
            .find(|cap| parses_at(*cap, 4))
            .expect("some cap admits a 4-operator chain");
        assert!(
            !parses_at(minimal_cap - 1, 4),
            "minimal cap should be exact"
        );
        assert!(
            !parses_at(minimal_cap, 5),
            "one more operator must exceed the same cap"
        );
        assert!(
            parses_at(minimal_cap + 1, 5),
            "one more operator must need exactly one more level"
        );
    }

    #[test]
    fn depth_violation_survives_recovery_and_does_not_poison_reuse() {
        // A violation absorbed by mid-tree recovery must still fail the parse
        // (the resource bound was hit), the reported error must be the depth
        // cap rather than a derived syntax error, and a second entry-rule
        // call on the same parser instance must not inherit the violation.
        let source = format!("{}a", "[".repeat(200));
        let lexer = NestLexer::new(InputStream::new(&source));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        let error = parser
            .s()
            .expect_err("depth violation must fail the parse even after recovery");
        assert!(
            error.to_string().contains("rule nesting depth limit of 64"),
            "depth cap must win over derived syntax errors: {error}"
        );

        let lexer = NestLexer::new(InputStream::new("a"));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        assert!(
            parser.expr().is_ok(),
            "a different entry rule on the same instance starts clean"
        );
    }

    /// The cel-rust `RecursionListener` shape: count `expr` nesting live,
    /// abort the parse past a limit — ported verbatim onto
    /// `add_parse_listener` (issue #202).
    struct RecursionListener {
        max: u16,
        depth: u16,
        high_water: std::sync::Arc<std::sync::atomic::AtomicU16>,
    }

    impl antlr4_runtime::ParseListener for RecursionListener {
        fn enter_every_rule(
            &mut self,
            event: &antlr4_runtime::EnterRuleEvent<'_>,
        ) -> Result<(), antlr4_runtime::AntlrError> {
            if event.rule_index == super::nest_parser::RULE_EXPR {
                self.depth += 1;
                self.high_water
                    .fetch_max(self.depth, std::sync::atomic::Ordering::Relaxed);
            }
            if self.depth > self.max {
                use antlr4_runtime::Token as _;
                let (line, column) = event
                    .current
                    .as_ref()
                    .map_or((0, 0), |token| (token.line(), token.column()));
                return Err(antlr4_runtime::AntlrError::ParserError {
                    line,
                    column,
                    message: format!("Recursion limit of {} exceeded", self.max),
                    offending: event.current.as_ref().map(antlr4_runtime::Token::token_id),
                });
            }
            Ok(())
        }

        fn exit_every_rule(&mut self, rule_index: usize) {
            if rule_index == super::nest_parser::RULE_EXPR {
                self.depth -= 1;
            }
        }
    }

    /// Records the event stream for order/balance assertions.
    struct TracingListener {
        tag: &'static str,
        events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl antlr4_runtime::ParseListener for TracingListener {
        fn enter_every_rule(
            &mut self,
            event: &antlr4_runtime::EnterRuleEvent<'_>,
        ) -> Result<(), antlr4_runtime::AntlrError> {
            self.events
                .lock()
                .expect("trace lock")
                .push(format!("enter{}:{}", self.tag, event.rule_index));
            Ok(())
        }

        fn exit_every_rule(&mut self, rule_index: usize) {
            self.events
                .lock()
                .expect("trace lock")
                .push(format!("exit{}:{}", self.tag, rule_index));
        }
    }

    #[test]
    fn parse_listener_counts_rules_and_aborts_past_a_limit() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU16, Ordering};

        // Under the limit: events fire, parse succeeds, enter/exit balance
        // (depth returns to zero, so high-water == max nesting seen).
        let high_water = Arc::new(AtomicU16::new(0));
        let lexer = NestLexer::new(InputStream::new(&nested(3)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 32,
            depth: 0,
            high_water: Arc::clone(&high_water),
        });
        assert!(parser.s().is_ok(), "shallow input parses under the limit");
        assert_eq!(
            high_water.load(Ordering::Relaxed),
            4,
            "one expr per bracket level plus the outermost expr"
        );

        // Successful left-recursive chain: the live depth counter returns to
        // its starting value. Proven through the public API: parse the same
        // under-limit chain twice with one listener instance — any residual
        // depth from parse one would raise parse two's high-water mark.
        let high_water = Arc::new(AtomicU16::new(0));
        let chain = vec!["a"; 20].join("+");
        let lexer = NestLexer::new(InputStream::new(&chain));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 1_000,
            depth: 0,
            high_water: Arc::clone(&high_water),
        });
        assert!(parser.s().is_ok(), "under-limit operator chain parses");
        let first_peak = high_water.load(Ordering::Relaxed);
        assert!(first_peak > 0, "the chain nests expr rules");
        let lexer = NestLexer::new(InputStream::new(&chain));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        assert!(parser.s().is_ok(), "same chain parses again");
        assert_eq!(
            high_water.load(Ordering::Relaxed),
            first_peak,
            "depth returned to zero after the successful LR parse"
        );

        // Past the limit: the listener aborts with its own positioned error,
        // sticky through recovery.
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 8,
            depth: 0,
            high_water: Arc::new(AtomicU16::new(0)),
        });
        let error = parser.s().expect_err("listener abort must fail the parse");
        assert!(
            error.to_string().contains("Recursion limit of 8 exceeded"),
            "unexpected error: {error}"
        );

        // Flat operator chains do NOT accumulate live listener depth: each
        // loop pass exits the outgoing iteration before the next expansion
        // enters (upstream recRuleSetPrevCtx), so a 40-term `a+a+...` chain
        // peaks at expr depth 2 in every ANTLR target — including this one —
        // and parses fine under a limit of 8.
        let chain = vec!["a"; 40].join("+");
        let high_water = Arc::new(AtomicU16::new(0));
        let lexer = NestLexer::new(InputStream::new(&chain));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 8,
            depth: 0,
            high_water: Arc::clone(&high_water),
        });
        assert!(
            parser.s().is_ok(),
            "flat operator chain stays at Java's live depth"
        );
        assert_eq!(
            high_water.load(Ordering::Relaxed),
            2,
            "operator chain peaks at depth 2, matching the Java oracle"
        );

        // The abort does not poison the instance: clearing listeners (which
        // also returns them and drops the sticky abort) and reusing the
        // parser parses clean input.
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(RecursionListener {
            max: 8,
            depth: 0,
            high_water: Arc::new(AtomicU16::new(0)),
        });
        let error = parser.s().expect_err("nested input exceeds the limit");
        assert!(
            error.to_string().contains("Recursion limit of 8 exceeded"),
            "unexpected error: {error}"
        );
        let lexer = NestLexer::new(InputStream::new("a"));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        let mut removed = parser.remove_parse_listeners();
        assert_eq!(removed.len(), 1, "removed listeners are handed back");
        assert!(parser.s().is_ok(), "reused parser starts clean");

        // Returned boxes re-register as-is (ParseListener is implemented for
        // Box<dyn ParseListener>), preserving accumulated listener state.
        let boxed = removed.pop().expect("one listener was removed");
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        parser.add_parse_listener(boxed);
        let error = parser
            .s()
            .expect_err("re-registered listener still enforces its limit");
        assert!(
            error.to_string().contains("Recursion limit of 8 exceeded"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parse_listener_event_order_matches_upstream() {
        use std::sync::{Arc, Mutex};

        // Two listeners: enters fire in registration order, exits in reverse
        // (upstream Parser.triggerExitRuleEvent walks back to front), and
        // pairs balance across recovery on malformed input.
        let events = Arc::new(Mutex::new(Vec::new()));
        let lexer = NestLexer::new(InputStream::new("[a]"));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(TracingListener {
            tag: "A",
            events: Arc::clone(&events),
        });
        parser.add_parse_listener(TracingListener {
            tag: "B",
            events: Arc::clone(&events),
        });
        assert!(parser.s().is_ok());
        let trace = events.lock().expect("trace lock").clone();
        let s_rule = super::nest_parser::RULE_S;
        assert_eq!(trace.first().map(String::as_str), Some(format!("enterA:{s_rule}").as_str()));
        assert_eq!(trace.get(1).map(String::as_str), Some(format!("enterB:{s_rule}").as_str()));
        // Last two events close the entry rule: B exits before A.
        assert_eq!(
            trace.last().map(String::as_str),
            Some(format!("exitA:{s_rule}").as_str())
        );
        assert_eq!(
            trace.get(trace.len() - 2).map(String::as_str),
            Some(format!("exitB:{s_rule}").as_str())
        );
        // Balance: every rule index enters exactly as often as it exits,
        // for both listeners.
        let count = |needle: &str| trace.iter().filter(|event| event.starts_with(needle)).count();
        assert_eq!(count("enterA:"), count("exitA:"));
        assert_eq!(count("enterB:"), count("exitB:"));

        // Recovery path: malformed input still balances.
        let events = Arc::new(Mutex::new(Vec::new()));
        let lexer = NestLexer::new(InputStream::new("[a"));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(TracingListener {
            tag: "R",
            events: Arc::clone(&events),
        });
        let _ = parser.s();
        let trace = events.lock().expect("trace lock").clone();
        let count = |needle: &str| trace.iter().filter(|event| event.starts_with(needle)).count();
        assert_eq!(
            count("enterR:"),
            count("exitR:"),
            "recovery keeps pairs balanced: {trace:?}"
        );

        // Successful left-recursive operator chain: each expansion fires a
        // simulated enter (upstream triggerEnterRuleEvent parity) and the
        // unroll fires the matching exits. `a+a+a+a` yields exactly 7
        // RULE_EXPR pairs: 1 rule dispatch + 3 expansions + 3 right-operand
        // dispatches.
        let events = Arc::new(Mutex::new(Vec::new()));
        let lexer = NestLexer::new(InputStream::new("a+a+a+a"));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.add_parse_listener(TracingListener {
            tag: "L",
            events: Arc::clone(&events),
        });
        assert!(parser.s().is_ok(), "operator chain parses");
        let trace = events.lock().expect("trace lock").clone();
        let expr_rule = super::nest_parser::RULE_EXPR;
        let count = |needle: String| trace.iter().filter(|event| **event == needle).count();
        assert_eq!(
            count(format!("enterL:{expr_rule}")),
            7,
            "expr enters = dispatch + expansions + operands: {trace:?}"
        );
        assert_eq!(
            count(format!("enterL:{expr_rule}")),
            count(format!("exitL:{expr_rule}")),
            "successful LR unroll balances expansion exits: {trace:?}"
        );
    }

    #[test]
    fn depth_cap_and_listener_abort_coexist() {
        // Whichever bound trips first surfaces; the other never fires because
        // the sticky abort stops rule entries (and thus stack growth). With a
        // tight listener limit the listener error wins the race...
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(64));
        parser.add_parse_listener(RecursionListener {
            max: 1,
            depth: 0,
            high_water: std::sync::Arc::new(std::sync::atomic::AtomicU16::new(0)),
        });
        let error = parser.s().expect_err("listener limit must fail the parse");
        assert!(
            error.to_string().contains("Recursion limit of 1 exceeded"),
            "listener abort surfaces when it trips first: {error}"
        );

        // ...and with a tight cap the depth violation wins the race.
        let lexer = NestLexer::new(InputStream::new(&nested(64)));
        let mut parser = NestParser::new(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(Some(8));
        parser.add_parse_listener(RecursionListener {
            max: 1_000,
            depth: 0,
            high_water: std::sync::Arc::new(std::sync::atomic::AtomicU16::new(0)),
        });
        let error = parser.s().expect_err("depth cap must fail the parse");
        assert!(
            error.to_string().contains("rule nesting depth limit of 8"),
            "depth-cap violation surfaces when it trips first: {error}"
        );

        let lexer = NestLexer::new(InputStream::new("a"));
        parser.set_token_stream(CommonTokenStream::new(lexer));
        parser.set_max_rule_depth(None);
        let _ = parser.remove_parse_listeners();
        assert!(parser.s().is_ok(), "instance is clean after either abort");
    }
}
"#,
    );
}

/// Issue #206: a grammar keeping its interpolation state inline in
/// `@lexer::members` — C# bodies, a nesting counter, and two stacks — generates
/// with **zero** hand-written hooks, purely from a `stack_member` pattern file.
///
/// The expected token streams below are not hand-derived: they were captured
/// from an ANTLR 4.13.2 **Java** lexer generated from the same grammar (bodies
/// mechanically ported to Java, semantics unchanged) and match byte-for-byte.
/// That is the differential validation the issue asks for — the stack state is
/// load-bearing in cases 4 and 5, where a scalar-only flag would leak across
/// adjacent strings.
#[test]
fn inline_lexer_member_stacks_generate_without_hooks() {
    let temp = temporary_directory("stack-member-lexer");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/stack-member-lexer");
    let out = temp.path().join("generated");

    // `--sem-unknown error` is the real assertion: generation fails if any
    // coordinate is left to a policy fallback or needs a hook.
    let output = run_antlr4_rust_gen(&[
        dir.join("CSharpInterpolation.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        dir.join("patterns.toml").as_os_str(),
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

    let manifest = fs::read_to_string(out.join("semantics.json")).expect("manifest is emitted");
    assert!(
        !manifest.contains("\"hooked\""),
        "no coordinate may need a hook: {manifest}"
    );
    assert!(
        !manifest.contains("\"assume-true\"") && !manifest.contains("\"assume-false\""),
        "no coordinate may fall back to a policy: {manifest}"
    );

    // Member state lowers into the SemIR table, not inline Rust or a hook trait.
    let lexer =
        fs::read_to_string(out.join("c_sharp_interpolation.rs")).expect("lexer should be emitted");
    for expected in [
        "fn lexer_semantics()",
        "AStmt::PushMember",
        "AStmt::PopMember",
        "PExpr::MemberTop",
    ] {
        assert!(lexer.contains(expected), "missing {expected} in: {lexer}");
    }
    assert!(
        !lexer.contains("pub trait CSharpInterpolationHooks"),
        "grammar must need no hook trait: {lexer}"
    );

    let test_source = r####"
#[cfg(test)]
mod stack_member_tests {
    use super::c_sharp_interpolation::CSharpInterpolation;
    use antlr4_runtime::{CommonTokenStream, InputStream, IntStream as _, Token as _};

    fn lex(input: &str) -> String {
        let lexer = CSharpInterpolation::new(InputStream::new(input));
        let mut stream = CommonTokenStream::new(lexer);
        stream.fill();
        (0..stream.size())
            .filter_map(|index| stream.get(index))
            .map(|token| {
                format!("({}, {})", token.token_type(), token.text().unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Each expectation is the ANTLR 4.13.2 Java lexer's output for the same
    /// grammar and input.
    #[test]
    fn matches_the_java_oracle_token_stream() {
        // Regular string: `{ !verbatium }?` admits REGULAR_STRING_INSIDE (8).
        assert_eq!(
            lex(r#"$"abc""#),
            r#"(1, $") (8, abc) (7, ") (-1, <EOF>)"#
        );
        // Verbatim string: `{ verbatium }?` admits VERBATIUM_INSIDE_STRING (9)
        // and lets `""` lex as one token (6) instead of closing the string.
        assert_eq!(
            lex(r#"$@"a""b""#),
            r#"(2, $@") (9, a) (6, "") (9, b) (7, ") (-1, <EOF>)"#
        );
        // Interpolation hole: `{` pushes curlyLevels and DEFAULT_MODE.
        assert_eq!(
            lex(r#"$"a{x}b""#),
            r#"(1, $") (8, a) (3, x) (3, b) (-1, <EOF>)"#
        );
        // Verbatim then regular: popping must clear the flag, or `y` would
        // wrongly lex as VERBATIUM_INSIDE_STRING.
        assert_eq!(
            lex(r#"$@"x"$"y""#),
            r#"(2, $@") (9, x) (7, ") (1, $") (8, y) (7, ") (-1, <EOF>)"#
        );
        // Regular then verbatim: the second string's `""` must still be one
        // token, so the flag has to be restored per string, not left false.
        assert_eq!(
            lex(r#"$"p"$@"q""r""#),
            r#"(1, $") (8, p) (7, ") (2, $@") (9, q) (6, "") (9, r) (7, ") (-1, <EOF>)"#
        );
    }

    /// A reused lexer must not carry interpolation state across inputs.
    #[test]
    fn state_does_not_leak_between_inputs() {
        let first = lex(r#"$@"a""b""#);
        let second = lex(r#"$"c""#);
        assert_eq!(second, r#"(1, $") (8, c) (7, ") (-1, <EOF>)"#);
        assert_ne!(first, second);
    }
}
"####;

    assert_generated_project(temp.path(), &["c_sharp_interpolation.rs"], test_source);
}

/// Issue #206 review: a grammar whose inline member declares a non-default
/// initializer must honor it. `{ enabled }?` over `private bool enabled = true;`
/// has to pass on a fresh lexer; a slot silently starting at 0 would reject
/// input the source grammar accepts while still reporting itself `translated`.
///
/// The expected token stream is an ANTLR 4.13.2 **Java** lexer's output for the
/// same grammar (`(1, a) (2, b) (-1, <EOF>)`).
#[test]
fn declared_member_initializers_reach_the_generated_lexer() {
    let temp = temporary_directory("member-initializer");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/member-initializer");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        dir.join("L.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        dir.join("patterns.toml").as_os_str(),
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

    let lexer = fs::read_to_string(out.join("l.rs")).expect("lexer should be emitted");
    assert!(
        lexer.contains(".with_initial_members([(0, 1)])"),
        "the declared initializer must seed the slot: {lexer}"
    );

    let test_source = r####"
#[cfg(test)]
mod member_initializer_tests {
    use super::l::L;
    use antlr4_runtime::{CommonTokenStream, InputStream, IntStream as _, Token as _};

    fn lex(input: &str) -> String {
        let lexer = L::new(InputStream::new(input));
        let mut stream = CommonTokenStream::new(lexer);
        stream.fill();
        (0..stream.size())
            .filter_map(|index| stream.get(index))
            .map(|token| {
                format!("({}, {})", token.token_type(), token.text().unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Matches the ANTLR 4.13.2 Java lexer for the same grammar and input.
    #[test]
    fn initialized_member_admits_its_guarded_rule() {
        assert_eq!(lex("ab"), "(1, a) (2, b) (-1, <EOF>)");
    }
}
"####;

    assert_generated_project(temp.path(), &["l.rs"], test_source);
}

/// Issue #206 review: a combined grammar's **parser** members need the same
/// initializer seeding the lexer got, and lexer/parser inventories must be
/// independent — a combined grammar may legally declare same-named members with
/// different kinds in each recognizer.
///
/// Expectations come from an ANTLR 4.13.2 **Java** parser built from the same
/// grammar: `"a"` parses with 0 syntax errors, `"b"` with 1.
#[test]
fn declared_parser_member_initializers_reach_the_generated_parser() {
    let temp = temporary_directory("parser-member-initializer");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/parser-member-initializer");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        dir.join("P.g4").as_os_str(),
        OsStr::new("--sem-patterns"),
        dir.join("patterns.toml").as_os_str(),
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

    // Each recognizer seeds its own slot 0 from its own inventory: the lexer's
    // `bool level = true` and the parser's `int level = 2`.
    let lexer = fs::read_to_string(out.join("p_lexer.rs")).expect("lexer should be emitted");
    assert!(
        lexer.contains(".with_initial_members([(0, 1)])"),
        "lexer must seed its own member: {lexer}"
    );
    let parser = fs::read_to_string(out.join("p_parser.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("base.set_initial_members([(0, 2)]);"),
        "parser must seed its own member: {parser}"
    );

    let test_source = r####"
#[cfg(test)]
mod parser_member_initializer_tests {
    use super::p_lexer::PLexer;
    use super::p_parser::PParser;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    fn syntax_errors(input: &str) -> usize {
        let lexer = PLexer::new(InputStream::new(input));
        let mut parser = PParser::new(CommonTokenStream::new(lexer));
        let _ = parser.s();
        parser.number_of_syntax_errors()
    }

    /// Matches the ANTLR 4.13.2 Java parser for the same grammar.
    ///
    /// `"a"` needs *both* declared initializers: the lexer's `level = true`
    /// admits `A`, and the parser's `level = 2` satisfies `{ level == 2 }?`.
    /// A slot silently starting at 0 on either side would fail it.
    #[test]
    fn both_recognizers_observe_their_own_declared_initial_values() {
        assert_eq!(syntax_errors("a"), 0);
        assert_eq!(syntax_errors("b"), 1);
    }
}
"####;

    assert_generated_project(temp.path(), &["p_lexer.rs", "p_parser.rs"], test_source);
}

/// Issue #151: a grammar with mutual (indirect) left recursion — which ANTLR
/// 4.13.2 rejects with error(119) — is reduced to direct left recursion and
/// generates a working precedence-climbing parser. The fixture distills the
/// tractable Roslyn cycle shapes: a hub-and-spoke expression cycle (including
/// the leading-optional range operator) and a two-rule `name` cycle. The
/// asserted trees are byte-identical to what ANTLR's own runtime produces from
/// the equivalent hand-inlined grammar.
#[test]
fn mutual_left_recursion_is_reduced_to_a_working_precedence_parser() {
    let temp = temporary_directory("mutual-left-recursion");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/mutual-left-recursion/MutualExpr.g4");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "mutual left recursion should now compile, not error(119)\nstdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let parser =
        fs::read_to_string(out.join("mutual_expr_parser.rs")).expect("parser should be emitted");
    // Hub-only satellites collapse into their hub; the hub becomes a rule method.
    // The generated parser is tens of thousands of lines, so failures report the
    // matching lines rather than the whole file.
    for collapsed in [
        "add_expr",
        "mul_expr",
        "call_expr",
        "range_expr",
        "qualified_name",
    ] {
        let needle = format!("fn {collapsed}(");
        let offenders = matching_lines(&parser, &needle);
        assert!(
            offenders.is_empty(),
            "hub-only satellite {collapsed:?} should be inlined away, found:\n{offenders}"
        );
    }
    for hub in ["fn expr(", "fn name(", "fn primary("] {
        assert!(
            parser.contains(hub),
            "hub {hub:?} should survive; emitted rule methods:\n{}",
            matching_lines(&parser, "    pub fn ")
        );
    }

    assert_generated_project(
        temp.path(),
        &["mutual_expr_lexer.rs", "mutual_expr_parser.rs"],
        r#"
#[cfg(test)]
mod mutual_left_recursion_tests {
    use super::mutual_expr_lexer::MutualExprLexer;
    use super::mutual_expr_parser::{parse, rule_names};
    use antlr4_runtime::tree::{Node, NodeKind};

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
            NodeKind::Terminal => out.push_str(&node.as_terminal().expect("terminal").text()),
            NodeKind::Error => out.push_str("<error>"),
        }
    }

    fn tree_of(src: &str) -> String {
        let parsed = parse(src, MutualExprLexer::new, |p| p.expr())
            .unwrap_or_else(|error| panic!("{src:?} should parse: {error}"));
        let mut out = String::new();
        lisp(parsed.tree(), rule_names(), &mut out);
        out
    }

    #[test]
    fn collapsed_cycles_match_antlr_trees() {
        // Precedence-climbing over the collapsed hub (default alt-order
        // precedence: `+` binds looser than `*`), left-associative.
        assert_eq!(
            tree_of("1+2*3"),
            "(expr (expr (expr (primary 1)) + (expr (primary 2))) * (expr (primary 3)))"
        );
        // Two-rule name cycle collapsed to left-recursive `name`.
        assert_eq!(tree_of("a.b.c"), "(expr (primary (name (name (name a) . b) . c)))");
        // Leading-optional range operator split into `expr '..' expr?` + primary.
        assert_eq!(
            tree_of("x..y"),
            "(expr (expr (primary (name x))) .. (expr (primary (name y))))"
        );
        assert_eq!(
            tree_of("f()..g()"),
            "(expr (expr (expr (primary (name f))) ( )) .. (expr (expr (primary (name g))) ( )))"
        );
    }
}
"#,
    );
}

/// Issue #269: terminals from a hub-only mutual-left-recursion satellite must
/// remain reachable without descending into nested expression children. The
/// operators are anonymous literals, so no stable per-token accessor name
/// exists; `direct_terminals()` provides the typed, grammar-agnostic surface.
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn inlined_satellite_terminals_are_typed_direct_children() {
    let temp = temporary_directory("inlined-token-accessors");
    let grammar = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/inlined-token-accessors/InlinedTokens.g4");
    let out = temp.path().join("generated");

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

    let parser =
        fs::read_to_string(out.join("inlined_tokens_parser.rs")).expect("parser should be emitted");
    insta::assert_debug_snapshot!(
        "inlined_token_accessors_generated_api",
        generated_parser_api(&parser)
    );

    assert_generated_project(
        temp.path(),
        &["inlined_tokens_lexer.rs", "inlined_tokens_parser.rs"],
        r####"
// Inline snapshots are intentional: the temporary crate is deleted after this
// test, so external snapshots cannot be accepted from the repository.
#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod inlined_token_tests {
    use super::inlined_tokens_lexer::InlinedTokensLexer;
    use super::inlined_tokens_parser::{
        ActiveContext, CollisionContext, ErrorNode, ExprContext, InlinedTokensListener,
        InlinedTokensParser, RecoveredContext, StartContext,
    };
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};
    use std::convert::Infallible;

    fn detached_direct_terminals<'a>(
        context: ExprContext<'a>,
    ) -> impl Iterator<Item = String> + 'a {
        context
            .direct_terminals()
            .map(|terminal| terminal.to_string())
    }

    fn direct_expression_terminals(input: &str) -> Vec<String> {
        let lexer = InlinedTokensLexer::new(InputStream::new(input));
        let mut parser = InlinedTokensParser::new(CommonTokenStream::new(lexer));
        let root = parser.start().expect("operator input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        let start = parsed
            .tree()
            .as_rule()
            .expect("start rule")
            .downcast_ref::<StartContext>()
            .expect("typed start context");
        let expression: ExprContext<'_> = start.expr().expect("root expression");
        detached_direct_terminals(expression).collect()
    }

    #[test]
    fn returns_only_the_operator_owned_by_the_hub_context() {
        insta::assert_debug_snapshot!(
            [
                ("assignment", direct_expression_terminals("left=right")),
                ("addition", direct_expression_terminals("left+right")),
                ("subtraction", direct_expression_terminals("left-right")),
            ],
            @r###"
        [
            (
                "assignment",
                [
                    "=",
                ],
            ),
            (
                "addition",
                [
                    "+",
                ],
            ),
            (
                "subtraction",
                [
                    "-",
                ],
            ),
        ]
        "###
        );
    }

    #[test]
    fn active_context_accessors_use_live_children() {
        let lexer = InlinedTokensLexer::new(InputStream::new("left=right"));
        let mut parser = InlinedTokensParser::new(CommonTokenStream::new(lexer));
        let root = parser.active().expect("active-context input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        let active = parsed
            .tree()
            .as_rule()
            .expect("active rule")
            .downcast_ref::<ActiveContext>()
            .expect("typed active context");

        insta::assert_snapshot!(active.seen, @"left,=");
    }

    #[derive(Default)]
    struct RecoveryTrace {
        errors: Vec<(String, bool)>,
    }

    impl InlinedTokensListener for RecoveryTrace {
        fn visit_error_node(&mut self, node: &ErrorNode) -> Result<(), Infallible> {
            self.errors.push((node.to_string(), node.is_missing()));
            Ok(())
        }
    }

    type RecoveredTerminal = (String, bool, bool);
    type RecoveredError = (String, bool);

    fn recovered_terminals(
        input: &str,
    ) -> (usize, Vec<RecoveredTerminal>, Vec<RecoveredError>) {
        let lexer = InlinedTokensLexer::new(InputStream::new(input));
        let mut parser = InlinedTokensParser::new(CommonTokenStream::new(lexer));
        let root = parser.recovered().expect("invalid input should recover");
        let syntax_errors = parser.number_of_syntax_errors();
        let parsed = parser.into_parsed_file(root);
        let tree = parsed.tree();
        let mut trace = RecoveryTrace::default();
        trace.walk(tree).expect("error-node walk");
        let recovered = tree
            .as_rule()
            .expect("recovered rule")
            .downcast_ref::<RecoveredContext>()
            .expect("typed recovered context");
        let terminals = recovered
            .direct_terminals()
            .map(|token| (token.to_string(), token.is_error(), token.is_missing()))
            .collect::<Vec<_>>();
        (syntax_errors, terminals, trace.errors)
    }

    #[test]
    fn distinguishes_recovered_error_nodes() {
        let (missing_errors, missing, missing_trace) = recovered_terminals("left right");
        assert_eq!(missing_errors, 1);
        let (deleted_errors, deleted, deleted_trace) =
            recovered_terminals("left = = right");
        assert_eq!(deleted_errors, 1);

        insta::assert_debug_snapshot!(
            [
                ("inserted", missing, missing_trace),
                ("deleted", deleted, deleted_trace),
            ],
            @r###"
        [
            (
                "inserted",
                [
                    (
                        "left",
                        false,
                        false,
                    ),
                    (
                        "<missing '='>",
                        true,
                        true,
                    ),
                    (
                        "right",
                        false,
                        false,
                    ),
                    (
                        "<EOF>",
                        false,
                        false,
                    ),
                ],
                [
                    (
                        "<missing '='>",
                        true,
                    ),
                ],
            ),
            (
                "deleted",
                [
                    (
                        "left",
                        false,
                        false,
                    ),
                    (
                        "=",
                        false,
                        false,
                    ),
                    (
                        "=",
                        true,
                        false,
                    ),
                    (
                        "right",
                        false,
                        false,
                    ),
                    (
                        "<EOF>",
                        false,
                        false,
                    ),
                ],
                [
                    (
                        "=",
                        false,
                    ),
                ],
            ),
        ]
        "###
        );
    }

    #[test]
    fn preserves_repeated_direct_token_accessor() {
        let lexer = InlinedTokensLexer::new(InputStream::new("direct direct value"));
        let mut parser = InlinedTokensParser::new(CommonTokenStream::new(lexer));
        let root = parser.collision().expect("collision input should parse");
        assert_eq!(parser.number_of_syntax_errors(), 0);
        let parsed = parser.into_parsed_file(root);
        let collision = parsed
            .tree()
            .as_rule()
            .expect("collision rule")
            .downcast_ref::<CollisionContext>()
            .expect("typed collision context");
        let direct = collision
            .direct_tokens()
            .map(|token| token.to_string())
            .collect::<Vec<_>>();
        let all = collision
            .direct_terminals()
            .map(|token| token.to_string())
            .collect::<Vec<_>>();

        insta::assert_debug_snapshot!(
            [("direct_tokens", direct), ("direct_terminals", all)],
            @r###"
        [
            (
                "direct_tokens",
                [
                    "direct",
                    "direct",
                ],
            ),
            (
                "direct_terminals",
                [
                    "direct",
                    "direct",
                    "value",
                    "<EOF>",
                ],
            ),
        ]
        "###
        );
    }
}
"####,
    );
}
