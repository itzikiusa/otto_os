#!/bin/bash
# Optional dependency-cache eviction for disk pressure; preview by default.
#
# Cargo can reuse an old hash for days while compiling another feature/host
# variant of the same crate today. Modification times do NOT identify unreachable
# artifacts. This heuristic offers a disk/rebuild-time tradeoff: it may evict
# still-usable dependencies and force their recompilation on the next build.
# Ordinary deploys preserve caches. Inspect disk usage with `du -sh target
# apps/desktop/src-tauri/target` and preview this helper before opting in.
#
# Only hashed artifacts in debug/release deps/ are candidates. Keep build/,
# .fingerprint/ and incremental/ intact: independently deleting build-script
# OUT_DIRs can leave Cargo's Fresh fingerprints pointing at missing includes.
# Bundles, receipts, top-level binaries and sources are never candidates.
# The process check is best-effort; run explicit cleanup while builds are idle.
#
# Usage:  packaging/prune-target.sh              preview, delete nothing
#         packaging/prune-target.sh --dry-run    preview, delete nothing
#         packaging/prune-target.sh --apply      evict older dependency variants
# Env:    PRUNE_WINDOW_SECS=3600  keep variants within this age of the newest (1h)
#         PRUNE=1 on either deploy entrypoint invokes --apply after building.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"

APPLY=0
case "${1:-}" in
    ""|--dry-run) ;;
    --apply) APPLY=1 ;;
    *) echo "usage: $0 [--dry-run|--apply]" >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { echo "usage: $0 [--dry-run|--apply]" >&2; exit 2; }

# Never race a live build. rustc publishes an artifact under its final name only
# at the very end, but the fingerprint and build dirs are live throughout.
# Skipping is the safe outcome — retry explicit cleanup once builds are idle.
if pgrep -x rustc >/dev/null 2>&1 || pgrep -f 'cargo (build|test|check|run|clippy)' >/dev/null 2>&1; then
    echo "    a cargo build is running — skipping cache eviction"
    exit 0
fi

ROOTS=()
for ws in "$ROOT/target" "$ROOT/apps/desktop/src-tauri/target"; do
    for profile in debug release; do
        [[ -d "$ws/$profile/deps" ]] && ROOTS+=("$ws/$profile/deps")
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
        # Keep variants within WINDOW of the newest. Older hashes may still be
        # live: this is explicit cache eviction, not reachability analysis.
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

verb = "evicted" if APPLY else "selected for eviction (dry run)"
print("    %.2f GB %s; %.2f GB retained" % (freed / 2**30, verb, kept / 2**30))
print("    older variants may still be usable; eviction can force recompilation")
PY
