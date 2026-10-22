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
  * every production function in every `crates/**/src` file stays under
    `function_max` (400) lines (architecture-4-04), unless it is listed in
    `functions` — then it must stay at or below its recorded span, which can
    only shrink. Entries are keyed `path::name` (so same-named functions in
    different files stay distinct); the three original bare-name entries
    (`execute_node`, `run_review_core`, `run_workflow_with_budget`) are still
    searched across crates/otto-server/src, so a split that moves one to
    another file keeps its cap.

Only PRODUCTION lines count (architecture-2-07): a top-level `#[cfg(test)]`
item (`mod tests { … }`, a test-only fn) and a bare top-level `mod tests { … }`
are skipped. Counting whole files let moving tests out "pay" for new
production code under the old cap.

    scripts/loc-ratchet.py                      # check (CI: Rust job)
    scripts/loc-ratchet.py --update-baseline    # LOWER entries to today's counts
                                                # (never raises; drops deleted files and
                                                # functions now under `function_max`). The
                                                # first run on a baseline without
                                                # `function_max` seeds it and records every
                                                # over-cap function's span.
    scripts/loc-ratchet.py --raise PATH|fn:NAME|fn:PATH::NAME ...
                                                # deliberately raise / add named entries
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
FUNCTION_MAX = 400


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
        if "{" in line and line.count("{") == line.count("}"):
            # A one-line body (`fn as_str(&self) -> &str { "x" }`): searching
            # for a closing brace would run into the next item's.
            out[m.group("name")] = max(out.get(m.group("name"), 0), 1)
            continue
        for j in range(i, len(lines)):
            if j > i and lines[j].rstrip() == close:
                out[m.group("name")] = max(out.get(m.group("name"), 0), j - i + 1)
                break
            if j == i and line.rstrip().endswith(";"):
                break  # a trait method declaration, no body
    return out


def production_text(path: Path) -> str:
    return "\n".join(production_lines(path.read_text(encoding="utf-8", errors="replace")))


def measure_functions(files: list[str], names: list[str]) -> dict[str, int]:
    """Spans keyed `path::name` for every production function in `files`,
    plus the legacy bare-name entries in `names` (longest match under
    FUNCTIONS_ROOT). Test-only items are skipped (`production_lines`), and
    `files` already excludes `tests/` dirs and `*_tests.rs`."""
    found: dict[str, int] = {}
    for f in files:
        spans = function_lengths(production_text(ROOT / f))
        for name, n in spans.items():
            found[f"{f}::{name}"] = n
            if name in names and "::" not in name and f.startswith(FUNCTIONS_ROOT + "/"):
                found[name] = max(found.get(name, 0), n)
    return found


def function_max(baseline: dict) -> int:
    return int(baseline.get("function_max", FUNCTION_MAX))


def legacy_covers(key: str, funcs: dict[str, int]) -> bool:
    """A `path::name` span is covered by a legacy bare-name entry when the file
    is under FUNCTIONS_ROOT and the bare name is tracked."""
    path, _, name = key.rpartition("::")
    return name in funcs and path.startswith(FUNCTIONS_ROOT + "/")


def over_cap(baseline: dict, fns: dict[str, int]) -> dict[str, int]:
    cap = function_max(baseline)
    return {k: n for k, n in fns.items() if "::" in k and n > cap}


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
    fns = fns or {}
    funcs = baseline.get("functions", {})
    for name, limit in sorted(funcs.items()):
        n = fns.get(name)
        if n is None:
            where = name.rpartition("::")[0] if "::" in name else FUNCTIONS_ROOT
            errors.append(f"fn {name}: not found under {where} — renamed? update `functions` in {BASELINE.name}")
        elif n > limit:
            errors.append(f"fn {name}: {n} lines > ratchet {limit} (+{n - limit}) — extract a helper instead of adding to it")
    cap_fn = function_max(baseline)
    for key, n in sorted(over_cap(baseline, fns).items()):
        if key not in funcs and not legacy_covers(key, funcs):
            errors.append(
                f"fn {key}: {n} lines — over the {cap_fn}-line function cap; split it "
                f"(or track it with --raise fn:{key})"
            )
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
    fns = fns or {}
    funcs = baseline.get("functions", {})
    cap_fn = function_max(baseline)
    # Only shrink: a bare legacy name keeps its entry while found; a
    # `path::name` entry is dropped once the function is gone (renamed or
    # split — the cap then covers whatever replaced it) or under the cap.
    out = {}
    for k, v in funcs.items():
        n = fns.get(k)
        if "::" not in k:
            out[k] = min(v, n if n is not None else v)
        elif n is not None and n > cap_fn:
            out[k] = min(v, n)
    if "function_max" not in baseline:
        # First adoption of the per-function cap: seed today's over-cap spans.
        # Afterwards a new over-cap function needs an explicit --raise.
        new["function_max"] = cap_fn
        for k, n in over_cap(baseline, fns).items():
            if k not in out and not legacy_covers(k, out):
                out[k] = n
    if out:
        new["functions"] = dict(sorted(out.items()))
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
