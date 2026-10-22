# Backend performance review — quality iteration 3 (2026-10-22)

Lens: backend-performance (Rust hot paths: N+1 / queries in loops, missing
indexes, blocking IO on the runtime, unbounded memory, needless
clone/serialization, polling loops, per-WS-message work, ClickHouse/usage cost).
Worktree: `fix/quality-loop-20261021` @ 3dd55d9f2. Read-only review (no builds, no tests).

## Score

Start 10.0. Open findings: 0 blocker, 0 major, 3 minor, 2 nit.

10.0 − 3×0.1 − 2×0.03 = 10.0 − 0.30 − 0.06 = **9.64 / 10**

(Iteration 2 scored 9.77. Two of the five findings are what is left of the
iteration-2 findings. The other three are in areas iteration 2 had not covered:
the review summarizer, the session spawn MCP reconcile, and the state archive.)

## Previous findings (iteration 2)

| id | status | evidence in current code |
|----|--------|--------------------------|
| 2-01 Outputs panel re-registers every artifact | **partly fixed; residual open (nit, see 3-05)** | Registration is fixed. `routes/transcript.rs:801-820` `register_work_artifacts` resolves the item once, reads `artifact_keys` once and inserts only the missing ones. `register_work_artifact` (:780-798) uses `has_artifact`. Still open: `session_artifacts` (:705-711) always calls `cached_fold`. A live session gets no `live_artifacts` accessor (FIXES lists this under "Not done"). |
| 2-02 History root scan per pruned session | **fixed; residual edge (nit, see 3-04)** | `transcript.rs:146-157`: a persisted path whose file is gone returns `TranscriptMissing` without a scan. The legacy route delegates to `history_page::legacy_entries` (`history_page.rs:116-133`). A path found by a scan is persisted (:199-205). Still open: a miss is not recorded for a row with a NULL `transcript_path`. |
| 2-03 fold cache re-serializes to size | **rejected in FIXES (not re-reported)** | `transcript_cache.rs:161-201` is unchanged. FIXES gives the reason: the cheap estimate would stop transcripts over about 7 MB from being cached. No new evidence. |

Iteration-1 findings 1-01 … 1-10 were all verified fixed in iteration 2. Spot
checks this pass found no regressions: `review_status` / `review_agents`
still serve the workflow pollers (`run_engine.rs:689,697`), and the WS event
auth caches are in `ws_events.rs:640-800`.

## Areas covered (this iteration)

- Re-verification of every iteration-2 finding (table above), including the new `artifact_keys`, `has_artifact` and `legacy_entries` code.
- The iteration-2 diff (`ab0a86f05..HEAD`, 166 crate files), checked for new hot-path costs: `otto-state/api_client.rs` single-statement PATCHes and the recursive-CTE cycle check, `otto-context/library.rs` stat-validated skill cache and `write_atomic`, `resource_sessions::check_many_with_pool`, `history_page` legacy delegation and path persistence, personal-agents owner checks.
- `ws_events.rs` per-event path: topic filter, `allowed()` with per-connection role/owner/admin caches and the TTL/cap, bounded send, lag resync. No per-event DB query after warm-up.
- Code-review engine internals (not covered before): `modules/review/{run,summarize,retry,lifecycle}.rs`, `otto-review/src/{summarizer,session}.rs` drive loop and paste/dispatch polls, and `review_is_cancelled`. Finding 3-01.
- Session spawn and resume path (not covered before): `SessionManager::sync_workspace_mcp`, `otto-sessions/src/mcp.rs` reconcile (`cwd_lock`, `atomic_write`, env files, git exclude), and `room_authority`. Finding 3-02.
- Per-session status task (`manager.rs:6580-6680`: DB writes only on transitions), `shutdown_*` sweeps.
- `agent_session.rs` turn watch: 1 s poll on the incremental `poll_claude_tail` and an mtime stat. Already O(new bytes).
- Rooms backend (not covered before): `rooms/socket.rs` run loop and 1 s `maintain` tick (in-memory `room_authority_snapshot`, no DB), recap archive (`recap/http.rs`, `archive.rs`, appends via `spawn_blocking`), and `recap_engines` bounded reads.
- `otto-vault` engine (not covered before): watcher-driven scans, `kick_scan` coalescing, `write_note_opts` (incremental `index_prepared`, no full walk), switcher (precomputed lowercase candidates), and `backlinks_context_window` (bounded hydration, hashed context cache with batch eviction).
- `otto-memory` search and recall: FTS hits ranked without re-fetching, background `bump_access`. `entity_graph_visible` does one `get` per link, but only for one entity's neighbourhood. Checked, not reported.
- `otto-brokers` (not covered before): consume caps (`MAX_CONSUME` 5000, 16 MiB `MAX_CONSUME_BYTES`, 50k filter scan cap), librdkafka calls on `spawn_blocking`, and replay. Replay produces one message at a time, which is deliberate so it stops at the first failed acknowledgement and records the acknowledged prefix. Not reported.
- `otto-k8s/monitor` collector loop: single-timer sleeps, failure backoff, NDJSON batch inserts per cycle.
- `otto-product` story watcher: 60 s scan, bounded concurrent polls (4), per-story cadence with a 5-minute floor.
- `otto-improve` evidence (bounded delta reads with cursors) and `otto-insights` (fs work on `spawn_blocking`, 60 s scheduler).
- `otto-telemetry-collector` worker and sampler loops (opt-in, 1 s ticks are cheap in-memory checks, flush every 5 min).
- `otto-git` `refs_with_base`: the new "landed in develop/main" sets are memoized per base (`merged_sets_cached`).
- `otto-dbviewer` query history indexes (`0021`, `0154`), and `otto-connections/transfers.rs` progress tick.
- `state_archive` export (not covered before). Finding 3-03.

