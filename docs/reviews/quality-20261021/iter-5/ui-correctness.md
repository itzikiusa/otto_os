# UI correctness review: iteration 5

Lens: logic bugs in the Svelte 5 UI (`ui/src/**`). This covers runes and reactivity, races between async loads and navigation, state left broken after errors, and API usage that does not match `ui/src/lib/api/types.ts` and the Rust producers.

Branch `fix/quality-iter5-20261022` @ `8177c4d85`. The review was read-only: no builds or tests were run. The room games and the classroom are in scope this time.

Svelte version check (`ui/node_modules/svelte` 5.57.1):
- `each.js:355-361` throws `each_key_duplicate` in production builds as well as in DEV. The app has no `<svelte:boundary>` (`rg` finds none).
- `runtime.js:455-488` (`update_effect`) runs the previous teardown with `active_reaction = null` (`effects.js:445-463`). It then re-runs the body tracked. Any `$state` read in a helper called synchronously from the body is a dependency.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 3 | 0.30 |
| nit | 1 | 0.03 |

**Score = 10.0 - 0.30 - 0.30 - 0.03 = 9.37 / 10**

## Previous-finding status (iteration 4)

All 7 iteration-4 findings are fixed in the current code.

| ID | Status | Evidence |
|---|---|---|
| ui-correctness-4-01 | Fixed | `RefineChat.svelte:71` `untrack(() => void loadThread(currentTid))`; `DiscoveryChat.svelte:91` `untrack(() => void loadChat(currentCid))`. |
| ui-correctness-4-02 | Fixed | `GraphView.svelte:362-376` records `newRefTips` (keys of `next` absent from `prev`). `:1043` reloads in full when `outside(newRefTips)`. `:1052-1053` clears both sets. |
| ui-correctness-4-03 | Fixed | `issueAccounts.svelte.ts:41-48` keys the cache by `auth.me?.id` (`owner`): an identity change resets it, and a load started under the old owner never fills the new cache (`inflight === run` guards). |
| ui-correctness-4-04 | Fixed | `proof.svelte.ts:224-229` adds `wantedId`. `refreshDetail` (`:297-300`) and the trailing timer (`:287`) refresh `wantedId ?? detail`. `closeDetail` clears it (`:327`). |
| ui-correctness-4-05 | Fixed | `apiClient.svelte.ts:234`: a delta poll that fails is retried with doubling backoff. `:1982` reports giving up as "lost track", never as "Last run failed". |
| ui-correctness-4-06 | Fixed | `AuditTab.svelte:79-81`: `load()` clears `loadingMore`. |
| ui-correctness-4-07 | Fixed | `PrDetail.svelte:547` `as reviewer, i (i)`; `:568` `as name, i (i)`. |

Counts: 7 fixed, 0 still open, 0 rejected as invalid.

## Areas covered

- **Re-verification** of all 7 iteration-4 fixes (table above). This includes the `issueAccounts` identity keying, which led to finding 5-02.
- **New since iteration 4.**
  - **MCP profiles.** I traced end to end, against `crates/otto-sessions/src/http.rs:541-562` (null clears the key):
    - `mcp-profiles.svelte.ts`, `ProfilesTab`, `ProfileEditor` (the snapshot fix `2555fed77`), `McpProfilePicker`, `SessionMcpProfiles` and `lib/mcpProfiles.ts`. `profileTools` de-duplicates.
    - `NewSession` passes `meta.mcp_profiles`.
    - The session `meta` patch.
    - **Finding 5-05.**
  - **Shared Confluence pickers.**
    - `ConfluencePagePicker`: the scope effect, debounce, abort and `seq`.
    - `ConfluenceSpaceSelect`, plus their hosts in TestCasesTab, PublishDialog and the workflow `ProductPublish` inspector.
    - No defect confirmed.
  - **Database store split.** `database/dashboards.svelte.ts` against the pre-split code at `6cff08270`: scope/epoch guards, in-flight sharing and widget create/update/delete. It is a faithful move, with no defect.
  - **Classroom (`ClassroomsBox` and `classrooms/*`):**
    - Live poll lifecycle.
    - Scene mount and context-loss remount budget.
    - Hold/unhold around kick-out and detention.
    - Keyed blocks (all by `k.id` / `r.id` / index).
    - The screen poller.
    - No correctness defect. The poll restart on every school recompute is a perf matter for the perf lens.
  - **Room games glue:**
    - `GameMatch.svelte`: round/generation resets and the host/guest seq filters. Rematch goes `Finished → Lobby (round+1) → Playing` (`game_rooms/registry.rs:493-507`), so `beginRound` fires.
    - `KartGame.svelte`: `start`/`stop` and guest interpolation. `ShooterGame` lifecycle hooks.
    - No defect confirmed.
