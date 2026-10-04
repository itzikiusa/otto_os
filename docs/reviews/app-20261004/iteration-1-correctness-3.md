# Iteration 1 — correctness partition 3

**Verdict:** Block. **Counts:** blocker 3 · major 5 · minor 0 · nit 0.

**Baseline:** `a16f4c71d5b158359fc3507c9f883c500c1f88a3` in `/Users/itziklavon/claude_ade-review`.

**Scope:** Existing application audit of Vault, Canvas, Design Hall, Product, Browser, Snip, and their persistence boundaries. Applied the `correctness-review` skill. This is a selective review of important user actions, not a complete read of every file in these modules. All findings below are **confirmed by hand trace**, not by running the app. No builds, tests, servers, source edits, or remote mutations were performed. The only authored file is this report. Purely visual issues and Vault refine undo were excluded as assigned.

**Intended behavior:** Drafts survive until saved or deliberately discarded; responses belong to the item that initiated them; version conflict checks refer to the actual current head; file rename never replaces an unrelated existing file; explicit Copy changes the clipboard; page marks belong to the displayed page. These expectations are stated by the cited comments, action labels, existing guards, and persistence contracts in the code.

## Findings

### C3-01 [blocker] Product draft navigation bypasses the unsaved-change guard

**Location:** `ui/src/modules/product/ProductPage.svelte:379`, `:410`; `ui/src/modules/product/OverviewTab.svelte:125`, `:315`; `ui/src/lib/leaveGuard.ts:27`.

**What / intended:** The draft editor explicitly promises “leaving asks,” but changing the Product story or tab drops unsaved title/body changes without asking. The only guard registered by OverviewTab is a router guard; neither action uses the router.

**Evidence — confirmed, hand trace:** Open draft A and change its body without pressing Save. Click another Product group: `selectGroup()` directly assigns `product.tab` at line 381. The conditional at `ProductPage.svelte:865` unmounts OverviewTab, so its local draft is lost and the router guard never executes. Reopen Overview: the form is seeded from the persisted source. Alternatively click draft B: `selectStory()` calls `openStory()` → `product.select()` directly; when B's detail loads, `reseedDraft()` sees a different ID and replaces both fields (`OverviewTab.svelte:314–318`, `draftSeed.ts:24–27`). Returning to A loads its saved body, not the discarded edits.

**Why it matters:** Normal list/tab navigation loses real authored work. This is independent of network timing.

**Fix:** Route all Product story/view/tab transitions through a shared leave check before mutating selection, or persist per-story drafts and restore them. Include tab buttons, keyboard activation, story selection, and view switches; a router-only guard cannot cover these state changes.

**Regression:** Edit draft A, attempt another story and another Product tab, choose Keep editing, and verify selection and text stay unchanged. Then explicitly discard and verify navigation succeeds. Also verify a saved draft navigates without a prompt.

### C3-02 [blocker] “Save mine on top” discards the design if version lookup failed

**Location:** `ui/src/modules/design-hall/ArtifactView.svelte:409` (support: `:383–398`, `:162–165`).

**What / intended:** Conflict resolution takes the destructive reload branch after the user explicitly chooses to preserve and save their local edits.

**Evidence — confirmed, hand trace:** Edit a design against v1; another writer commits v2. The save receives 409 and enters `resolveConflict()`. Let its `listVersions()` request fail transiently: the catch leaves `latest = null`. The dialog still offers “Save mine on top.” Selecting it reaches `if (!latest) { await load(id); return; }`. A successful subsequent artifact GET assigns `source = text` and `baseSource = text` in `load()`, replacing the unsaved local source with v2. No version containing the user's edits was written. The user did not choose the separately labelled discard option.

**Why it matters:** A temporary history-fetch failure causes irreversible local draft loss precisely when the user chooses to keep the draft.

**Fix:** Preserve the working source and return an actionable lookup error or retry fetching the head when `latest` is unavailable. Never call `load()` from the keep-mine branch until a successful save is confirmed. Retain the exact chosen local content across retries.

**Regression:** Stub content PUT → 409, version list → error, artifact GET → success. Choose Save mine on top and assert local source remains unchanged/dirty and no discard reload happens. Retry with a valid latest version and assert the preserved source is saved using that base.

