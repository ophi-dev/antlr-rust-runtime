#!/usr/bin/env bash
# SPDX-License-Identifier: BSD-3-Clause
# Copyright (c) 2026 Konstantin Vyatkin
set -euo pipefail

if [[ "$#" -ne 1 ]]; then
    echo "usage: $0 PACKAGE" >&2
    exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
package="$1"
package_args=(package --locked --list -p "$package")
if [[ "${ANTLR_RUST_RELEASE_ALLOW_DIRTY:-0}" == "1" ]]; then
    package_args+=(--allow-dirty)
fi
listing="$(cargo "${package_args[@]}")"

require_file() {
    if ! grep -Fqx "$1" <<<"$listing"; then
        echo "$package package is missing $1" >&2
        exit 1
    fi
}

require_file "LICENSE"
require_file "README.md"

case "$package" in
    antlr-rust-runtime)
        require_file "src/lib.rs"
        if grep -Eq '(^|/)(bin_support|tests|fixtures|codegen-direct)(/|$)|^src/bin/' \
            <<<"$listing"; then
            echo "runtime package contains codegen or test files" >&2
            exit 1
        fi
        ;;
    antlr-rust-g4-parser)
        require_file "src/frontend.rs"
        require_file "src/lexer_adaptor.rs"
        require_file "src/generated/antlr_v4_lexer.rs"
        require_file "src/generated/antlr_v4_parser.rs"
        ;;
    antlr-rust-rs-parser)
        require_file "src/generated/rust_lexer.rs"
        require_file "src/generated/rust_parser.rs"
        require_file "src/generated/semantics.json"
        require_file "src/generated/decisions.json"
        ;;
    antlr-rust-toml-parser)
        require_file "NOTICE"
        require_file "src/generated/toml_lexer.rs"
        require_file "src/generated/toml_parser.rs"
        require_file "src/lib.rs"
        ;;
    antlr-rust-codegen)
        require_file "src/bin/antlr4-rust-gen.rs"
        require_file "src/bin/antlr4-rust-testrig.rs"
        require_file "src/grammar/unicode_decomposition.bin"
        require_file "src/lib.rs"
        if grep -Eq '(^|/)(tests|fixtures|codegen-direct)(/|$)' <<<"$listing"; then
            echo "codegen package contains test corpora" >&2
            exit 1
        fi
        ;;
    *)
        echo "unknown publishable package: $package" >&2
        exit 2
        ;;
esac

echo "$package package contents verified"
