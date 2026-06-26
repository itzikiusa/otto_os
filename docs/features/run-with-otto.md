# Run with Otto

The flagship **one-button** flow. A single source item — a **Jira** story, a
**Confluence** page, a **GitHub** issue or PR, a **Slack/Telegram** thread, a
**Product** task, a **review finding**, a **failing test**, or a **scheduled-task
report** — becomes a tracked **`OttoRun`** that drives a fixed pipeline end to end:

> **source item → Otto Work Item → Context Packet → isolated branch/worktree →
> Goal Loop *or* single agent → Proof Pack → AI review findings → human approval →
> PR draft.**

It is meant to feel like **one button, not eight modules**: the eight subsystems
that already exist (context assembly, worktrees, goal loops, proof packs, AI review,
the findings workflow, PR drafting, Mission Control) are chained behind a single
entity and a single trigger. It is **workspace-scoped** and, in v1, **primarily
triggered from Slack** (with a generic webhook, a REST API, and a UI launcher for
everything else).

This guide documents what the code in `crates/otto-server/src/run_engine.rs`,
`run_service.rs`, `run_scheduler.rs`, `run_sources.rs`, `run_context.rs`,
`run_workspace.rs`, `run_channels.rs`, `routes/runs.rs`,
`crates/otto-state/src/runs.rs` (migration `0085_run_with_otto.sql`),
`crates/otto-core/src/run.rs`, and `ui/src/modules/run-with-otto/` actually does.
The API shape is specified in `docs/contracts/api.md` (Run with Otto) and
`docs/contracts/ws.md` (`otto_run_updated`), which remain authoritative.

---

## 1. Summary

| | |
|---|---|
| **What it is** | One trigger turns any of eight sources into a reviewed, evidence-backed **PR draft**, gated by human approval. |
| **Entity** | `OttoRun` (`otto_runs` + `otto_run_events`, migration **0085**). Its `status` **is** the pipeline stage machine (`otto_core::run::RunStatus`). |
| **Stages** | `queued → resolving_source → building_context → provisioning → executing → proving → reviewing → awaiting_approval → drafting_pr → completed` (+ `failed`/`rejected`/`cancelled`). |
| **Execute modes** | `single_agent` (a headless `Orchestrator::run_agent` on an `otto-run/<id>` worktree — the default) or `goal_loop` (a full Plan→Execute→Evaluate→Digest loop). |
| **Triggers** | **Slack/Telegram** (`/run <ref>`), **webhook** (`POST /webhooks/{ws}/run`), **REST** (`POST /workspaces/{ws}/runs`), and the **UI launcher**. All funnel into `RunService::launch`. |
| **Approval** | The run pauses at `awaiting_approval`; resolve with the UI/REST buttons or an `approve`/`reject` reply in the Slack/Telegram thread. |
| **PR** | Produces a **PR draft** (title + description + pushed branch) by default; opening the actual PR is an explicit, opt-in action that requires approval + a passed/waived proof pack. |
| **Traceability** | Each run projects into **Mission Control** as `WorkKind::OttoRun`. |
| **RBAC** | `Feature::RunWithOtto` (`run_with_otto`): View for reads, Edit for launch/approve/cancel/open-pr; every route also enforces the workspace role. |
| **WS event** | `otto_run_updated` (`{run_id, status}`), workspace-scoped. |

---

## 2. Where it lives & how the pieces fit

| Layer | File | Responsibility |
|---|---|---|
| **Stage machine** | `crates/otto-core/src/run.rs` | Pure `RunStatus` (`next_on_success`/`is_terminal`/`is_resumable_on_boot`), `SourceKind`, `RunMode`, `RunOrigin`, the DTOs, and the pure `parse_source_ref` source detector. |
| **Persistence** | `crates/otto-state/src/runs.rs` + migration `0085_run_with_otto.sql` | `RunsRepo` — compare-and-set status transitions, the COALESCE patch, the timeline, the thread-bound awaiting lookup, and the resumable/interrupted boot queries. |
| **Source adapters** | `crates/otto-server/src/run_sources.rs` | The eight `resolve_source` adapters → a unified `ResolvedSource`, plus `resolve_repo` (the registered git repo the run works in). |
| **Context packet** | `crates/otto-server/src/run_context.rs` | `build_packet` — the task prompt (redacted, capped). |
| **Worktree** | `crates/otto-server/src/run_workspace.rs` | Idempotent `otto-run/<id>` worktree provisioning + removal. |
| **Engine** | `crates/otto-server/src/run_engine.rs` | `advance` — the stage driver with the per-run in-flight guard + CAS transitions; each stage; Mission Control projection; Slack/Telegram origin posting. |
| **Service** | `crates/otto-server/src/run_service.rs` | `launch` (the funnel), `approve`/`reject`, `cancel`, `open_pr`. |
| **Scheduler** | `crates/otto-server/src/run_scheduler.rs` | Boot reaper (fail interrupted, re-drive resumable) + a 30 s tick. |
| **HTTP routes** | `crates/otto-server/src/routes/runs.rs` + `routes/channel_webhook.rs` | The `/runs` REST surface + the key-guarded `POST /webhooks/{ws}/run`. |
| **Channel trigger** | `crates/otto-server/src/run_channels.rs` + `otto-channels::run_trigger` | `/run <ref>` launches; `approve`/`reject` replies resolve the gate (injected into the channel `Bridge`). |
| **UI** | `ui/src/modules/run-with-otto/` | The launcher (the one button), the runs list, and the run detail (timeline, proof, approval, PR draft). |

