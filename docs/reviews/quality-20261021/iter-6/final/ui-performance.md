# Quality iteration 6 (final): ui-performance lens

The reviewer worked read-only with grep, sed, cat and git. It ran no cargo, npm, build, e2e or test
commands.

- Worktree: `fix/quality-iter6-20261023` @ 2065f00e9 (merge base with `origin/main`: 02a64019b).
- Excluded: `target/`, `node_modules/` and `docs/reviews/`.
- Scope: re-verification of the wave-0 findings, then a full pass over the UI runtime. That pass
  covered the 528 UI files this iteration changed (`git diff origin/main --stat -- ui/`) and the
  wave-0 areas.

## Score

Rubric: start at 10.0. Each confirmed open finding subtracts 1.0 (blocker), 0.3 (major), 0.1 (minor)
or 0.03 (nit).

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 1 | 0.10 |
| nit | 1 | 0.03 |
| **Total** | 2 | **0.13** |

**Score: 10.0 − 0.10 − 0.03 = 9.87 / 10**

Both wave-0 findings are fixed at every listed location. The two open findings are new. The fix
waves introduced both, and neither is a leak or continuous cost:

- **6f-01:** the git store now bumps `refsRev` on a pull. A graph that also requests its own
  refresh then reads the history twice.
- **6f-02:** the classroom's new on-demand loop restarts its 30 fps accumulator from zero on every
  wake. Corridor drag, wheel and hover therefore render later and less often than before.

## Previous-finding status (wave 0)

| id | Wave-0 severity | Status | Evidence |
|---|---|---|---|
| ui-performance-6-01 | minor | **Fixed** (all 3 locations) | `lib/three/stage.ts:79-133`: `demandLoop` re-arms only when `frame()` returns true and `kick()` restarts it. `watchVisibility` (`:45-71`) is now adopted. **School:** `scene.ts:500-586` returns `schoolHasWork(...)` (`scene/loop.ts:19-21`: tween, keys, open room without reduced motion, kicks, doors moving). Every dirty-only path (`openDoor` `:362-369`, camera `touch` `:351-357`, `update`, `setHover`, `setSelected`, `setScreen`, resize, theme) calls `kick()`. `watchVisibility(host, …, '120px 0px')` (`:599-609`) stops the loop off-screen. **Embed:** `embed.ts:470-510` returns `embedHasWork(...)` (`studio3d/loop.ts`). `watchVisibility` (`:429-438`), orbit `change` (`:404-407`) and the pointer handlers kick. **`Scene3DViewport`:** `invalidate` = `dirty = true; loop.kick()` (`:141-145`). `step` returns `viewportHasWork` (tween, camTween, turntable && !dragging) (`:255-273`). `watchVisibility` is in an `$effect` (`:895-903`). An idle viewport schedules no rAF. |
| ui-performance-6-02 | minor | **Fixed** | `studio3d/sources.ts:32-140`: a ref-counted LRU (`MODEL_CACHE_MAX = 12`). Unheld entries are revoked on eviction (`trimModelCache`). An entry evicted while held is orphaned and revoked on its last release. The key is `<artifact>@<resolved version id>` (`resolveKey`), so `@latest` / `@approved` follow new versions and a version is cached once. `modelScope()` is held and released by `embed.ts:192/527`, `ArtifactStage.svelte:201-202` (keyed per artifact in `ArtifactView`/`CompareModal`) and `Studio3D.svelte:194-200`. `releaseModelCache()` runs on `DesignHallPage` destroy (`:37`). |

## Areas covered

- **Re-verification:** the table above. It covers each loop's work predicate, every `dirty = true`
  writer in `school/scene.ts` (checked to reach a `kick()`), and the async `fly` → `touch` path.
  It also covers context-loss remount teardown in `embed.ts` (loop stopped, observers and cleanups
  run, renderer released, models released).
- **Git graph split (arch-1-09):**
  - `GraphView.svelte` with `graph/graphViewport` (rAF-coalesced scroll, fixed-pitch window via
    the shared `listRange`, ResizeObserver cleanup).
  - `graphData` (`$state.raw` payloads, `setRefs` no-op guard, gen-guarded `applyGraph`,
    splice-first `fetchGraph`).
  - `graphSync` (refsRev / bridge effects, hidden-graph parking, 4 s background throttle). This is
    where 6f-01 was found.
  - `graphRefIndex` (`laneNames` identity-stable, so the layout reruns only on real renames;
    `chipsFor` unchanged from main) and `graphSelection` (spine derived per selection).
  - `RefSidebar` / `RefSideLists` (capped `plan` slices, as on main).
- **Shared windowing:** `lib/listWindow.svelte.ts` and `TableWindow`.
  - The `attach` listener and ResizeObserver are removed. The rAF is cancelled.
  - `#sync` is untracked, so a re-measure doesn't re-attach.
  - The find provider is per window.
  - Adopters: `FileTree` (roving `tabRow` is O(n) per scroll frame, which is negligible), WIP
    panel and S3/SFTP tables.
