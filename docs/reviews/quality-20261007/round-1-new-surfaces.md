# Round 1 — newly pulled School, session screen and Team Performance

**Verdict: Approve with fixes.** Baseline `196048df`, additions since `922ae483`; October 7, 2026. **Performance: 7.8/10, provisional; correctness: 8.0/10, provisional.** New findings: 0 blocker, 2 major, 2 minor; one independently confirmed cross-reference to UX-04, excluded from the new-finding count. These scores cover the sampled paths below, not the entire plugin or app.

Source-only review: no build, test, browser, synthetic workload, live mutation or production-data inspection. The coordinator owns the heavy execution slot and the required concurrent-agent CPU/RAM measurements. All findings below are hand-traced; proposed regressions have not been executed. Existing round-1 reports were consulted; the saved-room trace overlaps UX-04 and is explicitly cross-referenced below. Keyboard/modal/loading/telemetry issues are not repeated.

## Scope and bounds

| Surface | Inspected paths and bounds |
| --- | --- |
| School rendering/lifecycle | `ClassroomsBox.svelte`, `school/{scene,assets,model,life,screens}.ts`; at most 36 front kids + 6 back kids + 3 bench kids + headmaster in the open room. The 1,000-row session fetch does **not** create 1,000 animated characters. Static geometry is instanced; nominal frame cap is 30 FPS, DPR at most 2, software renderer drops shadows/DPR. |
| School transport | At most 12 screens per nonoverlapping poll batch, two seconds after completion, API background-lane concurrency/deadline controls apply. Stop aborts inflight reads and suppresses late feeds. Canvas/material disposal and context loss are present on whole-widget teardown. |
| Screen endpoint | Feature `Agents:View`, workspace Viewer, owner-or-admin; offline read does not resume; blocking-pool capture; output at most 60 rows × 240 Unicode scalar values. Underlying emulator resize cap is 500 columns × 300 rows; capture cost is bounded even though clipping follows the emulator copy. |
| Plugin scheduling/cache | 15-minute default auto-scan, single job per account, child-process git index and blame analysis, four blame subprocess slots, cached per-SHA blame, scope LRU of four and memo cap 64, async dependency fingerprint and per-key rebuild coalescing. Deploy tags are all matching tags, without an age/count bound. |
| Plugin async UI | Account/projects/period/overview/people/scan controller. Full analytics/DORA formula correctness, report publishing and every view renderer were not exhaustively audited. |

## Findings

### NS1 [major, performance] Every scan repeats quadratic deployment-tag exclusions — `examples/plugins/team-performance/lib/gitscan.js:557`

**What:** Every matching deployment tag calls `tagContainsLog` with `tags.slice(0, i)`. That helper copies/deduplicates all earlier SHAs, builds a cache key containing all of them, and serializes them again into a separate `git log` process (`gitscan.js:186–203`). Its module-local cache is discarded after each scan: `server.js:105` spawns a fresh worker, and worker mode invokes `buildIndex` once (`gitscan.js:713`). Unique tag keys therefore receive no warm-scan hits in the production path.

**Cost:** T = matching tags across the entire history of one repository, not merely the selected report window. A scan starts T git-log processes and handles T(T−1)/2 earlier-tag entries, plus Git graph traversal. For 1,000 unique tags that is 499,500 exclusion SHAs, roughly 21 MB of exclusion stdin before cache keys/arrays; for 5,000 it is 12,497,500 entries, roughly 525 MB of serialized stdin cumulatively. These are source-derived sizing examples, not measured RSS or timings. The cache also retains quadratic-length string keys during the worker lifetime. This is warm periodic work every 15 minutes by default, repeated for the registered repository fleet, even when no commit or tag changed. The child protects the HTTP event loop but does not remove CPU/RAM/subprocess cost.

**Fix/payoff:** Persist or reuse a bounded range cache across actual worker invocations, with repository identity, tag SHA and a compact prefix digest covering the complete ordered earlier-tag set; invalidate the affected suffix when tags move/appear/disappear. Keep exclusion semantics for diverged branches: simply subtracting the immediately previous tag is not equivalent. Unchanged second scans should make zero range-log calls and avoid quadratic key retention. A later single graph walk could improve the cold scan, but is not required to repair the ineffective warm cache.

**Owner/tests:** Plugin git indexing: `lib/gitscan.js`, worker invocation/persistence in `server.js`, `test/gitscan.test.js`. Use a small isolated fixture with several annotated tags and diverged release branches; invoke worker mode twice, instrument range subprocess counts, compare complete results, then repoint/add an earlier tag and verify affected results refresh. Do not test repeated `buildIndex()` calls in one process only: that misses the production lifetime problem.

