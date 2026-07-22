#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
    mkdir,
    mkdtemp,
    readFile,
    readdir,
    rm,
    writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const JAVA_ERROR_TYPE_PATH = "tool/src/org/antlr/v4/tool/ErrorType.java";
const JAVA_PATH =
    "tool-testsuite/test/org/antlr/v4/test/tool/TestToolSyntaxErrors.java";
const JAVA_REPOSITORY = "https://github.com/antlr/antlr4.git";
const ANTLR_JAR_SHA256 =
    "eae2dfa119a64327444672aff63e9ec35a20180dc5b8090b7a6ab85125df4d76";
const CASES_PATH =
    "tests/codegen-direct/generated/tool-syntax-errors-cases.inc.rs";
const META_CASE = "AllErrorCodesDistinct";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const fixturesRoot = resolve(repositoryRoot, "tests/codegen-direct/fixtures");
const options = parseArguments(process.argv.slice(2));

verifyCommit(options.javaRoot, JAVA_COMMIT, "Java ANTLR");

const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const inventory = await load(
    "tests/codegen-direct/upstream-case-inventory.json",
);
const inventoryById = new Map(
    inventory.cases.map((testCase) => [testCase.id, testCase]),
);
const rows = testMap.rows
    .filter(
        (row) =>
            row.owner_phase === "B"
            && row.source_case_ids.some((id) =>
                id.startsWith("java-antlr:testtoolsyntaxerrors-"),
            ),
    )
    .sort((left, right) => left.logical_id.localeCompare(right.logical_id));
if (rows.length !== 31) {
    throw new Error(
        `expected 31 Phase B TestToolSyntaxErrors rows, found ${rows.length}`,
    );
}

const javaText = gitText(options.javaRoot, `${JAVA_COMMIT}:${JAVA_PATH}`);
const javaErrorTypeText = gitText(
    options.javaRoot,
    `${JAVA_COMMIT}:${JAVA_ERROR_TYPE_PATH}`,
);
const javaErrorTypeSource = {
    repository: JAVA_REPOSITORY,
    commit: JAVA_COMMIT,
    path: JAVA_ERROR_TYPE_PATH,
    source_sha256: digestText(javaErrorTypeText),
};
const jarHash = await digestFile(options.antlrJar);
if (jarHash !== ANTLR_JAR_SHA256) {
    throw new Error(
        `ANTLR jar SHA-256 mismatch: expected ${ANTLR_JAR_SHA256}, ` +
            `found ${jarHash}`,
    );
}
const javaErrorTypes = await readJavaErrorTypes();
const methods = extractMethods(javaText);
const definitions = rows.map((row) => createDefinition(row, methods));
if (
    definitions.filter((definition) => definition.kind === "Meta").length
    !== 1
) {
    throw new Error("expected one source-only TestToolSyntaxErrors meta case");
}

for (const definition of definitions) {
    if (options.update) {
        await updateFixture(definition);
    } else {
        await checkFixture(definition);
    }
    await verifyJavaDiagnostics(definition);
}
await checkFixtureSet(definitions);
await updateOrCheckCases(definitions);

const grammarCases = definitions.filter(
    (definition) => definition.kind !== "Meta",
);
const errorCount = grammarCases.filter(
    (definition) => definition.manifest.expected === "error",
).length;
const warningCount = grammarCases.filter(
    (definition) => definition.sourceDiagnostics.some(
        (diagnostic) => diagnostic.severity === "warning",
    ),
).length;
console.log(
    `${options.update ? "updated" : "verified"} ${definitions.length} ` +
        `Phase B TestToolSyntaxErrors fixtures (${errorCount} error, ` +
        `${warningCount} warning, ` +
        `${grammarCases.length - errorCount - warningCount} success, 1 meta)`,
);

