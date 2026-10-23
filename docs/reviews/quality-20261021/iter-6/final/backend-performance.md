# Backend performance review — quality iteration 6, FINAL (2026-10-23)

Lens: backend-performance. Rust hot paths: N+1 queries and queries in loops, missing indexes, blocking IO on the runtime, unbounded memory, needless clones and serialization, polling loops, per-WS-message work, ClickHouse/usage cost.
Worktree: `fix/quality-iter6-20261023` @ 2065f00e9. Read-only review: no builds and no tests were run (grep/rg/sed/git only).

## Score

Start 10.0. Open findings: 0 blocker, 0 major, 2 minor, 1 nit.

10.0 − 2×0.1 − 1×0.03 = 10.0 − 0.20 − 0.03 = **9.77 / 10**

Wave 0 scored 9.78. All five wave-0 findings are fixed, including 6-05 (the residual of 5-04). The new findings come from areas wave 0 listed as not covered. 6f-01 and 6f-02 are in agent-launch setup, which wave 0 did not trace (the "otto-context materialize … async call sites were not traced" item). 6f-03 is what is left after the 6-05 fix: the new keyset pager has no index to seek on. None of the three was introduced by this iteration's fixes.

## Previous findings (wave 0)

| id | status | evidence in current code |
|----|--------|--------------------------|
| 6-01: DB exports write synchronously on a tokio worker, and `/db/export` buffers the whole result | **fixed** | `crates/otto-dbviewer/src/export.rs:326-424`: `ExportWriter::Channel` is an in-memory 64 KiB chunk buffer feeding a bounded `mpsc::channel(8)`. `pace()` awaits `tx.send` once a chunk is full. `drain_to_file` (`:430-442`) owns `File::create` and every `write_all` on one `spawn_blocking` task (`blocking_recv`). `driver.rs:290-309`: `export_to_path` joins the driver with that drain. Every streaming driver paces each row or block: mysql `:1000`, postgres `:915`, mongodb `:899`, clickhouse `:3132/:3224/:3284`, each followed by `finish_paced`. `http.rs:794-833`: `/db/export` streams the bounded receiver as the body, and the `ChannelWriter` and unbounded channel are gone. |
| 6-02: the Mongo driver never sweeps its completion cache | **fixed** | `crates/otto-dbviewer/src/drivers/mongodb.rs:734-745`: `evict_idle` calls `self.completions.sweep()`, and calls `completions.invalidate(key)` for each evicted client next to the `samples` invalidation. |
| 6-03: the MCP governance switch runs launcher-file IO and a std `Mutex` on a runtime worker | **fixed** | `crates/otto-server/src/routes/resource_access.rs:160-171`: the `retire_user_server` loop over every root runs inside one `tokio::task::spawn_blocking` and returns the first error. The live-session kill loop stays async. |
| 6-04: the story test-case tab loads each run's cases with its own query | **fixed** | `crates/otto-product/src/http.rs:971-981`: one `list_testcases_for_runs(&ids)` call, grouped in memory. `crates/otto-state/src/product.rs:1551-1576`: `WHERE run_id IN (…)`, chunked at 500, served by `idx_product_testcases_run` (`migrations/0024_product.sql:130`). A test covers it (`:2281`). |
| 6-05: the admin session overview returns every session ever created, unpaginated (residual of 5-04) | **fixed (as specified); a new residual is 6f-03** | `crates/otto-server/src/routes/admin_sessions.rs:92-153,194-221`: unarchived sessions only unless `include_archived`, keyset-paged on `(created_at, id)` with a hex cursor. The default page is 200 and the maximum is 1000. The UI pages with `next_cursor` (`ui/src/modules/settings/AdminSessions.svelte:59-91`). The response no longer grows with the table. The SQL still scans the table on every page, which is 6f-03. |

## Areas covered

