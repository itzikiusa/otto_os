# Error handling: quality iteration 6, final review (2026-10-23)

Lens: resilience and error UX. This covers swallowed errors, raw exceptions shown instead of an inline Retry, missing recovery after a daemon restart or network drop, retry storms, misleading success or status messages, unhelpful error text and crash paths.

Branch `fix/quality-iter6-20261023` at `2065f00e9`, after fix waves 1–3. The review was read-only (grep, sed and git only; no builds or tests).

## Score

Rubric: start at 10.0 and subtract per confirmed open finding (blocker 1.0, major 0.3, minor 0.1, nit 0.03).

- 0 blocker, 0 major
- 4 minor × 0.1 = 0.40
- 1 nit × 0.03 = 0.03
- Total deductions: 0.43

**Score: 10.0 − 0.43 = 9.57 / 10** (wave 0 of this iteration: 9.44; iteration 5: 8.87; iteration 4: 9.41; iteration 3: 9.31; iteration 2: 9.17; iteration 1: 8.17)

All seven wave-0 findings are fixed at every listed location. Five findings remain open. Two are residual siblings of fixed patterns (raw load errors; the Insights schedule status). Three come from areas wave 0 did not cover: the Self-Improvement and Design Hall rule decisions, browser marks, and API-client Keychain reads. No new problem in this lens was found in the code the fix waves added or moved.

## Previous findings (wave 0)

