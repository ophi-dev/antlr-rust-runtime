import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";

export const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
export const ANTLR_NG_COMMIT = "1f68422ae4bfc62f93343769e144d01f305487b1";
export const VSCODE_COMMIT = "3e9469d1d490c71b3e3b909edf1235582a3f8db8";
export const SCAFFOLD_COMMIT = "75615945749dc93fca5d929cb22ad481f12dfdc9";
export const TEST_COMMIT = "a4258562c44818e2ba97d206587c64d4c38408d0";
export const IMPLEMENTATION_COMMIT = "8a00a3d6496779b969a42511d7e29c0d102d62d7";
export const FRONTEND_SYNTAX_TEST_COMMIT =
    "9ce58d9d60d5ce1226c16460c22819fb0bd3b06a";
export const FRONTEND_SYNTAX_TEST_PARENT =
    "c8a5df019c8183430febee19e9a71eb5d882b961";
export const PHASE_B_BASE_COMMIT =
    "63e800d1236d34721b0f870d8a2c723c74edfc5e";
export const PHASE_B_IMPLEMENTATION_COMMIT =
    "91359e85a4b2c8563edd40a7495eb2a05ad7a5ad";
export const ATN_SERIALIZATION_TEST_COMMIT =
    "5c5c82fb7879bce9d99d684855bfd07dd6405850";
export const ATN_CONSTRUCTION_BASE_COMMIT =
    "9bf1d7b5892bd03af02f2824d977d15a6cb43d20";
export const ATN_CONSTRUCTION_TEST_COMMIT =
    "62111d633ccc4bddcc85ab7126591febbeb18690";
export const ATN_CONSTRUCTION_IMPLEMENTATION_COMMIT =
    "5d0729a84eac4b89012e31819eb00f5acb654c3a";

export function stableStringify(value) {
    if (Array.isArray(value)) {
        return `[${value.map(stableStringify).join(",")}]`;
    }
    if (value && typeof value === "object") {
        return `{${Object.keys(value)
            .sort()
            .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
            .join(",")}}`;
    }
    return JSON.stringify(value);
}

export function digest(value) {
    return createHash("sha256").update(value).digest("hex");
}

export function gitShowOptional(cwd, commit, path) {
    const result = spawnSync("git", ["show", `${commit}:${path}`], {
        cwd,
        encoding: "utf8",
        maxBuffer: 32 * 1024 * 1024,
    });
    if (result.status !== 0) {
        return null;
    }
    return result.stdout;
}

export function parseMode(args, scriptName) {
    if (args.length !== 1 || !["--check", "--update"].includes(args[0])) {
        throw new Error(`usage: ${scriptName} --check|--update`);
    }
    return args[0] === "--update";
}
