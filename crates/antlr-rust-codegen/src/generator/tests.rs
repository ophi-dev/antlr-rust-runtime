#![allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
use super::*;
use antlr4_runtime::atn::parser_atn::{ParserAtnBuilder, ParserTransitionSpec};

fn rendered_context_impl<'a>(rendered: &'a str, name: &str) -> &'a str {
    let marker = format!("impl<'a, State: __RecoveryContextState> {name}<'a, State> {{");
    let tail = rendered
        .split_once(&marker)
        .unwrap_or_else(|| panic!("{name} recovery context impl"))
        .1;
    let end = tail
        .find("\nantlr4_runtime::__antlr4_rust_context!")
        .or_else(|| tail.find("\n/// Checks generated required-child invariants"))
        .unwrap_or_else(|| panic!("surface after {name} context"));
    &tail[..end]
}

fn rendered_context_declaration<'a>(rendered: &'a str, name: &str) -> &'a str {
    let marker = format!("antlr4_runtime::__antlr4_rust_context! {{\n    pub struct {name} {{");
    let start = rendered
        .find(&marker)
        .unwrap_or_else(|| panic!("{name} context declaration"));
    let tail = &rendered[start..];
    let end = tail
        .find("\n}\n\n#[allow(dead_code, private_bounds, clippy::all)]")
        .unwrap_or_else(|| panic!("{name} context declaration end"));
    &tail[..end + 2]
}

#[test]
fn renders_module_level_metadata_helpers() {
    let rendered = render_parser_metadata("TParser", &minimal_parser_data());

    assert!(rendered.contains("pub fn metadata() -> &'static GrammarMetadata {\n    &METADATA\n}"));
    assert!(rendered.contains(
        "pub fn rule_names() -> &'static [&'static str] {\n    METADATA.rule_names()\n}"
    ));
}

#[test]
fn generated_recognizers_reuse_cached_static_metadata() {
    let lexer = render_lexer(
        "TLexer",
        &predicate_lexer_data(),
        false,
        SemUnknownPolicy::default(),
        &SemPatternFile::default(),
        false,
    )
    .expect("lexer should render");
    let parser = render_parser("TParser", &minimal_parser_data()).expect("parser should render");

    let lexer_constructor = lexer
        .split_once("    pub fn with_hooks(input: I, hooks: H) -> Self {")
        .expect("lexer should render its constructor")
        .1
        .split_once("\n\n    pub fn metadata()")
        .expect("lexer metadata accessor should follow its constructor")
        .0;
    let parser_constructor = parser
        .split_once("    pub fn with_hooks(input: CommonTokenStream<L>, hooks: H) -> Self {")
        .expect("parser should render its constructor")
        .1
        .split_once("\n\n    pub fn metadata()")
        .expect("parser metadata accessor should follow its constructor")
        .0;
    let pattern_cache = parser
        .split_once("        static PATTERN_DATA")
        .expect("parser should render its pattern recognizer cache")
        .1
        .split_once("        matcher.compile(")
        .expect("pattern cache should precede pattern compilation")
        .0;
    let recognizer_construction = format!(
        "Lexer::with_hooks\n    pub fn with_hooks(input: I, hooks: H) -> Self {{{lexer_constructor}\n\n\
             Parser::with_hooks\n    pub fn with_hooks(input: CommonTokenStream<L>, hooks: H) -> Self {{{parser_constructor}\n\n\
             Pattern recognizer cache\n        static PATTERN_DATA{pattern_cache}"
    );

    insta::assert_snapshot!(
        "generated_recognizers_reuse_cached_static_metadata",
        recognizer_construction
    );
}

#[test]
fn converts_names_to_rust_identifiers() {
    assert_eq!(module_name("ExprLexer"), "expr_lexer");
    assert_eq!(rust_function_name("sourceFile"), "source_file");
    assert_eq!(rust_const_name("LPAREN"), "LPAREN");
    assert_eq!(rust_const_name("Q_COLONCOLON"), "Q_COLONCOLON");
    assert_eq!(rust_const_name("LineStrExprStart"), "LINE_STR_EXPR_START");
    assert_eq!(rust_const_name("UnicodeClassLL"), "UNICODE_CLASS_LL");
    assert_eq!(rust_function_name("gen"), "r#gen");
    assert_eq!(rust_function_name("try"), "r#try");
    assert_eq!(rust_function_name("Self"), "self_");
    assert_eq!(rust_function_name("crate"), "crate_");
    assert_eq!(rust_function_name("super"), "super_");
    assert_eq!(rust_identifier("Self"), "Self_");
    assert_eq!(rust_identifier("type"), "r#type");
    assert!(is_rust_keyword("Self"));
}

#[test]
fn renders_structural_channel_and_mode_constants() {
    let data = LexerCodegenData {
        common: RecognizerCodegenData::default(),
        channel_names: Vec::new(),
        channel_numbers: BTreeMap::from([
            ("DEFAULT_TOKEN_CHANNEL".to_owned(), 0),
            ("DIRECTIVE".to_owned(), 3),
            ("HIDDEN".to_owned(), 1),
        ]),
        mode_names: Vec::new(),
        mode_numbers: BTreeMap::from([
            ("DEFAULT_MODE".to_owned(), 0),
            ("INTERPOLATION_FORMAT".to_owned(), 2),
        ]),
        lexer_atn_words: Vec::new(),
        lexer_atn: LexerAtn::new(0),
        lexer_dfa_words: Vec::new(),
    };

    insta::assert_snapshot!(
        "structural_channel_and_mode_constants",
        render_lexer_state_constants(&data)
    );
}

#[test]
fn renders_parser_rustdoc_with_entry_rule_methods() {
    let data = RecognizerCodegenData {
        rule_names: vec![
            "sourceFile".to_owned(),
            "declaration".to_owned(),
            "script".to_owned(),
            "try".to_owned(),
        ],
        ..RecognizerCodegenData::default()
    };
    let entry_rule_indices = vec![0, 2];

    let rendered = render_parser_rustdoc(
        &parser_public_rule_method_names(&data.rule_names),
        &entry_rule_indices,
    );

    assert!(rendered.contains("Likely parser entry-rule methods"));
    assert!(rendered.contains("/// - `source_file()`"));
    assert!(rendered.contains("/// - `script()`"));
    assert!(rendered.contains("All parser rule methods:"));
    assert!(rendered.contains("/// - `declaration()`"));
    assert!(rendered.contains("/// - `r#try()`"));
    assert!(rendered.contains("cannot"));
    assert!(rendered.contains("semantic choice"));
    assert!(rendered.contains("explicit `EOF`"));
    assert!(rendered.contains("no other rule calls"));
}

#[test]
fn infers_entry_rule_candidates_from_rule_call_graph() {
    let atn = entry_candidate_atn();

    assert_eq!(
        likely_parser_entry_rule_indices_from_atn(&atn, 4),
        vec![0, 2, 3]
    );
}

#[test]
fn generated_parser_rustdoc_is_attached_to_parser_type() {
    let rendered = render_parser("DemoParser", &minimal_parser_data()).expect("parser renders");

    assert!(rendered.contains(
            "/// Generated parser. Each grammar rule is exposed as a public method.\n///\n/// Pick an entry-rule method"
        ));
    assert!(rendered.contains("/// Likely parser entry-rule methods:\n/// - `s()`"));
    assert!(rendered.contains(
            "/// All parser rule methods:\n/// - `s()`\n#[derive(Debug)]\npub struct DemoParser<L, H = antlr4_runtime::NoSemanticHooks>"
        ));
}

#[test]
fn generated_parser_embeds_only_versioned_packed_atn_data() {
    let rendered = render_parser("TParser", &minimal_parser_data()).expect("parser should render");

    assert!(rendered.contains("static PARSER_ATN_DATA: &[u32]"));
    assert!(rendered.contains("static ATN_CELL: OnceLock<ParserAtn>"));
    assert!(rendered.contains("ParserAtn::from_static(PARSER_ATN_DATA)"));
    assert!(rendered.contains("generated parser ATN is incompatible with this runtime"));
    assert!(rendered.contains("pub fn parser_atn() -> &'static ParserAtn"));
    assert!(rendered.contains("fn parser_atn() -> &'static ParserAtn"));
    assert!(!rendered.contains("AtnDeserializer"));
    assert!(!rendered.contains("SerializedAtn"));
}

