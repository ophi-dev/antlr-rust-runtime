// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
// Inventory of source-owned target-code *sections*: grammar-level and
// rule-level named actions (`@header`, `@definitions`, `@members`, `@init`,
// `@after`, …) plus rule exception clauses (`catch [...] { ... }` and
// `finally { ... }`).
//
// Unlike `SemanticsEntry` coordinates, sections own no ATN action/predicate
// coordinate, so the coordinate inventory alone can never account for them
// (issue #355). Every section is inventoried after import resolution and
// grammar transforms, receives a deterministic disposition in
// `semantics.json`, produces a warning when unsupported, and fails
// generation under `--require-full-semantics`.

/// Section kinds tracked by the `sections` manifest rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SectionKind {
    /// A grammar-level or rule-level `@name { ... }` action.
    NamedAction,
    /// A rule `catch [...] { ... }` exception handler.
    Catch,
    /// A rule `finally { ... }` clause.
    Finally,
}

impl SectionKind {
    const fn manifest_name(self) -> &'static str {
        match self {
            Self::NamedAction => "named-action",
            Self::Catch => "catch",
            Self::Finally => "finally",
        }
    }
}

/// How generation disposed of one target-code section.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SectionDisposition {
    /// The authored body is spliced into the generated Rust module (or the
    /// section is empty and there is nothing to splice).
    Embedded,
    /// The section's behavior is translated into generated metadata without
    /// splicing the authored body (e.g. `@members` state declared through
    /// `[[member]]` slots lowers into the semantic IR table).
    Translated,
    /// The section is legal ANTLR syntax but its target code is not executed
    /// by the generated module. Reported as a warning by default and rejected
    /// by `--require-full-semantics`.
    Unsupported,
}

impl SectionDisposition {
    const fn manifest_name(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::Translated => "translated",
            Self::Unsupported => "unsupported",
        }
    }
}

/// One source-owned target-code section inventoried for the manifest.
#[derive(Clone, Debug)]
pub(crate) struct SectionEntry {
    pub(crate) kind: SectionKind,
    /// Named-action name (`header`, `members`, `init`, …); `None` for
    /// exception clauses.
    pub(crate) name: Option<String>,
    /// Authored scope qualifier (`parser`, `lexer`, …) exactly as written.
    pub(crate) scope: Option<String>,
    pub(crate) rule_index: Option<usize>,
    pub(crate) rule_name: Option<String>,
    /// Logical source path for diagnostics (may be absolute).
    pub(crate) path: String,
    /// Source file name for the deterministic manifest row.
    pub(crate) source: Option<String>,
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) body: String,
    pub(crate) disposition: SectionDisposition,
    /// Remediation guidance rendered with unsupported-section diagnostics.
    pub(crate) note: Option<&'static str>,
}

impl SectionEntry {
    fn label(&self) -> String {
        let mut label = String::new();
        if let (Some(rule_name), Some(rule_index)) = (&self.rule_name, self.rule_index) {
            let _ = write!(label, "rule {rule_name}({rule_index}) ");
        }
        match self.kind {
            SectionKind::NamedAction => match (&self.scope, &self.name) {
                (Some(scope), Some(name)) => {
                    let _ = write!(label, "@{scope}::{name}");
                }
                (None, Some(name)) => {
                    let _ = write!(label, "@{name}");
                }
                _ => label.push_str("@<unnamed>"),
            },
            SectionKind::Catch => label.push_str("catch[...]"),
            SectionKind::Finally => label.push_str("finally"),
        }
        label
    }

    /// Renders the fail-loud diagnostic line for this section: source path,
    /// line, column, section kind, body, and remediation guidance.
    fn describe_unsupported(&self) -> String {
        let mut message = format!(
            "unsupported target-code section: {} at {}:{}:{}",
            self.label(),
            self.path,
            self.line,
            self.column
        );
        if !self.body.is_empty() {
            let _ = write!(message, ": {{{}}}", self.body);
        }
        if let Some(note) = self.note {
            let _ = write!(message, "; {note}");
        }
        message
    }
}

