// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
/// Renders a Rust parser module with one public method per grammar rule.
///
/// Parser methods use generated recursive-descent bodies for the ATN subset
/// covered by the optimized parser IR and keep the interpreter fallback for
/// unsupported constructs while the generated surface is expanded.
#[cfg(test)]
pub(crate) fn render_parser(
    grammar_name: &str,
    data: &ParserCodegenData<'_>,
) -> io::Result<String> {
    render_parser_with_options(grammar_name, data, ParserRenderOptions::default())
}

/// Final, structured input to parser module assembly.
///
/// Every recognition, routing, semantic, and surface decision is resolved
/// before this artifact reaches the formatter.
#[derive(Debug)]
pub(crate) struct ParserRenderModel {
    pub(crate) support_bindings: GeneratedSupportBindings,
    pub(crate) embedded_imports: &'static str,
    pub(crate) token_constants: String,
    pub(crate) rule_constants: String,
    pub(crate) metadata: String,
    pub(crate) parser_semantics_function: String,
    pub(crate) typed_hook_adapter: String,
    pub(crate) embedded_attrs_structs: String,
    pub(crate) embedded_module_items: String,
    pub(crate) embedded_header_items: String,
    pub(crate) embedded_definitions_items: String,
    pub(crate) parser_atn_data: String,
    pub(crate) parse_convenience: String,
    pub(crate) parser_rustdoc: String,
    pub(crate) type_name: String,
    pub(crate) adaptive_atn_preferred_rule_count: usize,
    pub(crate) base_initialization: String,
    pub(crate) embedded_struct_fields: String,
    pub(crate) embedded_field_inits: String,
    pub(crate) adaptive_direct_allowed: bool,
    pub(crate) parse_rule_fallback: String,
    pub(crate) generated_rule_dispatch: String,
    pub(crate) embedded_impl_items: String,
    pub(crate) rule_methods: String,
    pub(crate) action_method: String,
    pub(crate) typed_parser_constructor: String,
}

const GENERATED_PARSER_RESERVED_RULE_METHODS: &[&str] = &[
    "reset",
    "set_token_stream",
    "token_stream",
    "token_stream_mut",
    "token_store",
    "parse_tree_storage",
    "clear_dfa",
    "add_error_listener",
    "remove_error_listeners",
    "add_parse_listener",
    "remove_parse_listeners",
    "node",
    "into_token_stream",
    "into_token_store",
    "into_parsed_file",
    "compile_parse_tree_pattern",
];

pub(crate) fn parser_public_rule_method_names(rule_names: &[String]) -> Vec<String> {
    let mut used = GENERATED_PARSER_RESERVED_RULE_METHODS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    rule_names
        .iter()
        .map(|rule| {
            let base = rust_function_name(rule);
            let name = unique_rule_method_name(&base, &used);
            used.insert(name.clone());
            name
        })
        .collect()
}

fn unique_rule_method_name(base: &str, used: &BTreeSet<String>) -> String {
    if !used.contains(base) {
        return base.to_owned();
    }

    let plain = base.strip_prefix("r#").unwrap_or(base);
    let reserved_collision = GENERATED_PARSER_RESERVED_RULE_METHODS.contains(&base);
    let stem = if reserved_collision {
        format!("{plain}_rule")
    } else {
        plain.to_owned()
    };
    let (mut candidate, mut suffix) = if reserved_collision {
        (stem.clone(), 2)
    } else {
        (format!("{stem}_2"), 3)
    };
    while used.contains(&candidate) {
        candidate = format!("{stem}_{suffix}");
        suffix += 1;
    }
    candidate
}

/// Raw grammar-local booleans whose actions and predicates are portable
/// without executing target-language code.
#[derive(Debug, Default)]
pub(crate) struct PortableLocalData {
    /// Rule index -> generated local variable declarations.
    pub(crate) declarations: Vec<Vec<String>>,
    /// ATN action source state -> generated assignment.
    pub(crate) inline_actions: BTreeMap<usize, String>,
    /// Parser predicate coordinate -> generated boolean expression.
    pub(crate) predicates: BTreeMap<(usize, usize), (String, Option<String>)>,
    /// Rules whose local state has no equivalent interpreted representation.
    pub(crate) required_generated_rules: BTreeSet<usize>,
}

impl PortableLocalData {
    fn has_semantics(&self) -> bool {
        !self.required_generated_rules.is_empty()
    }

    fn step_render(&self) -> Option<PortableLocalStepRender<'_>> {
        self.has_semantics().then_some(PortableLocalStepRender {
            declarations: &self.declarations,
            predicates: &self.predicates,
            required_generated_rules: &self.required_generated_rules,
        })
    }
}

