# E15 — daemon lifecycle, event delivery and runtime

Verdict: **Approve with fixes pending execution and in-flight cancellation review**. One confirmed major lifecycle defect; ownership fix implemented. Provisional lens score **9.0/10**, not 9.8: runtime regressions have not executed and scheduler tick cooperation remains open.

Reviewed checkout: `c009da20cc113c8fb79f3bf0600442d0c6afa358`, with concurrent working-tree edits. This is a whole-existing-system lens, not a changed-line-only PR verdict. No production data, deployed daemon, commits or remote systems were touched.

## Confirmed finding E15-01 (major)

**Supervisors survive into dependency teardown, and plain cancellation handles never stop on drop.**

Original locations: `crates/ottod/src/main.rs:449-459`, `crates/otto-server/src/boot/tasks.rs:27-38,87-94`, `crates/otto-core/src/cancel_signal.rs:28-32`, `crates/otto-automation/src/scheduled_tasks_scheduler.rs:61-105`.

Trace: SIGTERM starts the bounded HTTP drain. The daemon then calls session/browser/telemetry/usage teardown while retaining Background. A scheduled-task supervisor whose 60-second timer expires during that window sees its CancelSignal still false, lists enabled tasks, claims a due task and spawns `run_task`. That opens a persistent run and can execute a shell task independently of session-manager shutdown. Background drops only after teardown; its boxed CancelSignal does not cancel on drop anyway. The supervisor remains alive until Tokio runtime termination. This is a real shutdown race, not an assumption that every active task must be killed immediately.

Impact: fresh automation can start while its dependencies are being dismantled, then be interrupted by runtime exit; user-visible interrupted runs and incomplete shell work are possible. A shell task can perform side effects before termination. The window is bounded by drain plus teardown (normally up to roughly 22 seconds), but admission should have stopped already.

### Implemented ownership fix

- `boot/tasks.rs`: cancellation-on-drop wrappers for scheduled tasks, personal agents, assistant, swarm, workflow schedule triggers and Kubernetes monitoring; explicit stop flag for workflow event triggers; abort wrappers for nudge/projector task handles.
- `Background::stop_on` owns those guards until the process shutdown signal, so shutdown stops future supervisor ticks while HTTP drains.
- `ottod/main.rs`: abort and await the ownership watcher before resource teardown if serving exits independently. A serve error also follows teardown rather than early `?` return.
- Existing drop-aware channel/story/insights/update/improvement handles retain their own shutdown semantics.

**Boundary:** cancelling a supervisor is not proof that an already-entered DB-awaiting tick or detached run finished. Automation owner Pele was contacted through the coordinator for post-await/pre-spawn cancellation checks. Detached retention/monitor/history/plugin tasks are not all lifecycle-owned by this patch. This report deliberately does not claim global quiescence.

## Regression evidence

Added three tests in `boot::tasks::background_shutdown_tests`:

1. Dropping Background cancels the real CancelSignal, sets the legacy flag, and aborts the owned pending task (oneshot sender destruction proves it).
2. Aborting/awaiting the real `Background::stop_on` watcher cancels ownership before teardown even when no signal resolves.
3. Resolving a local stop signal makes the real watcher cancel its owned guards before returning.

Falsification: removing the cancellation guard breaks the cancellation assertions; removing abort breaks the bounded oneshot assertion; leaking the watcher-owned Background instead of dropping it breaks the cancellation assertions. Tests avoid the process-global shutdown singleton, so they do not poison sibling tests.

Executed: `rustfmt --edition 2021` on owned files; `git diff --check` on owned files passed. **Cargo tests not executed**: coordinator owns the shared build slot. Requested `cargo test -p otto-server --lib background_shutdown_tests`; binary compilation and affected consumers still need the coordinated gate.

## Coverage and rejected suspects

Read daemon main startup/signal/listener/drain/teardown flows, profile-lock ordering as used by main, housekeeping, boot open/build/recovery/task assembly, process shutdown signal, shared event fanout and its tests, WebSocket event loop/auth-cache/lags, monitor recv-error branches, scheduler cancellation implementations, and existing shutdown-budget/startup-recovery tests.

- Fanout is bounded to 1,024 frames; sockets share an Arc and lazy serialized bytes. No per-socket deep clone regression found. Cost scales with retained frame bytes and active sockets; no measured performance claim is made.
- Lagged event receivers explicitly send resync to sockets; monitor loops continue after lag. No false claim that lag terminates these reviewed loops.
- Watch receiver `borrow()` does not mark the signal seen; the cloned listener receiver can still observe a signal delivered during startup. Rejected a suspected lost-shutdown signal on this basis.
- Shutdown deadline/reserve arithmetic has explicit boundary tests, including signal during boot. Tests inspected, not rerun.
- Startup recovery precedes HTTP admission, and run recovery failure is tested. This pass does not re-audit each engine's recovery semantics.

