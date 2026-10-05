# Iteration 4 — UX partition 2

**Verdict: Block.** One new blocker and two minor recovery findings. Provisional **7.2/10** under PLAN.md's five-dimension rubric. Source inspection and hand traces only; no tests, builds, servers, source edits or commits were performed.

Reviewed branch `fix/app-review-20261005`, HEAD `c5d4e999767fca39db497b6eb444e4eaaa04558d` (documentation commit over baseline `03f2bc3e`), on 2026-10-05. Scope: Git/workbench, database/connections, brokers, API client. Read AGENTS.md, PLAN.md, design guidelines for state/recovery/trust, correctness-2 and performance-2 reports, and the shared Claude findings/coordination notes. Only this report was written. These findings are additional to the previously assigned database confirmation race, API workspace/environment-save races, broker replay defects and workbench history pagination.

## Findings

### R4-U2-01 — Blocker: completing an import executes an unrelated, potentially destructive editor draft

**Location:** `ui/src/modules/database/ImportDialog.svelte:143`. Reachability: `ui/src/modules/database/SchemaTree.svelte:447`, `:470`; `ui/src/lib/stores/database.svelte.ts:3790`, `:3159`, `:3224`, `:3231`; unguarded write acceptance at `crates/otto-dbviewer/src/service.rs:1752`.

**Journey and hand trace:** On a writable non-production connection, leave `DELETE FROM import_target` in the active editor without running it. Right-click the target table and choose **Import into…**. That action only sets the import table and opens the dialog; it does not replace or run the editor text. Choose a CSV and press **Import**. When the stream returns `done`, the dialog reports success and closes at lines 136–140, then line 143 calls `database.runQuery()` whenever the active statement is nonempty. `runQuery` reads that live editor draft as its SQL at line 3159. This call supplies no `readOnly` option; the normal request executes writes on an unguarded connection. The imported rows can therefore be immediately deleted by a draft the person never chose to run. An older INSERT/UPDATE draft can likewise run an extra time. The same automatic call also replaces the person's current result state.

**Why it matters:** Import's visible promise is to insert the selected file into the selected table (`ImportDialog.svelte:179`). Clicking Import does not authorize executing arbitrary pending editor text. This crosses the boundary between a draft and an applied database change; it is not merely confusing success feedback. The blocker does not require a workspace/connection switch and is distinct from C4-2-02's confirmation identity race. Production write guards mitigate silent execution there; the demonstrated trigger uses a permitted writable connection.

**Repair direction:** Remove execution of the live editor draft from import completion. Refresh schema metadata and, only if automatic row refresh is needed, refresh a captured previously submitted read result through a read-only request tied to the original connection/tab. Preserve unsubmitted edits. Never infer that a statement is safe merely because it produced an earlier result.

**Regression idea:** In an isolated browser fixture, leave an unrun DELETE draft, complete a stubbed successful import, and assert no query POST occurs and the editor is unchanged. Repeat with a live draft changed from a previously submitted SELECT and with INSERT/multiple statements. If read-result refresh is retained, assert the request uses the captured SELECT, target and `read_only:true`; a context switch must not execute another tab's draft. A disposable database follow-up can confirm imported rows remain present, but no real user connection should be used.

**Existing coverage:** `ui/e2e/db-import.spec.ts` starts MySQL with a SELECT and Mongo with a find, then verifies row counts. Those happy paths conceal this trigger. This reviewer read those tests; did not execute them.

### R4-U2-02 — Minor: retrying Save request creates another copy of the collection

**Location:** `ui/src/modules/api/SaveRequestDialog.svelte:39`–`:44`. Caller: `ui/src/modules/api/RequestBuilder.svelte:1176`; failure return: `ui/src/lib/stores/apiClient.svelte.ts:1201`; collection insertion: `:978`. Same-name collections are permitted by `crates/otto-state/migrations/0014_api_client.sql:6`.

**Journey and hand trace:** Open Save request, select **New collection…**, enter a name and click Save. Collection creation succeeds and returns C1, then saving the request fails (for example, a transient request endpoint failure). `onsave` returns false and the sheet correctly stays open, but only local `collectionId` was assigned C1; persistent `target` remains `__new__`. Click Save again. The dialog creates C2 with the same name, then saves the request into C2. C1 remains empty. Every failed second stage repeats the first successful mutation, while the sheet continues to show New collection.

**Why it matters:** A reasonable recovery action changes the collection tree and creates indistinguishable destinations. A user can manually choose C1 before retrying, so this is bounded friction rather than a blocked task. It is independent of the already-owned cross-workspace store publication race.

**Repair direction:** After collection creation succeeds, adopt its ID as the dialog's selected target before attempting request persistence. Keep the completed collection step on failure and retry only the failed request step. Preserve the request/name draft.

**Regression idea:** Mount the sheet or drive a browser with one successful collection POST, first request save returning failure, second returning success. Assert one collection POST total, both request attempts use C1, the sheet remains open after failure, the selected destination shows C1, and success closes it. `desktop-api-redesign.spec.ts:78` covers saving into an existing collection; it does not exercise this partial-success recovery.

