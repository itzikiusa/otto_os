# Round 2 — independent design and UX review

**Final bounded assessment: Design 9.8/10; UX 9.8/10.** Confirmed findings R2-DUX-01–06 have focused closure evidence. The final addition fixes real generated-report contents navigation and verifies populated report hierarchy/data alternatives plus native accessibility names/values at 100/200% zoom. The earlier unexplained native foreground-loss observation remains documented; VoiceOver and packaged-release behavior are not certified. Earlier scores/statuses below are historical checkpoints. See the final populated-report/native-accessibility closure for the current rubric and limits.

Reviewed 2026-10-07, baseline `196048df` plus the current uncommitted repair diff. This is a bounded re-review of the changed School, application telemetry, API environment and Team Performance surfaces and their neighboring interaction paths. It is not an app-wide certification. The initial pass was read-only; later focused regressions and repairs were authored under coordinator-assigned ownership. Browser/build/test execution remained coordinator-owned.

**Design: 8.8/10. UX: 8.4/10 at this checkpoint.** Original design findings D1–D5 are repaired with source and targeted execution evidence. One new stale-data presentation defect remains; the previously discovered plugin post-save draft-loss race is still present in the source read for this checkpoint and the coordinator is repairing it. Missing plugin visual evidence and broad/native coverage also prevent an honest 9.8+ claim. Scores below must not silently change when fixes land: append the new evidence and reassess.

## Remaining findings

### R2-DUX-01 — Minor / P2: School hides a failed refresh after a successful load

**Location:** `ui/src/modules/home/boxes/ClassroomsBox.svelte:80`, `:120`, `:630`.

The loader retains `fetched` and sets `error` when a subsequent request fails. Markup renders that error only when both `fetched` and `ws.sessions` are empty. With existing content, execution enters the scene/list branch, which has no refresh-error feedback. Model staleness depends solely on the event connection, so connected WebSocket + failed cross-workspace HTTP refresh still looks current.

**User consequence:** removed/archived cross-workspace sessions can remain visible without an explanation, last-good-load marker or direct Retry. This is separate from D4: the initial load now has the correct state, but refresh does not. The failure is source-traced; no new browser reproduction was run by this reviewer.

**Fix:** retain the scene/list and add the shared compact stale-data warning with Retry when `error` exists alongside content. Do not replace successful content with an initial-load error or imply the WebSocket status establishes HTTP freshness.

**Acceptance:** load nonempty cross-workspace data, fail the next HTTP read with WebSocket still connected, verify visible stale feedback and existing rows, then Retry successfully and verify the warning disappears. Exercise List and 3D. Consider the independently swallowed archived-list request at line 72 when defining partial-refresh honesty; this review does not claim to have fully audited that separate recovery path.

### R2-DUX-02 — Major / P1, known coordinator-owned: plugin can discard edits during post-save people refresh

**Location at checkpoint:** `examples/plugins/team-performance/ui/views/settings.js:302`–`:311`; capture ownership guard at `:152`.

After `/config` succeeds, the handler calls `capture()` and deletes the draft if its revision matches the submitted one. It then awaits `app.refreshPeople()` before refreshing the form. During that await, controls remain editable but their capture function refuses updates because the draft has already been removed from the account map. The later view refresh therefore reconstructs server values and drops the newer edit.

This is a narrower surviving branch of UX-02, not a duplicate new finding. The coordinator already authored a regression and owns its repair. **Do not close UX-02 solely because the earlier delayed-Save test passes:** that test delayed the write, not the subsequent people read.

**Fix/acceptance:** retain draft ownership until every post-save awaited refresh is reconciled, acknowledge only the submitted revision, and preserve inputs typed while a delayed people response is pending. Verify a remount/navigation during that interval and failed refresh as well as same-form editing. New source/evidence after this checkpoint may supersede this open status.

## Original finding disposition

| Finding | Re-review result | Evidence / limits |
|---|---|---|
| D1 — card Enter intercepted | Fixed for tested flows | Stage handler ignores descendants/defaultPrevented; keyboard regression passes. Light/dark card captures show visible focus on native action. |
| D2 — invisible School focus | Fixed for tested flows | `.list.sr-only:focus-within` reveals companion; actual focused-row/menu image shows visible content and ring. No VoiceOver claim. |
| D3 — nested plugin Escape closes both | Fixed for tested flows | Explicit modal stack, inert covered layers, top-layer key ownership and connected-trigger restoration; focused plugin overlay and popover tests pass. |
| D4 — initial School states | Fixed | `empty` passed in initial/error/scene wrappers; delayed-load and scene Retry browser cases pass. R2-DUX-01 covers the newly inspected refresh branch. |
| D5 — empty/failed trace | Fixed | Empty explanation is `emptyView`; retained trace id supplies Retry; empty/retry/close-pending cases pass. Both theme captures inspected. |
| UX-01 — API environment lost on navigation | Fixed for tested transitions | `approveLeave`, router guard, create/selection guards, awaited Save ownership. Four environment browser journeys pass, including previous pending-save/secret-rename cases. |
| UX-02 — plugin settings ownership | Improved but not closed | Account-owned memory drafts and capture-before-rerender address prior paths. Post-save people await still has R2-DUX-02 at this checkpoint. |
| UX-03 — incomplete School fallback | Fixed for tested population | Complete `members` collection and 100-row pages keep scene geometry capped. Overflow fixture and 106-member paging/refresh-clamp pass. |
| UX-04 — remembered room lost | Fixed for tested transitions | Capture prior room before mount; persist scene callbacks. Delayed-asset restore and List/3D transitions pass. |
| UX-05 — plugin stale scan status | Fixed in source and targeted test | Failed reads label unavailable/reconnecting and expose Retry while retaining the known stage. |
| UX-06 — no Stop scan | Fixed within documented boundary | Reachable Stop, stopping/stopped copy and retained-results explanation; targeted cancellation/restart tests pass. Already accepted remote runs may finish because no remote cancellation handle exists; do not imply those were forcibly stopped. |

