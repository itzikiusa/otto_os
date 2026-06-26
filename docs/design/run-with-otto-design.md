# Run with Otto — Design

> Status: design (2026-06-26). Worktree `feat/run-with-otto`. Built autonomously
> per the user's "design in depth → review → plan → implement → E2E → merge →
> deploy" brief. This doc is the source of truth for the feature's shape; the API
> contract in `docs/contracts/{api,ws}.md` is authoritative for the wire shapes.

## 1. The one sentence

**Run with Otto** turns any of eight source items into a single, traceable,
evidence-backed change proposal — *one button*, not eight modules. A source item
(Jira / Confluence / GitHub issue or PR / Slack or Telegram thread / Product task /
Review finding / Failing test / Scheduled-task report) is normalized into an **Otto
Work Item**, given a **Context Packet**, worked on an **isolated branch/worktree**
by a **Goal Loop or a single agent**, fronted by a **Proof Pack**, run through **AI
review**, paused for **human approval**, and finished as a **PR draft**. It is
**workspace-scoped** and, in v1, **primarily triggered from Slack** (with a generic
webhook and REST/MCP entry for everything else).

## 2. Why a new entity, and what it reuses

Almost every stage already exists as a subsystem. The gap the user named is the
*spine*: a single object that owns the pipeline end-to-end and a single trigger
that starts it. So we add exactly one new first-class entity — **`OttoRun`** — and
wire it to the existing machinery rather than reimplementing any of it.

| Stage | Reuses (verified file:line) |
|-------|------------------------------|
| Source → Work Item | `otto-issues` `JiraClient::get_issue_full` / `ConfluenceClient::get_page`; `otto-git` `provider.get_pr`; `otto-product` `get_story` / `get_testcase_run`; `otto-state` `ReviewFindingsRepo::get_finding`, `ScheduledTasksRepo::get_run`; channel `Inbound` seed text |
| Context Packet | `otto-server::mcp_capabilities::context_packet` (code-grounded excerpts) + resolved source body; spawn-time soul/skills/memory handled by existing `PreSpawnHook` |
| Isolated branch/worktree | `otto-git::LocalGit::worktree_add{,_if_absent}`; mirrors `goal_loop_workspace::provision_worktree` and `finding_agent::provision_worktree` |
| Goal Loop or single agent | `goal_loop::start_loop` (mode=goal_loop) **or** `Orchestrator::run_agent` (mode=single_agent) |
| Proof Pack | `otto-server::proof::{gate,assemble_diff,run_command_artifact,upsert_content_artifact,recompute_and_emit}` (`WorkItemKind::Task`) |
| AI review findings | `otto-server::modules::start_local_review` / `run_review_core` → `review_findings` workflow |
| Human approval | new run-level gate (one approval per run), modeled on the findings approve/reject + workflow `human_approval` pause |
| PR draft | `otto-server::modules::draft_pr` internals (diff + `pull-request` skill + `run_agent`) + `LocalGit::push`; **does not auto-open** (outward-facing) unless explicitly opted in |
| One button | new `RunService::launch` funnel + Slack `RunTrigger` (mirrors `SwarmTrigger`) + webhook + REST + MCP |
| Traceability | projects into Mission Control via `WorkGraphService::record` as new `WorkKind::OttoRun` (kind is free TEXT — no migration churn) |

## 3. The entity: `OttoRun`

One row per run; the status field **is** the stage machine (no separate stage/status
split). Persistence: migration `0085_run_with_otto.sql`, repo `otto-state::RunsRepo`.

