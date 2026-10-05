# Iteration 4 — design partition 5

**Provisional source score: 9.1/10. One additional major and two additional minor findings; rendered/native acceptance remains open.**

Application baseline `03f2bc3e`; observed HEAD `c5d4e999` adds review documents. Reviewed `/Users/itziklavon/claude_ade-review`, leaving unrelated role-1/2 source edits untouched. Compared relevant `b751ca6f` source only to determine whether Claude already repaired a candidate. That external source is not integrated acceptance and does not replace the baseline score.

Read AGENTS.md, PLAN/TRACKER, partition-5 correctness/performance/UX reports, relevant design accessibility/components/layout/patterns/checklist guidance, Claude's consolidated findings, raw lens reports/brief and shared coordination. No tests, builds, servers, daemon requests, browser/native interactions or screenshots were executed. Only this report was authored.

## Additional findings

### R4-D5-01 — major: shared metric charts expose historical sample values only through pointer hover

**Locations:** `ui/src/lib/components/MetricChart.svelte:193`, `:230`, `:276`; consumers `ui/src/modules/aws/MetricsPanel.svelte:169` and `ui/src/modules/kubernetes/monitor/MonitorFleet.svelte:649`.

**Trace and impact:** `hover` starts null; only `onMove(PointerEvent)` assigns the timestamp and per-series values. The SVG has `role="img"`, a label containing series names, and pointer handlers, but no focus/key interaction. The timestamp/value tooltip exists only when `hover` is populated. Consequently a keyboard user cannot inspect the sample values the pointer user can inspect. A screen reader receives series names without an equivalent historical data view. AWS does render Current/Min/Max/Avg-or-Sum beneath the chart (`MetricsPanel.svelte:170`), but those aggregates do not identify values at a chosen time. The Fleet overview's charts likewise have no timestamp/value alternative; its separate workload/pod table is an aggregate resource table, not the plotted sample series.

This violates `accessibility.md`'s keyboard equivalence rule. It is one shared-component pattern across the consumers, not one deduction per chart. No claim is made about measured contrast or exact assistive-technology output. Claude's existing MetricChart note concerns physical tooltip offset, not this missing interaction; `b751ca6f` does not change MetricChart.

**Repair:** Add a keyboard-operable sample inspector with a discoverable name/instructions, Left/Right and Home/End movement through distinct sample times, visible focus, and readable timestamp/series/value output, or expose the same data through an accessible table/disclosure. Retain the current pointer inspection. Distinguish missing values from zero. A table should be bounded or paged for large series rather than mounting every historical sample by default.

**Runtime acceptance, unexecuted:** Seed at least two series with distinct timestamps, zero and null values. With keyboard alone, inspect first/middle/last samples and verify timestamp, units and every series value match the fixture; leaving the inspector must not trap focus. Inspect the accessibility tree and VoiceOver output. Repeat in AWS and Fleet views, phone/desktop, light/dark and RTL; compare pointer and keyboard results. Verify changing series/window does not leave an invalid selected sample. Claude owns this presentation/accessibility repair; root has been notified.

### R4-D5-02 — minor: phone Permissions view removes the explanatory rules

**Locations:** `ui/src/modules/assistant/PermissionsTab.svelte:11`, `:48`, `:130`–`:135`.

**Trace and impact:** Every row's explanation is rendered only in `.note-col`. At viewport widths of 640 px or less, the stylesheet applies `display: none` to that column and its header. This removes, rather than relocates, details including the per-destination/per-tool scope of Always allow, its purchase/production exceptions, and the stated 24-hour incognito retention. The remaining Action/Access columns and introductory paragraph do not contain those details. A phone user examining this read-only policy explanation cannot read the same rules as a desktop user. The hidden text also leaves the accessibility tree at that breakpoint. This is a narrow responsive information-loss finding, not a claim about whether the backend enforces the stated policies.

**Repair:** Keep the explanation beneath the action or in a named, keyboard/touch-operable Details disclosure on small screens. Associate it with its row; preserve the desktop table and readable wrapping without horizontal page overflow. Do not merely hide the text visually while leaving it available only to assistive technology.

**Runtime acceptance, unexecuted:** At 390 px and 640 px, open Assistant → Permissions and read each explanation with touch and keyboard. Assert the purchase/production exceptions and retention wording are visible or can be expanded. Repeat at 641 px, desktop and two zoom steps, in light/dark and RTL. `b751ca6f` does not change this file. Forwarded to root for Claude ownership.

### R4-D5-03 — minor: account lookup requires closing the form to recover

**Locations:** `ui/src/lib/components/AccountPicker.svelte:17`–`:23`, `:58`–`:60`, `:77`; live caller `ui/src/modules/agents/NewSession.svelte:490`.

**Trace and impact:** Opening New Session with Claude/Codex starts the account-list GET. While it is pending, the picker displays only Default CLI account without a list-loading state. Reject the GET: an inline error tells the user to “Reopen this form to retry,” but there is no retry control or explicit reload function. The form's Sign in/Check sign-in controls target an account; they do not rerun the account list. Recovering the lookup therefore requires leaving the form or changing the provider selection instead of retrying in place. This is not failure-as-empty after rejection—the error is visible—and the default account remains usable. The defect is the missing pending/recovery presentation for selecting an existing nondefault account.

**Repair:** Extract a provider-scoped account loader with pending, success and failure states and a compact inline Retry. Preserve the current session configuration and any account-label draft while retrying. Separate list failure from profile creation/sign-in errors so unrelated actions cannot silently clear an unresolved list failure. Reject late results after provider change/unmount. Keep intentional Default CLI account selection available; do not silently substitute it for an explicitly selected account whose list failed to load.

