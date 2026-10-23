# UI correctness review: iteration 6 (wave 0)

Lens: logic bugs in the Svelte 5 UI (`ui/src/**`). This covers runes and reactivity, races between async loads and navigation, state left broken after errors, and API usage that does not match `ui/src/lib/api/types.ts` and the Rust producers.

Branch `fix/quality-iter6-20261023` @ `02a64019b` (main after the iteration-5 merge, PR #121). The review was read-only: no builds, tests or e2e were run. The diff since the iteration-5 base (`8177c4d85..HEAD -- ui/src`, 186 files) was the first priority.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 4 | 0.40 |
| nit | 1 | 0.03 |

**Score = 10.0 - 0.40 - 0.03 = 9.57 / 10**

## Previous-finding status (iteration 5)

All 5 iteration-5 findings are fixed in the current code.

| ID | Status | Evidence |
|---|---|---|
| ui-correctness-5-01 | Fixed | `modules/product/storyReset.svelte.ts:12-22` (`onStoryChange`): the effect depends only on `storyId()`. The reset and teardown run in `untrack`. TestCasesTab `:259-267`, RewriteTab `:116-125` and PlanTab use it, so `startPolling()` no longer re-runs the reset. |
| ui-correctness-5-02 | Fixed | `lib/stores/issueAccounts.svelte.ts:41-46`: `load()` reads its gates inside `untrack`. The call sites are also untracked: PublishDialog `:146-151`, ImportDialog `:58`, JiraIssuePicker `:66`. |
| ui-correctness-5-03 | Fixed | `modules/product/design/DesignArena.svelte:265-283`: a story switch clears `ownAtts`, `childAtts` and `selectedId` before `loadAll()`. `:312-337`: `loadAll` commits only while `my === loadSeq && owned()`, and clears `loading` only for the newest load. |
| ui-correctness-5-04 | Fixed | The three lists are keyed by index: ToolStep `:309` (`as id, i (i)`), NeedsYouCard `:156`, MemoryTab `:281`. |
| ui-correctness-5-05 | Fixed | `modules/mcp/ProfilesTab.svelte:46-50`: `untrack(() => void mcpProfiles.load(true))`. The store still reads `this.loaded` first (`mcp-profiles.svelte.ts:21`), but no tracked caller is left: McpProfilePicker's re-run is a cache hit. |

Counts: 5 fixed, 0 still open, 0 rejected as invalid.

## Areas covered

- **Re-verification** of all 5 iteration-5 fixes (table above).
- **New or changed since iteration 5:**
  - **Database store split.** `lib/stores/database/{history,savedQueries,assist}.svelte.ts` and the façade in `database.svelte.ts:326-418`. I diffed each method against the pre-split code (`8177c4d85:ui/src/lib/stores/database.svelte.ts`).
    - `loadHistory`, `loadSavedQueries` and `saveQuery` are faithful moves. The epoch and request guards are unchanged.
    - The façade setters cover every field that `scrub` and the access-reset path write.
    - The assist move is faithful. Its close and stop paths have a defect that predates the split: **finding 6-01**.
  - **`lib/relaySocket.ts`** and both wrappers (`rooms/room-client.ts`, `rooms/games/client.ts`). I checked the silence deadline across a stalled handshake, retries against `maxAttempts`, `gave-up`, wake on `online` and `visibilitychange`, the `ended` dispose order, and a stale socket's handlers after `detach`. No defect.
  - **`RoomPage`**: attach, re-attach and Retry (`client.connect()` after `gave-up` gets one more attempt), and the recap wait being cleared on any non-connected status. No defect.
  - **Shell overlays** (`shell/overlays.svelte.ts`, Navigator, TabBar, ProductPage) and **`LazyMount`** together with `lazy-component.svelte.ts`. I checked retry on a fresh mount, reset of the toast de-duplication, and that `onerror` clears the open flag. No defect.
  - **App and tray daemon supervisor** (`App.svelte:98-150`, `desktop/daemonState.ts`, `TrayPage.svelte`): listener disposal and Restart/Reinstall single-flight. No defect.
  - **`AttachmentUrls`** (`product/attachmentUrls.svelte.ts`) in AttachmentsPanel and OverviewTab.
    - Generation guard and revoke-on-stale are correct.
    - The loaders are called after an `await` or inside `untrack`, so the cache reads create no subscriptions.
    - AttachmentsPanel's unguarded list load is safe because OverviewTab is `{#key product.selectedId}` (`ProductPage.svelte:907`).
  - **Home boxes:**
    - `freshness()` use in ClassroomsBox, UsageBox, InsightsBox, MissionControlBox and K8sBox: **finding 6-02**.
    - ClassroomsBox's screen poller keyed on `openRoomId` and `roomScreensKey`, `throttledNow`, `zoomedAway`, and the card refocus.
    - SessionsBox load states.
  - **WidgetCard and Dashboards:** LoadState wrapping, aborted runs, `noteWidgetColumns`, and `addBlocked`.
  - **Terminal** after removing `injectInput`: no remaining reader of `ws.injections`. ResultsGrid and TopicDetail use `ContextPacketDialog`, and `sendTo.ts` uses `sendInput` on the server side.
  - **`termGridSync.resizeSendStep`** against the inline code it replaced. It is equivalent.
  - **Brokers reset warning, `reviewFallback.ts`, `rowFocus.ts`, `diffRowFocus.ts`, WipPanel diff Retry, ConfluencePagePicker focus moves, and ApprovalsTab member labels.** No defect.
  - **`MonitorSettings`** inline validation: **finding 6-05**.
  - **Kart and Shooter context-loss remount** (`lossBudget`): a guest's `state` is refreshed on every frame, so the remount has a state to boot from. I also checked the Escape-to-menu and Resume focus effects. No defect.
- **Iteration-5 "not covered" list:**
  - **Terminal `switchEngine`, `adoptEngine` and `parkEngine`** (`Terminal.svelte:2229-2427`) against `termPark.ts` eviction:
    - `put` disposes a same-key entry first and never evicts the entry it just parked.
    - `wireParked`'s `onclose` evicts only its own value.
    - Adopt rejects a closed socket or detached element.
    - No defect.
  - **`RunInspector` live updates:** the result fetch is keyed on id, status and Retry, aborted on change, and `fetched` is matched by id. **SwarmPage** beyond the settings modal was not traced further.
  - **MCP `PoliciesTab`, `PolicyForm`, `RulesDrawer`, `TokensPanel`:** `latestOnly` load, import and replace, evaluate. No defect worth reporting. `evalServerId` can go stale across a workspace switch, but only while the modal drawer stays open over the switch, so I dropped it.
  - **Shooter `net.ts`** decode against encode, field by field:
    - The loot code arithmetic (`ITEM_SLOTS = ITEM_IDS.length + 1`).
    - The held code range 0..79.
    - Infected flags, mode state and capture points.
    - Optional `l` and `b` with the previous list reused.
    - Tracer events with 12 fields.
    - Consistent. Kart `net.ts` was skimmed only.
  - **`MockupViewer` outside DesignArena:** it has no other mount (`rg '<MockupViewer'` finds only `DesignArena.svelte:980-986`). DesignArena always passes `source` for html, so the html branch's missing gen check is unreachable. The mermaid live-edit path has a real defect: **finding 6-03**.
- **Sweep: lists keyed by item value** (`{#each … as x (x)}` and `(x.label)`, about 80 sites). I followed each server- or model-supplied one to its producer:
  - DocsAgentsView `changed_paths` and `written` are de-duplicated by `parse_results` (`vault_docs_agent.rs:4049-4057`).
  - MultiRun `candidates` comes one per cluster row.
  - CreatePr `reviewers` is de-duplicated at insert.
  - ErrorPanel `suggestions` are parsed ClickHouse names.
  - **MetricChart keys series by `label`**, and the K8s Fleet producer can emit duplicate labels: **finding 6-04**.

## Areas not yet covered (next iteration)

- `home/school/scene*.ts` (`mountSchool`, `pick`, camera): the WebGL side of the classroom.
- Kart `net.ts` encode/decode, traced field by field.
- `SwarmPage.svelte` beyond the settings modal and RunInspector.
- `lib/stores/database/model.ts` and `statement.ts` façade users (tab snapshot, capture and restore across a connection switch).
- `vault/GraphView.svelte` and `vault/DocsAgentsView.svelte` effects (large, last reviewed in iteration 3).
- `workflows/WorkflowsPage.svelte` and `WorkflowCanvas.svelte` (changed 3-5 times in the last week).

## Findings

### ui-correctness-6-01: minor: Closing (or Stopping) the DB assistant during its first turn leaves a hidden agent session running, and a reopened panel adopts the old turn's answer
- **File:**
  - `ui/src/lib/stores/database/assist.svelte.ts:229-245` (`closeAssist`), `:101-142` (`startAssist`), `:145-148` (`stopAssist`), `:165-173` (`adoptAssist`), `:71-87` (`openAssist`)
  - `ui/src/modules/database/DbAssistantPanel.svelte:79-88` (`close`)
- **Evidence:**
  - `closeAssist` sends `DELETE …/db/assist/{aid}` only `if (id && aid)` (`:238`). During the first turn `assistId` is null until the `db_assist_session_started` event is adopted, or until the POST resolves (`:130`).
  - The panel's Close asks for confirmation only when `assistSessionId || assistProposedSql`. During that window it closes at once.
  - `closeAssist` does not abort the in-flight POST (`assistAbort` is never touched), and it does not remember that a turn is outstanding. The adoption event is then dropped, because `adoptAssist` returns false once `!this.assistOpen` (`:166`).
  - When the POST resolves, it writes `assistId`, `assistSessionId`, `assistProposedSql` and `assistNote` into the closed store (`:130-133`). No DELETE ever follows.
  - The daemon starts that session (`db_assist.rs:309-325`, `run_session_turn`). Only `close` (`db_assist.rs:456-472`) kills it and removes its working directory.
  - The session is invisible:
    - It carries `meta.source = 'db_assist'`, which is in `BACKGROUND_SOURCES` (`lib/stores/sessionBuckets.ts:30`), so no Agents list shows it.
    - There is no reaper. It lives until the daemon restarts.
  - **Reopen race:** if the user reopens the panel and asks again before the old POST lands, `openAssist` resets `assistBusy = false`. The stale response then:
    - sets `assistId` to the OLD assist, so the next turn resumes the wrong assist;
    - runs `finally { this.assistBusy = false }` (`:140`) while the new POST is still in flight, which re-enables the input and Summarize.
  - **Stop:** `stopAssist` aborts the POST and clears `assistBusy`. A `db_assist_session_started` that arrives after Stop is rejected (`adoptAssist` needs `assistBusy`), so this session is orphaned the same way when the panel is then closed.
- **Failure:**
  1. In Database → Ask (DB assistant), type a question and press Enter.
  2. Within the first seconds, while the dir is seeded and the PTY spawns, click Close (no confirm appears) or Stop then Close.
  3. A Claude/Codex PTY keeps running on the connection's schema. It does not appear in Agents and cannot be closed from the UI.
  4. Reopening and asking again can bind the new panel to the old assist id.
- **Fix:**
  - Give the store a turn generation: `closeAssist` and `openAssist` bump it and abort `assistAbort`.
  - In `startAssist`, when the response belongs to an old generation, do not touch panel state, and fire `dbAssistClose(id, resp.assist_id)` to discard it. Guard `finally` with the generation too.
  - Keep a small "discard on sight" set of `(connId)` for turns closed before an id was known. When `db_assist_session_started` arrives for one of them (route `lib/events/routes/database.ts:7`), DELETE that assist instead of dropping the event.
  - Test: close mid-POST, then resolve, and assert that `dbAssistClose` was called with the minted id and that the store is still closed.

### ui-correctness-6-02: minor: An aborted Classroom load is stamped "fresh", so returning to the box within 30 s skips the reload for up to 5 minutes
- **File:**
  - `ui/src/modules/home/boxes/ClassroomsBox.svelte:105-124` (`load`; `:116` `if (signal?.aborted) return true;`), `:127-133` (`freshness` + `livePoll` `immediate: fresh.start('', 30_000)`)
  - `ui/src/modules/home/boxes/poll.ts:33-45` (`freshness.run` stamps `at` whenever `run` returns true)
  - `ui/src/lib/live.ts:158`, `:163-167` (`ms` is the 300 s safety net while the event socket is up)
- **Evidence:**
  - When `onScreen` turns false (another Home space, scrolled out, a hidden native pane), the effect's cleanup `p.stop()` aborts the in-flight `/sessions?…limit=1000` read (`lib/poll.ts:152`).
  - `load` returns `true` for an abort. `freshness.run` records it as a success (`at = Date.now()`), although nothing was assigned.
  - Back on screen within 30 s, `fresh.start` returns `false`, so the poller starts with `immediate: false`. With the event socket up, the next run is the 5-minute safety tick.
  - K8sBox fixed exactly this case (`K8sBox.svelte:64-67`: "An aborted read … loaded nothing: `false` keeps `freshness` from stamping it fresh"). ClassroomsBox was not changed to match.
- **Failure:**
  1. Open Home and switch Home spaces (or scroll the Classrooms box away) while the box's first load is running.
  2. Come back within 30 s.
  3. `fetched` and `archivedRows` stay empty (`loaded` is already true), so the school shows only the current workspace's sessions from `ws.sessions` (plus any `otherWsSessions`). Other workspaces' classrooms and the whole detention bench are missing until a session event or the 5-minute tick.
  4. A later aborted refresh keeps stale rows marked fresh the same way.
- **Fix:** In `load`'s catch, `if (signal?.aborted) return false;`, as K8sBox does. `finally { loaded = true }` should also skip the aborted case, so the first-load skeleton stays up.

### ui-correctness-6-03: minor: Editing a Mermaid artifact's source re-renders through a "Rendering…" blank and remounts the annotation overlay on every keystroke, refetching annotations and discarding a pin being placed
- **File:**
  - `ui/src/modules/product/MockupViewer.svelte:97-102` (render effect on `source`), `:109-151` (`renderFor` sets `rendering = true`, `mermaidSvg = null`, then awaits `renderMermaid`), `:190` (`{#if rendering}`), `:217-218` (`{#if !rendering && !renderError && renderBox}<MockupAnnotations …/>`)
  - `ui/src/modules/product/MockupAnnotations.svelte:28-35` (local `notes`, `pending`, `pendingBody`, `mode`), `:53-57` (mount effect → `loadNotes()` → `GET /product/attachments/{id}/annotations`)
  - `ui/src/modules/product/design/DesignArena.svelte:983` (unkeyed `<MockupViewer attachment={att} {source} hideToolbar />` for mermaid), `:1029` (`oninput={onCodeInput}`), `:500-503` (`applyLocalEdit` sets `source` with no debounce)
- **Evidence:**
  - With the code view open on a Mermaid artifact, every input event replaces `source`. MockupViewer's effect then calls `renderFor`, which synchronously sets `rendering = true` and `mermaidSvg = null`, and resolves only after the async mermaid parse and render. This differs from the html branch, which resolves `htmlContent = src` within the same flush.
  - So each keystroke:
    - replaces the diagram with "Rendering…";
    - unmounts `MockupAnnotations` and mounts it again when the render ends. Each mount fires `listAnnotations`, and its unsaved `pending` pin and `pendingBody` text are reset.
  - `renderMermaid` is called without `isStale` (`lib/render/mermaid.ts:80-87` supports it). Every keystroke's render runs in the serial queue.
  - An older render's `finally { rendering = false }` (`:149`) is not generation-guarded. It shows an empty box, and mounts the overlay again, between the newer renders.
- **Failure:**
  1. Product → Design → open a Mermaid artifact → Show source → type.
  2. The preview flickers to "Rendering…" on every character.
  3. The Network panel shows one annotations GET per keystroke, each with the error toast "Couldn’t load annotations" if it fails.
  4. A pin placed on the diagram, with its note half-typed, vanishes the moment the source changes.
- **Fix:**
  - In `renderFor`, when `src !== null`, keep the last good `mermaidSvg` and do not set `rendering`. Swap in the new SVG only when `myGen === gen`.
  - Pass `isStale: () => myGen !== gen` to `renderMermaid`.
  - Guard `finally` and `catch` with `myGen === gen`.
  - Optionally debounce the live preview by about 200 ms in DesignArena.
  - `MockupAnnotations` then stays mounted across edits. Its effect already reloads on an `attachmentId` change.

### ui-correctness-6-04: minor: MetricChart keys its series by display label, so two Kubernetes clusters with the same name crash the Monitor Fleet charts
- **File:**
  - `ui/src/lib/components/MetricChart.svelte:325`, `:347`, `:360`, `:372` (`{#each series as s, i (s.label)}` / `(s.label)`)
  - `ui/src/modules/kubernetes/monitor/MonitorFleet.svelte:330-337` (`toChart`: `label: … x.label`; the unique `x.key` is dropped), `:649`
  - Producer: `crates/otto-k8s/src/monitor/fleet.rs:1198-1220`. The label is the cluster's display name (`names.get(cid)…"name"`, plus `/{rest}` for `by` ≠ cluster). The `key` (cluster id) is what is unique.
  - Cluster names are not unique: `crates/otto-state/migrations/0114_k8s_clusters.sql:19` (`name TEXT NOT NULL`, plain index at `:36`). `clusters.rs` import and register take the name as given.
- **Evidence:**
  - The Fleet overview series are grouped by cluster id but labelled by cluster name.
  - Two registered clusters named alike (two kubeconfigs both exposing `docker-desktop`/`minikube`, or two EKS "prod" clusters in different accounts) give two series with the same `label`. With `by=namespace`/`workload`, a shared namespace such as `default` does the same.
  - MetricChart keys four `{#each}` blocks by that label. Svelte 5.57 throws `each_key_duplicate` in production builds, and the app has no `<svelte:boundary>`.
- **Failure:**
  1. Register two clusters with the same name, enable monitoring on both, and open Kubernetes → Monitor (Fleet).
  2. The memory, rps, 5xx and latency charts throw on render. The overview stops updating, and the error is logged instead of shown.
- **Fix:**
  - Key the series by index (`as s, i (i)`) in all four blocks. The header `{#each series as s (s.label)}` becomes `as s, i (i)`.
  - Optionally carry the unique `key` into `MetricChartSeries` and use it as the key.
  - Server side, disambiguate duplicate names in the label (for example, append a short id when two rows share a name).

### ui-correctness-6-05: nit: Monitoring settings keeps index-keyed validation errors after a probe, mapping or exclusion is removed, so the message moves to the next row
- **File:** `ui/src/modules/kubernetes/monitor/MonitorSettings.svelte`
  - `:54`, `:168` (`errors` is set only by Save's `validate`)
  - `:241-244` (`removeProbe`), `:265-267` (`removeMapping`), `:298-301` (`removeExclusion`)
  - `:335` (summary banner), `:400-446` (errors looked up by `probe.${i}…`, `probe.${i}.map.${j}`, `ex.${i}`)
- **Evidence:**
  - Validation errors are keyed by array index and only recomputed on Save. Removing a row re-indexes the rows after it but leaves `errors` untouched:
    - If the invalid row is removed, its message (for example "Name is required.") now renders under the row that slid into that index, which may be valid.
    - If it was the last row, the message disappears, but the banner still says "Fix the 1 highlighted field to save." with nothing highlighted.
- **Failure:**
  1. Add two probes, leave the first unnamed, and click Save. "Name is required." appears on probe 1.
  2. Delete probe 1. The named probe (now index 0) shows "Name is required.", and its input has `aria-invalid="true"`.
- **Fix:** After each remove, re-validate when errors are showing: `if (Object.keys(errors).length && cfg) errors = validate(cfg);`. Alternatively, key errors by a stable row id.

## Rejected candidates (checked and dropped)

- **AttachmentsPanel list load without a story guard:** OverviewTab, which hosts every AttachmentsPanel, is `{#key product.selectedId}` (`ProductPage.svelte:907`). A stale response writes into an unmounted instance.
- **`AttachmentUrls.load` subscribing a caller's effect:** both callers invoke it after an `await` (AttachmentsPanel `loadAttachments`) or inside `untrack` (OverviewTab `:388`).
- **PoliciesTab `evalServerId` surviving a workspace switch:** the tab lives inside the modal Rules drawer, which traps focus. A workspace switch with the drawer open is not a realistic path.
- **Scheduled Tasks resync reloading runs of other workspaces' expanded tasks** (`events/routes/automation.ts:29`): extra requests only, with no wrong state. This belongs to the perf lens.
- **MonitorSettings errors surviving a cluster switch:** MonitorCluster is `{#key monitorCluster.id}` (`KubernetesPage.svelte:144`).
- **Kart/Shooter context-loss remount for a guest with no local state:** the guest assigns `state = v` on every snapshot frame, so the remount boots from it.
- **WidgetCard blank after an aborted first run:** the effect re-runs on `active`, and an abort never stamps `ranKey`/`okAt`.
- **Duplicate keys in DocsAgentsView `changed_paths`/`written`, MultiRun `candidates`, CreatePr `reviewers` and ErrorPanel `suggestions`:** each producer de-duplicates or emits one entry per unique row.
