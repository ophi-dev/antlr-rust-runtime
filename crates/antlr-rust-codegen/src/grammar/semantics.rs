use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use super::action::{ActionReferenceParser, action_references};
use super::char_support::decode_string_literal;
use super::diagnostic::{CompilationError, Diagnostic, Severity};
use super::frontend::{SourceId, SourceSpan};
use super::integration::{
    IntegratedGrammarSet, IntegratedVocabulary, IntegratedVocabularySource, RootOutputs,
};
use super::left_recursion::rewrite_immediate_left_recursion;
use super::model::{
    Alternative, Block, Element, ElementKind, GrammarId, GrammarKind, GrammarPrequel, GrammarUnit,
    ModelIdAllocator, Quantifier, RecognizerModel, Rule, RuleId, RuleKind, SemanticGrammar,
    SetElement, Terminal, TokenDeclaration, TokenSymbol, TokenSymbolId, Vocabulary,
};
use super::mutual_recursion::eliminate_mutual_left_recursion_with_action_reference_parser;
use super::provenance::ProvenanceIndex;
use super::rule_reachability::{EntryRuleConfig, analyze as analyze_rule_reachability};
use super::source::SourceSet;

mod attributes;
mod bindings;

use attributes::check_symbol_conflicts;
#[cfg(test)]
pub(crate) use attributes::{ParsedAttributeDeclaration, parse_attribute_declarations};
use bindings::{BindingCollector, BindingCollectorContext};

const EOF_TOKEN_TYPE: i32 = -1;
const INVALID_TOKEN_TYPE: i32 = 0;
const MIN_USER_CHANNEL: i32 = 2;
const COMMON_CONSTANTS: &[&str] = &[
    "HIDDEN",
    "DEFAULT_TOKEN_CHANNEL",
    "DEFAULT_MODE",
    "SKIP",
    "MORE",
    "EOF",
    "MAX_CHAR_VALUE",
    "MIN_CHAR_VALUE",
];
const GRAMMAR_OPTIONS: &[&str] = &[
    "superClass",
    "contextSuperClass",
    "TokenLabelType",
    "tokenVocab",
    "language",
    "accessLevel",
    "exportMacro",
    "caseInsensitive",
];
const RULE_REF_OPTIONS: &[&str] = &["p", "tokenIndex"];
const TOKEN_OPTIONS: &[&str] = &["assoc", "tokenIndex"];
const TOKEN_ATTRIBUTES: &[&str] = &["text", "type", "line", "index", "pos", "channel", "int"];
const SUPPORTED_TARGET_LANGUAGES: &[&str] = &[
    "Cpp",
    "CSharp",
    "Dart",
    "Go",
    "JavaScript",
    "Java",
    "PHP",
    "Python3",
    "Rust",
    "Swift",
    "TypeScript",
];

#[derive(Clone, Debug)]
pub(crate) struct SemanticGrammarSet {
    pub(crate) grammars: Vec<SemanticGrammar>,
    pub(crate) roots: BTreeMap<GrammarId, RootOutputs>,
    pub(crate) source_outputs: BTreeMap<GrammarId, RootOutputs>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) provenance: ProvenanceIndex,
    pub(crate) ids: ModelIdAllocator,
}

pub(crate) fn analyze(
    sources: &SourceSet,
    integrated: IntegratedGrammarSet,
) -> Result<SemanticGrammarSet, CompilationError> {
    analyze_with_entry_rules(sources, integrated, &EntryRuleConfig::default())
}

pub(crate) fn analyze_with_entry_rules(
    sources: &SourceSet,
    integrated: IntegratedGrammarSet,
    entry_rules: &EntryRuleConfig,
) -> Result<SemanticGrammarSet, CompilationError> {
    analyze_with_action_reference_parser(sources, integrated, entry_rules, action_references)
}

pub(crate) fn analyze_with_action_reference_parser(
    sources: &SourceSet,
    mut integrated: IntegratedGrammarSet,
    entry_rules: &EntryRuleConfig,
    action_reference_parser: ActionReferenceParser,
) -> Result<SemanticGrammarSet, CompilationError> {
    let mut diagnostics = std::mem::take(&mut integrated.diagnostics);
    let deferred_diagnostics = channel_placement_diagnostics(&integrated.grammar.units);
    diagnostics.extend(basic_checks(sources, &integrated.grammar.units));
    if has_blocking_basic_errors(&diagnostics) {
        return Err(CompilationError::new(diagnostics));
    }
    let (unreachable_diagnostics, preserved_entry_rules) = check_unreachable_rules(
        &integrated.grammar.units,
        &integrated.grammar.target_units,
        entry_rules,
    );
    diagnostics.extend(unreachable_diagnostics);

    // Snapshot the authored units for symbol validation *before* any
    // left-recursion rewriting: the mutual-recursion pass may delete satellite
    // rules, and a symbol conflict involving a deleted rule's name (a return
    // value named like a rule, say) must still be reported against what the
    // author wrote.
    let symbol_units = integrated
        .grammar
        .units
        .iter()
        .map(|unit| (unit.id, unit.clone()))
        .collect::<BTreeMap<_, _>>();

    // Reduce tractable mutual (indirect) left recursion to direct left
    // recursion before the direct-recursion rewrite runs (issue #151). This is
    // a no-op on grammars with no reducible left-corner cycle; anything it
    // declines is reported later by the ATN-level G4A005 detector.
    eliminate_mutual_left_recursion_with_action_reference_parser(
        &mut integrated.grammar.units,
        &mut integrated.ids,
        &mut integrated.grammar.provenance,
        &preserved_entry_rules,
        action_reference_parser,
    );
    diagnostics.extend(rewrite_immediate_left_recursion(
        &mut integrated.grammar.units,
        &mut integrated.ids,
        &mut integrated.grammar.provenance,
    ));
    if has_blocking_basic_errors(&diagnostics) {
        return Err(CompilationError::new(diagnostics));
    }

    let mut vocabularies = BTreeMap::new();
    let mut grammars = Vec::with_capacity(integrated.grammar.units.len());
    for unit in std::mem::take(&mut integrated.grammar.units) {
        let unit_id = unit.id;
        let dependencies = integrated
            .vocabularies
            .iter()
            .filter(|dependency| dependency.consumer == unit_id)
            .collect::<Vec<_>>();
        let shares_tokens_with_implicit_lexer = dependencies
            .iter()
            .any(|dependency| dependency.declaration.is_none());
        let mut imported = VocabularyBuilder::new();
        for dependency in dependencies {
            import_dependency(
                dependency,
                &vocabularies,
                &mut imported,
                &mut integrated.ids,
                &mut diagnostics,
            );
        }

        let semantic = analyze_unit(
            UnitAnalysisContext {
                sources,
                symbol_unit: &symbol_units[&unit_id],
                shares_tokens_with_implicit_lexer,
                entry_rules,
                action_reference_parser,
            },
            unit,
            imported,
            &mut integrated.ids,
            &mut diagnostics,
        );
        vocabularies.insert(semantic.unit.id, semantic.recognizer.vocabulary.clone());
        grammars.push(semantic);
    }

    diagnostics.extend(deferred_diagnostics);
    if has_errors(&diagnostics) {
        return Err(CompilationError::new(diagnostics));
    }
    Ok(SemanticGrammarSet {
        grammars,
        roots: integrated.roots,
        source_outputs: integrated.source_outputs,
        diagnostics,
        provenance: integrated.grammar.provenance,
        ids: integrated.ids,
    })
}

fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
}

