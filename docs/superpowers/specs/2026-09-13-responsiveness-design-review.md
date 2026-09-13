# Responsiveness independent design reviews

All required amendments were accepted in the companion design document. These are pre-implementation review records, not claims of deployed behavior.

# Independent design review — Vault + shared responsiveness

Verdict: Approve with required amendments below before the implementation plan. No source changes or tests/builds performed. Reviewed the two supplied design files and relevant existing VirtualList/FolderPicker, Vault indexing, Git removal and API script call paths.

## Vault: one publication gap to close

**V1 — Cold hydration needs the same publication fence as mutations/scans (required).** The design correctly forbids stale scan snapshot swaps, but says cold hydration loads/builds off-thread then publishes under a short lock without specifying a generation fence. An old hydration can finish after an API save applied a newer delta and overwrite that delta. Choose either: hold the async publication gate through one cold hydration (only cold callers wait), or capture generation, build from a consistent DB snapshot, then publish only if generation still matches under the gate; retry otherwise. A save encountering absent cache must not race a separately publishing hydrator. Add the exact race test: block hydration after its SQL snapshot, finish an API edit/create/delete, release hydration, and verify final metadata, directory counts and resolver retain the newer state. The preferred minimal choice is gate-owned cold hydration; ordinary warm reads/writes remain unchanged.

**V2 — Spell out structural and incomplete-scan fences (required clarification).** Folder delete/rename/restore changes descendant paths. The per-path epoch/tombstone rule must cover every old/new affected descendant (or a prefix generation), not just the folder key. Preserve existing incomplete-enumeration semantics: permission/read_dir errors cannot authorize removals. Re-stat removal candidates immediately before applying them; externally reappearing files must not be pruned as absent. Add a paused-scan/folder-rename test and a partial-walk-error test. No new watcher or full filesystem transaction is needed.

Other Vault choices look sound: existing-target saves avoid incoming-link passes; real path/basename changes still reconcile ambiguity; alias semantics preserved; direct-child counts explicitly preserve descendants;4MiB threshold leaves sources unchanged and hashes via streaming; blocking permits remain owned until actual completion; parser/SQL failure is not falsely acknowledged as successful indexing. Treat the worker budget as peak retained parse bytes plus64KiB streaming chunks, not a hard process-RSS guarantee.

## Shared: finalize keyboard-safe virtualization

**S1 — Existing VirtualList cannot by itself fulfill the accessibility requirement (required design decision).** It only renders the visible index range and exposes no focus/scroll-to-index contract. Folder rows currently contain both an Open button and a separate Use button. Simply dropping those into the virtual list makes offscreen items unreachable by Tab and can remove a focused row.

Recommended concrete pattern: a keyboard-controlled ARIA grid adapter for FolderPicker, with one stable focusable grid container (`tabindex=0`), active row+action column state, and `aria-activedescendant` referencing a rendered gridcell. Native pointer Open/Use behavior stays intact; row buttons are removed from the sequential Tab order. Up/Down traverse ALL filtered rows; Left/Right selects Open versus Use where eligible; PageUp/PageDown use visible row count; Home/End choose first/last row; Enter/Space invoke the active action. Files expose Select; disallowed gitOnly Use cells cannot be activated. Tab/Shift+Tab leave the grid for other modal controls, never become trapped inside a virtual window. Provide a concise accessible label/instruction and row/column counts (`aria-rowcount`, `aria-rowindex`). The parent `..` action may stay a normal button outside the grid.

Extend VirtualList narrowly with controlled scroll-to-index and an optional pinned active row (at most one extra row). The active gridcell must stay mounted even when pointer scrolling puts it outside the viewport; container focus remains stable. On keyboard movement, scroll target into view before updating the active-descendant reference. Rows have fixed height and path-keyed identities. Filtering/path change resets scroll and active selection to a valid entry; busy/error state moves focus deliberately to Retry or filter, not document.body. Small and large lists should use the same keyboard model to avoid behavior changing at the virtualization threshold.