### R4-U2-03 — Minor: Git recovery Refresh clears a history-load error without retrying history

**Location:** `ui/src/modules/git/RecoveryTools.svelte:47`, `:127`, `:141`.

**Journey and hand trace:** Open Recovery history and let the reflog GET fail once. Initial mount calls status/bisect refresh and then `loadHistory(true)` at line 52; `run` catches the failure into the visible error. After connectivity recovers, click **Refresh**. Its handler calls only `refresh`, which reloads status and bisect but never calls `loadHistory`. `run` clears the error at line 37. The pane now says **No recovery history is available** despite never successfully loading it. The still-enabled **Load more** happens to retry offset zero because `more` starts true; closing/reopening also retries. Neither makes Refresh's outcome accurate.

**Why it matters:** The apparent recovery control converts a failed load into a false empty state, suggesting there are no recoverable commits. This violates the guideline that load failure and empty data remain distinct. A successful initial page with fewer than 50 rows also stays stale when Refresh is clicked, because Load more is disabled and no history refresh occurs.

**Repair direction:** Make Refresh reload the active mode's data, including `loadHistory(true)` for history, and retain a specific history failure until that request succeeds. An inline Retry can use the same loader. Preserve previously loaded entries when a reload fails.

**Regression idea:** Reject the first reflog request, allow the next, click Refresh, and assert a second `skip=0` reflog call and visible recovery rows. Also load one row, change the server history, click Refresh and assert the newer row appears without closing the sheet. Existing `desktop-git-recovery.spec.ts:31` covers branch recovery after a successful load; not this failure/retry path.

## Provisional score

Scores are review judgments over the finite matrix below, not whole-application reliability measurements. This initial score is retained; root should append later execution-calibrated results rather than overwrite it.

| UX dimension | /2 | Evidence and deduction |
|---|---:|---|
| Task completion/discovery | 1.8 | Inspected schema import entry, API save sheet, Git recovery controls and workbench save/reload paths. Existing actions are discoverable; recovery-history Refresh does not perform the advertised task. Broader journeys remain unexecuted. |
| Feedback/state clarity | 1.7 | Workbench exposes unsaved text and Retry; SFTP distinguishes load error/empty/filter-empty. Git recovery can erase a failure into a false empty state; API partial success is not reflected in the selected destination. |
| Recovery/retry | 1.6 | Two concrete repeat-action defects; both have manual workarounds. Source shows explicit retry paths for connection tests, SFTP loads and multi-run scope loading. |
| Draft/scope/trust preservation | 1.1 | Import executes arbitrary unsubmitted text, including destructive writes. Previously reported C4-2 ownership/trust blockers remain assigned elsewhere. PR drafting asks before replacing typed prose; broker replay and SFTP deletion show destinations in confirmation. |
| Executed end-to-end journeys | 1.0 | Shared verified green baseline credited per PLAN.md calibration. No current-review browser/native journey or new trigger executed; inspected test source is not an executed pass. |
| **Total** | **7.2/10** | **Provisional; the blocker prevents acceptance independently of arithmetic.** |

## Bounded coverage and omissions

- **Git:** Traced PR draft replacement, push/create phases, recovery initial load/refresh/pagination, and conflict abort/complete handlers. Positive evidence: `CreatePr.svelte:173` asks before replacement; `ConflictResolverView.svelte:153` names discarded resolutions. Inspected recovery test names/source. Did not execute provider auth, actual PR creation, branch/pull/push, conflict resolution, bisect or rebase; no new finding is asserted for those unexecuted journeys.
- **Workbench:** Read load/backup restore, save failures, close/flush, reload/keep-mine and visible Retry at `WorkbenchPage.svelte:535`. History growth is already P2-owned. No native multiwindow/offline/storage-failure, trash restore or delayed autosave journey executed; those are coverage limits, not a clean guarantee.
- **Database/connections:** Traced import initiation/completion, ordinary query submission/write guard, multi-run target-loading Retry, connection test/save and SFTP deletion/list error recovery. Did not execute MySQL/Postgres/Mongo/ClickHouse/Redis, export, approved changes, comparison, multi-run cancellation, live SSH/SFTP or schema migrations. QueryBuilder keyboard/add-join remains Claude's ownership.
- **Brokers:** Read replay destination confirmation/body/error completion plus topic/group load paths and C2's service trace. Byte/null preservation and Unicode preview are C2-owned; topic/group refresh presentation is Claude-owned. No Kafka/schema registry, partial replay, consumer reset, tail reconnect or cancel journey executed.
- **API:** Traced save sheet/collection/request steps and reviewed environment/save ownership findings, OAuth token flow guards, proto parsing and automation-recovery test inventory. No external HTTP, OAuth browser callback, SSE/WebSocket/gRPC, import/export, automation leave or workspace-switch journey executed here. C2 owns existing workspace/environment-draft races.
- **Visual/native:** No screenshots, light/dark rendering, keyboard/a11y acceptance, touch/RTL, packaged Tauri or real desktop interaction. Claude owns visual/copy/a11y work; this report asserts behavioral findings only.
