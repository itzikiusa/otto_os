# Quality iteration 5 — lens: ui-performance

Reviewer: ui-performance lens. Read-only (grep/rg/sed/git only). No cargo, npm, build, e2e or test
runs.

- Worktree: `fix/quality-iter5-20261022` @ 8177c4d85.
- Excluded: `target/`, `node_modules/`, `docs/reviews/`. The room games
  (`ui/src/modules/rooms/games/**`) and the classroom (`ui/src/modules/home/school/**`,
  `ui/src/modules/home/boxes/ClassroomsBox.svelte` + `boxes/classrooms/*`) are IN scope this pass.
  Iterations 2–4 excluded them, so they got most of the time.

## Score

Rubric: start at 10.0. For each confirmed open finding, subtract 1.0 for a blocker, 0.3 for a major,
0.1 for a minor and 0.03 for a nit.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 2 | 0.20 |
| nit | 1 | 0.03 |
| **Total** | 4 | **0.53** |

**Score: 10.0 − 0.53 = 9.47 / 10**

Both iteration-4 findings are fixed. All four new findings are in the Home "Classrooms" box (Otto
School). Earlier passes excluded it, and it is one of the most-changed UI areas in the last three
weeks: `ClassroomsBox.svelte` changed by 3,141 lines, `school/scene.ts` by 2,656, and
`school/life.ts` by 1,004. The room games (kart, shooter) came out clean (details below).

## Previous-finding status (iteration 4)

| id | Iter-4 severity | Status | Evidence |
|---|---|---|---|
| ui-performance-4-01 | major | **Fixed** | `workbench/WorkbenchPage.svelte:88` `editorPath = workbench/${doc.id}`. The path is stable per doc and has no language suffix. The language goes in through `language={cmExtFor(lang)}` (`:695`). `CodeEditor.svelte:1140-1155` adds a live `language` effect that does `langCompartment.reconfigure(...)` and `ensureLang(livePath)`, with no `setState`, so the caret and the undo history survive. Detection is sticky: `sniffText` is snapshotted on open, when loading finishes, on paste (`sniffNow`), and after 400 ms idle (`:63-79`). |
| ui-performance-4-02 | minor | **Fixed** | One sniff serves both the label and `lang`: `detected = $derived(detectLanguage(doc.name, sniffText))`, then `lang = explicitLanguage(doc.language) ?? detected` (`:80-81`). It runs on the debounced snapshot, not on every keystroke. `detect.ts:79-90` `firstLine` is an `indexOf` walk. `SNIFF_PREFIX = 16 KB` and `SNIFF_LINES = 200` bound the content sniff (`:92-100`). `PlaceholdersPanel.svelte:17-30` scans a 200 ms debounced copy, and applies a language change at once. |

## Areas covered this iteration

- **Re-verification** of both iteration-4 findings (table above).
- **The classroom (new this pass):**
  - `ClassroomsBox.svelte`, end to end: the native-pane / IntersectionObserver / space-active gate,
    the `livePoll` session load, the `sessions` → `buildSchool` → `byId` / `queue` / `busiest`
    deriveds, the scene mount effect (context-loss remount budget), the `h.update` /
    `setActive` / `setTheme` / `setSelected` / `setHover` effects, the screen-poll and
    `pushScreen` effects, `placeOverlay` (per rendered frame), and the pointer, wheel and key
    listeners (all removed in the cleanup).
  - `school/scene.ts`: the ≤30 fps loop (`FPS_MS`), which renders only when dirty, cancels on
    `visibilitychange` / `setActive(false)`, and calls `destroy()` → `dispose` +
    `forceContextLoss`. Also `update()`, which repaints keyed labels (`paint.ts` plate, sign and
    board keys) and keys screens through `screenKey`.
  - `school/scene/pick.ts`: the raycast against hit meshes only.
  - `school/screens.ts` `pollScreens`: in-flight guard, hidden pause, `MAX_LIVE = 12`, 2 s.
  - `school/model.ts` `buildSchool`, `attentionQueue`, `busiestRoom`.
  - `boxes/classrooms/*`: SchoolList (paged at 100), NeedsYouQueue, HoverTag, KidCard.
  - `HomePage` / `HomeBox` zoom and space mounting.
