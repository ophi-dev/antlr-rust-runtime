use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use antlr4_runtime::{
    AsRuleNode, CommonTokenStream, ErrorListener, InputStream, Node, NodeId, NodeKind, Parser,
    Recognizer, TOKEN_EOF as RUNTIME_TOKEN_EOF, Token,
};

use super::generated::antlr_v4_lexer::{
    AntlRv4Lexer, BLOCK_COMMENT, COLON, DOC_COMMENT, MODE, RANGE, RULE_REF, SEMI, STRING_LITERAL,
    UNTERMINATED_ARGUMENT, UNTERMINATED_CHAR_SET, UNTERMINATED_STRING_LITERAL,
};
use super::generated::antlr_v4_parser::{self as grammar_parser, ANTLRv4Listener, AntlRv4Parser};
use super::lexer_adaptor::LexerAdaptor;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceId(u32);

impl SourceId {
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SyntaxId(u64);

impl SyntaxId {
    pub(crate) const fn new(index: u32) -> Self {
        Self(index as u64)
    }

    pub(crate) const fn for_source(source: SourceId, index: u32) -> Self {
        Self(((source.0 as u64) << 32) | index as u64)
    }

    pub(crate) const fn index(self) -> usize {
        (self.0 as u32) as usize
    }

    pub(crate) const fn source(self) -> SourceId {
        SourceId::new((self.0 >> 32) as u32)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceSpan {
    pub(crate) source: SourceId,
    pub(crate) bytes: Range<u32>,
}

impl SourceSpan {
    pub(crate) const fn empty(source: SourceId) -> Self {
        Self {
            source,
            bytes: 0..0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SyntaxToken {
    pub(crate) token_type: i32,
    pub(crate) channel: i32,
    pub(crate) span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxNodeKind {
    Rule { rule_index: usize },
    Terminal { token_index: usize },
    Error { token_index: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SyntaxNode {
    pub(crate) kind: SyntaxNodeKind,
    pub(crate) span: SourceSpan,
    child_ids: Range<u32>,
}

#[derive(Debug)]
pub(crate) struct Cst {
    nodes: Box<[SyntaxNode]>,
    children: Box<[SyntaxId]>,
    root: SyntaxId,
}

impl Cst {
    pub(crate) const fn root_id(&self) -> SyntaxId {
        self.root
    }

    pub(crate) fn root(&self) -> &SyntaxNode {
        &self.nodes[self.root.index()]
    }

    pub(crate) fn node(&self, id: SyntaxId) -> Option<&SyntaxNode> {
        self.nodes.get(id.index())
    }

    pub(crate) fn children(&self, id: SyntaxId) -> impl DoubleEndedIterator<Item = SyntaxId> + '_ {
        self.node(id)
            .into_iter()
            .flat_map(|node| {
                self.children[node.child_ids.start as usize..node.child_ids.end as usize].iter()
            })
            .copied()
    }

    pub(crate) fn descendants(&self, id: SyntaxId) -> CstDescendants<'_> {
        CstDescendants {
            cst: self,
            pending: vec![id],
        }
    }
}

pub(crate) struct CstDescendants<'a> {
    cst: &'a Cst,
    pending: Vec<SyntaxId>,
}

impl Iterator for CstDescendants<'_> {
    type Item = SyntaxId;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.pending.pop()?;
        self.pending.extend(self.cst.children(id).rev());
        Some(id)
    }
}

#[derive(Debug)]
pub(crate) struct SourceFile {
    id: SourceId,
    logical_path: PathBuf,
    text: Box<str>,
    line_starts: Box<[u32]>,
    tokens: Box<[SyntaxToken]>,
    trivia: Box<[u32]>,
    cst: Cst,
}

impl SourceFile {
    pub(crate) const fn id(&self) -> SourceId {
        self.id
    }

    pub(crate) fn logical_path(&self) -> &Path {
        &self.logical_path
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn tokens(&self) -> &[SyntaxToken] {
        &self.tokens
    }

    pub(crate) fn token_text(&self, token: &SyntaxToken) -> &str {
        if token.token_type == RUNTIME_TOKEN_EOF {
            return "<EOF>";
        }
        let span = &token.span.bytes;
        &self.text[span.start as usize..span.end as usize]
    }

    pub(crate) fn trivia(&self) -> impl Iterator<Item = &SyntaxToken> {
        self.trivia
            .iter()
            .map(|index| &self.tokens[*index as usize])
    }

    pub(crate) const fn cst(&self) -> &Cst {
        &self.cst
    }

    pub(crate) fn line_column(&self, byte: u32) -> Option<(usize, usize)> {
        let byte = byte as usize;
        if byte > self.text.len() || !self.text.is_char_boundary(byte) {
            return None;
        }
        let line_index = self
            .line_starts
            .partition_point(|line_start| *line_start as usize <= byte)
            .saturating_sub(1);
        let line_start = self.line_starts[line_index] as usize;
        let column = self.text[line_start..byte].chars().count();
        Some((line_index + 1, column))
    }

    pub(crate) fn byte_offset(&self, line: usize, column: usize) -> Option<u32> {
        let line_start = *self.line_starts.get(line.checked_sub(1)?)? as usize;
        let line_end = self.text[line_start..]
            .find('\n')
            .map_or(self.text.len(), |offset| line_start + offset);
        let offset = if column == self.text[line_start..line_end].chars().count() {
            line_end
        } else {
            self.text[line_start..line_end]
                .char_indices()
                .nth(column)?
                .0
                + line_start
        };
        u32::try_from(offset).ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiagnosticStage {
    Source,
    Lexer,
    Parser,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SyntaxDiagnostic {
    pub(crate) code: &'static str,
    pub(crate) stage: DiagnosticStage,
    pub(crate) span: SourceSpan,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrontendError {
    diagnostics: Vec<SyntaxDiagnostic>,
}

impl FrontendError {
    pub(crate) fn diagnostics(&self) -> &[SyntaxDiagnostic] {
        &self.diagnostics
    }
}

impl fmt::Display for FrontendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "grammar frontend rejected the source with {} diagnostic(s)",
            self.diagnostics.len()
        )
    }
}

impl std::error::Error for FrontendError {}

pub(crate) struct RecoveredSource {
    pub(crate) file: SourceFile,
    pub(crate) diagnostics: Vec<SyntaxDiagnostic>,
}

pub(crate) fn parse_source(
    source: SourceId,
    logical_path: impl Into<PathBuf>,
    text: impl Into<Box<str>>,
) -> Result<SourceFile, FrontendError> {
    let recovered = parse_source_recovering(source, logical_path, text)?;
    if recovered.diagnostics.is_empty() {
        Ok(recovered.file)
    } else {
        Err(FrontendError {
            diagnostics: recovered.diagnostics,
        })
    }
}

pub(crate) fn parse_source_recovering(
    source: SourceId,
    logical_path: impl Into<PathBuf>,
    text: impl Into<Box<str>>,
) -> Result<RecoveredSource, FrontendError> {
    let logical_path = logical_path.into();
    let text = text.into();
    let line_starts = line_starts(source, &text)?;
    let input = InputStream::with_source_name(&text, logical_path.to_string_lossy());
    let mut lexer = AntlRv4Lexer::with_hooks(input, LexerAdaptor::default());
    lexer.remove_error_listeners();
    let mut token_stream = CommonTokenStream::try_new(lexer).map_err(|error| FrontendError {
        diagnostics: vec![SyntaxDiagnostic {
            code: "G4F001",
            stage: DiagnosticStage::Source,
            span: SourceSpan::empty(source),
            message: format!("could not buffer grammar tokens: {error}"),
        }],
    })?;
    token_stream.fill();

    let tokens = copy_tokens(source, &text, &token_stream)?;
    let mut diagnostics = token_stream
        .drain_source_errors()
        .into_iter()
        .map(|error| SyntaxDiagnostic {
            code: "G4F002",
            stage: DiagnosticStage::Lexer,
            span: diagnostic_span(
                source,
                &text,
                &line_starts,
                &tokens,
                error.line,
                error.column,
            ),
            message: error.message,
        })
        .collect::<Vec<_>>();
    diagnostics.extend(unterminated_diagnostics(source, &text, &tokens));
    if !diagnostics.is_empty() {
        return Err(FrontendError { diagnostics });
    }

    let collector = DiagnosticCollector::default();
    let mut parser = AntlRv4Parser::new(token_stream);
    parser.remove_error_listeners();
    parser.add_error_listener(collector.clone());
    let root = parser.grammar_spec();
    let syntax_error_count = parser.number_of_syntax_errors();
    let reported = collector.take();
    diagnostics.extend(reported.into_iter().map(|diagnostic| SyntaxDiagnostic {
        code: "G4F003",
        stage: DiagnosticStage::Parser,
        span: diagnostic_span(
            source,
            &text,
            &line_starts,
            &tokens,
            diagnostic.line,
            diagnostic.column,
        ),
        message: diagnostic.message,
    }));
    normalize_parser_diagnostics(&tokens, &mut diagnostics);

    let root = match root {
        Ok(root) => {
            if diagnostics.is_empty() {
                if syntax_error_count != 0 {
                    diagnostics.push(SyntaxDiagnostic {
                        code: "G4F003",
                        stage: DiagnosticStage::Parser,
                        span: SourceSpan::empty(source),
                        message: format!(
                            "grammar parser recovered from {syntax_error_count} syntax error(s)"
                        ),
                    });
                }
            }
            root
        }
        Err(error) => {
            if diagnostics.is_empty() {
                diagnostics.push(SyntaxDiagnostic {
                    code: "G4F003",
                    stage: DiagnosticStage::Parser,
                    span: SourceSpan::empty(source),
                    message: error.to_string(),
                });
            }
            return Err(FrontendError { diagnostics });
        }
    };

    let parsed = parser.into_parsed_file(root);
    let cst = match copy_cst(source, &tokens, parsed.tree()) {
        Ok(cst) => cst,
        Err(_) if !diagnostics.is_empty() => return Err(FrontendError { diagnostics }),
        Err(error) => return Err(error),
    };
    let trivia = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| {
            token.channel != antlr4_runtime::token::DEFAULT_CHANNEL
                && token.token_type != RUNTIME_TOKEN_EOF
        })
        .map(|(index, _)| index as u32)
        .collect();
    Ok(RecoveredSource {
        file: SourceFile {
            id: source,
            logical_path,
            text,
            line_starts,
            tokens: tokens.into_boxed_slice(),
            trivia,
            cst,
        },
        diagnostics,
    })
}

fn normalize_parser_diagnostics(tokens: &[SyntaxToken], diagnostics: &mut Vec<SyntaxDiagnostic>) {
    let significant = tokens
        .iter()
        .filter(|token| token.channel == antlr4_runtime::token::DEFAULT_CHANNEL)
        .collect::<Vec<_>>();

    for diagnostic in diagnostics.iter_mut() {
        let Some(index) = diagnostic_token_index(&significant, &diagnostic.span) else {
            continue;
        };
        if significant[index].token_type == RANGE
            && index > 0
            && significant[index - 1].token_type == STRING_LITERAL
            && significant
                .get(index + 1)
                .is_some_and(|token| token.token_type == STRING_LITERAL)
        {
            diagnostic.code = "G4S009";
            diagnostic.span = significant[index - 1].span.clone();
            "character ranges are not allowed in parser rules".clone_into(&mut diagnostic.message);
        }
    }

    let mut recovered = Vec::new();
    for diagnostic in diagnostics.iter() {
        let Some(index) = diagnostic_token_index(&significant, &diagnostic.span) else {
            continue;
        };
        if significant[index].token_type != RULE_REF
            || significant
                .get(index + 1)
                .is_none_or(|token| token.token_type != COLON)
        {
            continue;
        }
        let Some(mode_index) = significant[..index]
            .iter()
            .rposition(|token| token.token_type == MODE)
        else {
            continue;
        };
        if !significant[mode_index + 1..index]
            .iter()
            .any(|token| token.token_type == SEMI)
        {
            continue;
        }
        let Some(semicolon) = significant[index + 1..]
            .iter()
            .find(|token| token.token_type == SEMI)
        else {
            continue;
        };
        if diagnostics
            .iter()
            .chain(&recovered)
            .any(|existing| existing.span == semicolon.span)
        {
            continue;
        }
        recovered.push(SyntaxDiagnostic {
            code: "G4F003",
            stage: DiagnosticStage::Parser,
            span: semicolon.span.clone(),
            message: "mismatched input ';' expecting COLON while matching a lexer rule".to_owned(),
        });
    }
    diagnostics.extend(recovered);

    let mut seen = Vec::new();
    diagnostics.retain(|diagnostic| {
        if seen.contains(&diagnostic.span) {
            false
        } else {
            seen.push(diagnostic.span.clone());
            true
        }
    });
}

fn diagnostic_token_index(tokens: &[&SyntaxToken], span: &SourceSpan) -> Option<usize> {
    tokens
        .iter()
        .position(|token| token.span.bytes.start == span.bytes.start)
}

fn line_starts(source: SourceId, text: &str) -> Result<Box<[u32]>, FrontendError> {
    const LIMIT_EXCEEDED: &str = "grammar source exceeds the 4 GiB frontend limit";

    u32::try_from(text.len()).map_err(|_| invalid_span(source, LIMIT_EXCEEDED))?;

    let mut starts = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push((index + 1) as u32);
        }
    }
    Ok(starts.into_boxed_slice())
}

fn copy_tokens<S>(
    source: SourceId,
    text: &str,
    token_stream: &CommonTokenStream<S>,
) -> Result<Vec<SyntaxToken>, FrontendError>
where
    S: antlr4_runtime::TokenSource,
{
    token_stream
        .tokens()
        .map(|token| {
            let start = u32::try_from(token.start_byte());
            let end = u32::try_from(token.stop_byte());
            let (Ok(start), Ok(end)) = (start, end) else {
                return Err(invalid_span(source, "token byte span exceeds 4 GiB"));
            };
            if start > end
                || end as usize > text.len()
                || !text.is_char_boundary(start as usize)
                || !text.is_char_boundary(end as usize)
            {
                return Err(invalid_span(
                    source,
                    "token span is not on valid UTF-8 boundaries",
                ));
            }
            Ok(SyntaxToken {
                token_type: token.token_type(),
                channel: token.channel(),
                span: SourceSpan {
                    source,
                    bytes: start..end,
                },
            })
        })
        .collect()
}

fn invalid_span(source: SourceId, message: &str) -> FrontendError {
    FrontendError {
        diagnostics: vec![SyntaxDiagnostic {
            code: "G4F001",
            stage: DiagnosticStage::Source,
            span: SourceSpan::empty(source),
            message: message.to_owned(),
        }],
    }
}

fn unterminated_diagnostics(
    source: SourceId,
    text: &str,
    tokens: &[SyntaxToken],
) -> Vec<SyntaxDiagnostic> {
    tokens
        .iter()
        .filter_map(|token| {
            let token_text = token_text(text, token);
            let message = match token.token_type {
                UNTERMINATED_STRING_LITERAL => "unterminated string literal",
                UNTERMINATED_ARGUMENT => "unterminated argument",
                UNTERMINATED_CHAR_SET => "unterminated lexer character set",
                BLOCK_COMMENT | DOC_COMMENT if !token_text.ends_with("*/") => {
                    "unterminated block comment"
                }
                _ => return None,
            };
            Some(SyntaxDiagnostic {
                code: "G4F002",
                stage: DiagnosticStage::Lexer,
                span: SourceSpan {
                    source,
                    bytes: token.span.bytes.clone(),
                },
                message: message.to_owned(),
            })
        })
        .collect()
}

fn token_text<'a>(text: &'a str, token: &SyntaxToken) -> &'a str {
    if token.token_type == RUNTIME_TOKEN_EOF {
        "<EOF>"
    } else {
        &text[token.span.bytes.start as usize..token.span.bytes.end as usize]
    }
}

fn diagnostic_span(
    source: SourceId,
    text: &str,
    line_starts: &[u32],
    tokens: &[SyntaxToken],
    line: usize,
    column: usize,
) -> SourceSpan {
    let start = byte_offset(text, line_starts, line, column);
    if let Some(token) = tokens.iter().find(|token| token.span.bytes.start == start) {
        return token.span.clone();
    }
    let start_usize = start as usize;
    let end = text[start_usize..]
        .chars()
        .next()
        .map_or(start, |character| start + character.len_utf8() as u32);
    SourceSpan {
        source,
        bytes: start..end,
    }
}

fn byte_offset(text: &str, line_starts: &[u32], line: usize, column: usize) -> u32 {
    let line_start = line
        .checked_sub(1)
        .and_then(|index| line_starts.get(index))
        .copied()
        .unwrap_or_else(|| u32::try_from(text.len()).expect("source length checked"));
    let line_start_usize = line_start as usize;
    let line_end = text[line_start_usize..]
        .find('\n')
        .map_or(text.len(), |offset| line_start_usize + offset);
    let offset = text[line_start_usize..line_end]
        .char_indices()
        .nth(column)
        .map_or(line_end, |(offset, _)| line_start_usize + offset);
    u32::try_from(offset).expect("source length checked")
}

fn copy_cst(
    source: SourceId,
    tokens: &[SyntaxToken],
    root: Node<'_>,
) -> Result<Cst, FrontendError> {
    let mut builder = TypedCstBuilder::new(source, tokens);
    builder.walk(root)?;
    builder.finish()
}

struct OpenRule {
    syntax: SyntaxId,
    runtime: NodeId,
    rule_index: usize,
    children: Vec<SyntaxId>,
}

struct TypedCstBuilder<'tokens> {
    source: SourceId,
    tokens: &'tokens [SyntaxToken],
    nodes: Vec<SyntaxNode>,
    children: Vec<SyntaxId>,
    open_rules: Vec<OpenRule>,
    root: Option<SyntaxId>,
}

