#!/bin/sh
# Otto update finisher. Written by the app (updater/finish.rs) and run ONCE by
# launchd as `com.otto.update-finish`, outside the app's and the daemon's
# process trees, so it outlives both restarting:
#   wait for the old app to quit → atomic swap (renamex_np RENAME_SWAP: the
#   app path is never missing) → open the new app → poll /api/v1/meta for the
#   new version → keep it (the old app becomes the one rollback copy) or swap
#   back, reopen the old app and record why.
# The journal (finish.state: "<pid> <phase> <from> <to>") lets the next app
# launch finish an interrupted run (finish.rs `recover`).
# Never quits an app by name: only the pid it was given and the process whose
# executable is this exact bundle.
set -u
APP="$OTTO_APP"; SLOT="$OTTO_SLOT"; PREV="$OTTO_PREVIOUS"; FROM="$OTTO_FROM"; TO="$OTTO_TO"
STATE="$OTTO_UPDATES/finish.state"; RESULT="$OTTO_UPDATES/last-result.json"
OPEN="${OTTO_OPEN:-/usr/bin/open}"; QUIT_SECS="${OTTO_QUIT_SECS:-30}"; VERIFY_SECS="${OTTO_VERIFY_SECS:-120}"

log() { echo "$(date '+%Y-%m-%d %H:%M:%S') $*"; }
journal() { printf '%s %s %s %s\n' "$$" "$1" "$FROM" "$TO" > "$STATE.tmp" && mv -f "$STATE.tmp" "$STATE"; }
# result <ok> <skip> <reason>: the outcome the next launch shows; ends the journal.
result() {
    printf '{"ok":%s,"skip":%s,"from":"%s","to":"%s","reason":"%s","log":"%s"}\n' \
        "$1" "$2" "$FROM" "$TO" "$3" "${OTTO_LOG:-}" > "$RESULT.tmp" && mv -f "$RESULT.tmp" "$RESULT"
    rm -f "$STATE"
    log "RESULT ok=$1: $3"
}
swap() {
    [ "$(/usr/bin/osascript -l JavaScript -e 'function run(a){ObjC.import("stdio");return $.renamex_np(a[0],a[1],2)===0?"swapped":"failed"}' "$1" "$2" 2>&1)" = swapped ]
}
app_pid() {
    for p in $(pgrep -x otto-desktop); do
        [ "$(ps -p "$p" -o comm= | sed 's/^ *//')" = "$APP/Contents/MacOS/otto-desktop" ] && { echo "$p"; return 0; }
    done
    return 1
}
# wait_gone <pid> <secs>
wait_gone() {
    n=0
    while kill -0 "$1" 2>/dev/null; do
        [ "$n" -ge $(( $2 * 2 )) ] && return 1
        sleep 0.5; n=$((n + 1))
    done
}
stop_app() {
    p="$(app_pid)" || return 0
    kill -TERM "$p" 2>/dev/null
    wait_gone "$p" 10 || { kill -KILL "$p" 2>/dev/null; wait_gone "$p" 5; }
}
meta_version() {
    curl -fsS --max-time 3 "$OTTO_META_URL" 2>/dev/null | sed -n 's/.*"version":"\([^"]*\)".*/\1/p'
}

journal waiting
log "waiting for Otto $FROM (pid $OTTO_APP_PID) to quit"
if ! wait_gone "$OTTO_APP_PID" "$QUIT_SECS"; then
    kill -TERM "$OTTO_APP_PID" 2>/dev/null
    if ! wait_gone "$OTTO_APP_PID" "$QUIT_SECS"; then
        result false false "Otto did not quit, so the update was not installed. Nothing changed."
        exit 1
    fi
fi
if ! swap "$SLOT" "$APP"; then
    result false false "Otto could not be replaced in $(dirname "$APP"). Nothing changed."
    exit 1
fi
journal swapped
log "Otto $TO is in place; opening it"
"$OPEN" "$APP" || log "open failed"

why="the new version did not start"
end=$(( $(date +%s) + VERIFY_SECS ))
while [ "$(date +%s)" -lt "$end" ]; do
    if app_pid > /dev/null; then
        v="$(meta_version)"
        if [ "$v" = "$TO" ]; then
            rm -rf "$PREV"; mkdir -p "$(dirname "$PREV")"
            mv "$SLOT" "$PREV" || rm -rf "$SLOT"
            result true false "Updated to $TO."
            exit 0
        fi
        why="the daemon did not report version $TO (last answer: ${v:-none})"
    fi
    sleep 2
done

log "FAILED: $why — putting $FROM back"
stop_app
if swap "$SLOT" "$APP"; then
    journal rolled_back
    rm -rf "$SLOT"
    "$OPEN" "$APP" || log "open failed"
    result false true "Otto $TO did not come up within $VERIFY_SECS s ($why), so the update was undone and $FROM is back."
else
    result false true "Otto $TO did not come up ($why) and $FROM could not be put back automatically. It is at $SLOT."
fi
exit 1
