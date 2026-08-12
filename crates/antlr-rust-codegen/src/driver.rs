// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use crate::config::CompilerConfig;
use crate::generator::prelude::*;
use crate::lexer::{LexerRenderModel, render_lexer_model};
use crate::optimization::OptimizationPlan;
use crate::parser::{ParserRenderOptions, render_parser_with_decision_report};
use crate::rust_support::{self, PreparedRustSupport};
use crate::semantics::{
    DecisionReportGrammar, GrammarOptionEntry, SemUnknownPolicy, SemanticsEntry,
    collect_lexer_semantics, collect_parser_semantics_for_mode, collect_structural_grammar_options,
    enforce_require_full_options, enforce_require_full_semantics, enforce_sem_unknown,
    grammar_option_warning_messages, render_decisions_manifest, render_semantics_manifest,
};
use crate::test_rig::{
    MAIN_PATH as TEST_RIG_MAIN_PATH, TestRigLexer, TestRigParser, render_test_rig,
};

pub(crate) fn generate(
    args: &CompilerConfig,
    mut report: impl FnMut(&str) -> io::Result<()>,
) -> Result<Generation, Error> {
    let prepared_support = rust_support::prepare(args)?;
    let optimizations = OptimizationPlan::from_compiler_config(args)?;
    let action_reference_parser: grammar::action::ActionReferenceParser =
        if args.embedded_actions || prepared_support.has_bundles() {
            embedded::action_references
        } else {
            grammar::action::action_references
        };
    let compilation = grammar::compiler::compile_with_action_reference_parser(
        LoadOptions {
            roots: prepared_support.roots().to_vec(),
            library_directories: prepared_support.library_directories().to_vec(),
        },
        optimizations.transforms(),
        optimizations.report_only(),
        optimizations.entry_rules(),
        action_reference_parser,
    )
    .map_err(|error| compilation_error(&error, &args.roots, &prepared_support))?;
    let diagnostics = compilation_diagnostics(&compilation, &prepared_support);
    let mut warnings = diagnostics
        .iter()
        .map(render_diagnostic)
        .collect::<Vec<_>>();
    for warning in &warnings {
        report(warning).map_err(Error::generation)?;
    }
    let optimization_messages = optimizations.diagnostic_messages(&compilation);
    for message in &optimization_messages {
        report(message).map_err(Error::generation)?;
    }
    warnings.extend(optimization_messages);
    let mut inputs = compilation
        .input_paths()
        .map(|path| prepared_support.original_path(path))
        .collect::<Vec<_>>();
    inputs.extend(prepared_support.additional_inputs());
    if let Some(path) = &args.sem_patterns_path {
        let path = fs::canonicalize(path).map_err(Error::generation)?;
        if !inputs.contains(&path) {
            inputs.push(path);
        }
    }
    let mut seen_inputs = BTreeSet::new();
    inputs.retain(|path| seen_inputs.insert(path.clone()));
    let optimization_manifest = optimizations.render_manifest(&compilation);
    if optimizations.report_only() {
        let mut artifacts = GeneratedArtifacts::default();
        artifacts.insert(
            "optimizations.json",
            optimization_manifest.expect("report mode enables the optimization manifest"),
        )?;
        let outputs = artifacts.write_to(&args.out_dir)?;
        return Ok(Generation::new(inputs, outputs, warnings, diagnostics));
    }

    let mut grammar_options = Vec::new();
    let mut manifest_grammars: Vec<(&'static str, String, Vec<SemanticsEntry>)> = Vec::new();
    let mut decision_report_grammars: Vec<DecisionReportGrammar> = Vec::new();
    let mut rendered_modules = BTreeMap::<PathBuf, String>::new();
    let mut emitted_lexers = BTreeSet::new();
    let mut emitted_parsers = BTreeSet::new();
    let mut test_rig_lexers = Vec::new();
    let mut test_rig_parsers = Vec::new();

    for root in &compilation.roots {
        if let Some(grammar) = root.lexer
            && emitted_lexers.insert(grammar)
        {
            let compiled = compilation
                .lexer(grammar)
                .expect("compiled root lexer artifact exists");
            let data = LexerCodegenData::from_compiled(compiled, &compilation.sources);
            let support_enabled =
                source_uses_rust_support(&data, &compilation.sources, &prepared_support);
            let option_hooks = option_hooks(args, &data, support_enabled);
            let options = collect_structural_grammar_options(&data, &option_hooks)?;
            if support_enabled {
                enforce_require_full_options(true, &options)?;
            }
            grammar_options.extend(options);
            let embedded_actions = args.embedded_actions || support_enabled;
            let sem_unknown = if support_enabled {
                SemUnknownPolicy::Error
            } else {
                args.sem_unknown
            };
            let require_full_semantics = args.require_full_semantics || support_enabled;
            let entries = collect_lexer_semantics(
                &data,
                embedded_actions,
                args.allow_unsupported_lexer_actions,
                sem_unknown,
                &args.sem_patterns,
            )?;
            enforce_sem_unknown(sem_unknown, &entries)?;
            enforce_require_full_semantics(require_full_semantics, &entries)?;
            let grammar_name = compiled.semantic.recognizer.name.clone();
            let render_model = LexerRenderModel::new(
                &grammar_name,
                &data,
                args.allow_unsupported_lexer_actions,
                sem_unknown,
                &args.sem_patterns,
                embedded_actions,
            );
            let mut module = render_lexer_model(&render_model)?;
            prepared_support.decorate_rendered_module(&mut module);
            insert_rendered_module(&mut rendered_modules, &grammar_name, module)?;
            if args.test_rig.is_some() {
                test_rig_lexers.push(TestRigLexer::new(grammar_name.clone()));
            }
            manifest_grammars.push(("lexer", grammar_name, entries));
        }

        if let Some(grammar) = root.parser
            && emitted_parsers.insert(grammar)
        {
            let compiled = compilation
                .parser(grammar)
                .expect("compiled root parser artifact exists");
            let data = ParserCodegenData::from_compiled(compiled, &compilation.sources);
            let support_enabled =
                source_uses_rust_support(&data, &compilation.sources, &prepared_support);
            let option_hooks = option_hooks(args, &data, support_enabled);
            let options = collect_structural_grammar_options(&data, &option_hooks)?;
            if support_enabled {
                enforce_require_full_options(true, &options)?;
            }
            grammar_options.extend(options);
            let embedded_actions = args.embedded_actions || support_enabled;
            let sem_unknown = if support_enabled {
                SemUnknownPolicy::Error
            } else {
                args.sem_unknown
            };
            let require_full_semantics = args.require_full_semantics || support_enabled;
            let entries = collect_parser_semantics_for_mode(
                &data,
                embedded_actions,
                sem_unknown,
                &args.sem_patterns,
            )?;
            enforce_sem_unknown(sem_unknown, &entries)?;
            enforce_require_full_semantics(require_full_semantics, &entries)?;
            let grammar_name = compiled.semantic.recognizer.name.clone();
            let (mut module, decision_report_rows) = render_parser_with_decision_report(
                &grammar_name,
                &data,
                ParserRenderOptions {
                    require_generated_parser: args.require_generated_parser || support_enabled,
                    embedded: embedded_actions,
                    generate_listener: args.generate_listener,
                    generate_visitor: args.generate_visitor,
                    sem_unknown,
                    patterns: Some(&args.sem_patterns),
                    fixed_lookahead: args.fixed_lookahead,
                },
            )?;
            prepared_support.decorate_rendered_module(&mut module);
            insert_rendered_module(&mut rendered_modules, &grammar_name, module)?;
            if args.test_rig.is_some() {
                test_rig_parsers.push(TestRigParser::new(
                    grammar_name.clone(),
                    data.rule_names.clone(),
                ));
            }
            decision_report_grammars.push(DecisionReportGrammar {
                name: grammar_name.clone(),
                rule_names: data.rule_names.clone(),
                rows: decision_report_rows,
            });
            manifest_grammars.push(("parser", grammar_name, entries));
        }
    }

    deduplicate_grammar_options(&mut grammar_options);
    let option_warnings = grammar_option_warning_messages(&grammar_options);
    for warning in &option_warnings {
        report(warning).map_err(Error::generation)?;
    }
    warnings.extend(option_warnings);
    enforce_require_full_options(args.require_full_semantics, &grammar_options)?;
    let manifest_policy = if prepared_support.all_roots_supported() {
        SemUnknownPolicy::Error
    } else {
        args.sem_unknown
    };
    let manifest = render_semantics_manifest(manifest_policy, &grammar_options, &manifest_grammars);

    let mut artifacts = GeneratedArtifacts::default();
    for (path, module) in rendered_modules {
        artifacts.insert(path, module)?;
    }
    artifacts.insert("semantics.json", manifest)?;
    artifacts.insert(
        "decisions.json",
        render_decisions_manifest(args.fixed_lookahead, &decision_report_grammars),
    )?;
    if let Some(manifest) = optimization_manifest {
        artifacts.insert("optimizations.json", manifest)?;
    } else {
        artifacts.remove_if_present("optimizations.json")?;
    }
    if let Some(test_rig) = &args.test_rig {
        artifacts.insert(
            TEST_RIG_MAIN_PATH,
            render_test_rig(&test_rig.start_rule, &test_rig_lexers, &test_rig_parsers)?,
        )?;
    }
    prepared_support.add_artifacts(&mut artifacts)?;
    let outputs = artifacts.write_to(&args.out_dir)?;
    Ok(Generation::new(inputs, outputs, warnings, diagnostics))
}

fn source_uses_rust_support(
    data: &RecognizerCodegenData<'_>,
    sources: &SourceSet,
    prepared_support: &PreparedRustSupport,
) -> bool {
    data.semantic
        .and_then(|semantic| sources.canonical_path(semantic.unit.source))
        .is_some_and(|source| prepared_support.supports_source(source))
}

fn option_hooks(
    args: &CompilerConfig,
    data: &RecognizerCodegenData<'_>,
    support_enabled: bool,
) -> BTreeSet<String> {
    let mut hooks = args.option_hooks.clone();
    if support_enabled && let Some(semantic) = data.semantic {
        hooks.extend(
            semantic
                .unit
                .options
                .iter()
                .filter(|option| option.name.value == "superClass")
                .map(|option| format!("{}={}", option.name.value, option.value.value)),
        );
    }
    hooks
}

fn insert_rendered_module(
    modules: &mut BTreeMap<PathBuf, String>,
    grammar_name: &str,
    module: String,
) -> io::Result<()> {
    let path = PathBuf::from(format!("{}.rs", module_name(grammar_name)));
    match modules.entry(path.clone()) {
        Entry::Vacant(entry) => {
            entry.insert(module);
            Ok(())
        }
        Entry::Occupied(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "generated module collision for grammar {grammar_name}: {}",
                path.display()
            ),
        )),
    }
}