fn diagnostic_span_in_root(
    sources: &SourceSet,
    root: SourceId,
    original: &SourceSpan,
) -> SourceSpan {
    if original.source == root {
        return original.clone();
    }
    let Some(source) = sources.get(original.source) else {
        return original.clone();
    };
    let Some(target) = sources.get(root) else {
        return original.clone();
    };
    let Some((start_line, start_column)) = source.line_column(original.bytes.start) else {
        return original.clone();
    };
    let Some(start) = target.byte_offset(start_line, start_column) else {
        return original.clone();
    };
    let end = source
        .line_column(original.bytes.end)
        .and_then(|(line, column)| target.byte_offset(line, column))
        .filter(|end| *end >= start)
        .unwrap_or(start);
    SourceSpan {
        source: root,
        bytes: start..end,
    }
}

fn has_blocking_basic_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error && diagnostic.code != "G4S016")
}

fn basic_checks(sources: &SourceSet, units: &[GrammarUnit]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for unit in units {
        check_unit_basics(sources, unit, &mut diagnostics);
    }
    diagnostics
}

fn check_unit_basics(sources: &SourceSet, unit: &GrammarUnit, diagnostics: &mut Vec<Diagnostic>) {
    if unit.rules.is_empty() {
        diagnostics.push(Diagnostic::error(
            "G4S001",
            unit.span.clone(),
            format!("grammar {} has no rules", unit.name),
        ));
        return;
    }

    let mut rules = BTreeMap::<&str, &Rule>::new();
    for rule in &unit.rules {
        if let Some(previous) = rules.insert(&rule.name, rule) {
            diagnostics.push(
                Diagnostic::error(
                    "G4S002",
                    rule.span.clone(),
                    format!("rule {} is redefined", rule.name),
                )
                .with_related(previous.span.clone(), "first definition is here"),
            );
        }
        if COMMON_CONSTANTS.contains(&rule.name.as_str()) {
            diagnostics.push(Diagnostic::error(
                "G4S003",
                rule.span.clone(),
                format!("rule {} uses a reserved name", rule.name),
            ));
        }
        match (unit.kind, rule.kind) {
            (GrammarKind::Lexer, RuleKind::Parser) => diagnostics.push(Diagnostic::error(
                "G4S004",
                rule.span.clone(),
                format!(
                    "parser rule {} is not allowed in a lexer grammar",
                    rule.name
                ),
            )),
            (GrammarKind::Parser, RuleKind::Lexer) if rule.mode.is_none() => {
                diagnostics.push(Diagnostic::error(
                    "G4S005",
                    rule.span.clone(),
                    format!(
                        "lexer rule {} is not allowed in a parser grammar",
                        rule.name
                    ),
                ));
            }
            (GrammarKind::Parser, RuleKind::Lexer) => {}
            (GrammarKind::Combined, _) => diagnostics.push(Diagnostic::error(
                "G4S006",
                unit.span.clone(),
                "combined grammar reached semantics before splitting",
            )),
            (GrammarKind::Lexer, RuleKind::Lexer) | (GrammarKind::Parser, RuleKind::Parser) => {}
        }
    }

    check_source_prequels(unit, diagnostics);

    let rule_names = rules.keys().copied().collect::<BTreeSet<_>>();
    let mut alternative_label_owners = BTreeMap::new();
    for rule in &unit.rules {
        check_rule_options(unit, rule, diagnostics);
        check_block_options(&rule.block, &rule.name, diagnostics);
        visit_elements(&rule.block, &mut |_, _, element| {
            if let ElementKind::RuleCall(call) = &element.kind {
                if !rule_names.contains(call.name.as_str()) {
                    diagnostics.push(Diagnostic::error(
                        "G4S007",
                        element.span.clone(),
                        format!("reference to undefined rule {}", call.name),
                    ));
                }
                if unit.kind == GrammarKind::Lexer
                    && call.name.chars().next().is_some_and(char::is_lowercase)
                {
                    diagnostics.push(Diagnostic::error(
                        "G4S008",
                        element.span.clone(),
                        format!(
                            "parser rule reference {} is not allowed in lexer rule {}",
                            call.name, rule.name
                        ),
                    ));
                }
            }
            if unit.kind == GrammarKind::Parser && matches!(element.kind, ElementKind::Range(..)) {
                diagnostics.push(Diagnostic::error(
                    "G4S009",
                    element.span.clone(),
                    "character ranges are not allowed in parser rules",
                ));
            }
        });
        check_alt_labels(rule, &rules, &mut alternative_label_owners, diagnostics);
        if rule.fragment {
            visit_elements(&rule.block, &mut |_, _, element| {
                if matches!(element.kind, ElementKind::Action { .. }) {
                    diagnostics.push(Diagnostic::warning(
                        "G4S010",
                        element.span.clone(),
                        format!(
                            "fragment rule {} contains an action or predicate which cannot execute",
                            rule.name
                        ),
                    ));
                }
            });
            for command in rule
                .block
                .alternatives
                .iter()
                .flat_map(|alternative| &alternative.commands)
            {
                diagnostics.push(Diagnostic::warning(
                    "G4S010",
                    command.span.clone(),
                    format!(
                        "fragment rule {} contains an action or command which cannot execute",
                        rule.name
                    ),
                ));
            }
        }
    }

    check_named_actions(unit, diagnostics);
    check_modes(sources, unit, diagnostics);
    check_channel_declarations(unit, diagnostics);
}

fn check_alt_labels(
    rule: &Rule,
    rules: &BTreeMap<&str, &Rule>,
    owners: &mut BTreeMap<String, (String, SourceSpan)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let labeled = rule
        .block
        .alternatives
        .iter()
        .filter(|alternative| alternative.label.is_some())
        .count();
    if labeled != 0 && labeled != rule.block.alternatives.len() {
        diagnostics.push(Diagnostic::error(
            "G4S011",
            rule.span.clone(),
            format!("rule {} must label all alternatives or none", rule.name),
        ));
    }
    for alternative in &rule.block.alternatives {
        let Some(label) = &alternative.label else {
            continue;
        };
        let decapitalized = decapitalize(&label.value);
        if let Some(conflict) = rules.get(decapitalized.as_str()) {
            diagnostics.push(
                Diagnostic::error(
                    "G4S012",
                    label.span.clone(),
                    format!(
                        "alternative label {} conflicts with rule {}",
                        label.value, conflict.name
                    ),
                )
                .with_related(conflict.span.clone(), "conflicting rule is here"),
            );
        }
        let normalized = ascii_lowercase(&label.value);
        if let Some((owner, previous_span)) = owners.get(&normalized)
            && owner != &rule.name
        {
            diagnostics.push(
                Diagnostic::error(
                    "G4S013",
                    label.span.clone(),
                    format!("alternative label {} is redefined", label.value),
                )
                .with_related(previous_span.clone(), "first label is here"),
            );
        } else {
            owners
                .entry(normalized)
                .or_insert_with(|| (rule.name.clone(), label.span.clone()));
        }
    }
}

fn check_source_prequels(unit: &GrammarUnit, diagnostics: &mut Vec<Diagnostic>) {
    let mut checked_tokens = BTreeSet::new();
    let mut token_names = BTreeMap::new();
    for prequel in &unit.prequels {
        match prequel {
            GrammarPrequel::Options { declarations, .. } => {
                let options = &unit.options[declarations.clone()];
                check_options(options, GRAMMAR_OPTIONS, diagnostics);
                for option in options {
                    check_case_insensitive_value(option, diagnostics);
                    check_target_language(option, diagnostics);
                }
            }
            GrammarPrequel::Tokens { declarations, .. } => {
                for token in &unit.tokens[declarations.clone()] {
                    checked_tokens.insert(token.id);
                    check_token_declaration(token, &mut token_names, diagnostics);
                }
            }
            GrammarPrequel::Imports { .. } | GrammarPrequel::Channels { .. } => {}
        }
    }
    check_repeated_prequels(&unit.prequels, diagnostics);
    for token in &unit.tokens {
        if checked_tokens.insert(token.id) {
            check_token_declaration(token, &mut BTreeMap::new(), diagnostics);
        }
    }
}