#[test]
fn parser_rule_method_names_reserve_recognizer_reuse_accessors() {
    let rule_names = vec![
        "tokenStream".to_owned(),
        "into_token_stream".to_owned(),
        "token_stream_rule".to_owned(),
        "reset".to_owned(),
        "setTokenStream".to_owned(),
        "clearDfa".to_owned(),
        "addErrorListener".to_owned(),
        "removeErrorListeners".to_owned(),
        "addParseListener".to_owned(),
        "removeParseListeners".to_owned(),
        "compileParseTreePattern".to_owned(),
        "regularRule".to_owned(),
    ];

    assert_eq!(
        parser_public_rule_method_names(&rule_names),
        [
            "token_stream_rule",
            "into_token_stream_rule",
            "token_stream_rule_2",
            "reset_rule",
            "set_token_stream_rule",
            "clear_dfa_rule",
            "add_error_listener_rule",
            "remove_error_listeners_rule",
            "add_parse_listener_rule",
            "remove_parse_listeners_rule",
            "compile_parse_tree_pattern_rule",
            "regular_rule"
        ]
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn generated_modules_start_with_file_level_header() {
    let lexer = render_lexer(
        "TLexer",
        &predicate_lexer_data(),
        false,
        SemUnknownPolicy::default(),
        &SemPatternFile::default(),
        false,
    )
    .expect("lexer module should render");
    let parser = render_parser("TParser", &minimal_parser_data()).expect("parser should render");

    let generated_module_header = generated_module_header();
    insta::assert_snapshot!(
        "generated_module_file_header",
        generated_module_header.replace(env!("CARGO_PKG_VERSION"), "<generator-version>")
    );
    assert!(generated_module_header.contains(concat!(
        "@generated by ",
        env!("CARGO_PKG_NAME"),
        " v",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(generated_module_header.contains(env!("CARGO_PKG_REPOSITORY")));

    for rendered in [&lexer, &parser] {
        assert!(rendered.starts_with(&generated_module_header));
        assert!(rendered[generated_module_header.len()..].starts_with("use antlr4_runtime::"));
        assert!(!rendered.contains("#!["));
        assert!(rendered.contains(GENERATED_MODULE_FOOTER));
        assert!(
            rendered.ends_with("pub use self::__antlr4_rust_generated::*;\n"),
            "generated module should end with exactly one trailing newline and no blank line at EOF"
        );
    }
}

fn compile_test_parser_rule(
    atn: &ParserAtn,
    rule_index: usize,
    inline_action_states: &BTreeSet<usize>,
) -> Option<GeneratedParserRule> {
    let decision_by_state = decision_by_state(atn);
    let action_states = BTreeSet::new();
    let generated_action_states = BTreeSet::new();
    let action_indices = BTreeMap::new();
    let predicate_coordinates = BTreeSet::new();
    let generated_predicate_coordinates = BTreeSet::new();
    let context = GeneratedParserCompileContext {
        atn,
        decision_by_state: &decision_by_state,
        rule_args: &[],
        inline_action_states,
        action_states: &action_states,
        generated_action_states: &generated_action_states,
        action_indices: &action_indices,
        predicate_coordinates: &predicate_coordinates,
        generated_predicate_coordinates: &generated_predicate_coordinates,
    };
    compile_generated_parser_rule(&context, rule_index)
}

fn finish_atn(builder: ParserAtnBuilder) -> ParserAtn {
    builder.finish().expect("valid packed parser ATN")
}

fn transition_atn(
    make_transition: impl FnOnce(&mut ParserAtnBuilder) -> ParserTransitionSpec,
) -> ParserAtn {
    let mut builder = ParserAtnBuilder::new(10);
    for _ in 0..10 {
        builder
            .add_state(AtnStateKind::Basic, Some(0))
            .expect("state");
    }
    builder
        .set_rule_to_start_state(vec![0, 0, 0])
        .expect("rule start states");
    builder
        .set_rule_to_stop_state(vec![9, 9, 9])
        .expect("rule stop states");
    let transition = make_transition(&mut builder);
    builder
        .add_transition(0, transition)
        .expect("test transition");
    finish_atn(builder)
}

fn only_transition(atn: &ParserAtn) -> ParserTransition<'_> {
    atn.state(0)
        .expect("transition source")
        .transitions()
        .first()
        .expect("test transition")
}

fn mt(token_type: i32, follow_state: usize) -> GeneratedParserStep {
    GeneratedParserStep::MatchToken {
        token_type,
        follow_state,
    }
}

fn mts(token_set: usize, intervals: Vec<(i32, i32)>, follow_state: usize) -> GeneratedParserStep {
    GeneratedParserStep::MatchSet {
        token_set: Some(token_set),
        intervals,
        follow_state,
    }
}

fn mnts(token_set: usize, intervals: Vec<(i32, i32)>, follow_state: usize) -> GeneratedParserStep {
    GeneratedParserStep::MatchNotSet {
        token_set: Some(token_set),
        intervals,
        follow_state,
    }
}

fn cr(rule_index: usize) -> GeneratedParserStep {
    GeneratedParserStep::CallRule {
        source_state: 100 + rule_index,
        rule_index,
        precedence: GeneratedRuleCallPrecedence::Literal(0),
    }
}

fn adaptive_loop(decision: usize) -> GeneratedParserStep {
    GeneratedParserStep::StarLoop {
        state: 1_000 + decision,
        decision,
        enter_alt: 1,
        exit_alt: 2,
        track_alt_number: false,
        allow_semantic_context: false,
        force_context: false,
        plus_loop: false,
        fast_path: None,
        body: vec![mt(2, 0)],
    }
}

fn adaptive_decision(decision: usize, alt_count: usize) -> GeneratedParserStep {
    GeneratedParserStep::Decision {
        state: 2_000 + decision,
        decision,
        track_alt_number: false,
        allow_semantic_context: false,
        force_context: false,
        fast_path: None,
        alts: (0..alt_count).map(|_| vec![mt(2, 0)]).collect(),
    }
}

fn left_recursive_rule(
    rule_index: usize,
    decision_cost: usize,
    operator_alt_count: usize,
) -> GeneratedParserRule {
    let mut steps = (2..decision_cost).map(adaptive_loop).collect::<Vec<_>>();
    steps.push(GeneratedParserStep::LeftRecursiveLoop {
        state: 3_000 + rule_index,
        decision: 0,
        enter_alt: 1,
        exit_alt: 2,
        rule_index,
        entry_state: rule_index * 2,
        body: vec![adaptive_decision(1, operator_alt_count)],
    });
    GeneratedParserRule {
        rule_index,
        entry_state: rule_index * 2,
        left_recursive: true,
        steps,
    }
}

fn expensive_ladder_rule(rule_index: usize, next: Option<usize>) -> GeneratedParserRule {
    let mut steps = Vec::new();
    if let Some(next) = next {
        steps.push(cr(next));
    }
    steps.push(adaptive_loop(rule_index * 2));
    steps.push(adaptive_loop(rule_index * 2 + 1));
    if next.is_none() {
        steps.push(mt(1, 0));
    }
    test_rule(rule_index, steps)
}

fn test_rule(rule_index: usize, steps: Vec<GeneratedParserStep>) -> GeneratedParserRule {
    GeneratedParserRule {
        rule_index,
        entry_state: rule_index * 2,
        left_recursive: false,
        steps,
    }
}

#[test]
fn compiles_linear_parser_rule_body() {
    let atn = linear_rule_atn();
    let body =
        compile_test_parser_rule(&atn, 0, &BTreeSet::new()).expect("linear rule should compile");

    assert_eq!(body.rule_index, 0);
    assert_eq!(body.entry_state, 0);
    assert_eq!(body.steps, [mt(1, 2), mt(TOKEN_EOF, 3)]);

    let rendered = render_generated_rule_dispatch(&[Some(body)], &[], &BTreeMap::new(), false);
    assert!(rendered.contains("match_token_recovering(1, 2, atn())"));
    assert!(rendered.contains("generated_diagnostics_checkpoint()"));
    assert!(rendered.contains("rollback_generated_tree(__generated_diagnostic_marker)"));
}

#[test]
fn compiles_block_decision_with_adaptive_prediction() {
    let atn = block_decision_atn();
    let body = compile_test_parser_rule(&atn, 0, &BTreeSet::new())
        .expect("block decision rule should compile");

    // The compiled decision step (fast-path arms + per-alt token matches) is one structural
    // snapshot instead of a hand-transcribed GeneratedParserStep literal.
    insta::assert_debug_snapshot!(
        "compiles_block_decision_with_adaptive_prediction",
        body.steps
    );

    let rendered =
        render_generated_rule_dispatch(&[Some(body.clone())], &[], &BTreeMap::new(), false);
    assert!(rendered.contains("parse_generated_rule_0"));
    assert!(rendered.contains("sync_decision(atn(), 1, !__ctx.has_matched_child(), false)"));
    assert!(rendered.contains("ll1_decision_prediction(atn(), 1)"));
    // Stage 1 is the SLL probe (no LL loop on the empty-context conflict);
    // stage 2 re-runs with the real context only when full context is needed.
    assert!(rendered.contains("adaptive_predict_stream_info_sll_probe(0, 0"));
    assert!(rendered.contains("adaptive_predict_stream_info_with_context(0, 0"));
    assert!(rendered.contains(
            "intern_prediction_context(self.base.rule_context_version(), self.base.prediction_context_return_states(atn()))"
        ));
    assert!(!rendered.contains("self.base.prediction_context(atn())"));

    let rendered_with_alt_numbers =
        render_generated_rule_dispatch(&[Some(body)], &[], &BTreeMap::new(), true);
    assert!(rendered_with_alt_numbers.contains("__ctx.set_alt_number(1);"));
    assert!(rendered_with_alt_numbers.contains("__ctx.set_alt_number(2);"));
}

#[test]
fn compiles_star_loop_with_adaptive_prediction() {
    let atn = star_loop_atn();
    let body =
        compile_test_parser_rule(&atn, 0, &BTreeSet::new()).expect("star loop rule should compile");

    insta::assert_debug_snapshot!("compiles_star_loop_with_adaptive_prediction", body.steps);

    let rendered = render_generated_rule_dispatch(&[Some(body)], &[], &BTreeMap::new(), false);
    assert!(rendered.contains("loop {"));
    // A `*` loop starts NOT iterated: its first sync is at the loop entry
    // (single-token deletion), so the iteration flag inits to `false`.
    assert!(rendered.contains("let mut __loop_iter_1 = false;"));
    assert!(
        rendered.contains("sync_decision(atn(), 1, !__ctx.has_matched_child(), __loop_iter_1)")
    );
    assert!(rendered.contains("__loop_iter_1 = true;"));
    assert!(rendered.contains("1 => {"));
    assert!(rendered.contains("2 => {"));
    assert!(rendered.contains("break;"));
    assert!(rendered.contains("ll1_decision_prediction(atn(), 1)"));
    assert!(rendered.contains("adaptive_predict_stream_info_sll_probe(0, 0"));
    assert!(rendered.contains("adaptive_predict_stream_info_with_context(0, 0"));
}

#[test]
fn compiles_plus_loop_back_with_adaptive_prediction() {
    let atn = plus_loop_atn();
    let body =
        compile_test_parser_rule(&atn, 0, &BTreeSet::new()).expect("plus loop rule should compile");

    insta::assert_debug_snapshot!(
        "compiles_plus_loop_back_with_adaptive_prediction",
        body.steps
    );

    let rendered = render_generated_rule_dispatch(&[Some(body)], &[], &BTreeMap::new(), false);
    // A `+` loop's mandatory first element is iteration 1, so the iteration
    // flag inits to `true`: its first loop-back sync recovers with multi-token
    // `consumeUntil`, matching ANTLR's PLUS_LOOP_BACK.
    assert!(rendered.contains("let mut __loop_iter_4 = true;"));
    assert!(
        rendered.contains("sync_decision(atn(), 4, !__ctx.has_matched_child(), __loop_iter_4)")
    );
}

#[test]
fn compiles_plus_block_body_decision_with_adaptive_prediction() {
    let atn = plus_block_decision_atn();
    let body = compile_test_parser_rule(&atn, 0, &BTreeSet::new())
        .expect("plus block decision rule should compile");

    // The plus-block body decision is repeated inside the loop step; snapshot the whole
    // steps vec so both the leading decision and the loop body it feeds are one target.
    insta::assert_debug_snapshot!(
        "compiles_plus_block_body_decision_with_adaptive_prediction",
        body.steps
    );
}

#[test]
fn compiles_left_recursive_parser_rule() {
    let atn = left_recursive_rule_atn();
    let body = compile_test_parser_rule(&atn, 0, &BTreeSet::new())
        .expect("left-recursive rule should compile");

    assert!(body.left_recursive);
    assert_eq!(body.rule_index, 0);
    assert_eq!(body.entry_state, 0);
    // The left-recursive loop (precedence step, nested decision, self CallRule) is one snapshot;
    // the left_recursive/rule_index/entry_state flags above stay as explicit invariants.
    insta::assert_debug_snapshot!("compiles_left_recursive_parser_rule", body.steps);

    let rendered = render_generated_rule_dispatch(&[Some(body)], &[], &BTreeMap::new(), false);
    assert!(rendered.contains("parse_generated_rule_0_precedence(precedence, allow_fallback)"));
    assert!(rendered.contains("push_new_recursion_context_with_previous(0isize, 0, &mut __ctx)"));
    assert!(rendered.contains("parse_rule_precedence_from_generated(0, 3)"));
    assert!(rendered.contains("precpred(_ctx, 2)"));
    assert!(
            rendered.contains(
                "let __prediction = match self.base.left_recursive_loop_enter_prediction(atn(), 2, __precedence)"
            )
        );
    assert!(rendered.contains("Some(true) => antlr4_runtime::ParserAtnPrediction"));
    assert!(
        rendered.contains("adaptive_predict_stream_info_with_context(0, __prediction_precedence")
    );
    assert!(rendered.contains(
            "Err(antlr4_runtime::ParserAtnSimulatorError::NoViableAlt { .. }) => antlr4_runtime::ParserAtnPrediction { alt: 2, requires_full_context: true, has_semantic_context: false, diagnostic: None }"
        ));
}

#[test]
fn drops_generated_rules_that_call_disabled_rules() {
    let mut rules = vec![
        Some(GeneratedParserRule {
            rule_index: 0,
            entry_state: 0,
            left_recursive: false,
            steps: vec![GeneratedParserStep::CallRule {
                source_state: 4,
                rule_index: 1,
                precedence: GeneratedRuleCallPrecedence::Literal(0),
            }],
        }),
        None,
        Some(GeneratedParserRule {
            rule_index: 2,
            entry_state: 10,
            left_recursive: false,
            steps: vec![mt(1, 0)],
        }),
    ];

    drop_rules_calling_disabled_rules(&mut rules);

    assert!(rules[0].is_none());
    assert!(rules[1].is_none());
    assert!(rules[2].is_some());
}

#[test]
fn generated_parent_keeps_interpreted_child_call() {
    let rules = vec![
        Some(GeneratedParserRule {
            rule_index: 0,
            entry_state: 0,
            left_recursive: false,
            steps: vec![GeneratedParserStep::CallRule {
                source_state: 4,
                rule_index: 1,
                precedence: GeneratedRuleCallPrecedence::Literal(0),
            }],
        }),
        None,
    ];

    let rendered = render_generated_rule_dispatch(&rules, &[true, false], &BTreeMap::new(), false);

    assert!(
        rendered.contains(
            "0 => Some(self.parse_generated_rule_0_dispatch(precedence, allow_fallback))"
        )
    );
    assert!(rendered.contains("self.parse_rule_precedence_from_generated(1, 0)"));
    assert!(!rendered.contains("parse_generated_rule_1_dispatch"));
}

#[test]
fn classifies_expensive_long_leading_call_chains_as_atn_preferred() {
    let mut rules = (0..ATN_PREFERRED_LEADING_CALL_CHAIN_MIN)
        .map(|rule_index| {
            let next = if rule_index + 1 == ATN_PREFERRED_LEADING_CALL_CHAIN_MIN {
                None
            } else {
                Some(rule_index + 1)
            };
            Some(expensive_ladder_rule(rule_index, next))
        })
        .collect::<Vec<_>>();

    assert_eq!(
        generated_atn_preferred_rule_calls(&rules, &[]),
        vec![true; ATN_PREFERRED_LEADING_CALL_CHAIN_MIN]
    );
    let required = BTreeSet::from([ATN_PREFERRED_LEADING_CALL_CHAIN_MIN - 1]);
    assert_eq!(
        generated_atn_preferred_rule_calls_excluding(
            &rules,
            &[],
            &generated_rule_callers_reaching(&rules, &required),
        ),
        vec![false; ATN_PREFERRED_LEADING_CALL_CHAIN_MIN],
        "portable-local owners and generated callers must stay on the generated path"
    );

    rules.truncate(ATN_PREFERRED_LEADING_CALL_CHAIN_MIN - 1);
    assert_eq!(
        generated_atn_preferred_rule_calls(&rules, &[]),
        vec![false; ATN_PREFERRED_LEADING_CALL_CHAIN_MIN - 1]
    );
}

#[test]
fn graph_reachability_traverses_backwards_from_every_target() {
    let mut graph = DiGraph::new();
    let nodes = (0..6)
        .map(|value| graph.add_node(value))
        .collect::<Vec<_>>();
    graph.add_edge(nodes[0], nodes[1], ());
    graph.add_edge(nodes[1], nodes[2], ());
    graph.add_edge(nodes[3], nodes[4], ());

    assert_eq!(
        graph_nodes_reaching(&graph, &BTreeSet::from([2, 4, 99])),
        BTreeSet::from([0, 1, 2, 3, 4, 99])
    );
}

#[test]
fn atn_preferred_rule_calls_reject_simple_operator_ladders() {
    let simple_rules = (0..ATN_PREFERRED_LEADING_CALL_CHAIN_MIN)
        .map(|rule_index| {
            let steps = if rule_index + 1 == ATN_PREFERRED_LEADING_CALL_CHAIN_MIN {
                vec![adaptive_loop(rule_index), mt(1, 0)]
            } else {
                vec![cr(rule_index + 1), adaptive_loop(rule_index)]
            };
            Some(test_rule(rule_index, steps))
        })
        .collect::<Vec<_>>();

    assert_eq!(
        generated_atn_preferred_rule_calls(&simple_rules, &[]),
        vec![false; ATN_PREFERRED_LEADING_CALL_CHAIN_MIN]
    );

    let expensive_rules = (0..ATN_PREFERRED_LEADING_CALL_CHAIN_MIN)
        .map(|rule_index| {
            let next = if rule_index + 1 == ATN_PREFERRED_LEADING_CALL_CHAIN_MIN {
                None
            } else {
                Some(rule_index + 1)
            };
            Some(expensive_ladder_rule(rule_index, next))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        generated_atn_preferred_rule_calls(&expensive_rules, &[]),
        vec![true; ATN_PREFERRED_LEADING_CALL_CHAIN_MIN]
    );
}

#[test]
fn adaptive_atn_preferred_rule_calls_select_expensive_wrapper_boundaries() {
    let rules = vec![
        Some(test_rule(0, {
            let mut steps = (100..108).map(adaptive_loop).collect::<Vec<_>>();
            steps.push(cr(1));
            steps
        })),
        Some(left_recursive_rule(
            1,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS,
        )),
        Some(left_recursive_rule(
            2,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST - 1,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS,
        )),
        Some(left_recursive_rule(
            3,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS - 1,
        )),
        Some(left_recursive_rule(
            4,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS,
        )),
    ];

    assert_eq!(
        generated_atn_preferred_rule_calls(&rules, &[]),
        vec![false; rules.len()],
        "left-recursive routing must remain separate from unconditional cascade routing"
    );
    let preferred = generated_adaptive_atn_preferred_rule_calls(&rules);
    let routing =
        generated_adaptive_atn_routing_excluding(&rules, &BTreeSet::new(), &BTreeSet::new());

    assert!(
        preferred[0],
        "an expensive wrapper should become the candidate boundary"
    );
    assert!(
        preferred[1],
        "an expensive LR seed must remain eligible for direct entry paths"
    );
    assert!(
        !preferred[2],
        "decision cost below the threshold should stay generated"
    );
    assert!(
        !preferred[3],
        "operator fan-out below the threshold should stay generated"
    );
    assert!(
        preferred[4],
        "an expensive LR rule without a wrapper should remain a candidate"
    );
    assert_eq!(routing.probe_candidate_rules[1], vec![0]);
    assert!(routing.probe_candidate_rules[4].is_empty());

    let force_generated = generated_rule_callers_reaching(&rules, &BTreeSet::from([1]));
    assert_eq!(
        generated_adaptive_atn_preferred_rule_calls_excluding(
            &rules,
            &force_generated,
            &BTreeSet::new(),
        ),
        vec![false, false, false, false, true]
    );
}

#[test]
fn adaptive_atn_retries_exclude_effectful_action_rules_and_callers() {
    let mut action_seed = left_recursive_rule(
        1,
        ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST,
        ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS,
    );
    action_seed.steps.push(GeneratedParserStep::Action {
        source_state: 9_001,
        rule_index: 1,
        action_index: Some(0),
    });
    let mut synthetic_action_seed = left_recursive_rule(
        2,
        ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST,
        ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS,
    );
    synthetic_action_seed
        .steps
        .push(GeneratedParserStep::Action {
            source_state: 9_002,
            rule_index: 2,
            action_index: Some(0),
        });
    let rules = vec![
        Some(test_rule(0, {
            let mut steps = (100..108).map(adaptive_loop).collect::<Vec<_>>();
            steps.push(cr(1));
            steps
        })),
        Some(action_seed),
        Some(synthetic_action_seed),
    ];
    let effectful_action_states = BTreeSet::from([9_001]);

    let routing = generated_adaptive_atn_routing_excluding(
        &rules,
        &BTreeSet::new(),
        &effectful_action_states,
    );

    assert_eq!(routing.candidates, [false, false, true]);
    assert!(
        routing.probe_candidate_rules[1].is_empty(),
        "an effectful action seed must not request a retry from its generated caller"
    );
}

#[test]
fn atn_preferred_rule_calls_propagate_through_expensive_wrappers() {
    let mut rules = Vec::new();
    rules.push(Some(test_rule(
        0,
        vec![mt(9, 0), adaptive_loop(100), adaptive_loop(101), cr(1)],
    )));
    rules.push(Some(test_rule(
        1,
        vec![mt(8, 0), adaptive_loop(102), adaptive_loop(103), cr(2)],
    )));
    for rule_index in 2..(2 + ATN_PREFERRED_LEADING_CALL_CHAIN_MIN) {
        let next = if rule_index + 1 == 2 + ATN_PREFERRED_LEADING_CALL_CHAIN_MIN {
            None
        } else {
            Some(rule_index + 1)
        };
        rules.push(Some(expensive_ladder_rule(rule_index, next)));
    }
    rules.push(Some(test_rule(10, vec![cr(2)])));

    let mut expected = vec![true; 2 + ATN_PREFERRED_LEADING_CALL_CHAIN_MIN];
    expected.push(false);
    assert_eq!(generated_atn_preferred_rule_calls(&rules, &[]), expected);
}

#[test]
fn renders_atn_preferred_generated_child_calls_as_interpreted_by_default() {
    let rules = (0..ATN_PREFERRED_LEADING_CALL_CHAIN_MIN)
        .map(|rule_index| {
            let next = if rule_index + 1 == ATN_PREFERRED_LEADING_CALL_CHAIN_MIN {
                None
            } else {
                Some(rule_index + 1)
            };
            Some(expensive_ladder_rule(rule_index, next))
        })
        .collect::<Vec<_>>();
    let direct_generated_rule_calls = vec![true; rules.len()];
    let rule_names = Vec::new();

    let rendered = render_generated_rule_dispatch_with_rule_names(
        &rules,
        &direct_generated_rule_calls,
        &rule_names,
        &BTreeMap::new(),
        true,
        false,
        None,
        None,
        DecisionRoutingRender::default(),
    );

    // ATN-preferred children route through `parse_rule_precedence_from_generated`:
    // the rule's generated dispatch arm is `generated_only()`-guarded, so in normal
    // mode the wrapper parses the child interpreted (optimization preserved) while
    // buffering its actions in position (correct ordering).
    assert!(rendered.contains("self.parse_rule_precedence_from_generated(1, 0)"));
    assert!(!rendered.contains("self.parse_interpreted_rule_precedence(1, 0)"));
}

#[test]
fn renders_atn_preferred_dispatch_only_for_generated_only_mode() {
    let mut rules = Vec::new();
    rules.push(Some(test_rule(
        0,
        vec![mt(9, 0), adaptive_loop(100), adaptive_loop(101), cr(2)],
    )));
    rules.push(Some(test_rule(1, vec![mt(1, 0)])));
    for rule_index in 2..(2 + ATN_PREFERRED_LEADING_CALL_CHAIN_MIN) {
        let next = if rule_index + 1 == 2 + ATN_PREFERRED_LEADING_CALL_CHAIN_MIN {
            None
        } else {
            Some(rule_index + 1)
        };
        rules.push(Some(expensive_ladder_rule(rule_index, next)));
    }
    let direct_generated_rule_calls = vec![true; rules.len()];
    let rule_names = Vec::new();

    let rendered = render_generated_rule_dispatch_with_rule_names(
        &rules,
        &direct_generated_rule_calls,
        &rule_names,
        &BTreeMap::new(),
        true,
        false,
        None,
        None,
        DecisionRoutingRender::default(),
    );

    // A configured depth cap overrides the ATN preference: only generated
    // bodies enforce the bound, so the guard admits either trigger.
    assert!(rendered.contains(
            "0 if self.generated_only() || self.base.has_rule_depth_cap() || self.base.has_parse_listeners() => Some(self.parse_generated_rule_0_dispatch(precedence, allow_fallback))"
        ));
    assert!(
        !rendered.contains(
            "0 => Some(self.parse_generated_rule_0_dispatch(precedence, allow_fallback))"
        )
    );
    // The ATN-preferred child call routes through the buffering wrapper.
    assert!(rendered.contains("self.parse_rule_precedence_from_generated(2, 0)"));
    assert!(!rendered.contains("self.parse_interpreted_rule_precedence(2, 0)"));
}

#[test]
fn renders_adaptive_atn_preference_after_prediction_becomes_expensive() {
    let rules = vec![
        Some(test_rule(0, {
            let mut steps = (100..108).map(adaptive_loop).collect::<Vec<_>>();
            steps.push(cr(1));
            steps
        })),
        Some(left_recursive_rule(
            1,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST,
            ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS,
        )),
    ];

    let rendered =
        render_generated_rule_dispatch(&rules, &vec![true; rules.len()], &BTreeMap::new(), false);

    assert!(rendered.contains(
            "0 if self.generated_only() || self.base.has_rule_depth_cap() || self.base.has_parse_listeners() || self.base.observes_parser_decisions() => Some(self.parse_generated_rule_0_dispatch(precedence, allow_fallback))"
        ));
    assert!(rendered.contains(
            "0 if !self.adaptive_atn_preferred_rules[0] => Some(self.parse_generated_rule_0_adaptive_dispatch(precedence, allow_fallback, None))"
        ));
    assert!(rendered.contains(
            "ParserAtnSimulator::adaptive_prediction_delta_is_expensive(self.adaptive_atn_preference_starts[0], __adaptive_after)"
        ));
    assert!(rendered.contains(
        "self.base.number_of_syntax_errors() == self.adaptive_atn_syntax_error_starts[0]"
    ));
    assert!(rendered.contains("return Err(GeneratedRuleError::AdaptiveRetry);"));
    assert!(rendered.contains(
            "return self.parse_rule_precedence_from_generated(0, precedence).map_err(GeneratedRuleError::Interpreted);"
        ));
    assert!(rendered.contains("self.parse_generated_rule_1_adaptive_probe_dispatch(0, false)"));
    assert!(rendered.contains(
            "1 if !self.adaptive_atn_preferred_rules[1] => Some(self.parse_generated_rule_1_adaptive_dispatch(precedence, allow_fallback, None))"
        ));
    assert!(rendered.contains(
            "ParserAtnSimulator::adaptive_prediction_delta_is_decisive(self.adaptive_atn_preference_starts[0], __adaptive_after)"
        ));
    assert!(
        rendered.contains("if __adaptive_expensive {")
            && rendered.contains("self.adaptive_atn_retry_slot = Some(0);"),
        "an expensive outermost candidate must retry on the same invocation"
    );
    assert!(
        !rendered.contains("if __adaptive_expensive && !__adaptive_outermost"),
        "outermost candidates must not defer routing to parser reuse"
    );
}

#[test]
fn renders_bare_left_recursive_seed_retry_on_same_invocation() {
    let rules = vec![Some(left_recursive_rule(
        0,
        ATN_PREFERRED_LEFT_RECURSIVE_MIN_DECISION_COST,
        ATN_PREFERRED_LEFT_RECURSIVE_MIN_OPERATOR_ALTS,
    ))];

    let rendered = render_generated_rule_dispatch(&rules, &[true], &BTreeMap::new(), false);

    assert!(rendered.contains(
            "0 if !self.adaptive_atn_preferred_rules[0] => Some(self.parse_generated_rule_0_adaptive_dispatch(precedence, allow_fallback, None))"
        ));
    assert!(
        rendered.contains("if __adaptive_expensive {")
            && rendered.contains("self.adaptive_atn_retry_slot = Some(0);")
            && rendered.contains("__result = Err(GeneratedRuleError::AdaptiveRetry);"),
        "a bare seed must replay through the interpreter as soon as measured work is expensive"
    );
    assert!(!rendered.contains("_adaptive_probe_dispatch"));
    assert!(!rendered.contains("if __adaptive_expensive && !__adaptive_outermost"));
}

#[test]
fn embedded_rules_never_use_atn_preferred_fallback() {
    let rules = (0..ATN_PREFERRED_LEADING_CALL_CHAIN_MIN)
        .map(|rule_index| {
            let next = if rule_index + 1 == ATN_PREFERRED_LEADING_CALL_CHAIN_MIN {
                None
            } else {
                Some(rule_index + 1)
            };
            Some(expensive_ladder_rule(rule_index, next))
        })
        .collect::<Vec<_>>();
    let direct_generated_rule_calls = vec![true; rules.len()];
    let adaptive_decisions = BTreeSet::new();
    let complete_ll1_dispatches = BTreeMap::new();
    let predicates = BTreeMap::new();
    let rule_has_attrs = vec![false; rules.len()];
    let init_entry = BTreeMap::new();
    let after = BTreeMap::new();
    let call_args = BTreeMap::new();
    let rule_arg0 = vec![None; rules.len()];

    let rendered = render_generated_rule_dispatch_with_rule_names(
        &rules,
        &direct_generated_rule_calls,
        &[],
        &BTreeMap::new(),
        true,
        false,
        Some(EmbeddedStepRender {
            force_adaptive: false,
            adaptive_decisions: &adaptive_decisions,
            complete_ll1_dispatches: &complete_ll1_dispatches,
            predicates: &predicates,
            rule_has_attrs: &rule_has_attrs,
            init_entry: &init_entry,
            after: &after,
            call_args: &call_args,
            rule_arg0: &rule_arg0,
        }),
        None,
        DecisionRoutingRender::default(),
    );

    assert!(!rendered.contains("if self.generated_only()"));
    assert!(
        rendered.contains(
            "0 => Some(self.parse_generated_rule_0_dispatch(precedence, allow_fallback))"
        )
    );
    assert!(rendered.contains("self.parse_generated_rule_1_dispatch(0, false)"));
    assert!(!rendered.contains("self.parse_rule_precedence_from_generated(1, 0)"));
}

#[test]
fn compiles_token_set_transitions() {
    let empty_states = BTreeSet::new();
    let empty_coords = BTreeSet::new();
    let action_states = ActionStateSets {
        all: &empty_states,
        generated: &empty_states,
        inline: &empty_states,
        indices: &BTreeMap::new(),
    };
    let predicate_coords = PredicateCoordinateSets {
        all: &empty_coords,
        generated: &empty_coords,
    };
    let compile = |transition| {
        compile_generated_parser_transition(3, &[], transition, action_states, predicate_coords)
    };

    let range_atn = transition_atn(|_| ParserTransitionSpec::Range {
        target: 7,
        start: 2,
        stop: 4,
    });
    let set_atn = transition_atn(|builder| {
        let set = builder.add_interval_set([(1, 1), (5, 6)]).expect("set");
        ParserTransitionSpec::Set { target: 8, set }
    });
    let not_set_atn = transition_atn(|builder| {
        let set = builder.add_interval_set([(1, 1)]).expect("set");
        ParserTransitionSpec::NotSet { target: 9, set }
    });
    let dense_ranges = (1..=256)
        .step_by(2)
        .map(|token| (token, token))
        .collect::<Vec<_>>();
    let dense_set_atn = transition_atn(|builder| {
        let set = builder
            .add_interval_set(dense_ranges.iter().copied())
            .expect("dense set");
        ParserTransitionSpec::Set { target: 8, set }
    });

    // Range, sparse set, complement, and dense (bitset) set transitions all compile through the
    // same call; one snapshot of the four labelled outcomes replaces four ~15-line asserts.
    let compiled = [
        ("range", only_transition(&range_atn)),
        ("set", only_transition(&set_atn)),
        ("not_set", only_transition(&not_set_atn)),
        ("dense_set", only_transition(&dense_set_atn)),
    ]
    .map(|(label, transition)| (label, compile(transition)));
    insta::assert_debug_snapshot!("compiles_token_set_transitions", compiled);
}

#[test]
fn compiles_generated_action_transitions_only_for_allowed_states() {
    let action_atn = transition_atn(|_| ParserTransitionSpec::Action {
        target: 8,
        rule_index: 2,
        action_index: Some(0),
        context_dependent: false,
    });
    let action = only_transition(&action_atn);
    assert_eq!(
        compile_generated_parser_transition(
            4,
            &[],
            action,
            ActionStateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
                inline: &BTreeSet::new(),
                indices: &BTreeMap::new(),
            },
            PredicateCoordinateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
            }
        ),
        None
    );

    let mut generated_action_states = BTreeSet::new();
    generated_action_states.insert(4);
    assert_eq!(
        compile_generated_parser_transition(
            4,
            &[],
            action,
            ActionStateSets {
                all: &BTreeSet::new(),
                generated: &generated_action_states,
                inline: &BTreeSet::new(),
                indices: &BTreeMap::new(),
            },
            PredicateCoordinateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
            }
        ),
        Some((
            Some(GeneratedParserStep::Action {
                source_state: 4,
                rule_index: 2,
                action_index: Some(0),
            }),
            8
        ))
    );
}

#[test]
fn compiles_rule_call_precedence_from_rule_args() {
    let rule_atn = transition_atn(|_| ParserTransitionSpec::Rule {
        target: 1,
        rule_index: 2,
        follow_state: 8,
        precedence: 0,
    });
    let rule = only_transition(&rule_atn);

    assert_eq!(
        compile_generated_parser_transition(
            4,
            &[(4, 2, RuleArgTemplate::Literal(6))],
            rule,
            ActionStateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
                inline: &BTreeSet::new(),
                indices: &BTreeMap::new(),
            },
            PredicateCoordinateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
            }
        ),
        Some((
            Some(GeneratedParserStep::CallRule {
                source_state: 4,
                rule_index: 2,
                precedence: GeneratedRuleCallPrecedence::Literal(6),
            }),
            8
        ))
    );

    assert_eq!(
        compile_generated_parser_transition(
            4,
            &[(4, 2, RuleArgTemplate::InheritLocal)],
            rule,
            ActionStateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
                inline: &BTreeSet::new(),
                indices: &BTreeMap::new(),
            },
            PredicateCoordinateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
            }
        ),
        Some((
            Some(GeneratedParserStep::CallRule {
                source_state: 4,
                rule_index: 2,
                precedence: GeneratedRuleCallPrecedence::InheritLocal,
            }),
            8
        ))
    );
}

#[test]
fn parses_boolean_literal_rule_arguments() {
    let data = parser_fixture_data("boolean-rule-arguments/T.g4");
    let args = structural_parser_rule_args(&data)
        .expect("structural rule-call arguments should resolve")
        .into_iter()
        .map(|(_, rule_index, value)| (rule_index, value))
        .collect::<Vec<_>>();
    assert_eq!(
        args,
        [
            (1, RuleArgTemplate::Literal(1)),
            (1, RuleArgTemplate::Literal(0)),
        ]
    );
}

#[test]
fn rejects_unsupported_rule_argument_expressions() {
    let data = parser_fixture_data("unsupported-rule-argument/T.g4");
    let error = structural_parser_rule_args(&data)
        .expect_err("unsupported expressions must not be silently omitted");

    insta::assert_snapshot!(
        error,
        @"unsupported parser rule argument expression `1 + 2` for rule `child`; use an integer/boolean literal or forward the caller's first declared argument"
    );
}

#[test]
fn compiles_synthetic_noop_action_transitions_as_epsilon() {
    let action_atn = transition_atn(|_| ParserTransitionSpec::Action {
        target: 8,
        rule_index: 2,
        action_index: None,
        context_dependent: false,
    });
    let action = only_transition(&action_atn);
    assert_eq!(
        compile_generated_parser_transition(
            4,
            &[],
            action,
            ActionStateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
                inline: &BTreeSet::new(),
                indices: &BTreeMap::new(),
            },
            PredicateCoordinateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
            }
        ),
        Some((None, 8))
    );
}

#[test]
fn rejects_known_non_inline_noop_action_transitions() {
    let action_atn = transition_atn(|_| ParserTransitionSpec::Action {
        target: 8,
        rule_index: 2,
        action_index: None,
        context_dependent: false,
    });
    let action = only_transition(&action_atn);
    let mut action_states = BTreeSet::new();
    action_states.insert(4);
    assert_eq!(
        compile_generated_parser_transition(
            4,
            &[],
            action,
            ActionStateSets {
                all: &action_states,
                generated: &BTreeSet::new(),
                inline: &BTreeSet::new(),
                indices: &BTreeMap::new(),
            },
            PredicateCoordinateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
            }
        ),
        None
    );
}

#[test]
fn compiles_parser_predicates_as_viable_when_no_metadata_is_active() {
    let predicate_atn = transition_atn(|_| ParserTransitionSpec::Predicate {
        target: 8,
        rule_index: 2,
        pred_index: 1,
        context_dependent: false,
    });
    let predicate = only_transition(&predicate_atn);

    assert_eq!(
        compile_generated_parser_transition(
            4,
            &[],
            predicate,
            ActionStateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
                inline: &BTreeSet::new(),
                indices: &BTreeMap::new(),
            },
            PredicateCoordinateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
            }
        ),
        Some((None, 8))
    );
}

#[test]
fn compiles_generated_parser_predicate_transitions() {
    let predicate_atn = transition_atn(|_| ParserTransitionSpec::Predicate {
        target: 8,
        rule_index: 2,
        pred_index: 1,
        context_dependent: false,
    });
    let predicate = only_transition(&predicate_atn);
    let mut predicates = BTreeSet::new();
    predicates.insert((2, 1));
    let generated_predicates = predicates.clone();

    assert_eq!(
        compile_generated_parser_transition(
            4,
            &[],
            predicate,
            ActionStateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
                inline: &BTreeSet::new(),
                indices: &BTreeMap::new(),
            },
            PredicateCoordinateSets {
                all: &predicates,
                generated: &generated_predicates,
            }
        ),
        Some((
            Some(GeneratedParserStep::Predicate {
                rule_index: 2,
                pred_index: 1,
            }),
            8
        ))
    );
}

#[test]
fn renders_fail_option_parser_predicate_error() {
    let mut rendered = String::new();
    render_generated_step(
        &mut rendered,
        &GeneratedParserStep::Predicate {
            rule_index: 2,
            pred_index: 1,
        },
        0,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: None,
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &BTreeMap::new(),
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls: &[],
            atn_preferred_rule_calls: &[],
            adaptive_atn_preferred_rule_slots: &[],
            adaptive_atn_probe_rule_slots: &[],
        },
    );

    // A single predicate step renders into `rendered`; snapshot the whole emitted fragment
    // rather than probing for three substrings within it.
    insta::assert_snapshot!("renders_fail_option_parser_predicate_error", rendered);
}

fn render_call_rule_step(
    direct_generated_rule_calls: &[bool],
    atn_preferred_rule_calls: &[bool],
    adaptive_atn_preferred_rule_slots: &[Option<usize>],
) -> String {
    let mut rendered = String::new();
    render_generated_step(
        &mut rendered,
        &GeneratedParserStep::CallRule {
            source_state: 4,
            rule_index: 1,
            precedence: GeneratedRuleCallPrecedence::Literal(0),
        },
        2,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: None,
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &BTreeMap::new(),
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls,
            atn_preferred_rule_calls,
            adaptive_atn_preferred_rule_slots,
            adaptive_atn_probe_rule_slots: &[],
        },
    );
    rendered
}

#[test]
fn atn_preferred_child_with_after_action_routes_through_dispatch_wrapper() {
    // An ATN-preferred child that carries an `@after` action
    // (direct_generated_rule_calls[1] == false) must go through
    // `parse_rule_precedence_from_generated`, which preserves interpreted routing
    // (the rule's generated dispatch arm is guarded by `generated_only()`) while
    // BUFFERING the child's body actions and `@after` in position.
    let rendered = render_call_rule_step(&[true, false], &[true, true], &[]);

    assert!(rendered.contains("self.parse_rule_precedence_from_generated(1, 0)"));
    assert!(!rendered.contains("self.parse_interpreted_rule_precedence(1, 0)"));
}

#[test]
fn atn_preferred_child_without_after_also_routes_through_dispatch_wrapper() {
    // An ATN-preferred child WITHOUT `@after` (direct_generated_rule_calls[1] ==
    // true) must ALSO route through `parse_rule_precedence_from_generated`, not the
    // bare `parse_interpreted_rule_precedence`: the bare interpreted call runs the
    // child's body actions immediately, which reorders them before the generated
    // parent's buffered actions. The wrapper buffers them in position instead while
    // still parsing the child interpreted (the dispatch arm is `generated_only()`
    // guarded).
    let rendered = render_call_rule_step(&[true, true], &[true, true], &[]);

    assert!(rendered.contains("self.parse_rule_precedence_from_generated(1, 0)"));
    assert!(!rendered.contains("self.parse_interpreted_rule_precedence(1, 0)"));
}

#[test]
fn adaptive_atn_preferred_child_starts_with_direct_generated_dispatch() {
    let rendered = render_call_rule_step(&[true, true], &[], &[None, Some(0)]);

    assert!(rendered.contains(
            "if self.adaptive_atn_preferred_rules[0] { self.parse_rule_precedence_from_generated(1, 0) } else { self.parse_generated_rule_1_adaptive_dispatch(0, false, Some(4isize)).map_err(GeneratedRuleError::into_error) }"
        ));
}

#[test]
fn rejects_known_parser_predicates_without_generated_metadata() {
    let predicate_atn = transition_atn(|_| ParserTransitionSpec::Predicate {
        target: 8,
        rule_index: 2,
        pred_index: 1,
        context_dependent: false,
    });
    let predicate = only_transition(&predicate_atn);
    let mut predicates = BTreeSet::new();
    predicates.insert((2, 1));

    assert_eq!(
        compile_generated_parser_transition(
            4,
            &[],
            predicate,
            ActionStateSets {
                all: &BTreeSet::new(),
                generated: &BTreeSet::new(),
                inline: &BTreeSet::new(),
                indices: &BTreeMap::new(),
            },
            PredicateCoordinateSets {
                all: &predicates,
                generated: &BTreeSet::new(),
            }
        ),
        None
    );
}

