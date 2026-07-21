use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use antlr4_runtime::{
    CommonTokenStream, ErrorListener, InputStream, Node, NodeKind, Parser, Recognizer,
    TOKEN_EOF as RUNTIME_TOKEN_EOF, Token,
};

use super::generated::antlr_v4_lexer::{
    AntlRv4Lexer, BLOCK_COMMENT, DOC_COMMENT, UNTERMINATED_ARGUMENT, UNTERMINATED_CHAR_SET,
    UNTERMINATED_STRING_LITERAL,
};
use super::generated::antlr_v4_parser::AntlRv4Parser;
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

pub(crate) fn parse_source(
    source: SourceId,
    logical_path: impl Into<PathBuf>,
    text: impl Into<Box<str>>,
) -> Result<SourceFile, FrontendError> {
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

    let root = match root {
        Ok(root) if syntax_error_count == 0 && diagnostics.is_empty() => root,
        Ok(_) => {
            if diagnostics.is_empty() {
                diagnostics.push(SyntaxDiagnostic {
                    code: "G4F003",
                    stage: DiagnosticStage::Parser,
                    span: SourceSpan::empty(source),
                    message: format!(
                        "grammar parser recovered from {syntax_error_count} syntax error(s)"
                    ),
                });
            }
            return Err(FrontendError { diagnostics });
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
    let cst = copy_cst(source, &tokens, parsed.tree())?;
    let trivia = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| {
            token.channel != antlr4_runtime::token::DEFAULT_CHANNEL
                && token.token_type != RUNTIME_TOKEN_EOF
        })
        .map(|(index, _)| index as u32)
        .collect();
    Ok(SourceFile {
        id: source,
        logical_path,
        text,
        line_starts,
        tokens: tokens.into_boxed_slice(),
        trivia,
        cst,
    })
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
    let mut nodes = Vec::new();
    let mut children = Vec::new();
    let root = copy_node(source, tokens, root, &mut nodes, &mut children)?;
    Ok(Cst {
        nodes: nodes.into_boxed_slice(),
        children: children.into_boxed_slice(),
        root,
    })
}

fn copy_node(
    source: SourceId,
    tokens: &[SyntaxToken],
    node: Node<'_>,
    nodes: &mut Vec<SyntaxNode>,
    children: &mut Vec<SyntaxId>,
) -> Result<SyntaxId, FrontendError> {
    let node_index =
        u32::try_from(nodes.len()).map_err(|_| invalid_span(source, "CST exceeds 2^32 nodes"))?;
    let kind = match node.kind() {
        NodeKind::Rule => SyntaxNodeKind::Rule {
            rule_index: node.as_rule().expect("rule node kind checked").rule_index(),
        },
        NodeKind::Terminal => SyntaxNodeKind::Terminal {
            token_index: node
                .as_terminal()
                .expect("terminal node kind checked")
                .token_id()
                .index(),
        },
        NodeKind::Error => SyntaxNodeKind::Error {
            token_index: node
                .as_error()
                .expect("error node kind checked")
                .token_id()
                .index(),
        },
    };
    let span = node_span(source, tokens, node)?;
    nodes.push(SyntaxNode {
        kind,
        span,
        child_ids: 0..0,
    });

    let mut direct_children = Vec::new();
    for child in node.children() {
        direct_children.push(copy_node(source, tokens, child, nodes, children)?);
    }
    let child_start = u32::try_from(children.len())
        .map_err(|_| invalid_span(source, "CST exceeds 2^32 edges"))?;
    children.extend(direct_children);
    let child_end = u32::try_from(children.len())
        .map_err(|_| invalid_span(source, "CST exceeds 2^32 edges"))?;
    nodes[node_index as usize].child_ids = child_start..child_end;
    Ok(SyntaxId::for_source(source, node_index))
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
