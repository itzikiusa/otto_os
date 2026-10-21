# Backend performance review — quality iteration 1 (2026-10-21)

Lens: backend-performance (Rust hot paths: N+1 / queries in loops, missing
indexes, blocking IO on the runtime, unbounded memory, needless
clone/serialization, polling loops, per-WS-message work, ClickHouse/usage cost).
Worktree: `fix/quality-loop-20261021` @ d447516c4. Read-only review (no builds).

## Score

Start 10.0. Open findings: 2 major, 7 minor, 1 nit.

10.0 − 2×0.3 − 7×0.1 − 1×0.03 = 10.0 − 0.6 − 0.7 − 0.03 = **8.67 / 10**

## Previous findings

None. This is iteration 1 of this loop.

## Areas covered

- The usage tailer's new native Grok and Antigravity accounting (`ottod/src/usage_tailer*.rs`, f4f4eac7d): run loop, checkpoint and journal commit (`otto-usage/src/ingestion.rs`)
- Usage HTTP routes (`routes/usage.rs`): enrichment and rollup are already batched
- Git refs and landed-marks (`otto-git/src/local.rs`, eb295dcc2): `refs_with_base`, `merged_sets_cached`
- Workflow engine polling loops (`workflow_engine.rs`): CancelWatch, backoff, human_approval, code_review await
- run_engine review waiting, as the comparison baseline
- Session-input fan-out (`modules.rs`): broadcast, relay, `wait_session`, `input_session`
- `/ws/events` fan-out and authorization caches (`ws_events.rs`)
- Session WS handler (`otto-sessions/src/ws.rs`), skimmed
- Session status ticker (`manager.rs`)
- Mission control builder (`routes/mission.rs`): already batched
- Project sessions page (`routes/projects.rs`)
- Command-palette search (`routes/search.rs`, `otto-state/src/search.rs`) and the column order of the tables it reads
- Workbench repo and its indexes (migrations 0165/0184)
- 0186 background-scan indexes
- Game-room socket relay (backend only)
- Rooms recap finalize tick
- Database Explorer: multi-run store and the approved-change execution polls
- Slack directory cache
- Assistant polling loops
- A scripted scan for `std::fs` / `std::process` calls inside `async fn` bodies in the crates that opt out of the clippy blocking-call guard. Remaining hits are cold paths (k8s import/delete, sqlite maintenance, browser start). None is reported.

## Areas NOT covered yet (next iteration)

- `otto-sessions/src/manager.rs` beyond the status ticker: spawn/resume/archive paths and `busy_for_restart`
- `otto-sessions/src/mcp.rs` (agy MCP servers, 8a9897823)
- `ottod/src/mcp_tools.rs`, `mcp_outward.rs` / `mcp_http.rs`, and the governance per-call DB budget (only seen through its op_count tests)
- `transcript_tail.rs`, `transcript_cache.rs`, `routes/transcript.rs`. Long-transcript paging was only skimmed.
- `otto-vault` engine, watch and reindex
- `otto-design` service, retention and blob store
- `otto-dbviewer` drivers: result materialisation and row caps per engine
- `otto-brokers` replay and consume
- `otto-browser/src/live/*` (CDP proxy, screencast frames per WS message)
- `otto-k8s` monitor, gateway and list cache
- `otto-aws` logs and S3 download
- `otto-swarm` runtime scheduler and wake
- `otto-automation` scheduled-task supervisor tick
- `otto-assistant` personal-agents scheduler
- `otto-insights`
- `otto-usage/src/clickhouse.rs` query shapes (FINAL usage, partition pruning)
- `state_archive`
- Rooms backend: `rooms/socket.rs`, `terminal.rs`, `recap_engines`
- `otto-channels` mirror / transcript (100–500 ms sleeps)

## Findings

### backend-performance-1-01: native usage state grows without bound and is re-serialized on every tailer commit (major)

**Where:**
- `crates/ottod/src/usage_tailer/native.rs:10-19`, `:46-49`, `:61`, `:146`
- `crates/ottod/src/usage_tailer.rs:553-563` and `:600-623`
- `crates/otto-usage/src/ingestion.rs:73-102`, `:153-177`

**Evidence:**
- `NativeState { totals: HashMap<String, NativeTotals>, aliases: HashMap<String, String>, .. }` gets one entry per Grok turn and one per Antigravity step:
  - Grok key: `format!("grok:{}:{}", path.display(), row.dedupe_key)`, which embeds the full file path.
  - Antigravity: one `totals` key per step, plus one `aliases` entry per alias of each row (`delta_aliased`, lines 46-49).