- **Iteration-4 "not covered" list:**
  - **Swarm `KanbanBoard`:**
    - Drag/drop.
    - Bulk partial failure keeps the failed rows selected.
    - Plan/stop.
    - `swarm.updateTask`/`plan`: `plan` returns the full project list (`engine.rs:3334`).
    - No defect.
  - **MCP:**
    - `ApprovalsTab` (liveQuery, seq guard, canDecide). No defect.
    - `ToolsTab`. Every instance is per server row (`ServersTab.svelte:233`), so the server-switch path is unused.
  - **Product:**
    - TestCasesTab, RewriteTab and PlanTab polling. **Finding 5-01.**
    - DesignArena loads. **Finding 5-03.**
    - MockupAnnotations: it remounts per attachment, because MockupViewer toggles `rendering`.
    - PublishDialog loads. **Finding 5-02.**
  - **`AgentPage` / `PersonalAgentsPage`:**
    - Keyed by `agentId` (`PersonalAgentsPage.svelte:151`).
    - Chat get-or-create single-flight.
    - Runs reload.
    - The report `latestOnly` guard.
    - No defect.
  - **Module caches across an identity change.** `pr-cache.ts` holds no viewer-specific fields. GraphView is keyed per repo in both hosts (`GitPage.svelte:380`, `GitPanel.svelte:55`), so the cross-repo `lostRef` path cannot occur.
  - **Content-keyed `{#each}` blocks:**
    - `MemoryTab` `m.tags`, `NeedsYouCard` `ny.options` and `ToolStep` `image_ids`. **Finding 5-04.**
    - `TurnItem` `parts.images`: composer uploads get a ULID-prefixed path (`routes/transcript.rs:921`), so this is not reported.
- **Sweep:** every component that keeps a poller or timer handle in `$state` (`rg '$state(.raw)?<Poller|ReturnType<typeof set…> | null>'`). Only the three Product tabs do, and finding 5-01 covers them. AnalysisTab, RefineDrawer, DocsAgentsView, AthenaView and LogsView keep plain variables.

## Areas not yet covered (next iteration)

- `Terminal.svelte` `switchEngine` / `adoptEngine` / `parkEngine` (`:2243-2430`) and `termGridSync.ts`. I read them but did not trace them against `termPark.ts` eviction.
- `SwarmPage.svelte` beyond the settings modal, and `RunInspector` live updates.
- MCP `PoliciesTab` / `PolicyForm` / `AutoApprovePanel` / `ExposePanel` / `TokensPanel`.
- Shooter sim/net (`shooter/net.ts` decode with loot/builds deltas) and kart `net.ts` snapshot decoding versus the encoder.
- `home/school/scene*` (`mountSchool`, `pick`, `camera`): the WebGL side of the classroom.
- `lib/stores/database.svelte.ts` façade delegation beyond dashboards (`model.ts`, `statement.ts` moves).
- `MockupViewer` used outside DesignArena with `source === null` (the html branch has no `gen` check after `fetchText`).

## Findings