### NS2 [major, correctness] Late requests overwrite the selected account or period — `examples/plugins/team-performance/ui/views/app.js:220`

**Intent:** Metrics and project choices must describe the scope selected in the toolbar.

**Confirmed trace:** Select period A, starting overview request A; select period B, starting B. B resolves first and installs B's overview. A then resolves and unconditionally assigns `app.overview` at line 220 and renders at 226, while the toolbar and freshness period use current B. The displayed measurements now belong to A under B's label. An old rejection similarly clears successful current data at 222. Account loading has a stronger variant: A's delayed project response assigns `app.projects` at 188 after account B has been selected; lines 194–203 then read B's storage/account against A's project list, producing a mixed scope. `refreshPeople` has the same unguarded write at 209. None of these controls is disabled while fetching, and there is no generation check.

**Fix:** Capture an account-load generation and a complete overview scope key/request generation before awaiting; commit success, error and follow-on requests only if still current. Abort superseded requests where feasible, but keep the generation guard because completion can win the abort race. Scope scan-poll callbacks to their originating account as part of the same controller fix.

**Owner/tests:** Plugin controller `ui/views/app.js`, `test/ui-views.test.js` or browser test. Deferred responses: resolve B before A and assert metrics, project choices, people and period label stay B; also reject A after B succeeds. Verify an obsolete account callback cannot start another refresh or clear the new account's poll timer.

### NS3 [minor, performance/resource lifecycle] Removed actors do not dispose cloned skeleton textures — `ui/src/modules/home/school/scene.ts:667`

**What:** Both `syncCharacters` removal at 653–657 and `dropCharacters` at 667–677 stop animations and remove nodes without disposing the cloned skeletons. `SkeletonUtils.clone` creates a distinct skeleton per skinned mesh (`ui/node_modules/three/examples/jsm/utils/SkeletonUtils.js:414`); the renderer allocates its bone texture on first draw (`three/src/renderers/WebGLRenderer.js:2696`). The installed library's `Skeleton.dispose` explicitly releases that texture (`three/src/objects/Skeleton.js:298–304`). `assets.dispose()` only visits asset templates, not those per-character clones. Whole-widget context destruction eventually releases the context, but room transitions reuse it.

**Cost:** V = room visits/session removals while the widget stays mounted; S = rendered cloned skeletons per visit. Up to 46 actors can be present, each potentially having multiple skinned meshes. Every cycle allocates another S bone textures without an explicit release. Each skeleton's data texture has side `max(4, ceil(sqrt(4B)/4)*4)` for B bones, at 16 bytes/pixel (`Skeleton.js:252–263`). Exact resident-memory impact and browser garbage-collection behavior require a GPU/browser measurement; no measured leak rate is claimed. This is minor because actor count per room is bounded and leaving the widget forces context loss.

**Fix/payoff:** Centralize actor cleanup: stop/uncache the mixer, traverse the actor's skinned meshes, dispose each unique cloned skeleton once, then detach it. Preserve shared template geometry/materials. Apply to individual removal and whole-room removal. Explicitly live bone textures should then track current actors rather than accumulated visits.

**Owner/tests:** School scene owner; browser School spec. Render a room, return to corridor and repeat ten times, then remove a rendered kid. Instrument actual skeleton disposal or renderer texture counts and require return to a stable baseline after each cleanup. Include the headmaster. A pure assertion that `dispose` appears in source is insufficient.

### NS4 [cross-reference to UX-04, correctness] Initial corridor state erases the saved room before restoration — `ui/src/modules/home/boxes/ClassroomsBox.svelte:216`

**Existing ownership:** This independently confirmed trace is already reported as [UX-04](round-1-ux.md). Count and fix it once under UX-04; NS4 is an alias for coordination, not an additional finding.

**Intent:** Lines 172–176 promise to open the previously saved room once scene/data are ready.

**Confirmed trace:** Mount a box with `config.room = R`. `view` starts as corridor. The persistence effect runs immediately and calls `home.updateBoxConfig(..., {room:null})` at 216. The store synchronously replaces the box's config (`home.svelte.ts:290`). `mountSchool` must await dynamic imports and asset loading; only its later completion reads `box.config.room` at 175, which is now null. No `pendingRoom` is set and the remembered location is lost. Mounting in list/no-WebGL mode also runs the persistence effect and clears a saved room unnecessarily.

**Fix:** Capture the initial saved room before starting asynchronous mount and only persist scene-originated navigation after restoration has completed or been intentionally abandoned. Keep it through list/3D toggles and through slow data loading; distinguish initialization from an explicit corridor navigation.

