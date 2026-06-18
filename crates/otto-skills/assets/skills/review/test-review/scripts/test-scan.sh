#!/usr/bin/env bash
# Test Review — scan the change for test-coverage and test-quality SMELLS.
#
# Three deterministic hints to SEED the review — never replace it:
#   1. Changed source files that have NO matching changed test file (coverage risk).
#   2. Changed test files with weak-assertion / mock-only smell tokens (file:line).
#   3. Skipped / ignored / disabled tests shipped in the change (file:line).
#
# Every line printed is a HINT to look at, not a finding. The script cannot tell
# whether a test actually falsifies the code — only a human read + the mutation
# question ("would this fail if I broke the code?") can. Absence of hits proves
# nothing; presence is only a place to look. Verify or discard each.
#
# Usage: test-scan.sh [base-ref]
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

# A path is a "test file" if it matches any of these conventions (JS/TS, Go, Rust,
# Python, Java/Kotlin, Ruby). Tune per project; this is a heuristic, not law.
is_test='(\.test\.|\.spec\.|_test\.go$|_test\.py$|test_.*\.py$|/tests?/|Test\.(java|kt)$|_spec\.rb$|\.tests?\.)'

files=$(git diff --name-only "$range" 2>/dev/null || git diff --name-only "$base")

echo "== base: $base =="
echo
echo "== 1. changed SOURCE files with no changed TEST file (coverage risk — verify) =="
changed_tests=$(printf '%s\n' "$files" | grep -E "$is_test" || true)
had_orphan=0
for f in $files; do
  [ -f "$f" ] || continue                          # skip deletes
  printf '%s\n' "$f" | grep -Eq "$is_test" && continue   # this IS a test file
  case "$f" in
    *.md|*.json|*.yaml|*.yml|*.toml|*.lock|*.txt|*.svg|*.png|*.css|*.html|*.sql|*.sh|*.scss|*.cfg|*.ini|*.env|*.gitignore) continue ;;
  esac
  # crude stem match: does any changed test mention this file's basename stem?
  stem=$(basename "$f" | sed -E 's/\.[^.]+$//')
  if ! printf '%s\n' "$changed_tests" | grep -qiF "$stem"; then
    echo "  $f   (no changed test references '$stem')"
    had_orphan=1
  fi
done
[ "$had_orphan" -eq 0 ] && echo "  (none — every changed source file has a co-changed test by name)"

echo
echo "== 2. weak-assertion / mock-only smells in changed TEST files (file:line — HINTS) =="
# Truthiness oracles, call-not-effect asserts, no-throw-only, mock-element asserts.
weak='toBeTruthy|toBeFalsy|toBeDefined|toBeUndefined|not\.toThrow|assert[A-Za-z]*\(\s*[A-Za-z_][A-Za-z0-9_]*\s*\)|assert True\(|assert result|toHaveBeenCalled\b|\.Called\b|verify\(|-mock|data-testid=.*mock|expect\(true\)'
found_weak=0
for f in $changed_tests; do
  [ -f "$f" ] || continue
  if grep -nHE "$weak" "$f" 2>/dev/null; then found_weak=1; fi
done | head -200
[ "$found_weak" -eq 0 ] && echo "  (no weak-assertion tokens matched — still read every assertion; tokens miss plenty)"

echo
echo "== 3. skipped / ignored / disabled tests in the change (file:line — HINTS) =="
skipped='it\.skip|describe\.skip|test\.skip|xit\(|xdescribe\(|\.only\(|#\[ignore\]|t\.Skip\(|@Disabled|@Ignore|@pytest\.mark\.skip|pytest\.skip|@unittest\.skip'
found_skip=0
for f in $changed_tests; do
  [ -f "$f" ] || continue
  if grep -nHE "$skipped" "$f" 2>/dev/null; then found_skip=1; fi
done | head -100
[ "$found_skip" -eq 0 ] && echo "  (none found)"

echo
echo "(seed only — now read each test against references/coverage-and-cases.md and run the"
echo " falsification mutation per references/falsification.md. The scan finds smells, not gaps.)"