### C3-03 [blocker] Late Product mutations install story A's detail under selected story B

**Location:** `ui/src/lib/stores/product.svelte.ts:268`, `:366` (related unguarded collection publication at `:528`, `:537`).

**What / intended:** Detail writes do not apply the selection ownership check that `loadDetail()` already uses. Consequently the displayed draft can belong to A while subsequent save requests target B.

**Evidence — confirmed, hand trace:** Start `updateDraft()` for selected draft A; its PATCH captures A's ID at line 361. Before the response returns, select B and let B's detail GET finish. A's PATCH then completes and line 366 unconditionally sets `this.detail` to A's saved detail while `selectedId` remains B. OverviewTab derives `story`/`source` from this incorrect detail and reseeds its form to A. Edit that displayed body and press Save: `updateDraft()` now obtains B from `storyId()` and PATCHes A-derived content into B. There is no active detail/selection mismatch guard once `loadingDetail` is false (`OverviewTab.svelte:1194`). The simpler metadata path has the same ownership error: adding a tag to A and switching to B before the PATCH settles replaces `detail.story` with A while retaining B's `detail.source` at line 268. `patchStory()` at line 276 demonstrates the intended identity check.

**Related scope:** Story subcollections have the same publication problem. `loadNotes()` can finish A after B's notes load and replace `product.notes` with A's rows (`:528`); NotesTab renders these rows and edits/deletes their actual IDs (`NotesTab.svelte:91–111`, `:152`). This can modify A's notes while B is selected. Similar unguarded methods include questions, versions, analyses, and transcripts. This is one response-ownership remediation, not separate findings per collection.

**Why it matters:** A normal delayed request can lead to subsequent persistent edits on the wrong story, or wrong-story note edits/deletes.

**Fix:** Capture workspace/selection generations for every request. Update keyed list entries by their original ID, but publish detail/collection data only while the initiating workspace and story still own the view. Clear or key collections when selection changes, and prevent editing any detail whose story ID differs from `selectedId`. Guard loading flags too so a stale completion cannot clear a newer load's state.

**Regression:** Use deferred responses for A mutation and B detail load; resolve B first and A last. Assert B's detail and source stay together and the next Save contains B's data. Repeat with A/B notes GETs and verify only B's rows/actions appear.

### C3-04 [major] Concurrent design commits can publish older working files, links, and events after the new head

**Location:** `crates/otto-design/src/service.rs:735`, `:745`, `:1051`; `crates/otto-design/src/store.rs:1417`.

**What / intended:** The version-row transaction orders head changes, but working-copy writes and index/event publication happen afterward with no per-artifact serialization or current-head check. Those derived surfaces can end up representing an older version.

**Evidence — confirmed, hand trace of an allowed interleaving:** Two content PUTs omit `base_version` (explicitly allowed: `store.rs:945–947` says `None = unconditional`; HTTP forwards the optional field at `http.rs:814`). Writer A commits version v2, then pauses before its working-file write. Writer B commits v3 and completes its file write, reindex, and content event. A resumes: it writes v2 bytes at line 740, fetches the now-v3 artifact at line 745, and calls `reindex()` with v2's version and bytes. `replace_extracted()` unconditionally deletes all current extracted links for the artifact and inserts A's links (`store.rs:1427–1450`); search is also written from A's text. Finally A emits a v2 content event after v3. ArtifactView accepts that event's source when clean (`ArtifactView.svelte:312–315`). The DB head is v3, but working bytes, graph/search and clean client content can show v2. A subsequent named commit without content explicitly reads the working file (`service.rs:828–838`), making stale bytes actionable.

**Why it matters:** Concurrent agent/API edits produce inconsistent current designs and can re-commit older content. Versions themselves remain available; this is not a claim that blob history is lost.

**Fix:** Serialize the full commit/publication pipeline per artifact, including head resolution, working-file replacement, derived indexes and event order; or use version-aware CAS publication for every derived surface plus clients rejecting older sequence events. A transaction around the version insert alone is insufficient.

**Regression:** Add a barrier after A's version transaction, complete B, then resume A. Verify final head, working bytes, extracted links, search text, and client-visible event state all represent v3. Exercise unconditional writes and a no-content named commit afterward.

