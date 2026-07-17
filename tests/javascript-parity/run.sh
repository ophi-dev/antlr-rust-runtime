#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ANTLR4_JAR="${ANTLR4_JAR:-}"
GRAMMARS_V4="${GRAMMARS_V4:-}"
WORK_DIR=""
PYTHON="${PYTHON:-python3}"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --antlr-jar) ANTLR4_JAR="$2"; shift 2 ;;
        --grammars-v4) GRAMMARS_V4="$2"; shift 2 ;;
        --work-dir) WORK_DIR="$2"; shift 2 ;;
        --python) PYTHON="$2"; shift 2 ;;
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
    WORK_DIR="$(mktemp -d -t javascript-parity.XXXXXX)"
    trap 'rm -rf "$WORK_DIR"' EXIT
fi

UPSTREAM="$GRAMMARS_V4/javascript/javascript"
mkdir -p "$WORK_DIR/grammar" "$WORK_DIR/python-grammar" "$WORK_DIR/py-gen" \
    "$WORK_DIR/interp" "$WORK_DIR/rust-lexer" "$WORK_DIR/rust-parser"
cp "$UPSTREAM/JavaScriptLexer.g4" "$UPSTREAM/JavaScriptParser.g4" "$WORK_DIR/grammar/"
cp "$UPSTREAM/JavaScriptLexer.g4" "$UPSTREAM/JavaScriptParser.g4" \
    "$UPSTREAM/Python3/JavaScriptLexerBase.py" \
    "$UPSTREAM/Python3/JavaScriptParserBase.py" \
    "$UPSTREAM/Python3/transformGrammar.py" "$WORK_DIR/python-grammar/"

(
    cd "$WORK_DIR/grammar"
    java -jar "$ANTLR4_JAR" -o "$WORK_DIR/interp" -Xexact-output-dir \
        JavaScriptLexer.g4 JavaScriptParser.g4
)
(
    cd "$WORK_DIR/python-grammar"
    "$PYTHON" transformGrammar.py
    java -jar "$ANTLR4_JAR" -Dlanguage=Python3 -o "$WORK_DIR/py-gen" \
        -Xexact-output-dir JavaScriptLexer.g4 JavaScriptParser.g4
    cp JavaScriptLexerBase.py JavaScriptParserBase.py "$WORK_DIR/py-gen/"
)

cargo run --quiet --locked --release --manifest-path "$REPO_ROOT/Cargo.toml" \
    --bin antlr4-rust-gen -- \
    --lexer "$WORK_DIR/interp/JavaScriptLexer.interp" \
    --grammar "$WORK_DIR/grammar/JavaScriptLexer.g4" \
    --sem-patterns "$REPO_ROOT/patterns/javascript.toml" \
    --option-hook superClass=JavaScriptLexerBase \
    --sem-unknown error --require-full-semantics \
    --out-dir "$WORK_DIR/rust-lexer"
cargo run --quiet --locked --release --manifest-path "$REPO_ROOT/Cargo.toml" \
    --bin antlr4-rust-gen -- \
    --parser "$WORK_DIR/interp/JavaScriptParser.interp" \
    --grammar "$WORK_DIR/grammar/JavaScriptParser.g4" \
    --sem-patterns "$REPO_ROOT/patterns/javascript.toml" \
    --option-hook superClass=JavaScriptParserBase \
    --sem-unknown error --require-full-semantics \
    --out-dir "$WORK_DIR/rust-parser"

GEN_DIR="$SCRIPT_DIR/dumper/src/generated"
mkdir -p "$GEN_DIR"
cp "$WORK_DIR/rust-lexer/java_script_lexer.rs" \
    "$WORK_DIR/rust-parser/java_script_parser.rs" "$GEN_DIR/"
cargo build --quiet --release --manifest-path "$SCRIPT_DIR/dumper/Cargo.toml"
DUMPER="$SCRIPT_DIR/dumper/target/release/javascript-parity-dumper"

for snippet in "$SCRIPT_DIR"/snippets/*.js; do
    name="$(basename "$snippet")"
    for mode in tree tokens; do
        py_output="$WORK_DIR/$name.$mode.python.txt"
        rust_output="$WORK_DIR/$name.$mode.rust.txt"
        if [ "$mode" = tokens ]; then
            "$PYTHON" "$SCRIPT_DIR/dump_python.py" --gen-dir "$WORK_DIR/py-gen" \
                --input "$snippet" --tokens > "$py_output"
            "$DUMPER" --input "$snippet" --tokens > "$rust_output"
        else
            "$PYTHON" "$SCRIPT_DIR/dump_python.py" --gen-dir "$WORK_DIR/py-gen" \
                --input "$snippet" > "$py_output"
            "$DUMPER" --input "$snippet" > "$rust_output"
        fi
        diff -u "$py_output" "$rust_output"
    done
    echo "JavaScript parity [$name]: tokens and parse tree match"
done
