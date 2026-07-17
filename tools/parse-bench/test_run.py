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
