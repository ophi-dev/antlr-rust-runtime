#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
    mkdir,
    readFile,
    rm,
    writeFile,
} from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const ANTLR_NG_COMMIT = "1f68422ae4bfc62f93343769e144d01f305487b1";
const JAVA_PATH =
    "tool-testsuite/test/org/antlr/v4/test/tool/TestLookaheadTrees.java";
const ANTLR_NG_PATH = "tests/TestLookaheadTrees.spec.ts";
const JAVA_REPOSITORY = "https://github.com/antlr/antlr4.git";
const ANTLR_NG_REPOSITORY = "https://github.com/mike-lischke/antlr-ng.git";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const fixturesRoot = resolve(repositoryRoot, "tests/codegen-direct/fixtures");
const options = parseArguments(process.argv.slice(2));

verifyCommit(options.javaRoot, JAVA_COMMIT, "Java ANTLR");
verifyCommit(options.antlrNgRoot, ANTLR_NG_COMMIT, "antlr-ng");

const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const inventory = await load(
    "tests/codegen-direct/upstream-case-inventory.json",
);
const inventoryById = new Map(
    inventory.cases.map((testCase) => [testCase.id, testCase]),
);
const rows = testMap.rows
    .filter((row) => row.logical_id.startsWith("testlookaheadtrees-"))
    .sort((left, right) => left.logical_id.localeCompare(right.logical_id));
if (rows.length !== 4) {
    throw new Error(
        `expected 4 TestLookaheadTrees rows, found ${rows.length}`,
    );
}

const javaSource = gitText(
    options.javaRoot,
    `${JAVA_COMMIT}:${JAVA_PATH}`,
);
const antlrNgSource = gitText(
    options.antlrNgRoot,
    `${ANTLR_NG_COMMIT}:${ANTLR_NG_PATH}`,
);
const javaMethods = extractJavaMethods(javaSource);
const antlrNgMethods = extractAntlrNgMethods(antlrNgSource);
const javaLexer = extractAssignedString(javaSource, "lexerText");
const antlrNgLexer = extractAssignedString(antlrNgSource, "lexerText");
if (javaLexer !== antlrNgLexer) {
    throw new Error("Java and antlr-ng shared lexer inputs differ");
}

for (const row of rows) {
    const sourceCases = row.source_case_ids.map((id) => {
        const testCase = inventoryById.get(id);
        if (!testCase) {
            throw new Error(
                `${row.logical_id} references unknown source case ${id}`,
            );
        }
        return testCase;
    });
    const javaCase = sourceCases.find(
        (testCase) => testCase.implementation === "java-antlr",
    );
    const antlrNgCase = sourceCases.find(
        (testCase) => testCase.implementation === "antlr-ng",
    );
    if (!javaCase || !antlrNgCase) {
        throw new Error(
            `${row.logical_id} must have Java and antlr-ng source cases`,
        );
    }

    const javaMethod = javaMethods.get(javaCase.name);
    const antlrNgMethod = antlrNgMethods.get(antlrNgCase.name);
    if (!javaMethod || !antlrNgMethod) {
        throw new Error(
            `${row.logical_id} cannot locate both pinned source methods`,
        );
    }
    const javaParser = extractParserGrammar(javaMethod);
    const antlrNgParser = extractParserGrammar(antlrNgMethod);
    if (javaParser !== antlrNgParser) {
        throw new Error(
            `${row.logical_id} Java and antlr-ng parser inputs differ`,
        );
    }
    const javaInvocations = extractInvocations(javaMethod);
    const antlrNgInvocations = extractInvocations(antlrNgMethod);
    if (
        JSON.stringify(javaInvocations) !==
        JSON.stringify(antlrNgInvocations)
    ) {
        throw new Error(
            `${row.logical_id} Java and antlr-ng lookahead oracles differ`,
        );
    }

    const definition = createFixtureDefinition(
        row,
        sourceCases,
        javaCase,
        antlrNgCase,
        javaLexer,
        javaParser,
        javaInvocations,
    );
    if (options.update) {
        await updateFixture(row.logical_id, definition);
    } else {
        await checkFixture(row.logical_id, definition);
    }
}

