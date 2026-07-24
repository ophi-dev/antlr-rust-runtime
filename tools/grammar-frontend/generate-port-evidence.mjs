#!/usr/bin/env node

import {
    mkdir,
    readFile,
    writeFile,
} from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
    ANTLR_NG_COMMIT,
    ATTRIBUTE_CHECKS_BASE_COMMIT,
    ATTRIBUTE_CHECKS_BASE_PARENT_COMMIT,
    ATTRIBUTE_CHECKS_IMPLEMENTATION_COMMIT,
    ATTRIBUTE_CHECKS_TEST_COMMIT,
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
    COMPOSITE_GRAMMARS_BASE_COMMIT,
    COMPOSITE_GRAMMARS_BASE_PARENT_COMMIT,
    COMPOSITE_GRAMMARS_IMPLEMENTATION_COMMIT,
    COMPOSITE_GRAMMARS_IMPLEMENTATION_PARENT_COMMIT,
    COMPOSITE_GRAMMARS_TEST_COMMIT,
    EMPTY_VOCABULARY_BASE_COMMIT,
    EMPTY_VOCABULARY_IMPLEMENTATION_COMMIT,
    EMPTY_VOCABULARY_TEST_COMMIT,
    ERROR_SETS_BASE_COMMIT,
    ERROR_SETS_IMPLEMENTATION_COMMIT,
    ERROR_SETS_TEST_COMMIT,
    ESCAPE_SEQUENCE_IMPLEMENTATION_COMMIT,
    ESCAPE_SEQUENCE_SCAFFOLD_COMMIT,
    ESCAPE_SEQUENCE_SCAFFOLD_PARENT_COMMIT,
    ESCAPE_SEQUENCE_TEST_COMMIT,
    FRONTEND_SYNTAX_TEST_COMMIT,
    FRONTEND_SYNTAX_TEST_PARENT,
    GENERAL_ATN_DOT_BASE_COMMIT,
    GENERAL_ATN_DOT_BASE_PARENT_COMMIT,
    GENERAL_ATN_DOT_TEST_COMMIT,
    GRAPH_NODES_BASE_COMMIT,
    GRAPH_NODES_BASE_PARENT_COMMIT,
    GRAPH_NODES_IMPLEMENTATION_COMMIT,
    GRAPH_NODES_TEST_COMMIT,
    IMPLEMENTATION_COMMIT,
    JAVA_COMMIT,
    LEFT_RECURSION_BASE_COMMIT,
    LEFT_RECURSION_BASE_PARENT_COMMIT,
    LEFT_RECURSION_FIXTURE_COMMIT,
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
    PHASE_C_BASE_COMMIT,
    PHASE_C_BASE_PARENT_COMMIT,
    PHASE_C_IMPLEMENTATION_COMMIT,
    PHASE_C_TEST_COMMIT,
    SCAFFOLD_COMMIT,
    SCOPE_PARSING_BASE_COMMIT,
    SCOPE_PARSING_IMPLEMENTATION_COMMIT,
    SCOPE_PARSING_TEST_COMMIT,
    SYMBOL_ISSUES_BASE_COMMIT,
    SYMBOL_ISSUES_BASE_PARENT_COMMIT,
    SYMBOL_ISSUES_IMPLEMENTATION_COMMIT,
    SYMBOL_ISSUES_TEST_COMMIT,
    TEST_COMMIT,
    TOKEN_ASSIGNMENT_BASE_COMMIT,
    TOKEN_ASSIGNMENT_BASE_PARENT_COMMIT,
    TOKEN_ASSIGNMENT_FIXTURE_COMMIT,
    TOKEN_ASSIGNMENT_IMPLEMENTATION_COMMIT,
    TOKEN_ASSIGNMENT_TEST_COMMIT,
    TOKEN_POSITION_BASE_COMMIT,
    TOKEN_POSITION_IMPLEMENTATION_COMMIT,
    TOKEN_POSITION_TEST_COMMIT,
    TOOL_SYNTAX_ERRORS_BASE_COMMIT,
    TOOL_SYNTAX_ERRORS_BASE_PARENT_COMMIT,
    TOOL_SYNTAX_ERRORS_IMPLEMENTATION_COMMIT,
    TOOL_SYNTAX_ERRORS_TEST_COMMIT,
    TOPOLOGICAL_SORT_BASE_COMMIT,
    TOPOLOGICAL_SORT_TEST_COMMIT,
    UNICODE_DATA_BASE_COMMIT,
    UNICODE_DATA_BASE_PARENT_COMMIT,
    UNICODE_DATA_TEST_COMMIT,
    UNICODE_ESCAPE_IMPLEMENTATION_COMMIT,
    UNICODE_ESCAPE_SCAFFOLD_COMMIT,
    UNICODE_ESCAPE_TEST_COMMIT,
    UNICODE_GRAMMAR_BASE_COMMIT,
    UNICODE_GRAMMAR_BASE_PARENT_COMMIT,
    UNICODE_GRAMMAR_IMPLEMENTATION_COMMIT,
    UNICODE_GRAMMAR_TEST_COMMIT,
    VOCABULARY_BASE_COMMIT,
    VOCABULARY_IMPLEMENTATION_COMMIT,
    VOCABULARY_TEST_COMMIT,
    VSCODE_COMMIT,
    digest,
    gitShowOptional,
    parseMode,
    stableStringify,
} from "./evidence-common.mjs";

const TEST_COMMAND =
    "cargo test --locked --features codegen --bin antlr4-rust-gen grammar::frontend::tests::";
const TEST_MODULE_PATH = "src/bin_support/grammar/frontend.rs";
const TEST_MODULE_MARKER = "#[cfg(test)]";
const FRONTEND_SYNTAX_TEST_PATH =
    "src/bin_support/grammar/ported_tests.rs";
const FRONTEND_SYNTAX_TEST_MARKER =
    "fn frontend_tool_syntax_cases_match_upstream_outcomes() {";
const FRONTEND_SYNTAX_TEST_END =
    "\n    let rejected = [";
const FRONTEND_SYNTAX_REJECTED_MARKER =
    "    let rejected = [";
const FRONTEND_SYNTAX_REJECTED_END = "\n    ];\n";
const FRONTEND_SYNTAX_MODULE_PATH = "src/bin_support/grammar/mod.rs";
const FRONTEND_SYNTAX_MODULE_MARKER = "#[cfg(test)]\nmod ported_tests;";
const ATN_SERIALIZATION_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const ATN_SERIALIZATION_TEST_START =
    "    mod upstream_atn_serialization {";
const ATN_SERIALIZATION_TEST_END = "\n    fn assert_lexer_fixture";
const ATN_CONSTRUCTION_TEST_START =
    "    mod upstream_atn_construction {";
const ATN_CONSTRUCTION_TEST_END = "\n    struct GraphOracle";
const BASIC_SEMANTIC_TEST_START =
    "    mod upstream_basic_semantic_errors {";
const BASIC_SEMANTIC_TEST_END =
    "\n        const fn expected(";
const ERROR_SETS_TEST_START =
    "    mod upstream_error_sets {";
const ERROR_SETS_TEST_END =
    "\n        const fn expected(";
const TOKEN_POSITION_TEST_START =
    "    mod upstream_token_position_options {";
const TOKEN_POSITION_TEST_END =
    "\n    struct ExpectedSemanticDiagnostic {";
const TOPOLOGICAL_SORT_TEST_PATH =
    "src/bin_support/grammar/loader.rs";
const TOPOLOGICAL_SORT_TEST_START =
    "    mod upstream_topological_sort {";
const TOPOLOGICAL_SORT_TEST_END =
    "\n    struct Fixture {";
const VOCABULARY_TEST_PATH = "src/vocabulary.rs";
const EMPTY_VOCABULARY_TEST_START =
    "        #[test]\n        fn empty_vocabulary_matches_java() {";
const EMPTY_VOCABULARY_TEST_END =
    "\n        #[test]\n        fn vocabulary_from_token_names_matches_java() {";
const TOKEN_NAMES_VOCABULARY_TEST_START =
    "        #[test]\n        fn vocabulary_from_token_names_matches_java() {";
const TOKEN_NAMES_VOCABULARY_TEST_END =
    "\n            );\n";
const SCOPE_PARSING_TEST_PATH = "src/bin_support/embedded.rs";
const SCOPE_PARSING_TEST_START =
    "    mod upstream_scope_parsing {";
const SCOPE_PARSING_TEST_END =
    "\n    #[test]\n    fn translates_attr_and_rule_reads()";
const CHAR_SUPPORT_TEST_PATH =
    "src/bin_support/grammar/char_support.rs";
const CHAR_SUPPORT_TEST_MARKER = "mod tests {";
const NESTED_ACTION_TEST_PATH =
    "src/bin_support/grammar/syntax.rs";
const NESTED_ACTION_TEST_MARKER =
    "        let rule = unit\n" +
    "            .rules\n" +
    "            .iter()\n" +
    '            .find(|rule| rule.name == "a")';
const NESTED_ACTION_TEST_END = "\n    }\n\n    #[test]";
const NESTED_ACTION_LOGICAL_ID =
    "testlexeractions-nested-actions-3d175db5e5";
const ESCAPE_SEQUENCE_TEST_PATH =
    "src/bin_support/grammar/escape_sequence.rs";
const ESCAPE_SEQUENCE_TEST_MARKER = "mod tests {";
const ESCAPE_SEQUENCE_TEST_END =
    "\n    #[test]\n    fn parse_unicode_property_inverted_matches_java()";
const UNICODE_ESCAPE_TEST_PATH =
    "src/bin_support/grammar/unicode_escape.rs";
const UNICODE_ESCAPE_TEST_MARKER = "mod tests {";
const UNICODE_DATA_TEST_PATH =
    "src/bin_support/grammar/unicode.rs";
const UNICODE_DATA_TEST_MARKER = "mod tests {";
const UNICODE_GRAMMAR_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const UNICODE_GRAMMAR_TEST_START =
    "    mod upstream_unicode_grammar {";
const UNICODE_GRAMMAR_TEST_END =
    "\n    fn assert_combined_fixture";
const TOKEN_ASSIGNMENT_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const TOKEN_ASSIGNMENT_TEST_START =
    "    mod upstream_token_type_assignment {";
const TOKEN_ASSIGNMENT_TEST_END =
    "\n    fn assert_lexer_interp";
const LEFT_RECURSION_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const LEFT_RECURSION_TEST_START =
    "    mod upstream_left_recursion_tool_issues {";
const LEFT_RECURSION_TEST_END =
    "\n    mod upstream_atn_construction {";
const LOOKAHEAD_TREE_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const LOOKAHEAD_TREE_TEST_START =
    "    mod upstream_lookahead_trees {";
const LOOKAHEAD_TREE_TEST_END =
    "\n        #[derive(Debug)]";
const LOOKAHEAD_TREE_HISTORICAL_TEST_END =
    LOOKAHEAD_TREE_TEST_END;
const PHASE_C_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const PHASE_C_TEST_START =
    "    mod upstream_phase_c_runtime {";
const PHASE_C_TEST_END =
    "\n    mod upstream_left_recursion_tool_issues {";
const PHASE_C_CASES_PATH =
    "tests/codegen-direct/generated/phase-c-runtime-cases.inc.rs";
const PHASE_C_CASES_MARKER =
    "// Generated by tools/grammar-frontend/generate-phase-c-runtime-fixtures.mjs.";
const GRAPH_NODES_TEST_PATH = "src/prediction.rs";
const GRAPH_NODES_TEST_MARKER =
    "    mod upstream_graph_nodes {";
const SYMBOL_ISSUES_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const SYMBOL_ISSUES_TEST_START =
    "    mod upstream_symbol_issues {";
const SYMBOL_ISSUES_TEST_END =
    "\n    #[allow(clippy::too_many_arguments)]\n    mod upstream_composite_grammars {";
const SYMBOL_ISSUES_HISTORICAL_TEST_END =
    "\n    mod upstream_token_position_options {";
const ATTRIBUTE_CHECKS_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const ATTRIBUTE_CHECKS_TEST_START =
    "    mod upstream_attribute_checks {";
const ATTRIBUTE_CHECKS_TEST_END =
    "\n    mod upstream_symbol_issues {";
const ATTRIBUTE_CHECKS_CASES_PATH =
    "tests/codegen-direct/generated/attribute-checks-cases.inc.rs";
const ATTRIBUTE_CHECKS_CASES_MARKER =
    "// Generated by tools/grammar-frontend/generate-attribute-checks-fixtures.mjs.";
const TOOL_SYNTAX_ERRORS_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const TOOL_SYNTAX_ERRORS_TEST_START =
    "    mod upstream_tool_syntax_errors {";
const TOOL_SYNTAX_ERRORS_TEST_END =
    "\n        #[derive(Clone, Copy)]\n        struct ExpectedDiagnostic";
const TOOL_SYNTAX_ERRORS_CASES_PATH =
    "tests/codegen-direct/generated/tool-syntax-errors-cases.inc.rs";
const TOOL_SYNTAX_ERRORS_CASES_MARKER =
    "// Generated by tools/grammar-frontend/generate-tool-syntax-errors-fixtures.mjs.";
const COMPOSITE_GRAMMARS_TEST_PATH =
    "src/bin_support/grammar/atn/interp_test.rs";
const COMPOSITE_GRAMMARS_TEST_START =
    "    mod upstream_composite_grammars {";
const COMPOSITE_GRAMMARS_TEST_END =
    "\n    mod upstream_tool_syntax_errors {";
const COMPOSITE_GRAMMARS_CASES_PATH =
    "tests/codegen-direct/generated/composite-grammars-cases.inc.rs";
const COMPOSITE_GRAMMARS_CASES_MARKER =
    "// Generated by tools/grammar-frontend/generate-composite-grammars-fixtures.mjs.";
const GENERAL_ATN_DOT_TEST_PATH =
    "src/bin_support/grammar/atn/general_bug_test.rs";
