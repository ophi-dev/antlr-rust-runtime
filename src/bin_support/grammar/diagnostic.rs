use std::fmt;

use super::frontend::SourceSpan;

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
}

impl CompilationError {
    pub(crate) fn new(diagnostics: Vec<Diagnostic>) -> Self {
        debug_assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        );
        Self { diagnostics }
    }

    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
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
