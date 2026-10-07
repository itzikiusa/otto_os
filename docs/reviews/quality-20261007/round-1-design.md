# Round 1 — design review

Date: 2026-10-07. Reviewed baseline: `196048df` (after pulling main; initial shared-component reads at `922ae483`, unchanged in the pulled range). Read-only source review; this report is the only authored file. No browser, build, test, live daemon mutation, or publication was performed in this pass.

## Assessment

**Provisional sampled design score: 7.7/10. Not an app-wide visual certification.** The shared component system is mature, but new consumers bypass its loading/empty-state contract and keyboard protections. Five source-confirmed findings below prevent a credible 9.8+ claim. Closing them does not automatically earn that score: rendered review and remaining module coverage are still necessary.

| Dimension | /2 | Basis |
|---|---:|---|
| Product consistency and component integration | 1.7 | Shared PageHeader/PageBody, token usage and reusable load/dialog primitives are present. School and the isolated plugin introduce integration gaps. |
| Loading, empty, error and recovery states | 1.3 | School first load is blank; initial failures claim stale data exists. Empty trace explanation never renders; trace failures lack Retry. |
| Keyboard, focus and accessible operation | 1.2 | Strong shared Modal/ContextMenu foundations; invisible School tab stops, intercepted button activation, and plugin nested-dialog dismissal remain. |
| Responsive layout and visual foundations | 1.7 | Source uses internal scrolling, capped cards, semantic type/color tokens and shared chrome. Fresh light/dark/phone/RTL evidence unavailable; no measured contrast claim. |
| Information hierarchy and action clarity | 1.8 | Representative automation surfaces expose scoped actions, recovery, provenance-related controls and clear labels. Empty/misleading state feedback undermines trust in the new surfaces. |
| **Total** | **7.7** | Qualitative rubric for the inspected sample, not a numerical measurement of the whole application. |

Severity: **Major** blocks or materially misleads a normal interaction; **Minor** is a bounded quality defect with a practical workaround. No cosmetic preference is treated as a defect.

## Findings

### D1 — Major: School's selected-session buttons lose their Enter action

**Evidence:** `ui/src/modules/home/boxes/ClassroomsBox.svelte:403`, `:411`, `:432`, `:649`, `:662`; `ui/src/modules/home/school/scene.ts:1213`.

The stage installs a bubbling `keydown` listener. The selected-session card and all its real buttons are descendants of that stage. `onStageKey` handles every Enter without checking the event target or `defaultPrevented`: it calls `preventDefault()` and applies the stage's camera/open behavior. The scene's `key()` does not consume Enter first.

**Reachable trace:** select a kid in a room → focus the card's **Open session** button → press Enter. The stage handler prevents the button's default click and calls `lookAtScreen`, because `view.kind` is `room`. In screen view, Enter on **Back to the room**, **Detention**, or **Kick out…** can instead open the selected session. The button's visible label no longer predicts keyboard behavior. Space remains a workaround, so this is not a claim that all activation is impossible.

**Fix direction:** scope camera shortcuts to the stage itself, or explicitly exclude interactive descendants and already-handled events. Leave native button Enter/Space activation intact. Preserve Escape behavior only where it cannot override a nested control/layer.

**Acceptance:** in both room and screen views, Tab to each card action and activate with Enter and Space; verify the same action as clicking, including the kick confirmation. Separately focus the bare stage and verify Enter/Escape camera behavior still works.

### D2 — Major: School's accessible companion creates invisible keyboard stops

**Evidence:** `ui/src/modules/home/boxes/ClassroomsBox.svelte:697`, `:711`, `:718`, `:722`, `:731`; `ui/src/app.css:416`.

In 3D mode the entire companion list receives `sr-only`, while its Walk in, Open/Release and More actions buttons remain in the tab order. The global class clips the container to one pixel; neither the component nor the global rule reveals it on focus. Focusing a session row also calls `showIn3d`, which may move the camera and select a session, but that does not reveal which hidden control currently has focus. The selected card appears earlier in DOM order, so simply continuing forward through Tab does not move into its newly visible actions.

**Impact:** a sighted keyboard user tabs through controls whose focus ring, label and available action cannot be seen. A populated cross-workspace list can add many such stops. Screen-reader labels alone do not satisfy visible-focus requirements.

