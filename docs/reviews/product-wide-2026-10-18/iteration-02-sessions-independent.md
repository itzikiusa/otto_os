# Round 2 — independent session lifecycle review (E15)

**Verdict before follow-up implementation: Block on remaining capture overwrite; 9.1/10.** One major capture-identity defect and one minor grid-generation defect confirmed by hand trace. Reviewed original worker patch independently before accepting ownership of follow-up fixes. No Cargo executed. Coordinator reports 267 session tests, 51 PTY tests, clippy, and two red/green mutations; those results precede the additional fixes below and were not independently rerun here.

## R2-S1 — major: late capture can be overwritten by the old spawn-time capture

Locations in reviewed snapshot: `manager.rs:3298-3307,3337`, `3846-3862`, `5852-5856`.

Concrete trace: a Codex process receives first input and exits before its capture loop succeeds. `ensure_live` acquires the session resume lock, late-captures rollout A and persists A, then resumes A. Since the session now has a provider ID, restart does not replace its existing CaptureProbe. An old spawn-time task waiting for that same resume lock subsequently passes the generation check: the original probe still exists and has the same Arc identity. It never re-reads the row's already-assigned ID. The claimed-ID query excludes A from candidate selection. Another unclaimed rollout B in the same cwd/time window becomes the sole candidate, and the uncontended task uses `probe=None`, so it stores B over A. The running CLI remains on A, but the next resume points at B.

This is a remaining hole in the new capture protection, not a claim that UUID generation is wrong. Fix: under both capture and lifecycle locks, re-read the row and retire the task if an ID was already assigned; do not interpret a failed row lookup as permission to overwrite. Retire only that generation's probe. Regression needs an existing assignment plus a plausible different rollout, and must assert the ID survives after the actual capture task terminates.

## R2-S2 — minor: a delayed old-process grid write can overwrite the replacement grid

Location: `manager.rs:4487-4499` in reviewed snapshot. Existing code, not introduced by the worker patch.

The spawned persistence closure keeps the old PtyHandle. `size_persist` serializes execution but does not order tasks by the time resize was requested, nor compare process identity. On a multithread runtime an old closure can first be polled after a restart and after the new process's resize was persisted: it reads the old handle's final grid and overwrites the new metadata. Restart does not acquire size_persist. The next reopen restores the stale width/height. A current-generation check must be protected against replacement through the whole DB write by the lifecycle mutex; an unlocked check still races.

Fix direction: acquire the resume mutex, reject a handle whose spawn_seq differs from the current live handle, then serialize and persist the grid. Preserve same-generation latest-value behavior. Regression should explicitly run a delayed old-generation persistence after replacement/current persistence and assert current metadata survives.

## What the original patch handles correctly

- Fresh Claude restart now allocates a UUID, persists only after successful spawn, and returns the persisted row. The two-launch argv test checks both initial `--session-id` and subsequent `--resume`, stable identity, model, and lean options.
- Capture replacement uses pointer identity rather than timestamps. Generation is checked after the session mutex is acquired, and cleanup removes only its own generation.
- Lock ordering inspected: spawn capture takes session mutex then global capture mutex, matching ensure_live's late-capture path. No inverse global-to-session acquisition was found in the inspected nested-capture path; no confirmed deadlock reported.
- Moving capture registration before live publication closes the usual first-input-before-probe window.

## Regression assessment and residuals

The original superseded-task test verifies early retirement but has no input/rollout and never reaches scan or persistence. The fresh-restart probe test verifies replacement/reset. Neither guards R2-S1. Existing rapid-resize test covers one handle, so it does not guard R2-S2.

Residuals: file-scan duration while holding lifecycle mutex; DB failures after process spawn; failed UUID persistence cleanup; global capture claim atomicity across late-capture callers (the helper releases the global mutex before its caller persists). These are review limits or follow-up candidates, not additional confirmed blockers in this report. No assertion of exhaustive concurrency proof or 9.8 approval.

## Follow-up implementation status

Coordinator authorized E15 to implement R2-S1/R2-S2 after this independent report. Changes and validation to be appended after implementation; earlier worker changes remain intact.

### Implementation addendum

