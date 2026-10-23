# Error handling: quality iteration 6, wave 0 (2026-10-23)

Lens: resilience and error UX. This covers swallowed errors, raw exceptions shown instead of an inline Retry, missing recovery after a daemon restart or network drop, retry storms, misleading success or status messages, unhelpful error text and crash paths.

Branch `fix/quality-iter6-20261023` at `02a64019b`. The review was read-only (grep, sed and git only; no builds or tests).

## Score

Rubric: start at 10.0 and subtract per open finding (blocker 1.0, major 0.3, minor 0.1, nit 0.03).

- 0 blocker, 0 major
- 5 minor × 0.1 = 0.50
- 2 nit × 0.03 = 0.06
- Total deductions: 0.56

**Score: 10.0 − 0.56 = 9.44 / 10** (iteration 5: 8.87; iteration 4: 9.41; iteration 3: 9.31; iteration 2: 9.17; iteration 1: 8.17)

All ten iteration-5 findings are fixed or reduced to a residual. The only major (5-01, the wedged daemon with no in-app recovery) is fixed. The new findings come from the areas iteration 5 listed as not covered: Design Hall assist, Insights scheduling, workflow chat controls, and proof assembly. A residual raw-load-error pattern is also still open.

## Previous findings (iteration 5)

| ID | Status | Evidence |
|----|--------|----------|
| 5-01 Desktop never surfaces the supervisor's diagnosis or Restart (major) | Fixed | `ui/src/modules/desktop/daemonState.ts` listens to `otto://daemon-state` and wraps `daemon_status`/`daemon_start`/`daemon_restart`/`daemon_install`. `App.svelte:105-150` keeps the last report, shows `offlineDetail`, and offers Restart and Reinstall (`runDaemonAction`). `supervisor.rs:501-505` names the button that now exists. |
| 5-02 Five AWS views show "Load failed" (minor) | Fixed | `RdsView:77,124`, `EksView:74,98`, `Ec2View:89,174`, `SqsView:112` and `MetricsPanel:76` use `awsLoadText(e)`. The detail errors render through `awsErrorText(d.error)` (`RdsView:266`, `EksView:206`, `Ec2View:334`), and the lists through `LoadState` with Retry. |
| 5-03 WIP panel diff "Load failed" with no Retry (minor) | Fixed | `WipPanel.svelte:229` `diffError = loadErrorText(e)`; `:1057` `LoadState variant="compact" … onretry={() => diffRetry++}`. |
| 5-04 Raw load errors on FocusView, NetworkProfiles, MultiRunDialog, PublishDialog, CanvasPanel, AccessGroups, k8s (minor) | Fixed for the named sites; siblings remain (6-05) | The named load sites use `loadErrorText`/`providerErrorText` (`CanvasPanel:53,101,128`, `k8s LogsView:185`, `MetricsView:49,87`, …). What remains in those files are action failures (`NetworkProfiles:71` save, `AccessGroups:151` mutate, `MultiRunDialog:251` 409 classification), which are allowed. One load site in the same file was missed: `CanvasPanel.svelte:186` (scene list for "attach"); it is listed under 6-05. |
| 5-05 Summarizer cause dropped; UI blames "claude" (minor) | Fixed | `summarize.rs:260` `fallback_note(fallback_cause, n)`, with test `fallback_note_carries_the_cause` at `:507`. No `claude unavailable` text is left in `ui/src` (`RunAgents.svelte:241` only names the fix in a comment). |
| 5-06 Unwritable report recorded as a clean success (minor) | Fixed | `personal_agents_engine.rs:963` `saved_report(…)` records "The report couldn’t be saved: …" (`:1189`), and so does `scheduled_tasks_engine.rs:473,564` → `:809`. |
| 5-07 Budget config read failure reads as "no budgets" (minor) | Fixed | `budget/mod.rs:32-39` `read_config() -> Result<Option<_>>`. `try_config` propagates the error to admission and the route. `spend_unavailable` is set at `:133`. |
| 5-08 MCP server silently not attached (minor) | Mostly fixed; runtime residual open (6-06, nit) | Env keys are validated at save (`routes/mcp_servers.rs:240`). Codex gives a non-bare name a sanitized key (`mcp.rs:725-738`). Still log-only: a failed env-file write (`mcp.rs:384`, `:754`), a name with no free codex-safe key (`:731`), and a row saved before the validation existed (`:375`, `:744`). FIXES P09 lists the `mcp_skipped` meta as not done. |
| 5-09 Game room says "Reconnecting…" forever (minor) | Fixed | `lib/relaySocket.ts` emits `gave-up` and wakes on `online`/`visibilitychange`. `GameMatch.svelte:134,164,201-202` render "Couldn’t reconnect to the game room." with Reconnect. |
| 5-10 Summarizer re-run re-drafts kept comments (nit) | Fixed | `kept_comments` no longer uses `unwrap_or_default` (the only `unwrap_or_default` left in `summarize.rs`, at `:329`, is `c.reasoning`). |