**Fix direction:** reveal a usable companion panel on focus-within, or provide a visible keyboard entry that switches to List and removes the hidden duplicate controls from the tab sequence. Preserve the equivalent screen-reader and keyboard action path.

**Acceptance:** with multiple classrooms and sessions, traverse the full widget using Tab/Shift+Tab. Every focused element must have visible context and a visible focus indicator; every session and menu remains accessible without a pointer. Test 3D and List, plus no-WebGL fallback.

### D3 — Major: Escape from the plugin's download confirmation dismisses the report too

**Evidence:** `examples/plugins/team-performance/ui/components.js:259`, `:284`, `:300`; `examples/plugins/team-performance/ui/views/reports.js:75`, `:117`, `:120`.

Each plugin modal adds its own capture listener to `document`. Every such listener responds to Escape; there is no topmost-modal check. `stopPropagation()` does not stop another listener on the same document target. The report viewer opens a second modal through the Download confirmation, so this is an existing user flow, not hypothetical nesting.

**Reachable trace:** open report → type an unsent comment → Download → Escape. The report's older document listener closes the viewer; the confirmation listener closes the confirmation. The user intended to cancel download but also loses the report context and its unsent comment. Focus return can target the removed viewer button. The same shared listener design needs to account for a popover above a modal.

**Fix direction:** track the plugin overlay stack and let only its topmost layer handle/trap Escape and Tab. Honor handled events, make covered dialogs inert, and restore focus to the surviving parent. The plugin is a separate HTML surface, so importing the Svelte Modal is not required; reproduce its relevant behavior in the plugin primitive.

**Acceptance:** type a comment, open Download, press Escape once: only the confirmation closes, the report/comment survives and Download regains focus. A second Escape may close the report. Exercise Tab wrapping within each active layer and popover-over-modal Escape handling.

### D4 — Minor: School first load is blank and initial errors imply a successful prior load

**Evidence:** `ui/src/modules/home/boxes/ClassroomsBox.svelte:610`, `:613`, `:679`; shared contract at `ui/src/lib/components/LoadState.svelte:48`, `:88`, `:98`.

School calls `<LoadState loading={true}>` without `empty` or children during the initial empty fetch. `LoadState` defaults `empty` to false and only shows a skeleton when `loading && empty`, so this branch renders nothing. The initial-error and scene-error calls also omit `empty`, which selects the stale-data message, “showing the last good load,” even when no data/scene has ever loaded.

**Fix direction:** pass `empty` in these no-content branches (or use one correctly configured wrapper). Retain real stale data with a stale banner only when it exists. Keep the delayed loader announcement provided by the shared component.

**Acceptance:** delay session fetch beyond the 150 ms grace with empty workspace stores; expect a visible loading indicator and loading status. Fail initial fetch and scene initialization separately; expect “Couldn't load” with Retry, never “last good load.” Retry must recover to the scene/list without a misleading empty flash.

### D5 — Minor: expired trace dialog has no message, and failed trace has no Retry

**Evidence:** `ui/src/modules/usage/OttoUsage.svelte:117`, `:211`; shared contract at `ui/src/lib/components/LoadState.svelte:95`.

The trace wrapper sets `empty={!trace.length}` but places “This trace has expired or has not reached local storage yet” inside its ordinary children. On a successful empty response, `LoadState` chooses its `emptyView` branch, which is absent, and never renders those children. The dialog therefore has only chrome instead of the intended explanation. An error response sets `traceError`, but the wrapper supplies no `onretry`, leaving a failed trace without the standard recovery action.

**Fix direction:** move the expiration explanation into `emptyView`; retain the requested trace id and wire `onretry` to `openTrace(id)`. Preserve cancellation/request-sequence handling when the dialog closes.

**Acceptance:** return `[]` for a trace and assert the expiration/storage explanation; return 503 and assert Retry; make Retry succeed and assert spans. Close while retrying and ensure it stays closed.

## Scope inventory

