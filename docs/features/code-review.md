# AI Code Review (multi-agent orchestrator)

Otto's AI Code Review fans out several review agents over a change — a **pull
request** or your **local working tree** — and aggregates their findings into a
single, actionable list. Each review agent is a *real, openable* coding-agent
session (claude / codex / agy) running one **lens** (correctness, security,
performance, …). You watch each agent live, retry a stuck one, approve/decline
the deduplicated draft comments, and hand the findings off to a fresh agent to
fix them.

This document is the authoritative end-user + operator reference for the
feature. Where it describes wire-level behaviour, the source of truth is the
contract files in `docs/contracts/` (`api.md`, `ws.md`); the Rust handlers in
`crates/otto-server/src/modules.rs` + `crates/otto-server/src/review_session.rs`
and the Svelte UI in `ui/src/modules/git/` implement it.

---

## 1. Overview

A review run is a single fan-out + merge pipeline:

```
   diff (PR or working tree)
        │
        ▼
   ┌─────────────────────────────────────────────┐
   │ for each (agent lens × provider)            │
   │   spawn a real session, inject the lens     │  ← run concurrently
   │   prompt + the diff, collect its findings   │     (one stuck agent
   └─────────────────────────────────────────────┘     never aborts others)
        │  findings from every agent
        ▼
   summarizer agent  (dedupe + prioritise → ≤20 ranked draft comments)
        │
        ▼
   draft comments  +  per-agent breakdown  +  merge-readiness
```

Key properties (all implemented):

- **Two diff sources.** A GitHub/GitLab/Bitbucket PR, or your uncommitted local
  working tree vs a base ref.
- **One session per (lens × provider).** Lenses come from your *review*-category
  skills; a lens may run on more than one CLI, expanding to one session each.
- **Live + openable.** Each agent is an ordinary Otto session. You can Open its
  terminal, watch it work, and type into it to unblock it.
- **Resilient.** Agents are independent and auto-retried up to a bounded number
  of attempts; a per-agent grace period bounds how long a run waits.
- **Verified-review loop.** Findings are persisted with a stable fingerprint and
  a lifecycle state (open → fixing → resolved/regressed/declined) so merge
  readiness can be assembled across runs.

### Where it lives

| Surface | Location | Notes |
|---|---|---|
| **PR review** UI | Git → open a PR → **Review** tab (`ui/src/modules/git/PrDetail.svelte` → `ReviewPanel.svelte`) | Per-PR; full history of runs |
| **Local (working-tree) review** UI | Git → repo → **Review** tab (`ui/src/modules/git/RepoView.svelte` → `LocalReviewPanel.svelte`) | Per-repo; opens to a clean slate (§6.5) |
| Shared per-agent cards | `ui/src/modules/git/ReviewAgents.svelte` | Open / Retry / per-agent findings; used by both panels |
| Configure-agents modal | inside `ReviewPanel.svelte` ("⚙ Configure") | Edits the persisted `ReviewConfig` |
| Orchestration engine | `crates/otto-server/src/modules.rs` (`run_review_core`, route handlers) | Fan-out, summarizer, draft-comment storage |
| Per-agent session lifecycle | `crates/otto-server/src/review_session.rs` | Spawn, prompt injection, watch, recovery/retry |
| Summarizer / drafting agent runner | `crates/otto-orchestrator` (`Orchestrator::run_agent`) | A headless claude turn driven via a PTY |
| Lenses (skills) | `crates/otto-skills/assets/skills/review/*` | `review`-category bundled skills |
| Findings + merge-readiness store | `crates/otto-state/src/review_findings.rs`, migrations `0006`, `0007`, `0049`, `0054` | Persistent fingerprinted findings + state |

---

## 2. Prerequisites

1. **At least one agent CLI installed and logged in.** Reviewer agents run as
   real sessions; the default lenses use the workspace's default agent (else the
   global `default_provider`, else `claude`). The summarizer step is hard-wired
   to **claude** (its runner is the claude PTY driver), so a working,
   logged-in `claude` CLI is required for the final dedupe/merge even if your
   reviewer lenses run on `codex`/`agy`. If `claude` is unavailable the
   summarizer fails gracefully and the per-agent findings are concatenated
   instead.
