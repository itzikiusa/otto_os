#!/usr/bin/env python3
"""Audit repository documentation for coverage and minimum source depth."""

import argparse
import json
import re
import sys
from collections import Counter
from pathlib import Path
from urllib.parse import unquote, urlsplit

STATUSES = {"documented", "irrelevant", "generated", "uncertain"}
ROW_RE = re.compile(
    r"^\|\s*`?([^|`]+)`?\s*\|\s*([^|]+)\|\s*([^|]+)\|\s*([^|]+)\|\s*([^|]+)\|\s*([^|]+)\|",
    re.M,
)
LINK_RE = re.compile(r"\[[^]]+\]\(([^)]+)\)")
FENCE_RE = re.compile(r"```(?:json|yaml|yml|sql|text)?\s*\n([\s\S]*?)```", re.I)
CITATION_RE = re.compile(r"`?[A-Za-z0-9_.\-/]+:\d+`?")
HTTP_OPERATION_RE = re.compile(
    r"(?im)(?:^#{1,6}\s*|`|^\|\s*)(GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD)\s+(/[A-Za-z0-9_./{}:\-]+)"
)


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
        if candidate in rows:
            duplicates.add(candidate)
        rows[candidate] = {
            "kind": match.group(2).strip().lower().strip("`"),
            "evidence": match.group(3).strip().strip("`"),
            "status": match.group(4).strip().lower().strip("`"),
            "doc": match.group(5).strip(),
            "reason": match.group(6).strip(),
        }
    return rows, duplicates


def _resolve_doc(root: Path, cell: str):
    match = LINK_RE.search(cell)
    if not match:
        return None, "missing-link"
    raw = match.group(1).strip().split()[0].strip("<>")
    parsed = urlsplit(raw)
    if parsed.scheme or parsed.netloc or raw.startswith("/"):
        return None, "outside"
    relative = unquote(parsed.path)
    if not relative:
        return None, "missing-link"
    try:
        target = (root / relative).resolve(strict=False)
        target.relative_to(root)
    except (OSError, ValueError):
        return None, "outside"
    if target.suffix.lower() != ".md":
        return None, "not-markdown"
    if not target.is_file():
        return target, "missing"
    return target, None


def _sections(text: str):
    matches = list(re.finditer(r"(?m)^#{1,6}\s+(.+?)\s*$", text))
    sections = []
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        sections.append((match.group(1).strip().lower(), text[match.end():end].strip()))
    return sections


def _section(sections, *needles):
    bodies = [body for heading, body in sections if any(needle in heading for needle in needles)]
    return "\n".join(bodies)


def _has_nonempty_fence(text: str):
    return any(match.group(1).strip() for match in FENCE_RE.finditer(text))


def _api_findings(relative: str, text: str):
    results = []
    lower = text.lower()
    sections = _sections(text)
    request = _section(sections, "request body", "request schema")
    response = _section(sections, "success response", "response body", "responses")
    errors = _section(sections, "error")
    auth = _section(sections, "authentication", "authorization", "auth")
    flow = _section(sections, "flow")
    no_body = "no request body" in request.lower()
    checks = {
        "R_API_REQUEST": (bool(request) and (no_body or _has_nonempty_fence(request)), "request body/schema and section-local non-empty example are missing"),
        "R_API_RESPONSE": (bool(response) and _has_nonempty_fence(response), "response body and section-local non-empty example are missing"),
        "R_API_ERRORS": (bool(re.search(r"\b[45]\d\d\b", errors)) and ("{" in errors or _has_nonempty_fence(errors)), "material error status and response body are missing"),
        "R_API_AUTH": (bool(auth.strip()), "authentication/authorization behavior is missing"),
        "R_API_FLOW": (bool(flow) and bool(LINK_RE.search(flow)), "runtime flow link is missing"),
        "R_API_CITATIONS": (len(CITATION_RE.findall(text)) >= 3, "route, DTO, and handler/test citations are missing"),
        "R_API_OPERATION": (bool(HTTP_OPERATION_RE.search(text)), "HTTP method and exact path are missing"),
    }
    for rule, (ok, message) in checks.items():
        if not ok:
            results.append(finding(rule, relative, message))
    if "additionalproperties: true" in lower:
        results.append(finding("R_API_STUB", relative, "unbounded additionalProperties is a stub, not a body contract"))
    return results


