---
id: proof
title: Proof
group: Build
route: proof
summary: Evidence for each piece of agent work (tests, diffs, CI, reviews and approvals), with a status Otto works out from that evidence.
---

## What it's for

A proof pack collects the evidence that a piece of work is done: the diff, test runs, CI, review results, screenshots, API or database checks, and human approvals. Otto works out the status from the evidence. An agent can add evidence, but it can't mark its own work as passed.

## Getting started

1. Open **Proof** in the sidebar. Packs are created automatically for goal loops, reviews, workflow runs and agent sessions that finish all their tasks.
2. To check something by hand, choose **New proof pack** (the + in the list).
3. Open a pack and choose **Assemble**. Pick the working directory and Otto collects its diff as evidence.
4. To record a test run, choose **Add** → **Add artifact…**, set the kind to command, use the command itself as the title (for example `cargo test`), set the status and paste the output.
5. Use **Add** for screenshots or video, API, database or Kafka evidence, or a PR description check.
6. Read the done score and its checklist to see what is still missing.

## Everything it can do

**Status and scores**
- Every pack has a status: missing, partial, passed, failed or waived. Otto recalculates it after every change.
- Different work needs different evidence. Code changes need a diff and a passing test command. Reviews need a review result. Workflow runs need their approvals to pass.
- Only a command artifact whose title is a real test, build or lint command counts as a test (for example `cargo test`, `npm test`, `pytest`, `go test`, `vitest`, `playwright test`).
- **Risk score** (0–100) grows with the size of the change, risky files (migrations, SQL, lock files, auth or secret code), failing tests, unresolved reviews and untested diffs.
- **Done score** (0–100) is an itemised checklist: diff, tests, no failures, CI, review, PR consistency, UI evidence, data evidence, self-review and human approval. 80 or more counts as ready.
- Badges show what matters at a glance, such as tests passed, CI failed, risky change, UI verified and review unresolved.

**Evidence**
- Artifact kinds: command, log, screenshot, video, diff, CI, API, database, Kafka, review, approval, PR check and self-review.
- Screenshots and videos (PNG, JPEG, GIF, WebP, SVG, MP4, WebM, up to 25 MB) show inline.
- **Refresh CI** fetches the latest CI status for a pack linked to a pull request.
- **PR check** checks that a PR description matches the change. It fails the pack if the description claims tests pass when they don't.
- Every text artifact records a SHA-256 hash. Secrets such as tokens, keys and emails are redacted before anything is stored.
- Long content is shortened in the list; **Load full** shows everything.

**Pack actions**
- **Requirements** (for a pack linked to a repository): require a passing test, green CI, a consistent PR or a resolved review for that repository. Requirements can only make a pack stricter.
- **Snapshot** freezes the current evidence as a tamper-evident copy, with its own reports.
- **Export .md** and **Export .html** download a self-contained report.
- **Waive** accepts a pack without the evidence. You must give a reason of at least 10 characters, and you are recorded as the approver.
- **Delete** removes the pack with its artifacts and snapshots.
- Filter the list by all, passed, failed, partial, missing or waived.

**Where packs come from**
- Goal loops package their verify commands, diff and evaluation.
- AI code reviews add a review result that stays failed while findings are open.
- Workflow runs add each step's output and each approval.
- Agent sessions add a diff when every task is marked done.
- Pull requests opened with a linked pack capture CI and run the PR check automatically.

## Keyboard shortcuts

None specific to this page.

## Tips and limits

- Access needs the **Proof** feature. Reading needs the workspace Viewer role; adding evidence, waiving and deleting need Editor.
- A pull request opened with a linked pack that hasn't passed is blocked unless it's explicitly overridden. The override is itself recorded as evidence. Pull requests opened from the Git page don't link a pack, so they aren't blocked.
- Running tests automatically when a session finishes is off by default, because it runs in your live folder.
- The API client and Database Explorer don't add evidence to packs automatically yet.
- Stored text is capped at 2 MB per artifact; snapshots keep 64 KB per artifact but keep the full hash.

## Related

- [Git](#/walkthroughs/git)
- [Goal Loops](#/walkthroughs/loops)
- [Workflows](#/walkthroughs/workflows)
- [Run with Otto](#/walkthroughs/run-with-otto)
- [Mission Control](#/walkthroughs/mission-control)
