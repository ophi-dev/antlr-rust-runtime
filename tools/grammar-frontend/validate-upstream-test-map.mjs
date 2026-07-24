#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
    ANTLR_NG_COMMIT,
    JAVA_COMMIT,
    digest,
    stableStringify,
} from "./evidence-common.mjs";
const PHASES = new Set(["A", "B", "C", "existing"]);
const DISPOSITIONS = new Set([
    "port",
    "consult",
    "covered-existing",
    "out-of-scope",
]);
const TDD_STATES = new Set([
    "mapped",
    "primary-test-ported",
    "primary-test-locked-red",
    "primary-implementation-ported",
    "green",
    "done",
    "blocked",
]);
const RESOLUTIONS = new Set(["ported", "verified-covered-existing"]);

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const inventory = await load("tests/codegen-direct/upstream-case-inventory.json");
const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const externalMap = await load("tests/codegen-direct/external-fixture-map.json");
const failures = [];

expect(inventory.schema_version === 1, "inventory schema_version must be 1");
expect(testMap.schema_version === 1, "test-map schema_version must be 1");
expect(
    testMap.pins?.java_antlr === JAVA_COMMIT,
    "test-map Java pin differs",
);
expect(
    testMap.pins?.antlr_ng === ANTLR_NG_COMMIT,
    "test-map antlr-ng pin differs",
);
expect(
    testMap.source_inventory_case_count === inventory.case_count,
    "test-map source count differs from inventory",
);
expect(
    testMap.active_row_count === testMap.rows?.length,
    "test-map active row count is stale",
);

const inventoryIds = uniqueValues(
    inventory.cases?.map((testCase) => testCase.id),
    "inventory source-case ID",
);
const logicalIds = new Set();
const revisionIds = new Set();
const consumedSourceIds = [];
const upstreamExternalLinks = new Map();

for (const row of testMap.rows ?? []) {
    expect(
        typeof row.logical_id === "string" && !logicalIds.has(row.logical_id),
        `duplicate or missing logical ID: ${row.logical_id}`,
    );
    logicalIds.add(row.logical_id);
    expect(
        Array.isArray(row.source_case_ids),
        `${row.logical_id} source_case_ids must be an array`,
    );
    expect(
        Array.isArray(row.external_assertion_ids),
        `${row.logical_id} external_assertion_ids must be an array`,
    );
    expect(
        PHASES.has(row.owner_phase),
        `${row.logical_id} has invalid owner phase ${row.owner_phase}`,
    );
    expect(
        DISPOSITIONS.has(row.disposition),
        `${row.logical_id} has invalid disposition ${row.disposition}`,
    );
    for (const sourceCaseId of row.source_case_ids ?? []) {
        expect(
            inventoryIds.has(sourceCaseId),
            `${row.logical_id} references unknown source case ${sourceCaseId}`,
        );
        consumedSourceIds.push(sourceCaseId);
    }
    for (const assertionId of row.external_assertion_ids ?? []) {
        expect(
            !upstreamExternalLinks.has(assertionId),
            `external assertion ${assertionId} is linked by multiple upstream rows`,
        );
        upstreamExternalLinks.set(assertionId, row.logical_id);
    }

    if (row.disposition === "port") {
        const resolution = row.resolution ?? "ported";
        expect(
            typeof row.active_revision_id === "string" &&
                !revisionIds.has(row.active_revision_id),
            `${row.logical_id} has duplicate or missing active_revision_id`,
        );
        revisionIds.add(row.active_revision_id);
        expect(
            TDD_STATES.has(row.tdd_state),
            `${row.logical_id} has invalid TDD state ${row.tdd_state}`,
        );
        expect(
            RESOLUTIONS.has(resolution),
            `${row.logical_id} has invalid resolution ${resolution}`,
        );
        expect(
            (row.closure?.resolution ?? "ported") === resolution,
            `${row.logical_id} closure resolution differs`,
        );
        expect(
            row.closure?.logical_id === row.logical_id,
            `${row.logical_id} closure logical ID differs`,
        );
        expect(
            row.closure_sha256 === digest(stableStringify(row.closure)),
            `${row.logical_id} closure hash differs`,
        );
        expect(
            JSON.stringify(row.closure?.source_case_ids) ===
                JSON.stringify(row.source_case_ids),
            `${row.logical_id} closure source cases differ`,
        );
        expect(
            JSON.stringify(row.closure?.external_assertion_ids) ===
                JSON.stringify(row.external_assertion_ids),
            `${row.logical_id} closure external assertions differ`,
        );
        validateDeclaredSource(row, row.primary_test_source, "primary");
        validateDeclaredSource(row, row.alternate_test_source, "alternate");
        expect(
            row.primary_implementation_source === `antlr-ng@${ANTLR_NG_COMMIT}`,
            `${row.logical_id} primary implementation source differs`,
        );
        expect(
            row.alternate_implementation_source === `java-antlr@${JAVA_COMMIT}`,
            `${row.logical_id} alternate implementation source differs`,
        );
        expect(
            typeof row.unit_under_test === "string" &&
                row.unit_under_test.length > 0,
            `${row.logical_id} lacks unit_under_test`,
        );
        expect(
            typeof row.observable_equivalence === "string" &&
                row.observable_equivalence.length > 0,
            `${row.logical_id} lacks observable equivalence`,
        );
        if (row.owner_phase === "A") {
            expect(
                row.tdd_state === "done",
                `${row.logical_id} Phase A port is not done`,
            );
        } else if (["B", "C"].includes(row.owner_phase)) {
            expect(
                ["mapped", "done"].includes(row.tdd_state),
                `${row.logical_id} Phase ${row.owner_phase} port has invalid progress state`,
            );
        } else {
            expect(
                row.tdd_state === "mapped",
                `${row.logical_id} non-active port has advanced progress`,
            );
        }
        if (row.tdd_state === "done") {
            expect(
                typeof row.primary_test_commit === "string" &&
                    typeof row.primary_implementation_commit === "string",
                `${row.logical_id} lacks locked test or implementation commit`,
            );
            if (resolution === "verified-covered-existing") {
                expect(
                    row.verified_covered_existing?.exit_code === 0 &&
                        row.verified_covered_existing?.result?.includes("passed") &&
                        row.verified_covered_existing?.commit ===
                            row.primary_test_commit,
                    `${row.logical_id} lacks covered-existing evidence`,
                );
            } else {
                expect(
                    row.demonstrated_red?.exit_code !== 0 &&
                        row.demonstrated_red?.fingerprint,
                    `${row.logical_id} lacks demonstrated red evidence`,
                );
            }
            expect(
                row.green_result?.result?.includes("passed"),
                `${row.logical_id} lacks green evidence`,
            );
            expect(
                typeof row.evidence_path === "string",
                `${row.logical_id} lacks durable evidence path`,
            );
        } else {
            expect(
                row.tdd_state === "mapped",
                `${row.logical_id} incomplete port is not mapped`,
            );
        }
    } else {
        expect(
            row.active_revision_id === null,
            `${row.logical_id} non-port row has active_revision_id`,
        );
        expect(
            typeof row.rationale === "string" && row.rationale.length > 0,
            `${row.logical_id} non-port row lacks case-specific rationale`,
        );
        expect(
            typeof row.covering_evidence === "string" &&
                row.covering_evidence.length > 0,
            `${row.logical_id} non-port row lacks evidence`,
        );
        expect(
            typeof row.approving_reviewer === "string" &&
                row.approving_reviewer.length > 0,
            `${row.logical_id} non-port row lacks reviewer`,
        );
    }
}