def _data_findings(relative: str, text: str):
    lower = text.lower()
    checks = {
        "R_DATA_SCHEMA": (bool(re.search(r"\|[^\n]*(?:field|column)[^\n]*\|[^\n]*type", text, re.I)), "field/column schema table is missing"),
        "R_DATA_ACCESS": (bool(re.search(r"\b(read|select|lookup|scan)\b", lower)) and bool(re.search(r"\b(write|insert|update|delete|set)\b", lower)), "actual read and write access paths are missing"),
        "R_DATA_IMPACT": ("impact" in lower and len(CITATION_RE.findall(text)) >= 2, "field-level impact with citations is missing"),
        "R_DATA_INDEX_TTL": ("index" in lower and bool(re.search(r"\b(ttl|retention|expiry|expire)\b", lower)), "indexes and TTL/retention are missing"),
        "R_DATA_CONSISTENCY": (bool(re.search(r"\b(transaction|atomic|consisten|isolation|locking|merge)\b", lower)), "transaction/consistency semantics are missing"),
    }
    return [finding(rule, relative, message) for rule, (ok, message) in checks.items() if not ok]


def _runtime_findings(kind: str, relative: str, text: str):
    lower = text.lower()
    citations = len(CITATION_RE.findall(text))
    if kind == "messaging":
        checks = {
            "R_MESSAGING_DIRECTION": (bool(re.search(r"\b(producer|publish|consumer|subscribe)\b", lower)), "producer/consumer direction is missing"),
            "R_MESSAGING_PAYLOAD": (_has_nonempty_fence(text), "complete payload example is missing"),
            "R_MESSAGING_DELIVERY": (bool(re.search(r"\b(delivery|at-least|at-most|exactly|retry|backoff|dead.?letter|poison)\b", lower)), "delivery and failure semantics are missing"),
            "R_MESSAGING_CITATIONS": (citations >= 2, "registration and handler citations are missing"),
        }
    elif kind == "worker":
        checks = {
            "R_WORKER_TRIGGER": (bool(re.search(r"\b(schedule|cron|hourly|daily|interval|trigger|registration)\b", lower)), "worker registration/schedule is missing"),
            "R_WORKER_FAILURE": (bool(re.search(r"\b(failure|retry|recovery|idempoten|checkpoint)\b", lower)), "worker failure/retry behavior is missing"),
            "R_WORKER_CITATIONS": (citations >= 2, "worker registration and implementation citations are missing"),
        }
    else:
        checks = {
            "R_RUNTIME_LIFECYCLE": ("startup" in lower and "shutdown" in lower, "startup and shutdown lifecycle are missing"),
            "R_RUNTIME_FAILURE": (bool(re.search(r"\b(error|failure|retry|terminate|drain)\b", lower)), "runtime failure behavior is missing"),
            "R_RUNTIME_CITATIONS": (citations >= 2, "lifecycle registration and implementation citations are missing"),
        }
    return [finding(rule, relative, message) for rule, (ok, message) in checks.items() if not ok]


