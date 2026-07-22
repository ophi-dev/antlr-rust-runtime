#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
    mkdir,
    readFile,
    readdir,
    rm,
    writeFile,
} from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const ANTLR_NG_COMMIT = "1f68422ae4bfc62f93343769e144d01f305487b1";
const JAVA_PATH =
    "tool-testsuite/test/org/antlr/v4/test/tool/TestAttributeChecks.java";
const ANTLR_NG_PATH = "tests/TestAttributeChecks.spec.ts";
const JAVA_REPOSITORY = "https://github.com/antlr/antlr4.git";
const ANTLR_NG_REPOSITORY = "https://github.com/mike-lischke/antlr-ng.git";
const CASES_PATH =
    "tests/codegen-direct/generated/attribute-checks-cases.inc.rs";
const METHODS = [
    {
        javaName: "testMembersActions",
        javaArray: "membersChecks",
        antlrNgArray: "membersChecks",
        location: "members",
        title: "testMembersActions: %s",
    },
    {
        javaName: "testDynamicMembersActions",
        javaArray: "dynMembersChecks",
        antlrNgArray: "dynMembersChecks",
        location: "members",
        title: "testDynamicMembersActions: %s",
    },
    {
        javaName: "testInitActions",
        javaArray: "initChecks",
        antlrNgArray: "initChecks",
        location: "init",
        title: "testInitActions: %s",
    },
    {
        javaName: "testDynamicInitActions",
        javaArray: "dynInitChecks",
        antlrNgArray: "dynInitChecks",
        location: "init",
        title: "testDynamicInitActions: %s",
    },
    {
        javaName: "testInlineActions",
        javaArray: "inlineChecks",
        antlrNgArray: "inlineChecks",
        location: "inline",
        title: "testInlineActions",
    },
    {
        javaName: "testDynamicInlineActions",
        javaArray: "dynInlineChecks",
        antlrNgArray: "dynInlineChecks",
        location: "inline",
        title: "testDynamicInlineActions",
    },
    {
        javaName: "testBadInlineActions",
        javaArray: "bad_inlineChecks",
        antlrNgArray: "badInlineChecks",
        location: "inline",
        title: "testBadInlineActions",
    },
    {
        javaName: "testFinallyActions",
        javaArray: "finallyChecks",
        antlrNgArray: "finallyChecks",
        location: "finally",
        title: "testFinallyActions",
    },
    {
        javaName: "testDynamicFinallyActions",
        javaArray: "dynFinallyChecks",
        antlrNgArray: "dynFinallyChecks",
        location: "finally",
        title: "testDynamicFinallyActions",
    },
];

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const fixturesRoot = resolve(repositoryRoot, "tests/codegen-direct/fixtures");
const options = parseArguments(process.argv.slice(2));

verifyCommit(options.javaRoot, JAVA_COMMIT, "Java ANTLR");
verifyCommit(options.antlrNgRoot, ANTLR_NG_COMMIT, "antlr-ng");

const javaText = gitText(options.javaRoot, `${JAVA_COMMIT}:${JAVA_PATH}`);
const antlrNgText = gitText(
    options.antlrNgRoot,
    `${ANTLR_NG_COMMIT}:${ANTLR_NG_PATH}`,
);
const javaTemplate = extractAssignedString(
    javaText,
    /\bString\s+attributeTemplate\s*=/gu,
);
const antlrNgTemplate = extractAssignedString(
    antlrNgText,
    /\bconst\s+attributeTemplate\s*=/gu,
);
if (javaTemplate !== antlrNgTemplate) {
    throw new Error("Java and antlr-ng attribute templates differ");
}

const javaArrays = extractJavaArrays(javaText);
const antlrNgArrays = extractAntlrNgArrays(antlrNgText);
const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const inventory = await load(
    "tests/codegen-direct/upstream-case-inventory.json",
);
const inventoryById = new Map(
    inventory.cases.map((testCase) => [testCase.id, testCase]),
);
const rows = testMap.rows.filter((row) =>
    row.logical_id.startsWith("testattributechecks-")
);
if (rows.length !== 44) {
    throw new Error(`expected 44 TestAttributeChecks rows, found ${rows.length}`);
}
const rowBySourceId = new Map();
for (const row of rows) {
    for (const sourceCaseId of row.source_case_ids) {
        if (rowBySourceId.has(sourceCaseId)) {
            throw new Error(`source case belongs to multiple rows: ${sourceCaseId}`);
        }
        rowBySourceId.set(sourceCaseId, row);
    }
}

