#!/usr/bin/env python3
"""Audit an OKF bundle for deterministic conformance and content-depth gaps."""

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Dict, List, Optional

import validate_okf


HEADING_RE = re.compile(r"^#\s+(.+?)\s*$", re.MULTILINE)
FENCE_RE = re.compile(
    r"(?:^|\n)(?P<fence>```|~~~)(?P<language>[^\n]*)\n"
    r"(?P<body>.*?)(?:\n(?P=fence)[ \t]*(?=\n|$))",
    re.DOTALL,
)
TABLE_HEADER_RE = re.compile(
    r"\|[^\n]*\bfield\b[^\n]*\btype\b[^\n]*\bdescription\b[^\n]*\|",
    re.IGNORECASE,
)
LINK_RE = re.compile(r"\[[^\]]+\]\([^)]+\)")
UNKNOWN_RE = re.compile(r"\bunknown\b|\bn\s*/?\s*a\b|\bnot applicable\b", re.I)
DEPTH_KEYWORDS = {
    "access",
    "atomic",
    "auth",
    "authentication",
    "authorization",
    "body",
    "commit",
    "consistency",
    "delete",
    "description",
    "effect",
    "effects",
    "error",
    "errors",
    "example",
    "examples",
    "expire",
    "field",
    "fields",
    "flow",
    "header",
    "impact",
    "index",
    "indexes",
    "insert",
    "isolation",
    "join",
    "joins",
    "lookup",
    "parameter",
    "parameters",
    "path",
    "query",
    "read",
    "reads",
    "relationship",
    "relationships",
    "request",
    "response",
    "retention",
    "runtime",
    "scan",
    "side",
    "success",
    "transaction",
    "ttl",
    "type",
    "update",
    "validation",
    "write",
    "writes",
}


