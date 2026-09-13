# Otto Responsiveness Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans task-by-task with regression-first verification. Root coordinates three parallel workers plus local work, independent reviews and all commits/PR operations. Steps use checkbox syntax.

**Goal:** Implement all twelve approved performance findings without disturbing the running app or losing stored data.

**Architecture:** Bounded asynchronous work, per-resource lifecycle ownership, metadata-first views and incremental Vault indexing. Existing full-detail and legacy API contracts remain available; new summary paths serve the first-party UI.

**Tech Stack:** Rust/Tokio/Axum/SQLx SQLite, Svelte 5/TypeScript, Node unit tests and isolated Playwright fixtures.

**Reviewed design:** [Design](../specs/2026-09-13-responsiveness-design.md). Base f2b885b0; branch perf/responsiveness-20260913. One PR, green Actions, admin merge; no deployment or restart.

## Review resolutions and execution order

All four plans received independent review. Root reviewed Agents/Workflows; API reviewer reviewed shared interactions; Git reviewer reviewed Vault; Vault reviewer reviewed Connections/API. Required corrections are accepted and binding:

- Script input admission precedes the runner's first postMessage; Worker validates output before posting. Explicit clone/messageerror and zero-post oversize regressions are required.
- Every checkpoint import/raw insert receives safe stable cursor ordering; imported derived ordinals are not authoritative. Conditional progress must not hide invalidated projections. Prefer existing SQLite rowid where its proven semantics satisfy the tests.
- Closing connection cleanup owns precisely detached old-generation resources and completes after caller cancellation, without broad later removal that could affect a fresh generation.
- Vault structural reconciliation is batched; never run a global link pass once per newly discovered file. Bound pending preparations and retained batches as well as running workers.

The independent lanes may implement concurrently after these resolutions are incorporated. Root coordinates shared-file edits and targeted Cargo execution. Implementation then receives spec-compliance and code-quality review; relevant tests rerun after fixes. Broad local/CI gates precede the single PR's merge. No user questions or additional approval are required for this authorized process.


---

## Shared interactions

## Root lane: shared interactions

### R1. Terminate API scripts outside the UI thread

Files: modify `ui/src/lib/api/scripts.ts`, `ui/src/lib/stores/apiClient.svelte.ts`; create `ui/src/lib/api/scriptWorker.ts`, `ui/src/lib/api/scriptRunner.ts`, `ui/src/lib/api/scriptWorkerFactory.ts`, `ui/unit/scriptRuntime.test.ts`, `ui/unit/scriptRunner.test.ts`, `ui/e2e/desktop-api-script-worker.spec.ts`; update `docs/features/api-client.md`.

- [ ] Add runtime regressions for existing `pm` request/header/variable/test behavior and output bursts; assert 1001 logs/tests produce a clear budget error and retained text stays under 256KiB. Run `cd ui && node --test unit/scriptRuntime.test.ts` and record the expected missing-budget failure.
- [ ] Add one shared output-budget collector used by console methods and `pm.test`, counting entries/text before retaining. Keep existing synchronous exports pure for Worker use and small fixtures.
- [ ] Add typed Worker request/result messages carrying request or response plus variables. The runner checks the 8MiB input budget BEFORE its first postMessage. Worker executes synchronous runtime, bounds returned payload inside the Worker to 8MiB BEFORE posting, and posts structured results; catches serialization/runtime failures with clear errors.
- [ ] Keep Vite-specific `new Worker(new URL(..., import.meta.url))` construction in the tiny factory module so existing VM store fixtures can inject the worker boundary without parsing import.meta. Add an asynchronous runner accepting AbortSignal, with owned Worker construction, 5s deadline, single terminal cleanup and termination on success/error/abort/timeout. Construction errors reject without a main-thread fallback. Unit fixtures cover abort-before-start, timeout, late message, construction failure, oversized input with zero postMessage calls, synchronous DataCloneError, messageerror, and exactly-once cleanup.
- [ ] Move execute controller/sending setup before pre-script. A current-controller/workspace/tab identity owns all publications. Await pre Worker, publish successful request/variables only if current; never dispatch HTTP after canceled/failed pre. Await post Worker with same signal and publish only current results. Closing the owning tab cancels; workspace disposal/replacement send reuse cancelExecute.
- [ ] Run `cd ui && node --test unit/scriptRuntime.test.ts unit/scriptRunner.test.ts`. Expected all pass. Add actual-browser infinite pre/post loops, Cancel, finite mutation and stale execution checks; no arbitrary script runs on test runner/main thread.
- [ ] Document Worker environment (pm preserved, DOM unavailable) and execution/output budgets. Root records isolated browser command/results during R4 integration.

### R2. Bound directory browse work and virtualize the shared picker

Files: modify `crates/otto-server/src/routes/fs.rs`, `ui/src/lib/components/FolderPicker.svelte`, `ui/src/lib/components/VirtualList.svelte`, `ui/e2e/desktop-folder-picker.spec.ts`, `docs/contracts/api.md`; create a small `ui/unit/folderBrowse.test.ts` and pure helper only if ownership/deadline logic warrants it.

- [ ] Add Tokio fixture exercising browse admission with injected blocked work. Assert unrelated async task responds, four jobs retain permits after caller abort, fifth returns busy, and permits release only when actual jobs finish. Add complete temp-directory sorting/authorization fixtures. Run `CARGO_BUILD_JOBS=2 cargo test -p otto-server routes::fs::tests --lib` and capture intended failure before implementation.
- [ ] Extract existing synchronous browse into authorized blocking operation, preserving canonical allow/deny guards, full enumeration and returned shape. Use four try-acquired global permits, move permit into spawn_blocking, own cancellation flag/drop guard, check flag between entries/stages. Apply ten-second await deadline without pretending to interrupt OS syscalls. Return existing Conflict/Upstream errors with actionable Retry text. Sort with cached lowercase keys.
- [ ] Picker owns per-load AbortController plus 12s deadline, cancels replacement/close/destroy, retains generation checks and always clears timers. Preserve path/history/Favorites/Recents, full filter and selection checks. Timeout restores controls with Retry.
- [ ] Virtualize fixed-height entry rows via shared VirtualList while retaining full `shown` array. Reset viewport on path/filter changes. Use the reviewed ARIA-grid adapter for all listing sizes: stable container focus, active row/action column, Arrow/Page/Home/End navigation, Enter/Space invoke eligible action, Tab exits. Extend VirtualList with controlled scroll index and one pinned active row; aria-activedescendant remains valid under pointer scroll. Remove pointer buttons from sequential Tab order. Maintain viewport clamp. Test last-row keyboard Use, file Select, disallowed gitOnly actions, filter/path reset and valid active descendant after wheel scroll.
- [ ] Extend browser regression with delayed browse cancellation/Retry, a synthetic large complete listing, bounded rendered rows, end-of-list filter/selection and keyboard navigation, and existing mobile/Favorites/history flow. Run targeted server and helper tests; R4 runs actual browser fixtures.
- [ ] Contract documents busy/deadline errors and complete search scope; no response truncation is introduced.

### R3. Bound worktree status probes and represent unknown status safely

Files: modify `crates/otto-git/src/local.rs`, `crates/otto-git/src/parse.rs`, `crates/otto-git/src/http.rs`, `crates/otto-core/src/api.rs`, `ui/src/lib/api/types.ts`, `ui/src/modules/git/GraphView.svelte`, `docs/contracts/api.md`; add focused probe helper or UI force helper only where it makes real race/safety tests possible.

- [ ] Add real temporary-worktree regression for clean/dirty/missing paths and deterministic sleeping Git shim tests for bounded concurrent probes. Require `dirty_known=false` on failed/timeout probes; capture the pre-change failure with `CARGO_BUILD_JOBS=2 cargo test -p otto-git worktree --lib`.
- [ ] Add backward-deserializable `dirty_known` alongside dirty in Rust/TS/parser constructors. Parser defaults unknown until successful probe; preserve output ordering.
- [ ] Route status through LocalGit LocalRead runner with per-probe3s budget, four admitted tasks per listing, global8permits, ten-second optional phase. Acquire before spawning and retain permits in admitted tasks through bounded process-group cleanup after caller cancellation. Stop queueing when phase budget ends; remaining entries stay unknown. Preserve configured Git binary in probes for tests.
- [ ] Test timeout descendants cleanup with owned shim PIDs and temp files only. No real user worktree modifications. Gate HTTP forced removal on fresh known status; unknown force returns409, ordinary false remains Git-checked. Cover locked+unknown and ensure no force command dispatched.
- [ ] Render explicit unknown hint and explanatory removal text; never compute force from unknown dirty/locked. Update WorktreeInfo contract and any fixtures. Run targeted git tests and UI helper checks.

### R4. Root integration and release gates

