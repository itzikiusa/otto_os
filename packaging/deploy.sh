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
# Env:    SKIP_UI=1    reuse the existing ui/dist (skip `npm run build`)
#         EMBED_UI=0   build ottod WITHOUT the SPA baked in (see step 2)
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

# `embed-ui` bakes ui/dist into the binary so the daemon serves the SPA
# same-origin. The desktop app doesn't need it (its webview loads the frontend
# from the bundle), but REMOTE access does: without it every request to the
# network listener / Cloudflare Tunnel returns the "UI not embedded"
# placeholder, i.e. sharing the UI silently stops working after a redeploy.
# Default ON — a redeploy must never take remote access away. Opt out with
# EMBED_UI=0 (smaller binary, local desktop use only).
# Build order matters: step 1 must have written ui/dist before this compiles.
echo "==> 2/6  Daemon (release ottod) + sidecar"
if [[ "${EMBED_UI:-1}" == "0" ]]; then
    echo "    (EMBED_UI=0 — daemon will NOT serve the SPA; remote/mobile access disabled)"
    cargo build --release -p ottod
else
    cargo build --release -p ottod --features embed-ui
fi
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
LABEL="com.otto.daemon"
GUI_DOMAIN="gui/$(id -u)"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

health_ok()   { curl -fsS --max-time 4 localhost:7700/api/v1/health >/dev/null 2>&1; }
poll_health() { local n="${1:-20}"; for _ in $(seq 1 "$n"); do sleep 2; health_ok && return 0; done; return 1; }

# Is the launchd job REGISTERED? `kickstart` only restarts an already-registered
# job — against an unloaded label it fails with "Could not find service", which
# used to be swallowed by `2>/dev/null || true`, leaving no daemon and no error.
daemon_loaded() { launchctl print "$GUI_DOMAIN/$LABEL" >/dev/null 2>&1; }

# Boot out, wait for the reap, then bootstrap with retries. `bootout` is async and
# ottod's shutdown is slow when it has live sessions to terminate (measured
# 0.2–0.4s idle vs 4.9–6.0s under load), so bootstrapping into that tail fails
# with "Input/output error" — wait it out rather than race it.
bootstrap_daemon() {
    [[ -f "$PLIST" ]] || { echo "    ERROR: launchd plist missing: $PLIST" >&2; return 1; }
    launchctl bootout "$GUI_DOMAIN/$LABEL" 2>/dev/null || true
    for _ in $(seq 1 120); do daemon_loaded || break; sleep 0.25; done
    local err rc
    for _ in $(seq 1 20); do
        err="$(launchctl bootstrap "$GUI_DOMAIN" "$PLIST" 2>&1)"; rc=$?
        [[ $rc -eq 0 ]] && return 0
        case "$err" in *"already bootstrapped"*) return 0 ;; esac
        sleep 0.5
    done
    echo "    ERROR: launchctl bootstrap failed: ${err:-unknown}" >&2
    return 1
}

# Restart whichever way is actually valid for the current launchd state.
restart_daemon() {
    if daemon_loaded; then
        launchctl kickstart -k "$GUI_DOMAIN/$LABEL" 2>/dev/null || true
    else
        echo "    $LABEL is not registered with launchd — bootstrapping it"
        bootstrap_daemon || true
    fi
}

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
    reason="$(launchctl print "$GUI_DOMAIN/$LABEL" 2>/dev/null | grep -i 'last exit reason' | head -1 | xargs)"
    daemon_loaded || reason="${reason:-service not registered with launchd}"
    echo "    daemon not healthy yet — ${reason:-no launchd reason}"
    if echo "$reason" | grep -qi 'CODESIGNING'; then
        heal_codesign_inode
        poll_health 15 && echo "    daemon healthy after self-heal: $(curl -s localhost:7700/api/v1/health)" \
                        || echo "    WARN: still not healthy after self-heal — check the app/logs."
    else
        # Other cause: the supervisor copied a new binary but the running process
        # is stale — or, the case that used to end in a silently dead daemon, the
        # agent isn't registered at all (the app's install raced ottod's shutdown
        # and gave up). restart_daemon picks kickstart vs bootstrap accordingly.
        echo "    restarting the launchd agent…"
        restart_daemon
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

# The SPA check. A healthy /api/v1/health says nothing about whether the daemon
# can still serve the UI — that's a separate feature (`embed-ui`), and losing it
# breaks remote/mobile sharing while every other signal stays green. Assert it
# rather than assume it.
if [[ "${EMBED_UI:-1}" != "0" ]] && health_ok; then
    if curl -fsS --max-time 4 localhost:7700/ 2>/dev/null | grep -q 'UI not embedded'; then
        echo "    WARN: daemon is serving the 'UI not embedded' placeholder —"
        echo "          remote/mobile sharing will NOT work. Was ui/dist present at compile time?"
    else
        echo "    daemon serves the SPA ✓ (remote/mobile sharing live)"
    fi
fi

# NEVER exit 0 on a dead daemon. This script used to print a WARN and then
# "done.", so a deploy that left the machine with no backend looked successful —
# the failure was only discovered later, by hand, as "the app doesn't start".
# A deploy that ends without a serving daemon is a FAILED deploy: say so, exit 1.
if ! health_ok; then
    echo
    echo "FAILED: the daemon is NOT running after this deploy." >&2
    if ! daemon_loaded; then
        echo "  '$LABEL' is not registered with launchd. Recover with:" >&2
        echo "    launchctl bootstrap $GUI_DOMAIN $PLIST" >&2
    else
        launchctl print "$GUI_DOMAIN/$LABEL" 2>/dev/null | grep -E 'state|last exit' | sed 's/^/  /' >&2
    fi
    echo "  logs: ~/Library/Logs/Otto/ottod.log.*" >&2
    exit 1
fi
echo "done."
