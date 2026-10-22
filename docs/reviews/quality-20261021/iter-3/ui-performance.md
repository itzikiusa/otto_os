# Quality iteration 3 — lens: ui-performance

Reviewer: ui-performance lens. Read-only (grep/sed/git, plus a small node script that walks static
imports in the scratchpad); no cargo, npm, build, e2e or test runs.

- Worktree: `fix/quality-loop-20261021` @ 3dd55d9f2.
- Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
  `ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/`, `docs/reviews/`.

## Score

Rubric: start at 10.0; per confirmed open finding subtract blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 2 | 0.20 |
| nit | 0 | 0.00 |
| **Total** | 2 | **0.20** |

**Score: 10.0 − 0.20 = 9.80 / 10**

All five iteration-2 findings are fixed. The two new findings are in areas no earlier pass covered
(Kubernetes Monitor workloads table, AWS SQS auto-refresh).

## Previous-finding status (iteration 2)

| id | Iter-2 severity | Status | Evidence |
|---|---|---|---|
| ui-performance-2-01 | minor | **Fixed** | `rooms/room-media.ts:166-175`: `update()` now calls `refreshGraph()`, which rebuilds only when `graphSignature() !== this.graphSig` (`:597`). `schedule()` stays unconditional on purpose (comment `:171-174`: a pending reconcile abandons itself on any newer snapshot). |
| ui-performance-2-02 | major | **Fixed** | `personal-agents/AgentPage.svelte:97-105`: `lookedUp` key per `(workspace, agent)`, lookup read under `untrack`, so a list without the id no longer re-runs the effect. |
| ui-performance-2-03 | minor | **Fixed** | `settings/BrowserSettings.svelte:28-30`: guard now includes `!browserLive.loadError`. |
| ui-performance-2-04 | minor | **Fixed** | `product/design/DesignArena.svelte:265-292`: effect (a) tracks only `product.selectedId` (rest under `untrack`); effect (b) compares a child-id key (`lastChildIds`) and effect (a) seeds it so mount does not double-fetch; `loadChildren` has a `childSeq` token (`:330-342`). |
| ui-performance-2-05 | minor | **Fixed** (FIXES.md lists it under "Not done", but the code has it) | `agents/SessionView.svelte:224-231`: `ensuredFrom` once-per-id guard, `getSession` read under `untrack`. |

## Areas covered this iteration

- **Re-verification** of all five iteration-2 findings (table above), plus the iteration-2 UI fix
  commit `49ede5829` scanned for new effects/perf regressions (ChatView announce edge, MultiRunDialog
  stage focus, RightPanel `$effect.pre` focus restore, NotificationBell switch to `rel()`, workflow-runs
  store extraction, `lib/render/*` move).
- **Bundle/entry graph (static, no build):** walked static imports from `main.ts` (52 files, only
  `svelte` + `@fontsource/cousine`) and from `shell/App.svelte` (156 files, only `svelte`). `marked`,
  mermaid, three, xterm, CodeMirror and highlight.js stay in lazy page chunks; the scene3d move to
  `lib/design/` did not pull three.js into the shell.
- **Terminal.svelte** (iter-2 gap): flow control / `WriteQueue` / credit path, mount effect listener and
  timer balance (every add has a matching remove; RO, IntersectionObserver, HUD interval, verify timers,
  rAF, reconnect timer all cleared in cleanup).
- **Workflows** (iter-2 gap): `workflowRunBus` → WorkflowsPage fast-path/refetch/safety poll;
  `RunSteps`/`RunAgents` 1 s tickers (gated on a running step), review poller keyed on a string, session
  metadata fetched once per id; `workflowRuns` store `apply()`.
- **`workspace.svelte.ts`** (iter-2 gap): `sessionById` and `bucketSessions` are single O(n) passes per
  `sessions` write, and status writes are frame-coalesced; App's `sessionCmdKey` and Navigator's proof
  summary effect re-run per write but dedupe (`askedKeys`) and do no network work.
- **design-hall/site** (iter-2 gap): SiteStudio raw doc + 250 ms coalesced emit, SiteCanvas per-section
  render memo keyed by section identity (embed/asset fns are deriveds, theme stable), brand/embeds
  resolution, overlay measure rAF.