Tests: keyboard-only End reaches/selects the last of10k entries while DOM rows stay bounded; open nested folder and Back; action-column Use on a git repo; disallowed Use never fires; wheel away from active row retains a valid aria-activedescendant; filter removes current item without focus loss; Tab reaches Cancel; Shift+Tab returns; mobile/pointer paths remain usable. This is preferable to native tabbable buttons in a slice with unspecified focus retention.

## Shared: Git removal must fail closed for unknown probes

**S2 — `worktree_remove_checked` does not independently inspect dirtiness (required, root accepted).** Current `local.rs:880–890` merely invokes git and adds `--force` twice when force=true. Current GraphView `:1184` sends `force=w.dirty||w.locked`. Therefore unknown+locked can force-remove despite an unknown marker unless both call sites change. UI must send force=false for unknown, including locked entries; backend must reject force=true if its fresh listing's target has dirty_known=false, returning409 with Retry status guidance. Ordinary force=false removal retains Git's own refusal of unsafe/locked changes. Preserve main-worktree/path-membership checks. Test unknown+locked, unknown+client-force, status changed since UI snapshot and known-dirty explicit confirmation; assert no force subprocess is dispatched for unknown.

The per-probe/process-wide/overall budgets and task-owned permits otherwise align. Include queue-admission wait in overall deadline, and verify listing returns available results without waiting for cleanup beyond the response budget while admitted cleanup tasks retain permits.

## Shared: Worker output boundary clarification

**S3 — Enforce result budgets before postMessage (required clarification, not architecture change).** The8MiB result/log budget must be checked inside the worker before structured cloning/posting to the webview, not only after receipt. Initial request/response payloads should be size-checked before sending into the worker as well. Handle structured-clone failure and worker error/messageerror by terminating and settling exactly once; cancellation listeners/timers must be removed. Verify an infinite script that never returns and an oversized/cyclic result both leave Cancel/navigation responsive without an orphan worker. Existing5s elapsed deadline is a responsiveness boundary, not a claimed heap sandbox.

No other blocking scope or testability issue found. No need for added features or broader persistence redesign.


---

# Independent review: Agents/Workflows and Connections/API designs

**Verdict: approve the architecture after four concrete design amendments below; hold implementation planning for those seams until resolved.** No recommendation to expand beyond the 12 approved findings. Read-only source/design tracing in `/Users/itziklavon/.config/superpowers/worktrees/claude_ade/performance-20260913`; no code edits/builds/production calls. Reviewed `/tmp/otto-perf-design-agents-workflows.md` and `/tmp/otto-perf-design-connections-api.md` against current schemas, routes, cache acquisition and generic archive restoration.

## Required amendments

### 1. Connection close generation must cover driver acquisition, not just resolve

**Design location:** Connections/API A, service coordination. **Current evidence:** `crates/otto-dbviewer/src/service.rs:644–669` registers a driver key and returns Resolved; `:1279–1295` then awaits more checks before actual execution; close at `:678–720` removes the registered key/resources. Browse call sites also separate resolve from driver methods.

**Concrete sequence:** request resolves key K and passes the proposed resolve-end generation check; close advances generation and removes K; that older request subsequently invokes its driver's get_or_try_init(K), sees no slot, and creates a fresh pool. Per-slot cancellation cannot stop this because acquisition occurred after the removed slot ceased to exist. The design's stated invariant—already-started work cannot republish after close—would fail.

**Smallest amendment:** carry a close-generation/cancellation lease in ResolvedConfig/Resolved through driver acquisition and execution, and make retirement reject acquisition/publication using that stale lease. Newly started requests capture the new generation and may reconnect. Alternatively acquire an owned driver generation handle before leaving resolution, but avoid a service-wide lock around remote initialization. Make this mandatory, not “if needed.”

**Acceptance:** pause a real service-call fixture immediately after resolve but before driver initialization; close; resume; assert no pool/tunnel publication or query from the old generation, then a fresh request succeeds. Cover an already-created slot too. Generic cache tests alone cannot expose this seam.

