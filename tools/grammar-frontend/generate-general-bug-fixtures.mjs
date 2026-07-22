#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
    mkdir,
    mkdtemp,
    readFile,
    rm,
    writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const ANTLR_NG_COMMIT = "1f68422ae4bfc62f93343769e144d01f305487b1";
const ANTLR_JAR_SHA256 =
    "eae2dfa119a64327444672aff63e9ec35a20180dc5b8090b7a6ab85125df4d76";
const JAVA_DOT_PATH = "tool/src/org/antlr/v4/tool/DOTGenerator.java";
const GENERAL_PATH = "tests/bugs/General.spec.ts";
const JAVA_REPOSITORY = "https://github.com/antlr/antlr4.git";
const ANTLR_NG_REPOSITORY = "https://github.com/mike-lischke/antlr-ng.git";
const CASES = new Map([
    [
        "Bug #33 Escaping issues with backslash in .dot file comparison",
        {
            grammarPath: "tests/bugs/data/abbLexer.g4",
            rule: "EscapeSequence",
            javaAtnOutcome: "success",
        },
    ],
    [
        "Bug #35 Tool crashes with --atn",
        {
            grammarPath: "tests/bugs/data/GoLexer.g4",
            rule: "EOS",
            javaAtnOutcome: "error",
            javaFailure:
                "Not a valid Unicode code point: 0xFFFFFFFF",
        },
    ],
]);

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const fixturesRoot = resolve(repositoryRoot, "tests/codegen-direct/fixtures");
const options = parseArguments(process.argv.slice(2));

verifyCommit(options.javaRoot, JAVA_COMMIT, "Java ANTLR");
verifyCommit(options.antlrNgRoot, ANTLR_NG_COMMIT, "antlr-ng");
if (await digestFile(options.antlrJar) !== ANTLR_JAR_SHA256) {
    throw new Error("ANTLR jar SHA-256 differs from the pinned 4.13.2 artifact");
}

const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const inventory = await load("tests/codegen-direct/upstream-case-inventory.json");
const inventoryById = new Map(
    inventory.cases.map((testCase) => [testCase.id, testCase]),
);
const rows = testMap.rows
    .filter((row) =>
        row.logical_id.startsWith("general-bug-33-")
        || row.logical_id.startsWith("general-bug-35-")
    )
    .sort((left, right) => left.logical_id.localeCompare(right.logical_id));
if (rows.length !== CASES.size) {
    throw new Error(`expected ${CASES.size} General bug rows, found ${rows.length}`);
}

const generalSource = gitText(
    options.antlrNgRoot,
    `${ANTLR_NG_COMMIT}:${GENERAL_PATH}`,
);
const sourceCases = extractVitestCases(generalSource);
const javaDotSource = gitText(
    options.javaRoot,
    `${JAVA_COMMIT}:${JAVA_DOT_PATH}`,
);

for (const row of rows) {
    const [sourceCaseId] = row.source_case_ids;
    const sourceCase = inventoryById.get(sourceCaseId);
    if (
        row.source_case_ids.length !== 1
        || sourceCase?.implementation !== "antlr-ng"
        || sourceCase.suite !== "General"
    ) {
        throw new Error(
            `${row.logical_id} must reference one pinned antlr-ng General case`,
        );
    }
    const config = CASES.get(sourceCase.name);
    const method = sourceCases.get(sourceCase.name);
    if (!config || !method) {
        throw new Error(`${row.logical_id} has no supported General case definition`);
    }
    verifyMethodBinding(method, config);

    const grammar = gitText(
        options.antlrNgRoot,
        `${ANTLR_NG_COMMIT}:${config.grammarPath}`,
    );
    const grammarName = grammarDeclaration(grammar);
    const expectedEdge = extractExpectedEdge(method);
    const oracles = await generateDotOracles({
        grammar,
        grammarName,
        rule: config.rule,
        expectedEdge,
        javaAtnOutcome: config.javaAtnOutcome,
        javaFailure: config.javaFailure,
    });
    const definition = createDefinition({
        row,
        sourceCase,
        config,
        grammar,
        grammarName,
        expectedEdge,
        oracles,
        javaDotSource,
    });
    if (options.update) {
        await updateFixture(row.logical_id, definition);
    } else {
        await checkFixture(row.logical_id, definition);
    }
}

