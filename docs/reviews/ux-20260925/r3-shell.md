# Round 3 — fresh shell / Home / Agents / panels / desktop companion review

Worktree only: `/Users/itziklavon/otto-ux-audit-20260925`; isolated release daemon slot `ux3shell`, daemon7850, Vite5350. Read protocol, parent notes, AGENTS, design guidelines, R1/R2 shell reports. No commits, subagents, original-checkout edits, real service writes or provider-home mutations. Systematic debugging, test-first reproductions and verification workflow applied.

## Prior repairs independently rerun

Before modifications, `desktop-ux-shell`, `desktop-ux-r2-shell`, and `desktop-terminal-copy`: **24 passed (1.4m)**. Log `/tmp/otto-ux-r3-shell-prior.log`, result `/tmp/otto-ux-r3-shell-prior/.last-run.json` passed. This covers R1 Navigator/palette focus, Home shortcut isolation/mixed expansion/named prompts, R2 Outputs list/retry/races/keyboard/five themes, compact-selection protection, and clipboard paths.

## Confirmed repairs

- **P2 split/tile readability, Terminal.svelte:677.** Native1440×900 split rendered10px (viewed red screenshot). The original6px automatic minimum maintained80 columns by making text unreadable. Keep desktop >=80 columns, floor text at11px, and scroll overflow inside a named terminal region. Keyboard focus is added only while horizontal scrolling is needed. Full width returns to preferred13px. Final grid is computed before resize, avoiding transient narrow PTY sizes. Row reservation respects classic/overlay scrollbars. The fitting addon already uses xterm's measured internal render metrics; we use that same metric source, since DOM screen dimensions lag resize by a frame.
- **P2 delayed Outputs image URL survives unmount, OutputsPanel.svelte:65.** Browser held image GET, switched panel to Activity, resolved GET; created URL remained unrevoked. Unmount now invalidates request generation before URL cleanup; late blob resolution revokes itself. Closing an active preview also revokes its URL immediately.
- **P2 Outputs opens on no item and selected full identity is hover-only, OutputsPanel.svelte:175,310,326.** Loaded list had aria-selected=false for every row and no preview. Now opens first available item, respects explicit Close, and reopens on user selection. Full name/path wrap in bounded named scrolling regions, which join keyboard Tab order only if overflowing. Phone390×844 image preview and very long identity verified.
- **P2 tray endpoint errors falsely report no work, TrayPage.svelte:80,93,207.** Failing session fetch was swallowed into[] and displayed “No agents are working.” Session/approval/notification errors now propagate to inline alert+Retry. Last successful data stays available with explicit stale wording on later failures. Light/dark360×520 recovery and a long synthetic approval-list destination are checked.

- **P1 delayed terminal compacts can erase a later reading position, Terminal.svelte:450,604.** Corrected wheel-driven test proves actual visible first row104 jumps to137 after queued frames are released. Another settled resize could send a second compact while the first was pending, so the first response cleared the boolean and the next bypassed its selection guard. Optional compacts are now serialized; a pending one defers another. Its response rechecks both selection and a scrollback reading position, matching the existing request-side threshold. A delayed initial attach snapshot also allowed a compact before its epoch was known (WebKit red:2requests instead of1). Compact now waits until the initial epoch is received. Reconnect is separately exercised below; a live process replacement with a held selection remains a deeper target.

## Intermediate failures recorded honestly

- First new red run had 4 failures; last image teardown initially timed out because Activity was a folded tab. Corrected test uses existing tab keyboard navigation.
- Red2: 7 failed: six real assertions (split readability×2, default output, late URL, tray×2); initial long-scrollback snapshot setup failed to trigger compact. Not a product finding.
- First fit implementation measured DOM screen height immediately after resize. That old-frame/new-row ratio prevented grid stability confirmation. Prior delayed-selection test caught it. Replaced it with the same measured cell dimensions used by FitAddon; unchanged R2 regression passed again.
- Default preview added a legitimate bytes request to the old R2 list-retry test. Its synthetic report had no bytes route, creating a preview404 alert. Added a report-body fixture; kept all original assertions.
- A first deep-scroll test mistakenly read legacy `.xterm-viewport.scrollTop` under xterm6, which is always0. Parent caught this; that metric and its preservation claim are retracted. Corrected test drives the actual custom scrollbar with wheel, proves slider travel/overflow, compares visible first terminal row, and checks a real changed PTY geometry before releasing queued WS frames in order.