Residual review: in-flight supervisor admission cooperation, detached background-task ownership, startup failure after module construction, process-level signal/slow-request integration, memory/latency measurement under representative traffic. `usage_tailer.rs` belongs to E18 and `manager.rs` to session owner; neither was edited. The current evidence does not support 9.8 approval.

## Root integration follow-up

The workflow schedule tick now receives its supervisor cancellation signal and
checks it after loading candidates, before durable admission, and after the
admission await before spawning execution. If cancellation arrives during the
transaction, its committed pending run remains recoverable by
`resume_queued_runs`; it is not deleted and its occurrence is not silently lost.
Added `cancelled_schedule_tick_preserves_unclaimed_occurrence` using the real
state fixture. Formatting and whitespace checks passed; Cargo execution is
pending. This narrows the workflow tick race; it does not establish global
shutdown quiescence or replace the automation-owner follow-up.

Additional pending regression: `schedule_tick_cancelled_during_database_wait_does_not_admit` polls the real tick while holding the fixture's sole SQLite connection, cancels, then releases the connection and checks both cursor and run list. Event-trigger listener now checks shutdown after event receive and before/after durable admission too. Neither patch claims cancellation and DB commit are one atomic operation: a commit racing shutdown remains queued for recovery.

## Root shutdown integration follow-up

Pele supplied post-await admission checks in scheduled tasks, personal agents and
swarm; their race tests remain pending. Root verified boot/tasks.rs wraps all
three supervisor signals in CancelOnDrop, and main owns Background via stop_on,
then aborts/awaits the watcher before teardown. These stop new admission; they do
not drain already-detached jobs.

Source tracing found another early return: serve_network_listener's settings read
can fail after supervisor startup, and main used `await?` before common cleanup.
Root now matches that result and routes the error through the same watcher
abort/await and teardown path as a primary listener failure. Successful startup
retains boot.finish and listener behavior. Formatting/whitespace passed; ottod
compiled/runtime failure-path verification remains pending. No claim that this
source-only check validates shutdown timing or all detached-job cleanup.

## Root executed server regressions

All three background shutdown ownership tests and both workflow schedule cancellation tests passed in the rebuilt server focused run; the complete server library subsequently passed **1087 tests, 0 failures, 1 ignored**. Logs `/tmp/otto-root-server-focused-tests.log` and `/tmp/otto-root-server-lib-all.log`; hashes and exact commands in `verification-evidence.json`. Pele reports current scheduler libraries green. Earlier statements that these server tests are unexecuted are superseded. Process-level ottod shutdown/consumer and final workspace gates remain outstanding; already-detached job quiescence is still not claimed.

## Ronaldo (`agy`) Independent Re-Review of E15 Shutdown Cancel-Before-Teardown — Score: **9.7 / 10**

1. **Supervisor Ownership & Cancel-On-Drop (`crates/otto-server/src/boot/tasks.rs:30-77, 108-136, 580-769, 845-899`)**:
   - Verified `Background` (`tasks.rs:30-51`) wraps non-RAII supervisor handles in `CancelOnDrop` (`tasks.rs:55-61`: swarm `696`, workflow schedule triggers `720`, k8s monitor `729`, scheduled tasks `750`, personal agents `759`, assistant `768`), `StopFlagOnDrop` (`tasks.rs:63-69`: workflow event triggers `710`, storing `true` with `Ordering::SeqCst`), and `AbortOnDrop` (`tasks.rs:71-77`: conversation-view nudge `638`, workgraph projector `739`), alongside existing drop-aware handles (`channel_manager` `109`, `story_watcher` `111`, `self_improvement` `602, 613`, `insights` `121`, `cli_update` `123`, and `usage_tailer` in `ottod/src/main.rs:412`).
   - Verified `Background::stop_on` (`tasks.rs:42-50`) drops `self` immediately when `otto_server::shutdown::cancelled()` resolves, and all 3 unit tests in `boot::tasks::background_shutdown_tests` (`dropping_background_cancels_supervisors_and_aborts_owned_tasks`, `stop_watcher_drops_handles_before_teardown_even_without_signal`, `stop_watcher_cancels_on_signal_before_it_returns`) passed in `/tmp/otto-root-server-focused-tests.log:5-7` and `/tmp/otto-root-server-lib-all.log`.

