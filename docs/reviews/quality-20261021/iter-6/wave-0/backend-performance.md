# Backend performance review — quality iteration 6, wave 0 (2026-10-23)

Lens: backend-performance. Rust hot paths: N+1 queries and queries in loops, missing indexes, blocking IO on the runtime, unbounded memory, needless clones and serialization, polling loops, per-WS-message work, ClickHouse/usage cost.
Worktree: `fix/quality-iter6-20261023` @ 02a64019b. Read-only review: no builds and no tests were run.

## Score

Start 10.0. Open findings: 0 blocker, 0 major, 1 minor, 4 nit (one of the nits is the residual of 5-04).

10.0 − 1×0.1 − 4×0.03 = 10.0 − 0.10 − 0.12 = **9.78 / 10**

Iteration 5 scored 9.65. Six of its seven findings are fixed. The seventh (5-04) is partly fixed: the column projection shipped, and pagination plus an `archived = 0` default were deferred as a product decision. It stays open as a nit. The four new findings are in the DB Explorer export path, the Mongo driver's completion cache, the MCP governance switch, and the product test-case listing.

## Previous findings (iteration 5)

| id | status | evidence in current code |
|----|--------|--------------------------|
| 5-01 a non-root `/ws/lsp` open scans every session row and canonicalizes each cwd | **fixed** | `crates/otto-server/src/routes/session_files.rs:351-379`: candidates are narrowed in SQL first: `archived = 0`, the caller's own rows or rows in a workspace the caller administers, and a cwd under the raw or canonical data dir (`substr(cwd,1,length(?))`). Only the survivors are canonicalized, on the blocking pool (`:385-396`). |
| 5-02 every non-admin History page decodes the caller's session history and resolves each session | **fixed** | `crates/otto-server/src/routes/transcript.rs:90-110` with `own_session_for_path` (`:160-…`) makes one indexed query on `transcript_path IN (?,?)` or `provider_session_id`. The fallback scan skips rows with fresh missing marks (`:120,130`) and persists a hit (`record_found`, `:143`). |
| 5-03 the story watcher builds and discards a full story detail per poll | **fixed** | `crates/otto-product/src/watcher.rs:255` calls `refresh_source(&story, &account, …)` (`crates/otto-product/src/service.rs:348`) with the story and account it already holds. |
| 5-04 the admin session overview returns every session ever created, fully decoded and unpaginated | **partly fixed; residual open (nit, re-listed as 6-05)** | `crates/otto-server/src/routes/admin_sessions.rs:149-152` projects 7 columns, with no `meta_json` decode. It is still `FROM sessions ORDER BY created_at DESC`, with no `archived` filter and no limit. FIXES (iteration 5, P08 "Not done") defers pagination and the default view as a product decision. |
| 5-05 the session-files routes skip the `/fs/*` admission and deadline guard | **fixed** | `session_files.rs:197` and `:283` go through `filesystem_work(admission(&ADMISSION), DEADLINE, …)`, with a cancel check per entry. |
| 5-06 the workspace transcript touch decodes the whole session history | **fixed** | `transcript.rs:480-492` uses `ctx.manager.live_agent_sessions(&wid)`. |
| 5-07 sidecar logs rotate only on (re)spawn | **fixed** | `crates/otto-telemetry-collector/src/collector.rs:24-28,223`: `LogRotator` copy-truncates every 600 s. `crates/otto-browser/src/lightpanda.rs:92,166-206`: `spawn_log_rotator` does the same, with the copy-truncate on `spawn_blocking`. |

## Areas covered (this iteration)

- Re-verification of every iteration-5 finding (table above).
- The iteration-5 fix diff (`8177c4d85..HEAD`, 120 crate files), checked for new hot-path costs:
  - `otto-git/collections.rs`: `confined_worktree_file` per file. `git_dir()` is a stat or read of `.git`, not a subprocess.
  - `otto-sessions/ws.rs` (`TerminalConn` split): the input path is `human_input` → `note_engaged` (in memory) → `write_async_authorized`. No DB on keystrokes.
  - `otto-sessions/mcp.rs` (agy git exclude, codex keys): it runs inside the `spawn_blocking` reconcile (`manager/mcp_sync.rs:110-150`) and resolves the git dir without spawning git.
  - `rooms/capability.rs` and the `rooms/registry.rs` / `game_rooms/registry.rs` refactor: same cost, and the token shape is now checked before the room lock.
  - Workflow `review_run.rs` `await_review`: event-driven, re-reading only status and agent rows.
  - `otto-k8s/monitor/scrape.rs`: the probe body is now read with a cap (`read_body_capped`). This is an improvement.
  - `otto-brokers/replay.rs` `Interrupted` guard: one write on drop.
  - Slack upload client (a shared `OnceLock` client with a 120 s timeout) and the mirror's single upload retry.
  - Budget gate fail-closed: `session_totals_for_budget` keeps its generation and TTL memo.
  - `otto-mcp` secret locks, the swarm prompt-editor columns, `delete_repo` transaction, scheduled-task and personal-agent report-error paths.