### 2. Workflow projections need a direct-import/restore maintenance path

**Design location:** Agents/Workflows C, projection and migration. **Current evidence:** generic restore uses `state_archive/schema.rs:600–617` to insert arbitrary saved table rows directly, bypassing `WorkflowsRepo`; operational row normalization includes workflow_runs at `state_archive/schema.rs:353`. Existing runtime progress changes are in `otto-state/src/workflows.rs:878–891`.

**Concrete sequence:** restore an older valid archive without progress_json/checkpoint-summary columns after migration; no repository create/update helper runs. Or restore a newer archive whose cached projection still says running while restore normalizes the authoritative run status to canceled. Unless explicitly covered, summary polling shows missing/stale derived metadata even though detailed recovery rows are valid. Updating only repository writers plus a one-time migration does not cover future imports.

**Smallest amendment:** define projection ownership for every insert/update path. Use DB triggers where compact JSON extraction is practical, or mark projections dirty on generic insertion and perform one bounded, single-flight repair before serving summaries. Ignore/recompute imported derived values rather than trusting an imported summary over authoritative fields. Existing full nodes/checkpoint data remains exact. Lifecycle scalar fields in the response come from current authoritative columns. Ensure repair commits projection and revision consistently.

**Acceptance:** direct SQL insert equivalent to an old archive (omitted new fields), plus import with deliberately stale progress_json; progress/checkpoint summaries agree with exact detail and inert restored lifecycle without repeated full-body polling. API-history design already handles this with triggers and is a good compatibility precedent.

### 3. Do not invalidate checkpoint pagination on every unrelated run update

**Design location:** Agents/Workflows C, checkpoint cursor includes run/revision. **Current evidence:** `otto-state/src/workflows.rs:878–891` bumps run.rev for routine node progress; the design also adds checkpoint writes to this shared revision.

**Concrete sequence:** an active run has more checkpoint summaries than one page. User loads page one; an unrelated node log/progress update advances run.rev; Load more cursor is rejected and the list resets. Continuing progress can repeatedly prevent reaching later checkpoints. This is reachable without conflicting checkpoint membership changes.

**Smallest amendment:** give checkpoint pagination stable `(run_id,node_id)` live keyset semantics, or a separate checkpoint-list revision which is not changed by unrelated run progress. Define the behavior for actual list changes explicitly: dedupe refreshed identities, restart only when necessary, and never silently reuse detail from another run. Keep global run.rev for cheap progress invalidation; it need not also be the paging lifetime.

**Acceptance:** paginate more than 200 checkpoints while another node repeatedly advances run.rev; reach every original checkpoint without first-page starvation. Test actual checkpoint insert/delete separately. This is not a request for a new snapshot-storage system.

### 4. Legacy History array cannot represent candidate-budget continuation

**Design location:** Agents/Workflows B, legacy /history delegates to capped candidate engine. **Current evidence:** `routes/transcript.rs:943–1126` returns Vec<HistoryEntry>; current session arm resolves all candidate sessions, removes missing files, then truncates returned entries.

**Concrete sequence:** more than 4*limit newest candidate sessions have missing transcripts; older valid sessions exist. The new engine correctly returns an empty entries page plus a next_cursor after scanned candidates. Legacy array shape drops that cursor, so an older client receives [] and has no timestamp/identity with which to reach valid rows. Calling this identical legacy strict-before behavior is inaccurate.

**Smallest amendment:** apply bounded-candidate continuation to the new page endpoint/UI and retain legacy resolution semantics in the old compatibility endpoint (possibly sharing query/resolver primitives). Alternatively introduce an explicitly documented versioned contract rather than silently claiming compatibility. Optimizing the new UI does not require breaking the old array endpoint.

**Acceptance:** missing-candidate prefix larger than the page engine budget followed by a valid session; the new route exposes continuation and the legacy route still returns the valid entry according to its original limit/before contract.

## Additional clarification before implementation