2. **`review`-category skills installed** (recommended). The one-click lens menu
   is built from your installed review skills; Otto bundles a strong default set
   (§5). Missing/outdated review skills surface a non-blocking banner with a
   shortcut to **Settings → Skills**. See [`./skills-library.md`](./skills-library.md).
3. **For PR mode only — a git hosting account bound to the repo.** The repo must
   have a `provider`, a remote URL, and a bound git account whose token can read
   the PR diff (and, for posting approved comments, write PR comments). Only the
   account's **owner** (or root) may use that token — see §10. Set this up under
   Git; cross-ref [`./git.md`](./git.md).
4. **For local mode — nothing beyond a git repo.** The diff is computed locally
   (`git diff` against the chosen base); no hosting token is needed.
5. **Optional: a Jira account** if you want to attach the linked story as review
   context (PR mode). The issue account's owner is enforced the same way (§10).
6. **Workspace role.** Starting/retrying a review and approving/declining
   comments require **Editor**; reading reviews requires **Viewer** (§10).

---

## 3. Running a review

### 3.1 PR mode (review a pull request)

1. Open the repo in **Git**, go to **Pull Requests**, open a PR, select the
   **Review** tab.
2. *(Optional)* **+ Attach Jira story** — pick the linked issue; its
   key/summary/status/description are fetched and prepended to every lens prompt
   as context.
3. *(Optional)* In **"What should the reviewers focus on?"** type free-text
   guidance; it is prepended to every lens prompt under a "Reviewer guidance"
   block.
4. *(Optional)* **⚙ Configure** to edit lenses/providers/summarizer (§7).
5. Click **Send to review agents**.

What happens:

- `POST /repos/{id}/prs/{number}/review` with an optional `StartReviewReq` body
  `{ issue_account_id?, issue_key?, context? }`.
- A workspace **budget gate** runs first; if usage enforcement is on and the cap
  is exceeded the request is rejected before any session spawns.
- A `Review` row is created with `status: "running"` and returned immediately; a
  `review_changed` WS event (status `running`) fires.
- In the background Otto resolves the provider, **fetches the PR diff**, renders
  it to a unified diff **capped at 200 KB** (over-cap files are flagged
  `too_large` per file), optionally fetches the Jira story, then runs the
  fan-out (§4).

### 3.2 Local (working-tree) mode

1. Open the repo in **Git**, select the repo-level **Review** tab.
2. Choose a base in **"Compare to"** (defaults sensibly to
   `origin/develop` → `origin/main` → `origin/master` → local `develop`/`main`/
   `master`).
3. Click **Review changes**.

What happens:

- `POST /repos/{id}/local-review` with body `{ "base": "<ref>" }`.
- Otto runs `git diff` of the working tree against `base`. **If there are no
  changes** the run completes immediately as `done` with zero findings (note
  "No changes vs `<base>`"). Otherwise the same fan-out (§4) runs on that diff.
- Local reviews are stored with a sentinel `pr_number = 0`; `GET/POST
  .../local-review` always operate on that latest local run.

> Local mode uses the **same configured agents** as PR review (the
> `LocalReviewPanel` links to PR-review config). The only difference is the diff
> source and that local reviews are not posted back to any host (there is no PR);
> findings are acted on by **handoff** (§6.4).

---

## 4. How agents are spawned (the fan-out)

Implemented in `run_review_core` (`modules.rs`) + `run_agent_session*`
(`review_session.rs`):

1. **Load `ReviewConfig`** (§7) — the stored config or the built-in default.
2. **Expand (lens × provider) into runs.** For each configured agent, its
   effective provider list is `providers` when non-empty, else `[provider]`.
   Each entry becomes one **run**. When a lens has multiple providers the run is
   labelled `"<lens name> · <provider>"`; otherwise just the lens name. The
   summarizer is appended as a final, non-reviewer row.
3. **Seed live state.** Every run gets a `ReviewAgentState` row
   (`status: "pending"`), persisted so the UI shows the agent list instantly.
4. **Pre-trust the repo folder** for every provider that will run (reviewers +
   the claude summarizer) so no agent stalls on the interactive "trust this
   folder?" prompt.
5. **Write the diff to a temp file** (`otto-review-<id>.diff`). Agents *read the
   file themselves* rather than having a huge diff pasted into the prompt.
