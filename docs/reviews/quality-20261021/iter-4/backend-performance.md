# Backend performance review — quality iteration 4 (2026-10-22)

Lens: backend-performance. This covers Rust hot paths: N+1 queries and queries in loops, missing indexes, blocking IO on the runtime, unbounded memory, needless clones and serialization, polling loops, per-WS-message work, and ClickHouse/usage cost.
Worktree: `fix/quality-loop-20261021` @ 6cff08270. This was a read-only review: no builds and no tests were run.

## Score

Start 10.0. Open findings: 0 blocker, 0 major, 3 minor, 3 nit.

10.0 − 3×0.1 − 3×0.03 = 10.0 − 0.30 − 0.09 = **9.61 / 10**

Iteration 3 scored 9.64. Four of the five iteration-3 findings are fixed. The fifth (3-04) is still open, and FIXES lists it under "Not done". The other five findings are new and come from areas no earlier iteration covered: state restore, the vault scan's non-note files, the session spawn and relay paths, the Mission Control projector, and the design and canvas assist polls.

## Previous findings (iteration 3)

| id | status | evidence in current code |
|----|--------|--------------------------|
| 3-01 review summarizer polls a full review load every 100 ms | **fixed** | `run.rs:654-661` `review_is_cancelled` now reads the one-column `review_status`. `summarize.rs:165-191`: the in-memory flag is checked every 100 ms. The DB is read only on this review's `ReviewChanged`, on a lag, or on the `SUMMARIZER_CANCEL_RECHECK` tick. The draft loop checks the DB once per `CANCEL_CHECK_BATCH` (25) comments (`summarize.rs:13,306`). |
| 3-02 browser union runs one session SELECT per live session on every spawn | **fixed** | `manager/mcp_sync.rs:81-102` makes one `browser_session_cwds` query (`sessions.rs:657-671`, `json_each` id set plus `json_extract(meta_json,'$.browser') = 1`). The reconcile and canonicalize work runs on one blocking-pool thread (`mcp_sync.rs:109-…`). |
| 3-03 state-archive export sorts by re-serializing rows in the comparator | **fixed** | `state_archive/schema.rs:582-600` now orders rows in SQL (`ORDER BY` pk columns, then every column). `state_archive.rs:213`: file walk and assembly run in `spawn_blocking`. A linear residual is not re-reported: each row is still serialized once for the per-row size check (`schema.rs:628`) and once more per table (`state_archive.rs:186` `json_len`) on the runtime. |
| 3-04 History root-scans rows with no persisted path whose transcript is gone | **still open** (nit, re-reported as 4-06) | `history_page.rs:179-205`: only `Some((r, true))` is persisted. A miss is still not recorded. FIXES: "migration 0191 (`transcript_missing_at`) is not written." |
| 3-05 Outputs panel on a live session re-folds the whole file | **fixed** | `transcript.rs:705-718` tries `transcript_tail::live_artifacts` (`transcript_tail.rs:343`) first and calls `cached_fold` only when no tail runs. |

## Areas covered (this iteration)

- Re-verification of every iteration-3 finding (table above).
- The iteration-3 fix diff (`3dd55d9f2..HEAD`, 139 crate files), checked for new hot-path costs:
  - swarm `plan_rebalance` and `take_escalation_slot` (pure refactor, same cost);
  - `ottod/mcp_tools` per-request budgets and the client without a default timeout;
  - `usage_tailer` startup retry backoff;
  - `run_acting_user` (one indexed query per run, `idx_workflow_triggers_workflow`);
  - `KeyedLocks` (Weak entries, pruned on insert);
  - the assistant orphan-inbox sweep (hourly, one query per user);
  - `browser_session_cwds`.
