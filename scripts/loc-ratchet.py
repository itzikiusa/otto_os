#!/usr/bin/env python3
"""LOC ratchet for the Rust god-modules (S12-307): they may shrink, never grow.

Every fix iteration grew the modules that every otto-server edit recompiles
(workflow_engine.rs, modules.rs, routes/api_client.rs, ottod mcp_tools.rs…)
while the planned splits never started. Same shape as the UI's
scripts/ui-guards-baseline.json ratchet:

  * every tracked file (scripts/loc-baseline.json `files`) must stay at or
    below its recorded line count;
  * otto-server's total `src/**/*.rs` lines must stay at or below `totals`;
  * a Rust file under crates/ that is NOT tracked must stay under
    `new_file_max` lines — a new god-module fails until it is split.

    scripts/loc-ratchet.py                      # check (CI: Rust job)
    scripts/loc-ratchet.py --update-baseline    # LOWER entries to today's counts
                                                # (never raises; drops deleted files)
    scripts/loc-ratchet.py --raise PATH ...     # deliberately raise named entries
                                                # (say why in the PR — the split is the fix)
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASELINE = ROOT / "scripts" / "loc-baseline.json"
TOTALS = {"otto-server": "crates/otto-server/src"}


def count_lines(path: Path) -> int:
    with path.open("rb") as f:
        return sum(1 for _ in f)


def rust_files() -> list[str]:
    out = subprocess.check_output(["git", "ls-files", "crates"], cwd=ROOT, text=True)
    return [l for l in out.splitlines() if l.endswith(".rs") and (ROOT / l).is_file()]


def measure(files: list[str]) -> tuple[dict[str, int], dict[str, int]]:
    counts = {f: count_lines(ROOT / f) for f in files}
    totals = {
        name: sum(n for f, n in counts.items() if f.startswith(prefix + "/"))
        for name, prefix in TOTALS.items()
    }
    return counts, totals


def check(baseline: dict, counts: dict[str, int], totals: dict[str, int]) -> list[str]:
    errors = []
    tracked = baseline["files"]
    for f, limit in sorted(tracked.items()):
        n = counts.get(f)
        if n is not None and n > limit:
            errors.append(f"{f}: {n} lines > ratchet {limit} (+{n - limit}) — move code out instead of adding it")
    for name, limit in sorted(baseline["totals"].items()):
        n = totals.get(name, 0)
        if n > limit:
            errors.append(f"{name} src total: {n} lines > ratchet {limit} (+{n - limit})")
    cap = baseline["new_file_max"]
    for f, n in sorted(counts.items()):
        if f not in tracked and n >= cap:
            errors.append(f"{f}: {n} lines — a new file at/over {cap} lines; split it (or track it with --raise)")
    return errors


def lowered(baseline: dict, counts: dict[str, int], totals: dict[str, int]) -> dict:
    files = {f: min(limit, counts[f]) for f, limit in baseline["files"].items() if f in counts}
    tots = {k: min(v, totals.get(k, v)) for k, v in baseline["totals"].items()}
    return {**baseline, "files": dict(sorted(files.items())), "totals": tots}


def raised(baseline: dict, counts: dict[str, int], totals: dict[str, int], paths: list[str]) -> dict:
    files = dict(baseline["files"])
    tots = dict(baseline["totals"])
    for p in paths:
        if p in TOTALS:
            tots[p] = totals[p]
        elif p in counts:
            files[p] = counts[p]
        else:
            raise SystemExit(f"loc-ratchet: unknown path or total {p!r}")
    return {**baseline, "files": dict(sorted(files.items())), "totals": tots}


def main(argv: list[str]) -> int:
    baseline = json.loads(BASELINE.read_text())
    counts, totals = measure(rust_files())
    if "--update-baseline" in argv or "--raise" in argv:
        if "--raise" in argv:
            new = raised(baseline, counts, totals, argv[argv.index("--raise") + 1 :])
        else:
            new = lowered(baseline, counts, totals)
        BASELINE.write_text(json.dumps(new, indent=2) + "\n")
        print(f"loc-ratchet: wrote {BASELINE.relative_to(ROOT)}")
        return 0
    errors = check(baseline, counts, totals)
    for e in errors:
        print(f"::error::loc-ratchet: {e}")
    if not errors:
        print(
            "loc-ratchet: ok — "
            + ", ".join(f"{k} {totals[k]}/{v}" for k, v in baseline["totals"].items())
        )
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