6. **Spawn each run concurrently** as a real agent session (`SessionKind::Agent`,
   tagged `meta.source = "review"`, cwd = the repo path). Otto waits for the TUI
   to settle (up to ~40 s for a cold concurrent spawn), pastes the lens prompt
   (bracketed paste so multi-line stays one message), submits, and re-sends
   Enter once if the first submit was dropped.
7. **Each agent is strictly read-only by contract.** The injected prompt forbids
   editing/creating/deleting files or running mutating/build/test commands, and
   instructs the agent to review **only** the diff file (no `git`, no diffing
   against other branches), then write its findings as a JSON array to a
   per-agent temp file (`otto-review-<id>-<index>.json`). The findings file is
   the only write it may perform; for claude its JSONL transcript is a fallback
   capture path.
8. **Collect + summarize.** When all runs finish (or fail/time out), their
   findings batches are fed to the **summarizer** agent (`Orchestrator::run_agent`,
   120 s), which dedupes and returns **at most 20** items ranked by severity. The
   parsed comments are stored as **draft** `ReviewComment`s; the run is marked
   `done` (or `error`) and a `review_changed` event fires.

The findings JSON schema each agent emits (and the summarizer returns) is:

```json
[{ "path": "src/x.rs", "line": 42, "severity": "info|warn|bug", "body": "…" }]
```

---

## 5. Lenses & providers

### 5.1 Lenses come from skills

The Configure modal's **"+ Add preset"** menu is built **data-first from your
installed `review`-category skills** (`GET /library/skills`, filtered to
`category === 'review'`). Adding such a preset creates a lens whose prompt is
*"Apply the `<skill>` skill (it is available to you): <description> …"* plus the
strict JSON-output instruction — so the agent follows that skill's full method,
not just a one-line hint. Only if **no** review skills are installed does the
menu fall back to a hardcoded curated list (Security, Performance, Correctness,
Tests, Error handling, Concurrency, API design, Readability, Documentation,
Dependencies).

Bundled `review` skills (`crates/otto-skills/assets/skills/review/`):

| Skill | Lens focus |
|---|---|
| `grill` | Relentless, exhaustive, adversarial all-lens sweep down to nits |
| `correctness-review` | Correctness bugs only — logic, off-by-one, invariants, null handling |
| `security-review` | Taint-traced security: injection, authz/IDOR, secrets, SSRF, path traversal |
| `performance-review` | Cost-at-scale: N+1, O(n²), allocations, blocking I/O, missing indexes |
| `architecture-review` | Design/structure: SOLID, coupling, boundaries, abstractions |
| `devex-review` | API ergonomics: is this pleasant + safe for the next developer to use |
| `test-review` | Test quality: would this test fail if the code were broken |

The default `ReviewConfig` (when none is stored) ships **two lenses** —
"Correctness & bugs" and "Security & error handling" — both on the resolved
default provider, plus a claude summarizer. Installing/adding skill-based lenses
is how you extend it.

### 5.2 Providers

Each lens lists the CLIs to run it on (`providers`). UI options are `claude`,
`codex`, `agy`; the runner accepts any installed provider name. A lens with
`providers: ["claude","codex"]` spawns **two** agents (one each). The
`provider` field is kept in sync with `providers[0]` for backward compatibility.
The **summarizer** runs on its own provider (default `claude`); changing it is
allowed but the claude PTY runner is the supported path.

### 5.3 Models

Each agent has an optional `model` hint (`""` = provider default; e.g.
`haiku`/`sonnet`/`opus` for claude). Empty means the CLI's default model.

### 5.4 How a lens skill reaches each provider

Review sessions deliberately **skip** the normal context-materialization hook, so
the lens skill is delivered two ways, and the channel that matters differs by CLI:

1. **Inline in the prompt (ALL providers).** The lens skill's full method (body +
   `references/`) is resolved (`resolve_skill_inline`) and prepended to the agent
   prompt by `compose_review_lens_prompt`, fronted by a directive that **names the
   lens and makes it authoritative**: apply exactly this method, do not search for
   or substitute a *different* review skill (invoking the named lens itself is
   fine). This is the load-bearing channel for `codex` and `agy`.
2. **First-class `--add-dir` skills bundle (CLAUDE ONLY).** The lenses are also
   staged into a shared bundle laid out as `.claude/skills/<name>/`
   (`stage_review_skills`) and wired via `meta.extra_dirs` → `--add-dir` — but
   **only for claude** (`review_session::review_skills_extra_dirs`), the one CLI
   that first-class-loads `.claude/skills` from an added dir, so its reflexive
   `Skill(<lens>)` resolves.