### 2.1 Data flow

```
trigger (Slack /run · webhook · REST · UI)
        │
        ▼  RunService::launch
   otto_runs (status=queued)  ──projects──▶ Mission Control (WorkKind::OttoRun)
        │
        ▼  run_engine::advance  (in-flight guard + compare-and-set per stage)
 resolving_source → building_context → provisioning → executing
        │                                                   │ single_agent: run_agent on otto-run/<id>
        │                                                   │ goal_loop:    start_loop, adopt its branch
        ▼
   proving (proof::gate + assemble_diff + recompute) → reviewing (local AI review)
        │
        ▼
   awaiting_approval  ──posts summary to the Slack/Telegram thread, then PAUSES──
        │   approve (UI/REST or "approve" reply)        reject → rejected
        ▼
   drafting_pr (draft_pr_core + best-effort push) → completed
```

---

## 3. The eight entry points

Every source is normalized by an adapter in `run_sources.rs` into a `ResolvedSource`
(`title`, `body_md`, derived `goal`). All source bodies are redacted
(`otto_core::redact`) and capped.

| Source | `source_kind` | How it resolves |
|---|---|---|
| Jira story | `jira` | `JiraClient::get_issue_full(key)` → summary + description + top comments. |
| Confluence page | `confluence` | `ConfluenceClient::get_page(id)` → title + (tag-stripped) storage body. |
| GitHub PR | `github_pr` | `provider.get_pr(remote, n)` → description + discussion. |
| GitHub issue | `github_issue` | `GitHub::get_issue(remote, n)` (GitHub-only helper). |
| Slack/Telegram thread | `channel` | The trigger's seed message (no extra fetch in v1). |
| Product task | `product_story` | `ProductRepo::get_story(id)`. |
| Review finding | `finding` | `ReviewFindingsRepo::get_full(id)` → title + evidence + suggested fix. |
| Failing test | `test` | `ProductRepo::get_testcase_run(id)` + the run's failing test cases. |
| Scheduled-task report | `scheduled_report` | `ScheduledTasksRepo::get_run(id)` → the stored report markdown. |

A single free-text field auto-detects the kind via `parse_source_ref`: a bare Jira
key (`PROJ-123`), a GitHub `…/pull|issues/N` URL, a Confluence `…/pages/<id>` URL, or
an explicit `finding:`/`story:`/`test:`/`report:`/`jira:`/`confluence:` prefix.
Anything else (or any "describe what you want" text) becomes a **channel** run whose
goal is the text itself.

### 3.1 Repo resolution

