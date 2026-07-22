#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
    access,
    copyFile,
    mkdir,
    mkdtemp,
    readFile,
    readdir,
    rm,
    writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import {
    delimiter,
    dirname,
    join,
    relative,
    resolve,
    sep,
} from "node:path";
import { fileURLToPath } from "node:url";

const ANTLR_VERSION = "4.13.2";
const ANTLR_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const ANTLR_JAR_SHA256 =
    "eae2dfa119a64327444672aff63e9ec35a20180dc5b8090b7a6ab85125df4d76";
const ICU4J_VERSION = "78.1";
const ICU4J_JAR_SHA256 =
    "bbb70d3be23110d7295823eee0c2e896ac3b619b3c0f26168f65eb972df51d2a";
const UNICODE_VERSION = "17.0";
const EXPECTED_JAVA = {
    vendor: "Homebrew",
    runtime: "26.0.1",
    vm: "26.0.1",
};
const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const fixturesRoot = resolve(repositoryRoot, "tests/codegen-direct/fixtures");
const unicodeGenerator = resolve(
    scriptDirectory,
    "oracle/GenerateUnicodeData.java",
);
const options = parseArguments(process.argv.slice(2));
const fixtureDirectory = resolve(fixturesRoot, options.fixture);
if (
    fixtureDirectory !== fixturesRoot
    && !fixtureDirectory.startsWith(`${fixturesRoot}${sep}`)
) {
    throw new Error(`fixture escapes fixture root: ${options.fixture}`);
}

const manifestPath = join(fixtureDirectory, "fixture.json");
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
if (!Array.isArray(manifest.roots) || manifest.roots.length === 0) {
    throw new Error("fixture.json must contain a non-empty roots array");
}
const libraryDirectories = manifest.library_directories ?? [];
if (!Array.isArray(libraryDirectories) || libraryDirectories.length > 1) {
    throw new Error("fixture library_directories must contain at most one path");
}

const jarHash = await digestFile(options.antlrJar);
if (jarHash !== ANTLR_JAR_SHA256) {
    throw new Error(
        `ANTLR jar SHA-256 mismatch: expected ${ANTLR_JAR_SHA256}, found ${jarHash}`,
    );
}
const icuJarHash = await digestFile(options.icuJar);
if (icuJarHash !== ICU4J_JAR_SHA256) {
    throw new Error(
        `ICU4J jar SHA-256 mismatch: expected ${ICU4J_JAR_SHA256}, found ${icuJarHash}`,
    );
}
const javaMetadata = await readJavaMetadata(options.java);
for (const [key, expected] of Object.entries(EXPECTED_JAVA)) {
    if (javaMetadata[key] !== expected) {
        throw new Error(
            `expected Java ${key} ${expected}, found ${javaMetadata[key] ?? "<missing>"}`,
        );
    }
}

