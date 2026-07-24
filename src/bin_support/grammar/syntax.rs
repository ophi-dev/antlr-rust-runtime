use super::char_support::get_string_from_grammar_string_literal;
use super::frontend::{Cst, SourceFile, SourceSpan, SyntaxId, SyntaxNodeKind, SyntaxToken};
use super::generated::antlr_v4_parser as p;
use super::model::{
    Alternative, AttributeClause, Authored, Block, ChannelDeclaration, Element, ElementId,
    ElementKind, ExceptionHandler, GrammarHeader, GrammarId, GrammarKind, GrammarPrequel,
    GrammarUnit, ImportDecl, Label, LabelKind, LexerCommand, Mode, ModelIdAllocator, ModelNodeId,
    NamedAction, OptionDecl, ParsedGrammarUnit, Quantifier, Rule, RuleCall, RuleKind, SetElement,
    Terminal, TokenDeclaration,
};
use super::provenance::{Origin, ProvenanceIndex};

#[derive(Clone, Copy)]
pub(crate) struct SyntaxNodeRef<'a> {
    file: &'a SourceFile,
    id: SyntaxId,
}

impl<'a> SyntaxNodeRef<'a> {
    pub(crate) const fn new(file: &'a SourceFile, id: SyntaxId) -> Self {
        Self { file, id }
    }

    pub(crate) const fn id(self) -> SyntaxId {
        self.id
    }

    pub(crate) fn span(self) -> SourceSpan {
        self.node().span.clone()
    }

    pub(crate) fn kind(self) -> SyntaxNodeKind {
        self.node().kind
    }

    pub(crate) fn rule_index(self) -> Option<usize> {
        match self.kind() {
            SyntaxNodeKind::Rule { rule_index } => Some(rule_index),
            SyntaxNodeKind::Terminal { .. } | SyntaxNodeKind::Error { .. } => None,
        }
    }

    pub(crate) fn token(self) -> Option<&'a SyntaxToken> {
        match self.kind() {
            SyntaxNodeKind::Terminal { token_index } | SyntaxNodeKind::Error { token_index } => {
                self.file.tokens().get(token_index)
            }
            SyntaxNodeKind::Rule { .. } => None,
        }
    }

    pub(crate) fn text(self) -> &'a str {
        self.token().map_or_else(
            || {
                let span = self.span().bytes;
                &self.file.text()[span.start as usize..span.end as usize]
            },
            |token| self.file.token_text(token),
        )
    }

    pub(crate) fn children(self) -> impl Iterator<Item = Self> + 'a {
        self.file
            .cst()
            .children(self.id)
            .map(|id| Self::new(self.file, id))
    }

    pub(crate) fn descendants(self) -> impl Iterator<Item = Self> + 'a {
        self.file
            .cst()
            .descendants(self.id)
            .map(|id| Self::new(self.file, id))
    }

    pub(crate) fn child_rule(self, rule_index: usize) -> Option<Self> {
        self.children()
            .find(|child| child.rule_index() == Some(rule_index))
    }

    pub(crate) fn child_rules(self, rule_index: usize) -> impl Iterator<Item = Self> + 'a {
        self.children()
            .filter(move |child| child.rule_index() == Some(rule_index))
    }

    pub(crate) fn child_terminal(self, token_type: i32) -> Option<Self> {
        self.children().find(|child| {
            child
                .token()
                .is_some_and(|token| token.token_type == token_type)
        })
    }

    pub(crate) fn child_terminals(self, token_type: i32) -> impl Iterator<Item = Self> + 'a {
        self.children().filter(move |child| {
            child
                .token()
                .is_some_and(|token| token.token_type == token_type)
        })
    }

    pub(crate) fn has_terminal(self, token_type: i32) -> bool {
        self.child_terminal(token_type).is_some()
    }

    pub(crate) fn first_terminal(self) -> Option<Self> {
        self.descendants()
            .find(|node| matches!(node.kind(), SyntaxNodeKind::Terminal { .. }))
    }

    fn node(self) -> &'a super::frontend::SyntaxNode {
        self.file
            .cst()
            .node(self.id)
            .expect("syntax node ID belongs to source CST")
    }
}

pub(crate) const fn root(file: &SourceFile) -> SyntaxNodeRef<'_> {
    SyntaxNodeRef::new(file, file.cst().root_id())
}

pub(crate) fn parse_loader_unit(file: &SourceFile) -> ParsedGrammarUnit {
    let root = root(file);
    let declaration = root
        .child_rule(p::RULE_GRAMMAR_DECL)
        .expect("successful grammar parse has a declaration");
    let grammar_type = declaration
        .child_rule(p::RULE_GRAMMAR_TYPE)
        .expect("grammar declaration has a type");
    let kind = grammar_kind(grammar_type);
    let name_node = declaration
        .child_rule(p::RULE_IDENTIFIER)
        .and_then(SyntaxNodeRef::first_terminal)
        .expect("grammar declaration has a name");
    let header = GrammarHeader {
        name: authored_text(name_node),
        kind,
        declaration_span: declaration.span(),
    };
    let options = root
        .child_rules(p::RULE_PREQUEL_CONSTRUCT)
        .filter_map(|prequel| prequel.child_rule(p::RULE_OPTIONS_SPEC))
        .flat_map(|options| options.child_rules(p::RULE_OPTION))
        .filter_map(parse_option)
        .collect::<Vec<_>>();
    let token_vocab = options
        .iter()
        .find(|option| option.name.value == "tokenVocab")
        .map(|option| option.value.clone());
    ParsedGrammarUnit {
        source: file.id(),
        header,
        imports: parse_imports(file),
        options,
        token_vocab,
    }
}