const definitions = [];
const coveredRows = new Set();
const titleOccurrences = new Map();
for (const method of METHODS) {
    const javaPairs = requiredArray(javaArrays, method.javaArray, "Java");
    const antlrNgPairs = requiredArray(
        antlrNgArrays,
        method.antlrNgArray,
        "antlr-ng",
    );
    assertEquivalentPairs(method.javaName, javaPairs, antlrNgPairs);

    const javaCase = uniqueInventoryCase(
        "java-antlr",
        method.javaName,
    );
    const javaRow = requiredRow(javaCase.id);
    for (const [index, pair] of javaPairs.entries()) {
        const renderedTitle = method.title.includes("%s")
            ? method.title.replace("%s", pair.action)
            : method.title;
        const occurrence = increment(titleOccurrences, renderedTitle);
        const antlrNgId = stableId(
            "antlr-ng",
            `TestAttributeChecks/${renderedTitle}`,
            occurrence,
        );
        const antlrNgCase = inventoryById.get(antlrNgId);
        if (
            antlrNgCase?.implementation !== "antlr-ng"
            || antlrNgCase.suite !== "TestAttributeChecks"
        ) {
            throw new Error(
                `${method.javaName} variant ${index + 1} cannot locate ${antlrNgId}`,
            );
        }
        const antlrNgRow = requiredRow(antlrNgId);
        const fixtureName =
            index === 0
                ? javaRow.logical_id
                : `${javaRow.logical_id}-variant-${index + 1}`;
        definitions.push(
            createDefinition({
                fixtureName,
                grammar: renderTemplate(
                    javaTemplate,
                    method.location,
                    pair.action,
                ),
                root: "A.g4",
                logicalRows: [javaRow, antlrNgRow],
                javaCase,
                antlrNgCase,
                expectedError: pair.expected.length > 0,
                binding: {
                    method: method.javaName,
                    location: method.location,
                    action: pair.action,
                    index: index + 1,
                    count: javaPairs.length,
                },
            }),
        );
        coveredRows.add(javaRow.logical_id);
        coveredRows.add(antlrNgRow.logical_id);
    }
}

const javaTokenCase = uniqueInventoryCase("java-antlr", "testTokenRef");
const antlrNgTokenCase = uniqueInventoryCase("antlr-ng", "testTokenRef");
const javaTokenGrammar = extractGrammarFromMethod(
    javaText,
    "testTokenRef",
    /\bString\s+grammar\s*=/gu,
);
const antlrNgTokenGrammar = extractGrammarFromMethod(
    antlrNgText,
    "testTokenRef",
    /\bconst\s+grammar\s*=/gu,
);
if (javaTokenGrammar !== antlrNgTokenGrammar) {
    throw new Error("Java and antlr-ng testTokenRef grammars differ");
}
const tokenRow = requiredRow(javaTokenCase.id);
if (requiredRow(antlrNgTokenCase.id).logical_id !== tokenRow.logical_id) {
    throw new Error("testTokenRef source cases do not share one logical row");
}
definitions.push(
    createDefinition({
        fixtureName: tokenRow.logical_id,
        grammar: javaTokenGrammar,
        root: "S.g4",
        logicalRows: [tokenRow],
        javaCase: javaTokenCase,
        antlrNgCase: antlrNgTokenCase,
        expectedError: false,
        binding: {
            method: "testTokenRef",
            location: "inline",
            action: "Token t = $x; t = $ID;",
            index: 1,
            count: 1,
        },
    }),
);
coveredRows.add(tokenRow.logical_id);

if (definitions.length !== 121) {
    throw new Error(
        `expected 121 concrete TestAttributeChecks cases, found ${definitions.length}`,
    );
}
const missingRows = rows
    .map((row) => row.logical_id)
    .filter((logicalId) => !coveredRows.has(logicalId));
if (missingRows.length > 0) {
    throw new Error(`uncovered TestAttributeChecks rows: ${missingRows.join(", ")}`);
}