- **Room games (new this pass):**
  - `GameMatch.svelte`: snapshot and input buffers are plain fields, not `$state`. The
    20 Hz guest snapshot path touches no runes. The buffer is capped at 4.
  - `client.ts` (`GameClient`): ping, deadline and retry timers, all cleared on `detach` and
    `dispose`. A `bufferedAmount` back-pressure guard.
  - `KartGame.svelte` / `ShooterGame.svelte`:
    - rAF loops cancelled in `stop()`.
    - The HUD copy is throttled to 80–90 ms into `$state.raw`.
    - The minimap's static layer is cached per track.
    - The renderers are dynamic imports.
    - `fullscreenchange` is removed and `audio.dispose()` is called.
  - `common/view.ts`: the ResizeObserver is disconnected, owned meshes are freed, and dispose
    calls `forceContextLoss`. `common/input.ts` keeps a disposer list. `common/audio.ts`.
  - `kart/render.ts` `frame()` and `shooter/render.ts` `frame()`: no per-frame `new`, `map` or
    `filter`.
  - Entry chunk: every static `three` import sits under `rooms/games/**`, and that tree is
    reached only through `RoomsRoute` → `lazyComponent(import(...GamesHub))` and
    `App.svelte`'s lazy `GameGuest`. The school imports `three` dynamically.
- **Diff since iteration 4 (6cff08270..8177c4d85, 236 UI files):**
  - New code with no runtime cost:
    - The MCP profile picker, `SessionMcpProfiles`, `ProfilesTab` and `ProfileEditor` (small
      lists; `mcpProfiles.load()` dedupes on `loaded` / `#inflight`).
    - `ConfluencePagePicker`: a 250 ms debounced search with an abort and a seq guard.
    - `ConfluenceSpaceSelect`: the effect depends on `accountId` only.
    - The database store split (`dashboards.svelte.ts` has no timers).
  - Changed code checked:
    - The ResultsGrid run-announce effect (reads `running`, untracked body).
    - GraphView `newRefTips` (a full reload only when a new tip is outside the fresh page).
    - The RoomChat and RoomParticipants refocus effects.
    - `SplitNode` keyed retry.
- **Repo-wide audits repeated:**
  - All `setInterval` sites (unchanged from iteration 4).
  - `window` / `document` `addEventListener` without a matching remove. Every unpaired one is a
    module-level singleton: `d2.ts` pagehide, `dialogFocus.ts` capture listeners,
    `tickCoalescer` one-shot hook, `now.svelte.ts`, `events.svelte.ts`, `main.ts`.

## Areas NOT covered yet (for the next iteration)

- `school/life.ts` (708 lines): the per-frame ambient-life director, read only at its call sites.
  The same goes for `school/scene/camera.ts` (the walk and fly rig).
- `rooms/games/shooter/bots.ts`, `zombies.ts`, `sim/*` and `kart/sim.ts` / `cpu.ts`: the
  per-tick simulation cost with 16 bots. This needs a profiler, not a read.
- `rooms/games/common/textures.ts`: one-time procedural textures at load (only the load path).
- `lib/components/Terminal.svelte`: the WebGL atlas and the `termPark` re-adopt (owned by the
  terminal review).
- The `WorkflowsPage.svelte` inspector on runs with hundreds of nodes. AWS `Ec2View` / `RdsView`
  "All regions" tables are not windowed; the instance counts behind them were not measured, so
  this is not reported.
- `git/GraphView.svelte` beyond the refs and reload paths.
- Runtime measurement needs a running app, which the read-only rule excludes. That covers heap
  over time, long tasks during agent bursts, and the actual `/sessions/{id}/screen` request rate
  of finding 5-01.

## Findings

### ui-performance-5-01 — major — The classroom's live-screen poller restarts (aborting its batch and re-fetching every visible screen at once) on every session status event, not every 2 s

- **Files:**
  - `ui/src/modules/home/boxes/ClassroomsBox.svelte:126-133, 135-148, 272-275, 284-306`.
  - `ui/src/lib/stores/workspace.svelte.ts:1954-1972, 1919-1940`.
  - `ui/src/modules/home/school/screens.ts:214-278`.
  - `ui/src/modules/home/school/model.ts:340-400`.
