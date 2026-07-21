#!/usr/bin/env node

import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { readFile, realpath, writeFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const JAVA_REPOSITORY = "https://github.com/antlr/antlr4.git";
const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const ANTLR_NG_REPOSITORY = "https://github.com/mike-lischke/antlr-ng.git";
const ANTLR_NG_COMMIT = "1f68422ae4bfc62f93343769e144d01f305487b1";
const JAVA_SOURCE_PREFIX = "tool-testsuite/test/org/antlr/v4/test/tool";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const inventoryPath = resolve(
    repoRoot,
    "tests/codegen-direct/upstream-case-inventory.json",
);
const options = parseArguments(process.argv.slice(2));

verifyCommit(options.javaRoot, JAVA_COMMIT, "Java ANTLR");
verifyCommit(options.antlrNgRoot, ANTLR_NG_COMMIT, "antlr-ng");

const java = await inventoryJava(options.javaRoot);
const antlrNg = await inventoryAntlrNg(
    options.antlrNgRoot,
    options.vitestReport,
);
const cases = [...java.cases, ...antlrNg.cases];
const caseIds = new Set();
for (const testCase of cases) {
    if (caseIds.has(testCase.id)) {
        throw new Error(`duplicate source-case ID: ${testCase.id}`);
    }
    caseIds.add(testCase.id);
}

const inventory = {
    schema_version: 1,
    generated_by: "tools/grammar-frontend/inventory-upstream-tests.mjs",
    sources: {
        java_antlr: {
            repository: JAVA_REPOSITORY,
            commit: JAVA_COMMIT,
            runner_command:
                "mvn -q -pl tool-testsuite -am -DskipTests install && mvn -q -pl tool-testsuite test",
            source_file_count: java.sourceFileCount,
            runner_report_file_count: java.reportFileCount,
            case_count: java.cases.length,
            disabled_case_count: java.disabledCount,
            runner_discovery_sha256: digestJson(java.discovery),
            java_runtime: java.javaRuntime,
        },
        antlr_ng: {
            repository: ANTLR_NG_REPOSITORY,
            commit: ANTLR_NG_COMMIT,
            runner_command:
                "vitest --run --reporter=json --outputFile=<report.json>",
            source_file_count: antlrNg.sourceFileCount,
            direct_call_site_count: antlrNg.directCallCount,
            table_call_site_count: antlrNg.tableCallCount,
            case_count: antlrNg.cases.length,
            skipped_case_count: antlrNg.skippedCount,
            runner_discovery_sha256: digestJson(antlrNg.discovery),
            node_version: process.version,
        },
    },
    case_count: cases.length,
    cases,
};
const serialized = `${JSON.stringify(inventory, null, 2)}\n`;

if (options.update) {
    await writeFile(inventoryPath, serialized, "utf8");
    console.log(
        `updated upstream inventory: ${java.cases.length} Java + ${antlrNg.cases.length} antlr-ng cases`,
    );
} else {
    const checkedIn = await readFile(inventoryPath, "utf8");
    if (checkedIn !== serialized) {
        throw new Error(
            "upstream-case-inventory.json differs from pinned source and runner discovery",
        );
    }
    console.log(
        `verified upstream inventory: ${java.cases.length} Java + ${antlrNg.cases.length} antlr-ng cases`,
    );
}

async function inventoryJava(javaRoot) {
    const sourceDirectory = resolve(javaRoot, JAVA_SOURCE_PREFIX);
    const sourceFiles = gitLines(javaRoot, [
        "ls-tree",
        "-r",
        "--name-only",
        JAVA_COMMIT,
        "--",
        JAVA_SOURCE_PREFIX,
    ]).filter((path) => /\/Test[^/]+\.java$/u.test(path));
    if (sourceFiles.length !== 46) {
        throw new Error(`expected 46 Java tool test files, found ${sourceFiles.length}`);
    }

    const sourceByClass = new Map();
    for (const path of sourceFiles) {
        const text = gitText(javaRoot, ["show", `${JAVA_COMMIT}:${path}`]);
        const className = /\/(?<name>Test[^/]+)\.java$/u.exec(path)?.groups?.name;
        const methods = extractJavaTestMethods(text);
        const parent = /\bclass\s+\w+\s+extends\s+(?<parent>\w+)/u.exec(
            maskJava(text),
        )?.groups?.parent;
        sourceByClass.set(className, {
            path,
            text,
            sha256: digest(text),
            methods,
            parent,
        });
    }

    const reportDirectory = resolve(
        javaRoot,
        "tool-testsuite/target/surefire-reports",
    );
    const helper = resolve(scriptDir, "read-junit-report.py");
    const report = runJson("python3", [helper, reportDirectory]);
    if (report.report_file_count !== 46) {
        throw new Error(
            `expected 46 JUnit report files, found ${report.report_file_count}`,
        );
    }
    if (report.cases.length !== 690) {
        throw new Error(`expected 690 JUnit cases, found ${report.cases.length}`);
    }
    const failedCases = report.cases.filter(
        (runnerCase) => !["passed", "skipped"].includes(runnerCase.status),
    );
    if (failedCases.length > 0) {
        const first = failedCases[0];
        throw new Error(
            `JUnit report contains ${failedCases.length} failed/error case(s); first: ${first.classname}.${first.name} (${first.status})`,
        );
    }

    const runnerKeys = new Set();
    const identityCounts = new Map();
    const cases = report.cases.map((runnerCase) => {
        const suite = runnerCase.classname.split(".").at(-1);
        const display = parseJavaDisplayName(runnerCase.name);
        const methodName = display.method;
        const located = locateJavaMethod(sourceByClass, suite, methodName);
        if (!located) {
            throw new Error(`cannot locate Java test source: ${suite}.${methodName}`);
        }
        runnerKeys.add(`${suite}:${methodName}`);
        const identity = `${suite}/${runnerCase.name}`;
        const occurrence = increment(identityCounts, identity);
        return {
            id: stableId("java-antlr", identity, occurrence),
            implementation: "java-antlr",
            suite,
            name: methodName,
            parameters:
                display.index === null && display.qualifier === null
                    ? null
                    : {
                          qualifier: display.qualifier,
                          index: display.index,
                      },
            status: runnerCase.status === "skipped" ? "disabled" : "enabled",
            case_kind: display.index !== null
                ? "parameterized"
                : display.qualifier !== null
                  ? "injected"
                : located.declaringClass === suite
                  ? "test"
                  : "inherited",
            source: {
                path: located.source.path,
                line: located.method.line,
                sha256: located.source.sha256,
                declaring_suite: located.declaringClass,
            },
            runner: {
                status: runnerCase.status,
                skip_reason: runnerCase.skip_reason,
            },
        };
    });

    for (const [className, source] of sourceByClass) {
        for (const method of source.methods) {
            const expectedClass =
                source.parent && sourceByClass.has(source.parent)
                    ? className
                    : className;
            if (!runnerKeys.has(`${expectedClass}:${method.name}`)) {
                const inheritedBy = [...sourceByClass.entries()].some(
                    ([child, candidate]) =>
                        candidate.parent === className &&
                        runnerKeys.has(`${child}:${method.name}`),
                );
                if (!inheritedBy) {
                    throw new Error(
                        `Java source test was not discovered by JUnit: ${className}.${method.name}`,
                    );
                }
            }
        }
    }

    cases.sort(compareCases);
    return {
        cases,
        sourceFileCount: sourceFiles.length,
        reportFileCount: report.report_file_count,
        disabledCount: cases.filter((testCase) => testCase.status === "disabled")
            .length,
        javaRuntime: report.java_runtime,
        discovery: cases.map(discoveryIdentity),
        sourceDirectory,
    };
}

async function inventoryAntlrNg(antlrNgRoot, vitestReportPath) {
    const realRoot = await realpath(antlrNgRoot);
    const report = JSON.parse(await readFile(vitestReportPath, "utf8"));
    const testResults = report.testResults.filter((result) => {
        const normalized = result.name.replaceAll("\\", "/");
        return (
            /\/tests\/Test[^/]+\.spec\.ts$/u.test(normalized) ||
            normalized.endsWith("/tests/bugs/General.spec.ts")
        );
    });
    if (testResults.length !== 43) {
        throw new Error(`expected 43 Vitest source files, found ${testResults.length}`);
    }

    const require = createRequire(resolve(realRoot, "package.json"));
    const ts = require(require.resolve("typescript"));
    const cases = [];
    const identityCounts = new Map();
    const sourceCatalog = new Map();
    const reportPaths = new Map();
    for (const result of testResults) {
        const absolutePath = await realpath(result.name);
        const sourcePath = relative(realRoot, absolutePath).replaceAll("\\", "/");
        reportPaths.set(result, sourcePath);
        const sourceText = gitText(antlrNgRoot, [
            "show",
            `${ANTLR_NG_COMMIT}:${sourcePath}`,
        ]);
        const callSites = extractVitestCalls(ts, sourcePath, sourceText);
        const suite = sourcePath.endsWith("/bugs/General.spec.ts")
            ? "General"
            : /\/(?<suite>Test[^/]+)\.spec\.ts$/u.exec(sourcePath)?.groups?.suite;
        sourceCatalog.set(suite, {
            path: sourcePath,
            sha256: digest(sourceText),
            callSites,
        });
    }

    const matchedCallSites = new Set();
    for (const result of testResults) {
        const reportPath = reportPaths.get(result);
        for (const assertion of result.assertionResults) {
            const suite = assertion.ancestorTitles.at(-1) ?? reportPath;
            const source = sourceCatalog.get(suite);
            const site = source
                ? matchVitestCall(source.callSites, assertion.title)
                : null;
            if (!site) {
                throw new Error(
                    `cannot reconcile Vitest case ${reportPath}: ${suite} ${assertion.title}`,
                );
            }
            matchedCallSites.add(`${source.path}:${site.key}`);
            const identity = `${suite}/${assertion.title}`;
            const occurrence = increment(identityCounts, identity);
            const disabled = ["pending", "skipped", "todo"].includes(
                assertion.status,
            );
            if (!disabled && assertion.status !== "passed") {
                throw new Error(
                    `Vitest report contains a failed assertion: ${reportPath}: ${assertion.title} (${assertion.status})`,
                );
            }
            cases.push({
                id: stableId("antlr-ng", identity, occurrence),
                implementation: "antlr-ng",
                suite,
                name: assertion.title,
                parameters:
                    site.kind === "table" ? { rendered_title: assertion.title } : null,
                status: disabled ? "disabled" : "enabled",
                case_kind:
                    source.path !== reportPath
                        ? "imported"
                        : site.kind === "table"
                        ? "table"
                        : disabled
                          ? "skipped"
                          : "test",
                source: {
                    path: source.path,
                    line: site.line,
                    sha256: source.sha256,
                    declaring_suite: suite,
                },
                runner: {
                    status: assertion.status,
                    report_path: reportPath,
                    skip_reason: disabled ? "declared with it.skip" : null,
                },
            });
        }
    }
    for (const source of sourceCatalog.values()) {
        for (const site of source.callSites) {
            if (!matchedCallSites.has(`${source.path}:${site.key}`)) {
                throw new Error(
                    `Vitest source call was not discovered: ${source.path}:${site.line} ${site.title}`,
                );
            }
        }
    }

    if (cases.length !== 775) {
        throw new Error(`expected 775 Vitest cases, found ${cases.length}`);
    }
    cases.sort(compareCases);
    return {
        cases,
        sourceFileCount: testResults.length,
        directCallCount: [...sourceCatalog.values()]
            .flatMap((source) => source.callSites)
            .filter((site) => site.kind === "test").length,
        tableCallCount: [...sourceCatalog.values()]
            .flatMap((source) => source.callSites)
            .filter((site) => site.kind === "table").length,
        skippedCount: cases.filter((testCase) => testCase.status === "disabled")
            .length,
        discovery: cases.map(discoveryIdentity),
    };
}

function extractJavaTestMethods(text) {
    const masked = maskJava(text);
    const pattern =
        /@(?<kind>ParameterizedTest|Test)\b[\s\S]{0,600}?\b(?:public|protected|private)?\s*(?:static\s+)?void\s+(?<name>[$\w]+)\s*\(/gu;
    const methods = [];
    for (const match of masked.matchAll(pattern)) {
        methods.push({
            name: match.groups.name,
            kind:
                match.groups.kind === "ParameterizedTest"
                    ? "parameterized"
                    : "test",
            line: lineNumber(masked, match.index),
        });
    }
    return methods;
}

function maskJava(text) {
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
            } else if (current === '"') {
                output[index] = " ";
                state = "string";
            } else if (current === "'") {
                output[index] = " ";
                state = "character";
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
        } else {
            if (current === "\\") {
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
    }
    return output.join("");
}

function locateJavaMethod(sourceByClass, suite, methodName) {
    let className = suite;
    const visited = new Set();
    while (className && !visited.has(className)) {
        visited.add(className);
        const source = sourceByClass.get(className);
        const method = source?.methods.find((candidate) => candidate.name === methodName);
        if (source && method) {
            return { source, method, declaringClass: className };
        }
        className = source?.parent;
    }
    return null;
}

function extractVitestCalls(ts, path, text) {
    const source = ts.createSourceFile(
        path,
        text,
        ts.ScriptTarget.Latest,
        true,
        ts.ScriptKind.TS,
    );
    const calls = [];
    const visit = (node) => {
        if (ts.isCallExpression(node)) {
            const direct = directVitestKind(ts, node.expression);
            const table =
                ts.isCallExpression(node.expression) &&
                tableVitestKind(ts, node.expression.expression);
            const kind = direct ? "test" : table ? "table" : null;
            const titleNode = kind ? node.arguments[0] : null;
            if (
                titleNode &&
                (ts.isStringLiteral(titleNode) ||
                    ts.isNoSubstitutionTemplateLiteral(titleNode))
            ) {
                const line =
                    source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1;
                calls.push({
                    key: `${line}:${kind}:${titleNode.text}`,
                    kind,
                    title: titleNode.text,
                    line,
                    skipped: direct === "skip" || table === "skip",
                });
            }
        }
        ts.forEachChild(node, visit);
    };
    visit(source);
    return calls;
}

function directVitestKind(ts, expression) {
    if (ts.isIdentifier(expression) && ["it", "test"].includes(expression.text)) {
        return "test";
    }
    if (
        ts.isPropertyAccessExpression(expression) &&
        ts.isIdentifier(expression.expression) &&
        ["it", "test"].includes(expression.expression.text) &&
        ["skip", "todo", "only"].includes(expression.name.text)
    ) {
        return expression.name.text === "skip" ? "skip" : "test";
    }
    return null;
}

function tableVitestKind(ts, expression) {
    if (
        ts.isPropertyAccessExpression(expression) &&
        ts.isIdentifier(expression.expression) &&
        ["it", "test"].includes(expression.expression.text) &&
        expression.name.text === "each"
    ) {
        return "table";
    }
    return null;
}

function matchVitestCall(callSites, renderedTitle) {
    const exact = callSites.find(
        (site) => site.kind === "test" && site.title === renderedTitle,
    );
    if (exact) {
        return exact;
    }
    const tableCandidates = callSites.filter((site) => {
        if (site.kind !== "table") {
            return false;
        }
        const prefix = site.title.split(/[%$]/u, 1)[0];
        return prefix.length === 0 || renderedTitle.startsWith(prefix);
    });
    return tableCandidates.length === 1 ? tableCandidates[0] : null;
}

function parseJavaDisplayName(name) {
    const match =
        /^(?<method>[^{}]+)\{(?<qualifier>[^}]+)\}(?:\[(?<index>\d+)\])?$/u.exec(
            name,
        );
    return match
        ? {
              method: match.groups.method,
              qualifier: match.groups.qualifier,
              index:
                  match.groups.index === undefined
                      ? null
                      : Number(match.groups.index),
          }
        : { method: name, qualifier: null, index: null };
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
    const next = (counts.get(key) ?? 0) + 1;
    counts.set(key, next);
    return next;
}

function compareCases(left, right) {
    return (
        left.source.path.localeCompare(right.source.path) ||
        left.source.line - right.source.line ||
        left.id.localeCompare(right.id)
    );
}

function discoveryIdentity(testCase) {
    return {
        id: testCase.id,
        status: testCase.status,
        source: `${testCase.source.path}:${testCase.source.line}`,
    };
}

function verifyCommit(root, expected, label) {
    const actual = gitText(root, ["rev-parse", "HEAD"]).trim();
    if (actual !== expected) {
        throw new Error(`${label} checkout is ${actual}, expected ${expected}`);
    }
}

function gitLines(root, args) {
    return gitText(root, args).trim().split(/\r?\n/u).filter(Boolean);
}

function gitText(root, args) {
    const result = spawnSync("git", ["-C", root, ...args], {
        encoding: "utf8",
        maxBuffer: 32 * 1024 * 1024,
    });
    if (result.status !== 0) {
        throw new Error(`git ${args.join(" ")} failed: ${result.stderr}`);
    }
    return result.stdout;
}

function runJson(command, args) {
    const result = spawnSync(command, args, {
        encoding: "utf8",
        maxBuffer: 32 * 1024 * 1024,
    });
    if (result.status !== 0) {
        throw new Error(`${command} failed: ${result.stderr}`);
    }
    return JSON.parse(result.stdout);
}

function lineNumber(text, offset) {
    return text.slice(0, offset).split("\n").length;
}

function digest(text) {
    return createHash("sha256").update(text).digest("hex");
}

function digestJson(value) {
    return digest(JSON.stringify(value));
}

function parseArguments(args) {
    let update = false;
    let javaRoot;
    let antlrNgRoot;
    let vitestReport;
    for (let index = 0; index < args.length; index += 1) {
        switch (args[index]) {
            case "--check":
                update = false;
                break;
            case "--update":
                update = true;
                break;
            case "--java-root":
                javaRoot = args[++index];
                break;
            case "--antlr-ng-root":
                antlrNgRoot = args[++index];
                break;
            case "--vitest-report":
                vitestReport = args[++index];
                break;
            default:
                throw new Error(`unknown argument: ${args[index]}`);
        }
    }
    if (!javaRoot || !antlrNgRoot || !vitestReport) {
        throw new Error(
            "--java-root, --antlr-ng-root, and --vitest-report are required",
        );
    }
    return {
        update,
        javaRoot: resolve(javaRoot),
        antlrNgRoot: resolve(antlrNgRoot),
        vitestReport: resolve(vitestReport),
    };
}
