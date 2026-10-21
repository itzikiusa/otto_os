# Iteration 1 — integrations lens

Reviewer scope: external integration robustness (provider CLIs + MCP config injection, git/PR
flows, Jira/Confluence/Linear, Slack/Telegram, SSH/DB, Kafka, browser sidecar, AWS/k8s,
control-plane ↔ session tool-registry parity). Read-only; no builds/tests run.
Base: `fix/quality-loop-20261021` @ d447516c4 (includes PR #114 "give agy sessions their MCP servers").

## Score

Start 10.0. Open findings: 3 major (−0.3 each), 5 minor (−0.1 each).

`10.0 − 3×0.3 − 5×0.1 = 10.0 − 0.9 − 0.5 = **8.6**`

## Previous-finding status

First iteration for this lens — no previous findings.

## Areas covered

- `otto-sessions`: `mcp.rs` (all launcher reconciles: `.mcp.json`, agy `.agents/mcp_config.json`,
  grok TOML, codex `-c` overrides, retire), `manager.rs` spawn/restart MCP wiring
  (`maybe_enable_otto_tools`, `sync_workspace_mcp`), `providers.rs` (built-ins, alias/shell
  fallback), `agy.rs` capture, `accounts.rs`.
- `ottod/src/mcp_tools.rs`: governed stdio-name mapping, `tools/list`, gateway proxy, enabled
  cache; diffed against `otto-mcp/src/outward/catalog.rs` (aliasing table covers the
  control-plane → session parity; no missing tool found).
- `otto-mcp/src/client.rs` (stdio/HTTP downstream client, handshake, base env).
- `otto-git`: `commit_all_if_dirty`, GitHub provider create/merge paths, shared client
  (rate-limit/ETag cache); `workflow_engine.rs` git_pr node; `run_service.rs` proof gate.
- `otto-issues/src/linear.rs` (new tracker: auth header, error mapping, cursor walks), Jira search
  fallback.
- `otto-channels`: Telegram long-poll loop + error classification, Slack directory.
- `otto-connections/builders.rs`, `otto-ssh` tunnel args; `otto-aws/cli.rs` stderr classification;
  `otto-k8s/eks_token.rs` refresh; `otto-browser/lightpanda.rs` sidecar supervisor.
- UI: `ui/src/modules/agents/NewSession.svelte` browser toggle vs daemon wiring.

## Areas NOT covered yet (next iteration)

- `otto-brokers` (Kafka proxy runtime, SASL/SSL, schema registry), `otto-dbviewer` drivers
  (Mongo/ClickHouse/Redis error variants), SFTP pool.
- Bitbucket/GitLab provider write paths in depth, `transport.rs`/askpass token expiry on push.
- Confluence pager + ADF conversion, Jira transitions/createmeta variants.
- Slack Socket Mode reconnect (`slack.rs` ws loop), `bridge.rs`/`mirror.rs` send limits
  (UTF-16 length vs `FEED_CHAR_BUDGET`), email/webhook channels.
- `otto-browser/live/*` (Chromium runtime, proxy), `otto-k8s` actions/pod_http/logs,
  `otto-aws` native SigV4 paths, S3 streams.
- `otto-orchestrator` headless claude PTY parsing, `otto-agent-run` result-file recovery,
  codex/agy transcript parsing (`otto-transcript`), usage tailer agy native reader.
- `cli_update.rs` provider update commands; `routes/provider_accounts.rs` login flows.

## Findings

### integrations-1-01 — major — agy (and grok) launcher files are swept into workflow commits, secrets included

- **Where:** `crates/otto-git/src/local.rs:3422-3432` (`commit_all_if_dirty`), callers
  `crates/otto-server/src/workflow_engine.rs:6261` (git_pr node) and `:8150` (run reap);
  writer `crates/otto-sessions/src/manager.rs:2745` → `crates/otto-sessions/src/mcp.rs:110`
  (`agy_mcp_path`), env written at `mcp.rs:240`, secret values merged at
  `crates/otto-server/src/routes/mcp_servers.rs:356`.
- **Evidence:** PR #114 now reconciles the managed set into `<cwd>/.agents/mcp_config.json`
  on every agy spawn (the `otto` entry is always present because `otto_mcp` defaults ON). User
  servers are written with their `env`, and `DbMcpServerProvider::enabled_servers` overlays the
  Keychain secret values onto that env (`merge_secret_env`). `commit_all_if_dirty` runs
  `git add -A -- . :(exclude).mcp.json :(exclude).env :(exclude).env.* …` — only `.mcp.json` is
  treated as an Otto runtime artifact; `.agents/mcp_config.json` (and grok's
  `.grok/config.toml`, same story) are not excluded. A workflow whose agent node uses
  `provider: agy` runs in the run worktree, so the git_pr node's leftover sweep commits the
  file (absolute `ottod` path + resolved secret env of every enabled user server) and pushes
  it in the PR; the reap path commits it onto `otto-wf/<run_id>`.
- **Fix:** keep one list of Otto launcher artifacts in `otto-sessions::mcp` (e.g.
  `pub const LAUNCHER_FILES = [".mcp.json", ".agents/mcp_config.json", ".grok/config.toml"]`)
  and use it for the `:(exclude)` pathspecs here and in `run_service::check_proof_revision`
  (01/02). Better still, have the reconcile add these root-anchored paths to the repo's
  `info/exclude` (as `otto-swarm::exclude_helpers` does for its helpers) so a user/agent
  `git add -A` cannot pick them up either. Add a `commit_all_if_dirty` test with an untracked
  `.agents/mcp_config.json`.

### integrations-1-02 — major — Otto Runs on agy can never open a PR (proof-revision gate)

- **Where:** `crates/otto-server/src/run_service.rs:554-570` (`check_proof_revision`).
- **Evidence:** the gate rejects any change except untracked `.mcp.json`, `.env`, `.env.*`,
  `.DS_Store`. `LocalGit::status` uses `--untracked-files=all`
  (`crates/otto-git/src/local.rs:1637`), so an agy run worktree always reports untracked
  `.agents/mcp_config.json` (written at spawn, `manager.rs:2745`). `changes` is therefore true
  and the function returns `Conflict("run work changed since proof …")` no matter how often the
  user re-assembles proof — the run provider is resolved per workspace
  (`run_service.rs:70`), so an agy default makes Open PR permanently unavailable.
- **Fix:** match the shared launcher-file list from 01 (untracked only, so tracked edits still
  block), plus a test with an untracked `.agents/mcp_config.json`.

### integrations-1-03 — major — "Browser tools" toggle is a no-op for Codex sessions (and hidden for agy, which now supports it)

- **Where:** `ui/src/modules/agents/NewSession.svelte:292,356`;
  `crates/otto-sessions/src/manager.rs:2654-2762` (`sync_workspace_mcp`);
  `crates/otto-sessions/src/mcp.rs:206-213` (browser entry only in the JSON launchers).
- **Evidence:** the UI shows the toggle for `claude || codex` and sends `meta.browser = true`
  for codex. The daemon writes the `otto-browser` server only into `.mcp.json` (and, since #114,
  agy's `.agents/mcp_config.json`); for codex it returns only `codex_user_server_args` — there is
  no `-c mcp_servers.otto-browser.*` override anywhere (`grep browser_command` has no other
  caller). Codex does not read `.mcp.json` (stated in the same function), so the codex session
  gets no browser server, while its `mcp_servers` meta snapshot (`manager.rs:2726-2738`) still
  lists `otto-browser`. Personal agents set `browser: agent.browser` for any provider
  (`crates/otto-server/src/assistant_host.rs:310`), so a codex personal agent is affected too.
  Conversely agy now honours `cfg.browser` but the toggle excludes it.
- **Fix:** emit the browser server for codex the same way as user servers
  (`-c mcp_servers.otto-browser.command=… -c mcp_servers.otto-browser.args=[…]` when
  `cfg.browser`), and widen `supportsBrowser` to `claude | codex | agy`; make the snapshot
  per-provider (see 05).

### integrations-1-04 — minor — Codex user-server secret env values are passed on argv

- **Where:** `crates/otto-sessions/src/mcp.rs:350-379` (`codex_user_server_args`).
- **Evidence:** `-c mcp_servers.<name>.env={"KEY"="<value>"…}` is appended to the codex argv
  with the Keychain-resolved values (`merge_secret_env`). The same module deliberately keeps the
  per-session otto token off argv ("the token never touches argv", `codex_mcp_inject_args`), but
  user-server API keys are visible in `ps -ww` for the session's lifetime.
- **Fix:** reuse the creds-file mechanism: write the user servers' env into the per-session
  0600 file and point codex at it (or put secret keys in the codex process env and use codex's
  `env_vars` passthrough list instead of literal values).

### integrations-1-05 — minor — Codex silently drops user servers whose name isn't a bare TOML key, yet the session snapshot claims them

- **Where:** `crates/otto-sessions/src/mcp.rs:355-358`; snapshot at
  `crates/otto-sessions/src/manager.rs:2726-2738`.
- **Evidence:** a server named e.g. `GitHub Prod` or `jira.cloud` is skipped for codex with only
  a `tracing::debug!`; the create route accepts any non-empty name. The `mcp_servers` meta is
  taken from the `.mcp.json` reconcile result for every provider, so the session records the
  server as wired. Users see the server enabled and listed for the session but the codex agent
  never gets its tools, with no visible reason.
- **Fix:** return the names actually delivered per provider (codex: the names emitted by
  `codex_user_server_args` + `otto`; agy: the agy reconcile's result) for the snapshot, and log
  skipped names at `warn` (or reject such names at create/rename time with a clear message).

### integrations-1-06 — minor — Downstream MCP handshake rejects any server that negotiates a different protocol version

- **Where:** `crates/otto-mcp/src/client.rs:41,572-575`.
- **Evidence:** the client requests `2024-11-05` and fails the handshake unless the server
  answers with exactly that string. Under MCP version negotiation a server that no longer
  supports `2024-11-05` replies with its own (newer) version; Otto then reports
  `initialize: unsupported or missing protocolVersion` and the server is unhealthy/unusable
  instead of proceeding (or at least naming the offered version).
- **Fix:** accept a known-compatible set (`2024-11-05`, `2025-03-26`, `2025-06-18`), send the
  `MCP-Protocol-Version` header on subsequent HTTP requests when the negotiated version
  requires it, and include the server's offered version in the error otherwise.

### integrations-1-07 — minor — stdio MCP server failures lose their diagnostic (stderr discarded)

- **Where:** `crates/otto-mcp/src/client.rs:458-475` (`.stderr(Stdio::null())`), error at `:978`.
- **Evidence:** a stdio server that exits on startup (missing env var, bad token, `npx` package
  not found) writes the reason to stderr; Otto discards it and surfaces only
  `server closed before responding` in health/discovery, so the user cannot tell what to fix.
- **Fix:** pipe stderr into a bounded ring buffer (e.g. last 4 KiB, redacted with
  `otto_core::redact`) drained by a task, and append its tail to handshake/EOF errors.

### integrations-1-08 — minor — Lightpanda restart truncates the crash log it should preserve

- **Where:** `crates/otto-browser/src/lightpanda.rs:156-160` (`File::create(&log_path)`), called
  by the supervisor respawn at `:240`.
- **Evidence:** every respawn after an unexpected exit opens `lightpanda.log` with
  `File::create`, truncating the output of the instance that just crashed — the only evidence
  of why the sidecar died (the supervisor logs only "exited unexpectedly"). Secondary: if a
  respawned child stays alive but never passes `wait_ready`, the loop goes back to
  `c.wait()` on that child and never retries.
- **Fix:** open with `OpenOptions::new().create(true).append(true)` (rotate at a size cap, or
  rename the previous log to `lightpanda.log.1` before respawn); on a failed `wait_ready`
  after respawn, kill the child before looping.