A run always works inside a **registered git repo** (never a raw workspace root).
`resolve_repo` picks it: an explicit `repo_id` → a source-implied repo (the GitHub
remote, or a finding's `repo_id`) → the workspace's single (or first) registered repo
→ a clear error ("no git repo registered in this workspace; register one or pass
repo_id").

---

## 4. The stages

The `status` column **is** the stage. The engine advances success-only via
`RunStatus::next_on_success`, so the order is a single source of truth it cannot drift
from. Every transition is a compare-and-set (`set_status_cas`) guarded by a per-run
in-flight lock, so a racing boot reaper or a double-approve can never double-advance.

- **resolving_source** — fetch + normalize the source; resolve the repo.
- **building_context** — assemble the task prompt (the agent's own soul/skills/memory
  are injected separately at spawn by the out-of-tree context system).
- **provisioning** — create the isolated `otto-run/<id>` branch + worktree
  (`worktree_add_if_absent`, idempotent). Goal-loop mode defers to the loop's own
  `goal-loop/<id>` worktree.
- **executing** — `single_agent`: `Orchestrator::run_agent` in the worktree, which
  commits its change. `goal_loop`: create + `start_loop`, poll to a terminal state,
  then re-attach a worktree on the loop's branch for the rest of the pipeline.
- **proving** — `proof::gate(WorkItemKind::Task, …)` + `assemble_diff` (the run's
  `base..HEAD` diff) + a self-review artifact + `recompute_and_emit`; the derived
  status + risk are snapshotted onto the run.
- **reviewing** — launch a local AI review on the run's branch; snapshot the findings
  counts (`run_review_for_branch` + `review_findings_counts`).
- **awaiting_approval** — the only pause; the engine posts a summary to the origin
  thread and waits.
- **drafting_pr** — `draft_pr_core` (diff + the `pull-request` skill) → a stored draft;
  the branch is pushed best-effort.

---

## 5. Triggers

All four surfaces call `RunService::launch`.

- **Slack / Telegram** — a `/run <ref>` message (or "run with otto …") launches a run;
  an `approve`/`reject` reply in the run's thread resolves the gate. The bridge's
  existing `allowed_users` allowlist authorizes the chat user; the action runs as the
  daemon root user. Live status + the approval prompt + the final result are posted
  back to the thread.
- **Webhook** — `POST /api/v1/webhooks/{workspace_id}/run`, authenticated by the same
  per-workspace `X-Otto-Webhook-Key` as the channel webhook; returns `202 {run_id}`.
- **REST** — `POST /api/v1/workspaces/{wid}/runs` (the canonical programmatic entry).
- **UI** — the launcher on the **Run with Otto** page (sidebar, gated by the
  `run_with_otto` feature).

---

## 6. Approval & the PR draft

The run halts at `awaiting_approval` until a human decides. **Approve** advances to
`drafting_pr`; **reject** ends the run as `rejected` (the worktree is cleaned up; the
branch and its commits survive). The PR is produced as a **draft** — opening the real
PR is a separate, opt-in action (`POST /runs/{id}/open-pr`) that requires the run to
be approved **and** its proof pack to be `passed`/`waived` (mirroring the proof-pack
PR gate). This honors the repo rule that outward-facing actions are never taken
without explicit approval.

---

## 7. Capabilities & limitations (v1, honest)

**It does**: accept all eight sources; normalize each into one entity; build a task
packet; isolate work on a per-run branch/worktree; run either a single agent or a full
goal loop; assemble a proof pack; run AI review; gate on human approval; and produce a
PR draft — all workspace-scoped, with Mission Control traceability, driven from Slack,
a webhook, REST, or the UI.

**Deferred / scoped (documented, not hidden):**

- **No MCP tool surface yet.** An `otto.run` MCP tool is a planned follow-up; v1 is
  Slack-primary + webhook + REST + UI.
- **"Failing test" = a Product QA testcase run.** That is the only "test" entity the
  codebase models; CI-job / unit-test-failure ingestion is a follow-up.
- **GitHub issues only.** GitLab/Bitbucket *issues* aren't fetched in v1 (their PRs
  are).
- **Channel source = the trigger's seed message.** Deep Slack/Telegram thread-history
  ingestion is a follow-up.
- **Webhook result callback is minimal.** The inbound trigger returns `202 {run_id}`;
  the result is read via REST/WS/UI (a richer `callback_url` push is a follow-up).
- **Goal-loop mode** is wired and unit-tested, but the deterministic E2E exercises the
  `single_agent` path (the goal-loop controller spawns live agent PTYs the offline
  test stub can't satisfy).
- **Approval via Slack** uses thread `approve`/`reject` replies (Socket Mode), not
  interactive Block-Kit buttons.

---

## 8. Security

- Every route is workspace-scoped with `require_ws_role` IDOR guards; flat by-id routes
  load the run and re-check the role on its workspace.
- The webhook entry is key-guarded (constant-time) and classified `Exempt` in
  `policy.rs`; the channel approval path is gated by the integration's `allowed_users`.
- All source bodies and delivered summaries run through `otto_core::redact`.
- The agent works in an **isolated worktree branch** — never the user's checkout or
  branch.
- No PR is opened without approval + opt-in + a passing/waived proof pack.
- Loopback-only defaults are unchanged; no new network listeners.

---

## 9. Troubleshooting

- **"no git repo registered in this workspace"** — register a repo (Git tab) or pass an
  explicit `repo_id` on launch.
- **A run `failed` during `executing`/`reviewing` after a restart** — the boot reaper
  fails runs caught mid live-work (their agent/review processes are gone); the branch's
  commits are preserved, so just relaunch.
- **The PR draft says "branch not pushed"** — the repo has no authorized git account;
  the draft (title/description) is still produced. Bind a git account to push/open.
- **`open-pr` returns 409** — the run isn't approved, or its proof pack isn't
  `passed`/`waived`.
- **A Slack `approve` reply did nothing** — there must be a run currently
  `awaiting_approval` bound to that exact `(workspace, channel, thread)`, and the
  replying user must be in the integration's `allowed_users`.
