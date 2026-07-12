#!/usr/bin/env python3
"""Deterministic, read-only OKF v0.1 conformance validator."""

import argparse
import json
import posixpath
import re
import sys
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Tuple
from urllib.parse import unquote


ISO_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*$", re.MULTILINE)
LINK_RE = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")
KEY_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$")


def _finding(rule: str, path: str, message: str) -> Dict[str, str]:
    return {"rule": rule, "path": path, "message": message}


def _scalar(raw: str) -> Optional[str]:
    value = raw.strip()
    if not value or value in {"null", "Null", "NULL", "~", "[]", "{}"}:
        return None
    if value.startswith('"'):
        try:
            parsed = json.loads(value)
        except (TypeError, ValueError):
            return value
        return parsed if isinstance(parsed, str) else value
    if value.startswith("'") and value.endswith("'") and len(value) >= 2:
        return value[1:-1].replace("''", "'")
    if " #" in value:
        value = value.split(" #", 1)[0].rstrip()
    return value or None


def parse_frontmatter(text: str) -> Tuple[bool, bool, Dict[str, Optional[str]], str]:
    """Return (present, parse_error, top-level scalar map, body).

    The parser intentionally accepts only a conservative YAML mapping surface.
    Nested/list values are preserved as non-scalar metadata but never interpreted.
    """

    text = text.lstrip("\ufeff")
    lines = text.splitlines(keepends=True)
    if not lines or lines[0].rstrip("\r\n") != "---":
        return False, False, {}, text

    closing = next(
        (index for index, line in enumerate(lines[1:], 1) if line.rstrip("\r\n") == "---"),
        None,
    )
    if closing is None:
        return True, True, {}, ""

    values: Dict[str, Optional[str]] = {}
    last_key: Optional[str] = None
    parse_error = False
    for raw_line in lines[1:closing]:
        line = raw_line.rstrip("\r\n")
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if "\t" in line:
            parse_error = True
            continue
        if line[0].isspace() or line.lstrip().startswith("-"):
            if last_key is None:
                parse_error = True
            continue
        match = KEY_RE.match(line)
        if not match:
            parse_error = True
            continue
        key, raw_value = match.groups()
        if key in values:
            parse_error = True
            continue
        values[key] = _scalar(raw_value)
        last_key = key

    return True, parse_error, values, "".join(lines[closing + 1 :])


def _markdown_files(root: Path) -> List[Path]:
    return sorted(
        (path for path in root.rglob("*.md") if path.is_file()),
        key=lambda path: path.relative_to(root).as_posix(),
    )


def _read(path: Path) -> Tuple[str, bool]:
    try:
        return path.read_text(encoding="utf-8"), False
    except (OSError, UnicodeError):
        return "", True


def _strip_fenced_code(text: str) -> str:
    output: List[str] = []
    fence: Optional[str] = None
    for line in text.splitlines():
        stripped = line.lstrip()
        marker = "```" if stripped.startswith("```") else "~~~" if stripped.startswith("~~~") else None
        if marker:
            fence = None if fence == marker else marker if fence is None else fence
            continue
        if fence is None:
            output.append(line)
    return "\n".join(output)


def _internal_targets(source: str, body: str) -> Iterable[str]:
    for match in LINK_RE.finditer(_strip_fenced_code(body)):
        raw = match.group(1).strip()
        if raw.startswith("<") and raw.endswith(">"):
            raw = raw[1:-1].strip()
        else:
            raw = raw.split(maxsplit=1)[0]
        target = unquote(raw.split("#", 1)[0].split("?", 1)[0])
        if not target or target.startswith("#") or target.startswith("//"):
            continue
        if re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", target):
            continue
        if target.startswith("/"):
            normalized = posixpath.normpath(target.lstrip("/"))
        else:
            normalized = posixpath.normpath(posixpath.join(posixpath.dirname(source), target))
        if target.endswith("/"):
            normalized = posixpath.join(normalized, "index.md")
        yield normalized