for (const definition of definitions) {
    if (options.update) {
        await updateFixture(definition);
    } else {
        await checkFixture(definition);
    }
}
await checkFixtureSet(definitions);
await updateOrCheckCases(definitions);

const errorCount = definitions.filter(
    (definition) => definition.manifest.expected === "error",
).length;
console.log(
    `${options.update ? "updated" : "verified"} ${definitions.length} ` +
        `TestAttributeChecks fixtures (${errorCount} error, ` +
        `${definitions.length - errorCount} success) covering ${coveredRows.size} rows`,
);

function createDefinition({
    fixtureName,
    grammar,
    root,
    logicalRows,
    javaCase,
    antlrNgCase,
    expectedError,
    binding,
}) {
    const logicalIds = [
        ...new Set(logicalRows.map((row) => row.logical_id)),
    ].sort();
    const javaSource = sourceRecord(javaCase);
    const antlrNgSource = sourceRecord(antlrNgCase);
    return {
        fixtureName,
        grammar,
        root,
        manifest: {
            schema_version: 1,
            roots: [root],
            logical_ids: logicalIds,
            upstream_tests: [javaSource, antlrNgSource],
            java_antlr_test: javaSource,
            antlr_ng_test: antlrNgSource,
            expected: expectedError ? "error" : "success",
            fixture_variant: binding,
            attribute_checks_oracle: {
                artifacts: expectedError
                    ? ["diagnostics"]
                    : [".interp", ".tokens", "diagnostics"],
                agreement:
                    "Java and antlr-ng use the same rendered grammar and expected diagnostic category",
                compatibility_verdict: "Java ANTLR 4.13.2",
            },
        },
    };
}

function sourceRecord(testCase) {
    return {
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
    };
}

async function updateFixture(definition) {
    const directory = resolve(fixturesRoot, definition.fixtureName);
    await rm(directory, { recursive: true, force: true });
    await mkdir(directory, { recursive: true });
    await writeFile(resolve(directory, definition.root), definition.grammar, "utf8");
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
            definition.fixtureName,
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
            `fixture updater failed for ${definition.fixtureName} ` +
                `(${result.status}):\n${result.stdout}\n${result.stderr}`,
        );
    }
    process.stdout.write(result.stdout);
}

async function checkFixture(definition) {
    const directory = resolve(fixturesRoot, definition.fixtureName);
    await expectFile(resolve(directory, definition.root), definition.grammar);
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
        "attribute_checks_oracle",
    ]) {
        if (
            JSON.stringify(manifest[key])
            !== JSON.stringify(definition.manifest[key])
        ) {
            throw new Error(
                `${definition.fixtureName} fixture manifest field ${key} differs`,
            );
        }
    }
    for (const [path, expectedHash] of Object.entries(manifest.files ?? {})) {
        const actualHash = await digestFile(resolve(directory, path));
        if (actualHash !== expectedHash) {
            throw new Error(
                `${definition.fixtureName} fixture hash differs for ${path}`,
            );
        }
    }
}

async function checkFixtureSet(definitionsToCheck) {
    const expected = new Set(
        definitionsToCheck.map((definition) => definition.fixtureName),
    );
    const actual = new Set(
        (await readdir(fixturesRoot, { withFileTypes: true }))
            .filter(
                (entry) =>
                    entry.isDirectory()
                    && entry.name.startsWith("testattributechecks-"),
            )
            .map((entry) => entry.name),
    );
    const missing = [...expected].filter((name) => !actual.has(name));
    const stale = [...actual].filter((name) => !expected.has(name));
    if (missing.length > 0 || stale.length > 0) {
        throw new Error(
            `TestAttributeChecks fixture set differs; missing=${missing.join(",")}; ` +
                `stale=${stale.join(",")}`,
        );
    }
}

