# Iteration 6, wave 0: integrations lens

Scope: how robust Otto's external integrations are. That covers provider CLIs and MCP injection,
the outward MCP self-call budgets, git/PR providers, Jira/Confluence, Slack/Telegram, SSH/DB,
Kafka, the browser sidecar, and AWS/k8s. I only read the code. I also read real local
transcripts (`~/.claude/projects`, `~/.grok/sessions`) and the installed `codex --help` and
`claude` binary strings to check the CLI shapes the code parses. I ran no builds or tests.
Base: `fix/quality-iter6-20261023` @ 02a64019b.

## Score

Start at 10.0. Open findings: 0 blocker, 1 major, 2 minor, 1 nit.

`10.0 − 0.3 − 2 × 0.1 − 0.03 = **9.47**`

## Status of the previous findings (iteration 5)

| id | status | evidence (current code) |
|----|--------|-------------------------|
| integrations-5-01 | fixed | `crates/otto-issues/src/jira/create.rs:99-113` `get_json` goes through `self.t.send_raw` and maps 401 and rate limits through `self.t.fail`. Createmeta 403 → `status_error` (`:133-143`), no longer "does not exist". The POST uses `send_raw` + `t.fail` (`:399-405`). Wire tests are at `:703-750`. |
| integrations-5-02 | fixed | `crates/otto-k8s/src/monitor/scrape.rs:590` reads the port-forward body with `read_body_capped` (`:638-650`) and cuts it with `cut_at_char_boundary` (`:623-631`). It is shared with the proxy path (`:668`). |
| integrations-5-03 | fixed | `crates/otto-apiclient/src/grpc.rs:751-757` `client_tls_config` adds `with_native_roots()` + `with_webpki_roots()`. Test at `:1275-1280`. The optional `tls_verify=false` toggle was not done, but the UI shows no Settings tab for gRPC (`RequestBuilder.svelte:461`), so no toggle misleads the user. |
| integrations-5-04 | fixed | `crates/otto-channels/src/slack.rs:103` `UPLOAD_TIMEOUT = 120 s`. `build_upload_client` (`:158-170`) is used by `put_bytes_to_upload_url` (`:588-590`). |
| integrations-5-05 | fixed | `crates/otto-channels/src/mirror.rs:1101-1125` `upload_or_note` retries once on a hint ≤ 60 s, then posts "⚠️ Otto could not attach …". Tests at `:1400-1421`. |

Previous findings: 5 fixed, 0 still open, 0 rejected.

## Areas covered in this iteration

- Re-verified all five iteration-5 fixes (table above).
- The outward governed self-call budget (`otto-mcp/src/outward/exec.rs::call_timeout`, enforced
  in `otto-server/src/mcp_outward.rs:1336-1350`) against the provider flows it wraps:
  - Jira create (createmeta walk, parent lookup, POST, verify).
  - Confluence create, update and comment (write, full-width properties, read-back).
  - GitLab rebase-merge polling, Bitbucket 202 merge polling, GitHub create PR.
  - Design assist (detached, fine) and wait_session (≤ 25 s, fine).
- `turn_oracle.rs` user-turn detection against real claude transcripts. This was the open item
  from iteration 5. Checked `isMeta` skill expansions, `isCompactSummary`, and cross-session
  message lines.
- Slack/Telegram final-reply delivery (`ReplyDelivery`, pending-reply queue), Slack
  `completeUploadExternal` error variants, Telegram `getUpdates` 401/409 handling, and
  `send_formatted` fallbacks.
- k8s monitor `gateway.rs`: the `kubectl proxy` lifecycle, readiness, restart triggers and the
  fingerprint vs the EKS token overlay (`eks_token.rs`). Also `classify`/`health` stderr
  variants and `actions.rs` argv planning.
- gRPC transport parity with REST: there is no SSH-tunnel route, but the UI hides "Send through"
  for gRPC (`RequestBuilder.svelte:1145`), so nothing is ignored silently. Not a defect.
