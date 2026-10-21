# Quality iteration 1 — lens: ui-performance

Reviewer: ui-performance lens (read-only; no builds/tests run). Worktree `fix/quality-loop-20261021` @ d447516c4.
Excluded: `ui/src/modules/rooms/games/**`, target/, node_modules/, docs/reviews/.

## Score

Rubric: 10.0 − (blocker 1.0, major 0.3, minor 0.1, nit 0.03) per confirmed open finding.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 3 | 0.30 |
| nit | 2 | 0.06 |
| **Total** | 6 | **0.66** |

**Score: 10.0 − 0.66 = 9.34 / 10**

## Previous-finding status

First iteration of this loop: there are no earlier findings to track. The earlier perf passes (app-20261004/05, quality-20261007/08, terminal-redraw-20261009) were used only as coverage context. Their fixes are present and none were re-reported. Those fixes include the shared clock with minute-grain `rel()`, the lazy shell chunk, windowed lists, `pollWhileVisible`, the AWS poll backoff and the cancel-on-unmount handling.

## Areas covered

- Entry and shell chunking: `ui/src/main.ts`, `ui/src/App.svelte` (lazy shell/boot chunks) and the static import graph of `ui/src/shell/*`.
- Global timers and listeners: an audit of every `setInterval` (17 sites) and of `addEventListener`/`removeEventListener` balance across `.svelte` files.
- Shared clock: `lib/stores/now.svelte.ts`, `RelTime`, `LiveWorkingDot`.
- Sidebar: `shell/Navigator.svelte` session filtering and derived chains. `shell/Palette.svelte` search, which is debounced.
- Session rooms (not games): `RoomPage`, `RoomMedia`, `room-media.ts`, `RoomScreens`, `RoomAnnotations`, `RoomChat`, `RoomRecap`, `recap-capture.ts`, `room-client.ts`. Also the server snapshot cadence in `crates/otto-server/src/rooms/socket.rs`.
- Agents: `conversation/Composer.svelte` (per-keystroke path), the `SessionView` derived set, the `history/HistoryPage` list and `personal-agents/RoomsView` (windowed).
- Effects that trigger fetches: about 60 `$effect(() => void load…)` sites. The sync prefix of each loader was checked for unintended tracked reads (Workflows, Swarm, Settings tabs, MCP, WorkQueue, Channels, SkillActivity and others).
- Polling loops: Athena, Logs Insights, ReviewPanel, LocalReviewPanel, skills-eval RunDetail, Studio3D, AWS LoginSheet, k8s Metrics/WorkloadPods/MonitorInsights.
- Large lists: AWS Logs/SQS/S3, Kafka TopicDetail, MCP AuditTab, Kanban, Proof. Most are windowed or bounded.
- Database `ResultsGrid` elapsed timer. `Scene3DViewport` rAF/dispose. Vault `GraphView` sim loop.

## Areas NOT covered yet (next iteration)

- `lib/components/Terminal.svelte` (2.6k LOC) and `termFlow`/`termLatency`. Only the diagnostics interval was checked. A dedicated earlier review exists (terminal-redraw-20261009).
- `agents/conversation/ConversationView.svelte`, `ToolStep`, `CodeView`: per-message re-render cost on streaming.
- `lib/events.svelte.ts` dispatch fan-out and `lib/stores/workspace.svelte.ts` (2.2k LOC) bucket recompute per status event.
- `git/GraphView`, `git/DiffViewer`, `git/WipPanel`: large-repo behaviour.
- `database/GridView` and `QueryEditor` editing paths. `DatabasePage` tab switching.
- `workflows/WorkflowsPage.svelte` beyond the load effect (5.1k LOC; canvas drag/zoom, run merge).
- `canvas/*` (D2/Mermaid/Excalidraw re-render on edit). `design-hall/*`. `product/*`.
- `api/RequestBuilder` and `ResponseViewer` on large bodies. `browser/*` live view.
- Bundle-level verification. This needs a build, which was not run under the read-only rule.

## Findings

