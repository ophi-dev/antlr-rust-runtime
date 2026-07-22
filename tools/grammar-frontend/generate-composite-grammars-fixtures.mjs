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
    "tool-testsuite/test/org/antlr/v4/test/tool/TestCompositeGrammars.java";
const JAVA_GRAMMAR_PATH =
    "tool-testsuite/test/org/antlr/v4/test/tool/Java.g4";
const JAVA_ERROR_TYPE_PATH = "tool/src/org/antlr/v4/tool/ErrorType.java";
const JAVA_REPOSITORY = "https://github.com/antlr/antlr4.git";
const CASES_PATH =
    "tests/codegen-direct/generated/composite-grammars-cases.inc.rs";
const CORRECTED_HEADER_CASE =
    "testcompositegrammars-testheaderspropagatedcorrectlytoimportedgrammars-f8ff35ee27";
const JAVA_HEADER_CASE =
    "testcompositegrammars-testheaderspropogatedcorrectlytoimportedgrammars-e8e7638e04";

const CONFIG = new Map([
    ["testImportFileLocationInSubdir", config(["M.g4"], ["sub"])],
    ["testImportSelfLoop", config(["M.g4"])],
    ["testImportIntoLexerGrammar", config(["M.g4"])],
    ["testImportModesIntoLexerGrammar", config(["M.g4"])],
    ["testImportChannelsIntoLexerGrammar", config(["M.g4"])],
    ["testImportMixedChannelsIntoLexerGrammar", config(["M.g4"])],
    ["testImportClashingChannelsIntoLexerGrammar", config(["M.g4"])],
    ["testMergeModesIntoLexerGrammar", config(["M.g4"])],
    ["testEmptyModesInLexerGrammar", config(["M.g4"])],
    [
        "testCombinedGrammarImportsModalLexerGrammar",
        config(["M.g4"], [], [
            expectedDiagnostic("MODE_NOT_IN_LEXER", "error"),
        ]),
    ],
    ["testDelegatesSeeSameTokenType", config(["M.g4"])],
    [
        "testErrorInImportedGetsRightFilename",
        config(["M.g4"], [], [
            expectedDiagnostic("UNDEFINED_RULE_REF", "error"),
        ]),
    ],
    [
        "testImportFileNotSearchedForInOutputDir",
        config(["M.g4"], [], [
            expectedDiagnostic("CANNOT_FIND_IMPORTED_GRAMMAR", "error"),
        ]),
    ],
    [
        "testOutputDirShouldNotEffectImports",
        config(["M.g4"], ["sub"]),
    ],
    [
        "testTokensFileInOutputDirAndImportFileInSubdir",
        config(["MLexer.g4", "MParser.g4"], ["sub"]),
    ],
    [
        "testImportedTokenVocabIgnoredWithWarning",
        config(["M.g4"], [], [
            expectedDiagnostic("OPTIONS_IN_DELEGATE", "warning"),
        ]),
    ],
    [
        "testSyntaxErrorsInImportsNotThrownOut",
        config(["M.g4"], [], [
            expectedDiagnostic("SYNTAX_ERROR", "error"),
        ]),
    ],
    ["test3LevelImport", config(["M.g4"])],
    ["testBigTreeOfImports", config(["M.g4"])],
    ["testRulesVisibleThroughMultilevelImport", config(["M.g4"])],
    ["testNestedComposite", config(["G3.g4"])],
    [
        "testHeadersPropogatedCorrectlyToImportedGrammars",
        config(["M.g4"], [], [], { "M.g4": "master" }),
    ],
    [
        "testImportLargeGrammar",
        config(["NewJava.g4"], [], [], { "NewJava.g4": "master" }),
    ],
    [
        "testImportLeftRecursiveGrammar",
        config(["T.g4"], [], [], { "T.g4": "master" }),
    ],
    [
        "testCircularGrammarInclusion",
        config(["G2.g4"], [], [], { "G2.g4": "g2" }),
    ],
]);

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const fixturesRoot = resolve(repositoryRoot, "tests/codegen-direct/fixtures");
const options = parseArguments(process.argv.slice(2));

