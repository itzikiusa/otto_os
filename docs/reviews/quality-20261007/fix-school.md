# School repairs — evidence ledger

Baseline: `196048df`, working branch `fix/quality-20261007`. Scope: the Classrooms Home widget and its School model, scene ownership, screen transport, and focused tests. The corridor/classroom metaphor and the 36 front + 6 engine + 3 bench + headmaster geometry budget remain intact.

## Findings and regressions

| Finding | Regression | Baseline evidence | Repair / validation |
|---|---|---|---|
| UX-03, T5 complete membership | `school-life.test.ts`: complete membership keeps overflow sessions actionable while geometry stays capped | Coordinator ran `node --test unit/school-life.test.ts`; new assertion failed: 45 members, expected 51. `/tmp/otto-quality-20261007-school-members-red.log` | `Room.members` and `School.kids` retain all eligible input sessions; `Room.kids` and `Room.bench` remain bounded scene inputs. Browser overflow row/menu tests added. Focused unit green: 23/23 across actors/life/screens, coordinator log `/tmp/otto-quality-20261007-school-unit-green.log`. |
| NS5 hidden polling | `school-screens.test.ts`: hidden/deferred/reveal/stop and initial-hidden/cap cases | Coordinator ran `node --test unit/school-screens.test.ts`; both failed: hide did not abort, initial hidden poll fetched 12 instead of zero. `/tmp/otto-quality-20261007-school-screens-red.log` | Hide cancels timer and aborts the batch; late aborted feeds are suppressed. Reveal queues one fresh batch after any abort-insensitive transport settles. Stop removes the visibility listener. Focused unit green: 23/23 across actors/life/screens, coordinator log `/tmp/otto-quality-20261007-school-unit-green.log`. |
| NS3 actor resources | `school-actors.test.ts`: real Three SkeletonUtils clones, bone textures, active animation binding, duplicate skeleton references, 10 kid/headmaster visits | Coordinator ran `node --test unit/school-actors.test.ts`; new shared-disposer boundary assertion failed (undefined). `/tmp/otto-quality-20261007-school-actors-red.log`. This proves a missing disposal boundary; the original missing cleanup was also source-traced, but this is not a measured GPU leak. | Shared `disposeActor` stops/uncaches mixer, disposes unique owned skeletons and detaches root. Both individual removal and room teardown use it. Shared geometry/materials remain template-owned. Focused unit green: 23/23 across actors/life/screens, coordinator log `/tmp/otto-quality-20261007-school-unit-green.log`. |
| D1 card keyboard | `desktop-home-classrooms-3d.spec.ts`: native card controls retain Enter/Space | Added actual focused DOM button sequences: screen/back, Check on, Kick confirmation/Cancel, Close details and Open session. | Component repair applied; focused browser green pending. |
| D2 visible keyboard context | `desktop-home-classrooms-3d.spec.ts`: keyboard companion reveals focused row/menu | Asserts real list geometry, focused row/menu viewport bounds, keyboard menu/open action. | Component repair applied; focused browser green pending. |
| UX-04 / NS4 restoration | `desktop-home-classrooms-3d.spec.ts`: delayed asset restoration, list-first and list/3D remount | Holds actual kit request and checks persisted room before allowing completion; tests explicit Corridor and both mode toggles. | Component repair applied; focused browser green pending. |
| D4 load states | `desktop-home-classrooms-regressions.spec.ts`: initial School loading and failed data use truthful load states and Retry | Delays global sessions response with empty real workspace, fails it, checks load wording and Retry recovery. | Component repair applied; focused browser green pending. |

The new browser regressions run independently; original mutation-driven School journeys retain their existing serial scope. The no-WebGL overflow fixture uses synthetic intercepted session GETs, avoiding 46 spawned shells. It checks 1440 px and 390 px layouts, real keyboard menu/open actions and horizontal overflow.

## Limits

No native WKWebView/VoiceOver claim, long-term GPU/RSS leak claim, or visual-quality score follows from unit tests. Actual rendered cleanup, light/dark/phone screenshots and their inspection, RTL/reduced-motion checks, whole-UI type checking and the final affected-consumer gate remain coordinator-owned validation. No live user sessions/state, commits or published artifacts were changed by this worker.


## Browser baseline checkpoint

Coordinator ran the original eight focused browser cases before the widget edits. Four executed and failed: native Enter on Back to room navigated to the session instead (debug handle disappeared); initial loading status was absent; desktop and phone overflow fixtures rendered 42 rather than 46 rows. Four cases were skipped because an inherited outer serial scope still governed the nested default suite. This was test grouping, not a runtime pass. Original mutation-driven journeys now live inside their own serial describe; the new regressions have no serial ancestor.

