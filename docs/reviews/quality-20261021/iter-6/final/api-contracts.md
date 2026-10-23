# Quality iteration 6 (final): api-contracts lens

Reviewer scope: parity between `docs/contracts/{api,ws,rooms,room-recaps,product,game-rooms}.md`,
the axum routes and WS handlers in `crates/`, the TypeScript mirrors (`ui/src/lib/api/types/*.ts`,
module-local `types.ts`) and their UI callers, plus drift in `docs/features/`. The review was
read-only: no cargo, npm, build or test commands were run. Base: `fix/quality-iter6-20261023` at
2065f00e9 (all three fix waves), diffed against `origin/main` 02a64019b.

## Score

Start at 10.0. Open findings: 0 blockers, 0 majors, 4 minors and 2 nits.

`10.0 − 4×0.1 − 2×0.03 = 10.0 − 0.40 − 0.06 =` **9.54**

(Wave 0 scored 9.61 with 6 findings. All 6 are fixed. The 6 findings below are new: 2 come from
this iteration's changes (6f-03, 6f-04), and 4 are older gaps that the wider sweep found
(6f-01, 6f-02, 6f-05, 6f-06).)

## Previous-finding status (wave 0)

| id | status | evidence now |
|---|---|---|
| 6-01 (minor) | **fixed** | `api.md:2043` `RunNowResp {run_id}` + 409; `:2045` `{run, edits}`; `:2046` `?status=` default `pending`; `:4816` `{room, members: string[]}`; `:4672` `{session_id, created}`. All match `otto-improve/src/http.rs` and `routes/personal_agents.rs`. |
| 6-02 (minor) | **fixed** | The env-key 400 is on `api.md:941, 942, 4353, 4355`. Busy 409 / timeout 502 is on `:2369-2370`, and the `session_files.rs:172` comment now says 502. The workflow-delete task pause is on `:2360`. The shutdown 409 is on `:85` and `:778`, and `:778` returns `{killed: number}`. |
| 6-03 (minor) | **fixed** | Row 79 (`api.md:384`) now names the most recent prompt editor across task, project and swarm, which matches `otto-swarm/src/runtime/run.rs:309-380` (`acting_user`, access-control-6-02). It also splits the statuses: 409 for the swarm's own budget and pause, 400 for the workspace-budget block and for no active agent. |
| 6-04 (nit) | **fixed** | CP46 (`api.md:4408`) now lists the CP45 validation 400s and the case-insensitive duplicate-name 409. |
| 6-05 (nit) | **fixed** | `ws.md:967` reads `GET /workflow-runs/{id}/progress`. |
| 6-06 (nit) | **fixed** | `api.md:2694` reads `ApiAutomation[]`. |

Counts: 6 fixed, 0 still open, 0 rejected.

## Areas covered

- **Re-verification of the six wave-0 findings** (see the table above).
- **Scripted route inventory, both directions** (every `.route("…")` literal, including multi-line
  ones, against every `METHOD /path` in `docs/contracts/*.md`):
  - The contracts are clean. The only routes missing from the docs are test fixtures (`/botTOKEN/*`,
    `/rest/api/3/*`, `/wiki/*`, `/__e2e/*`, `/big`, `/chunked`, the `otto-mcp` client test `/mcp`),
    or routes the docs give in another form (`WS` table rows, `ANY /plugins/{slug}`).
  - The only doc paths with no route are relative shorthand (`/branch`, `/conflict`,
    `/browser/query` …) or external provider APIs.
  - This iteration's only route changes are the recap handler renames (`recap::http::*` →
    `rooms/recap.rs`, same paths and verbs) and one test router.
- **Scripted UI→route check** (`api.*` / `api.bg.*` template paths and verbs). Every leftover is a
  query-string template artifact, or the static `/workspaces/scratch` route shadowing
  `/workspaces/{id}` (GET is served there). The new typed git client `ui/src/lib/api/git.ts` hits
  real routes with the right verbs, and its types match the handlers:
  - status, refs, push, push-target, checkout, stage and unstage return `RepoStatusResp`;
  - pull returns `{status, note}`;
  - `prs` uses `?state=&page=&per_page=`.
- **Scripted feature-guide route check** (`docs/features/*.md`, both by full path and by
  relative suffix). One stale route was found (6f-06). The rest are shorthand or external
  Jira/actuator paths.
