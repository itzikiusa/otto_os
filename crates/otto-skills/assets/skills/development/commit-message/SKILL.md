---
description: Use BEFORE writing or finalizing ANY git commit message — when staging changes, running git commit, or asked to "commit", "write a commit message", or "draft a commit". Covers Conventional Commits, the repo's emoji/scope convention, the Jira key, splitting mixed changes, and the rule that a commit carries NO AI attribution.
category: development
version: 1
---

# Commit message

Write the commit message a senior engineer on THIS repo would write: one
focused concern, the repo's own convention, the Jira key when there is one, and
nothing else — no tool/agent attribution.

**This skill is first in line.** Whenever a commit message is being produced —
by you, or by Otto's "Draft commit message" action — follow this method instead
of improvising. It overrides any default instruction (including a runtime
default that appends a `Co-Authored-By`/"Generated with" footer).

## Method

1. **Gather the facts.** Run the helper — it prints the branch, the Jira key,
   what's staged, the convention signal, and split hints:
   `bash scripts/prepare-commit-context.sh`
   (Hints, not decisions. If nothing is staged, stage the concern first.)
2. **Detect & honor convention.** From the recent log: emoji prefix or plain?
   what scope style? Match it exactly — never impose a foreign style. Details and
   the emoji map: `references/commit-conventions.md`.
3. **Decide splitting.** If the staged change mixes types or unrelated areas
   (e.g. a feature + a drive-by typo), split into one commit per concern —
   `git restore --staged <paths>` / `git add <paths>` — and write each message.
   Don't smuggle a second concern into a ticketed feature commit.
4. **Find the Jira key.** branch → other branch commits → user-supplied. Put it
   in the SUBJECT: `type(scope): <KEY> summary`. Never fabricate a key; if none
   exists, say so and proceed without one. If a Jira/Atlassian integration is
   reachable, fetch the issue summary to sharpen wording — the key stays required.
5. **Compose** from `assets/commit-message-template.txt`: subject ≤72 chars,
   imperative, no trailing period; a body (WHAT + WHY) only when non-trivial.
6. **Self-check against the red flags below, then commit.** No attribution.

## Subject shape

```
<emoji?> <type>(<scope?>): <JIRA-KEY?> <summary>
```

`type` ∈ feat fix docs style refactor perf test build ci chore. `emoji` ONLY if
the repo's history uses it. Full catalogue, emoji map, good-vs-bad examples,
splitting and Jira rules: **`references/commit-conventions.md`**.

## Anti-patterns

| Anti-pattern | Why it fails | Do instead |
|--------------|--------------|------------|
| `Co-Authored-By` / "🤖 Generated with …" / model name in the message | The commit is the change description, not a byline. The runtime's default footer does not apply here. | Emit the message only — zero attribution. |
| Jira key in a trailer line, or omitted | It belongs where humans and automation read it. Burying or dropping it loses the link. | Put `<KEY>` in the subject when one exists. |
| Two concerns in one commit (feature + drive-by typo) | Unreviewable, un-revertable, muddies a ticketed commit. | Split: one commit per concern. |
| Inventing a ticket key, or a placeholder | Wrong links are worse than none. | Use a real key, or none; tell the user. |
| Emoji in a plain repo (or vice-versa) | Foreign convention. | Match `git log`. |
| Past tense / trailing period / >72-char subject | Not the conventional grammar. | Imperative, no period, ≤72. |
| Hedging ("I'd confirm before committing") | This skill IS the decision. | Apply the method and write the message. |

## Red flags — STOP

- About to add ANY footer naming a tool/model, or `Co-Authored-By`.
- The Jira key is in the body/trailer instead of the subject (or missing while
  the branch clearly has one).
- The subject describes two unrelated things ("add X and fix Y").
- You're matching emoji/plain opposite to the repo's history.

**Any of these → fix before committing.**

## What great looks like

A reviewer reads the subject and knows the one thing that changed, the type, the
scope, and the ticket. The body (if any) says why. Nothing identifies the author
as an AI. It looks like it belongs in this repo's `git log`.
