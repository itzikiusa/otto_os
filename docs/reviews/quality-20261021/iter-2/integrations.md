# Iteration 2 — integrations lens

Reviewer scope: external integration robustness (provider CLIs + MCP config injection, git/PR
flows, Jira/Confluence, Slack/Telegram/webhook channels, SSH/DB connections, Kafka, browser
sidecar, AWS/k8s, control-plane ↔ session tool-registry parity). Read-only; no builds/tests run.
Base: `fix/quality-loop-20261021` @ ab0a86f05 (includes the iteration-1 repairs, 66b55a25c).

## Score

Start 10.0. Open findings: 1 major (−0.3), 2 minor (−0.1 each), 1 nit (−0.03).

`10.0 − 0.3 − 2×0.1 − 0.03 = 10.0 − 0.53 = **9.47**`

## Previous-finding status

| id | status | evidence (current code) |
|----|--------|-------------------------|
| integrations-1-01 | fixed (core) | `otto_core::paths::LAUNCHER_ARTIFACTS` (`crates/otto-core/src/paths.rs:12`) is excluded by `commit_all_if_dirty` (`crates/otto-git/src/local.rs:3474-3486`). The optional `info/exclude` part was explicitly not done (iter-1 FIXES "Not done") — the residual risk is reported as integrations-2-01 with new evidence. |
| integrations-1-02 | fixed | `is_untracked_runtime_file` (`crates/otto-server/src/run_service.rs:557-564`) ignores launcher artifacts only while untracked. |
| integrations-1-03 | fixed | `codex_browser_args` (`crates/otto-sessions/src/mcp.rs:~389`) emitted for `session_browser` in `sync_workspace_mcp` (`crates/otto-sessions/src/manager.rs:2826-2829`); UI `BROWSER_PROVIDERS = ['claude','codex','agy']` (`ui/src/modules/agents/NewSession.svelte:293`). |
| integrations-1-04 | fixed | user-server env goes to a 0600 file + `/bin/sh` `ENV_EXEC_SCRIPT` wrapper (`mcp.rs:442-545`); files removed on respawn/retire (`manager.rs:188-201,2837,6715`). |
| integrations-1-05 | fixed | per-provider `delivered` snapshot (`manager.rs:2787-2858`); skipped names logged at `warn` (`mcp.rs:452-458`). |
| integrations-1-06 | fixed | `SUPPORTED_PROTOCOL_VERSIONS` + `MCP-Protocol-Version` header (`crates/otto-mcp/src/client.rs:42-48,190`). |
| integrations-1-07 | fixed | stdio stderr piped into `StderrTail` ring, redacted tail appended to errors (`client.rs:106-170,554-573`). |
| integrations-1-08 | fixed | lightpanda log opened `append(true)` with 4 MiB rotation; a respawn that fails `wait_ready` is killed (`crates/otto-browser/src/lightpanda.rs:156-170,267-277`). |

Previous findings: 8 fixed / 0 still open / 0 rejected.

## Areas covered (this iteration)

- Re-verification of all 8 iteration-1 fixes (sessions/MCP launchers, run proof gate, MCP client,
  lightpanda).
- `otto-channels`: Slack Socket Mode loop end to end (`apps.connections.open` error classes,
  dial timeout, watchdog/ping, `disconnect` reasons, admission/ACK ordering, dedup), Slack/Telegram
  `send`/`send_formatted`/`edit`, Telegram envelope decode + 429/`retry_after`, mirror long-reply
  split + rate-limit retry, bridge relay preamble, seeded reply instructions, webhook adapter
  (SSRF-guarded client, contract in `docs/contracts/api.md:1995-2000`).
- `otto-git`: askpass helper + `TransportCredential` origin confinement, Bitbucket API-token
  username, `hardened_command`, GitLab create/merge/rebase-pinned merge, GitHub create_pr /
  reviewers / CI rollup (`fetch_ci_status_for_sha`, `checks_from_check_runs`,
  `checks_from_statuses`, `ci_from_checks`), Bitbucket/GitLab CI fetch, CI consumer
  (`routes/proof.rs` ci refresh).
- `otto-issues`: Jira transitions, create_issue, issue types, `search/jql` token walk + classic
  fallback, JQL escaping; Confluence client (TLS gate, `_links.next` same-site resolution, create /
  update with 409 → Conflict, full-width property).