Both confirmed findings now have follow-up changes in manager.rs. Spawn capture re-reads the current persisted ID under the existing session/global lock order and retires its own probe if assignment already occurred. A lookup error stops the attempt rather than treating the row as unassigned. Grid writes use an extracted production `persist_grid` task which acquires the lifecycle mutex before checking the live handle generation and retains it through the metadata write. Same-generation writes still read the latest handle size under size_persist.

Added `capture_waiting_for_lifecycle_preserves_late_assigned_id` (isolated named-profile directory, competing unclaimed rollout, actual capture task retirement) and `delayed_predecessor_grid_cannot_overwrite_replacement` (explicitly schedules old persistence after the replacement write commits). No process-global HOME/CODEX_HOME mutation. Removing the new ID re-read permits OTHER-CONVERSATION to overwrite LATE-ASSIGNED; removing the generation check permits 90x25 to overwrite 150x45. These mutation outcomes are predicted from trace, **not yet executed**.

Formatting and diff checks pass. Cargo slot requested, no additional tests run yet. Post-fix provisional score **9.4/10**, pending execution and a separate review of these follow-ups. The author of the follow-ups cannot claim independent approval of their own implementation.

## Root compiled verification after follow-ups and test extraction

`cargo test -p otto-sessions --lib -- --test-threads=2`: **269 passed, 0 failed,
0 ignored**, 26.18s. Both named late-ID and predecessor-grid regressions passed.
This also compiled the extracted manager/tests/mod.rs plus existing archive/grid
child modules. Log /tmp/otto-product-wide-sessions-lib.log. Predicted mutation
outcomes above have not been executed and remain predictions; no blanket 9.8
claim or exhaustive concurrency proof follows from this package pass.

## Ronaldo (`agy`) Independent Re-Review of Follow-Up Fixes (`R2-S1`, `R2-S2`, `kill_session_locked`, and Cross-Session `late_capture_provider_id`) — Score: **9.8 / 10**

1. **`R2-S1` — Late-Assigned Provider ID Preserved Against Queued Spawn-Time Capture (`crates/otto-sessions/src/manager.rs:3298-3320, 3384-3386` & `manager/tests/mod.rs:681-753`)**:
   - **Lock & Generation Discipline**: `spawn_session_id_capture` acquires `resume_lock` (`3298-3299`), re-verifies `probes.get(&id).is_some_and(|p| Arc::ptr_eq(&p.generation, &generation))` (`3300-3305`), then acquires the global `codex_capture_lock` (`3306`) — matching the `resume_lock -> codex_capture_lock` hierarchy in `ensure_live` -> `late_capture_provider_id` (`3826-3863`).
   - **Row Re-Read Before Scan/Claim**: Under both locks, `repo.get(&id).await` (`3310-3320`) checks `current.provider_session_id.is_some()`. When `ensure_live` has already assigned an ID while the spawn-time task waited on `resume_lock`, the task sets `captured = current.provider_session_id` and breaks (`3311-3313`), causing `probes.remove_if(&id, |_, p| Arc::ptr_eq(&p.generation, &generation))` (`3385`) to retire its own probe without overwriting the row. A DB lookup error (`3316-3319`) aborts the capture loop rather than treating the row as unassigned.
   - **Regression Quality**: `capture_waiting_for_lifecycle_preserves_late_assigned_id` (`681-753`) uses an isolated named-account directory (no process-global `HOME`/`CODEX_HOME` mutation), seeds a competing unclaimed rollout `OTHER-CONVERSATION`, holds `resume_lock` while committing `LATE-ASSIGNED`, releases `resume_lock`, waits for `captures_in_flight` to drain, and asserts both `provider_session_id == Some("LATE-ASSIGNED")` and probe removal.

2. **`R2-S2` — Predecessor Grid Writes Gated by `spawn_seq` Under `resume_lock` (`crates/otto-sessions/src/manager.rs:4488-4508` & `manager/tests/mod.rs:858-881`)**:
   - **Ordering & Deadlock Freedom**: `persist_grid` acquires `lifecycle.lock().await` (`resume_lock(id)`, line `4496`), verifies `live.get(&sid)` still maps a handle with matching `spawn_seq()` (`4497-4502`), and holds `_lifecycle` across `order.lock().await` (`size_persist`, line `4503`) and `repo.merge_meta(&sid, &patch).await` (`4506`). Because no lifecycle operation (`restart_locked`, `kill_session_locked`, `suspend_inner`) ever acquires `size_persist`, the `resume_lock -> size_persist` order is acyclic.
   - **Regression Quality**: `delayed_predecessor_grid_cannot_overwrite_replacement` (`858-881`) deterministically schedules `manager.persist_grid(&id, &old).await` *after* the replacement handle (`150x45`) has been inserted and persisted, proving `90x25` is rejected while same-generation rapid resize ordering (`saved_grid_is_the_latest_after_rapid_resizes`, `1864-1893`) remains intact.

