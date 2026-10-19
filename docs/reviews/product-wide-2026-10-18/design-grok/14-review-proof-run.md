# Design review — Proof, Run with Otto, review surfaces

Reviewer: grok (read-only source trace, 2026-10-18). No browser, no screenshots, no `npm`/`cargo`. Light/dark and phone/tablet are traced from tokens, breakpoints, and markup, not from a rendered pass.

Scope read: `ui/src/modules/proof/ProofPage.svelte`, `ui/src/modules/run-with-otto/*`, `ui/src/modules/git/ReviewPanel.svelte`, `ReviewAgents.svelte`, `FindingsBoard.svelte`, `LocalReviewPanel.svelte`, `ProofPackView.svelte`, `FindingActions.svelte`, plus `ProofStatusChip.svelte` where the pack list renders it. Guidelines: `docs/design/guidelines/`.

## Scores

Honest. Target 9.8 is not met.

| Area | Score |
|---|---|
| Structure (archetype, one header, states) | 8.7 |
| Tokens, light/dark colour | 9.4 |
| Type, spacing, shape | 9.1 |
| Agents, drafts, outward actions | 8.6 |
| Copy | 8.5 |
| Accessibility | 8.6 |
| Phone and tablet | 8.0 |
| **Overall** | **8.6** |

Colour is the strong area: these files use semantic tokens, no hex, status dots plus words, and `prefers-reduced-motion` on the run rail. The gap is phone layout and a few review states that lie or cannot be stopped.

**Defects: 11.**

## What is already solid

- Proof uses `PageHeader` / `PageBody` / `LoadState` / `EmptyState` / `Modal` / `confirmer` / `ctxMenu`. A failed list is not shown as “No proof packs yet”. Phone detail has a header Back and hides the rail (`ProofPage.svelte` 794–798, 1691–1693). Delete, waive, and archive confirm. ⌘K registers the main verbs.
- Run with Otto has one primary (`Run with Otto`), source chips without rainbow hues, a stage rail, inline detect failure with Retry, repo-list failure with Retry, and Open PR goes through `confirmer` with where / what / who (`RunDetail.svelte` 92–111). Approve does not push. Stop is danger and is not the primary. Agent byline is on the gate and the PR draft.
- PR review keeps comments as drafts until `confirmOutward` (`ReviewPanel.svelte` 582–595, 666–673). Running reviews can be stopped (`1204–1211`). Findings board has loading / empty / error / stale-reload / timeline retry. Local review handoff is one primary only after the run is done.

## Defects

### 1. Phone Proof shows a “pick one” pane under the list

`ui/src/modules/proof/ProofPage.svelte:106-107`, `:883-907`, `:1682-1690`.

Auto-open is skipped on a phone, which is right, but `.main` still renders the page `EmptyState` “No proof pack open” while the rail is `flex: 1` in a column. The first screen is half list, half a void that only says to open a pack.

**Fix:** when `viewport.isPhone && !detail`, do not render `.main`. The rail already fills the page; opening a pack hides the rail and shows Back.

### 2. Proof filter counts are only the loaded page

`ui/src/modules/proof/ProofPage.svelte:164-178`, `:836-837`. Page size is 100 (`ui/src/lib/stores/proof.svelte.ts:15`).

Chips and the empty-detail sentence (`packSummary`) count `proof.packs`. If `nextCursor` is set, “100 packs · 40 passed” is not the workspace total, and choosing Passed reloads a different server page. Session packs make this common.

**Fix:** when `proof.nextCursor` is set, drop the numbers (show the status word only, and no numeric `packSummary`). Do not present a partial page as a total.

### 3. Two bare numbers on each pack row

`ui/src/modules/proof/ProofPage.svelte:850-851` and `ui/src/lib/components/ProofStatusChip.svelte:24-30`.

The row shows the done score and, when risk &gt; 0, a second number inside the status pill. Both meanings exist only in `title`, which a phone does not show. A reader sees `72 Passed 40`.

**Fix:** visible “72 done”. In the chip, prefix the score “risk 40” and set `aria-label` to `Proof passed, risk 40` (sentence-case the status). Keep the colour.

### 4. Proof filters and artifact actions are under 36px on a phone

`ui/src/modules/proof/ProofPage.svelte:1323-1335` (`.chip-btn` height 22px), `:1509-1513` (`.art-top` does not wrap), `:1549-1557` (`.link-btn` padding 0), `:1591-1594` (delete is 22×22).

`app.css` grows `.icon-btn` / `button.chip` on a coarse pointer. `.chip-btn` and `.link-btn` are not in that list. On a 320px width the title is the only flexible item, so the sha chip, status, Show, and delete eat the row and the title truncates to nothing.

**Fix:** at `max-width: 640px`, give `.chip-btn` and `.link-btn` `min-block-size: var(--hit-min)`. Let `.art-top` wrap, with the title on the first line (`flex-basis: 100%` or a second meta row).

### 5. Proof “More” does nothing when there is no workspace

`ui/src/modules/proof/ProofPage.svelte:819` and `:722-724`.

