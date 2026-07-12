#!/usr/bin/env python3
"""Audit repository documentation for coverage and minimum source depth."""

import argparse
import json
import re
import sys
from pathlib import Path

STATUSES = {"documented", "irrelevant", "generated", "uncertain"}
ROW_RE = re.compile(r"^\|\s*`?([^|`]+)`?\s*\|\s*([^|]+)\|\s*([^|]+)\|\s*([^|]+)\|\s*([^|]+)\|\s*([^|]+)\|", re.M)
FENCE_RE = re.compile(r"```(?:json|yaml|yml|sql|text)?\s*\n\s*[^\s`][\s\S]*?```", re.I)
CITATION_RE = re.compile(r"`?[A-Za-z0-9_.\-/]+:\d+`?")


def finding(rule, path, message, severity="error"):
    return {"rule": rule, "path": path, "message": message, "severity": severity}


def _read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError:
        return ""


def _coverage_rows(text: str):
    rows = {}
    duplicates = set()
    for match in ROW_RE.finditer(text):
        candidate = match.group(1).strip()
        if candidate.lower() in {"candidate", "---"}:
            continue
        status = match.group(4).strip().lower().strip("`")
        if candidate in rows:
            duplicates.add(candidate)
        rows[candidate] = {"status": status, "doc": match.group(5).strip(), "reason": match.group(6).strip()}
    return rows, duplicates


