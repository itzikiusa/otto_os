# Error handling: quality iteration 5 (2026-10-22)

Lens: resilience and error UX. This covers swallowed errors, raw exceptions shown instead of an inline Retry, missing recovery after a daemon restart or network drop, retry storms, misleading success or status messages, unhelpful error text and crash paths.

Branch `fix/quality-iter5-20261022` at `8177c4d85`. The review was read-only (grep, sed and git only; no builds or tests).

## Score

Rubric: start at 10.0 and subtract per open finding (blocker 1.0, major 0.3, minor 0.1, nit 0.03).

- 1 major × 0.3 = 0.30
- 8 minor × 0.1 = 0.80
- 1 nit × 0.03 = 0.03
- Total deductions: 1.13

**Score: 10.0 − 1.13 = 8.87 / 10** (iteration 4: 9.41; iteration 3: 9.31; iteration 2: 9.17; iteration 1: 8.17)

All eight iteration-4 findings are fixed. The score drops because this pass covered the areas iteration 4 listed as not covered: the desktop shell supervisor, the review summarizer fallback, the report writers, the budget service, user MCP server delivery, the room games transport and the remaining raw-error baseline. Most of the new findings are in those areas, not in code changed since iteration 4.

## Previous findings (iteration 4)

| ID | Status | Evidence |
|----|--------|----------|
| 4-01 Raw exception on core surfaces | Fixed | `graph-commit-diff.svelte:360` and `DiffViewer.svelte:861, 1018` use `loadErrorText(e)`, as do `notifications.svelte.ts:333` and `FloatingBar.svelte:316`. `DiffViewer.svelte:1005` keeps the raw `msg` only for the 400/409 classification. A sibling git surface (`WipPanel`) still has the old pattern; see 5-03. |
| 4-02 AWS views show "Load failed" | Fixed for the three named files; siblings open (5-02) | `aws/loadText.ts` adds `awsLoadText` (`isLoginRequired` ? raw : `providerErrorText`). It is used at 13 sites in `S3Browser`, `LogsView` and `AthenaView`. `RdsView`, `EksView`, `Ec2View`, `SqsView` and `MetricsPanel` still store the raw message. 4-02 named these siblings in its evidence. |
| 4-03 Convert-to-workflow reports success | Fixed | `routes/scheduled_tasks.rs:156-188`: a trigger error returns `ApiError` after `discard_converted_workflow`. The task is paused only after the trigger exists, and that write's error rolls back too. The response always carries `trigger_id`. |
| 4-04 Failed editmeta leaves fields read-only with a toast | Fixed | `issueEditorState.svelte.ts:115, 279-285` keeps `editmetaError`. `OverviewTab.svelte:302` gates the effect on it, and `:1627-1631` renders a LoadState with Retry. |
| 4-05 Failed restart reads "released to free memory" | Fixed | `manager.rs:6166-6193`: `respawn_failed` emits the status only after `update_status` succeeds, then stamps `SUSPEND_REASON_RESPAWN_FAILED` with the cause (`status.ts:331-338`). |
| 4-06 Swarm utilization swallows busy read, announces unsaved moves | Fixed | `scheduler.rs:343` `busy_agents(...).await?`. `:361-379`: a failed `update_task` is warned about and skipped, so only saved moves are counted and posted. |
| 4-07 `k8s_logs` broken body read as "no logs" | Fixed | `exec.rs:2120-2125` maps a 2xx `text()` error to `Error::Upstream("{status}: reading the response failed: …")`. |
| 4-08 Bulk archive/delete drop the cause | Fixed | `workspace.svelte.ts:1697, 1720` collect per-session causes (FIXES: new test `unit/workspaceBulkFailures.test.ts`). |

## Areas covered this iteration

