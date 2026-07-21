use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

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
pub(crate) struct SyntaxId(u32);

impl SyntaxId {
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
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
    pub(crate) fn root(&self) -> &SyntaxNode {
        &self.nodes[self.root.index()]
    }

    pub(crate) fn node(&self, id: SyntaxId) -> Option<&SyntaxNode> {
        self.nodes.get(id.index())
    }

    pub(crate) fn children(&self, id: SyntaxId) -> impl Iterator<Item = SyntaxId> + '_ {
        self.node(id)
            .into_iter()
            .flat_map(|node| {
                self.children[node.child_ids.start as usize..node.child_ids.end as usize].iter()
            })
            .copied()
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
    _logical_path: impl Into<PathBuf>,
    _text: impl Into<Box<str>>,
) -> Result<SourceFile, FrontendError> {
    Err(FrontendError {
        diagnostics: vec![SyntaxDiagnostic {
            code: "G4F000",
            stage: DiagnosticStage::Source,
            span: SourceSpan::empty(source),
            message: "the Stage 0 grammar frontend is not installed".to_owned(),
        }],
    })
}
