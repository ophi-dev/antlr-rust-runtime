#!/usr/bin/env node

import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
    ANTLR_NG_COMMIT,
    ATN_CONSTRUCTION_BASE_COMMIT,
    ATN_CONSTRUCTION_IMPLEMENTATION_COMMIT,
    ATN_CONSTRUCTION_TEST_COMMIT,
    ATN_SERIALIZATION_TEST_COMMIT,
    BASIC_SEMANTIC_BASE_COMMIT,
    BASIC_SEMANTIC_IMPLEMENTATION_COMMIT,
    BASIC_SEMANTIC_TEST_COMMIT,
    CHAR_SUPPORT_BASE_COMMIT,
    CHAR_SUPPORT_IMPLEMENTATION_COMMIT,
    CHAR_SUPPORT_TEST_COMMIT,
    EMPTY_VOCABULARY_BASE_COMMIT,
    EMPTY_VOCABULARY_IMPLEMENTATION_COMMIT,
    EMPTY_VOCABULARY_TEST_COMMIT,
    ERROR_SETS_BASE_COMMIT,
    ERROR_SETS_IMPLEMENTATION_COMMIT,
    ERROR_SETS_TEST_COMMIT,
    ESCAPE_SEQUENCE_IMPLEMENTATION_COMMIT,
    ESCAPE_SEQUENCE_SCAFFOLD_COMMIT,
    ESCAPE_SEQUENCE_TEST_COMMIT,
    FRONTEND_SYNTAX_TEST_COMMIT,
    IMPLEMENTATION_COMMIT,
    JAVA_COMMIT,
    LEFT_RECURSION_BASE_COMMIT,
    LEFT_RECURSION_IMPLEMENTATION_COMMIT,
    LEFT_RECURSION_TEST_COMMIT,
    LOOKAHEAD_TREE_FIXTURE_COMMIT,
    LOOKAHEAD_TREE_IMPLEMENTATION_COMMIT,
    LOOKAHEAD_TREE_TEST_COMMIT,
    NESTED_ACTION_BASE_COMMIT,
    NESTED_ACTION_IMPLEMENTATION_COMMIT,
    NESTED_ACTION_TEST_COMMIT,
    PHASE_B_BASE_COMMIT,
    PHASE_B_IMPLEMENTATION_COMMIT,
    SCAFFOLD_COMMIT,
    SCOPE_PARSING_BASE_COMMIT,
    SCOPE_PARSING_IMPLEMENTATION_COMMIT,
    SCOPE_PARSING_TEST_COMMIT,
    TEST_COMMIT,
    TOKEN_ASSIGNMENT_BASE_COMMIT,
    TOKEN_ASSIGNMENT_IMPLEMENTATION_COMMIT,
    TOKEN_ASSIGNMENT_TEST_COMMIT,
    TOKEN_POSITION_BASE_COMMIT,
    TOKEN_POSITION_IMPLEMENTATION_COMMIT,
    TOKEN_POSITION_TEST_COMMIT,
    TOPOLOGICAL_SORT_BASE_COMMIT,
    TOPOLOGICAL_SORT_TEST_COMMIT,
    UNICODE_DATA_BASE_COMMIT,
    UNICODE_DATA_TEST_COMMIT,
    UNICODE_ESCAPE_IMPLEMENTATION_COMMIT,
    UNICODE_ESCAPE_SCAFFOLD_COMMIT,
    UNICODE_ESCAPE_TEST_COMMIT,
    UNICODE_GRAMMAR_BASE_COMMIT,
    UNICODE_GRAMMAR_IMPLEMENTATION_COMMIT,
    UNICODE_GRAMMAR_TEST_COMMIT,
    VOCABULARY_BASE_COMMIT,
    VOCABULARY_IMPLEMENTATION_COMMIT,
    VOCABULARY_TEST_COMMIT,
    digest,
    parseMode,
    stableStringify,
} from "./evidence-common.mjs";

const APPROVING_REVIEW = "merged implementation plan PR #149, section 11.5";
const FRONTEND_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::frontend::tests::";
const FRONTEND_SYNTAX_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::ported_tests::frontend_tool_syntax_cases_match_upstream_outcomes";
const ATN_SERIALIZATION_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::atn::interp_test::tests::upstream_atn_serialization::";
const ATN_SERIALIZATION_TEST_MODULE =
    "src/bin_support/grammar/atn/interp_test.rs";
const ATN_CONSTRUCTION_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::atn::interp_test::tests::upstream_atn_construction::";
const ATN_CONSTRUCTION_COVERED_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen upstream_atn_construction -- --test-threads=1 --skip parser_rule_ref_in_lexer_rule --skip repeated_transitions_to_stop_state";
const ATN_CONSTRUCTION_RED_CASES = new Map([
    [
        "testatnconstruction-testforrepeatedtransitionstostopstate-a6e224cf58",
        "ATN graph contained RuleStop_e_3->BlockEnd_26 three times instead of once",
    ],
    [
        "testatnconstruction-testparserrulerefinlexerrule-34f2000a35",
        "missing G4S008 diagnostic; Stage 0 reported G4F003 no viable alternative at input 'a'",
    ],
]);
const BASIC_SEMANTIC_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen upstream_basic_semantic_errors -- --test-threads=1";
const BASIC_SEMANTIC_PORTS = new Map([
    [
        "testbasicsemanticerrors-testargumentretvallocalconflicts-fd702fec44",
        {
            rustTest:
                "grammar::atn::interp_test::tests::upstream_basic_semantic_errors::argument_retval_local_conflicts_match_java",
            redFingerprint:
                "expected 10 ordered diagnostics, but the direct compiler emitted 7 generic rule-wide diagnostics",
        },
    ],
    [
        "testbasicsemanticerrors-testillegalnonsetlabel-5c18487902",
        {
            rustTest:
                "grammar::atn::interp_test::tests::upstream_basic_semantic_errors::illegal_non_set_label_matches_java",
            redFingerprint:
                "the invalid label on a non-set block compiled successfully",
        },
    ],
    [
        "testbasicsemanticerrors-testu-c17a76a27e",
        {
            rustTest:
                "grammar::atn::interp_test::tests::upstream_basic_semantic_errors::u_matches_java",
            redFingerprint:
                "expected 11 ordered diagnostics, but the direct compiler emitted 3",
        },
    ],
]);
const ERROR_SETS_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen upstream_error_sets -- --test-threads=1";
const ERROR_SETS_PORTS = new Map([
    [
        "testerrorsets-testnotcharsetwithruleref-9d8ec8db7a",
        {
            rustTest:
                "grammar::atn::interp_test::tests::upstream_error_sets::not_char_set_with_rule_ref_matches_java",
            redFingerprint:
                "expected G4S065 at the lexer-set member, but the compiler emitted G4L003 for the enclosing set",
        },
    ],
    [
        "testerrorsets-testnotcharsetwithstring-04bc32a04f",
        {
            rustTest:
                "grammar::atn::interp_test::tests::upstream_error_sets::not_char_set_with_string_matches_java",
            redFingerprint:
                "expected G4S066 at the lexer-set member, but the compiler emitted G4L002 for the enclosing set",
        },
    ],
]);
const TOKEN_POSITION_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen upstream_token_position_options -- --test-threads=1";
const TOKEN_POSITION_PORTS = new Map([
    [
        "testtokenpositionoptions-testleftrecursionrewrite-0a7598fa91",
        {
            rustTest:
                "grammar::atn::interp_test::tests::upstream_token_position_options::left_recursion_rewrite_matches_java",
            resolution: "verified-covered-existing",
            implementationCommit: TOKEN_POSITION_BASE_COMMIT,
            testCommand:
                "cargo test --locked --features codegen --bin antlr4-rust-gen upstream_token_position_options::left_recursion_rewrite_matches_java -- --test-threads=1",
            greenResult: "1 passed; 0 failed",
        },
    ],
    [
        "testtokenpositionoptions-testleftrecursionwithlabels-6e604809f0",
        {
            rustTest:
                "grammar::atn::interp_test::tests::upstream_token_position_options::left_recursion_with_labels_matches_java",
            resolution: "ported",
            implementationCommit: TOKEN_POSITION_IMPLEMENTATION_COMMIT,
            testCommand: TOKEN_POSITION_TEST_COMMAND,
            greenResult: "3 passed; 0 failed",
            redFingerprint:
                "labeled rule and token references retained the label starts 33 and 59 instead of the Java target starts 35 and 61",
        },
    ],
    [
        "testtokenpositionoptions-testleftrecursionwithset-57f72a753d",
        {
            rustTest:
                "grammar::atn::interp_test::tests::upstream_token_position_options::left_recursion_with_set_matches_java",
            resolution: "verified-covered-existing",
            implementationCommit: TOKEN_POSITION_BASE_COMMIT,
            testCommand:
                "cargo test --locked --features codegen --bin antlr4-rust-gen upstream_token_position_options::left_recursion_with_set_matches_java -- --test-threads=1",
            greenResult: "1 passed; 0 failed",
        },
    ],
]);
const TOPOLOGICAL_SORT_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen upstream_topological_sort -- --test-threads=1";
const TOPOLOGICAL_SORT_PORTS = new Map([
    [
        "testtopologicalsort-testcyclicgraph-94f1aecafb",
        "cyclic_graph_matches_java",
    ],
    [
        "testtopologicalsort-testfairlylargegraph-a5f1fbf809",
        "fairly_large_graph_matches_java",
    ],
    [
        "testtopologicalsort-testparserlexercombo-8897396a63",
        "parser_lexer_combo_matches_java",
    ],
    [
        "testtopologicalsort-testrepeatededges-e97d12fac9",
        "repeated_edges_match_java",
    ],
    [
        "testtopologicalsort-testsimpletokendependence-02d55e6f25",
        "simple_token_dependence_matches_java",
    ],
]);
const VOCABULARY_TEST_COMMAND =
    "cargo test --locked --lib upstream_vocabulary -- --test-threads=1";