## Actual visual inspection

Seven coordinator-produced PNGs were opened using the image tool, rather than inferred from test names:

| Capture | Observed result |
|---|---|
| School card, light and dark (`/tmp/otto-quality-20261007-school-bounded/.../school-card-light.png`, `school-card-dark.png`) | Card is opaque and readable over the scene; one primary Open session, secondary camera actions, separate destructive row. Check on focus ring is visible in both schemes. Scene art remains bright in dark mode; that is not itself a contrast defect in the opaque controls. |
| School keyboard companion (`school-keyboard-focus.png`, same run) | Companion is a visible region beneath the resized scene; focused More button and associated row are visible. Selected card remains available above it. |
| School List at 1440 and 390 (`school-list-1440.png`, `school-list-390.png`, same run) | Long list stays inside the widget. Overflow member and its focused More action are visible. Phone capture is dark RTL, with mirrored row controls and no visible horizontal clipping. This is one captured scroll position, not evidence for every row/theme combination. |
| Trace empty, light/dark (`evidence/trace-empty-light.png`, `evidence/trace-empty-dark.png`) | Dialog clearly states expiration/local-storage delay; adequate visible text space, opaque sheet and unobscured Close action. No numerical contrast measurement performed. |

The School artifact subdirectories are identified by the test names in `/tmp/otto-quality-20261007-school-bounded.log`. These are temporary evidence paths; retain desired captures in the durable evidence bundle before publishing the review.

No current Team Performance light/dark screenshot was found in the quality evidence folder, plugin test folder or matching current `/tmp` artifacts. Its browser source currently has no screenshot calls. Prior unrelated plugin-settings screenshots are not evidence for this plugin revision. Thus the plugin's responsive browser assertions receive functional credit, but a complete visual review of the report, settings, scan status and overlays is **still open**.

## Verification evidence read

| Log | Result read | What it supports |
|---|---|---|
| `/tmp/otto-quality-20261007-school-bounded.log` | 11 passed, 1 failed | Eleven functional School cases passed. The remaining renderer-resource case failed with “Scene Three module was not loaded,” an instrumentation failure; not a UI visual finding and not a passing resource test. |
| `/tmp/otto-quality-20261007-ui-repair-browser-green.log` | 7 passed | Four environment and three trace journeys. |
| `/tmp/otto-quality-20261007-plugin-final-focused.log` | 15 passed | Targeted plugin overlay/draft/status/cancellation/cache cases; not fifteen complete visual journeys. |
| `/tmp/otto-quality-20261007-plugin-full-green.log` | 466 passed, 2 failed, 2 skipped | Filename is not the outcome. Full suite did not pass at this execution. |
| `/tmp/otto-quality-20261007-plugin-full-fixes.log` | 2 passed | Both previously failing targets passed after repairs. This is targeted recovery evidence, not a fresh whole-suite pass. |

## Separate scores

Each rubric uses five /2 dimensions, on the inspected scope. The uplift from round 1 is tied to repaired interactions and fresh browser/image evidence, not the requested target.

| Design dimension | /2 | Reason |
|---|---:|---|
| Consistency and primitive integration | 1.85 | Shared load/trace contracts and native card controls repaired. |
| State presentation and recovery | 1.65 | Initial/trace recovery verified; loaded School refresh still silently stale. |
| Focus and keyboard presentation | 1.80 | Real focus images and targeted overlay/keyboard passes; native assistive-technology coverage absent. |
| Responsive and visual foundations | 1.80 | School desktop/phone RTL and trace light/dark inspected; plugin and wider theme matrix missing. |
| Hierarchy and action clarity | 1.70 | Clear primary/card/dialog actions; broader plugin rendered hierarchy and populated telemetry tables unreviewed. |
| **Design total** | **8.80/10** | Bounded evidence-based assessment. |

| UX dimension | /2 | Reason |
|---|---:|---|
| Task completion and control | 1.85 | School actions and plugin Stop/restart are available with targeted tests. |
| Recovery and honest status | 1.65 | Scan and trace recovery improved; School stale refresh remains. |
| Preservation of user work | 1.35 | API leave guards verified; plugin post-save draft-loss branch still open. |
| Navigation and continuity | 1.80 | School restoration, complete membership and paging verified; broad app routes not rerun. |
| Accessible alternatives and predictability | 1.75 | Visible companion and native card action parity verified; VoiceOver/native WebKit remains unchecked. |
| **UX total** | **8.40/10** | Do not round up to the target or call app-wide. |

## Next evidence required

1. Close R2-DUX-01 and R2-DUX-02 with the specific success→failure/late-edit regression branches, then independently re-read those repairs.
2. Capture and inspect Team Performance populated Overview, Settings, report viewer + nested Download, and scan stale/stopping states at desktop light/dark and phone. Add RTL/keyboard assertions around the changed controls.
3. Finish the resource instrumentation repair separately, and run the final affected gates; do not classify instrumentation failures as a clean resource result.
4. Before claiming app-wide 9.8+, complete the previously listed representative automation/data/content journeys, Warm/custom-accent contrast, zoom/reduced-motion and native WKWebView/VoiceOver spot checks. This review does not certify those unsampled areas.