console.log(
    `${options.update ? "updated" : "verified"} ${rows.length} General bug fixtures`,
);

function createDefinition({
    row,
    sourceCase,
    config,
    grammar,
    grammarName,
    expectedEdge,
    oracles,
    javaDotSource,
}) {
    const root = `${grammarName}.g4`;
    const antlrNgDotPath =
        `oracle/antlr-ng-${grammarName}.${config.rule}.dot`;
    const javaDotPath =
        config.javaAtnOutcome === "success"
            ? `oracle/java-${grammarName}.${config.rule}.dot`
            : null;
    const files = new Map([
        [root, grammar],
        ["oracle/expected-dot-edge.txt", `${expectedEdge}\n`],
        [antlrNgDotPath, oracles.antlrNg.dot],
        ["oracle/antlr-ng-atn.stdout", oracles.antlrNg.stdout],
        ["oracle/antlr-ng-atn.stderr", oracles.antlrNg.stderr],
        ["oracle/java-atn.stdout", oracles.java.stdout],
        ["oracle/java-atn.stderr", oracles.java.stderr],
    ]);
    if (javaDotPath !== null) {
        files.set(javaDotPath, oracles.java.dot);
    }
    const upstreamTest = {
        source_case_id: sourceCase.id,
        repository: ANTLR_NG_REPOSITORY,
        commit: ANTLR_NG_COMMIT,
        path: sourceCase.source.path,
        case: sourceCase.name,
        source_sha256: sourceCase.source.sha256,
    };
    return {
        files,
        manifest: {
            schema_version: 1,
            roots: [root],
            logical_ids: [row.logical_id],
            upstream_tests: [upstreamTest],
            java_antlr_test: {
                repository: JAVA_REPOSITORY,
                commit: JAVA_COMMIT,
                path: JAVA_DOT_PATH,
                case: "independent generated ATN and DOT compatibility oracle",
                source_sha256: digest(javaDotSource),
                role:
                    "Java 4.13.2 supplies .interp and .tokens; its DOT generator outcome is retained explicitly",
            },
            antlr_ng_test: upstreamTest,
            grammar_source: {
                repository: ANTLR_NG_REPOSITORY,
                commit: ANTLR_NG_COMMIT,
                path: config.grammarPath,
                sha256: digest(grammar),
            },
            expected: "success",
            general_atn_dot_oracle: {
                rule: config.rule,
                expected_edge: "oracle/expected-dot-edge.txt",
                antlr_ng_dot: antlrNgDotPath,
                java_dot: javaDotPath,
                java_atn_outcome: config.javaAtnOutcome,
                java_failure_fingerprint: config.javaFailure ?? null,
                agreement:
                    config.javaAtnOutcome === "success"
                        ? "Java 4.13.2 and antlr-ng contain the same selected DOT edge"
                        : "Java 4.13.2 --atn crashes on EOF after ATN construction; antlr-ng supplies the no-crash DOT edge",
                compatibility_verdict:
                    "Java ANTLR 4.13.2 .interp and .tokens, with the pinned antlr-ng DOT regression assertion",
            },
        },
    };
}