fn grammar_kind(node: SyntaxNodeRef<'_>) -> GrammarKind {
    let terminals = node
        .descendants()
        .filter_map(SyntaxNodeRef::token)
        .map(|token| token.token_type)
        .collect::<Vec<_>>();
    if terminals.contains(&p::LEXER) {
        GrammarKind::Lexer
    } else if terminals.contains(&p::PARSER) {
        GrammarKind::Parser
    } else {
        GrammarKind::Combined
    }
}

fn parse_option(node: SyntaxNodeRef<'_>) -> Option<OptionDecl> {
    let identifiers = node
        .child_rules(p::RULE_IDENTIFIER)
        .filter_map(SyntaxNodeRef::first_terminal)
        .collect::<Vec<_>>();
    let name = identifiers.first().copied()?;
    let value = node
        .children()
        .find(|child| child.rule_index() == Some(p::RULE_OPTION_VALUE))?;
    Some(OptionDecl {
        name: authored_text(name),
        value: authored_node(value),
    })
}

fn parse_imports(file: &SourceFile) -> Vec<ImportDecl> {
    root(file)
        .descendants()
        .find(|node| node.rule_index() == Some(p::RULE_DELEGATE_GRAMMARS))
        .into_iter()
        .flat_map(|imports| imports.child_rules(p::RULE_DELEGATE_GRAMMAR))
        .filter_map(|declaration| {
            let names = declaration
                .child_rules(p::RULE_IDENTIFIER)
                .filter_map(SyntaxNodeRef::first_terminal)
                .collect::<Vec<_>>();
            match names.as_slice() {
                [grammar] => Some(ImportDecl {
                    alias: None,
                    grammar: authored_text(*grammar),
                }),
                [alias, grammar]
                    if declaration
                        .descendants()
                        .filter_map(SyntaxNodeRef::token)
                        .any(|token| token.token_type == p::ASSIGN) =>
                {
                    Some(ImportDecl {
                        alias: Some(authored_text(*alias)),
                        grammar: authored_text(*grammar),
                    })
                }
                _ => None,
            }
        })
        .collect()
}

fn authored_text(node: SyntaxNodeRef<'_>) -> Authored<String> {
    Authored {
        value: node.text().to_owned(),
        syntax: node.id(),
        span: node.span(),
    }
}

fn authored_node(node: SyntaxNodeRef<'_>) -> Authored<String> {
    Authored {
        value: node.text().to_owned(),
        syntax: node.id(),
        span: node.span(),
    }
}

pub(crate) fn parse_grammar_unit(
    file: &SourceFile,
    id: GrammarId,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> GrammarUnit {
    ModelBuilder {
        file,
        ids,
        provenance,
    }
    .grammar(id)
}

struct ModelBuilder<'a> {
    file: &'a SourceFile,
    ids: &'a mut ModelIdAllocator,
    provenance: &'a mut ProvenanceIndex,
}

