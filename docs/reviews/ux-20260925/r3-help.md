# Round 3 — Help, walkthrough player, guides, hotkeys and first-run coach

Fresh reviewer 10/10; worked only in `/Users/itziklavon/otto-ux-audit-20260925`. No commits, subagents, real provider/account writes, or media changes. Parent explicitly granted `FirstRunCoach.svelte`, `AgentsPage.svelte`, and `ShortcutsOverlay.svelte` ownership. Read protocol, r1/r2 reports, parent notes, AGENTS and design/accessibility/review guidelines. Used systematic debugging, test-first reproduction and verification-before-completion.

## Verification status

**Final combined run: 60/60 passed (4.7m), exit0**, 30 each Chromium and WebKit. Supplemental settled screenshot rerun: **4/4 passed (15.0s), exit0**. Both `.last-run.json` files report passed with no failed tests. Earlier independent prior run: **34/34 passed**. Final E2E tsc, UI guards and scoped diff checks passed. All test processes have exited. Parent separately reported global Svelte0errors/0warnings, production build success and454unitspassed; those were not executed by this reviewer. The final combined run included22 Help/coach/shortcut axe scans with zero violations.

## Earlier repairs independently rechecked

All r1/r2 Help and coach regressions rerun, using the published original instrumental asset `/tmp/otto-tour-published-20260925.mp4` through a range-capable same-origin fixture. Verified actual playback/audio decoding/caption cues, no autoplay, chapter seek/reveal, passive updates preserving scroll, pending seek through error recovery, guide-to-video and video-to-guide navigation, readable long chapter names, all five theme variants, phone search/no-results/unknown guide recovery, keyboard list navigation, RTL modifier ordering in guide prose, and coach phone LTR/RTL input labels/wrapping/Browse reachability. Original soundtrack, release assets and captions unchanged. Independently rehashed test asset: SHA256 `0e12d85efd1116ae9029c20d88f0b09d7807d9706d6761b31fdaeb2f320538f4`, matching prior published-byte evidence.

## New confirmed defects repaired

1. **P2 — startup discarded a draft accepted by the visible coach.** `ui/src/modules/agents/AgentsPage.svelte:55`. Hold initial `/workspaces` response, type path and deliberate workspace name in the already-visible coach, release response: scratch-session loading removed/recreated the component and the name became empty. Initially mistaken for fixture settling; parent correctly requested deliberate delayed reproduction, which failed with expected `My deliberate draft`, received `""`. Keep the no-workspace coach mounted during scratch loading; real workspace changes, ordinary empty state, tiled/mission views and existing panes retain prior loading behavior. Final regression permits either deferring the initial form or retaining accepted input; current implementation retains it.
2. **P2 — native media caption changes left CC state stale.** `ui/src/modules/help/TourFilm.svelte:125`. Native controls operate on `TextTrack.mode`, while the external CC button previously only tracked its own clicks. Setting the real track to disabled left `CC on`/aria-pressed true. A cleaned-up track change listener synchronizes the external state once metadata is ready. Regression also verifies off survives error/Retry, avoiding new-video default-track events resetting the preference.
3. **P3 — Watch the tour ignored reduced motion.** `ui/src/modules/help/Walkthroughs.svelte:81,88`. With reduced motion enabled and the long shortcuts guide scrolled down, the real Watch button requested smooth scrolling. Both watch entry points now use instant scrolling for the preference. Regression checks the invoked scroll behavior and actual player visibility.
4. **P2 — optional skill discovery silently disappeared on error.** `ui/src/modules/agents/FirstRunCoach.svelte:89,270`. A 503 from `/library/bundled` removed the entire optional step, leaving no explanation or recovery. It now announces loading/error and offers inline Retry skills, with an in-flight guard. Contract-faithful fixtures verify retry, failed install, restored install button, successful install and installed status. All install requests intercepted; no provider folders touched.
5. **P3 — optional badge contrast fell below AA.** `ui/src/modules/agents/FirstRunCoach.svelte:271`. Whole-step opacity lowered the badge to 4.49 Native dark, 4.33 Warm light and 4.31 Warm dark. Removed the opacity rather than changing shared theme tokens. Five-theme coach axe scans now pass. The initial CLI-chip contrast suspicion was rejected: actual axe scans found no chip violation.
6. **P3 — phone coach targets too small.** `ui/src/modules/agents/FirstRunCoach.svelte:560`. Dismiss measured 23px high; launch had a 32px style. Scoped phone buttons now have a 36px minimum, dismiss also a 36px width, with header space reserved so its hit area does not overlap the title. No shared button override/baseline increase. Measured tests and LTR/RTL screenshots verify final layout.
7. **P3 — global shortcut sheet reversed modifiers in RTL.** `ui/src/shell/ShortcutsOverlay.svelte:77`. R2 fixed the guide's chips but not the global `?` sheet: modifier x64.59 followed key x42.50, rendering K before Command. The chord container is now explicitly LTR while the dialog stays RTL. Regression covers modifier position, axe, Escape close, viewport fit and actual screenshot.

