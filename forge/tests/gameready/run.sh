#!/bin/bash
# Gameready stage tests (heavy: run through the lock):  tools/heavy.sh tests/gameready/run.sh
set -o pipefail
BLENDER="${BLENDER:-/Applications/Blender.app/Contents/MacOS/Blender}"
"$BLENDER" -b --factory-startup --python-exit-code 1 --python "$(cd "$(dirname "$0")" && pwd)/test_gameready.py" 2>&1 \
  | grep -vE '^[0-9:]+ \| (INFO|WARNING)|^(Blender |Read |Info:|Warning:|Fra:|Saved|Time:|Import finished|Blender quit)|DeprecationWarning|^ *$|use_nodes'
