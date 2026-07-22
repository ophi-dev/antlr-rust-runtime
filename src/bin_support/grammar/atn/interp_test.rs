use std::collections::BTreeMap;
use std::fmt::Write;

use antlr4_runtime::atn::AtnStateKind;
use antlr4_runtime::atn::serialized::SERIALIZED_VERSION;

use super::super::model::{GrammarKind, RecognizerModel};
use super::build::{FinalizedAtnGraph, FinalizedTransition, FinalizedTransitionKind};

const PARSER_ATN_TYPE: i32 = 1;
const EOF_TOKEN_TYPE: i32 = -1;

pub(super) fn serialize_parser(graph: &FinalizedAtnGraph) -> Vec<i32> {
    let transitions = transitions_by_id(graph);
    let sets = collect_sets(graph, &transitions);
    let mut data = vec![
        SERIALIZED_VERSION,
        PARSER_ATN_TYPE,
        graph.max_token_type,
        usize_to_i32(graph.states.len()),
    ];

    for state in &graph.states {
        data.push(state_type(state.kind));
        if state.kind == AtnStateKind::Invalid {
            continue;
        }
        data.push(state.rule_index.map_or(-1, usize_to_i32));
        match state.kind {
            AtnStateKind::LoopEnd => data.push(usize_to_i32(
                state
                    .loop_back_state
                    .expect("loop-end state has a loop-back state"),
            )),
            AtnStateKind::BlockStart
            | AtnStateKind::PlusBlockStart
            | AtnStateKind::StarBlockStart => data.push(usize_to_i32(
                state.end_state.expect("block-start state has an end state"),
            )),
            _ => {}
        }
    }

    append_state_list(
        &mut data,
        graph
            .states
            .iter()
            .enumerate()
            .filter_map(|(index, state)| state.non_greedy.then_some(index)),
    );
    append_state_list(
        &mut data,
        graph
            .states
            .iter()
            .enumerate()
            .filter_map(|(index, state)| {
                (state.kind == AtnStateKind::RuleStart && state.left_recursive_rule)
                    .then_some(index)
            }),
    );

    data.push(usize_to_i32(graph.rule_starts.len()));
    data.extend(graph.rule_starts.iter().copied().map(usize_to_i32));
    data.push(0);
    serialize_sets(&mut data, &sets);
    serialize_edges(&mut data, graph, &transitions, &sets);
    append_state_list(&mut data, graph.decisions.iter().copied());
    data
}

fn serialize_interp(recognizer: &RecognizerModel, atn: &[i32]) -> String {
    let mut output = String::new();
    write_optional_names(
        &mut output,
        "token literal names:",
        &recognizer.literal_names,
    );
    write_optional_names(
        &mut output,
        "token symbolic names:",
        &recognizer.symbolic_names,
    );
    write_names(&mut output, "rule names:", &recognizer.rule_names);
    match recognizer.kind {
        GrammarKind::Lexer => {
            write_optional_names(&mut output, "channel names:", &recognizer.channel_names);
            write_names(&mut output, "mode names:", &recognizer.mode_names);
        }
        GrammarKind::Parser => output.push('\n'),
        GrammarKind::Combined => {
            panic!("combined grammars must be split before .interp serialization")
        }
    }
    output.push_str("atn:\n[");
    for (index, value) in atn.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write!(output, "{value}").expect("writing to String cannot fail");
    }
    output.push(']');
    output
}

fn write_optional_names(output: &mut String, heading: &str, names: &[Option<String>]) {
    output.push_str(heading);
    output.push('\n');
    for name in names {
        output.push_str(name.as_deref().unwrap_or("null"));
        output.push('\n');
    }
    output.push('\n');
}

fn write_names(output: &mut String, heading: &str, names: &[String]) {
    output.push_str(heading);
    output.push('\n');
    for name in names {
        output.push_str(name);
        output.push('\n');
    }
    output.push('\n');
}

fn transitions_by_id(
    graph: &FinalizedAtnGraph,
) -> BTreeMap<super::super::model::BuildTransitionId, &FinalizedTransition> {
    graph
        .transitions
        .iter()
        .map(|transition| (transition.original, transition))
        .collect()
}

fn collect_sets(
    graph: &FinalizedAtnGraph,
    transitions: &BTreeMap<super::super::model::BuildTransitionId, &FinalizedTransition>,
) -> Vec<Vec<(i32, i32)>> {
    let mut sets = Vec::new();
    for state in &graph.states {
        for transition in &state.transitions {
            let Some(transition) = transitions.get(transition) else {
                continue;
            };
            let (FinalizedTransitionKind::Set(ranges) | FinalizedTransitionKind::NotSet(ranges)) =
                &transition.kind
            else {
                continue;
            };
            let normalized = normalize_ranges(ranges);
            if !sets.contains(&normalized) {
                sets.push(normalized);
            }
        }
    }
    sets
}

fn normalize_ranges(ranges: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut ranges = ranges.to_vec();
    ranges.sort_unstable();
    let mut normalized: Vec<(i32, i32)> = Vec::with_capacity(ranges.len());
    for (start, stop) in ranges {
        let (start, stop) = if start <= stop {
            (start, stop)
        } else {
            (stop, start)
        };
        if let Some((_, previous_stop)) = normalized.last_mut()
            && start <= previous_stop.saturating_add(1)
        {
            *previous_stop = (*previous_stop).max(stop);
        } else {
            normalized.push((start, stop));
        }
    }
    normalized
}

fn append_state_list(data: &mut Vec<i32>, states: impl IntoIterator<Item = usize>) {
    let states = states.into_iter().collect::<Vec<_>>();
    data.push(usize_to_i32(states.len()));
    data.extend(states.into_iter().map(usize_to_i32));
}

fn serialize_sets(data: &mut Vec<i32>, sets: &[Vec<(i32, i32)>]) {
    data.push(usize_to_i32(sets.len()));
    for set in sets {
        let contains_eof = set
            .iter()
            .any(|(start, stop)| *start <= EOF_TOKEN_TYPE && EOF_TOKEN_TYPE <= *stop);
        let eof_singleton = set
            .first()
            .is_some_and(|range| *range == (EOF_TOKEN_TYPE, EOF_TOKEN_TYPE));
        data.push(usize_to_i32(set.len() - usize::from(eof_singleton)));
        data.push(i32::from(contains_eof));
        for &(start, stop) in set {
            if (start, stop) == (EOF_TOKEN_TYPE, EOF_TOKEN_TYPE) {
                continue;
            }
            data.push(if start == EOF_TOKEN_TYPE { 0 } else { start });
            data.push(stop);
        }
    }
}

fn serialize_edges(
    data: &mut Vec<i32>,
    graph: &FinalizedAtnGraph,
    transitions: &BTreeMap<super::super::model::BuildTransitionId, &FinalizedTransition>,
    sets: &[Vec<(i32, i32)>],
) {
    let edge_count = graph
        .states
        .iter()
        .filter(|state| state.kind != AtnStateKind::RuleStop)
        .map(|state| {
            state
                .transitions
                .iter()
                .filter(|transition| transitions.contains_key(transition))
                .count()
        })
        .sum();
    data.push(usize_to_i32(edge_count));

    for state in &graph.states {
        if state.kind == AtnStateKind::RuleStop {
            continue;
        }
        for transition in &state.transitions {
            let Some(transition) = transitions.get(transition) else {
                continue;
            };
            let (target, edge_type, arg1, arg2, arg3) = edge_descriptor(transition, sets);
            data.extend([
                usize_to_i32(transition.source),
                usize_to_i32(target),
                edge_type,
                arg1,
                arg2,
                arg3,
            ]);
        }
    }
}

fn edge_descriptor(
    transition: &FinalizedTransition,
    sets: &[Vec<(i32, i32)>],
) -> (usize, i32, i32, i32, i32) {
    match &transition.kind {
        FinalizedTransitionKind::Epsilon => (transition.target, 1, 0, 0, 0),
        FinalizedTransitionKind::Range { start, stop } => (
            transition.target,
            2,
            if *start == EOF_TOKEN_TYPE { 0 } else { *start },
            *stop,
            i32::from(*start == EOF_TOKEN_TYPE),
        ),
        FinalizedTransitionKind::Rule {
            rule_index,
            follow,
            precedence,
            ..
        } => (
            *follow,
            3,
            usize_to_i32(transition.target),
            usize_to_i32(*rule_index),
            *precedence,
        ),
        FinalizedTransitionKind::Predicate {
            rule_index,
            predicate_index,
            context_dependent,
        } => (
            transition.target,
            4,
            usize_to_i32(*rule_index),
            usize_to_i32(*predicate_index),
            i32::from(*context_dependent),
        ),
        FinalizedTransitionKind::Atom(label) => (
            transition.target,
            5,
            if *label == EOF_TOKEN_TYPE { 0 } else { *label },
            0,
            i32::from(*label == EOF_TOKEN_TYPE),
        ),
        FinalizedTransitionKind::Action {
            rule_index,
            action_index,
            context_dependent,
        } => (
            transition.target,
            6,
            usize_to_i32(*rule_index),
            action_index.map_or(-1, usize_to_i32),
            i32::from(*context_dependent),
        ),
        FinalizedTransitionKind::Set(ranges) => {
            (transition.target, 7, set_index(sets, ranges), 0, 0)
        }
        FinalizedTransitionKind::NotSet(ranges) => {
            (transition.target, 8, set_index(sets, ranges), 0, 0)
        }
        FinalizedTransitionKind::Wildcard => (transition.target, 9, 0, 0, 0),
        FinalizedTransitionKind::Precedence(precedence) => {
            (transition.target, 10, *precedence, 0, 0)
        }
    }
}