- `otto-brokers`: librdkafka config (security protocol, SASL mechanisms, vendored SSL), SSH Kafka
  proxy protocol (`ApiVersions` clamp, non-flexible Metadata/FindCoordinator rewrite).
- `otto-connections/builders.rs`: ssh/mysql/redis/mongo/clickhouse/postgres command lines, jump-host
  wrapping (shell-quoting, env dropping, UI hint for prompts); `otto-ssh` tunnel base args.
- `otto-k8s`: kubectl runner (timeouts, capped reads, stderr classification incl. credential
  rejection, AWS key redaction), exec-plugin allow-list (`kubeconfig_policy.rs`), pod_http proxy →
  port-forward fallback and mutating-retry guard.
- `otto-aws/native.rs`: SigV4 signing, endpoint/CA-bundle CLI fallback, transport back-off.
- `otto-dbviewer` ClickHouse JSONCompact streaming + mid-stream exception decode.
- `cli_update.rs` (per-provider updater, version probe, kill_on_drop, idle-deferred reloads),
  provider account status/login args.
- MCP parity: `GOVERNED_BY_STDIO_NAME` / aliasing in `crates/ottod/src/mcp_tools.rs` and the new
  twin-schema test (`mcp_tools/tests/catalog_tests.rs`); outward JSON-RPC version negotiation.
- Codex transcript record types (`otto-transcript/src/codex.rs`), agy native usage reader.

## Areas NOT covered yet (next iteration)

- `otto-dbviewer` Mongo / Redis / MySQL / Postgres driver error variants and the SSH-tunnel pool
  they share; `otto-ssh/sftp.rs` pool staleness.
- `otto-browser/live/*` session/guard proxy runtime (only the launcher was read).
- `otto-k8s` logs streaming (`logs.rs`) and `actions.rs` command construction.
- `otto-aws` S3 streaming/download, Athena/Logs Insights CLI fallbacks, SSO export in `accounts.rs`.
- `otto-orchestrator` headless claude PTY (`claude_pty.rs`) and `otto-agent-run` result recovery.
- Bitbucket provider write paths (create_pr/merge) in depth; GitLab `wait_for_rebase` timeouts.
- Email channel (`email.rs`), `improve_notify.rs`.
- Claude `.mcp.json` project-server approval behaviour vs prompt-guard (not verified here).

## Findings

### integrations-2-01 — major — Launcher files carrying resolved MCP secrets are written world-readable into the work tree and nothing keeps them out of git

- **Where:** `crates/otto-sessions/src/mcp.rs:262-271` (JSON launchers put each user server's
  `env` into the entry), `:647` (grok `[mcp_servers.<name>].env`), `:103-115`
  (`.mcp.json` / `.agents/mcp_config.json` at the workspace root), `:706-730`
  (`write_doc` → `atomic_write` → `std::fs::write`, default umask ⇒ 0644); the values are the
  Keychain-resolved secrets (`merge_secret_env`, `crates/otto-server/src/routes/mcp_servers.rs:376-389`).
- **Evidence:** every claude/agy/grok spawn reconciles the enabled user servers, secrets included,
  into `<cwd>/.mcp.json`, `<cwd>/.agents/mcp_config.json` or `<cwd>/.grok/config.toml`. Only the
  Otto workflow auto-commit excludes these paths (`local.rs:3474-3486`); the reconcile never adds
  them to `.git/info/exclude` (no `exclude`/`gitignore` handling anywhere in `otto-sessions`), and
  the iteration-1 FIXES record this part as "Not done". Agents are explicitly told to commit their
  own work (`local.rs:3462-3466` doc comment), so an agent's `git add -A && git commit` in a run
  worktree or in the user's checkout — or the user's own "stage all" — commits API keys of every
  enabled MCP server, and the PR flow pushes them. If the repo already tracks a team `.mcp.json`,
  Otto rewrites a TRACKED file with secrets and `git diff` / any commit carries them. The files are
  also created 0644 while the codex counterpart (`write_env_file`, `mcp.rs:509-545`) and the codex
  creds file are 0600 — the same secret has two protection levels depending on the provider. This
  contradicts AGENTS.md "Secrets never live in the repo".
- **Fix:** (a) on every reconcile, append the root-anchored `LAUNCHER_ARTIFACTS` paths to the repo's
  `$(git rev-parse --git-common-dir)/info/exclude` (idempotent, as `otto-swarm` does for its
  helpers); (b) create launcher files with mode 0600 (`OpenOptions::mode(0o600)` for the temp file
  in `atomic_write`); (c) better, keep secrets out of the work tree entirely by reusing the codex
  mechanism for JSON/TOML launchers too — write `command: "/bin/sh", args: ["-c", ENV_EXEC_SCRIPT,
  <0600 env file under temp_dir/otto-mcp>, <command>, …]` and omit `env`. Add a test that a
  reconcile with a secret env never writes the value under the cwd.

