#!/usr/bin/env bash
# End-to-end Kotlin parse-tree parity smoke. Generates Python and Rust parsers
# from the antlr/grammars-v4 Kotlin grammar, parses every snippet under
# tests/kotlin-parity/snippets/*.kt and script-snippets/*.kts with both, and
# asserts the dumped trees are byte-identical.
#
# Required environment / arguments:
#   ANTLR4_JAR (or --antlr-jar):     path to antlr-4.13.2-complete.jar
#   GRAMMARS_V4 (or --grammars-v4):  path to a checkout of antlr/grammars-v4
#                                    (e.g. cloned via git sparse-checkout)
#
# Optional:
#   WORK_DIR (or --work-dir):  scratch directory; defaults to a tempdir.
#   PYTHON   (or --python):    Python interpreter with antlr4-python3-runtime;
#                              defaults to `python3` on PATH.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

WORK_DIR=""
ANTLR4_JAR="${ANTLR4_JAR:-}"
GRAMMARS_V4="${GRAMMARS_V4:-}"
PYTHON="${PYTHON:-python3}"

require_value() {
    if [ "$#" -lt 2 ]; then
        echo "missing value for $1" >&2
        exit 2
    fi
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --antlr-jar)   require_value "$@"; ANTLR4_JAR="$2"; shift 2 ;;
        --grammars-v4) require_value "$@"; GRAMMARS_V4="$2"; shift 2 ;;
        --work-dir)    require_value "$@"; WORK_DIR="$2"; shift 2 ;;
        --python)      require_value "$@"; PYTHON="$2"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

if [ -z "$ANTLR4_JAR" ]; then
    echo "ANTLR4_JAR is required (env var or --antlr-jar)" >&2
    exit 2
fi
if [ -z "$GRAMMARS_V4" ]; then
    echo "GRAMMARS_V4 is required (env var or --grammars-v4)" >&2
    exit 2
fi

if [ -z "$WORK_DIR" ]; then
    WORK_DIR="$(mktemp -d -t kotlin-parity.XXXXXX)"
    trap 'rm -rf "$WORK_DIR"' EXIT
fi
mkdir -p "$WORK_DIR/grammar" "$WORK_DIR/py-gen" "$WORK_DIR/interp"

KOTLIN_DIR="$GRAMMARS_V4/kotlin/kotlin"
for grammar in KotlinLexer.g4 KotlinParser.g4 UnicodeClasses.g4; do
    src="$KOTLIN_DIR/$grammar"
    if [ ! -f "$src" ]; then
        echo "missing $src; pass --grammars-v4 to a grammars-v4 checkout" >&2
        exit 2
    fi
    cp "$src" "$WORK_DIR/grammar/"
done

# --- Generate parsers once for all snippets ---
(
    cd "$WORK_DIR/grammar"
    java -jar "$ANTLR4_JAR" -Dlanguage=Python3 -o "$WORK_DIR/py-gen" \
        KotlinLexer.g4 KotlinParser.g4
    java -jar "$ANTLR4_JAR" -o "$WORK_DIR/interp" -Xexact-output-dir \
        KotlinLexer.g4 KotlinParser.g4
)
DUMPER_DIR="$SCRIPT_DIR/dumper"
DUMPER_GEN="$DUMPER_DIR/src/generated"
mkdir -p "$DUMPER_GEN"
cargo run --quiet --release --manifest-path "$REPO_ROOT/Cargo.toml" \
    --bin antlr4-rust-gen -- \
    --lexer  "$WORK_DIR/interp/KotlinLexer.interp" \
    --grammar "$WORK_DIR/grammar/KotlinLexer.g4" \
    --out-dir "$WORK_DIR/rust-gen"
cargo run --quiet --release --manifest-path "$REPO_ROOT/Cargo.toml" \
    --bin antlr4-rust-gen -- \
    --parser "$WORK_DIR/interp/KotlinParser.interp" \
    --grammar "$WORK_DIR/grammar/KotlinParser.g4" \
    --require-generated-parser \
    --out-dir "$WORK_DIR/rust-gen"
cp "$WORK_DIR/rust-gen/kotlin_lexer.rs" "$WORK_DIR/rust-gen/kotlin_parser.rs" "$DUMPER_GEN/"
cargo build --quiet --release --manifest-path "$DUMPER_DIR/Cargo.toml"
DUMPER_BIN="$DUMPER_DIR/target/release/kotlin-parity-dumper"

# --- Run each snippet ---
shopt -s nullglob
SNIPPETS=("$SCRIPT_DIR"/snippets/*.kt)
if [ "${#SNIPPETS[@]}" -eq 0 ]; then
    echo "no snippets found under $SCRIPT_DIR/snippets" >&2
    exit 2
fi

run_snippet() {
    local snippet="$1"
    local rule="$2"
    local name="$3"
    local py_out="$WORK_DIR/$rule-$name.python.txt"
    local rs_out="$WORK_DIR/$rule-$name.rust.txt"
    "$PYTHON" "$SCRIPT_DIR/dump_python.py" \
        --gen-dir "$WORK_DIR/py-gen" \
        --lexer KotlinLexer \
        --parser KotlinParser \
        --rule "$rule" \
        --input "$snippet" \
        --output "$py_out"
    "$DUMPER_BIN" --input "$snippet" --output "$rs_out" --rule "$rule"
    if diff -u "$py_out" "$rs_out"; then
        echo "Kotlin parity [$rule:$name]: parse trees match"
    else
        echo "Kotlin parity [$rule:$name]: parse trees diverge (diff above)" >&2
        failures=$((failures + 1))
    fi
}

failures=0
for snippet in "${SNIPPETS[@]}"; do
    name="$(basename "$snippet" .kt)"
    run_snippet "$snippet" kotlinFile "$name"
done

SCRIPT_SNIPPETS=("$SCRIPT_DIR"/script-snippets/*.kts)
for snippet in "${SCRIPT_SNIPPETS[@]}"; do
    name="$(basename "$snippet" .kts)"
    run_snippet "$snippet" script "$name"
done

if [ "$failures" -gt 0 ]; then
    echo "Kotlin parity: $failures snippet(s) failed" >&2
    exit 1
fi
