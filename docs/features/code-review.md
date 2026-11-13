# AI Code Review

Otto reviews pull requests and local changes by inspecting their scope, choosing
relevant lenses, and verifying the resulting claims against the code. Reviews
produce drafts for inspection. They can also start an isolated Goal Loop to fix
verified PR findings and review the result again.

The API and WebSocket contracts in [`../contracts/api.md`](../contracts/api.md)
and [`../contracts/ws.md`](../contracts/ws.md) are authoritative. The engine lives
in `crates/otto-review/`, server integration in
`crates/otto-server/src/modules/review/`, and the UI in `ui/src/modules/git/`.

## Start a review

Open **Git → Pull Requests → a PR → Review**, optionally attach a Jira story or
enter reviewer guidance, then choose **Send to review agents**. **Configure…**
controls provider preferences, lens instructions and the final verifier.

For uncommitted work, open the repository's **Review** tab, choose **Compare to**,
and select **Review changes**. Local reviews include tracked changes and
untracked, non-ignored files. No hosting account is needed for local reviews.
An empty diff finishes without launching reviewers; it does not establish that a
change was reviewed.

You need an installed, authenticated CLI for each selected provider. Supported UI
choices are Claude, Codex and agy. The configured summarizer provider also handles
scope assessment; it is not restricted to Claude. Installed review skills are
optional presets, not a prerequisite for adaptive review.

Starting a PR review requires a hosting account bound to the repository. The
caller must own that account, or be root. The same ownership rule applies to an
attached Jira account. Reading requires workspace Viewer; starting, retrying,
fixing and acting on findings require Editor and the applicable Git permission.
The workspace usage budget is checked before launching work.

## What runs

| Stage | Behavior |
|---|---|
| Establish the change | For a PR, materialize its source revision in an isolated checkout and calculate the complete merge-base-to-HEAD patch. Record both immutable SHAs. |
| Assess scope | Inspect changed behavior, definitions and affected consumers. Select 1–10 distinct lenses and record the reason for each. |
| Review | Run selected lenses through the shared bounded queue, one provider per lens by default. Each reviewer checks the relevant changed code and supporting definitions. |
| Verify | The configured summarizer independently checks claims, combines issues sharing a fix, and records evidence-backed dispositions for every input claim. |
| Present | Show verified draft comments, original agent results, verification notes and prior-finding dispositions. Incomplete verification remains an error, never a clean review. |

Open **Review scope** to see the selected lenses, their reasons, related modules,
risks and assigned providers. A small change need not launch a broad review. A
prose-only change under `docs/` can use a deterministic documentation scope; it
still runs a reviewer and verifier. Changes with prior findings or configured
required checks use scope assessment instead. A PR over the large-PR threshold
(5,000 changed lines by default) skips the assessment and runs every configured
lens on every provider.

The assessment gets two attempts inside a one-hour cap; each attempt fails only
when the assessor goes quiet for the reviewer idle budget. A usable plan with gaps
is kept (a missing reason or instruction is defaulted; a prior finding it left
out stays `still_open` with the evidence `not assessed by scope`). When both
attempts fail, the review does not stop: every configured lens runs, and
**Review scope** shows why the assessment was unavailable.

Prompts over 16 KB (reviewer, scope and verifier prompts routinely run 100+ KB)
are written to a private temp file and the session is typed a short pointer to
it, so a long paste cannot outlast the terminal write and leave an agent that
never started.

Enabled providers form a pool. By default lenses are distributed across that
pool. **Cross-provider check** explicitly runs selected configured lenses on their
configured providers; newly assessed lenses use the enabled pool. This expands
work and is useful when the additional comparison is wanted. Adaptive reviews
always use managed fan-out, including configurations retaining the legacy
`orchestrator` setting.

The daemon-wide reviewer limit defaults to eight; operators can override it with
`OTTO_REVIEWER_CONCURRENCY`. Queued reviewers consume no CLI process, and their
execution timeout starts when admitted. Scope assessment shares this queue.
Workflow multi-repository reviews additionally admit at most two repository
reviews together; they do not create separate reviewer limits per repository.

### Causal scope and verification

