#!/usr/bin/env python3
"""Fail when a `.config/nextest.toml` override filter names tests that no
longer exist.

Every `[[profile.*.overrides]]` filter is split on its TOP-LEVEL `|` and each
disjunct is listed with `cargo nextest list -E <disjunct>`; a disjunct that
matches 0 tests fails the check. A crate split or module move otherwise
orphans an entry silently: the 2026-10 split left three wall-clock tests
pointing at otto-server paths that had moved to otto-automation/otto-swarm, so
they ran unscaled under full CI contention, the exact flake the `timing`
group exists to stop. Write overrides as `(A) | (B) | ...` with each
disjunct naming real tests so this guard can check every one.

Run after the workspace tests are built (CI: right after `cargo nextest run`)
so listing only re-runs `--list` on the existing binaries.

    scripts/check-nextest-filters.py              # list + check every disjunct
    scripts/check-nextest-filters.py --print      # print the disjuncts only
"""

from __future__ import annotations

import json
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONFIG = ROOT / ".config" / "nextest.toml"


def split_top_level_or(expr: str) -> list[str]:
    """Split a nextest filterset on `|` at paren depth 0, outside `/regex/`."""
    parts: list[str] = []
    depth = 0
    in_regex = False
    cur: list[str] = []
    prev = ""
    for ch in expr:
        if ch == "/" and prev != "\\":
            in_regex = not in_regex
        elif not in_regex:
            if ch == "(":
                depth += 1
            elif ch == ")":
                depth -= 1
            elif ch == "|" and depth == 0:
                parts.append("".join(cur))
                cur = []
                prev = ch
                continue
        cur.append(ch)
        prev = ch
    parts.append("".join(cur))
    return [" ".join(p.split()) for p in parts if p.strip()]


def override_filters(config: dict) -> list[tuple[str, str]]:
    """(profile, filter) for every override in every profile."""
    out = []
    for name, profile in config.get("profile", {}).items():
        for ov in profile.get("overrides", []):
            if "filter" in ov:
                out.append((name, ov["filter"]))
    return out


def count_matches(listing: dict) -> int:
    n = 0
    for suite in listing.get("rust-suites", {}).values():
        for case in suite.get("testcases", {}).values():
            if case.get("filter-match", {}).get("status") == "matches":
                n += 1
    return n


def list_matches(disjunct: str) -> int:
    proc = subprocess.run(
        [
            "cargo", "nextest", "list", "--workspace",
            "--run-ignored", "all",
            "--message-format", "json",
            "-E", disjunct,
        ],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        text=True,
    )
    if proc.returncode != 0:
        raise SystemExit(f"cargo nextest list failed for: {disjunct}")
    return count_matches(json.loads(proc.stdout))


def main(argv: list[str]) -> int:
    config = tomllib.loads(CONFIG.read_text())
    disjuncts = []
    for profile, flt in override_filters(config):
        for d in split_top_level_or(flt):
            disjuncts.append((profile, d))
    if "--print" in argv:
        for profile, d in disjuncts:
            print(f"[{profile}] {d}")
        return 0
    orphaned = []
    for profile, d in disjuncts:
        n = list_matches(d)
        print(f"{n:5d}  [{profile}] {d}")
        if n == 0:
            orphaned.append((profile, d))
    for profile, d in orphaned:
        print(
            f"::error file=.config/nextest.toml::override filter in profile "
            f"'{profile}' matches 0 tests (moved or renamed?): {d}"
        )
    return 1 if orphaned else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
