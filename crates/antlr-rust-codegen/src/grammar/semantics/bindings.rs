// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use std::collections::BTreeMap;

use crate::grammar::action::{ActionReference, ActionReferenceKind, ActionReferenceParser};
use crate::grammar::diagnostic::Diagnostic;
use crate::grammar::frontend::SourceSpan;
use crate::grammar::model::{
    ActionBinding, ActionId, Alternative, AlternativeId, Block, Element, ElementKind, GrammarKind,
    GrammarUnit, Label, LabelBinding, LabelKind, LexerCommand, LexerCommandBinding,
    PredicateBinding, PredicateId, ResolvedLexerCommand, Rule, RuleCallBinding, RuleId, RuleKind,
    SemanticBindings, SetElement, Terminal, TerminalBinding, Vocabulary,
};

use super::attributes::rule_attributes;
use super::{COMMON_CONSTANTS, EOF_TOKEN_TYPE, TOKEN_ATTRIBUTES};

pub(super) struct BindingCollection {
    pub(super) bindings: SemanticBindings,
    pub(super) call_graph: BTreeMap<RuleId, Vec<RuleId>>,
    pub(super) action_numbers: BTreeMap<ActionId, usize>,
    pub(super) predicate_numbers: BTreeMap<PredicateId, usize>,
}

#[derive(Clone, Copy)]
enum ActionScope<'a> {
    Grammar,
    Rule(&'a Rule),
    Alternative {
        rule: &'a Rule,
        alternative: &'a Alternative,
        owner: &'a Alternative,
    },
}

impl<'a> ActionScope<'a> {
    const fn rule(self) -> Option<&'a Rule> {
        match self {
            Self::Grammar => None,
            Self::Rule(rule) | Self::Alternative { rule, .. } => Some(rule),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionTarget {
    Rule(RuleId),
    Token,
    Other,
}

#[derive(Clone, Copy)]
struct ResolvedLabel {
    kind: LabelKind,
    target: ActionTarget,
}

pub(super) struct BindingCollector<'a> {
    unit: &'a GrammarUnit,
    vocabulary: &'a Vocabulary,
    rules_by_name: &'a BTreeMap<&'a str, RuleId>,
    channel_numbers: &'a BTreeMap<String, i32>,
    mode_numbers: &'a BTreeMap<String, usize>,
    diagnostics: &'a mut Vec<Diagnostic>,
    bindings: SemanticBindings,
    call_graph: BTreeMap<RuleId, Vec<RuleId>>,
    action_numbers: BTreeMap<ActionId, usize>,
    predicate_numbers: BTreeMap<PredicateId, usize>,
    action_reference_parser: ActionReferenceParser,
}

pub(super) struct BindingCollectorContext<'a> {
    pub(super) unit: &'a GrammarUnit,
    pub(super) vocabulary: &'a Vocabulary,
    pub(super) rules_by_name: &'a BTreeMap<&'a str, RuleId>,
    pub(super) channel_numbers: &'a BTreeMap<String, i32>,
    pub(super) mode_numbers: &'a BTreeMap<String, usize>,
    pub(super) diagnostics: &'a mut Vec<Diagnostic>,
    pub(super) action_reference_parser: ActionReferenceParser,
}