console.log(
    `${options.update ? "updated" : "verified"} ` +
        `${rows.length} TestLookaheadTrees fixtures`,
);

function createFixtureDefinition(
    row,
    sourceCases,
    javaCase,
    antlrNgCase,
    lexerGrammar,
    parserGrammar,
    invocations,
) {
    const normalizedParser = parserGrammar.replace(/[ \t]+$/gmu, "");
    const adaptedParser = normalizedParser.replace(
        /^(parser\s+grammar\s+T\s*;\s*\n)/u,
        "$1options { tokenVocab=L; }\n",
    );
    if (adaptedParser === parserGrammar) {
        throw new Error(`${row.logical_id} parser grammar was not adapted`);
    }
    const sources = sourceCases.map((testCase) => ({
        source_case_id: testCase.id,
        repository:
            testCase.implementation === "java-antlr"
                ? JAVA_REPOSITORY
                : ANTLR_NG_REPOSITORY,
        commit:
            testCase.implementation === "java-antlr"
                ? JAVA_COMMIT
                : ANTLR_NG_COMMIT,
        path: testCase.source.path,
        case: testCase.name,
        source_sha256: testCase.source.sha256,
    }));
    return {
        files: new Map([
            ["L.g4", lexerGrammar],
            ["T.g4", adaptedParser],
        ]),
        manifest: {
            schema_version: 1,
            roots: [
                "L.g4",
                "T.g4",
            ],
            logical_ids: [row.logical_id],
            upstream_tests: sources,
            java_antlr_test: sources.find(
                (source) => source.source_case_id === javaCase.id,
            ),
            antlr_ng_test: sources.find(
                (source) => source.source_case_id === antlrNgCase.id,
            ),
            expected: "success",
            lookahead_tree_oracle: {
                invocations,
                source_adaptation:
                    "normalized trailing horizontal whitespace and added options { tokenVocab=L; } to express the upstream in-memory LexerGrammar dependency as a source-set edge",
                agreement:
                    "Java and antlr-ng use the same lexer, parser grammar, inputs, decisions, and expected lookahead trees",
                compatibility_verdict: "Java ANTLR 4.13.2",
            },
        },
    };
}

async function updateFixture(logicalId, definition) {
    const directory = resolve(fixturesRoot, logicalId);
    await rm(directory, { recursive: true, force: true });
    await mkdir(directory, { recursive: true });
    for (const [path, contents] of definition.files) {
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
    if (result.error) {
        throw result.error;
    }
    if (result.status !== 0) {
        throw new Error(
            `fixture updater failed for ${logicalId} (${result.status}):\n` +
                `${result.stdout}\n${result.stderr}`,
        );
    }
    process.stdout.write(result.stdout);
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
        "expected",
        "lookahead_tree_oracle",
    ]) {
        if (
            JSON.stringify(manifest[key]) !==
            JSON.stringify(definition.manifest[key])
        ) {
            throw new Error(
                `${logicalId} fixture manifest field ${key} differs`,
            );
        }
    }
    for (const [path, expectedHash] of Object.entries(manifest.files ?? {})) {
        const actualHash = await digestFile(resolve(directory, path));
        if (actualHash !== expectedHash) {
            throw new Error(
                `${logicalId} fixture hash differs for ${path}`,
            );
        }
    }
}

