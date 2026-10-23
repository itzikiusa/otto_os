# Iteration 6, final review: integrations lens

Scope: how robust Otto's external integrations are. That covers provider CLIs and MCP injection,
the outward MCP self-call budgets, the parity between the MCP control-plane catalog and the session
tool registry, git/PR providers, Jira/Confluence, Slack/Telegram, SSH/DB, the browser sidecar, and
AWS/k8s. I only read the code. I also checked the installed claude 2.1.309 binary's bundled JS for
the `.mcp.json` approval rule and the project-directory encoder. I ran no builds or tests.
Base: `fix/quality-iter6-20261023` @ 2065f00e9 (`git diff origin/main`: 864 files).

## Score

Start at 10.0. Open findings: 0 blocker, 0 major, 2 minor, 1 nit.

`10.0 − 2 × 0.1 − 0.03 = **9.77**`

## Status of the wave-0 findings

| id | status | evidence (current code) |
|----|--------|-------------------------|
| integrations-6-01 (major) | fixed | `crates/otto-mcp/src/outward/exec.rs:62-96` `call_timeout`: `merge_pr` 90, `create_issue` 120, the three Confluence writes 90, `create_pr` 60. All are ≤ `GOVERNED_RUN_BUDGET` (190 s, `otto-core/src/mcp_budgets.rs:32`). `timeout_error` (`exec.rs:103-127`) names the target and says it "may already have completed upstream … a retry can create a duplicate" for every `MULTI_REQUEST_WRITES` tool (`:42-49`). `mcp_outward.rs:1429-1442` uses it. The stdio `ottod mcp-server` allows `call_timeout + 15 s` (`mcp_server.rs:180-183`). The session bridge allows `15 s + 190 s` (`mcp_tools.rs:1315-1321`). Tests pin each budget against the provider constants (`outward/tests.rs:1544-1559`) and pin the message (`:1578-1594`). |
| integrations-6-02 (minor) | fixed | `crates/otto-transcript/src/turn.rs:346-349` `user_prompt_text` returns `None` for `is_meta_line` (`:440-444`: `isMeta` / `isCompactSummary`). The fold uses it (`:194-196`), so `last_user_text` and `user_turns` both skip injected lines. Same filter in `otto-orchestrator/src/claude_pty.rs:609-617`. Tests are at `turn.rs:878-884` and `claude_pty.rs:916-920`. |
| integrations-6-03 (minor) | fixed | `crates/otto-channels/src/mirror.rs:1189-1224` `ReplyDelivery::attempt`: a non-rate-limit upload error sets `attach_failed`, and the head is still sent with "⚠️ the full reply could not be attached ({why}); open the session in Otto for the rest." A rate limit still returns `Err` for the retry path. `post_reply` (`:1227-1245`) shares it. The test asserts the exact note (`:1419-1429`). |
| integrations-6-04 (nit) | fixed | `crates/otto-k8s/src/monitor/collector.rs:698-705` maps 401 to `Error::Unauthorized`. The cycle sets `gateway_rejected` (`:930`), and `drop_rejected_gateway` (`:731-737`, called at `:1058`) drops the `kubectl proxy`, so the next `ensure` restarts it. The status reads "credentials rejected … restarting the proxy" (`:713`), not RBAC. The module doc (`gateway.rs:24-29`) now matches. Test `gateway_401_is_credentials_rejected_not_rbac` (`:1363`). |

Previous findings: 4 fixed, 0 still open, 0 partially fixed, 0 rejected as invalid.

## Areas covered

- Re-verified all four wave-0 fixes (table above), including the outer layers around the new
  budgets:
  - The stdio `invoke_timeout` sends no `wait_seconds`, so there is no approval wait inside it.
  - The session bridge `governed_call` uses wait + `GOVERNED_RUN_BUDGET`.
  - `governed_native_write` budgets match the daemon's `call_timeout` per tool.