## Focused closure addendum — 2026-10-07

This addendum supersedes the earlier open status of R2-DUX-01 and R2-DUX-02. It follows a focused source/visual read; this reviewer ran no tests or builds.

### Functional closure

- **R2-DUX-01 closed:** `ClassroomsBox.svelte:633` now renders the shared stale-data warning with Retry above the retained scene/list. `/tmp/otto-quality-20261007-school-final.log` shows the success→failed refresh→Retry regression passing. The same log's rendered ten-cycle skeleton cleanup/removal case now passes, resolving the earlier instrumentation failure for that case; it does not establish a long-duration GPU/RSS leak rate.
- **R2-DUX-02 closed:** `settings.js:39` retires a saved baseline only when rendering the next form, after outgoing capture. The save handler records `acknowledgedRevision` instead of deleting live draft ownership before awaited people/overview refreshes. New edits still increment the revision, so the render cannot clear them. The final plugin suite includes separate successful regressions for edits during post-save people **and** overview requests.
- **D1 stronger action evidence:** `/tmp/otto-quality-20261007-school-actions-verified.log` shows both native Enter and Space journeys passing, including actual headmaster target assertions. This strengthens the previous callback-level closure.
- **Fresh full plugin outcome:** `/tmp/otto-quality-20261007-plugin-full-final.log`: **470 passed, 0 failed, 2 skipped**, 472 total. This supersedes the earlier mixed full-suite result and targeted-only recovery. Skips remain skips.

### Eight plugin images inspected

Opened `/tmp/otto-quality-20261007-plugin-visuals/{overview,settings}-{390,1280}-{light,dark}.png` with the image tool — all eight exact combinations.

Settings is legible in both schemes; phone fields stack into a usable column, and the horizontal tab strip keeps the selected Settings tab visible. Desktop fields and the people table remain inside their surface. Overview and Settings captures show no visible page-level horizontal clipping. The populated fixture also exposes the weak-data state below. These captures are at the current scroll position, so they do not prove every lower form field or table interaction is visible. Report viewer, nested Download and transient stale/stopping visual captures remain outside this image set, although related behavior has targeted browser coverage.

### R2-DUX-03 — Minor / P2: repeated full guardrail explanations dominate weak-data metric cards

**Evidence:** `examples/plugins/team-performance/ui/components.js:750` inserts the complete guard reason into every metric tile. `ui/app.css:1186` puts diagonal warning hatching over the entire weak tile and `:1193` dims its value. Current light/dark Overview captures at both widths show the resulting state.

In the 1280 px fixture, four DORA tiles become roughly 400 px tall, largely from repeated explanations, including the same stale repository warning. The separate Input checks section then repeats those warnings. On the phone, the first viewport contains the global controls and only about one and a half of the four metric cards. The visual weight falls on repeated warning prose and hatching instead of a compact comparison of metrics and their confidence. This is a concrete scanability problem in the captured weak-input state, not a claim that the warnings should be removed or that a measured contrast threshold failed.

**Fix direction:** retain an always-visible weak-input label and concise cause on each affected value; expose full metric-specific reasons through an accessible disclosure; consolidate shared repository/source remediation in the existing Input checks section. Reduce the full-card pattern to a small status marker or quiet edge, keeping readable data on an opaque surface. Preserve every warning and its association with the affected metric.

**Acceptance:** seed the same low-sample/stale-repository fixture and inspect light/dark at 390/1280. Metric values, labels and confidence remain easy to compare before expanding reasons; keyboard users can reach and expand all caveats; no warning disappears or becomes tooltip-only. Do not obtain compact cards by clipping warning text or shrinking it below the type floor.

### Updated bounded scores

| Design dimension | /2 | Closure rationale |
|---|---:|---|
| Consistency and primitive integration | 1.90 | Original shared-state and control integrations repaired. |
| State presentation and recovery | 1.95 | Initial, empty, failed and stale School/trace paths now verified. |
| Focus and keyboard presentation | 1.90 | Actual focus captures and native action/overlay tests; native AT remains unverified. |
| Responsive and visual foundations | 1.85 | Eight plugin captures add desktop/phone light/dark evidence; larger theme and native matrix still open. |
| Hierarchy and action clarity | 1.60 | Captured weak-data metric cards reveal R2-DUX-03; this dimension is reduced on fresh visual evidence. |
| **Design** | **9.20/10** | Not app-wide and not 9.8. |

| UX dimension | /2 | Closure rationale |
|---|---:|---|
| Task completion and control | 1.90 | Native School actions and plugin scan controls verified. |
| Recovery and honest status | 1.90 | School refresh now reports staleness and recovers. |
| Preservation of user work | 1.95 | Both post-save awaits retain newer edits; API draft guards remain verified. |
| Navigation and continuity | 1.85 | Restoration, complete membership, paging and scoped draft continuity covered. |
| Accessible alternatives and predictability | 1.70 | Strong functional fallback; degraded dashboard scanability and native AT gap remain. |
| **UX** | **9.30/10** | Bounded score; remaining coverage is not treated as passing. |

**Current open work:** R2-DUX-03 visual hierarchy, final coordinator integration gates, and the explicitly uninspected wider/native/theme matrix. The earlier outstanding functional findings and missing Overview/Settings captures are no longer open.

## Final visual closure — 2026-10-07

**R2-DUX-03 closed.** The shared tile now retains its visible weak-input badge and puts complete guard reasons in a closed native `<details>` with the summary **Input limitations** (`ui/components.js:750`). The CSS removes full-tile hatching and value opacity from weak metric tiles, retaining a quiet warning border (`ui/app.css:1186`). The full reason string still reaches the disclosure; source review confirms the previous `guardReason` only rendered bad-level guards too, so this change does not drop previously displayed warning-level paragraphs.

