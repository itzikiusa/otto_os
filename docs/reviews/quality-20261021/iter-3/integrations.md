# Iteration 3: integrations lens

Scope: how robust the external integrations are. That covers the provider CLIs and MCP launcher
injection, git/PR provider flows, Jira, Confluence and Linear, Slack, Telegram and email channels,
SSH tunnels and DB connections, Kafka, the browser sidecar and live proxy, AWS and k8s, and the
in-session stdio MCP bridge. I only read the code. I ran no builds or tests.
Base: `fix/quality-loop-20261021` @ 3dd55d9f2 (includes the iteration-2 fixes 52edfdbe7 / 49ede5829).

## Score

Start at 10.0. Open findings: 1 major (−0.3) and 2 minor (−0.1 each).

`10.0 − 0.3 − 0.1 − 0.1 = **9.50**`

## Status of the previous findings

| id | status | evidence (current code) |
|----|--------|-------------------------|
| integrations-2-01 | fixed | `launch_via_env_file` (`crates/otto-sessions/src/mcp.rs:352-385`) writes a server's env to a 0600 file under `<tmp>/otto-mcp/launch-*.env`. The JSON/TOML entry becomes `/bin/sh -c ENV_EXEC_SCRIPT <file> <cmd>…` with no `env` key. `atomic_write` creates launcher files 0600 (`:905-930`). `exclude_launcher_artifacts` appends root-anchored paths to `<git-common-dir>/info/exclude` (`:440-540`). The test `secret_env_never_lands_under_the_cwd_and_launchers_are_git_excluded` (`:1391`) covers this. The env files are never cleaned up; that residual is reported as integrations-3-03. |
| integrations-2-02 | fixed | `check_run_rows` → `Http::json_pages` follows `Link rel=next` (20-page cap) (`crates/otto-git/src/providers/github.rs:304-315`, `client.rs:880-924`). `checks_from_check_run_pages` adds a pending "N more checks (not read)" row when `total_count` is larger than the rows read (`github.rs:357-380`). Statuses come from the combined `/commits/{sha}/status` (`:317-350`). Bitbucket follows `next` (`bitbucket.rs:84-120,246-262`). |
| integrations-2-03 | fixed | `crates/otto-channels/src/gfm.rs` converts GFM to Slack mrkdwn and Telegram HTML. Slack `send_formatted` uses `to_slack_mrkdwn` (`slack.rs:461`). Telegram sends `parse_mode: "HTML"` and falls back to plain text on parse or length errors (`telegram.rs:331-369`). Unclosed fences, `snake_case` and code spans are handled (`gfm.rs:113-140,280-330`). |
| integrations-2-04 | fixed | `("completed", "stale") => "failure"` (`github.rs:431-434`). |

Previous findings: 4 fixed, 0 still open, 0 rejected.

## Areas covered in this iteration

- Re-verified all four iteration-2 fixes, including the new launcher env-file path, the
  `info/exclude` resolver (worktree `commondir`, `gitdir:` files), the GitHub/Bitbucket pagination
  and the GFM converters.
- Stdio MCP bridge (`crates/ottod/src/mcp_tools.rs`): `tools/list` composition, governed-proxy
  aliasing (`GOVERNED_ALIASED_BY_NATIVE`, `GOVERNED_BY_STDIO_NAME`), native mutating tools routed
  through `governed_native_write`, and per-call timeouts (`CALL_TIMEOUT`, `TEXT_CALL_TIMEOUT`,
  `post_json_within`) compared with the daemon routes they proxy (`/sessions/{id}/wait`,
  `/browser/summarize`, k8s logs).
- `otto-k8s`: `logs.rs` (argv construction, `--limit-bytes`, ring-capped one-shot read, follow stream
  with concurrent stderr drain, kill-on-drop) and `actions.rs` (scale/restart/rollout/ArgoCD
  sync/refresh/terminate/app-restart/CronJob trigger argv, per-step re-authorization).
- `otto-aws`: `accounts.rs` (`env_for`, assume-role cache and single-flight, SSO export-credentials
  cache and eviction, endpoint and region changes), `creds.rs`, `s3_download.rs` (`aws s3 cp -`
  streaming, O_EXCL part file, no-overwrite placement, stderr drain), `athena.rs`
  (start/status/cancel, native SigV4 path with CLI fallback, token compatibility), `logs.rs`
  (filter-log-events, Insights start/results/stop).
- `otto-git` providers: Bitbucket write paths (create_pr reviewer resolution, update, comment,
  resolve, approve, merge/merge_pinned, decline, request-changes, Basic→Bearer retry); GitLab
  `wait_for_rebase` (`include_rebase_in_progress`, `merge_error`, 60 s cap); `local.rs` autostash
  (`--include-untracked`, SHA-identified stash, `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`).
