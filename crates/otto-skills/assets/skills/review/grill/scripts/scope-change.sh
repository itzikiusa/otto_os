#!/usr/bin/env bash
# Grill — scope the change before the sweep.
#
# Lists changed files by churn vs a base ref, then greps the changed files for
# risky tokens (real file:line) to SEED — never replace — the line-by-line read.
# Every hit is a hint to look at, not a finding; verify or discard each.
#
# Usage: scope-change.sh [base-ref]
#   base-ref defaults to the first of: origin/HEAD, main, master, origin/main,
#   origin/master, else HEAD~1.
set -uo pipefail

base="${1:-}"
if [ -z "$base" ]; then
  for c in origin/HEAD main master origin/main origin/master; do
    if git rev-parse --verify -q "$c" >/dev/null 2>&1; then base="$c"; break; fi
  done
fi
[ -z "$base" ] && base="HEAD~1"

range="$base...HEAD"
git rev-parse --verify -q "$base" >/dev/null 2>&1 || { echo "base ref '$base' not found"; exit 1; }

echo "== base: $base =="
echo
echo "== changed files (churn) =="
git diff --stat "$range" 2>/dev/null || git diff --stat "$base"

echo
echo "== risky tokens in changed files (file:line — HINTS to verify, not findings) =="
files=$(git diff --name-only "$range" 2>/dev/null || git diff --name-only "$base")
# Cross-language smell tokens. Tune per project; absence proves nothing, presence
# is only a place to look.
patterns='unwrap\(\)|expect\(|panic!|unsafe|TODO|FIXME|XXX|HACK|except:|catch *\{ *\}|== *null|!= *null|console\.log|debugger|\.clone\(\)|sleep\(|as!|unwrap_unchecked|# *type: *ignore|// *nolint|eval\('
for f in $files; do
  [ -f "$f" ] || continue
  grep -nHE "$patterns" "$f" 2>/dev/null
done | head -200

echo
echo "(seed only — now sweep EVERY changed line against references/sweep-checklist.md)"