## Areas NOT covered yet (next iteration)

- `ottod/src/mcp_tools.rs` beyond `tools/list` / gateway caching (per-call `resolve_native_refs` round trips).
- `otto-aws` logs and S3 download streaming, plus the `otto-k8s` `list_gateway` / `list_cache`.
- `otto-swarm` board/task routes under many tasks (iteration 2 covered only the runtime).
- `otto-design-assist` / `otto-canvas` assist turns.
- `otto-apiclient` gRPC reflect cache and `pm` script engine (boa) per request.
- `otto-ssh` Kafka-aware proxy and the SFTP throughput path.
- `state_archive` restore (`insert_row` per row) and `backup_git` restore preview.

## Findings

### backend-performance-3-01: review summarizer polls a full review load (row, agents JSON and every comment) every 100 ms for its whole run (minor)

**Where:**
- `crates/otto-server/src/modules/review/summarize.rs:49-54` (`is_cancelled` → `review_is_cancelled`)
- `crates/otto-server/src/modules/review/summarize.rs:152-159` (`cancelled` future: `loop { if is_cancelled().await { break } sleep(100 ms) }`)
- `crates/otto-server/src/modules/review/summarize.rs:270-271` (`for c in parsed { if is_cancelled().await …`)
- `crates/otto-server/src/modules/review/run.rs:654-661` (`review_is_cancelled` → `reviews_store.get_review`)
- `crates/otto-state/src/reviews.rs:303-311` (`get_review` = `SELECT * FROM pr_reviews` + `comments_for_review` = `SELECT * FROM pr_review_comments WHERE review_id = ?`)
- `crates/otto-review/src/summarizer.rs:113-118` (`drive` selects on `cancelled` for the whole `budget`)

**Evidence:**
- `review_is_cancelled` checks the in-memory flag first. When the flag is not tripped (the normal case), it falls through to `get_review`. That loads the review row, decodes `agents_json`, and loads and decodes every comment row.
- `drive` keeps the `cancelled` future running for the whole summarizer turn. That future calls `is_cancelled()` every 100 ms.
- `summarizer_timeout` is `clamp(300 + 3 × findings, 300, 1800)` seconds (`summarize.rs:114-115`). A run is 3,000–18,000 full review loads.
- A one-column helper already exists for this purpose. `ReviewsRepo::review_status` (`reviews.rs:272-281`) has the doc comment "for pollers waiting on completion, which used to load the whole row (agents JSON) plus every comment each tick", and `run_engine.rs:689,697` uses it. `review_is_cancelled` was never switched to it.
- After the turn, the draft loop calls `is_cancelled()` once per parsed comment. The same loop inserts comments (`add_comment`), so each check reloads a growing comment list. That is O(N²) comment-row decodes for N parsed findings.
- The cancel endpoint always trips the in-memory flag first (`lifecycle.rs:96` `review_cancels.signal`). The DB read is only a fallback for flagless paths (cancel before the attempt registered, restart), so a status-only read is enough.