## Areas covered this iteration

- Re-verified all ten iteration-5 findings against the current code (table above).
- **otto-insights scheduler** (`lib.rs:897-1125` `run_insights`, `:1406-1595` the scheduler and `PeriodAttempts`), the routes, and the UI (`InsightsPage`, `InsightsSettings`, `ReportDetail`). Findings: 6-03.
- **otto-workflows triggers**: the schedule trigger (`triggers/schedule.rs`) and the event-trigger listener (`triggers.rs:300-450`) are sound. The schedule cursor is not advanced on a failed admission, so the tick retries, and the in-flight cap is atomic. Chat controls (`chat.rs`) are covered under 6-04.
- **otto-vault** scan, watcher and FTS: a missing root or a failed scan is stored as `scan_state = "error: …"` (`engine.rs:713-725, 744-750`) and rendered as a clickable rescan chip (`VaultPage.svelte:250, 322`). A failed FTS write is re-prepared by the next probe. Sound.
- **otto-design-assist** turn lifecycle (`lib.rs:1040-1110` in-memory registry, `:1840-1960` outcome and `restore_current_head`), plus the Design Hall UI: `OttoPanel`, `Studio3D`, `model.ts mergeTurns`, `site/siteApi.ts`, `PublishModal`, `CompareModal`, `CollectionView`, `LearnedCard` and `Lobby`. Findings: 6-01 and 6-05.
- **otto-connections / SSH**: `SessionNetworkStatus` (polling with backoff and a Refresh button; raw text in 6-05), `NetworkProfilePicker` and the SFTP list store (`sftp.svelte.ts`, rendered via `LoadState`).
- **otto-usage**: the embedded ClickHouse self-heal (`clickhouse.rs` lazy restart and parked state) and the Usage page (`LoadState` with `loadErrorText` on the report, summary and budgets). Sound.
- **Workbench** previews and Send-to (`sendTo.ts`): action failures toast the server's text, the image preview has Retry, and parse errors are local. Sound.
- **ottod single-instance lock** (`profile_lock.rs`): a second daemon fails with a clear message. The wedged-holder case is now recoverable through 5-01's Restart. Sound.
- **Proof packs**: the `assemble` route and `proof.rs` `assemble_diff`/`run_command_artifact` (6-02).
- **otto-git** diff targets (`local.rs:2800-2920`): the `Working` fallback is covered under 6-07.
- **Swarm runtime dispatch** (`runtime/engine.rs:585-700`): the `busy_agents(...).unwrap_or_default()` at `:592` is harmless, because `reserve_run` refuses a busy agent atomically (`otto-state/src/swarm.rs:1658-1676`) and the claim is reverted. Not a finding.
- **Classroom** (`ClassroomsBox`, `school/actions.ts`, `school/screens.ts`): kick-out, detention and release all report failure and restore the kid, and a failed screen read keeps the last frame. Sound.
- **Room games transport** (`relaySocket.ts`, `games/client.ts`) after the 5-09 fix. Sound.
- **Remaining raw-error baseline**: walked all 78 `raw-error` entries in `ui/scripts/ui-guards-baseline.json` and separated load failures (6-05) from action or parse failures, where the message itself is the useful part.

## Areas NOT covered yet (next iteration)

- `otto-browser` live view and annotation persistence after a daemon restart. Only the Lightpanda supervisor and `RemoteLiveView` classification were covered in iteration 5.
- `otto-channels` Telegram long-poll loop (only Slack socket-mode was covered).
- `otto-improve` self-improvement runs: a failed edit apply or rollback, and what the Improvement page shows.
- `otto-product` story watcher failure surfacing, beyond the iteration-5 runner and reaper.
- API client (`modules/api/*`) gRPC and stream (`api_stream.rs`) reconnect behaviour.
- `otto-skills` / Skills Lab eval runs: a failed eval and its scoring row.
- Plugin supervisor (`otto-server/src/plugins.rs`): plugin crash loops and their UI.

