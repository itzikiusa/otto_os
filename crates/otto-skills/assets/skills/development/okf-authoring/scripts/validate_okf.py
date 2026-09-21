#!/usr/bin/env python3
"""Deterministic, read-only OKF v0.2/v0.1 conformance validator."""

import argparse
from datetime import datetime
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
YAML_NON_STRING_SCALARS = {
    "false",
    "null",
    "true",
    "~",
}


def _finding(rule: str, path: str, message: str) -> Dict[str, str]:
    return {"rule": rule, "path": path, "message": message}


def _scalar(raw: str) -> Optional[str]:
    value = raw.strip()
    if not value or value.startswith("#"):
        return None
    if value.startswith('"'):
        try:
            parsed, end = json.JSONDecoder().raw_decode(value)
        except (TypeError, ValueError):
            return value
        remainder = value[end:].strip()
        if remainder and not remainder.startswith("#"):
            return value
        return parsed if isinstance(parsed, str) else value
    if value.startswith("'") and value.endswith("'") and len(value) >= 2:
        return value[1:-1].replace("''", "'")
    if " #" in value:
        value = value.split(" #", 1)[0].rstrip()
    if not value or value.lower() in YAML_NON_STRING_SCALARS:
        return None
    if value.startswith(("[", "{")):
        return None
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
        plain_value = raw_value.strip()
        if (
            plain_value
            and not plain_value.startswith(('"', "'", "[", "{"))
            and re.search(r":\s", plain_value.split(" #", 1)[0])
        ):
            parse_error = True
        values[key] = _scalar(raw_value)
        last_key = key

    return True, parse_error, values, "".join(lines[closing + 1 :])


def _flow_parts(raw: str) -> List[str]:
    """Split the documented YAML flow forms without splitting quoted URLs."""
    parts, start, depth, quote, escaped = [], 0, 0, None, False
    for index, char in enumerate(raw):
        if quote:
            if char == quote and not escaped:
                quote = None
            escaped = char == "\\" and not escaped
        elif char in "\"'":
            quote = char
        elif char in "[{":
            depth += 1
        elif char in "]}":
            depth -= 1
        elif char == "," and depth == 0:
            parts.append(raw[start:index].strip())
            start = index + 1
    parts.append(raw[start:].strip())
    return [part for part in parts if part]


def _metadata_value(raw: str):
    raw = raw.strip()
    quote, escaped = None, False
    for index, char in enumerate(raw):
        if quote:
            if char == quote and not escaped:
                quote = None
            escaped = char == "\\" and not escaped
        elif char in "\"'":
            quote = char
        elif char == "#" and (index == 0 or raw[index - 1].isspace()):
            raw = raw[:index].rstrip()
            break
    if re.fullmatch(r"[-+]?\d+(?:\.\d+)?", raw):
        return float(raw) if "." in raw else int(raw)
    if raw.startswith("{") and raw.endswith("}"):
        result = {}
        for item in _flow_parts(raw[1:-1]):
            key, separator, value = item.partition(":")
            if separator:
                result[key.strip().strip("\"'")] = _metadata_value(value)
        return result
    if raw.startswith("[") and raw.endswith("]"):
        return [_metadata_value(item) for item in _flow_parts(raw[1:-1])]
    return _scalar(raw)


def optional_metadata(text: str) -> dict:
    """Read common block/flow mappings and lists, without executing YAML tags.

    This advisory reader is deliberately not a complete YAML implementation.
    Anchors, tags and multiline flow syntax need Otto's full runtime parser.
    Unknown keys are untouched; malformed optional shapes remain warnings.
    """
    lines = text.lstrip("\ufeff").splitlines()
    if not lines or lines[0] != "---":
        return {}
    try:
        end = lines.index("---", 1)
    except ValueError:
        return {}
    entries = [(len(line) - len(line.lstrip()), line.strip()) for line in lines[1:end]
               if line.strip() and not line.lstrip().startswith("#")]

    def block(index, indent):
        sequence = entries[index][1].startswith("- ")
        result = [] if sequence else {}
        while index < len(entries) and entries[index][0] == indent:
            content = entries[index][1]
            if sequence:
                if not content.startswith("- "):
                    break
                content = content[2:].strip()
                if content.startswith(("{", "[")):
                    value = _metadata_value(content)
                elif KEY_RE.match(content):
                    key, rest = KEY_RE.match(content).groups()
                    value = {key: _metadata_value(rest)}
                else:
                    value = _metadata_value(content)
                index += 1
                if index < len(entries) and entries[index][0] > indent:
                    child, index = block(index, entries[index][0])
                    if isinstance(value, dict) and isinstance(child, dict):
                        value.update(child)
                result.append(value)
            else:
                match = KEY_RE.match(content)
                index += 1
                if not match:
                    continue
                key, rest = match.groups()
                value = _metadata_value(rest)
                if index < len(entries) and entries[index][0] > indent:
                    child, index = block(index, entries[index][0])
                    if not rest.strip() or rest.lstrip().startswith("#"):
                        value = child
                result[key] = value
        return result, index

    return block(0, entries[0][0])[0] if entries else {}