- MCP parity (commit 1ce84030a): the session twins are now generated from the control-plane
  specs.
  - `ROUTED_TWINS` / `twin_route` / `twin_call` / `twin_read` (`ottod/src/mcp_tools/dispatch.rs`).
  - `check_string_args`, which keeps the native type refusals.
  - `native_refs.rs` resolution, and the routing and parity tests
    (`every_control_plane_tool_is_in_the_session_listing_as_specified`,
    `query_db_reaches_sessions_through_the_governed_bridge`).
  - I compared each moved twin's budget with its native arm on `origin/main`: `open_session`,
    `send_message` and `comment_pr` used `CALL_TIMEOUT` then too, so the move changed no budget.
- `otto.query_db`:
  - `db_query_call` read and write routing.
  - `QUERY_DB_TIMEOUT_MS` (60 s) under a 75 s self-call budget.
  - Driver deadline handling: MySQL `run_tracked`, Postgres `statement_timeout`, ClickHouse
    `max_execution_time`, the mongosh script timeout, and the Redis timeout.
- `ottod mcp-server` MCP profiles (`--profile`). They filter on the client side only. The
  restricted token and the daemon's enabled list stay the real gate, so failing open on a failed
  profile read is not a security gap.
- Claude `.mcp.json` approval, which wave-0 left open. From the claude 2.1.309 bundle, the
  approval logic (`Ipt` → `gat`) works like this:
  - A project server that is still pending is approved "by bypass" when the session runs under
    `--dangerously-skip-permissions` and the user has accepted the bypass prompt.
  - Under skip-permissions, Otto's default (`providers.rs:233`), the `otto` entry therefore loads
    without a dialog.
  - With skip-permissions off, the claude approval dialog shows in the interactive session. That
    session needs a human for tool prompts anyway, so this is not a defect.
- `cli_update.rs`, which wave-0 left open. The built-in updaters are single commands (`claude
  update`, `codex update`, `agy update`, `providers.rs:262,292,323`). `kill_on_drop`, plus a
  10 min / 20 s timeout, covers the process the login shell runs. No npm grandchild tree is
  involved.
- GitHub `create_pr` under rate limits, which wave-0 left open. The reviewer request is
  best-effort and becomes `reviewer_warnings` (`github.rs:874-904`). Writes are re-sent only
  after a quota refusal or a connect error, never after a 5xx (`client.rs:594-641`).
- AWS `sso login` PTY flow, which wave-0 left open.
  - `accounts.rs:1237-1282` evicts the cached creds at spawn.
  - `exported_creds` (`:976-1048`) evicts on `LoginRequired` rather than backing off. The back-off
    only falls back to per-call profile resolution.
- Browser live session crash handling. The process watcher marks sessions crashed
  (`process.rs:469-475,595`), and `open` replaces crashed leftovers (`runtime.rs:336-338`).
- Session submit `RunGuard` (`otto-sessions/src/submit.rs`): it is checked before the paste, the
  re-paste, Enter and the re-Enter. The early-return paths leave no stray Enter.
- Transcript roots (`otto-transcript/src/roots.rs`): the account home, `$CODEX_HOME`,
  `~/.claude-*` and the isolated fixture. Also `transcript_path_in` and nested-agent capture
  across roots (`nested.rs`).
- Agent-run submit-confirm loop (`otto-agent-run/src/session_turn.rs:482-575`).
- git error classification split (`otto-git/src/local/errors.rs`). The `base_cmd` pins
  `LC_ALL=C`, so the English markers hold.
- Telegram poller `KeyedLocks`, the Slack doc change, and the Confluence nested-panel depth cap
  (`confluence.rs:801-835`).
- Codex MCP injection (`otto-sessions/src/mcp.rs` `codex_mcp_inject_args`,
  `codex_user_server_args`, `skipped_servers`).
- REST execute engine move: SSH SOCKS tunnel cache (`otto-apiclient/src/engine/tunnel.rs`). The
  code moved verbatim.

## Not covered

