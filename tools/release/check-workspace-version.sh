#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required" >&2
    exit 2
fi

metadata="$(cargo metadata --locked --no-deps --format-version 1)"
expected_packages=(
    antlr-rust-codegen
    antlr-rust-g4-parser
    antlr-rust-rs-parser
    antlr-rust-runtime
    antlr-rust-toml-parser
)
mapfile -t publishable_packages < <(
    jq -r '.packages[] | select(.publish != []) | .name' <<<"$metadata" | sort
)
if [[ "${publishable_packages[*]}" != "${expected_packages[*]}" ]]; then
    echo "unexpected publishable package set: ${publishable_packages[*]}" >&2
    exit 1
fi

mapfile -t versions < <(
    jq -r '.packages[] | select(.publish != []) | .version' <<<"$metadata" | sort -u
)
if [[ "${#versions[@]}" -ne 1 ]]; then
    echo "publishable package versions differ: ${versions[*]}" >&2
    exit 1
fi
version="${versions[0]}"

release_version="$(<VERSION)"
if [[ "$release_version" != "$version" ]]; then
    echo "VERSION $release_version does not match workspace version $version" >&2
    exit 1
fi

if [[ "${1:-}" == "--version" ]]; then
    if [[ "$#" -ne 2 ]]; then
        echo "usage: $0 [--version VERSION]" >&2
        exit 2
    fi
    if [[ "$version" != "$2" ]]; then
        echo "workspace version $version does not match expected version $2" >&2
        exit 1
    fi
elif [[ "$#" -ne 0 ]]; then
    echo "usage: $0 [--version VERSION]" >&2
    exit 2
fi

for package in "${expected_packages[@]}"; do
    if ! grep -Eq \
        "^${package} = \\{[^}]*version = \"=${version}\"[^}]*\\}$" \
        Cargo.toml; then
        echo "workspace dependency $package is not pinned to =$version" >&2
        exit 1
    fi
done

echo "workspace release set is consistent at $version"