Why claude-only for the bundle: `codex` has no first-class out-of-tree skills and
is spawned with `--search`; handed a `.claude/skills` bundle of *all* lenses it
would scavenge it and run the **wrong** skill instead of the indicated lens (the
original "codex used `review-skill`, not the one we indicated" bug). `agy` loads
`.agents/skills`, not the claude layout, so the bundle is inert for it. Both rely
on the inline method (1) instead — equivalent in outcome, and now authoritative.

---

## 6. Reading & acting on findings

### 6.1 Live progress

While `status === "running"` the panel shows one card per agent
(`ReviewAgents.svelte`) with a status pill:

| Pill | Meaning |
|---|---|
| `pending` | Seeded, not yet started |
| `running` | Working (spinner) |
| `waiting` | Looks **blocked on input** (~45 s idle with no findings) — **Open** it and respond |
| `done` | Finished; shows "N findings" |
| `error` | Failed after exhausting retries; the note says why (stuck / timed out / exited / could not start) |

The card's **note** is a short preview. The panel refreshes primarily off the
`review_changed` WS event; a visibility-gated fallback poll keeps it alive if the
socket drops.

### 6.2 Open an agent's session

Each non-summarizer agent that has spawned shows an **Open** button that mounts
its live terminal inline. The session is a normal Otto agent session: you can
read its output and type into it (e.g. approve a folder-access prompt that the
prompt-guard couldn't auto-accept) to unblock a `waiting` agent. See
[`./agent-sessions.md`](./agent-sessions.md).

### 6.3 Retry a failed/stuck agent

Each reviewer (not the summarizer) has a **Retry** button →
`POST /reviews/{review_id}/agents/{index}/retry`. It kills the agent's old
(likely stuck) session, marks the row `pending` ("retrying…"), and re-runs
**exactly that agent** in the background using the prompt persisted when the run
started (`otto-review-<id>-<index>.prompt`). If that prompt file is gone (e.g.
old run) the retry returns an error asking you to re-run the whole review. After
retry the panel resumes tracking the re-run.

This is **on top of** automatic recovery: each agent already auto-retries up to
`max_attempts` (default **3**) total attempts with a short backoff, killing the
prior session between tries (PR-review agents are autonomous, so unlike
interactive chat sessions they *are* safely auto-retried).

### 6.4 Acting on the aggregated findings

When the run is `done`:

- **PR mode** — the summarizer's deduped draft comments are listed with a
  severity chip, file:line, the body, and a collapsible diff snippet (±3 context
  lines). Per comment:
  - **Approve** → `POST /pr-review-comments/{cid}/approve`: posts the comment to
    the PR via the provider API **and** appends it to
    `<repo>/.otto/pr-<n>-review.md`. The chip becomes "posted". If the provider
    post fails (rate limit, no token) the comment is still recorded locally as
    approved with `posted=false`.
  - **Decline** → `POST /pr-review-comments/{cid}/decline`: discards the draft.
- **Local mode** — findings are listed with checkboxes. Select any subset and
  **Send to agent ▾** (choose a provider) → `POST /reviews/{review_id}/handoff`
  with `{ provider, comment_ids }`. Otto spawns a **new** agent session titled
  "Fix review findings", builds a brief listing each selected finding
  (`path:line [severity] body`), and injects "fix the valid ones, then summarize
  what you changed". You're navigated to that session. (PR-mode handoff uses the
  same endpoint; when `comment_ids` is omitted all non-declined comments are sent.)
- **Per-agent breakdown** (both modes) — expand any agent to see *its own*
  pre-summarization findings, each with a severity chip and (when persisted) a
  lifecycle-state chip.
- **Merge readiness** (PR mode) — a banner + panel assembled from
  `GET /reviews/{review_id}/merge-readiness`: open vs total findings, open
  bug-severity **blocker count**, CI status, human approvals, mergeable flag.
  The verdict and `blocker_count == 0` drive a "Merge-ready" vs "N blockers
  before merge" gate.
