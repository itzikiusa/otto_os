#!/bin/bash
# Sign ottod + Otto.app with the stable self-signed identity.
# Usage: packaging/sign.sh <path-to-Otto.app> [path-to-ottod]
#
# OTTO_SIGN_IDENTITY  signing identity (default "Otto Dev Signing", this Mac's
#                     own cert from make-cert.sh). Releases set the release one.
# OTTO_SIGN_KEYCHAIN  sign from this keychain file only (CI: a temporary
#                     keychain holding the release cert). It never needs to be
#                     in the search list, and the login keychain's trust step
#                     is skipped: trust is a property of the user's machine.
set -euo pipefail

CERT_NAME="${OTTO_SIGN_IDENTITY:-Otto Dev Signing}"
KEYCHAIN="${OTTO_SIGN_KEYCHAIN:-}"
APP="${1:?usage: sign.sh <Otto.app> [ottod]}"
OTTOD="${2:-}"

sign() {
    if [[ -n "$KEYCHAIN" ]]; then
        codesign --keychain "$KEYCHAIN" --force --options runtime --timestamp=none -s "$CERT_NAME" "$@"
    else
        codesign --force --options runtime --timestamp=none -s "$CERT_NAME" "$@"
    fi
}

# No -v: a self-signed identity that hasn't been trust-approved yet is still
# usable for signing; trust only affects verification UX.
if ! security find-identity -p codesigning ${KEYCHAIN:+"$KEYCHAIN"} | grep -qF "\"$CERT_NAME\""; then
    echo "Signing identity '$CERT_NAME' not found. Run packaging/make-cert.sh first." >&2
    exit 1
fi

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
[[ -n "$KEYCHAIN" ]] || ensure_codesign_trust

if [[ -n "$OTTOD" ]]; then
    sign "$OTTOD"
    echo "signed: $OTTOD"
fi

# Sign nested sidecar first if present, then the bundle.
SIDECAR="$APP/Contents/MacOS/ottod"
[[ -f "$SIDECAR" ]] && sign "$SIDECAR"

# Optional embedded headless Kiri sidecar: `kiri` requires
# `com.apple.security.virtualization` on its own binary (NOT on Otto.app/ottod).
# `codesign --deep` strips nested entitlements, so when `kiri` is bundled we
# sign the app bundle without `--deep` after signing nested sidecars explicitly.
KIRI_SIDECAR="$APP/Contents/MacOS/kiri"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ -f "$KIRI_SIDECAR" ]]; then
    sign --entitlements "$SCRIPT_DIR/kiri.entitlements" "$KIRI_SIDECAR"
    echo "signed: $KIRI_SIDECAR (with virtualization entitlement)"
    sign "$APP"
else
    sign --deep "$APP"
fi
codesign -dv "$APP" 2>&1 | grep -E 'Identifier|Authority' || true
echo "signed: $APP"
