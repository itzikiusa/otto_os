# Batch-3 reliability fixes (2026-06-19)

Scope: HTTP client timeouts for the issue/channel integrations, and replacing
Slack's deprecated `files.upload` with the current external-upload flow.

Owned files:
- `crates/otto-issues/src/jira.rs`
- `crates/otto-issues/src/confluence.rs`
- `crates/otto-channels/src/slack.rs`
- `crates/otto-channels/src/telegram.rs`
- `crates/otto-channels/src/mirror.rs`

## Task 1 — HTTP timeouts

Every client was constructed with a bare `reqwest::Client::new()`, which has **no
connect timeout and no overall request timeout**. A hung Jira/Confluence/Slack/
Telegram endpoint would block the request — and stack up in the watcher/listener
loops — indefinitely.

Each construction site now goes through a small `build_*_client()` helper that
uses `reqwest::Client::builder().connect_timeout(..).timeout(..).build()`. The
helper falls back to `Client::default()` if the builder ever fails, so the
existing infallible `new()` constructors keep their signatures (no caller churn).

Timeout values:

| Client | connect | overall | rationale |
|--------|---------|---------|-----------|
| Jira (`jira.rs`, all calls) | 10s | 30s | ordinary REST calls |
| Confluence (`confluence.rs`, all calls) | 10s | 30s | ordinary REST calls |
| Slack Web API (`slack.rs` adapter + listener) | 10s | 30s | `chat.postMessage`, `chat.update`, `apps.connections.open`, the new upload flow |
| Slack attachment download (`slack.rs` `collect_attachments`) | 10s | 120s | file downloads can be large |
| Telegram ordinary calls (`telegram.rs` adapter) | 10s | 30s | `sendMessage`, `editMessageText`, `sendChatAction` |
| Telegram `sendDocument` upload (`telegram.rs` adapter) | 10s | 120s | file uploads can be large |
| **Telegram long-poll listener (`telegram.rs` `run`)** | 10s | **40s** | **long-poll exception** |

### Long-poll exception (required)

The Telegram listener calls `getUpdates?timeout=25` (`LONG_POLL_TIMEOUT = 25s`):
the server holds the HTTP connection open for up to 25s waiting for updates. A
30s overall timeout would race the poll window and cut polling off. The listener
therefore uses a dedicated client whose overall timeout is
`LONG_POLL_TIMEOUT + 15 = 40s` (`LONG_POLL_REQUEST_TIMEOUT`), i.e. the poll
interval plus margin. A unit test
(`long_poll_request_timeout_exceeds_poll_interval`) guards this invariant so a
future change to `LONG_POLL_TIMEOUT` can't silently break long-polling.

Slack uses a Socket Mode **WebSocket** (via `tokio-tungstenite`), not HTTP
long-polling, so its Web API client needs no such exception — the listener's
HTTP client is only used for short control calls (`apps.connections.open`) and
the cancel-aware `tokio::select!` already bounds the WebSocket read.

## Task 2 — Slack `files.upload` → external-upload flow

`files.upload` is deprecated/sunset, so attachment uploads would break. The
`SlackAdapter::upload` method now implements the current three-step external
flow, factored into helper methods on `SlackAdapter` for clarity/testability:

1. `files.getUploadURLExternal` (form-encoded `filename` + byte `length`) →
   returns `upload_url` + `file_id`.
2. POST the **raw bytes** to `upload_url`.
3. `files.completeUploadExternal` (form-encoded `files=[{id,title}]` +
   `channel_id`, plus `thread_ts` when threading) → associates the file to the
   channel/thread.

Outward behavior is unchanged: a file is posted to the chat/thread. The bytes
are sent verbatim via `.body(bytes.to_vec())` — they do **not** go through
`from_utf8_lossy`, so binaries are uploaded intact.

## Task 3 + Task 2 (binary fix) — DONE

The binary-corruption fix required widening the `Adapter::upload` trait signature
from `content: &str` to `content: &[u8]`. That signature lives in
`crates/otto-channels/src/adapter.rs`, which was outside my initial owned-files
list, so I requested and received explicit approval from team-lead to edit it
(the change is fully contained: the only impls are slack.rs/telegram.rs and the
only callers are mirror.rs, all mine, plus the trait in adapter.rs).

A `&str` is always valid UTF-8, so a binary cannot round-trip through it — and
`mirror.rs::upload_file_path` was applying `String::from_utf8_lossy(&bytes)` to
the file's raw bytes *before* any adapter saw them, corrupting binaries at the
boundary. The fix, applied end-to-end:

- `adapter.rs`: `Adapter::upload(..., content: &[u8])` (default no-op otherwise
  unchanged); doc note that bytes are passed verbatim.
- `slack.rs` impl: takes `&[u8]`; POSTs the bytes verbatim to the external upload
  URL (`.body(content.to_vec())`), and `length` is `content.len()`.
- `telegram.rs` impl: takes `&[u8]`; `Part::bytes(content.to_vec())`; the part's
  content type changed from the hardcoded `text/markdown` to
  `application/octet-stream` so arbitrary binaries aren't mislabelled (the client
  still infers the kind from the filename extension).
- `mirror.rs::upload_file_path`: passes the raw file bytes directly (`&bytes`),
  dropping `from_utf8_lossy`.
- `mirror.rs::post_reply`: passes `text.as_bytes()` for the `investigation.md`
  attachment.

## Tests

- `jira.rs`: `test_build_http_client_succeeds` (builder yields a usable client).
- `confluence.rs`: `test_build_http_client_succeeds`.
- `telegram.rs` (new test module): `http_clients_build` (all three builders) and
  `long_poll_request_timeout_exceeds_poll_interval` (guards the long-poll
  invariant).

## Verification

- `cargo check -p otto-issues -p otto-channels` — clean.
- `cargo clippy -p otto-issues -p otto-channels --all-targets -- -D warnings` —
  clean (exit 0) for my owned crates.
- `cargo test -p otto-issues -p otto-channels` — 64 (otto-issues + shared) and
  16 (otto-channels) tests pass; 0 failures.

### Note on the workspace clippy

`cargo clippy --workspace --all-targets -- -D warnings` currently fails, but
**not** because of any file I own. The only error is in
`crates/otto-dbviewer/src/drivers/clickhouse.rs:190` — `method 'connect' is never
used` (dead code) — left by a concurrent dbviewer edit by another agent. My
crates (`otto-issues`, `otto-channels`, and the `adapter.rs` trait change) are
clippy-clean in isolation. Flagged to team-lead.