#[test]
fn parse_rule_fallback_runs_parser_actions() {
    let rule_args = [(4, 2, RuleArgTemplate::Literal(17))];
    let action_indices = [(5, 0)];
    let fallback = render_parser_parse_rule_fallback(ParserFallbackRender {
        track_alt_numbers: false,
        track_context_alt_numbers: false,
        rule_args: &rule_args,
        action_indices: &action_indices,
        has_action_dispatch: true,
        has_predicate_dispatch: false,
        unknown_policy_literal: None,
    });

    assert!(fallback.contains(
        "parse_atn_rule_with_runtime_options_and_precedence(atn(), rule_index, precedence"
    ));
    assert!(fallback.contains(
            "rule_args: &[antlr4_runtime::ParserRuleArg { source_state: 4, rule_index: 2, value: 17, inherit_local: false }]"
        ));
    assert!(fallback.contains("for action in actions { self.run_action(action, tree); }"));
    assert!(fallback.contains("Ok(tree)"));
}

#[test]
fn parser_action_dispatch_falls_back_to_semantic_hook() {
    let method = render_parser_action_method(true, &BTreeSet::new());

    assert!(method.contains("fn run_action"));
    assert!(method.contains("self.base.parser_action_hook(action, tree)"));
}

#[test]
fn parser_action_assume_override_gets_explicit_noop_arm() {
    // An `assume-*` action override drops its translated arm, but must NOT
    // fall through to the `parser_action_hook` catch-all — that would fail
    // loud under the Error policy (NoSemanticHooks) or run a user side
    // effect for a coordinate the manifest reports as a no-op fallback. It
    // gets an explicit empty arm instead. A `hook`/`error` override is not
    // in this set, so it still falls through to the hook.
    let mut assume_noop = BTreeSet::new();
    assume_noop.insert(7_usize);
    let method = render_parser_action_method(true, &assume_noop);

    // The assume-* state has its own empty arm, placed before the catch-all.
    let noop_at = method
        .find("7 => {}")
        .expect("assume-* action state gets an explicit no-op arm");
    let hook_at = method
        .find("self.base.parser_action_hook(action, tree)")
        .expect("the hook catch-all is still emitted for hook/unknown states");
    assert!(
        noop_at < hook_at,
        "the assume-* no-op arm must precede the hook catch-all"
    );
}

#[test]
fn parser_st_actions_do_not_emit_replay_machinery() {
    let rendered = render_parser("TParser", &minimal_parser_data()).expect("parser should render");

    for removed in [
        format!("{}{}", "Generated", "Action"),
        format!("{}{}", "generated", "_actions"),
        format!("{}{}", "Member", "Snapshot"),
        format!("{}{}", "run_after", "_actions"),
        format!("{}{}", "int_members", "_checkpoint"),
        format!("{}{}", "restore_int", "_members"),
        format!("{}{}", "CTX_ROOTED", "_ACTION_STATES"),
    ] {
        assert!(
            !rendered.contains(&removed),
            "removed parser replay machinery leaked into generated module: {removed}"
        );
    }
}

#[test]
fn embedded_init_action_runs_at_rule_entry() {
    let data = parser_fixture_data("embedded-init/T.g4");
    let rendered = render_parser_with_options(
        "TParser",
        &data,
        ParserRenderOptions {
            embedded: true,
            ..ParserRenderOptions::default()
        },
    )
    .expect("embedded parser should render");

    let start_at = rendered
        .find("let __rule_start")
        .expect("generated rule captures rule start");
    let init_at = rendered
        .find("println!(\"init\");")
        .expect("embedded @init body is emitted");
    let body_at = rendered
        .find("let mut __consumed_eof")
        .expect("generated rule body follows entry setup");

    assert!(
        start_at < init_at && init_at < body_at,
        "embedded @init must run after rule entry setup and before the rule body"
    );
}

#[test]
fn embedded_left_recursive_actions_resolve_deleted_leading_labels() {
    let data = parser_fixture_data("left-recursive-labels/T.g4");
    let model = structural_embedded_model(&data, false).expect("structural model should resolve");
    insta::assert_debug_snapshot!("left_recursive_label_alternatives", model.rules[1].alts);

    let embedded =
        build_embedded_parser_data(&data, "TParser", "T", ParserRenderOptions::default())
            .expect("embedded actions should resolve deleted left-recursive labels");
    insta::assert_debug_snapshot!("left_recursive_label_actions", embedded.inline_actions);
}

#[test]
fn embedded_listener_forwards_error_nodes() {
    let rendered = render_parser_with_options(
        "TParser",
        &minimal_parser_data(),
        ParserRenderOptions {
            embedded: true,
            ..ParserRenderOptions::default()
        },
    )
    .expect("embedded parser should render");

    assert!(rendered.contains("ErrorNodeView as RuntimeErrorNode"));
    assert!(rendered.contains("pub struct ErrorNode<'a>"));
    assert!(
        rendered.contains("fn visit_error_node(&mut self, _node: &ErrorNode) -> Result<(), E>")
    );
    assert!(rendered.contains("listener.visit_error_node(&ErrorNode::new("));
}

#[test]
fn embedded_contexts_delegate_stored_invocation_states_to_the_runtime_api() {
    let rendered = render_parser_with_options(
        "TParser",
        &parser_fixture_data("left-recursive-labels/T.g4"),
        ParserRenderOptions {
            embedded: true,
            ..ParserRenderOptions::default()
        },
    )
    .expect("embedded parser should render");

    assert!(rendered.contains("invocation_states: Vec<isize>"));
    assert!(rendered.contains("antlr4_runtime::__antlr4_rust_context!"));
    assert!(!rendered.contains("__invocation_states: Option<Vec<isize>>"));
    assert!(!rendered.contains("Self::__from_node_with_invocation_states(node, None)"));
    assert!(rendered.contains("::__from_child_node(node, self.__invocation_states.as_deref())"));
    assert!(rendered.contains("::__from_listener_node(context, invocation_states.as_deref())"));
    assert!(rendered.contains("pub fn walk_with_invocation_states"));
    assert!(
        !rendered.contains("node.invocation_states().collect"),
        "stored contexts must leave the derivable invocation-state chain lazy"
    );
    assert!(
        !rendered.contains("__GeneratedRuleContext::Active { .. } => Vec::new()"),
        "active contexts must preserve their invoking-state chain"
    );
}

#[test]
fn attrless_contexts_skip_generated_attrs_lookup() {
    let attrless = render_parser("TParser", &minimal_parser_data()).expect("parser should render");
    assert!(
        !attrless.contains("generated_attrs::<__RuleAttrs0>"),
        "attr-less context construction must not perform an Any downcast"
    );
    assert!(!attrless.contains("pub struct __RuleAttrs0"));

    let attributed = render_parser_with_options(
        "TParser",
        &parser_fixture_data("left-recursive-labels/T.g4"),
        ParserRenderOptions {
            embedded: true,
            ..ParserRenderOptions::default()
        },
    )
    .expect("embedded parser should render");
    assert!(attributed.contains("pub struct __RuleAttrs1"));
    assert!(
        attributed.contains("attributes: {\n            __RuleAttrs1 {\n                v: i32,")
    );
    assert!(!attributed.contains("live_attrs.downcast_ref::<__RuleAttrs1>()"));
    assert!(
        attributed.contains("T::__from_active(context, None, invocation_states, storage, tokens)"),
        "native embedded actions must retain the original active-context helper"
    );
    assert!(
        attributed.contains("T::__from_active(\n        context,\n        Some(live_attrs),"),
        "compatibility lowering must have a live-attribute helper"
    );
}

#[test]
fn context_surface_names_disambiguate_normalized_label_collisions() {
    let data = parser_fixture_data("context-name-collision/T.g4");
    let model = structural_embedded_model(&data, false).expect("structural model should resolve");
    let names = context_surface_names(&model);

    let every_rule = model
        .rules
        .iter()
        .position(|rule| rule.name == "everyRule")
        .expect("everyRule fixture rule");
    assert_ne!(names.rules[every_rule].listener_method, "every_rule");
    let stored_tree = model
        .rules
        .iter()
        .position(|rule| rule.name == "storedTree")
        .expect("storedTree fixture rule");
    assert_ne!(names.rules[stored_tree].context_type, "StoredTreeContext");
    let validated_tree = model
        .rules
        .iter()
        .position(|rule| rule.name == "validatedTree")
        .expect("validatedTree fixture rule");
    assert_ne!(
        names.rules[validated_tree].context_type,
        "ValidatedTreeContext"
    );

    insta::assert_debug_snapshot!("context_surface_name_collision", names);
}

#[test]
fn antlr4rust_compat_accessors_reserve_legacy_method_names() {
    let model = embedded::EmbeddedModel {
        rules: vec![
            embedded::RuleModel {
                name: "item".to_owned(),
                ..embedded::RuleModel::default()
            },
            embedded::RuleModel {
                name: "item_all".to_owned(),
                ..embedded::RuleModel::default()
            },
            embedded::RuleModel {
                name: "text".to_owned(),
                ..embedded::RuleModel::default()
            },
            embedded::RuleModel {
                name: "required".to_owned(),
                ..embedded::RuleModel::default()
            },
            embedded::RuleModel {
                name: "type".to_owned(),
                ..embedded::RuleModel::default()
            },
            embedded::RuleModel {
                name: "self".to_owned(),
                ..embedded::RuleModel::default()
            },
        ],
        parser_members: embedded::MembersModel::default(),
    };
    let mut child_cardinalities = BTreeMap::from([
        (
            "item".to_owned(),
            embedded::ChildCardinality { min: 0, max: None },
        ),
        ("item_all".to_owned(), embedded::ChildCardinality::ONE),
        (
            "text".to_owned(),
            embedded::ChildCardinality {
                min: 0,
                max: Some(1),
            },
        ),
        ("required".to_owned(), embedded::ChildCardinality::ONE),
        (
            "type".to_owned(),
            embedded::ChildCardinality {
                min: 0,
                max: Some(1),
            },
        ),
        (
            "self".to_owned(),
            embedded::ChildCardinality {
                min: 0,
                max: Some(1),
            },
        ),
        ("ID".to_owned(), embedded::ChildCardinality::ONE),
        (
            "IDS".to_owned(),
            embedded::ChildCardinality { min: 1, max: None },
        ),
    ]);
    let token_accessors = vec![("ID".to_owned(), 1), ("IDS".to_owned(), 2)];
    let collision = antlr4rust_compat_method_names(
        "CollisionContext",
        &model,
        &token_accessors,
        &child_cardinalities,
    )
    .expect_err("ambiguous legacy getters must fail generation");
    insta::assert_snapshot!("antlr4rust_compat_accessor_collision", collision);
    child_cardinalities.remove("item_all");
    let compatibility_methods = antlr4rust_compat_method_names(
        "CompatContext",
        &model,
        &token_accessors,
        &child_cardinalities,
    )
    .expect("collision-free legacy getters");
    assert!(compatibility_methods.contains("r#type"));
    assert!(compatibility_methods.contains("self_"));
    let common_methods = context_common_method_names(&compatibility_methods);
    assert_eq!(common_methods.text, "context_text");
    let mut used_methods = compatibility_methods;
    let native_item_all = allocate_context_method(
        "item_all".to_owned(),
        "item_all_rule_child",
        &mut used_methods,
    );
    assert_ne!(native_item_all, "item_all");

    let mut rendered = String::new();
    let mut emitted_methods = BTreeSet::new();
    render_antlr4rust_rule_all_accessor(
        &mut rendered,
        "item",
        "item_children",
        "ItemContext",
        embedded::ANTLR4RUST_CONTEXT_WRAPPER,
        &mut emitted_methods,
    );
    render_antlr4rust_single_rule_accessor(
        &mut rendered,
        Antlr4RustSingleRuleAccessorRender {
            source_name: "text",
            native_method: "text_rule_child",
            child_view: "TextContext",
            required: false,
            context_wrapper: embedded::ANTLR4RUST_CONTEXT_WRAPPER,
        },
        &mut emitted_methods,
    );
    render_antlr4rust_single_rule_accessor(
        &mut rendered,
        Antlr4RustSingleRuleAccessorRender {
            source_name: "required",
            native_method: "required_rule_child",
            child_view: "RequiredContext",
            required: true,
            context_wrapper: embedded::ANTLR4RUST_CONTEXT_WRAPPER,
        },
        &mut emitted_methods,
    );
    render_antlr4rust_single_rule_accessor(
        &mut rendered,
        Antlr4RustSingleRuleAccessorRender {
            source_name: "type",
            native_method: "type_rule_child",
            child_view: "TypeContext",
            required: false,
            context_wrapper: embedded::ANTLR4RUST_CONTEXT_WRAPPER,
        },
        &mut emitted_methods,
    );
    render_antlr4rust_single_rule_accessor(
        &mut rendered,
        Antlr4RustSingleRuleAccessorRender {
            source_name: "self",
            native_method: "self_rule_child",
            child_view: "SelfContext",
            required: false,
            context_wrapper: embedded::ANTLR4RUST_CONTEXT_WRAPPER,
        },
        &mut emitted_methods,
    );
    render_antlr4rust_single_token_accessor(
        &mut rendered,
        "ID",
        "id_token",
        true,
        &mut emitted_methods,
    );
    render_antlr4rust_token_all_accessor(&mut rendered, "IDS", "ids_tokens", &mut emitted_methods);

    insta::assert_snapshot!("antlr4rust_compat_reserved_accessors", rendered);
}

#[test]
fn typed_context_accessors_are_cardinality_aware_and_rust_shaped() {
    let rendered = render_parser(
        "TParser",
        &parser_fixture_data("left-recursive-labels/T.g4"),
    )
    .expect("parser should render");
    let s_context = rendered_context_impl(&rendered, "SContext");
    // Snapshot the whole sliced impl block: the generated accessor surface is the assertion.
    // This also subsumes the old `!contains(...)` guards — a forbidden method (self-reference,
    // `_all`) would surface as a diff instead of needing to be enumerated by hand.
    insta::assert_snapshot!("typed_context_accessors_s_context", s_context);

    let e_context = rendered_context_impl(&rendered, "EContext");
    insta::assert_snapshot!("typed_context_accessors_e_context", e_context);
}

#[test]
fn typed_context_mechanics_use_the_runtime_codegen_api() {
    let rendered = render_parser(
        "TParser",
        &parser_fixture_data("boolean-rule-arguments/T.g4"),
    )
    .expect("parser should render");

    assert_eq!(
        rendered
            .matches("antlr4_runtime::__antlr4_rust_context!")
            .count(),
        2
    );
    assert!(!rendered.contains("fn __from_node("));
    assert!(!rendered.contains("impl<State> std::fmt::Display for"));
    assert!(!rendered.contains("pub struct __RuleAttrs0"));
    assert!(rendered.contains("pub struct __RuleAttrs1"));

    insta::assert_snapshot!(
        "typed_context_runtime_support_declarations",
        format!(
            "{}\n\n{}",
            rendered_context_declaration(&rendered, "SContext"),
            rendered_context_declaration(&rendered, "FlagContext"),
        )
    );
}

#[test]
fn typed_context_accessors_reserve_direct_terminals() {
    let rendered = render_parser(
        "TParser",
        &parser_fixture_data("context-accessor-collision/T.g4"),
    )
    .expect("parser should render");
    let start_context = rendered_context_impl(&rendered, "StartContext");
    let labeled_context = rendered_context_impl(&rendered, "LabeledContext");
    let context_surface = format!("StartContext\n{start_context}LabeledContext\n{labeled_context}");

    insta::assert_snapshot!(
        "typed_context_accessors_reserved_direct_terminals",
        context_surface
    );
}

#[test]
fn typed_context_accessors_include_tokens_from_grouped_sets() {
    let rendered = render_parser(
        "TParser",
        &parser_fixture_data("grouped-token-accessors/T.g4"),
    )
    .expect("parser should render");
    let expression_context = rendered_context_impl(&rendered, "ExpressionContext");
    let sequence_context = rendered_context_impl(&rendered, "OperatorSequenceContext");
    let eof_choice_context = rendered_context_impl(&rendered, "EofChoiceContext");
    let context_surface = format!(
        "ExpressionContext\n{expression_context}OperatorSequenceContext\n{sequence_context}EofChoiceContext\n{eof_choice_context}"
    );

    insta::assert_snapshot!("typed_context_accessors_grouped_tokens", context_surface);
}

#[test]
fn literal_labels_keep_terminal_action_semantics() {
    let data = parser_fixture_data("typed-tree-walkers/Calculator.g4");
    let model = structural_embedded_model(&data, false).expect("structural model should resolve");
    let labeled_tokens = model
        .rules
        .iter()
        .find(|rule| rule.name == "labeledTokens")
        .expect("labeledTokens rule");
    let literal = labeled_tokens
        .alts
        .iter()
        .flat_map(|alternative| &alternative.refs)
        .find(|element| element.label.as_deref() == Some("literal"))
        .expect("literal label");

    assert!(literal.is_block);
    assert_eq!(literal.token_types.len(), 1);
}

#[test]
fn structural_set_token_types_expand_literal_ranges() {
    let data = parser_fixture_data("typed-tree-walkers/Calculator.g4");
    let vocabulary = &data
        .semantic
        .expect("semantic grammar")
        .recognizer
        .vocabulary;
    let mut literals = vocabulary
        .by_literal
        .iter()
        .map(|(literal, token_type)| (literal.clone(), *token_type))
        .collect::<Vec<_>>();
    literals.sort_unstable_by_key(|(_, token_type)| *token_type);
    assert!(literals.len() >= 3, "fixture needs three literal tokens");
    let (start, start_type) = literals[0].clone();
    let (stop, stop_type) = literals[2].clone();
    let range = SetElement::Range {
        source: grammar::model::ElementId::new(0),
        start,
        stop,
        span: SourceSpan::empty(SourceId::new(0)),
        options: Vec::new(),
    };
    let expected = (start_type..=stop_type).collect::<Vec<_>>();

    assert_eq!(
        structural_set_token_types(false, std::slice::from_ref(&range), vocabulary),
        expected
    );
    assert_eq!(
        structural_set_token_types(true, &[range], vocabulary),
        (1..=vocabulary.max_token_type())
            .filter(|token_type| !expected.contains(token_type))
            .collect::<Vec<_>>()
    );
}

#[test]
fn typed_context_accessors_preserve_ebnf_list_and_single_labels() {
    let data = parser_fixture_data("combined-contexts/Shapes.g4");
    let model = structural_embedded_model(&data, false).expect("structural model should resolve");
    let start = model
        .rules
        .iter()
        .find(|rule| rule.name == "start")
        .expect("start rule");
    let many = start
        .alts
        .iter()
        .find(|alternative| alternative.label.as_deref() == Some("Many"))
        .expect("many alternative");
    let rest = many
        .refs
        .iter()
        .filter(|element| element.label.as_deref() == Some("rest"))
        .collect::<Vec<_>>();
    assert_eq!(rest.len(), 2);
    assert!(rest.iter().all(|element| element.stable_accessor));
    assert_eq!(rest[0].cardinality, embedded::ChildCardinality::ONE);
    assert_eq!(
        rest[1].cardinality,
        embedded::ChildCardinality { min: 0, max: None }
    );

    let rendered = render_parser("ShapesParser", &data).expect("parser should render");
    let many_context = rendered_context_impl(&rendered, "ManyLabelContext");
    // The EBNF-list accessor (iterator over repeated `rest`) is captured whole.
    insta::assert_snapshot!("typed_context_accessors_many_context", many_context);

    let latest_context = rendered_context_impl(&rendered, "LatestContext");
    // The single-label accessor (`.skip(0).last()` selecting the latest occurrence) is captured
    // whole rather than probed for two substrings.
    insta::assert_snapshot!("typed_context_accessors_latest_context", latest_context);
}

#[test]
fn token_group_label_across_alternatives_unions_sets_and_guards_shadowing() {
    let data = parser_fixture_data("multi-alternative-label/T.g4");
    let rendered = render_parser("TParser", &data).expect("parser should render");

    // The `op` label spans two alternatives with different token groups;
    // the accessor must match the union of both sets.
    let calc_context = rendered_context_impl(&rendered, "CalcContext");
    insta::assert_snapshot!("multi_alternative_label_calc_context", calc_context);

    // `lead = PLUS? PLUS unary`: with `lead` absent the unlabeled PLUS
    // slides into `.nth(0)`, so no accessor may be emitted at all.
    let shadowed_context = rendered_context_impl(&rendered, "ShadowedContext");
    assert!(
        !shadowed_context.contains("pub fn lead("),
        "optional labeled token shadowed by a following union match must drop its accessor\n{shadowed_context}"
    );
    insta::assert_snapshot!("multi_alternative_label_shadowed_context", shadowed_context);

    // A following union member under the same optional block cannot outlive
    // the label, so the ordinary positional read remains faithful.
    insta::assert_snapshot!(
        "multi_alternative_label_shared_optional_block_context",
        rendered_context_impl(&rendered, "SharedOptionalBlockContext")
    );
    // A direct `?` on the label breaks that coupling and must still decline.
    insta::assert_snapshot!(
        "multi_alternative_label_direct_optional_in_shared_block_context",
        rendered_context_impl(&rendered, "DirectOptionalInSharedBlockContext")
    );
    // A repeated and a non-repeated declaration can share a last-match read
    // only when neither alternative has a later union member.
    insta::assert_snapshot!(
        "multi_alternative_label_mixed_repetition_context",
        rendered_context_impl(&rendered, "MixedRepetitionContext")
    );
    insta::assert_snapshot!(
        "multi_alternative_label_prefixed_mixed_repetition_context",
        rendered_context_impl(&rendered, "PrefixedMixedRepetitionContext")
    );
    insta::assert_snapshot!(
        "multi_alternative_label_mixed_repetition_followed_context",
        rendered_context_impl(&rendered, "MixedRepetitionFollowedContext")
    );
}

#[test]
fn context_label_selection_reconciliation_covers_each_compatibility_path() {
    let selection = |preferred, compatible_last_after| ContextLabelSelection {
        preferred,
        compatible_last_after,
    };
    let unanimous_nth = [
        selection(ContextLabelSelector::Nth(2), None),
        selection(ContextLabelSelector::Nth(2), Some(2)),
    ];
    let unanimous_list = [
        selection(ContextLabelSelector::AllAfter(1), None),
        selection(ContextLabelSelector::AllAfter(1), None),
    ];
    let promote_zero = [
        selection(ContextLabelSelector::Nth(0), Some(0)),
        selection(ContextLabelSelector::LastAfter(0), Some(0)),
    ];
    let promote_one = [
        selection(ContextLabelSelector::LastAfter(1), Some(1)),
        selection(ContextLabelSelector::Nth(1), Some(1)),
    ];
    let different_skips = [
        selection(ContextLabelSelector::Nth(0), Some(0)),
        selection(ContextLabelSelector::LastAfter(1), Some(1)),
    ];
    let unsafe_nth = [
        selection(ContextLabelSelector::Nth(0), None),
        selection(ContextLabelSelector::LastAfter(0), Some(0)),
    ];
    let incompatible_modes = [
        selection(ContextLabelSelector::Nth(0), Some(0)),
        selection(ContextLabelSelector::AllAfter(0), None),
    ];

    insta::assert_debug_snapshot!(
        "context_label_selection_reconciliation",
        [
            ("empty", reconcile_context_label_selections(&[])),
            (
                "unanimous nth",
                reconcile_context_label_selections(&unanimous_nth),
            ),
            (
                "unanimous list",
                reconcile_context_label_selections(&unanimous_list),
            ),
            (
                "promote zero",
                reconcile_context_label_selections(&promote_zero),
            ),
            (
                "promote one",
                reconcile_context_label_selections(&promote_one),
            ),
            (
                "different skips",
                reconcile_context_label_selections(&different_skips),
            ),
            (
                "unsafe nth",
                reconcile_context_label_selections(&unsafe_nth),
            ),
            (
                "incompatible modes",
                reconcile_context_label_selections(&incompatible_modes),
            ),
        ]
    );
}