pub(crate) fn section_warning_messages(entries: &[SectionEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.disposition == SectionDisposition::Unsupported)
        .map(|entry| format!("warning: {}", entry.describe_unsupported()))
        .collect()
}

/// Fails generation when `--require-full-semantics` is active and any
/// authored target-code section has no executed implementation.
pub(crate) fn enforce_require_full_sections(
    require: bool,
    entries: &[SectionEntry],
) -> io::Result<()> {
    if !require {
        return Ok(());
    }
    let unsupported = entries
        .iter()
        .filter(|entry| entry.disposition == SectionDisposition::Unsupported)
        .collect::<Vec<_>>();
    if unsupported.is_empty() {
        return Ok(());
    }
    let mut message = String::new();
    for entry in &unsupported {
        message.push_str(&entry.describe_unsupported());
        message.push('\n');
    }
    let _ = write!(
        message,
        "--require-full-semantics: {} target-code section(s) would be silently dropped; \
         implement them under --actions embedded, route them through a documented hook, or \
         remove them from the grammar",
        unsupported.len()
    );
    Err(io::Error::new(io::ErrorKind::InvalidData, message))
}

/// Per-recognizer configuration for section inventory.
#[derive(Clone, Copy)]
pub(crate) struct SectionInventoryOptions<'a> {
    /// `--actions embedded` (or a Rust-support bundle) is active.
    pub(crate) embedded: bool,
    pub(crate) patterns: &'a SemPatternFile,
    /// Whether unscoped (and unknown-scoped) grammar-level actions belong to
    /// this recognizer. True for every parser recognizer and for lexer
    /// recognizers compiled from an authored lexer grammar; false for the
    /// lexer half of a split combined grammar, whose unscoped actions belong
    /// to the parser (ANTLR's default scope for combined grammars).
    pub(crate) owns_unscoped_actions: bool,
}

/// Extracts the Rust binding name from an authored `catch [...]` argument.
///
/// Accepts a bare identifier (`catch [error]`) or the Java catch-all form
/// `catch [RecognitionException e]`, binding the last identifier. Narrower
/// exception types (e.g. `FailedPredicateException`) are rejected: the
/// generated handler receives every recognition error, so accepting a
/// narrowed clause would run it for errors Java's type match skips.
pub(crate) fn exception_catch_binding(argument: &str) -> Option<String> {
    let mut tokens = argument.split_whitespace().rev();
    let binding = tokens.next()?;
    let mut chars = binding.chars();
    let valid = chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric());
    // An optional leading type token must be the catch-all recognition-error
    // base type; the generated handler cannot narrow by exception type.
    let type_token = tokens.next();
    (valid
        && type_token.is_none_or(|token| token == "RecognitionException")
        && tokens.next().is_none()
        && !is_rust_keyword(binding))
    .then(|| binding.to_owned())
}