The ⋯ button is outside the `detail` guard. With no workspace the only header control is More → “Archive stale session packs…”, and `archiveStaleSessions` returns without a toast. `newPack` does toast.

**Fix:** omit the button when `!ws.currentId`, or toast “Add a workspace first” the way `newPack` does.

### 6. Run with Otto does not push on a phone

`ui/src/modules/run-with-otto/RunWithOttoPage.svelte:47-50`, `:134-192`. Close control: `RunDetail.svelte:138-140`.

List/detail is supposed to push, with Back in `PageHeader`’s `leading` slot (`layout.md` phone rules). Here the launcher, the full run list, and the detail stay stacked. `scrollIntoView` jumps to the detail, but Back is an X inside the card, below the page header, and the launcher is still above it. Tablet side-by-side (container 860px) is fine; this is the phone.

**Fix:** when `viewport.isPhone && openRun`, hide `RunLauncher` and `.list-col`, and put the same Back button Proof uses into `PageHeader` `leading` (it calls `runWithOtto.closeDetail()`).

### 7. Run source chips and the proof chip are 18–22px hits

`ui/src/modules/run-with-otto/RunLauncher.svelte:391-403` (`.src-chip` height 22px). `ui/src/modules/run-with-otto/RunDetail.svelte:180-188` and `:362-369` (the proof control is an unpadded button around an 18px pill).

Neither class is in the coarse-pointer hit list (`button.chip` / `.segmented > button` are). Eight source chips sit on a 6px gap. The proof control is the link from the approval gate to the evidence.

**Fix:** on phone, `min-block-size: var(--hit-min)` for `.src-chip` and `.chip-btn`. Do not grow the glyph; grow the hit box.

### 8. A local review cannot be stopped, and Cancelled looks finished

`ui/src/modules/git/LocalReviewPanel.svelte:488-506`. Cancel API already used by the PR panel: `ReviewPanel.svelte:521-528` (`POST /reviews/{id}/cancel`).

The running header is a spinner and “Reviewing…”. There is no Stop. Per-agent Stop in `ReviewAgents.svelte` does not exist until agent rows appear, and it does not cancel the review. Any non-`error` terminal status, including `cancelled` (Mission Control can cancel a review), falls through the `else` and renders the done board and “Send to agent”. `patterns.md` §1: stop is always reachable while work runs.

**Fix:** same running header as `ReviewPanel` (Stop → `/reviews/{id}/cancel`). Add a `cancelled` branch: “Review canceled.” and “Review again”. Do not render handoff for that status.

### 9. “Configure agents…” sits on top of the empty review

`ui/src/modules/git/ReviewPanel.svelte:1192-1197` and `:1880-1888`.

`.rp-cfg-btn` is `position: absolute; top: 0; inset-inline-end: 0` over `.rp-no-review`. On a phone or a narrow PR pane it covers the empty state. The control is also painted far from its DOM position, so focus order and sight disagree.

**Fix:** remove the absolute positioning. Leave the button in flow under the focus field, as the done state already does (`1278`).

### 10. Merge readiness hides a failed load and prints the raw CI enum

`ui/src/modules/git/ReviewPanel.svelte:324-325` and `:1332-1337`.

The `catch` leaves `mergeReadiness` null, so the card unmounts. A person cannot tell CI from “we never loaded it”, and there is no Retry. When it does load, the pill text is `CI ${ciState}` (`none`, `pending`, `success`, `failure`).

**Fix:** store an error string and render it in the card with Retry calling `loadFindingsAndReadiness`. Map the enum to “No CI”, “CI pending”, “CI passed”, “CI failed”. Keep the existing tone classes.

### 11. Review proof-pack modal and finding waive

`ui/src/modules/git/ProofPackView.svelte:88-91` (error is a paragraph, no Retry), `:81` and `:103-108` (`false_positive` via one `replace`, severity is the raw enum). Findings board already uses `sentenceCase` / `severityLabel` (`FindingsBoard.svelte:152-155`).

`ui/src/modules/git/FindingActions.svelte:146-147` and `:264-280`. `waiveFinding` and `falsePositiveFinding` take a reason (`ui/src/lib/api/client.ts:911-917`). The menu sends none. Proof packs require 10 characters (`ProofPage.svelte:1097-1114`). Waiving a finding is the audit exception; a one-click menu leaves no reason.

**Fix:** `LoadState` (or a Retry button) on `loadErr`. Render status with `sentenceCase` and severity with `severityLabel`, including the timeline suffix. Before waive and false-positive, `confirmer.promptText` (required reason) and pass it through.

## Unverified

- No light, dark, Warm, wallpaper, or reduced-transparency screenshots. Token use is consistent; contrast was not measured.
- No phone/tablet layout run. Overflow is inferred from flex and `min-width: 0`.
- E2E (`desktop-proof-packs.spec.ts`, `desktop-ux-r3-automation.spec.ts`, `desktop-page-chrome.spec.ts`) was not executed.
- Run WebSocket staleness (“Reconnecting…”) was not traced past the timeline’s `eventsError`.
