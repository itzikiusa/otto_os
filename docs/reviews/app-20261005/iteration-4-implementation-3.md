# Iteration 4 implementation — content partition 3

Status: first UI batches implemented; backend publication/metadata and performance work remain. This is a checkpoint, not release acceptance. Worktree `/Users/itziklavon/claude_ade-review`, branch `fix/app-review-20261005`. Parent runs all verification centrally; this implementer ran no tests, builds, servers or live mutations.

## Completed source batches

| Finding | Repair and ownership |
|---|---|
| C3-02 | `ui/src/lib/stores/browser.svelte.ts`: workspace generations fence reader/live tab creation, including A→B→A. Tab-list requests also have sequence ownership. Stale creation still returns its own server result without selecting/fetching into the new view. |
| C3-03 | Same store: reader navigation uses the request-local returned page, with per-tab navigation sequencing. All existing tab PATCH writers serialize by tab so stale network completions cannot leave the older URL persisted. Settled queue entries release; close invalidates navigation ownership and waits for pending mutations. |
| C3-04 | `ExcalidrawCanvas.svelte`: both scene replacement and initial load apply only persisted background/grid, with defaults for absent state. A restore→snapshot test proves these values survive the next save; viewport state stays transient. |
| UX3-03 | `ConversationPanel.svelte`, three Canvas editor generate handlers, and the handle type in `CanvasPage.svelte`: explicit accepted boolean. Failed, busy or empty generation retains the exact draft. Only accepted completion for the same editor, scene, save context and unchanged draft clears the composer. |
| UX3-02 | `PublishDialog.svelte`: project/space/issue-type loaders fence account, mode and request; project and space errors offer Retry for a single account and retain RFC title/parent. Destination errors are local so retry does not retain an obsolete form error. |
| D3-02 | `vault.svelte.ts`, `Switcher.svelte`, `TagsPanel.svelte`: lookup rejection remains an error; tags retain last good values with Retry; switcher rejects stale query/vault/workspace/generation responses. Ordinary Enter requires a current successful empty lookup; Shift+Enter remains intentional creation. Pending/error states do not claim no matches. |

Claude retains visual/CSS/accessibility ownership. Narrow loading/error/Retry wiring above and the CanvasPage type segment are reserved in `/tmp/otto-app-review-coordination-20261005.txt`. No global API contract/type edits yet.

## Verification evidence

- Parent first ran browser regressions: five intended failures and one fixture scheduling failure. The cross-tab test was corrected to await a real reader refresh rather than a fixed microtask count. It then failed at the intended `Title B` versus `Title A` assertion.
- Parent ran corrected browser plus Canvas: 12 tests, 11 intended failures, one passing scene-switch control. After authorized repair: 12/12 passed.
- Parent ran the second RED batch: 30 tests, 22 passed and eight intended failures (persisted live navigation order, project/space ownership, pending/failed switcher Enter, stale vault result, tag loss, swallowed lookup failure). Log: `/tmp/otto-review05-role3-lookup-red.log`.
- Parent's combined focused GREEN: **39/39**, 1.246 seconds, `/tmp/otto-review05-role3-ui-green2.log`. Files: `browserNavigationOwnership.test.ts`, `canvasRecovery.test.ts`, `vaultStore.test.ts`, `vaultLookupRecovery.test.ts`, `publishDestinationOwnership.test.ts`. Tests execute actual store/component functions through transport adapters; they do not claim rendered Svelte or native behavior.
- After that GREEN, parent requested two small source corrections: suppress “No tags match” when only a load error exists, and isolate destination errors from publish-form errors. These corrections are authored but have not yet been re-run at this checkpoint.

New unit-only helper: `ui/unit/componentFunctions.ts`. Existing `sourceHarness.ts` is reused. No new production test hooks or unimplemented APIs were required for the observed RED cases.

### Independent UI follow-up

The independent C3 recheck accepted the original repairs and found two adjacent cases: closing the active reader left its pending page/error response valid; Down during pending switcher lookup left selection at −1. Parent observed **17 tests: 14 passed, three intended failures** (late reader success/failure and pending Down→Enter). After repair and a failed-close recovery control, parent observed **18/18 passed**, 436 ms. Active close now invalidates reader/annotation ownership before awaiting transport, restores prior content on owned close failure, and leaves background-tab loads valid. Successful switcher lookup resets selection; empty arrow movement stays bounded. The independent follow-up resolved both findings in source; mounted/native acceptance remains pending. The earlier two error-state corrections also passed their **7/7** focused rerun.

### Backend test-authoring checkpoint (unrun)

Production backend code remains unchanged pending central RED. Added actual service/route regressions:

