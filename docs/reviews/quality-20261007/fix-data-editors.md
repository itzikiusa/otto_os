# Data editor repairs

Baseline: `196048df`, branch `fix/quality-20261007`. Covers C1/T1 (Redis list deletion) and UX-01/T6 (environment draft navigation).

## Redis

`ui/src/modules/database/edit-redis.ts:343` now refuses list deletion with `null`. The fixed marker is removed entirely. A marker plus `LREM` could delete unselected matching values anywhere in the list, including outside a partial `LRANGE`, and separate commands could leave partial changes. Safe list value edits still produce `LSET` with the original range offset.

The real UI execution path is `EditFlow.deleteSelected` → `deleteRows` → adapter `buildDelete`. `ui/src/modules/database/EditFlow.svelte.ts:814` only opens the review when a mutation exists. `runReview` returns immediately without a review. The source-harness regression exercises these production methods with the real Redis adapter; only transport and nonparticipating dependencies are stubbed.

An unsupported deletion now shows the shared informational toast explaining that the rows cannot be deleted safely and values can still be edited. Both selected-row and single-row entry points assert that explanation. The delete affordance remains available, but no longer silently does nothing.

`ui/unit/dbRedisLineSafety.test.ts:18` covers markers before/after selection, repeated values and multiple selected indices, partial ranges with a nonzero offset, and a one-row partial range. Assertions require an exactly null operation. A separate assertion preserves the exact safe command `LSET k 41 "replacement"`.

## API environments

`ui/src/modules/api/EnvironmentsView.svelte:47` exports the same `approveLeave` pattern as AutomationEditor, using the shared Save/Discard/Cancel dialog. The component registers `router.guard` at line 64; workspace selection already asks those guards before changing persistence context. Selection and creation ask the same guard. Draft rows, including secret plaintext, remain component memory only.

`save` at line 125 shares an existing pending write and returns a boolean. Failed or stale writes return false. A successful response approves leaving only if the draft is now clean; edits made during the write remain visible and dirty. Existing secret rename/value reconciliation and clean A→B→A pending-save behavior are retained. Create completion checks workspace, editor generation, and dirty state before replacing the form.

`ui/src/modules/api/ApiPage.svelte:78` asks the environment editor before changing views, covering request tabs, New request and close Environments. Header New environment delegates to the component's guarded create method. Explicit environment-id changes remount only after approval.

New browser coverage: `ui/e2e/desktop-api-environment-leave.spec.ts`. Journey 1 cancels request-tab, tab-close, sidebar, environment-selection, in-editor creation, header creation and real workspace-store selection, then verifies draft values and absence of the typed secret from browser storage. Confirmed Discard leaves saved values unchanged. Journey 2 checks failed Save, delayed Save, newer edits during Save, and successful Save before leaving. Existing `desktop-review6-api-environment.spec.ts` retains the two delayed-save/secret-rename cases.

## Verification ledger

All test processes are run by the coordinator to respect the single-heavy-command limit.

| Check | Evidence | Outcome |
|---|---|---|
| Redis red before production edit | `/tmp/otto-quality-20261007-redis-red.log` | Three new refusal cases fail because baseline emits `LSET` + `LREM`; five other cases pass. |
| Redis green, including supplemental real flow test | `/tmp/otto-quality-20261007-contract-redis-green.log` | All 9 Redis cases pass. The combined command has an unrelated telemetry test failure; this is not a claim that the whole command passed. |
| Refusal explanation red | `/tmp/otto-quality-20261007-redis-feedback-red.log` | Real flow assertion fails with 0 explanations instead of 2. Toast implementation then added; final 9-case green is pending. |
| Environment red before production edit | `/tmp/otto-quality-20261007-environment-red.log` | Both new journeys fail on the missing leave dialog after request-tab/Home navigation. |
| Environment green, existing save-race regressions | Coordinator queued | Not yet verified at this handoff. Run both environment browser files, desktop-browser, workers 1 (4 cases). |
| `npm run check` | Coordinator queued | Not yet verified at this handoff. |
| Scoped `git diff --check` | Worker inspection | Passed for modified production files and Redis tests. |

CI selection: Redis tests run under `npm run test:unit` and the UI scope of `scripts/check.sh`; the new `desktop-*.spec.ts` is selected by the desktop-browser functional suite, but is not added to the narrower smoke allowlist. No Docker Redis is required because list deletion is refused before any execution.

UI checklist source review: shared confirmation/toast primitives, no new CSS/tokens or native dialogs, existing labels and responsive layout retained. Fresh light/dark/phone screenshots, browser green and the type-check gate remain coordinator work. No production data, secrets, shared contracts, or API types were changed. Nothing was staged, committed or published.
