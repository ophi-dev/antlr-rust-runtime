use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::diagnostic::{CompilationError, Diagnostic, Severity};
use super::frontend::{SourceId, SourceSpan, parse_source, parse_source_recovering};
use super::model::{
    GrammarId, ImportEdge, LoadedGrammarSet, LookupKind, LookupRecord, ParsedGrammarUnit,
    VocabularyEdge, VocabularySource,
};
use super::source::SourceSet;
use super::syntax::parse_loader_unit;

#[derive(Clone, Debug, Default)]
pub(crate) struct LoadOptions {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) library_directories: Vec<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct LoadedSources {
    pub(crate) sources: SourceSet,
    pub(crate) grammars: LoadedGrammarSet,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisitState {
    Loading,
    Loaded,
}

struct Loader {
    options: LoadOptions,
    sources: SourceSet,
    grammars: Vec<ParsedGrammarUnit>,
    roots: Vec<GrammarId>,
    imports: Vec<ImportEdge>,
    vocabularies: Vec<VocabularyEdge>,
    lookups: Vec<LookupRecord>,
    by_name: BTreeMap<String, GrammarId>,
    grammar_for_source: BTreeMap<SourceId, GrammarId>,
    visits: BTreeMap<GrammarId, VisitState>,
    token_vocab_targets: BTreeSet<GrammarId>,
    resolved_vocabularies: BTreeSet<GrammarId>,
    stack: Vec<GrammarId>,
    load_order: Vec<GrammarId>,
    diagnostics: Vec<Diagnostic>,
}

pub(crate) fn load(options: LoadOptions) -> Result<LoadedSources, CompilationError> {
    let loaded = load_recovering(options);
    if loaded
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Err(CompilationError::new(loaded.diagnostics));
    }
    Ok(loaded)
}

pub(crate) fn load_recovering(options: LoadOptions) -> LoadedSources {
    let mut loader = Loader::new(options);
    loader.run();
    loader.finish()
}

impl Loader {
    fn new(options: LoadOptions) -> Self {
        Self {
            options,
            sources: SourceSet::default(),
            grammars: Vec::new(),
            roots: Vec::new(),
            imports: Vec::new(),
            vocabularies: Vec::new(),
            lookups: Vec::new(),
            by_name: BTreeMap::new(),
            grammar_for_source: BTreeMap::new(),
            visits: BTreeMap::new(),
            token_vocab_targets: BTreeSet::new(),
            resolved_vocabularies: BTreeSet::new(),
            stack: Vec::new(),
            load_order: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn run(&mut self) {
        if self.options.roots.is_empty() {
            self.diagnostics.push(Diagnostic::error(
                "G4L001",
                SourceSpan::empty(SourceId::new(0)),
                "at least one grammar root is required",
            ));
            return;
        }
        let roots = self.options.roots.clone();
        for path in roots {
            if let Some(grammar) = self.load_path(&path, Some(&path), false) {
                if !self.roots.contains(&grammar) {
                    self.roots.push(grammar);
                }
            }
        }
        let roots = self.roots.clone();
        for root in roots {
            self.visit(root);
        }
    }

    fn finish(self) -> LoadedSources {
        LoadedSources {
            sources: self.sources,
            grammars: LoadedGrammarSet {
                grammars: self.grammars,
                roots: self.roots,
                imports: self.imports,
                vocabularies: self.vocabularies,
                lookups: self.lookups,
                by_name: self.by_name,
                load_order: self.load_order,
            },
            diagnostics: self.diagnostics,
        }
    }

    fn load_path(
        &mut self,
        path: &Path,
        user_spelling: Option<&Path>,
        recover_syntax: bool,
    ) -> Option<GrammarId> {
        let canonical = match fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) => {
                self.diagnostics.push(Diagnostic::error(
                    "G4L002",
                    SourceSpan::empty(SourceId::new(0)),
                    format!("cannot open grammar {}: {error}", path.display()),
                ));
                return None;
            }
        };
        if let Some(source) = self.sources.id_for_canonical_path(&canonical) {
            return self.grammar_for_source.get(&source).copied();
        }
        let text = match fs::read_to_string(&canonical) {
            Ok(text) => text,
            Err(error) => {
                self.diagnostics.push(Diagnostic::error(
                    "G4L002",
                    SourceSpan::empty(SourceId::new(0)),
                    format!("cannot read grammar {}: {error}", canonical.display()),
                ));
                return None;
            }
        };
        let source = self.sources.next_id();
        let logical_path = user_spelling.unwrap_or(path).to_path_buf();
        let file = if recover_syntax {
            match parse_source_recovering(source, logical_path, text) {
                Ok(recovered) => {
                    self.diagnostics.extend(
                        recovered.diagnostics.into_iter().map(|syntax| {
                            Diagnostic::error(syntax.code, syntax.span, syntax.message)
                        }),
                    );
                    recovered.file
                }
                Err(error) => {
                    self.record_frontend_error(&error);
                    return None;
                }
            }
        } else {
            match parse_source(source, logical_path, text) {
                Ok(file) => file,
                Err(error) => {
                    self.record_frontend_error(&error);
                    return None;
                }
            }
        };
        let parsed = parse_loader_unit(&file);
        let grammar = GrammarId::new(
            u32::try_from(self.grammars.len()).expect("grammar count exceeds compact ID"),
        );
        self.check_file_name(&canonical, &parsed);
        self.check_duplicate_name(grammar, &parsed);
        self.sources
            .insert(canonical, file)
            .expect("canonical path checked before parsing");
        self.grammar_for_source.insert(source, grammar);
        self.grammars.push(parsed);
        Some(grammar)
    }