- `otto-ssh/sftp.rs` + `otto-connections/sftp_pool.rs` after a network change (ServerAlive,
  ConnectTimeout, `kill_on_drop`). These are fine.
- DB drivers' pool staleness: Postgres `idle_timeout` and sqlx `test_before_acquire`, ClickHouse
  `is_closed`, the Redis retire/reconnect. These are fine.
- `cli_update.rs` provider updaters: `codex update` exists in the installed codex, there are
  version probes, and `kill_on_drop` plus timeouts.
- Email (Gmail SMTP via lettre, 60 s default transport timeout), the Kafka schema registry
  (negative cache, netguard, tunnel bypass), and the AWS CLI/native SigV4 expiry
  classification.
- Grok usage parser vs a real `updates.jsonl` `turn_completed` line (field names match).
- Browser live SOCKS guard proxy (`live/proxy.rs`) handshake/connect timeouts and connection cap.
- Jira search on `/search/jql` with `nextPageToken`, with the classic fallback.

## Not covered yet (for the next iteration)

- Claude's `.mcp.json` project-server approval state (`enabledMcpjsonServers` /
  `enableAllProjectMcpServers`). The claude 2.1.309 binary has a "pending" status for
  unapproved project servers, and Otto writes no approval for its managed `otto` entry. I
  could not show from code alone whether a fresh workspace's first claude session blocks on
  the approval dialog or silently runs without Otto's tools.
- `kill_on_drop` on `$SHELL -l -c <updater>`: is a grandchild (`npm i -g …`) left running after
  the 10 min timeout?
- `otto-browser/live/session.rs` (1.7k lines) CDP reconnect after a Chromium crash.
- AWS `sso login` PTY flow and `accounts.rs` SSO export.
- GitHub `create_pr` under secondary rate limits (reviewers/labels follow-up calls).

## Findings

### integrations-6-01: major: the outward self-call budget (30 s) is shorter than the multi-request merge and Atlassian write flows it wraps, so the agent is told "timed out" after the write has landed or half-landed

- **Where:**
  - `crates/otto-mcp/src/outward/exec.rs:31-49` `call_timeout`: `merge_pr`, `create_issue`,
    `create_confluence_page`, `update_confluence_page` and `comment_confluence_page` all fall
    into `_ => 30`.
  - `crates/otto-server/src/mcp_outward.rs:1336-1350`: `tokio::time::timeout(budget, call)` →
    `"self-call: timed out after 30s"`. Dropping the self-call drops the HTTP request to the
    route.
  - The flows behind those routes:
    - `crates/otto-git/src/providers/gitlab.rs:96-114` `wait_for_rebase`: `POLLS = 60` × 1 s
      (`:42`), called by `merge_pinned` (`:716-726`) *between* the rebase PUT and the merge PUT.
    - `crates/otto-git/src/providers/bitbucket.rs:1123-1126`: `MERGE_POLL_BUDGET = 60 s` for a
      202 merge (`:836`).
    - The route is not detached: `crates/otto-git/src/http.rs:3093-3096` awaits
      `merge_pinned` inline.
    - `crates/otto-issues/src/jira/create.rs`: createmeta types (`:183`, 1 to 10 pages), then
      createmeta fields (`:269`, 1 to 10 pages), then the parent lookup (`:353`), then the POST
      (`:401`), then `verify_created` (`:430`). That is at least 4 sequential Atlassian
      requests. Each one has its own 30 s `REQUEST_TIMEOUT` (`atlassian.rs:29,52`), and a GET
      can add a ≤ 10 s rate-limit wait (`atlassian.rs:38,247-257`).
    - Confluence create, `crates/otto-issues/src/http.rs:615-630`: `create_page` POST, then 2
      `set_full_width` property POSTs (`confluence.rs:264-291`), then the `verify_page` GET
      (`confluence/verify.rs:48-62`).
    - Confluence update (`http.rs:653`): GET version, then PUT, then the verify GET.
    - Confluence comment (`http.rs:720-721`): POST, then the verify GET.