const GENERAL_ATN_DOT_TEST_MARKER =
    "use std::collections::{BTreeMap, BTreeSet, VecDeque};";
const GENERAL_ATN_DOT_MODULE_PATH =
    "src/bin_support/grammar/atn/mod.rs";
const GENERAL_ATN_DOT_MODULE_MARKER =
    "#[cfg(test)]\nmod general_bug_test;";
const GENERAL_ATN_DOT_MODULE_END =
    "\n#[cfg(test)]\nmod interp_test;";
const GENERAL_ATN_DOT_LOGICAL_IDS = new Set([
    "general-bug-33-escaping-issues-with-backslash-in-dot-file-comparison-1a90edd812",
    "general-bug-35-tool-crashes-with-atn-4bd74f316f",
]);
const EMPTY_VOCABULARY_LOGICAL_ID =
    "testvocabulary-testemptyvocabulary-66d31ad014";
const SYMBOL_INFO_SHA256 =
    "df274a0dca42823cc2ef2608d98d544be53246a48c56f96050b0a987ce0890f3";

const EXTERNAL_DEFINITIONS = {
    "vscode-tparser-source-spans": {
        source_test: {
            repository: "https://github.com/mike-lischke/vscode-antlr4.git",
            commit: VSCODE_COMMIT,
            path: "tests/backend/symbol-info.spec.ts",
            case: "Symbol ranges",
            sha256: SYMBOL_INFO_SHA256,
        },
        canonical_input:
            "tests/codegen-direct/external/vscode-antlr4/tests/backend/test-data/TParser.g4",
        expected_observable: {
            named_action_bytes: [1090, 1264],
            parser_rule_bytes: [3421, 3650],
            argument_block_bytes: [3484, 3511],
        },
        alternate_outcome:
            "antlr-ng grammar frontend nodes preserve the same token boundaries",
        java_verdict:
            "not-applicable: Java has source intervals but not the extension enclosing-symbol API",
    },
    "vscode-symbol-info-malformed-edit": {
        source_test: {
            repository: "https://github.com/mike-lischke/vscode-antlr4.git",
            commit: VSCODE_COMMIT,
            path: "tests/backend/symbol-info.spec.ts",
            case: "reparse: malformed a:: edit",
            sha256: SYMBOL_INFO_SHA256,
        },
        canonical_input: "grammar A; a:: b \n| c; c: b+;",
        expected_observable: {
            result: "fail-closed",
            parser_diagnostic_bytes: [
                [12, 14],
                [18, 19],
                [21, 22]
            ],
        },
        alternate_outcome:
            "antlr-ng reports grammar syntax errors and does not supply a transformable CST",
        java_verdict:
            "Java 4.13.2 also rejects the malformed grammar; exact editor ranges are extension-owned",
    },
    "vscode-symbol-info-valid-undefined-edit": {
        source_test: {
            repository: "https://github.com/mike-lischke/vscode-antlr4.git",
            commit: VSCODE_COMMIT,
            path: "tests/backend/symbol-info.spec.ts",
            case: "reparse: valid undefined-b edit",
            sha256: SYMBOL_INFO_SHA256,
        },
        canonical_input: "grammar A; a: b \n| c; c: b+;",
        expected_observable: {
            result: "usable-cst",
            root_bytes: [0, 28],
        },
        alternate_outcome:
            "antlr-ng returns a grammar CST before later undefined-rule diagnostics",
        java_verdict:
            "Java 4.13.2 accepts the syntax and reports undefined rules during semantics",
    },
};

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const update = parseMode(
    process.argv.slice(2),
    "generate-port-evidence.mjs",
);
const externalMapPath = resolve(
    repoRoot,
    "tests/codegen-direct/external-fixture-map.json",
);
const upstreamInventory = await load(
    "tests/codegen-direct/upstream-case-inventory.json",
);
const externalInventory = await load(
    "tests/codegen-direct/external-source-inventory.json",
);
const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const externalMap = await load("tests/codegen-direct/external-fixture-map.json");
const sourceCases = new Map(
    upstreamInventory.cases.map((testCase) => [testCase.id, testCase]),
);
const externalSources = new Map(
    externalInventory.artifacts.map((artifact) => [artifact.source_id, artifact]),
);
const completedRows = testMap.rows.filter(
    (row) => row.disposition === "port" && row.tdd_state === "done",
);
const expectedFiles = new Map();

