#!/bin/bash
# Kill ONLY processes started by a load-test run (by its temp root, the
# emulator path, or the pids each run recorded), then remove its temp root.
# Never touches anything else — in particular not the real daemon on :7700.
HERE="$(cd "$(dirname "$0")" && pwd)"
RUNS="${OTTO_LOADTEST_RUNS:-${TMPDIR:-/tmp}/otto-loadtest}"
RUNS="${RUNS%/}"
for f in "$RUNS"/*/pids.json; do
  [ -f "$f" ] || continue
  root=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1])).get('root',''))" "$f")
  for k in daemon browser; do
    pid=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1])).get(sys.argv[2]) or '')" "$f" "$k")
    [ -n "$pid" ] && ps -p "$pid" -o command= 2>/dev/null | grep -qE 'ottod|chrom' && { echo "kill $k $pid"; pkill -9 -P "$pid" 2>/dev/null; kill -9 "$pid" 2>/dev/null; }
  done
  case "$root" in
    */otto-loadtest-*)
      pgrep -f "$root" | while read -r p; do echo "kill root-proc $p"; kill -9 "$p"; done
      [ -d "$root" ] && rm -rf "$root"
      ;;
  esac
done
pgrep -f "$HERE/fake-claude.mjs" | while read -r p; do echo "kill emulator $p"; kill -9 "$p"; done
ps -axo pid,command | grep -E 'otto-loadtest-|fake-claude.mjs' | grep -v grep || echo "clean"
