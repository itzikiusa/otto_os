#!/usr/bin/env python3
"""Preserve Cargo target freshness across GitHub Actions checkouts.

`Swatinem/rust-cache` intentionally strips all workspace crates (`crates/*`),
in-repo `[patch.crates-io]` crates (`vendor/*`), and compiled test binaries
before saving its cache, which forced every warm PR run to recompile ~280k LOC
of workspace Rust (`otto-server` lib + unit tests + `it` + `ottod` = 6–8 min).

Meanwhile `actions/checkout` stamps every checked-out file and directory
(including `crates/otto-state/migrations`, watched via `cargo:rerun-if-changed`,
and any crate without explicit `rerun-if-changed` whose `Precalculated`
fingerprint records exact nanosecond mtimes) with `mtime = now`.

Subcommands:
  restore       On cold cache, stamp tracked files & directories to a fixed
                pre-build epoch (`STABLE_MTIME_NS`) before Cargo runs so
                recorded mtimes are deterministic. On warm cache, compare
                tracked files & directories against `target/.ci-source-hashes.json`,
                restoring the exact saved `mtime_ns` for unchanged entries and
                stamping changed/new entries with `now` so Cargo rebuilds *only*
                affected crates.
  prepare-save  Write `target/.ci-source-hashes.json` (recording `[sha256, mtime_ns]`)
                for the current checkout and prune transient/outdated files
                under `target/` before `actions/cache/save` archives it.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

STABLE_MTIME = 1577836800  # 2020-01-01T00:00:00Z — older than any CI artifact
STABLE_MTIME_NS = STABLE_MTIME * 1_000_000_000
MAX_ARTIFACT_AGE_SECS = 7 * 24 * 3600
MANIFEST_REL = Path("target") / ".ci-source-hashes.json"


def tracked_files(root: Path) -> list[str]:
    out = subprocess.check_output(["git", "ls-files", "-z"], cwd=root)
    return sorted(p for p in out.decode("utf-8", errors="surrogateescape").split("\0") if p)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(1 << 16):
            h.update(chunk)
    return h.hexdigest()


def snapshot(root: Path) -> tuple[dict[str, str], dict[str, str]]:
    """Return `(file_hashes, dir_hashes)` for all existing tracked paths."""
    files: dict[str, str] = {}
    dir_entries: dict[str, list[str]] = {}
    for rel in tracked_files(root):
        full = root / rel
        if not full.is_file() or full.is_symlink():
            continue
        digest = sha256_file(full)
        files[rel] = digest
        cur = Path(rel)
        child_token = f"F:{cur.name}:{digest}"
        parent = cur.parent
        while str(parent) != ".":
            dir_entries.setdefault(parent.as_posix(), []).append(child_token)
            child_token = f"D:{parent.name}"
            parent = parent.parent

    dirs: dict[str, str] = {}
    # Compute bottom-up so nested directory additions/deletions propagate up.
    for d in sorted(dir_entries, key=lambda p: (-p.count("/"), p)):
        h = hashlib.sha256()
        for entry in sorted(dir_entries[d]):
            if entry.startswith("D:"):
                sub_name = entry[2:]
                sub_rel = f"{d}/{sub_name}"
                h.update(f"D:{sub_name}:{dirs.get(sub_rel, '')}\n".encode())
            else:
                h.update(f"{entry}\n".encode())
        dirs[d] = h.hexdigest()
    return files, dirs


def _unpack_saved_entry(entry: Any) -> tuple[str | None, int]:
    """Decode either v1 (`"sha256"`) or v2 (`["sha256", mtime_ns]`)."""
    if isinstance(entry, str):
        return entry, STABLE_MTIME_NS
    if isinstance(entry, list) and len(entry) == 2 and isinstance(entry[0], str):
        try:
            return entry[0], int(entry[1])
        except (TypeError, ValueError):
            return entry[0], STABLE_MTIME_NS
    return None, STABLE_MTIME_NS


def _stamp_cold_baseline(root: Path, cur_files: dict[str, str], cur_dirs: dict[str, str]) -> None:
    for rel in cur_files:
        full = root / rel
        os.utime(full, ns=(STABLE_MTIME_NS, STABLE_MTIME_NS))
    for d in sorted(cur_dirs, key=lambda p: (-p.count("/"), p)):
        full = root / d
        if full.is_dir():
            os.utime(full, ns=(STABLE_MTIME_NS, STABLE_MTIME_NS))


def cmd_restore(root: Path) -> int:
    # Integration tests using `env!("CARGO_TARGET_TMPDIR")` expect `target/tmp`
    # to exist even when Cargo skips compilation on a warm cache hit.
    (root / "target" / "tmp").mkdir(parents=True, exist_ok=True)
    cur_files, cur_dirs = snapshot(root)
    manifest_path = root / MANIFEST_REL
    if not manifest_path.is_file():
        _stamp_cold_baseline(root, cur_files, cur_dirs)
        print(
            f"ci-cargo-cache: stamped cold baseline mtimes "
            f"({len(cur_files)} files, {len(cur_dirs)} dirs)"
        )
        return 0

    try:
        saved = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        _stamp_cold_baseline(root, cur_files, cur_dirs)
        print(f"ci-cargo-cache: ignoring unreadable manifest ({exc}); stamped cold baseline")
        return 0

    saved_files: dict[str, Any] = saved.get("files", {})
    saved_dirs: dict[str, Any] = saved.get("dirs", {})
    now_ns = time.time_ns()

    unchanged_files = 0
    changed_files = 0
    for rel, digest in cur_files.items():
        full = root / rel
        saved_digest, saved_ns = _unpack_saved_entry(saved_files.get(rel))
        if saved_digest == digest:
            os.utime(full, ns=(saved_ns, saved_ns))
            unchanged_files += 1
        else:
            os.utime(full, ns=(now_ns, now_ns))
            changed_files += 1

    # Set directory mtimes deepest-first after file mtimes are settled.
    unchanged_dirs = 0
    changed_dirs = 0
    for d in sorted(cur_dirs, key=lambda p: (-p.count("/"), p)):
        full = root / d
        if not full.is_dir():
            continue
        saved_digest, saved_ns = _unpack_saved_entry(saved_dirs.get(d))
        if saved_digest == cur_dirs[d]:
            os.utime(full, ns=(saved_ns, saved_ns))
            unchanged_dirs += 1
        else:
            os.utime(full, ns=(now_ns, now_ns))
            changed_dirs += 1

    print(
        f"ci-cargo-cache: restored mtimes ({unchanged_files} unchanged files, "
        f"{changed_files} changed; {unchanged_dirs} unchanged dirs, {changed_dirs} changed)"
    )
    return 0


def prune_profile_dir(profile_dir: Path, cutoff: float) -> int:
    removed = 0
    if not profile_dir.is_dir():
        return 0
    for transient in ("incremental",):
        p = profile_dir / transient
        if p.exists():
            shutil.rmtree(p, ignore_errors=True)
            removed += 1
    for sub in ("deps", ".fingerprint", "build"):
        subdir = profile_dir / sub
        if not subdir.is_dir():
            continue
        for item in list(subdir.iterdir()):
            try:
                if item.stat().st_mtime < cutoff:
                    if item.is_dir() and not item.is_symlink():
                        shutil.rmtree(item, ignore_errors=True)
                    else:
                        item.unlink(missing_ok=True)
                    removed += 1
            except OSError:
                pass
    return removed


def cmd_prepare_save(root: Path) -> int:
    target = root / "target"
    target.mkdir(parents=True, exist_ok=True)
    cur_files, cur_dirs = snapshot(root)
    files_v2 = {
        rel: [digest, (root / rel).stat().st_mtime_ns]
        for rel, digest in cur_files.items()
    }
    dirs_v2 = {
        d: [digest, (root / d).stat().st_mtime_ns]
        for d, digest in cur_dirs.items()
        if (root / d).is_dir()
    }
    manifest_path = root / MANIFEST_REL
    manifest_path.write_text(
        json.dumps({"version": 2, "files": files_v2, "dirs": dirs_v2}, sort_keys=True),
        encoding="utf-8",
    )
    cutoff = time.time() - MAX_ARTIFACT_AGE_SECS
    removed = 0
    for profile in ("debug", "release"):
        removed += prune_profile_dir(target / profile, cutoff)
    print(
        f"ci-cargo-cache: saved manifest ({len(files_v2)} files, {len(dirs_v2)} dirs), "
        f"pruned {removed} transient/stale target entries"
    )
    return 0


def main(argv: list[str]) -> int:
    if len(argv) < 1 or argv[0] not in ("restore", "prepare-save"):
        print("usage: ci-cargo-cache.py <restore|prepare-save> [repo_root]", file=sys.stderr)
        return 2
    root = Path(argv[1]).resolve() if len(argv) > 1 else Path.cwd().resolve()
    if argv[0] == "restore":
        return cmd_restore(root)
    return cmd_prepare_save(root)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
