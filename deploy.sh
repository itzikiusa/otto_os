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
# Usage:
#   ./deploy.sh                 # full rebuild + redeploy (default)
#   ./deploy.sh --dmg           # also produce a DMG alongside the .app
#   ./deploy.sh --force-ci      # force `npm ci` even if node_modules looks fresh
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
KEEP_BACKUPS=5

WANT_DMG=0
FORCE_CI=0
for arg in "$@"; do
    case "$arg" in
        --dmg)       WANT_DMG=1 ;;
        --force-ci)  FORCE_CI=1 ;;
        -h|--help)
            sed -n '2,41p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *) echo "unknown flag: $arg (try --help)" >&2; exit 2 ;;
    esac
done

# ---- pretty output --------------------------------------------------------
BOLD=$'\033[1m'; DIM=$'\033[2m'; GRN=$'\033[32m'; RED=$'\033[31m'; YEL=$'\033[33m'; RST=$'\033[0m'
START_TS=$(date +%s)
step() { echo; echo "${BOLD}▸ $*${RST}"; }
ok()   { echo "  ${GRN}✓${RST} $*"; }
warn() { echo "  ${YEL}!${RST} $*"; }
die()  { echo; echo "${RED}✗ FAILED:${RST} $*" >&2; echo "${DIM}  (nothing irreversible past the build phase unless noted above)${RST}" >&2; exit 1; }
run()  { echo "  ${DIM}\$ $*${RST}"; "$@"; }

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
    OTTO_DEPLOY_DETACHED=1 nohup "$0" "$@" >"$LOG" 2>&1 </dev/null & disown
    exit 0
fi

# ---- launchd helpers ------------------------------------------------------
# `launchctl bootout` is ASYNCHRONOUS — bootstrapping the same label before the
# previous instance is fully reaped fails with "Bootstrap failed: 5: I/O error".
# These helpers serialize teardown→bootstrap and never let a swallowed bootstrap
# error masquerade as a healthy daemon.
GUI_DOMAIN="gui/$(id -u)"
daemon_loaded() { launchctl print "$GUI_DOMAIN/${DAEMON_LABEL}" >/dev/null 2>&1; }
daemon_healthy() { curl -fsS --max-time 5 "$HEALTH_URL" >/dev/null 2>&1; }
# Wait (bounded) until the service is fully un-loaded; best-effort.
wait_daemon_gone() {
    for _ in $(seq 1 24); do daemon_loaded || return 0; sleep 0.25; done
}
# Cleanly (re)bootstrap: tear down any lingering instance, wait for the reap,
# then bootstrap. Idempotent — safe to call even when nothing is loaded.
bootstrap_daemon() {
    launchctl bootout "$GUI_DOMAIN/${DAEMON_LABEL}" 2>/dev/null || true
    wait_daemon_gone
    launchctl bootstrap "$GUI_DOMAIN" "$PLIST" 2>/dev/null || true
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
osascript -e 'tell application "Otto" to quit' >/dev/null 2>&1 || true
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
    mv "$INSTALLED_OTTOD" "$BAK" && ok "backed up old daemon → $(basename "$BAK")"
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
if curl -fsS --retry 15 --retry-delay 1 --retry-all-errors --max-time 30 "$HEALTH_URL" >/dev/null 2>&1; then
    ok "daemon healthy: $(curl -fsS "$HEALTH_URL")"
else
    warn "daemon didn't answer on first bootstrap (launchd reap race) — retrying"
    bootstrap_daemon
    if curl -fsS --retry 20 --retry-delay 1 --retry-all-errors --max-time 40 "$HEALTH_URL" >/dev/null 2>&1; then
        ok "daemon healthy after retry: $(curl -fsS "$HEALTH_URL")"
    else
        die "daemon did NOT come up healthy at $HEALTH_URL — check ~/Library/Logs/Otto/ottod.log.*"
    fi
fi
open -a "$APP_DST" || warn "could not 'open' the app"
osascript -e 'tell application "Otto" to activate' >/dev/null 2>&1 || true
ok "app launched + activated"

# settle past the supervisor relaunch-clobber window, then confirm the daemon
# is genuinely running (not throttled / spawn-scheduled)
sleep 10
DSTATE="$(launchctl print "$GUI_DOMAIN/${DAEMON_LABEL}" 2>/dev/null | grep -E '^\s*state =' | head -1 | awk '{print $3}')"
if curl -fsS --max-time 5 "$HEALTH_URL" >/dev/null 2>&1; then
    ok "post-launch daemon still healthy (state=${DSTATE:-?})"
else
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

ELAPSED=$(( $(date +%s) - START_TS ))
echo
echo "${BOLD}${GRN}✓ Deploy complete${RST} in ${ELAPSED}s — Otto is running on the new build."
echo "${DIM}  daemon: launchd ${DAEMON_LABEL} • health ${HEALTH_URL} • logs ~/Library/Logs/Otto/ottod.log.*${RST}"
