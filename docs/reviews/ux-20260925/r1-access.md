# Round 1 reviewer 10 — shared design/accessibility

Worktree `/Users/itziklavon/otto-ux-audit-20260925`. Owned changes: `ui/src/app.css`, `ui/src/lib/components/Modal.svelte`, new `ui/e2e/desktop-ux-r1-access.spec.ts`. No commits, module edits, or full npm check.

## Confirmed repairs

- **P2, RTL command corruption / directional spacing:** `ui/src/app.css:440`, `:449`, `:464`, `:469`. Seeded a real Skills Lab library skill containing Hebrew text, inline `./q --help`, a shell fence, lists and a quote. Phone RTL screenshot visibly rendered `q --help/.` before the fix. CSS inherited RTL direction; list padding and quote border were physical left properties. Added LTR + bidi isolation to code/pre, aligned pre to its own start, and converted lists/quote border to logical spacing. Regression failed before repair (4 theme tests red; last theme ran after repair), then all five theme combinations passed. This independently confirms/fixes the prior settings report residual.
- **P2, Cancel clipped offscreen in phone choice dialogs:** `ui/src/lib/components/Modal.svelte:173`. Exercised real `confirmer.choose` with the production bulk-close labels `Archive 123 sessions` and `Delete 123 sessions`, plus a long message, at 375×667. Cancel extended to x=-49.3 and was visibly clipped. Footer was an unwrappable flex row. Added `flex-wrap: wrap` and `flex-shrink: 0`; all actions remain inside the sheet and the message retains its own scrolling space. Verified Cancel by touch in iPhone WebKit as well as pointer in Chromium. No session deletion performed.

## Independent regression coverage

- New spec: five theme/scheme Markdown fixtures, including Warm light 375px RTL and Pro Dark 1024px RTL; computed bidi/spacing checks and page overflow.
- New Session → Browse → folder sheet: Tab and Shift+Tab wrap only inside the picker; first Escape closes just the picker and restores Browse; second closes New Session.
- Phone Navigator → Add Workspace: sheet takes keyboard ownership; Escape restores Add workspace inside the still-open drawer; second Escape restores Open navigator.
- Long choice content: all footer actions inside sheet and viewport, Cancel closes it. Shared component is invoked through its real public store; this test does not claim end-to-end bulk session deletion.
- Existing `desktop-ux-shell.spec.ts`: all 8 tests passed unchanged (navigator focus/return, prompt accessible name, dialog shortcut blocking, palette button Escape/Tab, Needs you expansion, three Home/prompt theme variants).
- Existing onboarding: LTR passed. RTL first run failed due a transient null `boundingBox` after coach had briefly mounted. Parent traced bootstrap remounting and corrected the test polling; this is recorded as a test race, not a confirmed layout defect. Actual screenshot exposed overlapping multiline CLI detection chips; parent owns and is repairing that separate coach issue.
- Parent's concern about Modal only checking `.sheet` while custom dialogs check all dialogs was examined. The reachable drawer→sheet and sheet→sheet paths above pass in both engines. No speculative focus-manager rewrite made. A sheet with a newly opened higher custom overlay remains a future explicit repro target.

## Commands and artifacts

All Playwright commands run from worktree `ui/` with:
`OTTO_E2E_SLOT=uxaccess OTTO_E2E_PORT=7821 OTTO_E2E_PW_PORT=5321 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/claude_ade/target/debug/ottod`.

1. `npx playwright test e2e/desktop-ux-r1-access.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r1-access-red`: 4 failed for missing isolation/direction, 2 passed; `/tmp/otto-ux-r1-access-red.log`.
2. Access + shell + onboarding combined: 15 passed, 2 failed (onboarding race and new test's Add Workspace title capitalization, corrected). `/tmp/otto-ux-r1-access.log`, output `/tmp/otto-ux-r1-access-results`.
3. Access `--grep phone`: 2 passed; 1 expected red for clipped Cancel. `/tmp/otto-ux-r1-access-phone-red.log`, output `/tmp/otto-ux-r1-access-phone-red`.
4. Final access suite across desktop-browser + iphone-portrait: result appended below. `/tmp/otto-ux-r1-access-final.log`, output `/tmp/otto-ux-r1-access-final`.
5. `npx tsc --noEmit -p tsconfig.e2e.json`: passed, log `/tmp/otto-ux-r1-access-types.log`. An initial unsupported top-level reducedMotion fixture option was corrected to `page.emulateMedia`.
6. `node scripts/ui-guards.mjs`: passed, 802 files, no ratchet regressions; log `/tmp/otto-ux-r1-access-guards.log`. Scoped `git diff --check` passed.

## Screenshots actually viewed

Under `/tmp/otto-ux-r1-access-results/`, viewed all five `desktop-ux-r1-access-Markd-*/markdown.png` images; nested-folder.png; three existing shell home-prompt.png images; LTR onboarding.png. Compared red phone Markdown to corrected Warm light phone screenshot. Native light/dark and Warm dark are readable and coherent; RTL quote/list markers now mirror, and command punctuation stays intact.

Viewed `/tmp/otto-ux-r1-access-phone-red/desktop-ux-r1-access-phone-64fcb-ong-choice-inside-the-sheet-desktop-browser/test-failed-1.png` showing Cancel clipped outside the sheet. Viewed corrected iPhone WebKit `long-confirm.png` and `drawer-sheet.png` under `/tmp/otto-ux-r1-access-final/`: all three choices visibly fit, controls are separated, and nested modal hierarchy/focus ring reads clearly.

## Scores (reviewed shared surfaces only)

| Surface / variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive |
|---|---:|---:|---:|---:|---:|
| Shared Markdown, Native light/dark | 9.6 | 9.5 | 9.5 | 9.5 | 9.5 |
| Shared Markdown, Warm dark | 9.6 | 9.5 | 9.5 | 9.5 | 9.5 |
| Shared Markdown, Warm light phone RTL / Pro Dark tablet RTL | 9.6 | 9.5 | 9.5 | 9.5 | 9.6 |
| Shared sheet/prompt/long-choice confirmation | 9.5 | 9.6 | 9.5 | 9.5 | 9.6 |
| Nested folder sheet and Navigator→workspace sheet | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 |
| Custom palette / broader overlay combinations | 9.3 | 9.3 | 9.2 | 9.2 | 9.3 |

These are narrow component scores, not entire Skills Lab, Home, Agents or application page-family ratings. 9.5 is supported for the repaired and explicitly exercised shared paths; the broader palette/overlay family does not yet earn 9.5 from this bounded pass.

## Limits / next-round targets

No native Tauri or VoiceOver session; WebKit browser emulation is not a physical device/keyboard. Theme captures cover content and prompts, not every component×theme×device combination. No app zoom, virtual keyboard occlusion, long context-menu touch scroll, or custom accent contrast measurement in this pass. No API contract or backend changes. Markdown long code lines and large tables remain useful next-round stress fixtures. Do not treat a source-only suspicion about DOM-order overlay ownership as a verified defect.

Final result: **16 passed (2.0m)** across Chromium desktop-browser and iPhone WebKit. `/tmp/otto-ux-r1-access-final/.last-run.json` is `{"status":"passed","failedTests":[]}`. No pending test processes.

Parent follow-up: long FirstRunCoach provider-chip overflow independently reproduced at 7px beyond its fixed height; parent applied scoped `height:auto; min-height:20px` and is repeating LTR/RTL verification. This shared reviewer did not edit the coach.
