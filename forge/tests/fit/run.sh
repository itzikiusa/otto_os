#!/bin/sh
# The whole fit suite holds ONE heavy slot: run it as  tools/heavy.sh tests/fit/run.sh  [--mutations]
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export FORGE_HEAVY_HELD=1
exec python3 -B "$here/test_fit.py" "$@"