**Runtime acceptance, unexecuted:** Defer then fail `/auth/provider-accounts`; verify list-loading feedback followed by Retry. Retain the entered session prompt/options, recover the GET without remounting New Session, and select the returned account. Verify a stale response after provider change cannot publish into the new provider and list retry does not mint or sign in to an account. `b751ca6f` leaves AccountPicker unchanged. Root has been notified to reserve the loader/retry behavior with Claude's markup coordination; no source reservation/edit was made by this reviewer.

## Existing external dependencies retained once

These baseline findings already belong to Claude. They are not new Codex tasks, and reported external fixes do not remove baseline deductions until integrated-source and rendered verification. This is a bounded list of independently inspected patterns, not an assertion that the larger external catalogue is otherwise clear.

| Pattern | Severity / deduction | Source, repair and acceptance |
|---|---|---|
| Fixed page-level list panes | minor / 0.1 | `assistant/AssistantPage.svelte:268` fixes the list at 240 px; `personal-agents/RoomsView.svelte:360` fixes the list at 220 px. Existing `02-layout.md` finding. Integrate the shared PaneDivider/LIST_PANE repair; verify pointer/keyboard resizing, persistence, long titles and phone list/detail navigation. Count the repeated pane pattern once. |
| Thread navigation persists on unrelated Assistant tabs | minor / 0.1 | `assistant/AssistantPage.svelte:122`, `:187` shows the thread list for every desktop tab, while selection applies to Chat at `:192`. Existing `02-layout.md` finding. Render the thread list on Chat only and verify Tasks/Memory/Permissions reclaim the space, without losing Chat selection on return. |
| Shipped personal-agent template uses an emoji identity | minor / 0.1 | `personal-agents/templates.ts:25` supplies the shield avatar; `personal-agents/AgentAvatar.svelte:17` prefers the supplied glyph over its monogram. Existing `08-agent-patterns.md` finding. Use the documented default monogram for shipped templates; retain explicit user avatar data. Inspect new-template cards, detail header and room messages in light/dark. |
| Proposal action hierarchy differs between hosts | minor / 0.1 | `assistant/MemoryTab.svelte:228`–`:229` uses ghost Reject/plain Keep; external review identifies primary Keep in NeedsYouCard. Existing `08-agent-patterns.md` finding. Integrate the shared neutral Reject/primary Keep convention and verify pending/disabled outcomes in both hosts. This is presentation, not an assertion that memory approval mutates incorrectly. |

The load-state pattern in R4-D5-03 is charged once; no extra deductions are added here for the broader external LoadState migration. Settings save/delete feedback, shared tokens/focus/motion, component/copy sweeps, Proof URL selection and notifications remain tracked in Claude's catalogue without being reissued as additional tasks. C5's schedule/run and Proof request races, P5's recap polling, and U5's Insights save queue/token reveal are excluded from this design score and remain separate dependencies.

## Fixed score and coverage

**10 − (1 major × 0.3) − (6 minors × 0.1) = 9.1/10.** The six minors are Permissions responsive content, AccountPicker load/retry, fixed panes, unrelated Assistant navigation, shipped template identity and proposal hierarchy. No blockers or nits were added. Each systemic pattern is counted once within this partition; app-wide aggregation must deduplicate shared patterns across partition reports. The major independently prevents 9.8 acceptance.

| Design dimension | Inspected source coverage | Evidence limits / required acceptance |
|---|---|---|
| Shared visual hierarchy | Assistant header/list/tab composition, proposal actions, personal-agent list/detail/rooms; sampled Settings Context Library and toggle/picker composition, PluginFrame, Usage/Insights, guest Share, room screens/recap/participants and Proof's earlier scoped report. Most sampled views use the shared header, labelled controls and scoped status text. | Full density, title/toolbar balance, long-content and all-panel hierarchy were not rendered. Proof and most settings subpages received bounded sampling rather than exhaustive inspection. |
| Tokens/consistency/readability | Semantic colors and shared Icon/status/LoadState usage sampled; source uses readable type tokens in the inspected controls. Existing template/proposal inconsistencies retained above. | No computed contrast measurements, all-theme/custom-accent sweep, image/composited-surface measurement or native font inspection. External token repairs require integration and verification. |
| Responsive layout | RoomScreens grid/pinned phone behavior, room controls' wrapping, Share hierarchy, ModelBreakdown's internal table scrolling, Assistant split and Permissions breakpoint inspected. | Phone/tablet/desktop, zoom and RTL rendering remain required. The Permissions content-removal trace is directly established by CSS, but no screenshot was executed. |
| Keyboard/accessibility | Shared MetricChart, ModelPicker/AccountPicker labels, FolderPicker grid/activedescendant/key wiring, Assistant thread arrow navigation, room annotation Point/Highlight keyboard path and real action controls inspected. Home widget resize/reorder source offers keyboard alternatives. | Full journeys, focus restoration, assistive-technology output and native shortcut routing unexecuted. Freehand room drawing was not classified as an additional keyboard defect merely because its inherently path-based input uses a pointer. |
| Inspected rendered states in light/dark | **None executed by this reviewer.** Prior app-20261004 verification and Claude's external reported gates remain contextual evidence, not newly executed acceptance of these fixtures. | Current integrated revision screenshots and pending/empty/error/retry/long-content/selected-state checks; packaged Tauri/WKWebView, VoiceOver and physical room media remain open. |

No separate numerical /2 score is assigned to these coverage dimensions. The 9.1 is a source-review judgment over this stated matrix, not whole-app rendered/native certification. Auth/login, full plugin install lifecycle, all Settings panels, Usage chart variants/report exports, cloud resource detail permutations and real room capture/multi-device behavior are not exhaustively covered.

**Handoff:** Three additions sent to root for ownership/coordination. Report-only review complete; no source edits, commits, builds, tests, servers or screenshots. Read-shell slot released.
