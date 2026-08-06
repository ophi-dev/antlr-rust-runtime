#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: tools/toml-syntax/update-generated.sh [--check|--update]

Regenerate the checked-in TOML recognizer with antlr4-rust-gen.
--check is the default and compares generated output without modifying the
repository.
EOF
}

mode=check
while (($#)); do
    case "$1" in
        --check)
            mode=check
            ;;
        --update)
            mode=update
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
    shift
done

for command in cargo cmp cp mktemp sed; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "required command not found: $command" >&2
        exit 2
    fi
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
grammar_dir="$repo_root/third_party/toml-grammar"
checked_in="$repo_root/crates/antlr-rust-toml-parser/src/generated"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/antlr-toml-syntax.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

cargo run \
    --quiet \
    --locked \
    --manifest-path "$repo_root/Cargo.toml" \
    -p antlr-rust-codegen \
    --bin antlr4-rust-gen \
    -- \
    "$grammar_dir/TomlLexer.g4" \
    "$grammar_dir/TomlParser.g4" \
    --lib "$grammar_dir" \
    --out-dir "$work_dir" \
    --sem-unknown error \
    --require-full-semantics \
    --require-generated-parser

outputs=(toml_lexer.rs toml_parser.rs)
if [[ "$mode" == update ]]; then
    for output in "${outputs[@]}"; do
        cp "$work_dir/$output" "$checked_in/$output"
    done
    echo "updated the checked-in TOML recognizer"
else
    for output in "${outputs[@]}"; do
        sed -E \
            -e '1s/v[^ ]+ -/v<generator-version> -/' \
            -e 's/(__antlr4_rust_require_codegen_api!\([0-9]+, )"[^"]+"/\1"<generator-version>"/' \
            "$work_dir/$output" >"$work_dir/generated-normalized-$output"
        sed -E \
            -e '1s/v[^ ]+ -/v<generator-version> -/' \
            -e 's/(__antlr4_rust_require_codegen_api!\([0-9]+, )"[^"]+"/\1"<generator-version>"/' \
            "$checked_in/$output" >"$work_dir/checked-normalized-$output"
        cmp "$work_dir/generated-normalized-$output" \
            "$work_dir/checked-normalized-$output"
    done
    echo "the checked-in TOML recognizer is current"
fi
