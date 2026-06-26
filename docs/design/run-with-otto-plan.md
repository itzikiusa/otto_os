# Run with Otto — Implementation Plan

> **For agentic workers:** implement task-by-task, TDD, commit per task. Steps use
> `- [ ]`. The design (`run-with-otto-design.md`, incl. §20 review resolutions) is the
> spec; where this plan and pre-§20 design text differ, §20 wins.

**Goal:** One trigger (Slack-first; also webhook/REST/UI) turns any of 8 source items
into a traceable, evidence-backed change proposed as a PR draft, gated by human approval.

**Architecture:** A new `OttoRun` entity whose `status` *is* a pure stage machine, driven
by an idempotent, status-gated `run_engine` that chains existing subsystems (context,
worktree, run_agent/goal-loop, proof, local review, PR draft) and projects into Mission
Control. Surfaces funnel into `RunService::launch`.

**Tech stack:** Rust (otto-core/state/server/channels/git/issues/product), Axum, SQLite
(sqlx), Svelte 5 + TS, Playwright.

## Global Constraints (verbatim)

- Workspace-scoped: every route `require_ws_role`; `Feature::RunWithOtto` (snake `run_with_otto`), View=reads / Edit=writes.
- Migrations append-only; `0085` is **provisional** — renumber at merge to one above settled main, FF-last, reconcile sqlx table before deploy.
- Contracts in lockstep: `docs/contracts/{api,ws}.md` + `ui/src/lib/api/types.ts`. `route_inventory` test requires every registered route in `api.md`.
- Secrets via Keychain only; no `.env`/keys committed. Outbound SSRF-guarded. Loopback-only default unchanged.
- No AI attribution in commits. Match surrounding code density/idiom.
- All source bodies + delivered text through `otto_core::redact`.
- Gates that must pass: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cd ui && npm run check && npm run build`, `npm run test:e2e`.
- E2E determinism: daemon runs with `OTTO_E2E=1` + non-existent `CLAUDE_BIN`; `run_agent` returns a canned reply; engine writes+commits `OTTO_RUN_NOTE.md` only under E2E to make the diff real.

---

## File structure

**otto-core** (pure domain, no I/O):
- `crates/otto-core/src/run.rs` (NEW) — `RunStatus` (stage machine + `next_on_success`/`is_terminal`/`can_transition`/`is_resumable_on_boot`), `SourceKind`, `RunMode`, `RunOrigin`, `OttoRun`, `RunEvent`, `LaunchRunReq`, `ResolvedSource`, `ApproveRunReq`, `parse_source_ref`. Unit tests inline.
- `domain.rs` — add `Feature::RunWithOtto` (enum + `parse` + `as_str`).
- `event.rs` — add `Event::OttoRunUpdated { run_id, workspace_id, status }`.
- `lib.rs` — `pub mod run;` re-exports.

**otto-state**:
- `crates/otto-state/migrations/0085_run_with_otto.sql` (NEW) — `otto_runs` + `otto_run_events`.
- `crates/otto-state/src/runs.rs` (NEW) — `RunsRepo` (create/get/list_by_workspace/update_status_cas/set_fields/add_event/list_events/find_awaiting_for_thread/reap helpers).
- `crates/otto-state/src/workgraph.rs` — `WorkKind::OttoRun` + arm in `WorkStatus::from_source`.
- `crates/otto-state/src/lib.rs` — `pub mod runs;` + re-export `RunsRepo`.

**otto-workgraph**:
- `crates/otto-workgraph/src/normalize.rs` — `OttoRun` arm in `risk`.

**otto-server**:
- `crates/otto-server/src/run_sources.rs` (NEW) — `resolve_source`, `resolve_repo`.
- `crates/otto-server/src/run_context.rs` (NEW) — `build_packet`.
- `crates/otto-server/src/run_workspace.rs` (NEW) — `provision_worktree`, `remove_worktree`.
- `crates/otto-server/src/run_engine.rs` (NEW) — `RunEngine`, `advance`, per-stage fns, in-flight guard, projection.
- `crates/otto-server/src/run_scheduler.rs` (NEW) — boot reaper + queued tick supervisor.
- `crates/otto-server/src/run_service.rs` (NEW) — `RunService::launch`, `approve`, `cancel`, `open_pr`.
- `crates/otto-server/src/run_channels.rs` (NEW) — `ChannelRunTrigger` (impl `RunTrigger`).
- `crates/otto-server/src/routes/runs.rs` (NEW) — REST routes + webhook run route.
- `crates/otto-server/src/modules.rs` — extract `pub(crate) draft_pr_core`, `run_review_for_branch`, `review_findings_counts` pub(crate); lift draft helpers to pub(crate).
- `crates/otto-server/src/proof.rs` — fix `assemble_diff` metadata range (§20.10).
- `crates/otto-server/src/policy.rs` — RunWithOtto route classes; webhook-run exempt.
- `crates/otto-server/src/state.rs` — `ServerCtx.runs: RunsRepo` + `runs_engine: Arc<RunEngine>`.
- `crates/otto-server/src/routes/mod.rs` + `modules.rs` module_routers — register `routes::runs::routes()`.

**otto-channels**:
- `crates/otto-channels/src/run_trigger.rs` (NEW) — `RunTrigger` trait + `RunAck`.
- `crates/otto-channels/src/bridge.rs` — call `run_trigger` for `/run …` + `approve`/`reject` replies; `Bridge.run_trigger: Option<Arc<dyn RunTrigger>>` + `with_run_trigger`.
- `crates/otto-channels/src/lib.rs` — `pub mod run_trigger;`.

**otto-git**:
- `crates/otto-git/src/providers/github.rs` — `pub async fn get_issue(&self, remote, number) -> Result<IssueLite>`.

**ottod**:
- `crates/ottod/src/main.rs` — construct `RunsRepo`, `RunEngine`, `RunService`; inject `ChannelRunTrigger` into `ChannelManager`; spawn `run_scheduler`.

**UI**:
- `ui/src/modules/run-with-otto/RunWithOttoPage.svelte` (+ `RunDetail.svelte`, `RunLauncher.svelte`).
- `ui/src/lib/api/runWithOtto.ts`, `ui/src/lib/stores/runWithOtto.svelte.ts`, `ui/src/lib/api/types.ts` (mirror), `ui/src/lib/sidebar.ts`, `ui/src/shell/App.svelte`.

**E2E:** `ui/e2e/run-with-otto.spec.ts` + `ui/e2e/seed.ts` helper (`seedRepo`).

**Docs:** `docs/contracts/api.md`, `docs/contracts/ws.md`, `docs/features/run-with-otto.md`, `docs/features/README.md` row.

---

## Task 1 — otto-core: RunStatus state machine + domain types

**Files:** Create `crates/otto-core/src/run.rs`; Modify `lib.rs`, `domain.rs`, `event.rs`.

**Produces (signatures later tasks rely on):**
```rust
pub enum RunStatus { Queued, ResolvingSource, BuildingContext, Provisioning, Executing,
    Proving, Reviewing, AwaitingApproval, DraftingPr, Completed, Failed, Rejected, Cancelled }