**Owner/tests:** School component owner, same file as existing design findings D1/D2/D4; coordinate edits. Seed room R in Home config, delay asset/data resolution, mount, then assert entry into R and unchanged config. Also mount list mode and switch to 3D, and verify explicit Back to corridor still stores null.

### NS5 [minor, performance] Screen polling ignores document visibility — `ui/src/modules/home/school/screens.ts:177`

**What:** The new timeout-chain poller checks only its own stopped/busy flags. The component stops it when its Home space is inactive, but that `active` prop means selected Home space (`HomePage.svelte:267,300`), not document visibility. The existing shared `lib/poll.ts:68,111,163` implements a hidden-document pause; the new screen poller does not use it. API background slots bound concurrency but do not check visibility.

**Cost:** N ≤ 12 visible-screen IDs remain wanted when an open room's window becomes hidden. Until browser/platform timer throttling intervenes, that is up to N/2 = 6 unnecessary authenticated screen reads/second per School widget, including DB authorization reads, blocking-pool emulator captures and texture repaint preparation. Browser throttling changes actual cadence, so this is an upper bound, not an observed hidden-window rate. It is avoidable work while no screen can be viewed.

**Fix/payoff:** Gate polling on document visibility, abort pending reads on hide, and schedule one fresh nonoverlapping batch on reveal; remove the listener on stop. Share the existing visibility lifecycle where practical without introducing its five-second box polling floor into the two-second screen contract. Hidden-window request rate becomes zero.

**Owner/tests:** School poller `school/screens.ts` and a focused unit test. Simulate hide during a deferred batch, assert abort/no late feed/no further scheduled fetches, reveal and assert one batch, then stop and verify no listener/timer work. This is independent from component keyboard fixes.

## Non-findings and remaining evidence

- The screen endpoint's route policy and handler both enforce the appropriate access gates. Existing `session_screen.rs` integration coverage exercises root owner, non-owner workspace viewer, nonmember, unknown ID and offline no-spawn. Read those tests; did not execute them. Extend only if implementation changes require it; do not infer a privacy bypass from live terminal content itself.
- Screen reads are bounded by emulator dimensions and output caps. At most 12 client requests per batch is explicit, and server capture runs off the async workers. There is no evidence here for an unbounded screen response or an async-runtime mutex stall.
- No 1,000-actor rendering finding: model seating caps limit the open room to 45 kids plus headmaster. Fresh rendered CPU/GPU/RSS measurements at 1/3/5 agents and a full 45-kid room are still needed, including hidden/reveal and memory recovery. The existing whole-app performance report's transport-only workload does not establish School render cost.
- The scope LRU and derived memo caps prevent unlimited key retention. Parsed-file cache helpers exist in `scopecache.js`, but production `loadScopeRaw` reads through `store.readJsonAsync`; do not claim parsed-file reuse from helper unit tests alone. Measure cold/rebuild cost before deciding whether this warrants a separate fix.
- School `feeds` and scene `screenInfo` retain visited-session entries until widget destruction. Current content is response-capped, but cumulative session churn is not pruned. Runtime churn profiling should quantify this alongside NS3; it is not needed to fix the confirmed missing skeleton cleanup.
- Plugin stuck-job recovery changes job state without cancelling old workers. This behavior predates the added tag pipeline; the new rework child can run for one hour while the watchdog threshold is 45 minutes. A follow-up should trace cancellation and generation-fenced writes before claiming stuck-job recovery prevents overlapping work. Not elevated here without a controlled long-job reproduction.

## Scores and verification ownership

| Dimension | Score / 10 | Evidence and limit |
| --- | ---: | --- |
| Performance | 7.8 | Strong screen/render/concurrency bounds and worker separation; NS1 repeats expensive history work, NS3 lacks owned-resource disposal, NS5 misses hidden pause. Source sizing only; no current-baseline CPU/RAM curve. |
| Correctness | 8.0 | Access/offline/clip paths traced and existing tests inspected; NS2 mislabels stale-scope data, NS4 loses saved state. Analytics formulas and all async view bodies remain outside the bounded pass. |

Fix ownership: School owner handles NS3/NS5 and UX-04 (NS4 alias) together with the existing School design findings; plugin owner handles NS1/NS2. The rendering repair/measurement plan must preserve the exact room cap: 36 front + 6 back + 3 bench kids + 1 headmaster = 46 actors. Coordinator schedules one heavy verification command at a time, then records exact baseline, isolated fixture inputs, CPU/RSS over time and recovery. No 9.8-level or application-wide completion claim follows from this report.
