#!/usr/bin/env node

import { readdir, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

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
    UNICODE_ESCAPE_SCAFFOLD_PARENT_COMMIT,
    UNICODE_ESCAPE_TEST_COMMIT,
    UNICODE_GRAMMAR_BASE_COMMIT,
    UNICODE_GRAMMAR_BASE_PARENT_COMMIT,
    UNICODE_GRAMMAR_IMPLEMENTATION_COMMIT,
    UNICODE_GRAMMAR_TEST_COMMIT,
    VOCABULARY_BASE_COMMIT,
    VOCABULARY_IMPLEMENTATION_COMMIT,
    VOCABULARY_TEST_COMMIT,
    digest,
    gitShowOptional,
    stableStringify,
} from "./evidence-common.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const EMPTY_VOCABULARY_LOGICAL_ID =
    "testvocabulary-testemptyvocabulary-66d31ad014";
const NESTED_ACTION_LOGICAL_ID =
    "testlexeractions-nested-actions-3d175db5e5";
const evidenceRoot = resolve(
    repoRoot,
    "tests/codegen-direct/port-evidence",
);
const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const externalMap = await load("tests/codegen-direct/external-fixture-map.json");
const upstreamInventory = await load(
    "tests/codegen-direct/upstream-case-inventory.json",
);
const externalInventory = await load(
    "tests/codegen-direct/external-source-inventory.json",
);
const differences = await load(
    "tests/codegen-direct/approved-differences.json",
);
const failures = [];
const records = new Map();
const sourceCases = new Map(
    (upstreamInventory.cases ?? []).map((testCase) => [testCase.id, testCase]),
);
const externalSources = new Map(
    (externalInventory.artifacts ?? []).map((source) => [source.source_id, source]),
);

for (const row of testMap.rows ?? []) {
    if (row.disposition === "port" && row.tdd_state === "done") {
        records.set(row.logical_id, {
            revisionId: row.active_revision_id,
            closure: row.closure,
            closureHash: row.closure_sha256,
            evidencePath: row.evidence_path,
            resolution: row.resolution ?? "ported",
            testCommit: row.primary_test_commit,
            implementationCommit: row.primary_implementation_commit,
            ownerPhase: row.owner_phase,
        });
    }
}
for (const fixture of externalMap.fixtures ?? []) {
    for (const assertion of fixture.assertions ?? []) {
        if (assertion.tdd_owner.startsWith("external:")) {
            records.set(assertion.id, {
                revisionId: assertion.active_revision_id,
                closure: assertion.tdd?.closure,
                closureHash: assertion.tdd?.closure_sha256,
                evidencePath: assertion.tdd?.evidence_path,
                resolution: "ported",
                testCommit: TEST_COMMIT,
                implementationCommit: IMPLEMENTATION_COMMIT,
                ownerPhase: assertion.phase,
            });
        }
    }
}

const actualDirectories = (await readdir(evidenceRoot, { withFileTypes: true }))
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
const expectedDirectories = [...records.keys()].sort();
expect(
    JSON.stringify(actualDirectories) === JSON.stringify(expectedDirectories),
    "port-evidence directories do not exactly match completed active records",
);

