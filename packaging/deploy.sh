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
#         packaging/deploy.sh --yes      no confirmation prompt (live sessions)
#         packaging/deploy.sh --dmg      also write Otto.dmg next to the bundle
#         packaging/deploy.sh --force-ci always `npm ci` before the UI build
#         packaging/deploy.sh --status   show the log of the last install/verify phase
#                                        (non-zero when it failed, is running, or
#                                        /Applications/Otto.app is missing)
# Env:    SKIP_UI=1    rejected: a source receipt requires a fresh UI build
#         OTTO_DEPLOY_YES=1 same as --yes
#         KEEP_DEPLOY_LOGS=20 deploy logs kept in ~/Library/Logs/Otto
#         EMBED_UI=0   build ottod WITHOUT the SPA baked in (see step 2)
#         DETACH=0     run steps 6–7 inline instead of under launchd (see below)
#         PRUNE=0      default: preserve Cargo caches (skip step 5)
#         PRUNE=1      evict older dependency variants; may force recompilation
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
#
# SAFETY (ported from the old root deploy.sh): the new bundle is staged to a
# hidden sibling (/Applications/.Otto.app.staging.<pid>) and verified BEFORE
# the running app is touched; the install is two same-volume renames (old →
# .Otto.app.old.<pid>, staging → Otto.app), so a killed deploy never leaves a
# half-copied app. If the post-install verification fails — or the phase dies
# after the swap — the previous app is renamed back and relaunched (its
# supervisor reinstalls the previous daemon). After a verified deploy the
# previous app is kept as /Applications/.Otto.app.previous (and the previous
# daemon as bin/ottod.prev) for a manual rollback. A lock dir
# (~/Library/Logs/Otto/deploy.lock) refuses concurrent deploys; the build
# foreground hands it to the detached finish job.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
APP_SRC="$ROOT/apps/desktop/src-tauri"
APP="$APP_SRC/target/release/bundle/macos/Otto.app"
INSTALLED_APP="/Applications/Otto.app"
BUILD_RECEIPT="$APP_SRC/target/release/deploy-build.receipt"

cd "$ROOT"

LOG_DIR_DEFAULT="$HOME/Library/Logs/Otto"
LOCK_DIR="$LOG_DIR_DEFAULT/deploy.lock"
WANT_STATUS=0
ASSUME_YES="${OTTO_DEPLOY_YES:-0}"
WANT_DMG=0
FORCE_CI=0
for arg in "$@"; do
    case "$arg" in
        --status) WANT_STATUS=1 ;;
        --yes|-y) ASSUME_YES=1 ;;
        --dmg) WANT_DMG=1 ;;
        --force-ci) FORCE_CI=1 ;;
        -h|--help) sed -n '2,/^set -euo pipefail/{ /^set -euo pipefail/d; s/^# \{0,1\}//; p; }' "$0"; exit 0 ;;
        *) echo "unknown flag: $arg (try --help)" >&2; exit 2 ;;
    esac
done