/// Inventories every source-owned target-code section visible to one
/// recognizer: grammar-level named actions (scope-routed), rule-level named
/// actions, and rule exception clauses.
///
/// Deterministic: grammar-level actions in declaration order, then rule-level
/// sections in rule-index order with declaration order within a rule.
pub(crate) fn collect_recognizer_sections(
    data: &RecognizerCodegenData<'_>,
    options: SectionInventoryOptions<'_>,
) -> io::Result<Vec<SectionEntry>> {
    let semantic = data.semantic.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "structural grammar model is unavailable",
        )
    })?;
    let recognizer_scope = match semantic.unit.kind {
        GrammarKind::Lexer => "lexer",
        GrammarKind::Parser | GrammarKind::Combined => "parser",
    };
    let mut entries = Vec::new();
    for action in &semantic.unit.actions {
        let (owned, known_scope) = match action.scope.as_deref() {
            Some(scope) if scope == recognizer_scope => (true, true),
            // The other recognizer of this grammar owns the section.
            Some("lexer" | "parser") => (false, true),
            None => (options.owns_unscoped_actions, true),
            // Unknown scopes follow the unit's default scope for ownership,
            // but no backend consumes them, so they can never be embedded.
            Some(_) => (options.owns_unscoped_actions, false),
        };
        if !owned {
            continue;
        }
        let (disposition, note) = if known_scope || action.body.trim().is_empty() {
            grammar_action_disposition(action, recognizer_scope, options)
        } else {
            (SectionDisposition::Unsupported, Some(UNKNOWN_SCOPE_NOTE))
        };
        entries.push(section_entry_for_action(
            data,
            action,
            None,
            disposition,
            note,
        ));
    }

    if recognizer_scope == "lexer" {
        // Lexer rules carry no rule-level actions or exception clauses
        // (enforced by the grammar frontend).
        return Ok(entries);
    }

    let mut rule_entries = Vec::new();
    for rule in &semantic.unit.rules {
        let Some(&rule_index) = semantic.recognizer.rule_numbers.get(&rule.id) else {
            continue;
        };
        let rule_ref = Some((rule_index, rule.name.as_str()));
        let mut seen_names = BTreeSet::new();
        for action in &rule.actions {
            let (disposition, note) =
                rule_action_disposition(action, options.embedded, &mut seen_names);
            rule_entries.push((
                rule_index,
                section_entry_for_action(data, action, rule_ref, disposition, note),
            ));
        }
        for handler in &rule.catches {
            let (disposition, note) = catch_disposition(handler, rule, options.embedded);
            let (line, column) = structural_line_column(data, &handler.span);
            rule_entries.push((
                rule_index,
                SectionEntry {
                    kind: SectionKind::Catch,
                    name: None,
                    scope: None,
                    rule_index: Some(rule_index),
                    rule_name: Some(rule.name.clone()),
                    path: section_path(data, &handler.span),
                    source: section_source(data, &handler.span),
                    line,
                    column,
                    body: one_line_action_body(&handler.body),
                    disposition,
                    note,
                },
            ));
        }
        if let Some(action) = &rule.finally_action {
            // An empty `finally` has no target code to lose in either mode.
            let disposition = if options.embedded || action.body.trim().is_empty() {
                SectionDisposition::Embedded
            } else {
                SectionDisposition::Unsupported
            };
            let note =
                (disposition == SectionDisposition::Unsupported).then_some(TEMPLATES_MODE_NOTE);
            let (line, column) = structural_line_column(data, &action.span);
            rule_entries.push((
                rule_index,
                SectionEntry {
                    kind: SectionKind::Finally,
                    name: None,
                    scope: None,
                    rule_index: Some(rule_index),
                    rule_name: Some(rule.name.clone()),
                    path: section_path(data, &action.span),
                    source: section_source(data, &action.span),
                    line,
                    column,
                    body: one_line_action_body(&action.body),
                    disposition,
                    note,
                },
            ));
        }
    }
    rule_entries.sort_by_key(|(rule_index, _)| *rule_index);
    entries.extend(rule_entries.into_iter().map(|(_, entry)| entry));
    Ok(entries)
}

const TEMPLATES_MODE_NOTE: &str = "templates mode does not execute authored target-code \
     sections; generate with --actions embedded from a Rust-authored grammar, or remove the \
     section";
const LEXER_SECTION_NOTE: &str = "lexer-scoped sections are not implemented in embedded mode; \
     move shared code into @parser::definitions or a hand-maintained module";
const UNKNOWN_SECTION_NOTE: &str = "unknown named action; embedded Rust generation implements \
     @header, @definitions, and @members at grammar scope";
const DUPLICATE_RULE_SECTION_NOTE: &str = "duplicate rule section; only the first occurrence \
     is executed";
const RULE_SECTION_NOTE: &str = "unknown rule action; embedded Rust generation implements \
     @init and @after at rule scope";
const MULTIPLE_CATCH_NOTE: &str = "multiple catch clauses are not supported; merge the \
     handlers into one clause and match on the bound error value";
const CATCH_ARGUMENT_NOTE: &str = "catch argument must be a Rust identifier (optionally \
     preceded by the catch-all RecognitionException type); the handler receives every \
     recognition error under that name and cannot narrow by exception type";
const UNKNOWN_SCOPE_NOTE: &str = "unknown section scope; supported scopes are lexer and parser";

