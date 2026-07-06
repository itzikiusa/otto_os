# Otto Feature-Set Review — 2026-07-06

Full review of six feature areas — **Connections, Agents, Git, Product, Workflows, API client** —
run as six independent read-only review agents (one per area, code-cited findings only).
Focus per the brief: fix and polish the existing feature set (correctness first, then flow/UX
simplification, then enhancements); new features only where they're deal-breakers vs
best-in-class competitors.

Every finding cites the file/line the reviewer read. Effort: **S** ≤ ½ day, **M** = 1–3 days, **L** = 1 wk+.

---

## Executive summary

The feature surface is broad and mostly competitive; the gaps are concentrated in a handful of
**correctness holes in core paths** and a few **table-stakes UX flows** competitors treat as free.
Ten P0 bugs (below) are individually small — seven are S-effort — but include real data loss
(git discard deleting a file), money values rendering as NULL (MySQL `DECIMAL`), a workflow
cache that can **replay a human approval without a human**, and a cross-workspace cookie jar
that leaks credentials between tenants.

**Yesterday's DATETIME bug is a class, not an incident.** All three SQL drivers in the Database
Explorer decode by trying concrete Rust types and falling through to `Value::Null`, so every
unhandled column type renders as a `∅` indistinguishable from SQL NULL. MySQL `DECIMAL`
(i.e. every balance/amount column) and a wide slice of Postgres types (arrays, enums, `INET`,
`INTERVAL`, `MONEY`) are hitting that fallthrough today. The structural fix — a shared
"never silent-Null; fall back to the driver's text representation" invariant — retires the class.

---

## Cross-cutting themes

1. **Decode-fallthrough → silent NULL** (CONN-A1/A2/A4/A5). Same class as the DATETIME fix.
   One shared fallback helper for all drivers converts a recurring bug into a guarantee.