def _openapi_operations(text: str):
    """Parse a conservative, indentation-based OpenAPI YAML subset."""
    findings = []
    if "\t" in text:
        findings.append(finding("R_OPENAPI_YAML", "api-openapi.yaml", "tabs are not valid YAML indentation"))
    if not re.search(r"(?m)^openapi:\s*3\.\d+(?:\.\d+)?\s*$", text):
        findings.append(finding("R_OPENAPI_VERSION", "api-openapi.yaml", "OpenAPI 3 version is missing"))
    lines = text.splitlines()
    paths_index = next((i for i, line in enumerate(lines) if re.match(r"^\s*paths:\s*$", line)), None)
    if paths_index is None:
        findings.append(finding("R_OPENAPI_PATHS", "api-openapi.yaml", "paths mapping is missing"))
        return {}, findings
    paths_indent = len(lines[paths_index]) - len(lines[paths_index].lstrip())
    operations = {}
    index = paths_index + 1
    current_path = None
    current_path_indent = None
    while index < len(lines):
        line = lines[index]
        stripped = line.strip()
        indent = len(line) - len(line.lstrip())
        if stripped and not stripped.startswith("#") and indent <= paths_indent:
            break
        path_match = re.match(r"^\s*(/[^:]+):\s*$", line)
        if path_match:
            current_path = path_match.group(1)
            current_path_indent = indent
            index += 1
            continue
        method_match = re.match(r"^\s*(get|post|put|patch|delete|options|head):\s*$", line, re.I)
        if method_match and current_path and indent > current_path_indent:
            method = method_match.group(1).upper()
            block_start = index + 1
            end = block_start
            while end < len(lines):
                probe = lines[end]
                probe_stripped = probe.strip()
                probe_indent = len(probe) - len(probe.lstrip())
                if probe_stripped and not probe_stripped.startswith("#") and probe_indent <= indent:
                    break
                end += 1
            block = "\n".join(lines[block_start:end])
            key = (method, current_path)
            operations[key] = block
            if not re.search(r"(?m)^\s*operationId:\s*[^\s#]+", block):
                findings.append(finding("R_OPENAPI_OPERATION_ID", "api-openapi.yaml", f"{method} {current_path} lacks operationId"))
            if not re.search(r"(?m)^\s*responses:\s*$", block) or not re.search(r"(?m)^\s*['\"]?[1-5]\d\d['\"]?:\s*", block):
                findings.append(finding("R_OPENAPI_RESPONSES", "api-openapi.yaml", f"{method} {current_path} lacks concrete responses"))
            needs_body = method in {"POST", "PUT", "PATCH"}
            if needs_body and not re.search(r"(?m)^\s*requestBody:\s*$", block):
                findings.append(finding("R_OPENAPI_BODIES", "api-openapi.yaml", f"{method} {current_path} lacks requestBody"))
            minimum = 2 if needs_body else 1
            if len(re.findall(r"(?m)^\s*schema:\s*$", block)) < minimum:
                findings.append(finding("R_OPENAPI_SCHEMAS", "api-openapi.yaml", f"{method} {current_path} lacks request/response schemas"))
            if len(re.findall(r"(?m)^\s*examples?:\s*", block)) < minimum:
                findings.append(finding("R_OPENAPI_EXAMPLES", "api-openapi.yaml", f"{method} {current_path} lacks request/response examples"))
            index = end
            continue
        index += 1
    if not operations:
        findings.append(finding("R_OPENAPI_OPERATION", "api-openapi.yaml", "OpenAPI has no concrete operation"))
    return operations, findings