- **Database store split:**
  - `workbenchPersist` (300 ms debounced tab persistence, unchanged-JSON skip, flush on hide).
  - `queryRuns` reattach loop (one per tab, superseded check).
  - `connections` keep-alive (`pollWhileVisible`, stopped when off).
  - `ResultsGrid`: dialogs moved from `{#await import()}` to module-level `lazyComponent` +
    `LazyMount`, so one promise per chunk.
- **apiClient store split:**
  - `tabs` (debounced localStorage writes, pagehide flush on the singleton).
  - `drafts` `isDirty` (bounded; as in wave 0).
  - `automations` run polling (timers per run, backoff).
  - `collectionsSync` (cold path).
- **Workflows split:** `runSync.svelte.ts`.
  - The WS fast path, single-flight rev-guarded `refetchRun`, and abort on hidden or run switch.
  - `liveQuery` safety poll only while live.
  - The approval preview, which aborts on supersede.
  - Final output fetched once per run (`finalOutputRunId` guard, untracked).
  - The `WorkflowsPage` inspector (1 s throttle while running, `sharedNodeBodies` cache).
- **Product `OverviewTab` split:**
  - `JiraIssueSection` (load / editmeta effects gated on error, no retry loop).
  - `StoryTranscripts` and `DraftEditor` (no effects or timers).
  - The body-token resolver keeps its `lastResolvedInput` guard.
  - `MockupViewer` now debounces live Mermaid edits (200 ms), which is an improvement.
- **Classroom box:**
  - `classrooms/load.svelte.ts` (an aborted load is no longer stamped fresh).
  - `DoorBubbles`, `lossBudget`, and `crumbs` → `upTo`.
- **Announcer sweep:**
  - `lib/announce.svelte.ts`: one rAF per announce, the newest replaces a pending one, and edge
    detection.
  - About 40 `$effect(() => { if (x) announce(…) })` sites, checked to read booleans or
    edge-stable strings. None announces per progress tick: S3 transfer bars key on the name, the
    live browser on state, kart / shooter on derived booleans.
- **Entry-chunk hygiene:**
  - `router.svelte.ts` now imports `lib/rooms/invites.ts` (dependency-free) instead of
    `modules/rooms/*`.
  - `lib/uiCommands/home.ts` / `vault.ts` reach their modules through ports (`homePort`,
    `vaultPort`, `pagePort`) instead of static module imports.
- **Rooms / games:**
  - `ShooterGame` HUD split (`Knocked`, `MatchResults`, `PauseMenu`); `say()` → `announce`.
  - `KartGame` status edges, `GameMatch`, `RoomRecap`, vault `previewStore` `aria-describedby`.
- **Repo-wide audits repeated:**
  - Every `setInterval` file has a matching `clearInterval`, except the module singletons
    `now.svelte.ts` and `api/mock.ts`.
  - Every perpetual rAF loop: `termLatency.ts` is the opt-in debug HUD only; the kart and
    shooter loops run while a match is mounted.
  - New `addEventListener` sites in the diff are all paired: `listWindow`, `Scene3DViewport`
    `{@attach}` keydown, `graphSync` / `graphViewport` / `runSync` `visibilitychange` and
    matchMedia. The apiClient `pagehide` is a store singleton.
  - New `JSON.stringify` sites in the diff are cold paths, debounced persistence, or the
    pre-existing `setRefs` / `sameJson` guards that run once per refresh.

## Areas NOT covered

- **`lib/components/Terminal.svelte` internals:** the WebGL atlas and `termPark`. The terminal
  review owns them.
- **Per-tick cost of the shooter sim (bots, zombies) and `kart/sim.ts` / `cpu.ts` with 16 bots.**
  This needs a profiler.
- **AWS `Ec2View` / `RdsView` "All regions" tables:** still not windowed (`Ec2View.svelte:257`,
  `RdsView.svelte:194`). Instance counts were not measured, so this stays unreported, as in
  iterations 4 to 6.
- **The `WorkflowCanvas` drag/zoom path with hundreds of nodes.**
- **Bundle-level verification:** chunk sizes and the entry graph need a build, which the
  read-only rule excludes.
- **Runtime measurement:** heap over time, long tasks during agent bursts, the real idle wake-up
  rate after 6-01, and CPU/RAM against N concurrent agents. These need a running app.

## Findings

### ui-performance-6f-01 (minor): a pull from the git graph or the toolbar reads the whole history twice and throws the first read away

The architecture-6-04 fix moved the post-mutation contract into the git store: `git.pull` now
stores the status and bumps `refsRev`. Before this iteration only push and checkout did that. The
open graph re-syncs on that bump, but the two pull entry points still also ask for an explicit
refresh. Push and toolbar branch-create follow the same double pattern.