- Re-verified all 8 iteration-4 findings against the current code.
- **Desktop shell** (`apps/desktop/src-tauri/src/supervisor.rs`, `main.rs`): `ensure_daemon` and its recovery plan (`Healthy` / `WaitForBoot` / `Install`), `install_binary`, the bootstrap retry, the `otto://daemon-state` event and the `daemon_*` commands. Also traced what the SPA shows offline (`App.svelte:63-143`, `TrayPage.svelte:282-286`).
- **k8s monitor**: the supervisor (`k8s_monitor_scheduler.rs`) and the collector loop (`otto-k8s/src/monitor/collector.rs:1120-1287`). It has failure backoff up to `MAX_BACKOFF`, schema-init backoff, a status row with `last_error`, a re-read baseline after a failed status write, and a reaped loop is restarted on the next scan. Sound.
- **otto-assistant personal agents**: the scheduler tick, the per-agent claim, the proactive budget (fails closed), `run_agent_after_supervisor`, `complete_agent_run`, `record_finish` (3 retries, then a minimal error row), `advance_cursor` (5 retries, then holds the claim for one scan) and `agent_notice`. Sound, except the report write (5-06).
- **otto-product runners**: `retry_analysis_agent` outcomes, the orphan reaper (`run.rs:1336-1480`), and `set_agent_status` merge semantics (`otto-state/src/product.rs:1155`). A stale error is kept on a re-run agent but only rendered while `status === 'error'` (`AnalysisTab.svelte:624, 860`), so there is no user-visible defect.
- **otto-review summarizer**: the managed attempt, the deterministic floor, the zero-comments guard, the agent note and the UI badges (5-05, 5-10).
- **otto-browser**: the Lightpanda sidecar supervisor (`lightpanda.rs:232-292`). It restarts with backoff up to 30 s, kills a sidecar that never becomes ready, and shutdown aborts the supervisor before taking the child lock. Sound. Also `RemoteLiveView` error classification (`:178-195, 435-437`).
- **otto-channels Slack** socket-mode listener: backoff, watchdog and cancel. Sound.
- **Scheduled tasks engine**: report write on success and failure (5-06), and `submit_prompt` callers (the bridge replies `DELIVERY_FAILED_REPLY`; scheduled tasks rely on the result watcher). Sound.
- **Budget service** (`otto-server/src/budget/mod.rs`, `routes/usage.rs:468-494`) (5-07).
- **User MCP server delivery** (`otto-sessions/src/mcp.rs` launchers and codex args; `routes/mcp_servers.rs` validation; `McpServers.svelte`) (5-08). MCP profiles fail open when the profile can't be resolved (`mcp_tools/profile.rs`). That is documented and only narrows a surface governance still checks, so it is not a finding.
- **Confluence read-back verification** (`otto-issues/src/confluence/verify.rs`): a failed read-back is reported as a failed `read_back` check, never as success. Sound.
- **Room games** transport (`rooms/games/client.ts`, `GameMatch.svelte`, the Kart and Shooter pause overlays) (5-09).
- **Classroom home box** (`home/boxes/ClassroomsBox.svelte`): LoadState for the first load and for a stale refresh, a scene LoadState with Retry, and abort-aware loading. Sound.
- **New-session sheet and MCP profile picker**: LoadState with Retry, and a hint that the session gets the default profile until profiles load. Sound.
- **Remaining raw-error baseline**: walked every `raw-error` entry in `ui/scripts/ui-guards-baseline.json` and separated load failures (5-02, 5-03, 5-04) from action failures where the server's text is the useful part.

## Areas NOT covered yet (next iteration)

- `otto-insights` scheduler: catch-up runs after a long sleep, and a failed headless run's report row.
- `otto-workflows` chat and event triggers: what a trigger that fails to start a run leaves behind (only the executor was covered before).
- `otto-vault` index rebuild and file-watcher errors (FTS rebuild failure, a vault folder removed on disk).
- `otto-design` / `otto-design-assist` assist turns: a failed agent turn, variant generation errors, and the Design Hall raw-error sites (`site/PublishModal`, `studio3d/*`, `assist/run.ts`).
- `otto-connections` SSH session drop mid-terminal, and `SessionNetworkStatus` (2 baselined raw errors).
- `otto-usage` embedded ClickHouse crash and restart while the Usage page is open (only the tailer was covered).
- Workbench (`modules/workbench/*`) preview and send-to errors (baselined).
- The `ottod` single-instance lock: what a second launch shows when the lock is held by a wedged process.

---

## Findings

### error-handling-5-01: The desktop app never surfaces the supervisor's daemon diagnosis or its Restart command; a wedged daemon can't be recovered from the app (major)