function extractJavaMethods(text) {
    const methods = new Map();
    const masked = maskCode(text);
    const pattern =
        /@Test\s+public\s+void\s+(?<name>[$\w]+)\s*\([^)]*\)[^{]*\{/gu;
    for (const match of masked.matchAll(pattern)) {
        const open = masked.indexOf("{", match.index);
        const close = matchingDelimiter(masked, open, "{", "}");
        methods.set(match.groups.name, text.slice(open + 1, close));
    }
    return methods;
}

function extractAntlrNgMethods(text) {
    const methods = new Map();
    const masked = maskCode(text);
    const pattern = /\bit\(\s*"(?<name>[^"]+)"/gu;
    for (const match of text.matchAll(pattern)) {
        const arrow = masked.indexOf("=>", match.index);
        const open = masked.indexOf("{", arrow);
        const close = matchingDelimiter(masked, open, "{", "}");
        methods.set(match.groups.name, text.slice(open + 1, close));
    }
    return methods;
}

function extractAssignedString(text, variable) {
    const masked = maskCode(text);
    const match = new RegExp(`\\b${variable}\\s*=`, "u").exec(masked);
    if (!match) {
        throw new Error(`source has no ${variable} assignment`);
    }
    const equals = masked.indexOf("=", match.index);
    const semicolon = masked.indexOf(";", equals + 1);
    if (semicolon < 0) {
        throw new Error(`unterminated ${variable} assignment`);
    }
    return stringExpression(text.slice(equals + 1, semicolon));
}

function extractParserGrammar(method) {
    const masked = maskCode(method);
    const constructor = /\bnew\s+Grammar\s*\(/gu.exec(masked);
    if (!constructor) {
        throw new Error("source method has no Grammar constructor");
    }
    const open = masked.indexOf("(", constructor.index);
    const close = matchingDelimiter(masked, open, "(", ")");
    const argumentsList = splitArguments(
        method.slice(open + 1, close),
        masked.slice(open + 1, close),
    );
    return stringExpression(argumentsList[0]);
}

function extractInvocations(method) {
    const invocations = [];
    const masked = maskCode(method);
    const pattern = /\btestLookaheadTrees\s*\(/gu;
    for (const match of masked.matchAll(pattern)) {
        const open = masked.indexOf("(", match.index);
        const close = matchingDelimiter(masked, open, "(", ")");
        const argumentsList = splitArguments(
            method.slice(open + 1, close),
            masked.slice(open + 1, close),
        );
        if (argumentsList.length !== 6) {
            throw new Error(
                `expected 6 lookahead arguments, found ${argumentsList.length}`,
            );
        }
        const prefix = method.slice(0, match.index);
        invocations.push({
            input: evaluateString(argumentsList[2], prefix),
            start_rule: evaluateString(argumentsList[3], prefix),
            decision: evaluateInteger(argumentsList[4], prefix),
            expected_trees: stringLiterals(argumentsList[5]),
        });
    }
    if (invocations.length === 0) {
        throw new Error("source method has no testLookaheadTrees invocation");
    }
    return invocations;
}

function evaluateString(expression, prefix) {
    const trimmed = expression.trim();
    if (trimmed.startsWith('"')) {
        return stringExpression(trimmed);
    }
    return stringExpression(lastAssignment(prefix, trimmed));
}

function evaluateInteger(expression, prefix) {
    const trimmed = expression.trim();
    if (/^\d+$/u.test(trimmed)) {
        return Number.parseInt(trimmed, 10);
    }
    const assigned = lastAssignment(prefix, trimmed).trim();
    if (!/^\d+$/u.test(assigned)) {
        throw new Error(`unsupported integer expression: ${assigned}`);
    }
    return Number.parseInt(assigned, 10);
}

function lastAssignment(text, variable) {
    const masked = maskCode(text);
    const pattern = new RegExp(`\\b${variable}\\s*=`, "gu");
    let last = null;
    for (const match of masked.matchAll(pattern)) {
        last = match;
    }
    if (!last) {
        throw new Error(`source has no assignment for ${variable}`);
    }
    const equals = masked.indexOf("=", last.index);
    const semicolon = masked.indexOf(";", equals + 1);
    if (semicolon < 0) {
        throw new Error(`unterminated assignment for ${variable}`);
    }
    return text.slice(equals + 1, semicolon);
}

function splitArguments(text, masked) {
    const result = [];
    let start = 0;
    const stack = [];
    const matching = new Map([
        [")", "("],
        ["]", "["],
        ["}", "{"],
    ]);
    for (let index = 0; index < masked.length; index += 1) {
        const current = masked[index];
        if ("([{".includes(current)) {
            stack.push(current);
        } else if (matching.has(current)) {
            if (stack.pop() !== matching.get(current)) {
                throw new Error("unbalanced argument expression");
            }
        } else if (current === "," && stack.length === 0) {
            result.push(text.slice(start, index));
            start = index + 1;
        }
    }
    if (stack.length !== 0) {
        throw new Error("unterminated argument expression");
    }
    result.push(text.slice(start));
    return result;
}

function stringLiterals(expression) {
    const values = [];
    for (const match of expression.matchAll(/"(?:\\.|[^"\\])*"/gu)) {
        values.push(decodeString(match[0]));
    }
    if (values.length === 0) {
        throw new Error(`expression has no string literals: ${expression}`);
    }
    return values;
}

function stringExpression(expression) {
    const strings = expression.match(/"(?:\\.|[^"\\])*"/gu) ?? [];
    if (strings.length === 0) {
        throw new Error(
            `expression has no string literals: ${expression}`,
        );
    }
    return strings.map(decodeString).join("");
}