impl ModelBuilder<'_> {
    fn grammar(&mut self, id: GrammarId) -> GrammarUnit {
        let root = root(self.file);
        let loader = parse_loader_unit(self.file);
        let prequels = root
            .child_rules(p::RULE_PREQUEL_CONSTRUCT)
            .collect::<Vec<_>>();
        let mut grammar_prequels = Vec::new();
        let mut options = Vec::new();
        let mut tokens = Vec::new();
        let mut channels = Vec::new();
        let mut actions = Vec::new();
        for prequel in prequels {
            if let Some(spec) = prequel.child_rule(p::RULE_OPTIONS_SPEC) {
                let start = options.len();
                options.extend(spec.child_rules(p::RULE_OPTION).filter_map(parse_option));
                grammar_prequels.push(GrammarPrequel::Options {
                    declarations: start..options.len(),
                    span: spec.span(),
                });
            }
            if let Some(imports) = prequel.child_rule(p::RULE_DELEGATE_GRAMMARS) {
                grammar_prequels.push(GrammarPrequel::Imports {
                    span: imports.span(),
                });
            }
            if let Some(spec) = prequel.child_rule(p::RULE_TOKENS_SPEC) {
                let start = tokens.len();
                let names = spec
                    .child_rule(p::RULE_ID_LIST)
                    .into_iter()
                    .flat_map(|list| list.child_rules(p::RULE_IDENTIFIER))
                    .filter_map(SyntaxNodeRef::first_terminal);
                for node in names {
                    let declaration = TokenDeclaration {
                        id: self.ids.token(),
                        name: authored_text(node),
                    };
                    self.authored(ModelNodeId::Token(declaration.id), node);
                    tokens.push(declaration);
                }
                grammar_prequels.push(GrammarPrequel::Tokens {
                    declarations: start..tokens.len(),
                    span: spec.span(),
                });
            }
            if let Some(spec) = prequel.child_rule(p::RULE_CHANNELS_SPEC) {
                let start = channels.len();
                let names = spec
                    .child_rule(p::RULE_ID_LIST)
                    .into_iter()
                    .flat_map(|list| list.child_rules(p::RULE_IDENTIFIER))
                    .filter_map(SyntaxNodeRef::first_terminal);
                for node in names {
                    let declaration = ChannelDeclaration {
                        id: self.ids.channel(),
                        name: authored_text(node),
                    };
                    self.authored(ModelNodeId::Channel(declaration.id), node);
                    channels.push(declaration);
                }
                grammar_prequels.push(GrammarPrequel::Channels {
                    declarations: start..channels.len(),
                    span: spec.span(),
                });
            }
            if let Some(action) = prequel.child_rule(p::RULE_ACTION) {
                if let Some(action) = self.named_action(action, None) {
                    actions.push(action);
                }
            }
        }

        let mut rules = Vec::new();
        if let Some(rule_list) = root.child_rule(p::RULE_RULES) {
            for rule_spec in rule_list.child_rules(p::RULE_RULE_SPEC) {
                if let Some(parser_rule) = rule_spec.child_rule(p::RULE_PARSER_RULE_SPEC) {
                    rules.push(self.parser_rule(parser_rule));
                } else if let Some(lexer_rule) = rule_spec.child_rule(p::RULE_LEXER_RULE_SPEC) {
                    rules.push(self.lexer_rule(lexer_rule, None));
                }
            }
        }

        let mut modes = Vec::new();
        for mode_node in root.child_rules(p::RULE_MODE_SPEC) {
            let mode_id = self.ids.mode();
            let name_node = mode_node
                .child_rule(p::RULE_IDENTIFIER)
                .and_then(SyntaxNodeRef::first_terminal);
            let name = name_node.map_or_else(String::new, |node| node.text().to_owned());
            let name_span = name_node.map_or_else(|| mode_node.span(), SyntaxNodeRef::span);
            let mut mode_rules = Vec::new();
            for rule_node in mode_node.child_rules(p::RULE_LEXER_RULE_SPEC) {
                let rule = self.lexer_rule(rule_node, Some(mode_id));
                mode_rules.push(rule.id);
                rules.push(rule);
            }
            self.authored(ModelNodeId::Mode(mode_id), mode_node);
            modes.push(Mode {
                id: mode_id,
                name,
                name_span,
                rules: mode_rules,
                syntax: mode_node.id(),
                span: mode_node.span(),
            });
        }

        self.authored(ModelNodeId::Grammar(id), root);
        GrammarUnit {
            id,
            source: self.file.id(),
            name: loader.header.name.value,
            kind: loader.header.kind,
            prequels: grammar_prequels,
            options,
            tokens,
            channels,
            actions,
            modes,
            rules,
            syntax: root.id(),
            span: root.span(),
        }
    }

    fn parser_rule(&mut self, node: SyntaxNodeRef<'_>) -> Rule {
        let id = self.ids.rule();
        let name_node = node.child_terminal(p::RULE_REF);
        let name = name_node.map_or_else(String::new, |name| name.text().to_owned());
        let name_span = name_node.map_or_else(|| node.span(), SyntaxNodeRef::span);
        let modifiers = node
            .child_rule(p::RULE_RULE_MODIFIERS)
            .into_iter()
            .flat_map(|modifiers| modifiers.child_rules(p::RULE_RULE_MODIFIER))
            .filter_map(SyntaxNodeRef::first_terminal)
            .map(authored_text)
            .collect::<Vec<_>>();
        let arguments = node
            .child_rule(p::RULE_ARG_ACTION_BLOCK)
            .map(attribute_clause);
        let returns = node
            .child_rule(p::RULE_RULE_RETURNS)
            .and_then(|returns| returns.child_rule(p::RULE_ARG_ACTION_BLOCK))
            .map(attribute_clause);
        let locals = node
            .child_rule(p::RULE_LOCALS_SPEC)
            .and_then(|locals| locals.child_rule(p::RULE_ARG_ACTION_BLOCK))
            .map(attribute_clause);
        let throws = node
            .child_rule(p::RULE_THROWS_SPEC)
            .into_iter()
            .flat_map(|throws| throws.child_rules(p::RULE_QUALIFIED_IDENTIFIER))
            .map(authored_node)
            .collect();
        let mut options = Vec::new();
        let mut actions = Vec::new();
        for prequel in node.child_rules(p::RULE_RULE_PREQUEL) {
            if let Some(spec) = prequel.child_rule(p::RULE_OPTIONS_SPEC) {
                options.extend(spec.child_rules(p::RULE_OPTION).filter_map(parse_option));
            }
            if let Some(action) = prequel.child_rule(p::RULE_RULE_ACTION) {
                if let Some(action) = self.named_action(action, None) {
                    actions.push(action);
                }
            }
        }
        let block_node = node
            .child_rule(p::RULE_RULE_BLOCK)
            .expect("parser rule has a block");
        let block = self.parser_rule_block(block_node);
        let exception_group = node.child_rule(p::RULE_EXCEPTION_GROUP);
        let catches = exception_group
            .into_iter()
            .flat_map(|group| group.child_rules(p::RULE_EXCEPTION_HANDLER))
            .filter_map(parse_exception_handler)
            .collect();
        let finally_action = exception_group
            .and_then(|group| group.child_rule(p::RULE_FINALLY_CLAUSE))
            .and_then(|finally| {
                finally.child_rule(p::RULE_ACTION_BLOCK).map(|action| {
                    self.named_action_from_parts(finally, None, "finally".to_owned(), action)
                })
            });
        self.authored(ModelNodeId::Rule(id), node);
        Rule {
            id,
            name,
            name_span,
            kind: RuleKind::Parser,
            fragment: false,
            modifiers,
            arguments,
            returns,
            locals,
            throws,
            options,
            actions,
            catches,
            finally_action,
            left_recursion: None,
            block,
            mode: None,
            case_insensitive: None,
            syntax: node.id(),
            span: node.span(),
        }
    }

    fn lexer_rule(&mut self, node: SyntaxNodeRef<'_>, mode: Option<super::model::ModeId>) -> Rule {
        let id = self.ids.rule();
        let name_node = node.child_terminal(p::TOKEN_REF);
        let name = name_node.map_or_else(String::new, |name| name.text().to_owned());
        let name_span = name_node.map_or_else(|| node.span(), SyntaxNodeRef::span);
        let fragment = node.has_terminal(p::FRAGMENT);
        let options = node
            .child_rule(p::RULE_OPTIONS_SPEC)
            .into_iter()
            .flat_map(|options| options.child_rules(p::RULE_OPTION))
            .filter_map(parse_option)
            .collect::<Vec<_>>();
        let case_insensitive = options
            .iter()
            .find(|option| option.name.value == "caseInsensitive")
            .and_then(|option| match option.value.value.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            });
        let block_node = node
            .child_rule(p::RULE_LEXER_RULE_BLOCK)
            .expect("lexer rule has a block");
        let block = self.lexer_rule_block(block_node);
        self.authored(ModelNodeId::Rule(id), node);
        Rule {
            id,
            name,
            name_span,
            kind: RuleKind::Lexer,
            fragment,
            modifiers: Vec::new(),
            arguments: None,
            returns: None,
            locals: None,
            throws: Vec::new(),
            options,
            actions: Vec::new(),
            catches: Vec::new(),
            finally_action: None,
            left_recursion: None,
            block,
            mode,
            case_insensitive,
            syntax: node.id(),
            span: node.span(),
        }
    }

    fn parser_rule_block(&mut self, node: SyntaxNodeRef<'_>) -> Block {
        let alternatives = node
            .child_rule(p::RULE_RULE_ALT_LIST)
            .into_iter()
            .flat_map(|list| list.child_rules(p::RULE_LABELED_ALT))
            .map(|alternative| self.parser_alternative(alternative, true))
            .collect();
        Block {
            alternatives,
            options: Vec::new(),
            syntax: node.id(),
            span: node.span(),
        }
    }

    fn parser_block(&mut self, node: SyntaxNodeRef<'_>) -> Block {
        let options = node
            .child_rule(p::RULE_OPTIONS_SPEC)
            .into_iter()
            .flat_map(|spec| spec.child_rules(p::RULE_OPTION))
            .filter_map(parse_option)
            .collect();
        let alternatives = node
            .child_rule(p::RULE_ALT_LIST)
            .into_iter()
            .flat_map(|list| list.child_rules(p::RULE_ALTERNATIVE))
            .map(|alternative| self.parser_alternative(alternative, false))
            .collect();
        Block {
            alternatives,
            options,
            syntax: node.id(),
            span: node.span(),
        }
    }

    fn lexer_rule_block(&mut self, node: SyntaxNodeRef<'_>) -> Block {
        let alternatives = node
            .child_rule(p::RULE_LEXER_ALT_LIST)
            .into_iter()
            .flat_map(|list| list.child_rules(p::RULE_LEXER_ALT))
            .map(|alternative| self.lexer_alternative(alternative))
            .collect();
        Block {
            alternatives,
            options: Vec::new(),
            syntax: node.id(),
            span: node.span(),
        }
    }

    fn lexer_block(&mut self, node: SyntaxNodeRef<'_>) -> Block {
        let alternatives = node
            .child_rule(p::RULE_LEXER_ALT_LIST)
            .into_iter()
            .flat_map(|list| list.child_rules(p::RULE_LEXER_ALT))
            .map(|alternative| self.lexer_alternative(alternative))
            .collect();
        Block {
            alternatives,
            options: Vec::new(),
            syntax: node.id(),
            span: node.span(),
        }
    }

    fn parser_alternative(&mut self, node: SyntaxNodeRef<'_>, labeled: bool) -> Alternative {
        let syntax = node;
        let alternative = if labeled {
            node.child_rule(p::RULE_ALTERNATIVE)
                .expect("labeled alternative has an alternative")
        } else {
            node
        };
        let id = self.ids.alternative();
        let options = alternative
            .child_rule(p::RULE_ELEMENT_OPTIONS)
            .map(parse_element_options)
            .unwrap_or_default();
        let elements = alternative
            .child_rules(p::RULE_ELEMENT)
            .map(|element| self.parser_element(element))
            .collect();
        let label = labeled
            .then(|| {
                node.child_rule(p::RULE_IDENTIFIER)
                    .and_then(SyntaxNodeRef::first_terminal)
                    .map(authored_text)
            })
            .flatten();
        self.authored(ModelNodeId::Alternative(id), syntax);
        Alternative {
            id,
            elements,
            label,
            options,
            commands: Vec::new(),
            syntax: syntax.id(),
            span: syntax.span(),
        }
    }

    fn lexer_alternative(&mut self, node: SyntaxNodeRef<'_>) -> Alternative {
        let id = self.ids.alternative();
        let elements = node
            .child_rule(p::RULE_LEXER_ELEMENTS)
            .into_iter()
            .flat_map(|elements| elements.child_rules(p::RULE_LEXER_ELEMENT))
            .map(|element| self.lexer_element(element))
            .collect();
        let commands = node
            .child_rule(p::RULE_LEXER_COMMANDS)
            .into_iter()
            .flat_map(|commands| commands.child_rules(p::RULE_LEXER_COMMAND))
            .filter_map(parse_lexer_command)
            .collect();
        self.authored(ModelNodeId::Alternative(id), node);
        Alternative {
            id,
            elements,
            label: None,
            options: Vec::new(),
            commands,
            syntax: node.id(),
            span: node.span(),
        }
    }

    fn parser_element(&mut self, node: SyntaxNodeRef<'_>) -> Element {
        let id = self.ids.element();
        let mut label = None;
        let (kind, quantifier, span) =
            if let Some(labeled) = node.child_rule(p::RULE_LABELED_ELEMENT) {
                label = self.element_label(labeled);
                let (kind, span) = labeled.child_rule(p::RULE_ATOM).map_or_else(
                    || {
                        let block = labeled
                            .child_rule(p::RULE_BLOCK)
                            .expect("labeled element has an atom or block");
                        (ElementKind::Block(self.parser_block(block)), block.span())
                    },
                    |atom| (Self::parser_atom(atom, id), atom.span()),
                );
                (
                    kind,
                    suffix_quantifier(node.child_rule(p::RULE_EBNF_SUFFIX)),
                    span,
                )
            } else if let Some(atom) = node.child_rule(p::RULE_ATOM) {
                (
                    Self::parser_atom(atom, id),
                    suffix_quantifier(node.child_rule(p::RULE_EBNF_SUFFIX)),
                    atom.span(),
                )
            } else if let Some(ebnf) = node.child_rule(p::RULE_EBNF) {
                let block = ebnf
                    .child_rule(p::RULE_BLOCK)
                    .expect("EBNF element has a block");
                let suffix = ebnf
                    .child_rule(p::RULE_BLOCK_SUFFIX)
                    .and_then(|suffix| suffix.child_rule(p::RULE_EBNF_SUFFIX));
                (
                    ElementKind::Block(self.parser_block(block)),
                    suffix_quantifier(suffix),
                    block.span(),
                )
            } else {
                let action = node
                    .child_rule(p::RULE_ACTION_BLOCK)
                    .expect("parser element has a recognized shape");
                (
                    self.action_or_predicate(node, action),
                    Quantifier::One,
                    action.span(),
                )
            };
        let option_owner = node
            .child_rule(p::RULE_LABELED_ELEMENT)
            .and_then(|labeled| labeled.child_rule(p::RULE_ATOM))
            .or_else(|| node.child_rule(p::RULE_ATOM));
        let options = option_owner
            .into_iter()
            .flat_map(SyntaxNodeRef::descendants)
            .find(|child| child.rule_index() == Some(p::RULE_ELEMENT_OPTIONS))
            .map(parse_element_options)
            .unwrap_or_default();
        self.authored(ModelNodeId::Element(id), node);
        Element {
            id,
            kind,
            quantifier,
            label,
            options,
            syntax: node.id(),
            span,
            enclosing_span: node.span(),
        }
    }

    fn lexer_element(&mut self, node: SyntaxNodeRef<'_>) -> Element {
        let id = self.ids.element();
        let (kind, quantifier, span) = node.child_rule(p::RULE_LEXER_ATOM).map_or_else(
            || {
                let Some(block) = node.child_rule(p::RULE_LEXER_BLOCK) else {
                    let action = node
                        .child_rule(p::RULE_ACTION_BLOCK)
                        .expect("lexer element has a recognized shape");
                    return (
                        self.action_or_predicate(node, action),
                        Quantifier::One,
                        action.span(),
                    );
                };
                (
                    ElementKind::Block(self.lexer_block(block)),
                    suffix_quantifier(node.child_rule(p::RULE_EBNF_SUFFIX)),
                    block.span(),
                )
            },
            |atom| {
                (
                    Self::lexer_atom(atom, id),
                    suffix_quantifier(node.child_rule(p::RULE_EBNF_SUFFIX)),
                    atom.span(),
                )
            },
        );
        let options = node
            .child_rule(p::RULE_LEXER_ATOM)
            .into_iter()
            .flat_map(SyntaxNodeRef::descendants)
            .find(|child| child.rule_index() == Some(p::RULE_ELEMENT_OPTIONS))
            .map(parse_element_options)
            .unwrap_or_default();
        self.authored(ModelNodeId::Element(id), node);
        Element {
            id,
            kind,
            quantifier,
            label: None,
            options,
            syntax: node.id(),
            span,
            enclosing_span: node.span(),
        }
    }

    fn parser_atom(node: SyntaxNodeRef<'_>, source: ElementId) -> ElementKind {
        if let Some(terminal) = node.child_rule(p::RULE_TERMINAL_DEF) {
            return terminal_kind(terminal, false);
        }
        if let Some(reference) = node.child_rule(p::RULE_RULEREF) {
            return rule_call_kind(reference);
        }
        if let Some(not_set) = node.child_rule(p::RULE_NOT_SET) {
            return ElementKind::Set {
                inverted: true,
                elements: parse_set_elements(not_set, source),
            };
        }
        if node.child_rule(p::RULE_WILDCARD).is_some() {
            return ElementKind::Terminal(Terminal::Wildcard);
        }
        ElementKind::Epsilon
    }

    fn lexer_atom(node: SyntaxNodeRef<'_>, source: ElementId) -> ElementKind {
        if let Some(range) = node.child_rule(p::RULE_CHARACTER_RANGE) {
            return parse_range(range);
        }
        if let Some(terminal) = node.child_rule(p::RULE_TERMINAL_DEF) {
            return terminal_kind(terminal, true);
        }
        if let Some(reference) = node.child_terminal(p::RULE_REF) {
            return ElementKind::RuleCall(RuleCall {
                name: reference.text().to_owned(),
                arguments: None,
                precedence: None,
            });
        }
        if let Some(not_set) = node.child_rule(p::RULE_NOT_SET) {
            return ElementKind::Set {
                inverted: true,
                elements: parse_set_elements(not_set, source),
            };
        }
        if let Some(char_set) = node.child_terminal(p::LEXER_CHAR_SET) {
            return ElementKind::Terminal(Terminal::LexerCharSet(char_set.text().to_owned()));
        }
        if node.child_rule(p::RULE_WILDCARD).is_some() {
            return ElementKind::Terminal(Terminal::Wildcard);
        }
        ElementKind::Epsilon
    }

    fn action_or_predicate(
        &mut self,
        owner: SyntaxNodeRef<'_>,
        action: SyntaxNodeRef<'_>,
    ) -> ElementKind {
        let body = delimited_contents(action.text(), '{', '}');
        if owner.has_terminal(p::QUESTION) {
            let id = self.ids.predicate();
            self.authored(ModelNodeId::Predicate(id), owner);
            ElementKind::Predicate {
                id,
                body,
                fail: predicate_fail_message(owner),
                precedence: None,
            }
        } else {
            let id = self.ids.action();
            self.authored(ModelNodeId::Action(id), owner);
            ElementKind::Action { id, body }
        }
    }

    fn element_label(&mut self, node: SyntaxNodeRef<'_>) -> Option<Label> {
        let name = node
            .child_rule(p::RULE_IDENTIFIER)
            .and_then(SyntaxNodeRef::first_terminal)?;
        let id = self.ids.label();
        let label = Label {
            id,
            name: name.text().to_owned(),
            kind: if node.has_terminal(p::PLUS_ASSIGN) {
                LabelKind::List
            } else {
                LabelKind::Single
            },
            syntax: name.id(),
            span: name.span(),
        };
        self.authored(ModelNodeId::Label(id), name);
        Some(label)
    }

    fn named_action(
        &mut self,
        node: SyntaxNodeRef<'_>,
        default_scope: Option<String>,
    ) -> Option<NamedAction> {
        let scope = node
            .child_rule(p::RULE_ACTION_SCOPE_NAME)
            .and_then(SyntaxNodeRef::first_terminal)
            .map(|scope| scope.text().to_owned())
            .or(default_scope);
        let name = node
            .child_rule(p::RULE_IDENTIFIER)
            .and_then(SyntaxNodeRef::first_terminal)?
            .text()
            .to_owned();
        let action = node.child_rule(p::RULE_ACTION_BLOCK)?;
        Some(self.named_action_from_parts(node, scope, name, action))
    }

    fn named_action_from_parts(
        &mut self,
        owner: SyntaxNodeRef<'_>,
        scope: Option<String>,
        name: String,
        action: SyntaxNodeRef<'_>,
    ) -> NamedAction {
        let id = self.ids.action();
        self.authored(ModelNodeId::Action(id), owner);
        NamedAction {
            id,
            scope,
            name,
            body: delimited_contents(action.text(), '{', '}'),
            body_span: action.span(),
            syntax: owner.id(),
            span: owner.span(),
        }
    }

    fn authored(&mut self, id: ModelNodeId, node: SyntaxNodeRef<'_>) {
        self.provenance.record_model(
            id,
            [Origin::Authored {
                syntax: node.id(),
                span: node.span(),
            }],
        );
    }
}

