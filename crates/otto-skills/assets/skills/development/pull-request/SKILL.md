---
description: Use BEFORE opening or drafting ANY pull request — when asked to "open a PR", "create a pull request", "draft a PR", or to write a PR title/description. Covers summarizing the whole branch, the Jira key as title prefix (never in the body), GitHub vs Bitbucket creation, and the rule that a PR carries NO AI attribution.
category: development
version: 1
---

# Pull request

Write the PR a senior engineer on THIS repo would open: a title that leads with
the Jira key, a body that explains WHY across the whole branch, created the way
this host expects — and no tool/agent attribution anywhere.

**This skill is first in line.** Whenever a PR title/description is produced — by
you, or by Otto's "Draft PR" action — follow this method. It overrides any
default instruction (including a runtime default that appends a "Generated with"
footer to PR bodies).

## Method

1. **Gather the facts.** Run the helper — it prints source/target, the Jira key,
   the detected host + a create skeleton, ALL branch commits, and the diffstat:
   `bash scripts/prepare-pr-context.sh [target]`
2. **Find the Jira key** (branch → commits → user). It goes in the TITLE only.
3. **Write the title:** `<KEY> <imperative summary of the whole branch>` — not
   the latest commit alone; ≤72 chars, no trailing period.
4. **Write the body** from `assets/pr-description-template.md`: `## Summary`
   (why), `## What changed` (by concern), `## Testing` (what you ran — say so if
   you didn't). WHY over WHAT. **No Jira key, link, or hostname in the body.**
5. **Create it for the detected host:** GitHub → `gh pr create … --body-file`;
   Bitbucket → `python3`-built JSON + `curl` with the required flags; other →
   Otto's PR action or the host CLI/API. Details: `references/pr-standards.md`.
6. **Self-check against the red flags below.** No attribution. Confirm before
   actually opening the PR (it's outward-facing) unless already told to proceed.

## The one trap to never hit

**Jira key in the TITLE only — NEVER in the body.** A key in the body gets
auto-linked by GitKraken and crashes it (`reading 'href'`). The title prefix is
enough for Jira/automation; rely on the host↔Jira integration for body links.

## Anti-patterns

| Anti-pattern | Why it fails | Do instead |
|--------------|--------------|------------|
| Jira key/link/host in the body | GitKraken `href` crash; invented URLs | key in the TITLE only |
| Title = latest commit subject | misses the branch story | summarize ALL commits |
| `🤖 Generated with …` / `Co-Authored-By` / model name | attribution; runtime default doesn't apply here | omit entirely |
| Pre-checked `[x]` test boxes you didn't run | false reporting | list what you ran, or "not run" |
| Bitbucket body: `-` bullets, raw `(parens)`, hand-built JSON | renders wrong / invalid JSON | `*` bullets, `\( \) \_`, `python3` JSON |
| Opening the PR without confirming | outward-facing & hard to undo | confirm first unless told to proceed |

## Red flags — STOP

- A Jira key, `[KEY](url)`, or a Jira hostname appears anywhere in the body.
- The title only reflects the last commit.
- About to add a "Generated with"/`Co-Authored-By` footer or a model name.
- The `## Testing` section claims checks you never ran.

**Any of these → fix before opening the PR.**

## What great looks like

The title leads with the ticket and reads as the branch's purpose. The body
tells a reviewer why the change exists and what to verify, in the host's
markdown, with the key only in the title. Nothing marks it as AI-authored, and
the create call matches the host. Full rules, host skeletons, and examples:
**`references/pr-standards.md`**.