### ui-performance-1-01 — major — Room media rebuilds the WebAudio mix graph and reconciles peers on every room object change, twice per snapshot

- **Files:** `ui/src/modules/rooms/RoomMedia.svelte:17-18` and `ui/src/modules/rooms/room-media.ts:143-170, 566-590`. Callers: `ui/src/modules/rooms/RoomPage.svelte:69-75` and `ui/src/modules/rooms/room-state.ts:29-38`.
- **Evidence:**
  - `RoomPage.receive()` calls `mediaPanel?.handleEvent(event)` first. For a snapshot that runs `media.update(event.room)` (RoomMedia.svelte:18). It then assigns `room = event.room`.
  - The prop change re-runs `$effect(() => { if (!media) return; if (connected) media.update(room); else media.disconnect(); })` (RoomMedia.svelte:17). That is a second `update()` with the same snapshot.
  - Every `annotation`, `clear_annotations` and `annotation_removed` event also produces a new room object: `applyRoomEvent` returns `{...room, annotations: …}`. That runs `update()` again even though no media field changed.
  - `update()` always ends with `this.rebuildGraph(); this.publishStreams(); this.schedule();` (room-media.ts:169).
  - `rebuildGraph()` calls `clearGraph()`, which disconnects every `MediaStreamAudioSourceNode`. It then creates a new `MediaStream` and source node for every live member track (room-media.ts:566-590).
  - `schedule()` queues a full `reconcile()` pass over every peer.
  - Chat is delivered as a full snapshot; there is no `chat` event type (room-types.ts:46-56). So every chat message tears down and rebuilds the host's audio mix twice, and every pen stroke or pointer click does so once.
- **Impact:** CPU and GC churn on the real-time media path, driven by unrelated UI traffic. Every rebuild is a source-node teardown in a live call, which risks audible dropouts or clicks in the host mix.
- **Fix:**
  - Make the `RoomMedia` effect depend only on `connected`, and read `room` with `untrack`. Snapshots already reach the client through `handleEvent`, and annotation events never change media state.
  - Alternatively, in `update()`, skip `rebuildGraph()` unless a field it depends on has changed: member ids, `connected`, `audio_joined`, `muted`, `room_muted`, generation, host id, or the peer audio track. Keep a small signature string the way `publishStreams` already diffs.

### ui-performance-1-02 — minor — The Workflows page load effect tracks `types`, `templates` and `current`, so every page entry fires the list fetch about 4×

- **File:** `ui/src/modules/workflows/WorkflowsPage.svelte:193-200`, which calls `load()` at `:623-655`.
- **Evidence:**
  - The effect is `$effect(() => { if (ws.currentId) { void load(); void ws.refreshActiveWorkflowRuns(); } })`.
  - `load()` runs synchronously up to its first `await`, so its reads are tracked. On the first run it reads `current` (`if (current && current.workspace_id !== wsId)`) and `types.length` (`if (types.length === 0) types = await …`).
  - `types` and `templates` are `$state` (lines 77-78).
  - When `types` lands, the effect re-runs. That second run reads `templates.length === 0` while the first run's templates request is still in flight, so it fires a second `/workflows/templates`.
  - Each `templates = …` assignment re-runs the effect again.
  - **Net per cold entry** (the page is remounted on navigation, and `types`/`templates` are component state): `/node-types` ×1, `/workflows/templates` ×2, `/workspaces/{id}/workflows` ×4 (three discarded by `loadSeq`), `refreshActiveWorkflowRuns` ×4.
- **Fix:** run the loaders untracked, for example `$effect(() => { const id = ws.currentId; if (id) untrack(() => { void load(); void ws.refreshActiveWorkflowRuns(); }); })`. `ScheduledTasksPage` and `SkillDetail` already use this idiom. Fetching `types` and `templates` once in parallel with `Promise.all` would also remove a sequential round trip.

### ui-performance-1-03 — minor — The chat composer forces about four synchronous layouts of the pane on every keystroke

