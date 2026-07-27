#!/bin/bash
#
# deploy.sh — one-shot: rebuild → resign → reinstall → replace the running Otto app.
#
# Does the entire cycle end-to-end and leaves the app running on the NEW build, so
# you never have to quit / replace / relaunch anything yourself:
#
#   1. build UI (ui/dist)            6. quit the running app + bootout the daemon
#   2. build ottod (embed-ui)        7. atomic-swap the staged app into place
#   3. tauri build (Otto.app)        8. sync daemon bin == bundle sidecar (byte-identical)
#   4. codesign (Otto Dev Signing)   9. bootstrap daemon + reopen app + verify
#   5. stage signed app in /Applications  (hidden sibling, verified)
#
# WHY self-detach (hard-won): a terminal / agent session runs as a CHILD of the
# ottod daemon, so the moment we bootout the daemon (step 6) launchd reaps this
# script too — worst case mid-swap, leaving a broken /Applications/Otto.app. So
# when we detect we're running under ottod we re-exec detached (nohup, own
# session, output → ~/Library/Logs/Otto/deploy-*.log) and let the parent die.
#
# WHY atomic swap (hard-won): stage the signed bundle to a hidden sibling
# (/Applications/.Otto.app.staging.$$), verify it, and only THEN rename() it into
# place (mv old → .Otto.app.old.$$, mv staging → Otto.app) — both renames sit on
# the SAME volume so each is atomic. A killed deploy can never leave a half-copied
# app: worst case the previous app survives as .old.$$ (recoverable), reaped only
# after every post-deploy verification passes.
#
# WHY the byte-identical dance (hard-won): the desktop supervisor copies the bundled
# sidecar (/Applications/Otto.app/Contents/MacOS/ottod) → installed bin/ottod whenever
# the two DIFFER, then restarts the daemon — racing launchd KeepAlive into an
# OS_REASON_CODESIGNING throttle (daemon down even though the binary verifies). Fix:
# sign the app ONCE, then `ditto` that SAME signed sidecar into bin/ottod so they are
# byte-identical (shasum matches) → the supervisor's byte-compare skips the copy → no
# relaunch clobber, and the app's signature seal stays intact (we copy FROM the bundle,
# never overwrite into it).
#
# WHY the exit trap + lock (hard-won): deploys DO die mid-flight (reaped shells,
# Ctrl-C, build crashes) — and a deploy that stops between "daemon booted out"
# and "daemon bootstrapped" leaves the machine with NO daemon and no explanation.
# So: a lock dir refuses concurrent deploys, every phase is checkpointed to a
# status file (~/Library/Logs/Otto/deploy-last.status), and an EXIT trap rolls
# back whatever teardown had happened (restore the old app, re-bootstrap the
# daemon) before reporting FAILED-at-phase-N. `./deploy.sh --status` shows the
# last outcome + whether a deploy is running right now.
#
# Usage:
#   ./deploy.sh                 # full rebuild + redeploy (default)
#   ./deploy.sh --dmg           # also produce a DMG alongside the .app
#   ./deploy.sh --force-ci      # force `npm ci` even if node_modules looks fresh
#   ./deploy.sh --status        # last deploy outcome + live progress, then exit
#   ./deploy.sh -h | --help
#
set -uo pipefail

# ---- config ---------------------------------------------------------------
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CERT_NAME="Otto Dev Signing"
DAEMON_LABEL="com.otto.daemon"
PLIST="$HOME/Library/LaunchAgents/${DAEMON_LABEL}.plist"
INSTALL_DIR="$HOME/Library/Application Support/Otto/bin"
INSTALLED_OTTOD="$INSTALL_DIR/ottod"
APP_DST="/Applications/Otto.app"
HEALTH_URL="http://127.0.0.1:7700/api/v1/health"
SERVE_URL="http://127.0.0.1:7700/"
TRIPLE="$(rustc -vV 2>/dev/null | sed -n 's/host: //p')"; TRIPLE="${TRIPLE:-aarch64-apple-darwin}"
SIDECAR_SRC="$ROOT/apps/desktop/src-tauri/binaries/ottod-${TRIPLE}"
BUILT_APP="$ROOT/apps/desktop/src-tauri/target/release/bundle/macos/Otto.app"
KEEP_BACKUPS=3
LOG_DIR="$HOME/Library/Logs/Otto"
STATUS_FILE="$LOG_DIR/deploy-last.status"
LOCK_DIR="$LOG_DIR/deploy.lock"