### integrations-2-02 — minor — PR CI rollups read only the first page of checks, so a failing check past it reads as green

- **Where:** `crates/otto-git/src/providers/github.rs:286-302` (`fetch_ci_status_for_sha`: one
  `get_cached_json` each for `check_runs_path` / `statuses_path`, `:306-318`, `per_page=100`, no
  `Link`/`total_count` follow-up); `crates/otto-git/src/providers/bitbucket.rs:234-262`
  (`pagelen=50`, no `next`); consumer `crates/otto-server/src/routes/proof.rs:1077-1090` records the
  rollup as a proof-pack CI artifact.
- **Evidence:** the GitHub check-runs response carries `total_count`, but only `check_runs[]` of
  page 1 is folded; the Statuses API returns EVERY status update newest-first (pending → success
  are separate rows), so on a busy commit a context whose latest row is past the 100th is dropped
  entirely before `checks_from_statuses` de-dups. `ci_from_checks` then reports `success` when
  every row it saw passed. Large matrix builds (>100 check runs) or many status updates therefore
  yield a false-green CI summary in the PR view and in proof evidence. The same crate already
  paginates elsewhere (`paginate_json_cached`, used for GitLab MR commits, `gitlab.rs:771-781`).
- **Fix:** follow `Link: rel="next"` for check-runs (or at least compare `total_count` with the rows
  read and return `pending`/"incomplete" when short); use the combined
  `GET /commits/{sha}/status` (one row per context) instead of the raw statuses list; follow
  Bitbucket `next`. Add a wire test with a 2-page check-runs fixture whose failing run is on page 2.

### integrations-2-03 — minor — Agent replies are relayed to Slack/Telegram in GitHub-flavoured Markdown that neither channel renders

- **Where:** `crates/otto-channels/src/slack.rs:449-472` (`send_formatted` posts the text verbatim
  with `mrkdwn: true`), `crates/otto-channels/src/telegram.rs:324-360` (`parse_mode: "Markdown"`
  legacy, falls back to plain text on a parse error); the relay preamble
  `crates/otto-channels/src/bridge.rs:1186-1213` and the seeded instructions
  `crates/otto-channels/src/seed.rs:18-23` never name the target dialect.
- **Evidence:** the mirror sends the agent's final message (or its `⟦otto-send⟧` blocks) through
  `ReplyDelivery::attempt` → `send_formatted` (`mirror.rs:1141-1165`) without any conversion
  (`rg mrkdwn crates/otto-channels/src` finds only these doc comments). Claude/Codex answer in GFM:
  `**bold**`, `[label](url)`, `## headings`, pipe tables. Slack mrkdwn shows those as literal
  asterisks/brackets/hashes; Telegram legacy Markdown rejects common agent text (an unpaired `_`
  in `snake_case`, `**`) with "can't parse entities", so the fallback posts the raw markup as plain
  text. The doc comments claim "`*bold*` … render", which only holds for text the agent was never
  told to produce.
- **Fix:** convert GFM before sending — Slack: `**x**`→`*x*`, `[l](u)`→`<u|l>`, headings→bold
  lines, tables→code block; Telegram: render to `parse_mode: "HTML"` with escaped text (far more
  forgiving than legacy Markdown). Alternatively state the dialect in the relay preamble. Unit-test
  a typical Claude answer through each converter.

### integrations-2-04 — nit — GitHub `stale` check-run conclusion keeps the CI state pending forever

- **Where:** `crates/otto-git/src/providers/github.rs:358-371` (`checks_from_check_runs`), folded by
  `ci_from_checks` (`:323-339`).
- **Evidence:** the match maps `success`/`neutral`/`skipped`, the failing conclusions, and
  `_ => "pending"`. GitHub marks a check run that sat incomplete for 14 days `status: completed,
  conclusion: stale` — terminal — so a PR carrying one shows "pending" indefinitely and never
  rolls up to success or failure.
- **Fix:** treat `("completed", "stale")` as its own terminal state (count it as not-passed, e.g.
  `failure` or a distinct `stale` the UI renders) and add it to the wire test table.