    fn record_frontend_error(&mut self, error: &super::frontend::FrontendError) {
        self.diagnostics
            .extend(error.diagnostics().iter().map(|syntax| {
                Diagnostic::error(syntax.code, syntax.span.clone(), syntax.message.clone())
            }));
    }

    fn check_file_name(&mut self, canonical: &Path, grammar: &ParsedGrammarUnit) {
        let file_name = canonical.file_stem().and_then(std::ffi::OsStr::to_str);
        if file_name == Some(grammar.header.name.value.as_str()) {
            return;
        }
        self.diagnostics.push(Diagnostic::error(
            "G4L003",
            grammar.header.name.span.clone(),
            format!(
                "grammar {} must be declared in a file named {}.g4",
                grammar.header.name.value, grammar.header.name.value
            ),
        ));
    }

    fn check_duplicate_name(&mut self, grammar: GrammarId, parsed: &ParsedGrammarUnit) {
        let name = parsed.header.name.value.clone();
        if let Some(previous) = self.by_name.get(&name).copied() {
            let previous_span = self.grammars[previous.index()].header.name.span.clone();
            self.diagnostics.push(
                Diagnostic::error(
                    "G4L004",
                    parsed.header.name.span.clone(),
                    format!("grammar name {name} is declared by more than one source"),
                )
                .with_related(previous_span, "first declaration is here"),
            );
        } else {
            self.by_name.insert(name, grammar);
        }
    }

    fn visit(&mut self, grammar: GrammarId) {
        match self.visits.get(&grammar) {
            Some(VisitState::Loaded) => {
                if self.should_resolve_token_vocab(grammar)
                    && self.resolved_vocabularies.insert(grammar)
                {
                    self.resolve_token_vocab(grammar);
                }
                return;
            }
            Some(VisitState::Loading) => {
                self.report_cycle(grammar);
                return;
            }
            None => {}
        }
        self.visits.insert(grammar, VisitState::Loading);
        self.stack.push(grammar);
        self.resolve_imports(grammar);
        if self.should_resolve_token_vocab(grammar) && self.resolved_vocabularies.insert(grammar) {
            self.resolve_token_vocab(grammar);
        }
        self.stack.pop();
        self.visits.insert(grammar, VisitState::Loaded);
        self.load_order.push(grammar);
    }

    fn resolve_imports(&mut self, importer: GrammarId) {
        let imports = self.grammars[importer.index()].imports.clone();
        for declaration in imports {
            if self.stack.iter().any(|grammar| {
                self.grammars[grammar.index()].header.name.value == declaration.grammar.value
            }) {
                continue;
            }
            let lookup = self.resolve_source_lookup(
                importer,
                &declaration.grammar.value,
                declaration.grammar.span.clone(),
                LookupKind::Import,
            );
            let lookup_index = self.lookups.len();
            self.lookups.push(lookup.record);
            let Some(path) = lookup.selected else {
                self.diagnostics.push(Diagnostic::error(
                    "G4L005",
                    declaration.grammar.span.clone(),
                    format!("cannot find imported grammar {}", declaration.grammar.value),
                ));
                continue;
            };
            let Some(imported) = self.load_path(&path, Some(&path), true) else {
                continue;
            };
            self.check_import(importer, imported, &declaration);
            let edge = ImportEdge {
                id: u32::try_from(self.imports.len()).expect("import edge count exceeds u32"),
                importer,
                imported,
                declaration,
                lookup: lookup_index,
            };
            self.imports.push(edge);
            self.visit(imported);
        }
    }