fn parse_exception_handler(node: SyntaxNodeRef<'_>) -> Option<ExceptionHandler> {
    let argument = node.child_rule(p::RULE_ARG_ACTION_BLOCK)?;
    let action = node.child_rule(p::RULE_ACTION_BLOCK)?;
    Some(ExceptionHandler {
        argument: delimited_contents(argument.text(), '[', ']'),
        body: delimited_contents(action.text(), '{', '}'),
        body_span: action.span(),
        syntax: node.id(),
        span: node.span(),
    })
}

fn parse_lexer_command(node: SyntaxNodeRef<'_>) -> Option<LexerCommand> {
    let name = node
        .child_rule(p::RULE_LEXER_COMMAND_NAME)?
        .first_terminal()?
        .text()
        .to_owned();
    let argument_node = node.child_rule(p::RULE_LEXER_COMMAND_EXPR);
    let argument = argument_node.map(|argument| argument.text().to_owned());
    Some(LexerCommand {
        name,
        argument,
        argument_span: argument_node.map(SyntaxNodeRef::span),
        syntax: node.id(),
        span: node.span(),
    })
}

fn terminal_kind(node: SyntaxNodeRef<'_>, lexer: bool) -> ElementKind {
    node.child_terminal(p::TOKEN_REF).map_or_else(
        || {
            node.child_terminal(p::STRING_LITERAL)
                .map_or(ElementKind::Epsilon, |literal| {
                    ElementKind::Terminal(Terminal::Literal(literal.text().to_owned()))
                })
        },
        |token| {
            if lexer && token.text() == "EOF" {
                ElementKind::Terminal(Terminal::Eof)
            } else if lexer {
                ElementKind::RuleCall(RuleCall {
                    name: token.text().to_owned(),
                    arguments: None,
                    precedence: None,
                })
            } else {
                ElementKind::Terminal(Terminal::Token(token.text().to_owned()))
            }
        },
    )
}

