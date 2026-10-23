# Quality iteration 6: lens scores

Iteration 6 ran on the branch `fix/quality-iter6-20261023` (base `main` 02a64019b). Wave 0 reviewed
all 15 lenses before any fix. Three fix waves followed, each re-scored on the four target lenses
(architecture, test-quality, accessibility-responsive, ux-design). After wave 3, every lens got a
fresh, full review of the same tree (`2065f00e9`); those reports are in [final/](./final/). **The
final column is the final score for the whole quality effort (2026-10-21 to 2026-10-23).**

All scores use the same rubric: start at 10.0 and subtract 1.0 per blocker, 0.3 per major, 0.1 per
minor and 0.03 per nit, for each confirmed open finding.

| Lens | Pre-fix (wave 0) | Wave 1 | Wave 2 | Wave 3 | Final | Change |
|------|-----------------:|-------:|-------:|-------:|------:|-------:|
| backend-correctness | 9.17 | | | | **9.97** | +0.80 |
| ui-correctness | 9.57 | | | | **9.70** | +0.13 |
| access-control | 9.20 | | | | **9.70** | +0.50 |
| backend-performance | 9.78 | | | | **9.77** | −0.01 |
| ui-performance | 9.80 | | | | **9.87** | +0.07 |
| concurrency-async | 9.57 | | | | **9.87** | +0.30 |
| data-integrity | 9.47 | | | | **9.77** | +0.30 |
| error-handling | 9.44 | | | | **9.57** | +0.13 |
| architecture | 7.11 | 7.78 | 9.11 | 9.88 | **9.68** | +2.57 |
| maintainability | 9.34 | | | | **9.31** | −0.03 |
| test-quality | 8.60 | 9.40 | 9.77 | 10.00 | **10.00** | +1.40 |
| api-contracts | 9.61 | | | | **9.54** | −0.07 |
| ux-design | 9.31 | 9.81 | 9.97 | 9.97 | **9.64** | +0.33 |
| accessibility-responsive | 8.87 | 9.60 | 9.87 | 9.87 | **9.57** | +0.70 |
| integrations | 9.47 | | | | **9.77** | +0.30 |

Blank cells: that lens was not re-scored in that wave.

- **Mean:** 9.22 before the fixes, **9.72 final**. Iteration 5 started at 9.06; iteration 1 at 8.58.
- **Lowest final lens:** maintainability, 9.31. Every lens is above 9.3. None has a blocker, and
  only one has a major (access-control-6f-01, below).
- **Round goal:** wave 1 set the goal that the four target lenses end at 9.0 or higher. All four
  did, including architecture, which had been the lowest lens in every iteration (6.30, 5.90,
  6.77, 7.31, 7.55, then 7.11 at the start of this one).
- **At or above 9.8:** backend-correctness (9.97), ui-performance (9.87), concurrency-async (9.87)
  and test-quality (10.0). The other 11 lenses are between 9.31 and 9.77.

## Why the final score is lower than wave 3 on three target lenses

No code changed between the wave-3 re-score and the final review (both read `2065f00e9`). The final
pass was deliberately wider: each reviewer re-read the code this iteration moved and the areas that
earlier waves had only sampled. The drops come from what that wider read found, not from
regressions:

- **architecture 9.88 → 9.68:** the four wave-3 nits are still open, plus two new minors in areas
  no wave had judged: two OAuth token clients in route files with different guards (6f-05), and
  list reorder hand-rolled six times (6f-06).
- **ux-design 9.97 → 9.64:** the wave-3 nit is still open, plus four findings in the git graph and
  database store code that the splits moved verbatim from `main` (the remote-branch delete and tag
  push confirmations, Cancel on "Push tag to origin?" still creating the tag, and a failed object
  search reading "No object matching …").
- **accessibility-responsive 9.87 → 9.57:** the two wave-3 findings are still open, plus three
  older problems found outside the swept paths (Move up/down focus in QueryBuilder and
  Appearance, more self-removing controls, the MetricChart tooltip flip).

