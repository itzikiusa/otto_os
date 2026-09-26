# Round 1 — Help guides and tour film

Reviewer scope: `ui/src/modules/help`, with real browser verification of the exact generated instrumental MP4. Reviewed initial desktop light/dark, phone and tablet RTL screenshots; inspected guide rendering, search/navigation, captions, chapter seeking, error recovery, and responsive controls.

## Confirmed defects and repairs

- Getting started could not open from the phone list or first chapter: `open()` normalized its route to the overview, while the phone only displays articles with an explicit guide parameter. Reproduced with a failing browser assertion; explicit guide routes repair both entry points.
- The phone rail intercepted ArrowDown/ArrowUp originating inside the tour, moving keyboard focus into unrelated guides. Reproduced cancellation and focus movement from a chapter button. Rail keyboard navigation is now restricted to search and guide rows.
- The phone search control shrank under the height of the player/list; `flex-shrink: 0` keeps its intended height.
- Chapter and captions controls were below the documented 36px phone target. Added a phone minimum height.
- Loading text was hidden from assistive technology; it now exposes a status region.
- Removed the conflicting physical list-padding override so logical RTL padding applies.

## Verification setup

- New regression suite: `ui/e2e/desktop-ux-help.spec.ts`.
- Local generated video is optional via `OTTO_E2E_TOUR_VIDEO`; static tests do not require it.
- Video is served with range handling. Playwright service workers are blocked because the app's worker otherwise bypasses media routing and returns the Vite HTML fallback.
- `npx tsc --project tsconfig.e2e.json --noEmit`: passed.
- Full UI checks intentionally left to the parent audit per task ownership.


## Final verification

- Final browser run: **12 passed (1.8m)**, six cases each in desktop Chromium and iPhone WebKit. The two confirmed navigation defects failed before repairs and pass afterwards.
- Real `otto-tour-instrumental-20260925.mp4` decoded and advanced during playback in both engines. Chromium reported decoded audio bytes; WebKit exposed an audio track; volume was nonzero and unmuted. Verified no autoplay, seeking to Git at 90.97 seconds, loaded/active caption cues, CC hidden/showing, chapter-to-guide navigation, guide-to-playing-video navigation, and error-to-Retry recovery without autoplay.
- An initial WebKit assertion incorrectly demanded decoded frames before Play despite `preload="metadata"`; changed it to require metadata and then verify time advances after a user click. This was a test correction, not a playback defect.
- Phone light/dark RTL, unknown-guide recovery, no-results clearing, shortcut search and Enter selection, module routing, and no horizontal overflow pass.
- Focused axe WCAG A/AA Help guide scans: **zero violations** in light and dark. Evidence: `/tmp/otto-ux-help-axe.json`.
- `npx tsc --project tsconfig.e2e.json --noEmit`: passed. `node scripts/ui-guards.mjs`: passed (802 files, no new debt).
- Additional repair: unknown-guide CTA now actually opens Getting started on phone; back chevron mirrors RTL.

## Scores (after repairs)

| Page / surface | Layout | Interaction | Accessibility | States | Responsive | Mean |
|---|---:|---:|---:|---:|---:|---:|
| Help guides/search/detail | 9.5 | 9.5 | 9.5 | 9.6 | 9.5 | 9.52 |
| Tour player / chapters | 9.4 | 9.1 | 9.4 | 9.5 | 9.0 | 9.28 |
| Combined Help | 9.45 | 9.30 | 9.45 | 9.55 | 9.25 | 9.40 |

## Residual for round 2

**Confirmed phone usability gap:** tapping a chapter below the player starts the chosen segment but leaves the scroll position at the chapter. Video is above the viewport, so users hear music without seeing the demonstration until scrolling back. Evidence: `/tmp/otto-ux-help-iphone-portrait-playing.png` shows active Git chapter with video out of view; playback/caption assertions at the same point passed. Suggested direction: chapter activation should reveal the player using the nearest scrolling container, without making passive time updates move the page. Add an assertion that the playing video is visible in the viewport after selecting a lower chapter. This prevents a 9.5 tour score in this round.

Limits: media bytes were the exact local generated artifact passed through the real HTML video element, not an end-to-end GitHub release/CDN request. The parent owns published-byte/hash and soundtrack-generation verification. No claim of human listening quality or physical Tauri-device playback; WebKit browser coverage is the proxy. Guide axe covers the Git guide and shared Help structure, not every prose document or VoiceOver. Warm-dark and tablet RTL were visually inspected in the supplied initial screenshots; this round's interaction matrix is Chromium desktop/375px and iPhone WebKit.

## Artifacts

- Suite: `/Users/itziklavon/otto-ux-audit-20260925/ui/e2e/desktop-ux-help.spec.ts`
- Final Playwright results: `/tmp/otto-ux-help-results/.last-run.json` (`status: passed`, no failed tests).
- HTML test report: `/Users/itziklavon/otto-ux-audit-20260925/ui/e2e/.report-uxhelp/index.html`.
- Actual desktop playback/captions: `/tmp/otto-ux-help-desktop-browser-playing.png`.
- Phone chapter playback/scroll residual: `/tmp/otto-ux-help-iphone-portrait-playing.png`.
- Guide screenshots: `/tmp/otto-ux-help-guide-light.png`, `/tmp/otto-ux-help-guide-dark.png`.
- Phone RTL screenshots: `/tmp/otto-ux-help-{desktop-browser,iphone-portrait}-{light,dark}-rtl.png`.

No soundtrack, manifest, generated media, commits, or files outside Help + its new test were edited by this reviewer.
