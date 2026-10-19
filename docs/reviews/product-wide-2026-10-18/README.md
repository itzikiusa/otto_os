# Full-product review — 2026-10-18

Current status and evidence: [current verification](current-verification.md),
[root UX closure](root-ux-closure-crosscheck.md), and
[provider UI closure](design-grok-closure-crosscheck.md). The roster below
preserves the original allocation, not a list of currently running sessions.
Only the original root, Ronaldo and Pele sessions continue the integration.

## Scope correction

The earlier database parity review covered 24 mostly database-related lenses. It
is evidence for that work, not a full Otto review. The user explicitly requested
full-product coverage, at least 20 distinct engineering reviewers, and then ten
dedicated design/UX reviewers in each of the agy and grok sessions (20 UX
reviewers). All findings and fixes remain in PR #111. Merge and installation are
held until this expanded review and its verification complete.

Target: 9.8+ for every lens, with up to ten review/fix iterations. Scores must be
supported by concrete coverage and validation, never inflated to satisfy the
target. A planned reviewer does not count as an active or completed reviewer.

## Engineering reviewer roster (original allocation)

| Reviewer | Scope | Actual agent | Status |
|---|---|---|---|
| E01 | Session lifecycle, providers, PTY and transcripts | /root/product_sessions | Reviewing/fixing |
| E02 | Workflow graph, execution, retry, triggers and recovery | /root/review02_workflows | Reviewing/fixing |
| E03 | Git, worktrees, dirty work, conflicts and PR operations | /root/review03_git | Reviewing/fixing |
| E04 | Scheduled-task engine | 01a14ff8-76af-77e0-9d76-76fee1df78f1 (Pele subagent) | Reviewing |
| E05 | Goal loops | 01a14ff8-76af-77e0-9d76-7706483b43ff (Pele subagent) | Reviewing |
| E06 | Assistant and personal agents | 01a14ff8-76af-77e0-9d76-77179e4a3fdb (Pele subagent) | Reviewing |
| E07 | Swarm runtime | 01a14ff8-76af-77e0-9d76-77206722c554 (Pele subagent) | Reviewing |
| E08 | api | 01M57ZKKJ6K5VEET03YF0DYZ80 | Review complete; fixes allocated |
| E09 | connections | 01M57ZKM55HMTDGJRBA365JE8A | Review complete; fixes allocated |
| E10 | vault | 01M57ZKMFGW8CCNV0YFZYW8NFH | Review complete; fixes allocated |
| E11 | design | 01M57ZKMSNCW5P83YC4N5EXMRJ | Review complete; fixes allocated |
| E12 | product | 01M57ZKNCARZC178CJ1PRCX5BW | Review complete; fixes allocated |
| E13 | review | 01M57ZKNXHPSMEM8TWRCF6SSDV | Review complete; fixes allocated |
| E14 | state | 01M57ZKQAVAB1M1R5R470C7AZX | Review complete; fixes allocated |
| E15 | runtime | /root/review15_runtime | Reviewing (replaces exited launch) |
| E16 | auth | 01M57ZKTRZRWAFXT2GFJ0JMYNC | Review complete; fixes allocated |
| E17 | channels | 01M57ZKWJ4P6823ME5QERW5BKZ | Review complete; fixes allocated |
| E18 | accounting | 01M57ZKXYS9ZEM3EYV0PCHVVHE | Review complete; fixes allocated |
| E19 | extensibility | 01M57ZKZ2RRH36F0C8VH1W056W | Review complete; fixes allocated |
| E20 | ui-contracts | 01M57ZM02HS93CD3C0Y403PNDG | Review complete; fixes allocated |

Each specialist reviews correctness, errors, cancellation/recovery, resource cost,
contracts and regression strength within its scope. Independent cross-review
follows fixes. Source ownership prevents concurrent conflicting edits; Cargo
runs are serialized, while read-only work and independent UI tests proceed.

## Design/UX reviewer assignments

Ronaldo (agy), session `01M57PYTS624JJ0SDSJ4RKSJPS`, is assigned ten distinct
reviewers: navigation, home, session/provider selection, terminal, chat,
git/PR/worktrees, connections/database/brokers, settings/onboarding, responsive
mobile/RTL, and accessibility/keyboard/focus.

Pele (grok), session `01M57PYXXY6TK198E7FN6AGXMS`, is assigned ten distinct
reviewers: workflows, schedules/loops, personal agents/messaging, swarm/mission
control, Vault, Design Hall/Canvas, Product, review/proof/run-with-Otto,
API/browser/integrations, and usage/insights/skills/plugins.

The leads must report actual reviewer identities and results. Assignments alone
are not evidence that those reviewers have started. Their individual records
belong in `design-agy/` and `design-grok/`.

## Evidence requirements

- Concrete file/line and reproducible or hand-traced behavior for findings.
- Explicit inspected flows, including loading/empty/error/retry and restoration.
- Meaningful regression checks for behavioral fixes; named browser specs only.
- Preserve legitimate root/privileged operations; no speculative restrictions.
- Separate confirmed defects, usability improvements and unverified questions.
- Record before/after scores, verification and residual gaps for every reviewer.

## User correction: delegation and provider balance

Do not open any more visible sessions. Existing extra Codex reviewers may finish
already-assigned work; completed sessions closed by the user stay closed. All
remaining delegation goes through the two original sessions only. Do not launch
new reviewers or sessions; the original reviewer roster above is historical.

Remaining engineering ownership is redistributed approximately evenly:

- Ronaldo / agy: E08 API, E09 connections, E10 Vault, E14 state, E18 accounting,
  E19 extensibility, E20 UI contracts (seven areas), plus its existing UX team.
- Pele / grok: E04 schedules, E05 goal loops, E06 assistant, E07 swarm,
  E11 Design/Canvas, E12 Product, E17 channels (seven areas), plus its UX team.
- Root / Codex: E01 sessions, E02 workflows, E03 git, E13 proof/review,
  E15 runtime, E16 credential correctness (six areas), shared integration/release.

A Cargo slot serializes builds only; it does not prevent independent review,
implementation, regression writing, or UI checks. The current slot and actual gate outcomes are maintained in `coordination.md`;
the original reviewer roster above is provenance, not a live-session claim.