---

## Findings

### error-handling-6-01: A Design Hall assist turn interrupted by a daemon restart stays "working" forever; Send stays disabled and Studio3D polls every 3 s for good (minor)

**Files:**
- `crates/otto-design-assist/src/lib.rs:1040-1105` (turns live only in the in-memory registry: `put_turn`, `update_turn`, `turns_for`)
- `ui/src/modules/design-hall/assist/model.ts:138-148` (`mergeTurns`), `:151-153` (`busyTurn`)
- `ui/src/modules/design-hall/assist/OttoPanel.svelte:130-134` (`loadTurns` → `mergeTurns`), `:186-190` (resync reload), `:257-259` (`busy`), `:623, :638` (quick asks and Send disabled while `busy`)
- `ui/src/modules/design-hall/studio3d/Studio3D.svelte:178-187` (`poll`)

**Evidence:**
- Assist turns are kept only in the process's registry (`with_registry`). After a daemon restart, `GET …/assist/turns` returns no record of a turn that was `running`.
- `mergeTurns` keeps every local turn missing from the fetched list (`for (const l of byId.values()) out.push(l)`). So the panel's local `running` turn survives the resync reload (`designAssistBus.resyncTick` → `loadAll`). `busyTurn` keeps returning it, `busy` stays true, and the quick-ask chips and Send stay disabled with the "working" state shown. The Stop control only interrupts the (now dead) session; it never ends the turn.
- `Studio3D.poll` treats `!t` (the turn is gone) the same as `running` and reschedules: `if (!t || t.status === 'starting' || t.status === 'running') { … schedulePoll(); return; }`. The 3 s poll and the "Otto is working" state continue until the view is left.
- The only recovery is to navigate away and back, because the component state is reset on mount. Nothing tells the user the turn was lost. The server's `restore_current_head` never ran for that turn either.

**Fix:**
- In `mergeTurns`, when the fetch is authoritative (a full resync or load, not the optimistic window right after `startAssist`), map a local non-terminal turn that the server no longer knows to `status: 'failed'` with `error: 'Otto restarted while this turn was running; the turn was lost. Ask again.'`.
- In `Studio3D.poll`, treat `!t` after the first successful poll as the same lost-turn failure: clear `agent` and show `toasts.error('Otto couldn’t finish', …)`.
- Optionally, have the daemon persist a minimal turn row, or report a `lost` status for turn ids it does not know (`GET …/turns/{id}` → 404), so the UI can tell "lost" from "not yet listed".

### error-handling-6-02: "Proof assembled" is shown even when nothing was assembled or the daemon asked for a retry (minor)

