# Data-integrity review — quality iteration 2 (2026-10-22)

Lens: persistence integrity. Covers otto-state SQLite and its migrations (append-only and rollback
compatibility), transactions, partial writes, non-atomic file writes, data loss on crash or restart,
caches that disagree with their source of truth, and id/ownership consistency.

Branch `fix/quality-loop-20261021` @ ab0a86f05. Read-only review: no builds, tests or e2e were run.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 4 | 0.40 |
| nit | 1 | 0.03 |

**Score = 10.0 − 0.30 − 0.40 − 0.03 = 9.27**

## Previous findings (iteration 1)

| Id | Severity | Status | Evidence |
|---|---|---|---|
| data-integrity-1-01 | major | **Fixed** | `DELETE /swarm/swarms/{sid}` and `/swarm/projects/{pid}` now live in `crates/otto-swarm/src/runtime/engine.rs:2753-2810`. Both take `operation_guard`, stop the runs, `signal_cancel` each one, kill its session (`halt_runs`, :2709-2723), and only then delete. The swarm path also stops the coordinator and verifier and sweeps helper sessions. |
| data-integrity-1-02 | minor | **Fixed** | `crates/otto-state/src/swarm.rs:877-918` and `:1206-1240`: one `begin()` (IMMEDIATE) transaction each. `swarm_goals` and `swarm_channel_triggers` were added to the swarm cascade, and goals to the project cascade. I checked the migrations: no other table has a `swarm_id` column (`product_discovery_runs` aside). |
| data-integrity-1-03 | minor | **Fixed** | `approve_edit` and `rollback_edit` (`crates/otto-improve/src/engine.rs:722-733`, `:831`) call `backup_and_replace` (:1341): a `.bak`, then temp + rename, with symlinks resolved. A re-created file is restored with `create_new`. |
| data-integrity-1-04 | minor | **Partially fixed, carried as 2-03** | `mcp_control.rs` now uses single `COALESCE` updates. `users.rs`, `aws_accounts.rs`, `workspaces.rs` and `api_client.rs` are unchanged, which FIXES.md "Not done" confirms. Two more repos with the same pattern were found (see 2-03). |
| data-integrity-1-05 | minor | **Fixed** | `crates/otto-design/src/store.rs:692-721`: a single `DELETE … AND NOT EXISTS`, with Conflict or NotFound reported afterwards. |
| data-integrity-1-06 | minor | **Fixed** | `crates/otto-design/src/blobs.rs:74-113`: `write_synced`, rename, then a directory fsync. A size-mismatched (torn) blob is republished. |
| data-integrity-1-07 | minor | **Fixed** | `crates/otto-state/src/notifications.rs:138-200`: lookup, then refresh or insert, inside one `BEGIN IMMEDIATE`. |

## Areas covered

- **Re-verification of every iteration-1 finding** (table above).
- **Pool routing semantics** (`crates/otto-state/src/pool.rs`). `DbPool::begin()` is `BEGIN IMMEDIATE` on a 2-connection writer pool, and plain `SELECT`s go to the 8-connection read-only reader pool. I confirmed that read-then-write inside `begin()` transactions cannot hit `SQLITE_BUSY_SNAPSHOT`, and that `add_version`'s MAX+1 allocation is safe.
- **Offline compaction and DB swap** (`crates/otto-state/src/maintenance.rs`, `crates/otto-server/src/boot/open.rs`): `VACUUM INTO` → `quick_check` → schema and row-count fingerprint → fsync → marker → rename. The swap is confirmed right after open/migrate and before any listener, and rolled back at the next start if unconfirmed. No issues found.
- **Workflow run persistence and crash recovery** (`crates/otto-state/src/workflows.rs`):
  - CAS `update_run_if`, `request_cancel` (it does not touch node state), and publish+snapshot in one transaction.
  - `delete` guards live runs in a single statement.
  - `interrupted_runs` / `prepare_resume` / `fail_interrupted_runs`.

  No issues found.
