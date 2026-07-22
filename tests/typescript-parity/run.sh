#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ANTLR4_JAR="${ANTLR4_JAR:-}"
GRAMMARS_V4="${GRAMMARS_V4:-}"
WORK_DIR=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --antlr-jar) ANTLR4_JAR="$2"; shift 2 ;;
        --grammars-v4) GRAMMARS_V4="$2"; shift 2 ;;
        --work-dir) WORK_DIR="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

if [ -z "$ANTLR4_JAR" ] || [ ! -f "$ANTLR4_JAR" ]; then
    echo "ANTLR4_JAR is required (env var or --antlr-jar)" >&2
    exit 2
fi
if [ -z "$GRAMMARS_V4" ] || [ ! -d "$GRAMMARS_V4" ]; then
    echo "GRAMMARS_V4 is required (env var or --grammars-v4)" >&2
    exit 2
fi
if [ -z "$WORK_DIR" ]; then
    WORK_DIR="$(mktemp -d -t typescript-parity.XXXXXX)"
    trap 'rm -rf "$WORK_DIR"' EXIT
fi

UPSTREAM="$GRAMMARS_V4/javascript/typescript"
mkdir -p "$WORK_DIR/grammar" "$WORK_DIR/java-gen" "$WORK_DIR/java-classes" \
    "$WORK_DIR/rust-lexer" "$WORK_DIR/rust-parser"
cp "$UPSTREAM/TypeScriptLexer.g4" "$UPSTREAM/TypeScriptParser.g4" "$WORK_DIR/grammar/"

(
    cd "$WORK_DIR/grammar"
    java -jar "$ANTLR4_JAR" -o "$WORK_DIR/java-gen" -Xexact-output-dir \
        TypeScriptLexer.g4 TypeScriptParser.g4
)
cp "$UPSTREAM/Java/TypeScriptLexerBase.java" \
    "$UPSTREAM/Java/TypeScriptParserBase.java" "$WORK_DIR/java-gen/"
javac --release 17 -cp "$ANTLR4_JAR" -d "$WORK_DIR/java-classes" \
    "$WORK_DIR/java-gen/"*.java "$SCRIPT_DIR/TypeScriptParityDumper.java"

cargo run --quiet --locked --release --manifest-path "$REPO_ROOT/Cargo.toml" \
    --features codegen \
    --bin antlr4-rust-gen -- \
    --lexer "$WORK_DIR/java-gen/TypeScriptLexer.interp" \
    --grammar "$WORK_DIR/grammar/TypeScriptLexer.g4" \
    --sem-patterns "$REPO_ROOT/patterns/javascript.toml" \
    --option-hook superClass=TypeScriptLexerBase \
    --sem-unknown error --require-full-semantics \
    --out-dir "$WORK_DIR/rust-lexer"
cargo run --quiet --locked --release --manifest-path "$REPO_ROOT/Cargo.toml" \
    --features codegen \
    --bin antlr4-rust-gen -- \
    --parser "$WORK_DIR/java-gen/TypeScriptParser.interp" \
    --grammar "$WORK_DIR/grammar/TypeScriptParser.g4" \
    --sem-patterns "$REPO_ROOT/patterns/javascript.toml" \
    --option-hook superClass=TypeScriptParserBase \
    --sem-unknown error --require-full-semantics \
    --out-dir "$WORK_DIR/rust-parser"

GEN_DIR="$SCRIPT_DIR/dumper/src/generated"
mkdir -p "$GEN_DIR"
cp "$WORK_DIR/rust-lexer/type_script_lexer.rs" \
    "$WORK_DIR/rust-parser/type_script_parser.rs" "$GEN_DIR/"
cargo build --quiet --release --manifest-path "$SCRIPT_DIR/dumper/Cargo.toml"
RUST_DUMPER="$SCRIPT_DIR/dumper/target/release/typescript-parity-dumper"
JAVA_CLASSPATH="$ANTLR4_JAR:$WORK_DIR/java-classes"

for snippet in "$SCRIPT_DIR"/snippets/*.ts; do
    name="$(basename "$snippet")"
    for mode in tree tokens; do
        java_output="$WORK_DIR/$name.$mode.java.txt"
        rust_output="$WORK_DIR/$name.$mode.rust.txt"
        if [ "$mode" = tokens ]; then
            java -cp "$JAVA_CLASSPATH" TypeScriptParityDumper \
                --input "$snippet" --tokens > "$java_output"
            "$RUST_DUMPER" --input "$snippet" --tokens > "$rust_output"
        else
            java -cp "$JAVA_CLASSPATH" TypeScriptParityDumper \
                --input "$snippet" > "$java_output"
            "$RUST_DUMPER" --input "$snippet" > "$rust_output"
        fi
        diff -u "$java_output" "$rust_output"
    done
    echo "TypeScript parity [$name]: tokens and parse tree match"
done
