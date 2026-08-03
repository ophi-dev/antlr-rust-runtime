/// Test-facing wrapper over [`render_parser_with_decision_report`] for the
/// many render assertions that never look at the manifest rows.
#[cfg(test)]
pub(crate) fn render_parser_with_options(
    grammar_name: &str,
    data: &ParserCodegenData<'_>,
    options: ParserRenderOptions<'_>,
) -> io::Result<String> {
    Ok(render_parser_with_decision_report(grammar_name, data, options)?.0)
}

/// [`render_parser_with_options`] plus the per-decision tier rows for the
/// `decisions.json` manifest, so callers that emit the manifest reuse the
/// classification the renderer already computed (the bounded LOOK(k)
/// enumeration is the expensive part under `--fixed-lookahead`).
pub(crate) fn render_parser_with_decision_report(
    grammar_name: &str,
    data: &ParserCodegenData<'_>,
    options: ParserRenderOptions<'_>,
) -> io::Result<(String, Vec<DecisionReportRow>)> {
    let empty_patterns = SemPatternFile::default();
    let patterns = options.patterns.unwrap_or(&empty_patterns);
    let type_name = rust_type_name(grammar_name);
    let compile_pattern_method = render_compile_parse_tree_pattern_method();
    let parse_listener_facade = render_parse_listener_facade();
    let metadata = render_parser_metadata(grammar_name, data);
    let parser_atn = data.parser_atn();
    let parser_atn_data = render_u32_slice(parser_atn.packed_words());
    let token_constants = render_token_constants(data);
    let rule_constants = render_rule_constants(data);
    // Decision routing: embedded mode always follows the tool
    // classification (Java parity); `--fixed-lookahead` additionally
    // compiles static dispatch for provable decisions in either mode. The
    // classification also carries the tier rows this function returns for
    // the `decisions.json` manifest.
    let decision_classification = classify_decisions(data, options.fixed_lookahead);
    let surface_model = build_parser_surface_model(data, &type_name, grammar_name, options)?;
    let embedded_data = surface_model.embedded_bindings();
    let structural_surface = surface_model.structural_bindings();
    let embedded_step_render = embedded_data
        .map(|embedded| embedded_step_render(embedded, &decision_classification));
    let decision_routing = decision_routing_render(&decision_classification, options);
    let mut portable_local_data = if options.embedded {
        PortableLocalData::default()
    } else {
        build_structural_portable_local_data(data, patterns)?
    };
    portable_local_data.required_generated_rules =
        parser_rule_callers_reaching(data, &portable_local_data.required_generated_rules);
    // A per-coordinate `assume-*` override is a documented no-op fallback; it
    // must not fall through to `parser_action_hook`, which would fail loud under
    // the Error policy or run a user side effect for a coordinate the manifest
    // reports as ignored. A `hook`/`error`
    // override falls through to the `parser_action_hook` catch-all, but an
    // `assume-*` override gets an explicit empty arm.
    let mut noop_action_states = collect_noop_action_states(data, patterns);
    // ANTLR-synthesized action states (left-recursion elimination, etc.) are
    // no-ops with no author intent. They must NOT reach `parser_action_hook`
    // either, or they would fail loud at runtime under the Error policy — the
    // same treatment `enforce_sem_unknown` gives them at codegen time. Give each
    // an explicit empty `run_action` arm.
    noop_action_states.extend(synthetic_parser_action_states(data)?);
    // Authored empty action bodies are explicit no-ops too: they carry source
    // provenance for the manifest but should not disable generated parsing or
    // fall through to runtime hooks under strict semantic policies.
    noop_action_states.extend(empty_parser_action_states(data)?);
    let predicates = if options.embedded {
        Vec::new()
    } else {
        structural_predicate_templates(data, SemanticsKind::ParserPredicate, patterns)?
    };
    let rule_args = if options.embedded {
        Vec::new()
    } else {
        structural_parser_rule_args(data)?
    };
    let parameterized_rules = if options.embedded {
        BTreeSet::new()
    } else {
        structural_parameterized_parser_rules(data)?
    };
    let ParserActionRouting {
        inline_statements: inline_action_statements,
        states: action_states,
        generated_states: generated_action_states,
        indices: action_indices,
        committed_indices: committed_action_indices,
    } = parser_action_routing(
        data,
        options.embedded,
        embedded_data,
        &portable_local_data,
        &parameterized_rules,
        &noop_action_states,
    )?;
    let inline_action_states = inline_action_statements
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    // Under a non-default unknown-coordinate policy every predicate transition
    // must reach the interpreter, which applies the policy to the complete
    // structurally bound coordinate inventory.
    let predicate_coordinates = parser_predicate_transitions(data)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut generated_predicate_coordinates = if options.embedded {
        predicate_coordinates.clone()
    } else {
        predicates
            .iter()
            .filter_map(|(coordinate, predicate)| {
                can_generate_parser_predicate(predicate).then_some(*coordinate)
            })
            .collect::<BTreeSet<_>>()
    };
    generated_predicate_coordinates.extend(portable_local_data.predicates.keys().copied());
    let has_action_dispatch = !action_states.is_empty();
    let has_predicate_dispatch = !predicates.is_empty();
    let track_alt_numbers = uses_alt_number_contexts(data);
    let track_context_alt_numbers = uses_structural_context_alt_numbers(data)?;
    let generated_rule_enabled = vec![true; data.rule_names.len()];
    let lowered_ir = lower_parser_ir(
        data,
        &generated_rule_enabled,
        &rule_args,
        ActionStateSets {
            all: &action_states,
            generated: &generated_action_states,
            inline: &inline_action_states,
            indices: &action_indices,
        },
        PredicateCoordinateSets {
            all: &predicate_coordinates,
            generated: &generated_predicate_coordinates,
        },
    );
    let optimized_ir = optimize_parser_ir(
        lowered_ir,
        (has_action_dispatch || has_predicate_dispatch || portable_local_data.has_semantics())
            && !options.embedded,
    )?;
    let generated_rules = optimized_ir.rules();
    let decision_report_rows =
        rendered_decision_report_rows(&decision_classification.report_rows, generated_rules);
    require_portable_local_rules_generated(
        generated_rules,
        &portable_local_data.required_generated_rules,
        data,
    )?;
    if options.require_generated_parser || options.embedded {
        // Embedded actions only run on the generated path, so every rule must
        // compile; an interpreted fallback would silently skip them.
        require_all_parser_rules_generated(generated_rules, data)?;
    }
    let portable_step_render = portable_local_data.step_render();
    let routing_plan = build_routing_plan(
        generated_rules,
        &data.rule_names,
        &inline_action_statements,
        embedded_step_render.is_some(),
        portable_step_render.map(|portable| portable.required_generated_rules),
    );
    let adaptive_atn_preferred_rule_count =
        routing_plan.adaptive_atn_preferred_rule_count();
    let generated_rule_dispatch = render_routing_plan(
        &routing_plan,
        generated_rules,
        &inline_action_statements,
        track_alt_numbers,
        track_context_alt_numbers,
        embedded_step_render,
        portable_step_render,
        decision_routing,
    );
    let unknown_policy_literal = parser_unknown_policy_literal(options.sem_unknown);
    let parse_rule_fallback = render_parser_parse_rule_fallback(ParserFallbackRender {
        track_alt_numbers,
        track_context_alt_numbers,
        rule_args: &rule_args,
        action_indices: &committed_action_indices,
        has_action_dispatch,
        has_predicate_dispatch,
        unknown_policy_literal,
    });
    let parser_semantics_function = render_parser_semantics_function(&predicates, data)?;
    let typed_hook_adapter =
        render_typed_hook_adapter(&type_name, &parser_typed_hook_mappings(data, patterns)?);
    let typed_parser_constructor = if typed_hook_adapter.is_empty() {
        String::new()
    } else {
        let trait_name = format!("{type_name}Hooks");
        let adapter_name = format!("{type_name}TypedHooks");
        format!(
            "impl<L, T> {type_name}<L, {adapter_name}<T>>\nwhere\n    L: TokenSource,\n    T: {trait_name},\n{{\n    pub fn with_typed_hooks(input: CommonTokenStream<L>, hooks: T) -> Self {{\n        Self::with_hooks(input, {adapter_name}::new(hooks))\n    }}\n}}\n"
        )
    };
    // The adaptive-direct path runs `parse_atn_rule_adaptive_or_fallback`, which
    // falls back through `parse_atn_rule` without the `ParserRuntimeOptions`
    // emitted below. A non-default unknown-predicate policy only reaches the
    // interpreter through those options, so it must not take that shortcut, or
    // an untranslated predicate would silently pass instead of applying the
    // configured fail/assume-false behavior.
    let adaptive_direct_allowed = !has_action_dispatch
        && !track_alt_numbers
        && !track_context_alt_numbers
        && !has_predicate_dispatch
        && unknown_policy_literal.is_none();
    let embedded_noop_states = BTreeSet::new();
    let action_method = render_parser_action_method(
        !action_states.is_empty() && !options.embedded,
        if options.embedded {
            &embedded_noop_states
        } else {
            &noop_action_states
        },
    );
    let parse_convenience =
        render_parser_parse_convenience(&type_name, parser_surface_name(grammar_name));
    // A grammar-declared `@members` initializer must seed the parser too, or a
    // predicate reading the slot would observe 0 and reject input the source
    // grammar accepts (issue #206 review).
    let parser_member_seeds = if options.embedded {
        String::new()
    } else {
        render_member_init_seeds(patterns, stack_member::MemberScope::Parser)?
    };
    let base_initialization =
        render_parser_base_initialization(unknown_policy_literal, &parser_member_seeds);
    let public_rule_method_names = parser_public_rule_method_names(&data.rule_names);
    let entry_rule_indices = likely_parser_entry_rule_indices(data);
    let parser_rustdoc = render_parser_rustdoc(&public_rule_method_names, &entry_rule_indices);
    let rule_methods = render_public_rule_methods(&public_rule_method_names);
    let (
        embedded_attrs_structs,
        embedded_module_items,
        embedded_struct_fields,
        embedded_field_inits,
        embedded_impl_items,
    ) = embedded_render_slots(surface_model.bindings());
    let support_bindings = GeneratedSupportBindings::current();
    let AdaptiveAtnParserRenderSlots {
        struct_field: adaptive_atn_preference_struct_field,
        field_init: adaptive_atn_preference_field_init,
        reset: adaptive_atn_preference_reset,
        retry_variant: adaptive_atn_retry_variant,
        retry_into_error: adaptive_atn_retry_into_error,
    } = adaptive_atn_parser_render_slots(adaptive_atn_preferred_rule_count);
    let generated_rule_error =
        render_generated_rule_error(adaptive_atn_retry_variant, adaptive_atn_retry_into_error);

    let embedded_imports = if embedded_data.is_some() || structural_surface.is_some() {
        "#[allow(unused_imports)]\nuse std::io::Write as _;\n#[allow(unused_imports)]\nuse antlr4_runtime::{java_style_list, PredictionMode, BailErrorStrategy, TerminalNodeView as RuntimeTerminalNode, ErrorNodeView as RuntimeErrorNode, RuleNodeView, AsRuleNode, FromRuleNode, MissingChildError, Token as _};\n"
    } else {
        ""
    };
    let render_model = ParserRenderModel {
        support_bindings,
        embedded_imports,
        token_constants,
        rule_constants,
        metadata,
        parser_semantics_function,
        typed_hook_adapter,
        embedded_attrs_structs,
        embedded_module_items,
        parser_atn_data,
        parse_convenience,
        parser_rustdoc,
        type_name,
        adaptive_atn_preference_struct_field,
        generated_rule_error,
        base_initialization,
        adaptive_atn_preference_field_init,
        embedded_struct_fields,
        embedded_field_inits,
        parse_listener_facade,
        adaptive_atn_preference_reset,
        compile_pattern_method,
        adaptive_direct_allowed,
        parse_rule_fallback,
        generated_rule_dispatch,
        embedded_impl_items,
        rule_methods,
        action_method,
        typed_parser_constructor,
    };
    Ok((
        render_parser_module(&render_model),
        decision_report_rows,
    ))
}