- Nothing ever removes entries. `native.rs` and `usage_tailer.rs` contain no `retain` or `remove` on `totals` or `aliases`.
- Every other piece of tailer state has a cap: `SEEN_KEYS_CAP = 100_000`, `CODEX_COUNTERS_CAP = 20_000`, `RECENT_RESPONSES_CAP = 4_096`.
- The comment on `persist` says "Serialization of the bounded state", but `native` is not bounded.
- Every non-idle pass does the following work, all O(total state):
  - `persist()` builds the whole checkpoint (`"native": self.native`) as a `serde_json::Value`.
  - `IngestionJournal::commit` clones it (`checkpoint.clone()`) and `flatten`s every leaf into a `BTreeMap<String, String>`. Each totals entry becomes about 6 leaves: model, input, output, cache_read, cache_write, cost_ticks.
  - It then compares that map against the previous map and keeps the new one in memory (`*previous = next`).
- Every leaf is also a row in `checkpoint_parts`, so the SQLite table grows forever as well.

**Cost model:** A pass runs every FS-debounced burst (2 s) while any Claude or Codex agent writes, and at least every 20 s otherwise. A heavy Grok or Antigravity user adds thousands of keys a week. After months, every commit re-encodes and diffs hundreds of thousands of leaves. That work runs on `block_in_place` and the blocking pool, and holds about 3 transient copies of the state.

**Fix:**
- Bound `native` like the other maps.
  - Grok: key the baseline per *file*, not per turn. `updates.jsonl` is append-only, and the cursor already dedupes replay. Alternatively, evict `grok:<path>:*` keys when the cursor for `<path>` is evicted (`evict_dead_cursors`).
  - Antigravity: keep baselines only for conversations whose DB still exists. Drop a conversation's keys and aliases when its file disappears from `agy::discover`.
  - Add a FIFO cap like `SeenKeys` as a backstop.
- Store keys relative to the provider root, not as absolute paths.

### backend-performance-1-02: Canvas source of command-palette search reads every scene's multi-MB overflow chain (major)

**Where:**
- `crates/otto-state/src/search.rs:66`, `:91-120`
- Schema: `crates/otto-state/migrations/0072_canvas_scenes.sql`; `section` added later by `ALTER TABLE canvas_scenes ADD COLUMN section`

**Evidence:**
- The Canvas projection is `SELECT id,title,section AS secondary,section AS subtitle,updated_at FROM canvas_scenes WHERE workspace_id=?1 ORDER BY updated_at DESC`.
- `section` was added with `ALTER TABLE`, so it sits physically after `doc_json` (the full scene JSON) and `thumbnail` (a data URL).
- To read it, SQLite must walk each row's overflow-page chain. Migration `0186_background_scan_indexes.sql` already documents this exact cost for canvas rows ("following each row past its multi-megabyte `doc_json` / `thumbnail` to reach the column").
- `search()` streams candidates and stops only after `limit` matches. A query with few or no canvas matches therefore touches every scene in the workspace.
- The route runs on every debounced palette and FloatingBar keystroke (`ui/src/shell/Palette.svelte:98`, `ui/src/lib/components/FloatingBar.svelte:234`).

**Cost model:** Disk I/O per search is O(total canvas bytes in the workspace). For example, 50 scenes × 1–2 MB is 50–100 MB of page reads per palette query.

**Fix:**
- Add a covering index in a new migration: `CREATE INDEX IF NOT EXISTS idx_canvas_scenes_search ON canvas_scenes(workspace_id, updated_at DESC, id, title, section);`. The query is then answered from the index alone and never touches the row.
- Or drop `section` from the projection: match on `title` only, and fetch the section for the ≤5 hits.
- Same class, smaller impact: `ApiRequest` reads `position` and `updated_at`, which sit after `body`/`auth_json`. That source would benefit from `(workspace_id, position, name, id, method, url, updated_at)`.

### backend-performance-1-03: tailer polls Grok and Antigravity trees on every pass, including a 20 s idle wake (minor)

**Where:**
- `crates/ottod/src/usage_tailer.rs:308-312`, `:330-337`, `:499`
- `crates/ottod/src/usage_tailer/native.rs:106-117`, `:197-206`

**Evidence:**
- The FSEvents watcher only watches the Claude and Codex roots (`w.ensure_roots(&roots)` with `claude_roots` plus `codex_root()`).
- The loop sleeps `timeout.min(SCAN_INTERVAL)`, which is at most 20 s, and then runs `scan_paths(paths)` even when `paths` is empty.
- `scan_paths` always calls `scan_native()`, which does three things:
  - Runs `discover_update_files`, a recursive walk of `~/.grok/sessions`.
  - Runs a `tokio::fs::metadata` (one blocking-pool hop) for **every** Grok `updates.jsonl` ever written.
  - Runs `agy::discover`, which does a `read_dir` and two `stat`s per Antigravity DB.