- [ ] Merge narrow shared-file edits from all lanes; validate all twelve report IDs map to code, tests, contracts and measured limitations. No source-only claim of full completion.
- [ ] Run `CARGO_BUILD_JOBS=2 cargo test --workspace`, `CARGO_BUILD_JOBS=2 cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` (baseline advisory formatting differences reported, do not reformat unrelated files). Run `cd ui && npm run test:unit && npm run check && npm run build`.
- [ ] Build only isolated test daemon via `CARGO_BUILD_JOBS=2 cargo build -p ottod`; run owned temporary-state browser fixtures with `OTTO_E2E_BIN` pointing at worktree target/debug/ottod, explicit unique ports/slot, providers disabled. Audit setup/teardown first: no installed binary or global process cleanup may be used. Target folder, Worker, history, workflow and Vault changed flows plus relevant existing regressions.
- [ ] Independent spec compliance review, then code-quality/correctness/performance/security review across lanes. Resolve material findings, rerun affected gates, document exact outcomes and remaining limits in consolidated report.
- [ ] Follow commit-message skill for focused commits; stage only owned files. Follow pull-request skill, summarize full branch and validation, open one PR. Run/watch required Actions and CodeQL on exact head; fix real failures, never weaken gates. Once every required check succeeds, merge with approved admin authority and exact head guard. Do not delete branch, deploy, restart, install, or replace running app.


---

## Agents and workflows

# Agents and Workflows Responsiveness Implementation Plan

> For agentic workers: execute task-by-task with `superpowers:executing-plans`, `superpowers:test-driven-development`, and `superpowers:verification-before-completion`. Root coordinates independent plan/implementation review; this owner does not spawn agents or commit.

**Goal:** Make chat recovery, Agents History and workflow progress proportional to visible information while preserving drafts, authorization, exact recovery bodies and legacy callers.

**Architecture:** Window-local view leases control transcript reads and intentional resume; a bounded daemon cache reuses unchanged folded transcripts. A new bounded metadata History cursor precedes filesystem resolution. Persisted workflow projections provide summary polling, stable checkpoint pages and explicit exact-body reads.

**Tech Stack:** Svelte 5/TypeScript, Rust/Axum/Tokio, SQLx/SQLite, existing Node unit runner and isolated Playwright fixtures.

**Binding inputs:** `docs/superpowers/specs/2026-09-13-responsiveness-design.md`, `/tmp/otto-perf-design-agents-workflows.md` including its binding review addendum. All application edits wait for independent plan acceptance.

## Working boundaries and commands

Every shell invocation sets workdir explicitly to `/Users/itziklavon/.config/superpowers/worktrees/claude_ade/performance-20260913` (UI commands use its `/ui` child). Original checkout, installed app/daemon, real provider roots and user databases remain untouched. No deployment, installation, restart, real provider invocation, commits or remote actions by this owner. Tests create disposable directories/databases; provider resume is an injected counter. Root owns build/CI scheduling and PR.

RED means the newly added behavioral assertion fails against existing behavior; missing symbols are a preliminary compilation check, not sufficient regression evidence. Where a new interface cannot compile against old code, introduce only an adapter preserving old behavior, observe the assertion fail, then implement the bounded behavior. Record command, exit status and decisive assertion in the owner handoff. Re-run the same command GREEN before the next task.

Shared edits: coordinate narrow snippets in server `state.rs`, `lib.rs`, routes `mod.rs`, `policy.rs`, core/UI DTOs and `docs/contracts/api.md` with root. No broad reformat during parallel edits. Migrations exclusively `0129_history_page.sql` and `0130_workflow_progress.sql`; do not alter old migrations or Vault's0131.

## File and responsibility map

Create:
- `ui/src/lib/stores/transcriptLifecycle.ts`: pure leases, request generation, bounded scheduling and inactive-cache admission; no HTTP or draft storage.
- `ui/unit/transcriptLifecycle.test.ts`: deferred-read and fake-clock lifecycle/cache regressions.
- `crates/otto-server/src/transcript_cache.rs`: daemon-owned, stamped immutable folded snapshots and bounded cold admission; unit tests in this module.
- `crates/otto-state/src/history_page.rs`: authorized candidate SQL/keyset query and batch metadata lookup; repository tests in module.
- `crates/otto-server/src/routes/history_page.rs`: page DTO/cursor validation, candidate resolution and compatibility filters; route-local tests.
- `ui/unit/historyPagination.test.ts`: cursor/filter/empty-page merge behavior; a small `ui/src/modules/agents/history/pagination.ts` helper only if needed to expose pure logic.
- `crates/otto-state/src/workflow_progress.rs`: bounded projection builders, projection queries/repair and versioned detail queries; repository tests in module.
- `crates/otto-server/src/routes/workflow_progress.rs`: authorization and additive progress/checkpoint/detail handlers.
- `ui/src/modules/workflows/runProgress.ts`: pure summary/detail merge and bounded cache state, independent of component rendering.
- `ui/unit/runProgress.test.ts`: stale snapshot, checkpoint continuation, expansion/cache and exact-detail behavior.
- `ui/e2e/desktop-agents-responsiveness.spec.ts`, `ui/e2e/desktop-workflow-progress.spec.ts`: isolated real-UI route/visibility/detail coverage.

Modify existing wiring only where needed:
- `ui/src/lib/stores/transcript.svelte.ts`, `ui/src/modules/agents/conversation/ConversationView.svelte`, `ui/src/modules/agents/SessionView.svelte`, `ui/src/shell/App.svelte`, `ui/src/lib/events.svelte.ts`.
- `crates/otto-server/src/routes/transcript.rs`, `transcript_tail.rs`, `state.rs`, `lib.rs`.
- `crates/otto-state/src/lib.rs`, `workflows.rs`; `crates/otto-core/src/workflows.rs`.
- `ui/src/modules/agents/history/history.svelte.ts`, `HistoryPage.svelte`.
- `ui/src/lib/api/types.ts`, `ui/src/lib/api/workflows.ts`, `ui/src/modules/workflows/WorkflowsPage.svelte`, `RunSteps.svelte`.
- `crates/otto-server/src/routes/mod.rs`, `policy.rs`, `docs/contracts/api.md`; narrowly `state_archive/schema.rs` only if derived-field exclusion is needed beyond SQL-trigger invalidation.

## Task1 — Active-view leases and cancellation fence

- [ ] Add deferred-resync tests: open A and B, close B, reconnect/visibility return must GET/touch only A; closing A between GET resolution and touch must result in zero touch. Two leases for A share one read; releasing one retains A. Different store/window instances remain independent.
- [ ] Run from UI: `node --test unit/transcriptLifecycle.test.ts`; observe the existing fan-out/late-touch assertions fail.
- [ ] Implement source-keyed lease reference counts, document visibility and generation checks after every read await. Queue at most one pending/trailing operation per leased source and run at most two different sources. Hide/unmount cancels reads and queued work; request cancellation does not cancel pending sends.
- [ ] Wire ConversationView mount/source cleanup. Remove SessionView's unused eager conversation/ensure probe. Replace shell/reconnect `resyncAll` calls with active-visible scheduling and remove shell-wide periodic workspace touch. Keep legacy touch endpoint and explicit active-visible touch behavior.
- [ ] Add tests for reconnect storms, hide/unhide, source change, failed/busy reads and delayed response from the prior daemon identity. Busy retries are delayed/coalesced, only while leased/visible, and never call touch after a failed GET.
- [ ] Re-run unit command GREEN; inspect all `ensure`, `resyncAll`, `touchWorkspace` call sites to ensure no hidden fallback remains.

## Task2 — Bound inactive conversations without losing drafts

- [ ] Extend lifecycle tests with 30 released conversations and synthetic body charge; retain at most24 inactive/32MiB, evict LRU inactive only. Oversized mounted older pages survive; releasing them makes them eligible. Draft, pending-send and view preference objects are unaffected by eviction.
- [ ] Run the same unit command RED against unbounded retention.
- [ ] Implement conservative byte charging and eviction on release/growth. Keep active lease metadata separate from fetched pages and preserve scroll/selection metadata separately when body eviction requires a reload. Do not evict active or currently sending conversation state. Clear read/cache generations on identity switch while retaining existing daemon/user-scoped draft keys.
- [ ] Re-run GREEN. Add browser assertions to the Agents spec: type an unsent draft, switch tabs/module, reconnect, revisit; draft remains and unselected cached sessions received no transcript/touch requests. Terminal-only session tabs make zero transcript requests until Chat opens.

## Task3 — Bounded immutable server transcript cache

- [ ] Add temporary-JSONL cache tests using a counting fold closure: repeated unchanged pages fold once; concurrent same-key calls fold once; two blocked distinct keys cause a third miss to return busy promptly; at most eight attached waiters per key; warm hit still succeeds when workers are saturated.
- [ ] Run root: `cargo test -p otto-server transcript_cache -- --nocapture` RED.
- [ ] Implement daemon-owned cache with max32 entries,128MiB total conservative charge,32MiB per-entry retention cutoff and2min idle expiry. Use two owned try-acquired worker permits and bounded same-key coalescing. No distinct-key queue; report retryable409. Map locks never span I/O/folding. Move actual work/permit ownership into blocking closure so canceled waiters cannot release capacity early.
- [ ] Key canonical data-root/path/provider/subagent/options identity and stamp dev/inode/size/nanosecondmtime plus relevant Claude subagent metadata. Capture before/after; changed-during-fold results cannot publish as current. Test append/truncate/same-size replacement/sidecar mutation, canceled waiter, eviction and independent-key progress with deterministic barriers, not sleeps.
- [ ] Wire session/history GET only AFTER existing authorization and canonical path confinement. Return page data from Arc snapshot, preserving existing page budgets and DTOs. Maintain current GET arm-tail behavior for running sessions; never add provider resume to cache lookup. Keep explicit touch as intentional resume.
- [ ] Allow existing tail publication to share its already-produced immutable snapshot only when stamp reflects exactly consumed state; otherwise skip publication. Do not introduce an extra full clone to fill cache.
- [ ] Re-run focused cache tests GREEN and existing transcript route/tail tests. Add authorization-hit test and first-open running-chat append fixture proving live delivery still starts, while closed reconnectable sessions remain unresumed.

