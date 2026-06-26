# Implementation plan — swarm enhancements

Ordered, verifiable. **Wave 0 locks all shared contracts** (migrations, domain types,
repo method signatures, trait signatures, route stubs, TS types) so the workspace
compiles; later waves fill in behaviour and can be partly parallel. Build/clippy after
each wave.

## Wave 0 — contracts & schema (do first, build green)

### Task 0.1 — Migrations (otto-state/migrations)
- [ ] `0077_swarm_goals.sql` — `swarm_goals` table (cols per design §3.1) + indexes on
      (swarm_id), (project_id), (task_id).
- [ ] `0078_swarm_project_cols.sql` — add to `swarm_projects`: `skills_json TEXT NOT NULL
      DEFAULT '[]'`, `integration_branch TEXT`, `origin_channel TEXT`, `origin_chat TEXT`,
      `origin_thread TEXT`.
- [ ] `0079_swarm_channel_triggers.sql` — `swarm_channel_triggers` table + index
      (workspace_id, channel).
- **Verify:** `cargo test -p otto-state` (migrations apply in the in-memory test).

### Task 0.2 — otto-state domain + repo (crates/otto-state/src/swarm.rs)
- [ ] Structs: `SwarmGoal`, `NewGoal`, `GoalPatch`; `SwarmChannelTrigger`, `NewTrigger`,
      `TriggerPatch`. Extend `NewProject`/`ProjectPatch`/`SwarmProject` with the new cols.
- [ ] Repo methods on `SwarmRepo`: goals `create_goal/list_goals_for_task/list_goals_for_project/
      list_standing_goals/get_goal/update_goal/delete_goal`; triggers
      `create_trigger/list_triggers/get_trigger/update_trigger/delete_trigger/find_trigger(ws,channel,chat)`;
      project col read/write through existing `update_project`. Mirror existing
      `NewX`/`XPatch` + sqlx patterns exactly.
- **Verify:** `cargo build -p otto-state` + a repo round-trip unit test (insert/list/patch a goal).

### Task 0.3 — otto-core / otto-swarm API + trait types
- [ ] `otto-core` (or otto-swarm `types.rs`): API DTOs for goals, triggers, verification
      state; `SwarmGoalUpdated` WS event variant in the event enum.
- [ ] `SwarmTrigger` trait + `LaunchAck` in **otto-channels** (no otto-server dep).
- **Verify:** `cargo build -p otto-core -p otto-swarm -p otto-channels`.