**Files:**
- `apps/desktop/src-tauri/src/main.rs:132-135` (commands registered) and `:197-206` (boot emits `otto://daemon-state`)
- `apps/desktop/src-tauri/src/supervisor.rs:499-504` (`not_answering_detail`), `:530-572` (`WaitForBoot`), `:385-417` (install errors), `:616-660` (`daemon_start` / `daemon_restart` / `daemon_install`)
- `ui/src/App.svelte:63-69, 131-143` (offline screen) and `ui/src/modules/desktop/TrayPage.svelte:282-286`

**Evidence:**
- At launch the shell runs `ensure_daemon` and emits `otto://daemon-state` with a precise `detail`. Examples: "daemon loaded but not answering after 40 s — use Restart daemon to replace a wedged process; see ~/Library/Logs/Otto/ottod.log", "install failed: launchctl bootstrap failed after 20s: … (recover with: launchctl bootstrap gui/501 …)", and "launchctl reported success but com.otto.daemon is not loaded".
- No UI code consumes this event. `grep -rn "daemon-state\|daemon_restart\|daemon_start\|daemon_install" ui/src` finds nothing. The only `otto://` listeners are the menu, bar, tray, browser and pane events (`lib/desktop.ts:164-167`, `menu.ts:97`, `nativeBrowser.ts`, `nativePane.ts`).
- The `daemon_restart` command (`kickstart -k`, or a full reinstall when the job isn't loaded) has no caller anywhere. There is also no native "Restart daemon" menu item (`main.rs:285-400`).
- The offline screen offers only "Retry now", which re-probes `/health` (`auth.boot(true)`). Its text says "Check the daemon log … then retry. Otto keeps trying in the background." The tray says "The Otto daemon isn’t running yet." with a Retry that also only re-probes.
- So the case the supervisor diagnoses best has no in-app recovery: a live but wedged ottod (loaded, not answering).
  - The detail text tells the user to "use Restart daemon", but that control does not exist.
  - Quitting and reopening Otto doesn't help either. `recovery_plan` returns `WaitForBoot` for a loaded, silent daemon (`supervisor.rs:183-190`), and that path deliberately kickstarts without `-k` (`:550-557`), so the wedged process survives every relaunch.
  - The only way out is `launchctl kickstart -k` in a terminal.
- An install failure (bootstrap refused) likewise shows only the generic 15 s "can’t reach the daemon" line. The concrete `launchctl` recovery command is computed and then dropped.

**Fix:**
- In `App.svelte` (desktop only), `listenNative('otto://daemon-state', …)` and keep the last `DaemonReport`. While `auth.phase === 'offline'`, show `report.detail` under the boot line.
- Add a "Restart daemon" button that invokes `daemon_restart` and then resumes the 2 s health poll. Add "Reinstall daemon" (`daemon_install`) when the report says `install failed`.
- On mount, call `daemon_status`/`daemon_start` so a window opened after the boot event still gets the report.
- Mirror the Restart action in the tray's offline state.
- Change `not_answering_detail` to name the button that now exists.

### error-handling-5-02: Five more AWS views still show "Load failed" and raw daemon 401/404 text (minor; carry-over of 4-02)

**Files:**
- `ui/src/modules/aws/RdsView.svelte:75, 122` (rendered `:176` via `awsErrorText(error)`, and `:264` `<p class="err">{d.error}</p>`)
- `ui/src/modules/aws/EksView.svelte:72, 96` (`:159`, `:204`)
- `ui/src/modules/aws/Ec2View.svelte:87, 172` (`:238`, `:332`)
- `ui/src/modules/aws/SqsView.svelte:110` (`:376`)
- `ui/src/modules/aws/MetricsPanel.svelte:75` (`:154`)

**Evidence:**
- Each site stores `e instanceof Error ? e.message : String(e)`.
- The list error goes through `awsErrorText`, which only rewrites the boto `An error occurred (Code) …` shape. The detail errors are printed raw, without even `awsErrorText`.
- So with the daemon restarting, the RDS, EKS, EC2, SQS and metrics panes read "Couldn’t list DB instances — Load failed", and an expired daemon session shows the server's raw 401 text instead of "Your session expired. Sign in again, then retry."
- The helper that fixes this, `awsLoadText`, already exists (`aws/loadText.ts`) and was applied only to the three files 4-02 used as examples. 4-02's evidence named `RdsView`, `EksView` and `Ec2View` explicitly.

**Fix:**
- Replace each assignment with `awsLoadText(e)`. It keeps the "login required" text that `isLoginRequired(new Error(error))` needs.
- Render the detail errors through `awsErrorText(d.error)`.
- Lower the `raw-error` baseline entries for these files.

### error-handling-5-03: The git WIP panel's file diff shows "Couldn’t load the diff: Load failed" with no Retry (minor)

**File:** `ui/src/modules/git/WipPanel.svelte:228-232` (store), `:1058-1059` (render), `:247-260` (live refetch)

**Evidence:**
- The commit panel's diff load stores `diffError = e instanceof Error ? e.message : String(e)` and renders `<div class="dim wp-empty">Couldn’t load the diff: {diffError}</div>`.
- There is no Retry. The live refetch path swallows its own errors (`.catch(() => { /* keep the diff on screen; the next change or a reselect retries */ })`), and it never clears a standing `diffError` unless it succeeds.
- So a daemon restart while a file is selected leaves "Couldn’t load the diff: Load failed" on the panel the user commits from. The only recovery is to click another file and back.
- This is the sibling of the `graph-commit-diff.svelte` site fixed under 4-01. It breaks both rules in `lib/loadError.ts`: the cause is raw, and there is no inline Retry.

**Fix:**
- Store `loadErrorText(e)`.
- Render the failure through `LoadState variant="compact" what="this file’s diff"` with `onretry` re-running the select effect (bump a retry counter that the effect reads).

### error-handling-5-04: Remaining raw-exception load errors on the git focus view, network profiles, DB multi-run, product publish, canvas panel, access groups and k8s views (minor)

**Files (load failures only; action failures that quote the server's refusal were left alone):**
- `ui/src/modules/git/FocusView.svelte:118, 136, 252`. These are rendered through `LoadState` at `:415, 424` and inline at `:488`. Issue-tracker accounts, "your Jira work" and the quick issue lookup.
- `ui/src/modules/connections/NetworkProfiles.svelte:43, 56`: the SSH connections list (LoadState `:85`) and the saved-profile reload.
- `ui/src/modules/database/MultiRunDialog.svelte:98, 198, 318`: schema scopes, the plan and item detail (`:386, 534, 711`).
- `ui/src/modules/product/PublishDialog.svelte:126, 195, 237`: "Couldn’t load the content preview. {raw}", Jira projects and Confluence spaces.
- `ui/src/modules/panels/CanvasPanel.svelte:101, 128`: scene previews.
- `ui/src/modules/settings/AccessGroups.svelte:131`: "Couldn’t load the members of X. {raw}".
- `ui/src/modules/kubernetes/LogsView.svelte:184`, `kubernetes/MetricsView.svelte:49`.

**Evidence:**
- Each site stores `e instanceof Error ? e.message : String(e)` for a GET or read.
- During a daemon restart or a network drop, WebKit rejects with `TypeError("Load failed")`, so these surfaces read "Couldn’t load your Jira work. Load failed" and "Couldn’t load the content preview. Load failed".
- A daemon 401 shows the raw server text instead of the sign-in line.
- `lib/loadError.ts` exists for this exact case. The Jira, Confluence and kubectl surfaces should use `providerErrorText`, which keeps a provider 403's actionable text.
- All of these are still in the `raw-error` baseline.

**Fix:**
- Use `loadErrorText(e)` for the daemon-only loads: NetworkProfiles, MultiRunDialog, CanvasPanel and AccessGroups.
- Use `providerErrorText(e)` for the provider-backed loads: FocusView, PublishDialog and the k8s views.
- Run `node scripts/ui-guards.mjs --update-baseline`.

### error-handling-5-05: The review summarizer's failure cause is dropped, and the UI blames "claude" whatever the provider and whatever happened (minor)

**Files:**
- `crates/otto-server/src/modules/review/summarize.rs:228-250` (cause logged only), `:252-266` (note)
- `ui/src/modules/git/ReviewPanel.svelte:1261-1266`, `ReviewAgents.svelte:151-154`, `LocalReviewPanel.svelte:464, 562-563`

**Evidence:**
- When the managed summarizer fails, the error `e` goes only to `tracing::warn!("summarizer failed: {e}; deterministic fallback")`. It might be a provider not logged in, a timeout, a spawn failure or an invalid result file.
- The stored agent note is just "deterministic fallback — N final comments". No field on the review or the agent row carries the cause.
- The second fallback path fires when the summarizer ran fine but returned 0 comments for N findings (`:244-250`). It sets the same flag.
- Every UI surface then says "deterministic fallback (claude unavailable)" / "The claude summarizer was unavailable". That text is wrong in two ways:
  - The summarizer provider is configurable (`Attempt::new(provider, …)`; codex and agy are valid).
  - In the zero-comments case the summarizer was available.
- A user whose summarizer always falls back (expired login, wrong model id) gets no hint why, and the "Retry summary" button just repeats the failure.

**Fix:**
- Store the cause on the summarizer row (`agent_states[summarizer_idx].error`, or a `fallback_reason`): the clipped `e` for a failed run, or "returned no comments for N findings" for the guard.
- Render it in the badge's tooltip and in the note line.
- Name the row's actual provider instead of the hard-coded "claude".

### error-handling-5-06: A run whose report couldn't be written is recorded as a clean success with no report and no reason (minor)

**Files:**
- `crates/otto-assistant/src/personal_agents_engine.rs:962-968, 997-1018`
- `crates/otto-automation/src/scheduled_tasks_engine.rs:473-479` (success path) and `:559-568` (failed-run report)
- UI: `ui/src/modules/personal-agents/AgentPage.svelte:642-668`, `scheduled-tasks/ScheduledTasksPage.svelte:1313`

**Evidence:**
- On a successful run, `write_report(&abs, &out.report)` failing (disk full, permissions, a removed data dir) is only `warn!`ed. The run is then finished with `status: "ok"`, `report_path: None` and `report_rel: None`.
- `FinishAgentRun` and the scheduled-task finish carry no report body, so the agent's Markdown report is lost.
- The Runs list shows a green OK row with the summary and no "View report" button (`AgentPage.svelte:653` gates it on `r.report_rel`). Nothing says the report wasn't saved. `agent_notice` sees no error, so it also clears the failure streak.
- For a failed scheduled run, the report that "shows why it failed" (`:552-554`) is dropped the same way. The summary falls back to `String::new()`, so the row shows only the generic error.

**Fix:**
- On a write failure, keep the run's status and set a visible note. Either add `report_error: "The report couldn’t be saved: {e}"`, or append it to `delivery_error`/`error` and set a `report_unsaved` flag.
- Render it next to the row as the delivery error is.
- Raise the same failure notice (`agent_notice`, `notify_run_failure`) so an unattended agent losing every report is noticed.

### error-handling-5-07: The budget service turns a failed config read into "no budgets", both on the page and in the admission gate (minor)

**Files:**
- `crates/otto-server/src/budget/mod.rs:29-43` (`stored_config` / `config`), `:97-108` (`budget_status`), `:214-218` (`check_with`)
- `crates/otto-server/src/routes/usage.rs:471-479`

**Evidence:**
- `stored_config` does `SettingsRepo::get(BUDGETS_KEY).await.ok().flatten().and_then(|v| serde_json::from_value(v).ok())`, and `config()` falls back to `UsageBudgetConfig::default()` (enforcement off, no caps). So a SQLite read error or a stored value that no longer parses gives three outcomes:
  - `GET /usage/budgets` answers 200 with an empty config. The page shows no budgets instead of an error.
  - A root user who then edits and saves from that state overwrites the real stored caps (`put_budgets` replaces the whole config).
  - `check_with` returns `BudgetVerdict::default()` (admit). Every gated surface fails OPEN: goal loops, reviews, swarm turns, product runs and the workflow `budget_gate` node.
- The same function deliberately fails closed when spend is unavailable (`:241-250`, "Budget spend unavailable; retry after usage accounting recovers"). The config read is the inconsistent half.
- The foreground status (`budget_status`) also maps an engine error to `unwrap_or_default()` totals. The page then shows "$0.00 of $X" as if nothing were spent, with nothing marking the number as unknown.

**Fix:**
- Make `stored_config` return `Result<Option<_>>`. `GET` maps an error to `ApiError` (the page shows its LoadState and Retry, and offers no editable empty form).
- `check_with` treats a read or parse error like unavailable spend (blocked when `block_on_exceed`, with a reason).
- Add `spend_unavailable: bool` to `UsageBudgetStatus` and render "Spend unavailable" instead of $0.

### error-handling-5-08: A user MCP server is silently left out of sessions when its name or env keys don't fit the launcher, and the form accepts both (minor)

**Files:**
- `crates/otto-sessions/src/mcp.rs:261-269, 367-385` (JSON launchers and grok, `:901-907`), `:700-737` (codex)
- `crates/otto-server/src/routes/mcp_servers.rs:231-236, 304-311` (create/update validation)
- `ui/src/modules/settings/McpServers.svelte:486`

**Evidence:**
- At spawn, these cases are only `tracing::warn!`ed and the server is not attached:
  - An env key that isn't a shell variable name (for example `LINEAR-API-KEY`) is dropped for every provider ("env key is not a shell variable name; server not attached").
  - A failed env-file write drops the server the same way.
  - For codex sessions, a server name that isn't a bare TOML key is also dropped ("not attached to this codex session"). That covers any name with a space or a dot, such as "Linear MCP".
- The routes validate only "name/command must not be empty", and the form has no pattern check. So a user can save an enabled server that can never be delivered.
- The MCP page and the settings row show it as On. Every new session starts without its tools, and the only trace is a daemon log line. The user sees an agent that "doesn't have" the tool, with nothing pointing at the cause.

**Fix:**
- Validate at save time: env keys `^[A-Za-z_][A-Za-z0-9_]*$`, and names `^[A-Za-z0-9_-]+$`, or quote non-bare names in the codex TOML instead. Return 400 with the rule, and mirror the check inline in `McpServers.svelte`.
- For the remaining runtime failure (the env-file write), record it on the session (`meta.mcp_skipped: [{server, reason}]`) and show it in the session's MCP menu.

### error-handling-5-09: A game room stops reconnecting after five attempts but keeps saying "Reconnecting…" / "paused while someone reconnects" (minor)

**Files:**
- `ui/src/modules/rooms/games/client.ts:47-52`
- `ui/src/modules/rooms/games/GameMatch.svelte:117-123, 138-145, 153`
- `kart/KartGame.svelte:407`, `shooter/ShooterGame.svelte:617`

**Evidence:**
- `onclose` reschedules `connect()` only `if (this.attempts < 5)`, with delays of 1, 2, 4, 5 and 5 s. Each attempt can also take up to 12 s to time out.
- After the fifth failure the client stops for good. It has no `online` or `visibilitychange` handler, and the status stays `'disconnected'`.
- `GameMatch` keeps rendering "Connection lost. Reconnecting…" in the lobby. In a match, it passes `paused={connection !== 'connected'}`, so the overlay reads "Waiting for players — The race is paused while someone reconnects."
- Neither view has a Reconnect action. A laptop that slept or a Wi-Fi hand-off of about 30 s therefore leaves the player on a permanent "reconnecting" screen. The host's match also stays paused for everyone.

**Fix:**
- Expose a `'gave-up'` (or `'offline'`) status when the attempts run out.
- Render "Couldn’t reconnect to the game room." with a "Reconnect" button that resets `attempts` and calls `connect()`.
- Also retry on `window` `online` and when the page becomes visible.

### error-handling-5-10: A summarizer re-run that can't read the review's kept comments re-drafts approved and posted ones (nit)

**File:** `crates/otto-server/src/modules/review/summarize.rs:288-298, 318-327`

**Evidence:**
- `kept_comments` is `ctx.reviews_store.get_review(review_id).await.map(…).unwrap_or_default()`.
- A failed read (SQLite busy) is treated as "nothing kept", so every summarized comment matching an approved or posted one is inserted again as a fresh draft.
- The comment just above names the consequence: "approving that copy posted a duplicate".

**Fix:** Propagate the error with `?` (the run is then marked failed and can be retried), or skip storing drafts this run, rather than treating the read failure as an empty list.
