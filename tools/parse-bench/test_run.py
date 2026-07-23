import contextlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


RUN_PATH = Path(__file__).with_name("run.py")
SPEC = importlib.util.spec_from_file_location("parse_bench_run", RUN_PATH)
assert SPEC is not None and SPEC.loader is not None
RUN = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = RUN
SPEC.loader.exec_module(RUN)

COMPARE_PATH = Path(__file__).with_name("compare.py")
COMPARE_SPEC = importlib.util.spec_from_file_location("parse_bench_compare", COMPARE_PATH)
assert COMPARE_SPEC is not None and COMPARE_SPEC.loader is not None
COMPARE = importlib.util.module_from_spec(COMPARE_SPEC)
sys.modules[COMPARE_SPEC.name] = COMPARE
COMPARE_SPEC.loader.exec_module(COMPARE)


class DumpTreeHelpersTests(unittest.TestCase):
    def generated_sources(self) -> tuple[str, str]:
        with tempfile.TemporaryDirectory() as temp:
            work_dir = Path(temp)
            spec = RUN.LANGUAGES["java"]
            rust_runner = RUN.write_rust_runner(
                work_dir,
                [spec],
                RUN.ROOT,
                rust_thin_lto=False,
            )
            go_runner = RUN.write_go_runner(work_dir, [spec])
            return (
                (rust_runner / "src" / "main.rs").read_text(),
                (go_runner / "main.go").read_text(),
            )

    def test_detects_error_nodes(self) -> None:
        clean = 'Rule(root, children=1)\n  Term("x")\n'
        dirty = 'Rule(root, children=1)\n  Err("!")\n'
        self.assertFalse(RUN.dump_tree_has_error_nodes(clean))
        self.assertTrue(RUN.dump_tree_has_error_nodes(dirty))

    def test_format_tree_diff_mentions_both_runtimes(self) -> None:
        diff = RUN.format_tree_diff("Rule(a, children=0)\n", "Rule(b, children=0)\n")
        self.assertIn("rust-antlr", diff)
        self.assertIn("go-antlr", diff)
        self.assertIn("-Rule(a, children=0)", diff)
        self.assertIn("+Rule(b, children=0)", diff)

    def test_generated_dumpers_share_unicode_scalar_encoding(self) -> None:
        rust_source, go_source = self.generated_sources()

        self.assertIn("tree_text(tree.as_terminal()", rust_source)
        self.assertIn("tree_text(tree.as_error()", rust_source)
        self.assertIn("escaped.extend(ch.escape_unicode());", rust_source)
        self.assertNotIn("Term({:?})", rust_source)
        self.assertNotIn("Err({:?})", rust_source)

        self.assertIn("treeText(node.GetText())", go_source)
        self.assertIn(r"fmt.Fprintf(&b, `\u{%x}`, r)", go_source)
        self.assertNotIn("rustDebugStr", go_source)

        sample = "\u0301\u00a0\u2028a"
        expected = '"\\u{301}\\u{a0}\\u{2028}\\u{61}"'
        self.assertEqual(
            '"' + "".join(f"\\u{{{ord(ch):x}}}" for ch in sample) + '"',
            expected,
        )

    def test_generated_dumpers_reject_lexer_and_parser_diagnostics(self) -> None:
        rust_source, go_source = self.generated_sources()
        rust_dump = rust_source[rust_source.index("fn dump_tree_java") :]
        go_dump = go_source[go_source.index("func dumpTreeJava") :]

        rust_fill = rust_dump.index("tokens.fill();")
        rust_lexer_errors = rust_dump.index("tokens.drain_source_errors().len()")
        rust_parse = rust_dump.index("let root = parser")
        self.assertLess(rust_fill, rust_lexer_errors)
        self.assertLess(rust_lexer_errors, rust_parse)
        self.assertIn(
            "if lexer_errors != 0 || syntax_errors != 0",
            rust_dump,
        )

        go_listener = go_dump.index("lexer.AddErrorListener(lexerErrors)")
        go_fill = go_dump.index("tokens.Fill()")
        go_parse = go_dump.index("tree := p.CompilationUnit()")
        self.assertLess(go_listener, go_fill)
        self.assertLess(go_fill, go_parse)
        self.assertIn(
            "lexerErrors.count != 0 || parserErrors.count != 0",
            go_dump,
        )