- **Scheduled tasks** (`crates/otto-state/src/scheduled_tasks.rs`, `routes/scheduled_tasks.rs`): generation-fenced `settle_generation` and `admit_run`; `prune_runs` checks report-path ownership inside IMMEDIATE; `reap_running` leaves workflow hand-offs alone. The delete route cancels the live run and waits for it to settle (personal agents use the same path). No issues found.
- **Database-changes ledger** (`crates/otto-state/src/database_changes.rs`, `crates/otto-server/src/database_changes.rs`): the claim / attempt / finish / cancel / reconcile state machine and boot recovery. Two gaps found (2-04).
- **Vault revision history** (`crates/otto-vault/src/recovery.rs` `prepare_revision`, `commit_revision`, `write_light`, `write_blob`). Revision bodies are written durably before `meta.json` (committed), and a coalesced `a-<sha>` is retired only after the commit. No issues found.
- **Encrypted secret store** (`crates/otto-keychain/src/encrypted.rs`): `write_atomic` (fsync, rename, directory fsync) under a process lock. No issues found.
- **otto-memory vault mirror** (`write_atomic`, and the hard-link publish in `write_missing`). No issues found.
- **Proof media store** (`crates/otto-state/src/proof.rs` `media_put`). No issues found.
- **Product story cascade** (`delete_story`, one transaction) and version allocation. No issues found.
- **Channel integrations**: upsert and delete, and the keychain refs. The refs are deterministic, so a ref race cannot orphan a secret. No issues found.
- **Library / context store and its in-process skill cache**, plus every writer that targets `<data_dir>/library` (UI editor, skill-eval promote, self-improvement approve / auto-apply / rollback, the design-rule skill, bundled install/update, product seeding). Two findings (2-01, 2-02).
- **API client ownership references** (collection `parent_id`, request `collection_id`) and the UI tree that renders them (2-05).
- **A sweep of every production `fs::write` / `File::create` call site in `crates/` and `apps/desktop`.** Out of 120 sites, I triaged the ones that write user-owned state; most are regenerable temp, prompt or bundle files.
- **A re-run of the multi-autocommit sweep** over `otto-state` plus the crates that issue their own SQL (`otto-rbac/tokens.rs`, `otto-design/store.rs`, `otto-vault/store.rs`, `otto-usage`, workgraph projector, `otto-mcp`, `otto-connections`, `otto-product`).
- **otto-git `merged_sets_cached`**: the memo is keyed on base oid plus every ref tip, and a failed walk is never memoized. No consistency issue.
- **UI autosave flush on teardown**:
  - Vault: `pagehide` flush.
  - Excalidraw: `onDestroy` commits and saves the pending edit.
  - Mermaid / D2: `flushPendingCode`.

  No issues found.

## Areas NOT covered yet (for iteration 3)

- `crates/otto-sessions/src/manager.rs` (45 commits in 3 weeks): whether DB session status/meta agrees with the in-memory registry across a restart, and the dormant/auto-archive passes (`mark_dormant_except`).
- `crates/otto-product/src/run.rs` and the story watcher (analysis/agent status lifecycle and resume after a restart: `list_unfinished_agents`, `bump_resume_count`).
- `crates/otto-vault/src/watch.rs` / `scan.rs`: whether external edits are reconciled into the derived index after a crash between a file write and an index update.
- otto-workgraph `append_event` / `upsert_item`, and the projector (`crates/otto-server/src/workgraph_projector.rs`).
- `crates/otto-usage/src/ingestion.rs` and the `ottod` usage tailer cursors (double-counting or a skipped range after a crash).
- `crates/otto-connections/src/transfers.rs` (transfer job state) and the k8s monitor persistence.
- otto-assistant task lifecycle (`crates/otto-state/src/assistant.rs` tasks, routing limits).
- UI localStorage drafts outside vault and canvas (API client unsaved request tabs, DB explorer query tabs, chat composer).

---

## Findings

### data-integrity-2-01 — major — The Library skill cache is never invalidated by the writers that bypass `Library`, so prompts and the editor serve the pre-edit skill, and saving from the editor silently reverts an applied change