const EMPTY_VOCABULARY_TEST_COMMAND =
    "cargo test --locked --lib empty_vocabulary_matches_java -- --test-threads=1";
const VOCABULARY_PORTS = new Map([
    [
        "testvocabulary-testemptyvocabulary-66d31ad014",
        {
            rustTest:
                "vocabulary::tests::upstream_vocabulary::empty_vocabulary_matches_java",
            scaffoldCommit: EMPTY_VOCABULARY_BASE_COMMIT,
            testCommit: EMPTY_VOCABULARY_TEST_COMMIT,
            implementationCommit: EMPTY_VOCABULARY_IMPLEMENTATION_COMMIT,
            testCommand: EMPTY_VOCABULARY_TEST_COMMAND,
            greenResult: "1 passed; 0 failed",
            redFingerprint:
                "E0599: no associated function or constant named empty found for Vocabulary",
        },
    ],
    [
        "testvocabulary-testvocabularyfromtokennames-d047506a84",
        {
            rustTest:
                "vocabulary::tests::upstream_vocabulary::vocabulary_from_token_names_matches_java",
            scaffoldCommit: VOCABULARY_BASE_COMMIT,
            testCommit: VOCABULARY_TEST_COMMIT,
            implementationCommit: VOCABULARY_IMPLEMENTATION_COMMIT,
            testCommand: VOCABULARY_TEST_COMMAND,
            greenResult: "2 passed; 0 failed",
            redFingerprint:
                "E0599: no associated function or constant named from_token_names found for Vocabulary",
        },
    ],
]);
const SCOPE_PARSING_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen embedded::tests::upstream_scope_parsing::argument_declarations_match_java";
const CHAR_SUPPORT_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::char_support::tests::";
const NESTED_ACTION_LOGICAL_ID =
    "testlexeractions-nested-actions-3d175db5e5";