const checkedInTestModule = sectionAtMarker(
    await readFile(resolve(repoRoot, TEST_MODULE_PATH), "utf8"),
    TEST_MODULE_MARKER,
);
const testModule = gitShowOptional(repoRoot, TEST_COMMIT, TEST_MODULE_PATH);
const implementationTestModule = gitShowOptional(
    repoRoot,
    IMPLEMENTATION_COMMIT,
    TEST_MODULE_PATH,
);
if (testModule === null) {
    warnMissingHistoricalSource(
        "locked frontend test verification",
        TEST_COMMIT,
        TEST_MODULE_PATH,
    );
}
if (implementationTestModule === null) {
    warnMissingHistoricalSource(
        "locked frontend implementation verification",
        IMPLEMENTATION_COMMIT,
        TEST_MODULE_PATH,
    );
}
if (testModule !== null && implementationTestModule !== null) {
    const lockedTestModule = sectionAtMarker(testModule, TEST_MODULE_MARKER);
    const implementedTestModule = sectionAtMarker(
        implementationTestModule,
        TEST_MODULE_MARKER,
    );
    if (lockedTestModule !== implementedTestModule) {
        throw new Error(
            "implementation commit changed the locked frontend test module",
        );
    }
    if (lockedTestModule !== checkedInTestModule) {
        throw new Error("checked-in frontend tests differ from the locked tests");
    }
}
const lockedTestModuleHash = digest(checkedInTestModule);
const checkedInSyntaxSource = await readFile(
    resolve(repoRoot, FRONTEND_SYNTAX_TEST_PATH),
    "utf8",
);
const checkedInSyntaxTest = sectionBetweenMarkers(
    checkedInSyntaxSource,
    FRONTEND_SYNTAX_TEST_MARKER,
    FRONTEND_SYNTAX_TEST_END,
);
const checkedInRejectedSyntaxCases = sectionBetweenMarkers(
    checkedInSyntaxSource,
    FRONTEND_SYNTAX_REJECTED_MARKER,
    FRONTEND_SYNTAX_REJECTED_END,
);
const recordedSyntaxTest = gitShowOptional(
    repoRoot,
    FRONTEND_SYNTAX_TEST_COMMIT,
    FRONTEND_SYNTAX_TEST_PATH,
);
if (recordedSyntaxTest === null) {
    warnMissingHistoricalSource(
        "frontend syntax test verification",
        FRONTEND_SYNTAX_TEST_COMMIT,
        FRONTEND_SYNTAX_TEST_PATH,
    );
} else if (
    (sectionBetweenMarkers(
        recordedSyntaxTest,
        FRONTEND_SYNTAX_TEST_MARKER,
        FRONTEND_SYNTAX_TEST_END,
    ) !== checkedInSyntaxTest ||
        sectionBetweenMarkers(
            recordedSyntaxTest,
            FRONTEND_SYNTAX_REJECTED_MARKER,
            FRONTEND_SYNTAX_REJECTED_END,
        ) !== checkedInRejectedSyntaxCases)
) {
    throw new Error("checked-in frontend syntax port differs from its test commit");
}
const checkedInSyntaxModule = sectionAtMarker(
    await readFile(resolve(repoRoot, FRONTEND_SYNTAX_MODULE_PATH), "utf8"),
    FRONTEND_SYNTAX_MODULE_MARKER,
);
const recordedSyntaxModule = gitShowOptional(
    repoRoot,
    FRONTEND_SYNTAX_TEST_COMMIT,
    FRONTEND_SYNTAX_MODULE_PATH,
);
if (recordedSyntaxModule === null) {
    warnMissingHistoricalSource(
        "frontend syntax module verification",
        FRONTEND_SYNTAX_TEST_COMMIT,
        FRONTEND_SYNTAX_MODULE_PATH,
    );
} else if (
    sectionAtMarker(recordedSyntaxModule, FRONTEND_SYNTAX_MODULE_MARKER) !==
    checkedInSyntaxModule
) {
    throw new Error("checked-in frontend syntax test module differs from its test commit");
}
const defaultLockedSections = [
    {
        path: TEST_MODULE_PATH,
        marker: TEST_MODULE_MARKER,
        sha256: lockedTestModuleHash,
    },
];
const syntaxLockedSections = [
    {
        path: FRONTEND_SYNTAX_TEST_PATH,
        marker: FRONTEND_SYNTAX_TEST_MARKER,
        end_marker: FRONTEND_SYNTAX_TEST_END,
        sha256: digest(checkedInSyntaxTest),
    },
    {
        path: FRONTEND_SYNTAX_TEST_PATH,
        marker: FRONTEND_SYNTAX_REJECTED_MARKER,
        end_marker: FRONTEND_SYNTAX_REJECTED_END,
        sha256: digest(checkedInRejectedSyntaxCases),
    },
    {
        path: FRONTEND_SYNTAX_MODULE_PATH,
        marker: FRONTEND_SYNTAX_MODULE_MARKER,
        sha256: digest(checkedInSyntaxModule),
    },
];
const checkedInAtnSerializationTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, ATN_SERIALIZATION_TEST_PATH), "utf8"),
    ATN_SERIALIZATION_TEST_START,
    ATN_SERIALIZATION_TEST_END,
);
const recordedAtnSerializationTests = gitShowOptional(
    repoRoot,
    ATN_SERIALIZATION_TEST_COMMIT,
    ATN_SERIALIZATION_TEST_PATH,
);
if (recordedAtnSerializationTests === null) {
    warnMissingHistoricalSource(
        "ATN serialization test verification",
        ATN_SERIALIZATION_TEST_COMMIT,
        ATN_SERIALIZATION_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        recordedAtnSerializationTests,
        ATN_SERIALIZATION_TEST_START,
        ATN_SERIALIZATION_TEST_END,
    ) !== checkedInAtnSerializationTests
) {
    throw new Error(
        "checked-in ATN serialization ports differ from their test commit",
    );
}
const atnSerializationLockedSections = [
    {
        path: ATN_SERIALIZATION_TEST_PATH,
        marker: ATN_SERIALIZATION_TEST_START,
        end_marker: ATN_SERIALIZATION_TEST_END,
        sha256: digest(checkedInAtnSerializationTests),
    },
];
const checkedInAtnConstructionTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, ATN_SERIALIZATION_TEST_PATH), "utf8"),
    ATN_CONSTRUCTION_TEST_START,
    ATN_CONSTRUCTION_TEST_END,
);
const recordedAtnConstructionTests = gitShowOptional(
    repoRoot,
    ATN_CONSTRUCTION_TEST_COMMIT,
    ATN_SERIALIZATION_TEST_PATH,
);
const implementedAtnConstructionTests = gitShowOptional(
    repoRoot,
    ATN_CONSTRUCTION_IMPLEMENTATION_COMMIT,
    ATN_SERIALIZATION_TEST_PATH,
);
if (recordedAtnConstructionTests === null) {
    warnMissingHistoricalSource(
        "ATN construction test verification",
        ATN_CONSTRUCTION_TEST_COMMIT,
        ATN_SERIALIZATION_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        recordedAtnConstructionTests,
        ATN_CONSTRUCTION_TEST_START,
        ATN_CONSTRUCTION_TEST_END,
    ) !== checkedInAtnConstructionTests
) {
    throw new Error(
        "checked-in ATN construction ports differ from their test commit",
    );
}
if (implementedAtnConstructionTests === null) {
    warnMissingHistoricalSource(
        "ATN construction implementation verification",
        ATN_CONSTRUCTION_IMPLEMENTATION_COMMIT,
        ATN_SERIALIZATION_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        implementedAtnConstructionTests,
        ATN_CONSTRUCTION_TEST_START,
        ATN_CONSTRUCTION_TEST_END,
    ) !== checkedInAtnConstructionTests
) {
    throw new Error(
        "ATN construction implementation changed the locked test ports",
    );
}
const atnConstructionLockedSections = [
    {
        path: ATN_SERIALIZATION_TEST_PATH,
        marker: ATN_CONSTRUCTION_TEST_START,
        end_marker: ATN_CONSTRUCTION_TEST_END,
        sha256: digest(checkedInAtnConstructionTests),
    },
];
const checkedInBasicSemanticTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, ATN_SERIALIZATION_TEST_PATH), "utf8"),
    BASIC_SEMANTIC_TEST_START,
    BASIC_SEMANTIC_TEST_END,
);
const recordedBasicSemanticTests = gitShowOptional(
    repoRoot,
    BASIC_SEMANTIC_TEST_COMMIT,
    ATN_SERIALIZATION_TEST_PATH,
);
const implementedBasicSemanticTests = gitShowOptional(
    repoRoot,
    BASIC_SEMANTIC_IMPLEMENTATION_COMMIT,
    ATN_SERIALIZATION_TEST_PATH,
);
if (recordedBasicSemanticTests === null) {
    warnMissingHistoricalSource(
        "basic semantic test verification",
        BASIC_SEMANTIC_TEST_COMMIT,
        ATN_SERIALIZATION_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        recordedBasicSemanticTests,
        BASIC_SEMANTIC_TEST_START,
        BASIC_SEMANTIC_TEST_END,
    ) !== checkedInBasicSemanticTests
) {
    throw new Error(
        "checked-in basic semantic ports differ from their test commit",
    );
}
if (implementedBasicSemanticTests === null) {
    warnMissingHistoricalSource(
        "basic semantic implementation verification",
        BASIC_SEMANTIC_IMPLEMENTATION_COMMIT,
        ATN_SERIALIZATION_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        implementedBasicSemanticTests,
        BASIC_SEMANTIC_TEST_START,
        BASIC_SEMANTIC_TEST_END,
    ) !== checkedInBasicSemanticTests
) {
    throw new Error(
        "basic semantic implementation changed the locked test ports",
    );
}
const basicSemanticLockedSections = [
    {
        path: ATN_SERIALIZATION_TEST_PATH,
        marker: BASIC_SEMANTIC_TEST_START,
        end_marker: BASIC_SEMANTIC_TEST_END,
        sha256: digest(checkedInBasicSemanticTests),
    },
];
const checkedInErrorSetsTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, ATN_SERIALIZATION_TEST_PATH), "utf8"),
    ERROR_SETS_TEST_START,
    ERROR_SETS_TEST_END,
);
const recordedErrorSetsTests = gitShowOptional(
    repoRoot,
    ERROR_SETS_TEST_COMMIT,
    ATN_SERIALIZATION_TEST_PATH,
);
const implementedErrorSetsTests = gitShowOptional(
    repoRoot,
    ERROR_SETS_IMPLEMENTATION_COMMIT,
    ATN_SERIALIZATION_TEST_PATH,
);
if (recordedErrorSetsTests === null) {
    warnMissingHistoricalSource(
        "lexer set error test verification",
        ERROR_SETS_TEST_COMMIT,
        ATN_SERIALIZATION_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        recordedErrorSetsTests,
        ERROR_SETS_TEST_START,
        ERROR_SETS_TEST_END,
    ) !== checkedInErrorSetsTests
) {
    throw new Error(
        "checked-in lexer set error ports differ from their test commit",
    );
}
if (implementedErrorSetsTests === null) {
    warnMissingHistoricalSource(
        "lexer set error implementation verification",
        ERROR_SETS_IMPLEMENTATION_COMMIT,
        ATN_SERIALIZATION_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        implementedErrorSetsTests,
        ERROR_SETS_TEST_START,
        ERROR_SETS_TEST_END,
    ) !== checkedInErrorSetsTests
) {
    throw new Error(
        "lexer set error implementation changed the locked test ports",
    );
}
const errorSetsLockedSections = [
    {
        path: ATN_SERIALIZATION_TEST_PATH,
        marker: ERROR_SETS_TEST_START,
        end_marker: ERROR_SETS_TEST_END,
        sha256: digest(checkedInErrorSetsTests),
    },
];
const checkedInTokenPositionTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, ATN_SERIALIZATION_TEST_PATH), "utf8"),
    TOKEN_POSITION_TEST_START,
    TOKEN_POSITION_TEST_END,
);
const recordedTokenPositionTests = gitShowOptional(
    repoRoot,
    TOKEN_POSITION_TEST_COMMIT,
    ATN_SERIALIZATION_TEST_PATH,
);
const implementedTokenPositionTests = gitShowOptional(
    repoRoot,
    TOKEN_POSITION_IMPLEMENTATION_COMMIT,
    ATN_SERIALIZATION_TEST_PATH,
);
if (recordedTokenPositionTests === null) {
    warnMissingHistoricalSource(
        "token position test verification",
        TOKEN_POSITION_TEST_COMMIT,
        ATN_SERIALIZATION_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        recordedTokenPositionTests,
        TOKEN_POSITION_TEST_START,
        TOKEN_POSITION_TEST_END,
    ) !== checkedInTokenPositionTests
) {
    throw new Error(
        "checked-in token position ports differ from their test commit",
    );
}
if (implementedTokenPositionTests === null) {
    warnMissingHistoricalSource(
        "token position implementation verification",
        TOKEN_POSITION_IMPLEMENTATION_COMMIT,
        ATN_SERIALIZATION_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        implementedTokenPositionTests,
        TOKEN_POSITION_TEST_START,
        TOKEN_POSITION_TEST_END,
    ) !== checkedInTokenPositionTests
) {
    throw new Error(
        "token position implementation changed the locked test ports",
    );
}
const tokenPositionLockedSections = [
    {
        path: ATN_SERIALIZATION_TEST_PATH,
        marker: TOKEN_POSITION_TEST_START,
        end_marker: TOKEN_POSITION_TEST_END,
        sha256: digest(checkedInTokenPositionTests),
    },
];
const checkedInTopologicalSortTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, TOPOLOGICAL_SORT_TEST_PATH), "utf8"),
    TOPOLOGICAL_SORT_TEST_START,
    TOPOLOGICAL_SORT_TEST_END,
);
const recordedTopologicalSortTests = gitShowOptional(
    repoRoot,
    TOPOLOGICAL_SORT_TEST_COMMIT,
    TOPOLOGICAL_SORT_TEST_PATH,
);
if (recordedTopologicalSortTests === null) {
    warnMissingHistoricalSource(
        "topological sort test verification",
        TOPOLOGICAL_SORT_TEST_COMMIT,
        TOPOLOGICAL_SORT_TEST_PATH,
    );
} else if (
    sectionBetweenMarkers(
        recordedTopologicalSortTests,
        TOPOLOGICAL_SORT_TEST_START,
        TOPOLOGICAL_SORT_TEST_END,
    ) !== checkedInTopologicalSortTests
) {
    throw new Error(
        "checked-in topological sort ports differ from their test commit",
    );
}
const topologicalSortLockedSections = [
    {
        path: TOPOLOGICAL_SORT_TEST_PATH,
        marker: TOPOLOGICAL_SORT_TEST_START,
        end_marker: TOPOLOGICAL_SORT_TEST_END,
        sha256: digest(checkedInTopologicalSortTests),
    },
];
const checkedInEmptyVocabularyTest = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, VOCABULARY_TEST_PATH), "utf8"),
    EMPTY_VOCABULARY_TEST_START,
    EMPTY_VOCABULARY_TEST_END,
);
const recordedEmptyVocabularyTest = gitShowOptional(
    repoRoot,
    EMPTY_VOCABULARY_TEST_COMMIT,
    VOCABULARY_TEST_PATH,
);
const implementedEmptyVocabularyTest = gitShowOptional(
    repoRoot,
    EMPTY_VOCABULARY_IMPLEMENTATION_COMMIT,
    VOCABULARY_TEST_PATH,
);
if (
    recordedEmptyVocabularyTest === null ||
    sectionBetweenMarkers(
        recordedEmptyVocabularyTest,
        EMPTY_VOCABULARY_TEST_START,
        EMPTY_VOCABULARY_TEST_END,
    ) !== checkedInEmptyVocabularyTest
) {
    throw new Error(
        "checked-in empty vocabulary port differs from its test commit",
    );
}
if (
    implementedEmptyVocabularyTest === null ||
    sectionBetweenMarkers(
        implementedEmptyVocabularyTest,
        EMPTY_VOCABULARY_TEST_START,
        EMPTY_VOCABULARY_TEST_END,
    ) !== checkedInEmptyVocabularyTest
) {
    throw new Error(
        "empty vocabulary implementation changed the locked test port",
    );
}
const emptyVocabularyLockedSections = [
    {
        path: VOCABULARY_TEST_PATH,
        marker: EMPTY_VOCABULARY_TEST_START,
        end_marker: EMPTY_VOCABULARY_TEST_END,
        sha256: digest(checkedInEmptyVocabularyTest),
    },
];
const checkedInTokenNamesVocabularyTest = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, VOCABULARY_TEST_PATH), "utf8"),
    TOKEN_NAMES_VOCABULARY_TEST_START,
    TOKEN_NAMES_VOCABULARY_TEST_END,
);
const recordedTokenNamesVocabularyTest = gitShowOptional(
    repoRoot,
    VOCABULARY_TEST_COMMIT,
    VOCABULARY_TEST_PATH,
);
const implementedTokenNamesVocabularyTest = gitShowOptional(
    repoRoot,
    VOCABULARY_IMPLEMENTATION_COMMIT,
    VOCABULARY_TEST_PATH,
);
if (
    recordedTokenNamesVocabularyTest === null ||
    sectionBetweenMarkers(
        recordedTokenNamesVocabularyTest,
        TOKEN_NAMES_VOCABULARY_TEST_START,
        TOKEN_NAMES_VOCABULARY_TEST_END,
    ) !== checkedInTokenNamesVocabularyTest
) {
    throw new Error(
        "checked-in token-names vocabulary port differs from its test commit",
    );
}
if (
    implementedTokenNamesVocabularyTest === null ||
    sectionBetweenMarkers(
        implementedTokenNamesVocabularyTest,
        TOKEN_NAMES_VOCABULARY_TEST_START,
        TOKEN_NAMES_VOCABULARY_TEST_END,
    ) !== checkedInTokenNamesVocabularyTest
) {
    throw new Error(
        "token-names vocabulary implementation changed the locked test port",
    );
}
const tokenNamesVocabularyLockedSections = [
    {
        path: VOCABULARY_TEST_PATH,
        marker: TOKEN_NAMES_VOCABULARY_TEST_START,
        end_marker: TOKEN_NAMES_VOCABULARY_TEST_END,
        sha256: digest(checkedInTokenNamesVocabularyTest),
    },
];
const checkedInScopeParsingTest = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, SCOPE_PARSING_TEST_PATH), "utf8"),
    SCOPE_PARSING_TEST_START,
    SCOPE_PARSING_TEST_END,
);
const recordedScopeParsingTest = gitShowOptional(
    repoRoot,
    SCOPE_PARSING_TEST_COMMIT,
    SCOPE_PARSING_TEST_PATH,
);
const implementedScopeParsingTest = gitShowOptional(
    repoRoot,
    SCOPE_PARSING_IMPLEMENTATION_COMMIT,
    SCOPE_PARSING_TEST_PATH,
);
if (
    recordedScopeParsingTest === null ||
    sectionBetweenMarkers(
        recordedScopeParsingTest,
        SCOPE_PARSING_TEST_START,
        SCOPE_PARSING_TEST_END,
    ) !== checkedInScopeParsingTest
) {
    throw new Error(
        "checked-in scope parsing port differs from its test commit",
    );
}
if (
    implementedScopeParsingTest === null ||
    sectionBetweenMarkers(
        implementedScopeParsingTest,
        SCOPE_PARSING_TEST_START,
        SCOPE_PARSING_TEST_END,
    ) !== checkedInScopeParsingTest
) {
    throw new Error(
        "scope parsing implementation changed the locked test port",
    );
}
const scopeParsingLockedSections = [
    {
        path: SCOPE_PARSING_TEST_PATH,
        marker: SCOPE_PARSING_TEST_START,
        end_marker: SCOPE_PARSING_TEST_END,
        sha256: digest(checkedInScopeParsingTest),
    },
];
const checkedInCharSupportTests = sectionAtMarker(
    await readFile(resolve(repoRoot, CHAR_SUPPORT_TEST_PATH), "utf8"),
    CHAR_SUPPORT_TEST_MARKER,
);
const recordedCharSupportTests = gitShowOptional(
    repoRoot,
    CHAR_SUPPORT_TEST_COMMIT,
    CHAR_SUPPORT_TEST_PATH,
);
const implementedCharSupportTests = gitShowOptional(
    repoRoot,
    CHAR_SUPPORT_IMPLEMENTATION_COMMIT,
    CHAR_SUPPORT_TEST_PATH,
);
if (
    recordedCharSupportTests === null ||
    sectionAtMarker(
        recordedCharSupportTests,
        CHAR_SUPPORT_TEST_MARKER,
    ) !== checkedInCharSupportTests
) {
    throw new Error(
        "checked-in character support ports differ from their test commit",
    );
}
if (
    implementedCharSupportTests === null ||
    sectionAtMarker(
        implementedCharSupportTests,
        CHAR_SUPPORT_TEST_MARKER,
    ) !== checkedInCharSupportTests
) {
    throw new Error(
        "character support implementation changed the locked test ports",
    );
}
const charSupportLockedSections = [
    {
        path: CHAR_SUPPORT_TEST_PATH,
        marker: CHAR_SUPPORT_TEST_MARKER,
        sha256: digest(checkedInCharSupportTests),
    },
];
const checkedInNestedActionTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, NESTED_ACTION_TEST_PATH), "utf8"),
    NESTED_ACTION_TEST_MARKER,
    NESTED_ACTION_TEST_END,
);
const recordedNestedActionTests = gitShowOptional(
    repoRoot,
    NESTED_ACTION_TEST_COMMIT,
    NESTED_ACTION_TEST_PATH,
);
const implementedNestedActionTests = gitShowOptional(
    repoRoot,
    NESTED_ACTION_IMPLEMENTATION_COMMIT,
    NESTED_ACTION_TEST_PATH,
);
if (
    recordedNestedActionTests === null ||
    sectionBetweenMarkers(
        recordedNestedActionTests,
        NESTED_ACTION_TEST_MARKER,
        NESTED_ACTION_TEST_END,
    ) !== checkedInNestedActionTests
) {
    throw new Error(
        "checked-in nested action port differs from its test commit",
    );
}
if (
    implementedNestedActionTests === null ||
    sectionBetweenMarkers(
        implementedNestedActionTests,
        NESTED_ACTION_TEST_MARKER,
        NESTED_ACTION_TEST_END,
    ) !== checkedInNestedActionTests
) {
    throw new Error(
        "nested action implementation changed the locked test port",
    );
}
const nestedActionLockedSections = [
    {
        path: NESTED_ACTION_TEST_PATH,
        marker: NESTED_ACTION_TEST_MARKER,
        end_marker: NESTED_ACTION_TEST_END,
        sha256: digest(checkedInNestedActionTests),
    },
];
const checkedInEscapeSequenceTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, ESCAPE_SEQUENCE_TEST_PATH), "utf8"),
    ESCAPE_SEQUENCE_TEST_MARKER,
    ESCAPE_SEQUENCE_TEST_END,
);
const recordedEscapeSequenceTests = gitShowOptional(
    repoRoot,
    ESCAPE_SEQUENCE_TEST_COMMIT,
    ESCAPE_SEQUENCE_TEST_PATH,
);
const implementedEscapeSequenceTests = gitShowOptional(
    repoRoot,
    ESCAPE_SEQUENCE_IMPLEMENTATION_COMMIT,
    ESCAPE_SEQUENCE_TEST_PATH,
);
if (
    recordedEscapeSequenceTests === null ||
    sectionBetweenMarkers(
        recordedEscapeSequenceTests,
        ESCAPE_SEQUENCE_TEST_MARKER,
        ESCAPE_SEQUENCE_TEST_END,
    ) !== checkedInEscapeSequenceTests
) {
    throw new Error(
        "checked-in escape sequence ports differ from their test commit",
    );
}
if (
    implementedEscapeSequenceTests === null ||
    sectionBetweenMarkers(
        implementedEscapeSequenceTests,
        ESCAPE_SEQUENCE_TEST_MARKER,
        ESCAPE_SEQUENCE_TEST_END,
    ) !== checkedInEscapeSequenceTests
) {
    throw new Error(
        "escape sequence implementation changed the locked test ports",
    );
}
const escapeSequenceLockedSections = [
    {
        path: ESCAPE_SEQUENCE_TEST_PATH,
        marker: ESCAPE_SEQUENCE_TEST_MARKER,
        end_marker: ESCAPE_SEQUENCE_TEST_END,
        sha256: digest(checkedInEscapeSequenceTests),
    },
];
const checkedInUnicodeEscapeTests = sectionAtMarker(
    await readFile(resolve(repoRoot, UNICODE_ESCAPE_TEST_PATH), "utf8"),
    UNICODE_ESCAPE_TEST_MARKER,
);
const recordedUnicodeEscapeTests = gitShowOptional(
    repoRoot,
    UNICODE_ESCAPE_TEST_COMMIT,
    UNICODE_ESCAPE_TEST_PATH,
);
const implementedUnicodeEscapeTests = gitShowOptional(
    repoRoot,
    UNICODE_ESCAPE_IMPLEMENTATION_COMMIT,
    UNICODE_ESCAPE_TEST_PATH,
);
if (
    recordedUnicodeEscapeTests === null ||
    sectionAtMarker(
        recordedUnicodeEscapeTests,
        UNICODE_ESCAPE_TEST_MARKER,
    ) !== checkedInUnicodeEscapeTests
) {
    throw new Error(
        "checked-in Unicode escape ports differ from their test commit",
    );
}
if (
    implementedUnicodeEscapeTests === null ||
    sectionAtMarker(
        implementedUnicodeEscapeTests,
        UNICODE_ESCAPE_TEST_MARKER,
    ) !== checkedInUnicodeEscapeTests
) {
    throw new Error(
        "Unicode escape implementation changed the locked test ports",
    );
}
const unicodeEscapeLockedSections = [
    {
        path: UNICODE_ESCAPE_TEST_PATH,
        marker: UNICODE_ESCAPE_TEST_MARKER,
        sha256: digest(checkedInUnicodeEscapeTests),
    },
];
const checkedInUnicodeDataTests = sectionAtMarker(
    await readFile(resolve(repoRoot, UNICODE_DATA_TEST_PATH), "utf8"),
    UNICODE_DATA_TEST_MARKER,
);
const recordedUnicodeDataTests = gitShowOptional(
    repoRoot,
    UNICODE_DATA_TEST_COMMIT,
    UNICODE_DATA_TEST_PATH,
);
if (
    recordedUnicodeDataTests === null ||
    sectionAtMarker(
        recordedUnicodeDataTests,
        UNICODE_DATA_TEST_MARKER,
    ) !== checkedInUnicodeDataTests
) {
    throw new Error(
        "checked-in Unicode data ports differ from their test commit",
    );
}
const unicodeDataLockedSections = [
    {
        path: UNICODE_DATA_TEST_PATH,
        marker: UNICODE_DATA_TEST_MARKER,
        sha256: digest(checkedInUnicodeDataTests),
    },
];
const checkedInUnicodeGrammarTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, UNICODE_GRAMMAR_TEST_PATH), "utf8"),
    UNICODE_GRAMMAR_TEST_START,
    UNICODE_GRAMMAR_TEST_END,
);
const recordedUnicodeGrammarTests = gitShowOptional(
    repoRoot,
    UNICODE_GRAMMAR_TEST_COMMIT,
    UNICODE_GRAMMAR_TEST_PATH,
);
const implementedUnicodeGrammarTests = gitShowOptional(
    repoRoot,
    UNICODE_GRAMMAR_IMPLEMENTATION_COMMIT,
    UNICODE_GRAMMAR_TEST_PATH,
);
if (
    recordedUnicodeGrammarTests === null ||
    sectionBetweenMarkers(
        recordedUnicodeGrammarTests,
        UNICODE_GRAMMAR_TEST_START,
        UNICODE_GRAMMAR_TEST_END,
    ) !== checkedInUnicodeGrammarTests
) {
    throw new Error(
        "checked-in Unicode grammar ports differ from their test commit",
    );
}
if (
    implementedUnicodeGrammarTests === null ||
    sectionBetweenMarkers(
        implementedUnicodeGrammarTests,
        UNICODE_GRAMMAR_TEST_START,
        UNICODE_GRAMMAR_TEST_END,
    ) !== checkedInUnicodeGrammarTests
) {
    throw new Error(
        "Unicode grammar implementation changed the locked test ports",
    );
}
const unicodeGrammarLockedSections = [
    {
        path: UNICODE_GRAMMAR_TEST_PATH,
        marker: UNICODE_GRAMMAR_TEST_START,
        end_marker: UNICODE_GRAMMAR_TEST_END,
        sha256: digest(checkedInUnicodeGrammarTests),
    },
];
const checkedInTokenAssignmentTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, TOKEN_ASSIGNMENT_TEST_PATH), "utf8"),
    TOKEN_ASSIGNMENT_TEST_START,
    TOKEN_ASSIGNMENT_TEST_END,
);
const recordedTokenAssignmentTests = gitShowOptional(
    repoRoot,
    TOKEN_ASSIGNMENT_TEST_COMMIT,
    TOKEN_ASSIGNMENT_TEST_PATH,
);
const implementedTokenAssignmentTests = gitShowOptional(
    repoRoot,
    TOKEN_ASSIGNMENT_IMPLEMENTATION_COMMIT,
    TOKEN_ASSIGNMENT_TEST_PATH,
);
if (
    recordedTokenAssignmentTests === null ||
    sectionBetweenMarkers(
        recordedTokenAssignmentTests,
        TOKEN_ASSIGNMENT_TEST_START,
        TOKEN_ASSIGNMENT_TEST_END,
    ) !== checkedInTokenAssignmentTests
) {
    throw new Error(
        "checked-in token assignment ports differ from their test commit",
    );
}
if (
    implementedTokenAssignmentTests === null ||
    sectionBetweenMarkers(
        implementedTokenAssignmentTests,
        TOKEN_ASSIGNMENT_TEST_START,
        TOKEN_ASSIGNMENT_TEST_END,
    ) !== checkedInTokenAssignmentTests
) {
    throw new Error(
        "token assignment implementation changed the locked test ports",
    );
}
const tokenAssignmentLockedSections = [
    {
        path: TOKEN_ASSIGNMENT_TEST_PATH,
        marker: TOKEN_ASSIGNMENT_TEST_START,
        end_marker: TOKEN_ASSIGNMENT_TEST_END,
        sha256: digest(checkedInTokenAssignmentTests),
    },
];
const checkedInLeftRecursionTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, LEFT_RECURSION_TEST_PATH), "utf8"),
    LEFT_RECURSION_TEST_START,
    LEFT_RECURSION_TEST_END,
);
const recordedLeftRecursionTests = gitShowOptional(
    repoRoot,
    LEFT_RECURSION_TEST_COMMIT,
    LEFT_RECURSION_TEST_PATH,
);
const implementedLeftRecursionTests = gitShowOptional(
    repoRoot,
    LEFT_RECURSION_IMPLEMENTATION_COMMIT,
    LEFT_RECURSION_TEST_PATH,
);
if (
    recordedLeftRecursionTests === null ||
    sectionBetweenMarkers(
        recordedLeftRecursionTests,
        LEFT_RECURSION_TEST_START,
        LEFT_RECURSION_TEST_END,
    ) !== checkedInLeftRecursionTests
) {
    throw new Error(
        "checked-in left-recursion issue ports differ from their test commit",
    );
}
if (
    implementedLeftRecursionTests === null ||
    sectionBetweenMarkers(
        implementedLeftRecursionTests,
        LEFT_RECURSION_TEST_START,
        LEFT_RECURSION_TEST_END,
    ) !== checkedInLeftRecursionTests
) {
    throw new Error(
        "left-recursion implementation changed the locked test ports",
    );
}
const leftRecursionLockedSections = [
    {
        path: LEFT_RECURSION_TEST_PATH,
        marker: LEFT_RECURSION_TEST_START,
        end_marker: LEFT_RECURSION_TEST_END,
        sha256: digest(checkedInLeftRecursionTests),
    },
];
const checkedInLookaheadTreeTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, LOOKAHEAD_TREE_TEST_PATH), "utf8"),
    LOOKAHEAD_TREE_TEST_START,
    LOOKAHEAD_TREE_TEST_END,
);
const recordedLookaheadTreeTests = gitShowOptional(
    repoRoot,
    LOOKAHEAD_TREE_TEST_COMMIT,
    LOOKAHEAD_TREE_TEST_PATH,
);
const implementedLookaheadTreeTests = gitShowOptional(
    repoRoot,
    LOOKAHEAD_TREE_IMPLEMENTATION_COMMIT,
    LOOKAHEAD_TREE_TEST_PATH,
);
if (
    recordedLookaheadTreeTests === null ||
    sectionBetweenMarkers(
        recordedLookaheadTreeTests,
        LOOKAHEAD_TREE_TEST_START,
        LOOKAHEAD_TREE_HISTORICAL_TEST_END,
    ) !== checkedInLookaheadTreeTests
) {
    throw new Error(
        "checked-in lookahead tree ports differ from their test commit",
    );
}
if (
    implementedLookaheadTreeTests === null ||
    sectionBetweenMarkers(
        implementedLookaheadTreeTests,
        LOOKAHEAD_TREE_TEST_START,
        LOOKAHEAD_TREE_HISTORICAL_TEST_END,
    ) !== checkedInLookaheadTreeTests
) {
    throw new Error(
        "lookahead tree implementation changed the locked test ports",
    );
}
const lookaheadTreeLockedSections = [
    {
        path: LOOKAHEAD_TREE_TEST_PATH,
        marker: LOOKAHEAD_TREE_TEST_START,
        end_marker: LOOKAHEAD_TREE_TEST_END,
        sha256: digest(checkedInLookaheadTreeTests),
    },
];
const checkedInPhaseCTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, PHASE_C_TEST_PATH), "utf8"),
    PHASE_C_TEST_START,
    PHASE_C_TEST_END,
);
const checkedInPhaseCCases = sectionAtMarker(
    await readFile(resolve(repoRoot, PHASE_C_CASES_PATH), "utf8"),
    PHASE_C_CASES_MARKER,
);
for (const [label, commit] of [
    ["test", PHASE_C_TEST_COMMIT],
    ["implementation", PHASE_C_IMPLEMENTATION_COMMIT],
]) {
    const moduleSource = gitShowOptional(repoRoot, commit, PHASE_C_TEST_PATH);
    const casesSource = gitShowOptional(repoRoot, commit, PHASE_C_CASES_PATH);
    if (
        moduleSource === null ||
        sectionBetweenMarkers(
            moduleSource,
            PHASE_C_TEST_START,
            PHASE_C_TEST_END,
        ) !== checkedInPhaseCTests
    ) {
        throw new Error(
            `Phase C ${label} commit differs from the locked runtime test module`,
        );
    }
    if (
        casesSource === null ||
        sectionAtMarker(casesSource, PHASE_C_CASES_MARKER) !==
            checkedInPhaseCCases
    ) {
        throw new Error(
            `Phase C ${label} commit differs from the generated runtime cases`,
        );
    }
}
const phaseCLockedSections = [
    {
        path: PHASE_C_TEST_PATH,
        marker: PHASE_C_TEST_START,
        end_marker: PHASE_C_TEST_END,
        sha256: digest(checkedInPhaseCTests),
    },
    {
        path: PHASE_C_CASES_PATH,
        marker: PHASE_C_CASES_MARKER,
        sha256: digest(checkedInPhaseCCases),
    },
];
const checkedInGraphNodesTests = sectionAtMarker(
    await readFile(resolve(repoRoot, GRAPH_NODES_TEST_PATH), "utf8"),
    GRAPH_NODES_TEST_MARKER,
);
const recordedGraphNodesTests = gitShowOptional(
    repoRoot,
    GRAPH_NODES_TEST_COMMIT,
    GRAPH_NODES_TEST_PATH,
);
const implementedGraphNodesTests = gitShowOptional(
    repoRoot,
    GRAPH_NODES_IMPLEMENTATION_COMMIT,
    GRAPH_NODES_TEST_PATH,
);
if (
    recordedGraphNodesTests === null ||
    sectionAtMarker(recordedGraphNodesTests, GRAPH_NODES_TEST_MARKER) !==
        checkedInGraphNodesTests
) {
    throw new Error(
        "checked-in GraphNodes ports differ from their test commit",
    );
}
if (
    implementedGraphNodesTests === null ||
    sectionAtMarker(implementedGraphNodesTests, GRAPH_NODES_TEST_MARKER) !==
        checkedInGraphNodesTests
) {
    throw new Error(
        "GraphNodes implementation changed the locked test ports",
    );
}
const graphNodesLockedSections = [
    {
        path: GRAPH_NODES_TEST_PATH,
        marker: GRAPH_NODES_TEST_MARKER,
        sha256: digest(checkedInGraphNodesTests),
    },
];
const checkedInSymbolIssuesTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, SYMBOL_ISSUES_TEST_PATH), "utf8"),
    SYMBOL_ISSUES_TEST_START,
    SYMBOL_ISSUES_TEST_END,
);
const recordedSymbolIssuesTests = gitShowOptional(
    repoRoot,
    SYMBOL_ISSUES_TEST_COMMIT,
    SYMBOL_ISSUES_TEST_PATH,
);
const implementedSymbolIssuesTests = gitShowOptional(
    repoRoot,
    SYMBOL_ISSUES_IMPLEMENTATION_COMMIT,
    SYMBOL_ISSUES_TEST_PATH,
);
if (
    recordedSymbolIssuesTests === null ||
    sectionBetweenMarkers(
        recordedSymbolIssuesTests,
        SYMBOL_ISSUES_TEST_START,
        recordedSymbolIssuesTests.includes(SYMBOL_ISSUES_TEST_END)
            ? SYMBOL_ISSUES_TEST_END
            : SYMBOL_ISSUES_HISTORICAL_TEST_END,
    ) !== checkedInSymbolIssuesTests
) {
    throw new Error(
        "checked-in symbol issue ports differ from their test commit",
    );
}
if (
    implementedSymbolIssuesTests === null ||
    sectionBetweenMarkers(
        implementedSymbolIssuesTests,
        SYMBOL_ISSUES_TEST_START,
        implementedSymbolIssuesTests.includes(SYMBOL_ISSUES_TEST_END)
            ? SYMBOL_ISSUES_TEST_END
            : SYMBOL_ISSUES_HISTORICAL_TEST_END,
    ) !== checkedInSymbolIssuesTests
) {
    throw new Error(
        "symbol issue implementation changed the locked test ports",
    );
}
const symbolIssuesLockedSections = [
    {
        path: SYMBOL_ISSUES_TEST_PATH,
        marker: SYMBOL_ISSUES_TEST_START,
        end_marker: SYMBOL_ISSUES_TEST_END,
        historical_end_marker: SYMBOL_ISSUES_HISTORICAL_TEST_END,
        sha256: digest(checkedInSymbolIssuesTests),
    },
];
const checkedInAttributeChecksTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, ATTRIBUTE_CHECKS_TEST_PATH), "utf8"),
    ATTRIBUTE_CHECKS_TEST_START,
    ATTRIBUTE_CHECKS_TEST_END,
);
const checkedInAttributeChecksCases = sectionAtMarker(
    await readFile(resolve(repoRoot, ATTRIBUTE_CHECKS_CASES_PATH), "utf8"),
    ATTRIBUTE_CHECKS_CASES_MARKER,
);
for (const [label, commit] of [
    ["test", ATTRIBUTE_CHECKS_TEST_COMMIT],
    ["implementation", ATTRIBUTE_CHECKS_IMPLEMENTATION_COMMIT],
]) {
    const recordedTest = gitShowOptional(
        repoRoot,
        commit,
        ATTRIBUTE_CHECKS_TEST_PATH,
    );
    const recordedCases = gitShowOptional(
        repoRoot,
        commit,
        ATTRIBUTE_CHECKS_CASES_PATH,
    );
    if (
        recordedTest === null ||
        sectionBetweenMarkers(
            recordedTest,
            ATTRIBUTE_CHECKS_TEST_START,
            ATTRIBUTE_CHECKS_TEST_END,
        ) !== checkedInAttributeChecksTests
    ) {
        throw new Error(
            `checked-in attribute-check ${label} harness differs from the locked test commit`,
        );
    }
    if (
        recordedCases === null ||
        sectionAtMarker(
            recordedCases,
            ATTRIBUTE_CHECKS_CASES_MARKER,
        ) !== checkedInAttributeChecksCases
    ) {
        throw new Error(
            `checked-in attribute-check ${label} cases differ from the locked test commit`,
        );
    }
}
const attributeChecksLockedSections = [
    {
        path: ATTRIBUTE_CHECKS_TEST_PATH,
        marker: ATTRIBUTE_CHECKS_TEST_START,
        end_marker: ATTRIBUTE_CHECKS_TEST_END,
        sha256: digest(checkedInAttributeChecksTests),
    },
    {
        path: ATTRIBUTE_CHECKS_CASES_PATH,
        marker: ATTRIBUTE_CHECKS_CASES_MARKER,
        sha256: digest(checkedInAttributeChecksCases),
    },
];
const checkedInToolSyntaxErrorsTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, TOOL_SYNTAX_ERRORS_TEST_PATH), "utf8"),
    TOOL_SYNTAX_ERRORS_TEST_START,
    TOOL_SYNTAX_ERRORS_TEST_END,
);
const checkedInToolSyntaxErrorsCases = sectionAtMarker(
    await readFile(resolve(repoRoot, TOOL_SYNTAX_ERRORS_CASES_PATH), "utf8"),
    TOOL_SYNTAX_ERRORS_CASES_MARKER,
);
for (const [label, commit] of [
    ["test", TOOL_SYNTAX_ERRORS_TEST_COMMIT],
    ["implementation", TOOL_SYNTAX_ERRORS_IMPLEMENTATION_COMMIT],
]) {
    const recordedTest = gitShowOptional(
        repoRoot,
        commit,
        TOOL_SYNTAX_ERRORS_TEST_PATH,
    );
    const recordedCases = gitShowOptional(
        repoRoot,
        commit,
        TOOL_SYNTAX_ERRORS_CASES_PATH,
    );
    if (
        recordedTest === null ||
        sectionBetweenMarkers(
            recordedTest,
            TOOL_SYNTAX_ERRORS_TEST_START,
            TOOL_SYNTAX_ERRORS_TEST_END,
        ) !== checkedInToolSyntaxErrorsTests
    ) {
        throw new Error(
            `checked-in tool-syntax ${label} harness differs from the locked test commit`,
        );
    }
    if (
        recordedCases === null ||
        sectionAtMarker(
            recordedCases,
            TOOL_SYNTAX_ERRORS_CASES_MARKER,
        ) !== checkedInToolSyntaxErrorsCases
    ) {
        throw new Error(
            `checked-in tool-syntax ${label} cases differ from the locked test commit`,
        );
    }
}
const toolSyntaxErrorsLockedSections = [
    {
        path: TOOL_SYNTAX_ERRORS_TEST_PATH,
        marker: TOOL_SYNTAX_ERRORS_TEST_START,
        end_marker: TOOL_SYNTAX_ERRORS_TEST_END,
        sha256: digest(checkedInToolSyntaxErrorsTests),
    },
    {
        path: TOOL_SYNTAX_ERRORS_CASES_PATH,
        marker: TOOL_SYNTAX_ERRORS_CASES_MARKER,
        sha256: digest(checkedInToolSyntaxErrorsCases),
    },
];
const checkedInCompositeGrammarsTests = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, COMPOSITE_GRAMMARS_TEST_PATH), "utf8"),
    COMPOSITE_GRAMMARS_TEST_START,
    COMPOSITE_GRAMMARS_TEST_END,
);
const checkedInCompositeGrammarsCases = sectionAtMarker(
    await readFile(resolve(repoRoot, COMPOSITE_GRAMMARS_CASES_PATH), "utf8"),
    COMPOSITE_GRAMMARS_CASES_MARKER,
);
for (const [label, commit] of [
    ["test", COMPOSITE_GRAMMARS_TEST_COMMIT],
    ["implementation", COMPOSITE_GRAMMARS_IMPLEMENTATION_COMMIT],
]) {
    const recordedTest = gitShowOptional(
        repoRoot,
        commit,
        COMPOSITE_GRAMMARS_TEST_PATH,
    );
    const recordedCases = gitShowOptional(
        repoRoot,
        commit,
        COMPOSITE_GRAMMARS_CASES_PATH,
    );
    if (
        recordedTest === null ||
        sectionBetweenMarkers(
            recordedTest,
            COMPOSITE_GRAMMARS_TEST_START,
            COMPOSITE_GRAMMARS_TEST_END,
        ) !== checkedInCompositeGrammarsTests
    ) {
        throw new Error(
            `checked-in composite-grammar ${label} harness differs from the locked test commit`,
        );
    }
    if (
        recordedCases === null ||
        sectionAtMarker(
            recordedCases,
            COMPOSITE_GRAMMARS_CASES_MARKER,
        ) !== checkedInCompositeGrammarsCases
    ) {
        throw new Error(
            `checked-in composite-grammar ${label} cases differ from the locked test commit`,
        );
    }
}
const compositeGrammarsLockedSections = [
    {
        path: COMPOSITE_GRAMMARS_TEST_PATH,
        marker: COMPOSITE_GRAMMARS_TEST_START,
        end_marker: COMPOSITE_GRAMMARS_TEST_END,
        sha256: digest(checkedInCompositeGrammarsTests),
    },
    {
        path: COMPOSITE_GRAMMARS_CASES_PATH,
        marker: COMPOSITE_GRAMMARS_CASES_MARKER,
        sha256: digest(checkedInCompositeGrammarsCases),
    },
];
const checkedInGeneralAtnDotTests = sectionAtMarker(
    await readFile(resolve(repoRoot, GENERAL_ATN_DOT_TEST_PATH), "utf8"),
    GENERAL_ATN_DOT_TEST_MARKER,
);
const checkedInGeneralAtnDotModule = sectionBetweenMarkers(
    await readFile(resolve(repoRoot, GENERAL_ATN_DOT_MODULE_PATH), "utf8"),
    GENERAL_ATN_DOT_MODULE_MARKER,
    GENERAL_ATN_DOT_MODULE_END,
);
const recordedGeneralAtnDotTests = gitShowOptional(
    repoRoot,
    GENERAL_ATN_DOT_TEST_COMMIT,
    GENERAL_ATN_DOT_TEST_PATH,
);
const recordedGeneralAtnDotModule = gitShowOptional(
    repoRoot,
    GENERAL_ATN_DOT_TEST_COMMIT,
    GENERAL_ATN_DOT_MODULE_PATH,
);
if (
    recordedGeneralAtnDotTests === null ||
    sectionAtMarker(
        recordedGeneralAtnDotTests,
        GENERAL_ATN_DOT_TEST_MARKER,
    ) !== checkedInGeneralAtnDotTests
) {
    throw new Error(
        "checked-in General ATN DOT tests differ from their test commit",
    );
}
if (
    recordedGeneralAtnDotModule === null ||
    sectionBetweenMarkers(
        recordedGeneralAtnDotModule,
        GENERAL_ATN_DOT_MODULE_MARKER,
        GENERAL_ATN_DOT_MODULE_END,
    ) !== checkedInGeneralAtnDotModule
) {
    throw new Error(
        "checked-in General ATN DOT module declaration differs from its test commit",
    );
}
const generalAtnDotLockedSections = [
    {
        path: GENERAL_ATN_DOT_TEST_PATH,
        marker: GENERAL_ATN_DOT_TEST_MARKER,
        sha256: digest(checkedInGeneralAtnDotTests),
    },
    {
        path: GENERAL_ATN_DOT_MODULE_PATH,
        marker: GENERAL_ATN_DOT_MODULE_MARKER,
        end_marker: GENERAL_ATN_DOT_MODULE_END,
        sha256: digest(checkedInGeneralAtnDotModule),
    },
];