All eight final images were opened: `/tmp/otto-quality-20261007-plugin-visuals-final/{overview,settings}-{390,1280}-{light,dark}.png`. The revised desktop Overview shows four compact, readable metric cards, with complete Input checks below. Phone Overview now shows three complete cards before the next starts, compared with approximately one and a half previously. Weak-input status remains visible beside each value; the focused disclosure summary has a visible ring in both themes. Settings retains its readable stacked phone fields and contained desktop layout in these captures. No new clipping or visual defect was observed in this focused comparison.

`/tmp/otto-quality-20261007-plugin-visual-final.log` records **4 passed, 0 failed, 0 skipped**: 390/1280 × light/dark. The inspected browser assertions verify the reason is initially hidden, the collapsed tile remains compact, Enter reveals the complete reason, and the overview/settings layout has no page-level horizontal overflow. These are targeted post-repair passes; the earlier full-suite result predates this final visual patch and must not be relabeled a new full-suite run.

Updated scoring changes only the dimensions supported by this closure:

| Design dimension | /2 |
|---|---:|
| Consistency and primitive integration | 1.90 |
| State presentation and recovery | 1.95 |
| Focus and keyboard presentation | 1.90 |
| Responsive and visual foundations | 1.85 |
| Hierarchy and action clarity | 1.90 |
| **Design total** | **9.50/10** |

| UX dimension | /2 |
|---|---:|
| Task completion and control | 1.90 |
| Recovery and honest status | 1.90 |
| Preservation of user work | 1.95 |
| Navigation and continuity | 1.85 |
| Accessible alternatives and predictability | 1.90 |
| **UX total** | **9.50/10** |

The repaired hierarchy earns the design increase; native keyboard-accessible warning disclosure earns the UX increase. These are **bounded qualitative scores**, not percentages of an exhaustive checklist. No open confirmed finding from this report remains. The limits still stand: native WKWebView/VoiceOver, the full custom-theme/contrast matrix, report/transient-overlay image coverage, and unsampled app-wide journeys were not newly verified here. Their absence does not create invented defects, but it prevents an honest app-wide 9.8+ certification. Final integration gates remain coordinator-owned.

## Representative evidence closure — 2026-10-07, 12:42–12:43 local

The coordinator executed the finite selection in [remaining-browser-selection.md](remaining-browser-selection.md). This reviewer independently read both actual logs and opened all eleven fresh images. **14 Chromium tests passed (26.7 s); one iPhone WebKit RTL/Home test passed (6.6 s).** Neither selected invocation reported a failure or skip. No test/build/browser process was launched by this reviewer.

The eleven images and two logs are retained in [evidence/representative/](evidence/representative/), with original source paths and SHA-256 digests in [manifest.json](evidence/representative/manifest.json). This avoids relying on temporary screenshot paths remaining available.

### What the new evidence establishes

- **Automation and continuity:** the selected browser journeys exercise scheduled-task retry and pending-save edits; goal Keep/Discard across workspaces; workflow preflight and preservation of a disabled cron trigger. These are fixture-backed interactions, not real scheduled provider executions.
- **Data and content:** database executor/review gating, Kafka input validation/native keyboard row selection/failed Produce draft retention, Redis review-and-Cancel without a write, Vault trash/version recovery and property/graph navigation all passed. The Vault property test does not independently compare the note body despite its broad title, so no new body-preservation claim is made.
- **Warm/custom accents:** eight setup/shared-sheet/focus images show readable opaque surfaces in Warm light and dark, including RTL phone/tablet layouts, yellow/near-black custom accents, and wallpaper enabled/disabled. Actions remain contained in the sheets; the focused input boundary remains visible. The executed tests independently measure text contrast ≥4.5, text size ≥11px and focused-input boundary contrast ≥3 in their scoped production components. These are scoped measurements, not a claim that every app text element was measured.
- **Reduced motion:** both shared-sheet theme cases passed the actual animation-duration assertion (≤0.001 s). This closes the previously unexecuted representative reduced-motion check; it does not prove every animation respects the preference.
- **Visual data hierarchy:** the Kafka capture shows a selected message and a visibly focused native offset control alongside its detail. The Redis capture presents the before/after value, editable native statement, explanatory copy and separate Cancel/Run controls within an opaque modal. These two captures are desktop/light only.
- **Phone content layout:** the 375px Warm/dark Design Hall capture contains the preview, source/device controls and version strip without visible page-level clipping; fixture HTML remains on its own white preview surface. The test verifies details navigation, save/version creation, notch clearance and horizontal overflow.
- **WebKit scope:** the existing iPhone project passed RTL direction, horizontal overflow and shared Axe checks for Home. The helper rejects all critical violations and new serious violations against its page baseline. This is Playwright WebKit evidence, not packaged Tauri or VoiceOver evidence.

### Evidence caveat, not a confirmed product finding

The Design Hall screenshot displays version 2 while the iframe still reads the initial “Release checklist”; the test edited its source to “R4 saved responsive draft.” `ArtifactStage.svelte:113`–`:123` intentionally debounces typed preview updates by 350 ms. The selected test (`desktop-ux-r4-content.spec.ts:92`–`:99`) waits for two version chips, then captures without waiting for the iframe heading. The available evidence therefore establishes saved-version creation and layout, but not settled saved-preview content. Waiting for that heading before the final capture would close the evidence gap; this screenshot alone does not demonstrate a persistent stale-preview defect.