## Task4 — Bounded authorized History candidate engine

- [ ] Add fixture repository tests for10,000 sessions/index records, owner/admin scope, global cross-provider claimed-ID exclusion, timestamp ties and duplicate indexed IDs. Count SQL queries and resolver invocations through a test-injected resolver. A100-entry page must never resolve more than400 candidates.
- [ ] Run root: `cargo test -p otto-state history_page -- --nocapture` and `cargo test -p otto-server history_page -- --nocapture` RED.
- [ ] Add migration0129 only for missing workspace/owner/activity/keyset, global claimed-path/PSID and transcript lookup indexes; record EXPLAIN QUERY PLAN fixtures for indexed scope and NOT EXISTS lookups. Verify existing indexes before adding duplicates.
- [ ] Build session and admin-only on-disk metadata candidate streams in stable `(last_active_at DESC, source_key ASC)` order. Apply workspace/owner/status/cwd/cursor in SQL; on-disk scope intentionally remains machine-wide for admin/root only. Use indexed NOT EXISTS, never fetch all claimed rows into memory. Preserve global PSID exclusion without provider restriction.
- [ ] Read no more than `4*limit` candidates (default100/max1000). Normalize q and authoritative title/first_prompt/cwd with Rust `to_lowercase`, exact current substring semantics; literal percent/underscore remain literal. Resolve only candidates still possibly matching; uncertain resolver-derived metadata must not be filtered out prematurely. Bound resolution in blocking workers and batch-fetch actual resolved-path index metadata with bounded SQL chunks, not N per-row queries. Effective provider is the resolver result.
- [ ] Advance cursor after the last scanned candidate, including filtered/missing/unresolvable rows; stop immediately when page fills. Candidate lookahead must not skip an unprocessed row. If budget is exhausted and rows remain, return continuation even if entries is empty.
- [ ] Add Unicode (including non-ASCII case mapping), cwd sibling boundary, provider-metadata mismatch, match at candidate250 and match beyond400 reachable on continuation tests. Add deletion/import between pages and equal-time tie paging with identity dedupe.
- [ ] Re-run both focused commands GREEN. Keep legacy `history` function unchanged and add compatibility assertions demonstrating it has not acquired bounded early-stop behavior.

## Task5 — History API and UI continuation

- [ ] Specify additive `GET /workspaces/{wid}/history/page` in contracts and mirror DTOs: `{entries,next_cursor}`; validate <=4KiB versioned cursor scoped to workspace/actor-role/filter digest. Recheck current authorization on every page; cursor is not authority.
- [ ] Register handler/policy; add route tests rejecting malformed/scope-mismatched cursors and unauthorized admins-only on-disk exposure. Error responses contain no candidate paths from denied scopes.
- [ ] Add UI unit tests for filter-change stale response, tied IDs, empty-but-continuing page and final empty page. Run UI: `node --test unit/historyPagination.test.ts` RED.
- [ ] Move History store to cursor API; keep selected conversation and drafts independent. Display actionable Load more whenever cursor remains, including zero matches; never display final No results prematurely and never automatically exhaust arbitrary empty pages. Reset cursor on filter/workspace changes; preserve stale-generation guards.
- [ ] Re-run unit command GREEN and add browser fixture match beyond first100/budget continuation behavior to Agents responsiveness spec.

## Task6 — Durable workflow projections and direct-write safety

- [ ] Add repo tests with30 checkpoint bodies of512KiB each. Assert warm progress/unchanged reads select no `nodes_json`, `input_json` or `checkpoint_json`, return bounded summaries, and exact detail remains byte/value-equivalent. Use query/decode instrumentation rather than latency thresholds.
- [ ] Run root: `cargo test -p otto-state workflow_progress -- --nocapture` RED.
- [ ] Add migration0130 nullable `progress_json` and checkpoint summary/version/ordinal fields; run-level checkpoint_rev and checkpoint_generation. Add AFTER INSERT/UPDATE OF authoritative-body triggers invalidating copied/old projections; checkpoint mutations advance checkpoint/run revisions, and deletion/reset advances generation. Allocate immutable append ordinals transactionally; preserve on normal updates. Existing rows initially repair lazily rather than a giant JSON-decode migration transaction.
- [ ] Add pure projection builders with bounded error/phase previews, identity/status/timing/attempts/session IDs, output-presence and log counts only. No log text, input or output body copies. Normal repository writes serialize authoritative body once, write it then publish derived projection in the SAME transaction; triggers cannot leave a successful repository write stale.
- [ ] Audit create_run, update_run, update_run_progress, checkpoint save, prepare_retry, approval/proof/version/terminal mutation methods. Progress revision must change for every visible summary change; terminal-state guards/retry atomicity remain intact. Do not introduce revision races by querying current rev outside the write transaction.
- [ ] Implement missing-projection repair from an exact SQL read snapshot with two admitted workers/no unbounded queue, per-run single-flight and fenced conditional publication. If authoritative state changed, discard obsolete projection and retry within bounded attempts or return retryable busy. Preserve exact authoritative body; invalid body errors explicitly. Warm normal reads must not invoke repair.
- [ ] Add raw SQL insert/update tests (including supplied forged projection), raw checkpoint insert, imported archive-shaped rows, and blocked-repair/concurrent mutation. Assert latest state wins and exact bodies remain unchanged. Re-run focused test GREEN.

## Task7 — Stable checkpoint paging and exact details

- [ ] Add tests: page one, then update unrelated node/log/checkpoint and append checkpoint, then continue without duplicates/reset/starvation; retry/reset invalidates old generation. Updates preserve ordinal and new rows append. Use several pages and lexically misleading IDs such as `#2`/`#10`.
- [ ] Run the workflow_progress repo command RED.
- [ ] Implement checkpoint page cursor `(run,generation,last_ordinal)` with max200/default100, independent of checkpoint_rev. Return checkpoint_rev for freshness. Reset/deletion/retry increment generation atomically. Read cursor generation and rows in one SQL snapshot to avoid mixed-generation pages.
- [ ] Implement exact node/checkpoint detail query + body version from the SAME read snapshot. A checkpoint is a PK lookup; node detail may parse the existing full nodes blob only on explicit request. Compute/return the version from the exact returned body, never from separately-read progress metadata.
- [ ] Add concurrent update/detail assertion, absent/cross-run node and full recovery roundtrip tests; re-run GREEN.

## Task8 — Workflow additive endpoints and guarded summary UI

- [ ] Document/register/policy/type `GET /workflow-runs/{id}/progress?after_rev=N`, checkpoint page and node/checkpoint detail endpoints. Progress returns200 `{changed:false,rev}` when unchanged or bounded summary otherwise; authorize workspace Viewer before both branches. Summary identifies checkpoint_rev/generation/count, approvals/proof/session refs and version/context controls without embedded bodies.
- [ ] Add route tests for denied progress (including after_rev hit), denied checkpoint/detail, encoded `#` node IDs and exact detail-version matching. Run root: `cargo test -p otto-server workflow_progress -- --nocapture` RED then GREEN after handler wiring.
- [ ] Add UI tests for checkpoint-only progress change, preservation of loaded pages on checkpoint_rev update, generation reset, stale detail request after run switch, body cache16entries/16MiB, errors auto-open once and manual expansions retained. Run UI: `node --test unit/runProgress.test.ts` RED.
- [ ] Switch WorkflowsPage initial selected-run load,2.5s poll, WS-gap and terminal refresh to progress. Poll only visible mounted run; use single-flight/trailing and response generation checks. Keep legacy full GET/mutation APIs compatible. Update list/open paths so an immediate full GET does not negate the projection.
- [ ] Render RunSteps from summaries. Construct JSON/markdown and request node/checkpoint bodies only on expansion/zoom; error auto-expansion triggers one explicit detail fetch. Reject stale body versions; update an open body when its version changes. Refresh currently displayed checkpoint summary pages on checkpoint_rev changes while retaining other loaded pages; ordinary changes never reset the whole pagination cursor. Generation change clears stale checkpoint bodies/pages while preserving valid expansion intent.
- [ ] Preserve approve/retry/proof/context/version behavior: summary carries only necessary small references; actions needing full input explicitly fetch it once when invoked. No polling fallback to full-body GET. Re-run UI and route tests GREEN.

## Task9 — Isolated integration and owner review handoff

