#!/usr/bin/env python3
"""Conservatively inventory repository surfaces for a Vault scan."""

import argparse
import hashlib
import json
import os
import re
import sys
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from subprocess import CalledProcessError, run

SKIP_DIRS = {
    ".git", ".hg", ".svn", ".idea", ".vscode", "node_modules", "vendor",
    "target", "dist", "build", "coverage", "__pycache__", ".venv", "venv",
}
TEXT_SUFFIXES = {
    ".bash", ".c", ".cc", ".clj", ".cljs", ".cpp", ".cs", ".dart",
    ".ex", ".exs", ".fs", ".fsx", ".go", ".gql", ".gradle", ".graphql",
    ".h", ".hcl", ".hpp", ".java", ".js", ".jsx", ".kt", ".kts",
    ".lua", ".php", ".pl", ".proto", ".py", ".rb", ".rs", ".scala",
    ".sh", ".sql", ".swift", ".tf", ".toml", ".ts", ".tsx", ".xml",
    ".yaml", ".yml", ".json", ".zsh",
}
MAX_BYTES = 2 * 1024 * 1024

PATTERNS = (
    (
        "api",
        re.compile(
            r"(?ix)(?:"
            r"\broute\s*\(|\.route\s*\(|"
            r"\b(?:app|router|routes|server|api|mux|group|fastify|fiber|gin|engine)\s*\.\s*(?:get|post|put|patch|delete|options|head)\s*\(|"
            r"\#\s*\[\s*(?:get|post|put|patch|delete|route)\s*\(|"
            r"@(?:get|post|put|patch|delete|request)(?:mapping)?\s*\(|"
            r"\b(?:add_url_rule|add_api_route|HandleFunc)\s*\(|"
            r"\b(?:rpc)\s+[A-Za-z_]\w*\s*\(|"
            r"\btype\s+(?:Query|Mutation|Subscription)\b|"
            r"\badd_service\s*\()"
        ),
    ),
    (
        "data",
        re.compile(
            r"(?ix)(?:"
            r"\b(?:sqlx::(?:query|query_as)|diesel::|sea_query::)[A-Za-z_:]*\s*!?\s*\(|"
            r"\b(?:db|database|pool|conn|client|store|repo|repository)\s*\.\s*(?:query|queryContext|exec|execContext|execute|insert|update|delete)\s*\(|"
            r"\b(?:db|database|store|repo|repository|mongo|client)\s*\.\s*(?:collection|table)\s*\(|"
            r"\b(?:redis|cache|redis_client|redis_conn|conn)\s*\.\s*(?:get|set|mget|mset|hget|hset|scan|expire|lpush|rpush|sadd|zadd)\s*\(|"
            r"\b(?:findOne|findMany|findBy[A-Z]\w*|aggregate|bulkWrite|updateOne|replaceOne|deleteOne)\s*\(|"
            r"\b(?:JpaRepository|CrudRepository|EntityManager|Repository<|Model\.(?:find|create|update|delete))\b)"
        ),
    ),
    ("messaging", re.compile(r"(?i)\b(?:kafka|producer|consumer|publish|subscribe|topic|queue|nats|rabbitmq|dead.?letter)\b")),
    ("worker", re.compile(r"(?i)\b(?:cron|schedule|interval|ticker|background[_ ]?job|worker|reconcile|celery|sidekiq)\b")),
    ("runtime", re.compile(r"(?i)(?:\b(?:startup|shutdown|graceful|signal|serve|listen)\b|\bmain\s*\(|\btokio::spawn\b|\badd_signal_handler\b)")),
)
PY_API_PATTERN = re.compile(r"\b(?:path|re_path)\s*\(")
GO_API_PATTERN = re.compile(r"\b(?:r|e|g)\.(?:GET|POST|PUT|PATCH|DELETE|OPTIONS|HEAD)\s*\(")
SQL_PATTERN = re.compile(
    r"(?i)\b(?:select\s+.+\s+from|insert\s+into|update\s+[A-Za-z_]|delete\s+from|create\s+table|alter\s+table)\b"
)


def _safe_git(root: Path, *args: str, check=True):
    """Run metadata-only git commands with hooks and external diffs disabled."""
    env = os.environ.copy()
    env.update({
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_EXTERNAL_DIFF": "",
    })
    return run(
        [
            "git", "-c", "core.hooksPath=/dev/null", "-c", "diff.external=",
            "-C", str(root), *args,
        ],
        check=check,
        capture_output=True,
        text=True,
        env=env,
    )


