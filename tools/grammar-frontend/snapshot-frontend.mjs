#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const EXPECTED_ANTLR_NG =
    "1f68422ae4bfc62f93343769e144d01f305487b1";

const options = parseArguments(process.argv.slice(2));
const repo = resolve(options.repo ?? process.cwd());
const antlrNg = resolve(
    options["antlr-ng"] ?? "/tmp/antlr-cleanroom/antlr-ng-1f68422",
);
const corpusPath = resolve(
    repo,
    options.corpus ?? "tests/codegen-direct/frontend-corpus.json",
);
const outputPath = resolve(
    repo,
    options.output ?? "tests/codegen-direct/frontend-snapshots.tsv",
);

const corpus = JSON.parse(await readFile(corpusPath, "utf8"));
if (corpus.antlr_ng.commit !== EXPECTED_ANTLR_NG) {
    throw new Error(`unexpected corpus antlr-ng pin: ${corpus.antlr_ng.commit}`);
}

const antlr = await import(
    pathToFileURL(join(antlrNg, "node_modules/antlr4ng/dist/index.mjs")).href
);
const { ANTLRv4Lexer } = await import(
    pathToFileURL(join(antlrNg, "dist/src/generated/ANTLRv4Lexer.js")).href
);
const { ANTLRv4Parser } = await import(
    pathToFileURL(join(antlrNg, "dist/src/generated/ANTLRv4Parser.js")).href
);

const rows = [];
for (const testCase of corpus.cases) {
    const source = await readFile(resolve(repo, testCase.path), "utf8");
    const snapshot = snapshotSource(source, antlr, ANTLRv4Lexer, ANTLRv4Parser);
    rows.push({
        id: testCase.id,
        path: testCase.path,
        sourceSha256: sha256(source),
        ...snapshot,
    });
}

const header = [
    "id",
    "path",
    "source_sha256",
    "token_count",
    "token_fnv1a64",
    "cst_node_count",
    "cst_fnv1a64",
].join("\t");
const body = rows.map((row) => [
    row.id,
    row.path,
    row.sourceSha256,
    row.tokenCount,
    row.tokenHash,
    row.nodeCount,
    row.treeHash,
].join("\t"));
await writeFile(outputPath, `${header}\n${body.join("\n")}\n`);

function snapshotSource(source, runtime, LexerType, ParserType) {
    const lexerErrors = [];
    const lexer = new LexerType(runtime.CharStream.fromString(source));
    lexer.removeErrorListeners();
    lexer.addErrorListener(errorCollector(lexerErrors));

    const tokens = new runtime.CommonTokenStream(lexer);
    tokens.fill();
    if (lexerErrors.length > 0) {
        throw new Error(`lexer rejected source: ${JSON.stringify(lexerErrors)}`);
    }

    const tokenRows = [];
    const byteOffsets = scalarByteOffsets(source);
    for (const token of tokens.getTokens()) {
        if (token.type === runtime.Token.EOF) {
            continue;
        }
        const start = byteOffsets[token.start];
        const end = byteOffsets[token.stop + 1];
        if (start === undefined || end === undefined) {
            throw new Error(`invalid token scalar span ${token.start}..${token.stop}`);
        }
        tokenRows.push(
            `${token.type}\t${token.channel}\t${start}\t${end}\t${JSON.stringify(token.text ?? "")}\n`,
        );
    }

    const parserErrors = [];
    const parser = new ParserType(tokens);
    parser.removeErrorListeners();
    parser.addErrorListener(errorCollector(parserErrors));
    const tree = parser.grammarSpec();
    if (parser.numberOfSyntaxErrors > 0 || parserErrors.length > 0) {
        throw new Error(`parser rejected source: ${JSON.stringify(parserErrors)}`);
    }

    const treeRows = [];
    let nodeCount = 0;
    visitTree(tree, treeRows, () => {
        nodeCount += 1;
    });
    return {
        tokenCount: tokenRows.length,
        tokenHash: fnv1a64(tokenRows),
        nodeCount,
        treeHash: fnv1a64(treeRows),
    };
}

function visitTree(node, output, count) {
    count();
    const childCount = node.getChildCount();
    const payload = node.getPayload();
    if (typeof payload?.ruleIndex === "number") {
        output.push(`R\t${payload.ruleIndex}\t${childCount}\n`);
    } else {
        const token = node.symbol ?? payload;
        const prefix = node.constructor?.name === "ErrorNode" ? "E" : "T";
        output.push(`${prefix}\t${token.type}\t${JSON.stringify(token.text ?? "")}\n`);
    }
    for (let index = 0; index < childCount; index += 1) {
        const child = node.getChild(index);
        if (child === null) {
            throw new Error("parse tree contained a null child");
        }
        visitTree(child, output, count);
    }
}

function errorCollector(errors) {
    return {
        syntaxError(_recognizer, _symbol, line, column, message) {
            errors.push({ line, column, message });
        },
        reportAmbiguity() {},
        reportAttemptingFullContext() {},
        reportContextSensitivity() {},
    };
}

function scalarByteOffsets(source) {
    const result = [0];
    let offset = 0;
    for (const scalar of source) {
        offset += Buffer.byteLength(scalar);
        result.push(offset);
    }
    return result;
}

function sha256(value) {
    return createHash("sha256").update(value).digest("hex");
}

function fnv1a64(chunks) {
    let hash = 0xcbf29ce484222325n;
    for (const chunk of chunks) {
        for (const byte of Buffer.from(chunk)) {
            hash ^= BigInt(byte);
            hash = BigInt.asUintN(64, hash * 0x100000001b3n);
        }
    }
    return hash.toString(16).padStart(16, "0");
}

function parseArguments(argumentsList) {
    const result = {};
    for (let index = 0; index < argumentsList.length; index += 1) {
        const argument = argumentsList[index];
        if (!argument.startsWith("--")) {
            throw new Error(`unexpected argument: ${argument}`);
        }
        const name = argument.slice(2);
        const value = argumentsList[index + 1];
        if (value === undefined || value.startsWith("--")) {
            throw new Error(`missing value for ${argument}`);
        }
        result[name] = value;
        index += 1;
    }
    return result;
}