**Passive transcript GET changes an existing side effect.** `routes/transcript.rs:440–443` currently arms a live tail, and `docs/contracts/api.md:489` promises it. The proposal correctly separates active-view touch from passive cached GET, but its compatibility statement should say explicitly whether it removes GET tail-arming globally or introduces a passive opt-out for the new UI. Returning the same JSON shape alone does not preserve this behavior. Prefer the smallest intentional contract choice, document it, and test that active ConversationView touch still starts live events while a late closed-view GET cannot rearm a tail. This is a compatibility clarification, not a reason to restore hidden-view warming.

## Designs accepted without additional scope

- Shared ResourceCache is justified by five concrete drivers plus tunnels. Map lock duration, same-key single-flight, owned ready-resource cleanup outside global lock, per-profile fingerprints, and Arc tunnel leases are appropriate. Keep explicit disposal of a connector result that wins a cancellation race; tests should use native-driver-shaped cleanup, not only drop counters.
- API history summary DTO is honest and separate from replay detail. Trigger-maintained metadata covers old archives and direct writes, and selected-detail ownership prevents stale fetches from overwriting a draft. Current query/filter parity and malformed-JSON migration cases are explicitly included.
- Targeted SFTP stat avoids parent-directory cost; bounded stdout/stderr and process cleanup are essential. Exact-path quoting and independent authorization cadence are already treated as requirements, not assumed.
- Active-view leases, identity-scoped reset, inactive-only UI eviction, cached-read authorization before lookup, and bounded cold-fold concurrency correctly address the intended costs without redesigning transcripts. Cache stamps cover replacement/truncation and sidecars; active draft/page state is not an eviction target.
- History metadata-first keyset query and resolver budget are sound for the new API. Preserve the specified provider fallback and owner/admin constraints; Unicode/literal-filter fixtures remain necessary, but no additional search subsystem is warranted.
- Workflow detail-on-expansion and persisted summary projection are the right separation. Full recovery/checkpoint bodies remain authoritative. The amendments above concern projection lifecycle and live paging, not the core approach.

Suggested handoff: resolve the four amendments in the assembled design, then put the exact race/import/pagination fixtures into the implementation plans before beginning code. No additional agents or broad redesign needed.


---

# Independent design review — shared, Agents and Workflows

Verdict: **Approve after the concrete amendments below.** No architecture rewrite is needed. Items1–4 should be resolved in the design/plan before implementation; item5 is a small budget clarification. Root has already accepted and forwarded the first three themes. Read-only review; no source edits, builds or live actions.

## 1. Preserve legacy History completeness (required compatibility amendment)

Location: `/tmp/otto-perf-design-agents-workflows.md`, History Compatible API paragraph (line52 in reviewed version).

The legacy array route cannot both delegate to a4×limit candidate budget and preserve its prior completeness. With limit100,400 missing session transcript paths followed by valid sessions can return `[]`; legacy clients have no next_cursor and infer end-of-history. The new page envelope correctly represents empty-but-continuing pages, but that does not repair the array contract.

Minimal resolution: leave the existing legacy `/history` completeness/strict-before implementation intact, or use an explicit compatibility execution mode without the candidate-budget truncation. Move first-party UI to `/history/page`, where bounded scanned candidates and continuation are explicit. Do not claim both routes have identical cost bounds. Add a fixture with more than4×limit invalid candidates before a valid row: new page returns continuation, legacy route still returns the valid row.

## 2. Keep checkpoint pagination usable while a run changes (required amendment)

Location: same design, workflow checkpoint list route (line73).

A cursor bound to global workflow run rev is invalidated by unrelated node/log/progress writes, not just insertion/deletion of checkpoint identities. Users may repeatedly lose the second page of an actively updating run. This can reproduce the original responsiveness problem as repeated first-page reloads even though each response is small.

Minimal resolution: keyset-page by node_id scoped to run plus a reset/attempt generation, independent of normal progress rev. Upsert returned summaries by identity; ordinary status/log changes update their detail revision. Retry/reset increments the collection generation and explicitly resets the list. New keys earlier than an existing cursor can be picked up by a deliberate first-page refresh on structural count/generation changes, without invalidating on every heartbeat. Test advancing past page1 while unrelated run rev changes, and explicit reset invalidating an old cursor. A summary-specific collection revision is also viable if it advances only when ordering/membership changes.

