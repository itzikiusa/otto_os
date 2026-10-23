# Quality iteration 6 (wave 3): ux-design lens

Reviewer: ux-design. Static and read-only: I ran no cargo, npm, builds or tests, only grep/sed/git.
Base: `fix/quality-iter6-20261023` @ `2065f00e9`. The wave-3 fixes were read as `git diff 4f6d6ab30 HEAD -- ui/src` (wave 2's base to now, 82 files). Added lines across the whole branch were also scanned against `origin/main`.
Scope: the same as waves 0 to 2, which is `ui/src` limited to the areas listed below. Excluded: `target/`, `node_modules/` and `docs/reviews/`.

## Score

Rubric: start at 10.0. For each open finding, subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Count | Deduction |
|---|---:|---:|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 0 | 0.00 |
| nit | 1 | 0.03 |
| **Total** | 1 | **0.03** |

**Score: 10.0 − 0.03 = 9.97 / 10.** Wave 2 also scored 9.97, and wave 1 scored 9.81.

The one wave-2 finding (6w2-01) is fixed at every listed location. The one open finding is new, and the wave-3 fixes caused it. Two Rooms recap error states were moved onto `LoadState` (the a11y focus-retention fixes). In both, the `LoadState` headline was added in front of an existing message without adjusting either one. The result is a doubled "Couldn’t load … Couldn’t load …" line in one place and a "load" vs "check" mismatch in the other.

## Previous-finding status (wave 2)

| Id | Status | Evidence |
|---|---|---|
| ux-design-6w2-01 (nit) | **Fixed** | `mcp/sessionAttach.ts` adds `builtInToolsLine(view, attach)`. It returns "attached"/"not attached" only when the state is `ready`. Otherwise it returns "checking…" (loading), "unknown (see above)" (error) or "select a workspace". `OttoServerHome.svelte:510` renders it, and the `attached = attach?.attached ?? true` derived value is gone. The failed-save revert at `:254`/`:269` now uses the reported value (`const reported = attach.attached`) instead of `?? true`. The gateway count: `gatewayView()` returns `loading` while a load runs with nothing loaded yet. `:516` renders `<LoadState what="the gateway tools" variant="compact" loading empty rows={1} />` for it, so "…: 0" at `:518` only shows after a successful empty load. A refresh over an existing list keeps the list. The `N tools · N exposed` counts line at `:370` also renders only once `status` has loaded. |

Totals: 1 fixed, 0 partly fixed, 0 still open, 0 invalid.

## Areas covered this iteration

The same list as waves 0 to 2:

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
  - `Couldn’t copy` / `copyText` paths with no next step. None are left; the clipboard greps (`navigator.clipboard?.writeText`, "clipboard write was blocked") find no UI hits.
  - "could not / cannot / failed to" in user strings. None are left in the covered areas.
  - `toastError('Couldn’t load …')`, tracing what each view shows after the toast. These are unchanged and still accepted.
  - Hand-rolled "Couldn’t load" blocks and whether each has a Retry. These were re-listed in the covered areas: SourceSearch, AnalysisTab, PublishDialog, StoryTranscripts, RecoveryView, TopicDetail, AgentDocuments, WorkItemDetail, LeftPanel and Inspector. All are unchanged since wave 2, or changed only in how they are announced.
  - Module `*Page.svelte` files without `PageHeader`/`PageBody`. Five exist: Share, Capabilities, Agents, Tray and SkillsEval. All five are identical to `origin/main` and outside the area list. The diff adds none.
  - `PageHeader` blocks with more than one `.primary`. No new cases.
  - Hand-rolled `role="dialog"`. Only the known popovers, the palette and the bell. The one `role="dialog"` added in the diff is `git/graph/RefPopover.svelte`. It is the existing ref popover, moved verbatim out of `GraphView.svelte:2827`.
  - Toasts used for field validation. None found.
- **Fix-diff review (`git diff 4f6d6ab30 HEAD -- ui/src`, and `origin/main` for the whole branch):**
  - The 6,108 added lines were scanned for hex colours, native `confirm`/`alert`/`prompt`, raw `e.message`/`String(e)` in UI copy, and new `.primary` buttons. None were found.
  - The changed covered-area files were read:
    - `LoadState`.
    - Product: AnalysisTab, DiscoveryChat, RefineChat, PublishDialog.
    - Design Hall: Site `Inspector`, OttoPanel, VariantsTray.
    - Rooms: RoomLobby, RoomPage, RoomChat, RoomHost, RoomRecap, RecapImage.
    - MCP: OttoServerHome, `sessionAttach.ts`, PolicyForm, ToolsTab.
  - Most of these are the a11y announce/focus sweep and keep their visible copy and states. The lens-relevant changes:
    - RoomLobby: the open-window failure became an action toast with "Open again". That is fine: it is an action, and the room list stays as loaded.
    - RoomPage: one banner for the whole outage. Its "Reconnect" button stays mounted while connecting. Fine.
    - RoomChat: "Resend message". RoomRecap: "Resume speech capture". Both are clearer copy.
    - PolicyForm and ToolsTab: JSON errors use `loadErrorText`. Fine.
    - RecapImage and RoomRecap: moved onto `LoadState`. See 6w3-01.

## Areas NOT covered yet

These are unchanged from wave 1 and were deliberately not widened:

- Game HUD and menus at phone width, which needs screenshots.
- Swarm and Loops detail flows.
- Assistant, Snip editor and Proof flows, and the tray pages.
- Canvas and the Workflows inspector.
- The remaining Settings pages.
- Phone and tablet layouts.
- The other MCP tabs (ProfilesTab, AuditTab, ExposePanel, TokensPanel). PolicyForm and ToolsTab were read only for their wave-3 diff lines.

---

## Findings

### ux-design-6w3-01: nit. Rooms recap: the new `LoadState` wrappers doubled or changed the error copy (a fix-introduced regression)

- **Files:**
  - `ui/src/modules/rooms/RecapImage.svelte:12`, `:15` (with `ui/src/modules/rooms/recap-client.ts:21`)
  - `ui/src/modules/rooms/RoomRecap.svelte:54`, `:106`, `:111`
- **Evidence:**
  - **RecapImage: doubled headline.**
    - Wave 3 replaced the fixed line "Couldn’t load this screen sample. Try again." with `error = loadErrorText(e)` (`:12`). That value is rendered in `<LoadState what="this screen sample" … {error} empty onretry={load} />` (`:15`).
    - `recapBlob` throws `new Error('Couldn’t load the recap file. Try again.')` on any non-OK response (`recap-client.ts:21`). `loadErrorText` passes that message through unchanged (`lib/loadError.ts:28`).
    - On any HTTP failure, the compact `LoadState` therefore reads "Couldn’t load this screen sample. Couldn’t load the recap file. Try again." [Retry]. That is two headlines for one failure, plus a "Try again." sentence next to the Retry button.
    - `lib/loadError.ts:1-3` (content.md §4) defines the detail line as "the cause, in the user's terms", not a second headline.
  - **RoomRecap: two wordings for one failure.**
    - The engine-capability check failure (`:54`, `fail('Couldn’t check the local recap engines.', …, true)`) is now shown in the prepare modal through `<LoadState what="the local recap engines" …>` (`:111`). That reads "Couldn’t load the local recap engines. {detail}".
    - Nothing loads the engines; the check of whether they exist failed. A reader can take "couldn’t load the engines" to mean the engines themselves are broken.
    - The same failure, seen after the modal is closed, still reads "Couldn’t check the local recap engines." in the banner (`:106`).
- **Fix:**
  - `recap-client.ts:21`: throw a cause, not a headline. For example, ``throw new Error(`The daemon answered ${response.status}.`)``, or a `StatusError`-shaped object (`{status, message}`) so `loadErrorText` maps 401, 403 and 404 to its standard lines. Drop the trailing "Try again." The same applies to the `recapRequest` fallback at `:13` ("The recap request failed. Try again."), which feeds the same `LoadState`.
  - `RoomRecap.svelte:111`: use a noun that matches the action, such as `what="the recap engine status"`, which reads "Couldn’t load the recap engine status." Or keep "check" by giving `LoadState` the error through an `errorActions`/custom headline. Then make the `:54` `what` string match the modal, so both surfaces use the same wording.
