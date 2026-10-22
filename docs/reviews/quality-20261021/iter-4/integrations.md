# Iteration 4: integrations lens

Scope: how robust the external integrations are. That covers the provider CLIs and MCP launcher
injection (claude / codex / agy / grok), the stdio MCP bridge and the governed gateway, git/PR
providers, Jira/Confluence, Slack/Telegram, SSH/DB connections, Kafka, the browser sidecar, and
AWS/k8s. I only read the code. I ran no builds or tests.
Base: `fix/quality-loop-20261021` @ 6cff08270 (includes the iteration-3 fixes e1c4a8f29 / 00943460a).

## Score

Start at 10.0. Open findings: 3 major (−0.3 each) and 3 minor (−0.1 each).

`10.0 − 0.3 − 0.3 − 0.3 − 0.1 − 0.1 − 0.1 = **8.80**`

## Status of the previous findings

| id | status | evidence (current code) |
|----|--------|-------------------------|
| integrations-3-01 | fixed | `crates/ottod/src/mcp_tools/budgets.rs` adds `wait_session_budget` (the clamped `timeout_secs`, 1..=25, plus a 5 s margin) and `SUMMARIZE_CALL_TIMEOUT` (15 min). `read_route` attaches the wait budget through `.within(wait_session_budget(args))` (`mcp_tools.rs:2563-2572`). `browser_summarize` uses `post_json_within(…, SUMMARIZE_CALL_TIMEOUT)` (`:4024-4030`). `get_json_within` and `post_json_within` set a per-request `.timeout(budget)` (`:214-283`), and the client no longer has a client-wide timeout (`:4729-4731`). |
| integrations-3-02 | fixed | `merge_polling` (`crates/otto-git/src/providers/bitbucket.rs:216-275`) handles a 202. It resolves `Location` against the merge URL and accepts it only when it is same-origin. It polls `task_status` until it reads `SUCCESS` (Ok) or `FAILED` (`Conflict` carrying Bitbucket's reason). If the budget runs out, it returns a `Conflict` saying the merge has not finished. Wire tests are in `mod merge_202_tests` (`:1309+`), including a cross-origin `Location`. |
| integrations-3-03 | fixed | Reconcile now calls `remove_launcher_env` for every prior name that has gone (`crates/otto-sessions/src/mcp.rs:330-339`) and for a server whose env is now empty (`:363-367`). The delete route calls `sweep_env_files` (`crates/otto-server/src/routes/mcp_servers.rs:292-313`), which wraps `sweep_launcher_env_files` (`mcp.rs:454`). The doc comment now describes the 0600 env-file mechanism (`mcp_servers.rs:325-330`). The test `reconcile_and_sweep_remove_stale_launcher_env_files` (`mcp.rs:1812`) covers the cleanup. |

Previous findings: 3 fixed, 0 still open, 0 rejected.

## Areas covered in this iteration

- Re-checked all three iteration-3 fixes against the code, as the table above shows.
- The stdio bridge's other proxies. I compared the budget of each upstream call with the daemon
  pipeline it waits on:
  - `gateway_call` against `/mcp/gateway/invoke` and the `otto-mcp` client's `OP_TIMEOUT`, slot
    queue, cold `open` and `CLIENT_IDLE_TTL`.
  - `governed_call` and `governed_native_write`, whose budget is `GOVERNED_WAIT_SECS` plus the run
    budget.
  - The DB tools (`otto_db_*`).
- The outward exec `call_timeout` table (`crates/otto-mcp/src/outward/exec.rs`) and how
  `wait_session` is bounded on that side (30 s default, against a daemon cap of 25 s; it is fine).
- `otto-dbviewer` `mcp-query` end to end: `McpQueryReq` → `run_read_only` → `run_inner` (inline,
  request-scoped execution) → the MySQL, Postgres and ClickHouse drivers' timeout handling. I also
  checked whether a dropped request cancels its query (no `Drop` cancel exists).
- `otto-brokers` consumer-group offset reset: the service, the HTTP handler and audit, and
  `reset_offsets`. I checked them against librdkafka 2.12.1's `rd_kafka_handle_OffsetCommit` error
  folding, using the vendored source in the cargo registry.
- MCP launcher formats:
  - The agy `.agents/mcp_config.json` reconcile from the recent commit 8a9897823 (shared
    `reconcile_json_launcher`, `info/exclude` coverage through `LAUNCHER_ARTIFACTS`).
  - The codex `-c mcp_servers.*` overrides. I checked them against the installed codex 0.163.0's
    `RawMcpServerConfig` keys (`tool_timeout_sec` and `startup_timeout_sec` are present).
- `otto-agent-run/turn_oracle.rs` codex scanning (`feed_codex_line`, ordinals). I checked it
  against a real codex 0.163.0 rollout on this machine: `ordinal` and the
  `task_started`/`task_complete` event names are present. It is fine.
- MCP server secret resolution (`load_secret_blob`, `write_secret_env`, `DbMcpServerProvider`).
- The Atlassian transport (`atlassian.rs` `send`, Jira search token walk) compared with the git
  providers' rate-limit classification.
- Telegram `poll_once`: token redaction, and 401/409 marked permanent. Slack socket mode: hello,
  disconnect, the zombie watchdog and acks. Both are fine.
- The browser sidecar: lightpanda spawn, readiness, supervisor, and the CDP `Fetch` netguard
  vetting. It is fine.
- Schema-registry client: netguard, TLS-or-loopback, Basic auth. It is fine.

## Not covered yet (for the next iteration)

- agy's MCP per-tool timeout, and whether agy honours any timeout key in
  `.agents/mcp_config.json`. This is the agy counterpart of integrations-4-03.
- `turn_oracle.rs` agy transcript variants, and claude transcript edge cases such as
  compaction and resumed sessions.
- `otto-connections/http.rs` session lifecycle (PTY reconnect after the machine sleeps).
- The grok `[mcp_servers.*]` TOML launcher against the current grok CLI reader.
- Linear and Confluence rate limits (429) and mid-walk behaviour under a 429.
- The outward `mcp_http` JSON-RPC transport under client disconnect: does a dropped HTTP client
  cancel the self-call?
- Slack/Telegram file-upload error variants (`files.getUploadURLExternal`, `sendDocument` 413).

## Findings

### integrations-4-01: major: the stdio bridge gives governed gateway calls 20 s, less than the pipeline they proxy needs

- **Where:**
  - `crates/ottod/src/mcp_tools.rs:4301`: `gateway_call` uses
    `ctx.post_json("/mcp/gateway/invoke", &body)`, so it gets the fixed `CALL_TIMEOUT = 20 s`
    (`:118`, `:263-264`).
  - `crates/otto-mcp/src/client.rs:51`: `OP_TIMEOUT = 20 s`.
  - `client.rs:269-275`: the slot queue waits up to `OP_TIMEOUT`.
  - `client.rs:446-456`: a cold `open` (spawn plus `initialize`) gets up to `OP_TIMEOUT`.
  - `client.rs:408-412`: `tools/call` itself gets up to `OP_TIMEOUT`.
  - `crates/otto-mcp/src/service.rs:59`: `CLIENT_IDLE_TTL = 5 min`, so cold opens are common.
  - `service.rs:977-990,1203-1215`: the audit row is inserted first, and `PendingAuditRow` marks
    it failed when the future is dropped.
- **Evidence:** every downstream `mcp__<server>__<tool>` call a session makes goes through
  `gateway_call`.
  - The daemon side allows 20 s for the `tools/call` alone. On top of that it allows up to 20 s
    for a cold spawn/initialize, which follows any 5-minute idle, and up to 20 s in the admission
    queue.
  - The bridge gives up at 20 s for the whole request.
  - So a tool that answers in 15 s after a 6 s `npx` cold start succeeds on the daemon side, but
    the agent gets `upstream timeout after 20s`.
- **Why it is worse than a wrong error:** the dropped HTTP request drops the axum handler future
  in the middle of `call_tool_checked`.
  - The request may already be on the downstream server's stdin when this happens.
  - For a mutating downstream tool (a Jira/GitHub MCP "create"), the side effect can happen while
    the `PendingAuditRow` drop marks the governed audit row failed.
  - The agent is told the call timed out, so it retries and duplicates the write.
- **Background:** this is the same class as integrations-3-01. That fix gave per-route budgets to
  `otto_wait_session` and `browser_summarize` only. The governed `otto.*` proxy already uses
  `GOVERNED_WAIT_SECS + 190` (`:4338-4345`), but the downstream gateway proxy was missed.
- **Fix:**
  - Use `post_json_within("/mcp/gateway/invoke", &body, GATEWAY_CALL_TIMEOUT)`, where the budget
    is at least queue + open + call + margin (for example 3 × `OP_TIMEOUT` + 5 s = 65 s). Put the
    constant next to `OP_TIMEOUT` in `otto-mcp` so the two cannot drift.
  - Add a bridge test with a stub daemon that answers `/mcp/gateway/invoke` after 21 s. Assert
    that the agent gets the executed envelope.

### integrations-4-02: major: `otto_db_query` sets no server-side time limit, so a query the bridge gave up on keeps running on the user's database

- **Where:**
  - `crates/otto-dbviewer/src/http.rs:609-641`: `McpQueryReq` has no timeout, and `mcp_query`
    builds `QueryRequest { …, ..Default::default() }`, so `timeout_ms: None`.
  - `crates/otto-dbviewer/src/service.rs:2657`: passes `timeout_ms` through.
  - `service.rs:2314-2319`: without a `query_id`, the query runs inline ("request-scoped execution
    (agents over MCP, widgets) stays inline"). No `Drop` cancels it: the only `impl Drop`s in the
    crate are the `InFlightGuard` of the detached path, the cache, and test types.
  - `crates/otto-dbviewer/src/drivers/mysql.rs:706-709`: `deadline = None`, so no
    `MAX_EXECUTION_TIME`.
  - `crates/otto-dbviewer/src/drivers/postgres.rs:1656-1659`: `RESET statement_timeout`, which
    falls back to the role default, usually unlimited.
  - `crates/otto-dbviewer/src/drivers/clickhouse.rs:126`: the default `max_execution_time` is
    55 s.
  - The caller: `crates/ottod/src/mcp_tools.rs:3492-3494`, `post_json` (20 s). The outward
    `otto.query_db_readonly` gets `call_timeout`'s 30 s default (`crates/otto-mcp/src/outward/exec.rs:60`).
- **Evidence:** an agent sends a heavy `SELECT` (a large `GROUP BY` or a join on an unindexed
  column).
  - The bridge returns `upstream timeout after 20s`. Dropping the request drops the handler and
    the sqlx future.
  - MySQL and Postgres keep running the statement until they next write to the socket, which for
    an aggregate is when it finishes. ClickHouse runs on to 55 s.
  - Nothing issues `KILL QUERY` or `pg_cancel_backend`.
  - The agent is told it timed out, so it usually retries or rephrases, and each retry stacks
    another runaway query on what is often a shared or production database.
- **Note:** the UI path avoids this, because it uses a `query_id`, detached execution and a
  cancel route. The agent path has neither a cancel nor a server-side bound.
- **Fix:**
  - In `run_read_only`, force `timeout_ms = Some(min(req.timeout_ms.unwrap_or(MCP_QUERY_TIMEOUT_MS), MCP_QUERY_TIMEOUT_MS))`
    with `MCP_QUERY_TIMEOUT_MS` below the bridge budget (for example 15 000). The engines then stop
    the statement themselves: the MySQL hint, Postgres `SET LOCAL statement_timeout`, ClickHouse
    `max_execution_time`, and the Redis/Mongo client deadline.
  - The agent then gets a clean "query timed out" message instead of a transport error.
  - Optionally, add a drop guard on the inline path that cancels through the `CancelToken`'s
    recorded handle.
  - Add a driver-level test that `run_read_only` always sends a bounded timeout.

### integrations-4-03: major: codex sessions cut every Otto MCP tool off at codex's 60 s default, below the budgets of the long tools

- **Where:**
  - `crates/otto-sessions/src/mcp.rs:619-630` (`codex_mcp_inject_args`): emits only
    `mcp_servers.otto.command` and `.args`.
  - `:636-645` (`codex_browser_args`) and `:731-740` (`codex_user_server_args`): the same.
  - No `tool_timeout_sec` appears anywhere in `crates/` (`rg tool_timeout_sec` finds nothing).
  - The long tools on the bridge:
    - `SUMMARIZE_CALL_TIMEOUT = 15 min` (`crates/ottod/src/mcp_tools/budgets.rs:24`).
    - `TEXT_CALL_TIMEOUT = 65 s` for k8s logs (`mcp_tools.rs:136`).
    - The governed proxy, `GOVERNED_WAIT_SECS (15) + 190 s` (`:4338-4345`). Its run budgets
      include `api_run_automation` 180 s and `start_pr_review`/`run_workflow`/`open_session` 60 s
      (`otto-mcp/src/outward/exec.rs:55-59`).
- **Evidence:**
  - Codex applies a per-tool `tool_timeout_sec` to every MCP call, 60 s by default. The installed
    codex 0.163.0 binary still carries the `tool_timeout_sec` key in its `RawMcpServerConfig`, and
    Otto never sets it.
  - So in a codex session:
    - `browser_summarize` (a whole agent turn, budgeted 15 min) fails at 60 s, while the
      summarizer keeps working and its result is thrown away.
    - A governed `otto_start_pr_review` or `otto_run_workflow` that first waits up to 15 s on an
      approval and then runs for up to 60 s fails.
    - So does `otto_api_run_automation` (up to 180 s) and a k8s log fetch near its 60 s kubectl
      budget.
  - The bridge budgets were raised for exactly these routes in iteration 3, but for codex the
    client-side cap is now the binding one. The agent sees a codex timeout, and a mutating
    governed call may still complete afterwards, which invites a duplicate retry.
- **Fix:**
  - Append `-c mcp_servers.otto.tool_timeout_sec=<N>` in `codex_mcp_inject_args`, with N at least
    the largest bridge budget (for example 960, which covers `SUMMARIZE_CALL_TIMEOUT` plus
    margin). Do the same for `otto-browser`.
  - For user servers, pass through a configured value, or use a generous default.
  - Derive N from the bridge constants (export them from `ottod`, or put them in `otto-core`) so
    they cannot drift.
  - Extend the argv test to assert the key is emitted.

### integrations-4-04: minor: a consumer-group offset reset neither checks the group state nor records an audit entry when it partly fails

- **Where:**
  - `crates/otto-brokers/src/service.rs:846-925` (`reset_group_offsets`): describe → resolve →
    commit, with no check of `detail.state`.
  - `crates/otto-brokers/src/kafka.rs:1414-1429`: a member-less `commit(…, CommitMode::Sync)`.
  - `crates/otto-brokers/src/http.rs:496-505`: `audit_write` runs only after `?` succeeds.
- **Evidence:**
  - Kafka accepts a member-less offset commit only for an `Empty` group. An active (`Stable`)
    group answers `UNKNOWN_MEMBER_ID` per partition, and a rebalancing group answers
    `REBALANCE_IN_PROGRESS`.
  - Otto never checks `state` (`rg "Stable|Empty" crates/otto-brokers/src` finds nothing), and
    the dry-run preview does not warn either. The user confirms a guarded reset and gets a raw
    librdkafka "Broker: Unknown member" error, where Kafka's own tooling says "the group is
    active; stop its consumers first".
  - Partial failure is the worse case. With per-topic ACLs, one topic can be denied
    (`TOPIC_AUTHORIZATION_FAILED`) while the others commit.
  - librdkafka 2.12.1 (`rdkafka_request.c`, `rd_kafka_handle_OffsetCommit`) returns that
    partition error without rolling the others back. So `reset_offsets` returns `Err`, and the
    handler's `?` skips `audit_write`.
  - The result is a destructive write that was partly applied to a guarded cluster and left no
    audit row.
- **Fix:**
  - Refuse the reset unless `detail.state` is `Empty` or `Dead`, with a clear message. Show the
    same warning in the dry-run response.
  - Write the audit entry whether the commit succeeds or fails, with the outcome and the
    per-partition errors. To get those, read the `TopicPartitionList` errors after `commit`
    instead of only the folded error.

### integrations-4-05: minor: a Keychain read failure for an MCP server's secrets is treated as "no secrets", which silently starts the server without them and can wipe them on the next edit

- **Where:** `crates/otto-server/src/routes/mcp_servers.rs`:
  - `:73-80` (`load_secret_blob`: `.get(..).ok().flatten()…unwrap_or_else(|| json!({}))`), used
    at spawn by `DbMcpServerProvider::enabled_servers` (`:362-366`).
  - `:93-98` (`write_secret_env`, the same swallow).
  - `:105-112` (the "KEY= keeps it" sentinel resolves against the empty `previous`).
  - `:120-123` (an empty merged env with no headers → `delete_async` of the Keychain entry).
- **Evidence:**
  - **At spawn:** a locked or denied Keychain (for example `errSecInteractionNotAllowed` after
    sleep) is mapped to `{}`. The server is launched without its token, with no warning. That
    breaks the `McpServerSource` contract (`crates/otto-sessions/src/mcp.rs:155-160`: "the lookup
    failed must stay distinct from nothing enabled"), and the agent sees confusing downstream auth
    errors.
  - **On a PATCH:** the UI sends `KEY=` sentinels for unchanged secrets. When the read fails, the
    sentinels resolve to nothing. The blob is rewritten without the stored values and without the
    Control-Plane-owned `headers`. If every key was a sentinel, the Keychain entry is deleted
    outright.
  - A transient read error therefore destroys the user's stored credentials.
- **Fix:**
  - Make `load_secret_blob` return `Result`. Treat `Err` and unparsable JSON as a lookup failure:
    `enabled_servers` → `Err`, so reconcile preserves the entries, and the PATCH → 503 "Keychain
    unavailable", with nothing written.
  - Map only `Ok(None)` to `{}`.

### integrations-4-06: minor: the Jira/Confluence transport has no rate-limit or auth-expiry handling, unlike the git providers

- **Where:**
  - `crates/otto-issues/src/atlassian.rs:158-173` (`send`: any non-2xx →
    `upstream_status` → `Error::Upstream`, which is HTTP 502 per
    `crates/otto-server/src/error.rs:33`).
  - `crates/otto-issues/src/jira.rs:557-571` (the search token walk fails the whole search on the
    first non-2xx page, with the body uncapped).
  - Compare `crates/otto-git/src/providers/client.rs:454-500` (`rate_limit_wait` /
    `rate_limited_err`) and `:1014-1040` (401/403 → `Forbidden` with a check-your-token hint).
- **Evidence:**
  - Jira Cloud answers bursts with `429` plus `Retry-After`, which the story watcher, JQL "load
    more" windows and multi-page searches all produce. Otto retries nothing and does not read the
    wait.
  - The user sees a 502 "jira search failed (429 Too Many Requests): …", and a page-3 failure
    throws away pages 1-2.
  - An expired or revoked API token (`401`) is also a 502 "upstream" error, not a 403 the UI can
    turn into "reconnect this account".
  - The git providers solved exactly this, so the two integration families now behave
    differently for the same failure.
- **Fix:**
  - Move `rate_limit_wait` into a shared helper, or copy it into `atlassian.rs`.
  - On 429 (or 503 with `Retry-After`), retry reads once when the wait is short (≤ 10 s, as the
    git client does). Otherwise return `Error::Upstream("jira rate limited — retry in Ns")`.
  - Map 401 and 403 to `Error::Forbidden` with an "update the Jira token" hint.
  - Cap the `jira.rs:567-571` body with the existing `BODY_SNIPPET_MAX` snippet.
