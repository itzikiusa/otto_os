# Quality iteration 6 (wave 1): ux-design lens

Reviewer: ux-design. Static and read-only: I ran no cargo, npm, builds or tests, only grep/sed/git.
Base: `fix/quality-iter6-20261023` @ `37738d908`. The fix diff was reviewed against `origin/main`.
Scope: the same as wave 0, which is `ui/src` limited to the areas listed below. Excluded: `target/`, `node_modules/` and `docs/reviews/`.

## Score

Rubric: start at 10.0. For each open finding, subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 1 | 0.10 |
| nit | 3 | 0.09 |
| **Total** | 4 | **0.19** |

**Score: 10.0 − 0.10 − 0.09 = 9.81 / 10.** Wave 0 scored 9.31.

All 9 wave-0 findings are fixed at every listed location. The 4 open findings below are new. Each sits inside an area wave 0 covered:

- 6w1-01 is a sibling of 6-06, in RecapPanel.
- 6w1-02 and 6w1-03 are sites that the wave-0 cross-cutting greps for clipboard copy and "could not" missed.
- 6w1-04 is the loading half of 6-05.

The fix diff introduced no new lens problems. The checks behind that:

- No hex colours, no native `confirm` or `alert`, and no new hand-rolled dialogs.
- No new `*Page.svelte`.
- The moved code keeps its states: OverviewTab → StoryTranscripts and DraftEditor, the ShooterGame HUD split, and Inspector → LinkListField.

## Previous-finding status (wave 0)

| Id | Status | Evidence |
|---|---|---|
| ux-design-6-01 (minor) | **Fixed** | The bare `{#await import()}` count in `ResultsGrid.svelte` is 0. There are 7 module-level `lazyComponent`s (`:7-13`), each rendered by `<LazyMount … quiet onerror>` (`:1815-1897`). Each `onerror` resets its open flag: `flow.docEditor = null`, `flow.rawDoc = null`, `showExportDialog = false`, `compare = null`, `pipelineOpen = pipelineMounted = false`, `flow.closeReview()` and `sendToAgentOpen = false`. So the next click retries. `closeReview` (`EditFlow.svelte.ts:721`) clears only the review, not the drafts. |
| ux-design-6-02 (minor) | **Fixed** | All 8 sites are now `LoadState`. **StructureView** `:660`. **LogsView** `:544`, `:619` and `:707`, each keeping Sign in via `errorActions`. **MetricsPanel** `:143-148`, where the denied case moves into the error detail. **MissionControlPage** `:383`. **PrList** `:195`, with `errorCause` plus the message and Retry. A scripted scan of every `<EmptyState>` opening tag finds no "Couldn’t", Retry or error title without `tone`. |
| ux-design-6-03 (minor) | **Fixed** | `AnalysisTab.svelte:96-97` adds `detailLoading` and `detailError`, `:330` uses `loadErrorText` with no toast, `:887-890` renders `LoadState` before the "Configure lenses…" branch, and `:314` re-select retries when `activeDetail` is null. There is also `retrySelect` (`:336`). |
| ux-design-6-04 (minor) | **Fixed** | **AgentPage** `:441-443`: `agentsError` → `LoadState` with Retry, ahead of "Agent not found", and `:93` clears `lookedUp` on failure. **MockupAnnotations** `:34`, `:405-411`: `loadError` → `LoadState`, and the "No annotations yet" line is suppressed. **LinkedCanvases** `:51-62`, `:171-177`: `pickerLoading` and `pickerError` → `LoadState`, and "No other canvases" waits for a successful load. **RecoveryView** `:137`: `&& !error`. |
| ux-design-6-05 (minor) | **Fixed** (the loading half is new as 6w1-04) | `OttoServerHome.svelte:331-333`: a failed attach load replaces the checkbox with `LoadState` + Retry. `:516-517`: gateway tools use `LoadState` + Retry. `:245` and `:282` use `loadErrorText`. |
| ux-design-6-06 (minor) | **Fixed** | `RoomRecap.svelte:20-21`: `error` is `{ what, detail, check }`, with `loadErrorText` and an assertive announce. `:47` "Couldn’t check the local recap engines." and `:49` "Couldn’t prepare the recap.". `:105`: "Retry checks" shows only when `error.check`. `:98`: the banner shows only `!prepare`. |
| ux-design-6-07 (nit) | **Fixed** (the listed sites; the rest of the class is new as 6w1-02) | `lib/clipboard.ts:61` throws "The clipboard isn’t available here. Select the text and copy it manually.", which fixes every `toastError` site. The nine "restates the cause" bodies now carry the next step. `WorkItemDetail.svelte:317` has its `else`. `GameMatch.svelte:138-139` uses `copyText` with "Select the invitation link and copy it." |
| ux-design-6-08 (nit) | **Fixed** | All 12 listed files have 0 "could not", "Cannot open" or "failed to load" hits. Examples: `FileTree.svelte` "Couldn’t open {path}" and `PrList` "Couldn’t load pull requests.". |
| ux-design-6-09 (nit) | **Fixed** | `OpenApiView.svelte:208-210`: `{#if groups.size === 0}<EmptyState … title="No operations in this spec" …/>`. |