    fn resolve_token_vocab(&mut self, importer: GrammarId) {
        let Some(declaration) = self.grammars[importer.index()].token_vocab.clone() else {
            return;
        };
        if let Some(producer) = self.by_name.get(&declaration.value).copied() {
            self.bind_source_vocab(importer, producer, declaration, None);
            return;
        }
        let source_lookup = self.resolve_source_lookup(
            importer,
            &declaration.value,
            declaration.span.clone(),
            LookupKind::TokenVocabSource,
        );
        let source_lookup_index = self.lookups.len();
        let source_path = source_lookup.selected.clone();
        self.lookups.push(source_lookup.record);
        if let Some(path) = source_path {
            if let Some(producer) = self.load_path(&path, Some(&path), false) {
                self.bind_source_vocab(importer, producer, declaration, Some(source_lookup_index));
                return;
            }
        }
        self.bind_tokens_file(importer, declaration);
    }

    fn bind_source_vocab(
        &mut self,
        importer: GrammarId,
        producer: GrammarId,
        declaration: super::model::Authored<String>,
        lookup: Option<usize>,
    ) {
        let producer_kind = self.grammars[producer.index()].header.kind;
        if producer_kind == super::model::GrammarKind::Parser {
            self.diagnostics.push(Diagnostic::error(
                "G4L006",
                declaration.span,
                format!(
                    "parser grammar {} cannot provide token vocabulary",
                    declaration.value
                ),
            ));
            return;
        }
        let lookup = lookup.unwrap_or_else(|| {
            let index = self.lookups.len();
            let selected = self
                .sources
                .canonical_path(self.grammars[producer.index()].source)
                .map(Path::to_path_buf);
            self.lookups.push(LookupRecord {
                kind: LookupKind::TokenVocabSource,
                requested: declaration.value.clone(),
                selected,
                shadowed: Vec::new(),
                at: declaration.span.clone(),
            });
            index
        });
        self.vocabularies.push(VocabularyEdge {
            importer,
            source: VocabularySource::Grammar(producer),
            declaration,
            lookup,
        });
        self.token_vocab_targets.insert(producer);
        self.visit(producer);
    }

    fn should_resolve_token_vocab(&self, grammar: GrammarId) -> bool {
        self.roots.contains(&grammar) || self.token_vocab_targets.contains(&grammar)
    }

    fn bind_tokens_file(
        &mut self,
        importer: GrammarId,
        declaration: super::model::Authored<String>,
    ) {
        let lookup =
            self.resolve_tokens_lookup(importer, &declaration.value, declaration.span.clone());
        let lookup_index = self.lookups.len();
        let selected = lookup.selected.clone();
        self.lookups.push(lookup.record);
        if let Some(path) = selected {
            self.vocabularies.push(VocabularyEdge {
                importer,
                source: VocabularySource::TokensFile(path),
                declaration,
                lookup: lookup_index,
            });
        } else {
            self.diagnostics.push(Diagnostic::error(
                "G4L007",
                declaration.span,
                format!("cannot find token vocabulary {}", declaration.value),
            ));
        }
    }

    fn check_import(
        &mut self,
        importer: GrammarId,
        imported: GrammarId,
        declaration: &super::model::ImportDecl,
    ) {
        let parent = &self.grammars[importer.index()];
        let child = &self.grammars[imported.index()];
        if !parent.header.kind.accepts_import(child.header.kind) {
            self.diagnostics.push(
                Diagnostic::error(
                    "G4L008",
                    declaration.grammar.span.clone(),
                    format!(
                        "{} grammar {} cannot import {:?} grammar {}",
                        kind_name(parent.header.kind),
                        parent.header.name.value,
                        child.header.kind,
                        child.header.name.value
                    ),
                )
                .with_related(
                    child.header.declaration_span.clone(),
                    "imported grammar is declared here",
                ),
            );
        }
    }