A finding must identify a concrete defect caused by the supplied change, with a
reachable trigger, consequence, and inspected code evidence. Related unchanged
code is context for checking definitions and consumers; reviewers are not limited
to reading the patch in isolation. Reverting the change must remove the defect.
An existing issue merely widened by the change belongs in a summary note, not a
new finding thread. Author preferences and hypothetical cleanup are not defects.

The final verifier accounts for each original claim with a source ID and an
explicit disposition. Retained findings need `verified` plus evidence. Rejected,
unverified and duplicate claims appear as verification notes instead of PR
comments. Missing claims, missing evidence, failed reviewers or a failed verifier
leave the review incomplete. Code tracing is distinguished from executed tests.
Test review targets newly changed critical paths and demonstrable false greens,
not exhaustive hypothetical coverage.

There is no 20-comment cap: every distinct verified in-scope finding is retained.
Severity describes impact: `bug` means must fix, `warn` means should fix, and
`info` means optional. Issues sharing one fix are consolidated. Raw reviewer
findings remain available, including when final verification fails.

### How a review ends

| Outcome | Review status | What it means |
|---|---|---|
| Verified | `done` | The verifier accounted for the reviewers' claims. A claim it did not account for is still stored, as a draft whose body starts `[unverified] `, and still counts; omitting a claim never makes it disappear. **Verification coverage** in the scope notes says "N of M reviewer claims verified; K carried forward unverified". |
| Incomplete — not a verdict | `error`, message `Review incomplete — not a verdict: …` | The reviewers' findings are stored, but there is no verdict: the verifier did not run or accounted for no claim (every finding is stored unverified), a reviewer did not finish (named with why it failed, e.g. `Security (timed out with no result)`), or the reviewed change was only partly covered (a partial diff, or a PR source that could not be checked out). Never read it as clean or as a score of 0. |
| Error | `error`, any other message | Nothing usable (the diff could not be written, the checkout failed, …). Re-run the review. |

To finish an incomplete review: when every reviewer finished and only the
verifier failed, **Retry summarizer** (or, in a workflow, **Retry step**)
verifies the stored findings again without re-running the reviewers. A failed
reviewer is retried on its own card. A partial diff or checkout needs a fresh
review. The verifier itself gets two start attempts before the run is settled.

A summarizer re-run of the same review (**Retry summarizer**, **Retry step**, or
a restart resume) replaces the review's undecided drafts, `[unverified]` ones
included; drafts you already approved, declined or posted stay. When the re-run's
verifier ran and accounted for the claims, the tracked findings that only the
earlier attempt's `[unverified]` drafts had created, and that this attempt did not
raise again, are closed (their `state` becomes `declined`). A finding linked to a
comment you decided on, or one a later review raised, is left alone. The triage
`status` is not changed, so such a finding still counts toward merge readiness
and the review verdict until someone triages it.

Unverified findings are never posted by a workflow as verified: an incomplete
review fails its `review_run` step before any downstream posting step runs.
When no reviewer finished at all, the message starts `no reviewer finished`, and
neither a score nor a partial score is reported (the verifier's own rating is
left out too, since it had nothing to rate).

### Output formats

Every agent in a review answers in a fixed JSON shape: reviewer findings with a
`lens_score` element, the scope plan inside its one-element transport array, and
the verifier's comments, notes and overall score. Each shape has one canonical
example in `crates/otto-review/src/contract.rs` (`REVIEWER_OUTPUT_EXAMPLE`,
`LENS_FILE_EXAMPLE`, `SCOPE_PLAN_EXAMPLE`, `VERIFIER_OUTPUT_EXAMPLE`). The
prompt embeds that example, and `contract_tests.rs` runs it through the reader
the daemon uses. The allowed severities and dispositions a prompt lists are
rendered from the lists the checker uses (`FINDING_SEVERITIES`,
`verifier_dispositions()`, `PRIOR_SCOPE_DISPOSITIONS`). When you edit a prompt
or a reader, change the example in `contract.rs` and run those tests.