- **Persistent findings + lifecycle** — `GET /reviews/{review_id}/findings`
  returns fingerprinted `ReviewFindingRow`s with a `state` ∈ `open | fixing |
  resolved | regressed | declined`; `POST /reviews/{review_id}/findings/{fingerprint}/state`
  transitions it (optionally linking the `fix_session_id` doing the fixing).
  Fingerprints dedupe the *same* finding across re-runs.
- **History** — both panels list **Past reviews** (every stored run except the
  active one), expandable to the comments/findings of that older run.

### 6.5 Grace period

The per-agent **grace period** (`timeout`) bounds how long a single agent may run
before it's marked stuck/failed. By default it scales with diff size
(`review_agent_timeout`): **10 min** for a small diff (< 4 KB), **20 min**
(< 20 KB), **30 min** otherwise. An explicit `ReviewConfig.timeout_secs`
overrides the heuristic for every agent. Two faster thresholds sit inside the
grace window: at ~45 s of silence with no findings the agent is flagged
`waiting` (so a human can Open it), and at ~3 min of total silence it's treated
as **stuck** and fails fast into the auto-retry path rather than waiting out the
full grace period.

### 6.6 Clean-slate Review tab (commit `e9208802`)

The **local** Review tab opens to a **clean slate**. On mount Otto re-adopts a
review as the active panel **only if it is still `running`** (so an in-flight run
survives navigating away and back, with polling resumed). A finished
(`done`/`error`) run is **never** resurrected as the active view — its findings
were computed against an earlier working tree that may have since been edited,
committed, or discarded, so showing them as current would be misleading. Every
stored run lives under **Past reviews** instead. (The PR Review panel adopts the
newest run for the PR, since a PR diff is stable across navigations.)

---

## 7. PR-review configuration reference

Stored in the `settings` table under key `pr_review`; read/written via:

| Method & path | Auth | Body | Response |
|---|---|---|---|
| `GET /settings/pr-review` | root | — | `ReviewConfig` (stored, else default) |
| `PUT /settings/pr-review` | root | `ReviewConfig` | the saved config |

> Editing happens **in the Configure-agents modal** inside the Review panel
> (which calls these endpoints), not in a standalone settings page. `GET` is
> readable by any authenticated user; **`PUT` requires root.** The config is
> global (not per-workspace).

`ReviewConfig` (`crates/otto-core/src/api.rs`):

| Field | Type | Meaning |
|---|---|---|
| `agents` | `ReviewAgentCfg[]` | The reviewer lenses (each expands to one run per provider) |
| `summarizer` | `ReviewAgentCfg` | The dedupe/merge agent (its `providers` list is ignored; runs on `provider`, default `claude`) |
| `custom_presets` | `ReviewAgentCfg[]` | Your saved reusable lenses offered in the "+ Add preset" menu; **persisted but not run** |
| `max_attempts` | `number \| null` | Max total attempts per agent (initial + retries). `null` → **3** |
| `timeout_secs` | `number \| null` | Per-agent grace period in seconds; **overrides** the diff-size heuristic |

`ReviewAgentCfg`:

| Field | Type | Meaning |
|---|---|---|
| `name` | `string` | Display name / lens label |
| `provider` | `string` | Back-compat single provider; kept in sync with `providers[0]` |
| `providers` | `string[]` | CLIs to run this lens on (one run each). Empty → `[provider]` |
| `model` | `string` | Model hint; `""` = provider default |
| `prompt` | `string` | The lens instructions — what this agent looks for |

In the modal you can: add/remove lenses, toggle each lens's **Run on (CLIs)**
checkboxes (always ≥1), edit its **Lens / instructions** prompt, **Save as
preset** (persisted immediately), add a built-in/skill/your-own preset, and edit
the summarizer's name/provider/prompt. **Save** writes the whole config.

---

## 8. API & contract reference

