#!/usr/bin/env bash
# prepare-commit-context.sh — gather the facts needed to write a commit message.
#
# Prints, for the current repo:
#   - branch + any Jira key found in it
#   - whether there are staged changes (what the commit will contain)
#   - the convention signal (recent subjects, emoji-or-not)
#   - a stat of what will be committed
#   - changed files grouped by top-level area  ← SPLIT HINTS, not decisions
#
# These are HINTS, not findings. You decide the type/scope/wording and whether
# to split. Run from inside the repo. Read-only: never stages, commits, or edits.
set -eu

run() { git "$@" 2>/dev/null || true; }

branch=$(run rev-parse --abbrev-ref HEAD)
echo "## Branch"
echo "${branch:-<detached>}"

echo
echo "## Jira key (branch)"
# First [A-Z]+-[0-9]+ token in the branch name.
key=$(printf '%s' "$branch" | grep -oE '[A-Z][A-Z0-9]*-[0-9]+' | head -n1 || true)
if [ -n "${key:-}" ]; then
  echo "$key"
else
  echo "(none in branch — check commit log or ask the user; do NOT invent one)"
fi

echo
echo "## What will be committed"
if ! run diff --cached --quiet; then
  echo "STAGED changes present — the commit will contain exactly these:"
  scope="--cached"
else
  echo "Nothing staged — showing the WORKING tree (stage before committing):"
  scope=""
fi

echo
echo "## Convention signal (recent subjects)"
run log --oneline -20 || echo "(no history yet)"
emoji=$(run log --pretty=%s -20 | grep -cE '^[^[:alnum:][:space:]]' || true)
echo
if [ "${emoji:-0}" -gt 0 ]; then
  echo "-> history appears to use an EMOJI prefix; match it."
else
  echo "-> history appears to be PLAIN conventional (no emoji); match it."
fi

echo
echo "## Stat"
# shellcheck disable=SC2086
run diff $scope --stat

echo
echo "## Split hints — files grouped by top-level area"
echo "(More than one group usually means more than one commit.)"
# shellcheck disable=SC2086
run diff $scope --name-only | awk -F/ '{print ($1=="" ? "." : $1)}' | sort | uniq -c | sort -rn

echo
echo "# Reminder: one concern per commit; Jira key in the SUBJECT when present;"
echo "# no AI attribution / Co-Authored-By / 'Generated with' footer of any kind."