Totals: 9 fixed, 0 partly fixed, 0 invalid.

## Areas covered this iteration

The same list as wave 0:

- All 16 iteration-5 findings, re-checked against the current code (closed in wave 0; re-confirmed that none regressed).
- **Product (open since iteration 5):** AnalysisTab past-run load (`selectHistory`), PlanTab (`loadError` → `LoadState`, fine), RewriteTab (`LoadState` + stale bar, fine), OverviewTab version picker and transcripts (now in `StoryTranscripts.svelte` / `DraftEditor.svelte`), MockupAnnotations, LinkedCanvases, SourceSearch filters.
- **Design Hall Site studio internals (open since iteration 5):**
  - SiteStudio: library load, insert, page and section confirms, layer rename prompt. New: the brand-kit `LoadState` in `Inspector.svelte:607-608`.
  - LeftPanel: library error with Retry.
  - Assist: OttoPanel load and `LoadState` (now `loadErrorText`), apply-variant and apply-conflict confirms, VariantsTray Apply titles, TurnMessage "Apply on top…".
- **Brokers (open since iteration 5):**
  - TopicDetail: consume, peek and tail states, produce validation, the prod confirm, config change confirm, typed-name delete. The live regions moved to `announce()`.
  - GroupsTab: opens on the first group, plus a 403 ACL banner.
  - ReplayPanel.
- **Personal Agents and Mission Control detail (open since iteration 5):** AgentPage, AgentActivity, AgentAutonomy, AgentMemoryInspector, AgentDocuments, WorkItemDetail.
- **Vault (open since iteration 5):** RefineDrawer, VaultPage note-open error banner, RecoveryView (trash and history), OpenApiView.
- **Rooms:**
  - RoomRecap: consent and capture controls, prepare modal.
  - RecapPanel and RecapSettingsModal.
  - GameMatch: invite copy, error and reconnect.
  - KartGame and ShooterGame: boot, error and loading covers. New: the HUD split into `hud/Knocked`, `MatchResults` and `PauseMenu`.
- **MCP:** OttoServerHome (session attach toggle, gateway tools).
- **Agents:** JiraIssuePicker states (now `providerErrorText`), plus panels FileTree root and viewer errors.
- **Cross-cutting greps across `ui/src`:**
  - Bare `{#await import(`. The only hit is `UsagePage.svelte:711`, which has pending, then and catch, as accepted before.
  - `EmptyState` used as an error without `tone="error"`. A scripted scan of every opening tag finds none.
  - `Couldn’t copy` / `copyText` paths with no next step. See 6w1-02.
  - "could not / cannot / failed to" in user strings. See 6w1-03.
  - `toastError('Couldn’t load …')`, tracing what each view shows after the toast. These are unchanged from wave 0 and still accepted.
  - Hand-rolled "Couldn’t load" blocks and whether each has a Retry.
  - Module `*Page.svelte` files without `PageHeader`/`PageBody`. The diff adds none.
  - `PageHeader` blocks with more than one `.primary`. No new cases.
  - Hand-rolled `role="dialog"`. Only the known popovers, the palette and the bell.
  - Toasts used for field validation.
- **Fix-diff review (`git diff origin/main -- ui/src`):**
  - Added lines scanned for hex colours, native dialogs, raw `e.message` in UI copy, and new `.primary` buttons.
  - The changed covered-area files were read: Product, Site studio, Brokers, Personal Agents, Mission Control, Vault, Rooms, MCP, panels and JiraIssuePicker.

## Areas NOT covered yet

These are unchanged from wave 0 and were deliberately not widened:

- Game HUD and menus at phone width, which needs screenshots.
- Swarm and Loops detail flows.
- Assistant, Snip editor and Proof flows, and the tray pages.
- Canvas and the Workflows inspector.
- The remaining Settings pages.
- Phone and tablet layouts.

---

## Findings

### ux-design-6w1-01: minor. Room recap panel: "Generate summary" failure shows only the raw server message, and a failed refresh shows a bare cause with Retry (sibling of 6-06)

- **File:** `ui/src/modules/rooms/RecapPanel.svelte:91`, `:112-113`
- **Evidence:**
  - `generate()` catches with `actionError = e instanceof Error ? e.message : 'Summary generation couldn’t start.'` (`:91`).
  - `recapRequest` throws `new Error(problem.message …)` (`recap-client.ts`), so the visible line (`:113`, `<p class="error">{actionError}</p>`) and the assertive announce (`:21`) carry only the daemon's text. There is no "Couldn’t start the summary." headline. This is the same defect 6-06 fixed in `RoomRecap.svelte`, where `fail(what, e)` now keeps `{ what, detail }`.
  - The other actions in the same file already lead with what failed: `:92` "Couldn’t cancel generation. Try again.", `:93` "Couldn’t export the recap. Try again." and `:95` "Couldn’t copy the draft…".
  - A failed background refresh (`:112`, `{:else if error}<p class="error">{error} <button …>Retry</button></p>`) renders `loadErrorText(e)` alone, for example "The daemon isn’t reachable." + Retry. Nothing says that the recap refresh is what failed.
  - `content.md` §4: the title is what failed, and raw detail goes last.