async function updateOrCheckCases(definitionsToWrite) {
    const lines = [
        "// Generated by tools/grammar-frontend/generate-attribute-checks-fixtures.mjs.",
        "// Every entry is one concrete upstream parameterized action check.",
        "",
    ];
    for (const definition of definitionsToWrite) {
        const diagnostics = await fixtureDiagnostics(definition.fixtureName);
        lines.push(
            "case!(",
            `    ${definition.fixtureName.replaceAll("-", "_")},`,
            `    ${rustString(definition.fixtureName)},`,
            `    ${rustString(definition.root)},`,
            `    ${definition.manifest.expected === "error"},`,
            "    [",
        );
        for (const diagnostic of diagnostics) {
            lines.push(
                `        expected(${rustString(rustAttributeCode(diagnostic.code))}, ` +
                    `${diagnostic.line}, ${diagnostic.column}, ` +
                    `${rustString(diagnostic.message)}),`,
            );
        }
        lines.push("    ]", ");");
    }
    lines.push("");
    const text = lines.join("\n");
    const path = resolve(repositoryRoot, CASES_PATH);
    if (options.update) {
        await mkdir(dirname(path), { recursive: true });
        await writeFile(path, text, "utf8");
    } else {
        await expectFile(path, text);
    }
}

async function fixtureDiagnostics(fixtureName) {
    const path = resolve(fixturesRoot, fixtureName, "diagnostics.json");
    try {
        const diagnostics = JSON.parse(await readFile(path, "utf8"));
        return diagnostics.java_antlr ?? [];
    } catch (error) {
        if (error.code === "ENOENT") {
            return [];
        }
        throw error;
    }
}

function rustAttributeCode(javaCode) {
    const codes = new Map([
        [57, "G4S071"],
        [63, "G4S072"],
        [64, "G4S073"],
        [65, "G4S074"],
        [67, "G4S075"],
        [135, "G4S076"],
    ]);
    const code = codes.get(javaCode);
    if (!code) {
        throw new Error(`unexpected Java attribute diagnostic code ${javaCode}`);
    }
    return code;
}

function rustString(value) {
    return JSON.stringify(value)
        .replaceAll("\\b", "\\x08")
        .replaceAll("\\f", "\\x0c");
}

function extractJavaArrays(text) {
    return extractArrays(
        text,
        /\b(?:final\s+static\s+)?String\[\]\s+(?<name>\w+)\s*=\s*\{/gu,
        "{",
        "}",
        false,
    );
}

function extractAntlrNgArrays(text) {
    return extractArrays(
        text,
        /\bconst\s+(?<name>\w+Checks)\s*=\s*\[/gu,
        "[",
        "]",
        true,
    );
}

function extractArrays(text, pattern, opening, closing, nestedPairs) {
    const arrays = new Map();
    const masked = maskCode(text);
    for (const match of masked.matchAll(pattern)) {
        const open = masked.indexOf(opening, match.index);
        const close = matchingDelimiter(masked, open, opening, closing);
        const body = text.slice(open + 1, close);
        const bodyMask = masked.slice(open + 1, close);
        const expressions = splitTopLevel(body, bodyMask);
        const pairs = [];
        if (nestedPairs) {
            for (const expression of expressions) {
                const trimmed = trimCode(expression);
                if (!trimmed.startsWith("[") || !trimmed.endsWith("]")) {
                    throw new Error(
                        `${match.groups.name} contains a non-pair entry: ${trimmed}`,
                    );
                }
                const inner = trimmed.slice(1, -1);
                const values = splitTopLevel(inner, maskCode(inner));
                if (values.length !== 2) {
                    throw new Error(
                        `${match.groups.name} pair has ${values.length} values`,
                    );
                }
                pairs.push({
                    action: stringExpression(values[0]),
                    expected: stringExpression(values[1]),
                });
            }
        } else {
            if (expressions.length % 2 !== 0) {
                throw new Error(
                    `${match.groups.name} has an odd number of expressions`,
                );
            }
            for (let index = 0; index < expressions.length; index += 2) {
                pairs.push({
                    action: stringExpression(expressions[index]),
                    expected: stringExpression(expressions[index + 1]),
                });
            }
        }
        arrays.set(match.groups.name, pairs);
    }
    return arrays;
}

function trimCode(text) {
    const masked = maskCode(text);
    const start = masked.search(/\S/u);
    if (start < 0) {
        return "";
    }
    let end = masked.length - 1;
    while (end >= start && /\s/u.test(masked[end])) {
        end -= 1;
    }
    return text.slice(start, end + 1);
}

function splitTopLevel(text, masked) {
    const values = [];
    let start = 0;
    let round = 0;
    let square = 0;
    let curly = 0;
    for (let index = 0; index < masked.length; index += 1) {
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
                    values.push(text.slice(start, index));
                    start = index + 1;
                }
                break;
            default:
                break;
        }
    }
    if (text.slice(start).trim().length > 0) {
        values.push(text.slice(start));
    }
    return values;
}

