#!/usr/bin/env python3
"""Compare two parse-benchmark JSON reports and fail on Rust regressions."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


DEFAULT_BENCHMARK_VARIANT = "default"


def result_key(result: dict) -> tuple[str, str, str]:
    return (
        str(result["language"]),
        str(result["fixture"]),
        str(result["runtime"]),
    )


def load_results(path: Path) -> dict[tuple[str, str, str], dict]:
    data = json.loads(path.read_text())
    indexed: dict[tuple[str, str, str], dict] = {}
    for result in data["results"]:
        key = result_key(result)
        if key in indexed:
            raise ValueError(f"duplicate benchmark result key in {path}: {key}")
        indexed[key] = result
    return indexed


def result_variant(result: dict) -> str:
    return str(result.get("benchmark_variant", DEFAULT_BENCHMARK_VARIANT))


def parse_speedup_requirement(value: str) -> tuple[str, str, str, float]:
    parts = value.split(":")
    if len(parts) != 4:
        raise argparse.ArgumentTypeError(
            "expected LANGUAGE:FAST_RUNTIME:SLOW_RUNTIME:MIN_RATIO"
        )
    language, fast_runtime, slow_runtime, ratio_text = parts
    try:
        min_ratio = float(ratio_text)
    except ValueError as error:
        raise argparse.ArgumentTypeError(
            f"invalid MIN_RATIO {ratio_text!r}: {error}"
        ) from error
    if min_ratio <= 0:
        raise argparse.ArgumentTypeError("MIN_RATIO must be greater than zero")
    return language, fast_runtime, slow_runtime, min_ratio


def check_speedup_requirements(
    results: dict[tuple[str, str, str], dict],
    requirements: list[tuple[str, str, str, float]],
) -> list[str]:
    failures: list[str] = []
    fixtures_by_language = sorted({(language, fixture) for language, fixture, _ in results})
    for language, fast_runtime, slow_runtime, min_ratio in requirements:
        compared = 0
        for fixture_language, fixture in fixtures_by_language:
            if fixture_language != language:
                continue
            fast = results.get((language, fixture, fast_runtime))
            slow = results.get((language, fixture, slow_runtime))
            if fast is None or slow is None:
                # A speedup requirement applies to every fixture in the language;
                # missing either runtime is a failure, not a silent skip.
                missing = [
                    runtime
                    for runtime, value in ((fast_runtime, fast), (slow_runtime, slow))
                    if value is None
                ]
                failures.append(
                    f"{language}/{fixture}: missing runtime result(s) for "
                    f"speedup requirement: {', '.join(missing)}"
                )
                continue
            compared += 1
            fast_avg = float(fast["avg_ns"])
            slow_avg = float(slow["avg_ns"])
            if fast_avg <= 0:
                failures.append(
                    f"{language}/{fixture} {fast_runtime}: invalid avg_ns {fast_avg}"
                )
                continue
            ratio = slow_avg / fast_avg
            if ratio < min_ratio:
                failures.append(
                    f"{language}/{fixture}: {fast_runtime} speedup over "
                    f"{slow_runtime} is {ratio:.2f}x, below {min_ratio:.2f}x "
                    f"({fast_avg / 1_000_000:.3f}ms vs "
                    f"{slow_avg / 1_000_000:.3f}ms)"
                )
        if compared == 0:
            failures.append(
                f"{language}: no comparable {fast_runtime}/{slow_runtime} "
                "results for speedup requirement"
            )
    return failures


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
    parser.add_argument(
        "--allow-empty",
        action="store_true",
        help="Exit successfully when there are no matching baseline/current results.",
    )
    parser.add_argument(
        "--require-speedup",
        action="append",
        default=[],
        type=parse_speedup_requirement,
        metavar="LANGUAGE:FAST_RUNTIME:SLOW_RUNTIME:MIN_RATIO",
        help=(
            "Require FAST_RUNTIME to be at least MIN_RATIO faster than "
            "SLOW_RUNTIME for every fixture in LANGUAGE; repeat for multiple "
            "requirements."
        ),
    )
    args = parser.parse_args()

    baseline = load_results(args.baseline)
    current = load_results(args.current)
    runtimes = set(args.runtime or ["rust-antlr"])

    regression_failures: list[str] = []
    variant_mismatches: list[tuple[tuple[str, str, str], str, str]] = []
    compared = 0
    for key, head in sorted(current.items()):
        language, fixture, runtime = key
        base = baseline.get(key)
        if runtime not in runtimes or base is None:
            continue
        base_variant = result_variant(base)
        head_variant = result_variant(head)
        if base_variant != head_variant:
            variant_mismatches.append((key, base_variant, head_variant))
            continue
        compared += 1
        base_avg = float(base["avg_ns"])
        head_avg = float(head["avg_ns"])
        if base_avg <= 0:
            continue
        ratio = head_avg / base_avg
        if ratio > args.max_regression:
            regression_failures.append(
                f"{language}/{fixture} {runtime}: "
                f"{head_avg / 1_000_000:.3f}ms vs "
                f"{base_avg / 1_000_000:.3f}ms ({ratio:.2f}x)"
            )
    speedup_failures = check_speedup_requirements(current, args.require_speedup)

    if variant_mismatches:
        print(
            "parse benchmark compare skipped "
            f"{len(variant_mismatches)} result(s) with changed benchmark variants:"
        )
        for (language, fixture, runtime), base_variant, head_variant in variant_mismatches:
            print(
                f"  {language}/{fixture} {runtime}: "
                f"{base_variant} -> {head_variant}"
            )
    if compared == 0:
        message = (
            "parse benchmark compare found no matching baseline/current "
            f"result pairs for runtime(s): {', '.join(sorted(runtimes))}"
        )
        if args.allow_empty:
            print(f"{message}; skipping regression comparison")
        else:
            print(message, file=sys.stderr)
            return 1

    if regression_failures:
        print(
            f"parse benchmark regression exceeds {args.max_regression:.2f}x:",
            file=sys.stderr,
        )
        for failure in regression_failures:
            print(f"  {failure}", file=sys.stderr)
    if speedup_failures:
        print("parse benchmark speedup requirement failed:", file=sys.stderr)
        for failure in speedup_failures:
            print(f"  {failure}", file=sys.stderr)
    if regression_failures or speedup_failures:
        return 1

    print(
        f"parse benchmark compare passed: {compared} result(s), "
        f"threshold {args.max_regression:.2f}x"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
