#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const ANTLR_NG_COMMIT = "1f68422ae4bfc62f93343769e144d01f305487b1";
const TEST_COMMIT = "a4258562c44818e2ba97d206587c64d4c38408d0";
const IMPLEMENTATION_COMMIT = "8a00a3d6496779b969a42511d7e29c0d102d62d7";
const SCAFFOLD_COMMIT = "75615945749dc93fca5d929cb22ad481f12dfdc9";
const APPROVING_REVIEW = "merged implementation plan PR #149, section 11.5";

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
const update = parseMode(process.argv.slice(2));
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
            "grammar::frontend::tests::malformed_bootstrap_inputs_fail_closed",
        unitUnderTest: "Stage 0 lexer/parser fail-closed boundary",
        observable:
            "grammar syntax errors and unterminated constructs return diagnostics without a usable CST",
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
        primary_test_commit: TEST_COMMIT,
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
        active_revision_id: `${logicalId}-r1`,
        tdd_state: "done",
        rust_test: rustTest,
        primary_test_source: primaryTestSource,
        alternate_test_source: alternateTestSource,
        primary_implementation_source: `antlr-ng@${ANTLR_NG_COMMIT}`,
        alternate_implementation_source: `java-antlr@${JAVA_COMMIT}`,
        prerequisites: ["behavior-free grammar frontend scaffold"],
        unit_under_test: unitUnderTest,
        expected_red_failure_fingerprint:
            "red fingerprint: Stage 0 frontend is not installed",
        observable_equivalence: observable,
        scaffold_commit: SCAFFOLD_COMMIT,
        primary_test_commit: TEST_COMMIT,
        demonstrated_red: {
            command:
                "cargo test --locked --bin antlr4-rust-gen grammar::frontend::tests::",
            exit_code: 101,
            fingerprint: "G4F000 Stage 0 frontend is not installed",
        },
        primary_implementation_commit: IMPLEMENTATION_COMMIT,
        green_result: {
            command:
                "cargo test --locked --bin antlr4-rust-gen grammar::frontend::tests::",
            result: "5 passed; 0 failed",
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
    if (policy.disposition !== "port") {
        return {
            logical_id: logicalId,
            source_case_ids: sourceCaseIds,
            external_assertion_ids: [],
            owner_phase: policy.owner,
            disposition: policy.disposition,
            active_revision_id: null,
            rationale: `${policy.rationale}; case ${cases[0].suite}.${cases[0].name}`,
            covering_evidence: policy.evidence,
            approving_reviewer: APPROVING_REVIEW,
        };
    }

    const closure = {
        logical_id: logicalId,
        source_case_ids: sourceCaseIds,
        external_assertion_ids: [],
        owner_phase: policy.owner,
        disposition: "port",
        rust_test: `planned:tests/codegen-direct/fixtures/${logicalId}`,
        unit_under_test: policy.unit,
        observable: `pinned ${cases[0].suite}.${cases[0].name} behavior`,
    };
    return {
        logical_id: logicalId,
        source_case_ids: sourceCaseIds,
        external_assertion_ids: [],
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

function stableStringify(value) {
    if (Array.isArray(value)) {
        return `[${value.map(stableStringify).join(",")}]`;
    }
    if (value && typeof value === "object") {
        return `{${Object.keys(value)
            .sort()
            .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
            .join(",")}}`;
    }
    return JSON.stringify(value);
}

function digest(value) {
    return createHash("sha256").update(value).digest("hex");
}

function parseMode(args) {
    if (args.length !== 1 || !["--check", "--update"].includes(args[0])) {
        throw new Error("usage: generate-upstream-test-map.mjs --check|--update");
    }
    return args[0] === "--update";
}