- **Fix:**
  - In `generate()`, set `actionError = \`Couldn’t start the summary. ${loadErrorText(e)}\``, or adopt RoomRecap's `{ what, detail }` shape and render `<strong>{what}</strong> <span>{detail}</span>`.
  - At `:112`, render `<strong>Couldn’t refresh this recap.</strong> <span>{error}</span>` before Retry, or use `<LoadState what="this recap" variant="compact" error={error} onretry={() => load()} />`, which shows the stale bar when `empty` is false.

### ux-design-6w1-02: nit. Nine more clipboard-failure paths still don't say what to do next, and one is silent on origins without `navigator.clipboard`

- **Files:**
  - Bodies that restate the cause or carry no body:
    - `ui/src/modules/settings/Users.svelte:57`, `:66` ("The clipboard write was blocked.")
    - `ui/src/modules/settings/Logs.svelte:140` (same body)
    - `ui/src/modules/settings/TrustSafety.svelte:190` (same body)
    - `ui/src/shell/NotificationBell.svelte:252` ("Copy failed", "The clipboard is not available here.")
    - `ui/src/shell/Palette.svelte:150` ("The clipboard is not available here.")
    - `ui/src/shell/Navigator.svelte:444` (`toasts.warn('Couldn’t copy the path')`, with no body)
    - `ui/src/modules/agents/history/HistoryPage.svelte:223` (body is the value only, with no instruction)
  - Bypassing `lib/clipboard.ts`: `ui/src/modules/agents/conversation/Markdown.svelte:108-112`.
- **Evidence:**
  - `content.md:100`: "Clipboard failures say what to do next ('Select the text and copy it manually.'), not the title again."
  - Wave 1 fixed the ~25 sites 6-07 listed, plus the `copyTextOrThrow` message. These sites were outside that grep.
  - `Markdown.svelte:109` is `void navigator.clipboard?.writeText(text).then(…)`. Where `navigator.clipboard` is undefined (the LAN and self-signed origins `lib/clipboard.ts` exists for), the optional chain short-circuits the whole call. Neither toast fires, so the click does nothing. This is the same bypass 6-07 fixed in GameMatch.
- **Fix:**
  - Change each body to "The clipboard isn’t available here. Select the text and copy it manually." (or "…Select the path / the entry…"). Alternatively, route them through a shared `copyWithToast(text, what)`; one exists in `design-hall/brand/edit.ts:13` and could move to `lib/clipboard.ts`.
  - In `Markdown.svelte`, use `copyText(text)` with the same success and failure toasts.

### ux-design-6w1-03: nit. "could not" and "Failed to load" remain in four user-visible strings

- **Files:**
  - `ui/src/modules/snip/SnipEditor.svelte:119` ('The image could not be decoded. Retry loading the snip.')
  - `ui/src/modules/aws/RegionErrors.svelte:18` ("{n} regions could not be listed — the rows below come from the others.")
  - `ui/src/lib/design/scene3d/Scene3DViewport.svelte:1037` ("{n} models failed to load")
  - `ui/src/lib/stores/browser.svelte.ts:368` (the fallback 'Failed to load page', rendered via `BrowserView.svelte:832` `error={browser.pageError}`)
- **Evidence:**
  - `content.md:91-93`: "Write 'Couldn’t …', never 'Could not …', in titles, bodies and toasts alike."
  - 6-08 fixed the 12 listed sites. These four were missed by the wave-0 grep. The `lib/uiCommands/*` "Could not …" strings are agent-facing command errors and are excluded here.
- **Fix:** Use "The image couldn’t be decoded.", "{n} regions couldn’t be listed…", "Couldn’t load {n} models" and "Couldn’t load the page.".

### ux-design-6w1-04: nit. MCP server home: while the session-attach setting loads, the checkbox shows a guessed "attached" tick

- **File:** `ui/src/modules/mcp/OttoServerHome.svelte:238`, `:343-344`
- **Evidence:**
  - `loadAttach` sets `attach = null` (`:238`). Until the response arrives, the checkbox renders `checked={attach?.attached ?? true}` and `disabled={!attach || …}`. That is a disabled, ticked "Attach to sessions in {workspace}", a value the daemon hasn't reported.
  - 6-05 removed this guess for the failure path (`:331-333` now shows `LoadState`). The loading path still shows it on every workspace switch, and for a detached workspace the box visibly flips from ticked to unticked.
  - AGENTS.md: "Design every state: loading, empty, error and loaded."
- **Fix:** While `attach === null && !attachError`, render `<LoadState what="the session-attach setting" variant="compact" loading empty rows={1} />` (or a one-row `Skeleton`) in place of the checkbox. Alternatively, keep the box but render it unticked with `aria-busy="true"` and "Checking…" in the copy line.
