use std::collections::BTreeMap;

use super::atn::{CompiledLexer, CompiledParser, compile_lexer, compile_parser};
use super::diagnostic::{CompilationError, Diagnostic};
use super::loader::{LoadOptions, LoadedSources, load_recovering};
use super::model::{GrammarId, GrammarKind};
use super::semantics::{SemanticGrammarSet, analyze};
use super::source::SourceSet;
use super::transform::{RootOutputs, TransformRegistry, TransformReport, integrate_loaded};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompiledRoot {
    pub(crate) source: GrammarId,
    pub(crate) lexer: Option<GrammarId>,
    pub(crate) parser: Option<GrammarId>,
}

#[derive(Debug)]
pub(crate) struct Compilation {
    pub(crate) sources: SourceSet,
    pub(crate) roots: Vec<CompiledRoot>,
    pub(crate) lexers: BTreeMap<GrammarId, CompiledLexer>,
    pub(crate) parsers: BTreeMap<GrammarId, CompiledParser>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) transform_report: TransformReport,
}

impl Compilation {
    pub(crate) fn lexer(&self, grammar: GrammarId) -> Option<&CompiledLexer> {
        self.lexers.get(&grammar)
    }

    pub(crate) fn parser(&self, grammar: GrammarId) -> Option<&CompiledParser> {
        self.parsers.get(&grammar)
    }

    pub(crate) fn lexer_named(&self, name: &str) -> Option<&CompiledLexer> {
        self.lexers
            .values()
            .find(|compiled| compiled.semantic.unit.name == name)
    }

    pub(crate) fn parser_named(&self, name: &str) -> Option<&CompiledParser> {
        self.parsers
            .values()
            .find(|compiled| compiled.semantic.unit.name == name)
    }
}

pub(crate) fn compile(options: LoadOptions) -> Result<Compilation, CompilationError> {
    compile_with_transforms(options, &TransformRegistry::default())
}

fn compile_with_transforms(
    options: LoadOptions,
    transforms: &TransformRegistry,
) -> Result<Compilation, CompilationError> {
    let loaded = load_recovering(options);
    let root_order = loaded.grammars.roots.clone();
    let mut integrated = integrate_loaded(&loaded)?;
    let transform_report = transforms
        .run(&mut integrated.grammar, false)
        .map_err(|diagnostic| CompilationError::new(vec![diagnostic]))?;
    let semantics = analyze(&loaded.sources, integrated)?;
    let LoadedSources { sources, .. } = loaded;
    compile_semantics(sources, root_order, semantics, transform_report)
}