```
otto_runs
  id             TEXT PK (ULID)
  workspace_id   TEXT NOT NULL → workspaces(id) ON DELETE CASCADE
  title          TEXT NOT NULL
  source_kind    TEXT NOT NULL  -- jira|confluence|github_pr|github_issue|channel|product_story|finding|test|scheduled_report
  source_ref     TEXT NOT NULL  -- the handle (issue key / page id / pr number / id / thread ts)
  source_url     TEXT
  goal           TEXT NOT NULL  -- normalized task instruction
  mode           TEXT NOT NULL  -- single_agent|goal_loop
  provider       TEXT NOT NULL  -- claude|codex|agy (default = workspace default)
  repo_id        TEXT           -- optional; else workspace root
  repo_path      TEXT NOT NULL  -- resolved cwd (repo or workspace root)
  base_branch    TEXT NOT NULL
  branch         TEXT           -- otto-run/<id> (single_agent) or goal-loop/<id>
  worktree_path  TEXT
  base_commit    TEXT
  status         TEXT NOT NULL  -- the stage machine (see §4)
  error          TEXT
  origin_kind    TEXT NOT NULL  -- slack|telegram|webhook|ui|mcp|api
  origin_chat    TEXT           -- channel id (for thread replies)
  origin_thread  TEXT
  origin_user    TEXT
  callback_url   TEXT           -- webhook origin: where to POST the final result
  goal_loop_id   TEXT           -- when mode=goal_loop
  review_id      TEXT
  proof_pack_id  TEXT
  proof_status   TEXT           -- snapshot for list view (missing|partial|passed|failed|waived)
  risk_score     INTEGER        -- snapshot 0..100
  findings_total INTEGER NOT NULL DEFAULT 0
  findings_blocking INTEGER NOT NULL DEFAULT 0
  pr_draft_json  TEXT           -- {title,description,source_branch,target_branch}
  pr_url         TEXT           -- set only if a PR is actually opened
  auto_open_pr   INTEGER NOT NULL DEFAULT 0
  approval_decision TEXT        -- approved|rejected
  approved_by    TEXT
  approved_at    TEXT
  result_summary TEXT
  context_summary TEXT          -- truncated context packet, for transparency
  created_by     TEXT NOT NULL
  created_at     TEXT NOT NULL
  updated_at     TEXT NOT NULL

otto_run_events            -- append-only timeline (audit + Slack/feed source)
  id            TEXT PK
  run_id        TEXT NOT NULL → otto_runs(id) ON DELETE CASCADE
  workspace_id  TEXT NOT NULL
  kind          TEXT NOT NULL   -- stage_enter|stage_ok|stage_error|note|approval|delivery
  status        TEXT            -- the run status at the time
  message       TEXT NOT NULL
  detail_json   TEXT
  created_at    TEXT NOT NULL
```

Indexes: `(workspace_id, updated_at DESC)` on runs; `(run_id, created_at)` on events.

## 4. The stage machine (pure, in `otto-core`)

`RunStatus` enum + pure `next_on_success`, `is_terminal`, `can_transition`, fully
unit-tested in `otto-core` (no I/O), so the ordering is a single source of truth the
engine cannot drift from.

```
queued
  → resolving_source     fetch + normalize the source → ResolvedSource
  → building_context     assemble the Context Packet (task prompt + repo orientation)
  → provisioning         isolated branch/worktree (otto-run/<id> or goal loop's)
  → executing            run_agent (single) OR start_loop+await (goal loop)
  → proving              assemble Proof Pack (diff + tests + self-review) → derive status/risk
  → reviewing            launch local AI review on the branch → count findings
  → awaiting_approval    PAUSE — post summary to origin; wait for approve/reject
  → drafting_pr          generate PR draft (+ push branch; open only if opted-in)
  → completed
terminal: completed | failed | rejected | cancelled
```

`awaiting_approval` is the only pause. Reject → `rejected`. Cancel (any non-terminal)
→ `cancelled`. Any stage error → `failed` with `error` set. The engine advances
success-only via `next_on_success`; it never hard-codes the order inline.

## 5. Source adapters (`run_sources.rs`)

A `ResolvedSource { title, body_md, goal, source_url, repo_hint, metadata }` produced
by `resolve(ctx, run) -> ResolvedSource`, dispatched on `source_kind`:

