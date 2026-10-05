## Lens: State design (loading, empty, error, stale, offline, partial, in-flight, failure-as-empty, flicker)
**Score: 8.1/10.** Arithmetic: 10.0 − 3 major (0.9) − 9 minor (0.9) − 2 nit (0.06) = 8.14. Failure-as-empty, hand-rolled loading and in-flight gaps keep it below 9.8, even though the shared `LoadState` machinery is strong.

Verified and credited (no regressions found):
- `ui/src/lib/components/LoadState.svelte` is a correct four-state wrapper. It has a 150 ms skeleton grace with reserved height, an error-with-nothing-to-show state with Retry, and a stale bar that keeps the data.
- `mcp/TokensPanel.svelte:37-50` keeps a separate `tokensError`, so a failed load never shows "No MCP tokens yet".
- `home/HomeBox.svelte:21-36` drops the in-flight promise on failure, so Retry re-imports the chunk.
- `shell/StatusBar.svelte:14-66`, `TabBar.svelte:104` and `Navigator.svelte:73` surface "Reconnecting…" and pass the stale flag, which matches patterns.md §1.
- `AttachProductStory.svelte:23-25` and `SkillPicker.svelte:68-72` explicitly fixed failure-as-empty.
- The Settings async buttons (Channels, IssueAccounts, AssistantSettings) have busy labels.

### Findings

1. `[major] Failed fetch shown as an empty state in Linked Canvases` — `modules/product/LinkedCanvases.svelte:25-34, 171-174`
   - The `catch` sets `scenes = []`, so the UI says "No canvases linked yet. Create one to design this story visually." when the request failed. This breaks "failed load shows inline with Retry".
   - Fix: add `loadError` and render `<LoadState what="linked canvases" variant="compact" …>`.

2. `[major] Bulk swarm task ops swallow per-item failures` — `lib/stores/swarm.svelte.ts:589-605`
   - `bulkUpdateTasks` and `bulkDeleteTasks` wrap each call in `.catch(() => null)`, then reload and return normally. If some patches or deletes fail, the board silently reverts or partially applies, with no toast and no count.
   - Fix: use `Promise.allSettled`, count the failures, and have callers `toastError('Couldn’t move 2 of 5 tasks', …)`.

3. `[major] Brokers tabs replace the whole list with "Loading…" on every reload (flicker, scroll and selection loss)` — `brokers/TopicsTab.svelte:323-324` with `loading = true` at line 74; `brokers/GroupsTab.svelte:301-302` with `loading = true` at line 74, and `GroupsTab.svelte:334-335` (`detailLoading` replaces the open group detail).
   - The `{:else if loading}` branch comes before the data branch, so refresh, poll and manual reload blank the table. `LoadState` is used only for the error case.
   - Fix: wrap the list in `<LoadState … loading error empty>` so a refresh keeps the rows and only a first load shows the skeleton. Show the loading text only when `topics.length === 0`.

4. `[minor] Copy sweep regression: straight apostrophe "Couldn't" remains (about 217 hits in 99 files)` — including the shared component `lib/components/LoadState.svelte:64,70`, plus `ProofPage.svelte` (15), `ScheduledTasksPage.svelte` (9), `SwarmPage.svelte` (11), `KanbanBoard.svelte` (8), `WorkbenchPage.svelte` (8), `McpServers.svelte`, `Channels.svelte`, `TopicDetail.svelte`.
   - The curly "Couldn’t" is the sweep's standard. The shared component emits the inconsistent form in every load-error block.
   - Fix: replace `Couldn't` with `Couldn’t` (and `"Couldn't …"` string literals). Run `grep -rn "Couldn't" ui/src`. Add an `ui-guards` rule so it can't return.

5. `[minor] About 40 hand-rolled plain-text "Loading…" lines instead of Skeleton or LoadState (no grace, no reserved height)`
   - `brokers/TopicsTab.svelte:324`, `GroupsTab.svelte:302,335`, `TopicDetail.svelte:853,885`, `SchemaTab.svelte:88`, `SchemaVersionsPanel.svelte:124,161`, `LagAlertsPanel.svelte:100`, `mcp/ToolsTab.svelte:197`
   - `kubernetes/PodHttpPanel.svelte:219`, `panels/CanvasPanel.svelte:259,350`, `panels/OutputsPanel.svelte:286`, `panels/FileTree.svelte:332`, `agents/AttachProductStory.svelte:84`, `swarm/StoryLinkCard.svelte:72`
   - +about 25 more: `grep -rnE ">\s*Loading[^<{]*…?\s*</" ui/src/modules`
   - These flash on fast loads, and their height differs from the loaded layout (layout shift). The AWS views already show the intended pattern: a status line plus `<Skeleton>`.
   - Fix: route them through `LoadState`, or at least `Skeleton`.

