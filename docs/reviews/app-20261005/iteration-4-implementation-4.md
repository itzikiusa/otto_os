# Iteration 4 implementation — partition 4

Status: in progress. Root owns all test/build execution. No local tests, builds, servers, commits, staging, or destructive git actions performed by this role.

## Implemented, central verification in progress

- UX4-01: ScheduledTasksPage and GoalDefineForm now register the shared router leave guard for both module and workspace navigation. Their existing local cancellation uses the same decision. Goal dirty state includes criteria, budget, providers/models, paths, and delivery/review options. Pending callbacks are fenced by form generation, workspace identity, and component lifetime.
- UX4-02: task saves acknowledge the submitted form snapshot, retain newer edits, and adopt the created ID before a second save. Errors retain the draft. Scheduled task store create/update refreshes are owned by their original workspace generation; list success/error/finally are owned by the latest request, including A→B→A.
- P4-02: shared completed node bodies obey both 48 entries and a 16 MiB serialized UTF-16 estimate, use access LRU, and deliver oversized results without caching. Cache eviction does not mutate displayed references. Aborted older requests cannot replace a newer cached result. The estimate is not process RSS.

## Executed evidence (root)

1. Initial actual-source UI regression batch: 21 tests, 9 passed, 12 intended behavioral failures, no harness errors; `/tmp/otto-review05-role4-ui-red.log`.
2. After form/cache repair and addition of the store creation test: 22 tests, 21 passed; all original 12 failures green, only the newly added cross-workspace create refresh failed (`task-A != task-B`).
3. After create refresh repair and new response-order tests: 25 tests, 22 passed; A→B→A old success/error cleared the current loading state and reversed same-workspace completion overwrote the newer rows. These three failures justified generation fencing.
4. Generation-fencing production repair is source-ready; root green run and independent source review pending.

These handler/store tests execute production TypeScript with deterministic transports. They do not replace mounted/browser or native navigation acceptance.

## Backend test checkpoint, repairs pending intended red

- C4-01: real isolated repository update/bump/snapshot calls reproduce the deterministic save/save and save/restore instruction/version mismatch, including a pinned run definition.
- C4-02: behavior-preserving extraction exposes the actual shared success/error/cancellation settlement boundary; isolated DB regression retimes a task before old settlement and checks next occurrence, due calculation, and independent history.
- C4-03: existing trigger update regression covers fired once retiming, recurring-to-once, timezone-only occurrence change, unchanged cursor ownership, and resume without duplicate firing.
- P4-01: bounded metadata version pages not yet implemented. Plan preserves array response compatibility, adds bounded keyset summary reads, and keeps single full-version fetch/restore and every historical row reachable.

Backend regressions have not yet run centrally; no backend production repair has been made beyond the approved behavior-preserving settlement extraction.

## Independent UI recheck follow-up

Root confirmed the previous focused suite at 25/25 (378 ms). Independent source review then found two integration gaps (`iteration-4-role4-ui-recheck.md`): accepted goal workspace leave did not close the persistent parent's creation view, and the actual loops store could still refresh the old workspace after launch.

New actual-source tests exercise the current LoopsPage cancellation callback together with GoalDefineForm guard registration; the pending-launch test now uses the real LoopsStore with deferred transport. Added list/create A→B→A, reversed response, error/finally, clean departure, and single-cancel controls. Root observed 26/34 passed (401 ms), with eight intended failures and the local Cancel control passing.

Repairs now invalidate the form before calling the parent's cancellation callback once on accepted leave, and fence loops list success/error/finally plus mutation refresh by request/workspace generations. One earlier harness test was corrected to edit a fresh form after a clean accepted leave, matching real unmount semantics. Mounted `ui/e2e/desktop-goal-draft-ownership.spec.ts` covers real workspace selection, confirmation, persistent page state and non-resurrection with only the provider Define response stubbed; authored but not executed. Focused green rerun and independent review are pending.

## Version paging test checkpoint

Authored four HTTP/state regressions in the existing workflows route unit module, using the repository's existing isolated ServerCtx fixture and actual Axum handlers: default/max page bounds, thin summaries, exclusive keyset traversal despite an intervening save, and full-detail/old-history restore controls (including live versus historical name/description semantics and retained history). The fixture contains 121 revisions with 4 KiB graph bodies, sufficient to prove boundedness without a large load test.

`ui/unit/workflowVersionPaging.test.ts` executes the real API wrapper and production drawer handler: requests summary/limit/exclusive cursor, retains one 50-row window, traverses to version 1 despite a new revision, and preserves separate full-version access. These tests are authored only; root's intended-red runs remain pending. No backend pagination production code or contracts have changed in this checkpoint.

## Version paging UI repair checkpoint

Root observed both new UI tests fail (0/2): the wrapper omitted `summary=true`, and the actual drawer handler did not request a bounded 50-row window. The wrapper now requests an array of metadata summaries with a clamped 1–100 limit and an exclusive `before_version`; full-version reads remain separate. The drawer retains one 50-row page and provides Older/Newest navigation, refresh/retry of the requested cursor, and an explicit end-of-history empty state. Existing selected-workflow/view/request generations continue to fence success/error/finally. Added a delayed stale-error/finally plus exact-cursor retry regression. Tests describe handler state accurately; mounted acceptance is not claimed.

