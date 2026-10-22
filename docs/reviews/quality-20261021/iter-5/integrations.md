# Iteration 5: integrations lens

Scope: how robust Otto's external integrations are. That covers the provider CLIs and MCP
launcher injection, the stdio bridge budgets, MCP profiles, git/PR providers, Jira/Confluence/Linear,
Slack/Telegram, SSH/DB, Kafka, the k8s monitor, the gRPC API client and AWS/k8s. I only read
the code (plus `strings` on the installed agy and grok binaries to check their config readers).
I ran no builds or tests.
Base: `fix/quality-iter5-20261022` @ 8177c4d85.

## Score

Start at 10.0. Open findings: 0 blocker, 0 major, 5 minor (−0.1 each).

`10.0 − 5 × 0.1 = **9.50**`

## Status of the previous findings (iteration 4)

| id | status | evidence (current code) |
|----|--------|-------------------------|
| integrations-4-01 | fixed | `crates/otto-mcp/src/client.rs:60` `GATEWAY_CALL_TIMEOUT = 3 × OP_TIMEOUT + 5 s` (65 s), asserted at `:1162`. `gateway_call` passes it to the bridge request (`crates/ottod/src/mcp_tools.rs:4320`). A bridge test answers after 21 s (`mcp_tools/tests/budget_tests.rs:52,84`). |
| integrations-4-02 | fixed | `crates/otto-dbviewer/src/service.rs:115-123` (`MCP_QUERY_TIMEOUT_MS = 15 000`, `mcp_query_timeout_ms` clamps it and maps `None` or `0` to the cap). `run_read_only` always sets it (`:2674`). The drivers turn it into the MySQL deadline and Postgres `SET LOCAL statement_timeout` (`drivers/postgres.rs:1663-1666`). It is tested in `service/perf_budget_tests.rs:704-761`. |
| integrations-4-03 | fixed | `crates/otto-core/src/mcp_budgets.rs` is the one home for the bridge budgets. `CLIENT_TOOL_TIMEOUT_SECS` = summarize + 60 = 960 s, and a test checks it covers every budget. `codex_mcp_inject_args`, `codex_browser_args` and `codex_user_server_args` emit `mcp_servers.<key>.tool_timeout_sec` (`crates/otto-sessions/src/mcp.rs:637,660,761`), and the argv tests assert 960 and 600. grok's reader defaults `tool_timeout_sec` to 6000 s (grok 1.0.67 `strings`), so grok needs no key. |
| integrations-4-04 | fixed | `reset_group_offsets` refuses with 409 unless the group is `Empty` or `Dead` (`crates/otto-brokers/src/service.rs:895-897`, `reset_blocked_reason` at `:1675`). The dry run shows the same warning (`:1093`). The attempt is audited before the error propagates, with the outcome and per-partition errors (`:921-950`). |
| integrations-4-05 | fixed | `parse_secret_blob` (`crates/otto-server/src/routes/mcp_servers.rs:76-85`) maps only `Ok(None)` to `{}`. A failed read or an unparsable blob is an `Err`. The PATCH answers 503 `keychain_unavailable` (`:102-135`). |
| integrations-4-06 | fixed for the code it named | `AtlassianTransport::send_raw`, `check` and `fail` (`crates/otto-issues/src/atlassian.rs:222-285`) read the 429 / 503 `Retry-After`, retry a GET or HEAD once, and map 401/403 to `Forbidden`. The new Jira create module (commit 9d46b0845, merged after the fix) bypasses the transport again. That is reported as integrations-5-01. |

Previous findings: 6 fixed, 0 still open, 0 rejected.

## Areas covered in this iteration

- Re-checked all six iteration-4 fixes against the code, as the table above shows.
- New Jira create flow (`otto-issues/src/jira/create.rs`): createmeta walk, POST, read-back
  verification. Also its outward wiring: `route_for` `create_issue`, `list_issue_*`, and the
  `call_timeout` default of 30 s.
- Confluence validated storage bodies (c83cba144): `update_page` 409 → `Conflict`, `add_comment`
  through `send`, and the read-back verifier.