- **Files:**
  - `crates/otto-context/src/library.rs:247-275` (`get_skill`), `:541-545` (`evict`)
  - `crates/otto-improve/src/pathsafe.rs:67-71`
  - `crates/otto-improve/src/engine.rs:656`, `:728`, `:831`, `:1341`
  - `crates/otto-improve/src/design.rs:147-160`
  - `crates/otto-skills/src/lib.rs:221-236`
  - `crates/otto-review/src/engine.rs:125-137`
  - `crates/otto-context/src/http.rs:166-193`
- **Evidence:**
  - **The cache is never re-checked against disk.** `Library::get_skill` returns the cached `LibrarySkill` without looking at the file at all: no mtime or size check (`library.rs:250-254`). Only `put_skill`, `write_skill_file` and `delete_skill` evict, and they do it through the private `evict` (`:286`, `:298`, `:365`).
  - **Self-improvement writes go around it.** Skill edits resolve to the Library file first: `resolve_target` returns `<library_root>/skills/<ref>/SKILL.md` when it exists (`pathsafe.rs:67-71`). The engine then writes that file directly with `backup_and_replace` in `safe_auto_apply` (:656), `approve_edit` (:728) and `rollback_edit` (:831). The design-team-style rule skill uses the same path (`design.rs:147-160`). None of these goes through `otto_context::Library`, so the daemon's shared `ctx.context_library` cache keeps the old body.
  - **Bundled install/update also goes around it.** `otto_skills::install_into` runs `remove_dir_all` and re-seeds `<library>/skills/<name>` (`lib.rs:230-234`) without evicting.
  - **What readers then see:**
    - `GET /library/skills/{name}` and `list_skills` (`http.rs:166-176`, used by the Library editor through `ui/src/lib/api/context.ts:25-29`) keep returning the old body and version.
    - `otto_review::resolve_skill_inline` (`engine.rs:132-137`) inlines `library.get_skill(name)`. It is used by code-review lenses, workflow nodes (`workflow_engine.rs:620`, `:6044`), scheduled tasks (`automation_ctx.rs:202`) and vault-docs agents. All of them keep running with the pre-edit skill until the daemon restarts, while the improvement row says `Applied`.
  - **The data-loss path.** An editor opened after an approved improvement (or after a bundled update) shows the stale cached body. Saving it (`PUT /library/skills/{name}` → `put_skill`) overwrites the applied change on disk. The UI gives no conflict signal, and the improvement's later `rollback_edit` then reports `Conflict`.
  - **A second gap in `get_skill`.** It reads the file and inserts the parsed result in two steps (`:258`, `:271`). A reader that read the file just before a `put_skill` can insert after the writer's `evict`, which pins the previous body in the cache.
- **Fix:**
  - Make the cache self-validating. Store `(mtime, len)` (or a content hash) next to each entry and re-stat in `get_skill`; on a mismatch, re-read. That covers every out-of-band writer (improve, design rules, `install_into`, a user editing `~/.otto/library` by hand) and closes the insert-after-evict race.
  - Alternatively, expose `Library::evict_skill(name)` and call it from `install_into` and from the improve engine (pass a callback or the `Library` into `ImprovementEngine`). Also re-check after insert: only insert if the file's stat is unchanged.
  - Add a regression test: `put_skill` → `get_skill` (cached) → external `std::fs::write` of `SKILL.md` → `get_skill` must return the new body.

### data-integrity-2-02 — minor — Library skill, soul, context and default-soul writes truncate the only copy in place

- **File:** `crates/otto-context/src/library.rs:277-290` (`put_skill`: `fs::write(path, body)` at :284), `:350-366` (`write_skill_file`), `:623-625` (`set_default_soul`), `:662-667` (`write_md_entry`, used by `put_soul` / `put_context`)
- **Evidence:**
  - **Every write is an in-place truncate.** All four paths use `std::fs::write`: open with `O_TRUNC`, then write.
  - **This is the source of truth.** The library at `<data_dir>/library` is what the improve engine's own docs call "the library is the source of truth" (`otto-improve/src/engine.rs:59-60`). Provider bundles are regenerated from it, and no copy exists elsewhere.
  - **Unguarded callers:**
    - The root-only Library editor (`http.rs:179-193`, `:280-293`, `:424`, `:474`).
    - Skill-eval promotion, which overwrites an existing library skill with the iteration body (`crates/otto-server/src/skill_eval.rs:2748-2750`) and keeps no backup.
  - **Result:** a crash, kill or full disk mid-write leaves an empty or partial `SKILL.md` or soul, and nothing restores it.
  - **The atomic pattern already exists elsewhere.** `otto-improve::backup_and_replace`, `otto-memory::VaultWriter::write_atomic`, `otto-keychain::write_atomic` and the vault engine all write a temp sibling, fsync, rename, and fsync the directory. Data-integrity-1-03 fixed the same defect on the improve path.
