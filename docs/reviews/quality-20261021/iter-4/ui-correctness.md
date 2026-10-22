# UI correctness review: iteration 4

Lens: logic bugs in the Svelte 5 UI (`ui/src/**`). This covers runes and reactivity, races between async loads and navigation, state left broken after errors, and API usage that does not match `ui/src/lib/api/types.ts` and the Rust producers.

Branch `fix/quality-loop-20261021` @ `6cff08270`. The review was read-only: no builds or tests were run. Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`, `ui/src/modules/home/boxes/ClassroomsBox.svelte`.

Svelte version check (`ui/node_modules/svelte`): `each.js:356-361` throws `each_key_duplicate` in production builds as well as in DEV. A duplicate key in a keyed `{#each}` therefore breaks the whole rendered subtree; the app has no `<svelte:boundary>`.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 7 | 0.70 |
| nit | 0 | 0.00 |

**Score = 10.0 - 0.70 = 9.30 / 10**

## Previous-finding status (iteration 3)

All 7 iteration-3 findings are fixed in the current code.

| ID | Status | Evidence |
|---|---|---|
| ui-correctness-3-01 | Fixed | `sql-builder.ts:550-558`: `suggestGroupBy` returns `uniqueRefs(...)`. `QueryBuilder.svelte:1163` keys the chips by index (`as r, i (i)`), `:1259` keys the issues by index, and "Group by them" goes through `mergeGroupBy` (`:1265`). |
| ui-correctness-3-02 | Fixed | `ArtifactCard.svelte:59` keys by index (`as key, i (i)`). `ReferencesPanel.svelte:194` de-duplicates (`[...new Set(h.story_ids)]`) and keys by index. FIXES.md records why the server half (`DISTINCT` / `rel='implements'`) was rejected: the rows are already de-duplicated, and the story filter matches any relation. |
| ui-correctness-3-03 | Fixed | `crates/otto-k8s/src/resources.rs:314-325` de-duplicates in order with a `HashSet` seen-filter. `ResourceDrawer.svelte:319` keys by index. |
| ui-correctness-3-04 | Fixed | `EditFlow.svelte.ts:708-716` adds `clearCoveredDrafts()`, which runs before the refresh at `:750`. It drops only the rows whose patch is the one the review was built from. |
| ui-correctness-3-05 | Fixed | `WorkflowsPage.svelte:382`: `renderMarkdownGfm(md, { breaks: true })`. |
| ui-correctness-3-06 | Fixed | `workspace.svelte.ts:621`: `workflowRuns.clear(id)` in the switch reset. `workflowRuns.svelte.ts:44` drops a response whose id is no longer the target. |
| ui-correctness-3-07 | Fixed | `library.svelte.ts:273`: `this.stories = { ...this.stories, ...next }` merges into the live map. |

Counts: 7 fixed, 0 still open, 0 rejected as invalid.

## Areas covered

- **Iteration-3 refactor regressions (`eec25c214`).** I compared the extractions against the code they replaced:
  - **Jira issue editor** (`product/jira/issueEditorState.svelte.ts`, `IssueEditor.svelte`) and its host in `OverviewTab`. I checked the `isCurrent` guards after every await, `reset()` on a story switch, and the keyed tab. No defect.
  - **`lib/api/issues.ts`.** The assignee payload is unchanged from before the split. The **`issueAccounts` store is new. Finding 4-03.**
  - **Workflows inspector split** (`inspector/*`):
    - `OptionCache` try-once semantics, `retry`, `IdPicker`, `RepoPicker` and `SwarmTask` effects.
    - `NodeParams` prop wiring to `onParam` / `setParams`.
    - `graph` is a deep `$state`, so a param mutation re-renders.
    - No defect.
  - **Shell split:** `installShell.svelte.ts`, `paletteSources.svelte.ts` (each source in its own effect inside `$effect.root`) and `closeConfirm.ts`. Also the Navigator bulk archive/delete paths. No defect.
  - **SQS attribute fan-out** moved into the aws store: `live()` guard, access revision, batched commits. No defect.
  - **Workspace store:** unknown-chunk reconcile (`kept` / `reconcileTabs(unknown)`), the removed `api*` getters (no callers left), and the event-bus field removals. No defect.
- **Iteration-3 "not covered" list:**
  - **Terminal reconnect ladder:**
    - `scheduleReconnect` and `reconnectDelay` (the guest fast→slow→stop ladder).
    - `connect()`: single-socket enforcement, handler detach, and the state reset.
    - `wireSocket` onopen resets.
    - Frame switch: status, exit-probe and the snapshot rule.
    - No defect.
  - **GraphView splice:** `fetchGraph` / `applyGraph` / `spliceHistory` / `lostRef`, and the FIRST_PAGE vs PAGE prefetch. **Finding 4-02.**
  - **Product:**
    - OverviewTab. No defect.
    - RefineChat and DiscoveryChat. **Finding 4-01.**
    - RefineTab and ChatTab mounting.
  - **ScheduledTasksPage:**
    - Form generation and `ownsForm`.
    - The save outward confirm.
    - Workflow picker reset per form open.
    - The workspace effect (untracked loaders).
    - No defect.
  - **ProofPage and the proof store:** list/more sequences, open/refresh, lazy media and its revocation. **Finding 4-04.**
  - **HistoryPage and the history store:**
    - Rescan: `beginHistoryRescan` resets `done` synchronously, so the effect cannot fire early.
    - Resume ownership guard.
    - `loadMore` resets in `load`.
    - Disk-artifact loads.
    - No defect.
  - **`apiClient.svelte.ts`:** automation run loop, `noteRunProgress` latch and wake, `waitRunProgress`. **Finding 4-05.**
  - **Swarm:** RunInspector result fetch (abort per id/status/attempt), SwarmSettings goals/triggers, the SwarmPage modal mounting. No defect confirmed.
  - **MCP AuditTab:** load / load-more generations. **Finding 4-06.**
  - **Personal agents RoomsView and its store:**
    - `loadRooms`: `roomsWs` is a plain field, so it is not a tracked read.
    - `loadMessages` / `loadOlder` request guards.
    - Render window and per-room drafts.
    - No defect.
  - **Forge-fed keyed `{#each}` blocks:**
    - `PrDetail` reviewers and approvals. **Finding 4-07.**
    - GitHub reviewers are de-duplicated by login at the source (`github.rs:803`).
- **Loading-flag sweep:** every `if (<generation matches>) loadingMore = false` site (`rg`):
  - PrList resets `loadingMore` in its load effect.
  - The history store resets in `load`.
  - The proof store resets in `loadPacks`.
  - AuditTab does not (4-06).
- **Async-loader-in-`$effect` sweep:** I listed the 60 effects that call a loader without `untrack` and inspected the ones whose loader reads component state before its first await. RefineChat and DiscoveryChat are the confirmed case (4-01).

## Areas not yet covered (next iteration)

- `Terminal.svelte` `switchEngine` / TermPark adopt (`:2403-2520`) and the new `termGridSync.ts` (moved in iteration 3). I read the reconnect half only.
- `GraphView` lane layout memo (`graph-lanes.ts` `LaneMemo`) under splice, and `graph-cache.ts` snapshot restore on a repo switch.
- Swarm `KanbanBoard` drag/drop and optimistic moves. `SwarmPage` (1246 lines) beyond the settings modal.
- MCP `PoliciesTab`, `ApprovalsTab`, `ToolsTab`.
- Product `DesignArena`, `RewriteTab`, `TestCasesTab` (1549 lines), `PublishDialog` preview loads, `MockupAnnotations`.
- `AgentPage` / `PersonalAgentsPage` (19 and 18 changes in 3 weeks).
- Module-level caches that may outlive an identity change (the class behind 4-03): `prListCache`, `graphCache`, `pr-cache.ts`, `graph-diff-cache.ts`, and the swarm and personal-agents singletons.
- The remaining string-keyed `{#each}` blocks fed by external data: `MemoryTab` `m.tags`, `NeedsYouCard` `ny.options`, `ToolStep` `image_ids`, `TurnItem` `parts.images`.

## Findings

### ui-correctness-4-01: minor: After switching refinement threads or discovery chats, the first keystroke reloads the thread and disables the composer mid-typing
- **File:**
  - `ui/src/modules/product/RefineChat.svelte:62-69` (effect) and `:71-85` (`loadThread`)
  - `ui/src/modules/product/DiscoveryChat.svelte:83-104` (same shape, `loadChat`)
- **Evidence:**
  - The id effect calls `void loadThread(currentTid)` with no `untrack`. The synchronous prefix of `loadThread` therefore subscribes the effect.
  - On a switch (`activeId !== threadId` with a previous `activeId`), that prefix reads `sending` and `stopping`, then `inputText` (`drafts.set(activeId, inputText)`). It then writes `inputText = drafts.get(threadId) ?? ''`.
  - When the new thread's draft equals the current text (the usual case: both `''`), the write does not change the value, so Svelte schedules no self-invalidating re-run. The effect stays subscribed to `inputText`, `sending` and `stopping`.
  - The first keystroke (`bind:value={inputText}`, `:368`) re-runs the effect:
    - `loadThread(tid)` sets `loading = true` and re-fetches the thread.
    - It also clears `stopNotice` and `backgroundTurn`.
  - The composer is `disabled={sending || stopping || loading || …}` (`RefineChat.svelte:370`, `DiscoveryChat.svelte:427`).
  - In DiscoveryChat a starter-chip click (it writes `inputText`) triggers the same reload.
  - Neither component is keyed per thread: `RefineTab.svelte:245` and `ChatTab.svelte:184` mount one instance and pass a new `tid` / `cid`.
- **Failure:**
  1. In Product, open Refine (or Chat), view thread A with an empty composer, and click thread B.
  2. Type a character. The textarea disables for one GET round trip. It loses focus, and the following keystrokes are dropped.
  3. The transcript also flashes the loading state. After it re-enables, the person must click back into the box.
- **Fix:** Run the loader untracked: `untrack(() => void loadThread(currentTid))` (and the same for `loadChat`). Effect deps are then only `tid` / `cid`.

### ui-correctness-4-02: minor: A background graph reload drops the commits of a newly fetched branch whose commits are older than the first page
- **File:**
  - `ui/src/modules/git/GraphView.svelte:1020-1029` (splice path)
  - `:1033-1041` (`applyGraph` guard)
  - `:359-372` (`lostRef`)
  - `ui/src/modules/git/graph-splice.ts:15-30`
- **Evidence:**
  - Every background reload (a commit, the 30 s auto-fetch) reads `FIRST_PAGE` = 2000 commits of `log --all --date-order`. It splices them onto the held tail whenever `held.length > FIRST_PAGE`.
  - That condition holds routinely: the next `PAGE` = 10000 is prefetched right after mount (`:278-282`).
  - `spliceHistory` checks only that the page is consistent with the OLD list.
  - The guard in `applyGraph` re-reads in full only when a ref was REMOVED (`refRemoved`), or when the OLD tip of a MOVED ref is outside the fresh prefix (`movedRefTips` stores `prev` shas, `:368`).
  - A ref that is NEW in this refresh (a key absent from `prev`) is never checked. Its commits are newly reachable. With `--date-order` they sit at their commit-date position, below the 2000-commit page whenever they are older than it. They are not in the old tail either, so the spliced list simply lacks them.
  - `more = heldMore` carries over, and later splices keep building on the incomplete list. Nothing reads them again until a full reload or a repo switch.
- **Failure:**
  1. Take a repo with more than 2000 commits across its refs.
  2. A colleague pushes a branch whose commits are a few weeks old (or the user adds a fork remote and fetches it).
  3. The next auto-fetch adds the branch to the sidebar. Its commits and chip never appear in the graph, and jumping to it finds nothing, until the repo is re-opened.
- **Fix:** In `lostRef`, record NEW tips too: for every key of `next` absent from `prev`, add `next`'s sha to a `newRefTips` set. In `applyGraph`, call `reloadGraph(true)` when a new tip is missing from the spliced list and it is not provably below the held tail. The simplest safe rule is "not in `freshPrefix`".

### ui-correctness-4-03: minor: The shared issue-account cache survives sign-out and impersonation, so pickers offer the previous identity's Jira/Linear accounts
- **File:**
  - `ui/src/lib/stores/issueAccounts.svelte.ts:24-26` (`if (this.loaded && !force) return Promise.resolve(this.list)`)
  - Consumers: `product/ImportDialog.svelte:62`, `product/PublishDialog.svelte:150`, `agents/JiraIssuePicker.svelte:61`
  - `ui/src/lib/events.svelte.ts:641-646` (`resyncForIdentity`)
- **Evidence:**
  - Iteration 3 replaced per-open `listIssueAccounts()` calls with a module singleton.
  - Once `loaded`, every non-forced `load()` returns the cached list. Nothing resets it:
    - `resyncForIdentity` resets notifications, activity and proof only.
    - `auth.logout()`, `auth.impersonate()` and `stopImpersonating()` (`auth.svelte.ts:402-444`, `:482-505`) change identity without a page reload.
  - Issue accounts are per user (`workflows/inspector/options.svelte.ts:93` says so).
- **Failure:**
  1. An admin opens Product → Import (or the agents' Jira picker), which caches the admin's accounts.
  2. The admin impersonates user U (or signs out and U signs in).
  3. U's Import and Publish dialogs and the Jira picker list the admin's account names and sites. Picking one sends an `account_id` U does not own, so search, import or publish fails.
  4. After impersonation stops, the admin sees U's accounts until Settings → Issue accounts forces a reload.
- **Fix:**
  - Add `reset()` (`list = []`, `loaded = false`, `error = null`, `inflight = null`) and call it from `resyncForIdentity`.
  - Or key the cache by `auth.me?.id` and treat a different id as not loaded.

### ui-correctness-4-04: minor: A live update for the open proof pack cancels a click on another pack, which stays on screen
- **File:**
  - `ui/src/lib/stores/proof.svelte.ts:281-284` (trailing refresh)
  - `:291-293` (`refreshDetail`)
  - `:222-239` (`open`)
  - `ui/src/modules/proof/ProofPage.svelte:233-241`, `:862`
- **Evidence:**
  - `open(id)` bumps `openSeq` and assigns only when its sequence is still current.
  - `refreshDetail()` is `if (this.detail) await this.open(this.detail.pack.id)`, so it also bumps `openSeq`.
  - While the person's click on pack B is in flight, `this.detail` is still pack A.
  - A `proof_pack_updated` event for A (`applyEvent` → `pendingDetail.add(A)` → the 300 ms trailing timer) calls `refreshDetail()`. That supersedes B's open, so B's response is dropped and A is re-fetched and shown.
  - `ProofPage.open` then clears `openingId` for B. The list highlights A (`class:active={detail?.pack.id === p.id}`), and the click looks ignored.
- **Failure:**
  1. A pack is assembling, which emits one event per artifact.
  2. Click another pack in the list. Often the detail pane stays on the assembling pack, and the click has to be repeated.
- **Fix:** Track the wanted pack (for example `wantedId`, set in `open`). Make `refreshDetail` refresh `wantedId ?? this.detail?.pack.id`, or skip when an open for a different id is in flight. The trailing timer should do the same.

### ui-correctness-4-05: minor: One failed progress poll stops tracking an automation run and labels the still-running run "Last run failed"
- **File:**
  - `ui/src/lib/stores/apiClient.svelte.ts:1913-1951` (`runAutomation`)
  - `ui/src/modules/api/AutomationEditor.svelte:258-260`, `:276-277`
- **Evidence:**
  - The delta poll `await api.get(...automation-runs/${run.id}?after=${have})` sits inside the single `try` around the whole loop. A transient failure (daemon blip, a timeout, a 5xx) jumps to the `catch`:
    - It toasts "Couldn’t run automation".
    - It returns `null`.
    - `finally` sets `running = false`.
  - `currentRun` / `lastRun` keep the partial report.
  - `noteRunProgress` ignores later progress events, because no waiter is armed and `running` is false.
  - The server starts every run with `report.passed: false` (`crates/otto-server/src/routes/api_automation_runs.rs:158-162`).
  - The chip renders `apiClient.running ? 'Running…' : showRun.passed ? 'Last run passed' : 'Last run failed'`, with a partial `n/m`.
- **Failure:**
  1. A run with slow steps is in progress, and one poll errors.
  2. The editor flips to "Last run failed · 2/2" and re-enables Run, so a second, concurrent run can be started. The first run is still executing and later finishes server-side.
  3. The view never updates. It shows a failure that did not happen.
- **Fix:**
  - Catch errors per delta GET inside the loop. Retry with backoff, keeping `running` and the waiter armed, and give up only after N consecutive failures.
  - On give-up, say "Lost track of the run — refresh run history" rather than reporting a failed run. Reload `loadAutomationRuns(id)` so the real status shows when it lands.

### ui-correctness-4-06: minor: The MCP audit log's "Load more" stays stuck on "Loading more rows…" after the filters change during a page load
- **File:** `ui/src/modules/mcp/AuditTab.svelte:72-95` (`load`), `:97-114` (`loadMore`), `:249-254` (button)
- **Evidence:**
  - `loadMore` sets `loadingMore = true` and clears it only `if (gen === generation)` in `finally`.
  - `load()` bumps `generation` but never resets `loadingMore`. It resets only `moreError`.
  - `load()` runs on every server or decision select change (the `$effect` at `:116-123`) and on Apply.
  - If it runs while a page is loading, the old `loadMore` returns without clearing the flag.
  - From then on, `if (loadingMore) return` (`:98`) blocks every Load more. The button is `disabled={loadingMore}` and reads "Loading more rows…" until the tab remounts.
  - The sibling lists reset the flag on reload: `PrList.svelte:51` and `history.svelte.ts:150`.
- **Failure:**
  1. In MCP → Audit, click "Load more".
  2. Before it lands, change the Server or Decision filter.
  3. The new list loads, but "Load more" is permanently disabled. Older audit rows for the new filter cannot be paged in without leaving the tab.
- **Fix:** In `load()`, add `loadingMore = false;` next to `moreError = null;`. The generation guard already drops the stale page.

### ui-correctness-4-07: minor: The PR detail view crashes when two Bitbucket participants (or two GitLab approvers) share a display name
- **File:**
  - `ui/src/modules/git/PrDetail.svelte:545`: `{#each pr.reviewers as reviewer (reviewer.name)}`
  - `ui/src/modules/git/PrDetail.svelte:566`: `{#each pr.approved_by as name (name)}`
  - Producers:
    - `crates/otto-git/src/providers/bitbucket.rs:598-612`: `participants[*].user.display_name`, no de-duplication.
    - `crates/otto-git/src/providers/gitlab.rs:440-457`: `approved_by[*].user.name`, which is the full name, not the unique `username`.
- **Evidence:**
  - Display names are not unique. Atlassian shows every deleted account as "Former user", and two colleagues can share a full name.
  - Bitbucket lists each participant once by account, but two accounts with the same `display_name` produce two identical keys. A participant without a user object maps to `""` (`vstr`) and collides the same way.
  - The GitHub provider merges by login (`github.rs:803`), which hides the problem on GitHub only.
  - The keyed block throws `each_key_duplicate` in production (see the header).
- **Failure:** Open a Bitbucket PR that two deactivated accounts commented on or approved (both "Former user"), or a GitLab MR approved by two people with the same name. The PR detail pane throws while rendering the reviewers card and the whole PR view stops rendering.
- **Fix:**
  - Key both blocks by index (`as reviewer, i (i)` / `as name, i (i)`).
  - Optionally carry a stable id from the providers: Bitbucket `user.uuid` / `account_id`, GitLab `user.username`.

## Rejected candidates (checked and dropped)

- **`personalAgents.loadRooms` self-invalidation (double fetch):** `roomsWs` is a plain private field, not `$state`, so the read is not tracked.
- **History rescan effect firing on a stale `done`:** `history.rescan` calls `activity.beginHistoryRescan()` (which resets `done: false`) synchronously, before the effect can run.
- **Workflows inspector `OptionCache` never refreshing a workspace's list:** this is by design (fetched once per page instance, Retry on failure). It is not a correctness defect.
- **Jira unassign sends `account_id: ''`:** this is unchanged from before the split, and Jira's handling of `""` could not be confirmed from code. Not reported.
- **SwarmSettings goals not reloading on a swarm switch:** it is a modal over the swarm page. I could not confirm a path that switches the swarm while it is open.
- **ManualTrigger goals textarea normalisation (blank lines collapse):** a controlled-input quirk carried over unchanged by the split. Below this lens's bar.