- **This iteration's wire changes, field by field (Rust ↔ TS ↔ docs):**
  - `AdminSessionsQuery` / `AdminSessionsResp.next_cursor` (`otto-core/src/api/auth.rs:398-420`,
    `types/auth.ts:104-117`, `api.md:2908`). The feature guide was not updated (→ 6f-04).
  - `InsightsConfigView.schedule_status` / `ScheduleAttempt` (`otto-insights/src/lib.rs:207-228`,
    `types/insights-share.ts:25-38`, `api.md:3063`).
  - `DiffAssembly` / `AssembleSummary` / `AssembleResp` (`otto-core/src/api/proof.rs:22-61`,
    `types/proof.ts:109-134`, `api.md:4283`). The wrapper and the feature guide are stale
    (→ 6f-03).
  - `meta.mcp_skipped` (`otto-sessions/src/manager/mcp_sync.rs:213-229`, the `json_patch` null
    clears it, `types/sessions.ts:8-19`, `api.md:933`, `features/agent-sessions.md`).
  - The swarm hire gate #66 (`otto-swarm/src/http.rs:239-335`). It matches the row's field list
    and the 403 text.
  - The goal and standing-goal prompt-editor stamping (#62, #72, the goal rows): these are
    storage-only fields (`SwarmPatch` / `ProjectPatch` / `GoalPatch` / `TaskPatch`), and no
    response field was claimed.
  - `otto.query_db` and the agent `write_blocked:` / `read-only` / `review_required:` texts
    (`otto-dbviewer/src/service.rs:58-59, 1983`; `otto-mcp/src/outward/exec.rs:80, 560`) against
    `api.md:221-230`, `features/database-explorer.md` and `features/mcp-control-plane.md`.
  - The twin counts in `features/mcp-control-plane.md`: 46 `NATIVE_TWINS` and 30 `ROUTED_TWINS`
    were counted in `ottod/src/mcp_tools/{catalog,dispatch}.rs`, and `mcp_tools/listing.rs`
    exists.
- **Refactor moves with a contract surface:**
  - The `otto-apiclient` engine: the `EngineError` → `otto_core::Error` mapping is 1:1, and the
    automation rows match.
  - The `otto-rooms` extraction: the recap handlers keep their statuses and content types
    (`rooms/recap.rs` vs the old `recap/http.rs`).
  - The `otto-product` run split: the events moved (→ 6f-05).
  - `otto-git/src/local/errors.rs`: the same 409 / 502 split.
  - The single webhook key guard: the 404, 401 and 500 problems are unchanged.
  - `otto-product` publish, detached: still awaited, same `{url, ref}`.
- **New statuses added this iteration:**
  - The proof assemble 409.
  - The assist resume 403, now shared in `otto-agent-run`, already enforced before (→ 6f-02).
  - The goal-loop budget 400, now pinned by a test (→ 6f-01).
  - The chat-abort reply texts (chat body only).
- **Citations in docs:** every `crates/…/*.rs` path cited in `docs/contracts` and `docs/features`
  exists, apart from `otto-state/src/mcp_registry.rs` in the design-stage
  `mcp-control-plane-plan.md`. Every `file.rs::fn` citation resolves.
- **WS:** the `Event::type_name()` arm count (80) still matches `ws.md:602`, and no event was
  added or removed. Product event emitters and status values are checked in 6f-05.
- **K8s monitor `json!` payloads** (an area wave 0 did not cover):
  - overview / restricted rows (`otto-k8s/src/monitor/http.rs:600-700`), workloads, series, the
    probe test body, and fleet filters / requests (`fleet.rs:851-853, 1347`), all against
    `api.md:5800-5880`;
  - the new `forbidden: credentials rejected …` `metrics_server` value fits the documented
    `'forbidden: <message>'`.

  All clean.

## Areas NOT covered

- **Session stdio MCP twin reply shapes.** Twenty-six more twins now dispatch through `route_for`
  (ROUTED_TWINS grew from 4 to 30). The docs do not specify per-tool reply bodies, and the
  snapshot tests pin them, so I did not diff the 30 replies field by field.
- **Notification rows** (`api.md` "Notifications"). They still describe responses in prose.
  Unscored, as in waves 5 and 6.
- **The PTY-holder protocol** (internal), and the `product.md` rows other than the agent and
  publish routes.
- **`docs/features/git.md`, `database-explorer.md` and `kubernetes-console.md` status-code
  claims.** I checked these only by route and method, not row by row for status text.

## Findings

### api-contracts-6f-01: minor. The workspace usage-budget 400 is missing from the goal-loop and Product agent rows

**Pattern.** When the workspace usage budget blocks, a point-of-action gate refuses the request
with **400** `Invalid`. This includes the fail-closed "Budget settings unavailable…" case for an
unreadable config. The contract documents this 400 for swarm row 79 (`api.md:384`) and for local
reviews (`api.md:1509`), but not for these routes:

| Route (row) | Gate in code |
|---|---|
| `POST …/goal-loops/define` (#91, `api.md:3772`) | `crates/otto-server/src/routes/goal_loops.rs:88` |
| `POST …/goal-loops` with `autostart` (#93, `:3774`) | `goal_loops.rs:236-237`. Runs **after** the loop row is created, so the 400 leaves a Draft loop behind. |
| `POST /goal-loops/{id}/start` (#96, `:3778`) | `goal_loops.rs:424` |
| `POST /goal-loops/{id}/resume` (#98, `:3780`) | `goal_loops.rs:481` |
| `POST …/product/stories/{sid}/analyze` (`api.md:1746`, `product.md:37` P10) | `crates/otto-product/src/analysis.rs:55-66` "Budget exceeded — analysis blocked: …" |
| `…/rewrite` (`api.md:1747`, `product.md:50` P23) | `analysis.rs:314-323` |
| `…/testcases/generate` (`api.md:1748`, `product.md:51` P24) | `analysis.rs:379-388` |
| `…/plan/generate` (`api.md:1749`) | `analysis.rs:445-453` |

`goal_loops.rs:69-75` maps a blocked verdict to `Error::Invalid("Budget exceeded: …")`. This
iteration's new test,
`budget_gate_tests::a_blocked_budget_refuses_to_start_a_loop_and_leaves_it_a_draft`
(`goal_loops.rs`), pins the 400, the "Budget settings unavailable" text, and the loop staying
Draft.

**Failure.** A client or MCP agent that reads rows #93/#96 (`→ GoalLoop`) or P10/P23 (`200` /
`202`) treats the 400 as a malformed request, and does not tell the user to raise the budget.
After a refused `autostart` create it also does not know a Draft loop now exists.

**Fix.** Add the following to rows #91, #93, #96 and #98, and to the four Product agent rows in
`api.md` and `product.md`: "**400** 'Budget exceeded…: <reason>' when the workspace usage budget
blocks (an unreadable budget config also blocks — 'Budget settings unavailable…')". On #93, add
"the loop is created first, so a refused autostart leaves it in Draft". Alternatively, state it
once in the Product section note (`api.md:1640-1645`) and in the goal-loops heading.

### api-contracts-6f-02: minor. The 403 for resuming another user's assist session is not in the assist rows

**Where:**
- `docs/contracts/api.md:3861` (#107 `POST /canvas/scenes/{id}/assist`, which lists only the 409);
- `api.md:3999` (`POST /design/artifacts/{id}/assist`, which lists 400, the reference 403/404 and
  the 409);
- `api.md:1804` (`POST /product/stories/{sid}/mockups/assist`).

**Evidence.** Every assist turn runs through `otto_agent_run::session_turn::run_agent_turn`. Its
first statement is `require_owned_resume(manager, ws, user, t.existing)`
(`crates/otto-agent-run/src/session_turn.rs:165`). That check returns
`Error::Forbidden("cannot resume an assistant session owned by another user or workspace")`
(**403**, `session_turn/resume.rs:37-54`) when the stored session was created by someone else or
in another workspace. The calls are:
- Canvas passes `existing: self.scene.session_id` (`otto-canvas/src/assist.rs:241`);
- mockups pass `self.session_id` (`otto-server/src/mockup_assist.rs:243`);
- Design Hall passes its `existing_session` (`otto-design-assist/src/job.rs:27`).

This iteration documented the 403 in `docs/features/canvas.md` ("Only the user who created that
session in this workspace may resume it (403 otherwise)") and in `docs/features/design-hall.md`
("resuming `meta.assist`'s session is refused (403)"). It did not reach the authoritative rows.

**Failure.** A second editor who uses Ask AI on a scene or artifact that a colleague started gets
a 403 that the contract never lists. An API or MCP client (`otto.design_assist`) cannot tell this
403 from a missing workspace grant.

**Fix.** Add the following to the three rows: "**403** 'cannot resume an assistant session owned
by another user or workspace' when the stored assist session (`scene.session_id` /
`meta.assist` / the mockup's session) was created by another user or in another workspace".

### api-contracts-6f-03: minor. The new `AssembleResp` shape did not reach the proof feature guide or the typed UI wrapper

**Where:**
- `docs/features/proof-packs.md:624` (row 122): "… `AssembleReq {cwd?, base?, commands?}` |
  `ProofPackResp`".
- `ui/src/lib/api/proof.ts:111-112`:
  `assembleProof(id, body): Promise<ProofPackResp>` → `api.post<ProofPackResp>(…/assemble)`.

**Evidence.** In this iteration (error-handling-6-02), the handler changed:
- It now returns `Json<AssembleResp>` (`crates/otto-server/src/routes/proof.rs:633, 669-672`),
  which is `ProofPackResp` flattened plus `assembled: {diff, commands}`
  (`otto-core/src/api/proof.rs:22-61`).
- It now propagates `assemble_diff` errors (`proof.rs:647-649`), including the 409 "repository
  HEAD changed while assembling proof; retry". Before, it ignored them.

`api.md:4283` and `ui/src/lib/api/types/proof.ts:109-134` were updated. The only caller,
`ui/src/modules/proof/ProofPage.svelte:425`, works around the stale wrapper with a cast:
`(await assembleProof(…)) as AssembleResp`. The feature guide's troubleshooting table
(`proof-packs.md:801-809`) also does not cover the new 409.

**Failure.** Any other caller of `assembleProof` is typed without `assembled` and cannot see
`not_a_repo` or `diff_failed`. That is the "silent success" error-handling-6-02 removed. A reader
of the feature guide expects a bare pack, and is not told that the HEAD-moved case is a retryable
409.

**Fix:**
- In `ui/src/lib/api/proof.ts:111`, return `Promise<AssembleResp>` from
  `api.post<AssembleResp>(…)`, and drop the cast in `ProofPage.svelte:425`.
- In `proof-packs.md:624`, change the response to "`AssembleResp` = `ProofPackResp` +
  `assembled: {diff: DiffAssembly | null, commands: [{cmd, status, error}]}`; 409 when HEAD
  moves mid-assembly".
- In `api.md:4283`, say `diff` is `null` (not absent) without `cwd`, since `Option` is serialized
  without `skip_serializing_if`. Also note that `commands` run only when `cwd` is given.

### api-contracts-6f-04: minor. The rbac feature guide still describes the admin sessions overview as one unpaged list of every session

**Where:**
- `docs/features/rbac-multiuser-sharing.md:252-258`: "a daemon-wide list of **every** session
  across all workspaces and users".
- `:457`: `GET /admin/sessions` → "`AdminSessionsResp {sessions: AdminSessionRow[]}`", with no
  query.

**Evidence.** backend-performance-6-05 (this iteration) changed the route:
- It takes `AdminSessionsQuery {include_archived, limit, cursor}` and, by default, lists only
  **unarchived** sessions, 200 per page (max 1000), newest first.
- It returns `next_cursor` for the next page, and a malformed cursor gives 400
  (`crates/otto-server/src/routes/admin_sessions.rs:92-156`, `otto-core/src/api/auth.rs:398-420`).

`api.md:2908`, `types/auth.ts:104-117` and the UI's **Load more**
(`ui/src/modules/settings/AdminSessions.svelte:59-96, 317`) all follow the new contract. The
feature guide was not in this iteration's docs commits.

**Failure.** An admin script built from the feature guide calls `GET /admin/sessions` once,
believes it has every session, never pages, and never sees archived ones. A "remove all exited"
sweep built that way stops after the first 200 rows.

**Fix.**
- In `rbac-multiuser-sharing.md:252-258`, say: "unarchived sessions (live plus recent exited) by
  default, newest first, 200 per page with **Load more**; *Include archived* adds the history".
- In `:457`, write "`?include_archived=&limit=&cursor=` → `AdminSessionsResp {sessions,
  next_cursor?}` (malformed cursor → 400)".

### api-contracts-6f-05: nit. The `product_changed` section in `ws.md` names one emitter file for four run kinds and omits the `partial` status

**Where:** `docs/contracts/ws.md:878-879` and `:886`.

**Evidence:**
- **The emitter citation.** This iteration's `run.rs` split changed the citation to "Emitted by
  `crates/otto-product/src/run/analysis.rs` at the end of every AI-run task (analysis, rewrite,
  test-case generation, plan generation)". Only analysis emits there
  (`run/analysis.rs:504-509`). Rewrite, test cases and plan emit from `run/rewrite.rs:179, 207`,
  `run/tests_gen.rs:218, 246` and `run/plan.rs:415, 438`.
- **The status values.** The documented value set is `"done|error|draft|changed"`, but the
  analysis event sends `status: final_status`, which is `"partial"` when any lens failed
  (`run/analysis.rs:495, 508`). That also predates the split (`run.rs:1001` on main).
  `features/product.md` (this iteration) says a failing or panicking lens "marks the analysis
  `partial`".

**Fix.**
- At `ws.md:878`, cite `crates/otto-product/src/run/{analysis,rewrite,tests_gen,plan}.rs`.
- At `:886`, use `"done|partial|error|draft|changed"` ("`partial` — analysis only, when at least
  one lens failed").

### api-contracts-6f-06: nit. The skills-evaluator guide cites a route that does not exist

**Where:** `docs/features/skills-evaluator.md:362`: "it writes to the shared library, like
`PUT /library/skills`".

**Evidence.** `/library/skills` is `GET` + `POST` only (`crates/otto-context/src/http.rs:95-96`).
The library write that this sentence means is `PUT /library/skills/{name}` (`api.md:2190`), or
`POST /library/skills` (`:2187`).

**Fix:** "like `PUT /library/skills/{name}`".
