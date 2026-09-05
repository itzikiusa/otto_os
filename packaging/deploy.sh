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
# Usage:  packaging/deploy.sh            full deploy
#         packaging/deploy.sh --status   show the log of the last install/verify phase
# Env:    SKIP_UI=1    reuse the existing ui/dist (skip `npm run build`)
#         EMBED_UI=0   build ottod WITHOUT the SPA baked in (see step 2)
#         DETACH=0     run steps 6–7 inline instead of under launchd (see below)
#         PRUNE=0      keep every stale build artifact (skip step 5)
#
# RESILIENCE (the 90%-interrupted problem): this script is usually run from an
# agent/shell session that ottod itself owns (a PTY child of the daemon). Step 5
# relaunches the app, the app's supervisor replaces the daemon, and ottod's
# shutdown hangs up every session PTY — which SIGKILLs the shell running THIS
# script, mid-verify, exit 137. So steps 6–7 (install → relaunch → verify) are
# handed to a one-shot launchd agent (`com.otto.deploy-finish`) that is NOT in
# ottod's process tree and therefore survives the restart. It logs to
# ~/Library/Logs/Otto/deploy-finish-<ts>.log; the foreground just tails that
# log, and if the foreground dies the phase still completes — read the outcome
# later with `packaging/deploy.sh --status`.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
APP_SRC="$ROOT/apps/desktop/src-tauri"
APP="$APP_SRC/target/release/bundle/macos/Otto.app"

cd "$ROOT"

if [[ "${1:-}" == "--status" ]]; then
    latest="$HOME/Library/Logs/Otto/deploy-finish-latest.log"
    [[ -f "$latest" ]] || { echo "no detached deploy phase has run yet"; exit 2; }
    echo "== $(readlink "$latest" || echo "$latest")"
    grep -v "^DEPLOY-FINISH EXIT=" "$latest"
    if line="$(grep "^DEPLOY-FINISH EXIT=" "$latest" | tail -1)" && [[ -n "$line" ]]; then
        rc="${line#DEPLOY-FINISH EXIT=}"
        [[ "$rc" == "0" ]] && echo "== finished OK" || echo "== FAILED (exit $rc)"
        exit "$rc"
    fi
    if launchctl print "gui/$(id -u)/com.otto.deploy-finish" 2>/dev/null | grep -q 'state = running'; then
        echo "== still running"; exit 3
    fi
    echo "== no exit marker and job not running — phase was interrupted"; exit 4
fi

# ---------------------------------------------------------------------------
# Steps 6–7: install → relaunch → verify. Runs either inline (DETACH=0) or as
# the body of the detached launchd job (OTTO_DEPLOY_PHASE=finish). Everything
# in here must be safe to run with no controlling terminal and no inherited
# environment beyond what the plist sets.
# ---------------------------------------------------------------------------
finish_phase() {
echo "==> 6/7  Install & relaunch"
# The app redeploys the daemon ONLY at start, so it must be quit first (the
# launchd agent has KeepAlive, so quitting the app doesn't stop the daemon).
# Under launchd, osascript may lack Automation rights for Otto (TCC) and quit
# silently does nothing — fall back to a plain TERM on the shell process; the
# daemon is a separate launchd job, so nothing user-facing is lost.
osascript -e 'quit app "Otto"' 2>/dev/null || true
for _ in $(seq 1 12); do pgrep -x otto-desktop >/dev/null || break; sleep 0.5; done
if pgrep -x otto-desktop >/dev/null; then
    echo "    app did not quit via AppleScript — terminating it"
    pkill -TERM -x otto-desktop || true
    for _ in $(seq 1 10); do pgrep -x otto-desktop >/dev/null || break; sleep 0.5; done
fi
rm -rf /Applications/Otto.app
ditto "$APP" /Applications/Otto.app
open /Applications/Otto.app

echo "==> 7/7  Verify (+ self-heal codesigning crash-loop)"

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
}

LOG_DIR="$HOME/Library/Logs/Otto"
FINISH_LABEL="com.otto.deploy-finish"
FINISH_PLIST="$HOME/Library/Application Support/Otto/deploy/$FINISH_LABEL.plist"
FINISH_LATEST="$LOG_DIR/deploy-finish-latest.log"
SENTINEL="DEPLOY-FINISH EXIT="

# Detached-phase entry: run steps 5–6, stamp the exit code on the last line so
# the foreground (or a later --status) can tell "still running" from "done".
if [[ "${OTTO_DEPLOY_PHASE:-}" == "finish" ]]; then
    trap 'echo "$SENTINEL$?"' EXIT   # stamped on every exit path, set -e included
    finish_phase
    exit 0
fi

echo "==> 1/7  Frontend → ui/dist"
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
echo "==> 2/7  Daemon (release ottod) + sidecar"
TRIPLE="$(rustc -vV | sed -n 's/host: //p')"
if [[ "${EMBED_UI:-1}" == "0" ]]; then
    echo "    (EMBED_UI=0 — daemon will NOT serve the SPA; remote/mobile access disabled)"
    cargo build --release -p ottod