- `ottod/mcp_tools` `resolve_native_refs` → `/refs/resolve` → `agent_refs::load_candidates`. This was listed as not covered. Only name references (not ids) are resolved. The per-workspace list fan-out is what name-ambiguity detection needs, so it is not reported.
- `otto-aws`: logs are paginated and capped (`INSIGHTS_MAX_*`). S3 preview is capped (64 KiB default, 1 MiB max, 25 MiB inline). Download streams to a file. Listed as not covered.
- `otto-k8s` resource list: 10 s `list_cache` plus single-flight, list gateway, `spawn_blocking` body build, ETag/304. Listed as not covered.
- `otto-swarm` board and graph: `detail` counts instead of decoding, `graph` makes one `latest_task_sessions` projection, the board is capped at 300. Listed as not covered.
- `otto-apiclient`: TTL descriptor-pool and channel caches, proto compile in `spawn_blocking`, `pm` scripts out of process behind a semaphore. Listed as not covered.
- `otto-ssh` and SFTP: the control socket setup is a few small sync fs calls, and transfers are done by the `sftp` binary. Listed as not covered.
- `state_archive` restore and preview. Listed as not covered. Finding 4-01.
- `otto-vault` full scan. Notes were reviewed before; non-note files and the prune loop were not. Finding 4-02.
- Session-history listings across the daemon. A scan for `list_by_workspace` callers found these, and one is on the session spawn path. Finding 4-03.
- Mission Control `workgraph_projector`: the live loop and the reconcile. Finding 4-04.
- Design-assist and canvas-assist live polls. Finding 4-05.
- `otto-usage` ClickHouse query shapes:
  - every dashboard rollup has an `event_date` predicate (partition prune);
  - `OPTIMIZE … FINAL` is a one-time, marker-guarded run;
  - `session_totals_for` is on-demand only (item detail, `/usage/sessions/{id}`).
- `feature_guard` per-request path (root bypasses the grant lookups). The `/events` WS shares one serialized frame per event across sockets (`ws_fanout`). The terminal WS live re-auth is deduplicated per session.
- `otto-channels` bridge (in-memory conversation map plus a SQL-filtered `live_channel_sessions` fallback), and the turn and idle polls.
- Monitor loops: credential monitor (6 h), session-event listener, idle-confirm debounce.
- A heuristic sweep for `std::fs::*` in `async fn` bodies (the clippy `disallowed-methods` list covers only `read_dir`, `Command::output/status`, `remove_dir_all` and secrets). The remaining hits are small: `canonicalize`, `create_dir_all`, and KB-sized writes. The exception is restore (4-01).
- A heuristic sweep for DB calls inside loops (65 hits outside `otto-state`). Each was triaged. Bounded, user-triggered or deliberately paced loops are not reported: API-client `secure_all`, design link extraction, workflow trigger scheduler (only due triggers), chat trigger candidates, DB-change execution re-auth (500 ms, by design), history index (5 ms paced), `/sessions/bulk` (capped).

## Areas NOT covered yet (next iteration)

- `otto-browser` live tabs and the Lightpanda fetch engine: per-tab memory and DOM annotation storage.
- `otto-product` story watcher fan-out after the iteration-3 Jira/Confluence client split (`otto-issues/atlassian.rs`): HTTP request counts per poll.
- `otto-connections/transfers.rs` and the import path (`conn_import.rs` has three blocking opt-outs).
- `otto-server/src/plugins.rs` sidecar reverse proxy: body buffering vs streaming.
- `vault_docs_agent.rs`: the run loop and its result-file reads (`std::fs::read_to_string` in async at 2881 and 3038, sizes unknown).
- `otto-telemetry-collector` file log rotation.
- Restore preview at scale. It runs the whole `apply_rows` inside `BEGIN IMMEDIATE` and rolls it back. That is inherent to a dry run, but a large archive holds the writer for the whole insert pass. This iteration checked only the file half (4-01).

## Findings

### backend-performance-4-01: state restore holds the SQLite write lock while it decodes, writes, re-reads and fsyncs every archived file synchronously on a runtime worker (minor)

**Where:**
- `crates/otto-server/src/state_archive.rs:693-697`: `begin_with("BEGIN IMMEDIATE")`.
- `:700`: `apply_rows`.
- `:727-737`: base64 decode, then `std::fs::write` of every pending file into the stage dir.
- `:740-747`: `std::fs::read` of each staged file back into memory.
- `:748`: `files::publish`.
- `:767`: `tx.commit()`.
- `crates/otto-server/src/state_archive/files.rs:423-426`: `publish` = `write_all` + `sync_all`.
- `crates/otto-state/src/db.rs:44`: `busy_timeout(5 s)`.

