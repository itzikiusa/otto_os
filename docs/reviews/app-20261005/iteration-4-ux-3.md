# Iteration 4 — UX partition 3

**Verdict: Block.** New findings: one major, two minor. Provisional **7.4/10**. Reviewed source at `c5d4e999767fca39db497b6eb444e4eaaa04558d` (documentation over baseline `03f2bc3e`), with unrelated role-1 working edits excluded. Scope: Vault, Canvas, Design Hall, Product, Browser and Snip.

Read AGENTS.md, PLAN.md including the fixed rubric/execution calibration, design patterns/content/components, partition-3 correctness/performance findings, Claude's consolidated design findings and current coordination. This is a bounded source review and hand trace, not an executed reproduction. No builds, tests, servers, source edits, commits or external actions were performed. Only this report was written.

## New findings

### R4-UX3-01 — Major: Publish can send a newer version than the user reviewed

**Locations:** `ui/src/modules/product/PublishDialog.svelte:83`, `:192`, `:202`, `:214`; `crates/otto-product/src/service.rs:1314`, `:1564`, `:1650`; request definitions `crates/otto-product/src/types.rs:303` and `:313`.

**Source-traced reproduction:** Open Publish for a draft whose preferred suggested version V1 contains “Internal review only.” Accounts/destination and preview load successfully. Leave the confirmation open while another window or an already-running agent creates suggested V2 with different content. Click Publish. The dialog retains V1's `previewBody`; its effect depends on selected story identity, not a content-version identity. Its submit guard checks only successful preview ownership by story ID. The request carries destination fields but no previewed version ID or content fingerprint. Both publish service methods independently call `best_content_version` at submission and select V2, then send V2 to Jira/Confluence. The displayed “What is published” therefore differs from the actual outward write even though the preview GET succeeded.

**Why it matters:** A person approves content that is subsequently replaced by an unreviewed revision before publication. This breaks the explicit what/where/who confirmation contract. The historical failed-preview/loading guard remains fixed; this is a different trigger involving two successful versions of the same story.

**Fix direction:** Bind the confirmation to an immutable publication payload/version. Send the previewed version ID (and bind title/reference metadata that affects the outbound payload), validate its ownership, and publish exactly those reviewed bytes; alternatively reject a changed publication snapshot with 409 and require a refreshed preview and another explicit Publish. A client-only refresh just before submission leaves a race. Update Rust request types, TypeScript types and API contracts together. Preserve a valid intentionally empty preview.

**Verification idea:** In an isolated fixture, open the dialog on V1, create V2 before submission, capture the outbound Jira/Confluence client payload and prove it is V1 or no external request occurs until re-review of V2. Include draft/source/suggested precedence and RFC-to-Jira reference text. The existing `ui/unit/knowledgeRecovery.test.ts:82` checks preview failure/retry only; it does not exercise content changing after preview success. No real remote publishing is needed for the regression.

### R4-UX3-02 — Minor: A failed destination lookup has no in-place retry

**Locations:** `ui/src/modules/product/PublishDialog.svelte:142`, `:177`, `:271`, `:319`, `:388`.

**Source-traced reproduction:** Configure exactly one Jira/Confluence account. Open Publish and let accounts load successfully, then fail the projects request (story mode) or spaces request (RFC mode) once. The catch sets `formError`; the destination select contains “No projects found”/“No spaces found” and is disabled because the array is empty. Only the account select's `change` handler can rerun the lookup. With one account there is no other selection to change to. The visible error has no Retry. “Retry preview” fetches content only and the accounts Retry exists only when accounts themselves failed. Restoring connectivity does not recover this dialog: the practical workaround is Cancel and reopen, losing any entered RFC title/parent fields.

**Why it matters:** A recoverable lookup failure interrupts a nearly-completed publish task and requires restarting the form. This is a behavior/recovery gap, distinct from Claude's copy/style work.