impl<'tokens> TypedCstBuilder<'tokens> {
    const fn new(source: SourceId, tokens: &'tokens [SyntaxToken]) -> Self {
        Self {
            source,
            tokens,
            nodes: Vec::new(),
            children: Vec::new(),
            open_rules: Vec::new(),
            root: None,
        }
    }

    fn finish(self) -> Result<Cst, FrontendError> {
        if !self.open_rules.is_empty() {
            return Err(invalid_span(
                self.source,
                "typed CST traversal left parser rules open",
            ));
        }
        let root = self
            .root
            .ok_or_else(|| invalid_span(self.source, "typed CST traversal produced no root"))?;
        Ok(Cst {
            nodes: self.nodes.into_boxed_slice(),
            children: self.children.into_boxed_slice(),
            root,
        })
    }

    fn enter_rule_context<'tree>(
        &mut self,
        context: &impl AsRuleNode<'tree>,
        expected_rule_index: usize,
    ) -> Result<(), FrontendError> {
        let rule = context.as_rule_node();
        if rule.rule_index() != expected_rule_index {
            return Err(invalid_span(
                self.source,
                &format!(
                    "typed listener dispatched rule {} as {expected_rule_index}",
                    rule.rule_index()
                ),
            ));
        }
        let node = rule.node();
        let span = node_span(self.source, self.tokens, node)?;
        let syntax = self.push_node(
            SyntaxNodeKind::Rule {
                rule_index: expected_rule_index,
            },
            span,
        )?;
        self.open_rules.push(OpenRule {
            syntax,
            runtime: node.id(),
            rule_index: expected_rule_index,
            children: Vec::new(),
        });
        Ok(())
    }

    fn exit_rule_context<'tree>(
        &mut self,
        context: &impl AsRuleNode<'tree>,
        expected_rule_index: usize,
    ) -> Result<(), FrontendError> {
        let rule = context.as_rule_node();
        let frame = self.open_rules.pop().ok_or_else(|| {
            invalid_span(
                self.source,
                "typed CST traversal exited a rule that was not entered",
            )
        })?;
        if frame.runtime != rule.node().id() || frame.rule_index != expected_rule_index {
            return Err(invalid_span(
                self.source,
                "typed CST traversal exited parser rules out of order",
            ));
        }
        let child_start = u32::try_from(self.children.len())
            .map_err(|_| invalid_span(self.source, "CST exceeds 2^32 edges"))?;
        self.children.extend(frame.children);
        let child_end = u32::try_from(self.children.len())
            .map_err(|_| invalid_span(self.source, "CST exceeds 2^32 edges"))?;
        self.nodes[frame.syntax.index()].child_ids = child_start..child_end;
        Ok(())
    }

    fn push_token(&mut self, token_index: usize, error: bool) -> Result<(), FrontendError> {
        let token = self
            .tokens
            .get(token_index)
            .ok_or_else(|| invalid_span(self.source, "CST references a missing token"))?;
        let kind = if error {
            SyntaxNodeKind::Error { token_index }
        } else {
            SyntaxNodeKind::Terminal { token_index }
        };
        self.push_node(kind, token.span.clone())?;
        Ok(())
    }

    fn push_node(
        &mut self,
        kind: SyntaxNodeKind,
        span: SourceSpan,
    ) -> Result<SyntaxId, FrontendError> {
        let node_index = u32::try_from(self.nodes.len())
            .map_err(|_| invalid_span(self.source, "CST exceeds 2^32 nodes"))?;
        let syntax = SyntaxId::for_source(self.source, node_index);
        self.nodes.push(SyntaxNode {
            kind,
            span,
            child_ids: 0..0,
        });
        if let Some(parent) = self.open_rules.last_mut() {
            parent.children.push(syntax);
        } else if self.root.is_none() {
            self.root = Some(syntax);
        } else {
            return Err(invalid_span(
                self.source,
                "typed CST traversal produced multiple roots",
            ));
        }
        Ok(syntax)
    }
}

