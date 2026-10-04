# Iteration 1 implementation — partition 5

Implemented C5-01–05, P5-01–03 and UX5-01–02 on the shared review branch. Original findings were source-traced; no pre-fix runtime failure is claimed. This role ran source inspection and rustfmt only. Builds, unit suites and browser execution are serialized by the root agent; their reported results are recorded below with their scope. No real user sessions, AWS resources or installed plugins were used as fixtures.

## Changes and regression mapping

| Finding | Implementation | Regression evidence |
|---|---|---|
| C5-01 canonical approvals | `otto-state/mcp_control.rs` makes pending→decided conditional in SQL. Assistant reconciles a losing request to the actual canonical decision and creates an always-allow grant only after winning approval. Terminal task rows reject late state overwrites. | Concurrent approve/deny has one winner; canonical denial followed by Assistant approve-with-grant returns denied and leaves grants empty. |
| C5-02 execution cancellation | Internal task `thread_execution_id` binds an agent task to the current exclusive thread driver. Stop signals and waits for that exact driver; stale ownership cannot cancel its replacement. Delegation Stop signals the recorded run and waits for settlement. Personal-agent runs reserve cancellation before their atomic configured DB insert or running event, keeping the guard through execution. Canceled session creation completes its cleanup without submitting a prompt. | Thread replacement signal isolation and exclusive claim release; terminal task cannot be revived. Registration-before-insert and creation cleanup were additionally source-traced after root found the original publish window. |
| C5-03 takeover/handback | Thread takeover suspends its owned CLI and waits for driver exit. Handback submits a continuation before settling the task, so failed submission retains the actionable card. Unsupported delegation takeover/handback return 409; BrowserCard omits those controls. Unbound legacy running tasks reject unsafe control. | Transition test rejects delegation takeover and handback; ownership signal test. No real provider pause/resume acceptance run is claimed. |
| C5-04 plugin reinstall | One administrative lifecycle lock serializes install/enable/disable/remove. Reinstall disables and stops the old process before replacing its local files or rotating its token; persistence forces disabled state until explicit enable. | Isolated shell-process regression verifies old process removed, replacement disabled, changed executable recorded and both disabled tokens rejected. |
| C5-05 Athena history | History responses retain their originating region; opening one sets the execution region used by status/results/cancel. Region/catalog/history/execution generations reject obsolete responses and destroy invalidates polling. | Production-function regression exercises EU history execution ownership, region change and late foreign response rejection. No live AWS call. |
| P5-01 Assistant indexing | ReplyIndex requests finalized turns since its record cursor; initial/replaced files use the shared bounded snapshot for the most recent 30 turns. Only changed textual replies produce upserts. Exact-text dedup retains at most 30 replies, each capped at 64 KiB. | Existing whole-file equivalence fixture now accumulates deltas, covers unchanged and replaced files; text extraction tests retain tool-only and UTF-8 bounds. |
| P5-02 Assistant retention | ChatView acquires/releases history. Last release aborts its request and evicts heavy data. Reconnect fetches only acquired threads with concurrency two; monotonic tickets reject stale loads; live history is capped at 200 turns / approximately 2 MiB serialized UTF-16 text, retaining at least the newest turn. Pending sends remain separate. | Production-store tests visit 100 threads then refresh only one acquired view; released late fetch/live events cannot restore history; pending send survives and large live histories remain bounded. |
| P5-03 Insights cost | Specific-key report-status performs three artifact stats, no archive enumeration. Completion requires changed HTML revision, not an early summary. Archive listing hydrates only an offset page (max 200); summaries are opt-in. Home requests at most one newest report per kind. A 64-entry summary preview cache reads at most 64 KiB/file and invalidates on mtime/length. | Counted real filesystem operations with 300/1,000 synthetic report triples: 10 status calls = 30 metadata calls, zero summary reads; summary-first does not change HTML revision. Page and Home projection budgets; external-edit/cache-cap checks. Existing regeneration/timeout browser mocks now model status and HTML revision. |
| UX5-01 drafts | Documents and Autonomy register dirty guards; pending saves block departure. Autonomy fields cannot change during save. Router exposes awaited goChecked so AgentPage keyboard navigation focuses the accepted tab after a canceled guard. | Production-guard unit tests; new browser tests mount actual Documents/Autonomy with ConfirmDialog, exercise keep-editing, pending/failed saves, successful save or explicit discard. |
| UX5-02 template retry | Sheet retains the first created agent and remaining schedule request, updates that same agent on retry, and presents partial-success feedback. Schedule POST accepts optional idempotency_key, persisted under a unique agent/key constraint; conflicting reuse returns 409. Template cannot change midway through partial setup. | Production save-function test asserts one create and stable schedule agent/key across failure/retry; concurrent repository retries return the same schedule and reject a different payload. |