## Deeper executed flows

- Full mocked coach flow: failed workspace creation retains fields → retry creates/selects a synthetic workspace → failed first-agent launch restores enabled action → retry opens returned session → exactly one delayed starter prompt targets that synthetic session. Verified submitted workspace path/name, session kind/title/cwd/source, prompt text and submit=true. All writes intercepted.
- No-provider fixture: no CLI found and disabled launch; Re-check detects synthetic tools; launch remains disabled until workspace exists.
- Delayed video resolver: select Git then Insights before metadata; latest requested chapter plays with correct current marker. Chromium fullscreen enters/exits the real video, preserves caption setting, then Read guide removes video and opens Insights. WebKit exercises the delayed chapter/navigation portions; native WebKit fullscreen is not claimed.
- Guide/player and coach main surfaces across Native light desktop, Native dark phone, Warm light tablet RTL, Warm dark phone RTL and Pro Dark desktop. Reduced motion exercised on coach variants and explicit Help scrolling. Shared `?` dialog specifically inspected Warm dark RTL phone.

## Scores after repairs

Columns are layout/readability, interaction, accessibility, states/recovery, responsiveness. Scores describe exercised UI plus rendered inspection, not production cloud mutations or native assistive technology. Each table row is a separate reviewed variant.

| Surface / variant | L | I | A | S | R |
|---|---:|---:|---:|---:|---:|
| Guides — Native light desktop | 9.6 | 9.6 | 9.5 | 9.6 | 9.5 |
| Guides — Native dark phone | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Guides — Warm light tablet RTL | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Guides — Warm dark phone RTL | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Guides — Pro Dark desktop | 9.6 | 9.6 | 9.5 | 9.6 | 9.5 |
| Tour — Native light desktop | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Tour — Native dark phone | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Tour — Warm light tablet RTL | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Tour — Warm dark phone RTL | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Tour — Pro Dark desktop | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Coach — Native light desktop | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Coach — Native dark phone | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Coach — Warm light tablet RTL | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Coach — Warm dark phone RTL | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Coach — Pro Dark desktop | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Shortcut sheet — Warm dark phone RTL | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |

Deductions reflect long phone scroll, dense chapter presentation, the coach's long diagnostic chips under stress, and limited keyboard/assistive-technology evidence beyond browser automation. Help hierarchy, wrapped chapter identities, readable prose/chords, reachable actions and explicit recovery support the grades. Coach now has genuine end-to-end UI mutation/recovery evidence rather than only the r2 layout check. No numerical scores assigned to uninspected native fullscreen/VoiceOver or other shortcut-sheet theme variants.

## Commands, logs and honest intermediate failures

Each Playwright invocation from `WT/ui` used this exact environment prefix:

```sh
OTTO_E2E_SLOT=ux3help OTTO_E2E_PORT=7859 OTTO_E2E_PW_PORT=5359 \
OTTO_E2E_SWEEP_ORPHANS=0 \
OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod \
OTTO_E2E_TOUR_VIDEO=/tmp/otto-tour-published-20260925.mp4
```

One process per slot; every invocation waited to exit before the next began. Each command used `--workers=1` and its corresponding output directory below.