fn rule_call_kind(node: SyntaxNodeRef<'_>) -> ElementKind {
    let name = node
        .child_terminal(p::RULE_REF)
        .map_or_else(String::new, |name| name.text().to_owned());
    let arguments = node
        .child_rule(p::RULE_ARG_ACTION_BLOCK)
        .map(|argument| delimited_contents(argument.text(), '[', ']'));
    ElementKind::RuleCall(RuleCall {
        name,
        arguments,
        precedence: None,
    })
}

fn parse_range(node: SyntaxNodeRef<'_>) -> ElementKind {
    let literals = node
        .child_terminals(p::STRING_LITERAL)
        .map(authored_text)
        .collect::<Vec<_>>();
    let operator_span = node
        .child_terminal(p::RANGE)
        .map_or_else(|| node.span(), SyntaxNodeRef::span);
    match literals.as_slice() {
        [start, stop] => ElementKind::Range(start.clone(), stop.clone(), operator_span),
        _ => ElementKind::Epsilon,
    }
}

fn parse_set_elements(node: SyntaxNodeRef<'_>, source: ElementId) -> Vec<SetElement> {
    let parent = node.child_rule(p::RULE_BLOCK_SET).unwrap_or(node);
    parent
        .child_rules(p::RULE_SET_ELEMENT)
        .filter_map(|element| parse_set_element(element, source))
        .collect()
}