const globalRevisionIds = new Set();
for (const [logicalId, record] of records) {
    expect(
        record.evidencePath ===
            `tests/codegen-direct/port-evidence/${logicalId}`,
        `${logicalId} evidence path differs`,
    );
    const index = await load(`${record.evidencePath}/index.json`);
    expect(index.schema_version === 1, `${logicalId} index schema differs`);
    expect(index.logical_id === logicalId, `${logicalId} index logical ID differs`);
    expect(
        index.active_revision_id === record.revisionId,
        `${logicalId} active revision differs`,
    );
    expect(
        Array.isArray(index.revisions) && index.revisions.length > 0,
        `${logicalId} has no ledger revisions`,
    );

    const revisions = new Map();
    const successors = new Map();
    for (const revision of index.revisions ?? []) {
        expect(
            typeof revision.revision_id === "string" &&
                !globalRevisionIds.has(revision.revision_id),
            `${logicalId} has duplicate or missing global revision ID`,
        );
        globalRevisionIds.add(revision.revision_id);
        revisions.set(revision.revision_id, revision);
        if (revision.supersedes_revision_id) {
            const count = (successors.get(revision.supersedes_revision_id) ?? 0) + 1;
            successors.set(revision.supersedes_revision_id, count);
            expect(
                count === 1,
                `${logicalId} revision has multiple direct successors`,
            );
        }
    }
    const leaves = [...revisions.keys()].filter(
        (revisionId) => !successors.has(revisionId),
    );
    expect(
        leaves.length === 1 && leaves[0] === record.revisionId,
        `${logicalId} active revision is not the unique leaf`,
    );

    for (const revision of revisions.values()) {
        if (revision.supersedes_revision_id) {
            expect(
                revisions.has(revision.supersedes_revision_id),
                `${logicalId} revision has a missing predecessor`,
            );
        }
        const manifest = await load(revision.manifest_path);
        expect(
            manifest.logical_id === logicalId,
            `${logicalId} manifest logical ID differs`,
        );
        expect(
            manifest.revision_id === revision.revision_id,
            `${logicalId} manifest revision ID differs`,
        );
        expect(
            manifest.supersedes_revision_id === revision.supersedes_revision_id,
            `${logicalId} supersession edge differs`,
        );
        expect(
            revision.closure_sha256 === manifest.closure_sha256,
            `${logicalId} index closure hash differs`,
        );
        expect(
            manifest.closure_sha256 === digest(stableStringify(manifest.closure)),
            `${logicalId} manifest closure hash is invalid`,
        );
        await validateAllowedInputs(logicalId, manifest);
        for (const evidenceFile of manifest.evidence_files ?? []) {
            const contents = await readFile(resolve(repoRoot, evidenceFile.path));
            expect(
                digest(contents) === evidenceFile.sha256,
                `${logicalId} evidence hash differs for ${evidenceFile.path}`,
            );
        }
        for (const section of manifest.locked_oracle_sections ?? []) {
            const activeRevision = revision.revision_id === record.revisionId;
            if (activeRevision) {
                const checkedIn = lockedSection(
                    await readFile(resolve(repoRoot, section.path), "utf8"),
                    section,
                );
                expect(
                    digest(checkedIn) === section.sha256,
                    `${logicalId} locked oracle section hash differs`,
                );
            }
            const testSource = gitShowOptional(
                repoRoot,
                manifest.commits.primary_test,
                section.path,
            );
            const implementationSource = gitShowOptional(
                repoRoot,
                manifest.commits.primary_implementation,
                section.path,
            );
            if (testSource !== null) {
                const locked = lockedSection(testSource, section);
                expect(
                    digest(locked) === section.sha256,
                    `${logicalId} historical locked oracle section hash differs`,
                );
                if (
                    (manifest.resolution ?? "ported") === "ported" &&
                    implementationSource !== null
                ) {
                    const afterImplementation = lockedSection(
                        implementationSource,
                        section,
                    );
                    expect(
                        locked === afterImplementation,
                        `${logicalId} implementation commit edited its locked oracle section`,
                    );
                }
            }
        }
        const resolution = manifest.resolution ?? "ported";
        expect(
            ["ported", "verified-covered-existing"].includes(resolution),
            `${logicalId} manifest resolution is invalid`,
        );
        expect(
            manifest.owner_phase === record.ownerPhase,
            `${logicalId} manifest owner phase differs`,
        );
        const atnSerialization = logicalId.startsWith(
            "testatnserialization-",
        );
        const atnConstruction = logicalId.startsWith(
            "testatnconstruction-",
        );
        const basicSemantic = logicalId.startsWith(
            "testbasicsemanticerrors-",
        );
        const errorSets = logicalId.startsWith(
            "testerrorsets-",
        );
        const tokenPosition = logicalId.startsWith(
            "testtokenpositionoptions-",
        );
        const topologicalSort = logicalId.startsWith(
            "testtopologicalsort-",
        );
        const vocabulary = logicalId.startsWith(
            "testvocabulary-",
        );
        const scopeParsing = logicalId.startsWith(
            "testscopeparsing-",
        );
        const charSupport = logicalId.startsWith(
            "testcharsupport-",
        );
        const nestedAction =
            logicalId === NESTED_ACTION_LOGICAL_ID;
        const escapeSequence = logicalId.startsWith(
            "testescapesequenceparsing-",
        );
        const unicodeEscape = logicalId.startsWith(
            "testunicodeescapes-",
        );
        const unicodeData = logicalId.startsWith(
            "testunicodedata-",
        );
        const unicodeGrammar = logicalId.startsWith(
            "testunicodegrammar-",
        );
        const tokenAssignment = logicalId.startsWith(
            "testtokentypeassignment-",
        );
        const leftRecursion = logicalId.startsWith(
            "testleftrecursiontoolissues-",
        );
        const lookaheadTree = logicalId.startsWith(
            "testlookaheadtrees-",
        );
        const graphNodes = logicalId.startsWith(
            "testgraphnodes-",
        );
        const symbolIssues = logicalId.startsWith(
            "testsymbolissues-",
        );
        const attributeChecks = logicalId.startsWith(
            "testattributechecks-",
        );
        const toolSyntaxErrors = logicalId.startsWith(
            "testtoolsyntaxerrors-",
        );
        const compositeGrammars = logicalId.startsWith(
            "testcompositegrammars-",
        );
        if (resolution === "verified-covered-existing") {
            if (atnSerialization) {
                expect(
                    manifest.commits.scaffold === PHASE_B_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            ATN_SERIALIZATION_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            PHASE_B_IMPLEMENTATION_COMMIT,
                    `${logicalId} Phase B covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            PHASE_B_IMPLEMENTATION_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            PHASE_B_BASE_COMMIT,
                    `${logicalId} Phase B covered-existing ancestry differs`,
                );
            } else if (atnConstruction) {
                expect(
                    manifest.commits.scaffold ===
                            ATN_CONSTRUCTION_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            ATN_CONSTRUCTION_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            PHASE_B_IMPLEMENTATION_COMMIT,
                    `${logicalId} ATN construction covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            ATN_CONSTRUCTION_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            PHASE_B_BASE_COMMIT,
                    `${logicalId} ATN construction covered-existing ancestry differs`,
                );
            } else if (tokenPosition) {
                expect(
                    manifest.commits.scaffold ===
                            TOKEN_POSITION_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            TOKEN_POSITION_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            TOKEN_POSITION_BASE_COMMIT,
                    `${logicalId} token position covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            TOKEN_POSITION_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            ERROR_SETS_IMPLEMENTATION_COMMIT,
                    `${logicalId} token position covered-existing ancestry differs`,
                );
            } else if (topologicalSort) {
                expect(
                    manifest.commits.scaffold ===
                            TOPOLOGICAL_SORT_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            TOPOLOGICAL_SORT_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            TOPOLOGICAL_SORT_BASE_COMMIT,
                    `${logicalId} topological sort covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            TOPOLOGICAL_SORT_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            TOKEN_POSITION_IMPLEMENTATION_COMMIT,
                    `${logicalId} topological sort covered-existing ancestry differs`,
                );
            } else if (escapeSequence) {
                expect(
                    manifest.commits.scaffold ===
                            ESCAPE_SEQUENCE_SCAFFOLD_COMMIT &&
                        manifest.commits.primary_test ===
                            ESCAPE_SEQUENCE_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            ESCAPE_SEQUENCE_SCAFFOLD_COMMIT,
                    `${logicalId} escape sequence covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            ESCAPE_SEQUENCE_SCAFFOLD_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            ESCAPE_SEQUENCE_SCAFFOLD_PARENT_COMMIT,
                    `${logicalId} escape sequence covered-existing ancestry differs`,
                );
            } else if (unicodeData) {
                expect(
                    manifest.commits.scaffold ===
                            UNICODE_DATA_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            UNICODE_DATA_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            UNICODE_DATA_BASE_COMMIT,
                    `${logicalId} Unicode data covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            UNICODE_DATA_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            UNICODE_DATA_BASE_PARENT_COMMIT,
                    `${logicalId} Unicode data covered-existing ancestry differs`,
                );
            } else if (unicodeGrammar) {
                expect(
                    manifest.commits.scaffold ===
                            UNICODE_GRAMMAR_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            UNICODE_GRAMMAR_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            UNICODE_GRAMMAR_BASE_COMMIT,
                    `${logicalId} Unicode grammar covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            UNICODE_GRAMMAR_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            UNICODE_GRAMMAR_BASE_PARENT_COMMIT,
                    `${logicalId} Unicode grammar covered-existing ancestry differs`,
                );
            } else if (tokenAssignment) {
                expect(
                    manifest.commits.scaffold ===
                            TOKEN_ASSIGNMENT_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            TOKEN_ASSIGNMENT_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            TOKEN_ASSIGNMENT_BASE_COMMIT,
                    `${logicalId} token assignment covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            TOKEN_ASSIGNMENT_FIXTURE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            TOKEN_ASSIGNMENT_BASE_PARENT_COMMIT,
                    `${logicalId} token assignment covered-existing ancestry differs`,
                );
            } else if (leftRecursion) {
                expect(
                    manifest.commits.scaffold ===
                            LEFT_RECURSION_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            LEFT_RECURSION_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            LEFT_RECURSION_BASE_COMMIT,
                    `${logicalId} left recursion covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            LEFT_RECURSION_FIXTURE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            LEFT_RECURSION_BASE_PARENT_COMMIT,
                    `${logicalId} left recursion covered-existing ancestry differs`,
                );
            } else if (graphNodes) {
                expect(
                    manifest.commits.scaffold ===
                            GRAPH_NODES_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            GRAPH_NODES_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            GRAPH_NODES_BASE_COMMIT,
                    `${logicalId} GraphNodes covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            GRAPH_NODES_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            GRAPH_NODES_BASE_PARENT_COMMIT,
                    `${logicalId} GraphNodes covered-existing ancestry differs`,
                );
            } else if (symbolIssues) {
                expect(
                    manifest.commits.scaffold ===
                            SYMBOL_ISSUES_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            SYMBOL_ISSUES_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            SYMBOL_ISSUES_BASE_COMMIT,
                    `${logicalId} symbol issues covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            SYMBOL_ISSUES_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            SYMBOL_ISSUES_BASE_PARENT_COMMIT,
                    `${logicalId} symbol issues covered-existing ancestry differs`,
                );
            } else if (attributeChecks) {
                expect(
                    manifest.commits.scaffold ===
                            ATTRIBUTE_CHECKS_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            ATTRIBUTE_CHECKS_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            ATTRIBUTE_CHECKS_BASE_COMMIT,
                    `${logicalId} attribute checks covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            ATTRIBUTE_CHECKS_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            ATTRIBUTE_CHECKS_BASE_PARENT_COMMIT,
                        `${logicalId} attribute checks covered-existing ancestry differs`,
                );
            } else if (toolSyntaxErrors) {
                expect(
                    manifest.commits.scaffold ===
                            TOOL_SYNTAX_ERRORS_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            TOOL_SYNTAX_ERRORS_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            TOOL_SYNTAX_ERRORS_BASE_COMMIT,
                    `${logicalId} tool syntax covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            TOOL_SYNTAX_ERRORS_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            TOOL_SYNTAX_ERRORS_BASE_PARENT_COMMIT,
                    `${logicalId} tool syntax covered-existing ancestry differs`,
                );
            } else if (compositeGrammars) {
                expect(
                    manifest.commits.scaffold ===
                            COMPOSITE_GRAMMARS_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            COMPOSITE_GRAMMARS_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            COMPOSITE_GRAMMARS_BASE_COMMIT,
                    `${logicalId} composite grammar covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            COMPOSITE_GRAMMARS_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            COMPOSITE_GRAMMARS_BASE_PARENT_COMMIT,
                    `${logicalId} composite grammar covered-existing ancestry differs`,
                );
            } else {
                expect(
                    manifest.commits.scaffold === SCAFFOLD_COMMIT &&
                        manifest.commits.primary_test ===
                            FRONTEND_SYNTAX_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            IMPLEMENTATION_COMMIT,
                    `${logicalId} covered-existing commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            FRONTEND_SYNTAX_TEST_PARENT &&
                        manifest.ancestry.primary_implementation_parent === null,
                    `${logicalId} covered-existing ancestry differs`,
                );
            }
            expect(
                manifest.verified_covered_existing?.exit_code === 0 &&
                    manifest.verified_covered_existing
                        ?.covering_implementation_commit ===
                        manifest.commits.primary_implementation &&
                    manifest.green_result?.exit_code === 0,
                `${logicalId} lacks covered-existing execution evidence`,
            );
        } else {
            if (atnConstruction) {
                expect(
                    manifest.commits.scaffold ===
                            ATN_CONSTRUCTION_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            ATN_CONSTRUCTION_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            ATN_CONSTRUCTION_IMPLEMENTATION_COMMIT,
                    `${logicalId} ATN construction evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            ATN_CONSTRUCTION_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            ATN_CONSTRUCTION_TEST_COMMIT,
                    `${logicalId} ATN construction recorded ancestry differs`,
                );
            } else if (basicSemantic) {
                expect(
                    manifest.commits.scaffold ===
                            BASIC_SEMANTIC_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            BASIC_SEMANTIC_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            BASIC_SEMANTIC_IMPLEMENTATION_COMMIT,
                    `${logicalId} basic semantic evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            BASIC_SEMANTIC_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            BASIC_SEMANTIC_TEST_COMMIT,
                    `${logicalId} basic semantic recorded ancestry differs`,
                );
            } else if (errorSets) {
                expect(
                    manifest.commits.scaffold === ERROR_SETS_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            ERROR_SETS_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            ERROR_SETS_IMPLEMENTATION_COMMIT,
                    `${logicalId} lexer set evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            ERROR_SETS_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            ERROR_SETS_TEST_COMMIT,
                    `${logicalId} lexer set recorded ancestry differs`,
                );
            } else if (tokenPosition) {
                expect(
                    manifest.commits.scaffold ===
                            TOKEN_POSITION_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            TOKEN_POSITION_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            TOKEN_POSITION_IMPLEMENTATION_COMMIT,
                    `${logicalId} token position evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            TOKEN_POSITION_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            TOKEN_POSITION_TEST_COMMIT,
                    `${logicalId} token position recorded ancestry differs`,
                );
            } else if (vocabulary) {
                const emptyVocabulary =
                    logicalId === EMPTY_VOCABULARY_LOGICAL_ID;
                expect(
                    manifest.commits.scaffold ===
                            (emptyVocabulary
                                ? EMPTY_VOCABULARY_BASE_COMMIT
                                : VOCABULARY_BASE_COMMIT) &&
                        manifest.commits.primary_test ===
                            (emptyVocabulary
                                ? EMPTY_VOCABULARY_TEST_COMMIT
                                : VOCABULARY_TEST_COMMIT) &&
                        manifest.commits.primary_implementation ===
                            (emptyVocabulary
                                ? EMPTY_VOCABULARY_IMPLEMENTATION_COMMIT
                                : VOCABULARY_IMPLEMENTATION_COMMIT),
                    `${logicalId} vocabulary evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            (emptyVocabulary
                                ? EMPTY_VOCABULARY_BASE_COMMIT
                                : VOCABULARY_BASE_COMMIT) &&
                        manifest.ancestry.primary_implementation_parent ===
                            (emptyVocabulary
                                ? EMPTY_VOCABULARY_TEST_COMMIT
                                : VOCABULARY_TEST_COMMIT),
                    `${logicalId} vocabulary recorded ancestry differs`,
                );
            } else if (scopeParsing) {
                expect(
                    manifest.commits.scaffold ===
                            SCOPE_PARSING_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            SCOPE_PARSING_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            SCOPE_PARSING_IMPLEMENTATION_COMMIT,
                    `${logicalId} scope parsing evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            SCOPE_PARSING_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            SCOPE_PARSING_TEST_COMMIT,
                    `${logicalId} scope parsing recorded ancestry differs`,
                );
            } else if (charSupport) {
                expect(
                    manifest.commits.scaffold ===
                            CHAR_SUPPORT_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            CHAR_SUPPORT_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            CHAR_SUPPORT_IMPLEMENTATION_COMMIT,
                    `${logicalId} character support evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            CHAR_SUPPORT_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            CHAR_SUPPORT_TEST_COMMIT,
                    `${logicalId} character support recorded ancestry differs`,
                );
            } else if (nestedAction) {
                expect(
                    manifest.commits.scaffold ===
                            NESTED_ACTION_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            NESTED_ACTION_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            NESTED_ACTION_IMPLEMENTATION_COMMIT,
                    `${logicalId} nested action evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            NESTED_ACTION_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            NESTED_ACTION_TEST_COMMIT,
                    `${logicalId} nested action recorded ancestry differs`,
                );
            } else if (escapeSequence) {
                expect(
                    manifest.commits.scaffold ===
                            ESCAPE_SEQUENCE_SCAFFOLD_COMMIT &&
                        manifest.commits.primary_test ===
                            ESCAPE_SEQUENCE_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            ESCAPE_SEQUENCE_IMPLEMENTATION_COMMIT,
                    `${logicalId} escape sequence evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            ESCAPE_SEQUENCE_SCAFFOLD_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            ESCAPE_SEQUENCE_TEST_COMMIT,
                    `${logicalId} escape sequence recorded ancestry differs`,
                );
            } else if (unicodeEscape) {
                expect(
                    manifest.commits.scaffold ===
                            UNICODE_ESCAPE_SCAFFOLD_COMMIT &&
                        manifest.commits.primary_test ===
                            UNICODE_ESCAPE_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            UNICODE_ESCAPE_IMPLEMENTATION_COMMIT,
                    `${logicalId} Unicode escape evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            UNICODE_ESCAPE_SCAFFOLD_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            UNICODE_ESCAPE_TEST_COMMIT,
                    `${logicalId} Unicode escape recorded ancestry differs`,
                );
            } else if (unicodeGrammar) {
                expect(
                    manifest.commits.scaffold ===
                            UNICODE_GRAMMAR_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            UNICODE_GRAMMAR_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            UNICODE_GRAMMAR_IMPLEMENTATION_COMMIT,
                    `${logicalId} Unicode grammar evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            UNICODE_GRAMMAR_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            UNICODE_GRAMMAR_TEST_COMMIT,
                    `${logicalId} Unicode grammar recorded ancestry differs`,
                );
            } else if (tokenAssignment) {
                expect(
                    manifest.commits.scaffold ===
                            TOKEN_ASSIGNMENT_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            TOKEN_ASSIGNMENT_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            TOKEN_ASSIGNMENT_IMPLEMENTATION_COMMIT,
                    `${logicalId} token assignment evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            TOKEN_ASSIGNMENT_FIXTURE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            TOKEN_ASSIGNMENT_TEST_COMMIT,
                    `${logicalId} token assignment recorded ancestry differs`,
                );
            } else if (leftRecursion) {
                expect(
                    manifest.commits.scaffold ===
                            LEFT_RECURSION_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            LEFT_RECURSION_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            LEFT_RECURSION_IMPLEMENTATION_COMMIT,
                    `${logicalId} left recursion evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            LEFT_RECURSION_FIXTURE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            LEFT_RECURSION_TEST_COMMIT,
                    `${logicalId} left recursion recorded ancestry differs`,
                );
            } else if (graphNodes) {
                expect(
                    manifest.commits.scaffold ===
                            GRAPH_NODES_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            GRAPH_NODES_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            GRAPH_NODES_IMPLEMENTATION_COMMIT,
                    `${logicalId} GraphNodes evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            GRAPH_NODES_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            GRAPH_NODES_TEST_COMMIT,
                    `${logicalId} GraphNodes recorded ancestry differs`,
                );
            } else if (symbolIssues) {
                expect(
                    manifest.commits.scaffold ===
                            SYMBOL_ISSUES_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            SYMBOL_ISSUES_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            SYMBOL_ISSUES_IMPLEMENTATION_COMMIT,
                    `${logicalId} symbol issues evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            SYMBOL_ISSUES_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            SYMBOL_ISSUES_TEST_COMMIT,
                    `${logicalId} symbol issues recorded ancestry differs`,
                );
            } else if (attributeChecks) {
                expect(
                    manifest.commits.scaffold ===
                            ATTRIBUTE_CHECKS_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            ATTRIBUTE_CHECKS_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            ATTRIBUTE_CHECKS_IMPLEMENTATION_COMMIT,
                    `${logicalId} attribute checks evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            ATTRIBUTE_CHECKS_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            ATTRIBUTE_CHECKS_TEST_COMMIT,
                        `${logicalId} attribute checks recorded ancestry differs`,
                );
            } else if (toolSyntaxErrors) {
                expect(
                    manifest.commits.scaffold ===
                            TOOL_SYNTAX_ERRORS_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            TOOL_SYNTAX_ERRORS_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            TOOL_SYNTAX_ERRORS_IMPLEMENTATION_COMMIT,
                    `${logicalId} tool syntax evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            TOOL_SYNTAX_ERRORS_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            TOOL_SYNTAX_ERRORS_TEST_COMMIT,
                    `${logicalId} tool syntax recorded ancestry differs`,
                );
            } else if (compositeGrammars) {
                expect(
                    manifest.commits.scaffold ===
                            COMPOSITE_GRAMMARS_BASE_COMMIT &&
                        manifest.commits.primary_test ===
                            COMPOSITE_GRAMMARS_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            COMPOSITE_GRAMMARS_IMPLEMENTATION_COMMIT,
                    `${logicalId} composite grammar evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            COMPOSITE_GRAMMARS_BASE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            COMPOSITE_GRAMMARS_IMPLEMENTATION_PARENT_COMMIT,
                    `${logicalId} composite grammar recorded ancestry differs`,
                );
            } else if (lookaheadTree) {
                expect(
                    manifest.commits.scaffold ===
                            LOOKAHEAD_TREE_FIXTURE_COMMIT &&
                        manifest.commits.primary_test ===
                            LOOKAHEAD_TREE_TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            LOOKAHEAD_TREE_IMPLEMENTATION_COMMIT,
                    `${logicalId} lookahead tree evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent ===
                            LOOKAHEAD_TREE_FIXTURE_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            LOOKAHEAD_TREE_TEST_COMMIT,
                    `${logicalId} lookahead tree recorded ancestry differs`,
                );
            } else {
                expect(
                    manifest.commits.scaffold === SCAFFOLD_COMMIT &&
                        manifest.commits.primary_test === TEST_COMMIT &&
                        manifest.commits.primary_implementation ===
                            IMPLEMENTATION_COMMIT,
                    `${logicalId} evidence commit identities differ`,
                );
                expect(
                    manifest.ancestry.primary_test_parent === SCAFFOLD_COMMIT &&
                        manifest.ancestry.primary_implementation_parent ===
                            TEST_COMMIT,
                    `${logicalId} recorded ancestry differs`,
                );
            }
            expect(
                manifest.demonstrated_red?.exit_code !== 0 &&
                    manifest.green_result?.exit_code === 0,
                `${logicalId} lacks red/green execution evidence`,
            );
        }
    }

    const active = revisions.get(record.revisionId);
    const activeManifest = await load(active.manifest_path);
    expect(
        activeManifest.state === "done",
        `${logicalId} active manifest is not done`,
    );
    expect(
        activeManifest.closure_sha256 === record.closureHash,
        `${logicalId} map and ledger closure hashes differ`,
    );
    expect(
        stableStringify(activeManifest.closure) === stableStringify(record.closure),
        `${logicalId} map and ledger closures differ`,
    );
    expect(
        (activeManifest.resolution ?? "ported") === record.resolution &&
            activeManifest.commits.primary_test === record.testCommit &&
            activeManifest.commits.primary_implementation ===
                record.implementationCommit,
        `${logicalId} map and ledger resolution evidence differs`,
    );
}