impl<'a> BindingCollector<'a> {
    pub(super) fn new(context: BindingCollectorContext<'a>) -> Self {
        let BindingCollectorContext {
            unit,
            vocabulary,
            rules_by_name,
            channel_numbers,
            mode_numbers,
            diagnostics,
            action_reference_parser,
        } = context;
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
            action_reference_parser,
        }
    }

    pub(super) fn collect(mut self) -> BindingCollection {
        for rule in &self.unit.rules {
            self.bindings
                .attributes
                .insert(rule.id, rule_attributes(rule));
            self.call_graph.entry(rule.id).or_default();
        }
        for action in &self.unit.actions {
            self.validate_action(ActionScope::Grammar, &action.body, &action.body_span);
        }
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
        for action in &rule.actions {
            self.validate_action(ActionScope::Rule(rule), &action.body, &action.body_span);
        }
        self.collect_block(rule, &rule.block);
        for handler in &rule.catches {
            self.validate_action(ActionScope::Rule(rule), &handler.body, &handler.body_span);
        }
        if let Some(action) = &rule.finally_action {
            self.validate_action(ActionScope::Rule(rule), &action.body, &action.body_span);
        }
        if rule.kind == RuleKind::Lexer {
            self.collect_commands(rule);
        }
    }

    fn collect_block(&mut self, rule: &Rule, block: &Block) {
        for alternative in &block.alternatives {
            self.collect_alternative(rule, alternative, alternative);
        }
    }

    fn collect_alternative(&mut self, rule: &Rule, alternative: &Alternative, scope: &Alternative) {
        self.bindings.alternatives.insert(alternative.id, rule.id);
        for element in &alternative.elements {
            self.collect_element(rule, alternative, scope, element);
        }
    }

    fn collect_element(
        &mut self,
        rule: &Rule,
        alternative: &Alternative,
        scope: &Alternative,
        element: &Element,
    ) {
        if let Some(label) = &element.label {
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
                for nested_alternative in &nested.alternatives {
                    self.collect_alternative(rule, nested_alternative, scope);
                }
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
                        context_dependent: action_is_context_dependent(
                            body,
                            self.action_reference_parser,
                        ),
                    },
                );
                self.validate_action(
                    ActionScope::Alternative {
                        rule,
                        alternative: scope,
                        owner: alternative,
                    },
                    body,
                    &element.span,
                );
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
                        context_dependent: action_is_context_dependent(
                            body,
                            self.action_reference_parser,
                        ),
                    },
                );
                self.validate_action(
                    ActionScope::Alternative {
                        rule,
                        alternative: scope,
                        owner: alternative,
                    },
                    body,
                    &element.span,
                );
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

    fn validate_action(&mut self, scope: ActionScope<'_>, body: &str, body_span: &SourceSpan) {
        for reference in (self.action_reference_parser)(body) {
            let diagnostic =
                match reference.kind {
                    ActionReferenceKind::Attribute { name, assignment } => self
                        .validate_simple_reference(scope, reference, name, assignment, body_span),
                    ActionReferenceKind::Qualified { name, attribute } => self
                        .validate_qualified_reference(scope, reference, name, attribute, body_span),
                    ActionReferenceKind::NonLocal { rule, attribute } => {
                        self.validate_non_local_reference(reference, rule, attribute, body_span)
                    }
                };
            if let Some(diagnostic) = diagnostic {
                self.diagnostics.push(diagnostic);
            }
        }
    }

    fn validate_simple_reference(
        &self,
        scope: ActionScope<'_>,
        reference: ActionReference<'_>,
        name: &str,
        assignment: bool,
        body_span: &SourceSpan,
    ) -> Option<Diagnostic> {
        if self.resolves_to_simple_attribute(scope, name) {
            return None;
        }
        let name_span = action_identifier_span(body_span, reference.name_offset, name.len());
        if assignment {
            return Some(if self.resolves_to_list_label(scope, name) {
                Diagnostic::error(
                    "G4S076",
                    name_span,
                    format!("cannot assign a value to list label {name}"),
                )
            } else {
                unknown_simple_attribute(name_span, name, reference.expression)
            });
        }
        if self.resolves_to_token(scope, name) || self.resolves_to_list_label(scope, name) {
            return None;
        }
        if self.isolated_rule(scope, name).is_some() {
            return Some(Diagnostic::error(
                "G4S075",
                name_span,
                format!(
                    "missing attribute access on rule reference {name} in {}",
                    reference.expression
                ),
            ));
        }
        Some(unknown_simple_attribute(
            name_span,
            name,
            reference.expression,
        ))
    }

    fn validate_qualified_reference(
        &self,
        scope: ActionScope<'_>,
        reference: ActionReference<'_>,
        name: &str,
        attribute: &str,
        body_span: &SourceSpan,
    ) -> Option<Diagnostic> {
        if self.resolves_to_simple_attribute(scope, name) {
            return None;
        }
        let name_span = action_identifier_span(body_span, reference.name_offset, name.len());
        let attribute_span = action_identifier_span(
            body_span,
            reference
                .attribute_offset
                .expect("qualified reference has an attribute offset"),
            attribute.len(),
        );
        match self.attribute_dictionary(scope, name) {
            Some(ActionTarget::Rule(rule)) => {
                return self.validate_rule_attribute(
                    self.rule(rule).expect("action target rule belongs to unit"),
                    attribute,
                    attribute_span,
                    reference.expression,
                    false,
                );
            }
            Some(ActionTarget::Token) => {
                return (!TOKEN_ATTRIBUTES.contains(&attribute)).then(|| {
                    Diagnostic::error(
                        "G4S077",
                        attribute_span,
                        format!(
                            "attribute {attribute} isn't a valid property in {}",
                            reference.expression
                        ),
                    )
                });
            }
            Some(ActionTarget::Other) | None => {}
        }
        if let Some(rule) = self.isolated_rule(scope, name) {
            return self.validate_rule_attribute(
                self.rule(rule).expect("isolated rule belongs to unit"),
                attribute,
                attribute_span,
                reference.expression,
                false,
            );
        }
        Some(unknown_simple_attribute(
            name_span,
            name,
            reference.expression,
        ))
    }

    fn validate_non_local_reference(
        &self,
        reference: ActionReference<'_>,
        rule_name: &str,
        attribute: &str,
        body_span: &SourceSpan,
    ) -> Option<Diagnostic> {
        let Some(rule) = self.rule_named(rule_name) else {
            return Some(Diagnostic::error(
                "G4S071",
                action_identifier_span(body_span, reference.name_offset, rule_name.len()),
                format!(
                    "reference to undefined rule {rule_name} in non-local ref {}",
                    reference.expression
                ),
            ));
        };
        let attribute_span = action_identifier_span(
            body_span,
            reference
                .attribute_offset
                .expect("non-local reference has an attribute offset"),
            attribute.len(),
        );
        self.validate_rule_attribute(rule, attribute, attribute_span, reference.expression, true)
    }

    fn validate_rule_attribute(
        &self,
        rule: &Rule,
        attribute: &str,
        attribute_span: SourceSpan,
        expression: &str,
        include_parameters_and_locals: bool,
    ) -> Option<Diagnostic> {
        let attributes = &self.bindings.attributes[&rule.id];
        let is_return = attributes
            .returns
            .iter()
            .any(|candidate| candidate.name == attribute);
        let is_parameter = attributes
            .arguments
            .iter()
            .any(|candidate| candidate.name == attribute);
        let is_local = attributes
            .locals
            .iter()
            .any(|candidate| candidate.name == attribute);
        if is_return
            || predefined_attribute(attribute)
            || (include_parameters_and_locals && (is_parameter || is_local))
        {
            return None;
        }
        if is_parameter {
            return Some(Diagnostic::error(
                "G4S073",
                attribute_span,
                format!(
                    "parameter {attribute} of rule {} is not accessible in this scope: {expression}",
                    rule.name
                ),
            ));
        }
        Some(Diagnostic::error(
            "G4S074",
            attribute_span,
            format!(
                "unknown attribute {attribute} for rule {} in {expression}",
                rule.name
            ),
        ))
    }

    fn resolves_to_simple_attribute(&self, scope: ActionScope<'_>, name: &str) -> bool {
        let Some(rule) = scope.rule() else {
            return false;
        };
        let attributes = &self.bindings.attributes[&rule.id];
        predefined_attribute(name)
            || attributes
                .arguments
                .iter()
                .chain(&attributes.returns)
                .chain(&attributes.locals)
                .any(|attribute| attribute.name == name)
    }

    fn resolves_to_list_label(&self, scope: ActionScope<'_>, name: &str) -> bool {
        self.label_target(scope, name)
            .is_some_and(|label| label.kind == LabelKind::List)
    }

    fn resolves_to_token(&self, scope: ActionScope<'_>, name: &str) -> bool {
        let labeled_token = self.label_target(scope, name).is_some_and(|label| {
            label.kind == LabelKind::Single && label.target == ActionTarget::Token
        });
        labeled_token
            || matches!(
                scope,
                ActionScope::Alternative { alternative, .. }
                    if alternative_has_token_reference(alternative, name)
            )
    }

    fn isolated_rule(&self, scope: ActionScope<'_>, name: &str) -> Option<RuleId> {
        let rule = scope.rule()?;
        if rule.name == name {
            return Some(rule.id);
        }
        if let Some(label) = self.label_target(scope, name)
            && label.kind == LabelKind::Single
            && let ActionTarget::Rule(target) = label.target
        {
            return Some(target);
        }
        match scope {
            ActionScope::Alternative { alternative, .. } => {
                alternative_rule_reference(alternative, name)
                    .and_then(|name| self.rule_named(name))
                    .map(|rule| rule.id)
            }
            ActionScope::Grammar | ActionScope::Rule(_) => None,
        }
    }

    fn attribute_dictionary(&self, scope: ActionScope<'_>, name: &str) -> Option<ActionTarget> {
        if let Some(label) = self.label_target(scope, name)
            && label.kind == LabelKind::Single
        {
            return Some(label.target);
        }
        let ActionScope::Alternative { alternative, .. } = scope else {
            return None;
        };
        if let Some(rule_name) = alternative_rule_reference(alternative, name) {
            return self
                .rule_named(rule_name)
                .map(|rule| ActionTarget::Rule(rule.id));
        }
        alternative_has_token_reference(alternative, name).then_some(ActionTarget::Token)
    }

    fn label_target(&self, scope: ActionScope<'_>, name: &str) -> Option<ResolvedLabel> {
        if let ActionScope::Alternative { rule, owner, .. } = scope
            && let Some(target) = self.removed_left_recursive_label_target(rule, Some(owner), name)
        {
            return Some(target);
        }
        let resolved = match scope {
            ActionScope::Grammar => None,
            ActionScope::Rule(rule) => find_label_in_block(&rule.block, name),
            ActionScope::Alternative { alternative, .. } => {
                find_label_in_alternative(alternative, name)
            }
        };
        resolved.map_or_else(
            || {
                scope
                    .rule()
                    .and_then(|rule| self.removed_left_recursive_label_target(rule, None, name))
            },
            |(label, element)| {
                let target = match &element.kind {
                    ElementKind::RuleCall(call) => self
                        .rules_by_name
                        .get(call.name.as_str())
                        .copied()
                        .map_or(ActionTarget::Other, ActionTarget::Rule),
                    ElementKind::Terminal(_) | ElementKind::Range(..) | ElementKind::Set { .. } => {
                        ActionTarget::Token
                    }
                    ElementKind::Block(_)
                    | ElementKind::Action { .. }
                    | ElementKind::Predicate { .. }
                    | ElementKind::Epsilon => ActionTarget::Other,
                };
                Some(ResolvedLabel {
                    kind: label.kind,
                    target,
                })
            },
        )
    }

    fn removed_left_recursive_label_target(
        &self,
        rule: &Rule,
        owner: Option<&Alternative>,
        name: &str,
    ) -> Option<ResolvedLabel> {
        let left_recursion = rule.left_recursion.as_ref()?;
        let removed = left_recursion.deleted_labels.values().find(|removed| {
            removed.label.name == name
                && owner.is_none_or(|owner| {
                    left_recursion
                        .original_to_rewritten
                        .get(&removed.original_alternative)
                        .is_some_and(|rewritten| {
                            alternative_contains(&rule.block, *rewritten, owner.id)
                        })
                })
        })?;
        let target = self
            .rules_by_name
            .get(removed.target.as_str())
            .copied()
            .map_or(ActionTarget::Other, ActionTarget::Rule);
        Some(ResolvedLabel {
            kind: removed.label.kind,
            target,
        })
    }

    fn rule_named(&self, name: &str) -> Option<&Rule> {
        self.rules_by_name
            .get(name)
            .copied()
            .and_then(|id| self.rule(id))
    }

    fn rule(&self, id: RuleId) -> Option<&Rule> {
        self.unit.rules.iter().find(|rule| rule.id == id)
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

    fn resolve_command(&mut self, command: &LexerCommand) -> Option<ResolvedLexerCommand> {
        let Some(command_name) = canonical_lexer_command_name(&command.name) else {
            self.diagnostics.push(Diagnostic::error(
                "G4S049",
                command.span.clone(),
                format!("unsupported lexer command {}", command.name),
            ));
            return None;
        };
        let no_arg = match command_name {
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

        let Some(argument) = command.argument.as_deref() else {
            self.diagnostics.push(Diagnostic::error(
                "G4S050",
                command.span.clone(),
                format!("command {} requires an argument", command.name),
            ));
            return None;
        };

        match command_name {
            "mode" | "pushMode" => {
                if argument != "DEFAULT_MODE" && COMMON_CONSTANTS.contains(&argument) {
                    self.diagnostics.push(Diagnostic::error(
                        "G4S024",
                        command
                            .argument_span
                            .clone()
                            .unwrap_or_else(|| command.span.clone()),
                        format!("mode {argument} uses a reserved name"),
                    ));
                    return None;
                }
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
                if command_name == "mode" {
                    Some(ResolvedLexerCommand::Mode(value))
                } else {
                    Some(ResolvedLexerCommand::PushMode(value))
                }
            }
            "type" => {
                let value = if argument == "EOF" {
                    Some(EOF_TOKEN_TYPE)
                } else if COMMON_CONSTANTS.contains(&argument) {
                    self.diagnostics.push(Diagnostic::error(
                        "G4S018",
                        command
                            .argument_span
                            .clone()
                            .unwrap_or_else(|| command.span.clone()),
                        format!("token {argument} uses a reserved name"),
                    ));
                    return None;
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
                if !matches!(argument, "HIDDEN" | "DEFAULT_TOKEN_CHANNEL")
                    && COMMON_CONSTANTS.contains(&argument)
                {
                    self.diagnostics.push(Diagnostic::error(
                        "G4S021",
                        command
                            .argument_span
                            .clone()
                            .unwrap_or_else(|| command.span.clone()),
                        format!("channel {argument} uses a reserved name"),
                    ));
                    return None;
                }
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

fn canonical_lexer_command_name(name: &str) -> Option<&'static str> {
    // ANTLR target templates expose these exact initial-cap aliases.
    match name {
        "skip" | "Skip" => Some("skip"),
        "more" | "More" => Some("more"),
        "popMode" | "PopMode" => Some("popMode"),
        "mode" | "Mode" => Some("mode"),
        "pushMode" | "PushMode" => Some("pushMode"),
        "type" | "Type" => Some("type"),
        "channel" | "Channel" => Some("channel"),
        _ => None,
    }
}

fn unknown_simple_attribute(span: SourceSpan, name: &str, expression: &str) -> Diagnostic {
    Diagnostic::error(
        "G4S072",
        span,
        format!("unknown attribute reference {name} in {expression}"),
    )
}

fn action_identifier_span(body_span: &SourceSpan, offset: usize, length: usize) -> SourceSpan {
    let offset = u32::try_from(offset).expect("action reference offset exceeds u32");
    let length = u32::try_from(length).expect("action reference length exceeds u32");
    let start = body_span
        .bytes
        .start
        .checked_add(1)
        .and_then(|start| start.checked_add(offset))
        .expect("action reference span exceeds u32");
    let end = start
        .checked_add(length)
        .expect("action reference span exceeds u32");
    SourceSpan {
        source: body_span.source,
        bytes: start..end,
    }
}

fn find_label_in_block<'a>(block: &'a Block, name: &str) -> Option<(&'a Label, &'a Element)> {
    block
        .alternatives
        .iter()
        .find_map(|alternative| find_label_in_alternative(alternative, name))
}

fn find_label_in_alternative<'a>(
    alternative: &'a Alternative,
    name: &str,
) -> Option<(&'a Label, &'a Element)> {
    for element in &alternative.elements {
        if let Some(label) = &element.label
            && label.name == name
        {
            return Some((label, element));
        }
        if let ElementKind::Block(block) = &element.kind
            && let Some(found) = find_label_in_block(block, name)
        {
            return Some(found);
        }
    }
    None
}

fn alternative_contains(block: &Block, root: AlternativeId, candidate: AlternativeId) -> bool {
    for alternative in &block.alternatives {
        if alternative.id == root {
            return alternative_tree_contains(alternative, candidate);
        }
        for element in &alternative.elements {
            if let ElementKind::Block(nested) = &element.kind
                && alternative_contains(nested, root, candidate)
            {
                return true;
            }
        }
    }
    false
}

fn alternative_tree_contains(alternative: &Alternative, candidate: AlternativeId) -> bool {
    alternative.id == candidate
        || alternative.elements.iter().any(|element| {
            let ElementKind::Block(block) = &element.kind else {
                return false;
            };
            block
                .alternatives
                .iter()
                .any(|nested| alternative_tree_contains(nested, candidate))
        })
}

fn alternative_rule_reference<'a>(alternative: &'a Alternative, name: &str) -> Option<&'a str> {
    for element in &alternative.elements {
        match &element.kind {
            ElementKind::RuleCall(call) if call.name == name => {
                return Some(call.name.as_str());
            }
            ElementKind::Block(block) => {
                if let Some(found) = block
                    .alternatives
                    .iter()
                    .find_map(|nested| alternative_rule_reference(nested, name))
                {
                    return Some(found);
                }
            }
            ElementKind::Terminal(_)
            | ElementKind::RuleCall(_)
            | ElementKind::Range(..)
            | ElementKind::Set { .. }
            | ElementKind::Action { .. }
            | ElementKind::Predicate { .. }
            | ElementKind::Epsilon => {}
        }
    }
    None
}

fn alternative_has_token_reference(alternative: &Alternative, name: &str) -> bool {
    alternative
        .elements
        .iter()
        .any(|element| match &element.kind {
            ElementKind::Terminal(Terminal::Token(token)) => token == name,
            ElementKind::Set { elements, .. } => elements.iter().any(|member| {
                matches!(
                    member,
                    SetElement::Terminal {
                        value: Terminal::Token(token),
                        ..
                    } if token == name
                )
            }),
            ElementKind::Block(block) => block
                .alternatives
                .iter()
                .any(|nested| alternative_has_token_reference(nested, name)),
            ElementKind::Terminal(_)
            | ElementKind::RuleCall(_)
            | ElementKind::Range(..)
            | ElementKind::Action { .. }
            | ElementKind::Predicate { .. }
            | ElementKind::Epsilon => false,
        })
}

fn terminal_token_type(terminal: &Terminal, vocabulary: &Vocabulary) -> Option<i32> {
    match terminal {
        Terminal::Token(name) => vocabulary.by_name.get(name).copied(),
        Terminal::Literal(literal) => vocabulary.by_literal.get(literal).copied(),
        Terminal::Eof => Some(EOF_TOKEN_TYPE),
        Terminal::LexerCharSet(_) | Terminal::Wildcard => None,
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

fn action_is_context_dependent(body: &str, action_reference_parser: ActionReferenceParser) -> bool {
    !action_reference_parser(body).is_empty()
}

fn predefined_attribute(name: &str) -> bool {
    matches!(name, "parser" | "text" | "start" | "stop" | "ctx")
}
