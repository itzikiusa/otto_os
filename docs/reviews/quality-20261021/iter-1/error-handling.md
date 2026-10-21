# Error handling: quality iteration 1 (2026-10-21)

Lens: resilience and error UX. Covers swallowed errors, raw exceptions in toasts, missing recovery after a daemon restart or network drop, retry storms, misleading success messages, unhelpful error text and crash paths.

Branch `fix/quality-loop-20261021` at `d447516c4`. The review was read-only (grep, sed and git only; no builds or tests).

## Score

Rubric: start at 10.0 and subtract per open finding (blocker 1.0, major 0.3, minor 0.1, nit 0.03).

- 2 major × 0.3 = 0.60
- 12 minor × 0.1 = 1.20
- 1 nit × 0.03 = 0.03
- Total deductions: 1.83

**Score: 10.0 − 1.83 = 8.17 / 10**

## Previous findings

None. This is the first error-handling pass of this loop.

## Areas covered

These areas were read and found sound:
- UI transport: `lib/api/client.ts`, which covers lanes, the alias suspend/probe, read deadlines, the 401 hand-off and `problemOf`.
- Shared error helpers: `loadError.ts`, `toastError.ts` and `clipboard.ts`.
- Events WebSocket reconnect, resync and boot-id handling in `lib/events.svelte.ts`.
- Auth boot, offline and capabilities handling in `lib/stores/auth.svelte.ts`, and the root `App.svelte` offline poll.
- Terminal reconnect policy in `Terminal.svelte` (surface check).
- Workspace session bootstrap catches in `workspace.svelte.ts`.
- Git: auto-fetch backoff in `git.svelte.ts`, GraphView refresh-error counting, the landed-marks diff (PR #113), ConflictResolverView, RepoView merge-status, GitToolbar, FocusView and ReviewPanel.
- Database store load paths: capabilities, children, object and diagram. Also the grid copy actions in EditFlow, GridView and ResultsGrid.
- Product Story tabs: PlanTab, AnalysisTab, RewriteTab and TestCasesTab pollers.
- Agents: NewSession batch spawn and retry, SessionView actions, and the ConversationView repo lookup.
- Rooms, excluding `games/**`: room-client WebSocket reconnect, RoomPage join/invite, RoomHost, room-media and recap-capture catches.
- Rooms server: action guards in `actions.rs`, `presentation.rs` and `registry.rs`; recap archive store open and locking.
- Sessions: MCP reconcile (PR #114: `.mcp.json`, agy `.agents/mcp_config.json`, grok), `sync_workspace_mcp`, and the status-update discards in the PTY supervisor.
- Server: `error.rs` Problem mapping; boot recovery ordering and fatal versus logged steps; `unwrap`/`expect` sweep over routes, sessions, git, dbviewer and connections (all checked invariants, no reachable panics found); `let _ =` state-write sweep.
- Usage tailer native and agy scans (PR #112).
- Settings: McpServers (move to Control Plane). API client store: git pull collections.
- Workbench store save and load errors (sound). Share page access recheck (sound).

## Areas not covered yet (next iteration)

- `crates/otto-server/src/modules.rs` (sessions, reviews and PR routes). It is large; only a sweep was done.
- `workflow_engine.rs`: node failure surfacing and retry policy.
- `otto-git` HTTP provider error mapping: how stderr and rate-limit text reach the UI.
- `otto-dbviewer` driver error text for MySQL, Postgres, Mongo, Redis and ClickHouse.
- Kubernetes, AWS and brokers stores (`k8s.svelte.ts`, `aws`). They use the same raw `errMsg` shape and need a per-site check.
- Vault, Design Hall, Canvas, Swarm, Assistant, Personal Agents, Insights, Scheduled Tasks, MCP Control Plane pages.
- Tauri shell (`apps/desktop/src-tauri`): native menu and window command failures.
- Retry storms on the server side: WebSocket fan-out loops, plugin supervisor restarts and channel bridge reconnects.
- The e2e assertions for error states (whether "Retry" is exercised).

---

## Findings

### error-handling-1-01: Product Story pollers swallow every failure and then report a misleading timeout, or spin forever (major)

**Files:**
- `ui/src/modules/product/PlanTab.svelte:156-158` (timeout at :144-147)
- `ui/src/modules/product/RewriteTab.svelte:68-70` (timeout at :57-60)
- `ui/src/modules/product/TestCasesTab.svelte:200-202` (timeout at :187-190)
- `ui/src/modules/product/AnalysisTab.svelte:130-132` (no time bound at all)

**Evidence:**
- Each poll's `catch (e)` only calls `console.error('[XTab] poll error', e)`. No error state is set and the poll keeps running.
- When the daemon is down, restarting, or returns 403/404 for the whole 2-minute window, Plan, Rewrite and Test cases end with a warning such as `'Plan timed out', 'No plan appeared within 2 minutes.'`. That blames the agent, when the real cause was that Otto could not read the result. The agent may well have written it.
- `AnalysisTab.pollOnce` has no `POLL_MAX_MS`. A persistent failure, such as an analysis deleted by another user (404) or a revoked grant (403), keeps the "running" detail on screen and re-polls forever. That is every 3 s with the events socket down and every 15 s with it up, with nothing shown to the user.

**Fix:**
- Track the consecutive poll failures. Keep `lastPollError = loadErrorText(e)`.
- After two failures, render an inline "Couldn't check progress: <cause>" with Retry next to the waiting state.
- When the timeout fires after failed reads, say "Couldn't check for the plan: <cause>" instead of "No plan appeared".
- In AnalysisTab, stop polling on 403/404 and show the cause inline. Add the same overall bound as the other tabs.

### error-handling-1-02: Core stores toast raw exception text instead of the shared `loadErrorText` (major)

**Files:**
- `ui/src/lib/stores/database.svelte.ts:83-85` (38 call sites)
- `ui/src/lib/stores/apiClient.svelte.ts:334-336` (41 call sites)
- `ui/src/lib/stores/product.svelte.ts:140-142`
- `ui/src/lib/api/usage.svelte.ts:282-284`

**Evidence:**
- Each store has a local `function errMsg(e) { return e instanceof Error ? e.message : String(e); }`. It feeds `toasts.error('Couldn't load children', errMsg(e))`, `'Couldn't load DB capabilities'`, `'Couldn't load object'`, `'Couldn't load cookies'`, `'Couldn't load run history'`, and so on.
- When the daemon is unreachable, the webview's `fetch` rejects with `TypeError("Load failed")`, so the toast body reads "Load failed".
- A 401 shows the server's raw text instead of "Your session expired. Sign in again, then retry."
- The shared helper exists for exactly this purpose. `toastError.ts` says: "the human cause (loadErrorText) as the body — never the raw exception".

**Fix:**
- Replace each local `errMsg` with `loadErrorText`, or route the toasts through `toastError(title, e)`.
- `loadErrorText` still passes `e.message` through for 4xx/5xx Problems, so SQL and driver error text is kept.

### error-handling-1-03: Plan tab toasts "Plan ready" even when the plan body failed to load (minor)

**File:** `ui/src/modules/product/PlanTab.svelte:128-141, 150-155`

**Evidence:**
- `loadPlanBody` catches its own error. It either toasts "Couldn't load plan" or sets `loadError`, and returns normally.
- `pollForPlan` then runs `clearPoll(); await loadPlanBody(plan); toasts.success('Plan ready', 'Implementation plan generated.')` unconditionally.
- The user gets a success toast next to an error. When a plan was already on screen, the poll is also cleared, so the new plan never appears.

**Fix:** Have `loadPlanBody` return a boolean, or rethrow. Toast success and clear the poll only when the body actually loaded; otherwise keep polling or show inline Retry.

### error-handling-1-04: Axum extractor rejections reach the user as a bare HTTP status phrase (minor)

**File:** `ui/src/lib/api/client.ts:428-436`

**Server side:** there is no rejection mapping anywhere in `crates/otto-server` (`rg JsonRejection|WithRejection` returns nothing). Handlers use plain `axum::Json`, `Path` and `Query` extractors.

**Evidence:**
- Axum answers a body or path that fails to deserialize with a text/plain body, for example "Failed to deserialize the JSON body into the target type: unknown variant `x`…", with status 400, 415 or 422.
- `problemOf` tries `resp.json()`, fails, and keeps `resp.statusText`. The toast then reads "Unprocessable Entity" or "Bad Request", and the actionable reason is thrown away.
- This is exactly the case after a background update leaves the UI and daemon one version apart, which `App.svelte:1232` already acknowledges.

**Fix:**
- In `problemOf`, read `await resp.text()` once and try `JSON.parse`. If that fails, use the trimmed text (capped at about 300 chars) as `message`, with `code: 'invalid'`.
- Optionally, add a server-side rejection-to-Problem mapping so every error body is `{code, message}`.

### error-handling-1-05: A failed `/auth/capabilities` read silently becomes "no access to anything" until reload (minor)

**File:** `ui/src/lib/stores/auth.svelte.ts:151-161, 247-253`

**Evidence:**
- Both paths set `this.capabilities = {}` and swallow the error: `catch { /* non-fatal: capabilities stay empty (all-deny for non-root) */ }`. Boot then goes to `phase = 'ready'`.
- `can()` reads `this.capabilities[feature] ?? 'none'`, so a non-root user gets every gated module hidden or denied, with no error.
- Nothing re-reads capabilities later. `events.noteBoot` only calls `auth.refreshMeta()`, and there is no reconnect hook. A transient timeout at boot, such as a daemon under load or the 20 s read deadline, persists until a manual reload.

**Fix:**
- Keep a `capabilitiesError` flag. Retry with backoff, and also on WebSocket reconnect or boot-id change.
- While the flag is set, show a shell banner: "Couldn't load your permissions — Retry". Do not show an all-deny shell.

### error-handling-1-06: A failed MCP-server lookup at spawn is treated as "user disabled every server" and strips the shared config files (minor)

**Files:**
- `crates/otto-server/src/routes/mcp_servers.rs:366-369`
- `crates/otto-sessions/src/manager.rs:2659-2680, 2712-2758`

**Evidence:**
- On `Err(e)`, `enabled_servers` logs `"mcp servers unavailable for spawn"` and returns `Vec::new()`. The comment there even says lookup failure "is a fault".
- `sync_workspace_mcp` cannot tell this from "none enabled". It reconciles `.mcp.json` with `user_servers: []`, which removes every previously managed user server from the marker set.
- It does the same to agy's `.agents/mcp_config.json` (new in PR #114) and grok's `.grok/config.toml`. It also records `mcp_servers` without them.
- These files are per-cwd and shared by concurrent sessions, so one transient DB or bridge failure silently drops the user's MCP tools for every session that launches in that cwd before the next healthy spawn.

**Fix:**
- Make the provider return `Result<Vec<McpServerSpec>, String>`.
- On `Err`, reconcile only Otto's own entries (otto, browser) and leave the previously managed user entries untouched. One way is `user_servers: Option<Vec<_>>`, where `None` means "unknown, preserve".
- Surface the failure in the session meta, for example `mcp_servers_error`.

### error-handling-1-07: `read_doc` treats any read error as an empty file, then overwrites the user's MCP config (minor)

**File:** `crates/otto-sessions/src/mcp.rs:525-532` (used by `reconcile_json_launcher` :251/:304 and by `retire_user_server`)

**Evidence:**
- `match std::fs::read_to_string(path) { Ok(s) if !s.trim().is_empty() => parse…, _ => Ok(Map::new()) }`.
- Only a JSON parse error is propagated. A non-UTF-8 file (for example an editor that saved UTF-16) or a permission error (EACCES) yields an empty map.
- The reconcile then `atomic_write`s a fresh document over the user's `.mcp.json` or `.agents/mcp_config.json`. Rename only needs directory write permission, so the user's hand-added servers are lost.

**Fix:** Return `Ok(Map::new())` only for `ErrorKind::NotFound` or an empty or whitespace file. Propagate every other read error, as a parse error already is, so the reconcile is skipped and logged.

### error-handling-1-08: The conflict resolver swallows a failed merge-status read and shows "No conflicted files." (minor)

**File:** `ui/src/modules/git/ConflictResolverView.svelte:94-100` (rendering at :251-252)

**Evidence:**
- `.catch(() => { status = { merging: initialFiles.length > 0, … }; pending = [...initialFiles]; … })`.
- When the resolver is opened with an empty seed, the list shows the neutral empty state "No conflicted files." with no error and no Retry, while the repo is still mid-merge.
- The seed is empty when it is opened from the "Resolve" banner after RepoView's own merge-status read also failed. RepoView's catch at `RepoView.svelte:151-157` sets `merging` from `conflictedPaths` but leaves `conflictSeed` at `{files: []}`.

**Fix:** In the catch, set `loadError = loadErrorText(e)`. When the seed is empty, render `LoadState` with Retry instead of the file panel. Keep the seed fallback only when it actually has files.

### error-handling-1-09: "Save & restart" reports "Couldn't save the session" when only the restart failed (minor)

**File:** `ui/src/modules/agents/SessionView.svelte:385-401`

**Evidence:**
- `await ws.updateSessionMeta(…)` succeeds, then `await ws.restartSession(sessionId, { quiet: true })` throws. The single catch toasts `'Couldn't save the session'`.
- The directories were saved. The user is told otherwise, and the dialog stays open inviting a duplicate save.

**Fix:** Split the try. On a restart failure, close the dialog and toast `Directories saved, but the session didn't restart` with the cause, and offer a Restart action.

### error-handling-1-10: Move to Control Plane swallows a failed rollback and leaves the server live in both registries (minor)

**File:** `ui/src/modules/settings/McpServers.svelte:268-283`

**Evidence:**
- If `mcpApi.update(s.id, {enabled:false})` fails, the code runs `await mcpCpApi.cpDelete(created.id).catch(() => {})` and rethrows the first error.
- If the delete also fails, the server is enabled both here and on the Control Plane, which is the state the comment says must never happen ("Never leave the server live in BOTH registries"). The toast only says "Couldn't move X: <disable error>".

**Fix:** Capture the rollback failure. When it happens, toast that the Control Plane copy still exists and is enabled, with the cause, and reload both lists so the duplicate is visible.

### error-handling-1-11: Git "Pull collections" counts failed imports as imported (minor)

**File:** `ui/src/lib/stores/apiClient.svelte.ts:1186-1195` (with `importParsed` :1057-1068 and `saveCollection` :1036-1040)

**Evidence:**
- `importParsed` never throws on a save failure. `saveCollection` toasts and returns `null`, then `if (!root || !base) return;`. The viewer-role branch also returns early.
- `gitPullCollections` therefore runs `imported++` for files that created nothing, and finishes with `toasts.success('Pulled from git', plural(imported, 'collection file'))`.
- `catch { /* skip non-collection json */ }` also hides real network or API errors under "not a collection".

**Fix:** Have `importParsed` return whether the root was created and how many items failed. Count only real imports, and report skipped files (parse errors) separately from failed imports (API errors).

### error-handling-1-12: DB grid "Copy" actions toast "Copied" without checking the clipboard result (minor)

**Files:**
- `ui/src/modules/database/EditFlow.svelte.ts:885-886`
- `ui/src/modules/database/GridView.svelte:777-778`
- `ui/src/modules/database/ResultsGrid.svelte:867-871`

**Evidence:**
- The pattern is `void copyText(where); toasts.success('Copied', …)`.
- `copyText` returns `false` when both the async API and the legacy path fail, for example on an unfocused document or a blocked permission. `clipboard.ts:54-56` documents `copyTextOrThrow` precisely "instead of silently claiming success".
- `RecordDiff.svelte:78` and `Terminal.svelte:1904` in the same area do check the result.

**Fix:** Use `if (await copyText(x)) toasts.success(...) else toasts.error('Couldn't copy', 'Select the text and copy it manually.')`, or use `copyTextOrThrow` with a catch.

### error-handling-1-13: The room join shows the raw network exception ("Load failed") to guests (minor)

**Files:**
- `ui/src/modules/rooms/room-client.ts:5-14`
- `ui/src/modules/rooms/RoomPage.svelte:64-68`

The same raw pattern appears in `StartRoomModal.svelte:23`, `RoomSettingsModal.svelte:24` and `RecapSettingsModal.svelte:13`.

**Evidence:**
- `joinRoom` calls `fetch` unguarded. If the host's daemon is unreachable, the fetch rejects with `TypeError("Load failed")`.
- `join()` renders `e instanceof Error ? e.message : '…'`, so the guest sees "Load failed". The intended "Could not join…" text only covers HTTP errors.

**Fix:** In `joinRoom`, wrap `fetch` and map a network failure to `new Error('Couldn't reach the host. Check the link or ask the host to keep Otto open, then try again.')`, or use `loadErrorText` at the render sites.

### error-handling-1-14: One unwritable recap archive makes every recap of every user unavailable (minor)

**Files:**
- `crates/otto-server/src/rooms/recap/archive.rs:119-139`
- `crates/otto-server/src/rooms/recap/mod.rs:113-124`

**Evidence:**
- `Store::open` iterates every archive. Load errors are skipped silently (`if let Ok(a) = store.load(&id)`), but the restart normalisation does `a.save(&s)?`, so a single archive whose metadata can't be written aborts the whole open.
- Causes include ENOSPC, which this machine has hit, a read-only file, or a directory removed mid-scan.
- `store()` uses `get_or_try_init`, so every recap request re-scans all archives and fails with that one disk error.

**Fix:** Inside the loop, log and skip a failed save, as a failed load is already skipped. Optionally, mark that archive as "needs repair" in memory instead of failing the store. Also log (with `warn!`) the archives skipped on load so they don't vanish without a trace.

### error-handling-1-15: ConversationView caches a failed repo lookup for the life of the app (nit)

**File:** `ui/src/modules/agents/conversation/ConversationView.svelte:15, 276-281`

**Evidence:**
- `reposByWs` is module-level, so it is shared by every conversation. The code is `p = api.get(...).catch(() => [] as Repo[]); reposByWs.set(id, p)`.
- A single failure, for example during a daemon restart at launch, is memoized as `[]` until a full reload.
- As a result, `#123` issue links never resolve in that workspace, and repos added later never appear either.

**Fix:** Delete the map entry on failure (`.catch(() => { reposByWs.delete(id); return []; })`), and clear the cache on the events reconnect/resync hook.