fn render_parser_module(model: &ParserRenderModel) -> String {
    let generated_header = model.support_bindings.module_header();
    let generated_footer = model.support_bindings.module_footer();
    let ParserRenderModel {
        embedded_imports,
        token_constants,
        rule_constants,
        metadata,
        parser_semantics_function,
        typed_hook_adapter,
        embedded_attrs_structs,
        embedded_module_items,
        parser_atn_data,
        parse_convenience,
        parser_rustdoc,
        type_name,
        adaptive_atn_preference_struct_field,
        generated_rule_error,
        base_initialization,
        adaptive_atn_preference_field_init,
        embedded_struct_fields,
        embedded_field_inits,
        parse_listener_facade,
        adaptive_atn_preference_reset,
        compile_pattern_method,
        adaptive_direct_allowed,
        parse_rule_fallback,
        generated_rule_dispatch,
        embedded_impl_items,
        rule_methods,
        action_method,
        typed_parser_constructor,
        ..
    } = model;
    format!(
        r#"{generated_header}use antlr4_runtime::recognizer::RecognizerData;
use antlr4_runtime::token::TokenSource;
use antlr4_runtime::token_stream::CommonTokenStream;
use antlr4_runtime::atn::parser_atn::ParserAtn;
use antlr4_runtime::{{BaseParser, GeneratedParser, GrammarMetadata, Parser, Recognizer}};
use std::sync::OnceLock;
{embedded_imports}

{token_constants}
{rule_constants}
{metadata}
{parser_semantics_function}
{typed_hook_adapter}
{embedded_attrs_structs}
{embedded_module_items}

static PARSER_ATN_DATA: &[u32] = &{parser_atn_data};
static ATN_CELL: OnceLock<ParserAtn> = OnceLock::new();

/// Validates and caches the packed grammar ATN for all parser instances.
fn atn() -> &'static ParserAtn {{
    ATN_CELL.get_or_init(|| {{
        ParserAtn::from_static(PARSER_ATN_DATA)
            .unwrap_or_else(|error| panic!("generated parser ATN is incompatible with this runtime: {{error}}"))
    }})
}}

