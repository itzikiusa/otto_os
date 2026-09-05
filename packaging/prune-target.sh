#!/bin/bash
# Drop superseded build generations from the cargo target dirs.
#
# WHY: cargo never garbage-collects. Every build whose feature set, profile or
# dependency graph differs writes a NEW <crate>-<hash> artifact beside the old
# one and leaves the old one forever. In this workspace a single `ottod` test
# binary is ~285 MB and `libotto_server.rlib` ~490 MB, so a day of agent-driven
# build/test cycles is measured in tens of gigabytes: when this script was
# written the tree held 43 GB of unreachable duplicates against 6.5 GB of live
# artifacts — 87% garbage, and the reason the disk filled.
#
# WHAT "SUPERSEDED" MEANS (and why it is not simply "keep the newest hash"):
# cargo keeps SEVERAL hashes of the same crate live simultaneously — one per
# feature unification, plus separate host/build-script builds. A real example
# from this repo, all three reachable from one build:
#     libfutures_core-6b65906c19f3226d.rlib
#     libfutures_core-717c7bf216492267.rmeta
#     libfutures_core-a1d26ce20fab25d6.rlib
# Keeping only the newest would delete two LIVE artifacts on every run and make
# the next build recompile them — a treadmill, not a cleanup. So instead this
# keeps a whole GENERATION: within each <crate> family it finds the newest
# artifact and keeps every variant written within WINDOW seconds of it (the
# co-live variants of one build always land within one build's duration), then
# deletes the older generations. Superseded copies go; live ones stay.
#
# SAFETY: touches only cargo's own regenerable output under target/ — never
# sources, never target/release/ottod, never target/release/bundle, never
# anything outside a target/ dir. Worst case a pruned artifact is rebuilt. It
# also refuses to run while a build is in flight, so it cannot delete an
# artifact out from under rustc.
#
# Usage:  packaging/prune-target.sh              prune (default)
#         packaging/prune-target.sh --dry-run    report what would go, delete nothing
# Env:    PRUNE_WINDOW_SECS=3600   how wide one "generation" is (default 1h)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"

APPLY=1
[[ "${1:-}" == "--dry-run" ]] && APPLY=0

# Never race a live build. rustc publishes an artifact under its final name only
# at the very end, but the fingerprint and build dirs are live throughout.
# Skipping is the safe outcome — the next deploy prunes instead.
if pgrep -x rustc >/dev/null 2>&1 || pgrep -f 'cargo (build|test|check|run|clippy)' >/dev/null 2>&1; then
    echo "    a cargo build is running — skipping the prune (it will run on the next deploy)"
    exit 0
fi

ROOTS=()
for ws in "$ROOT/target" "$ROOT/apps/desktop/src-tauri/target"; do
    for profile in debug release; do
        for sub in deps build .fingerprint incremental; do
            [[ -d "$ws/$profile/$sub" ]] && ROOTS+=("$ws/$profile/$sub")
        done
    done
done
[[ ${#ROOTS[@]} -gt 0 ]] || { echo "    no target dirs to prune"; exit 0; }

APPLY="$APPLY" WINDOW="${PRUNE_WINDOW_SECS:-3600}" python3 - "${ROOTS[@]}" <<'PY'
import collections, os, re, shutil, sys

APPLY  = os.environ.get("APPLY") == "1"
WINDOW = float(os.environ.get("WINDOW", 3600))
# <crate>-<hash>[.ext]. Cargo uses 16 hex chars; accept 7+ to stay future-proof.
HASHED = re.compile(r"^(.*)-([0-9a-f]{7,})(\..*)?$")

def weigh(path):
    """(bytes, newest mtime, is_dir) for a file or a whole directory tree."""
    if os.path.isdir(path) and not os.path.islink(path):
        size, mtime = 0, os.path.getmtime(path)
        for dirpath, _dirs, files in os.walk(path):
            for name in files:
                try:
                    st = os.lstat(os.path.join(dirpath, name))
                except OSError:
                    continue
                size += st.st_size
                mtime = max(mtime, st.st_mtime)
        return size, mtime, True
    st = os.lstat(path)
    return st.st_size, st.st_mtime, False

freed = kept = 0
for root in sys.argv[1:]:
    # crate family -> hash -> [(path, size, mtime, is_dir), ...]
    families = collections.defaultdict(lambda: collections.defaultdict(list))
    try:
        names = os.listdir(root)
    except OSError:
        continue
    for name in names:
        match = HASHED.match(name)
        if not match:
            continue                      # unhashed → not a versioned artifact, leave it alone
        crate, digest, _ext = match.groups()
        path = os.path.join(root, name)
        try:
            size, mtime, is_dir = weigh(path)
        except OSError:
            continue
        families[crate][digest].append((path, size, mtime, is_dir))

    for crate, by_hash in families.items():
        if len(by_hash) < 2:
            kept += sum(s for e in by_hash.values() for _, s, _, _ in e)
            continue
        # The current generation: every variant last written within WINDOW of the
        # newest one. Co-live variants of a single build fall inside it; earlier
        # builds' leftovers do not.
        newest = max(max(m for _, _, m, _ in e) for e in by_hash.values())
        for digest, entries in by_hash.items():
            current = max(m for _, _, m, _ in entries) >= newest - WINDOW
            for path, size, _mtime, is_dir in entries:
                if current:
                    kept += size
                    continue
                freed += size
                if APPLY:
                    try:
                        shutil.rmtree(path) if is_dir else os.remove(path)
                    except OSError as exc:
                        print("    ! could not remove %s: %s" % (path, exc))

verb = "reclaimed" if APPLY else "reclaimable (dry run)"
print("    %.2f GB %s; %.2f GB of current artifacts kept" % (freed / 2**30, verb, kept / 2**30))
PY
