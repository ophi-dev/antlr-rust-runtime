#!/usr/bin/env node

import { readFile, readdir } from "node:fs/promises";
import { dirname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { digest } from "./evidence-common.mjs";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const fixturesRoot = resolve(
    repositoryRoot,
    "tests/codegen-direct/fixtures",
);
const failures = [];
let artifactCount = 0;
let manifestCount = 0;

const fixtureEntries = await readdir(fixturesRoot, { withFileTypes: true });
fixtureEntries.sort((left, right) => left.name.localeCompare(right.name));
for (const entry of fixtureEntries) {
    if (!entry.isDirectory()) {
        continue;
    }
    const fixtureDirectory = resolve(fixturesRoot, entry.name);
    const manifestPath = resolve(fixtureDirectory, "fixture.json");
    let manifest;
    try {
        manifest = JSON.parse(await readFile(manifestPath, "utf8"));
    } catch (error) {
        if (error.code === "ENOENT") {
            continue;
        }
        failures.push(`${entry.name}: cannot read fixture.json: ${error.message}`);
        continue;
    }
    manifestCount += 1;

    expect(
        manifest.schema_version === 1,
        `${entry.name}: fixture schema_version must be 1`,
    );
    expect(
        isRecord(manifest.files),
        `${entry.name}: fixture files must be an object`,
    );
    for (const [path, expectedHash] of Object.entries(manifest.files ?? {})) {
        const artifactPath = containedPath(fixtureDirectory, path);
        if (artifactPath === null) {
            failures.push(`${entry.name}: artifact path escapes fixture: ${path}`);
            continue;
        }
        await validateHash(
            artifactPath,
            expectedHash,
            `${entry.name}/${path}`,
        );
    }

    const unicodeData = manifest.java_antlr?.unicode_data;
    if (unicodeData !== undefined) {
        expect(
            typeof unicodeData.helper === "string",
            `${entry.name}: Unicode helper path is missing`,
        );
        expect(
            typeof unicodeData.helper_sha256 === "string",
            `${entry.name}: Unicode helper hash is missing`,
        );
        if (
            typeof unicodeData.helper === "string"
            && typeof unicodeData.helper_sha256 === "string"
        ) {
            const helperPath = containedPath(
                repositoryRoot,
                unicodeData.helper,
            );
            if (helperPath === null) {
                failures.push(
                    `${entry.name}: Unicode helper escapes repository: ${unicodeData.helper}`,
                );
            } else {
                await validateHash(
                    helperPath,
                    unicodeData.helper_sha256,
                    `${entry.name} Unicode helper`,
                );
            }
        }
    }
}

if (failures.length > 0) {
    for (const failure of failures) {
        console.error(failure);
    }
    process.exitCode = 1;
} else {
    console.log(
        `interp fixtures valid: ${manifestCount} manifests, ${artifactCount} hashed artifacts`,
    );
}

async function validateHash(path, expectedHash, label) {
    if (typeof expectedHash !== "string") {
        failures.push(`${label}: SHA-256 must be a string`);
        return;
    }
    try {
        const actualHash = digest(await readFile(path));
        artifactCount += 1;
        expect(actualHash === expectedHash, `${label}: SHA-256 differs`);
    } catch (error) {
        failures.push(`${label}: cannot read artifact: ${error.message}`);
    }
}

function containedPath(root, path) {
    const candidate = resolve(root, path);
    return candidate === root || candidate.startsWith(`${root}${sep}`)
        ? candidate
        : null;
}

function isRecord(value) {
    return value !== null && typeof value === "object" && !Array.isArray(value);
}

function expect(condition, message) {
    if (!condition) {
        failures.push(message);
    }
}