/// Borrows the validated packed parser ATN embedded in this module.
pub fn parser_atn() -> &'static ParserAtn {{
    atn()
}}

{parse_convenience}

{parser_rustdoc}#[derive(Debug)]
pub struct {type_name}<L, H = antlr4_runtime::NoSemanticHooks>
where
    L: TokenSource,
    H: antlr4_runtime::SemanticHooks,
{{
    base: BaseParser<L, H>,
    simulator: Option<antlr4_runtime::ParserAtnSimulator<'static>>,
    generated_only: bool,
{adaptive_atn_preference_struct_field}{embedded_struct_fields}}}

{generated_rule_error}
impl<L> {type_name}<L, antlr4_runtime::NoSemanticHooks>
where
    L: TokenSource,
{{
    pub fn new(input: CommonTokenStream<L>) -> Self {{
        Self::with_hooks(input, antlr4_runtime::NoSemanticHooks)
    }}
}}

impl<L, H> {type_name}<L, H>
where
    L: TokenSource,
    H: antlr4_runtime::SemanticHooks,
{{
    pub fn with_hooks(input: CommonTokenStream<L>, hooks: H) -> Self {{
        let grammar_metadata = metadata();
        let data = grammar_metadata.recognizer_data();
{base_initialization}
        Self {{
            base,
            simulator: None,
            generated_only: std::env::var_os("ANTLR4_RUST_GENERATED_ONLY").is_some(),
{adaptive_atn_preference_field_init}{embedded_field_inits}        }}
    }}

    pub fn metadata() -> &'static GrammarMetadata {{
        metadata()
    }}

    /// Adds a listener for parser diagnostics.
    pub fn add_error_listener<T>(&mut self, listener: T)
    where
        T: for<'a> antlr4_runtime::ErrorListener<dyn antlr4_runtime::Recognizer + 'a> + Send + 'static,
    {{
        self.base.add_error_listener(listener);
    }}

    /// Removes every parser error listener, including the default console listener.
    pub fn remove_error_listeners(&mut self) {{
        self.base.remove_error_listeners();
    }}