## 3. Read detail and its revision from one snapshot (required safety amendment)

Location: same design, workflow detail endpoints and stale-result rule (lines74–78).

Writes are planned atomically, but the read algorithm is not specified. Reading body, then reading a newer revision separately can label an old body as current. The UI accepts it and may never refetch until another update. Conversely progress rev and projection must correspond to each other.

Minimal resolution: select checkpoint_json plus its detail version and parent identity in one joined SELECT/transactional snapshot; select nodes_json and run rev together for cold node-detail reads. Return a revision guaranteed to describe those exact bytes. Authorization can be a separate fresh gate, but payload/version consistency cannot rely on successive independent reads. Add a controlled update between would-be reads and assert the endpoint returns either the old matched pair or the new matched pair, never old body/new version. UI detail cache should key by per-detail version, not invalidate every open body for unrelated run revisions.

## 4. Bound cold-fold admission as well as running folds (required queue amendment)

Location: same design, Server read cache paragraph (line24).

The two-fold semaphore bounds executing blocking work, but does not by itself bound queued HTTP waiters or initializing-key slots. Many distinct historical requests can accumulate outside the32-entry ready cache while awaiting permits. Cancellation cleanup is described well, but the admission policy and overload response are missing.

Minimal resolution: use try-acquire for the two cold-fold permits and return an explicit retryable busy response, or choose a small fixed pending bound and queue deadline. Coalesce same-key waiters before admission, retain permits in the real blocking task, and remove cancelled/failed initializing slots. Cached hits bypass admission. Document that128MiB is retained-cache accounting, not a hard limit on the two cold folds' transient input/parse memory; oversized entries being served without retention preserves existing cold-read semantics. Test saturation, no growth beyond the chosen pending cap, and cached-key responsiveness under saturated folds.

## 5. Put Worker payload checks before structured-clone boundaries (small shared amendment)

Location: `/tmp/otto-perf-design-shared.md`, Interactive API scripts.

The5s elapsed timer and8MiB returned-payload budget are sound, but the text should explicitly require output validation **inside the Worker before postMessage**, not only after the main thread receives it. Define an input admission budget/check before posting a large request/body/variable graph; otherwise startup can synchronously clone a huge payload before the timer can run. No exact JS-heap guarantee is implied or required.

Minimal resolution: set/document the input ceiling (or use transferables for large approved buffers), reject oversize with an actionable script-budget error before posting, and check returned serialized text/entry budgets in the Worker. Main thread still validates shape/current identity. Terminate on postMessage/DataCloneError as well as constructor/runtime errors so no worker is orphaned. Add input/output-size and clone-error fixtures; keep un-scripted HTTP body behavior independent.

## Accepted safeguards

- Worker execution identity spans pre-script/HTTP/post-script; state precedes pre-script; cancellation/late results cannot publish to a replacement tab. No silent synchronous fallback. Security/heap limitations are explicit.
- Folder browse uses try-acquire, semaphore permit owned by the blocking closure, and cooperative cancellation between steps. It honestly does not claim to stop a blocked syscall; client Cancel and admission remain meaningful.
- Git probes have per-probe and overall budgets, no one-task-per-worktree fan-out, and permits survive detached cleanup. Unknown dirtiness does not authorize destructive removal.
- Transcript cache checks authorization before reuse, validates pre/post stamps and subagent inputs, preserves drafts and mounted older pages, and does not resume from a passive GET. Active-view leases and late-read guards address the reported hidden-session resume path.
- Workflow summaries are additive; authoritative bodies/recovery state remain intact. Checkpoint-only writes increment progress revision atomically, detail is lazy, body caches are bounded, and unchanged responses avoid shared304 behavior changes.

No claim that any proposed behavior is implemented, measured on production, deployed or fixed. This review does not duplicate the implementation review that must follow the plan and code stages.