Readers take an answer element by element. An element they cannot use is
skipped and counted, and the rest are kept. A reviewer's card shows
`· N items could not be read`. The scope notes say `Scope assessment: N lens
entries skipped (…)`, and the verification notes say
`Verifier output: N of M items could not be read and were skipped (…)`. The
claims a skipped verifier item covered are carried forward unverified. Fenced
JSON, a sentence before the JSON, numbers given as strings for lines, a listed
disposition in another case or spelling (`Verified`, `not-relevant`), and a
scope plan sent without its transport array are all accepted. An answer that
is not JSON at all fails with a message naming the expected shape. A scope plan
that cannot be used still never stops the review: every configured lens runs.

### Re-review

Starting a fresh review assesses the latest change and brings forward previous
tracked findings for the same repository and PR. Local lineage is also scoped to
the source branch (detached PR checkouts are named by their remote branch; with
no branch name at all, nothing is brought forward). Every prior finding needs an
explicit `fixed`, `still_open` or `not_relevant` disposition with evidence, and is
also handed to the final verifier as a claim of its own. These dispositions are
visible in **Review scope** and under **Resolved since the last run**.

A prior finding is marked resolved (its tracked `state` becomes `resolved`) only
when all of these hold:

- the scope assessment said `fixed` with evidence (a disposition filled in as
  `not assessed by scope`, or by a fallback plan, never counts);
- the final verifier re-checked that claim and answered it as fixed. Its prompt
  gives one answer per case: fixed in the current code → `not_relevant` with
  evidence citing the code that fixes it; still present → `verified` with the
  current file:line (it stays a comment); cannot tell → `unverified`. Any item
  keeping the claim (`verified`, `duplicate` or `unverified`) vetoes the close;
- the managed verifier ran (not the deterministic fallback), the diff was not
  partial, and a PR's source was checked out;
- this run did not raise the same finding again.

Disappearing from a new run's output resolves nothing: the scope picks different
lenses per run, the verifier drops pre-existing issues as out of scope, and a
re-worded finding can come back under a new fingerprint. Resolution only touches
the engine's `state`; the triage `status` people set (accepted, waived, …), and
so merge readiness, is never changed by a review.