### Task 0.4 — contracts docs
- [ ] `docs/contracts/api.md` (#87+ goals/verify/triggers), `docs/contracts/ws.md`
      (`swarm_goal_updated`), `ui/src/lib/api/types.ts` mirror.
- **Verify:** referenced in code review; types.ts compiles in Wave 4.

## Wave 1 — worktree isolation + merge (req 1)

### Task 1.1 — otto-git changed-files helper (crates/otto-git/src/local.rs)
- [ ] `changed_files(&self, base:&str) -> Result<Vec<String>>` (= `git diff --name-only base...HEAD`).
- **Verify:** `cargo test -p otto-git` new unit test on a temp repo.

### Task 1.2 — swarm_workspace.rs
- [ ] `cwd_mode`: default `worktree` when `has_repo` (drop the `repo` default); keep
      per-agent `repo`/`scratch` opt-outs.
- [ ] `ensure_cwd_info() -> CwdInfo{path,mode,created,branch,base}`; pin base to
      `project.integration_branch` (create-or-read it on first call); keep `ensure_cwd` wrapper.
- [ ] `provision_agent(ctx, swarm, project, agent, identity_md, cwd)` — union skills
      (agent ∪ project.skills_json ∪ swarm.config.skills, must_use OR). New `resolve_skills`.
- **Verify:** `cargo build`; unit test `resolve_skills` union + dedupe.

### Task 1.3 — swarm_run.rs wiring
- [ ] Use `ensure_cwd_info`; on `created`, `system_post(kind="worktree", meta)`.
- [ ] After turn persists, compute `changed_files(integration_branch)`, update the
      per-swarm shared-files map, post `kind:"shared"` on a new overlap (deduped).
- [ ] Pass swarm+project to `provision_agent`.
- **Verify:** `cargo build`; the git integration test (Task 5.2) covers overlap+merge.

### Task 1.4 — swarm_merge.rs (new) + integration worktree
- [ ] `ensure_integration_worktree(ctx, swarm, project) -> path`; `merge_task_branch(...)`
      under a per-integration-branch async mutex (registry); `merge_preview`+`merge_branch(MergeCommit, auto_stash:false)`; conflict → return files.
- **Verify:** git integration test (Task 5.2).

## Wave 2 — goals + verification controller (req 3)

### Task 2.1 — swarm_verify.rs (new): pure engine
- [ ] `Goal`, `Verdict`, `Decision`, `FixOutcome`, `VerifyOps` trait, `classify()`,
      `run_verification()` exactly per design §3.3.
- **Verify:** Task 5.1 unit tests (scripted ops).

### Task 2.2 — swarm_verify.rs: real `SwarmVerifyOps` + controller + registry + recovery
- [ ] `SwarmVerifyOps` impl (verify/fix via `run_swarm_agent` w/ provider; merge via
      swarm_merge; record/post/escalate; cancelled/over_budget).
- [ ] `start_verification(ctx, task)` spawns the controller (test-and-set guard, RAII drop);
      sets task `verifying`→`done`/`blocked`. Leader resolution helper.
- [ ] `recover_verifications(ctx, swarm_id)` startup sweep.
- [ ] verify registry (CancelState keyed by task_id) + `stop_verification`.
- **Verify:** `cargo build`; route smoke + manual live smoke.

### Task 2.3 — swarm_agent_run.rs extensions
- [ ] `provider: &str` param + optional `task_id`; backfill tokens/cost (factor
      `session_usage` shared helper from swarm_run.rs). Callers updated.
- **Verify:** `cargo build` (plan/recruit still pass "claude").

### Task 2.4 — swarm_runtime.rs hooks
- [ ] route_result `"done"` arm: if `reviews.empty() && task has blocking goals` →
      `verifying` + `start_verification`.
- [ ] tick: skip agents `under_verification`.
- [ ] start_coordinator: call `recover_verifications`.
- [ ] abort(): set verify cancel flags + stop verify/fix runs.
- [ ] Seed standing goals on swarm create (and lazily on first verify if absent).
- **Verify:** `cargo build` + `cargo test -p otto-server`.

### Task 2.5 — goals + verify routes (otto-server routes + policy.rs)
- [ ] CRUD goals, standing-goals get/put, `POST tasks/{tid}/verify`,
      `GET tasks/{tid}/verification`, `POST tasks/{tid}/verify/stop`. Emit `swarm_goal_updated`.
- [ ] **Register every new route path in otto-server `policy.rs`** (memory gotcha: else 403).
- **Verify:** route smoke (401 not 404/403 unauth).

## Wave 3 — channel/webhook launch (req 4)

### Task 3.1 — swarm_webhook.rs: goals + origin
- [ ] Extend `SwarmTriggerReq` with `goals`; create goal rows; record origin (callback URL).
- **Verify:** `cargo build`.

### Task 3.2 — SwarmTrigger impl + Bridge injection
- [ ] `otto-channels` `Bridge`: `swarm_trigger: Option<Arc<dyn SwarmTrigger>>` + `with_swarm_trigger`;
      call `try_launch` in `handle()` between ws-resolve and session-create; on `Some`, reply + return.
- [ ] otto-server `swarm_trigger.rs`: impl `SwarmTrigger` (find_trigger → launch like webhook + record origin).
- [ ] `ottod/main.rs`: build the impl with `ctx`, pass into `ChannelManager::new`→`Bridge`.
- **Verify:** `cargo build --workspace`.

### Task 3.3 — triggers CRUD routes + channel-origin notify
- [ ] CRUD `swarm_channel_triggers` (+ policy.rs). 
- [ ] Factor `improve_notify` send into shared `channel_send(secrets, integ, chat, thread, text)`;
      `notify_channel_origin(ctx, project, text)`; fire on launch ack, goal-unmet escalation, project-complete summary.
- **Verify:** route smoke + `cargo build`.

## Wave 4 — UI (ui/src/modules/swarm)

### Task 4.1 — types + store
- [ ] types.ts: SwarmGoal, SwarmChannelTrigger, verification state, new feed kinds.
- [ ] swarm.svelte.ts: goal/trigger CRUD, verify trigger/stop, `swarm_goal_updated` event.
- **Verify:** `npm run check`.

### Task 4.2 — components
- [ ] Goals panel (per task/project) + add/edit modal (metric/target/block/verify_cmd/max_retries/blocking) + status chips + measured/verdict.
- [ ] Swarm settings modal: standing-goals editor + team skills picker + triggers panel.
- [ ] Project modal: project skills picker.
- [ ] BoardFeed: chips for worktree/shared/merge/verify/escalation; RunInspector shows verify/fix.
- **Verify:** `npm run check && npm run build`.

## Wave 5 — tests

### Task 5.1 — Rust: pure engine (swarm_verify tests)
- [ ] Mock `VerifyOps`; assert sequential ordering; pass→next; warn-after-retries (Gap A);
      fail→fix→re-verify w/ scrutiny++; max_retries→unmet+escalate (continue next goal);
      advisory→warned no-fix; merge iff no blocking unmet; failed-fix distinct; cancel; budget.
- **Verify:** `cargo test -p otto-server swarm_verify`.

### Task 5.2 — Rust: real git integration
- [ ] temp repo; two agent worktrees via ensure_cwd_info; overlapping edits → shared-files
      detector; merge one clean + one conflict → fix path; skill/goal-set resolution union.
- **Verify:** `cargo test`.

### Task 5.3 — UI E2E (ui/e2e/swarm-goals.spec.ts)
- [ ] Seed swarm+project (temp repo)+task+goals+trigger via API; assert Goals panel CRUD,
      verdict chips, standing-goals editor, skills pickers, triggers panel, feed kind chips.
- **Verify:** `npm run test:e2e -- swarm-goals`.

## Wave 6 — gate + delivery
- [ ] `cargo build/test --workspace`, `clippy -D`, `npm run check/build`, swarm E2E.
- [ ] Live manual smoke (webhook → verify→fix→merge on temp repo).
- [ ] commit on worktree branch → merge to feature branch → re-gate → merge to LOCAL main
      (no push) → rebuild+sign+install ottod+app → force-quit+relaunch+activate.