const testParent = gitOptional(["rev-parse", `${TEST_COMMIT}^`]);
if (testParent !== null) {
    expect(
        testParent.trim() === SCAFFOLD_COMMIT,
        "primary test commit is not directly based on the scaffold",
    );
}
const implementationParent = gitOptional([
    "rev-parse",
    `${IMPLEMENTATION_COMMIT}^`,
]);
if (implementationParent !== null) {
    expect(
        implementationParent.trim() === TEST_COMMIT,
        "primary implementation commit is not directly based on the locked test",
    );
}
const frontendSyntaxTestParent = gitOptional([
    "rev-parse",
    `${FRONTEND_SYNTAX_TEST_COMMIT}^`,
]);
if (frontendSyntaxTestParent !== null) {
    expect(
        frontendSyntaxTestParent.trim() === FRONTEND_SYNTAX_TEST_PARENT,
        "frontend syntax test commit has an unexpected parent",
    );
}
const atnSerializationTestParent = gitOptional([
    "rev-parse",
    `${ATN_SERIALIZATION_TEST_COMMIT}^`,
]);
if (atnSerializationTestParent !== null) {
    expect(
        atnSerializationTestParent.trim() === PHASE_B_IMPLEMENTATION_COMMIT,
        "ATN serialization test commit is not based on the Phase B implementation",
    );
}
const phaseBImplementationParent = gitOptional([
    "rev-parse",
    `${PHASE_B_IMPLEMENTATION_COMMIT}^`,
]);
if (phaseBImplementationParent !== null) {
    expect(
        phaseBImplementationParent.trim() === PHASE_B_BASE_COMMIT,
        "Phase B implementation commit has an unexpected parent",
    );
}
const atnConstructionTestParent = gitOptional([
    "rev-parse",
    `${ATN_CONSTRUCTION_TEST_COMMIT}^`,
]);
if (atnConstructionTestParent !== null) {
    expect(
        atnConstructionTestParent.trim() === ATN_CONSTRUCTION_BASE_COMMIT,
        "ATN construction test commit has an unexpected parent",
    );
}
const atnConstructionImplementationParent = gitOptional([
    "rev-parse",
    `${ATN_CONSTRUCTION_IMPLEMENTATION_COMMIT}^`,
]);
if (atnConstructionImplementationParent !== null) {
    expect(
        atnConstructionImplementationParent.trim() ===
            ATN_CONSTRUCTION_TEST_COMMIT,
        "ATN construction implementation commit is not based on its locked tests",
    );
}
const basicSemanticTestParent = gitOptional([
    "rev-parse",
    `${BASIC_SEMANTIC_TEST_COMMIT}^`,
]);
if (basicSemanticTestParent !== null) {
    expect(
        basicSemanticTestParent.trim() === BASIC_SEMANTIC_BASE_COMMIT,
        "basic semantic test commit is not based on its recorded base",
    );
}
const basicSemanticImplementationParent = gitOptional([
    "rev-parse",
    `${BASIC_SEMANTIC_IMPLEMENTATION_COMMIT}^`,
]);
if (basicSemanticImplementationParent !== null) {
    expect(
        basicSemanticImplementationParent.trim() ===
            BASIC_SEMANTIC_TEST_COMMIT,
        "basic semantic implementation commit is not based on its locked tests",
    );
}
const errorSetsTestParent = gitOptional([
    "rev-parse",
    `${ERROR_SETS_TEST_COMMIT}^`,
]);
if (errorSetsTestParent !== null) {
    expect(
        errorSetsTestParent.trim() === ERROR_SETS_BASE_COMMIT,
        "lexer set test commit is not based on its recorded base",
    );
}
const errorSetsImplementationParent = gitOptional([
    "rev-parse",
    `${ERROR_SETS_IMPLEMENTATION_COMMIT}^`,
]);
if (errorSetsImplementationParent !== null) {
    expect(
        errorSetsImplementationParent.trim() === ERROR_SETS_TEST_COMMIT,
        "lexer set implementation commit is not based on its locked tests",
    );
}
const tokenPositionTestParent = gitOptional([
    "rev-parse",
    `${TOKEN_POSITION_TEST_COMMIT}^`,
]);
if (tokenPositionTestParent !== null) {
    expect(
        tokenPositionTestParent.trim() === TOKEN_POSITION_BASE_COMMIT,
        "token position test commit is not based on its recorded base",
    );
}
const tokenPositionImplementationParent = gitOptional([
    "rev-parse",
    `${TOKEN_POSITION_IMPLEMENTATION_COMMIT}^`,
]);
if (tokenPositionImplementationParent !== null) {
    expect(
        tokenPositionImplementationParent.trim() ===
            TOKEN_POSITION_TEST_COMMIT,
        "token position implementation commit is not based on its locked tests",
    );
}
const topologicalSortTestParent = gitOptional([
    "rev-parse",
    `${TOPOLOGICAL_SORT_TEST_COMMIT}^`,
]);
if (topologicalSortTestParent !== null) {
    expect(
        topologicalSortTestParent.trim() === TOPOLOGICAL_SORT_BASE_COMMIT,
        "topological sort test commit is not based on its recorded base",
    );
}
const vocabularyTestParent = gitOptional([
    "rev-parse",
    `${VOCABULARY_TEST_COMMIT}^`,
]);
if (vocabularyTestParent !== null) {
    expect(
        vocabularyTestParent.trim() === VOCABULARY_BASE_COMMIT,
        "vocabulary test commit is not based on its recorded base",
    );
}
const vocabularyImplementationParent = gitOptional([
    "rev-parse",
    `${VOCABULARY_IMPLEMENTATION_COMMIT}^`,
]);
if (vocabularyImplementationParent !== null) {
    expect(
        vocabularyImplementationParent.trim() ===
            VOCABULARY_TEST_COMMIT,
        "vocabulary implementation commit is not based on its locked test",
    );
}
const emptyVocabularyTestParent = gitOptional([
    "rev-parse",
    `${EMPTY_VOCABULARY_TEST_COMMIT}^`,
]);
if (emptyVocabularyTestParent !== null) {
    expect(
        emptyVocabularyTestParent.trim() ===
            EMPTY_VOCABULARY_BASE_COMMIT,
        "empty vocabulary test commit is not based on its recorded base",
    );
}
const emptyVocabularyImplementationParent = gitOptional([
    "rev-parse",
    `${EMPTY_VOCABULARY_IMPLEMENTATION_COMMIT}^`,
]);
if (emptyVocabularyImplementationParent !== null) {
    expect(
        emptyVocabularyImplementationParent.trim() ===
            EMPTY_VOCABULARY_TEST_COMMIT,
        "empty vocabulary implementation commit is not based on its locked test",
    );
}
const nestedActionTestParent = gitOptional([
    "rev-parse",
    `${NESTED_ACTION_TEST_COMMIT}^`,
]);
if (nestedActionTestParent !== null) {
    expect(
        nestedActionTestParent.trim() === NESTED_ACTION_BASE_COMMIT,
        "nested action test commit is not based on its recorded base",
    );
}
const nestedActionImplementationParent = gitOptional([
    "rev-parse",
    `${NESTED_ACTION_IMPLEMENTATION_COMMIT}^`,
]);
if (nestedActionImplementationParent !== null) {
    expect(
        nestedActionImplementationParent.trim() ===
            NESTED_ACTION_TEST_COMMIT,
        "nested action implementation commit is not based on its locked test",
    );
}
const escapeSequenceScaffoldParent = gitOptional([
    "rev-parse",
    `${ESCAPE_SEQUENCE_SCAFFOLD_COMMIT}^`,
]);
if (escapeSequenceScaffoldParent !== null) {
    expect(
        escapeSequenceScaffoldParent.trim() ===
            ESCAPE_SEQUENCE_SCAFFOLD_PARENT_COMMIT,
        "escape sequence scaffold commit has an unexpected parent",
    );
}
const escapeSequenceTestParent = gitOptional([
    "rev-parse",
    `${ESCAPE_SEQUENCE_TEST_COMMIT}^`,
]);
if (escapeSequenceTestParent !== null) {
    expect(
        escapeSequenceTestParent.trim() ===
            ESCAPE_SEQUENCE_SCAFFOLD_COMMIT,
        "escape sequence test commit is not based on its scaffold",
    );
}
const escapeSequenceImplementationParent = gitOptional([
    "rev-parse",
    `${ESCAPE_SEQUENCE_IMPLEMENTATION_COMMIT}^`,
]);
if (escapeSequenceImplementationParent !== null) {
    expect(
        escapeSequenceImplementationParent.trim() ===
            ESCAPE_SEQUENCE_TEST_COMMIT,
        "escape sequence implementation commit is not based on its locked tests",
    );
}
const unicodeEscapeScaffoldParent = gitOptional([
    "rev-parse",
    `${UNICODE_ESCAPE_SCAFFOLD_COMMIT}^`,
]);
if (unicodeEscapeScaffoldParent !== null) {
    expect(
        unicodeEscapeScaffoldParent.trim() ===
            UNICODE_ESCAPE_SCAFFOLD_PARENT_COMMIT,
        "Unicode escape scaffold commit has an unexpected parent",
    );
}
const unicodeEscapeTestParent = gitOptional([
    "rev-parse",
    `${UNICODE_ESCAPE_TEST_COMMIT}^`,
]);
if (unicodeEscapeTestParent !== null) {
    expect(
        unicodeEscapeTestParent.trim() ===
            UNICODE_ESCAPE_SCAFFOLD_COMMIT,
        "Unicode escape test commit is not based on its scaffold",
    );
}
const unicodeEscapeImplementationParent = gitOptional([
    "rev-parse",
    `${UNICODE_ESCAPE_IMPLEMENTATION_COMMIT}^`,
]);
if (unicodeEscapeImplementationParent !== null) {
    expect(
        unicodeEscapeImplementationParent.trim() ===
            UNICODE_ESCAPE_TEST_COMMIT,
        "Unicode escape implementation commit is not based on its locked tests",
    );
}
const unicodeDataBaseParent = gitOptional([
    "rev-parse",
    `${UNICODE_DATA_BASE_COMMIT}^`,
]);
if (unicodeDataBaseParent !== null) {
    expect(
        unicodeDataBaseParent.trim() ===
            UNICODE_DATA_BASE_PARENT_COMMIT,
        "Unicode data base commit has an unexpected parent",
    );
}
const unicodeDataTestParent = gitOptional([
    "rev-parse",
    `${UNICODE_DATA_TEST_COMMIT}^`,
]);
if (unicodeDataTestParent !== null) {
    expect(
        unicodeDataTestParent.trim() === UNICODE_DATA_BASE_COMMIT,
        "Unicode data test commit is not based on its recorded base",
    );
}
const unicodeGrammarBaseParent = gitOptional([
    "rev-parse",
    `${UNICODE_GRAMMAR_BASE_COMMIT}^`,
]);
if (unicodeGrammarBaseParent !== null) {
    expect(
        unicodeGrammarBaseParent.trim() ===
            UNICODE_GRAMMAR_BASE_PARENT_COMMIT,
        "Unicode grammar base commit has an unexpected parent",
    );
}
const unicodeGrammarTestParent = gitOptional([
    "rev-parse",
    `${UNICODE_GRAMMAR_TEST_COMMIT}^`,
]);
if (unicodeGrammarTestParent !== null) {
    expect(
        unicodeGrammarTestParent.trim() ===
            UNICODE_GRAMMAR_BASE_COMMIT,
        "Unicode grammar test commit is not based on its recorded base",
    );
}
const unicodeGrammarImplementationParent = gitOptional([
    "rev-parse",
    `${UNICODE_GRAMMAR_IMPLEMENTATION_COMMIT}^`,
]);
if (unicodeGrammarImplementationParent !== null) {
    expect(
        unicodeGrammarImplementationParent.trim() ===
            UNICODE_GRAMMAR_TEST_COMMIT,
        "Unicode grammar implementation commit is not based on its locked tests",
    );
}
const tokenAssignmentBaseParent = gitOptional([
    "rev-parse",
    `${TOKEN_ASSIGNMENT_BASE_COMMIT}^`,
]);
if (tokenAssignmentBaseParent !== null) {
    expect(
        tokenAssignmentBaseParent.trim() ===
            TOKEN_ASSIGNMENT_BASE_PARENT_COMMIT,
        "token assignment base commit has an unexpected parent",
    );
}
const tokenAssignmentFixtureParent = gitOptional([
    "rev-parse",
    `${TOKEN_ASSIGNMENT_FIXTURE_COMMIT}^`,
]);
if (tokenAssignmentFixtureParent !== null) {
    expect(
        tokenAssignmentFixtureParent.trim() ===
            TOKEN_ASSIGNMENT_BASE_COMMIT,
        "token assignment fixture commit is not based on its recorded base",
    );
}
const tokenAssignmentTestParent = gitOptional([
    "rev-parse",
    `${TOKEN_ASSIGNMENT_TEST_COMMIT}^`,
]);
if (tokenAssignmentTestParent !== null) {
    expect(
        tokenAssignmentTestParent.trim() ===
            TOKEN_ASSIGNMENT_FIXTURE_COMMIT,
        "token assignment test commit is not based on its fixture port",
    );
}
const tokenAssignmentImplementationParent = gitOptional([
    "rev-parse",
    `${TOKEN_ASSIGNMENT_IMPLEMENTATION_COMMIT}^`,
]);
if (tokenAssignmentImplementationParent !== null) {
    expect(
        tokenAssignmentImplementationParent.trim() ===
            TOKEN_ASSIGNMENT_TEST_COMMIT,
        "token assignment implementation commit is not based on its locked tests",
    );
}
const leftRecursionBaseParent = gitOptional([
    "rev-parse",
    `${LEFT_RECURSION_BASE_COMMIT}^`,
]);
if (leftRecursionBaseParent !== null) {
    expect(
        leftRecursionBaseParent.trim() ===
            LEFT_RECURSION_BASE_PARENT_COMMIT,
        "left recursion base commit has an unexpected parent",
    );
}
const leftRecursionFixtureParent = gitOptional([
    "rev-parse",
    `${LEFT_RECURSION_FIXTURE_COMMIT}^`,
]);
if (leftRecursionFixtureParent !== null) {
    expect(
        leftRecursionFixtureParent.trim() === LEFT_RECURSION_BASE_COMMIT,
        "left recursion fixture commit is not based on its recorded base",
    );
}
const leftRecursionTestParent = gitOptional([
    "rev-parse",
    `${LEFT_RECURSION_TEST_COMMIT}^`,
]);
if (leftRecursionTestParent !== null) {
    expect(
        leftRecursionTestParent.trim() === LEFT_RECURSION_FIXTURE_COMMIT,
        "left recursion test commit is not based on its fixture port",
    );
}
const leftRecursionImplementationParent = gitOptional([
    "rev-parse",
    `${LEFT_RECURSION_IMPLEMENTATION_COMMIT}^`,
]);
if (leftRecursionImplementationParent !== null) {
    expect(
        leftRecursionImplementationParent.trim() ===
            LEFT_RECURSION_TEST_COMMIT,
        "left recursion implementation commit is not based on its locked tests",
    );
}
const graphNodesBaseParent = gitOptional([
    "rev-parse",
    `${GRAPH_NODES_BASE_COMMIT}^`,
]);
if (graphNodesBaseParent !== null) {
    expect(
        graphNodesBaseParent.trim() === GRAPH_NODES_BASE_PARENT_COMMIT,
        "GraphNodes base commit has an unexpected parent",
    );
}
const graphNodesTestParent = gitOptional([
    "rev-parse",
    `${GRAPH_NODES_TEST_COMMIT}^`,
]);
if (graphNodesTestParent !== null) {
    expect(
        graphNodesTestParent.trim() === GRAPH_NODES_BASE_COMMIT,
        "GraphNodes test commit is not based on its recorded base",
    );
}
const graphNodesImplementationParent = gitOptional([
    "rev-parse",
    `${GRAPH_NODES_IMPLEMENTATION_COMMIT}^`,
]);
if (graphNodesImplementationParent !== null) {
    expect(
        graphNodesImplementationParent.trim() ===
            GRAPH_NODES_TEST_COMMIT,
        "GraphNodes implementation commit is not based on its locked tests",
    );
}
const symbolIssuesBaseParent = gitOptional([
    "rev-parse",
    `${SYMBOL_ISSUES_BASE_COMMIT}^`,
]);
if (symbolIssuesBaseParent !== null) {
    expect(
        symbolIssuesBaseParent.trim() ===
            SYMBOL_ISSUES_BASE_PARENT_COMMIT,
        "symbol issues base commit has an unexpected parent",
    );
}
const symbolIssuesTestParent = gitOptional([
    "rev-parse",
    `${SYMBOL_ISSUES_TEST_COMMIT}^`,
]);
if (symbolIssuesTestParent !== null) {
    expect(
        symbolIssuesTestParent.trim() === SYMBOL_ISSUES_BASE_COMMIT,
        "symbol issues test commit is not based on its recorded base",
    );
}
const symbolIssuesImplementationParent = gitOptional([
    "rev-parse",
    `${SYMBOL_ISSUES_IMPLEMENTATION_COMMIT}^`,
]);
if (symbolIssuesImplementationParent !== null) {
    expect(
        symbolIssuesImplementationParent.trim() ===
            SYMBOL_ISSUES_TEST_COMMIT,
        "symbol issues implementation commit is not based on its locked tests",
    );
}
const attributeChecksBaseParent = gitOptional([
    "rev-parse",
    `${ATTRIBUTE_CHECKS_BASE_COMMIT}^`,
]);
if (attributeChecksBaseParent !== null) {
    expect(
        attributeChecksBaseParent.trim() ===
            ATTRIBUTE_CHECKS_BASE_PARENT_COMMIT,
        "attribute checks base commit has an unexpected parent",
    );
}
const attributeChecksTestParent = gitOptional([
    "rev-parse",
    `${ATTRIBUTE_CHECKS_TEST_COMMIT}^`,
]);
if (attributeChecksTestParent !== null) {
    expect(
        attributeChecksTestParent.trim() ===
            ATTRIBUTE_CHECKS_BASE_COMMIT,
        "attribute checks test commit is not based on its recorded base",
    );
}
const attributeChecksImplementationParent = gitOptional([
    "rev-parse",
    `${ATTRIBUTE_CHECKS_IMPLEMENTATION_COMMIT}^`,
]);
if (attributeChecksImplementationParent !== null) {
    expect(
        attributeChecksImplementationParent.trim() ===
            ATTRIBUTE_CHECKS_TEST_COMMIT,
        "attribute checks implementation commit is not based on its locked tests",
    );
}
const toolSyntaxErrorsBaseParent = gitOptional([
    "rev-parse",
    `${TOOL_SYNTAX_ERRORS_BASE_COMMIT}^`,
]);
if (toolSyntaxErrorsBaseParent !== null) {
    expect(
        toolSyntaxErrorsBaseParent.trim() ===
            TOOL_SYNTAX_ERRORS_BASE_PARENT_COMMIT,
        "tool syntax base commit has an unexpected parent",
    );
}
const toolSyntaxErrorsTestParent = gitOptional([
    "rev-parse",
    `${TOOL_SYNTAX_ERRORS_TEST_COMMIT}^`,
]);
if (toolSyntaxErrorsTestParent !== null) {
    expect(
        toolSyntaxErrorsTestParent.trim() ===
            TOOL_SYNTAX_ERRORS_BASE_COMMIT,
        "tool syntax test commit is not based on its recorded base",
    );
}
const toolSyntaxErrorsImplementationParent = gitOptional([
    "rev-parse",
    `${TOOL_SYNTAX_ERRORS_IMPLEMENTATION_COMMIT}^`,
]);
if (toolSyntaxErrorsImplementationParent !== null) {
    expect(
        toolSyntaxErrorsImplementationParent.trim() ===
            TOOL_SYNTAX_ERRORS_TEST_COMMIT,
        "tool syntax implementation commit is not based on its locked tests",
    );
}
const compositeGrammarsBaseParent = gitOptional([
    "rev-parse",
    `${COMPOSITE_GRAMMARS_BASE_COMMIT}^`,
]);
if (compositeGrammarsBaseParent !== null) {
    expect(
        compositeGrammarsBaseParent.trim() ===
            COMPOSITE_GRAMMARS_BASE_PARENT_COMMIT,
        "composite grammar base commit has an unexpected parent",
    );
}
const compositeGrammarsTestParent = gitOptional([
    "rev-parse",
    `${COMPOSITE_GRAMMARS_TEST_COMMIT}^`,
]);
if (compositeGrammarsTestParent !== null) {
    expect(
        compositeGrammarsTestParent.trim() ===
            COMPOSITE_GRAMMARS_BASE_COMMIT,
        "composite grammar test commit is not based on its recorded base",
    );
}
const initialCompositeGrammarsImplementationParent = gitOptional([
    "rev-parse",
    `${COMPOSITE_GRAMMARS_IMPLEMENTATION_PARENT_COMMIT}^`,
]);
if (initialCompositeGrammarsImplementationParent !== null) {
    expect(
        initialCompositeGrammarsImplementationParent.trim() ===
            COMPOSITE_GRAMMARS_TEST_COMMIT,
        "initial composite grammar implementation is not based on its locked tests",
    );
}
const compositeGrammarsImplementationParent = gitOptional([
    "rev-parse",
    `${COMPOSITE_GRAMMARS_IMPLEMENTATION_COMMIT}^`,
]);
if (compositeGrammarsImplementationParent !== null) {
    expect(
        compositeGrammarsImplementationParent.trim() ===
            COMPOSITE_GRAMMARS_IMPLEMENTATION_PARENT_COMMIT,
        "final composite grammar implementation has an unexpected parent",
    );
}