- **snip/SnipEditor** (iter-2 gap): committed-layer / drag-layer caching, one repaint per frame, blob URL
  revocation, off-screen canvas release.
- **swarm/KanbanBoard** (iter-2 gap): single-pass column map, DONE_CAP, waiting poll (15 s, visible-only)
  plus 800 ms debounced nudge.
- **skills-lab** (iter-2 gap): SkillReviewPanel bus + fallback poll, once-per-workspace loads,
  SkillActivity/SkillsBrowser one-shot guards.
- **connections** (iter-2 gap): SftpBrowser windowing, transfer poller cadence, `attempted` guard;
  NetworkProfilePicker; SessionNetworkStatus poller.
- **brokers TopicDetail** (iter-2 gap): `$state.raw` results, windowed table, tail ring (TAIL_CAP),
  adaptive cadence, agent port binding.
- **api/RequestBuilder** (iter-2 gap): body-var scan deferral above 64 KB, form-row reseed guard, docs
  preview derived (lazy, docs tab only), CodeEditor bindings.
- **AWS views:** SqsView (3-02), S3Browser load effect, ViewToolbar auto-refresh chain, account rail.
- **Kubernetes monitor:** MonitorCluster (3-01), MonitorInsights timers.
- **Repo-wide audits repeated:** all 18 `setInterval` sites; effects that call a loader without
  `untrack` (175 candidates scanned by script, the store-reading ones traced into the loader: Users,
  RoomsView/`loadRooms`, Navigator/`proof.loadSummary`, `resourceAccess.load` dedupe/TTL, FindingsBoard
  bus reload, PrDetail, ReportDetail, ScheduledTasksPage, S3Browser); effects that invoke a listener
  handler synchronously at setup (21 sites; only MonitorCluster tracks state through it).
- **Other pages read:** shell `App.svelte` effects (53), Palette, WipPanel live diff refetch, PersonalAgents
  RoomsView, vault GraphView (worker + invalidate-on-demand rAF), QueryBuilder card scroll sync,
  ResultsGrid re-seed/elapsed effects, InsightsPage run poll, AWS SQS, HistoryPage/AuditTab paging.

## Areas NOT covered yet (for the next iteration)

- `lib/components/Terminal.svelte` WebGL atlas / renderer swap and `termPark` re-adopt cost (dedicated
  terminal review owns it); only flow control and lifecycle balance were read.
- `WorkflowsPage.svelte` (5,158 lines) inspector forms and `applyLiveNode` merge cost on very large runs
  (hundreds of nodes with long log arrays).
- AWS `AthenaView`, `Ec2View`, `EksView`, `RdsView` onrefresh paths in detail (only their wiring to the
  shared ViewToolbar chain was checked).
- `canvas/ConversationPanel`, `insights/ReportDetail` iframe sizing, `workbench/WorkbenchPage`,
  `skills-eval/RunDetail`, `panels/BrowserPanel`, `mcp/*` beyond AuditTab paging.
- `vault/VaultPage` editor/reading-view switch and the OKF validator panel.
- Runtime measurement (heap over time, long tasks during agent bursts): needs a running app, outside the
  read-only rule.

## Findings

### ui-performance-3-01 — minor — Kubernetes Monitor workloads table re-subscribes its scroll/resize listeners and re-measures 2-3x per scroll frame

- **File:** `ui/src/modules/kubernetes/monitor/MonitorCluster.svelte:350-396`.
- **Evidence:**
  - `win` is `$derived(visible.length > WINDOW_MIN_ROWS ? tableWindow({ count, rowH, viewTop, viewH, expanded, detailH }) : null)`
    (`:351-353`); `tableWindow()` returns a fresh object (`tableWindow.ts:38-…`), so `win` is a new
    value every time `viewTop` moves — i.e. on every scrolled frame.
  - The listener effect (`:370-389`) defines `onMove = () => { if (raf || !win) return; … }` and then
    calls `onMove()` synchronously at setup (`:383`). On that call `raf === 0`, so `!win` is evaluated
    inside the effect and `win` becomes a dependency of the effect.
  - Result per scroll frame: scroll → rAF `measure()` writes `viewTop` → `win` changes → the effect is torn
    down and re-run: `removeEventListener('scroll'/'resize')`, `addEventListener` x2 again, and its
    `onMove()` queues **another** rAF `measure()`.
  - The re-measure effect (`:391-396`) does `void shown` where `shown = $derived(win ? visible.slice(win.start, win.end) : visible)`
    (`:354`) — a new array on every `win` change even when `start`/`end` did not move — so it also runs
    `tick().then(() => untrack(measure))` every scrolled frame.
  - `measure()` (`:355-368`) reads two `getBoundingClientRect()`s and `offsetHeight` of every rendered
    `tr.wl-row` (≈ viewport rows + 2×overscan, each with two SVG sparklines). The tick-time call reads
    right after the slice DOM update, forcing a synchronous layout.
  - Net: up to three measure passes plus a listener unsubscribe/resubscribe per frame instead of one
    measure, only on clusters with more than 80 workloads (the windowed case) — exactly the large tables
    the windowing was added for (perf K8s R5).
