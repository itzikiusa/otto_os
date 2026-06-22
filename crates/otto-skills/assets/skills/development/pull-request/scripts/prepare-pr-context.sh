#!/usr/bin/env bash
# prepare-pr-context.sh — gather the facts needed to write & open a PR.
#
# Usage:  bash scripts/prepare-pr-context.sh [target-branch]
#         target-branch defaults to origin/HEAD's branch, else main, else master.
#
# Prints: source/target branches, the Jira key, the detected host + a ready
# create-command skeleton, ALL commits on the branch, and a diffstat.
# Read-only: never pushes, commits, or opens anything. Hints, not decisions.
set -eu

run() { git "$@" 2>/dev/null || true; }

source_branch=$(run rev-parse --abbrev-ref HEAD)

# Resolve target: arg, else origin default branch, else main, else master.
target="${1:-}"
if [ -z "$target" ]; then
  target=$(run symbolic-ref --short refs/remotes/origin/HEAD | sed 's#^origin/##')
  [ -z "$target" ] && run show-ref --verify --quiet refs/heads/main && target=main
  [ -z "$target" ] && run show-ref --verify --quiet refs/heads/master && target=master
  [ -z "$target" ] && target=main
fi

echo "## Branches"
echo "source: ${source_branch:-<detached>}"
echo "target: $target"

echo
echo "## Jira key (branch) — goes in the PR TITLE only, never the body"
key=$(printf '%s' "$source_branch" | grep -oE '[A-Z][A-Z0-9]*-[0-9]+' | head -n1 || true)
if [ -n "${key:-}" ]; then
  echo "$key"
else
  echo "(none in branch — check commits or ask; do NOT invent one or a Jira URL)"
fi

echo
echo "## Host"
remote=$(run remote get-url origin)
echo "remote: ${remote:-<none>}"
case "$remote" in
  *github.com*)
    echo "host: github  ->  gh pr create --base $target --head $source_branch \\"
    echo "                     --title \"${key:+$key }<summary>\" --body-file <body.md>"
    ;;
  *bitbucket.org*)
    echo "host: bitbucket  ->  build JSON with python3 + POST via curl (see"
    echo "     assets/pr-description-template.md). Needs \$BITBUCKET_TOKEN."
    echo "     Escape \\( \\) \\_ in the body; bullets use *; close_source_branch:false."
    ;;
  *)
    echo "host: other (gitlab/self-hosted/Otto). Author title+body to standard;"
    echo "     create via Otto's PR action or the host's CLI/API."
    ;;
esac

echo
echo "## Commits on this branch (summarize ALL of these, not just the latest)"
run log "$target..HEAD" --oneline || echo "(none — is $target correct / fetched?)"

echo
echo "## Diffstat vs $target"
run diff "$target...HEAD" --stat

echo
echo "# Reminder: Jira key in the TITLE only (body-key crashes GitKraken);"
echo "# no AI attribution / 'Generated with' / Co-Authored-By footer of any kind."
