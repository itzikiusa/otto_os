# Iteration 4 — independent role 3 UI checkpoint recheck

**Verdict: Request changes.** Original assigned UI triggers are repaired in inspected source. Two adjacent cases remain: one major, one minor. This is a checkpoint recheck within iteration 4, not a new review iteration or score increase.

Read the partition-3 correctness, performance, UX and design reports first, then implementation-3 and VERIFICATION, followed by the current dirty production diffs, callers and focused tests in `/Users/itziklavon/claude_ade-review` on `fix/app-review-20261005`. Applied the correctness-review skill. No tests, builds, servers, source edits or commits were performed; this report is the only file written.

## Specification compliance first

| Requirement | Source disposition |
|---|---|
| C3-02: reader/live creation must not publish into another workspace visit | Repaired for the traced A→B and A→B→A completions. A workspace generation fences the resulting strip/selection/page publication. Returning the created server result remains allowed. |
| C3-03: navigation must use its own page metadata and preserve newest persisted URL | Repaired for the traced cross-tab title and reversed reader-response cases. `loadPage` returns its request-local page even when it cannot display it; per-tab sequence prevents obsolete reader navigation from issuing a PATCH. All store tab PATCH writers use `patchTab`, whose chain orders actual transport writes, including overlapping live navigations. Rejection does not poison the successor, and map entries release only if still the final chain. Close waits for the already-enqueued chain; the separate late reader-display gap is below. |
| C3-04: restore supported Excalidraw persisted app state | Repaired: `loadScene` and `initialData` use the same background/grid subset and defaults. `snapshotDoc` then serializes those values, while transient viewport properties remain excluded. |
| UX3-03: rejected assistant generation must retain retryable text | Repaired: all three generators return an explicit accepted boolean; empty, busy, and caught failures return false. The composer retains exact whitespace/newlines and only clears on accepted completion with the same editor, scene, auth-save context and unchanged submitted draft. `CanvasPage` and panel handle types agree. |
| UX3-02: destination failure must recover in-place without losing RFC fields | Repaired: scoped projects/spaces errors expose Retry, successful retry clears its own error, and loaders retain title/parent. Account/mode/request ownership prevents obsolete destinations and issue types from winning. General publish-form errors remain separate. |
| D3-02: pending/failed lookup must not imply an empty successful result | Repaired for ordinary Enter and create-button paths: only a successful current query permits implicit creation. Shift+Enter remains deliberate creation. Tags retain last successful values on failure, expose Retry, and no longer show the empty/no-match claim when the only available state is a failed initial load. The pending arrow-selection case is below. |

**Explicitly still open:** C3-01 backend metadata atomicity; UX3-01 exact reviewed publish version/title/RFC-reference binding; P3-01 transcript summaries/lazy bodies/search; P3-02 bounded large diff. This recheck does not claim or reopen those assigned findings as new defects. Destination-loader success does not establish publication payload safety.

## Remaining concrete findings

### R4-R3-UI-01 — major: a late reader load displays a tab after it was closed

**Locations:** `ui/src/lib/stores/browser.svelte.ts:226`–246; reader ownership guard at `:307`, publication at `:320`; rendering caller `ui/src/modules/browser/BrowserView.svelte:836`.

**Intended:** Closing the final tab leaves the reader empty; no delayed load owned by that closed tab may repopulate its content or annotations. The new close invalidation must cover visible reader state as well as the later persisted navigation.

**Confirmed by hand trace, not execution:** Start with one reader tab A. Call `navigate('https://a-new.test')` and hold the `loadPage` page/annotation pair unresolved. Its captured `pageSeq` is P. Call and complete `closeTab(A)`: line 227 removes the per-tab navigation sequence; no PATCH is queued yet, so close removes A and sets `activeId=null`, `page=null`, `annotations=[]`. It never increments `pageSeq`. Resolve the old page load. Line 307 still sees P and the same workspace, so lines 320–321 republish A's page and annotations. The outer navigate guard correctly prevents PATCHing the deleted tab, but is too late to prevent that earlier visible mutation. BrowserView's ReaderView is in the non-live branch without an `activeTab` guard, so the closed page reappears with an empty tab strip. The analogous late failure can repopulate a stale page error.

