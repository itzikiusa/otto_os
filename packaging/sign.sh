#!/bin/bash
# Sign ottod + Otto.app with the stable self-signed identity.
# Usage: packaging/sign.sh <path-to-Otto.app> [path-to-ottod]
set -euo pipefail

CERT_NAME="Otto Dev Signing"
APP="${1:?usage: sign.sh <Otto.app> [ottod]}"
OTTOD="${2:-}"

# No -v: a self-signed identity that hasn't been trust-approved yet is still
# usable for signing; trust only affects verification UX.
if ! security find-identity -p codesigning | grep -q "$CERT_NAME"; then
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
ensure_codesign_trust

if [[ -n "$OTTOD" ]]; then
    codesign --force --options runtime --timestamp=none -s "$CERT_NAME" "$OTTOD"
    echo "signed: $OTTOD"
fi

# Sign nested sidecar first if present, then the bundle.
SIDECAR="$APP/Contents/MacOS/ottod"
[[ -f "$SIDECAR" ]] && codesign --force --options runtime --timestamp=none -s "$CERT_NAME" "$SIDECAR"

codesign --force --deep --options runtime --timestamp=none -s "$CERT_NAME" "$APP"
codesign -dv "$APP" 2>&1 | grep -E 'Identifier|Authority' || true
echo "signed: $APP"
