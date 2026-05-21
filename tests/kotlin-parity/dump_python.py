#!/usr/bin/env python3
"""Parse a source file with an antlr4-python3-runtime parser and dump the parse
tree in a stable, diff-friendly form.

The script imports a generated lexer and parser produced by `antlr -Dlanguage=Python3`
from a directory passed as `--gen-dir`. A common module name is required so the
import works without inventing per-grammar logic; pass `--lexer` and `--parser`
to override.
"""
from __future__ import annotations

import argparse
import importlib
import sys
from pathlib import Path

from antlr4 import CommonTokenStream, InputStream
from antlr4.tree.Tree import TerminalNodeImpl, ErrorNodeImpl


def rust_debug_str(text: str) -> str:
    """Format a Python string the same way Rust's `{:?}` Debug format does for
    `&str`: double-quoted, escaping `"`, `\\`, and standard ASCII control chars.
    Keeps the Rust and Python tree dumps byte-identical so the parity check can
    diff them directly without target-specific repr normalization."""
    out = ['"']
    for ch in text:
        if ch == "\\":
            out.append("\\\\")
        elif ch == '"':
            out.append('\\"')
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\r":
            out.append("\\r")
        elif ch == "\t":
            out.append("\\t")
        elif ch == "\0":
            out.append("\\0")
        elif ord(ch) < 0x20 or ord(ch) == 0x7F:
            out.append(f"\\u{{{ord(ch):x}}}")
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


def dump(node, rule_names, depth=0, out=sys.stdout):
    pad = "  " * depth
    if isinstance(node, TerminalNodeImpl):
        out.write(f"{pad}Term({rust_debug_str(node.getText())})\n")
        return
    if isinstance(node, ErrorNodeImpl):
        out.write(f"{pad}Err({rust_debug_str(node.getText())})\n")
        return
    rule_index = node.getRuleIndex()
    name = rule_names[rule_index] if 0 <= rule_index < len(rule_names) else "<?>"
    children = node.children or []
    out.write(f"{pad}Rule({name}, children={len(children)})\n")
    for child in children:
        dump(child, rule_names, depth + 1, out)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--gen-dir", required=True, help="Directory with Python ANTLR-generated modules")
    parser.add_argument("--lexer", required=True, help="Lexer module name, e.g. KotlinLexer")
    parser.add_argument("--parser", required=True, help="Parser module name, e.g. KotlinParser")
    parser.add_argument("--rule", required=True, help="Entry-point rule name, e.g. kotlinFile")
    parser.add_argument("--input", required=True, help="Source file to parse")
    parser.add_argument("--output", default="-", help="Output path (default: stdout)")
    args = parser.parse_args()

    sys.path.insert(0, args.gen_dir)
    lexer_module = importlib.import_module(args.lexer)
    parser_module = importlib.import_module(args.parser)
    lexer_cls = getattr(lexer_module, args.lexer)
    parser_cls = getattr(parser_module, args.parser)

    src = Path(args.input).read_text()
    stream = CommonTokenStream(lexer_cls(InputStream(src)))
    parser_obj = parser_cls(stream)
    tree = getattr(parser_obj, args.rule)()
    out = sys.stdout if args.output == "-" else open(args.output, "w")
    try:
        dump(tree, parser_obj.ruleNames, out=out)
    finally:
        if out is not sys.stdout:
            out.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
