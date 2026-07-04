#!/usr/bin/env python3
"""Run static evals for the bundled skills-reviewer fixtures."""
from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path
from typing import Dict, List, Tuple


def load_reviewer(script_path: Path):
    spec = importlib.util.spec_from_file_location("skill_review", script_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Could not load reviewer script: {script_path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["skill_review"] = module
    spec.loader.exec_module(module)
    return module


def check_case(case: Dict[str, object], evals_root: Path, reviewer) -> Tuple[bool, List[str]]:
    errors: List[str] = []
    fixture = evals_root / str(case["fixture"])
    result = reviewer.review_skill(fixture)
    expect = case.get("expect", {}) if isinstance(case.get("expect", {}), dict) else {}

    expected_verdict = expect.get("verdict")
    if expected_verdict and result["verdict"] != expected_verdict:
        errors.append(f"expected verdict {expected_verdict}, got {result['verdict']}")

    min_avg = expect.get("min_average_score")
    if isinstance(min_avg, (int, float)) and float(result["average_score"]) < float(min_avg):
        errors.append(f"expected average >= {min_avg}, got {result['average_score']}")

    max_avg = expect.get("max_average_score")
    if isinstance(max_avg, (int, float)) and float(result["average_score"]) > float(max_avg):
        errors.append(f"expected average <= {max_avg}, got {result['average_score']}")

    findings = result.get("findings", [])
    codes = {f.get("code") for f in findings if isinstance(f, dict)}
    severities = {f.get("severity") for f in findings if isinstance(f, dict)}

    for code in expect.get("must_find_codes", []) or []:
        if code not in codes:
            errors.append(f"missing expected finding code {code}; got {sorted(codes)}")

    for code in expect.get("must_not_find_codes", []) or []:
        if code in codes:
            errors.append(f"unexpected finding code {code}")

    for severity in expect.get("must_not_find_severities", []) or []:
        if severity in severities:
            errors.append(f"unexpected severity {severity}")

    return len(errors) == 0, errors


def main() -> int:
    parser = argparse.ArgumentParser(description="Run skills-reviewer eval fixtures.")
    parser.add_argument("--evals", default="evals/evals.json", help="Path to evals.json")
    args = parser.parse_args()

    evals_path = Path(args.evals).resolve()
    package_root = evals_path.parent.parent
    reviewer = load_reviewer(package_root / "scripts" / "skill_review.py")
    data = json.loads(evals_path.read_text(encoding="utf-8"))
    cases = data.get("cases", [])

    total = 0
    passed = 0
    failures: List[str] = []
    for case in cases:
        total += 1
        ok, errors = check_case(case, evals_path.parent, reviewer)
        case_id = case.get("id", f"case-{total}")
        if ok:
            passed += 1
            print(f"PASS {case_id}")
        else:
            print(f"FAIL {case_id}")
            for err in errors:
                print(f"  - {err}")
            failures.append(str(case_id))

    print(f"\n{passed}/{total} evals passed")
    if failures:
        print("Failures: " + ", ".join(failures))
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
