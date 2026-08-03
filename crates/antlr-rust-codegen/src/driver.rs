use crate::cli::CompilerConfig;
use crate::generator::prelude::*;
use crate::lexer::{LexerRenderModel, render_lexer_model};
use crate::optimization::OptimizationPlan;
use crate::parser::{ParserRenderOptions, render_parser_with_decision_report};
use crate::semantics::{
    DecisionReportGrammar, GrammarOptionEntry, SemanticsEntry, collect_lexer_semantics,
    collect_parser_semantics_for_mode, collect_structural_grammar_options,
    enforce_require_full_options, enforce_require_full_semantics, enforce_sem_unknown,
    grammar_option_warning_messages, render_decisions_manifest, render_semantics_manifest,
};

pub(crate) fn generate(
    args: &CompilerConfig,
    mut report: impl FnMut(&str) -> io::Result<()>,
) -> Result<Generation, Error> {
    let optimizations = OptimizationPlan::from_compiler_config(args)?;
    let action_reference_parser: grammar::action::ActionReferenceParser = if args.embedded_actions {
        embedded::action_references
    } else {
        grammar::action::action_references
    };
    let compilation = grammar::compiler::compile_with_action_reference_parser(
        LoadOptions {
            roots: args.roots.clone(),
            library_directories: args.library_directories.clone(),
        },
        optimizations.transforms(),
        optimizations.report_only(),
        optimizations.entry_rules(),
        action_reference_parser,
    )
    .map_err(|error| compilation_error(&error, &args.roots))?;
    let mut warnings = compilation_warning_messages(&compilation);
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
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    if let Some(path) = &args.sem_patterns_path {
        let path = fs::canonicalize(path).map_err(Error::generation)?;
        if !inputs.contains(&path) {
            inputs.push(path);
        }
    }
    let optimization_manifest = optimizations.render_manifest(&compilation);
    if optimizations.report_only() {
        let mut artifacts = GeneratedArtifacts::default();
        artifacts.insert(
            "optimizations.json",
            optimization_manifest.expect("report mode enables the optimization manifest"),
        )?;
        let outputs = artifacts.write_to(&args.out_dir)?;
        return Ok(Generation::new(inputs, outputs, warnings));
    }

    let mut grammar_options = Vec::new();
    let mut manifest_grammars: Vec<(&'static str, String, Vec<SemanticsEntry>)> = Vec::new();
    let mut decision_report_grammars: Vec<DecisionReportGrammar> = Vec::new();
    let mut rendered_modules = BTreeMap::<PathBuf, String>::new();
    let mut emitted_lexers = BTreeSet::new();
    let mut emitted_parsers = BTreeSet::new();

    for root in &compilation.roots {
        if let Some(grammar) = root.lexer
            && emitted_lexers.insert(grammar)
        {
            let compiled = compilation
                .lexer(grammar)
                .expect("compiled root lexer artifact exists");
            let data = LexerCodegenData::from_compiled(compiled, &compilation.sources);
            grammar_options.extend(collect_structural_grammar_options(
                &data,
                &args.option_hooks,
            )?);
            let entries = collect_lexer_semantics(
                &data,
                args.embedded_actions,
                args.allow_unsupported_lexer_actions,
                args.sem_unknown,
                &args.sem_patterns,
            )?;
            enforce_sem_unknown(args.sem_unknown, &entries)?;
            enforce_require_full_semantics(args.require_full_semantics, &entries)?;
            let grammar_name = compiled.semantic.recognizer.name.clone();
            let render_model = LexerRenderModel::new(
                &grammar_name,
                &data,
                args.allow_unsupported_lexer_actions,
                args.sem_unknown,
                &args.sem_patterns,
                args.embedded_actions,
            );
            let module = render_lexer_model(&render_model)?;
            insert_rendered_module(&mut rendered_modules, &grammar_name, module)?;
            manifest_grammars.push(("lexer", grammar_name, entries));
        }

        if let Some(grammar) = root.parser
            && emitted_parsers.insert(grammar)
        {
            let compiled = compilation
                .parser(grammar)
                .expect("compiled root parser artifact exists");
            let data = ParserCodegenData::from_compiled(compiled, &compilation.sources);
            grammar_options.extend(collect_structural_grammar_options(
                &data,
                &args.option_hooks,
            )?);
            let entries = collect_parser_semantics_for_mode(
                &data,
                args.embedded_actions,
                args.sem_unknown,
                &args.sem_patterns,
            )?;
            enforce_sem_unknown(args.sem_unknown, &entries)?;
            enforce_require_full_semantics(args.require_full_semantics, &entries)?;
            let grammar_name = compiled.semantic.recognizer.name.clone();
            let (module, decision_report_rows) = render_parser_with_decision_report(
                &grammar_name,
                &data,
                ParserRenderOptions {
                    require_generated_parser: args.require_generated_parser,
                    embedded: args.embedded_actions,
                    generate_listener: args.generate_listener,
                    generate_visitor: args.generate_visitor,
                    sem_unknown: args.sem_unknown,
                    patterns: Some(&args.sem_patterns),
                    fixed_lookahead: args.fixed_lookahead,
                },
            )?;
            insert_rendered_module(&mut rendered_modules, &grammar_name, module)?;
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
    let manifest =
        render_semantics_manifest(args.sem_unknown, &grammar_options, &manifest_grammars);

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
    let outputs = artifacts.write_to(&args.out_dir)?;
    Ok(Generation::new(inputs, outputs, warnings))
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

fn compilation_error(error: &grammar::diagnostic::CompilationError, roots: &[PathBuf]) -> Error {
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
        let path =
            location.map_or_else(|| fallback.to_path_buf(), |location| location.path.clone());
        let position = location.and_then(|location| location.position);
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
            position,
        ));
    }
    Error::compilation(message, diagnostics)
}

fn compilation_warning_messages(compilation: &grammar::compiler::Compilation) -> Vec<String> {
    let mut messages = Vec::new();
    for diagnostic in compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == grammar::diagnostic::Severity::Warning)
    {
        let source = compilation.sources.get(diagnostic.primary.source);
        let path = source.map_or_else(
            || "<grammar>".to_owned(),
            |source| source.logical_path().display().to_string(),
        );
        let position = source
            .and_then(|source| source.line_column(diagnostic.primary.bytes.start))
            .map_or_else(String::new, |(line, column)| format!(":{line}:{column}"));
        messages.push(format!(
            "warning[{}]: {path}{position}: {}",
            diagnostic.code, diagnostic.message
        ));
    }
    messages
}