- **jira** — `JiraClient::get_issue_full(key)`; title=summary, body=description+AC+top comments, goal="Implement/resolve <key>: <summary>".
- **confluence** — `ConfluenceClient::get_page(id)` + `adf`→markdown.
- **github_pr** — `provider.get_pr(remote, n)`; body=PR description + review threads; goal="Address PR #n".
- **github_issue** — minimal `GitHub::get_issue(remote, n)` (GitHub-only helper, not the trait) → title/body; goal="Resolve issue #n". Falls back to URL+passed text if the token/provider is unavailable.
- **channel** — no outbound fetch; uses the seed text captured by the trigger and stored on the run (`goal`/`context_summary`). Thread history fetch is a documented follow-up.
- **product_story** — `ProductRepo::get_story(id)` (+ latest version).
- **finding** — `ReviewFindingsRepo::get_finding(id)`; body=title+evidence+suggested_fix; goal="Fix finding: <title>". (Complements the existing `finding_agent` fix flow — Run with Otto wraps the *whole* pipeline incl. proof+re-review+PR.)
- **test** — `ProductRepo::get_testcase_run(id)` + `list_testcases`; body=failing cases; goal="Make the failing test(s) pass".
- **scheduled_report** — `ScheduledTasksRepo::get_run(id)`; body=report markdown (read from `report_path`); goal="Act on the scheduled report".

`parse_source_ref(s) -> Option<(SourceKind, ref, url?)>` (pure, unit-tested) lets a
single free-text field auto-detect: `PROJ-123` (jira), `…/pages/123` (confluence),
`github.com/o/r/pull|issues/N`, `finding:<id>`, `story:<id>`, `test:<id>`,
`report:<id>`, or an explicit `source_kind` override in REST/webhook.

## 6. Context Packet (`run_context.rs`)

`build_packet(ctx, run, resolved) -> ContextPacket` assembles the **task prompt** that
drives execute:

1. Goal (imperative) + acceptance hint.
2. Source body (truncated, redacted via `otto_core::redact`).
3. Repo orientation: workspace name/root + (optional) top code excerpts from the
   existing confined `context_packet`/`code_search` capability when a query is derivable.
4. Working agreement: "work on branch `<branch>`, commit your changes, then print a
   short summary of what you changed and how you verified it."

The packet is capped and stored truncated on the run (`context_summary`). The
*environmental* context (soul, installed skills, memory, repo rules, hooks) is injected
automatically at session spawn by the existing out-of-tree `PreSpawnHook` — we do not
duplicate it. The assembled prompt is what we feed `run_agent` / the goal loop's goal.

## 7. Provisioning & Execute

- **single_agent** (default, deterministic for E2E): `run_workspace::provision(ctx, run)`
  creates branch `otto-run/<id>` + worktree `<data_dir>/otto-runs/<id>/work` from the
  repo HEAD via `worktree_add`. Execute = `Orchestrator::run_agent(prompt, worktree,
  provider_model, NO_PROGRESS)`. The agent commits to the branch. We capture its reply
  as `result_summary`.
- **goal_loop**: create a `NewGoalLoop` (repo_path = run.repo_path, definition = goal +
  derived acceptance criteria, sensible limits) → `goal_loop::start_loop`. The loop owns
  `goal-loop/<id>` and its own Plan→Execute→Evaluate→Digest controller and proof pack.
  We poll the loop to a terminal state, then adopt its branch + proof pack for the
  remaining stages. Record `goal_loop_id`.

**E2E determinism:** under `OTTO_E2E=1`, `run_agent` returns a stubbed reply (the daemon
is started with a non-existent `CLAUDE_BIN`). To give the downstream proof/diff/PR stages
real, deterministic material, the engine — only when `OTTO_E2E=1` — writes a deterministic
`OTTO_RUN_NOTE.md` describing the run and commits it to the branch *after* the stub reply.
Production never does this. This mirrors the scheduled-tasks `OTTO_TASK` stub seam.

## 8. Proving

`proof::gate(ctx, WorkItemKind::Task, run_id, ws, title, created_by)` →
`assemble_diff(pack, worktree, base_commit)` → optional `run_command_artifact` for a
derivable test command (skipped/`true` under E2E) → `upsert_content_artifact(SelfReview,
result_summary)` → `recompute_and_emit`. Snapshot `proof_status`/`risk_score` onto the
run. For goal_loop mode the loop already assembled a `goal_loop` pack; we link it.

## 9. Reviewing

`start_local_review`'s core launches a local-diff AI review on `repo_id`+branch (PR #0
sentinel), returning a `review_id` we store. Findings flow into `review_findings`
(the full 6-state workflow + Proof Pack ingest already shipped). We snapshot
`findings_total` and `findings_blocking` (critical+high, status=open) onto the run.
Under `OTTO_E2E`, the review completes with the stub's deterministic (empty) findings;
the stage is wired and visible regardless of review depth.