async function generateDotOracles({
    grammar,
    grammarName,
    rule,
    expectedEdge,
    javaAtnOutcome,
    javaFailure,
}) {
    const scratch = await mkdtemp(resolve(tmpdir(), "antlr-general-bug-"));
    const grammarPath = resolve(scratch, `${grammarName}.g4`);
    const antlrNgOutput = resolve(scratch, "antlr-ng");
    const javaOutput = resolve(scratch, "java");
    await mkdir(antlrNgOutput);
    await mkdir(javaOutput);
    await writeFile(grammarPath, grammar, "utf8");
    try {
        const antlrNg = spawnSync(
            resolve(options.antlrNgRoot, "node_modules/.bin/tsx"),
            [
                resolve(options.antlrNgRoot, "cli/runner.ts"),
                "--atn",
                "true",
                "-o",
                antlrNgOutput,
                grammarPath,
            ],
            {
                cwd: options.antlrNgRoot,
                encoding: "utf8",
                maxBuffer: 64 * 1024 * 1024,
            },
        );
        ensureSpawned(antlrNg, "antlr-ng --atn");
        if (antlrNg.status !== 0) {
            throw new Error(
                `antlr-ng --atn failed (${antlrNg.status}): ${antlrNg.stderr}`,
            );
        }
        const antlrNgDot = await readFile(
            resolve(antlrNgOutput, `${grammarName}.${rule}.dot`),
            "utf8",
        );
        requireEdge(antlrNgDot, expectedEdge, "antlr-ng");

        const java = spawnSync(
            options.java,
            [
                "-jar",
                options.antlrJar,
                "-atn",
                "-o",
                javaOutput,
                grammarPath,
            ],
            {
                encoding: "utf8",
                maxBuffer: 64 * 1024 * 1024,
            },
        );
        ensureSpawned(java, "Java ANTLR --atn");
        const javaSucceeded = java.status === 0;
        if ((javaAtnOutcome === "success") !== javaSucceeded) {
            throw new Error(
                `Java --atn exit ${java.status} does not match ${javaAtnOutcome}: ${java.stderr}`,
            );
        }
        let javaDot = null;
        if (javaSucceeded) {
            javaDot = await readFile(
                resolve(javaOutput, `${grammarName}.${rule}.dot`),
                "utf8",
            );
            requireEdge(javaDot, expectedEdge, "Java");
        } else if (!java.stderr.includes(javaFailure)) {
            throw new Error(
                `Java --atn failure does not contain ${javaFailure}: ${java.stderr}`,
            );
        }
        return {
            antlrNg: {
                dot: antlrNgDot,
                stdout: antlrNg.stdout,
                stderr: antlrNg.stderr,
            },
            java: {
                dot: javaDot,
                stdout: java.stdout,
                stderr: java.stderr,
            },
        };
    } finally {
        await rm(scratch, { force: true, recursive: true });
    }
}

async function updateFixture(logicalId, definition) {
    const directory = resolve(fixturesRoot, logicalId);
    await rm(directory, { force: true, recursive: true });
    await mkdir(directory, { recursive: true });
    for (const [path, contents] of definition.files) {
        await mkdir(dirname(resolve(directory, path)), { recursive: true });
        await writeFile(resolve(directory, path), contents, "utf8");
    }
    await writeFile(
        resolve(directory, "fixture.json"),
        `${JSON.stringify(definition.manifest, null, 2)}\n`,
        "utf8",
    );

    const updater = resolve(scriptDirectory, "update-interp-fixtures.mjs");
    const result = spawnSync(
        process.execPath,
        [
            updater,
            logicalId,
            "--antlr-jar",
            options.antlrJar,
            "--icu-jar",
            options.icuJar,
            "--java",
            options.java,
        ],
        {
            cwd: repositoryRoot,
            encoding: "utf8",
            maxBuffer: 64 * 1024 * 1024,
        },
    );
    ensureSpawned(result, "fixture updater");
    if (result.status !== 0) {
        throw new Error(
            `fixture updater failed for ${logicalId} (${result.status}):\n` +
                `${result.stdout}\n${result.stderr}`,
        );
    }
    process.stdout.write(result.stdout);

    const manifestPath = resolve(directory, "fixture.json");
    const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
    manifest.regeneration_command =
        "node tools/grammar-frontend/generate-general-bug-fixtures.mjs " +
        "--update --antlr-ng-root /tmp/antlr-cleanroom/antlr-ng-1f68422 " +
        "--java-root /tmp/antlr-cleanroom/antlr4-4.13.2-tool " +
        "--antlr-jar /tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar " +
        "--icu-jar /tmp/antlr-cleanroom/tools/icu4j-78.1.jar";
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

async function checkFixture(logicalId, definition) {
    const directory = resolve(fixturesRoot, logicalId);
    for (const [path, contents] of definition.files) {
        await expectFile(resolve(directory, path), contents);
    }
    const manifest = JSON.parse(
        await readFile(resolve(directory, "fixture.json"), "utf8"),
    );
    for (const key of [
        "schema_version",
        "roots",
        "logical_ids",
        "upstream_tests",
        "java_antlr_test",
        "antlr_ng_test",
        "grammar_source",
        "expected",
        "general_atn_dot_oracle",
    ]) {
        if (
            JSON.stringify(manifest[key])
            !== JSON.stringify(definition.manifest[key])
        ) {
            throw new Error(`${logicalId} fixture manifest field ${key} differs`);
        }
    }
    for (const [path, expectedHash] of Object.entries(manifest.files ?? {})) {
        const actualHash = await digestFile(resolve(directory, path));
        if (actualHash !== expectedHash) {
            throw new Error(`${logicalId} fixture hash differs for ${path}`);
        }
    }
}

function extractVitestCases(source) {
    const methods = new Map();
    const masked = maskCode(source);
    const pattern = /\bit\(\s*"(?<name>[^"]+)"/gu;
    for (const match of source.matchAll(pattern)) {
        const arrow = masked.indexOf("=>", match.index);
        const open = masked.indexOf("{", arrow);
        const close = matchingDelimiter(masked, open, "{", "}");
        methods.set(match.groups.name, source.slice(open + 1, close));
    }
    return methods;
}