function assertEquivalentPairs(name, javaPairs, antlrNgPairs) {
    if (javaPairs.length !== antlrNgPairs.length) {
        throw new Error(
            `${name} case count differs: Java ${javaPairs.length}, ` +
                `antlr-ng ${antlrNgPairs.length}`,
        );
    }
    for (let index = 0; index < javaPairs.length; index += 1) {
        const javaPair = javaPairs[index];
        const antlrNgPair = antlrNgPairs[index];
        if (
            javaPair.action !== antlrNgPair.action
            || Boolean(javaPair.expected) !== Boolean(antlrNgPair.expected)
        ) {
            throw new Error(`${name} variant ${index + 1} differs between sources`);
        }
    }
}

function extractGrammarFromMethod(text, name, assignmentPattern) {
    const masked = maskCode(text);
    const methodPattern =
        new RegExp(`\\b(?:void|it\\()\\s*${escapeRegExp(name)}\\b`, "u");
    let match = methodPattern.exec(masked);
    if (!match && text.includes(`"${name}"`)) {
        const title = text.indexOf(`"${name}"`);
        match = { index: title };
    }
    if (!match) {
        throw new Error(`cannot locate method ${name}`);
    }
    const open = masked.indexOf("{", match.index);
    const close = matchingDelimiter(masked, open, "{", "}");
    return extractAssignedString(text.slice(open + 1, close), assignmentPattern);
}

function extractAssignedString(text, pattern) {
    const masked = maskCode(text);
    pattern.lastIndex = 0;
    const match = pattern.exec(masked);
    if (!match) {
        throw new Error(`cannot locate assigned string matching ${pattern}`);
    }
    const equals = masked.indexOf("=", match.index);
    const semicolon = masked.indexOf(";", equals);
    if (semicolon < 0) {
        throw new Error("unterminated string assignment");
    }
    return stringExpression(text.slice(equals + 1, semicolon));
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

function renderTemplate(template, location, action) {
    const marker = `<${location}>`;
    if (!template.includes(marker)) {
        throw new Error(`attribute template has no ${marker}`);
    }
    return template
        .replace(marker, action)
        .replaceAll(/<(?:members|init|inline|inline2|finally)>/gu, "")
        .replaceAll(/^[\t ]+$/gmu, "");
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

function uniqueInventoryCase(implementation, name) {
    const matches = inventory.cases.filter(
        (testCase) =>
            testCase.implementation === implementation
            && testCase.suite === "TestAttributeChecks"
            && testCase.name === name,
    );
    if (matches.length !== 1) {
        throw new Error(
            `expected one ${implementation} TestAttributeChecks.${name}, ` +
                `found ${matches.length}`,
        );
    }
    return matches[0];
}

function requiredArray(arrays, name, source) {
    const value = arrays.get(name);
    if (!value) {
        throw new Error(`${source} source has no ${name} array`);
    }
    return value;
}

function requiredRow(sourceCaseId) {
    const row = rowBySourceId.get(sourceCaseId);
    if (!row) {
        throw new Error(`no TestAttributeChecks row owns ${sourceCaseId}`);
    }
    return row;
}

function stableId(implementation, identity, occurrence) {
    const readable = identity
        .normalize("NFKD")
        .toLowerCase()
        .replaceAll(/[^a-z0-9]+/gu, "-")
        .replaceAll(/^-|-$/gu, "")
        .slice(0, 72);
    const suffix = createHash("sha256")
        .update(`${identity}\0${occurrence}`)
        .digest("hex")
        .slice(0, 12);
    return `${implementation}:${readable}@${suffix}`;
}

function increment(counts, key) {
    const value = (counts.get(key) ?? 0) + 1;
    counts.set(key, value);
    return value;
}

function escapeRegExp(value) {
    return value.replaceAll(/[.*+?^${}()|[\]\\]/gu, "\\$&");
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
            "usage: generate-attribute-checks-fixtures.mjs --check|--update",
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
