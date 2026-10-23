# Backend correctness — quality iteration 6, final review (2026-10-23)

Lens: logic bugs in the Rust daemon crates (`crates/**`): wrong conditions,
off-by-one errors, broken state transitions, unhandled edge cases, wrong error
mapping, and panics that input can reach. The review was read-only: no cargo,
npm or tests were run. Every finding below was confirmed by reading the code
at the cited lines. Scope: branch `fix/quality-iter6-20261023` against
`origin/main` (271 crate files changed), excluding `target/`, `node_modules/`
and `docs/reviews/`.

## Score

Start at 10.0. Open findings: 0 blocker, 0 major, 0 minor, 1 nit (-0.03).

| id | severity | deduction |
|----|----------|-----------|
| backend-correctness-6f-01 | nit | -0.03 |

**10.0 - 1 x 0.03 = 9.97 / 10**

## Previous findings (iteration 6, wave 0)

All five are fixed at every listed location. None was invalid.

| id | status | evidence in current code |
|----|--------|--------------------------|
| 6-01 (Discovery Chat cut the reply at the first ```` ```json ```` fence and lost the actions behind an example block) | **fixed** | `otto-product/src/chat.rs:664-691` `split_actions` finds the block with `extract_json_where(raw, has_actions)` (non-empty `actions` array) and `actions_span` (`:696-734`) splices out only the fence or balanced object that parsed to it. Example fences, `{orderId}` placeholders and trailing prose survive. Tests at `:923-945` cover an example fence before the actions fence and a bare object after a placeholder. |
| 6-02 (`storage_to_markdown` recursed once per nested panel macro and could overflow the stack) | **fixed** | `otto-issues/src/confluence.rs:819` `MAX_PANEL_DEPTH = 16`. `panel_body_markdown` (`:823-828`) emits plain text past the bound instead of recursing. The document is lowercased once (`:809`) and the slices are reused (`extract_body_tag` now returns a range into the caller's lowercase copy, `:1018-1025`), so the quadratic re-lowercasing is gone too. Test at `:1887`. |
| 6-03 (Mongo keyset paging ended the walk at the first `_id` type change) | **fixed** | `otto-dbviewer/src/drivers/mongodb/keyset.rs:89-101` `id_after` builds `{$or: [{_id: {$gt: last}}, {_id: {$type: [every later bracket]}}]}` from `ID_TYPE_ORDER` (`:49-62`, MongoDB's `{_id: 1}` order). An unrankable cursor type keeps plain `$gt`. |
| 6-04 (four agent-reply parsers took the first parseable JSON span) | **fixed** | Every production caller now uses a shape predicate: `workflow_engine/nodes/review_run.rs:133-136` (`score` integer or `goals` array; a reply with no overall score is reported in the detail, `:143-151`); `otto-product/src/refine.rs:576` (`reply` string); `otto-canvas/src/assist.rs:892` (non-empty `elements` or `nodes`); `chat.rs:670` (6-01). The only remaining `extract_json` caller is `otto-swarm/src/recruiter.rs:130` (`parse_recruited`), which deserializes into a typed struct, so a decoy fails closed rather than shadowing silently. |
| 6-05 (`RoomAction::Audio` indexed `members[&self.host]`) | **fixed** | The rooms code moved to `otto-rooms`. `crates/otto-rooms/src/actions.rs:144-146` uses `self.members.get(&self.host).is_none_or(…)`. The remaining `members[..]` indexes (`registry.rs:490`, `:505`, `presentation.rs:17`) follow a `connected(..)?` / `member(..)?` / `admitted(..)?` check in the same `&mut self` call, so the key is present. |

## Areas covered

- **Re-verified** the five wave-0 findings above at every location.
- **Code changed in this iteration** (`git diff origin/main --stat -- crates/`), read as new code:
  - `otto-agent-run/src/fanout.rs` (`run`, `run_with_quorum`, `fan_in`, `summarize`) and `fanout/retry.rs` (`RetryQueue`, `attempts`, `wait_for_retry`, `run_slots`, `summarize_with_retries`). These cover slot state edges, the user and automatic retry budgets, closing a key on exit, and the panic-to-`Failed` mapping. Callers checked: product analysis (`run/analysis.rs`, the summarizer row's `on_state` edges), product plan (`run/plan.rs`, quorum 2, fallback compared with the pre-split behaviour), vault docs (`vault_docs_agent/run.rs`, cancel during the summarizer: `cancel_run` finishes the run first and `finish_run` is a no-op on a terminal run), and code review (`modules/review/summarize.rs`).
  - `otto-agent-run/src/session_turn.rs`: `confirm_needle`, `user_text_has`, `transcript_path` (account home taken from `provider_home.parent()`), `gate_validated_result`.
  - `otto-transcript/src/roots.rs`: precedence, `encode_cwd`, `discover` / `for_session` (an invalid account id resolves to nothing), `claude_session_file`, `dedup`. Consumers: the usage tailer's `driver.rs` (legacy-offset seeding only for `RootKind::Default` Claude roots, per-root Codex listing), `routes/transcript.rs` (`transcript_path` validation across every root, `confine_artifact` / `read_artifact_capped` off the runtime), `otto-sessions` `manager.rs` / `manager/transcript_roots.rs` (`locate_activity_artifact`, `persist_transcript_path`).
  - `otto-sessions/src/submit.rs`: the `RunGuard` checks before the paste, the re-paste, Enter and the re-Enter, and the `Option<Option<Instant>>` contract. Swarm callers: `engine::submit_turn_prompt` / `run_guard` and `agent_run.rs` (a fresh session for each attempt, so a stop between the check and the paste only reaches a doomed session).
  - `workflow_engine/nodes/review_run.rs` in full: `review_score`, `blended_score`, `review_verdict`, `goals_from_reply`, `aggregate` (multi-target strict union), `plan_review`, `launch` (checkpoint adoption), `score_target`. Also `workflow_engine/run/schedule.rs` `run_attempts` (retry, cancel, skip and budget arms; attempt limit against `otto-workflows` `retry_backoff`).
  - `otto-server/src/budget/mod.rs`: `budget_rows`, `verdict_from` (enforcement off, exactly-at-cap, per-scope relevance).
  - `otto-dbviewer`: `export.rs` `ExportWriter` (`pace`, `close`, `drain_to_file`), the driver default `export_to_writer` / `export_to_path` (the `tokio::join!` error precedence), the `/db/export` stream (the error oneshot sent after the writer drops), and `service.rs` write-guard messages for agents.
  - `otto-mcp/src/outward/exec.rs`: `db_query_call` (read-only route vs `/db/query` with `mask`, `timeout_ms`, clamped `max_rows`; fields exist on `QueryRequest`), `call_timeout`, `timeout_error`.
  - `otto-state`: `swarm.rs` prompt-editor stamping (`stamp_prompt_editor`, `stamp_goal_editor`, `prompt_edit`, `create_goal_by`, `replace_standing_goals_by`), migration 0196 (nullable, rollback-safe), and `mcp_profiles::switch_default`. `state_archive` `restore_mcp_profile` / `restore_default_profile` (built-in merge only while the local copy is pristine and unlocked; default applied once, only over the initial `full`).
  - Routes: `admin_sessions.rs` keyset cursor (`created_at|id` hex, `(created_at, id) <` predicate, `archived` filter), the shared `webhook_key_guard` in `channel_webhook.rs` / `swarm_webhook.rs`, `mission.rs` / `telegram.rs` `KeyedLocks` swaps.
  - `otto-channels/src/mirror.rs` `ReplyDelivery` (a permanently refused upload settles once, and a rate limit is still retried).
  - `otto-insights/src/lib.rs`: `write_atomic` / `unique_temp`, `ScheduleAttempt` status, `PeriodAttempts`, `catch_up_allowed` (finding 6f-01).
  - `otto-product/src/service.rs`: detached publishing (`run_detached` + `PUBLISH_LOCKS`). The spawned task drops the request-credential task-local, and none of the publish paths consult it. Also `run/reap.rs`.
  - `otto-vault`: `engine.rs` rename in its own task, and `parse.rs` `md_link` / `rewrite_line` / `md_path_span` (parse and splice agree on angle brackets, titles and anchors; the `unwrap` follows a successful `md_link`).
  - `otto-issues/src/confluence.rs`: the depth-bounded converter, `convert_ac_macro` (`extract_param` slicing when `>` is missing) and `convert_ac_image`.
  - `otto-git/src/local/errors.rs` (`unborn_or_err` is only reached from `diff … HEAD` calls; `local_refusal`, `push_err`, `upstream_err`). Also `otto-k8s` `metrics_denial` (401 is now `Unauthorized`, not RBAC) and `otto-apiclient/src/engine/vars.rs` substitution (char-boundary walk, three bounded passes).
  - `otto-rooms` (extracted crate): `actions.rs`, `presentation.rs`, `terminal.rs` (`ResyncQueue`, `FlowGate`, `Frame`, `decode_input`).
- **Sweep:** every `.unwrap()`, `.expect(`, `[0]` and `len() - 1` on an added non-test line in `crates/` (`git diff origin/main -U0`). Each production hit is guarded by a prior check (`execute.rs:412` after the `Err` arm returned, `history.rs:55` after normalizing to an object, `vars.rs:49` inside `i < len` on a char boundary, `multipart.rs:50` on static MIME strings, `session_turn.rs:1177` under `requires_validated_result`).

## Areas NOT covered (honest gaps)

- `ottod/src/mcp_tools/dispatch.rs` (1,774 lines) and `catalog.rs`: the generated session tool twins. Only the read-only/write split of `query_db` was traced; the per-tool argument mapping of the other twins was not.
- `otto-apiclient/src/engine/{execute,send,steps,binding,secrets}.rs` beyond the variable substitution and the two `expect`s above (largely moved from `otto-server`; not re-read line by line).
- `otto-design/src/format/scene3d.rs` (moved from `otto-product`) and `otto-design-assist/src/job.rs`.
- `otto-agent-run/src/session_turn.rs` `run_session_turn_inner` / `oracle_watch` state machine, and `session_turn/lifecycle.rs` / `resume.rs`, beyond the helpers listed.
- `otto-mcp/src/governance.rs` and `service_stages.rs` (the new governance stage runner).
- `vault_docs_agent/review_loop.rs` and `refine.rs`; `skill_review.rs` beyond its fan-out wiring; `mockup_assist.rs`.
- Carried over from wave 0: `otto-server/src/policy.rs` past the exempt block, `otto-sessions/src/http.rs`, `manager.rs` `ensure_live` / respawn / `claim_input_authority` / `may_resize`, and `rooms/recap/*`.

## Findings

### backend-correctness-6f-01 — nit — the scheduled-Insights status reports an in-memory attempt count that resets on restart, while the durable cap stops the scheduler, so `attempts` / `gave_up` go stale

- **Where:**
  - `crates/otto-insights/src/lib.rs:1641-1645`: `catch_up_allowed(&dir, &key)` (the durable gate) and `attempts.admit(&key)` (in memory), then `let attempt_no = attempts.count(&key);`.
  - `:1700-1713`: the status written after a failed run uses `attempts: attempt_no, gave_up: attempt_no >= MAX_PERIOD_ATTEMPTS`.
  - `:389-399` `catch_up_allowed` reads the manifest's `attempt` (written at `:1139-1142` via `next_attempt`) and the `stopped` flag.
  - `:1504-1531` `PeriodAttempts` is "in memory; a restart starts over".
- **Evidence:**
  - Two scheduled attempts at a period fail in process A. The status is `attempts: 2, gave_up: false`, and the pending manifest has `attempt: 2`.
  - The daemon restarts. Process B's map is empty, so `admit` makes the in-memory count 1, and the run writes manifest `attempt: 3` (`next_attempt` = 2 + 1). It fails, and the status is rewritten as `attempts: 1, gave_up: false`.
  - On the next hourly tick, `catch_up_allowed` is false (`3 < 3` fails) and the `continue` at `:1641` writes nothing. The scheduler has given up for the period, but `GET /insights/config` keeps returning `schedule_status.<cadence> = {attempts: 1, gave_up: false}`.
  - The same happens when a person pressed Stop (`stopped: true`): the scheduler stops retrying, and the status still says it has not given up.
  - The UI (`ui/src/modules/insights/scheduleStatus.ts:26`) words its empty state from `attempts` ("the last run" instead of "the last 3 runs"). `gave_up` is wrong in the API but is not rendered today, hence a nit.
- **Fix:**
  - Take the attempt number from the durable manifest rather than the in-memory map. For example, have `run_insights` return the `attempt` it wrote, or re-read `completion_manifest` after the run, and set `gave_up` from `attempt >= MAX_PERIOD_ATTEMPTS || stopped`.
  - When `catch_up_allowed` refuses a period that still has a failure status, rewrite that status once with `gave_up: true`.
  - Add a test that seeds a manifest with `attempt: 2`, runs one failing tick on a fresh `PeriodAttempts`, and asserts `attempts == 3 && gave_up`.
