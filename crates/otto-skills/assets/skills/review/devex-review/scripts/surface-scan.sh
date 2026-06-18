#!/usr/bin/env bash
# Devex review — seed the surface walk.
#
# Greps the changed files for two devex-relevant token classes and prints them
# as real file:line HINTS to look at — never findings:
#   1) NEW PUBLIC SURFACE added in the diff (added `+` lines only): exported
#      fns/types, route declarations, CLI flag definitions, config keys. Each is
#      a surface to judge for naming, defaults, and docs.
#   2) EMITTED-ERROR sites (whole changed files): panic/throw/Err/raise and bare
#      HTTP error responses. Each is a message to read against the three-tier
#      model (what / why / what-next) in references/dx-principles.md.
#
# Every hit is a place to LOOK, not a verdict. Absence proves nothing (a footgun
# can be a perfectly ordinary signature); presence is only a starting point. The
# real review is walking each surface as the caller — this just finds the surfaces.
#
# Limits: regex, not a parser. It cannot tell a good error from a bad one, a safe
# default from a footgun, or a documented surface from an undocumented one — that
# is your job. It does not see whether a signature has same-typed adjacent params
# (read those by eye). Tune the patterns per language/project.
#
# Usage: surface-scan.sh [base-ref]
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
echo
echo "== changed files (churn) =="
git diff --stat "$range" 2>/dev/null || git diff --stat "$base"

files=$(git diff --name-only "$range" 2>/dev/null || git diff --name-only "$base")

# --- 1) NEW PUBLIC SURFACE (added lines only) -------------------------------
# Scan the unified diff and keep only added lines (+, not +++ header). These are
# surfaces this change introduces — judge each for naming / defaults / docs.
echo
echo "== new public surface added in this diff (HINTS — judge each for naming, defaults, docs) =="
surface='(^\+.*\b(pub fn|pub struct|pub enum|pub trait|pub type|pub const)\b)|'         # rust
surface+='(^\+.*\bexport (default |const |function |class |interface |type |enum )\b)|' # ts/js
surface+='(^\+.*\bdef [a-zA-Z_][a-zA-Z0-9_]*\()|'                                       # python (filter dunder by eye)
surface+='(^\+.*\bfunc [A-Z][A-Za-z0-9_]*\()|'                                          # go exported (Capitalized)
surface+='(^\+.*\b(public|protected)\s+\w.*\()|'                                        # java/c#/kotlin
surface+='(^\+.*\.(route|add_route|HandleFunc)\s*\()|'                                  # http routes (router-level)
surface+='(^\+.*\b(get|post|put|patch|delete)\s*\(\s*["'"'"'/])|'                       # http routes by path literal: get("/...")
surface+='(^\+.*#\[(get|post|put|patch|delete)\()|'                                     # rust attr routes: #[post("/...")]
surface+='(^\+.*(add_argument|addOption|\.option\(|\.flag\(|StringVar|BoolVar|clap::Arg))' # cli flags
git diff "$range" 2>/dev/null -- $files | grep -nE "$surface" | head -120 || echo "(none detected — may still have surface the regex missed; read the diff)"

# --- 2) EMITTED-ERROR SITES (whole changed files) ---------------------------
# The messages a caller/on-call reads when it fails. Read each against the
# three-tier model: does it say WHAT went wrong, WHY/which input, and WHAT NEXT?
echo
echo "== emitted-error sites in changed files (HINTS — read each against the 3-tier model) =="
err='panic!\(|panic\(|throw new |throw |raise |Err\(|\.expect\(|fmt\.Errorf\(|errors\.New\('
err+='|abort\(|HttpException|BadRequest|res\.status\(|reply\.code\(|\.send_error|JsonResponse.*status'
for f in $files; do
  [ -f "$f" ] || continue
  grep -nHE "$err" "$f" 2>/dev/null
done | head -150 || true

echo
echo "(seed only — now walk EVERY surface above as the caller, per references/ergonomics-hunt-list.md;"
echo " read EVERY error message against the 3-tier model in references/dx-principles.md.)"