impl RunStatus {
  pub fn as_str(&self)->&'static str; pub fn parse(s:&str)->Option<Self>;
  pub fn next_on_success(&self)->Option<RunStatus>; // Queued→ResolvingSource→…→DraftingPr→Completed; AwaitingApproval has NO auto-next
  pub fn is_terminal(&self)->bool;                  // Completed|Failed|Rejected|Cancelled
  pub fn is_resumable_on_boot(&self)->bool;         // true for Queued/ResolvingSource/BuildingContext/Provisioning/Proving/DraftingPr; false for Executing/Reviewing/AwaitingApproval/terminal
  pub fn is_active(&self)->bool;                    // !terminal
}
pub enum SourceKind { Jira, Confluence, GithubPr, GithubIssue, Channel, ProductStory, Finding, Test, ScheduledReport } // as_str/parse
pub enum RunMode { SingleAgent, GoalLoop }   // as_str/parse, Default=SingleAgent
pub enum RunOrigin { Slack, Telegram, Webhook, Ui, Mcp, Api } // as_str/parse
pub struct OttoRun { /* every column in §3, typed; status: RunStatus etc. */ }
pub struct RunEvent { pub id:Id, pub run_id:Id, pub workspace_id:Id, pub kind:String, pub status:Option<String>, pub message:String, pub detail:Option<serde_json::Value>, pub created_at:DateTime<Utc> }
pub struct LaunchRunReq { pub source_kind:Option<SourceKind>, pub source_ref:Option<String>, pub url:Option<String>, pub seed_text:Option<String>, pub mode:Option<RunMode>, pub provider:Option<String>, pub repo_id:Option<String>, pub auto_open_pr:Option<bool>, pub title:Option<String> }
pub struct ResolvedSource { pub title:String, pub body_md:String, pub goal:String, pub source_url:Option<String>, pub repo_hint:Option<String>, pub metadata:serde_json::Value }
pub struct ApproveRunReq { pub decision:String /*approve|reject*/, pub note:Option<String> }
pub fn parse_source_ref(s:&str)->Option<(SourceKind,String,Option<String>)>;
```

- [ ] Write failing unit tests in `run.rs`: `next_on_success` full happy path ordering; `AwaitingApproval.next_on_success()==None`; `is_terminal`/`is_resumable_on_boot` per variant; `parse_source_ref("PROJ-123")==Jira`, `pull/42`→GithubPr, `issues/42`→GithubIssue, `/pages/123`→Confluence, `finding:<id>`/`story:<id>`/`test:<id>`/`report:<id>`; round-trip `as_str`/`parse` for all enums.
- [ ] `cargo test -p otto-core run::` → FAIL.
- [ ] Implement `run.rs`; `pub mod run;` in lib.rs; add `Feature::RunWithOtto` (3 sites) + `Event::OttoRunUpdated`.
- [ ] `cargo test -p otto-core` → PASS. `cargo build -p otto-core`.
- [ ] Commit `feat(core): Run with Otto domain + RunStatus state machine`.

## Task 2 — otto-state: migration 0085 + RunsRepo

**Consumes:** Task 1 types. **Produces:**
```rust
pub struct RunsRepo { /* pool */ }
impl RunsRepo {
  pub fn new(pool:SqlitePool)->Self;
  pub async fn create(&self, n:NewRun)->Result<OttoRun>;
  pub async fn get(&self, id:&Id)->Result<OttoRun>;
  pub async fn list_by_workspace(&self, ws:&Id, limit:i64)->Result<Vec<OttoRun>>;
  pub async fn set_status_cas(&self, id:&Id, from:RunStatus, to:RunStatus)->Result<bool>; // compare-and-set, returns rows_affected==1
  pub async fn set_status(&self, id:&Id, to:RunStatus)->Result<()>;
  pub async fn set_fields(&self, id:&Id, patch:&RunPatch)->Result<()>; // COALESCE-guarded optional cols
  pub async fn set_error(&self, id:&Id, err:&str)->Result<()>;
  pub async fn add_event(&self, e:NewRunEvent)->Result<RunEvent>;
  pub async fn list_events(&self, run_id:&Id)->Result<Vec<RunEvent>>;
  pub async fn find_awaiting_for_thread(&self, ws:&Id, chat:&str, thread:Option<&str>)->Result<Option<OttoRun>>;
  pub async fn list_resumable(&self)->Result<Vec<OttoRun>>;      // is_active && is_resumable_on_boot
  pub async fn list_interrupted(&self)->Result<Vec<OttoRun>>;    // status in (executing,reviewing)
}
```
- [ ] Write `0085_run_with_otto.sql` (header doc + 2 tables + indexes per §3). `otto_runs.repo_id` NOT NULL? — keep NULLABLE at create, set on resolve; review/PR stages guard on presence.
- [ ] Write failing tests `runs.rs#[cfg(test)]` (in-memory pool, `.foreign_keys(true)`): create→get round-trip; `set_status_cas` returns false on stale `from`; `add_event`/`list_events` order; `find_awaiting_for_thread` matches only `awaiting_approval`.
- [ ] `cargo test -p otto-state runs::` → FAIL (no table/repo).
- [ ] Implement repo + `pub mod runs;`. Run migrations in test setup via existing harness.
- [ ] `cargo test -p otto-state runs::` → PASS.
- [ ] Add `WorkKind::OttoRun` + arms in `WorkStatus::from_source` (state) and `normalize::risk` (otto-workgraph). `cargo build -p otto-state -p otto-workgraph`.
- [ ] Commit `feat(state): otto_runs schema + RunsRepo + WorkKind::OttoRun`.