- `crates/otto-usage/src/tailer/driver.rs` (167 changed lines): the root rediscovery and backfill
  semantics under the new `RootKind`.
- `crates/otto-server/src/mcp_http.rs` (the HTTP MCP transport) end-to-end budgets.
- Kafka / schema registry and the SSH/DB connection pools: no changes this iteration, not
  re-read.
- Whether Confluence storage→Markdown converts nested panels correctly. The first-close-tag
  match pre-dates this iteration; I only checked the new depth bound.
- Whether hyper drops an in-flight axum handler when the bridge disconnects. This affects how
  often finding 6f-02 duplicates rather than half-applies.

## Findings

### integrations-6f-01: minor: Otto's Claude project-directory encoder never applies claude's 200-character cap and hash, so in a long cwd the transcript is looked up in a directory claude never writes, and an agent turn re-pastes its prompt and then fails

- **Where:**
  - `crates/otto-transcript/src/roots.rs:46-50` `encode_cwd`: every non-ASCII-alphanumeric `char`
    becomes `-`, with no length cap.
  - Users of it:
    - `roots.rs:315-323` `claude_session_file`: returns the primary root's expected path when no
      file exists.
    - `otto-sessions/src/lifecycle.rs:39,45,186` (`claude_project_dir_name` alias,
      `claude_transcript_path`, `transcript_path_in_roots`).
    - `otto-sessions/src/nested.rs` `claude_transcript_in_window`.
    - `otto-context/src/materialize.rs:147,400-404` (the context dir and `memory/MEMORY.md`).
    - `otto-improve/src/pathsafe.rs:19`.
  - Consumer that fails: `otto-agent-run/src/session_turn.rs:490-575`. `tpath` is resolved once
    before the submit, and `can_confirm = tpath.is_some()` is true.
- **Evidence:**
  - claude 2.1.309's encoder (bundled JS) is:
    `var $Se=200; function k(e){return e.replace(/[^a-zA-Z0-9]/g,"-")}
    function wH(e){let n=k(e);if(n.length<=$Se)return n;return`${n.slice(0,$Se)}-${Te(e)}`}`.
  - `Te` is `Math.abs(hash(e)).toString(36)`, and the projects dir is
    `function Ofn(e){return Qi(Om(),wH(e))+nn}`.
  - So a cwd whose encoded name is longer than 200 characters is stored as
    `<first 200>-<base36 hash>`. Otto looks for the untruncated name.
  - Also, the JS regex works on UTF-16 code units, so a non-BMP character (an emoji in a folder
    name) becomes `--` in claude and `-` in Otto.
  - Failure path: in such a cwd, `claude_session_file` returns the never-written expected path.
    `poll_claude_tail` sees nothing, and the confirm loop:
    - re-pastes the same prompt into the working agent at 8, 16 and 24 s
      (`RESUBMIT_EVERY`, `MAX_SUBMIT_ATTEMPTS = 4`);
    - then fails with "agent never accepted the prompt (stuck on a startup screen?)".
  - Every claude workflow, assist or product turn in that directory fails this way. The
    `lifecycle.rs:192-198` read_dir fallback cannot help, because it scans only
    `<root>/*/<psid>.jsonl` and `transcript_path_in` is not the path the oracle uses.
  - Otto's own scratch and scheduled-task worktrees stay under 200 characters. The trigger is a
    deep user repository path.
- **Fix:**
  - Mirror claude in `encode_cwd`: encode per UTF-16 code unit (`cwd.encode_utf16()`, each
    non-`[A-Za-z0-9]` unit → `-`). When the result is longer than 200 characters, keep the first
    200 and append `-` plus the base-36 absolute value of claude's string hash over the original
    cwd.
  - Port the hash from the bundle (`Cpe`, the 32-bit `(h<<5)-h+code` string hash) and pin it with
    a golden test against a real long-path directory name.
  - Until then, make `claude_session_file` fall back to scanning `<root>/*/<psid>.jsonl`, as
    `transcript_path_in_roots` already does, on every poll while the file is missing. A known
    `psid` then finds the transcript whatever the directory name.