- `npx playwright test desktop-ux-help.spec.ts desktop-ux-r2-help.spec.ts desktop-ux-onboarding.spec.ts --project=desktop-browser --project=iphone-portrait --output=/tmp/otto-ux-r3-help-prior-results`: **34 passed, 3.8m, exit0**, `/tmp/otto-ux-r3-help-prior.log`.
- New `desktop-ux-r3-help.spec.ts`, desktop-only, `/tmp/otto-ux-r3-help-red-results`: **7 failed/3 passed**, expected contrast/recovery/caption/motion failures and the first observed startup field loss; `/tmp/otto-ux-r3-help-red.log`.
- Intermediate `/tmp/otto-ux-r3-help-green1-results`: **4 failed/6 passed**; not final evidence because edits were applied while this intermediate invocation was beginning (initial edit command had the wrong cwd and changed no file). Early contrast captures were still pre-fix; the startup form issue remained. `/tmp/otto-ux-r3-help-green1.log`.
- New suite `--grep coach`, desktop-only, `/tmp/otto-ux-r3-help-coach-red2-results`: **3 failed/4 passed**. Expected 23px touch-target failures; a test-only attempt to inspect the Vite store through a direct import produced an unrelated store instance and was removed. Replaced with actual scratch response/paint waiting for ordinary settled-flow tests; the separate delayed-bootstrap regression then proved the real lifecycle defect. `/tmp/otto-ux-r3-help-coach-red2.log`.
- New suite both engines, `/tmp/otto-ux-r3-help-green2-results`: **20 passed, 1.8m, exit0**. `/tmp/otto-ux-r3-help-green2.log`.
- New suite `--grep 'RTL shortcut|unavailable provider'`, desktop-only, `/tmp/otto-ux-r3-help-shortcuts-red-results`: **1 failed/1 passed**, expected RTL chip position failure, provider recheck passed. `/tmp/otto-ux-r3-help-shortcuts-red.log`.
- New suite `--grep 'slow initial'`, desktop-only, `/tmp/otto-ux-r3-help-draft-red-results`: **1 failed**, deliberate draft became empty after released initial workspace discovery. `/tmp/otto-ux-r3-help-draft-red.log`.
- Final combined command:

```sh
npx playwright test desktop-ux-r3-help.spec.ts desktop-ux-r2-help.spec.ts \
  desktop-ux-help.spec.ts desktop-ux-onboarding.spec.ts \
  --project=desktop-browser --project=iphone-portrait \
  --output=/tmp/otto-ux-r3-help-final-results --workers=1 \
  > /tmp/otto-ux-r3-help-final.log 2>&1
```

- `npx tsc -p tsconfig.e2e.json --noEmit` → exit0; `/tmp/otto-ux-r3-help-tsc-final.log` (empty).
- `node scripts/ui-guards.mjs` → exit0,803files; `/tmp/otto-ux-r3-help-guards-final.log`. Parent caught an intermediate new global-class style hit; changed to feature-scoped `.coach button`, no baseline increase.
- Scoped `git diff --check` passed. Full Svelte check, global build and other suites owned by parent.

## Screenshots actually viewed

Using `view_image`, inspected these newly generated prior-regression images (prefix `/tmp/otto-ux-r3-help-prior-results/`):

- `desktop-ux-r2-help-native--3ea1d-rch-and-readable-long-guide-iphone-portrait/native-dark-phone-guide.png` and `native-dark-phone-error.png`.
- `desktop-ux-r2-help-warm-da-0dfc9-rch-and-readable-long-guide-iphone-portrait/warm-dark-phone-rtl-film.png`.
- `desktop-ux-r2-help-warm-li-4a557-rch-and-readable-long-guide-iphone-portrait/warm-light-tablet-rtl-guide.png`.
- `desktop-ux-r2-help-native--be6e6-rch-and-readable-long-guide-iphone-portrait/native-light-desktop-film.png`.
- `desktop-ux-r2-help-pro-dar-9b6c2-rch-and-readable-long-guide-iphone-portrait/pro-dark-desktop-film.png`.
- `desktop-ux-onboarding-firs-d95e5-s-reachable-on-a-phone-rtl--iphone-portrait/onboarding.png`.

Also viewed red Native dark phone and Warm light tablet coach captures, and green2 Native dark WebKit phone, Warm dark Chromium RTL phone, skill-loading/error-transition capture and actual Chromium fullscreen Insights playback. The green2 `coach-skill-error.png` captured a startup reload/loading transition, so it is not represented as settled error-state visual evidence; final settled error shot required below.