macro_rules! typed_cst_rule_callbacks {
    ($( $enter:ident, $exit:ident => $context:ident, $rule:ident; )+) => {
        $(
            fn $enter(
                &mut self,
                context: &grammar_parser::$context<'_>,
            ) -> Result<(), FrontendError> {
                self.enter_rule_context(context, grammar_parser::$rule)
            }

            fn $exit(
                &mut self,
                context: &grammar_parser::$context<'_>,
            ) -> Result<(), FrontendError> {
                self.exit_rule_context(context, grammar_parser::$rule)
            }
        )+
    };
}

impl ANTLRv4Listener<FrontendError> for TypedCstBuilder<'_> {
    typed_cst_rule_callbacks! {
        enter_grammar_spec, exit_grammar_spec => GrammarSpecContext, RULE_GRAMMAR_SPEC;
        enter_grammar_decl, exit_grammar_decl => GrammarDeclContext, RULE_GRAMMAR_DECL;
        enter_grammar_type, exit_grammar_type => GrammarTypeContext, RULE_GRAMMAR_TYPE;
        enter_prequel_construct, exit_prequel_construct => PrequelConstructContext, RULE_PREQUEL_CONSTRUCT;
        enter_options_spec, exit_options_spec => OptionsSpecContext, RULE_OPTIONS_SPEC;
        enter_option, exit_option => OptionContext, RULE_OPTION;
        enter_option_value, exit_option_value => OptionValueContext, RULE_OPTION_VALUE;
        enter_delegate_grammars, exit_delegate_grammars => DelegateGrammarsContext, RULE_DELEGATE_GRAMMARS;
        enter_delegate_grammar, exit_delegate_grammar => DelegateGrammarContext, RULE_DELEGATE_GRAMMAR;
        enter_tokens_spec, exit_tokens_spec => TokensSpecContext, RULE_TOKENS_SPEC;
        enter_channels_spec, exit_channels_spec => ChannelsSpecContext, RULE_CHANNELS_SPEC;
        enter_id_list, exit_id_list => IdListContext, RULE_ID_LIST;
        enter_action, exit_action => ActionContext, RULE_ACTION;
        enter_action_scope_name, exit_action_scope_name => ActionScopeNameContext, RULE_ACTION_SCOPE_NAME;
        enter_action_block, exit_action_block => ActionBlockContext, RULE_ACTION_BLOCK;
        enter_arg_action_block, exit_arg_action_block => ArgActionBlockContext, RULE_ARG_ACTION_BLOCK;
        enter_mode_spec, exit_mode_spec => ModeSpecContext, RULE_MODE_SPEC;
        enter_rules, exit_rules => RulesContext, RULE_RULES;
        enter_rule_spec, exit_rule_spec => RuleSpecContext, RULE_RULE_SPEC;
        enter_parser_rule_spec, exit_parser_rule_spec => ParserRuleSpecContext, RULE_PARSER_RULE_SPEC;
        enter_exception_group, exit_exception_group => ExceptionGroupContext, RULE_EXCEPTION_GROUP;
        enter_exception_handler, exit_exception_handler => ExceptionHandlerContext, RULE_EXCEPTION_HANDLER;
        enter_finally_clause, exit_finally_clause => FinallyClauseContext, RULE_FINALLY_CLAUSE;
        enter_rule_prequel, exit_rule_prequel => RulePrequelContext, RULE_RULE_PREQUEL;
        enter_rule_returns, exit_rule_returns => RuleReturnsContext, RULE_RULE_RETURNS;
        enter_throws_spec, exit_throws_spec => ThrowsSpecContext, RULE_THROWS_SPEC;
        enter_locals_spec, exit_locals_spec => LocalsSpecContext, RULE_LOCALS_SPEC;
        enter_rule_action, exit_rule_action => RuleActionContext, RULE_RULE_ACTION;
        enter_rule_modifiers, exit_rule_modifiers => RuleModifiersContext, RULE_RULE_MODIFIERS;
        enter_rule_modifier, exit_rule_modifier => RuleModifierContext, RULE_RULE_MODIFIER;
        enter_rule_block, exit_rule_block => RuleBlockContext, RULE_RULE_BLOCK;
        enter_rule_alt_list, exit_rule_alt_list => RuleAltListContext, RULE_RULE_ALT_LIST;
        enter_labeled_alt, exit_labeled_alt => LabeledAltContext, RULE_LABELED_ALT;
        enter_lexer_rule_spec, exit_lexer_rule_spec => LexerRuleSpecContext, RULE_LEXER_RULE_SPEC;
        enter_lexer_rule_block, exit_lexer_rule_block => LexerRuleBlockContext, RULE_LEXER_RULE_BLOCK;
        enter_lexer_alt_list, exit_lexer_alt_list => LexerAltListContext, RULE_LEXER_ALT_LIST;
        enter_lexer_alt, exit_lexer_alt => LexerAltContext, RULE_LEXER_ALT;
        enter_lexer_elements, exit_lexer_elements => LexerElementsContext, RULE_LEXER_ELEMENTS;
        enter_lexer_element, exit_lexer_element => LexerElementContext, RULE_LEXER_ELEMENT;
        enter_lexer_block, exit_lexer_block => LexerBlockContext, RULE_LEXER_BLOCK;
        enter_lexer_commands, exit_lexer_commands => LexerCommandsContext, RULE_LEXER_COMMANDS;
        enter_lexer_command, exit_lexer_command => LexerCommandContext, RULE_LEXER_COMMAND;
        enter_lexer_command_name, exit_lexer_command_name => LexerCommandNameContext, RULE_LEXER_COMMAND_NAME;
        enter_lexer_command_expr, exit_lexer_command_expr => LexerCommandExprContext, RULE_LEXER_COMMAND_EXPR;
        enter_alt_list, exit_alt_list => AltListContext, RULE_ALT_LIST;
        enter_alternative, exit_alternative => AlternativeContext, RULE_ALTERNATIVE;
        enter_element, exit_element => ElementContext, RULE_ELEMENT;
        enter_predicate_options, exit_predicate_options => PredicateOptionsContext, RULE_PREDICATE_OPTIONS;
        enter_predicate_option, exit_predicate_option => PredicateOptionContext, RULE_PREDICATE_OPTION;
        enter_labeled_element, exit_labeled_element => LabeledElementContext, RULE_LABELED_ELEMENT;
        enter_ebnf, exit_ebnf => EbnfContext, RULE_EBNF;
        enter_block_suffix, exit_block_suffix => BlockSuffixContext, RULE_BLOCK_SUFFIX;
        enter_ebnf_suffix, exit_ebnf_suffix => EbnfSuffixContext, RULE_EBNF_SUFFIX;
        enter_lexer_atom, exit_lexer_atom => LexerAtomContext, RULE_LEXER_ATOM;
        enter_atom, exit_atom => AtomContext, RULE_ATOM;
        enter_wildcard, exit_wildcard => WildcardContext, RULE_WILDCARD;
        enter_not_set, exit_not_set => NotSetContext, RULE_NOT_SET;
        enter_block_set, exit_block_set => BlockSetContext, RULE_BLOCK_SET;
        enter_set_element, exit_set_element => SetElementContext, RULE_SET_ELEMENT;
        enter_block, exit_block => BlockContext, RULE_BLOCK;
        enter_ruleref, exit_ruleref => RulerefContext, RULE_RULEREF;
        enter_character_range, exit_character_range => CharacterRangeContext, RULE_CHARACTER_RANGE;
        enter_terminal_def, exit_terminal_def => TerminalDefContext, RULE_TERMINAL_DEF;
        enter_element_options, exit_element_options => ElementOptionsContext, RULE_ELEMENT_OPTIONS;
        enter_element_option, exit_element_option => ElementOptionContext, RULE_ELEMENT_OPTION;
        enter_identifier, exit_identifier => IdentifierContext, RULE_IDENTIFIER;
        enter_qualified_identifier, exit_qualified_identifier => QualifiedIdentifierContext, RULE_QUALIFIED_IDENTIFIER;
    }

    fn visit_terminal(
        &mut self,
        node: &grammar_parser::TerminalNode<'_>,
    ) -> Result<(), FrontendError> {
        self.push_token(node.symbol().token_id().index(), false)
    }

    fn visit_error_node(
        &mut self,
        node: &grammar_parser::ErrorNode<'_>,
    ) -> Result<(), FrontendError> {
        self.push_token(node.symbol().token_id().index(), true)
    }
}