- Areas listed as not covered in iteration 5:
  - Workflow engine `update_run_progress`: per-node projection memo (`workflow_progress.rs:105-170`), serialization on the blocking pool, one `UPDATE … RETURNING rev`, log and activity bursts coalesced (`workflow_engine.rs` `log_persist.flush_at`). Not reported.
  - `otto-browser/src/live/proxy.rs` (128-connection cap, handshake and connect timeouts) and `cdp.rs` (one connection per fetch, bounded by the page timeout, per-navigation verdict cache). The page cache is capped by entries and bytes, and its gates are released. Not reported.
  - `otto-dbviewer` Redis driver (SCAN capped by keys and rounds, pipelined `TYPE`, `HSCAN`/`SSCAN` previews), `multirun` (50 targets, 200 runs, concurrency 8), and the `complete/` caches. Finding 6-02. The export path (finding 6-01) was found while doing this.
  - `otto-core::text::balanced_object_spans`: the O(n²) worst case needs many unclosed `{` or an unterminated string in a large agent reply. It is still not shown to be reachable at size, so it is not reported.
  - `otto-workgraph` service: broadcasts only on create or status change, high-volume signals go through `ingest_event` without a broadcast, and `work_events` has retention (`otto-state/src/retention.rs`). `otto-skills`: the bundled list is 32 embedded skills plus 32 small `SKILL.md` stat-reads per Skills Lab list. Not reported.
  - `apps/desktop/src-tauri`: `install_daemon` and `launchctl` polls run on `spawn_blocking`, window-frame saves are debounced by 400 ms, and health polls are bounded by a deadline. Not reported.
