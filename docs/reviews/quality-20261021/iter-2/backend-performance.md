# Backend performance review — quality iteration 2 (2026-10-22)

Lens: backend-performance (Rust hot paths: N+1 / queries in loops, missing
indexes, blocking IO on the runtime, unbounded memory, needless
clone/serialization, polling loops, per-WS-message work, ClickHouse/usage cost).
Worktree: `fix/quality-loop-20261021` @ ab0a86f05. Read-only review (no builds, no tests).

## Score

Start 10.0. Open findings: 0 blocker, 0 major, 2 minor, 1 nit.

10.0 − 2×0.1 − 1×0.03 = 10.0 − 0.2 − 0.03 = **9.77 / 10**

## Previous findings (iteration 1)

| id | status | evidence in current code |
|----|--------|--------------------------|
| 1-01 native usage state unbounded | fixed | `usage_tailer/native.rs:15` `NATIVE_TOTALS_CAP = 100_000` + FIFO `enforce_cap_at` (:78-107); `forget_grok_files` (:67) on cursor eviction; `forget_vanished_agy` (:391) |
| 1-02 canvas palette search overflow chain | fixed | migration `0188_palette_search_covering_indexes.sql` (canvas + api_requests covering indexes) |
| 1-03 native trees scanned every pass | fixed | `usage_tailer.rs:495-528` — `scan_native_parts(grok_dirty, agy_dirty)` on event passes, full `scan_native` only on reconcile (:483) |
| 1-04 agy DB re-read in full | fixed | `native/agy.rs:98-140` reads `WHERE idx > ?` past a persisted `Mark`; `agy_files` now serialized (native.rs:28-33) |
| 1-05 workflow code_review polls full review | fixed | `workflow_engine.rs:5681-5751` wakes on `ReviewChanged`, reads `review_agents` |
| 1-06 wait_session 500 ms row poll | fixed | `modules.rs:6206-6250` subscribe-before-read, 5 s safety recheck |
| 1-07 broadcast/relay history scan + 3× auth | fixed | `modules.rs:45-87,152-158` `InputAuthority` loaded once, `list_filtered` |
| 1-08 projects page row-by-row auth | fixed | `routes/projects.rs:111` `resource_sessions::check_many` |
| 1-09 refs_with_base extra spawns | fixed | `otto-git/src/local.rs:2334-2360` one `for-each-ref --merged`, base oid from listings |
| 1-10 human_approval 2 s poll | fixed | `workflow_engine.rs:206-210` event wake + `APPROVAL_SAFETY_RECHECK = 30 s` |

All 10 previous findings are fixed. None re-reported.

## Areas covered (this iteration)

- Re-verification of every iteration-1 finding (table above).
- Request chokepoint: `auth_middleware` → `AuthRepo::authenticate` (cache, touch throttle; uncached agent/managed tokens are a deliberate security trade-off) and `feature_guard`.
- Session listing (`otto-sessions/src/http.rs` `list_sessions` / `list_all_sessions` / `visible_out`) — already one SQL query + batched resource check.
- `SessionManager::busy_for_restart` (bounded tail read off the runtime).
- Session WS handler re-auth loop (`ws.rs:1270-1310`, shared per session) and attach path.
- Transcript stack end to end: `routes/transcript.rs` (`get_transcript`, `transcript_tool`, `list_artifacts`, `register_work_artifact`, legacy `history`), `transcript_cache.rs` (admission, eviction, janitor), `transcript_tail.rs` (700 ms poll, incremental `step`, artifact registration).
- History: `routes/history_page.rs` + `otto-state/src/history_page.rs` (bounded candidates, batched `indexed_paths`), `resolve_transcript_sync`, `transcript_path_in_roots`, `codex_rollout_path_under`.
- Work-graph projector live loop (`workgraph_projector.rs`): trail ingest, swarm/review/workflow upserts, `add_artifact_if_absent`; `swarm_runs(project_id, cost_usd)` index present.
- Usage ClickHouse: schema (partitioning, TTL, `usage_accounted` FINAL view), per-session totals queries and their callers (session pane throttled 60 s, swarm per-turn, item detail). Checked but not reported: ORDER BY keys ahead of `session_id` are low-cardinality, so generic exclusion search already prunes granules.
- Scheduled-task supervisor and workflow hand-off poll (status-only, 2 s), swarm runtime (`while_run_active` 250 ms PK poll during readiness only, wake bells, tick locks), personal-agents scheduler (60 s, single list query), assistant thread turn loop.
- `otto-channels` mirror (event-driven status loop, adaptive transcript tail cadence).
- `monitor.rs` (usage recorder provider cache with eviction, AuthScanner bounded tails, session event listener).
- `otto-browser/src/live/session.rs` screencast frame path (flow control, single Arc frame per viewer).
- Game-room socket relay (`game_rooms/socket.rs`) — per-member serialization on small rooms only.
- Database Explorer drivers row caps (ClickHouse `row_cap` / CompactStream, MySQL streaming `max_rows+1`).
- `otto-design` store list/paging (keyset cursor, IN-batched lookups).
- Notifications list (LIMITed range reads), activity ingest hook path (1 PK read + 1 insert per hook).
- Global `OnceLock<Mutex<HashMap>>` registries scanned for unbounded growth (all prune or are keyed by bounded sets; `swarm/runtime/merge.rs` branch locks never pruned but are tiny — not reported).
- `otto-git` PR list / PR diff (single-flight + memo), repo directory listing.