else
    cargo build --release -p ottod --features embed-ui
fi
mkdir -p "$APP_SRC/binaries"
cp "$ROOT/target/release/ottod" "$APP_SRC/binaries/ottod-$TRIPLE"

echo "==> 3/7  Desktop app (Tauri bundle)"
( cd "$APP_SRC" && npx --yes @tauri-apps/cli@^2 build --bundles app )

echo "==> 4/7  Sign (+ ensure 'Otto Dev Signing' is trusted for code signing)"
bash "$HERE/sign.sh" "$APP" "$ROOT/target/release/ottod"

# Cargo never garbage-collects the artifacts of previous builds: each changed
# feature set / dependency graph writes a NEW <crate>-<hash> file next to the old
# one. With ~285 MB test binaries in this workspace that silently reached 43 GB
# of unreachable duplicates (vs 6.5 GB of live ones) before it was noticed as a
# full disk. Prune here — right after the last cargo invocation of this deploy,
# so the generation just built is the one the pruner keeps.
echo "==> 5/7  Prune superseded build artifacts (keep the newest generation)"
if [[ "${PRUNE:-1}" == "0" ]]; then
    echo "    (PRUNE=0 — leaving stale artifacts in place)"
else
    bash "$HERE/prune-target.sh" || echo "    WARN: prune failed — not fatal, the deploy continues."
fi

if [[ "${DETACH:-1}" == "0" ]]; then
    finish_phase
    exit $?
fi

echo "==> 6/7+7/7 handed to launchd job $FINISH_LABEL (survives the daemon restart)"
mkdir -p "$LOG_DIR" "$(dirname "$FINISH_PLIST")"
FINISH_LOG="$LOG_DIR/deploy-finish-$(date +%Y%m%d-%H%M%S).log"
: > "$FINISH_LOG"
ln -sfn "$FINISH_LOG" "$FINISH_LATEST"

# One-shot agent: RunAtLoad, no KeepAlive, PATH pinned to what the phase needs
# (launchd gives a bare PATH — `codesign`, `shasum`, `curl`, `open` all live in
# the standard dirs, but a Homebrew-only `bash` or cargo-related tool would not).
cat > "$FINISH_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>$FINISH_LABEL</string>
  <key>ProgramArguments</key><array>
    <string>/bin/bash</string><string>$HERE/deploy.sh</string>
  </array>
  <key>EnvironmentVariables</key><dict>
    <key>OTTO_DEPLOY_PHASE</key><string>finish</string>
    <key>EMBED_UI</key><string>${EMBED_UI:-1}</string>
    <key>HOME</key><string>$HOME</string>
    <key>PATH</key><string>$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin:/opt/homebrew/bin</string>
  </dict>
  <key>WorkingDirectory</key><string>$ROOT</string>
  <key>StandardOutPath</key><string>$FINISH_LOG</string>
  <key>StandardErrorPath</key><string>$FINISH_LOG</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
</dict></plist>
PLIST

# A previous run leaves the (exited) job registered; bootstrap would then fail
# with "already bootstrapped" and RunAtLoad would never fire. Clear it first.
launchctl bootout "gui/$(id -u)/$FINISH_LABEL" 2>/dev/null || true
for _ in $(seq 1 40); do launchctl print "gui/$(id -u)/$FINISH_LABEL" >/dev/null 2>&1 || break; sleep 0.25; done
launchctl bootstrap "gui/$(id -u)" "$FINISH_PLIST"
echo "    log: $FINISH_LOG"
echo "    (if this shell is killed by the restart, run: packaging/deploy.sh --status)"

# Tail the log until the sentinel, then exit with the phase's code. `tail -f`
# would outlive the sentinel; a poll keeps it simple and interruption-safe.
seen=0; tick=0
while :; do
    tick=$((tick+1))
    total="$(wc -l < "$FINISH_LOG" | tr -d ' ')"
    if (( total > seen )); then
        sed -n "$((seen+1)),${total}p" "$FINISH_LOG" | grep -v "^$SENTINEL" | sed 's/^/    /'
        seen=$total
    fi
    if line="$(grep "^$SENTINEL" "$FINISH_LOG" 2>/dev/null | tail -1)" && [[ -n "$line" ]]; then
        exit "${line#"$SENTINEL"}"
    fi
    # No sentinel and launchd says the job is no longer running → it died
    # before the trap could stamp one (e.g. a missing tool on launchd's PATH).
    if (( tick > 5 )) && ! launchctl print "gui/$(id -u)/$FINISH_LABEL" 2>/dev/null | grep -q 'state = running'; then
        sleep 1   # let a final write land
        if ! grep -q "^$SENTINEL" "$FINISH_LOG" 2>/dev/null; then
            code="$(launchctl print "gui/$(id -u)/$FINISH_LABEL" 2>/dev/null | sed -n 's/.*last exit code = //p' | head -1)"
            echo "FAILED: $FINISH_LABEL exited (code ${code:-?}) without finishing — see $FINISH_LOG" >&2
            exit 1
        fi
    fi
    sleep 1
done
