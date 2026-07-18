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


if __name__ == "__main__":
    unittest.main()
