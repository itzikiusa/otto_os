#!/bin/bash
# Sign a built Otto.app with the release identity and write the release assets.
# Usage: packaging/release-artifacts.sh <Otto.app> <version> <out-dir> <asset-base-url>
#   asset-base-url: https://github.com/<owner>/<repo>/releases/download/v<version>
# Env: OTTO_SIGN_KEYCHAIN (keychain file holding the identity named in
#      apps/desktop/src-tauri/release-identity.json), TAURI_SIGNING_PRIVATE_KEY
#      (+ TAURI_SIGNING_PRIVATE_KEY_PASSWORD): the updater's minisign key.
# Writes Otto_<v>_aarch64.app.tar.gz (+ .sig), Otto_<v>_aarch64.dmg (+ .sha256),
# latest.json and notes.md. Run from anywhere; CI runs exactly this.
set -euo pipefail

APP="${1:?usage: release-artifacts.sh <Otto.app> <version> <out-dir> <asset-base-url>}"
VERSION="${2:?version}"
OUT="${3:?out-dir}"
BASE_URL="${4:?asset-base-url}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
manifest() { (cd "$ROOT" && python3 -I packaging/release-manifest.py "$@"); }
# Pinned: --app-version needs tauri-cli >= 2.12 (an older cached 2.x rejects it).
TAURI_CLI="@tauri-apps/cli@2.12.6"

mkdir -p "$OUT"
NAME="Otto_${VERSION}_aarch64"
TAR="$OUT/$NAME.app.tar.gz"

OTTO_SIGN_IDENTITY="$(manifest identity)" "$HERE/sign.sh" "$APP"

# Each binary against its OWN identifier and the accepted leaf certs: the same
# check the installed app runs on a staged update.
verify() { # <bundle-or-binary> <identifier>
    codesign --verify --deep --strict -R="$(manifest requirement "$2")" "$1"
}
verify_bundle() {
    verify "$1" com.otto.app
    verify "$1/Contents/MacOS/ottod" ottod
    [[ ! -f "$1/Contents/MacOS/kiri" ]] || verify "$1/Contents/MacOS/kiri" kiri
}
verify_bundle "$APP"
[[ "$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$APP/Contents/Info.plist")" == "$VERSION" ]] \
    || { echo "Info.plist version is not $VERSION" >&2; exit 1; }

# No AppleDouble (._*) entries: every fresh file carries com.apple.provenance,
# and the updater's tar crate would extract ._ files into the bundle and break
# its seal. `tar -t` hides them, so prove it with a literal extraction instead.
(cd "$(dirname "$APP")" && COPYFILE_DISABLE=1 tar --no-mac-metadata --no-xattrs -czf "$TAR" "$(basename "$APP")")
CHECK="$(mktemp -d)"
trap 'rm -rf "$CHECK"' EXIT
python3 -I -c 'import sys, tarfile; tarfile.open(sys.argv[1]).extractall(sys.argv[2], filter="tar")' "$TAR" "$CHECK"
verify_bundle "$CHECK/$(basename "$APP")"

npx --yes "$TAURI_CLI" signer sign --app-version "$VERSION" "$TAR" >/dev/null
base64 --decode < "$TAR.sig" | grep -q $'\tversion:'"$VERSION"'$' \
    || { echo "signature does not carry version:$VERSION" >&2; exit 1; }

"$HERE/dmg.sh" "$APP" "$OUT/$NAME.dmg" >/dev/null
(cd "$OUT" && shasum -a 256 "$NAME.dmg" > "$NAME.dmg.sha256")

manifest notes "$VERSION" > "$OUT/notes.md"
manifest latest-json "$VERSION" "$TAR.sig" "$BASE_URL/$NAME.app.tar.gz" > "$OUT/latest.json"
ls -l "$OUT"