function decodeString(literal) {
    let result = "";
    for (let index = 1; index < literal.length - 1; index += 1) {
        const current = literal[index];
        if (current !== "\\") {
            result += current;
            continue;
        }
        const escaped = literal[++index];
        const simple = {
            b: "\b",
            f: "\f",
            n: "\n",
            r: "\r",
            t: "\t",
            '"': '"',
            "'": "'",
            "\\": "\\",
        };
        if (Object.hasOwn(simple, escaped)) {
            result += simple[escaped];
        } else {
            throw new Error(
                `unsupported source string escape \\${escaped}`,
            );
        }
    }
    return result;
}

function matchingDelimiter(text, open, opening, closing) {
    if (open < 0 || text[open] !== opening) {
        throw new Error(`cannot find opening ${opening}`);
    }
    let depth = 0;
    for (let index = open; index < text.length; index += 1) {
        if (text[index] === opening) {
            depth += 1;
        } else if (
            text[index] === closing &&
            --depth === 0
        ) {
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
            } else if (current === '"' || current === "'") {
                output[index] = " ";
                state = current === '"' ? "string" : "character";
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
            (state === "string" && current === '"') ||
            (state === "character" && current === "'")
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
            process.env.ANTLR4_TOOL_ROOT ??
            "/tmp/antlr-cleanroom/antlr4-4.13.2-tool",
        antlrNgRoot:
            process.env.ANTLR_NG_ROOT ??
            "/tmp/antlr-cleanroom/antlr-ng-1f68422",
        antlrJar:
            process.env.ANTLR4_JAR ??
            "/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar",
        icuJar:
            process.env.ICU4J_JAR ??
            "/tmp/antlr-cleanroom/tools/icu4j-78.1.jar",
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
                result.javaRoot = resolve(
                    requiredValue(args, ++index, argument),
                );
                break;
            case "--antlr-ng-root":
                result.antlrNgRoot = resolve(
                    requiredValue(args, ++index, argument),
                );
                break;
            case "--antlr-jar":
                result.antlrJar = resolve(
                    requiredValue(args, ++index, argument),
                );
                break;
            case "--icu-jar":
                result.icuJar = resolve(
                    requiredValue(args, ++index, argument),
                );
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
            "usage: generate-lookahead-tree-fixtures.mjs --check|--update",
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
    return JSON.parse(
        await readFile(resolve(repositoryRoot, path), "utf8"),
    );
}

async function expectFile(path, expected) {
    const actual = await readFile(path, "utf8");
    if (actual !== expected) {
        throw new Error(`${relative(repositoryRoot, path)} differs`);
    }
}

async function digestFile(path) {
    return createHash("sha256")
        .update(await readFile(path))
        .digest("hex");
}
