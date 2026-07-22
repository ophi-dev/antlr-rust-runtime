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
    "tool-testsuite/test/org/antlr/v4/test/tool/TestLeftRecursionToolIssues.java";
const JAVA_ERROR_PATH = "tool/src/org/antlr/v4/tool/ErrorType.java";
const ANTLR_NG_PATH = "tests/TestLeftRecursionToolIssues.spec.ts";
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
    .filter((row) =>
        row.logical_id.startsWith("testleftrecursiontoolissues-")
    )
    .sort((left, right) => left.logical_id.localeCompare(right.logical_id));
if (rows.length !== 7) {
    throw new Error(
        `expected 7 TestLeftRecursionToolIssues rows, found ${rows.length}`,
    );
}

const javaMethods = extractJavaMethods(
    gitText(options.javaRoot, `${JAVA_COMMIT}:${JAVA_PATH}`),
);
const antlrNgMethods = extractAntlrNgMethods(
    gitText(options.antlrNgRoot, `${ANTLR_NG_COMMIT}:${ANTLR_NG_PATH}`),
);
const javaErrorCodes = extractJavaErrorCodes(
    gitText(options.javaRoot, `${JAVA_COMMIT}:${JAVA_ERROR_PATH}`),
);

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
    const javaGrammar = stringExpression(
        assignedExpression(javaMethod, "grammar", "java"),
        new Map(),
        null,
    );
    const antlrNgGrammar = stringExpression(
        assignedExpression(antlrNgMethod, "grammar", "typescript"),
        new Map(),
        null,
    );
    if (javaGrammar !== antlrNgGrammar) {
        throw new Error(
            `${row.logical_id} Java and antlr-ng grammar inputs differ`,
        );
    }

    const javaExpectedExpression = assignedExpression(
        javaMethod,
        "expected",
        "java",
    );
    const antlrNgExpectedExpression = assignedExpression(
        antlrNgMethod,
        "expected",
        "typescript",
    );
    const javaExpected = stringExpression(
        javaExpectedExpression,
        javaErrorCodes,
        /ErrorType\.(?<name>[A-Z][A-Z0-9_]*)\.code/gu,
    );
    const normalizedJavaExpected = stringExpression(
        javaExpectedExpression,
        new Map(),
        /ErrorType\.(?<name>[A-Z][A-Z0-9_]*)\.code/gu,
        "<CODE>",
    );
    const normalizedAntlrNgExpected = stringExpression(
        antlrNgExpectedExpression,
        new Map(),
        /IssueCode\.(?<name>[A-Za-z][A-Za-z0-9_]*)/gu,
        "<CODE>",
    );
    if (normalizedJavaExpected !== normalizedAntlrNgExpected) {
        throw new Error(
            `${row.logical_id} Java and antlr-ng expected diagnostics differ`,
        );
    }

    const definition = createFixtureDefinition(
        row,
        sourceCases,
        javaCase,
        antlrNgCase,
        javaGrammar,
        javaExpected,
    );
    if (options.update) {
        await updateFixture(row.logical_id, definition);
    } else {
        await checkFixture(row.logical_id, definition);
    }
}

console.log(
    `${options.update ? "updated" : "verified"} ` +
        `${rows.length} TestLeftRecursionToolIssues fixtures`,
);

function createFixtureDefinition(
    row,
    sourceCases,
    javaCase,
    antlrNgCase,
    grammar,
    expectedOutput,
) {
    const declaration = grammarDeclaration(grammar);
    const root = `${declaration.name}.g4`;
    const diagnostic = parseExpectedDiagnostic(expectedOutput);
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
        root,
        grammar,
        expectedOutput,
        manifest: {
            schema_version: 1,
            roots: [root],
            logical_ids: [row.logical_id],
            upstream_tests: sources,
            java_antlr_test: sources.find(
                (source) => source.source_case_id === javaCase.id,
            ),
            antlr_ng_test: sources.find(
                (source) => source.source_case_id === antlrNgCase.id,
            ),
            expected: diagnostic === null ? "success" : "error",
            left_recursion_oracle: {
                diagnostic,
                agreement:
                    "Java and antlr-ng source tests use the same grammar and expected diagnostic text",
                compatibility_verdict: "Java ANTLR 4.13.2",
            },
        },
    };
}

async function updateFixture(logicalId, definition) {
    const directory = resolve(fixturesRoot, logicalId);
    await rm(directory, { recursive: true, force: true });
    await mkdir(directory, { recursive: true });
    await writeFile(
        resolve(directory, definition.root),
        definition.grammar,
        "utf8",
    );
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
    await expectOracleOutput(directory, definition.expectedOutput);
    process.stdout.write(result.stdout);
}

