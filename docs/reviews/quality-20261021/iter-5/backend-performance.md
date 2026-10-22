# Backend performance review — quality iteration 5 (2026-10-22)

Lens: backend-performance. This covers Rust hot paths: N+1 queries and queries in loops, missing indexes, blocking IO on the runtime, unbounded memory, needless clones and serialization, polling loops, per-WS-message work, and ClickHouse/usage cost.
Worktree: `fix/quality-iter5-20261022` @ 8177c4d85. This was a read-only review: no builds and no tests were run.

## Score

Start 10.0. Open findings: 0 blocker, 0 major, 2 minor, 5 nit.

10.0 − 2×0.1 − 5×0.03 = 10.0 − 0.20 − 0.15 = **9.65 / 10**

Iteration 4 scored 9.61. All six iteration-4 findings are fixed in the current code (see the table). Every finding below is new. They come from code that landed in iteration 4 (`session_files.rs` and its LSP hook), from areas that iteration 4 listed as not covered (the product story watcher, the plugin proxy, `transfers`/`conn_import`, `vault_docs_agent`, the telemetry log), and from one level below earlier passes (the History path gate and the admin session overview).

## Previous findings (iteration 4)

| id | status | evidence in current code |
|----|--------|--------------------------|
| 4-01 restore holds the write lock while it decodes, writes, re-reads and fsyncs files on a runtime worker | **fixed** | `state_archive.rs:692-701`: `stage_files` decodes, writes and fsyncs every file on `offload::blocking` before `BEGIN IMMEDIATE` (`:707`). `plan_files` runs on the blocking pool (`:718`). `publish_staged` (`:742-748`, `:825-861`) publishes with a `renameat_with(..., NOREPLACE)` per file (`:901`). The read-back and second write happen only in the no-rename fallback (`:842-844`). |
| 4-02 vault scan writes non-note files and prunes one autocommit at a time | **fixed** | `otto-vault/src/engine.rs:34-36` sets `SCAN_BATCH_FILES = 500`. Changed files are verified under one publication-lock hold per batch (`:845-851`) and written with `store.upsert_files_batch` (`:891`). Removals use `remove_notes_batch` / `remove_files_batch` (`:973-974`). `store.rs:696,725` define the batch writers. |
| 4-03 spawn naming, relay, broadcast and cleanups decode the whole session history | **fixed** | `manager.rs:3206-3224`: `allocate_session_name` reads `open_agent_titles` (`sessions.rs:395-…`, a `lower(title)` projection). `broadcast_message` (`manager.rs:2180`) and `cli_update.rs:516` use `live_agent_sessions` (`manager.rs:3262-3278`: `list_filtered` by live ids). `goal_loop.rs:1304` uses `list_filtered`. `retire_mcp` still loads every row, but it needs every historical cwd to retire the config there, so it is not re-reported. |
| 4-04 the projector loads every review comment and never uses them | **fixed** | `workgraph_projector.rs:636` calls `review_head(review_id)`. The backfill makes one `recent_heads_in_workspace(ws_id, 50)` query and uses no comments (`:883-890`). |
| 4-05 design and canvas assist previews re-read the whole file every 900 ms | **fixed** | `otto-agent-run/src/live_file.rs:26-…` adds `StatGate`. A read happens only when `(len, mtime)` changes. It is used by `otto-design-assist/src/lib.rs:1687` and `otto-canvas/src/assist.rs:756`. |
| 4-06 History root-scans rows that have no path and no transcript | **fixed** | Migration `0191_session_transcript_missing.sql` exists. `transcript/resolve.rs:192-230`: `fresh_missing_marks` and `mark_missing` (PK `IN`, chunks of 400, TTL 1 h), and `record_found` clears the mark (`:233-241`). |

## Areas covered (this iteration)