3. **`kill_session_locked` Generation Retirement & Error-Restore (`crates/otto-sessions/src/manager.rs:4564-4590` & `manager/tests/mod.rs:3939-3965`)**:
   - Under `resume_lock`, `self.live.remove(id)` retires the generation before `handle.kill()`, records `self.retiring.insert(id.clone(), handle.on_exit())` on success (so `restart_locked` waits for child exit via `await_retired_exit`), and runs `self.networks.clear(id)` only after `handle.kill()` succeeds so a local backend error restores `self.live` without tearing down active session tunnels. (Note: `HeldPty::kill()` is infallible with async fallback, whereas `LocalPty::kill()` can return `Err`.)
   - **Executed Evidence**: `otto-sessions --lib` **270 passed, 0 failed, 0 ignored** (`27.23s`, `/tmp/otto-product-wide-sessions-lib.log`), `kill_retires_live_generation_before_returning_without_status_poller` **1/1 passed** (`/tmp/otto-root-kill-retirement-test.log`), and `otto-server --lib` **1087 passed, 0 failed, 1 ignored** (`87.10s`, `/tmp/otto-root-server-lib-all.log`, including `agent_session::lifecycle::tests::tracked_startup_cancellation_drains_created_and_resumed_cleanup`).

4. **Cross-Session `late_capture_provider_id` Claim Atomicity & Fail-Closed Lookup (`crates/otto-sessions/src/manager.rs:3856-3863, 4162-4246` & `manager/tests/mod.rs:756-855`)**:
   - **Fix**: Closed the cross-session claim window where `late_capture_provider_id` dropped `self.codex_capture_lock` before `ensure_live` persisted `set_provider_session(id, &psid)`. `late_capture_provider_id` now holds `codex_capture_lock` across the claimed-ID lookup (`self.repo.provider_session_ids()`), the blocking rollout/conversation scan, `self.repo.set_provider_session(&session.id, &psid)`, `persist_transcript_path`, and `self.capture_probes.remove(&session.id)`, preserving the `resume_lock -> codex_capture_lock` order. In addition, `self.repo.provider_session_ids().await` now fails closed (`Err(e) => return None`) instead of `.unwrap_or_default()`.
   - **Regression (`late_capture_serializes_cross_session_claim_and_fails_closed`, `manager/tests/mod.rs:756-855`)**: Uses an isolated named-account directory (zero process-global env mutation), seeds two uncaptured Codex sessions competing for a sole rollout `SOLE-ROLLOUT-1` while holding `codex_capture_lock` until both tasks hold their `resume_lock` and contend at the capture gate, asserts that exactly one session claims and persists `SOLE-ROLLOUT-1` while the other receives `None`, and verifies that closing the DB pool causes `late_capture_provider_id` to fail closed (`None`).
   - **Executed Verification & Boundary**:
     - `PATH=/tmp/otto-nextest-0.9.146:$PATH cargo nextest run -p otto-sessions -p otto-server --no-fail-fast`: **1586 passed, 0 failed, 6 skipped** (`123.668s`, `/tmp/otto-product-wide-final-sessions-server.log`, SHA256 `5a40f8a7d95167a1f968a01f74fb0a77d7174e94bd722bbeb800de2eb0de466b`), with `manager::tests::late_capture_serializes_cross_session_claim_and_fails_closed` passing in **`0.287s`**.
     - Affected-consumer clippy (`/tmp/otto-product-wide-final-consumer-clippy.log`, `1m 41s` across all 42 workspace crates through `otto-server` and `ottod`): **PASS (0 warnings)**.
     - Residual boundary: blocking rollout/conversation directory scans still run inside `spawn_blocking` while the session's `resume_lock` and `codex_capture_lock` are held, bounding cross-session capture latency by filesystem scan duration rather than introducing a correctness hazard.
