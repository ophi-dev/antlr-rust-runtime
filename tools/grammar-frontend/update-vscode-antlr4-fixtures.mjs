#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
    mkdir,
    readFile,
    writeFile,
} from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const REPOSITORY = "https://github.com/mike-lischke/vscode-antlr4.git";
const COMMIT = "3e9469d1d490c71b3e3b909edf1235582a3f8db8";
const REQUIRED_LICENSE = "License.txt";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const inventoryPath = resolve(
    repoRoot,
    "tests/codegen-direct/external-source-inventory.json",
);
const mirrorRoot = resolve(
    repoRoot,
    "tests/codegen-direct/external/vscode-antlr4",
);
const options = parseArguments(process.argv.slice(2));

git(options.source, ["cat-file", "-e", `${COMMIT}^{commit}`]);
const entries = parseTree(
    git(options.source, ["ls-tree", "-rz", "--full-tree", COMMIT]),
).filter(
    (entry) =>
        entry.path === REQUIRED_LICENSE || entry.path.endsWith(".g4"),
);

const grammarCount = entries.filter((entry) => entry.path.endsWith(".g4")).length;
if (
    entries.length !== 13 ||
    grammarCount !== 12 ||
    !entries.some((entry) => entry.path === REQUIRED_LICENSE)
) {
    throw new Error(
        `expected License.txt plus 12 .g4 files, found ${entries.length} artifacts and ${grammarCount} grammars`,
    );
}

const artifacts = [];
for (const entry of entries) {
    const contents = git(options.source, ["show", `${COMMIT}:${entry.path}`]);
    const artifact = {
        source_id: `vscode-antlr4:${entry.path}`,
        path: entry.path,
        mode: entry.mode,
        git_blob: entry.object,
        sha256: digest(contents),
        mirror_path: `tests/codegen-direct/external/vscode-antlr4/${entry.path}`,
    };
    artifacts.push(artifact);

    const mirrorPath = resolve(mirrorRoot, entry.path);
    if (options.update) {
        await mkdir(dirname(mirrorPath), { recursive: true });
        await writeFile(mirrorPath, contents);
    } else {
        const mirror = await readFile(mirrorPath);
        if (!mirror.equals(contents)) {
            throw new Error(`checked-in mirror differs from pinned blob: ${entry.path}`);
        }
    }
}

const inventory = {
    schema_version: 1,
    repository: REPOSITORY,
    commit: COMMIT,
    required_artifact_query:
        "git ls-tree -r <commit>: License.txt plus every tracked *.g4",
    generated_by:
        "tools/grammar-frontend/update-vscode-antlr4-fixtures.mjs",
    artifacts,
};
const serialized = `${JSON.stringify(inventory, null, 2)}\n`;

if (options.update) {
    await writeFile(inventoryPath, serialized, "utf8");
    console.log(`updated ${artifacts.length} pinned vscode-antlr4 artifacts`);
} else {
    const checkedIn = await readFile(inventoryPath, "utf8");
    if (checkedIn !== serialized) {
        throw new Error(
            "external-source-inventory.json does not match the pinned Git tree",
        );
    }
    console.log(`verified ${artifacts.length} pinned vscode-antlr4 artifacts`);
}

function parseArguments(args) {
    let update = false;
    let source;
    for (let index = 0; index < args.length; index += 1) {
        switch (args[index]) {
            case "--check":
                update = false;
                break;
            case "--update":
                update = true;
                break;
            case "--source":
                source = args[++index];
                break;
            case "-h":
            case "--help":
                console.log(
                    "Usage: update-vscode-antlr4-fixtures.mjs [--check|--update] --source CHECKOUT",
                );
                process.exit(0);
                break;
            default:
                throw new Error(`unknown argument: ${args[index]}`);
        }
    }
    if (!source) {
        throw new Error("--source is required");
    }
    return { update, source: resolve(source) };
}

function git(cwd, args) {
    const result = spawnSync("git", ["-C", cwd, ...args], {
        encoding: null,
        maxBuffer: 32 * 1024 * 1024,
    });
    if (result.status !== 0) {
        throw new Error(
            `git ${args.join(" ")} failed: ${result.stderr.toString("utf8")}`,
        );
    }
    return result.stdout;
}

function parseTree(buffer) {
    return buffer
        .toString("utf8")
        .split("\0")
        .filter(Boolean)
        .map((record) => {
            const match =
                /^(?<mode>\d+) (?<type>\w+) (?<object>[0-9a-f]+)\t(?<path>.+)$/u.exec(
                    record,
                );
            if (!match?.groups || match.groups.type !== "blob") {
                throw new Error(`unexpected git ls-tree record: ${record}`);
            }
            return match.groups;
        });
}

function digest(contents) {
    return createHash("sha256").update(contents).digest("hex");
}