/// Issue #201: labels nested inside an unlabeled grouping block, and a
/// single/list label pair on one rule, both reach the typed surface — while
/// layouts where positional lookup could resolve to another choice branch's
/// child still decline.
#[test]
fn grouped_and_mixed_same_rule_labels_emit_accessors_without_crossing_branches() {
    let data = parser_fixture_data("multi-alternative-label/T.g4");
    let rendered = render_parser("TParser", &data).expect("parser should render");

    // `(doc = IDENT)? (oneway = STAR | IN errors += unary ...)?`: the labels
    // sit inside unlabeled grouping blocks, so collapsing each block into
    // one token-group ref would swallow them.
    insta::assert_snapshot!(
        "multi_alternative_label_grouped_context",
        rendered_context_impl(&rendered, "GroupedContext")
    );

    // `name = unary ... errors += unary`: a single and a list label on the
    // same rule must each resolve past the other's children.
    insta::assert_snapshot!(
        "multi_alternative_label_mixed_context",
        rendered_context_impl(&rendered, "MixedContext")
    );

    // A label buried under redundant grouping levels still reaches the
    // surface — the collapse check descends nested blocks.
    insta::assert_snapshot!(
        "multi_alternative_label_nested_group_context",
        rendered_context_impl(&rendered, "NestedGroupContext")
    );

    // The three declining shapes are snapshotted whole rather than probed
    // with `!contains`, so the absent accessor is visible alongside
    // everything the context *does* expose:
    //
    // * `mixed_unbounded` — a variable count of the label's own target ahead
    //   of it leaves no fixed `.skip(N)`;
    // * `branch_hazard` — only one branch supplies the label while its
    //   sibling matches the same target unlabeled, so `.nth(0)` could read
    //   the sibling's child;
    // * `branch_rival` — rival labels on one target across branches must not
    //   read each other's child.
    // An exhaustive choice keeps a following label's accessor (its prefix
    // count is fixed at one however the choice branches), while a preceding
    // *overlapping* token group does not (only some parses put a matching
    // child ahead of the label).
    insta::assert_snapshot!(
        "multi_alternative_label_exhaustive_prefix_context",
        rendered_context_impl(&rendered, "ExhaustivePrefixContext")
    );
    insta::assert_snapshot!(
        "multi_alternative_label_overlapping_group_context",
        rendered_context_impl(&rendered, "OverlappingGroupContext")
    );
    // Making that same choice optional removes the fixed position, so the
    // following label loses its accessor — the branch-local cardinality is
    // what distinguishes the two.
    insta::assert_snapshot!(
        "multi_alternative_label_optional_prefix_context",
        rendered_context_impl(&rendered, "OptionalPrefixContext")
    );
    // One label over mutually exclusive branches merges into a single read; and
    // restricting to the label's own path lets a sibling branch be ignored
    // rather than demanded.
    insta::assert_snapshot!(
        "multi_alternative_label_merged_rivals_context",
        rendered_context_impl(&rendered, "MergedRivalsContext")
    );
    // Repeated scalar declarations merge as a *last*-match read, since ANTLR
    // overwrites a scalar label on every iteration.
    insta::assert_snapshot!(
        "multi_alternative_label_merged_repeats_context",
        rendered_context_impl(&rendered, "MergedRepeatsContext")
    );
    insta::assert_snapshot!(
        "multi_alternative_label_path_restricted_context",
        rendered_context_impl(&rendered, "PathRestrictedContext")
    );
    // Two ways the on-path restriction can overstate what it knows, both
    // declining as a result:
    //
    // * `closed_repeat_prefix` — sharing every *optional* group with the label
    //   proves those groups ran, but a closed `+` inside them ran an unknown
    //   number of times, so the prefix count stays unfixed;
    // * `inner_choice_arity` — dropping the taken outer choice must drop its
    //   arity too, or the surviving three-way inner choice is read as an
    //   exhaustive two-way one and the prefix count is wrongly fixed at 1.
    insta::assert_snapshot!(
        "multi_alternative_label_closed_repeat_prefix_context",
        rendered_context_impl(&rendered, "ClosedRepeatPrefixContext")
    );
    insta::assert_snapshot!(
        "multi_alternative_label_inner_choice_arity_context",
        rendered_context_impl(&rendered, "InnerChoiceArityContext")
    );
    // Nesting the exhaustive choice keeps the count fixed: the inner choice's
    // agreed contribution rolls up into the outer branch.
    insta::assert_snapshot!(
        "multi_alternative_label_nested_exhaustive_prefix_context",
        rendered_context_impl(&rendered, "NestedExhaustivePrefixContext")
    );

    for (name, snapshot) in [
        (
            "MixedUnboundedContext",
            "multi_alternative_label_mixed_unbounded_context",
        ),
        (
            "BranchHazardContext",
            "multi_alternative_label_branch_hazard_context",
        ),
        (
            "BranchRivalContext",
            "multi_alternative_label_branch_rival_context",
        ),
    ] {
        insta::assert_snapshot!(snapshot, rendered_context_impl(&rendered, name));
    }
}

/// Every label shape whose *resolution outcome* this module decides, kept in
/// one place so a change to any guard shows up as a diff here rather than as a
/// silent behaviour change in a grammar nobody tests.
///
/// A label resolves only when the read `translate_element_read` emits provably
/// selects that label's own element. `resolve` means the grammar generates;
/// `decline` means resolution fails loudly (`cannot translate $x`), which is
/// always preferable to a read that returns some *other* child. Each entry
/// records why, because the two outcomes are easy to swap by accident — most
/// of these were originally over-rejections introduced while fixing a
/// miscompile, or vice versa.
#[test]
fn label_resolution_corpus_matches_expected_outcomes() {
    // (fixture, label, resolves, why). `label` is the accessor/read the case
    // turns on: rendering succeeds either way for a grammar without actions, so
    // a declined *accessor* shows up as the method being absent rather than as
    // a render error.
    const CORPUS: &[(&str, &str, bool, &str)] = &[
        // Declines: the read would select a child the label never bound.
        (
            "SiblingUnlabeledSameTarget",
            "xs",
            false,
            "an action after a choice runs for every branch, so a sibling's token is not the label's",
        ),
        (
            "OptionalBlockFollowedByTerminal",
            "x",
            false,
            "an absent optional block lets the follower occupy its index",
        ),
        (
            "ActionAfterNestedChoice",
            "xs",
            false,
            "a list read would fold in the sibling branch's child",
        ),
        (
            "LiteralAliasDifferingOccurrence",
            "x",
            false,
            "block and token reads index in different units, so occurrence 1 means different children",
        ),
        (
            "MergedDeclarationOptionalFollower",
            "x",
            false,
            "an absent optional declaration lets the follower slide into the merged read",
        ),
        (
            "ListDeclarationsDifferingStart",
            "xs",
            false,
            "one `AllAfter` skip cannot serve branches that begin at different offsets",
        ),
        (
            "InnerGroupClosedBeforeAction",
            "x",
            false,
            "an inner group that closed before the action proves nothing about what matched",
        ),
        (
            "AliasDifferingOccurrenceInAlt",
            "x",
            false,
            "block and token occurrences are comparable only at zero",
        ),
        (
            "DeclarationsDifferingOccurrence",
            "x",
            false,
            "one positional read cannot serve declarations at different occurrences",
        ),
        (
            "DeclarationsDifferingRepetition",
            "x",
            false,
            "a repeated declaration needs a last-match read the others do not",
        ),
        (
            "RepeatedBlockLabel",
            "x",
            false,
            "a repeated block label exposes its last match, which a positional read cannot express",
        ),
        (
            "RepeatedSiblingSpansIndex",
            "x",
            false,
            "a repeated sibling spans a range of terminal positions, not just its start",
        ),
        (
            "RepeatedMergeFollowedByMatch",
            "x",
            false,
            "a shared last-match read would return a following unlabeled child on the non-repeated branch",
        ),
        (
            "InitScalarRuleLabel",
            "x",
            false,
            "a scalar rule read lowers to `.expect(...)`, which panics at rule entry",
        ),
        (
            "AliasDeclarationsInChoice",
            "x",
            false,
            "known limitation: alias declarations inside one nested choice still decline (see PR discussion)",
        ),
        (
            "FallbackReadSiblingAlternative",
            "x",
            false,
            "a `last()` fallback can select any terminal, so a non-declaring alternative satisfies it",
        ),
        (
            "MixedModeLeadingTerminal",
            "x",
            false,
            "mixed-mode occurrence zero coincides only when no terminal precedes either side",
        ),
        (
            "PrecedingSiblingBranch",
            "x",
            false,
            "a same-target sibling *before* the label impersonates it as readily as one after",
        ),
        (
            "ExpandedBlockTerminalState",
            "x",
            false,
            "an expanded block still matched a terminal, so what follows it is not leading",
        ),
        (
            "ListAliasAcrossModes",
            "xs",
            false,
            "a list read has no form common to token and block mode, so aliases cannot merge",
        ),
        (
            "ListLabelWithoutIterator",
            "xs",
            false,
            "a list label whose target names no rule or token type has no iterator read",
        ),
        (
            "InnerChoiceArityInAction",
            "x",
            false,
            "dropping the taken outer choice must drop its arity, or a three-way inner choice reads as two-way",
        ),
        // Resolves: valid reads that must not be rejected.
        (
            "ExhaustiveInnerChoiceInAction",
            "x",
            true,
            "a genuinely exhaustive inner choice keeps its fixed prefix count of one",
        ),
        (
            "ActionInCollapsibleChoice",
            "x",
            true,
            "a token-only choice holding an action keeps its branch spans",
        ),
        (
            "CollapsedBlockIsTerminal",
            "x",
            false,
            "a collapsed token group is itself a terminal child, so what follows is not leading",
        ),
        (
            "UnrelatedLaterChoiceConfinement",
            "x",
            false,
            "confinement to a later choice says nothing about an earlier sibling",
        ),
        (
            "ListPrefixRepeatsWithLabel",
            "xs",
            false,
            "a same-target prefix sharing a repeated group interleaves with the labeled children",
        ),
        (
            "ClosedRepeatedGroupPrefix",
            "x",
            false,
            "a closed repeated group still contributes an unfixed number of preceding children",
        ),
        (
            "RecoveredDeletedTokenIndex",
            "x",
            true,
            "the positional block read skips deleted-token errors, so recovery cannot shift its index",
        ),
        (
            "ReassignedAfterAction",
            "x",
            true,
            "a declaration after the action has not assigned the label yet, so it cannot conflict",
        ),
        (
            "ForwardBlockLabel",
            "x",
            true,
            "a forward label's prefix is entirely in the action's future, so it cannot make the index inexact",
        ),
        (
            "OptionalGroupSharedWithLabel",
            "x",
            true,
            "an optional group shared with the label is taken wherever the label is bound",
        ),
        (
            "SiblingBranchShorterThanOccurrence",
            "x",
            true,
            "a sibling branch that cannot reach the selected occurrence is no collision",
        ),
        (
            "IdenticalDeclarationsInChoice",
            "x",
            true,
            "isolating one declaration must not reclassify its twin as an impostor",
        ),
        (
            "MandatoryInnerGroupClosed",
            "x",
            true,
            "a mandatory inner group relaxes nothing, so its closing before the action is irrelevant",
        ),
        (
            "InlineActionBeforeLabel",
            "x",
            true,
            "an inline action sees no children, so a later unbounded run cannot poison it",
        ),
        (
            "SiblingDeclarationIrrelevant",
            "x",
            true,
            "a branch-confined action never sees a sibling branch's declaration",
        ),
        (
            "NestedChoiceInsideConfinedBranch",
            "x",
            true,
            "confinement to an outer branch does not restrict a nested choice inside it",
        ),
        (
            "ListAliasDeclarations",
            "xs",
            true,
            "list declarations name one token type through two source forms",
        ),
        (
            "ChoicePrefixOutsideLabel",
            "x",
            true,
            "a choice before the label contributes a fixed count when its branches agree",
        ),
        (
            "LiteralAliasSameOccurrence",
            "x",
            true,
            "block and token reads coincide at occurrence zero",
        ),
        (
            "NestedChoiceSatisfiability",
            "x",
            true,
            "nested choices fold rather than sum: the alternative builds one child",
        ),
        (
            "RepeatedScalarMerge",
            "x",
            true,
            "a repeated scalar label exposes its last match",
        ),
        (
            "ActionInsideTakenGroup",
            "x",
            true,
            "the enclosing group's quantifier is satisfied wherever the action runs",
        ),
        (
            "ActionOnlyBranch",
            "x",
            true,
            "a branch holding only an action still identifies itself by span",
        ),
        (
            "InitActionBeforeChildren",
            "xs",
            true,
            "an `@init` body runs before any child exists, so no read can be polluted",
        ),
        (
            "ExhaustiveChoicePrefix",
            "x",
            true,
            "an exhaustive choice contributes a fixed count",
        ),
        (
            "NotSetLabelBeforeTerminal",
            "t",
            true,
            "ANTLR's `Sets/ParserNotTokenWithLabel` shape",
        ),
        (
            "ConjuredLiteralLabel",
            "x",
            true,
            "ANTLR's `ParserErrors/ConjuringUpToken` shape",
        ),
    ];

    let mut wrong = Vec::new();
    for &(fixture, label, resolves, why) in CORPUS {
        let data = parser_fixture_data(&format!("label-resolution/{fixture}.g4"));
        let rendered = render_parser_with_options(
            &format!("{fixture}Parser"),
            &data,
            ParserRenderOptions {
                embedded: true,
                ..ParserRenderOptions::default()
            },
        );
        // Which signal reports the decision depends on how the fixture reads its
        // label. A grammar whose *action* reads it fails to render outright when
        // resolution declines; one that relies on the typed accessor renders
        // either way, and the decision surfaces as the method's presence.
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/antlr4-rust-gen/label-resolution")
                .join(format!("{fixture}.g4")),
        )
        .expect("fixture should be readable");
        let reads_via_action = source.contains(&format!("${label}"));
        let resolved = rendered
            .as_ref()
            .is_ok_and(|parser| reads_via_action || parser.contains(&format!("pub fn {label}(")));
        if resolved != resolves {
            let outcome = if resolves { "resolve" } else { "decline" };
            let error = rendered
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default();
            wrong.push(format!(
                "  {fixture} (${label}): expected {outcome} — {why} {error}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "label resolution changed:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn non_embedded_parser_action_runs_at_generated_position() {
    let rendered = render_parser("TParser", &action_parser_data()).expect("parser should render");

    assert!(
        rendered.contains("parse_generated_rule_0_dispatch"),
        "a hook-routed parser action must remain on the generated path"
    );
    assert!(rendered.contains("parser_action_at_current_indexed"));
    assert!(rendered.contains("parser_action_hook_with_context(action, &__ctx)"));
    assert!(rendered.contains("action_indices: &[("));
    assert!(rendered.contains("self.base.parser_action_hook(action, tree)"));
    assert!(!rendered.contains(&format!("{}{}", "Generated", "Action")));
    assert!(!rendered.contains(&format!("{}{}", "generated", "_actions")));
}

#[test]
fn untranslated_parser_predicate_keeps_generated_rule() {
    // Issue #209: an untranslated predicate body must lower as a
    // generatable `Unknown` template (hook → unknown-policy chain), not
    // leave its coordinate uncovered — an uncovered coordinate disables
    // the rule's generated body and the drop cascades to every calling
    // rule, forcing the whole grammar onto the 5-6x slower interpreter.
    let templates = structural_predicate_templates(
        &predicate_parser_data(),
        SemanticsKind::ParserPredicate,
        &SemPatternFile::default(),
    )
    .expect("templates should collect");
    insta::assert_debug_snapshot!("untranslated_parser_predicate_templates", templates);

    let rendered =
        render_parser("SParser", &predicate_parser_data()).expect("parser should render");
    assert!(
        rendered.contains("parse_generated_rule_0_dispatch"),
        "an untranslated predicate must not disable the generated rule"
    );
    // The coordinate reaches SemIR as a hook node, so an attached
    // typed/closure hook is still consulted before the policy applies.
    assert!(rendered.contains("PExpr::Hook"));
}

#[test]
fn context_superclass_does_not_disable_generated_rules() {
    let data = parser_fixture_data("context-superclass/T.g4");
    let rendered = render_parser("TParser", &data).expect("parser should render");

    assert!(rendered.contains("parse_generated_rule_0"));
    assert!(rendered.contains("track_alt_numbers: true"));
}

#[test]
fn generated_parser_handles_diagnostic_reporting() {
    let rendered = render_parser("TParser", &minimal_parser_data()).expect("parser should render");

    assert!(!rendered.contains("if !self.base.report_diagnostic_errors() || __generated_only"));
    assert!(rendered.contains("self.parse_interpreted_rule_precedence(rule_index, precedence)?"));
}

#[test]
fn generated_only_mode_disables_missing_rule_fallback() {
    let rendered = render_parser("TParser", &minimal_parser_data()).expect("parser should render");

    assert!(rendered.contains("ANTLR4_RUST_GENERATED_ONLY"));
    assert!(rendered.contains("let __generated_only = self.generated_only();"));
    assert!(!rendered.contains("GeneratedRuleError::Recoverable"));
    assert!(rendered.contains("generated parser did not emit rule {}"));
}

#[test]
fn require_generated_parser_reports_missing_rules() {
    let error = require_all_parser_rules_generated(&[None], &minimal_parser_data())
        .expect_err("missing generated rule should fail strict mode");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "generated parser did not emit 1 rule(s): s"
    );
}

#[test]
fn portable_local_semantics_reject_missing_generated_owner() {
    let error = require_portable_local_rules_generated(
        &[None],
        &BTreeSet::from([0]),
        &minimal_parser_data(),
    )
    .expect_err("portable local semantics cannot use interpreted fallback");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "portable local semantics require 1 generated parser rule(s): s"
    );
}

#[test]
fn portable_local_semantics_reject_missing_generated_caller() {
    let required = atn_rule_callers_reaching(&entry_candidate_atn(), &BTreeSet::from([1]), 4);
    assert_eq!(required, BTreeSet::from([0, 1, 2]));

    let rules = vec![
        None,
        Some(test_rule(1, Vec::new())),
        Some(test_rule(2, Vec::new())),
        Some(test_rule(3, Vec::new())),
    ];
    let data = RecognizerCodegenData {
        rule_names: vec![
            "firstEntry".to_owned(),
            "child".to_owned(),
            "secondEntry".to_owned(),
            "recursive".to_owned(),
        ],
        ..RecognizerCodegenData::default()
    };
    let error = require_portable_local_rules_generated(&rules, &required, &data)
        .expect_err("interpreted callers cannot bypass generated local state");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "portable local semantics require 1 generated parser rule(s): firstEntry"
    );
}

#[test]
fn renders_parse_convenience_without_replacing_manual_constructor() {
    let rendered = render_parser("TParser", &minimal_parser_data()).expect("parser should render");

    insta::assert_snapshot!(
        "parser_parse_convenience",
        render_parser_parse_convenience("TParser", "T")
    );
    assert!(rendered.contains("pub struct TParserParseOutput<R, L>"));
    assert!(rendered.contains("pub result: R,"));
    assert!(rendered.contains("pub parser: TParser<L>,"));
    assert!(rendered.contains("pub fn parse<L: TokenSource>("));
    assert!(rendered.contains("pub fn parse_with_parser<L: TokenSource, R>("));
    assert!(
        rendered.contains("pub fn parse_stream<I: antlr4_runtime::CharStream, L: TokenSource>(")
    );
    assert!(rendered.contains(
        "pub fn parse_stream_with_parser<I: antlr4_runtime::CharStream, L: TokenSource, R>("
    ));
    assert!(
        !rendered
            .contains(") -> Result<R, antlr4_runtime::AntlrError>\nwhere\n    L: TokenSource,")
    );
    assert!(!rendered.contains(
            ") -> Result<TParserParseOutput<R, L>, antlr4_runtime::AntlrError>\nwhere\n    L: TokenSource,"
        ));
    assert!(rendered.contains("lexer: impl FnOnce(antlr4_runtime::InputStream) -> L"));
    assert!(
        rendered.contains(
            "parse_stream(antlr4_runtime::InputStream::new(input.as_ref()), lexer, entry)"
        )
    );
    assert!(rendered.contains("let lexer = lexer(input);"));
    assert!(rendered.contains("let tokens = CommonTokenStream::new(lexer);"));
    assert!(rendered.contains("let result = entry(&mut parser)?;"));
    assert!(rendered.contains("Ok(TParserParseOutput { result, parser })"));
    assert!(rendered.contains(
        "parse_stream_with_parser(\n        antlr4_runtime::InputStream::new(input.as_ref()),"
    ));
    assert!(rendered.contains("Ok(parser.into_parsed_file(result))"));
    assert!(rendered.contains("pub fn new(input: CommonTokenStream<L>) -> Self"));
    assert!(rendered.contains("pub fn with_hooks(input: CommonTokenStream<L>, hooks: H) -> Self"));
}

#[test]
fn validated_parse_names_match_lowercase_grammar_surface() {
    let rendered = render_parser("u", &minimal_parser_data()).expect("parser should render");

    assert!(rendered.contains("pub struct uValidatedTree"));
    assert!(rendered.contains("pub enum uValidationError"));
    assert!(rendered.contains("pub fn validate(self) -> Result<uValidatedTree, uValidationError>"));
    assert!(!rendered.contains("UValidatedTree"));
    assert!(!rendered.contains("UValidationError"));
}

#[test]
fn generated_parse_output_name_does_not_collide_with_parser_type() {
    let rendered =
        render_parser("ParseOutput", &minimal_parser_data()).expect("parser should render");

    assert!(rendered.contains("pub struct ParseOutputParseOutput<R, L>"));
    assert!(rendered.contains("pub parser: ParseOutput<L>,"));
    assert!(
        rendered.contains(") -> Result<ParseOutputParseOutput<R, L>, antlr4_runtime::AntlrError>")
    );
    assert!(rendered.contains("Ok(ParseOutputParseOutput { result, parser })"));
}

#[test]
fn generated_parser_reports_diagnostics_at_outer_boundaries() {
    let rendered = render_parser("TParser", &minimal_parser_data()).expect("parser should render");

    assert!(rendered.contains("if allow_generated_fallback {"));
    assert!(rendered.contains("self.base.report_generated_parser_diagnostics();"));
    assert!(rendered.contains("self.base.report_unrecovered_parser_error(&error);"));
    assert!(rendered.contains("fn number_of_syntax_errors(&self) -> usize"));
    assert!(!rendered.contains("self.base.report_token_source_errors();"));
}

#[test]
fn generated_parser_exposes_owned_token_stream() {
    let rendered = render_parser("TParser", &minimal_parser_data()).expect("parser should render");

    assert!(rendered.contains("pub const fn token_stream(&self) -> &CommonTokenStream<L>"));
    assert!(rendered.contains("self.base.token_stream()"));
    assert!(
        rendered.contains("pub const fn token_stream_mut(&mut self) -> &mut CommonTokenStream<L>")
    );
    assert!(rendered.contains("self.base.token_stream_mut()"));
    assert!(rendered.contains("pub fn set_token_stream(&mut self, input: CommonTokenStream<L>)"));
    assert!(rendered.contains("self.base.set_token_stream(input)"));
    assert!(rendered.contains("pub fn reset(&mut self)"));
    assert!(rendered.contains("self.base.reset()"));
    assert!(rendered.contains("pub fn clear_dfa(&mut self)"));
    assert!(rendered.contains("simulator.clear_dfa()"));
    assert!(rendered.contains("ParserAtnSimulator::clear_shared_dfa(atn())"));
    assert!(rendered.contains("pub fn add_error_listener<T>(&mut self, listener: T)"));
    assert!(rendered.contains(
            "T: for<'a> antlr4_runtime::ErrorListener<dyn antlr4_runtime::Recognizer + 'a> + Send + 'static,"
        ));
    assert!(rendered.contains("self.base.add_error_listener(listener)"));
    assert!(rendered.contains("pub fn remove_error_listeners(&mut self)"));
    assert!(rendered.contains("self.base.remove_error_listeners()"));
    assert!(rendered.contains("pub fn into_token_stream(self) -> CommonTokenStream<L>"));
    assert!(rendered.contains("self.base.into_token_stream()"));
    assert!(rendered.contains("pub const fn token_store(&self) -> &antlr4_runtime::TokenStore"));
    assert!(rendered.contains("self.base.token_store()"));
    assert!(rendered.contains("pub fn into_token_store(self) -> antlr4_runtime::TokenStore"));
    assert!(rendered.contains("self.base.into_token_store()"));
}

#[test]
fn generated_parser_renames_rule_wrapper_that_collides_with_token_stream_accessor() {
    let mut data = minimal_parser_data();
    data.common.rule_names = vec!["tokenStream".to_owned()];

    let rendered = render_parser("TParser", &data).expect("parser should render");

    assert!(rendered.contains("pub const fn token_stream(&self) -> &CommonTokenStream<L>"));
    assert!(rendered.contains(
            "pub fn token_stream_rule(&mut self) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError>"
        ));
    assert!(rendered.contains("/// - `token_stream_rule()`"));
    assert!(!rendered.contains("/// - `token_stream()`"));
    assert!(!rendered.contains("pub fn token_stream(&mut self)"));
}

#[test]
fn generated_rule_recovers_own_sync_failure_unless_top_level() {
    // A rule's own sync failure (`__sync_error`) is fatal only at the top-level
    // public entry (`allow_fallback`); a nested child recovers it locally and
    // returns a partial subtree (so a parent never recovers the child's failure
    // on the parent context, losing the child subtree). Assert the generated
    // catch arm gates the `Fatal` return on `allow_fallback` and otherwise runs
    // `recover_generated_rule` + `finish_rule` + `Ok`.
    let rendered = render_parser("TParser", &minimal_parser_data()).expect("parser should render");
    let sync_arm = rendered
        .find("if let Some(__error) = __sync_error {")
        .expect("sync-error catch arm present");
    let rest = &rendered[sync_arm..];
    // Inside the sync arm, the Fatal return is guarded by `if allow_fallback`.
    let guard = rest
        .find("if allow_fallback {")
        .expect("fatal return gated on allow_fallback");
    let fatal = rest
        .find("return Err(GeneratedRuleError::Fatal(__error));")
        .expect("fatal return present");
    assert!(
        guard < fatal,
        "Fatal return must be inside the allow_fallback guard"
    );
    let count = rest
        .find("self.base.record_generated_syntax_error();")
        .expect("fatal sync path records syntax error");
    let rollback = rest
        .find("self.base.rollback_generated_tree(__generated_diagnostic_marker);")
        .expect("fatal sync path rolls back only partial tree state");
    assert!(
        guard < rollback && rollback < count && count < fatal,
        "fatal sync path must preserve diagnostics, roll back the tree, and increment before returning"
    );
    // And the nested-child path recovers locally and returns Ok.
    let recover = rest
        .find("self.base.recover_generated_rule(&mut __ctx, atn(), __error);")
        .expect("local recovery present in sync arm");
    assert!(
        recover > guard,
        "recover path follows the guarded fatal return"
    );
    assert!(rest[recover..].contains("return Ok(__tree);"));
}

#[test]
fn call_rule_step_skips_child_action_scaffolding_without_parser_actions() {
    let rendered = render_call_rule_step(&[true, true], &[false, false], &[]);

    assert!(rendered.contains("let __child = self.parse_generated_rule_1_dispatch(0, false).map_err(GeneratedRuleError::into_error);"));
    assert!(rendered.contains("self.base.discard_invoking_state(__invoking_marker);"));
    assert!(rendered.contains("let __child = __child?;"));
    assert!(rendered.contains("self.base.add_parse_child(&mut __ctx, __child);"));
    assert!(!rendered.contains("__child_action_marker"));
    assert!(!rendered.contains("__child_member_checkpoint"));
    assert!(!rendered.contains(&format!(
        "{}{}{}{}",
        "Generated", "Action::", "Member", "Snapshot"
    )));
    assert!(!rendered.contains(&format!(
        "{}{}{}",
        "CTX_ROOTED", "_ACTION_STATES", ".contains"
    )));
}

#[test]
fn renders_wildcard_match_through_recovering_path() {
    // A wildcard (`.`) must go through the recovering match (modeled as an
    // empty-complement not-set over the full vocabulary) so a wildcard at EOF
    // performs ANTLR's single-token insertion instead of aborting the rule.
    let rule = GeneratedParserRule {
        rule_index: 0,
        entry_state: 0,
        left_recursive: false,
        steps: vec![GeneratedParserStep::MatchWildcard { follow_state: 7 }],
    };

    let rendered = render_generated_rule_dispatch(&[Some(rule)], &[], &BTreeMap::new(), false);

    // Recovering not-set over 1..=max with an empty exclusion = "any token",
    // threading the wildcard's follow state for EOF-insertion follow checks.
    assert!(
        rendered.contains("match_not_set_recovering(&[], 1, atn().max_token_type(), 7, atn())")
    );
    assert!(rendered.contains("__consumed_eof |= __match.consumed_eof();"));
    // The old non-recovering call must be gone.
    assert!(!rendered.contains("self.base.match_wildcard()"));
}

#[test]
fn renders_packed_token_sets_for_generated_matches_and_lookahead() {
    let step = mts(4, vec![(2, 4), (9, 9)], 7);
    let rule = GeneratedParserRule {
        rule_index: 0,
        entry_state: 0,
        left_recursive: false,
        steps: vec![step.clone()],
    };

    let rendered = render_generated_rule_dispatch(&[Some(rule)], &[], &BTreeMap::new(), false);

    assert!(rendered.contains(
            "match_token_set_recovering(atn().token_set(4).expect(\"generated parser token-set index\"), 7, atn())"
        ));
    assert_eq!(
        leading_lookahead_condition(&[step], "__la"),
        Some(
            "atn().token_set(4).expect(\"generated parser token-set index\").contains(__la)"
                .to_owned()
        )
    );

    let not_step = mnts(5, vec![(3, 6)], 8);
    let not_rule = GeneratedParserRule {
        rule_index: 0,
        entry_state: 0,
        left_recursive: false,
        steps: vec![not_step.clone()],
    };
    let rendered = render_generated_rule_dispatch(&[Some(not_rule)], &[], &BTreeMap::new(), false);

    assert!(rendered.contains(
            "match_not_token_set_recovering(atn().token_set(5).expect(\"generated parser token-set index\"), 1, atn().max_token_type(), 8, atn())"
        ));
    assert_eq!(
            leading_lookahead_condition(&[not_step], "__la"),
            Some(
                "(1..=atn().max_token_type()).contains(&__la) && !(atn().token_set(5).expect(\"generated parser token-set index\").contains(__la))"
                    .to_owned()
            )
        );
}

#[test]
fn generated_decision_does_not_reject_semantic_context_metadata() {
    let alts = vec![vec![mt(1, 0)], vec![]];
    let mut rendered = String::new();

    render_generated_decision(
        &mut rendered,
        DecisionRender {
            state: 1,
            decision: 0,
            track_alt_number: false,
            allow_semantic_context: false,
            force_context: false,
            fast_path: None,
            alts: &alts,
        },
        0,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: None,
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &BTreeMap::new(),
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls: &[],
            atn_preferred_rule_calls: &[],
            adaptive_atn_preferred_rule_slots: &[],
            adaptive_atn_probe_rule_slots: &[],
        },
    );

    assert!(rendered.contains("ll1_decision_prediction(atn(), 1)"));
    assert!(rendered.contains("prediction_mode() != antlr4_runtime::PredictionMode::Sll"));
    assert!(!rendered.contains("has_semantic_context"));
}

