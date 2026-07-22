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

pub(super) fn serialize_interp(recognizer: &RecognizerModel, atn: &[i32]) -> String {
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

    use antlr4_runtime::atn::lexer::{next_token_compiled_with_hooks, next_token_with_hooks};
    use antlr4_runtime::atn::lexer_dfa::CompiledLexerDfa;
    use antlr4_runtime::atn::serialized::{AtnDeserializer, SerializedAtn};
    use antlr4_runtime::lexer::{BaseLexer, Lexer};
    use antlr4_runtime::token::{
        TOKEN_EOF, Token, TokenId, TokenSink, TokenSource, TokenSourceError, TokenStoreError,
    };
    use antlr4_runtime::token_stream::CommonTokenStream;
    use antlr4_runtime::vocabulary::Vocabulary;
    use antlr4_runtime::{
        BaseParser, InputStream, ParserRuntimeOptions, RecognizerData, SemanticHooks,
        UnknownSemanticPolicy,
    };

    use super::*;
    use crate::grammar::compiler::{Compilation, compile};
    use crate::grammar::loader::{LoadOptions, load};
    use crate::grammar::model::LeftRecursiveAlternativeKind;
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

    mod upstream_token_type_assignment {
        use super::*;

        #[derive(Clone, Copy)]
        enum FixtureKind {
            Combined,
            Lexer,
            Parser,
        }

        macro_rules! case {
            ($name:ident, $fixture:literal, $kind:ident) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java_interps_and_tokens() {
                        assert_fixture($fixture, FixtureKind::$kind);
                    }
                }
            };
        }

        case!(
            combined_grammar_literals,
            "testtokentypeassignment-testcombinedgrammarliterals-74842182c1",
            Combined
        );
        case!(
            combined_grammar_with_ref_to_literal_but_no_token_id_ref,
            "testtokentypeassignment-testcombinedgrammarwithreftoliteralbutnotokenidref-fd2391c14b",
            Combined
        );
        case!(
            lexer_tokens_section,
            "testtokentypeassignment-testlexertokenssection-67f7fb02d9",
            Lexer
        );
        case!(
            literal_in_parser_and_lexer,
            "testtokentypeassignment-testliteralinparserandlexer-177a82c119",
            Combined
        );
        case!(
            parser_char_literal_with_basic_unicode_escape,
            "testtokentypeassignment-testparsercharliteralwithbasicunicodeescape-8afd5248f1",
            Combined
        );
        case!(
            parser_char_literal_with_escape,
            "testtokentypeassignment-testparsercharliteralwithescape-15c4d62b48",
            Combined
        );
        case!(
            parser_char_literal_with_extended_unicode_escape,
            "testtokentypeassignment-testparsercharliteralwithextendedunicodeescape-e6f767b0b7",
            Combined
        );
        case!(
            parser_simple_tokens,
            "testtokentypeassignment-testparsersimpletokens-809afdc7eb",
            Parser
        );
        case!(
            parser_tokens_section,
            "testtokentypeassignment-testparsertokenssection-f0930e6dae",
            Parser
        );
        case!(
            pred_does_not_hide_name_to_literal_map_in_lexer,
            "testtokentypeassignment-testpreddoesnothidenametoliteralmapinlexer-a1fc06a563",
            Combined
        );
        case!(
            set_does_not_miss_token_aliases,
            "testtokentypeassignment-testsetdoesnotmisstokenaliases-92cf195953",
            Combined
        );

        fn assert_fixture(fixture_name: &str, kind: FixtureKind) {
            let compilation =
                compile_fixture(fixture_name, &["t.g4"]).expect("fixture should compile");
            let directory = fixture(fixture_name);
            match kind {
                FixtureKind::Combined => {
                    let lexer = lexer_named(&compilation, "tLexer");
                    let parser = parser_named(&compilation, "tParser");
                    assert_lexer_interp(lexer, &directory.join("tLexer.interp"));
                    assert_parser_interp(parser, &directory.join("t.interp"));
                    assert_tokens(&lexer.semantic.recognizer, &directory.join("tLexer.tokens"));
                    assert_tokens(&parser.semantic.recognizer, &directory.join("t.tokens"));
                }
                FixtureKind::Lexer => {
                    let lexer = lexer_named(&compilation, "t");
                    assert_lexer_interp(lexer, &directory.join("t.interp"));
                    assert_tokens(&lexer.semantic.recognizer, &directory.join("t.tokens"));
                }
                FixtureKind::Parser => {
                    let parser = parser_named(&compilation, "t");
                    assert_parser_interp(parser, &directory.join("t.interp"));
                    assert_tokens(&parser.semantic.recognizer, &directory.join("t.tokens"));
                }
            }
        }

        fn assert_tokens(recognizer: &RecognizerModel, expected_path: &Path) {
            let expected = std::fs::read_to_string(expected_path).expect("fixture tokens");
            assert_eq!(serialize_tokens(recognizer), expected);
        }

        fn serialize_tokens(recognizer: &RecognizerModel) -> String {
            let mut output = String::new();
            for name in &recognizer.vocabulary.name_order {
                let number = recognizer.vocabulary.by_name[name];
                writeln!(output, "{name}={number}").expect("writing to String cannot fail");
            }
            for literal in &recognizer.vocabulary.literal_order {
                let number = recognizer.vocabulary.by_literal[literal];
                writeln!(output, "{literal}={number}").expect("writing to String cannot fail");
            }
            output
        }
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

    mod upstream_lookahead_trees {
        use super::*;

        #[test]
        fn alternatives_match_java() {
            let compilation = assert_lookahead_fixture("testlookaheadtrees-testalts-ea8f84416c");
            assert_lookahead_trees(
                &compilation,
                "a.b;",
                "s",
                0,
                0,
                &[("e", "(e:1 a . b)"), ("e", "(e:2 a <error .>)")],
            );
        }

        #[test]
        fn left_recursive_loop_match_java() {
            let compilation = assert_lookahead_fixture("testlookaheadtrees-testalts2-4e81c43326");
            assert_lookahead_trees(
                &compilation,
                "a;",
                "s",
                1,
                1,
                &[
                    ("e", "(e:2 (e:1 a) <error ;>)"),
                    ("s", "(s:1 (e:1 a) ; <EOF>)"),
                ],
            );
        }

        #[test]
        fn include_eof_matches_java() {
            let compilation =
                assert_lookahead_fixture("testlookaheadtrees-testincludeeof-41ef07554a");
            assert_lookahead_trees(
                &compilation,
                "a.b",
                "s",
                0,
                0,
                &[("e", "(e:1 a . b <EOF>)"), ("e", "(e:2 a . b <EOF>)")],
            );
        }

        #[test]
        fn calls_left_recursive_rule_match_java() {
            let compilation =
                assert_lookahead_fixture("testlookaheadtrees-testcallleftrecursiverule-410ec32fb8");
            assert_lookahead_trees(
                &compilation,
                "x;!",
                "s",
                0,
                0,
                &[("a", "(a:1 (e:4 x) ;)"), ("a", "(a:2 x ;)")],
            );
            assert_lookahead_trees(
                &compilation,
                "x+1;!",
                "s",
                2,
                1,
                &[
                    ("e", "(e:1 (e:4 x) <error +>)"),
                    ("e", "(e:2 (e:4 x) + (e:5 1))"),
                    ("e", "(e:3 (e:4 x) <error +>)"),
                ],
            );
        }

        fn assert_lookahead_fixture(fixture_name: &str) -> Compilation {
            let compilation = compile_fixture(fixture_name, &["L.g4", "T.g4"])
                .expect("lookahead grammar should compile");
            assert_lexer_interp(
                lexer_named(&compilation, "L"),
                &fixture(fixture_name).join("L.interp"),
            );
            assert_parser_interp(
                parser_named(&compilation, "T"),
                &fixture(fixture_name).join("T.interp"),
            );
            assert!(compilation.diagnostics.is_empty());
            compilation
        }

        fn assert_lookahead_trees(
            compilation: &Compilation,
            input: &str,
            start_rule: &str,
            decision: usize,
            decision_input: usize,
            expected: &[(&str, &str)],
        ) {
            let lexer = lexer_named(compilation, "L");
            let compiled = parser_named(compilation, "T");
            let decision_state = compiled.graph.decisions[decision];
            assert_eq!(
                compiled.graph.states[decision_state].transitions.len(),
                expected.len(),
            );
            let start_rule = compiled
                .semantic
                .recognizer
                .rule_names
                .iter()
                .position(|name| name == start_rule)
                .expect("upstream start rule should exist");

            let left_recursive_alt_numbers = left_recursive_alt_numbers(compiled);
            for (alternative, &(expected_rule, expected_tree)) in expected.iter().enumerate() {
                let hooks = ForcedDecisionHooks {
                    decision,
                    input_index: decision_input,
                    alternative: alternative + 1,
                    reached: false,
                };
                let source = DirectTokenSource {
                    base: BaseLexer::new(
                        InputStream::with_source_name(input, "lookahead-test"),
                        recognizer_data(&lexer.semantic.recognizer),
                    ),
                    atn: &lexer.atn,
                    strategy: LexerStrategy::Interpreted,
                };
                let stream = CommonTokenStream::try_new(source)
                    .expect("lookahead input token stream should fit");
                let mut parser = BaseParser::with_semantic_hooks(
                    stream,
                    recognizer_data(&compiled.semantic.recognizer),
                    hooks,
                );
                let expected_rule = compiled
                    .semantic
                    .recognizer
                    .rule_names
                    .iter()
                    .position(|name| name == expected_rule)
                    .expect("expected subtree rule should exist");
                let (tree, _) = parser
                    .parse_atn_rule_with_runtime_options(
                        &compiled.packed,
                        start_rule,
                        ParserRuntimeOptions {
                            track_alt_numbers: true,
                            unknown_predicate_policy: UnknownSemanticPolicy::AssumeFalse,
                            ..ParserRuntimeOptions::default()
                        },
                    )
                    .unwrap_or_else(|error| {
                        panic!(
                            "decision {decision}, alternative {} should produce a tree: {error:?}",
                            alternative + 1,
                        )
                    });
                let subtree = parser
                    .node(tree)
                    .first_rule(expected_rule)
                    .expect("expected subtree should exist");
                assert_eq!(
                    lookahead_tree_string(
                        subtree,
                        &compiled.semantic.recognizer.rule_names,
                        &left_recursive_alt_numbers,
                    ),
                    expected_tree,
                    "decision {decision}, alternative {}",
                    alternative + 1,
                );
            }
        }

        #[derive(Debug)]
        struct ForcedDecisionHooks {
            decision: usize,
            input_index: usize,
            alternative: usize,
            reached: bool,
        }

        impl SemanticHooks for ForcedDecisionHooks {
            fn observes_parser_decisions(&self) -> bool {
                true
            }

            fn parser_decision_override(
                &mut self,
                decision: usize,
                input_index: usize,
                alternative_count: usize,
            ) -> Option<usize> {
                if self.reached || decision != self.decision || input_index != self.input_index {
                    return None;
                }
                assert!(self.alternative <= alternative_count);
                self.reached = true;
                Some(self.alternative)
            }
        }

        #[derive(Debug)]
        struct LeftRecursiveAltNumbers {
            primary: Vec<usize>,
            operator: Vec<usize>,
        }

        fn left_recursive_alt_numbers(
            compiled: &super::super::super::parser::CompiledParser,
        ) -> Vec<Option<LeftRecursiveAltNumbers>> {
            let mut mappings = (0..compiled.semantic.recognizer.rule_names.len())
                .map(|_| None)
                .collect::<Vec<_>>();
            for rule in &compiled.semantic.unit.rules {
                let Some(info) = &rule.left_recursion else {
                    continue;
                };
                let rule_index = compiled.semantic.recognizer.rule_numbers[&rule.id];
                let mut primary = Vec::new();
                let mut operator = Vec::new();
                for (index, kind) in info.alternative_kinds.values().enumerate() {
                    let original_alt_number = index + 1;
                    match kind {
                        LeftRecursiveAlternativeKind::Primary
                        | LeftRecursiveAlternativeKind::Prefix => {
                            primary.push(original_alt_number);
                        }
                        LeftRecursiveAlternativeKind::Binary
                        | LeftRecursiveAlternativeKind::Suffix => {
                            operator.push(original_alt_number);
                        }
                    }
                }
                mappings[rule_index] = Some(LeftRecursiveAltNumbers { primary, operator });
            }
            mappings
        }

        fn recognizer_data(recognizer: &RecognizerModel) -> RecognizerData {
            RecognizerData::new(
                recognizer.name.clone(),
                Vocabulary::new(
                    recognizer.literal_names.clone(),
                    recognizer.symbolic_names.clone(),
                    vec![None::<String>; recognizer.symbolic_names.len()],
                ),
            )
            .with_rule_names(recognizer.rule_names.clone())
        }

        fn lookahead_tree_string(
            node: antlr4_runtime::tree::Node<'_>,
            rule_names: &[String],
            left_recursive_alt_numbers: &[Option<LeftRecursiveAltNumbers>],
        ) -> String {
            render_lookahead_tree(node, rule_names, left_recursive_alt_numbers).0
        }

        fn render_lookahead_tree(
            node: antlr4_runtime::tree::Node<'_>,
            rule_names: &[String],
            left_recursive_alt_numbers: &[Option<LeftRecursiveAltNumbers>],
        ) -> (String, bool) {
            if let Some(rule) = node.as_rule() {
                let name = &rule_names[rule.rule_index()];
                let alt_number = original_alt_number(rule, left_recursive_alt_numbers);
                let display_name = if alt_number == 0 {
                    name.clone()
                } else {
                    format!("{name}:{alt_number}")
                };
                if rule.child_count() == 0 {
                    return (display_name, false);
                }
                let mut children = Vec::new();
                let mut hit_error = false;
                for child in rule.children() {
                    let (child, child_hit_error) =
                        render_lookahead_tree(child, rule_names, left_recursive_alt_numbers);
                    children.push(child);
                    if child_hit_error {
                        hit_error = true;
                        break;
                    }
                }
                return (
                    format!("({display_name} {})", children.join(" ")),
                    hit_error,
                );
            }
            if node.as_error().is_some() {
                return (format!("<error {}>", node.text()), true);
            }
            (node.to_string_tree_with_names::<String>(&[]), false)
        }

        fn original_alt_number(
            rule: antlr4_runtime::tree::RuleNodeView<'_>,
            mappings: &[Option<LeftRecursiveAltNumbers>],
        ) -> usize {
            let rewritten_alt_number = rule.alt_number();
            let Some(mapping) = &mappings[rule.rule_index()] else {
                return rewritten_alt_number;
            };
            let starts_with_same_rule = rule
                .children()
                .next()
                .and_then(antlr4_runtime::tree::Node::as_rule)
                .is_some_and(|child| child.rule_index() == rule.rule_index());
            let authored_alt_numbers = if starts_with_same_rule {
                &mapping.operator
            } else {
                &mapping.primary
            };
            if rewritten_alt_number == 0 && authored_alt_numbers.len() == 1 {
                return authored_alt_numbers[0];
            }
            authored_alt_numbers
                .get(rewritten_alt_number.saturating_sub(1))
                .copied()
                .unwrap_or(rewritten_alt_number)
        }
    }

    mod upstream_left_recursion_tool_issues {
        use super::*;
        use crate::grammar::diagnostic::Severity::Error;

        macro_rules! error_case {
            ($name:ident, $fixture:literal, $code:literal, $line:literal, $message:literal) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java_diagnostic() {
                        assert_diagnostic($fixture, $code, $line, $message);
                    }
                }
            };
        }

        error_case!(
            no_non_left_recursive_alternative,
            "testleftrecursiontoolissues-testcheckfornonleftrecursiverule-477f42142e",
            "G4R002",
            3,
            "left recursive rule a must contain an alternative which is not left recursive"
        );
        error_case!(
            empty_left_recursive_follow,
            "testleftrecursiontoolissues-testcheckforleftrecursiveemptyfollow-558283d55a",
            "G4A002",
            3,
            "left recursive rule a contains a left recursive alternative which can be followed by the empty string"
        );
        error_case!(
            recursive_rule_reference_with_argument,
            "testleftrecursiontoolissues-testleftrecursiverulerefwitharg-40cd52608d",
            "G4R001",
            6,
            "rule expressionA is left recursive but doesn't conform to a pattern ANTLR can handle"
        );
        error_case!(
            recursive_rule_reference_with_argument_and_parameter,
            "testleftrecursiontoolissues-testleftrecursiverulerefwitharg2-7332bdbd4f",
            "G4R001",
            2,
            "rule a is left recursive but doesn't conform to a pattern ANTLR can handle"
        );
        error_case!(
            recursive_rule_reference_with_argument_without_parameter,
            "testleftrecursiontoolissues-testleftrecursiverulerefwitharg3-719e121a92",
            "G4R001",
            2,
            "rule a is left recursive but doesn't conform to a pattern ANTLR can handle"
        );
        error_case!(
            isolated_left_recursive_rule_reference,
            "testleftrecursiontoolissues-testisolatedleftrecursiveruleref-43f8252e7d",
            "G4R001",
            2,
            "rule a is left recursive but doesn't conform to a pattern ANTLR can handle"
        );

        mod argument_on_primary_rule {
            use super::*;

            #[test]
            fn matches_java_interps() {
                let compilation = assert_combined_fixture(
                    "testleftrecursiontoolissues-testargonprimaryruleinleftrecursiverule-e2b3d25b22",
                    "T",
                );
                assert!(compilation.diagnostics.is_empty());
            }
        }

        fn assert_diagnostic(fixture_name: &str, code: &str, line: usize, message: &str) {
            let error = compile_fixture(fixture_name, &["T.g4"])
                .expect_err("upstream invalid grammar should fail");
            let [diagnostic] = error.diagnostics() else {
                panic!("{fixture_name} should report exactly one diagnostic: {error:#?}");
            };
            assert_eq!(diagnostic.code, code, "{fixture_name}: {diagnostic:#?}");
            assert_eq!(
                diagnostic.severity, Error,
                "{fixture_name}: {diagnostic:#?}",
            );
            assert_eq!(
                diagnostic.message, message,
                "{fixture_name}: {diagnostic:#?}",
            );
            let source = std::fs::read_to_string(fixture(fixture_name).join("T.g4"))
                .expect("fixture source");
            assert_eq!(
                diagnostic.primary.bytes.start,
                fixture_byte_offset(&source, line, 0),
                "{fixture_name}: {diagnostic:#?}",
            );
        }
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
        let semantics =
            analyze(&loaded.sources, integrated).expect("fixture should pass semantic analysis");
        let grammar = semantics
            .grammars
            .iter()
            .find(|grammar| grammar.unit.name == "P")
            .expect("fixture parser grammar");
        let (graph, _) =
            super::super::parser::build_graph_for_test(grammar, semantics.provenance.clone());
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
                        direct_atn_string(&compiled.semantic.recognizer, &compiled.graph, start,),
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
                        write!(output, "-{}->", recognizer.rule_names[*rule_index])
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
                recognizer.rule_names[state.rule_index.expect("rule-start index")],
                state_number
            ),
            AtnStateKind::RuleStop => format!(
                "RuleStop_{}_{}",
                recognizer.rule_names[state.rule_index.expect("rule-stop index")],
                state_number
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
            format!(
                "{}..{}",
                token_name(recognizer, start),
                token_name(recognizer, stop)
            )
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

    fn state_in_rule(graph: &FinalizedAtnGraph, rule: usize, kind: AtnStateKind) -> usize {
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

    mod upstream_attribute_checks {
        use super::*;

        macro_rules! case {
            (
                $name:ident,
                $fixture:literal,
                $root:literal,
                $expects_error:literal,
                [$($expected:expr),* $(,)?]
            ) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_attribute_fixture(
                            $fixture,
                            $root,
                            $expects_error,
                            &[$($expected),*],
                        );
                    }
                }
            };
        }

        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/codegen-direct/generated/attribute-checks-cases.inc.rs"
        ));

        fn assert_attribute_fixture(
            fixture_name: &str,
            root: &str,
            expects_error: bool,
            expected: &[ExpectedDiagnostic],
        ) {
            let directory = fixture(fixture_name);

            match compile_fixture(fixture_name, &[root]) {
                Ok(compilation) => {
                    assert!(!expects_error, "{fixture_name}: expected semantic failure");
                    assert_diagnostics(fixture_name, root, &compilation.diagnostics, expected);
                    let grammar_name = root
                        .strip_suffix(".g4")
                        .expect("attribute fixture root ends in .g4");
                    let parser = parser_named(&compilation, grammar_name);
                    assert_parser_interp(parser, &directory.join(format!("{grammar_name}.interp")));
                    assert_tokens(
                        &parser.semantic.recognizer,
                        &directory.join(format!("{grammar_name}.tokens")),
                    );
                }
                Err(error) => {
                    assert!(expects_error, "{fixture_name}: {error:#?}");
                    assert_diagnostics(fixture_name, root, error.diagnostics(), expected);
                }
            }
        }

        struct ExpectedDiagnostic {
            code: &'static str,
            line: usize,
            column: usize,
            message: &'static str,
        }

        const fn expected(
            code: &'static str,
            line: usize,
            column: usize,
            message: &'static str,
        ) -> ExpectedDiagnostic {
            ExpectedDiagnostic {
                code,
                line,
                column,
                message,
            }
        }

        fn assert_diagnostics(
            fixture_name: &str,
            root: &str,
            actual: &[crate::grammar::diagnostic::Diagnostic],
            expected: &[ExpectedDiagnostic],
        ) {
            assert_eq!(actual.len(), expected.len(), "{fixture_name}: {actual:#?}");
            let source = std::fs::read_to_string(fixture(fixture_name).join(root))
                .expect("attribute fixture source");
            for (actual, expected) in actual.iter().zip(expected) {
                assert_eq!(actual.code, expected.code, "{fixture_name}: {actual:#?}");
                assert_eq!(
                    actual.severity,
                    crate::grammar::diagnostic::Severity::Error,
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

        fn assert_tokens(recognizer: &RecognizerModel, expected_path: &Path) {
            let expected = std::fs::read_to_string(expected_path).expect("fixture tokens");
            let mut actual = String::new();
            for name in &recognizer.vocabulary.name_order {
                let number = recognizer.vocabulary.by_name[name];
                writeln!(actual, "{name}={number}").expect("writing to String cannot fail");
            }
            for literal in &recognizer.vocabulary.literal_order {
                let number = recognizer.vocabulary.by_literal[literal];
                writeln!(actual, "{literal}={number}").expect("writing to String cannot fail");
            }
            assert_eq!(actual, expected, "{}", expected_path.display());
        }
    }

    mod upstream_symbol_issues {
        use super::*;
        use crate::grammar::diagnostic::Severity::{Error, Warning};

        #[derive(Clone, Copy)]
        enum FixtureKind {
            Combined,
            Lexer,
            Parser,
        }

        macro_rules! case {
            (
                $name:ident,
                $fixture:literal,
                $root:literal,
                $kind:ident,
                [$($expected:expr),* $(,)?]
            ) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_symbol_fixture(
                            $fixture,
                            $root,
                            FixtureKind::$kind,
                            &[$($expected),*],
                        );
                    }
                }
            };
        }

        case!(
            a,
            "testsymbolissues-testa-2e644f226d",
            "A.g4",
            Combined,
            [
                at("G4S016", Error, 5, 1),
                at("G4S016", Error, 7, 1),
                at("G4S014", Warning, 2, 10),
                at("G4S014", Warning, 2, 21),
                at("G4S016", Error, 5, 1),
                at("G4S030", Warning, 9, 27),
                at("G4S030", Warning, 10, 20),
                at("G4S030", Warning, 11, 4),
                at("G4S042", Error, 9, 37),
                at("G4S043", Error, 10, 31),
            ]
        );
        case!(
            b,
            "testsymbolissues-testb-9ccf14c21c",
            "B.g4",
            Parser,
            [
                at("G4S038", Error, 4, 4),
                at("G4S038", Error, 4, 9),
                at("G4S039", Error, 4, 15),
                at("G4S041", Error, 6, 9),
                at("G4S031", Error, 4, 20),
            ]
        );
        case!(
            case_insensitive_chars_collision,
            "testsymbolissues-testcaseinsensitivecharscollision-1c64211182",
            "L.g4",
            Lexer,
            [
                at("G4S068", Warning, 3, 18),
                at("G4S068", Warning, 4, 32),
                unlocated("G4S068", Warning),
                unlocated("G4S068", Warning),
            ]
        );
        case!(
            case_insensitive_option_in_parser_rule,
            "testsymbolissues-testcaseinsensitiveoptioninparserule-1827b66149",
            "G.g4",
            Combined,
            [at("G4S014", Warning, 2, 15)]
        );
        case!(
            case_insensitive_with_unicode_ranges,
            "testsymbolissues-testcaseinsensitivewithunicoderanges-abbbfc3fea",
            "L.g4",
            Lexer,
            []
        );
        case!(
            chars_collision,
            "testsymbolissues-testcharscollision-2c32d921bf",
            "L.g4",
            Lexer,
            [
                at("G4S068", Warning, 2, 18),
                at("G4S068", Warning, 3, 18),
                at("G4S068", Warning, 4, 38),
                unlocated("G4S068", Warning),
                unlocated("G4S068", Warning),
            ]
        );
        case!(
            d,
            "testsymbolissues-testd-8d77089073",
            "D.g4",
            Parser,
            [at("G4S062", Error, 4, 21), at("G4S059", Error, 6, 22),]
        );
        case!(
            duplicated_commands,
            "testsymbolissues-testduplicatedcommands-e809cd0730",
            "Lexer.g4",
            Lexer,
            [
                at("G4S046", Warning, 4, 27),
                at("G4S046", Warning, 12, 34),
                at("G4S046", Warning, 13, 40),
                at("G4S046", Warning, 13, 59),
            ]
        );
        case!(
            e,
            "testsymbolissues-teste-cd9e1fc41d",
            "E.g4",
            Combined,
            [at("G4S019", Warning, 3, 4)]
        );
        case!(
            empty_lexer_mode_detection,
            "testsymbolissues-testemptylexermodedetection-1e410b0d53",
            "L.g4",
            Lexer,
            [at("G4S026", Error, 3, 5)]
        );
        case!(
            empty_lexer_rule_detection,
            "testsymbolissues-testemptylexerruledetection-f95b017788",
            "L.g4",
            Lexer,
            [at("G4A006", Warning, 3, 0), at("G4A006", Warning, 5, 2),]
        );
        case!(
            f,
            "testsymbolissues-testf-d585f92343",
            "F.g4",
            Lexer,
            [at("G4S034", Error, 3, 0)]
        );
        case!(
            illegal_case_insensitive_option_value,
            "testsymbolissues-testillegalcaseinsensitiveoptionvalue-3c5253859c",
            "L.g4",
            Lexer,
            [at("G4S015", Warning, 2, 28), at("G4S015", Warning, 3, 36),]
        );
        case!(
            incompatible_commands,
            "testsymbolissues-testincompatiblecommands-43fa4a3fd8",
            "L.g4",
            Lexer,
            [
                at("G4S047", Warning, 5, 20),
                at("G4S047", Warning, 6, 20),
                at("G4S047", Warning, 7, 20),
                at("G4S047", Warning, 8, 20),
                at("G4S047", Warning, 9, 20),
                at("G4S047", Warning, 10, 20),
                at("G4S047", Warning, 11, 27),
                at("G4S047", Warning, 12, 27),
                at("G4S047", Warning, 13, 33),
                at("G4S047", Warning, 14, 33),
            ]
        );
        case!(
            labels_for_tokens_with_mixed_types,
            "testsymbolissues-testlabelsfortokenswithmixedtypes-0a6a086afc",
            "L.g4",
            Combined,
            [
                at("G4S041", Error, 8, 13),
                at("G4S041", Error, 11, 15),
                at("G4S041", Error, 24, 0),
                at("G4S041", Error, 24, 0),
                at("G4S041", Error, 24, 0),
            ]
        );
        case!(
            labels_for_tokens_with_mixed_types_lr_with_labels,
            "testsymbolissues-testlabelsfortokenswithmixedtypeslrwithlabels-5841a22629",
            "L.g4",
            Combined,
            []
        );
        case!(
            labels_for_tokens_with_mixed_types_lr_without_labels,
            "testsymbolissues-testlabelsfortokenswithmixedtypeslrwithoutlabels-15b35eab8e",
            "L.g4",
            Combined,
            [at("G4S041", Error, 3, 0), at("G4S041", Error, 3, 0),]
        );
        case!(
            not_implied_characters,
            "testsymbolissues-testnotimpliedcharacters-4c3481ae89",
            "Test.g4",
            Lexer,
            [at("G4S070", Warning, 2, 8), at("G4S070", Warning, 3, 8),]
        );
        case!(
            not_implied_characters_with_case_insensitive_option,
            "testsymbolissues-testnotimpliedcharacterswithcaseinsensitiveoption-f08fa6ab47",
            "Test.g4",
            Lexer,
            [at("G4S070", Warning, 3, 7)]
        );
        case!(
            redundant_case_insensitive_lexer_rule_option_true,
            "testsymbolissues-testredundantcaseinsensitivelexerruleoption-a9ec701b5c",
            "L.g4",
            Lexer,
            [at("G4S067", Warning, 3, 16)]
        );
        case!(
            redundant_case_insensitive_lexer_rule_option_false,
            "testsymbolissues-testredundantcaseinsensitivelexerruleoption-a9ec701b5c-variant-2",
            "L.g4",
            Lexer,
            [at("G4S067", Warning, 3, 16)]
        );
        case!(
            string_literal_redefinitions,
            "testsymbolissues-teststringliteralredefs-9b1f541902",
            "L.g4",
            Lexer,
            []
        );
        case!(
            declaration_conflicts_with_reserved_names,
            "testsymbolissues-testtokensmodeschannelsdeclarationconflictswithreserved-9319caf796",
            "L.g4",
            Lexer,
            [
                at("G4S003", Error, 5, 0),
                at("G4S024", Error, 4, 0),
                at("G4S021", Error, 2, 11),
                at("G4S021", Error, 2, 17),
            ]
        );
        case!(
            command_arguments_conflict_with_reserved_names,
            "testsymbolissues-testtokensmodeschannelsusingconflictswithreserved-c8a93f7227",
            "L.g4",
            Lexer,
            [
                at("G4S021", Error, 2, 18),
                at("G4S018", Error, 3, 15),
                at("G4S024", Error, 4, 15),
            ]
        );
        case!(
            undefined_label_regression,
            "testsymbolissues-testundefinedlabel-d2fa215436",
            "Test.g4",
            Combined,
            [at("G4S042", Error, 3, 6)]
        );
        case!(
            unreachable_tokens,
            "testsymbolissues-testunreachabletokens-43df5f5f59",
            "Test.g4",
            Lexer,
            [
                at("G4S069", Warning, 4, 0),
                at("G4S069", Warning, 5, 0),
                at("G4S069", Warning, 7, 0),
                at("G4S069", Warning, 7, 0),
                at("G4S069", Warning, 9, 0),
                at("G4S069", Warning, 11, 0),
                at("G4S069", Warning, 12, 0),
                at("G4S069", Warning, 13, 0),
                at("G4S069", Warning, 13, 0),
            ]
        );
        case!(
            wrong_id_for_type_channel_mode_command,
            "testsymbolissues-testwrongidfortypechannelmodecommand-5245d6d5c3",
            "L.g4",
            Lexer,
            [
                at("G4S052", Error, 4, 22),
                at("G4S053", Error, 4, 41),
                at("G4S051", Error, 4, 54),
            ]
        );

        #[derive(Clone, Copy)]
        struct ExpectedDiagnostic {
            code: &'static str,
            severity: crate::grammar::diagnostic::Severity,
            position: Option<(usize, usize)>,
        }

        const fn at(
            code: &'static str,
            severity: crate::grammar::diagnostic::Severity,
            line: usize,
            column: usize,
        ) -> ExpectedDiagnostic {
            ExpectedDiagnostic {
                code,
                severity,
                position: Some((line, column)),
            }
        }

        const fn unlocated(
            code: &'static str,
            severity: crate::grammar::diagnostic::Severity,
        ) -> ExpectedDiagnostic {
            ExpectedDiagnostic {
                code,
                severity,
                position: None,
            }
        }

        fn assert_symbol_fixture(
            fixture_name: &str,
            root: &str,
            kind: FixtureKind,
            expected: &[ExpectedDiagnostic],
        ) {
            let expects_error = expected
                .iter()
                .any(|diagnostic| diagnostic.severity == Error);
            match compile_fixture(fixture_name, &[root]) {
                Ok(compilation) => {
                    assert!(!expects_error, "{fixture_name}: expected semantic failure");
                    assert_diagnostics(fixture_name, root, &compilation.diagnostics, expected);
                    assert_artifacts(fixture_name, root, kind, &compilation);
                }
                Err(error) => {
                    assert!(expects_error, "{fixture_name}: {error:#?}");
                    assert_diagnostics(fixture_name, root, error.diagnostics(), expected);
                }
            }
        }

        fn assert_diagnostics(
            fixture_name: &str,
            root: &str,
            actual: &[crate::grammar::diagnostic::Diagnostic],
            expected: &[ExpectedDiagnostic],
        ) {
            assert_eq!(actual.len(), expected.len(), "{fixture_name}: {actual:#?}");
            let source = std::fs::read_to_string(fixture(fixture_name).join(root))
                .expect("symbol fixture source");
            for (actual, expected) in actual.iter().zip(expected) {
                assert_eq!(actual.code, expected.code, "{fixture_name}: {actual:#?}");
                assert_eq!(
                    actual.severity, expected.severity,
                    "{fixture_name}: {actual:#?}",
                );
                if let Some((line, column)) = expected.position {
                    assert_eq!(
                        actual.primary.bytes.start,
                        fixture_byte_offset(&source, line, column),
                        "{fixture_name}: expected {line}:{column} for {actual:#?}",
                    );
                }
            }
        }

        fn assert_artifacts(
            fixture_name: &str,
            root: &str,
            kind: FixtureKind,
            compilation: &Compilation,
        ) {
            let directory = fixture(fixture_name);
            let grammar_name = root
                .strip_suffix(".g4")
                .expect("symbol fixture root ends in .g4");
            match kind {
                FixtureKind::Lexer => {
                    let lexer = lexer_named(compilation, grammar_name);
                    assert_lexer_interp(lexer, &directory.join(format!("{grammar_name}.interp")));
                    assert_tokens(
                        &lexer.semantic.recognizer,
                        &directory.join(format!("{grammar_name}.tokens")),
                    );
                }
                FixtureKind::Parser => {
                    let parser = parser_named(compilation, grammar_name);
                    assert_parser_interp(parser, &directory.join(format!("{grammar_name}.interp")));
                    assert_tokens(
                        &parser.semantic.recognizer,
                        &directory.join(format!("{grammar_name}.tokens")),
                    );
                }
                FixtureKind::Combined => {
                    let parser = parser_named(compilation, &format!("{grammar_name}Parser"));
                    assert_parser_interp(parser, &directory.join(format!("{grammar_name}.interp")));
                    assert_tokens(
                        &parser.semantic.recognizer,
                        &directory.join(format!("{grammar_name}.tokens")),
                    );
                    let lexer_interp = directory.join(format!("{grammar_name}Lexer.interp"));
                    if lexer_interp.is_file() {
                        let lexer = lexer_named(compilation, &format!("{grammar_name}Lexer"));
                        assert_lexer_interp(lexer, &lexer_interp);
                        assert_tokens(
                            &lexer.semantic.recognizer,
                            &directory.join(format!("{grammar_name}Lexer.tokens")),
                        );
                    }
                }
            }
        }

        fn assert_tokens(recognizer: &RecognizerModel, expected_path: &Path) {
            let expected = std::fs::read_to_string(expected_path).expect("fixture tokens");
            let mut actual = String::new();
            for name in &recognizer.vocabulary.name_order {
                let number = recognizer.vocabulary.by_name[name];
                writeln!(actual, "{name}={number}").expect("writing to String cannot fail");
            }
            for literal in &recognizer.vocabulary.literal_order {
                let number = recognizer.vocabulary.by_literal[literal];
                writeln!(actual, "{literal}={number}").expect("writing to String cannot fail");
            }
            assert_eq!(actual, expected, "{}", expected_path.display());
        }
    }

    #[allow(clippy::too_many_arguments)]
    mod upstream_composite_grammars {
        use super::*;
        use crate::grammar::diagnostic::Severity::{Error, Warning};

        macro_rules! case {
            (
                $name:ident,
                $fixture:literal,
                [$($root:literal),* $(,)?],
                [$($library:literal),* $(,)?],
                $expects_error:literal,
                [$($source:literal),* $(,)?],
                [$($expected:expr),* $(,)?],
                [$($artifact:expr),* $(,)?]
            ) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_composite_fixture(
                            $fixture,
                            &[$($root),*],
                            &[$($library),*],
                            $expects_error,
                            &[$($source),*],
                            &[$($expected),*],
                            &[$($artifact),*],
                        );
                    }
                }
            };
        }

        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/codegen-direct/generated/composite-grammars-cases.inc.rs"
        ));

        #[derive(Clone, Copy)]
        struct ExpectedDiagnostic {
            java_code: u32,
            severity: crate::grammar::diagnostic::Severity,
            location: Option<(&'static str, usize, usize)>,
        }

        const fn at(
            java_code: u32,
            severity: crate::grammar::diagnostic::Severity,
            source: &'static str,
            line: usize,
            column: usize,
        ) -> ExpectedDiagnostic {
            ExpectedDiagnostic {
                java_code,
                severity,
                location: Some((source, line, column)),
            }
        }

        const fn unlocated(
            java_code: u32,
            severity: crate::grammar::diagnostic::Severity,
        ) -> ExpectedDiagnostic {
            ExpectedDiagnostic {
                java_code,
                severity,
                location: None,
            }
        }

        #[derive(Clone, Copy)]
        enum ArtifactKind {
            Lexer,
            Parser,
        }

        #[derive(Clone, Copy)]
        struct ExpectedArtifact {
            kind: ArtifactKind,
            recognizer: &'static str,
            interp: &'static str,
            tokens: &'static str,
        }

        const fn lexer(
            recognizer: &'static str,
            interp: &'static str,
            tokens: &'static str,
        ) -> ExpectedArtifact {
            ExpectedArtifact {
                kind: ArtifactKind::Lexer,
                recognizer,
                interp,
                tokens,
            }
        }

        const fn parser(
            recognizer: &'static str,
            interp: &'static str,
            tokens: &'static str,
        ) -> ExpectedArtifact {
            ExpectedArtifact {
                kind: ArtifactKind::Parser,
                recognizer,
                interp,
                tokens,
            }
        }

        fn assert_composite_fixture(
            fixture_name: &str,
            roots: &[&str],
            library_directories: &[&str],
            expects_error: bool,
            source_order: &[&str],
            expected: &[ExpectedDiagnostic],
            artifacts: &[ExpectedArtifact],
        ) {
            let directory = fixture(fixture_name);
            let result = compile(LoadOptions {
                roots: roots.iter().map(|root| directory.join(root)).collect(),
                library_directories: library_directories
                    .iter()
                    .map(|library| directory.join(library))
                    .collect(),
            });
            match result {
                Ok(compilation) => {
                    assert!(
                        !expects_error,
                        "{fixture_name}: expected compilation failure"
                    );
                    assert_diagnostics(
                        fixture_name,
                        source_order,
                        Some(&compilation.sources),
                        &compilation.diagnostics,
                        expected,
                    );
                    assert_artifacts(fixture_name, &compilation, artifacts);
                }
                Err(error) => {
                    assert!(expects_error, "{fixture_name}: {error:#?}");
                    assert_diagnostics(
                        fixture_name,
                        source_order,
                        None,
                        error.diagnostics(),
                        expected,
                    );
                }
            }
        }

        fn assert_diagnostics(
            fixture_name: &str,
            source_order: &[&str],
            sources: Option<&crate::grammar::source::SourceSet>,
            actual: &[crate::grammar::diagnostic::Diagnostic],
            expected: &[ExpectedDiagnostic],
        ) {
            assert_eq!(actual.len(), expected.len(), "{fixture_name}: {actual:#?}");
            for (actual, expected) in actual.iter().zip(expected) {
                assert!(
                    rust_code_matches(expected.java_code, actual.code),
                    "{fixture_name}: Java error({}) does not match Rust code {}: {actual:#?}",
                    expected.java_code,
                    actual.code,
                );
                assert_eq!(
                    actual.severity, expected.severity,
                    "{fixture_name}: {actual:#?}",
                );
                if let Some((source, line, column)) = expected.location {
                    let expected_source = source_order
                        .iter()
                        .position(|candidate| *candidate == source)
                        .unwrap_or_else(|| {
                            panic!("{fixture_name}: unknown expected source {source}")
                        });
                    assert_eq!(
                        actual.primary.source.index(),
                        expected_source,
                        "{fixture_name}: expected diagnostic source {source}: {actual:#?}",
                    );
                    if let Some(sources) = sources {
                        assert!(
                            sources
                                .canonical_path(actual.primary.source)
                                .is_some_and(|path| path.ends_with(source)),
                            "{fixture_name}: expected diagnostic source {source}: {actual:#?}",
                        );
                    }
                    let text = std::fs::read_to_string(fixture(fixture_name).join(source))
                        .expect("composite fixture source");
                    assert_eq!(
                        actual.primary.bytes.start,
                        fixture_byte_offset(&text, line, column),
                        "{fixture_name}: expected {source}:{line}:{column} for {actual:#?}",
                    );
                }
            }
        }

        fn rust_code_matches(java_code: u32, rust_code: &str) -> bool {
            match java_code {
                50 => matches!(rust_code, "G4F001" | "G4F002" | "G4F003"),
                56 => rust_code == "G4S007",
                108 => rust_code == "G4S019",
                109 => rust_code == "G4T001",
                110 => rust_code == "G4L005",
                120 => rust_code == "G4S023",
                125 => rust_code == "G4S030",
                _ => false,
            }
        }

        fn assert_artifacts(
            fixture_name: &str,
            compilation: &Compilation,
            expected: &[ExpectedArtifact],
        ) {
            let directory = fixture(fixture_name);
            for artifact in expected {
                match artifact.kind {
                    ArtifactKind::Lexer => {
                        let compiled = lexer_named(compilation, artifact.recognizer);
                        assert_lexer_interp(compiled, &directory.join(artifact.interp));
                        assert_tokens(
                            &compiled.semantic.recognizer,
                            &directory.join(artifact.tokens),
                        );
                    }
                    ArtifactKind::Parser => {
                        let compiled = parser_named(compilation, artifact.recognizer);
                        assert_parser_interp(compiled, &directory.join(artifact.interp));
                        assert_tokens(
                            &compiled.semantic.recognizer,
                            &directory.join(artifact.tokens),
                        );
                    }
                }
            }
        }

        fn assert_tokens(recognizer: &RecognizerModel, expected_path: &Path) {
            let expected = std::fs::read_to_string(expected_path).expect("fixture tokens");
            let mut actual = String::new();
            for name in &recognizer.vocabulary.name_order {
                let number = recognizer.vocabulary.by_name[name];
                writeln!(actual, "{name}={number}").expect("writing to String cannot fail");
            }
            for literal in &recognizer.vocabulary.literal_order {
                let number = recognizer.vocabulary.by_literal[literal];
                writeln!(actual, "{literal}={number}").expect("writing to String cannot fail");
            }
            assert_eq!(actual, expected, "{}", expected_path.display());
        }
    }

    mod upstream_tool_syntax_errors {
        use super::*;
        use crate::grammar::diagnostic::Severity::{Error, Warning};

        #[derive(Clone, Copy)]
        enum FixtureKind {
            Combined,
            Lexer,
            Parser,
        }

        macro_rules! meta_case {
            (
                $name:ident,
                $fixture:literal,
                [$($expected:expr),* $(,)?]
            ) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_distinct_diagnostic_codes($fixture, &[$($expected),*]);
                    }
                }
            };
        }

        macro_rules! case {
            (
                $name:ident,
                $fixture:literal,
                $root:literal,
                $kind:ident,
                $expects_error:literal,
                [$($expected:expr),* $(,)?]
            ) => {
                mod $name {
                    use super::*;

                    #[test]
                    fn matches_java() {
                        assert_tool_syntax_fixture(
                            $fixture,
                            $root,
                            FixtureKind::$kind,
                            $expects_error,
                            &[$($expected),*],
                        );
                    }
                }
            };
        }

        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/codegen-direct/generated/tool-syntax-errors-cases.inc.rs"
        ));

        #[derive(Clone, Copy)]
        struct ExpectedDiagnostic {
            java_code: u32,
            severity: crate::grammar::diagnostic::Severity,
            position: Option<(usize, usize)>,
        }

        const fn at(
            java_code: u32,
            severity: crate::grammar::diagnostic::Severity,
            line: usize,
            column: usize,
        ) -> ExpectedDiagnostic {
            ExpectedDiagnostic {
                java_code,
                severity,
                position: Some((line, column)),
            }
        }

        const fn unlocated(
            java_code: u32,
            severity: crate::grammar::diagnostic::Severity,
        ) -> ExpectedDiagnostic {
            ExpectedDiagnostic {
                java_code,
                severity,
                position: None,
            }
        }

        fn assert_tool_syntax_fixture(
            fixture_name: &str,
            root: &str,
            kind: FixtureKind,
            expects_error: bool,
            expected: &[ExpectedDiagnostic],
        ) {
            match compile_fixture(fixture_name, &[root]) {
                Ok(compilation) => {
                    assert!(
                        !expects_error,
                        "{fixture_name}: expected compilation failure"
                    );
                    assert_diagnostics(fixture_name, root, &compilation.diagnostics, expected);
                    assert_artifacts(fixture_name, root, kind, &compilation);
                }
                Err(error) => {
                    assert!(expects_error, "{fixture_name}: {error:#?}");
                    assert_diagnostics(fixture_name, root, error.diagnostics(), expected);
                }
            }
        }

        fn assert_diagnostics(
            fixture_name: &str,
            root: &str,
            actual: &[crate::grammar::diagnostic::Diagnostic],
            expected: &[ExpectedDiagnostic],
        ) {
            assert_eq!(actual.len(), expected.len(), "{fixture_name}: {actual:#?}");
            let source = std::fs::read_to_string(fixture(fixture_name).join(root))
                .expect("tool syntax fixture source");
            for (actual, expected) in actual.iter().zip(expected) {
                assert!(
                    rust_code_matches(expected.java_code, actual.code),
                    "{fixture_name}: Java error({}) does not match Rust code {}: {actual:#?}",
                    expected.java_code,
                    actual.code,
                );
                assert_eq!(
                    actual.severity, expected.severity,
                    "{fixture_name}: {actual:#?}",
                );
                if let Some((line, column)) = expected.position {
                    assert_eq!(
                        actual.primary.bytes.start,
                        fixture_byte_offset(&source, line, column),
                        "{fixture_name}: expected {line}:{column} for {actual:#?}",
                    );
                }
            }
        }

        fn rust_code_matches(java_code: u32, rust_code: &str) -> bool {
            match java_code {
                31 | 157 => rust_code == "G4S014",
                50 => matches!(rust_code, "G4F001" | "G4F002" | "G4F003"),
                51 => rust_code == "G4S002",
                144 => matches!(rust_code, "G4L002" | "G4S066"),
                149 => rust_code == "G4S049",
                150 => rust_code == "G4S050",
                151 => rust_code == "G4S048",
                153 => rust_code == "G4A001",
                154 => rust_code == "G4A004",
                156 | 182 => rust_code == "G4L002",
                158 => rust_code == "G4S010",
                159 => rust_code == "G4S003",
                163 | 164 => rust_code == "G4S020",
                174 => matches!(rust_code, "G4L001" | "G4L002"),
                177 => rust_code == "G4S053",
                181 => rust_code == "G4S009",
                186 => rust_code == "G4A003",
                _ => false,
            }
        }

        const fn java_error(code: u32, name: &'static str) -> (u32, &'static str) {
            (code, name)
        }

        fn assert_distinct_diagnostic_codes(fixture_name: &str, expected: &[(u32, &'static str)]) {
            let manifest = std::fs::read_to_string(fixture(fixture_name).join("fixture.json"))
                .expect("tool syntax meta fixture");
            assert!(
                manifest.contains("\"AllErrorCodesDistinct\""),
                "{fixture_name}: wrong Java source binding",
            );

            let oracle =
                std::fs::read_to_string(fixture(fixture_name).join("oracle/java-error-types.tsv"))
                    .expect("Java ErrorType oracle");
            let oracle_entries = oracle
                .lines()
                .map(|line| {
                    let (code, name) = line.split_once('\t').expect("Java ErrorType oracle record");
                    (
                        code.parse::<u32>()
                            .expect("Java ErrorType oracle numeric code"),
                        name,
                    )
                })
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(expected.len(), oracle_entries.len(), "{fixture_name}");
            let mut codes = std::collections::HashSet::new();
            let mut names = std::collections::HashSet::new();
            for &(code, name) in expected {
                assert!(
                    oracle_entries.contains(&(code, name)),
                    "{fixture_name}: missing Java error type {name}={code}",
                );
                assert!(
                    codes.insert(code),
                    "{fixture_name}: duplicate Java code {code}"
                );
                assert!(
                    names.insert(name),
                    "{fixture_name}: duplicate Java error type {name}"
                );
            }
        }

        fn assert_artifacts(
            fixture_name: &str,
            root: &str,
            kind: FixtureKind,
            compilation: &Compilation,
        ) {
            let directory = fixture(fixture_name);
            let grammar_name = root
                .strip_suffix(".g4")
                .expect("tool syntax fixture root ends in .g4");
            match kind {
                FixtureKind::Lexer => {
                    let lexer = lexer_named(compilation, grammar_name);
                    assert_lexer_interp(lexer, &directory.join(format!("{grammar_name}.interp")));
                    assert_tokens(
                        &lexer.semantic.recognizer,
                        &directory.join(format!("{grammar_name}.tokens")),
                    );
                }
                FixtureKind::Parser => {
                    let parser = parser_named(compilation, grammar_name);
                    assert_parser_interp(parser, &directory.join(format!("{grammar_name}.interp")));
                    assert_tokens(
                        &parser.semantic.recognizer,
                        &directory.join(format!("{grammar_name}.tokens")),
                    );
                }
                FixtureKind::Combined => {
                    let parser = parser_named(compilation, &format!("{grammar_name}Parser"));
                    assert_parser_interp(parser, &directory.join(format!("{grammar_name}.interp")));
                    assert_tokens(
                        &parser.semantic.recognizer,
                        &directory.join(format!("{grammar_name}.tokens")),
                    );
                    let lexer_interp = directory.join(format!("{grammar_name}Lexer.interp"));
                    if lexer_interp.is_file() {
                        let lexer = lexer_named(compilation, &format!("{grammar_name}Lexer"));
                        assert_lexer_interp(lexer, &lexer_interp);
                        assert_tokens(
                            &lexer.semantic.recognizer,
                            &directory.join(format!("{grammar_name}Lexer.tokens")),
                        );
                    }
                }
            }
        }

        fn assert_tokens(recognizer: &RecognizerModel, expected_path: &Path) {
            let expected = std::fs::read_to_string(expected_path).expect("fixture tokens");
            let mut actual = String::new();
            for name in &recognizer.vocabulary.name_order {
                let number = recognizer.vocabulary.by_name[name];
                writeln!(actual, "{name}={number}").expect("writing to String cannot fail");
            }
            for literal in &recognizer.vocabulary.literal_order {
                let number = recognizer.vocabulary.by_literal[literal];
                writeln!(actual, "{literal}={number}").expect("writing to String cannot fail");
            }
            assert_eq!(actual, expected, "{}", expected_path.display());
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