- [ ] Add real browser journeys in the two new specs, using intercepted request counters where suitable and real isolated API fixtures for authorization/repository integration. Agents: terminal-only zero reads, one active view does not resync closed chats, delayed response cannot touch after unmount, retained draft and older pages. Workflows: large checkpoint fixtures, repeated unchanged polling with no full-run/body calls, one expanded exact body, checkpoint-only update visible, retry generation behavior.
- [ ] Coordinate a test daemon artifact with root; do not build/install/restart app independently. Root may build a test-only binary for the throwaway harness under user-authorized test execution. From UI run `OTTO_E2E_SLOT=perf-agents-workflows OTTO_E2E_PORT=7892 OTTO_E2E_PW_PORT=5292 npm run test:e2e -- --project=desktop-browser --workers=1 desktop-agents-responsiveness.spec.ts desktop-workflow-progress.spec.ts` with `OTTO_E2E_BIN` explicitly pointing at that test artifact. Never default to an installed daemon.
- [ ] Run focused complete crates/modules after all edits: `cargo test -p otto-state history_page`, `cargo test -p otto-state workflow_progress`, `cargo test -p otto-server transcript_cache`, `cargo test -p otto-server history_page`, `cargo test -p otto-server workflow_progress`; UI `node --test unit/transcriptLifecycle.test.ts unit/historyPagination.test.ts unit/runProgress.test.ts` and `npm run check`.
- [ ] Run route gates: `cargo test -p otto-server --test route_inventory --test policy_coverage`. Coordinate root's broad workspace/UI gates and strict Clippy; fix only actual relevant failures.
- [ ] Run `git diff --check`; format owned Rust files while coordinating shared edits. Provide independent reviewer exact changed files and regression evidence, especially authorization, cursor empty-page semantics, trigger/direct archive path and cancellation permit lifetime. No own commit; root consolidates reviewed changes.

## Acceptance and honest limits

No cached closed session is resumed by visibility/reconnect. Visible live transcript streaming and drafts remain intact. Unchanged retained transcript pages reuse one fold; first cold folds and oversized non-retained files may still cost O(file size). Busy fold admission is explicit and retryable, not an unbounded queue.

History per-request work is bounded by metadata/resolver candidate budget; substring search can require explicit continuation through sparse metadata. Legacy consumers retain previous behavior and costs. No unrestricted machine-wide history visibility is added.

Warm workflow polls exclude authoritative bodies; large selected detail can still be expensive and is explicit. Node projection computation at an actual node update remains O(nodes); this work does not redesign existing engine checkpoint persistence or large authoritative blobs. Cold migration/archive repair is one-time bounded-admission work. Checkpoint pages remain traversable during active updates, and exact recovery/detail data is unchanged.

## Binding plan-review clarifications

- All Rust commands above execute with `CARGO_BUILD_JOBS=2`; root coordinates shared Cargo scheduling.
- Task6/7: ordinal allocation is enforced for every raw SQL INSERT as well as repository inserts. Imported supplied ordinals are derived, never authoritative: allocate from the run's monotonic ordinal sequence under the write transaction, enforce uniqueness `(run_id,ordinal)`, and repair migration-era missing/duplicate values before allowing page reads. A transaction-local allocator/trigger must not fail first on an untrusted imported duplicate before it can normalize it; use a separate derived ordering table or an insert-trigger allocation scheme whose staging column is not prematurely uniqueness-constrained. Keep checkpoint_json byte-for-byte unchanged. Add two raw inserts omitting ordinal and two supplying the same hostile ordinal, then traverse limit1 pages and assert every checkpoint appears exactly once. Include existing migrated rows and concurrent insert fixture.
- Task6: missing/stale projection detection precedes `after_rev` comparison. Trigger invalidation/raw restore and successful cold repair must advance or invalidate the visible revision used by conditional polling; a client holding the old revision cannot receive unchanged while its restored/changed projection is missing or obsolete. Add `after_rev` fixture across raw body update and cold repair, including a supplied stale archive projection and concurrent update. Returned revision and summary must be from one coherent publication; no unbounded revision-driven repair loop.


---

## Connections and API

# Connections and API responsiveness implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task after the parent accepts plan review. Steps use checkbox syntax. The parent owns independent review and integration; do not spawn agents or make independent commits.

**Goal:** Keep healthy DB operations independent of slow connection setup, make API history refresh proportional to metadata, and avoid listing remote directories to report one upload’s progress.

**Architecture:** Use per-key resource initialization with a connection lifecycle fence spanning resolution through execution. Add materialized API history metadata and an additive summary endpoint, with coalesced refresh and lazy detail ownership in the UI. Probe one literal SFTP staging path with bounded output and backoff.

**Tech Stack:** Existing Rust/Tokio/SQLx SQLite and driver libraries; Svelte5/TypeScript/Node test runner; existing OpenSSH SFTP subprocess transport. No new runtime dependencies planned.

---

## Scope, source of truth and execution rules

Accepted design: `/tmp/otto-perf-design-connections-api.md`, including the full-lifecycle amendment. Consolidated finding authority: `/Users/itziklavon/claude_ade/docs/reports/2026-09-13-performance-review.md`. This contribution owns three findings; root owns interactive script workers and other workers own their feature areas.

Every command below runs with tool `workdir` explicitly set to `/Users/itziklavon/.config/superpowers/worktrees/claude_ade/performance-20260913`. Rust commands set `CARGO_BUILD_JOBS=2`. Commands are written relative to that root. No commands have been executed as implementation verification yet. Run one relevant RED command before implementation, then the same command GREEN. A compiler failure naming the newly required interface is acceptable RED; unrelated failures are not evidence.

No live DB/SSH calls, installed daemon, provider sessions, deployment, restart, installation, global formatting or new agents. Unit fixtures use disposable state and fake executables/futures. Parent alone may later run browser fixtures after auditing E2E global setup for process ownership and setting an explicit isolated daemon binary. Do not run E2E from this contribution. Parent coordinates final workspace tests/lints/PR/merge.

Preserve existing security, connection cache keys, native cancellation, data retention, old full history/MCP responses, query timeouts, transfer authorization cadence and final publication behavior.

## File map

- Create `crates/otto-dbviewer/src/resource_cache.rs`: short-held registry plus single-flight cancellable slots and lifecycle token types, with deterministic unit fixtures.
- Modify `crates/otto-dbviewer/src/lib.rs`, `types.rs`, `config.rs`: register internal module, carry the nonserialized lifecycle token in resolved configuration, update configuration construction/Debug safely. Audit other `ResolvedConfig` literals across workspace.
- Modify `crates/otto-dbviewer/src/drivers/{mysql,postgres,redis,mongodb,clickhouse}.rs`: integrate keyed acquisition and existing-resource-only close cancellation. Keep ClickHouse HTTP constructor synchronous.
- Modify `crates/otto-dbviewer/src/service.rs`, `service/changes.rs` only where needed: lifecycle fence, tunnel slots/leases, active-key transitions, resolved operation wrappers and close cleanup. Add service seam tests inside the existing module or `service/lifecycle_tests.rs` if separate fixtures improve readability.
- Create `crates/otto-state/migrations/0132_api_history_summaries.sql`; modify `crates/otto-state/src/api_client.rs` and `crates/otto-core/src/domain.rs`: compact history metadata/query/DTO and regression fixtures.
- Modify `crates/otto-server/src/routes/api_client.rs`, `modules.rs`, `policy.rs` only for summary route/tests; modify `ui/src/lib/api/types.ts` matching DTO.
- Create `ui/src/lib/stores/apiHistory.ts` and `ui/unit/apiHistory.test.ts`: pure testable refresh/detail ownership coordinator. Modify history-only areas of `apiClient.svelte.ts`, `ui/src/modules/api/HistoryList.svelte`, `ApiPanel.svelte`; coordinate the one history event argument in `ui/src/lib/events.svelte.ts` with root before touching it.
- Modify `crates/otto-ssh/src/sftp.rs`, `crates/otto-connections/src/transfers.rs`: exact-file probe, bounded capture and backoff, plus fixtures.
- Modify `docs/contracts/api.md`, `docs/features/api-client.md`, `docs/features/database-explorer.md`, `docs/features/connections-ssh-sftp.md` alongside behavior changes.

## Task1: Lock in cross-key isolation and lifecycle cache semantics

**Files:** New `resource_cache.rs`, module registration in `lib.rs`; tests in the new module.

- [ ] Read existing cache acquisition/close implementations and local test patterns. Record existing native driver cleanup semantics; do not assume dropping a cloned pool equals closing it.
- [ ] Add tests with oneshot-controlled fake connectors: warm B returns while A remains blocked; same-key A calls initialize once; dropped initiating caller lets a waiter retry; failed initialization can retry.
- [ ] Add retirement cases: remove during blocked initialization never returns/publishes a late resource; close B does not wait for A; retired waiter fails rather than looking up a fresh slot; value produced in completion/cancellation race is disposed. Track fake resource drops/close calls with counters.
- [ ] Run RED: `CARGO_BUILD_JOBS=2 cargo test -p otto-dbviewer --lib resource_cache -- --nocapture`.
- [ ] Implement `ResourceCache<K,V>` with map transitions containing no network await. Slot owns an async initializer guard and latched retirement signal. Map operations select or detach slots; initialization races retirement and rechecks before publication. Initialization lives in caller future, not an orphan task.
- [ ] Add `get_ready` for cleanup callers, which never inserts a slot or initializes a resource. Detached ready resources are returned to drivers for their correct close/shutdown outside the map lock.
- [ ] Keep failed empty slots from accumulating when unreferenced; test repeated failed keys do not grow registry without bound. Do not add arbitrary eviction of successful active pools.
- [ ] Run the same command GREEN; assert ordering and counts, not timing thresholds.