- **Evidence:**
  - The poll effect is:

    ```ts
    $effect(() => {
      const h = handle;
      if (!h || !openRoomModel || !onScreen) return;
      const p = pollScreens({ ... });
      return () => p.stop();
    });
    ```

    It depends on `openRoomModel` (`:291`).
  - `openRoomModel = $derived.by(() => … school.rooms.find((r) => r.id === v.roomId))` (`:272-275`).
    `school = $derived(buildSchool({ sessions, statusOf: (id) => ws.statusMap[id], needsYou: …, stale: events.state !== 'connected', … }))`
    (`:135-147`).
  - `buildSchool` allocates fresh `Room` and `Kid` objects on every call (`model.ts:340-400`), so
    the derived compares unequal on every rebuild.
  - Every `session_status` event writes `ws.statusMap[id]` (`workspace.svelte.ts:1966`). It also
    queues `{ status, at: new Date().toISOString() }`. `flushStatus` writes a NEW `this.sessions`
    and `this.otherWsSessions` array once per frame (`:1919-1940`). Both are inputs of `sessions`
    / `school`.
  - Net effect, while a room is open and on screen: every agent status flip (working ↔ idle, needs
    you, exit) rebuilds `school` and changes `openRoomModel`. That re-runs the effect, and the
    cleanup runs `p.stop()`, which aborts the in-flight batch (`screens.ts:266-270`). The new
    poller's `schedule(0)` (`:264`) immediately fires up to `MAX_LIVE = 12`
    `GET /sessions/{id}/screen` requests.
  - The intended cadence is one batch per `POLL_MS = 2000`. With N busy agents the real rate
    becomes one batch per status event, which scales with agent count: the parallel-load case.
    Requests that are aborted mid-flight are wasted daemon screen renders. When events arrive
    faster than a batch round-trip, feeds stop landing at all.
  - The same identity churn also re-runs the `pushScreen` effect (`:284-288`) for every kid in the
    room on every event. It calls `setScreen` → `applyScreen` → `screenKey`, which runs
    `visibleLines` over each feed, plus a `kick()`. `h.update(s)` already re-applies every
    screen (`scene.ts:652`), so each kid's key is built twice per event.
- **Fix:**
  - Key the poller on the room's identity, not on the room object, for example
    `const openRoomId = $derived(view.kind === 'corridor' ? null : view.roomId)`. The effect then
    reads `openRoomId`, and `byId` stays read lazily inside `wanted` and `onFeed` (as it is now).
    Status changes then reach the screens through the next 2 s tick, and through `h.update`'s
    existing `applyScreen` loop.
  - Drop the `pushScreen` effect's dependency on the room object the same way. Only push when a
    kid's `title` / `pose` / `character` actually changed, or rely on `h.update`, which already
    calls `applyScreen` for every known screen.
  - Optional: if a status change should refresh a screen sooner, call the poller's `now()` at
    most once per `POLL_MS`, and do not recreate the poller.

### ui-performance-5-02 — minor — The Classrooms box re-fetches up to 1,000 sessions every time it scrolls back into view, its space slides back, or a zoom ends (it is the only box not using `freshness()`)

- **Files:** `ui/src/modules/home/boxes/ClassroomsBox.svelte:76-85, 119-125`;
  `ui/src/modules/home/boxes/poll.ts:15-46`; `ui/src/modules/home/HomePage.svelte:264-270, 307`.
- **Evidence:**
  - `onScreen = $derived(spaceActive && nativeVisible && inView)`. `inView` comes from an
    IntersectionObserver with the default threshold 0 (`:76-85`), so it flips each time the box
    crosses the viewport edge.
  - The load effect is
    `$effect(() => { if (!onScreen) return; const p = livePoll(load, 30_000, [...], { immediate: true }); … })`
    (`:119-125`). Every false → true flip builds a new poller that runs `load` at once. `load`
    is `GET /sessions?archived=false&limit=1000` plus `GET /sessions?archived=true&limit=60`,
    each followed by a `visibleOnThisDevice` filter and a full `buildSchool`.
  - `poll.ts:15-23` says why the other boxes don't do this: every Home space stays mounted, and
    `freshness()` exists so that "a slide back no longer re-fetches every box". `K8sBox`,
    `InsightsBox`, `MissionControlBox` and `UsageBox` all pass `immediate: fresh.start(key, ms)`.
    ClassroomsBox hard-codes `immediate: true`.
  - Zoom also triggers it. `HomePage.svelte:307` passes `active={on && !home.zoomedId}`, so the
    grid copy goes inactive while zoomed and refetches on un-zoom.
  - The box is event-fed (session events re-run `load`), so data is never stale for more than one
    event while it is on screen.
