# Round 4 — fresh shell / Home / Agents / panels / desktop companion review

Worktree only `/Users/itziklavon/otto-ux-audit-20260925`. Read the full review protocol, assignments, parent notes, AGENTS/design guidelines, R1/R2/R3 shell reports. No commits, subagents, original-checkout edits, production mutations or provider-home writes. Systematic debugging / test-first regressions / fresh verification applied. This report covers one reviewer, not whole-app completion.

## Confirmed defects and repairs

1. **P1: crossing desktop/tablet destroyed the module component, losing local draft/selection/focus.** `ui/src/shell/App.svelte:1087`. Independently reran the automation author's unchanged Swarm Feed test: red at focused compact picker. The desktop and mobile branches invoked `centerContent()` at separate mount sites. One stable shell/content hierarchy now changes only its surrounding chrome. Original Swarm1440→834RTL→1440 test passes Chromium and WebKit; further real API URL draft/focus and Files preview/section persistence cover1440→834→390→1440. Popout/embedded chrome and existing side-pane behavior retain their separate rules.
2. **P1: right-panel presentation discarded local state and focus across breakpoints.** `ui/src/shell/Drawer.svelte:45`, `App.svelte:1200`. Added inline presentation to the same mounted Drawer host; inline removes dialog role/modal focus behavior, while overlay keeps the existing focus trap/return. A pre-update focus snapshot restores the same descendant when changing `display:contents` into the overlay box blurs it. `SessionView.svelte:184` stops treating a viewport change as an instruction to refocus its terminal. Held Notes PATCH regression verifies exact text,6–12 selection range and focus through desktop→tablet→phone→desktop; the returned desktop has no Activity dialog role. Existing drawer/palette and side-by-side tests rerun.
3. **P2: terminal Zoom in had no visible effect in a narrow desktop split.** `Terminal.svelte:704` automatic fitting reapplied the width cap to the requested font itself; red computed-font test stayed11 after Zoom in. Automatic sizing now computes the default13px fit and applies the user's zoom offset afterward, preserving >=11px text and >=80desktop PTYcolumns with internal horizontal access. Split11→12→11, Reset→11 and expanded default13 checks pass. `SessionView.svelte:872` and Terminal's standalone toolbar expose a named Reset terminal zoom button on the size display. Existing compact/selection/scrollback/copy/resize tests remain unchanged.
4. **P2: Notes failure returned to “Saved to this workspace as you type”, with no inline Retry.** `RightPanel.svelte:180`. A held/failing PATCH reproduced missing inline recovery. Notes now retain failed text, show the error with Retry, and keep a persistent Saved state after success. Saves are serialized so a slow old PATCH cannot overwrite a newer draft; revision checks prevent old completion from changing the current draft's status. The textarea has a real accessible name and visible focus treatment. A separate ordered-request test verifies only the old write is sent while held, then the new write, then reload retrieves the new text. Cross-workspace late failures retain their existing toast rather than disappearing.
5. **P2: a live same-ID artifact replacement left the previous turn's bytes in its preview.** `OutputsPanel.svelte:102,192`. Real-shaped `artifact_added` WS event replaces the selected artifact with a newer producing turn. Red stayed on First report; new identity tracking refreshes that selection, invalidates an older pending request, and preserves selection through unrelated list changes. All prior preview race, image URL lifetime, keyboard and close behavior checks pass.
6. **P2: sparse Files trees reserved45% of each section, cutting document content while leaving large blank space above it.** `FileTree.svelte:491`. Viewed loaded light/dark two-section screenshots; two entries occupied a180.9px tree and clipped the preview halfway through its short list. Red bounds assertion recorded180.890625px. Tree now sizes to its content up to45%, preserving scroll for a large listing and giving the document spare height. Converted row indentation to logical padding. Final source/preview toggle and multi-breakpoint checks listed below.

## Independent prior verification and additional flows

The broad run reran all R1, R2, R3 shell suites, original terminal-copy, side-by-side embedding/navigation/persistence, nested split layouts, and right-panel tabs: **54 passed (3.5m)**. It includes the new first8R4 cases; no old assertion was skipped or weakened. `/tmp/otto-ux-r4-shell-broad.log`, result directory `/tmp/otto-ux-r4-shell-broad/`, `.last-run.json` passed.