- `otto-design/src/service.rs`: concurrent metadata edits through separate service instances with symlink-alias roots must preserve acknowledged title, tags and independent JSON keys. A second test holds the real canonical publication mutex and requires metadata/approval to respect it, then checks durable title/status/approved head.
- `otto-product/src/http.rs`: 12 real HTTP/client tests use only isolated wiremock outbound accounts. Each Jira/Confluence flow previews via the actual version route, then independently mutates the same version body, adds a newer preferred version, changes title, changes RFC reference, or omits review identity. Each unsafe case must return 409 before any outbound request. Two unchanged controls capture and compare the exact outgoing title and rendered description/storage payload, including RFC reference and parent.
- The body hash is required: `update_draft_body` edits an existing version in place, so version ID alone cannot bind reviewed bytes. Proposed identity includes version ID (nullable), SHA-256 of exact UTF-8 body, story title, source kind and URL. Missing identity is a documented fail-closed 409 compatibility change. Parent granted narrow Product publish contract/type sections. This contract is not implemented yet.

Central commands requested: `cargo test -p otto-design --lib metadata_` and `cargo test -p otto-product --lib http::tests::reviewed_`. No execution or result is claimed for these new tests.

### Large-content UI checkpoint

Parent observed **0/10** on the first large-content RED batch: equal 50k-line input became 100k changed operations; sparse input became 50k deletes; actual DiffView template inputs held 10k–20k rows; opening 100 collapsed 256 KiB transcripts transferred **26,227,002 serialized fixture bytes**. These are actual handler/algorithm assertions with deterministic transport adapters, not rendered DOM counts, RSS or latency measurements.

Implemented DiffView prefix/suffix stripping, ordered unique anchors, a shared 4M-cell DP allocation budget, a bounded repeated-line Myers fallback (distance ≤256 and 4M comparison budget), and 500-row template pages. Next/Previous handlers reach every changed line; full before/after downloads preserve exact bytes. **Line/operation/row metadata remains O(N+M)**; work budgets bound the expensive diff stages and rendering, not total input-size storage. Parent passed the original batch plus 20-page navigation/Blob-byte tests (**9/9** including async-find tests). A follow-up correctly exposed repeated-line sparse changes (49,971 false deletions) and departed-provider errors. After repair, parent observed **18/18**, 493 ms. Parent subsequently passed **22/22**, 581 ms, including insertion/deletion/reordering reconstruction and the oversized-body download control.

Product UI now requests 50-item summary pages, loads individual bodies on expansion, and retains at most four bodies/4 MiB charged UTF-16 string bytes. Larger legacy bodies display an explicit Download path and never enter the retained body cache. The full-body download is an explicit transient allocation; it is not claimed to fit the retained-cache bound. Page find has an optional abortable async-provider hook; synchronous providers retain their existing literal, case-insensitive behavior. Product search keeps bounded match metadata independently of the visible page, so revealing an older collapsed hit fetches its ID and body rather than changing match indices during pagination. Search and load errors offer Retry; aborted/obsolete queries cannot publish into a new provider/story.

Root's six transcript handler/cache tests passed against proposed endpoint adapters. **The backend endpoints are still absent**, so this is not end-to-end acceptance. The later oversized-body download control passed in the 22/22 batch. The actual HTTP transcript regression batch now covers 100×256KiB summary response bounds, legacy full-array compatibility, timestamp-tie/new-import cursor stability, literal Unicode search over the oldest of 105 transcripts, full individual-body read, and owning-story Viewer authorization for all three read paths. Requested command: `cargo test -p otto-product --lib http::tests::transcript_`. All five new Rust tests remain unrun; backend production is unchanged pending central RED.

### Independent performance UI follow-up

The independent review found three lifecycle gaps: successful import could displace a boundary row while retaining its old cursor; provider unregister did not release a pending paged search; and an async search reveal could expand/highlight/scroll after close or a replacement query. Parent ran `productTranscriptLifecycle.test.ts`: **1 passed / 11 failed**, 516 ms, all at the intended assertions; the normal real-reveal control passed. Tests cover 50/51/100-row boundaries, importing from older pages, stale page completion, search-reveal page restoration, actual registry unregister, and the actual Overview provider plus FindInPage reveal/navigation functions.

Authorized repairs now refresh the coherent first summary page after import, fence pre-import page completions, retain and restore the ordinary page around off-page search reveals, release provider state on unregister, and propagate an optional abort signal/lifetime through async reveal. New query/close invalidates pending navigation and Product expansion. Source is ready for the central GREEN run; no result is claimed yet. Backend production remains unchanged and its tests remain queued for central RED.

## Outstanding scope and limits

1. C3-01: atomic metadata/approval preservation across canonical-root service aliases; deterministic Rust regression first.
2. UX3-01: exact reviewed version, title and RFC reference binding; isolated Jira/Confluence outbound capture tests. No real outward publication is authorized or used by this work.
3. P3-01: bounded transcript summaries/lazy bodies with cross-history find preserved, including collapsed history beyond the first page. Contracts/types must be coordinated with parent.
4. P3-02: large shared diff equality/prefix/suffix and bounded rendering with all omitted changes reachable.
5. Independent source/spec review, rendered Retry/Canvas/browser journeys, central type/unit/Rust gates and required performance/native measurements remain. No completion score or native/performance claim is made here.