## What moved each target lens

### Architecture: 7.11 → 9.68

The largest gain of the effort. Almost every long-standing structural finding was closed:

- **Wave 1 (7.78):** one list of transcript roots in `otto_transcript::roots` (codex account
  rollouts and `$CODEX_HOME` are now imported); turn folds in `otto_transcript::turn`; the
  resumable turn runner moved into `otto_agent_run::session_turn` (`agent_session.rs` 1,359 → about
  265 lines); `run_workflow_with_budget` 1,250 → about 90 lines; `review_run::execute` split into
  phases; a shared fan-out runner `otto_agent_run::fanout`; a shared `ListWindow`; a typed git
  client; `lib/` stopped importing most of `modules/`.
- **Wave 2 (9.11):** one assist turn engine (`otto_agent_run::assist`) under canvas, Design Hall and
  mockups; resume authority in `session_turn`; the MCP governance stage runner, which also closes
  architecture-5-03, deferred in iteration 5; vault docs and code-review summarizer on `fanout`;
  `DatabaseStore` 3,210 → 1,003 lines; WorkflowsPage 3,710 → 2,688; OverviewTab 2,494 → 1,772;
  `lib/` → `modules/` imports at zero and enforced as a hard guard.
- **Wave 3 (9.88):** the REST engine moved into `otto-apiclient` behind `ApiClientCtx`
  (`routes/api_client.rs` 3,261 → 917 lines); the new `otto-rooms` crate behind `RoomsCtx`, which
  closes architecture-2-06, deferred in wave 1 and in iteration 5; all 46 hand-written MCP session
  twins generated from the control-plane specs (`mcp_tools.rs` 4,882 → 1,852 production lines);
  product analysis and plan on `fanout`; GraphView 4,082 → 1,064 lines; the apiClient store
  2,113 → 764 lines; the swarm's own paste path deleted. The otto-server `src` total fell from
  108,541 to 100,055 lines.

**Still open (final, 2 minors and 4 nits):** two OAuth token-endpoint clients in route files with
different guards (6f-05); list reorder hand-rolled six times, with the one shared controller under
`product/` (6f-06); two more cwd-encoder copies in `routes/handover.rs` and `session_turn.rs`
(6f-01); the API automation run loop in an axum handler (6f-02); the `RefChip` type and component
share a name (6f-03); product's per-lens retry duplicates `LensTurn::run` (6f-04).

### Test-quality: 8.60 → 10.0

- **Wave 1 (9.40):** pure, tested review-gate and budget logic (`review_score`, `verdict_from`,
  `budget_rows`) with caller tests for an unreadable budget config; one shared `webhook_key_guard`
  pinned by the new `public_entry_gates.rs` suite; `database_changes_api.rs` for approve/reject
  authority; `StepSecretBinding::for_caller` shared by the route and its test.
- **Wave 2 (9.77):** `blended_score` tests; every `/ingest/*` route and the plugin-host routes
  pinned against missing, empty and foreign tokens; peer cancel and reconcile cases for database
  changes; source-regex UI tests replaced by behaviour tests.
- **Wave 3 (10.0):** the assist tests wait on a `Notify` instead of a fixed sleep (50/50 runs
  passed); the MCP DB-execute test now requires the driver's error, so its read case can fail; the
  ingest token test counts trail rows per post; 8 product engine tests over a scripted host. Each
  new test was checked to fail with its fix reverted.

**Still open:** nothing. The final pass found no new finding.

### Accessibility-responsive: 8.87 → 9.57

- **Wave 1 (9.60):** the shared `announce()` and the always-mounted `Announcer`, adopted at the
  state edge across most modules; APG tree keys in FileTree; repo tabs movable by keyboard; focus
  after moved or removed rows (AutomationEditor, TestCasesTab, AggregateBuilder); 3D shortcuts bound
  to the viewport instead of the window.
