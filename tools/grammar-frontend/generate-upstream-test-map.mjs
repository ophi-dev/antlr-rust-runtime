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
    FRONTEND_SYNTAX_TEST_COMMIT,
    IMPLEMENTATION_COMMIT,
    JAVA_COMMIT,
    PHASE_B_BASE_COMMIT,
    PHASE_B_IMPLEMENTATION_COMMIT,
    SCAFFOLD_COMMIT,
    TEST_COMMIT,
    digest,
    parseMode,
    stableStringify,
} from "./evidence-common.mjs";

const APPROVING_REVIEW = "merged implementation plan PR #149, section 11.5";
const FRONTEND_TEST_COMMAND =
    "cargo test --locked --bin antlr4-rust-gen grammar::frontend::tests::";
const FRONTEND_SYNTAX_TEST_COMMAND =
    "cargo test --locked --bin antlr4-rust-gen grammar::ported_tests::frontend_tool_syntax_cases_match_upstream_outcomes";
const ATN_SERIALIZATION_TEST_COMMAND =
    "cargo test --locked --bin antlr4-rust-gen grammar::atn::interp_test::tests::upstream_atn_serialization::";
const ATN_SERIALIZATION_TEST_MODULE =
    "src/bin_support/grammar/atn/interp_test.rs";
const ATN_CONSTRUCTION_TEST_COMMAND =
    "cargo test --locked --bin antlr4-rust-gen grammar::atn::interp_test::tests::upstream_atn_construction::";
const ATN_CONSTRUCTION_COVERED_COMMAND =
    "cargo test --locked --bin antlr4-rust-gen upstream_atn_construction -- --test-threads=1 --skip parser_rule_ref_in_lexer_rule --skip repeated_transitions_to_stop_state";
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
    "cargo test --locked --bin antlr4-rust-gen upstream_basic_semantic_errors -- --test-threads=1";
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
    if (ports.size !== 79) {
        throw new Error(`expected 79 completed Phase B ports, found ${ports.size}`);
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