## Task 3 — otto-git: GitHub get_issue helper

**Produces:** `GitHub::get_issue(&self, remote:&RemoteRef, number:u64)->Result<IssueLite{number,title,body,url,state}>`.
- [ ] Add `IssueLite` struct + `get_issue` (GET `/repos/{o}/{r}/issues/{n}`, same auth header as `get_pr`). Unit: parse a canned issue JSON fixture.
- [ ] `cargo test -p otto-git` → PASS. Commit `feat(git): GitHub get_issue`.

## Task 4 — otto-server: source resolution + repo resolution + context packet

**Consumes:** Tasks 1-3, `JiraClient`, `ConfluenceClient`+`adf`, providers, `ProductRepo`, `ReviewFindingsRepo`, `ScheduledTasksRepo`, `GitStore`. **Produces:**
```rust
// run_sources.rs
pub(crate) async fn resolve_source(ctx:&ServerCtx, run:&OttoRun)->Result<ResolvedSource>;
pub(crate) async fn resolve_repo(ctx:&ServerCtx, ws:&Id, req:&LaunchRunReq, resolved:Option<&ResolvedSource>, source_kind:SourceKind, source_ref:&str)->Result<Repo>; // §20.1 algorithm
// run_context.rs
pub(crate) struct ContextPacket { pub prompt:String, pub summary:String }
pub(crate) fn build_packet(run:&OttoRun, resolved:&ResolvedSource, repo:&Repo)->ContextPacket; // redacted, capped
```
- [ ] Tests: `resolve_repo` — explicit id; single-repo workspace; zero-repo error; github remote match. `build_packet` includes goal+body, redacts secrets, caps length, mentions the branch. (Use fakes for clients where needed; resolve_repo testable via in-memory GitStore.)
- [ ] Implement adapters dispatch (`match source_kind`) + repo resolution + packet. Channel source uses `run.context_summary`/seed (no fetch). Scheduled-report reads `report_path` file.
- [ ] `cargo test -p otto-server run_sources:: run_context::` → PASS. Commit `feat(server): Run source adapters + repo/context resolution`.

