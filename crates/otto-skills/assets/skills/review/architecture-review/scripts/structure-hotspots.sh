#!/usr/bin/env bash
# Architecture review — list structural hot-spots in the changed files.
#
# A deterministic SEED for the design sweep, not a verdict. It surfaces the
# places most likely to hide size/shape (lens 5) and duplication (lens 6)
# problems, as real file:line. Every line below is a HINT to go read with your
# own eyes and judge against the codebase's own norms — large is not the same
# as bad, and small is not the same as good. Absence here proves nothing.
#
# It is intentionally language-agnostic and approximate: it counts lines and
# braces/indentation, it does not parse. Tune the thresholds per project.
#
# Usage: structure-hotspots.sh [base-ref]
#   base-ref defaults to the first of: origin/HEAD, main, master, origin/main,
#   origin/master, else HEAD~1.
set -uo pipefail

# --- thresholds (tune per project) -----------------------------------------
FILE_LINES=400        # files longer than this are worth a cohesion look
FUNC_LINES=60         # function/block runs longer than this may do too much
NEST_COLS=24          # leading-whitespace columns ~ deep nesting (arrow code)
DUP_MIN_LEN=12        # min trimmed line length to consider for duplicate scan
DUP_MIN_HITS=4        # a line repeated >= this many times across changed files

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
echo "(HINTS only — read each spot and judge shape against the codebase's own modules)"
echo

files=$(git diff --name-only "$range" 2>/dev/null || git diff --name-only "$base")
# Keep likely source files; skip lockfiles, generated, binaries, vendored.
srcfiles=""
for f in $files; do
  [ -f "$f" ] || continue
  case "$f" in
    *.lock|*.min.*|*.map|*.svg|*.png|*.jpg|*.pdf|*.snap) continue ;;
    */vendor/*|*/node_modules/*|*/dist/*|*/build/*|*/.git/*|*/target/*) continue ;;
  esac
  case "$f" in
    *.rs|*.ts|*.tsx|*.js|*.jsx|*.go|*.py|*.java|*.kt|*.rb|*.c|*.cc|*.cpp|*.h|*.hpp|*.cs|*.swift|*.php|*.scala|*.svelte)
      srcfiles="$srcfiles $f" ;;
  esac
done

if [ -z "${srcfiles// }" ]; then
  echo "No reviewable source files changed against $base."
  exit 0
fi

# --- 1. Large files (lens 1 cohesion / lens 5 size) -------------------------
echo "== large files (> ${FILE_LINES} lines — candidate junk-drawers / low cohesion) =="
for f in $srcfiles; do
  n=$(wc -l < "$f" | tr -d ' ')
  [ "$n" -gt "$FILE_LINES" ] && printf '  %6d  %s\n' "$n" "$f"
done | sort -rn
echo

# --- 2. Long function/block runs (lens 5 size & shape) ----------------------
# Heuristic: a run of >= FUNC_LINES non-blank lines all at the SAME or deeper
# indentation than a line that looks like a function/method opener. Approximate
# — flags the opener line so you can read the body.
echo "== long function/block openers (body may run > ${FUNC_LINES} lines — does it do one thing?) =="
for f in $srcfiles; do
  awk -v MIN="$FUNC_LINES" '
    function flush(   l) {
      if (open != "" && (NR_cur - open_line) >= MIN)
        printf "  %5d lines  %s:%d  %s\n", (NR_cur - open_line), FILENAME, open_line, open
    }
    {
      line=$0; NR_cur=NR
      # crude opener detection across languages. Require a real declaration
      # keyword followed by a name+paren, so we anchor on functions/methods —
      # not closing braces, CSS selectors, or random if/catch lines.
      if (line ~ /(^|[ \t])(func|fn|def|function|sub|fun)[ \t]+[A-Za-z_]/ ||
          line ~ /(public|private|protected|static|async|override)[ \t].*[A-Za-z_][A-Za-z0-9_]*[ \t]*\(.*\)[ \t]*\{?[ \t]*$/) {
        flush(); open=line; gsub(/^[ \t]+/,"",open); open_line=NR
      }
    }
    END { NR_cur=NR+1; flush() }
  ' "$f"
done | sort -rn | head -40
echo

# --- 3. Deep nesting (lens 5 arrow code) ------------------------------------
echo "== deeply-indented lines (>= ${NEST_COLS} cols of leading whitespace — arrow code) =="
for f in $srcfiles; do
  awk -v COLS="$NEST_COLS" '
    { match($0, /^[ \t]*/); lead=RLENGTH
      # count a tab as 4 cols
      t=gsub(/\t/,"",$0); lead = lead - t + t*4
      if (lead >= COLS && $0 !~ /^[ \t]*$/) printf "  col%-4d %s:%d\n", lead, FILENAME, NR
    }
  ' "$f"
done | head -30
echo

# --- 4. Duplicate-line candidates (lens 6 duplication) ----------------------
# Non-trivial trimmed lines that repeat across the changed files. Catches
# copy-paste blocks and redefined constants. Trivial/structural lines filtered.
echo "== repeated non-trivial lines (>= ${DUP_MIN_HITS}x — copy-paste / redefinition?) =="
for f in $srcfiles; do
  awk -v MINLEN="$DUP_MIN_LEN" '
    { s=$0; gsub(/^[ \t]+|[ \t]+$/,"",s)
      if (length(s) < MINLEN) next
      if (s ~ /^[\}\)\]\{,;]+$/) next             # pure brackets/punctuation
      if (s ~ /^(import|from|use|#include|package|require|using)\b/) next
      if (s ~ /^(\/\/|#|\*|\/\*|--)/) next         # comments
      print s
    }
  ' "$f"
done | sort | uniq -c | sort -rn | awk -v H="$DUP_MIN_HITS" '$1 >= H { print "  "$0 }' | head -25
echo "  (to locate a repeated line: grep -nF '<the line>' <changed files>)"
echo

echo "== reminder =="
echo "These are seeds for lenses 5 (size/shape) and 6 (duplication). Now run the"
echo "full sweep in references/review-lenses.md — most design findings (coupling,"
echo "wrong seam, leaky abstraction, dependency direction) are NOT line-countable"
echo "and will only surface by reading the change against its neighbours."
