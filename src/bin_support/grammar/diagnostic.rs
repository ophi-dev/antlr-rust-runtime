use std::fmt;
use std::path::PathBuf;

use super::frontend::SourceSpan;
use super::source::SourceSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RelatedDiagnostic {
    pub(crate) span: SourceSpan,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Diagnostic {
    pub(crate) code: &'static str,
    pub(crate) severity: Severity,
    pub(crate) message: String,
    pub(crate) primary: SourceSpan,
    pub(crate) related: Vec<RelatedDiagnostic>,
}

impl Diagnostic {
    pub(crate) fn error(
        code: &'static str,
        primary: SourceSpan,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            primary,
            related: Vec::new(),
        }
    }

    pub(crate) fn warning(
        code: &'static str,
        primary: SourceSpan,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            message: message.into(),
            primary,
            related: Vec::new(),
        }
    }

    #[must_use]
    pub(crate) fn with_related(mut self, span: SourceSpan, message: impl Into<String>) -> Self {
        self.related.push(RelatedDiagnostic {
            span,
            message: message.into(),
        });
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompilationError {
    diagnostics: Vec<Diagnostic>,
    locations: Vec<Option<DiagnosticLocation>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiagnosticLocation {
    pub(crate) path: PathBuf,
    pub(crate) position: Option<(usize, usize)>,
}

impl CompilationError {
    pub(crate) fn new(diagnostics: Vec<Diagnostic>) -> Self {
        debug_assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        );
        let locations = vec![None; diagnostics.len()];
        Self {
            diagnostics,
            locations,
        }
    }

    pub(crate) fn with_sources(mut self, sources: &SourceSet) -> Self {
        for (diagnostic, location) in self.diagnostics.iter().zip(&mut self.locations) {
            if location.is_some() {
                continue;
            }
            let source = diagnostic.primary.source;
            let Some(path) = sources.logical_path(source) else {
                continue;
            };
            *location = Some(DiagnosticLocation {
                path: path.to_path_buf(),
                position: sources.line_column(source, diagnostic.primary.bytes.start),
            });
        }
        self
    }

    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn location(&self, index: usize) -> Option<&DiagnosticLocation> {
        self.locations.get(index).and_then(Option::as_ref)
    }
}

impl fmt::Display for CompilationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let errors = self
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count();
        write!(
            formatter,
            "grammar compilation failed with {errors} error(s)"
        )
    }
}

impl std::error::Error for CompilationError {}