### Backend implementation checkpoint after observed RED

Parent observed Design `metadata_` **0/2** (independent title lost; canonical lock bypass), and Product HTTP **10 passed / 14 failed**: ten stale/missing publication cases still published, summary response was26,234,081bytes, summary cursor envelope absent, and search/detail404. Two unchanged publication controls and legacy transcript reads passed. Metadata/approval now lock before the read and retain the existing canonical publication boundary through signals/events. Separate importer/thumbnail regressions are authored and unrun; their production repair is still pending.

Product source now includes thin transcript summary keyset pages, an append-only0171 ordering index, bounded streamed explicit-search pages with literal Unicode/nonoverlapping counts, and detail authorization before reading the body. Legacy full-array reads remain compatible. Publishing now captures body/version/story metadata once, validates reviewed identity/hash, and uses the captured values throughout the outbound operation; missing identity fails closed409. HTTP and workflow publishing callers pass the same typed request. Authoritative Product contracts and shared TypeScript exports are updated.

Parent observed UI publication **0/4** after correcting a strip-only TypeScript fixture syntax error (the parser error is not a behavioral RED). Authorized UI repair freezes metadata, hashes exact preview UTF-8 bytes, includes reviewed_content, offers the complete preview, and invalidates identity after409 until explicit reload/review. Central Product/UI GREEN remains pending at this checkpoint. No real outbound publishing is used; tests use isolated wiremock endpoints only.

### Workflow usability and adjacent Design checkpoint

Parent's central results: Product HTTP **24/24 GREEN**, publication/recovery/destination UI **17/17 GREEN**, and Design **125 passed /3 failed /1 ignored**. The original metadata regressions passed; the three additional importer/thumbnail regressions failed as intended. Their authorized source repair now uses the canonical lock and fresh row for importer merge and thumbnail no-op/GC. Central rerun pending.

Root identified that fail-closed workflow publishing lacked a usable acquisition path. Added a bounded feature scope: destination-configured dry-run snapshot → actual Human Approval → live publication of that identical stored snapshot. Dedicated `workflow_product_publish.rs` keeps this policy out of the general executor arms. The gate persists its exact pending preview, the UI lazily reads that node body with Retry and ownership/version guards, and live publication checks the specific successful persisted approval node, run identity, decision and immutable graph. Frozen account URL is checked against the same fetched account used by the service. New actual executor/SQLite/wiremock tests are tagged `review4_product_publish`; three UI handler tests cover full preview, detail-version retry and departed-run response. These workflow tests are **authored but unrun**; no observed RED or GREEN is claimed for them. Full rendered approval acceptance and independent source review remain pending.

Stable checkpoint: all assigned production sources and narrow contracts/types authored. Remaining work is central tests/type checks, independent source feedback, focused repairs, and rendered/performance/native evidence coordinated by root. No other workflow features were added; non-Product approval remains unchanged.

### Approval retry identity follow-up (R4-R3-BE-01/02)

Independent review found the human-decision seam: a P1 banner could approve a P2 replacement at the same run/gate after rejection and retry. Root observed UI **4 passed /4 failed**,306ms: missing displayed identity on approve/deny, missing-preview submit and absent409 invalidation. Authorized UI repair now retains the successfully fetched node detail version, submits that captured `expected_detail_version`, refuses decisions before display, and invalidates/reloads after409. Its GREEN is pending.

Added an actual driver/API regression tagged `review4_product_publish_real_api_rejects_stale_same_run_gate_after_retry`: real preview/gate execution, GET node detail, HTTP rejection, source mutation, HTTP include-downstream retry, stale P1 decision409, missing identity409, current P2 decision then isolated exact Jira/Confluence capture. A normal non-Product API control remains. These new Rust cases are unrun; approval backend production remains unchanged awaiting central RED. Updated the obsolete PO lifecycle template and feature guidance to a destination-configured preview → human review → approved live publish; its narrow graph regression is authored and unrun.

### Atomic approval source-ready checkpoint

Root observed the real driver/API retry regression return **200 instead of409** for stale P1 approval at the replacement P2 gate, authorizing the backend repair. The earlier UI identity batch is **8/8 GREEN**. `WorkflowsRepo::record_approval` now reads only the selected pending node body, compares the displayed SHA-256 detail version for Product decisions, and uses one UPDATE guarded by the captured revision, running status, waiting flag and gate ID. Ordinary approvals without Product output retain their request shape. Both approve and deny use the same boundary; the route delegates after workspace authorization. Contract updated. Central GREEN pending.

The workflow frozen-destination test's **3 vs1** failure was fixture accounting, not duplicate publication: Confluence `create_page` deliberately sends two POSTs to `/content/101/property` to set published/draft full-width presentation after the one page POST. The shared assertion now checks exactly one publishing POST and exactly those two known property path/payload mutations for Confluence, no other requests; Jira still has exactly one request. Diagnostic failures include captured methods/paths/bodies. The real-API retry test uses the same accounting. These corrected tests remain centrally unrun at this checkpoint.
