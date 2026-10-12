#!/bin/bash
# Disk floor for heavy builds (scripts/check.sh, packaging/deploy.sh): the Mac
# must keep at least DISK_FLOOR_GB (30) free at ALL times.
#
# Why: cargo never garbage-collects target/ and a workspace test build peaks
# at ~30-40 GB (a test binary per crate, each linking every dependency, plus
# the codegen units of in-flight links). On 2026-10-12 one filled a 228 GB
# disk to 0 bytes twice; every shell and file write on the machine then
# failed with ENOSPC until someone cleaned up from another terminal.
#
# This script NEVER deletes anything — what to remove is the user's call. It:
#   check NEED_GB [label]  refuses (exit 1) unless free − NEED_GB ≥ the floor,
#                          listing what holds the space (stale artifacts,
#                          leftover `*.rcgu.o`, target dirs) for the user;
#   watch PID [label]      polls every 3 s while PID lives and, once free
#                          space falls under the floor, stops PID's process
#                          tree (the build) and says why.
# DISK_GUARD=0 skips both.
set -euo pipefail
[[ "${DISK_GUARD:-1}" == 0 ]] && exit 0

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FLOOR="${DISK_FLOOR_GB:-30}"

free_gb() { df -Pk "$ROOT" | awk 'NR==2 {printf "%d", $4 / 1048576}'; }

report() {
    echo "  What holds space under $ROOT (nothing was deleted):" >&2
    local d
    for d in "$ROOT"/target/* "$ROOT"/apps/desktop/src-tauri/target/*; do
        if [[ -d "$d" ]]; then du -sh "$d" 2>/dev/null | sed 's/^/    /' >&2 || true; fi
    done
    local o
    # `|| true`: no target dir (a fresh clone / CI) must not trip set -e and
    # end the watchdog before it stops the build.
    o=$( { find "$ROOT"/target "$ROOT"/apps/desktop/src-tauri/target -maxdepth 3 -name '*.rcgu.o' 2>/dev/null || true; } | wc -l | tr -d ' ')
    if [[ "$o" != 0 ]]; then echo "    $o leftover *.rcgu.o (an aborted link's codegen units)" >&2; fi
    echo "  Preview stale-variant eviction: packaging/prune-target.sh (dry run)." >&2
}

# Kill PID and every descendant, children first (the cargo → rustc → linker tree).
kill_tree() {
    local p
    for p in $(pgrep -P "$1" 2>/dev/null); do kill_tree "$p"; done
    kill -TERM "$1" 2>/dev/null || true
}

case "${1:-}" in
    check)
        need="${2:?usage: disk-guard.sh check NEED_GB [label]}"
        label="${3:-build}"
        free="$(free_gb)"
        if (( free - need < FLOOR )); then
            echo "disk-guard ($label): ${free} GB free; this build needs ~${need} GB and ${FLOOR} GB must stay free — not starting." >&2
            report
            exit 1
        fi
        echo "disk-guard ($label): ${free} GB free (needs ~${need} GB, floor ${FLOOR} GB)"
        ;;
    watch)
        pid="${2:?usage: disk-guard.sh watch PID [label]}"
        label="${3:-build}"
        while kill -0 "$pid" 2>/dev/null; do
            free="$(free_gb)"
            if (( free < FLOOR )); then
                echo "" >&2
                echo "disk-guard ($label): free space fell to ${free} GB (< ${FLOOR} GB) — stopping the build." >&2
                report
                for c in $(pgrep -P "$pid" 2>/dev/null); do [[ "$c" == "$BASHPID" ]] || kill_tree "$c"; done
                exit 1
            fi
            sleep 3
        done
        ;;
    *)
        echo "usage: disk-guard.sh check NEED_GB [label] | watch PID [label]" >&2
        exit 2
        ;;
esac