**Fix direction:** Retain a scoped destination-load error and expose Retry for the current account/mode. Clear the corresponding error on retry success, preserve form fields, and distinguish a successful empty list from a failed request. Fence retried responses to their initiating account/mode so recovery cannot publish stale destinations.

**Verification idea:** With one account, fail projects/spaces once, assert an inline Retry remains available, then succeed and verify destination options load without closing the modal or clearing RFC fields. Cover an actually empty destination list separately and switching accounts during the retry. No execution claimed.

### R4-UX3-03 — Minor: A rejected Canvas assistant request erases the retryable prompt

**Locations:** `ui/src/modules/canvas/ConversationPanel.svelte:49`; `ui/src/modules/canvas/ExcalidrawCanvas.svelte:219`; `ui/src/modules/canvas/MermaidCanvas.svelte:270`; `ui/src/modules/canvas/D2Canvas.svelte:267`; request at `ui/src/lib/stores/canvas.svelte.ts:482`.

**Source-traced reproduction:** Open a new Excalidraw scene with no assistant session. Enter a detailed drawing request and make the `/canvas/scenes/{id}/assist` request fail before an agent is started. `send()` copies the prompt to a local variable and clears `draft` before awaiting `editor.generate`. Excalidraw's generate catches the failure, toasts it and resolves `Promise<void>` normally. The composer becomes enabled again but is blank; it exposes no Retry/restore action and no agent terminal exists containing the prompt. Mermaid and D2 use the same composer and swallow failures too. They append the request to `canvas.convo`, but this assistant renders a Terminal/empty state, not that array, so it does not provide a visible recovery path.

**Why it matters:** A transient request failure forces the user to reconstruct their detailed instruction. This concerns an unsubmitted request, not undoing an agent edit or restoring an Excalidraw background (already correctness-owned).

**Fix direction:** Return a success/failure result or propagate failure from generate, and clear the composer only after acceptance or restore the submitted text on failure. Scope restoration to the initiating scene/composer so a late failure cannot insert an old request into another canvas or overwrite a newer draft. Keep the failure feedback visible and avoid automatically resubmitting ambiguous network outcomes.

**Verification idea:** For each editor format, fail assist before dispatch, verify the complete prompt remains available, retry successfully and assert exactly the intended prompt is sent. Also switch scene or remount the assistant while a request is pending and verify failure does not overwrite the new scene's draft. No provider or renderer execution was performed.

## Coverage and existing ownership

| Journey | Source evidence / bounded conclusion | Execution evidence inspected |
|---|---|---|
| Vault search → note → edit/save → leave | `SearchPanel.svelte:22` distinguishes not-yet-submitted query, searching, failure with Retry and empty results. `vault.svelte.ts:1157` fences query responses; `:851` drains edits made during save and retains dirty state on error; `:430` permits leave because local drafts are retained. No additional defect asserted in these sampled paths. | Existing baseline only; no search/save/conflict journey run by this reviewer. |
| Vault trash/history → inspect → restore | `RecoveryView.svelte:63` binds revision detail to vault identity; `:98` sends current hash on restore. `:115` supplies refresh/retry. Inspected restore test at `ui/e2e/desktop-vault-recovery.spec.ts:25` asserts final note bytes, not just a success toast. | Test source read; not a new pass. Full failed-restore/paging/native matrix remains unverified. |
| Canvas select → edit → save failure → recover; assist/restore | `CanvasPage.svelte:184` exposes retained draft and Retry save; `:230` preserves the open board after another scene fails to load. `ConversationPanel.svelte:84` confirms restore and checks original scene after choice. New assistant retry finding above. | `ui/e2e/desktop-canvas-versions.spec.ts:15` tests large save and Mermaid restore, not new prompt failure or Excalidraw app-state fidelity. |
| Design Hall create → save → conflict → compare/restore | `ArtifactView.svelte:114` installs dirty guard; `:345` retains changed source when save fails; `:384` preserves edits if latest-version lookup fails; `:438` requires clean source before version restore and explicitly preserves history. | `ui/e2e/desktop-design-hall.spec.ts:97` source covers create/edit/version/compare/restore; `knowledgeRecovery.test.ts:93` covers keep-mine lookup failure. Neither executed here. |
| Product draft → publish preview → external publication | Traced complete preview, destination lookup, submit request and server body selection. Existing pending/error preview guard retained; three-state preview is useful but lacks content-version binding. | `knowledgeRecovery.test.ts:82` source checks failed-preview retry. `desktop-product-edit.spec.ts:290` covers draft body reload, not publication snapshot stability. |
| Browser reader → annotate → send/save to Vault | `ReaderView.svelte:71` resets a composer for changed page ownership; `:151` captures the page/selection for save. Existing load/retry and empty-state callbacks inspected. Browser creation/title races remain correctness-owned. | `desktop-browser-reader.spec.ts:74`, `:88`, `:112` source covers mark feedback, chosen-session request and Vault save context. Native/CDP journeys not executed. |
| Snip annotate → persist/copy → close/error recovery | `SnipEditor.svelte:263` distinguishes persistence from clipboard result; `:290` drains newer annotations; `:300` offers Retry saving/explicit discard; router/native close paths use that guard. Previous failed-save leave defect remains closed. | `desktop-snip.spec.ts:65` verifies annotated PNG bytes/dimensions in a fixture sink; `knowledgeRecovery.test.ts:62` and `:71` cover failed-save leave and pending-save drain. Native capture/clipboard not executed here. |