fn parse_set_element(node: SyntaxNodeRef<'_>, source: ElementId) -> Option<SetElement> {
    if let Some(range) = node.child_rule(p::RULE_CHARACTER_RANGE) {
        let literals = range
            .child_terminals(p::STRING_LITERAL)
            .map(|literal| literal.text().to_owned())
            .collect::<Vec<_>>();
        return match literals.as_slice() {
            [start, stop] => Some(SetElement::Range {
                source,
                start: start.clone(),
                stop: stop.clone(),
                span: range
                    .child_terminal(p::RANGE)
                    .map_or_else(|| range.span(), SyntaxNodeRef::span),
                options: parse_set_member_options(node),
            }),
            _ => None,
        };
    }
    if let Some(token) = node.child_terminal(p::TOKEN_REF) {
        return Some(SetElement::Terminal {
            source,
            value: Terminal::Token(token.text().to_owned()),
            span: token.span(),
            options: parse_set_member_options(node),
        });
    }
    if let Some(literal) = node.child_terminal(p::STRING_LITERAL) {
        return Some(SetElement::Terminal {
            source,
            value: Terminal::Literal(literal.text().to_owned()),
            span: literal.span(),
            options: parse_set_member_options(node),
        });
    }
    node.child_terminal(p::LEXER_CHAR_SET)
        .map(|set| SetElement::Terminal {
            source,
            value: Terminal::LexerCharSet(set.text().to_owned()),
            span: set.span(),
            options: parse_set_member_options(node),
        })
}