fn grammar_action_disposition(
    action: &grammar::model::NamedAction,
    recognizer_scope: &str,
    options: SectionInventoryOptions<'_>,
) -> (SectionDisposition, Option<&'static str>) {
    if action.body.trim().is_empty() {
        // An empty section has no target code to lose.
        return (SectionDisposition::Embedded, None);
    }
    let member_scope = if recognizer_scope == "lexer" {
        stack_member::MemberScope::Lexer
    } else {
        stack_member::MemberScope::Parser
    };
    if !options.embedded {
        if action.name == "members" && options.patterns.has_member_declarations(member_scope) {
            // `[[member]]` slots lower the declared state into the semantic IR
            // table; the authored body is replaced, not hook-routed.
            return (SectionDisposition::Translated, None);
        }
        return (SectionDisposition::Unsupported, Some(TEMPLATES_MODE_NOTE));
    }
    if recognizer_scope == "lexer" {
        return (SectionDisposition::Unsupported, Some(LEXER_SECTION_NOTE));
    }
    match action.name.as_str() {
        "header" | "definitions" | "members" => (SectionDisposition::Embedded, None),
        _ => (SectionDisposition::Unsupported, Some(UNKNOWN_SECTION_NOTE)),
    }
}

fn rule_action_disposition(
    action: &grammar::model::NamedAction,
    embedded: bool,
    seen_names: &mut BTreeSet<String>,
) -> (SectionDisposition, Option<&'static str>) {
    // Even an empty occurrence reserves the name: `structural_embedded_model`
    // executes only the *first* `@init` / `@after`, so a later non-empty
    // duplicate after an empty first one is still dropped.
    let first = seen_names.insert(action.name.clone());
    if action.body.trim().is_empty() {
        return (SectionDisposition::Embedded, None);
    }
    if !embedded {
        return (SectionDisposition::Unsupported, Some(TEMPLATES_MODE_NOTE));
    }
    match action.name.as_str() {
        "init" | "after" if first => (SectionDisposition::Embedded, None),
        "init" | "after" => (
            SectionDisposition::Unsupported,
            Some(DUPLICATE_RULE_SECTION_NOTE),
        ),
        _ => (SectionDisposition::Unsupported, Some(RULE_SECTION_NOTE)),
    }
}

fn catch_disposition(
    handler: &grammar::model::ExceptionHandler,
    rule: &Rule,
    embedded: bool,
) -> (SectionDisposition, Option<&'static str>) {
    if !embedded {
        return (SectionDisposition::Unsupported, Some(TEMPLATES_MODE_NOTE));
    }
    if rule.catches.len() > 1 {
        return (SectionDisposition::Unsupported, Some(MULTIPLE_CATCH_NOTE));
    }
    if exception_catch_binding(&handler.argument).is_none() {
        return (SectionDisposition::Unsupported, Some(CATCH_ARGUMENT_NOTE));
    }
    (SectionDisposition::Embedded, None)
}

fn section_entry_for_action(
    data: &RecognizerCodegenData<'_>,
    action: &grammar::model::NamedAction,
    rule: Option<(usize, &str)>,
    disposition: SectionDisposition,
    note: Option<&'static str>,
) -> SectionEntry {
    let (line, column) = structural_line_column(data, &action.span);
    SectionEntry {
        kind: SectionKind::NamedAction,
        name: Some(action.name.clone()),
        scope: action.scope.clone(),
        rule_index: rule.map(|(index, _)| index),
        rule_name: rule.map(|(_, name)| name.to_owned()),
        path: section_path(data, &action.span),
        source: section_source(data, &action.span),
        line,
        column,
        body: one_line_action_body(&action.body),
        disposition,
        note,
    }
}

fn section_path(data: &RecognizerCodegenData<'_>, span: &SourceSpan) -> String {
    data.sources
        .and_then(|sources| sources.logical_path(span.source))
        .map_or_else(|| "<grammar>".to_owned(), |path| path.display().to_string())
}

/// File name only: manifests must stay deterministic across checkouts, so
/// absolute logical paths never appear in `semantics.json`.
fn section_source(data: &RecognizerCodegenData<'_>, span: &SourceSpan) -> Option<String> {
    data.sources
        .and_then(|sources| sources.logical_path(span.source))
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
}