- **Evidence:**
  - The 30 s outer budget equals one inner request's own timeout. Any slow request, a single
    rate-limit wait, or GitLab's asynchronous rebase (which routinely takes several seconds to
    tens of seconds on a busy instance) exceeds it.
  - When the budget fires, the agent gets an error with no key, page id or merge status, while:
    - (a) Jira or Confluence has already created the issue, page or comment. The verify read was
      the step that ran over. The agent's natural retry creates a duplicate outward-facing
      artifact.
    - (b) For GitLab rebase-merge, dropping the route mid-`wait_for_rebase` leaves the MR
      rebased (head rewritten, CI re-triggered) but not merged.
    - (c) For a Bitbucket 202, the merge completes server-side while the agent is told it failed.
  - These are the governed, approval-gated tools. The human approved one merge or one issue,
    and the retry produces a second one.
  - The other outer layers already allow longer: `GOVERNED_RUN_BUDGET = 190 s`
    (`otto-core/src/mcp_budgets.rs`), `GATEWAY_CALL_TIMEOUT` 65 s, and codex
    `tool_timeout_sec` 960 s. Only the per-tool `call_timeout` is short. This is the same class
    as integrations-3-01 and 4-01, now at the innermost layer.
- **Fix:**
  - Give these tools budgets that cover their flows, in `call_timeout`:
    - `merge_pr` → 90 (60 s poll + the merge + headroom).
    - `create_issue` → 120.
    - `create_confluence_page`, `update_confluence_page`, `comment_confluence_page` → 90.
    - `create_pr` → 60 is a reasonable precaution.
  - Keep each budget ≤ `GOVERNED_RUN_BUDGET`, and extend the `call_timeout` test
    (`outward/tests.rs:1448`) to assert each of these covers its provider's poll constant.
  - Separately, make the write and verify steps survive a dropped caller. Run the post-write
    part in a `tokio::spawn` and await its handle, so a timeout after the POST can still
    return the created key. Otherwise put the created key in the timeout error ("created
    PROJ-123; verification did not finish").

### integrations-6-02: minor: claude's `isMeta` user lines (skill expansions, compaction summaries, cross-session messages) count as the user's latest prompt, so the submit-confirm loop can miss its own prompt and paste it again into a working agent

- **Where:**
  - `crates/otto-agent-run/src/turn_oracle.rs:426-456` `user_prompt_text`: it accepts any
    `message.role == "user"` with text and never checks `isMeta` / `isCompactSummary`.
  - The same applies to `feed_line` `:270-273`, which sets `last_user_text` and increments
    `user_turns`.
  - The same rule is in `crates/otto-orchestrator/src/claude_pty.rs:603-640` `last_user_text`.
  - Consumer: `crates/otto-server/src/agent_session.rs:510-547`. It confirms only when
    `user_text_has(scan.last_user_text, needle)`. Otherwise it calls `submit_once` again every
    `RESUBMIT_EVERY = 8 s` (`:81`), up to `MAX_SUBMIT_ATTEMPTS = 4`, and then fails with "agent
    never accepted the prompt".
- **Evidence:**
  - Real claude 2.1.x transcripts write the Skill tool's expansion as
    `{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"Base directory for this skill: …"}]}}`
    right after the prompt. In an Otto insights session
    (`~/.claude/projects/-Users-itziklavon-Library-Application-Support-Otto-insights/305106fe-….jsonl`)
    the prompt is line 8 at 02:36:46.552 and the meta skill body is line 36 at 02:36:48.789,
    2.2 s later.
  - A scan of the 150 most recent local transcripts found plain prompts followed by these user
    lines: meta "Base directory for this skill" ×9, meta `[Image: source: …]` ×9, meta "Another
    Claude session sent a message" ×5, and one `isCompactSummary`.
  - The confirm loop polls every `POLL = 1 s` (`agent_session.rs:73`), and each poll returns the
    cumulative scan.
  - Failure path: a prompt that makes claude invoke a skill first is common under the user's
    "invoke skills before any response" setup and in workflow steps that name a skill. If the
    model's first tool call lands before the next poll (sub-second, or when polls are delayed
    under load), the first scan that sees the prompt already has the skill body as
    `last_user_text`. The needle is never found, and the prompt is re-pasted into the
    now-working session at 8, 16 and 24 s. The agent receives the same instruction again as
    queued messages, and the step may fail after the confirm window.
  - `crates/otto-sessions/src/titles.rs:77` already skips `isMeta` for the same reason.