- Before f4f4eac7d, an empty `scan_paths` cost nothing. Full listings only ran on the 600 s `RECONCILE_INTERVAL`.

**Cost model:** O(all Grok session files + all Antigravity DBs) stats every ≤20 s, forever. During any Claude or Codex activity it also runs on every 2 s debounced burst. This holds even when Grok and Antigravity have been idle for weeks.

**Fix:** Pick one:
- Add the Grok sessions root and the Antigravity conversation dirs to the watcher's `ensure_roots`, and run `scan_native` only when one of their paths is dirty or on the reconcile pass.
- Or rate-limit `scan_native` to the reconcile cadence when no native file changed. Remember per-root directory mtimes and skip the walk when they are unchanged.

### backend-performance-1-04: an active Antigravity conversation DB is fully re-read on every change (minor)

**Where:**
- `crates/ottod/src/usage_tailer/native.rs:206-213`
- `crates/ottod/src/usage_tailer/native/agy.rs:65-104`

**Evidence:**
- When a DB's `(db, wal)` size/mtime fingerprint changes, `agy::read` opens a new SQLite connection and runs `SELECT idx, data FROM gen_metadata`, `SELECT data FROM trajectory_metadata_blob`, and `SELECT idx, metadata FROM steps WHERE metadata IS NOT NULL`.
- Those queries read every row of every table, and `parse_agy_conversation_blobs` re-parses all of them.
- Every row then goes through `delta_aliased`, a HashMap lookup plus alias inserts, only for unchanged steps to return `None`.
- The WAL fingerprint changes on every write of a live conversation.
- `agy_files` is `#[serde(skip)]`, so after a daemon restart every Antigravity DB is re-read in full once.

**Cost model:** A live conversation of S steps costs O(S) per pass (every ≤20 s, or 2 s during Claude activity). Over the conversation's life that is O(S²) total.

**Fix:**
- Persist a per-conversation high-water mark: the max `steps.idx` already accounted, plus the gen/trajectory row counts.
- Query only `WHERE idx > ?`, plus the small set of trailing steps the provider may still revise in place, for example the last N or rows whose metadata blob size changed.
- Persist `agy_files` fingerprints in the checkpoint so a restart does not re-read every DB.

### backend-performance-1-05: workflow `code_review` node polls the full review and all its comments every 2 s (minor)

**Where:**
- `crates/otto-server/src/workflow_engine.rs:5613-5646`
- Compare `crates/otto-server/src/run_engine.rs:682-695`
- `crates/otto-state/src/reviews.rs:272-291`

**Evidence:**
- The `await_done` loop does `sleep(2s)`, then `ctx.reviews_store.get_review(&review_id)` (`SELECT * FROM pr_reviews` plus `comments_for_review`, which decodes **every** finding), then `is_canceled(run_id)`, for the review's whole lifetime (up to `timeout_s`).
- It only needs `status` and the agents' `session_id`s.
- `ReviewsRepo::review_status` exists precisely "for pollers waiting on completion, which used to load the whole row … plus every comment each tick".
- `run_engine` already waits event-driven (`wait_until_event` on `Event::ReviewChanged` plus a `review_status` safety recheck).

**Cost model:** A 15-lens review with 200–400 comments running 15–30 min means 450–900 polls, each decoding every comment row. This is paid per concurrent workflow review.

**Fix:**
- Reuse run_engine's `wait_until_event` pattern on `ReviewChanged`.
- Read status with `review_status`, and session ids with a narrow `SELECT status, agents_json FROM pr_reviews WHERE id=?` (a new `review_agents` repo method).
- Load the full review only on the timeout/cancel path, which needs `cancel_running_review(&review)`.

### backend-performance-1-06: `GET /sessions/{id}/wait` polls the session row every 500 ms (minor)

**Where:** `crates/otto-server/src/modules.rs:6231-6273` (`MAX_WAIT_SESSION_SECS = 25`, line 6071)

**Evidence:**
- Each iteration runs `sleep(500ms)` then `ctx.manager.get(&session_id)`.
- `manager.get` is `self.repo.get(id)`: `SELECT *` plus `row_to_session`, which includes decoding `meta_json`.
- Status transitions are already broadcast on `ctx.events` as `Event::SessionStatus { session_id, status, .. }`, sent by the status ticker in `manager.rs`.

**Cost model:** About 50 full-row reads and decodes per 25 s wait, per waiter. Orchestrator agents call this in a loop (MCP `otto_wait_session`) for every worker they supervise. Twenty supervised workers is about 40 row decodes/s, sustained.

**Fix:**
- Subscribe to `ctx.events` before the first read, then wait on `SessionStatus` / `SessionRemoved` for this id, with a slow safety re-read (for example every 5 s, and on `Lagged`).
- Re-read the row only when an event for this session arrives.