Core invariants to implement (pseudocode):

```text
acquire(key, request_lifecycle):
    request_lifecycle.check_active()
    slot = map.lookup_or_insert(key)             # synchronous short transition
    select retirement(request_lifecycle, slot) / lock(slot.initializer)
    check both retirement signals
    if ready and usable: return ready.clone()
    value = select retirement / initializer()
    under slot state transition: publish only while both tokens active
    otherwise dispose value and return connection_closed
remove(key): detach and retire slot under map transition; close ready outside it
get_ready(key): clone ready only; never insert, wait for initialization or initialize
```

## Task2: Integrate driver caches without changing keys or cancellation behavior

**Files:** Five driver modules above; `types.rs`, `config.rs` and other explicit `ResolvedConfig` literal sites.

- [ ] Add lifecycle token as an internal optional field on `ResolvedConfig`; construction outside the service uses an unattached/active token policy rather than serialized params. Preserve redacted Debug. Update all compile-time struct literal sites deliberately (`rg -n 'ResolvedConfig \\{' crates`).
- [ ] Add at least one driver-level connector fixture that exercises its actual acquisition function: blocked config A must not block warm config B. Prefer a narrowly injectable connector future over bypassing production cache call sites.
- [ ] Add Redis logical-db isolation/predicate-close tests, native ClickHouse dead-client replacement, and changed credential/params/access-scope key reuse regressions.
- [ ] Run RED: `CARGO_BUILD_JOBS=2 cargo test -p otto-dbviewer --lib cache -- --nocapture` (new tests contain `cache` in their names).
- [ ] Convert MySQL/Postgres pools, Redis managers, Mongo clients and native ClickHouse clients to keyed slots, retaining their existing key derivation and resource disposal.
- [ ] Check lifecycle before slot lookup, across initializer wait and before return; keep the same token for all acquisitions belonging to one resolved operation.
- [ ] Preserve close-native-cancel ordering: MySQL/Postgres cancellation borrows a ready pool through `get_ready`; missing pool means best-effort no-op. ClickHouse cancellation uses an existing HTTP client. This cleanup path must not create a new resource or clear a lifecycle token to obtain general acquisition authority.
- [ ] Run GREEN focused cache tests and `CARGO_BUILD_JOBS=2 cargo test -p otto-dbviewer --lib`.

## Task3: Fence the full service lifecycle and isolate tunnel initialization

**Files:** `service.rs`, optional `service/lifecycle_tests.rs`, relevant `service/changes.rs` call sites, `resource_cache.rs` token support.

- [ ] Add deterministic lifecycle/service tests before wiring behavior. Required seam: resolve completes; verification gate blocks before acquisition; close completes; release gate; connector count remains zero and no active/cache key is recreated. Then a new request acquires a fresh generation successfully.
- [ ] Add a query task queued before close but first polled after close, a close during tunnel initialization, close during driver initialization, and close-caller cancellation while the first native-cancel future is blocked. For that last case, assert the independent cleanup continues, all captured old ready pools/tunnel ownership are eventually disposed, closing ends, and a fresh generation stays usable/cached after old completion. Also test native-cancel/shutdown phase deadlines and concurrent close coalescing.
- [ ] Add cancel-order fixture: ready pool is usable by native cancellation after admission retirement but before teardown; a missing pool is never initialized by cancellation. Requests arriving while close drains return a closed error; they cannot become new entries drained by the old close.
- [ ] Run RED: `CARGO_BUILD_JOBS=2 cargo test -p otto-dbviewer --lib lifecycle -- --nocapture`.
- [ ] Implement short-held connection lifecycle registry `{generation, token, closing}`. Capture token before resolve awaits. Register active keys atomically with retirement check using the same registry transition. Close retires and detaches only old-generation ownership before any await, then transfers the captured resources/tunnel leases to one owned cleanup task. A caller awaits shared completion; caller abort cannot abandon the task. Concurrent closes join it. A task-owned finalizer updates closing state only for its own generation.
- [ ] Carry token in config and add a small `Resolved::with_lifecycle` operation wrapper using a retirement-biased select. Apply across verification, browse, query, export and mutation paths, and inside detached tasks rather than merely before spawn. This preserves native cancellation of already-submitted remote work; it does not claim transactional rollback.
- [ ] Replace tunnel global awaited initialization with per-profile slots and forwarding/SSH/target fingerprint. Keep successful-use timestamps, existing TTL, dead-tunnel retry, and active Arc leases. No secret fingerprint plaintext is logged.
- [ ] During close: retire admission and detach old registry ownership in synchronous transitions; retain resources/tunnel leases in a captured cleanup bundle so native cancellation still has transport. Spawn cleanup without an intervening await. The cleanup task issues native cancellation using captured ready handles (never fresh lookup/initialization), then shuts down/disposes those captured resources outside map locks. Never run a later broad key/connection removal that could drain fresh-generation entries. Native-cancel and shutdown phases each have a five-second aggregate deadline (short injectable budgets in tests); timeout drops captured ownership using existing driver-specific immediate shutdown/RAII semantics. Finalize only the matching generation and signal all waiting close callers. Preserve superseded driver-key eviction.
- [ ] Add fake tunnel tests for A/B isolation, same-profile single-flight, fingerprint change and TTL not killing a held lease.
- [ ] Run GREEN lifecycle/cache tests, then `CARGO_BUILD_JOBS=2 cargo test -p otto-dbviewer --lib` once after this integration.
- [ ] Update Database Explorer guide with independent initialization and close semantics; do not claim a new timeout policy or measured production latency.

## Task4: Materialize API history metadata with safe legacy backfill

**Files:** New migration0132, state `api_client.rs`, core `domain.rs`.

- [ ] Add summary DTO with existing scalar fields, optional `request_id`, and explicit `{kind:string,session_id:Option<String>,via:Option<String>}` source metadata. Full entry DTO stays intact.
- [ ] Add summary repository tests reusing existing source/q/status/request filter fixtures; include same-timestamp deterministic ordering, default/max limit via route tests later, and absent/null/unknown explicit source kinds.
- [ ] Seed100 entries with512KiB response text each plus large request bodies. Assert summary serialized bytes contain no body sentinel and remain proportional to summary metadata; full detail and old list remain exact. Fixture size assertions are correctness evidence, not production benchmarks.
- [ ] Add a migration regression applying the new migration to a disposable pre-migration schema containing malformed, null and legacy JSON. Preserve exact request/response strings. Test later direct INSERT omitting summary columns and UPDATE OF request_json as old archives/imports do.
- [ ] Run RED: `CARGO_BUILD_JOBS=2 cargo test -p otto-state --lib history_summar -- --nocapture`.
- [ ] Implement append-only migration0132 adding `source_kind`, `source_session_id`, `source_via`, `request_id`. Guard JSON with `json_valid` and type-aware extraction. Missing/null kind follows old human fallback; legacy string source follows existing filtering; malformed rows get safe absent metadata and retain original bytes.
- [ ] Backfill once, then maintain identical extraction rules in AFTER INSERT/UPDATE OF request_json triggers. Updating metadata alone must not recurse. Do not copy or rebuild full body rows/table; no migration renumbering.
- [ ] Implement explicit scalar SELECT summary query and filters without selecting/parsing request_json or response_json. Keep full query untouched. Use query-plan fixture to justify any added metadata filter indexes.
- [ ] Run GREEN focused command, then `CARGO_BUILD_JOBS=2 cargo test -p otto-state --lib api_client`.
- [ ] Document that migration is a single cold pass over existing request bytes; tests verify malformed legacy data and old-format restore writer behavior.

## Task5: Expose additive history summaries with matching contracts

**Files:** Server API client route, route registration/policy, core/TS types, contracts and feature guide.

- [ ] Update contract and TypeScript DTO in lockstep: `GET /workspaces/{wid}/api-client/history/summaries`, Viewer authorization, filters identical to full history, default100/max500, metadata-only summary array, existing detail/full routes retained.
- [ ] Add route-level fixture checking metadata-only result, exact detail, cross-workspace denial, invalid/edge limits, and route policy/inventory coverage. Reuse disposable route test context; no daemon launch.
- [ ] Run RED: `CARGO_BUILD_JOBS=2 cargo test -p otto-server --lib history_summar -- --nocapture`.
- [ ] Wire route to the explicit summary repository method, retaining existing auth and source semantics. Ensure literal `summaries` route is not handled as history id. Keep old MCP history response complete.
- [ ] Run GREEN focused test plus `CARGO_BUILD_JOBS=2 cargo test -p otto-server --test policy_coverage --test route_inventory`.
- [ ] Notify root that shared route/type/doc edits are ready before integration; do not reformat unrelated sections.