def audit(root: Path, manifest_path: Path):
    root = root.resolve()
    findings = []
    for required in ("index.md", "overview.md", "coverage.md", "log.md"):
        if not (root / required).is_file():
            findings.append(finding("R_REQUIRED_FILE", required, "required bundle file is missing"))

    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return findings + [finding("R_MANIFEST", str(manifest_path), f"cannot read inventory manifest: {error}")]

    coverage = _read(root / "coverage.md")
    rows, duplicates = _coverage_rows(coverage)
    for candidate in sorted(duplicates):
        findings.append(finding("R_COVERAGE_DUPLICATE", "coverage.md", f"candidate {candidate} appears more than once"))
    expected = {str(item.get("id", "")) for item in manifest.get("candidates", []) if item.get("id")}
    for candidate in sorted(expected - set(rows)):
        findings.append(finding("R_COVERAGE_MISSING", "coverage.md", f"candidate {candidate} is not reconciled"))
    for candidate, row in sorted(rows.items()):
        if candidate not in expected:
            findings.append(finding("R_COVERAGE_UNKNOWN", "coverage.md", f"candidate {candidate} is absent from manifest", "warning"))
        if row["status"] not in STATUSES:
            findings.append(finding("R_COVERAGE_STATUS", "coverage.md", f"candidate {candidate} has invalid status {row['status']}"))
        if row["status"] == "documented" and not re.search(r"\[[^]]+\]\([^)]+\)", row["doc"]):
            findings.append(finding("R_COVERAGE_DOC", "coverage.md", f"documented candidate {candidate} needs a document link"))
        if row["status"] in {"irrelevant", "generated", "uncertain"} and len(row["reason"].strip()) < 8:
            findings.append(finding("R_COVERAGE_REASON", "coverage.md", f"candidate {candidate} needs a concrete reason"))
        if row["status"] == "uncertain":
            findings.append(finding("R_COVERAGE_UNCERTAIN", "coverage.md", f"candidate {candidate} remains uncertain", "warning"))

    markdown = list(root.rglob("*.md"))
    endpoint_docs = []
    data_docs = []
    for path in markdown:
        text = _read(path)
        lower = text.lower()
        relative = path.relative_to(root).as_posix()
        if re.search(r"(?m)^type:\s*(?:api endpoint|endpoint)\s*$", text, re.I):
            endpoint_docs.append((relative, text, lower))
        if re.search(r"(?m)^type:\s*(?:data asset|database table|table|collection|redis key)\s*$", text, re.I):
            data_docs.append((relative, text, lower))

    for relative, text, lower in endpoint_docs:
        checks = {
            "R_API_REQUEST": ("request" in lower and (FENCE_RE.search(text) or "no request body" in lower), "request body/schema and non-empty example are missing"),
            "R_API_RESPONSE": ("response" in lower and FENCE_RE.search(text), "response body and non-empty example are missing"),
            "R_API_ERRORS": (bool(re.search(r"\b[45]\d\d\b", text)), "material error status/body is missing"),
            "R_API_FLOW": ("flow" in lower and bool(re.search(r"\[[^]]+\]\([^)]+\)", text)), "runtime flow link is missing"),
            "R_API_CITATIONS": (len(CITATION_RE.findall(text)) >= 2, "route and DTO/handler citations are missing"),
        }
        for rule, (ok, message) in checks.items():
            if not ok:
                findings.append(finding(rule, relative, message))

    for relative, text, lower in data_docs:
        checks = {
            "R_DATA_SCHEMA": (bool(re.search(r"\|[^\n]*(?:field|column)[^\n]*\|[^\n]*type", text, re.I)), "field/column schema table is missing"),
            "R_DATA_ACCESS": (bool(re.search(r"\b(read|select|lookup|scan)\b", lower)) and bool(re.search(r"\b(write|insert|update|delete|set)\b", lower)), "actual read and write access paths are missing"),
            "R_DATA_IMPACT": ("impact" in lower and len(CITATION_RE.findall(text)) >= 2, "field-level impact with citations is missing"),
            "R_DATA_INDEX_TTL": ("index" in lower and bool(re.search(r"\b(ttl|retention|expiry|expire)\b", lower)), "indexes and TTL/retention are missing"),
            "R_DATA_CONSISTENCY": (bool(re.search(r"\b(transaction|atomic|consisten|isolation|locking)\b", lower)), "transaction/consistency semantics are missing"),
        }
        for rule, (ok, message) in checks.items():
            if not ok:
                findings.append(finding(rule, relative, message))

    kinds = {str(item.get("kind")) for item in manifest.get("candidates", [])}
    all_text = "\n".join(_read(path).lower() for path in markdown)
    if "worker" in kinds and not re.search(r"worker|schedule|reconcil", all_text):
        findings.append(finding("R_WORKERS", ".", "worker candidates exist but worker runtime documentation is missing"))
    if "messaging" in kinds and not re.search(r"producer|consumer|topic|queue|message", all_text):
        findings.append(finding("R_MESSAGING", ".", "messaging candidates exist but messaging documentation is missing"))

    openapi = next((path for name in ("api-openapi.yaml", "api-openapi.yml") if (path := root / name).is_file()), None)
    if endpoint_docs:
        if openapi is None:
            findings.append(finding("R_OPENAPI", "api-openapi.yaml", "API docs exist but OpenAPI artifact is missing"))
        else:
            spec = _read(openapi)
            if "paths:" not in spec or not re.search(r"(?m)^\s+(get|post|put|patch|delete):\s*$", spec):
                findings.append(finding("R_OPENAPI_OPERATION", openapi.name, "OpenAPI has no concrete operation"))
            if "responses:" not in spec or not re.search(r"(?m)^\s+requestBody:\s*$", spec):
                findings.append(finding("R_OPENAPI_BODIES", openapi.name, "OpenAPI operation lacks requestBody or responses"))

    findings.sort(key=lambda item: (item["path"], item["rule"], item["message"]))
    return findings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--format", choices=("json", "text"), default="text")
    args = parser.parse_args(argv)
    results = audit(args.bundle, args.manifest)
    if args.format == "json":
        print(json.dumps(results, indent=2, sort_keys=True))
    elif not results:
        print("REPOSITORY BUNDLE AUDIT CLEAN")
    else:
        for item in results:
            print(f'{item["severity"].upper()} {item["rule"]} {item["path"]}: {item["message"]}')
    return 1 if any(item["severity"] == "error" for item in results) else 0


if __name__ == "__main__":
    sys.exit(main())
