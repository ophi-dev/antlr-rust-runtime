#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ANTLR_VERSION = "4.13.2";
const ANTLR_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
const UNICODE_DATA_SHA256 =
    "19e4a0ddf10d9c08397dafad778b6a5a80347ded3a7422345d903f02092936bc";
const PROPERTY_COUNT = 1185;
const EXPECTED_JAVA = {
    vendor: "Homebrew",
    runtime: "26.0.1",
    vm: "26.0.1",
};

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const defaultOutput = resolve(
    repositoryRoot,
    "src/bin_support/grammar/unicode_data.rs",
);
const options = parseArguments(process.argv.slice(2));
const source = await readFile(options.unicodeData, "utf8");
const sourceHash = digest(source);
if (sourceHash !== UNICODE_DATA_SHA256) {
    throw new Error(
        `UnicodeData.java SHA-256 mismatch: expected ${UNICODE_DATA_SHA256}, found ${sourceHash}`,
    );
}

const properties = parseProperties(source);
const aliases = parseAliases(source);
const caseData = await generateCaseMappings(options.java);
const generated = render(properties, aliases, caseData);

if (options.update) {
    await writeFile(options.output, generated, "utf8");
    console.log(
        `updated ${options.output}: ${properties.length} properties, ` +
            `${aliases.length} aliases, ${caseData.lower.length} lower-case mappings, ` +
            `${caseData.upper.length} upper-case mappings`,
    );
} else {
    const checkedIn = await readFile(options.output, "utf8");
    if (checkedIn !== generated) {
        throw new Error(`${options.output} differs from pinned Unicode data`);
    }
    console.log(`verified pinned Unicode data in ${options.output}`);
}

function parseArguments(args) {
    const result = {
        java: "java",
        output: defaultOutput,
        unicodeData: null,
        update: false,
    };
    for (let index = 0; index < args.length; index++) {
        const argument = args[index];
        switch (argument) {
            case "--java":
                result.java = requiredValue(args, ++index, argument);
                break;
            case "--output":
                result.output = resolve(requiredValue(args, ++index, argument));
                break;
            case "--unicode-data":
                result.unicodeData = resolve(requiredValue(args, ++index, argument));
                break;
            case "--update":
                result.update = true;
                break;
            default:
                throw new Error(`unknown argument: ${argument}`);
        }
    }
    if (result.unicodeData === null) {
        throw new Error("--unicode-data PATH is required");
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

function parseProperties(source) {
    const pattern =
        /addProperty\("(?<name>[^"]+)", new int\[\] \{(?<values>[^}]*)\}\);/gu;
    const properties = [];
    for (const match of source.matchAll(pattern)) {
        const name = match.groups.name;
        const rawValues = match.groups.values.trim();
        const values = rawValues.length === 0
            ? []
            : rawValues
                .split(",")
                .map((value) => Number.parseInt(value.trim(), 10));
        if (values.length % 2 !== 0) {
            throw new Error(`property ${name} has an invalid interval list`);
        }
        if (normalize(name) !== name) {
            throw new Error(`property name is not normalized: ${name}`);
        }
        properties.push({ name, values });
    }
    if (properties.length !== PROPERTY_COUNT) {
        throw new Error(
            `expected ${PROPERTY_COUNT} properties, found ${properties.length}`,
        );
    }
    return properties;
}

function parseAliases(source) {
    const declaration = /String\[\] rawAliases = new String\[\] \{(?<body>[\s\S]*?)\};/u.exec(
        source,
    );
    if (!declaration) {
        throw new Error("cannot find rawAliases in UnicodeData.java");
    }
    const values = [...declaration.groups.body.matchAll(/"(?<value>[^"]*)"/gu)].map(
        (match) => match.groups.value,
    );
    if (values.length % 2 !== 0) {
        throw new Error("rawAliases must contain alias/property pairs");
    }
    const aliases = [];
    for (let index = 0; index < values.length; index += 2) {
        const name = values[index];
        const property = values[index + 1];
        if (normalize(property) !== property) {
            throw new Error(`alias target is not normalized: ${name} -> ${property}`);
        }
        aliases.push({ name, property });
    }
    return aliases;
}

async function generateCaseMappings(javaExecutable) {
    const directory = await mkdtemp(join(tmpdir(), "antlr-rust-unicode-"));
    const sourcePath = join(directory, "DumpSimpleCaseMappings.java");
    const javaSource = `public class DumpSimpleCaseMappings {
    public static void main(String[] args) {
        System.out.println("META\\tvendor\\t" + System.getProperty("java.vendor"));
        System.out.println("META\\truntime\\t" + System.getProperty("java.runtime.version"));
        System.out.println("META\\tvm\\t" + System.getProperty("java.vm.version"));
        for (int codePoint = Character.MIN_CODE_POINT;
             codePoint <= Character.MAX_CODE_POINT;
             codePoint++) {
            int lower = Character.toLowerCase(codePoint);
            int upper = Character.toUpperCase(codePoint);
            if (lower != codePoint) {
                System.out.println("LOWER\\t" + codePoint + "\\t" + lower);
            }
            if (upper != codePoint) {
                System.out.println("UPPER\\t" + codePoint + "\\t" + upper);
            }
        }
    }
}
`;
    try {
        await writeFile(sourcePath, javaSource, "utf8");
        const result = spawnSync(javaExecutable, [sourcePath], {
            encoding: "utf8",
            maxBuffer: 16 * 1024 * 1024,
        });
        if (result.error) {
            throw result.error;
        }
        if (result.status !== 0) {
            throw new Error(
                `Java case-mapping helper failed (${result.status}): ${result.stderr}`,
            );
        }
        return parseCaseMappings(result.stdout);
    } finally {
        await rm(directory, { force: true, recursive: true });
    }
}

