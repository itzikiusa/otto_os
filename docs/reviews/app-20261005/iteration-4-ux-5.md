# Iteration 4 — UX partition 5

**Verdict:** Changes needed. **New findings:** 0 blockers, 1 major, 1 minor. **Provisional score: 7.5/10.**

Application baseline: `03f2bc3e`; reviewed HEAD `c5d4e999767fca39db497b6eb444e4eaaa04558d` adds review documents. Concurrent role-1 changes are outside this partition. This is a bounded source review of adjacent user journeys, not a claim that the whole partition was exercised. No tests, builds, servers, browser/native sessions, screenshots, external operations or source edits were performed by this reviewer. Only this report was written.

Read AGENTS.md, PLAN.md (including execution calibration), SCORES.md, TRACKER.md, partition-5 correctness/performance reports, the prior final VERIFICATION.md, and Claude's consolidated iteration-4 findings/coordination note. Visual, accessibility and copy findings remain Claude's ownership. No new skill was needed for this end-user UX lens; the available devex-review skill concerns developer interfaces, a different scope.

## Findings

### R4-U5-01 — major — Changing the Insights model during a schedule save silently leaves the old model configured

**Locations:** `ui/src/modules/settings/InsightsSettings.svelte:102` queues the agent patch; `:120` drains it only after an agent save; `:127`–`:142` is the schedule save with no drain. The model remains editable at `:190`–`:195`; `ui/src/lib/components/ModelPicker.svelte:61`–`:83` has no disabled input prop.

**Intended journey:** Enable a reporting cadence, choose the report model, and leave the page with both choices saved. Provider/model controls are immediate-save settings, so the visible choice must match persisted configuration or show that it is pending/failed.

**Confirmed source trace / reproduction:** Start with model A. Toggle Daily and hold the `/insights/config` PUT pending for more than 500 ms. Select or type model B while that PUT is pending. `onModelChange` immediately displays B and its timer calls `saveAgent({model: B})`. Because `saving` is true, this only stores `queuedAgent` and returns. Resolve the schedule PUT with the saved configuration containing A. `toggle()` updates `cfg`, flashes Saved for Schedule and clears `saving`; it never consumes the queued model patch. There is no second request, no agent-save failure and no pending indicator. `modelDraft` still displays B. Reloading settings restores A; the intended model change never reached the server. This is deterministic under the described deferred response, not a timing claim based on execution.

**Impact:** The immediate-save workflow fails silently and subsequent reports use the old model. The schedule's Saved indicator is scoped to Schedule; the finding does not claim it explicitly says the model was saved. The material problem is an accepted model edit left indefinitely unsaved without feedback.

**Repair:** Use one serialized save/queue mechanism for schedule and agent patches, or make both completion paths drain the latest queued patch against the last acknowledged configuration. Preserve the newest displayed model while older responses settle. On failure, retain an actionable pending/error state or accurately revert it. Guard against replaying a superseded queued model after a later provider/model change. Existing optimistic schedule rollback should remain.

**Required verification:** A component/browser regression should defer the schedule PUT, change the model, advance past the debounce, resolve the PUT, and require a second PUT carrying both the final cadence and B. Reload and assert B remains selected. Repeat with schedule rejection, successive B/C edits, a model-save rejection, and a subsequent provider change; assert no old queued model is resurrected. No such regression was executed here.

**Reservation:** `ui/src/modules/settings/InsightsSettings.svelte` save queue/drain and corresponding feedback behavior only. Root has been notified; preserve Claude's markup/copy changes. No API contract change is required for the described repair.

### R4-U5-02 — minor — Creating another token replaces an uncopied one-time secret

**Locations:** `ui/src/modules/settings/PersonalAccessTokens.svelte:75` checks only `minting`; `:82` replaces `freshSecret`; `:176` invokes mint on Enter and `:179` disables Create only while a request is running. The explicit Done acknowledgement is at `:156`.

**Intended journey:** Create a token, copy its one-time secret, then explicitly dismiss the reveal before creating another token.

**Confirmed source trace / reproduction:** Create token A and wait for its response. Do not copy or press Done. Enter label B and click the enabled Create token button (or press Enter in the label field). A successful second response replaces the banner's `freshSecret`/`freshInfo` with B. A remains in `tokens`, but its full secret is no longer available in the UI. Token creation stores the hash, not a recoverable raw secret (`crates/otto-rbac/src/tokens.rs:580`–`:585`), so reopening settings cannot restore A's secret. This does not require an automatic rotation or a failed server operation.

**Impact and recovery:** This is avoidable setup rework, not an access-loss blocker: the user can revoke A through the existing row action and create a replacement. No existing configured client's token is changed or revoked by this trace. The one-time warning is already present, but the UI permits another ordinary create action to discard the secret without using Done.

**Repair:** Require explicit reveal dismissal before another mint, guarding both `mint()` and the Create/Enter paths while `freshSecret` exists. Keep Copy/manual selection and Done available, including after a clipboard failure. Do not persist raw secrets to recover this UI state and do not automatically revoke the first token.

