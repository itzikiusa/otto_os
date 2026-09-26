# Round 2 — Help / tour / limited onboarding recheck

Fresh reviewer 10/10. Work stayed in `/Users/itziklavon/otto-ux-audit-20260925`; no commits, subagents, user runtime changes, provider installs, or real account writes. Read the review protocol, AGENTS, design guidelines/checklist/accessibility, `r1-help.md`, and `media.md`. Used systematic debugging, test-first reproduction, and verification-before-completion.

## Result

**34/34 final browser checks passed (1.6m)**: 17 each on desktop Chromium and iPhone WebKit. This includes all 12 earlier Help checks, four independently rerun onboarding layout checks, and 18 new Help checks. The dedicated iPad portrait Warm light RTL check also **passed (15.6s)**, for **35 final checks total**. All eleven Help guide axe scans returned **zero WCAG A/AA violations**. E2E TypeScript, UI guards, and scoped diff whitespace checks pass. Both final `.last-run.json` files report `passed` with no failed tests.

## Prior repairs independently verified

- Getting started opens through both the phone guide row and first chapter guide button.
- Unknown-guide recovery opens an actual article on phone.
- Search/no-results clearing, shortcut query and Enter, Git guide-to-module routing work.
- Tour arrow keys do not trigger guide-list navigation or steal focus.
- Phone search and chapter/CC controls remain readable and reachable; RTL back icon mirrors.
- Real video advances in Chromium and WebKit, audio decodes, captions load and toggle, Git chapter seeks, active cues appear, guide → Watch this part plays, and ordinary Retry does not autoplay.
- Independently hashed the actual test asset `/tmp/otto-tour-published-20260925.mp4`: SHA256 `0e12d85efd1116ae9029c20d88f0b09d7807d9706d6761b31fdaeb2f320538f4`, matching the parent's downloaded/published evidence. Served these bytes through ranged fixture responses into real HTMLVideoElements, with service workers blocked. No soundtrack/media changes.

## Confirmed findings repaired

1. **P2 — chapter selection played the movie out of view.** `ui/src/modules/help/TourFilm.svelte:141`. On 375×812, scroll to Insights, click its chapter: playback starts while the video top remains `-379px`. The new regression failed before the fix. Explicit `playAt()` requests now reveal the frame; pending requests reveal again after metadata/recovery. `timeupdate` does not scroll. Pointer and Enter activation, passive chapter changes, and failed-media → chapter recovery pass in both engines. Instant nearest scrolling respects reduced motion and preserves already-visible placement. Final screenshot waits for seeking to finish and playback to advance, and shows the actual Insights frame.
2. **P3 — RTL reverses shortcut chip order.** `ui/src/modules/help/Walkthroughs.svelte:575`. The phone RTL Keyboard shortcuts guide displayed `K ⌘`; measured modifier x=107.17 and key x=85.86. Explicit LTR direction on key-chip groups preserves `⌘ K` in prose, tables and search results while retaining RTL page layout. New position-based regression failed first, then passes in both engines; final Warm dark RTL screenshot inspected.
3. **P3 — chapter columns truncate the sections being taught.** `ui/src/modules/help/TourFilm.svelte:450`. At 1440px, the Mission Control chapter hid 48px of its name; its tooltip only said the start time. Titles now wrap, while timestamps stay on one line. New desktop regression failed first, then passes in both engines. Final Pro Dark desktop and Warm light tablet RTL screenshots show full names. The first narrower Chromium test did not reproduce (different column arrangement); the corrected 1440px test targets the visually confirmed desktop layout instead of weakening an assertion.

## Onboarding recheck (separate, limited scope)

Parent granted ownership of `ui/e2e/desktop-ux-onboarding.spec.ts`. Initial independent run: Chromium LTR/RTL passed; WebKit LTR/RTL failed because its service worker bypassed the empty-workspace fixture. The page showed an actual isolated seeded E2E workspace, so the expected Browse form did not exist. Added `serviceWorkers: 'block'`, preserving every original width, clipping, chip and accessible-name assertion. Added a synthetic bundled-skills response so screenshot evidence contains no host installation state.

