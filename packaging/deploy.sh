#!/bin/bash
# One-command local deploy of the Otto desktop app (macOS only):
#   rebuild frontend + daemon  →  bundle Tauri app  →  sign (+ ensure cert trust)
#   →  replace /Applications/Otto.app  →  relaunch  →  verify.
#
# There is no Makefile; this just chains the documented steps in docs/RELEASE.md
# so a redeploy is a single command and the signing cert is always trusted (no
# recurring keychain password prompt — see packaging/README.md).
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

echo "==> 6/6  Verify"
healthy=""
for _ in $(seq 1 20); do
    sleep 2
    if curl -fsS --max-time 4 localhost:7700/api/v1/health >/dev/null 2>&1; then
        healthy=1
        break
    fi
done
[[ -n "$healthy" ]] && echo "    daemon healthy: $(curl -s localhost:7700/api/v1/health)" \
                    || echo "    WARN: daemon not healthy yet — check the app/logs."

dep="$HOME/Library/Application Support/Otto/bin/ottod"
if [[ -f "$dep" ]]; then
    a="$(shasum -a 256 "$dep" | awk '{print $1}')"
    b="$(shasum -a 256 "$APP/Contents/MacOS/ottod" | awk '{print $1}')"
    if [[ "$a" == "$b" ]]; then
        echo "    deployed daemon matches the new bundle ✓"
    else
        # Known gotcha: the supervisor copied the new binary but the running
        # process is stale. Force the installed binary to (re)start in one op.
        echo "    deployed daemon is stale — kickstarting the launchd agent…"
        launchctl kickstart -k "gui/$(id -u)/com.otto.daemon" 2>/dev/null || true
    fi
fi
echo "done."
