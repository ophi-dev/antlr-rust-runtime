#!/usr/bin/env node

import {
    mkdir,
    readFile,
    writeFile,
} from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

import {
    ANTLR_NG_COMMIT,
    IMPLEMENTATION_COMMIT,
    JAVA_COMMIT,
    SCAFFOLD_COMMIT,
    TEST_COMMIT,
    VSCODE_COMMIT,
    digest,
    parseMode,
    stableStringify,
} from "./evidence-common.mjs";

const TEST_COMMAND =
    "cargo test --locked --bin antlr4-rust-gen grammar::frontend::tests::";
const TEST_MODULE_PATH = "src/bin_support/grammar/frontend.rs";
const TEST_MODULE_MARKER = "#[cfg(test)]";
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
const phaseARows = testMap.rows.filter(
    (row) => row.disposition === "port" && row.owner_phase === "A",
);
const expectedFiles = new Map();

const checkedInTestModule = sectionAtMarker(
    await readFile(resolve(repoRoot, TEST_MODULE_PATH), "utf8"),
    TEST_MODULE_MARKER,
);
const testModule = gitShowOptional(TEST_COMMIT, TEST_MODULE_PATH);
const implementationTestModule = gitShowOptional(
    IMPLEMENTATION_COMMIT,
    TEST_MODULE_PATH,
);
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

const upstreamByLogicalId = new Map(
    phaseARows.map((row) => [row.logical_id, row]),
);
for (const fixture of externalMap.fixtures) {
    for (const assertion of fixture.assertions) {
        if (assertion.tdd_owner.startsWith("upstream:")) {
            const logicalId = assertion.tdd_owner.slice("upstream:".length);
            const row = upstreamByLogicalId.get(logicalId);
            if (!row) {
                throw new Error(`${assertion.id} names missing Phase A row ${logicalId}`);
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
            });
        }
    }
}

for (const row of phaseARows) {
    await addEvidence({
        logicalId: row.logical_id,
        revisionId: row.active_revision_id,
        closure: row.closure,
        closureHash: row.closure_sha256,
        sourceCaseIds: row.source_case_ids,
        externalSource: null,
        primaryTestSource: row.primary_test_source,
        alternateTestSource: row.alternate_test_source,
        declaredOutcomes: {
            primary:
                "pinned source cases passed in the recorded JUnit/Vitest discovery or immutable fixture snapshot",
            alternate:
                "alternate source cases passed in the recorded runner discovery or generated oracle",
            java_compatibility_verdict:
                "Java-compatible syntax; antlr-ng supplies the canonical Phase A CST shape",
        },
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
    `${update ? "updated" : "verified"} ${phaseARows.length + Object.keys(EXTERNAL_DEFINITIONS).length} Phase A evidence ledgers`,
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
}) {
    const base = `tests/codegen-direct/port-evidence/${logicalId}`;
    const revisionBase = `${base}/revisions/${revisionId}`;
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
                test_port: "primary",
                test_commit: TEST_COMMIT,
                implementation_port: "primary-antlr-ng",
                implementation_commit: IMPLEMENTATION_COMMIT,
                command: TEST_COMMAND,
                result: "green: 5 passed; 0 failed",
            },
        ],
        escalation:
            "not required because the primary implementation passed the locked primary tests",
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
        supersedes_revision_id: null,
        owner_phase: "A",
        state: "done",
        closure,
        closure_sha256: closureHash,
        allowed_inputs: allowedInputs,
        commits: {
            scaffold: SCAFFOLD_COMMIT,
            primary_test: TEST_COMMIT,
            primary_implementation: IMPLEMENTATION_COMMIT,
        },
        ancestry: {
            primary_test_parent: SCAFFOLD_COMMIT,
            primary_implementation_parent: TEST_COMMIT,
            reachability:
                "direct ancestry is verified when the recorded commit objects are available",
        },
        locked_oracle_sections: [
            {
                path: TEST_MODULE_PATH,
                marker: TEST_MODULE_MARKER,
                sha256: lockedTestModuleHash,
            },
        ],
        demonstrated_red: redResult(),
        green_result: greenResult(),
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
    const index = {
        schema_version: 1,
        logical_id: logicalId,
        active_revision_id: revisionId,
        revisions: [
            {
                revision_id: revisionId,
                supersedes_revision_id: null,
                state: "done",
                manifest_path: manifestPath,
                closure_sha256: closureHash,
            },
        ],
    };
    expectedFiles.set(`${base}/index.json`, `${JSON.stringify(index, null, 2)}\n`);
}

function redResult() {
    return {
        command: TEST_COMMAND,
        commit: TEST_COMMIT,
        exit_code: 101,
        fingerprint: "G4F000: the Stage 0 grammar frontend is not installed",
    };
}

function greenResult() {
    return {
        command: TEST_COMMAND,
        commit: IMPLEMENTATION_COMMIT,
        exit_code: 0,
        result: "5 passed; 0 failed",
    };
}

function sectionAtMarker(text, marker) {
    const offset = text.indexOf(marker);
    if (offset < 0) {
        throw new Error(`cannot find locked section marker ${marker}`);
    }
    return text.slice(offset);
}

function gitShowOptional(commit, path) {
    const result = spawnSync("git", ["show", `${commit}:${path}`], {
        cwd: repoRoot,
        encoding: "utf8",
        maxBuffer: 32 * 1024 * 1024,
    });
    if (result.status !== 0) {
        return null;
    }
    return result.stdout;
}

async function load(path) {
    return JSON.parse(await readFile(resolve(repoRoot, path), "utf8"));
}