WANT_DMG=0
FORCE_CI=0
for arg in "$@"; do
    case "$arg" in
        --dmg)       WANT_DMG=1 ;;
        --force-ci)  FORCE_CI=1 ;;
        --status)
            if [[ -d "$LOCK_DIR" ]] && kill -0 "$(cat "$LOCK_DIR/pid" 2>/dev/null)" 2>/dev/null; then
                echo "deploy RUNNING (pid $(cat "$LOCK_DIR/pid"), log $(cat "$LOCK_DIR/log" 2>/dev/null || echo '?'))"
            else
                echo "no deploy running"
            fi
            [[ -f "$STATUS_FILE" ]] && { echo "last deploy:"; cat "$STATUS_FILE"; }
            exit 0 ;;
        -h|--help)
            sed -n '2,51p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *) echo "unknown flag: $arg (try --help)" >&2; exit 2 ;;
    esac
done

# ---- pretty output --------------------------------------------------------
BOLD=$'\033[1m'; DIM=$'\033[2m'; GRN=$'\033[32m'; RED=$'\033[31m'; YEL=$'\033[33m'; RST=$'\033[0m'
START_TS=$(date +%s)
PHASE="preflight"
step() { PHASE="$*"; checkpoint "RUNNING"; echo; echo "${BOLD}▸ [$(date +%H:%M:%S)] $*${RST}"; }
ok()   { echo "  ${GRN}✓${RST} $*"; }
warn() { echo "  ${YEL}!${RST} $*"; }
die()  { echo; echo "${RED}✗ FAILED:${RST} $*" >&2; echo "${DIM}  (nothing irreversible past the build phase unless noted above)${RST}" >&2; exit 1; }
run()  { echo "  ${DIM}\$ $*${RST}"; "$@"; }

# Checkpoint every phase transition so `--status` (and a human tailing the log)
# can always tell where a deploy is — or where a dead one stopped.
checkpoint() {
    mkdir -p "$LOG_DIR"
    printf '%s  pid=%s  phase=%s  %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" $$ "$PHASE" "$1" >"$STATUS_FILE" 2>/dev/null || true
}

# Run a command with a hard time limit (macOS has no GNU `timeout`). Anything
# that talks to another process (osascript quit, open) can hang indefinitely —
# e.g. on an unsaved-changes dialog — and silently wedge the whole deploy.
bounded() {
    local secs="$1"; shift
    "$@" & local cmd_pid=$!
    ( sleep "$secs"; kill -9 "$cmd_pid" 2>/dev/null ) & local killer_pid=$!
    wait "$cmd_pid" 2>/dev/null; local rc=$?
    kill "$killer_pid" 2>/dev/null; wait "$killer_pid" 2>/dev/null
    return $rc
}

