#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPOSITORY = "https://github.com/mike-lischke/vscode-antlr4.git";
const COMMIT = "3e9469d1d490c71b3e3b909edf1235582a3f8db8";
const PHASES = new Set(["A", "B", "C"]);
const OUTCOMES = new Set(["valid", "semantic-error", "dependency-error"]);

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const inventory = await load("tests/codegen-direct/external-source-inventory.json");
const fixtureMap = await load("tests/codegen-direct/external-fixture-map.json");
const failures = [];

expect(inventory.schema_version === 1, "inventory schema_version must be 1");
expect(fixtureMap.schema_version === 1, "fixture-map schema_version must be 1");
expect(inventory.repository === REPOSITORY, "inventory repository pin differs");
expect(fixtureMap.repository === REPOSITORY, "fixture-map repository pin differs");
expect(inventory.commit === COMMIT, "inventory commit pin differs");
expect(fixtureMap.commit === COMMIT, "fixture-map commit pin differs");
expect(
    Array.isArray(inventory.artifacts) && inventory.artifacts.length === 13,
    "inventory must contain exactly 13 artifacts",
);
expect(
    Array.isArray(fixtureMap.fixtures) && fixtureMap.fixtures.length === 12,
    "fixture map must contain exactly 12 grammar rows",
);

const inventoryById = uniqueMap(
    inventory.artifacts,
    "source_id",
    "inventory source ID",
);
const consumedSourceIds = [];
const license = fixtureMap.repository_license;
expect(
    license?.source_id === "vscode-antlr4:License.txt",
    "repository license must own vscode-antlr4:License.txt",
);
consumedSourceIds.push(license?.source_id);

for (const artifact of inventory.artifacts ?? []) {
    const mirrorPath = resolve(repoRoot, artifact.mirror_path);
    let contents;
    try {
        contents = await readFile(mirrorPath);
    } catch (error) {
        failures.push(`cannot read mirror ${artifact.mirror_path}: ${error.message}`);
        continue;
    }
    expect(
        digest(contents) === artifact.sha256,
        `mirror hash differs for ${artifact.source_id}`,
    );
}

const fixtureIds = new Set();
const assertionIds = new Set();
for (const fixture of fixtureMap.fixtures ?? []) {
    expect(
        typeof fixture.id === "string" && !fixtureIds.has(fixture.id),
        `duplicate or missing fixture ID: ${fixture.id}`,
    );
    fixtureIds.add(fixture.id);
    consumedSourceIds.push(fixture.source_id);

    const source = inventoryById.get(fixture.source_id);
    expect(Boolean(source), `unknown fixture source ID: ${fixture.source_id}`);
    expect(
        source?.path.endsWith(".g4"),
        `fixture source is not a grammar: ${fixture.source_id}`,
    );
    expect(
        source?.mirror_path === fixture.mirror_path,
        `mirror path differs for ${fixture.id}`,
    );
    expect(
        fixture.licenses?.includes("vscode-antlr4:repository-license"),
        `${fixture.id} does not name the repository license`,
    );
    expect(
        PHASES.has(fixture.owner_phase),
        `${fixture.id} has invalid owner phase ${fixture.owner_phase}`,
    );
    expect(
        OUTCOMES.has(fixture.compiler_outcome),
        `${fixture.id} has invalid compiler outcome ${fixture.compiler_outcome}`,
    );
    expect(
        fixture.phase_contracts?.A === "token-and-cst-snapshot",
        `${fixture.id} lacks the Phase A syntax contract`,
    );
    expect(
        Array.isArray(fixture.assertions) && fixture.assertions.length > 0,
        `${fixture.id} has no assertions`,
    );

    for (const assertion of fixture.assertions ?? []) {
        expect(
            typeof assertion.id === "string" && !assertionIds.has(assertion.id),
            `duplicate or missing external assertion ID: ${assertion.id}`,
        );
        assertionIds.add(assertion.id);
        expect(
            typeof assertion.tdd_owner === "string" &&
                /^(upstream|external):[^:][\w:./-]*$/u.test(assertion.tdd_owner),
            `${assertion.id} has invalid tdd_owner`,
        );
        expect(
            PHASES.has(assertion.phase),
            `${assertion.id} has invalid phase ${assertion.phase}`,
        );
        expect(
            typeof assertion.rust_test === "string" &&
                assertion.rust_test.length > 0,
            `${assertion.id} lacks a Rust test or planned fixture`,
        );
        if (assertion.tdd_owner.startsWith("external:")) {
            expect(
                assertion.tdd_owner === `external:${assertion.id}`,
                `${assertion.id} external owner is not self-identifying`,
            );
            expect(
                typeof assertion.active_revision_id === "string",
                `${assertion.id} lacks active_revision_id`,
            );
        }
    }
}

const expectedIds = [...inventoryById.keys()].sort();
const actualIds = consumedSourceIds.filter(Boolean).sort();
expect(
    JSON.stringify(actualIds) === JSON.stringify(expectedIds),
    "fixture rows plus repository license do not exactly partition the inventory",
);

if (failures.length > 0) {
    for (const failure of failures) {
        console.error(failure);
    }
    process.exitCode = 1;
} else {
    console.log(
        `external fixture map valid: ${actualIds.length} artifacts, ${assertionIds.size} assertions`,
    );
}

async function load(path) {
    return JSON.parse(await readFile(resolve(repoRoot, path), "utf8"));
}

function uniqueMap(entries, field, label) {
    const result = new Map();
    for (const entry of entries ?? []) {
        const key = entry[field];
        expect(
            typeof key === "string" && !result.has(key),
            `duplicate or missing ${label}: ${key}`,
        );
        result.set(key, entry);
    }
    return result;
}

function digest(contents) {
    return createHash("sha256").update(contents).digest("hex");
}

function expect(condition, message) {
    if (!condition) {
        failures.push(message);
    }
}
