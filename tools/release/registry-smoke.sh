#!/usr/bin/env bash
# SPDX-License-Identifier: BSD-3-Clause
# Copyright (c) 2026 Konstantin Vyatkin
set -euo pipefail

if [[ "$#" -ne 1 ]]; then
    echo "usage: $0 VERSION" >&2
    exit 2
fi

version="$1"
work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT
mkdir -p "$work_dir/grammar" "$work_dir/src"

cat >"$work_dir/Cargo.toml" <<EOF
[package]
name = "antlr-rust-registry-smoke"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
antlr-rust-runtime = "=${version}"

[build-dependencies]
antlr-rust-codegen = "=${version}"
EOF

cat >"$work_dir/build.rs" <<'EOF'
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"),
    );
    let output_dir =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let generation = antlr_rust_codegen::Builder::new()
        .grammar(manifest_dir.join("grammar/SmokeLexer.g4"))
        .grammar(manifest_dir.join("grammar/SmokeParser.g4"))
        .library_directory(manifest_dir.join("grammar"))
        .out_dir(output_dir)
        .generate()
        .expect("registry codegen should generate the smoke parser");
    generation.emit_rerun_if_changed();
}
EOF

cat >"$work_dir/grammar/SmokeLexer.g4" <<'EOF'
lexer grammar SmokeLexer;

ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
EOF

cat >"$work_dir/grammar/SmokeParser.g4" <<'EOF'
parser grammar SmokeParser;

options {
    tokenVocab = SmokeLexer;
}

start: ID EOF;
EOF

cat >"$work_dir/src/main.rs" <<'EOF'
mod smoke_lexer {
    include!(concat!(env!("OUT_DIR"), "/smoke_lexer.rs"));
}

mod smoke_parser {
    include!(concat!(env!("OUT_DIR"), "/smoke_parser.rs"));
}

use antlr4_runtime::{CommonTokenStream, InputStream};
use smoke_lexer::SmokeLexer;
use smoke_parser::SmokeParser;

fn main() {
    let lexer = SmokeLexer::new(InputStream::new("release"));
    let mut parser = SmokeParser::new(CommonTokenStream::new(lexer));
    parser.start().expect("registry-generated parser should run");
}
EOF

cargo generate-lockfile --manifest-path "$work_dir/Cargo.toml"
metadata="$(cargo metadata --locked --format-version 1 \
    --manifest-path "$work_dir/Cargo.toml")"
for package in \
    antlr-rust-runtime \
    antlr-rust-g4-parser \
    antlr-rust-rs-parser \
    antlr-rust-toml-parser \
    antlr-rust-codegen
do
    if ! jq -e \
        --arg package "$package" \
        --arg version "$version" \
        '.packages[] |
         select(.name == $package and .version == $version) |
         .source | startswith("registry+")' \
        <<<"$metadata" >/dev/null
    then
        echo "$package $version did not resolve from a registry" >&2
        exit 1
    fi
done

cargo run --locked --quiet --manifest-path "$work_dir/Cargo.toml"
echo "registry build-dependency smoke test passed for $version"
