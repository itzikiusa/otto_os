#!/usr/bin/env python3
"""Run the package's deterministic, fixture-backed evaluations."""

import argparse
import json
import re
import sys
from pathlib import Path

import audit_repo_bundle
import inventory_repo


def _activation(prompt: str) -> bool:
    lower = prompt.lower()
    negative = "code review" in lower or "review this pull request" in lower
    durable_docs = bool(re.search(r"\b(vault|document|documentation|index|scan)\b", lower))
    explicitly_no_docs = "do not create vault documentation" in lower
    return durable_docs and not negative and not explicitly_no_docs


def _evaluate(package: Path, case: dict):
    errors = []
    fixture = package / "evals" / case["fixture"]
    if not fixture.exists():
        return [f"fixture does not exist: {case['fixture']}"]
    kind = case["kind"]
    if kind == "inventory":
        result = inventory_repo.inventory(fixture)
        kinds = {item["kind"] for item in result["candidates"]}
        missing = set(case.get("expect_kinds", [])) - kinds
        if missing:
            errors.append(f"missing candidate kinds: {sorted(missing)}")
        if result["mode"] != case.get("expect_mode"):
            errors.append(f"mode={result['mode']!r}")
        if result["counts"]["files_scanned"] < case.get("minimum_scanned_files", 0):
            errors.append("too few scanned files")
    elif kind == "audit":
        manifest = fixture / case.get("manifest", "manifest.json")
        findings = audit_repo_bundle.audit(fixture, manifest)
        rules = {item["rule"] for item in findings}
        if case.get("expect_clean") is True and findings:
            errors.append(f"expected clean audit, got {sorted(rules)}")
        for rule in case.get("expect_rules", []):
            if rule not in rules:
                errors.append(f"missing audit rule {rule}")
    elif kind == "activation":
        actual = _activation(case["prompt"])
        if actual is not case["expect_activation"]:
            errors.append(f"activation={actual}")
    elif kind == "skill-contract":
        skill = re.sub(r"\s+", " ", (package / "SKILL.md").read_text(encoding="utf-8").lower())
        for phrase in case.get("expect_phrases", []):
            if phrase.lower() not in skill:
                errors.append(f"SKILL.md lacks {phrase!r}")
    else:
        errors.append(f"unknown eval kind {kind!r}")
    return errors


def run(package: Path):
    package = package.resolve()
    config = json.loads((package / "evals" / "evals.json").read_text(encoding="utf-8"))
    results = []
    for case in config.get("cases", []):
        errors = _evaluate(package, case)
        results.append({"id": case.get("id", "missing-id"), "passed": not errors, "errors": errors})
    return results


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", nargs="?", type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args(argv)
    results = run(args.package)
    print(json.dumps(results, indent=2, sort_keys=True))
    return 1 if any(not item["passed"] for item in results) else 0


if __name__ == "__main__":
    sys.exit(main())
