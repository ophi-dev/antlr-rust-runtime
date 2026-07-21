#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
    mkdir,
    readFile,
    readdir,
    rm,
    stat,
    writeFile,
} from "node:fs/promises";
import { dirname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const ANTLR_NG_COMMIT = "1f68422ae4bfc62f93343769e144d01f305487b1";
const JAVA_PATH =
    "tool-testsuite/test/org/antlr/v4/test/tool/TestATNConstruction.java";
const ANTLR_NG_PATH = "tests/TestATNConstruction.spec.ts";
const JAVA_REPOSITORY = "https://github.com/antlr/antlr4.git";
const ANTLR_NG_REPOSITORY = "https://github.com/mike-lischke/antlr-ng.git";
const JAVA_ALIAS = new Map([
    ["testLexerIsNotSetMultiCharString", "testLexerIsntSetMultiCharString"],
]);
const JAVA_CLI_ERROR_METHODS = new Set([
    "testAorBorEmptyPlus",
    "testParserRuleRefInLexerRule",
]);

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const fixturesRoot = resolve(repositoryRoot, "tests/codegen-direct/fixtures");
const options = parseArguments(process.argv.slice(2));

verifyCommit(options.javaRoot, JAVA_COMMIT, "Java ANTLR");
verifyCommit(options.antlrNgRoot, ANTLR_NG_COMMIT, "antlr-ng");

const testMap = await load("tests/codegen-direct/upstream-test-map.json");
const inventory = await load("tests/codegen-direct/upstream-case-inventory.json");
const inventoryById = new Map(inventory.cases.map((testCase) => [testCase.id, testCase]));
const rows = testMap.rows
    .filter((row) => row.logical_id.startsWith("testatnconstruction-"))
    .sort((left, right) => left.logical_id.localeCompare(right.logical_id));
if (rows.length !== 40) {
    throw new Error(`expected 40 TestATNConstruction rows, found ${rows.length}`);
}

const javaSource = gitText(options.javaRoot, `${JAVA_COMMIT}:${JAVA_PATH}`);
const antlrNgSource = gitText(
    options.antlrNgRoot,
    `${ANTLR_NG_COMMIT}:${ANTLR_NG_PATH}`,
);
const javaMethods = extractJavaMethods(javaSource);
const antlrNgMethods = extractAntlrNgMethods(antlrNgSource);

for (const row of rows) {
    const sourceCases = row.source_case_ids.map((id) => {
        const testCase = inventoryById.get(id);
        if (!testCase) {
            throw new Error(`${row.logical_id} references unknown source case ${id}`);
        }
        return testCase;
    });
    const javaCase = sourceCases.find(
        (testCase) => testCase.implementation === "java-antlr",
    );
    const antlrNgCase = sourceCases.find(
        (testCase) => testCase.implementation === "antlr-ng",
    );
    const javaMethodName =
        javaCase?.name ?? JAVA_ALIAS.get(antlrNgCase?.name);
    const javaMethod = javaMethods.get(javaMethodName);
    if (!javaMethod) {
        throw new Error(
            `${row.logical_id} cannot locate Java method ${javaMethodName}`,
        );
    }
    const antlrNgMethod = antlrNgCase
        ? antlrNgMethods.get(antlrNgCase.name)
        : null;
    if (antlrNgCase && !antlrNgMethod) {
        throw new Error(
            `${row.logical_id} cannot locate antlr-ng method ${antlrNgCase.name}`,
        );
    }

    const javaDefinition = extractDefinition(javaMethod);
    const antlrNgDefinition = antlrNgMethod
        ? extractDefinition(antlrNgMethod)
        : null;
    if (
        antlrNgDefinition
        && javaDefinition.grammar !== antlrNgDefinition.grammar
    ) {
        throw new Error(
            `${row.logical_id} Java and antlr-ng grammar inputs differ`,
        );
    }

    const fixtureDefinition = createFixtureDefinition(
        row,
        sourceCases,
        javaDefinition,
        antlrNgDefinition,
        javaCase,
        antlrNgCase,
    );
    if (options.update) {
        await updateFixture(row.logical_id, fixtureDefinition);
    } else {
        await checkFixture(row.logical_id, fixtureDefinition);
    }
}

console.log(
    `${options.update ? "updated" : "verified"} ${rows.length} TestATNConstruction fixtures`,
);

function createFixtureDefinition(
    row,
    sourceCases,
    javaDefinition,
    antlrNgDefinition,
    javaCase,
    antlrNgCase,
) {
    const grammarName = grammarDeclaration(javaDefinition.grammar).name;
    const root = `${grammarName}.g4`;
    const javaOracle = oracleFiles("java", javaDefinition);
    const antlrNgOracle = antlrNgDefinition
        ? oracleFiles("antlr-ng", antlrNgDefinition)
        : new Map();
    const agreement = compareDefinitions(javaDefinition, antlrNgDefinition);
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
    const manifest = {
        schema_version: 1,
        roots: [root],
        logical_ids: [row.logical_id],
        upstream_tests: sources,
        java_antlr_test:
            javaCase === undefined
                ? {
                      repository: JAVA_REPOSITORY,
                      commit: JAVA_COMMIT,
                      path: JAVA_PATH,
                      case: javaMethodNameForDefinition(javaDefinition),
                      role: "generated compatibility oracle for antlr-ng-only row",
                  }
                : sources.find(
                      (source) => source.source_case_id === javaCase.id,
                  ),
        antlr_ng_test:
            antlrNgCase === undefined
                ? null
                : sources.find(
                      (source) => source.source_case_id === antlrNgCase.id,
                  ),
        expected:
            javaDefinition.checks.length === 0
            || JAVA_CLI_ERROR_METHODS.has(javaDefinition.methodName)
                ? "error"
                : "success",
        atn_printer_oracle: {
            java_index:
                javaDefinition.checks.length === 0
                    ? null
                    : "oracle/java-atn.index",
            antlr_ng_index:
                antlrNgDefinition?.checks.length
                    ? "oracle/antlr-ng-atn.index"
                    : null,
            agreement,
            compatibility_verdict: "Java ANTLR 4.13.2",
        },
    };
    return {
        root,
        grammar: javaDefinition.grammar,
        manifest,
        files: new Map([...javaOracle, ...antlrNgOracle]),
    };
}

function oracleFiles(prefix, definition) {
    const files = new Map();
    if (definition.checks.length === 0) {
        if (definition.methodName === "testParserRuleRefInLexerRule") {
            files.set(
                `oracle/${prefix}-outcome.txt`,
                prefix === "java"
                    ? "PARSER_RULE_REF_IN_LEXER_RULE [a, A]\n"
                    : "syntax error containing no viable alternative at input 'a'\n",
            );
        }
        return files;
    }

    const declaration = grammarDeclaration(definition.grammar);
    const usedNames = new Map();
    const index = [];
    for (const check of definition.checks) {
        const base = `${check.target}-${slug(check.selector)}`;
        const occurrence = (usedNames.get(base) ?? 0) + 1;
        usedNames.set(base, occurrence);
        const suffix = occurrence === 1 ? "" : `-${occurrence}`;
        const fileName = `${prefix}-atn-${base}${suffix}.txt`;
        const recognizer =
            check.target === "mode"
                ? declaration.name
                : declaration.kind === "combined"
                  ? `${declaration.name}Parser`
                  : declaration.name;
        const interp =
            check.target === "mode"
                ? `${declaration.name}.interp`
                : `${declaration.name}.interp`;
        const kind = check.target === "mode" ? "lexer" : "parser";
        index.push(
            [kind, recognizer, interp, check.target, check.selector, fileName].join(
                "\t",
            ),
        );
        files.set(`oracle/${fileName}`, check.expected);
    }
    files.set(`oracle/${prefix}-atn.index`, `${index.join("\n")}\n`);
    if (definition.astStateMap !== null) {
        files.set(
            `oracle/${prefix}-ast-state-map.txt`,
            `${definition.astStateMap}\n`,
        );
    }
    return files;
}

function compareDefinitions(javaDefinition, antlrNgDefinition) {
    if (antlrNgDefinition === null) {
        return "not-applicable: no antlr-ng source case";
    }
    if (
        javaDefinition.checks.length === 0
        || antlrNgDefinition.checks.length === 0
    ) {
        return "divergent-error-surface: Java reaches semantic analysis; antlr-ng rejects during parsing";
    }
    if (javaDefinition.checks.length !== antlrNgDefinition.checks.length) {
        return "divergent: oracle check counts differ";
    }
    const identical = javaDefinition.checks.every((check, index) => {
        const alternate = antlrNgDefinition.checks[index];
        return (
            check.target === alternate.target
            && check.selector === alternate.selector
            && check.expected === alternate.expected
        );
    });
    return identical
        ? "identical"
        : "divergent: Java 4.13.2 is the compatibility verdict";
}

async function updateFixture(logicalId, definition) {
    const directory = resolve(fixturesRoot, logicalId);
    await rm(directory, { recursive: true, force: true });
    await mkdir(resolve(directory, "oracle"), { recursive: true });
    await writeFile(resolve(directory, definition.root), definition.grammar, "utf8");
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
            `fixture updater failed for ${logicalId} (${result.status}):\n${result.stdout}\n${result.stderr}`,
        );
    }
    process.stdout.write(result.stdout);
}