Append-only migrations: `0168_assistant_task_execution.sql` (internal, serde-skipped ownership) and `0169_personal_agent_schedule_request.sql` (optional schedule request key). API contracts and TypeScript mirrors were updated together for Insights, schedule idempotency, Assistant control semantics and disabled plugin reinstall. Role1's shared transcript API is consumed without whole snapshot cloning. Other roles' files and Claude's visual changes were preserved; Insights `Canceled` spelling is retained.

## Verification received from root

- `/tmp/otto-review-platform-state.log`: two focused state regressions passed (atomic approval winner and canceled task cannot revive).
- `/tmp/otto-review-platform-assistant2.log`: 49 server Assistant tests and seven state tests passed. This preceded the final personal-agent registration/session-creation cleanup and final additional regressions; it does not certify those later edits.
- `/tmp/otto-review-final-recovery-unit.log`: platformRecovery plus Product/Vault targeted run passed 26 tests.
- `/tmp/otto-review-platform-check.log`: npm check, zero errors/warnings.
- `/tmp/otto-review-platform-full-check.log` and `...-unit.log`: full UI check passed, 1,072 unit tests passed. Later fixture/status mocks, BrowserCard legacy delegation control gating, template selector disable and Autonomy save-field disable still need the final UI rerun.
- Root's stable broad clippy compiled this role's final state/server production changes; two unrelated Swarm lints were corrected centrally. Combined seven-package Rust library verification is currently running against frozen production source.
- Rust formatting completed successfully for this role's changed Rust paths. No build/test processes were started by this role.

Exact focused commands for central execution (repo root unless marked ui):

```sh
cargo test -p otto-server --lib assistant::
cargo test -p otto-server --lib insights::
cargo test -p otto-server --lib plugins::proxy_timeout_tests::replacing_enabled_plugin_stops_old_process_and_disables_rotated_token
cargo test -p otto-state --lib schedule_create_retry_after_lost_response_is_idempotent
cargo test -p otto-state --lib simultaneous_approval_decisions_have_one_winner
cargo test -p otto-state --lib cancellation_cannot_be_revived_by_a_late_completion
```

UI root: `npm run check`; `npm run test:unit`. Isolated browser specs: `desktop-personal-documents.spec.ts`, `desktop-personal-autonomy.spec.ts`, and `desktop-ux-r{2,3,4}-insights.spec.ts`; use root's selected Playwright project and slot. New tests use routed synthetic APIs and temporary report/process fixtures.

## Remaining validation and bounds

The parent owns final integration/browser gates, independent re-review, serialized CPU/RAM workloads, CI and merge. Those results belong in the central verification record; this report does not infer green outcomes from authored tests.

Archive page discovery still enumerates filenames O(R); it no longer opens/stat-hydrates every report or repeats that enumeration for completion polling. Assistant initial/replaced transcript folding still reads the file, and Folder retains its canonical history; this change removes repeated deep snapshots and redundant textual upserts, not all transcript memory. Shared turn-delta extraction may still scan lightweight record metadata. The browser budget is an estimated serialized-text bound plus one newest turn, not a measured heap guarantee. Physical provider takeover/resume, arbitrary plugin crash behavior and real AWS calls remain outside the synthetic verification. Delegated runs deliberately do not advertise resumable takeover.