### Revised bounded scores

The small increases below reflect broader observed evidence, not another product change or a pass-count formula. No additional confirmed design/UX defect was found in these images.

| Design dimension | /2 | New evidence and remaining scope |
|---|---:|---|
| Consistency and primitive integration | 1.90 | Shared sheets remain coherent across the additional themes; no new implementation change. |
| State presentation and recovery | 1.95 | Prior repaired-state evidence retained; additional retry journeys passed. |
| Focus and keyboard presentation | 1.95 | Visible custom-accent focus plus measured focus contrast and actual Kafka keyboard selection extend prior evidence. |
| Responsive and visual foundations | 1.90 | Warm phone/tablet, extreme accent, wallpaper and motion cases now observed; native zoom and full theme matrix remain unsampled. |
| Hierarchy and action clarity | 1.90 | Clear data review controls and compact repaired plugin metrics; no broad new hierarchy survey. |
| **Design total** | **9.60/10** | Bounded sampled assessment. |

| UX dimension | /2 | New evidence and remaining scope |
|---|---:|---|
| Task completion and control | 1.95 | Representative automation/data/content completion and cancellation paths now executed. |
| Recovery and honest status | 1.90 | Prior recovery closure plus Kafka retry/Vault restoration; not an exhaustive recovery sweep. |
| Preservation of user work | 1.95 | Prior API/plugin ownership evidence reinforced by scheduled/goal draft transitions. |
| Navigation and continuity | 1.90 | Workspace Keep/Discard, Vault graph/history and phone Design Hall details navigation broaden continuity evidence. |
| Accessible alternatives and predictability | 1.90 | Additional measured contrast/motion and mobile WebKit evidence; native assistive-technology behavior remains unobserved. |
| **UX total** | **9.60/10** | Bounded sampled assessment. |

**Current disposition:** all confirmed findings from this report are closed. The previously proposed representative theme/motion and automation/data/content journeys have now run. Packaged WKWebView/native zoom, VoiceOver, report/transient-overlay images and other unsampled flows remain verification limits; they are not invented defects or a universal score ceiling. The browser shell test proves CSS zoom is not incorrectly applied in a browser; it does not prove native zoom works. This evidence supports the revised bounded scores, not an app-wide 9.8+ certification. Integration/telemetry gate outcomes remain coordinator-owned and are not inferred from these UI passes.

## Saved-preview and telemetry closure — 2026-10-07, 12:49 local

Independently read `/tmp/otto-quality-20261007-telemetry-export-window.log`: **7 passed, no failures or skips (2.8 minutes)**. This comprises all six telemetry cases plus the strengthened Warm/dark Design Hall case. Opened eight unique fresh PNGs: settled Design Hall preview; populated telemetry light/dark; telemetry settings light/dark/phone; trace empty light/dark. Copies and the run log are retained in [evidence/telemetry-preview-closure/](evidence/telemetry-preview-closure/), with provenance/digests in its [manifest](evidence/telemetry-preview-closure/manifest.json).

**Design Hall evidence caveat closed.** The existing responsive test now waits for the actual iframe heading before its final capture; every prior assertion remains. The fresh phone image visibly shows “R4 saved responsive draft” and v2. The earlier screenshot caught the intentional preview debounce; this closure required a stronger test wait, not a product repair. The earlier image is retained as historical evidence, not the current saved preview.

**Telemetry populated-state review:** desktop light/dark images show distinct local-collection status, accepted/queued/discarded counts, a Refresh action and four contained resource cards. CPU/RAM plots have readable units and non-overlapping time ticks. The host card labels unavailable CPU/memory explicitly instead of drawing invented zero values. Native “Show data” disclosures are visible beneath charts. The separate settings captures show opt-in copy and named controls, with a single stacked phone column; the phone image covers the upper form rather than its entire scroll range. Fresh trace-empty captures continue to show the explanatory message inside an opaque modal with a reachable close control. No new visual defect was observed in these captures.

The passing full-chain case independently verifies real browser batches are accepted, parent-linked navigation/render/client/server spans reach local trace storage, populated charts retain the expected scale/non-overlapping ticks, and disabling collection stops collector readiness. Source and log support those exact claims; images alone would not. The active captures emphasize resource cards at the current scroll position. They do not newly establish the visual behavior of every lower measured-operation row, long trace, or profiler output.

**Final bounded scores remain Design 9.6/10 and UX 9.6/10**, with the five subdimensions in the preceding table unchanged. This closes a specific evidence caveat and strengthens confidence in the existing state/visual assessment; another successful run does not by itself warrant another score increase. All confirmed findings in this report are closed. Native packaged WKWebView/zoom and VoiceOver, report/transient-overlay visuals and unsampled app-wide behavior remain explicitly unverified. No missing verification is being reported as a proven defect, and no app-wide 9.8+ claim is made.

## Native and transient follow-up — focus repairs and verification checkpoint

This follow-up extends the earlier bounded scope through the isolated harness described in [native-and-transient-acceptance.md](native-and-transient-acceptance.md). Its native probe runs the actual bundled SPA in nonpersistent Tauri/WKWebView stores against a throwaway daemon; it does not launch or modify the installed Otto app. Browser/source work discovered two additional focus defects, so the preceding “all findings closed” wording is historical until the final closure below.

### R2-DUX-04 — Minor / P2: loading report viewer did not acquire keyboard focus

**Location:** `examples/plugins/team-performance/ui/components.js:341`; trigger in `ui/views/reports.js:81`.

