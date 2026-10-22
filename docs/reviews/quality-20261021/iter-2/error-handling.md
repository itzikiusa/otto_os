# Error handling: quality iteration 2 (2026-10-22)

Lens: resilience and error UX. Covers swallowed errors, raw exceptions instead of inline Retry, missing recovery after a daemon restart or network drop, retry storms, misleading success or status messages, unhelpful error text and crash paths.

Branch `fix/quality-loop-20261021` at `ab0a86f05`. The review was read-only (grep, sed and git only; no builds or tests).

## Score

Rubric: start at 10.0 and subtract per open finding (blocker 1.0, major 0.3, minor 0.1, nit 0.03).

- 1 major × 0.3 = 0.30
- 5 minor × 0.1 = 0.50
- 1 nit × 0.03 = 0.03
- Total deductions: 0.83

**Score: 10.0 − 0.83 = 9.17 / 10** (iteration 1: 8.17)

## Previous findings (iteration 1)

| ID | Status | Evidence |
|----|--------|----------|
| 1-01 Product pollers | **Partly open** (now 2-01) | PlanTab and AnalysisTab are fixed: `pollFailures`/`lastPollError`, the "Couldn't check for the plan" timeout, and a stop on 403/404 (`AnalysisTab.svelte:154-157`). RewriteTab and TestCasesTab are unchanged. |
| 1-02 Raw exception text | **Partly open** (now 2-02) | Fixed in database, product and usage. `apiClient.svelte.ts:334` `errMsg` is still raw at 41 sites (FIXES.md lists it as deferred). More raw sites were found this pass. |
| 1-03 "Plan ready" after a failed body | Fixed | `loadPlanBody` returns a boolean. The poll keeps going on `false` (`PlanTab.svelte:171`). |
| 1-04 Extractor rejection text | Fixed | `problemOf` reads the text body and caps it at 300 chars (`client.ts:434-453`). |
| 1-05 Capabilities all-deny | Fixed | `capabilitiesError` with retry (`auth.svelte.ts:112-211`) and a banner (`App.svelte:167`). |
| 1-06 MCP lookup strips config | Fixed | `meta.mcp_servers_error`; unknown user set preserves the managed entries (per FIXES, with a test). |
| 1-07 `read_doc` treats a read error as empty | Fixed | `read_launcher` returns `Ok(None)` only for NotFound or whitespace (`mcp.rs:697-704`). |
| 1-08 Conflict resolver "No conflicted files" | Fixed | `loadError = loadErrorText(e)` (`ConflictResolverView.svelte:104`). |
| 1-09 Save & restart | Fixed | Separate try, with a "Directories saved, but the session didn't restart" toast and a Restart action (`SessionView.svelte:406-413`). |
| 1-10 Failed rollback | Fixed | The "enabled in BOTH registries" toast and a reload (`McpServers.svelte:276-288`). |
| 1-11 Git pull counts failures | Fixed | Imported, skipped and failed are counted separately (`apiClient.svelte.ts:1199-1226`). |
| 1-12 "Copied" unchecked | Fixed | Every `copyText` followed by a success toast now checks the result. |
| 1-13 Guest sees "Load failed" | Fixed | `JOIN_UNREACHABLE` (`room-client.ts:4-17`). The three modals use `loadErrorText` for a TypeError. |
| 1-14 One archive breaks recap | Fixed | Load and save failures are logged and skipped (`archive.rs:124-150`). |
| 1-15 Repo lookup cached forever | Fixed | The entry is deleted on failure and the cache is cleared on resync (`ConversationView.svelte:19-22, 287-298`). |

## Areas covered this iteration