verifyCommit(options.javaRoot, JAVA_COMMIT, "Java ANTLR");
const javaText = gitText(options.javaRoot, `${JAVA_COMMIT}:${JAVA_PATH}`);
const javaGrammar = gitText(
    options.javaRoot,
    `${JAVA_COMMIT}:${JAVA_GRAMMAR_PATH}`,
);
const javaErrorTypes = parseJavaErrorTypes(
    gitText(options.javaRoot, `${JAVA_COMMIT}:${JAVA_ERROR_TYPE_PATH}`),
);
const methods = extractMethods(javaText);
const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const inventory = await load(
    "tests/codegen-direct/upstream-case-inventory.json",
);
const inventoryById = new Map(
    inventory.cases.map((testCase) => [testCase.id, testCase]),
);
const rows = testMap.rows
    .filter((row) =>
        row.logical_id.startsWith("testcompositegrammars-")
    )
    .sort((left, right) => left.logical_id.localeCompare(right.logical_id));
if (rows.length !== 26) {
    throw new Error(
        `expected 26 Phase B TestCompositeGrammars rows, found ${rows.length}`,
    );
}

const definitions = rows.map(createDefinition);
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

const errorCount = definitions.filter(
    (definition) => definition.manifest.expected === "error",
).length;
const warningCount = definitions.filter(
    (definition) => definition.expectedDiagnostics.some(
        (diagnostic) => diagnostic.severity === "warning",
    ),
).length;
console.log(
    `${options.update ? "updated" : "verified"} ${definitions.length} ` +
        `Phase B TestCompositeGrammars fixtures (${errorCount} error, ` +
        `${warningCount} warning, ` +
        `${definitions.length - errorCount - warningCount} success)`,
);

function config(
    roots,
    libraryDirectories = [],
    diagnostics = [],
    rootBindings = {},
) {
    return { diagnostics, libraryDirectories, rootBindings, roots };
}

function expectedDiagnostic(errorType, severity) {
    return { errorType, severity };
}

