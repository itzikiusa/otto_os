#!/usr/bin/env python3
"""Repository-wide static ratchet guards shared by scripts/check.sh and CI.

Single source of truth for the fast static guards so local `scripts/check.sh`
and `.github/workflows/ci.yml` enforce the exact same rules:

  * keyed-lock     no new hand-rolled per-key `HashMap<K, Arc|Weak<Mutex<()>>>`
                   lock registries outside the shrinking allowlist; use
                   `otto_core::keyed_lock::KeyedLocks` (architecture-3-04).
  * otto-e2e       no direct `env::var(_os)?("OTTO_E2E")` reads outside
                   `crates/otto-core/src/e2e.rs` (architecture-6-01).
  * prompt-submit  no hand-rolled bracketed-paste / CR submit outside
                   `otto-sessions` and the shrinking allowlist; use
                   `SessionManager::submit_prompt` (architecture-4-01).
  * c0-bytes       no raw C0 control bytes (except TAB/LF/CR) in tracked/new
                   `*.ts`, `*.svelte`, `*.rs`, `*.js` sources
                   (maintainability-6-04).
  * ui-lib-shim    no new `export (type )?* from '../.../lib/…'` re-export
                   shims under `ui/src/modules` (maintainability-4-03, 5-01).

Usage:
    python3 scripts/repo-guards.py --all     # every guard (default)
    python3 scripts/repo-guards.py --rust    # keyed-lock, otto-e2e, prompt-submit, c0-bytes
    python3 scripts/repo-guards.py --ui      # ui-lib-shim, c0-bytes
    python3 scripts/repo-guards.py --only NAME [NAME ...]
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path
from typing import Iterable

ROOT = Path(__file__).resolve().parent.parent

KEYED_LOCK_ALLOW = re.compile(
    r"^crates/("
    r"otto-core/src/keyed_lock\.rs"
    r"|otto-aws/src/(accounts|regions)\.rs"
    r"|otto-brokers/src/service\.rs"
    r"|otto-browser/src/lib\.rs"
    r"|otto-dbviewer/src/(complete/mod|service)\.rs"
    r"|otto-k8s/src/(eks_token|monitor/cache)\.rs"
    r"|otto-sessions/src/(manager|mcp)\.rs"
    r"|otto-vault/src/engine\.rs"
    r")$"
)
KEYED_LOCK_PAT = re.compile(
    r"(?:HashMap|BTreeMap|DashMap)<[^;\n]*(?:Arc|Weak)<(?:tokio::sync::)?(?:Mutex|AsyncMutex)<\(\)>>"
)

OTTO_E2E_MAX = 0
OTTO_E2E_PAT = re.compile(r'env::var(?:_os)?\("OTTO_E2E"\)')
OTTO_E2E_SRC = re.compile(r"^crates/[^/]+/src/.*\.rs$")
OTTO_E2E_OWNER = "crates/otto-core/src/e2e.rs"

SUBMIT_ALLOW = re.compile(
    r"^crates/("
    r"otto-sessions/"
    r"|[^/]+/tests/"
    r"|otto-orchestrator/src/claude_pty\.rs"
    r"|otto-swarm/src/runtime/"
    r"|otto-server/src/swarm_host\.rs"
    r")"
)
SUBMIT_PAT = re.compile(r'bracketed_paste\(|b"\\r"|\\x1b\[200~')

C0_PAT = re.compile(rb"[\x00-\x08\x0b\x0c\x0e-\x1f]")

UI_LIB_SHIM_MAX = 2
UI_LIB_SHIM_PAT = re.compile(r"export (?:type )?\* from '(?:\.\./)+lib/")
UI_LIB_SHIM_FILE = re.compile(r"^ui/src/modules/.*\.(?:ts|js|svelte)$")

ALL_GUARDS = ("keyed-lock", "otto-e2e", "prompt-submit", "c0-bytes", "ui-lib-shim")
RUST_GUARDS = ("keyed-lock", "otto-e2e", "prompt-submit", "c0-bytes")
UI_GUARDS = ("ui-lib-shim", "c0-bytes")


def git_files(root: Path, patterns: list[str]) -> list[str]:
    """Return sorted repo-relative paths (tracked + untracked non-ignored)."""
    out = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *patterns],
        cwd=root,
    )
    paths = sorted({p.decode("utf-8", errors="surrogateescape") for p in out.split(b"\0") if p})
    return [p for p in paths if (root / p).is_file()]


def read_text_map(root: Path, paths: Iterable[str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for rel in paths:
        try:
            out[rel] = (root / rel).read_text(encoding="utf-8", errors="replace")
        except OSError:
            pass
    return out


def read_bytes_map(root: Path, paths: Iterable[str]) -> dict[str, bytes]:
    out: dict[str, bytes] = {}
    for rel in paths:
        try:
            out[rel] = (root / rel).read_bytes()
        except OSError:
            pass
    return out


def check_keyed_lock(texts: dict[str, str]) -> list[str]:
    hits = [
        rel
        for rel, text in sorted(texts.items())
        if rel.startswith("crates/")
        and rel.endswith(".rs")
        and not KEYED_LOCK_ALLOW.match(rel)
        and KEYED_LOCK_PAT.search(text)
    ]
    if not hits:
        return []
    return [
        "new hand-rolled per-key lock registry — use otto_core::keyed_lock::KeyedLocks:\n"
        + "\n".join(f"  {h}" for h in hits)
    ]


def check_otto_e2e(texts: dict[str, str]) -> tuple[int, list[str]]:
    hits: list[str] = []
    for rel, text in sorted(texts.items()):
        if not OTTO_E2E_SRC.match(rel) or rel == OTTO_E2E_OWNER:
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            if OTTO_E2E_PAT.search(line):
                hits.append(f"{rel}:{lineno}:{line}")
    if len(hits) <= OTTO_E2E_MAX:
        return len(hits), []
    return len(hits), [
        "read OTTO_E2E through otto_core::e2e::isolated(), not env::var:\n"
        + "\n".join(f"  {h}" for h in hits)
    ]


def check_prompt_submit(texts: dict[str, str]) -> list[str]:
    hits = [
        rel
        for rel, text in sorted(texts.items())
        if rel.startswith("crates/")
        and rel.endswith(".rs")
        and not SUBMIT_ALLOW.match(rel)
        and SUBMIT_PAT.search(text)
    ]
    if not hits:
        return []
    return [
        "hand-rolled paste/Enter outside otto-sessions — use SessionManager::submit_prompt:\n"
        + "\n".join(f"  {h}" for h in hits)
    ]


def check_c0_bytes(blobs: dict[str, bytes]) -> list[str]:
    hits = [rel for rel, raw in sorted(blobs.items()) if C0_PAT.search(raw)]
    if not hits:
        return []
    return ["raw C0 control bytes — write them as escapes:\n" + "\n".join(f"  {h}" for h in hits)]


def check_ui_lib_shims(texts: dict[str, str]) -> tuple[int, list[str], list[str]]:
    hits: list[str] = []
    for rel, text in sorted(texts.items()):
        if not UI_LIB_SHIM_FILE.match(rel):
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            if UI_LIB_SHIM_PAT.search(line):
                hits.append(f"{rel}:{lineno}:{line}")
    notes: list[str] = []
    if len(hits) < UI_LIB_SHIM_MAX:
        notes.append(
            f"(shims went down — lower UI_LIB_SHIM_MAX in scripts/repo-guards.py to {len(hits)})"
        )
    if len(hits) <= UI_LIB_SHIM_MAX:
        return len(hits), notes, []
    return (
        len(hits),
        notes,
        [
            "new module → lib re-export shim — import from ui/src/lib directly:\n"
            + "\n".join(f"  {h}" for h in hits)
        ],
    )


def select_guards(args: argparse.Namespace) -> list[str]:
    if args.only:
        return list(dict.fromkeys(args.only))
    selected: list[str] = []
    if args.all or (not args.rust and not args.ui):
        return list(ALL_GUARDS)
    if args.rust:
        selected.extend(RUST_GUARDS)
    if args.ui:
        selected.extend(UI_GUARDS)
    return list(dict.fromkeys(selected))


def main(argv: list[str] | None = None, root: Path = ROOT) -> int:
    parser = argparse.ArgumentParser(description="Run repository static ratchet guards.")
    parser.add_argument("--all", action="store_true", help="Run all guards (default).")
    parser.add_argument("--rust", action="store_true", help="Run Rust guards + c0-bytes.")
    parser.add_argument("--ui", action="store_true", help="Run UI guards + c0-bytes.")
    parser.add_argument(
        "--only",
        nargs="+",
        choices=ALL_GUARDS,
        help="Run only the named guard(s).",
    )
    args = parser.parse_args(argv)
    guards = select_guards(args)

    errors: list[str] = []
    crate_texts: dict[str, str] | None = None
    if any(g in guards for g in ("keyed-lock", "otto-e2e", "prompt-submit")):
        crate_texts = read_text_map(root, git_files(root, ["crates/*.rs", "crates/**/*.rs"]))

    if "keyed-lock" in guards:
        errs = check_keyed_lock(crate_texts or {})
        if errs:
            errors.extend(errs)
        else:
            print("repo-guards: keyed-lock ok")

    if "otto-e2e" in guards:
        count, errs = check_otto_e2e(crate_texts or {})
        print(f"repo-guards: otto-e2e {count}/{OTTO_E2E_MAX}")
        errors.extend(errs)

    if "prompt-submit" in guards:
        errs = check_prompt_submit(crate_texts or {})
        if errs:
            errors.extend(errs)
        else:
            print("repo-guards: prompt-submit ok")

    if "c0-bytes" in guards:
        blobs = read_bytes_map(root, git_files(root, ["*.ts", "*.svelte", "*.rs", "*.js"]))
        errs = check_c0_bytes(blobs)
        if errs:
            errors.extend(errs)
        else:
            print(f"repo-guards: c0-bytes ok ({len(blobs)} files)")

    if "ui-lib-shim" in guards:
        ui_texts = read_text_map(
            root,
            git_files(
                root,
                ["ui/src/modules/**/*.ts", "ui/src/modules/**/*.js", "ui/src/modules/**/*.svelte"],
            ),
        )
        count, notes, errs = check_ui_lib_shims(ui_texts)
        print(f"repo-guards: ui-lib-shim {count}/{UI_LIB_SHIM_MAX}")
        for note in notes:
            print(note)
        errors.extend(errs)

    for err in errors:
        print(f"::error::repo-guards: {err}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