Final **4/4 onboarding checks pass**. Viewed LTR and RTL screenshots: coach fits, long fake-CLI chips wrap, form labels are exposed, paths remain LTR, Browse stays reachable, and the full coach scrolls. No FirstRunCoach production changes were necessary. This establishes the repaired phone setup layout, **not** a complete workspace-create / skill-install / first-session-launch audit. Those workflows were not executed by this reviewer; no full onboarding workflow score is assigned. Inspected layout/readability, input labeling and responsiveness each rate 9.5 for this limited surface.

## Deeper coverage and scores

Both engines exercised each listed variant: real loaded video/no autoplay; pointer and keyboard CC controls; error presentation and Retry back to paused media; search ArrowDown/Enter; long Keyboard shortcuts guide; zero Help-scoped axe violations; no horizontal overflow. Unknown guide/no results/guide navigation use the re-run prior suite. Explicit chapter and recovery flows have separate tests in both engines.

Scores are for the verified surfaces, not averaged with onboarding or unrelated modules. Columns: layout/readability, interaction, accessibility, states/recovery, responsiveness.

| Surface / variant | Layout | Interaction | A11y | States | Responsive | Mean |
|---|---:|---:|---:|---:|---:|---:|
| Guides — Native light desktop | 9.6 | 9.6 | 9.5 | 9.6 | 9.5 | 9.56 |
| Guides — Native dark phone | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.54 |
| Guides — Warm light tablet RTL | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.54 |
| Guides — Warm dark phone RTL | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.54 |
| Guides — Pro Dark desktop | 9.6 | 9.6 | 9.5 | 9.6 | 9.5 | 9.56 |
| Tour — Native light desktop | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.54 |
| Tour — Native dark phone | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.54 |
| Tour — Warm light tablet RTL | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.54 |
| Tour — Warm dark phone RTL | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.54 |
| Tour — Pro Dark desktop | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 | 9.54 |

The improved tour scores reflect repaired chapter visibility and fully readable names, with direct playback/recovery proof. Remaining deductions reflect the dense fifteen-chapter presentation and long scroll on phone, rather than a known blocked interaction. Guide scores reflect clear hierarchy, legible prose and functional keyboard navigation; phone long-form content is naturally scroll-heavy. No known material Help UX defect remains in these tested flows. Scores do not imply testing every document, all device/theme cross-products, or physical assistive technology.

## Exact verification commands and logs

All Playwright commands ran in `/Users/itziklavon/otto-ux-audit-20260925/ui` with these environment values (passed inline on each actual invocation):

```sh
export OTTO_E2E_SLOT=ux2help
export OTTO_E2E_PORT=7849
export OTTO_E2E_PW_PORT=5349
export OTTO_E2E_SWEEP_ORPHANS=0
export OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
export OTTO_E2E_TOUR_VIDEO=/tmp/otto-tour-published-20260925.mp4

npx playwright test desktop-ux-help.spec.ts desktop-ux-onboarding.spec.ts --project=desktop-browser --project=iphone-portrait --output=/tmp/otto-ux-r2-help-prior-results --workers=1 > /tmp/otto-ux-r2-help-prior.log 2>&1
# Initial: 14 passed, 2 failed (onboarding WebKit fixture bypass, corrected).

npx playwright test desktop-ux-r2-help.spec.ts --grep 'chapter activation' --project=desktop-browser --output=/tmp/otto-ux-r2-help-red-results --workers=1 > /tmp/otto-ux-r2-help-red.log 2>&1
# Expected failure: video top -379.

npx playwright test desktop-ux-r2-help.spec.ts desktop-ux-help.spec.ts desktop-ux-onboarding.spec.ts --project=desktop-browser --project=iphone-portrait --output=/tmp/otto-ux-r2-help-results --workers=1 > /tmp/otto-ux-r2-help.log 2>&1
# Intermediate: 30 passed (2.8m).

npx playwright test desktop-ux-r2-help.spec.ts --grep 'RTL keyboard|long chapter' --project=desktop-browser --output=/tmp/otto-ux-r2-help-readability-red-results --workers=1 > /tmp/otto-ux-r2-help-readability-red.log 2>&1
# RTL failed as expected; narrower Chromium title case passed.

npx playwright test desktop-ux-r2-help.spec.ts --grep 'long chapter' --project=desktop-browser --output=/tmp/otto-ux-r2-help-title-red-results --workers=1 > /tmp/otto-ux-r2-help-title-red.log 2>&1
# Expected failure at observed 1440px desktop layout: 48px hidden text.

npx playwright test desktop-ux-r2-help.spec.ts desktop-ux-help.spec.ts desktop-ux-onboarding.spec.ts --project=desktop-browser --project=iphone-portrait --output=/tmp/otto-ux-r2-help-final-results --workers=1 > /tmp/otto-ux-r2-help-final.log 2>&1
# Final: 34 passed (1.6m), exit 0; .last-run.json status passed.

npx playwright test desktop-ux-r2-help.spec.ts --grep 'warm-light-tablet' --project=ipad-portrait --output=/tmp/otto-ux-r2-help-ipad-results --workers=1 > /tmp/otto-ux-r2-help-ipad.log 2>&1
# Passed: 1 test (15.6s), exit 0; .last-run.json status passed.

npx tsc -p tsconfig.e2e.json --noEmit > /tmp/otto-ux-r2-help-tsc-final.log 2>&1
# Passed, exit 0.
node scripts/ui-guards.mjs > /tmp/otto-ux-r2-help-guards-final.log 2>&1
# Passed: 803 files; no new debt. Baseline not changed.
```