const scratch = await mkdtemp(join(tmpdir(), "antlr-rust-fixture-"));
const outputDirectory = join(scratch, "generated");
await mkdir(outputDirectory);
try {
    const unicodeOverlay = await buildUnicodeOverlay(scratch);
    const commandArguments = [
        "-cp",
        [unicodeOverlay.classDirectory, options.antlrJar].join(delimiter),
        "org.antlr.v4.Tool",
        "-o",
        outputDirectory,
    ];
    if (libraryDirectories.length === 1) {
        commandArguments.push(
            "-lib",
            resolve(fixtureDirectory, libraryDirectories[0]),
        );
    }
    commandArguments.push(...manifest.roots);
    const result = spawnSync(options.java, commandArguments, {
        cwd: fixtureDirectory,
        encoding: "utf8",
        maxBuffer: 64 * 1024 * 1024,
    });
    if (result.error) {
        throw result.error;
    }
    const exitStatus = result.status ?? 1;
    verifyExpectedOutcome(manifest.expected ?? "success", exitStatus);

    const generatedFiles = (await filesRecursively(outputDirectory)).filter(
        (path) => path.endsWith(".interp") || path.endsWith(".tokens"),
    );
    for (const source of generatedFiles) {
        const destination = join(
            fixtureDirectory,
            relative(outputDirectory, source),
        );
        await mkdir(dirname(destination), { recursive: true });
        await copyFile(source, destination);
    }

    const oracleDirectory = join(fixtureDirectory, "oracle");
    await mkdir(oracleDirectory, { recursive: true });
    await writeFile(
        join(oracleDirectory, "java-antlr.stdout"),
        result.stdout,
        "utf8",
    );
    await writeFile(
        join(oracleDirectory, "java-antlr.stderr"),
        result.stderr,
        "utf8",
    );
    await writeFile(
        join(oracleDirectory, "java-unicode-data.stdout"),
        unicodeOverlay.stdout,
        "utf8",
    );
    await writeFile(
        join(oracleDirectory, "java-unicode-data.stderr"),
        unicodeOverlay.stderr,
        "utf8",
    );
    if (options.fixture === "lexer-unicode") {
        await copyFile(
            unicodeOverlay.propertyOracle,
            join(oracleDirectory, "java-unicode-properties.tsv"),
        );
    }
    await updateDiagnostics(fixtureDirectory, result.stdout, result.stderr);

    manifest.java_antlr = {
        ...(manifest.java_antlr ?? {}),
        version: ANTLR_VERSION,
        commit: ANTLR_COMMIT,
        jar_sha256: ANTLR_JAR_SHA256,
        jdk: `${javaMetadata.vendor} OpenJDK ${javaMetadata.runtime}`,
        java_runtime_version: javaMetadata.runtime,
        java_vm_version: javaMetadata.vm,
        unicode_data: {
            generator:
                "org.antlr.v4.unicode.UnicodeDataTemplateController",
            helper:
                "tools/grammar-frontend/oracle/GenerateUnicodeData.java",
            helper_sha256: await digestFile(unicodeGenerator),
            icu4j_version: unicodeOverlay.metadata.icu4j_version,
            icu4j_jar_sha256: ICU4J_JAR_SHA256,
            unicode_version: unicodeOverlay.metadata.unicode_version,
        },
        exit_status: exitStatus,
    };
    manifest.regeneration_command =
        `tools/grammar-frontend/update-interp-fixtures.sh ${options.fixture} ` +
        "--antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar " +
        "--icu-jar /tmp/antlr-cleanroom/tools/icu4j-78.1.jar";
    manifest.files = await fixtureHashes(fixtureDirectory);
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

    console.log(
        `updated ${options.fixture}: ${generatedFiles.length} generated file(s), ` +
            `${Object.keys(manifest.files).length} hashed artifact(s), Java exit ${exitStatus}`,
    );
} finally {
    await rm(scratch, { force: true, recursive: true });
}

function parseArguments(args) {
    const result = {
        antlrJar:
            process.env.ANTLR4_JAR
            ?? "/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar",
        fixture: null,
        icuJar:
            process.env.ICU4J_JAR
            ?? "/tmp/antlr-cleanroom/tools/icu4j-78.1.jar",
        java: process.env.JAVA ?? "java",
    };
    for (let index = 0; index < args.length; index++) {
        const argument = args[index];
        switch (argument) {
            case "--antlr-jar":
                result.antlrJar = resolve(requiredValue(args, ++index, argument));
                break;
            case "--java":
                result.java = requiredValue(args, ++index, argument);
                break;
            case "--icu-jar":
                result.icuJar = resolve(requiredValue(args, ++index, argument));
                break;
            default:
                if (argument.startsWith("-")) {
                    throw new Error(`unknown argument: ${argument}`);
                }
                if (result.fixture !== null) {
                    throw new Error("only one fixture name may be supplied");
                }
                result.fixture = argument;
        }
    }
    if (result.fixture === null) {
        throw new Error(
            "usage: update-interp-fixtures.sh FIXTURE " +
                "[--antlr-jar PATH] [--icu-jar PATH] [--java PATH]",
        );
    }
    return result;
}

async function buildUnicodeOverlay(scratch) {
    const sourceDirectory = join(scratch, "unicode-source");
    const classDirectory = join(scratch, "unicode-overlay");
    const propertyOracle = join(scratch, "unicode-properties.tsv");
    await mkdir(sourceDirectory);
    await mkdir(classDirectory);
    const result = spawnSync(
        options.java,
        [
            "--class-path",
            [options.antlrJar, options.icuJar].join(delimiter),
            unicodeGenerator,
            sourceDirectory,
            classDirectory,
            propertyOracle,
        ],
        {
            encoding: "utf8",
            maxBuffer: 64 * 1024 * 1024,
        },
    );
    if (result.error) {
        throw result.error;
    }
    if (result.status !== 0) {
        throw new Error(
            `Unicode data overlay failed (${result.status}): ${result.stderr}`,
        );
    }

    const metadata = parseTabMetadata(result.stdout);
    verifyToolVersion(
        "ICU4J",
        metadata.icu4j_version,
        ICU4J_VERSION,
    );
    verifyToolVersion(
        "Unicode",
        metadata.unicode_version,
        UNICODE_VERSION,
    );
    await access(
        join(
            classDirectory,
            "org/antlr/v4/unicode/UnicodeData.class",
        ),
    );
    return {
        classDirectory,
        metadata,
        propertyOracle,
        stderr: result.stderr,
        stdout: result.stdout,
    };
}