| ID | Status | Evidence |
|----|--------|----------|
| 6-01 Design Hall assist turn lost on daemon restart stays "working" (minor) | Fixed | `assist/model.ts:133-160`: `LOST_TURN_ERROR`; `mergeTurns(local, fetched, known)` turns a known non-terminal turn that the server no longer lists into `failed`. `OttoPanel.svelte:134-137` passes the `known` set on the authoritative reload. The optimistic send at `:316` passes no set, so a just-sent turn is kept. `turnOutcome(undefined)` returns the lost-turn error, and `Studio3D.svelte:184-188` ends the poll with that toast. |
| 6-02 "Proof assembled" shown when nothing was assembled (minor) | Fixed | `routes/proof.rs:637-672` collects `AssembleSummary { diff, commands }`. A HEAD that moved is propagated as the 409 (`map_err(ApiError)?`), and each command's store error is reported. `proof.rs:309-384` returns `DiffAssembly::{NotARepo, DiffFailed, NoChanges, Assembled}`. `ProofPage.svelte:425-430` with `assembleOutcome.ts` picks the info, error or success tone from the summary. |
| 6-03 Failed scheduled Insights runs leave no trace; the empty page says "off" (minor) | Fixed (a residual is listed as 6f-05) | `otto-insights/src/lib.rs:203-257` persists `schedule-state.json`. `:1662-1712` records `{period, attempts, last_error, gave_up}` on `Ok(None)`, on `Err`, and on a run that ends without a report. The record is cleared on success, at `:1692`. `scheduleStatus.ts` says "off" only when every cadence is off, and names the failing cadence and its last error. Residual: this is rendered only in the no-reports empty state (6f-05). |
| 6-04 Chat "abort" replies "Aborting…" when the cancel failed or the run had already finished (minor) | Fixed | `otto-workflows/src/chat.rs:1120-1161` `abort_reply` has three branches (aborted, already finished, couldn't abort). Each branch has a unit test at `:1168-1192`. |
| 6-05 Remaining load failures show the raw exception (minor) | Fixed at every listed site (siblings are listed as 6f-01) | `loadErrorText` is used in `CollectionView:99`, `LearnedCard:40`, `CompareModal:83`, `OttoPanel:152`, `siteApi.ts:129`, `Switcher:39`, `NetworkProfilePicker:20`, `SessionNetworkStatus:43`, `sftp.svelte.ts:99`, `OttoServerHome:95,247,285`, `PreviewBody:147`, `transcript.svelte.ts:283,308,516` and `CanvasPanel:186`. `providerErrorText` is used in `JiraIssuePicker:159`. The gateway tools render through `LoadState … onretry` (`OttoServerHome:514`). The brand kit renders through `LoadState` with Retry (`Inspector.svelte:607-608`), and the audits skip brand checks while the kit failed to load (`SiteStudio:514`, `Inspector:125-126`). `SessionNetworkStatus:53` is an action (`choose`), which is allowed. |
| 6-06 A user MCP server can drop out of a session with only a log line (nit) | Fixed | `otto-sessions/src/mcp.rs:673-700` `skipped_servers` derives `{server, reason}` from the delivered set, which covers the JSON launchers and codex alike. `manager/mcp_sync.rs:210-228` writes `meta.mcp_skipped`, and a null clears it. `SessionMcpProfiles.svelte:28-35,82` lists the skipped servers with their reasons. Test: `manager/tests/submit.rs:281-341`. |
| 6-07 `Working` diff fallback swallows git failures (nit) | Fixed | `otto-git/src/local.rs:2825-2841` and `:2896-2911` fall back only through `unborn_or_err` (`local/errors.rs:13-26`), which accepts only unborn-HEAD stderr and otherwise returns the error. The fallbacks use `?`. Tests: `errors.rs:201-210` and `local/tests.rs:3226`. |

## Areas covered

- Re-verified all seven wave-0 findings (table above) and spot-checked the iteration-5 fixes for regressions. These are still in place: `daemonState` with Restart/Reinstall (`App.svelte:114,143,222-226`), the `relaySocket` `gave-up` state, `fallback_note` (`summarize.rs:223`) and `saved_report` (`personal_agents_engine.rs:963,1179`).
- **Code changed this iteration** (`git diff origin/main`: 864 files):
  - New swallowed catches in `ui/src`. The added `refreshAfter().catch(() => {})` lines in `git/graph/*` are moved code, and `refreshAfter` records its own failure through `noteBackgroundRead`, `graphData.svelte.ts:123-128,506-511`.
  - New raw-error expressions and new `toasts.success` calls. They are moved `apiClient` store and `git` graph code, plus clipboard helpers.
  - New pollers or retry loops. There are none in the UI. The `fanout` retries are bounded by `max_user_retries`/`max_auto_retries` (`otto-agent-run/src/fanout/retry.rs`).
- **otto-agent-run fan-out** (`fanout.rs`, `fanout/retry.rs`) and its users:
  - product analysis and plan (`otto-product/src/run/analysis.rs`, `run/plan.rs`): a run where every planner failed still writes the `plan_error` event and `ProductChanged{status:"error"}`, at `plan.rs:421-440`
  - the vault docs retry endpoints
  - the design-assist job (`otto-design-assist/src/job.rs`): restoring the working head after a failure is kept
- **otto.query_db** (`df56f044e`): the write-guard refusals now name the setting that lifts them (`otto-dbviewer/src/service.rs`), and the self-call timeout covers the statement deadline. Sound.
- **MCP governance stage runner** (`otto-mcp/src/governance.rs`): no swallowed results.
- **Swarm submit run guard** (`otto-swarm/src/runtime/agent_run.rs`): `TurnSubmit::{Sent, Stopped, NotSent}` keeps the kill-and-retry and stop paths.
- **otto-channels Telegram long-poll** (`telegram.rs:542-700`), which was not covered before:
  - exponential backoff from 3 s to 60 s
  - `health.failed` with a permanent flag for Unauthorized and Conflict
  - the offset is advanced only over a handled prefix
  - cancel is honoured mid-poll

  Sound.
- **otto-improve** approve, reject and rollback (`engine.rs:679-860`, `http.rs:236-261`), which was not covered before. The claim and revert logic is sound. The UI reports a conflict as success (6f-02).
- **otto-product story watcher** (`watcher.rs`): a failed comment fetch sends a `Notice`. The refresh and self-improvement steps are best-effort and logged. Acceptable.
- **Plugin supervisor** (`otto-server/src/plugins.rs:185-381`): a crashed sidecar has no auto-restart loop, and its cause is kept in `last_errors` and shown in the admin list. Sound.
- **API client**:
  - the stream relay (`apiStream.svelte.ts`) closes to `closed`/`error` with the message
  - the REST execute engine that moved into `otto-apiclient` (`engine/*`, `secrets.rs`) (6f-04)
- **otto-browser**:
  - live view (`live/connection.ts`) has backoff and an `ended` state after max attempts
  - reader page and marks loading (6f-03, 6f-01)
- **Skills eval**: a failed run is finalized as `Error` with a notice (`skill_eval.rs:1195-1260`). A failed multi-signal scoring pass at `:1563` is only logged, and the scorecard is simply absent. This is not counted, because the iteration's judge score and status are still recorded.
- **Remaining raw-error baseline**: re-walked all 57 entries of `ui-guards-baseline.json` `raw-error` (66 hits, down from 90) and separated load failures (6f-01) from action, parse and local-compute failures.

## Areas NOT covered

- The gRPC describe, reflect and invoke paths in `otto-apiclient` (`grpc*`), and the server-side `pm` script engine.
- `otto-rooms` beyond the moved code paths: room media, games authority and terminal.
- The workflow engine executor's new `run/schedule.rs` and `run/admission.rs` beyond the chat controls. These are refactors, and only spot reads were done.
- `vault_docs_agent` review loop and refine (`review_loop.rs`, `refine.rs`) beyond the retry endpoints.
- AWS and Kubernetes consoles beyond the iteration-5 fixes and `S3Preview`.

---

## Findings

### error-handling-6f-01: Five more load failures still show the raw exception ("Load failed") (minor; residual of 6-05)

**Files (GET/read failures, each already rendered with a Retry):**
- `ui/src/modules/desktop/TrayPage.svelte:215`: `errors[key] = e instanceof Error ? e.message : String(e)`, rendered as "Couldn’t refresh your work: {loadError}" at `:314`. The menu-bar tray is the surface most likely to be open while the daemon restarts.
- `ui/src/modules/insights/ReportDetail.svelte:125`: `htmlError = e instanceof Error ? …`, rendered at `:254` and announced at `:208`.
- `ui/src/modules/agents/conversation/PreviewBody.svelte:116`: the image branch (`authedBlobUrl(...)` rejection). The text branch at `:147` was fixed to `loadErrorText`; this one, in the same file, was missed.
- `ui/src/lib/stores/browser.svelte.ts:368`: `pageError = e instanceof Error ? e.message : 'Couldn’t load the page.'`, rendered by `ReaderView` (`BrowserView.svelte:833`).
- `ui/src/modules/aws/S3Preview.svelte:128`: `mediaError = e instanceof Error ? …`, rendered at `:158` and announced at `:142`. The other AWS views use `awsLoadText`.

**Evidence:**
- Each site stores the exception text of a daemon read.
- When the daemon is unreachable, WebKit rejects with `TypeError("Load failed")`. The tray then reads "Couldn’t refresh your work: Load failed", and the report reads "Couldn’t load the HTML report … Load failed".
- An expired token shows the raw 401 body instead of "Your session expired. Sign in again, then retry."
- `lib/loadError.ts:1-31` documents the rule and maps a `TypeError` to "Otto can’t reach the daemon."
- All five files are still in the `raw-error` baseline.

**Fix:**
- Use `loadErrorText(e)` at `TrayPage:215`, `ReportDetail:125`, `PreviewBody:116` and `browser.svelte.ts:368`.
  - The browser page proxy reports the remote site's failure as a non-401/403/404 status, so `loadErrorText` keeps that message.
- Use `awsLoadText(e)` at `S3Preview:128`.
- Then run `node scripts/ui-guards.mjs --update-baseline`.

### error-handling-6f-02: Approving or rolling back an improvement edit says it succeeded when the daemon answered `conflict` and wrote nothing (minor)

**Files:**
- `crates/otto-improve/src/engine.rs:684-694`: `approve_edit` returns `Ok(edit)` with `status = Conflict` when the target file changed since the snapshot.
- `crates/otto-improve/src/engine.rs:791-801`: `rollback_edit` returns `Ok(edit)` with `status = Conflict` when the file no longer matches.
- `crates/otto-improve/src/http.rs:236-261`: both handlers answer 200 with that edit.
- `ui/src/modules/settings/SelfImprovement.svelte:257-258`: `await improveApi[action](edit.id); toasts.success(action === 'approve' ? 'Edit approved and applied' : …)`.
- `ui/src/modules/design-hall/assist/ruleActions.ts:28-29`: `await ruleEdits.approve(...)` then `toasts.success('Added to {skill}', 'Rollback is available under Rules.')`.
- `ui/src/modules/design-hall/assist/ruleActions.ts:64-65`: `await ruleEdits.rollback(...)` then `toasts.success('Rule rolled back')`.
- The helper `why()` at `ruleActions.ts:15` handles a 409 "The rules file changed since this was proposed", but that status is never sent for this case.

**Evidence:**
- The conflict is reported as a successful response whose body has `status: "conflict"`. None of the three callers reads the returned status.
- So when the skill file was edited after Otto proposed a change, the user is told "Edit approved and applied" or "Added to design-team-style", while nothing was written. The same happens with "Rule rolled back" when the rule is still in the file.
- The list reload after the toast shows the edit as Conflict. The headline the person reads, and that the shell announcer speaks, is still the success.
- The UI types already include `'conflict'` (`types/skills-context.ts:24`, `types/design-assist.ts:170`).

**Fix:**
- In the three callers, branch on the returned `edit.status`:
  - `'conflict'` → `toasts.warn('The file changed since this was proposed', 'Nothing was applied. Otto re-proposes it against the current file on the next pass.')`, or "Nothing was rolled back — the file was edited after this rule was added." for a rollback
  - `'applied'` or `'rolled_back'` → keep the success toast
- Alternatively, have the daemon answer 409 for these two cases, which `why()` already expects. Then update `docs/contracts/api.md:2047-2049`.

### error-handling-6f-03: A failed marks load shows "No marks yet" (minor)

**Files:**
- `ui/src/lib/stores/browser.svelte.ts:358`: `browserApi.listAnnotations(ws, url).catch(() => [] as BrowserAnnotation[])`
- `ui/src/lib/stores/browser.svelte.ts:378-381`: `loadAnnotations`: `try { next = await browserApi.listAnnotations(ws, url); } catch { next = []; }`
- `ui/src/modules/browser/NotesRail.svelte:57-60`: `{#if annotations.length === 0}` "No marks yet. Turn on Mark passage above the page…"

**Evidence:**
- Both reads of a page's saved marks turn any failure into an empty list. This includes a daemon restart, a dropped network, a 401 and a 5xx.
- The Marks rail then shows a count of 0 and "No marks yet", so the user's notes look deleted. The empty state also invites them to mark the passages again, which creates duplicates once the read works.
- `lib/loadError.ts:4` states the rule this breaks: "a swallowed load error must never fall through to a 'No X yet' empty state."
- Nothing re-reads the marks until the tab or page changes.

**Fix:**
- Add `annotationsError` to the store. Set it to `loadErrorText(e)` in both catch sites, keeping the previous `annotations` for the same URL instead of `[]`.
- In `NotesRail`, render `<LoadState variant="compact" what="this page’s marks" error={annotationsError} onretry={() => browser.loadAnnotations(url)} />` ahead of the empty state.

### error-handling-6f-04: API client Keychain read failures become blank credentials (execute), or an emptied secret set (environment save) (minor)

**Files:**
- `crates/otto-apiclient/src/secrets.rs:486-492` (`load_blob`) and `:505-511` (`load_blob_async`): `.ok().flatten().and_then(…).unwrap_or_default()`, so a Keychain error or corrupt JSON gives `{}`.
- `crates/otto-apiclient/src/secrets.rs:157-167`: `resolve_auth_markers` reads through `load_blob`. The `Prefetched` snapshot (`:573-592`) keeps the read error, but `load_blob` drops it. A `$secret` auth member then becomes `""` (`unwrap_or_default()` at `:166`).
- `crates/otto-apiclient/src/engine/env.rs:32-43`: the environment's secret variables come from `load_blob_async`. On failure the variables are simply absent.
- `crates/otto-server/src/routes/api_client.rs:715-716`: a single secret field resolves through `load_blob_async`, also to `unwrap_or_default()`.
- Write paths that read the blob with the unchecked loader and then write it back:
  - `crates/otto-server/src/routes/api_client/environments.rs:69-76`: environment PATCH, which calls `load_blob_async`, then `apply_secret_changes`, then `store_blob_async`
  - `routes/api_client.rs:851-857`: securing environment variables

**Evidence:**
- On macOS, a Keychain read can fail when the login keychain is locked or when the person clicks Deny on the access prompt.
- In that case a saved request is sent without its bearer token or API key, and environment `{{token}}` variables are empty. The person sees the upstream's 401 and has nothing that points at the Keychain.
- The crate already has the checked loader for this purpose: `load_blob_checked(_async)` at `secrets.rs:470-482` and `:513-520`, with the text "Could not read saved credentials from Keychain. Unlock it and retry without replacing credentials." The request-secret write paths use it (`api_client.rs:800`, `requests.rs:156,159`, `api_oauth.rs:98,263,380`).
- The environment write paths do not use it. After a failed or unreadable read they rebuild the blob from `{}` and store it, which replaces every other stored secret of that environment. An empty blob deletes the entry (`store_blob_async`, `secrets.rs:530-531`).
- The test `secrets/tests.rs:219` pins the unchecked loader's empty result for a locked store.

**Fix:**
- Use `load_blob_checked_async` in `engine/env.rs:35`, `environments.rs:69`, `api_client.rs:715` and `api_client.rs:851`, and propagate the error.
- Make `resolve_auth_markers` use `load_blob_checked` and map its error into the returned `Err(String)`. The prefetched snapshot already carries the read result.
- Execute then fails with "Couldn’t read saved credentials from Keychain. Unlock it and retry.", and an environment save refuses instead of overwriting.
- Keep the unchecked `load_blob` only for redaction-only callers, such as `api_automation_runs.rs:125`.

### error-handling-6f-05: Failing scheduled Insights runs are shown only while there are no reports, and a failed config read offers to "turn on" schedules (nit; residual of 6-03)

**Files:**
- `ui/src/modules/insights/InsightsPage.svelte:125`: `emptySchedule = $derived(scheduleCopy(schedule))`, used only in the EmptyState at `:788-793`
- `ui/src/modules/insights/InsightsPage.svelte:130`: `getConfig().catch(() => { schedule = null; })`
- `ui/src/modules/insights/scheduleStatus.ts:18`: `if (!cfg) return { text: '', offerTurnOn: true, … }`
- `ui/src/modules/settings/InsightsSettings.svelte`: does not read `schedule_status`

**Evidence:**
- The daemon now records the failures: `schedule_status` with `last_error` and `gave_up` (`otto-insights/src/lib.rs:1700-1712`).
- The only place the UI shows them is the no-reports empty state. A user who already has reports, and whose daily schedule then starts failing (for example after the provider signs out), sees the old reports list and no hint that new ones stopped. The settings page that owns the schedule doesn't show it either.
- If the config read fails, the empty state offers "Turn on scheduled reports" even when schedules are on.

**Fix:**
- Render the failing-cadence sentence from `scheduleCopy` as an inline warning above the report list whenever `failing` is true, with Run now. Show the same line under the cadence toggles in `InsightsSettings`.
- On a failed config read, keep `offerTurnOn: false` with a neutral "Schedule settings" label.