function createDefinition(row, methodBodies) {
    const javaIds = row.source_case_ids.filter((id) =>
        id.startsWith("java-antlr:"),
    );
    if (javaIds.length !== 1) {
        throw new Error(
            `${row.logical_id} must reference exactly one Java source case`,
        );
    }
    const javaCase = inventoryById.get(javaIds[0]);
    if (
        javaCase?.implementation !== "java-antlr"
        || javaCase.suite !== "TestToolSyntaxErrors"
    ) {
        throw new Error(
            `${row.logical_id} references an invalid Java source case`,
        );
    }
    const body = methodBodies.get(javaCase.name);
    if (body === undefined) {
        throw new Error(
            `${row.logical_id} cannot locate Java method ${javaCase.name}`,
        );
    }

    const javaSource = sourceRecord(javaCase);
    const commonManifest = {
        schema_version: 1,
        logical_ids: [row.logical_id],
        upstream_tests: [javaSource],
        java_antlr_test: javaSource,
        fixture_binding: {
            method: javaCase.name,
            source: "pinned Java TestToolSyntaxErrors.java",
        },
    };
    if (javaCase.name === META_CASE) {
        return {
            fixtureName: row.logical_id,
            kind: "Meta",
            root: null,
            grammar: null,
            sourceDiagnostics: [],
            javaErrorTypes,
            manifest: {
                ...commonManifest,
                roots: [],
                expected: "meta",
                java_error_type_source: javaErrorTypeSource,
                tool_syntax_errors_oracle: {
                    artifacts: [
                        "oracle/java-error-types.tsv",
                        "diagnostic-code-distinctness",
                    ],
                    error_type_count: javaErrorTypes.length,
                    compatibility_verdict: "Java ANTLR 4.13.2",
                },
            },
        };
    }

    const extracted = extractCase(body, javaCase.name);
    const declaration = grammarDeclaration(extracted.grammar);
    const sourceDiagnostics = sourceDiagnosticKinds(
        extracted.expectedExpression,
        javaCase.name,
    );
    const expectedError = sourceDiagnostics.some(
        (diagnostic) => diagnostic.severity === "error",
    );
    return {
        fixtureName: row.logical_id,
        kind: rustKind(declaration.kind),
        root: `${declaration.name}.g4`,
        grammar: extracted.grammar,
        sourceDiagnostics,
        manifest: {
            ...commonManifest,
            roots: [`${declaration.name}.g4`],
            expected: expectedError ? "error" : "success",
            tool_syntax_errors_oracle: {
                artifacts: expectedError
                    ? ["diagnostics"]
                    : [".interp", ".tokens", "diagnostics"],
                diagnostic_types: sourceDiagnostics.map(
                    (diagnostic) => diagnostic.errorType,
                ),
                compatibility_verdict: "Java ANTLR 4.13.2",
            },
        },
    };
}

function sourceRecord(testCase) {
    return {
        source_case_id: testCase.id,
        repository: JAVA_REPOSITORY,
        commit: JAVA_COMMIT,
        path: testCase.source.path,
        case: testCase.name,
        source_sha256: testCase.source.sha256,
    };
}

