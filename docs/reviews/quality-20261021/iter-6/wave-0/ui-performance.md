# Quality iteration 6 (wave 0): ui-performance lens

The reviewer worked read-only with grep, sed, cat and git. It ran no cargo, npm, build, e2e or test
commands.

- Worktree: `fix/quality-iter6-20261023` @ 02a64019b.
- Excluded: `target/`, `node_modules/` and `docs/reviews/`.
- In scope: the room games and the classroom (`ui/src/modules/home/school/**`,
  `ui/src/modules/home/boxes/ClassroomsBox.svelte`, `boxes/classrooms/*`).

## Score

Rubric: start at 10.0. Each confirmed open finding subtracts 1.0 (blocker), 0.3 (major), 0.1 (minor)
or 0.03 (nit).

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 2 | 0.20 |
| nit | 0 | 0.00 |
| **Total** | 2 | **0.20** |

**Score: 10.0 − 0.20 = 9.80 / 10**

All four iteration-5 findings are fixed. Both new findings are in the three.js surfaces: the
classroom scene, the Design Hall studio and its embed, and the shared `Scene3DViewport`. Earlier
passes read the start and stop of those render loops, but not what the loops do while idle. They
also did not read the model-bytes cache.

## Previous-finding status (iteration 5)

| id | Iter-5 severity | Status | Evidence |
|---|---|---|---|
| ui-performance-5-01 | major | **Fixed** | `ClassroomsBox.svelte:282` `openRoomId = $derived(view.kind === 'corridor' ? null : view.roomId)`. The poller effect (`:304-331`) reads only `handle`, `openRoomId` and `onScreen`. `byId` and `hover` are read inside the callbacks, so they are not tracked. `pushScreen` runs off `openRoomScreens = $derived(roomScreensKey(openRoomModel))` (`:285`, `screens.ts:290-294`), a string that changes only when a kid's title, provider, character or pose changes, and its body is `untrack`ed (`:294-302`). Status nudges go through `throttledNow(p, POLL_MS)` (`screens.ts:297-305`), which caps them at one per 2 s. |
| ui-performance-5-02 | minor | **Fixed** | `const fresh = freshness(load)` and `livePoll(fresh.run, 30_000, [...], { immediate: fresh.start('', 30_000) })` (`ClassroomsBox.svelte:125-132`). The IntersectionObserver has `rootMargin: '120px 0px'` (`:82-86`), so the box no longer flaps at the viewport edge. |
| ui-performance-5-03 | minor | **Fixed** | `HomeBox.svelte:79` `zoomedAway = $derived(!zoomed && home.zoomedId === box.id)` is passed to the body (`:231`). The scene-mount effect returns early on it (`ClassroomsBox.svelte:190`), and its cleanup destroys the grid copy's scene, so two WebGL schools never coexist. |
| ui-performance-5-04 | nit | **Fixed** | The `now() ? relTime(…) : ''` guards are gone from `SchoolList`, `KidCard` and `HoverTag`. The one remaining `now()` read is `isAsleep(hoverKid, now())` (`HoverTag.svelte:25`), which needs the second, as the fix suggested. |

## Areas covered this iteration

- **Re-verification** of the four iteration-5 findings (table above).
- **The diff since the iteration-5 review base** (`8177c4d85..HEAD`, 186 UI files):
  - **Root `App.svelte`:** the daemon-supervisor `onMount` (a disposed guard; unlisten on cleanup).
    `daemonState.ts` is self-contained and small.
  - **Shell overlays:** moved to module-level `lazyComponent`s (`shell/overlays.svelte.ts`) and
    rendered through `LazyMount quiet`. One promise per chunk, so re-renders never re-import.
  - **`lib/relaySocket.ts`:** the heartbeat interval, silence and retry timers, and the
    `online` / `visibilitychange` listeners are all cleared in `detach` / `dispose` / `unlisten`.
    `bufferedAmount` back-pressure is applied.
  - **`lib/three/stage.ts`:** helpers only; `watchVisibility` is not adopted yet (see 6-01).
  - **Database store split:**
    - `history.svelte.ts`: `$state.raw`, a request guard, a 1,000 cap.
    - `savedQueries`, `assist`, `dashboards`: `noteWidgetColumns` is a no-op when nothing
      changed.
    - `WidgetCard` / `Dashboards`: `resultColNames` is read only while editing.
  - **`product/attachmentUrls.svelte.ts`:** generation guard, in-flight dedupe, revoke on
    reset/forget/dispose. `OverviewTab` body-token resolve keeps its `lastResolvedInput` guard.
  - **`SessionsBox`:** the `LoadState` wrap. **`TrayPage`:** the daemon listener. **Terminal:**
    the resize-step extraction and the removed injection effect.
  - **`QueryBuilder`:** focus restore. **`MonitorSettings`:** inline errors. Neither has any
    runtime cost.