## Task6: Coalesce history refresh and lazily load owned detail

**Files:** New `apiHistory.ts` coordinator and `ui/unit/apiHistory.test.ts`; history portions of API store and both history selectors; one event dispatch argument after root coordination.

- [ ] Add deferred-request unit fixtures for duplicate direct/WS signal, signal during in-flight fetch, later append, workspace switch, stale detail A vs B, tab switch, draft edit, and detail failure. Assert actual fetch counts/state changes using injected clock/scheduler, not sleeps or implementation string matching.
- [ ] Run RED: `node --test ui/unit/apiHistory.test.ts`.
- [ ] Implement refresh coordinator:150ms batching, at most one current-workspace fetch and one follow-up invalidation, captured workspace/epoch, cleanup on workspace change. Track appended entry id where supplied so matching direct/event duplicates can be satisfied by a snapshot containing that id; later absent id still triggers the bounded follow-up.
- [ ] Switch initial/history refresh to summaries; history state uses summary DTO. Preserve root-owned execute/scripts blocks and existing `loadHistory()` call site. Coordinate event dispatch argument with root before editing.
- [ ] Implement detail request selection token capturing workspace, selected id, active tab and draft revision. Abort old detail request; recheck every ownership field before calling the existing synchronous detail-to-draft loader. Request errors preserve current draft.
- [ ] Wire both HistoryList and compact ApiPanel selectors to the async detail loader with pending/error feedback. Agent/source badges now read summary metadata, not request JSON.
- [ ] Run GREEN: `node --test ui/unit/apiHistory.test.ts ui/unit/apiClientOwnership.test.ts`; then `npm --prefix ui run check`.
- [ ] Give root an E2E checklist only: initial/sidebar/compact fetch summaries with no detail; select one item fetches exactly its detail and loads original draft; delayed detail cannot overwrite an edited/new tab; duplicate send/event refresh is coalesced. Root owns authoring/running isolated browser integration if needed.

## Task7: Probe one SFTP file with bounded output

**Files:** `crates/otto-ssh/src/sftp.rs` tests and implementation.

- [ ] Add fake executable tests for exact staging-file command, regular-file size parsing, missing/ambiguous/directory results, quoted spaces/quotes/backslashes/glob/brace/leading-dash paths, and control-character rejection. Inspect OpenSSH argument handling from the accepted design’s primary source rather than assuming shell glob quoting applies unchanged.
- [ ] Add bounded-output fake cases for stdout/stderr over32KiB, concurrent draining (neither pipe blocks the other), deadline and caller cancellation while child sleeps. Track child identity inside the fixture; assertions must not inspect/kill unrelated processes.
- [ ] Run RED: `CARGO_BUILD_JOBS=2 cargo test -p otto-ssh --lib file_size -- --nocapture`.
- [ ] Implement `file_size(full_path)` issuing numeric long listing of one literal file, never parent listing. Require exactly one regular-file metadata record. Returned name may be full path or basename. Prefix a leading-dash relative path safely; ensure brace/glob syntax cannot broaden work.
- [ ] Implement bounded probe runner with concurrently drained32KiB stdout and stderr caps. Terminate/reap owned child on overflow or deadline, and preserve kill-on-drop cancellation cleanup; use a narrowly owned cleanup task only if necessary to guarantee reaping after future drop. Do not replace all transfer/list execution semantics.
- [ ] Run GREEN focused command and `CARGO_BUILD_JOBS=2 cargo test -p otto-ssh --lib`.

## Task8: Integrate upload progress cadence without weakening authorization

**Files:** `crates/otto-connections/src/transfers.rs`, SFTP feature guide.

- [ ] Add fake transfer tests asserting progress probes receive stage path rather than parent, successful size updates bytes, and simulated10,000 siblings do not add any requested metadata records.
- [ ] Add deterministic backoff test: success cadence1s; failure delays2/4/8s capped8; success resets1s. Authorization still checked every1s while progress is backed off. Existing500ms probe deadline and1–600s copy timeout stay distinct.
- [ ] Run RED: `CARGO_BUILD_JOBS=2 cargo test -p otto-connections --lib progress -- --nocapture`.
- [ ] Replace parent list/find with bounded exact-file probe, track next-probe deadline/failure streak independently of authorization tick. Best-effort probe errors preserve last confirmed bytes.
- [ ] Preserve separately awaited final publication and outcome_unknown semantics; do not move final rename under the copy cancellation select. Cancellation only removes owned staging files.
- [ ] Run GREEN focused test then `CARGO_BUILD_JOBS=2 cargo test -p otto-connections --lib transfers`.
- [ ] Update SFTP guide: progress remains best-effort, probes one file, backs off failures; no claim that copy timeout bounds final publication.

## Task9: Independent review handoff and integration validation

- [ ] Inspect own diff for accidental shared script/send changes, sensitive values, broad formatting and migration overlap; report exact owned file list to root.
- [ ] Send RED/GREEN evidence and tests skipped to root. Distinguish synthetic payload sizes and deterministic scheduling evidence from latency measurements.
- [ ] Ask parent reviewer to trace (a) close after resolve/before acquisition and native cancel, (b) migration trigger metadata/source compatibility including archive inserts, (c) coordinator ownership and duplicate invalidation, (d) literal SFTP path/output bound/authorization cadence.
- [ ] Address concrete review findings with focused new regressions and rerun only affected suites. Root coordinates broad workspace test/Clippy/UI build/browser gates and a single PR; this worker does not independently run broad builds or publish.

**Completion criteria:** All three behaviors are usable at their actual call sites, old full API data remains available, close cannot reopen retired resources from delayed work, exact SFTP probes remain bounded, relevant regressions pass, docs/contracts/types match, and parent independent review accepts. No application deployment is included.


---

## Vault

# Vault Responsiveness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:executing-plans to implement this plan task-by-task after root explicitly opens implementation. Steps use checkbox syntax. User forbids additional agents and per-worker commits; root owns review, the single PR, CI and merge. Do not follow a skill default that conflicts with those instructions.

**Goal:** Make warm note saves and directory reads proportional to changed/displayed data, and bound automatic Markdown parsing without weakening source/history/freshness guarantees.

**Architecture:** Transactional per-file index deltas update keyed resolver, switcher and directory caches. Gate-owned cold hydration and fenced external scans share a publication boundary. Bounded off-thread file preparation enforces the existing 4 MiB content-indexing policy while computing exact source hashes by streaming.

**Tech Stack:** Rust/Tokio/sqlx/SQLite, Svelte5/TypeScript, Node unit tests and isolated Playwright fixtures.

**Reviewed design:** `/tmp/otto-perf-design-vault.md`, incorporating root hydration guidance and Git review V1/V2 from `/tmp/otto-perf-design-review-git.md`.

---

## Execution constraints and file map

Every shell/tool command must explicitly use workdir `/Users/itziklavon/.config/superpowers/worktrees/claude_ade/performance-20260913` (UI commands use its `/ui` child). No commands in the original checkout. No live Vault/daemon/provider actions, installation, restart, deployment, commits or new agents. Only temporary fixtures may mutate filesystem/SQLite. Application edits start only after independent plan review and root authorization.

Create:
- `crates/otto-vault/src/index.rs`: keyed lookup/cache deltas and per-vault publication/scan fences; pure data-structure tests.
- `crates/otto-vault/src/prepare.rs`: bounded streaming hash/body retention, parser jobs and preparation policy; pure/temp-file tests.
- `crates/otto-vault/src/performance_tests.rs`: engine-level race/work-count fixtures, included as a cfg(test) child of engine.rs.
- `crates/otto-state/migrations/0131_vault_content_indexing.sql`: reserved migration; never edit another migration.
- `ui/src/modules/vault/treeRefresh.ts`: immutable visible-branch refresh with four-worker request bound.
- `ui/unit/vaultTreeRefresh.test.ts`: deferred fake-transport tests of the real helper.
- `ui/e2e/desktop-vault-performance.spec.ts`: isolated UI integration, coordinated with root.

Modify:
- `crates/otto-vault/src/lib.rs` module registrations; `types.rs` indexing status; `store.rs` transaction-aware derived-row APIs, hydration reads and single-note queries.
- `engine.rs`: delta save/artifact publication, cached dir/resolver/switcher access, external reconciliation, cleanup; `scan.rs`: explicit completeness and change categories; `resolve.rs`: mutable idempotent path membership.
- `recovery.rs`: individual restore deltas and structural prefix fencing.
- `crates/otto-vault/tests/engine.rs`, `tests/http.rs`: public freshness/hash/history/contract coverage.
- `ui/src/modules/vault/vault.svelte.ts`, `RightPanel.svelte`, `ui/unit/vaultStore.test.ts`: tree integration, race guards and metadata-only notice.
- `ui/src/lib/api/types.ts`, `docs/contracts/api.md`, `docs/features/vault.md`: narrow Vault-only changes; notify root before shared-file edits.