## Commands and current results

All Playwright commands use:
`OTTO_E2E_SLOT=ux3shell OTTO_E2E_PORT=7850 OTTO_E2E_PW_PORT=5350 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod`

- Prior suite:24passed as above.
- New initial repaired suite:6passed/1setup failure (`/tmp/otto-ux-r3-shell-fixed1.log`).
- Focused sizing/R2 selection/list recovery:5passed53.8s (`/tmp/otto-ux-r3-shell-sizing.log`). The old legacy-scroll metric in this run is invalid evidence, as explained above.
- Broad run: `npx playwright test e2e/desktop-ux-r3-shell.spec.ts e2e/desktop-ux-r2-shell.spec.ts e2e/desktop-terminal-copy.spec.ts e2e/desktop-split-layout.spec.ts e2e/desktop-rpanel-tabs.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-shell-final`:33passed/1failed3.7m. Failure proved a second optional compact was sent while the first remained pending (3requests total rather than attach+1compact). Passing cases include7nested split/layout checks, pinned/reachable panel tabs,4copy paths, prior12Outputs/session checks, repeated narrow↔wide font/grid/keyboard checks, Outputs lifetime/identity/close, tray and bar. Follow-up repairs and final reruns are listed below.
- `npx tsc -p tsconfig.e2e.json --noEmit`:passed, `/tmp/otto-ux-r3-shell-tsc.log`.
- `node scripts/ui-guards.mjs`:passed, `/tmp/otto-ux-r3-shell-guards.log`; no baseline increase.
- Direct Svelte compiler on3changed components:0warnings each, `/tmp/otto-ux-r3-shell-svelte.log`. Replaced initial region tabindex markup warnings with conditional DOM actions.
- `git diff --check` on owned files:passed. Parent owns full npm/check/build integration.

## Viewed visual evidence

Actually viewed red Native dark split at1440×900 (10px), repaired Native light/dark split at1440×900 (11px), phone390×844 Outputs image/full identity, Warm dark RTL390×844 and Warm light RTL1024×900 loaded Outputs, Native light/dark tray360×520, and native-dark assistant bar640×360. Sources are synthetic or repository-redacted fixtures, not user account/usage screenshots. Final screenshots live beside their tests under `/tmp/otto-ux-r3-shell-final/`; earlier viewed images under `/tmp/otto-ux-r3-shell-fixed1/` and `/tmp/otto-ux-r3-shell-verify/`.

## Remaining deeper targets / limits

- Physical Tauri vibrancy, native tray hide/show, system clipboard menu, VoiceOver and phone soft keyboard need native/manual evidence. Browser bridges are intentionally no-ops; no claim that browser navigation proves native focus/activation.
- Activity/Files: broader pending task/note draft updates, cross-session draft isolation and live filesystem interaction remain deeper targets. Earlier regression checks do not certify these asynchronous flows.
- Desktop assistant bar: verified loaded command surface, suggestion and Escape, not a full agent answer/cancellation/error workflow in the standalone native panel.
- Outputs live list has no removal UI/event; selection fallback exists when the selected item disappears, but a dedicated embedded-history prop-list replacement test remains a deeper target.
- Terminal deeper targets: font-family changes, saved zoom below11, many simultaneous read-only monitors, real process-epoch replacement while a selection exists. Exited-shell reconnect passed the final original-panel regression run.