const NESTED_ACTION_TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::syntax::tests::nested_actions_match_upstream -- --exact";
const ESCAPE_SEQUENCE_TEST_PREFIX =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::escape_sequence::tests::";
const ESCAPE_SEQUENCE_RED_CASES = new Map([
    [
        "testParseNewline",
        "left Invalid, right CodePoint { value: 10, start: 0, stop: 2 }",
    ],
    [
        "testParseTab",
        "left Invalid, right CodePoint { value: 9, start: 0, stop: 2 }",
    ],
    [
        "testParseUnicodeBMP",
        "left Invalid, right CodePoint { value: 43981, start: 0, stop: 6 }",
    ],
    [
        "testParseUnicodeSMP",
        "left Invalid, right CodePoint { value: 1092557, start: 0, stop: 10 }",
    ],
    [
        "testParseUnicodeProperty",
        "left Invalid, right Property with ranges [(66560, 66639)]",
    ],
    [
        "testParseUnicodePropertyInverted",
        "left Invalid, right Property with ranges [(0, 66559), (66640, 1114111)]",
    ],
]);
const UNICODE_ESCAPE_EXPECTED = new Map([
    ["latinJavaEscape", "\\u0061"],
    ["latinPythonEscape", "\\u0061"],
    ["latinSwiftEscape", "\\u{0061}"],
    ["bmpJavaEscape", "\\uABCD"],
    ["bmpPythonEscape", "\\uABCD"],
    ["bmpSwiftEscape", "\\u{ABCD}"],
    ["smpJavaEscape", "\\uD83D\\uDCA9"],
    ["smpPythonEscape", "\\U0001F4A9"],
    ["smpSwiftEscape", "\\u{1F4A9}"],
]);
const UNICODE_ESCAPE_TEST_PREFIX =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::unicode_escape::tests::";
const UNICODE_DATA_TEST_PREFIX =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::unicode::tests::";
const UNICODE_DATA_TEST_NAMES = new Map([
    [
        "testUnicodeGeneralCategoriesLatin",
        "unicode_general_categories_latin_matches_java",
    ],
    [
        "testUnicodeGeneralCategoriesBMP",
        "unicode_general_categories_bmp_matches_java",
    ],
    [
        "testUnicodeGeneralCategoriesSMP",
        "unicode_general_categories_smp_matches_java",
    ],
    ["testUnicodeCategoryAliases", "unicode_category_aliases_match_java"],
    ["testUnicodeBinaryProperties", "unicode_binary_properties_match_java"],
    [
        "testUnicodeBinaryPropertyAliases",
        "unicode_binary_property_aliases_match_java",
    ],
    ["testUnicodeScripts", "unicode_scripts_match_java"],
    ["testUnicodeScriptEquals", "unicode_script_equals_matches_java"],
    ["testUnicodeScriptAliases", "unicode_script_aliases_match_java"],
    ["testUnicodeBlocks", "unicode_blocks_match_java"],
    ["testUnicodeBlockEquals", "unicode_block_equals_matches_java"],
    ["testUnicodeBlockAliases", "unicode_block_aliases_match_java"],
    ["testEnumeratedPropertyEquals", "enumerated_property_equals_matches_java"],
    ["extendedPictographic", "extended_pictographic_matches_java"],
    ["emojiPresentation", "emoji_presentation_matches_java"],
    [
        "testPropertyCaseInsensitivity",
        "property_case_insensitivity_matches_java",
    ],
    [
        "testPropertyDashSameAsUnderscore",
        "property_dash_same_as_underscore_matches_java",
    ],
    [
        "modifyingUnicodeDataShouldThrow",
        "modifying_unicode_data_should_throw_matches_java",
    ],
]);
const UNICODE_GRAMMAR_TEST_PREFIX =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::atn::interp_test::tests::upstream_unicode_grammar::";
const UNICODE_GRAMMAR_PORTS = new Map([
    [
        "testunicodegrammar-binarygrammar-611ebe1d6f",
        {
            testName: "binary",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testunicodegrammar-matchingdanglingsurrogateininput-8b7976ab4f",
        {
            testName: "dangling_surrogate",
            resolution: "ported",
            redFingerprint:
                "G4L002: Unicode escape is not a scalar value: 0xd83c",
        },
    ],
    [
        "testunicodegrammar-unicodebmpliteralingrammar-4e3b8e43e6",
        {
            testName: "bmp_literal",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testunicodegrammar-unicodesmpliteralingrammar-b41d70815f",
        {
            testName: "smp_literal",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testunicodegrammar-unicodesmprangeingrammar-69d43e47cb",
        {
            testName: "smp_range",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testunicodegrammar-unicodesurrogatepairliteralingrammar-d1ada97cc5",
        {
            testName: "disabled_surrogate_pair_literal",
            resolution: "ported",
            redFingerprint:
                "G4L002: Unicode escape is not a scalar value: 0xd83c",
        },
    ],
]);
const TOKEN_ASSIGNMENT_TEST_PREFIX =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::atn::interp_test::tests::upstream_token_type_assignment::";
const TOKEN_ASSIGNMENT_PORTS = new Map([
    [
        "testtokentypeassignment-testcombinedgrammarliterals-74842182c1",
        {
            testName: "combined_grammar_literals",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testtokentypeassignment-testcombinedgrammarwithreftoliteralbutnotokenidref-fd2391c14b",
        {
            testName:
                "combined_grammar_with_ref_to_literal_but_no_token_id_ref",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testtokentypeassignment-testlexertokenssection-67f7fb02d9",
        {
            testName: "lexer_tokens_section",
            resolution: "ported",
            redFingerprint:
                "left: \"C=1\\nD=2\\nA=3\\n'c'=1\\n'a'=3\\n\"; right: \"C=1\\nD=2\\nA=3\\n'a'=3\\n'c'=1\\n\"",
        },
    ],
    [
        "testtokentypeassignment-testliteralinparserandlexer-177a82c119",
        {
            testName: "literal_in_parser_and_lexer",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testtokentypeassignment-testparsercharliteralwithbasicunicodeescape-8afd5248f1",
        {
            testName:
                "parser_char_literal_with_basic_unicode_escape",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testtokentypeassignment-testparsercharliteralwithescape-15c4d62b48",
        {
            testName: "parser_char_literal_with_escape",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testtokentypeassignment-testparsercharliteralwithextendedunicodeescape-e6f767b0b7",
        {
            testName:
                "parser_char_literal_with_extended_unicode_escape",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testtokentypeassignment-testparsersimpletokens-809afdc7eb",
        {
            testName: "parser_simple_tokens",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testtokentypeassignment-testparsertokenssection-f0930e6dae",
        {
            testName: "parser_tokens_section",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testtokentypeassignment-testpreddoesnothidenametoliteralmapinlexer-a1fc06a563",
        {
            testName:
                "pred_does_not_hide_name_to_literal_map_in_lexer",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testtokentypeassignment-testsetdoesnotmisstokenaliases-92cf195953",
        {
            testName: "set_does_not_miss_token_aliases",
            resolution: "verified-covered-existing",
        },
    ],
]);
const LEFT_RECURSION_TEST_PREFIX =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::atn::interp_test::tests::upstream_left_recursion_tool_issues::";
const LEFT_RECURSION_PORTS = new Map([
    [
        "testleftrecursiontoolissues-testargonprimaryruleinleftrecursiverule-e2b3d25b22",
        {
            testName: "argument_on_primary_rule",
            testFunction: "matches_java_interps",
            resolution: "verified-covered-existing",
        },
    ],
    [
        "testleftrecursiontoolissues-testcheckforleftrecursiveemptyfollow-558283d55a",
        {
            testName: "empty_left_recursive_follow",
            testFunction: "matches_java_diagnostic",
            resolution: "ported",
            redFingerprint:
                "G4A002 used closure terminology instead of Java's left-recursive alternative diagnostic",
        },
    ],
    [
        "testleftrecursiontoolissues-testcheckfornonleftrecursiverule-477f42142e",
        {
            testName: "no_non_left_recursive_alternative",
            testFunction: "matches_java_diagnostic",
            resolution: "ported",
            redFingerprint:
                "G4R002 used non-left-recursive terminology instead of Java's exact diagnostic",
        },
    ],
    [
        "testleftrecursiontoolissues-testisolatedleftrecursiveruleref-43f8252e7d",
        {
            testName: "isolated_left_recursive_rule_reference",
            testFunction: "matches_java_diagnostic",
            resolution: "ported",
            redFingerprint:
                "G4R001 used the Rust implementation-pattern message instead of Java's exact diagnostic",
        },
    ],
    [
        "testleftrecursiontoolissues-testleftrecursiverulerefwitharg-40cd52608d",
        {
            testName: "recursive_rule_reference_with_argument",
            testFunction: "matches_java_diagnostic",
            resolution: "ported",
            redFingerprint:
                "the direct compiler accepted a left-recursive rule reference carrying arguments",
        },
    ],
    [
        "testleftrecursiontoolissues-testleftrecursiverulerefwitharg2-7332bdbd4f",
        {
            testName:
                "recursive_rule_reference_with_argument_and_parameter",
            testFunction: "matches_java_diagnostic",
            resolution: "ported",
            redFingerprint:
                "the direct compiler accepted a parameterized left-recursive rule reference carrying arguments",
        },
    ],
    [
        "testleftrecursiontoolissues-testleftrecursiverulerefwitharg3-719e121a92",
        {
            testName:
                "recursive_rule_reference_with_argument_without_parameter",
            testFunction: "matches_java_diagnostic",
            resolution: "ported",
            redFingerprint:
                "the direct compiler accepted an argument on a left-recursive rule without parameters",
        },
    ],
]);
const LOOKAHEAD_TREE_TEST_PREFIX =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::atn::interp_test::tests::upstream_lookahead_trees::";
const LOOKAHEAD_TREE_PORTS = new Map([
    [
        "testlookaheadtrees-testalts-ea8f84416c",
        {
            testName: "alternatives_match_java",
            redFingerprint:
                'decision 0, alternative 2 produced "(e:1 a . b)" instead of "(e:2 a <error .>)"',
        },
    ],
    [
        "testlookaheadtrees-testalts2-4e81c43326",
        {
            testName: "left_recursive_loop_match_java",
            redFingerprint:
                'decision 1, alternative 1 produced "(e:1 a)" instead of "(e:2 (e:1 a) <error ;>)"',
        },
    ],
    [
        "testlookaheadtrees-testcallleftrecursiverule-410ec32fb8",
        {
            testName: "calls_left_recursive_rule_match_java",
            redFingerprint:
                'decision 0, alternative 2 produced "(a:1 (e:4 x) ;)" instead of "(a:2 x ;)"',
        },
    ],
    [
        "testlookaheadtrees-testincludeeof-41ef07554a",
        {
            testName: "include_eof_matches_java",
            redFingerprint:
                'decision 0, alternative 2 produced "(e:1 a . b <EOF>)" instead of "(e:2 a . b <EOF>)"',
        },
    ],
]);
const CHAR_SUPPORT_PORTS = new Map([
    [
        "testcharsupport-testcapitalize-25cbf55e21",
        {
            testName: "capitalize_matches_java",
            missingFunction: "capitalize",
        },
    ],
    [
        "testcharsupport-testgetantlrcharliteralforchar-5f81e9b4e6",
        {
            testName: "antlr_char_literal_for_char_matches_java",
            missingFunction: "get_antlr_char_literal_for_char",
        },
    ],
    [
        "testcharsupport-testgetcharvaluefromcharingrammarliteral-94ddda545b",
        {
            testName: "char_value_from_char_in_grammar_literal_matches_java",
            missingFunction: "get_char_value_from_char_in_grammar_literal",
        },
    ],
    [
        "testcharsupport-testgetcharvaluefromgrammarcharliteral-7e17776ef5",
        {
            testName: "char_value_from_grammar_char_literal_matches_java",
            missingFunction: "get_char_value_from_grammar_char_literal",
        },
    ],
    [
        "testcharsupport-testgetintervalsetescapedstring-6bc4eb94c4",
        {
            testName: "interval_set_escaped_string_matches_java",
            missingFunction: "get_interval_set_escaped_string",
        },
    ],
    [
        "testcharsupport-testgetrangeescapedstring-ecd6bf8c9f",
        {
            testName: "range_escaped_string_matches_java",
            missingFunction: "get_range_escaped_string",
        },
    ],
    [
        "testcharsupport-testgetstringfromgrammarstringliteral-601aa92456",
        {
            testName: "string_from_grammar_string_literal_matches_java",
            missingFunction: "get_string_from_grammar_string_literal",
        },
    ],
    [
        "testcharsupport-testparsehexvalue-de6de267d9",
        {
            testName: "parse_hex_value_matches_java",
            missingFunction: "parse_hex_value",
        },
    ],
]);

const PHASE_B_SUITES = new Set([
    "TestATNConstruction",
    "TestATNSerialization",
    "TestAttributeChecks",
    "TestBasicSemanticErrors",
    "TestCharSupport",
    "TestCompositeGrammars",
    "TestErrorSets",
    "TestEscapeSequenceParsing",
    "TestGraphNodes",
    "TestLeftRecursionToolIssues",
    "TestLookaheadTrees",
    "TestScopeParsing",
    "TestSymbolIssues",
    "TestTokenPositionOptions",
    "TestTokenTypeAssignment",
    "TestTopologicalSort",
    "TestUnicodeData",
    "TestUnicodeEscapes",
    "TestUnicodeGrammar",
    "TestVocabulary",
]);
const PHASE_C_SUITES = new Set([
    "TestAmbigParseTrees",
    "TestATNInterpreter",
    "TestATNLexerInterpreter",
    "TestATNParserPrediction",
    "TestCodeGeneration",
    "TestGrammarParserInterpreter",
    "TestParserExec",
    "TestParserInterpreter",
]);
const COVERED_EXISTING = new Map([
    [
        "TestActionSplitter",
        "existing embedded-action splitter and body parsing tests are authoritative for Rust",
    ],
    [
        "TestActionTranslation",
        "existing embedded action/template lowering tests are authoritative for Rust target syntax",
    ],
    [
        "TestATNDeserialization",
        "existing runtime ATN deserializer tests cover the retained runtime boundary",
    ],
    [
        "TestDollarParser",
        "existing embedded Rust attribute translation tests cover dollar references",
    ],
]);
const OUT_OF_SCOPE = new Map([
    ["TestBufferedTokenStream", "runtime token-stream container behavior"],
    ["TestCommonTokenStream", "runtime token-stream container behavior"],
    ["TestFastQueue", "Java-only utility container"],
    ["TestIntervalSet", "runtime interval-set utility behavior"],
    ["TestParseTreeMatcher", "runtime parse-tree matching utility"],
    ["TestParserProfiler", "runtime parser profiling"],
    ["TestPerformance", "performance is governed by section 13 benchmarks"],
    ["TestUnbufferedCharStream", "Java-only unbuffered stream utility"],
    ["TestUnbufferedTokenStream", "Java-only unbuffered stream utility"],
    ["TestUtils", "upstream implementation utility behavior"],
    ["TestXPath", "runtime XPath utility behavior"],
]);

const FRONTEND_SYNTAX_CASES = new Set(
    [
        "testA",
        "testExtraColon",
        "testMissingRuleSemi",
        "testMissingRuleSemi2",
        "testMissingRuleSemi3",
        "testMissingRuleSemi4",
        "testMissingRuleSemi5",
        "testBadRulePrequelStart",
        "testBadRulePrequelStart2",
        "testUnterminatedStringLiteral",
        "testParserRuleNameStartingWithUnderscore",
        "testEmptyGrammarOptions",
        "testEmptyRuleOptions",
        "testEmptyBlockOptions",
        "testEmptyTokensBlock",
    ].map(canonicalName),
);
const GENERAL_FRONTEND_CASES = new Set(
    [
        "Grammar with element options",
        "Non-greedy optionals",
        "Bug #62 Triple quoted strings in actions",
    ].map(canonicalName),
);

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const inventoryPath = resolve(
    repoRoot,
    "tests/codegen-direct/upstream-case-inventory.json",
);
const externalMapPath = resolve(
    repoRoot,
    "tests/codegen-direct/external-fixture-map.json",
);
const externalInventoryPath = resolve(
    repoRoot,
    "tests/codegen-direct/external-source-inventory.json",
);
const outputPath = resolve(
    repoRoot,
    "tests/codegen-direct/upstream-test-map.json",
);
const update = parseMode(
    process.argv.slice(2),
    "generate-upstream-test-map.mjs",
);
const inventory = JSON.parse(await readFile(inventoryPath, "utf8"));
const externalMap = JSON.parse(await readFile(externalMapPath, "utf8"));
const externalInventory = JSON.parse(
    await readFile(externalInventoryPath, "utf8"),
);
const externalSources = new Map(
    externalInventory.artifacts.map((artifact) => [artifact.source_id, artifact]),
);
const externalAssertions = new Map(
    externalMap.fixtures.flatMap((fixture) =>
        fixture.assertions.map((assertion) => [
            assertion.id,
            { fixture, assertion },
        ]),
    ),
);
const completedPhaseBPorts = await loadCompletedPhaseBPorts();

const unassigned = new Map(inventory.cases.map((testCase) => [testCase.id, testCase]));
const rows = [];
rows.push(
    phaseARow({
        logicalId: "frontend-token-cst-parity",
        cases: takeCases((testCase) => testCase.suite === "TestASTStructure"),
        externalAssertionIds: externalMap.fixtures
            .flatMap((fixture) => fixture.assertions)
            .filter(
                (assertion) =>
                    assertion.tdd_owner === "upstream:frontend-token-cst-parity",
            )
            .map((assertion) => assertion.id)
            .sort(),
        rustTest:
            "grammar::frontend::tests::pinned_frontend_corpus_matches_token_and_tree_oracles",
        unitUnderTest: "Stage 0 tokenization and lossless CST construction",
        observable:
            "complete token streams and canonical grammar parse trees from the pinned frontend",
    }),
);
rows.push(
    phaseARow({
        logicalId: "frontend-fail-closed-syntax",
        cases: takeCases(
            (testCase) =>
                testCase.suite === "TestToolSyntaxErrors" &&
                FRONTEND_SYNTAX_CASES.has(canonicalName(testCase.name)),
        ),
        externalAssertionIds: [],
        rustTest:
            "grammar::ported_tests::frontend_tool_syntax_cases_match_upstream_outcomes",
        unitUnderTest: "Stage 0 syntax acceptance and fail-closed boundary",
        observable:
            "ported grammar syntax cases return a CST or diagnostics according to the pinned upstream outcomes",
        revision: 2,
        resolution: "verified-covered-existing",
        testCommit: FRONTEND_SYNTAX_TEST_COMMIT,
        testCommand: FRONTEND_SYNTAX_TEST_COMMAND,
        greenResult: "1 passed; 0 failed",
    }),
);
rows.push(
    phaseARow({
        logicalId: "frontend-source-regressions",
        cases: takeCases(
            (testCase) =>
                testCase.suite === "General" &&
                GENERAL_FRONTEND_CASES.has(canonicalName(testCase.name)),
        ),
        externalAssertionIds: [],
        rustTest:
            "grammar::frontend::tests::pinned_frontend_corpus_matches_token_and_tree_oracles",
        unitUnderTest: "Stage 0 grammar-source lexer adaptor",
        observable:
            "element options, nongreedy EBNF, and nested action strings remain lossless",
    }),
);
rows.push(
    phaseARow({
        logicalId: "frontend-bootstrap-corpus",
        cases: [],
        externalAssertionIds: [],
        rustTest:
            "grammar::frontend::tests::pinned_frontend_corpus_matches_token_and_tree_oracles",
        unitUnderTest: "Stage 0 grammar frontend bootstrap corpus",
        observable:
            "all nine pinned antlr-ng bootstrap grammars match token and CST snapshots",
        fixturePaths: [
            "tests/codegen-direct/frontend-corpus.json",
            "tests/codegen-direct/frontend-snapshots.tsv",
        ],
    }),
);

const groups = new Map();
for (const testCase of unassigned.values()) {
    const key = `${testCase.suite}\0${canonicalName(testCase.name)}\0${parameterKey(testCase)}`;
    const group = groups.get(key) ?? [];
    group.push(testCase);
    groups.set(key, group);
}
for (const [key, cases] of [...groups.entries()].sort(([left], [right]) =>
    left.localeCompare(right),
)) {
    const suite = cases[0].suite;
    const name = cases[0].name;
    const logicalId = logicalCaseId(suite, name, key);
    const policy = policyFor(suite, name);
    rows.push(mappedRow(logicalId, cases, policy));
}

rows.sort((left, right) => left.logical_id.localeCompare(right.logical_id));
const map = {
    schema_version: 1,
    generated_by: "tools/grammar-frontend/generate-upstream-test-map.mjs",
    pins: {
        java_antlr: JAVA_COMMIT,
        antlr_ng: ANTLR_NG_COMMIT,
    },
    source_inventory_case_count: inventory.case_count,
    active_row_count: rows.length,
    rows,
};
const serialized = `${JSON.stringify(map, null, 2)}\n`;
if (update) {
    await writeFile(outputPath, serialized, "utf8");
    console.log(`updated upstream test map with ${rows.length} active rows`);
} else {
    if ((await readFile(outputPath, "utf8")) !== serialized) {
        throw new Error("upstream-test-map.json is not reproducible from its inventory");
    }
    console.log(`verified upstream test map with ${rows.length} active rows`);
}

function takeCases(predicate) {
    const selected = [];
    for (const [id, testCase] of unassigned) {
        if (predicate(testCase)) {
            selected.push(testCase);
            unassigned.delete(id);
        }
    }
    selected.sort(compareSourceCases);
    return selected;
}

function phaseARow({
    logicalId,
    cases,
    externalAssertionIds,
    rustTest,
    unitUnderTest,
    observable,
    fixturePaths = [],
    revision = 1,
    resolution = "ported",
    testCommit = TEST_COMMIT,
    testCommand = FRONTEND_TEST_COMMAND,
    greenResult = "5 passed; 0 failed",
}) {
    if (cases.length === 0 && fixturePaths.length === 0) {
        throw new Error(`Phase A row ${logicalId} has no source cases or fixtures`);
    }
    const sourceCaseIds = cases.map((testCase) => testCase.id);
    const closure = {
        logical_id: logicalId,
        source_case_ids: sourceCaseIds,
        external_assertion_ids: externalAssertionIds,
        external_assertion_inputs: externalAssertionIds.map(
            externalAssertionInput,
        ),
        fixture_paths: fixturePaths,
        owner_phase: "A",
        disposition: "port",
        rust_test: rustTest,
        unit_under_test: unitUnderTest,
        observable,
        scaffold_commit: SCAFFOLD_COMMIT,
        primary_test_commit: testCommit,
        ...(resolution === "ported" ? {} : { resolution }),
    };
    const closureHash = digest(stableStringify(closure));
    const javaSource = sourceIdentity(cases, "java-antlr");
    const antlrNgSource = sourceIdentity(cases, "antlr-ng");
    const hasJavaSource = javaSource.source_case_ids.length > 0;
    const hasAntlrNgSource = antlrNgSource.source_case_ids.length > 0;
    const primaryTestSource = hasJavaSource
        ? javaSource
        : hasAntlrNgSource
          ? antlrNgSource
          : {
                implementation: "antlr-ng",
                commit: ANTLR_NG_COMMIT,
                source_case_ids: [],
                fixture_paths: fixturePaths,
                reason: "pinned antlr-ng bootstrap corpus",
            };
    const alternateTestSource = hasJavaSource
        ? antlrNgSource
        : {
              implementation: "independent-generated-oracle",
              commit: JAVA_COMMIT,
              source_case_ids: [],
              reason: "Java fixture generated from the same canonical grammar input",
          };
    return {
        logical_id: logicalId,
        source_case_ids: sourceCaseIds,
        external_assertion_ids: externalAssertionIds,
        owner_phase: "A",
        disposition: "port",
        active_revision_id: `${logicalId}-r${revision}`,
        tdd_state: "done",
        ...(resolution === "ported" ? {} : { resolution }),
        rust_test: rustTest,
        primary_test_source: primaryTestSource,
        alternate_test_source: alternateTestSource,
        primary_implementation_source: `antlr-ng@${ANTLR_NG_COMMIT}`,
        alternate_implementation_source: `java-antlr@${JAVA_COMMIT}`,
        prerequisites: ["behavior-free grammar frontend scaffold"],
        unit_under_test: unitUnderTest,
        expected_red_failure_fingerprint:
            resolution === "ported"
                ? "red fingerprint: Stage 0 frontend is not installed"
                : "not applicable: the case-specific port passed against the existing Phase A frontend",
        observable_equivalence: observable,
        scaffold_commit: SCAFFOLD_COMMIT,
        primary_test_commit: testCommit,
        ...(resolution === "ported"
            ? {
                  demonstrated_red: {
                      command: FRONTEND_TEST_COMMAND,
                      exit_code: 101,
                      fingerprint: "G4F000 Stage 0 frontend is not installed",
                  },
              }
            : {
                  verified_covered_existing: {
                      command: testCommand,
                      commit: testCommit,
                      exit_code: 0,
                      result: greenResult,
                  },
              }),
        primary_implementation_commit: IMPLEMENTATION_COMMIT,
        green_result: {
            command: testCommand,
            result: greenResult,
        },
        closure,
        closure_sha256: closureHash,
        evidence_path: `tests/codegen-direct/port-evidence/${logicalId}`,
    };
}

function externalAssertionInput(assertionId) {
    const linked = externalAssertions.get(assertionId);
    if (!linked) {
        throw new Error(`unknown linked external assertion: ${assertionId}`);
    }
    const source = externalSources.get(linked.fixture.source_id);
    if (!source) {
        throw new Error(
            `external assertion ${assertionId} has unknown source ${linked.fixture.source_id}`,
        );
    }
    return {
        assertion_id: assertionId,
        source_id: source.source_id,
        source_sha256: source.sha256,
        observable: linked.assertion.observable,
        rust_test: linked.assertion.rust_test,
    };
}

function mappedRow(logicalId, cases, policy) {
    const sourceCaseIds = cases.map((testCase) => testCase.id).sort();
    const externalAssertionIds = [...externalAssertions]
        .filter(
            ([, { assertion }]) =>
                assertion.tdd_owner === `upstream:${logicalId}`,
        )
        .map(([assertionId]) => assertionId)
        .sort();
    if (policy.disposition !== "port") {
        return {
            logical_id: logicalId,
            source_case_ids: sourceCaseIds,
            external_assertion_ids: externalAssertionIds,
            owner_phase: policy.owner,
            disposition: policy.disposition,
            active_revision_id: null,
            rationale: `${policy.rationale}; case ${cases[0].suite}.${cases[0].name}`,
            covering_evidence: policy.evidence,
            approving_reviewer: APPROVING_REVIEW,
        };
    }
    const completed = completedPhaseBPorts.get(logicalId);
    if (completed) {
        return completedPhaseBRow(
            logicalId,
            cases,
            policy,
            externalAssertionIds,
            completed,
        );
    }

    const closure = {
        logical_id: logicalId,
        source_case_ids: sourceCaseIds,
        external_assertion_ids: externalAssertionIds,
        ...(externalAssertionIds.length === 0
            ? {}
            : {
                  external_assertion_inputs: externalAssertionIds.map(
                      externalAssertionInput,
                  ),
              }),
        owner_phase: policy.owner,
        disposition: "port",
        rust_test: `planned:tests/codegen-direct/fixtures/${logicalId}`,
        unit_under_test: policy.unit,
        observable: `pinned ${cases[0].suite}.${cases[0].name} behavior`,
    };
    return {
        logical_id: logicalId,
        source_case_ids: sourceCaseIds,
        external_assertion_ids: externalAssertionIds,
        owner_phase: policy.owner,
        disposition: "port",
        active_revision_id: `${logicalId}-r1`,
        tdd_state: "mapped",
        rust_test: closure.rust_test,
        primary_test_source: sourceIdentity(cases, "java-antlr"),
        alternate_test_source: sourceIdentity(cases, "antlr-ng"),
        primary_implementation_source: `antlr-ng@${ANTLR_NG_COMMIT}`,
        alternate_implementation_source: `java-antlr@${JAVA_COMMIT}`,
        prerequisites: [`Phase ${policy.owner} compiler boundary`],
        unit_under_test: policy.unit,
        expected_red_failure_fingerprint: "not demonstrated while state is mapped",
        observable_equivalence: closure.observable,
        closure,
        closure_sha256: digest(stableStringify(closure)),
        evidence_path: null,
    };
}

function completedPhaseBRow(
    logicalId,
    cases,
    policy,
    externalAssertionIds,
    completed,
) {
    if (policy.owner !== "B") {
        throw new Error(`${logicalId} completed Phase B port has owner ${policy.owner}`);
    }
    const sourceCaseIds = cases.map((testCase) => testCase.id).sort();
    const observable =
        completed.kind === "atn-serialization"
            ? `direct Rust serialization matches the complete Java 4.13.2 .interp ` +
              `for ${cases[0].suite}.${cases[0].name}`
            : completed.kind === "atn-construction"
              ? `direct Rust ATN construction matches the Java 4.13.2 graph, ` +
                `.interp, or diagnostic for ${cases[0].suite}.${cases[0].name}`
              : completed.kind === "error-sets"
                ? `direct Rust lexer-set diagnostics match Java 4.13.2 exactly ` +
                  `for ${cases[0].suite}.${cases[0].name}`
                : completed.kind === "token-position-options"
                  ? `direct Rust left-recursion source bindings and .interp match ` +
                    `Java 4.13.2 for ${cases[0].suite}.${cases[0].name}`
                  : completed.kind === "topological-sort"
                    ? `direct Rust grammar dependencies preserve Java's dependency-first ` +
                      `order for ${cases[0].suite}.${cases[0].name}`
                    : completed.kind === "vocabulary"
                      ? `the Rust vocabulary API matches Java 4.13.2 name classification ` +
                        `for ${cases[0].suite}.${cases[0].name}`
                      : completed.kind === "scope-parsing"
                        ? `direct Rust declaration parsing matches Java 4.13.2 names, ` +
                          `types, and initializers for ${cases[0].suite}.${cases[0].name}`
                        : completed.kind === "char-support"
                          ? `direct Rust character literal support matches Java 4.13.2 ` +
                            `for ${cases[0].suite}.${cases[0].name}`
                          : completed.kind === "nested-action"
                            ? `direct Rust grammar modeling preserves nested member actions and ` +
                              `normalizes predicate fail messages for ${cases[0].suite}.${cases[0].name}`
                            : completed.kind === "escape-sequence"
                              ? `direct Rust escape parsing matches Java 4.13.2 result kind, ` +
                                `value or property set, and consumed span for ${cases[0].suite}.${cases[0].name}`
                              : completed.kind === "unicode-escape"
                                ? `direct Rust Unicode escape rendering matches Java 4.13.2 ` +
                                  `for ${cases[0].suite}.${cases[0].name}`
                                : completed.kind === "unicode-data"
                                  ? `direct Rust Unicode property data matches Java 4.13.2 ` +
                                    `for ${cases[0].suite}.${cases[0].name}`
                                  : completed.kind === "unicode-grammar"
                                    ? `direct Rust lexer and parser serialization matches both complete ` +
                                      `Java 4.13.2 .interp files for ${cases[0].suite}.${cases[0].name}`
                                    : completed.kind === "token-type-assignment"
                                      ? `direct Rust recognizer metadata and token vocabulary text match ` +
                                        `Java 4.13.2 exactly for ${cases[0].suite}.${cases[0].name}`
                                      : completed.kind ===
                                          "left-recursion-tool-issues"
                                        ? `direct Rust left-recursion diagnostics and accepted serialization match ` +
                                          `Java 4.13.2 for ${cases[0].suite}.${cases[0].name}`
                                        : completed.kind === "lookahead-trees"
                                          ? `direct Rust forced-alternative parse trees and complete serialization match ` +
                                            `Java 4.13.2 for ${cases[0].suite}.${cases[0].name}`
                        : `direct Rust semantic diagnostics match Java 4.13.2 exactly ` +
                          `for ${cases[0].suite}.${cases[0].name}`;
    const coveredExisting =
        completed.resolution === "verified-covered-existing";
    const closure = {
        logical_id: logicalId,
        source_case_ids: sourceCaseIds,
        external_assertion_ids: externalAssertionIds,
        ...(externalAssertionIds.length === 0
            ? {}
            : {
                  external_assertion_inputs: externalAssertionIds.map(
                      externalAssertionInput,
                  ),
              }),
        fixture_paths: completed.fixturePaths,
        owner_phase: "B",
        disposition: "port",
        rust_test: completed.rustTest,
        unit_under_test: policy.unit,
        observable,
        scaffold_commit: completed.scaffoldCommit,
        primary_test_commit: completed.testCommit,
        ...(coveredExisting
            ? { resolution: "verified-covered-existing" }
            : {}),
    };
    return {
        logical_id: logicalId,
        source_case_ids: sourceCaseIds,
        external_assertion_ids: externalAssertionIds,
        owner_phase: "B",
        disposition: "port",
        active_revision_id: `${logicalId}-r1`,
        tdd_state: "done",
        ...(coveredExisting
            ? { resolution: "verified-covered-existing" }
            : {}),
        rust_test: completed.rustTest,
        primary_test_source: sourceIdentity(cases, "java-antlr"),
        alternate_test_source: sourceIdentity(cases, "antlr-ng"),
        primary_implementation_source: `antlr-ng@${ANTLR_NG_COMMIT}`,
        alternate_implementation_source: `java-antlr@${JAVA_COMMIT}`,
        prerequisites: ["Phase B compiler boundary"],
        unit_under_test: policy.unit,
        expected_red_failure_fingerprint: coveredExisting
            ? "not applicable: the case-specific port passed against the existing Phase B compiler"
            : completed.redFingerprint,
        observable_equivalence: observable,
        scaffold_commit: completed.scaffoldCommit,
        primary_test_commit: completed.testCommit,
        ...(coveredExisting
            ? {
                  verified_covered_existing: {
                      command: completed.testCommand,
                      commit: completed.testCommit,
                      exit_code: 0,
                      result: completed.greenResult,
                  },
              }
            : {
                  demonstrated_red: {
                      command: completed.testCommand,
                      exit_code: 101,
                      fingerprint: completed.redFingerprint,
                  },
              }),
        primary_implementation_commit: completed.implementationCommit,
        green_result: {
            command: completed.testCommand,
            result: completed.greenResult,
        },
        closure,
        closure_sha256: digest(stableStringify(closure)),
        evidence_path: `tests/codegen-direct/port-evidence/${logicalId}`,
    };
}

async function loadCompletedPhaseBPorts() {
    const source = await readFile(
        resolve(repoRoot, ATN_SERIALIZATION_TEST_MODULE),
        "utf8",
    );
    const ports = new Map();
    const serializationPattern =
        /case!\(\s*(\w+),\s*(parser|lexer),\s*"(testatnserialization-[^"]+)",\s*"[^"]+"\s*\);/gu;
    for (const match of source.matchAll(serializationPattern)) {
        const [, moduleName, , logicalId] = match;
        ports.set(logicalId, {
            fixturePaths: await fixturePaths(logicalId),
            rustTest:
                "grammar::atn::interp_test::tests::upstream_atn_serialization::" +
                `${moduleName}::matches_java`,
            kind: "atn-serialization",
            resolution: "verified-covered-existing",
            scaffoldCommit: PHASE_B_BASE_COMMIT,
            testCommit: ATN_SERIALIZATION_TEST_COMMIT,
            implementationCommit: PHASE_B_IMPLEMENTATION_COMMIT,
            testCommand: ATN_SERIALIZATION_TEST_COMMAND,
            greenResult: "36 passed; 0 failed",
        });
    }
    const serializationCount = ports.size;
    if (serializationCount !== 36) {
        throw new Error(
            `expected 36 completed TestATNSerialization ports, found ${serializationCount}`,
        );
    }

    const constructionPattern =
        /(?:case|partial_case|error_case)!\(\s*(\w+),\s*"(testatnconstruction-[^"]+)"(?:,\s*"[^"]+")?\s*\);/gu;
    let constructionCount = 0;
    for (const match of source.matchAll(constructionPattern)) {
        const [, moduleName, logicalId] = match;
        const redFingerprint = ATN_CONSTRUCTION_RED_CASES.get(logicalId);
        ports.set(logicalId, {
            fixturePaths: await fixturePaths(logicalId),
            rustTest:
                "grammar::atn::interp_test::tests::upstream_atn_construction::" +
                `${moduleName}::matches_java`,
            kind: "atn-construction",
            resolution: redFingerprint
                ? "ported"
                : "verified-covered-existing",
            scaffoldCommit: ATN_CONSTRUCTION_BASE_COMMIT,
            testCommit: ATN_CONSTRUCTION_TEST_COMMIT,
            implementationCommit: redFingerprint
                ? ATN_CONSTRUCTION_IMPLEMENTATION_COMMIT
                : PHASE_B_IMPLEMENTATION_COMMIT,
            testCommand: redFingerprint
                ? `${ATN_CONSTRUCTION_TEST_COMMAND}${moduleName}`
                : ATN_CONSTRUCTION_COVERED_COMMAND,
            greenResult: redFingerprint
                ? "1 passed; 0 failed"
                : "38 passed; 0 failed",
            redFingerprint,
        });
        constructionCount += 1;
    }
    if (constructionCount !== 40) {
        throw new Error(
            `expected 40 completed TestATNConstruction ports, found ${constructionCount}`,
        );
    }

    for (const [logicalId, definition] of BASIC_SEMANTIC_PORTS) {
        ports.set(logicalId, {
            fixturePaths: await fixturePaths(logicalId),
            rustTest: definition.rustTest,
            kind: "basic-semantic-errors",
            resolution: "ported",
            scaffoldCommit: BASIC_SEMANTIC_BASE_COMMIT,
            testCommit: BASIC_SEMANTIC_TEST_COMMIT,
            implementationCommit: BASIC_SEMANTIC_IMPLEMENTATION_COMMIT,
            testCommand: BASIC_SEMANTIC_TEST_COMMAND,
            greenResult: "3 passed; 0 failed",
            redFingerprint: definition.redFingerprint,
        });
    }
    for (const [logicalId, definition] of ERROR_SETS_PORTS) {
        ports.set(logicalId, {
            fixturePaths: await fixturePaths(logicalId),
            rustTest: definition.rustTest,
            kind: "error-sets",
            resolution: "ported",
            scaffoldCommit: ERROR_SETS_BASE_COMMIT,
            testCommit: ERROR_SETS_TEST_COMMIT,
            implementationCommit: ERROR_SETS_IMPLEMENTATION_COMMIT,
            testCommand: ERROR_SETS_TEST_COMMAND,
            greenResult: "2 passed; 0 failed",
            redFingerprint: definition.redFingerprint,
        });
    }
    for (const [logicalId, definition] of TOKEN_POSITION_PORTS) {
        ports.set(logicalId, {
            fixturePaths: await fixturePaths(logicalId),
            rustTest: definition.rustTest,
            kind: "token-position-options",
            resolution: definition.resolution,
            scaffoldCommit: TOKEN_POSITION_BASE_COMMIT,
            testCommit: TOKEN_POSITION_TEST_COMMIT,
            implementationCommit: definition.implementationCommit,
            testCommand: definition.testCommand,
            greenResult: definition.greenResult,
            redFingerprint: definition.redFingerprint,
        });
    }
    for (const [logicalId, testName] of TOPOLOGICAL_SORT_PORTS) {
        ports.set(logicalId, {
            fixturePaths: [],
            rustTest:
                "grammar::loader::tests::upstream_topological_sort::" +
                testName,
            kind: "topological-sort",
            resolution: "verified-covered-existing",
            scaffoldCommit: TOPOLOGICAL_SORT_BASE_COMMIT,
            testCommit: TOPOLOGICAL_SORT_TEST_COMMIT,
            implementationCommit: TOPOLOGICAL_SORT_BASE_COMMIT,
            testCommand: TOPOLOGICAL_SORT_TEST_COMMAND,
            greenResult: "5 passed; 0 failed",
        });
    }
    for (const [logicalId, definition] of VOCABULARY_PORTS) {
        ports.set(logicalId, {
            fixturePaths: [],
            rustTest: definition.rustTest,
            kind: "vocabulary",
            resolution: "ported",
            scaffoldCommit: definition.scaffoldCommit,
            testCommit: definition.testCommit,
            implementationCommit: definition.implementationCommit,
            testCommand: definition.testCommand,
            greenResult: definition.greenResult,
            redFingerprint: definition.redFingerprint,
        });
    }
    for (const [logicalId, definition] of CHAR_SUPPORT_PORTS) {
        ports.set(logicalId, {
            fixturePaths: [],
            rustTest: "grammar::char_support::tests::" + definition.testName,
            kind: "char-support",
            resolution: "ported",
            scaffoldCommit: CHAR_SUPPORT_BASE_COMMIT,
            testCommit: CHAR_SUPPORT_TEST_COMMIT,
            implementationCommit: CHAR_SUPPORT_IMPLEMENTATION_COMMIT,
            testCommand: CHAR_SUPPORT_TEST_COMMAND,
            greenResult: "8 passed; 0 failed",
            redFingerprint:
                `E0425: cannot find function \`${definition.missingFunction}\` ` +
                "in this scope",
        });
    }
    ports.set(NESTED_ACTION_LOGICAL_ID, {
        fixturePaths: [],
        rustTest: "grammar::syntax::tests::nested_actions_match_upstream",
        kind: "nested-action",
        resolution: "ported",
        scaffoldCommit: NESTED_ACTION_BASE_COMMIT,
        testCommit: NESTED_ACTION_TEST_COMMIT,
        implementationCommit: NESTED_ACTION_IMPLEMENTATION_COMMIT,
        testCommand: NESTED_ACTION_TEST_COMMAND,
        greenResult: "1 passed; 0 failed",
        redFingerprint:
            "predicate fail message retained grammar quotes: " +
            "left Some(\"'custom message'\"), right Some(\"custom message\")",
    });
    const escapeSequenceGroups = new Map();
    for (const testCase of inventory.cases) {
        if (testCase.suite !== "TestEscapeSequenceParsing") {
            continue;
        }
        const key =
            `${testCase.suite}\0${canonicalName(testCase.name)}` +
            `\0${parameterKey(testCase)}`;
        const group = escapeSequenceGroups.get(key) ?? [];
        group.push(testCase);
        escapeSequenceGroups.set(key, group);
    }
    for (const [key, cases] of escapeSequenceGroups) {
        const logicalId = logicalCaseId(
            cases[0].suite,
            cases[0].name,
            key,
        );
        const testName = escapeSequenceRustTestName(cases[0].name);
        const redFingerprint = ESCAPE_SEQUENCE_RED_CASES.get(
            cases[0].name,
        );
        ports.set(logicalId, {
            fixturePaths: [],
            rustTest:
                `grammar::escape_sequence::tests::${testName}`,
            kind: "escape-sequence",
            resolution: redFingerprint
                ? "ported"
                : "verified-covered-existing",
            scaffoldCommit: ESCAPE_SEQUENCE_SCAFFOLD_COMMIT,
            testCommit: ESCAPE_SEQUENCE_TEST_COMMIT,
            implementationCommit: redFingerprint
                ? ESCAPE_SEQUENCE_IMPLEMENTATION_COMMIT
                : ESCAPE_SEQUENCE_SCAFFOLD_COMMIT,
            testCommand:
                `${ESCAPE_SEQUENCE_TEST_PREFIX}${testName} -- --exact`,
            greenResult: "1 passed; 0 failed",
            redFingerprint,
        });
    }
    if (escapeSequenceGroups.size !== 17) {
        throw new Error(
            `expected 17 completed TestEscapeSequenceParsing ports, found ${escapeSequenceGroups.size}`,
        );
    }
    const unicodeEscapeGroups = new Map();
    for (const testCase of inventory.cases) {
        if (testCase.suite !== "TestUnicodeEscapes") {
            continue;
        }
        const key =
            `${testCase.suite}\0${canonicalName(testCase.name)}` +
            `\0${parameterKey(testCase)}`;
        const group = unicodeEscapeGroups.get(key) ?? [];
        group.push(testCase);
        unicodeEscapeGroups.set(key, group);
    }
    for (const [key, cases] of unicodeEscapeGroups) {
        const logicalId = logicalCaseId(
            cases[0].suite,
            cases[0].name,
            key,
        );
        const testName = escapeSequenceRustTestName(cases[0].name);
        const expected = UNICODE_ESCAPE_EXPECTED.get(cases[0].name);
        if (expected === undefined) {
            throw new Error(
                `missing Unicode escape expectation for ${cases[0].name}`,
            );
        }
        ports.set(logicalId, {
            fixturePaths: [],
            rustTest:
                `grammar::unicode_escape::tests::${testName}`,
            kind: "unicode-escape",
            resolution: "ported",
            scaffoldCommit: UNICODE_ESCAPE_SCAFFOLD_COMMIT,
            testCommit: UNICODE_ESCAPE_TEST_COMMIT,
            implementationCommit:
                UNICODE_ESCAPE_IMPLEMENTATION_COMMIT,
            testCommand:
                `${UNICODE_ESCAPE_TEST_PREFIX}${testName} -- --exact`,
            greenResult: "1 passed; 0 failed",
            redFingerprint:
                `left empty string, right ${JSON.stringify(expected)}`,
        });
    }
    if (unicodeEscapeGroups.size !== 9) {
        throw new Error(
            `expected 9 completed TestUnicodeEscapes ports, found ${unicodeEscapeGroups.size}`,
        );
    }
    const unicodeDataGroups = new Map();
    for (const testCase of inventory.cases) {
        if (testCase.suite !== "TestUnicodeData") {
            continue;
        }
        const key =
            `${testCase.suite}\0${canonicalName(testCase.name)}` +
            `\0${parameterKey(testCase)}`;
        const group = unicodeDataGroups.get(key) ?? [];
        group.push(testCase);
        unicodeDataGroups.set(key, group);
    }
    for (const [key, cases] of unicodeDataGroups) {
        const logicalId = logicalCaseId(
            cases[0].suite,
            cases[0].name,
            key,
        );
        const testName = UNICODE_DATA_TEST_NAMES.get(cases[0].name);
        if (testName === undefined) {
            throw new Error(
                `missing Unicode data test mapping for ${cases[0].name}`,
            );
        }
        ports.set(logicalId, {
            fixturePaths: [],
            rustTest: `grammar::unicode::tests::${testName}`,
            kind: "unicode-data",
            resolution: "verified-covered-existing",
            scaffoldCommit: UNICODE_DATA_BASE_COMMIT,
            testCommit: UNICODE_DATA_TEST_COMMIT,
            implementationCommit: UNICODE_DATA_BASE_COMMIT,
            testCommand:
                `${UNICODE_DATA_TEST_PREFIX}${testName} -- --exact`,
            greenResult: "1 passed; 0 failed",
        });
    }
    if (unicodeDataGroups.size !== 18) {
        throw new Error(
            `expected 18 completed TestUnicodeData ports, found ${unicodeDataGroups.size}`,
        );
    }
    for (const [logicalId, definition] of UNICODE_GRAMMAR_PORTS) {
        ports.set(logicalId, {
            fixturePaths: await fixturePaths(logicalId),
            rustTest:
                "grammar::atn::interp_test::tests::upstream_unicode_grammar::" +
                `${definition.testName}::matches_java_interps`,
            kind: "unicode-grammar",
            resolution: definition.resolution,
            scaffoldCommit: UNICODE_GRAMMAR_BASE_COMMIT,
            testCommit: UNICODE_GRAMMAR_TEST_COMMIT,
            implementationCommit:
                definition.resolution === "ported"
                    ? UNICODE_GRAMMAR_IMPLEMENTATION_COMMIT
                    : UNICODE_GRAMMAR_BASE_COMMIT,
            testCommand:
                `${UNICODE_GRAMMAR_TEST_PREFIX}${definition.testName}` +
                "::matches_java_interps -- --exact",
            greenResult: "1 passed; 0 failed",
            redFingerprint: definition.redFingerprint,
        });
    }
    if (UNICODE_GRAMMAR_PORTS.size !== 6) {
        throw new Error(
            `expected 6 completed TestUnicodeGrammar ports, found ${UNICODE_GRAMMAR_PORTS.size}`,
        );
    }
    for (const [logicalId, definition] of TOKEN_ASSIGNMENT_PORTS) {
        ports.set(logicalId, {
            fixturePaths: await fixturePaths(logicalId),
            rustTest:
                "grammar::atn::interp_test::tests::upstream_token_type_assignment::" +
                `${definition.testName}::matches_java_interps_and_tokens`,
            kind: "token-type-assignment",
            resolution: definition.resolution,
            scaffoldCommit: TOKEN_ASSIGNMENT_BASE_COMMIT,
            testCommit: TOKEN_ASSIGNMENT_TEST_COMMIT,
            implementationCommit:
                definition.resolution === "ported"
                    ? TOKEN_ASSIGNMENT_IMPLEMENTATION_COMMIT
                    : TOKEN_ASSIGNMENT_BASE_COMMIT,
            testCommand:
                `${TOKEN_ASSIGNMENT_TEST_PREFIX}${definition.testName}` +
                "::matches_java_interps_and_tokens -- --exact",
            greenResult: "1 passed; 0 failed",
            redFingerprint: definition.redFingerprint,
        });
    }
    if (TOKEN_ASSIGNMENT_PORTS.size !== 11) {
        throw new Error(
            `expected 11 completed TestTokenTypeAssignment ports, found ${TOKEN_ASSIGNMENT_PORTS.size}`,
        );
    }
    for (const [logicalId, definition] of LEFT_RECURSION_PORTS) {
        ports.set(logicalId, {
            fixturePaths: await fixturePaths(logicalId),
            rustTest:
                "grammar::atn::interp_test::tests::upstream_left_recursion_tool_issues::" +
                `${definition.testName}::${definition.testFunction}`,
            kind: "left-recursion-tool-issues",
            resolution: definition.resolution,
            scaffoldCommit: LEFT_RECURSION_BASE_COMMIT,
            testCommit: LEFT_RECURSION_TEST_COMMIT,
            implementationCommit:
                definition.resolution === "ported"
                    ? LEFT_RECURSION_IMPLEMENTATION_COMMIT
                    : LEFT_RECURSION_BASE_COMMIT,
            testCommand:
                `${LEFT_RECURSION_TEST_PREFIX}${definition.testName}::` +
                `${definition.testFunction} -- --exact`,
            greenResult: "1 passed; 0 failed",
            redFingerprint: definition.redFingerprint,
        });
    }
    if (LEFT_RECURSION_PORTS.size !== 7) {
        throw new Error(
            `expected 7 completed TestLeftRecursionToolIssues ports, found ${LEFT_RECURSION_PORTS.size}`,
        );
    }
    for (const [logicalId, definition] of LOOKAHEAD_TREE_PORTS) {
        ports.set(logicalId, {
            fixturePaths: await fixturePaths(logicalId),
            rustTest:
                "grammar::atn::interp_test::tests::upstream_lookahead_trees::" +
                definition.testName,
            kind: "lookahead-trees",
            resolution: "ported",
            scaffoldCommit: LOOKAHEAD_TREE_FIXTURE_COMMIT,
            testCommit: LOOKAHEAD_TREE_TEST_COMMIT,
            implementationCommit: LOOKAHEAD_TREE_IMPLEMENTATION_COMMIT,
            testCommand:
                `${LOOKAHEAD_TREE_TEST_PREFIX}${definition.testName} -- --exact`,
            greenResult: "1 passed; 0 failed",
            redFingerprint: definition.redFingerprint,
        });
    }
    if (LOOKAHEAD_TREE_PORTS.size !== 4) {
        throw new Error(
            `expected 4 completed TestLookaheadTrees ports, found ${LOOKAHEAD_TREE_PORTS.size}`,
        );
    }
    const scopeGroups = new Map();
    for (const testCase of inventory.cases) {
        if (testCase.suite !== "TestScopeParsing") {
            continue;
        }
        const key =
            `${testCase.suite}\0${canonicalName(testCase.name)}` +
            `\0${parameterKey(testCase)}`;
        const group = scopeGroups.get(key) ?? [];
        group.push(testCase);
        scopeGroups.set(key, group);
    }
    for (const [key, cases] of scopeGroups) {
        const logicalId = logicalCaseId(
            cases[0].suite,
            cases[0].name,
            key,
        );
        ports.set(logicalId, {
            fixturePaths: [],
            rustTest:
                "embedded::tests::upstream_scope_parsing::argument_declarations_match_java",
            kind: "scope-parsing",
            resolution: "ported",
            scaffoldCommit: SCOPE_PARSING_BASE_COMMIT,
            testCommit: SCOPE_PARSING_TEST_COMMIT,
            implementationCommit: SCOPE_PARSING_IMPLEMENTATION_COMMIT,
            testCommand: SCOPE_PARSING_TEST_COMMAND,
            greenResult: "1 passed; 0 failed",
            redFingerprint:
                "E0425: cannot find function `parse_scope_decls` in this scope",
        });
    }
    if (scopeGroups.size !== 47) {
        throw new Error(
            `expected 47 completed TestScopeParsing ports, found ${scopeGroups.size}`,
        );
    }
    if (ports.size !== 219) {
        throw new Error(`expected 219 completed Phase B ports, found ${ports.size}`);
    }
    return ports;
}

async function fixturePaths(logicalId) {
    const fixtureBase = `tests/codegen-direct/fixtures/${logicalId}`;
    const manifest = JSON.parse(
        await readFile(resolve(repoRoot, fixtureBase, "fixture.json"), "utf8"),
    );
    return [
        `${fixtureBase}/fixture.json`,
        ...Object.keys(manifest.files ?? {}).map(
            (path) => `${fixtureBase}/${path}`,
        ),
    ].sort();
}

function policyFor(suite, name) {
    if (COVERED_EXISTING.has(suite)) {
        return {
            owner: "existing",
            disposition: "covered-existing",
            rationale: COVERED_EXISTING.get(suite),
            evidence:
                "cargo test --locked and existing runtime/generator unit tests",
        };
    }
    if (OUT_OF_SCOPE.has(suite)) {
        return {
            owner: "existing",
            disposition: "out-of-scope",
            rationale: OUT_OF_SCOPE.get(suite),
            evidence: "docs/issue-141-direct-g4-codegen-plan.md section 11.5",
        };
    }
    if (suite === "General" || suite === "TestToolSyntaxErrors") {
        return {
            owner: "B",
            disposition: "port",
            unit: "semantic checks or post-parse compiler diagnostics",
        };
    }
    if (suite === "TestLexerActions") {
        const structural = canonicalName(name).includes("nestedactions");
        return {
            owner: structural ? "B" : "C",
            disposition: "port",
            unit: structural
                ? "structural lexer action collection"
                : "compiled lexer action behavior",
        };
    }
    if (PHASE_B_SUITES.has(suite)) {
        return {
            owner: "B",
            disposition: "port",
            unit: "direct grammar semantic pipeline and ATN construction",
        };
    }
    if (PHASE_C_SUITES.has(suite)) {
        return {
            owner: "C",
            disposition: "port",
            unit: "wired source-only compiler behavior",
        };
    }
    throw new Error(`no test-map policy for ${suite}.${name}`);
}

function sourceIdentity(cases, implementation) {
    const ids = cases
        .filter((testCase) => testCase.implementation === implementation)
        .map((testCase) => testCase.id)
        .sort();
    if (ids.length > 0) {
        return {
            implementation,
            commit:
                implementation === "java-antlr" ? JAVA_COMMIT : ANTLR_NG_COMMIT,
            source_case_ids: ids,
        };
    }
    return {
        implementation: "independent-generated-oracle",
        commit:
            implementation === "java-antlr" ? JAVA_COMMIT : ANTLR_NG_COMMIT,
        source_case_ids: [],
        reason: `no ${implementation} source case exposes this exact observable`,
    };
}

function logicalCaseId(suite, name, key) {
    const base = `${slug(suite)}-${slug(name)}`.slice(0, 88);
    return `${base}-${digest(key).slice(0, 10)}`;
}

function parameterKey(testCase) {
    if (testCase.parameters?.index !== null && testCase.parameters?.index !== undefined) {
        return `index:${testCase.parameters.index}`;
    }
    if (testCase.parameters?.rendered_title) {
        return `title:${canonicalName(testCase.parameters.rendered_title)}`;
    }
    return "";
}

function canonicalName(name) {
    return name.normalize("NFKD").toLowerCase().replaceAll(/[^a-z0-9]+/gu, "");
}

function escapeSequenceRustTestName(name) {
    return name
        .replace(/^test/u, "")
        .replace(/([A-Z]+)([A-Z][a-z])/gu, "$1_$2")
        .replace(/([a-z0-9])([A-Z])/gu, "$1_$2")
        .toLowerCase() + "_matches_java";
}

function slug(value) {
    return value
        .normalize("NFKD")
        .toLowerCase()
        .replaceAll(/[^a-z0-9]+/gu, "-")
        .replaceAll(/^-|-$/gu, "");
}

function compareSourceCases(left, right) {
    return (
        left.implementation.localeCompare(right.implementation) ||
        left.source.path.localeCompare(right.source.path) ||
        left.source.line - right.source.line ||
        left.id.localeCompare(right.id)
    );
}
