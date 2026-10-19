# Design review 08 — Proof, Run with Otto, review UI

Static review of `ui/src/modules/proof/ProofPage.svelte`, `ui/src/modules/run-with-otto/` (`RunWithOttoPage`, `RunLauncher`, `RunDetail`, `RunStageRail`, `runStatus`, `runEvidence`), and the review UI in `ui/src/modules/git/` (`ReviewPanel`, `ReviewAgents`, `FindingsBoard`, `LocalReviewPanel`, `ProofPackView`, `FindingActions`). Diff viewer and PTY internals were not reviewed. The inline “Open” terminal chrome in `ReviewAgents` was.

Checked against `docs/design/guidelines/` (structure, tokens, type, components, agents/outward actions, copy, accessibility, phone/tablet, keyboard). Not rendered in light/dark or at 1440/phone; colour claims are from token use, not a measured contrast pass. Target 9.8 is not met.

## Scores

| Area | Score | Why it is not 9.8 |
|---|---|---|
| Structure | 9.3 | Proof and Run with Otto both break phone list/detail. Review’s start controls sit after, or off, the button that uses them. |
| Tokens and colour | 9.7 | Module CSS uses semantic tokens and the `--fs-*` scale. No hex. No measured contrast miss. |
| Type and spacing | 9.6 | Body copy stays on the scale. A few local controls are 21–22 px tall (called out under accessibility). |
| Components and states | 9.2 | Loading / empty / error / stale are present almost everywhere. The proof-pack modal and a running local review are the holes. |
| Agents and outward actions | 9.3 | PR comments and “Open PR” confirm where / what / who, and drafts stay drafts. Waive and false-positive do not record why. A local review cannot be stopped. |
| Copy | 9.4 | “CI none”, raw `false_positive` transitions, and `Failed to fetch` as the whole error. |
| Accessibility | 9.1 | Phone hit areas under 36 px, finding titles that shrink away, paths ellipsized from the end, a checkbox label that eats text selection. |
| Responsive | 9.0 | Phone proof and Run with Otto do not push. Review paths and finding titles fail in a narrow column even when the viewport is wide. |
| Keyboard | 9.5 | Controls are real buttons, menus go through `ctxMenu`, focus rings are kept. Review’s main verbs are not in ⌘K. |

**Overall: 9.3 / 10. Defects: 14.**

What already holds: one `PageHeader` and one primary on Proof (`Assemble…`) and on Run with Otto (`Run with Otto`); `LoadState` + `EmptyState` on both pages; proof deep-link and phone back button once a pack is open; run approval names the agent, branch, proof and findings and links to them before Approve; Open PR and Post to PR use an outward confirm; reduced motion cancels the stage pulse; RTL flips the stage arrow; file paths that are marked `dir="ltr"` stay LTR.

## Defects

1. **Phone proof list is only half the page.** `ui/src/modules/proof/ProofPage.svelte:901` still renders the “No proof pack open” pane whenever `showRail` is true, including on a phone, where auto-open is deliberately skipped (`:107`). At `ProofPage.svelte:1682` the split becomes a column and both `.rail` and `.main` are `flex: 1`, and `.main` is never hidden. The pack list is cut to about half the viewport, under a dead empty state with no CTA. **Fix:** when `viewport.isPhone && !detail`, do not render `<section class="main">`. Let `.rail` be `flex: 1; min-height: 0` with the list scrolling inside it.

2. **Run with Otto does not push on a phone.** `ui/src/modules/run-with-otto/RunWithOttoPage.svelte:49` stacks the detail under the launcher and the full run list, and `show()` only `scrollIntoView`s it (`:105`). `PageHeader` (`:121`) has no `leading` back control. Proof already does the push (`ProofPage.svelte:795`). On a phone, opening a run leaves the launcher and every row above the detail. **Fix:** when `viewport.isPhone && openRun`, hide the launcher and the list, and put a “Back to runs” `icon-btn` in `PageHeader`’s `leading` slot that calls `runWithOtto.closeDetail()`.

3. **Artifact title collapses; row actions are below 36 px.** `ProofPage.svelte:1509` (`.art-top`) is a nowrap flex. `.art-title` is `flex: 1; min-width: 0` (`:1514`), so on a phone the title ellipsizes to nothing while the sha chip, status, Show and Delete keep their width (`:968`). Filter chips are 22 px (`:1323`), Show/Hide is an unpadded `.link-btn` (`:1549`), and Delete is `.icon-btn.small` at 22 px (`:1591`). Those classes are not `button.chip`, so `app.css`’s coarse-pointer `::after` does not grow them to `--hit-min` (36 px). **Fix:** `flex-wrap: wrap` and `flex: 1 0 100%` on `.art-title` (actions on the next line). Under `@media (pointer: coarse)`, set `min-block-size: var(--hit-min)` on `.chip-btn`, `.link-btn` and `.icon-btn.small`.

4. **Source chips are 22 px on a phone.** `ui/src/modules/run-with-otto/RunLauncher.svelte:391` (`.src-chip`, `height: 22px`) is the control that inserts a Jira/GitHub/story template. It is not `button.chip`, so the coarse-pointer hit rule never applies. **Fix:** `@media (pointer: coarse) { .src-chip { min-block-size: var(--hit-min); } }`.

5. **Run failures show the raw exception.** `RunDetail.svelte:63`, `:78` and `:108`, and `RunLauncher.svelte:162` and `:203`, set `error` to `e.message`. A dead daemon surfaces as “Failed to fetch”; a 401 is not turned into “sign in again”. The rest of these modules use `loadErrorText` / `toastError`. **Fix:** keep a fixed title in the alert (“Couldn’t approve the run”, “Couldn’t launch the run”, …) and put `loadErrorText(e)` under it.