**Evidence:**
- `restore_owned` opens an IMMEDIATE transaction, which takes the database write lock, before it touches any file. It keeps the lock until `tx.commit()` at `:767`.
- Between those two points, all of this runs in plain synchronous code inside the `async fn`, with no `spawn_blocking` and no `offload::blocking`:
  - every pending archive file is base64-decoded and written to a temp stage dir;
  - every staged file is read back into a `Vec`;
  - every file is written again, and fsynced (`sync_all`), by `files::publish`.
- `plan_files` (`:578-…`) also runs synchronously under the lock. It runs one `files::existing` openat chain per archived file.
- The clippy blocking list does not catch these calls: `disallowed-methods` names only `read_dir`, `Command::output/status`, `remove_dir_all` and the secret store.
- Each byte is written twice and read once. The stage-then-read-back round trip adds no safety, because publish copies bytes and does not rename the staged file.

**Cost model:**
- The archive cap is 256 MiB (`MAX_ARCHIVE_BYTES`). Vault notes, design blobs, canvas files, product attachments and transcripts can each run to thousands of files.
- Each `sync_all` is a disk barrier, roughly 1–10 ms on APFS.
- For a few thousand files this is seconds to tens of seconds. During that time:
  - one tokio worker is pinned;
  - every other writer in the daemon waits on the write lock. That includes session status transitions, trail inserts, usage, workgraph and settings.
- Writers that wait longer than `busy_timeout` (5 s) fail with `SQLITE_BUSY`, so a long restore surfaces as unrelated write errors across the app.
- Restore is root-only and rare, hence minor.

**Fix:**
- Stage before taking the lock. In one `spawn_blocking`, before `BEGIN IMMEDIATE`, decode every candidate file and write and fsync it into the stage dir. Do the same for `plan_files`' existence probes.
- Inside the transaction, publish with a no-replace rename from the stage dir into place, done on the blocking pool. On macOS use `renameatx_np(RENAME_EXCL)`; elsewhere use `linkat` + `unlink`. Keep the existing rollback-on-commit-failure. This removes the read-back and the second write and fsync.
- If the rename cannot be done atomically, at least move the publish loop into `spawn_blocking` so it does not run on a runtime worker.

### backend-performance-4-02: vault scans index non-note files (and prune removed notes and files) with one autocommit, one stat and one publication-lock round trip per file; only notes are batched (minor)

**Where:**
- `crates/otto-vault/src/engine.rs:842-888`: changed non-note files, with `state.publication.lock()` at `:843`, `symlink_metadata` at `:851` and `store.upsert_file` at `:877`.
- `:897-955`: removed notes and files, with `publication.lock()` at `:902`, `symlink_metadata` at `:911`, and `remove_note` / `remove_file` at `:940-943`.
- `crates/otto-vault/src/store.rs:672-691` (`upsert_file`) and `:693-…` (`remove_file`): single-statement `.execute(&self.pool)`, i.e. an autocommit each.
- `crates/otto-vault/src/scan.rs:150-155`: every non-hidden, non-`node_modules` file that is not `.md` goes into `files`.

**Evidence:**
- Changed notes go through `index_scan_batch`. That path holds a bounded batch of prepared bodies and commits each batch in one transaction (`engine.rs:802-835`, comment "commits each batch in one transaction (F7)").
- The non-note `changed_files` loop that follows does not batch. For each file it takes the publication mutex, re-stats the file, runs one `INSERT … ON CONFLICT DO UPDATE` as its own transaction, then updates the cache.
- The prune loop has the same shape: lock, stat, delete, one autocommit per row.
- `scan::walk` counts everything except `.md`, dot-entries and `node_modules` as a file. A vault rooted at a code repository (vault-repo-docs, or an Obsidian vault on a project dir) therefore lists every source file, image and build artifact. A Rust `target/` is not skipped.

