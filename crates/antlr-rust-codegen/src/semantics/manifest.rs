pub(crate) fn render_semantics_manifest(
    policy: SemUnknownPolicy,
    options: &[GrammarOptionEntry],
    grammars: &[(&'static str, String, Vec<SemanticsEntry>)],
) -> String {
    const DEPRECATION_NOTE: &str = "unknown coordinates currently default to assume-true; \
                                    a future minor release changes the default to error";
    let mut out = String::new();
    out.push_str("{\n  \"version\": 2,\n");
    let _ = writeln!(
        out,
        "  \"policy\": {},",
        json_string(policy.manifest_name())
    );
    let _ = writeln!(out, "  \"note\": {},", json_string(DEPRECATION_NOTE));
    out.push_str("  \"options\": [");
    for (position, option) in options.iter().enumerate() {
        if position > 0 {
            out.push(',');
        }
        out.push_str("\n    ");
        write_grammar_option_entry(&mut out, option);
    }
    if options.is_empty() {
        out.push_str("],\n");
    } else {
        out.push_str("\n  ],\n");
    }
    out.push_str("  \"grammars\": [");
    for (grammar_position, (kind, name, entries)) in grammars.iter().enumerate() {
        if grammar_position > 0 {
            out.push(',');
        }
        out.push_str("\n    {\n");
        let _ = writeln!(out, "      \"kind\": {},", json_string(kind));
        let _ = writeln!(out, "      \"name\": {},", json_string(name));
        out.push_str("      \"coordinates\": [");
        for (entry_position, entry) in entries.iter().enumerate() {
            if entry_position > 0 {
                out.push(',');
            }
            out.push_str("\n        ");
            write_semantics_entry(&mut out, entry);
        }
        if entries.is_empty() {
            out.push_str("]\n    }");
        } else {
            out.push_str("\n      ]\n    }");
        }
    }
    if grammars.is_empty() {
        out.push_str("]\n}\n");
    } else {
        out.push_str("\n  ]\n}\n");
    }
    out
}

/// Per-parser-grammar rows for the `decisions.json` manifest.
pub(crate) struct DecisionReportGrammar {
    pub(crate) name: String,
    pub(crate) rule_names: Vec<String>,
    pub(crate) rows: Vec<DecisionReportRow>,
}

/// Renders the `decisions.json` manifest: one row per parser decision with
/// its classifier tier — `ll1` (Java compiles a token switch), `fixed`
/// (`--fixed-lookahead` proved disjointness at `lookahead` tokens and a
/// static dispatch table was emitted), or `adaptive` with the reason the
/// decision keeps `adaptivePredict`. Each row also reports whether its emitted
/// path can defer to adaptive prediction. Deterministic: rows are in decision
/// order, and the classifier itself is pure.
pub(crate) fn render_decisions_manifest(
    fixed_lookahead: Option<usize>,
    grammars: &[DecisionReportGrammar],
) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"version\": 2,\n");
    // `null` when the flag is unset: flag-off and `--fixed-lookahead 1`
    // emit different parsers (only the latter compiles static LL(1)
    // dispatch), so the manifest must not conflate them.
    let _ = writeln!(
        out,
        "  \"fixedLookahead\": {},",
        json_optional_number(fixed_lookahead)
    );
    out.push_str("  \"grammars\": [");
    for (grammar_position, grammar) in grammars.iter().enumerate() {
        if grammar_position > 0 {
            out.push(',');
        }
        out.push_str("\n    {\n");
        let _ = writeln!(out, "      \"name\": {},", json_string(&grammar.name));
        let (mut ll1, mut fixed, mut adaptive) = (0_usize, 0_usize, 0_usize);
        for row in &grammar.rows {
            match row.tier {
                DecisionTierReport::Ll1 => ll1 += 1,
                DecisionTierReport::Fixed { .. } => fixed += 1,
                DecisionTierReport::Adaptive { .. } => adaptive += 1,
            }
        }
        let _ = writeln!(
            out,
            "      \"summary\": {{\"total\": {}, \"ll1\": {ll1}, \"fixed\": {fixed}, \"adaptive\": {adaptive}}},",
            grammar.rows.len()
        );
        out.push_str("      \"decisions\": [");
        for (row_position, row) in grammar.rows.iter().enumerate() {
            if row_position > 0 {
                out.push(',');
            }
            out.push_str("\n        ");
            write_decision_report_row(&mut out, row, &grammar.rule_names);
        }
        if grammar.rows.is_empty() {
            out.push_str("]\n    }");
        } else {
            out.push_str("\n      ]\n    }");
        }
    }
    if grammars.is_empty() {
        out.push_str("]\n}\n");
    } else {
        out.push_str("\n  ]\n}\n");
    }
    out
}

