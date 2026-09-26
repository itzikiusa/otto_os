# Round 4 — Help, guides, tour, shortcuts and first-run coach

Final-round reviewer 10/10. Work confined to `/Users/itziklavon/otto-ux-audit-20260925`; no commits or subagents. Parent granted FirstRunCoach ownership and a backward-compatible auth.refreshMeta result. Read AGENTS, design/accessibility/review guidelines, protocol, final-round override, and all prior Help/coach reports. Applied systematic debugging, test-first regression reproduction, verification-before-completion.

## Safety and media

All setup mutations/provider installs/session spawns remain intercepted, except the harness's own disposable fake CLIs. No provider-home writes, production publishes or real cloud mutations. Screenshots use synthetic fixtures. Media uses actual published instrumental MP4 `/tmp/otto-tour-published-20260925.mp4`; freshly checked SHA256 `0e12d85efd1116ae9029c20d88f0b09d7807d9706d6761b31fdaeb2f320538f4`. Original music, manifest and caption asset unchanged. Range-capable same-origin media fixture; service workers blocked for faithful interception.

## Verification commands

All commands from WT/ui with this environment:

```sh
OTTO_E2E_SLOT=ux4help OTTO_E2E_PORT=7870 OTTO_E2E_PW_PORT=5370 \
OTTO_E2E_SWEEP_ORPHANS=0 \
OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod \
OTTO_E2E_TOUR_VIDEO=/tmp/otto-tour-published-20260925.mp4
```

Prior independent command:

```sh
npx playwright test desktop-ux-r3-help.spec.ts desktop-ux-r2-help.spec.ts \
  desktop-ux-help.spec.ts desktop-ux-onboarding.spec.ts \
  --project=desktop-browser --project=iphone-portrait \
  --output=/tmp/otto-ux-r4-help-prior-results --workers=1 \
  > /tmp/otto-ux-r4-help-prior.log 2>&1
```

Independent prior rerun: **60/60 passed (6.8m), exit 0**. `.last-run.json` reports passed with no failed tests. This independently rechecks all R3 native CC, reduced motion, skills discovery/install retry, five-theme coach contrast/touch targets, RTL shortcut sheet, unavailable-provider gating and delayed-bootstrap draft retention, alongside R1/R2 media/guide regressions. Final combined verification also passed, as detailed below.


## Fresh screenshots actually inspected

Using `view_image`, inspected these exact files beneath `/tmp/otto-ux-r4-help-prior-results/`:

- `desktop-ux-r2-help-native--be6e6-rch-and-readable-long-guide-iphone-portrait/native-light-desktop-film.png`
- `desktop-ux-r2-help-warm-li-4a557-rch-and-readable-long-guide-iphone-portrait/warm-light-tablet-rtl-guide.png`
- `desktop-ux-r2-help-pro-dar-9b6c2-rch-and-readable-long-guide-iphone-portrait/pro-dark-desktop-film.png`
- `desktop-ux-r2-help-native--3ea1d-rch-and-readable-long-guide-iphone-portrait/native-dark-phone-error.png`
- `desktop-ux-r2-help-warm-da-0dfc9-rch-and-readable-long-guide-iphone-portrait/warm-dark-phone-rtl-guide.png`
- `desktop-ux-r3-help-coach-n-95123-ols-and-dismiss-persistence-iphone-portrait/native-light-coach.png`
- `desktop-ux-r3-help-coach-w-e5d88-ols-and-dismiss-persistence-iphone-portrait/warm-light-coach.png`
- `desktop-ux-r3-help-coach-w-7f785-ols-and-dismiss-persistence-iphone-portrait/warm-dark-coach.png`
- `desktop-ux-r3-help-RTL-sho-8671f-ords-and-keyboard-dismissal-iphone-portrait/shortcut-sheet-warm-dark-rtl.png`

Observed: clear Help hierarchy, fully wrapped chapter names, opaque readable article/card surfaces, coherent RTL layout and modifier-first chords. Phone coach remains scroll-heavy under deliberately long CLI diagnostic versions, but controls remain reachable; the ordinary desktop/tablet layout has sufficient breathing room. The phone media-error image motivated direct target-size measurements rather than treating overflow tests as sufficient.

