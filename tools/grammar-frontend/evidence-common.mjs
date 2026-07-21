import { createHash } from "node:crypto";

export const JAVA_COMMIT = "cc82115a4e7f53d71d9d905caa2c2dfa4da58899";
export const ANTLR_NG_COMMIT = "1f68422ae4bfc62f93343769e144d01f305487b1";
export const VSCODE_COMMIT = "3e9469d1d490c71b3e3b909edf1235582a3f8db8";
export const SCAFFOLD_COMMIT = "75615945749dc93fca5d929cb22ad481f12dfdc9";
export const TEST_COMMIT = "a4258562c44818e2ba97d206587c64d4c38408d0";
export const IMPLEMENTATION_COMMIT = "8a00a3d6496779b969a42511d7e29c0d102d62d7";

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

export function parseMode(args, scriptName) {
    if (args.length !== 1 || !["--check", "--update"].includes(args[0])) {
        throw new Error(`usage: ${scriptName} --check|--update`);
    }
    return args[0] === "--update";
}