Narrow `WorkflowVersionSummary` UI type and wrapper are source-ready. Root focused verification is pending; backend HTTP tests and production implementation remain unchanged awaiting central red. UI summary requests depend on that pending backend change before thin payload behavior is complete end to end.

## Backend repair checkpoint

Root established intended Rust red before repair: state `review4_` 0/2 (11.48 s compile, 0.51 s tests), showing the retained fired cursor and live/run instruction mismatch; server `review4_` 1/5 (1 m 25 s compile, 2.44 s tests), showing stale schedule settlement and all three pagination failures, while full detail/restore control passed.

Implemented source, pending central verification:

- C4-01: workflow create and publication transact the definition, version allocation and snapshot together. Existing repository update delegates to publication; PATCH and restore call publication with the actor and preserve restore's historical-snapshot versus live-label semantics. Removed the standalone bump API. Conflicting snapshot inserts fail and roll back instead of being ignored. `create_run` already inserts its version from the live row in its write transaction, so admission now sees complete committed pairs. State concurrency/rollback tests and actual HTTP PATCH/restore plus run-admission tests exercise the production boundary.
- C4-02: append-only migration `0170_scheduled_task_generation.sql` adds a monotonic internal generation (highest migration rechecked at 0169 before creation). Domain serialization skips this internal field. Schedule edit comparison, generation, rearm/fired-flag reset and next-fire calculation commit together under a write transaction. Dispatch carries the generation on the same task snapshot as its schedule; the actual shared success/error/canceled settlement boundary advances only that generation. Independent history still settles. Old unguarded runtime/rearm helpers are now test-only. Real PATCH tests cover retime away/back and recurring→once, all three terminal statuses, ordinary edits preserving generation, and successful settlement of the new occurrence.
- C4-03: trigger edits acquire the write transaction before comparing scheduling keys, including timezone. A new one-shot occurrence clears the old cursor; unchanged edits and resume preserve the database cursor and ignore a client cursor.
- P4-01: metadata queries select only id/workflow/version/note/author/time; all HTTP list pages are bounded (default 50, clamp 1–100) with exclusive version cursors. Optional summary preserves the existing array protocol; no-summary rows retain full fields. Full-version and restore endpoints stay available, with no pruning. Workflow contract and narrow UI/Rust summary types now agree. Audited repository, HTTP, UI and outward/session MCP references; no dedicated MCP version-history caller was found. The sole UI list wrapper uses summaries; direct full-version reads stay separate.

Suggested central commands: `cargo test -p otto-state --lib review4_`; `cargo test -p otto-server --lib review4_`, then the affected state workflow/trigger/scheduled suites and ordinary server checks. Root must format changed Rust before checks. All backend production and tests were stable at handoff; no test/build was executed by this role. Browser/native acceptance and performance RSS measurements are not claimed.

## Follow-up eligibility/admission test checkpoint

After central state 3/3 and server 8/8 green, independent review found that enabled-only changes incorrectly changed scheduled occurrence identity. Root also traced a stale workflow-trigger tick crossing workflow/workspace/active-run awaits before unconditional cursor advancement and queue insertion. These new paths are not accepted by the earlier green results.

Authored six actual-settlement tests for disabled one-shots: success/error/canceled, both complete-before-resume and resume-before-completion; all assert the same occurrence remains consumed, preserves history/status, and cannot become due again.

With root approval, extracted the scheduler's actual durable admission boundary without changing its two unconditional writes. Tick calls this helper. Real isolated database tests capture a due trigger, commit real retime/disable/away-back edits, then call that production boundary; they assert no queued row and no new cursor/config mutation. Added competing same-snapshot admission, an actual SQLite queue-insert failure with cursor rollback oracle, and an unchanged due admission/pinned-definition control. No provider/engine is spawned by these admission tests. No new migration or production repair in this checkpoint; root's intended-red execution is pending.

Commands: `cargo test -p otto-server --lib review4_once_pause_`; `cargo test -p otto-server --lib review4_trigger_`.

## Follow-up eligibility/admission repair checkpoint

Root's consolidated server run reproduced all 12 intended failures (six once-pause order/status cases and six invalid-trigger admission cases); current due admission and all eight earlier backend controls passed.

The scheduled-task generation now identifies timing/timezone changes only. Disabling or re-enabling affects eligibility and arming without changing the identity of an already-running occurrence, so that occurrence may settle and retain its one-shot fired flag.

Verified 0171 was the highest migration, then added reserved append-only `0172_workflow_trigger_admission.sql`. Its internal, nonserialized trigger admission generation advances on scheduling-key or enabled changes, including changes away and back. The scheduler's production admission seam now calls a repository transaction that checks enabled state, captured generation and cursor, workflow/workspace identity, and absence of any active workflow run. The cursor claim and queued run plus progress projection commit together. A failed insertion rolls the cursor back. Run insertion is shared with ordinary admission and still pins the committed workflow version. Removed the unused raw trigger-spec overwrite method; unconditional cursor seeding is test-only.

The 13 regressions remain intact. Added only two direct admission controls: concurrent different triggers cannot bypass the transactional overlap check; a captured old cursor cannot claim again after its first run has finished. Updated the existing chat-trigger test constructor for the internal field. Shared workflows repository changes were applied only around create_run/insert_run, preserving role 3's concurrent approval work.

Source-ready, unexecuted by this role. Central formatting and `cargo test -p otto-server --lib review4_`, plus affected state workflow/trigger/scheduled suites, are pending. No new API wire fields or mounted/browser claims.
