#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: tools/rust-syntax/update-generated.sh [--check|--update]

Regenerate the checked-in action-free Rust recognizer with antlr4-rust-gen.
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

for command in cargo cmp cp mktemp; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "required command not found: $command" >&2
        exit 2
    fi
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
grammar="$repo_root/third_party/rust-grammar/Rust.g4"
checked_in="$repo_root/crates/antlr-rust-rs-parser/src/generated"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/antlr-rust-syntax.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

cargo run \
    --quiet \
    --locked \
    --manifest-path "$repo_root/Cargo.toml" \
    -p antlr-rust-codegen \
    --bin antlr4-rust-gen \
    -- \
    "$grammar" \
    --out-dir "$work_dir" \
    --sem-unknown error \
    --require-full-semantics \
    --require-generated-parser

outputs=(rust_lexer.rs rust_parser.rs semantics.json decisions.json)
if [[ "$mode" == update ]]; then
    for output in "${outputs[@]}"; do
        cp "$work_dir/$output" "$checked_in/$output"
    done
    echo "updated the checked-in Rust recognizer"
else
    for output in "${outputs[@]}"; do
        if [[ "$output" == *.rs ]]; then
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
        else
            cmp "$work_dir/$output" "$checked_in/$output"
        fi
    done
    echo "the checked-in Rust recognizer is current"
fi