Standalone bar uses intercepted contract-faithful orchestrator plan/error responses. Confirmed proposed plan→Cancel executes zero actions, preserves attributed answer, then planner502 leaves an editable input with the error in the thread. No provider/cloud action runs. An early combined run accidentally matched “Review…” to the already seeded “Zoom review” session through the legitimate addressed-send parser; changed the synthetic prompt to unambiguous “Evaluate deployment readiness…” and corrected the execute endpoint matcher to `/orchestrate/execute`. This was fixture ambiguity, not a product finding.

Files is intentionally a **read-only** preview/source viewer. Tests cover root-load failure→Retry, file preview, source view, two sections and breakpoint persistence; editable saving is exercised in Notes. Activity synthetic tasks_updated events move an item through in-progress→completed and trail_appended shows a live event. Actual PTY epoch coverage is listed in final results; do not infer it from same-epoch selection tests.

## Commands and intermediate outcomes

All Playwright commands run from WT/ui with:

```
OTTO_E2E_SLOT=ux4shell OTTO_E2E_PORT=7865 OTTO_E2E_PW_PORT=5365 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod
```

- Swarm independent red: `npx playwright test e2e/desktop-ux-r4-automation.spec.ts --grep desktop-to-tablet --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r4-shell-red` →1failed focused-picker assertion; `/tmp/otto-ux-r4-shell-red.log`.
- Stable central mount: same test with `--project=desktop-browser --project=iphone-portrait` →2passed11.6s; `/tmp/otto-ux-r4-shell-mount.log`.
- New Notes focus/terminal zoom red →2failed; `/tmp/otto-ux-r4-shell-deep-red.log`. An earlier mistaken cwd prevented writing the new spec and produced “No tests found”; it is not verification.
- Intermediate host/zoom run →2passed4failed, `/tmp/otto-ux-r4-shell-deep-green.log`: Notes focus still moved, and Reset existed only in Terminal's hidden standalone toolbar. Fixed actual SessionView toolbar and host focus preservation; did not weaken assertions.
- `/tmp/otto-ux-r4-shell-deep2.log` →4passed4failed, both engines; zoom/bar passed, Notes focus and missing inline failure recovery remained red.
- `/tmp/otto-ux-r4-shell-deep3.log` →6passed2fixture failures: both engines passed Notes focus, zoom/reset and Notes error/retry; addressed-send prompt ambiguity explained above.
- More depth `/tmp/otto-ux-r4-shell-more.log` →4passed1failed: API and Files transitions/bar passed; same-ID artifact preview replacement red.
- Broad54green exact specs: `desktop-ux-r4-shell`, `desktop-ux-r3-shell`, `desktop-ux-r2-shell`, `desktop-ux-shell`, `desktop-terminal-copy`, `desktop-side-by-side`, `desktop-split-layout`, `desktop-rpanel-tabs`, `--project=desktop-browser --workers=1 --output=/tmp/otto-ux-r4-shell-broad`.
- Follow-up `/tmp/otto-ux-r4-shell-final-depth.log` →2passed3failed: Notes save ordering and Activity live passed; sparse Files composition red in both themes; epoch test failed before restart because selection setup raced attach, not an epoch bug. Restricted epoch collection to the actual session socket and waited for its initial attach/geometry before selecting.
- Initial E2E TypeScript and UI guards exit0: `/tmp/otto-ux-r4-shell-types.log`, `/tmp/otto-ux-r4-shell-guards.log`. Direct Svelte compilation of changed components has0warnings. Final checks appended below; parent owns full npm check/build.

Broad log contains one `ResizeObserver loop completed with undelivered notifications` during nested split testing; no assertions failed. It is recorded rather than treated as a confirmed semantic defect or silently removed. No source-runtime exception is excused by a passing assertion. The parent separately owns global integration and actual production-worker checks.

## Rendered evidence actually viewed

All five fresh loaded Outputs screenshots in `/tmp/otto-ux-r4-shell-broad/desktop-ux-r2-shell-loaded-Outputs-*/loaded-outputs.png`: Native light/dark1440, Pro Dark1440, Warm lightRTL1024, Warm darkRTL390. They show contained panels/scrolling, complete selected identity and readable report content.

