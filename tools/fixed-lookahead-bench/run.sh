#!/bin/bash
# Differential validation + A/B benchmark for `--fixed-lookahead` (issue #150).
#
# For each target grammar (Thrift, EDN, Rego from antlr/grammars-v4) this
# script generates a baseline parser and a `--fixed-lookahead` parser,
# parses the grammar's full example corpus with both (valid AND invalid
# inputs), and requires byte-identical parse trees and error output. It then
# reports interleaved min-parse timings.
#
# Usage: tools/fixed-lookahead-bench/run.sh [--grammars-v4 DIR]
# Everything lands under target/fixed-lookahead-bench/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$ROOT/target/fixed-lookahead-bench"
GRAMMARS=""
while [ $# -gt 0 ]; do
  case "$1" in
    --grammars-v4) GRAMMARS="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

mkdir -p "$WORK"

# 1. grammars-v4 checkout (sparse: the three target grammars).
if [ -z "$GRAMMARS" ]; then
  GRAMMARS="$WORK/grammars-v4"
  if [ ! -d "$GRAMMARS/.git" ]; then
    git init -q "$GRAMMARS"
    git -C "$GRAMMARS" remote add origin https://github.com/antlr/grammars-v4.git
    git -C "$GRAMMARS" sparse-checkout init --cone
    git -C "$GRAMMARS" sparse-checkout set thrift edn rego
    git -C "$GRAMMARS" fetch --depth 1 origin master
    git -C "$GRAMMARS" checkout -q FETCH_HEAD
  fi
fi

# 2. Build the generator and generate base/fixed parser pairs.
cargo build --release -p antlr-rust-codegen --bin antlr4-rust-gen \
    --manifest-path "$ROOT/Cargo.toml"
GEN="$ROOT/target/release/antlr4-rust-gen"

"$GEN" "$GRAMMARS/thrift/Thrift.g4" --require-generated-parser \
    --out-dir "$WORK/gen/thrift-base"
"$GEN" "$GRAMMARS/thrift/Thrift.g4" --require-generated-parser \
    --fixed-lookahead 2 --out-dir "$WORK/gen/thrift-fixed"
"$GEN" "$GRAMMARS/edn/edn.g4" --require-generated-parser \
    --out-dir "$WORK/gen/edn-base"
"$GEN" "$GRAMMARS/edn/edn.g4" --require-generated-parser \
    --fixed-lookahead 2 --out-dir "$WORK/gen/edn-fixed"
"$GEN" "$GRAMMARS/rego/RegoLexer.g4" "$GRAMMARS/rego/RegoParser.g4" \
    --require-generated-parser --out-dir "$WORK/gen/rego-base"
"$GEN" "$GRAMMARS/rego/RegoLexer.g4" "$GRAMMARS/rego/RegoParser.g4" \
    --require-generated-parser --fixed-lookahead 3 --out-dir "$WORK/gen/rego-fixed"

# 3. Scaffold the six-binary driver crate (tree dump + timing per file).
DRIVER="$WORK/driver"
mkdir -p "$DRIVER/src/bin"
cat > "$DRIVER/Cargo.toml" <<EOF
[package]
name = "fixed-lookahead-driver"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
antlr4_runtime = { package = "antlr-rust-runtime", path = "$ROOT" }

[workspace]
EOF