- MCP profiles end to end:
  - `OTTO_MCP_PROFILES` env (claude/agy/grok) and the codex creds file `profiles` key.
  - The bridge `enabled_path` sanitising, the `refuse` hook and the fail-open on an unreachable
    daemon (documented).
  - `GET /mcp/otto-server/enabled?profiles=` → `resolve_param`. An unknown id falls back to the
    default profile (`otto-state/src/mcp_profiles.rs:131-153`).
  - The outward `ProfiledTools` for `/mcp/http?profile=`.
- The outward MCP HTTP transport under client disconnect. `governed_invoke` inserts the audit row
  first and arms `PendingAuditRow`, so a dropped caller is recorded (it is fine).
- agy MCP config: the installed agy binary's docs list `.agents/` as a customization root that
  can carry `mcp_config.json`. Its `timeoutSeconds` is a server *launch* timeout. agy's per-call
  tool timeout cannot be confirmed from the binary, so there is no finding.
- grok `[mcp_servers.*]` TOML launcher against grok 1.0.67's reader: `command`, `args` and
  `enabled` are accepted, and `tool_timeout_sec` defaults to 6000 s. It is fine.
- Codex `tool_timeout_sec` value type: an integer TOML value deserialises into codex's
  `Option<f64>` field. It is fine.
- Linear GraphQL transport (`linear.rs` `gql` / `map_error`). Rate-limit and auth errors carry
  actionable text, so there is no finding.
- Slack Socket Mode events path: the ack is immediate for admitted work, and busy notices are
  acked after the notice with pending-key dedup. It is fine.
- Slack / Telegram attachment upload paths (`slack.rs` external upload, `telegram.rs`
  `sendDocument`, `mirror.rs` `upload_file_path`).
- k8s monitor (not covered before):
  - `scrape.rs` proxy, gateway and port-forward transports.
  - The `ForwardPool` lifecycle (`kill_on_drop`, readiness timeout, stdout drain).
  - The collector `run_loop` and its scheduler restart behaviour.
- gRPC API client (`otto-apiclient/src/grpc.rs`, not covered before): netguard pinning, deadlines
  and the TLS root store, compared with the REST client.
- GitHub merge (`merge_with`), including head-sha pinning and best-effort branch delete. It is
  fine.
- SSH interactive argv (`otto-connections/builders.rs`): ServerAlive options are added only when
  the user's ssh_config sets none. It is fine.
- `turn_oracle.rs` claude user-turn scan against real local transcripts. `[Image: source: …]`
  `isMeta` user lines follow an image prompt immediately. This is not reported: Otto's
  `agent_session` prompts are pasted text, and I could not show that an image attachment
  reaches that path.

## Not covered yet (for the next iteration)

- `turn_oracle.rs`: `isMeta` / `isCompactSummary` user lines feeding `last_user_text` and
  `user_turns`. Can any Otto submit path (slash-command prompts in workflow steps, or prompts
  that make claude attach an image) leave a meta line as the last user text and trigger the
  8 s re-submit?
- Outward `create_issue` under a slow Jira. The 30 s default `call_timeout` has to cover up to
  ~6 sequential Atlassian requests. What happens to a POST that is already sent when the
  self-call times out?
- The k8s monitor `gateway.rs` (`kubectl proxy` over a unix socket): restart and back-off, and
  the `classify.rs` / `health.rs` stderr variants.
- The gRPC client's lack of an SSH-tunnel / SOCKS route, which REST requests have
  (`api_client.rs` proxy).
- `otto-connections/http.rs` PTY reconnect after sleep, and the SFTP pool (`sftp_pool.rs`)
  after a network change.
- Slack `files.completeUploadExternal` error variants (`not_in_channel`, `invalid_thread_ts`).

## Findings

### integrations-5-01: minor: the new Jira create flow bypasses the Atlassian transport, so token expiry and rate limits get the pre-fix handling

- **Where:** `crates/otto-issues/src/jira/create.rs`
  - `:92-104` `get_json`: a raw `.send()` used for the createmeta pages, the parent lookup and
    the verify read.
  - `:123-138` status mapping.
  - `:371-391` the `POST /rest/api/3/issue`, also a raw `.send()`.
  - `:421-432` the verify read-back.
