#!/usr/bin/env python3
"""Conservatively inventory repository surfaces for a Vault full scan."""

import argparse
import hashlib
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

SKIP_DIRS = {
    ".git", ".hg", ".svn", ".idea", ".vscode", "node_modules", "vendor",
    "target", "dist", "build", "coverage", "__pycache__", ".venv", "venv",
}
TEXT_SUFFIXES = {
    ".c", ".cc", ".cpp", ".cs", ".go", ".h", ".hpp", ".java", ".js",
    ".jsx", ".kt", ".kts", ".php", ".py", ".rb", ".rs", ".scala",
    ".sql", ".swift", ".ts", ".tsx", ".yaml", ".yml", ".toml", ".json",
}
MAX_BYTES = 2 * 1024 * 1024

PATTERNS = (
    ("api", re.compile(r"(?i)(?:route\s*\(|\.(?:get|post|put|patch|delete)\s*\(|@(?:get|post|put|patch|delete)|HandleFunc\s*\()")),
    ("data", re.compile(r"(?i)(?:\b(?:select|insert\s+into|update|delete\s+from|create\s+table|alter\s+table)\b|\b(?:collection|table|query|exec)\s*\(|\b(?:get|set|hget|hset|scan|expire)\s*\()")),
    ("messaging", re.compile(r"(?i)\b(?:kafka|producer|consumer|publish|subscribe|topic|queue|nats|rabbitmq)\b")),
    ("worker", re.compile(r"(?i)\b(?:cron|schedule|interval|ticker|background[_ ]?job|worker|reconcile)\b")),
    ("runtime", re.compile(r"(?i)\b(?:startup|shutdown|graceful|signal|serve|listen|main\s*\()\b")),
)


def _git_head(root: Path) -> str:
    """Read HEAD without executing repository-controlled programs or hooks."""
    try:
        marker = root / ".git"
        if marker.is_file():
            value = marker.read_text(encoding="utf-8").strip()
            if not value.startswith("gitdir: "):
                return "unknown"
            git_dir = (root / value[8:]).resolve()
        else:
            git_dir = marker
        head = (git_dir / "HEAD").read_text(encoding="utf-8").strip()
        if not head.startswith("ref: "):
            return head
        reference = head[5:]
        candidates = [git_dir / reference]
        common_marker = git_dir / "commondir"
        if common_marker.is_file():
            common = (git_dir / common_marker.read_text(encoding="utf-8").strip()).resolve()
            candidates.append(common / reference)
        for candidate in candidates:
            if candidate.is_file():
                return candidate.read_text(encoding="utf-8").strip()
        for packed_root in [git_dir, *(candidate.parent.parent.parent for candidate in candidates[1:])]:
            packed = packed_root / "packed-refs"
            if packed.is_file():
                for line in packed.read_text(encoding="utf-8", errors="replace").splitlines():
                    if line and not line.startswith(("#", "^")):
                        commit, name = line.split(" ", 1)
                        if name == reference:
                            return commit
    except (OSError, ValueError):
        pass
    return "unknown"


def _files(root: Path):
    for base, dirs, names in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS and not d.startswith("."))
        for name in sorted(names):
            path = Path(base) / name
            if path.suffix.lower() not in TEXT_SUFFIXES:
                continue
            try:
                if path.is_symlink() or path.stat().st_size > MAX_BYTES:
                    continue
            except OSError:
                continue
            yield path


def _summary(line: str) -> str:
    return re.sub(r"\s+", " ", line.strip())[:180]


def inventory(root: Path):
    root = root.resolve()
    if not root.is_dir():
        raise ValueError(f"repository is not a directory: {root}")
    candidates = []
    for path in _files(root):
        relative = path.relative_to(root).as_posix()
        try:
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            continue
        for line_number, line in enumerate(lines, 1):
            for kind, pattern in PATTERNS:
                if not pattern.search(line):
                    continue
                evidence = f"{relative}:{line_number}"
                digest = hashlib.sha1(f"{kind}\0{evidence}\0{_summary(line)}".encode()).hexdigest()[:12]
                candidates.append({
                    "id": f"{kind}:{digest}",
                    "kind": kind,
                    "name": _summary(line),
                    "path": relative,
                    "line": line_number,
                    "evidence": evidence,
                })
    candidates.sort(key=lambda item: (item["path"], item["line"], item["kind"], item["id"]))
    return {
        "version": 1,
        "repo": str(root),
        "commit": _git_head(root),
        "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "candidates": candidates,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("repo", type=Path)
    parser.add_argument("--format", choices=("json", "text"), default="text")
    args = parser.parse_args(argv)
    try:
        result = inventory(args.repo)
    except ValueError as error:
        parser.error(str(error))
    if args.format == "json":
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        for item in result["candidates"]:
            print(f'{item["id"]}\t{item["kind"]}\t{item["evidence"]}\t{item["name"]}')
    return 0


if __name__ == "__main__":
    sys.exit(main())