emit_bin() {
  local bin=$1 gendir=$2 module=$3 lexer=$4 _parser=$5 entry=$6
  cat > "$DRIVER/src/bin/$bin.rs" <<EOF
#[path = "$WORK/gen/$gendir/${module}_lexer.rs"]
mod glexer;
#[path = "$WORK/gen/$gendir/${module}_parser.rs"]
mod gparser;

use antlr4_runtime::{Node, NodeKind, Parser};
use std::io::Write;
use std::time::Instant;

fn dump(out: &mut impl Write, tree: Node<'_>, rule_names: &[&str], depth: usize) {
    let pad = "  ".repeat(depth);
    match tree.kind() {
        NodeKind::Rule => {
            let rule = tree.as_rule().expect("rule node");
            let name = rule_names.get(rule.rule_index()).copied().unwrap_or("<?>");
            writeln!(out, "{pad}Rule({name}, children={})", rule.child_count()).expect("write");
            for child in rule.children() {
                dump(out, child, rule_names, depth + 1);
            }
        }
        NodeKind::Terminal => {
            writeln!(out, "{pad}Term({:?})", tree.as_terminal().expect("terminal").text())
                .expect("write");
        }
        NodeKind::Error => {
            writeln!(out, "{pad}Err({:?})", tree.as_error().expect("error").text())
                .expect("write");
        }
    }
}

fn main() {
    let mut file = None;
    let mut iters = 1usize;
    let mut report_time = false;
    let mut quiet = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--iters" => iters = args.next().expect("--iters value").parse().expect("usize"),
            "--time" => report_time = true,
            "--quiet" => quiet = true,
            other => file = Some(other.to_owned()),
        }
    }
    let file = file.expect("usage: $bin FILE [--iters N] [--time] [--quiet]");
    let text = std::fs::read_to_string(&file).expect("readable input");
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let mut min_us = u128::MAX;
    let mut result = None;
    for _ in 0..iters {
        let started = Instant::now();
        let outcome = gparser::parse_with_parser(&text, glexer::$lexer::new, |parser| {
            parser.$entry()
        });
        min_us = min_us.min(started.elapsed().as_micros());
        result = Some(outcome);
    }
    if report_time {
        writeln!(out, "TIME_MIN_US={min_us}").expect("write");
    }
    match result.expect("at least one iteration") {
        Ok(output) => {
            writeln!(out, "SYNTAX_ERRORS={}", output.parser.number_of_syntax_errors())
                .expect("write");
            if !quiet {
                let parsed = output.parser.into_parsed_file(output.result);
                dump(&mut out, parsed.tree(), gparser::rule_names(), 0);
            }
        }
        Err(error) => {
            writeln!(out, "ABORT={error}").expect("write");
        }
    }
    out.flush().expect("flush");
}
EOF
  cat >> "$DRIVER/Cargo.toml" <<EOF

[[bin]]
name = "$bin"
path = "src/bin/$bin.rs"
EOF
}

emit_bin thrift_base thrift-base thrift ThriftLexer ThriftParser document
emit_bin thrift_fixed thrift-fixed thrift ThriftLexer ThriftParser document
emit_bin edn_base edn-base edn EdnLexer EdnParser program
emit_bin edn_fixed edn-fixed edn EdnLexer EdnParser program
emit_bin rego_base rego-base rego RegoLexer RegoParser root
emit_bin rego_fixed rego-fixed rego RegoLexer RegoParser root

cargo build --release --quiet --manifest-path "$DRIVER/Cargo.toml"
BIN="$DRIVER/target/release"

# 4. Differential: every corpus file, trees + error output must match.
run_corpus() {
  local grammar=$1
  local out_base="$WORK/$grammar-base.txt" out_fixed="$WORK/$grammar-fixed.txt"
  : > "$out_base"; : > "$out_fixed"
  local count=0
  while IFS= read -r file; do
    count=$((count + 1))
    for cfg in base fixed; do
      local sink="$WORK/$grammar-$cfg.txt"
      printf '=== %s\n' "$file" >> "$sink"
      "$BIN/${grammar}_${cfg}" "$file" > "$WORK/.out" 2> "$WORK/.err"
      cat "$WORK/.out" "$WORK/.err" >> "$sink"
    done
  done
  echo "$grammar: $count files"
  if diff -q "$out_base" "$out_fixed" > /dev/null; then
    echo "$grammar: IDENTICAL"
  else
    echo "$grammar: DIVERGED"
    diff "$out_base" "$out_fixed" | head -20
    exit 1
  fi
}
find "$GRAMMARS/thrift/examples" -type f -name '*.thrift' | sort | run_corpus thrift
find "$GRAMMARS/edn/examples" -type f ! -name '*.errors' ! -name '*.tree' | sort | run_corpus edn
find "$GRAMMARS/rego/examples" -type f ! -name '*.errors' ! -name '*.tree' | sort | run_corpus rego

# 5. Interleaved A/B benchmark.
GRAMMARS_DIR="$GRAMMARS" DRIVER_BIN="$BIN" python3 "$(dirname "$0")/bench.py"