Private implementation boundaries: `PreparedNote` holds bounded parsed metadata plus exact hash/signature/status; `VaultIndexes` owns path records, resolver, directory children and keyed switcher entries; `IndexDelta` is prepared before durable source replacement and applied after successful SQL commit. `ensure_indexes()` is callable only without the publication guard; `ensure_indexes_locked()` is explicitly internal if needed. No whole-cache clone on an existing-note update. Use one SQL transaction per note/delta batch; avoid retaining an entire scan's parsed note bodies in memory.

## Task 1 — Establish failing work-count and safety regressions

**Files:** create `performance_tests.rs`; modify `engine.rs`, `store.rs` only for cfg(test) counters/barriers, and existing `tests/engine.rs` fixtures.

- [ ] Add test-only per-engine counters for filesystem walks, cache hydrations/all-index loads, global link passes, body parser calls and active/peak preparation jobs. Counters must instrument real boundaries, not substitute fake indexing.
- [ ] Add `warm_save_does_not_scan_unrelated_files`: seed a temp Vault, scan/warm it, reset counters, write one existing note, assert zero walk/all-index/global-link increments and immediate note/tags/outgoing/backlink/search/history correctness. Seed 2,000 unrelated indexed notes for the regression, not production files.
- [ ] Add `warm_directory_reads_do_not_reload_all_rows`: seed 20,000 SQLite note/file records with folder structure, hydrate once, request 100 directories and assert no further all-index reads plus exact entries/counts/order.
- [ ] Run `cargo test -p otto-vault --lib performance_tests::warm_ -- --nocapture`. Expected RED: actual current scan/all-list counts violate assertions. Capture the failure output for handoff.
- [ ] Keep tests scoped to the three findings; do not alter unrelated auth, provider or backup tests to manufacture passing results.

## Task 2 — Define and migrate explicit indexing status

**Files:** migration0131, `types.rs`, `store.rs`, `performance_tests.rs`, `tests/http.rs`, Vault section in UI types/contracts.

- [ ] Add `indexing_status_round_trips` using a real migrated test pool and note metadata serialization; assert full and size_limited are distinguishable from parse_error, and older metadata JSON without the new field defaults to full.
- [ ] Run `cargo test -p otto-vault --lib performance_tests::indexing_status -- --nocapture`; expected RED before adding the field/migration (compile failure is acceptable only for the new symbol boundary).
- [ ] Add migration: `ALTER TABLE vault_notes ADD COLUMN content_index_status TEXT NOT NULL DEFAULT 'full' CHECK(content_index_status IN ('full','size_limited'));`. It changes only the rebuildable index schema, never source files.
- [ ] Add serde-compatible `ContentIndexStatus` with Full default and snake_case wire values; plumb it through NoteRow/NoteMeta row mapping and constructors. Add the optional compatible TS field and contract explanation together. Update existing literal fixture constructors without weakening their assertions.
- [ ] Add query support to select legacy rows with size >4 MiB and status full for policy reindexing even if their signatures have not changed.
- [ ] Rerun the targeted test; expected GREEN. Migration is picked up by existing sqlx migrator; do not manually enumerate or renumber migration files.

## Task 3 — Bound file preparation and move parsing off async workers

**Files:** create `prepare.rs`; modify `lib.rs`, `engine.rs`, `scan.rs`; tests in prepare.rs and performance_tests.rs.

- [ ] Add threshold/exact-hash tests for exactly 4 MiB and 4 MiB+1 bytes, normal-to-oversize-to-normal transitions, and unchanged legacy oversized rows. Assert source bytes unchanged, parse count zero for oversized, and full SHA-256 equals an independently computed fixture digest.
- [ ] Add a two-job semaphore barrier test: block two actual preparation jobs, enqueue a third, cancel one waiting caller, and assert active jobs never exceed two and permits are not released while a blocking job remains running.
- [ ] Run `cargo test -p otto-vault --lib prepare::tests -- --nocapture`; expected RED for absent preparation policy.
- [ ] Implement a 64 KiB chunked reader with before/after file signature validation. Retain at most the content ceiling plus a boundary probe; discard retained bytes when oversized but continue exact hashing. Reject non-regular/raced files conservatively. Return a retryable unstable result rather than publishing a mixed version.
- [ ] Implement globally two admitted preparation jobs per engine: acquire OwnedSemaphorePermit before spawn_blocking and move it into the closure. Parse accepted bodies inside that closure; check a cooperative cancellation/supersession flag between chunks. Cancellation does not pretend to interrupt an OS syscall.
- [ ] Produce filename-derived metadata for size-limited files, empty body-derived metadata, and explicit status. Preserve real reserved flags/hash/size/mtime. The transaction task below clears stale tags/links/FTS body on downgrade and restores them on shrink.
- [ ] Rerun preparation tests GREEN. Document peak retained body budget (roughly two accepted body ceilings plus parsed objects/chunks), not a hard RSS ceiling.

## Task 4 — Build mutable lookup indexes and safe cold hydration

**Files:** index.rs, resolve.rs, engine.rs, store.rs, performance_tests.rs.

- [ ] Add pure tests for idempotent resolver insert/remove, unique-basename ambiguity, case-only rename, directory descendant counts, reserved ordering and title/type updates that touch only their direct entries. No YAML title/alias link resolution is introduced.
- [ ] Add `cold_hydration_serializes_save`: pause hydration after its SQL snapshot, queue an API edit/create/delete, verify mutation publication waits for hydration, release it, then assert newest SQL/cache state. Add concurrent cold-reader counter assertion: one hydration.
- [ ] Run `cargo test -p otto-vault --lib index::tests -- --nocapture` and `cargo test -p otto-vault --lib performance_tests::cold_hydration -- --nocapture`; expected RED initially.
- [ ] Implement VaultIndexes with keyed path/switcher records and directory-to-direct-child maps. One content metadata delta changes one note record and parent entry; additions/removals adjust ancestor counts and basename buckets. Sort only returned siblings or maintain ordered sibling keys, not all notes.
- [ ] Implement gate-owned hydration: fast cache hit without publication gate; miss acquires gate, rechecks, loads all required rows from one consistent read transaction, constructs the cache off-thread and publishes while retaining the gate. Hydration must not call any helper that reacquires this gate.
- [ ] Mutation callers hydrate before acquiring publication guard. Under the guard, missing/invalidated cache means release, hydrate and retry before source replacement. Remove cache/fence state on unregister, respecting outstanding readers.
- [ ] Rerun index/hydration tests GREEN. Preserve fuzzy query ranking, but replace full switcher Vec rebuilding with keyed per-note updates.

## Task 5 — Commit single-file metadata/link/tag/FTS deltas atomically

**Files:** store.rs, index.rs, performance_tests.rs.

- [ ] Add `index_delta_rolls_back_all_derived_rows`: force one transaction write to fail using a test-only SQLite trigger; previous note/tags/links/FTS and cache generation must remain coherent. Add size-limited transition clearing stale body-derived rows and title-only discoverability.
- [ ] Run `cargo test -p otto-vault --lib performance_tests::index_delta -- --nocapture`; expected RED until the shared transaction exists.
- [ ] Add Store transaction helpers for note upsert, source link/tag replacement, FTS replacement (when FTS is available), attachment signature update and note/file removal. Execute repeated link/tag inserts on the same transaction, not auto-committed pool calls.
- [ ] Precompute/validate the cache delta and resolve outgoing links against the warm index. For path-set additions/removals, prepare the resulting resolver and changed incoming/unresolved destinations; update those in the same publication transaction. Existing-path content/title/alias changes do not load global links.
- [ ] After SQL commit, publish infallible keyed cache updates and generation. On unexpected publication failure invalidate the cache and mark repair required; never leave an apparently fresh partial cache or falsely acknowledge indexing success.
- [ ] Rerun transaction/size transition tests GREEN. Retain the existing behavior when SQLite lacks FTS; do not make basic Vault writes depend on optional FTS availability.

## Task 6 — Switch ordinary writes and reads to the delta path

**Files:** engine.rs, recovery.rs, tests/engine.rs, performance_tests.rs.

- [ ] Add `title_alias_edit_preserves_path_resolution`, `delta_save_preserves_conflict_and_revisions`, and `single_restore_updates_index_before_success`. Assert new title/aliases appear in switcher and tree, bare aliases still do not resolve, outgoing/backlinks update, old hash conflicts remain rejected, and exact recovery versions remain available.
- [ ] Run `cargo test -p otto-vault --lib performance_tests::delta_ -- --nocapture` and the original warm-save RED test.
- [ ] Integrate write_note: prepare submitted content off-thread; preserve mutation gate/hash/recovery checks; acquire publication gate before atomic replacement; commit history; apply transactional delta/cache; return metadata only after publication. Preserve the durable source on later index failure and schedule/mark repair.
- [ ] Integrate one-file artifact write and individual revision/trash restore with the same boundary. Structural operations remain for Task8. Do not advance the full external-scan freshness clock for a local delta.
- [ ] Replace per-note GET resolver reconstruction with the warm resolver and apply the same content parsing policy to explicitly requested raw bytes. Preserve exact full raw-note response and real hash; do not truncate the explicit read response.
- [ ] Replace dir and switcher reads with the shared indexes; rerun warm-save and warm-dir tests GREEN. Run `cargo test -p otto-vault --test engine` to verify existing public behavior.