def validate_bundle(root: Path) -> Dict[str, object]:
    root = Path(root)
    if not root.is_dir():
        raise ValueError("ROOT must be an existing directory")

    files = _markdown_files(root)
    existing = {path.relative_to(root).as_posix() for path in files}
    errors: List[Dict[str, str]] = []
    warnings: List[Dict[str, str]] = []
    dirs_with_concepts = set()
    dirs_with_indexes = set()
    bodies: List[Tuple[str, str]] = []

    for path in files:
        relative = path.relative_to(root).as_posix()
        directory = posixpath.dirname(relative)
        basename = posixpath.basename(relative).lower()
        text, read_error = _read(path)
        present, parse_error, frontmatter, body = parse_frontmatter(text)

        if basename == "index.md":
            dirs_with_indexes.add(directory)
            if read_error:
                errors.append(_finding("E3", relative, "index.md is not readable UTF-8"))
            elif present:
                root_index = "/" not in relative
                only_version = set(frontmatter) <= {"okf_version"}
                if not root_index or not only_version or parse_error:
                    message = (
                        "root index.md frontmatter may only carry okf_version"
                        if root_index
                        else "index.md must not have frontmatter"
                    )
                    errors.append(_finding("E3", relative, message))
            bodies.append((relative, body if present else text))
            continue

        if basename == "log.md":
            if read_error:
                errors.append(_finding("E3", relative, "log.md is not readable UTF-8"))
            elif present:
                errors.append(_finding("E3", relative, "log.md must not have frontmatter"))
            log_body = body if present else text
            for level, heading in HEADING_RE.findall(log_body):
                if len(level) == 2 and not ISO_DATE_RE.fullmatch(heading.strip()):
                    warnings.append(
                        _finding(
                            "W5",
                            relative,
                            "log heading `## {}` is not an ISO date (YYYY-MM-DD)".format(
                                heading.strip()
                            ),
                        )
                    )
            bodies.append((relative, log_body))
            continue

        dirs_with_concepts.add(directory)
        if read_error or not present or parse_error:
            message = (
                "frontmatter is not parseable YAML"
                if present or read_error
                else "missing YAML frontmatter block"
            )
            errors.append(_finding("E1", relative, message))
            continue

        if not frontmatter.get("type"):
            errors.append(
                _finding("E2", relative, "missing required frontmatter field `type`")
            )
        if not frontmatter.get("title") or not frontmatter.get("description"):
            warnings.append(
                _finding(
                    "W1",
                    relative,
                    "missing recommended `title` and/or `description`",
                )
            )
        if "timestamp" not in frontmatter:
            warnings.append(
                _finding(
                    "W3",
                    relative,
                    "missing `timestamp` (ISO 8601 last-meaningful-change)",
                )
            )
        bodies.append((relative, body))

    for source, body in bodies:
        for target in _internal_targets(source, body):
            if target == ".." or target.startswith("../") or target not in existing:
                warnings.append(
                    _finding("W2", source, "broken internal link -> `{}`".format(target))
                )

    for directory in sorted(dirs_with_concepts):
        if directory not in dirs_with_indexes:
            warnings.append(
                _finding(
                    "W4",
                    directory or "/",
                    "directory has no index.md (progressive disclosure)",
                )
            )

    errors.sort(key=lambda item: (item["path"], item["rule"], item["message"]))
    warnings.sort(key=lambda item: (item["path"], item["rule"], item["message"]))
    return {
        "conformant": not errors,
        "errors": errors,
        "warnings": warnings,
        "checked_notes": len(files),
    }


def _render_text(report: Dict[str, object]) -> str:
    lines = [
        "OKF {}: {} markdown files".format(
            "CONFORMANT" if report["conformant"] else "NONCONFORMANT",
            report["checked_notes"],
        )
    ]
    for severity, key in (("ERROR", "errors"), ("WARNING", "warnings")):
        for item in report[key]:
            lines.append(
                "{} {} {}: {}".format(
                    severity, item["rule"], item["path"], item["message"]
                )
            )
    return "\n".join(lines)


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--format", choices=("json", "text"), default="text")
    args = parser.parse_args(argv)
    try:
        report = validate_bundle(args.root)
    except ValueError as error:
        parser.error(str(error))
    if args.format == "json":
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(_render_text(report))
    return 0 if report["conformant"] else 1


if __name__ == "__main__":
    sys.exit(main())