From the audit root, `git diff --check -- ui/src/modules/help/TourFilm.svelte ui/src/modules/help/Walkthroughs.svelte ui/e2e/desktop-ux-r2-help.spec.ts ui/e2e/desktop-ux-onboarding.spec.ts` passed. Full `npm run check` is deliberately left to the parent per shared-run protocol. Browser logs include expected worker-blocking and NO_COLOR harness warnings.

## Screenshots actually viewed

Viewed the original chapter-out-of-view image, failed onboarding fixture image, intermediate guide/tour views across all five themes, and these **final** synthetic browser captures with `view_image`:

- `.../desktop-ux-r2-help-chapter-a05c9-s-preserve-reading-position-iphone-portrait/chapter-revealed.png`
- `.../desktop-ux-r2-help-warm-da-0dfc9-rch-and-readable-long-guide-iphone-portrait/warm-dark-phone-rtl-guide.png`
- `.../desktop-ux-r2-help-warm-li-4a557-rch-and-readable-long-guide-iphone-portrait/warm-light-tablet-rtl-film.png`
- `.../desktop-ux-r2-help-native--3ea1d-rch-and-readable-long-guide-iphone-portrait/native-dark-phone-error.png`
- `.../desktop-ux-r2-help-pro-dar-9b6c2-rch-and-readable-long-guide-iphone-portrait/pro-dark-desktop-film.png`
- `.../desktop-ux-r2-help-native--be6e6-rch-and-readable-long-guide-desktop-browser/native-light-desktop-error.png`
- `.../desktop-ux-onboarding-firs-d95e5-s-reachable-on-a-phone-rtl--iphone-portrait/onboarding.png`

Here `...` is `/tmp/otto-ux-r2-help-final-results`. The test artifacts contain all three film/error/guide shots for each theme in each engine; the final evidence above specifically verifies the actual repaired states. Earlier LTR onboarding screenshot was also visually inspected; final RTL screenshot uses synthetic skills.

Also viewed actual iPad-project `warm-light-tablet-rtl-guide.png` and `warm-light-tablet-rtl-error.png` under `/tmp/otto-ux-r2-help-ipad-results/desktop-ux-r2-help-warm-li-4a557-rch-and-readable-long-guide-ipad-portrait/`: guide prose/chords and the error card remain readable, with reachable Retry and no clipping.

## Limits / follow-up scope

- Chromium and Playwright WebKit, not the native Tauri binary or physical iOS hardware/VoiceOver.
- The media is the byte-identical published asset through an intercepted ranged endpoint. The browser itself did not traverse GitHub/CDN redirects; parent owns that deployment verification.
- Runtime failure is tested using aborted media requests and video error events; no real network was disabled. No claim of exhaustive stall/timeout behavior.
- Missing-build manifest/guides cannot occur in this build and were not synthesized. Per-guide content accuracy across all 36 documents, 200% zoom, arbitrary accent colors and full onboarding mutations remain beyond this focused round.
- Media soundtrack/listening quality was not changed or regraded.

Changed files: `ui/src/modules/help/TourFilm.svelte`, `ui/src/modules/help/Walkthroughs.svelte`, new `ui/e2e/desktop-ux-r2-help.spec.ts`, and the parent-authorized fixture correction in `ui/e2e/desktop-ux-onboarding.spec.ts`.
