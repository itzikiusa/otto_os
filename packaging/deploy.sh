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
# Env:    SKIP_UI=1    rejected: a source receipt requires a fresh UI build
#         EMBED_UI=0   build ottod WITHOUT the SPA baked in (see step 2)
#         DETACH=0     run steps 6–7 inline instead of under launchd (see below)
#         PRUNE=0      keep every stale build artifact (skip step 5)
#         BUILD_ONLY=1 build/sign and write a receipt without installing
#         OTTO_DEPLOY_PHASE=queue-finish queue a previously receipted build
#         FINISH_DELAY_SECONDS=15 delay detached install (integer 0–60)
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
INSTALLED_APP="/Applications/Otto.app"
BUILD_RECEIPT="$APP_SRC/target/release/deploy-build.receipt"

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
# These helpers are also exercised by packaging/tests/test_deploy.py with all
# process and network commands replaced; the tests never install an app.
deployed_daemon_path() { printf '%s/Library/Application Support/Otto/bin/ottod\n' "$HOME"; }
sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
fail_verify() { echo "FAILED: $*" >&2; return 1; }
health_ok() {
    local body
    body="$(curl -fsS --max-time 4 http://127.0.0.1:7700/api/v1/health 2>/dev/null)" || return 1
    [[ "$(printf '%s' "$body" | tr -d '[:space:]')" == '{"ok":true}' ]]
}
daemon_pid() {
    launchctl print "gui/$(id -u)/com.otto.daemon" 2>/dev/null |
        sed -n 's/^[[:space:]]*pid = \([0-9][0-9]*\)$/\1/p' | head -1
}
process_path() { ps -p "$1" -o comm= 2>/dev/null | sed 's/^[[:space:]]*//'; }
app_pid() {
    local candidate
    for candidate in $(pgrep -x otto-desktop || true); do
        if [[ "$(process_path "$candidate")" == "$INSTALLED_APP/Contents/MacOS/otto-desktop" ]]; then
            echo "$candidate"; return 0
        fi
    done
    return 1
}

# Code signatures seal bundle resources; executable hashes pin the signed build.
# Refuse a dirty source tree so HEAD identifies the complete build input.
build_manifest() {
    local app_hash daemon_hash commit
    [[ -z "$(git -C "$ROOT" status --porcelain --untracked-files=normal)" ]] ||
        { fail_verify 'source tree changed or is dirty; rebuild from the clean merged commit'; return 1; }
    codesign --verify --deep --strict "$APP" || return 1
    codesign --verify --strict "$APP/Contents/MacOS/ottod" || return 1
    commit="$(git -C "$ROOT" rev-parse HEAD)" || return 1
    app_hash="$(sha256 "$APP/Contents/MacOS/otto-desktop")" || return 1
    daemon_hash="$(sha256 "$APP/Contents/MacOS/ottod")" || return 1
    printf 'commit=%s\napp_sha256=%s\ndaemon_sha256=%s\nembed_ui=%s\n' \
        "$commit" "$app_hash" "$daemon_hash" "${EMBED_UI:-1}"
}
validate_build_receipt() {
    local actual expected
    [[ -f "$BUILD_RECEIPT" ]] || { fail_verify 'no successful build receipt; run BUILD_ONLY=1 packaging/deploy.sh first'; return 1; }
    actual="$(build_manifest)" || return 1
    expected="$(cat "$BUILD_RECEIPT")"
    [[ "$actual" == "$expected" ]] || { fail_verify 'source or signed artifacts changed since build; rebuild before queuing'; return 1; }
}