{parse_listener_facade}
    /// Fully resets parser-owned state and rewinds the current token stream.
    pub fn reset(&mut self) {{
        self.base.reset();
        if let Some(simulator) = self.simulator.as_mut() {{
            simulator.reset();
        }}
{adaptive_atn_preference_reset}    }}

    /// Replaces the token stream and fully resets parser-owned state.
    pub fn set_token_stream(&mut self, input: CommonTokenStream<L>) {{
        self.base.set_token_stream(input);
        if let Some(simulator) = self.simulator.as_mut() {{
            simulator.reset();
        }}
{adaptive_atn_preference_reset}    }}

    #[must_use]
    pub const fn token_stream(&self) -> &CommonTokenStream<L> {{
        self.base.token_stream()
    }}

    #[must_use]
    pub const fn token_stream_mut(&mut self) -> &mut CommonTokenStream<L> {{
        self.base.token_stream_mut()
    }}

    #[must_use]
    pub const fn token_store(&self) -> &antlr4_runtime::TokenStore {{
        self.base.token_store()
    }}

    #[must_use]
    pub const fn parse_tree_storage(&self) -> &antlr4_runtime::ParseTreeStorage {{
        self.base.parse_tree_storage()
    }}

    #[must_use]
    pub fn prediction_context_stats(&self) -> antlr4_runtime::PredictionContextStats {{
        self.simulator.as_ref().map_or_else(
            antlr4_runtime::PredictionContextStats::default,
            antlr4_runtime::ParserAtnSimulator::prediction_context_stats,
        )
    }}

    #[must_use]
    pub fn parser_dfa_stats(&self) -> antlr4_runtime::ParserDfaStats {{
        self.simulator.as_ref().map_or_else(
            antlr4_runtime::ParserDfaStats::default,
            antlr4_runtime::ParserAtnSimulator::parser_dfa_stats,
        )
    }}

    /// Clears this grammar's learned parser decision DFAs.
    pub fn clear_dfa(&mut self) {{
        if let Some(simulator) = self.simulator.as_mut() {{
            simulator.clear_dfa();
        }} else {{
            antlr4_runtime::ParserAtnSimulator::clear_shared_dfa(atn());
        }}
{adaptive_atn_preference_reset}    }}

    #[must_use]
    pub fn node(&self, id: antlr4_runtime::NodeId) -> antlr4_runtime::Node<'_> {{
        self.base.node(id)
    }}

    #[must_use]
    pub fn into_token_stream(self) -> CommonTokenStream<L> {{
        self.base.into_token_stream()
    }}

    #[must_use]
    pub fn into_token_store(self) -> antlr4_runtime::TokenStore {{
        self.base.into_token_store()
    }}

    #[must_use]
    pub fn into_parsed_file(self, root: antlr4_runtime::NodeId) -> antlr4_runtime::ParsedFile {{
        self.base.into_parsed_file(root)
    }}
{compile_pattern_method}
    #[allow(dead_code)]
    fn simulator(&mut self) -> &mut antlr4_runtime::ParserAtnSimulator<'static> {{
        self.simulator
            .get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new_shared(atn()))
    }}

    #[allow(dead_code)]
    fn generated_only(&self) -> bool {{
        self.generated_only
    }}

    #[allow(dead_code)]
    fn parse_rule(&mut self, rule_index: usize) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{
        self.parse_rule_precedence(rule_index, 0)
    }}

    #[allow(dead_code)]
    fn parse_rule_precedence(&mut self, rule_index: usize, precedence: i32) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{
        self.parse_rule_precedence_inner(rule_index, precedence, true)
    }}

    #[allow(dead_code)]
    fn parse_rule_precedence_from_generated(&mut self, rule_index: usize, precedence: i32) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{
        self.parse_rule_precedence_inner(rule_index, precedence, false)
    }}

    #[allow(dead_code)]
    fn parse_rule_precedence_inner(&mut self, rule_index: usize, precedence: i32, allow_generated_fallback: bool) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{
        if allow_generated_fallback {{
            // True top-level entry: drop any fail-loud coordinates left by a
            // previous parse so a reused parser starts clean. Mid-parse the hits
            // are preserved so a generated parent can surface a recovered child's
            // fail-loud coordinate at this boundary.
            self.base.reset_unknown_semantic_hits();
            // Likewise drop stale sticky aborts (depth-cap violation,
            // parse-listener abort): entry rules share one parser instance,
            // and the flags must not poison the next parse when the previous
            // one exited through an error path.
            let _ = self.base.take_parse_abort();
        }}
        let __rule_start = antlr4_runtime::IntStream::index(self.base.input());
        let __generated_only = self.generated_only();
        let __tree = if let Some(result) = self.parse_generated_rule(rule_index, precedence, allow_generated_fallback) {{
            match result {{
                Ok(tree) => tree,
                Err(error) => {{
                    antlr4_runtime::IntStream::seek(self.base.input(), __rule_start);
                    let __report_error =
                        matches!(&error, GeneratedRuleError::Fatal(_));
                    // A fatal unwind retains recovery diagnostics committed
                    // earlier in this entry. Dispatch them before a semantic
                    // or parser-abort override can return, or they would leak
                    // into the next entry on a reused parser.
                    if allow_generated_fallback && __report_error {{
                        self.base.report_generated_parser_diagnostics();
                    }}
                    if allow_generated_fallback {{
                        // A sticky abort (depth cap, listener) wins over an
                        // error or semantic miss derived after recovery absorbed
                        // the aborted rule. Drain any masked semantic miss too,
                        // so neither condition poisons the next entry.
                        if let Some(abort) = self.base.take_parse_abort() {{
                            let _ = self.base.take_unknown_semantic_error();
                            return Err(abort);
                        }}
                        // A generated predicate that consulted an unimplemented
                        // hook fails the alternative and surfaces here as a generic
                        // failed-predicate/rule error. Prefer the recorded fail-loud
                        // semantic error when no parser abort occurred.
                        if let Some(semantic_error) = self.base.take_unknown_semantic_error() {{
                            return Err(semantic_error);
                        }}
                    }}
                    let error = error.into_error();
                    if allow_generated_fallback && __report_error {{
                        self.base.report_unrecovered_parser_error(&error);
                    }}
                    return Err(error);
                }}
            }}
        }} else if __generated_only {{
            return Err(antlr4_runtime::AntlrError::Unsupported(format!("generated parser did not emit rule {{}}", rule_index)));
        }} else {{
            self.parse_interpreted_rule_precedence(rule_index, precedence)?
        }};
        if allow_generated_fallback {{
            self.base.report_generated_parser_diagnostics();
            // A sticky abort (depth-cap violation, listener abort) is not a
            // syntax error: rule-level recovery may have produced a tree
            // and semantic miss anyway, but the abort is the root cause. Drain
            // both sticky conditions before returning so parser reuse is clean.
            if let Some(error) = self.base.take_parse_abort() {{
                let _ = self.base.take_unknown_semantic_error();
                return Err(error);
            }}
            // Surface unknown predicate/action coordinates recorded under the
            // Error policy only after parser aborts have been ruled out.
            if let Some(error) = self.base.take_unknown_semantic_error() {{
                return Err(error);
            }}
        }}
        Ok(__tree)
    }}

    #[allow(dead_code)]
    fn parse_interpreted_rule(&mut self, rule_index: usize) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{
        self.parse_interpreted_rule_precedence(rule_index, 0)
    }}

    #[allow(dead_code)]
    fn parse_interpreted_rule_precedence(&mut self, rule_index: usize, precedence: i32) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{
        if precedence == 0 && {adaptive_direct_allowed} && std::env::var_os("ANTLR4_RUST_ADAPTIVE_DIRECT").is_some() {{
            let simulator = self
                .simulator
                .get_or_insert_with(|| antlr4_runtime::ParserAtnSimulator::new_shared(atn()));
            self.base
                .parse_atn_rule_adaptive_or_fallback(atn(), simulator, rule_index)
        }} else {{
{parse_rule_fallback}
        }}
    }}

