#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: tools/grammar-frontend/update-stage0.sh [--check|--update] [--antlr-jar PATH]

Regenerate the checked-in Stage 0 ANTLR grammar frontend with the pinned
ANTLR 4.13.2 tool. --check is the default and does not modify the repository.
EOF
}

mode=check
antlr_jar="${ANTLR4_JAR:-/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar}"

while (($#)); do
    case "$1" in
        --check)
            mode=check
            shift
            ;;
        --update)
            mode=update
            shift
            ;;
        --antlr-jar)
            if (($# < 2)); then
                echo "--antlr-jar requires a path" >&2
                exit 2
            fi
            antlr_jar="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
java_bin="${JAVA:-java}"

if [[ ! -f "$antlr_jar" ]]; then
    echo "ANTLR 4.13.2 jar not found: $antlr_jar" >&2
    exit 2
fi
if ! command -v "$java_bin" >/dev/null 2>&1; then
    echo "Java launcher not found: $java_bin" >&2
    exit 2
fi
if ! command -v node >/dev/null 2>&1; then
    echo "Node.js is required to validate the Stage 0 JSON manifest" >&2
    exit 2
fi

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/antlr-stage0.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT
mkdir -p "$work_dir/interp" "$work_dir/generated"

seed_dir="$repo_root/third_party/antlr-v4-grammar"
cp "$seed_dir/ANTLRv4Lexer.g4" \
    "$seed_dir/ANTLRv4Parser.g4" \
    "$seed_dir/predefined.tokens" \
    "$work_dir/"

"$java_bin" -version 2>"$work_dir/java-version.txt"
(
    cd "$work_dir"
    "$java_bin" -jar "$antlr_jar" \
        -o "$work_dir/interp" \
        -Xexact-output-dir \
        ANTLRv4Lexer.g4 ANTLRv4Parser.g4
)

cargo build \
    --quiet \
    --locked \
    --manifest-path "$repo_root/Cargo.toml" \
    --bin antlr4-rust-gen

generator="$repo_root/target/debug/antlr4-rust-gen"
"$generator" \
    --lexer "$work_dir/interp/ANTLRv4Lexer.interp" \
    --grammar "$work_dir/ANTLRv4Lexer.g4" \
    --out-dir "$work_dir/generated" \
    --sem-patterns "$seed_dir/antlr-v4.toml" \
    --option-hook superClass=LexerAdaptor \
    --sem-unknown error \
    --require-full-semantics
"$generator" \
    --parser "$work_dir/interp/ANTLRv4Parser.interp" \
    --grammar "$work_dir/ANTLRv4Parser.g4" \
    --out-dir "$work_dir/generated" \
    --sem-patterns "$seed_dir/antlr-v4.toml" \
    --sem-unknown error \
    --require-full-semantics \
    --require-generated-parser

node "$script_dir/validate-stage0.mjs" \
    "--$mode" \
    --work-dir "$work_dir" \
    --antlr-jar "$antlr_jar"
