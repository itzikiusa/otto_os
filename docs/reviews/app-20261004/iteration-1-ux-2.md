# UX review — partition 2

**Baseline:** `a16f4c71`. **Scope:** Git/workbench, Database/connections/brokers, API client. **Result:** 2 major findings, 1 minor finding.

All findings are **source-only, hand-traced scenarios**. No browser session, build, server, external connection, or test execution was performed by this reviewer. Existing E2E source is supporting coverage evidence, not a claim those tests passed in this review. No screenshots were inspected and no visual-layout or contrast conclusions are asserted.

Read `AGENTS.md` and design guidelines `README.md`, `patterns.md`, `content.md`, and `components.md`. Compared against partition 2 correctness/performance reports and `/tmp/otto-app-review-coordination-20261004.txt`. Excluded their SQL identity, ClickHouse mutation, auto-stash, script-variable race, result-budget, batch-drain, and schema-history findings. Excluded Claude's shared visual/a11y/copy work, DB assistant/insert/close confirmations, and Git draft-overwrite confirmations. Parent confirmed API scratch-close protection is not in that other ownership scope. A read-only comparison with the design worktree found only an unrelated empty-state change in the three affected components.

## UX2-01 — Major: Closing a populated scratch request permanently discards unsent work

**Location:** `ui/src/modules/api/ApiPage.svelte:90`, especially the `requestId` condition at line 92; `ui/src/lib/stores/apiClient.svelte.ts:417`.

**Failed user scenario:** Create a request, spend time entering its URL, JSON body, headers, and pre-request script, but do not save or send it yet. Close its request tab. The tab disappears immediately. Reopening the app cannot restore it, and History contains no request because it was never sent. This is the same nontrivial unsaved work protected when the tab happens to refer to a saved request.

**Existing expected behavior:** `patterns.md` §7 requires confirmation when discarding more than a trivial edit. The API already has a dirty indicator and a discard confirmation for saved-request edits; preserving an empty scratch tab is not requested.

**Evidence — source-only:**

- `ApiPage.svelte:87` explicitly explains that scratch requests close silently because their sends are in History. That rationale does not cover unsent drafts or edits since the last send.
- `closeRequestTab` tests `t?.requestId && apiClient.isDirty(t)` before asking. An unsaved request has no `requestId`, so the prompt is skipped regardless of its contents.
- `apiClient.svelte.ts:1897` already detects meaningful unsaved fields, including body, headers, auth, scripts/settings extras, and URL. The caller deliberately bypasses this detector for scratch requests.
- `closeTab` drops the response slot, removes the tab (or replaces the last one with a blank draft), and persists the reduced set at lines 417–428. There is no closed-tab recovery in this path.
- Existing `ui/e2e/desktop-api-tabs-persist.spec.ts:90` tests that closing a populated unsaved URL tab persists its removal. That is evidence of the current behavior; the test needs to accommodate an intentional discard decision rather than preserving silent destruction. The earlier test in that file establishes that open drafts otherwise survive reload.

**Smallest fix:** Apply the existing dirty check to scratch tabs as well as saved requests. Keep empty scratch closes immediate. Use a scratch-specific consequence in the existing confirmation so it does not promise a saved request is kept. Retain the existing stable-tab-ID lookup after the dialog resolves.

**Verification:** An unsent scratch containing body and script must show a discard choice. Cancel keeps every field and the tab; explicit discard removes it and stays removed after reload. A pristine blank draft closes without a prompt. Retain coverage for saved clean/dirty tabs, and update the persistence test to click the explicit discard action.

## UX2-02 — Major: Navigating away from an automation silently loses its edited steps

**Location:** `ui/src/modules/api/ApiPage.svelte:108` and `:378`; `ui/src/modules/api/AutomationEditor.svelte:31`.

**Failed user scenario:** Open automation A and change several assertions/extracted variables or reorder its requests. Select automation B to inspect its configuration, then return to A. A now contains its old saved steps: the working copy was discarded without a choice. Clicking an existing request tab, the automation's close button, or Manage environments also destroys the editor instance and its working copy. These are routine navigation actions during the existing automation-editing workflow.

**Existing expected behavior:** The editor expressly treats edits as a working copy until Save. `patterns.md` §7 requires protection before discarding nontrivial edits. The neighboring request editor already protects unsaved saved-request edits; users should not have to infer a different data-loss rule for an automation.

**Evidence — source-only:**

- `AutomationEditor.svelte:31` stores `steps` and `dirty` only in component-local state. Mutations such as `patchStep` at line 133 mark the working copy dirty; `save` at line 156 is the path that publishes it.
- `ApiPage.svelte:108` changes `view` directly when the automation list selects another item. The editor is keyed by `view.id` at lines 378–381, so changing the selection destroys the old component.
- On mounting again, `AutomationEditor.svelte:46` copies `automation.steps` from the saved record and clears dirty state. No draft restoration or leave guard participates.
- Other ordinary exits directly call `showRequest` (`ApiPage.svelte:74`, request tab at `:352`, close button at `:369`) or set `view` to environments (`:104`). The parent receives no dirty-state/save/leave callback from the editor.
- `ui/e2e/desktop-api-run-recovery.spec.ts:66` exercises durable executed runs/history after reload; that does not protect unsaved automation definitions. Existing run durability and unsaved editor durability are separate user tasks.