**Cost model:**
- The first scan of a vault with F non-note files issues F write transactions on the shared SQLite writer. Bulk deletes or renames do the same for removed files.
- Under WAL with `synchronous=NORMAL` each commit is cheap, about 0.1–0.3 ms. But with F = 50k (a repo with a `target/` dir) that is 10–15 s of writer churn interleaved with the rest of the daemon, plus 50k stat calls and 50k mutex round trips.
- The same work batched like notes would take a few dozen transactions.

**Fix:**
- Collect the verified `(rel, size, mtime)` tuples for changed files into a batch (for example 500). Re-check `changed_since` and the stat under one publication-lock hold per batch, then write them with one multi-row upsert, or a loop inside one transaction (`store.upsert_files_batch`).
- Do the same for removals: one `DELETE … WHERE vault_id = ? AND path IN (SELECT value FROM json_each(?))` per batch.
- Optionally skip build-output dirs (`target`, `dist`, `build`) in `is_skipped_dir`, or honour `.gitignore`, for vaults rooted at a repo.

### backend-performance-4-03: spawn naming, relay, broadcast and several cleanups decode a workspace's entire session history to find a few open agent sessions (minor)

**Where:**
- `crates/otto-sessions/src/manager.rs:3346-3370` (`allocate_session_name`): `self.repo.list_by_workspace(ws_id)` at `:3362-3366`, then `.filter(|s| !s.archived && s.kind == Agent)`. It runs on every untitled agent create (`:2918-2923`).
- `crates/otto-sessions/src/manager.rs:2244-2268` (`broadcast_message`, `:2250`) and `:2280-…` (`relay`, `:2281`): both filter to live agent sessions in Rust.
- `crates/otto-automation/src/goal_loop.rs:1291-1303`: `cleanup_executor_sessions_inner` filters on `meta.source == "goal_loop"` and `loop_id` in Rust.
- `crates/otto-server/src/cli_update.rs:513-521`: loops over every workspace and keeps only `is_live` agents.
- `crates/otto-server/src/routes/resource_access.rs:145-158`: `retire_mcp`.
- `crates/otto-state/src/sessions.rs:386-393`: `list_by_workspace` = `SELECT * FROM sessions WHERE workspace_id = ? ORDER BY created_at` with full `row_to_session` (including the `meta_json` parse) and no limit.

**Evidence:**
- Each of these callers needs only the open or live agent rows, or one `meta.source`. Each loads and decodes every session row the workspace ever had: archived rows, review and swarm reviewers, workflow and assistant sessions.
- The repo already has the narrow form. `SessionsRepo::list_filtered` with `SessionListFilter { archived, kind, status, source, … }` applies these filters in SQL and is indexed by `idx_sessions_ws_archived_created` (0158).
- `SessionListFilter`'s own doc records the scale: "a main workspace with 1.9 k hidden review agents used to ship them all".
- The bridge (`otto-channels/bridge.rs:771-786` `live_channel_sessions`) and the list routes were moved to `list_filtered` for this reason. These callers were not.

**Cost model:**
- For a workspace with S historical sessions, each call decodes S rows plus S `meta_json` documents, while holding a pool connection.
- With S ≈ 2–20k that is tens to a few hundred ms per call.
- `allocate_session_name` is on the user's session create path: every "New agent" without a title, before the CLI spawns. `relay` and `broadcast` run once per ⌘K relay message.
- The cleanup paths are rarer, but `cli_update` multiplies the cost by the number of workspaces.

**Fix:**
- `allocate_session_name`: use `list_filtered(&[scope], &SessionListFilter { archived: Some(false), kind: Some("agent"), .. })`. Better, add a `SELECT lower(title) FROM sessions WHERE workspace_id = ? AND archived = 0 AND kind = 'agent'` projection (`open_agent_titles`).
- `relay` / `broadcast_message`: use the same filter plus a status filter, or a status `IN (running, working, idle)` variant. Alternatively start from the in-memory `live` map and fetch those ids with `filter.ids`.
- `cleanup_executor_sessions_inner`: `source: Some("goal_loop")` plus the `loop_id` check in Rust on that small set.
- `cli_update` / `retire_mcp`: iterate the in-memory live handles, or use `list_filtered` with `archived: Some(false)` and `kind: Some("agent")`.

### backend-performance-4-04: the Mission Control projector loads every review comment on each review event and for 50 reviews per workspace per reconcile, and never uses them (nit)