## Areas NOT covered yet (next iteration)

- `otto-vault` engine reindex/watch (has its own `performance_tests.rs`; not re-audited).
- `otto-brokers` replay / consume memory bounds.
- `otto-k8s` monitor/list gateway/list cache, `otto-aws` logs + S3 download.
- `otto-insights` and `otto-improve` engines.
- `otto-memory` recall/governance per agent call.
- `otto-sessions/src/mcp.rs` (agy MCP servers) and `ottod/src/mcp_tools.rs` beyond `tools/list` caching.
- Rooms media/recap backend (`rooms/socket.rs`, `terminal.rs`, `recap_engines`).
- `otto-product` runners and story watcher; `otto-review` engine internals.
- `state_archive` export/import (cold path; reads whole tables into memory).

## Findings

### backend-performance-2-01: opening a session's Outputs panel re-registers every artifact with 2 queries each and re-reads the whole artifact list per artifact (minor)

**Where:**
- `crates/otto-server/src/routes/transcript.rs:709-726` (`list_artifacts` spawns a loop over every artifact)
- `crates/otto-server/src/routes/transcript.rs:730-767` (`register_work_artifact`)
- `crates/otto-state/src/workgraph.rs:1069-1079` (`artifacts_for`: `SELECT … payload_json … WHERE work_item_id = ?`)
- `crates/otto-server/src/routes/transcript.rs:700-706` (`session_artifacts` → `cached_fold`)
- UI trigger: `ui/src/lib/stores/activity.svelte.ts:104-107` (`watchArtifacts` → `loadArtifacts`) and `:130-132` (every reconnect `resync`)

**Evidence:**
- `list_artifacts` returns the list, then `tokio::spawn`s `for a in &snapshot { register_work_artifact(&cx, &s, a).await; }` on **every** call.
- For each artifact, `register_work_artifact` runs `find_by_source(workspace, Session, session.id)` — the same row every iteration — then `artifacts_for(&item.id)`, which loads and decodes every `work_artifacts` row of that item (including `payload_json`), only to check `existing.iter().any(|e| e.reference == reference)`.
- Nearly all of these artifacts were already registered by the live tail (`transcript_tail.rs:794-805` calls the same function as each artifact appears), so the work is repeated for nothing on every panel open and every WS-gap `resync`.
- A one-row existence probe already exists: `WorkGraphRepo::has_artifact(work_item_id, kind, ref)` (`workgraph.rs:1049-1066`), used by the projector's `add_artifact_if_absent` (`workgraph_projector.rs:198-208`).
- Unlike `get_transcript` (`live_page_settled`) and `transcript_tool` (`live_tool`), `session_artifacts` always goes through `cached_fold`. For a live session whose file keeps growing, the fold is never cached as current (`transcript_cache.rs:340-346`), so each open runs a second whole-file fold on the shared 2-permit fold pool next to the tail's own fold.

**Cost model:** A artifacts (one per distinct file/URL the agent produced; 100–300 is common in a long coding session) → 2A queries and A×A decoded artifact rows per panel open/reconnect. 300 artifacts is 600 queries and 90 000 row decodes with JSON payloads, on the shared 8-connection pool, plus a full transcript fold for a live session.

**Fix:**
- In `list_artifacts`, resolve the work item once, load the existing `(kind, ref)` set once (`SELECT kind, "ref" FROM work_artifacts WHERE work_item_id = ?`, no payload), and insert only the missing ones. Or drop the re-registration from the read route entirely, since the live tail already registers each new artifact.
- In `register_work_artifact` (still used by the tail), replace `artifacts_for` + scan with `has_artifact(&item.id, kind, reference.as_deref())`.
- Serve a live session's artifacts from the tail's fold (a `live_artifacts` accessor like `live_tool`) before falling back to `cached_fold`.