async function checkFixture(logicalId, definition) {
    const directory = resolve(fixturesRoot, logicalId);
    await expectFile(
        resolve(directory, definition.root),
        definition.grammar,
    );
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
        "left_recursion_oracle",
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
    await expectOracleOutput(directory, definition.expectedOutput);
    for (const [path, expectedHash] of Object.entries(manifest.files ?? {})) {
        const actualHash = await digestFile(resolve(directory, path));
        if (actualHash !== expectedHash) {
            throw new Error(
                `${logicalId} fixture hash differs for ${path}`,
            );
        }
    }
}

async function expectOracleOutput(directory, expected) {
    const stdout = await readFile(
        resolve(directory, "oracle/java-antlr.stdout"),
        "utf8",
    );
    const stderr = await readFile(
        resolve(directory, "oracle/java-antlr.stderr"),
        "utf8",
    );
    if (`${stdout}${stderr}` !== expected) {
        throw new Error(
            `${relative(repositoryRoot, directory)} Java output differs ` +
                "from the pinned source test",
        );
    }
}

function parseExpectedDiagnostic(expected) {
    if (expected === "") {
        return null;
    }
    const match =
        /^(?<severity>warning|error)\((?<code>\d+)\): (?<file>[^:\n]+):(?<line>\d+):(?<column>\d+): (?<message>[^\n]+)\n$/u.exec(
            expected,
        );
    if (!match) {
        throw new Error(`unsupported expected diagnostic: ${expected}`);
    }
    return {
        severity: match.groups.severity,
        antlr_code: Number.parseInt(match.groups.code, 10),
        file: match.groups.file,
        line: Number.parseInt(match.groups.line, 10),
        column: Number.parseInt(match.groups.column, 10),
        message: match.groups.message,
    };
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

function extractJavaErrorCodes(text) {
    const codes = new Map();
    const pattern =
        /^\s*(?<name>[A-Z][A-Z0-9_]*)\s*\(\s*(?<code>\d+)\s*,/gmu;
    for (const match of text.matchAll(pattern)) {
        codes.set(match.groups.name, match.groups.code);
    }
    return codes;
}

function assignedExpression(method, variable, language) {
    const masked = maskCode(method);
    const declaration =
        language === "java"
            ? new RegExp(`\\bString\\s+${variable}\\s*=`, "u")
            : new RegExp(`\\bconst\\s+${variable}\\s*=`, "u");
    const match = declaration.exec(masked);
    if (!match) {
        throw new Error(`source method has no ${variable} assignment`);
    }
    const equals = masked.indexOf("=", match.index);
    const semicolon = masked.indexOf(";", equals + 1);
    if (semicolon < 0) {
        throw new Error(`unterminated ${variable} assignment`);
    }
    return method.slice(equals + 1, semicolon);
}

function stringExpression(
    expression,
    constants,
    constantPattern,
    constantReplacement = null,
) {
    const terms = [];
    const pattern =
        constantPattern === null
            ? /"(?:\\.|[^"\\])*"/gu
            : new RegExp(
                  `"(?:\\\\.|[^"\\\\])*"|${constantPattern.source}`,
                  "gu",
              );
    let cursor = 0;
    for (const match of expression.matchAll(pattern)) {
        expectSeparator(expression.slice(cursor, match.index), terms.length);
        if (match[0].startsWith('"')) {
            terms.push(decodeString(match[0]));
        } else if (constantReplacement !== null) {
            terms.push(constantReplacement);
        } else {
            const constant = constants.get(match.groups.name);
            if (constant === undefined) {
                throw new Error(
                    `unknown source constant ${match.groups.name}`,
                );
            }
            terms.push(constant);
        }
        cursor = match.index + match[0].length;
    }
    expectSeparator(expression.slice(cursor), terms.length, true);
    if (terms.length === 0) {
        throw new Error(
            `expression has no string terms: ${expression}`,
        );
    }
    return terms.join("");
}

function expectSeparator(text, termCount, trailing = false) {
    const pattern =
        termCount === 0 || trailing ? /^\s*$/u : /^\s*\+\s*$/u;
    if (!pattern.test(text)) {
        throw new Error(`unsupported string expression syntax: ${text}`);
    }
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

function grammarDeclaration(grammar) {
    const match =
        /^\s*(?:(?:lexer|parser)\s+)?grammar\s+(?<name>[A-Za-z_]\w*)\s*;/u.exec(
            grammar,
        );
    if (!match) {
        throw new Error(
            `cannot read grammar declaration: ${grammar.slice(0, 80)}`,
        );
    }
    return { name: match.groups.name };
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
            "usage: generate-left-recursion-tool-issue-fixtures.mjs " +
                "--check|--update",
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
