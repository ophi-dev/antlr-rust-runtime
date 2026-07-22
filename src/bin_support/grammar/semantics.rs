use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crate::embedded::parse_scope_decls;

use super::diagnostic::{CompilationError, Diagnostic, Severity};
use super::left_recursion::rewrite_immediate_left_recursion;
use super::model::{
    ActionBinding, ActionId, Alternative, AttributeClause, AttributeSymbol, Block, Element,
    ElementKind, GrammarId, GrammarKind, GrammarPrequel, GrammarUnit, LabelBinding, LabelKind,
    LexerCommandBinding, ModelIdAllocator, PredicateBinding, Quantifier, RecognizerModel,
    ResolvedLexerCommand, Rule, RuleAttributes, RuleCallBinding, RuleId, RuleKind,
    SemanticBindings, SemanticGrammar, SetElement, Terminal, TerminalBinding, TokenDeclaration,
    TokenSymbol, TokenSymbolId, Vocabulary,
};
use super::provenance::ProvenanceIndex;
use super::transform::{
    IntegratedGrammarSet, IntegratedVocabulary, IntegratedVocabularySource, RootOutputs,
};

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
    mut integrated: IntegratedGrammarSet,
) -> Result<SemanticGrammarSet, CompilationError> {
    let mut diagnostics = std::mem::take(&mut integrated.diagnostics);
    diagnostics.extend(basic_checks(&integrated.grammar.units));
    if has_errors(&diagnostics) {
        return Err(CompilationError::new(diagnostics));
    }

    diagnostics.extend(rewrite_immediate_left_recursion(
        &mut integrated.grammar.units,
        &mut integrated.ids,
        &mut integrated.grammar.provenance,
    ));
    if has_errors(&diagnostics) {
        return Err(CompilationError::new(diagnostics));
    }

    let mut vocabularies = BTreeMap::new();
    let mut grammars = Vec::with_capacity(integrated.grammar.units.len());
    for unit in std::mem::take(&mut integrated.grammar.units) {
        let dependencies = integrated
            .vocabularies
            .iter()
            .filter(|dependency| dependency.consumer == unit.id)
            .collect::<Vec<_>>();
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

        let semantic = analyze_unit(unit, imported, &mut integrated.ids, &mut diagnostics);
        vocabularies.insert(semantic.unit.id, semantic.recognizer.vocabulary.clone());
        grammars.push(semantic);
    }

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

fn basic_checks(units: &[GrammarUnit]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for unit in units {
        check_unit_basics(unit, &mut diagnostics);
    }
    diagnostics
}

fn check_unit_basics(unit: &GrammarUnit, diagnostics: &mut Vec<Diagnostic>) {
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
            (GrammarKind::Parser, RuleKind::Lexer) => diagnostics.push(Diagnostic::error(
                "G4S005",
                rule.span.clone(),
                format!(
                    "lexer rule {} is not allowed in a parser grammar",
                    rule.name
                ),
            )),
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
    for rule in &unit.rules {
        check_rule_options(rule, diagnostics);
        check_block_options(&rule.block, diagnostics);
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
        check_alt_labels(rule, &rules, diagnostics);
        if rule.fragment {
            visit_elements(&rule.block, &mut |_, _, element| {
                if matches!(
                    element.kind,
                    ElementKind::Action { .. } | ElementKind::Predicate { .. }
                ) {
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
            if rule
                .block
                .alternatives
                .iter()
                .any(|alternative| !alternative.commands.is_empty())
            {
                diagnostics.push(Diagnostic::warning(
                    "G4S010",
                    rule.span.clone(),
                    format!(
                        "fragment rule {} contains a command which cannot execute",
                        rule.name
                    ),
                ));
            }
        }
    }

    check_named_actions(unit, diagnostics);
    check_channel_declarations(unit, diagnostics);
    check_modes(unit, diagnostics);
}

fn check_alt_labels(rule: &Rule, rules: &BTreeMap<&str, &Rule>, diagnostics: &mut Vec<Diagnostic>) {
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
    let mut labels = BTreeMap::new();
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
        if let Some(previous) = labels.insert(ascii_lowercase(&label.value), label) {
            diagnostics.push(
                Diagnostic::error(
                    "G4S013",
                    label.span.clone(),
                    format!("alternative label {} is redefined", label.value),
                )
                .with_related(previous.span.clone(), "first label is here"),
            );
        }
    }
}

fn check_source_prequels(unit: &GrammarUnit, diagnostics: &mut Vec<Diagnostic>) {
    let mut checked_tokens = BTreeSet::new();
    let mut token_names = BTreeMap::new();
    for prequel in &unit.prequels {
        match prequel {
            GrammarPrequel::Options { declarations, .. } => {
                check_options(
                    &unit.options[declarations.clone()],
                    GRAMMAR_OPTIONS,
                    diagnostics,
                );
            }
            GrammarPrequel::Tokens { declarations, .. } => {
                for token in &unit.tokens[declarations.clone()] {
                    checked_tokens.insert(token.id);
                    check_token_declaration(token, &mut token_names, diagnostics);
                }
            }
            GrammarPrequel::Imports { .. } => {}
        }
    }
    check_repeated_prequels(&unit.prequels, diagnostics);
    for token in &unit.tokens {
        if checked_tokens.insert(token.id) {
            check_token_declaration(token, &mut token_names, diagnostics);
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
            | GrammarPrequel::Tokens { span, .. } => span,
        };
        diagnostics.push(Diagnostic::error(
            "G4S054",
            span.clone(),
            "repeated grammar prequel spec (options, tokens, or import); please merge",
        ));
    }
}

fn check_rule_options(rule: &Rule, diagnostics: &mut Vec<Diagnostic>) {
    let legal = match rule.kind {
        RuleKind::Parser => &[][..],
        RuleKind::Lexer => &["caseInsensitive", "p", "tokenIndex"][..],
    };
    check_options(&rule.options, legal, diagnostics);
    for option in &rule.options {
        if option.name.value == "caseInsensitive"
            && !matches!(option.value.value.as_str(), "true" | "false")
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
}

fn check_block_options(block: &Block, diagnostics: &mut Vec<Diagnostic>) {
    check_options(&block.options, &[], diagnostics);
    for alternative in &block.alternatives {
        for element in &alternative.elements {
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
                        check_assigned_element_options(options, TOKEN_OPTIONS, diagnostics);
                    }
                }
                ElementKind::Block(nested) => check_block_options(nested, diagnostics),
                ElementKind::Range(..)
                | ElementKind::Action { .. }
                | ElementKind::Predicate { .. }
                | ElementKind::Epsilon => {}
            }
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
                    action.span.clone(),
                    format!("action {} is redefined", action.name),
                )
                .with_related(previous.span.clone(), "first action is here"),
            );
        }
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
    if unit.kind != GrammarKind::Lexer && !unit.channels.is_empty() {
        for channel in &unit.channels {
            diagnostics.push(Diagnostic::error(
                "G4S020",
                channel.name.span.clone(),
                "channels blocks are only allowed in lexer grammars",
            ));
        }
    }
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

fn check_modes(unit: &GrammarUnit, diagnostics: &mut Vec<Diagnostic>) {
    if unit.kind != GrammarKind::Lexer {
        for mode in &unit.modes {
            diagnostics.push(Diagnostic::error(
                "G4S023",
                mode.span.clone(),
                format!("mode {} is only allowed in lexer grammars", mode.name),
            ));
        }
        return;
    }
    let mut names = BTreeMap::new();
    for mode in &unit.modes {
        if COMMON_CONSTANTS.contains(&mode.name.as_str()) {
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
                mode.span.clone(),
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
                let span = dependency.declaration.as_ref().map_or_else(
                    || super::frontend::SourceSpan::empty(super::frontend::SourceId::new(0)),
                    |declaration| declaration.span.clone(),
                );
                diagnostics.push(Diagnostic::error(
                    "G4S027",
                    span,
                    format!(
                        "source vocabulary producer {grammar:?} was not analyzed before its consumer"
                    ),
                ));
            }
        }
        IntegratedVocabularySource::TokensFile(path) => {
            let span = dependency.declaration.as_ref().map_or_else(
                || super::frontend::SourceSpan::empty(super::frontend::SourceId::new(0)),
                |declaration| declaration.span.clone(),
            );
            let text = match fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) => {
                    diagnostics.push(Diagnostic::error(
                        "G4S028",
                        span,
                        format!("cannot read token vocabulary {}: {error}", path.display()),
                    ));
                    return;
                }
            };
            for (line_index, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Some((name, number)) = line.rsplit_once('=') else {
                    diagnostics.push(Diagnostic::error(
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
                        diagnostics.push(Diagnostic::error(
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
                    diagnostics.push(Diagnostic::error(
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

fn analyze_unit(
    unit: GrammarUnit,
    mut vocabulary: VocabularyBuilder,
    ids: &mut ModelIdAllocator,
    diagnostics: &mut Vec<Diagnostic>,
) -> SemanticGrammar {
    vocabulary.define_builtin_eof();
    for declaration in &unit.tokens {
        if vocabulary.by_name.contains_key(&declaration.name.value) {
            diagnostics.push(Diagnostic::warning(
                "G4S019",
                declaration.name.span.clone(),
                format!("token {} is already defined", declaration.name.value),
            ));
        }
        vocabulary.define_name(&declaration.name.value, None, Some(declaration.id), ids);
    }

    match unit.kind {
        GrammarKind::Lexer => assign_lexer_tokens(&unit, &mut vocabulary, ids),
        GrammarKind::Parser => {
            assign_parser_tokens(&unit, &mut vocabulary, ids, diagnostics);
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

    let (channel_names, channel_numbers) = assign_channels(&unit, &vocabulary, diagnostics);
    let (mode_names, mode_numbers) = assign_modes(&unit, &vocabulary, diagnostics);
    let collection = BindingCollector::new(
        &unit,
        &vocabulary,
        &rules_by_name,
        &channel_numbers,
        &mode_numbers,
        diagnostics,
    )
    .collect();

    let literal_names = name_table(vocabulary.max_token_type(), &vocabulary.by_literal);
    let symbolic_names = symbolic_name_table(&vocabulary);
    let entry_rules = if unit.kind == GrammarKind::Parser {
        unit.rules
            .iter()
            .filter(|rule| {
                !collection
                    .call_graph
                    .values()
                    .any(|targets| targets.contains(&rule.id))
            })
            .map(|rule| rule.id)
            .collect()
    } else {
        Vec::new()
    };

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
                        element.span.clone(),
                        format!("implicit definition of token {name} in parser"),
                    ));
                    vocabulary.define_name(name, None, None, ids);
                }
            }
            ElementKind::Terminal(Terminal::Literal(literal)) => {
                if !vocabulary.by_literal.contains_key(literal) {
                    diagnostics.push(Diagnostic::error(
                            "G4S031",
                            element.span.clone(),
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
                                element.span.clone(),
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
                                    element.span.clone(),
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

fn attribute_symbols(clause: &AttributeClause) -> Vec<AttributeSymbol> {
    parse_scope_decls(&clause.text)
        .into_iter()
        .map(|declaration| {
            let offset =
                u32::try_from(declaration.name_offset).expect("attribute name offset exceeds u32");
            let length =
                u32::try_from(declaration.name.len()).expect("attribute name length exceeds u32");
            let start = clause
                .span
                .bytes
                .start
                .checked_add(1)
                .and_then(|start| start.checked_add(offset))
                .expect("attribute name span exceeds u32");
            let end = start
                .checked_add(length)
                .expect("attribute name span exceeds u32");
            AttributeSymbol {
                name: declaration.name,
                ty: declaration.ty.unwrap_or_default(),
                span: super::frontend::SourceSpan {
                    source: clause.span.source,
                    bytes: start..end,
                },
            }
        })
        .collect()
}

struct BindingCollection {
    bindings: SemanticBindings,
    call_graph: BTreeMap<RuleId, Vec<RuleId>>,
    action_numbers: BTreeMap<ActionId, usize>,
    predicate_numbers: BTreeMap<super::model::PredicateId, usize>,
}

struct BindingCollector<'a> {
    unit: &'a GrammarUnit,
    vocabulary: &'a Vocabulary,
    rules_by_name: &'a BTreeMap<&'a str, RuleId>,
    channel_numbers: &'a BTreeMap<String, i32>,
    mode_numbers: &'a BTreeMap<String, usize>,
    diagnostics: &'a mut Vec<Diagnostic>,
    bindings: SemanticBindings,
    call_graph: BTreeMap<RuleId, Vec<RuleId>>,
    action_numbers: BTreeMap<ActionId, usize>,
    predicate_numbers: BTreeMap<super::model::PredicateId, usize>,
}

impl<'a> BindingCollector<'a> {
    fn new(
        unit: &'a GrammarUnit,
        vocabulary: &'a Vocabulary,
        rules_by_name: &'a BTreeMap<&'a str, RuleId>,
        channel_numbers: &'a BTreeMap<String, i32>,
        mode_numbers: &'a BTreeMap<String, usize>,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        Self {
            unit,
            vocabulary,
            rules_by_name,
            channel_numbers,
            mode_numbers,
            diagnostics,
            bindings: SemanticBindings::default(),
            call_graph: BTreeMap::new(),
            action_numbers: BTreeMap::new(),
            predicate_numbers: BTreeMap::new(),
        }
    }

    fn collect(mut self) -> BindingCollection {
        for rule in &self.unit.rules {
            self.collect_rule(rule);
        }
        for targets in self.call_graph.values_mut() {
            targets.sort_unstable();
            targets.dedup();
        }
        BindingCollection {
            bindings: self.bindings,
            call_graph: self.call_graph,
            action_numbers: self.action_numbers,
            predicate_numbers: self.predicate_numbers,
        }
    }

    fn collect_rule(&mut self, rule: &Rule) {
        let attributes = self.collect_attributes(rule);
        self.bindings.attributes.insert(rule.id, attributes.clone());
        self.call_graph.entry(rule.id).or_default();

        let mut label_types =
            BTreeMap::<String, (LabelKind, String, super::frontend::SourceSpan)>::new();
        self.collect_block(rule, &rule.block, &attributes, &mut label_types);
        if rule.kind == RuleKind::Lexer {
            self.collect_commands(rule);
        }
    }

    fn collect_attributes(&mut self, rule: &Rule) -> RuleAttributes {
        let attributes = RuleAttributes {
            arguments: rule
                .arguments
                .as_ref()
                .map(attribute_symbols)
                .unwrap_or_default(),
            returns: rule
                .returns
                .as_ref()
                .map(attribute_symbols)
                .unwrap_or_default(),
            locals: rule
                .locals
                .as_ref()
                .map(attribute_symbols)
                .unwrap_or_default(),
        };

        self.check_attribute_name_conflicts(&attributes.arguments, "parameter", "G4S056", rule);
        self.check_attribute_name_conflicts(&attributes.returns, "return value", "G4S057", rule);
        self.check_attribute_name_conflicts(&attributes.locals, "local", "G4S058", rule);
        self.check_attribute_overlap(
            &attributes.returns,
            &attributes.arguments,
            "return value",
            "parameter",
            "G4S059",
        );
        self.check_attribute_overlap(
            &attributes.locals,
            &attributes.arguments,
            "local",
            "parameter",
            "G4S060",
        );
        self.check_attribute_overlap(
            &attributes.locals,
            &attributes.returns,
            "local",
            "return value",
            "G4S061",
        );
        attributes
    }

    fn check_attribute_name_conflicts(
        &mut self,
        attributes: &[AttributeSymbol],
        kind: &str,
        rule_code: &'static str,
        rule: &Rule,
    ) {
        for attribute in attributes {
            if self.rules_by_name.contains_key(attribute.name.as_str()) {
                self.diagnostics.push(Diagnostic::error(
                    rule_code,
                    attribute.span.clone(),
                    format!(
                        "{kind} {} conflicts with rule with same name",
                        attribute.name
                    ),
                ));
            }
            if self.vocabulary.by_name.contains_key(&attribute.name) {
                self.diagnostics.push(Diagnostic::error(
                    "G4S037",
                    attribute.span.clone(),
                    format!(
                        "{kind} {} conflicts with token with same name in rule {}",
                        attribute.name, rule.name
                    ),
                ));
            }
        }
    }

    fn check_attribute_overlap(
        &mut self,
        attributes: &[AttributeSymbol],
        reference: &[AttributeSymbol],
        kind: &str,
        reference_kind: &str,
        code: &'static str,
    ) {
        for attribute in attributes {
            if reference
                .iter()
                .any(|candidate| candidate.name == attribute.name)
            {
                self.diagnostics.push(Diagnostic::error(
                    code,
                    attribute.span.clone(),
                    format!(
                        "{kind} {} conflicts with {reference_kind} with same name",
                        attribute.name
                    ),
                ));
            }
        }
    }

    fn collect_block(
        &mut self,
        rule: &Rule,
        block: &Block,
        attributes: &RuleAttributes,
        label_types: &mut BTreeMap<String, (LabelKind, String, super::frontend::SourceSpan)>,
    ) {
        for alternative in &block.alternatives {
            self.bindings.alternatives.insert(alternative.id, rule.id);
            for element in &alternative.elements {
                self.collect_element(rule, alternative, element, attributes, label_types);
            }
        }
    }

    fn collect_element(
        &mut self,
        rule: &Rule,
        alternative: &Alternative,
        element: &Element,
        attributes: &RuleAttributes,
        label_types: &mut BTreeMap<String, (LabelKind, String, super::frontend::SourceSpan)>,
    ) {
        if let Some(label) = &element.label {
            if self.rules_by_name.contains_key(label.name.as_str()) {
                self.diagnostics.push(Diagnostic::error(
                    "G4S038",
                    label.span.clone(),
                    format!("label {} conflicts with rule with same name", label.name),
                ));
            }
            if self.vocabulary.by_name.contains_key(&label.name) {
                self.diagnostics.push(Diagnostic::error(
                    "G4S039",
                    label.span.clone(),
                    format!("label {} conflicts with token with same name", label.name),
                ));
            }
            if attributes
                .arguments
                .iter()
                .any(|attribute| attribute.name == label.name)
            {
                self.diagnostics.push(Diagnostic::error(
                    "G4S062",
                    label.span.clone(),
                    format!(
                        "label {} conflicts with parameter with same name",
                        label.name
                    ),
                ));
            }
            if attributes
                .returns
                .iter()
                .any(|attribute| attribute.name == label.name)
            {
                self.diagnostics.push(Diagnostic::error(
                    "G4S063",
                    label.span.clone(),
                    format!(
                        "label {} conflicts with return value with same name",
                        label.name
                    ),
                ));
            }
            if attributes
                .locals
                .iter()
                .any(|attribute| attribute.name == label.name)
            {
                self.diagnostics.push(Diagnostic::error(
                    "G4S064",
                    label.span.clone(),
                    format!("label {} conflicts with local with same name", label.name),
                ));
            }
            let target = label_target(&element.kind);
            if let Some((previous_kind, previous_target, previous_span)) =
                label_types.get(&label.name)
            {
                if *previous_kind != label.kind || previous_target != &target {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "G4S041",
                            label.span.clone(),
                            format!("label {} has a conflicting type", label.name),
                        )
                        .with_related(previous_span.clone(), "first label is here"),
                    );
                }
            } else {
                label_types.insert(label.name.clone(), (label.kind, target, label.span.clone()));
            }
            self.bindings.labels.insert(
                label.id,
                LabelBinding {
                    alternative: alternative.id,
                    element: element.id,
                },
            );
        }

        match &element.kind {
            ElementKind::RuleCall(call) => {
                let Some(target) = self.rules_by_name.get(call.name.as_str()).copied() else {
                    return;
                };
                let target_rule = self
                    .unit
                    .rules
                    .iter()
                    .find(|candidate| candidate.id == target)
                    .expect("resolved rule belongs to unit");
                match (call.arguments.as_ref(), target_rule.arguments.as_ref()) {
                    (Some(_), None) => self.diagnostics.push(Diagnostic::error(
                        "G4S042",
                        element.span.clone(),
                        format!("rule {} has no defined parameters", call.name),
                    )),
                    (None, Some(_)) => self.diagnostics.push(Diagnostic::error(
                        "G4S043",
                        element.span.clone(),
                        format!("missing arguments on rule reference {}", call.name),
                    )),
                    (Some(_), Some(_)) | (None, None) => {}
                }
                self.bindings.rule_calls.insert(
                    element.id,
                    RuleCallBinding {
                        caller: rule.id,
                        target,
                        precedence: call.precedence.unwrap_or(0),
                    },
                );
                self.call_graph.entry(rule.id).or_default().push(target);
            }
            ElementKind::Terminal(terminal) => {
                if let Some(token_type) = terminal_token_type(terminal, self.vocabulary) {
                    self.bindings
                        .terminals
                        .insert(element.id, TerminalBinding { token_type });
                }
            }
            ElementKind::Set { elements, .. } => {
                self.validate_set(elements, element);
            }
            ElementKind::Block(nested) => {
                self.collect_block(rule, nested, attributes, label_types);
            }
            ElementKind::Action { id, body } => {
                let index = self.action_numbers.len();
                self.action_numbers.insert(*id, index);
                self.bindings.actions.insert(
                    *id,
                    ActionBinding {
                        rule: rule.id,
                        alternative: alternative.id,
                        element: element.id,
                        index,
                        context_dependent: action_is_context_dependent(body),
                    },
                );
                self.validate_action(rule, alternative, body, element);
            }
            ElementKind::Predicate {
                id,
                body,
                precedence,
                ..
            } => {
                let index = self.predicate_numbers.len();
                self.predicate_numbers.insert(*id, index);
                self.bindings.predicates.insert(
                    *id,
                    PredicateBinding {
                        rule: rule.id,
                        alternative: alternative.id,
                        element: element.id,
                        index,
                        precedence: *precedence,
                        context_dependent: action_is_context_dependent(body),
                    },
                );
                self.validate_action(rule, alternative, body, element);
            }
            ElementKind::Range(..) | ElementKind::Epsilon => {}
        }
    }

    fn validate_set(&mut self, elements: &[SetElement], owner: &Element) {
        if self.unit.kind != GrammarKind::Parser {
            return;
        }
        for element in elements {
            let terminal = match element {
                SetElement::Terminal { value, .. } => value,
                SetElement::Range { .. } => {
                    self.diagnostics.push(Diagnostic::error(
                        "G4S009",
                        owner.span.clone(),
                        "character ranges are not allowed in parser sets",
                    ));
                    continue;
                }
            };
            if terminal_token_type(terminal, self.vocabulary).is_none()
                && !matches!(terminal, Terminal::Wildcard)
            {
                self.diagnostics.push(Diagnostic::error(
                    "G4S044",
                    owner.span.clone(),
                    format!("set member {terminal:?} has no token type"),
                ));
            }
        }
    }

    fn validate_action(
        &mut self,
        rule: &Rule,
        alternative: &Alternative,
        body: &str,
        owner: &Element,
    ) {
        let attributes = &self.bindings.attributes[&rule.id];
        let attribute_names = attributes
            .arguments
            .iter()
            .chain(&attributes.returns)
            .chain(&attributes.locals)
            .map(|attribute| attribute.name.as_str())
            .collect::<BTreeSet<_>>();
        let labels = alternative
            .elements
            .iter()
            .filter_map(|element| element.label.as_ref().map(|label| label.name.as_str()))
            .collect::<BTreeSet<_>>();
        for reference in dollar_references(body) {
            if predefined_attribute(reference)
                || attribute_names.contains(reference)
                || labels.contains(reference)
                || self.rules_by_name.contains_key(reference)
                || self.vocabulary.by_name.contains_key(reference)
            {
                continue;
            }
            self.diagnostics.push(Diagnostic::error(
                "G4S045",
                owner.span.clone(),
                format!(
                    "unknown attribute reference ${reference} in rule {}",
                    rule.name
                ),
            ));
        }
    }

    fn collect_commands(&mut self, rule: &Rule) {
        let mut seen = Vec::<String>::new();
        for alternative in &rule.block.alternatives {
            for (index, command) in alternative.commands.iter().enumerate() {
                if command.name != "pushMode"
                    && command.name != "popMode"
                    && seen.iter().any(|previous| previous == &command.name)
                {
                    self.diagnostics.push(Diagnostic::warning(
                        "G4S046",
                        command.span.clone(),
                        format!("duplicated command {}", command.name),
                    ));
                }
                if let Some(previous) = incompatible_command(&seen, &command.name) {
                    self.diagnostics.push(Diagnostic::warning(
                        "G4S047",
                        command.span.clone(),
                        format!("incompatible commands {previous} and {}", command.name),
                    ));
                }
                if let Some(resolved) = self.resolve_command(command) {
                    self.bindings.commands.insert(
                        (alternative.id, index),
                        LexerCommandBinding {
                            rule: rule.id,
                            command: resolved,
                        },
                    );
                }
                seen.push(command.name.clone());
            }
        }
    }

    fn resolve_command(
        &mut self,
        command: &super::model::LexerCommand,
    ) -> Option<ResolvedLexerCommand> {
        let no_arg = match command.name.as_str() {
            "skip" => Some(ResolvedLexerCommand::Skip),
            "more" => Some(ResolvedLexerCommand::More),
            "popMode" => Some(ResolvedLexerCommand::PopMode),
            _ => None,
        };
        if let Some(resolved) = no_arg {
            if command.argument.is_some() {
                self.diagnostics.push(Diagnostic::error(
                    "G4S048",
                    command.span.clone(),
                    format!("command {} does not take an argument", command.name),
                ));
                return None;
            }
            return Some(resolved);
        }

        let requires_arg = matches!(
            command.name.as_str(),
            "mode" | "pushMode" | "type" | "channel"
        );
        if !requires_arg {
            self.diagnostics.push(Diagnostic::error(
                "G4S049",
                command.span.clone(),
                format!("unsupported lexer command {}", command.name),
            ));
            return None;
        }
        let Some(argument) = command.argument.as_deref() else {
            self.diagnostics.push(Diagnostic::error(
                "G4S050",
                command.span.clone(),
                format!("command {} requires an argument", command.name),
            ));
            return None;
        };

        match command.name.as_str() {
            "mode" | "pushMode" => {
                let value = self
                    .mode_numbers
                    .get(argument)
                    .copied()
                    .or_else(|| argument.parse::<usize>().ok());
                let Some(value) = value else {
                    self.diagnostics.push(Diagnostic::error(
                        "G4S051",
                        command
                            .argument_span
                            .clone()
                            .unwrap_or_else(|| command.span.clone()),
                        format!("{argument} is not a recognized mode"),
                    ));
                    return None;
                };
                if command.name == "mode" {
                    Some(ResolvedLexerCommand::Mode(value))
                } else {
                    Some(ResolvedLexerCommand::PushMode(value))
                }
            }
            "type" => {
                let value = if argument == "EOF" {
                    Some(EOF_TOKEN_TYPE)
                } else {
                    self.vocabulary
                        .by_name
                        .get(argument)
                        .copied()
                        .or_else(|| argument.parse::<i32>().ok())
                };
                value.map(ResolvedLexerCommand::Type).or_else(|| {
                    self.diagnostics.push(Diagnostic::error(
                        "G4S052",
                        command
                            .argument_span
                            .clone()
                            .unwrap_or_else(|| command.span.clone()),
                        format!("{argument} is not a recognized token"),
                    ));
                    None
                })
            }
            "channel" => {
                let value = self
                    .channel_numbers
                    .get(argument)
                    .copied()
                    .or_else(|| argument.parse::<i32>().ok());
                value.map(ResolvedLexerCommand::Channel).or_else(|| {
                    self.diagnostics.push(Diagnostic::error(
                        "G4S053",
                        command
                            .argument_span
                            .clone()
                            .unwrap_or_else(|| command.span.clone()),
                        format!("{argument} is not a recognized channel"),
                    ));
                    None
                })
            }
            _ => unreachable!("required command set checked above"),
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
    let command_alias = rest.is_empty() && matches!(alternative.commands.as_slice(), [_] | [_, _]);
    (plain_alias || command_alias)
        .then_some(literal)
        .map(String::as_str)
}

fn terminal_token_type(terminal: &Terminal, vocabulary: &Vocabulary) -> Option<i32> {
    match terminal {
        Terminal::Token(name) => vocabulary.by_name.get(name).copied(),
        Terminal::Literal(literal) => vocabulary.by_literal.get(literal).copied(),
        Terminal::Eof => Some(EOF_TOKEN_TYPE),
        Terminal::LexerCharSet(_) | Terminal::Wildcard => None,
    }
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

fn label_target(kind: &ElementKind) -> String {
    match kind {
        ElementKind::Terminal(Terminal::Token(name)) => format!("token:{name}"),
        ElementKind::Terminal(Terminal::Literal(literal)) => format!("literal:{literal}"),
        ElementKind::Terminal(Terminal::LexerCharSet(set)) => format!("charset:{set}"),
        ElementKind::Terminal(Terminal::Wildcard) => "wildcard".to_owned(),
        ElementKind::Terminal(Terminal::Eof) => "eof".to_owned(),
        ElementKind::RuleCall(call) => format!("rule:{}", call.name),
        ElementKind::Range(start, stop) => format!("range:{start}:{stop}"),
        ElementKind::Set { inverted, .. } => format!("set:{inverted}"),
        ElementKind::Block(_) => "block".to_owned(),
        ElementKind::Action { .. } => "action".to_owned(),
        ElementKind::Predicate { .. } => "predicate".to_owned(),
        ElementKind::Epsilon => "epsilon".to_owned(),
    }
}

fn incompatible_command<'a>(seen: &'a [String], command: &str) -> Option<&'a str> {
    let candidates: &[&str] = match command {
        "skip" => &["more", "type", "channel"],
        "more" => &["skip", "type", "channel"],
        "type" | "channel" => &["more", "skip"],
        _ => &[],
    };
    candidates.iter().find_map(|candidate| {
        seen.iter()
            .find(|value| value == candidate)
            .map(String::as_str)
    })
}

fn action_is_context_dependent(body: &str) -> bool {
    !dollar_references(body).is_empty()
}

fn dollar_references(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut references = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end] == b'_' || bytes[end].is_ascii_alphanumeric()) {
            end += 1;
        }
        if end > start && (bytes[start] == b'_' || bytes[start].is_ascii_alphabetic()) {
            references.push(&body[start..end]);
        }
        index = end.max(index + 1);
    }
    references
}

fn predefined_attribute(name: &str) -> bool {
    matches!(name, "parser" | "text" | "start" | "stop" | "ctx")
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
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::grammar::loader::{LoadOptions, load};
    use crate::grammar::transform::integrate_loaded;

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
        let loaded = load(LoadOptions {
            roots: vec![fixture.path(root)],
            library_directories: Vec::new(),
        })?;
        analyze(integrate_loaded(&loaded)?)
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