### C3-05 [major] Unchanged-content design saves check a stale head and bypass optimistic concurrency

**Location:** `crates/otto-design/src/service.rs:690–710`; caller `crates/otto-design/src/http.rs:801–802`.

**What / intended:** Deduplication decides an unchanged save against the artifact snapshot loaded before body parsing/validation, not the current head. It can return success for a stale `base_version` that must conflict.

**Evidence — confirmed, hand trace:** Request A loads artifact head v1 at `http.rs:801`, carrying content identical to v1 and `base_version = v1`. While A parses/validates its body, request B commits different content as v2. A's `commit_bytes()` uses `artifact.head_version_id` from its old snapshot to load v1 at line 690. The SHA equals v1; the base equals v1, so it returns `created: false`/v1 at line 705 without calling `commit_version()` and its atomic current-head predicate (`store.rs:960–961`). The actual head is v2. Expected: 409 for the stale base, or an explicitly current-head dedup result under the same atomic concurrency semantics; actual: successful obsolete result.

**Why it matters:** A save can acknowledge content that is not the current saved content and give the editor an obsolete base version. The same stale comparison can skip an unconditional request intended to restore the earlier bytes.

**Fix:** Resolve current head, check the requested base, and perform the no-op decision under the same serialization/transaction boundary as a normal commit. Merely fetching the head once more before returning still leaves a race with another writer.

**Regression:** Pause A after artifact loading, commit v2 with B, then resume A's identical-to-v1 request. With base v1 assert 409; with no base assert behavior based on the current v2 head rather than the stale snapshot.

### C3-06 [major] Case-only Vault rename replaces a distinct existing note on a case-sensitive volume

**Location:** `crates/otto-vault/src/engine.rs:1391–1393`, `:1455–1466`.

**What / intended:** The target-exists rejection exempts all case-insensitive string matches. That assumes both spellings refer to one filesystem entry, which is false on case-sensitive volumes.

**Evidence — confirmed, hand trace:** In a case-sensitive registered vault create distinct `A.md` and `a.md` with different contents; request rename `A.md` → `a.md`. `abs_guarded()` returns the joined paths without normalizing away the final component (`engine.rs:886–900`). Both paths exist, but `case_only` is true because lowercasing produces equal strings, so the existing-target conflict is skipped. The two-step branch renames `A.md` to its temporary filename, then `tokio::fs::rename(tmp, a.md)` replaces the distinct destination file. Its previous content was not placed in trash or revision history by this rename path. Expected: target-exists conflict and both files unchanged. Actual: `a.md` contains A's content and its old content is lost.

**Why it matters / severity:** Real data loss, limited to case-sensitive vault volumes or equivalent host filesystems; rated major for the narrower deployment trigger. The default case-insensitive APFS case-only rename is valid and should continue working.

**Fix:** Exempt an existing target only when filesystem identity confirms it is the source entry. Reject distinct targets even when strings differ only by case, and use an exclusive/no-replace final move where available to close the existence-check race. Roll back the temporary move if the second step fails.

**Regression:** On a case-sensitive test volume, seed both names, attempt the rename, and assert conflict plus preservation of both contents. Retain a case-insensitive APFS test for a legitimate one-entry case-only rename.

### C3-07 [major] Cached-page annotation reads can replace the active page's marks with another page's

**Location:** `ui/src/lib/stores/browser.svelte.ts:295–303`; callers `:140–148`; actions `ui/src/modules/browser/NotesRail.svelte:38–41`.

**What / intended:** Annotation loads check workspace identity but omit active page/tab ownership, unlike the sequence-guarded full page loader. Cached tab switching therefore can display and operate on marks from the wrong URL.

**Evidence — confirmed, hand trace:** With reader pages A and B cached, select A; the cache path starts `loadAnnotations(A.url)` without a page token. Immediately select B; the same path starts B's request. Resolve B, then A. Both workspace checks pass, so the last A response sets `annotations` even though active tab/page is B. NotesRail renders the supplied rows directly and its Delete/Send actions use those A IDs. Expected: only B's marks remain visible/actionable. Actual: A's marks appear under B and Delete removes A's persisted mark. The live-event path already compares annotation URL against activeTab URL (`browser.svelte.ts:396–402`), but that does not guard this HTTP completion.