- Re-verification of every wave-0 finding (table above), at every listed location.
- The iteration's crate diff (`git diff origin/main --stat -- crates apps`: 271 files, mostly moves and splits), checked for new hot-path costs:
  - **Transcript roots** (`otto-transcript/src/roots.rs`, architecture-6-01): `for_session` does no filesystem listing. `discover` runs on the blocking pool in the usage tailer (`tailer/driver.rs:749`, a full listing every 600 s with a watcher) and in the History index scan (`history_index.rs:189-195`). It also runs inline in `confine_history_path` (`routes/transcript.rs:944`): one `read_dir($HOME)` plus a few canonicalizes per History transcript, artifact or image request. That is sub-millisecond local IO, so it is not reported. `persist_transcript_path` and `resolve_transcript` stay off the runtime (`manager/transcript_roots.rs:31`, `transcript/resolve.rs:134`).
  - **Usage tailer**: Codex files are now canonicalized for dedup too. This happens only on full listings, on the blocking pool. Not reported.
  - **Budget gate** (`budget/mod.rs`): the admission verdict is now pure (`verdict_from`). Workspace-name lookups moved to the status view only, which is an improvement.
  - **MCP**: `governance::run` is the same stage order and query count as before. The session `tools/list` catalog is built once per process (`ottod/src/mcp_tools/catalog.rs:20`, `LazyLock`). `otto.query_db` returns masked rows capped at 200 and has a 60 s statement timeout.
  - **Agent-run split** (`otto-agent-run`): `session_turn.rs` polls once per second with an incremental tail (`poll_claude_tail`, O(new bytes)). The fan-out retry wait (`fanout/retry.rs:240-262`) checks only in-memory callbacks. The assist live-file poll is stat-gated (`assist/mod.rs:306-330`).
  - **Product run split** (`run/*.rs`): the same per-run context writes as before the split. No new per-poll database work.
  - **Swarm**: `acting_user` (`runtime/run.rs:309-385`) runs a few point queries per turn, bounded by `MAX_ORIGIN_HOPS`.
  - **Mongo keyset** (`drivers/mongodb/keyset.rs`): the `$or` of `_id $gt` and `_id $type` still uses the `_id` index for both branches, and the result is limited.
  - Also checked: channels mirror upload fallback, insights scheduler (IO via `spawn_blocking`/`ctx::blocking`), `state_archive` profile merge (a small table), review `summarize.rs` (cancel check batched), workflow `run/schedule.rs` (moved code).
- Areas listed as not covered in wave 0:
  - `otto-state/src/workbench.rs`: revisions are keyset-paged on the `(doc_id, seq)` primary key (`:752-776`). Autosaves coalesce. Blob GC is a single `DELETE … NOT EXISTS` on `idx_workbench_revisions_hash`. The diff runs on `spawn_blocking`. Not reported.
  - `otto-orchestrator`: the `claude_pty` reply wait tails incrementally on the blocking pool (`poll_reply_tail`, `ReplyTail::poll`). Not reported.
  - `otto-git/src/providers/*`: one forge request per PR page, with an ETag cache (`github.rs:671-705`). PR detail fetches its lists concurrently. Not reported.
  - `otto-context` materialize call sites: traced. Personal agents and swarm provision on `spawn_blocking`. The assistant does not (finding 6f-02). The trust pre-grant that every agent launch pairs with it led to 6f-01.
  - `otto-design` GC: the orphan sweep and `gc_blobs` run under the exclusive GC fence on the scheduled pass. `blob_in_use` is checked once per candidate and the publish pins are indexed once per pass. Not reported.
- Sweeps over the added lines of this iteration's diff for `std::fs::`, `unbounded_channel`, `std::thread::sleep` and `block_on`. The hits are moved code, test code, blocking-pool closures, or single small writes per agent run.

## Areas NOT covered

- `otto-apiclient` engine and `otto-rooms` extraction: these are moves behind new ctx traits. Their diffs were skimmed for added loops, but the moved code was not re-read in full.
- `otto-design-assist/src/job.rs` and `otto-canvas/src/assist.rs` beyond the trust and context writes.
- ClickHouse usage query shapes. They were unchanged this iteration and not re-read.

## Findings

### backend-performance-6f-01: about 20 agent-launch paths call the synchronous `ensure_trusted` from async code, so `~/.claude.json` is read, parsed and rewritten under a process-wide std `Mutex` on a tokio worker (minor)