fn deduplicate_grammar_options(options: &mut Vec<GrammarOptionEntry>) {
    options.sort_by(|left, right| {
        (&left.key, &left.value, left.line, left.column).cmp(&(
            &right.key,
            &right.value,
            right.line,
            right.column,
        ))
    });
    options.dedup_by(|left, right| {
        left.key == right.key
            && left.value == right.value
            && left.line == right.line
            && left.column == right.column
    });
}

fn compilation_error(
    error: &grammar::diagnostic::CompilationError,
    roots: &[PathBuf],
    prepared_support: &PreparedRustSupport,
) -> Error {
    let fallback = roots
        .first()
        .map_or_else(|| Path::new("<grammar>"), PathBuf::as_path);
    let mut message = String::new();
    let mut diagnostics = Vec::with_capacity(error.diagnostics().len());
    for (index, diagnostic) in error.diagnostics().iter().enumerate() {
        let (severity_name, severity) = match diagnostic.severity {
            grammar::diagnostic::Severity::Warning => ("warning", Severity::Warning),
            grammar::diagnostic::Severity::Error => ("error", Severity::Error),
        };
        let location = error.location(index);
        let path = location.map_or_else(
            || fallback.to_path_buf(),
            |location| prepared_support.original_path(&location.path),
        );
        let primary = diagnostic.primary_source_span();
        let remapped = location.is_some_and(|location| location.path != path);
        let position = (!remapped)
            .then(|| location.and_then(|location| location.position))
            .flatten();
        let structured_position = primary.and(position);
        let byte_span = (!remapped)
            .then(|| location.and(primary).map(source_byte_span))
            .flatten();
        let display_position =
            position.map_or_else(String::new, |(line, column)| format!(":{line}:{column}"));
        let _ = writeln!(
            message,
            "{severity_name}[{}]: {}{display_position}: {}",
            diagnostic.code,
            path.display(),
            diagnostic.message
        );
        diagnostics.push(Diagnostic::new(
            diagnostic.code,
            severity,
            diagnostic.message.clone(),
            path,
            structured_position,
            byte_span,
        ));
    }
    Error::compilation(message, diagnostics)
}