- `otto-issues`: the new Linear client (`linear.rs`: auth header form, GraphQL error mapping, cursor
  walk for offset paging) and `tracker.rs` dispatch (JQL vs IssueFilter guards, no direct
  `JiraClient` bypasses).
- `otto-channels`: `email.rs` (lettre STARTTLS relay, self-send verify), `improve_notify.rs`,
  `slack_directory.rs` (cursor walk, scope fallback, 429), the mirror's long-reply split with GFM.
- `otto-ssh` (`SshTunnel` launch/readiness/TOCTOU guard/stderr drain, base args) and the dbviewer
  tunnel cache (`tunnel_for` fingerprinting, reap_idle, handle eviction).
- `otto-browser/live/proxy.rs` (SOCKS5 netguard proxy: handshake, vetting, IP literal in the domain
  field, connection cap).
- `otto-orchestrator/claude_pty.rs` (argv per TurnMode, Seatbelt wrap, transcript path, end_turn
  detection, retry/backoff) and the `otto-agent-run` module outline.
- Claude `.mcp.json` project-server approval: Otto writes no `enabledMcpjsonServers` and the claude
  prompt-guard table has no MCP-approval needle. Whether the CLI asks for approval when it runs
  with `--dangerously-skip-permissions` cannot be settled from this repository's code, so this is
  not reported.

## Not covered yet (for the next iteration)

- `otto-dbviewer` Mongo, Redis, MySQL and Postgres driver error variants in depth, and the
  `mcp-query` server-side time budget compared with the bridge's 20 s (not confirmed here).
- `otto-brokers` consumer-group offset commit against an active group (per-partition commit errors
  from `rd_kafka_commit`), and schema-registry auth variants.
- `otto-connections/http.rs` session lifecycle (PTY reconnect after sleep).
- The agy `.agents/mcp_config.json` and grok TOML launcher formats against the current CLIs'
  readers.
- `otto-agent-run/turn_oracle.rs` (2.7k lines): transcript variants for codex/agy.
- The outward MCP HTTP transport (`mcp_http`) time budgets for the same long-running tools.

## Findings

### integrations-3-01: major: the stdio MCP bridge cuts long-running tools off at 20 s, below the daemon routes it proxies

- **Where:** `crates/ottod/src/mcp_tools.rs:113` (`CALL_TIMEOUT = 20 s`), `:202-216` (`get_json`
  wraps every GET in `tokio::time::timeout(CALL_TIMEOUT, …)` → `"upstream timeout after 20s"`),
  `:248-250` (`post_json` = `post_json_within(…, CALL_TIMEOUT)`), `:4731-4733` (the reqwest client
  is also built with `.timeout(CALL_TIMEOUT)`).
- **Evidence:**
  - **`otto_wait_session`.** The tool advertises "block up to 25 s" with `timeout_secs` "1..25,
    default 20" (`mcp_tools.rs:1273-1275`). The call goes `read_route` → `ctx.get_json`
    (`:2573-2580`, `:4251-4266`). The daemon route waits `timeout_secs.unwrap_or(20).min(25)`
    (`crates/otto-server/src/modules/session_input.rs:228,414`). So with the documented default
    the daemon answers `reached:false` at 20 s, exactly when the bridge's own 20 s timer fires.
    That timer started earlier, before the request was sent, so the bridge usually wins. Any
    `timeout_secs` ≥ 20 always loses. The agent gets an error result instead of the
    `{reached:false}` it is told to loop on. This is the delegation primitive
    (`otto_open_session` says "poll otto_wait_session"). It matches the known workaround in the
    team's notes ("pass 14").
  - **`browser_summarize`.** The bridge `post_json`s `/workspaces/{ws}/browser/summarize`
    (`mcp_tools.rs:4025-4038`) under the 20 s budget. That route runs a full agent-session turn:
    it spawns claude/codex in a PTY, waits for the TUI, and waits up to `STUCK_IDLE` for the turn
    (`crates/otto-server/src/routes/browser.rs:1051-1076`). That routinely takes more than 20 s.
    When the bridge drops the request, the client disconnect drops the handler future, and
    `KillSessionOnDrop` (`browser.rs:1060,1089`) kills the summarizing agent. The work is thrown
    away and the agent sees "upstream timeout after 20s". The k8s log fetch already has its own
    longer budget (`TEXT_CALL_TIMEOUT = 65 s`, `:126-131`), and governed calls use
    `post_json_within`. These two routes were missed.