**Cost model:** Each running summarizer issues 20 queries/s (two statements per check) on the shared 8-connection SQLite pool for 5–30 minutes. Each check decodes `agents_json` plus every comment row. A summarizer retry or re-run keeps the earlier decided comments (`kept_comments`), so a review with 100 comments decodes about 1,000 comment rows per second. Workflows that run several reviews in parallel multiply this.

**Fix:**
- In `review_is_cancelled`, replace `get_review(..).is_ok_and(|r| r.status == Cancelled)` with `review_status(review_id).await.is_ok_and(|s| s == ReviewStatus::Cancelled)`.
- Make the summarizer's `cancelled` future event-driven. Wait on `ReviewChanged` for this id (as the workflow `code_review` node does since 1-05) or on the cancel flag, with a slow safety recheck (for example 5 s) through `review_status`, instead of a 100 ms DB poll.
- In the draft loop, check the in-memory flag per comment and read the durable status once per batch, not once per parsed comment.

### backend-performance-3-02: every non-browser agent spawn or resume runs one session-row SELECT per live session to compute the browser union (minor)

**Where:**
- `crates/otto-sessions/src/manager.rs:2746-2771` (inside `sync_workspace_mcp`, called from create at `:3195` and resume at `:5972`)
- `crates/otto-sessions/src/mcp.rs:123-131`, `:228-…`, `:905-931` (`cwd_lock` std mutex, `reconcile_json_launcher`, `atomic_write`: synchronous file IO in the same async call)

**Evidence:**
- When the session being spawned is not a browser session (almost every spawn), the code snapshots all live ids (`self.live.iter()`). For each other live session it then:
  - awaits `self.repo.get(&other_id)` (a full `SELECT * FROM sessions` row decode, including `meta_json`);
  - calls `std::fs::canonicalize` on that session's cwd, on the tokio worker;
  - stops early only if it finds a browser session in the same cwd.
- Browser sessions are rare and `live` includes shells and connections, so the loop normally walks every live session.
- Every other field the loop needs is already in memory (`live` handles) or could come from one query. Only `meta.browser` and `cwd` are read.
- After the loop, the same async function runs `reconcile_managed_servers` (and `_agy` / grok / codex env files). That is a synchronous read-modify-write of `.mcp.json` under a `std::sync::Mutex` per cwd, on the runtime thread. Concurrent spawns into one cwd (review reviewers, swarm workers, workflow fan-out) therefore park tokio workers on that std mutex while the other spawn does file IO.

**Cost model:** A spawn or resume with L live sessions costs L sequential PK queries plus L `canonicalize` calls. Burst spawns (a 6-reviewer review, a swarm wave, a workflow `parallel` node) cost B × L. With L = 60 and B = 10 that is 600 sequential queries and 600 path canonicalizations in the spawn critical path, all before the CLI starts.

**Fix:**
- Replace the loop with one query: `SELECT cwd FROM sessions WHERE id IN (<live ids>) AND json_extract(meta_json,'$.browser') = 1`, or better `… WHERE status != 'exited' AND archived = 0 AND json_extract(meta_json,'$.browser') = 1`. Canonicalize only the (usually zero) matches.
- Or keep an in-memory `cwd → browser-session count` map, updated where sessions with `meta.browser` are spawned and retired.
- Run the reconcile body (`reconcile_*`, `write_env_file`, `canonicalize`) inside one `spawn_blocking`, so a contended `cwd_lock` blocks a blocking-pool thread, not a runtime worker.

### backend-performance-3-03: state-archive export sorts every table by re-serializing rows inside the comparator, on the async runtime (minor)

**Where:**
- `crates/otto-server/src/state_archive/schema.rs:621` (`output.sort_by_key(|row| serde_json::to_string(row).unwrap_or_default())`)
- `crates/otto-server/src/state_archive/schema.rs:613-616` (per-row `serde_json::to_vec` only to count size)
- `crates/otto-server/src/state_archive.rs:172-174` (the whole table serialized again to count size)
- Callers: `routes/backup.rs:498` (export) and `routes/backup_git.rs:681` (`build`, used by git preview, export, commit and sync)

