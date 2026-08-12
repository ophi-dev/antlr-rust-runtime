# SPDX-License-Identifier: BSD-3-Clause
# Copyright (c) 2026 Konstantin Vyatkin
"""Pinned execution surface from grammars-v4 java/java/Rust/transformGrammar.py.

Source commit: 42b1ea521705c126d84331186b221c9819db1058
Source blob: 9547fea684f0e54a8d38621cd78ed04170304a74
Only the unrelated record-component replacement and phase-2 generated-file
patch are omitted from this focused fixture.
"""
import sys
import re
import shutil
from glob import glob
from pathlib import Path

_IS_NOT_IDENTIFIER_ASSIGN = (
    "(!matches!(recog.input.la(1), "
    "JavaParser_IDENTIFIER | JavaParser_MODULE | JavaParser_OPEN | "
    "JavaParser_REQUIRES | JavaParser_EXPORTS | JavaParser_OPENS | "
    "JavaParser_TO | JavaParser_USES | JavaParser_PROVIDES | "
    "JavaParser_WHEN | JavaParser_WITH | JavaParser_TRANSITIVE | "
    "JavaParser_YIELD | JavaParser_SEALED | JavaParser_PERMITS | "
    "JavaParser_RECORD | JavaParser_VAR) "
    "|| recog.input.la(2) != JavaParser_ASSIGN)"
)


def needs_transform():
    for f in glob("./*.g4"):
        with open(f, encoding="utf-8") as fp:
            if "this." in fp.read():
                return True
    return False


def transform_grammar(file_path):
    print("Altering " + file_path)
    if not Path(file_path).is_file():
        print(f"Could not find file: {file_path}")
        sys.exit(1)
    shutil.move(file_path, file_path + ".bak")
    with open(file_path + ".bak", "r", encoding="utf-8") as input_file:
        with open(file_path, "w", encoding="utf-8") as output_file:
            for line in input_file:
                line = re.sub(
                    r"this\.IsNotIdentifierAssign\(\)",
                    _IS_NOT_IDENTIFIER_ASSIGN,
                    line,
                )
                output_file.write(line)


if __name__ == "__main__" and needs_transform():
    for file in glob("./*.g4"):
        transform_grammar(file)