function verifyMethodBinding(method, config) {
    const relativeGrammar = config.grammarPath.slice("tests/bugs/".length);
    if (
        !method.includes(`new URL("${relativeGrammar}"`)
        || !method.includes(`getRule("${config.rule}")`)
    ) {
        throw new Error(
            `General case no longer binds ${config.grammarPath} rule ${config.rule}`,
        );
    }
}

function extractExpectedEdge(method) {
    const marker = "expect(result.indexOf(";
    const start = method.indexOf(marker);
    if (start < 0) {
        throw new Error("General case has no DOT edge assertion");
    }
    const open = start + marker.length - 1;
    const masked = maskCode(method);
    const close = matchingDelimiter(masked, open, "(", ")");
    const expression = method.slice(open + 1, close);
    const pieces = [];
    const templatePattern =
        /(?<raw>String\.raw)?`(?<body>(?:\\.|[^`])*)`/gu;
    for (const match of expression.matchAll(templatePattern)) {
        pieces.push(
            match.groups.raw
                ? match.groups.body
                : decodeTemplate(match.groups.body),
        );
    }
    if (pieces.length === 0) {
        throw new Error("General DOT assertion has no template strings");
    }
    return pieces.join("");
}

function decodeTemplate(body) {
    let output = "";
    for (let index = 0; index < body.length; index += 1) {
        if (body[index] !== "\\") {
            output += body[index];
            continue;
        }
        const escaped = body[++index];
        const simple = {
            n: "\n",
            r: "\r",
            t: "\t",
            "`": "`",
            "\\": "\\",
        };
        if (!Object.hasOwn(simple, escaped)) {
            throw new Error(`unsupported template escape \\${escaped}`);
        }
        output += simple[escaped];
    }
    return output;
}

function grammarDeclaration(grammar) {
    const match =
        /^\s*(?:lexer\s+)?grammar\s+(?<name>[A-Za-z_]\w*)\s*;/mu.exec(grammar);
    if (!match) {
        throw new Error("cannot read lexer grammar declaration");
    }
    return match.groups.name;
}

function requireEdge(dot, expectedEdge, implementation) {
    if (!dot.split("\n").includes(expectedEdge)) {
        throw new Error(
            `${implementation} DOT does not contain ${expectedEdge}`,
        );
    }
}

function ensureSpawned(result, label) {
    if (result.error) {
        throw new Error(`${label} failed to start: ${result.error.message}`);
    }
}

function matchingDelimiter(text, open, opening, closing) {
    if (open < 0 || text[open] !== opening) {
        throw new Error(`cannot find opening ${opening}`);
    }
    let depth = 0;
    for (let index = open; index < text.length; index += 1) {
        if (text[index] === opening) {
            depth += 1;
        } else if (text[index] === closing && --depth === 0) {
            return index;
        }
    }
    throw new Error(`unterminated ${opening}${closing} block`);
}

function maskCode(text) {
    const output = [...text];
    let state = "code";
    for (let index = 0; index < text.length; index += 1) {
        const current = text[index];
        const next = text[index + 1];
        if (state === "code") {
            if (current === "/" && next === "/") {
                output[index] = output[index + 1] = " ";
                state = "line-comment";
                index += 1;
            } else if (current === "/" && next === "*") {
                output[index] = output[index + 1] = " ";
                state = "block-comment";
                index += 1;
            } else if (current === '"' || current === "'" || current === "`") {
                output[index] = " ";
                state =
                    current === '"' ? "string" : current === "'" ? "character" : "template";
            }
        } else if (state === "line-comment") {
            if (current === "\n") {
                state = "code";
            } else {
                output[index] = " ";
            }
        } else if (state === "block-comment") {
            if (current === "*" && next === "/") {
                output[index] = output[index + 1] = " ";
                state = "code";
                index += 1;
            } else if (current !== "\n") {
                output[index] = " ";
            }
        } else if (current === "\\") {
            output[index] = " ";
            if (text[index + 1] !== "\n") {
                output[index + 1] = " ";
            }
            index += 1;
        } else if (
            (state === "string" && current === '"')
            || (state === "character" && current === "'")
            || (state === "template" && current === "`")
        ) {
            output[index] = " ";
            state = "code";
        } else if (current !== "\n") {
            output[index] = " ";
        }
    }
    return output.join("");
}