### backend-performance-2-02: History page re-scans the provider roots for every session whose transcript is gone, on every page and search keystroke (minor)

**Where:**
- `crates/otto-server/src/routes/history_page.rs:109` (`budget = 4 * limit`, default 400 candidates) and `:131-151` (every candidate goes through `resolve_transcript_sync`)
- `crates/otto-server/src/routes/transcript.rs:146-176` (`resolve_transcript_sync`: a persisted path that no longer exists falls through to the root scan)
- `crates/otto-sessions/src/lifecycle.rs:159-176` (Claude: stat the exact path, then `read_dir(claude_root)` and stat `<dir>/<psid>.jsonl` in **every** project dir)
- `crates/otto-sessions/src/manager.rs:479-514` (Codex: `codex_rollout_path_under` walks the whole `~/.codex/sessions` tree when the rollout is missing)
- UI trigger: `ui/src/modules/agents/history/HistoryPage.svelte:84` (search refetch, 250 ms debounce) and every scroll page.
- Legacy route still mounted: `crates/otto-server/src/routes/mod.rs:664` → `transcript::history` (`transcript.rs:1097-1279`)

**Evidence:**
- Claude prunes transcripts after about 30 days, and the History page exists to list old sessions. For every such session, `resolve_transcript_sync` sees `persisted` (`p.is_file()` false), then runs `transcript_path_in_roots`, which lists `~/.claude/projects` and stats one candidate per project directory before returning `TranscriptMissing`.
- The resolved `None` drops the entry (`history_page.rs:166` needs both `session` and `resolved`; session candidates carry `indexed: None`), so the scan result is thrown away and nothing records it. The next page, filter change or debounced keystroke repeats the scan for the same dead sessions.
- A Codex session whose rollout was removed costs a full recursive walk of the rollout tree per candidate (`walk` sorts and descends every date directory).
- The legacy `GET /workspaces/{wid}/history` (no longer called by the UI, still exposed to API/agent tokens) does the same resolution for **every** agent session in the workspace with no bound. It also issues one `transcript_index.get(path)` per session (`transcript.rs:1180-1187`) and reads `transcript_path` and `provider_session_id` for every session machine-wide (`:1135-1152`).

**Cost model:** Per History request: (pruned Claude candidates ≤ 400) × (project dirs, often 100–300) stat calls, about 40k–120k syscalls on the blocking pool. Codex misses cost a full tree walk each. This is paid again on every page, filter change and search keystroke, and it grows as sessions age past the prune window.

**Fix:**
- When a persisted path exists and is missing, return `TranscriptMissing` without the root scan. The persisted path is the file the capture scan found, and a missing file means the provider pruned it.
- Or record the miss: `UPDATE sessions SET transcript_path = NULL, transcript_missing_at = ?` (additive column) and skip resolution for rows marked missing within a TTL. Or exclude them from `history_page::candidates` in SQL.
- For the list view, resolve only the persisted path (cheap stat) and leave the root scan to the transcript open route, which persists what it finds.
- Retire the legacy unbounded `/workspaces/{wid}/history` route, or make it delegate to `history_page::list`.

### backend-performance-2-03: transcript fold cache re-serializes every fold just to size it (nit)

**Where:** `crates/otto-server/src/transcript_cache.rs:161-201` (`Snapshot::charge`), called at `:351` for every completed fold.

**Evidence:**
- `charge()` runs `serde_json::to_writer` over every `FoldedTurn.turn`, all artifacts and all subagent metadata into a byte-counting sink. This is a full JSON serialization of the fold (up to `MAX_ENTRY_BYTES = 32 MiB`), and its only output is a size estimate (×2 + constants).
- It runs inside the fold job on the shared 2-permit `fold_workers()` pool, so it lengthens the time the permit is held. The live tail's initial fold and other queued reads wait on that same pool.
- The tail already sizes the same data with a cheap estimate: `transcript_tail.rs:118` `fold_charge(bytes, records)` from the source length and record count.

**Cost model:** One extra full serialization per cache fill. For a 30 MB fold that is tens of milliseconds of CPU on the 2-permit pool, paid on every miss: a new file, an append, or eviction after 120 s idle.

**Fix:** Charge with the same estimate as the tail: `fold_charge(stamp.target.bytes, folded.record_count())` plus sidecar sizes. Or have the fold accumulate its retained bytes as it builds turns. Drop the serialization pass.