**Fix:** Invalidate the active reader-load ownership when closing its tab, before awaiting the close operation; clear/settle its loading state appropriately. Prefer binding reader requests to their initiating tab/selection generation, while allowing an explicit next-tab load to establish its own generation. Do not globally invalidate an unrelated selected tab's request when closing a background tab.

**Regression:** Defer the sole tab's reader load, complete close, then resolve/reject the old request. Assert `activeId` and `page` remain null, annotations remain empty, loading is false and no obsolete error appears. Include closing a background tab while the active tab's load is pending as a non-cancellation control. A mounted check should assert the closed markdown is absent, not merely the tab row.

### R4-R3-UI-02 — minor: Down during a lookup leaves successful results with selection -1

**Locations:** `ui/src/modules/vault/Switcher.svelte:31`, `:34`, `:66`, `:75`.

**Intended:** After a successful lookup, keyboard Enter opens a selected result. Waiting for a request must not put selection outside the result list.

**Confirmed by hand trace, not execution:** Start `refresh('Existing')`; line 31 sets `hits=[]`, `sel=0`, `loading=true`. Press ArrowDown before the response: line 66 calculates `min(1, -1)`, leaving `sel=-1`. Resolve with one valid result. The new successful-response branch sets `hits` and `resolvedQuery` but does not reset/clamp `sel` (the earlier implementation reset selection on completion). Finally clears loading. Enter then passes the current-success checks, calls `pick(hits[-1])`, and returns without opening anything. This needs only a keyboard user anticipating a result during ordinary network latency.

**Fix:** Ignore or safely clamp arrow selection while no current results exist, and reset/clamp selection when publishing the current successful result. Preserve the current error/pending creation gate.

**Regression:** Deferred refresh → ArrowDown → resolve one hit → Enter must open that hit. Include an empty result control and movement through multiple successful results. Existing pending-Enter tests do not exercise selection mutation during pending lookup.

## Quality, lifecycle and draft review

- Inspected all changed Browser store methods and immediate BrowserView render/mode callers. Queue ownership and release are explicit; no detached rejecting cleanup promise was introduced. The above close/read generation mismatch is the concrete remaining defect; broad native/CDP/WebSocket ordering is not certified here.
- Inspected CanvasPage's editor type, ConversationPanel send/restore caller, all three generate handlers, and Excalidraw load/snapshot/initialData. Success/failure handshakes agree. No new source path was found that overwrites a newer composer draft on accepted completion. Scene/auth guards precede successful document ingestion. Actual renderer mounting, destruction and restore→edit flows remain unexecuted.
- Inspected Vault select/reset, loadTags/switcherQuery, Switcher and TagsPanel. Generation/sequence/ID checks fence responses. In the switcher effect, synchronous reads through refresh establish vault/workspace dependencies; the query predicate is evaluated after await, so the new predicate is not itself a synchronous query dependency causing an effect reset on each keystroke. This is a source trace, not a compiled rune-runtime test. Close/reopen obtains a newer refresh sequence. Guards remain after the note-leave check, preserving failed-save draft behavior.
- Inspected PublishDialog effects, accounts→destination wiring, loaders, Retry markup and submit context. Request counters are plain bookkeeping, and destination publication happens after await behind ownership guards. No new RFC title/parent clearing was introduced by Retry. The unchanged unbound publication payload remains explicitly assigned elsewhere.

## Actual evidence and coverage limits

The supplied `/tmp/otto-review05-role3-ui-green2.log` reports **39/39 passed**, **1246.143083 ms**, with verified named counts: Browser 7; Canvas 9; Vault store 16; lookup 4; destination 3. The VERIFICATION ledger separately records the subsequent lookup/destination rerun **7/7**, **270 ms** after the two error-presentation corrections. This reviewer read the 39-test log and ledger; did not execute either command.

Read test sources: `browserNavigationOwnership.test.ts`, `canvasRecovery.test.ts`, `vaultLookupRecovery.test.ts`, `publishDestinationOwnership.test.ts`, plus `componentFunctions.ts`. The latter extracts and runs actual component function declarations but explicitly does not emulate Svelte reactivity; the store harness similarly does not mount the page. Therefore these are real handler/store assertions, not browser/native acceptance. The two findings above are not covered by the existing 39 tests.

Still pending: new regressions for this report, current combined typecheck/guards, mounted switcher timing and Retry states, Canvas three-format prompt retry/restore, Browser close/navigation journeys, native WebKit/CDP, screenshots/light/dark and full affected gates. No score was increased and no runtime/typecheck result was inferred from source or earlier checkpoints.

