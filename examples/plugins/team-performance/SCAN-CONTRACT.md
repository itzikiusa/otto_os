# Scan control and local state

The plugin sidecar owns these routes. They are proxied through Otto's plugin API;
they are not routes on the main daemon's typed API.

- `POST /scan` accepts `{ "account": "…", "projects": ["TP"], "full": false }`
  and optional `assignees`. It returns `200 { "started": true, "projects": ["TP"] }`.
  An existing running or stopping scan for the account returns `409`.
- `GET /scan/status?account=…` returns the account's job. States are `idle`,
  `running`, `stopping`, `stopped`, `done`, and `error`. Existing progress fields
  (`step`, `fetched`, `total`, `project`, timestamps) remain available.
- `POST /scan/stop` accepts `{ "account": "…" }`. It returns
  `200 { "state": "stopping" }` while cancellation unwinds, or the existing
  terminal state if already finished. Missing account returns `400`; an account
  without an in-memory job returns `404`. Poll status until terminal.

Stop cancels upstream socket requests, interrupts pacing/backoff, prevents new
Jira pages, PR detail requests and estimator batches, and kills the owned git
worker process group (including git subprocesses). Estimator lanes settle before
the scan is terminal. Completed estimates/PRs and completed Jira issue analyses
are retained. A partly completed full rescan merges into the previous corpus and
does not advance its incremental watermark. A later scan can resume safely.

Stop also pauses the remembered automatic scan. A manual Scan clears that pause.
The watchdog uses the same cancellation path before another scan may start.

Cancellation does not roll back already completed writes. A remote agent run
already accepted by Otto may continue on the host after its HTTP caller closes:
the plugin host API provides a synchronous result, not a cancellable run handle.
Stop immediately prevents further agent dispatch and releases the local request;
it does not claim to terminate an accepted remote provider run. Local synchronous
analysis already executing reaches its next event-loop boundary before Stop is
handled. Upstream reads have a 60-second socket timeout in addition to explicit
abort; PR fetches also have a 60-second total timeout. Host agent requests allow
11 minutes for the existing provider limits (Claude three minutes, Codex ten).
Scan-status reads time out after five seconds so even a hung socket produces
visible recovery feedback.

The UI preserves the last known progress when polling fails, labels it status
unavailable, shows last successful update when available, and offers Retry status.
It never infers completion from a network failure. Account/request generations
fence successful and failed responses and scan-completion refreshes.

Settings drafts are held in memory by account, including per-project status maps,
worker rows, time off and unfinished controls. They survive metric refreshes and
tab/scope changes. Saving acknowledges its submitted revision; newer edits and
failed saves remain drafts. Reloading the plugin frame ends this in-memory draft
lifetime. Configuration and the people registry are still globally stored by the
existing sidecar API; account-owned UI drafts do not change that storage contract.

Deployment range results are cached in `deploy-tag-ranges.json` under the plugin's
data directory, bounded to 5,000 entries and 16 MiB. Each key contains a compact
SHA-256 digest of repository path, current tag identity and the ordered preceding
tag identities. Added, moved or deleted earlier tags invalidate the affected
suffix. Cold misses exclude every earlier deployment SHA, preserving divergent
release-branch semantics. Warm worker processes reuse ranges without per-tag git
log calls when the working set fits the cache. The cold scan retains its existing
quadratic exclusion-input cost; an oversized working set may evict/recompute
ranges. Cache corruption, absence or write failure never changes correctness.
