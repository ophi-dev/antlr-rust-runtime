use std::fmt;
use std::io;
use std::ops::Range;
use std::path::PathBuf;

/// Broad category of a generator failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    Configuration,
    Compilation,
    Generation,
    Filesystem,
}

/// Severity of one grammar diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Warning,
    Error,
}

/// A grammar compiler diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    code: &'static str,
    severity: Severity,
    message: String,
    path: PathBuf,
    line: Option<usize>,
    column: Option<usize>,
    byte_span: Option<Range<usize>>,
}

impl Diagnostic {
    pub(crate) fn new(
        code: &'static str,
        severity: Severity,
        message: String,
        path: PathBuf,
        position: Option<(usize, usize)>,
        byte_span: Option<Range<usize>>,
    ) -> Self {
        Self {
            code,
            severity,
            message,
            path,
            line: position.map(|(line, _)| line),
            column: position.map(|(_, column)| column),
            byte_span,
        }
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }

    pub const fn severity(&self) -> Severity {
        self.severity
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub const fn line(&self) -> Option<usize> {
        self.line
    }

    pub const fn column(&self) -> Option<usize> {
        self.column
    }

    /// Half-open UTF-8 byte range of the primary subject within [`Self::path`].
    ///
    /// Returns `None` when the diagnostic has no source-backed primary subject.
    pub fn byte_span(&self) -> Option<Range<usize>> {
        self.byte_span.clone()
    }
}

/// Structured error returned by [`crate::Builder`].
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    diagnostics: Vec<Diagnostic>,
    source: Option<io::Error>,
}

impl Error {
    pub(crate) fn configuration(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Configuration,
            message: message.into(),
            diagnostics: Vec::new(),
            source: None,
        }
    }

    pub(crate) const fn compilation(message: String, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            kind: ErrorKind::Compilation,
            message,
            diagnostics,
            source: None,
        }
    }

    pub(crate) fn generation(source: io::Error) -> Self {
        let kind = if matches!(
            source.kind(),
            io::ErrorKind::NotFound
                | io::ErrorKind::PermissionDenied
                | io::ErrorKind::AlreadyExists
                | io::ErrorKind::ReadOnlyFilesystem
        ) {
            ErrorKind::Filesystem
        } else {
            ErrorKind::Generation
        };
        Self {
            kind,
            message: source.to_string(),
            diagnostics: Vec::new(),
            source: Some(source),
        }
    }

    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| -> &(dyn std::error::Error + 'static) { source })
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Self::generation(source)
    }
}
