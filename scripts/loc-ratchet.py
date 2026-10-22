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
    `new_file_max` lines — a new god-module fails until it is split;
  * each tracked god FUNCTION (`functions`, keyed by name and searched across
    crates/otto-server/src, so a split that moves it to another file keeps
    the cap) must stay at or below its recorded line count.

Only PRODUCTION lines count (architecture-2-07): a top-level `#[cfg(test)]`
item (`mod tests { … }`, a test-only fn) and a bare top-level `mod tests { … }`
are skipped. Counting whole files let moving tests out "pay" for new
production code under the old cap.

    scripts/loc-ratchet.py                      # check (CI: Rust job)
    scripts/loc-ratchet.py --update-baseline    # LOWER entries to today's counts
                                                # (never raises; drops deleted files)
    scripts/loc-ratchet.py --raise PATH|fn:NAME ...  # deliberately raise named entries
                                                # (say why in the PR — the split is the fix)
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASELINE = ROOT / "scripts" / "loc-baseline.json"
TOTALS = {"otto-server": "crates/otto-server/src"}
FUNCTIONS_ROOT = "crates/otto-server/src"


def production_lines(text: str) -> list[str]:
    """The file's lines minus top-level test-only items (rustfmt layout: a
    top-level item's closing brace is a `}` in column 0)."""
    lines = text.splitlines()
    out: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        is_test_attr = line.rstrip() == "#[cfg(test)]"
        if is_test_attr or re.match(r"mod tests\s*\{", line):
            j = i + 1 if is_test_attr else i
            # Further attributes / doc comments on the same item.
            while is_test_attr and j < len(lines) and lines[j].startswith(("#[", "///")):
                j += 1
            if j < len(lines) and lines[j].rstrip().endswith(";"):
                i = j + 1  # `mod tests;` / `use …;` — a one-line item
                continue
            while j < len(lines) and lines[j].rstrip() != "}":
                j += 1
            i = j + 1
            continue
        out.append(line)
        i += 1
    return out


def count_lines(path: Path) -> int:
    return len(production_lines(path.read_text(encoding="utf-8", errors="replace")))


FN_DEF = re.compile(r"^(?P<indent>\s*)(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(?P<name>\w+)\b")


def function_lengths(text: str) -> dict[str, int]:
    """Line span of every `fn` (signature through its closing brace, rustfmt
    layout: the brace sits at the `fn` line's indent). A name defined twice
    keeps its longest body."""
    lines = text.splitlines()
    out: dict[str, int] = {}
    for i, line in enumerate(lines):
        m = FN_DEF.match(line)
        if not m:
            continue
        close = m.group("indent") + "}"
        for j in range(i, len(lines)):
            if j > i and lines[j].rstrip() == close:
                out[m.group("name")] = max(out.get(m.group("name"), 0), j - i + 1)
                break
            if j == i and line.rstrip().endswith(";"):
                break  # a trait method declaration, no body
    return out


def measure_functions(files: list[str], names: list[str]) -> dict[str, int]:
    found: dict[str, int] = {}
    for f in files:
        if not f.startswith(FUNCTIONS_ROOT + "/"):
            continue
        for name, n in function_lengths((ROOT / f).read_text(encoding="utf-8", errors="replace")).items():
            if name in names:
                found[name] = max(found.get(name, 0), n)
    return found


def is_test_file(path: str) -> bool:
    """A test-only source file (an integration `tests/` dir, a `tests.rs` /
    `*_tests.rs` module, benches): none of it is production code."""
    parts = path.split("/")
    name = parts[-1]
    return "tests" in parts[:-1] or "benches" in parts[:-1] or name == "tests.rs" or name.endswith("_tests.rs")


def rust_files() -> list[str]:
    # Untracked (not yet committed) files count too: a split in progress puts
    # moved code in new files, and a tracked function may now live there.
    out = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "crates"], cwd=ROOT, text=True
    )
    return sorted(
        {l for l in out.splitlines() if l.endswith(".rs") and not is_test_file(l) and (ROOT / l).is_file()}
    )


def measure(files: list[str]) -> tuple[dict[str, int], dict[str, int]]:
    counts = {f: count_lines(ROOT / f) for f in files}
    totals = {
        name: sum(n for f, n in counts.items() if f.startswith(prefix + "/"))
        for name, prefix in TOTALS.items()
    }
    return counts, totals


def check(baseline: dict, counts: dict[str, int], totals: dict[str, int], fns: dict[str, int] | None = None) -> list[str]:
    errors = []
    for name, limit in sorted(baseline.get("functions", {}).items()):
        n = (fns or {}).get(name)
        if n is None:
            errors.append(f"fn {name}: not found under {FUNCTIONS_ROOT} — renamed? update `functions` in {BASELINE.name}")
        elif n > limit:
            errors.append(f"fn {name}: {n} lines > ratchet {limit} (+{n - limit}) — extract a helper instead of adding to it")
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


def lowered(baseline: dict, counts: dict[str, int], totals: dict[str, int], fns: dict[str, int] | None = None) -> dict:
    cap = baseline["new_file_max"]
    # A tracked file whose production count fell under the cap is no longer a
    # god-module: drop it (the untracked rule keeps it under the cap).
    files = {f: min(limit, counts[f]) for f, limit in baseline["files"].items() if f in counts and counts[f] >= cap}
    tots = {k: min(v, totals.get(k, v)) for k, v in baseline["totals"].items()}
    new = {**baseline, "files": dict(sorted(files.items())), "totals": tots}
    if "functions" in baseline:
        new["functions"] = {k: min(v, (fns or {}).get(k, v)) for k, v in sorted(baseline["functions"].items())}
    return new


def raised(baseline: dict, counts: dict[str, int], totals: dict[str, int], paths: list[str], fns: dict[str, int] | None = None) -> dict:
    files = dict(baseline["files"])
    tots = dict(baseline["totals"])
    funcs = dict(baseline.get("functions", {}))
    for p in paths:
        if p in TOTALS:
            tots[p] = totals[p]
        elif p.startswith("fn:") and p[3:] in (fns or {}):
            funcs[p[3:]] = (fns or {})[p[3:]]
        elif p in counts:
            files[p] = counts[p]
        else:
            raise SystemExit(f"loc-ratchet: unknown path, total or fn:NAME {p!r}")
    new = {**baseline, "files": dict(sorted(files.items())), "totals": tots}
    if funcs:
        new["functions"] = dict(sorted(funcs.items()))
    return new


def main(argv: list[str]) -> int:
    baseline = json.loads(BASELINE.read_text())
    files = rust_files()
    counts, totals = measure(files)
    fns = measure_functions(files, list(baseline.get("functions", {})))
    if "--update-baseline" in argv or "--raise" in argv:
        if "--raise" in argv:
            new = raised(baseline, counts, totals, argv[argv.index("--raise") + 1 :], fns)
        else:
            new = lowered(baseline, counts, totals, fns)
        BASELINE.write_text(json.dumps(new, indent=2) + "\n")
        print(f"loc-ratchet: wrote {BASELINE.relative_to(ROOT)}")
        return 0
    errors = check(baseline, counts, totals, fns)
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