if [[ "$WANT_STATUS" == 1 ]]; then
    latest="$LOG_DIR_DEFAULT/deploy-finish-latest.log"
    if [[ -d "$LOCK_DIR" ]] && read -r holder _ < "$LOCK_DIR/pid" 2>/dev/null && kill -0 "$holder" 2>/dev/null; then
        echo "== deploy lock held by pid $holder"
    fi
    app_rc=0
    if [[ ! -d "$INSTALLED_APP" ]]; then
        # A deploy killed between the two swap renames leaves the previous app
        # at .Otto.app.old.<pid> — THAT is the copy to restore (the next deploy
        # does it automatically); .previous is the one before it.
        aside="$(ls -1dt "$(dirname "$INSTALLED_APP")"/.Otto.app.old.* 2>/dev/null | head -1 || true)"
        echo "== $INSTALLED_APP is MISSING — restore ${aside:-$(dirname "$INSTALLED_APP")/.Otto.app.previous} (mv it back) or redeploy"
        app_rc=5
    fi
    [[ -f "$latest" ]] || { echo "no detached deploy phase has run yet"; exit $(( app_rc ? app_rc : 2 )); }
    echo "== $(readlink "$latest" || echo "$latest")"
    grep -v "^DEPLOY-FINISH EXIT=" "$latest" || true
    if line="$(grep "^DEPLOY-FINISH EXIT=" "$latest" | tail -1)" && [[ -n "$line" ]]; then
        rc="${line#DEPLOY-FINISH EXIT=}"
        [[ "$rc" == "0" ]] && echo "== finished OK" || echo "== FAILED (exit $rc)"
        [[ "$app_rc" == 0 ]] || exit "$app_rc"
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
    # Re-copy from the RECEIPTED sidecar only. Never re-sign here: signing
    # $APP after the receipt changes the build verify_install compares
    # against (diff -qr), turning a heal into a rollback.
    cp -f "$side" "$dep.fresh"
    if ! codesign --verify "$dep.fresh" 2>/dev/null; then
        command rm -f "$dep.fresh"
        echo "    self-heal: the bundled sidecar fails codesign --verify — not installing it" >&2
        return 1
    fi
    mv -f "$dep.fresh" "$dep"          # atomic replace → NEW inode
    launchctl kickstart -k "gui/$(id -u)/com.otto.daemon" 2>/dev/null || true
}

# ---- staged install, atomic swap, rollback --------------------------------
# State read by rollback_install / finish_cleanup (also from an EXIT trap).
STAGED_APP=""     # hidden staged copy of the signed build (delete on failure)
PREVIOUS_APP=""   # the app the swap moved aside (restore on failure)
SWAP_DONE=0       # 1 once the new app sits at $INSTALLED_APP
apps_dir() { dirname "$INSTALLED_APP"; }

# Copy + verify the signed build next to the final path while the old app and
# daemon still run: a failure here leaves the running install untouched.
stage_app() {
    STAGED_APP="$(apps_dir)/.Otto.app.staging.$$"
    rm -rf "$STAGED_APP"
    ditto "$APP" "$STAGED_APP" || { fail_verify "could not stage the app at $STAGED_APP"; return 1; }
    codesign --verify --deep --strict "$STAGED_APP" ||
        { fail_verify 'staged app failed signature verification'; return 1; }
}

# Two same-volume renames — each atomic — so /Applications never holds a
# half-written Otto.app; if the second fails the first is undone.
swap_app() {
    PREVIOUS_APP=""
    if [[ -d "$INSTALLED_APP" ]]; then
        PREVIOUS_APP="$(apps_dir)/.Otto.app.old.$$"
        mv "$INSTALLED_APP" "$PREVIOUS_APP" ||
            { PREVIOUS_APP=""; fail_verify 'could not move the installed app aside'; return 1; }
    fi
    if ! mv "$STAGED_APP" "$INSTALLED_APP"; then
        [[ -z "$PREVIOUS_APP" ]] || mv "$PREVIOUS_APP" "$INSTALLED_APP"
        PREVIOUS_APP=""
        fail_verify 'swap failed — previous app restored'; return 1
    fi
    STAGED_APP=""
    SWAP_DONE=1
}

# The app redeploys the daemon ONLY at start, so it must be quit first (the
# launchd agent has KeepAlive, so quitting the app doesn't stop the daemon).
# Under launchd, osascript may lack Automation rights for Otto (TCC) and quit
# silently does nothing — fall back to a plain TERM on the shell process; the
# daemon is a separate launchd job, so nothing user-facing is lost.
# Only the app AT $INSTALLED_APP is waited on or signalled (app_pid matches the
# executable path): a `tauri dev` build or another copy is never touched.
quit_app() {
    local pid
    osascript -e 'quit app "Otto"' 2>/dev/null || true
    for _ in $(seq 1 12); do app_pid >/dev/null || break; sleep 0.5; done
    if pid="$(app_pid)"; then
        echo "    app did not quit via AppleScript — terminating pid $pid"
        kill -TERM "$pid" 2>/dev/null || true
        for _ in $(seq 1 10); do app_pid >/dev/null || break; sleep 0.5; done
    fi
    if app_pid >/dev/null; then fail_verify "app did not exit"; return 1; fi
}

