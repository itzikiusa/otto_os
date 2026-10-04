# Iteration 1 — UX partition 3

**Scope:** Vault, Canvas, Design Hall, Product, Browser, and Snip. Focused source review of recovery, save/close outcomes, and publishing. **Result: 3 major findings.** These are source-traced scenarios, not runtime reproductions.

Read `AGENTS.md`, the design guidelines README, components and patterns guidance, correctness/performance partition 3 reports, and `/tmp/otto-app-review-coordination-20261004.txt`. Compared Claude's design worktree changes against `a16f4c71`; none of the three finding locations below is in that design diff. Existing Product draft navigation/response ownership, ArtifactView missing-head conflict handling, Browser annotation ownership, Snip unchanged-copy, and Vault rename findings are not counted again. No builds, tests, servers, source edits, or external mutations were performed.

## UX3-01 — Major: Failed publishing preview is presented as an empty body while Publish remains enabled

**Evidence:** `ui/src/modules/product/PublishDialog.svelte:80`, `:87`, `:363`, `:392`; `crates/otto-product/src/service.rs:1308`, `:1558`, `:1644`, `:1657`.

**Scenario:** Open Publish for a draft containing a body. Let the version-list or body-preview GET fail while account/project loading succeeds. `loadPreview()` catches that failure and assigns an empty string. The confirmation then says “No body — only the title is published.” Publish is enabled because its disabled predicate checks account loading and submission, not preview readiness or failure. Clicking Publish calls the daemon, which independently reads the actual preferred version and sends its full body to Jira or Confluence. A failed preview therefore changes what the person believes they are authorizing without changing what is sent. Publish is also available while the preview is still loading.

**Smallest fix:** Keep separate loading, loaded-empty, loaded-content, and error preview states; show an inline retry for the failed preview and prevent submission until the preview is successfully loaded. Check the same predicate in `submit()`, not only the button. Do not translate a failed read into a claim about the outgoing content.

**Verification:** In an isolated UI test, fail both preview stages in separate cases and assert an error/Retry, no “only the title” claim, and no publish request. Resolve Retry with body content and verify publication becomes available. Also delay the preview and verify Publish stays unavailable; a successfully loaded genuinely empty body should remain publishable.

**Limit:** This trace uses stable story content. Binding a preview to a version when another writer changes it during confirmation is a separate contract consideration, not a proven additional finding here.

## UX3-02 — Major: Brand Kit “Save mine on top” cannot complete after a normal conflict

**Evidence:** `ui/src/modules/design-hall/brand/BrandEditor.svelte:244`, `:272`, `:275`, `:293`, `:309`.

**Scenario:** Edit a kit based on v1 while another writer creates v2. Save receives 409 and awaits `resolveConflict()` while `saving` is still true. The version lookup succeeds and the user selects “Save mine on top.” That branch awaits `save(latest.id)`, but `save()` immediately returns at line 245 because the outer save has not reached its `finally` yet. The dialog closes without creating a version or explaining the failed action. Pressing Save again uses the unchanged v1 base and repeats the conflict loop. The advertised recovery path never succeeds even when the network is healthy.

**Smallest fix:** Handle conflict resolution within one save operation: capture the chosen local text and retry the commit with the resolved base inside that operation, or return the chosen retry base and start a new save after the busy state is released. Preserve duplicate-click protection. Only mark saved after a successful response.

**Verification:** Stub first commit to 409, history to v2, choose Save mine on top, and assert a second commit sends the preserved local text with base v2, then clears dirty state on success. On retry failure retain the draft and expose the error. Repeated clicks must not create concurrent commits.

**Dedup note:** Line 310 also reloads/discards edits when the head lookup fails, matching C3-02's ArtifactView issue. That duplicate is not an additional finding; the parent was notified to extend the existing fix to BrandEditor. The successful-lookup no-op above is the uncovered behavior.

## UX3-03 — Major: Closing Snip after a failed save abandons the only editable draft

**Evidence:** `ui/src/modules/snip/SnipEditor.svelte:50`, `:135`, `:147`, `:268`, `:285`, `:290`, `:618`; `ui/src/lib/snip.ts:39`.

**Scenario:** Draw annotations while the daemon is unavailable and let the autosave fail. The annotations remain only in component-local `annos`. Press Close while the daemon remains unavailable. `close()` navigates away or closes the native window without checking pending/failed persistence. Component cleanup can start another `copyNow()`, but it is fire-and-forget; another failed upload leaves no mounted editor in which to retry and no persisted draft. Reopening loads the original PNG and initializes an empty annotation list. The user loses the unsaved annotations through the ordinary Close action without a chance to keep editing or deliberately discard them.

**Smallest fix:** Provide an awaitable save-drain operation and use it before leaving when annotations differ from the saved hash. Keep the editor open if persistence fails, with Retry and an explicit discard choice. Distinguish persistence failure from clipboard-only failure so a saved image does not unnecessarily block close. Apply the same policy to native close requests and router navigation, or retain a recoverable draft beyond component teardown.

**Verification:** Draw an annotation, fail its annotated-image POST, then press Close. Assert the editor/draft remains and retry is available; restore the endpoint and verify retry persists before closing. Verify explicit discard exits. Delay an in-flight save and verify close waits for the newest annotation state. Native window teardown requires a desktop check; it was not exercised here.

**Dedup note:** C3-08 concerns explicit recopy of already saved, unchanged content. This finding concerns preservation/recovery of content whose save failed; fixing the copy endpoint path alone does not address it.

## Coverage and limits

- **Vault:** Sampled history/trash loading, revision selection, hash-based restore, and recovery outcomes. No additional finding asserted; existing rename and backlink work remains covered by the other reports.
- **Canvas:** Sampled selection, phone back-to-list save behavior, retained-draft recovery controls, and legacy toolbar export. No additional finding asserted. Full export/import fidelity and native renderer behavior were not executed.
- **Design Hall:** Sampled site publish/export sheets and Brand Kit save, impact, approval, export, and conflict handling. Existing ArtifactView loss is deduplicated above.
- **Product:** Traced the publish preview through the backend's actual content selection. Attachment flows were sampled; cross-story ownership is already assigned and is not reported again.
- **Browser:** Sampled credential editing/reveal and send-to-session outcomes. Visual/error-state work in Claude's worktree and correctness annotation ownership are excluded.
- **Snip:** Traced failed save, cleanup, explicit close, and reopen inputs. No claim about measured worker/window cancellation timing or clipboard behavior was made.

All proposed verification cases remain to be run by implementation/verification owners. This finite pass does not certify the entire six-module surface.