6. **The control that starts a review is in the wrong place, and two states hide it.** `ReviewPanel.svelte:1166` puts “Send to review agents” inside `EmptyState`, then the Jira row (`:1173`) and the focus textarea (`:1184`) that `startReview` actually sends (`:500`). `.rp-cfg-btn` is `position: absolute; top: 0` (`:1883`), so in a narrow PR column whose viewport is still wider than 640 px it overlaps the empty-state title. The ≤640 rule (`:2445`) does not see that column. Cancelled (`:1220`) and error (`:1228`) call `startReview` with no focus field on screen. **Fix:** in every state that can start a run, render Jira, focus, then Configure, then one primary, in normal flow. Delete the absolute positioning; if the corner placement stays, gate it with a container query on `.rp`, not the viewport.

7. **A running local review cannot be stopped.** `LocalReviewPanel.svelte:488` shows “Reviewing…” and, before any agent row exists, only “Agents starting…” (`:496`). There is no cancel. The PR panel stops the same resource with `POST /reviews/{id}/cancel` (`ReviewPanel.svelte:522`). **Fix:** add that Stop button to the local running header. Confirm it (`confirmer.ask`, danger, confirm “Stop review”), matching “Stop run…” in `RunDetail.svelte:72`. Per-agent Stop in `ReviewAgents` can stay.

8. **Waive and false-positive record no reason.** `FindingActions.svelte:264` and `:277` call `falsePositiveFinding` / `waiveFinding` with no reason. Both client functions accept `reason` (`ui/src/lib/api/client.ts:911` and `:916`) and the server writes it on the audit event. One menu click drops a finding, including a blocker, with nothing to read later. Proof’s waive already requires 10 characters (`ProofPage.svelte:1096`). **Fix:** label the items “Mark false positive…” and “Waive…”, `promptText` for a reason, and pass it through. Do not fire on the first click.

9. **Finding title shrinks to a sliver.** `FindingsBoard.svelte:397` (`.fb-card-head`) wraps, and `.fb-title` (`:410`) is `min-width: 0` + `nowrap` with the default shrink. Badges keep their content width, so on a phone the title collapses beside “Critical / Open / Needs approval” instead of taking the next line. The accessible name is still the full title; a sighted user does not see it. **Fix:** `.fb-title { flex: 1 0 100%; }` and keep `.fb-caret { flex-shrink: 0 }`.

10. **Review paths hide the filename; “Show diff” is a 16 px target.** `ReviewPanel.svelte:2059` and `FindingsBoard.svelte:427` ellipsize with `nowrap`. The ≤1024 overrides (`ReviewPanel.svelte:2423`, `FindingsBoard.svelte:489`) only set `max-width: 100%`, so a touch user sees the directory prefix. `LocalReviewPanel.svelte:845` already wraps. `.rp-diff-toggle` (`ReviewPanel.svelte:2100`) is `padding: 0` and is not in the block that grows the other zero-padding buttons (`:2427`). **Fix:** use the local-review path rules (`white-space: normal; overflow-wrap: anywhere`). Add `.rp-diff-toggle { min-height: var(--hit-min); }` inside the existing `@media (max-width: 1024px)`.

11. **Proof Pack modal: failed load has no Retry, and Export still fires.** `ProofPackView.svelte:90` renders `loadErr` as a paragraph. The footer (`:171`) disables Export only while `loading` or `exporting`, so after a failure Export is enabled and fails again. `transitionLabel` (`:81`) prints raw `from_status` (`false_positive`); `FindingsBoard` sentence-cases the same values. **Fix:** `LoadState` with `onretry={() => load(reviewId)}`. Disable Export when `!pack`. Run both statuses through `sentenceCase`.

12. **Merge readiness says “CI none”.** `ReviewPanel.svelte:1332` prints `CI {ciState}` with a fallback of `'none'` (`MergeReadiness.ci_status` is `"success" | "failure" | "pending" | "none"`). `PrList.svelte:139` already maps those to “CI passing / CI failing / CI running” and hides the none case. **Fix:** use that mapping; omit the chip when status is null or `none`.

13. **Selecting finding text toggles the checkbox.** `LocalReviewPanel.svelte:542` wraps the path and body in `<label for="lrp-chk-…">`. A drag to copy the file path activates the checkbox, so the set sent by “Send to agent” changes under the pointer. **Fix:** keep the `<label>` on the checkbox only (`aria-label="Select finding"` plus the severity). Leave `.lrp-comment-body` outside it.

14. **Review verbs are not in ⌘K.** `ReviewPanel` and `LocalReviewPanel` never call `registry.register`. `GitPage`’s git commands do not include them. “Send to review agents”, “Stop review” and “Post N comments to PR…” are buttons only. Proof (`ProofPage.svelte:775`) and Run with Otto (`RunLauncher.svelte:211`) register theirs. **Fix:** while the panel is mounted, register the verb that matches the current status (start, stop, or post), with `group: 'Git'` and keywords `review findings pr`.

## Not counted

- Provider ids (`claude`, `codex`) shown as option labels. They are the product names the daemon runs.
- Comment severity chips `info` / `warn` / `bug` rendered uppercased by CSS, with a matching colour class. Readable, and not the only signal.
- No light/dark screenshot in this pass. Token use does not show a theme-specific break; that is not a measured contrast sign-off.