fn check_repeated_prequels(prequels: &[GrammarPrequel], diagnostics: &mut Vec<Diagnostic>) {
    for second in [
        prequels
            .iter()
            .filter(|prequel| matches!(prequel, GrammarPrequel::Options { .. }))
            .nth(1),
        prequels
            .iter()
            .filter(|prequel| matches!(prequel, GrammarPrequel::Imports { .. }))
            .nth(1),
        prequels
            .iter()
            .filter(|prequel| matches!(prequel, GrammarPrequel::Tokens { .. }))
            .nth(1),
    ]
    .into_iter()
    .flatten()
    {
        let span = match second {
            GrammarPrequel::Options { span, .. }
            | GrammarPrequel::Imports { span }
            | GrammarPrequel::Tokens { span, .. }
            | GrammarPrequel::Channels { span, .. } => span,
        };
        diagnostics.push(Diagnostic::error(
            "G4S054",
            span.clone(),
            "repeated grammar prequel spec (options, tokens, or import); please merge",
        ));
    }
}

fn check_rule_options(unit: &GrammarUnit, rule: &Rule, diagnostics: &mut Vec<Diagnostic>) {
    let legal = match rule.kind {
        RuleKind::Parser => &[][..],
        RuleKind::Lexer => &["caseInsensitive", "p", "tokenIndex"][..],
    };
    check_options(&rule.options, legal, diagnostics);
    let global_case_insensitive = unit
        .options
        .iter()
        .find(|option| option.name.value == "caseInsensitive")
        .and_then(|option| parse_boolean_option(&option.value.value))
        .unwrap_or(false);
    for option in &rule.options {
        if option.name.value != "caseInsensitive" {
            continue;
        }
        if let Some(value) = parse_boolean_option(&option.value.value) {
            if rule.kind == RuleKind::Lexer && value == global_case_insensitive {
                diagnostics.push(Diagnostic::warning(
                    "G4S067",
                    option.name.span.clone(),
                    format!(
                        "caseInsensitive lexer rule option is redundant because its value equals the global value ({value})"
                    ),
                ));
            }
        } else {
            check_case_insensitive_value(option, diagnostics);
        }
    }
}

fn check_case_insensitive_value(
    option: &super::model::OptionDecl,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if option.name.value == "caseInsensitive" && parse_boolean_option(&option.value.value).is_none()
    {
        diagnostics.push(Diagnostic::warning(
            "G4S015",
            option.value.span.clone(),
            format!(
                "unsupported option value caseInsensitive={}",
                option.value.value
            ),
        ));
    }
}

fn check_target_language(option: &super::model::OptionDecl, diagnostics: &mut Vec<Diagnostic>) {
    if option.name.value == "language"
        && !SUPPORTED_TARGET_LANGUAGES.contains(&option.value.value.as_str())
    {
        diagnostics.push(Diagnostic::error(
            "G4S014",
            option.value.span.clone(),
            format!(
                "ANTLR cannot generate {} code because the target is not supported",
                option.value.value
            ),
        ));
    }
}

fn parse_boolean_option(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn check_block_options(block: &Block, rule_name: &str, diagnostics: &mut Vec<Diagnostic>) {
    check_options(&block.options, &[], diagnostics);
    for alternative in &block.alternatives {
        for element in &alternative.elements {
            check_misplaced_assoc_option(&element.options, rule_name, diagnostics);
            if let Some(label) = &element.label {
                if matches!(element.kind, ElementKind::Block(_)) {
                    diagnostics.push(Diagnostic::error(
                        "G4S055",
                        label.span.clone(),
                        format!(
                            "label {} assigned to a block which is not a set",
                            label.name
                        ),
                    ));
                }
            }
            match &element.kind {
                ElementKind::RuleCall(_) => {
                    check_assigned_element_options(&element.options, RULE_REF_OPTIONS, diagnostics);
                }
                ElementKind::Terminal(_) => {
                    check_assigned_element_options(&element.options, TOKEN_OPTIONS, diagnostics);
                }
                ElementKind::Set { elements, .. } => {
                    for member in elements {
                        let options = match member {
                            SetElement::Terminal { options, .. }
                            | SetElement::Range { options, .. } => options,
                        };
                        check_misplaced_assoc_option(options, rule_name, diagnostics);
                        check_assigned_element_options(options, TOKEN_OPTIONS, diagnostics);
                    }
                }
                ElementKind::Block(nested) => {
                    check_block_options(nested, rule_name, diagnostics);
                }
                ElementKind::Range(..)
                | ElementKind::Action { .. }
                | ElementKind::Predicate { .. }
                | ElementKind::Epsilon => {}
            }
        }
    }
}

fn check_misplaced_assoc_option(
    options: &[super::model::OptionDecl],
    rule_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for option in options {
        if option.name.value == "assoc" {
            diagnostics.push(Diagnostic::warning(
                "G4S014",
                option.name.span.clone(),
                format!(
                    "rule {rule_name} contains an assoc terminal option in an unrecognized location"
                ),
            ));
        }
    }
}

fn check_assigned_element_options(
    options: &[super::model::OptionDecl],
    legal: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for option in options {
        if !option.value.value.is_empty() && !legal.contains(&option.name.value.as_str()) {
            unsupported_option(option, diagnostics);
        }
    }
}

fn check_options(
    options: &[super::model::OptionDecl],
    legal: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for option in options {
        if !legal.contains(&option.name.value.as_str()) {
            unsupported_option(option, diagnostics);
        }
    }
}

fn unsupported_option(option: &super::model::OptionDecl, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(Diagnostic::warning(
        "G4S014",
        option.name.span.clone(),
        format!("unsupported option {}", option.name.value),
    ));
}

fn check_named_actions(unit: &GrammarUnit, diagnostics: &mut Vec<Diagnostic>) {
    let default_scope = match unit.kind {
        GrammarKind::Lexer => "lexer",
        GrammarKind::Parser | GrammarKind::Combined => "parser",
    };
    let mut actions = BTreeMap::new();
    for action in &unit.actions {
        let key = (
            action.scope.as_deref().unwrap_or(default_scope),
            action.name.as_str(),
        );
        if let Some(previous) = actions.insert(key, action) {
            diagnostics.push(
                Diagnostic::error(
                    "G4S016",
                    named_action_diagnostic_span(action),
                    format!("action {} is redefined", action.name),
                )
                .with_related(
                    named_action_diagnostic_span(previous),
                    "first action is here",
                ),
            );
        }
    }
}

fn named_action_diagnostic_span(action: &super::model::NamedAction) -> SourceSpan {
    let start = action.span.bytes.start.saturating_add(1);
    SourceSpan {
        source: action.span.source,
        bytes: start..action.span.bytes.end.max(start),
    }
}

fn check_token_declaration<'a>(
    token: &'a TokenDeclaration,
    tokens: &mut BTreeMap<&'a str, &'a TokenDeclaration>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !is_token_name(&token.name.value) {
        diagnostics.push(Diagnostic::error(
            "G4S017",
            token.name.span.clone(),
            format!(
                "token name {} must start with an uppercase letter",
                token.name.value
            ),
        ));
    }
    if COMMON_CONSTANTS.contains(&token.name.value.as_str()) {
        diagnostics.push(Diagnostic::error(
            "G4S018",
            token.name.span.clone(),
            format!("token {} uses a reserved name", token.name.value),
        ));
    }
    if let Some(previous) = tokens.insert(token.name.value.as_str(), token) {
        diagnostics.push(
            Diagnostic::warning(
                "G4S019",
                token.name.span.clone(),
                format!("token {} is already defined", token.name.value),
            )
            .with_related(previous.name.span.clone(), "first declaration is here"),
        );
    }
}

