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
    "tool-testsuite/test/org/antlr/v4/test/tool/TestSymbolIssues.java";
const ANTLR_NG_PATH = "tests/TestSymbolIssues.spec.ts";
const JAVA_REPOSITORY = "https://github.com/antlr/antlr4.git";
const ANTLR_NG_REPOSITORY = "https://github.com/mike-lischke/antlr-ng.git";
const ERROR_CASES = new Set([
    "testA",
    "testB",
    "testD",
    "testF",
    "testEmptyLexerModeDetection",
    "testLabelsForTokensWithMixedTypes",
    "testLabelsForTokensWithMixedTypesLRWithoutLabels",
    "testTokensModesChannelsDeclarationConflictsWithReserved",
    "testTokensModesChannelsUsingConflictsWithReserved",
    "testUndefinedLabel",
    "testWrongIdForTypeChannelModeCommand",
]);

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
    .filter((row) => row.logical_id.startsWith("testsymbolissues-"))
    .sort((left, right) => left.logical_id.localeCompare(right.logical_id));
if (rows.length !== 26) {
    throw new Error(`expected 26 TestSymbolIssues rows, found ${rows.length}`);
}

const javaCases = extractJavaCases(
    gitText(options.javaRoot, `${JAVA_COMMIT}:${JAVA_PATH}`),
);
const antlrNgCases = extractAntlrNgCases(
    gitText(options.antlrNgRoot, `${ANTLR_NG_COMMIT}:${ANTLR_NG_PATH}`),
);

let fixtureCount = 0;
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

    const javaVariants = javaCases.get(javaCase.name);
    const antlrNgVariants = antlrNgCases.get(antlrNgCase.name);
    if (!javaVariants || !antlrNgVariants) {
        throw new Error(
            `${row.logical_id} cannot locate both pinned source methods`,
        );
    }
    if (
        JSON.stringify(javaVariants.map(({ grammar }) => grammar))
        !== JSON.stringify(antlrNgVariants.map(({ grammar }) => grammar))
    ) {
        throw new Error(
            `${row.logical_id} Java and antlr-ng grammar inputs differ`,
        );
    }

    for (const [index, variant] of javaVariants.entries()) {
        const fixtureName =
            index === 0 ? row.logical_id : `${row.logical_id}-variant-${index + 1}`;
        const definition = createFixtureDefinition(
            row,
            sourceCases,
            javaCase,
            antlrNgCase,
            variant,
            index,
            javaVariants.length,
        );
        if (options.update) {
            await updateFixture(fixtureName, definition);
        } else {
            await checkFixture(fixtureName, definition);
        }
        fixtureCount += 1;
    }
}

console.log(
    `${options.update ? "updated" : "verified"} ${fixtureCount} ` +
        "TestSymbolIssues fixtures",
);

function createFixtureDefinition(
    row,
    sourceCases,
    javaCase,
    antlrNgCase,
    variant,
    variantIndex,
    variantCount,
) {
    const declaration = grammarDeclaration(variant.grammar);
    const root = `${declaration.name}.g4`;
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
    const expectationsAgree = javaCase.name !== "testUndefinedLabel";
    return {
        root,
        grammar: variant.grammar,
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
            expected: ERROR_CASES.has(javaCase.name) ? "error" : "success",
            ...(variantCount > 1
                ? {
                      fixture_variant: {
                          index: variantIndex + 1,
                          count: variantCount,
                          source_binding: variant.binding,
                      },
                  }
                : {}),
            symbol_issues_oracle: {
                artifacts: [".interp", ".tokens", "diagnostics"],
                agreement: expectationsAgree
                    ? "Java and antlr-ng use the same grammar input and expected diagnostic categories"
                    : "Java and antlr-ng use the same grammar input but disagree on the expected diagnostic",
                compatibility_verdict: expectationsAgree
                    ? "Java ANTLR 4.13.2"
                    : "Java ANTLR 4.13.2 internal-error verdict is normative",
            },
        },
    };
}

async function updateFixture(fixtureName, definition) {
    const directory = resolve(fixturesRoot, fixtureName);
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
            fixtureName,
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
            `fixture updater failed for ${fixtureName} (${result.status}):\n` +
                `${result.stdout}\n${result.stderr}`,
        );
    }
    process.stdout.write(result.stdout);
}

async function checkFixture(fixtureName, definition) {
    const directory = resolve(fixturesRoot, fixtureName);
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
        "fixture_variant",
        "symbol_issues_oracle",
    ]) {
        if (
            JSON.stringify(manifest[key])
            !== JSON.stringify(definition.manifest[key])
        ) {
            throw new Error(
                `${fixtureName} fixture manifest field ${key} differs`,
            );
        }
    }
    for (const [path, expectedHash] of Object.entries(manifest.files ?? {})) {
        const actualHash = await digestFile(resolve(directory, path));
        if (actualHash !== expectedHash) {
            throw new Error(`${fixtureName} fixture hash differs for ${path}`);
        }
    }
}

