#!/usr/bin/env python3
"""Generate ANTLR parsers and benchmark parse throughput across runtimes."""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Iterable


ROOT = Path(__file__).resolve().parents[2]
BENCH_ROOT = Path(__file__).resolve().parent
FIXTURE_ROOT = BENCH_ROOT / "fixtures"
DEFAULT_ANTLR_JAR = Path("/tmp/antlr-cleanroom/tools/antlr-4.13.2-complete.jar")
DEFAULT_GRAMMARS_V4 = Path("/tmp/antlr-cleanroom/grammars-v4")
# ANTLR 4.13.2 still generates Go imports for github.com/antlr4-go/antlr/v4,
# whose latest published module tag is v4.13.1.
GO_ANTLR_RUNTIME = "v4.13.1"


@dataclasses.dataclass(frozen=True)
class LanguageSpec:
    name: str
    grammar_rel: Path
    grammar_files: tuple[str, ...]
    lexer_name: str
    parser_name: str
    rust_lexer_module: str
    rust_parser_module: str
    rust_lexer_type: str
    rust_parser_type: str
    rust_entry: str
    python_entry: str
    go_entry: str
    tree_sitter_name: str
    python_support: tuple[Path, ...] = ()
    go_support: tuple[Path, ...] = ()


LANGUAGES: dict[str, LanguageSpec] = {
    "kotlin": LanguageSpec(
        name="kotlin",
        grammar_rel=Path("kotlin/kotlin"),
        grammar_files=("KotlinLexer.g4", "KotlinParser.g4", "UnicodeClasses.g4"),
        lexer_name="KotlinLexer",
        parser_name="KotlinParser",
        rust_lexer_module="kotlin_lexer",
        rust_parser_module="kotlin_parser",
        rust_lexer_type="KotlinLexer",
        rust_parser_type="KotlinParser",
        rust_entry="kotlin_file",
        python_entry="kotlinFile",
        go_entry="KotlinFile",
        tree_sitter_name="kotlin",
    ),
    "csharp": LanguageSpec(
        name="csharp",
        grammar_rel=Path("csharp/v7"),
        grammar_files=("CSharpLexer.g4", "CSharpParser.g4"),
        lexer_name="CSharpLexer",
        parser_name="CSharpParser",
        rust_lexer_module="c_sharp_lexer",
        rust_parser_module="c_sharp_parser",
        rust_lexer_type="CSharpLexer",
        rust_parser_type="CSharpParser",
        rust_entry="compilation_unit",
        python_entry="compilation_unit",
        go_entry="Compilation_unit",
        tree_sitter_name="csharp",
        python_support=(
            Path("csharp/v7/Python3/CSharpLexerBase.py"),
            Path("csharp/v7/Python3/CSharpParserBase.py"),
        ),
        go_support=(
            Path("csharp/v7/Go/CSharpLexerBase.go"),
            Path("csharp/v7/Go/CSharpParserBase.go"),
        ),
    ),
    "java": LanguageSpec(
        name="java",
        grammar_rel=Path("java/java"),
        grammar_files=("JavaLexer.g4", "JavaParser.g4"),
        lexer_name="JavaLexer",
        parser_name="JavaParser",
        rust_lexer_module="java_lexer",
        rust_parser_module="java_parser",
        rust_lexer_type="JavaLexer",
        rust_parser_type="JavaParser",
        rust_entry="compilation_unit",
        python_entry="compilationUnit",
        go_entry="CompilationUnit",
        tree_sitter_name="java",
    ),
    "trino": LanguageSpec(
        name="trino",
        grammar_rel=Path("sql/trino"),
        grammar_files=("TrinoLexer.g4", "TrinoParser.g4"),
        lexer_name="TrinoLexer",
        parser_name="TrinoParser",
        rust_lexer_module="trino_lexer",
        rust_parser_module="trino_parser",
        rust_lexer_type="TrinoLexer",
        rust_parser_type="TrinoParser",
        rust_entry="parse",
        python_entry="parse",
        go_entry="Parse",
        tree_sitter_name="sql",
    ),
}

RUNTIMES = ("rust-antlr", "python-antlr", "go-antlr", "tree-sitter")


@dataclasses.dataclass(frozen=True)
class Fixture:
    language: str
    path: Path
    source: str
    license: str
    description: str

    @property
    def name(self) -> str:
        return self.path.name

    @property
    def abs_path(self) -> Path:
        return FIXTURE_ROOT / self.path


@dataclasses.dataclass(frozen=True)
class Measurement:
    language: str
    fixture: str
    runtime: str
    min_ns: int
    avg_ns: int
    bytes: int
    source: str
    license: str
    description: str