def _git_head(root: Path) -> str:
    try:
        return _safe_git(root, "rev-parse", "--verify", "HEAD^{commit}").stdout.strip()
    except (OSError, CalledProcessError):
        return "unknown"


def _incremental_paths(root: Path, baseline: str):
    if not baseline or baseline.startswith("-") or not re.fullmatch(r"[A-Za-z0-9_./~^-]+", baseline):
        return None, "invalid baseline revision"
    try:
        resolved = _safe_git(
            root, "rev-parse", "--verify", f"{baseline}^{{commit}}"
        ).stdout.strip()
        ancestor = _safe_git(
            root, "merge-base", "--is-ancestor", resolved, "HEAD", check=False
        )
        if ancestor.returncode != 0:
            return None, "baseline is not an ancestor of HEAD"
        changed = _safe_git(
            root,
            "diff",
            "--no-ext-diff",
            "--name-only",
            "--diff-filter=ACDMRTUXB",
            resolved,
            "--",
        ).stdout.splitlines()
        untracked = _safe_git(
            root, "ls-files", "--others", "--exclude-standard", "--"
        ).stdout.splitlines()
        return sorted(set(changed + untracked)), None
    except (OSError, CalledProcessError):
        return None, "invalid or unavailable baseline revision"


def _inside(root: Path, path: Path) -> bool:
    try:
        path.resolve(strict=False).relative_to(root)
        return True
    except (OSError, ValueError):
        return False


def _classify_file(root: Path, path: Path, scanned, exclusions):
    relative = path.relative_to(root).as_posix()
    try:
        if path.is_symlink():
            exclusions.append({"path": relative, "reason": "symlink"})
            return
        if not path.is_file():
            exclusions.append({"path": relative, "reason": "deleted or missing"})
            return
        if path.suffix.lower() not in TEXT_SUFFIXES:
            exclusions.append({"path": relative, "reason": "unsupported extension"})
            return
        if path.stat().st_size > MAX_BYTES:
            exclusions.append({"path": relative, "reason": "too large"})
            return
    except OSError:
        exclusions.append({"path": relative, "reason": "unreadable"})
        return
    scanned.append(relative)


def _discover_files(root: Path, selected=None):
    scanned = []
    exclusions = []
    if selected is not None:
        for relative in sorted(set(selected)):
            candidate = root / relative
            if Path(relative).is_absolute() or not _inside(root, candidate):
                exclusions.append({"path": str(relative), "reason": "outside repository"})
                continue
            _classify_file(root, candidate, scanned, exclusions)
        return scanned, exclusions

    for base, dirs, names in os.walk(root, followlinks=False):
        base_path = Path(base)
        kept = []
        for name in sorted(dirs):
            path = base_path / name
            relative = path.relative_to(root).as_posix()
            if name in SKIP_DIRS or name.startswith("."):
                exclusions.append({"path": relative + "/", "reason": "skipped directory"})
            elif path.is_symlink():
                exclusions.append({"path": relative + "/", "reason": "symlink"})
            else:
                kept.append(name)
        dirs[:] = kept
        for name in sorted(names):
            _classify_file(root, base_path / name, scanned, exclusions)
    return sorted(scanned), sorted(exclusions, key=lambda item: (item["path"], item["reason"]))


def _summary(line: str) -> str:
    return re.sub(r"\s+", " ", line.strip())[:240]


def _code_line(line: str) -> str:
    stripped = line.lstrip()
    if stripped.startswith(("//", "/*", "*", "--")):
        return ""
    if stripped.startswith("#") and not stripped.startswith("#["):
        return ""
    return line


def _candidate_kinds(line: str, suffix: str):
    code = _code_line(line)
    if not code:
        return []
    kinds = [kind for kind, pattern in PATTERNS if pattern.search(code)]
    if suffix == ".py" and PY_API_PATTERN.search(code):
        kinds.append("api")
    if suffix == ".go" and GO_API_PATTERN.search(code):
        kinds.append("api")
    quoted_sql = suffix == ".sql" or any(marker in code for marker in ('"', "'", "`"))
    if quoted_sql and SQL_PATTERN.search(code):
        kinds.append("data")
    return list(dict.fromkeys(kinds))