fn write_decision_report_row(out: &mut String, row: &DecisionReportRow, rule_names: &[String]) {
    let rule_name = row
        .rule_index
        .and_then(|rule_index| rule_names.get(rule_index));
    out.push('{');
    let _ = write!(out, "\"decision\": {}", row.decision);
    let _ = write!(
        out,
        ", \"rule\": {}",
        json_optional_string(rule_name.map(String::as_str))
    );
    let _ = write!(out, ", \"state\": {}", row.state);
    let _ = write!(
        out,
        ", \"canDefer\": {}",
        if row.fallback.can_defer() {
            "true"
        } else {
            "false"
        }
    );
    match row.tier {
        DecisionTierReport::Ll1 => {
            let _ = write!(out, ", \"tier\": \"ll1\"");
        }
        DecisionTierReport::Fixed { lookahead } => {
            let _ = write!(out, ", \"tier\": \"fixed\", \"lookahead\": {lookahead}");
        }
        DecisionTierReport::Adaptive {
            reason,
            probed_lookahead,
        } => {
            let _ = write!(
                out,
                ", \"tier\": \"adaptive\", \"reason\": {}, \"probedLookahead\": {probed_lookahead}",
                json_string(reason.manifest_name())
            );
        }
    }
    out.push('}');
}

fn write_grammar_option_entry(out: &mut String, entry: &GrammarOptionEntry) {
    out.push('{');
    let _ = write!(out, "\"name\": {}", json_string(&entry.key));
    let _ = write!(out, ", \"value\": {}", json_string(&entry.value));
    let _ = write!(out, ", \"line\": {}", entry.line);
    let _ = write!(out, ", \"column\": {}", entry.column);
    let _ = write!(
        out,
        ", \"disposition\": {}",
        json_string(entry.disposition.manifest_name())
    );
    out.push('}');
}

fn write_semantics_entry(out: &mut String, entry: &SemanticsEntry) {
    out.push('{');
    let _ = write!(out, "\"kind\": {}", json_string(entry.kind.manifest_name()));
    let _ = write!(
        out,
        ", \"rule\": {}",
        json_optional_string(entry.rule_name.as_deref())
    );
    let _ = write!(
        out,
        ", \"rule_index\": {}",
        json_optional_number(entry.rule_index)
    );
    let _ = write!(out, ", \"index\": {}", json_optional_number(entry.index));
    let _ = write!(
        out,
        ", \"atn_state\": {}",
        json_optional_number(entry.atn_state)
    );
    let _ = write!(out, ", \"line\": {}", json_optional_number(entry.line));
    let _ = write!(out, ", \"column\": {}", json_optional_number(entry.column));
    let _ = write!(
        out,
        ", \"body\": {}",
        json_optional_string(entry.body.as_deref())
    );
    let _ = write!(
        out,
        ", \"disposition\": {}",
        json_string(entry.disposition.manifest_name())
    );
    let _ = write!(
        out,
        ", \"template\": {}",
        json_optional_string(entry.template.as_deref())
    );
    out.push('}');
}

fn json_optional_number(value: Option<usize>) -> String {
    value.map_or_else(|| "null".to_owned(), |number| number.to_string())
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

/// Escapes a string for JSON output; the generator hand-rolls this to avoid a
/// serialization dependency in a binary meant to be vendored into pipelines.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