The report begins with a disabled Download button. The shared modal's initial selector chose it, and calling `.focus()` on it left focus outside the new dialog. The dedicated regression holds the real report HTML response and asserts focus ownership before any test-side focus operation: `/tmp/otto-quality-20261007-report-focus-red.log` failed with `owned:false`. The minimal repair selects an enabled, visible, non-inert control, then falls back to the dialog. The focused regression and all four real report/scan journeys passed in `/tmp/otto-quality-20261007-plugin-focus-green.log`: **5 passed, 0 failed/skipped**. This finding is closed for the tested flows; broader plugin regression gates remain coordinator-owned.

### R2-DUX-05 — Minor / P2: shared modal loses input focus when the desktop bar unmounts

**Location:** `ui/src/lib/components/Modal.svelte:98`; synchronous observer in `ui/src/lib/components/FloatingBar.svelte:572`/`:603`; responsive unmount in `ui/src/shell/App.svelte:1248`.

The real native probe retained the connected form field, draft and dialog bounds through zoom, but input focus moved to the old Pane window trigger at CSS width 1000. Native diagnostics recorded both `.focus()` call stacks inside the Modal bundle, and the failure persisted after two seconds without refocusing. The same failure reproduced in Chromium on a normal 1500→1000 viewport resize (`desktop-modal-focus-resize.spec.ts`; ten-second inactive-field failure).

The modal effect untracked only the focusable-element lookup. Its subsequent `.focus()` synchronously dispatched `focusin` to the desktop FloatingBar listener, which read its reactive `rootEl`. Removing that bar on a breakpoint change therefore reran the modal effect: autofocus executed again, then its previous queued cleanup restored the old trigger. This is not a speculative platform focus issue.

The repair wraps the complete intended mount-only focus lifecycle in `untrack`, preserving its returned close-time cleanup. The new browser regression explicitly observes the desktop bar mounting/unmounting and allows two frames to settle; it never refocuses after resize. The native test likewise does not restore focus to manufacture a pass. **Repair verification pending at this checkpoint.**

### Twenty report and transient images inspected

All images in `/tmp/otto-quality-20261007-plugin-transients-final/` were opened: report, nested Download, stale status, stopping and stopped, each at 390/1280 and light/dark. They are retained with the failure/success logs and SHA-256 provenance in [evidence/native-transient/](evidence/native-transient/).

The report shows a clear title/masking badge, visible Close focus, a contained readable document and comments beside it on desktop/below it on phone. The nested Download confirmation states the local destination and forwarding audience, retains a visible Cancel focus ring, and keeps the unsent comment visible behind the dimmed parent. The fixture report's white document background in dark mode belongs to the supplied HTML, while plugin chrome follows dark tokens; this is not itself a theme defect. Scan status remains separate from retained metrics, exposes Retry alongside the stale progress, disables Stop while stopping, and explains that completed results remain after stopping. No new clipping or visual hierarchy defect was observed. These images close the previously missing representative report/transient visual set; they do not audit every possible generated HTML report.

The prior 9.6 scores are the last completed checkpoint, not a claim that the newly observed shared-modal defect has already passed verification. Final bounded scoring follows after the focused browser and native repairs are verified.

## Final native/transient closure — 2026-10-07, 14:00 local

**R2-DUX-05 closed for the reproduced defect.** The strengthened Chromium breakpoint regression passed after the `untrack` repair, and both existing nested-dialog tests passed with the original trigger removed or inert. They establish that close-time focus fallback still works. The actual bundled-SPA native probe then passed with all strict assertions unchanged: native menu zoom **100→200→190→100%**, focused connected `nw-name` throughout, unchanged unsaved text, contained dialog/field, zero horizontal page overflow, retained host/child DOM identity and real detach/return behavior. The native snapshots record CSS widths 1500→750→789→1500, `documentFocused:true`, and empty focus-call/event diagnostic arrays at all four checkpoints. No corrective refocus occurs after zoom.

### Independently read final evidence

| Evidence log, retained in `evidence/native-transient/` | Actual result | Scope |
|---|---|---|
| `otto-quality-20261007-modal-resize-green.log` | 1 passed | Real production Modal/FloatingBar breakpoint reproduction, after its recorded RED |
| `otto-quality-20261007-modal-nested-final.log` | 2 passed | Nested parent focus fallback for removed/inert triggers |
| `otto-quality-20261007-native-foreground.log` | Final PASS; coordinator reported exit 0 | Freshly built current-source Tauri/WKWebView full-SPA probe with strict native zoom/focus/draft/bounds/pane assertions |
| `otto-quality-20261007-plugin-native-final.log` | 487 passed, 0 failed, 1 skipped | Full plugin suite after the initial-focus repair; skipped light-theme amber assertion is explicitly marked in the dark suite |
| `otto-quality-20261007-native-clippy-final.log` | Finished successfully; coordinator reported exit 0 | Strict native example Clippy gate |

The UI check (zero errors/warnings), 1698 unit passes and production build were also reported green by the coordinator; those broader logs are owned by the master validation record. This reviewer does not equate the native example's gate with all desktop production targets passing.

**Ambiguous native observation retained:** the immediately preceding fresh run (`otto-quality-20261007-native-spa-final.log`) preserved DOM input ownership and draft/bounds, but lost whole-application foreground activation during zoom. Its cause is unknown. A read-only activation diagnostic was added, and the next run passed without firing it or relaxing assertions. That successful run establishes the bounded acceptance above; it does not retroactively diagnose the earlier failure as external interference, prove it cannot recur, or hide that failed execution. The original reproducible modal defect is independently explained and covered by browser RED→GREEN and native call-stack evidence.