#[test]
fn generated_decision_filters_semantic_predicate_alts() {
    let alts = vec![
        vec![
            GeneratedParserStep::Predicate {
                rule_index: 1,
                pred_index: 0,
            },
            mt(1, 2),
        ],
        vec![
            GeneratedParserStep::Predicate {
                rule_index: 1,
                pred_index: 1,
            },
            mt(1, 3),
        ],
        vec![mt(2, 4)],
    ];
    let mut rendered = String::new();

    render_generated_decision(
        &mut rendered,
        DecisionRender {
            state: 1,
            decision: 0,
            track_alt_number: false,
            allow_semantic_context: true,
            force_context: false,
            fast_path: None,
            alts: &alts,
        },
        0,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: None,
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &BTreeMap::new(),
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls: &[],
            atn_preferred_rule_calls: &[],
            adaptive_atn_preferred_rule_slots: &[],
            adaptive_atn_probe_rule_slots: &[],
        },
    );

    // One decision renders into a fresh String; snapshot the whole emitted control flow (the
    // semantic-context gate, both predicate probes, the alt rewrite, the no-viable fallback)
    // instead of six positive probes plus one negative guard.
    insta::assert_snapshot!(
        "generated_decision_filters_semantic_predicate_alts",
        rendered
    );
}

#[test]
fn generated_decision_does_not_hoist_portable_predicate_past_local_action() {
    let alts = vec![
        vec![
            GeneratedParserStep::Action {
                source_state: 5,
                rule_index: 1,
                action_index: Some(0),
            },
            GeneratedParserStep::Predicate {
                rule_index: 1,
                pred_index: 0,
            },
            mt(1, 2),
        ],
        vec![mt(1, 3)],
    ];
    let declarations = vec![
        Vec::new(),
        vec!["let mut __antlr_local_seen = false;".to_owned()],
    ];
    let inline_actions = BTreeMap::from([(5, "__antlr_local_seen = true;".to_owned())]);
    let predicates = BTreeMap::from([((1, 0), ("__antlr_local_seen".to_owned(), None))]);
    let required_generated_rules = BTreeSet::from([1]);
    let mut rendered = String::new();

    render_generated_decision(
        &mut rendered,
        DecisionRender {
            state: 1,
            decision: 0,
            track_alt_number: false,
            allow_semantic_context: true,
            force_context: false,
            fast_path: None,
            alts: &alts,
        },
        0,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: Some(PortableLocalStepRender {
                declarations: &declarations,
                predicates: &predicates,
                required_generated_rules: &required_generated_rules,
            }),
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &inline_actions,
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls: &[],
            atn_preferred_rule_calls: &[],
            adaptive_atn_preferred_rule_slots: &[],
            adaptive_atn_probe_rule_slots: &[],
        },
    );

    // Snapshot the whole rendered decision so the (non-)hoisting of the portable predicate is
    // visible in context; the explicit ordering invariant below is kept because it names the
    // guarantee crisply — a snapshot shows the layout but does not assert the ordering.
    insta::assert_snapshot!(
        "generated_decision_does_not_hoist_portable_predicate_past_local_action",
        rendered
    );
    let assignment = rendered
        .find("__antlr_local_seen = true;")
        .expect("portable assignment is rendered in the committed alternative");
    let predicate = rendered
        .find("if !(__antlr_local_seen)")
        .expect("portable predicate is evaluated in the committed alternative");
    assert!(assignment < predicate);
}

#[test]
fn generated_decision_records_adaptive_diagnostics() {
    let alts = vec![vec![mt(1, 4)], vec![mt(2, 5)]];
    let mut rendered = String::new();

    render_generated_decision(
        &mut rendered,
        DecisionRender {
            state: 16,
            decision: 0,
            track_alt_number: false,
            allow_semantic_context: false,
            force_context: false,
            fast_path: None,
            alts: &alts,
        },
        0,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: None,
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &BTreeMap::new(),
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls: &[],
            atn_preferred_rule_calls: &[],
            adaptive_atn_preferred_rule_slots: &[],
            adaptive_atn_probe_rule_slots: &[],
        },
    );

    assert!(rendered.contains("record_generated_prediction_diagnostic(atn(), 16, &__prediction)"));
    assert!(!rendered.contains("__diagnostic_la"));
}

#[test]
fn generated_semantic_decision_reports_filtered_ambiguity_diagnostics() {
    let alts = vec![
        vec![mt(2, 4)],
        vec![mt(2, 5)],
        vec![
            GeneratedParserStep::Predicate {
                rule_index: 1,
                pred_index: 0,
            },
            mt(2, 6),
        ],
    ];
    let mut rendered = String::new();

    render_generated_decision(
        &mut rendered,
        DecisionRender {
            state: 16,
            decision: 0,
            track_alt_number: false,
            allow_semantic_context: true,
            force_context: false,
            fast_path: None,
            alts: &alts,
        },
        0,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: None,
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &BTreeMap::new(),
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls: &[],
            atn_preferred_rule_calls: &[],
            adaptive_atn_preferred_rule_slots: &[],
            adaptive_atn_probe_rule_slots: &[],
        },
    );

    // The diagnostic-reporting branch (guard, lookahead, per-alt pushes, ambiguity record) is
    // one snapshot rather than six substring probes.
    insta::assert_snapshot!(
        "generated_semantic_decision_reports_filtered_ambiguity_diagnostics",
        rendered
    );
}

#[test]
fn generated_loop_filters_failed_leading_predicate_to_exit_alt() {
    let body = vec![
        GeneratedParserStep::Predicate {
            rule_index: 1,
            pred_index: 0,
        },
        mt(3, 4),
    ];
    let mut rendered = String::new();

    render_generated_star_loop(
        &mut rendered,
        StarLoopRender {
            state: 1,
            decision: 0,
            alts: (1, 2),
            track_alt_number: false,
            allow_semantic_context: true,
            force_context: false,
            plus_loop: false,
            fast_path: None,
            body: &body,
        },
        0,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: None,
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &BTreeMap::new(),
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls: &[],
            atn_preferred_rule_calls: &[],
            adaptive_atn_preferred_rule_slots: &[],
            adaptive_atn_probe_rule_slots: &[],
        },
    );

    // The whole rendered star-loop captures the leading-predicate-to-exit-alt filtering.
    insta::assert_snapshot!(
        "generated_loop_filters_failed_leading_predicate_to_exit_alt",
        rendered
    );
}

#[test]
fn generated_loop_filters_portable_local_predicate() {
    let body = vec![
        GeneratedParserStep::Predicate {
            rule_index: 1,
            pred_index: 0,
        },
        mt(3, 4),
    ];
    let declarations = vec![vec!["let mut __antlr_local_seen = false;".to_owned()]];
    let predicates = BTreeMap::from([((1, 0), ("__antlr_local_seen".to_owned(), None))]);
    let required_generated_rules = BTreeSet::from([1]);
    let mut rendered = String::new();

    render_generated_star_loop(
        &mut rendered,
        StarLoopRender {
            state: 1,
            decision: 0,
            alts: (1, 2),
            track_alt_number: false,
            allow_semantic_context: true,
            force_context: false,
            plus_loop: false,
            fast_path: None,
            body: &body,
        },
        0,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: Some(PortableLocalStepRender {
                declarations: &declarations,
                predicates: &predicates,
                required_generated_rules: &required_generated_rules,
            }),
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &BTreeMap::new(),
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls: &[],
            atn_preferred_rule_calls: &[],
            adaptive_atn_preferred_rule_slots: &[],
            adaptive_atn_probe_rule_slots: &[],
        },
    );

    // Snapshotting the whole loop captures that the portable local predicate is inlined
    // (`&& (__antlr_local_seen)`) and, crucially, that the SemIR hook path is NOT emitted — the
    // old `!contains("parser_semantic_ir_predicate_matches")` guard is now a visible absence.
    insta::assert_snapshot!("generated_loop_filters_portable_local_predicate", rendered);
}

#[test]
fn generated_loop_filters_first_nested_predicated_decision() {
    let body = vec![GeneratedParserStep::Decision {
        state: 1,
        decision: 0,
        track_alt_number: false,
        allow_semantic_context: true,
        force_context: false,
        fast_path: None,
        alts: vec![
            vec![mt(1, 4)],
            vec![mt(3, 4)],
            vec![
                GeneratedParserStep::Predicate {
                    rule_index: 2,
                    pred_index: 0,
                },
                mt(2, 4),
            ],
        ],
    }];
    let mut rendered = String::new();

    render_generated_star_loop(
        &mut rendered,
        StarLoopRender {
            state: 1,
            decision: 1,
            alts: (1, 2),
            track_alt_number: false,
            allow_semantic_context: true,
            force_context: false,
            plus_loop: false,
            fast_path: None,
            body: &body,
        },
        0,
        GeneratedStepRenderContext {
            current_rule_index: 0,
            embedded: None,
            portable_locals: None,
            decision_routing: DecisionRoutingRender::default(),
            inline_action_statements: &BTreeMap::new(),
            track_alt_numbers: false,
            track_context_alt_numbers: false,
            direct_generated_rule_calls: &[],
            atn_preferred_rule_calls: &[],
            adaptive_atn_preferred_rule_slots: &[],
            adaptive_atn_probe_rule_slots: &[],
        },
    );

    // The lookahead guard comes first so `&&` short-circuits on it before the predicate hook
    // runs; a non-matching lookahead must not trigger a fail-loud hit for an unknown/hook
    // predicate on a non-candidate alt. The whole rendered loop makes that guard ordering and
    // the nested-decision candidate condition visible in one snapshot.
    insta::assert_snapshot!(
        "generated_loop_filters_first_nested_predicated_decision",
        rendered
    );
}

#[test]
fn semantic_candidate_condition_guards_predicate_behind_lookahead() {
    // A leading predicate followed by a token match must render as
    // `lookahead && predicate`, so `&&` short-circuits on the side-effect-free
    // lookahead before invoking the predicate hook. Otherwise an alternative
    // whose first token cannot match the current lookahead would still
    // evaluate its hook/unknown predicate, recording a spurious fail-loud
    // `Unsupported` hit under `--sem-unknown=hook`/`error` and rejecting a
    // later syntactically viable alternative.
    let steps = vec![
        GeneratedParserStep::Predicate {
            rule_index: 2,
            pred_index: 0,
        },
        mt(7, 4),
    ];
    let condition = semantic_alt_candidate_condition(&steps, None, None);
    let la_at = condition
        .find("__semantic_la == 7")
        .expect("condition includes the leading lookahead guard");
    let pred_at = condition
        .find("parser_semantic_ir_predicate_matches_with_context_and_local")
        .expect("condition includes the leading predicate");
    assert!(
        la_at < pred_at,
        "lookahead must be evaluated before the predicate hook: {condition}"
    );
    assert!(
        condition.starts_with("__semantic_la == 7 &&"),
        "the lookahead guard must be the first `&&` operand: {condition}"
    );
}

#[test]
fn semantic_alt_guard_classifies_unresolved_rule_call_alt() {
    // An alt whose first consuming step is a rule call, with no leading
    // predicate or lookahead, is "unresolved" (FIRST set not computed here).
    let rule_call = vec![GeneratedParserStep::CallRule {
        source_state: 5,
        rule_index: 1,
        precedence: GeneratedRuleCallPrecedence::Literal(0),
    }];
    assert!(semantic_alt_guard_is_unresolved(&rule_call, None));
    // A token-led alt is resolved (concrete lookahead), so NOT unresolved.
    assert!(!semantic_alt_guard_is_unresolved(&[mt(7, 4)], None));
    // A predicate-led alt is resolved (guarded by the predicate).
    assert!(!semantic_alt_guard_is_unresolved(
        &[GeneratedParserStep::Predicate {
            rule_index: 2,
            pred_index: 0,
        }],
        None,
    ));
    // A pure epsilon alt (no consuming step) legitimately matches; not unresolved.
    assert!(!semantic_alt_guard_is_unresolved(&[], None));
}

#[test]
fn semantic_alt_search_orders_unresolved_alts_last() {
    // `{p()}? 'a' | x | 'a'` (alt 2 = rule call `x`): when alt 1's predicate
    // fails, the two-pass search tries resolved-guard alts first (alt 3's
    // concrete lookahead), then the unresolved rule-call alt as a last
    // resort. So alt 3 is NOT shadowed by alt 2, AND alt 2 stays reachable.
    let alts = vec![
        vec![
            GeneratedParserStep::Predicate {
                rule_index: 0,
                pred_index: 0,
            },
            mt(1, 4),
        ],
        vec![GeneratedParserStep::CallRule {
            source_state: 5,
            rule_index: 1,
            precedence: GeneratedRuleCallPrecedence::Literal(0),
        }],
        vec![mt(1, 4)],
    ];
    let alt_conditions = alts
        .iter()
        .map(|steps| semantic_alt_candidate_condition(steps, None, None))
        .collect::<Vec<_>>();
    let mut rendered = String::new();
    render_semantic_alt_search(&mut rendered, "", &alt_conditions, &alts, None);

    // The resolved token alt 3 is emitted before the unresolved rule-call
    // alt 2 (which keeps its real `true` condition as a last-resort branch),
    // so alt 3 wins on a matching lookahead but alt 2 is still reachable.
    let alt3_at = rendered
        .find("Some(3)")
        .expect("resolved token alt 3 is present");
    let alt2_at = rendered
        .find("Some(2)")
        .expect("unresolved rule-call alt 2 is still present (last resort)");
    assert!(
        alt3_at < alt2_at,
        "resolved alt 3 must be tried before the unresolved rule-call alt 2: {rendered}"
    );
    // Alt 2 is NOT disabled (`if false`); it keeps a reachable branch.
    assert!(
        !rendered.contains("if false {"),
        "unresolved alt must be a last-resort branch, not disabled: {rendered}"
    );
    assert!(
        rendered.contains("if __semantic_la == 1 {\n                Some(3)"),
        "the token alt keeps its concrete lookahead guard: {rendered}"
    );
}

#[test]
fn semantic_alt_search_keeps_lone_unresolved_alt_reachable() {
    // `{p()}? 'a' | x` (alt 2 = rule call `x`, the only non-predicated alt):
    // when alt 1's predicate fails and no resolved alt matches, the search
    // must still try the unresolved alt rather than emitting nothing (which
    // would be a spurious NoViableAlt). Codex's counter-example.
    let alts = vec![
        vec![
            GeneratedParserStep::Predicate {
                rule_index: 0,
                pred_index: 0,
            },
            mt(1, 4),
        ],
        vec![GeneratedParserStep::CallRule {
            source_state: 5,
            rule_index: 1,
            precedence: GeneratedRuleCallPrecedence::Literal(0),
        }],
    ];
    let alt_conditions = alts
        .iter()
        .map(|steps| semantic_alt_candidate_condition(steps, None, None))
        .collect::<Vec<_>>();
    let mut rendered = String::new();
    render_semantic_alt_search(&mut rendered, "", &alt_conditions, &alts, None);

    // The lone unresolved alt is reachable via its real condition, not disabled.
    assert!(
        rendered.contains("Some(2)") && !rendered.contains("if false {"),
        "a lone unresolved alt must remain reachable as a last resort: {rendered}"
    );
}

#[test]
fn parses_column_predicate_templates() {
    assert_eq!(
        parse_predicate_template(r#"<TokenStartColumnEquals("0")>"#),
        Some(PredicateTemplate::TokenStartColumnEquals(0))
    );
    assert_eq!(
        parse_predicate_template(r#"<Column()> \< 2"#),
        Some(PredicateTemplate::ColumnLessThan(2))
    );
    assert_eq!(
        parse_predicate_template("<Column()> >= 2"),
        Some(PredicateTemplate::ColumnGreaterOrEqual(2))
    );
}

#[test]
fn native_comparison_predicate_falls_through_instead_of_aborting() {
    // A native target-language comparison predicate merely contains a `<`
    // operator; it is not an ANTLR `<...>` StringTemplate, so it must fall
    // through to the unknown-predicate policy (as a generatable `Unknown`
    // lowering) rather than aborting codegen with "unsupported target
    // predicate template".
    let native = parser_fixture_data("native-comparison/T.g4");
    let templates = structural_predicate_templates(
        &native,
        SemanticsKind::ParserPredicate,
        &SemPatternFile::default(),
    )
    .expect("native comparison predicate must not abort generation");
    assert!(
        templates
            .iter()
            .all(|(_, template)| matches!(template, PredicateTemplate::Unknown)),
        "native `<` comparison lowers as Unknown, deferring to policy: {templates:?}"
    );
    assert!(!templates.is_empty());

    // A genuine untranslated `<...>` StringTemplate still errors.
    let unsupported = parser_fixture_data("unsupported-predicate-template/T.g4");
    let error = structural_predicate_templates(
        &unsupported,
        SemanticsKind::ParserPredicate,
        &SemPatternFile::default(),
    )
    .expect_err("an untranslated <...> StringTemplate predicate must still abort");
    assert!(
        error
            .to_string()
            .contains("unsupported target predicate template")
    );
}

#[test]
fn parses_predicate_fail_option_message() {
    let data = parser_fixture_data("predicate-fail/T.g4");
    let predicate = structural_predicates(&data)
        .expect("structural predicate should resolve")
        .into_iter()
        .next()
        .expect("fixture has one predicate");
    assert_eq!(predicate.fail, Some("custom message".to_owned()));
    assert_eq!(
        predicate_template_with_fail_message(PredicateTemplate::False, "custom message".to_owned(),),
        PredicateTemplate::FalseWithMessage {
            message: "custom message".to_owned()
        }
    );
    // A non-constant-false predicate (hook, member, lookahead, …) preserves
    // its `<fail=...>` message via the transparent `WithFailMessage` wrapper
    // rather than discarding it.
    let wrapped =
        predicate_template_with_fail_message(PredicateTemplate::Hook, "hook failed".to_owned());
    assert_eq!(
        wrapped,
        PredicateTemplate::WithFailMessage {
            inner: Box::new(PredicateTemplate::Hook),
            message: "hook failed".to_owned(),
        }
    );
    // The wrapper is transparent to evaluation and generatability, and it
    // exposes the message.
    assert_eq!(
        predicate_effective_template(&wrapped),
        &PredicateTemplate::Hook
    );
    assert!(can_generate_parser_predicate(&wrapped));
    assert_eq!(
        predicate_template_fail_message(&wrapped),
        Some("hook failed")
    );
    // Disposition follows the inner: a wrapped hook is still `Hooked`.
    assert_eq!(
        predicate_template_disposition(Some(&wrapped), SemUnknownPolicy::AssumeTrue),
        SemanticsDisposition::Hooked
    );
    let unknown = PredicateTemplate::UnknownWithFailMessage {
        message: "unknown failed".to_owned(),
    };
    assert_eq!(
        predicate_template_disposition(Some(&unknown), SemUnknownPolicy::AssumeFalse),
        SemanticsDisposition::AssumeFalse
    );
    assert_eq!(
        predicate_template_fail_message(&unknown),
        Some("unknown failed")
    );
    assert!(can_generate_parser_predicate(&unknown));
    // A later `<fail=...>` replaces the message rather than nesting wrappers.
    let rewrapped = predicate_template_with_fail_message(wrapped, "again".to_owned());
    assert_eq!(
        rewrapped,
        PredicateTemplate::WithFailMessage {
            inner: Box::new(PredicateTemplate::Hook),
            message: "again".to_owned(),
        }
    );
}

#[test]
fn parses_supported_predicate_helpers() {
    // Each supported helper input paired with the template it parses to; one snapshot pins the
    // whole recognition table (and which specialized parser owns each form) at once.
    let parsed = [
        (
            "invoke:true",
            parse_invoke_predicate(r#"True():Invoke_pred()"#),
        ),
        (
            "invoke:false",
            parse_invoke_predicate(r#"False():Invoke_pred()"#),
        ),
        (
            "parser_property_call",
            parse_predicate_template(r#"ParserPropertyCall({$parser}, "Property()")"#),
        ),
        ("literal_true", parse_predicate_template("true")),
        ("zero_eq_zero", parse_predicate_template("0==0")),
        ("zero_ne_zero", parse_predicate_template("0 != 0")),
        (
            "val_equals",
            parse_val_equals_predicate(r#"ValEquals("$i","2")"#),
        ),
        (
            "raw_local_int_le",
            parse_raw_local_int_less_or_equal_predicate("5 >= $_p"),
        ),
        (
            "foreign_predicate",
            parse_predicate_template("this.ForeignPredicate()"),
        ),
        (
            "foreign_context_check",
            parse_predicate_template("this.ForeignContextCheck()"),
        ),
    ];
    insta::assert_debug_snapshot!("parses_supported_predicate_helpers", parsed);
}

#[test]
fn semantic_patterns_lower_structural_parser_predicates() {
    let patterns = parse_sem_patterns(
        r#"
version = 1

[[helper]]
kind = "parser-predicate"
name = "tokensTouch"
returns = "bool"
lower = "token_index_adjacent"

[[helper]]
kind = "parser-predicate"
name = "isTyped"
returns = "bool"
lower = "cmp(ne, ctx_rule_text(local_type), str(\"var\"))"
"#,
    )
    .expect("pattern file parses");
    let lowered = [
        patterns
            .predicate_template(SemanticsKind::ParserPredicate, "this.tokensTouch()")
            .expect("pattern lookup succeeds"),
        patterns
            .predicate_template(SemanticsKind::ParserPredicate, "this.isTyped()")
            .expect("pattern lookup succeeds"),
    ];

    insta::assert_debug_snapshot!("structural_parser_predicate_patterns", lowered);
}

#[test]
fn maps_kotlin_rcurl_java_action_to_lexer_pop_mode() {
    for body in [
        "popMode()",
        "popMode();",
        "this.popMode()",
        "this.popMode();",
        "if (!_modeStack.isEmpty()) { popMode(); }",
        "if (!this._modeStack.isEmpty()) { popMode(); }",
        "if (!_modeStack.isEmpty()) popMode()",
        "if (!this._modeStack.isEmpty()) popMode();",
    ] {
        assert_eq!(
            parse_lexer_pop_mode_action(body),
            Some(ActionTemplate::LexerPopMode),
            "{body}"
        );
    }

    let action = parse_lexer_pop_mode_action("if (!_modeStack.isEmpty()) { popMode(); }")
        .expect("Kotlin RCURL action should lower");
    let method = render_lexer_action_method(&[((1, 0), action)]);
    assert!(method.contains("fn run_action"));
    assert!(method.contains("_base.pop_mode();"));
}

#[test]
fn embedded_lexer_semantics_are_translated_without_template_classification() {
    let data = lexer_fixture_data("embedded-lexer-semantics/L.g4");

    let entries = collect_lexer_semantics(
        &data,
        true,
        false,
        SemUnknownPolicy::Error,
        &SemPatternFile::default(),
    )
    .expect("embedded Rust bodies bypass portable template classification");

    insta::assert_debug_snapshot!("embedded_lexer_semantics", entries);
}

#[test]
fn unsupported_lexer_action_renders_todo_marker() {
    let data = lexer_fixture_data("unsupported-lexer-action/L.g4");
    let actions = structural_lexer_action_templates(&data, &SemPatternFile::default())
        .expect("structural lexer action should resolve")
        .into_iter()
        .map(|(_, action)| action)
        .collect::<Vec<_>>();

    assert_eq!(
        actions,
        [ActionTemplate::UnsupportedLexerAction {
            rule_name: "ID".to_owned(),
            body: "customJava();".to_owned(),
        }]
    );
    insta::assert_snapshot!(
        render_lexer_action_statement(&actions[0]),
        @"/* TODO unsupported embedded lexer action in rule ID: {customJava();}; rewrite target-specific actions as portable lexer commands where possible */"
    );
    let method = render_lexer_action_method(&[((1, 0), actions[0].clone())]);
    assert!(method.contains("TODO unsupported embedded lexer action in rule ID"));
    assert!(!method.contains("fn run_action"));
    assert_eq!(rust_block_comment_text("a */ b"), "a * / b");

    let error = reject_unsupported_lexer_action_templates(&actions, false).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains(
        "unsupported embedded lexer action in rule ID: {customJava();}; \
                 rewrite target-specific actions as portable lexer commands where possible"
    ));
    reject_unsupported_lexer_action_templates(&actions, true)
        .expect("unsupported-only lexer actions should be allowed in compatibility mode");
}

#[test]
fn mixed_supported_and_unsupported_lexer_actions_fail_even_when_allowed() {
    let actions = vec![
        ActionTemplate::UnsupportedLexerAction {
            rule_name: "ID".to_owned(),
            body: "setType(Foo);".to_owned(),
        },
        ActionTemplate::LexerPopMode,
    ];

    let error = reject_unsupported_lexer_action_templates(&actions, true).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains(
        "unsupported embedded lexer action in rule ID: {setType(Foo);}; \
                 rewrite target-specific actions as portable lexer commands where possible"
    ));
}

#[test]
fn lexer_action_diagnostic_summary_truncates_on_char_boundary() {
    let body = format!("{}\u{00e9} tail", "a".repeat(95));

    let summary = one_line_action_body(&body);

    assert_eq!(summary, format!("{}...", "a".repeat(95)));
}

fn linear_rule_atn() -> ParserAtn {
    let mut atn = ParserAtnBuilder::new(2);
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(0))
            .expect("state")
            .index(),
        0
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        1
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        2
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(0))
            .expect("state")
            .index(),
        3
    );
    atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(
        1,
        ParserTransitionSpec::Atom {
            target: 2,
            label: 1,
        },
    )
    .expect("transition");
    atn.add_transition(
        2,
        ParserTransitionSpec::Atom {
            target: 3,
            label: TOKEN_EOF,
        },
    )
    .expect("transition");
    atn.set_rule_to_start_state(vec![0])
        .expect("rule start states");
    atn.set_rule_to_stop_state(vec![3])
        .expect("rule stop states");
    finish_atn(atn)
}

fn block_decision_atn() -> ParserAtn {
    let mut atn = ParserAtnBuilder::new(2);
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(0))
            .expect("state")
            .index(),
        0
    );
    assert_eq!(
        atn.add_state(AtnStateKind::BlockStart, Some(0))
            .expect("state")
            .index(),
        1
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        2
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        3
    );
    assert_eq!(
        atn.add_state(AtnStateKind::BlockEnd, Some(0))
            .expect("state")
            .index(),
        4
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(0))
            .expect("state")
            .index(),
        5
    );
    atn.set_end_state(1, 4).expect("block end state");
    atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
        .expect("transition");
    atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 3 })
        .expect("transition");
    atn.add_transition(
        2,
        ParserTransitionSpec::Atom {
            target: 4,
            label: 1,
        },
    )
    .expect("transition");
    atn.add_transition(
        3,
        ParserTransitionSpec::Atom {
            target: 4,
            label: 2,
        },
    )
    .expect("transition");
    atn.add_transition(4, ParserTransitionSpec::Epsilon { target: 5 })
        .expect("transition");
    atn.add_decision_state(1).expect("decision state");
    atn.set_rule_to_start_state(vec![0])
        .expect("rule start states");
    atn.set_rule_to_stop_state(vec![5])
        .expect("rule stop states");
    finish_atn(atn)
}

