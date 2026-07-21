#!/usr/bin/env node

import { readdir, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

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
    FRONTEND_SYNTAX_TEST_PARENT,
    IMPLEMENTATION_COMMIT,
    JAVA_COMMIT,
    PHASE_B_BASE_COMMIT,
    PHASE_B_IMPLEMENTATION_COMMIT,
    SCAFFOLD_COMMIT,
    TEST_COMMIT,
    digest,
    gitShowOptional,
    stableStringify,
} from "./evidence-common.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
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
    const end = text.indexOf(section.end_marker, offset);
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