fn parse_set_member_options(node: SyntaxNodeRef<'_>) -> Vec<OptionDecl> {
    node.child_rule(p::RULE_ELEMENT_OPTIONS)
        .map(parse_element_options)
        .unwrap_or_default()
}

fn parse_element_options(node: SyntaxNodeRef<'_>) -> Vec<OptionDecl> {
    node.child_rules(p::RULE_ELEMENT_OPTION)
        .filter_map(|option| {
            let identifiers = option
                .descendants()
                .filter(|child| child.rule_index() == Some(p::RULE_IDENTIFIER))
                .filter_map(SyntaxNodeRef::first_terminal)
                .collect::<Vec<_>>();
            let name = identifiers.first().copied()?;
            let value_node = if option.has_terminal(p::ASSIGN) {
                option
                    .children()
                    .filter(|child| {
                        child.rule_index() == Some(p::RULE_QUALIFIED_IDENTIFIER)
                            || child.token().is_some_and(|token| {
                                matches!(token.token_type, p::STRING_LITERAL | p::INT)
                            })
                    })
                    .last()
            } else {
                None
            };
            Some(OptionDecl {
                name: authored_text(name),
                value: value_node.map_or_else(
                    || Authored {
                        value: String::new(),
                        syntax: option.id(),
                        span: option.span(),
                    },
                    authored_node,
                ),
            })
        })
        .collect()
}

fn suffix_quantifier(suffix: Option<SyntaxNodeRef<'_>>) -> Quantifier {
    let Some(suffix) = suffix else {
        return Quantifier::One;
    };
    let greedy = suffix.child_terminals(p::QUESTION).count()
        <= usize::from(!suffix.has_terminal(p::STAR) && !suffix.has_terminal(p::PLUS));
    if suffix.has_terminal(p::STAR) {
        Quantifier::ZeroOrMore { greedy }
    } else if suffix.has_terminal(p::PLUS) {
        Quantifier::OneOrMore { greedy }
    } else {
        Quantifier::Optional { greedy }
    }
}