**Smallest fix:** Expose a leave decision from the active automation editor and route API-view/automation-selection transitions through it before unmounting or changing the selected ID. Offer Save / Discard / Cancel, using the existing save result to keep the editor open when saving fails. Reuse the application's navigation-guard mechanism for route/workspace exits if available. Do not auto-save simply because the user inspected another item.

**Verification:** Edit A, attempt to open B, and choose Cancel: A remains selected with its edited assertions/extractions/order intact. Discard opens B and leaves the saved A unchanged. Save persists A before opening B; a failed save stays on A with its draft. Repeat the same decision flow through a request tab, automation close, and environment navigation. A clean automation navigates without a prompt.

## UX2-03 — Minor: A failed Kafka live tail keeps claiming it is receiving live updates

**Location:** `ui/src/modules/brokers/TopicDetail.svelte:358` and `:742`.

**Failed user scenario:** Open a topic, enable Live, and receive at least one message so the tail has cursors. The broker or network then becomes unavailable while the daemon/UI remains reachable. Repeated tail fetches fail, but the existing message buffer continues to display “Live tail active — appending new messages,” potentially with the previous `+N new` suffix. A user observing an incident cannot distinguish a quiet topic from an inaccessible stream.

**Existing expected behavior:** `patterns.md` §1 says stale live data must be identified instead of indefinitely retaining an active claim. Preserving the last good buffer and retrying is useful; its freshness must remain truthful.

**Evidence — source-only:**

- The incremental request catches every rejection and returns without recording any failure (`TopicDetail.svelte:355–359`). `consuming` is reset in `finally`; the old `result`, cursors, and `tailAdded` remain.
- The polling effect at lines 260–280 retries, but maintains no last-success timestamp or degraded state. Its cadence treats the failed tick like one that appended no data.
- The live note at lines 742–745 depends only on `autoPoll && tailOffsets.size > 0`; it does not depend on a successful poll. `consumeError` is input validation, not network status.
- Existing `ui/e2e/brokers-sweep.spec.ts:285` covers successful Peek and message inspection. The inspected brokers specs do not cover an established live tail losing connectivity. The finding does not depend on screenshot interpretation or on global daemon connectivity: a broker-side failure with an otherwise healthy UI reaches this catch.

**Smallest fix:** Track the last successful poll and current tail error, clear the previous new-message count on failure, and render an inline degraded/retrying state while retaining the buffer and automatic retries. Return to the active state after a successful poll, including a successful empty result. Keep recovery reachable through an immediate retry or the existing Peek action; no repeated error-toast loop is needed.

**Verification:** Fulfill the initial tail read with one message, fail subsequent consume requests, and assert the buffer remains while the active/appending claim is replaced with failure/freshness feedback. Restore successful empty and nonempty responses and assert automatic recovery, correct cursor continuation, and no duplicate messages.

## Coverage and limits

| Area | Reviewed paths and existing evidence | Outcome / remaining limit |
|---|---|---|
| Git/workbench | Repository landing/load recovery, repository status recovery, WIP stage/discard/commit/amend controls, recovery-history/rebase/bisect flows; sampled `desktop-ux-r2-git` and `desktop-ux-r4-git` E2E assertions | No additional substantive UX finding asserted. Existing recovery controls and amend warnings are present. Full interactive conflict resolution, remote-provider auth, and physical keyboard flows were not exercised. |
| Database | Unified connection landing/status and workbench navigation; schema search and inline expansion retries; query/history/saved-query entry points | No additional finding asserted. DB draft-close/assistant/insert areas remain with Claude; mutation targeting remains in correctness review. Engine-specific live queries/import/export and phone layouts were not exercised. |
| Connections / SFTP | Connection form test/save, SFTP errors/transfers, and `desktop-connections-reliability.spec.ts` coverage for Retry and failed connection tests | Source includes inline SFTP Retry and actionable connection test details. No external SSH/database activity or transfer cancellation was run. |
| Brokers | Cluster/topic landing, topic Peek/Live/produce and details/error paths, relevant `brokers-sweep.spec.ts` source | UX2-03. Consumer-group resets and schema-history performance were already assigned to the other lenses; replay and every broker failure mode were not exhaustively reviewed. |
| API | Onboarding, saved/scratch tabs, save dialog, request send/stop, environment transitions, automation editor, history; persistence/redesign/run-recovery E2E source | UX2-01 and UX2-02. OAuth/gRPC/stream transport behavior and every automation execution outcome were not verified. |

Only this report was authored. No application source was changed and no tests are reported as passing from this pass.
