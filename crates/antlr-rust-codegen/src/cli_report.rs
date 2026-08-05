use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::ops::Range;

use miette::{LabeledSpan, NamedSource, SourceCode};

use crate::error::{
    Diagnostic as CodegenDiagnostic, Error as CodegenError, ErrorKind, Severity as CodegenSeverity,
};

pub(crate) fn codegen_error(error: CodegenError) -> miette::Report {
    miette::Report::new(CodegenErrorReport::new(error))
}

pub(crate) fn install_handler_from_env() {
    let Some(width) = std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| *width > 0)
    else {
        return;
    };
    let _ = miette::set_hook(Box::new(move |_| {
        Box::new(miette::MietteHandlerOpts::new().width(width).build())
    }));
}

#[derive(Debug)]
struct CodegenErrorReport {
    error: CodegenError,
    message: String,
    code: &'static str,
    related: Vec<CodegenDiagnosticReport>,
}

impl CodegenErrorReport {
    fn new(error: CodegenError) -> Self {
        let message = if error.kind() == ErrorKind::Compilation {
            let error_count = error
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.severity() == CodegenSeverity::Error)
                .count();
            format!("grammar compilation failed with {error_count} error(s)")
        } else {
            error.to_string()
        };
        let code = match error.kind() {
            ErrorKind::Configuration => "antlr4_rust_codegen::configuration",
            ErrorKind::Compilation => "antlr4_rust_codegen::compilation",
            ErrorKind::Generation => "antlr4_rust_codegen::generation",
            ErrorKind::Filesystem => "antlr4_rust_codegen::filesystem",
        };
        let related = error
            .diagnostics()
            .iter()
            .map(CodegenDiagnosticReport::new)
            .collect();
        Self {
            error,
            message,
            code,
            related,
        }
    }
}

impl fmt::Display for CodegenErrorReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for CodegenErrorReport {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        StdError::source(&self.error).filter(|source| source.to_string() != self.message)
    }
}

impl miette::Diagnostic for CodegenErrorReport {
    fn code(&self) -> Option<Box<dyn fmt::Display + '_>> {
        Some(Box::new(self.code))
    }

    fn related(&self) -> Option<Box<dyn Iterator<Item = &dyn miette::Diagnostic> + '_>> {
        if self.related.is_empty() {
            return None;
        }
        let related: Box<dyn Iterator<Item = &dyn miette::Diagnostic>> = Box::new(
            self.related
                .iter()
                .map(|diagnostic| -> &dyn miette::Diagnostic { diagnostic }),
        );
        Some(related)
    }
}

#[derive(Debug)]
struct CodegenDiagnosticReport {
    code: &'static str,
    severity: miette::Severity,
    message: String,
    source: Option<NamedSource<String>>,
    span: Option<Range<usize>>,
}

impl CodegenDiagnosticReport {
    fn new(diagnostic: &CodegenDiagnostic) -> Self {
        let span = diagnostic.byte_span();
        let source = span.as_ref().and_then(|span| {
            let contents = fs::read_to_string(diagnostic.path()).ok()?;
            (span.end <= contents.len()).then(|| {
                NamedSource::new(diagnostic.path().display().to_string(), contents)
                    .with_language("ANTLR")
            })
        });
        let message = if source.is_some() {
            diagnostic.message().to_owned()
        } else {
            let position = diagnostic
                .line()
                .zip(diagnostic.column())
                .map_or_else(String::new, |(line, column)| format!(":{line}:{column}"));
            format!(
                "{}{position}: {}",
                diagnostic.path().display(),
                diagnostic.message()
            )
        };
        let severity = match diagnostic.severity() {
            CodegenSeverity::Warning => miette::Severity::Warning,
            CodegenSeverity::Error => miette::Severity::Error,
        };
        let span = source.is_some().then_some(span).flatten();
        Self {
            code: diagnostic.code(),
            severity,
            message,
            source,
            span,
        }
    }
}

impl fmt::Display for CodegenDiagnosticReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for CodegenDiagnosticReport {}

impl miette::Diagnostic for CodegenDiagnosticReport {
    fn code(&self) -> Option<Box<dyn fmt::Display + '_>> {
        Some(Box::new(self.code))
    }

    fn severity(&self) -> Option<miette::Severity> {
        Some(self.severity)
    }

    fn source_code(&self) -> Option<&dyn SourceCode> {
        self.source
            .as_ref()
            .map(|source| -> &dyn SourceCode { source })
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        self.span.as_ref().map(|span| {
            let labels: Box<dyn Iterator<Item = LabeledSpan>> =
                Box::new(std::iter::once(LabeledSpan::underline(span.clone())));
            labels
        })
    }
}