fn predicate_fail_message(node: SyntaxNodeRef<'_>) -> Option<String> {
    let options = node.child_rule(p::RULE_PREDICATE_OPTIONS)?;
    options
        .child_rules(p::RULE_PREDICATE_OPTION)
        .find_map(|option| {
            let text = option.text();
            let (name, value) = text.split_once('=')?;
            if name.trim() != "fail" {
                return None;
            }
            let value = value.trim();
            predicate_fail_value(value)
        })
}

fn predicate_fail_value(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(message) = grammar_string_literal(value) {
        return Some(message);
    }
    let action = value
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .map(str::trim);
    action
        .and_then(grammar_string_literal)
        .or_else(|| Some(value.to_owned()))
}

fn grammar_string_literal(value: &str) -> Option<String> {
    let quote = value.chars().next()?;
    (matches!(quote, '\'' | '"') && value.ends_with(quote))
        .then(|| get_string_from_grammar_string_literal(value))
        .flatten()
}

fn attribute_clause(node: SyntaxNodeRef<'_>) -> AttributeClause {
    AttributeClause {
        text: delimited_contents(node.text(), '[', ']'),
        span: node.span(),
    }
}

fn delimited_contents(text: &str, open: char, close: char) -> String {
    let text = text.trim();
    text.strip_prefix(open)
        .and_then(|text| text.strip_suffix(close))
        .unwrap_or(text)
        .to_owned()
}

pub(crate) const fn cst(file: &SourceFile) -> &Cst {
    file.cst()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;
    use crate::grammar::frontend::{SourceId, parse_source};

    #[test]
    fn converts_parser_rules_and_nested_elements() {
        let source = r#"
parser grammar P;
options { tokenVocab=L; }
tokens { EXTRA }
@parser::members { fn member() {} }
entry[int x] returns [int y] throws Error locals [int z]
options { memoize=true; }
@init { setup(); }
    : left=ID<assoc=right> (COMMA values+=entry[1])* {ready()}? <fail='no'> # Main
    |
    ;
catch [Error error] { recover(error); }
finally { finish(); }
"#;
        let file = parse_source(SourceId::new(2), "P.g4", source).expect("valid parser grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);

        // The whole converted parser unit (kind, tokens, rule arg/return/locals/throws clauses,
        // actions, catch/finally, alternative labels, element labels, quantifiers, predicate kinds)
        // is one structural regression target — far more observable than a dozen hand-picked pokes.
        insta::assert_debug_snapshot!("converts_parser_rules_and_nested_elements", unit);
        // Provenance is tracked in a separate index a `unit` snapshot cannot express, so keep it as
        // an explicit invariant.
        assert!(
            !provenance
                .origins(ModelNodeId::Rule(unit.rules[0].id))
                .is_empty()
        );
    }

    #[test]
    fn nested_actions_match_upstream() {
        let source = r#"
grammar T;
@definitions {
}
@members {
static isIdentifierChar (c: string) {
    return c.match(/^[0-9a-zA-Z_]+$/);
}
}
s : a ;
a : a ID {false}?<fail='custom message'>
  | ID
  ;
ID : 'a'..'z'+ ;
WS : (' '|'\n') -> skip ;
"#;
        let file = parse_source(SourceId::new(4), "T.g4", source).expect("valid combined grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);

        let members = unit
            .actions
            .iter()
            .find(|action| action.name == "members")
            .expect("members action");
        // The nested-brace action body is spliced verbatim; snapshot it whole rather than probing
        // for one substring, so any brace-matching or whitespace drift is visible.
        insta::assert_snapshot!("nested_actions_match_upstream", members.body);

        let rule = unit
            .rules
            .iter()
            .find(|rule| rule.name == "a")
            .expect("rule a");
        let (body, fail) = rule
            .block
            .alternatives
            .iter()
            .flat_map(|alternative| &alternative.elements)
            .find_map(|element| match &element.kind {
                ElementKind::Predicate { body, fail, .. } => Some((body.as_str(), fail.as_deref())),
                _ => None,
            })
            .expect("semantic predicate");
        assert_eq!(body, "false");
        assert_eq!(fail, Some("custom message"));
    }

    #[test]
    fn unwraps_braced_predicate_fail_literal() {
        assert_eq!(
            predicate_fail_value("{\n  \"custom message\"\n}").as_deref(),
            Some("custom message")
        );
        assert_eq!(
            predicate_fail_value("{ make_message() }").as_deref(),
            Some("{ make_message() }")
        );
    }

    #[test]
    fn converts_lexer_modes_sets_and_commands() {
        let source = r#"
lexer grammar L;
channels { COMMENTS }
TOKEN options { caseInsensitive=true; } : 'a'..'z'+ -> channel(COMMENTS), pushMode(M);
fragment FRAG : [a-z];
mode M;
MORE : ~('x'|'y')?? -> popMode;
"#;
        let file = parse_source(SourceId::new(3), "L.g4", source).expect("valid lexer grammar");
        let mut ids = ModelIdAllocator::after_loaded_grammars(1);
        let mut provenance = ProvenanceIndex::default();
        let unit = parse_grammar_unit(&file, GrammarId::new(0), &mut ids, &mut provenance);

        // Channels, case-insensitivity, ranges/sets (with inversion), commands, fragments, modes
        // and mode->rule wiring, and quantifiers all land in one lexer-unit snapshot.
        insta::assert_debug_snapshot!("converts_lexer_modes_sets_and_commands", unit);
    }
}