**Files:**
- `crates/otto-server/src/routes/proof.rs:637-643` (`assemble`: `let _ = engine::assemble_diff(…)` and `let _ = engine::run_command_artifact(…)`)
- `crates/otto-server/src/proof.rs:310-312, 329-331, 336-338` (silent `Ok(())` when the folder isn't a repo or the diff fails), `:339-343` (`Err(Conflict("repository HEAD changed while assembling proof; retry"))`), `:394-404` (the upsert `?`)
- `ui/src/modules/proof/ProofPage.svelte:436-438`

**Evidence:**
- The route discards both results. If HEAD moved during assembly, `assemble_diff` returns a Conflict whose text says "retry", and the route drops it. If storing a requested command's output fails (`upsert_content_artifact(…).await?`), the route drops that error too.
- `assemble_diff` itself returns `Ok(())` without an artifact for a folder that isn't a git repo, or when the diff fails ("skip silently").
- The route then answers 200 with the recomputed pack. The page always toasts `toasts.success('Proof assembled', 'Re-assembled from the working folder.')`.
- So a user who typed the wrong folder, or whose HEAD moved, is told that proof was re-assembled when no diff artifact was added. The gate then still reads as missing proof, with nothing pointing at the cause.

**Fix:**
- In the route, collect outcomes instead of `let _`. Return a Conflict from `assemble_diff` as the response error (the UI then shows "Couldn’t assemble proof — … retry"). Report a failed command upsert per command.
- Have `assemble_diff` return what it did (`Assembled | NotARepo | DiffFailed(String)`). Add an `assembled: { diff: …, commands: [...] }` summary to the response so `ProofPage` can say "No git repository at that folder — nothing to assemble" instead of a success.
- Update `docs/contracts/api.md` and `types.ts` for the new response field.

### error-handling-6-03: Scheduled Insights runs that fail leave no trace, and the empty page says schedules are off (minor)

**Files:**
- `crates/otto-insights/src/lib.rs:1415-1442` (`MAX_PERIOD_ATTEMPTS = 3`, in memory), `:1549` (`attempts.admit` before the spawn), `:1560-1570` (`Ok(None)` and `Err(e)` only logged), `:919-922` (no host workspace → `Ok(None)` + `warn!`)
- `ui/src/modules/insights/InsightsPage.svelte:785-796` (EmptyState body)

**Evidence:**
- A scheduled catch-up run that cannot start (no host workspace or member, provider resolution or session-create error) or that never lands a report (provider not signed in, agent failure) is only `warn!`ed.
- After three attempts for a period, `PeriodAttempts::admit` returns false and the scheduler leaves that period alone without a sound. No status file, event or notice records the failures. `Event::InsightReady` is sent only on success (`:1584-1590`).
- `InsightsSettings` warns about a missing skill, but nothing covers the other causes.
- `InsightsPage` never reads `/insights/config`. With no reports, its EmptyState says unconditionally: "Scheduled reports are off until you turn them on."
- So a user who turned on daily reports and whose runs fail sees a page that claims the schedule is off, with a button to turn on what is already on. There is no hint that three runs failed.

**Fix:**
- Persist the last scheduled attempt per cadence: `{period, attempts, last_error, gave_up}` in `<insights>/schedule-state.json`, written in the spawned task on `Err`/`Ok(None)` and when `wait_run_archived` returns with the period still not done. Serve it on `GET /insights/config` (or a new `/insights/schedule-status`).
- In `InsightsPage`, read it. Render "Scheduled daily reports are on, but the last 3 runs didn’t produce a report: {last_error}" with "Run now" and "Open the session". Only say "off" when the config says so.
- Optionally, raise the same failure notice as scheduled tasks once a period gives up.

### error-handling-6-04: The workflow chat "abort" command replies "Aborting run…" even when the cancel failed or the run had already finished (minor)

**File:** `crates/otto-workflows/src/chat.rs:1115-1140`

**Evidence:**
```rust
match repo.request_cancel(&run.id).await {
    Ok(None) => {}
    Ok(Some(rev)) => { /* emit WorkflowRunUpdated */ }
    Err(e) => tracing::warn!("workflow chat abort: cancel failed: {e}"),
}
Some(WorkflowChatAck {
    reply: format!("🛑 Aborting run `{short}` — stopping its agents."),
})
```
- `request_cancel` returns `Ok(None)` when the conditional status flip matched nothing, because the run reached a terminal state between `find_active_run_for_thread` and the cancel. It returns `Err` on a database error.
- In both cases the Slack/Telegram thread still gets "🛑 Aborting run — stopping its agents."
- On `Err` the run keeps going and keeps spending, while the person who aborted it from chat believes it is stopping. This is the outward-facing surface where they can't see Otto's UI.

**Fix:**
- `Ok(Some(_))` → keep the current reply.
- `Ok(None)` → "Run `{short}` already finished — nothing to abort."
- `Err(e)` → "⚠️ Couldn’t abort run `{short}`: {e}. Use Cancel in Otto." (clip `e`).
- Add a unit test with a fake repo for the three branches.

### error-handling-6-05: Remaining load failures still show the raw exception ("Load failed", raw 401/404 text) (minor; residual of 4-01/5-04)

**Files (GET/read failures only; action and parse failures that quote the useful message were left alone):**
- Design Hall:
  - `ui/src/modules/design-hall/CollectionView.svelte:98`, rendered through `LoadState` at `:308`
  - `design-hall/LearnedCard.svelte:39`, `LoadState` at `:84`
  - `design-hall/CompareModal.svelte:82`, rendered as "Couldn’t load this version. {raw}" at `:149`
  - `design-hall/assist/OttoPanel.svelte:149`, `LoadState` at `:523`
  - `design-hall/site/siteApi.ts:119`: "Couldn’t load the brand kit ({raw})." This is shown in `site/Inspector.svelte:614` with no Retry. The preview and the contrast audit (`SiteStudio.svelte:518`) silently fall back to the default palette.
- Vault: `vault/Switcher.svelte:37`, rendered as "Couldn’t search notes. {raw}" at `:101`.
- Connections:
  - `connections/NetworkProfilePicker.svelte:18` (`:34`)
  - `connections/SessionNetworkStatus.svelte:40`, a 5 s status poll; "Load failed" sits next to the stale "Connected" summary
  - `lib/stores/sftp.svelte.ts:98`, the folder listing (`SftpBrowser.svelte:372`)
- MCP: `mcp/OttoServerHome.svelte:282`, rendered as "Couldn’t load gateway tools: {raw}" at `:515` with no Retry.
- Agents:
  - `agents/conversation/PreviewBody.svelte:145`, the file preview
  - `agents/JiraIssuePicker.svelte:159`, a provider search ("Search failed: {raw}" at `:259`)
  - `lib/stores/transcript.svelte.ts:516`, the subagent transcript page
- Panels: `panels/CanvasPanel.svelte:186`, the "attach a scene" list (`LoadState` at `:345`). It was missed in the 5-04 fix of the same file.

**Evidence:**
- Each site stores `e instanceof Error ? e.message : String(e)` for a daemon GET.
- During a daemon restart or a network drop, WebKit rejects with `TypeError("Load failed")`, so these surfaces read "Couldn’t load these designs — Load failed" or "Couldn’t search notes. Load failed".
- An expired session shows the server's raw 401 body instead of "Your session expired. Sign in again, then retry."
- `lib/loadError.ts` documents this rule. `loadErrorText` keeps a plain `Error`'s message, so the custom messages thrown in `transcript.svelte.ts` survive a drop-in swap.
- All of these files are still in the `raw-error` baseline.
- Two surfaces also lack a Retry: `OttoServerHome` gateway tools and the Site Studio brand kit.

**Fix:**
- Use `loadErrorText(e)` at the daemon-only sites and `providerErrorText(e)` in `JiraIssuePicker`.
- Render `OttoServerHome`'s gateway error through `LoadState variant="compact" … onretry`.
- In `siteApi.resolveBrand`, return `{ error }` separately from `note`. Have the Inspector show "Couldn’t load the brand kit — the preview uses the default palette" with a Retry (`loadBrand()`), and skip the contrast audit while the kit failed to load.
- Run `node scripts/ui-guards.mjs --update-baseline`.

### error-handling-6-06: A user MCP server can still drop out of a session with only a log line (nit; residual of 5-08)

**Files:** `crates/otto-sessions/src/mcp.rs:375-385` (JSON launchers: a bad env key, a failed env-file write), `:729-737` (codex: no free safe key), `:743-756` (codex: a bad env key, a failed env-file write)

**Evidence:**
- Save-time validation now stops new bad env keys, and codex sanitizes names, so the common causes are gone. The remaining paths still `tracing::warn!` and `continue`:
  - a failed env-file write (disk full, permissions)
  - a name with no free codex-safe key
  - a row saved before the validation existed
- The session starts without the server's tools, and the MCP page shows it as On.
- FIXES (P09) lists the runtime `mcp_skipped` meta and the session MCP menu as not done.

**Fix:**
- Collect `{server, reason}` for every skipped server in the launcher and store it as `meta.mcp_skipped` on the session.
- Show "N MCP servers weren’t attached" with the reasons in the session's MCP menu.
- Optionally, re-validate stored rows on `GET /mcp/servers` and flag the old bad ones.

### error-handling-6-07: The `Working` diff fallback swallows git failures and returns an empty diff (nit)

**File:** `crates/otto-git/src/local.rs:2825-2837` (`diff`) and `:2894-2904` (`diff_summary`)

**Evidence:**
- For `DiffTarget::Working`, a failed `git diff HEAD` is assumed to mean "HEAD is unborn" (per the comment), and the code falls back to `--cached` plus the worktree diff. Both fallbacks use `.unwrap_or_default()`.
- `git diff HEAD` can fail for other reasons: a corrupt index, a missing HEAD object, a failing external tool. In those cases the fallbacks fail too, and the result is an empty diff instead of an error.
- `WipPanel` requests this target for most file selections (`selTarget` → `'working'`, `WipPanel.svelte:188-198`). So a broken repository renders as "no changes" in the commit panel, not through the `LoadState` with Retry that 5-03 added.

**Fix:**
- Fall back only when `head_err` shows an unborn HEAD (`ambiguous argument 'HEAD'` / `unknown revision`). Otherwise return `upstream_err(&head_err, …)`.
- In the fallback, propagate with `?` instead of `unwrap_or_default()`.
