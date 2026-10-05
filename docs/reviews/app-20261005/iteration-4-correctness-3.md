# Iteration 4 correctness — partition 3

**Verdict:** Block.
**Counts:** blocker 1 · major 2 · minor 1 · nit 0 · question 0.
**Baseline:** `03f2bc3e`, branch `fix/app-review-20261005`.
**Scope:** bounded, read-only source review of Vault / Canvas / Design Hall / Product / Browser / Snip. Findings below are hand-traced, not runtime reproductions. No builds, tests, servers, or source mutations were performed.
**Intended behavior reviewed against:** BrowserStore's workspace/tab ownership comments and callers; DesignService's partial metadata patch semantics; Canvas restore and Excalidraw serialization; Vault guarded-write/recovery comments; Product owner guards; Snip persistence-drain semantics.

## Findings

### R4-C3-01 [blocker] Concurrent independent metadata patches lose an acknowledged update

**Location:** `crates/otto-design/src/service.rs:1284`, `crates/otto-design/src/service.rs:1339`; whole-row write at `crates/otto-design/src/store.rs:840`.

**What:** A PATCH merges into an unlocked artifact snapshot, then writes every mutable metadata column. Two requests modifying different fields can overwrite each other's successful changes.

**Intended:** `update_meta` applies the supplied metadata patch; an omitted title/tags/status field must retain its current value. The public Editor-gated route reaches this method directly at `crates/otto-design/src/http.rs:693`.

**Evidence:** confirmed by hand trace. Initial artifact is `{title:"Old", tags:[], status:"draft"}`. Request A patches `title:"New"`; request B patches `tags:["review"]`. Both calls finish `require_artifact` at line 1284 before either writes. A writes `{title:"New",tags:[]}` and returns success. B subsequently writes its merged snapshot `{title:"Old",tags:["review"]}` through the unconditional SQL at store line 840. Final persisted title is `Old`, despite B never specifying title and A having succeeded. The content commit publication lock at service line 689 does not wrap this metadata method. The same stale status column can undo a concurrent approval's status.

**Why it matters:** Normal concurrent windows/API callers silently lose metadata edits; reloading cannot recover the overwritten title/tags from this patch path.

**Fix:** Serialize metadata read/merge/write with the canonical per-artifact publication lock, with every other read/merge/write metadata path participating (including approval/thumbnail where relevant), or implement an atomic transaction/field-scoped patch that preserves unspecified latest columns. Merely locking the final SQL does not fix the stale read.

**Regression:** Coordinate two metadata patches at the initial read using a barrier and assert both the new title and tags remain after both return. Add approve-versus-title-patch coverage so status remains approved. Use separate DesignService instances sharing the database/root, matching the existing aliased-service commit test.

### R4-C3-02 [major] A delayed tab creation inserts workspace A's tab into workspace B

**Location:** `ui/src/lib/stores/browser.svelte.ts:112` (also `openLiveTab` at line 181).

**What:** Creation captures the workspace only in the outbound call. Its completion unconditionally changes whichever workspace the singleton now represents.

**Intended:** BrowserStore represents the current workspace's tabs/pages; `loadTabs` explicitly invalidates earlier page ownership when switching workspaces. `BrowserView.svelte:50` calls it whenever `ws.currentId` changes.

**Evidence:** confirmed by hand trace. Begin `openTab("https://a.test")` while `wsId=A`, holding `createTab(A, ...)` unresolved. Switch to B and finish `loadTabs(B)` with B's existing tab selected. Resolve the create as `{id:a, workspace_id:A, url:"https://a.test"}`. Lines 112–115 append `a` to B's tabs and select it. Line 117 starts `loadPage` using the now-current workspace B. Result: a tab owned by A is shown and mutable from B's strip, with page/annotation requests scoped to B. The event filter cannot help: this is the direct HTTP completion. `openLiveTab` has the same unconditional append/select after its two awaits.

**Fix:** Capture a workspace generation before creation and apply selection/list/page updates only if it still owns the store after every await. Creation can still return the original tab to its caller; its late completion must not start a page request for another workspace. Use a generation as well as ID to cover A→B→A.

**Regression:** Deferred create with a completed workspace switch, for reader and live creation; assert B's tabs/activeId/page remain unchanged and no A URL is fetched under B. Repeat A→B→A and selection changes during creation according to the intended foreground-selection policy.

### R4-C3-03 [major] Navigation persists the other selected tab's title

**Location:** `ui/src/lib/stores/browser.svelte.ts:240`–243.

**What:** After fetching the navigation target, `navigate` derives the title from the global visible page. That page can now belong to a different tab because the fetch ownership guard correctly rejected the older result.

**Intended:** The comment above `navigate` says the tab adopts the fetched page's title; the title must correspond to the URL being persisted for that tab.

**Evidence:** confirmed by hand trace. Tab A is selected. Start `navigate("https://new-a.test")`, hold its page request. Select tab B and complete its page request with title `B report`. Resolve A's page request. `loadPage` sees a newer `pageSeq` and returns without replacing B's page. The waiting `navigate` still executes lines 241–243 and PATCHes tab A with `{url:"https://new-a.test", title:"B report"}`. Wrong title is persisted server-side, not merely displayed transiently. The same problem exists after a workspace switch.

**Fix:** Return/use the request-local fetched result when updating that tab's metadata, independently of whether the result may be displayed. Do not read shared `this.page` after awaiting a request. Preserve newest-navigation ownership per tab so an older navigation cannot subsequently overwrite its newer URL/title.