- Re-verification of every iteration-4 finding (table above).
- The iteration-4 fix diff (`6cff08270..HEAD`, 191 crate files), checked for new hot-path costs:
  - MCP profiles. On the stdio bridge, the profile filter rides the cached enable-list read (`ottod/src/mcp_tools.rs:551-600`, `ENABLED_CACHE_TTL`). The daemon resolves it with one `McpProfileRepo::resolve` query (`otto-mcp/src/profiles/mod.rs:483-505`). `ProfiledTools` filters in memory. Migration 0194 has the default and name indexes.
  - `outward/exec.rs` issue-route split: a pure refactor.
  - `budget/mod.rs`: code moved, same cost.
  - Swarm `identity_changes` and `find_agent_session` (`list_live_by_meta`).
  - Broker replay checkpoints: every 100 acks, bounded by `MAX_CONSUME`.
  - Connection delete cascade (one transaction).
  - `transcript/resolve.rs` missing marks.
  - `submit.rs`, the unified paste → Enter path: 250 ms polls over in-memory PTY state, bounded by deadlines.
  - The orphan session-folder sweep: daily, one PK `get` per folder.
  - `mcp_servers` keychain reads on spawn: on the blocking pool via `mcp_sync.rs:38`.
  - Jira create pre-checks: user-triggered and page-capped.
  - The `otto-issues` Markdown↔ADF/storage converters.
- New `session_files.rs` routes and the `/ws/lsp` work-area hook. Findings 5-01 and 5-05.
- History transcript, artifacts and import routes for non-admin callers (`history_path_gate`). Finding 5-02.
- `otto-product` story watcher. This was listed as not covered. Per-poll Atlassian calls are 1 source fetch plus the comment listing (cursor-bounded, `list_comments_since`). The supervisor admits at most 4 polls at once and has a 5 min cadence floor. Finding 5-03.
- Admin session overview (`GET /admin/sessions`). Finding 5-04.
- `POST /workspaces/{wid}/transcript/touch`. Finding 5-06.
- `otto-server/src/plugins.rs` sidecar proxy. This was listed as not covered. Request bodies are buffered up to 16 MiB. Responses are buffered with a `Content-Length` pre-check and a running cap of 64 MiB (`:495`, `:535-589`). This is bounded, so it is not reported.
- `otto-connections/transfers.rs` and `conn_import.rs`. These were listed as not covered. The job map is capped at 128 with a 30 min TTL, and progress is a 1 s tick. `conn_import`'s sync scans run only under `spawn_blocking` (`http.rs`).
- `vault_docs_agent.rs` result-file reads. These were listed as not covered. Each turn reads a KB-sized agent JSON once, and the persister coalesces writes through a channel (`:494-512`). Not reported.
- `otto-telemetry-collector` log rotation and the Lightpanda sidecar log. These were listed as not covered. Finding 5-07.
- `otto-browser` Lightpanda supervisor and live screencast. Frames are window-limited with per-viewer ack backpressure (`live/session.rs:673-760`). Skimmed only.
- Game rooms (`game_rooms/socket.rs`, `registry.rs`). The member cap is 16, messages are 20 KiB, and serialization is per member. This is the same verdict as iteration 2: small rooms, not reported.
- DB Explorer Postgres and Mongo drivers: `fetch` streams with `max_rows+1` (`postgres.rs:879-890`), and Mongo uses `bound_pipeline` / `collect_docs` caps.
- A targeted sweep for whole-table `FROM sessions` reads (`grep`) and for `list_by_workspace` / `list_all` callers. Each was triaged. The remaining unfiltered reads are narrow projections (`list_usage_labels`, `list_usage_attribution`, `provider_session_ids` on the backed-off capture scan) or are reported below.

## Areas NOT covered yet (next iteration)

- Workflow engine `update_run_progress`: each node transition rewrites the whole `nodes_json`. It is serialized on the blocking pool, but not re-measured for 200+ node runs with large outputs.
- `otto-browser/src/live/proxy.rs` and `cdp.rs`: per-message CDP proxy work and the paused-request store.
- `otto-dbviewer` Redis driver, `multirun`, and the `complete/` autocomplete schema caches.
- `otto-core::text::balanced_object_spans`: it restarts after an unclosed `{`, so worst-case input is O(n²). It is used by `extract_json_where` on agent output. Not shown to be reachable with large inputs.
- `otto-workgraph` service write and broadcast amplification, and the `otto-skills` library load.
- The `apps/desktop/src-tauri` workspace (a separate Cargo workspace, never reviewed under this lens).