class ClearWorkDirTests(unittest.TestCase):
    def test_rejects_runtime_root_and_ancestor(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            runtime_root = root / "checkout"
            runtime_root.mkdir()
            marker = runtime_root / "Cargo.toml"
            marker.touch()

            with self.assertRaises(SystemExit):
                RUN.clear_work_dir(runtime_root, runtime_root)
            with self.assertRaises(SystemExit):
                RUN.clear_work_dir(root, runtime_root)

            self.assertTrue(marker.exists())

    def test_allows_disjoint_and_nested_work_directories(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            runtime_root = root / "checkout"
            runtime_root.mkdir()
            marker = runtime_root / "Cargo.toml"
            marker.touch()

            sibling_work = root / "work"
            sibling_work.mkdir()
            RUN.clear_work_dir(sibling_work, runtime_root)
            self.assertFalse(sibling_work.exists())
            self.assertTrue(marker.exists())

            nested_work = runtime_root / "target" / "parse-bench"
            nested_work.mkdir(parents=True)
            RUN.clear_work_dir(nested_work, runtime_root)
            self.assertFalse(nested_work.exists())
            self.assertTrue(marker.exists())


class RuntimeGrammarPreparationTests(unittest.TestCase):
    JAVA_PARSER = """parser grammar JavaParser;
options {
    tokenVocab = JavaLexer;
    superClass = JavaParserBase;
}
annotationFieldValue:
    { this.IsNotIdentifierAssign() }? annotationValue
    | identifier '=' annotationValue
    ;
recordComponentList
    : recordComponent (',' recordComponent)* { this.DoLastRecordComponent() }?
    ;
"""

    def prepare_java(self, root: Path, runtime: str) -> str:
        grammar_source = root / "grammars-v4" / "java" / "java"
        grammar_source.mkdir(parents=True)
        (grammar_source / "JavaLexer.g4").write_text("lexer grammar JavaLexer;\n")
        (grammar_source / "JavaParser.g4").write_text(self.JAVA_PARSER)
        target = root / runtime
        RUN.prepare_runtime_grammar(
            RUN.LANGUAGES["java"],
            root / "grammars-v4",
            target,
            runtime,
        )
        return (target / "JavaParser.g4").read_text()

    def test_rust_java_grammar_preserves_superclass_and_predicates(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            parser = self.prepare_java(Path(temp), "rust-antlr")

        self.assertEqual(parser, self.JAVA_PARSER)

    def test_portable_java_targets_remove_unavailable_base_predicates(self) -> None:
        for runtime in ("python-antlr", "go-antlr"):
            with self.subTest(runtime=runtime), tempfile.TemporaryDirectory() as temp:
                parser = self.prepare_java(Path(temp), runtime)

            self.assertNotIn("superClass = JavaParserBase", parser)
            self.assertNotIn("IsNotIdentifierAssign", parser)
            self.assertNotIn("DoLastRecordComponent", parser)
            self.assertIn("identifier '=' annotationValue", parser)


class BenchmarkVariantTests(unittest.TestCase):
    @staticmethod
    def result(
        language: str,
        fixture: str,
        avg_ns: int,
        variant: str | None = None,
    ) -> dict[str, object]:
        result: dict[str, object] = {
            "language": language,
            "fixture": fixture,
            "runtime": "rust-antlr",
            "avg_ns": avg_ns,
        }
        if variant is not None:
            result["benchmark_variant"] = variant
        return result

    def compare(
        self,
        baseline: list[dict[str, object]],
        current: list[dict[str, object]],
    ) -> tuple[int, str, str]:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            baseline_path = root / "baseline.json"
            current_path = root / "current.json"
            baseline_path.write_text(json.dumps({"results": baseline}))
            current_path.write_text(json.dumps({"results": current}))
            stdout = io.StringIO()
            stderr = io.StringIO()
            argv = [
                "compare.py",
                "--baseline",
                str(baseline_path),
                "--current",
                str(current_path),
                "--max-regression",
                "1.15",
            ]
            with (
                mock.patch.object(sys, "argv", argv),
                contextlib.redirect_stdout(stdout),
                contextlib.redirect_stderr(stderr),
            ):
                status = COMPARE.main()
        return status, stdout.getvalue(), stderr.getvalue()

    def test_java_rust_parse_results_use_predicate_grammar_variant(self) -> None:
        self.assertEqual(
            RUN.benchmark_variant("java", "rust-antlr", "parse"),
            RUN.JAVA_RUST_PREDICATE_VARIANT,
        )
        self.assertEqual(
            RUN.benchmark_variant("java", "rust-antlr", "lex"),
            RUN.DEFAULT_BENCHMARK_VARIANT,
        )
        self.assertEqual(
            RUN.benchmark_variant("java", "go-antlr", "parse"),
            RUN.DEFAULT_BENCHMARK_VARIANT,
        )

    def test_compare_skips_only_changed_benchmark_variants(self) -> None:
        baseline = [
            self.result("java", "Example.java", 100),
            self.result("kotlin", "Example.kt", 100),
        ]
        current = [
            self.result(
                "java",
                "Example.java",
                1_000,
                RUN.JAVA_RUST_PREDICATE_VARIANT,
            ),
            self.result("kotlin", "Example.kt", 100),
        ]

        status, stdout, stderr = self.compare(baseline, current)

        self.assertEqual(status, 0)
        self.assertIn("skipped 1 result(s)", stdout)
        self.assertIn("passed: 1 result(s)", stdout)
        self.assertEqual(stderr, "")

    def test_compare_allows_only_changed_benchmark_variants(self) -> None:
        baseline = [self.result("java", "Example.java", 100)]
        current = [
            self.result(
                "java",
                "Example.java",
                1_000,
                RUN.JAVA_RUST_PREDICATE_VARIANT,
            )
        ]

        status, stdout, stderr = self.compare(baseline, current)

        self.assertEqual(status, 0)
        self.assertIn("skipped 1 result(s)", stdout)
        self.assertIn("skipping regression comparison", stdout)
        self.assertEqual(stderr, "")

    def test_compare_rejects_unrelated_result_sets(self) -> None:
        baseline = [self.result("java", "Example.java", 100)]
        current = [self.result("kotlin", "Example.kt", 100)]

        status, _, stderr = self.compare(baseline, current)

        self.assertEqual(status, 1)
        self.assertIn("no matching baseline/current result pairs", stderr)

    def test_compare_enforces_threshold_for_matching_variants(self) -> None:
        variant = RUN.JAVA_RUST_PREDICATE_VARIANT
        baseline = [self.result("java", "Example.java", 100, variant)]
        current = [self.result("java", "Example.java", 1_000, variant)]

        status, _, stderr = self.compare(baseline, current)

        self.assertEqual(status, 1)
        self.assertIn("10.00x", stderr)


class RustCodegenFlagsTests(unittest.TestCase):
    def test_combines_native_and_profile_generation(self) -> None:
        self.assertEqual(
            RUN.rust_codegen_flags(
                native=True,
                pgo_generate=Path("/tmp/parse-profraw"),
                pgo_use=None,
            ),
            [
                "-Ctarget-cpu=native",
                "-Cprofile-generate=/tmp/parse-profraw",
            ],
        )

    def test_profile_use_warns_about_missing_functions(self) -> None:
        self.assertEqual(
            RUN.rust_codegen_flags(
                native=False,
                pgo_generate=None,
                pgo_use=Path("/tmp/parse.profdata"),
            ),
            [
                "-Cprofile-use=/tmp/parse.profdata",
                "-Cllvm-args=-pgo-warn-missing-function",
            ],
        )


if __name__ == "__main__":
    unittest.main()