# ---- self-detach if running under the daemon we're about to restart -------
# A terminal / agent session lives as a CHILD of the ottod daemon. When phase 6
# boots the daemon out, launchd reaps that whole process tree — INCLUDING this
# script — mid-deploy (worst case between the two swap renames → a broken
# /Applications/Otto.app). So walk our own process ancestry; if any ancestor is
# ottod, re-exec ourselves fully detached (nohup, own session, output → a log)
# and let the doomed parent shell exit cleanly. The re-exec sets
# OTTO_DEPLOY_DETACHED so the detached copy runs straight through.
under_ottod() {
    local pid=$$ comm
    while [[ -n "$pid" && "$pid" != 0 && "$pid" != 1 ]]; do
        comm="$(ps -o comm= -p "$pid" 2>/dev/null)"
        case "$comm" in *ottod*) return 0 ;; esac
        pid="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')"
    done
    return 1
}
if [[ -z "${OTTO_DEPLOY_DETACHED:-}" ]] && under_ottod; then
    LOG_DIR="$HOME/Library/Logs/Otto"
    mkdir -p "$LOG_DIR"
    LOG="$LOG_DIR/deploy-$(date +%Y%m%d-%H%M%S).log"
    echo
    echo "${BOLD}${YEL}! RE-LAUNCHING DETACHED${RST} — this deploy is running under an Otto session,"
    echo "  whose terminal is a child of the ottod daemon. Restarting the daemon"
    echo "  mid-deploy would kill this script (worst case: a half-swapped Otto.app)."
    echo "  Re-launching detached so the deploy survives the daemon restart."
    echo "  ${DIM}log:${RST}    $LOG"
    echo "  ${DIM}follow:${RST} tail -f \"$LOG\""
    # Re-exec by ABSOLUTE path: `bash deploy.sh` leaves $0 slash-less, and nohup
    # PATH-looks-up bare names — the detached copy would die with "No such file
    # or directory" before doing anything (and the parent exits 0, silently).
    SELF="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
    # nohup+disown is NOT enough: the re-exec'd copy still lives in the
    # com.otto.daemon launchd cgroup, and when phase 6 boots the daemon out,
    # launchd sweeps the whole group — the "detached" deploy dies with SIGTERM
    # (exit 143) at exactly that phase (observed repeatedly; survival was a
    # race). Escape the cgroup by handing the copy to launchd as a ONE-SHOT
    # job: unique per-run label, NO KeepAlive (can never loop — see the
    # com.otto.deploy.* restart-loop incident), plist in a temp dir (never
    # ~/Library/LaunchAgents) and deleted right after bootstrap, so nothing
    # persists past this run, let alone to the next login.
    RUN_LABEL="com.otto.deploy-once.$(date +%s).$$"
    PLIST="$(mktemp -d)/$RUN_LABEL.plist"
    cat >"$PLIST" <<PLISTEOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>$RUN_LABEL</string>
  <key>RunAtLoad</key><true/>
  <key>ProgramArguments</key><array>
    <string>/bin/bash</string>
    <string>$SELF</string>
  </array>
  <key>EnvironmentVariables</key><dict>
    <key>OTTO_DEPLOY_DETACHED</key><string>1</string>
    <key>OTTO_DEPLOY_LOG</key><string>$LOG</string>
    <key>PATH</key><string>$PATH</string>
    <key>HOME</key><string>$HOME</string>
  </dict>
  <key>StandardOutPath</key><string>$LOG</string>
  <key>StandardErrorPath</key><string>$LOG</string>