fn portable_local_name(name: &str) -> String {
    format!("__antlr_local_{name}")
}

fn parse_portable_bool_assignment(body: &str) -> Option<(&str, bool)> {
    let body = body.trim();
    let body = body.strip_suffix(';').unwrap_or(body);
    let (target, value) = body.split_once('=')?;
    let target = target.trim().strip_prefix('$')?;
    let value = match value.trim() {
        "true" => true,
        "false" => false,
        _ => return None,
    };
    (!target.is_empty()
        && target
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric()))
    .then_some((target, value))
}

fn parse_portable_bool_predicate(body: &str) -> Option<(&str, bool)> {
    let body = body.trim();
    let (body, negated) = body
        .strip_prefix('!')
        .map_or((body, false), |body| (body.trim(), true));
    let name = body.strip_prefix('$')?.trim();
    (!name.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric()))
    .then_some((name, negated))
}

pub(crate) fn build_structural_portable_local_data(
    data: &RecognizerCodegenData<'_>,
    patterns: &SemPatternFile,
) -> io::Result<PortableLocalData> {
    let model = structural_embedded_model(data, false)?;
    let mut out = PortableLocalData {
        declarations: vec![Vec::new(); data.rule_names.len()],
        ..PortableLocalData::default()
    };
    let mut local_names = Vec::with_capacity(data.rule_names.len());
    for (rule_index, rule) in model.rules.iter().enumerate() {
        let mut names = BTreeMap::new();
        for attr in rule.attrs.iter().filter(|attr| attr.ty == "bool") {
            let initial = if rule.local_names.contains(&attr.name) {
                "false"
            } else if rule.arg_names.first() == Some(&attr.name) {
                "__precedence != 0"
            } else {
                continue;
            };
            let local = portable_local_name(&attr.name);
            out.declarations[rule_index].push(format!("let mut {local} = {initial};"));
            names.insert(attr.name.clone(), local);
        }
        local_names.push(names);
    }

    for action in structural_actions(data)? {
        let Some((name, value)) = parse_portable_bool_assignment(&action.body) else {
            continue;
        };
        let Some(local) = local_names
            .get(action.rule_index)
            .and_then(|names| names.get(name))
        else {
            continue;
        };
        if patterns
            .coordinate_disposition(
                SemanticsKind::ParserAction,
                data.rule_names.get(action.rule_index).map(String::as_str),
                Some(action.action_index),
                Some(action.state),
            )
            .is_some()
        {
            continue;
        }
        out.inline_actions
            .insert(action.state, format!("{local} = {value};"));
        out.required_generated_rules.insert(action.rule_index);
    }

    for predicate in structural_predicates(data)? {
        let Some((name, negated)) = parse_portable_bool_predicate(&predicate.body) else {
            continue;
        };
        let Some(local) = local_names
            .get(predicate.rule_index)
            .and_then(|names| names.get(name))
        else {
            continue;
        };
        if patterns
            .coordinate_disposition(
                SemanticsKind::ParserPredicate,
                data.rule_names
                    .get(predicate.rule_index)
                    .map(String::as_str),
                Some(predicate.predicate_index),
                None,
            )
            .is_some()
        {
            continue;
        }
        let expression = if negated {
            format!("!{local}")
        } else {
            local.clone()
        };
        out.predicates.insert(
            (predicate.rule_index, predicate.predicate_index),
            (expression, predicate.fail),
        );
        out.required_generated_rules.insert(predicate.rule_index);
    }

    Ok(out)
}

/// Collects `assume-*`-overridden action states that should become explicit
/// empty `run_action` arms instead of falling through to user hooks.
fn collect_noop_action_states(
    data: &ParserCodegenData<'_>,
    patterns: &SemPatternFile,
) -> BTreeSet<usize> {
    let mut noop_action_states = BTreeSet::new();
    let action_state_coordinates = parser_action_state_coordinates(data);
    for state in action_state_coordinates.keys() {
        if parser_action_assume_overridden(patterns, data, &action_state_coordinates, *state) {
            noop_action_states.insert(*state);
        }
    }
    noop_action_states
}

/// Public per-rule entry methods (`pub fn s(&mut self) -> …`).
fn render_public_rule_methods(public_rule_method_names: &[String]) -> String {
    let mut rule_methods = String::new();
    for (index, rule_method_name) in public_rule_method_names.iter().enumerate() {
        writeln!(
            rule_methods,
            "    pub fn {rule_method_name}(&mut self) -> Result<antlr4_runtime::ParseTree, antlr4_runtime::AntlrError> {{"
        )
        .expect("writing to a string cannot fail");
        writeln!(rule_methods, "        self.parse_rule({index})")
            .expect("writing to a string cannot fail");
        writeln!(rule_methods, "    }}").expect("writing to a string cannot fail");
    }
    rule_methods
}