expect(
    differences.java_antlr_commit === JAVA_COMMIT,
    "approved-differences Java pin differs",
);
expect(
    differences.antlr_ng_commit === ANTLR_NG_COMMIT,
    "approved-differences antlr-ng pin differs",
);
expect(
    Array.isArray(differences.differences) && differences.differences.length === 0,
    "active phases have unreviewed or unexpected approved differences",
);

if (failures.length > 0) {
    for (const failure of failures) {
        console.error(failure);
    }
    process.exitCode = 1;
} else {
    console.log(
        `port evidence valid: ${records.size} active ledgers, ${globalRevisionIds.size} revisions`,
    );
}

async function load(path) {
    return JSON.parse(await readFile(resolve(repoRoot, path), "utf8"));
}

async function validateAllowedInputs(logicalId, manifest) {
    const expectedKeys = [
        ...(manifest.closure.source_case_ids ?? []).map((id) => `case:${id}`),
        ...(manifest.closure.source_id
            ? [`source:${manifest.closure.source_id}`]
            : []),
        ...(manifest.closure.fixture_paths ?? []).map((path) => `path:${path}`),
    ].sort();
    const actualKeys = [];
    const inputs = Array.isArray(manifest.allowed_inputs)
        ? manifest.allowed_inputs
        : [];
    expect(
        Array.isArray(manifest.allowed_inputs),
        `${logicalId} allowed inputs must be an array`,
    );

    for (const input of inputs) {
        const hasSourceCase = typeof input.source_case_id === "string";
        const hasExternalSource = typeof input.source_id === "string";
        const isFixture = !hasSourceCase && !hasExternalSource;
        const identityCount =
            Number(hasSourceCase) + Number(hasExternalSource) + Number(isFixture);
        expect(
            identityCount === 1 &&
                typeof input.path === "string" &&
                typeof input.sha256 === "string",
            `${logicalId} has a malformed allowed input`,
        );

        if (hasSourceCase) {
            const sourceCase = sourceCases.get(input.source_case_id);
            expect(
                Boolean(sourceCase) &&
                    input.path === sourceCase?.source.path &&
                    input.sha256 === sourceCase?.source.sha256,
                `${logicalId} source-case input differs for ${input.source_case_id}`,
            );
            actualKeys.push(`case:${input.source_case_id}`);
        } else if (hasExternalSource) {
            const source = externalSources.get(input.source_id);
            expect(
                Boolean(source) &&
                    input.path === source?.mirror_path &&
                    input.sha256 === source?.sha256,
                `${logicalId} external input differs for ${input.source_id}`,
            );
            if (
                source &&
                input.path === source.mirror_path &&
                input.sha256 === source.sha256
            ) {
                await expectLocalHash(
                    logicalId,
                    source.mirror_path,
                    source.sha256,
                );
            }
            actualKeys.push(`source:${input.source_id}`);
        } else {
            const declared = (manifest.closure.fixture_paths ?? []).includes(
                input.path,
            );
            expect(
                declared,
                `${logicalId} names undeclared fixture input ${input.path}`,
            );
            if (
                declared &&
                typeof input.path === "string" &&
                typeof input.sha256 === "string"
            ) {
                await expectLocalHash(logicalId, input.path, input.sha256);
            }
            actualKeys.push(`path:${input.path}`);
        }
    }

    actualKeys.sort();
    expect(
        JSON.stringify(actualKeys) === JSON.stringify(expectedKeys),
        `${logicalId} allowed inputs do not exactly match its closure`,
    );
}