const upstreamByLogicalId = new Map(
    testMap.rows.map((row) => [row.logical_id, row]),
);
for (const fixture of externalMap.fixtures) {
    for (const assertion of fixture.assertions) {
        if (assertion.tdd_owner.startsWith("upstream:")) {
            const logicalId = assertion.tdd_owner.slice("upstream:".length);
            const row = upstreamByLogicalId.get(logicalId);
            if (!row) {
                throw new Error(`${assertion.id} names missing upstream row ${logicalId}`);
            }
            assertion.upstream_active_revision_id = row.active_revision_id;
            assertion.transitive_closure_sha256 = row.closure_sha256;
        } else if (assertion.tdd_owner.startsWith("external:")) {
            const definition = EXTERNAL_DEFINITIONS[assertion.id];
            if (!definition) {
                throw new Error(`missing evidence definition for ${assertion.id}`);
            }
            const source = externalSources.get(fixture.source_id);
            const closure = {
                assertion_id: assertion.id,
                source_id: source.source_id,
                source_sha256: source.sha256,
                owner_phase: assertion.phase,
                observable: assertion.observable,
                rust_test: assertion.rust_test,
                canonical_input: definition.canonical_input,
                expected_observable: definition.expected_observable,
                primary_test_source: definition.source_test,
                alternate_test_source: {
                    repository: "https://github.com/mike-lischke/antlr-ng.git",
                    commit: ANTLR_NG_COMMIT,
                    oracle: "independent grammar frontend token/tree/diagnostic observation",
                },
                scaffold_commit: SCAFFOLD_COMMIT,
                primary_test_commit: TEST_COMMIT,
            };
            const closureHash = digest(stableStringify(closure));
            assertion.tdd = {
                active_revision_id: assertion.active_revision_id,
                state: "done",
                prerequisites: ["behavior-free grammar frontend scaffold"],
                unit_under_test: "Stage 0 source spans and fail-closed boundary",
                failure_fingerprint: "G4F000 Stage 0 frontend is not installed",
                primary_test_source: definition.source_test,
                alternate_test_source: closure.alternate_test_source,
                primary_implementation_source: `antlr-ng@${ANTLR_NG_COMMIT}`,
                alternate_implementation_source: `java-antlr@${JAVA_COMMIT}`,
                scaffold_commit: SCAFFOLD_COMMIT,
                primary_test_commit: TEST_COMMIT,
                demonstrated_red: redResult(),
                primary_implementation_commit: IMPLEMENTATION_COMMIT,
                green_result: greenResult(),
                closure,
                closure_sha256: closureHash,
                evidence_path: `tests/codegen-direct/port-evidence/${assertion.id}`,
            };
            await addEvidence({
                logicalId: assertion.id,
                revisionId: assertion.active_revision_id,
                closure,
                closureHash,
                sourceCaseIds: [],
                externalSource: source,
                primaryTestSource: definition.source_test,
                alternateTestSource: closure.alternate_test_source,
                declaredOutcomes: {
                    primary: definition.expected_observable,
                    alternate: definition.alternate_outcome,
                    java_compatibility_verdict: definition.java_verdict,
                },
                resolution: "ported",
                testCommit: TEST_COMMIT,
                implementationCommit: IMPLEMENTATION_COMMIT,
                testCommand: TEST_COMMAND,
                greenResultText: "5 passed; 0 failed",
                lockedSections: defaultLockedSections,
                ownerPhase: assertion.phase,
                scaffoldCommit: SCAFFOLD_COMMIT,
                testParent: SCAFFOLD_COMMIT,
                implementationParent: TEST_COMMIT,
                reachability:
                    "direct ancestry is verified when the recorded commit objects are available",
            });
        }
    }
}