2. **Daemon Shutdown Ordering & Early-Exit Cleanup (`crates/ottod/src/main.rs:411-478`)**:
   - `background_stop` is armed at `main.rs:416` (`background.stop_on(otto_server::shutdown::cancelled())`) before `serve_network_listener` and `serve_loopback`.
   - On normal signal shutdown (`main.rs:328`), `otto_server::shutdown::begin()` wakes both long-poll handlers and `background_stop` *before* the 3 s HTTP drain (`HTTP_DRAIN_CAP`), stopping new supervisor ticks during the HTTP drain window.
   - If `serve_network_listener` fails (e.g. a settings DB read failure after supervisors and sessions are already live, `main.rs:439-450`) or `serve_loopback` returns without a signal, `main.rs:454-455` explicitly executes `background_stop.abort(); let _ = background_stop.await;` so the task's captured `Background` drops and cancels all supervisors *before* secondary-listener drain (`458-465`) and `teardown(...)` (`468-475`).

3. **Workflow Schedule & Event Trigger In-Flight Cancellation (`crates/otto-server/src/workflow_trigger_scheduler.rs:59-165` & `crates/otto-workflows/src/triggers.rs:190-346`)**:
   - `workflow_trigger_scheduler::tick` checks `cancel.is_cancelled()` at entry (`60`), per trigger (`68`), immediately before `admit_schedule_run` (`130`), and immediately after `admit_schedule_run` (`145`) before `workflow_engine::spawn_run`. If cancellation arrives while `claim_scheduled_run` is committing, the durable queued run is preserved for next-boot `resume_queued_runs` without launching side effects during teardown.
   - `spawn_event_trigger_listener` (`triggers.rs:196, 210, 244, 299, 325`) checks `cancel2` after `rx.recv()`, per matching trigger, before `admit_run_if_idle`, and after `admit_run_if_idle` before `ctx.spawn_run`.
   - Verified both regressions (`cancelled_schedule_tick_preserves_unclaimed_occurrence` and `schedule_tick_cancelled_during_database_wait_does_not_admit` in `crates/otto-server/tests/unit/workflow_trigger_scheduler.rs:4-60`) passed in `/tmp/otto-root-server-focused-tests.log:75-76` and `/tmp/otto-root-server-lib-all.log`.

4. **Executed Process-Level & Workspace Verification + Honest Boundaries (Score: 9.7 / 10)**:
   - **Workspace Nextest & Clippy**: `scripts/check.sh --all --check --no-ui` (`/tmp/otto-product-wide-workspace-gate-r2.log`) passed **5973 tests (`0 failed, 124 skipped`, `246.796s`)** including `ottod` binary tests, and final consumer clippy (`/tmp/otto-product-wide-final-consumer-clippy.log`) passed across all 42 crates through `otto-server` and `ottod` with **0 warnings**.
   - **Isolated `ottod` Fresh & Repeat Boot + SIGTERM Smoke (`docs/reviews/product-wide-2026-10-18/daemon-shutdown-smoke.json`)**:
     - Binary `/Users/itziklavon/otto_os/target/debug/ottod` (SHA256 `f0e66f77555d13ebbf28d852f1bfc4185180d4deab5c8eebe88c39740d7a60bb`) booted against isolated data dir `/var/folders/6p/t4qb4qmd2jj3gvd85w0shhmc0000gn/T/otto-root-shutdown-smoke-x0ruzw40`.
     - Attempt 1 (fresh boot): `/health` `true`, `SIGTERM` exit `0` in **`0.043s`**.
     - Attempt 2 (repeat boot): `/health` `true`, `SIGTERM` exit `0` in **`0.040s`**.
   - **Explicit Boundary (Why Score Remains 9.7 / 10 Rather Than Overclaiming Global Quiescence)**:
     - The isolated `ottod` shutdown smoke runs an empty E2E-mode daemon and verifies fresh/repeat boot health + graceful `SIGTERM` exit (`0.043s`/`0.040s`), **not** active-job drain under load.
      - Cancelling supervisor admission stops new ticks/runs from starting during HTTP drain and resource teardown, while already-spawned detached run tasks and low-risk periodic housekeeping loops in `boot/tasks.rs` (`spawn_brokers_reaper`, `spawn_db_explorer_reaper`, session sweeps `205-320`, retention `97-98`, `start_model_catalog` `781`, `spawn_mcp_health_sweep` `817`) remain bounded by engine/session teardown and `runtime.shutdown_timeout(RUNTIME_SHUTDOWN_CAP)`.



## Run-with-Otto supervisor follow-up

### Updated daemon: active held-session restart evidence