- **Wave 2 (9.87):** inserted live regions taken to zero module by module (rooms 17, MCP 7, git and
  the shell modules 40, data tools 58, api/workflows/product 29); `LoadState` announces a stale
  refresh once; the `lib-imports-modules` and other guard rules.
- **Wave 3 (9.87):** the live-region guard widened to `{:else}` branches and nested regions, with a
  separate `inserted-alert` ratchet; git, rooms, product, API and Design Hall swept again.

**Still open (final, 4 minors and 1 nit):** status regions still inserted with their text beyond the
guard's two-line window or inside `{#each}` (6f-01); Move up/down in QueryBuilder and Appearance
drops focus to `<body>` at the list edge (6f-03); more controls that remove themselves without
moving focus (6f-04); the MetricChart tooltip runs off a narrow chart (6f-05); the assistant's limit
line speaks an old limit when a thread opens (6f-02, nit).

### UX-design: 9.31 → 9.64

- **Wave 1 (9.81):** inline `LoadState` with Retry replaced toasts and guessed empty states in AWS,
  PR list, Mission Control, the agent page and product; clipboard failures say how to copy by hand;
  lazy ResultsGrid dialogs that reset on failure.
- **Wave 2 (9.97):** `copyWithToast` and `COPY_FAILED_BODY` everywhere; recap and room-start failure
  copy; the MCP attach checkbox renders only once the daemon reports its value.
- **Wave 3 (9.97):** OttoServerHome no longer guesses "attached" or shows a 0 gateway count while
  loading or after an error.

**Still open (final, 3 minors and 2 nits):** remote-branch delete and the create-tag push confirm
without saying who sees it (6f-02); Cancel on "Push tag to origin?" still creates the tag (6f-03);
a failed database object search reads "No object matching …" (6f-04); the recap `LoadState`
wrappers double the error copy (6f-01, nit); "Checkout" and "Check out" side by side in the graph
menus (6f-05, nit).

## The other lenses

Wave 1 planned every wave-0 finding of every lens, so the non-target lenses moved too. They were not
re-scored until the final review.

- **backend-correctness 9.17 → 9.97:** product chat removes only the actions span; every
  agent-reply parser picks the first JSON that fits the expected shape; Mongo keyset paging past a
  string `_id`; room audio needs a host. Open: one nit (the scheduled Insights attempt count resets
  on restart).
- **access-control 9.20 → 9.70:** identity-changing swarm hires need the creator or an Admin;
  prompt edits are stamped with their editor. Open: **one major that the 6-02 fix introduced**
  (access-control-6f-01). `acting_user` lets any timestamped scope edit (standing goals, a project
  goal) beat a task's untimed originator, so a person's new task can run as whoever last saved a
  swarm or project setting. This is the most important item left.
- **concurrency-async 9.57 → 9.87:** the artifact read checks and reads in one blocking hop; vault
  rename and product publish finish when the request is dropped; prompt edits and stamps land in
  one statement. Open: a parent-directory swap between check and open still escapes the allow-list
  (minor); the plugin `free_port` race (1-05, nit, still deferred).
- **data-integrity 9.47 → 9.77:** multi-statement MySQL/Postgres reviews run in one transaction;
  `mcp_profiles` are archived again; atomic Insights config writes. Open: agent-suggested learnings
  are active before review; archive restore does not remap user ids in editor columns (minors); the
  design-assist working copy is rewritten in place (nit).
- **integrations 9.47 → 9.77:** multi-request MCP writes get budgets that cover them; Kubernetes
  401s read as rejected credentials; a channel reply whose upload is refused still posts its
  head. Open: Otto's Claude project-directory encoder misses claude's 200-character cap, so a
  long cwd breaks the agent turn (minor); the PR-comment budget is shorter than its route (minor);
  stray spaces in the Codex skipped-MCP reason (nit).
