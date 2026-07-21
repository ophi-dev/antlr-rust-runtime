#!/usr/bin/env python3

import json
import pathlib
import sys
import xml.etree.ElementTree as ET


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: read-junit-report.py SUREFIRE_REPORT_DIRECTORY")

    report_dir = pathlib.Path(sys.argv[1])
    cases = []
    properties = {}
    report_files = sorted(report_dir.glob("TEST-*.xml"))
    for report_path in report_files:
        suite = ET.parse(report_path).getroot()
        if not properties:
            properties = {
                element.attrib["name"]: element.attrib.get("value", "")
                for element in suite.findall("./properties/property")
            }
        for testcase in suite.findall("testcase"):
            skipped = testcase.find("skipped")
            failure = testcase.find("failure")
            error = testcase.find("error")
            if failure is not None:
                status = "failed"
                failure_message = failure.attrib.get("message", "")
            elif error is not None:
                status = "error"
                failure_message = error.attrib.get("message", "")
            elif skipped is not None:
                status = "skipped"
                failure_message = None
            else:
                status = "passed"
                failure_message = None
            cases.append(
                {
                    "classname": testcase.attrib["classname"],
                    "name": testcase.attrib["name"],
                    "status": status,
                    "skip_reason": (
                        skipped.attrib.get("message", "") if skipped is not None else None
                    ),
                    "failure_message": failure_message,
                }
            )

    json.dump(
        {
            "report_file_count": len(report_files),
            "java_runtime": {
                "vendor": properties.get("java.vm.vendor"),
                "version": properties.get("java.runtime.version"),
                "vm": properties.get("java.vm.name"),
            },
            "cases": cases,
        },
        sys.stdout,
        ensure_ascii=True,
        separators=(",", ":"),
    )


if __name__ == "__main__":
    main()