- **Fix:** Add one `write_atomic(path, bytes)` helper in otto-context: a temp file in the same directory, `sync_all`, `rename`, then a directory fsync. Use it in `put_skill`, `write_skill_file`, `write_md_entry` and `set_default_soul`. For `skill_eval` promote over an existing skill, write a `.bak` first, as `install_one(backup=true)` already does.

### data-integrity-2-03 — minor — Multi-field PATCH repos still write each column as its own autocommit (remainder of 1-04, plus two more repos)

- **Files:**
  - `crates/otto-state/src/api_client.rs:511-543` (`update_environment`: name, `variables_json`, `secret_keys_json`), `:242-265` (`update_collection`), `:622-648` (`update_automation`)
  - `crates/otto-state/src/users.rs:142-174`
  - `crates/otto-state/src/aws_accounts.rs:159-202`
  - `crates/otto-state/src/workspaces.rs:196-238`
  - **New:** `crates/otto-state/src/mcp_servers.rs:128-186` (6 statements)
  - **New:** `crates/otto-state/src/broker_clusters.rs:184-253` (1 bulk statement plus up to 6 more)
- **Evidence:**
  - Each `if let Some(v)` block runs `.execute(&self.pool)` on its own, with no transaction. FIXES.md lists the first four as "Not done".
  - **api_environments:** if the `secret_keys_json` write fails after `variables_json` committed, the API returns an error but leaves a new variable stored and not marked secret.
  - **broker_clusters:** the safety flags `environment` and `read_only` are written last, after `bootstrap_servers` and the credential refs. A failure in between (or a concurrent reader on the WAL reader pool) sees a cluster that already points at the new, possibly production, brokers with the old `read_only = 0`.
  - **mcp_servers:** a failure after the `command` write but before `env_json` leaves a server whose command and env disagree.
- **Fix:** Collapse each update into a single `UPDATE … SET col = COALESCE(?, col), …, updated_at = ? WHERE id = ?`, as `canvas.rs` and `mcp_control.rs` already do. Or wrap the existing statements in one `self.pool.begin()` / `tx.commit()`.

### data-integrity-2-04 — minor — The reviewed-database-change ledger can mislabel an attempt that may have committed as `failed`, or strand a change in `outcome_unknown` with nothing left to reconcile

- **Files:** `crates/otto-state/src/database_changes.rs:374-390` (`finish`), `:422-454` (`reconcile`), `:456-468` (`recover_interrupted`); `crates/otto-server/src/database_changes.rs:316-334` (`run_claimed`)
- **Evidence:**
  1. **A possibly-applied attempt is recorded as `failed`.**
     - `run_claimed` records an attempt's outcome with `let _ = repo.finish_attempt_progress(...)`, discarding the error (:318). It then always calls `repo.finish`.
     - If that write failed (SQLITE_BUSY after the 5 s busy timeout, or a full disk), the attempt is still `running`.
     - `finish`'s classifier (`SUM(state='outcome_unknown')>0 → outcome_unknown; all succeeded → succeeded; any succeeded/partially_applied → partially_applied; cancellation → cancelled; ELSE failed`) has no case for `running`. A first attempt that actually executed SQL on the target is therefore finalized as `failed`.
     - That status is terminal: `reconcile` requires `old.status == "outcome_unknown"`.
     - At the next boot, `recover_interrupted` flips the attempt to `outcome_unknown` but only updates changes `WHERE status='running' OR (status='outcome_unknown' AND cancellation_requested=1 …)`. The change stays `failed` and holds an unknown attempt that can never be reconciled. The ledger contradicts itself about a production schema change.
  2. **A reconcile can strand the change.**
     - `reconcile` commits the attempt's new state in one transaction, then calls `self.finish(...)` as a second transaction (:453).
     - If `finish` fails, the change stays `outcome_unknown` but no attempt is `outcome_unknown` any more. Every retry of `POST /database-changes/{id}/reconcile` returns Conflict, because the guarded `UPDATE … AND state='outcome_unknown'` matches 0 rows.
     - `recover_interrupted` only repairs this state when `cancellation_requested=1`, so a plain reconcile is stuck forever.
