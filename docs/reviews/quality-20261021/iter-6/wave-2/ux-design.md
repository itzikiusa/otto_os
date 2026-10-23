# Quality iteration 6 (wave 2): ux-design lens

Reviewer: ux-design. Static and read-only: I ran no cargo, npm, builds or tests, only grep/sed/git.
Base: `fix/quality-iter6-20261023` @ `4f6d6ab30`. The wave-2 fixes were read as `git diff 37738d908 HEAD -- ui/src` (wave 1's base to now), and the whole fix diff was also scanned against `origin/main`.
Scope: the same as wave 1, which is `ui/src` limited to the areas listed below. Excluded: `target/`, `node_modules/` and `docs/reviews/`.

## Score

Rubric: start at 10.0. For each open finding, subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 0 | 0.00 |
| nit | 1 | 0.03 |
| **Total** | 1 | **0.03** |

**Score: 10.0 − 0.03 = 9.97 / 10.** Wave 1 scored 9.81.

All 4 wave-1 findings are fixed at every listed location. The one open finding is new. It is in the same view as 6w1-04 (OttoServerHome): the fix removed the guessed tick from the checkbox, but the "What my sessions see" panel lower on the page still shows the same guess, plus a "0" tool count while loading.

The fix diff introduced no new lens problems. The checks behind that:

- No hex colours in added lines, no native `confirm`/`alert`/`prompt`, no new hand-rolled `role="dialog"`, no new `*Page.svelte`, and no new `.primary` inside a `PageHeader`.
- The wave-2 changes in the covered areas are mostly the a11y live-region sweep (`role="status"`/`role="alert"` → `announce()`/`announceShown`). None of them changed visible copy or removed a state.
- The `[role='alert'] { color: var(--danger) }` selectors became `.form-error`/`.join-error`/`.recap-error`, so error lines are still red (RoomLobby, RoomSettingsModal, RecapSettingsModal, StartRoomModal, GameGuest, RecapImage).
- Two pieces of code moved and kept their states:
  - The Jira block moved from OverviewTab to `JiraIssueSection.svelte`. It still has its 3 `LoadState`s (Jira details, editmeta, development info). OverviewTab keeps all 12 `toastError` action failures and the discovery `confirmer.ask`. The combined count of `LoadState`/`toastError`/`confirmer` is 18, the same as before the move.
  - TestCasesTab reorder moved to `caseOrder.svelte.ts`. It keeps the "Couldn’t save the order" toast and the position announce.

## Previous-finding status (wave 1)

| Id | Status | Evidence |
|---|---|---|
| ux-design-6w1-01 (minor) | **Fixed** | `RecapPanel.svelte:92`: `generate()` now sets ``actionError = `Couldn’t start the summary. ${loadErrorText(e)}` ``, so the headline says what failed and the cause comes after it. `:112-143`: the old bare `{error} Retry` line is gone. The whole panel is now one `<LoadState what="this recap" variant="compact" {loading} {error} empty={!detail} onretry=…>`. With no detail it shows "Couldn’t load this recap" + Retry. Over a loaded recap it shows the stale bar ("Couldn’t refresh this recap — showing the last good load. {error}" + Retry), which `LoadState` now announces itself. The sibling `StartRoomModal.svelte:25-28` also leads with what failed ("Couldn’t open the room window." / "Couldn’t start this room."). |
| ux-design-6w1-02 (nit) | **Fixed** | `lib/clipboard.ts:67` adds `COPY_FAILED_BODY` ("The clipboard isn’t available here. Select the text and copy it manually.") and `:73` adds `copyWithToast`. All 9 sites use one of them or a matching body. **Users** `:57`, `:66`. **Logs** `:140`. **TrustSafety** `:191`. **NotificationBell** `:250`. **Palette** `:150`. **Navigator** `:444` ("…Select the path and copy it manually: {path}"). **HistoryPage** `:223`. **Markdown.svelte** `:112-114` now uses `copyText`, so the `navigator.clipboard?.` short-circuit is gone and the failure toast fires. A repo-wide grep for "clipboard write was blocked", "clipboard is not available" and `navigator.clipboard?.writeText` finds no UI hits left. |
| ux-design-6w1-03 (nit) | **Fixed** | `SnipEditor.svelte:119`: "The image couldn’t be decoded. Retry loading the snip." `RegionErrors.svelte:17`, `:23`: "{n} regions couldn’t be listed — …". `Scene3DViewport.svelte:1037`: "Couldn’t load {n models}". `browser.svelte.ts:368`: the fallback is "Couldn’t load the page." `ReaderView` renders it under its own headline. |
| ux-design-6w1-04 (nit) | **Fixed** | `mcp/sessionAttach.ts` adds `attachView` → `'no-workspace' \| 'error' \| 'loading' \| 'ready'`. `OttoServerHome.svelte:328-352`: `error` → `LoadState` + Retry, and `loading` → `<LoadState … loading empty rows={1} />`. The checkbox renders only when the state is `ready && attach`, with `checked={attach.attached}`. The `?? true` guess is gone from the checkbox. The same guess survives elsewhere in the view; see 6w2-01. |

Totals: 4 fixed, 0 partly fixed, 0 still open, 0 invalid.

## Areas covered this iteration

The same list as wave 1 (and wave 0):

- All 16 iteration-5 findings, re-checked against the current code (closed in wave 0; re-confirmed that none regressed).
- **Product (open since iteration 5):** AnalysisTab past-run load (`selectHistory`), PlanTab (`loadError` → `LoadState`, fine), RewriteTab (`LoadState` + stale bar, fine), OverviewTab version picker and transcripts (now in `StoryTranscripts.svelte` / `DraftEditor.svelte`; the Jira block is now `JiraIssueSection.svelte`), MockupAnnotations, LinkedCanvases, SourceSearch filters.
- **Design Hall Site studio internals (open since iteration 5):**
  - SiteStudio: library load, insert, page and section confirms, layer rename prompt. Also the brand-kit `LoadState` in `Inspector.svelte:607-608`.
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
  - KartGame and ShooterGame: boot, error and loading covers, plus the HUD split into `hud/Knocked`, `MatchResults` and `PauseMenu`.
- **MCP:** OttoServerHome (session attach toggle, gateway tools).
- **Agents:** JiraIssuePicker states (now `providerErrorText`), plus panels FileTree root and viewer errors.
- **Cross-cutting greps across `ui/src`:**
  - Bare `{#await import(`. The only hit is `UsagePage.svelte:711`, which has pending, then and catch, as accepted before.
  - `EmptyState` used as an error without `tone="error"`. A scripted scan of every opening tag finds none.
  - `Couldn’t copy` / `copyText` paths with no next step. None are left; see 6w1-02.
  - "could not / cannot / failed to" in user strings. None are left in the covered areas; the remaining hits are code comments, the agent-facing `lib/uiCommands/*`, and game item hints.
  - `toastError('Couldn’t load …')`, tracing what each view shows after the toast. These are unchanged and still accepted.
  - Hand-rolled "Couldn’t load" blocks and whether each has a Retry.
  - Module `*Page.svelte` files without `PageHeader`/`PageBody`. The diff adds none.
  - `PageHeader` blocks with more than one `.primary`. No new cases.
  - Hand-rolled `role="dialog"`. Only the known popovers, the palette and the bell.
  - Toasts used for field validation.
- **Fix-diff review (`git diff 37738d908 HEAD -- ui/src`, and `origin/main` for the whole branch):**
  - Added lines scanned for hex colours, native dialogs, raw `e.message` in UI copy, and new `.primary` buttons.
  - The changed covered-area files were read: Product, Site studio, Brokers, Personal Agents, Mission Control, Vault, Rooms, MCP, panels and the shell copy sites.

## Areas NOT covered yet

These are unchanged from wave 1 and were deliberately not widened:

- Game HUD and menus at phone width, which needs screenshots.
- Swarm and Loops detail flows.
- Assistant, Snip editor and Proof flows, and the tray pages.
- Canvas and the Workflows inspector.
- The remaining Settings pages.
- Phone and tablet layouts.
- The other MCP tabs (PolicyForm, ServerForm, ProfilesTab, AuditTab, ExposePanel, TokensPanel). Wave 2 touched them, but they were never in this lens's area list.

---

## Findings

### ux-design-6w2-01: nit. MCP server home, "What my sessions see": guessed "attached" and a "0" gateway count while loading or failed (the 6w1-04 guess, a second time in the same view)

- **File:** `ui/src/modules/mcp/OttoServerHome.svelte:58`, `:510`, `:517`
- **Evidence:**
  - `:58` `const attached = $derived(attach?.attached ?? true);`. It is used only at `:510`: `Built-in tools: {attached ? 'attached' : 'not attached'} — Otto’s read-only tool server …`.
  - `attach` is `null` in three cases:
    - while `loadAttach` runs (`:241`, on every workspace switch);
    - after a failed read (`attachError`);
    - with no workspace (`:303`).
  - In all three, this line states "Built-in tools: attached", a value the daemon hasn't reported. The hero row above it now honestly shows a loading `LoadState` or "Couldn’t load the session-attach setting" + Retry (the 6w1-04 fix at `:328-331`). So during a failure the page contradicts itself. For a detached workspace, the line also flips from "attached" to "not attached" once the load lands.
  - Same panel, `:517`: `Gateway tools from governed external servers: {gatewayNames.length}` renders inside the `{:else}` branch of `{#if !wsId} … {:else if gatewayError}`. While `loadGatewayTools` runs (`gatewayTools = []` at `:305`, then `gatewayLoading = true` at `:276`), it reads "…: 0", and only the Refresh button says "Refreshing…".
  - AGENTS.md: "Design every state: loading, empty, error and loaded." The 6w1-04 rationale also applies: never show a guessed value.
- **Fix:**
  - Drive `:510` from `attachState`, which is already derived at `:40`:
    - `ready` → `attached` / `not attached`, from `attach.attached`;
    - `loading` → "Built-in tools: checking…";
    - `error` → "Built-in tools: unknown (see above)".
  - Then delete the `?? true` at `:58`, or inline `attach.attached`.
  - Add `{:else if gatewayLoading && !gatewayTools.length}<LoadState what="the gateway tools" variant="compact" loading empty rows={1} />` before the count branch, so "0" only shows after a successful empty load.