Corrected scrollback red3:1failed at final visible-row assertion, expectedSCROLL-REVIEW-104/received137 (`/tmp/otto-ux-r3-shell-scroll-red3.log`). Earlier corrected setup expected one wheel event to reach top; xterm normalizes it to3rows, so the final test uses10real wheel events and compares the actual reading position rather than claiming scrollTop0.

## Final checks

- Corrected scrollback/serialization plus R2 and clipboard: `npx playwright test e2e/desktop-ux-r3-shell.spec.ts e2e/desktop-ux-r2-shell.spec.ts e2e/desktop-terminal-copy.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-shell-final2`: **26 passed (2.1m)**. Before the initial-attach regression was added.
- Initial WebKit10:8passed/2failed1.4m (`/tmp/otto-ux-r3-shell-webkit.log`). The two failures were keyboard-region scroll metrics. R2 access review had independently established Playwright WebKit does not natively arrow-scroll even a standalone overflow div, so these failures do **not** prove a native app defect. Explicit scoped key handlers now provide deterministic horizontal terminal / vertical identity scrolling. They run only on the exact focused region, leaving child xterm input untouched.
- New attach red plus keyboard recheck:3passed/1failed49.6s (`/tmp/otto-ux-r3-shell-webkit-deep.log`); only attach request count failed (expected1, received2). Then added the epoch-null guard.
- Final WebKit: `npx playwright test --config=playwright.ux3shell.config.ts --workers=1 --output=/tmp/otto-ux-r3-shell-webkit-final`: **11 passed (1.0m)**. `.last-run.json` passed. Temporary config copies base config and overrides only projects with `{name:'desktop-webkit',testMatch:/desktop-ux-r3-shell\.spec\.ts/,use:{browserName:'webkit',viewport:{width:1280,height:800}}}`; preserved at `/tmp/playwright.ux3shell.config.ts`, removed from worktree.
- Final E2E TypeScript:exit0 (`/tmp/otto-ux-r3-shell-tsc-final.log`). Final UIguards:exit0 (`/tmp/otto-ux-r3-shell-guards-final.log`). Latest3component direct compiler:0warnings each. Owned-path `git diff --check`:exit0.
- Original panels/session5test check initially4passed/1failed54.3s (`/tmp/otto-ux-r3-shell-panels.log`). It exposed an intermediate regression in preserving the exact terminal grid when expanding the right panel to leave <20columns visible. Restored this existing sliver guard and retained horizontal access; final recheck passed all five original cases (recorded below).

Latest final WebKit screenshots actually viewed: `readable-split.png` Native light/dark 1440×900, `outputs-identity-phone.png` Native light 390×844, `tray-recovered.png` Native dark 360×520, all under `/tmp/otto-ux-r3-shell-webkit-final/`. Both terminal screenshots show11px content with a contained scrollbar and focus ring; all80columns remain reachable. Phone output shows its full three-line name/two-line path and image, no page overflow.

No whole-app completion claim.

## Scores by reviewed page / important variant

These scores assess actual reviewed UI, not the number of tests. Native/manual environment limits are separate from unverified workflows.

| Page / variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsiveness | Overall |
|---|---:|---:|---:|---:|---:|---:|
| Shell Navigator / palette / prompts; desktop, phone RTL | 9.6 | 9.6 | 9.6 | 9.5 | 9.5 | 9.6 |
| Home populated cards / spaces / shared prompts; Native light/dark, Warm phone RTL | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 | 9.5 |
| Agents full terminal; desktop selection/copy/resize/reconnect | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.5 |
| Agents loaded Chat / Split; Native light/dark 1440×900 | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Agents nested desktop splits / tile layouts | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Agents phone/tablet terminal controls/output/tiled | 9.4 | 9.5 | 9.3 | 9.4 | 9.5 | 9.4 |
| Outputs; five themes, phone/tablet RTL, preview/list recovery/lifetime/keyboard | 9.6 | 9.6 | 9.6 | 9.6 | 9.6 | 9.6 |
| Activity task creation/persistence and panel tab coordination | 9.5 | 9.5 | 9.4 | 9.4 | 9.5 | 9.4 |
| Files / Notes panel workflows | 9.3 | 9.3 | 9.2 | 9.2 | 9.4 | 9.3 |
| Desktop tray; Native light/dark360×520, Warm dark long approvals | 9.5 | 9.5 | 9.5 | 9.6 | 9.5 | 9.5 |
| Desktop assistant bar; compact Native dark640×360 | 9.5 | 9.4 | 9.4 | 9.3 | 9.5 | 9.4 |

