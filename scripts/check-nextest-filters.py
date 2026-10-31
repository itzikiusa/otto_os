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

Also run by scripts/check.sh when `.config/nextest.toml` changed. Run after
the workspace tests are built (CI: right after `cargo nextest run`)
so listing only re-runs `--list` on the existing binaries.

    scripts/check-nextest-filters.py              # list + check every disjunct
    scripts/check-nextest-filters.py --print      # print the disjuncts only
"""

from __future__ import annotations

import argparse
import json
import re
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


def regexes(disjunct: str) -> list[str]:
    """The `/regex/` literals inside one filterset disjunct."""
    out: list[str] = []
    cur: list[str] | None = None
    prev = ""
    for ch in disjunct:
        if ch == "/" and prev != "\\":
            if cur is None:
                cur = []
            else:
                out.append("".join(cur))
                cur = None
        elif cur is not None:
            cur.append(ch)
        prev = ch
    return out


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


def disjunct_packages(disjunct: str) -> set[str]:
    """Every `package(<name>)` predicate mentioned in a disjunct."""
    return set(re.findall(r"package\(([A-Za-z0-9_-]+)\)", disjunct))


def list_matches(disjunct: str, extra_cargo_args: list[str] | None = None) -> int:
    scope = extra_cargo_args if extra_cargo_args else ["--workspace"]
    proc = subprocess.run(
        [
            "cargo", "nextest", "list", *scope,
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
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--print", action="store_true")
    ap.add_argument("--packages", nargs="+")
    ap.add_argument("--cargo-arg", action="append", default=[])
    ns, _ = ap.parse_known_args(argv)

    config = tomllib.loads(CONFIG.read_text())
    disjuncts = []
    for profile, flt in override_filters(config):
        for d in split_top_level_or(flt):
            disjuncts.append((profile, d))
    if ns.print:
        for profile, d in disjuncts:
            print(f"[{profile}] {d}")
        return 0

    extra_args: list[str] = []
    if ns.packages:
        wanted = set(ns.packages)
        disjuncts = [
            (profile, d)
            for profile, d in disjuncts
            if disjunct_packages(d) and disjunct_packages(d) <= wanted
        ]
        if not disjuncts:
            return 0
        # Keep the caller's full `--packages` set on the cargo invocation so
        # resolver-v2 feature unification matches the preceding `cargo nextest
        # run` step and reuses its compiled binaries without recompiling.
        for pkg in ns.packages:
            extra_args.extend(["-p", pkg])
        extra_args.extend(ns.cargo_arg)
    elif ns.cargo_arg:
        extra_args.extend(["--workspace", *ns.cargo_arg])

    # Build/list the target scope ONCE up front: when the tests don't compile
    # (the nextest step already failed on that), every per-disjunct
    # `cargo nextest list` would retry the failed build — minutes of noise.
    scope = extra_args if extra_args else ["--workspace"]
    pre = subprocess.run(
        ["cargo", "nextest", "list", *scope, "--message-format", "json"],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
    )
    if pre.returncode != 0:
        print("::error::cargo nextest list failed (tests do not build) — fix the build first")
        return 1
    orphaned = []
    for profile, d in disjuncts:
        n = list_matches(d, extra_args or None)
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
