#!/usr/bin/env bash
# Performance review — seed the sweep with cost hotspot candidates.
#
# Greps the CHANGED files for the textual signatures of the perf patterns this
# lens hunts — query-in-loop / N+1, full-scan / missing-index predicates,
# unbounded queries, blocking I/O, and large copies — and prints real file:line.
#
# These are HINTS, not findings. A grep cannot size `n`, cannot tell a hot path
# from a cold one, and cannot confirm an N+1 — only reading the code can. Every
# hit is a place to look and build a cost model (see references/cost-model-and-
# severity.md); absence of hits proves nothing. Verify or discard each one.
#
# Usage: scan-hotspots.sh [base-ref]
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

git rev-parse --verify -q "$base" >/dev/null 2>&1 || { echo "base ref '$base' not found"; exit 1; }
range="$base...HEAD"

echo "== base: $base =="
files=$(git diff --name-only "$range" 2>/dev/null || git diff --name-only "$base")
[ -z "$files" ] && { echo "no changed files vs $base"; exit 0; }

# Helper: grep changed files for a pattern, label each block. Hits are hints.
scan() {
  local label="$1" pat="$2"
  local out="" f
  for f in $files; do
    [ -f "$f" ] || continue
    out+=$(grep -nHE "$pat" "$f" 2>/dev/null)$'\n'
  done
  out=$(printf '%s' "$out" | grep -v '^$')
  [ -z "$out" ] && return 0
  echo
  echo "== $label (file:line — HINTS, verify each) =="
  printf '%s\n' "$out" | head -60
}

# 1. Data-access calls — anywhere; cross-ref by eye against nearby loops (pass 1).
#    A query/Redis/HTTP call whose line sits inside a for/while is the N+1 signal.
scan "data-access calls (is any inside a loop? → N+1)" \
  'ExecuteSingleResultQuery|ExecuteQuery|GetContext|SelectContext|QueryContext|GetBrandConnector|\.Query\(|\.Exec\(|db\.(Get|Select|Query)|redis|\bHGET\b|\bGET\b|restClient\.|client\.(Get|Post|Do)\('

# 2. Loop constructs — to eyeball whether a data-access call lives inside one.
scan "loops (scan for data-access / linear-search inside)" \
  '\bfor\b|\bwhile\b|\.forEach\(|\.map\(|range '

# 3. Raw SQL predicates that defeat indexes or pull fat rows (pass 2).
scan "SQL: SELECT */non-sargable/leading-wildcard (full-scan risk)" \
  'SELECT \*|LIKE +.%|DATE\(|WHERE +[A-Za-z_]+\(|ORDER BY|GROUP BY|OFFSET '

# 4. Unbounded reads — list/find with no obvious LIMIT (pass 7).
scan "unbounded reads (missing LIMIT / pagination?)" \
  'SELECT .*FROM|\.find\(|\.Find\(|findAll|ListAll|GetAll|fetchAll'

# 5. Migrations — confirm a new predicate ships its index (pass 2).
scan "migration index hooks (does the new predicate get an index?)" \
  'CREATE +(UNIQUE +)?INDEX|ADD +INDEX|KEY +`|ALTER +TABLE'

# 6. Linear lookup in iteration / quadratic smells (pass 3).
scan "linear lookup / quadratic smell (contains/indexOf/concat in a loop)" \
  '\.contains\(|\.indexOf\(|\.includes\(|IndexOf\(|\bin \[|\+= +.|string +\+'

# 7. Large copies and load-all-into-memory (pass 4 & 5).
scan "copies / load-all (clone, to_vec, ReadAll, Marshal round-trip)" \
  '\.clone\(\)|to_vec\(\)|copy\(|ReadAll|ioutil\.ReadAll|json\.(Marshal|Unmarshal)|deepcopy|DeepCopy'

# 8. Blocking / sleeps on a path (pass 6).
scan "blocking calls / sleeps (on a hot path?)" \
  'time\.Sleep|sleep\(|\.lock\(\)|Lock\(\)|\.Wait\(\)|blocking'

echo
echo "(seed only — every changed line still gets read against references/cost-catalogue.md;"
echo " a hit means 'go size n and build a cost model', not 'this is a finding')"