- **File:** `ui/src/modules/agents/conversation/Composer.svelte:121-158, 414`.
- **Evidence:**
  - `autosize()` first calls `paneBudget()`. That loops over every child of the pane, calling `getComputedStyle(el).flexGrow` and `el.offsetHeight`, then reads `composerEl.offsetHeight` and `ta.offsetHeight`. This forces style and layout, because the keystroke just dirtied the textarea.
  - It then writes `ta.style.height = 'auto'` and reads `ta.scrollHeight`, which forces a second layout, and writes the height again.
  - `autosize` runs twice per keystroke. It is bound synchronously as `oninput={autosize}` (line 414). The `$effect(() => { void text; void tick().then(autosize); })` (lines 155-158) also fires because `text` is the bound draft.
  - The pane is the session chat pane, including the conversation view.
- **Impact:** typing latency that grows with pane and DOM size. This is classic read/write layout thrash on the hottest input in the app.
- **Fix:**
  - Compute `paneBudget()` only in the ResizeObserver `measure` callback, and when attachments or rows change. Cache it in a variable.
  - In `autosize()`, do only `height:auto` → `scrollHeight` → set.
  - Drop one of the two triggers: keep the effect for restored or programmatic drafts, but skip it when the last change came from `oninput`. Or remove `oninput={autosize}` and rely on the effect.

### ui-performance-1-04 — minor — The Studio3D agent poll survives unmount, and keeps polling forever on errors

- **File:** `ui/src/modules/design-hall/studio3d/Studio3D.svelte:165-196`.
- **Evidence:**
  - Cleanup is `$effect(() => () => { if (pollTimer) clearTimeout(pollTimer); })`. There is no disposed flag.
  - `poll()` awaits `listTurns(artifact.id)`, then calls `schedulePoll()` when `agent?.turnId === a.turnId` in both the running branch and the `catch` branch.
  - `agent` is component state and is never cleared on destroy.
  - If the view unmounts while a poll request is in flight, the continuation arms a fresh 3 s timer that nothing ever clears. It keeps polling while the turn runs. If the request keeps failing (artifact deleted, daemon restarting), the `catch` branch reschedules indefinitely, every 3 s, for the life of the window. It also raises toasts from a dead view.
- **Fix:** add `let destroyed = false;`, set it in the cleanup, and guard `schedulePoll()` and the post-await continuation with `if (destroyed) return;`. `RunDetail.svelte` (`disposed`) and `ReviewPanel.svelte` already do this.

### ui-performance-1-05 — nit — The room annotation overlay re-filters all marks every 500 ms, even with no marks and while hidden

- **File:** `ui/src/modules/rooms/RoomAnnotations.svelte:12-13`.
- **Evidence:**
  - `$effect(() => { const timer = setInterval(() => now = Date.now(), 500); … })` runs for every mounted overlay. There is one per visible shared screen.
  - Each tick re-evaluates `marks = (room.annotations ?? []).filter(… Date.parse(a.expires_at) > now)` over up to 400 annotations (`applyRoomEvent` caps them with `.slice(-400)`). That produces a new array and a keyed `{#each}` reconcile.
  - This happens when there are zero marks, and when the window is hidden.
- **Fix:** schedule a single `setTimeout` for the earliest `expires_at` among the current marks (re-armed when `room.annotations` changes), or skip ticks when `document.hidden` or when there are no marks. Parse `expires_at` once per mark.

### ui-performance-1-06 — nit — A Logs Insights result poll in flight at unmount re-arms itself

- **File:** `ui/src/modules/aws/LogsView.svelte:402-421, 485-491`.
- **Evidence:**
  - `onDestroy` calls `stopPolling()`, which only clears `iPoll`, and stops the query.
  - It does not change `qid` or set a disposed flag.
  - `pollResults()`'s only post-await guard is `if (qid !== id) return;`. A poll in flight at unmount therefore schedules another `setTimeout(pollResults…)`, and each poll is an `aws` CLI process (about 0.3 s CPU).
  - This is bounded in practice: the stop makes the next result `done`.
- **Fix:** set a `disposed` flag in `onDestroy` and check it after the `await` and before re-arming. This is the same pattern as Athena's `executionGeneration++`.