## Confirmed repairs — all verified

1. **P2 — Browse overwrote an explicitly typed workspace name.** `ui/src/modules/agents/FirstRunCoach.svelte:346`. Typed `Deliberate project title`, opened the real FolderPicker UI against synthetic folder data, exercised failed browse → Retry → keyboard Use this folder. The name became `chosen-directory`. Cause: the callback reset `wsNameTouched` to false, re-enabling automatic path-derived naming. Removed that reset; automatic suggestions continue for untouched names. Regression also checks selected path and focus restoration to Browse.
2. **P2 — provider recheck had no pending state or failure feedback.** `ui/src/modules/agents/FirstRunCoach.svelte:42,246`; supporting backward-compatible result in `ui/src/lib/stores/auth.svelte.ts:66`. Hold `/meta` after keyboard activation: Re-check stayed enabled with no checking label. Release as 503: no error appeared, leaving the stale missing-CLI state indistinguishable from a successful check. Confirmed in the dedicated second red run, not inferred from the earlier cold-start timeout. Added guarded pending state, disabled Checking… action, inline status explanation, and retry recovery. `refreshMeta()` now reports boolean success while preserving its existing quiet failure behavior for Settings callers.
3. **P3 — Help phone recovery/search targets were undersized.** `ui/src/modules/help/TourFilm.svelte:486`, `ui/src/modules/help/Walkthroughs.svelte:373`. Measured actual browser bounds after a media error: Retry 22 px, Open in browser 22 px, search input 18 px. Recovery actions now have 36 px minimum height; phone search input is 36 px inside a 38 px frame. Existing chapter/CC sizing remains intact. No shared CSS or baseline increases.

## New coverage and honest rejected candidates

- New four-case suite exercises held/failing/recovered provider discovery, actual folder-picker failure/retry/keyboard selection with deliberate draft retention, real MP4 pause/end/keyboard replay preserving CC, and direct phone recovery/search target bounds.
- Initial full new-case red run had 4 failures: two confirmed product issues (name overwrite and target sizes), one `page.goto` cold-start timeout before the UI, and one incorrect test assertion that the last chapter remains current at the film's end. `chapterAt` deliberately uses half-open intervals; after the last interval there is no current chapter. That is coherent ended-state behavior, not a proven defect. Removed that invented requirement; retained actual ended, paused, replay and caption assertions.
- New tests now wait for DOMContentLoaded and explicit UI readiness, with a 120 s test ceiling for cold Vite startup under concurrent workspace compilation. No old assertion weakened.
- Dedicated provider red rerun failed both intended assertions: no disabled Checking… action and no inline failed-check status. Log `/tmp/otto-ux-r4-help-provider-red.log`; artifacts `/tmp/otto-ux-r4-help-provider-red-results`.
- Red commands used `desktop-ux-r4-help.spec.ts --project=desktop-browser --workers=1`; first output `/tmp/otto-ux-r4-help-red-results` and log `/tmp/otto-ux-r4-help-red.log`; provider rerun added `--grep 'provider recheck'`.
- A test-only edit command initially used the wrong relative path from UI cwd and changed nothing. Immediately applied it from the correct root before the provider rerun loaded its test; no production edit or result claimed from that failed command.

## Final gates

Combined command adds `desktop-ux-r4-help.spec.ts` to the prior four suites, with both projects and `--output=/tmp/otto-ux-r4-help-final-results --workers=1`; stdout/stderr `/tmp/otto-ux-r4-help-final.log`. **68/68 passed (8.1m), exit 0**, 34 per engine. `.last-run.json` reports `passed` with no failed tests. All test processes have exited. The combined run includes 22 axe scans with zero violations across Help, coach and shortcut surfaces.

`npx tsc -p tsconfig.e2e.json --noEmit` passed, exit 0; `/tmp/otto-ux-r4-help-tsc-final.log` empty. `node scripts/ui-guards.mjs` passed, exit 0, 805 files; `/tmp/otto-ux-r4-help-guards-final.log`. Scoped `git diff --check` passed. Full npm check/build owned by parent; this reviewer did not run those global gates.