## 10. Human approval

The run halts at `awaiting_approval` and the engine posts a summary to the origin
(proof status + risk + findings counts + "Approve to draft a PR, or reject."). Resolution:

- **REST/UI**: `POST /workspaces/{wid}/runs/{id}/approve {decision, note?}` (Edit role).
- **Slack/Telegram**: a reply of `approve` / `reject` (or `/run approve|reject`) in the
  origin thread, matched by `(workspace, chat, thread)` to the awaiting run via `RunTrigger`.
- **Webhook**: `POST /webhooks/{wid}/run/approve` (key-guarded) or the same REST route.

Approve → `drafting_pr`. Reject → `rejected` (worktree retained for inspection; reaped on
delete). Records `approval_decision`/`approved_by`/`approved_at` + an event.

## 11. Drafting PR

Reuse `draft_pr` internals: diff(branch vs base) + `pull-request` skill + `run_agent`
→ `{title, description}`; store as `pr_draft_json`. `LocalGit::push(token)` so the branch
is openable. **The PR is not opened automatically** (an outward-facing publish). If
`auto_open_pr=true` *and* approved *and* the proof pack is `passed|waived`, the engine
opens it via `provider.create_pr` honoring `gate_pr`, recording `pr_url`. Otherwise we
surface the draft + an "Open PR" affordance (UI button / Slack message with the compare
URL). Default is draft-only — the requirement says *PR draft*, and AGENTS.md forbids
unprompted outward actions. Then `completed`; post the final summary to the origin.

## 12. Engine & supervisor

- `run_engine::advance(ctx, run_id)` runs the next stage, transitions via
  `next_on_success`, emits `Event::OttoRunUpdated`, appends an event, posts a thread
  update, and re-arms itself until it hits `awaiting_approval` or a terminal state.
- `run_scheduler` (supervisor, spawned in `ottod`): a startup **reaper** re-drives
  non-terminal, non-awaiting runs (restart recovery), and a small tick picks up `queued`
  runs. Concurrency is bounded by a semaphore (`OTTO_RUN_MAX_CONCURRENT`, default 2).
- On approve, the approve handler calls `advance` to resume from `drafting_pr`.

## 13. The one button — `RunService::launch`

All surfaces funnel into `RunService::launch(req: LaunchRunReq, origin: RunOrigin)`:
creates the `queued` row, records the Mission Control work item, kicks `advance`, returns
the run. Surfaces:

1. **Slack/Telegram** — `RunTrigger` trait (in `otto-channels`, injected from
   `ottod/main.rs` exactly like `SwarmTrigger`). `bridge.rs` calls it for a `/run <ref>`
   command (and a "run with otto <ref>" mention). Implemented by
   `otto-server::run_channels::ChannelRunTrigger`. Also resolves `approve`/`reject`
   replies for an awaiting run bound to the thread. Replies/updates go back to the thread.