for (const row of completedRows) {
    const coveredExisting = row.resolution === "verified-covered-existing";
    if (row.owner_phase === "C") {
        await addEvidence({
            logicalId: row.logical_id,
            revisionId: row.active_revision_id,
            closure: row.closure,
            closureHash: row.closure_sha256,
            sourceCaseIds: row.source_case_ids,
            primaryTestSource: row.primary_test_source,
            alternateTestSource: row.alternate_test_source,
            declaredOutcomes: {
                primary:
                    "the case-specific Rust fixture and runtime assertion match the pinned Java ANTLR 4.13.2 observable",
                alternate:
                    "the pinned antlr-ng case supplies the same grammar and expected runtime behavior where available",
                java_compatibility_verdict:
                    "exact Java ANTLR 4.13.2 token, prediction, tree, action, diagnostic, or generated-code behavior",
            },
            resolution: row.resolution ?? "ported",
            testCommit: row.primary_test_commit,
            implementationCommit: row.primary_implementation_commit,
            testCommand: row.green_result.command,
            greenResultText: row.green_result.result,
            lockedSections: phaseCLockedSections,
            ownerPhase: row.owner_phase,
            scaffoldCommit: PHASE_C_BASE_COMMIT,
            testParent: PHASE_C_BASE_COMMIT,
            implementationParent: coveredExisting
                ? PHASE_C_BASE_PARENT_COMMIT
                : PHASE_C_TEST_COMMIT,
            reachability: coveredExisting
                ? "the case-specific test passed against the existing Phase C implementation at its parent"
                : "the implementation commit is directly based on the locked red Phase C runtime tests",
            demonstratedRed: coveredExisting
                ? undefined
                : row.demonstrated_red,
        });
        continue;
    }
    const phaseBAtnSerialization = row.logical_id.startsWith(
        "testatnserialization-",
    );
    const phaseBAtnConstruction = row.logical_id.startsWith(
        "testatnconstruction-",
    );
    const phaseBBasicSemantic = row.logical_id.startsWith(
        "testbasicsemanticerrors-",
    );
    const phaseBErrorSets = row.logical_id.startsWith(
        "testerrorsets-",
    );
    const phaseBTokenPosition = row.logical_id.startsWith(
        "testtokenpositionoptions-",
    );
    const phaseBTopologicalSort = row.logical_id.startsWith(
        "testtopologicalsort-",
    );
    const phaseBVocabulary = row.logical_id.startsWith(
        "testvocabulary-",
    );
    const phaseBScopeParsing = row.logical_id.startsWith(
        "testscopeparsing-",
    );
    const phaseBCharSupport = row.logical_id.startsWith(
        "testcharsupport-",
    );
    const phaseBNestedAction =
        row.logical_id === NESTED_ACTION_LOGICAL_ID;
    const phaseBEscapeSequence = row.logical_id.startsWith(
        "testescapesequenceparsing-",
    );
    const phaseBUnicodeEscape = row.logical_id.startsWith(
        "testunicodeescapes-",
    );
    const phaseBUnicodeData = row.logical_id.startsWith(
        "testunicodedata-",
    );
    const phaseBUnicodeGrammar = row.logical_id.startsWith(
        "testunicodegrammar-",
    );
    const phaseBTokenAssignment = row.logical_id.startsWith(
        "testtokentypeassignment-",
    );
    const phaseBLeftRecursion = row.logical_id.startsWith(
        "testleftrecursiontoolissues-",
    );
    const phaseBLookaheadTree = row.logical_id.startsWith(
        "testlookaheadtrees-",
    );
    const phaseBGraphNodes = row.logical_id.startsWith(
        "testgraphnodes-",
    );
    const phaseBSymbolIssues = row.logical_id.startsWith(
        "testsymbolissues-",
    );
    const phaseBAttributeChecks = row.logical_id.startsWith(
        "testattributechecks-",
    );
    const phaseBToolSyntaxErrors = row.logical_id.startsWith(
        "testtoolsyntaxerrors-",
    );
    const phaseBCompositeGrammars = row.logical_id.startsWith(
        "testcompositegrammars-",
    );
    const phaseBGeneralAtnDot =
        GENERAL_ATN_DOT_LOGICAL_IDS.has(row.logical_id);
    if (
        row.owner_phase === "B" &&
        !phaseBAtnSerialization &&
        !phaseBAtnConstruction &&
        !phaseBBasicSemantic &&
        !phaseBErrorSets &&
        !phaseBTokenPosition &&
        !phaseBTopologicalSort &&
        !phaseBVocabulary &&
        !phaseBScopeParsing &&
        !phaseBCharSupport &&
        !phaseBNestedAction &&
        !phaseBEscapeSequence &&
        !phaseBUnicodeEscape &&
        !phaseBUnicodeData &&
        !phaseBUnicodeGrammar &&
        !phaseBTokenAssignment &&
        !phaseBLeftRecursion &&
        !phaseBLookaheadTree &&
        !phaseBGraphNodes &&
        !phaseBSymbolIssues &&
        !phaseBAttributeChecks &&
        !phaseBToolSyntaxErrors &&
        !phaseBCompositeGrammars &&
        !phaseBGeneralAtnDot
    ) {
        throw new Error(`missing Phase B evidence profile for ${row.logical_id}`);
    }
    const phaseBProfile = phaseBAtnSerialization
        ? {
              lockedSections: atnSerializationLockedSections,
              scaffoldCommit: PHASE_B_BASE_COMMIT,
              testParent: PHASE_B_IMPLEMENTATION_COMMIT,
              implementationParent: PHASE_B_BASE_COMMIT,
              reachability:
                  "the case-specific test commit is directly based on the existing Phase B implementation",
          }
        : phaseBAtnConstruction
          ? {
                lockedSections: atnConstructionLockedSections,
                scaffoldCommit: ATN_CONSTRUCTION_BASE_COMMIT,
                testParent: ATN_CONSTRUCTION_BASE_COMMIT,
                implementationParent: coveredExisting
                    ? PHASE_B_BASE_COMMIT
                    : ATN_CONSTRUCTION_TEST_COMMIT,
                reachability: coveredExisting
                    ? "the case-specific test passed against the Phase B implementation already reachable from its parent"
                    : "the implementation commit is directly based on the locked red construction tests",
            }
          : phaseBBasicSemantic
            ? {
                  lockedSections: basicSemanticLockedSections,
                  scaffoldCommit: BASIC_SEMANTIC_BASE_COMMIT,
                  testParent: BASIC_SEMANTIC_BASE_COMMIT,
                  implementationParent: BASIC_SEMANTIC_TEST_COMMIT,
                  reachability:
                      "the implementation commit is directly based on the locked red semantic tests",
              }
            : phaseBErrorSets
              ? {
                    lockedSections: errorSetsLockedSections,
                    scaffoldCommit: ERROR_SETS_BASE_COMMIT,
                    testParent: ERROR_SETS_BASE_COMMIT,
                    implementationParent: ERROR_SETS_TEST_COMMIT,
                    reachability:
                        "the implementation commit is directly based on the locked red lexer-set tests",
                }
              : phaseBTokenPosition
                ? {
                      lockedSections: tokenPositionLockedSections,
                      scaffoldCommit: TOKEN_POSITION_BASE_COMMIT,
                      testParent: TOKEN_POSITION_BASE_COMMIT,
                      implementationParent: coveredExisting
                          ? ERROR_SETS_IMPLEMENTATION_COMMIT
                          : TOKEN_POSITION_TEST_COMMIT,
                      reachability: coveredExisting
                          ? "the case-specific test passed against the Phase B implementation reachable from its parent"
                          : "the implementation commit is directly based on the locked red token-position tests",
                  }
                : phaseBTopologicalSort
                  ? {
                        lockedSections: topologicalSortLockedSections,
                        scaffoldCommit: TOPOLOGICAL_SORT_BASE_COMMIT,
                        testParent: TOPOLOGICAL_SORT_BASE_COMMIT,
                        implementationParent:
                            TOKEN_POSITION_IMPLEMENTATION_COMMIT,
                        reachability:
                            "the case-specific test passed against the Phase B loader implementation reachable from its parent",
                    }
                  : phaseBVocabulary
                    ? row.logical_id === EMPTY_VOCABULARY_LOGICAL_ID
                        ? {
                              lockedSections:
                                  emptyVocabularyLockedSections,
                              scaffoldCommit:
                                  EMPTY_VOCABULARY_BASE_COMMIT,
                              testParent:
                                  EMPTY_VOCABULARY_BASE_COMMIT,
                              implementationParent:
                                  EMPTY_VOCABULARY_TEST_COMMIT,
                              reachability:
                                  "the empty vocabulary implementation commit is directly based on its locked red test",
                          }
                        : {
                              lockedSections:
                                  tokenNamesVocabularyLockedSections,
                              scaffoldCommit: VOCABULARY_BASE_COMMIT,
                              testParent: VOCABULARY_BASE_COMMIT,
                              implementationParent:
                                  VOCABULARY_TEST_COMMIT,
                              reachability:
                                  "the token-names vocabulary implementation commit is directly based on its locked red test",
                          }
                    : phaseBScopeParsing
                      ? {
                            lockedSections: scopeParsingLockedSections,
                            scaffoldCommit: SCOPE_PARSING_BASE_COMMIT,
                            testParent: SCOPE_PARSING_BASE_COMMIT,
                            implementationParent:
                                SCOPE_PARSING_TEST_COMMIT,
                            reachability:
                                "the scope parsing implementation commit is directly based on its locked red test",
                        }
                      : phaseBCharSupport
                        ? {
                              lockedSections: charSupportLockedSections,
                              scaffoldCommit: CHAR_SUPPORT_BASE_COMMIT,
                              testParent: CHAR_SUPPORT_BASE_COMMIT,
                              implementationParent:
                                  CHAR_SUPPORT_TEST_COMMIT,
                              reachability:
                                  "the character support implementation commit is directly based on its locked red tests",
                          }
                        : phaseBNestedAction
                          ? {
                                lockedSections:
                                    nestedActionLockedSections,
                                scaffoldCommit:
                                    NESTED_ACTION_BASE_COMMIT,
                                testParent:
                                    NESTED_ACTION_BASE_COMMIT,
                                implementationParent:
                                    NESTED_ACTION_TEST_COMMIT,
                                reachability:
                                    "the nested action implementation commit is directly based on its locked red test",
                            }
                          : phaseBEscapeSequence
                            ? {
                                  lockedSections:
                                      escapeSequenceLockedSections,
                                  scaffoldCommit:
                                      ESCAPE_SEQUENCE_SCAFFOLD_COMMIT,
                                  testParent:
                                      ESCAPE_SEQUENCE_SCAFFOLD_COMMIT,
                                  implementationParent: coveredExisting
                                      ? ESCAPE_SEQUENCE_SCAFFOLD_PARENT_COMMIT
                                      : ESCAPE_SEQUENCE_TEST_COMMIT,
                                  reachability: coveredExisting
                                      ? "the case-specific invalid-input test passed against the behavior-free escape parser scaffold"
                                      : "the escape sequence implementation commit is directly based on its locked red tests",
                              }
                            : phaseBUnicodeEscape
                              ? {
                                    lockedSections:
                                        unicodeEscapeLockedSections,
                                    scaffoldCommit:
                                        UNICODE_ESCAPE_SCAFFOLD_COMMIT,
                                    testParent:
                                        UNICODE_ESCAPE_SCAFFOLD_COMMIT,
                                    implementationParent:
                                        UNICODE_ESCAPE_TEST_COMMIT,
                                    reachability:
                                        "the Unicode escape implementation commit is directly based on its locked red tests",
                                }
                              : phaseBUnicodeData
                                ? {
                                      lockedSections:
                                          unicodeDataLockedSections,
                                      scaffoldCommit:
                                          UNICODE_DATA_BASE_COMMIT,
                                      testParent:
                                          UNICODE_DATA_BASE_COMMIT,
                                      implementationParent:
                                          UNICODE_DATA_BASE_PARENT_COMMIT,
                                      reachability:
                                          "the case-specific tests passed against the existing generated Unicode data implementation",
                                  }
                                : phaseBUnicodeGrammar
                                  ? {
                                        lockedSections:
                                            unicodeGrammarLockedSections,
                                        scaffoldCommit:
                                            UNICODE_GRAMMAR_BASE_COMMIT,
                                        testParent:
                                            UNICODE_GRAMMAR_BASE_COMMIT,
                                        implementationParent:
                                            coveredExisting
                                                ? UNICODE_GRAMMAR_BASE_PARENT_COMMIT
                                                : UNICODE_GRAMMAR_TEST_COMMIT,
                                        reachability: coveredExisting
                                            ? "the case-specific .interp test passed against the direct compiler implementation in its parent"
                                            : "the surrogate escape implementation commit is directly based on the locked red .interp tests",
                                    }
                                  : phaseBTokenAssignment
                                    ? {
                                          lockedSections:
                                              tokenAssignmentLockedSections,
                                          scaffoldCommit:
                                              TOKEN_ASSIGNMENT_BASE_COMMIT,
                                          testParent:
                                              TOKEN_ASSIGNMENT_FIXTURE_COMMIT,
                                          implementationParent:
                                              coveredExisting
                                                  ? TOKEN_ASSIGNMENT_BASE_PARENT_COMMIT
                                                  : TOKEN_ASSIGNMENT_TEST_COMMIT,
                                          reachability: coveredExisting
                                              ? "the case-specific .interp and .tokens test passed against the direct compiler implementation already present at the batch base"
                                              : "the insertion-order implementation commit is directly based on the locked red token-assignment representation test",
                                      }
                                    : phaseBLeftRecursion
                                      ? {
                                            lockedSections:
                                                leftRecursionLockedSections,
                                            scaffoldCommit:
                                                LEFT_RECURSION_BASE_COMMIT,
                                            testParent:
                                                LEFT_RECURSION_FIXTURE_COMMIT,
                                            implementationParent:
                                                coveredExisting
                                                    ? LEFT_RECURSION_BASE_PARENT_COMMIT
                                                    : LEFT_RECURSION_TEST_COMMIT,
                                            reachability: coveredExisting
                                                ? "the accepted primary-rule-argument .interp test passed against the direct compiler implementation already present at the batch base"
                                                : "the left-recursion implementation commit is directly based on the locked red diagnostic tests",
                                        }
                                      : phaseBGraphNodes
                                        ? {
                                              lockedSections:
                                                  graphNodesLockedSections,
                                              scaffoldCommit:
                                                  GRAPH_NODES_BASE_COMMIT,
                                              testParent:
                                                  GRAPH_NODES_BASE_COMMIT,
                                              implementationParent:
                                                  coveredExisting
                                                      ? GRAPH_NODES_BASE_PARENT_COMMIT
                                                      : GRAPH_NODES_TEST_COMMIT,
                                              reachability: coveredExisting
                                                  ? "the case-specific DOT assertion passed against prediction-context merging already present at the batch base"
                                                  : "the recursive parent-merge implementation commit is directly based on the locked red DOT assertions",
                                          }
                                        : phaseBSymbolIssues
                                          ? {
                                                lockedSections:
                                                    symbolIssuesLockedSections,
                                                scaffoldCommit:
                                                    SYMBOL_ISSUES_BASE_COMMIT,
                                                testParent:
                                                    SYMBOL_ISSUES_BASE_COMMIT,
                                                implementationParent:
                                                    coveredExisting
                                                        ? SYMBOL_ISSUES_BASE_PARENT_COMMIT
                                                        : SYMBOL_ISSUES_TEST_COMMIT,
                                                reachability: coveredExisting
                                                    ? "the case-specific symbol assertion passed against the direct compiler implementation already present at the batch base"
                                                    : "the symbol semantics implementation commit is directly based on the locked red parity tests",
                                            }
                                        : phaseBAttributeChecks
                                          ? {
                                                lockedSections:
                                                    attributeChecksLockedSections,
                                                scaffoldCommit:
                                                    ATTRIBUTE_CHECKS_BASE_COMMIT,
                                                testParent:
                                                    ATTRIBUTE_CHECKS_BASE_COMMIT,
                                                implementationParent:
                                                    coveredExisting
                                                        ? ATTRIBUTE_CHECKS_BASE_PARENT_COMMIT
                                                        : ATTRIBUTE_CHECKS_TEST_COMMIT,
                                                reachability: coveredExisting
                                                    ? "the case-specific accepted-action assertions passed against the direct compiler implementation already present at the batch base"
                                                    : "the action attribute implementation commit is directly based on the locked red parity tests",
                                            }
                                          : phaseBToolSyntaxErrors
                                            ? {
                                                  lockedSections:
                                                      toolSyntaxErrorsLockedSections,
                                                  scaffoldCommit:
                                                      TOOL_SYNTAX_ERRORS_BASE_COMMIT,
                                                  testParent:
                                                      TOOL_SYNTAX_ERRORS_BASE_COMMIT,
                                                  implementationParent:
                                                      coveredExisting
                                                          ? TOOL_SYNTAX_ERRORS_BASE_PARENT_COMMIT
                                                          : TOOL_SYNTAX_ERRORS_TEST_COMMIT,
                                                  reachability: coveredExisting
                                                      ? "the case-specific tool diagnostic or artifact assertion passed against the direct compiler implementation already present at the batch base"
                                                      : "the tool diagnostic implementation commit is directly based on the locked red parity tests",
                                              }
                                            : phaseBCompositeGrammars
                                              ? {
                                                    lockedSections:
                                                        compositeGrammarsLockedSections,
                                                    scaffoldCommit:
                                                        COMPOSITE_GRAMMARS_BASE_COMMIT,
                                                    testParent:
                                                        COMPOSITE_GRAMMARS_BASE_COMMIT,
                                                    implementationParent:
                                                        coveredExisting
                                                            ? COMPOSITE_GRAMMARS_BASE_PARENT_COMMIT
                                                            : COMPOSITE_GRAMMARS_IMPLEMENTATION_PARENT_COMMIT,
                                                    reachability: coveredExisting
                                                        ? "the case-specific composite-grammar assertion passed against the direct compiler implementation already present at the batch base"
                                                        : "the final implementation commit descends from the locked red composite-grammar tests without changing them",
                                                }
                                            : phaseBGeneralAtnDot
                                              ? {
                                                    lockedSections:
                                                        generalAtnDotLockedSections,
                                                    scaffoldCommit:
                                                        GENERAL_ATN_DOT_BASE_COMMIT,
                                                    testParent:
                                                        GENERAL_ATN_DOT_BASE_COMMIT,
                                                    implementationParent:
                                                        GENERAL_ATN_DOT_BASE_PARENT_COMMIT,
                                                    reachability:
                                                        "the case-specific .interp, .tokens, and DOT assertions passed against the direct compiler implementation already present at the batch base",
                                                }
                                          : phaseBLookaheadTree
                                            ? {
                                                  lockedSections:
                                                      lookaheadTreeLockedSections,
                                                  scaffoldCommit:
                                                      LOOKAHEAD_TREE_FIXTURE_COMMIT,
                                                  testParent:
                                                      LOOKAHEAD_TREE_FIXTURE_COMMIT,
                                                  implementationParent:
                                                      LOOKAHEAD_TREE_TEST_COMMIT,
                                                  reachability:
                                                      "the lookahead tree implementation commit is directly based on the locked red forced-alternative tests",
                                              }
          : null;
    await addEvidence({
        logicalId: row.logical_id,
        revisionId: row.active_revision_id,
        closure: row.closure,
        closureHash: row.closure_sha256,
        sourceCaseIds: row.source_case_ids,
        externalSource: null,
        primaryTestSource: row.primary_test_source,
        alternateTestSource: row.alternate_test_source,
        declaredOutcomes: phaseBAtnSerialization
            ? {
                  primary:
                      "the complete direct Rust .interp matches the immutable Java 4.13.2 fixture",
                  alternate:
                      "the pinned antlr-ng TestATNSerialization case exposes the same ATN observable",
                  java_compatibility_verdict:
                      "exact Java 4.13.2 recognizer metadata and serialized ATN equality",
              }
            : phaseBAtnConstruction
              ? {
                    primary:
                        "the direct Rust ATN graph, complete .interp, or semantic diagnostic matches the immutable Java 4.13.2 oracle",
                    alternate:
                        "the pinned antlr-ng TestATNConstruction case and retained divergence artifacts expose the alternate outcome",
                    java_compatibility_verdict:
                        "Java 4.13.2 supplies the compatibility verdict for graph, serialization, Unicode, and diagnostic differences",
                }
              : phaseBBasicSemantic
                ? {
                      primary:
                          "the direct Rust compiler emits the same ordered semantic diagnostics and source positions as Java 4.13.2",
                      alternate:
                          "the pinned antlr-ng TestBasicSemanticErrors case exposes the same semantic category and position contract",
                      java_compatibility_verdict:
                          "exact Java 4.13.2 diagnostic severity, ordering, position, and message equality",
                  }
                : phaseBErrorSets
                  ? {
                        primary:
                            "the direct Rust compiler emits the same lexer-set diagnostic category and source position as Java 4.13.2",
                        alternate:
                            "the pinned antlr-ng TestErrorSets case exposes the same lexer-set category and position contract",
                        java_compatibility_verdict:
                            "exact Java 4.13.2 diagnostic severity, position, and message equality",
                    }
                  : phaseBTokenPosition
                    ? {
                          primary:
                              "the direct Rust compiler preserves Java 4.13.2 authored token positions through left-recursion rewriting and matches both complete .interp files",
                          alternate:
                              "the pinned antlr-ng TestTokenPositionOptions case exposes the same rewritten structure and token positions",
                          java_compatibility_verdict:
                              "exact Java 4.13.2 parser and lexer .interp equality plus authored token-position equality",
                      }
                    : phaseBTopologicalSort
                      ? {
                            primary:
                                "the direct Rust loader preserves Java 4.13.2 dependency-first ordering, duplicate-edge handling, and cycle traversal",
                            alternate:
                                "the pinned antlr-ng TestTopologicalSort case exposes the same dependency ordering",
                            java_compatibility_verdict:
                                "exact Java 4.13.2 topological order with source-backed vocabulary edges",
                        }
                      : phaseBVocabulary
                        ? {
                              primary:
                                  "the Rust vocabulary API matches Java 4.13.2 empty, display, literal, symbolic, and EOF name behavior",
                              alternate:
                                  "the pinned antlr-ng TestVocabulary case exposes the same vocabulary behavior",
                              java_compatibility_verdict:
                                  "exact Java 4.13.2 vocabulary name classification",
                          }
                        : phaseBScopeParsing
                          ? {
                                primary:
                                    "the direct Rust declaration parser matches Java 4.13.2 names, authored types, and initializers",
                                alternate:
                                    "the paired pinned source cases expose the same prefix and postfix declaration behavior",
                                java_compatibility_verdict:
                                    "exact Java 4.13.2 scope declaration parsing",
                            }
                          : phaseBCharSupport
                            ? {
                                  primary:
                                      "the shared Rust character support matches Java 4.13.2 literal parsing, escaping, ranges, and capitalization",
                                  alternate:
                                      "the paired pinned antlr-ng TestCharSupport cases expose the same utility behavior",
                                  java_compatibility_verdict:
                                      "exact Java 4.13.2 TestCharSupport outputs",
                              }
                            : phaseBNestedAction
                              ? {
                                    primary:
                                        "the direct Rust grammar model preserves the nested members body and decodes the generated Java predicate fail-message oracle",
                                    alternate:
                                        "the pinned antlr-ng Nested actions case exposes the same nested action and predicate option observable",
                                    java_compatibility_verdict:
                                        "Java 4.13.2 supplies the predicate fail-message compatibility verdict",
                                }
                              : phaseBEscapeSequence
                                ? {
                                      primary:
                                          "the direct Rust escape parser matches Java 4.13.2 result kind, code point or property set, and consumed span",
                                      alternate:
                                          "the pinned antlr-ng TestEscapeSequenceParsing case exposes the same result contract",
                                      java_compatibility_verdict:
                                          "exact Java 4.13.2 TestEscapeSequenceParsing result equality",
                                  }
                                : phaseBUnicodeEscape
                                  ? {
                                        primary:
                                            "the direct Rust Unicode formatter matches Java 4.13.2 UTF-16, fixed-width scalar, and braced scalar escapes",
                                        alternate:
                                            "the pinned antlr-ng TestUnicodeEscapes case exposes the same rendered escape",
                                        java_compatibility_verdict:
                                            "exact Java 4.13.2 TestUnicodeEscapes output equality",
                                    }
                                  : phaseBUnicodeData
                                    ? {
                                          primary:
                                              "the direct Rust Unicode property tables match Java 4.13.2 categories, aliases, scripts, blocks, and emoji properties while exposing read-only static slices",
                                          alternate:
                                              "paired pinned antlr-ng cases where available, plus the generated property-table oracle, expose the same membership behavior",
                                          java_compatibility_verdict:
                                              "exact Java 4.13.2 property membership; Rust's read-only slice statically enforces Java's mutation rejection contract",
                                      }
                                    : phaseBUnicodeGrammar
                                      ? {
                                            primary:
                                                "both complete direct Rust .interp files match the immutable Java 4.13.2 lexer and parser fixtures",
                                            alternate:
                                                "the pinned antlr-ng TestUnicodeGrammar case exposes the same compiler-level Unicode literal behavior where available",
                                            java_compatibility_verdict:
                                                "exact Java 4.13.2 recognizer metadata and serialized lexer and parser ATN equality",
                                        }
                                      : phaseBTokenAssignment
                                        ? {
                                              primary:
                                                  "the complete direct Rust .interp and .tokens text match the immutable Java 4.13.2 fixtures",
                                              alternate:
                                                  "the pinned antlr-ng TestTokenTypeAssignment case uses the same grammar and token/rule expectations",
                                              java_compatibility_verdict:
                                                  "exact Java 4.13.2 token numbering, aliases, insertion order, recognizer metadata, and serialized ATN equality",
                                          }
                                        : phaseBLeftRecursion
                                          ? {
                                                primary:
                                                    "the direct Rust compiler matches the immutable Java 4.13.2 left-recursion diagnostic or complete accepted parser and lexer .interp",
                                                alternate:
                                                    "the pinned antlr-ng TestLeftRecursionToolIssues case uses the same grammar and expected outcome",
                                                java_compatibility_verdict:
                                                    "exact Java 4.13.2 diagnostic severity, position, and message equality or complete accepted recognizer metadata and serialized ATN equality",
                                            }
                                          : phaseBGraphNodes
                                            ? {
                                                  primary:
                                                      "the Rust prediction-context merge produces the exact deterministic DOT graph expected by Java 4.13.2",
                                                  alternate:
                                                      "the pinned antlr-ng TestGraphNodes case exercises the same singleton and array prediction-context merge behavior",
                                                  java_compatibility_verdict:
                                                      "exact Java 4.13.2 TestGraphNodes DOT graph equality",
                                              }
                                            : phaseBSymbolIssues
                                              ? {
                                                    primary:
                                                        "the direct Rust compiler matches the immutable Java 4.13.2 diagnostics and complete .interp and .tokens fixtures",
                                                    alternate:
                                                        "the pinned antlr-ng TestSymbolIssues case exercises the same symbol, command, and lexer-range behavior",
                                                    java_compatibility_verdict:
                                                        "exact Java 4.13.2 diagnostic ordering and positions, token metadata, and serialized ATN equality",
                                                }
                                            : phaseBAttributeChecks
                                              ? {
                                                    primary:
                                                        "the direct Rust compiler matches the immutable Java 4.13.2 action-attribute diagnostics and accepted .interp and .tokens fixtures",
                                                    alternate:
                                                        "the pinned antlr-ng TestAttributeChecks case uses the same rendered grammar and diagnostic expectation",
                                                    java_compatibility_verdict:
                                                        "exact Java 4.13.2 diagnostic code, message, ordering, and position or complete accepted recognizer metadata equality",
                                                }
                                              : phaseBToolSyntaxErrors
                                                ? {
                                                      primary:
                                                          "the direct Rust compiler matches the immutable Java 4.13.2 tool diagnostics and accepted .interp and .tokens fixtures",
                                                      alternate:
                                                          "the pinned antlr-ng TestToolSyntaxErrors case uses the same grammar and tool-diagnostic expectation",
                                                      java_compatibility_verdict:
                                                          "exact Java 4.13.2 diagnostic category, severity, ordering, and position or complete accepted recognizer metadata equality",
                                                  }
                                                : phaseBCompositeGrammars
                                                  ? {
                                                        primary:
                                                            "the direct Rust compiler matches the immutable Java 4.13.2 composite-grammar diagnostics and complete .interp and .tokens fixtures",
                                                        alternate:
                                                            "the pinned antlr-ng TestCompositeGrammars case uses the same grammar import graph and observable",
                                                        java_compatibility_verdict:
                                                            "exact Java 4.13.2 import integration, diagnostic category, severity, ordering, source position, and accepted recognizer metadata equality",
                                                    }
                                                : phaseBGeneralAtnDot
                                                  ? {
                                                        primary:
                                                            "the complete direct Rust .interp and .tokens match the immutable Java 4.13.2 fixtures and the selected ATN DOT edge matches the pinned upstream oracle",
                                                        alternate:
                                                            "the pinned antlr-ng General case supplies the ATN DOT edge and no-crash observable",
                                                        java_compatibility_verdict:
                                                            "exact Java 4.13.2 recognizer metadata and serialized ATN equality, with pinned antlr-ng supplying the EOF DOT oracle where Java 4.13.2 crashes",
                                                    }
                                              : phaseBLookaheadTree
                                                ? {
                                                      primary:
                                                          "the complete direct Rust lexer and parser .interp plus every forced-alternative parse tree match the immutable Java 4.13.2 oracles",
                                                      alternate:
                                                          "the pinned antlr-ng TestLookaheadTrees case uses the same grammar, decision, input index, and expected parse trees",
                                                      java_compatibility_verdict:
                                                          "exact Java 4.13.2 recognizer metadata, serialized ATN, and forced-alternative parse-tree equality",
                                                  }
            : {
                  primary: coveredExisting
                      ? "the case-specific Rust port matches the pinned accepted and rejected syntax outcomes"
                      : "pinned source cases passed in the recorded JUnit/Vitest discovery or immutable fixture snapshot",
                  alternate:
                      "alternate source cases passed in the recorded runner discovery or generated oracle",
                  java_compatibility_verdict:
                      "Java-compatible syntax; antlr-ng supplies the canonical Phase A CST shape",
              },
        resolution: row.resolution ?? "ported",
        testCommit: row.primary_test_commit,
        implementationCommit: row.primary_implementation_commit,
        testCommand: row.green_result.command,
        greenResultText: row.green_result.result,
        lockedSections: phaseBProfile
            ? phaseBProfile.lockedSections
            : coveredExisting
              ? syntaxLockedSections
              : defaultLockedSections,
        ownerPhase: row.owner_phase,
        scaffoldCommit: phaseBProfile
            ? phaseBProfile.scaffoldCommit
            : SCAFFOLD_COMMIT,
        testParent: phaseBProfile
            ? phaseBProfile.testParent
            : coveredExisting
              ? FRONTEND_SYNTAX_TEST_PARENT
              : SCAFFOLD_COMMIT,
        implementationParent: phaseBProfile
            ? phaseBProfile.implementationParent
            : coveredExisting
              ? null
              : TEST_COMMIT,
        reachability: phaseBProfile
            ? phaseBProfile.reachability
            : coveredExisting
              ? "the case-specific test passed against an implementation already present in its parent"
              : "direct ancestry is verified when the recorded commit objects are available",
        demonstratedRed:
            phaseBAtnConstruction ||
            phaseBBasicSemantic ||
            phaseBErrorSets ||
            phaseBVocabulary ||
            phaseBScopeParsing ||
            phaseBCharSupport ||
            phaseBNestedAction ||
            (phaseBEscapeSequence && !coveredExisting) ||
            phaseBUnicodeEscape ||
            (phaseBUnicodeGrammar && !coveredExisting) ||
            (phaseBTokenAssignment && !coveredExisting) ||
            (phaseBTokenPosition && !coveredExisting) ||
            (phaseBLeftRecursion && !coveredExisting) ||
            (phaseBGraphNodes && !coveredExisting) ||
            (phaseBSymbolIssues && !coveredExisting) ||
            (phaseBAttributeChecks && !coveredExisting) ||
            (phaseBToolSyntaxErrors && !coveredExisting) ||
            (phaseBCompositeGrammars && !coveredExisting) ||
            phaseBLookaheadTree
            ? row.demonstrated_red
            : undefined,
    });
}

