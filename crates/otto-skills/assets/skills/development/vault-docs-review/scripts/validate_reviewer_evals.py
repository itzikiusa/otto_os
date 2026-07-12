#!/usr/bin/env python3
"""Validate reviewer eval fixtures and the shared findings contract."""

import argparse
import json
import sys
from pathlib import Path

REQUIRED = {"severity", "category", "summary", "evidence", "missed_item", "required_fix"}
SEVERITIES = {"blocking", "major", "minor"}
CATEGORIES = {"coverage", "api", "data", "runtime", "evidence", "quality"}


def _load(path):
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def validate_finding(finding, label):
    errors = []
    if not isinstance(finding, dict) or set(finding) != REQUIRED:
        return [f"{label}: finding keys must be exactly {sorted(REQUIRED)}"]
    if finding["severity"] not in SEVERITIES:
        errors.append(f"{label}: invalid severity")
    if finding["category"] not in CATEGORIES:
        errors.append(f"{label}: invalid category")
    if not all(isinstance(finding[key], str) and finding[key].strip() for key in ("category", "summary", "missed_item", "required_fix")):
        errors.append(f"{label}: textual finding fields must be non-empty")
    evidence = finding["evidence"]
    if not isinstance(evidence, list) or not evidence:
        errors.append(f"{label}: evidence must be a non-empty array")
    else:
        for index, item in enumerate(evidence):
            repo = isinstance(item, dict) and set(item) == {"repo_path", "line"} and isinstance(item["repo_path"], str) and bool(item["repo_path"].strip()) and isinstance(item["line"], int) and not isinstance(item["line"], bool) and item["line"] > 0
            doc = isinstance(item, dict) and set(item) == {"doc_path", "section"} and isinstance(item["doc_path"], str) and bool(item["doc_path"].strip()) and isinstance(item["section"], str) and bool(item["section"].strip())
            if not (repo or doc):
                errors.append(f"{label}: evidence[{index}] is not a repo or doc location")
    return errors


def validate_package(root):
    errors = []
    evals_path = root / "evals" / "evals.json"
    data = _load(evals_path)
    cases = data.get("cases", [])
    seeded = clean = False
    for case in cases:
        fixture_name = case.get("fixture")
        if not fixture_name:
            errors.append(f"{root.name}:{case.get('id')}: every eval case needs a fixture")
            continue
        fixture_path = root / "evals" / fixture_name
        if not fixture_path.is_file():
            errors.append(f"{root.name}:{case.get('id')}: missing fixture {fixture_name}")
            continue
        fixture = _load(fixture_path)
        if not isinstance(fixture.get("repo"), list) or not isinstance(fixture.get("bundle"), list):
            errors.append(f"{root.name}:{case.get('id')}: fixture needs repo and bundle arrays")
        findings = fixture.get("expected_findings")
        if not isinstance(findings, list):
            errors.append(f"{root.name}:{case.get('id')}: expected_findings must be an array")
            continue
        for index, finding in enumerate(findings):
            errors.extend(validate_finding(finding, f"{root.name}:{case.get('id')}[{index}]"))
        required = case.get("expect", {}).get("must_find_all", [])
        serialized_findings = json.dumps(findings).lower()
        for term in required:
            if str(term).lower() not in serialized_findings:
                errors.append(f"{root.name}:{case.get('id')}: expected finding token {term!r} is absent")
        seeded |= bool(findings)
        clean |= findings == []
    if not seeded:
        errors.append(f"{root.name}: no fixture-backed seeded finding")
    if not clean:
        errors.append(f"{root.name}: no fixture-backed clean convergence case")
    example = _load(root / "examples" / "review-output.json")
    for index, finding in enumerate(example):
        errors.extend(validate_finding(finding, f"{root.name}:example[{index}]"))
    return errors


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("packages", nargs="+", type=Path)
    args = parser.parse_args(argv)
    errors = []
    for package in args.packages:
        errors.extend(validate_package(package))
    if errors:
        print("\n".join(errors))
        return 1
    print(f"REVIEWER EVALS CLEAN ({len(args.packages)} packages)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