- **error-handling 9.44 → 9.57:** proof assembly and scheduled Insights report failures; a Working
  diff fallback only for an unborn HEAD. Open: five more raw "Load failed" errors; improvement edit
  approve/rollback reports success on `conflict`; a failed marks load shows "No marks yet";
  API-client Keychain read failures become blank credentials (minors); the Insights failure list
  hides once reports exist (nit).
- **ui-correctness 9.57 → 9.70:** stale DB-assistant replies are discarded; chart series key by
  position. Open (minors): duplicate keys in Jira "Linked issues"; Home/End in a windowed file tree
  focuses nothing; admin session paging mixes two listings after a filter change.
- **ui-performance 9.80 → 9.87:** on-demand rendering for the 3D scenes and a bounded model cache.
  Open, both introduced by the fixes: a pull from the graph reads the history twice (minor); the
  classroom's on-demand loop renders corridor drag at about 20 fps (nit).
- **backend-performance 9.78 → 9.77 (flat):** all five wave-0 findings fixed (streamed exports,
  admin session paging, batched test-case loads). The final pass read agent-launch setup for the
  first time and found about 20 paths calling the synchronous `ensure_trusted` on a runtime worker,
  a skill-library copy on a worker (minors), and no index under the new keyset pager (nit).
- **api-contracts 9.61 → 9.54 (down):** all six wave-0 findings fixed. Two new minors come from this
  iteration's changes (the proof `AssembleResp` shape and the paged admin sessions overview are not in
  the guides); two more minors and two nits are older gaps the wider sweep found (budget 400 and
  resume 403 missing from rows, a stale `ws.md` emitter, a route that does not exist).
- **maintainability 9.34 → 9.31 (down):** all eight wave-0 findings fixed, but the large refactors
  left the same kinds of residue again: comments and a shipped skill pointing at deleted paths, five
  named re-export shims the shim ratchet cannot see, the stop-turn state machine copied into
  DiscoveryChat and RefineChat, thirteen "is focus in a text field?" checks, two copies of the
  transcript-roots helpers, 46 unit-test files named after work packages (`w3p1-…`), and helpers
  copied into both halves of a split.

## Deferred items

- **concurrency-async-1-05 (still deferred):** the plugin `free_port` TOCTOU. The fix changes the
  plugin protocol (the plugin binds `:0` and reports its port, or `/health` echoes
  `OTTO_PLUGIN_TOKEN`), which breaks existing third-party plugins, so it needs a protocol-versioning
  decision. It counts as a nit in the final concurrency-async score.
- **architecture-2-06 (deferred in wave 1, done in wave 3):** the `otto-rooms` crate extraction
  landed in wave 3 behind `RoomsCtx`.
- Waves 2 and 3 deferred nothing.

## Suggested next steps

1. Fix access-control-6f-01 first. It is the only major, and the 6-02 fix introduced it: a task's
   own originator should win over a later scope stamp, with a test that creates the task after a
   standing-goals save.
2. integrations-6f-01 (the 200-character project-directory cap) breaks agent turns in deep
   worktrees. The fix belongs in `otto_transcript::roots::encode_cwd`, which is now the one encoder
   apart from the two copies in architecture-6f-01.
3. Maintainability and api-contracts dropped for the same reason: big moves leave stale pointers.
   A post-split checklist (stale paths grep, shim count, contracts and feature guide rows) would
   catch most of it.
4. Run the Playwright suites. No wave's gate reports a Playwright run, and the waves changed
   several spec selectors that pinned removed live roles.

## Reports

- Pre-fix: [wave-0/](./wave-0/)
- Fix plans and outcomes: [wave-1/](./wave-1/), [wave-2/](./wave-2/), [wave-3/](./wave-3/)
  (each has `PLAN.md` and `FIXES.md`, plus the target-lens re-scores)
- Final scores: [final/](./final/)