</dict></plist>
PLISTEOF
    if [[ $# -eq 0 ]] && launchctl bootstrap "gui/$(id -u)" "$PLIST" 2>/dev/null; then
        rm -f "$PLIST"    # job is running; the file is not needed again
    else
        # Fallback (bootstrap can fail on odd launchd states): the old racy
        # nohup detach — better than not deploying at all.
        rm -f "$PLIST"
        OTTO_DEPLOY_DETACHED=1 OTTO_DEPLOY_LOG="$LOG" nohup bash "$SELF" "$@" >"$LOG" 2>&1 </dev/null & disown
    fi
    exit 0
fi

# ---- single-deploy lock + phase-aware rollback trap -----------------------
# One deploy at a time: two concurrent runs (a terminal + an agent, or a re-run
# while a detached copy is still going) interleave bootout/bootstrap/mv and
# reliably wreck each other. mkdir is atomic; a lock whose pid is dead is stale
# (a previous deploy was killed) and gets swept.
mkdir -p "$LOG_DIR"
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
    HOLDER="$(cat "$LOCK_DIR/pid" 2>/dev/null)"
    if [[ -n "$HOLDER" ]] && kill -0 "$HOLDER" 2>/dev/null; then
        die "another deploy is already running (pid $HOLDER, log $(cat "$LOCK_DIR/log" 2>/dev/null || echo '?')) — try ./deploy.sh --status"
    fi
    warn "sweeping stale deploy lock (pid ${HOLDER:-?} is gone)"
    rm -rf "$LOCK_DIR"
    mkdir "$LOCK_DIR" || die "could not take deploy lock at $LOCK_DIR"
fi
echo $$ >"$LOCK_DIR/pid"
echo "${OTTO_DEPLOY_LOG:-terminal}" >"$LOCK_DIR/log"

# Rollback state — updated as the deploy progresses, read by the EXIT trap.
DEPLOY_DONE=0        # set to 1 only after every verification passed
DAEMON_TORN_DOWN=0   # 1 between bootout (phase 6) and bootstrap (phase 9)
STAGING=""           # hidden staged bundle (delete on failure)
OLD=""               # previous app moved aside (restore on failure)

# Whatever kills this script — die(), Ctrl-C, SIGTERM, a crash — the trap puts
# the system back into a RUNNING state: restore the old app if the swap was
# mid-flight, re-bootstrap the daemon if it was torn down, record the outcome.
on_exit() {
    local rc=$?
    trap - EXIT
    if [[ $DEPLOY_DONE -eq 1 ]]; then
        checkpoint "SUCCESS"
    else
        echo; echo "${RED}✗ deploy did not complete (phase: $PHASE) — rolling back${RST}" >&2
        if [[ -n "$OLD" && -d "$OLD" && ! -d "$APP_DST" ]]; then
            mv "$OLD" "$APP_DST" 2>/dev/null && echo "  restored previous $APP_DST" >&2
        fi
        [[ -n "$STAGING" ]] && rm -rf "$STAGING" 2>/dev/null
        if [[ $DAEMON_TORN_DOWN -eq 1 ]] && ! daemon_healthy; then
            echo "  daemon was down — re-bootstrapping previous daemon" >&2
            bootstrap_daemon
            if curl -fsS --retry 10 --retry-delay 1 --retry-all-errors --max-time 20 "$HEALTH_URL" >/dev/null 2>&1; then
                echo "  ${GRN}daemon restored and healthy${RST}" >&2
            else
                echo "  ${RED}daemon still down — run: launchctl kickstart -k $GUI_DOMAIN/$DAEMON_LABEL${RST}" >&2
            fi
        fi
        checkpoint "FAILED (exit $rc)"
        echo "  status: $STATUS_FILE" >&2
    fi
    rm -rf "$LOCK_DIR"
    exit "$rc"
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# ---- launchd helpers ------------------------------------------------------
# `launchctl bootout` is ASYNCHRONOUS — bootstrapping the same label before the
# previous instance is fully reaped fails with "Bootstrap failed: 5: I/O error".
# These helpers serialize teardown→bootstrap and never let a swallowed bootstrap
# error masquerade as a healthy daemon.
GUI_DOMAIN="gui/$(id -u)"
daemon_loaded() { launchctl print "$GUI_DOMAIN/${DAEMON_LABEL}" >/dev/null 2>&1; }
daemon_healthy() { curl -fsS --max-time 5 "$HEALTH_URL" >/dev/null 2>&1; }
# Wait (bounded) until the service is fully un-loaded; best-effort.
# BUDGET (measured, not guessed): ottod's shutdown terminates live sessions,
# flushes ClickHouse and closes Slack sockets first — 0.2–0.4s idle but 4.9–6.0s
# under load. The old 24×0.25s = 6.0s budget sat *inside* that range (a 6.02s
# shutdown was observed), so the wait could return while the job was still dying
# and the bootstrap below would fail. 30s is far past the worst case.
wait_daemon_gone() {
    for _ in $(seq 1 120); do daemon_loaded || return 0; sleep 0.25; done
    return 1
}
# Cleanly (re)bootstrap: tear down any lingering instance, wait for the reap,
# then bootstrap. Idempotent — safe to call even when nothing is loaded.
# Retries, because `bootout` is async and bootstrapping into its tail fails with
# "Input/output error". Echoes the real launchd error instead of swallowing it —
# a silently-swallowed bootstrap failure leaves the machine with NO daemon and no
# explanation, which is the failure this whole dance exists to prevent.
bootstrap_daemon() {
    launchctl bootout "$GUI_DOMAIN/${DAEMON_LABEL}" 2>/dev/null || true
    wait_daemon_gone || warn "daemon still loaded after 30s — bootstrapping anyway"
    local err rc
    for _ in $(seq 1 20); do
        err="$(launchctl bootstrap "$GUI_DOMAIN" "$PLIST" 2>&1)"; rc=$?
        [[ $rc -eq 0 ]] && return 0
        case "$err" in *"already bootstrapped"*) return 0 ;; esac
        sleep 0.5
    done
    warn "launchctl bootstrap failed after 20 attempts: ${err:-unknown error}"
    return 1
}

cd "$ROOT" || die "cannot cd into repo root $ROOT"

# ---- preflight ------------------------------------------------------------
step "Preflight"
[[ -f "$PLIST" ]] || die "launchd plist not found: $PLIST"
security find-identity -p codesigning 2>/dev/null | grep -q "$CERT_NAME" \
    || die "signing identity '$CERT_NAME' not found (run packaging/make-cert.sh)"
TAURI="$(command -v tauri || true)"
[[ -n "$TAURI" ]] || die "tauri CLI not found on PATH"
command -v cargo >/dev/null || die "cargo not found"
command -v npm   >/dev/null || die "npm not found"
command -v cmake >/dev/null || die "cmake not found — needed to build librdkafka (the Kafka/Message Brokers driver) from source. Install it: 'brew install cmake'"
ok "identity '$CERT_NAME', tauri at $TAURI, triple $TRIPLE"

# =====================================================================
# PHASE 1 — BUILD
# =====================================================================
step "1/9  Build UI  (ui/dist)"
cd "$ROOT/ui" || die "no ui/ dir"
need_ci=$FORCE_CI
if [[ ! -d node_modules ]]; then need_ci=1
elif [[ package-lock.json -nt node_modules/.package-lock.json ]]; then need_ci=1; fi
if [[ $need_ci -eq 1 ]]; then
    run npm ci || die "npm ci failed"
else
    ok "node_modules fresh — skipping npm ci  (--force-ci to override)"
fi
run npm run build || die "npm run build failed"
FRESH_JS="$(ls -1 "$ROOT"/ui/dist/assets/index-*.js 2>/dev/null | head -1)"
FRESH_JS="$(basename "${FRESH_JS:-unknown}")"
ok "ui/dist built — main chunk: $FRESH_JS"
cd "$ROOT"

step "2/9  Build ottod  (release, embed-ui)"
run cargo build --release -p ottod --features embed-ui || die "cargo build ottod failed"
[[ -x "$ROOT/target/release/ottod" ]] || die "target/release/ottod missing after build"
run cp "$ROOT/target/release/ottod" "$SIDECAR_SRC" || die "could not stage sidecar binary"
ok "ottod built + staged as $(basename "$SIDECAR_SRC")"

step "3/9  Tauri build  (Otto.app bundle)"
BUNDLES="app"; [[ $WANT_DMG -eq 1 ]] && BUNDLES="app,dmg"
( cd "$ROOT/apps/desktop/src-tauri" && run "$TAURI" build --bundles "$BUNDLES" ) || die "tauri build failed"
[[ -d "$BUILT_APP" ]] || die "built bundle not found at $BUILT_APP"
ok "bundle: $BUILT_APP"

# =====================================================================
# PHASE 2 — SIGN  (seal the app incl. its nested sidecar)
# =====================================================================
step "4/9  Sign + verify"
run "$ROOT/packaging/sign.sh" "$BUILT_APP" || die "signing failed"
run codesign --verify --deep --strict "$BUILT_APP" || die "code-signature verification failed"
ok "signed + seal verified"

# =====================================================================
# PHASE 3 — STAGE the signed app INTO /Applications  (atomic-swap prep)
# =====================================================================
# Copy the freshly-signed bundle to a hidden sibling of the final path so the
# actual install (phase 5) is a single same-volume rename() — atomic, so a
# killed deploy can never leave a half-written /Applications/Otto.app. Do this
# while the old app + daemon are still UP: nothing is torn down until a verified
# copy is staged and ready, so a failure here leaves the running install intact.
step "5/9  Stage signed app in /Applications  (atomic-swap prep)"
STAGING="/Applications/.Otto.app.staging.$$"
# sweep any stale staging / old siblings a killed previous run may have left behind
rm -rf /Applications/.Otto.app.staging.* /Applications/.Otto.app.old.* 2>/dev/null || true
run ditto "$BUILT_APP" "$STAGING" || die "could not stage bundle at $STAGING"
run codesign --verify --deep --strict "$STAGING" \
    || { rm -rf "$STAGING"; die "staged bundle failed signature verification"; }
ok "staged + verified at $(basename "$STAGING")"

# =====================================================================
# PHASE 4 — STOP the running app + daemon
# =====================================================================
step "6/9  Quit running app + stop daemon"
# bounded: `osascript … quit` blocks for as long as the app takes to answer the
# Apple event — a wedged app or a modal dialog stalls the deploy forever.
bounded 15 osascript -e 'tell application "Otto" to quit' >/dev/null 2>&1 || true
for _ in $(seq 1 16); do
    pgrep -f 'Otto.app/Contents/MacOS/otto-desktop' >/dev/null 2>&1 || break
    sleep 0.5
done
if pgrep -f 'Otto.app/Contents/MacOS/otto-desktop' >/dev/null 2>&1; then
    warn "graceful quit lingered — force-killing otto-desktop"
    pkill -9 -f 'Otto.app/Contents/MacOS/otto-desktop' 2>/dev/null || true
    sleep 1
fi
pgrep -f 'Otto.app/Contents/MacOS/otto-desktop' >/dev/null 2>&1 \
    && die "otto-desktop still running — refusing to replace /Applications" || ok "app not running"
launchctl bootout "$GUI_DOMAIN/${DAEMON_LABEL}" 2>/dev/null || true
wait_daemon_gone
DAEMON_TORN_DOWN=1
ok "daemon booted out"

# =====================================================================
# PHASE 5 — SWAP the staged app into place  (atomic rename)
# =====================================================================
# Two same-volume renames: move the old app aside to .old.$$, then move the
# pre-verified staging copy onto the final path. Each mv is atomic; if the second
# fails we restore the old app so /Applications is never left without an Otto.app.
step "7/9  Swap staged app into /Applications/Otto.app"
OLD="/Applications/.Otto.app.old.$$"
[[ -d "$APP_DST" ]] && { run mv "$APP_DST" "$OLD" || die "could not move old app aside"; }
run mv "$STAGING" "$APP_DST" || {
    [[ -d "$OLD" ]] && mv "$OLD" "$APP_DST"
    die "swap failed — previous app restored"
}
ok "atomically swapped in fresh Otto.app"

# =====================================================================
# PHASE 6 — Make installed daemon byte-identical to the bundle sidecar
# =====================================================================
step "8/9  Sync daemon binary == bundle sidecar (byte-identical)"
mkdir -p "$INSTALL_DIR"
if [[ -f "$INSTALLED_OTTOD" ]]; then
    BAK="$INSTALLED_OTTOD.bak.$(date +%s)"
    # FATAL if this fails: writing the new daemon over the old file's inode
    # leaves the kernel's cached code signature stale → every exec dies with
    # SIGKILL (Code Signature Invalid) and KeepAlive respawns into a kill loop.
    # The mv guarantees the ditto below creates a FRESH inode.
    mv "$INSTALLED_OTTOD" "$BAK" || die "could not move old daemon aside (would overwrite in place → stale-signature SIGKILL loop)"
    ok "backed up old daemon → $(basename "$BAK")"
fi
# prune old backups, keep the most recent $KEEP_BACKUPS (deploy cruft, not user data).
# Portable: no `mapfile` (absent in macOS stock bash 3.2).
pruned=0
while IFS= read -r bak; do
    [[ -n "$bak" ]] || continue
    rm -f "$bak" && pruned=$((pruned + 1))
done < <(ls -1t "$INSTALL_DIR"/ottod.bak.* 2>/dev/null | tail -n +$((KEEP_BACKUPS + 1)))
[[ $pruned -gt 0 ]] && ok "pruned $pruned old backup(s), kept $KEEP_BACKUPS"
run ditto "$APP_DST/Contents/MacOS/ottod" "$INSTALLED_OTTOD" || die "could not sync daemon binary"
run codesign --verify --strict "$INSTALLED_OTTOD" || die "installed daemon failed signature verification — refusing to bootstrap a binary the kernel will SIGKILL"
BIN_SHA="$(shasum -a 256 "$INSTALLED_OTTOD" | awk '{print $1}')"
BUNDLE_SHA="$(shasum -a 256 "$APP_DST/Contents/MacOS/ottod" | awk '{print $1}')"
[[ "$BIN_SHA" == "$BUNDLE_SHA" ]] || die "bin/ottod != bundle sidecar (clobber risk): $BIN_SHA vs $BUNDLE_SHA"
ok "bin == bundle sidecar  (${BIN_SHA:0:12}…)"

# =====================================================================
# PHASE 7 — START daemon + app, then verify
# =====================================================================
step "9/9  Start daemon + app, verify"
bootstrap_daemon
# health (curl retries until the daemon answers or we give up). A swallowed
# bootstrap error (launchd reap race) shows up here as no health → retry ONE
# clean teardown+bootstrap before giving up, so the race self-heals.
# When the daemon won't come up, SAY WHY: surface the launchd state and any
# fresh crash report (a "Code Signature Invalid" .ips here means a stale-inode
# / bad-signature binary — the kernel kills it before it can log a single line).
diagnose_daemon() {
    echo "  ${DIM}-- launchd state --${RST}"
    launchctl print "$GUI_DOMAIN/${DAEMON_LABEL}" 2>/dev/null | grep -E 'state|last exit|spawn' | sed 's/^/  /'
    local ips
    ips="$(ls -t "$HOME"/Library/Logs/DiagnosticReports/ottod-*.ips 2>/dev/null | head -1)"
    if [[ -n "$ips" && -n "$(find "$ips" -newermt "@$START_TS" 2>/dev/null)" ]]; then
        echo "  ${DIM}-- crash report $(basename "$ips") --${RST}"
        grep -oE '"signal":"[^"]*"|"type":"[^"]*"|"terminationReason[^,]*' "$ips" 2>/dev/null | head -5 | sed 's/^/  /'
    fi
}
if curl -fsS --retry 15 --retry-delay 1 --retry-all-errors --max-time 30 "$HEALTH_URL" >/dev/null 2>&1; then
    DAEMON_TORN_DOWN=0
    ok "daemon healthy: $(curl -fsS "$HEALTH_URL")"
else
    warn "daemon didn't answer on first bootstrap (launchd reap race) — retrying"
    bootstrap_daemon
    if curl -fsS --retry 20 --retry-delay 1 --retry-all-errors --max-time 40 "$HEALTH_URL" >/dev/null 2>&1; then
        DAEMON_TORN_DOWN=0
        ok "daemon healthy after retry: $(curl -fsS "$HEALTH_URL")"
    else
        diagnose_daemon
        die "daemon did NOT come up healthy at $HEALTH_URL — check ~/Library/Logs/Otto/ottod.log.*"
    fi
fi
open -a "$APP_DST" || warn "could not 'open' the app"
bounded 10 osascript -e 'tell application "Otto" to activate' >/dev/null 2>&1 || true
ok "app launched + activated"

# settle past the supervisor relaunch-clobber window, then confirm the daemon
# is genuinely running (not throttled / spawn-scheduled)
sleep 10
DSTATE="$(launchctl print "$GUI_DOMAIN/${DAEMON_LABEL}" 2>/dev/null | grep -E '^\s*state =' | head -1 | awk '{print $3}')"
if curl -fsS --max-time 5 "$HEALTH_URL" >/dev/null 2>&1; then
    ok "post-launch daemon still healthy (state=${DSTATE:-?})"
else
    diagnose_daemon
    die "daemon DROPPED after app launch (state=${DSTATE:-?}) — likely codesigning throttle; bin/bundle hash mismatch?"
fi

# served JS == freshly-built dist (stale-build guard)
SERVED_JS="$(curl -fsS --max-time 5 "$SERVE_URL" 2>/dev/null | grep -oE 'index-[A-Za-z0-9_-]+\.js' | head -1)"
if [[ -n "$SERVED_JS" ]]; then
    if [[ "$SERVED_JS" == "$FRESH_JS" ]]; then ok "served UI == fresh build ($SERVED_JS)"
    else warn "served UI ($SERVED_JS) != fresh build ($FRESH_JS) — possible stale embed"; fi
else
    warn "could not read served UI chunk (skipping stale-build check)"
fi

# app binaries match what we just built
APP_SHA="$(shasum -a 256 "$APP_DST/Contents/MacOS/otto-desktop" | awk '{print $1}')"
BUILT_SHA="$(shasum -a 256 "$BUILT_APP/Contents/MacOS/otto-desktop" | awk '{print $1}')"
[[ "$APP_SHA" == "$BUILT_SHA" ]] && ok "installed app == built app (${APP_SHA:0:12}…)" \
    || warn "installed app binary != built app binary"

# every post-deploy verification passed — drop the previous app we kept aside
# for rollback safety (only exists if an old app was swapped out)
[[ -d "$OLD" ]] && rm -rf "$OLD" && ok "removed rollback copy of previous app"

# sweep stale build artifacts: cargo never deletes superseded per-hash outputs
# (each dep bump leaves another ~600MB libotto_server rlib in target/…/deps
# forever — target has hit 79GB+). Runs only after a fully verified deploy.
# 2 days ≈ several deploys back at the current cadence (a 7-day window was
# measured to keep ~60GB of dead rlibs alive).
#
# NEVER touch build/ or .fingerprint/: build-script OUT_DIRs (libsqlite3-sys
# bindgen.rs, tree-sitter stdlib-symbols.txt, …) keep their ORIGINAL mtime
# forever while staying live — cargo trusts the fingerprint and hard-errors
# on the missing include instead of regenerating. Sweeping them by mtime
# corrupted the cache and broke the NEXT deploy's build, repeatedly (the
# recurring "couldn't read OUT_DIR/bindgen.rs" failures). deps/ rlibs are
# safe: cargo stats those artifacts and rebuilds when missing.
SWEEP_DAYS=2
swept_kb=0
for tdir in "$ROOT/target" "$ROOT/apps/desktop/src-tauri/target"; do
    [[ -d "$tdir" ]] || continue
    before_kb=$(du -sk "$tdir" 2>/dev/null | awk '{print $1}')
    find "$tdir" \
        \( -path '*/build/*' -o -path '*/.fingerprint/*' \) -prune \
        -o -type f -mtime +"$SWEEP_DAYS" -delete 2>/dev/null
    find "$tdir" \
        \( -path '*/build' -o -path '*/.fingerprint' \) -prune \
        -o -type d -empty -delete 2>/dev/null
    after_kb=$(du -sk "$tdir" 2>/dev/null | awk '{print $1}')
    swept_kb=$(( swept_kb + before_kb - after_kb ))
done
[[ $swept_kb -gt 0 ]] && ok "swept $(( swept_kb / 1024 ))MB of stale build artifacts (>${SWEEP_DAYS}d old)"

DEPLOY_DONE=1
ELAPSED=$(( $(date +%s) - START_TS ))
echo
echo "${BOLD}${GRN}✓ Deploy complete${RST} in ${ELAPSED}s — Otto is running on the new build."
echo "${DIM}  daemon: launchd ${DAEMON_LABEL} • health ${HEALTH_URL} • logs ~/Library/Logs/Otto/ottod.log.*${RST}"
