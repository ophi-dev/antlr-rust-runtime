#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
    copyFile,
    mkdir,
    readFile,
    writeFile,
} from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PINNED_ANTLR_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const PINNED_ANTLR_JAR_SHA256 =
    "eae2dfa119a64327444672aff63e9ec35a20180dc5b8090b7a6ab85125df4d76";
const LEGACY_GENERATOR_COMMIT =
    "481fa1e5933e67d1ec43755ef7ddd004714b6add";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const manifestPath = resolve(
    repoRoot,
    "third_party/antlr-v4-grammar/stage0-manifest.json",
);

const options = parseArguments(process.argv.slice(2));
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
const failures = [];

expectEqual(manifest.schema_version, 1, "manifest schema_version");
expectEqual(manifest.stage, "stage0", "manifest stage");
expectEqual(
    manifest.antlr_tool?.commit,
    PINNED_ANTLR_COMMIT,
    "pinned ANTLR commit",
);
expectEqual(
    manifest.antlr_tool?.jar_sha256,
    PINNED_ANTLR_JAR_SHA256,
    "pinned ANTLR jar hash",
);
expectEqual(
    manifest.legacy_generator?.commit,
    LEGACY_GENERATOR_COMMIT,
    "legacy generator commit",
);
expectEqual(
    await sha256(options.antlrJar),
    PINNED_ANTLR_JAR_SHA256,
    "provided ANTLR jar hash",
);

if (options.update) {
    for (const input of manifest.inputs) {
        input.sha256 = await sha256(resolve(repoRoot, input.path));
    }
    for (const intermediate of manifest.intermediates) {
        intermediate.sha256 = await sha256(
            resolve(options.workDir, intermediate.seed_path),
        );
    }
    for (const output of manifest.outputs) {
        const candidate = resolve(options.workDir, output.seed_path);
        const destination = resolve(repoRoot, output.checked_in_path);
        await mkdir(dirname(destination), { recursive: true });
        await copyFile(candidate, destination);
        output.sha256 = await sha256(candidate);
    }
    manifest.seed_java_runtime = (
        await readFile(resolve(options.workDir, "java-version.txt"), "utf8")
    )
        .trim()
        .split(/\r?\n/u);
    await writeFile(
        manifestPath,
        `${JSON.stringify(manifest, null, 2)}\n`,
        "utf8",
    );
}

for (const input of manifest.inputs) {
    expectEqual(
        await sha256(resolve(repoRoot, input.path)),
        input.sha256,
        `input hash ${input.path}`,
    );
}
for (const intermediate of manifest.intermediates) {
    expectEqual(
        await sha256(resolve(options.workDir, intermediate.seed_path)),
        intermediate.sha256,
        `intermediate hash ${intermediate.seed_path}`,
    );
}
for (const output of manifest.outputs) {
    const candidateHash = await sha256(
        resolve(options.workDir, output.seed_path),
    );
    expectEqual(candidateHash, output.sha256, `generated hash ${output.seed_path}`);
    expectEqual(
        await sha256(resolve(repoRoot, output.checked_in_path)),
        output.sha256,
        `checked-in hash ${output.checked_in_path}`,
    );
}

if (failures.length > 0) {
    for (const failure of failures) {
        console.error(failure);
    }
    process.exitCode = 1;
} else {
    console.log(
        options.update
            ? "updated Stage 0 generated files and manifest"
            : "Stage 0 generated files reproduce exactly",
    );
}

function parseArguments(args) {
    let update = false;
    let workDir;
    let antlrJar;
    for (let index = 0; index < args.length; index += 1) {
        switch (args[index]) {
            case "--check":
                update = false;
                break;
            case "--update":
                update = true;
                break;
            case "--work-dir":
                workDir = args[++index];
                break;
            case "--antlr-jar":
                antlrJar = args[++index];
                break;
            default:
                throw new Error(`unknown argument: ${args[index]}`);
        }
    }
    if (!workDir || !antlrJar) {
        throw new Error("--work-dir and --antlr-jar are required");
    }
    return {
        update,
        workDir: resolve(workDir),
        antlrJar: resolve(antlrJar),
    };
}

async function sha256(path) {
    const contents = await readFile(path);
    return createHash("sha256").update(contents).digest("hex");
}

function expectEqual(actual, expected, label) {
    if (actual !== expected) {
        failures.push(`${label}: expected ${expected}, got ${actual}`);
    }
}