def _signature(kind: str, line: str) -> str:
    normalized = _summary(line)
    if kind == "api":
        operation = _api_operation(normalized)
        if operation:
            return f'{operation[0]} {operation[1]}'
    return normalized.lower()


def _api_operation(line: str):
    patterns = (
        r"(?i)\.route\s*\(\s*[\"']([^\"']+)[\"']\s*,\s*(get|post|put|patch|delete|options|head)",
        r"(?i)#\s*\[\s*(get|post|put|patch|delete)\s*\(\s*[\"']([^\"']+)[\"']",
        r"(?i)@(get|post|put|patch|delete)(?:mapping)?\s*\(\s*[\"']([^\"']+)[\"']",
        r"(?i)\.(get|post|put|patch|delete|options|head)\s*\(\s*[\"']([^\"']+)[\"']",
    )
    for index, pattern in enumerate(patterns):
        match = re.search(pattern, line)
        if not match:
            continue
        if index == 0:
            return match.group(2).upper(), match.group(1)
        return match.group(1).upper(), match.group(2)
    rpc = re.search(r"(?i)\brpc\s+([A-Za-z_]\w*)\s*\(", line)
    if rpc:
        return "RPC", rpc.group(1)
    return None


def inventory(root: Path, changed_since=None, include_files=None):
    root = root.resolve()
    if not root.is_dir():
        raise ValueError(f"repository is not a directory: {root}")

    mode = "full"
    fallback_reason = None
    selected = None
    if changed_since:
        selected, fallback_reason = _incremental_paths(root, changed_since)
        if selected is None:
            mode = "full-fallback"
        else:
            mode = "incremental"
            selected = sorted(set(selected + list(include_files or [])))
    elif include_files:
        selected = sorted(set(include_files))
        mode = "focused"

    scanned_files, exclusions = _discover_files(root, selected)
    candidates = []
    occurrences = defaultdict(int)
    for relative in scanned_files:
        path = root / relative
        try:
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            exclusions.append({"path": relative, "reason": "unreadable"})
            continue
        for line_number, line in enumerate(lines, 1):
            for kind in _candidate_kinds(line, path.suffix.lower()):
                signature = _signature(kind, line)
                key = (kind, relative, signature)
                occurrence = occurrences[key]
                occurrences[key] += 1
                digest = hashlib.sha1(
                    f"{kind}\0{relative}\0{signature}\0{occurrence}".encode()
                ).hexdigest()[:12]
                candidate = {
                    "id": f"{kind}:{digest}",
                    "kind": kind,
                    "name": _summary(line),
                    "signature": signature,
                    "path": relative,
                    "line": line_number,
                    "evidence": f"{relative}:{line_number}",
                }
                operation = _api_operation(line) if kind == "api" else None
                if operation:
                    candidate.update({"method": operation[0], "route": operation[1]})
                candidates.append(candidate)
    candidates.sort(key=lambda item: (item["path"], item["line"], item["kind"], item["id"]))
    by_kind = Counter(item["kind"] for item in candidates)
    exclusions.sort(key=lambda item: (item["path"], item["reason"]))
    return {
        "version": 2,
        "repo": str(root),
        "commit": _git_head(root),
        "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "mode": mode,
        "requested_changed_since": changed_since,
        "fallback_reason": fallback_reason,
        "scanned_files": scanned_files,
        "exclusions": exclusions,
        "counts": {
            "files_scanned": len(scanned_files),
            "files_excluded": len(exclusions),
            "candidates": len(candidates),
            "by_kind": dict(sorted(by_kind.items())),
        },
        "candidates": candidates,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("repo", type=Path)
    parser.add_argument("--changed-since", help="scan files changed since an ancestor revision; invalid baselines safely fall back to full")
    parser.add_argument("--include-file", action="append", default=[], help="also scan a repository-relative registration/contract dependency")
    parser.add_argument("--format", choices=("json", "text"), default="text")
    args = parser.parse_args(argv)
    try:
        result = inventory(args.repo, args.changed_since, args.include_file)
    except ValueError as error:
        parser.error(str(error))
    if args.format == "json":
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        for item in result["candidates"]:
            print(f'{item["id"]}\t{item["kind"]}\t{item["evidence"]}\t{item["name"]}')
        print(
            f'# mode={result["mode"]} scanned={result["counts"]["files_scanned"]} '
            f'excluded={result["counts"]["files_excluded"]} candidates={result["counts"]["candidates"]}',
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
