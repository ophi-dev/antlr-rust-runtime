#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 0 ]]; then
    echo "usage: $0" >&2
    exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

package_args=(package --locked --no-verify)
if [[ "${ANTLR_RUST_RELEASE_ALLOW_DIRTY:-0}" == "1" ]]; then
    package_args+=(--allow-dirty)
fi

package_archive() {
    local package="$1"
    shift
    tools/release/verify-package-contents.sh "$package"
    cargo "${package_args[@]}" -p "$package" "$@"
}

package_archive antlr-rust-runtime
package_archive antlr-rust-g4-parser \
    --config 'patch.crates-io.antlr-rust-runtime.path="crates/antlr-rust-runtime"'
package_archive antlr-rust-rs-parser \
    --config 'patch.crates-io.antlr-rust-runtime.path="crates/antlr-rust-runtime"'
package_archive antlr-rust-codegen \
    --config 'patch.crates-io.antlr-rust-runtime.path="crates/antlr-rust-runtime"' \
    --config 'patch.crates-io.antlr-rust-g4-parser.path="crates/antlr-rust-g4-parser"' \
    --config 'patch.crates-io.antlr-rust-rs-parser.path="crates/antlr-rust-rs-parser"'

echo "all publishable package archives built successfully"