Root rebuilt `ottod` successfully (the existing linker warning remains), then
ran `python3 docs/reviews/product-wide-2026-10-18/daemon-active-restart-smoke.py`
against an isolated data directory, workspace, port and file-backed secret store.
The fixture shell survived graceful SIGTERM, the restarted daemon reattached it,
and a second input produced the same shell PID. Both daemon exits were zero
(0.254s and 0.042s). The fixture was explicitly killed before final shutdown;
the PID was absent afterward. Binary hash and evidence are in
`daemon-active-restart-smoke.json`.

This verifies the intended preservation of holder-backed user work during
restart. It does not assert that every detached job drains, and the remaining
bounded-runtime shutdown boundary is unchanged. The score remains 9.7 pending
the final independent assessment; no global session-kill behavior was added.

Root confirmed that `run_scheduler::spawn` remained detached and could redrive resumable runs during teardown. This is admission work, unlike the remaining periodic cache/reaper housekeeping. The supervisor now returns a CancelSignal owned by Background's CancelOnDrop, selects cancellation against its read-only DB scan, and checks cancellation after the scan and again before a spawned continuation starts advancing. Already-running stages retain existing cleanup semantics; no global active-job drain is asserted. Added a regression holding the sole DB connection while cancellation must finish the supervisor before release. Existing boot/recovery test now retains its supervisor guard. Focused compile/tests and independent review pending.

Root run-scheduler follow-up verification: `cargo test -p otto-server --lib
run_scheduler -- --test-threads=2` passed 4/4 (1.50s), including the new blocked
DB scan cancellation test and existing boot recovery tests. Server/ottod
all-target clippy with `-D warnings` passed (15.79s). Logs:
`/tmp/otto-product-wide-run-scheduler-tests.log` and
`/tmp/otto-product-wide-run-scheduler-clippy.log`. This closes admission by the
previous detached run scheduler, not a claim that all detached work is drained.

## Ronaldo (`agy`) Independent Review of `run_scheduler` Shutdown Admission Fix — Score: **9.7 / 10**

1. **Supervisor Cancellation & Ownership (`crates/otto-server/src/run_scheduler.rs:21-38, 76-94` & `crates/otto-server/src/boot/tasks.rs:133, 773-777`)**:
   - `run_scheduler::spawn(ctx)` (`run_scheduler.rs:21-25`) now creates and returns a `CancelSignal`, which `boot::tasks::start_run_scheduler` (`tasks.rs:773-777`) wraps in `CancelOnDrop` and registers in `Background` (`tasks.rs:133`).
   - In `run_until_stopped` (`run_scheduler.rs:27-38`), `tokio::select! { biased; _ = stop.cancelled() => return, _ = redrive_resumable(&ctx, &stop) => {} }` immediately aborts an in-flight read-only `ctx.runs.list_resumable()` scan when shutdown begins, and `stop.sleep(TICK).await` exits on cancellation during the 30 s sleep.
   - In `redrive_resumable` (`run_scheduler.rs:76-94`), cancellation is checked twice after `list_resumable()` returns: once per row in the loop (`if stop.is_cancelled() { return; }` at `79-81`) before `tokio::spawn`, and once inside the spawned continuation (`if !stop.is_cancelled() { run_engine::advance(&c, id).await; }` at `88-90`) so a queued task that has not yet been polled when `Background` drops cannot start a new stage during teardown.
2. **Regression & Boot Test Coverage (`crates/otto-server/src/run_scheduler.rs:96-122` & `crates/otto-server/tests/unit/run_scheduler_boot.rs:68`)**:
   - `stopping_during_run_scan_finishes_before_database_is_released` (`run_scheduler.rs:101-121`) holds the sole SQLite connection from `mem_pool()` so `list_resumable()` blocks on pool acquisition, verifies `run_until_stopped` is pending for 20 ms, cancels `stop`, and proves `run_until_stopped` finishes within 1 s *before* `drop(connection)`.
   - `run_scheduler_start_waits_for_startup_recovery` (`tests/unit/run_scheduler_boot.rs:68`) now binds `let _scheduler = start_run_scheduler(&ctx);` so the `CancelOnDrop` guard stays alive for the 50 ms assertion window instead of dropping immediately at statement end.
   - Executed in `/tmp/otto-product-wide-run-scheduler-tests.log` (`4 passed, 0 failed` in `1.50s`) and clippy-clean in `/tmp/otto-product-wide-run-scheduler-clippy.log` (`0 warnings`).
3. **Score & Boundary Assessment**:
   - Closes the last admission-capable detached supervisor in `boot/tasks.rs`. Score remains **9.7 / 10** because already-advancing detached stage tasks and non-admission housekeeping loops still rely on bounded engine/runtime shutdown (`RUNTIME_SHUTDOWN_CAP`), not a global active-job drain.