- Re-verified all 15 iteration-1 findings against the current code.
- UI polling: the `lib/poll.ts` backoff contract (it backs off only when a run throws or returns `false`), and how the product pollers interact with it.
- Transport: `withDeadline` and `timeoutError` (sound; `loadErrorText` gives "The Otto daemon didn't answer in time."), `laneFetch` alias fallback, and the global `error`/`unhandledrejection` handlers in `main.ts` (sound).
- Stores: scheduledTasks (sound), aws, k8s, vault (autosave failure path and toast dedupe, sound apart from raw text), design-hall library, git (repos and PRs), transcript.
- Pages: Mission Control (inline error and banner, sound), Swarm Kanban, MCP AuditTab, Personal Agents (page, agent, rooms; sound), Insights, Proof, Skills-eval RunDetail, Vault DocsAgentsView (sound), Workflows page (delete, cancel and settle loop; sound), Composer send (sound), DatabasePage and QueryEditor, DesignArena (Blender), S3 download loop, PluginFrame and PluginsSettings.
- Server:
  - `workflow_engine.rs`: recovery and settle writes, and the `http_request` node.
  - `modules.rs`: the review diff and prompt materialisation, and the retry path.
  - `otto-git` provider error mapping: rate-limit classification with Retry-After, and one retry on reads. Sound.
  - `otto-dbviewer` `ApiErr` mapping, which passes the driver message through. Sound.
  - Channels: Slack and Telegram reconnect backoff with cancel. Sound, no retry storm.
  - Plugin supervisor: no auto-restart, so no storm, but see 2-04.
  - `otto-mcp` client retry.
  - Scheduled-tasks scheduler and engine settle.
  - Personal-agents scheduler and engine settle (sound; used as the reference for 2-03).
  - Broadcast `Lagged` handling in `ws_events.rs` and `otto-sessions/ws.rs`. Sound.
- Tauri shell: `supervisor.rs` and an unwrap/expect sweep. All hits are tests or checked invariants.

## Areas NOT covered yet (next iteration)

- `otto-swarm` runtime engine (206 lines changed in the fix commit): turn-runner failures and how a failed verifier is surfaced.
- `goal_loop.rs`: many `let _ = ctx.goal_loops_repo().set_*` writes (lines 630-782). Check whether a failed ledger or plan write leaves the loop UI stale.
- `otto-sessions/ws.rs` input queue (new in the fix commit): the error text when queued input is dropped.
- `routes/projects.rs` and `routes/share.rs` (changed in the fix commit): error responses.
- `apps/desktop` window and pane commands (`panes.rs`, `popout.rs`, `host_rooms.rs`): failures surfaced to the SPA.
- Workflow run UI (`RunSteps`, `RunAgents`): how a node error reaches the run view.
- e2e coverage of error states: whether Retry is exercised.
- The `/fs/read` final-output panel in WorkflowsPage hides on any error. Its access policy for non-root users was not traced.

---

## Findings

### error-handling-2-01: Rewrite and Test cases pollers still swallow every failure and then blame the agent (minor; carry-over of 1-01)

**Files:**
- `ui/src/modules/product/RewriteTab.svelte:55-72`
- `ui/src/modules/product/TestCasesTab.svelte:186-204`

**Evidence:**
- `catch (e) { console.error('[RewriteTab] poll error', e); }` and `catch (e) { console.error('[TestCasesTab] poll error', e); }`.
- `product.loadVersions()` and `loadTestcases()` rethrow (`product.svelte.ts:640-650, 1219-1231`, `try/finally` only). With the daemon down or returning 403/404 for 2 minutes, the user gets `'Rewrite timed out', 'No suggested version appeared within 2 minutes.'` or `'Generate timed out', 'No new test run appeared…'`.
- Because the run never throws or returns `false`, `pollWhileVisible` (`lib/poll.ts:117-121`) never backs off. The poll keeps its base cadence through the outage.
- PlanTab and AnalysisTab got the fix in iteration 1; these two did not.

**Fix:**
- Copy the PlanTab pattern: `pollFailures++`, `lastPollError = loadErrorText(e)`, an inline "Couldn't check progress: <cause>" with Retry after two failures, and the timeout toast `Couldn't check for the rewrite` with the cause when the last reads failed.
- Return `false` from the run on failure so the poller backs off.

### error-handling-2-02: Core stores and pages still show raw exception text ("Load failed") instead of `loadErrorText` (major; carry-over and extension of 1-02)