- **Fix:** wrap `load` in `freshness()`, as the sibling boxes do:
  `const fresh = freshness(load); … livePoll(fresh.run, 30_000, [...], { immediate: fresh.start('', 30_000) })`.
  Optionally give the IntersectionObserver a small `rootMargin` or threshold, so that
  edge-hovering scrolls do not flap `onScreen`. That flapping also toggles `setActive` and the
  screen poller.

### ui-performance-5-03 — minor — Zooming the Classrooms box builds a second complete 3D school (a new WebGL context, all `.glb` assets re-parsed, PMREM) while the grid copy keeps its own scene alive underneath

- **Files:** `ui/src/modules/home/HomePage.svelte:260-270, 307`;
  `ui/src/modules/home/boxes/ClassroomsBox.svelte:166-230`;
  `ui/src/modules/home/school/scene.ts:66-80, 85-90`;
  `ui/src/modules/home/school/assets.ts:30-60`.
- **Evidence:**
  - While zoomed, HomePage mounts a second `<HomeBox box={home.zoomed} … zoomed />` (`:260-263`).
    The grid "stays mounted underneath, paused" (`:264-270`), with `active=false` (`:307`).
  - For ClassroomsBox, "paused" means `handle.setActive(false)`: the rAF loop stops, but the
    renderer, its GPU buffers and textures, the env render target and the loaded GLTFs all stay
    allocated (`scene.ts:739-749`). Only `destroy()` frees them.
  - The zoomed copy runs its own scene-mount effect (`ClassroomsBox.svelte:166`).
    `mountSchool` → `loadSchoolAssets` parses `school-kit.glb`, `headmaster-otto.glb` and six
    `kid-*.glb` again with a fresh `GLTFLoader` (`assets.ts:30-45`); nothing caches them at
    module level. It also loads `env.hdr` and builds a new `WebGLRenderer` and PMREM environment
    (`createStage`).
  - Each zoom therefore pays a full cold start (download from the HTTP cache, then parse, upload
    and shader compile), and holds two live WebGL contexts. The box's own context-loss message
    names this pressure: "lost its graphics context (too many 3D views open at once)"
    (`ClassroomsBox.svelte:221`). Zoom is a header double-click and a toolbar button
    (`HomeBox.svelte:213, 219`).
- **Fix:** do one of these two:
  - While its box is the zoomed one, have the grid copy release its scene: pass `zoomedAway`
    (`home.zoomedId === box.id`) from HomeBox, and make the mount effect return early on it.
  - Or keep one scene and memoise the parsed assets at module level (a ref-counted
    `loadSchoolAssets` promise, disposed when the last scene goes). A second mount then only
    clones from the cache instead of re-parsing every `.glb`.

  Either removes the second context while zoomed.

### ui-performance-5-04 — nit — The classroom's relative-time labels subscribe to the 1 s clock and defeat `rel()`'s minute-grain subscription

- **Files:** `ui/src/modules/home/boxes/classrooms/SchoolList.svelte:48, 78`;
  `KidCard.svelte:34`; `HoverTag.svelte:25`; `ui/src/lib/stores/now.svelte.ts:72-86`.
- **Evidence:**
  - Every label is written as `now() ? relTime(k.lastActiveAt) : ''`. `now()` reads `clock.ms`, so
    every row's expression re-runs each second (`kidAriaLabel` → `kidLines` → `join`).
  - `rel()` (which `relTime` wraps) already subscribes as finely as the label can change. Rows
    more than an hour old follow `minuteMs` only (`now.svelte.ts:76-86`, the A3 optimisation:
    "every one of them used to re-run its label every second"). The redundant `now()` read undoes
    it.
  - SchoolList is mounted in 3D mode too, as the accessible list (`ClassroomsBox.svelte:685`), in
    every Home space that holds the box. It keeps ticking while the box is scrolled away or its
    space is inactive, since the clock pauses only when the document is hidden.
  - The cost is a few dozen small string builds per second. It is bounded by `LIST_PAGE_SIZE =
    100` per room, so this is a nit.
- **Fix:** drop the `now() ? … : ''` guard and call `relTime(k.lastActiveAt)` directly; it is
  already reactive. Keep `now()` only where the value itself needs the second
  (`isAsleep(hoverKid, now())` in HoverTag).
