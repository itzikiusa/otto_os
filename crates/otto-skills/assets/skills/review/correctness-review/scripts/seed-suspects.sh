#!/usr/bin/env bash
# Correctness review — seed the suspect list before tracing.
#
# Greps the CHANGED lines for tokens that correlate with the correctness bug
# classes (boundary arithmetic, null/absence, unhandled branches, lossy
# conversion, copy-paste indices), grouped by class so each hit maps to a pass
# in references/correctness-hunt-list.md.
#
# Every hit is a place to LOOK, not a finding. A correctness bug cannot be
# confirmed by grep — you still owe a hand-trace or a repro (references/
# trace-and-reproduce.md) before any hit becomes a finding. Absence here proves
# nothing: the worst correctness bugs (an inverted condition, a wrong invariant)
# match no token at all. This only saves you the first scan; it never replaces
# reading every changed line.
#
# Usage: seed-suspects.sh [base-ref]
#   base-ref defaults to the first existing of: origin/HEAD, main, master,
#   origin/main, origin/master, else HEAD~1.
set -uo pipefail

base="${1:-}"
if [ -z "$base" ]; then
  for c in origin/HEAD main master origin/main origin/master; do
    if git rev-parse --verify -q "$c" >/dev/null 2>&1; then base="$c"; break; fi
  done
fi
[ -z "$base" ] && base="HEAD~1"

git rev-parse --verify -q "$base" >/dev/null 2>&1 || { echo "base ref '$base' not found"; exit 1; }
range="$base...HEAD"

echo "== base: $base =="
files=$(git diff --name-only "$range" 2>/dev/null || git diff --name-only "$base")
[ -z "$files" ] && { echo "no changed files vs $base"; exit 0; }

# scan: <label> <extended-regex>  — prints "file:line: match" for each hit.
scan() {
  local label="$1" pat="$2" any=0
  for f in $files; do
    [ -f "$f" ] || continue
    grep -nHE "$pat" "$f" 2>/dev/null && any=1
  done | sed 's/^/  /'
  return 0
}

section() {
  echo
  echo "== $1 — hint: $2 =="
  scan "$1" "$3"
}

# 2. Off-by-one & boundaries — index math, range ends, length arithmetic.
section "boundaries / off-by-one" \
  "look for the first/last element, empty, exactly-at-limit" \
  '\[[^]]*[+-] *1\]|\.length *[+-]|len\([^)]*\) *[+-]|<= *[a-zA-Z_]|>= *[a-zA-Z_]|\.\.=?|[iI]ndex *[+-]'

# 3. Null / None / nil / absent — deref and absence handling.
section "null / none / nil / absent" \
  "trace the path where the value IS absent" \
  'unwrap\(\)|expect\(|\bnil\b|\bNone\b|null|undefined|\?\.|!\.|\bget\(|\.find\(|Optional|\.ok_or|panic!'

# 4. Branches & cases — the path the author may not have run.
section "branches / unhandled cases" \
  "did the else / default / missing arm get exercised?" \
  '\belse\b|default:|switch *\(|match |case |\?\s*[^:]+:|=> *\{|return;|continue;|break;'

# 6. Data & math — lossy conversion, rounding, float equality, sign.
section "data / math / conversion" \
  "is the conversion lossy? rounding direction? unit/scale?" \
  '\bas i32\b|\bas i64\b|\bas u[0-9]+\b|\bas f[0-9]+\b|\bint\(|\bfloat\(|\(int\)|\(long\)|parseInt|parseFloat|Math\.(floor|round|ceil)|/ *[0-9]|% |== *[0-9.]'

# 7. Copy-paste & stale references — loop indices, duplicated blocks.
section "copy-paste / stale index" \
  "does the pasted block still use the old variable / index?" \
  'for *\(|for [a-zA-Z_]+ in|\.forEach|\bi\+\+|\bj\+\+|enumerate\('

echo
echo "(seed only — every hit is a place to LOOK. Now trace each suspect against the"
echo " intended behavior; no trace and no repro → not a finding. References:"
echo " correctness-hunt-list.md · trace-and-reproduce.md)"