def run(
    cmd: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    quiet: bool = False,
) -> subprocess.CompletedProcess[str]:
    if not quiet:
        print("+ " + " ".join(cmd))
    completed = subprocess.run(
        cmd,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        if completed.stdout:
            print(completed.stdout, file=sys.stderr)
        if completed.stderr:
            print(completed.stderr, file=sys.stderr)
        raise subprocess.CalledProcessError(
            completed.returncode,
            cmd,
            output=completed.stdout,
            stderr=completed.stderr,
        )
    return completed


def parse_csv(value: str, allowed: Iterable[str], label: str) -> list[str]:
    allowed_set = set(allowed)
    items = [item.strip() for item in value.split(",") if item.strip()]
    unknown = [item for item in items if item not in allowed_set]
    if unknown:
        raise SystemExit(f"unknown {label}: {', '.join(unknown)}")
    return items


def require_path(path: Path, label: str) -> None:
    if not path.exists():
        raise SystemExit(f"{label} does not exist: {path}")


def load_fixtures(languages: set[str]) -> list[Fixture]:
    manifest = json.loads((FIXTURE_ROOT / "manifest.json").read_text())
    fixtures = []
    for item in manifest["fixtures"]:
        language = str(item["language"])
        if language not in languages:
            continue
        fixture = Fixture(
            language=language,
            path=Path(str(item["path"])),
            source=str(item["source"]),
            license=str(item["license"]),
            description=str(item["description"]),
        )
        require_path(fixture.abs_path, f"fixture {fixture.path}")
        fixtures.append(fixture)
    return fixtures


def copy_grammar(spec: LanguageSpec, grammars_v4: Path, target: Path) -> None:
    target.mkdir(parents=True, exist_ok=True)
    source_dir = grammars_v4 / spec.grammar_rel
    for grammar in spec.grammar_files:
        source = source_dir / grammar
        require_path(source, f"{spec.name} grammar")
        shutil.copy2(source, target / grammar)


def transform_csharp_python(grammar_dir: Path) -> None:
    for grammar in grammar_dir.glob("*.g4"):
        text = grammar.read_text()
        text = re.sub(r"!this\.", "not self.", text)
        text = re.sub(r"this\.", "self.", text)
        grammar.write_text(text)


def transform_csharp_go(grammar_dir: Path) -> None:
    for grammar in grammar_dir.glob("*.g4"):
        if grammar.name == "CSharpLexer.g4":
            grammar.write_text(transform_go_lexer_actions(grammar.read_text()))
        elif grammar.name == "CSharpParser.g4":
            grammar.write_text(grammar.read_text().replace("this.", "p."))


def transform_go_lexer_actions(grammar: str) -> str:
    def predicate_replacement(match: re.Match[str]) -> str:
        body = match.group("body").replace("this.", "p.")
        return "{" + body + "}" + match.group("suffix")

    grammar = re.sub(
        r"\{(?P<body>[^{}]*this\.[^{}]*)\}(?P<suffix>\s*\?)",
        predicate_replacement,
        grammar,
        flags=re.DOTALL,
    )
    return grammar.replace("this.", "l.")


def transform_java(grammar_dir: Path) -> None:
    parser = grammar_dir / "JavaParser.g4"
    text = parser.read_text()
    text = text.replace("    superClass = JavaParserBase;\n", "")
    text = re.sub(
        r"annotationFieldValue:\s*\{ this\.IsNotIdentifierAssign\(\) \}\? annotationValue\s*\|\s*identifier '=' annotationValue\s*;",
        "annotationFieldValue:\n    identifier '=' annotationValue\n    | annotationValue\n    ;",
        text,
        flags=re.MULTILINE,
    )
    text = text.replace(
        "recordComponent (',' recordComponent)* { this.DoLastRecordComponent() }?",
        "recordComponent (',' recordComponent)*",
    )
    parser.write_text(text)


def generate_antlr(
    antlr_jar: Path,
    spec: LanguageSpec,
    grammar_dir: Path,
    output_dir: Path,
    language: str | None,
    go_package: str | None = None,
) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    cmd = ["java", "-jar", str(antlr_jar)]
    if language is not None:
        cmd.append(f"-Dlanguage={language}")
    if language == "Go":
        cmd.extend(["-package", go_package or "parser"])
    cmd.extend(["-o", str(output_dir), "-Xexact-output-dir"])
    cmd.extend(spec.grammar_files[:2])
    run(cmd, cwd=grammar_dir)


def prepare_python_support(spec: LanguageSpec, grammars_v4: Path, py_gen_dir: Path) -> None:
    for rel_path in spec.python_support:
        source = grammars_v4 / rel_path
        require_path(source, f"{spec.name} Python support")
        target = py_gen_dir / source.name
        shutil.copy2(source, target)
        if spec.name == "csharp" and source.name == "CSharpLexerBase.py":
            target.write_text(
                target.read_text().replace(
                    'tok = CommonToken(type=CSharpLexer.SKIPPED_SECTION, text="".join(text))',
                    'tok = CommonToken(type=CSharpLexer.SKIPPED_SECTION)\n'
                    '        tok.text = "".join(text)',
                )
            )


def prepare_go_support(
    spec: LanguageSpec,
    grammars_v4: Path,
    go_parser_dir: Path,
    package_name: str,
) -> None:
    for rel_path in spec.go_support:
        source = grammars_v4 / rel_path
        require_path(source, f"{spec.name} Go support")
        target = go_parser_dir / source.name
        shutil.copy2(source, target)
        target.write_text(target.read_text().replace("package parser", f"package {package_name}", 1))


def generate_rust_modules(
    spec: LanguageSpec,
    grammar_dir: Path,
    interp_dir: Path,
    rust_generated_dir: Path,
    require_generated_parser: bool,
) -> None:
    common = [
        "cargo",
        "run",
        "--quiet",
        "--release",
        "--manifest-path",
        str(ROOT / "Cargo.toml"),
        "--bin",
        "antlr4-rust-gen",
        "--",
        "--out-dir",
        str(rust_generated_dir),
    ]
    run(
        [
            *common,
            "--lexer",
            str(interp_dir / f"{spec.lexer_name}.interp"),
            "--grammar",
            str(grammar_dir / spec.grammar_files[0]),
        ]
    )
    parser_cmd = [
        *common,
        "--parser",
        str(interp_dir / f"{spec.parser_name}.interp"),
        "--grammar",
        str(grammar_dir / spec.grammar_files[1]),
    ]
    if require_generated_parser:
        parser_cmd.append("--require-generated-parser")
    run(parser_cmd)


def write_rust_runner(work_dir: Path, specs: list[LanguageSpec]) -> Path:
    runner = work_dir / "rust-runner"
    generated = runner / "src" / "generated"
    generated.mkdir(parents=True, exist_ok=True)
    (runner / "Cargo.toml").write_text(
        "\n".join(
            [
                "[package]",
                'name = "parse-bench-rust-runner"',
                'version = "0.0.0"',
                'edition = "2024"',
                "",
                "[features]",
                "default = []",
                'perf-counters = ["antlr-rust-runtime/perf-counters"]',
                "",
                "[dependencies]",
                f'antlr-rust-runtime = {{ path = "{ROOT}" }}',
                "",
            ]
        )
    )
    modules = "\n".join(
        f"    pub mod {spec.rust_lexer_module};\n    pub mod {spec.rust_parser_module};"
        for spec in specs
    )
    arms = "\n".join(
        f'        "{spec.name}" => parse_{spec.name}(&src).map_err(|err| err.to_string())?,'
        for spec in specs
    )
    functions = "\n\n".join(rust_parse_function(spec) for spec in specs)
    (runner / "src" / "main.rs").write_text(
        f"""#![allow(clippy::print_stdout)]

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use antlr4_runtime::{{CommonTokenStream, InputStream}};

mod generated {{
    #![allow(dead_code, unused_imports, unreachable_pub, unused_qualifications)]
{modules}
}}

{functions}

fn main() -> ExitCode {{
    match run_main() {{
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {{
            eprintln!("{{err}}");
            ExitCode::from(1)
        }}
    }}
}}

fn run_main() -> Result<(), String> {{
    let mut args = env::args().skip(1);
    let mut language: Option<String> = None;
    let mut input: Option<PathBuf> = None;
    let mut iters = 1_usize;
    let mut warmups = 0_usize;
    while let Some(arg) = args.next() {{
        match arg.as_str() {{
            "--language" => language = args.next(),
            "--input" => input = args.next().map(PathBuf::from),
            "--iters" => iters = parse_usize(args.next(), "--iters")?,
            "--warmups" => warmups = parse_usize(args.next(), "--warmups")?,
            other => return Err(format!("unknown argument: {{other}}")),
        }}
    }}
    let language = language.ok_or("missing --language <name>")?;
    let input = input.ok_or("missing --input <path>")?;
    if iters == 0 {{
        return Err("--iters must be greater than 0".to_owned());
    }}
    let src = fs::read_to_string(&input)
        .map_err(|err| format!("failed to read {{}}: {{err}}", input.display()))?;

    for _ in 0..warmups {{
        parse_once(&language, &src)?;
    }}
    #[cfg(feature = "perf-counters")]
    antlr4_runtime::reset_prediction_perf_counters();

    let mut min_ns = u128::MAX;
    let mut total_ns = 0_u128;
    for _ in 0..iters {{
        let started = Instant::now();
        parse_once(&language, &src)?;
        let elapsed = started.elapsed().as_nanos();
        min_ns = min_ns.min(elapsed);
        total_ns += elapsed;
    }}
    let avg_ns = total_ns / iters as u128;
    println!("min_ns={{min_ns}} avg_ns={{avg_ns}}");
    #[cfg(feature = "perf-counters")]
    if env::var_os("ANTLR_PERF_DUMP").is_some() {{
        antlr4_runtime::dump_prediction_perf_counters();
    }}
    Ok(())
}}

fn parse_once(language: &str, src: &str) -> Result<(), String> {{
    match language {{
{arms}
        other => return Err(format!("unsupported language: {{other}}")),
    }}
    Ok(())
}}

fn parse_usize(value: Option<String>, flag: &str) -> Result<usize, String> {{
    let value = value.ok_or_else(|| format!("missing value for {{flag}}"))?;
    value
        .parse::<usize>()
        .map_err(|err| format!("invalid {{flag}} value {{value}}: {{err}}"))
}}
"""
    )
    return runner


def rust_parse_function(spec: LanguageSpec) -> str:
    return f"""fn parse_{spec.name}(src: &str) -> Result<(), antlr4_runtime::AntlrError> {{
    let lexer = generated::{spec.rust_lexer_module}::{spec.rust_lexer_type}::new(InputStream::new(src));
    let tokens = CommonTokenStream::new(lexer);
    let mut parser = generated::{spec.rust_parser_module}::{spec.rust_parser_type}::new(tokens);
    let tree = parser.{spec.rust_entry}()?;
    black_box(&tree);
    Ok(())
}}"""


def write_python_antlr_runner(work_dir: Path, specs: list[LanguageSpec], py_gen_dir: Path) -> Path:
    runner = work_dir / "python-antlr-runner"
    runner.mkdir(parents=True, exist_ok=True)
    spec_lines = ",\n".join(
        f'    "{spec.name}": ("{spec.lexer_name}", "{spec.parser_name}", "{spec.python_entry}")'
        for spec in specs
    )
    script = runner / "bench_antlr.py"
    script.write_text(
        f"""#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib
import sys
import time
from pathlib import Path

from antlr4 import CommonTokenStream, InputStream

GEN_DIR = {str(py_gen_dir)!r}
SPECS = {{
{spec_lines}
}}
SINK = None
CLASSES = {{}}


def parser_classes(language: str):
    if language not in CLASSES:
        lexer_name, parser_name, rule_name = SPECS[language]
        lexer_module = importlib.import_module(lexer_name)
        parser_module = importlib.import_module(parser_name)
        CLASSES[language] = (
            getattr(lexer_module, lexer_name),
            getattr(parser_module, parser_name),
            rule_name,
        )
    return CLASSES[language]


def parse_once(language: str, src: str) -> None:
    global SINK
    lexer_cls, parser_cls, rule_name = parser_classes(language)
    stream = CommonTokenStream(lexer_cls(InputStream(src)))
    parser = parser_cls(stream)
    tree = getattr(parser, rule_name)()
    SINK = tree


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--language", required=True, choices=sorted(SPECS))
    parser.add_argument("--input", required=True)
    parser.add_argument("--iters", type=int, default=1)
    parser.add_argument("--warmups", type=int, default=0)
    args = parser.parse_args()
    if args.iters <= 0:
        raise SystemExit("--iters must be greater than 0")
    sys.path.insert(0, GEN_DIR)
    src = Path(args.input).read_text()
    for _ in range(args.warmups):
        parse_once(args.language, src)
    elapsed = []
    for _ in range(args.iters):
        started = time.perf_counter_ns()
        parse_once(args.language, src)
        elapsed.append(time.perf_counter_ns() - started)
    print(f"min_ns={{min(elapsed)}} avg_ns={{sum(elapsed) // len(elapsed)}}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
"""
    )
    return script


def write_tree_sitter_runner(work_dir: Path, specs: list[LanguageSpec]) -> Path:
    runner = work_dir / "tree-sitter-runner"
    runner.mkdir(parents=True, exist_ok=True)
    spec_lines = ",\n".join(
        f'    "{spec.name}": "{spec.tree_sitter_name}"'
        for spec in specs
    )
    script = runner / "bench_tree_sitter.py"
    script.write_text(
        f"""#!/usr/bin/env python3
from __future__ import annotations

import argparse
import time
from pathlib import Path

from tree_sitter_language_pack import get_parser

SPECS = {{
{spec_lines}
}}
SINK = None


def parse_once(parser, src: bytes):
    global SINK
    tree = parser.parse(src)
    if tree.root_node.has_error:
        raise RuntimeError("tree-sitter parse produced ERROR nodes")
    SINK = tree


def main() -> int:
    arg_parser = argparse.ArgumentParser()
    arg_parser.add_argument("--language", required=True, choices=sorted(SPECS))
    arg_parser.add_argument("--input", required=True)
    arg_parser.add_argument("--iters", type=int, default=1)
    arg_parser.add_argument("--warmups", type=int, default=0)
    args = arg_parser.parse_args()
    if args.iters <= 0:
        raise SystemExit("--iters must be greater than 0")
    parser = get_parser(SPECS[args.language])
    src = Path(args.input).read_bytes()
    for _ in range(args.warmups):
        parse_once(parser, src)
    elapsed = []
    for _ in range(args.iters):
        started = time.perf_counter_ns()
        parse_once(parser, src)
        elapsed.append(time.perf_counter_ns() - started)
    print(f"min_ns={{min(elapsed)}} avg_ns={{sum(elapsed) // len(elapsed)}}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
"""
    )
    return script


def write_go_runner(work_dir: Path, specs: list[LanguageSpec]) -> Path:
    runner = work_dir / "go-runner"
    runner.mkdir(parents=True, exist_ok=True)
    (runner / "go.mod").write_text(
        "\n".join(
            [
                "module parsebench",
                "",
                "go 1.23",
                "",
                f"require github.com/antlr4-go/antlr/v4 {GO_ANTLR_RUNTIME}",
                "",
            ]
        )
    )
    imports = "\n".join(
        f'    {go_package_name(spec)} "parsebench/{go_package_name(spec)}"'
        for spec in specs
    )
    arms = "\n".join(
        f'    case "{spec.name}":\n        parse{go_func_name(spec.name)}(src)'
        for spec in specs
    )
    functions = "\n\n".join(go_parse_function(spec) for spec in specs)
    (runner / "main.go").write_text(
        f"""package main

import (
    "fmt"
    "os"
    "strconv"
    "time"

    "github.com/antlr4-go/antlr/v4"
{imports}
)

var sink any

func main() {{
    if err := runMain(); err != nil {{
        fmt.Fprintln(os.Stderr, err)
        os.Exit(1)
    }}
}}

func runMain() error {{
    language := ""
    input := ""
    iters := 1
    warmups := 0
    for i := 1; i < len(os.Args); i++ {{
        switch os.Args[i] {{
        case "--language":
            i++
            language = requiredArg(i, "--language")
        case "--input":
            i++
            input = requiredArg(i, "--input")
        case "--iters":
            i++
            iters = parsePositiveInt(requiredArg(i, "--iters"), "--iters")
        case "--warmups":
            i++
            warmups = parseNonNegativeInt(requiredArg(i, "--warmups"), "--warmups")
        default:
            return fmt.Errorf("unknown argument: %s", os.Args[i])
        }}
    }}
    if language == "" {{
        return fmt.Errorf("missing --language <name>")
    }}
    if input == "" {{
        return fmt.Errorf("missing --input <path>")
    }}
    bytes, err := os.ReadFile(input)
    if err != nil {{
        return err
    }}
    src := string(bytes)
    for i := 0; i < warmups; i++ {{
        parseOnce(language, src)
    }}
    minNs := int64(1<<63 - 1)
    totalNs := int64(0)
    for i := 0; i < iters; i++ {{
        started := time.Now()
        parseOnce(language, src)
        elapsed := time.Since(started).Nanoseconds()
        if elapsed < minNs {{
            minNs = elapsed
        }}
        totalNs += elapsed
    }}
    fmt.Printf("min_ns=%d avg_ns=%d\\n", minNs, totalNs/int64(iters))
    return nil
}}

func parseOnce(language string, src string) {{
    switch language {{
{arms}
    default:
        panic("unsupported language: " + language)
    }}
}}

func requiredArg(index int, flag string) string {{
    if index >= len(os.Args) {{
        panic("missing value for " + flag)
    }}
    return os.Args[index]
}}

func parsePositiveInt(value string, flag string) int {{
    parsed, err := strconv.Atoi(value)
    if err != nil || parsed <= 0 {{
        panic("invalid " + flag + " value: " + value)
    }}
    return parsed
}}

func parseNonNegativeInt(value string, flag string) int {{
    parsed, err := strconv.Atoi(value)
    if err != nil || parsed < 0 {{
        panic("invalid " + flag + " value: " + value)
    }}
    return parsed
}}

{functions}
"""
    )
    return runner


def go_func_name(name: str) -> str:
    return "".join(part.capitalize() for part in name.split("-"))


def go_package_name(spec: LanguageSpec) -> str:
    return f"{spec.name}parser"


def go_parse_function(spec: LanguageSpec) -> str:
    return f"""func parse{go_func_name(spec.name)}(src string) {{
    input := antlr.NewInputStream(src)
    lexer := {go_package_name(spec)}.New{spec.lexer_name}(input)
    tokens := antlr.NewCommonTokenStream(lexer, antlr.TokenDefaultChannel)
    p := {go_package_name(spec)}.New{spec.parser_name}(tokens)
    tree := p.{spec.go_entry}()
    sink = tree
}}"""


def copy_generated_go(go_gen_dir: Path, go_parser_dir: Path) -> None:
    for path in go_gen_dir.glob("*.go"):
        shutil.copy2(path, go_parser_dir / path.name)


def prepare_work(
    args: argparse.Namespace,
    specs: list[LanguageSpec],
    runtimes: set[str],
) -> dict[str, Path]:
    work_dir = args.work_dir
    clear_work_dir(work_dir)
    work_dir.mkdir(parents=True, exist_ok=True)

    rust_runner = write_rust_runner(work_dir, specs)
    rust_generated = rust_runner / "src" / "generated"
    py_gen = work_dir / "python-gen"
    go_runner = write_go_runner(work_dir, specs)

    for spec in specs:
        if "rust-antlr" in runtimes:
            base_grammar = work_dir / "grammars" / spec.name / "base"
            copy_grammar(spec, args.grammars_v4, base_grammar)
            if spec.name == "java":
                transform_java(base_grammar)
            interp_dir = work_dir / "generated" / spec.name / "interp"
            generate_antlr(args.antlr_jar, spec, base_grammar, interp_dir, None)
            generate_rust_modules(
                spec,
                base_grammar,
                interp_dir,
                rust_generated,
                args.rust_generated_only,
            )

        if "python-antlr" in runtimes:
            py_grammar = work_dir / "grammars" / spec.name / "python"
            copy_grammar(spec, args.grammars_v4, py_grammar)
            if spec.name == "csharp":
                transform_csharp_python(py_grammar)
            if spec.name == "java":
                transform_java(py_grammar)
            py_lang_gen = py_gen
            generate_antlr(args.antlr_jar, spec, py_grammar, py_lang_gen, "Python3")
            prepare_python_support(spec, args.grammars_v4, py_lang_gen)

        if "go-antlr" in runtimes:
            go_grammar = work_dir / "grammars" / spec.name / "go"
            copy_grammar(spec, args.grammars_v4, go_grammar)
            if spec.name == "csharp":
                transform_csharp_go(go_grammar)
            if spec.name == "java":
                transform_java(go_grammar)
            go_gen = work_dir / "generated" / spec.name / "go"
            package_name = go_package_name(spec)
            generate_antlr(args.antlr_jar, spec, go_grammar, go_gen, "Go", package_name)
            go_parser_dir = go_runner / package_name
            go_parser_dir.mkdir(parents=True, exist_ok=True)
            copy_generated_go(go_gen, go_parser_dir)
            prepare_go_support(spec, args.grammars_v4, go_parser_dir, package_name)

    runners: dict[str, Path] = {}
    if "rust-antlr" in runtimes:
        rust_build_cmd = ["cargo", "build", "--quiet", "--release", "--manifest-path", str(rust_runner / "Cargo.toml")]
        if os.environ.get("ANTLR_PERF_DUMP"):
            rust_build_cmd.extend(["--features", "perf-counters"])
        run(rust_build_cmd)
        runners["rust-antlr"] = rust_runner / "target" / "release" / "parse-bench-rust-runner"
    if "python-antlr" in runtimes:
        runners["python-antlr"] = write_python_antlr_runner(work_dir, specs, py_gen)
    if "go-antlr" in runtimes:
        run(["go", "mod", "tidy"], cwd=go_runner)
        run(["go", "build", "-o", "parse-bench-go-runner", "."], cwd=go_runner)
        runners["go-antlr"] = go_runner / "parse-bench-go-runner"
    if "tree-sitter" in runtimes:
        runners["tree-sitter"] = write_tree_sitter_runner(work_dir, specs)
    return runners


def clear_work_dir(work_dir: Path) -> None:
    if not work_dir.exists():
        return
    if not work_dir.is_dir():
        raise SystemExit(f"Refusing to delete non-directory work path: {work_dir}")
    if work_dir == ROOT or work_dir in ROOT.parents:
        raise SystemExit(f"Refusing to delete project root or parent directory: {work_dir}")
    shutil.rmtree(work_dir)


def ensure_python_dependencies(python: str, runtimes: set[str]) -> None:
    modules = []
    if "python-antlr" in runtimes:
        modules.append("antlr4")
    if "tree-sitter" in runtimes:
        modules.append("tree_sitter_language_pack")
    if not modules:
        return
    code = "\n".join(f"import {module}" for module in modules)
    try:
        run([python, "-c", code], quiet=True)
    except subprocess.CalledProcessError as err:
        print(err.stderr, file=sys.stderr)
        raise SystemExit(
            "missing Python benchmark dependencies; run:\n"
            f"  {python} -m pip install -r {BENCH_ROOT / 'requirements.txt'}"
        ) from err


RUNNER_OUTPUT = re.compile(r"min_ns=(?P<min>\d+)\s+avg_ns=(?P<avg>\d+)")


def measure_fixture(
    fixture: Fixture,
    runtime: str,
    runner: Path,
    args: argparse.Namespace,
) -> Measurement:
    env = None
    if runtime == "rust-antlr" and args.rust_generated_only:
        env = os.environ.copy()
        env["ANTLR4_RUST_GENERATED_ONLY"] = "1"
    if runtime in {"python-antlr", "tree-sitter"}:
        cmd = [args.python, str(runner)]
    else:
        cmd = [str(runner)]
    cmd.extend(
        [
            "--language",
            fixture.language,
            "--input",
            str(fixture.abs_path),
            "--iters",
            str(args.iters),
            "--warmups",
            str(args.warmups),
        ]
    )
    completed = run(cmd, env=env, quiet=True)
    match = RUNNER_OUTPUT.search(completed.stdout)
    if match is None:
        raise SystemExit(
            f"{runtime} runner did not report timing for {fixture.path}:\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return Measurement(
        language=fixture.language,
        fixture=fixture.name,
        runtime=runtime,
        min_ns=int(match.group("min")),
        avg_ns=int(match.group("avg")),
        bytes=fixture.abs_path.stat().st_size,
        source=fixture.source,
        license=fixture.license,
        description=fixture.description,
    )


def print_table(results: list[Measurement]) -> None:
    rust_by_fixture = {
        (result.language, result.fixture): result.avg_ns
        for result in results
        if result.runtime == "rust-antlr"
    }
    rows = []
    for result in results:
        rust_avg = rust_by_fixture.get((result.language, result.fixture))
        relative = ""
        if rust_avg:
            relative = format_relative(result.avg_ns / rust_avg)
        rows.append(
            [
                result.language,
                result.fixture,
                result.runtime,
                str(result.bytes),
                f"{result.min_ns / 1_000_000:.3f}",
                f"{result.avg_ns / 1_000_000:.3f}",
                relative,
            ]
        )
    headers = ["language", "fixture", "runtime", "bytes", "min ms", "avg ms", "vs rust"]
    widths = [
        max(len(row[index]) for row in rows + [headers])
        for index in range(len(headers))
    ]
    print(" | ".join(header.ljust(widths[index]) for index, header in enumerate(headers)))
    print("-|-".join("-" * width for width in widths))
    for row in rows:
        print(" | ".join(cell.ljust(widths[index]) for index, cell in enumerate(row)))


def write_json(results: list[Measurement], args: argparse.Namespace) -> None:
    if args.json is None:
        return
    args.json.parent.mkdir(parents=True, exist_ok=True)
    data = {
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "iters": args.iters,
        "warmups": args.warmups,
        "rust_generated_only": args.rust_generated_only,
        "repo": str(ROOT),
        "grammars_v4": {
            "path": str(args.grammars_v4),
            "commit": git_rev(args.grammars_v4),
        },
        "results": [dataclasses.asdict(result) for result in results],
    }
    args.json.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    print(f"wrote JSON report: {args.json}")


def write_markdown(results: list[Measurement], args: argparse.Namespace) -> None:
    if args.markdown is None:
        return
    rust_by_fixture = {
        (result.language, result.fixture): result.avg_ns
        for result in results
        if result.runtime == "rust-antlr"
    }
    lines = [
        "# Parse Benchmark",
        "",
        f"- Iterations: `{args.iters}`",
        f"- Warmups: `{args.warmups}`",
        f"- Rust generated-only: `{'yes' if args.rust_generated_only else 'no'}`",
        f"- grammars-v4: `{git_rev(args.grammars_v4) or args.grammars_v4}`",
        "",
        "| Language | Fixture | Runtime | Bytes | Min ms | Avg ms | vs Rust |",
        "| --- | --- | --- | ---: | ---: | ---: | ---: |",
    ]
    for result in results:
        rust_avg = rust_by_fixture.get((result.language, result.fixture))
        relative = format_relative(result.avg_ns / rust_avg) if rust_avg else ""
        lines.append(
            f"| {result.language} | {result.fixture} | {result.runtime} | "
            f"{result.bytes} | {result.min_ns / 1_000_000:.3f} | "
            f"{result.avg_ns / 1_000_000:.3f} | {relative} |"
        )
    args.markdown.parent.mkdir(parents=True, exist_ok=True)
    args.markdown.write_text("\n".join(lines) + "\n")
    print(f"wrote Markdown report: {args.markdown}")


def format_relative(value: float) -> str:
    if value < 0.01:
        return "<0.01x"
    return f"{value:.2f}x"


def git_rev(path: Path) -> str | None:
    if not (path / ".git").exists():
        return None
    try:
        return run(["git", "rev-parse", "HEAD"], cwd=path, quiet=True).stdout.strip()
    except subprocess.CalledProcessError:
        return None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--antlr-jar",
        type=Path,
        default=Path(os.environ.get("ANTLR4_JAR", DEFAULT_ANTLR_JAR)),
    )
    parser.add_argument(
        "--grammars-v4",
        type=Path,
        default=Path(os.environ.get("GRAMMARS_V4", DEFAULT_GRAMMARS_V4)),
    )
    parser.add_argument("--work-dir", type=Path, default=ROOT / "target" / "parse-bench")
    parser.add_argument("--python", default=os.environ.get("PYTHON", sys.executable))
    parser.add_argument("--languages", default="kotlin,csharp,java")
    parser.add_argument("--runtimes", default="rust-antlr,python-antlr,go-antlr,tree-sitter")
    parser.add_argument("--iters", type=int, default=10)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--quick", action="store_true", help="Use 3 timed iterations and 1 warmup.")
    parser.add_argument(
        "--rust-generated-only",
        action="store_true",
        help=(
            "Require all Rust parser rules to be generated, then run rust-antlr "
            "with ANTLR4_RUST_GENERATED_ONLY=1 so missing coverage fails instead "
            "of falling back to the interpreter."
        ),
    )
    parser.add_argument("--json", type=Path, help="Write machine-readable results.")
    parser.add_argument("--markdown", type=Path, help="Write a Markdown table report.")
    return parser.parse_args()