function createDefinition(row) {
    const javaIds = row.source_case_ids.filter((id) =>
        id.startsWith("java-antlr:"),
    );
    let methodName;
    let javaCase;
    let independentOracle = false;
    if (javaIds.length === 1) {
        javaCase = inventoryById.get(javaIds[0]);
        methodName = javaCase?.name;
    } else if (
        javaIds.length === 0
        && row.logical_id === CORRECTED_HEADER_CASE
    ) {
        const javaRow = rows.find(
            (candidate) => candidate.logical_id === JAVA_HEADER_CASE,
        );
        const javaId = javaRow?.source_case_ids.find((id) =>
            id.startsWith("java-antlr:"),
        );
        javaCase = inventoryById.get(javaId);
        methodName = "testHeadersPropogatedCorrectlyToImportedGrammars";
        independentOracle = true;
    } else {
        throw new Error(
            `${row.logical_id} must reference one Java source case or ` +
                "the documented corrected-header independent oracle",
        );
    }
    if (
        javaCase?.implementation !== "java-antlr"
        || javaCase.suite !== "TestCompositeGrammars"
        || methodName === undefined
    ) {
        throw new Error(
            `${row.logical_id} references an invalid Java source case`,
        );
    }
    const body = methods.get(methodName);
    const caseConfig = CONFIG.get(methodName);
    if (body === undefined || caseConfig === undefined) {
        throw new Error(
            `${row.logical_id} cannot locate Java method/config ${methodName}`,
        );
    }
    const files = extractMethodFiles(body);
    for (const [path, binding] of Object.entries(caseConfig.rootBindings)) {
        const value = files.bindings.get(binding);
        if (value === undefined) {
            throw new Error(`${methodName}: missing root binding ${binding}`);
        }
        files.sources.set(path, value);
    }
    for (const root of caseConfig.roots) {
        if (!files.sources.has(root)) {
            throw new Error(`${methodName}: missing root source ${root}`);
        }
    }
    const expectedDiagnostics = caseConfig.diagnostics.map((diagnostic) => {
        const errorType = javaErrorTypes.get(diagnostic.errorType);
        if (errorType === undefined) {
            throw new Error(
                `${methodName}: unknown Java ErrorType ${diagnostic.errorType}`,
            );
        }
        if (errorType.severity !== diagnostic.severity) {
            throw new Error(
                `${methodName}: Java ErrorType ${diagnostic.errorType} ` +
                    `severity is ${errorType.severity}, not ` +
                    diagnostic.severity,
            );
        }
        return { ...diagnostic, javaCode: errorType.code };
    });
    const upstreamTests = row.source_case_ids.map((id) => {
        const testCase = inventoryById.get(id);
        if (testCase === undefined) {
            throw new Error(`${row.logical_id}: unknown source case ${id}`);
        }
        return sourceRecord(testCase);
    });
    const javaSource = sourceRecord(javaCase);
    const expected = expectedDiagnostics.some(
        (diagnostic) => diagnostic.severity === "error",
    )
        ? "error"
        : "success";
    return {
        expectedDiagnostics,
        files: files.sources,
        fixtureName: row.logical_id,
        libraryDirectories: caseConfig.libraryDirectories,
        roots: caseConfig.roots,
        sourceOrder: sourceOrder(
            files.sources,
            caseConfig.roots,
            caseConfig.libraryDirectories,
        ),
        manifest: {
            schema_version: 1,
            roots: caseConfig.roots,
            library_directories: caseConfig.libraryDirectories,
            logical_ids: [row.logical_id],
            upstream_tests: upstreamTests,
            java_antlr_test: javaSource,
            fixture_binding: {
                method: methodName,
                source: independentOracle
                    ? "independent Java 4.13.2 generated oracle for the corrected antlr-ng case name"
                    : "pinned Java TestCompositeGrammars.java",
            },
            expected,
            composite_grammars_oracle: {
                artifacts: expected === "error"
                    ? ["diagnostics"]
                    : [".interp", ".tokens", "diagnostics"],
                diagnostic_types: expectedDiagnostics.map(
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
        repository: testCase.implementation === "java-antlr"
            ? JAVA_REPOSITORY
            : "https://github.com/mike-lischke/antlr-ng.git",
        commit: testCase.implementation === "java-antlr"
            ? JAVA_COMMIT
            : ANTLR_NG_COMMIT,
        path: testCase.source.path,
        case: testCase.name,
        source_sha256: testCase.source.sha256,
    };
}

function extractMethodFiles(body) {
    const bindings = new Map();
    const sources = new Map();
    const masked = maskCode(body);
    const eventPattern =
        /(?:\b(?:String\s+)?(?<binding>[A-Za-z_]\w*)\s*=)|(?:\bwriteFile\s*\()/gu;
    for (const match of masked.matchAll(eventPattern)) {
        if (match.groups.binding !== undefined) {
            const equals = masked.indexOf("=", match.index);
            const semicolon = masked.indexOf(";", equals);
            if (semicolon < 0) {
                throw new Error(
                    `unterminated assignment ${match.groups.binding}`,
                );
            }
            const expression = body.slice(equals + 1, semicolon);
            const value = evaluateString(expression, bindings);
            if (value !== undefined) {
                bindings.set(match.groups.binding, value);
            }
            continue;
        }
        const open = masked.indexOf("(", match.index);
        const close = matchingDelimiter(masked, open, "(", ")");
        const callArguments = splitTopLevel(
            body.slice(open + 1, close),
            masked.slice(open + 1, close),
        );
        if (callArguments.length !== 3) {
            throw new Error(
                `writeFile expected 3 arguments, found ${callArguments.length}`,
            );
        }
        const directory = fixtureDirectory(callArguments[0]);
        const fileName = evaluateString(callArguments[1], bindings);
        const source = evaluateString(callArguments[2], bindings);
        if (fileName === undefined || source === undefined) {
            throw new Error(
                `cannot evaluate writeFile(${callArguments.join(", ")})`,
            );
        }
        sources.set(
            directory.length === 0 ? fileName : `${directory}/${fileName}`,
            source,
        );
    }
    return { bindings, sources };
}

function fixtureDirectory(expression) {
    const name = expression.trim();
    if (name === "tempDirPath") {
        return "";
    }
    if (name === "subdir") {
        return "sub";
    }
    if (name === "outdir") {
        return "out";
    }
    throw new Error(`unknown fixture directory expression ${expression}`);
}

function evaluateString(expression, bindings) {
    const loadMatch = /^\s*load\("Java\.g4"\)\s*$/u.exec(expression);
    if (loadMatch !== null) {
        return javaGrammar;
    }
    const name = expression.trim();
    if (/^[A-Za-z_]\w*$/u.test(name)) {
        return bindings.get(name);
    }
    const literals =
        stripComments(expression).match(/"(?:\\.|[^"\\])*"/gu) ?? [];
    if (literals.length === 0) {
        return undefined;
    }
    return literals.map(decodeString).join("");
}

async function updateFixture(definition) {
    const directory = resolve(fixturesRoot, definition.fixtureName);
    await rm(directory, { recursive: true, force: true });
    await mkdir(directory, { recursive: true });
    for (const [path, source] of definition.files) {
        const destination = resolve(directory, path);
        await mkdir(dirname(destination), { recursive: true });
        await writeFile(destination, source, "utf8");
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
    for (const [path, source] of definition.files) {
        await expectFile(resolve(directory, path), source);
    }
    const manifest = JSON.parse(
        await readFile(resolve(directory, "fixture.json"), "utf8"),
    );
    for (const key of [
        "schema_version",
        "roots",
        "library_directories",
        "logical_ids",
        "upstream_tests",
        "java_antlr_test",
        "fixture_binding",
        "expected",
        "composite_grammars_oracle",
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
    const actual = await oracleDiagnostics(definition);
    if (
        definition.manifest.expected === "success"
        && actual.some((diagnostic) => diagnostic.severity === "error")
    ) {
        throw new Error(
            `${definition.fixtureName} Java emitted an unexpected error: ` +
                JSON.stringify(actual),
        );
    }
    let searchFrom = 0;
    for (const expected of definition.expectedDiagnostics) {
        const index = actual.findIndex(
            (diagnostic, candidateIndex) =>
                candidateIndex >= searchFrom
                && diagnostic.severity === expected.severity
                && diagnostic.code === expected.javaCode,
        );
        if (index < 0) {
            throw new Error(
                `${definition.fixtureName} is missing the pinned ` +
                    `${expected.severity}(${expected.javaCode}) assertion: ` +
                    JSON.stringify(actual),
            );
        }
        searchFrom = index + 1;
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
                    && entry.name.startsWith("testcompositegrammars-"),
            )
            .map((entry) => entry.name),
    );
    const missing = [...expected].filter((name) => !actual.has(name));
    const stale = [...actual].filter((name) => !expected.has(name));
    if (missing.length > 0 || stale.length > 0) {
        throw new Error(
            `TestCompositeGrammars fixture set differs; ` +
                `missing=${missing.join(",")}; stale=${stale.join(",")}`,
        );
    }
}

async function updateOrCheckCases(definitionsToWrite) {
    const lines = [
        "// Generated by tools/grammar-frontend/generate-composite-grammars-fixtures.mjs.",
        "// Inputs and diagnostics come only from Java ANTLR 4.13.2.",
        "",
    ];
    for (const definition of definitionsToWrite) {
        const diagnostics = await oracleDiagnostics(definition);
        const artifacts = await rootArtifacts(definition);
        lines.push(
            "case!(",
            `    ${definition.fixtureName.replaceAll("-", "_")},`,
            `    ${rustString(definition.fixtureName)},`,
            `    ${rustStringArray(definition.roots)},`,
            `    ${rustStringArray(definition.libraryDirectories)},`,
            `    ${definition.manifest.expected === "error"},`,
            `    ${rustStringArray(definition.sourceOrder)},`,
            "    [",
        );
        for (const diagnostic of diagnostics) {
            if (
                diagnostic.file === undefined
                || diagnostic.line === undefined
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
                        `${rustString(diagnostic.file)}, ` +
                        `${diagnostic.line}, ${diagnostic.column}),`,
                );
            }
        }
        lines.push("    ],", "    [");
        for (const artifact of artifacts) {
            lines.push(`        ${artifact},`);
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

async function oracleDiagnostics(definition) {
    const directory = resolve(fixturesRoot, definition.fixtureName);
    const outputs = await Promise.all(
        ["oracle/java-antlr.stdout", "oracle/java-antlr.stderr"].map(
            async (path) => readFile(resolve(directory, path), "utf8"),
        ),
    );
    const diagnostics = [];
    const pattern =
        /^(?<severity>warning|error)\((?<code>\d+)\): (?:(?<file>.*?):(?<line>\d+):(?<column>\d+): )?(?<message>.*)$/gmu;
    for (const match of outputs.join("\n").matchAll(pattern)) {
        const diagnostic = {
            severity: match.groups.severity,
            code: Number.parseInt(match.groups.code, 10),
            message: match.groups.message,
        };
        if (match.groups.file !== undefined) {
            diagnostic.file = normalizeOracleFile(
                match.groups.file,
                definition.files,
            );
            diagnostic.line = Number.parseInt(match.groups.line, 10);
            diagnostic.column = Number.parseInt(match.groups.column, 10);
        }
        diagnostics.push(diagnostic);
    }
    return diagnostics;
}

function normalizeOracleFile(file, files) {
    const normalized = file.replaceAll("\\", "/");
    const matches = [...files.keys()].filter(
        (path) =>
            normalized === path
            || normalized.endsWith(`/${path}`),
    );
    if (matches.length === 1) {
        return matches[0];
    }
    const base = normalized.slice(normalized.lastIndexOf("/") + 1);
    const baseMatches = [...files.keys()].filter(
        (path) => path === base || path.endsWith(`/${base}`),
    );
    if (baseMatches.length === 1) {
        return baseMatches[0];
    }
    throw new Error(`cannot normalize Java diagnostic file ${file}`);
}

async function rootArtifacts(definition) {
    if (definition.manifest.expected === "error") {
        return [];
    }
    const directory = resolve(fixturesRoot, definition.fixtureName);
    const artifacts = [];
    for (const root of definition.roots) {
        const source = definition.files.get(root);
        const declaration = grammarDeclaration(source);
        if (declaration.kind === "lexer") {
            await requireArtifacts(
                directory,
                [`${declaration.name}.interp`, `${declaration.name}.tokens`],
            );
            artifacts.push(
                `lexer(${rustString(declaration.name)}, ` +
                    `${rustString(`${declaration.name}.interp`)}, ` +
                    `${rustString(`${declaration.name}.tokens`)})`,
            );
        } else if (declaration.kind === "parser") {
            await requireArtifacts(
                directory,
                [`${declaration.name}.interp`, `${declaration.name}.tokens`],
            );
            artifacts.push(
                `parser(${rustString(declaration.name)}, ` +
                    `${rustString(`${declaration.name}.interp`)}, ` +
                    `${rustString(`${declaration.name}.tokens`)})`,
            );
        } else {
            const parserInterp = `${declaration.name}.interp`;
            const parserTokens = `${declaration.name}.tokens`;
            await requireArtifacts(
                directory,
                [parserInterp, parserTokens],
            );
            artifacts.push(
                `parser(${rustString(`${declaration.name}Parser`)}, ` +
                    `${rustString(parserInterp)}, ` +
                    `${rustString(parserTokens)})`,
            );
            const lexerInterp = `${declaration.name}Lexer.interp`;
            try {
                await readFile(resolve(directory, lexerInterp));
                const lexerTokens = `${declaration.name}Lexer.tokens`;
                await requireArtifacts(directory, [lexerTokens]);
                artifacts.push(
                    `lexer(${rustString(`${declaration.name}Lexer`)}, ` +
                        `${rustString(lexerInterp)}, ` +
                        `${rustString(lexerTokens)})`,
                );
            } catch (error) {
                if (error.code !== "ENOENT") {
                    throw error;
                }
            }
        }
    }
    return artifacts;
}

async function requireArtifacts(directory, paths) {
    for (const path of paths) {
        await readFile(resolve(directory, path));
    }
}

function sourceOrder(files, roots, libraryDirectories) {
    const order = [];
    const loaded = new Set();
    for (const root of roots) {
        add(root);
    }
    for (const root of roots) {
        visit(root);
    }
    return order;

    function add(path) {
        if (!loaded.has(path)) {
            loaded.add(path);
            order.push(path);
        }
    }

    function visit(path) {
        const source = files.get(path);
        if (source === undefined) {
            return;
        }
        for (const name of grammarImports(source)) {
            const parent = path.includes("/")
                ? path.slice(0, path.lastIndexOf("/"))
                : "";
            const candidates = [
                parent.length === 0
                    ? `${name}.g4`
                    : `${parent}/${name}.g4`,
                ...libraryDirectories.map(
                    (directory) => `${directory}/${name}.g4`,
                ),
            ];
            const imported = candidates.find((candidate) =>
                files.has(candidate)
            );
            if (imported === undefined || loaded.has(imported)) {
                continue;
            }
            add(imported);
            visit(imported);
        }
    }
}

function grammarImports(source) {
    const match = /\bimport\s+(?<imports>[^;]+);/u.exec(stripComments(source));
    if (match === null) {
        return [];
    }
    return match.groups.imports
        .split(",")
        .map((entry) => entry.trim())
        .map((entry) => entry.split(/\s*=\s*/u).at(-1))
        .filter(Boolean);
}

function grammarDeclaration(grammar) {
    const source = stripComments(grammar);
    const match =
        /^\s*(?:(?<kind>lexer|parser)\s+)?grammar\s+(?<name>[A-Za-z_]\w*)\s*;/u.exec(
            source,
        );
    if (match === null) {
        throw new Error(
            `cannot read grammar declaration: ${grammar.slice(0, 80)}`,
        );
    }
    return {
        kind: match.groups.kind ?? "combined",
        name: match.groups.name,
    };
}

function parseJavaErrorTypes(text) {
    const result = new Map();
    const pattern =
        /^\s*(?<name>[A-Z][A-Z0-9_]*)\((?<code>\d+),.*?ErrorSeverity\.(?<severity>WARNING|WARNING_ONE_OFF|ERROR|ERROR_ONE_OFF|FATAL)\)/gmu;
    for (const match of text.matchAll(pattern)) {
        result.set(match.groups.name, {
            code: Number.parseInt(match.groups.code, 10),
            severity: match.groups.severity.startsWith("WARNING")
                ? "warning"
                : "error",
        });
    }
    return result;
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

function rustString(value) {
    return JSON.stringify(value)
        .replaceAll("\\b", "\\x08")
        .replaceAll("\\f", "\\x0c");
}

function rustStringArray(values) {
    return `[${values.map(rustString).join(", ")}]`;
}

function rustSeverity(severity) {
    return severity === "error" ? "Error" : "Warning";
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
            "usage: generate-composite-grammars-fixtures.mjs " +
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
        maxBuffer: 64 * 1024 * 1024,
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
