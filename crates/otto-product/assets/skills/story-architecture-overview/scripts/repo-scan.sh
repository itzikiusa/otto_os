#!/usr/bin/env bash
# repo-scan.sh — bootstrap discovery for story architecture scoping
#
# Usage:
#   bash repo-scan.sh [<keyword>]
#
# Prints a read-only survey of the working directory:
#   1. Top-level directory layout
#   2. Build manifests found (go.mod, Cargo.toml, package.json, pyproject.toml)
#   3. Recently changed files (last 30 days)
#   4. Grep / ripgrep hits for <keyword> across source files
#
# Safe: read-only, no writes, no network.
# Runs from any directory; uses the cwd at invocation time.

set -euo pipefail

KEYWORD="${1:-}"
MAX_GREP_RESULTS=60
RECENT_DAYS=30

# ── helpers ──────────────────────────────────────────────────────────────────

hr() { printf '\n%s\n' "──────────────────────────────────────────────────────────────────────"; }

have() { command -v "$1" >/dev/null 2>&1; }

# ── 1. Top-level layout ───────────────────────────────────────────────────────

hr
echo "## Top-level directory layout"
hr

# Show up to two levels deep, excluding common noise dirs
if have tree; then
  tree -L 2 -I '.git|node_modules|target|dist|.next|vendor|__pycache__|.DS_Store' --dirsfirst -a 2>/dev/null | head -80
else
  find . -maxdepth 2 \
    ! -path './.git/*' \
    ! -path './node_modules/*' \
    ! -path './target/*' \
    ! -path './dist/*' \
    ! -path './.next/*' \
    ! -path './vendor/*' \
    -print | sort | head -80
fi

# ── 2. Build manifests ────────────────────────────────────────────────────────

hr
echo "## Build manifests (go.mod, Cargo.toml, package.json, pyproject.toml)"
hr

find . \
  ! -path './.git/*' \
  ! -path './node_modules/*' \
  ! -path './target/*' \
  ! -path './vendor/*' \
  \( \
    -name 'go.mod' \
    -o -name 'Cargo.toml' \
    -o -name 'package.json' \
    -o -name 'pyproject.toml' \
    -o -name 'build.gradle' \
    -o -name 'pom.xml' \
  \) \
  -print | sort | head -40

# ── 3. Recently changed files ─────────────────────────────────────────────────

hr
echo "## Files changed in the last ${RECENT_DAYS} days (source files only)"
hr

find . \
  ! -path './.git/*' \
  ! -path './node_modules/*' \
  ! -path './target/*' \
  ! -path './vendor/*' \
  ! -path './.next/*' \
  -newer "./$(find . -maxdepth 0 -printf '')" \
  -mtime "-${RECENT_DAYS}" \
  \( \
    -name '*.go' \
    -o -name '*.rs' \
    -o -name '*.ts' \
    -o -name '*.tsx' \
    -o -name '*.svelte' \
    -o -name '*.py' \
    -o -name '*.java' \
    -o -name '*.sql' \
    -o -name '*.toml' \
    -o -name '*.yaml' \
    -o -name '*.yml' \
    -o -name '*.json' \
    -o -name '*.md' \
  \) \
  -print 2>/dev/null | sort | head -50

# ── 4. Keyword search ─────────────────────────────────────────────────────────

if [ -n "${KEYWORD}" ]; then
  hr
  echo "## Keyword search: '${KEYWORD}'"
  hr

  EXCLUDE_DIRS='(\.git|node_modules|target|dist|\.next|vendor|__pycache__)'

  if have rg; then
    # ripgrep: fast, respects .gitignore, coloured output
    rg \
      --color=always \
      --heading \
      --line-number \
      --max-count=5 \
      --max-filesize=500K \
      --type-add 'source:*.{go,rs,ts,tsx,svelte,py,java,sql,toml,yaml,yml,md,sh}' \
      --type source \
      --glob "!${EXCLUDE_DIRS}" \
      "${KEYWORD}" \
      . \
      2>/dev/null | head -"${MAX_GREP_RESULTS}" || true
  else
    # fallback: POSIX grep
    grep \
      --color=always \
      -rn \
      --include='*.go' \
      --include='*.rs' \
      --include='*.ts' \
      --include='*.tsx' \
      --include='*.svelte' \
      --include='*.py' \
      --include='*.java' \
      --include='*.sql' \
      --include='*.toml' \
      --include='*.yaml' \
      --include='*.yml' \
      --include='*.md' \
      --include='*.sh' \
      --exclude-dir='.git' \
      --exclude-dir='node_modules' \
      --exclude-dir='target' \
      --exclude-dir='dist' \
      --exclude-dir='vendor' \
      "${KEYWORD}" \
      . \
      2>/dev/null | head -"${MAX_GREP_RESULTS}" || true
  fi
else
  hr
  echo "## Keyword search"
  hr
  echo "(No keyword provided. Re-run with: bash repo-scan.sh <keyword>)"
fi

# ── 5. Summary hint ───────────────────────────────────────────────────────────

hr
echo "## Next steps"
hr
cat <<'EOF'
Use the output above to:
  1. Identify the relevant modules / packages from the layout and manifest list.
  2. Follow the keyword hits to candidate entrypoints and shared types.
  3. Check recently changed files for context on active development nearby.
  4. Re-run with a more specific keyword if the first pass is too broad.

Then open references/codebase-mapping.md for the systematic tracing method.
EOF