- **Classroom, deeper (iteration-5 gap):**
  - The `school/scene.ts` frame loop end to end (`:483-578`), `stepDoors`, `openDoor`,
    `setActive` and `visibilitychange`.
  - `scene/camera.ts` `stepInput` / `drag` / `wheel`.
  - `life.ts` `step` / `kidView` / `pickNext` / `wanderTarget` (per-frame cost is
    O(kids in the open room), with small allocations, at ≤30 fps).
  - `screens.ts` `pollScreens`, `roomScreensKey` and `throttledNow`.
- **Design Hall 3D (not covered before):**
  - `studio3d/embed.ts` (mount, IntersectionObserver, frame loop, destroy).
  - `studio3d/sources.ts` (`resolveModelRef`, `loadBrandKit`).
  - `lib/design/scene3d/Scene3DViewport.svelte` (loop, invalidate, camera tween).
  - `SiteCanvas` live-embed reconciliation.
  - `ArtifactThumb`'s evicting blob cache, plus the blob-URL lifecycles in `ArtifactView`,
    `CompareModal`, `LogosSection` and `SiteStudio`.
  - `MockupLivePreview` and `mockup-assist.svelte.ts` (one `liveContent` write per file write, not
    per token).
- **Rooms:**
  - `RoomPage` event application, `room-state.ts` `applyRoomEvent` (unknown events return the same
    object).
  - `RoomAnnotations` (sends only on stroke end; one expiry timer).
  - `RoomChat` (keyed list; scroll follow).
- **Workflows (iteration-2/3/4 gap):**
  - `WorkflowsPage` inspector body fetch (1 s throttle for running nodes, abort on supersede,
    `sharedNodeBodies` cache).
  - The approval-preview effect, single-flight `refetchRun`, the WS fast path, and the
    `liveQuery` safety poll.
  - The final-output panel fetched once per run.
  - `RunSteps`' 1 s ticker, which runs only while a step runs and is read only by running rows.
- **Repo-wide audits repeated:**
  - Every `setInterval` file has a matching `clearInterval`, except the module singletons
    `now.svelte.ts` and `api/mock.ts`.
  - Every component-level `addEventListener` without a paired remove was traced. The cases are
    `GridView` editor inputs (removed with the node), `PluginFrame` iframe window, `QueryEditor`
    abort-signal listeners (`once`), the `ShooterGame` canvas (removed with the canvas),
    `room-media` track `ended` (`once`), `Scene3DViewport` orbit/gizmo (disposed with the
    controls), `runProgress` (`once`) and `PreviewModal`'s iframe document. None leaks.
  - Search inputs bound to state across 39 files: every one that fetches is debounced and aborts
    (`HistoryList`, `JiraIssuePicker`, `HistoryPage`, `ConfluencePagePicker`). The rest filter
    bounded client lists.
  - `JSON.stringify` inside deriveds/effects: none on a hot path. (`ApiPage`'s per-tab `isDirty`
    runs ×3 per tab render, but only the active tab re-evaluates on a keystroke. It is bounded
    and not reported.)
  - `createObjectURL` sites (31): all revoked or download-and-revoke, except the deliberate
    unrevoked pop-out URL in `InsightsPage.svelte:413` (one per click, needed by the new
    window) and finding 6-02.
  - Lists: `brokers` TopicsTab (paged 50), GroupsTab (`TableWindow`), TopicDetail (windowed,
    `TAIL_CAP`), `aws/LogsView` (windowed ring), `git/GraphView` (row windowing, rAF-coalesced
    scroll), `git/WipPanel` (windowed rows), `swarm/KanbanBoard` (one-pass column map,
    `DONE_CAP`), `agents/history/HistoryPage` (250 ms debounced server search).

