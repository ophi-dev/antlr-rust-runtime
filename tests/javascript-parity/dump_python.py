#!/usr/bin/env python3
import argparse
import importlib
import json
import sys

from antlr4 import CommonTokenStream, FileStream, Token
from antlr4.tree.Tree import ErrorNode, TerminalNode


def dump_tree(node, parser, out, depth=0):
    pad = "  " * depth
    if isinstance(node, ErrorNode):
        out.write(f"{pad}Err({json.dumps(node.getText(), ensure_ascii=False)})\n")
        return
    if isinstance(node, TerminalNode):
        out.write(f"{pad}Term({json.dumps(node.getText(), ensure_ascii=False)})\n")
        return
    name = parser.ruleNames[node.getRuleIndex()]
    children = list(node.getChildren())
    out.write(f"{pad}Rule({name}, children={len(children)})\n")
    for child in children:
        dump_tree(child, parser, out, depth + 1)


def main():
    arguments = argparse.ArgumentParser()
    arguments.add_argument("--gen-dir", required=True)
    arguments.add_argument("--input", required=True)
    arguments.add_argument("--tokens", action="store_true")
    args = arguments.parse_args()

    sys.path.insert(0, args.gen_dir)
    lexer_class = importlib.import_module("JavaScriptLexer").JavaScriptLexer
    parser_class = importlib.import_module("JavaScriptParser").JavaScriptParser

    lexer = lexer_class(FileStream(args.input, encoding="utf-8"))
    stream = CommonTokenStream(lexer)
    stream.fill()
    if args.tokens:
        for token in stream.tokens:
            if token.type != Token.EOF:
                text = json.dumps(token.text, ensure_ascii=False)
                print(f"{token.type}\t{token.channel}\t{text}")
        return

    parser = parser_class(stream)
    tree = parser.program()
    if parser.getNumberOfSyntaxErrors() != 0:
        raise SystemExit(f"parse produced {parser.getNumberOfSyntaxErrors()} syntax error(s)")
    dump_tree(tree, parser, sys.stdout)


if __name__ == "__main__":
    main()