fn set_index(sets: &[Vec<(i32, i32)>], ranges: &[(i32, i32)]) -> i32 {
    let ranges = normalize_ranges(ranges);
    usize_to_i32(
        sets.iter()
            .position(|candidate| *candidate == ranges)
            .expect("serialized transition set was collected"),
    )
}

const fn state_type(kind: AtnStateKind) -> i32 {
    match kind {
        AtnStateKind::Invalid => 0,
        AtnStateKind::Basic => 1,
        AtnStateKind::RuleStart => 2,
        AtnStateKind::BlockStart => 3,
        AtnStateKind::PlusBlockStart => 4,
        AtnStateKind::StarBlockStart => 5,
        AtnStateKind::TokenStart => 6,
        AtnStateKind::RuleStop => 7,
        AtnStateKind::BlockEnd => 8,
        AtnStateKind::StarLoopBack => 9,
        AtnStateKind::StarLoopEntry => 10,
        AtnStateKind::PlusLoopBack => 11,
        AtnStateKind::LoopEnd => 12,
    }
}

fn usize_to_i32(value: usize) -> i32 {
    i32::try_from(value).expect("test ATN value exceeds i32")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, VecDeque};
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use antlr4_runtime::InputStream;
    use antlr4_runtime::RecognizerData;
    use antlr4_runtime::atn::lexer::{next_token_compiled_with_hooks, next_token_with_hooks};
    use antlr4_runtime::atn::lexer_dfa::CompiledLexerDfa;
    use antlr4_runtime::atn::serialized::{AtnDeserializer, SerializedAtn};
    use antlr4_runtime::lexer::{BaseLexer, Lexer};
    use antlr4_runtime::token::{
        TOKEN_EOF, Token, TokenId, TokenSink, TokenSource, TokenSourceError, TokenStoreError,
    };
    use antlr4_runtime::token_stream::CommonTokenStream;
    use antlr4_runtime::vocabulary::Vocabulary;

    use super::*;
    use crate::grammar::compiler::{Compilation, compile};
    use crate::grammar::loader::{LoadOptions, load};
    use crate::grammar::provenance::Origin;
    use crate::grammar::semantics::analyze;
    use crate::grammar::transform::integrate_loaded;

    #[test]
    fn parser_basic_matches_java_serialization_and_direct_packing() {
        assert_parser_fixture("parser-basic", "ParserBasic");
    }

    #[test]
    fn parser_shapes_match_java_serialization_and_direct_packing() {
        assert_parser_fixture("parser-shapes", "ParserShapes");
    }

    #[test]
    fn parser_left_recursion_matches_java_serialization_and_direct_packing() {
        assert_parser_fixture("parser-left-recursion", "ParserLeftRecursion");
    }

    #[test]
    fn parser_full_left_recursion_matches_java_serialization_and_direct_packing() {
        assert_parser_fixture("parser-left-recursion-full", "ParserLeftRecursionFull");
    }

    #[test]
    fn parser_indirect_left_recursion_is_diagnosed_after_atn_construction() {
        let error = compile_parser_fixture(
            "parser-indirect-left-recursion",
            "ParserIndirectLeftRecursion",
        )
        .expect_err("mutual left recursion must be fatal");
        let diagnostic = error
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code == "G4A005")
            .expect("mutual left-recursion diagnostic");
        assert!(diagnostic.message.contains('a'));
        assert!(diagnostic.message.contains('b'));
        assert!(diagnostic.message.contains('c'));
    }

    #[test]
    fn parser_epsilon_closure_is_fatal() {
        let error = compile_parser_fixture("parser-epsilon-closure", "ParserEpsilonClosure")
            .expect_err("nullable closure must be fatal");
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "G4A001")
        );
    }

    #[test]
    fn parser_epsilon_optional_warns_and_still_matches_java() {
        let compilation = assert_parser_fixture("parser-epsilon-optional", "ParserEpsilonOptional");
        let compiled = parser_named(&compilation, "ParserEpsilonOptional");
        assert!(
            compiled
                .analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "G4A004")
        );
    }

    #[test]
    fn parser_ll1_analysis_tracks_disjoint_overlap_and_predicates() {
        let compilation = assert_parser_fixture("parser-lookahead", "ParserLookahead");
        let compiled = parser_named(&compilation, "ParserLookahead");
        let lookahead = &compiled.analysis.decision_lookahead;
        assert_eq!(lookahead.len(), 3);
        assert_eq!(
            lookahead[0].alternatives,
            [Some(vec![(1, 1)]), Some(vec![(2, 2)])]
        );
        assert!(lookahead[0].disjoint);
        assert_eq!(
            lookahead[1].alternatives,
            [Some(vec![(1, 1)]), Some(vec![(1, 1)])]
        );
        assert!(!lookahead[1].disjoint);
        assert_eq!(lookahead[2].alternatives, [None, Some(vec![(4, 4)])]);
        assert!(!lookahead[2].disjoint);
    }

    #[test]
    fn lexer_basic_matches_java_serialization_and_direct_artifact() {
        assert_lexer_fixture("lexer-basic", "LexerBasic");
    }

    #[test]
    fn lexer_shapes_match_java_serialization_and_direct_artifact() {
        let compilation = assert_lexer_fixture("lexer-shapes", "LexerShapes");
        let compiled = lexer_named(&compilation, "LexerShapes");
        let warnings = compiled
            .analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "G4A006")
            .count();
        assert_eq!(warnings, 2);
    }

    #[test]
    fn lexer_unicode_matches_pinned_java_properties_and_case_mappings() {
        assert_lexer_fixture("lexer-unicode", "LexerUnicode");
    }

    #[test]
    fn lexer_indirect_recursion_is_tracked_with_java_atn_parity() {
        let compilation = assert_lexer_fixture("lexer-recursion", "LexerRecursion");
        let compiled = lexer_named(&compilation, "LexerRecursion");
        assert_eq!(compiled.analysis.recursive_components.len(), 1);
        assert_eq!(compiled.analysis.recursive_components[0].len(), 2);
    }

    #[test]
    fn direct_lexer_interpreted_and_compiled_token_streams_match() {
        let split =
            compile_fixture("vscode-split", &["TParser.g4"]).expect("split fixture should compile");
        let lexer = lexer_named(&split, "TLexer");
        for input in [
            "return foo bar # comment\r\n \t,&",
            "$.",
            "{.",
            "\u{80}9 \"quoted\"",
        ] {
            assert_lexer_strategies_match(lexer, input);
        }

        let basic =
            compile_fixture("lexer-basic", &["LexerBasic.g4"]).expect("basic lexer should compile");
        assert_lexer_strategies_match(lexer_named(&basic, "LexerBasic"), "a \u{2603} word");

        let unicode = compile_fixture("lexer-unicode", &["LexerUnicode.g4"])
            .expect("Unicode lexer should compile");
        assert_lexer_strategies_match(
            lexer_named(&unicode, "LexerUnicode"),
            "\u{10330}\u{10400}\u{3b2}",
        );
    }

    #[test]
    fn vscode_sentences_combined_parser_and_unicode_lexer_match_java() {
        let fixture = fixture("vscode-sentences");
        let compilation =
            compile_fixture("vscode-sentences", &["sentences.g4"]).expect("fixture should compile");
        let lexer = lexer_named(&compilation, "sentencesLexer");
        assert_lexer_interp(lexer, &fixture.join("sentencesLexer.interp"));

        let parser = parser_named(&compilation, "sentencesParser");
        assert_parser_interp(parser, &fixture.join("sentences.interp"));
    }

    #[test]
    fn vscode_alternate_meta_grammar_and_import_match_java() {
        assert_lexer_fixture("vscode-meta-grammar", "LexBasic");

        let compilation = compile_fixture("vscode-meta-grammar", &["ANTLRv4Parser.g4"])
            .expect("fixture should compile");
        let lexer = lexer_named(&compilation, "ANTLRv4Lexer");
        assert_lexer_interp(
            lexer,
            &fixture("vscode-meta-grammar").join("ANTLRv4Lexer.interp"),
        );
        let parser = parser_named(&compilation, "ANTLRv4Parser");
        assert_parser_interp(
            parser,
            &fixture("vscode-meta-grammar").join("ANTLRv4Parser.interp"),
        );
    }

    #[test]
    fn vscode_split_grammar_matches_java() {
        let compilation =
            compile_fixture("vscode-split", &["TParser.g4"]).expect("fixture should compile");
        let lexer = lexer_named(&compilation, "TLexer");
        assert_lexer_interp(lexer, &fixture("vscode-split").join("TLexer.interp"));
        let parser = parser_named(&compilation, "TParser");
        assert_parser_interp(parser, &fixture("vscode-split").join("TParser.interp"));
    }

    #[test]
    fn vscode_cpp14_combined_grammar_matches_java() {
        let compilation =
            compile_fixture("vscode-cpp14", &["CPP14.g4"]).expect("fixture should compile");
        let lexer = lexer_named(&compilation, "CPP14Lexer");
        assert_lexer_interp(lexer, &fixture("vscode-cpp14").join("CPP14Lexer.interp"));
        let parser = parser_named(&compilation, "CPP14Parser");
        assert_parser_interp(parser, &fixture("vscode-cpp14").join("CPP14.interp"));
    }

    #[test]
    fn vscode_odd_expr_large_atns_match_java_without_state_limit() {
        let compilation =
            compile_fixture("vscode-odd-expr", &["OddExpr.g4"]).expect("fixture should compile");
        assert_eq!(
            compilation
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity
                    == crate::grammar::diagnostic::Severity::Warning)
                .count(),
            22
        );
        let lexer = lexer_named(&compilation, "OddExprLexer");
        assert_eq!(lexer.atn.states().len(), 101_246);
        assert_lexer_interp(
            lexer,
            &fixture("vscode-odd-expr").join("OddExprLexer.interp"),
        );
        let parser = parser_named(&compilation, "OddExprParser");
        assert_parser_interp(parser, &fixture("vscode-odd-expr").join("OddExpr.interp"));
    }

    mod upstream_atn_serialization {
        use super::*;

        macro_rules! case {
            ($name:ident, parser, $fixture:literal, $grammar:literal) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_parser_fixture($fixture, $grammar);
                    }
                }
            };
            ($name:ident, lexer, $fixture:literal, $grammar:literal) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_lexer_fixture($fixture, $grammar);
                    }
                }
            };
        }

        case!(
            simple_no_block,
            parser,
            "testatnserialization-testsimplenoblock-a3dba6abcf",
            "T"
        );
        case!(eof, parser, "testatnserialization-testeof-1a07e98df4", "T");
        case!(
            eof_in_set,
            parser,
            "testatnserialization-testeofinset-74204a5ce0",
            "T"
        );
        case!(not, parser, "testatnserialization-testnot-ed1dd74e70", "T");
        case!(
            wildcard,
            parser,
            "testatnserialization-testwildcard-bc7511076a",
            "T"
        );
        case!(
            peg_achilles_heel,
            parser,
            "testatnserialization-testpegachillesheel-72c27bc08f",
            "T"
        );
        case!(
            three_alts,
            parser,
            "testatnserialization-test3alts-9c04d047af",
            "T"
        );
        case!(
            simple_loop,
            parser,
            "testatnserialization-testsimpleloop-1e309afd1c",
            "T"
        );
        case!(
            rule_ref,
            parser,
            "testatnserialization-testruleref-2d16d8280e",
            "T"
        );
        case!(
            lexer_two_rules,
            lexer,
            "testatnserialization-testlexertworules-1b3e930083",
            "L"
        );
        case!(
            lexer_unicode_smp_literal_serialized_to_set,
            lexer,
            "testatnserialization-testlexerunicodesmpliteralserializedtoset-e23baf8432",
            "L"
        );
        case!(
            lexer_unicode_smp_range_serialized_to_set,
            lexer,
            "testatnserialization-testlexerunicodesmprangeserializedtoset-500544d4bb",
            "L"
        );
        case!(
            lexer_unicode_smp_and_bmp_set_serialized,
            lexer,
            "testatnserialization-testlexerunicodesmpandbmpsetserialized-146bb39ee2",
            "L"
        );
        case!(
            lexer_with_0xfffc_in_set,
            lexer,
            "testatnserialization-testlexerwith0xfffcinset-710d43c742",
            "L"
        );
        case!(
            lexer_not_literal,
            lexer,
            "testatnserialization-testlexernotliteral-3daee1c629",
            "L"
        );
        case!(
            lexer_range,
            lexer,
            "testatnserialization-testlexerrange-02c072a36f",
            "L"
        );
        case!(
            lexer_eof,
            lexer,
            "testatnserialization-testlexereof-d9c7a74fa3",
            "L"
        );
        case!(
            lexer_eof_in_set,
            lexer,
            "testatnserialization-testlexereofinset-8834d9f1d9",
            "L"
        );
        case!(
            lexer_loops,
            lexer,
            "testatnserialization-testlexerloops-a28bd27385",
            "L"
        );
        case!(
            lexer_action,
            lexer,
            "testatnserialization-testlexeraction-f8a3b073f1",
            "L"
        );
        case!(
            lexer_not_set,
            lexer,
            "testatnserialization-testlexernotset-36d16f4f79",
            "L"
        );
        case!(
            lexer_set_with_range,
            lexer,
            "testatnserialization-testlexersetwithrange-4f6560f50f",
            "L"
        );
        case!(
            lexer_not_set_with_range,
            lexer,
            "testatnserialization-testlexernotsetwithrange-f8651463b0",
            "L"
        );
        case!(
            lexer_unicode_unescaped_bmp_not_set,
            lexer,
            "testatnserialization-testlexerunicodeunescapedbmpnotset-8cc0c75996",
            "L"
        );
        case!(
            lexer_unicode_unescaped_bmp_set_with_range,
            lexer,
            "testatnserialization-testlexerunicodeunescapedbmpsetwithrange-a91b9ab9ec",
            "L"
        );
        case!(
            lexer_unicode_unescaped_bmp_not_set_with_range,
            lexer,
            "testatnserialization-testlexerunicodeunescapedbmpnotsetwithrange-2eab3c760f",
            "L"
        );
        case!(
            lexer_unicode_escaped_bmp_not_set,
            lexer,
            "testatnserialization-testlexerunicodeescapedbmpnotset-d1fbc5a933",
            "L"
        );
        case!(
            lexer_unicode_escaped_bmp_set_with_range,
            lexer,
            "testatnserialization-testlexerunicodeescapedbmpsetwithrange-2f6bdd4701",
            "L"
        );
        case!(
            lexer_unicode_escaped_bmp_not_set_with_range,
            lexer,
            "testatnserialization-testlexerunicodeescapedbmpnotsetwithrange-0cc277891c",
            "L"
        );
        case!(
            lexer_unicode_escaped_smp_not_set,
            lexer,
            "testatnserialization-testlexerunicodeescapedsmpnotset-6be9938ee5",
            "L"
        );
        case!(
            lexer_unicode_escaped_smp_set_with_range,
            lexer,
            "testatnserialization-testlexerunicodeescapedsmpsetwithrange-285ba196a9",
            "L"
        );
        case!(
            lexer_unicode_escaped_smp_not_set_with_range,
            lexer,
            "testatnserialization-testlexerunicodeescapedsmpnotsetwithrange-4f8a23d048",
            "L"
        );
        case!(
            lexer_wildcard_with_mode,
            lexer,
            "testatnserialization-testlexerwildcardwithmode-76c46a8f0f",
            "L"
        );
        case!(
            lexer_not_set_with_range2,
            lexer,
            "testatnserialization-testlexernotsetwithrange2-0eaf17b0b8",
            "L"
        );
        case!(
            mode_in_lexer,
            lexer,
            "testatnserialization-testmodeinlexer-01129db88a",
            "L"
        );
        case!(
            two_modes_in_lexer,
            lexer,
            "testatnserialization-test2modesinlexer-3039cb7f21",
            "L"
        );
    }

    fn assert_lexer_fixture(fixture_name: &str, grammar_name: &str) -> Compilation {
        let compilation =
            compile_lexer_fixture(fixture_name, grammar_name).expect("lexer ATN should compile");
        let fixture = fixture(fixture_name);
        assert_lexer_interp(
            lexer_named(&compilation, grammar_name),
            &fixture.join(format!("{grammar_name}.interp")),
        );
        compilation
    }

    mod upstream_unicode_grammar {
        use super::*;

        macro_rules! case {
            ($name:ident, $fixture:literal, $grammar:literal) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java_interps() {
                        assert_combined_fixture($fixture, $grammar);
                    }
                }
            };
        }

        case!(
            bmp_literal,
            "testunicodegrammar-unicodebmpliteralingrammar-4e3b8e43e6",
            "Unicode"
        );
        case!(
            disabled_surrogate_pair_literal,
            "testunicodegrammar-unicodesurrogatepairliteralingrammar-d1ada97cc5",
            "Unicode"
        );
        case!(
            smp_literal,
            "testunicodegrammar-unicodesmpliteralingrammar-b41d70815f",
            "Unicode"
        );
        case!(
            smp_range,
            "testunicodegrammar-unicodesmprangeingrammar-69d43e47cb",
            "Unicode"
        );
        case!(
            dangling_surrogate,
            "testunicodegrammar-matchingdanglingsurrogateininput-8b7976ab4f",
            "Unicode"
        );
        case!(
            binary,
            "testunicodegrammar-binarygrammar-611ebe1d6f",
            "Binary"
        );
    }

    fn assert_combined_fixture(fixture_name: &str, grammar_name: &str) -> Compilation {
        let compilation = compile_fixture(fixture_name, &[&format!("{grammar_name}.g4")])
            .expect("combined grammar should compile");
        let directory = fixture(fixture_name);
        assert_lexer_interp(
            lexer_named(&compilation, &format!("{grammar_name}Lexer")),
            &directory.join(format!("{grammar_name}Lexer.interp")),
        );
        assert_parser_interp(
            parser_named(&compilation, &format!("{grammar_name}Parser")),
            &directory.join(format!("{grammar_name}.interp")),
        );
        compilation
    }

    fn assert_lexer_interp(compiled: &super::super::lexer::CompiledLexer, expected_path: &Path) {
        let expected = read_atn(expected_path);
        let serialized = SerializedAtn::from_i32(&expected);
        let legacy_atn = AtnDeserializer::new(&serialized)
            .deserialize()
            .expect("Java fixture should deserialize");
        if compiled.runtime_artifact.atn_words != expected {
            let actual = &compiled.runtime_artifact.atn_words;
            let differences = actual
                .iter()
                .zip(&expected)
                .enumerate()
                .filter_map(|(index, (actual, expected))| {
                    (actual != expected).then_some((index, *actual, *expected))
                })
                .take(20)
                .collect::<Vec<_>>();
            panic!(
                "direct lexer ATN differs from Java fixture: actual length {}, expected length {}, first differences {differences:?}, actual rule starts {:?}, expected rule starts {:?}, actual rule stops {:?}, expected rule stops {:?}, actual states per rule {:?}, expected states per rule {:?}, actual differing rule states {:#?}, expected differing rule states {:#?}",
                actual.len(),
                expected.len(),
                compiled.atn.rule_to_start_state(),
                legacy_atn.rule_to_start_state(),
                compiled.atn.rule_to_stop_state(),
                legacy_atn.rule_to_stop_state(),
                lexer_rule_state_counts(&compiled.atn),
                lexer_rule_state_counts(&legacy_atn),
                lexer_rule_states(&compiled.atn, 10),
                lexer_rule_states(&legacy_atn, 10),
            );
        }
        assert_eq!(compiled.atn, legacy_atn);
        assert_eq!(
            CompiledLexerDfa::from_serialized(&compiled.runtime_artifact.dfa_words)
                .expect("direct compiled DFA should round trip")
                .serialize(),
            compiled.runtime_artifact.dfa_words,
        );
        assert_complete_interp(
            &serialize_interp(
                &compiled.semantic.recognizer,
                &compiled.runtime_artifact.atn_words,
            ),
            expected_path,
        );
    }

    fn lexer_rule_state_counts(atn: &antlr4_runtime::atn::LexerAtn) -> Vec<usize> {
        let mut counts = vec![0; atn.rule_to_start_state().len()];
        for state in atn.states() {
            if let Some(rule) = state.rule_index {
                counts[rule] += 1;
            }
        }
        counts
    }

    fn lexer_rule_states(
        atn: &antlr4_runtime::atn::LexerAtn,
        rule: usize,
    ) -> Vec<&antlr4_runtime::atn::LexerAtnState> {
        atn.states()
            .iter()
            .filter(|state| state.rule_index == Some(rule))
            .collect()
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct LexerTokenSnapshot {
        token_type: i32,
        channel: i32,
        byte_start: usize,
        byte_stop: usize,
        text: String,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct LexerStreamSnapshot {
        tokens: Vec<LexerTokenSnapshot>,
        errors: Vec<TokenSourceError>,
        final_mode: i32,
        mode_stack: Vec<i32>,
    }

    #[derive(Clone, Copy)]
    enum LexerStrategy<'a> {
        Interpreted,
        Compiled(&'a CompiledLexerDfa),
    }

    struct DirectTokenSource<'a> {
        base: BaseLexer<InputStream>,
        atn: &'a antlr4_runtime::atn::LexerAtn,
        strategy: LexerStrategy<'a>,
    }

    impl TokenSource for DirectTokenSource<'_> {
        fn next_token(&mut self, sink: &mut TokenSink<'_>) -> Result<TokenId, TokenStoreError> {
            match self.strategy {
                LexerStrategy::Interpreted => next_token_with_hooks(
                    &mut self.base,
                    sink,
                    self.atn,
                    |_, _| {},
                    |_, _| true,
                    |_, _, _| {},
                ),
                LexerStrategy::Compiled(dfa) => next_token_compiled_with_hooks(
                    &mut self.base,
                    sink,
                    self.atn,
                    dfa,
                    |_, _| {},
                    |_, _| true,
                    |_, _, _| {},
                ),
            }
        }

        fn line(&self) -> usize {
            self.base.line()
        }

        fn column(&self) -> usize {
            self.base.column()
        }

        fn source_name(&self) -> &str {
            self.base.source_name()
        }

        fn source_text(&self) -> Option<Rc<str>> {
            self.base.source_text()
        }

        fn drain_errors(&mut self) -> Vec<TokenSourceError> {
            self.base.drain_errors()
        }
    }

    fn assert_lexer_strategies_match(compiled: &super::super::lexer::CompiledLexer, input: &str) {
        let interpreted = lexer_stream(compiled, input, LexerStrategy::Interpreted);
        let accelerated = lexer_stream(compiled, input, LexerStrategy::Compiled(&compiled.dfa));
        assert_eq!(accelerated, interpreted, "input {input:?}");
    }

    fn lexer_stream(
        compiled: &super::super::lexer::CompiledLexer,
        input: &str,
        strategy: LexerStrategy<'_>,
    ) -> LexerStreamSnapshot {
        let recognizer = &compiled.semantic.recognizer;
        let vocabulary = Vocabulary::new(
            recognizer.literal_names.clone(),
            recognizer.symbolic_names.clone(),
            vec![None::<String>; recognizer.symbolic_names.len()],
        );
        let data = RecognizerData::new(recognizer.name.clone(), vocabulary)
            .with_rule_names(recognizer.rule_names.clone())
            .with_channel_names(
                recognizer
                    .channel_names
                    .iter()
                    .map(|name| name.clone().unwrap_or_default()),
            )
            .with_mode_names(recognizer.mode_names.clone());
        let source = DirectTokenSource {
            base: BaseLexer::new(InputStream::with_source_name(input, "direct-test"), data),
            atn: &compiled.atn,
            strategy,
        };
        let mut stream = CommonTokenStream::try_new(source).expect("token stream should fit");
        let tokens = stream
            .tokens()
            .map(|token| LexerTokenSnapshot {
                token_type: token.token_type(),
                channel: token.channel(),
                byte_start: token.start_byte(),
                byte_stop: token.stop_byte(),
                text: token.text().to_owned(),
            })
            .collect::<Vec<_>>();
        assert_eq!(tokens.last().map(|token| token.token_type), Some(TOKEN_EOF));
        let errors = stream.drain_source_errors();
        let source = stream.token_source_mut();
        let final_mode = source.base.mode();
        let mut mode_stack = Vec::new();
        while let Some(mode) = source.base.pop_mode() {
            mode_stack.push(mode);
        }
        LexerStreamSnapshot {
            tokens,
            errors,
            final_mode,
            mode_stack,
        }
    }

    fn assert_parser_fixture(fixture_name: &str, grammar_name: &str) -> Compilation {
        let compilation =
            compile_parser_fixture(fixture_name, grammar_name).expect("parser ATN should compile");
        let fixture = fixture(fixture_name);
        assert_parser_interp(
            parser_named(&compilation, grammar_name),
            &fixture.join(format!("{grammar_name}.interp")),
        );
        compilation
    }

    fn assert_parser_interp(compiled: &super::super::parser::CompiledParser, expected_path: &Path) {
        let expected = read_atn(expected_path);
        let actual = serialize_parser(&compiled.graph);

        assert_eq!(actual, expected);
        let serialized = SerializedAtn::from_i32(&expected);
        let legacy_packed = AtnDeserializer::new(&serialized)
            .deserialize_parser()
            .expect("Java fixture should pack");
        if compiled.packed != legacy_packed {
            let differences = compiled
                .packed
                .packed_words()
                .iter()
                .zip(legacy_packed.packed_words())
                .enumerate()
                .filter_map(|(index, (actual, expected))| {
                    (actual != expected).then_some((index, *actual, *expected))
                })
                .take(20)
                .collect::<Vec<_>>();
            panic!("direct packed ATN differs from Java fixture: {differences:?}");
        }
        assert_complete_interp(
            &serialize_interp(&compiled.semantic.recognizer, &actual),
            expected_path,
        );
    }

    mod upstream_atn_construction {
        use super::*;

        macro_rules! case {
            ($name:ident, $fixture:literal) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_atn_construction_fixture($fixture);
                    }
                }
            };
        }

        macro_rules! partial_case {
            ($name:ident, $fixture:literal) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_partial_atn_construction_fixture($fixture);
                    }
                }
            };
        }

        macro_rules! error_case {
            ($name:ident, $fixture:literal, $code:literal) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_atn_construction_error($fixture, $code);
                    }
                }
            };
        }

        case!(a, "testatnconstruction-testa-44f6db366a");
        case!(ab, "testatnconstruction-testab-a42f21b0b8");
        case!(ab_or_cd, "testatnconstruction-testaborcd-b267bf19a6");
        case!(a_optional, "testatnconstruction-testaoptional-4417b81a74");
        case!(a_or_b, "testatnconstruction-testaorb-e4661cbc08");
        case!(
            a_or_b_optional,
            "testatnconstruction-testaorboptional-5b7e48e9fa"
        );
        partial_case!(
            a_or_b_or_empty_plus,
            "testatnconstruction-testaorboremptyplus-d75dc1c5fb"
        );
        case!(a_or_b_plus, "testatnconstruction-testaorbplus-a9453b1daf");
        case!(a_or_b_star, "testatnconstruction-testaorbstar-da7179ee92");
        case!(
            a_or_b_then_c,
            "testatnconstruction-testaorbthenc-b185264ad6"
        );
        case!(
            a_or_epsilon,
            "testatnconstruction-testaorepsilon-799ca3b396"
        );
        case!(a_plus, "testatnconstruction-testaplus-f4b19d5e31");
        case!(
            a_plus_single_alt_has_plus_ast_pointing_at_loop_back_state,
            "testatnconstruction-testaplussinglealthasplusastpointingatloopbackstate-3c6626e10f"
        );
        case!(a_star, "testatnconstruction-testastar-6226189691");
        case!(ba, "testatnconstruction-testba-addc3f424e");
        case!(char_set, "testatnconstruction-testcharset-3be8423fea");
        case!(
            char_set_range,
            "testatnconstruction-testcharsetrange-b5d5ea2237"
        );
        case!(
            char_set_unicode_bmp_escape,
            "testatnconstruction-testcharsetunicodebmpescape-f297bd0c00"
        );
        case!(
            char_set_unicode_bmp_escape_range,
            "testatnconstruction-testcharsetunicodebmpescaperange-cbdbffbfb6"
        );
        case!(
            char_set_unicode_multiple_property_escape,
            "testatnconstruction-testcharsetunicodemultiplepropertyescape-b3c6b5bbe8"
        );
        case!(
            char_set_unicode_property_escape,
            "testatnconstruction-testcharsetunicodepropertyescape-1ca01ebe06"
        );
        case!(
            char_set_unicode_property_invert_escape,
            "testatnconstruction-testcharsetunicodepropertyinvertescape-993e27b80e"
        );
        case!(
            char_set_unicode_property_overlap,
            "testatnconstruction-testcharsetunicodepropertyoverlap-e358f7dd65"
        );
        case!(
            char_set_unicode_smp_escape,
            "testatnconstruction-testcharsetunicodesmpescape-4527f8d566"
        );
        case!(
            char_set_unicode_smp_escape_range,
            "testatnconstruction-testcharsetunicodesmpescaperange-9416df1cdc"
        );
        case!(
            default_mode,
            "testatnconstruction-testdefaultmode-a515412628"
        );
        case!(
            empty_or_empty,
            "testatnconstruction-testemptyorempty-ba4d562660"
        );
        case!(follow, "testatnconstruction-testfollow-335ed81c22");
        case!(
            repeated_transitions_to_stop_state,
            "testatnconstruction-testforrepeatedtransitionstostopstate-a6e224cf58"
        );
        case!(
            lexer_is_not_set_multi_char_string,
            "testatnconstruction-testlexerisnotsetmulticharstring-25d141ff2e"
        );
        case!(
            lexer_isnt_set_multi_char_string,
            "testatnconstruction-testlexerisntsetmulticharstring-a38c8db90d"
        );
        case!(mode, "testatnconstruction-testmode-19f1fe46af");
        case!(
            nested_a_star,
            "testatnconstruction-testnestedastar-9175a1e843"
        );
        error_case!(
            parser_rule_ref_in_lexer_rule,
            "testatnconstruction-testparserrulerefinlexerrule-34f2000a35",
            "G4S008"
        );
        case!(
            predicated_a_or_b,
            "testatnconstruction-testpredicatedaorb-9fe924cddd"
        );
        case!(range, "testatnconstruction-testrange-22a5123557");
        case!(
            range_or_range,
            "testatnconstruction-testrangeorrange-7cd17abe9a"
        );
        case!(set_a_or_b, "testatnconstruction-testsetaorb-ee6a743346");
        case!(
            set_a_or_b_optional,
            "testatnconstruction-testsetaorboptional-a3d32b77ad"
        );
        case!(
            string_literal_in_parser,
            "testatnconstruction-teststringliteralinparser-4579c9c18c"
        );
    }

    struct GraphOracle {
        recognizer_kind: String,
        recognizer: String,
        interp: String,
        target: String,
        selector: String,
        expected: String,
    }

    fn assert_atn_construction_fixture(fixture_name: &str) {
        let root = fixture_root(fixture_name);
        let compilation = compile_fixture(fixture_name, &[&root])
            .unwrap_or_else(|error| panic!("{fixture_name} should compile: {error:#?}"));
        assert_graph_oracles(fixture_name, &compilation);
    }

    fn assert_partial_atn_construction_fixture(fixture_name: &str) {
        let root = fixture_root(fixture_name);
        let directory = fixture(fixture_name);
        let loaded = load(LoadOptions {
            roots: vec![directory.join(&root)],
            library_directories: Vec::new(),
        })
        .expect("fixture should load");
        let integrated = integrate_loaded(&loaded).expect("fixture should integrate");
        let semantics = analyze(integrated).expect("fixture should pass semantic analysis");
        let grammar = semantics
            .grammars
            .iter()
            .find(|grammar| grammar.unit.name == "P")
            .expect("fixture parser grammar");
        let (graph, _) = super::super::parser::build_graph_for_test(
            grammar,
            semantics.provenance.clone(),
        );
        let oracles = graph_oracles(fixture_name);
        let [oracle] = oracles.as_slice() else {
            panic!("{fixture_name} should have one graph oracle");
        };
        let rule = grammar
            .recognizer
            .rule_names
            .iter()
            .position(|name| name == &oracle.selector)
            .expect("oracle rule exists");
        assert_eq!(
            direct_atn_string(&grammar.recognizer, &graph, graph.rule_starts[rule]),
            oracle.expected,
        );

        let error = compile_fixture(fixture_name, &[&root])
            .expect_err("nullable closure must fail after ATN construction");
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "G4A001")
        );
    }

    fn assert_atn_construction_error(fixture_name: &str, code: &str) {
        let root = fixture_root(fixture_name);
        let error = compile_fixture(fixture_name, &[&root])
            .expect_err("upstream invalid grammar should fail");
        let diagnostic = error
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code == code)
            .unwrap_or_else(|| panic!("missing {code} diagnostic: {error:#?}"));
        assert!(diagnostic.message.contains("parser rule reference"));
        assert!(diagnostic.message.contains("lexer rule"));
    }

    fn assert_graph_oracles(fixture_name: &str, compilation: &Compilation) {
        for oracle in graph_oracles(fixture_name) {
            match oracle.recognizer_kind.as_str() {
                "parser" => {
                    assert_eq!(oracle.target, "rule");
                    let compiled = parser_named(compilation, &oracle.recognizer);
                    assert_parser_interp(compiled, &fixture(fixture_name).join(&oracle.interp));
                    let rule = compiled
                        .semantic
                        .recognizer
                        .rule_names
                        .iter()
                        .position(|name| name == &oracle.selector)
                        .expect("oracle rule exists");
                    assert_eq!(
                        direct_atn_string(
                            &compiled.semantic.recognizer,
                            &compiled.graph,
                            compiled.graph.rule_starts[rule],
                        ),
                        oracle.expected,
                        "{fixture_name} rule {}",
                        oracle.selector,
                    );
                }
                "lexer" => {
                    assert_eq!(oracle.target, "mode");
                    let compiled = lexer_named(compilation, &oracle.recognizer);
                    assert_lexer_interp(compiled, &fixture(fixture_name).join(&oracle.interp));
                    let mode = compiled
                        .semantic
                        .recognizer
                        .mode_names
                        .iter()
                        .position(|name| name == &oracle.selector)
                        .expect("oracle mode exists");
                    let start = compiled
                        .graph
                        .states
                        .iter()
                        .enumerate()
                        .filter(|(_, state)| state.kind == AtnStateKind::TokenStart)
                        .nth(mode)
                        .map(|(index, _)| index)
                        .expect("mode start exists");
                    assert_eq!(
                        direct_atn_string(
                            &compiled.semantic.recognizer,
                            &compiled.graph,
                            start,
                        ),
                        oracle.expected,
                        "{fixture_name} mode {}",
                        oracle.selector,
                    );
                }
                other => panic!("unknown oracle recognizer kind {other}"),
            }
        }
        assert_ast_state_map(fixture_name, compilation);
    }

    fn graph_oracles(fixture_name: &str) -> Vec<GraphOracle> {
        let directory = fixture(fixture_name);
        let index = std::fs::read_to_string(directory.join("oracle/java-atn.index"))
            .expect("fixture ATN oracle index");
        index
            .lines()
            .map(|line| {
                let fields = line.split('\t').collect::<Vec<_>>();
                let [kind, recognizer, interp, target, selector, expected] = fields.as_slice()
                else {
                    panic!("invalid ATN oracle index line {line:?}");
                };
                GraphOracle {
                    recognizer_kind: (*kind).to_owned(),
                    recognizer: (*recognizer).to_owned(),
                    interp: (*interp).to_owned(),
                    target: (*target).to_owned(),
                    selector: (*selector).to_owned(),
                    expected: std::fs::read_to_string(directory.join("oracle").join(expected))
                        .expect("fixture ATN graph"),
                }
            })
            .collect()
    }

    fn fixture_root(fixture_name: &str) -> String {
        let mut roots = std::fs::read_dir(fixture(fixture_name))
            .expect("fixture directory")
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|extension| extension.to_str()) == Some("g4"))
                    .then(|| entry.file_name().to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>();
        roots.sort();
        let [root] = roots.as_slice() else {
            panic!("{fixture_name} should have exactly one grammar root");
        };
        root.clone()
    }

    fn direct_atn_string(
        recognizer: &RecognizerModel,
        graph: &FinalizedAtnGraph,
        start: usize,
    ) -> String {
        let transitions = transitions_by_id(graph);
        let mut work = VecDeque::from([start]);
        let mut marked = BTreeSet::new();
        let mut output = String::new();

        while let Some(state_number) = work.pop_front() {
            if !marked.insert(state_number) {
                continue;
            }
            let state = &graph.states[state_number];
            for transition_id in &state.transitions {
                let transition = transitions
                    .get(transition_id)
                    .expect("state transition exists");
                if state.kind != AtnStateKind::RuleStop {
                    match transition.kind {
                        FinalizedTransitionKind::Rule { follow, .. } => {
                            work.push_back(follow);
                        }
                        _ => work.push_back(transition.target),
                    }
                }

                output.push_str(&state_name(recognizer, graph, state_number));
                match &transition.kind {
                    FinalizedTransitionKind::Epsilon => output.push_str("->"),
                    FinalizedTransitionKind::Rule { rule_index, .. } => {
                        write!(
                            output,
                            "-{}->",
                            recognizer.rule_names[*rule_index]
                        )
                        .expect("writing to String cannot fail");
                    }
                    FinalizedTransitionKind::Predicate {
                        rule_index,
                        predicate_index,
                        ..
                    } => {
                        write!(output, "-pred_{rule_index}:{predicate_index}->")
                            .expect("writing to String cannot fail");
                    }
                    FinalizedTransitionKind::Action {
                        rule_index,
                        action_index,
                        ..
                    } => {
                        let action = action_index.map_or(-1, |index| {
                            i32::try_from(index).expect("action index exceeds i32")
                        });
                        write!(output, "-action_{rule_index}:{action}->")
                            .expect("writing to String cannot fail");
                    }
                    FinalizedTransitionKind::Precedence(precedence) => {
                        write!(output, "-{precedence} >= _p->")
                            .expect("writing to String cannot fail");
                    }
                    FinalizedTransitionKind::Atom(label) => {
                        write!(output, "-{}->", atom_name(recognizer, *label))
                            .expect("writing to String cannot fail");
                    }
                    FinalizedTransitionKind::Range { start, stop } => {
                        write!(output, "-{}->", range_name(recognizer, *start, *stop))
                            .expect("writing to String cannot fail");
                    }
                    FinalizedTransitionKind::Set(ranges) => {
                        write!(output, "-{}->", set_name(recognizer, ranges))
                            .expect("writing to String cannot fail");
                    }
                    FinalizedTransitionKind::NotSet(ranges) => {
                        write!(output, "-~{}->", set_name(recognizer, ranges))
                            .expect("writing to String cannot fail");
                    }
                    FinalizedTransitionKind::Wildcard => output.push_str("-.->"),
                }
                output.push_str(&state_name(recognizer, graph, transition.target));
                output.push('\n');
            }
        }
        output
    }

    fn state_name(
        recognizer: &RecognizerModel,
        graph: &FinalizedAtnGraph,
        state_number: usize,
    ) -> String {
        let state = &graph.states[state_number];
        match state.kind {
            AtnStateKind::StarBlockStart => format!("StarBlockStart_{state_number}"),
            AtnStateKind::PlusBlockStart => format!("PlusBlockStart_{state_number}"),
            AtnStateKind::BlockStart => format!("BlockStart_{state_number}"),
            AtnStateKind::BlockEnd => format!("BlockEnd_{state_number}"),
            AtnStateKind::RuleStart => format!(
                "RuleStart_{}_{}",
                recognizer.rule_names[state.rule_index.expect("rule-start index")], state_number
            ),
            AtnStateKind::RuleStop => format!(
                "RuleStop_{}_{}",
                recognizer.rule_names[state.rule_index.expect("rule-stop index")], state_number
            ),
            AtnStateKind::PlusLoopBack => format!("PlusLoopBack_{state_number}"),
            AtnStateKind::StarLoopBack => format!("StarLoopBack_{state_number}"),
            AtnStateKind::StarLoopEntry => format!("StarLoopEntry_{state_number}"),
            _ => format!("s{state_number}"),
        }
    }

    fn atom_name(recognizer: &RecognizerModel, label: i32) -> String {
        if recognizer.kind == GrammarKind::Lexer {
            antlr_char_literal(label)
        } else {
            token_name(recognizer, label)
        }
    }

    fn range_name(recognizer: &RecognizerModel, start: i32, stop: i32) -> String {
        if recognizer.kind == GrammarKind::Lexer {
            format!("'{}'..'{}'", raw_code_point(start), raw_code_point(stop))
        } else {
            format!("{}..{}", token_name(recognizer, start), token_name(recognizer, stop))
        }
    }

    fn set_name(recognizer: &RecognizerModel, ranges: &[(i32, i32)]) -> String {
        let ranges = normalize_ranges(ranges);
        if recognizer.kind == GrammarKind::Lexer {
            let size = ranges.iter().fold(0_u64, |size, (start, stop)| {
                size.saturating_add(u64::try_from(stop - start + 1).unwrap_or(u64::MAX))
            });
            let values = ranges
                .iter()
                .map(|(start, stop)| {
                    if start == stop {
                        if *start == EOF_TOKEN_TYPE {
                            "<EOF>".to_owned()
                        } else {
                            start.to_string()
                        }
                    } else {
                        format!("{start}..{stop}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            if size > 1 {
                format!("{{{values}}}")
            } else {
                values
            }
        } else {
            let values = ranges
                .iter()
                .flat_map(|(start, stop)| *start..=*stop)
                .map(|token| token_name(recognizer, token))
                .collect::<Vec<_>>();
            if values.len() > 1 {
                format!("{{{}}}", values.join(", "))
            } else {
                values.join("")
            }
        }
    }

    fn token_name(recognizer: &RecognizerModel, token: i32) -> String {
        if token == EOF_TOKEN_TYPE {
            return "EOF".to_owned();
        }
        if token == 0 {
            return "<INVALID>".to_owned();
        }
        let index = usize::try_from(token).ok();
        index
            .and_then(|index| recognizer.literal_names.get(index))
            .and_then(Clone::clone)
            .or_else(|| {
                index
                    .and_then(|index| recognizer.symbolic_names.get(index))
                    .and_then(Clone::clone)
            })
            .unwrap_or_else(|| token.to_string())
    }

    fn antlr_char_literal(value: i32) -> String {
        if value == EOF_TOKEN_TYPE {
            return "EOF".to_owned();
        }
        let escaped = match value {
            0x08 => "\\b".to_owned(),
            0x09 => "\\t".to_owned(),
            0x0A => "\\n".to_owned(),
            0x0C => "\\f".to_owned(),
            0x0D => "\\r".to_owned(),
            0x27 => "\\'".to_owned(),
            0x5C => "\\\\".to_owned(),
            0x20..=0x7E => raw_code_point(value),
            0..=0xFFFF => format!("\\u{value:04X}"),
            _ => raw_code_point(value),
        };
        format!("'{escaped}'")
    }

    fn raw_code_point(value: i32) -> String {
        u32::try_from(value)
            .ok()
            .and_then(char::from_u32)
            .map_or_else(|| value.to_string(), |character| character.to_string())
    }

    fn assert_ast_state_map(fixture_name: &str, compilation: &Compilation) {
        let expected_path = fixture(fixture_name).join("oracle/java-ast-state-map.txt");
        if !expected_path.exists() {
            return;
        }
        let expected = std::fs::read_to_string(expected_path)
            .expect("fixture AST-state map")
            .trim()
            .to_owned();
        let compiled = compilation
            .parsers
            .values()
            .next()
            .expect("AST-state fixture parser");
        let rule = compiled
            .semantic
            .recognizer
            .rule_names
            .iter()
            .position(|name| name == "a")
            .expect("rule a");
        let plus_block = state_in_rule(&compiled.graph, rule, AtnStateKind::PlusBlockStart);
        let plus_loop = state_in_rule(&compiled.graph, rule, AtnStateKind::PlusLoopBack);
        let token = compiled.semantic.recognizer.vocabulary.by_name["A"];
        let atom = compiled
            .graph
            .transitions
            .iter()
            .find(|transition| {
                compiled.graph.states[transition.source].rule_index == Some(rule)
                    && transition.kind == FinalizedTransitionKind::Atom(token)
            })
            .expect("A transition");
        let actual = format!(
            "{{RULE={}, BLOCK={plus_block}, +={plus_loop}, BLOCK={plus_block}, A={}}}",
            compiled.graph.rule_starts[rule], atom.source,
        );
        assert_eq!(actual, expected);

        for state in [
            compiled.graph.rule_starts[rule],
            plus_block,
            plus_loop,
            atom.source,
        ] {
            assert!(
                compiled
                    .provenance
                    .state_origins(compiled.graph.states[state].original)
                    .iter()
                    .any(|origin| matches!(origin, Origin::Authored { .. })),
                "state {state} must retain an authored origin",
            );
        }
        assert!(
            compiled
                .provenance
                .transition_origins(atom.original)
                .iter()
                .any(|origin| matches!(origin, Origin::Authored { .. }))
        );
    }

    fn state_in_rule(
        graph: &FinalizedAtnGraph,
        rule: usize,
        kind: AtnStateKind,
    ) -> usize {
        graph
            .states
            .iter()
            .position(|state| state.rule_index == Some(rule) && state.kind == kind)
            .unwrap_or_else(|| panic!("missing {kind:?} state in rule {rule}"))
    }

    mod upstream_basic_semantic_errors {
        use super::*;
        use crate::grammar::diagnostic::Severity::{Error, Warning};

        #[test]
        fn u_matches_java() {
            assert_basic_semantic_errors(
                "testbasicsemanticerrors-testu-c17a76a27e",
                "U.g4",
                &[
                    expected("G4S014", Warning, 2, 10, "unsupported option foo"),
                    expected("G4S014", Warning, 2, 19, "unsupported option k"),
                    expected(
                        "G4S017",
                        Error,
                        5,
                        8,
                        "token name f must start with an uppercase letter",
                    ),
                    expected("G4S014", Warning, 9, 10, "unsupported option x"),
                    expected(
                        "G4S054",
                        Error,
                        9,
                        0,
                        "repeated grammar prequel spec (options, tokens, or import); please merge",
                    ),
                    expected(
                        "G4S054",
                        Error,
                        8,
                        0,
                        "repeated grammar prequel spec (options, tokens, or import); please merge",
                    ),
                    expected("G4S014", Warning, 12, 10, "unsupported option blech"),
                    expected("G4S014", Warning, 12, 21, "unsupported option greedy"),
                    expected("G4S014", Warning, 15, 16, "unsupported option ick"),
                    expected("G4S014", Warning, 15, 25, "unsupported option greedy"),
                    expected("G4S014", Warning, 16, 16, "unsupported option x"),
                ],
            );
        }

        #[test]
        fn illegal_non_set_label_matches_java() {
            assert_basic_semantic_errors(
                "testbasicsemanticerrors-testillegalnonsetlabel-5c18487902",
                "T.g4",
                &[expected(
                    "G4S055",
                    Error,
                    2,
                    5,
                    "label op assigned to a block which is not a set",
                )],
            );
        }

        #[test]
        fn argument_retval_local_conflicts_match_java() {
            assert_basic_semantic_errors(
                "testbasicsemanticerrors-testargumentretvallocalconflicts-fd702fec44",
                "T.g4",
                &[
                    expected(
                        "G4S056",
                        Error,
                        2,
                        7,
                        "parameter expr conflicts with rule with same name",
                    ),
                    expected(
                        "G4S057",
                        Error,
                        2,
                        26,
                        "return value expr conflicts with rule with same name",
                    ),
                    expected(
                        "G4S058",
                        Error,
                        3,
                        12,
                        "local expr conflicts with rule with same name",
                    ),
                    expected(
                        "G4S059",
                        Error,
                        2,
                        26,
                        "return value expr conflicts with parameter with same name",
                    ),
                    expected(
                        "G4S060",
                        Error,
                        3,
                        12,
                        "local expr conflicts with parameter with same name",
                    ),
                    expected(
                        "G4S061",
                        Error,
                        3,
                        12,
                        "local expr conflicts with return value with same name",
                    ),
                    expected(
                        "G4S038",
                        Error,
                        4,
                        4,
                        "label expr conflicts with rule with same name",
                    ),
                    expected(
                        "G4S062",
                        Error,
                        4,
                        4,
                        "label expr conflicts with parameter with same name",
                    ),
                    expected(
                        "G4S063",
                        Error,
                        4,
                        4,
                        "label expr conflicts with return value with same name",
                    ),
                    expected(
                        "G4S064",
                        Error,
                        4,
                        4,
                        "label expr conflicts with local with same name",
                    ),
                ],
            );
        }

        const fn expected(
            code: &'static str,
            severity: crate::grammar::diagnostic::Severity,
            line: usize,
            column: usize,
            message: &'static str,
        ) -> ExpectedSemanticDiagnostic {
            ExpectedSemanticDiagnostic {
                code,
                severity,
                line,
                column,
                message,
            }
        }
    }

    mod upstream_error_sets {
        use super::*;
        use crate::grammar::diagnostic::Severity::Error;

        #[test]
        fn not_char_set_with_rule_ref_matches_java() {
            assert_basic_semantic_errors(
                "testerrorsets-testnotcharsetwithruleref-9d8ec8db7a",
                "T.g4",
                &[expected(
                    "G4S065",
                    Error,
                    3,
                    10,
                    "rule reference B is not currently supported in a set",
                )],
            );
        }

        #[test]
        fn not_char_set_with_string_matches_java() {
            assert_basic_semantic_errors(
                "testerrorsets-testnotcharsetwithstring-04bc32a04f",
                "T.g4",
                &[expected(
                    "G4S066",
                    Error,
                    3,
                    10,
                    "multi-character literals are not allowed in lexer sets: 'aa'",
                )],
            );
        }

        const fn expected(
            code: &'static str,
            severity: crate::grammar::diagnostic::Severity,
            line: usize,
            column: usize,
            message: &'static str,
        ) -> ExpectedSemanticDiagnostic {
            ExpectedSemanticDiagnostic {
                code,
                severity,
                line,
                column,
                message,
            }
        }
    }

    mod upstream_token_position_options {
        use super::*;
        use crate::grammar::model::{Block, ElementKind, SetElement, Terminal};

        #[test]
        fn left_recursion_rewrite_matches_java() {
            let compilation = assert_combined_fixture(
                "testtokenpositionoptions-testleftrecursionrewrite-0a7598fa91",
            );
            let parser = parser_named(&compilation, "TParser");
            assert_eq!(
                authored_positions(&parser.semantic.unit.rules),
                [
                    "rule:s@11",
                    "e@15",
                    "';'@17",
                    "rule:e@23",
                    "'-'@64",
                    "e@68",
                    "ID@74",
                    "'*'@29",
                    "e@33",
                    "'+'@41",
                    "e@45",
                    "'.'@53",
                    "ID@57",
                ],
            );
            assert!(authored_labels(&parser.semantic.unit.rules).is_empty());
        }

        #[test]
        fn left_recursion_with_labels_matches_java() {
            let compilation = assert_combined_fixture(
                "testtokenpositionoptions-testleftrecursionwithlabels-6e604809f0",
            );
            let parser = parser_named(&compilation, "TParser");
            assert_eq!(
                authored_positions(&parser.semantic.unit.rules),
                [
                    "rule:s@11",
                    "e@15",
                    "';'@17",
                    "rule:e@23",
                    "'-'@68",
                    "e@72",
                    "ID@78",
                    "'*'@29",
                    "e@35",
                    "'+'@43",
                    "e@47",
                    "'.'@55",
                    "ID@61",
                ],
            );
            assert_eq!(
                authored_labels(&parser.semantic.unit.rules),
                ["x@33", "y@59"],
            );
        }

        #[test]
        fn left_recursion_with_set_matches_java() {
            let compilation = assert_combined_fixture(
                "testtokenpositionoptions-testleftrecursionwithset-57f72a753d",
            );
            let parser = parser_named(&compilation, "TParser");
            assert_eq!(
                authored_positions(&parser.semantic.unit.rules),
                [
                    "rule:s@11",
                    "e@15",
                    "';'@17",
                    "rule:e@23",
                    "'-'@73",
                    "e@77",
                    "ID@83",
                    "'*'@33",
                    "'/'@37",
                    "e@42",
                    "'+'@50",
                    "e@54",
                    "'.'@62",
                    "ID@66",
                ],
            );
            assert_eq!(authored_labels(&parser.semantic.unit.rules), ["op@29"]);
        }

        fn assert_combined_fixture(fixture_name: &str) -> Compilation {
            let compilation =
                compile_fixture(fixture_name, &["T.g4"]).expect("combined fixture should compile");
            let path = fixture(fixture_name);
            assert_parser_interp(
                parser_named(&compilation, "TParser"),
                &path.join("T.interp"),
            );
            assert_lexer_interp(
                lexer_named(&compilation, "TLexer"),
                &path.join("TLexer.interp"),
            );
            compilation
        }

        fn authored_positions(rules: &[crate::grammar::model::Rule]) -> Vec<String> {
            let mut positions = Vec::new();
            for rule in rules {
                positions.push(format!("rule:{}@{}", rule.name, rule.span.bytes.start));
                collect_block_positions(&rule.block, &mut positions);
            }
            positions
        }

        fn collect_block_positions(block: &Block, positions: &mut Vec<String>) {
            for alternative in &block.alternatives {
                for element in &alternative.elements {
                    match &element.kind {
                        ElementKind::Terminal(terminal) => positions.push(format!(
                            "{}@{}",
                            terminal_name(terminal),
                            element.span.bytes.start,
                        )),
                        ElementKind::RuleCall(call) => {
                            positions.push(format!("{}@{}", call.name, element.span.bytes.start));
                        }
                        ElementKind::Set { elements, .. } => {
                            for member in elements {
                                if let SetElement::Terminal { value, span, .. } = member {
                                    positions.push(format!(
                                        "{}@{}",
                                        terminal_name(value),
                                        span.bytes.start,
                                    ));
                                }
                            }
                        }
                        ElementKind::Block(nested) => {
                            collect_block_positions(nested, positions);
                        }
                        ElementKind::Range(..)
                        | ElementKind::Action { .. }
                        | ElementKind::Predicate { .. }
                        | ElementKind::Epsilon => {}
                    }
                }
            }
        }

        fn authored_labels(rules: &[crate::grammar::model::Rule]) -> Vec<String> {
            fn collect(block: &Block, labels: &mut Vec<String>) {
                for alternative in &block.alternatives {
                    for element in &alternative.elements {
                        if let Some(label) = &element.label {
                            labels.push(format!("{}@{}", label.name, label.span.bytes.start));
                        }
                        if let ElementKind::Block(nested) = &element.kind {
                            collect(nested, labels);
                        }
                    }
                }
            }

            let mut labels = Vec::new();
            for rule in rules {
                collect(&rule.block, &mut labels);
            }
            labels
        }

        fn terminal_name(terminal: &Terminal) -> &str {
            match terminal {
                Terminal::Token(name) | Terminal::Literal(name) | Terminal::LexerCharSet(name) => {
                    name
                }
                Terminal::Wildcard => ".",
                Terminal::Eof => "EOF",
            }
        }
    }

    struct ExpectedSemanticDiagnostic {
        code: &'static str,
        severity: crate::grammar::diagnostic::Severity,
        line: usize,
        column: usize,
        message: &'static str,
    }

    fn assert_basic_semantic_errors(
        fixture_name: &str,
        root: &str,
        expected: &[ExpectedSemanticDiagnostic],
    ) {
        let error = compile_fixture(fixture_name, &[root])
            .expect_err("upstream invalid grammar should fail semantic analysis");
        assert_eq!(
            error.diagnostics().len(),
            expected.len(),
            "{fixture_name}: {error:#?}",
        );
        let source = std::fs::read_to_string(fixture(fixture_name).join(root))
            .expect("semantic fixture source");
        for (actual, expected) in error.diagnostics().iter().zip(expected) {
            assert_eq!(actual.code, expected.code, "{fixture_name}: {actual:#?}");
            assert_eq!(
                actual.severity, expected.severity,
                "{fixture_name}: {actual:#?}",
            );
            assert_eq!(
                actual.primary.bytes.start,
                fixture_byte_offset(&source, expected.line, expected.column),
                "{fixture_name}: expected {}:{} for {actual:#?}",
                expected.line,
                expected.column,
            );
            assert_eq!(
                actual.message, expected.message,
                "{fixture_name}: {actual:#?}",
            );
        }
    }

    fn fixture_byte_offset(text: &str, line: usize, column: usize) -> u32 {
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

    fn compile_fixture(
        fixture_name: &str,
        roots: &[&str],
    ) -> Result<Compilation, crate::grammar::diagnostic::CompilationError> {
        let fixture = fixture(fixture_name);
        compile(LoadOptions {
            roots: roots.iter().map(|root| fixture.join(root)).collect(),
            library_directories: Vec::new(),
        })
    }

    fn lexer_named<'a>(
        compilation: &'a Compilation,
        grammar_name: &str,
    ) -> &'a super::super::lexer::CompiledLexer {
        compilation
            .lexer_named(grammar_name)
            .unwrap_or_else(|| panic!("fixture should contain lexer grammar {grammar_name}"))
    }

    fn parser_named<'a>(
        compilation: &'a Compilation,
        grammar_name: &str,
    ) -> &'a super::super::parser::CompiledParser {
        compilation
            .parser_named(grammar_name)
            .unwrap_or_else(|| panic!("fixture should contain parser grammar {grammar_name}"))
    }

    fn compile_parser_fixture(
        fixture_name: &str,
        grammar_name: &str,
    ) -> Result<Compilation, crate::grammar::diagnostic::CompilationError> {
        compile_fixture(fixture_name, &[&format!("{grammar_name}.g4")])
    }

    fn compile_lexer_fixture(
        fixture_name: &str,
        grammar_name: &str,
    ) -> Result<Compilation, crate::grammar::diagnostic::CompilationError> {
        compile_fixture(fixture_name, &[&format!("{grammar_name}.g4")])
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/codegen-direct/fixtures")
            .join(name)
    }

    fn read_atn(path: &Path) -> Vec<i32> {
        let text = std::fs::read_to_string(path).expect("fixture interp");
        let values = text
            .split_once("atn:\n[")
            .expect("fixture has ATN section")
            .1
            .trim_end()
            .strip_suffix(']')
            .expect("ATN list is closed");
        values
            .split(',')
            .map(|value| value.trim().parse().expect("integer ATN value"))
            .collect()
    }

    fn assert_complete_interp(actual: &str, expected_path: &Path) {
        let expected = std::fs::read_to_string(expected_path).expect("fixture interp");
        let expected = expected.strip_suffix('\n').unwrap_or(&expected);
        if actual == expected {
            return;
        }
        let first_difference = actual
            .bytes()
            .zip(expected.bytes())
            .position(|(actual, expected)| actual != expected)
            .unwrap_or_else(|| actual.len().min(expected.len()));
        let line = memchr::memchr_iter(
            b'\n',
            &expected.as_bytes()[..first_difference.min(expected.len())],
        )
        .count()
            + 1;
        panic!(
            "complete direct .interp differs from Java fixture {} at byte {} (line {}): actual {:?}, expected {:?}",
            expected_path.display(),
            first_difference,
            line,
            text_context(actual, first_difference),
            text_context(expected, first_difference),
        );
    }

    fn text_context(text: &str, index: usize) -> String {
        let start = index.saturating_sub(40);
        let end = index.saturating_add(80).min(text.len());
        String::from_utf8_lossy(&text.as_bytes()[start..end]).into_owned()
    }
}