### ui-correctness-5-01: major: Product Rewrite, Plan and Test cases stop polling the moment Generate starts it, re-enable Generate, and wipe the tab's state
- **File:**
  - `ui/src/modules/product/TestCasesTab.svelte:179` (`let pollTimer = $state.raw<Poller | null>(null)`), `:191-196` (`clearPoll` reads `pollTimer`), `:231-241` (`startPolling` writes it), `:258-268` (reset effect), `:511-523` (`generate`), `:642` (button)
  - `ui/src/modules/product/RewriteTab.svelte:41`, `:55-60`, `:100-111`, `:114-125`, `:297-318`
  - `ui/src/modules/product/PlanTab.svelte:106`, `:118-123`, `:196-205`, `:208-217`, `:294`, `:465-519`, `:598-601`
- **Evidence:**
  - Each tab's "reset on story change" effect calls `clearPoll()` synchronously in its body. `clearPoll` reads `pollTimer !== null`. Because `pollTimer` is component `$state.raw`, the read makes it a dependency of the reset effect, besides the intended `product.selectedId`.
  - `generate()` then calls `startPolling()`, which assigns `pollTimer = liveQuery(...)`. That write re-runs the reset effect, and Svelte first runs the previous teardown `() => { clearPoll(); }` (`effects.js:445-463`). That teardown stops the poller that was just started and sets `pollTimer = null`.
  - The body then resets the tab:
    - **TestCasesTab:** `activeRunId = null`, `caseActions = {}` (any case being edited), `showPublishForm = false`, `selected = new Set()`, `orderedIds = []`, then `loadRuns()`.
    - **RewriteTab:** `sourceVersion = suggestedVersion = null`, then `initialLoad()`.
    - **PlanTab:** `planVersion = null`, `body = ''`, `planSessionIds = []`, then `initialLoad()`.
  - The `pollTimer = null` written in the teardown re-dirties the effect once more, so the reset and reload happen twice.
  - Every "waiting" affordance is gated on `pollTimer !== null`:
    - The Generate / Regenerate buttons are `disabled={generating || pollTimer !== null}`.
    - The labels "Polling for result…" and "Generating…".
    - "Stop waiting" and "checking every 3s…".
    - The failed-read notice.
    - The 2-minute timeout in `pollTestcases` / `pollVersions` / `pollForPlan`.
  - None of these ever appear, and the timeout never fires.
  - AnalysisTab (`:102`) keeps its poller in a plain `let`, and it does not have this problem. The handle moved to `$state.raw` in `c369a8ec6` so the template could read it. Before that it was `$state`, which behaves the same way.
- **Failure:**
  1. In Product → Rewrite (or Plan, or Test cases), click Generate.
  2. The toast says "Waiting for a new run to appear…", but the button turns straight back to an enabled "Generate suggested rewrite" / "Regenerate…" / "Generate". A second click starts a second agent run, or a second set of planner agents.
  3. The shown plan or rewrite blanks and reloads.
  4. On Test cases, an open case edit, the bulk selection, a local reorder and an open publish form are discarded.
  5. The result appears only if a `product_changed` WS event arrives. With the event socket down, the generated run, plan or rewrite never shows up until the user reloads, and no timeout tells them.
- **Fix:**
  - Do not let the reset effect subscribe to the poller. Either `untrack(clearPoll)` in the body (the cleanup is already untracked), or keep the handle in a plain `let poller` with a separate `let polling = $state(false)` for the template, as AnalysisTab does.
  - Apply this in all three tabs.
  - Add a unit test that calls `generate()` and asserts that the poller is still running and the button is disabled.

### ui-correctness-5-02: minor: Opening Publish with a cold account cache loads everything twice, and the content preview flashes back to loading
- **File:**
  - `ui/src/lib/stores/issueAccounts.svelte.ts:41-48` (`load` reads `auth.me?.id` and `this.loaded` synchronously)
  - `ui/src/modules/product/PublishDialog.svelte:145-148` (mount effect), `:150-163` (`loadAccounts`), `:105-123` (`loadPreview`)
  - The same untracked-loader shape: `product/ImportDialog.svelte:54-56`, `agents/JiraIssuePicker.svelte:53-55`