| Area | Evidence inspected | Coverage limits |
|---|---|---|
| Design contract | AGENTS.md; design README, foundations, layout, components, accessibility, review-checklist | Read rules and relevant APIs. Checklist's old “No --text-dim on --surface-3” contradicts foundations' current AA matrix; valid token usage was not reported as a defect. |
| Shared chrome and primitives | PageHeader sizing/overflow implementation; complete Modal, LoadState, PageBody; ContextMenu keyboard/clamping landmarks; Navigator control/ARIA landmarks; global sr-only CSS | Source review, no geometry/VoiceOver verification. Navigator and ContextMenu were sampled, not exhaustively reviewed. |
| New School | ClassroomsBox data/view/action/keyboard/markup/styles; scene key handling; existing School browser specs | No rendered asset, lighting, animation, performance or visual-composition verdict. No judgment against the requested school metaphor. |
| New application telemetry | Complete OttoUsage; UsagePage tab integration and state/control landmarks; desktop-telemetry spec | No collector running or metrics accuracy verification; instrumentation pipeline belongs to correctness/performance review. |
| New Team Performance plugin | Modal/popover primitive; report viewer and Download nesting; browser matrix | Did not read every analytics view or exported report chart. Standalone plugin is assessed by shared behavior goals, not penalized for lacking Svelte imports. |
| Automation | Loops page; Scheduled Tasks form/state setup; Workflows remembered/deep-linked selection and error/action landmarks | Representative source sample; not full run/create/approval lifecycle verification. |
| Data/content | ResultsGrid orchestration/API boundary sample; Vault toolbar, load/empty and pane-state integration | Grid interactions, all document/editor modes and other modules remain unverified. |

No assertion of exhaustive app coverage, fresh passing tests, theme contrast, or 9.8 readiness follows from this inventory.

## Browser acceptance matrix for the next verification slot

Use isolated fixtures/throwaway daemon; never the user's sessions. These are proposed runs/extensions, not completed checks.

| Surface | Existing starting point | Required checks / fixtures |
|---|---|---|
| Shared chrome | `ui/e2e/desktop-page-chrome.spec.ts`, `desktop-dialogs-tokens.spec.ts` | 1440×900 light/dark; one header, reachable overflow, long titles; nested sheets, focus return, Escape closes one layer; tall content and long menus fully in viewport. |
| Global responsive/RTL | `ui/e2e/pages.spec.ts`, `theme.spec.ts`, `rtl.spec.ts` | Phone 390×844; tablet 768×1024; no page overflow, list/detail push flow, two zoom steps, direction-aware navigation. These route sweeps do not automatically exercise every nested tab/widget. |
| School | `ui/e2e/desktop-home-classrooms.spec.ts`, `desktop-home-classrooms-3d.spec.ts` | Existing tests include scene load, screenshot points, basic Enter/Escape, dark mode and context menu. Add D1/D2/D4 flows, long labels, small widget and zoomed page, room/selected-card light+dark+phone screenshots, RTL, reduced motion and no-WebGL. |
| Otto usage | `ui/e2e/desktop-telemetry.spec.ts` | Existing spec covers light/dark captures, phone overflow, config stale/error recovery. Add populated metrics + trace empty/error/retry, keyboard horizontal table scroll, RTL code/span direction, reduced motion, Warm dark/custom accent chart contrast. |
| Team Performance | `examples/plugins/team-performance/test/browser.e2e.test.js` | Existing source matrix is 390/1280 × light/dark with long project-menu clamp checks. Add nested report Download D3 flow, comment preservation, focus cycling, RTL, full report rendering and keyboard-only metric drilldown. Run independently from heavyweight app builds. |
| Automation/data/content | Existing module-specific E2E plus page sweeps | Seed nonempty lists, remember/deep-link selection, initial error/retry, stale refresh, phone back navigation, long content, unsaved drafts and keyboard-only main action. |

For changed surfaces capture 1440×900 light/dark and one phone image; add Warm dark for tinted controls. Inspect images rather than treating screenshot generation as a pass. `expectAccessible` only fails critical violations by default: explicitly assert serious contrast findings are absent on the changed surfaces. A manual native WKWebView/VoiceOver spot-check remains separate from Chromium evidence.

## Exit criteria for this round

Fix and verify D1–D5, rerun the affected type/unit/browser gates, inspect the captured images, then rescore only the dimensions supported by that new evidence. Broader modules and native accessibility remain explicitly outside this bounded pass until sampled in subsequent rounds.