### Final five-part rubric

| Design dimension | /2 | Evidence supporting the assessment |
|---|---:|---|
| Consistency and primitive integration | 1.95 | Shared loading/dialog primitives now handle reviewed initial, stale, nested and async-content paths; the native-discovered autofocus dependency is repaired centrally. |
| State presentation and recovery | 1.95 | Initial/empty/error/stale School and trace states plus real report/scan transient views were exercised and visually inspected. |
| Focus and keyboard presentation | 1.95 | Actual nested focus, report loading focus, visible browser focus captures, contrast measurements and strict native modal focus retention are now covered. |
| Responsive and visual foundations | 1.95 | Light/dark desktop/phone, representative Warm/extreme accents/RTL/reduced-motion, and actual native 200% dialog bounds have evidence. |
| Hierarchy and action clarity | 1.90 | Repaired weak-metric cards, actual report/Download audience copy and scan status/control hierarchy are clear in the observed fixtures; the generated report itself was represented by short sample HTML rather than its full production-rendered hierarchy. |
| **Design total** | **9.70/10** | Bounded qualitative assessment, not an app-wide certificate. |

| UX dimension | /2 | Evidence supporting the assessment |
|---|---:|---|
| Task completion and control | 1.95 | School/API/plugin plus representative automation/data/content journeys and local-report cancellation complete as tested. |
| Recovery and honest status | 1.95 | Retained stale progress, explicit Retry, disabled stopping action and retained-results copy have both behavior and image evidence. |
| Preservation of user work | 1.95 | API/plugin/scheduled/goal drafts, nested report comment draft, native pane state and actual modal text survive their sampled transitions. |
| Navigation and continuity | 1.95 | School restoration/paging, workspace transitions, content details/history and actual native pane detach/return and zoom continuity are covered. |
| Accessible alternatives and predictability | 1.90 | Keyboard/native alternatives, scoped Axe/contrast and motion checks are strong; no VoiceOver reading/announcement/rotor assessment was performed. |
| **UX total** | **9.70/10** | Reflects the reviewed scope; no forced 9.8 uplift. |

The rise from 9.6 comes from real additional fixes and materially broader observed native/transient behavior. No confirmed design/UX finding remains open in this report. Native zoom is no longer wholly unexecuted: the isolated current-source Tauri probe now has a strict passing run. Remaining limits are VoiceOver/OS accessibility behavior, physical menu-shortcut delivery, packaged-release/install acceptance, physical multi-display transitions, arbitrary generated report content and unsampled app-wide states. Those limits are not invented defects or a universal numeric ceiling; this evidence supports **9.7 for this reviewed scope** and does not establish an honest app-wide 9.8+ result.


### Finite next evidence within this scope (prepared, not yet credited)

The next acceptance is deliberately limited to the reviewed report and scan surfaces. Four new `quality populated report` browser cases load actual `buildReportModel` → `renderReport` HTML into the production sandboxed report viewer at 390/1280 pixels in light/dark. The deterministic fixture includes measured and unavailable phases, weak estimate coverage, three long person names and populated ticket tables. Assertions cover one document title/main, named contents and sections, caveat-before-metrics reading order, sensible heading nesting, a descriptive chart alternative with native table disclosure, real keyboard fragment navigation, reflow with contained wide tables, and comment-draft/focus continuity. Top, phases and people screenshots permit an independent hierarchy assessment beyond the earlier short HTML fixture.

The existing four scan transient cases additionally assert a polite status region, named scan steps and a textual current phase, alongside their already tested stale → retry → stopping → stopped transitions. These assertions establish browser semantics and observable status content, not spoken announcements. A separate reviewer owns a read-only, probe-scoped native accessibility capability check; no global VoiceOver toggle or permission change is authorized or needed for this finite set.

These cases can reduce specific uncertainty in hierarchy/action clarity and accessible alternatives/predictability, and may also strengthen foundations/state presentation. They are not a purchased 0.1 score increase: scores stay 9.7 until execution and visual/native evidence are assessed. Physical VoiceOver output/rotor behavior, hardware transitions and packaged-install acceptance remain explicit boundaries rather than an exhaustive-product prerequisite imposed on this bounded review.


### R2-DUX-06 — Minor / P2: report contents links replace the embedded document

**Location:** `examples/plugins/team-performance/ui/views/reports.js:114` (original assignment at line 114); relative links emitted by `lib/reportmodel.js:1331`.

The finite populated-renderer check found a real navigation defect. On Enter on “Where the time goes”, the `srcdoc` document inherited the plugin page's base URL. The link navigated the frame to `index.html?...#phases`, replacing the report with the plugin page. Four populated cases failed after the jump; the focused diagnostic RED records original URL `about:srcdoc`, a resolved link to the embedding page, then title/heading “Team Performance” and `hasPhase:false` (`/tmp/otto-quality-20261007-report-fragment-red.log`). The report's named contents navigation was therefore unusable, even though scrolling the document worked.

The viewer-only repair inserts `<base href="about:srcdoc">` into the embedded copy as text, without parsing report resources in the parent context, and leaves the original HTML used for download unchanged. The scripts-disabled iframe sandbox remains intact. The regression retains the actual Enter navigation and adds sandbox, absolute-link preservation and downloaded-byte equality checks. Verification is pending at this checkpoint; the prior 9.7 score is a completed earlier assessment, not credit for an unverified fix. All four initial populated top images were independently inspected: title, scope/period, weak-input caveats, missing-data note, attributed summary and comments remain readable and contained in both schemes/sizes.