6. `[minor] Failed lookups in the Design Hall site Inspector read as "none exist"` — `design-hall/site/Inspector.svelte:175-188, 222-238`
   - `pickArtifacts` failure sets `list = []`. The menu then says "No 3D artifacts in this workspace yet" or "No images in this workspace yet". `linkKit` shows an empty kit list in the same way.
   - Fix: on `catch`, show a disabled "Couldn’t load artifacts — try again" item, or call `toastError`.

7. `[minor] Skill options silently empty on partial failure` — `skills-lab/SkillReviewPanel.swift`-style site: `skills-lab/SkillReviewPanel.svelte:90-105`
   - Each of the three lists uses `.catch(() => [])`, and the outer `catch` sets `skillOpts = []`. A dead library endpoint shows only bundled and provider skills with no hint that the library is missing.
   - Fix: use `allSettled` and show a "Library skills didn’t load · Retry" note.

8. `[minor] CanvasPanel hides good data when a refresh fails, and uses a bespoke error block` — `panels/CanvasPanel.svelte:258-267`
   - The `loadError` branch comes before `refs.length`, so a failed refresh removes the list that was already loaded. The error block is custom, with no stale-bar semantics.
   - Fix: use `LoadState` (error plus data gives the stale bar). Other bespoke error blocks that bypass it: `panels/ActivityPanel.svelte:192`, `panels/OutputsPanel.svelte:289,349`, `agents/AttachProductStory.svelte:85-89`.

9. `[minor] "Create topic" has no in-flight state` — `brokers/TopicsTab.svelte:220-250, 306-311`
   - The button is disabled only on an empty name, with no busy flag or "Creating…" label. Double-clicking, or waiting on the guarded-cluster confirm, can send two creates.
   - Fix: add `creatingBusy`, set `disabled` and label it "Creating…".

10. `[minor] Destructive icon buttons stay enabled while the delete or remove request is in flight`
    - `settings/SessionNames.svelte:88-103` (button at 165), `settings/Channels.svelte:246-258` (button at 436), `settings/IssueAccounts.svelte:259` (the Test button next to it does have a busy state).
    - `Providers.svelte:608` correctly uses `disabled={saving}`, so the Settings screens are inconsistent with each other.
    - Fix: add a per-row busy id and disable the button while it is set, as `McpServers.svelte` does with `busyId`.

11. `[minor] Stale background refresh is not signalled in the Git graph` — `git/GraphView.svelte:682-695, 706-712, 996-999`
    - Failures from refs, stashes, worktrees and submodules are swallowed with `.catch(() => {})` or `.catch(() => null)`. The graph then looks current when it may not be. This is intentional ("quietly"), but nothing marks "Last refresh failed".
    - Fix: set a lightweight `refreshStale` flag and show the existing slim stale bar with Retry, only if the failure repeats.

12. `[minor] Command-palette data failures silently drop commands` — `lib/components/FloatingBar.svelte:155-165`
    - Workspaces, sessions and repos failures become `[]`. "Focus session" and "Open repo" commands disappear without a hint.
    - Fix: add a single "Couldn’t load sessions" disabled row, or a footer line.

13. `[nit] No in-flight state when saving the snip shortcut` — `settings/SnipSettings.svelte:43-53, 131-136`
    - The buttons stay clickable during the Tauri `invoke`. Errors do show inline via `saveError`.

14. `[nit] Silent failures where the user may expect a list` — `mcp/TokensPanel.svelte:52-58` (owner select empty when `/users` fails), `vault/vault.svelte.ts:798-803` (backlinks fail to `[]`).
    - I did not check whether the backlinks UI says "No backlinks". Confirm before fixing.

### What would get this lens to 9.8
- Make every load path go through `LoadState`: finding 3 (Brokers), finding 5 (about 40 text-only loaders) and finding 8 (bespoke error blocks). Then add an ui-guards ratchet for `>Loading…<` and for a bare `{:else if loading}` placed before the data branch.
- Remove failure-as-empty in findings 1, 6, 7 and 12. Replace `catch { x = [] }` with an error flag plus an inline message or Retry, and add an `ui-guards` pattern for `catch … = []` where the result feeds an empty message.
- Make partial-failure results visible with `Promise.allSettled` and a count toast, as in finding 2.
- Add a busy flag to every mutating button: finding 9 (create topic), finding 10 (delete and remove icon buttons) and finding 13 (snip shortcut).
- Finish the `Couldn’t` sweep, starting with `LoadState.svelte` and adding a guard (finding 4).
- Signal stale background refreshes (Git graph refs, stashes) instead of leaving them silent (finding 11).

Key file: `/Users/itziklavon/claude_ade-design/ui/src/lib/components/LoadState.svelte`