## Follow-up disposition — two reviewer findings only

**Verdict for R4-R3-UI-01/02: repaired in source, with root-observed focused regression evidence.** This bounded follow-up inspected the specification first, then the changed handlers and regression assertions. It does not replace the broader pending acceptance limits or increase any score. No tests/builds/source edits were performed by this reviewer.

- **R4-R3-UI-01 — resolved for the reported trigger.** `closeTab` now invalidates both page and annotation generations synchronously before its first await when closing the active tab, and clears visible page/annotations/error/loading. Therefore either a late success or rejection fails the original reader ownership predicate before it can republish. Closing a background tab does not invalidate the active reader. On failed active close, the captured previous page and marks are restored only when tab, workspace generation and page generation still belong to that close; a missing prior reader page triggers a fresh load. Existing queued tab mutations remain ordered before DELETE. Inspected regressions assert success/failure invalidation while DELETE is still pending, final empty state, no obsolete PATCH, unaffected background-close reads, and failed-close content/selection preservation.
- **R4-R3-UI-02 — resolved for the reported trigger.** ArrowDown now floors selection at zero for an empty list, and publishing a current successful result resets selection to zero. Pending Down → one arriving hit → Enter consequently opens that hit. Existing pending/error creation guards remain intact. Inspected regressions assert that exact sequence, multiple-hit bounds/Up navigation, and successful empty lookup retaining deliberate creation after pending Down.

**Evidence:** VERIFICATION records the expected RED result **14 passed / 3 failed**, 336 ms. Root reports the latest focused GREEN for `browserNavigationOwnership.test.ts` + `vaultLookupRecovery.test.ts`: **18/18**, **436 ms**, including the added failed-close control. The inspected files contain 11 Browser and 7 lookup cases. Execution belongs to root; this reviewer inspected production code and test assertions only. Mounted Svelte/browser/native behavior and the combined current typecheck remain separate pending acceptance, as already recorded above. No remaining defect was found within this two-finding follow-up scope.

## Follow-up — backlinks, installed Excalidraw contract, Product lifecycle

**Disposition:** Backlink ownership and the three Product lifecycle repairs satisfy the reported source-level requirements. Excalidraw restore/serialization now matches the installed API; one adjacent minor autosave gap remains below. No score uplift. This reviewer ran no tests/builds and changed only this report.

### Vault backlinks — repaired for the reported triggers

Inspected `vault.svelte.ts` select reset, accepted note navigation, `reloadBacklinks` and the three new actual-store tests. Refresh now sets pending/error state, keeps existing same-note rows until a successful response, and publishes success/error/finally only for the same vault, note, workspace and backlink generation. An accepted different note clears the former note's rows/error/loading and invalidates its response; these resets occur only after both leave/save checks and the note read succeed, preserving the previous editable note on failed navigation. Vault selection resets the same state. A newer same-note request invalidates older errors. Root's ledger records three expected RED failures followed by **26/26** combined Vault tests, **1.303 s**. StructuredNote loading/error/Retry presentation is still Claude-owned and requires integrated rendered acceptance.

### Excalidraw installed contract — normalization repair verified, autosave seam remains

Verified against the installed package, not just the fixture:

- `ui/node_modules/@excalidraw/excalidraw/dist/types/excalidraw/types.d.ts:298` requires numeric `gridSize`; line 300 requires boolean `gridModeEnabled`.
- `dist/types/excalidraw/constants.d.ts:152` declares `DEFAULT_GRID_SIZE = 20`; installed `dist/dev/chunk-4FTI6OG3.js:479`–481 initializes size 20 and enabled false.
- Current `persistedAppState`, `initialData`, source replacement and `snapshotDoc` use numeric size with default 20 and an independent enabled flag defaulting false. Legacy null normalizes to 20/disabled; false is retained rather than mistaken for absence. This supersedes the earlier report's acceptance of null as the grid default.

Root reports **9/9** Canvas tests green, including enabled/disabled restoration and legacy-null normalization. These tests directly exercise actual source load/snapshot functions, but not the edit-trigger path below.

#### R4-R3-UI-03 — minor: changing only the grid does not schedule persistence

**Locations:** `ui/src/modules/canvas/ExcalidrawCanvas.svelte:313`–320; installed toggle implementation `ui/node_modules/@excalidraw/excalidraw/dist/dev/index.js:8414`–8422.