### integrations-6f-02: minor: the PR-comment write (session twin `otto_comment_pr`, outward `otto.comment_pr`) has a 20 s / 30 s budget, shorter than its own route can legitimately take, and its timeout does not warn that the comment may already be posted

- **Where:**
  - Session bridge:
    - `crates/ottod/src/mcp_tools/dispatch.rs:194` (`otto_comment_pr` is a `ROUTED_TWINS` write).
    - `:345` `twin_call` → `ctx.post_json`.
    - `crates/ottod/src/mcp_tools.rs:278-280` → `CALL_TIMEOUT` = 20 s
      (`otto-core/src/mcp_budgets.rs:15`).
  - Outward: `crates/otto-mcp/src/outward/exec.rs:93` (`comment_pr` falls into `_ => 30`). It is
    missing from `MULTI_REQUEST_WRITES` (`:42-49`), so `timeout_error` returns only
    "self-call: timed out after 30s".
  - The route:
    - `crates/otto-git/src/http.rs:3036-3048` awaits `provider.comment` inline.
    - GitHub inline comment, `crates/otto-git/src/providers/github.rs:953-967`: `pr_raw` GET for
      the head sha, then the POST.
    - Each request has its own 20 s timeout (`providers/client.rs:515`).
    - A 429 / secondary-rate-limit refusal of any method, the POST included, is slept out for up
      to `RATE_LIMIT_MAX_WAIT = 30 s` and re-sent (`client.rs:447,620-631`).
- **Evidence:**
  - The route's own worst case is ≈ 20 + 30 + 20 s. One short `retry-after` on the POST is
    enough to exceed the 20 s bridge budget. GitHub's secondary content-creation limit is the
    usual case when a review agent posts a burst of inline comments, and posting PR comments is
    what the review workflow does (`dispatch.rs:190-193`).
  - When the bridge gives up, the agent gets a bare timeout and no hint. Its natural retry then
    posts a second copy of an outward-facing comment if the first copy landed after the wait.
  - This is the same class as integrations-6-01, which the wave-1 fix closed for the six
    multi-request writes but not for this one.
- **Fix:**
  - Give the session twin an explicit budget: route `otto_comment_pr` through `post_json_within`
    with ≈ 75 s (2 × 20 s requests + the 30 s rate-limit wait + headroom).
  - In `call_timeout`, set `"comment_pr" => 75`.
  - Add `comment_pr` to `MULTI_REQUEST_WRITES`, with a target of "comment on PR #{n} in repo
    {repo_id}", and have the session twin's timeout error carry the same "may already have been
    posted — list the PR comments before retrying" text.
  - Extend `outward/tests.rs:1544` to assert that `comment_pr` covers
    2 × request timeout + `RATE_LIMIT_MAX_WAIT`.

### integrations-6f-03: nit: the Codex "skipped MCP server" reason has 18 stray spaces in it and blames a missing key when the env file failed

- **Where:** `crates/otto-sessions/src/mcp.rs:681-701` `skipped_servers`.
- **Evidence:**
  - The literal at `:694` continues onto the next source line without a `\`, so the string the
    session meta's `mcp_skipped[].reason` stores is "…no free one was left" + 18 spaces +
    "for this server" (confirmed with `od -c`).
  - The branch is also chosen for any non-bare server name (`!is_bare_toml_key`). A server such as
    "Linear MCP" that did get a free key (`linear_mcp`) but whose env file write failed
    (`codex_user_server_args` `:783-788`) is reported as "no free one was left" rather than "Otto
    couldn't write its … private env file".
- **Fix:**
  - End the line with `\` (or use `concat!`) so the text has no gap.
  - Compute the codex-key reason from the same key assignment `codex_user_server_args` uses: have
    it return the skipped names with their reason. Only a name whose `codex_server_key` collided
    should get "no free key".