const externalSerialized = `${JSON.stringify(externalMap, null, 2)}\n`;
if (update) {
    await writeFile(externalMapPath, externalSerialized, "utf8");
} else if ((await readFile(externalMapPath, "utf8")) !== externalSerialized) {
    throw new Error("external-fixture-map.json evidence fields are stale");
}

for (const [path, contents] of expectedFiles) {
    const absolutePath = resolve(repoRoot, path);
    if (update) {
        await mkdir(dirname(absolutePath), { recursive: true });
        await writeFile(absolutePath, contents, "utf8");
    } else if ((await readFile(absolutePath, "utf8")) !== contents) {
        throw new Error(`port evidence is stale: ${path}`);
    }
}

console.log(
    `${update ? "updated" : "verified"} ${completedRows.length + Object.keys(EXTERNAL_DEFINITIONS).length} completed evidence ledgers`,
);

async function addEvidence({
    logicalId,
    revisionId,
    closure,
    closureHash,
    sourceCaseIds,
    externalSource,
    primaryTestSource,
    alternateTestSource,
    declaredOutcomes,
    resolution,
    testCommit,
    implementationCommit,
    testCommand,
    greenResultText,
    lockedSections,
    ownerPhase,
    scaffoldCommit,
    testParent,
    implementationParent,
    reachability,
    demonstratedRed,
}) {
    const base = `tests/codegen-direct/port-evidence/${logicalId}`;
    const revisionBase = `${base}/revisions/${revisionId}`;
    const indexPath = `${base}/index.json`;
    const existingIndex = await loadOptional(indexPath);
    const existingRevision = existingIndex?.revisions?.find(
        (revision) => revision.revision_id === revisionId,
    );
    const supersedesRevisionId =
        existingRevision?.supersedes_revision_id ??
        (existingIndex?.active_revision_id &&
        existingIndex.active_revision_id !== revisionId
            ? existingIndex.active_revision_id
            : null);
    const coveredExisting = resolution === "verified-covered-existing";
    const oracleResults = {
        schema_version: 1,
        logical_id: logicalId,
        revision_id: revisionId,
        primary_test_source: primaryTestSource,
        alternate_test_source: alternateTestSource,
        outcomes: declaredOutcomes,
    };
    const matrixResults = {
        schema_version: 1,
        logical_id: logicalId,
        revision_id: revisionId,
        cells: [
            {
                test_port: coveredExisting ? "coverage-extension" : "primary",
                test_commit: testCommit,
                implementation_port: coveredExisting
                    ? "existing-primary-antlr-ng"
                    : "primary-antlr-ng",
                implementation_commit: implementationCommit,
                command: testCommand,
                result: `green: ${greenResultText}`,
            },
        ],
        escalation: coveredExisting
            ? ownerPhase === "A"
                ? "not required because the case-specific test passed against the existing Phase A frontend"
                : `not required because the case-specific test passed against the existing Phase ${ownerPhase} implementation`
            : "not required because the primary implementation passed the locked primary tests",
    };
    const oraclePath = `${revisionBase}/oracle-results/declared-sources.json`;
    const matrixPath = `${revisionBase}/matrix-results/results.json`;
    const oracleSerialized = `${JSON.stringify(oracleResults, null, 2)}\n`;
    const matrixSerialized = `${JSON.stringify(matrixResults, null, 2)}\n`;
    expectedFiles.set(oraclePath, oracleSerialized);
    expectedFiles.set(matrixPath, matrixSerialized);

    const allowedInputs = sourceCaseIds.map((id) => {
        const testCase = sourceCases.get(id);
        if (!testCase) {
            throw new Error(`${logicalId} references unknown source case ${id}`);
        }
        return {
            source_case_id: id,
            path: testCase.source.path,
            sha256: testCase.source.sha256,
        };
    });
    if (externalSource) {
        allowedInputs.push({
            source_id: externalSource.source_id,
            path: externalSource.mirror_path,
            sha256: externalSource.sha256,
        });
    }
    for (const fixturePath of closure.fixture_paths ?? []) {
        allowedInputs.push({
            path: fixturePath,
            sha256: digest(await readFile(resolve(repoRoot, fixturePath))),
        });
    }

    const manifest = {
        schema_version: 1,
        logical_id: logicalId,
        revision_id: revisionId,
        supersedes_revision_id: supersedesRevisionId,
        owner_phase: ownerPhase,
        state: "done",
        ...(coveredExisting ? { resolution } : {}),
        closure,
        closure_sha256: closureHash,
        allowed_inputs: allowedInputs,
        commits: {
            scaffold: scaffoldCommit,
            primary_test: testCommit,
            primary_implementation: implementationCommit,
        },
        ancestry: {
            primary_test_parent: testParent,
            primary_implementation_parent: implementationParent,
            reachability,
        },
        locked_oracle_sections: lockedSections,
        ...(coveredExisting
            ? {
                  verified_covered_existing: {
                      command: testCommand,
                      commit: testCommit,
                      exit_code: 0,
                      result: greenResultText,
                      covering_implementation_commit: implementationCommit,
                  },
              }
            : {
                  demonstrated_red: redResult(
                      demonstratedRed?.command ?? testCommand,
                      testCommit,
                      demonstratedRed?.exit_code ?? 101,
                      demonstratedRed?.fingerprint,
                  ),
              }),
        green_result: greenResult(
            testCommand,
            coveredExisting ? testCommit : implementationCommit,
            greenResultText,
        ),
        implementation_sources: {
            primary: `antlr-ng@${ANTLR_NG_COMMIT}`,
            alternate: `java-antlr@${JAVA_COMMIT}`,
        },
        evidence_files: [
            {
                path: oraclePath,
                sha256: digest(oracleSerialized),
            },
            {
                path: matrixPath,
                sha256: digest(matrixSerialized),
            },
        ],
    };
    const manifestPath = `${revisionBase}/manifest.json`;
    expectedFiles.set(
        manifestPath,
        `${JSON.stringify(manifest, null, 2)}\n`,
    );
    const revisions = (existingIndex?.revisions ?? []).filter(
        (revision) => revision.revision_id !== revisionId,
    );
    revisions.push({
        revision_id: revisionId,
        supersedes_revision_id: supersedesRevisionId,
        state: "done",
        manifest_path: manifestPath,
        closure_sha256: closureHash,
    });
    const index = {
        schema_version: 1,
        logical_id: logicalId,
        active_revision_id: revisionId,
        revisions,
    };
    expectedFiles.set(indexPath, `${JSON.stringify(index, null, 2)}\n`);
}

