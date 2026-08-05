"""Pinned execution surface from grammars-v4 c/Rust/transformGrammar.py.

Source commit: 42b1ea521705c126d84331186b221c9819db1058
Source blob: 1f77ae3762e6a1bcdf49453e688749e34d9fbc76
The fixture retains the current script's imports, staged rename/rewrite, support
copy, and the predicate/action forms exercised by its C grammar.
"""
import re
import shutil
import sys
from glob import glob
from pathlib import Path


def _lt(k: int) -> str:
    return (
        f"recog.input.lt({k})"
        f".map(|t| t.get_text().to_owned())"
        f".unwrap_or_default()"
    )


_IS_TYPEDEF_NAME = (
    f"let __t1 = {_lt(1)}; "
    "crate::c_parser_base::is_typedef_name(&__t1)"
)

ALL_REPLACEMENTS = [
    (
        r"\{this\.IsTypedefName\(\)\}\?",
        "{" + _IS_TYPEDEF_NAME + "}?",
    ),
    (
        r"\{this\.EnterScope\(\);\}",
        "{crate::c_parser_base::enter_scope();}",
    ),
    (
        r"\{this\.ExitScope\(\);\}",
        "{crate::c_parser_base::exit_scope();}",
    ),
]


def needs_transform() -> bool:
    for f in glob("./*.g4"):
        with open(f, encoding="utf-8") as fp:
            content = fp.read()
            for line in content.splitlines():
                if not line.lstrip().startswith("//") and "this." in line:
                    return True
    return False


def transform_grammar(file_path: str) -> None:
    src = Path(file_path)
    if not src.is_file():
        print(f"Not found: {file_path}", file=sys.stderr)
        sys.exit(1)
    shutil.move(file_path, file_path + ".bak")
    with open(file_path + ".bak", encoding="utf-8") as inp, \
         open(file_path, "w", encoding="utf-8") as out:
        for line in inp:
            if not line.lstrip().startswith("//"):
                for pattern, replacement in ALL_REPLACEMENTS:
                    line = re.sub(pattern, replacement, line)
            out.write(line)


def copy_support() -> None:
    source = Path(__file__).resolve().parent / "c_parser_base.rs"
    destination = Path("src") / "c_parser_base.rs"
    if source.is_file() and not destination.exists():
        shutil.copy(source, destination)


if __name__ == "__main__":
    if needs_transform():
        for file in glob("./*.g4"):
            transform_grammar(file)
    copy_support()