const expectedSourceIds = [...inventoryIds].sort();
const actualSourceIds = consumedSourceIds.sort();
expect(
    JSON.stringify(actualSourceIds) === JSON.stringify(expectedSourceIds),
    "active test-map rows do not exactly partition upstream source-case IDs",
);

const externalAssertions = new Map();
for (const fixture of externalMap.fixtures ?? []) {
    for (const assertion of fixture.assertions ?? []) {
        expect(
            !externalAssertions.has(assertion.id),
            `duplicate external assertion ID ${assertion.id}`,
        );
        externalAssertions.set(assertion.id, assertion);
    }
}
for (const [assertionId, logicalId] of upstreamExternalLinks) {
    const assertion = externalAssertions.get(assertionId);
    expect(Boolean(assertion), `${logicalId} links unknown external assertion ${assertionId}`);
    expect(
        assertion?.tdd_owner === `upstream:${logicalId}`,
        `${assertionId} does not link back to upstream:${logicalId}`,
    );
}
for (const assertion of externalAssertions.values()) {
    if (assertion.tdd_owner.startsWith("upstream:")) {
        const logicalId = assertion.tdd_owner.slice("upstream:".length);
        expect(
            upstreamExternalLinks.get(assertion.id) === logicalId,
            `${assertion.id} has a dangling or one-way upstream owner`,
        );
    }
}

if (failures.length > 0) {
    for (const failure of failures) {
        console.error(failure);
    }
    process.exitCode = 1;
} else {
    console.log(
        `upstream test map valid: ${testMap.rows.length} rows partition ${actualSourceIds.length} source cases`,
    );
}

function validateDeclaredSource(row, source, label) {
    expect(
        source && typeof source.implementation === "string",
        `${row.logical_id} lacks ${label} test source`,
    );
    expect(
        Array.isArray(source?.source_case_ids),
        `${row.logical_id} ${label} source IDs must be an array`,
    );
    for (const sourceCaseId of source?.source_case_ids ?? []) {
        expect(
            row.source_case_ids.includes(sourceCaseId),
            `${row.logical_id} ${label} source references a case outside its row`,
        );
    }
    if (source?.implementation === "java-antlr") {
        expect(
            source.commit === JAVA_COMMIT,
            `${row.logical_id} ${label} Java pin differs`,
        );
    } else if (source?.implementation === "antlr-ng") {
        expect(
            source.commit === ANTLR_NG_COMMIT,
            `${row.logical_id} ${label} antlr-ng pin differs`,
        );
    } else {
        expect(
            source?.implementation === "independent-generated-oracle" &&
                typeof source.reason === "string",
            `${row.logical_id} ${label} source override is not documented`,
        );
    }
}

async function load(path) {
    return JSON.parse(await readFile(resolve(repoRoot, path), "utf8"));
}

function uniqueValues(values, label) {
    const result = new Set();
    for (const value of values ?? []) {
        expect(
            typeof value === "string" && !result.has(value),
            `duplicate or missing ${label}: ${value}`,
        );
        result.add(value);
    }
    return result;
}

function expect(condition, message) {
    if (!condition) {
        failures.push(message);
    }
}