function redResult(
    command = TEST_COMMAND,
    commit = TEST_COMMIT,
    exitCode = 101,
    fingerprint = "G4F000: the Stage 0 grammar frontend is not installed",
) {
    return {
        command,
        commit,
        exit_code: exitCode,
        fingerprint,
    };
}

function greenResult(
    command = TEST_COMMAND,
    commit = IMPLEMENTATION_COMMIT,
    result = "5 passed; 0 failed",
) {
    return {
        command,
        commit,
        exit_code: 0,
        result,
    };
}

function sectionAtMarker(text, marker) {
    const offset = text.indexOf(marker);
    if (offset < 0) {
        throw new Error(`cannot find locked section marker ${marker}`);
    }
    return text.slice(offset);
}

function sectionBetweenMarkers(text, marker, endMarker) {
    const offset = text.indexOf(marker);
    if (offset < 0) {
        throw new Error(`cannot find locked section marker ${marker}`);
    }
    const end = text.indexOf(endMarker, offset);
    if (end < 0) {
        throw new Error(`cannot find locked section end marker ${endMarker}`);
    }
    return text.slice(offset, end);
}

function warnMissingHistoricalSource(label, commit, path) {
    console.warn(
        `warning: skipped ${label}; unavailable pinned Git source ${commit}:${path}`,
    );
}

async function load(path) {
    return JSON.parse(await readFile(resolve(repoRoot, path), "utf8"));
}

async function loadOptional(path) {
    try {
        return await load(path);
    } catch (error) {
        if (error.code === "ENOENT") {
            return null;
        }
        throw error;
    }
}