**Where:**
- `crates/otto-sessions/src/trust.rs:152-168` (`ensure_trusted`, sync) and `:105-150` (`update_config`). On a memo miss, `update_config` takes `CONFIG_LOCK` (a `std::sync::Mutex`, `:37`), then `read_to_string`, a full `serde_json` parse in `claude_with_trust` (`:249-…`), a serialize, and a temp-file write plus rename.
- `:170-178`: `ensure_trusted_async` exists for exactly this reason. Its doc says that, called inline, it "parked the tokio worker (and, in a review or workflow fan-out, every worker queued on the lock)". Only `SessionManager` uses it (`manager.rs:2737,5527`).
- Sync calls made directly from `async fn`s, with no blocking hop:
  - `otto-server/src/db_assist.rs:291` (`assist`)
  - `otto-server/src/routes/browser.rs:1052` (`summarize_page`)
  - `otto-server/src/modules.rs:303` (`orchestrate`) and `:760` (`db_explain_with_agent`)
  - `otto-server/src/modules/review/run.rs:384` (`run_review_core`, per provider)
  - `otto-server/src/skill_review.rs:467` (`retry_agent`), `:604` (`apply_fixes`) and `:741` (`run_review_inner`, per provider, after its `spawn_blocking` has closed)
  - `otto-server/src/skill_eval.rs:1314,1416` (`run_skill_eval_core`, per provider)
  - `otto-server/src/vault_docs_agent/run.rs:73` and `refine.rs:115`
  - `otto-server/src/assistant_host.rs:222` (`run_agent_session`)
  - `otto-server/src/mockup_assist.rs:219` (`load`)
  - `otto-insights/src/lib.rs:1025` (`run_insights`)
  - `otto-automation/src/scheduled_tasks_engine.rs:948` (`execute_agent`)
  - `otto-assistant/src/assistant/threads.rs:574` (`open_session`)
  - `otto-design-assist/src/lib.rs:1782` (`start_assist`) and `:1930` (`start_variants`, once per variant in a loop)
  - `otto-product/src/run/context.rs:104-110` (`trust_providers`, called from `analysis.rs:111` and `plan.rs:116`) and `run/scratch.rs:55` (`trust_session_cwd`, called from `retry.rs:122`, `tests_gen.rs:106` and `rewrite.rs:95`)
  - The sync ctx methods `otto-server/src/modules/ctx_impls.rs:539-544`, called from `otto-canvas/src/assist.rs:224,509`

**Evidence:**
- The fingerprint memo (`trust.rs:84-100,114-118`) makes a repeat call for the same cwd a single `stat`, but only while `~/.claude.json` is unchanged. Every running Claude CLI rewrites that file, so with active sessions the fingerprint keeps moving and the memo misses.
- A new cwd always misses and also writes. Several of these paths create a fresh directory per run:
  - `db_assist` uses `data_dir/db_assist/<assist_id>` (`db_assist.rs:240-243`).
  - Design variants use `root/<k>` per variant (`lib.rs:1927`).
  - Skill review uses a staged temp directory.
- The surrounding code in those functions is careful: `tokio::fs::write` for `SCHEMA.md` and `CONTEXT.md` (`db_assist.rs:273-285`), and an async `write_context` (`design-assist lib.rs:1929`). The trust grant is the one blocking step left on the worker.
- Every one of these launches then goes through `SessionManager::create`, which trusts the same cwd again off the runtime (`manager.rs:2733-2738`). The inline pre-grant is the expensive copy of work the spawn path already does safely.

**Cost model:**
- A memo miss costs a full read, parse and serialize of a file that the code itself calls "routinely megabytes": tens of ms of CPU and IO on one runtime worker.
- `CONFIG_LOCK` is a std mutex. In a fan-out (several reviewers, design variants, product lenses, vault-doc writers) the second caller parks a second worker for as long as the first holds the lock.
- PTY relay, WS fan-out and HTTP routes share those workers.

**Fix:**
- Replace each listed call with `otto_sessions::trust::ensure_trusted_async(..).await`, and add a `ensure_trusted_scratch_async` twin for the scratch helpers.
- Make the canvas ctx trait methods async, or have them call the async helper.
- Where the session is created through `SessionManager::create` with no account, the pre-grant can simply be dropped, because the spawn path already trusts that cwd off the runtime.
- Optionally add a clippy `disallowed-methods` entry for `otto_sessions::trust::ensure_trusted` outside `otto-sessions`, so new async callers cannot reintroduce it.