def _nonempty(value) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _datetime(value) -> bool:
    if not isinstance(value, str):
        return False
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        return "T" in value and parsed.utcoffset() is not None
    except ValueError:
        return False


def optional_metadata_warnings(metadata: dict) -> List[str]:
    warnings = []
    def event(value):
        return isinstance(value, dict) and _nonempty(value.get("by")) and _datetime(value.get("at"))
    if "generated" in metadata and not event(metadata["generated"]):
        warnings.append("`generated` should carry nonempty `by` and an offset-bearing ISO datetime `at`")
    if "verified" in metadata:
        value = metadata["verified"]
        events = [value] if isinstance(value, dict) else value
        if not isinstance(events, list) or not all(event(item) for item in events):
            warnings.append("`verified` should be a {by, at} mapping or list of verification mappings")
    if "sources" in metadata:
        value = metadata["sources"]
        if not isinstance(value, list) or not all(isinstance(item, dict) and _nonempty(item.get("resource")) for item in value):
            warnings.append("`sources` should be a list of mappings with a nonempty `resource` (path, URI or scope descriptor)")
    if "status" in metadata and metadata["status"] not in ("draft", "stable", "deprecated"):
        warnings.append("recommended lifecycle `status` values are draft, stable or deprecated")
    if "stale_after" in metadata and not _datetime(metadata["stale_after"]):
        warnings.append("`stale_after` should be an absolute ISO datetime with a UTC offset")
    return warnings


def _markdown_files(root: Path) -> List[Path]:
    return sorted(
        (path for path in root.rglob("*.md") if path.is_file()),
        key=lambda path: path.relative_to(root).as_posix(),
    )


def _all_files(root: Path) -> List[Path]:
    return sorted(
        (path for path in root.rglob("*") if path.is_file()),
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


def _internal_targets(source: str, body: str) -> Iterable[Tuple[str, str]]:
    for match in LINK_RE.finditer(_strip_fenced_code(body)):
        raw = match.group(1).strip()
        if raw.startswith("<") and raw.endswith(">"):
            raw = raw[1:-1].strip()
        else:
            raw = raw.split(maxsplit=1)[0]
        target = unquote(raw.split("#", 1)[0].split("?", 1)[0])
        if not target or target.startswith("#") or target.startswith("//"):
            continue
        scheme = re.match(r"^([A-Za-z][A-Za-z0-9+.-]*):", target)
        if scheme and scheme.group(1).lower() == "file":
            yield "L1", raw
            continue
        if scheme:
            continue
        if target.endswith("/"):
            yield "W2_DIRECTORY", raw
            continue
        if target.startswith("/"):
            normalized = posixpath.normpath(target.lstrip("/"))
        else:
            normalized = posixpath.normpath(posixpath.join(posixpath.dirname(source), target))
        yield "W2", normalized


def _file_uri_duplicates_vault_note(raw: str, existing: set) -> bool:
    path = unquote(raw.split("#", 1)[0].split("?", 1)[0])
    path = path[len("file://") :] if path.lower().startswith("file://") else path
    parts = [part for part in path.split("/") if part]
    for start in range(max(0, len(parts) - 1)):
        if "/".join(parts[start:]) in existing:
            return True
    return False


def validate_bundle(root: Path) -> Dict[str, object]:
    root = Path(root)
    if not root.is_dir():
        raise ValueError("ROOT must be an existing directory")

    files = _markdown_files(root)
    existing = {path.relative_to(root).as_posix() for path in _all_files(root)}
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
        metadata = optional_metadata(text)
        changed_at = metadata.get("generated", {}).get("at") if isinstance(metadata.get("generated"), dict) else None
        if "generated" not in metadata:
            changed_at = frontmatter.get("timestamp")
        if not _nonempty(changed_at):
            warnings.append(
                _finding(
                    "W3",
                    relative,
                    "missing `generated.at` (or legacy `timestamp`) for the last content change",
                )
            )
        warnings.extend(_finding("W6", relative, message) for message in optional_metadata_warnings(metadata))
        bodies.append((relative, body))

    for source, body in bodies:
        for rule, target in _internal_targets(source, body):
            if rule == "L1" and _file_uri_duplicates_vault_note(target, existing):
                warnings.append(
                    _finding(
                        "L1",
                        source,
                        "machine-local file URI duplicates a Vault note -> `{}`".format(
                            target
                        ),
                    )
                )
            elif rule == "W2_DIRECTORY":
                warnings.append(
                    _finding(
                        "W2",
                        source,
                        "directory link must name index.md -> `{}`".format(target),
                    )
                )
            elif target == ".." or target.startswith("../") or target not in existing:
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
    parser.add_argument(
        "--strict",
        action="store_true",
        help="exit non-zero when deterministic warnings are present",
    )
    args = parser.parse_args(argv)
    try:
        report = validate_bundle(args.root)
    except ValueError as error:
        parser.error(str(error))
    if args.format == "json":
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(_render_text(report))
    return 0 if report["conformant"] and (not args.strict or not report["warnings"]) else 1


if __name__ == "__main__":
    sys.exit(main())