2. **One incoherent egress policy.** The netguard blocks the API client from localhost/private
   targets (the #1 thing an API client is for — API-B1), kills SSH-tunnelled requests *before*
   the proxy is applied (API-A2), yet workflow `http_request`/`api_run` nodes bypass the guard
   entirely (WF-A4). Decide the policy once: user-initiated requests get an explicit per-workspace
   opt-in to local/private targets; tunnelled requests are checked at the far end, not locally;
   *every* outbound path goes through the guard.
3. **Tenancy/isolation leaks.** Global cookie jar shared across workspaces/users (API-A1),
   plaintext auth + tokens written to history (API-A3), event triggers unscoped by workspace
   (WF-A3).
4. **Silent side-effect replay.** The workflow node cache is keyed globally, so `human_approval`,
   `budget_gate`, `git_pr`, `channel_notify` can be "satisfied" from a previous run's output
   (WF-A1). Highest-severity single finding in the review.
5. **Lossy saves & lost updates.** API request Save drops scripts/settings/docs (API-A4);
   agent canvas turns clobber concurrent user edits (PROD-A3); session `meta` read-modify-write
   races (AGT-A3).
6. **Resource leaks on repeat automation.** `otto-wf/<run_id>` worktrees + branches are never
   reaped (WF-A5) and schedules have no overlap guard (WF-A6) — an hourly workflow compounds both.
7. **Status legibility.** Both the Agents wall ("Idle" hides *waiting-for-you* vs *quietly working*,
   AGT-B1/B2) and Workflows (scheduled runs finish silently, WF-B1) under-report state the user
   needs to act.

---

## Prioritized roadmap

### P0 — correctness / safety / data-loss (ship first)

| ID | Area | Fix | Effort |
|----|------|-----|--------|
| WF-A1 | Workflows | Exclude gating + side-effecting nodes from the result cache (`human_approval`, `budget_gate`, `git_pr`, `channel_notify`, `swarm_task`, `http_request`, `api_run`, …) | S |
| GIT-A1 | Git | Discard on a staged rename must restore `orig_path` + `path` (currently deletes the file) | S |
| CONN-A1 | Connections | MySQL `DECIMAL`/`NUMERIC` → add `BigDecimal` branch (money columns render `∅` today) | S |
| CONN-A2 | Connections | Postgres arrays/enums/`INET`/`INTERVAL`/`MONEY`/`BIT` → raw-text fallback before Null | M |
| API-A1 | API client | Per-workspace (or per-user) cookie jars; scope list/clear; stop returning cookie values to viewers | M |
| API-A3 | API client | Redact auth + `Authorization`/`Cookie`/`Set-Cookie` before writing history | S |
| WF-A2 | Workflows | `human_approval` gets its own timeout (hours/`params.timeout_s`), not the 120s agent constant | S |
| WF-A3 | Workflows | Event triggers: exclude engine-internal events, workspace-scope matching, in-flight cap, implement or drop `filter_json` | M |
| AGT-A1 | Agents | Canonicalize session cwd before codex/agy session-id capture (symlinked `/var`,`/tmp` → non-resumable sessions) | S |
| PROD-A1 | Product | Discovery-Chat "create canvas" emits legacy doc shape → always-blank canvas; emit `{format, source}` | S |

### P1 — high-impact UX + remaining medium bugs

| ID | Area | Item | Effort |
|----|------|------|--------|
| API-B1 | API client | Per-workspace opt-in "Allow local & private targets" (the defining friction of the API client) | M |
| AGT-B1 | Agents | Provider-agnostic "awaiting input" state feeding the needsYou inbox (unblocks AGT-B2/C1/D2) | M |
| CONN-B1 | Connections | Test connection *before* save (`POST /connections/test` with unsaved config + form button) | M |
| WF-B1 | Workflows | Scheduled/event/webhook runs deliver results (default `result_chat`/`result_webhook` on trigger spec) | M |
| GIT-A2 | Git | Amend with empty message → `git commit --amend --no-edit`; prefill HEAD message in UI | S |
| GIT-A3 | Git | Create-PR must push the *selected* source branch, not HEAD | S–M |
| API-A4 | API client | Persist scripts/settings/docs/graphql-vars on request save (migration + round-trip) | M |
| PROD-A3 | Product | Canvas lost-update: `updated_at` precondition on PUT + skip live ingest while editor dirty | M |
| WF-A5 | Workflows | Reap `otto-wf/<run_id>` worktrees + branches at run finalize + startup sweep | M |
| WF-A6 | Workflows | Schedule overlap guard (skip when a run is in flight; policy field later) | S |
| WF-A4 | Workflows | Route `http_request`/`api_run` nodes through netguard + redirect policy | S |
| API-A2 | API client | Skip/relax local SSRF pre-check when `ssh_connection_id` set (tunnel is the sanctioned egress) | M |
| CONN-A3 | Connections | Apply the 1 MiB `cap_cell` to MySQL/Postgres rows (only ClickHouse caps today) | S |
| GIT-A4 | Git | Merge-commit diffs: use first-parent diff (combined `--cc` renders empty/garbled) | S |
| GIT-A5 | Git | `core.quotePath=false` for untracked-file listing (non-ASCII names invisible in Changes) | S |
| AGT-A2 | Agents | Idle-suspend: skip sessions with live descendant processes / require 2 idle sweeps | M |
| AGT-A3 | Agents | Session `meta` writes: `json_set` merge or serialized writes (resize races un-pin/detach) | M |
| PROD-A2 | Product | Jira Cloud pagination: thread `nextPageToken` (Load-more currently duplicates page 1) | M |
| PROD-A4 | Product | Paginate Jira/Confluence comments (watcher misses newest comments past page 1) | M |
| WF-A7 | Workflows | Validate trigger cadence/cron at write time (unknown cadence silently never fires) | S |

### P2 — flow simplifications & enhancements to existing features

| ID | Area | Item | Effort |
|----|------|------|--------|
| CONN-C1 | Connections | Shared "no silent Null" decode invariant across all SQL drivers (retires the bug class) | M |
| CONN-B3 | Connections | Auto-append preview `LIMIT` to un-limited `SELECT *` (whole table buffered in RAM today) | M |
| CONN-B2 | Connections | ClickHouse enums: render labels (parse `Enum8('a'=1,…)` from column type) | M |
| CONN-C2 | Connections | Hex/base64 toggle in the cell inspector; Mongo UUID-subtype + Redis binary rendering (CONN-A4/A5) | S each |
| AGT-B3 | Agents | New-session dialog: optional first task (via `submit_text`) + model picker (`meta.model` already honored) | M |
| AGT-B2 | Agents | Suppress idle/suspend countdown for working-but-quiet sessions (after AGT-B1) | S |
| AGT-A5 | Agents | Scrollback search jump: anchor by matched text via SearchAddon (logical vs wrapped rows) | M |
| GIT-B1 | Git | Conflicting cherry-pick/revert opens the existing ConflictResolverView | M |
| GIT-B2 | Git | Pull with auto-stash (parity with local merge) | S–M |
| GIT-B3 | Git | Graph pagination past the 200-commit cap | M |
| GIT-B4 | Git | "Commit & Push" / post-commit push nudge | S |
| GIT-C2 | Git | PR CI/checks + mergeable badge in PR list/detail | M |
| GIT-C3 | Git | Invalidate open diff after external git ops (auto-fetch refreshes status only) | M |
| PROD-B1 | Product | Consolidate the 13-tab story workspace into one Discovery surface (draft + chat + canvases + mockups) | M |
| PROD-B2 | Product | One-click "publish draft to Jira/Confluence" from Discovery chat | M |
| PROD-C2 | Product | Jira 401/403 vs 429 handling (Retry-After backoff in watcher) | S–M |
| API-B2 | API client | Collection picker (tree/dropdown) instead of "type a number" prompt | M |
| API-B3 | API client | Open saved/history request in a new tab instead of clobbering the active draft | S |
| API-C1 | API client | Import Postman *environments* (not just collections); richer Postman auth mapping (API-C3) | M |
| API-C2 | API client | Consolidate the two diverging cURL generators (multipart-aware) | S |
| API-C4 | API client | Secret-typed environment variables (masked + Keychain-backed) | M |
| API-C5 | API client | HTML/PDF response preview tab (sandboxed) | M |
| WF-C1 | Workflows | Per-node cache opt-out + TTL (`params.no_cache`) | S |
| WF-B2 | Workflows | Surface AI-generation fallback ("you got the degenerate 2-node graph") | S |
| AGT-A4/A6, GIT-C1, PROD-A5/A6, PROD-C3, WF-A8 | misc | Low-severity fixes — see per-area sections | S each |

### P3 — deal-breaker new features (worth adding)

| ID | Area | Feature | Why it's a deal-breaker | Effort |
|----|------|---------|------------------------|--------|
| GIT-D1 | Git | **Hunk/line-level staging** (`git apply --cached` on selected lines; DiffViewer already renders per-line origins) | The single biggest gap vs Fork/Tower/GitKraken/VS Code | L |
| AGT-D1 | Agents | **Per-session "what changed" strip** — changed-files chip + inline diff sheet in the agent pane | The review loop competitors (Conductor, VS Code agent mode) sell; all pieces exist | L |
| AGT-D2 | Agents | **Follow-up task queue** — queue prompts while agent works, auto-submit on awaiting-input (builds on AGT-B1) | Unattended multi-session use; standout differentiator | M |
| WF-D2 | Workflows | **Retry from first failed step** (machinery exists: `start_node` + cached upstream outputs) | Every CI product has it; closes the run-debug loop | S–M |
| WF-D1 | Workflows | **`call_workflow` sub-workflow node** (all 4 templates repeat the same spine verbatim) | Composition is table stakes in n8n/GH Actions | M |
| API-D1 | API client | **Collection/folder-level auth + variables inheritance** | The main thing keeping real API suites DRY in Postman/Insomnia/Bruno | L |
| API-D2 | API client | Data-file iterations for the existing runner (CSV/JSON row per run) | Postman Collection Runner parity | M |
| PROD-D1 | Product | **Live PRD canvas** — one always-in-sync doc stitching draft + diagrams + top mockup + open questions; exportable/publishable | Mostly assembly of existing data; the artifact ChatPRD/Productboard sell | M–L |
| PROD-D2 | Product | Bi-directional Jira sync with reviewable upstream-change diffs | Makes Otto a Jira front-end, not a read-mostly companion | L |
| GIT-D2 | Git | Reword / squash-last-N (interactive-rebase-lite, unpushed-only guard) | One-click in Tower/GitKraken | L |

---

## Per-area findings (full detail)

## Connections — Findings

The datetime bug is not a one-off. The SQL decoders decode by *trying concrete Rust types in
sequence and falling through to `Value::Null`*, and sqlx's per-type `compatible()` gate silently
rejects several common column types. Every type not explicitly handled renders as a dimmed `∅`
that is **indistinguishable from a real SQL NULL** (`ui/src/modules/database/ResultsGrid.svelte:1792`).

### A. Bugs / correctness fixes

**A1. MySQL `DECIMAL`/`NUMERIC` columns render as NULL — high, S**
`mysql_value_to_json` (`crates/otto-dbviewer/src/drivers/mysql.rs:1525`) tries `i64 → u64 → f64 →
bool → JSON → NaiveDateTime/Date/Time → String → Vec<u8> → Null`. No `BigDecimal` branch, and
sqlx 0.8.6 excludes `DECIMAL` from every branch that exists (`real_compatible` matches only
`Float|Double`; `str`/`bytes` `compatible()` lack `Decimal|NewDecimal`). A `DECIMAL` cell falls
through to `Value::Null` (`mysql.rs:1575`), hitting both grid (`mysql.rs:1494`) and export
(`mysql.rs:575`). Money columns (`latestBalance`, transaction amounts) silently show `∅`.
The `bigdecimal` sqlx feature is already enabled (`crates/otto-dbviewer/Cargo.toml:21`).
**Fix:** add before the `String` branch, mirroring the Postgres driver:
```rust
if let Ok(v) = row.try_get::<Option<BigDecimal>, _>(idx) {
    return v.map(|n| Value::String(n.to_string())).unwrap_or(Value::Null);
}
```

**A2. Postgres arrays, `INTERVAL`, `MONEY`, `INET`/`CIDR`/`MACADDR`, `BIT`, user enums render as NULL — high, M**
`pg_value_to_json` (`crates/otto-dbviewer/src/drivers/postgres.rs:1331`) handles scalars/`BigDecimal`/
temporal/`Uuid`/`Vec<u8>` then falls to `Value::Null` (`postgres.rs:1386`). No array branch; the
dbviewer features don't include `ipnetwork`/`mac_address`/`bit-vec`; `PgInterval`/`PgMoney`
unhandled. **Fix:** final raw-value text fallback before Null (`row.try_get_raw(idx)` →
`raw.as_str()` catches enums/inet/interval/money/arrays as text) plus explicit `Vec<String>`
array handling.

**A3. Oversized cells capped only for ClickHouse — medium, S**
`cap_cell`/`MAX_CELL_CHARS = 1 MiB` exists solely in the ClickHouse driver
(`clickhouse.rs:1185`, applied at `:587`). MySQL (`mysql.rs:1491-1496`) and Postgres row loops
push cells uncapped — a `LONGTEXT`/`bytea`/large JSON freezes the webview. **Fix:** run the same
`cap_cell` over MySQL/Postgres cells.

**A4. Mongo UUID-subtype binary shows as base64 gibberish — low/med, S**
`bson_to_json` maps every `Bson::Binary` to base64 (`mongodb.rs:1729`) ignoring `bin.subtype`.
**Fix:** when subtype ∈ {Uuid, UuidOld}, format as `UUID("…")` (`BsonUuid` already imported,
`mongodb.rs:16`).

**A5. Redis binary values unrecoverable — low, S**
`bytes_to_json` returns `"<N bytes>"` for non-UTF-8 values (`redis.rs:775-777`), discarding the
payload. **Fix:** hex/base64 fallback like MySQL/PG `Vec<u8>`.

### B. UX / flow simplifications

**B1. No "Test Connection" before saving — high impact, M**
`ConnectionForm.svelte:711` has only Save; all test endpoints require a persisted id
(`crates/otto-dbviewer/src/http.rs:286`, `crates/otto-connections/src/http.rs:140`). The user
must write the connection + stash the password in Keychain before learning the creds are wrong —
opposite of TablePlus/DBeaver/DataGrip. **Fix:** `POST …/connections/test` accepting an unsaved
config (reuse the `DbTester` probe) + a Test button in the form. *Treat as must-have (see D).*

**B2. ClickHouse enums display as integers — med impact, M**
`value_to_json` maps `V::Enum8/Enum16` to backing ints (`clickhouse.rs:1006-1008`). The label
mapping is in the column type string, already available via `Column::typed` (`clickhouse.rs:576`).

**B3. `SELECT *` buffers the whole table in daemon memory — med impact, M**
All three SQL read paths `fetch_all`/collect before truncating display to `max_rows`
(`mysql.rs:1476`, `postgres.rs:478`, `clickhouse.rs:583`). **Fix:** inject preview
`LIMIT max_rows+1` for un-limited `SELECT`s (streaming export already does it right).

**B4. `ErrorPanel` stale comment says Postgres isn't wired — trivial, S** (`ErrorPanel.svelte:11`).

### C. Enhancements

- **Shared "no silent Null" decode invariant** across drivers — the structural fix for the A1/A2 class.
- **Hex/base64 toggle in the cell inspector** (`ResultsGrid.svelte:363 openCell`) so binary is inspectable.
- **Distinct glyph/tooltip for decode-unsupported vs real NULL** until A1/A2 land.

### D. Deal-breaker

Pre-save connection validation (B1) is the one table-stakes gap. Otherwise the surface (SSH
tunnels, SFTP, import, query plan, inline edit, ER view, NL→SQL, Kafka produce/consume/offset
reset, schema registry) is already broad — close A1–A3 before adding modules.

---

## Agents — Findings

The area is mature — the orphaned-process leak, multibyte-trim panic, WS reconnect storm, and
PTY drop-kill are already fixed and regression-tested. These are the remaining edges.

### A. Bugs / correctness fixes

**A1. codex/agy session-id capture fails on symlinked cwds → silently non-resumable — high, S**
`spawn_session_id_capture` captures raw cwd (`crates/otto-sessions/src/manager.rs:1265`) and
compares exactly (`manager.rs:224`, `:296`). codex/agy resolve symlinks (macOS `/var`→
`/private/var`) so the compare never matches → no `provider_session_id` → conversation lost on
restart/suspend. `claude_pty.rs:83` already canonicalizes for this reason. **Fix:**
`std::fs::canonicalize(&session.cwd)` before compare/lookup.

**A2. Idle-suspend can kill a resumable agent mid-run on quiet work — med, M**
`suspend_idle_unattached` SIGKILLs unattached sessions idle > grace (`manager.rs:1591-1637`,
5 min default `:343`); "idle" == no output. A 6-min quiet build/test looks idle → PTY killed
mid-command, in-flight result lost. **Fix:** skip when the child has live descendant processes,
or require two consecutive idle sweeps.

**A3. Lost-update race on `session.meta` — med, M**
`resize()` (`manager.rs:1459-1469`) and `update_meta()` (`:1482-1498`) both do whole-JSON
read-modify-write with no lock; a resize racing a `keep_alive`/`issue` toggle silently reverts
it. **Fix:** SQL `json_set` merge for pty dims, or serialize meta writes.

**A4. Two divergent claude cwd→project-dir encoders — low, S**
`lifecycle.rs:37` (replaces `/`,`.`,`_`) vs `claude_pty.rs:245` (all non-alphanumerics — claude's
real convention). Fast path silently never matches for cwds with spaces/`@`; unify on the latter.

**A5. Scrollback search jump lands on the wrong line — low, M**
`goToServerMatch` calls `term.scrollToLine(match.line)` (`Terminal.svelte:193`) with the ring
buffer's *logical* line index (`ring.rs:36-52`) but xterm addresses wrapped/visual rows, and after
reconnect the client only holds ~1000 replayed rows (`otto-pty/src/lib.rs:277`). **Fix:** return
an opaque text anchor and re-locate via SearchAddon.

**A6. Viewer attached to a dead non-resumable session never sees a later respawn — low, M**
`serve_terminal` captures the PTY handle once (`ws.rs:415`); if `None` (`ws.rs:389`) it pends
forever even after another client restarts the session. **Fix:** re-subscribe on status→running.

### B. UX / flow simplifications

**B1. No "waiting for input / turn finished" state — the biggest gap vs competitors — high, M**
Status is only `Working` (output within 5s) or `Idle` (`manager.rs:1969-1972`). Finished-and-
waiting, asking-a-question, and mid-long-tool-call all render "Idle". The only "needs you" signal
is claude's Notification hook (`ui/src/lib/events.svelte.ts:329-337`) — codex/agy/shell never
produce it. **Fix:** derive a provider-agnostic awaiting-input signal (Idle right after sustained
Working, and/or trailing-prompt tail-scan) feeding the existing needsYou inbox. Unblocks B2, C1, D2.

**B2. "Idle · suspends in Nm" shown for active-but-quiet sessions — med, S** (after B1)
`SessionView.svelte:45-59` shows the countdown against genuinely busy sessions.

**B3. Can't start a session with a first task or model — med, M**
`NewSession.svelte` has no initial-prompt or model picker though the backend honors `meta.model`
(`manager.rs:57-69`) and has `submit_text`. **Fix:** optional "First task" textarea + model dropdown.

### C. Enhancements

- **Extend needs-you/auto-continue beyond claude** — pair B1's heuristic with `prompt_guard`'s
  per-provider tables (`prompt_guard.rs:40-74`).
- **"Re-fit terminal" affordance** — one click calling existing `safeFit`+`sendResize(true)`.
- **Make server-side scrollback search the primary find path** (bundled with A5).

### D. Deal-breakers

**D1. Per-session "what changed" review strip** — changed-files chip + one-click diff sheet in
the agent pane (pieces exist: otto-git diffs, session cwd, work graph). **L**
**D2. Follow-up task queue** — hold prompts while working, submit on awaiting-input (needs B1). **M**

---

## Git — Findings

### A. Bugs / correctness fixes

**A1. "Discard" on a staged rename loses the file — HIGH, S**
`LocalGit::discard` runs `git restore --staged --worktree --source=HEAD -- <path>` for
non-untracked/added kinds (`crates/otto-git/src/local.rs:684-697`) but only pushes `c.path`,
never `orig_path` (`parse.rs:111-127`). For staged `old→new`, restore finds `new` absent at HEAD
and *removes* it while `old` stays staged-deleted — the file vanishes. **Fix:** for `renamed`,
restore both `orig_path` and `path`.

**A2. Amend-without-message rejected instead of reusing the message — MED, S**
Composer enables Commit for amend with empty subject (`ChangesView.svelte:549`) and sends
`message: ""` (`:377-381`); server rejects empty and always passes `-m` (`local.rs:712-720`).
**Fix:** `git commit --amend --no-edit` when amend && empty; prefill HEAD's message in the UI.

**A3. Create-PR pushes the wrong branch when Source ≠ checked-out — MED, S–M**
`CreatePr.create()` always `POST /repos/{id}/push` (`CreatePr.svelte:80-81`) which pushes the
*current* branch (`http.rs:878-888` → `local.rs:732-765`), while the PR opens from the selected
`source_branch` (`CreatePr.svelte:89-93,111-113`). **Fix:** `git push origin <source>` explicitly.

**A4. Merge-commit diffs render empty/garbled — MED, S**
`DiffTarget::Commit` uses `git show` (`local.rs:591-593`) whose merge default is combined `--cc` —
omits files, `@@@` headers unparsed by `parse_diff` (`parse.rs:446-489,611-623`). **Fix:**
first-parent diff (`git show -m --first-parent` or `git diff <sha>^ <sha>`).

**A5. Untracked non-ASCII filenames missing from the working diff — MED, S**
`ls-files --others` output is quotePath-escaped (`local.rs:575-585`), then fed verbatim to
`git diff --no-index` which can't find it; the failure flag is discarded. **Fix:**
`-c core.quotePath=false` (and/or `-z` + NUL split).

### B. UX / flow simplifications

**B1. Conflicting cherry-pick/revert doesn't open the resolver — MED, M**
Raw `Err(stderr)` + comment admitting the graph UI doesn't auto-open the resolver
(`local.rs:814-827`). Route into the existing `ConflictResolverView` (`merge_status`/
`conflict_file`/`write_resolution` all exist).

**B2. Pull refuses on a dirty tree while merge auto-stashes — MED, S–M**
`merge_branch` has `auto_stash` (`local.rs:1109-1130`); `pull` is bare (`local.rs:767-768`).

**B3. Graph hard-capped at 200 commits, no Load more — MED, M**
`GraphView.svelte:187,218` vs `HistoryView.svelte:41-43,116` which pages properly.

**B4. No "Commit & Push" — LOW, S** (`ChangesView.svelte:373-395`).

### C. Enhancements

- **Server-side large-diff guard** — `FileDiff.too_large` is in the contract
  (`otto-core/src/api.rs:1168`) but hard-coded `None` (`parse.rs:553`, `providers/gitlab.rs:292`).
- **PR CI/checks + mergeable badge** in PrList/PrDetail (`ci_status` already called at create,
  `http.rs:1410`).
- **Invalidate open diff after external git ops** (auto-fetch updates status only,
  `stores/git.svelte.ts:429-435`).

### D. Deal-breakers

**D1. Hunk/line staging** — routes are whole-path only (`http.rs:97-98,834-854`;
`local.rs:651-669`; `ChangesView.svelte:74-101`); DiffViewer already renders per-line origins, so
add "stage selected lines" via computed patch + `git apply --cached`. **L**
**D2. Reword / squash-last-N** (unpushed-only guard). **L**

---

## Product — Findings

### A. Bugs / correctness fixes

**A1. Discovery-Chat "create canvas" produces a permanently empty canvas — high, S**
`canvas_doc_from_action` stores the legacy Svelte-Flow shape
(`crates/otto-server/src/product_chat.rs:621-635` — mermaid under `nodes[0].mermaid.src`), but
Canvas Studio reads only `{format, source}` (`ui/src/lib/stores/canvas.svelte.ts:120-122`;
`CanvasPage.svelte:24-25`). Result: "Open in Canvas" always lands blank. Same latent shape in
`empty_doc` (`crates/otto-canvas/src/types.rs:42`). **Fix:** emit
`{"type":"otto-canvas","version":1,"format":"mermaid","source":src}`.

**A2. Jira "Load more" duplicates rows on Jira Cloud — med, M**
`search_jql` paginates `/rest/api/3/search/jql` with `startAt` (`crates/otto-issues/src/jira.rs:380-391`)
but that endpoint is token-paginated (`nextPageToken`, ignored `startAt`); no token handling in
the crate; UI appends by offset (`SourceSearch.svelte:159`). **Fix:** thread `nextPageToken`, or
use classic `/search` for pagination.

**A3. Agent canvas turn clobbers concurrent user edits — med, M**
`assist_scene` snapshots at turn start (`canvas_assist.rs:107-109`) and commits last-write-wins
(`otto-state/src/canvas.rs:280` — no rev guard); UI ingests live pushes unconditionally even
mid-edit (`MermaidCanvas.svelte:246-251` + D2/Excalidraw twins). **Fix:** `updated_at`/rev
precondition on update; skip ingest while the source editor is dirty.

**A4. Story watcher only sees the first page of comments — med, M**
`JiraClient::list_comments` (`jira.rs:592`, `orderBy=created`, no paging) and Confluence
(`confluence.rs:470`) fetch one default page; newest comments — what `list_new_comments`
(`otto-product/src/service.rs:376-420`) needs — fall off the end. **Fix:** paginate to last page
or `orderBy=-created` + explicit `maxResults`.

**A5. Comment cursor can permanently drop a same-timestamp comment — low, S**
Strict `>` string compare (`service.rs:413`; cursor set at `product_watcher.rs:227-236`).
**Fix:** dedupe by comment id.

**A6. D2 WASM load failure is sticky for the session — low, S**
`_loading ??= import('@terrastruct/d2')` caches a rejected promise (`ui/src/modules/canvas/d2.ts:28`).
**Fix:** null `_loading` in `.catch` so the next call retries.

### B. UX / flow simplifications

**B1. Story workspace fragmented across 13 tabs — high, M**
`ProductPage.svelte:13-25`. The core arc (chat → canvas → mockup → questions → Jira) spans ≥4
destinations. Consider one Discovery surface showing draft + chat + linked canvases + mockups.

**B2. Discovery `apply_draft` has no path to Jira — med, M**
Only `update_draft_body` (`product_chat.rs:301`); add one-click "publish draft to source".

**B3. "Generating…" stub reads as stuck — med, S**
Mockup seeds literal stub HTML (`mockup_assist.rs:308-321`); tie visible progress to the turn's
running/idle state (already emitted via `MockupSessionStarted`).

### C. Enhancements

- **Collaboration-safe canvas** — agent-editing lock surfaced in UI + `updated_at` precondition (with A3).
- **Jira sync resilience** — distinguish 401/403 (reconnect) from 429 (honor Retry-After); watcher
  currently retries generically (`product_watcher.rs:273-293`).
- **Comment-derived question dedupe** — `poll_story` inserts every reconcile question with no
  dedupe (`product_watcher.rs:350-375`).

*(Checked and fine: mockup HTML sandbox — `sandbox=""`, interactivity adds only `allow-scripts`
behind a warning; `MockupViewer.svelte:6-10,135-136`.)*

### D. Deal-breakers

**D1. Live PRD canvas** — one always-in-sync, exportable doc stitching shaped draft + diagrams +
top mockup + open questions; mostly assembly of existing data. **M–L**
**D2. Bi-directional Jira sync** — surface upstream field/description changes as a reviewable
diff; push accepted local edits back (`refresh_story` diffing already exists,
`service.rs:320-366`). **L**

---

## Workflows — Findings

*(The known "0058 CHECK blocks new trigger kinds" gotcha is already fixed by
`0097_workflow_trigger_chat_kind.sql`.)*

### A. Bugs / correctness fixes

**A1. Node-result cache silently bypasses gating + side-effecting nodes — high, S**
Cache participates for every kind except `prepare_context` (`workflow_engine.rs:1010-1016`),
keyed `(workflow_id, node_id, params_hash, input_hash)` — global across runs. Deterministic
early-graph inputs → run #2 replays run #1's output: `human_approval` replays `{approved:true}`
(**human gate bypassed**, `:2335-2419` never run); `budget_gate` replays stale under-budget
(`:2296-2326`); `git_pr`/`channel_notify`/`swarm_task`/`http_request`/`api_run` replay success
**without the side effect**. **Fix:** extend the guard at read `:1010` / write `:1277` to a set of
excluded kinds.

**A2. `human_approval` times out after 120 seconds — high, S**
`NODE_AGENT_TIMEOUT` (`:382`) reused as the approval deadline (`:2360`); the shipped
`po-lifecycle` template has two approval nodes (`routes/workflows.rs:812,816`) no human can clear
in 2 min. Doc comment says 30s backoff, code sleeps 2s (`:2331-2333,2362`). **Fix:** dedicated
timeout (hours / `params.timeout_s` / none).

**A3. Event triggers: self-trigger explosion, no workspace scoping, `filter_json` inert — high, M**
`workflow_trigger_scheduler.rs:182` offers `workflow_run_updated` as an `event_kind`, but the
engine emits it on every node transition → N² unbounded spawns (uncapped `tokio::spawn` `:284`).
Listener matches only `event_kind` (`:229-235`), ignoring the event's `workspace_id`. Migration
0058 documents `filter_json`; never read. **Fix:** exclude engine-internal events / guard
re-entrancy; per-workflow in-flight cap; workspace scoping; implement or drop `filter_json`.

**A4. `http_request` and `api_run` nodes bypass the SSRF net-guard — med, S**
Raw clients, no check/redirect policy (`workflow_engine.rs:1947`, `:2524`) while every other
outbound path checks (`api_client.rs:899`, `api_stream.rs:154`, `grpc.rs:296`,
`run_callback.rs:68`, `scheduled_tasks_engine.rs:977`). **Fix:** route through
`net_guard::check_url` + `redirect_policy()`.

**A5. Run worktrees + `otto-wf/<run_id>` branches never cleaned up — med, M**
`provision_wf_worktrees` (`workflow_engine.rs:4316-4322`) creates them; grep finds no reap. An
hourly workflow leaves 24 worktrees+branches/day in the user's real repo. **Fix:** remove at run
finalize + startup sweep of `workflow-runs/`.

**A6. Schedule trigger has no overlap guard — med, S**
`workflow_trigger_scheduler.rs:62-131` fires regardless of an active run
(`RUN_WALL_CLOCK_TIMEOUT` is 10h, `:392`) — overlapping copies each cut worktrees (compounds A5).
**Fix:** skip when a run is in flight (policy field later).

**A7. Unknown/monthly cadence silently never fires — low, S**
`is_due` (`:142-150`) returns false for unknown cadences; `create/update_trigger`
(`routes/workflows.rs:1022-1091`) validate only `kind`. **Fix:** validate cadence/cron at write time.

**A8. Chat status summary labels non-legacy runs generically — low, S**
`run_status_summary` reads `run.input.name` (`workflow_chat.rs:523`) set only by the legacy path
(`:591`). **Fix:** fall back to workflow lookup by `run.workflow_id`.

### B. UX / flow simplifications

**B1. Scheduled/webhook/event runs deliver their result nowhere — high, M**
`deliver_run_result` early-returns without chat origin or webhook (`workflow_engine.rs:1627-1630`);
a nightly job completes silently. **Fix:** default `result_chat`/`result_webhook` on the trigger
spec, seeded into run input.

**B2. AI-generated workflows fail open to a trivial fallback with no signal — med, S**
`generate_graph` falls back to a bare 2-node graph on LLM failure (`routes/workflows.rs:266-283`,
`:319-348`) returning 200. Surface a degraded flag/toast.

*(Worth preserving: failed-run debugging is already strong — per-step logs, failure trace files
(`workflow_engine.rs:2730-2741`), context-dir browser, `final-output.md` rendering, and the
rev-guarded merge-in-place live sync (`WorkflowsPage.svelte:117-213`).)*

### C. Enhancements

- **Per-node cache opt-out + TTL** (`params.no_cache`; `get/set_cached_output` `:1011/:1279`).
- **Trigger concurrency policy** (`skip|queue|allow`) consumed by both schedulers (closes A3/A6).
- **`human_approval` timeout param + chat re-ping while paused** (pause is invisible in chat today).
- **Trigger validation + "next fire at…" preview** in `TriggersPanel.svelte`.

### D. Deal-breakers

**D1. `call_workflow` sub-workflow node** — all four `flow_templates`
(`routes/workflows.rs:703-985`) repeat the same prepare→implement→review→PR→report spine. **M**
**D2. "Retry failed steps" as a first-class run action** — machinery exists (`start_node` +
`descendants_inclusive`, `:633-637`); expose "re-run from first failed step" reusing cached
upstream outputs. **S–M**

---

## API Client — Findings

### A. Bugs / correctness fixes

**A1. Cookie jar is a single process-global shared across every user and workspace — high, M**
`cookie_jar()` is one `OnceLock` for the daemon (`routes/api_client.rs:69-77`), attached to every
request; `list_cookies`/`clear_cookies` take a `wid` but ignore it — and `list_cookies` returns
cookie **values** (`:578-615`). Session tokens captured by user A are re-sent on user B's requests
to the same domain and readable by any workspace viewer; staging and prod workspaces share one
jar. **Fix:** per-workspace (or per-user) jar map; per-request client built with that jar;
scope list/clear.

**A2. SSH-tunnelled requests killed by the local SSRF pre-check — med, M**
`build_and_send` runs `net_guard::check_url` (`api_client.rs:899`) *before* the SOCKS proxy;
`check_url` resolves locally (`netguard/lib.rs:84-113`). RFC1918 literals are rejected; hostnames
that only resolve on the bastion fail DNS locally — so the tunnel feature can only reach public
hosts. **Fix:** when `ssh_connection_id` is set, skip/relax the local resolve+block.

**A3. Auth secrets + response tokens written to history in plaintext — med/high, S**
`execute()` snapshots `req.auth` and the full response (incl. Set-Cookie) into `api_history`
(`api_client.rs:731-741,758-761`), Viewer-readable (`:522-534`) — contradicts the "DB stores only
opaque key references" rule. **Fix:** redact auth + `Authorization`/`Cookie`/`Set-Cookie` before
`insert_history`.

**A4. Saving a request drops scripts, per-request settings, docs, GraphQL variables — med, M**
`ApiRequest` persists only method/url/headers/query/body/auth/ssh (`otto-core/src/domain.rs:2068-2092`);
`saveDraft` sends exactly those (`apiClient.svelte.ts:584-604`) while the draft carries
`pre_request_script`/`post_response_script`/`settings`/`docs`/`graphql_variables` (`:57-87`) —
localStorage-only, not restored by `loadRequestIntoDraft` (`:993-1010`). **Fix:** migration +
extend `UpsertApiRequestReq` + round-trip.

**A5. Basic auth sends `Authorization: Basic Og==` for empty creds — low, S**
(`api_client.rs:1184-1198`). Skip when both empty.

**A6. Variable substitution single-pass; dynamics shadow user vars — low, S**
`substitute` (`api_client.rs:1528-1557`) checks `resolve_dynamic_var` first and never re-expands
nested `{{…}}`. Resolve user/env first + one bounded recursive pass.

### B. UX / flow simplifications

**B1. Cannot test localhost / LAN / private APIs at all — high, M**
`is_blocked_ip` rejects loopback/RFC1918/link-local/CGNAT with no opt-out (`netguard/lib.rs:25-63`);
docs confirm intentional (`docs/features/api-client.md:33,567-569`). A local dev server is the #1
target of an API client; Postman/Insomnia/Bruno/Hoppscotch all allow it. **Fix:** explicit,
off-by-default, per-workspace "Allow local & private targets" toggle for user-initiated requests
only. *Highest-leverage change in this area.*

**B2. Saving into a collection asks the user to type a number — med, M**
Two sequential modals, numbered text list (`RequestBuilder.svelte:631-659`). Replace with a
collection/folder tree picker.

**B3. Opening a saved/history request clobbers the active tab — med, S**
`loadRequestIntoDraft`/`loadHistoryIntoDraft` assign `this.draft` (`apiClient.svelte.ts:993,1013`),
destroying in-progress edits. Open-in-new-tab (or focus existing tab for the id).

**B4. History has no search/filter — low/med, S** (`HistoryList.svelte:40-63`).

### C. Enhancements

- **Import Postman environments** — `detectAndParse` (`importers.ts:26-37`) throws on
  `*.postman_environment.json`, silently losing variables.
- **Consolidate the two diverging cURL generators** — `toCurl` multipart-aware
  (`apiClient.svelte.ts:887-898`) vs `genCurl` `--data`-for-everything + `-X GET` (`codegen.ts:71-76`).
- **Postman import auth fidelity** — `pmAuth` maps only bearer/basic (`importers.ts:103-115`);
  collection/folder auth + `event` scripts ignored.
- **Secret-typed environment variables** — masked + Keychain-backed (`EnvSelector.svelte:118-119`).
- **HTML/PDF response preview** — images-only today (`ResponseViewer.svelte:132-137`).

### D. Deal-breakers

**D1. Collection/folder-level variables + auth inheritance** — collections carry only
name/parent/position; every benchmark tool leans on "set bearer once at the root". **L**
**D2. Data-driven runner iterations** — `run_automation` executes each step once
(`api_client.rs:1312-1339`); add CSV/JSON data-file rows. **M**

---

## Suggested execution order

1. **P0 sweep (1 short wave):** WF-A1, WF-A2, GIT-A1, CONN-A1, API-A3, AGT-A1, PROD-A1 are all
   S-effort and independent — one batch, one deploy. CONN-A2, API-A1, WF-A3 follow as the three
   M-effort P0s.
2. **Egress-policy decision** (theme 2) before touching API-B1 / API-A2 / WF-A4 so all three land
   coherently.
3. **AGT-B1 (awaiting-input)** early in P1 — it unblocks four other items (AGT-B2, AGT-C1, AGT-D2).
4. **CONN-C1 (no-silent-Null invariant)** with or right after CONN-A1/A2 to retire the class.
5. P2 flow work and P3 features per the tables above.