## Task 5 — otto-server: worktree provisioning + draft_pr_core + review core + assemble_diff fix

**Produces:**
```rust
// run_workspace.rs
pub(crate) async fn provision_worktree(ctx:&ServerCtx, run:&OttoRun, repo:&Repo)->Result<(String/*branch*/,String/*path*/,String/*base_commit*/)>; // otto-run/<id>, worktree_add_if_absent
pub(crate) async fn remove_worktree(ctx:&ServerCtx, run:&OttoRun);
// modules.rs (extracted, pub(crate)):
pub(crate) async fn draft_pr_core(ctx:&ServerCtx, worktree_path:&str, branch:&str, base:&str)->Result<DraftPrResp>;
pub(crate) async fn run_review_for_branch(ctx:&ServerCtx, repo_id:&Id, worktree_path:&str, base_commit:&str)->Result<Id/*review_id*/>;
pub(crate) async fn review_findings_counts(ctx:&ServerCtx, review_id:&Id)->Result<(u32,u32,u32)>; // total, open, blocker(severity=="bug"&&open)
```
- [ ] Refactor `draft_pr` handler to call `draft_pr_core(ctx,&repo.path,&source,&body.base)`; lift `resolve_skill_inline`/`compose_draft_prompt`/`parse_pr_draft`/`jira_key_from_branch`/`ensure_jira_in_subject` to `pub(crate)`. Existing `draft_pr` tests still pass.
- [ ] Add `run_review_for_branch` (diff worktree vs base_commit → `create_review(repo_id,0)` → spawn `run_review`); make `review_findings_counts` `pub(crate)`.
- [ ] Fix `proof::assemble_diff` structured metadata to use `base..HEAD` range (§20.10). Run `cargo test -p otto-server proof:: ` + goal-loop proof tests → PASS (risk now from run's diff).
- [ ] Test: `provision_worktree` creates `otto-run/<id>` + non-empty `base_commit`; second call reuses (idempotent). `cargo test -p otto-server run_workspace::` → PASS.
- [ ] Commit `feat(server): run worktree + extract draft_pr_core/review-for-branch + assemble_diff range fix`.

## Task 6 — otto-server: run_engine (the stage machine driver)

**Consumes:** Tasks 1-5 + `proof::{gate,assemble_diff,run_command_artifact,upsert_content_artifact,recompute_and_emit}`, `Orchestrator::run_agent`, goal-loop `start_loop`+`goal_loops_repo`, `WorkGraphService::record`. **Produces:**
```rust
pub struct RunEngine { /* ctx-lite deps + Mutex<HashSet<Id>> inflight + Arc<WorkGraphService> + events */ }
impl RunEngine {
  pub fn new(...)->Arc<Self>;
  pub async fn advance(self:&Arc<Self>, run_id:Id); // claims inflight; runs ONE stage; CAS-transition; emits; re-arms until AwaitingApproval/terminal
  pub async fn resume_after_approval(self:&Arc<Self>, run_id:Id);
}
```
- [ ] Per-stage private fns: `stage_resolve_source`, `stage_build_context`, `stage_provision`, `stage_execute` (single_agent: run_agent in worktree + E2E commit OTTO_RUN_NOTE.md; goal_loop: start_loop + poll terminal + adopt branch/proof), `stage_prove`, `stage_review` (skip if no repo_id), `stage_await_approval` (post summary to origin, STOP), `stage_draft_pr` (draft_pr_core + best-effort push), then Completed.
- [ ] Each transition: `set_status_cas(from,to)`; on false → return (lost the race). On stage error → `set_error` + `set_status(Failed)` + event + emit. After each stage: `add_event` + `Event::OttoRunUpdated` + `WorkGraphService::record` (projection) + origin thread post.
- [ ] Tests (with `OTTO_E2E=1`, fake orchestrator stub, real RunsRepo + temp git repo): full single_agent run advances Queued→…→AwaitingApproval and stops; double-`advance` is a no-op (inflight guard); `set_status_cas` blocks a stale transition. (These exercise the engine without HTTP.)
- [ ] `cargo test -p otto-server run_engine::` → PASS. Commit `feat(server): run_engine stage driver + Mission Control projection`.

## Task 7 — otto-server: RunService + scheduler + routes + policy + state wiring

**Produces:**
```rust
// run_service.rs
impl RunService { pub async fn launch(ctx,&LaunchRunReq, origin:RunOrigin, origin_chat,thread,user)->Result<OttoRun>;
  pub async fn approve(ctx, run_id,&ApproveRunReq, approver:&str)->Result<OttoRun>;
  pub async fn cancel(ctx, run_id)->Result<OttoRun>; pub async fn open_pr(ctx, run_id, user)->Result<PrSummary>; }
// routes/runs.rs
pub fn routes()->Router<ServerCtx>; // POST /workspaces/{wid}/runs, GET …/runs, GET /runs/{id}, GET /runs/{id}/events,
  // POST /runs/{id}/approve, POST /runs/{id}/cancel, POST /runs/{id}/open-pr, GET /run-with-otto/sources?q=
pub fn webhook_routes()->Router<ServerCtx>; // POST /webhooks/{wid}/run (key-guarded, public)
// run_scheduler.rs
pub fn spawn(ctx:ServerCtx); // boot: fail interrupted (executing/reviewing), re-drive resumable; tick: pick Queued
```
- [ ] `launch`: parse/resolve source_kind+ref (parse_source_ref or explicit), create `queued` run, record work item, `engine.advance` (spawned). `approve`: CAS AwaitingApproval→DraftingPr (or →Rejected), then `resume_after_approval`. `cancel`: →Cancelled + remove_worktree. `open_pr`: gate (approved + proof passed/waived) → `provider.create_pr`.
- [ ] Routes with `require_ws_role`; webhook route reuses `channel_webhook` key check; `policy.rs` classes (`/workspaces/{wid}/runs*`, `/runs/{id}*`, `/run-with-otto/*` → Require(RunWithOtto,View|Edit)); `/webhooks/{wid}/run` Exempt. `ServerCtx.runs`+`runs_engine`; register routers; `ottod` construct + `run_scheduler::spawn`.
- [ ] Tests: `policy.rs::tests` for the new routes; route_inventory updated. Booting smoke (ottod test or server test) — scheduler spawns clean.
- [ ] `cargo test --workspace` (server) → PASS. Commit `feat(server): RunService funnel + routes + policy + scheduler`.

## Task 8 — otto-channels: RunTrigger + Slack/Telegram wiring

**Produces:** `run_trigger.rs`: `trait RunTrigger { async fn try_command(ws,channel,chat,thread,user,text)->Option<RunAck>; async fn try_approval(ws,channel,chat,thread,user,text)->Option<RunAck>; }` + `struct RunAck{reply:String}`. Bridge: `with_run_trigger`, call `try_command` on `/run …`/"run with otto …", `try_approval` on `approve`/`reject` when a run awaits in the thread.
- [ ] `ChannelRunTrigger` in otto-server impls it (parse ref → `RunService::launch` origin=slack/telegram; `approve`/`reject` → `find_awaiting_for_thread` + `RunService::approve`, gated by integration `allowed_users`). Inject from `ottod/main.rs` `.with_run_trigger(...)`.
- [ ] Tests: bridge unit — `/run PROJ-1` invokes trigger and replies; non-command passes through; `approve` resolves an awaiting run. `cargo test -p otto-channels` → PASS.
- [ ] Commit `feat(channels): Run with Otto Slack/Telegram trigger + approval replies`.

## Task 9 — UI module + types + api client + store + sidebar

- [ ] `types.ts`: mirror `OttoRun`/`RunEvent`/`RunStatus`/`SourceKind`/`LaunchRunReq`/`ApproveRunReq`; add `'run_with_otto'` to the `Feature` union.
- [ ] `lib/api/runWithOtto.ts`: list/get/events/launch/approve/cancel/openPr/detectSource.
- [ ] `lib/stores/runWithOtto.svelte.ts`: runs map + `otto_run_updated` WS handler.
- [ ] `modules/run-with-otto/RunWithOttoPage.svelte` (RunLauncher: one input + detect + mode/provider/auto-open + Run; runs list), `RunDetail.svelte` (timeline, proof badges, findings link, Approve/Reject, PR draft + Open PR).
- [ ] `sidebar.ts` entry; `App.svelte` import+route+command; bottom-nav if applicable.
- [ ] `cd ui && npm run check` → 0 errors; `npm run build` → ok. Commit `feat(ui): Run with Otto module`.

## Task 10 — Contracts + docs

- [ ] `api.md`: "Run with Otto" section listing every route (required by route_inventory).
- [ ] `ws.md`: `otto_run_updated`.
- [ ] `docs/features/run-with-otto.md` (code-grounded guide; honest deferrals = §19/§20.11) + README row.
- [ ] Commit `docs: Run with Otto contracts + feature guide`.

## Task 11 — E2E + full gates

- [ ] `ui/e2e/seed.ts`: add `seedRepo(ctx,base,wsId)` (init a temp git repo, register via `/workspaces/{ws}/repos`).
- [ ] `ui/e2e/run-with-otto.spec.ts`: launch (REST) a `channel`/`finding` single_agent run on a seeded repo → poll to `awaiting_approval` (assert proof artifact + 0 findings) → approve → poll `completed` with a PR draft; ws-scoping 403 on cross-ws; 404 unknown id; UI: launcher detects a Jira key; detail shows the timeline + Approve. Import types from `types.ts` only (no `.svelte.ts` typeof import).
- [ ] Run all gates: `cargo test --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cd ui && npm run check && npm run build`; `npm run test:e2e` (built worktree ottod, slot-isolated). Fix until green.
- [ ] Add a Rust goal_loop-mode construction test (§20.5). Commit `test: Run with Otto E2E + goal-loop construction`.

## Task 12 — Merge + deploy

- [ ] Renumber `0085`→ above settled main max; `git merge main`; resolve append-points (Feature/Event/WorkKind/policy/routes/types.ts/api.md); re-run ALL gates on merged tree.
- [ ] FF-merge `feat/run-with-otto` into local main (only if main HEAD == merge-base and clean). Do NOT push.
- [ ] Reconcile sqlx migration table; `./deploy.sh` (quit app first; `mkdir -p apps/desktop/src-tauri/binaries`; `~/.hermes/node/bin` on PATH). Verify `/health` ok, `/workspaces/x/runs` 401-not-404, new ottod pid.

## Self-review (spec coverage)

Every §18 requirement maps to a task: 8 sources (T4 + T3), Work Item (T1/T2), Context Packet (T4),
worktree (T5), goal-loop|single (T6), Proof (T6+T5), review (T6+T5), approval (T7+T8), PR draft (T5/T7).
One button: RunService (T7) + Slack (T8) + launcher (T9). Slack-primary (T8). Webhook (T7). Workspace
level (T2/T7 policy). §20 fixes mapped: 20.1→T4, 20.2→T4, 20.3→T8, 20.4→T3, 20.5→T11, 20.6→T5, 20.7→T5,
20.8→T6/T7, 20.9→T2, 20.10→T5, 20.11 trims (no MCP task), 20.12→T12. No placeholders; signatures consistent
across tasks (RunsRepo/`set_status_cas`, `draft_pr_core`, `run_review_for_branch`, `review_findings_counts`).