    fn resolve_source_lookup(
        &self,
        importer: GrammarId,
        name: &str,
        at: SourceSpan,
        kind: LookupKind,
    ) -> Lookup {
        let mut candidates = Vec::new();
        if let Some(parent) = self
            .sources
            .canonical_path(self.grammars[importer.index()].source)
            .and_then(Path::parent)
        {
            candidates.push(parent.join(format!("{name}.g4")));
        }
        candidates.extend(
            self.options
                .library_directories
                .iter()
                .map(|directory| directory.join(format!("{name}.g4"))),
        );
        select_lookup(kind, name, at, candidates)
    }

    fn resolve_tokens_lookup(&self, importer: GrammarId, name: &str, at: SourceSpan) -> Lookup {
        let mut candidates = self
            .options
            .library_directories
            .iter()
            .map(|directory| directory.join(format!("{name}.tokens")))
            .collect::<Vec<_>>();
        if let Some(parent) = self
            .sources
            .canonical_path(self.grammars[importer.index()].source)
            .and_then(Path::parent)
        {
            candidates.push(parent.join(format!("{name}.tokens")));
        }
        select_lookup(LookupKind::TokenVocabFile, name, at, candidates)
    }

    fn report_cycle(&mut self, repeated: GrammarId) {
        let start = self
            .stack
            .iter()
            .position(|grammar| *grammar == repeated)
            .unwrap_or(0);
        let mut cycle = self.stack[start..]
            .iter()
            .map(|grammar| self.grammars[grammar.index()].header.name.value.as_str())
            .collect::<Vec<_>>();
        cycle.push(self.grammars[repeated.index()].header.name.value.as_str());
        self.diagnostics.push(Diagnostic::error(
            "G4L009",
            self.grammars[repeated.index()].header.name.span.clone(),
            format!("grammar dependency cycle: {}", cycle.join(" -> ")),
        ));
    }
}

struct Lookup {
    selected: Option<PathBuf>,
    record: LookupRecord,
}

fn select_lookup(
    kind: LookupKind,
    requested: &str,
    at: SourceSpan,
    candidates: Vec<PathBuf>,
) -> Lookup {
    let mut existing = Vec::new();
    let mut seen = BTreeSet::new();
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        let canonical = fs::canonicalize(&candidate).unwrap_or(candidate);
        if seen.insert(canonical.clone()) {
            existing.push(canonical);
        }
    }
    let selected = existing.first().cloned();
    let shadowed = existing.into_iter().skip(1).collect();
    Lookup {
        selected: selected.clone(),
        record: LookupRecord {
            kind,
            requested: requested.to_owned(),
            selected,
            shadowed,
            at,
        },
    }
}