- **Locations:**
  - `ui/src/lib/stores/git.svelte.ts:725-733`. `pull` now calls `this.bumpRefs(repoId)`; on
    `origin/main` it was a bare `api.post`.
  - `ui/src/modules/git/graph/graphActions.svelte.ts:449-452`. `pullCurrent` does
    `await runPull(...)` and then `await refreshAfter()`, with no `adoptRefsRev()`. The checkout
    paths at `:375` / `:387` do call it.
  - `ui/src/modules/git/GitToolbar.svelte:102-103` (pull), `:151-152` (push) and `:171-173`
    (create branch). Each does `git.*` (bumps `refsRev`) and then `onrefresh?.()`, which becomes
    `gitBridge.refreshGraph` (`GitPage.svelte:356`, `RepoView.svelte:360`) and then
    `graphSync`'s bridge effect, then `h.refreshAfter()` (`graphSync.svelte.ts:60-68`).
- **Evidence (the order is deterministic):**
  1. `bumpRefs` schedules the `graphSync` refsRev effect (`graphSync.svelte.ts:48-59`) before the
     `await` continuations resume. So `requestResync` → `resyncRefs` issues `GET /refs` first
     (`:104-110`).
  2. The caller then runs `refreshAfter`. That is `Promise.all([git.refs, fetchGraph])` with a
     second `GET /refs` plus `log?all=true&limit=…`, `/stashes` and `/worktrees`
     (`graphData.svelte.ts:506-514`, `:541-575`).
  3. `refreshAfter` assigns refs only after the slower log returns. So `resyncRefs` still compares
     against the pre-pull refs, and `historyMoved` is true: a pull moves the branch tip, and a
     push moves the remote-tracking ref (`:475-492`).
  4. `resyncRefs` then calls `backgroundReload` → `reloadGraph` → a second `fetchGraph`. That
     bumps `loadGen`, so the first `fetchGraph`'s result is discarded by
     `if (g.gen !== loadGen) return` (`:577`).
  - **Net cost per pull (graph menu or toolbar) and per toolbar push:**
    - 2× `/refs`.
    - 2× `git log --all` page. If the splice fails, the daemon re-reads as many commits as the
      user had paged in.
    - 2× `/stashes`.
    - 2× `/worktrees`.
    - One full set is thrown away.
  - **Toolbar create-branch** costs a duplicate `/refs` plus `refreshAfter`'s unconditional log
    read. A new branch on a loaded commit does not count as moved.
  - **Pre-existing part:** the toolbar push and create-branch doubles existed before (the store
    already bumped there).
  - **New part:** both pull paths, which are new this iteration.
- **Fix:** one refresh per mutation. Either:
  - **Graph:** in `pullCurrent`, call `adoptRefsRev()` right after `runPull`, as the checkout
    paths do. Or drop the trailing `refreshAfter()` and let the bump re-sync, which already reads
    `/refs` first and the log only if history moved.
  - **Toolbar:** remove `onrefresh?.()` after `doPull`, `doPush` and `doCreateBranch`, because
    the store's bump is now the contract. Keep it for stash and pop, which do not bump.
  - **Or, at the sink:** in `graphSync`, have the bridge effect that runs `refreshAfter` also
    mark the current `refsRev` seen (`adoptRefsRev()` before `h.refreshAfter()`), so a bump and a
    bridge request in the same tick collapse into the one full refresh.

### ui-performance-6f-02 (nit): after the on-demand loop change, corridor drag, wheel and hover render at about 20 fps and at least 33 ms late

The new classroom loop resets its timing on every wake but keeps the 30 fps accumulator, so the
first frame after idle always waits two to three display frames.

- **Location:** `ui/src/modules/home/school/scene.ts:588-593` (`kick`) together with
  `:500-511` (`frame`).
- **Evidence:**
  - In the corridor, `schoolHasWork` is false right after a render (no tween, keys, room or
    doors), so the loop idles. `rig.drag` / `rig.wheel` move the camera directly with no damping
    (`scene/camera.ts:124-152`), so every input `kick()`s from idle.
  - `kick` sets `last = performance.now()`, and `acc` was zeroed by the previous render (`:513`).
  - In `frame`, `raw = max(0, now - last)` is about 0 on the first callback. `acc < FPS_MS`
    (33.3 ms, `scene/shared.ts:19`) returns `true` until about two more 60 Hz frames have passed
    (four on a 120 Hz display).
  - The render then returns false and idles, and the next pointermove restarts the count from 0.
  - **Result:**
    - A continuous corridor drag or wheel renders every third display frame (about 20 fps at
      60 Hz, about 24 fps at 120 Hz). The old always-on loop's carried-over accumulator gave a
      steady 30.
    - Every hover or drag response waits at least 33 ms instead of the next 30 fps boundary
      (about 16 ms on average).
  - The embed and `Scene3DViewport` loops draw on the first frame after a kick and are not
    affected. Rooms with motion run continuously and are not affected either.
- **Fix:**
  - Measure the cap from the last drawn frame, not from the wake. Keep `lastDraw` (set where
    `renderer.render` runs, `:576-583`).
  - In `kick()`, when `!loop.pending`, set `acc = Math.min(FPS_MS, performance.now() - lastDraw)`.
    An input after an idle stretch then draws on the very next frame, and inputs closer than
    33 ms stay capped at 30 fps.
  - Life steps are unaffected: the loop only idles where `life.step` does not run (the corridor,
    or reduced motion).