function parseTabMetadata(output) {
    const entries = output
        .trimEnd()
        .split("\n")
        .filter(Boolean)
        .map((line) => {
            const separator = line.indexOf("\t");
            if (separator < 1) {
                throw new Error(`unexpected Unicode helper output: ${line}`);
            }
            return [line.slice(0, separator), line.slice(separator + 1)];
        });
    return Object.fromEntries(entries);
}

function verifyToolVersion(tool, actual, expected) {
    if (
        actual === undefined
        || actual.split(".").slice(0, 2).join(".") !== expected
    ) {
        throw new Error(
            `expected ${tool} ${expected}, found ${actual ?? "<missing>"}`,
        );
    }
}

function requiredValue(args, index, option) {
    const value = args[index];
    if (value === undefined) {
        throw new Error(`${option} requires a value`);
    }
    return value;
}

function verifyExpectedOutcome(expected, status) {
    if (expected === "error" ? status === 0 : status !== 0) {
        throw new Error(
            `Java ANTLR exit ${status} does not match expected fixture outcome ${expected}`,
        );
    }
}

async function readJavaMetadata(javaExecutable) {
    const directory = await mkdtemp(join(tmpdir(), "antlr-rust-java-"));
    const sourcePath = join(directory, "PrintJavaMetadata.java");
    const source = `public class PrintJavaMetadata {
    public static void main(String[] args) {
        System.out.println("vendor\\t" + System.getProperty("java.vendor"));
        System.out.println("runtime\\t" + System.getProperty("java.runtime.version"));
        System.out.println("vm\\t" + System.getProperty("java.vm.version"));
    }
}
`;
    try {
        await writeFile(sourcePath, source, "utf8");
        const result = spawnSync(javaExecutable, [sourcePath], {
            encoding: "utf8",
        });
        if (result.error) {
            throw result.error;
        }
        if (result.status !== 0) {
            throw new Error(
                `Java metadata helper failed (${result.status}): ${result.stderr}`,
            );
        }
        return Object.fromEntries(
            result.stdout
                .trimEnd()
                .split("\n")
                .map((line) => line.split("\t")),
        );
    } finally {
        await rm(directory, { force: true, recursive: true });
    }
}

async function updateDiagnostics(directory, stdout, stderr) {
    const diagnostics = [];
    const pattern =
        /^(?<severity>warning|error)\((?<code>\d+)\): (?<detail>.*)$/gmu;
    for (const match of `${stdout}\n${stderr}`.matchAll(pattern)) {
        const diagnostic = {
            severity: match.groups.severity,
            code: Number.parseInt(match.groups.code, 10),
        };
        const position =
            /:(?<line>\d+):(?<column>\d+):/u.exec(match.groups.detail);
        if (position !== null) {
            diagnostic.line = Number.parseInt(position.groups.line, 10);
            diagnostic.column = Number.parseInt(position.groups.column, 10);
            diagnostic.message = match.groups.detail.slice(
                position.index + position[0].length,
            ).trimStart();
        } else {
            diagnostic.message = match.groups.detail;
        }
        diagnostics.push(diagnostic);
    }
    const path = join(directory, "diagnostics.json");
    let existing = {};
    try {
        existing = JSON.parse(await readFile(path, "utf8"));
    } catch (error) {
        if (error.code !== "ENOENT") {
            throw error;
        }
    }
    if (diagnostics.length === 0 && Object.keys(existing).length === 0) {
        return;
    }
    existing.java_antlr = diagnostics;
    await writeFile(path, `${JSON.stringify(existing, null, 2)}\n`, "utf8");
}

async function fixtureHashes(directory) {
    const hashes = {};
    for (const path of await filesRecursively(directory)) {
        const relativePath = relative(directory, path).split(sep).join("/");
        if (
            relativePath === "fixture.json"
            || (!relativePath.endsWith(".g4")
                && !relativePath.endsWith(".interp")
                && !relativePath.endsWith(".tokens")
                && relativePath !== "diagnostics.json"
                && !relativePath.startsWith("oracle/"))
        ) {
            continue;
        }
        hashes[relativePath] = await digestFile(path);
    }
    return Object.fromEntries(
        Object.entries(hashes).sort(([left], [right]) =>
            left < right ? -1 : left > right ? 1 : 0,
        ),
    );
}

async function filesRecursively(directory) {
    const result = [];
    const entries = await readdir(directory, { withFileTypes: true });
    for (const entry of entries) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) {
            result.push(...(await filesRecursively(path)));
        } else if (entry.isFile()) {
            result.push(path);
        }
    }
    result.sort();
    return result;
}

async function digestFile(path) {
    return createHash("sha256")
        .update(await readFile(path))
        .digest("hex");
}