fn star_loop_atn() -> ParserAtn {
    let mut atn = ParserAtnBuilder::new(2);
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(0))
            .expect("state")
            .index(),
        0
    );
    assert_eq!(
        atn.add_state(AtnStateKind::StarLoopEntry, Some(0))
            .expect("state")
            .index(),
        1
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        2
    );
    assert_eq!(
        atn.add_state(AtnStateKind::LoopEnd, Some(0))
            .expect("state")
            .index(),
        3
    );
    assert_eq!(
        atn.add_state(AtnStateKind::StarLoopBack, Some(0))
            .expect("state")
            .index(),
        4
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(0))
            .expect("state")
            .index(),
        5
    );
    atn.set_loop_back_state(3, 4).expect("loop back state");
    atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
        .expect("transition");
    atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 3 })
        .expect("transition");
    atn.add_transition(
        2,
        ParserTransitionSpec::Atom {
            target: 4,
            label: 1,
        },
    )
    .expect("transition");
    atn.add_transition(4, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(3, ParserTransitionSpec::Epsilon { target: 5 })
        .expect("transition");
    atn.add_decision_state(1).expect("decision state");
    atn.set_rule_to_start_state(vec![0])
        .expect("rule start states");
    atn.set_rule_to_stop_state(vec![5])
        .expect("rule stop states");
    finish_atn(atn)
}

fn plus_loop_atn() -> ParserAtn {
    let mut atn = ParserAtnBuilder::new(2);
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(0))
            .expect("state")
            .index(),
        0
    );
    assert_eq!(
        atn.add_state(AtnStateKind::PlusBlockStart, Some(0))
            .expect("state")
            .index(),
        1
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        2
    );
    assert_eq!(
        atn.add_state(AtnStateKind::BlockEnd, Some(0))
            .expect("state")
            .index(),
        3
    );
    assert_eq!(
        atn.add_state(AtnStateKind::PlusLoopBack, Some(0))
            .expect("state")
            .index(),
        4
    );
    assert_eq!(
        atn.add_state(AtnStateKind::LoopEnd, Some(0))
            .expect("state")
            .index(),
        5
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(0))
            .expect("state")
            .index(),
        6
    );
    atn.set_end_state(1, 3).expect("block end state");
    atn.set_loop_back_state(5, 4).expect("loop back state");
    atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
        .expect("transition");
    atn.add_transition(
        2,
        ParserTransitionSpec::Atom {
            target: 3,
            label: 1,
        },
    )
    .expect("transition");
    atn.add_transition(3, ParserTransitionSpec::Epsilon { target: 4 })
        .expect("transition");
    atn.add_transition(4, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(4, ParserTransitionSpec::Epsilon { target: 5 })
        .expect("transition");
    atn.add_transition(5, ParserTransitionSpec::Epsilon { target: 6 })
        .expect("transition");
    atn.add_decision_state(4).expect("decision state");
    atn.set_rule_to_start_state(vec![0])
        .expect("rule start states");
    atn.set_rule_to_stop_state(vec![6])
        .expect("rule stop states");
    finish_atn(atn)
}

fn plus_block_decision_atn() -> ParserAtn {
    let mut atn = ParserAtnBuilder::new(2);
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(0))
            .expect("state")
            .index(),
        0
    );
    assert_eq!(
        atn.add_state(AtnStateKind::PlusBlockStart, Some(0))
            .expect("state")
            .index(),
        1
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        2
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        3
    );
    assert_eq!(
        atn.add_state(AtnStateKind::BlockEnd, Some(0))
            .expect("state")
            .index(),
        4
    );
    assert_eq!(
        atn.add_state(AtnStateKind::PlusLoopBack, Some(0))
            .expect("state")
            .index(),
        5
    );
    assert_eq!(
        atn.add_state(AtnStateKind::LoopEnd, Some(0))
            .expect("state")
            .index(),
        6
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(0))
            .expect("state")
            .index(),
        7
    );
    atn.set_end_state(1, 4).expect("block end state");
    atn.set_loop_back_state(6, 5).expect("loop back state");
    atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 2 })
        .expect("transition");
    atn.add_transition(1, ParserTransitionSpec::Epsilon { target: 3 })
        .expect("transition");
    atn.add_transition(
        2,
        ParserTransitionSpec::Atom {
            target: 4,
            label: 1,
        },
    )
    .expect("transition");
    atn.add_transition(
        3,
        ParserTransitionSpec::Atom {
            target: 4,
            label: 2,
        },
    )
    .expect("transition");
    atn.add_transition(4, ParserTransitionSpec::Epsilon { target: 5 })
        .expect("transition");
    atn.add_transition(5, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(5, ParserTransitionSpec::Epsilon { target: 6 })
        .expect("transition");
    atn.add_transition(6, ParserTransitionSpec::Epsilon { target: 7 })
        .expect("transition");
    atn.add_decision_state(1).expect("decision state");
    atn.add_decision_state(5).expect("decision state");
    atn.set_rule_to_start_state(vec![0])
        .expect("rule start states");
    atn.set_rule_to_stop_state(vec![7])
        .expect("rule stop states");
    finish_atn(atn)
}

fn left_recursive_rule_atn() -> ParserAtn {
    let mut atn = ParserAtnBuilder::new(2);
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(0))
            .expect("state")
            .index(),
        0
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        1
    );
    assert_eq!(
        atn.add_state(AtnStateKind::StarLoopEntry, Some(0))
            .expect("state")
            .index(),
        2
    );
    assert_eq!(
        atn.add_state(AtnStateKind::StarBlockStart, Some(0))
            .expect("state")
            .index(),
        3
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        4
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        5
    );
    assert_eq!(
        atn.add_state(AtnStateKind::BlockEnd, Some(0))
            .expect("state")
            .index(),
        6
    );
    assert_eq!(
        atn.add_state(AtnStateKind::LoopEnd, Some(0))
            .expect("state")
            .index(),
        7
    );
    assert_eq!(
        atn.add_state(AtnStateKind::StarLoopBack, Some(0))
            .expect("state")
            .index(),
        8
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(0))
            .expect("state")
            .index(),
        9
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        10
    );
    atn.set_left_recursive_rule(0)
        .expect("left-recursive rule start");
    atn.set_precedence_rule_decision(2)
        .expect("precedence decision");
    atn.set_end_state(3, 6).expect("block end state");
    atn.set_loop_back_state(7, 8).expect("loop back state");
    atn.add_transition(0, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(
        1,
        ParserTransitionSpec::Atom {
            target: 2,
            label: 1,
        },
    )
    .expect("transition");
    atn.add_transition(2, ParserTransitionSpec::Epsilon { target: 3 })
        .expect("transition");
    atn.add_transition(2, ParserTransitionSpec::Epsilon { target: 7 })
        .expect("transition");
    atn.add_transition(3, ParserTransitionSpec::Epsilon { target: 4 })
        .expect("transition");
    atn.add_transition(
        4,
        ParserTransitionSpec::Precedence {
            target: 5,
            precedence: 2,
        },
    )
    .expect("transition");
    atn.add_transition(
        5,
        ParserTransitionSpec::Atom {
            target: 10,
            label: 2,
        },
    )
    .expect("transition");
    atn.add_transition(
        10,
        ParserTransitionSpec::Rule {
            target: 0,
            rule_index: 0,
            follow_state: 6,
            precedence: 3,
        },
    )
    .expect("transition");
    atn.add_transition(6, ParserTransitionSpec::Epsilon { target: 8 })
        .expect("transition");
    atn.add_transition(8, ParserTransitionSpec::Epsilon { target: 2 })
        .expect("transition");
    atn.add_transition(7, ParserTransitionSpec::Epsilon { target: 9 })
        .expect("transition");
    atn.add_decision_state(2).expect("decision state");
    atn.add_decision_state(3).expect("decision state");
    atn.set_rule_to_start_state(vec![0])
        .expect("rule start states");
    atn.set_rule_to_stop_state(vec![9])
        .expect("rule stop states");
    finish_atn(atn)
}

fn entry_candidate_atn() -> ParserAtn {
    let mut atn = ParserAtnBuilder::new(2);
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(0))
            .expect("state")
            .index(),
        0
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(0))
            .expect("state")
            .index(),
        1
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(0))
            .expect("state")
            .index(),
        2
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(1))
            .expect("state")
            .index(),
        3
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(1))
            .expect("state")
            .index(),
        4
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(2))
            .expect("state")
            .index(),
        5
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(2))
            .expect("state")
            .index(),
        6
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(2))
            .expect("state")
            .index(),
        7
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStart, Some(3))
            .expect("state")
            .index(),
        8
    );
    assert_eq!(
        atn.add_state(AtnStateKind::RuleStop, Some(3))
            .expect("state")
            .index(),
        9
    );
    assert_eq!(
        atn.add_state(AtnStateKind::Basic, Some(3))
            .expect("state")
            .index(),
        10
    );
    atn.add_transition(
        0,
        ParserTransitionSpec::Rule {
            target: 3,
            rule_index: 1,
            follow_state: 2,
            precedence: 0,
        },
    )
    .expect("transition");
    atn.add_transition(2, ParserTransitionSpec::Epsilon { target: 1 })
        .expect("transition");
    atn.add_transition(3, ParserTransitionSpec::Epsilon { target: 4 })
        .expect("transition");
    atn.add_transition(
        5,
        ParserTransitionSpec::Rule {
            target: 3,
            rule_index: 1,
            follow_state: 7,
            precedence: 0,
        },
    )
    .expect("transition");
    atn.add_transition(7, ParserTransitionSpec::Epsilon { target: 6 })
        .expect("transition");
    atn.add_transition(
        8,
        ParserTransitionSpec::Rule {
            target: 8,
            rule_index: 3,
            follow_state: 10,
            precedence: 0,
        },
    )
    .expect("transition");
    atn.add_transition(10, ParserTransitionSpec::Epsilon { target: 9 })
        .expect("transition");
    atn.set_rule_to_start_state(vec![0, 3, 5, 8])
        .expect("rule start states");
    atn.set_rule_to_stop_state(vec![1, 4, 6, 9])
        .expect("rule stop states");
    finish_atn(atn)
}

fn compile_test_fixture(relative: &str) -> &'static grammar::compiler::Compilation {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen")
        .join(relative);
    let compilation = grammar::compiler::compile(LoadOptions {
        roots: vec![root],
        library_directories: Vec::new(),
    })
    .unwrap_or_else(|error| panic!("test fixture {relative} should compile: {error:?}"));
    Box::leak(Box::new(compilation))
}

fn parser_fixture_data(relative: &str) -> ParserCodegenData<'static> {
    let compilation = compile_test_fixture(relative);
    let root = compilation
        .roots
        .first()
        .expect("test grammar has one root");
    let parser = compilation
        .parser(root.parser.expect("test grammar has a parser"))
        .expect("test parser artifact exists");
    ParserCodegenData::from_compiled(parser, &compilation.sources)
}

fn lexer_fixture_data(relative: &str) -> LexerCodegenData<'static> {
    let compilation = compile_test_fixture(relative);
    let root = compilation
        .roots
        .first()
        .expect("test grammar has one root");
    let lexer = compilation
        .lexer(root.lexer.expect("test grammar has a lexer"))
        .expect("test lexer artifact exists");
    LexerCodegenData::from_compiled(lexer, &compilation.sources)
}

fn minimal_parser_data() -> ParserCodegenData<'static> {
    parser_fixture_data("minimal/T.g4")
}

fn action_parser_data() -> ParserCodegenData<'static> {
    parser_fixture_data("parser-action/T.g4")
}

fn predicate_parser_data() -> ParserCodegenData<'static> {
    parser_fixture_data("predicate-parser/S.g4")
}

fn translated_predicate_parser_data() -> ParserCodegenData<'static> {
    parser_fixture_data("translated-predicate/S.g4")
}

fn predicate_lexer_data() -> LexerCodegenData<'static> {
    lexer_fixture_data("predicate-lexer/S.g4")
}

fn portable_bool_parser_data() -> ParserCodegenData<'static> {
    parser_fixture_data("portable-bool/S.g4")
}

#[test]
fn fixed_lookahead_classifier_tiers_thrift_like_decision() {
    let data = parser_fixture_data("fixed-lookahead/T.g4");

    // Flag off: Java-parity classification only. The `a` decision's
    // first two alternatives share 'ns', so it stays adaptive.
    let default_classification = classify_decisions(&data, None);
    assert!(default_classification.fixed_lookahead_tables.is_empty());
    assert_eq!(default_classification.adaptive_decisions.len(), 1);
    let adaptive_row = default_classification
        .report_rows
        .iter()
        .find(|row| matches!(row.tier, DecisionTierReport::Adaptive { .. }))
        .expect("one adaptive row");
    assert_eq!(
        adaptive_row.tier,
        DecisionTierReport::Adaptive {
            reason: AdaptiveReason::NotDisjoint,
            probed_lookahead: 1,
        }
    );

    // Flag on: the same decision is provably LL(2) and earns a table;
    // it stays in `adaptive_decisions` so its miss arm renders the
    // adaptive body it would have without the table.
    let classification = classify_decisions(&data, Some(2));
    assert_eq!(classification.fixed_lookahead_tables.len(), 1);
    let (decision, table) = classification
        .fixed_lookahead_tables
        .iter()
        .next()
        .expect("one fixed table");
    assert!(classification.adaptive_decisions.contains(decision));
    assert_eq!(table.lookahead, 2);
    insta::assert_debug_snapshot!("fixed_lookahead_thrift_like_table", table);

    // Deterministic and idempotent: same inputs, same classification.
    let again = classify_decisions(&data, Some(2));
    assert_eq!(
        format!("{:?}", classification.fixed_lookahead_tables),
        format!("{:?}", again.fixed_lookahead_tables)
    );
    assert_eq!(
        format!("{:?}", classification.report_rows),
        format!("{:?}", again.report_rows)
    );
}

#[test]
fn fixed_lookahead_manifest_reports_tiers() {
    let data = parser_fixture_data("fixed-lookahead/T.g4");
    let classification = classify_decisions(&data, Some(2));
    let manifest = render_decisions_manifest(
        Some(2),
        &[DecisionReportGrammar {
            name: "T".to_owned(),
            rule_names: data.rule_names.clone(),
            rows: classification.report_rows,
        }],
    );
    insta::assert_snapshot!("fixed_lookahead_decisions_manifest", manifest);

    // Flag-off and `--fixed-lookahead 1` emit different parsers (only
    // the latter compiles static LL(1) dispatch), so the manifest must
    // not conflate them: unset renders as null.
    let flag_off = render_decisions_manifest(None, &[]);
    assert!(flag_off.contains("\"fixedLookahead\": null"));
    let depth_one = render_decisions_manifest(Some(1), &[]);
    assert!(depth_one.contains("\"fixedLookahead\": 1"));
}

#[test]
fn complete_ll1_dispatches_do_not_emit_adaptive_fallbacks() {
    let data = parser_fixture_data("ll1-no-fallback/T.g4");
    let classification = classify_decisions(&data, Some(3));

    assert_eq!(classification.complete_ll1_dispatches.len(), 7);
    assert!(classification.adaptive_decisions.is_empty());
    insta::assert_debug_snapshot!(
        "complete_ll1_dispatches",
        classification.complete_ll1_dispatches
    );

    for (mode, options) in [
        ("plain", ParserRenderOptions::default()),
        (
            "embedded",
            ParserRenderOptions {
                embedded: true,
                ..ParserRenderOptions::default()
            },
        ),
        (
            "fixed",
            ParserRenderOptions {
                fixed_lookahead: Some(3),
                ..ParserRenderOptions::default()
            },
        ),
    ] {
        let rendered = render_parser_with_options("T", &data, options).expect("parser renders");
        assert!(
            !rendered.contains("adaptive_predict_stream_info_sll_probe("),
            "{mode} complete LL(1) decisions must not emit SLL fallback"
        );
        assert!(
            !rendered.contains("adaptive_predict_stream_info_with_context("),
            "{mode} complete LL(1) decisions must not emit full-context fallback"
        );
        assert!(
            !rendered.contains("ll1_decision_prediction("),
            "{mode} recovery misses must reuse the proven-complete dispatch"
        );
    }
}

#[test]
fn decision_manifest_reports_render_forced_adaptive_fallback() {
    let data = parser_fixture_data("decision-manifest-fallback/T.g4");
    let (rendered, rows) =
        render_parser_with_decision_report("T", &data, ParserRenderOptions::default())
            .expect("parser renders");
    let ll1 = rows
        .iter()
        .find(|row| row.decision == 0)
        .expect("nested LL(1) decision");

    assert_eq!(ll1.tier, DecisionTierReport::Ll1);
    assert_eq!(
        ll1.fallback,
        DecisionFallbackCapability::CanDefer,
        "render-forced full-context prediction must be visible in the manifest"
    );
    assert!(rendered.contains("adaptive_predict_stream_info_with_context(0, 0"));

    let manifest = render_decisions_manifest(
        None,
        &[DecisionReportGrammar {
            name: "T".to_owned(),
            rule_names: data.rule_names.clone(),
            rows,
        }],
    );
    insta::assert_snapshot!("render_forced_adaptive_decisions_manifest", manifest);
}

#[test]
fn fixed_lookahead_dispatch_commits_bare_and_syncs_on_miss() {
    let data = parser_fixture_data("fixed-lookahead/T.g4");
    let without_flag = render_parser("T", &data).expect("parser renders");
    assert!(!without_flag.contains("__fixed_lookahead_alt"));

    let rendered = render_parser_with_options(
        "T",
        &data,
        ParserRenderOptions {
            fixed_lookahead: Some(2),
            ..ParserRenderOptions::default()
        },
    )
    .expect("parser renders");
    let dispatch_start = rendered
        .find("let __fixed_lookahead_alt")
        .expect("fixed dispatch rendered");
    assert!(rendered.contains("match self.base.la(2)"));
    // A table hit commits without recovery synchronization — its arms
    // are restricted to lookahead where `sync_decision` provably
    // no-ops — so the decision's sync must render only in the miss
    // arm, after the dispatch.
    // Anchor at the enclosing generated method so the checked window is
    // defined by the generated structure (rule entry .. dispatch), not
    // an arbitrary byte count that preamble growth could outrun.
    let method_start = rendered[..dispatch_start]
        .rfind("fn parse_generated_rule_")
        .expect("dispatch is inside a generated rule method");
    assert!(!rendered[method_start..dispatch_start].contains("sync_decision"));
    assert!(rendered[dispatch_start..].contains("sync_decision"));
    assert!(
        !rendered.contains("adaptive_predict_stream_info_sll_probe(0, 0"),
        "the complete LL(1) loop must not emit an adaptive miss path"
    );
    assert!(
        rendered.contains("adaptive_predict_stream_info_sll_probe(1, 0"),
        "the fixed-LL(2) decision must retain its adaptive miss path"
    );
}

#[test]
fn fixed_lookahead_arms_stay_inside_sync_noop_lookahead() {
    // Every emitted dispatch arm must lie inside the decision's
    // within-rule lookahead: outside it, `sync_decision` performs real
    // recovery work (context-aware single-token deletion) that a bare
    // table hit would skip, moving error reports between rules.
    let data = parser_fixture_data("fixed-lookahead/T.g4");
    let atn = data.parser_atn().clone();
    let classification = classify_decisions(&data, Some(2));
    assert!(!classification.ll1_dispatch_tables.is_empty());
    let tables = classification
        .ll1_dispatch_tables
        .iter()
        .chain(classification.fixed_lookahead_tables.iter());
    for (decision, table) in tables {
        let state_number = atn
            .decision_to_state()
            .iter()
            .nth(*decision)
            .expect("decision state number");
        let state = atn.state(state_number).expect("decision state");
        let allowed = sync_noop_symbol_intervals(&atn, state);
        let FixedLookaheadNode::Probe(arms) = &table.root else {
            panic!("decision tables always probe la(1)");
        };
        for (intervals, _) in arms {
            assert_eq!(
                &intersect_interval_sets(intervals, &allowed),
                intervals,
                "decision {decision}: arm {intervals:?} escapes sync-no-op set {allowed:?}"
            );
        }
    }
}

#[test]
fn runtime_channel_names_are_dense_by_value() {
    // The semantic model keeps the `.interp`-shaped table (two `null`
    // placeholder rows before user channels); the generated runtime
    // metadata must instead match Java's dense `channelNames` array so
    // `channel_names[value]` resolves the declared name.
    let data = lexer_fixture_data("custom-channels/L.g4");
    assert_eq!(
        data.channel_names,
        [
            "DEFAULT_TOKEN_CHANNEL".to_owned(),
            "HIDDEN".to_owned(),
            "COMMENTS_AND_FORMATTING".to_owned(),
        ]
    );
    assert_eq!(data.channel_numbers["COMMENTS_AND_FORMATTING"], 2);
}

