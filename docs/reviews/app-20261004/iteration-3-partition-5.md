# Iteration 3 — partition 5 ownership closure

**Verdict: Approve for the focused source recheck — zero remaining findings in this repair.** R2-5-01 is addressed, closing the remaining C5-02 source defect. Build/test execution and overall application performance acceptance remain with the parent's final gate.

Scope: the frozen follow-up in `assistant/tools.rs`, `assistant/tasks.rs`, `assistant/threads.rs`, `otto-state/src/assistant.rs`, its API documentation/type comment and `iteration-2-implementation-5.md`. This pass reviewed only the resumed-task fix and its regressions; it did not reopen the broader partition. No builds, tests, servers, process probes or application-source edits were performed. This report is the only file written.

## Disposition

| Item | Result | Source evidence |
|---|---|---|
| R2-5-01 / remaining C5-02 | **Addressed in source** | The production tool passes its resolved `tid` into `agent_update` (`crates/otto-server/src/assistant/tools.rs:178`). The existing resolver derives that thread from the calling session, checks session ownership and fetches the thread under the user (`:55`, `:62`). The mutation loads the task under the same owner and requires an exact thread match before writing (`assistant/tasks.rs:303`). |
| Explicit E1→E2 resume | **Correct in traced path** | A `running` update on a thread task requires its current execution (`assistant/tasks.rs:315`), even when a prior execution ID is already stored. `set_task_state_with_execution` updates state, needs-you payload, result and execution binding together in one SQL UPDATE (`otto-state/src/assistant.rs:788`), guarded against already-terminal rows. The production wrapper retains the task lock across the operation. |
| Untouched stale task isolation | **Preserved** | Stop still matches the stored execution ID exactly (`assistant/threads.rs:115`) and does not fall back to the newest turn. Without an explicit resumed update, E1 cannot cancel E2. An explicit authorized update binds E2; subsequent Stop targets that ID. If E2 ends before a later E3 starts, the bound E2 likewise cannot signal E3. |
| Foreign/no-active caller | **Rejected before mutation** | A foreign owner fails the owner-scoped lookup. A foreign or absent calling thread targeting a thread-bound task fails the equality check. A matching thread without an active execution fails with Conflict before the SQL update (`assistant/tasks.rs:303`, `:318`). User-scoped tasks with both thread values absent retain their documented user-scoped behavior; they are not assigned an unrelated execution. |
| Terminal/takeover task | **Preserved** | Taken-over tasks and terminal tasks are rejected by the production mutation (`assistant/tasks.rs:309`); the SQL terminal-state guard remains a second barrier (`otto-state/src/assistant.rs:794`). |
| Public contract | **Aligned** | `docs/contracts/api.md:4400` describes owning-thread authorization, atomic running rebinding, no-active-execution conflict and threadless behavior. `ui/src/lib/api/types.ts:11137` adds the corresponding ownership comment without exposing the internal execution ID. |

## Regression-source assessment

`explicit_task_resume_rebinds_stop_to_the_new_execution` (`crates/otto-server/src/assistant/threads.rs:959`) uses migrated in-memory SQLite and real `TurnClaim`/cancel signals. It creates an E1-bound task, drops E1, claims E2, verifies untouched stale Stop cannot signal E2, invokes the production mutation helper, checks the stored E2 binding and cleared question, then requires Stop to signal and finish E2 within a one-second timeout. It subsequently verifies terminal cancelled persistence. This test would fail on the reported pre-fix binding behavior.

`foreign_thread_or_owner_cannot_rebind_an_agent_task` (`assistant/threads.rs:1033`) exercises a foreign thread, an absent caller thread, another owner and a same-thread update after its execution ends. Each rejection preserves the stored binding, and current/foreign controls remain unsignaled. The existing `stopping_an_old_task_never_signals_a_replacement_turn` (`:1107`) provides additional unchanged stale-control isolation coverage.

These tests execute the actual repository mutation and in-memory cancellation mechanism when run; they do not themselves invoke the HTTP/auth stack, a real CLI process or the complete `act` endpoint. This reviewer inspected their source but did not execute them. The implementation report accurately states these limits. The parent owns final Assistant/state tests and integration checks against this revision; earlier passing results do not certify this follow-up.

The iteration-2 Insights policy correction remains source-addressed as recorded in that report. All ten original partition-5 IDs now have a source-level resolution across the three review iterations. That closure does not assert physical provider acceptance or an overall CPU/RAM improvement; the outstanding live ClickHouse workload attribution and centralized performance measurements remain outside this focused pass.

## Browser integration follow-up — P5-02 acquisition lifetime

The parent's final browser run exposed a lifecycle regression that the earlier source/store-only checks missed: replacing the thread metadata object with another object carrying the same ID reran ChatView's acquisition effect. Its cleanup released and evicted history, then acquisition issued another GET that could replace a just-received live reply. The parent recorded first GET at 1745 ms, loaded assertion at 1918 ms, live event at 1940 ms and unwanted GET at 1945 ms, with lost text.

The focused fix at `ui/src/modules/assistant/ChatView.svelte:40` derives the primitive `threadId` and makes the effect depend on that value, rather than reading `thread.id` directly inside the effect. Source recheck: same-ID metadata replacements preserve the acquisition because the derived string is unchanged; changing the ID still runs the old cleanup and acquires the new thread, and unmount still releases it. `acquireTurns` remains inside `untrack`, so its store reads do not add dependencies. This closes the identified P5-02 lifecycle regression without changing eviction or reconnect bounds.

The parent reports that the repaired browser run now retains both incremental reply texts, observes exactly one history GET and exercises Stop successfully. The remaining assertion failure concerns `Cancelled` versus the separately owned expected copy `Canceled`; therefore this note does **not** claim the complete browser spec is green. This reviewer inspected only the focused source diff and did not rerun tests. The prior store-harness tests could establish cache eviction but could not establish the Svelte prop/effect lifetime; the browser evidence supplies that missing integration check.