**Intended:** Grid size/enabled state are now deliberately persisted and restored with the board; a user's grid-only change must enter the save path just like a background change.

**Confirmed source trace:** Begin with a mounted baseline of element-version sum V, file count F, background white and `gridModeEnabled=false`. The installed `actionToggleGridMode.perform` toggles only app state (`gridModeEnabled` and snap mode), leaving elements/files/background unchanged. Otto receives `onSceneChange` with enabled true, but its fingerprint contains only V/F/background. `unchanged` is true and line 320 returns before setting `changePending`, staging a draft or scheduling save. Close/reopen without another drawing/background edit: no grid save was sent, so the saved disabled state returns. A grid-size-only change has the same omitted-fingerprint mechanism. The snapshot roundtrip test cannot detect this because it directly calls `snapshotDoc`.

**Fix:** Include the persisted normalized grid size and enabled flag in the content fingerprint, keeping transient viewport/selection excluded. Preserve suppression of programmatic source loads and the initial baseline.

**Regression:** Invoke actual `onSceneChange` with a baseline then toggle-only app-state change and assert dirty/staged persistence captures enabled true. Repeat enabled→disabled and size-only change; viewport-only/no-op controls must still avoid saving. Keep installed numeric defaults in those assertions. Mounted reload remains the final user-journey check.

### Product lifecycle findings from the performance UI report — repaired for the reported triggers

Inspected `product.svelte.ts` import/page/search/reveal/release methods, actual Overview provider registration and reveal, `findProviders.ts` unregister, `FindInPage.svelte` search/drop/navigation, and `productTranscriptLifecycle.test.ts`.

- **R3-UI-01:** Successful import now awaits a fresh coherent first summary page rather than prepending and truncating against an old cursor. The named page request sequence invalidates pre-import responses. Off-page search preserves the complete ordinary page separately and release restores it; a successfully loaded page replaces that saved model. The 50/51/100-row and older-page tests assert reachability through actual store calls.
- **R3-UI-02:** Registry unregister invokes the provider's release exactly once on removal. Product release invalidates its search generation and clears retained result rows, so the already-pending page may finish but cannot start its next cursor GET or republish old rows. The source tests use the actual registry cleanup and search loop, including rejection.
- **R3-UI-03:** New search and close invalidate navigation and abort the search lifetime. That signal reaches Overview reveal; its post-body-await check additionally binds provider reveal generation and selected story before expanding. FindInPage checks lifetime before its later element/highlight/scroll work. Release restores the ordinary summary page after a canceled off-page reveal. The tests invoke the actual Overview provider and find navigation with controlled body/DOM boundaries, including the normal reveal control.

Root's VERIFICATION ledger records **34/34**, **563 ms**, for Product lifecycle/loading, async find and shared diff tests. These are transport/DOM-adapter results, not mounted Svelte or backend HTTP acceptance. No backend transcript/publication/metadata decision is made in this follow-up. Current full-matrix typecheck/render/native/performance and later formal rescore remain root-owned.

### Grid-only edit follow-up — R4-R3-UI-03 resolved

Inspected the exact fingerprint repair and regression source without running tests/builds. `onSceneChange` now includes normalized `gridSize ?? 20` and `gridModeEnabled ?? false`, matching the persisted subset and installed API defaults. A mode-only or size-only edit therefore changes the fingerprint and reaches the existing dirty/debounce path; viewport movement remains excluded. Baseline and programmatic-load suppression remain intact.

The two added regressions execute actual `onSceneChange` → captured timer → `commitPending` → `snapshotDoc`, assert staged grid values, and first prove viewport-only movement creates neither dirty state nor a timer. Root reports **0/2 RED**, **245 ms**, followed by full `canvasRecovery` **11/11 GREEN**, **281 ms**. The reported grid-only autosave trigger is resolved. No new defect found in this narrow follow-up; mounted persistence/reload and later integrated acceptance remain separate. No score change or reviewer execution claim.

## Backend/workflow and adjacent Design recheck

**Verdict: Request changes for the new workflow publication integration.** One major approval-binding finding and one minor template/documentation migration finding below. Product source publication binding, transcript backend shape and the Design adjacent-writer repairs satisfy the specifically traced requirements. This is a read-only checkpoint; no builds/tests executed and no score raised. New workflow executor/UI tests and repaired Design adjacent cases remain unrun at this review checkpoint.