fn check_channel_declarations(unit: &GrammarUnit, diagnostics: &mut Vec<Diagnostic>) {
    let mut channels = BTreeMap::new();
    for channel in &unit.channels {
        if COMMON_CONSTANTS.contains(&channel.name.value.as_str()) {
            diagnostics.push(Diagnostic::error(
                "G4S021",
                channel.name.span.clone(),
                format!("channel {} uses a reserved name", channel.name.value),
            ));
        }
        if let Some(previous) = channels.insert(channel.name.value.as_str(), channel) {
            diagnostics.push(
                Diagnostic::error(
                    "G4S022",
                    channel.name.span.clone(),
                    format!("channel {} is already defined", channel.name.value),
                )
                .with_related(previous.name.span.clone(), "first declaration is here"),
            );
        }
    }
}

fn channel_placement_diagnostics(units: &[GrammarUnit]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for unit in units {
        if unit.kind == GrammarKind::Lexer {
            continue;
        }
        for prequel in &unit.prequels {
            if let GrammarPrequel::Channels { span, .. } = prequel {
                diagnostics.push(Diagnostic::error(
                    "G4S020",
                    span.clone(),
                    "channels blocks are only allowed in lexer grammars",
                ));
            }
        }
    }
    diagnostics
}

fn check_modes(sources: &SourceSet, unit: &GrammarUnit, diagnostics: &mut Vec<Diagnostic>) {
    if unit.kind != GrammarKind::Lexer {
        for mode in &unit.modes {
            diagnostics.push(Diagnostic::error(
                "G4S023",
                diagnostic_span_in_root(sources, unit.source, &mode.name_span),
                format!("mode {} is only allowed in lexer grammars", mode.name),
            ));
        }
        return;
    }
    let mut names = BTreeMap::new();
    for mode in &unit.modes {
        if mode.name != "DEFAULT_MODE" && COMMON_CONSTANTS.contains(&mode.name.as_str()) {
            diagnostics.push(Diagnostic::error(
                "G4S024",
                mode.span.clone(),
                format!("mode {} uses a reserved name", mode.name),
            ));
        }
        if let Some(previous) = names.insert(mode.name.as_str(), mode) {
            diagnostics.push(
                Diagnostic::error(
                    "G4S025",
                    mode.span.clone(),
                    format!("mode {} is already defined", mode.name),
                )
                .with_related(previous.span.clone(), "first declaration is here"),
            );
        }
        let has_non_fragment = mode.rules.iter().any(|id| {
            unit.rules
                .iter()
                .find(|rule| rule.id == *id)
                .is_some_and(|rule| !rule.fragment)
        });
        if !has_non_fragment {
            diagnostics.push(Diagnostic::error(
                "G4S026",
                mode.name_span.clone(),
                format!(
                    "lexer mode {} must contain at least one non-fragment rule",
                    mode.name
                ),
            ));
        }
    }
}