def _manifest_findings(manifest):
    results = []
    if not isinstance(manifest, dict) or not isinstance(manifest.get("candidates"), list):
        return [finding("R_MANIFEST", "manifest.json", "manifest candidates must be a list")]
    if int(manifest.get("version", 0) or 0) < 2:
        results.append(finding("R_MANIFEST_VERSION", "manifest.json", "manifest must include scan-accounting version 2"))
    scanned = manifest.get("scanned_files")
    exclusions = manifest.get("exclusions")
    counts = manifest.get("counts")
    if not isinstance(scanned, list) or not isinstance(exclusions, list) or not isinstance(counts, dict):
        results.append(finding("R_MANIFEST_SCAN_RECORD", "manifest.json", "scanned_files, exclusions, and counts are required"))
    ids = [str(item.get("id", "")) for item in manifest["candidates"] if isinstance(item, dict)]
    for candidate, count in Counter(ids).items():
        if candidate and count > 1:
            results.append(finding("R_MANIFEST_DUPLICATE", "manifest.json", f"candidate {candidate} appears {count} times"))
    if isinstance(scanned, list) and not manifest["candidates"] and scanned and manifest.get("mode") in {"full", "full-fallback"}:
        results.append(finding("R_INVENTORY_EMPTY", "manifest.json", "non-empty source scan produced zero candidates; manually inspect and record confirmed surfaces"))
    if isinstance(counts, dict):
        expected = {
            "files_scanned": len(scanned or []),
            "files_excluded": len(exclusions or []),
            "candidates": len(manifest["candidates"]),
            "by_kind": dict(sorted(Counter(
                str(item.get("kind", "")) for item in manifest["candidates"]
                if isinstance(item, dict) and item.get("kind")
            ).items())),
        }
        for key, value in expected.items():
            if counts.get(key) != value:
                results.append(finding("R_MANIFEST_COUNTS", "manifest.json", f"{key} count does not match manifest contents"))
    return results


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
    findings.extend(_manifest_findings(manifest))
    candidates = [item for item in manifest.get("candidates", []) if isinstance(item, dict)]
    candidate_by_id = {str(item.get("id")): item for item in candidates if item.get("id")}

    coverage = _read(root / "coverage.md")
    rows, duplicates = _coverage_rows(coverage)
    for candidate in sorted(duplicates):
        findings.append(finding("R_COVERAGE_DUPLICATE", "coverage.md", f"candidate {candidate} appears more than once"))
    expected = set(candidate_by_id)
    for candidate in sorted(expected - set(rows)):
        findings.append(finding("R_COVERAGE_MISSING", "coverage.md", f"candidate {candidate} is not reconciled"))

    targets = {}
    for candidate, row in sorted(rows.items()):
        source = candidate_by_id.get(candidate)
        if source is None:
            findings.append(finding("R_COVERAGE_UNKNOWN", "coverage.md", f"candidate {candidate} is absent from manifest"))
            continue
        if row["kind"] != str(source.get("kind", "")).lower():
            findings.append(finding("R_COVERAGE_KIND", "coverage.md", f"candidate {candidate} kind does not match manifest"))
        if row["evidence"] != str(source.get("evidence", "")):
            findings.append(finding("R_COVERAGE_EVIDENCE", "coverage.md", f"candidate {candidate} evidence does not match manifest"))
        if row["status"] not in STATUSES:
            findings.append(finding("R_COVERAGE_STATUS", "coverage.md", f"candidate {candidate} has invalid status {row['status']}"))
            continue
        if len(row["reason"].strip()) < 8:
            findings.append(finding("R_COVERAGE_REASON", "coverage.md", f"candidate {candidate} needs a concrete reason"))
        if row["status"] == "uncertain":
            findings.append(finding("R_COVERAGE_UNCERTAIN", "coverage.md", f"candidate {candidate} remains uncertain"))
        if row["status"] == "irrelevant" and manifest.get("mode") in {"full", "full-fallback"} and re.search(r"out.?of.?scope|focus", row["reason"], re.I):
            findings.append(finding("R_COVERAGE_SCOPE", "coverage.md", f"full-scan candidate {candidate} cannot be dismissed as out of scope"))
        if row["status"] not in {"documented", "generated"}:
            continue
        target, error = _resolve_doc(root, row["doc"])
        if row["status"] == "generated" and error:
            findings.append(finding("R_COVERAGE_GENERATED_DOC", "coverage.md", f"generated candidate {candidate} needs a provenance document"))
        elif error == "missing-link":
            findings.append(finding("R_COVERAGE_DOC", "coverage.md", f"documented candidate {candidate} needs a document link"))
        elif error in {"outside", "not-markdown"}:
            findings.append(finding("R_COVERAGE_DOC_PATH", "coverage.md", f"candidate {candidate} document must be a Markdown file inside the bundle"))
        elif error == "missing":
            findings.append(finding("R_COVERAGE_DOC_MISSING", "coverage.md", f"candidate {candidate} links a missing document"))
        elif target:
            targets[(str(source.get("kind", "")), target)] = target.relative_to(root).as_posix()

    documented_api_ops = set()
    for (kind, target), relative in sorted(targets.items(), key=lambda item: (item[0][0], item[1])):
        text = _read(target)
        if kind == "api":
            findings.extend(_api_findings(relative, text))
            documented_api_ops.update((method.upper(), path) for method, path in HTTP_OPERATION_RE.findall(text))
        elif kind == "data":
            findings.extend(_data_findings(relative, text))
        elif kind in {"messaging", "worker", "runtime"}:
            findings.extend(_runtime_findings(kind, relative, text))

    if any(kind == "api" for kind, _ in targets):
        openapi = next((path for name in ("api-openapi.yaml", "api-openapi.yml") if (path := root / name).is_file()), None)
        if openapi is None:
            findings.append(finding("R_OPENAPI", "api-openapi.yaml", "API docs exist but OpenAPI artifact is missing"))
        else:
            operations, openapi_findings = _openapi_operations(_read(openapi))
            findings.extend(openapi_findings)
            spec_ops = set(operations)
            for method, path in sorted(documented_api_ops - spec_ops):
                findings.append(finding("R_OPENAPI_MISMATCH", openapi.name, f"documented {method} {path} is absent from OpenAPI"))
            for method, path in sorted(spec_ops - documented_api_ops):
                findings.append(finding("R_OPENAPI_MISMATCH", openapi.name, f"OpenAPI {method} {path} is absent from API docs"))

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
