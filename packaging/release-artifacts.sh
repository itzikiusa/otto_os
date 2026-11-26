#!/bin/bash
# The release assets, in two phases so the two private keys are never on the
# machine at the same time (.github/workflows/release.yml runs exactly this):
#
#   release-artifacts.sh bundle <Otto.app> <version> <out-dir>
#       Env OTTO_SIGN_KEYCHAIN: keychain file holding the identity named in
#       apps/desktop/src-tauri/release-identity.json (unset: the login
#       keychain). Env for Apple's notary service: OTTO_NOTARY_PROFILE (a
#       `notarytool store-credentials` profile, on the owner's Mac) or
#       OTTO_NOTARY_KEY (an App Store Connect API key file) + OTTO_NOTARY_KEY_ID
#       + OTTO_NOTARY_ISSUER (CI). Signs, verifies, notarizes and staples the
#       app, writes Otto_<v>_aarch64.app.tar.gz, the DMG (signed, notarized,
#       stapled) + .sha256, notes.md.
#   release-artifacts.sh update <version> <out-dir> <asset-base-url>
#       Env TAURI_SIGNING_PRIVATE_KEY (+ _PASSWORD): the updater's minisign key.
#       Signs the archive bound to <version> and writes latest.json.
#       asset-base-url: https://github.com/<owner>/<repo>/releases/download/v<version>
#
# Both refuse the TEST identity / TEST update key the repo carries until the
# owner's real ones replace them (release-manifest.py check-release-config).
set -euo pipefail

PHASE="${1:-}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
manifest() { (cd "$ROOT" && python3 -I packaging/release-manifest.py "$@"); }
manifest check-release-config

case "$PHASE" in
bundle)
    APP="${2:?usage: release-artifacts.sh bundle <Otto.app> <version> <out-dir>}"
    VERSION="${3:?version}"
    OUT="${4:?out-dir}"
    ;;
update)
    VERSION="${2:?usage: release-artifacts.sh update <version> <out-dir> <asset-base-url>}"
    OUT="${3:?out-dir}"
    BASE_URL="${4:?asset-base-url}"
    ;;
*)
    echo "usage: release-artifacts.sh bundle|update …" >&2
    exit 2
    ;;
esac
NAME="Otto_${VERSION}_aarch64"
TAR="$OUT/$NAME.app.tar.gz"

if [[ "$PHASE" == update ]]; then
    # The Tauri CLI from packaging/signer's lockfile (npm ci --ignore-scripts);
    # --app-version needs >= 2.12.
    "${OTTO_TAURI_CLI:-$HERE/signer/node_modules/.bin/tauri}" signer sign --app-version "$VERSION" "$TAR" >/dev/null
    base64 --decode < "$TAR.sig" | grep -q $'\tversion:'"$VERSION"'$' \
        || { echo "signature does not carry version:$VERSION" >&2; exit 1; }
    manifest latest-json "$VERSION" "$TAR.sig" "$BASE_URL/$NAME.app.tar.gz" > "$OUT/latest.json"
    ls -l "$OUT"
    exit 0
fi

mkdir -p "$OUT"
OTTO_SIGN_IDENTITY="$(manifest identity)"
export OTTO_SIGN_IDENTITY
"$HERE/sign.sh" "$APP"

# Each binary against its OWN identifier and the accepted team: the same
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

# Apple notarizes <file>; its ticket is stapled to <target>, so the first open
# passes Gatekeeper offline too. A rejected submission prints Apple's log.
notarize() { # <file-to-submit> <app-or-dmg-to-staple>
    local auth out id
    if [[ -n "${OTTO_NOTARY_PROFILE:-}" ]]; then
        auth=(--keychain-profile "$OTTO_NOTARY_PROFILE")
    else
        auth=(--key "${OTTO_NOTARY_KEY:?OTTO_NOTARY_PROFILE or OTTO_NOTARY_KEY}"
            --key-id "${OTTO_NOTARY_KEY_ID:?}" --issuer "${OTTO_NOTARY_ISSUER:?}")
    fi
    out="$(xcrun notarytool submit "$1" --wait "${auth[@]}" 2>&1)" || true
    if ! grep -q 'status: Accepted' <<<"$out"; then
        echo "$out" >&2
        id="$(awk '$1 == "id:" { print $2; exit }' <<<"$out")"
        [[ -z "$id" ]] || xcrun notarytool log "$id" "${auth[@]}" >&2 || true
        exit 1
    fi
    # The ticket can take some seconds to reach Apple's CDN after "Accepted".
    xcrun stapler staple "$2" || { sleep 20 && xcrun stapler staple "$2"; }
}
CHECK="$(mktemp -d)"
trap 'rm -rf "$CHECK"' EXIT
ditto -c -k --keepParent "$APP" "$CHECK/notarize.zip"
notarize "$CHECK/notarize.zip" "$APP"
rm -f "$CHECK/notarize.zip"

# The update archive, from the stapled app.
# No AppleDouble (._*) entries: every fresh file carries com.apple.provenance,
# and the updater's tar crate would extract ._ files into the bundle and break
# its seal. `tar -t` hides them, so prove it with a literal extraction instead.
(cd "$(dirname "$APP")" && COPYFILE_DISABLE=1 tar --no-mac-metadata --no-xattrs -czf "$TAR" "$(basename "$APP")")
python3 -I -c 'import sys, tarfile; tarfile.open(sys.argv[1]).extractall(sys.argv[2], filter="tar")' "$TAR" "$CHECK"
verify_bundle "$CHECK/$(basename "$APP")"
xcrun stapler validate -q "$CHECK/$(basename "$APP")"

"$HERE/dmg.sh" "$APP" "$OUT/$NAME.dmg" >/dev/null
"$HERE/sign.sh" "$OUT/$NAME.dmg"
notarize "$OUT/$NAME.dmg" "$OUT/$NAME.dmg"
(cd "$OUT" && shasum -a 256 "$NAME.dmg" > "$NAME.dmg.sha256")
manifest notes "$VERSION" > "$OUT/notes.md"
