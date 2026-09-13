# Otto responsiveness design

Base: `f2b885b016bd49fb776658a07a3f595e50e98962` (merged feature PR #45). Scope: all 12 findings in [the performance report](../../reports/2026-09-13-performance-review.md).

## Goal and delivery

Keep ordinary navigation and editing responsive when a remote endpoint stalls or local histories and Vaults grow. Preserve exact stored data, existing external API compatibility, recovery semantics and authorization. Optimize observable amplification rather than microbenchmarks. One reviewed PR, green required GitHub Actions, then admin merge. No installation, deployment, daemon restart, real database queries, or provider launches.

Work occurs in the isolated `perf/responsiveness-20260913` worktree. Three parallel implementation lanes plus root stay below the user's five-agent maximum. Design review precedes plan review; source implementation starts only after both gates. The user's instruction to proceed without questions supplies authorization for routine design choices and PR/merge operations.

## Ownership and integration

- Root: dedicated script workers, shared folder listing/picker, bounded Git worktree probes.
- Agents/workflows lane: visible transcript leases/cache, metadata History pagination, workflow progress/detail separation; migrations 0129 and 0130.
- Connections/API lane: per-key resource initialization, API history summaries, exact SFTP probes; migration 0132.
- Vault lane: incremental note indexing, cached directory children/visible refresh, bounded oversized-file preparation; migration 0131.

All public DTO/route changes update authoritative contracts and TypeScript together. Migrations are append-only. Shared files receive narrow edits coordinated by ownership; root owns script execution regions and API lane owns history regions of the API store. No whole-file overwrite of another lane's changes.

## Acceptance and limits

Acceptance uses deterministic work counts, payload contents, lifecycle races, and isolated regression fixtures. Timing budgets are behavior, not a claim about production latency. Blocking syscalls cannot be forcibly interrupted: permits remain attached to actual work after request cancellation. Large explicit detail reads remain possible; background/summary paths avoid them. Full scans and structural Vault changes may remain proportional to total content. Worker termination protects UI responsiveness but is not a security sandbox or an absolute heap cap.


---

## Shared interactions

## Shared responsiveness design (root-owned)

### Interactive API scripts

Reuse the existing pm runtime inside a new dedicated module Worker. `scripts.ts` remains the pure synchronous pm implementation used by that Worker and unit fixtures; UI callers use a new asynchronous `scriptRunner.ts` wrapper. Each invocation owns one Worker and AbortSignal, terminates in all terminal paths, and has a 5-second elapsed deadline. Initial constants: 1,000 log/test entries, 256 KiB aggregate logs/test text, and 8 MiB input and returned request/variable/result payload. Validate input size before posting into the Worker, and validate output size INSIDE the Worker before postMessage/structured cloning. Unscripted HTTP body behavior remains unchanged. Count log/test output before retaining it, throw a clear budget error, and verify returned structured data before publishing. Handle postMessage/DataCloneError and messageerror with the same exactly-once cleanup. Worker termination is a responsiveness boundary, not a security sandbox or hard JS-heap limit. pm request/header/environment/test behavior remains compatible; window/DOM globals are not part of pm and are unavailable in a Worker. No silent main-thread fallback if Worker construction fails.

Create the execution controller and visible sending state BEFORE pre-request work. One execution identity owns pre-script, HTTP, post-script and final cleanup. Cancel, workspace disposal, replacement send, or owning-tab closure terminate scripts and abort HTTP. Late worker/results must never mutate the new workspace/tab or its variables. Variable/request mutations return as structured values and publish only after successful, current-identity completion; runtime errors retain the documented error result without accidentally sending a partial request. API-agent history work is limited to history methods, avoiding root's execute() region.

Validation: retain existing pm behavior tests; actual browser Worker finite-heavy and infinite-loop fixtures remain navigable and cancellable; timeout/output-burst/error/cancellation race/worker-construction failure tests; cancelled pre-script never dispatches HTTP, completed scripts still mutate request/vars and evaluate tests. Tests use only isolated mock routes, never live endpoints. Update API feature guide with budgets and Worker environment semantics.

### Folder browse

Split `/fs/browse` into an async admission/deadline wrapper and the existing synchronous authorized browse implementation. Preserve canonical-path authorization/deny-list behavior inside the blocking closure. Use a process-wide semaphore with four permits, try-acquire (no unbounded queue), and move the permit INTO spawn_blocking so a timed-out or disconnected request cannot release capacity while a syscall still runs. Ten-second server deadline returns a clear existing Upstream problem (502); saturated admission returns a clear existing Conflict problem (409). Do not globally change the shared HTTP timeout or mutation semantics.

A request-owned cooperative cancellation flag is checked between directory entries and major filesystem steps, and set when the request ends. It does not pretend to interrupt an in-flight filesystem syscall. Runtime workers never perform browse filesystem operations; stale blocked operations retain the permit until the real work exits. Directory enumeration remains complete for the existing client-side Filter; do not silently truncate entries or change search scope. Bound active listings, not user data. Cache lowercase sort keys once per entry.

FolderPicker uses an AbortController per load, a 12-second client deadline, abort-on-close/replacement, and existing generation checks. Timeout displays an actionable error/Retry and re-enables navigation; explicit Cancel always closes. Preserve history/crumbs/Favorites/Recents, selection checks and file/gitOnly behavior. Render large listings with the existing VirtualList primitive (fixed row heights and viewport cap), preserving complete client-side filtering and resetting scroll position when filter/path changes. Small lists can keep the current markup if simpler for keyboard behavior. Use one keyboard model for small and large lists: an ARIA grid adapter with a stable focusable container, active row/action column and aria-activedescendant pointing to a rendered gridcell. Pointer Open/Use buttons remain but are removed from sequential Tab order. Arrow Up/Down traverses all filtered rows, Left/Right selects Open/Use where eligible, Page keys move by viewport rows, Home/End select endpoints and Enter/Space activates. Tab leaves for the next modal control; the parent action can stay outside the grid. Extend VirtualList narrowly with scroll-to-index and one pinned active row so pointer scrolling cannot unmount the active descendant; row/cell identities use paths, fixed heights, explicit row counts/indexes. Reset active position and scroll on path/filter changes, preserve focus when transitioning to Retry/filter, and test last-row keyboard selection plus pointer scrolling and gitOnly restrictions.

Validation: fake blocking browse work proves unrelated async tasks respond and admission remains bounded after cancellation; cancellation permits release only after worker exit. Temp directory fixture confirms exact complete sorted results/auth boundaries; browser delayed request proves Cancel/Retry and navigation, large-directory fixture proves bounded DOM plus filtering/selection at end, mobile viewport and shortcut/history regressions.

### Git worktree dirtiness

Retain existing `dirty: bool` for compatible consumers, add `dirty_known: bool` (false for failed, skipped, timed-out probes). UI presents unknown explicitly; unknown never authorizes force removal. The HTTP remove route rejects a forced removal when its fresh listing has `dirty_known=false` (409 with Retry guidance), including a locked worktree. A normal force=false removal lets Git enforce its existing refusal. The checked helper itself does not independently test dirtiness when force is true; tests must cover the HTTP gate and UI force calculation.

Keep bounded `git worktree list` command. Probe non-prunable entries through the shared LocalGit LocalRead runner with a short per-probe budget (3 seconds) and at most four probes per listing; shared process-wide admission bounds total probes to eight across listings. Overall optional-probe phase budget is ten seconds, after which unprobed/pending entries return unknown. List ordering is preserved. Admission/wait consumes the overall budget; do not spawn one task for every worktree up front. Each admitted probe task retains its permit and runs to the shared runner's bounded cleanup even if the caller leaves, ensuring descendant cleanup is not skipped by outer cancellation. No shared Git mutation locks, no writes to real worktrees. Reuse helper timeout configuration hooks for short deterministic tests.

Update core API WorktreeInfo, its parser constructors, TS types, contracts and GraphView's hint/removal handling together. A clean/dirty known result stays identical; prunable/missing/busy/timed-out checks are unknown. Timeout messages must not claim unknown means clean.

Validation: real temp repositories produce ordered clean/dirty results; fake pending/sleeping probes prove concurrency and overall bounds/unknown results; process-shim test confirms timed-out helper cleanup. UI fixture shows unknown status and never submits force removal solely from an unknown result.

### Integration and validation refinements

Worker deadlines and Git/folder limits use injectable helper configuration only in tests; production defaults are constants. Exact UI execution ownership includes current controller, workspace and initiating tab; pending script responses from a replaced execution are ignored even when workspace/tab IDs match. Read-only UI tests use an isolated test daemon built in this worktree with explicit temporary state/ports and disabled providers; test orchestration must never sweep or stop unrelated processes.


---

## Agents and workflows

# Agents + Workflows responsiveness — design for review

Worktree: `/Users/itziklavon/.config/superpowers/worktrees/claude_ade/performance-20260913`, base `f2b885b0`. Design only; no source edits/builds/commits. Root owns the design/plan review gates. User authorized this scope and no further questions are needed.

## Scope and approach

Implement the three assigned report findings: active-view resync (including whole-transcript amplification), page-before-resolve History, and lightweight workflow/checkpoint progress with lazy bodies. Preserve drafts, older pages in mounted conversations, auth scopes, exact detailed run recovery data, and legacy HTTP consumers. No provider changes, deployment or installed-state operations.

Alternatives considered:
1. Only remove global resync and add request timers. Smallest patch, but leaves repeated full-fold work, History N+1 and large checkpoint transfers. Insufficient.
2. **Recommended:** explicit view lifecycle + bounded snapshot cache, SQL keyset history pages, persisted workflow summary projection + lazy detail. Uses existing stores/repositories and additive APIs; makes work proportional to displayed state.
3. Full event-sourced transcript/history/workflow rewrite. Could remove more work but imposes new persistence/recovery semantics unrelated to the reported failures. Reject.

## A. Active-view resync and transcript cache

### UI ownership

Add `acquireView(source)` / release lease to TranscriptStore, called by ConversationView's source-keyed mount effect. Multiple visible panes for one source share one conversation and one request. Lease ownership is window-local: each Otto webview independently represents its own mounted panes. A mounted pane only becomes an active network consumer while its document is visible. Returning visibility resyncs that window's active leases; a different window's visible pane continues normally. Do not use focus/blur as visibility: an unfocused but visible split window is still a legitimate viewer.

`resyncAll` becomes `resyncVisible`: only current active leases, maximum 2 requests in flight, single-flight per source plus at most one trailing request. Source changes, unmount and hiding cancel queued work and abort in-flight reads; generation/lease checks after every await prevent a completed old read from touching/resuming a closed view. `touch` for active visible chats retains intentional resume parity. Remove shell-wide periodic `touchWorkspace`/focus warming from the normal UI (keep the endpoint for compatibility). The initial SessionView `conversation`/`ensure` effect is unused except as an eager probe; remove it so terminal-only tabs do not fold/cache transcripts before Chat is opened. ConversationView performs first load when actually mounted.

Keep drafts in the existing separate draft/sessionStorage subsystem; releasing a view never clears its draft, pending-send lock, view preference or scroll/page state. Closed transcripts become inactive cache entries: LRU of 24 inactive conversations and 32 MiB estimated retained text/body bytes, excluding mounted entries. Evict on release/new-cache growth, not only on reconnect. The inactive quota is not allowed to discard active older pages; mounted oversized conversations remain intact until released. `forget` disposes read controllers only. Clear conversation/body caches and active leases on daemon/identity changes; drafts remain scoped by the existing daemon/user/window keys. A pending submission is not canceled by transcript-read cancellation.

### Server read cache (minimal compatible amplifier fix)

New `transcript_cache.rs` owns bounded folded snapshots: max32 entries, max128 MiB charged resident payload, idle expiry2min, and at most2 cold folds concurrently. Cache byte charging includes text/JSON buffers, structural allowance and subagent metadata; document it as a conservative accounting budget rather than exact RSS. Entries exceeding per-entry32MiB charge are served without retention. In-flight folds retain permits until blocking work actually returns even if the HTTP future is dropped. No lock on the cache map spans file I/O or folding. Per-key single-flight protects duplicate readers, with eviction/generation checks before publishing.

Key: daemon data root + canonical transcript path + provider + parent/subagent identity + fold options/schema version. Freshness: file identity (dev/inode where available), length and nanosecond mtime; Claude parent snapshots additionally validate subagent metadata stamps. A changed/truncated/replaced file, provider/sidecar change or unknown stamp invalidates. Capture stamps before and after cold fold; do not publish a cache entry if the file changed during the fold. Access/session/history-path authorization and canonical confinement always run BEFORE lookup; the cache is computation reuse, never an auth decision.

GET pages reuse an `Arc<Folded>` and copy only requested turns. Existing live tail can publish its already-created folded snapshot into this cache without another deep clone, but only with a stamp matching its consumed file prefix/end; otherwise it skips publication and the next GET cold-folds. Cached reads never arm a tail or resume a session by themselves. Current touch handler remains the explicit active-view operation. Initial cold folds may still process the full file (and initial tail creation may fold once separately); this change specifically eliminates repeated unchanged historical refolds and hidden-view fan-out without building a new on-disk transcript format.

Contracts: existing transcript GET shapes remain unchanged. No new endpoint is necessary for the removed terminal-only probe. Clarify that touch may resume, whereas passive GET does not. ServerCtx may hold `Arc<TranscriptCache>` (preferred daemon ownership) with root coordinating its constructor edits; alternatively a daemon-keyed cache in the new module must not mix test/server instances.

Races/tests: closed-before-GET-resolves must never send touch; hidden/visible/reconnect storms coalesce; two panes same source share reads; two independent window stores do not cancel each other; terminal-only session opens make zero transcript GETs; cache/draft eviction preserves draft and mounted older pages. Cache tests use temporary JSONL and counters: unchanged same-file pages fold once, two concurrent misses fold once, unrelated key remains usable, append/truncate/same-size replacement/sidecar changes invalidate, actor denial occurs even on cache hit, canceled waiter cannot release cold-fold permit early, byte/entry eviction works. Provider resume is a fake counter, never a real CLI.

Files: `ui/src/lib/stores/transcript.svelte.ts`, small testable resync/lease helper if needed, `ConversationView.svelte`, `SessionView.svelte`, `ui/src/shell/App.svelte`, `ui/src/lib/events.svelte.ts`, identity-reset hook; `crates/otto-server/src/transcript_cache.rs`, `routes/transcript.rs`, `transcript_tail.rs`, narrow ServerCtx/module registration. No transcript DB migration.

## B. History: filter and page metadata before resolving files

### Repository/query

New `HistoryRepo` (or small `history_page.rs` under otto-state) returns a unified metadata candidate stream:
- Session arm: agent sessions in requested workspace; creator constraint for non-admins; LEFT JOIN indexed transcript metadata by persisted path, falling back to deterministic `(provider, provider_session_id)` match. Derive effective provider/nested metadata consistently with the current resolver. Missing index metadata must not hide an otherwise valid session.
- On-disk arm: enabled only for workspace admin/root; globally unclaimed entries via indexed `NOT EXISTS` on sessions' transcript path/provider ID; this arm stays machine-wide as existing contract states, not widened for other roles.
- Apply provider, title/first_prompt/cwd search, exact-or-descendant cwd, status and cursor predicates BEFORE LIMIT. Escape SQL wildcard metacharacters for literal search/cwd semantics. Preserve case-insensitive behavior deliberately; add Unicode fixture coverage and choose a normalized searchable column/function if SQLite NOCASE cannot reproduce current Rust lowercase behavior.
- Stable order `(last_active_at DESC, source_key ASC)`, where source_key matches existing entry identity (session id or `path:` prefix). No duplicate rows from multiple indexed paths for one provider ID; persisted matching path wins, otherwise deterministic most-recent compatible candidate. Claimed exclusion is global, but returned sessions are still workspace/owner scoped.

Only selected candidates go through existing canonical/path resolution, in bounded blocking work; index metadata is already joined, so no per-row SQL calls or global claimed-set fetch. A page with missing files may scan further candidates, but resolution stops after `4*limit` candidates per request (limit default100/max1000, preserving existing cap). If budget exhausts before filling a page, return the continuation after the last SCANNED candidate, even if no entry was returned; clients must use next_cursor rather than entries.length to decide completion. This prevents an invalid-heavy archive from turning one page into an unbounded scan.

### Compatible API and UI

Add `GET /workspaces/{wid}/history/page?...&cursor=<opaque>&limit=100` -> `{entries: HistoryEntry[], next_cursor: string|null}`. Existing `/history?...before=<timestamp>` array API remains and delegates to the bounded candidate engine, keeping its legacy strict-before semantics; new UI moves to the page API. Cursor encodes version + timestamp + source_key + filter/owner/workspace scope digest, size capped4KiB and decoded/validated. It is not an authority token: every page redoes current role/owner checks. Role/filter/workspace changes invalidate cursor and restart listing. Timestamp ties are covered; deleting or importing a row between pages cannot cause duplicate display (UI existing identity dedupe stays).

Update HistoryStore to track next_cursor, retain stale-response guards, reset cursor on filters/workspace changes and append by stable identity. Handle empty-but-continuing pages with a bounded explicit Load more affordance; no automatic endless refetch. Drafts/conversation selection independent of metadata pagination.

Migration reserved `0129_history_page.sql`: indexes for `(workspace_id,last_active_at DESC,id)`, `(workspace_id,created_by,last_active_at DESC,id)`, claimed transcript paths, provider IDs; transcript-index provider/session/path lookup and activity/path ordering. Verify existing indexes first and avoid duplicates. Query-plan fixture asserts index-assisted scope/keyset/claim lookups; arbitrary substring q may still scan metadata within the authorized scope (honest limit), but never N filesystem resolves/SQL calls before paging.

Tests:1,000/10,000 synthetic metadata rows with resolver counter bounded by candidate budget; returned100 with bounded query count; filter match beyond first100; identical timestamps span pages losslessly; missing files and empty continuation; owner/editor/admin and cross-workspace claim exclusion; duplicate provider-ID index candidates; Unicode/literal `%`/`_` and cwd boundary; import/delete between pages; hostile/malformed cursor and changing scope. No real provider-root scans.

Files: new otto-state history query module + exports, migration0129, transcript route/query structs and route registration, core/UI DTOs, HistoryStore/HistoryPage, contracts/api.md.

## C. Workflow: summary polling and lazy checkpoint bodies

### Projection and revision

Keep full `nodes_json`, input and checkpoint_json as authoritative recovery/detail data. Migration `0130_workflow_progress.sql` adds lightweight `progress_json` to workflow_runs, and metadata summary columns to workflow_checkpoints (node/loop identity, iteration/index, kind/name, status, attempts, error preview, log count, updated_at). No input/output/log bodies in summary columns. Backfill existing rows using bounded migration iteration/JSON extraction; malformed legacy JSON must not silently make recovery unreadable—record a summary-unavailable fallback and use one cold detail repair path when requested. Use the existing run `rev` as the progress revision, bumping it atomically in the same transaction whenever checkpoints change, including retry/reset/delete. Audit all run mutation helpers (approval/proof/version/terminal/retry) so any surfaced metadata change advances revision.

Every update_run/update_run_progress writes the small node projection alongside existing nodes_json, without an extra read. Projection nodes carry only identity/status/timing/attempts, bounded error/phase preview, session IDs, log_count, has_output and detail revision/fingerprint. Checkpoint save updates its summary columns + run revision atomically. Full data remains exact and is still available for resume/retry. Index `(run_id,node_id)` already exists; add only ordering/summary indexes needed for paging.

### Additive HTTP

- `GET /workflow-runs/{id}/progress?after_rev=N` -> `{changed:false,rev}` when current revision unchanged; otherwise `{changed:true,rev,run:WorkflowRunProgress}` with lightweight run/node metadata and checkpoint counts. HTTP200 avoids requiring shared-client304 behavior changes. Query identity/workspace/revision first; enforce workspace Viewer BEFORE even returning unchanged. Do not load authoritative blob columns on unchanged or summary paths.
- `GET /workflow-runs/{id}/checkpoints?cursor=...&limit=100` -> `{rev,items:WorkflowCheckpointSummary[],next_cursor}`; stable node-key paging, max200; cursor includes run/revision so list invalidates cleanly when checkpoints change.
- `GET /workflow-runs/{id}/checkpoints/{node_id}` -> full checkpoint + revision, PK lookup, on explicit expansion only.
- `GET /workflow-runs/{id}/nodes/{node_id}` -> full NodeRunState + revision, on explicit expansion/zoom only. Initial implementation may extract that node from existing nodes_json (cold detail cost); never used by a collapsed-row poll.
- Existing full GET/run mutation responses remain compatible for external clients. UI summary/open paths must not accidentally fall back to full GET on every WS gap/terminal event.

All routes reuse run workspace access checks, feature policy and exact node membership; URL-encode node IDs (`#` in loop identities). Detail reads carry revision and UI rejects stale results if run or detail version changed during fetch. No cross-run cached body reuse.

### UI

WorkflowsPage uses progress responses for initial opening,2.5s fallback, WS gaps and terminal refresh. Poll only visible mounted run; retain existing single-flight/trailing merge. Unchanged response does no object replacement. Full-node WS events may populate an already-open detail cache, but summaries remain separate from bodies. Large-node/unknown-revision events fetch progress rather than full data.

RunSteps renders summaries; body requests and JSON/markdown construction happen only while an individual node/checkpoint is expanded (errors still auto-open once). Preserve user expansion/scroll/selection across merges and run retry. Load checkpoint summary pages while checkpoint panel is open; cache already-seen summaries by node ID. Explicitly merge/reset checkpoint summary revision/count on every changed progress snapshot, fixing the noted missing-checkpoint merge. Expanded bodies refresh only when their detail version changes; collapsed bodies may remain in a bounded per-run cache (16MiB/16 entries), never required for progress. Input-dependent actions receive the small required references in the run summary or request full run detail only when invoked. Proof/context directory and approval controls must retain current behavior.

Tests:30 checkpoints containing512KiB combined bodies produce small summary payloads, unchanged poll constant bytes and no blob decode; one expansion returns exact body and increments a single detail-read counter; checkpoint-only transition bumps rev and reaches UI; source run changes during pending request ignored; same checkpoint ID in another run cannot collide; owner/workspace denial on progress/unchanged/detail; errors auto-open once; retry resets proper checkpoints and refreshes summaries, preserving manual expansion; hidden run makes no polls. Query/mutation tests verify revisions and summary authority in one transaction.

Files: core workflows DTOs, otto-state/workflows.rs or projection child module, migration0130, server routes/workflows.rs + registrations/policy, UI api/types/workflows helpers, WorkflowsPage/RunSteps and small pure merge/detail-cache helpers, contracts/api.md.

## Review gates and integration ownership

Root reviews this design and shares affected shared-file snippets with other owners before implementation plan. Then produce task-by-task RED/GREEN plan and independent plan review; no app source changes until those gates pass. Schema allocations0129/0130 are exclusively this owner; Vault0131 remains untouched. Shared ServerCtx, API types, policy and contract changes are narrow and coordinated. Validation is isolated unit/repository/router/browser fixtures; no real provider, user dataset, installed app, deployment or restart.

Acceptance is bounded work/behavior, not arbitrary latency: one visible chat never resumes other cached sessions; warm unchanged transcript pages do not re-fold; History resolves at most its explicit candidate budget; unchanged workflow polls never read/transfer bodies; exact selected detail/drafts/recovery remain available.

## Binding review addendum (supersedes conflicting earlier text)

- Preserve the existing GET behavior that arms a file tail for a running session; arming a reader is distinct from `ensure_live` provider resume. Only current active-view touch may intentionally resume. Test first-open running chat receives subsequent appended records, while hidden/closed reconnectable sessions receive neither touch nor provider resume. Cached GET must retain this lifecycle parity.
- Two cold fold workers use `try_acquire_owned`, with no waiting queue for distinct misses. Saturation returns a documented retryable 409. Same-key work coalesces with at most eight attached waiters per key; extra waiters receive the same busy response. Cache hits bypass admission. Cancellation never returns a worker permit before its blocking closure exits. Visible UI retains existing content and uses bounded delayed retry; it never turns busy into a touch.
- Legacy `/history` retains its existing complete implementation and array semantics. Only `/history/page` uses bounded work. SQL scopes owner/workspace/status/cwd/keyset and global claimed IDs (without provider qualification). Metadata windows are normalized with exact Rust `str::to_lowercase` before path resolution when metadata is authoritative; uncertain resolver-derived provider/title/first-prompt filters are deferred until selected candidates resolve. At most `4*limit` candidates are scanned per request and resolved paths get one bounded batch index lookup, never per-row queries. This is an explicit replacement for SQL-only Unicode substring filtering. Continuation follows the last scanned candidate, including empty pages; UI must show Load more and must not announce final No results while continuation exists. Resolver-derived provider wins over conflicting session/index metadata.
- Workflow projection compatibility: add nullable derived projection/summary columns. SQL triggers invalidate those columns on authoritative body INSERT/UPDATE (including generic archive writes). Normal repository writes update authoritative body then republish its projection in the SAME transaction. Thus imported supplied derived values cannot be trusted. A missing projection invokes bounded, single-flight cold repair from authoritative JSON before returning a summary; publication compares the same captured authoritative value/revision and retries if changed. Never replace or rewrite authoritative bodies merely to repair metadata. Invalid authoritative JSON returns an explicit detail/summary error, not a fabricated empty run. Body and version are obtained from one SQL read snapshot; the version is calculated from that exact body when needed.
- Separate workflow `checkpoint_rev` (checkpoint freshness) from `checkpoint_generation` (reset/deletion/retry structure). Existing run rev also advances for checkpoint changes so conditional progress detects them. Checkpoint cursors bind `(run, generation, last immutable insertion ordinal)` rather than run rev or checkpoint rev. Each new checkpoint receives a stable monotonic ordinal in its write transaction; updates preserve it. Reset/replay/retry changes generation. Unrelated node/log updates and checkpoint updates do not restart pagination. UI keeps loaded summaries and refreshes currently displayed summary pages when checkpoint_rev changes; it does not discard all pages. New checkpoints append after older ordinals and remain discoverable via Load more. Detail versions independently refresh expanded exact bodies.
- Workflow cold repair shares a small two-worker, no-unbounded-wait admission policy; repair completion is revision fenced. Ordinary warm unchanged/changed summary reads do not decode or read body columns. Archive direct insertion and concurrent repair/update are required regressions, not optional integration follow-up.


---

## Connections and API

# Connections/API performance design contribution

Design only, 2026-09-13. Worktree: `/Users/itziklavon/.config/superpowers/worktrees/claude_ade/performance-20260913`, branch `perf/responsiveness-20260913`, base `f2b885b0`. Covers three approved findings: cross-key DB initialization blocking, full-body API history refresh, and parent-directory SFTP progress scans. Interactive JS worker belongs to root; no changes to script execution are proposed here. No builds, code edits, commits or live actions have occurred.

## A. DBviewer per-key initialization

### Choice and alternatives

Recommended: a small shared internal `ResourceCache<K,V>` with a short-held map mutex and cancellable per-key slots. This gives distinct keys independent initialization and same-key single-flight without duplicating race-sensitive machinery in five drivers and tunnels.

Alternative1: drop the map lock, initialize, then insert-if-absent. Smaller code, but simultaneous same-key requests create duplicate remote connections/tunnels and close races leave late publications. Reject.

Alternative2: one global initializer semaphore or longer timeout. Bounds load but leaves healthy cache hits coupled to unrelated endpoints. Reject.

### State and API

New `crates/otto-dbviewer/src/resource_cache.rs` (crate-private), registered in `lib.rs`. A slot contains a per-key async mutex over an optional ready resource and a latched cancellation signal (Tokio watch; no new dependency required). The global map lock is used only for slot lookup, insertion, selection and removal. No network/DNS/connect or resource shutdown is awaited under that map lock.

Operations:

- `get_or_try_init(key, is_usable, initializer)` obtains the slot, waits only on that key, rechecks cancellation/ready value, then awaits initialization racing slot cancellation. Ready values are cloned cheaply. A dead native ClickHouse client fails `is_usable` and is replaced within its slot.
- `remove(key)` and predicate-based removal atomically detach matching slots and latch cancellation before releasing the map lock. They collect owned ready values for existing driver shutdown (`Pool::close`, Mongo `shutdown`, or RAII drop), outside the global lock. Pending initialization is cancelled; it cannot publish a resource into a removed slot. Waiters on the retired slot return a stable connection-closed error instead of recreating it. A later explicit acquisition obtains a fresh slot.
- Failed or caller-aborted initialization leaves a retryable empty slot; it must not strand waiters on an unresolved notification. Idle empty slots are removed when unreferenced, so repeated failed arbitrary configurations do not create a new unbounded registry. Preserve successful cache lifetime/eviction policy; no arbitrary eviction of an active driver pool.
- Resource cleanup must cover both completion/cancellation races: a completed value visible in a slot is collected by removal; any value produced after cancellation is disposed rather than returned. Prefer synchronous map/slot state transitions plus explicit cleanup over detached fire-and-forget initialization tasks.

The first caller owns the initializer future. Dropping that request drops its pending connect and releases the per-key guard; another same-key waiter may retry. There is no orphan initializer task. Retiring one key does not wait for an unrelated key's setup or shutdown. Existing engine-native query cancellation remains before physical pool/tunnel teardown; retirement first stops admission of ordinary work without taking the transport away from cancellation.

### Driver and tunnel integration

Convert MySQL, Postgres, Redis and Mongo caches; also native ClickHouse (`drivers/clickhouse.rs:412` holds the same map lock over native connect). ClickHouse's synchronous HTTP-client constructor need not be refactored merely for uniformity. Redis keeps its full existing key including logical database and predicate-removes every `cache_key|db=...` entry on close. Keep all current configuration/authorization/TLS cache-key inputs and completion invalidation.

Tunnels retain idle TTL and live Arc leases. Use per-profile initialization slots with an explicit fingerprint of forwarding mode, SSH configuration and target host/port. A changed fingerprint retires the previous cached generation; an old initializer cannot republish after retirement. Do not compare or log plaintext credentials. Reaping selects expired cache ownership only; an in-flight query's Arc keeps its tunnel alive. Closing a profile cancels its initializing slot and drops cached tunnel ownership. Track last-used at successful acquisition, not before a potentially long connect.

Service coordination uses a full-lifecycle retirement token, not only a final resolution check. Keep a short-held per-connection lifecycle registry: `{ generation, token, closing }`; the token is a cloneable latched cancellation signal. Capture it at the start of resolution, before any awaited credential/tunnel work, and carry it in the internal `ResolvedConfig` (no serialized/API field). Tunnel and driver cache acquisitions receive that same token. The cache checks it before lookup, while awaiting its slot/initializer, and immediately before returning/publishing a value. An old caller may never obtain a new-generation token implicitly. Validate active-key registration under the same short registry transition used to retire the generation.

`close_connection` synchronously marks the current generation closing/retired before any await. New ordinary acquisitions during close return the stable connection-closed error; after close finishes, an explicit later request receives a fresh generation. This prevents new work from racing into entries that the close operation is still draining. Retiring the token cancels pending initialization and prevents stale key recreation even if close happens after `resolve()` returns but while `run()` awaits `is_enforced` or `verify_native` (`service.rs:1280–1297`). Carry the token through the actual driver operation: a small `Resolved::with_lifecycle(future)` helper checks retirement before polling work and races the operation against retirement, including inside the detached query task. Driver resource acquisition independently checks the same token before/after awaited acquisition, so a missed service wrapper cannot reopen a retired resource. Audit browse, verification, query, export and mutation paths, not only `run`. Already-issued remote queries retain existing best-effort native cancellation semantics; this change does not promise remote rollback.

Native cancellation must still run while old pools/tunnel leases are alive. After retirement, `close_connection` calls driver cancellation through an explicit existing-resource-only path: MySQL/Postgres borrow a ready cached pool without starting initialization; ClickHouse uses its existing HTTP client; missing resources make best-effort cancellation a no-op. Do not clear the token or provide a general bypass that can create/cache a new resource. Close cleanup is explicitly owned beyond the HTTP caller: under short synchronous transitions retire admission, detach old-generation active keys/cache slots, and capture their ready resources plus tunnel leases into a cleanup bundle. Pending old slots are retired and cannot publish. Spawn one close-cleanup task owning that bundle; the caller awaits its shared completion, but dropping the caller does not abort cleanup. Native cancellation receives existing ready handles from this bundle, so detaching registry ownership does not tear down transport before cancellation. The task then closes/disposes those same captured resources outside map locks; it never performs later broad `remove(conn_id)` or cache-key removal that could touch a new generation. Registry transitions happen before spawning/awaiting, with no cancellation point between retirement and transferring ownership to the task.

Bound the task’s best-effort native-cancel phase to five seconds and its explicit driver-shutdown phase to five seconds in aggregate, with injectable shorter test budgets. On deadline, stop waiting and drop only the captured ownership (driver-specific immediate shutdown/RAII as appropriate); already-issued remote work still has best-effort cancellation semantics, not a guaranteed remote rollback. A task-owned finalizer releases the old generation’s closing state only if it is still the generation being finalized. Concurrent closes join the same completion; no per-caller cleanup tasks are created. A new request after completion gets a fresh generation; old cleanup never looks that generation up. Keep the existing cache-key inputs and `active_keys` superseded-key eviction. This is a close fence for already-started work, not a ban on subsequent explicit reconnects.

Existing SSH readiness12s and driver-specific connect limits remain. The goal is latency isolation, not changing query timeouts or cancelling another authorized query. No public API/DB migration is required for this change.

### Regression cases

1. Fake initializer A waits on a oneshot; warmed key B must finish while A remains blocked. Assert by event ordering, not a fragile millisecond threshold.
2. Two callers for A invoke its initializer once and receive the same ready resource.
3. Cancel the first initializer caller; a waiting caller can retry, no deadlock/orphan task.
4. Close A during initialization, then release the fake connector: no retired resource is returned/published; its cleanup occurs; a subsequent A acquisition uses a fresh generation.
5. Close B while A is blocked; B's removal/cleanup is independent.
6. Configuration revision/fingerprint changes, Redis db isolation and scope-key separation remain intact.
7. Tunnel expiry drops cache ownership but not a held query lease; failed/dead-tunnel initialization can retry.
8. At least one driver integration fixture and tunnel/service close fixture exercise actual cache call sites, not only a generic helper.
9. Full service seam: pause after resolve returns but before driver acquisition (the verification await seam), close the connection completely, resume the stale request, and assert its connector was never invoked, no key/resource was recreated, and a later fresh request succeeds. Also cover a detached query queued before close but first polled afterward.
10. Native cancel can borrow an existing ready pool after retirement, cannot initialize a missing pool, and runs before teardown. Pause inside the first native-cancel call, abort the close caller, release/expire the cancellation phase, and assert all captured old pools/tunnel cache ownership are eventually disposed and closing ends. Then create a fresh generation and assert it remains cached/usable; completion of any old cleanup cannot remove it. Concurrent closes join one cleanup task and bounded phase deadlines prevent permanent closing state.

## B. API history summaries and lazy detail

### Choice and compatibility

Recommended: additive `GET /workspaces/{wid}/api-client/history/summaries`, same Viewer authorization and filter semantics as `/history`, returning `ApiHistorySummary[]`. Keep existing full `/history`, `/history/{id}`, MCP history behavior and retained replay data unchanged. UI switches to summaries and fetches one detail on selection.

Changing existing `/history` to silently omit required fields would break API callers; retaining that endpoint as the compatibility surface is intentional. Merely selecting `json_extract(request_json,...)` in every summary query would still parse large request bodies. Therefore materialize compact metadata once at write/backfill time.

### Persistence and migration

Reserved migration **0132_api_history_summaries.sql**, append-only. Add compact columns on `api_history`: `source_kind` (default human), `source_session_id`, `source_via`, and `request_id`. Existing scalar id/workspace/method/url/status/duration/executed_at columns supply the rest. No request/response body is copied into a second table, removed, truncated, or rebuilt.

Backfill existing rows once with `json_valid`/type guards. Missing/null `request.source.kind` follows the existing human fallback; recognized agent metadata is preserved; legacy `source:"automation_run"` remains non-agent, consistent with current filtering. Preserve nonstandard explicit scalar kinds for filter parity rather than quietly classifying them as human. Malformed legacy JSON must not abort the migration; it receives safe absent metadata and the original bytes remain unchanged. Document that this is one cold metadata extraction pass proportional to existing request bytes.

Use insert/update-of-request_json triggers to maintain these columns for **all** writers, including archive restore and old-format imported history. Avoid relying only on `ApiClientRepo::insert_history`, since restore uses direct row insertion. Triggers update metadata only; they must not recurse on metadata-only updates. New application writes may let these triggers provide the single source of truth. This keeps old backups (missing the new columns) correct without changes to the archive API. Existing archived bodies remain byte-identical. Add metadata-filter indexes only where actual query plan benefits justify them (workspace+source/time and workspace+request/time); unfiltered workspace/time index already exists.

Summary query explicitly selects scalar summary columns and applies `q/status/request_id/source` using metadata, never selecting/parsing request_json or response_json. Default100/max500, ordering `executed_at DESC,id DESC`, and authorization remain unchanged.

### DTO

`ApiHistorySummary`:

- `id`, `workspace_id`, `method`, `url`, nullable `status`, nullable `duration_ms`, `executed_at`.
- nullable `request_id`.
- `source: { kind: string, session_id: string|null, via: string|null }` (small metadata; `kind` is string to preserve legacy explicit values).

It deliberately has no `request`, `response`, `body` or headers. Domain/core and TypeScript types remain separate from `ApiHistoryEntry`; full detail is still `ApiHistoryEntry`. A summary does not pretend to contain a replayable request.

### UI ownership and coalescing

`apiClient.history` becomes summaries. Both `HistoryList.svelte` and compact `ApiPanel.svelte` request detail by id when selected, show pending/error feedback, then invoke the existing synchronous detail-to-draft loader. Abort any older detail selection and suppress results after workspace switch, different active tab or newer selection. Capture a draft ownership/revision token so a detail fetch cannot overwrite edits made after selection. Preserve current replay behavior (it creates/loads a request draft, not automatically executing it).

Centralize summary refresh in `loadHistory`/a small testable request coordinator: a150ms scheduled window, one in-flight fetch per active workspace, and at most one queued follow-up for an invalidation arriving after the fetch began. Direct Send and its WS event share this path. A successful snapshot can satisfy the duplicate direct/event signal; an event for an entry absent from the fetched page schedules the follow-up, avoiding a permanently missed append. Late workspace responses never replace current state. Initial workspace loading also uses summaries and this ownership discipline.

Do not edit root's script/send implementation; existing execute() `loadHistory()` call can stay. Confine API store edits to history state/loading/detail selection and coordinate before integration. If the event entry_id is passed into `noteHistoryAppended`, modify only that dispatch argument in `ui/src/lib/events.svelte.ts` after notifying root/Agents owner.

### Files and tests

Files: migration0132; `otto-core/src/domain.rs` (summary DTO); `otto-state/src/api_client.rs`; `otto-server/src/routes/api_client.rs` + route registration in `otto-server/src/modules.rs`; policy/contract route coverage; `ui/src/lib/api/types.ts`; history-only `apiClient.svelte.ts`; `HistoryList.svelte`, `ApiPanel.svelte`; extracted coordinator test module if useful; `docs/contracts/api.md`, `docs/features/api-client.md`.

Regressions:

1.100 history rows each with512KiB response text plus a large request body: summary JSON contains no fixture-body sentinel and stays proportional to metadata; exact `/history/{id}` detail and old `/history` remain intact.
2. Source/Agent filter, request filter, status, q escaping, stable limit/order and cross-workspace authorization match existing semantics.
3. Migration handles malformed/null/legacy JSON; bytes of request_json/response_json unchanged. Insert after migration (including omitted summary columns as an old archive would do) and request_json update maintain metadata.
4. Deferred fake fetches prove duplicate direct/event refresh signals coalesce; a genuinely later append is not lost; workspace switch discards stale results.
5. Slow detail A cannot overwrite newer detail B, another tab/workspace or a draft edited while A waits; failure leaves the current draft intact.
6. Browser fixture verifies both full sidebar and compact selector load full detail only after selection. No arbitrary millisecond performance gate; assert payload fields/size and request count.

## C. Targeted SFTP progress

Recommended: add `SftpSession::file_size(path)` using a literal exact-file long-list command and a small bounded-output runner. Preserve the existing OpenSSH control-master transport; do not introduce a second native SFTP protocol implementation for this change. Parsing transfer-progress stderr would avoid probes but depends on terminal/progress output and creates a less stable contract.

The API remains unchanged. On successful upload progress ticks, probe the **full staging-file path**, never its parent. Use numeric long output (one file), require one regular-file metadata record, and extract size without assuming the returned name is only a basename. Progress is best-effort: missing/ambiguous output leaves the last confirmed bytes unchanged. Reset successful cadence to1s; after failures/timeouts back off2,4,8 seconds (maximum8s). Authorization checks still run at1s independently of progress backoff. Preserve500ms probe timeout, copy1–600s timeout, job/session limits, cancellation, and the separately awaited final publication.

Literal-path handling is essential: spaces, quotes, backslashes, glob characters, brace syntax, leading-dash relative paths and control characters must not expand into multiple-file/directory work. OpenSSH's makeargv already escapes quoted `?`, `[` and `*`; do not naively double-escape them. Its glob call includes brace expansion, so verify brace/backslash treatment explicitly. Prefix a leading-dash relative path safely; retain control-character rejection. Primary source inspected: [OpenSSH sftp.c makeargv and do_globbed_ls](https://raw.githubusercontent.com/openssh/openssh-portable/master/sftp.c). A literal file match avoids directory enumeration; if a malicious/replaced staging path becomes a directory, cap output and reject non-file/ambiguous results instead of treating its contents as progress.

The new probe subprocess collects at most32KiB stdout and32KiB stderr, drains concurrently, and kills/reaps on excess, cancellation or deadline. This limit applies to the single-file probe only; no change to existing directory browsing/transfer output semantics. The current run helper can factor common command spawn safely, but avoid an unrelated transport rewrite. Correctly bounded output prevents an unexpected directory or hostile server reply from allocating an entire listing before the500ms deadline.

Files: `crates/otto-ssh/src/sftp.rs`; `crates/otto-connections/src/transfers.rs`; fake transport tests in those modules; `docs/features/connections-ssh-sftp.md` progress/capability wording. No migrations, routes or DTO changes.

Regressions: fake program receives exact staging-file probe (not parent); quoted/literal filenames including brace/glob characters do not broaden probes; size changes update bytes; malformed/oversized output is bounded; failing probes back off deterministically; successful probe resets cadence; authorization is still rechecked each1s; cancellation during a sleeping probe kills the fixture child promptly and cleans only the owned staging file; finalization behavior stays unchanged. A directory with10,000 conceptual siblings must not change the number of requested metadata records.

## Review handoff and implementation limits

Root assembles and independently reviews this design before planning. No implementation has started. Root owns interactive script worker and shared event/store integration; notify before editing shared files. All future commands must explicitly set the isolated worktree as workdir. Tests use temporary databases, injectable resource futures, fake executable transport and isolated browser fixtures only. No installed app rebuild/restart/deployment, real credential reads, live DB/SSH calls or provider sessions.


---

## Vault

# Vault responsiveness design

Status: proposed for root design assembly and independent review; design only. No application edits/builds/commits. Working tree: `/Users/itziklavon/.config/superpowers/worktrees/claude_ade/performance-20260913`, base f2b885b0. Scope: the three approved Vault findings in `docs/reports/2026-09-13-performance-review.md`. Apply the brainstorming design workflow under the user's explicit no-questions/no-extra-agents instructions; root coordinates review before planning.

## Outcomes and invariants

1. An existing-note autosave performs work proportional to its own bytes, tags and outgoing links, with no whole-Vault walk, all-note fetch, global incoming-link pass, or full switcher rebuild on a warm cache.
2. Directory lookup returns cached direct children; tree refresh touches visible branches and leaves collapsed branches stale until opened.
3. Background indexing never allocates or parses an oversized Markdown body; accepted bodies parse outside Tokio worker threads with bounded concurrency. The source file is preserved and the limitation is visible.
4. Preserve atomic file replacement, before/after recovery versions, optimistic hash checks, workspace authorization, and successful-save read-after-write behavior. Failure must never report index/cache publication as complete or delete the saved source file. Scans remain an explicit recovery mechanism.
5. Preserve current resolver semantics. `resolve.rs` indexes real paths and case-insensitive basenames, not YAML title/aliases. Metadata edits update tree/switcher/search without adding new alias-target semantics. Real additions/removals/case-renames must recompute ambiguous/unresolved target resolution correctly.

## Alternatives considered

A. **Recommended: per-file transactional indexing plus shared in-memory lookup indexes.** Fits the existing derived SQLite index and existing in-memory switcher, keeps `/dir` shape unchanged, and avoids a new durable directory schema. The main cost is coordinating external scans and API mutations; explicit publication locking/version fences make that reviewable.

B. Add persisted parent-directory rows and SQL triggers, while retaining existing full-scan saves. This improves directories but leaves the main autosave amplification. Maintaining parent counts across bulk imports and migrations adds schema machinery without replacing the save bottleneck.

C. Merely debounce scans longer and parallelize tree requests. Small implementation, but each save still does N+E work, hidden directories still multiply queries, and read-after-write freshness becomes weaker if success precedes indexing. Rejected.

## Components and ownership

- New focused `crates/otto-vault/src/index.rs`: per-vault in-memory resolver, switcher and directory indexes, delta preparation/apply orchestration, generation/publication coordination. Keep scanning enumeration and parsing separate from this module. Exact private structs can be finalized in the plan.
- `scan.rs`: walk/diff plus bounded file preparation; expose additions, modifications and removals separately instead of conflating them in `structure_changed`.
- `store.rs`: transaction-aware note replacement/removal and link/tag/FTS writes, loading one note/signature, initial cache hydration. Do not leave per-link auto-commit operations on the normal indexing path.
- `engine.rs`: existing public authorization and file/history boundaries; call shared delta indexing after guarded mutations and use shared indexes for directory/switcher/resolver reads.
- `resolve.rs`: idempotent insert, remove, and rename-safe basename bucket updates. No title/alias target expansion.
- `recovery.rs`: reuse delta application for a restored individual note/file; folder restores/renames may keep a full reconciliation because these are structural operations.
- `types.rs`, `ui/src/lib/api/types.ts`, `docs/contracts/api.md`: additive content-indexing status on note metadata.
- `vault.svelte.ts` plus a small extracted tree-refresh helper if needed: visible-branch refresh/invalidation and request ownership. `RightPanel.svelte`: concise metadata-only notice.
- `0131_vault_content_indexing.sql`: reserved with root; one derived-index status column, described below. `docs/features/vault.md` updated with accurate limits and freshness behavior.

## Shared indexes: bounded by current Vault contents

One per-vault cache contains path-keyed records, a ResolveIndex, path-keyed switcher records, and directory -> direct-child maps. Use keyed mutable structures; replacing one title/alias must not clone a complete Arc<Vec> or rebuild a whole HashMap. Directory entries maintain the same ordering and counts users currently see (current implementation counts descendant indexed files for directory badges; do not silently change this while optimizing). Update ancestor counts only on path additions/removals, and update the direct parent entry when title/type/reserved metadata changes. Complexity O(path depth + log sibling count), rather than O(N).

Cold cache hydration owns the async index-publication gate from its cache-presence recheck through a consistent SQLite read transaction, off-thread cache construction and final publication. Concurrent cold readers therefore share one completed hydration, and API/scan deltas cannot publish between its DB snapshot and cache publication. This gate may be held for the cold build; warm cache readers never acquire it. A cache miss may cost one whole-index build; warm operations must not. Remove cache/coordination entries when a Vault is unregistered; caches contain current files/directories and bounded active-scan bookkeeping, not a growing history.

Cold hydration must finish before a caller acquires the index-publication gate. The hydration publisher may itself take that gate; a caller holding it must never await the public hydration helper. Recheck cache presence/version under the gate before changing source bytes; if an intervening invalidation removed it, release the gate, hydrate, and retry preparation. An internal already-locked helper, if needed, is explicitly separate and must not recursively acquire the gate. Preconstruct/validate the cache delta before durable file replacement wherever practical so post-transaction publication consists only of infallible keyed updates. Unexpected publication failure invalidates the cache and schedules repair rather than leaving a apparently fresh partial cache.

Read locks only cover already-built structures and result copying; never hold a synchronous map lock across an await or filesystem call. Preserve deterministic directory ordering and switcher ranking. Fuzzy switcher queries may still scan the switcher by design, but ordinary saves update one keyed entry.

## API write and external scan publication

Keep the existing per-Vault mutation lock (despite its path-shaped key, `write_lock` currently serializes the whole Vault). Add an async per-Vault index-publication gate shared by API deltas and scan publication. Lock order: mutation gate, then publication gate; scans never acquire mutation gate. Synchronous cache locks are held only for brief reads/updates after DB work.

Prepare the bounded submitted note outside the publication critical section. Under the existing mutation guard, validate the current hash and prepare the recovery record as today. Acquire the publication gate before changing the file; atomically replace it, commit history, then commit note metadata/tags/links/FTS in one SQLite transaction. Resolve the changed note's outgoing targets against the warm path index. Publish the matching cache delta, increment generation, and return success only after these steps. A known cache-missing path is an addition, not an ordinary content edit. API artifact writes similarly update the single `vault_files` row/signature and directory/resolver entries.

Existing-target content changes replace only that note's outgoing rows and content metadata. Title/alias-only changes also update its switcher and tree entry; incoming `dst_path` values do not change because titles/aliases are not resolver keys. Adding/removing a real path changes the resolver candidate set and requires a global incoming/unresolved pass (including already-resolved basenames which may become ambiguous). Batch changed link destinations in a transaction. Structural changes are less frequent and retain O(E) reconciliation; ordinary saves do not.

External scanning remains single-flight and retains the five-second freshness policy. Enumerate and prepare changed files outside the publication gate. Capture a scan start epoch. API mutations record per-path last-change epochs, including a tombstone for deletions while a scan is active. At publication, skip candidates/removals whose path was changed by an API mutation after the scan began, so a prepared old snapshot cannot overwrite a successful save or resurrect a deleted note. Tombstones are discarded once the active scan finishes. Structural delete/rename/restore must fence every old/new affected descendant, using boundary-aware prefix generations or enumerated descendant epochs; link-rewrite source notes outside the moved folder are fenced too. The full walk explicitly records incomplete read_dir/entry/metadata failures. Incomplete enumeration cannot authorize removals; the conservative implementation skips all removal candidates for that scan, exposes an incomplete/error state, and does not advance last-complete freshness. Re-stat each removal candidate immediately before applying it and retain any file that reappeared or whose absence cannot be established. Revalidate file signatures around preparation; changed-during-read files are retried by a coalesced follow-up scan, not published as a stable new index. Apply accepted deltas to the current cache, never replace it with a stale full-cache snapshot.

A scan with no actual content/path changes does not rebuild switcher/directory indexes or advance content generation. It still updates the completed-scan timestamp. A content-only external change updates the same one-file structures used by API writes. A real path-set change invalidates/resolves incoming/unresolved links before publication. A failed index update keeps prior derived state, clears/invalidate affected cache as needed, records a scan error, and schedules repair; the source and recovery version remain intact. Do not mark `last_scan` fresh because one API file was saved: doing so would indefinitely defer unrelated external edits during continuous typing.

First startup/full cold scan is still O(N + E + changed bytes); warm polling retains O(N) metadata enumeration until a future filesystem watcher. This design removes repeated parsing and index rebuilds from unchanged scans and full enumeration from API saves; it does not claim to eliminate all external reconciliation cost.

## Directory endpoint and UI freshness

`GET /dir` request/response stays compatible. It reads one direct-child list from the shared cache, sorted as before; no all_notes/all_file_paths queries on a warm hit. A missing path behaves as currently documented. The shared index is updated on external additions/removals and API mutations, including ancestors whose child counts changed.

Refresh root plus branches whose ancestors and own node are open. For any collapsed branch, invalidate its loaded generation without fetching it; opening it fetches once and then refreshes any retained-open descendants as needed. Preserve expansion state and avoid publishing stale nested nodes after navigation. Capture workspace ID, Vault ID and refresh sequence at request start; only publish a matching tree. Keep the overlap guard and a pending-dirty flag so a generation arriving during a refresh causes one follow-up, rather than being lost or creating unbounded refreshes. Fetch independent visible siblings with a small bounded concurrency of four; do not create one request per historical loaded folder.

This is generation-based invalidation, not a new websocket/directory-event protocol. It reduces D to visible branches and removes N from each lookup. Later per-directory change events are unnecessary for the approved scope.

## Oversized Markdown policy and contract

Keep the existing advertised 4 MiB content-indexing threshold, now enforced before retaining/parsing the body. Streaming-read a file in fixed 64 KiB chunks to compute its real SHA-256 and detect a change during read; retain at most 4 MiB plus a small boundary probe for content parsing. Once oversized, discard the retained body and continue hashing only. Thus oversized indexing still performs O(S) sequential disk/hash work to preserve real hashes, but has O(64 KiB) ongoing buffer memory and no body/YAML/link parser cost. Source bytes are never truncated or rewritten.

Bound file-preparation/parser blocking jobs globally within VaultEngine to two concurrent workers, acquiring permits before spawning. Retain the permit until the blocking job really finishes. Coalesce scans and check cancellation/supersession between chunks. A blocking syscall cannot be forcibly interrupted; do not claim a timeout cancels the OS read or release a permit while the worker remains active. This keeps a slow file off Tokio workers without allowing an unbounded blocking queue. Full source size is finite input; no arbitrary Markdown file becomes an unbounded allocation.

For files <=4 MiB, parse/hash in the bounded worker and persist full metadata. For larger files, persist filename-derived title, size, mtime, real hash, reserved flag, empty body-derived fields, and `content_index_status = 'size_limited'`; clear stale outgoing links, tags and FTS body from a formerly small file. Maintain a title-only FTS row (or equivalent existing filename fallback) so the note remains discoverable by name. Incoming links can still target it by real path. If the file shrinks, parse normally and restore full indexing. Returning to full indexing updates the status.

Migration 0131 adds `vault_notes.content_index_status TEXT NOT NULL DEFAULT 'full'` with allowed values full/size_limited. Existing oversized rows with status full must be reconsidered even if mtime/size match, so the first post-migration scan removes stale full-body indexing. No destructive source changes or rewriting older migrations.

Expose the additive `NoteMeta.content_index_status: 'full' | 'size_limited'` (TS optional during compatibility if needed, older absent values mean full). `parse_error` remains strictly a parse error, not a size flag. RightPanel shows: "Content indexing skipped: this note exceeds 4 MiB. Search by file name; the original file is unchanged." Contract states headings/tags/aliases/outgoing body links are unavailable while size-limited. Individual raw-note GET remains the existing explicitly requested full-body operation; apply the same bounded parsing policy and warm resolver cache so it cannot accidentally repopulate omitted metadata or trigger a whole-Vault resolver rebuild. This work does not silently change GET to return truncated raw Markdown or weaken its hash matching.

## Test acceptance criteria (regression-first in implementation)

Use synthetic temporary Vaults/SQLite only. Prefer deterministic work counters/query spies over wall-clock thresholds. Introduce test-only counters around walks, all-index hydration, global re-resolution and parser invocation if needed; no production telemetry endpoint.

1. Warm a Vault with many unrelated notes/files/links. Save one existing note: zero additional full walks/all-note hydrations/global link passes; note metadata, title/aliases, tags, outgoing/backlinks, FTS and recovery before/after are immediately correct. A second cold-cache caller shares one hydration.
2. Title/alias edit updates switcher/tree labels; path-based links remain correct, and a bare alias that never resolved does not gain new semantics. Change outgoing targets and confirm only corresponding backlink membership changes.
3. Add a unique basename target that repairs an unresolved source; add a duplicate that makes it ambiguous; remove one duplicate that resolves it again; test case-rename/root-relative/attachment targets. Verify directory counts and both existing incoming and unresolved rows.
4. Block external scan preparation, perform a successful API edit/delete/create, release scan, and assert it cannot revert metadata/hash, resurrect deleted paths, erase the new path, or publish stale directory/switcher state. External unrelated edits still appear under continuous API saves.
5. Pause gate-owned cold hydration after its consistent DB snapshot, queue API edit/create/delete, and verify publication ordering: hydration finishes first, then the newer mutation survives in SQL, resolver, switcher and directory counts. Pause a scan across folder rename/restore (including descendants and rewritten sources); stale results cannot resurrect old names. Inject a partial-walk error and a reappeared removal candidate; indexed notes survive.
6. Inject SQL/cache preparation failure: no false successful-save response, source/history preserved, next scan repairs indexes; no mutex lock-order deadlock. Small-note writes/read-after-write/hash conflicts/recovery continue passing existing tests.
7. Synthetic 20,000-file tree: warm requests for 100 directories do not fetch all rows repeatedly. Frontend collapsed trees cause no recursive requests; reopened stale branches refresh; visible siblings respect concurrency four. Workspace/Vault change while loading cannot publish stale children.
8. Exact 4 MiB and threshold+1 bytes: parser invoked only for the former. Size-limited indexing keeps the exact full hash and original file bytes; no body links/tags/full-body FTS remain. Shrinking restores full metadata; migrating an unchanged oversized indexed row still updates it. Two-worker limit holds while a third preparation is queued before spawn.
9. End-to-end isolated UI: ordinary save updates filename/title and recovery without manual rescan; collapse/change/reopen tree shows fresh contents; metadata-only notice is truthful. Root owns broad E2E integration and shared gates.

## Limits and review focus

- Full structural changes can still require a global link pass; initial scan and external detection remain proportional to Vault size. The promise is cheap ordinary saves and repeated directory reads, not constant-time full reindexing.
- Exact source hashing remains O(S) I/O, but oversized bodies no longer create O(S) retained memory or parser work. Explicit raw-note viewing retains existing full-body semantics.
- Review the publication epoch/tombstone lifecycle and transaction/cache boundary first. Do not implement a simpler stale snapshot swap or asynchronous-success indexing that violates read-after-write.
- No provider launch, daemon restart, deployment/install, user-file migration, or new external service is involved.


## Binding design-review resolutions

Independent cross-reviews accepted the architecture with these required corrections; they supersede any earlier proposed wording in lane contributions:

1. Legacy array `/history` retains its existing completeness and strict-before behavior; only the additive `/history/page` route uses bounded candidate continuation.
2. Checkpoint paging uses a stable keyset scoped to run plus reset/attempt generation, independent of routine progress revision; detail bytes/version come from a single SQL snapshot. Workflow projections must be repaired or maintained for direct archive inserts and restore normalization, not only repository writes.
3. Cold transcript folds use bounded admission in addition to two running workers; cached hits bypass admission. Preserve and document the existing distinction between tail arming and provider resume; no late invisible-view request may resume another session.
4. Connection retirement tokens cover resolution, driver acquisition and execution, including the gap after resolve. A closed old generation cannot create a fresh slot.
5. Vault cold hydration shares the publication fence and cannot replace a newer save. Structural change fences cover descendants; incomplete enumeration never authorizes removal, and removal candidates are revalidated. Hydration never reenters the publication lock.
6. Folder virtualization uses the specified stable-focus ARIA grid and at most one pinned active row. Keyboard access covers the complete filtered listing.
7. Git HTTP force removal rejects fresh unknown dirtiness (409), including locked trees; UI never derives force from unknown status.
8. Script input is checked before posting to the Worker; output is checked inside it before cloning back. Clone/message failures receive exactly-once termination/cleanup.

Each resolution has an explicit regression in the implementation plan. Three independent reviews covered all four lanes; root accepted their material findings and owns integration verification. No app source changes preceded this gate. The plan receives its own independent review before source implementation.

## Subsequent delivery authorization

During implementation the user explicitly requested rebuilding, reinstalling and replacing the running app **after the new PR passes and merges**. This supersedes earlier deployment exclusions for the final delivery step only. Root owns that step after merge; all development and testing remain isolated and workers must not restart or deploy the app. This is current user authorization, so no repeated permission question is needed.