- **Fix:**
  - In `finish`, inside the same transaction, first run `UPDATE database_change_attempts SET state='outcome_unknown', summary=… WHERE change_id=? AND state='running'`. Also make the classifier treat `running` as unknown.
  - In `run_claimed`, retry or propagate a `finish_attempt_progress` error instead of `let _ =`.
  - Do the reconcile attempt update and the finish classification in ONE transaction: factor `finish`'s body into a `finish_in(&mut tx, …)`.
  - Extend `recover_interrupted`'s last clause to any `status='outcome_unknown'` change with no unknown attempt.

### data-integrity-2-05 — minor — API-client collection `parent_id` and request `collection_id` are never checked for workspace ownership or cycles, so the saved objects become invisible

- **Files:**
  - `crates/otto-server/src/routes/api_client.rs:580-601` (`create_collection`), `:603-620` (`update_collection`), `:714-756` (`create_request` → `req_to_new`, :866-871)
  - `crates/otto-state/src/api_client.rs:242-265`
  - `crates/otto-state/migrations/0014_api_client.sql:10,22`
  - `ui/src/modules/api/CollectionsTree.svelte:109-140`
- **Evidence:**
  - **No validation anywhere.** The routes check only that the target row is in `wid` (`ensure_in_workspace` on the collection being renamed). `req.parent_id` and `req.collection_id` go straight into the row.
  - **The FK can't catch it.** It (`REFERENCES api_collections(id)`) only proves the id exists somewhere, in any workspace. Nothing rejects:
    - a parent in another workspace,
    - `parent_id = id`,
    - a parent that is the collection's own descendant.
  - **The UI loses the rows.** The tree is built from the `parent_id = null` roots (`buildTree(null, …)`), with `seen` only to stop infinite recursion. A collection in a cycle, or one whose parent is in another workspace, is never reached. Its requests are dropped too: `reqsByCol` files a request only under a collection the tree reaches, and a request with an unknown `collection_id` is excluded from the ungrouped list.
  - **Cross-workspace cascade.** `ON DELETE CASCADE` on `parent_id` means deleting a collection in workspace B removes a workspace-A collection that was filed under it. The A collection's requests then fall to `collection_id = NULL` through the request FK.
  - **Reachable without a crafted UI call.** Agents reach these routes through `otto_api_upsert_request` (`notify_changed` doc, :563-565), so a model-chosen `collection_id` from another workspace is enough.
- **Fix:**
  - In the routes, reject a `parent_id` or `collection_id` whose row is not in `wid` (one `SELECT workspace_id FROM api_collections WHERE id = ?`).
  - For `update_collection`, also reject `parent_id == id`, or any `parent_id` whose ancestor chain contains `id` (a recursive CTE `WITH RECURSIVE up(id, parent_id) …`).
  - Or enforce it in the repo: `INSERT … SELECT … WHERE EXISTS (SELECT 1 FROM api_collections WHERE id = ? AND workspace_id = ?)`.

### data-integrity-2-06 — nit — The Insights opt-in config is rewritten in place, and a torn file silently turns Insights off

- **File:** `crates/otto-insights/src/lib.rs:160-164` (`write_config`), `:152-157` (`read_config`)
- **Evidence:** `write_config` calls `std::fs::write(config_path(dir), json)`, which truncates and writes in place. `read_config` maps any parse error to `InsightsConfig::default()`, which is all-off. A crash mid-write therefore disables the user's scheduled insights with no error or log. The same file already uses temp + rename for the completion manifest (`mark_stopped`, :315-317).
- **Fix:** Write `config.json.tmp`, then rename it over `config.json`, as `mark_stopped` does. Optionally `tracing::warn!` when an existing `config.json` fails to parse instead of silently defaulting.