fn node_span(
    source: SourceId,
    tokens: &[SyntaxToken],
    node: Node<'_>,
) -> Result<SourceSpan, FrontendError> {
    let token_span = |index: usize| {
        tokens
            .get(index)
            .map(|token| token.span.bytes.clone())
            .ok_or_else(|| invalid_span(source, "CST references a missing token"))
    };
    let bytes = match node.kind() {
        NodeKind::Terminal => token_span(
            node.as_terminal()
                .expect("terminal node kind checked")
                .token_id()
                .index(),
        )?,
        NodeKind::Error => token_span(
            node.as_error()
                .expect("error node kind checked")
                .token_id()
                .index(),
        )?,
        NodeKind::Rule => {
            let rule = node.as_rule().expect("rule node kind checked");
            let start = rule
                .start()
                .map(|token| token_span(token.token_id().index()))
                .transpose()?
                .map_or(0, |span| span.start);
            let end = rule
                .stop()
                .map(|token| token_span(token.token_id().index()))
                .transpose()?
                .map_or(start, |span| span.end)
                .max(start);
            start..end
        }
    };
    Ok(SourceSpan { source, bytes })
}

#[derive(Clone, Debug)]
struct ReportedDiagnostic {
    line: usize,
    column: usize,
    message: String,
}