### backend-performance-6f-02: opening an assistant thread session copies the whole skill library on a runtime worker (minor)

**Where:**
- `crates/otto-assistant/src/assistant/threads.rs:620-643` (`ensure_workspace`, reached from `open_session` at `:573`). It calls `otto_context::materialize::provision(...)` (`:635`) inline in the async fn.
- `crates/otto-context/src/materialize.rs:166-183` (`provision_with_home` → `plan` → `execute`):
  - `cfg.skills` is `None` here (`threads.rs:627-631` uses `..Default::default()`), so `active_skills` returns `library.list_skills()` (`:372-377`), which is a `read_dir` plus a `get_skill` read per skill (`library.rs:292-309`).
  - `execute` (`:656-700`) then does a `remove_dir_all` and a `copy_dir_all` of every skill directory, plus the CLAUDE.md / AGENTS.md writes.

**Evidence:**
- The same call is already off the runtime at the other two call sites, for this exact reason:
  - `otto-assistant/src/personal_agents_engine.rs:380-392`: "Skill-tree copies + CLAUDE.md/AGENTS.md writes are a burst of blocking file IO … off the runtime worker … (perf §15 N9)", inside `spawn_blocking`.
  - `otto-swarm/src/runtime/run.rs:520-539`: `provision_agent` inside `spawn_blocking`.
- The assistant thread path was missed. It runs whenever an assistant thread opens or reopens its background session. Those sessions are reclaimable by the idle sweep (`threads.rs:575-579`), so a reopen is routine.

**Cost model:** one full skill-library materialization per assistant session open. That is S skills times (one `rmdir -r` plus a recursive copy of SKILL.md, references and scripts), roughly hundreds of syscalls for a few dozen skills, all on one tokio worker. It is followed by the 6f-01 trust parse on the same worker (`:574`).

**Fix:**
- Wrap the `provision` call in `tokio::task::spawn_blocking`, cloning the library, cfg, cwd and provider as `personal_agents_engine.rs:383-392` does.
- Fold the `ensure_trusted` call (`:574`) into that same blocking hop, or switch it to `ensure_trusted_async`.

### backend-performance-6f-03: the admin overview's keyset pager has no index to seek on, so every page scans the whole `sessions` table (nit)

**Where:**
- `crates/otto-server/src/routes/admin_sessions.rs:204-207`:
  ```sql
  SELECT … FROM sessions WHERE (? OR archived = 0)
  AND (? IS NULL OR created_at < ? OR (created_at = ? AND id < ?))
  ORDER BY created_at DESC, id DESC LIMIT ?
  ```

**Evidence:**
- No `sessions` index leads with `created_at` or `archived`. The candidates are:
  - `idx_sessions_list_order (workspace_id, archived, created_at, id)` (`migrations/0158_sessions_list_order_index.sql:7`), which leads with `workspace_id`.
  - `idx_sessions_ws_archived (workspace_id, archived)` (`0002`).
  - `idx_sessions_created_by` (`0042`).
- The bound-parameter `(? OR archived = 0)` also keeps the planner from using any index on `archived`.
- So each page, the first one included, is a full table scan plus a top-N sort.
- `archived` was added by `ALTER TABLE` (`0002_session_archived.sql:3`), so it is the last column, after `meta_json`. Reading it walks each row's overflow chain. The scan costs the whole table's bytes, not just its row count.
- The 6-05 fix bounded the response size. It did not bound the query.

**Cost model:** O(all sessions × row size) of page reads per admin page open, "load more" or post-action refresh. That is tens of MB on a long-lived install with large `meta_json` rows. The page is admin-only and rarely opened.

**Fix:**
- Add a migration with `CREATE INDEX idx_sessions_admin_overview ON sessions(archived, created_at DESC, id DESC)`.
- Split the SQL into two static variants, `WHERE archived = 0 AND …` and one with no archived predicate, chosen in Rust, so the default view seeks the index.
- Alternatively, use the partial index `… ON sessions(created_at DESC, id DESC) WHERE archived = 0` for the default view.