def _normalize_heading(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", " ", value.lower()).strip()


def _sections(body: str) -> Dict[str, str]:
    matches = list(HEADING_RE.finditer(body))
    sections: Dict[str, str] = {}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(body)
        sections[_normalize_heading(match.group(1))] = body[match.end() : end].strip()
    return sections


def _section(sections: Dict[str, str], *aliases: str) -> str:
    normalized = {_normalize_heading(alias) for alias in aliases}
    for heading, content in sections.items():
        if heading in normalized:
            return content
    return ""


def _prose(content: str) -> str:
    content = FENCE_RE.sub(" ", content)
    content = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", content)
    content = re.sub(r"[`|#*_<>:-]", " ", content)
    return re.sub(r"\s+", " ", content).strip()


def _has_substantive_content(content: str) -> bool:
    words = [word.lower() for word in re.findall(r"[A-Za-z0-9]+", _prose(content))]
    return len(words) >= 4 and any(word not in DEPTH_KEYWORDS for word in words)


def _has_evidence_backed_unknown(content: str) -> bool:
    return bool(
        UNKNOWN_RE.search(content)
        and LINK_RE.search(content)
        and _has_substantive_content(content)
    )


def _has_depth(content: str) -> bool:
    if UNKNOWN_RE.search(content):
        return _has_evidence_backed_unknown(content)
    return _has_substantive_content(content)


def _has_explicit_absence(content: str, nouns: str) -> bool:
    if not _has_substantive_content(content):
        return False
    return bool(
        re.search(r"\b(?:no|without)\b.{{0,32}}\b(?:{})\b".format(nouns), content, re.I)
        or re.search(r"\b(?:{})\b.{{0,20}}\bnone\b".format(nouns), content, re.I)
    )


def _has_schema_table(content: str) -> bool:
    lines = content.splitlines()
    for index, line in enumerate(lines):
        if not TABLE_HEADER_RE.search(line):
            continue
        if index + 2 >= len(lines):
            return False
        separator = lines[index + 1].strip().strip("|")
        if not separator or not all(
            re.fullmatch(r"\s*:?-{3,}:?\s*", cell)
            for cell in separator.split("|")
        ):
            return False
        for row in lines[index + 2 :]:
            if row.strip().startswith("|"):
                cells = [cell.strip() for cell in row.strip().strip("|").split("|")]
                if len(cells) >= 3 and all(cells[:3]):
                    return True
            elif row.strip():
                break
        return False
    return False


def _is_structured_example(language: str, body: str) -> bool:
    language = language.strip().lower().split(maxsplit=1)[0] if language.strip() else ""
    body = body.strip()
    if not body:
        return False
    if language == "json" or body.startswith(("{", "[")):
        try:
            parsed = json.loads(body)
        except (TypeError, ValueError):
            return False
        return isinstance(parsed, (dict, list)) and bool(parsed)
    if language in {"sql", "postgresql", "mysql"}:
        return bool(re.search(r"\b(?:select|insert|update|delete|create|with)\b", body, re.I))
    if language in {"yaml", "yml"}:
        return bool(re.search(r"^\s*[A-Za-z_][\w.-]*\s*:\s*\S+", body, re.MULTILINE))
    if language in {"xml", "html"}:
        return bool(re.search(r"<([A-Za-z_][\w:.-]*)\b[^>]*>.*</\1>", body, re.DOTALL))
    if language in {"http", "https"} or re.search(
        r"^(?:GET|POST|PUT|PATCH|DELETE|HTTP/)\s+", body, re.MULTILINE
    ):
        return True
    if language in {"graphql", "gql"}:
        return bool(re.search(r"\b(?:query|mutation|subscription)\b", body))
    if language in {"bash", "console", "sh", "shell", "zsh"}:
        return bool(re.search(r"^(?:\$\s*)?(?:curl|http|wget|redis-cli|psql|mysql)\b", body, re.MULTILINE))
    return bool(
        re.search(r"^\s*[A-Za-z_][\w.-]*\s*[=:]\s*\S+", body, re.MULTILINE)
    )


def _has_structured_example(content: str) -> bool:
    return any(
        _is_structured_example(match.group("language"), match.group("body"))
        for match in FENCE_RE.finditer(content)
    )


def _has_body_contract(content: str, allow_absence: bool = False) -> bool:
    if _has_evidence_backed_unknown(content):
        return True
    if _has_schema_table(content) or _has_structured_example(content):
        return True
    return allow_absence and _has_explicit_absence(content, "body|payload|response")


def _finding(rule: str, path: str, message: str, severity: str = "warning") -> Dict[str, str]:
    return {"rule": rule, "path": path, "message": message, "severity": severity}


def _require(
    findings: List[Dict[str, str]],
    condition: bool,
    rule: str,
    path: str,
    message: str,
) -> None:
    if not condition:
        findings.append(_finding(rule, path, message))


def _audit_api(path: str, sections: Dict[str, str], findings: List[Dict[str, str]]) -> None:
    authentication = _section(sections, "Authentication", "Authorization")
    parameters = _section(sections, "Parameters", "Path and Query Parameters")
    request = _section(sections, "Request", "Request Body")
    success = _section(sections, "Success Response", "Response", "Success")
    errors = _section(sections, "Error Responses", "Errors")
    validation = _section(sections, "Validation", "Validation and Side Effects")
    side_effects = _section(sections, "Side Effects", "Validation and Side Effects")
    flow = _section(sections, "Flow", "Runtime Flow")
    citations = _section(sections, "Citations", "Sources")

    _require(findings, _has_depth(authentication), "Q_API_AUTH", path, "API endpoint needs authentication and authorization details")
    _require(findings, _has_depth(parameters), "Q_API_PARAMETERS", path, "API endpoint needs path, query, and header parameter details or an explicit none")
    _require(findings, _has_body_contract(request, allow_absence=True), "Q_API_REQUEST", path, "API endpoint needs a request schema/body and realistic example or an explicit no-body statement")
    _require(findings, _has_body_contract(success, allow_absence=True), "Q_API_SUCCESS", path, "API endpoint needs a success response body and realistic example")
    _require(
        findings,
        _has_evidence_backed_unknown(errors)
        or _has_explicit_absence(errors, "error|errors|response|responses")
        or (
            _has_body_contract(errors)
            and bool(re.search(r"\b[45]\d\d\b", errors))
        ),
        "Q_API_ERRORS",
        path,
        "API endpoint needs material error status and response body examples",
    )
    _require(findings, _has_depth(validation), "Q_API_VALIDATION", path, "API endpoint needs validation rules")
    _require(findings, _has_depth(side_effects), "Q_API_SIDE_EFFECTS", path, "API endpoint needs side effects or an explicit none")
    _require(findings, _has_depth(flow), "Q_API_FLOW", path, "API endpoint needs a link or description of its runtime flow")
    _require(findings, bool(citations) and bool(LINK_RE.search(citations)), "Q_CITATIONS", path, "Concept needs source citations")


def _audit_data(path: str, sections: Dict[str, str], findings: List[Dict[str, str]]) -> None:
    overview = _section(sections, "Overview")
    fields = _section(sections, "Schema", "Fields")
    access = _section(sections, "Access Paths", "Reads and Writes", "Access Patterns")
    index_ttl = _section(sections, "Indexes and TTL", "Indexes TTL and Retention")
    transactions = _section(sections, "Transactions and Consistency", "Consistency")
    relationships = _section(sections, "Relationships", "Joins")
    impact = _section(sections, "Impact", "Field-Level Impact")
    examples = _section(sections, "Examples", "Common Query Patterns")
    citations = _section(sections, "Citations", "Sources")

    _require(
        findings,
        _has_evidence_backed_unknown(overview)
        or (
            _has_depth(overview)
            and bool(re.search(r"\b(one|each)\s+(row|document|record|value|event)\b", overview, re.I))
        ),
        "Q_DATA_GRAIN",
        path,
        "Data asset needs an explicit grain or key-value purpose",
    )
    _require(
        findings,
        _has_evidence_backed_unknown(fields) or _has_schema_table(fields),
        "Q_DATA_FIELDS",
        path,
        "Data asset needs full known fields with types and descriptions",
    )
    _require(
        findings,
        _has_evidence_backed_unknown(access)
        or (
            _has_depth(access)
            and bool(re.search(r"\bread|select|lookup|scan\b", access, re.I))
            and bool(re.search(r"\bwrite|insert|update|delete|publish\b", access, re.I))
        ),
        "Q_DATA_ACCESS",
        path,
        "Data asset needs actual read and write access paths or explicit none",
    )
    _require(
        findings,
        _has_evidence_backed_unknown(index_ttl)
        or (
            _has_depth(index_ttl)
            and "index" in index_ttl.lower()
            and bool(re.search(r"\bttl|retention|expire|archive\b", index_ttl, re.I))
        ),
        "Q_DATA_INDEX_TTL",
        path,
        "Data asset needs indexes and TTL or retention behavior",
    )
    _require(
        findings,
        _has_evidence_backed_unknown(transactions)
        or (
            _has_depth(transactions)
            and bool(re.search(r"\btransaction|atomic|consisten|isolation|commit\b", transactions, re.I))
        ),
        "Q_DATA_TRANSACTIONS",
        path,
        "Data asset needs transaction and consistency behavior",
    )
    _require(findings, _has_depth(relationships), "Q_DATA_RELATIONSHIPS", path, "Data asset needs joins or relationships or an explicit none")
    _require(
        findings,
        _has_evidence_backed_unknown(impact)
        or (
            _has_depth(impact)
            and "`" in impact
            and bool(re.search(r"\bread|write|update|insert|delete\b", impact, re.I))
        ),
        "Q_DATA_IMPACT",
        path,
        "Data asset needs field-level read/write impact paths",
    )
    _require(
        findings,
        _has_evidence_backed_unknown(examples) or _has_structured_example(examples),
        "Q_DATA_EXAMPLES",
        path,
        "Data asset needs a realistic query or payload example",
    )
    _require(findings, bool(citations) and bool(LINK_RE.search(citations)), "Q_CITATIONS", path, "Concept needs source citations")


def audit_bundle(root: Path) -> List[Dict[str, str]]:
    root = Path(root)
    report = validate_okf.validate_bundle(root)
    findings: List[Dict[str, str]] = [
        _finding(item["rule"], item["path"], item["message"], "error")
        for item in report["errors"]
    ]
    findings.extend(
        _finding(item["rule"], item["path"], item["message"], "warning")
        for item in report["warnings"]
    )

    for path in validate_okf._markdown_files(root):
        relative = path.relative_to(root).as_posix()
        if path.name.lower() in {"index.md", "log.md"}:
            continue
        text, read_error = validate_okf._read(path)
        present, parse_error, frontmatter, body = validate_okf.parse_frontmatter(text)
        if read_error or not present or parse_error or not frontmatter.get("type"):
            continue
        concept_type = str(frontmatter["type"]).strip().lower()
        sections = _sections(body)
        if concept_type in {"api endpoint", "endpoint", "http endpoint", "rpc endpoint"}:
            _audit_api(relative, sections, findings)
        if concept_type in {
            "bigquery table",
            "collection",
            "data asset",
            "database table",
            "dataset",
            "redis key",
            "table",
        }:
            _audit_data(relative, sections, findings)

    findings.sort(key=lambda item: (item["path"], item["rule"], item["message"]))
    return findings


def _render_text(findings: List[Dict[str, str]]) -> str:
    if not findings:
        return "OKF AUDIT CLEAN"
    return "\n".join(
        "{} {} {}: {}".format(
            item["severity"].upper(), item["rule"], item["path"], item["message"]
        )
        for item in findings
    )


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--format", choices=("json", "text"), default="text")
    args = parser.parse_args(argv)
    try:
        findings = audit_bundle(args.root)
    except ValueError as error:
        parser.error(str(error))
    if args.format == "json":
        print(json.dumps(findings, indent=2, sort_keys=True))
    else:
        print(_render_text(findings))
    return 1 if any(item["severity"] == "error" for item in findings) else 0


if __name__ == "__main__":
    sys.exit(main())