async function checkFixture(logicalId, definition) {
    const directory = resolve(fixturesRoot, logicalId);
    await expectFile(resolve(directory, definition.root), definition.grammar);
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
        "atn_printer_oracle",
    ]) {
        if (
            JSON.stringify(manifest[key]) !==
            JSON.stringify(definition.manifest[key])
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

function extractJavaMethods(text) {
    const methods = new Map();
    const masked = maskCode(text);
    const pattern =
        /@Test\s+public\s+void\s+(?<name>[$\w]+)\s*\([^)]*\)[^{]*\{/gu;
    for (const match of masked.matchAll(pattern)) {
        const open = masked.indexOf("{", match.index);
        const close = matchingDelimiter(masked, open, "{", "}");
        methods.set(match.groups.name, {
            methodName: match.groups.name,
            text: text.slice(open + 1, close),
        });
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
        methods.set(match.groups.name, {
            methodName: match.groups.name,
            text: text.slice(open + 1, close),
        });
    }
    return methods;
}

function extractDefinition(method) {
    const masked = maskCode(method.text);
    const constructor =
        /\bnew\s+(?:LexerGrammar|Grammar)\s*\(/gu.exec(masked);
    let grammar;
    if (constructor) {
        const open = masked.indexOf("(", constructor.index);
        const close = matchingDelimiter(masked, open, "(", ")");
        const argument = method.text.slice(open + 1, close).trim();
        grammar = /^[A-Za-z_$][\w$]*$/u.test(argument)
            ? stringVariable(method.text, argument, constructor.index)
            : javaStringExpression(argument);
    } else {
        const variable =
            method.methodName === "testParserRuleRefInLexerRule"
                ? /\b(?:gstr|grammarString)\b/u.exec(masked)?.[0]
                : null;
        if (!variable) {
            throw new Error(`${method.methodName} has no grammar constructor`);
        }
        grammar = stringVariable(method.text, variable, masked.length);
    }

    const assignments = [];
    const assignmentPattern =
        /(?:(?:String|const|let)\s+)?\bexpecting\s*=/gu;
    for (const match of masked.matchAll(assignmentPattern)) {
        const start = match.index + match[0].length;
        const end = masked.indexOf(";", start);
        if (end < 0) {
            throw new Error(`${method.methodName} has unterminated expectation`);
        }
        assignments.push({
            offset: match.index,
            expected: javaStringExpression(method.text.slice(start, end)),
        });
    }

    const checks = [];
    const callPattern =
        /\b(?<call>RuntimeTestUtils\.checkRuleATN|checkRuleATN|checkTokensRule)\s*\(\s*\w+\s*,\s*(?<selector>null|"(?:\\.|[^"\\])*")\s*,\s*expecting\s*\)/gu;
    for (const match of method.text.matchAll(callPattern)) {
        const assignment = assignments
            .filter((candidate) => candidate.offset < match.index)
            .at(-1);
        if (!assignment) {
            throw new Error(`${method.methodName} check has no expectation`);
        }
        const target = match.groups.call.endsWith("checkTokensRule")
            ? "mode"
            : "rule";
        const sourceSelector =
            match.groups.selector === "null"
                ? "DEFAULT_MODE"
                : javaStringExpression(match.groups.selector);
        checks.push({
            target,
            selector:
                target === "mode" && sourceSelector === ""
                    ? "DEFAULT_MODE"
                    : sourceSelector,
            expected: assignment.expected,
        });
    }

    const stateMapMatch =
        /assertEquals\(\s*("(?:\\.|[^"\\])*")\s*,\s*covered\.toString\(\)\s*\)/u.exec(
            method.text,
        );
    return {
        methodName: method.methodName,
        grammar,
        checks,
        astStateMap:
            stateMapMatch === null
                ? null
                : javaStringExpression(stateMapMatch[1]),
    };
}

function stringVariable(text, name, before) {
    const masked = maskCode(text);
    const pattern = new RegExp(
        `(?:(?:String|const|let)\\s+)?\\b${escapeRegExp(name)}\\s*=`,
        "gu",
    );
    const assignments = [...masked.matchAll(pattern)].filter(
        (match) => match.index < before,
    );
    const assignment = assignments.at(-1);
    if (!assignment) {
        throw new Error(`cannot find string variable ${name}`);
    }
    const start = assignment.index + assignment[0].length;
    const end = masked.indexOf(";", start);
    if (end < 0) {
        throw new Error(`unterminated string variable ${name}`);
    }
    return javaStringExpression(text.slice(start, end));
}

function javaStringExpression(expression) {
    const strings = expression.match(/"(?:\\.|[^"\\])*"/gu) ?? [];
    if (strings.length === 0) {
        throw new Error(`expression has no string literals: ${expression}`);
    }
    return strings.map(decodeJavaString).join("");
}

function decodeJavaString(literal) {
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
        } else if (escaped === "u") {
            while (literal[index + 1] === "u") {
                index += 1;
            }
            const digits = literal.slice(index + 1, index + 5);
            if (!/^[0-9a-f]{4}$/iu.test(digits)) {
                throw new Error(`invalid Java Unicode escape in ${literal}`);
            }
            result += String.fromCodePoint(Number.parseInt(digits, 16));
            index += 4;
        } else {
            throw new Error(`unsupported Java string escape \\${escaped}`);
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
        throw new Error(`cannot read grammar declaration: ${grammar.slice(0, 80)}`);
    }
    return {
        kind: match.groups.kind ?? "combined",
        name: match.groups.name,
    };
}

function javaMethodNameForDefinition(definition) {
    return definition.methodName;
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
            case "--java":
                result.java = requiredValue(args, ++index, argument);
                break;
            default:
                throw new Error(`unknown argument: ${argument}`);
        }
    }
    if (result.update === null) {
        throw new Error(
            "usage: generate-atn-construction-fixtures.mjs --check|--update",
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
            `${label} root must be at ${expected}; found ${result.stdout.trim() || result.stderr.trim()}`,
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
    return createHash("sha256").update(await readFile(path)).digest("hex");
}

function slug(value) {
    return value
        .normalize("NFKD")
        .toLowerCase()
        .replaceAll(/[^a-z0-9]+/gu, "-")
        .replaceAll(/^-|-$/gu, "");
}

function escapeRegExp(value) {
    return value.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}
