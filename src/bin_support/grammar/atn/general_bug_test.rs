use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use antlr4_runtime::atn::AtnStateKind;

use super::build::{FinalizedAtnGraph, FinalizedTransition, FinalizedTransitionKind};
use super::interp_test::serialize_interp;
use crate::grammar::compiler::compile;
use crate::grammar::loader::LoadOptions;
use crate::grammar::model::RecognizerModel;

const EOF_TOKEN_TYPE: i32 = -1;

#[test]
fn bug_33_backslash_dot_edge_matches_upstream() {
    assert_general_bug_fixture(
        "general-bug-33-escaping-issues-with-backslash-in-dot-file-comparison-1a90edd812",
        "abbLexer",
        "EscapeSequence",
        "oracle/antlr-ng-abbLexer.EscapeSequence.dot",
    );
}

#[test]
fn bug_35_eof_dot_edge_does_not_crash() {
    assert_general_bug_fixture(
        "general-bug-35-tool-crashes-with-atn-4bd74f316f",
        "GoLexer",
        "EOS",
        "oracle/antlr-ng-GoLexer.EOS.dot",
    );
}

fn assert_general_bug_fixture(
    fixture_name: &str,
    grammar_name: &str,
    rule_name: &str,
    upstream_dot_path: &str,
) {
    let directory = fixture(fixture_name);
    let compilation = compile(LoadOptions {
        roots: vec![directory.join(format!("{grammar_name}.g4"))],
        library_directories: Vec::new(),
    })
    .unwrap_or_else(|error| panic!("{fixture_name} should compile: {error:#?}"));
    let compiled = compilation
        .lexer_named(grammar_name)
        .unwrap_or_else(|| panic!("{fixture_name} should contain lexer {grammar_name}"));

    let expected_interp =
        read_without_final_newline(&directory.join(format!("{grammar_name}.interp")));
    assert_eq!(
        serialize_interp(
            &compiled.semantic.recognizer,
            &compiled.runtime_artifact.atn_words,
        ),
        expected_interp,
        "{fixture_name} .interp",
    );
    assert_eq!(
        serialize_tokens(&compiled.semantic.recognizer),
        std::fs::read_to_string(directory.join(format!("{grammar_name}.tokens")))
            .expect("fixture tokens"),
        "{fixture_name} .tokens",
    );

    let rule = compiled
        .semantic
        .recognizer
        .rule_names
        .iter()
        .position(|name| name == rule_name)
        .unwrap_or_else(|| panic!("{fixture_name} should contain rule {rule_name}"));
    let expected_edge = read_without_final_newline(&directory.join("oracle/expected-dot-edge.txt"));
    let upstream_dot =
        std::fs::read_to_string(directory.join(upstream_dot_path)).expect("upstream DOT fixture");
    assert!(
        upstream_dot.lines().any(|line| line == expected_edge),
        "{fixture_name}: pinned upstream DOT does not contain {expected_edge:?}",
    );

    let actual_edges = lexer_atom_dot_edges(&compiled.graph, compiled.graph.rule_starts[rule]);
    assert!(
        actual_edges
            .iter()
            .any(|edge| edge.as_str() == expected_edge),
        "{fixture_name}: direct ATN DOT edges do not contain {expected_edge:?}: {actual_edges:#?}",
    );
}

fn lexer_atom_dot_edges(graph: &FinalizedAtnGraph, start: usize) -> Vec<String> {
    let transitions = graph
        .transitions
        .iter()
        .map(|transition| (transition.original, transition))
        .collect::<BTreeMap<_, _>>();
    let mut work = VecDeque::from([start]);
    let mut visited = BTreeSet::new();
    let mut output = Vec::new();

    while let Some(state_number) = work.pop_front() {
        if !visited.insert(state_number) {
            continue;
        }
        let state = &graph.states[state_number];
        if state.kind == AtnStateKind::RuleStop {
            continue;
        }
        for (index, transition_id) in state.transitions.iter().enumerate() {
            let transition = transitions
                .get(transition_id)
                .copied()
                .expect("ATN state transition exists");
            match &transition.kind {
                FinalizedTransitionKind::Rule { follow, .. } => work.push_back(*follow),
                _ => work.push_back(transition.target),
            }
            if let FinalizedTransitionKind::Atom(label) = transition.kind {
                output.push(atom_dot_edge(
                    transition,
                    index,
                    state.transitions.len(),
                    label,
                ));
            }
        }
    }
    output
}

fn atom_dot_edge(
    transition: &FinalizedTransition,
    index: usize,
    transition_count: usize,
    label: i32,
) -> String {
    let source = if transition_count > 1 {
        format!("s{}:p{index}", transition.source)
    } else {
        format!("s{}", transition.source)
    };
    format!(
        "{source} -> s{} [fontsize=11, fontname=\"Courier\", arrowsize=.7, label = \"{}\", arrowhead = normal];",
        transition.target,
        lexer_atom_dot_label(label),
    )
}

fn lexer_atom_dot_label(value: i32) -> String {
    if value == EOF_TOKEN_TYPE {
        return dot_escape("EOF");
    }
    let value = u32::try_from(value).expect("lexer atom is negative");
    let character = char::from_u32(value).expect("lexer atom is not a Unicode scalar value");
    dot_escape(&format!("'{}'", dot_escape(&character.to_string())))
}

fn dot_escape(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\\\n"),
            '\r' => {}
            _ => output.push(character),
        }
    }
    output
}

fn serialize_tokens(recognizer: &RecognizerModel) -> String {
    let mut output = String::new();
    for name in &recognizer.vocabulary.name_order {
        writeln!(output, "{name}={}", recognizer.vocabulary.by_name[name])
            .expect("writing to String cannot fail");
    }
    for literal in &recognizer.vocabulary.literal_order {
        writeln!(
            output,
            "{literal}={}",
            recognizer.vocabulary.by_literal[literal],
        )
        .expect("writing to String cannot fail");
    }
    output
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/codegen-direct/fixtures")
        .join(name)
}

fn read_without_final_newline(path: &Path) -> String {
    let text = std::fs::read_to_string(path).expect("fixture text");
    text.strip_suffix('\n').unwrap_or(&text).to_owned()
}