- A heuristic sweep of crates whose `clippy.toml` opts out of the blocking-call list (otto-git, otto-k8s, otto-browser, otto-channels, otto-design, otto-dbviewer, otto-context, otto-vault, otto-improve, otto-ssh, otto-transcript, otto-skills, otto-state, otto-core, otto-pty) for `std::process::Command`, `std::fs::read_dir` and `remove_dir_all`. Every production hit runs in a blocking closure, a sync-only helper, a `Drop` on a tiny directory, or boot (`Lightpanda::locate`'s `which`). The exception is the DB export file sink (6-01).
- A sweep for `unbounded_channel` in production code. LSP (backlog-capped), mcp_tools (bounded by the concurrent-call limit), the workflow pumps, the channels transcript tailers (bounded by the transcript on disk) and CDP (per-fetch) are fine. The exception is `/db/export` (6-01).
- A sweep for DB queries inside loops in crates outside `otto-state`. New hits were triaged: the AWS account list (2 access checks per account, a handful of accounts), network profiles, global search (8 sources, streamed with early stop, covering indexes on the heavy sources), `secure_all` (a one-shot admin action), and seed presets (a marker short-circuit). Finding 6-04.
- Room games (`game_rooms/socket.rs`, `registry.rs`): 32 rooms, 16 members, 20 KiB frames, token buckets. Same verdict as iterations 2 and 5: small rooms, not reported. The classroom is UI-only (`ui/src/modules/home/school/*`). It has no backend under this lens.

## Areas NOT covered yet (next iteration)

- `otto-state/src/workbench.rs` revision and blob store at scale (many revisions per doc).
- `otto-orchestrator` ⌘K plan parsing and `otto-agent-run` result watchers under many concurrent agents.
- `otto-git/src/providers/*` PR inbox fan-out across many repos (request counts per refresh).
- `otto-context` library and materialize: skill materialization per spawn (sync `fs::read_dir` helpers; the async call sites were not traced).
- `otto-design` service retention and GC at scale (the blob-store listing and usage walks were checked; the GC fence and orphan sweep were not).

## Findings

### backend-performance-6-01: DB Explorer exports write synchronously on a tokio worker, and `/db/export` buffers the whole result with no backpressure (minor)

**Where:**
- `crates/otto-dbviewer/src/export.rs:315-318`: `open_file_sink` is `std::fs::File::create` plus a 64 KiB `std::io::BufWriter`.
- `crates/otto-dbviewer/src/driver.rs:279-292`: the default `export_to_path` hands that sync writer to the async `export_to_writer`.
- `crates/otto-dbviewer/src/drivers/mysql.rs:914-1006`: an example writer loop. `rows.try_next().await` (async) feeds `sink.write_row(&cells)` (`:997`), a sync write into the `BufWriter`. Postgres, ClickHouse and Mongo use the same `ExportSink` over the same `w`.
- `crates/otto-dbviewer/src/http.rs:913-1000` (`export_to_path`, the route the UI's `ExportDialog.svelte:163` uses) runs this inside a `tokio::spawn`.
- `crates/otto-dbviewer/src/http.rs:789-855` (`/connections/{id}/db/export`): an `unbounded_channel` (`:796`) is fed by `ChannelWriter` (`:841-855`).

**Evidence:**
- Every 64 KiB the `BufWriter` flushes with a blocking `write(2)` on whatever tokio worker polls the export future, and `File::create` also runs there. Nothing moves this IO to the blocking pool. The crate's `clippy.toml` opts out of the blocking-call list, and plain `Write::write` is not on that list anyway.
- The export is uncapped by design ("stream an uncapped result to a file"), so a multi-GB export makes tens of thousands of blocking writes. The destination is user-chosen: an external, USB or network-mounted volume, or a disk under dirty-page throttling, can stall each write for tens to hundreds of ms while it holds a runtime worker.
- `/db/export` is the HTTP-streaming twin. Its comment admits "worst-case buffering is the full result": the driver's row loop never waits on the consumer, so a slow reader (an API client or a remote/shared session over a slow link) lets the daemon queue the whole uncapped result in memory. The current UI does not call this route (only `export-to-path`), which limits the impact to API callers.

**Cost model:**
- Export-to-path: about one blocking syscall per 64 KiB on a runtime worker, for the length of the export. That is harmless on a fast local SSD. On a slow volume, each stall takes away 1 of N workers, and the PTY relay, WS fan-out and HTTP routes share those workers.
- `/db/export`: memory proportional to (rows produced − bytes consumed). For a 10M-row table and a stalled reader, that is GBs of `Bytes` chunks.

**Fix:**
- Give `export_to_writer` a bounded async hand-off. The driver loop keeps writing rows into an in-memory `ExportSink` over a `Vec<u8>`. When the buffer passes 64 KiB it `send(chunk).await`s on a `tokio::sync::mpsc::channel(8)`, so the driver waits when the consumer lags.
- For `export_to_path`, the consumer is one `spawn_blocking` task that owns the `File` (`create` included) and drains with `blocking_recv`.
- For `/db/export`, the bounded receiver is the response body stream, which replaces `ChannelWriter` and the unbounded channel.
- Keep the 300 ms byte-count ticker as it is.

### backend-performance-6-02: the Mongo driver never sweeps its completion cache, so expired snapshots and per-collection field lists stay resident (nit)

**Where:**
- `crates/otto-dbviewer/src/drivers/mongodb.rs:60` (`completions: CompletionCache`, created with no capacity) and `:733-739` (`evict_idle`).
- Compare `crates/otto-dbviewer/src/drivers/mysql.rs:587-591`, `postgres.rs:545-548` and `clickhouse.rs:2800-2804`, which call `self.completions.sweep()`.
- `crates/otto-dbviewer/src/complete/mod.rs:570-575` (`sweep` doc: "snapshots, Mongo field lists, first-db probes … Called from the service's 5-minute reaper (DB2-07)") and `:604-610` (`prune_for_insert` returns at once when `capacity` is `None`).
- `crates/otto-dbviewer/src/service.rs:1528-1545` says "drivers sweep their own in `evict_idle`".

**Evidence:**
- Mongo's `evict_idle` drops idle clients and invalidates their `samples`. It never calls `self.completions.sweep()`, and nothing else does.
- `get_*` only filters stale entries. `put_*` prunes only with a capacity, and the driver cache has none.
- So every `(cache_key, db)` collection-list snapshot, every `(cache_key, db, collection)` field list built by `completion_fields` (`mongodb.rs:1463-1475`), and every `first_db` probe stays in memory until the same key is rebuilt or the connection is refreshed. This is the exact leak DB2-07 fixed for the three SQL drivers. Mongo is the one engine with per-collection field lists, and it was missed.

**Cost model:** memory grows with the number of distinct databases and collections the user has typed against since boot. That is KBs per collection (index paths plus sampled field paths), held for the daemon's lifetime: small, but unbounded.

**Fix:**
- Add `self.completions.sweep();` at the top of `MongoDriver::evict_idle`, as MySQL does.
- When a client is evicted, also call `self.completions.invalidate(key)` for each evicted key, next to the existing `samples.invalidate_where`.

### backend-performance-6-03: the MCP governance switch runs launcher-file IO and a std `Mutex` on a runtime worker, once per distinct cwd in the workspace's history (nit)

**Where:**
- `crates/otto-server/src/routes/resource_access.rs:144-164` (`AccessCtx::retire_mcp`). It loads `self.manager.list_by_workspace(&server.workspace_id)` (every session, archived included), collects every distinct `session.cwd`, then calls `otto_sessions::mcp::retire_user_server(&root, …)` for each, inline in the async fn.
- Callers: the policy preview (`:700`, `apply = false`) and the policy save (`:423`, `apply = true`).
- `crates/otto-sessions/src/mcp.rs:1057-1110` (`retire_user_server`): `cwd_lock(workspace_root)` is a `std::sync::Mutex` (`:124-131`, `:1058-1059`). Then, for each of `.mcp.json` and the agy file, `path.exists()` and `read_doc`. Then `read_grok_doc`, and writes when `apply`.

**Evidence:**
- The session spawn path calls the same family of functions (`reconcile_managed_servers*`) only on the blocking pool. `crates/otto-sessions/src/manager/mcp_sync.rs:110-111` says why: "run them … on ONE blocking-pool thread, so a contended cwd parks that thread, never a tokio worker".
- `retire_mcp` breaks that rule. It takes the same per-cwd std mutex and does sync stat, read and write work directly on the runtime. A concurrent spawn in one of those cwds that holds the lock parks the worker.
- The iteration-4 note on `retire_mcp` accepted the full-history load, because every historical cwd needs retiring. The cost here is where that work runs, not how much there is.

**Cost model:** a rare admin action (switching an MCP server from legacy to governed, preview then save). For W distinct historical cwds (thousands of worktrees on a long-lived install), each call makes about 3W stats plus reads on one tokio worker: tens to hundreds of ms of blocked worker per call.

**Fix:**
- Collect the roots in async code (the live-session kill loop stays there).
- Run the `for root in roots { retire_user_server(…) }` loop inside one `tokio::task::spawn_blocking`, and return its first error.
- Optionally narrow the session read to a `SELECT DISTINCT cwd` projection instead of `list_by_workspace` (`SELECT *` plus a meta parse).

### backend-performance-6-04: the story test-case tab loads each run's cases with its own query (nit)

**Where:**
- `crates/otto-product/src/http.rs:965-978` (`list_testcase_runs`). It calls `list_testcase_runs(&sid)`, then `ctx.product_repo().list_testcases(&run.id)` once per run, in a loop.
- Each call is `SELECT * FROM product_testcases WHERE run_id = ? ORDER BY order_idx, created_at` (`crates/otto-state/src/product.rs:1535-1543`).

**Evidence:** a classic N+1. Every "generate test cases" adds a run to the story and the route never pages, so the query count grows with each regeneration (1 + R per open of the tab), and each query decodes full test-case rows.

**Cost model:** 1 + R pool round trips per tab load. R is usually small (re-runs per story), so the cost is low, but it grows without limit.

**Fix:**
- Add `ProductRepo::list_testcases_for_runs(&[Id])`: one `WHERE run_id IN (…)` query (chunked at 500), ordered by `run_id, order_idx, created_at`.
- Group the rows in memory into `ProductTestcaseRunDetail`. Optionally cap or paginate runs (newest N).

### backend-performance-6-05 (residual of 5-04): the admin session overview still returns every session ever created, unpaginated (nit)

**Where:** `crates/otto-server/src/routes/admin_sessions.rs:149-152`: `SELECT id, workspace_id, created_by, kind, provider, title, status FROM sessions ORDER BY created_at DESC`, with no `WHERE` and no `LIMIT`. Called on every page open and after every terminate or evict (`ui/src/modules/settings/AdminSessions.svelte`).

**Evidence:**
- The projection fix removed the `meta_json` decode. The row count is unchanged: archived history, review reviewers and swarm workers are all returned.
- The "active-sessions overview" still sends O(all sessions) of JSON (20k rows on an old install).
- FIXES (iteration 5) defers pagination and the `archived = 0` default as a product decision.

**Cost model:** O(all sessions) of rows, a JSON response and DOM rows per admin open or action. The page is admin-only and rarely opened.

**Fix:**
- Default to `archived = 0` (or live sessions plus the last N exited), with keyset pagination on `(created_at, id)`.
- Keep an "include archived" toggle for the full listing.
