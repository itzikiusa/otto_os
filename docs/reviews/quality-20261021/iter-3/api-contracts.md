# Quality iteration 3 — lens: api-contracts

Reviewer scope: parity between `docs/contracts/{api,ws,rooms,room-recaps,game-rooms,product}.md`,
the axum routes and WS handlers in `crates/`, the TS mirrors (`ui/src/lib/api/types/*.ts`
behind the `types.ts` barrel, module-local `types.ts`) and their UI callers, plus `docs/features/`
drift. Read-only. No builds or tests were run. The excluded game, school and classroom UI paths
were not read.

## Score

Start 10.0. Open findings: 0 blockers, 0 majors, 8 minors, 3 nits.

`10.0 − 8×0.1 − 3×0.03 = 10.0 − 0.8 − 0.09 =` **9.11**

## Previous-finding status (iteration 2)

All 13 iteration-2 findings were re-verified against the current code. 12 are fixed. One (2-10) is
partly fixed; its residue is reported again as 3-01.

| id | status | evidence now |
|---|---|---|
| 2-01 | fixed | `api.md:733-736`: owner/admin auth, `AppendTrailReq` → `TrailEvent`, `PutTasksReq {tasks}` → `AgentTask[]`. |
| 2-02 | fixed | `api.md:2162-2183, 2298-2299`: library GETs are `member (Skills View)`; `:2058` skill-eval GET is `member (SkillEval View)`. |
| 2-03 | fixed | `api.md:2166, 2177, 2181`: `UpsertLibraryEntryReq {body}` → `LibrarySkill`/`LibrarySoul`/`LibraryContext`. |
| 2-04 | fixed | `channels-slack-telegram.md:586, 589`: `ws editor + root`. |
| 2-05 | fixed | `connections-ssh-sftp.md:404-406, 419-420`, `daemon-http-api.md:279, 282`. |
| 2-06 | fixed | `message-brokers.md:547-549` point at `connection-sections`. |
| 2-07 | fixed | `api.md:1683`: `UpdateDraftReq {title, body_md}` → `ProductStoryDetail`. |
| 2-08 | fixed | The drafts-list row is gone (`product.md:316` POST only); `canvas.md:431` says GET; `discovery-chat.md:190` says PATCH. |
| 2-09 | fixed | `api.md:859`: response is `ConnectionSection`. |
| 2-10 | **partly fixed** | `ImproveConfig`, `CompatibilityResult`, `Transcript` and `Learning` are gone (`api.md:2018, 3573, 1678-1688`). `api.md:1673` still names `Testcase` → see 3-01. |
| 2-11 | fixed | `ws.md:446-451` (events) and `:82-83` (term) document the 30 s / 60 s / 10 s liveness. |
| 2-12 | fixed | `swarm.ts:308-311`, `memory.ts:48-52`, `activity.ts:79`, `canvas/types.ts:209`, `product.ts:93, 168`. |
| 2-13 | fixed | `room-recaps.md:7, 17-18`: root for both GET and PUT. |

## Areas covered (this iteration)

Each check started as a scratch script over the source. Every reported item was then read by hand
at the cited lines.