fn compile_semantics(
    sources: SourceSet,
    root_order: Vec<GrammarId>,
    semantics: SemanticGrammarSet,
    transform_report: TransformReport,
) -> Result<Compilation, CompilationError> {
    let SemanticGrammarSet {
        grammars,
        roots,
        diagnostics: mut all_diagnostics,
        provenance,
        ..
    } = semantics;
    let roots = root_order
        .into_iter()
        .map(|source| {
            let RootOutputs { lexer, parser } = roots[&source];
            CompiledRoot {
                source,
                lexer,
                parser,
            }
        })
        .collect();
    let mut lexers = BTreeMap::new();
    let mut parsers = BTreeMap::new();
    for grammar in grammars {
        let grammar_id = grammar.unit.id;
        match grammar.unit.kind {
            GrammarKind::Lexer => {
                let compiled = compile_lexer(grammar, provenance.clone())?;
                all_diagnostics.extend(compiled.analysis.diagnostics.iter().cloned());
                lexers.insert(grammar_id, compiled);
            }
            GrammarKind::Parser => {
                let compiled = compile_parser(grammar, provenance.clone())?;
                all_diagnostics.extend(compiled.analysis.diagnostics.iter().cloned());
                parsers.insert(grammar_id, compiled);
            }
            GrammarKind::Combined => {
                unreachable!("combined grammars are split before semantic analysis")
            }
        }
    }
    Ok(Compilation {
        sources,
        roots,
        lexers,
        parsers,
        diagnostics: all_diagnostics,
        transform_report,
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::diagnostic::Severity;
    use super::*;

    #[test]
    fn combined_root_owns_shared_direct_artifacts() {
        let compilation = compile(LoadOptions {
            roots: vec![fixture("vscode-sentences").join("sentences.g4")],
            library_directories: Vec::new(),
        })
        .expect("combined fixture should compile");

        let [root] = compilation.roots.as_slice() else {
            panic!("one requested root should produce one compiled root");
        };
        let lexer = compilation
            .lexer(root.lexer.expect("combined root has a lexer"))
            .expect("root lexer artifact exists");
        let parser = compilation
            .parser(root.parser.expect("combined root has a parser"))
            .expect("root parser artifact exists");
        assert_eq!(lexer.semantic.unit.name, "sentencesLexer");
        assert_eq!(parser.semantic.unit.name, "sentencesParser");
        assert_eq!(compilation.sources.len(), 1);
        assert!(compilation.transform_report.entries.is_empty());
    }

    #[test]
    fn missing_token_vocab_matches_java_diagnostic_site() {
        let error = compile_fixture("vscode-split-errors", &["TLexer2.g4", "TParser2.g4"])
            .expect_err("missing token vocabulary must be fatal");
        assert_diagnostic(
            &error,
            "G4L007",
            Severity::Error,
            "TLexer2.g4",
            Some((4, 14)),
            "cannot find token vocabulary nonexisting",
        );
    }

    #[test]
    fn implicit_token_warning_precedes_unknown_channel_error() {
        let error = compile_fixture("vscode-diagnostics", &["t.g4"])
            .expect_err("unknown channel must be fatal");
        assert_diagnostic(
            &error,
            "G4S030",
            Severity::Warning,
            "t.g4",
            Some((3, 3)),
            "implicit definition of token ZZ in parser",
        );
        assert_diagnostic(
            &error,
            "G4S053",
            Severity::Error,
            "t.g4",
            Some((8, 18)),
            "BLAH is not a recognized channel",
        );
    }

    #[test]
    fn indirect_left_recursion_matches_java_cycle_members() {
        let error = compile_fixture("vscode-indirect-left-recursion", &["t2.g4"])
            .expect_err("mutual left recursion must be fatal");
        assert_diagnostic(
            &error,
            "G4A005",
            Severity::Error,
            "t2.g4",
            None,
            "mutually left-recursive rules: [a, c, b]",
        );
    }

    fn compile_fixture(
        fixture_name: &str,
        roots: &[&str],
    ) -> Result<Compilation, CompilationError> {
        let directory = fixture(fixture_name);
        compile(LoadOptions {
            roots: roots.iter().map(|root| directory.join(root)).collect(),
            library_directories: Vec::new(),
        })
    }

    fn assert_diagnostic(
        error: &CompilationError,
        code: &str,
        severity: Severity,
        source_name: &str,
        position: Option<(usize, usize)>,
        message: &str,
    ) {
        let diagnostic = error
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code == code)
            .unwrap_or_else(|| panic!("missing {code} diagnostic: {error:#?}"));
        assert_eq!(diagnostic.severity, severity);
        assert_eq!(diagnostic.message, message);
        if let Some((line, column)) = position {
            let text = std::fs::read_to_string(
                fixture_directory_for_source(source_name)
                    .unwrap_or_else(|| panic!("unknown fixture source {source_name}")),
            )
            .expect("fixture source should be readable");
            assert_eq!(
                diagnostic.primary.bytes.start,
                byte_offset(&text, line, column),
                "{source_name}:{line}:{column}",
            );
        }
    }

    fn fixture_directory_for_source(source_name: &str) -> Option<PathBuf> {
        [
            "vscode-split-errors",
            "vscode-diagnostics",
            "vscode-indirect-left-recursion",
        ]
        .into_iter()
        .map(|fixture_name| fixture(fixture_name).join(source_name))
        .find(|path| path.is_file())
    }

    fn byte_offset(text: &str, line: usize, column: usize) -> u32 {
        let line_start = text
            .split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::len)
            .sum::<usize>();
        let byte_column = text[line_start..]
            .chars()
            .take(column)
            .map(char::len_utf8)
            .sum::<usize>();
        u32::try_from(line_start + byte_column).expect("fixture offset exceeds u32")
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/codegen-direct/fixtures")
            .join(name)
    }
}