function parseCaseMappings(output) {
    const metadata = {};
    const lower = [];
    const upper = [];
    for (const line of output.trimEnd().split("\n")) {
        const [kind, left, right] = line.split("\t");
        if (kind === "META") {
            metadata[left] = right;
        } else if (kind === "LOWER" || kind === "UPPER") {
            const mapping = [Number.parseInt(left, 10), Number.parseInt(right, 10)];
            (kind === "LOWER" ? lower : upper).push(mapping);
        } else {
            throw new Error(`unexpected Java helper output: ${line}`);
        }
    }
    for (const [key, expected] of Object.entries(EXPECTED_JAVA)) {
        if (metadata[key] !== expected) {
            throw new Error(
                `expected Java ${key} ${expected}, found ${metadata[key] ?? "<missing>"}`,
            );
        }
    }
    return { metadata, lower, upper };
}

function render(properties, aliases, caseData) {
    const propertyByName = new Map();
    const uniqueRanges = new Map();
    const flatRanges = [];
    for (const property of properties) {
        const key = property.values.join(",");
        let range = uniqueRanges.get(key);
        if (range === undefined) {
            range = { offset: flatRanges.length, length: property.values.length };
            uniqueRanges.set(key, range);
            flatRanges.push(...property.values);
        }
        propertyByName.set(property.name, range);
    }

    const lookup = new Map(propertyByName);
    for (const alias of aliases) {
        const range = propertyByName.get(alias.property);
        if (range === undefined) {
            throw new Error(
                `alias ${alias.name} targets unknown property ${alias.property}`,
            );
        }
        if (!lookup.has(alias.name)) {
            lookup.set(alias.name, range);
        }
    }
    const entries = [...lookup.entries()].sort(([left], [right]) =>
        left < right ? -1 : left > right ? 1 : 0,
    );

    return `// @generated by tools/grammar-frontend/generate-unicode-data.mjs; do not edit.
// ANTLR ${ANTLR_VERSION} (${ANTLR_COMMIT}) UnicodeData.java
// source SHA-256: ${UNICODE_DATA_SHA256}
// case mappings: ${caseData.metadata.vendor} OpenJDK ${caseData.metadata.runtime} (VM ${caseData.metadata.vm})

#![allow(clippy::unreadable_literal)]

#[derive(Clone, Copy, Debug)]
pub(super) struct PropertyEntry {
    pub(super) name: &'static str,
    pub(super) offset: u32,
    pub(super) length: u32,
}

#[rustfmt::skip]
pub(super) static PROPERTY_ENTRIES: &[PropertyEntry] = &[
${entries
    .map(
        ([name, range]) =>
            `    PropertyEntry { name: ${JSON.stringify(name)}, offset: ${range.offset}, length: ${range.length} },`,
    )
    .join("\n")}
];

#[rustfmt::skip]
pub(super) static PROPERTY_RANGES: &[i32] = &[
${renderNumbers(flatRanges)}
];

#[rustfmt::skip]
pub(super) static SIMPLE_LOWERCASE: &[(i32, i32)] = &[
${renderMappings(caseData.lower)}
];

#[rustfmt::skip]
pub(super) static SIMPLE_UPPERCASE: &[(i32, i32)] = &[
${renderMappings(caseData.upper)}
];
`;
}

function renderNumbers(values) {
    const lines = [];
    for (let index = 0; index < values.length; index += 16) {
        lines.push(`    ${values.slice(index, index + 16).join(", ")},`);
    }
    return lines.join("\n");
}

function renderMappings(mappings) {
    const lines = [];
    for (let index = 0; index < mappings.length; index += 8) {
        lines.push(
            `    ${mappings
                .slice(index, index + 8)
                .map(([source, target]) => `(${source}, ${target})`)
                .join(", ")},`,
        );
    }
    return lines.join("\n");
}

function normalize(value) {
    return [...value]
        .map((character) => {
            if (character === "-") {
                return "_";
            }
            const code = character.codePointAt(0);
            return code >= 0x41 && code <= 0x5A
                ? String.fromCodePoint(code + 0x20)
                : character;
        })
        .join("");
}

function digest(value) {
    return createHash("sha256").update(value).digest("hex");
}
