use std::collections::BTreeMap;

use super::frontend::{SourceId, SourceSpan, SyntaxId};

macro_rules! dense_id {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub(crate) struct $name(u32);

        impl $name {
            pub(crate) const fn new(index: u32) -> Self {
                Self(index)
            }

            pub(crate) const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

dense_id!(GrammarId);
dense_id!(RuleId);
dense_id!(AlternativeId);
dense_id!(ElementId);
dense_id!(LabelId);
dense_id!(ActionId);
dense_id!(PredicateId);
dense_id!(ModeId);
dense_id!(TokenSymbolId);
dense_id!(ChannelId);
dense_id!(TransformId);
dense_id!(BuildStateId);
dense_id!(BuildTransitionId);

#[derive(Clone, Debug, Default)]
pub(crate) struct ModelIdAllocator {
    grammar: u32,
    rule: u32,
    alternative: u32,
    element: u32,
    label: u32,
    action: u32,
    predicate: u32,
    mode: u32,
    token: u32,
    channel: u32,
}

impl ModelIdAllocator {
    pub(crate) fn after_loaded_grammars(count: usize) -> Self {
        Self {
            grammar: u32::try_from(count).expect("grammar count exceeds compact ID"),
            ..Self::default()
        }
    }

    pub(crate) const fn grammar(&mut self) -> GrammarId {
        let id = GrammarId::new(self.grammar);
        self.grammar = self.grammar.checked_add(1).expect("grammar ID overflow");
        id
    }

    pub(crate) const fn rule(&mut self) -> RuleId {
        let id = RuleId::new(self.rule);
        self.rule = self.rule.checked_add(1).expect("rule ID overflow");
        id
    }

    pub(crate) const fn alternative(&mut self) -> AlternativeId {
        let id = AlternativeId::new(self.alternative);
        self.alternative = self
            .alternative
            .checked_add(1)
            .expect("alternative ID overflow");
        id
    }

    pub(crate) const fn element(&mut self) -> ElementId {
        let id = ElementId::new(self.element);
        self.element = self.element.checked_add(1).expect("element ID overflow");
        id
    }

    pub(crate) const fn label(&mut self) -> LabelId {
        let id = LabelId::new(self.label);
        self.label = self.label.checked_add(1).expect("label ID overflow");
        id
    }

    pub(crate) const fn action(&mut self) -> ActionId {
        let id = ActionId::new(self.action);
        self.action = self.action.checked_add(1).expect("action ID overflow");
        id
    }

    pub(crate) const fn predicate(&mut self) -> PredicateId {
        let id = PredicateId::new(self.predicate);
        self.predicate = self
            .predicate
            .checked_add(1)
            .expect("predicate ID overflow");
        id
    }

    pub(crate) const fn mode(&mut self) -> ModeId {
        let id = ModeId::new(self.mode);
        self.mode = self.mode.checked_add(1).expect("mode ID overflow");
        id
    }

    pub(crate) const fn token(&mut self) -> TokenSymbolId {
        let id = TokenSymbolId::new(self.token);
        self.token = self.token.checked_add(1).expect("token ID overflow");
        id
    }

    pub(crate) const fn channel(&mut self) -> ChannelId {
        let id = ChannelId::new(self.channel);
        self.channel = self.channel.checked_add(1).expect("channel ID overflow");
        id
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum ModelNodeId {
    Grammar(GrammarId),
    Rule(RuleId),
    Alternative(AlternativeId),
    Element(ElementId),
    Label(LabelId),
    Action(ActionId),
    Predicate(PredicateId),
    Mode(ModeId),
    Token(TokenSymbolId),
    Channel(ChannelId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GrammarKind {
    Lexer,
    Parser,
    Combined,
}

impl GrammarKind {
    pub(crate) const fn accepts_import(self, imported: Self) -> bool {
        matches!(
            (self, imported),
            (Self::Lexer, Self::Lexer)
                | (Self::Parser, Self::Parser)
                | (Self::Combined, Self::Lexer | Self::Parser | Self::Combined)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Authored<T> {
    pub(crate) value: T,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrammarHeader {
    pub(crate) name: Authored<String>,
    pub(crate) kind: GrammarKind,
    pub(crate) declaration_span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportDecl {
    pub(crate) alias: Option<Authored<String>>,
    pub(crate) grammar: Authored<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OptionDecl {
    pub(crate) name: Authored<String>,
    pub(crate) value: Authored<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParsedGrammarUnit {
    pub(crate) source: SourceId,
    pub(crate) header: GrammarHeader,
    pub(crate) imports: Vec<ImportDecl>,
    pub(crate) options: Vec<OptionDecl>,
    pub(crate) token_vocab: Option<Authored<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LookupKind {
    Import,
    TokenVocabSource,
    TokenVocabFile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LookupRecord {
    pub(crate) kind: LookupKind,
    pub(crate) requested: String,
    pub(crate) selected: Option<std::path::PathBuf>,
    pub(crate) shadowed: Vec<std::path::PathBuf>,
    pub(crate) at: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportEdge {
    pub(crate) id: u32,
    pub(crate) importer: GrammarId,
    pub(crate) imported: GrammarId,
    pub(crate) declaration: ImportDecl,
    pub(crate) lookup: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum VocabularySource {
    Grammar(GrammarId),
    TokensFile(std::path::PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VocabularyEdge {
    pub(crate) importer: GrammarId,
    pub(crate) source: VocabularySource,
    pub(crate) declaration: Authored<String>,
    pub(crate) lookup: usize,
}

#[derive(Debug)]
pub(crate) struct LoadedGrammarSet {
    pub(crate) grammars: Vec<ParsedGrammarUnit>,
    pub(crate) roots: Vec<GrammarId>,
    pub(crate) imports: Vec<ImportEdge>,
    pub(crate) vocabularies: Vec<VocabularyEdge>,
    pub(crate) lookups: Vec<LookupRecord>,
    pub(crate) by_name: BTreeMap<String, GrammarId>,
    pub(crate) load_order: Vec<GrammarId>,
}

impl LoadedGrammarSet {
    pub(crate) fn grammar(&self, id: GrammarId) -> &ParsedGrammarUnit {
        &self.grammars[id.index()]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Quantifier {
    One,
    Optional { greedy: bool },
    ZeroOrMore { greedy: bool },
    OneOrMore { greedy: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LabelKind {
    Single,
    List,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Label {
    pub(crate) id: LabelId,
    pub(crate) name: String,
    pub(crate) kind: LabelKind,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuleCall {
    pub(crate) name: String,
    pub(crate) arguments: Option<String>,
    pub(crate) precedence: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Terminal {
    Token(String),
    Literal(String),
    LexerCharSet(String),
    Wildcard,
    Eof,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SetElement {
    Terminal {
        source: ElementId,
        value: Terminal,
        options: Vec<OptionDecl>,
    },
    Range {
        source: ElementId,
        start: String,
        stop: String,
        options: Vec<OptionDecl>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ElementKind {
    Terminal(Terminal),
    RuleCall(RuleCall),
    Range(String, String),
    Set {
        inverted: bool,
        elements: Vec<SetElement>,
    },
    Block(Block),
    Action {
        id: ActionId,
        body: String,
    },
    Predicate {
        id: PredicateId,
        body: String,
        fail: Option<String>,
        precedence: Option<u32>,
    },
    Epsilon,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Element {
    pub(crate) id: ElementId,
    pub(crate) kind: ElementKind,
    pub(crate) quantifier: Quantifier,
    pub(crate) label: Option<Label>,
    pub(crate) options: Vec<OptionDecl>,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LexerCommand {
    pub(crate) name: String,
    pub(crate) argument: Option<String>,
    pub(crate) argument_span: Option<SourceSpan>,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Alternative {
    pub(crate) id: AlternativeId,
    pub(crate) elements: Vec<Element>,
    pub(crate) label: Option<Authored<String>>,
    pub(crate) options: Vec<OptionDecl>,
    pub(crate) commands: Vec<LexerCommand>,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Block {
    pub(crate) alternatives: Vec<Alternative>,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuleKind {
    Parser,
    Lexer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NamedAction {
    pub(crate) id: ActionId,
    pub(crate) scope: Option<String>,
    pub(crate) name: String,
    pub(crate) body: String,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExceptionHandler {
    pub(crate) argument: String,
    pub(crate) body: String,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LeftRecursiveAlternativeKind {
    Primary,
    Prefix,
    Binary,
    Suffix,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LeftRecursionInfo {
    pub(crate) original_to_rewritten: BTreeMap<AlternativeId, AlternativeId>,
    pub(crate) alternative_kinds: BTreeMap<AlternativeId, LeftRecursiveAlternativeKind>,
    pub(crate) deleted_labels: BTreeMap<LabelId, AlternativeId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Rule {
    pub(crate) id: RuleId,
    pub(crate) name: String,
    pub(crate) kind: RuleKind,
    pub(crate) fragment: bool,
    pub(crate) modifiers: Vec<Authored<String>>,
    pub(crate) arguments: Option<String>,
    pub(crate) returns: Option<String>,
    pub(crate) locals: Option<String>,
    pub(crate) throws: Vec<Authored<String>>,
    pub(crate) options: Vec<OptionDecl>,
    pub(crate) actions: Vec<NamedAction>,
    pub(crate) catches: Vec<ExceptionHandler>,
    pub(crate) finally_action: Option<NamedAction>,
    pub(crate) left_recursion: Option<LeftRecursionInfo>,
    pub(crate) block: Block,
    pub(crate) mode: Option<ModeId>,
    pub(crate) case_insensitive: Option<bool>,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Mode {
    pub(crate) id: ModeId,
    pub(crate) name: String,
    pub(crate) rules: Vec<RuleId>,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TokenDeclaration {
    pub(crate) id: TokenSymbolId,
    pub(crate) name: Authored<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChannelDeclaration {
    pub(crate) id: ChannelId,
    pub(crate) name: Authored<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrammarUnit {
    pub(crate) id: GrammarId,
    pub(crate) source: SourceId,
    pub(crate) name: String,
    pub(crate) kind: GrammarKind,
    pub(crate) options: Vec<OptionDecl>,
    pub(crate) tokens: Vec<TokenDeclaration>,
    pub(crate) channels: Vec<ChannelDeclaration>,
    pub(crate) actions: Vec<NamedAction>,
    pub(crate) modes: Vec<Mode>,
    pub(crate) rules: Vec<Rule>,
    pub(crate) syntax: SyntaxId,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SemanticBindings {
    pub(crate) alternatives: BTreeMap<AlternativeId, RuleId>,
    pub(crate) actions: BTreeMap<ActionId, ActionBinding>,
    pub(crate) predicates: BTreeMap<PredicateId, PredicateBinding>,
    pub(crate) labels: BTreeMap<LabelId, LabelBinding>,
    pub(crate) rule_calls: BTreeMap<ElementId, RuleCallBinding>,
    pub(crate) terminals: BTreeMap<ElementId, TerminalBinding>,
    pub(crate) commands: BTreeMap<(AlternativeId, usize), LexerCommandBinding>,
    pub(crate) attributes: BTreeMap<RuleId, RuleAttributes>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActionBinding {
    pub(crate) rule: RuleId,
    pub(crate) alternative: AlternativeId,
    pub(crate) element: ElementId,
    pub(crate) index: usize,
    pub(crate) context_dependent: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PredicateBinding {
    pub(crate) rule: RuleId,
    pub(crate) alternative: AlternativeId,
    pub(crate) element: ElementId,
    pub(crate) index: usize,
    pub(crate) precedence: Option<u32>,
    pub(crate) context_dependent: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LabelBinding {
    pub(crate) alternative: AlternativeId,
    pub(crate) element: ElementId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuleCallBinding {
    pub(crate) caller: RuleId,
    pub(crate) target: RuleId,
    pub(crate) precedence: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalBinding {
    pub(crate) token_type: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResolvedLexerCommand {
    Skip,
    More,
    PopMode,
    Mode(usize),
    PushMode(usize),
    Type(i32),
    Channel(i32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LexerCommandBinding {
    pub(crate) rule: RuleId,
    pub(crate) command: ResolvedLexerCommand,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RuleAttributes {
    pub(crate) arguments: Vec<AttributeSymbol>,
    pub(crate) returns: Vec<AttributeSymbol>,
    pub(crate) locals: Vec<AttributeSymbol>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttributeSymbol {
    pub(crate) name: String,
    pub(crate) ty: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TokenSymbol {
    pub(crate) id: TokenSymbolId,
    pub(crate) number: i32,
    pub(crate) name: Option<String>,
    pub(crate) literal: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Vocabulary {
    pub(crate) tokens: Vec<TokenSymbol>,
    pub(crate) by_name: BTreeMap<String, i32>,
    pub(crate) by_literal: BTreeMap<String, i32>,
}

impl Vocabulary {
    pub(crate) fn max_token_type(&self) -> i32 {
        self.tokens
            .iter()
            .map(|token| token.number)
            .max()
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecognizerModel {
    pub(crate) grammar: GrammarId,
    pub(crate) name: String,
    pub(crate) kind: GrammarKind,
    pub(crate) rule_names: Vec<String>,
    pub(crate) rule_numbers: BTreeMap<RuleId, usize>,
    pub(crate) vocabulary: Vocabulary,
    pub(crate) literal_names: Vec<Option<String>>,
    pub(crate) symbolic_names: Vec<Option<String>>,
    pub(crate) channel_names: Vec<Option<String>>,
    pub(crate) channel_numbers: BTreeMap<String, i32>,
    pub(crate) mode_names: Vec<String>,
    pub(crate) mode_numbers: BTreeMap<String, usize>,
    pub(crate) action_numbers: BTreeMap<ActionId, usize>,
    pub(crate) predicate_numbers: BTreeMap<PredicateId, usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct SemanticGrammar {
    pub(crate) unit: GrammarUnit,
    pub(crate) recognizer: RecognizerModel,
    pub(crate) bindings: SemanticBindings,
    pub(crate) call_graph: BTreeMap<RuleId, Vec<RuleId>>,
    pub(crate) entry_rules: Vec<RuleId>,
}
