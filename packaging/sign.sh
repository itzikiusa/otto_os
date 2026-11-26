#!/bin/bash
# Sign ottod + Otto.app with the stable self-signed identity, or a release
# with the Developer ID one.
# Usage: packaging/sign.sh <path-to-Otto.app> [path-to-ottod]
#        packaging/sign.sh <path-to.dmg>
#
# OTTO_SIGN_IDENTITY  signing identity (default "Otto Dev Signing", this Mac's
#                     own cert from make-cert.sh). Releases set
#                     "Developer ID Application: …", which also gets Apple's
#                     secure timestamp (notarization requires it).
# OTTO_SIGN_KEYCHAIN  sign from this keychain file only (CI: a temporary
#                     keychain holding the release cert). It never needs to be
#                     in the search list, and the login keychain's trust step
#                     is skipped: trust is a property of the user's machine.
set -euo pipefail

CERT_NAME="${OTTO_SIGN_IDENTITY:-Otto Dev Signing}"
KEYCHAIN="${OTTO_SIGN_KEYCHAIN:-}"
APP="${1:?usage: sign.sh <Otto.app> [ottod] | <file.dmg>}"
OTTOD="${2:-}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Developer ID: Apple's secure timestamp, which notarization requires. The
# self-signed dev identity signs offline.
DEVELOPER_ID=""
TIMESTAMP=--timestamp=none
if [[ "$CERT_NAME" == "Developer ID Application:"* ]]; then
    DEVELOPER_ID=1
    TIMESTAMP=--timestamp
fi

# codesign gets the certificate's SHA-1, not its name: a name is not found in
# a keychain outside the search list (CI's temporary one).
# Developer ID: valid identities only (-v), so a renewed certificate is picked
# over the expired one of the same name. Dev identity, no -v: a self-signed
# one that hasn't been trust-approved yet still signs.
CERT_HASH="$(security find-identity ${DEVELOPER_ID:+-v} -p codesigning ${KEYCHAIN:+"$KEYCHAIN"} |
    awk -v name="\"$CERT_NAME\"" 'index($0, name) { print $2; exit }')"
if [[ -z "$CERT_HASH" ]]; then
    echo "Signing identity '$CERT_NAME' not found in ${KEYCHAIN:-the keychain search list}." \
        "Dev identity: run packaging/make-cert.sh. Release identity: docs/RELEASE.md." >&2
    exit 1
fi

sign() {
    codesign ${KEYCHAIN:+--keychain "$KEYCHAIN"} --force --options runtime \
        "$TIMESTAMP" -s "$CERT_HASH" "$@"
}

# Ensure the cert is TRUSTED for code signing. This is what makes a keychain
# "Always Allow" survive a rebuild: when trusted, macOS anchors a STABLE code
# identity (identifier "ottod" + this cert) instead of pinning each build's
# signature — otherwise every re-sign invalidates the grant and the login-
# keychain password prompt returns on every rebuild. `make-cert.sh` attempts
# this once, but its admin prompt can be dismissed/silently fail; re-asserting
# it here (idempotently) keeps it from coming back. See packaging/README.md.
ensure_codesign_trust() {
    # Already trusted in either the user or admin domain → nothing to do, no prompt.
    if security dump-trust-settings 2>/dev/null | grep -qF "$CERT_NAME" \
       || security dump-trust-settings -d 2>/dev/null | grep -qF "$CERT_NAME"; then
        return 0
    fi
    echo "Trusting '$CERT_NAME' for code signing (one-time; a password prompt may appear)…"
    local pem
    pem="$(mktemp -t otto-dev-signing)" || return 0
    if ! security find-certificate -c "$CERT_NAME" -p > "$pem" 2>/dev/null; then
        rm -f "$pem"
        echo "WARN: could not export '$CERT_NAME' to trust it; keychain prompts may recur." >&2
        return 0
    fi
    if security add-trusted-cert -r trustRoot -p codeSign \
         -k "$HOME/Library/Keychains/login.keychain-db" "$pem" 2>/dev/null; then
        echo "trusted: $CERT_NAME (code signing)"
    else
        echo "WARN: could not add trust automatically. Fix once in Keychain Access:" >&2
        echo "      login → '$CERT_NAME' → Trust → Code Signing: Always Trust." >&2
    fi
    rm -f "$pem"
}
# Apple's chain already trusts a Developer ID certificate.
[[ -n "$KEYCHAIN" || -n "$DEVELOPER_ID" ]] || ensure_codesign_trust

if [[ "$APP" == *.dmg ]]; then
    sign "$APP"
    echo "signed: $APP"
    exit 0
fi

if [[ -n "$OTTOD" ]]; then
    sign "$OTTOD"
    echo "signed: $OTTOD"
fi

# Sign nested sidecar first if present, then the bundle.
SIDECAR="$APP/Contents/MacOS/ottod"
[[ -f "$SIDECAR" ]] && sign "$SIDECAR"

# Optional embedded headless Kiri sidecar: `kiri` requires
# `com.apple.security.virtualization` on its own binary (NOT on Otto.app/ottod).
KIRI_SIDECAR="$APP/Contents/MacOS/kiri"
if [[ -f "$KIRI_SIDECAR" ]]; then
    sign --entitlements "$SCRIPT_DIR/kiri.entitlements" "$KIRI_SIDECAR"
    echo "signed: $KIRI_SIDECAR (with virtualization entitlement)"
fi
# No --deep: it would put the app's entitlements on the sidecars and strip
# kiri's. Every nested binary is signed above.
sign --entitlements "$SCRIPT_DIR/otto.entitlements" "$APP"
codesign -dv "$APP" 2>&1 | grep -E 'Identifier|Authority' || true
echo "signed: $APP"