Also personally opened:
- Broad `desktop-ux-r3-shell-split--4d5a1-le-full-terminal-grid-light-desktop-browser/readable-split.png` and `...split--b2cb8-ble-full-terminal-grid-dark.../readable-split.png`.
- Broad R2 settled Home prompt light; R1 Warm-phone screenshot was mid-transition (do not use for publication).
- `/tmp/otto-ux-r4-shell-more/desktop-ux-r4-shell-Files--{d1fc8-oss-shell-transitions-light,f34f9-ross-shell-transitions-dark}-desktop-browser/files-preview.png` (pre-composition repair evidence).
- `/tmp/otto-ux-r4-shell-deep3/desktop-ux-r4-shell-Notes--dfcd9--retries-the-preserved-text-desktop-browser/notes-saved.png`.
- Broad standalone-answer-error.png; Native dark tray-recovered.png and Warm-dark tray-loaded-warm.png.

These contain synthetic/isolated fixtures or repository-redacted transcripts. No captured real account/session/usage data is intended for publication. Final WebKit/layout images and final per-family scores appended after verification.


## Parent completion of the handoff

The reviewer was interrupted before its final addendum. The parent inspected the completed logs: final broad54/54 passed; the subsequent two-engine run passed22 with four Files fixture failures, and its last run exposed the real phone zoom defect. Files readonly checks now compare CodeMirror line text (not layout-dependent innerText newlines), preserve aria-readonly assertions, and attempt actual keyboard editing. Phone zoom now applies the user offset above the15px rendered baseline. All14 selected handoff checks passed across Chromium/WebKit,1.9m, `/tmp/otto-ux-r4-parent-handoffs.log`, including the unchanged Swarm transition regression, Files light/dark, phone zoom and old content Retry checks.

Parent also reproduced and repaired queued Notes being submitted under a replacement login; the captured token is checked before dispatch, around the workspace read/merge/write, and before accepting completions. See [parent evidence](r4-parent.md). Notes failure/retry, serialized latest-write persistence, identity, exact focused selection across all breakpoints, initial coach draft, tablet Canvas and D2 checks passed14/14 across Chromium/WebKit,1.3m, `/tmp/otto-ux-r4-parent-final.log`.

Parent actually viewed the final light/dark Files previews and phone Notes selection screenshot from those two result folders. Sparse trees now leave both bullet points visible; the Notes range remains selected with a clear focus boundary. Browser results do not establish physical software-keyboard behavior or native VoiceOver. The interrupted review's remaining scenarios are explicitly limited below; no fifth round is planned following the user's change.

## Final scoped scores

L/I/A/S/R are layout, interaction, accessibility, states/recovery and responsiveness. Prior independently rerun surfaces retain prior judgments; changed workflows are judged from their new executed behavior and rendered evidence.

| Family / variant | L | I | A | S | R |
|---|---:|---:|---:|---:|---:|
| Shell navigation, palette and prompts, desktop/phone RTL |9.6|9.6|9.6|9.5|9.5|
| Home populated cards and spaces |9.5|9.6|9.5|9.5|9.5|
| Desktop terminal, Chat/Split/nested layouts and zoom |9.5|9.5|9.5|9.5|9.5|
| Phone/tablet terminal, output and tiles |9.4|9.5|9.3|9.4|9.5|
| Outputs, five themes and live artifact replacement |9.6|9.6|9.6|9.6|9.6|
| Activity task and trail live updates |9.5|9.5|9.4|9.5|9.5|
| Files loaded/error/source/preview, light/dark and breakpoint retention |9.5|9.5|9.4|9.5|9.5|
| Notes draft/focus/save/error/retry/identity |9.5|9.5|9.5|9.5|9.5|
| Desktop tray, long approvals and recovery |9.5|9.5|9.5|9.6|9.5|
| Desktop companion bar answer/error/plan cancellation |9.5|9.5|9.4|9.5|9.5|
| Swarm and API page state through desktop/tablet/phone |9.5|9.5|9.5|9.5|9.5|

Below-target qualifications remain specific: mobile tiled input and reading are less deeply exercised than desktop, Files' complete long-tree keyboard matrix is not established, Activity and companion-bar screen-reader announcements are not comprehensively measured. These are coverage/readability deductions, not unresolved confirmed regressions. The actual restarted-PTY epoch test passed in the22-case final run; Files fixture failures did not suppress it.