Do not duplicate existing repairs: R4-C3-01 metadata lost update (blocker); R4-C3-02/03 browser workspace/title ownership (major); R4-C3-04 Excalidraw background/grid restore (minor); P3-01 eager transcript bodies and P3-02 large diff output (major). They still affect task completion, state trust and responsiveness until independently repaired. Claude owns visual/accessibility/copy, Product URL routing and Plan/Rewrite Stop; no competing source edits or separate findings for those items. External provider, Jira/Confluence and native browser dependencies remain acknowledged acceptance limits.

## Fixed-rubric score

| UX dimension | /2 | Evidence and deduction |
|---|---:|---|
| Task completion/discovery | 1.7 | Sampled creation/search/restore paths offer concrete next actions; destination failure interrupts publishing, Canvas failure requires prompt reconstruction, and existing browser ownership/large-content findings remain. |
| Feedback/state clarity | 1.7 | Preview pending/error, retained Canvas draft and Snip persistence outcomes are explicit; failed destination lookups still render empty choices and existing wrong Browser titles undermine state meaning. Full runtime state matrix absent. |
| Recovery/retry | 1.6 | Vault and Snip have substantive recovery paths and Design Hall preserves failed conflict drafts; destination retry and failed assistant-prompt recovery are missing, with known Excalidraw restore gap. |
| Draft/scope/trust preservation | 1.4 | Significant concrete gap: successful publish preview does not bind what leaves the Mac. Existing metadata loss/browser ownership also remain; sampled local draft guards provide useful protection. |
| Executed end-to-end journeys | 1.0 | Verified shared green baseline `03f2bc3e` credited under PLAN.md calibration, without a current named journey/result mapping. Reading tests is not running them; no new passing result claimed. |
| **Total** | **7.4/10** | **Provisional; major/blocker findings prevent 9.8 acceptance.** |

Initial score already applies execution calibration. Preserve it and append repair/execution results rather than replacing it. Confidence is high in the stated source traces and bounded elsewhere. Omitted: actual provider-assisted creation, external publishing, native capture/clipboard/CDP, physical keyboard/VoiceOver, rendered light/dark/phone/tablet acceptance, all cross-window permutations, long-content latency and exhaustive module coverage.

Root's next execution matrix should map named results for stale publish preview in both destinations; one-account destination failure/retry; all three Canvas format failed-prompt retry and scene-switch ownership; existing metadata/browser/restore repairs; and representative happy/failure journeys listed above. A green aggregate gate alone does not certify that matrix.