fn compilation_diagnostics(
    compilation: &grammar::compiler::Compilation,
    prepared_support: &PreparedRustSupport,
) -> Vec<Diagnostic> {
    compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == grammar::diagnostic::Severity::Warning)
        .map(|diagnostic| {
            let primary = diagnostic.primary_source_span();
            let source_path = primary
                .and_then(|span| compilation.sources.logical_path(span.source))
                .map(Path::to_path_buf);
            let path = source_path
                .as_deref()
                .map(|path| prepared_support.original_path(path));
            let remapped = source_path
                .as_ref()
                .zip(path.as_ref())
                .is_some_and(|(source, original)| source != original);
            let byte_span = primary
                .filter(|_| path.is_some() && !remapped)
                .map(source_byte_span);
            let path = path.unwrap_or_else(|| PathBuf::from("<grammar>"));
            let position = primary.and_then(|span| {
                (!remapped)
                    .then(|| {
                        compilation
                            .sources
                            .line_column(span.source, span.bytes.start)
                    })
                    .flatten()
            });
            Diagnostic::new(
                diagnostic.code,
                Severity::Warning,
                diagnostic.message.clone(),
                path,
                position,
                byte_span,
            )
        })
        .collect()
}

fn render_diagnostic(diagnostic: &Diagnostic) -> String {
    let severity = match diagnostic.severity() {
        Severity::Warning => "warning",
        Severity::Error => "error",
    };
    let position = diagnostic
        .line()
        .zip(diagnostic.column())
        .map_or_else(String::new, |(line, column)| format!(":{line}:{column}"));
    format!(
        "{severity}[{}]: {}{position}: {}",
        diagnostic.code(),
        diagnostic.path().display(),
        diagnostic.message()
    )
}

fn source_byte_span(span: &SourceSpan) -> std::ops::Range<usize> {
    usize::try_from(span.bytes.start).expect("source offset exceeds usize")
        ..usize::try_from(span.bytes.end).expect("source offset exceeds usize")
}
