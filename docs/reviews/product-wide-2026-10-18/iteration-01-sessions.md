# Iteration 01 — session execution and conversation lifecycle

Status: fixes implemented; focused tests, throttled subsystem suites and clippy passed. Two regressions also proved red under restored pre-fix behavior. No numerical product score is inferred from this partial subsystem review.

## Scope and method

Applied correctness-review, test-review, and architecture-review. Read existing production paths, not only the PR diff. The ownership slice is P01–P04; this document is not a whole-product verdict. Snapshot began on `fix/db-parity-session-consistency` at `c009da20cc113c8fb79f3bf0600442d0c6afa358`, with a clean working tree. Other agents own other subsystem edits.

| Lens | Source inspected and behavior traced | Evidence level |
|---|---|---|
| P01 session lifecycle/recovery | `otto-sessions/src/manager.rs`: create, ensure_live, suspend, kill, remove, restart_locked, capture task, late capture; `lifecycle.rs`: provider transcript existence and filesystem failures | Hand traces of fresh restart, resume, removed/archived row, missing provider ID, delayed capture; new executable regressions below |
| P02 provider routing/config/model handoff | `manager.rs`: model_args, lean_turn_args, restart argv; `providers.rs`: build_spec and shell fallback; `accounts.rs`: account routing/home; `otto-orchestrator/src/claude_pty.rs`: per-turn UUID, model argv, reply retry/exit | Compared fresh and resumed launch paths; fake-provider regression uses actual child argv and persisted identity |
| P03 PTY input/output/grid/reconnect | `otto-pty/src/lib.rs`: write queues, resize rollback, snapshot-and-subscribe, feed/reset, kill/drop; `held.rs`: reader/reconnect/ack settlement; `ring.rs`: bounded eviction/search; manager human submit/input/resize and generation checks | Static traces: snapshot and broadcast share parser lock; failed resize restores mirror; process replacement prevents delayed Enter to successor; reconnect does not guess a live holder is dead. No new confirmed defect in these sampled paths |
| P04 transcript persistence/reconnect | `otto-transcript/src/tailer.rs`: offset, partial lines, shrink, bounded reads; `otto-server/src/transcript_tail.rs`: registry touch, generation slot replacement, fold budget; `routes/transcript.rs`: persisted path, provider roots, account homes; orchestrator ReplyTail | Static traces of partial UTF-8/JSON lines, initial-missing file, shrink, account-root resolution and nested provider change. Existing tests inspected; no new confirmed defect in these sampled paths |

## Confirmed findings and corrections

1. **Major / confirmed by trace — fresh restart uses invalid and unrecorded provider ID.** Before this change `manager.rs::restart_locked` used `new_id()` (ULID) when the persisted provider ID was absent. `create` explicitly uses UUID because Claude `--session-id` requires it. An override-created agent row (or legacy row) has no provider ID, and a normal restart then passes the invalid ULID. Even a permissive provider would lose conversation identity because the ID was never persisted. The restart now allocates a UUID and persists it after successful spawn before publishing the live handle; a failed persistence kills the unpublished child. A second restart uses that same ID with resume argv.

2. **Major / confirmed by trace — restarted self-ID providers stop capturing their new conversation.** `create` starts `spawn_session_id_capture`, while `restart_locked` previously did not. If initial capture expired without an ID and the user explicitly restarts, the fresh Codex/agy process cannot acquire an ID through the regular capture task; its retained probe still contains the previous process input. Restart now registers an empty capture before publishing the live handle. Each capture has an identity token, checks it before polling and under the session lifecycle lock before persisting, and conditionally removes only its own probe. This prevents an older capture from recording its predecessor conversation after restart or removing its successor probe. Create registers its probe before live publication as well, eliminating the first-input gap.

3. **Minor / confirmed by trace — lean-turn options disappear on restart.** `create` appended `lean_turn_args` but restart only appended directories/model/read-only options. A restarted lean draft session silently loaded tools/MCP excluded by its persisted configuration. Restart now applies the same lean options. This preserves an existing opt-in behavior; no new restriction was introduced.

## Regression evidence and mutation intent

- `fresh_restart_retains_uuid_and_launch_options`: temporary DB and provider account home, fake CLI records real argv; checks valid UUID, persisted same ID on second launch, `--session-id` then `--resume`, model and lean flags. Reverting UUID/persistence or lean options must fail these assertions.
- `fresh_restart_replaces_expired_capture_probe`: starts from an expired probe and restarts a fake Codex process in a temporary account home; asserts fresh generation, empty input and no first-input timestamp. Removing restart capture registration fails it.
- `superseded_capture_task_retires_without_touching_successor`: starts two capture generations for one row; waits for old task retirement, verifies successor probe remains and no guessed provider ID was persisted. Removing generation filtering leaves both capture tasks alive and fails the bounded wait.

## Architecture assessment

