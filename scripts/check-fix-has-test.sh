#!/usr/bin/env bash
# Advisory guard: every commit in BASE..HEAD whose subject starts with
# `🐛 fix(` and that changes ui/src must also change ui/unit or ui/e2e — a UI
# bug fix without a regression test tends to come back. Prints a GitHub
# `::warning::` per offender and ALWAYS exits 0 (CI runs it non-blocking).
#
#   scripts/check-fix-has-test.sh [BASE] [HEAD]   # defaults: origin/main HEAD
set -euo pipefail
base="${1:-origin/main}"
head="${2:-HEAD}"
offenders=0
while IFS= read -r sha; do
  [[ -z "$sha" ]] && continue
  subject=$(git log -1 --format=%s "$sha")
  [[ "$subject" == "🐛 fix("* ]] || continue
  files=$(git diff-tree --no-commit-id --name-only -r "$sha")
  grep -q '^ui/src/' <<<"$files" || continue
  if ! grep -qE '^ui/(unit|e2e)/' <<<"$files"; then
    echo "::warning::${sha:0:10} \"$subject\" changes ui/src without a ui/unit or ui/e2e test"
    offenders=$((offenders + 1))
  fi
done < <(git rev-list --no-merges "$base..$head")
echo "check-fix-has-test: $offenders UI fix commit(s) without a test"
exit 0
