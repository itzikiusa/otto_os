#!/bin/bash
# Disk guard for heavy builds (scripts/check.sh, packaging/deploy.sh).
#
# Cargo never garbage-collects target/: every feature/profile variant of a
# workspace crate stays in deps/ forever, and a link killed mid-way (a full
# disk, a cancelled run) leaves its `*.rcgu.o` codegen units behind — 11 GB of
# them on 2026-10-12, when a test build filled a 228 GB disk to 0 bytes and
# the whole agent session (every shell, every file write) stopped dead.
#
# Before building, so the build itself never runs a disk dry:
#   1. always: drop stray `*.rcgu.o` in target/*/deps (rustc deletes its own
#      on success; a leftover is an aborted link's) — only while no cargo runs;
#   2. free < DISK_GUARD_LOW_GB (15): evict old dependency variants
#      (packaging/prune-target.sh --apply; may force some recompilation);
#   3. still < DISK_GUARD_MIN_GB (8): refuse, saying what to delete.
# DISK_GUARD=0 skips it. Usage: scripts/disk-guard.sh [label]
set -euo pipefail
[[ "${DISK_GUARD:-1}" == 0 ]] && exit 0

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOW="${DISK_GUARD_LOW_GB:-15}"
MIN="${DISK_GUARD_MIN_GB:-8}"
label="${1:-build}"

free_gb() { df -Pk "$ROOT" | awk 'NR==2 {printf "%d", $4 / 1048576}'; }
building() { pgrep -x rustc >/dev/null 2>&1 || pgrep -f 'cargo (build|test|check|run|clippy|nextest)' >/dev/null 2>&1; }

if ! building; then
    for deps in "$ROOT"/target/*/deps "$ROOT"/apps/desktop/src-tauri/target/*/deps; do
        [[ -d "$deps" ]] && find "$deps" -maxdepth 1 -name '*.rcgu.o' -delete 2>/dev/null || true
    done
fi

free="$(free_gb)"
if (( free < LOW )); then
    echo "disk-guard ($label): ${free} GB free (< ${LOW} GB) — evicting old cargo artifacts"
    bash "$ROOT/packaging/prune-target.sh" --apply || true
    free="$(free_gb)"
fi
if (( free < MIN )); then
    echo "disk-guard ($label): only ${free} GB free (< ${MIN} GB) — refusing to start a build that would fill the disk." >&2
    echo "  Free space first, e.g. 'rm -rf $ROOT/target/debug' (rebuilt on demand), or set DISK_GUARD=0 to override." >&2
    exit 1
fi
echo "disk-guard ($label): ${free} GB free"