validate_delay() {
    local delay="${FINISH_DELAY_SECONDS:-0}"
    case "$delay" in
        ""|*[!0-9]*) fail_verify 'FINISH_DELAY_SECONDS must be 0–60'; return 1 ;;
    esac
    [[ ${#delay} -le 2 ]] && (( 10#$delay <= 60 )) ||
        { fail_verify 'FINISH_DELAY_SECONDS must be 0–60'; return 1; }
    FINISH_DELAY_SECONDS=$((10#$delay))
}

verify_install() {
    local installed_side="$INSTALLED_APP/Contents/MacOS/ottod" current_pid current_app_pid ready=0 html
    codesign --verify --deep --strict "$INSTALLED_APP" || return 1
    codesign --verify --strict "$installed_side" || return 1
    diff -qr "$APP" "$INSTALLED_APP" || { fail_verify 'installed app differs from the signed build'; return 1; }
    # Startup can briefly serve the old healthy daemon while the supervisor is
    # still copying/restarting. Reconcile bytes, process identity and health together.
    for _ in $(seq 1 30); do
        current_pid="$(daemon_pid)" || current_pid=""
        current_app_pid="$(app_pid)" || current_app_pid=""
        if [[ -n "$current_pid" && -n "$current_app_pid" && -f "$dep" ]] &&
            [[ "$(process_path "$current_pid")" == "$dep" ]] &&
            cmp -s "$side" "$dep" &&
            { [[ "$old_hash" == "$(sha256 "$side")" ]] || [[ "$current_pid" != "$old_pid" ]]; } && health_ok; then
            ready=1; break
        fi
        sleep 2
    done
    [[ "$ready" == 1 ]] || { fail_verify 'expected app/daemon processes, deployed hash and healthy API did not converge'; return 1; }
    codesign --verify --strict "$dep" || return 1
    if [[ "${EMBED_UI:-1}" != 0 ]]; then
        html="$(curl -fsS --max-time 4 http://127.0.0.1:7700/ 2>/dev/null)" ||
            { fail_verify 'embedded UI fetch failed'; return 1; }
        [[ "$html" == *'<div id="app"></div>'* && "$html" == *'<script type="module"'* && "$html" == *'/assets/'* && "$html" != *'UI not embedded'* ]] ||
            { fail_verify 'daemon did not serve the embedded Otto SPA'; return 1; }
    fi
    printf 'DEPLOY-RECEIPT commit=%s app_sha256=%s daemon_sha256=%s app_pid=%s daemon_pid=%s health=ok embedded_ui=%s\n' \
        "$(git -C "$ROOT" rev-parse HEAD)" "$(sha256 "$INSTALLED_APP/Contents/MacOS/otto-desktop")" \
        "$(sha256 "$dep")" "$current_app_pid" "$current_pid" "${EMBED_UI:-1}"
}

finish_phase() {
local old_pid old_hash
old_pid="$(daemon_pid)" || old_pid=""
dep="$(deployed_daemon_path)"
old_hash=""
[[ ! -f "$dep" ]] || old_hash="$(sha256 "$dep")"
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
if pgrep -x otto-desktop >/dev/null; then fail_verify "old app did not exit"; return 1; fi
rm -rf "$INSTALLED_APP"
ditto "$APP" "$INSTALLED_APP"
open "$INSTALLED_APP"

echo "==> 7/7  Verify (+ self-heal codesigning crash-loop)"

dep="$(deployed_daemon_path)"
side="$APP/Contents/MacOS/ottod"
LABEL="com.otto.daemon"
GUI_DOMAIN="gui/$(id -u)"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

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
# THE GOTCHA (older installed app versions): the app self-deployed ottod by overwriting
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
# Current supervisor versions already use atomic rename; retain this recovery
# for previously poisoned inodes left by an older installed app.
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

verify_install || return 1
echo "done."
}

LOG_DIR="$HOME/Library/Logs/Otto"
FINISH_LABEL="com.otto.deploy-finish"
FINISH_PLIST="$HOME/Library/Application Support/Otto/deploy/$FINISH_LABEL.plist"
FINISH_LATEST="$LOG_DIR/deploy-finish-latest.log"
SENTINEL="DEPLOY-FINISH EXIT="

validate_delay
case "${OTTO_DEPLOY_PHASE:-}" in
    ""|finish|queue-finish) ;;
    *) fail_verify "unknown OTTO_DEPLOY_PHASE"; exit 2 ;;
esac

# Detached-phase entry: run steps 5–6, stamp the exit code on the last line so
# the foreground (or a later --status) can tell "still running" from "done".
if [[ "${OTTO_DEPLOY_PHASE:-}" == "finish" ]]; then
    trap 'echo "$SENTINEL$?"' EXIT   # stamped on every exit path, set -e included
    validate_build_receipt
    sleep "${FINISH_DELAY_SECONDS:-0}"
    validate_build_receipt
    finish_phase
    exit 0
fi

if [[ "${OTTO_DEPLOY_PHASE:-}" != "queue-finish" ]]; then
[[ -z "$(git status --porcelain --untracked-files=normal)" ]] || { fail_verify "build requires a clean source tree"; exit 1; }
BUILD_SOURCE_COMMIT="$(git rev-parse HEAD)"
[[ "${SKIP_UI:-0}" != 1 ]] || { fail_verify "receipted deployment requires a fresh UI build; unset SKIP_UI"; exit 1; }
echo "==> 1/7  Frontend → ui/dist"
( cd ui && npm run build )

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

[[ "$(git rev-parse HEAD)" == "$BUILD_SOURCE_COMMIT" ]] || { fail_verify "source commit changed during build"; exit 1; }
build_manifest > "$BUILD_RECEIPT.tmp"
mv "$BUILD_RECEIPT.tmp" "$BUILD_RECEIPT"
echo "BUILD-RECEIPT $BUILD_RECEIPT"
cat "$BUILD_RECEIPT"
if [[ "${BUILD_ONLY:-0}" == "1" ]]; then exit 0; fi
fi
validate_build_receipt

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
    <key>FINISH_DELAY_SECONDS</key><string>${FINISH_DELAY_SECONDS:-0}</string>
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
if [[ "${OTTO_DEPLOY_PHASE:-}" == "queue-finish" ]]; then
    echo "DEPLOY-QUEUED delay_seconds=$FINISH_DELAY_SECONDS log=$FINISH_LOG"
    cat "$BUILD_RECEIPT"
    exit 0
fi

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