## Areas NOT covered yet (for the next iteration)

- **`lib/components/Terminal.svelte` internals:** the WebGL atlas and `termPark` re-adopt. The
  dedicated terminal review owns them.
- **Per-tick cost of `rooms/games/shooter` bots/zombies/sim and `kart/sim.ts` / `cpu.ts` with 16
  bots:** the code reads as linear per bot, but this needs a profiler.
- **AWS `Ec2View` / `RdsView` "All regions" tables:** still not windowed (`Ec2View.svelte:252`).
  The instance counts were not measured, so this stays unreported, as in iterations 4 and 5.
- **The `WorkflowCanvas` drag/zoom path on graphs with hundreds of nodes.**
- **Bundle-level verification:** the entry and shell chunk graphs. `TrayPage` →
  `workspace.svelte` is a static import of the root `App.svelte`, but the chunk sizes need a
  build, which the read-only rule excludes.
- **Runtime measurement:** heap over time, long tasks during agent bursts, and the actual idle
  wake-up rate of finding 6-01. This needs a running app.

## Findings

### ui-performance-6-01 (minor): three.js render loops re-arm `requestAnimationFrame` 60×/s while idle, and two of them also while off-screen

The repo has already settled this rule. `vault/GraphView.svelte:196-209` says: "Frames are
requested only when something changed — an idle graph schedules no rAF at all (a perpetual 'check
dirty' loop woke WebContent 60×/s for nothing)". Three 3D surfaces still run that perpetual
check-dirty loop.

- **Locations:**
  1. `ui/src/modules/home/school/scene.ts:493-578`. `frame()` always ends with
     `raf = requestAnimationFrame(frame)` (`:577`), even when it rendered nothing.
  2. `ui/src/modules/design-hall/studio3d/embed.ts:400-435`. The loop re-arms first (`:402`), then
     returns when `!visible` (`:405`) or `!needsFrame` (`:427`).
  3. `ui/src/lib/design/scene3d/Scene3DViewport.svelte:248-266`. The loop re-arms first (`:250`)
     and returns when `!dirty` (`:263`). `invalidate` only sets `dirty = true` (`:139-141`).
     There is no off-screen or hidden pause at all; `grep visibilitychange|IntersectionObserver`
     finds nothing in the file.
- **Evidence:**
  - **School:** the box is a Home dashboard widget, so it is the surface left open all day.
    - In the corridor (`openRoom === null`), the frame does only `stepInput` (a no-op with no keys
      held, `camera.ts:98-99`) and `stepDoors` (a no-op once the doors settle). `dirty` stays
      false, so nothing renders, yet `:577` schedules the next frame.
    - The same happens inside a room under reduced motion (`else if (dirty)` at `:540`).
    - The loop stops only on `setActive(false)` or a hidden document (`:495`, `:745-750`,
      `:584-592`).
    - The infrastructure to stop is already there. `kick()` (`:580-584`) restarts the loop, and
      every input path (pointer, drag, wheel, select, enter or leave a room, `setScreen`) already
      calls it (`:596-744`). The loop simply never takes the stop branch.
  - **Studio embed:** the file header promises "Off-screen embeds stop rendering" (`:19-20`). They
    stop rendering, but each one keeps a 60 Hz rAF callback alive for as long as the Site Studio
    canvas holds it. `SiteCanvas.svelte:365-388` mounts one per 3D block, so N embeds mean N
    callbacks per frame.
  - **`Scene3DViewport`:** this is the Design Hall artifact view, the Studio3D editor, the
    product `DesignArena` and `MockupLivePreview`. An idle viewport, with nothing dragged, no
    tween and no turntable, still runs `stepTween` / `stepCamera` and a dirty check 60×/s while
    the page is open.
  - **Why it matters in Otto in particular:** every Otto webview disables WebKit's background
    throttling. See `apps/desktop/src-tauri/src/throttle.rs:6-14`
    (`BackgroundThrottlingPolicy::Disabled`): "once WebKit decides a window is occluded … those
    clamp to ~1 s". An occluded or unfocused Otto window therefore keeps all of these loops at
    full display rate. The same file justifies that choice with "the UI's own pollers already
    pause on `visibilitychange`", which these loops do not honour beyond `document.hidden`. With
    more agents running, the pattern multiplies the main-thread wake-ups the
    parallel-agent perf rule measures.
- **Fix (one pattern for all three):** re-arm only while there is work, and let the existing
  `kick()` / `invalidate()` restart the loop.
  - **`school/scene.ts`:**
    - At the end of `frame()`, schedule the next frame only if
      `cam.tween || keys.size > 0 || (openRoom && !reduced) || kicks.size > 0 || doorsMoving`.
      `doorsMoving` is true when `stepDoors` changed an angle. Otherwise leave `raf = 0`.
    - Make `openDoor` (`:358-364`) and the other places that only set `dirty = true` call
      `kick()`. The `key()` handler (`:718-725`) already does, so held keys keep the loop
      alive through `keys.size > 0`.
  - **`embed.ts`:**
    - Move `raf = requestAnimationFrame(frame)` to the end of `frame`.
    - Re-arm only when `needsFrame || tween || (opts.autoRotate && !reduced) || tilt is still
      converging || controls damping`.
    - In the IntersectionObserver callback (`:367-369`) and the pointer handlers, start a frame
      when one is not pending, instead of only setting `needsFrame`.
  - **`Scene3DViewport.svelte`:** make `invalidate` request a frame
    (`dirty = true; if (!raf) raf = requestAnimationFrame(loop)`). Re-arm at the end of `loop`
    only while `tween || camTween || (turntable && !dragging)`. Gizmo, orbit `change` and
    reconcile already call `invalidate`.
  - **All three:** adopt the unused `lib/three/stage.ts` `watchVisibility` helper, so the
    off-screen and hidden pause is one shared implementation (this also closes the "not adopted"
    remainder of architecture-5-05).

### ui-performance-6-02 (minor): the Design Hall 3D model cache keeps every GLB/glTF blob URL for the whole session and never revokes it

- **Files:**
  - `ui/src/modules/design-hall/studio3d/sources.ts:32-64`.
  - `ui/src/lib/api/design.ts:209-211` (`fetchContent` creates the blob URL).
  - Callers: `embed.ts:297`, `ArtifactStage.svelte:196-197`, `Studio3D.svelte:658`.
- **Evidence:**
  - The cache is `const modelCache = new Map<string, Promise<string>>()` at module level. On a
    miss, `resolveModelRef(ref)` stores `loadModel(ref)`, and that resolves to
    `fetchContent(...).blobUrl`, which is `URL.createObjectURL(blob)` (`design.ts:210-211`), or to
    `authedBlobUrl(...)` for a legacy product attachment.
  - Only a rejected promise is ever deleted (`:43`). No code path clears the map or calls
    `URL.revokeObjectURL` on its entries: `grep -rn modelCache ui/src` hits only `sources.ts:32-43`.
  - Each entry pins the full model bytes in the WebContent process until the app reloads. A GLB is
    typically MBs to tens of MBs. Otto is a long-running desktop app, and every 3D artifact,
    version (`@vN`) or embed the user opens adds an entry. Browsing Design Hall versions, or Site
    Studio pages with 3D blocks, therefore grows memory without bound.
  - The cache key includes the selector, so the same artifact at `@latest`, `@approved` and `@v3`
    is held up to three times. The `@latest` entry also never refreshes ("`@latest` re-resolves on
    reload" means a full page reload).
  - The sibling caches already have bounds:
    - `product.svelte.ts:1011-1034` `blobCache` revokes everything in `revokeBlobCache()` on
      teardown and per id in `invalidateBlobUrl`.
    - `ArtifactThumb.svelte:48-53` evicts and revokes its blob URLs.
- **Fix:**
  - Give `modelCache` a bound, for example a small LRU of 8 to 16 refs that revokes the evicted
    URL once no live scene holds it. Ref-count by `acquire`/`release` from `embed.destroy()`,
    `ArtifactStage` and Studio3D teardown.
  - Or at minimum add a `releaseModelCache()` that revokes everything, and call it from the
    Design Hall module teardown, as `product.teardown()` does.
  - Key `@latest` entries with the resolved `versionId` (which `fetchContent` already returns), so
    a new version replaces and revokes the old blob instead of adding to the cache.