## Repaired-state visual inspection

Actually viewed with `view_image`, all beneath `/tmp/otto-ux-r4-help-final-results/`:

- `desktop-ux-r4-help-phone-t-d04c7-ave-reachable-touch-targets-iphone-portrait/tour-recovery-touch-targets.png`
- `desktop-ux-r4-help-folder--286ca-etry-and-keyboard-selection-iphone-portrait/chosen-folder-preserved-name-rtl.png`
- `desktop-ux-r4-help-provide-29bf1--without-duplicate-requests-iphone-portrait/provider-recheck-error-phone.png`
- `desktop-ux-r4-help-tour-pa-406a8-preserve-caption-preference-iphone-portrait/tour-replayed.png`
- `desktop-ux-r2-help-native--3ea1d-rch-and-readable-long-guide-iphone-portrait/native-dark-phone-error.png`
- `desktop-ux-r3-help-coach-p-5590f-ols-and-dismiss-persistence-iphone-portrait/pro-dark-coach.png`
- `desktop-ux-r3-help-coach-n-f7d56-ols-and-dismiss-persistence-iphone-portrait/native-dark-coach.png`

Both light and dark recovery states have visibly enlarged, unclipped actions and a comfortably tappable search field. Warm dark RTL shows the explicit project title preserved with visible Browse focus and LTR path input. Provider error copy is visible directly below the still-reachable Re-check button. The replay capture shows native playing control state, focus on the first chapter and CC off, preserving the user's original film. These are real browser captures, not mocks of screenshots.

## Limits

Chromium and Playwright WebKit are browser proxies; native Tauri controls, physical VoiceOver, macOS fullscreen UI and physical touch are not claimed. Chromium DOM fullscreen enter/exit is rerun in R3; WebKit's native fullscreen control panel is not driven. Media bytes are exact published bytes, but this suite routes them locally rather than traversing GitHub/CDN redirects. No soundtrack-generation/listening judgement added.

Provider discovery/install, workspace creation, first-session creation and starter input use contract-faithful intercepted transport; folder navigation uses synthetic filesystem responses. These establish UI behavior without touching real provider homes. Shared first-account onboarding/workspace-store bootstrap errors are owned by access/shell; the Help-owned accepted-coach-draft regression was independently rerun. This is not a content-accuracy audit of every sentence in all 36 guides, a full 200% zoom/custom-accent cross-product, an exhaustive streamed-media stall test or a worker deployment test. No fifth-round work is proposed.


Also viewed final Chromium `provider-recheck-error-phone.png` and `chosen-folder-preserved-name-rtl.png` in the matching `desktop-browser` directories, plus `desktop-ux-r3-help-queued--05f36-d-survive-fullscreen-return-desktop-browser/tour-fullscreen.png`. These confirm the same repaired setup behavior in Chromium and actual fullscreen Insights chapter playback from the unchanged film.

## Scores after repairs

Scores describe the inspected UI and executed faithful browser flows. L = layout/readability; I = interaction; A = accessibility; S = states/recovery; R = responsiveness. No automatic increase for the round number or absence of production writes. No known material defect remains in the exercised owned flows.

| Surface / important variant | L | I | A | S | R |
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

Clear hierarchy, readable long prose/chords, wrapped chapter identities, keyboard operation, explicit retry states and actual setup mutation/recovery flows support the grades. Deductions remain for long phone scrolling, dense fifteen-chapter presentation and long diagnostic provider chips under stress. Native control panels and untested combinations are unscored rather than implicitly covered by these rows. Scores retain R3's calibrated levels after repairing newly exposed small gaps; they do not claim every possible interaction is perfect.

## Handoff

Production changes: `ui/src/modules/agents/FirstRunCoach.svelte`, `ui/src/lib/stores/auth.svelte.ts`, `ui/src/modules/help/TourFilm.svelte`, `ui/src/modules/help/Walkthroughs.svelte`. New regression file: `ui/e2e/desktop-ux-r4-help.spec.ts` (four cases per engine). Source stable, all checks above completed, no further edits pending. Parent owns global checks, integration, PR/CI/merge and reinstall. This completes the assigned final round4 scope; no round5 deferral.