**Evidence:**
- `slice::sort_by_key` calls the key function on every comparison, not once per element. Each comparison therefore serializes two rows to JSON strings: about 2·n·log₂n serializations per table.
- `sort_by_cached_key` (one key per element) or an SQL `ORDER BY` would give the same determinism.
- All of this is synchronous CPU inside `async fn read_rows` / `build_snapshot_parts`. Nothing is offloaded with `spawn_blocking`, and it runs while a read transaction stays open across all tables.
- Each row is serialized three more times besides the sort: once for the per-row size check, once for the per-table size check, and once for the final archive.

**Cost model:** The archive cap is 256 MiB (`MAX_ARCHIVE_BYTES`). For a 200k-row table (activity, trail, query history), the sort alone is about 2 × 200k × 18 ≈ 7M row serializations. That ties up a tokio worker for seconds to tens of seconds on each git-backup preview, export, commit or sync. The path is user-initiated and root-only, so this is minor, but a stalled worker delays unrelated requests and WS frames on that thread.

**Fix:**
- Sort in SQL instead: `ORDER BY` the table's primary key, or `rowid` for rowid tables. Drop the in-memory sort.
- If byte-order sorting is required, use `output.sort_by_cached_key(...)`. Even better, keep the `Vec<u8>` produced for the size check and sort `(bytes, row)` pairs, so each row is serialized once.
- Run the table walk, size accounting and archive assembly inside `spawn_blocking` (or a dedicated thread), not on the runtime.

### backend-performance-3-04: History still root-scans, on every page, sessions that have no persisted path and whose transcript is gone (nit)

**Where:** `crates/otto-server/src/routes/history_page.rs:178-210`, `crates/otto-server/src/routes/transcript.rs:146-184`, `crates/otto-sessions/src/lifecycle.rs:159-176`

**Evidence:**
- The 2-02 fix stops the scan when a persisted path exists, and it persists a path a scan finds (`history_page.rs:201` only matches `Some((r, true))`).
- A miss is not recorded. A candidate with a NULL `sessions.transcript_path` takes this path on every History page, filter change and search keystroke:
  - a session created before migration `0121_sessions_transcript_path.sql`;
  - or a session whose capture scan never resolved a path.
- For such a Claude candidate with a pruned file, `resolve_transcript_sync` calls `transcript_path_in_roots`, which runs `read_dir(~/.claude/projects)` and stats one candidate per project dir. A Codex candidate gets a full `codex_rollout_path_under` walk.
- FIXES says "A row with no persisted path still scans once". That holds only when the scan finds the file.

**Cost model:** This is bounded by how many such rows fall in the 4×limit candidate window: each costs one `read_dir` plus about 100–300 stats (Claude) or a full tree walk (Codex). The population shrinks over time, because new sessions get a path when they are captured.

**Fix:** Record the miss. For example, write an additive `transcript_missing_at` column, or a sentinel in `transcript_path`, when `resolve_transcript_sync` returns `TranscriptMissing` after a root scan. Skip the scan for rows marked within a TTL, and clear the mark on the next successful capture.

### backend-performance-3-05: Outputs panel on a live session still runs a whole-file fold that is never cached (nit; residual of 2-01)

**Where:** `crates/otto-server/src/routes/transcript.rs:705-711` (`session_artifacts` → `cached_fold`), `crates/otto-server/src/transcript_cache.rs:340-352` (a fold of a file that grew meanwhile is returned but not stored as current), `crates/otto-server/src/transcript_tail.rs:281-336` (`live_page_settled` / `live_tool` exist, there is no `live_artifacts`)

**Evidence:**
- `get_transcript` and `transcript_tool` serve a live session from the tail's fold. `list_artifacts` does not: it always folds the whole file on the shared 2-permit `fold_workers()` pool.
- For an active session the file grows between requests, so the cache stamp never matches. Each such open re-folds from byte 0 next to the tail's own fold, and the result is not cached.
- The UI now loads artifacts once per session and page lifetime, plus once per WS-gap resync (`ui/src/lib/stores/activity.svelte.ts:213-214` `artifactsLoaded`). Live deltas arrive over WS. That keeps the cost to one fold per (session, load/resync), hence nit.

**Cost model:** One extra full fold (up to the 32 MiB entry cap, hundreds of ms of CPU) per live-session Outputs open or resync, competing for the 2 fold permits with other transcript opens.

**Fix:** Add `transcript_tail::live_artifacts(session_id, provider, path)` that clones `st.folder`'s artifact list under the tail lock (like `live_tool`). In `session_artifacts`, try it first and fall back to `cached_fold` only when no tail is running.