## Findings

### backend-performance-5-01: a non-root `/ws/lsp` open in an agent work area scans every session row in every workspace and canonicalizes each cwd (minor)

**Where:**
- `crates/otto-server/src/routes/session_files.rs:311-366` (`lsp_root_in_own_session`). The query `SELECT id, cwd FROM sessions WHERE cwd <> ''` is at `:329-339`, and the `canonicalize()` of every row on the blocking pool is at `:341-353`.
- Called on every non-root language-server socket open, at `crates/otto-server/src/lsp/mod.rs:309-315`.

**Evidence:**
- After the cheap `in_work_area` test (`:318-326`), the function loads `(id, cwd)` for every session ever created, from every workspace and every user, archived included. It has no workspace, owner or prefix predicate, and `cwd` has no index.
- It then calls `realpath` on every one of those cwds to find the ones under the root.
- Only afterwards does it check ownership, with one `manager.get` plus a role check per match (`:354-364`).
- The only rows that can pass are sessions the caller owns, or sessions in a workspace the caller administers, whose cwd lies under the data dir.
- FIXES (iteration 4, P01 "Not done") names this as a follow-up. It has not been done.

**Cost model:**
- For S total sessions (2–20k on a long-lived install, including 1.9k hidden review agents in one workspace per `SessionListFilter`'s doc), each open costs S rows and S `realpath` syscalls. Missing worktrees fail slowly through the full path walk.
- That is about 0.1–0.5 s of blocking-pool time and a pool connection per open.
- It is paid per LSP socket open for a workflow, persona, goal-loop or swarm worktree. Every editor (re)connect in such a session pays it, and every language opens its own socket.

**Fix:**
- Filter in SQL before canonicalizing:
  - `created_by = ?` (indexed by `idx_sessions_created_by`), OR `workspace_id IN (<workspaces where the caller is Admin>)`;
  - plus `archived = 0` (an archived session's editor is not open);
  - plus a prefix test on both the raw and the canonical data dir (`substr(cwd,1,length(?1)) = ?1`, as `unarchived_cwds_under` does).
- Canonicalize only the survivors.
- Optionally cache `(user, root) → bool` for a few seconds, since a reconnect storm opens one socket per language.

### backend-performance-5-02: every non-admin History transcript page decodes the caller's whole session history and resolves (often root-scans) each session to authorize one path (minor)

**Where:**
- `crates/otto-server/src/routes/transcript.rs:80-117` (`history_path_gate`). It calls `list_by_workspace_for_user` (`SELECT *` plus the `meta_json` parse, no limit, `crates/otto-state/src/sessions.rs:415-425`) and `transcript_paths_for_workspace` (every session of the workspace, `:649-664`).
- It then runs `resolve_transcript_sync` per owned session until one matches. That function does a root scan through `transcript_path_in_roots` for rows with no persisted path (`crates/otto-server/src/transcript/resolve.rs:110-143`).
- Callers: `history_transcript` (`:1050`, every page and scroll-back), `history_artifacts` (`:1083`), `history_transcript_image` (`:1111`) and `history_import` (`:1129`).

**Evidence:**
- Admins and root return early. Every other caller pays the full gate on each request.
- The scan does not consult the iteration-4 `transcript_missing_at` marks (`fresh_missing_marks`). An owned session whose transcript is gone therefore pays a `read_dir` of `~/.claude/projects` plus one stat per project dir (or a Codex rollout walk), on every page.
- It also canonicalizes each resolved path.
- The answer it computes is one indexed lookup:
  - the transcript filename already yields the provider session id (`store_key` → `session_id_from_name`, `:140-142`);
  - `sessions.transcript_path` and `sessions.provider_session_id` are both indexed (`0129_history_page.sql:4-5`).

**Cost model:**
- For a non-admin member with N own sessions in the workspace, each History page costs:
  - N full row decodes;
  - plus all W workspace `(id, transcript_path)` rows;
  - plus up to N resolutions, each up to one directory scan.
- With N = 500 and a few dozen expired transcripts, that is hundreds of ms of blocking-pool work per page. The History reader pages backwards through `?before=`, so a long transcript pays it once per page, and the Outputs panel pays it again.

**Fix:**
- Authorize with one query:
  `SELECT 1 FROM sessions WHERE workspace_id = ? AND created_by = ? AND (transcript_path = ? OR provider_session_id = ?) LIMIT 1`,
  binding the canonical path and the id from `session_id_from_name`.
- Fall back to the current scan only on a miss, and in that fallback skip rows with a fresh missing mark and rows whose persisted path is set but different.
- On a fallback hit, persist the path (`record_found`), so the next page takes the indexed route.

### backend-performance-5-03: each story-watcher poll builds and discards a full story detail (every analysis, question and note row) and re-reads the story twice (nit)

**Where:**
- `crates/otto-product/src/watcher.rs:250` calls `product.refresh_story(...)` and keeps only the error.
- `crates/otto-product/src/service.rs:338-384` (`refresh_story`) ends with `self.story_detail(story_id).await`.
- `:209-238` (`story_detail`) runs eight queries, including `list_analyses` (`SELECT * FROM product_analyses WHERE story_id = ?`, full report rows, `crates/otto-state/src/product.rs:1081-1090`), `list_questions` and `list_notes`.
- `:394-401` (`list_new_comments`) runs `get_story` and `get_account` again right after.

**Evidence:**
- The watcher calls `refresh_story` only for its side effect, recording a new source version when the body changed. The `ProductStoryDetail` it returns, which carries every analysis body, is dropped.
- `list_new_comments` then repeats the story and account reads that `refresh_story` just made.

**Cost model:** Per watched story per cadence (5 min floor), about 10 extra queries. One of them decodes every analysis report of the story, and those grow with each re-analysis. With 50 watched stories that is about 100 wasted queries a minute.

**Fix:** Split `refresh_story` into `refresh_source(&story, &account) -> bool`, which the watcher calls with the story it already holds, and a route wrapper that adds `story_detail`. Pass the loaded story and account into `list_new_comments`.

### backend-performance-5-04: the admin session overview returns every session ever created, fully decoded and unpaginated (nit)

**Where:**
- `crates/otto-server/src/routes/admin_sessions.rs:91-130`: `ctx.sessions_repo().list_all()`.
- `crates/otto-state/src/sessions.rs:495-501`: `SELECT * FROM sessions ORDER BY created_at DESC`, with a `row_to_session` meta parse per row.
- The UI loads it on mount and after every terminate or evict (`ui/src/modules/settings/AdminSessions.svelte:54-66`).

**Evidence:**
- The page is "the daemon-wide active-sessions overview", but the query has no `archived`, status, limit or projection. Archived history, review reviewers and swarm workers are all included.
- The response row needs 9 columns and two in-memory live lookups. `meta_json` is parsed and thrown away.

**Cost model:** O(all sessions) of decode, then a JSON response of all rows, then a DOM list. That is 20k rows and several MB on an old install, paid on every open and after every admin action. The page is admin-only and rarely opened.

**Fix:**
- Project the 9 needed columns with no `meta_json`.
- Default to `archived = 0` (or live plus the last N exited) with keyset pagination (`created_at, id`).
- Keep an "include archived" toggle for the full listing.

### backend-performance-5-05: the new session-files routes skip the `/fs/*` admission and deadline guard (nit)

**Where:**
- `crates/otto-server/src/routes/session_files.rs:164-170` (plain `spawn_blocking`), used by `browse` (`:175-189`) and `read` (`:233-252`).
- Compare `crates/otto-server/src/routes/fs.rs:158-190` (`filesystem_work`: `try_acquire` on a 4-permit semaphore, a 10 s `timeout` and a cancel flag) and its uses in `browse` and `read_file` (`:404-428`).

**Evidence:**
- `/fs/browse` and `/fs/read` bound host-filesystem work to 4 in-flight blocking tasks. A slow or networked path then returns 409 "busy" or 504 "timed out" instead of piling up blocking threads.
- The session-scoped twins do the same `read_dir`, a `canonicalize` plus a `metadata` per entry, and up to 400 KiB reads. They have no admission, no deadline and no cancel check.
- The UI's `sessionFs` retries every 403'd `/fs/*` call on these routes, so for non-root users in work areas they are the main path.

**Cost model:** Normally negligible. A cwd on a stalled mount, or a huge directory (`node_modules`, a `target/` tree), lets concurrent tree expansions occupy unbounded blocking-pool threads, which the rest of the daemon shares, with no timeout.

**Fix:** Route both through `crate::routes::fs::filesystem_work`. Make it `pub(crate)`, with its own semaphore or the shared one, the same 10 s deadline and `browse_canceled` checks in the per-entry loop.

### backend-performance-5-06: `POST /workspaces/{wid}/transcript/touch` decodes the workspace's whole session history to find its live agents (nit)

**Where:** `crates/otto-server/src/routes/transcript.rs:452-480`: `ctx.manager.list_by_workspace(&wid)` (`SELECT *` plus the meta parse, `crates/otto-state/src/sessions.rs:386-393`), then `kind != Agent || !is_live` filtering in Rust.

**Evidence:**
- This is the same pattern iteration 4 fixed elsewhere (4-03). `SessionManager::live_agent_sessions(ws)` (`crates/otto-sessions/src/manager.rs:3262-3278`, live ids through `list_filtered`) is the narrow form.
- The doc comment says clients ping this "every minute … on switching … and on regaining focus".
- The current UI does not call it (`grep` finds only the per-session `/sessions/{id}/transcript/touch` in `ui/src/lib/stores/transcript.svelte.ts:394`; the workspace route is used by `ui/e2e/desktop-conversation.spec.ts:412`). Impact is limited to API and e2e callers, hence nit.

**Cost model:** O(workspace history) of row and meta decode per call, to touch at most `MAX_TAILS` live sessions.

**Fix:** Use `ctx.manager.live_agent_sessions(&wid)`. Either delete the route, or fix the doc comment if the per-minute ping is intended.

### backend-performance-5-07: sidecar logs are rotated only when the process (re)spawns, so a long-running daemon grows them without bound (nit)

**Where:**
- `crates/otto-telemetry-collector/src/collector.rs:21-23` (`LOG_LIMIT = 1 MiB`), `:199-207` (`.stderr(log)`) and `:242-260` (`open_log`: the size check only at open). The doc says "at most two bounded files ever exist".
- `crates/otto-browser/src/lightpanda.rs:155-178` (`LOG_CAP_BYTES = 4 MiB`, also checked only in `open_log`), with stdout and stderr appended at `:180-201`.

**Evidence:**
- Both sidecars inherit an append-mode file as stdout/stderr. The cap is enforced only on the next spawn.
- The collector and the Lightpanda server normally live as long as the daemon (days or weeks).
- Anything they log in that time accumulates in one file: collector export failures and rejected batches at `warn`, and Lightpanda's per-connection and per-request output.

**Cost model:** Disk growth proportional to the sidecar's log rate times uptime. It is bounded only by a crash or restart. It is small unless an exporter fails in a loop, which is exactly when it logs on every batch.

**Fix:**
- Pipe the child's stderr through the daemon (`Stdio::piped()`) into a size-capped rotating writer that checks the length on each write.
- Or run a cheap periodic check (for example every 10 min, on the existing supervisor or sampler tick) that truncates or rotates the file once it is over the cap. `copy`, then truncate in place, works because the child holds an `O_APPEND` fd.
