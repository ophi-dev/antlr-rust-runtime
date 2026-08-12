// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use serde::Serialize;

use crate::json::to_pretty_json;

#[derive(Serialize)]
struct SemanticsManifest<'a> {
    version: u8,
    policy: &'static str,
    note: &'static str,
    options: Vec<GrammarOptionManifest<'a>>,
    grammars: Vec<SemanticsGrammarManifest<'a>>,
}

#[derive(Serialize)]
struct GrammarOptionManifest<'a> {
    name: &'a str,
    value: &'a str,
    line: usize,
    column: usize,
    disposition: &'static str,
}

impl<'a> From<&'a GrammarOptionEntry> for GrammarOptionManifest<'a> {
    fn from(entry: &'a GrammarOptionEntry) -> Self {
        Self {
            name: &entry.key,
            value: &entry.value,
            line: entry.line,
            column: entry.column,
            disposition: entry.disposition.manifest_name(),
        }
    }
}

#[derive(Serialize)]
struct SemanticsGrammarManifest<'a> {
    kind: &'static str,
    name: &'a str,
    coordinates: Vec<SemanticsCoordinateManifest<'a>>,
}

#[derive(Serialize)]
struct SemanticsCoordinateManifest<'a> {
    kind: &'static str,
    rule: Option<&'a str>,
    rule_index: Option<usize>,
    index: Option<usize>,
    atn_state: Option<usize>,
    line: Option<usize>,
    column: Option<usize>,
    body: Option<&'a str>,
    disposition: &'static str,
    template: Option<&'a str>,
}

impl<'a> From<&'a SemanticsEntry> for SemanticsCoordinateManifest<'a> {
    fn from(entry: &'a SemanticsEntry) -> Self {
        Self {
            kind: entry.kind.manifest_name(),
            rule: entry.rule_name.as_deref(),
            rule_index: entry.rule_index,
            index: entry.index,
            atn_state: entry.atn_state,
            line: entry.line,
            column: entry.column,
            body: entry.body.as_deref(),
            disposition: entry.disposition.manifest_name(),
            template: entry.template.as_deref(),
        }
    }
}

pub(crate) fn render_semantics_manifest(
    policy: SemUnknownPolicy,
    options: &[GrammarOptionEntry],
    grammars: &[(&'static str, String, Vec<SemanticsEntry>)],
) -> String {
    const DEPRECATION_NOTE: &str = "unknown coordinates currently default to assume-true; \
                                    a future minor release changes the default to error";
    let options = options.iter().map(GrammarOptionManifest::from).collect();
    let grammars = grammars
        .iter()
        .map(|(kind, name, entries)| SemanticsGrammarManifest {
            kind,
            name,
            coordinates: entries
                .iter()
                .map(SemanticsCoordinateManifest::from)
                .collect(),
        })
        .collect();
    to_pretty_json(&SemanticsManifest {
        version: 2,
        policy: policy.manifest_name(),
        note: DEPRECATION_NOTE,
        options,
        grammars,
    })
}

/// Per-parser-grammar rows for the `decisions.json` manifest.
pub(crate) struct DecisionReportGrammar {
    pub(crate) name: String,
    pub(crate) rule_names: Vec<String>,
    pub(crate) rows: Vec<DecisionReportRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DecisionsManifest<'a> {
    version: u8,
    fixed_lookahead: Option<usize>,
    grammars: Vec<DecisionGrammarManifest<'a>>,
}

#[derive(Serialize)]
struct DecisionGrammarManifest<'a> {
    name: &'a str,
    summary: DecisionSummaryManifest,
    decisions: Vec<DecisionRowManifest<'a>>,
}

#[derive(Serialize)]
struct DecisionSummaryManifest {
    total: usize,
    ll1: usize,
    fixed: usize,
    adaptive: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DecisionRowManifest<'a> {
    decision: usize,
    rule: Option<&'a str>,
    state: usize,
    can_defer: bool,
    #[serde(flatten)]
    tier: DecisionTierManifest,
}

#[derive(Serialize)]
#[serde(tag = "tier", rename_all = "lowercase")]
enum DecisionTierManifest {
    Ll1,
    Fixed {
        lookahead: usize,
    },
    Adaptive {
        reason: &'static str,
        #[serde(rename = "probedLookahead")]
        probed_lookahead: usize,
    },
}

impl DecisionSummaryManifest {
    fn from_rows(rows: &[DecisionReportRow]) -> Self {
        let (mut ll1, mut fixed, mut adaptive) = (0, 0, 0);
        for row in rows {
            match row.tier {
                DecisionTierReport::Ll1 => ll1 += 1,
                DecisionTierReport::Fixed { .. } => fixed += 1,
                DecisionTierReport::Adaptive { .. } => adaptive += 1,
            }
        }
        Self {
            total: rows.len(),
            ll1,
            fixed,
            adaptive,
        }
    }
}

fn decision_row_manifest<'a>(
    row: &DecisionReportRow,
    rule_names: &'a [String],
) -> DecisionRowManifest<'a> {
    let tier = match row.tier {
        DecisionTierReport::Ll1 => DecisionTierManifest::Ll1,
        DecisionTierReport::Fixed { lookahead } => DecisionTierManifest::Fixed { lookahead },
        DecisionTierReport::Adaptive {
            reason,
            probed_lookahead,
        } => DecisionTierManifest::Adaptive {
            reason: reason.manifest_name(),
            probed_lookahead,
        },
    };
    DecisionRowManifest {
        decision: row.decision,
        rule: row
            .rule_index
            .and_then(|rule_index| rule_names.get(rule_index))
            .map(String::as_str),
        state: row.state,
        can_defer: row.fallback.can_defer(),
        tier,
    }
}

/// Renders the `decisions.json` manifest: one row per parser decision with
/// its classifier tier - `ll1` (Java compiles a token switch), `fixed`
/// (`--fixed-lookahead` proved disjointness at `lookahead` tokens and a
/// static dispatch table was emitted), or `adaptive` with the reason the
/// decision keeps `adaptivePredict`. Each row also reports whether its emitted
/// path can defer to adaptive prediction. Deterministic: rows are in decision
/// order, and the classifier itself is pure.
pub(crate) fn render_decisions_manifest(
    fixed_lookahead: Option<usize>,
    grammars: &[DecisionReportGrammar],
) -> String {
    let grammars = grammars
        .iter()
        .map(|grammar| DecisionGrammarManifest {
            name: &grammar.name,
            summary: DecisionSummaryManifest::from_rows(&grammar.rows),
            decisions: grammar
                .rows
                .iter()
                .map(|row| decision_row_manifest(row, &grammar.rule_names))
                .collect(),
        })
        .collect();
    to_pretty_json(&DecisionsManifest {
        version: 2,
        fixed_lookahead,
        grammars,
    })
}
