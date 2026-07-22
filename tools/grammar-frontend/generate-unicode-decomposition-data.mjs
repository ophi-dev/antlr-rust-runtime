#!/usr/bin/env node

import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const MAGIC = Buffer.from("ANTLRDT1", "ascii");
const UNICODE_VERSION = [17, 0, 0];
const EXPECTED_SOURCE_HASH =
    "f44e5ceaf40edc1fe06ea0404e8bebc7d356dcc38aac076543b6874008a06e3e";
const EXPECTED_RECORD_COUNT = 561;
const DECOMPOSITION_TYPES = new Map([
    ["Compat", 2],
    ["Circle", 3],
    ["Final", 4],
    ["Font", 5],
    ["Fraction", 6],
    ["Initial", 7],
    ["Isolated", 8],
    ["Medial", 9],
    ["Narrow", 10],
    ["Nobreak", 11],
    ["Small", 12],
    ["Square", 13],
    ["Sub", 14],
    ["Super", 15],
    ["Vertical", 16],
    ["Wide", 17],
]);

const [inputArgument, outputArgument] = process.argv.slice(2);
if (inputArgument === undefined || outputArgument === undefined) {
    throw new Error(
        "usage: generate-unicode-decomposition-data.mjs " +
            "<DerivedDecompositionType.txt> <output.bin>",
    );
}

const inputPath = resolve(inputArgument);
const outputPath = resolve(outputArgument);
const source = await readFile(inputPath);
const sourceHash = createHash("sha256").update(source).digest("hex");
if (sourceHash !== EXPECTED_SOURCE_HASH) {
    throw new Error(
        `expected Unicode 17 DerivedDecompositionType.txt hash ` +
            `${EXPECTED_SOURCE_HASH}, found ${sourceHash}`,
    );
}

const sourceRecords = [];
for (const [lineIndex, line] of source.toString("utf8").split("\n").entries()) {
    const content = line.replace(/#.*/u, "").trim();
    if (content.length === 0) {
        continue;
    }
    const match =
        /^(?<start>[0-9A-F]{4,6})(?:\.\.(?<stop>[0-9A-F]{4,6}))?\s*;\s*(?<type>[A-Za-z]+)$/u.exec(
            content,
        );
    if (match === null) {
        throw new Error(`invalid UCD line ${lineIndex + 1}: ${line}`);
    }
    if (match.groups.type === "Canonical") {
        continue;
    }
    const decompositionType = DECOMPOSITION_TYPES.get(match.groups.type);
    if (decompositionType === undefined) {
        throw new Error(
            `unknown decomposition type ${match.groups.type} on line ${lineIndex + 1}`,
        );
    }
    const start = Number.parseInt(match.groups.start, 16);
    const stop = Number.parseInt(match.groups.stop ?? match.groups.start, 16);
    sourceRecords.push({ decompositionType, start, stop });
}

const records = [];
for (const decompositionType of DECOMPOSITION_TYPES.values()) {
    const typeRecords = sourceRecords
        .filter((record) => record.decompositionType === decompositionType)
        .sort((left, right) => left.start - right.start);
    for (const record of typeRecords) {
        const previous = records.at(-1);
        if (
            previous?.decompositionType === decompositionType &&
            record.start <= previous.stop + 1
        ) {
            previous.stop = Math.max(previous.stop, record.stop);
        } else {
            records.push({ ...record });
        }
    }
}

if (records.length !== EXPECTED_RECORD_COUNT) {
    throw new Error(
        `expected ${EXPECTED_RECORD_COUNT} compatibility decomposition records, ` +
            `found ${records.length}`,
    );
}

const header = Buffer.alloc(MAGIC.length + 3 + sourceHash.length + 4);
let offset = 0;
offset += MAGIC.copy(header, offset);
offset += Buffer.from(UNICODE_VERSION).copy(header, offset);
offset += Buffer.from(sourceHash, "ascii").copy(header, offset);
header.writeUInt32BE(records.length, offset);

const encodedRecords = Buffer.alloc(records.length * 9);
for (const [index, record] of records.entries()) {
    const recordOffset = index * 9;
    encodedRecords[recordOffset] = record.decompositionType;
    encodedRecords.writeUInt32BE(record.start, recordOffset + 1);
    encodedRecords.writeUInt32BE(record.stop, recordOffset + 5);
}

await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, Buffer.concat([header, encodedRecords]));
console.log(
    `wrote ${records.length} Unicode ${UNICODE_VERSION.join(".")} ` +
        `compatibility decomposition records to ${outputPath}`,
);