/// Step-render view over the embedded data.
///
/// `force_adaptive` stays off: per-decision routing follows the tool
/// classification in `adaptive_decisions` instead, matching which decisions
/// Java compiles to switches versus `adaptivePredict` calls.
fn embedded_step_render<'a>(
    embedded: &'a ParserSurfaceBindings,
    decisions: &'a ParserDecisionAnalysis,
) -> EmbeddedStepRender<'a> {
    EmbeddedStepRender {
        force_adaptive: false,
        adaptive_decisions: &decisions.adaptive_decisions,
        complete_ll1_dispatches: &decisions.complete_ll1_dispatches,
        predicates: &embedded.predicates,
        rule_has_attrs: &embedded.rule_has_attrs,
        init_entry: &embedded.init_entry,
        after: &embedded.after,
        catch_clauses: &embedded.catch_clauses,
        finally_bodies: &embedded.finally_bodies,
        call_args: &embedded.call_args,
        rule_arg0: &embedded.rule_arg0,
    }
}

/// The module/struct/impl text [`ParserSurfaceBindings`] contributes to the
/// rendered parser, empty in template mode.
#[derive(Default)]
struct EmbeddedRenderSlots {
    attrs_structs: String,
    module_items: String,
    header_items: String,
    definitions_items: String,
    struct_fields: String,
    field_inits: String,
    impl_items: String,
}

fn embedded_render_slots(embedded_data: Option<&ParserSurfaceBindings>) -> EmbeddedRenderSlots {
    embedded_data.map_or_else(EmbeddedRenderSlots::default, |embedded| {
        EmbeddedRenderSlots {
            attrs_structs: embedded.attrs_structs.clone(),
            module_items: embedded.module_items.clone(),
            header_items: embedded.header_items.clone(),
            definitions_items: embedded.definitions_items.clone(),
            struct_fields: embedded.struct_fields.clone(),
            field_inits: embedded.field_inits.clone(),
            impl_items: embedded.impl_items.clone(),
        }
    })
}

/// Step-render view over the opt-in `--fixed-lookahead` routing. Embedded
/// mode reads its LL(1) switch tables through `EmbeddedStepRender`; the
/// depth-1 dispatch tables are for plain mode, where static LL(1) switches
/// are part of the opt-in flag.
fn decision_routing_render<'a>(
    classification: &'a ParserDecisionAnalysis,
    options: ParserRenderOptions<'_>,
) -> DecisionRoutingRender<'a> {
    DecisionRoutingRender {
        complete_ll1_dispatches: (!options.embedded)
            .then_some(&classification.complete_ll1_dispatches),
        ll1_dispatch_tables: (!options.embedded && options.fixed_lookahead.is_some())
            .then_some(&classification.ll1_dispatch_tables),
        fixed_lookahead_tables: options
            .fixed_lookahead
            .is_some_and(|depth| depth >= 2)
            .then_some(&classification.fixed_lookahead_tables),
    }
}

/// Builds the mode-specific action/attribute surface: embedded mode
/// translates and splices the grammar's real Rust action/predicate bodies
/// (rendered through the target `.test.stg`) instead of recognizing
/// template markup; plain mode builds the structural surface.
fn build_parser_surface_model(
    data: &RecognizerCodegenData<'_>,
    type_name: &str,
    grammar_name: &str,
    options: ParserRenderOptions<'_>,
) -> io::Result<ParserSurfaceModel> {
    if options.embedded {
        let embedded = build_embedded_parser_data(data, type_name, grammar_name, options)?;
        Ok(ParserSurfaceModel::embedded(embedded))
    } else {
        let surface = build_structural_parser_surface(data, grammar_name, options)?;
        Ok(ParserSurfaceModel::structural(surface))
    }
}

struct ParserActionRouting {
    inline_statements: BTreeMap<usize, String>,
    states: BTreeSet<usize>,
    generated_states: BTreeSet<usize>,
    indices: BTreeMap<usize, usize>,
    committed_indices: Vec<(usize, usize)>,
}