**Fix:** Capture the active tab ID, URL and request generation and only publish when all still match. Apply the same ownership rule to `createAnnotation()`'s post-await append so creating a mark and switching pages cannot contaminate the new page either.

**Regression:** Seed both page-cache entries, resolve annotations for B before A, and assert the store/rail retain only B IDs; repeat a delayed create on A followed by switching to B.

### C3-08 [major] Explicit Snip Copy reports success without recopying unchanged images

**Location:** `ui/src/modules/snip/SnipEditor.svelte:277–280`, `:288–289`, `:752`.

**What / intended:** The autosave deduplication gate is also used by the explicit Copy button and ⌘C. Once the annotations are saved, pressing Copy does not touch the clipboard.

**Evidence — confirmed, hand trace:** Let a snip annotation save succeed; `savedHash` is set to the annotation hash at line 288. Copy unrelated text in another app, return to the unchanged snip, and press Copy (`:752`) or ⌘C (`:577–579`). `uploadNeeded()` compares only annotation hashes and returns null, so line 279 sets `copyState = 'copied'` and returns without any clipboard call. Pasting still yields the unrelated text. There is a second deterministic failure: when annotated save succeeds but returns `{ copied: false }`, the code still updates `savedHash`; pressing Copy to retry immediately takes the same no-op path and falsely reports copied.

**Why it matters:** The prominent export action lies about its outcome and cannot retry a failed clipboard operation unless the user modifies the image.

**Fix:** Separate autosave deduplication from explicit recopy. For explicit Copy with unchanged content call the existing `POST /snips/{id}/copy` endpoint (`crates/otto-server/src/routes/snips.rs:655–669`); changed content can still flatten/save. Update the reported state from the actual copy response and allow retry after `copied: false`.

**Regression:** After an initial successful upload, change the simulated clipboard and press Copy; assert the copy endpoint is invoked and the image replaces it without another image upload. Also simulate a saved image with clipboard failure and verify a subsequent Copy retries rather than merely setting success text.

## Coverage and limits for the next wave

- **Vault:** Read UI save draining, ownership checks, conflict handling, draft retention, navigation, and Rust note writes/rename/link-rewrite sequencing. The existing note save path captures vault/path and drains edits typed during a save; it was not reported as an unguarded save. No review of refine undo. Recovery restore/trash combinations, index watcher interleavings, external-editor races, and actual case-sensitive filesystem execution remain untested.
- **Canvas:** Read open/save ownership, staged per-scene drafts, write queue, restore ordering, HTTP updates, SQLite scene updates/snapshots/file refs, and assist optimistic-concurrency logic. No confirmed Canvas-specific finding in this pass. Legacy scene saves versus the current Excalidraw/Mermaid/D2 editors, image reference GC under simultaneous delete/save, and assist poll/final-commit races need deeper execution. No claim of exhaustive Canvas correctness.
- **Design Hall:** Read keyed artifact mounting (so an old component save does not directly mutate a newly mounted artifact component), unsaved routing, source/base handling, conflict dialog, version storage/commit/reindex, and site/studio ownership boundaries. The keyed mount eliminated one suspected cross-artifact UI race. Full brand impact, site export, 3D import/optimization, retention and variant promotion were not fully reviewed.
- **Product:** Traced selection, local draft seeding, metadata/draft mutations, notes and other collection fetches, and caller navigation. Read relevant parent-tree and publication surfaces selectively, but did not exercise Jira/Confluence or claim remote publication correctness. Rich Jira field editing and attachment content save queues remain follow-up targets.
- **Browser:** Traced reader cache/load ownership, tab switching, marks rendering/edit/delete/send, live overlay drain, and native/live branch selection. Chromium protocol/control, credential behavior, and native navigation/close races were not exhaustively traced.
- **Snip:** Traced annotation hashing, flatten/save/copy sequencing, explicit button/keyboard path, cleanup, server annotated writes and recopy endpoint. Native-window teardown while encoding, concurrent editors, and original-vs-annotated reopen behavior remain unexecuted.

**Verification:** Source inspection and explicit scheduling/input traces only. Regression cases above are proposals, not tests that were run or added. No passing build/test claim is made. No external systems or real user data were touched.
