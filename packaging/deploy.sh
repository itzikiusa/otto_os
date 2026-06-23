#!/bin/bash
# One-command local deploy of the Otto desktop app (macOS only):
#   rebuild frontend + daemon  →  bundle Tauri app  →  sign (+ ensure cert trust)
#   →  replace /Applications/Otto.app  →  relaunch  →  verify.
#
# There is no Makefile; this just chains the documented steps in docs/RELEASE.md
# so a redeploy is a single command and the signing cert is always trusted (no
# recurring keychain password prompt — see packaging/README.md).
#
# Plug-and-play: step 6 auto-detects and self-heals the OS_REASON_CODESIGNING
# "Invalid Page" daemon crash-loop (see the big comment on heal_codesign_inode
# below) so a fresh checkout deploys cleanly without manual intervention.
#
# Usage:  packaging/deploy.sh
# Env:    SKIP_UI=1   reuse the existing ui/dist (skip `npm run build`)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
APP_SRC="$ROOT/apps/desktop/src-tauri"
APP="$APP_SRC/target/release/bundle/macos/Otto.app"
TRIPLE="$(rustc -vV | sed -n 's/host: //p')"

cd "$ROOT"

echo "==> 1/6  Frontend → ui/dist"
if [[ "${SKIP_UI:-}" == "1" && -f ui/dist/index.html ]]; then
    echo "    (SKIP_UI=1 — reusing existing ui/dist)"
else
    ( cd ui && npm run build )
fi

echo "==> 2/6  Daemon (release ottod) + sidecar"
cargo build --release -p ottod
mkdir -p "$APP_SRC/binaries"
cp "$ROOT/target/release/ottod" "$APP_SRC/binaries/ottod-$TRIPLE"

echo "==> 3/6  Desktop app (Tauri bundle)"
( cd "$APP_SRC" && npx --yes @tauri-apps/cli@^2 build --bundles app )

echo "==> 4/6  Sign (+ ensure 'Otto Dev Signing' is trusted for code signing)"
bash "$HERE/sign.sh" "$APP" "$ROOT/target/release/ottod"

echo "==> 5/6  Install & relaunch"
# The app redeploys the daemon ONLY at start, so it must be quit first (the
# launchd agent has KeepAlive, so quitting the app doesn't stop the daemon).
osascript -e 'quit app "Otto"' 2>/dev/null || true
sleep 3
rm -rf /Applications/Otto.app
ditto "$APP" /Applications/Otto.app
open /Applications/Otto.app

echo "==> 6/6  Verify (+ self-heal codesigning crash-loop)"

dep="$HOME/Library/Application Support/Otto/bin/ottod"
side="$APP/Contents/MacOS/ottod"

health_ok()   { curl -fsS --max-time 4 localhost:7700/api/v1/health >/dev/null 2>&1; }
poll_health() { local n="${1:-20}"; for _ in $(seq 1 "$n"); do sleep 2; health_ok && return 0; done; return 1; }

# ---------------------------------------------------------------------------
# Self-heal for the "Invalid Page" / OS_REASON_CODESIGNING daemon crash-loop.
#
# THE GOTCHA (debugged the hard way): the app self-deploys ottod by overwriting
# ~/Library/Application Support/Otto/bin/ottod *in place*. If the launchd agent
# (KeepAlive) already had that binary mapped and running, overwriting the file
# mid-flight invalidates the mapped code pages and macOS SIGKILLs the process
# with "Invalid Page". Crucially, the kernel caches code-signing validity PER
# INODE — so that inode is then rejected on EVERY relaunch (a permanent
# crash-loop, runs=N climbing, `last exit reason = OS_REASON_CODESIGNING`),
# even though the bytes on disk are validly signed: `codesign --verify` passes,
# and a byte-identical copy at a different path runs fine.
#
# THE FIX: give the deployed binary a FRESH INODE — atomic rename of a clean
# copy of the signed bundle sidecar. New inode = fresh code-signing evaluation
# = the validly-signed bytes run. Then kickstart the launchd agent.
# (Root cause to fix in-code one day: the self-deploy should atomic-rename a new
#  inode instead of overwriting the running binary in place.)
# ---------------------------------------------------------------------------
heal_codesign_inode() {
    [[ -f "$dep" && -f "$side" ]] || return 1
    echo "    self-heal: ottod is in an OS_REASON_CODESIGNING crash-loop"
    echo "    → replacing the deployed binary with a fresh inode (clears the poisoned per-inode CS cache)…"
    cp -f "$side" "$dep.fresh"
    codesign --verify "$dep.fresh" 2>/dev/null || bash "$HERE/sign.sh" "$APP" >/dev/null 2>&1 || true
    mv -f "$dep.fresh" "$dep"          # atomic replace → NEW inode
    launchctl kickstart -k "gui/$(id -u)/com.otto.daemon" 2>/dev/null || true
}

if poll_health 20; then
    echo "    daemon healthy: $(curl -s localhost:7700/api/v1/health)"
else
    reason="$(launchctl print "gui/$(id -u)/com.otto.daemon" 2>/dev/null | grep -i 'last exit reason' | head -1 | xargs)"
    echo "    daemon not healthy yet — ${reason:-no launchd reason}"
    if echo "$reason" | grep -qi 'CODESIGNING'; then
        heal_codesign_inode
        poll_health 15 && echo "    daemon healthy after self-heal: $(curl -s localhost:7700/api/v1/health)" \
                        || echo "    WARN: still not healthy after self-heal — check the app/logs."
    else
        # Other cause: supervisor copied a new binary but the running process is
        # stale — force a restart in one op.
        echo "    kickstarting the launchd agent…"
        launchctl kickstart -k "gui/$(id -u)/com.otto.daemon" 2>/dev/null || true
        poll_health 10 && echo "    daemon healthy: $(curl -s localhost:7700/api/v1/health)" \
                        || echo "    WARN: daemon not healthy — check the app/logs."
    fi
fi

# Final reconcile: the deployed binary should match the freshly built bundle.
if [[ -f "$dep" ]]; then
    a="$(shasum -a 256 "$dep" | awk '{print $1}')"
    b="$(shasum -a 256 "$side" | awk '{print $1}')"
    [[ "$a" == "$b" ]] && echo "    deployed daemon matches the new bundle ✓" \
                       || echo "    note: deployed daemon differs from bundle (the app self-deploys at start)."
fi
echo "done."
