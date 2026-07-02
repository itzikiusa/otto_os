#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <diagram.d2> [layout] [format]" >&2
  echo "Example: $0 docs/diagrams/system-context.d2 dagre svg" >&2
  exit 2
fi

INPUT="$1"
LAYOUT="${2:-dagre}"
FORMAT="${3:-svg}"

if [[ ! -f "$INPUT" ]]; then
  echo "D2 input file not found: $INPUT" >&2
  exit 2
fi

if ! command -v d2 >/dev/null 2>&1; then
  echo "D2 CLI is not installed or not on PATH." >&2
  echo "Install: go install oss.terrastruct.com/d2@latest" >&2
  exit 127
fi

DIR=$(dirname "$INPUT")
BASE=$(basename "$INPUT" .d2)
OUTPUT="$DIR/$BASE.$FORMAT"

# Format in-place when supported. If fmt behavior differs by version, do not fail the whole render.
if d2 fmt "$INPUT" >/tmp/d2_fmt.out 2>/tmp/d2_fmt.err; then
  :
else
  echo "Warning: d2 fmt failed or is unsupported by this version; continuing to render." >&2
  cat /tmp/d2_fmt.err >&2 || true
fi

d2 --layout="$LAYOUT" "$INPUT" "$OUTPUT"
echo "$OUTPUT"
