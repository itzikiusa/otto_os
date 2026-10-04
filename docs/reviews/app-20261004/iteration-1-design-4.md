# Iteration 1 — visual design partition 4

**Verdict: one uncovered minor load-state finding.** One additional major accessibility gap is already owned by the parallel design effort and is recorded only for verification. Scope: workflows, Goal Loops, Swarm, Scheduled Tasks, Mission Control and MCP.

This is a bounded **source-trace review**, not a runtime visual inspection. The review checkout is at `a16f4c71` with concurrent application fixes. Read AGENTS.md, the design guidelines README and review checklist, partition-4 correctness/performance reports, and `/tmp/otto-app-review-coordination-20261004.txt`. Compared the scoped design changes in `/Users/itziklavon/claude_ade-design` through `372a8944`, including its current uncommitted changes. No application files were edited; no builds, tests, servers, screenshots, or runtime interactions were performed.

## External-owned verification — Mission Control fullscreen detail focus (major impact)

**Ownership update:** Parent confirmed Claude iteration 2, implementer B task 7 already owns this change (push navigation or shared Drawer focus). This is **not a new finding or an application-review fix request**; verify the completed external change with the checks below.

**Locations:** `ui/src/modules/mission-control/MissionControlPage.svelte:99`, `:389`, `:550`; `ui/src/modules/mission-control/WorkItemDetail.svelte:213`.

**Evidence and trigger:** At phone/tablet widths up to 1024 px, selecting a work item renders `.mc-detail` as a fixed, full-viewport layer. The selection button stays mounted in the list (`WorkItemList.svelte:37`). Selection changes the ID/URL but does not move focus. The only overlay lifecycle handling calls `ui.pushModal()`/`popModal()`; those methods merely maintain the native-webview visibility counter (`ui/src/lib/stores/ui.svelte.ts:282`). The detail is an ordinary `aside`, without modal semantics, `dialogFocus`, a focus trap, an Escape handler, or background inertness. Neither the shell nor this module supplies those missing behaviors globally.

**User impact:** Open the first of several work items using Enter on a tablet keyboard: focus stays on the now-covered list button, and the next Tab can move to another covered list item instead of the visible detail. Enter can then select another item behind the sheet. Screen-reader navigation likewise continues through the covered page without a modal boundary. Escape does not close the detail. This impairs the main review/approval flow on the supported tablet/phone layout.

**Smallest fix:** In the non-desktop branch, use a modal surface with an accessible title and the existing `dialogFocus` behavior, or adapt the shared Modal to the fullscreen presentation. Move focus into it on open, contain Tab, close the top layer on Escape, and restore the selected row on close. Keep the covered background out of interaction/accessibility navigation. Retain the ordinary side-pane behavior on desktop. Coordinate the close callback with the separate UX review's dirty-edit guard.

**Verification to run:** At 390 px and 768 px, seed several work items, keyboard-open the first, and assert focus is inside the visible detail. Tab/Shift+Tab must stay there; Escape must close it and return focus to the row. Verify nested approval dialogs own Escape until closed. Inspect the accessibility tree for a named modal and inaccessible background. At 1440 px, confirm the persistent side pane remains normally navigable. Repeat light/dark; no visual claim is made here.

**Deduplication:** The inspected parallel design diff had not yet implemented the fullscreen focus change; parent subsequently confirmed it is already assigned to Claude iteration 2. UX partition 4 covers loss of edit drafts on navigation; its close guard should compose with the external accessibility fix.

## D4-01 — Minor: Swarm skill discovery presents a failed library load as an empty picker

**Locations:** `ui/src/modules/swarm/SkillPicker.svelte:19`, `:59`; `ui/src/lib/stores/swarm.svelte.ts:869`.

**Evidence and trigger:** The team/project skill picker loads `/library/skills` on mount. The store catches every failure and assigns `librarySkills = []`, exposing neither loading nor error state. The picker renders an empty datalist from that array, with only the independently valid “No skills selected” text when the selection is empty. There is no error explanation or Retry. A first load delayed or rejected by the daemon is visually indistinguishable from a successfully loaded empty catalogue.

**User impact:** Users configuring team or project skills cannot tell whether suggestions are unavailable, still loading, or truly absent. They must guess a skill name or close and reopen the editor to retry discovery. This violates the guideline that each data region distinguishes loading, empty, error and loaded states.

**Smallest fix:** Expose library loading/error state and display a compact inline status with Retry next to the add field. Distinguish a successful empty catalogue from a failed read. Preserve the current selections and draft text through retry; typing a known skill can remain available. This requires a small coordinated store change in addition to markup.

**Verification to run:** Delay the first library request and assert a loading status. Reject it and assert an inline error plus Retry instead of silent empty suggestions. Recover with a nonempty catalogue and assert suggestions appear without closing the settings/project editor or losing typed/selected skills. Also verify successful empty results have their own state.

**Deduplication:** Claude's pending Swarm store changes add run-list error handling only; `loadLibrarySkills` and this picker remain unchanged. UX partition 4 confirmed it is not covering library loading.

## Coverage and limits

Sampled page structures, action/field markup, states, graph controls and responsive CSS across the six assigned modules. Followed Mission Control's list/detail selection, overlay registration and shell focus infrastructure; Swarm's goal/recruit/settings forms, graph views and shared skill loader; MCP policy/auto-approval forms and rule drawer; workflow popovers/resizers/triggers; and Goal Loop/Scheduled Task forms and details.

Excluded already-owned fixes for workflow popovers/resizers/tabs, MCP Rules drawer focus, MCP catalogue load states, approval actions, scheduled Run-now confirmation, and Swarm run-list states. Correctness/performance partition-4 findings are not repeated. Multi-column forms merit phone rendering in the later visual sweep, but no unmeasured overflow or contrast failure is asserted. Browser-specific focus behavior, light/dark appearance, touch target dimensions, and runtime responsive layouts still require the targeted checks above.