- **Fix:**
  - In the listener effect read the window untracked: `if (raf || !untrack(() => win)) return;` (or wrap
    the setup `onMove()` call in `untrack`). The effect then depends only on the host/tbody elements.
  - Key the re-measure effect on the slice bounds, not the array identity:
    `const sliceKey = $derived(win ? \`${win.start}:${win.end}\` : 'all');` and
    `$effect(() => { void sliceKey; void expanded; void seriesLoading; if (win) void tick().then(() => untrack(measure)); });`
    (read `win` there with `untrack` as well).

### ui-performance-3-02 — minor — SQS auto-refresh fans out up to 40 `aws` CLI calls per tick without waiting for them, so the CLI never idles and later queues' counts starve

- **Files:** `ui/src/modules/aws/SqsView.svelte:84-111, 355-362`; `ui/src/modules/aws/ViewToolbar.svelte:39-50`;
  `ui/src/modules/aws/refresh.ts:6`; `ui/src/lib/stores/aws.svelte.ts:303-313`; backend
  `crates/otto-aws/src/sqs.rs:239-250` (one `aws sqs get-queue-attributes` process per call).
- **Evidence:**
  - The toolbar's auto-refresh is a chain that promises "next tick 10 s after the previous refresh
    SETTLES" (`ViewToolbar.svelte:39-50`, `pollWhileVisible(async () => { await fn(); }, { ms })` with
    `AUTO_REFRESH_MS = 10_000`). SqsView passes `onrefresh={load}` (`:361`).
  - `load()` resolves as soon as the queue list arrives: the attribute fan-out is fire-and-forget —
    `void mapLimit(list.slice(0, 40), 2, (q) => seq === loadSeq ? aws.loadSqsAttrs(...) : …)` (`:101-103`).
    So the chain's "settled" point excludes the up-to-40 attribute calls.
  - Each `loadSqsAttrs` is one `aws` CLI process on the daemon (`sqs.rs:244-250` `run_json(… "sqs",
    "get-queue-attributes" …)`), typically ~0.5-1.5 s each; 40 at concurrency 2 is ≈ 10-30 s, longer
    than the 10 s cadence.
  - The next tick's `load()` bumps `loadSeq`, which stops the previous fan-out from launching more calls
    and starts a new one from index 0. With auto-refresh on and ≥ ~20 queues:
    - two `aws` processes run back-to-back for as long as the toggle is on (the CLI is never idle);
    - the first queues are re-read every tick while queues past roughly index 15-20 are never refreshed —
      their "approximate messages" column stays at the first value, which is the number auto-refresh is
      meant to keep current;
    - each completed call does `this.sqsAttrs = { ...this.sqsAttrs, [url]: a }` (`aws.svelte.ts:308`),
      an O(n) map copy that re-renders the queue list (`SqsView.svelte:380` reads `aws.sqsAttrs[q.url]`
      per row) once per call.
- **Fix:**
  - Make the refresh promise cover the fan-out when it comes from auto-refresh, e.g. `onrefresh={() => load({ awaitAttrs: true })}`
    and in `load()` `const fan = mapLimit(...); if (opts.awaitAttrs) await fan;` so the 10 s gap starts
    after the last attribute call.
  - And/or refresh attributes on auto ticks only for the visible/selected queues (or rotate the 40-queue
    window across ticks so every queue is eventually refreshed).
  - Batch the store writes: collect the results and assign `sqsAttrs` once per fan-out instead of per
    queue.