async function expectLocalHash(logicalId, path, expected) {
    try {
        const contents = await readFile(resolve(repoRoot, path));
        expect(
            digest(contents) === expected,
            `${logicalId} allowed input hash differs for ${path}`,
        );
    } catch (error) {
        failures.push(
            `${logicalId} cannot read allowed input ${path}: ${error.message}`,
        );
    }
}

function gitOptional(args) {
    const result = spawnSync("git", args, {
        cwd: repoRoot,
        encoding: "utf8",
        maxBuffer: 32 * 1024 * 1024,
    });
    if (result.status !== 0) {
        return null;
    }
    return result.stdout;
}

function sectionAtMarker(text, marker) {
    const offset = text.indexOf(marker);
    if (offset < 0) {
        throw new Error(`cannot find locked section marker ${marker}`);
    }
    return text.slice(offset);
}

function lockedSection(text, section) {
    if (!section.end_marker) {
        return sectionAtMarker(text, section.marker);
    }
    const offset = text.indexOf(section.marker);
    if (offset < 0) {
        throw new Error(`cannot find locked section marker ${section.marker}`);
    }
    let end = text.indexOf(section.end_marker, offset);
    if (end < 0 && section.historical_end_marker) {
        end = text.indexOf(section.historical_end_marker, offset);
    }
    if (end < 0) {
        throw new Error(
            `cannot find locked section end marker ${section.end_marker}`,
        );
    }
    return text.slice(offset, end);
}

function expect(condition, message) {
    if (!condition) {
        failures.push(message);
    }
}