fn parser_action_routing(
    data: &ParserCodegenData<'_>,
    embedded: bool,
    embedded_data: Option<&ParserSurfaceBindings>,
    portable_local_data: &PortableLocalData,
    parameterized_rules: &BTreeSet<usize>,
    noop_states: &BTreeSet<usize>,
) -> io::Result<ParserActionRouting> {
    let mut inline_statements = embedded_data.map_or_else(
        || portable_local_data.inline_actions.clone(),
        |embedded| embedded.inline_actions.clone(),
    );
    let states = parser_action_states(data)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let structural_actions = structural_actions(data)?;
    let indices = structural_actions
        .iter()
        .map(|action| (action.state, action.action_index))
        .collect::<BTreeMap<_, _>>();
    let action_rules = structural_actions
        .iter()
        .map(|action| (action.state, action.rule_index))
        .collect::<BTreeMap<_, _>>();
    let committed_indices = structural_actions
        .iter()
        .filter(|action| {
            !embedded
                && action.authored
                && !action.body.trim().is_empty()
                && !noop_states.contains(&action.state)
        })
        .map(|action| (action.state, action.action_index))
        .collect::<Vec<_>>();
    let mut generated_states = if embedded {
        states.clone()
    } else {
        noop_states.intersection(&states).copied().collect()
    };
    generated_states.extend(portable_local_data.inline_actions.keys().copied());
    if !embedded {
        for state in states
            .difference(noop_states)
            .filter(|state| !portable_local_data.inline_actions.contains_key(state))
        {
            let statement = if action_rules
                .get(state)
                .is_some_and(|rule_index| parameterized_rules.contains(rule_index))
            {
                "let _ = self.base.parser_action_hook_with_context_and_local(action, &__ctx, __precedence);"
            } else {
                "let _ = self.base.parser_action_hook_with_context(action, &__ctx);"
            };
            inline_statements.insert(*state, statement.to_owned());
            generated_states.insert(*state);
        }
    }
    Ok(ParserActionRouting {
        inline_statements,
        states,
        generated_states,
        indices,
        committed_indices,
    })
}

/// Renders stable authored parser-action coordinates for committed fallback.
fn render_parser_action_index_array(action_indices: &[(usize, usize)]) -> String {
    let items = action_indices
        .iter()
        .map(|(source_state, action_index)| format!("({source_state}, {action_index})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

/// Renders parser rule-argument metadata for generated calls into the runtime.
fn render_parser_rule_arg_array(args: &[(usize, usize, RuleArgTemplate)]) -> String {
    let items = args
        .iter()
        .map(|(source_state, rule_index, value)| {
            let (value, inherit_local) = match value {
                RuleArgTemplate::Literal(value) => (*value, false),
                RuleArgTemplate::InheritLocal => (0, true),
            };
            format!(
                "antlr4_runtime::ParserRuleArg {{ source_state: {source_state}, rule_index: {rule_index}, value: {value}, inherit_local: {inherit_local} }}"
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

/// Renders the generated parser base construction.
///
/// When a non-default unknown-predicate policy is configured, the constructor
/// installs it on the `BaseParser` so the generated recursive-descent path
/// (which evaluates predicates without going through `ParserRuntimeOptions`)
/// honors `--sem-unknown` too, rather than leaving the field at `AssumeTrue`.
///
/// `member_seeds` carries a grammar's declared `@members` initial values
/// (issue #206). Parsers need this for the same reason lexers do: a predicate
/// reading a slot that silently started at 0 would reject input the source
/// grammar accepts.
fn render_parser_base_initialization(
    unknown_policy_literal: Option<&str>,
    member_seeds: &str,
) -> String {
    let needs_mut = unknown_policy_literal.is_some() || !member_seeds.is_empty();
    let mut out = if needs_mut {
        "        let mut base = BaseParser::with_semantic_hooks(input, data, hooks);".to_owned()
    } else {
        "        let base = BaseParser::with_semantic_hooks(input, data, hooks);".to_owned()
    };
    if let Some(policy) = unknown_policy_literal {
        write!(
            out,
            "\n        base.set_unknown_predicate_policy({policy});"
        )
        .expect("writing to a string cannot fail");
    }
    if !member_seeds.is_empty() {
        write!(out, "\n        base.set_initial_members({member_seeds});")
            .expect("writing to a string cannot fail");
    }
    out
}

/// Renders parser-module conveniences that wire text or a caller-provided
/// character stream through the lexer, token stream, parser, and entry rule.
///
/// The whole surface (the `<Type>ParseOutput` alias, the validation bridge,
/// and the `parse*` / `parse_stream*` functions) is owned by the runtime's
/// `__antlr4_rust_parser_entry_points!` macro; this renderer only wires the
/// module's grammar-specific names into the invocation.
pub(crate) fn render_parser_parse_convenience(type_name: &str, surface_name: &str) -> String {
    let output_type_name = format!("{type_name}ParseOutput");
    let validated_tree_name = format!("{surface_name}ValidatedTree");
    let validation_error_name = format!("{surface_name}ValidationError");
    format!(
        r"antlr4_runtime::__antlr4_rust_parser_entry_points! {{
    parser: {type_name},
    output: {output_type_name},
    validated_tree: {validated_tree_name},
    validation_error: {validation_error_name},
    validate_tree: validate_tree_structure,
}}"
    )
}
