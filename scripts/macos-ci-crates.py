#!/usr/bin/env python3
"""Which workspace crates the macOS workflow (.github/workflows/macos.yml)
must lint + test on Darwin, and which paths must trigger it.

Linux CI never compiles `cfg(target_os = "macos")` code, so a crate with
macOS-only code (or a macOS-only TEST, like otto-automation's Seatbelt proof
for sandboxed scheduled tasks, or otto-ssh's real-OpenSSH SFTP test) is only
exercised if the macOS job names it. S12-306: two such crates were in no
macOS workflow, so a sandbox regression passed every CI run.

  * MACOS crates = every workspace crate whose sources or manifest mention
    `target_os = "macos"`, plus DARWIN_EXTRA (Darwin behaviour without an
    explicit cfg: PTY semantics, launchd/Keychain wiring).
  * TRIGGER paths = those crates plus every workspace crate that depends on
    them (the cargo-metadata reverse closure, as scripts/check.sh computes).

    scripts/macos-ci-crates.py            # print both lists
    scripts/macos-ci-crates.py --check    # fail when macos.yml drifts from them

`--check` asserts macos.yml's clippy and nextest `-p` lists equal the MACOS
crates and its pull_request `crates/<dir>/**` paths equal the TRIGGER set.
It runs in ci.yml's Rust job and in scripts/test_macos_ci_crates.py.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "macos.yml"
MACOS_CFG = re.compile(r'target_os\s*=\s*"macos"')
# Darwin-specific behaviour with no `target_os = "macos"` text: otto-pty's
# SIGHUP-on-close / TIOCSWINSZ / socket-path limits, ottod's launchd wiring.
DARWIN_EXTRA = {"otto-pty", "ottod"}


def metadata() -> dict:
    return json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"],
            cwd=ROOT,
        )
    )


def crate_dirs(meta: dict) -> dict[str, Path]:
    members = set(meta["workspace_members"])
    return {
        p["name"]: Path(p["manifest_path"]).parent
        for p in meta["packages"]
        if p["id"] in members
    }


def mentions_macos(crate_dir: Path) -> bool:
    for f in [crate_dir / "Cargo.toml", *crate_dir.rglob("*.rs")]:
        if "target" in f.relative_to(crate_dir).parts[:1]:
            continue
        try:
            if MACOS_CFG.search(f.read_text(errors="ignore")):
                return True
        except OSError:
            pass
    return False


def reverse_closure(meta: dict, seeds: set[str]) -> set[str]:
    dirs = crate_dirs(meta)
    rdeps: dict[str, set[str]] = {n: set() for n in dirs}
    members = set(meta["workspace_members"])
    for p in meta["packages"]:
        if p["id"] not in members:
            continue
        for d in p["dependencies"]:
            if d["name"] in rdeps and d.get("path"):
                rdeps[d["name"]].add(p["name"])
    seen, todo = set(seeds), list(seeds)
    while todo:
        for r in rdeps[todo.pop()]:
            if r not in seen:
                seen.add(r)
                todo.append(r)
    return seen


def compute(meta: dict | None = None) -> tuple[set[str], set[str], dict[str, Path]]:
    meta = meta or metadata()
    dirs = crate_dirs(meta)
    macos = {n for n, d in dirs.items() if mentions_macos(d)} | (DARWIN_EXTRA & set(dirs))
    return macos, reverse_closure(meta, macos), dirs


def workflow_packages(text: str, step_marker: str) -> set[str]:
    """The `-p` crates of the (possibly folded) `run:` under a step name."""
    i = text.index(step_marker)
    j = text.find("- name:", i + len(step_marker))
    block = text[i : j if j != -1 else len(text)]
    return set(re.findall(r"-p\s+([A-Za-z0-9_-]+)", block))


def workflow_crate_paths(text: str, event: str = "pull_request") -> set[str]:
    """`crates/<dir>/**` entries of the pull_request (or push) trigger."""
    on = text[text.index("\non:") : text.index("\npermissions:")]
    if event == "pull_request":
        section = on[on.index("pull_request:") : on.index("push:")]
    else:
        section = on[on.index("push:") :]
    return set(re.findall(r"'crates/([A-Za-z0-9_-]+)/\*\*'", section))


def check(text: str, macos: set[str], trigger: set[str], dirs: dict[str, Path]) -> list[str]:
    errors = []
    for step in ("cargo clippy (macOS cfg)", "cargo nextest"):
        got = workflow_packages(text, f"- name: {step}")
        if got != macos:
            if missing := sorted(macos - got):
                errors.append(f"macos.yml `{step}` is missing -p {' -p '.join(missing)}")
            if extra := sorted(got - macos):
                errors.append(f"macos.yml `{step}` names crates with no macOS code: {', '.join(extra)}")
    want_paths = {dirs[n].name for n in trigger}
    got_paths = workflow_crate_paths(text)
    if missing := sorted(want_paths - got_paths):
        errors.append(f"macos.yml pull_request paths miss: {', '.join(f'crates/{d}/**' for d in missing)}")
    if extra := sorted(got_paths - want_paths):
        errors.append(f"macos.yml pull_request paths list crates outside the closure: {', '.join(extra)}")
    if workflow_crate_paths(text, "push") != got_paths:
        errors.append("macos.yml push paths differ from its pull_request paths")
    return errors


def main(argv: list[str]) -> int:
    macos, trigger, dirs = compute()
    if "--check" not in argv:
        print("macos crates: ", *sorted(macos))
        print("trigger paths:")
        for n in sorted(trigger, key=lambda n: dirs[n].name):
            print(f"      - 'crates/{dirs[n].name}/**'")
        return 0
    errors = check(WORKFLOW.read_text(), macos, trigger, dirs)
    for e in errors:
        print(f"::error file=.github/workflows/macos.yml::{e}")
    if errors:
        print("regenerate with: python3 scripts/macos-ci-crates.py")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