### backend-performance-1-07: broadcast and relay scan the whole workspace history, then repeat per-target authorization 3× (minor)

**Where:** `crates/otto-server/src/modules.rs:43-70`, `:95-147`, `:1303-1315`

**Evidence:**
- `input_agents` calls `ctx.manager.list_by_workspace(ws_id)`. That is `SELECT * FROM sessions WHERE workspace_id = ?` with no archived or status filter (`otto-state/src/sessions.rs:386-393`), so every archived or exited session's `meta_json` is decoded before being filtered out in Rust.
- For each live agent it then runs `input_session`: `sessions.get`, `users.get`, `role_of`, and the `check_with_pool` user reload.
- `broadcast_sessions` then calls `submit_session_text` for each target, which runs `input_session` again, and `human_submit_text_checked`'s closure runs it a third time.
- The user and role rows are identical for every target.

**Cost model:** One workspace-wide history decode plus about 12 queries × live agents per broadcast or relay message. Relay runs for every name-addressed chat message.

**Fix:**
- Use the SQL-filtered `list_filtered` with `archived: Some(false), kind: Some("agent")` (as `routes/mission.rs` does), filtering status in SQL.
- Load the user and the workspace role once per request.
- Pass the already-authorized `Session` into `submit_session_text` and keep only the single re-check inside `human_submit_text_checked`.

### backend-performance-1-08: project sessions page authorizes row-by-row instead of using the batched checker (minor)

**Where:**
- `crates/otto-server/src/routes/projects.rs:78-118`
- `crates/otto-server/src/auth.rs:172-179`
- `crates/otto-server/src/resource_sessions.rs:96-120`

**Evidence:**
- The handler already computes `admin` and pushes the owner filter into SQL (`session_page(.., owner, ..)`).
- It still calls `require_session_owner_or_admin` for each of up to 500 rows. For a non-root admin that is one `roles.check` → `role_of` query per non-owned row; `RbacRoleChecker` has no cache.
- For every k8s, aws or db_assist-bound row, `resource_sessions::check` reloads the user (`UsersRepo::get`) and re-decides the same (resource, op).
- `resource_sessions::check_many` exists precisely to batch this for lists (comment: "the list used to reload the user and the page grants for every k8s/aws/db row (F9)").

**Cost model:** Up to 500 × (1 role query + bound-row checks) per page load.

**Fix:**
- Drop the per-row owner/admin check. The SQL owner filter plus the precomputed `admin` already decide it.
- Run each page through `resource_sessions::check_many(&ctx, &user, page.items)`.

### backend-performance-1-09: `refs_with_base` pays an extra rev-parse per integration branch on every call, and 2 `branch --merged` per integration on any ref move (minor)

**Where:**
- `crates/otto-git/src/local.rs:2332-2337`, `:2496-2514`, `:2550-2558`
- Doc comment at `:805-810` ("costs no extra spawn")

**Evidence:**
- Since eb295dcc2, `refs` computes merged sets for every integration branch: the base plus develop/main/master.
- `merged_sets_cached` begins with `self.resolve_commit(b)`, a git spawn, before even checking the memo. That is 1–3 extra spawns per `refs` call, on every 30 s auto-fetch of every open repo, even when nothing moved.
- The tip being resolved is already in `local_out` / `remote_out`, which are hashed into the memo key.
- On a miss (any ref moved, which is common after a fetch in a busy repo), each integration runs **two** `git branch [-r] --merged <base>` walks. That multiplies the pre-existing cost by up to 3× for repos with thousands of remote branches.

**Fix:**
- Read each integration's oid from the for-each-ref listings already in hand and drop the `resolve_commit` spawn.
- Replace the local/remote pair with one `git for-each-ref --merged=<base> --format=%(refname) refs/heads refs/remotes` per integration.
- Optionally reuse the previous sets when only non-integration remote refs moved: re-test only the changed tips with `merge-base --is-ancestor`.

### backend-performance-1-10: `human_approval` node polls its run row every 2 s for up to 24 h (nit)

**Where:** `crates/otto-server/src/workflow_engine.rs:4585-4625`

**Evidence:**
- Parked approvals run `loop { sleep(2s); SELECT status, waiting_approval, approved_by, approval_note FROM workflow_runs WHERE id = ? }` until decided. The default `timeout_s` is 86 400, which is about 43 k reads per parked run.
- The same file's `CancelWatch` already shows the announced pattern (`subscribe_cancels` plus a slow safety tick).

**Fix:**
- Announce approve/reject through a broadcast, like `request_cancel` → `subscribe_cancels`, and wake on it.
- Keep a slow (for example 30 s) safety read.
