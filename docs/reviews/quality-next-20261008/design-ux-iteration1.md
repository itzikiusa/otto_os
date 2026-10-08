# Design and UX — iteration 1

Reviewed baseline `7cff9e538deccdd0e346af7c6285e11863285c00` on `review/quality-20261008-next`, followed by the repairs below in the shared checkout. These are scoped engineering assessments, not a whole-product certification. Final consumer-wide UI checks are coordinated by root.

## Findings and repairs

| ID | Severity | Confirmed behavior and cause | Repair and verification |
|---|---|---|---|
| DUX-01 | P2 | `ui/src/lib/components/PageHeader.svelte:252`: focusing API **Sync with Git…**, then resizing 1440 → 390 px, hides the focused action without handing the keyboard to **More actions**. The browser drops the user's toolbar focus. Expanding the toolbar can likewise remove the focused overflow button. | Header remembers the original action and hands focus to the overflow button after render, then back to the original action when the button disappears. It does not move focus from an unrelated editor. Actual production API page regression failed at `toBeFocused` before repair; final three-theme cases pass, verify menu availability and actual dialog activation, and preserve a URL draft/focus as a negative control. |
| DUX-02 | P2 | `ui/src/shell/App.svelte:338`: the identity reload used `void ws.load()` without handling rejection. A routed 503 workspace-list response left an empty-looking shell without Retry and raised an unhandled rejection. | Shell owns a compact shared `LoadState` recovery banner and catches the reload. Generation/token ownership prevents old identity responses from publishing. Real isolated authenticated browser regression failed because the recovery banner was absent; repair passes error → Retry → saved workspace restored, checks no page errors and a visible phone recovery action. A separate held old request is rejected after a newer effective identity loads; it neither publishes the old error nor throws an unhandled rejection. |
| DUX-03 | P2 | Existing `desktop-page-chrome.spec.ts:193` failed after selecting a Proof pack and reloading: expected **Chrome proof A**, received **Proof packs**. `ProofPage.svelte` started deep-link detail loading before workspace initialization; the later workspace reset invalidated it, while automatic selection skipped deep links. | Root independently traced and repaired the workspace-readiness gate in `ProofPage.svelte`. This reviewer added delayed-workspace startup coverage and reran the original regression. Both pass: no early detail request, one detail request once workspace startup settles, named pack displayed and URL preserved. |

No confirmed finding above remains open in the tested snapshot. No general popup/modal redesign or abstraction rewrite was justified.

## Coverage

Read `AGENTS.md`; design guidelines README, accessibility, layout and review checklist; prior R16/R13 reports and their explicit evidence limits. Inspected production shared `PageHeader`, `Modal`, `Tabs`, `LoadState`, `ContextMenu`, context-menu store, dialog focus helpers, tab keys, Drawer, NotificationBell, shell identity/boot composition and their callers. Inspected actual API and Proof navigation flows around the findings.

Executed the existing top-level header sweep across its 34 listed routes: exactly one PageHeader at the expected height (Agents separately retains its TabBar). This is shared-chrome coverage of initial states, not populated workflow or end-to-end coverage of all 34 modules.

Independent screenshots opened: repaired API phone light; API desktop native dark; API phone Warm dark; phone workspace failure with Retry. Captures show visible focus rings, readable hierarchy, unclipped controls and coherent shared materials. Durable images include desktop and phone in native light/dark and Warm dark, plus workspace failure desktop/phone. Theme fixture assertions separately measure selected shared-sheet/setup text at ≥11 px and ≥4.5:1 contrast and run axe on the scoped setup/sheet surfaces, including RTL and extreme custom accents. Those tests do not establish that every populated page is accessible.

## Commands and evidence

All execution used Node 26 (`PATH=/opt/homebrew/bin:$PATH`), Chromium `desktop-browser`, one worker, slot `next-design1`, isolated daemon port `17826`, Vite port `5196`, `OTTO_E2E_SWEEP_ORPHANS=0`, and a temporary fixture profile. No installed app, live profile, provider account or production database was manipulated.

Daemon: `/Users/itziklavon/otto_os/target/debug/ottod`, SHA-256 `d7e61497d0aa8318793985c67aded624797547689fc5e235f44a479c4786bb0f`. This baseline binary validates these UI fixes; it does not include concurrent Rust repairs. Final source hashes are in [source-sha256.txt](evidence/design-ux/source-sha256.txt).

- [shared-baseline.log](evidence/design-ux/shared-baseline.log): **12 passed, 28.8 s**. Named selections from `desktop-page-chrome`, `desktop-modal-focus-resize`, `desktop-ux-r3-access`, `desktop-ux-r4-access`: all-route toolbar, overflow, materials/transparency, modal draft/focus, offline recovery, asynchronous confirmation focus return, login recovery and five shared contrast combinations. Login recovery passed its assertions but logged the unhandled workspace rejection subsequently investigated as DUX-02.
- [header-red.log](evidence/design-ux/header-red.log): expected red at overflow focus; [header-focus-red.png](evidence/design-ux/header-focus-red.png). [header-green-first.log](evidence/design-ux/header-green-first.log): initial focused repair regression **1 passed**.
- [workspace-red.log](evidence/design-ux/workspace-red.log): expected red, missing inline recovery; [workspace-load-red.png](evidence/design-ux/workspace-load-red.png).
- [repairs-green.log](evidence/design-ux/repairs-green.log): **6 passed, 4 failed**. One genuine Proof reload finding and three new-test setup errors: after testing header focus, the test looked for Request URL while still on API onboarding. Corrected by clicking the real New request action. These errors are not counted as product defects or hidden as passing checks. [proof-reload-red.png](evidence/design-ux/proof-reload-red.png) preserves the genuine failure.
- [repairs-final.log](evidence/design-ux/repairs-final.log): **12 passed, 24.1 s** using `npx playwright test desktop-quality-next-design.spec.ts desktop-page-chrome.spec.ts desktop-modal-focus-resize.spec.ts --project=desktop-browser --workers=1`. Includes all three header theme cases, workspace recovery, stale identity response, full six-case page-chrome spec (including new delayed startup case) and modal regression. No retries or suppressed assertions. Earlier groups overlap; counts are not additive unique-test coverage.

## Scores and limits

Rubric: design judges hierarchy/readability, shared-component/material consistency, responsive control reach and accessible presentation; UX judges keyboard continuity, recovery, preserved user intent, truthful state and navigation restoration. A 9+ scoped score requires all confirmed material findings repaired with direct behavior evidence. A 9.8+ whole-product claim requires representative populated critical journeys, native/assistive-technology evidence and integrated-snapshot validation beyond these samples. A green test fraction is not the score.

| Vertical | Scoped post-repair score | Confidence and limit |
|---|---:|---|
| Design | **9.4/10** | Medium-high for shared chrome and inspected responsive states: consistent toolbar, semantic recovery, visible rings, actual light/dark/Warm captures and scoped contrast assertions. Does not rate every module's populated hierarchy, density or visual polish. |
| UX / usability | **9.3/10** | Medium for sampled shell journeys: two independently reproduced failures repaired, Proof restoration independently verified, focus/draft controls and recovery/identity cases pass. Native VoiceOver, real keyboard-only whole-product journeys and long-duration native composition remain untested. |

The requested 9.8 is **not established**. Remaining limitations are missing evidence, not invented assertions that native or unsampled flows are broken. R13's old native pane/visibility results are contextual history and were not rerun or presented as current evidence. Root owns fresh integrated UI checks/builds, Rust validation and final score reconciliation.