#[test]
fn set_lexer_predicate_template_replaces_or_appends() {
    // A per-coordinate override must WIN over a built-in translation, so
    // setting a covered coordinate replaces its template rather than adding
    // a duplicate arm; an uncovered coordinate is appended.
    let mut predicates = vec![((0, 0), PredicateTemplate::True)];
    set_lexer_predicate_template(&mut predicates, (0, 0), PredicateTemplate::False);
    assert_eq!(
        predicates,
        [((0, 0), PredicateTemplate::False)],
        "replaces covered"
    );

    set_lexer_predicate_template(&mut predicates, (1, 2), PredicateTemplate::True);
    assert_eq!(
        predicates,
        [
            ((0, 0), PredicateTemplate::False),
            ((1, 2), PredicateTemplate::True)
        ],
        "appends uncovered"
    );
}

#[test]
fn parser_action_assume_override_gets_noop_disposition() {
    // Assumed parser actions get explicit no-op arms; hook/error overrides
    // keep falling through to the parser action hook.
    let data = predicate_parser_data(); // rule 0 = "s"
    let mut action_state_coordinates = BTreeMap::new();
    action_state_coordinates.insert(4_usize, (0_usize, Some(0_usize)));

    for dispose in ["assume-true", "assume-false"] {
        let patterns = parse_sem_patterns(&format!(
                "version = 1\n[[coordinate]]\nkind = \"action\"\nrule = \"s\"\ndispose = \"{dispose}\"\n"
            ))
            .expect("pattern file parses");
        assert!(
            parser_action_assume_overridden(&patterns, &data, &action_state_coordinates, 4),
            "dispose {dispose}: an action state in rule `s` is assumed"
        );
    }
    let indexed_patterns = parse_sem_patterns(
            "version = 1\n[[coordinate]]\nkind = \"action\"\nrule = \"s\"\nindex = 0\ndispose = \"assume-true\"\n",
        )
        .expect("indexed pattern file parses");
    assert!(
        parser_action_assume_overridden(&indexed_patterns, &data, &action_state_coordinates, 4),
        "an index-specific assume override should suppress hook routing"
    );
    let other_index_patterns = parse_sem_patterns(
            "version = 1\n[[coordinate]]\nkind = \"action\"\nrule = \"s\"\nindex = 1\ndispose = \"assume-true\"\n",
        )
        .expect("other-index pattern file parses");
    assert!(
        !parser_action_assume_overridden(
            &other_index_patterns,
            &data,
            &action_state_coordinates,
            4
        ),
        "an override for another action index must not suppress this hook"
    );
    let hook_patterns = parse_sem_patterns(
        "version = 1\n[[coordinate]]\nkind = \"action\"\nrule = \"s\"\ndispose = \"hook\"\n",
    )
    .expect("pattern file parses");
    assert!(
        !parser_action_assume_overridden(&hook_patterns, &data, &action_state_coordinates, 4),
        "hook overrides should keep routing through the hook arm"
    );
    assert!(
        !parser_action_assume_overridden(
            &SemPatternFile::default(),
            &data,
            &action_state_coordinates,
            4
        ),
        "no override -> concrete arm is kept"
    );
}

#[test]
fn sem_pattern_file_lowers_exact_predicate_body() {
    let patterns = parse_sem_patterns(
        r#"
[[pattern]]
match = "isTypeName()"
lower = "bool(false)"
"#,
    )
    .expect("pattern file parses");
    let predicates = structural_predicate_templates(
        &predicate_parser_data(),
        SemanticsKind::ParserPredicate,
        &patterns,
    )
    .expect("pattern should lower predicate");

    assert_eq!(predicates[0].1, PredicateTemplate::False);
}