- **Fix:**
  - In `user_prompt_text` (and `claude_pty::last_user_text`), return `None` when
    `v["isMeta"] == true` or `v["isCompactSummary"] == true`, mirroring `titles.rs:77`.
  - Add a `turn_oracle` test: prompt line, then an `isMeta` skill-body user line. Assert that
    `last_user_text` is the prompt and that `user_turns == 1`.

### integrations-6-03: minor: a long chat reply whose `investigation.md` attachment fails for any reason other than a rate limit is never posted, not even its head

- **Where:**
  - `crates/otto-channels/src/mirror.rs:1171-1196` `ReplyDelivery::attempt`: for text over
    `LONG_REPLY_THRESHOLD = 1800` chars (`:61`), it calls `adapter.upload(…)?` first. The
    `?` returns before `send_formatted` ever runs.
  - On that error the queue (`:513-526`) retries only `FeedSend::RateLimited`. Every other error
    posts the generic notice "Otto could not confirm delivery of the full reply. Open the
    session in Otto…" and drops the reply.
  - `post_reply` (`:1200-1220`, the improvement-summary path) does the same.
- **Evidence:** these upload failures are permanent or slow, and none of them is a rate limit:
  - Slack `missing_scope` (an app installed before `files:write` was added).
  - Slack `not_allowed_token_type`.
  - Telegram "not enough rights to send documents" in a group that restricts media, or
    "Request Entity Too Large".
  - An upload timeout (`slack.rs:103`, `telegram.rs` `UPLOAD_TIMEOUT`).

  The text half has not been sent, so posting it cannot duplicate anything. The design comment
  only guards against re-sending *accepted* parts. The chat user loses the whole answer to a
  failure in the optional attachment.
- **Fix:**
  - In `attempt`, when the upload fails with a non-rate-limit error, mark the attachment as
    abandoned and still send the head. Use the suffix "⚠️ the full reply could not be attached
    ({short error}); open the session in Otto for the rest." in place of "📎 full reply
    attached".
  - Keep the existing retry path for rate limits.
  - Add a stub-adapter test: upload fails with `missing_scope`, and exactly one
    `send_formatted` carries the head and the note.

### integrations-6-04: nit: the k8s monitor gateway's documented "restart after a 401" does not exist, and a 401 from metrics-server is reported as "cluster RBAC"

- **Where:**
  - `crates/otto-k8s/src/monitor/gateway.rs:24-27` (module doc: "the caller restarts it when that
    changes, when the child exits, or after a 401").
  - `usable_for` (`:198-208`) checks only that the child is running and that the fingerprint
    matches. `ensure` (`:349-364`) has no status input.
  - `crates/otto-k8s/src/monitor/collector.rs:686-691` maps `401 | 403` to
    `Error::Forbidden("cluster RBAC: metrics-server answered HTTP 401")`.
- **Evidence:**
  - No caller drops `state.gateway` on a 401 (`rg "gateway = None"` finds nothing).
  - A proxy whose loaded credential is rejected keeps answering 401 until the kubeconfig mtime
    changes. The status row then blames RBAC rather than an expired credential.
  - Impact is low: the EKS overlay is re-minted before expiry, which changes the mtime and
    restarts the proxy.
- **Fix:** either implement it (on a gateway 401, set `state.gateway = None` so the next
  `ensure` restarts it, and word 401 as "credentials rejected") or correct the module doc.