**Regression:** Defer A's navigation, select/load B, resolve A, and assert A's PATCH title is the fetched A title while B stays displayed. Add two navigations on the same tab resolved in reverse order.

### R4-C3-04 [minor] Restoring an Excalidraw version fails to restore its saved background

**Location:** `ui/src/modules/canvas/ExcalidrawCanvas.svelte:207`; persisted app state at line 277.

**What:** Live source loads restore elements and files but discard the saved app state. The next edit saves the old editor background over the restored version.

**Intended:** `ConversationPanel.svelte:106` restores a prior canvas version; `CanvasStore.restoreVersion` applies the returned source. Excalidraw snapshots explicitly include `viewBackgroundColor` and `gridSize` at line 277.

**Evidence:** confirmed by hand trace. The open editor has a red background. Restore a version whose JSON source contains the same elements and `appState.viewBackgroundColor:"#ffffff"`. Store `restoreVersion` changes `canvas.source`; the source effect calls `loadScene`, parses that app state, but line 207 only invokes `ex.updateScene({elements})`. Background remains red. A subsequent element edit reaches `snapshotDoc`, reads `getAppState()` at line 268, and persists red again. Grid state is similarly omitted from live loads, and initialData at line 372 supplies only background, despite gridSize being serialized.

**Fix:** Apply the supported persisted app-state subset on source loads and initialData, including background/gridSize with explicit defaults. Keep transient editor state out of restored content.

**Regression:** Mount/load a board with one background/grid setting, apply restored source with different settings, assert editor state changes, then make an element edit and verify the next serialized source retains the restored settings.

## Coverage and limits

Inspected source (whole store files where stated; named function windows elsewhere):

- `ui/src/lib/stores/browser.svelte.ts` (whole): list/open/select/close/navigate, cache, annotation ownership, summary/event mutations.
- `ui/src/lib/api/browser.ts` (whole) and `ui/src/modules/browser/BrowserView.svelte` (workspace effect, address-bar submit).
- `ui/src/lib/stores/canvas.svelte.ts` (whole): auth context, queued writes, open, restore, live pushes, autosave.
- `ui/src/modules/canvas/ExcalidrawCanvas.svelte`: source load/generate/effects, snapshot/fingerprint/debounce, initialData/mount/unmount; `MermaidCanvas.svelte`: staged typing/debounce/flush; `CanvasPage.svelte`: mount key/back-to-list; `ConversationPanel.svelte`: restore caller.
- `crates/otto-design/src/service.rs`: restore/version resolution/detail, commit publication, approve, metadata patch, hard delete; `store.rs`: metadata/thumbnail/approval SQL; `http.rs`: metadata route.
- `ui/src/lib/stores/product.svelte.ts`: draft/create/update, transcripts, publish, versions, analysis, questions, notes, events, rewrite/plan and swarm dispatch methods (approximately lines 360–705). Existing owner checks prevent several late mutation responses from replacing the newly selected story.
- `crates/otto-vault/src/engine.rs`: guarded note/text writes, soft delete, folder creation, rename/link rewrite orchestration (approximately lines 1110–1530). Traced stale hash rejection before replace, existing-target rename rejection and case-only rollback, post-move rewrite failures proceeding to rescan. `recovery.rs`: revision path cache/metadata read and append (first 220 lines).
- `ui/src/modules/snip/annotations.ts`: render/shape paths through arrow rendering; `SnipEditor.svelte`: serialized copy, persistence drain, failed-save leave guard (approximately lines 245–324); `crates/otto-server/src/routes/snips.rs`: save annotated/copy/delete (lines 639–685). Traced unsuccessful upload returning false versus successful upload with unavailable clipboard returning persistence success.
- Existing regression source read: `ui/unit/browserAnnotationOwnership.test.ts`, `ui/unit/canvasPersistSupersede.test.ts`, and the aliased DesignService commit test in `service.rs`. These tests cover adjacent cases, not the four findings above.

Omissions: no exhaustive review of these large modules; no Browser CDP/native/remote runtime or provider/login integration; no full Product HTTP/agent lifecycle/media review; no full Vault scan/recovery restore implementation sweep; no Design graph/prune/site/brand audit; no native clipboard or Excalidraw runtime execution; no visual, accessibility, or copy review. Claude's Product route selection and Plan/Rewrite Stop ownership were not reviewed or edited. All existing historical findings were treated as closed; this pass inspected current source.

## Provisional fixed-rubric score

| Correctness dimension | Score /2 | Evidence and deduction |
|---|---:|---|
| Contract/data integrity | 1.4 | Traced metadata lost update and incorrect persisted Browser title; guarded Vault hash path is sound in inspected code. |
| State/concurrency ownership | 1.3 | Browser creation crosses workspace ownership; queued Canvas writes and Product mutation owner checks are present, but do not cover the traced gaps. |
| Boundary/error behavior | 1.8 | Traced Vault rename collision/rollback and Snip upload-versus-clipboard failure paths; broad runtime error matrix remains unexecuted. |
| Persistence/recovery | 1.7 | Version restore exists and write queues preserve ordering, but Excalidraw does not apply all restored persisted state; metadata race loses acknowledged edits. |
| Executed regression coverage | 0.0 | No tests or journeys executed in this assigned read-only review. Existing test source is evidence of intent only; root may append actual execution separately. |
| **Total** | **6.2 /10** | **Provisional source review; not acceptance. Source dimensions total 6.2/8.** |

Confidence is high for the four explicit source traces and bounded for the rest of this partition. The blocker and major findings preclude 9.8 acceptance regardless of a later execution score. Re-score from repair and regression evidence; do not relabel this unexecuted review as runtime coverage.
