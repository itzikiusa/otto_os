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