function parseArguments(args) {
    const result = {
        update: null,
        javaRoot:
            process.env.ANTLR4_TOOL_ROOT
            ?? "/tmp/antlr-cleanroom/antlr4-4.13.2-tool",
        antlrNgRoot:
            process.env.ANTLR_NG_ROOT
            ?? "/tmp/antlr-cleanroom/antlr-ng-1f68422",
        antlrJar:
            process.env.ANTLR4_JAR
            ?? "/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar",
        icuJar:
            process.env.ICU4J_JAR
            ?? "/tmp/antlr-cleanroom/tools/icu4j-78.1.jar",
        java: process.env.JAVA ?? "java",
    };
    for (let index = 0; index < args.length; index += 1) {
        const argument = args[index];
        switch (argument) {
            case "--update":
                result.update = true;
                break;
            case "--check":
                result.update = false;
                break;
            case "--java-root":
                result.javaRoot = resolve(requiredValue(args, ++index, argument));
                break;
            case "--antlr-ng-root":
                result.antlrNgRoot = resolve(
                    requiredValue(args, ++index, argument),
                );
                break;
            case "--antlr-jar":
                result.antlrJar = resolve(requiredValue(args, ++index, argument));
                break;
            case "--icu-jar":
                result.icuJar = resolve(requiredValue(args, ++index, argument));
                break;
            case "--java":
                result.java = requiredValue(args, ++index, argument);
                break;
            default:
                throw new Error(`unknown argument: ${argument}`);
        }
    }
    if (result.update === null) {
        throw new Error(
            "usage: generate-general-bug-fixtures.mjs --check|--update",
        );
    }
    return result;
}

function requiredValue(args, index, option) {
    const value = args[index];
    if (value === undefined) {
        throw new Error(`${option} requires a value`);
    }
    return value;
}

function verifyCommit(root, expected, label) {
    const result = spawnSync("git", ["rev-parse", "HEAD"], {
        cwd: root,
        encoding: "utf8",
    });
    if (result.status !== 0 || result.stdout.trim() !== expected) {
        throw new Error(
            `${label} root must be at ${expected}; found ` +
                `${result.stdout.trim() || result.stderr.trim()}`,
        );
    }
}

function gitText(root, object) {
    const result = spawnSync("git", ["show", object], {
        cwd: root,
        encoding: "utf8",
        maxBuffer: 16 * 1024 * 1024,
    });
    if (result.status !== 0) {
        throw new Error(`git show ${object} failed: ${result.stderr}`);
    }
    return result.stdout;
}

async function load(path) {
    return JSON.parse(await readFile(resolve(repositoryRoot, path), "utf8"));
}

async function expectFile(path, expected) {
    const actual = await readFile(path, "utf8");
    if (actual !== expected) {
        throw new Error(`${relative(repositoryRoot, path)} differs`);
    }
}

function digest(value) {
    return createHash("sha256").update(value).digest("hex");
}

async function digestFile(path) {
    return createHash("sha256").update(await readFile(path)).digest("hex");
}