#[derive(Clone, Debug, Default)]
struct DiagnosticCollector(Arc<Mutex<Vec<ReportedDiagnostic>>>);

impl DiagnosticCollector {
    fn take(&self) -> Vec<ReportedDiagnostic> {
        std::mem::take(
            &mut *self
                .0
                .lock()
                .expect("grammar diagnostic collector mutex poisoned"),
        )
    }
}

impl<R> ErrorListener<R> for DiagnosticCollector
where
    R: Recognizer + ?Sized,
{
    fn syntax_error(
        &mut self,
        _recognizer: &R,
        line: usize,
        column: usize,
        message: &str,
        _error: Option<&antlr4_runtime::AntlrError>,
    ) {
        self.0
            .lock()
            .expect("grammar diagnostic collector mutex poisoned")
            .push(ReportedDiagnostic {
                line,
                column,
                message: message.to_owned(),
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::fs;

    const SNAPSHOTS: &str = include_str!("../../../tests/codegen-direct/frontend-snapshots.tsv");

    #[test]
    fn pinned_frontend_corpus_matches_token_and_tree_oracles() {
        for (case_index, row) in SNAPSHOTS.lines().skip(1).enumerate() {
            let fields = row.split('\t').collect::<Vec<_>>();
            assert_eq!(fields.len(), 7, "malformed snapshot row: {row}");
            let path = workspace_root().join(fields[1]);
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            let file = parse_source(SourceId::new(case_index as u32), fields[1], text)
                .unwrap_or_else(|error| {
                    panic!("{}: {error}: {:?}", fields[0], error.diagnostics())
                });

            let (token_count, token_hash) = token_snapshot(&file);
            assert_eq!(token_count.to_string(), fields[3], "{} tokens", fields[0]);
            assert_eq!(token_hash, fields[4], "{} tokens", fields[0]);

            let (node_count, tree_hash) = tree_snapshot(&file);
            assert_eq!(node_count.to_string(), fields[5], "{} CST", fields[0]);
            assert_eq!(tree_hash, fields[6], "{} CST", fields[0]);
        }
    }

    #[test]
    fn malformed_bootstrap_inputs_fail_closed() {
        for name in ["BadAlternative.g4", "MissingDelimiter.g4"] {
            let path = workspace_root()
                .join("tests/codegen-direct/bootstrap/malformed")
                .join(name);
            let text = fs::read_to_string(&path).expect("malformed fixture should be readable");
            let error = parse_source(SourceId::new(0), &path, text)
                .expect_err("malformed grammar must not return a CST");
            assert_frontend_installed(&error);
            assert!(
                error
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.stage == DiagnosticStage::Parser),
                "{name}: {:?}",
                error.diagnostics()
            );
        }
    }

    #[test]
    fn malformed_editor_edit_fails_but_valid_undefined_rules_return_a_tree() {
        let malformed = "grammar A; a:: b \n| c; c: b+;";
        let error = parse_source(SourceId::new(7), "memory:malformed-edit", malformed)
            .expect_err("a:: must fail closed");
        assert_frontend_installed(&error);
        let spans = error
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.stage == DiagnosticStage::Parser)
            .map(|diagnostic| diagnostic.span.bytes.clone())
            .collect::<Vec<_>>();
        assert_eq!(spans, [12..14, 18..19, 21..22]);

        let valid = "grammar A; a: b \n| c; c: b+;";
        let file = parse_source(SourceId::new(8), "memory:undefined-rules", valid)
            .expect("syntax-only Phase A must return a CST");
        assert_eq!(file.cst().root().span.bytes, 0..valid.len() as u32);
    }

    #[test]
    fn unterminated_constructs_fail_closed() {
        let cases = [
            ("string", "grammar A; a: 'unterminated\n;"),
            ("action", "grammar A; @members { unterminated"),
            ("argument", "grammar A; a[unterminated: A;"),
            ("character set", "lexer grammar A; A: [unterminated;"),
            ("comment", "grammar A; /* unterminated"),
        ];
        for (name, text) in cases {
            let error = parse_source(SourceId::new(0), name, text)
                .expect_err("unterminated input must not return a CST");
            assert_frontend_installed(&error);
            assert!(
                error.diagnostics().iter().any(|diagnostic| matches!(
                    diagnostic.stage,
                    DiagnosticStage::Lexer | DiagnosticStage::Parser
                )),
                "{name}: {:?}",
                error.diagnostics()
            );
        }
    }

    #[test]
    fn tparser_preserves_named_action_rule_and_argument_spans() {
        const ACTION_BLOCK_RULE: usize = 14;
        const ARG_ACTION_BLOCK_RULE: usize = 15;
        const PARSER_RULE_SPEC_RULE: usize = 19;

        let path = workspace_root()
            .join("tests/codegen-direct/external/vscode-antlr4/tests/backend/test-data/TParser.g4");
        let text = fs::read_to_string(&path).expect("TParser fixture should be readable");
        let file = parse_source(SourceId::new(11), &path, text.clone())
            .expect("TParser should be syntactically valid");

        assert_rule_span(
            &file,
            ACTION_BLOCK_RULE,
            byte_offset(&text, 30, 17)..byte_offset(&text, 37, 1),
        );
        assert_rule_span(
            &file,
            PARSER_RULE_SPEC_RULE,
            byte_offset(&text, 82, 0)..byte_offset(&text, 90, 1),
        );
        assert_rule_span(
            &file,
            ARG_ACTION_BLOCK_RULE,
            byte_offset(&text, 82, 63)..byte_offset(&text, 82, 90),
        );
    }

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn assert_frontend_installed(error: &FrontendError) {
        assert!(
            error
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.code != "G4F000"),
            "red fingerprint: Stage 0 frontend is not installed"
        );
    }

    fn assert_rule_span(file: &SourceFile, rule_index: usize, expected: Range<u32>) {
        assert!(
            file.cst.nodes.iter().any(|node| {
                node.kind == SyntaxNodeKind::Rule { rule_index } && node.span.bytes == expected
            }),
            "missing rule {rule_index} span {expected:?}"
        );
    }

    fn byte_offset(text: &str, one_based_line: usize, column: usize) -> u32 {
        let line_start = text
            .split_inclusive('\n')
            .take(one_based_line.saturating_sub(1))
            .map(str::len)
            .sum::<usize>();
        let line = text[line_start..]
            .split_once('\n')
            .map_or(&text[line_start..], |(line, _)| line);
        let column_bytes = line
            .char_indices()
            .nth(column)
            .map_or(line.len(), |(offset, _)| offset);
        u32::try_from(line_start + column_bytes).expect("fixture offset should fit in u32")
    }

    fn token_snapshot(file: &SourceFile) -> (usize, String) {
        let mut hash = Fnv1a64::new();
        let mut count = 0;
        for token in file
            .tokens()
            .iter()
            .filter(|token| token.token_type != antlr4_runtime::TOKEN_EOF)
        {
            let mut row = format!(
                "{}\t{}\t{}\t{}\t",
                token.token_type, token.channel, token.span.bytes.start, token.span.bytes.end
            );
            push_json_string(&mut row, file.token_text(token));
            row.push('\n');
            hash.update(row.as_bytes());
            count += 1;
        }
        (count, hash.finish())
    }

    fn tree_snapshot(file: &SourceFile) -> (usize, String) {
        let mut hash = Fnv1a64::new();
        let mut count = 0;
        snapshot_node(file, file.cst.root, &mut hash, &mut count);
        (count, hash.finish())
    }

    fn snapshot_node(file: &SourceFile, id: SyntaxId, hash: &mut Fnv1a64, count: &mut usize) {
        *count += 1;
        let node = file.cst().node(id).expect("CST child ID should resolve");
        let mut row = String::new();
        match node.kind {
            SyntaxNodeKind::Rule { rule_index } => {
                writeln!(
                    row,
                    "R\t{}\t{}",
                    rule_index,
                    file.cst().children(id).count()
                )
                .expect("writing to String cannot fail");
            }
            SyntaxNodeKind::Terminal { token_index } | SyntaxNodeKind::Error { token_index } => {
                let prefix = if matches!(node.kind, SyntaxNodeKind::Error { .. }) {
                    'E'
                } else {
                    'T'
                };
                let token = &file.tokens()[token_index];
                write!(row, "{prefix}\t{}\t", token.token_type)
                    .expect("writing to String cannot fail");
                push_json_string(&mut row, file.token_text(token));
                row.push('\n');
            }
        }
        hash.update(row.as_bytes());
        for child in file.cst().children(id) {
            snapshot_node(file, child, hash, count);
        }
    }

    fn push_json_string(output: &mut String, text: &str) {
        output.push('"');
        for character in text.chars() {
            match character {
                '"' => output.push_str("\\\""),
                '\\' => output.push_str("\\\\"),
                '\u{0008}' => output.push_str("\\b"),
                '\u{000c}' => output.push_str("\\f"),
                '\n' => output.push_str("\\n"),
                '\r' => output.push_str("\\r"),
                '\t' => output.push_str("\\t"),
                '\u{0000}'..='\u{001f}' => {
                    write!(output, "\\u{:04x}", character as u32)
                        .expect("writing to String cannot fail");
                }
                _ => output.push(character),
            }
        }
        output.push('"');
    }

    struct Fnv1a64(u64);

    impl Fnv1a64 {
        const fn new() -> Self {
            Self(0xcbf2_9ce4_8422_2325)
        }

        fn update(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 ^= u64::from(*byte);
                self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }

        fn finish(self) -> String {
            format!("{:016x}", self.0)
        }
    }
}