The component now guards stage shortcuts by actual event target/defaultPrevented; reveals the screen-reader list on focus-within; feeds full membership into list actions while restricting scene-only actions to seated actors; captures room configuration before asset loading and persists only guarded scene-originated navigation; and passes `empty=true` to the initial/scene LoadState branches.

Additional browser assertions cover actual rendered skeleton allocation/disposal for ten visits and individual removal, scene initialization failure/Retry, explicit light/dark card captures, visible keyboard context, and a dark RTL phone fallback. These require the coordinator's fresh execution and screenshot inspection; they were not present in the eight-case baseline run.

## Render bounds follow-up

The widget's own requests fetch at most 1,000 live + 60 archived rows. The overlay union also includes workspace stores: `workspace.svelte.ts` requests shown-list/other-workspace sessions without a limit, and `otto-sessions/src/http.rs::ListSessionsQuery::into_filter` preserves an omitted limit. Thus those own-request limits do not establish an overall DOM bound. Coordinator approved 100-row classroom pagination, keeping all model/action membership. The 106-row real DOM regression checks Next/Previous, last-page menu access and clamp after refresh removes rows; Baseline pagination assertion failed with 106 rows instead of 100 (`/tmp/otto-quality-20261007-school-fallback.log`). The component now renders 100 rows per classroom with labeled Previous/Next buttons and clamps its effective page to the current data. All model membership remains available. Focused green is queued.


## Browser execution and visual inspection

- `/tmp/otto-quality-20261007-school-browser-green.log`: Enter, Space and visible-companion flows passed (3 executed passes). Coordinator interrupted the following delayed-assets test to bound CPU; seven remaining cases did not run. This is not an 11-case green result.
- The interrupted trace shows the scene and saved classroom successfully loaded, but the test's raw `requested` promise never resolved: its intercepted request did not reach `page.route`. The app service worker bypassed routing. Harness now blocks service workers and bounds the interception wait at 15 seconds with guaranteed gate release. The new functional describe uses reduced motion and 1100×800; the original animated journeys retain their settings.
- `/tmp/otto-quality-20261007-school-fallback.log`: initial loading/error/Retry, light desktop overflow, and dark RTL phone overflow passed (3); pagination failed as expected before pagination implementation (1).
- Visually inspected the real `school-card-light.png`, `school-card-dark.png`, and `school-keyboard-focus.png` in the first output directory: card text/buttons remain readable and inside the stage, Check on has a clear focus indicator in both schemes, and the keyboard companion reveals a usable lower pane with a visible row/menu focus ring. Inspected `school-list-1440.png` and `school-list-390.png` in `/tmp/otto-quality-20261007-school-fallback`: complete overflow rows and RTL phone context are readable without horizontal overflow. The outer focus outline touched the scroll container edge, so the list now reserves 4 px of inline padding; that final polish awaits recapture.
- Held camera keys are now tracked and all released on stage blur; a new real keyboard/camera assertion moves with D, focuses List, then verifies the camera remains stationary through twelve browser animation frames.

### Software-renderer test configuration

The interrupted software renderer consumed approximately ten CPU cores, observed by the coordinator. Installed Chromium 1243's `libvk_swiftshader.dylib` contains the `SwiftShader.ini` and `ThreadCount` configuration keys. [SwiftShader runtime configuration](https://github.com/google/swiftshader/blob/master/docs/RuntimeConfiguration.md) documents a working-directory INI with `[Processor]` / `ThreadCount=2`; a coordinator-owned temporary file can constrain the next functional run. This is a test configuration, not a production or performance optimization. Reduced motion alone does not cap this renderer's CPU: the School frame loop still draws open rooms at its existing 30 FPS bound. Actual CPU bound, final green, remaining scene lifecycle assertions and final type gates still need verification.


## Round 2 — refresh failure with existing content

The design reviewer found that a refresh failure was only displayed in the no-data branch. Added `failed School refresh retains the loaded list and exposes Retry until recovery` in `desktop-home-classrooms-regressions.spec.ts`: initial success → explicit Refresh with HTTP 503 → retained student and compact stale banner → Retry returns a changed title and removes the banner. Coordinator ran the test against the unchanged component; it failed at the missing `load-stale` assertion (`/tmp/otto-quality-20261007-school-refresh-red.log`, exit 1).

The loaded branch now adds the shared `LoadState` with `empty=false`, the actual error and Retry callback. The scene/list stay mounted as sibling content. This is the existing shared stale-data presentation; initial failures retain their no-data error state. Focused green remains coordinator-owned and queued. Scoped whitespace check passed after the repair.