# Put the previous app back after a failed verify (or a phase that died after
# the swap) and relaunch it. Its supervisor sees bundle sidecar != bin/ottod
# and reinstalls the PREVIOUS daemon, so the daemon rolls back with it.
rollback_install() {
    [[ "$SWAP_DONE" == 1 ]] || return 0
    if [[ -z "$PREVIOUS_APP" || ! -d "$PREVIOUS_APP" ]]; then
        echo "    ROLLBACK: no previous app to restore — leaving the new one in place" >&2
        return 1
    fi
    echo "==> ROLLBACK  restoring the previous Otto.app"
    # The FAILED build's daemon: success means a different process.
    local failed_pid
    failed_pid="$(daemon_pid)" || failed_pid=""
    quit_app || true
    local failed
    failed="$(apps_dir)/.Otto.app.failed.$$"
    if [[ -d "$INSTALLED_APP" ]] && ! mv "$INSTALLED_APP" "$failed"; then
        echo "    ROLLBACK FAILED: could not move the new app aside" >&2; return 1
    fi
    if ! mv "$PREVIOUS_APP" "$INSTALLED_APP"; then
        [[ ! -d "$failed" ]] || mv "$failed" "$INSTALLED_APP"
        echo "    ROLLBACK FAILED: could not restore $PREVIOUS_APP" >&2; return 1
    fi
    PREVIOUS_APP=""
    SWAP_DONE=0
    rm -rf "$failed"
    open "$INSTALLED_APP" || true
    if verify_rollback "$failed_pid"; then
        echo "    ROLLED BACK: previous app relaunched; previous daemon verified (pid $ROLLBACK_PID, healthy)"
    else
        echo "    ROLLED BACK: previous app restored, but the previous daemon was NOT verified" \
             "(still the failed build's pid, a different binary, or unhealthy) — check ~/Library/Logs/Otto/ and packaging/README.md#recovery" >&2
    fi
    echo "    note: migrations the new daemon applied stay applied (older builds boot on the newer additive schema);" \
         "pre-migration DB snapshots are in ~/Library/Application Support/Otto/backups/"
}

# The restored app's supervisor must have put ITS sidecar in place and that
# daemon must be the one answering: a pid other than the failed build's, at
# the deployed path, with the restored bundle's bytes, and healthy. Health
# alone used to pass against the failed daemon before the supervisor booted it out.
ROLLBACK_PID=""
verify_rollback() {
    local failed_pid="$1" pid restored_side="$INSTALLED_APP/Contents/MacOS/ottod"
    for _ in $(seq 1 30); do
        pid="$(daemon_pid)" || pid=""
        if [[ -n "$pid" && "$pid" != "$failed_pid" && -f "$dep" ]] &&
            [[ "$(process_path "$pid")" == "$dep" ]] &&
            cmp -s "$restored_side" "$dep" && health_ok; then
            ROLLBACK_PID="$pid"; return 0
        fi
        sleep 2
    done
    return 1
}

# Last-resort cleanup on ANY exit of the install phase (set -e, a signal).
finish_cleanup() {
    local rc="$1"
    if [[ "$rc" != 0 && "$SWAP_DONE" == 1 ]]; then rollback_install || true; fi
    # Killed between the two renames: the old app is aside and nothing is installed.
    if [[ -n "$PREVIOUS_APP" && -d "$PREVIOUS_APP" && ! -d "$INSTALLED_APP" ]]; then
        mv "$PREVIOUS_APP" "$INSTALLED_APP" && echo "    restored the previous $INSTALLED_APP" >&2
    fi
    [[ -z "$STAGED_APP" ]] || rm -rf "$STAGED_APP"
}