const fn kind_name(kind: super::model::GrammarKind) -> &'static str {
    match kind {
        super::model::GrammarKind::Lexer => "lexer",
        super::model::GrammarKind::Parser => "parser",
        super::model::GrammarKind::Combined => "combined",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn loads_diamond_import_once_and_preserves_both_edges() {
        let fixture = Fixture::new("diamond");
        fixture.write("Root.g4", "grammar Root; import Left, Right; root: shared;");
        fixture.write(
            "Left.g4",
            "parser grammar Left; import Shared; left: shared;",
        );
        fixture.write(
            "Right.g4",
            "parser grammar Right; import Shared; right: shared;",
        );
        fixture.write("Shared.g4", "parser grammar Shared; shared: 'x';");

        let loaded = load(LoadOptions {
            roots: vec![fixture.path("Root.g4")],
            library_directories: Vec::new(),
        })
        .expect("diamond should load");

        assert_eq!(loaded.sources.len(), 4);
        assert_eq!(loaded.grammars.imports.len(), 4);
        let shared = loaded.grammars.by_name["Shared"];
        assert_eq!(
            loaded
                .grammars
                .imports
                .iter()
                .filter(|edge| edge.imported == shared)
                .count(),
            2
        );
    }

    #[test]
    fn ignores_recursive_import_edges_like_java() {
        let fixture = Fixture::new("cycle");
        fixture.write("A.g4", "parser grammar A; import B; a: b;");
        fixture.write("B.g4", "parser grammar B; import C; b: c;");
        fixture.write("C.g4", "parser grammar C; import A; c: a;");
        let loaded = load(LoadOptions {
            roots: vec![fixture.path("A.g4")],
            library_directories: Vec::new(),
        })
        .expect("recursive import should be ignored");
        assert_eq!(loaded.grammars.imports.len(), 2);
        assert_eq!(
            loaded
                .grammars
                .load_order
                .iter()
                .map(|grammar| {
                    loaded.grammars.grammars[grammar.index()]
                        .header
                        .name
                        .value
                        .as_str()
                })
                .collect::<Vec<_>>(),
            ["C", "B", "A"],
        );
    }

    #[test]
    fn ordered_libraries_select_the_first_and_record_shadows() {
        let fixture = Fixture::new("libraries");
        let first = fixture.path("first");
        let second = fixture.path("second");
        fs::create_dir_all(&first).expect("first library");
        fs::create_dir_all(&second).expect("second library");
        fixture.write(
            "Root.g4",
            "parser grammar Root; import Shared; root: shared;",
        );
        write_file(
            &first.join("Shared.g4"),
            "parser grammar Shared; shared: 'a';",
        );
        write_file(
            &second.join("Shared.g4"),
            "parser grammar Shared; shared: 'b';",
        );

        let loaded = load(LoadOptions {
            roots: vec![fixture.path("Root.g4")],
            library_directories: vec![first.clone(), second],
        })
        .expect("ordered libraries should load");
        let lookup = loaded
            .grammars
            .lookups
            .iter()
            .find(|lookup| lookup.kind == LookupKind::Import)
            .expect("import lookup");
        let selected =
            fs::canonicalize(first.join("Shared.g4")).expect("selected import is canonicalizable");
        assert_eq!(lookup.selected.as_deref(), Some(selected.as_path()));
        assert_eq!(lookup.shadowed.len(), 1);
    }

    mod upstream_topological_sort {
        use super::*;

        #[test]
        fn fairly_large_graph_matches_java() {
            let fixture = Fixture::new("topological-fairly-large");
            fixture.write("C.g4", "parser grammar C; import F, G, A, B;");
            fixture.write("A.g4", "parser grammar A; import D, E;");
            fixture.write("B.g4", "parser grammar B; import E;");
            fixture.write("D.g4", "parser grammar D; import E, F;");
            fixture.write("E.g4", "parser grammar E; import F;");
            fixture.write("F.g4", "parser grammar F; import H;");
            fixture.write("G.g4", "parser grammar G;");
            fixture.write("H.g4", "parser grammar H;");

            let loaded = load_fixture(&fixture, &["C.g4"]).expect("graph should load");

            assert_eq!(
                load_order_names(&loaded.grammars),
                ["H", "F", "G", "E", "D", "A", "B", "C"],
            );
        }

        #[test]
        fn cyclic_graph_matches_java() {
            let fixture = Fixture::new("topological-cycle");
            fixture.write("A.g4", "parser grammar A; import B;");
            fixture.write("B.g4", "parser grammar B; import C;");
            fixture.write("C.g4", "parser grammar C; import A, D;");
            fixture.write("D.g4", "parser grammar D;");
            let mut loader = Loader::new(load_options(&fixture, &["A.g4"]));

            loader.run();

            assert_eq!(
                load_order_names_from_parts(&loader.grammars, &loader.load_order),
                ["D", "C", "B", "A"],
            );
            assert!(loader.diagnostics.is_empty());
        }

        #[test]
        fn repeated_edges_match_java() {
            let fixture = Fixture::new("topological-repeated-edges");
            fixture.write("A.g4", "parser grammar A; import B, B;");
            fixture.write("B.g4", "parser grammar B; import C;");
            fixture.write("C.g4", "parser grammar C; import D;");
            fixture.write("D.g4", "parser grammar D;");

            let loaded = load_fixture(&fixture, &["A.g4"]).expect("graph should load");

            assert_eq!(load_order_names(&loaded.grammars), ["D", "C", "B", "A"],);
        }

        #[test]
        fn simple_token_dependence_matches_java() {
            let fixture = Fixture::new("topological-token-dependence");
            fixture.write(
                "Java.g4",
                "grammar Java; options { tokenVocab=MyJava; } id: ID; ID: 'a';",
            );
            fixture.write("Def.g4", "parser grammar Def; options { tokenVocab=Java; }");
            fixture.write("Ref.g4", "parser grammar Ref; options { tokenVocab=Java; }");
            fixture.write("MyJava.tokens", "ID=1\n");

            let loaded = load_fixture(&fixture, &["Java.g4", "Def.g4", "Ref.g4"])
                .expect("token dependency graph should load");

            assert_eq!(load_order_names(&loaded.grammars), ["Java", "Def", "Ref"],);
            assert_vocabulary_source(
                &loaded.grammars,
                "Java",
                ExpectedVocabularySource::TokensFile("MyJava.tokens"),
            );
            assert_vocabulary_source(
                &loaded.grammars,
                "Def",
                ExpectedVocabularySource::Grammar("Java"),
            );
            assert_vocabulary_source(
                &loaded.grammars,
                "Ref",
                ExpectedVocabularySource::Grammar("Java"),
            );
        }

        #[test]
        fn parser_lexer_combo_matches_java() {
            let fixture = Fixture::new("topological-parser-lexer");
            fixture.write("JavaLexer.g4", "lexer grammar JavaLexer; ID: 'a';");
            fixture.write(
                "JavaParser.g4",
                "parser grammar JavaParser; options { tokenVocab=JavaLexer; }",
            );
            fixture.write(
                "Def.g4",
                "parser grammar Def; options { tokenVocab=JavaLexer; }",
            );
            fixture.write(
                "Ref.g4",
                "parser grammar Ref; options { tokenVocab=JavaLexer; }",
            );

            let loaded = load_fixture(&fixture, &["JavaParser.g4", "Def.g4", "Ref.g4"])
                .expect("parser/lexer dependency graph should load");

            assert_eq!(
                load_order_names(&loaded.grammars),
                ["JavaLexer", "JavaParser", "Def", "Ref"],
            );
            for consumer in ["JavaParser", "Def", "Ref"] {
                assert_vocabulary_source(
                    &loaded.grammars,
                    consumer,
                    ExpectedVocabularySource::Grammar("JavaLexer"),
                );
            }
        }

        fn load_fixture(
            fixture: &Fixture,
            roots: &[&str],
        ) -> Result<LoadedSources, CompilationError> {
            load(load_options(fixture, roots))
        }

        fn load_options(fixture: &Fixture, roots: &[&str]) -> LoadOptions {
            LoadOptions {
                roots: roots.iter().map(|root| fixture.path(root)).collect(),
                library_directories: Vec::new(),
            }
        }

        fn load_order_names(grammars: &LoadedGrammarSet) -> Vec<&str> {
            load_order_names_from_parts(&grammars.grammars, &grammars.load_order)
        }

        fn load_order_names_from_parts<'a>(
            grammars: &'a [ParsedGrammarUnit],
            order: &[GrammarId],
        ) -> Vec<&'a str> {
            order
                .iter()
                .map(|grammar| grammars[grammar.index()].header.name.value.as_str())
                .collect()
        }

        #[derive(Clone, Copy)]
        enum ExpectedVocabularySource<'a> {
            Grammar(&'a str),
            TokensFile(&'a str),
        }

        fn assert_vocabulary_source(
            grammars: &LoadedGrammarSet,
            consumer: &str,
            expected: ExpectedVocabularySource<'_>,
        ) {
            let consumer = grammars.by_name[consumer];
            let source = &grammars
                .vocabularies
                .iter()
                .find(|edge| edge.importer == consumer)
                .expect("consumer should have a vocabulary edge")
                .source;
            match (source, expected) {
                (
                    VocabularySource::Grammar(actual),
                    ExpectedVocabularySource::Grammar(expected),
                ) => {
                    assert_eq!(grammars.grammar(*actual).header.name.value, expected);
                }
                (
                    VocabularySource::TokensFile(actual),
                    ExpectedVocabularySource::TokensFile(expected),
                ) => {
                    assert_eq!(
                        actual.file_name().and_then(std::ffi::OsStr::to_str),
                        Some(expected),
                    );
                }
                _ => panic!("vocabulary source kind differs"),
            }
        }
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "antlr-rust-phase-b-{name}-{}-{serial}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("fixture directory");
            Self { root }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }

        fn write(&self, relative: &str, text: &str) {
            write_file(&self.path(relative), text);
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write_file(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent");
        }
        let mut file = fs::File::create(path).expect("fixture file");
        file.write_all(text.as_bytes()).expect("fixture contents");
    }
}