2. **Webhook** — `POST /api/v1/webhooks/{workspace_id}/run` (webhook-key guarded, reusing
   `channel_webhook`'s constant-time key check), body `{source_kind?, ref|url, mode?,
   provider?, auto_open_pr?, callback_url?}` → 202 + `run_id`. Final result delivered to
   `callback_url` (SSRF-guarded `WebhookAdapter`).
3. **REST** — `POST /workspaces/{wid}/runs` (canonical; UI + MCP use it). Plus
   `GET /workspaces/{wid}/runs`, `GET /runs/{id}`, `GET /runs/{id}/events`,
   `POST /runs/{id}/approve`, `POST /runs/{id}/cancel`, `POST /runs/{id}/open-pr`,
   `GET /run-with-otto/sources` (detect/preview a ref).
4. **MCP** — outward `otto.run` + `otto.run_status` tools (mirroring the scheduled-tasks
   MCP pattern; `otto.run` is DANGEROUS/off-by-default with `dangerous_detail` exposing
   the source + goal to the approver).

## 14. Mission Control projection

On create and each status change, `RunService` calls `WorkGraphService::record` with
`kind = WorkKind::OttoRun` (new variant; `work_items.kind` is free TEXT so no migration
churn), `source_id = run_id`, normalized status/risk, repo/branch. Edges: when the source
already has a work item (finding/product story/pr/external trigger), add a `fixes`/
`reviews`/`spawned` edge; link spawned `goal_loop`/`review`/`pr` items as they appear.
This is what makes it "one flow, not eight modules" inside Mission Control too. The
`WorkStatus` normalizer gets an `OttoRun` arm mapping run statuses → active/blocked/done.

## 15. RBAC, events, contracts

- **Feature**: `Feature::RunWithOtto` (snake `run_with_otto`). View=reads, Edit=launch/
  approve/cancel/open-pr. `policy.rs`: `/workspaces/{wid}/runs*` + `/runs/{id}*` +
  `/run-with-otto/*` → `Require(RunWithOtto, View|Edit)`; the `/webhooks/{wid}/run*`
  routes are public (key-guarded), added to the Exempt block like `channel_webhook`.
- **Event**: `Event::OttoRunUpdated { run_id, workspace_id, status }` → `ws.md`
  `otto_run_updated`. Scoped to the workspace.
- **Contracts**: new "Run with Otto" section in `api.md` (the ~10 routes) — required by
  the `route_inventory` test — and the WS event in `ws.md`. TS mirror in `types.ts`.

## 16. UI (`ui/src/modules/run-with-otto/`)

- **Launcher (the button):** one input — "Paste a Jira key, a Confluence/GitHub URL, or
  a finding/story/test id…" — with live source-kind detection (`/run-with-otto/sources`),
  a mode toggle (Single agent ⇄ Goal loop), provider, and an auto-open-PR checkbox, then
  **Run with Otto**. This is the literal one button.
- **Runs list:** status/stage chip, source badge, proof badge, risk, findings count.
- **Run detail:** the stage **timeline** (events), the context packet, the linked proof
  pack (reusing `ProofBadges`), the findings (link into the findings board), the
  **approval gate** (Approve / Reject when `awaiting_approval`), and the **PR draft**
  (title/description + "Open PR"). Live via the `otto_run_updated` WS event.
- Sidebar: `{ id:'run-with-otto', icon:'rocket', label:'Run with Otto', feature:'run_with_otto' }`.
  App.svelte route + command-palette entry. API client `lib/api/runWithOtto.ts`, store
  `lib/stores/runWithOtto.svelte.ts`.
- A small **"Run with Otto"** action is also surfaced on the Findings board and a Product
  story (cheap inline launchers that POST the central endpoint) to reinforce one-button.

## 17. Security & "do no harm"

- Everything workspace-scoped with `require_ws_role` IDOR guards on every route.
- Webhook routes key-guarded (constant-time), SSRF-guarded callback delivery.
- All source bodies + delivered summaries run through `otto_core::redact`.
- The agent runs in an **isolated worktree branch** — never the user's checkout/branch.
- **No outward action without approval**: PRs are drafts by default; opening a PR needs
  approval + opt-in + a passing/waived proof pack (`gate_pr`).
- MCP `otto.run` is DANGEROUS/off-by-default; the approver sees source+goal+mode.
- Loopback-only defaults unchanged; no new listeners.

## 18. Requirements traceability

| Requirement | Where met |
|---|---|
| Jira / Confluence entry | §5 jira, confluence |
| GitHub issue / PR entry | §5 github_pr, github_issue |
| Slack / Telegram thread entry | §5 channel + §13.1 |
| Product task entry | §5 product_story |
| Review finding entry | §5 finding |
| Failing test entry | §5 test |
| Scheduled task report entry | §5 scheduled_report |
| Source item → Otto Work Item | §3 `OttoRun` + §5 adapters |
| → Context Packet | §6 |
| → isolated branch/worktree | §7 |
| → Goal Loop or single agent | §7 (both modes) |
| → Proof Pack | §8 |
| → AI review findings | §9 |
| → human approval | §10 |
| → PR draft | §11 |
| Feels like one button | §13 `RunService::launch` + §16 launcher |
| Primarily Slack | §13.1 `RunTrigger` |
| Webhook later | §13.2 |
| Workspace level | §3 + §15 (every route ws-scoped, `Feature::RunWithOtto`) |

## 19. Explicit non-goals / honest deferrals (v1)

- Slack interactive **buttons** (Block Kit) for approval — v1 uses thread `approve`/
  `reject` replies + the UI/REST buttons (Socket Mode already in place; no interactivity
  endpoint added).
- Deep Slack/Telegram **thread-history** ingestion as source body — v1 uses the trigger's
  seed message.
- Multi-repo runs — one repo (or workspace root) per run.
- Auto-opening PRs is opt-in only; the default is a draft.
- Goal-loop mode is wired + unit-tested; the deterministic E2E exercises single_agent
  (the goal-loop controller spawns live PTYs that the stubbed CLI can't satisfy).

## 20. Adversarial review resolutions (folded in — these AMEND the sections above)

Two independent adversarial reviews (requirements + architecture) returned
APPROVE-WITH-FIXES. Every must-fix is folded in below; the implementation follows §20,
not the pre-review text where they differ.

### 20.1 Repo & base-branch resolution (amends §3, §5, §7) — *requirements #1*
The agent always works in a **registered git repo** (`GitStore`), never a raw,
possibly-non-git workspace root. `resolve_repo(ctx, ws, req, resolved) -> Repo`:
1. explicit `req.repo_id` → `git_store.get_repo`;
2. else source-implied: `github_pr`/`github_issue` → match a registered repo by `remote_url`;
   `finding` → `finding.repo_id`; (others have no inherent repo);
3. else `git_store.list_repos(ws)`: exactly one → use it; more than one → use the most-
   recently-created and record which (or honor a future workspace-default);
   zero → fail `Invalid("no git repo registered in this workspace; register one or pass repo_id")`.
`base_branch = LocalGit::new(repo.path).current_branch()`, `base_commit = rev_parse("HEAD")`.
`repo_id` becomes **NOT NULL** on the run once resolved (review/PR stages require it).

### 20.2 "Failing test" source (amends §5 test) — *requirements #2*
Bound to the only test entity that exists: a **Product QA testcase run** with failed
cases (`ProductRepo::get_testcase_run` + `list_testcases`). CI-job / unit-test-failure
ingestion is a documented follow-up (no such entity exists in the codebase yet). Stated
plainly in the feature doc so no one expects red-CI ingestion in v1.

### 20.3 Channel approver authorization (amends §10) — *requirements #3*
A Slack/Telegram `approve`/`reject` reply is authorized by the **integration's existing
`allowed_users` allowlist** (the same gate the bridge already enforces) and executes as
the daemon **root user** (the established channel-trust model — channel actions already
run as root). `approved_by` records the Slack/Telegram user id. A thread reply resolves
**the single run currently `awaiting_approval` bound to `(workspace, chat, thread)`** (the
most-recent one if more than one ever overlaps). REST/UI approval uses normal `require_ws_role(Edit)`.

### 20.4 GitHub issue adapter (amends §5 github_issue) — *requirements #4*
Build a real `GitHub::get_issue(remote, number)` helper (GitHub-only, `GET
/repos/{o}/{r}/issues/{n}`) — no empty fallback. GitLab/Bitbucket *issues* are out of
scope for v1 (PRs are supported for all three; issues = GitHub only), documented.

### 20.5 Goal-loop verification (amends §7, §19) — *requirements #5*
`single_agent` stays the E2E-verified default. Goal-loop mode gets a focused **Rust test**
asserting the run→`NewGoalLoop` construction (a synthesized manual `AcceptanceCriterion`,
serde-defaulted limits/config) and terminal-status adoption logic, so the required mode is
verified at the unit level even though its live-PTY controller can't run under the E2E stub.

### 20.6 Callable review core (amends §9) — *architecture #1*
`start_local_review` is a handler that diffs `repo.path` (the main checkout) with **no
branch param** — it cannot review the run's worktree. So extract
`pub(crate) async fn run_review_for_branch(ctx, repo_id, worktree_path, base_commit, jira?, user?) -> Id`
that: `LocalGit::new(worktree_path).diff_text_against(base_commit)` → `reviews_store.create_review(repo_id, 0)`
→ spawn `run_review(...)`. The engine polls `reviews_store.get_review(review_id)` for
`Done|Error`, then reads counts via `pub(crate) review_findings_counts(ctx, review_id) ->
(total, open, blocker)` using the **existing** `blocker = severity=="bug" && open` semantics
(NOT "critical+high"). The review stage **requires a resolved `repo_id`** (always true per §20.1).
Empty diff → review completes immediately (existing fast-path) → 0 findings (deterministic for E2E).

### 20.7 Callable PR-draft core + best-effort push (amends §11) — *architecture #2*
Extract `pub(crate) async fn draft_pr_core(ctx, worktree_path, branch, base) -> DraftPrResp`
and lift `resolve_skill_inline`/`compose_draft_prompt`/`parse_pr_draft`/`jira_key_from_branch`/
`ensure_jira_in_subject` to `pub(crate)`. **The draft needs no git token** (local diff +
agent), so it always succeeds. **Pushing the branch is best-effort**: resolve the token via
the repo's bound git account; if absent or `created_by` doesn't own it, skip the push and
record "branch not pushed (no authorized git account)" — the run still `completed` with a
draft. Actually opening a PR (the opt-in "Open PR") requires the token + approval + a
passing/waived proof pack (`gate_pr`) and fails loudly if unauthorized.

### 20.8 Restart & concurrency safety (replaces §12's reaper) — *architecture #3*
- **Per-run in-flight guard:** `RunEngine` holds a `Mutex<HashSet<Id>>`; `advance` claims the
  run id, no-ops if already in-flight, releases on yield (`awaiting_approval`) or terminal.
- **Compare-and-set transitions:** every stage transition is `UPDATE otto_runs SET status=?
  WHERE id=? AND status=?`; 0 rows updated ⇒ someone else moved it ⇒ abort quietly. A late
  reaper or a double-approve cannot double-advance.
- **Boot policy (mirrors goal_loop `fail_running`, NOT blind re-drive):** on startup, runs in
  `executing` or `reviewing` (had live background work now gone) → **`failed`** with
  "interrupted by restart" (branch + commits preserved). Short, idempotent stages
  (`queued`, `resolving_source`, `building_context`, `provisioning`, `proving`,
  `drafting_pr`) → re-drive. `awaiting_approval` → leave (waits for approval). Provisioning
  uses `worktree_add_if_absent` for idempotency.

### 20.9 Second exhaustive WorkKind arm (amends §14) — *architecture #4*
Adding `WorkKind::OttoRun` forces an arm in **both** `WorkStatus::from_source`
(`otto-state/src/workgraph.rs:133`) **and** `otto_workgraph::normalize::risk`
(`otto-workgraph/src/normalize.rs:31`). Map `OttoRun` like `Session`/`GoalLoop` (status →
active/blocked/done; risk passthrough).

### 20.10 assemble_diff metadata/risk fix (new; amends §8) — *architecture #5*
Pre-existing bug shared with `goal_loop.rs`: `proof::assemble_diff` builds its structured
metadata (`files_changed/additions/deletions/risky_files`) from `git show <base>` (the base
commit's own patch) instead of the actual `base..HEAD` range, so `compute_risk` scores the
**wrong commit**. Fix `assemble_diff` to compute structured metadata over the real diff
range (working-tree/`HEAD` vs `base`), matching its already-correct text artifact. Verify
existing goal-loop proof tests still pass (this also corrects their risk score).

### 20.11 Scope trims (de-risk; amends §13, §16, §19)
- **Defer MCP `otto.run`/`otto.run_status` tools** to a follow-up (user asked Slack-primary +
  webhook; MCP is unrequested surface + DANGEROUS-gating + test burden).
- **Defer inline "Run with Otto" launchers** on the Findings board / Product story; the
  central launcher (which accepts a finding/story id or URL) already delivers the one button.
- **Keep** the inbound webhook trigger (user named it) with **minimal** callback delivery
  (optional `callback_url`, reusing the existing SSRF-guarded `WebhookAdapter`).

### 20.12 Migration discipline — *architecture #6*
`0085` is **provisional**. Other worktrees are live; claim the number only at merge, renumber
to one above settled main's max, FF-merge last, and reconcile the sqlx migration table before
deploying (avoids the `(version,checksum)` deploy-brick).