### R4-R3-BE-01 — major: approving a stale banner can approve a replacement preview in the same run

**Locations:** `ui/src/modules/workflows/WorkflowsPage.svelte:1258`–1269; `crates/otto-server/src/routes/workflows.rs:1633`, `:1685`, `:1699`–1713; downstream trust check `crates/otto-server/src/workflow_product_publish.rs:157`–180.

**Intended:** The person approves the exact publication content/destination displayed by the pending banner. The new live-node check must bind that human decision to the reviewed snapshot, not merely to whichever snapshot currently occupies that node.

**Confirmed by source trace:** Client A fetches/displays publication preview P1 for run R, gate G. Another client rejects G, the run settles, and an editor changes the story body to P2. The supported `retry_run_node` path with `include_downstream:true` permits retrying settled preview nodes (`routes/workflows.rs:583`–632), reuses R, and executes preview→G again. Client A has not yet received/refetched the newer progress and still displays P1. Its Approve sends only `{node_id:G, approved:true}`. `ApproveRunReq` has no expected preview identity. Server now sees R running/waiting at G, so both the node-ID check and SQL predicate pass and approve P2. The human node returns P2 and the driver persists its successful output. Live publish then compares P2 against that same persisted P2 and publishes it: all new same-run/approved-by/output checks succeed, although this human saw P1. No forged input or permission bypass is needed. Missing a WS/poll update before a click is sufficient; a pending request delayed until the second gate presents the same mismatch.

The lazy loader's `detail_version` check only verifies its GET against the summary it was asked to load; it is not sent to or atomically checked by the approval mutation. The summary already exposes a node-body SHA version (`otto-state/src/workflow_progress.rs:75`), so there is an available identity to bind.

**Smallest fix:** For approval of a gate containing `publication_preview`, require the identity of the exact fetched preview (a canonical preview digest or expected node `detail_version`) in the approval request. Validate and record the decision atomically against that same pending gate and identity; use a transaction/CAS that also checks node ID and pending lifecycle so a read followed by an unqualified UPDATE cannot approve a newer snapshot. Wire the UI to submit its successfully loaded identity, invalidate/reload on 409, and update the narrow request contract/types. Preserve ordinary non-Product gate compatibility; this does not require redesigning every generic approval.

**Regression:** Use the real approve HTTP handler plus engine/repository state: A reads P1 → reject/settle → same R retries preview/G with P2 → stale P1 approval returns 409 and sends nothing externally → current P2 approval succeeds → live node sends exactly P2 to isolated Jira/Confluence. Assert destination identity too. Keep a normal non-Product approval control. Current new tests call actual node arms but simulate approval by directly writing SQL and manually persisting the gate result; therefore they cannot cover this missing HTTP/request boundary or the full driver's retry integration.

### R4-R3-BE-02 — minor: shipped Product template and feature guide still promise the removed dry-run behavior

**Locations:** `crates/otto-server/src/routes/workflows.rs:1071`–1077 and `:1101`–1112; `docs/features/workflows.md:368`, `:1376`; new required preview parameters at `workflow_product_publish.rs:96`–135.

**Confirmed trace:** The `po-lifecycle` template describes providing `story_id` as sufficient to persist/publish, emits its final Product Publish with only `{kind:"rfc",dry_run:true}`, and places its review gate before that preview. The old feature table calls dry-run a no-op note and says only live publication needs account/destination. Under the new executor, even after giving the Product Publish node a valid story ID, its absent `account_id` fails with Conflict before generating the promised preview. Following the old caveat by toggling `dry_run:false` and supplying account/space also fails, because the graph never produced the required preview→approval handoff. This is an actionable compatibility migration mismatch, not a request to weaken the new fail-closed behavior.

**Fix:** Update the template description/configuration guidance and graph to expose the new destination-configured preview→Human Approval→live-publish flow, with explicit account/space prerequisites. Keep outward publication gated/default-safe. Update the feature table and limitations to the authoritative API migration text instead of describing a no-op preview or direct live params. Verify that a configured copy of the template can reach a review banner and, in an isolated fixture after actual approval, its intended live step.

### Product backend and UI contract disposition