def normalize_paths(args: argparse.Namespace) -> None:
    args.antlr_jar = args.antlr_jar.expanduser().resolve()
    args.grammars_v4 = args.grammars_v4.expanduser().resolve()
    args.work_dir = args.work_dir.expanduser().resolve()
    if args.json is not None:
        args.json = args.json.expanduser().resolve()
    if args.markdown is not None:
        args.markdown = args.markdown.expanduser().resolve()


def main() -> int:
    args = parse_args()
    normalize_paths(args)
    if args.quick:
        args.iters = 3
        args.warmups = 1
    if args.iters <= 0:
        raise SystemExit("--iters must be greater than 0")
    if args.warmups < 0:
        raise SystemExit("--warmups must be >= 0")

    languages = parse_csv(args.languages, LANGUAGES, "language")
    runtimes = set(parse_csv(args.runtimes, RUNTIMES, "runtime"))
    specs = [LANGUAGES[name] for name in languages]
    fixtures = load_fixtures(set(languages))
    require_path(args.antlr_jar, "ANTLR jar")
    require_path(args.grammars_v4, "grammars-v4 checkout")
    ensure_python_dependencies(args.python, runtimes)

    runners = prepare_work(args, specs, runtimes)

    results: list[Measurement] = []
    for fixture in fixtures:
        for runtime in RUNTIMES:
            if runtime not in runtimes:
                continue
            result = measure_fixture(fixture, runtime, runners[runtime], args)
            results.append(result)
            print(
                f"{fixture.language}/{fixture.name} {runtime}: "
                f"avg {result.avg_ns / 1_000_000:.3f}ms"
            )

    print()
    print_table(results)
    write_json(results, args)
    write_markdown(results, args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
