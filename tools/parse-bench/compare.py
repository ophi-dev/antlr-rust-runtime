#!/usr/bin/env python3
"""Compare two parse-benchmark JSON reports and fail on Rust regressions."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def result_key(result: dict) -> tuple[str, str, str]:
    return (
        str(result["language"]),
        str(result["fixture"]),
        str(result["runtime"]),
    )


def load_results(path: Path) -> dict[tuple[str, str, str], dict]:
    data = json.loads(path.read_text())
    return {result_key(result): result for result in data["results"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--current", required=True, type=Path)
    parser.add_argument("--max-regression", type=float, default=1.15)
    parser.add_argument(
        "--runtime",
        action="append",
        default=None,
        help="Runtime to compare; repeat for multiple runtimes.",
    )
    args = parser.parse_args()

    baseline = load_results(args.baseline)
    current = load_results(args.current)
    runtimes = set(args.runtime or ["rust-antlr"])

    failures: list[str] = []
    for key, head in sorted(current.items()):
        language, fixture, runtime = key
        if runtime not in runtimes or key not in baseline:
            continue
        base_avg = float(baseline[key]["avg_ns"])
        head_avg = float(head["avg_ns"])
        if base_avg <= 0:
            continue
        ratio = head_avg / base_avg
        if ratio > args.max_regression:
            failures.append(
                f"{language}/{fixture} {runtime}: "
                f"{head_avg / 1_000_000:.3f}ms vs "
                f"{base_avg / 1_000_000:.3f}ms ({ratio:.2f}x)"
            )

    if failures:
        print(
            f"parse benchmark regression exceeds {args.max_regression:.2f}x:",
            file=sys.stderr,
        )
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    compared = sum(
        1
        for key in current
        if key in baseline and key[2] in runtimes
    )
    print(
        f"parse benchmark compare passed: {compared} result(s), "
        f"threshold {args.max_regression:.2f}x"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