- **Fix:**
  - Give each long-running read its own budget, as `TEXT_CALL_TIMEOUT` already does. Add
    `get_json_within(path, budget)`.
  - For `otto_wait_session`, use `budget = clamp(timeout_secs, 1, 25) + 5 s`, or clamp the
    forwarded `timeout_secs` to `CALL_TIMEOUT − 5` and change the schema text to match.
  - For `browser_summarize`, use `post_json_within(…, STUCK_IDLE + margin)` (a minute or more).
  - Raise or remove the client-level `.timeout(CALL_TIMEOUT)`, so the per-request budget applies.
  - Add a bridge test with a stub daemon that answers after 21 s, and assert that the default
    `otto_wait_session` returns the upstream body rather than an error.

### integrations-3-02: minor: a Bitbucket merge that returns 202 Accepted is reported as merged

- **Where:** `crates/otto-git/src/providers/bitbucket.rs:752-770` (`merge` → `send` →
  `Http::into_result`, where any 2xx is success), `crates/otto-git/src/providers/client.rs:1000-1008`,
  the route `crates/otto-git/src/http.rs:3112-3133` (returns 204 "merged"), and `merge_body`
  (`bitbucket.rs:1053-1061`, no `async` handling).
- **Evidence:** Bitbucket Cloud's `POST …/pullrequests/{id}/merge` runs synchronously only up to a
  server timeout. Past it, it answers **202** with a `Location` header pointing at
  `…/merge/task-status/{task_id}`, and the merge may still fail later (a hook rejection, a
  conflict, a failing merge check). The client never looks at the status code beyond
  `is_success()`, never reads `Location` and never polls the task. A slow merge on a large
  repository is therefore reported to the UI and to `otto_merge_pr` callers as done, even though
  the PR may still be open or the merge may fail. Nothing in the crate handles 202 or task-status:
  `rg "202|task-status"` in `providers/bitbucket.rs` and `client.rs` finds only test timestamps.
- **Fix:** In `merge`, keep the response. On `202`, poll the `Location` task-status URL
  (same-origin, with the same auth) until `task_status` is `SUCCESS` (return Ok) or `FAILED` (map
  `merge_result` to `Error::Conflict`). Cap the poll at about 60 s. If it hasn't finished by then,
  return a distinct "merge queued" outcome (a 202-style or `Conflict` message) rather than claiming
  success. Add a wire test where the merge POST answers 202 and the task first reports `PENDING`,
  then `SUCCESS` or `FAILED`.

### integrations-3-03: minor: MCP launcher env files keep a deleted or disabled server's secrets on disk indefinitely, and the feature guide still says `.mcp.json` holds them

- **Where:** `crates/otto-sessions/src/mcp.rs:371-385` (each reconcile writes
  `<tmp>/otto-mcp/launch-<cwd-hash>.<server>-<hash>.env` holding the Keychain-resolved values),
  `:386-411` (the comment says the `launch-` prefix is deliberately "clear of the per-session
  `<session_id>.*` files the session cleanup removes"). Reconcile drops a server's entry from the
  launcher (`:322-330`) but never deletes its env file. The only `remove_file` calls in `mcp.rs` are
  temp-file cleanups (`:425,927`). The delete route `crates/otto-server/src/routes/mcp_servers.rs:287-299`
  removes the DB row and the Keychain blob only. The docs are stale:
  `crates/otto-server/src/routes/mcp_servers.rs:310-315` and
  `docs/features/connections-ssh-sftp.md:448-452` still say "the rendered `.mcp.json` on disk does
  contain the real values".
- **Evidence:** When a user deletes an MCP server, or disables it, to revoke a leaked token, Otto
  drops the Keychain copy. A plaintext copy stays in one `launch-*.env` per cwd the server was ever
  launched in. Nothing removes these files: there is no sweep, they are not tied to a session, and
  they are not removed on delete or disable. The files are 0600 in the per-user temp dir, so other
  local users cannot read them. But this goes against "Secrets never live [outside the Keychain]",
  and the user-facing guide misstates where the secret now lives.
- **Fix:**
  - In `reconcile`, delete `launcher_env_path(workspace_root, name)` for every `prior` name that is
    no longer desired, and for any desired server whose env is now empty.
  - In the delete route, and on a disable PATCH, sweep `<tmp>/otto-mcp/launch-*.<safe-name>-*.env`.
  - Optionally, sweep `launch-*.env` files older than N days at daemon boot.
  - Update the `DbMcpServerProvider` doc comment and `connections-ssh-sftp.md` §7 to describe the
    0600 env-file mechanism.
  - Add a test: reconcile with a server, reconcile without it, and assert its env file is gone.