Create and restart assemble provider argv independently; the missing lean flag is concrete evidence of drift. This pass aligns their behavior locally. A future shared pure launch-argument builder would reduce drift across model/directory/lean/read-only policy, but broad extraction of the 10k-line manager is not needed to make these fixes correct. Lifecycle and provider capture now share the existing per-session lock rather than adding a second competing lifecycle mechanism. Lock ordering is session resume lock then global capture arbitration lock, matching late capture from ensure_live.

## Verification and residual limits

Focused tests and subsystem suites passed (details below). No real model provider, user session, deployed daemon, or account credentials were used. Tests use temporary database/profile directories and local fake processes. Browser reconnect behavior and real provider upgrades are outside this iteration's executed evidence. Not every function in these large subsystems has been read; subsequent iterations must expand coverage and independently review the fixes. In-place same-size replacement of append-only transcript files remains outside the tailer's documented shrink-detection contract; it is not asserted as a product bug without a supported producer path.

## Source anchors (working tree after fixes)

- Provider UUID/persistence: `crates/otto-sessions/src/manager.rs:5675`, `:5836`.
- Lean-turn parity: `crates/otto-sessions/src/manager.rs:5692`; original create counterpart `:2975`.
- Capture generation and lifecycle lock: `crates/otto-sessions/src/manager.rs:626`, `:3267`, `:3299`; restart registration `:5855`.
- Regression tests: `crates/otto-sessions/src/manager.rs:7375`, `:7439`, `:7501`.

Focused `cargo test -p otto-sessions --lib fresh_restart_`: **2 passed**. First combined PTY/session lib run stopped before sessions: PTY **50 passed, 1 failed** on native `openpty` allocation (`Device not configured`, OS error 6), not on a behavioral assertion. Retry with `--test-threads=2` passed: **otto-pty 51/51; otto-sessions 267/267**. `cargo clippy -p otto-sessions --all-targets -- -D warnings` passed after moving the now-test-only `new_id` import into the test module.

Mutation execution: temporarily reinstated ULID generation and omitted restart capture registration. `fresh_restart_` returned exit 101, **0 passed / 2 failed**, respectively at `provider requires a UUID` and the probe-generation assertion. The exact original file was restored in a `finally` block. Post-restoration focused rerun: **2 passed / 0 failed**.

## Round-2 Follow-Ups & Final Executed Verification (`E01` — **9.8 / 10**)

All Round-2 independent review findings (`docs/reviews/product-wide-2026-10-18/iteration-02-sessions-independent.md`) are closed and verified:
- **`R2-S1` (late-assigned provider ID vs queued spawn-time capture)**: `spawn_session_id_capture` (`crates/otto-sessions/src/manager.rs:3298-3320, 3384-3386`) re-reads `repo.get(&id).await` under `resume_lock -> codex_capture_lock` and retires its own probe if an ID is already assigned; lookup errors abort rather than overwriting (`capture_waiting_for_lifecycle_preserves_late_assigned_id`, `manager/tests/mod.rs:681-753`).
- **`R2-S2` (delayed predecessor grid write vs replacement handle)**: `persist_grid` (`crates/otto-sessions/src/manager.rs:4488-4508`) gates metadata persistence on `live.get(&sid)` matching `spawn_seq()` under `resume_lock -> size_persist` (`delayed_predecessor_grid_cannot_overwrite_replacement`, `manager/tests/mod.rs:858-881`).
- **`kill_session_locked` live generation retirement & error-restore**: `kill_session_locked` (`crates/otto-sessions/src/manager.rs:4564-4590`) removes `self.live` before `handle.kill()`, registers `self.retiring` and clears `self.networks` only after `handle.kill()` succeeds, and restores `self.live` on local backend error (`kill_retires_live_generation_before_returning_without_status_poller`, `manager/tests/mod.rs:3939-3965`).
- **Cross-session `late_capture_provider_id` claim atomicity & fail-closed lookup**: `late_capture_provider_id` (`crates/otto-sessions/src/manager.rs:3856-3863, 4162-4246`) holds `codex_capture_lock` across `repo.provider_session_ids()`, rollout scan, `repo.set_provider_session(&session.id, &psid)`, `persist_transcript_path`, and `capture_probes.remove(&session.id)`, and fails closed on `provider_session_ids()` errors (`late_capture_serializes_cross_session_claim_and_fails_closed`, `manager/tests/mod.rs:756-855`, `0.287s`).
- **Final Executed Gate**: `PATH=/tmp/otto-nextest-0.9.146:$PATH cargo nextest run -p otto-sessions -p otto-server --no-fail-fast` passed **1586 passed, 0 failed, 6 skipped (`123.668s`)** (`/tmp/otto-product-wide-final-sessions-server.log`), and affected-consumer clippy across all 42 workspace crates passed with **0 warnings** (`/tmp/otto-product-wide-final-consumer-clippy.log`).
