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
export const BASIC_SEMANTIC_BASE_COMMIT =
    "685575f01d17b4d6791de8ac4207afe6f38d4b40";
export const BASIC_SEMANTIC_TEST_COMMIT =
    "d133e49cebf5b149b9cdf0dff70171128cd8ded5";
export const BASIC_SEMANTIC_IMPLEMENTATION_COMMIT =
    "1733fa4e06a1b97951592da68689a9bff27ba86c";
export const ERROR_SETS_BASE_COMMIT =
    "78ab1df577b28595576051b902285dfbe669fe7a";
export const ERROR_SETS_TEST_COMMIT =
    "01e1d8fce4bb5638f2b4f726049a4c8c4378047a";
export const ERROR_SETS_IMPLEMENTATION_COMMIT =
    "181eb633a681f90b6047b649a1d190062d38aa99";
export const TOKEN_POSITION_BASE_COMMIT =
    "7e83b079daa595ee26314b14d3d1bffa7924771f";
export const TOKEN_POSITION_TEST_COMMIT =
    "a82df4a391d4bec1c355569f3e94dedafe4f0e2e";
export const TOKEN_POSITION_IMPLEMENTATION_COMMIT =
    "b058502fd73d34776cf53afd2bcff3cbb2517c3f";
export const TOPOLOGICAL_SORT_BASE_COMMIT =
    "6b1c62d0e8d6c06e07a39a773354044cf92b47b4";
export const TOPOLOGICAL_SORT_TEST_COMMIT =
    "738ada36e551037733b02e556809b03b8c2c73ea";
export const VOCABULARY_BASE_COMMIT =
    "f7a868948f3d40e64c9a10f27bdb3d0028456618";
export const VOCABULARY_TEST_COMMIT =
    "d725eac437b8425eb6429f9a0b1844a409f45194";
export const VOCABULARY_IMPLEMENTATION_COMMIT =
    "134b4e7304defa10da58e2b2ef90c6517f20213f";
export const EMPTY_VOCABULARY_BASE_COMMIT =
    VOCABULARY_IMPLEMENTATION_COMMIT;
export const EMPTY_VOCABULARY_TEST_COMMIT =
    "d2211267b0573bf147d1baa57af8f7c13ce2d245";
export const EMPTY_VOCABULARY_IMPLEMENTATION_COMMIT =
    "2de0397b1db0d20cad0d59f2ea31c55b989edae2";
export const SCOPE_PARSING_BASE_COMMIT =
    "2246948ef1f6a73d98bccfe0e75e272ce6d3c2f2";
export const SCOPE_PARSING_TEST_COMMIT =
    "dfb3d07d4536f91094931dcc4884567530f78b11";
export const SCOPE_PARSING_IMPLEMENTATION_COMMIT =
    "9bef3f59a6e29b5319e25807a2af6702e2d387a5";
export const CHAR_SUPPORT_BASE_COMMIT =
    "d29eacaf62e6c1afc5f3461025a53bc7bd26e1c4";
export const CHAR_SUPPORT_TEST_COMMIT =
    "8970016dbac6705533db4d9ad55d996b61bed026";
export const CHAR_SUPPORT_IMPLEMENTATION_COMMIT =
    "28b8fe8bf72608dc19752d8cb39feab7ecb21fc3";
export const NESTED_ACTION_BASE_COMMIT =
    "98219b854f02c72685ea13546c5b491a59b6d384";
export const NESTED_ACTION_TEST_COMMIT =
    "8ab5c20ff8a99514d51672268b632f6b2bac7678";
export const NESTED_ACTION_IMPLEMENTATION_COMMIT =
    "78c6d271b6379fa230bb9de133307785a6877587";
export const ESCAPE_SEQUENCE_SCAFFOLD_PARENT_COMMIT =
    "17ae053923ef3d11121a1c7523335c06e6a4e657";
export const ESCAPE_SEQUENCE_SCAFFOLD_COMMIT =
    "ce366e7033cff879abe0d23cd5f896c577e42358";
export const ESCAPE_SEQUENCE_TEST_COMMIT =
    "ed8396cd044c5e9893f7076f0fddf92c2e77d16e";
export const ESCAPE_SEQUENCE_IMPLEMENTATION_COMMIT =
    "56d2ac6a31671f9d786f2c3fa6323fffa6474375";

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