- `publication_snapshot` captures story metadata plus preferred full version/body; `reviewed_publication` compares version ID, exact body SHA and title/source kind/URL before returning those captured values. Both outbound methods then use the captured body/title/reference, rather than rereading after awaits. Mutable same-version body changes therefore fail; absent review also fails closed. HTTP routes retain workspace Editor and account authorization before service publication.
- The direct PublishDialog computes exact UTF-8 SHA, freezes reviewed metadata/body, includes identity in both typed requests and invalidates it on 409. The new workflow preview additionally freezes `reviewed_account_url`; its live service compares that against the very account object used to build the outbound client. **That URL field is optional and direct PublishDialog currently does not send it.** Do not claim universal account-base-URL binding for direct HTTP/UI callers; only workflow preview requests supply that additional destination check. Title override is a request field for RFC, while the reviewed story title/reference remain bound.
- Summary paging is metadata-only with `(created_at,id)` descending, limit+1 detection and an exclusive cursor preserving raw timestamp spelling. Empty search pages may carry a cursor, and the UI search loop continues them. Search counts literal non-overlapping lowercased title+body occurrences, caps scan rows and match metadata, and honors owning-story Viewer authorization; individual body authorization resolves the story ID first. UI summary/body/search field names and cursor use agree with these backend shapes. Legacy full-list compatibility is retained deliberately; this is not a claim that every legacy read is bounded.
- Existing root evidence: Product HTTP **24/24**, publication/recovery/destination UI **17/17**, both before this new workflow path. Those results certify their named fixtures, not the new engine/approval integration or representative runtime performance.

### Engine persistence/order, UI review and resume disposition

The normal driver writes successful preview output before starting its gate, `expose_pending` writes the exact gate input before setting `waiting_approval`, and the successful human output is persisted before the live successor. Single-upstream `assemble_input` forwards the full object. The live node compares against durable successful gate output and the pinned definition, rejects conflicting live params, checks story workspace/editor/account authority again, and uses the frozen typed request. These are substantive improvements; the major finding is the earlier human-decision binding seam.

Restart classification re-enters a waiting human gate and rebuilds its input from retained upstream outputs; a product_publish caught mid-flight is not restart-safe and is not automatically replayed. Retry operates on the same run with its pinned graph, which is precisely why the stale-client regression above must include retry. Arbitrary loops/multi-input transforms and every failure/retry permutation were not certified. New tests exercise individual real executor arms and the real repository, but use a direct SQL decision rather than the HTTP approval route or full run driver. Approval-preview UI loading is scoped to run/gate/request and checks the detail version on GET; mounted polling/abort/approval behavior remains unexecuted.

### Design adjacent writers — repaired in source for their three RED triggers

Inspected original metadata/approval lock additions, importer `stamp_source`, thumbnail publication and all whole-metadata write callers found under `otto-design/src`.

- `update_meta` and `approve` acquire the canonical-root/artifact lock before reading and retain it through publication, preserving independent acknowledged updates across service aliases.
- `stamp_source` acquires that same lock, reloads the current row, then updates only the import-stamp keys in the freshly read metadata before writing. The stale importer snapshot no longer restores old title/tags/status/thumb/custom import metadata.
- `set_thumbnail` locks by canonical root/artifact, reloads durable state before no-op comparison and old-thumb reference checks, then publishes. The same-artifact A→B→A stale snapshot no longer incorrectly skips restoration; alias writers participate in the boundary.

Root's broader Design run was **125 passed / 3 failed / 1 ignored**: original metadata tests passed, these three adjacent tests were RED. Their current source repairs have not yet been centrally rerun, so this is source disposition, not full Design acceptance. Cross-artifact blob-GC races and broader import/content conflict policy were outside this bounded recheck.

**Scope/evidence limits:** Read the changed Product service/HTTP/state/types, publication UI/type contract, workflow child modules/tests, real engine persistence/assemble/resume/retry/approval paths, WorkflowsPage preview/approval wiring, authoritative API migration text and the cited stale feature/template surfaces, plus Design service/import/store write paths. No remote user systems, app sessions, builds or tests were touched. New workflow source/test execution, Design follow-up GREEN, integrated typecheck, actual rendered human review, native behavior and performance measurements remain pending; formal rescore awaits that matrix.


## Narrow backend follow-up — approval identity and template migration

**Disposition:** R4-R3-BE-01 and R4-R3-BE-02 are addressed in the inspected source. Backend execution acceptance remains pending the central run; this is no score increase and no broader re-audit.