# A finish job SIGKILLed between swap_app's two renames (no EXIT trap) leaves
# no $INSTALLED_APP and the previous app at .Otto.app.old.<pid>. Put a lone
# such copy back before anything else, so this run has a real rollback target
# — and prune .old copies orphaned next to a present app.
recover_interrupted_swap() {
    local olds=() o
    for o in "$(apps_dir)"/.Otto.app.old.*; do [[ -d "$o" ]] && olds+=("$o"); done
    [[ ${#olds[@]} -gt 0 ]] || return 0
    if [[ ! -d "$INSTALLED_APP" ]]; then
        if [[ ${#olds[@]} -eq 1 ]]; then
            mv "${olds[0]}" "$INSTALLED_APP" ||
                { fail_verify "could not restore ${olds[0]} to $INSTALLED_APP"; return 1; }
            echo "    restored $INSTALLED_APP from ${olds[0]} (an earlier deploy was killed mid-swap)"
            return 0
        fi
        fail_verify "$INSTALLED_APP is missing and several .Otto.app.old.* copies exist — mv the right one back by hand"
        return 1
    fi
    for o in "${olds[@]}"; do
        rm -rf "$o" && echo "    pruned orphaned $o"
    done
}

# A verified deploy keeps ONE previous app for a manual rollback.
keep_previous_app() {
    [[ -n "$PREVIOUS_APP" && -d "$PREVIOUS_APP" ]] || return 0
    local keep
    keep="$(apps_dir)/.Otto.app.previous"
    rm -rf "$keep"
    mv "$PREVIOUS_APP" "$keep" || rm -rf "$PREVIOUS_APP"
    PREVIOUS_APP=""
    echo "    previous app kept as $keep"
}

finish_phase() {
local old_pid old_hash
old_pid="$(daemon_pid)" || old_pid=""
dep="$(deployed_daemon_path)"
side="$APP/Contents/MacOS/ottod"
old_hash=""
[[ ! -f "$dep" ]] || old_hash="$(sha256 "$dep")"
echo "==> 6/7  Stage, install & relaunch"
# Sweep stale siblings a killed earlier run may have left (never .previous).
rm -rf "$(apps_dir)"/.Otto.app.staging.* "$(apps_dir)"/.Otto.app.failed.* 2>/dev/null || true
recover_interrupted_swap || return 1
stage_app || return 1
quit_app || return 1
swap_app || return 1
open "$INSTALLED_APP"

echo "==> 7/7  Verify (+ self-heal codesigning crash-loop)"

if poll_health 20; then
    echo "    daemon healthy: $(curl -s localhost:7700/api/v1/health)"
else
    # `|| true`: no "last exit reason" line must not kill the phase (set -e + pipefail).
    reason="$(launchctl print "$GUI_DOMAIN/$LABEL" 2>/dev/null | grep -i 'last exit reason' | head -1 | xargs || true)"
    daemon_loaded || reason="${reason:-service not registered with launchd}"
    echo "    daemon not healthy yet — ${reason:-no launchd reason}"
    if echo "$reason" | grep -qi 'CODESIGNING'; then
        heal_codesign_inode || echo "    WARN: self-heal skipped"
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

if ! verify_install; then
    rollback_install || true
    return 1
fi
keep_previous_app
echo "done."
}

# ---- single-deploy lock ---------------------------------------------------
# mkdir is atomic. `$LOCK_DIR/pid` holds "<pid> <process start time>": a lock
# whose pid is dead — or alive but started at another time (a reused pid) — is
# stale. A stale lock is renamed aside (atomic) and the rename re-checked, so
# two deploys that both saw it stale can't both take it. The build foreground
# hands the lock to the detached finish job by passing its own pid as
# OTTO_DEPLOY_LOCK_HANDOFF; that job then takes the lock over.
LOCK_HELD=0
proc_start() { ps -p "$1" -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//; s/[[:space:]]*$//'; }
lock_holder_alive() {
    [[ -n "$1" ]] && kill -0 "$1" 2>/dev/null || return 1
    [[ -z "$2" ]] && return 0          # a lock written before start times were recorded
    [[ "$(proc_start "$1")" == "$2" ]]
}
lock_dir_age() {
    local m
    m="$(stat -f %m "$LOCK_DIR" 2>/dev/null || stat -c %Y "$LOCK_DIR" 2>/dev/null)" || { echo 0; return; }
    echo $(( $(date +%s) - m ))
}
lock_acquire() {
    local holder start moved stale tries=0
    mkdir -p "$(dirname "$LOCK_DIR")"
    while ! mkdir "$LOCK_DIR" 2>/dev/null; do
        holder=""; start=""
        read -r holder start < "$LOCK_DIR/pid" 2>/dev/null || true
        if [[ -n "$holder" && "$holder" == "${OTTO_DEPLOY_LOCK_HANDOFF:-}" ]]; then
            break   # handed to us by the build foreground
        fi
        if [[ -z "$holder" && "$(lock_dir_age)" -lt 10 ]]; then
            fail_verify "another deploy is taking the lock right now — see packaging/deploy.sh --status"
            return 1
        fi
        if lock_holder_alive "$holder" "$start"; then
            fail_verify "another deploy is running (pid $holder) — see packaging/deploy.sh --status"
            return 1
        fi
        tries=$((tries + 1))
        if [[ $tries -gt 3 ]]; then
            fail_verify "could not take over the stale deploy lock $LOCK_DIR"; return 1
        fi
        echo "    sweeping stale deploy lock (pid ${holder:-?} is gone)"
        stale="$LOCK_DIR.stale.$$"
        command rm -rf "$stale"
        mv "$LOCK_DIR" "$stale" 2>/dev/null || continue
        moved=""
        read -r moved _ < "$stale/pid" 2>/dev/null || true
        if [[ "$moved" != "$holder" ]]; then
            # Another deploy swept it and re-locked in between: that's theirs.
            mv "$stale" "$LOCK_DIR" 2>/dev/null || true
            fail_verify "another deploy took the lock (pid ${moved:-?}) — see packaging/deploy.sh --status"
            return 1
        fi
        command rm -rf "$stale"
    done
    printf '%s %s\n' "$$" "$(proc_start $$)" > "$LOCK_DIR/pid"
    LOCK_HELD=1
}
lock_release() {
    local holder=""
    [[ "$LOCK_HELD" == 1 ]] || return 0
    read -r holder _ < "$LOCK_DIR/pid" 2>/dev/null || true
    [[ "$holder" == "$$" ]] && rm -rf "$LOCK_DIR"
    LOCK_HELD=0
}

# ---- live sessions a deploy will end (read-only) --------------------------
# The relaunch restarts ottod. User-started agent sessions — shell terminals
# included (kind=agent, provider=shell) — run in PTY holders and SURVIVE
# (session persistence, on by default). What dies: connection terminals
# (kind=connection: ssh / db / k8s exec) and background or non-manual agents
# (meta.source in the background list, or meta.work.origin other than
# "manual"). This mirrors otto-sessions `is_user_started` +
# `Session::is_foreground_agent`; BACKGROUND_SOURCES must match
# otto-core BACKGROUND_SESSION_SOURCES (a test pins it). Read-only GET; needs
# a token (OTTO_API_TOKEN, else the session's OTTO_MCP_TOKEN) — without one
# the count is unknown. Prints "<total> <held> <connections> <background>".
BACKGROUND_SOURCES="channel review review_summarizer skilleval skillreview product-analysis product_refine swarm canvas_assist canvas_assist_preview mockup_assist db_assist workflow vault-docs vault-docs-review pr-draft commit-draft insights run_with_otto goal_loop discovery_chat scheduled_task finding assistant design_assist browser_summarize"
classify_live_sessions() {
    BACKGROUND_SOURCES="$BACKGROUND_SOURCES" /usr/bin/python3 -c '
import json, os, sys
background = set(os.environ["BACKGROUND_SOURCES"].split())
rows = json.load(sys.stdin)
live = [r for r in rows if r.get("status") in ("running", "working", "idle")]
held = conn = bg = 0
for r in live:
    meta = r.get("meta") or {}
    if r.get("kind") == "connection":
        conn += 1
    elif meta.get("source") in background:
        bg += 1
    elif ((meta.get("work") or {}).get("origin") or "manual") != "manual":
        bg += 1
    else:
        held += 1
print(len(live), held, conn, bg)
'
}
live_session_counts() {
    local token="${OTTO_API_TOKEN:-${OTTO_MCP_TOKEN:-}}" body
    [[ -n "$token" ]] || return 1
    body="$(curl -fsS --max-time 4 -H "Authorization: Bearer $token" \
        http://127.0.0.1:7700/api/v1/sessions 2>/dev/null)" || return 1
    printf '%s' "$body" | classify_live_sessions 2>/dev/null
}
confirm_session_loss() {
    local counts total held conn bg answer
    if counts="$(live_session_counts)" && [[ -n "$counts" ]]; then
        read -r total held conn bg <<< "$counts"
        echo "==> This deploy restarts the daemon: $total live session(s) visible to this token."
        echo "    $held user agent/shell session(s) survive in PTY holders;" \
             "$conn connection terminal(s) (ssh/db/k8s exec) and $bg background/workflow agent(s) are terminated."
        [[ $((conn + bg)) -gt 0 ]] || return 0
    else
        echo "==> This deploy restarts the daemon (live session count unavailable — no token or daemon down)."
        echo "    User agent and shell sessions survive in PTY holders; connection terminals (ssh/db/k8s exec)" \
             "and background/workflow agents are terminated."
    fi
    [[ "$ASSUME_YES" != 1 && -t 0 ]] || return 0
    read -r -p "    Continue? [y/N] " answer
    case "$answer" in y|Y|yes|YES) return 0 ;; *) echo "aborted — nothing was changed"; return 1 ;; esac
}

# ---- launchd / log hygiene ------------------------------------------------
# Leftover one-shot jobs from the old root deploy.sh self-detach
# (`com.otto.deploy-once.<ts>.<pid>`) stay registered after they exit. Boot out
# only the ones that are no longer running.
sweep_deploy_once_jobs() {
    local label
    for label in $(launchctl list 2>/dev/null | awk '$3 ~ /^com\.otto\.deploy-once\./ {print $3}'); do
        launchctl print "gui/$(id -u)/$label" 2>/dev/null | grep -q 'state = running' && continue
        launchctl bootout "gui/$(id -u)/$label" 2>/dev/null && echo "    removed stale launchd job $label"
    done
    return 0
}
# Keep the newest KEEP_DEPLOY_LOGS deploy logs of each kind.
prune_deploy_logs() {
    local keep="${KEEP_DEPLOY_LOGS:-20}" pattern old
    for pattern in 'deploy-finish-[0-9]*.log' 'deploy-foreground-[0-9]*.log' 'deploy-[0-9]*.log'; do
        while IFS= read -r old; do
            [[ -n "$old" ]] && rm -f "$old"
        done < <(cd "$1" 2>/dev/null && ls -1t $pattern 2>/dev/null | tail -n +$((keep + 1)) | sed "s|^|$1/|")
    done
}

LOG_DIR="$LOG_DIR_DEFAULT"
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
# The EXIT trap also rolls a half-finished install back (finish_cleanup) and
# releases the lock, on every exit path — set -e and signals included.
finish_trap() {
    local rc="$1"
    trap - EXIT
    finish_cleanup "$rc"
    lock_release
    echo "$SENTINEL$rc"
    exit "$rc"
}
if [[ "${OTTO_DEPLOY_PHASE:-}" == "finish" ]]; then
    trap 'finish_trap $?' EXIT   # stamped on every exit path, set -e included
    trap 'exit 143' TERM
    trap 'exit 130' INT
    lock_acquire
    validate_build_receipt
    sleep "${FINISH_DELAY_SECONDS:-0}"
    validate_build_receipt
    finish_phase
    exit 0
fi

# A real deploy (not BUILD_ONLY) holds the lock from the start, so a second
# run can't rebuild the bundle under a finish phase that is installing it,
# and asks before it ends live sessions — up front, not after the build.
if [[ "${BUILD_ONLY:-0}" != "1" ]]; then
    lock_acquire || exit 1
    trap 'lock_release' EXIT
    confirm_session_loss || exit 1
fi

if [[ "${OTTO_DEPLOY_PHASE:-}" != "queue-finish" ]]; then
[[ -z "$(git status --porcelain --untracked-files=normal)" ]] || { fail_verify "build requires a clean source tree"; exit 1; }
BUILD_SOURCE_COMMIT="$(git rev-parse HEAD)"
[[ "${SKIP_UI:-0}" != 1 ]] || { fail_verify "receipted deployment requires a fresh UI build; unset SKIP_UI"; exit 1; }
echo "==> 1/7  Frontend → ui/dist"
# A pull that changed package-lock.json needs `npm ci` first, or the build
# fails on a missing module.
need_ci="$FORCE_CI"
if [[ ! -d ui/node_modules ]] || [[ ui/package-lock.json -nt ui/node_modules/.package-lock.json ]]; then need_ci=1; fi
if [[ "$need_ci" == 1 ]]; then ( cd ui && npm ci ); fi
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
if [[ "$WANT_DMG" == 1 ]]; then
    bash "$HERE/dmg.sh" "$APP" "$APP_SRC/target/release/bundle/Otto.dmg"
fi

# Cargo does not touch Fresh artifacts, so a dependency's age or a newer hash
# cannot prove it is unused. Preserve caches unless disk cleanup is requested.
echo "==> 5/7  Cargo cache policy"
if [[ "${PRUNE:-0}" == "1" ]]; then
    bash "$HERE/prune-target.sh" --apply || echo "    WARN: prune failed — not fatal, the deploy continues."
else
    echo "    Cargo caches preserved (PRUNE=1 opts into eviction; may force recompilation)"
fi

[[ "$(git rev-parse HEAD)" == "$BUILD_SOURCE_COMMIT" ]] || { fail_verify "source commit changed during build"; exit 1; }
build_manifest > "$BUILD_RECEIPT.tmp"
mv "$BUILD_RECEIPT.tmp" "$BUILD_RECEIPT"
echo "BUILD-RECEIPT $BUILD_RECEIPT"
cat "$BUILD_RECEIPT"
if [[ "${BUILD_ONLY:-0}" == "1" ]]; then exit 0; fi
fi
validate_build_receipt

sweep_deploy_once_jobs
prune_deploy_logs "$LOG_DIR"

if [[ "${DETACH:-1}" == "0" ]]; then
    trap 'rc=$?; trap - EXIT; finish_cleanup "$rc"; lock_release; exit "$rc"' EXIT
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
    <key>OTTO_DEPLOY_LOCK_HANDOFF</key><string>$$</string>
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
# with "already bootstrapped" and RunAtLoad would never fire. Clear it first —
# but never a finish phase that is still RUNNING (it would be killed mid-swap;
# the lock normally stops a second deploy long before this point).
if launchctl print "gui/$(id -u)/$FINISH_LABEL" 2>/dev/null | grep -q 'state = running'; then
    fail_verify "$FINISH_LABEL is still running an install — see packaging/deploy.sh --status"
    exit 1
fi
launchctl bootout "gui/$(id -u)/$FINISH_LABEL" 2>/dev/null || true
for _ in $(seq 1 40); do launchctl print "gui/$(id -u)/$FINISH_LABEL" >/dev/null 2>&1 || break; sleep 0.25; done
launchctl bootstrap "gui/$(id -u)" "$FINISH_PLIST"
# The finish job takes the lock over (OTTO_DEPLOY_LOCK_HANDOFF); this shell
# must not release it on exit.
LOCK_HELD=0
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