async function updateFixture(definition) {
    const directory = resolve(fixturesRoot, definition.fixtureName);
    await rm(directory, { recursive: true, force: true });
    await mkdir(directory, { recursive: true });
    if (definition.kind === "Meta") {
        const oracleDirectory = resolve(directory, "oracle");
        await mkdir(oracleDirectory);
        const oraclePath = resolve(
            oracleDirectory,
            "java-error-types.tsv",
        );
        await writeFile(
            oraclePath,
            errorTypesText(definition.javaErrorTypes),
            "utf8",
        );
        const manifest = {
            ...definition.manifest,
            files: {
                "oracle/java-error-types.tsv": await digestFile(oraclePath),
            },
        };
        await writeFile(
            resolve(directory, "fixture.json"),
            `${JSON.stringify(manifest, null, 2)}\n`,
            "utf8",
        );
        return;
    }

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
    if (definition.kind !== "Meta") {
        await expectFile(
            resolve(directory, definition.root),
            definition.grammar,
        );
    } else {
        await expectFile(
            resolve(directory, "oracle/java-error-types.tsv"),
            errorTypesText(definition.javaErrorTypes),
        );
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
        "fixture_binding",
        "expected",
        "java_error_type_source",
        "tool_syntax_errors_oracle",
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

async function verifyJavaDiagnostics(definition) {
    if (definition.kind === "Meta") {
        return;
    }
    const actual = await fixtureDiagnostics(definition.fixtureName);
    if (actual.length !== definition.sourceDiagnostics.length) {
        throw new Error(
            `${definition.fixtureName} Java diagnostic count differs from ` +
                `the pinned test source: expected ` +
                `${definition.sourceDiagnostics.length}, found ${actual.length}`,
        );
    }
    for (let index = 0; index < actual.length; index += 1) {
        const actualDiagnostic = actual[index];
        const sourceDiagnostic = definition.sourceDiagnostics[index];
        if (actualDiagnostic.severity !== sourceDiagnostic.severity) {
            throw new Error(
                `${definition.fixtureName} Java diagnostic ${index + 1} ` +
                    `severity differs from the pinned test source`,
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
                    && entry.name.startsWith("testtoolsyntaxerrors-"),
            )
            .map((entry) => entry.name),
    );
    const missing = [...expected].filter((name) => !actual.has(name));
    const stale = [...actual].filter((name) => !expected.has(name));
    if (missing.length > 0 || stale.length > 0) {
        throw new Error(
            `TestToolSyntaxErrors fixture set differs; ` +
                `missing=${missing.join(",")}; stale=${stale.join(",")}`,
        );
    }
}

async function updateOrCheckCases(definitionsToWrite) {
    const diagnosticsByFixture = new Map();
    const javaCodesByType = new Map();
    for (const definition of definitionsToWrite) {
        if (definition.kind === "Meta") {
            continue;
        }
        const diagnostics = await fixtureDiagnostics(
            definition.fixtureName,
        );
        diagnosticsByFixture.set(definition.fixtureName, diagnostics);
        for (let index = 0; index < diagnostics.length; index += 1) {
            const errorType = definition.sourceDiagnostics[index].errorType;
            const javaCode = diagnostics[index].code;
            const previous = javaCodesByType.get(errorType);
            if (previous !== undefined && previous !== javaCode) {
                throw new Error(
                    `${errorType} uses Java codes ${previous} and ${javaCode}`,
                );
            }
            javaCodesByType.set(errorType, javaCode);
        }
    }
    const typeByJavaCode = new Map();
    for (const { name: errorType, code: javaCode } of javaErrorTypes) {
        const previous = typeByJavaCode.get(javaCode);
        if (previous !== undefined && previous !== errorType) {
            throw new Error(
                `Java code ${javaCode} is shared by ${previous} and ${errorType}`,
            );
        }
        typeByJavaCode.set(javaCode, errorType);
    }
    const allJavaCodesByType = new Map(
        javaErrorTypes.map(({ name, code }) => [name, code]),
    );
    for (const [errorType, javaCode] of javaCodesByType) {
        if (allJavaCodesByType.get(errorType) !== javaCode) {
            throw new Error(
                `${errorType} fixture code ${javaCode} differs from ` +
                    `the Java ErrorType oracle`,
            );
        }
    }

    const lines = [
        "// Generated by tools/grammar-frontend/generate-tool-syntax-errors-fixtures.mjs.",
        "// Inputs and diagnostics come only from Java ANTLR 4.13.2.",
        "",
    ];
    for (const definition of definitionsToWrite) {
        const name = definition.fixtureName.replaceAll("-", "_");
        if (definition.kind === "Meta") {
            lines.push(
                "meta_case!(",
                `    ${name},`,
                `    ${rustString(definition.fixtureName)},`,
                "    [",
            );
            for (
                const { name: errorType, code: javaCode } of [
                    ...definition.javaErrorTypes,
                ].sort(
                    (left, right) => left.name.localeCompare(right.name),
                )
            ) {
                lines.push(
                    `        java_error(${javaCode}, ` +
                        `${rustString(errorType)}),`,
                );
            }
            lines.push("    ]", ");");
            continue;
        }

        const diagnostics = diagnosticsByFixture.get(definition.fixtureName);
        lines.push(
            "case!(",
            `    ${name},`,
            `    ${rustString(definition.fixtureName)},`,
            `    ${rustString(definition.root)},`,
            `    ${definition.kind},`,
            `    ${definition.manifest.expected === "error"},`,
            "    [",
        );
        for (const diagnostic of diagnostics) {
            if (
                diagnostic.line === undefined
                || diagnostic.column === undefined
            ) {
                lines.push(
                    `        unlocated(${diagnostic.code}, ` +
                        `${rustSeverity(diagnostic.severity)}),`,
                );
            } else {
                lines.push(
                    `        at(${diagnostic.code}, ` +
                        `${rustSeverity(diagnostic.severity)}, ` +
                        `${diagnostic.line}, ${diagnostic.column}),`,
                );
            }
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

async function readJavaErrorTypes() {
    const directory = await mkdtemp(
        resolve(tmpdir(), "antlr-rust-error-types-"),
    );
    const sourcePath = resolve(directory, "PrintErrorTypes.java");
    const source = `import org.antlr.v4.tool.ErrorType;

public class PrintErrorTypes {
    public static void main(String[] args) {
        for (ErrorType errorType : ErrorType.class.getEnumConstants()) {
            System.out.println(errorType.code + "\\t" + errorType.name());
        }
    }
}
`;
    try {
        await writeFile(sourcePath, source, "utf8");
        const result = spawnSync(
            options.java,
            ["--class-path", options.antlrJar, sourcePath],
            {
                encoding: "utf8",
                maxBuffer: 16 * 1024 * 1024,
            },
        );
        if (result.error) {
            throw result.error;
        }
        if (result.status !== 0) {
            throw new Error(
                `Java ErrorType helper failed (${result.status}): ` +
                    result.stderr,
            );
        }
        const errorTypes = result.stdout
            .trimEnd()
            .split("\n")
            .filter(Boolean)
            .map((line) => {
                const [rawCode, name, ...extra] = line.split("\t");
                const code = Number.parseInt(rawCode, 10);
                if (
                    !Number.isSafeInteger(code)
                    || name === undefined
                    || name.length === 0
                    || extra.length > 0
                ) {
                    throw new Error(
                        `unexpected Java ErrorType helper output: ${line}`,
                    );
                }
                return { code, name };
            });
        if (errorTypes.length === 0) {
            throw new Error("Java ErrorType helper returned no records");
        }
        return errorTypes;
    } finally {
        await rm(directory, { recursive: true, force: true });
    }
}

function errorTypesText(errorTypes) {
    return errorTypes
        .map(({ code, name }) => `${code}\t${name}\n`)
        .join("");
}

function extractMethods(text) {
    const methods = new Map();
    const masked = maskCode(text);
    const pattern =
        /@Test\s+(?:public\s+)?void\s+(?<name>\w+)\s*\([^)]*\)[^{]*\{/gu;
    for (const match of masked.matchAll(pattern)) {
        const open = masked.indexOf("{", match.index);
        const close = matchingDelimiter(masked, open, "{", "}");
        methods.set(match.groups.name, text.slice(open + 1, close));
    }
    return methods;
}

function extractCase(body, methodName) {
    const bindings = extractStringBindings(body);
    let grammar = bindings.get("grammar");
    let expected = bindings.get("expected");

    const pair = extractPair(body, bindings);
    grammar ??= pair?.[0];
    expected ??= pair?.[1];
    if (grammar === undefined || expected === undefined) {
        throw new Error(
            `${methodName}: cannot extract grammar and expected output`,
        );
    }
    return {
        grammar: grammar.value,
        expectedExpression: expected.expression,
    };
}

function extractStringBindings(text) {
    const bindings = new Map();
    const masked = maskCode(text);
    const pattern = /\bString\s+(?<name>\w+)\s*=/gu;
    for (const match of masked.matchAll(pattern)) {
        const equals = masked.indexOf("=", match.index);
        const semicolon = masked.indexOf(";", equals);
        if (semicolon < 0) {
            throw new Error(
                `unterminated string assignment ${match.groups.name}`,
            );
        }
        const expression = text.slice(equals + 1, semicolon);
        bindings.set(match.groups.name, {
            expression,
            value: stringExpression(expression),
        });
    }
    return bindings;
}

function extractPair(text, bindings) {
    const masked = maskCode(text);
    const declaration = /\bString\s*\[\]\s+\w+\s*=/gu.exec(masked);
    let searchFrom;
    if (declaration !== null) {
        searchFrom = declaration.index;
    } else {
        const call = /\btestErrors\s*\(/gu.exec(masked);
        if (call === null) {
            return null;
        }
        searchFrom = call.index;
    }
    const open = masked.indexOf("{", searchFrom);
    if (open < 0) {
        return null;
    }
    const close = matchingDelimiter(masked, open, "{", "}");
    const body = text.slice(open + 1, close);
    const expressions = splitTopLevel(body, masked.slice(open + 1, close));
    if (expressions.length < 2) {
        throw new Error("TestToolSyntaxErrors pair has fewer than two values");
    }
    return expressions.slice(0, 2).map((expression) => {
        const name = expression.trim();
        const binding = bindings.get(name);
        if (binding !== undefined) {
            return binding;
        }
        return {
            expression,
            value: stringExpression(expression),
        };
    });
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

function sourceDiagnosticKinds(expression, methodName) {
    const source = stripComments(expression);
    const output = stringExpression(source);
    const severities = [
        ...output.matchAll(/\b(?<severity>error|warning)\(/gu),
    ].map((match) => match.groups.severity);
    const errorTypes = [
        ...source.matchAll(/\bErrorType\.(?<name>\w+)\.code\b/gu),
    ].map((match) => match.groups.name);
    if (severities.length !== errorTypes.length) {
        throw new Error(
            `${methodName}: expected-output diagnostic markers ` +
                `(${severities.length}) ` +
                `differ from ErrorType references (${errorTypes.length}): ` +
                JSON.stringify(output),
        );
    }
    return severities.map((severity, index) => ({
        severity,
        errorType: errorTypes[index],
    }));
}

function stringExpression(expression) {
    const strings =
        stripComments(expression).match(/"(?:\\.|[^"\\])*"/gu) ?? [];
    if (strings.length === 0) {
        throw new Error(`expression has no string literals: ${expression}`);
    }
    return strings.map(decodeString).join("");
}

function stripComments(text) {
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
            index += 1;
        } else if (
            (state === "string" && current === '"')
            || (state === "character" && current === "'")
        ) {
            state = "code";
        }
    }
    return output.join("");
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

function rustKind(kind) {
    return {
        combined: "Combined",
        lexer: "Lexer",
        parser: "Parser",
    }[kind];
}

function rustSeverity(severity) {
    return severity === "error" ? "Error" : "Warning";
}

function rustString(value) {
    return JSON.stringify(value)
        .replaceAll("\\b", "\\x08")
        .replaceAll("\\f", "\\x0c");
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
            "usage: generate-tool-syntax-errors-fixtures.mjs --check|--update",
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

function digestText(text) {
    return createHash("sha256").update(text).digest("hex");
}
