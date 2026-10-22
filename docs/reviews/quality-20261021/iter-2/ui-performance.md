# Quality iteration 2 — lens: ui-performance

Reviewer: ui-performance lens. The review was read-only: no cargo, npm, build, e2e or test runs.

- Worktree: `fix/quality-loop-20261021` @ ab0a86f05.
- Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`, `ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/` and `docs/reviews/`.

## Score

Rubric: start at 10.0 and subtract per confirmed open finding: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 4 | 0.40 |
| nit | 0 | 0.00 |
| **Total** | 5 | **0.70** |

**Score: 10.0 − 0.70 = 9.30 / 10**

Of the five findings, one is carried over from iteration 1 in a smaller form (2-01). The other four were not covered by earlier passes. Three are request loops or repeated fetches started by effects: 2-02, 2-03 and 2-05.

## Previous-finding status (iteration 1)

| id | Iter-1 severity | Status | Evidence |
|---|---|---|---|
| ui-performance-1-01 | major | **Partly fixed → re-filed as 2-01 (minor)** | `RoomMedia.svelte:19` now reads `room` under `untrack`, so the second `update()` per snapshot is gone and annotation events no longer rebuild anything. However, `handleEvent` still calls `media.update(event.room)` on every snapshot (`RoomMedia.svelte:20`), and `update()` still ends with an unconditional `rebuildGraph(); publishStreams(); schedule();` (`room-media.ts:169`). Chat still arrives as a full snapshot (`rooms/socket.rs:176` `Notice::State` → `RoomEvent::Snapshot`, and `registry.rs:398` `changed()`). FIXES.md says the graph is no longer rebuilt on every chat message; it is still rebuilt once per message. |
| ui-performance-1-02 | minor | Fixed | `WorkflowsPage.svelte:196-205` tracks only `ws.currentId` and runs `load()` and `refreshActiveWorkflowRuns()` inside `untrack`. |
| ui-performance-1-03 | minor | Fixed | `Composer.svelte:121-150`: `paneBudget()` is cached in `budget` and refreshed only by the ResizeObserver and by changes to the attachments or `termInput`. `autosize()` is a single reset → `scrollHeight` → set, and there is no `oninput` resize alongside the effect (comment at :180). |
| ui-performance-1-04 | minor | Fixed | `Studio3D.svelte:167-201`: a `destroyed` flag is checked in `schedulePoll()` and after both awaits, and it is set in the cleanup. |
| ui-performance-1-05 | nit | Fixed | `RoomAnnotations.svelte:12-21`: expiry is parsed once per mark, and one `setTimeout` targets `nextExpiry`. The 500 ms interval is gone. |
| ui-performance-1-06 | nit | Fixed | `aws/LogsView.svelte:404-431`: `disposed` is checked after each await and before re-arming the timer. |

## Areas covered this iteration

- **Re-verification:** all six iteration-1 findings, against current code and the iteration-1 fix diff (`66b55a25c`). The diff was also checked for perf regressions it might have introduced: QueryEditor tab strip, BarHost lazy import, QuickOpen `scrollIntoView` effect, PlanTab/AnalysisTab pollers, ProofPage.
- **Event fan-out:**
  - `lib/events.svelte.ts` dispatch (onmessage → `liveEvents.dispatch` → `peek()`-gated stores).
  - The transcript store's `applyLive`/`applyDelta`, which is hidden-gated and has an identity check.
  - `workspace.svelte.ts` `applyEvent` for `session_status`, coalesced through `queueStatus` with one `sessions` write per frame.
- **Agents:**
  - `ConversationView`: live-draft follow, debounced search, `WeakMap` turn-text cache, find provider.
  - `SessionView`: effects and the usage poller.
- **Git:** `GraphView` (row windowing, rAF-coalesced scroll, memoised lane layout, hidden-graph resync parking) and `DiffViewer` (row and segment virtualisation, nav window, `MAX_LOADS`).
- **Database:** `GridView` (vertical and horizontal virtualisation) and `QueryEditor` (statement analysis idle-deferred above 16 KB, identity-preserving, per-tab editor height).
- **Workflows:** `WorkflowCanvas` drag path (no derived graph serialisation per move; `JSON.stringify` only at save).
- **Canvas:** D2 and Mermaid code edits (500 ms debounce, latest-wins render queue, last-rendered short-circuit).
- **Other modules:**
  - API: `ResponseViewer` (parse cache, size caps).
  - Browser: `RemoteLiveView` (socket generation guards, single pending-frame decode pump, 1 s meter clock only while live).
  - Kubernetes: `LogsView` (ring, rAF flush, incremental filter) and `ClusterWorkspace` access loads.
  - Panels: `FileTree` (windowed) and `ActivityPanel` (trail capped at 500).
  - Shell: `NotificationBell` (capped notices) plus `StatusBar`, `TabBar` and `Rail`.
  - `home/boxes/*` other than Classrooms (shared `poll`/`livePoll`/`freshness`).
- **Repo-wide audits:**
  - Every `setInterval` site (19).
  - Every global `addEventListener` without a matching removal: the remaining ones are module singletons, which is intended.
  - Every `requestAnimationFrame` site without a cancel.
  - WebSocket/EventSource open-versus-close balance.
  - `$derived` built from `JSON.stringify`.
  - Static imports of heavy libraries (`marked` only, in page chunks).
  - Effects that call a loader under a "missing" or "loading" guard. This audit produced 2-02 and 2-03.
- **Product:** `DesignArena` (2-04), `OverviewTab` effects, `PlanTab`, `AnalysisTab`.
- **Personal agents:** `PersonalAgentsPage` and `AgentPage` (2-02).
- **Mission control:** `MissionControlPage`.
- **Vault:** `DocsAgentsView`, `RefineDrawer`.
- **Design hall:** load effects in `LearnedCard`, `LearnedPage`, `ArtifactView`, `BrandEditor`, `OttoPanel`.

## Areas NOT covered yet (for the next iteration)

- **`lib/components/Terminal.svelte` internals:** the write path, `termFlow` and the WebGL atlas. Only the listener and timer balance and find were checked. Terminal-redraw-20261009 is the dedicated review.
- **`WorkflowsPage.svelte` beyond the load effect and canvas:** the run-merge path from `workflowRunBus.apply`, and `RunSteps`/`RunAgents` with 1 s tickers over large runs.
- **`workspace.svelte.ts`:** a full trace of `bucketSessions` recompute cost at a few thousand sessions.
- **`design-hall/site/*` (SiteStudio and its engine):** not covered.
- **`snip/SnipEditor`:** not covered.
- **`swarm/KanbanBoard`:** drag path.
- **`skills-lab/*`:** not covered.
- **`connections/*`:** not covered.
- **`brokers` TopicDetail:** message tail beyond the earlier windowing check.
- **`api/RequestBuilder`:** script editor and GraphQL schema paths.
- **Bundle-level verification:** chunk sizes and the entry graph. This needs a build, which the read-only rule excludes.

## Findings

### ui-performance-2-02 — major — `AgentPage` refetches the agent list back-to-back forever while "Agent not found" is shown

- **Files:**
  - `ui/src/modules/personal-agents/AgentPage.svelte:93-95`.
  - `ui/src/lib/stores/personalAgents.svelte.ts:24`, `:64-66` and `:77-99`.
- **Evidence:**
  - The effect is `$effect(() => { if (!personalAgents.agent(agentId) && ws.currentId) void personalAgents.loadAgents(ws.currentId); })`.
  - `agent(id)` is `this.agents.find(...)`, and `agents` is a deep `$state([])`. The effect therefore tracks the list.
  - `loadAgents` always ends a successful read with `this.agents = list` (`:88`), which is a new proxy and re-runs the effect.
  - When `agentId` is not in the list, the condition is true again and the next `loadAgents` starts at once. Nothing records that the id was already looked up. This happens for an agent deleted in another window, a stale notification or bookmark link, or an agent from another workspace.
  - Each round is one `GET` of the workspace's agent list, paced only by the round-trip time (loopback, so milliseconds). The previous round's `mapLimit(loadSchedules)` is cut short by `current()`.
  - Every round also toggles `loadingAgents`, so the body flips between the Skeleton (`:425-426`) and the "Agent not found" `EmptyState` (`:428-434`). The flicker is visible.
  - `PersonalAgentsPage.svelte:46-48` already loads the list on entry, so the AgentPage effect is redundant in the normal deep-link case anyway.
- **Impact:** an unbounded request storm against the daemon, plus a re-render loop in the webview, for as long as the not-found page stays open.
- **Fix:**
  - Look the id up once per `agentId` and workspace pair, for example: `let lookedUp = ''; $effect(() => { const key = `${ws.currentId}|${agentId}`; if (!ws.currentId || key === lookedUp) return; if (!untrack(() => personalAgents.agent(agentId))) { lookedUp = key; void personalAgents.loadAgents(ws.currentId); } });`.
  - Alternatively, gate the effect on a `loadedFor` workspace id that the store sets after the first successful read.

### ui-performance-2-01 — minor — Room media still tears down and rebuilds the host audio mix on every snapshot, including every chat message (carried over from 1-01, reduced)

- **Files:** `ui/src/modules/rooms/RoomMedia.svelte:20` and `ui/src/modules/rooms/room-media.ts:143-170, 565-591`. Server side: `crates/otto-server/src/rooms/socket.rs:176` and `crates/otto-server/src/rooms/registry.rs:398-400`.
- **Evidence:**
  - `handleEvent` runs `media?.update(event.room)` for every `snapshot`.
  - The server sends a snapshot for every state-changing action, chat included (`Notice::State` → `RoomEvent::Snapshot`). There is no `chat` event type (`room-types.ts:46-56`).
  - `update()` ends with an unconditional `this.rebuildGraph(); this.publishStreams(); this.schedule();`.
  - `rebuildGraph()` → `clearGraph()` disconnects every `MediaStreamAudioSourceNode`, then calls `createMediaStreamSource(createStream([track]))` again for each live member track and reconnects every peer mix.
  - Iteration 1's fix removed the second, prop-driven `update()` call. The per-snapshot rebuild remains, so FIXES.md's claim ("no longer rebuilds the audio graph on every chat message") is not accurate.
- **Impact:** the host's live audio mix is rebuilt once per chat line, join/leave or permission change. Each rebuild means GC churn and a risk of an audible click or dropout, from traffic that does not touch media.
- **Fix:**
  - In `update()`, compute a small graph signature and call `rebuildGraph()` only when it changes. The signature should cover: host id, `state.audioJoined`/`muted`, and, per member, id, generation, `connected`, `audio_joined`, `muted`, `room_muted` and whether a live audio track is present.
  - Do the same for `publishStreams()`, which already diffs, and for `schedule()`, which should run only when the peer set or generations change.
  - Track additions in `ontrack` (`:341`) already call `rebuildGraph()` directly, so they stay correct.

### ui-performance-2-03 — minor — Settings › Browser re-requests the engine status in a tight loop whenever that request fails

- **Files:** `ui/src/modules/settings/BrowserSettings.svelte:25-27` and `ui/src/lib/stores/browserLive.svelte.ts:112-129`.
- **Evidence:**
  - The effect is `$effect(() => { if (!browserLive.status && !browserLive.loading && browserLive.supported !== false) void browserLive.load(); })`. It tracks `status`, `loading` and `supported`.
  - `load()` sets `loading = true` and, in `finally`, `loading = false`.
  - On any error other than 404 or 405, `status` stays `null` and `supported` is not set to `false`. The error is stored only in `loadError`, which the effect does not read.
  - The `loading` flip back to `false` re-runs the effect, its condition holds again, and `load()` fires again. That happens immediately and indefinitely.
  - Triggers:
    - A 403: the route needs `Browser:View` (`crates/otto-server/src/policy.rs:1315-1316`), so a custom role without it hits this.
    - A 5xx.
    - The daemon restarting while this settings section is open: a refused connection fails in about a millisecond, so the loop spins hot.
  - The inline `LoadState` with Retry (`:146-148`) flickers on and off rather than staying up.
  - OverviewTab documents this exact trap (`OverviewTab.svelte:304-305`, "a failed load stays failed until Retry — without it the effect re-fired on every settle and hammered the daemon in a loop").
- **Fix:** add `&& !browserLive.loadError` to the guard, as `Channels.svelte:249` and `ScheduledTasksPage.svelte:369` already do. The existing Retry button calls `load()`, which clears `loadError` on success. Alternatively, run the load once on mount with `untrack`.

### ui-performance-2-04 — minor — Design Arena reloads every artifact list (own + each child story's) twice on mount and again on every Assistant open/close

- **File:** `ui/src/modules/product/design/DesignArena.svelte:99, 265-278, 291-321, 528-541, 728-744`.
- **Evidence:**
  - **Effect (a), lines 265-273:** reads `product.selectedId`, `mockupAssist.active` and, while active, `mockupAssist.storyId` before its `untrack`. It then does `childFilter = ''; void untrack(() => loadAll());`.
    - `loadAll()` is non-quiet (`loading = true`). It does `listAttachments()` and then `loadChildren(children)`, which is one `GET /product/stories/{child}/attachments` per child (`:309-321`).
    - "Create with AI" (`mockupAssist.openNew`, `:538`) and "Refine" (`openRefine`, `:730`) flip `active` from false to true. That re-runs the effect, so the full list (1 + N requests) reloads, the asset list flashes its loading state, and the user's child filter is wiped.
    - `closeAssist()` (`:740-744`) calls `loadAll(true)` itself. It also flips `active` from true to false, which re-runs the effect and starts a second, non-quiet `loadAll()` in parallel. That is 2 × (1 + N) requests.
  - **Effect (b), lines 275-278:** `children` is `$derived(product.childrenOf(storyId))`, which is a `this.stories.filter(...)` (`product.svelte.ts:152-154`) and therefore a new array whenever `stories` is reassigned (`product.svelte.ts:292, 310, 349, 359, 386, 407, 423, 448, 462`). Its effect re-runs `loadChildren` (N requests) each time.
    - On mount, (a) and (b) both run, so the children are fetched twice: 1 + 2N.
    - The computed `ids` key is never compared with the previous one.
  - Neither path has a request token, so two overlapping `loadChildren` calls race and the last response wins.
- **Fix:**
  - In (a), track only the selected story: `const sid = product.selectedId; untrack(() => { if (mockupAssist.active && mockupAssist.storyId !== sid) {…} childFilter = ''; void loadAll(); });`.
  - In (b), keep `let lastIds = ''` and return early when `ids === lastIds`. Skip the first run, because `loadAll()` already loads the children.
  - Add a sequence token to `loadChildren`.
- **Cross-lens note (correctness, not scored here):** because (a) tracks `mockupAssist.storyId`, "Refine with Otto" on a **child** story's artifact in an epic's arena sets `storyId = att.story_id`. That differs from `product.selectedId`, so the same effect immediately calls `mockupAssist.close()` and the refine panel closes as soon as it opens. The fix above removes this too.

### ui-performance-2-05 — minor — A session view whose handover source is gone re-fetches that source on every `sessions` write

- **Files:** `ui/src/modules/agents/SessionView.svelte:217-231` and `ui/src/lib/stores/workspace.svelte.ts:107-111, 816-837`.
- **Evidence:**
  - The effect is `$effect(() => { if (handoverFromId && !ws.getSession(handoverFromId)) void ws.ensureSession(handoverFromId); })`.
  - `getSession` reads `sessionById`, which is `$derived(new Map(this.sessions.map(...)))`. That is a new Map on every `sessions` write, including the once-per-frame status flush (`queueStatus`, `:1998`), renames and creates.
  - `ensureSession` dedupes only requests still in flight (`ensuring`). When `GET /sessions?ids=…` returns no listable row, nothing is added and nothing records the miss. A row is not listable when the source was deleted after the handover, belongs to another workspace, or is hidden by device isolation.
  - The next status burst in the workspace therefore fires the same GET again. With several agents working, that means a request on most status transitions, for every mounted pane showing such a session.
  - The store's own `openWhenLoaded` (`:839-845`) explicitly avoids this kind of retry loop ("once (no retry loop when the row turns out not to be listable here)").
- **Fix:** ensure once per id, for example: `let ensuredFrom: string | null = null; $effect(() => { const id = handoverFromId; if (!id || id === ensuredFrom) return; if (!untrack(() => ws.getSession(id))) { ensuredFrom = id; void ws.ensureSession(id); } });`. Alternatively, add a short-lived negative cache in `ensureSession`.