### R4-R3-BE-01 — source repair accepted; backend GREEN pending

`crates/otto-state/src/workflows.rs:1198` now reads the run revision, lifecycle, pending gate and that gate's node body in one SELECT. A Product `publication_preview` requires `expected_detail_version` for both approval and denial (`:1237`–1250). The full node is parsed into `serde_json::Value` and serialized before SHA-256, matching the HTTP detail body's Value serialization/hash in `crates/otto-state/src/workflow_progress.rs:19` and its detail response. UTF-8 `to_string` bytes and `to_vec` have the same JSON encoding here; SQLite's extracted JSON is reparsed, so raw storage whitespace/key order is not used as the identity.

The decision UPDATE at `workflows.rs:1254` matches the captured revision, running status, waiting flag and same gate ID. Therefore a replacement preview or any intervening revision update between SELECT and UPDATE makes the CAS fail; two concurrent decisions cannot both succeed. It modifies only decision fields and revision, rather than rewriting an old node snapshot. The authenticated route delegates to this repository operation after workspace authorization. Ordinary non-Product requests without the optional identity retain their previous shape; an explicitly supplied identity is checked.

`ui/src/modules/workflows/WorkflowsPage.svelte:1259` captures the version returned by the successful full-node preview GET, rather than substituting a newer summary version. Unread/failed/loading Product previews cannot submit. Both approve and deny send that captured version. Preview loading remains run/gate/request scoped; a 409 invalidates the pending displayed identity and refetches only when the same run/gate remains selected (`:1275`). A departed run's rejection cannot invalidate the newly selected run's preview. These checks close the traced P1 → rejection/retry → P2 → stale P1 approval path without weakening ordinary approval.

The new real-engine/API regression in `crates/otto-server/src/workflow_product_publish_tests.rs` exercises rejection, same-run downstream retry, a changed body, stale and missing identity rejection, current identity success, and exact outbound body/destination for Jira and Confluence. The ordinary API control remains at `:518`. Parent reports actual stale-request **200 versus expected 409 RED** before this fix and actual UI **8/8 GREEN** after the UI repair. I did not execute either suite; the current backend GREEN was pending at this checkpoint.

Fixture correction is suitably narrow: `workflow_product_publish_tests.rs:12` requires exactly one POST to the publication endpoint. Every other Confluence request must be a property POST on the created page, with exactly the expected two-field payload and `full-width` value; the sorted property keys must equal `content-appearance-draft` and `content-appearance-published` exactly. Jira permits no such extras. This admits the service's two intended Confluence property writes without allowing duplicate publications, duplicate/missing properties or arbitrary additional requests.

### R4-R3-BE-02 — source migration accepted

The `po-lifecycle` template in `crates/otto-server/src/routes/workflows.rs:1071` now explains the story/account/destination prerequisites and connects Refine → configured dry-run Preview → Human Approval → live Product Publish. The live node consumes the approved handoff; supplying account configuration remains an explicit setup requirement. The feature table and limitations in `docs/features/workflows.md:368` and `:1376` now describe a real preview and the required migration rather than the removed no-op behavior. `docs/contracts/api.md:1478` and `:3231` document the exact displayed detail identity, approve/deny requirement, CAS and conflict behavior. The template graph regression is at `routes/workflows.rs:2146`.

**Coverage limits:** This follow-up inspected the repository decision, HTTP detail hashing/approval contract, route authorization/delegation, UI preview/decision scopes, template/guide migration and strict outbound fixture assertions. No tests, builds, servers, remote writes or source edits were performed. Central backend GREEN, mounted UI behavior and the full acceptance matrix remain separate evidence requirements. No additional defect was established within the two assigned findings.


### Central execution update after the narrow source checkpoint

Root subsequently reports **30/30 server `review4_` GREEN** (1m19s compile /15.56s tests), now recorded in VERIFICATION.md. This includes the actual P1 rejection → same-run P2 retry → stale identity409 → current identity successful exact Jira/Confluence publication, ordinary approval compatibility and migrated template. R4-R3-BE-01/02 therefore have both the inspected source disposition and named backend regression GREEN. Root also reports the complete Design library **128 passed /1 ignored**, including original and adjacent metadata/importer/thumbnail repairs. These supersede the specific pending backend GREEN entries above; rendered UI, native and full affected-consumer acceptance remain separate. No reviewer tests/builds were executed.