**Where:** `crates/otto-server/src/workgraph_projector.rs:299-303` (live `ReviewChanged` → `upsert_review`), `:880-891` (backfill: the latest 50 reviews per workspace → `upsert_review`), `:634-635` (`ctx.reviews_store.get_review(review_id)`), and `crates/otto-state/src/reviews.rs:339-347,609-617` (`get_review` = the row plus `SELECT * FROM pr_review_comments WHERE review_id = ?`, decoded).

**Evidence:**
- `upsert_review` reads only these review fields: `repo_id`, `status`, `pr_number`, `summary_md`, `verdict`, `blocker_count`.
- `get_review` also fetches and decodes every comment row of the review. `comments` is never read in the projector.
- The live path runs on each `ReviewChanged` (start, session ready, transitions).
- The backfill runs at boot, after any lag, and every 30 min. It covers 50 reviews per workspace, serially, on the projector task. The projector logs "slow event" at 25 ms, and a slow event delays every later event.

**Cost model:** One extra comment query and decode per review event. Per full pass that is up to W × 50 × C comment rows, for W workspaces and C comments per review. Reviews with re-runs keep earlier decided comments, so C reaches the hundreds.

**Fix:** Add `ReviewsRepo::review_head(id)` (the row without comments; `row_to_review(&row, Vec::new())`, or a narrow struct) and use it in `upsert_review`. The backfill could also select the needed columns for its 50 reviews in one query instead of 50 `get_review` calls.

### backend-performance-4-05: design and canvas assist live previews re-read the whole agent file every 900 ms without a stat gate (nit)

**Where:**
- `crates/otto-design-assist/src/lib.rs:1690-1731` (`spawn_live_poll`: `sleep(POLL)` then `read_capped(&path, MAX_CONTENT_BYTES)`), `:1596-1606` (`read_capped`: `metadata` then a full `tokio::fs::read`), `:85` (`POLL = 900 ms`).
- `crates/otto-design/src/format.rs:10` (`MAX_CONTENT_BYTES = 25 MiB`).
- `crates/otto-canvas/src/assist.rs:1017-1032` and `:927-940` (`read_agent_source`: a full read up to `SCENE_BODY_LIMIT` each tick, then a `String` compare against `last`).

**Evidence:**
- For a whole agent turn (minutes), each poll reads the entire file into a fresh `Vec`/`String` and compares it byte for byte with the previous content. It only skips when they match.
- `read_capped` already calls `metadata`, but uses only the length. An unchanged `(len, mtime)` would skip the read.

**Cost model:** About 1.1 reads per second of up to 25 MiB (design) or the scene limit (canvas) per active assist turn, plus the same-size allocation and compare. Small documents make this negligible. Large mockups or diagrams cost tens of MB/s of reads and allocations while the agent is thinking, and no preview is produced.

**Fix:** Keep the `(len, modified)` from the previous poll, skip the read when both are unchanged, and only then read and compare. Both loops can share one helper.

### backend-performance-4-06: History still root-scans, on every page, rows that have no persisted path and whose transcript is gone (nit; carried from 3-04)

**Where:** `crates/otto-server/src/routes/history_page.rs:179-205`, `crates/otto-server/src/routes/transcript.rs:146-184`.

**Evidence:** Unchanged since iteration 3. Only a successful scan is persisted (`Some((r, true))`). A row with a NULL `transcript_path` whose file is gone runs `transcript_path_in_roots` (one `read_dir` of `~/.claude/projects` plus one stat per project dir), or a Codex rollout walk. That happens on every History page, filter change and search. FIXES (iteration 3) lists the `transcript_missing_at` migration as not done.

**Cost model:** Bounded by the number of such rows in the 4×limit candidate window. Each costs one `read_dir` plus 100–300 stats, or a tree walk. The population shrinks as old rows age out.

**Fix:** As proposed in iteration 3. Add an additive `transcript_missing_at` column (append-only migration), set it when `resolve_transcript_sync` returns `TranscriptMissing` after a root scan, skip the scan for rows marked within a TTL, and clear the mark on the next successful capture.
