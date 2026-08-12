#!/usr/bin/env python3
# SPDX-License-Identifier: BSD-3-Clause
# Copyright (c) 2026 Konstantin Vyatkin
"""Same-machine interleaved A/B benchmark: baseline vs --fixed-lookahead.

Driven by run.sh (env: GRAMMARS_DIR, DRIVER_BIN). For each corpus file the
base and fixed drivers run alternately, each reporting the minimum of N
in-process parses; a second interleaved round keeps the per-file minimum.
Reports total min-parse time, per-file geomean speedup, and cold (first
in-process parse) time on the largest file, min over 10 process samples.
"""

import math
import os
import subprocess
from pathlib import Path

GRAMMARS_DIR = Path(os.environ["GRAMMARS_DIR"])
DRIVER_BIN = Path(os.environ["DRIVER_BIN"])
CORPORA = {
    "thrift": (GRAMMARS_DIR / "thrift/examples", "*.thrift", 25),
    "edn": (GRAMMARS_DIR / "edn/examples", "*", 25),
    "rego": (GRAMMARS_DIR / "rego/examples", "*.stmt", 7),
}


def min_us(binary: Path, source: Path, iters: int) -> int:
    out = subprocess.run(
        [binary, str(source), "--quiet", "--time", "--iters", str(iters)],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    for line in out.splitlines():
        if line.startswith("TIME_MIN_US="):
            return int(line.split("=", 1)[1])
    raise SystemExit(f"no timing line from {binary} {source}")


def main() -> None:
    for grammar, (root, glob, iters) in CORPORA.items():
        files = sorted(
            f
            for f in root.rglob(glob)
            if f.is_file() and not f.name.endswith((".errors", ".tree"))
        )
        base_total = 0
        fixed_total = 0
        ratios = []
        for source in files:
            base = min_us(DRIVER_BIN / f"{grammar}_base", source, iters)
            fixed = min_us(DRIVER_BIN / f"{grammar}_fixed", source, iters)
            base = min(base, min_us(DRIVER_BIN / f"{grammar}_base", source, iters))
            fixed = min(fixed, min_us(DRIVER_BIN / f"{grammar}_fixed", source, iters))
            base_total += base
            fixed_total += fixed
            if base > 0 and fixed > 0:
                ratios.append(base / fixed)
        geomean = (
            math.exp(sum(map(math.log, ratios)) / len(ratios)) if ratios else float("nan")
        )
        largest = max(files, key=lambda f: f.stat().st_size)
        cold_base = min(min_us(DRIVER_BIN / f"{grammar}_base", largest, 1) for _ in range(10))
        cold_fixed = min(
            min_us(DRIVER_BIN / f"{grammar}_fixed", largest, 1) for _ in range(10)
        )
        print(
            f"{grammar}: files={len(files)} "
            f"total_min base={base_total / 1000:.2f}ms fixed={fixed_total / 1000:.2f}ms "
            f"({base_total / max(fixed_total, 1):.3f}x) geomean={geomean:.3f}x "
            f"cold({largest.name}) base={cold_base / 1000:.2f}ms fixed={cold_fixed / 1000:.2f}ms"
        )


if __name__ == "__main__":
    main()