## Task 7 — Make external scans incremental without overwriting API writes

**Files:** scan.rs, engine.rs, index.rs, prepare.rs, performance_tests.rs.

- [ ] Add `scan_cannot_overwrite_newer_api_delta` with barriers around actual scan preparation; perform edit/create/delete after the captured scan epoch, then release it. Assert no stale hash/title, resurrected note, erased addition or stale directory/resolver/switcher publication.
- [ ] Add `unchanged_scan_keeps_indexes_and_generation` and a content-only external edit test requiring zero global link passes; add external unrelated change while API saves continue (full freshness clock must not be reset by deltas).
- [ ] Run `cargo test -p otto-vault --lib performance_tests::scan_ -- --nocapture`; expected RED for stale/current amplification paths before implementation.
- [ ] Separate scan serialization from publication. Capture scan epoch and indexed signatures; enumerate/prepare outside publication gate. Do not accumulate all parsed bodies: prepare/apply bounded batches, releasing retained content promptly.
- [ ] Record API per-path generations; under publication gate skip prepared changes/removals superseded since scan start. Apply accepted deltas against the current cache. Global link pass runs only after real path additions/removals, not mtime-only modifications.
- [ ] Unchanged scan updates completed-scan time without rebuilding keyed caches or content generation. Stable successful scan advances freshness; unstable/incomplete work schedules at most one coalesced follow-up, with no busy retry loop.
- [ ] Rerun scan tests GREEN. Ensure same-path API data always wins over an earlier prepared scan snapshot.

## Task 8 — Fence structural mutations and incomplete walks

**Files:** engine.rs delete/rename/create-folder, recovery.rs restore_trash, scan.rs, index.rs, performance_tests.rs; existing rename/trash tests.

- [ ] Add `scan_folder_rename_fences_descendants`, covering old/new folder prefixes and a rewritten linking note outside the folder. Add analogous restore/delete descendant assertions.
- [ ] Add `incomplete_walk_preserves_indexed_notes` using an injectable test walker/entry error (not chmod behavior that differs under privileged runners). Add `reappeared_removal_candidate_is_retained` paused before removal validation.
- [ ] Run `cargo test -p otto-vault --lib performance_tests::scan_folder_ -- --nocapture`, `cargo test -p otto-vault --lib performance_tests::incomplete_walk -- --nocapture`, and `cargo test -p otto-vault --lib performance_tests::reappeared_ -- --nocapture`; expected RED first.
- [ ] Before structural filesystem mutations, record boundary-aware affected old/new prefix epochs and rewritten-source path epochs under publication gate; retain through any older active scan. Prefix a/ must not match ab/. Release publication gate before awaiting an explicit full scan; never recursively lock it.
- [ ] WalkResult records enumeration completeness; every read_dir/entry/required-metadata failure marks it incomplete instead of being swallowed by flatten. Conservative policy: incomplete walk may apply verified changed files but authorizes no removals, exposes error/incomplete state, and does not mark last-complete freshness.
- [ ] Re-stat prospective removals immediately before apply; only definite absence permits pruning. A reappeared file, inaccessible path or concurrent metadata change remains indexed and is retried. Keep this an explicit best-effort external-filesystem boundary, not a claim of filesystem transactions.
- [ ] Rerun these tests GREEN plus `cargo test -p otto-vault --test engine`; confirm no mutation→scan publication lock inversion.

## Task 9 — Refresh visible tree branches with stale-response ownership

**Files:** treeRefresh.ts, vaultTreeRefresh.test.ts, vault.svelte.ts, vaultStore.test.ts.

- [ ] Add deferred-transport tests: 100 loaded-but-collapsed folders produce root-only refresh; open ancestors cause only visible descendants to fetch; maximum four concurrent sibling calls; a generation during refresh produces one trailing refresh.
- [ ] Add collapsed/reopen test retaining nested expansion state, and workspace/Vault navigation test proving an old request cannot mutate even a nested node in the new tree.
- [ ] From isolated worktree `/ui`, run `node --test unit/vaultTreeRefresh.test.ts`; expected RED for absent helper/behavior.
- [ ] Implement immutable refreshVisibleTree: fetch root, merge cloned entries retaining open state, enqueue only open descendants with open ancestors, mark collapsed descendants stale, and publish only a complete ownership-matching result. Reopening a stale branch refreshes it before showing cached children as current.
- [ ] Integrate helper with store's existing overlap guard plus pending generation invalidation, workspace/Vault/sequence capture and toggleDir. Update existing VM harness to call the real helper rather than replacing it with a stub.
- [ ] Run `node --test unit/vaultTreeRefresh.test.ts unit/vaultStore.test.ts unit/vaultProperties.test.ts`; expected GREEN. Then `npm run check` to catch type/API integration issues.

## Task 10 — Complete contract/UI notices and focused end-to-end coverage

**Files:** RightPanel.svelte, docs/contracts/api.md, docs/features/vault.md, tests/http.rs, desktop-vault-performance.spec.ts, shared UI types only narrow Vault fields.

- [ ] Add HTTP metadata test for full/size_limited wire values and unchanged note raw/hash; execute `cargo test -p otto-vault --test http` RED/GREEN around the final serialization wiring.
- [ ] Render the size-limited notice independently from parse_error; explain name search and preserved source, without implying a corrupt file. Default absent status to full for older servers.
- [ ] Update docs: 4 MiB parser/body-index cap, exact streaming hash, status values, omitted tags/links/headings/aliases for oversized files, delta read-after-write, external scan completeness, unchanged directory shape/count semantics. Correct stale claims only in owned Vault sections.
- [ ] Add isolated E2E: save title/alias and observe fresh tree/metadata/recovery; collapse branch, externally create temp fixture note, rescan, reopen and see it; oversized fixture shows limitation with source unchanged. Do not drive a live daemon or real Vault.
- [ ] Notify root that E2E requires a current isolated daemon binary. Root coordinates build and slot; proposed command from isolated `/ui`: `OTTO_E2E_SLOT=perf-vault OTTO_E2E_PORT=7895 OTTO_E2E_PW_PORT=5295 npx playwright test e2e/desktop-vault-performance.spec.ts --project=desktop-browser --workers=1`. Reserve ports with root before running; this is not permission to restart an installed daemon.

## Task 11 — Focused verification and review handoff

- [ ] Run `cargo test -p otto-vault` after all Rust deltas stabilize; expected all crate unit/engine/HTTP tests pass, including explicit RED-to-GREEN evidence from above.
- [ ] Run `cargo clippy -p otto-vault --all-targets -- -D warnings`; fix only owned failures. Coordinate Cargo lock with other workers rather than launching redundant broad builds.
- [ ] From isolated `/ui`, run `node --test unit/vaultTreeRefresh.test.ts unit/vaultStore.test.ts unit/vaultProperties.test.ts` and `npm run check`. Broader UI/workspace gates are root-owned.
- [ ] Format only changed Vault Rust files with rustfmt (skip child modules when formatting engine if needed); never cargo fmt the whole workspace. Run `git diff --check` and inspect the scoped diff.
- [ ] Report tests, exact work-count reductions, verified race invariants, migration number, additive DTOs, remaining full-scan/structural/hash costs, and any unrun E2E honestly. Root conducts implementation cross-review before commit/PR.

## Completion checklist

- [ ] Warm existing-note save: zero unrelated walks/all-index queries/global incoming passes.
- [ ] Immediate metadata/tags/links/backlinks/search/switcher/tree and recovery coherence.
- [ ] One cold hydration; gate-safe API ordering; no stale scan or structural descendant resurrection.
- [ ] Incomplete walk and reappeared paths cannot be pruned.
- [ ] Warm directory requests avoid D*N materialization; collapsed branches do not fetch.
- [ ] Oversized source/hash preserved; no oversized body parser; two admitted blocking jobs maximum.
- [ ] Migration0131/contracts/UI status complete, existing public tests and focused UI tests green.
- [ ] No live data/actions, commits, deployment, installation, restart or additional agents.

## Accepted plan-review clarifications before implementation

- [ ] Structural full scans collect accepted membership additions/removals into one publication batch and reconcile global links once for that batch, never once per added note. Stream/prepare bodies sequentially or through a bounded batch; separate membership reconciliation from per-note body persistence. A fixture with 100 added targets and pre-existing unresolved links asserts one global pass, correct final resolution, and bounded preparation peaks.
- [ ] Bound waiting preparations as well as running workers: scan preparation is a sequential stream, not one future/task per file; API writes enter the existing per-Vault mutation admission before retaining/starting expensive preparation. Add a small shared preparation-admission ceiling with explicit retryable busy/cancellation semantics for cross-Vault requests; rejected work does not spawn or retain a new owned body copy. Do not clone whole submitted bodies before admission. A deferred-worker fixture exceeding admission verifies bounded running+queued work and eventual permit release, including canceled callers.

