import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


RUN_PATH = Path(__file__).with_name("run.py")
SPEC = importlib.util.spec_from_file_location("parse_bench_run", RUN_PATH)
assert SPEC is not None and SPEC.loader is not None
RUN = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = RUN
SPEC.loader.exec_module(RUN)


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