{generated_rule_dispatch}

{embedded_impl_items}
{rule_methods}

{action_method}
}}

{typed_parser_constructor}

impl<L, H> GeneratedParser for {type_name}<L, H>
where
    L: TokenSource,
    H: antlr4_runtime::SemanticHooks,
{{
    fn metadata() -> &'static GrammarMetadata {{
        metadata()
    }}

    fn parser_atn() -> &'static ParserAtn {{
        parser_atn()
    }}
}}

impl<L, H> Recognizer for {type_name}<L, H>
where
    L: TokenSource,
    H: antlr4_runtime::SemanticHooks,
{{
    fn data(&self) -> &antlr4_runtime::RecognizerData {{
        self.base.data()
    }}

    fn data_mut(&mut self) -> &mut antlr4_runtime::RecognizerData {{
        self.base.data_mut()
    }}
}}

impl<L, H> Parser for {type_name}<L, H>
where
    L: TokenSource,
    H: antlr4_runtime::SemanticHooks,
{{
    fn build_parse_trees(&self) -> bool {{ self.base.build_parse_trees() }}
    fn set_build_parse_trees(&mut self, build: bool) {{ self.base.set_build_parse_trees(build); }}
    fn number_of_syntax_errors(&self) -> usize {{ self.base.number_of_syntax_errors() }}
    fn report_diagnostic_errors(&self) -> bool {{ self.base.report_diagnostic_errors() }}
    fn set_report_diagnostic_errors(&mut self, report: bool) {{ self.base.set_report_diagnostic_errors(report); }}
    fn prediction_mode(&self) -> antlr4_runtime::PredictionMode {{ self.base.prediction_mode() }}
    fn set_prediction_mode(&mut self, mode: antlr4_runtime::PredictionMode) {{ self.base.set_prediction_mode(mode); }}
    fn max_rule_depth(&self) -> Option<usize> {{ self.base.max_rule_depth() }}
    fn set_max_rule_depth(&mut self, depth: Option<usize>) {{ self.base.set_max_rule_depth(depth); }}
    // Route through the trait impl: BaseParser's inherent generic method
    // would re-box the already-boxed listener.
    fn add_parse_listener(&mut self, listener: Box<dyn antlr4_runtime::ParseListener>) {{ antlr4_runtime::Parser::add_parse_listener(&mut self.base, listener); }}
    fn remove_parse_listeners(&mut self) -> Vec<Box<dyn antlr4_runtime::ParseListener>> {{ self.base.remove_parse_listeners() }}
}}
{generated_footer}"#
    )
}