## Final populated-report and native-accessibility closure

**R2-DUX-06 is closed by the reproduced navigation repair and focused GREEN.** Independently read `otto-quality-20261007-populated-report-green-final.log`: **8 passed, 0 failed/skipped, 15.9 seconds**. The four production-renderer cases now keep the report document on actual Enter navigation to phases and people, expose chart numbers through a native keyboard disclosure with row/column headers, preserve the comment draft across nested Download cancellation, preserve absolute ticket destinations, and download exactly the original report bytes. The scripts-disabled sandbox remains `allow-popups`. The four existing scan cases also pass their new polite-status, named-steps and textual-current-phase assertions.

Two intermediate failures after the navigation repair were incorrect new test selectors, not product defects: chart tables intentionally have a row header as well as column headers, and People's first disclosure is its chart table rather than an individual's tickets. The corrected assertions require those actual semantics and explicitly open the first person's Tickets disclosure. The original fragment-navigation RED remains preserved.

**All twelve final populated images were opened**: top, phases and people at 390/1280 pixels in light/dark. The production report now has observed scope/date and warning hierarchy, an attributed summary, consistent phase colours with descriptive text, unavailable measurements labelled “not tracked”, and named person cards with contextual capacity/coverage explanations. Desktop chart values and their accessible table agree. Phone disclosures retain visible keyboard focus and keep wide tables inside their own scroll regions; their screenshots intentionally show the focused, scrolled location rather than the entire long report. No new clipping or visual hierarchy finding arose from this fixture.

**Actual native accessibility exposure was checked, not inferred from browser ARIA.** Independently read `otto-quality-20261007-native-ax-client.log`: the public `AXUIElement` client, constrained to the isolated probe's own application, found the named Add Workspace dialog, Name field with the exact unsaved fixture value, and enabled Cancel/Create controls at **100% and 200%**. Both snapshots report own-application ancestry, one web area, 17 visited nodes/eight ancestor nodes, no errors, truncation or deadline exhaustion. The same run passes the original strict 100→200→190→100% native focus/draft/bounds/detach assertions. Initial in-process accessibility attempts exposed only remote objects and were not counted as a pass; the public-client route supplied the actual named web-content evidence. No VoiceOver toggle or permission change was made.

Images and the focused RED/GREEN/native logs are retained in [evidence/populated-report-native-ax/](evidence/populated-report-native-ax/) with [SHA-256 provenance](evidence/populated-report-native-ax/manifest.json). Final full-plugin and native-lint gates completed successfully; exact results are recorded below.

### Current five-part rubric

| Design dimension | /2 | Current evidence and change from 9.7 |
|---|---:|---|
| Consistency and primitive integration | 1.95 | Shared primitives and themed production report remain consistent; unchanged. |
| State presentation and recovery | 1.95 | Existing initial/error/stale/retry evidence now also asserts polite scan-status structure; unchanged. |
| Focus and keyboard presentation | 2.00 | Reproduced focus repairs, native zoom retention, named native dialog/input/actions, and visible focused report disclosures satisfy this sampled criterion; +0.05. |
| Responsive and visual foundations | 1.95 | Actual generated report now joins phone/desktop light/dark and native zoom evidence; sampled long tables remain contained; unchanged. |
| Hierarchy and action clarity | 1.95 | Replaces the short-HTML evidence gap with real populated warning, chart, table and people hierarchy; +0.05. |
| **Design total** | **9.80/10** | Bounded qualitative assessment; final broader gates passed. |

| UX dimension | /2 | Current evidence and change from 9.7 |
|---|---:|---|
| Task completion and control | 1.95 | Reviewed workflows plus original-byte local report download complete; unchanged. |
| Recovery and honest status | 1.95 | Stale → retry → stopping → retained-results behavior and truthful missing metrics remain established; unchanged. |
| Preservation of user work | 1.95 | Report comment cancellation/download and native input/AX value retain drafts alongside prior preservation cases; unchanged. |
| Navigation and continuity | 2.00 | New real contents-navigation defect is repaired with actual keyboard transitions, nested focus return and original standalone download preserved; +0.05. |
| Accessible alternatives and predictability | 1.95 | Native named/value exposure and production chart/table/section semantics close concrete uncertainty; +0.05. |
| **UX total** | **9.80/10** | Same reviewed scope; no claim of universal accessibility certification. |

The increase is supported by a further confirmed defect repair and specific additional rendered/native behavior, not repetition of passing tests or expansion to an exhaustive-product requirement. No confirmed finding remains open after focused closure. Remaining limits include VoiceOver spoken announcements/rotor behavior, native AX exclusion of covered background content (explicitly not asserted), physical keyboard-menu delivery, packaged installation and multi-display hardware transitions. The earlier unexplained foreground-loss run remains an unresolved observation despite subsequent strict passing runs; its cause has not been reclassified as external. These limits remain material qualifications to the bounded 9.8 assessment.


**Final broader gate closure:** independently read the completed full-plugin log: **491 passed, 0 failed, 1 known skipped, 492 total (65.98 seconds)** in `otto-quality-20261007-plugin-populated-final.log`. The skip remains the intentionally excluded light-theme amber assertion in the dark suite. Native strict Clippy finished successfully in **3.03 seconds** in `otto-quality-20261007-native-ax-clippy-final.log`; the coordinator confirmed exit 0 for both commands. Both logs are copied into the final evidence folder/manifest. The gate hold is removed. **Final Design 9.8/10 and UX 9.8/10** stand with the explicit scope and remaining limits above. No further source edits or executions were performed by this reviewer after the freeze.
