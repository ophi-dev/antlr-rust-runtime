#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: tools/grammar-frontend/update-stage0.sh [--check|--update] [--keep-work-dir]

Regenerate the checked-in ANTLR grammar frontend with the direct source
compiler and prove the Stage 0 -> Stage 1 -> Stage 2 fixed point.

--check is the default and does not modify the repository. --update replaces
the checked-in generated frontend only after the fixed point and candidate
frontend corpus pass. Java and Node.js are not used.
EOF
}

mode=check
keep_work_dir=false

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
        --keep-work-dir)
            keep_work_dir=true
            shift
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

for command in cargo cmp cp mktemp rsync; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "required command not found: $command" >&2
        exit 2
    fi
done

if command -v shasum >/dev/null 2>&1; then
    sha256=(shasum -a 256)
    sha256_check=(shasum -a 256 -c)
elif command -v sha256sum >/dev/null 2>&1; then
    sha256=(sha256sum)
    sha256_check=(sha256sum -c)
else
    echo "required command not found: shasum or sha256sum" >&2
    exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
seed_dir="$repo_root/third_party/antlr-v4-grammar"
checked_in_dir="$repo_root/src/bin_support/grammar/generated"
hash_file="$seed_dir/self-hosted.sha256"

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/antlr-self-host.XXXXXX")"
if [[ "$keep_work_dir" == true ]]; then
    echo "self-host work directory: $work_dir"
else
    trap 'rm -rf "$work_dir"' EXIT
fi

stage1_dir="$work_dir/stage1"
stage2_dir="$work_dir/stage2"
stage1_repo="$work_dir/stage1-repo"
mkdir -p "$stage1_dir" "$stage2_dir" "$stage1_repo"

generate_frontend() {
    local generator="$1"
    local source_root="$2"
    local out_dir="$3"
    local grammar_dir="$source_root/third_party/antlr-v4-grammar"

    "$generator" \
        "$grammar_dir/ANTLRv4Lexer.g4" \
        "$grammar_dir/ANTLRv4Parser.g4" \
        --lib "$grammar_dir" \
        --out-dir "$out_dir" \
        --sem-patterns "$grammar_dir/antlr-v4.toml" \
        --option-hook superClass=LexerAdaptor \
        --sem-unknown error \
        --require-full-semantics \
        --require-generated-parser
}

echo "building Stage 0 generator"
cargo build \
    --quiet \
    --locked \
    --manifest-path "$repo_root/Cargo.toml" \
    --target-dir "$work_dir/stage0-target" \
    --features codegen \
    --bin antlr4-rust-gen

echo "generating Stage 1"
generate_frontend \
    "$work_dir/stage0-target/debug/antlr4-rust-gen" \
    "$repo_root" \
    "$stage1_dir"

# Compile the candidate from a separate absolute checkout path. Besides making
# Stage 1 usable as the frontend for Stage 2, this makes path leakage observable
# in the fixed-point comparison.
rsync \
    --archive \
    --exclude .git \
    --exclude target \
    "$repo_root/" \
    "$stage1_repo/"
cp "$stage1_dir/antl_rv4_lexer.rs" \
    "$stage1_repo/src/bin_support/grammar/generated/antlr_v4_lexer.rs"
cp "$stage1_dir/antl_rv4_parser.rs" \
    "$stage1_repo/src/bin_support/grammar/generated/antlr_v4_parser.rs"

echo "building Stage 1 generator"
cargo build \
    --quiet \
    --locked \
    --manifest-path "$stage1_repo/Cargo.toml" \
    --target-dir "$work_dir/stage1-target" \
    --features codegen \
    --bin antlr4-rust-gen

echo "generating Stage 2"
generate_frontend \
    "$work_dir/stage1-target/debug/antlr4-rust-gen" \
    "$stage1_repo" \
    "$stage2_dir"

for output in antl_rv4_lexer.rs antl_rv4_parser.rs semantics.json; do
    if ! cmp -s "$stage1_dir/$output" "$stage2_dir/$output"; then
        echo "self-host fixed point differs: $output" >&2
        cmp "$stage1_dir/$output" "$stage2_dir/$output" >&2 || true
        exit 1
    fi
done
echo "Stage 1 and Stage 2 are byte-identical"

echo "testing the Stage 1 frontend corpus"
cargo test \
    --quiet \
    --locked \
    --manifest-path "$stage1_repo/Cargo.toml" \
    --target-dir "$work_dir/stage1-target" \
    --features codegen \
    --bin antlr4-rust-gen \
    grammar::frontend::tests::pinned_frontend_corpus_matches_token_and_tree_oracles \
    -- \
    --exact
cargo test \
    --quiet \
    --locked \
    --manifest-path "$stage1_repo/Cargo.toml" \
    --target-dir "$work_dir/stage1-target" \
    --features codegen \
    --bin antlr4-rust-gen \
    grammar::frontend::tests::malformed_bootstrap_inputs_fail_closed \
    -- \
    --exact

if [[ "$mode" == update ]]; then
    cp "$stage1_dir/antl_rv4_lexer.rs" \
        "$checked_in_dir/antlr_v4_lexer.rs"
    cp "$stage1_dir/antl_rv4_parser.rs" \
        "$checked_in_dir/antlr_v4_parser.rs"
    (
        cd "$repo_root"
        "${sha256[@]}" \
            third_party/antlr-v4-grammar/ANTLRv4Lexer.g4 \
            third_party/antlr-v4-grammar/ANTLRv4Parser.g4 \
            third_party/antlr-v4-grammar/predefined.tokens \
            third_party/antlr-v4-grammar/antlr-v4.toml \
            src/bin_support/grammar/generated/antlr_v4_lexer.rs \
            src/bin_support/grammar/generated/antlr_v4_parser.rs \
            >"$hash_file"
    )
    echo "updated the checked-in self-hosted frontend and hashes"
else
    cmp "$stage1_dir/antl_rv4_lexer.rs" \
        "$checked_in_dir/antlr_v4_lexer.rs"
    cmp "$stage1_dir/antl_rv4_parser.rs" \
        "$checked_in_dir/antlr_v4_parser.rs"
    (
        cd "$repo_root"
        "${sha256_check[@]}" "$hash_file"
    )
    echo "the checked-in frontend is the tested self-hosting fixed point"
fi