#[test]
fn strip_toml_comment_respects_quoted_strings() {
    // A `#` outside quotes starts a comment.
    assert_eq!(strip_toml_comment("key = 1 # trailing"), "key = 1 ");
    assert_eq!(strip_toml_comment("# whole line"), "");
    // A `#` inside a basic or literal string is NOT a comment.
    assert_eq!(
        strip_toml_comment(r#"match = "text == '#'""#),
        r#"match = "text == '#'""#
    );
    assert_eq!(
        strip_toml_comment(r##"lower = 'str("#")'"##),
        r##"lower = 'str("#")'"##
    );
    // A `#` after a closed string is still a comment.
    assert_eq!(
        strip_toml_comment(r#"match = "a" # note"#),
        r#"match = "a" "#
    );
    // A `\"` escape inside a basic string does not close it, so a later `#`
    // stays inside the string.
    assert_eq!(
        strip_toml_comment(r##"match = "a\"#b""##),
        r##"match = "a\"#b""##
    );
}

#[test]
fn sem_patterns_keep_hash_inside_quoted_match() {
    // A `#` inside the quoted `match` body must survive parsing, not be
    // truncated as a comment (which would silently change the pattern).
    let patterns = parse_sem_patterns(
        "[[pattern]]\nmatch = \"col == '#'\"  # a real comment\nlower = \"bool(false)\"\n",
    )
    .expect("pattern file parses");
    let template = patterns
        .predicate_template(SemanticsKind::ParserPredicate, "col == '#'")
        .expect("pattern lookup should not fail")
        .expect("the '#'-bearing match body must be retained");
    assert_eq!(template, PredicateTemplate::False);
}

/// Two `[[pattern]]` entries matching one action body must be an error, not
/// a first-wins pick: they lower to different mutations, so silently taking
/// the first would make merely reordering the pattern file change runtime
/// behavior. Predicate matching already rejects this; actions now match.
#[test]
fn ambiguous_member_action_patterns_are_rejected() {
    let patterns = parse_sem_patterns(
        r#"
[[member]]
name = "depths"
kind = "stack"

[[member]]
name = "other"
kind = "stack"

[[pattern]]
id = "first"
match = "depths.Push(1);"
lower = "push_member(depths, int(1))"

[[pattern]]
id = "second"
match = "depths.Push(1);"
lower = "push_member(other, int(99))"
"#,
    )
    .expect("pattern file parses");

    let error = patterns
        .member_action_stmt(SemanticsKind::LexerAction, "depths.Push(1);")
        .expect_err("an ambiguous action body must fail");
    insta::assert_snapshot!(
        error.to_string(),
        @r#"ambiguous semantic patterns for "depths.Push(1)": first, second"#
    );
}

/// One optional trailing `;` is the grammar's statement terminator, not part
/// of the body — the same rule `parse_semantic_helper_call` has always
/// applied to action bodies, so both matchers agree. Everything else stays
/// significant: a *run* of semicolons must not collapse, or two distinct
/// declared patterns would merge onto one lowering.
#[test]
fn member_action_match_normalizes_one_trailing_semicolon_only() {
    let patterns = parse_sem_patterns(
        r#"
[[member]]
name = "depths"
kind = "stack"

[[pattern]]
match = "depths.Pop()"
lower = "pop_member(depths)"
"#,
    )
    .expect("pattern file parses");

    for body in ["depths.Pop()", "depths.Pop();", "  depths.Pop() ; "] {
        assert!(
            patterns
                .member_action_stmt(SemanticsKind::LexerAction, body)
                .expect("lookup should not fail")
                .is_some(),
            "body {body:?} should match"
        );
    }

    // A second `;` is a different body, not the same one.
    assert!(
        patterns
            .member_action_stmt(SemanticsKind::LexerAction, "depths.Pop();;")
            .expect("lookup should not fail")
            .is_none(),
        "a run of semicolons must not collapse onto the declared body"
    );
}

#[test]
fn parse_toml_scalar_strips_single_quoted_literals() {
    // TOML literal strings are single-quoted with no escape processing.
    // `strip_toml_comment` already treats `'...'` as a string, so the scalar
    // parser must strip those quotes too (verbatim body).
    assert_eq!(parse_toml_scalar("'assume-false'"), "assume-false");
    assert_eq!(parse_toml_scalar("  'hook'  "), "hook");
    // A literal string is verbatim: a backslash is not an escape.
    assert_eq!(parse_toml_scalar(r"'a\nb'"), r"a\nb");
    // Double-quoted basic strings still unescape.
    assert_eq!(parse_toml_scalar(r#""a\nb""#), "a\nb");
    // A bare scalar and a lone quote are unchanged.
    assert_eq!(parse_toml_scalar("42"), "42");
    assert_eq!(parse_toml_scalar("'"), "'");
}

#[test]
fn sem_patterns_accept_single_quoted_dispose() {
    // A single-quoted (TOML literal) `dispose` must resolve to the disposition,
    // not keep its quotes (which would fail to match / be rejected).
    let patterns = parse_sem_patterns(
        "[[coordinate]]\nkind = 'predicate'\nrule = 's'\nindex = 0\ndispose = 'assume-false'\n",
    )
    .expect("pattern file parses");
    assert_eq!(
        patterns.coordinate_predicate_template(SemanticsKind::ParserPredicate, Some("s"), Some(0),),
        Some(Some(PredicateTemplate::False)),
        "single-quoted dispose must resolve to assume-false"
    );
}

#[test]
fn semantic_helper_calls_capture_kind_negation_and_literals() {
    let expected = Some(SemanticHelperCall {
        name: "isTypeName".to_owned(),
        arguments: Vec::new(),
        negated: false,
    });
    for body in ["isTypeName()", "this.isTypeName()", "self.isTypeName()"] {
        assert_eq!(
            parse_semantic_helper_call(body, SemanticsKind::ParserPredicate, None),
            expected,
            "receiver form {body:?} should parse"
        );
    }
    assert!(
        parse_semantic_helper_call(
            "recognizer.isTypeName()",
            SemanticsKind::ParserPredicate,
            None,
        )
        .is_none()
    );
    assert_eq!(
        parse_semantic_helper_call(
            "recognizer.isTypeName()",
            SemanticsKind::ParserPredicate,
            Some("recognizer"),
        ),
        expected
    );
    assert_eq!(
        parse_semantic_helper_call(
            r#"!this.matches("marker", true, -2)"#,
            SemanticsKind::LexerPredicate,
            None,
        ),
        Some(SemanticHelperCall {
            name: "matches".to_owned(),
            arguments: vec![
                SemanticLiteral::String("marker".to_owned()),
                SemanticLiteral::Bool(true),
                SemanticLiteral::Integer(-2),
            ],
            negated: true,
        })
    );
    assert_eq!(
        parse_semantic_helper_call("this.HandleAction();", SemanticsKind::LexerAction, None,),
        Some(SemanticHelperCall {
            name: "HandleAction".to_owned(),
            arguments: Vec::new(),
            negated: false,
        })
    );
    assert!(
        parse_semantic_helper_call("this.n(dynamicValue)", SemanticsKind::ParserPredicate, None,)
            .is_none()
    );
    assert_eq!(
        parse_semantic_helper_call(
            r"this.n('line\n\'quoted\'')",
            SemanticsKind::ParserPredicate,
            None,
        )
        .expect("single-quoted literal parses")
        .arguments,
        [SemanticLiteral::String("line\n'quoted'".to_owned())]
    );
}

#[test]
fn semantic_helper_patterns_are_scoped_by_kind_and_signature() {
    let patterns = parse_sem_patterns(
            "version = 1\n[[helper]]\nkind = \"lexer-action\"\nname = \"handle\"\nreturns = \"unit\"\nlower = \"hook\"\n[[helper]]\nname = \"n\"\nreceiver = \"recognizer\"\narguments = \"string\"\nreturns = \"bool\"\nlower = \"hook\"\n",
        )
        .expect("pattern file parses");
    assert!(
        patterns
            .hook_helper_call(SemanticsKind::LexerAction, "this.handle();")
            .expect("matching cannot fail")
            .is_some()
    );
    assert!(
        patterns
            .hook_helper_call(SemanticsKind::LexerPredicate, "this.handle()")
            .expect("matching cannot fail")
            .is_none()
    );
    let call = patterns
        .hook_helper_call(SemanticsKind::ParserPredicate, r#"this.n("value")"#)
        .expect("matching cannot fail")
        .expect("string argument matches");
    assert_eq!(
        call.arguments,
        [SemanticLiteral::String("value".to_owned())]
    );
    assert!(
        patterns
            .hook_helper_call(SemanticsKind::LexerPredicate, r#"this.n("value")"#)
            .expect("matching cannot fail")
            .is_some()
    );
    assert!(
        patterns
            .hook_helper_call(SemanticsKind::ParserPredicate, r#"recognizer.n("value")"#,)
            .expect("matching cannot fail")
            .is_some()
    );
    assert!(
        patterns
            .hook_helper_call(SemanticsKind::ParserPredicate, r#"other.n("value")"#)
            .expect("matching cannot fail")
            .is_none()
    );
    assert!(
        patterns
            .hook_helper_call(SemanticsKind::LexerAction, r#"this.n("value");"#)
            .expect("matching cannot fail")
            .is_none()
    );
}

#[test]
fn semantic_helper_patterns_reject_invalid_receiver_aliases() {
    let error = parse_sem_patterns(
            "version = 1\n[[helper]]\nname = \"n\"\nreceiver = \"recognizer.state\"\nreturns = \"bool\"\nlower = \"hook\"\n",
        )
        .expect_err("receiver aliases must be identifiers");
    insta::assert_snapshot!(
        error.to_string(),
        @r#"semantic helper receiver must be an identifier, got "recognizer.state""#
    );
}

#[test]
fn lexer_typed_hook_signatures_reject_normalized_conflicts() {
    let mappings = [
        LexerTypedHookMapping {
            rule_index: 0,
            coordinate_index: 0,
            kind: LexerTypedHookKind::Action,
            method_name: "handle_value".to_owned(),
            call: SemanticHelperCall {
                name: "HandleValue".to_owned(),
                arguments: vec![SemanticLiteral::String("x".to_owned())],
                negated: false,
            },
        },
        LexerTypedHookMapping {
            rule_index: 1,
            coordinate_index: 0,
            kind: LexerTypedHookKind::Action,
            method_name: "handle_value".to_owned(),
            call: SemanticHelperCall {
                name: "handle_value".to_owned(),
                arguments: vec![SemanticLiteral::Integer(1)],
                negated: false,
            },
        },
    ];

    let error = validate_lexer_typed_hook_signatures(&mappings)
        .expect_err("conflicting normalized signatures must fail generation");
    assert!(error.to_string().contains("conflicting literal signatures"));
}

#[test]
fn coordinate_override_routes_parser_predicate_to_typed_hook() {
    let patterns = parse_sem_patterns(
        r#"
[[coordinate]]
kind = "predicate"
rule = "s"
index = 0
dispose = "hook"
"#,
    )
    .expect("pattern file parses");
    let entries = collect_parser_semantics(
        &predicate_parser_data(),
        SemUnknownPolicy::AssumeTrue,
        &patterns,
    )
    .expect("collection should succeed");
    assert_eq!(entries[0].disposition, SemanticsDisposition::Hooked);

    let module = render_parser_with_options(
        "SParser",
        &predicate_parser_data(),
        ParserRenderOptions {
            patterns: Some(&patterns),
            ..ParserRenderOptions::default()
        },
    )
    .expect("parser should render");
    assert!(module.contains("pub trait SParserHooks"));
    assert!(module.contains("fn is_type_name"));
    assert!(module.contains("(0, 0) => Some(self.0.is_type_name(ctx))"));
    assert!(module.contains("PExpr::Hook"));
    // The typed action escape hatch must report handled actions: the trait's
    // `custom_action` returns `bool` and the adapter propagates it (so a
    // typed action hook satisfies a hook/error policy instead of being
    // treated as unhandled and failing loud).
    assert!(
        module.contains("_action: antlr4_runtime::ParserAction) -> bool"),
        "custom_action must return a handled-bool"
    );
    assert!(
        module.contains("self.0.custom_action(ctx, action)")
            && !module.contains("self.0.custom_action(ctx, action);"),
        "the adapter must return custom_action's result, not discard it"
    );
}

#[test]
fn typed_hook_mapping_skips_lexer_rule_predicates() {
    // A combined grammar owns lexer and parser predicates structurally;
    // only the parser predicate belongs in the parser hook adapter.
    let data = parser_fixture_data("mixed-parser-lexer-predicates/S.g4");
    let mappings = parser_typed_hook_mappings(&data, &SemPatternFile::default())
        .expect("typed hook mapping should succeed");

    // Only the parser-rule helper (`isTypeName` on rule `s`, pred 0) maps;
    // the lexer-rule `aheadIsDigit` helper is not wired to a parser hook.
    assert_eq!(
        mappings.len(),
        1,
        "only the parser-rule helper maps: {mappings:?}"
    );
    assert_eq!(
        (mappings[0].rule_index, mappings[0].coordinate_index),
        (0, 0)
    );
    assert_eq!(mappings[0].method_name, "is_type_name");
}

#[test]
fn typed_hook_predicate_method_name_avoids_action_hook_collision() {
    let mut mappings = [
        TypedHookMapping {
            rule_index: 0,
            coordinate_index: 0,
            kind: ParserTypedHookKind::Predicate,
            method_name: "custom_action".to_owned(),
            call: SemanticHelperCall {
                name: "customAction".to_owned(),
                arguments: Vec::new(),
                negated: false,
            },
        },
        TypedHookMapping {
            rule_index: 0,
            coordinate_index: 1,
            kind: ParserTypedHookKind::Predicate,
            method_name: "is_type_name".to_owned(),
            call: SemanticHelperCall {
                name: "isTypeName".to_owned(),
                arguments: Vec::new(),
                negated: false,
            },
        },
    ];
    disambiguate_parser_typed_hook_names(&mut mappings);
    assert_eq!(mappings[0].method_name, "custom_action_pred");
    assert_eq!(mappings[1].method_name, "is_type_name");
}

#[test]
fn typed_hook_action_method_names_remain_unique_after_suffixing() {
    let mut mappings = [
        TypedHookMapping {
            rule_index: 0,
            coordinate_index: 0,
            kind: ParserTypedHookKind::Action,
            method_name: "custom_action".to_owned(),
            call: SemanticHelperCall {
                name: "custom_action".to_owned(),
                arguments: Vec::new(),
                negated: false,
            },
        },
        TypedHookMapping {
            rule_index: 0,
            coordinate_index: 1,
            kind: ParserTypedHookKind::Action,
            method_name: "custom_action".to_owned(),
            call: SemanticHelperCall {
                name: "custom_action".to_owned(),
                arguments: Vec::new(),
                negated: false,
            },
        },
        TypedHookMapping {
            rule_index: 0,
            coordinate_index: 2,
            kind: ParserTypedHookKind::Action,
            method_name: "custom_action_action".to_owned(),
            call: SemanticHelperCall {
                name: "custom_action_action".to_owned(),
                arguments: Vec::new(),
                negated: false,
            },
        },
    ];

    disambiguate_parser_typed_hook_names(&mut mappings);

    insta::assert_debug_snapshot!(
        "typed_hook_action_method_names_remain_unique_after_suffixing",
        mappings
            .iter()
            .map(|mapping| (
                mapping.coordinate_index,
                mapping.kind,
                mapping.method_name.as_str(),
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn typed_hook_adapter_disambiguates_custom_action_helper() {
    // End-to-end: a bare `customAction()` predicate helper must not collide
    // with the fixed `custom_action` action-hook method on the trait.
    let data = parser_fixture_data("custom-action-predicate/S.g4");
    let mappings = parser_typed_hook_mappings(&data, &SemPatternFile::default())
        .expect("typed hook mapping should succeed");
    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings[0].method_name, "custom_action_pred");

    let adapter = render_typed_hook_adapter("SParser", &mappings);
    // The predicate method is the disambiguated name; the action hook keeps
    // the reserved name — two distinct methods, so the trait compiles.
    assert!(adapter.contains("fn custom_action_pred<L>"));
    assert!(adapter.contains(
            "fn custom_action<L>(&mut self, _ctx: &mut antlr4_runtime::ParserSemCtx<'_, L>, _action: antlr4_runtime::ParserAction) -> bool"
        ));
    assert!(adapter.contains("Some(self.0.custom_action_pred(ctx))"));
}

#[test]
fn manifest_predicate_provenance_skips_lexer_rule_block() {
    // A combined grammar's parser manifest must use the parser predicate's
    // structural provenance, never a lexer predicate's body.
    let data = parser_fixture_data("mixed-parser-lexer-predicates/S.g4");
    let entries = collect_parser_semantics(
        &data,
        SemUnknownPolicy::AssumeTrue,
        &SemPatternFile::default(),
    )
    .expect("collection should succeed");

    let predicate = entries
        .iter()
        .find(|entry| entry.kind == SemanticsKind::ParserPredicate)
        .expect("parser predicate coordinate present");
    assert_eq!(predicate.rule_name.as_deref(), Some("s"));
    assert_eq!(
        predicate.body.as_deref(),
        Some("isTypeName()"),
        "provenance must be the parser predicate body, not the lexer-rule aheadIsDigit()"
    );
}

#[test]
fn require_full_semantics_rejects_policy_fallbacks_but_allows_hooks() {
    let fallback = collect_parser_semantics(
        &predicate_parser_data(),
        SemUnknownPolicy::AssumeTrue,
        &SemPatternFile::default(),
    )
    .expect("collection should succeed");
    assert!(enforce_require_full_semantics(true, &fallback).is_err());

    let mut hooked = fallback;
    hooked[0].disposition = SemanticsDisposition::Hooked;
    enforce_require_full_semantics(true, &hooked).expect("hooked coordinates are complete");
}

#[test]
fn collect_parser_semantics_inventories_untranslated_predicates() {
    let entries = collect_parser_semantics(
        &predicate_parser_data(),
        SemUnknownPolicy::AssumeTrue,
        &SemPatternFile::default(),
    )
    .expect("collection should succeed");

    insta::assert_debug_snapshot!("untranslated_parser_predicate_semantics", entries);
}

#[test]
fn collect_parser_semantics_marks_supported_predicates_translated() {
    let entries = collect_parser_semantics(
        &translated_predicate_parser_data(),
        SemUnknownPolicy::AssumeTrue,
        &SemPatternFile::default(),
    )
    .expect("collection should succeed");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].disposition, SemanticsDisposition::Translated);
    assert_eq!(entries[0].template.as_deref(), Some("True"));
}

#[test]
fn collect_parser_semantics_marks_helper_hooked_predicates_hooked() {
    let patterns = parse_sem_patterns(
        "version = 1\n\n[[helper]]\nname = \"isTypeName\"\nreturns = \"bool\"\nlower = \"hook\"\n",
    )
    .expect("pattern file should parse");
    let entries =
        collect_parser_semantics(&predicate_parser_data(), SemUnknownPolicy::Error, &patterns)
            .expect("collection should succeed");

    assert_eq!(entries.len(), 1);
    // A helper routed to the hook trait is accounted for, but it is NOT a
    // translation: the manifest must say `hooked` so users know the
    // coordinate needs a runtime hook implementation.
    assert_eq!(entries[0].disposition, SemanticsDisposition::Hooked);
    enforce_sem_unknown(SemUnknownPolicy::Error, &entries)
        .expect("hooked coordinates satisfy strict mode");
}

#[test]
fn enforce_sem_unknown_error_lists_untranslated_coordinates() {
    let entries = collect_parser_semantics(
        &predicate_parser_data(),
        SemUnknownPolicy::Error,
        &SemPatternFile::default(),
    )
    .expect("collection should succeed");

    let error = enforce_sem_unknown(SemUnknownPolicy::Error, &entries)
        .expect_err("untranslated predicate must fail generation");
    let message = error.to_string();
    insta::assert_snapshot!("untranslated_parser_predicate_error", message);
}

#[test]
fn enforce_sem_unknown_error_accepts_fully_translated_grammars() {
    let entries = collect_parser_semantics(
        &translated_predicate_parser_data(),
        SemUnknownPolicy::Error,
        &SemPatternFile::default(),
    )
    .expect("collection should succeed");

    enforce_sem_unknown(SemUnknownPolicy::Error, &entries)
        .expect("fully translated grammar passes strict mode");
}

#[test]
fn authored_empty_parser_action_is_explicit_noop() {
    let data = parser_fixture_data("empty-parser-action/T.g4");
    let entries =
        collect_parser_semantics(&data, SemUnknownPolicy::Error, &SemPatternFile::default())
            .expect("collection should succeed");
    let action = entries
        .iter()
        .find(|entry| entry.kind == SemanticsKind::ParserAction)
        .expect("action entry is inventoried");

    let state = action
        .atn_state
        .expect("action state is bound structurally");
    assert_eq!(action.body.as_deref(), Some(""));
    assert_eq!(action.disposition, SemanticsDisposition::Synthetic);
    enforce_sem_unknown(SemUnknownPolicy::Error, &entries)
        .expect("empty authored action is a strict-policy no-op");

    let module = render_parser_with_options(
        "TParser",
        &data,
        ParserRenderOptions {
            require_generated_parser: true,
            embedded: false,
            sem_unknown: SemUnknownPolicy::Error,
            patterns: None,
            ..ParserRenderOptions::default()
        },
    )
    .expect("empty action should not block generated parser output");
    assert!(module.contains(&format!("{state} => {{}}")));
}

#[test]
fn parser_action_attribution_ignores_non_embedded_member_syntax() {
    let data = parser_fixture_data("member-and-parser-action/T.g4");
    let entries = collect_parser_semantics(
        &data,
        SemUnknownPolicy::AssumeTrue,
        &SemPatternFile::default(),
    )
    .expect("Java-style members are irrelevant to action attribution");
    let action = entries
        .iter()
        .find(|entry| entry.kind == SemanticsKind::ParserAction)
        .expect("action entry is inventoried");

    assert!(action.atn_state.is_some());
    assert_eq!(action.body.as_deref(), Some("native();"));
}

#[test]
fn enforce_sem_unknown_error_exempts_synthetic_actions() {
    // A `Synthetic` action (ANTLR-inserted, e.g. LR elimination) must NOT
    // fail under the Error policy: there is no author intent to implement.
    let synthetic = SemanticsEntry {
        kind: SemanticsKind::ParserAction,
        rule_index: Some(1),
        rule_name: Some("declarator".to_owned()),
        index: None,
        atn_state: Some(9),
        line: None,
        column: None,
        body: None,
        disposition: SemanticsDisposition::Synthetic,
        template: None,
    };
    enforce_sem_unknown(SemUnknownPolicy::Error, std::slice::from_ref(&synthetic))
        .expect("a synthetic action is exempt from the error gate");
    // ...but an authored untranslated action (Ignored) at the same shape must fail.
    let authored = SemanticsEntry {
        disposition: SemanticsDisposition::Ignored,
        ..synthetic
    };
    enforce_sem_unknown(SemUnknownPolicy::Error, std::slice::from_ref(&authored))
        .expect_err("an authored untranslated action must fail loud under error");
}

#[test]
fn enforce_sem_unknown_is_lenient_under_default_policy() {
    let entries = collect_parser_semantics(
        &predicate_parser_data(),
        SemUnknownPolicy::AssumeTrue,
        &SemPatternFile::default(),
    )
    .expect("collection should succeed");

    enforce_sem_unknown(SemUnknownPolicy::AssumeTrue, &entries)
        .expect("assume-true keeps the historical lenient behavior");
}

#[test]
fn enforce_sem_unknown_fails_per_coordinate_error_under_default_policy() {
    // A per-coordinate `dispose = "error"` override must fail codegen even
    // when the global policy is the lenient default; otherwise the
    // coordinate lowers to no SemIR entry and silently falls back to
    // AssumeTrue at runtime.
    let patterns = parse_sem_patterns(
            "version = 1\n[[coordinate]]\nkind = \"predicate\"\nrule = \"s\"\nindex = 0\ndispose = \"error\"\n",
        )
        .expect("pattern file parses");
    let entries = collect_parser_semantics(
        &predicate_parser_data(),
        SemUnknownPolicy::AssumeTrue,
        &patterns,
    )
    .expect("collection should succeed");

    let error = enforce_sem_unknown(SemUnknownPolicy::AssumeTrue, &entries)
        .expect_err("per-coordinate error override must fail even under assume-true");
    assert!(
        error.to_string().contains("pred_index=0"),
        "message should name the rejected coordinate: {error}"
    );
}

#[test]
fn semantics_manifest_renders_coordinates_and_policy() {
    let entries = collect_parser_semantics(
        &predicate_parser_data(),
        SemUnknownPolicy::AssumeTrue,
        &SemPatternFile::default(),
    )
    .expect("collection should succeed");
    let manifest = render_semantics_manifest(
        SemUnknownPolicy::AssumeTrue,
        &[],
        &[("parser", "SParser".to_owned(), entries)],
    );

    insta::assert_snapshot!("semantics_manifest_with_untranslated_predicate", manifest);
}

#[test]
fn translates_portable_boolean_local_semantics() {
    let data = portable_bool_parser_data();
    let portable = build_structural_portable_local_data(&data, &SemPatternFile::default())
        .expect("portable local semantics should build");

    // declarations, required_generated_rules, inline_actions (keyed by action state), and
    // predicates all land in one snapshot; the BTreeMap/BTreeSet backing keeps it deterministic.
    insta::assert_debug_snapshot!("translates_portable_boolean_local_semantics", portable);

    let module = render_parser("SParser", &data).expect("portable grammar should render");
    assert!(module.contains("let mut __antlr_local_seen = false;"));
    assert!(module.contains("__antlr_local_seen = true;"));
    assert!(module.contains("if !(__antlr_local_seen) {"));
    assert!(module.contains("failed_predicate_option_error(0, \"not seen\".to_owned())"));

    let entries =
        collect_parser_semantics(&data, SemUnknownPolicy::Error, &SemPatternFile::default())
            .expect("portable coordinates should be inventoried");
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .all(|entry| entry.disposition == SemanticsDisposition::Translated)
    );
    assert!(
        entries
            .iter()
            .all(|entry| entry.template.as_deref() == Some("PortableBooleanLocal"))
    );
    enforce_sem_unknown(SemUnknownPolicy::Error, &entries)
        .expect("portable coordinates satisfy strict semantics");
}

#[test]
fn indexed_action_overrides_precede_portable_boolean_lowering() {
    let data = portable_bool_parser_data();
    let action = structural_actions(&data)
        .expect("portable action inventory should build")
        .into_iter()
        .next()
        .expect("portable fixture has one action");

    for dispose in ["assume-true", "assume-false", "hook"] {
        let patterns = parse_sem_patterns(&format!(
                "version = 1\n[[coordinate]]\nkind = \"action\"\nrule = \"s\"\nindex = {}\ndispose = \"{dispose}\"\n",
                action.action_index
            ))
            .expect("indexed action override should parse");
        let portable = build_structural_portable_local_data(&data, &patterns)
            .expect("portable local semantics should build");

        assert!(
            portable.inline_actions.is_empty(),
            "indexed {dispose} override must suppress portable action lowering"
        );
    }
}

#[test]
fn semantics_manifest_renders_empty_inventory() {
    let manifest = render_semantics_manifest(
        SemUnknownPolicy::AssumeTrue,
        &[],
        &[("parser", "SParser".to_owned(), Vec::new())],
    );

    assert!(manifest.contains("\"coordinates\": []"));
}

#[test]
fn assume_false_policy_reaches_generated_runtime_options() {
    let module = render_parser_with_options(
        "SParser",
        &predicate_parser_data(),
        ParserRenderOptions {
            require_generated_parser: false,
            embedded: false,
            sem_unknown: SemUnknownPolicy::AssumeFalse,
            patterns: None,
            ..ParserRenderOptions::default()
        },
    )
    .expect("parser should render");

    assert!(
        module.contains(
            "unknown_predicate_policy: antlr4_runtime::UnknownSemanticPolicy::AssumeFalse"
        )
    );
}

#[test]
fn generated_top_level_entry_surfaces_unknown_semantic_error() {
    // The public generated entry must surface Error-policy coordinates the
    // generated-direct predicate path recorded, or a parse that consulted an
    // unimplemented hook predicate returns a recovered Ok tree instead of
    // AntlrError::Unsupported.
    let module = render_parser_with_options(
        "SParser",
        &predicate_parser_data(),
        ParserRenderOptions {
            require_generated_parser: false,
            embedded: false,
            sem_unknown: SemUnknownPolicy::AssumeFalse,
            patterns: None,
            ..ParserRenderOptions::default()
        },
    )
    .expect("parser should render");
    let surface_at = module
        .find("if let Some(error) = self.base.take_unknown_semantic_error()")
        .expect("generated top-level entry must surface recorded unknown-semantic coordinates");
    // Guarded by the public entry only, not the nested (from-generated) path.
    assert!(module.contains("if allow_generated_fallback {"));
    // The fail-loud check must run before the generated entry can return Ok.
    let ok_at = module[surface_at..]
        .find("Ok(__tree)")
        .map(|offset| surface_at + offset)
        .expect("generated entry returns the tree after semantic checks");
    assert!(surface_at < ok_at);
}

#[test]
fn generated_rule_error_drains_diagnostics_before_recorded_overrides() {
    // When a generated-direct predicate consulted an unimplemented hook
    // (returning None under the Error policy), the alternative fails and
    // `parse_generated_rule` returns a generic `failed_predicate_error`. The
    // top-level `Err` arm must first drain any recorded fail-loud coordinate
    // and return that `AntlrError::Unsupported`, otherwise the documented
    // fail-loud error is shadowed by the generic rule error. The check is
    // gated on `allow_generated_fallback` so a nested child keeps its hits for
    // the generated parent to surface at its own boundary.
    let module = render_parser_with_options(
        "SParser",
        &predicate_parser_data(),
        ParserRenderOptions {
            require_generated_parser: false,
            embedded: false,
            sem_unknown: SemUnknownPolicy::Hook,
            patterns: None,
            ..ParserRenderOptions::default()
        },
    )
    .expect("parser should render");

    // Locate the generated-rule `Err` arm's generic return.
    let error_conversion_at = module
        .find("let error = error.into_error();")
        .expect("generated-rule Err arm converts the generic rule error");
    let generic_return_at = module[error_conversion_at..]
        .find("return Err(error);")
        .map(|offset| error_conversion_at + offset)
        .expect("generated-rule Err arm returns the generic rule error");
    // The fail-loud drain must appear inside that arm, before the generic
    // return, under the top-level gate.
    let arm_start = module[..generic_return_at]
        .rfind("Err(error) => {")
        .expect("generic return lives in the Err arm");
    let arm = &module[arm_start..generic_return_at];
    let diagnostics_at = arm
        .find("self.base.report_generated_parser_diagnostics();")
        .expect("the fatal Err arm drains retained diagnostics");
    let semantic_at = arm
        .find("if let Some(semantic_error) = self.base.take_unknown_semantic_error()")
        .expect("the Err arm drains a recorded semantic error");
    let abort_at = arm
        .find("if let Some(abort) = self.base.take_parse_abort()")
        .expect("the Err arm drains a recorded parser abort");
    assert!(
        diagnostics_at < abort_at && abort_at < semantic_at,
        "retained diagnostics must dispatch first, then parser aborts must precede semantic misses"
    );
}

#[test]
fn interpreted_fallback_action_miss_is_surfaced_at_public_entry() {
    // A public entry that falls back to the interpreted ATN path runs the
    // non-buffered `run_action` loop immediately (an untranslated action
    // routed to `parser_action_hook` records an `unhandled_action_hit` under
    // the Error policy). The top-level surfacing check that drains those hits
    // is gated on the SAME `allow_generated_fallback` condition as the branch
    // that runs the interpreted fallback, so the miss cannot escape as `Ok`.
    // (Verified end-to-end: parsing an untranslated action through the
    // interpreted path under `--sem-unknown=hook` with a declining hook
    // returns `AntlrError::Unsupported("unhandled semantic action: ...")`.)
    //
    // Emitting a *separate* check inside `parse_interpreted_rule_precedence`
    // would be both dead (the outer check already drains the hits) and
    // unsafe: an early `return Err` there would bypass the caller's ordinary
    // generated-rule cleanup and error reporting path.
    let module = render_parser_with_options(
        "SParser",
        &predicate_parser_data(),
        ParserRenderOptions {
            require_generated_parser: false,
            embedded: false,
            sem_unknown: SemUnknownPolicy::Hook,
            patterns: None,
            ..ParserRenderOptions::default()
        },
    )
    .expect("parser should render");

    // The interpreted call site in the top-level entry and the surfacing check
    // share the `allow_generated_fallback` gate, so the surfacing check follows
    // the interpreted call and drains any action-hook miss (or predicate miss)
    // the fallback recorded.
    let interpreted_call_at = module
        .find("self.parse_interpreted_rule_precedence(rule_index, precedence)?")
        .expect("top-level entry runs the interpreted fallback under allow_generated_fallback");
    let surface_at = module[interpreted_call_at..]
        .find("if let Some(error) = self.base.take_unknown_semantic_error()")
        .map(|offset| interpreted_call_at + offset)
        .expect("the public entry must drain recorded semantic misses after the fallback");
    // Between the interpreted fallback call and the surfacing check the entry
    // must not return `Ok`, or an action-hook miss recorded by the immediate
    // `run_action` loop would escape as a recovered success.
    assert!(
        !module[interpreted_call_at..surface_at].contains("Ok(__tree)"),
        "the entry must not return Ok between the interpreted fallback and the surfacing check"
    );
    // Both are under the same gate: the branch that runs the fallback and the
    // check that drains its misses share `if allow_generated_fallback {`.
    assert!(
        module[..interpreted_call_at].contains("} else {"),
        "the interpreted fallback remains the non-generated fallback path"
    );
}

#[test]
fn hook_predicate_does_not_escalate_default_policy() {
    // A per-coordinate `dispose = "hook"` predicate under the default global
    // policy must NOT flip the whole parser to Error — that would turn
    // unrelated `assume-true` coordinates into fail-loud. The hook falls
    // through to the configured (default) policy per coordinate; users opt
    // into fail-loud with --sem-unknown=error.
    let patterns = parse_sem_patterns(
            "version = 1\n[[coordinate]]\nkind = \"predicate\"\nrule = \"s\"\nindex = 0\ndispose = \"hook\"\n",
        )
        .expect("pattern file parses");
    let module = render_parser_with_options(
        "SParser",
        &predicate_parser_data(),
        ParserRenderOptions {
            require_generated_parser: false,
            embedded: false,
            sem_unknown: SemUnknownPolicy::AssumeTrue,
            patterns: Some(&patterns),
            ..ParserRenderOptions::default()
        },
    )
    .expect("parser should render");

    assert!(
        !module.contains("UnknownSemanticPolicy::Error"),
        "a hook predicate must not escalate the default policy to Error"
    );
}

#[test]
fn non_default_policy_installs_on_generated_parser_constructor() {
    // The generated-direct predicate path reads BaseParser's
    // `unknown_predicate_policy`, which the interpreter options never set on
    // that path. The constructor must install a non-default policy so a
    // generated rule's hook predicate honors --sem-unknown instead of the
    // AssumeTrue default.
    for (policy, literal) in [
        (
            SemUnknownPolicy::AssumeFalse,
            "antlr4_runtime::UnknownSemanticPolicy::AssumeFalse",
        ),
        (
            SemUnknownPolicy::Error,
            "antlr4_runtime::UnknownSemanticPolicy::Error",
        ),
    ] {
        let module = render_parser_with_options(
            "SParser",
            &predicate_parser_data(),
            ParserRenderOptions {
                require_generated_parser: false,
                embedded: false,
                sem_unknown: policy,
                patterns: None,
                ..ParserRenderOptions::default()
            },
        )
        .expect("parser should render");
        assert!(
            module.contains(&format!("base.set_unknown_predicate_policy({literal});")),
            "policy {policy:?} must be installed on the generated constructor"
        );
    }

    // The default policy leaves the constructor untouched (no needless call).
    let default_module = render_parser_with_options(
        "SParser",
        &predicate_parser_data(),
        ParserRenderOptions::default(),
    )
    .expect("parser should render");
    assert!(!default_module.contains("set_unknown_predicate_policy"));
}

#[test]
fn default_policy_emits_assume_true_options_field() {
    let module = render_parser_with_options(
        "SParser",
        &translated_predicate_parser_data(),
        ParserRenderOptions::default(),
    )
    .expect("parser should render");

    assert!(
        module.contains(
            "unknown_predicate_policy: antlr4_runtime::UnknownSemanticPolicy::AssumeTrue"
        )
    );
}

#[test]
fn non_default_policy_disables_adaptive_direct_gate() {
    // A grammar with no predicates/actions would normally allow the
    // adaptive-direct shortcut, but that path drops the emitted
    // ParserRuntimeOptions (and thus the policy). The gate must be disabled
    // so a non-default policy always reaches the options-carrying call.
    let default_module = render_parser_with_options("TParser", &minimal_parser_data(), {
        ParserRenderOptions::default()
    })
    .expect("parser should render under default policy");
    assert!(
        default_module.contains("&& true && std::env::var_os(\"ANTLR4_RUST_ADAPTIVE_DIRECT\")"),
        "the predicate-free fixture must allow adaptive-direct by default, or this test proves nothing"
    );

    for policy in [SemUnknownPolicy::AssumeFalse, SemUnknownPolicy::Error] {
        let module = render_parser_with_options(
            "TParser",
            &minimal_parser_data(),
            ParserRenderOptions {
                require_generated_parser: false,
                embedded: false,
                sem_unknown: policy,
                patterns: None,
                ..ParserRenderOptions::default()
            },
        )
        .expect("parser should render under a non-default policy");
        assert!(
            module.contains("&& false && std::env::var_os(\"ANTLR4_RUST_ADAPTIVE_DIRECT\")"),
            "policy {policy:?} must disable the adaptive-direct gate"
        );
    }
}

#[test]
fn lexer_assume_false_policy_renders_failing_predicate_hook() {
    let module = render_lexer(
        "SLexer",
        &predicate_lexer_data(),
        false,
        SemUnknownPolicy::AssumeFalse,
        &SemPatternFile::default(),
        false,
    )
    .expect("lexer should render");

    assert!(module.contains("next_token_compiled_with_hooks"));
    assert!(module.contains("|_, _| false"));
}

#[test]
fn lexer_per_coordinate_hook_override_routes_to_owned_hooks() {
    let patterns = parse_sem_patterns(
        "version = 1\n[[coordinate]]\nkind = \"lexer-predicate\"\nindex = 0\ndispose = \"hook\"\n",
    )
    .expect("pattern file parses");
    let module = render_lexer(
        "SLexer",
        &predicate_lexer_data(),
        false,
        SemUnknownPolicy::AssumeTrue,
        &patterns,
        false,
    )
    .expect("per-coordinate hooks should render generated lexer plumbing");
    assert!(module.contains("next_token_compiled_with_semantic_dispatch"));
    assert!(module.contains("=> { None }"));
}

#[test]
fn lexer_per_coordinate_assume_false_override_renders_failing_arm() {
    // A per-coordinate `dispose = "assume-false"` on an uncovered lexer
    // predicate must render an explicit failing `run_predicate` arm and take
    // the hook-taking token path even under the default global policy, so
    // the override recorded in the manifest actually removes the guarded
    // alternative at runtime.
    let patterns = parse_sem_patterns(
            "version = 1\n[[coordinate]]\nkind = \"lexer-predicate\"\nindex = 0\ndispose = \"assume-false\"\n",
        )
        .expect("pattern file parses");
    let module = render_lexer(
        "SLexer",
        &predicate_lexer_data(),
        false,
        SemUnknownPolicy::AssumeTrue,
        &patterns,
        false,
    )
    .expect("lexer should render");

    // The uncovered coordinate now has an explicit `false` predicate arm and
    // the lexer takes the hook-carrying token path (not next_token_compiled).
    assert!(
        module.contains("=> { Some(false) }"),
        "override renders a failing arm"
    );
    assert!(module.contains("run_predicate"));
    assert!(!module.contains("next_token_compiled(&mut self.base, sink, atn(), lexer_dfa())"));
}

#[test]
fn coordinate_override_applies_to_structural_predicate() {
    // A `--sem-patterns` coordinate override replaces the body-derived
    // structural predicate and reaches generated SemIR.
    let patterns = parse_sem_patterns(
            "version = 1\n[[coordinate]]\nkind = \"predicate\"\nrule = \"s\"\nindex = 0\ndispose = \"assume-false\"\n",
        )
        .expect("pattern file parses");
    let templates = structural_predicate_templates(
        &predicate_parser_data(),
        SemanticsKind::ParserPredicate,
        &patterns,
    )
    .expect("override synthesis should succeed");
    assert_eq!(templates, [((0, 0), PredicateTemplate::False)]);

    // The rendered parser carries the SemIR predicate for that coordinate.
    let module = render_parser_with_options(
        "SParser",
        &predicate_parser_data(),
        ParserRenderOptions {
            patterns: Some(&patterns),
            ..ParserRenderOptions::default()
        },
    )
    .expect("parser should render");
    assert!(
        module.contains("rule_index: 0, pred_index: 0"),
        "override-derived predicate must reach parser_semantics()"
    );
}

#[test]
fn lexer_hook_policy_routes_uncovered_predicates_to_owned_hooks() {
    let module = render_lexer(
        "SLexer",
        &predicate_lexer_data(),
        false,
        SemUnknownPolicy::Hook,
        &SemPatternFile::default(),
        false,
    )
    .expect("hook policy should render generated hook plumbing");
    assert!(module.contains("next_token_compiled_with_semantic_dispatch"));
    assert!(module.contains("hooks: H"));
}

#[test]
fn hook_disposed_lexer_actions_require_semantic_dispatch() {
    let coordinates = [(0, 0)];
    let rule_names = ["A".to_owned()];
    let no_actions = [];

    assert!(lexer_actions_require_semantic_hooks(
        &coordinates,
        &rule_names,
        &no_actions,
        &SemPatternFile::default(),
        SemUnknownPolicy::Hook,
    ));

    let patterns = parse_sem_patterns(
        "version = 1\n[[coordinate]]\nkind = \"lexer-action\"\nindex = 0\ndispose = \"hook\"\n",
    )
    .expect("pattern file parses");
    assert!(lexer_actions_require_semantic_hooks(
        &coordinates,
        &rule_names,
        &no_actions,
        &patterns,
        SemUnknownPolicy::AssumeTrue,
    ));

    let translated = [((0, 0), ActionTemplate::LexerPopMode)];
    assert!(!lexer_actions_require_semantic_hooks(
        &coordinates,
        &rule_names,
        &translated,
        &SemPatternFile::default(),
        SemUnknownPolicy::Hook,
    ));
}

#[test]
fn lexer_hook_disposition_is_full_semantics() {
    let entry = SemanticsEntry {
        kind: SemanticsKind::LexerAction,
        rule_index: Some(0),
        rule_name: Some("A".to_owned()),
        index: Some(0),
        atn_state: None,
        line: None,
        column: None,
        body: Some("this.handle();".to_owned()),
        disposition: SemanticsDisposition::Hooked,
        template: None,
    };
    enforce_require_full_semantics(true, &[entry])
        .expect("hooked lexer actions are implemented by generated hook plumbing");
}

#[test]
fn lexer_default_policy_keeps_compiled_token_path() {
    let module = render_lexer(
        "SLexer",
        &predicate_lexer_data(),
        false,
        SemUnknownPolicy::AssumeTrue,
        &SemPatternFile::default(),
        false,
    )
    .expect("lexer should render");

    assert!(module.contains("next_token_compiled(&mut self.base, sink, atn(), lexer_dfa())"));
    assert!(module.contains("if H::ENABLES_LEXER_LIFECYCLE"));
    assert!(module.contains("next_token_compiled_with_semantic_dispatch"));
    assert!(module.contains("pub fn reset(&mut self)"));
    assert!(module.contains("reset_with_semantic_hooks"));
    assert!(module.contains("pub fn set_input_stream(&mut self, input: I)"));
    assert!(module.contains("set_input_stream_with_semantic_hooks"));
    assert!(module.contains("self.base.set_input_stream(input)"));
    assert!(module.contains("pub fn clear_dfa(&self)"));
    assert!(module.contains("self.base.clear_dfa()"));
    assert!(module.contains("pub fn add_error_listener<T>(&mut self, listener: T)"));
    assert!(module.contains(
            "T: for<'a> antlr4_runtime::ErrorListener<dyn antlr4_runtime::Recognizer + 'a> + Send + 'static,"
        ));
    assert!(module.contains("self.base.add_error_listener(listener)"));
    assert!(module.contains("pub fn remove_error_listeners(&mut self)"));
    assert!(module.contains("self.base.remove_error_listeners()"));
    assert!(module.contains(
        "fn next_token(&mut self, sink: &mut TokenSink<'_>) -> Result<TokenId, TokenStoreError>"
    ));
    assert!(
        module.contains(
            "fn source_text(&self) -> Option<std::rc::Rc<str>> { self.base.source_text() }"
        )
    );
    assert!(module.contains(
        "fn report_error(&self, source_error: &antlr4_runtime::token::TokenSourceError) -> bool"
    ));
    assert!(module.contains("Recognizer::notify_error_listeners(self, source_error.into());"));
    assert!(!module.contains("CommonToken"));
    assert!(!module.contains("TokenFactory"));
}

#[test]
fn lexer_superclass_emits_typed_lifecycle_contract_without_semantic_helpers() {
    let module = render_lexer(
        "LLexer",
        &lexer_fixture_data("lexer-superclass/L.g4"),
        false,
        SemUnknownPolicy::AssumeTrue,
        &SemPatternFile::default(),
        false,
    )
    .expect("lexer superclass should render a lifecycle contract");

    assert!(module.contains("pub trait LLexerHooks: Sized"));
    assert!(module.contains("fn lexer_reset<I>"));
    assert!(module.contains("fn lexer_before_token<I>"));
    assert!(module.contains("fn lexer_after_accept<I>"));
    assert!(module.contains("fn token_emitted"));
    assert!(module.contains("self.0.lexer_reset(ctx);"));
    assert!(module.contains("self.0.lexer_before_token(ctx);"));
    assert!(module.contains("self.0.lexer_after_accept(ctx);"));
    assert!(module.contains("pub fn with_typed_hooks(input: I, hooks: T) -> Self"));
}

#[test]
fn option_hook_requires_an_exact_assignment() {
    assert_eq!(
        normalize_option_hook(" superClass = BaseLexer ")
            .expect("valid hook assignment should normalize"),
        "superClass=BaseLexer"
    );
    assert!(normalize_option_hook("superClass").is_err());
    assert!(normalize_option_hook("=BaseLexer").is_err());
    assert!(normalize_option_hook("superClass=").is_err());
}

#[test]
fn lexer_run_predicate_default_arm_follows_policy() {
    // A mixed lexer (one covered coordinate plus an uncovered one that
    // lands on the catch-all arm) must honor `--sem-unknown`: assume-false
    // rejects the uncovered predicate instead of leaving it viable.
    let predicates = [((0_usize, 0_usize), PredicateTemplate::True)];

    let assume_true = render_lexer_predicate_method(&predicates, SemUnknownPolicy::AssumeTrue);
    assert!(assume_true.contains("_ => Some(true),"));
    assert!(!assume_true.contains("_ => Some(false),"));

    let assume_false = render_lexer_predicate_method(&predicates, SemUnknownPolicy::AssumeFalse);
    assert!(assume_false.contains("_ => Some(false),"));
    assert!(!assume_false.contains("_ => Some(true),"));

    // `error`/`hook` keep the conservative assume-true lexer default that
    // matches historical behavior (lexer fail-loud is a codegen-time error,
    // not a runtime catch-all).
    let error = render_lexer_predicate_method(&predicates, SemUnknownPolicy::Error);
    assert!(error.contains("_ => Some(true),"));
}
