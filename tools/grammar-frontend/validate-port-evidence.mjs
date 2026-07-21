#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const TEST_COMMIT = "a4258562c44818e2ba97d206587c64d4c38408d0";
const IMPLEMENTATION_COMMIT = "8a00a3d6496779b969a42511d7e29c0d102d62d7";
const SCAFFOLD_COMMIT = "75615945749dc93fca5d929cb22ad481f12dfdc9";
const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const ANTLR_NG_COMMIT = "1f68422ae4bfc62f93343769e144d01f305487b1";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const evidenceRoot = resolve(
    repoRoot,
    "tests/codegen-direct/port-evidence",
);
const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const externalMap = await load("tests/codegen-direct/external-fixture-map.json");
const differences = await load(
    "tests/codegen-direct/approved-differences.json",
);
const failures = [];
const records = new Map();

for (const row of testMap.rows ?? []) {
    if (row.disposition === "port" && row.owner_phase === "A") {
        records.set(row.logical_id, {
            revisionId: row.active_revision_id,
            closure: row.closure,
            closureHash: row.closure_sha256,
            evidencePath: row.evidence_path,
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
    "port-evidence directories do not exactly match active Phase A records",
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
            manifest.closure_sha256 === digest(stableStringify(manifest.closure)),
            `${logicalId} manifest closure hash is invalid`,
        );
        for (const evidenceFile of manifest.evidence_files ?? []) {
            const contents = await readFile(resolve(repoRoot, evidenceFile.path));
            expect(
                digest(contents) === evidenceFile.sha256,
                `${logicalId} evidence hash differs for ${evidenceFile.path}`,
            );
        }
        for (const section of manifest.locked_oracle_sections ?? []) {
            const locked = sectionAtMarker(
                gitShow(manifest.commits.primary_test, section.path),
                section.marker,
            );
            const afterImplementation = sectionAtMarker(
                gitShow(manifest.commits.primary_implementation, section.path),
                section.marker,
            );
            expect(
                digest(locked) === section.sha256,
                `${logicalId} locked oracle section hash differs`,
            );
            expect(
                locked === afterImplementation,
                `${logicalId} implementation commit edited its locked oracle section`,
            );
        }
        expect(
            manifest.commits.scaffold === SCAFFOLD_COMMIT &&
                manifest.commits.primary_test === TEST_COMMIT &&
                manifest.commits.primary_implementation === IMPLEMENTATION_COMMIT,
            `${logicalId} evidence commit identities differ`,
        );
        expect(
            manifest.ancestry.primary_test_parent === SCAFFOLD_COMMIT &&
                manifest.ancestry.primary_implementation_parent === TEST_COMMIT,
            `${logicalId} recorded ancestry differs`,
        );
        expect(
            manifest.demonstrated_red?.exit_code !== 0 &&
                manifest.green_result?.exit_code === 0,
            `${logicalId} lacks red/green execution evidence`,
        );
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
}

expect(
    git(["rev-parse", `${TEST_COMMIT}^`]).trim() === SCAFFOLD_COMMIT,
    "primary test commit is not directly based on the scaffold",
);
expect(
    git(["rev-parse", `${IMPLEMENTATION_COMMIT}^`]).trim() === TEST_COMMIT,
    "primary implementation commit is not directly based on the locked test",
);
for (const commit of [SCAFFOLD_COMMIT, TEST_COMMIT, IMPLEMENTATION_COMMIT]) {
    const result = spawnSync("git", ["merge-base", "--is-ancestor", commit, "HEAD"], {
        cwd: repoRoot,
    });
    expect(result.status === 0, `${commit} is not reachable from the Phase A branch`);
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
    "Phase A has unreviewed or unexpected approved differences",
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

function gitShow(commit, path) {
    return git(["show", `${commit}:${path}`]);
}

function git(args) {
    const result = spawnSync("git", args, {
        cwd: repoRoot,
        encoding: "utf8",
        maxBuffer: 32 * 1024 * 1024,
    });
    if (result.status !== 0) {
        throw new Error(`git ${args.join(" ")} failed: ${result.stderr}`);
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

function expect(condition, message) {
    if (!condition) {
        failures.push(message);
    }
}