Final captures actually viewed:

- `/tmp/otto-ux-r3-help-final-results/desktop-ux-r3-help-coach-n-95123-ols-and-dismiss-persistence-iphone-portrait/native-light-coach.png`
- `/tmp/otto-ux-r3-help-final-results/desktop-ux-r3-help-coach-w-e5d88-ols-and-dismiss-persistence-iphone-portrait/warm-light-coach.png`
- `/tmp/otto-ux-r3-help-final-results/desktop-ux-r3-help-coach-p-5590f-ols-and-dismiss-persistence-iphone-portrait/pro-dark-coach.png`
- `/tmp/otto-ux-r3-help-evidence-results/desktop-ux-r3-help-RTL-sho-8671f-ords-and-keyboard-dismissal-iphone-portrait/shortcut-sheet-warm-dark-rtl.png`
- `/tmp/otto-ux-r3-help-evidence-results/desktop-ux-r3-help-RTL-sho-8671f-ords-and-keyboard-dismissal-desktop-browser/shortcut-sheet-warm-dark-rtl.png`
- `/tmp/otto-ux-r3-help-evidence-results/desktop-ux-r3-help-coach-f-8a543-skill-installation-recovers-iphone-portrait/coach-skill-error.png`
- `/tmp/otto-ux-r3-help-evidence-results/desktop-ux-r3-help-coach-f-8a543-skill-installation-recovers-desktop-browser/coach-skill-error.png`

Final initial shortcut capture was caught mid entrance animation, and the phone error was below the viewport. Corrected screenshot setup to disable finite animations and explicitly scroll Retry skills into view; no production edits. Supplemental command with same environment:

```sh
npx playwright test desktop-ux-r3-help.spec.ts --project=desktop-browser --project=iphone-portrait \
  --grep 'RTL shortcut|failed skill discovery' \
  --output=/tmp/otto-ux-r3-help-evidence-results --workers=1 \
  > /tmp/otto-ux-r3-help-evidence.log 2>&1
```

All4passed. Settled screenshots show opaque, legible shortcut sheet with Command before K, visible scrollable content and close action; error screenshots show actual explanatory text and reachable Retry skills. Screenshot transients were evidence-capture timing, not classified as production defects.

## Limits and useful next-round depth

- Chromium + Playwright WebKit are browser proxies, not native Tauri/physical iOS/VoiceOver. Fullscreen explicitly exercised only in Chromium. No physical keyboard/media accessibility panel interaction claim.
- Original byte-identical published MP4 is served through the fixture; real GitHub CDN redirect/auth transport remains parent-owned prior verification. No autoplay or audio-generation changes.
- Provider detection, skill install, workspace create, session create and starter input use contract-faithful intercepted transport. They establish UI behavior; they do not establish real install/filesystem/cloud side effects. Native folder chooser is not executed.
- `ws.load()` initial workspace-list errors still have no shared explicit retry/error state; code inspection only in this round, separate R4 shell/access candidate. This reviewer deliberately avoided a global workspace-store redesign. Current fix specifically prevents accepted no-workspace draft loss during scratch discovery.
- Not a complete content-accuracy audit of all36guide documents, a 200% zoom/custom accent matrix, exhaustive stalled-stream testing, or all shortcut-dialog theme/device cross-products. Reduced-motion explicit watch paths and current root theme fixtures are verified.
- Service workers blocked for fixture determinism. Production worker-enabled/offline behavior belongs to parent/R4 shared review.


## Handoff

Changed production files: `ui/src/modules/agents/AgentsPage.svelte`, `ui/src/modules/agents/FirstRunCoach.svelte`, `ui/src/modules/help/TourFilm.svelte`, `ui/src/modules/help/Walkthroughs.svelte`, `ui/src/shell/ShortcutsOverlay.svelte`. New meaningful regression suite: `ui/e2e/desktop-ux-r3-help.spec.ts` (13cases per engine). No existing test assertions weakened. No further source changes pending. This completes the assigned round3 review only; parent owns integration and mandatory rounds4/5.