Deductions: mobile tiled accessibility, multi-pane reading and input remain less deeply exercised than desktop; Activity has main add/reload flow but limited asynchronous task transitions; Files/Notes retain limited fresh loaded-tree/edit/error evidence; the standalone bar has only command-surface/search/Escape coverage and no full answer/error/cancellation cycle. These are narrower review conclusions, not invented product defects. Desktop zoom and guest/read-only monitors should be explicitly deepened next round.

Final Chromium after **all** terminal guards and keyboard-region handlers: `npx playwright test e2e/desktop-ux-r3-shell.spec.ts e2e/desktop-ux-r2-shell.spec.ts e2e/desktop-terminal-copy.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-shell-chromium-final`: **27 passed (4.1m)**, log `/tmp/otto-ux-r3-shell-chromium-final.log`, `.last-run.json` passed. This includes unchanged delayed-selection/copy assertions, all 12 R2 cases, and all 11 R3 cases. Final WebKit Native light tray screenshot also viewed; both themes preserve footer controls and clean state layout.

Final original-panel regressions after restoring the <20column sliver guard: `npx playwright test e2e/desktop-history-tasks.spec.ts e2e/desktop-session-commands.spec.ts --grep 'activity panel|outputs panel|reconnect|terminal fits|expanding the right' --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-shell-panels-final`: **5 passed (48.7s)**. Log `/tmp/otto-ux-r3-shell-panels-final.log`, `.last-run.json` passed. Existing exact-grid assertions are unchanged. Covers Activity task add/reload, real fixture Outputs markdown, exited-shell reconnect, >100column wide terminal fit, expand/restore exact grid.

Owned source: `ui/src/lib/components/Terminal.svelte`, `ui/src/modules/panels/OutputsPanel.svelte`, `ui/src/modules/desktop/TrayPage.svelte`. Tests: new `ui/e2e/desktop-ux-r3-shell.spec.ts`; existing `ui/e2e/desktop-ux-r2-shell.spec.ts` adds only the missing synthetic report bytes route. No production changes outside ownership. No commits.

Final actual WebKit mobile/tablet: `npx playwright test e2e/sessions-mobile.spec.ts --project=iphone-portrait --project=ipad-portrait --workers=1 --grep 'session view:|pane header|viewing data:|writing commands:|tab bar:|tiled view|floating controls|drawers' --output=/tmp/otto-ux-r3-shell-mobile`: **13 passed, 3 existing device skips (58.4s)**. Log `/tmp/otto-ux-r3-shell-mobile.log`, `.last-run.json` passed. Skips: phone typing needs a real soft keyboard; two phone-only controls/drawer cases do not apply to iPad. Both device layouts, header bounds, terminal output, session switching and tiles pass; iPad actual typed command echoes. These existing tests take no passing screenshots, so this is behavioral evidence, not an additional visual-review claim.

All test processes completed; slot `ux3shell` released. Final source checks passed without raising UI guard baseline. Parent owns full npm/check/build/global integration and subsequent rounds. Next fresh review should prioritize terminal zoom/reset in narrow and shared/tiled panes; actual new PTY epoch replacing a held selection; Outputs embedded-list replacement/removal; loaded Files/Notes asynchronous navigation; richer Activity updates; standalone assistant answer/error/cancel. No confirmed feasible finding from this round remains unrepaired.