Authoritative: `docs/contracts/api.md` (## PR review agents, ## PR-review
config) and `docs/contracts/ws.md` (## PR-review status change). All routes are
under `/api/v1`.

### Endpoints

| Method & path | Auth | Request | Response |
|---|---|---|---|
| `POST /repos/{id}/prs/{number}/review` | ws editor | `StartReviewReq` (optional) | `Review` (starts the fan-out) |
| `GET /repos/{id}/prs/{number}/review` | ws viewer | — | latest `Review` with live agent state |
| `GET /repos/{id}/prs/{number}/reviews` | ws viewer | — | `Review[]` (PR run history) |
| `POST /repos/{id}/local-review` | ws editor | `LocalReviewReq` `{ base }` | `Review` (working-tree diff) |
| `GET /repos/{id}/local-review` | ws viewer | — | latest local `Review` |
| `GET /repos/{id}/local-reviews` | ws viewer | — | `Review[]` (local run history) |
| `POST /pr-review-comments/{cid}/approve` | ws editor | — | `ReviewComment` (posts to PR + appends to `.otto/…`) |
| `POST /pr-review-comments/{cid}/decline` | ws editor | — | `ReviewComment` (discarded) |
| `POST /reviews/{review_id}/handoff` | ws editor | `HandoffReq` `{ provider, comment_ids? }` | `Session` (new "Fix review findings" agent) |
| `POST /reviews/{review_id}/agents/{index}/retry` | ws editor | — | `Review` (re-run one agent) |
| `GET /reviews/{review_id}/findings` | ws viewer | — | `ReviewFindingRow[]` (fingerprinted, with state) |
| `POST /reviews/{review_id}/findings/{fingerprint}/state` | ws editor | `{ state, fix_session_id? }` | updated `ReviewFindingRow` |
| `GET /reviews/{review_id}/merge-readiness` | ws viewer | — | merge-readiness object (findings + CI + approvals + mergeable) |
| `GET /settings/pr-review` | root | — | `ReviewConfig` |
| `PUT /settings/pr-review` | root | `ReviewConfig` | saved config |

### WebSocket event

`review_changed` (`/ws/events`, workspace-scoped) fires on every status
transition (`running → done | error`) so an open panel re-fetches immediately
instead of waiting for the fallback poll:

```json
{
  "type": "review_changed",
  "workspace_id": "<Id>",
  "session_id": "<session_id | null>",
  "review_id": "<review_uuid>",
  "status": "running|done|error"
}
```

(`session_id` is `null` for these review runs; the contract lists the broader
`queued|running|done|error|cancelled` set, but `ReviewStatus` in code is
`running|done|error`.) The UI matches `review_id` and short-cuts its back-off.

### Key DTOs (TS in `ui/src/lib/api/types.ts`)

- **`Review`** — `id, repo_id, pr_number, status, error?, comments[],
  agents[] (live ReviewAgentState), created_at, verdict?, blocker_count?,
  summary_md?`.
- **`ReviewComment`** — `severity: info|warn|bug`, `state: draft|approved|
  declined`, `posted`, `path?`, `line?`.
- **`ReviewAgentState`** — `name, provider, model, status, note, comment_count,
  session_id?, findings[]`.
- **`ReviewFindingRow`** — `fingerprint, path?, line?, severity, category?, body,
  state, fix_session_id?, updated_at`.
- **`StartReviewReq`** — `issue_account_id?, issue_key?, context?` (all optional;
  empty body is valid). **`LocalReviewReq`** — `{ base }`. **`HandoffReq`** —
  `{ provider, comment_ids? }`.

---

## 9. Capabilities & limitations

**Can:**

- Review a PR or the local working tree with multiple concurrent lens agents.
- Drive lenses from installed skills (one agent per lens × provider).
- Show each agent as an openable live session with per-agent findings, retry,
  and a `waiting`-on-input flag.
- Dedupe/prioritise into ≤20 ranked draft comments; post approved PR comments and
  log them to `.otto/pr-<n>-review.md`.
- Attach a Jira story and free-text guidance as review context (PR mode).
- Persist fingerprinted findings with a lifecycle state and assemble
  merge-readiness (findings + CI + approvals + mergeable).
- Hand findings to a fresh agent to fix them.

**Limitations / honest caveats:**

- **Diff cap.** PR diffs are rendered to **200 KB**; files beyond the cap are
  flagged `too_large` and not reviewed. Diffs fed to the *commit/PR drafting*
  helpers (a separate feature) are capped at 40 KB.
- **Summarizer is claude-bound.** The summarizer/drafting runner is the claude
  PTY driver; choosing a different summarizer provider isn't the supported path,
  and if claude is missing the summarizer degrades to concatenating per-agent
  findings.
- **LLM output, not ground truth.** Findings can include false positives;
  they're **drafts** you approve/decline. Severity is whatever the agent
  assigns.
- **No cancel button.** A running review has no manual stop; it ends when agents
  finish, error, or hit the grace period (auto-retries first). The
  `cancelled` status in the contract is reserved/unused by the PR-review runner.
- **Local reviews aren't posted anywhere** (no PR) — act on them via handoff.
- **Retry needs the persisted prompt** (`otto-review-<id>-<index>.prompt`); if
  it's gone you must re-run the whole review.
- **`custom_presets` are stored, not executed** — they only populate the
  "+ Add preset" menu.
- **Diff/prompt files live in the OS temp dir** (`otto-review-<id>*`) for the run.

---

## 10. Security & permissions

- **Workspace RBAC.** Reading reviews/findings/merge-readiness = **Viewer**;
  starting/retrying a review, approving/declining comments, handoff, and finding
  state-changes = **Editor**. `PUT /settings/pr-review` = **root**. See
  [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).
- **Credential ownership (S4).** A repo binds exactly one git account and a
  workspace can have many members, so the Editor check is not sufficient: the
  caller who starts/approves a review must **own** (or be root over) the repo's
  bound git account — and any attached Jira account — before its token is used.
  This is enforced in `resolve_provider_remote` / the Jira fetch, so a review
  never acts through another user's hosting/issue credentials.
- **Read-only agents.** The injected review prompt forbids any file edit, repo/
  index mutation, or build/test command; the only write an agent performs is its
  findings file. Agents are pre-trusted on the repo folder so they don't stall on
  a folder-trust prompt, and run with the providers' non-interactive permission
  posture. (The fix-it **handoff** agent is intentionally *not* read-only — it
  edits — and is spawned only on explicit user action.)
- **Run-as.** Review agent sessions run as the **root** user on the daemon (like
  channel sessions), but credential ownership above is still checked against the
  *caller*.
- **Budget gate.** With usage enforcement on, a review is rejected before any
  session spawns if the workspace cap is exceeded.
- **Secrets.** Tokens come from the macOS Keychain via opaque key references;
  the daemon listens on loopback only.

---

## 11. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Agent stuck on `waiting` | It's blocked on an input prompt the guard couldn't auto-accept. **Open** the agent and respond, or **Retry**. |
| Agent ends `error: stuck — no output for ~3m` | The CLI never produced output. Retry; check the CLI is installed/logged in. |
| Agent ends `error: timed out (grace period elapsed)` | Diff too large/slow for the grace window. Raise `timeout_secs`, narrow the diff, or split the PR. |
| `error: could not start` | Session creation failed — provider not installed, or budget gate hit. |
| Review starts but **0 comments** | Summarizer found no findings, or the summarizer (claude) failed and per-agent batches were empty. Check claude is logged in; expand per-agent cards to see raw findings. |
| "this agent's prompt is no longer available — re-run the review" | The persisted per-agent prompt file is gone (old run). Re-run the whole review. |
| Approve says posted but no comment on the PR | Provider post failed (rate limit / token / scope); the comment is recorded locally (`posted=false`). Verify the bound git account's token can write PR comments. |
| `403`/ownership error starting a PR review | You don't own the repo's bound git account (or attached Jira account). Bind/own the account or have its owner run it (S4). |
| `402` budget-exceeded | Workspace usage cap reached; raise/adjust the budget. |
| Some PR files not reviewed | Diff exceeded the 200 KB render cap (`too_large`). Review those files in a separate, smaller change. |
| Local Review tab shows "No active review" after a finished run | **Expected** — finished local runs are kept under **Past reviews**, not resurrected (§6.5). Click **Review changes** for a fresh run. |
| "N review skills aren't installed" banner | Non-blocking; install/update via **Settings → Skills** to enable those one-click lenses. |

---

## 12. Related docs

- [`./git.md`](./git.md) — repos, git accounts/tokens, PRs, diffs (PR-mode
  prerequisites and the surface that hosts the Review tabs).
- [`./skills-library.md`](./skills-library.md) — installing/updating the
  `review`-category skills that drive the lens menu.
- [`./agent-sessions.md`](./agent-sessions.md) — agent sessions (each review
  agent is one), opening terminals, providers/CLIs.
- [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — workspace roles and
  credential-ownership (S4) enforced here.
- `docs/contracts/api.md`, `docs/contracts/ws.md` — authoritative endpoint + WS
  contract.