- **Evidence:**
  - integrations-4-06 made `AtlassianTransport::send_raw` / `check` the one path for Jira and
    Confluence calls (`atlassian.rs:222-285`): 429 or 503 with `Retry-After` → one GET retry, or
    "rate limited — retry in Ns"; 401/403 → `Forbidden` with a token hint. FIXES P09 says
    "every remaining Jira and Confluence call goes through the transport".
  - `create.rs` (commit 9d46b0845) only takes `self.t.http()` and classifies statuses itself:
    - createmeta `403` is reported as `Invalid("project '…' does not exist or this Jira account
      cannot create issues in it")`.
    - `401` (expired or revoked token) falls into `_ => Error::Upstream("jira createmeta
      failed (401 …)")`, a 502.
    - `429` is a 502 too, with no retry and no wait, even though the createmeta GETs are exactly
      the reads the transport would retry.
    - The create POST maps every non-2xx (401, 403, 429) to `Upstream`.
    - A `429` on the read-back GET makes `verification.ok=false` ("the issue was created but
      reading it back answered 429"). The user is told verification failed for an issue that
      is fine.
  - Result: `otto.create_issue` and the UI create form say "project does not exist" or
    "upstream failed" where every other Jira call says "update the Jira token" or "retry in
    Ns".
- **Fix:**
  - Route `get_json` through `self.t.send_raw(req, what)`. It keeps the raw status that the
    createmeta 400/404 branch needs, and gains the one GET retry.
  - Map 401/403 through `status_error`, or `self.t.fail(resp, what)`, before the createmeta
    `400 | 404` branch. Drop 403 from the "does not exist" arm.
  - Send the POST with `self.t.send_raw`, which never re-sends a write. Map its non-2xx through
    `self.t.fail` and keep `jira_error_text` for the 400 body.
  - Add a wire test: createmeta 401 → `Forbidden`, createmeta 429 + `Retry-After: 0` → retried.

### integrations-5-02: minor: on the port-forward transport the k8s monitor buffers whole probe bodies and can panic while cutting them, killing the collector loop

- **Where:**
  - `crates/otto-k8s/src/monitor/scrape.rs:590-594` (`get_probes`, used by every pooled and
    one-shot port-forward fetch, `:562`, `:790`):
    `match resp.text().await { Ok(mut body) => { if body.len() > MAX_BODY { body.truncate(MAX_BODY); } … }`.
  - Compare `fetch_proxy` at `:628-645`. It reads through `run_timeout_capped(…, MAX_BODY)` (the
    S6-09 fix: "a probe on `/actuator/heapdump` × the cycle's concurrency must not buffer whole
    bodies before the cap"), and it walks back to a char boundary before truncating.
- **Evidence:**
  - `String::truncate` panics when the new length is not a char boundary. `resp.text()` decodes
    lossily, so a binary or large body (a heapdump, or a non-UTF-8 `/metrics`) is full of 3-byte
    U+FFFD replacement characters. The 8 MiB cut then very likely lands inside one, and the
    task panics.
  - The body is also read in full before the cap. The port-forward transport is the one chosen
    when `pods/proxy` RBAC is denied ("Rancher-managed clusters in practice", `:5-10`), so the
    memory bound S6-09 added does not hold there.
  - The panic unwinds the `collector::run_loop` task (`otto-server/src/k8s_monitor_scheduler.rs:117`).
    The scheduler reaps it as "loop ended" and restarts it on the next scan. Every cycle then
    dies at the same pod, the status row never records an error, and the forward pool is torn
    down each time.
- **Fix:**
  - Read the port-forward response with a capped stream (`resp.chunk()` up to `MAX_BODY + 1`,
    then `String::from_utf8_lossy`).
  - Reuse the boundary-safe cut from `fetch_proxy`. Put it in one helper and use it on both
    paths.
  - Add a test: a fake forward serving 9 MiB of `0xFF` bytes returns a capped body and does not
    panic.

### integrations-5-03: minor: the gRPC client trusts only the bundled Mozilla roots, so internal gRPC services that the REST client reaches fail TLS

- **Where:**
  - `crates/otto-apiclient/src/grpc.rs:737` and `:745`: `ClientTlsConfig::new().with_webpki_roots()`.
  - `Cargo.toml:158`: `tonic` features `tls-ring` and `tls-webpki-roots` only.
  - Compare `Cargo.toml:140`: `reqwest` 0.13 with `rustls`, which resolves `rustls-platform-verifier`
    (`Cargo.lock`, reqwest dependencies). REST requests therefore verify against the macOS trust
    store, including corporate or internal CAs the user installed. The REST client also has a
    `no_verify` opt-out (`crates/otto-server/src/routes/api_client.rs:282-284`).
- **Evidence:**
  - Internal gRPC services are usually served with an internal or corporate CA. A user who adds
    `grpcs://payments.internal:443` gets `connect failed: … invalid peer certificate:
    UnknownIssuer` on describe, reflect and invoke.
  - An `https://` REST request to the same host in the same API client succeeds, because it uses
    the keychain.
  - gRPC has no TLS-verify toggle, so the user has no workaround.
- **Fix:**
  - Enable tonic's `tls-native-roots` feature. Build the config with
    `ClientTlsConfig::new().with_native_roots().with_webpki_roots()` at both sites, so the trust
    store matches reqwest's.
  - Optionally, honour the same per-request `tls_verify=false` the REST path does.
  - Add a test that the TLS config builder includes native roots.

### integrations-5-04: minor: a Slack attachment upload of up to 20 MB has the 30 s budget of an ordinary API call

- **Where:**
  - `crates/otto-channels/src/slack.rs:568-574` (`put_bytes_to_upload_url` POSTs the bytes with
    `self.http`).
  - `self.http` is `build_http_client()` (`:421`), with `.timeout(REQUEST_TIMEOUT)` = 30 s
    (`:93-95`, `:131`).
  - The attachment cap is `MAX_ATTACHMENT_BYTES = 20 MiB`
    (`crates/otto-channels/src/attach_guard.rs:27`).
  - Compare Telegram's `sendDocument`, which uses a dedicated `UPLOAD_TIMEOUT = 120 s` client
    (`telegram.rs:25-27`, `:199`), and Slack's own downloads, which get `DOWNLOAD_TIMEOUT = 120 s`
    (`slack.rs:97`).
- **Evidence:**
  - Sending 20 MiB in 30 s needs about 5.6 Mbit/s of sustained uplink. On a VPN or home uplink
    below that, every large `⟦otto-file⟧` attachment to Slack fails with "slack upload PUT:
    operation timed out".
  - The same file sent to Telegram succeeds.
- **Fix:** add an upload client to the Slack adapter, mirroring `build_download_client`, with
  `UPLOAD_TIMEOUT = 120 s`. Use it in `put_bytes_to_upload_url`.

### integrations-5-05: minor: a failed chat attachment upload is dropped silently, with no thread note and no retry on the rate-limit hint

- **Where:**
  - `crates/otto-channels/src/mirror.rs:1083-1091`:
    `match adapter.upload(…) { Ok(()) => info!(…), Err(e) => warn!("mirror: file upload {}: {e}", …) }`.
  - Compare the refusal path at `:1045-1048`, which posts "⚠️ Otto did not attach `…`" to the
    thread, and the reply path, which retries rate-limited posts using `retry_after_hint`
    (`:1095-1110`, `REPLY_ATTEMPTS = 3`).
- **Evidence:**
  - Both adapters turn a rate limit into the hint text that `retry_after_hint` parses:
    - Slack: `"slack upload PUT: ratelimited (retry after {seconds}s)"` (`slack.rs:575-583`).
    - Telegram: `"Too Many Requests: retry after {seconds}; …"` (`telegram.rs:252-256`).
  - The upload path ignores that hint. Any upload error (a 429, the timeout in integrations-5-04,
    Telegram's "Request Entity Too Large") is only a daemon log line.
  - The chat user sees the agent's reply that refers to the file, with no file and no
    explanation.
- **Fix:**
  - On `Err`, retry once after `retry_after_hint(&e)` when one is present, capped at the same
    bound the reply path uses.
  - If it still fails, post the same kind of note as the refusal path: "⚠️ Otto could not attach
    `{filename}`: {short error}".
  - Add a test with a stub adapter whose `upload` fails once with a retry hint and then succeeds,
    and a second that always fails and must produce the note.