**Files:**
- `ui/src/lib/stores/apiClient.svelte.ts:334-336`. Local `errMsg` is raw, at 41 call sites (`'Couldn't load cookies'`, run history, collections…). FIXES.md lists it as deferred.
- `ui/src/lib/stores/git.svelte.ts:399` (`reposError`) and `:440` (`prError`).
- `ui/src/lib/stores/transcript.svelte.ts:285, 310`: the conversation read error.
- `ui/src/lib/stores/k8s.svelte.ts:277, 304, 545, 639` (status, clusters, namespaces and rows errors) and `ui/src/lib/stores/aws.svelte.ts:158-162, 208` (status, accounts).
- `ui/src/modules/vault/vault.svelte.ts:1366` (`msg`, used by tree, save, rescan and OKF toasts), `ui/src/modules/design-hall/library.svelte.ts:15` (`errText` → `this.error`), and `ui/src/modules/settings/PluginsSettings.svelte:54`.

**Evidence:**
- Each uses `e instanceof Error ? e.message : String(e)`.
- The transport never maps a network failure. `laneFetch` (`client.ts:317-331`) rethrows the WebKit `TypeError("Load failed")`, so with the daemon restarting the Git repos pane, transcript, Kubernetes and AWS views, vault autosave toast and design library all read "Load failed".
- A 401 shows the server's raw text instead of "Your session expired. Sign in again, then retry."
- `loadError.ts` documents this as the one sanctioned way to describe a failed load ("never the raw exception").

**Fix:**
- Replace each local helper with `loadErrorText`. It still passes 4xx/5xx Problem messages through, so kubectl, AWS and SQL text is kept.
- Keep the k8s `CLUSTER_SCOPE_HINT` prefix logic on top of it.
- Then add a lint (ui-guards) that flags `e instanceof Error ? e.message : String(e)` under `ui/src/lib/stores` and `ui/src/modules`.

### error-handling-2-03: A failed scheduled-task cursor write is discarded, so the same occurrence re-fires every 60 s (minor)

**Files:**
- `crates/otto-automation/src/scheduled_tasks_engine.rs:525-531, 581-585`
- `crates/otto-automation/src/scheduled_tasks_scheduler.rs:111-129`

**Evidence:**
- On both the success and the failure paths: `if settles_schedule(trigger, …) { let _ = settle_schedule(repo, task, status, served).await; }`. The `Result` is dropped with no log.
- The scheduler advances nothing itself. The comment at `scheduled_tasks_scheduler.rs:111-114` says the engine "advances it only on completion". `is_due_since` reads `last_run_at`, so when the `UPDATE` fails (for example SQLite busy or a disk error), the in-flight guard drops when the spawned task ends and the next `SCAN` (60 s, `:46`) fires the same occurrence again. That repeats every minute, re-running the agent or shell and re-delivering the report to Slack, Telegram or email, until a write succeeds.
- The sibling engine handles exactly this case. `personal_agents_engine.rs:1126-1155` says "`Err` is a failed write. Retry it before the in-flight guard drops, or the next tick launches the same occurrence again", and retries 5× with backoff, then holds the claim across one scan.

**Fix:**
- Port the personal-agents settle loop (retry with backoff, `warn!` per attempt, hold the in-flight guard across one scan on persistent failure) to both scheduled-task call sites.
- Also log the `set_last_status` failure on the manual-run branch.

### error-handling-2-04: A crashed or failed-to-start plugin sidecar is invisible: Settings says "Enabled" and nothing restarts it (minor)

**Files:**
- `crates/otto-server/src/plugins.rs:162-176` (boot `start_enabled`)
- `crates/otto-server/src/plugins.rs:249-276` (`adopt` watcher)
- `crates/otto-server/src/plugins.rs:553-560` (`admin_list`)
- `ui/src/modules/settings/PluginsSettings.svelte:180`