fn import_dependency(
    dependency: &IntegratedVocabulary,
    compiled: &BTreeMap<GrammarId, Vocabulary>,
    destination: &mut VocabularyBuilder,
    ids: &mut ModelIdAllocator,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &dependency.source {
        IntegratedVocabularySource::Grammar(grammar) => {
            if let Some(vocabulary) = compiled.get(grammar) {
                destination.import(vocabulary);
            } else {
                let span = dependency
                    .declaration
                    .as_ref()
                    .map(|declaration| declaration.span.clone());
                diagnostics.push(Diagnostic::error_with_optional_span(
                    "G4S027",
                    span,
                    format!(
                        "source vocabulary producer {grammar:?} was not analyzed before its consumer"
                    ),
                ));
            }
        }
        IntegratedVocabularySource::TokensFile(path) => {
            let span = dependency
                .declaration
                .as_ref()
                .map(|declaration| declaration.span.clone());
            let text = match fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) => {
                    diagnostics.push(Diagnostic::error_with_optional_span(
                        "G4S028",
                        span,
                        format!("cannot read token vocabulary {}: {error}", path.display()),
                    ));
                    return;
                }
            };
            // `.tokens` files are generated sidecars rather than grammar
            // source, so the frontend's byte order mark handling never sees
            // them. Drop a leading mark here: U+FEFF is not `char::is_whitespace`,
            // so `trim` would otherwise leave it glued to the first token name.
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            for (line_index, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Some((name, number)) = line.rsplit_once('=') else {
                    diagnostics.push(Diagnostic::error_with_optional_span(
                        "G4S029",
                        span.clone(),
                        format!(
                            "invalid token definition on line {} of {}",
                            line_index + 1,
                            path.display()
                        ),
                    ));
                    continue;
                };
                let name = name.trim();
                let number = match number.trim().parse::<i32>() {
                    Ok(number) if number > INVALID_TOKEN_TYPE => number,
                    _ => {
                        diagnostics.push(Diagnostic::error_with_optional_span(
                            "G4S029",
                            span.clone(),
                            format!(
                                "invalid token type on line {} of {}",
                                line_index + 1,
                                path.display()
                            ),
                        ));
                        continue;
                    }
                };
                if is_grammar_literal(name) {
                    destination.define_literal(name, Some(number), None, ids);
                } else if is_identifier(name) {
                    destination.define_name(name, Some(number), None, ids);
                } else {
                    diagnostics.push(Diagnostic::error_with_optional_span(
                        "G4S029",
                        span.clone(),
                        format!(
                            "invalid token name on line {} of {}",
                            line_index + 1,
                            path.display()
                        ),
                    ));
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct UnitAnalysisContext<'a> {
    sources: &'a SourceSet,
    symbol_unit: &'a GrammarUnit,
    shares_tokens_with_implicit_lexer: bool,
    entry_rules: &'a EntryRuleConfig,
    action_reference_parser: ActionReferenceParser,
}

fn analyze_unit(
    context: UnitAnalysisContext<'_>,
    unit: GrammarUnit,
    mut vocabulary: VocabularyBuilder,
    ids: &mut ModelIdAllocator,
    diagnostics: &mut Vec<Diagnostic>,
) -> SemanticGrammar {
    let UnitAnalysisContext {
        sources,
        symbol_unit,
        shares_tokens_with_implicit_lexer,
        entry_rules,
        action_reference_parser,
    } = context;
    let imported_names = vocabulary.by_name.keys().cloned().collect::<BTreeSet<_>>();
    let mut token_diagnostics = Vec::new();
    vocabulary.define_builtin_eof();
    for declaration in &unit.tokens {
        if imported_names.contains(&declaration.name.value)
            && !(shares_tokens_with_implicit_lexer && declaration.name.span.source == unit.source)
        {
            token_diagnostics.push(Diagnostic::warning(
                "G4S019",
                diagnostic_span_in_root(sources, unit.source, &declaration.name.span),
                format!("token {} is already defined", declaration.name.value),
            ));
        }
        vocabulary.define_name(&declaration.name.value, None, Some(declaration.id), ids);
    }

    match unit.kind {
        GrammarKind::Lexer => assign_lexer_tokens(&unit, &mut vocabulary, ids),
        GrammarKind::Parser => {
            assign_parser_tokens(sources, &unit, &mut vocabulary, ids, &mut token_diagnostics);
        }
        GrammarKind::Combined => unreachable!("combined grammar is split before semantics"),
    }

    let vocabulary = vocabulary.finish();
    let rule_numbers = unit
        .rules
        .iter()
        .enumerate()
        .map(|(index, rule)| (rule.id, index))
        .collect::<BTreeMap<_, _>>();
    let rule_names = unit
        .rules
        .iter()
        .map(|rule| rule.name.clone())
        .collect::<Vec<_>>();
    let rules_by_name = unit
        .rules
        .iter()
        .map(|rule| (rule.name.as_str(), rule.id))
        .collect::<BTreeMap<_, _>>();

    let symbol_diagnostics = check_symbol_conflicts(symbol_unit, &vocabulary);
    let mut mode_diagnostics = Vec::new();
    let (mode_names, mode_numbers) = assign_modes(&unit, &vocabulary, &mut mode_diagnostics);
    let unreachable_diagnostics = check_unreachable_tokens(&unit);
    let mut channel_diagnostics = Vec::new();
    let (channel_names, channel_numbers) =
        assign_channels(&unit, &vocabulary, &mut channel_diagnostics);
    let mut binding_diagnostics = Vec::new();
    let collection = BindingCollector::new(BindingCollectorContext {
        unit: &unit,
        vocabulary: &vocabulary,
        rules_by_name: &rules_by_name,
        channel_numbers: &channel_numbers,
        mode_numbers: &mode_numbers,
        diagnostics: &mut binding_diagnostics,
        action_reference_parser,
    })
    .collect();
    diagnostics.extend(symbol_diagnostics);
    diagnostics.extend(token_diagnostics);
    diagnostics.extend(mode_diagnostics);
    diagnostics.extend(unreachable_diagnostics);
    diagnostics.extend(channel_diagnostics);
    diagnostics.extend(binding_diagnostics);

    let literal_names = name_table(vocabulary.max_token_type(), &vocabulary.by_literal);
    let symbolic_names = symbolic_name_table(&vocabulary);
    let entry_rules = analyze_rule_reachability(&unit, entry_rules).entry_rules;

    let recognizer = RecognizerModel {
        grammar: unit.id,
        name: unit.name.clone(),
        kind: unit.kind,
        rule_names,
        rule_numbers,
        vocabulary,
        literal_names,
        symbolic_names,
        channel_names,
        channel_numbers,
        mode_names,
        mode_numbers,
        action_numbers: collection.action_numbers,
        predicate_numbers: collection.predicate_numbers,
    };
    SemanticGrammar {
        unit,
        recognizer,
        bindings: collection.bindings,
        call_graph: collection.call_graph,
        entry_rules,
    }
}

fn check_unreachable_rules(
    units: &[GrammarUnit],
    target_units: &BTreeSet<GrammarId>,
    entry_config: &EntryRuleConfig,
) -> (Vec<Diagnostic>, BTreeSet<RuleId>) {
    let mut diagnostics = Vec::new();
    let mut entry_rules = BTreeSet::new();
    for unit in units.iter().filter(|unit| target_units.contains(&unit.id)) {
        let reachability = analyze_rule_reachability(unit, entry_config);
        entry_rules.extend(reachability.entry_rules.iter().copied());
        if reachability.unreachable_rules.is_empty() {
            continue;
        }
        let rules = unit
            .rules
            .iter()
            .map(|rule| (rule.id, rule))
            .collect::<BTreeMap<_, _>>();
        let entries = reachability
            .entry_rules
            .iter()
            .filter_map(|rule| rules.get(rule))
            .map(|rule| rule.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        for rule in reachability
            .unreachable_rules
            .iter()
            .filter_map(|rule| rules.get(rule))
        {
            diagnostics.push(Diagnostic::warning(
                "G4S078",
                rule.name_span.clone(),
                format!(
                    "parser rule {} is unreachable from entry rule{} {entries}",
                    rule.name,
                    if reachability.entry_rules.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
            ));
        }
    }
    (diagnostics, entry_rules)
}

fn assign_lexer_tokens(
    unit: &GrammarUnit,
    vocabulary: &mut VocabularyBuilder,
    ids: &mut ModelIdAllocator,
) {
    for rule in &unit.rules {
        if !rule.fragment && !has_type_or_more_command(rule) {
            vocabulary.define_name(&rule.name, None, None, ids);
        }
    }

    let mut conflicting_literals = BTreeSet::new();
    for rule in &unit.rules {
        if rule.fragment {
            continue;
        }
        let Some(literal) = lexer_literal_alias(rule) else {
            continue;
        };
        if vocabulary.by_literal.contains_key(literal) {
            conflicting_literals.insert(literal.to_owned());
        } else {
            vocabulary.define_alias(&rule.name, literal, ids);
        }
    }
    for literal in conflicting_literals {
        vocabulary.remove_literal(&literal);
    }
}

fn assign_parser_tokens(
    sources: &SourceSet,
    unit: &GrammarUnit,
    vocabulary: &mut VocabularyBuilder,
    ids: &mut ModelIdAllocator,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for rule in &unit.rules {
        visit_elements(&rule.block, &mut |_, _, element| match &element.kind {
            ElementKind::Terminal(Terminal::Token(name)) => {
                if name != "EOF" && !vocabulary.by_name.contains_key(name) {
                    diagnostics.push(Diagnostic::warning(
                        "G4S030",
                        diagnostic_span_in_root(sources, unit.source, &element.span),
                        format!("implicit definition of token {name} in parser"),
                    ));
                    vocabulary.define_name(name, None, None, ids);
                }
            }
            ElementKind::Terminal(Terminal::Literal(literal)) => {
                if !vocabulary.by_literal.contains_key(literal) {
                    diagnostics.push(Diagnostic::error(
                            "G4S031",
                            diagnostic_span_in_root(sources, unit.source, &element.span),
                            format!(
                                "cannot create implicit token for string literal {literal} in a parser grammar"
                            ),
                        ));
                }
            }
            ElementKind::Set { elements, .. } => {
                for member in elements {
                    match member {
                        SetElement::Terminal {
                            value: Terminal::Token(name),
                            ..
                        } if name != "EOF" && !vocabulary.by_name.contains_key(name) => {
                            diagnostics.push(Diagnostic::warning(
                                "G4S030",
                                diagnostic_span_in_root(sources, unit.source, &element.span),
                                format!("implicit definition of token {name} in parser"),
                            ));
                            vocabulary.define_name(name, None, None, ids);
                        }
                        SetElement::Terminal {
                            value: Terminal::Literal(literal),
                            ..
                        } if !vocabulary.by_literal.contains_key(literal) => {
                            diagnostics.push(Diagnostic::error(
                                    "G4S031",
                                    diagnostic_span_in_root(sources, unit.source, &element.span),
                                    format!(
                                        "cannot create implicit token for string literal {literal} in a parser grammar"
                                    ),
                                ));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        });
    }
}

fn assign_channels(
    unit: &GrammarUnit,
    vocabulary: &Vocabulary,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<Option<String>>, BTreeMap<String, i32>) {
    if unit.kind != GrammarKind::Lexer {
        return (Vec::new(), BTreeMap::new());
    }
    let mut names = vec![
        Some("DEFAULT_TOKEN_CHANNEL".to_owned()),
        Some("HIDDEN".to_owned()),
    ];
    let mut numbers = BTreeMap::from([
        ("DEFAULT_TOKEN_CHANNEL".to_owned(), 0),
        ("HIDDEN".to_owned(), 1),
    ]);
    let modes = unit
        .modes
        .iter()
        .map(|mode| mode.name.as_str())
        .collect::<BTreeSet<_>>();
    if !unit.channels.is_empty() {
        names.extend([None, None]);
    }
    for (index, channel) in unit.channels.iter().enumerate() {
        let name = &channel.name.value;
        if vocabulary.by_name.contains_key(name) {
            diagnostics.push(Diagnostic::error(
                "G4S032",
                channel.name.span.clone(),
                format!("channel {name} conflicts with a token"),
            ));
        }
        if modes.contains(name.as_str()) {
            diagnostics.push(Diagnostic::error(
                "G4S033",
                channel.name.span.clone(),
                format!("channel {name} conflicts with a mode"),
            ));
        }
        let number = MIN_USER_CHANNEL + i32::try_from(index).expect("channel count exceeds i32");
        numbers.entry(name.clone()).or_insert(number);
        names.push(Some(name.clone()));
    }
    (names, numbers)
}

fn assign_modes(
    unit: &GrammarUnit,
    vocabulary: &Vocabulary,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<String>, BTreeMap<String, usize>) {
    if unit.kind != GrammarKind::Lexer {
        return (Vec::new(), BTreeMap::new());
    }
    let mut names = vec!["DEFAULT_MODE".to_owned()];
    let mut numbers = BTreeMap::from([("DEFAULT_MODE".to_owned(), 0)]);
    for mode in &unit.modes {
        if vocabulary.by_name.contains_key(&mode.name) {
            diagnostics.push(Diagnostic::error(
                "G4S034",
                mode.span.clone(),
                format!("mode {} conflicts with a token", mode.name),
            ));
        }
        let index = names.len();
        numbers.entry(mode.name.clone()).or_insert(index);
        names.push(mode.name.clone());
    }
    (names, numbers)
}

fn check_unreachable_tokens(unit: &GrammarUnit) -> Vec<Diagnostic> {
    if unit.kind != GrammarKind::Lexer {
        return Vec::new();
    }
    let default_rules = unit
        .rules
        .iter()
        .filter(|rule| rule.mode.is_none())
        .collect::<Vec<_>>();
    let mut modes = vec![default_rules];
    for mode in &unit.modes {
        modes.push(
            mode.rules
                .iter()
                .filter_map(|id| unit.rules.iter().find(|rule| rule.id == *id))
                .collect(),
        );
    }

    let mut diagnostics = Vec::new();
    for rules in modes {
        let literal_rules = rules
            .into_iter()
            .filter(|rule| !rule.name.starts_with("T__"))
            .filter_map(|rule| {
                let values = simple_literal_alternatives(rule);
                (!values.is_empty()).then_some((rule, values))
            })
            .collect::<Vec<_>>();
        for (index, (first_rule, first_values)) in literal_rules.iter().enumerate() {
            report_literal_overlaps(
                first_rule,
                first_rule,
                first_values,
                first_values,
                &mut diagnostics,
            );
            if first_rule.fragment {
                continue;
            }
            for (second_rule, second_values) in &literal_rules[index + 1..] {
                if !second_rule.fragment {
                    report_literal_overlaps(
                        first_rule,
                        second_rule,
                        first_values,
                        second_values,
                        &mut diagnostics,
                    );
                }
            }
        }
    }
    diagnostics
}

fn simple_literal_alternatives(rule: &Rule) -> Vec<Vec<i32>> {
    rule.block
        .alternatives
        .iter()
        .filter_map(|alternative| {
            let mut value = Vec::new();
            for element in &alternative.elements {
                let ElementKind::Terminal(Terminal::Literal(literal)) = &element.kind else {
                    return None;
                };
                if element.quantifier != Quantifier::One {
                    return None;
                }
                value.extend(decode_string_literal(literal).ok()?);
            }
            Some(value)
        })
        .collect()
}

fn report_literal_overlaps(
    first_rule: &Rule,
    second_rule: &Rule,
    first_values: &[Vec<i32>],
    second_values: &[Vec<i32>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (first_index, first) in first_values.iter().enumerate() {
        let second_start = if first_rule.id == second_rule.id {
            first_index + 1
        } else {
            0
        };
        for second in &second_values[second_start..] {
            if first == second {
                diagnostics.push(Diagnostic::warning(
                    "G4S069",
                    second_rule.name_span.clone(),
                    format!(
                        "token {} is unreachable because value is matched by {}",
                        second_rule.name, first_rule.name
                    ),
                ));
            }
        }
    }
}

#[derive(Clone, Debug)]
struct MutableToken {
    id: TokenSymbolId,
    number: i32,
    name: Option<String>,
    literal: Option<String>,
}

#[derive(Clone, Debug)]
struct VocabularyBuilder {
    by_number: BTreeMap<i32, MutableToken>,
    by_name: BTreeMap<String, i32>,
    by_literal: BTreeMap<String, i32>,
    name_order: Vec<String>,
    literal_order: Vec<String>,
    next: i32,
}

impl VocabularyBuilder {
    const fn new() -> Self {
        Self {
            by_number: BTreeMap::new(),
            by_name: BTreeMap::new(),
            by_literal: BTreeMap::new(),
            name_order: Vec::new(),
            literal_order: Vec::new(),
            next: 1,
        }
    }

    fn define_builtin_eof(&mut self) {
        self.by_name.insert("EOF".to_owned(), EOF_TOKEN_TYPE);
    }

    fn import(&mut self, vocabulary: &Vocabulary) {
        for token in &vocabulary.tokens {
            let entry = self
                .by_number
                .entry(token.number)
                .or_insert_with(|| MutableToken {
                    id: token.id,
                    number: token.number,
                    name: None,
                    literal: None,
                });
            if entry.name.is_none() {
                entry.name.clone_from(&token.name);
            }
            if entry.literal.is_none() {
                entry.literal.clone_from(&token.literal);
            }
            self.next = self.next.max(token.number.saturating_add(1));
        }
        for name in &vocabulary.name_order {
            let number = vocabulary.by_name[name];
            if !self.by_name.contains_key(name) {
                self.by_name.insert(name.clone(), number);
                self.name_order.push(name.clone());
            }
        }
        for (name, number) in &vocabulary.by_name {
            if !self.by_name.contains_key(name) {
                self.by_name.insert(name.clone(), *number);
                if name != "EOF" {
                    self.name_order.push(name.clone());
                }
            }
        }
        for literal in &vocabulary.literal_order {
            let number = vocabulary.by_literal[literal];
            if !self.by_literal.contains_key(literal) {
                self.by_literal.insert(literal.clone(), number);
                self.literal_order.push(literal.clone());
            }
        }
        for (literal, number) in &vocabulary.by_literal {
            if !self.by_literal.contains_key(literal) {
                self.by_literal.insert(literal.clone(), *number);
                self.literal_order.push(literal.clone());
            }
        }
    }

    fn define_name(
        &mut self,
        name: &str,
        number: Option<i32>,
        preferred_id: Option<TokenSymbolId>,
        ids: &mut ModelIdAllocator,
    ) -> i32 {
        if let Some(number) = self.by_name.get(name) {
            return *number;
        }
        let number = number.unwrap_or(self.next);
        let entry = self
            .by_number
            .entry(number)
            .or_insert_with(|| MutableToken {
                id: preferred_id.unwrap_or_else(|| ids.token()),
                number,
                name: None,
                literal: None,
            });
        if entry.name.is_none() {
            entry.name = Some(name.to_owned());
        }
        self.by_name.insert(name.to_owned(), number);
        self.name_order.push(name.to_owned());
        self.next = self.next.max(number.saturating_add(1));
        number
    }

    fn define_literal(
        &mut self,
        literal: &str,
        number: Option<i32>,
        preferred_id: Option<TokenSymbolId>,
        ids: &mut ModelIdAllocator,
    ) -> i32 {
        if let Some(number) = self.by_literal.get(literal) {
            return *number;
        }
        let number = number.unwrap_or(self.next);
        let entry = self
            .by_number
            .entry(number)
            .or_insert_with(|| MutableToken {
                id: preferred_id.unwrap_or_else(|| ids.token()),
                number,
                name: None,
                literal: None,
            });
        if entry.literal.is_none() {
            entry.literal = Some(literal.to_owned());
        }
        self.by_literal.insert(literal.to_owned(), number);
        self.literal_order.push(literal.to_owned());
        self.next = self.next.max(number.saturating_add(1));
        number
    }

    fn define_alias(&mut self, name: &str, literal: &str, ids: &mut ModelIdAllocator) {
        let number = self.define_name(name, None, None, ids);
        if self.by_literal.insert(literal.to_owned(), number).is_none() {
            self.literal_order.push(literal.to_owned());
            if let Some(token) = self.by_number.get_mut(&number) {
                token.literal = Some(literal.to_owned());
            }
        }
    }

    fn remove_literal(&mut self, literal: &str) {
        let Some(number) = self.by_literal.remove(literal) else {
            return;
        };
        if let Some(token) = self.by_number.get_mut(&number) {
            if token.literal.as_deref() == Some(literal) {
                token.literal = None;
            }
        }
        self.literal_order.retain(|candidate| candidate != literal);
    }

    fn finish(self) -> Vocabulary {
        let tokens = self
            .by_number
            .into_values()
            .map(|token| TokenSymbol {
                id: token.id,
                number: token.number,
                name: token.name,
                literal: token.literal,
            })
            .collect::<Vec<_>>();
        Vocabulary {
            tokens,
            by_name: self.by_name,
            by_literal: self.by_literal,
            name_order: self.name_order,
            literal_order: self.literal_order,
        }
    }
}

fn name_table(max_token_type: i32, names: &BTreeMap<String, i32>) -> Vec<Option<String>> {
    let len = usize::try_from(max_token_type.saturating_add(1)).unwrap_or(0);
    let mut table = vec![None; len];
    for (name, number) in names {
        let Ok(index) = usize::try_from(*number) else {
            continue;
        };
        if let Some(slot) = table.get_mut(index) {
            if slot.is_none() {
                *slot = Some(name.clone());
            }
        }
    }
    table
}

fn symbolic_name_table(vocabulary: &Vocabulary) -> Vec<Option<String>> {
    let len = usize::try_from(vocabulary.max_token_type().saturating_add(1)).unwrap_or(0);
    let mut table = vec![None; len];
    for token in &vocabulary.tokens {
        let Some(name) = &token.name else {
            continue;
        };
        if name.starts_with("T__") {
            continue;
        }
        if let Ok(index) = usize::try_from(token.number) {
            if let Some(slot) = table.get_mut(index) {
                *slot = Some(name.clone());
            }
        }
    }
    table
}

fn has_type_or_more_command(rule: &Rule) -> bool {
    rule.block.alternatives.iter().any(|alternative| {
        alternative
            .commands
            .iter()
            .any(|command| matches!(command.name.as_str(), "type" | "more"))
    })
}

fn lexer_literal_alias(rule: &Rule) -> Option<&str> {
    let [alternative] = rule.block.alternatives.as_slice() else {
        return None;
    };
    let (first, rest) = alternative.elements.split_first()?;
    if first.quantifier != Quantifier::One {
        return None;
    }
    let ElementKind::Terminal(Terminal::Literal(literal)) = &first.kind else {
        return None;
    };
    let plain_alias = alternative.commands.is_empty()
        && matches!(
            rest,
            [] | [Element {
                kind: ElementKind::Action { .. } | ElementKind::Predicate { .. },
                ..
            }]
        );
    let command_alias = rest.is_empty()
        && match alternative.commands.as_slice() {
            [_] => true,
            [first, second] => first.argument.is_none() || second.argument.is_none(),
            _ => false,
        };
    (plain_alias || command_alias)
        .then_some(literal)
        .map(String::as_str)
}

fn visit_elements(block: &Block, visitor: &mut impl FnMut(&Alternative, usize, &Element)) {
    for alternative in &block.alternatives {
        for (index, element) in alternative.elements.iter().enumerate() {
            visitor(alternative, index, element);
            if let ElementKind::Block(nested) = &element.kind {
                visit_elements(nested, visitor);
            }
        }
    }
}

fn is_token_name(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn is_grammar_literal(name: &str) -> bool {
    name.starts_with('\'') && name.ends_with('\'') && name.len() >= 2
}

fn decapitalize(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    first.to_lowercase().chain(characters).collect()
}

fn ascii_lowercase(value: &str) -> String {
    value
        .chars()
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::grammar::integration::integrate_loaded;
    use crate::grammar::loader::{LoadOptions, load};
    use crate::grammar::model::ResolvedLexerCommand;

    #[test]
    fn combined_grammar_numbers_implicit_literals_before_lexer_rules() {
        let fixture = Fixture::new("combined-numbering");
        fixture.write(
            "Mini.g4",
            r#"
grammar Mini;
start : 'if' ID;
ID : 'id';
WS : [ \t]+ -> skip;
"#,
        );

        let semantic = compile(&fixture, "Mini.g4").expect("combined grammar should analyze");
        let lexer = semantic
            .grammars
            .iter()
            .find(|grammar| grammar.unit.kind == GrammarKind::Lexer)
            .expect("implicit lexer");
        let parser = semantic
            .grammars
            .iter()
            .find(|grammar| grammar.unit.kind == GrammarKind::Parser)
            .expect("combined parser");

        assert_eq!(lexer.recognizer.rule_names, ["T__0", "ID", "WS"]);
        assert_eq!(
            lexer.recognizer.literal_names,
            [None, Some("'if'".to_owned()), Some("'id'".to_owned()), None]
        );
        assert_eq!(
            lexer.recognizer.symbolic_names,
            [None, None, Some("ID".to_owned()), Some("WS".to_owned())]
        );
        assert_eq!(
            parser.recognizer.vocabulary.by_literal["'if'"],
            lexer.recognizer.vocabulary.by_literal["'if'"]
        );
        assert_eq!(parser.entry_rules, [parser.unit.rules[0].id]);
    }

    #[test]
    fn source_vocabulary_is_analyzed_before_parser_bindings() {
        let fixture = Fixture::new("source-vocabulary");
        fixture.write(
            "Root.g4",
            "parser grammar Root; options { tokenVocab=Lex; } root : ID;",
        );
        fixture.write("Lex.g4", "lexer grammar Lex; ID : 'id';");

        let semantic = compile(&fixture, "Root.g4").expect("source vocabulary should analyze");
        assert_eq!(
            semantic
                .grammars
                .iter()
                .map(|grammar| grammar.unit.name.as_str())
                .collect::<Vec<_>>(),
            ["Lex", "Root"]
        );
        let parser = &semantic.grammars[1];
        assert_eq!(parser.recognizer.vocabulary.by_name["ID"], 1);
        let terminal = parser.unit.rules[0].block.alternatives[0].elements[0].id;
        assert_eq!(parser.bindings.terminals[&terminal].token_type, 1);
    }

    #[test]
    fn basic_errors_stop_before_left_recursion_and_numbering() {
        let fixture = Fixture::new("basic-stop");
        fixture.write("Broken.g4", "parser grammar Broken; a : a b | 'x';");
        let error = compile(&fixture, "Broken.g4").expect_err("undefined b must be fatal");
        assert!(
            error.diagnostics().iter().any(|diagnostic| {
                diagnostic.code == "G4S007" && diagnostic.message.contains('b')
            })
        );
        assert!(
            error
                .diagnostics()
                .iter()
                .all(|diagnostic| !diagnostic.code.starts_with("G4R"))
        );
    }

    #[test]
    fn repeated_alternative_label_within_one_rule_is_allowed() {
        compile_committed_fixture("common-alternative-label/Labels.g4")
            .expect("ANTLR permits a common alternative label within one rule");
    }

    #[test]
    fn alternative_label_reused_by_another_rule_is_rejected() {
        let error = compile_committed_fixture("cross-rule-alternative-label/Labels.g4")
            .expect_err("alternative label ownership is rule-scoped");
        insta::assert_debug_snapshot!(
            "cross_rule_alternative_label_diagnostic",
            error.diagnostics()
        );
        assert_eq!(
            error
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code == "G4S013")
                .count(),
            1
        );
    }

    #[test]
    fn actions_predicates_labels_and_attributes_keep_structural_owners() {
        let fixture = Fixture::new("bindings");
        fixture.write(
            "Bindings.g4",
            r#"
parser grammar Bindings;
tokens { ID }
entry[int x] returns [int y] locals [boolean seen]
    : item=ID { $y = $x; } { $item.text != ""; }?
    ;
"#,
        );
        let semantic = compile(&fixture, "Bindings.g4").expect("bindings should analyze");
        let grammar = &semantic.grammars[0];
        let rule = &grammar.unit.rules[0];
        let attributes = &grammar.bindings.attributes[&rule.id];
        assert_eq!(attributes.arguments[0].name, "x");
        assert_eq!(attributes.returns[0].name, "y");
        assert_eq!(attributes.locals[0].name, "seen");
        assert_eq!(grammar.bindings.labels.len(), 1);
        assert_eq!(grammar.bindings.actions.len(), 1);
        assert_eq!(grammar.bindings.predicates.len(), 1);
        assert!(
            grammar
                .bindings
                .predicates
                .values()
                .all(|binding| binding.rule == rule.id && binding.context_dependent)
        );
    }

    #[test]
    fn attribute_declarations_support_target_type_syntax() {
        let cases = [
            ("int[] i, int j[]", [("i", "int[]"), ("j", "int []")]),
            (
                "Map<A,List<B>>[] value, int count = other[3]",
                [("value", "Map<A,List<B>>[]"), ("count", "int")],
            ),
            (
                "x:T?, f:func(array[3] of int)",
                [("x", "T?"), ("f", "func(array[3] of int)")],
            ),
            (
                "std::vector<std::string> values, map[string]int lookup",
                [
                    ("values", "std::vector<std::string>"),
                    ("lookup", "map[string]int"),
                ],
            ),
        ];

        for (input, expected) in cases {
            let actual = parse_attribute_declarations(input)
                .into_iter()
                .map(|declaration| (declaration.name, declaration.ty.unwrap_or_default()))
                .collect::<Vec<_>>();
            let expected = expected
                .into_iter()
                .map(|(name, ty)| (name.to_owned(), ty.to_owned()))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "input {input:?}");
        }
    }

    #[test]
    fn attribute_initializers_allow_comparison_operators() {
        let actual = parse_attribute_declarations(
            "Map<A,List<B>> values = defaults, boolean less = other < 0, \
             boolean greater = other > 0, boolean atMost = other <= 0, \
             boolean atLeast = other >= 0, int count",
        )
        .into_iter()
        .map(|declaration| {
            (
                declaration.name,
                declaration.ty.unwrap_or_default(),
                declaration.initializer,
            )
        })
        .collect::<Vec<_>>();

        assert_eq!(
            actual,
            [
                (
                    "values".to_owned(),
                    "Map<A,List<B>>".to_owned(),
                    Some("defaults".to_owned()),
                ),
                (
                    "less".to_owned(),
                    "boolean".to_owned(),
                    Some("other < 0".to_owned()),
                ),
                (
                    "greater".to_owned(),
                    "boolean".to_owned(),
                    Some("other > 0".to_owned()),
                ),
                (
                    "atMost".to_owned(),
                    "boolean".to_owned(),
                    Some("other <= 0".to_owned()),
                ),
                (
                    "atLeast".to_owned(),
                    "boolean".to_owned(),
                    Some("other >= 0".to_owned()),
                ),
                ("count".to_owned(), "int".to_owned(), None),
            ]
        );
    }

    #[test]
    fn lexer_channels_preserve_java_interp_holes_and_commands_resolve() {
        let fixture = Fixture::new("channels");
        fixture.write(
            "Channels.g4",
            r#"
lexer grammar Channels;
channels { COMMENTS }
A : 'a' -> channel(COMMENTS);
"#,
        );
        let semantic = compile(&fixture, "Channels.g4").expect("lexer should analyze");
        let grammar = &semantic.grammars[0];
        assert_eq!(
            grammar.recognizer.channel_names,
            [
                Some("DEFAULT_TOKEN_CHANNEL".to_owned()),
                Some("HIDDEN".to_owned()),
                None,
                None,
                Some("COMMENTS".to_owned()),
            ]
        );
        assert_eq!(grammar.recognizer.channel_numbers["COMMENTS"], 2);
        assert!(
            grammar
                .bindings
                .commands
                .values()
                .any(|binding| { binding.command == ResolvedLexerCommand::Channel(2) })
        );
    }

    #[test]
    fn capitalized_standard_lexer_commands_resolve() {
        let fixture = Fixture::new("capitalized-commands");
        fixture.write(
            "Commands.g4",
            r#"
lexer grammar Commands;
tokens { REMAPPED }
channels { COMMENTS }
A : 'a' -> Skip;
B : 'b' -> More;
C : 'c' -> PopMode;
D : 'd' -> Mode(DEFAULT_MODE);
E : 'e' -> PushMode(OTHER);
F : 'f' -> Type(REMAPPED);
G : 'g' -> Channel(COMMENTS);
mode OTHER;
H : 'h';
"#,
        );

        let semantic = compile(&fixture, "Commands.g4").expect("lexer should analyze");
        let grammar = &semantic.grammars[0];
        let commands = grammar
            .bindings
            .commands
            .values()
            .map(|binding| binding.command)
            .collect::<Vec<_>>();
        insta::assert_debug_snapshot!(
            "capitalized_standard_lexer_commands",
            (&grammar.recognizer.vocabulary.by_name, commands)
        );
    }

    #[test]
    fn capitalized_lexer_commands_keep_exact_spelling_diagnostics() {
        let fixture = Fixture::new("capitalized-command-diagnostics");
        fixture.write(
            "Commands.g4",
            r#"
lexer grammar Commands;
channels { COMMENTS }
A : 'a' -> Skip(1);
B : 'b' -> Channel;
C : 'c' -> CHANNEL(COMMENTS);
D : 'd' -> Channel(COMMENTS), Channel(COMMENTS);
E : 'e' -> channel(COMMENTS), Channel(COMMENTS);
F : 'f' -> skip, Channel(COMMENTS);
"#,
        );

        let error = compile(&fixture, "Commands.g4").expect_err("invalid commands must fail");
        let diagnostics = error
            .diagnostics()
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.code,
                    diagnostic.severity,
                    diagnostic.message.as_str(),
                )
            })
            .collect::<Vec<_>>();
        insta::assert_debug_snapshot!("capitalized_lexer_command_diagnostics", diagnostics);
    }

    #[test]
    fn fragment_literal_rules_do_not_consume_token_types() {
        let fixture = Fixture::new("fragment-token-types");
        fixture.write(
            "Fragments.g4",
            "lexer grammar Fragments; fragment A : 'a'; fragment B : A | 'b';",
        );
        let semantic = compile(&fixture, "Fragments.g4").expect("lexer should analyze");
        let grammar = &semantic.grammars[0];
        assert_eq!(grammar.recognizer.vocabulary.max_token_type(), 0);
        assert!(!grammar.recognizer.vocabulary.by_name.contains_key("A"));
        assert!(!grammar.recognizer.vocabulary.by_name.contains_key("B"));
        assert!(grammar.recognizer.vocabulary.by_literal.is_empty());
    }

    fn compile(fixture: &Fixture, root: &str) -> Result<SemanticGrammarSet, CompilationError> {
        compile_root(fixture.path(root))
    }

    fn compile_committed_fixture(relative: &str) -> Result<SemanticGrammarSet, CompilationError> {
        compile_root(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/grammar-semantics")
                .join(relative),
        )
    }

    fn compile_root(root: PathBuf) -> Result<SemanticGrammarSet, CompilationError> {
        let loaded = load(LoadOptions {
            roots: vec![root],
            library_directories: Vec::new(),
        })?;
        analyze(&loaded.sources, integrate_loaded(&loaded)?)
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "antlr-rust-phase-b-semantics-{name}-{}-{serial}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("fixture directory");
            Self { root }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.path(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("fixture parent");
            }
            fs::write(path, text).expect("fixture contents");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