- **Evidence:**
  - The iteration-4 fix for 4-03 made `issueAccounts.load()` read `auth.me?.id` and then `this.loaded` (both `$state`) before its first await.
  - PublishDialog's mount effect calls `void loadAccounts(); void loadPreview(product.selectedId);` with no `untrack`. The effect therefore subscribes to `issueAccounts.loaded` and `auth.me`.
  - On the first open of a page life the cache is cold. When the fetch resolves, `this.loaded = true` (`:52`) re-runs the effect:
    - `loadAccounts` runs again. It overwrites `accountId`, and `loadProjects` / `loadSpaces` reset `projectKey`/`spaceKey` and fetch again.
    - `loadPreview` runs again. It sets `previewBody = null; reviewedContent = null` and re-fetches the story, its versions and possibly a version body. While it does, Publish is disabled (`:244`).
  - ImportDialog and JiraIssuePicker re-run too, but their second load is a cache hit.
- **Failure:**
  1. First Publish-as-story / Publish-as-RFC of the session (no Import or Jira picker opened earlier).
  2. The preview shows, then drops back to its loading state and reloads.
  3. The tracker projects/spaces are requested twice (a rate-limited Jira call), and the account choice is reapplied.
- **Fix:** Make `load()` read its gate untracked:
  ```ts
  const who = untrack(() => auth.me?.id ?? null);
  // and test `untrack(() => this.loaded)`
  ```
  Or wrap the three mount effects' loader calls in `untrack`, keeping `product.selectedId` as PublishDialog's only dependency.

### ui-correctness-5-03: minor: In the Design arena, a story switch can leave the previous story's artifacts listed (and editable) under the new story
- **File:** `ui/src/modules/product/design/DesignArena.svelte`
  - `:265-280` (story effect)
  - `:302-322` (`loadAll`)
  - `:104` (`ownAtts`)
  - `:114-123` (`rows` labels every own artifact with the current `story`)
  - `ui/src/lib/stores/product.svelte.ts:846-849` (`listAttachments` reads `storyId()` with no ownership guard)
- **Evidence:**
  - `MockupsTab` mounts one unkeyed `DesignArena` (`ProductPage.svelte:931`). A story switch re-runs `loadAll()`.
  - `loadAll` assigns `ownAtts = await product.listAttachments()` with no request token and no `product.selectedId` check. The child loads are guarded (`childSeq`), but this one is not.
  - When story A's list response lands after story B's, `ownAtts` ends up as A's attachments. `rows` presents them with `owner: story` (B), and `selectedId` defaults to one of them.
  - The switch does not clear `ownAtts`/`selectedId` either. Until B's list lands, A's open artifact stays on the canvas, attributed to B.
  - A's `finally { loading = false }` also clears B's spinner early.
- **Failure:**
  1. Switch quickly between two stories in Product → Design (or switch while the first list is slow).
  2. The second story's arena lists the first story's mockups.
  3. Edits autosave, and Delete/Refine act on those attachment ids, so changes land on the other story's artifacts.
- **Fix:** Capture the story at the start (`const owned = product.captureSelection()`, or `const sid = product.selectedId` plus a `++loadSeq` token). Assign `ownAtts`, `selectedId` and `loading` only while it is still current. Clear `ownAtts`, `childAtts` and `selectedId` in the story effect before `loadAll()`.