function extractJavaCases(text) {
    const globals = extractArrayBindings(
        text,
        /\bfinal\s+static\s+String\[\]\s+(?<name>\w+)\s*=\s*\{/gu,
        "{",
        "}",
    );
    return extractMethodCases(
        extractMethods(
            text,
            /@Test\s+(?:public\s+)?void\s+(?<name>\w+)\s*\([^)]*\)[^{]*\{/gu,
        ),
        globals,
        /\bString\[\]\s+(?<name>\w+)\s*=\s*\{/gu,
        "{",
        "}",
        /\bString\s+grammar\s*=/gu,
    );
}

function extractAntlrNgCases(text) {
    const globals = extractArrayBindings(
        text,
        /\bconst\s+(?<name>testData\w+)\s*=\s*\[/gu,
        "[",
        "]",
    );
    return extractMethodCases(
        extractMethods(text, /\bit\(\s*"(?<name>[^"]+)"/gu, true),
        globals,
        /\bconst\s+(?<name>test\d*)\s*=\s*\[/gu,
        "[",
        "]",
        /\bconst\s+grammar\s*=/gu,
    );
}

function extractMethods(text, pattern, arrowBody = false) {
    const methods = new Map();
    const masked = maskCode(text);
    for (const match of text.matchAll(pattern)) {
        let open;
        if (arrowBody) {
            const arrow = masked.indexOf("=>", match.index);
            open = masked.indexOf("{", arrow);
        } else {
            open = masked.indexOf("{", match.index);
        }
        const close = matchingDelimiter(masked, open, "{", "}");
        methods.set(match.groups.name, text.slice(open + 1, close));
    }
    return methods;
}

function extractMethodCases(
    methods,
    globals,
    localArrayPattern,
    opening,
    closing,
    grammarPattern,
) {
    const cases = new Map();
    for (const [name, method] of methods) {
        let localArrays;
        try {
            localArrays = extractArrayBindings(
                method,
                localArrayPattern,
                opening,
                closing,
            );
        } catch (error) {
            throw new Error(`${name}: ${error.message}`, { cause: error });
        }
        if (localArrays.size > 0) {
            cases.set(
                name,
                [...localArrays].map(([binding, grammar]) => ({
                    binding,
                    grammar,
                })),
            );
            continue;
        }

        const reference =
            /\b(?:ToolTestUtils\.)?testErrors\(\s*(?<name>\w+)/u.exec(
                method,
            )?.groups.name;
        if (reference && globals.has(reference)) {
            cases.set(name, [
                { binding: reference, grammar: globals.get(reference) },
            ]);
            continue;
        }

        const grammar = extractAssignedString(method, grammarPattern);
        if (grammar !== null) {
            cases.set(name, [{ binding: "grammar", grammar }]);
        }
    }
    return cases;
}

function extractArrayBindings(text, pattern, opening, closing) {
    const bindings = new Map();
    const masked = maskCode(text);
    for (const match of masked.matchAll(pattern)) {
        const open = masked.indexOf(opening, match.index);
        const close = matchingDelimiter(masked, open, opening, closing);
        const comma = firstTopLevelComma(
            masked,
            open + 1,
            close,
            match.groups.name,
            text,
        );
        const expression = text.slice(open + 1, comma);
        bindings.set(match.groups.name, stringExpression(expression));
    }
    return bindings;
}

function extractAssignedString(text, pattern) {
    const masked = maskCode(text);
    const match = pattern.exec(masked);
    if (!match) {
        return null;
    }
    const equals = masked.indexOf("=", match.index);
    const semicolon = masked.indexOf(";", equals);
    if (semicolon < 0) {
        throw new Error("unterminated grammar string assignment");
    }
    return stringExpression(text.slice(equals + 1, semicolon));
}

function firstTopLevelComma(masked, start, end, binding, original) {
    let round = 0;
    let square = 0;
    let curly = 0;
    for (let index = start; index < end; index += 1) {
        switch (masked[index]) {
            case "(":
                round += 1;
                break;
            case ")":
                round -= 1;
                break;
            case "[":
                square += 1;
                break;
            case "]":
                square -= 1;
                break;
            case "{":
                curly += 1;
                break;
            case "}":
                curly -= 1;
                break;
            case ",":
                if (round === 0 && square === 0 && curly === 0) {
                    return index;
                }
                break;
            default:
                break;
        }
    }
    throw new Error(
        `fixture array ${binding} has no expected-output element: ` +
            JSON.stringify(original.slice(start, end)),
    );
}

function stringExpression(expression) {
    const strings = expression.match(/"(?:\\.|[^"\\])*"/gu) ?? [];
    if (strings.length === 0) {
        throw new Error(`expression has no string literals: ${expression}`);
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
            throw new Error(`unsupported source string escape \\${escaped}`);
        }
    }
    return result;
}

function grammarDeclaration(grammar) {
    const match =
        /^\s*(?:(?<kind>lexer|parser)\s+)?grammar\s+(?<name>[A-Za-z_]\w*)\s*;/u.exec(
            grammar,
        );
    if (!match) {
        throw new Error(
            `cannot read grammar declaration: ${grammar.slice(0, 80)}`,
        );
    }
    return {
        kind: match.groups.kind ?? "combined",
        name: match.groups.name,
    };
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
            (state === "string" && current === '"')
            || (state === "character" && current === "'")
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
            "usage: generate-symbol-issues-fixtures.mjs --check|--update",
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

async function digestFile(path) {
    return createHash("sha256")
        .update(await readFile(path))
        .digest("hex");
}
