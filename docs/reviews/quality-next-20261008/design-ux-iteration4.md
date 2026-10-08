# Independent design and usability — iteration 4

**Verdict: approve the inspected UI repairs, conditional on final integrated validation. Zero new confirmed production findings; one confirmed native-probe evidence gap.** Base `7cff9e538deccdd0e346af7c6285e11863285c00`, branch `review/quality-20261008-next`, with concurrent campaign changes in the shared worktree. This report does not certify an integrated final commit.

## Native modal investigation

No production race is confirmed by the available evidence. The first iteration-2 School modal attempt observed `visible=false` after Cancel and timed out. That failed helper had no host/store diagnostics. The subsequent diagnostic-only helper passed three cycles; no production fix occurred. The first failure is still unexplained and must not be presented as repaired, disproved or established to be a fixture failure.

The independent source trace covers `NewWorkspace.svelte` Cancel → `App.svelte:1360` clearing `ui.newWorkspaceOpen` → `Modal.svelte` unmount releasing its overlay count → `SidePane.svelte:73` observing the changed overlay state → `:45` reading current showing/readiness/occlusion at the animation frame → `:53` queuing the latest visibility. `LatestPaneLayout` (`nativePanePolicy.ts:37`) serializes sends and retains the newest pending state. `nativePane.ts:44` further orders native mutations; `panes.rs:283` holds the native action lock while applying and publishing visibility. A pending hide followed by show cannot simply overtake the show in this path. Nested modals correctly remain occluding until their own teardown. Geometry changes and native state events reschedule layout. School rendering separately requires both the pane's published visibility and `document.hidden=false`.

This trace does not rule out an IPC failure, a host animation-frame stall, another legitimate occluder, or a native event/visibility fault in the original failed run. Its log did not record enough state to select among those cases. No speculative production edit was made.

**Confirmed evidence gap, not a production finding:** the iteration-2 helper `apps/desktop/src-tauri/examples/probe_support/school.rs:108–115` (retained as `evidence/native-iteration2/school-final.rs.txt`) takes a frame baseline before focusing the host and merely waits for one larger counter after child blur. A frame rendered before blur can satisfy the assertion. The durable green `school-diagnostic.txt` prints `NATIVE_SCHOOL_VISIBLE_UNFOCUSED` with `hidden:true`, so that sample does not establish sustained visible-but-unfocused rendering. The valid three modal hide/resume cycles and identity/room checks remain separate evidence. A stronger future probe should wait for native-visible, document-visible and unfocused state, take its baseline after settling, and observe fresh frame growth over a bounded interval. Original evidence and assertions are preserved. The coordinator subsequently strengthened the helper to establish document-visible/unfocused state before taking a settled baseline and checking further frame growth. That new native execution is coordinator-owned and was not run or scored by this reviewer.

## Changed production surfaces

Independently inspected PageHeader focus transfer, including the guard against stealing focus from another editor, Shell workspace Retry and token/generation ownership, and Proof workspace-readiness/deep-link gating. Workspace store ownership guards are distinct from shell error-publication guards. No additional confirmed defect was found in those changes during this static pass.

## Execution and visual evidence

Executed after explicit release of the uncontended performance lease. Browser slot `next-design4`, isolated daemon **17828**, Vite **5198**, one Chromium `desktop-browser` worker, Node 26, orphan sweep disabled. No installed app, live port 7700, real database or outward provider write was used. The baseline daemon SHA-256 is `d7e61497d0aa8318793985c67aded624797547689fc5e235f44a479c4786bb0f`; it predates concurrent backend repairs. Current UI source and daemon hashes are retained in [manifest.json](evidence/design-ux-iteration4/manifest.json). Ordinary coordinated Cargo work overlapped this functional run; no timing/performance conclusion is drawn.

**28 passed, zero failed/skipped/retried**, one run, [browser.log](evidence/design-ux-iteration4/browser.log) and [machine-readable result](evidence/design-ux-iteration4/browser-results.json). Exact selection:

```sh
PATH=/opt/homebrew/bin:$PATH OTTO_E2E_SLOT=next-design4 OTTO_E2E_PORT=17828 OTTO_E2E_PW_PORT=5198 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod npx playwright test desktop-quality-next-populated.spec.ts desktop-quality-next-design.spec.ts desktop-page-chrome.spec.ts desktop-db-grid-keyboard.spec.ts desktop-db-error-panel.spec.ts desktop-api-tabs-persist.spec.ts desktop-git-hunk-staging.spec.ts desktop-git-conflict-resolver.spec.ts --project=desktop-browser --workers=1
```

Coverage is behavior-specific:

- **Database:** real UI with mocked engine responses; populated query result, Enter commits one cell draft, Escape retains its earlier value, Review shows the intended statement without executing UPDATE, phone-size modal controls remain reachable, Cancel preserves pending edits, result rerun preserves scrolling, MySQL suggestion edits the query and PostgreSQL caret points to the reported column.
- **API:** actual isolated workspace/tab persistence; method, URL and header drafts survive reload; cancellation retains a dirty tab; confirmed discard keeps it closed; workspace drafts stay separate; corrupt persisted payload recovers. New light/dark desktop→phone journeys verify that Cancel retains method/URL/header content. These are populated draft workflows, not HTTP transport or populated response-body coverage.
- **Git:** real disposable local repositories and daemon Git operations; mixed-side conflict resolution, partial hunk staging/commit leaving the other hunk dirty, one-hunk discard with backup, CRLF staging and stale-token rejection. Assertions inspect actual Git bytes/index where supplied by existing fixtures. No remote push or forge mutation.
- **Current repairs:** three-theme header focus/action continuity and editor-focus negative control; workspace failure→Retry; stale identity failure suppression; Proof reload and delayed-workspace deep link. The all-route header sweep is shared-chrome coverage only and is not counted as populated coverage of every module.

New regression ownership: only `ui/e2e/desktop-quality-next-populated.spec.ts`. All four cases passed on their first execution. `npm run check` passed with **0 errors, 0 warnings** ([ui-check.log](evidence/design-ux-iteration4/ui-check.log)). `node --experimental-strip-types --test unit/nativePane.test.ts` passed **9 tests** ([native-policy.log](evidence/design-ux-iteration4/native-policy.log)); these policy tests support ordering/focus rules and do not reproduce the original native timeout. No production files changed in this pass.

Independently opened all eight new screenshots: native light/dark database results, phone SQL review, API drafts and phone dirty-tab confirmation. The captures show readable content hierarchy, clear draft/pending status, opaque content/forms, consistent shared chrome, bounded modal actions and no page-wide horizontal overflow. Dense database columns stay within their grid; the phone review keeps its Run/Cancel controls visible and lets the SQL field scroll. This visual review does not measure every text contrast ratio or replace assistive-technology testing.

| Surface | Light | Dark |
|---|---|---|
| Database results | [desktop](evidence/design-ux-iteration4/database-populated-desktop-light.png) | [desktop](evidence/design-ux-iteration4/database-populated-desktop-dark.png) |
| Database review | [phone](evidence/design-ux-iteration4/database-review-phone-light.png) | [phone](evidence/design-ux-iteration4/database-review-phone-dark.png) |
| API draft | [desktop](evidence/design-ux-iteration4/api-populated-desktop-light.png) | [desktop](evidence/design-ux-iteration4/api-populated-desktop-dark.png) |
| API confirmation | [phone](evidence/design-ux-iteration4/api-draft-confirm-phone-light.png) | [phone](evidence/design-ux-iteration4/api-draft-confirm-phone-dark.png) |

## Assessment

Rubric: design considers hierarchy/readability, shared materials/components, responsive control reach and accessible presentation; UX considers intent preservation, keyboard continuity, truthful state, error recovery and navigation/persistence. Scores assess the observed surfaces rather than a pass percentage. A 9+ assessment requires no outstanding confirmed material defect in those surfaces. Near-perfect product acceptance additionally needs integrated-build and representative native/assistive evidence, not simply more empty-state screenshots.

| Vertical | Scoped score | Confidence and limits |
|---|---:|---|
| Design | **9.5/10** | Medium-high on the reviewed shared chrome plus populated DB/API surfaces. Eight new native-light/dark captures and bounded desktop/phone controls broaden iteration 1. No exhaustive per-theme/per-module visual audit or new contrast measurement. |
| UX / usability | **9.4/10** | Medium-high for sampled browser journeys: fresh independent replay of the repairs, preserved DB/API drafts, actual local Git operations, reload/identity/error cases. Medium for native composition because the original modal resume failure is unexplained; no current VoiceOver journey or independently executed new native probe. |

**The requested whole-product 9.8 is not established.** No new confirmed production blocker emerged; the native timeout remains an unresolved observation and the detached-frame claim was under-asserted in the old helper. Other limits: baseline daemon rather than final integrated server; mocked DB engines; no real API-response transport, provider/forge workflow, physical-keyboard end-to-end session, native AX/VoiceOver, long-duration native soak or all-module populated sweep. These are evidence boundaries, not invented product defects. The coordinator owns final integrated gates and the stronger native probe; this bounded pass is complete.