### ui-correctness-5-04: minor: Duplicate model- or content-supplied values in three keyed lists crash the conversation, Assistant card or memory list
- **File:**
  - `ui/src/modules/agents/conversation/ToolStep.svelte:307`: `{#each result.image_ids as id (id)}`
    - Producer: `crates/otto-transcript/src/claude.rs:367-372` pushes one id per image block.
    - The id is the SHA-1 of the base64 data (`images.rs:34-38`, `:54-55`). No de-duplication.
  - `ui/src/modules/assistant/cards/NeedsYouCard.svelte:156`: `{#each ny.options as o (o)}`
    - Producer: `crates/otto-assistant/src/assistant/tasks.rs:249-255`. The agent's `options` are taken verbatim (`take(8)`, no de-duplication or trim).
  - `ui/src/modules/assistant/MemoryTab.svelte:281`: `{#each m.tags as t (t)}`
    - Producer: `crates/otto-assistant/src/assistant/memory.rs:122` (`tags.into_iter().take(20)`, from the agent's `remember` call). No de-duplication.
- **Evidence:**
  - All three keys are values the server does not keep unique.
  - A tool result that carries the same image twice gives two identical content-hash ids. Examples: a screenshot tool returning an unchanged screen twice, or a multi-page render with two blank pages.
  - An agent asking "Proceed?" with `["Yes","Yes"]`, or tagging a memory `["db","db"]`, does the same.
  - Svelte 5.57 throws `each_key_duplicate` in production. With no `<svelte:boundary>`, the subtree stops rendering.
- **Failure:**
  - The session's conversation view breaks on that tool step.
  - The Assistant's needs-you card cannot be answered.
  - The Memory tab stops rendering.
- **Fix:**
  - Key by index (`as id, i (i)`, `as o, i (i)`, `as t, i (i)`), as `PrDetail` now does.
  - Optionally also de-duplicate at the producers (`dedup` the options and tags; keep image order but key by position).

### ui-correctness-5-05: nit: MCP → Profiles fetches the profile list twice on first open
- **File:**
  - `ui/src/modules/mcp/ProfilesTab.svelte:45-47`
  - `ui/src/lib/stores/mcp-profiles.svelte.ts:20-21`
- **Evidence:**
  - `load(true)` evaluates `this.loaded && !force`, which reads `loaded` (`$state`) before `!force` short-circuits it. The tab's mount effect, which has no `untrack`, therefore subscribes to `loaded`.
  - On a cold store, the first response flips `loaded` from false to true, the effect re-runs, and a second `GET /mcp/profiles` follows.
  - `McpProfilePicker` (`:22-24`) has the same subscription, but its re-run is a cache hit.
- **Failure:** The first visit to MCP → Profiles issues two identical profile requests. There is no visible flicker, because LoadState shows loading only while `!loaded`.
- **Fix:** Use `if (!force && this.loaded)` (or `untrack`) in `load`, or `untrack(() => void mcpProfiles.load(true))` in the tab.

## Rejected candidates (checked and dropped)

- **TurnItem `parts.images` duplicate keys:** composer attachments get unique ULID-prefixed paths (`routes/transcript.rs:921`). A duplicate needs the user to hand-type the same `[Image: …]` line twice.
- **ToolsTab tester result surviving a server switch:** every ToolsTab instance is embedded per server row (`ServersTab.svelte:233`), so `selectedServerId` never changes within an instance.
- **ConfluencePagePicker scope effect subscribing to the host's state through `onchange`:** this happens only on an account change with a parent already picked. The only effect of the extra re-run is clearing the picker's unsent query.
- **GraphView `setRefs` comparing two repos' refs on a repo switch:** GraphView is remounted per repo in both hosts (`GitPage.svelte:380`, `GitPanel.svelte:55`).
- **Classroom screen poll restarting on every session status event:** this costs an aborted batch plus an immediate re-fetch. It is a perf matter, not wrong output.
- **GameMatch round start after rematch:** the server goes through Lobby (`registry.rs:502-504`), so `beginRound` runs.
- **TestCasesTab "default space" leaving the parent picker unusable:** the user can pick the space explicitly. It is a capability question, not a logic bug.