**Required verification:** Return A from the first POST, then attempt click and Enter creation before Done; assert no second POST and that A remains selectable/copyable. Reject the clipboard operation and assert the reveal remains available. After Done, create B successfully. Verify the existing explicit revoke recovery still targets A. Tests are proposed, not executed.

**Reservation:** `ui/src/modules/settings/PersonalAccessTokens.svelte` mint/reveal lifecycle and guards only; coordinate adjacent markup with Claude. Root has been notified.

## Checked journeys and evidence limits

| Journey | Source evidence checked | Result / limit |
|---|---|---|
| Settings immediate save | Insights configuration load/retry, schedule toggles, provider reset, model debounce and queue; ModelPicker control interface | New major above. Most other settings panels, backup/restore and native close persistence were not audited. |
| Token setup and revocation | PersonalAccessTokens mint/reveal/copy/dismiss, individual revoke, batch partial failure; RBAC token hash storage | New minor above; actual clipboard/credential operations were not performed. |
| Plugin install and recovery | PluginsSettings load retry, source field, install/toggle/remove and post-action refresh | Inline install failure retains the attempted source; removal confirmation explains process/sidebar effects and retained files. Source trace only, no installed plugin lifecycle execution. |
| Assistant draft and navigation | AssistantComposer send failure restoration/attachment merge, dirty guard; AssistantPage route navigation and keyed thread view; shared leaveGuard | Failure restores text and attachments, navigation passes through the shared draft guard. Prior verified incremental-turn browser regression remains credited. No new message/upload/approval execution. |
| Rooms and recaps | StartRoomModal retained creation on failed native open; RoomChat nonce retry; RoomRecapsPage keyed selection; RecapPanel errors, paging, export, summary consent and draft coverage | Useful recovery and explicit summary sending scope in source. Prior Rooms tail/earlier pagination passes are credited. Physical media, consent across two devices, recap generation/export and native room windows were not exercised. |
| Guest share access | SharePage scoped token/generation, OTP validation, failed verification, expiry/revocation recheck | Denial/OTP branches preserve safe read-only defaults and distinguish actionable recovery. No real recipient, mail or remote access execution. |
| Usage and Home | AttributionDrilldown grouping/window generation, clearing old-scope rows, Retry and clipboard failure; Home Today source/failure state | Source ownership and failure paths sampled. No historical Usage report/export execution or full Home aggregation audit. |
| Auth and cloud | Auth boot in-flight guard/capability fallback; Kubernetes ScaleDialog target and validation; AWS LogsView account-scoped entry setup | Samples only, insufficient for cloud/auth acceptance. No AWS/Kubernetes changes or sign-in tests performed. |
| Personal agents / Proof | Read the partition-5 correctness report and caller/source traces it records | Existing schedule/run lost updates, Proof filter/cursor mixing and close-refresh reopening remain open cross-lens dependencies. Not reissued as new UX findings; not independently executed here. |
| Shared components | ModelPicker and leaveGuard substantive reads; other shared components only through callers | No exhaustive component/native keyboard/light-dark review. Visual/a11y/copy remains Claude's lane. |

The prior final `../app-20261004/VERIFICATION.md` records green combined UI checks/1,081 units, 98 distinct affected browser cases verified across the initial group and repaired reruns, Assistant incremental messages without an extra history GET, and Rooms tail/earlier-page passes. These are existing verified baseline results, not this reviewer's execution. They cover useful named adjacent behavior but do not substantially cover this broad partition's settings, share, cloud, token and recovery matrix, nor the two newly traced cases. Accordingly the shared execution dimension remains 1.0 rather than erasing baseline credit or treating a green baseline as current repair acceptance.

## Fixed UX rubric

| Dimension | Score / 2 | Evidence and deduction |
|---|---:|---|
| Task completion/discovery | 1.7 | Existing plugin/Assistant/room recovery paths are reachable in source; report model configuration can silently fail to complete. Full partition discovery was not exercised. |
| Feedback/state clarity | 1.6 | Inline retry and scope-aware rows are positive; model selection can disagree with saved configuration indefinitely. Existing Proof filter/result mismatch remains an open dependency. |
| Recovery/retry | 1.6 | Message and native-room-open retries preserve useful state; queued model save never drains and a replaced secret requires revoke/recreate work. Failure matrix remains bounded. |
| Draft/scope/trust preservation | 1.6 | Assistant drafts, scoped guest requests and explicit recap-generation consent provide positive source evidence; uncopied token reveal is replaceable and model choice is not reliably persisted. |
| Executed end-to-end journeys | 1.0 | Verified baseline credited under PLAN.md calibration; named Assistant/Rooms passes do not establish substantial coverage of this partition's acceptance matrix. No new execution. |
| **Total** | **7.5 / 10** | Provisional judgment over this stated matrix. The major finding independently prevents 9.8 acceptance. |

No numerical increase is warranted until the accepted repairs and their deferred/failure journeys are executed. This report neither duplicates C5/P5 defects nor certifies uninspected native, cloud, multi-user or visual behavior.