For workflow runs, use **Re-review** in the run monitor or send `re-review` in the
original workflow chat thread. This creates a linked run, refreshes PR refs and
context, and repeats the configured workflow, including approvals and its
configured outward actions. The previous run remains available. See
[`workflows.md`](workflows.md#re-review-a-pr-after-fixes) for context reuse and the
portable upgrade procedure for workflows configured on another machine.

## Watch, stop and retry

Reviewers are real, openable sessions. Agent cards show pending, running, waiting,
done, skipped or error states, findings and a short progress note. Open a session
to inspect its output. Updates use `review_changed` events with fallback polling.
The summarizer also has an openable managed session on its configured provider.

- **Stop** cancels one active reviewer and leaves other reviewers running.
- **Retry** reruns that reviewer using its persisted prompt, diff and reviewed
  context. It is not a review of newly pushed commits; start a fresh review for that.
- **Retry summarizer** repeats final verification over the stored reviewer results
  (needs at least one stored finding; otherwise start a new review).
- **Cancel review** stops the complete run and tears down its sessions. Cancelled
  reviews cannot be retried using their deleted persisted prompts.

Automatic reviewer recovery allows three total attempts by default, configurable
through `max_attempts`. Reviewer execution budgets scale with diff size: 10 minutes
below 4,000 bytes, 20 below 20,000, 30 below 100,000, one hour below 400,000, and two
hours above that. `timeout_secs` overrides this. These are failure ceilings, not
expected durations. Waiting is signaled after about two minutes of idle time;
about fifteen minutes of silence triggers stuck recovery.

The verifier budget is `300 + 3 × input findings` seconds, capped at 30 minutes.
A timeout, provider failure or malformed response can preserve a deterministic
fallback, but that fallback is not independent verification: the review remains
incomplete. Cancellation is terminal and does not start a fallback attempt.

**Daemon restart.** A review running when the daemon stops is settled at the next
start. If every reviewer had finished and stored findings, only the summarizer is
run again over those findings (as **Retry summarizer** does) and the review shows
as running; a workflow step waiting on it picks it up. Each such resume is noted in
the scope's verification notes, and after two a review is failed instead
(`Interrupted by daemon restarts while summarizing, after 2 automatic resumes`),
so a crash loop stops. A review whose reviewers had not all finished fails with
`Interrupted by a daemon restart — re-run to review again.` A cancelled review is
left cancelled.

## Act on findings

PR findings appear as draft comments with severity, location and supporting
context. **Approve** posts the selected comment through the hosting provider and
records it locally in `.otto/pr-<number>-review.md`. Check `posted` when provider
posting fails; local approval does not prove a remote comment was delivered.
**Decline** discards a draft. Posting is an explicit user action.

The findings workflow also supports acceptance, false-positive decisions, waivers,
fix agents, verification and an audit trail. The API exposes the current statuses
`open`, `accepted`, `false_positive`, `fixed`, `verified` and `waived`. The older
fingerprint/state endpoint remains for compatibility. A waiver or dismissal is
not test evidence; prior-finding verification records why an item no longer applies.

**Send to agent** hands selected findings to a fresh editing session. This is a
one-shot handoff, separate from the bounded fix-and-review loop below. Local
reviews have no PR to post to; use handoff or inspect and edit locally.

**Merge readiness** combines findings with hosting-provider CI, approvals,
mergeability and branch state. Agent judgment is evidence to inspect, not a
replacement for those checks. **Past reviews** retains previous runs. The local
Review tab only re-adopts a running review after navigation; finished local
results stay in history because the working tree may have changed.

### Run an agent to fix

A completed PR review offers **Run agent to fix…**. The dialog defaults to a 9.5/10
reviewer target and starts a Goal Loop on an isolated worktree at the reviewed
commit. Follow progress, evidence, Stop and Resume in Goal Loops. Fixes remain
local and uncommitted for inspection; this action does not authorize publication.

The agent batches related must-fix and should-fix work, verifies previous findings,
then performs a fresh review. Optional lows may be explicitly deferred. The score
is a subjective assessment supported by evidence, not an objective measurement.
Unresolved important findings or unavailable material checks prevent completion.
The default budget is six iterations or two active hours. Exhaustion remains
incomplete and can be resumed. Legacy reviews need a fresh scope assessment first;
local uncommitted reviews cannot seed this commit-based fix flow.

## Configuration

Configure review agents in the Review panel. Repositories can inherit the global
configuration, follow a named preset, or store their own configuration. A preset
reference follows later preset edits; editing it into a custom repo configuration
does not silently change the shared preset. Global settings writes require root.

| Field | Meaning |
|---|---|
| `agents` | Candidate lens instructions and enabled provider preferences. Scope assessment selects applicable lenses and can add missing ones. |
| `summarizer` | Provider, model and instructions for final verification; its provider also runs scope assessment. |
| `cross_provider_check` | Defaults to `false`; enables additional provider comparisons for selected lenses. |
| `custom_presets` | Saved lens suggestions for the menu; not run merely because they are saved. |
| `max_attempts` | Total reviewer attempts; absent means three. |
| `timeout_secs` | Explicit per-reviewer execution budget. |
| `mode` | Legacy `fan_out`/`orchestrator` preference; adaptive review executes managed fan-out. |
| `previous_review_id` | Explicit workflow lineage, validated against repository and PR; normally set by the workflow rerun path. |

Each agent configuration has `name`, `provider`, `providers`, `model`, `prompt`
and optional `skill`. An empty model uses the provider default. Installed review
skills can supply optional presets; the selected instructions are authoritative
and adaptive review does not require installing a preset skill.

## API entry points

All paths below have the `/api/v1` prefix. See the contract for request/response
schemas and full finding-action routes.

| Purpose | Endpoint |
|---|---|
| Start / latest PR review | `POST` / `GET /repos/{id}/prs/{number}/review` |
| PR history | `GET /repos/{id}/prs/{number}/reviews` |
| Start / latest local review | `POST` / `GET /repos/{id}/local-review` |
| Local history | `GET /repos/{id}/local-reviews` |
| Retry / stop one reviewer | `POST /reviews/{id}/agents/{index}/retry` or `/stop` |
| Retry final verification | `POST /reviews/{id}/summarizer/retry` |
| Cancel | `POST /reviews/{id}/cancel` |
| Handoff | `POST /reviews/{id}/handoff` |
| Findings / readiness | `GET /reviews/{id}/findings` or `/merge-readiness` |
| Approve / decline a draft | `POST /pr-review-comments/{id}/approve` or `/decline` |
| Global configuration | `GET` / `PUT /settings/pr-review` |
| Repository configuration | `GET` / `PUT /repos/{id}/review-config` |
| Named configurations | `GET` / `PUT /settings/pr-review/presets` |

`Review.scope_plan` carries the inspected SHAs, scope, selected lenses,
prior-finding dispositions and verification notes. `Review.agents` exposes live
agent states and raw results. `summary_fallback` identifies deterministic fallback
output; it must not be interpreted as verified completion.

## Limits and security

PR reviews use a complete local causal patch, not a forge preview capped at
200 KB. The PR patch has a **32 MiB hard safety limit**: exceeding it fails the
review instead of silently reviewing a prefix. A missing source checkout, target
branch or merge-base also prevents causal review.

Local reviews use a separate working-tree diff path. Untracked additions are
bounded to 1 MiB per file, 2,000 files, and an 8 MiB remaining aggregate budget;
omitted content is explicitly marked and prevents a complete verification claim.
Local working-tree changes are not an immutable PR snapshot. Commit/PR drafting
and ordinary diff previews have their own limits; they do not define PR review
coverage.

Review sessions use read-only metadata and the sandbox, with project settings
excluded so a reviewed PR cannot load its own provider hooks. The prompt allows
reading supporting code but forbids repository mutations; its result artifact is
an allowed write. Ordinary review is not permission to install dependencies, run
arbitrary test suites or edit code. Explicit fix actions start editing agents in
their separately authorized scope.

Review sessions use the daemon's root run-as identity; hosting and Jira credential
ownership are checked against the requesting user before credentials are used.
Credentials remain in Keychain. Provider conclusions can still be wrong: inspect
evidence and distinguish code traces, actual checks and unavailable checks.

## Troubleshooting

| Symptom | Action |
|---|---|
| Scope assessment failed | The review continued with every configured lens; **Review scope** shows why. Inspect the provider session if it keeps failing (two attempts per review). |
| Reviewer is queued | Other reviews may occupy the shared slots. Queued time does not consume the reviewer execution timeout. |
| Reviewer is waiting or stuck | Open its session, inspect the cause, or retry that reviewer. |
| `Review incomplete — not a verdict: the final verifier …` | The findings are stored unverified. **Retry summarizer** (or **Retry step** in a workflow) verifies them without re-running the reviewers. |
| `Review incomplete — not a verdict: reviewer(s) did not finish: X (why)` | The other lenses were verified; the score is partial. Retry reviewer X on its card. |
| `Review incomplete — not a verdict: the reviewed change is not fully covered` | Part of the diff was omitted or the PR source was not checked out. Fix the cause and start a fresh review. |
| `Interrupted by a daemon restart …` | Reviewers had not finished: re-run the review. After two automatic summarizer resumes: **Retry summarizer**. |
| A fixed finding still shows open | Only an explicit, verified `fixed` resolves a finding (see Re-review); triage it yourself, or re-review so the scope and verifier assess it. |
| PR revision or merge-base unavailable | Fetch/repair the relevant source and target refs, then start a new review. Otto does not verify another checkout as if it were the PR. |
| Local files were omitted | Inspect the explicit omission markers; narrow or commit the change before seeking complete coverage. |
| Posted comment is missing | Check `posted` and the provider error; verify account access and rate limits. |
| Local review disappeared after navigation | Finished local reviews live in **Past reviews**. Only active runs are automatically restored. |
| Fix action unavailable | Complete a current PR review with scope evidence first. Local uncommitted changes and old reviews cannot seed this fix flow. |

Related: [`git.md`](git.md), [`workflows.md`](workflows.md),
[`agent-sessions.md`](agent-sessions.md), [`goal-loops.md`](goal-loops.md),
[`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).