- **Role column vs handler role, per crate (iteration 2's main gap).** I mapped each route to its
  handler and then to the `WorkspaceRole::*` the handler checks. I joined that map to the `api.md`
  auth column for otto-git (84 routes), otto-dbviewer, otto-k8s, otto-aws, otto-design,
  otto-vault, otto-canvas, and then all crates. The only drift found is 3-09 and 3-11.
- **Unconditional `require_root` handlers vs the auth column.** This re-checks iteration 2's claim
  with a stricter "first-statement `require_root(&user)?`" match. It found `/usage/attribution` and
  `/usage/forecast` (3-08). The rest were name collisions, which I checked by hand.
- **Route inventory vs docs.** All ~1,290 registered routes are mentioned in `docs/contracts/*`. The
  two exceptions are the test-only `/workspaces/{ws}/__e2e/*` routes.
- **Request side ("—" rows).** These are rows that document no request, where the handler extracts
  `Json<T>` with required fields or `Query<T>` with required keys. See 3-06.
- **Response side.**
  - Rows that document `202`/`204` where the handler returns a JSON body, and the reverse. All
    of these are consistent now.
  - Documented response type names vs the handler's `Json<T>`. Most differences are TS-name
    aliases. The real drift is in 3-01 and 3-07.
- **Query parameters.** I compared each handler's `Query<T>` fields with the `?a=&b=` lists in the
  rows. Only `/usage/attribution?days=` is undocumented (folded into 3-08).
- **Design Hall DTOs (iteration 2 gap).** I paired the differently named Rust and TS types field by
  field:
  - otto-design `types.rs` against `types/design.ts` (21 pairs).
  - otto-design-assist against `types/design-assist.ts` (12 pairs).
  - brand impact/contrast and site export/pinned refs.

  All are clean. `BrandModel` is internal (not `Serialize`); the wire shape is the DTCG `BrandDoc`.
- **API-client DTOs with different names (iteration 2 gap).** gRPC describe, OAuth2 token, Postman
  sync, the streaming open frame (`api.md:2790`) and `RunQueryReq`/`QueryRequest`. All are clean.
- **`Event` enum vs the TS `OttoEvent` union.** All 77 variants match field for field (single-line
  variants were checked by hand). All 77 `type_name()` tags are in `ws.md`.
- **`/ws/events` per-connection frames.** `hello`, `presence`, `subscribe`/`subscribe_ack` (64×64
  caps), `resync`, `hello_ack`, the 4401 close and the liveness constants all match `ws_events.rs`.
- **`/ws/term` client frames.** The `ClientFrame` enum (input, resize, scrollback, search, claim,
  pause/resume, resync, credit, ack, probe) matches the frames `ws.md` §1 documents.
- **Root-level sockets.** `/ws/lsp` and `/ws/api-client/stream` handlers vs `api.md:2678, 3215-3216`
  and `daemon-http-api.md` §2.4. See 3-05 and 3-09.
- **ottod session MCP tool self-calls.** Every literal `/api/v1/...` path in
  `crates/ottod/src/mcp_tools.rs` exists as a route. The canvas routes are `.merge`-built and were
  checked by hand.
- **Iteration-2 behaviour changes.** These were not covered by the docs commit 11e640f0f:
  - personal-agent owner-only fields (3-02)
  - review agent retry 409/500 (3-03)
  - legacy `/history` on the bounded pager (3-04)
  - plugin `running`/`last_error` (clean)
  - `GET /users` for Users:Admin (clean)
  - `expand_home_strict` 400s (clean)
- **Routes added in the last three weeks.** Each was checked against its row's auth, request and
  response:
  - `/sessions/{id}/screen`, `/sessions/{id}/resume`
  - `/fs/stat`
  - `/auth/shares*`
  - `/insights/*` (see 3-06 for `/insights/report`)
  - `/telemetry/*`, `/mcp/approvals/count`
  - workbench diff and assets
  - `/repos/{id}/push-target`, `/repos/{id}/head/remotes`
- **`docs/features/*` troubleshooting strings (iteration 2 gap).** Every quoted error string of 18 or
  more characters in a Troubleshooting section was grepped in `crates/`, `ui/src` and the desktop
  shell. Paraphrases and UI labels were dropped. See 3-10.

## Areas NOT covered yet (next iteration)

- **Prose response rows field by field.** These are `api.md` rows whose response is prose (for
  example "conflict listing" or "config"). Iteration 3 caught the ones whose handler needs input
  (3-06), but not every prose response against its handler's return struct.
- **404/409 semantics across all rows.** Only the routes changed in iteration 2, the review retry
  and the memory governance routes were read.
- **`product.md` P1–P36 request/response field shapes.** Only the DTO names were checked
  (iteration 2).
- **`rooms.md` WS command/event unions** against `rooms/socket.rs` after the most recent rooms
  changes. Iteration 2 checked the limits and HTTP routes.
- **Module-local TS types for usage, insights, k8s-monitor and the browser** (`ui/src/modules/*/types.ts`)
  against their Rust DTOs with different names.
- **The PTY-holder protocol** (`otto-pty/src/holder.rs`). It is internal and process-to-process, so
  it is not a public contract, but `api.md:321-325` describes its behaviour.

## Findings

### api-contracts-3-01 — nit — `api.md` still names `Testcase` and `CommitInfo[]` for routes that return other types

- **Where:**
  - `docs/contracts/api.md:1673`: `PATCH /product/testcases/{tid} | … | UpdateTestcaseReq | Testcase`.
  - `docs/contracts/api.md:1487`: `GET /repos/{id}/prs/{number}/commits | ws viewer | — | CommitInfo[]`.
- **Evidence:**
  - `update_testcase` (`crates/otto-product/src/http.rs:980-1009`) returns the
    `ProductTestcase` the repo's `update_testcase` gives back. `Testcase` exists in neither Rust
    nor TS. This is the residue of 2-10.
  - `pr_commits` returns `Vec<PrCommit>` (`crates/otto-core/src/api.rs:1950-1956`:
    `sha, short_sha, author, date, subject`). `CommitInfo` (`:1381-1391`) also carries
    `parents` and `refs`, so a client typed from the contract expects two arrays that never
    arrive. The UI already uses `PrCommit[]` (`ui/src/modules/git/PrDetail.svelte:202`,
    `types/git-merge.ts:303`).
- **Fix:** Change the response to `ProductTestcase` on `:1673` and to `PrCommit[]` on `:1487`.

### api-contracts-3-02 — minor — Personal-agent memory and context writes are owner-only, but `api.md` says any Editor; the PATCH row lists fields that don't exist

- **Where:**
  - `docs/contracts/api.md:4727, 4729`: `PUT /personal-agents/{id}/memory` and `/context` are
    `scheduled_tasks edit + ws editor`.
  - `docs/contracts/api.md:4625`: PATCH lists "`save_memory` and `save_context`" as owner-only
    fields.
  - `docs/features/personal-agents.md:304-308`: the same, and it names only the two PATCH routes
    as the 403 source.
- **Evidence:**
  - `save_memory` and `save_context` (`crates/otto-server/src/routes/personal_agents.rs:809-820,
    840-850`) call `require_agent_owner(&ctx, &user, &agent, &["memory"])` and `&["context"]`
    unconditionally. A non-owner Editor who is not a ws Admin gets 403 "This agent runs as its
    owner; only the owner or a workspace admin can change its memory." (`:334-336`). An
    ownerless preset is adopted by the caller (`:325-331`).
  - `UpdateAgentReq` (`:146-157`) has no `save_memory` / `save_context` field. Serde drops unknown
    keys, so a client that follows the row and sends them gets a silent no-op, not the 403 the
    row promises.
- **Fix:**
  - `api.md:4727, 4729`: auth becomes `scheduled_tasks edit + ws editor; the agent's owner, a ws
    admin or root (else 403; an ownerless preset is adopted by the caller)`.
  - `api.md:4625` and `personal-agents.md:304-308`: replace "`save_memory` / `save_context`" with
    "its memory and context (`PUT …/memory`, `PUT …/context`)", and add those two routes to the
    troubleshooting entry's route list.

### api-contracts-3-03 — minor — Review agent retry doesn't document the new 409 and 500 outcomes

- **Where:** `docs/contracts/api.md:1512` (`POST /reviews/{review_id}/agents/{index}/retry`). It
  documents `400` (no prompt), `409` (agent not settled or review running) and `400` (budget), and
  says the diff is "DB-persisted at dispatch".
- **Evidence:** `crates/otto-server/src/modules/review/retry.rs` returns more statuses than the
  row lists. Iteration 2's FIXES.md (error-handling-2-06) added the 409 and 500 here, and the docs
  commit 11e640f0f did not carry them into `api.md`.

  | Status | Cause | Code |
  |---|---|---|
  | 409 | "this review has no stored diff to retry against; start a new review" (a pre-0100 review whose temp diff is gone) | `:280-286` |
  | 500 | "Couldn't write the diff …" when the stored diff can't be read or rewritten | `:287-297` |
  | 409 | "agent {index} is already being retried" (a concurrent retry) | `:232-242` |
  | 409 | "review revision is unavailable" / "cannot be checked out — start a new review" (branch reviews, `:17-45`) | `:17-45` |
  | 400 | "no review agent at index {index}" | `:179-183` |
- **Fix:** Extend the row's status list with these five outcomes. Also say that a review recorded
  before migration 0100 can no longer be retried once its temp diff is swept (409; start a new
  review).

### api-contracts-3-04 — minor — `api.md` says the legacy `/history` array is unchanged; it now runs on the bounded pager and can return short or empty pages with more data left

- **Where:**
  - `docs/contracts/api.md:6211`: "The legacy `/history` array endpoint remains unchanged."
  - `:820` (`GET /workspaces/{wid}/history?…&before=&limit=`): documents a `HistoryEntry[]` paged by
    `before`, with nothing about short pages.
- **Evidence:**
  - Since 52edfdbe7, `legacy_entries` (`crates/otto-server/src/routes/history_page.rs:112-133`)
    calls the same `page()` as `/history/page`, with a synthetic exclusive cursor (`:109-110,
    142-150`).
  - `page()` resolves at most `4*limit` candidates (`:154, 181-183`). `finish_page` then applies
    the `q`/`provider` filter (`:294-296`).
  - The legacy array drops `next_cursor` (`:131-132`, `.entries` only).
  - So with a `q` or `provider` filter, the array can be shorter than `limit`, or empty, while more
    matches exist. A legacy client that stops on an empty page, or pages by the last row's
    `last_active_at`, misses them. `api.md:6217` documents exactly this for `/history/page` ("only
    null indicates exhaustion"), but the legacy contract still promises nothing changed. (The UI
    uses `/history/page`; the legacy route remains for API clients.)
- **Fix:**
  - Replace "remains unchanged" with: "the legacy `/history` array is served by the same bounded
    scan (≤ `4*limit` candidates) with `before` as an exclusive cursor. A filtered page may be
    short or empty before exhaustion and carries no cursor; use `/history/page` to page reliably."
  - Add the same caveat to row `:820`.

### api-contracts-3-05 — minor — Six feature-doc passages still tell clients that `?token=` works on the WebSockets

- **Where:**
  - `docs/features/daemon-http-api.md:132-148`. The §2.4 title is "Root-level `?token=` routers".
    The rows list `?token=` as the fallback for `/ws/term` and `/ws/events`, and as the auth for
    `/ws/lsp?…&token=` and `/ws/api-client/stream?token=`. The note says "`?token=` remains a
    backward-compatible fallback".
  - `daemon-http-api.md:362-364` ("or `?token=`") and `:395` (a `wscat … /ws/events?token=…`
    example).
  - `docs/features/agent-sessions.md:249-250`: "`?token=<bearer>` is accepted as a
    backward-compatible fallback".
  - `docs/features/api-client.md:635`: `GET /ws/api-client/stream?token=…&workspace_id=…`.
  - `docs/features/rbac-multiuser-sharing.md:332` ("or `?token=<token>` on the WS endpoints") and
    `:433` ("or `?token=` on WS").
  - `docs/features/remote-mobile-access.md:475`: "`?token=` is a fallback only".
- **Evidence:** The fallback was removed (S11-312). The code:
  - `crates/otto-server/src/ws_events.rs:82-88`: the token comes only from the subprotocol.
  - `crates/otto-sessions/src/ws.rs:3, 820-850`: "never `?token=`".
  - `crates/otto-server/src/routes/api_stream.rs:60-66`: 401 "missing token" with no subprotocol.
  - `crates/otto-server/src/lsp/mod.rs:230-241`.

  The contract says so: `ws.md:11-21` ("A `?token=` … is **not** accepted (401) — on this or any
  other daemon socket") and `api.md:3213-3216`. A script built from the feature docs' `wscat`
  example gets a 401.
- **Fix:**
  - Retitle §2.4 to "Root-level WebSocket and proxy routers".
  - Make the auth `Sec-WebSocket-Protocol: otto-bearer, <token>` only (`?token=` → 401) on every
    row and passage above.
  - Delete the `?token=` `wscat` line.

### api-contracts-3-06 — minor — Six `api.md` rows document no request input, but the handler requires a body or a query key

- **Where and evidence.** Each handler rejects the call (422, or a 400 / 404 for a missing path)
  when the input is absent. The UI callers send the real shapes.

  | `api.md` row | Says | Handler takes / returns | UI caller |
  |---|---|---|---|
  | `:760` `POST /sessions/{id}/handover` | `— \| starts a handover` | `Json<HandoverReq>`: `target` (tagged `kind`) is required; `focus?`, `title?`, `brief?`, `include_git?` (`otto-core/src/api.rs:2384-2400`). Returns `Session` (the target). | `ui/src/modules/agents/Handover.svelte:157` (`api.post<Session>`) |
  | `:761` `POST /sessions/{id}/handover/brief` | `— \| generates a handover brief` | `Json<HandoverBriefReq {focus?, include_git?, fast?}>` → `HandoverBriefResp {brief, fallback, …}` (`:2414-2428`) | — |
  | `:1510` `POST /reviews/{review_id}/handoff` | `— \| hand the review findings to an agent session` | `Json<HandoffReq>`: `provider` is required; `comment_ids?` (`otto-core/src/api.rs:2360-2367`). Returns `Session`. | `ui/src/modules/git/LocalReviewPanel.svelte:395` |
  | `:1490` `POST /repos/{id}/api-collections/push` | `— \| commit API-client collections into the repo` | `Json<PushCollectionsReq>`: `files` and `message` are required; `branch?` (`otto-git/src/http.rs:2138-2143`). Returns `{commit, files}`. | `ui/src/lib/stores/apiClient.svelte.ts:1234` |
  | `:1664` `POST /product/stories/{sid}/questions/post` | `— \| post questions back to the source story` | `Json<PostQuestionsReq>`: `ids` is required; `format?` (`otto-product/src/types.rs:95-99`) | `ui/src/lib/stores/product.svelte.ts:742` |
  | `:1477` `GET /repos/{id}/conflict` | `— \| conflict listing` | `Query<ConflictQuery>`: `path` is required (an empty one is 400). Returns ONE `ConflictFile` (`otto-git/src/http.rs:2786-2797`), not a listing. | `ui/src/lib/stores/git.svelte.ts:800` |

  `api.md:3038` (`GET /insights/report | Insights:View | — | one report's HTML`) also omits the
  required `?path=`. The handler (`otto-insights/src/lib.rs:1302-1320`) canonicalizes it, returns
  403 outside the insights directory and 404 when it is missing. The UI caller is
  `ui/src/lib/api/insights.ts:43, 52`.
- **Fix:** Fill in each request cell with the DTO and its required fields:
  - `HandoverReq {target:{kind:"new_agent",provider}|{kind:"existing",session_id}, focus?, title?, brief?, include_git?}` → `Session`
  - `HandoverBriefReq {focus?, include_git?, fast?}` → `HandoverBriefResp`
  - `HandoffReq {provider, comment_ids?}` → `Session`
  - `{files:[{name,content}], message, branch?}` → `{commit, files}`
  - `PostQuestionsReq {ids, format?}`
  - `?path=` → `ConflictFile`
  - `?path=` (under the insights dir; 403 outside, 404 missing)

### api-contracts-3-07 — minor — Memory merge and split responses are wrapped objects, not the bare `Memory` / `Memory[]` the contract states

- **Where:** `docs/contracts/api.md:3598` (`POST /workspaces/{ws}/memory/merge` → "merged `Memory`") and
  `:3599` (`POST /workspaces/{ws}/memory/{mid}/split` → "`Memory[]`").
- **Evidence:**
  - `crates/otto-server/src/memory_gov.rs:95-109`: `merge` returns `Json<MergeResp>`, which is
    `{memory: Memory}` (`crates/otto-memory/src/governance.rs:64-66`).
  - `:115-126`: `split` returns `Json<SplitResp>`, which is `{memories: Memory[]}`
    (`governance.rs:84-86`).
  - A client that reads `res.id` or iterates `res` gets `undefined`, or a non-iterable object.
  - The sibling rows are right: `state` and `forget/undo` return a bare `Memory` (`memory_gov.rs:44,
    78`), which makes the wrappers easy to miss. No UI caller exists yet, so nothing breaks today,
    but the rows are the only spec for the governance API.
- **Fix:** Response `MergeResp {memory: Memory}` on `:3598` and `SplitResp {memories: Memory[]}` on
  `:3599`. Also note that `parts` needs 2 or more entries (`governance.rs:77-79`).

### api-contracts-3-08 — minor — `/usage` auth is wrong in both docs: attribution and forecast are root-only, and the rest are not

- **Where:**
  - `docs/contracts/api.md:3563-3564`: `GET /usage/attribution` and `POST /usage/forecast` are
    `ws viewer (Usage:View)`. The attribution row also lists only `?by=`.
  - `docs/features/usage-and-cost.md:549`: "All `/usage/*` routes are **root**-only".
- **Evidence:**
  - `attribution` and `forecast` (`crates/otto-server/src/routes/usage.rs:851-872`) both open with
    `require_root(&user)?`. The policy guard admits Usage:View
    (`policy_decisions.txt:3888`), so a non-root Usage:View member passes the guard and then gets 403.
    The UI calls both from the Usage page (`ui/src/modules/usage/AttributionDrilldown.svelte:56`,
    `CostForecastChip.svelte:37`). `attribution` also reads `?days=` (default 30, clamped
    1–3650, `:845-858`), which the row doesn't list.
  - In the other direction, `status`, `summary`, `report` and `by_kind` (`:75-89, 163-175,
    214, 262-273`) run no root check. They redact or scope for non-root callers, which `api.md:3019-3021`
    documents correctly. So the feature doc's "all root-only" is wrong for four routes.
- **Fix:**
  - `api.md:3563-3564`: auth becomes `root (Usage:View passes the guard; non-root → 403)`, and
    the attribution request becomes `?by=…&days=` (default 30, 1–3650).
  - `usage-and-cost.md:549`: "`status`, `summary`, `report` and `by-kind` are Usage:View (non-root:
    redacted / own sessions); every other `/usage/*` route is root-only".

### api-contracts-3-09 — minor — `/ws/lsp` is documented as `ws editor`, but any workspace member (Viewer included) can open it; the second `/ws/api-client/stream` row omits its required query and grant

- **Where:**
  - `docs/contracts/api.md:3215` (`GET /ws/lsp?lang=&root= | … ws editor`) and
    `docs/features/daemon-http-api.md:142` (`ws editor`).
  - `api.md:3216` (`GET /ws/api-client/stream | … ws editor | …`).
- **Evidence:**
  - `lsp_ws` (`crates/otto-server/src/lsp/mod.rs:225-357`) checks no workspace role.
    `root_visible_to` (`:362-388`) collects the roots of every workspace in
    `workspaces.list_for_user(&user.id)` and ignores the role (`.map(|(w, _)| w.root_path)`), so a
    **Viewer** of the workspace containing `root` gets a language server over the whole tree.
  - The row's own prose ("inside a workspace they are a member of") contradicts its auth column.
  - Undocumented:
    - agent-session credentials are refused (403 "an agent session credential cannot open a
      language server", `feature_guard.rs:420-421`)
    - 400 for a missing or unknown `lang` / `root` (`lsp/mod.rs:263-304`)
    - 503 `server_unavailable` when no binary is found (`:306-314`)
  - For `/ws/api-client/stream`, `stream_ws` (`routes/api_stream.rs:37-40, 84-97`) needs
    `?workspace_id=` (a `Query` extraction failure is a 400). It also needs ws Editor **and** an
    API Client grant of at least Edit, else 403 "API Client edit access is required in this
    workspace". The row at `:2678` documents both. The duplicate at `:3216` documents neither.
- **Fix:**
  - `:3215` and `daemon-http-api.md:142`: auth becomes `otto-bearer only; root, or a member (any
    role) of a workspace whose root contains root; share / MCP / agent-session tokens 403`, plus
    the 400/503 cases.
  - `:3216`: becomes `?workspace_id=` with `ws editor + API Client Edit`, or a link to `:2678`.
  - If the intent was Editor, the code is the bug. Flag that to the access-control lens:
    `root_visible_to` would need a role filter.

### api-contracts-3-10 — nit — Feature-doc troubleshooting quotes error text the code never produces

- **Where and evidence:**
  - `docs/features/insights.md:442-443` lists two distinct reasons, "Insights skill not available"
    and "no workspace available". `post_run` returns one combined `reason`: "the 'insights' skill
    is not installed, or no workspace is available to host the run"
    (`crates/otto-insights/src/lib.rs:1392-1400`). A user can't tell which row applies.
  - `docs/features/workflows.md:316` and `:1485` quote "run exceeded the N-minute time limit".
    The code says "run exceeded the {}-hour time limit", where `RUN_WALL_CLOCK_TIMEOUT` is 10 h
    (`crates/otto-server/src/workflow_engine/run/finalize.rs:56-59`, `workflow_engine.rs:665`).
    The running step is marked error with "workflow active time budget exhausted" (`:51`), not
    skipped as `:316` says.
  - `docs/features/skills-evaluator.md:480` quotes "validation did not complete". No such text
    exists. The engine writes "validation produced {n} of {passes} required findings verdicts"
    (`crates/otto-server/src/skill_eval.rs:596-598`).
- **Fix:** Quote the real strings. For insights, merge the two rows into one with both causes. For
  workflows, say "10-hour" and that the running step errors.

### api-contracts-3-11 — nit — Two auth cells disagree with the guard or the handler

- **Where and evidence:**
  - `docs/contracts/api.md:2172-2174`: the three `/library/provider-skills*` GETs are `any member`.
    The feature guard requires `Skills:View` for them
    (`crates/otto-server/tests/snapshots/policy_decisions.txt:1863, 1868`). That is the same rule
    as the rest of the `/library/*` table, which says `member (Skills View)`.
  - `api.md:3719`: `DELETE /mission-views/{id}` is `ws editor (Agents:Edit, owner)`. `delete_view`
    (`crates/otto-server/src/routes/mission.rs:538-556`) requires the owner **or root** and only
    `WorkspaceRole::Viewer`. So the view's owner can delete it after being demoted to Viewer, and
    root can delete anyone's view.
- **Fix:**
  - `:2172-2174`: `member (Skills View)`.
  - `:3719`: `Agents:Edit + ws viewer + the view's owner or root`.