**Evidence:**
- A boot spawn failure is only `tracing::warn!(… "plugins: spawn failed: {e}")`. An unexpected exit only logs "down until re-enabled or the daemon restarts" and un-publishes the port.
- `admin_list` returns the DB `PluginRecord`, which has no running or last-error field. The settings row renders `StatusBadge tone='success' label='Enabled'` from `p.enabled` alone.
- The plugin page still loads, because `/plugins/{slug}/ui/*` is served statically from `ui_dir` (`:374-383`), so `PluginFrame`'s probe says `ok`. Every API call the plugin makes then gets a bare 502 `"plugin not running"` (`:424`).
- The frame's "Restart {name}" button (`PluginFrame.svelte:166-170`) only re-probes and reloads the iframe. It does not respawn the sidecar.
- Recovery needs a guess: Disable then Enable, or a daemon restart.

**Fix:**
- Add `running: bool` and `last_error: Option<String>` (from the spawn error or exit status) to the admin list.
- In Settings, render "Enabled · not running — <cause>" with a "Start again" action that calls enable.
- In `PluginFrame`, probe a sidecar-backed path, or the new running flag, so a dead sidecar shows an inline "This plugin's service stopped" with Start again instead of a page whose calls all 502.

### error-handling-2-05: A failed tool probe is shown as "not installed" (mongosh, Blender) (minor)

**Files:**
- `ui/src/modules/database/QueryEditor.svelte:238-244` (rendered at `:1446-1449`)
- `ui/src/modules/product/design/DesignArena.svelte:752-755` (rendered at `:1072-1073`)

**Evidence:**
- QueryEditor: `catch { mongoshInfo = { available: false }; }`. A timeout, 403 or unreachable daemon renders "mongosh is not installed on the daemon's PATH — `brew install mongosh`, then retry."
- DesignArena: `.catch(() => (blender = { installed: false, path: null, version: null }))` renders "Blender isn't installed (set OTTO_BLENDER or install it in /Applications)". `blenderChecked = true` means it is never re-probed while the arena is open, and there is no Retry.
- Both tell the user to install software they may already have, and hide the real cause.

**Fix:** Keep a separate `probeError = loadErrorText(e)` state. Render "Couldn't check for mongosh/Blender: <cause>" with Retry, and reserve the install hint for a successful probe that says `available/installed: false`.

### error-handling-2-06: A review's diff file write failure is only logged, and the reviewers are launched against a missing file (minor)

**Files:**
- `crates/otto-server/src/modules.rs:2091-2094` (new run)
- `crates/otto-server/src/modules.rs:4490-4499` (agent retry)

**Evidence:**
- New run: `if let Err(e) = tokio::fs::write(&diff_path, &diff_text).await { tracing::warn!(… "could not write review diff file: {e}"); }`. Every agent prompt is then built with `Diff file — read it fully …: {diff_path_str}` (`:2255-2261`) and the agents are spawned.
- Retry: `if let Ok(Some(diff)) = ctx.reviews_store.get_diff(&review_id).await { …; let _ = std::fs::write(&diff_file, diff); }`. Both a `get_diff` error and a write error are discarded.
- On a full disk (ENOSPC, which this machine has hit), each reviewer burns a full agent turn on a path that does not exist. The review settles as a normal run with whatever the agents improvise, which is at best "couldn't read the diff" findings and at worst `[]`, read as a clean review. The user sees no error.

**Fix:**
- Treat the diff write as fatal for the run. Settle the review as `error` with "Couldn't write the diff for the reviewers: <cause>" before spawning any agent.
- On retry, return a 409/500 Problem when the diff can't be re-materialised, instead of launching the agent.

### error-handling-2-07: Design library story chips memoize failed lookups for the life of the app (nit)

**File:** `ui/src/modules/design-hall/library.svelte.ts:213-252`

**Evidence:**
- `this.tried.add(\`ws:${wsId}\`)` and `this.tried.add(id)` are set before the request, and the `catch { /* no product access */ }` blocks never remove them.
- A transient failure, such as a daemon restart while the library loads, is treated like "no access". Story chips stay unlabeled until a full reload, even after every later `load()`. This is the same shape as 1-15, which was fixed in ConversationView.

**Fix:** In each catch, delete the key unless the error is a 403 or 404 (`isStatusError`). Clear `tried` on the events resync hook.
