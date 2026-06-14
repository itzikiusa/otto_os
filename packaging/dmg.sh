#!/bin/bash
# Build a DMG from a signed Otto.app.
# Usage: packaging/dmg.sh <path-to-Otto.app> [out.dmg]
set -euo pipefail

APP="${1:?usage: dmg.sh <Otto.app> [out.dmg]}"
OUT="${2:-Otto.dmg}"

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

hdiutil create -volname "Otto" -srcfolder "$STAGE" -ov -format UDZO "$OUT"
echo "created: $OUT"
