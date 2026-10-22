# Usage & Cost Tracking

Otto records **real, per-turn token usage and USD cost** for the coding-agent CLIs
it runs (Claude Code and Codex) without any manual instrumentation. A background
**tailer** reads the transcript files those CLIs already write to disk; an embedded
**ClickHouse** engine (run locally, no server, no port) stores every turn as a row,
with the four token classes broken out — **input, output, cache-read, cache-write** —
plus an estimated cost. The same engine samples **system CPU / memory** telemetry,
exposes per-provider / per-day / per-session / per-feature rollups, and feeds
**opt-in spend budgets** with a live `budget_exceeded` WebSocket alert.

This is the definitive end-user + operator guide. It documents what the code in
`crates/otto-usage/`, `crates/ottod/src/usage_tailer.rs`, `crates/otto-server/src/routes/usage.rs`,
`crates/otto-server/src/monitor.rs`, and `ui/src/modules/usage/` actually does —
exact endpoints, columns, setting keys, on-disk paths, and UI labels.

> Related docs: **[Insights](./insights.md)** (scheduled HTML usage reports built
> on top of this data) and the **[daemon HTTP API](./daemon-http-api.md)** (auth,
> tokens, and how to call `/usage/*` yourself). Cost estimates are produced by the
> bundled rate table in `crates/otto-usage/src/pricing.rs` and are surfaced as
> **estimates**, never as authoritative billing.

---

## 1. Summary

| | |
|---|---|
| **What it is** | A local usage/cost meter + system-metrics store for the agent CLIs Otto runs. |
| **How usage is captured** | A background **tailer** imports local Claude/custom-account, Codex, Grok and Antigravity history. Zero manual instrumentation. |
| **What is recorded** | One row per agent turn: `input / output / cache-read / cache-write` tokens, model, provider, an estimated `cost_usd`, and work-graph attribution. |
| **Storage engine** | Embedded **ClickHouse** in `clickhouse local --path` mode — no daemon, no port, data on disk. Degrades to a no-op if the binary is missing. |
| **On disk** | `~/Library/Application Support/Otto/clickhouse/` (data) · `~/Library/Application Support/Otto/bin/clickhouse` (Otto-installed binary). |
| **Retention** | A `MergeTree` `TTL`, default **180 days**, changeable live (no restart). |
| **System metrics** | CPU %, memory, 1-min load, the `ottod` process's own RSS/CPU, and the live-session count — sampled every **60 s** by default. |
| **Budgets** | Per-workspace / per-provider USD spend caps. **Opt-in** (`enforce` defaults off) — informational until you turn them on. |
| **Where it lives** | The **Usage & Metrics** page (top-level nav): tokens first (cost secondary), a per-model breakdown, a **Report** view with a downloadable self-contained HTML report, and (root) an opt-in **ccusage cross-check**. Needs `Usage:View`; root sees every session, everyone else their own. |
| **Daemon** | `ottod` on `127.0.0.1:7700`; routes under `/api/v1/usage/*` (reads `Usage:View`, scoped; config/install/budget writes/metrics/ccusage root) plus the per-session `/api/v1/ingest/usage`. |
| **WS events** | `usage_metrics_tick` (after each metrics sample) and `budget_exceeded` (on a cap crossing). |

---

## 2. Overview & where it lives

The **Usage & Metrics** page needs **`Usage:View`**. Root sees every session
(`scope: "all"`, with the *Otto only* filter); any other user holding the grant
sees **only the sessions they created** (`scope: "own"`, external sessions never
included) and none of the admin surfaces — engine config/install, budgets
editor, system CPU/RAM metrics and the ccusage cross-check stay root-only, and
`/usage/status` redacts paths, version, sizes and row counts for non-root.
(The per-session ingest route is separate; see §9.)

If a ClickHouse read fails, the summary/report endpoints return an **error**
(the page shows it inline with Retry) — never a 200 with all-zero totals.

Two storage layers cooperate:

| Layer | Crate / file | Responsibility |
|---|---|---|
| **Capture** | `crates/ottod/src/usage_tailer.rs` | Tails Claude/Codex transcripts as they change (FSEvents; 10-min reconcile), parses per-turn tokens, attributes them to an Otto session (or `external`), and records a [`UsageEvent`]. |
| **Engine façade** | `crates/otto-usage/src/engine.rs` (`UsageEngine`) | Owns the ClickHouse handle + a batched event writer; runs all the rollup queries; manages install / retention / status. |
| **ClickHouse wrapper** | `crates/otto-usage/src/clickhouse.rs` | A thin async wrapper that shells out to `clickhouse local --path <dir>`. |
| **Schema / DDL** | `crates/otto-usage/src/schema.rs` | `CREATE TABLE` for `usage_events` + `system_metrics`, TTL, and additive column migrations. |
| **Pricing** | `crates/otto-usage/src/pricing.rs` | Per-model USD rate table → `estimate_cost(...)`. |
| **Metrics sampler** | `crates/otto-usage/src/metrics.rs` + `crates/otto-server/src/monitor.rs` | Host/process telemetry via `sysinfo`, sampled on a loop. |
| **Budgets** | `crates/otto-server/src/routes/usage.rs` + `crates/otto-usage/src/budget_dedup.rs` | Spend caps, status rows, and de-duplicated crossing events. |
| **HTTP routes** | `crates/otto-server/src/routes/usage.rs` | All `/usage/*` endpoints + `/ingest/usage`. |
| **UI** | `ui/src/modules/usage/` (`UsagePage.svelte`, `AttributionDrilldown.svelte`, `CostForecastChip.svelte`) | The dashboard, attribution drilldown, and forecast chip. |

The engine is started once at daemon boot
(`UsageEngine::start(usage_config, data_dir)` in `crates/otto-server/src/boot/build.rs`) and
the tailer right after it (`UsageTailer::new(usage, pool, data_dir, home).start()`).

---

## 3. How usage is captured (transcript tailing)

Otto does **not** instrument the PTY or intercept the API. The agent CLIs already
write exact, per-turn token counts and the model id to JSONL transcript files; the
**tailer** is the single source of truth, and it survives session resumes, channel
sessions, and daemon restarts.

### 3.1 Which files are tailed

| Provider | Transcript glob | Per-turn signal |
|---|---|---|
| **Claude Code and custom Claude providers** | `~/.claude/projects/<enc_cwd>/<session-uuid>.jsonl`, `~/.claude-*/projects/` (including `claude-work`), `~/.config/claude/projects/`, and Otto subscription-account `projects/` directories, plus subagent transcripts `<session-uuid>/subagents/*.jsonl` (billed to the parent session) | `type=="assistant"` lines carry `message.usage.*`; model at `message.model`. Each API response is counted once by its `message.id:requestId` key, so a subagent response that also appears in the parent file is not double counted. |
| **Codex** | `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` | `type=="event_msg"` + `payload.type=="token_count"` lines; counts from `payload.info.total_token_usage`, differenced against a persisted per-session baseline. Model from `session_meta` and `turn_context` (else `"codex"`). |
| **Grok** | `~/.grok/sessions/**/updates.jsonl` | Completed-turn model usage; cache is split out of input and reasoning is already included in output. Reported `costUsdTicks` is used when available; otherwise cost is estimated. |
| **Antigravity (`agy`)** | `~/.gemini/{antigravity-cli,antigravity,antigravity-ide}/conversations/*.db` | Read-only per-step SQLite metadata supplies historical dates, models, and disjoint input/output/cache counts. JSONL mirrors are not summed again. |

`GROK_HOME` and `ANTIGRAVITY_DATA_DIR` override/add provider roots for the
normal daemon. Isolated E2E daemons ignore those environment overrides. Additional
Antigravity backup/ACP and `~/.config/antigravity` roots are discovered as well.

### 3.2 Token field mapping (verified against the parsers)

Parsers live in `crates/otto-transcript/src/{usage,grok_usage,agy}.rs`;
`otto-usage/src/tailer.rs` re-exports the legacy usage parsers. Missing fields default to `0`;
non-matching lines, lines without usage, and parse failures are silently skipped.

**Claude** (`parse_claude_line`):

| Normalized field | Transcript source (`message.usage.*`) |
|---|---|
| `input` | `input_tokens` |
| `output` | `output_tokens` |
| `cache_read` | `cache_read_input_tokens` |
| `cache_write` | `cache_creation_input_tokens` |

**Codex** (`parse_codex_line`) reads cumulative `total_token_usage`. The tailer
records only the increase from the last committed session total. Input excludes
`cached_input_tokens`; output uses `output_tokens` (reasoning is already included).
Cache-read uses `cached_input_tokens`, and cache-write is zero.

### 3.3 Attribution (which session a turn belongs to)

The tailer keeps an attribution index built from the SQLite `sessions` table.
It is built on the first pass that actually reads new lines and **reused across
passes** (event-driven passes run every ~2 s while an agent streams): it is
re-read only when it is older than 60 s, or when a lookup misses **and** a
session was created / deleted / given its provider-session id since the index
was built (a write generation bumped by `SessionsRepo`). A transcript that is
simply not Otto's (external) never re-queries. Indexes:

- **Claude** — by transcript filename stem (= the CLI session UUID =
  `provider_session_id` on the Otto session row).
- **Codex** — by the exact `session_meta` provider session ID.
- **Grok** — by the exact `params.sessionId` (or session-directory fallback).
- **Antigravity** — by conversation UUID, then recorded parent/root conversation.

Shared working directories do not establish ownership.

Custom Claude sessions keep the provider name stored on their Otto session, such
as `claude-work`. Unmatched transcripts in `~/.claude-work/projects` use the
`claude-work` label and are visible in the **All** view.

A turn that maps to an Otto session inherits that session's `workspace_id` and id.
A turn that maps to nothing is recorded under the special workspace id
**`external`** (constant `EXTERNAL_WORKSPACE`) with the transcript UUID as the
session id. External usage gives a complete machine-wide picture but is excluded by
default in the dashboard (the **Otto / All** toggle — see §6).

### 3.4 No double-counting, true-time stamping

Source offsets, response deduplication, held streaming responses, and Codex
cumulative baselines commit atomically with owed usage events to
`usage-ingestion.sqlite3`. Delivery to ClickHouse reuses each journal event ID;
replay after a crash does not add the event twice. A failed journal commit stops
source advancement until the same batch commits successfully. Legacy cursor
files are imported once and are no longer advanced independently.

- **Byte offsets:** only complete JSONL lines are consumed. Reads are capped at
  8 MiB per step. An unchanged file is not parsed again.
- **Claude response identity:** repeated content blocks and resume copies share
  `message.id:requestId`. The tailer folds streaming counts, releases final usage,
  and records only positive corrections for late updates. Held responses and
  their provider labels survive restart in the checkpoint.
- **Original timestamps:** Claude and Codex events retain their source event
  times, so catch-up is attributed to the day the work happened.
- **Account history:** newly discovered custom Claude account directories are
  read from the beginning. Existing default-Claude/Codex checkpoints and their
  startup baseline policy are preserved; upgrading does not delete or rebuild
  already recorded usage. Historical records remain subject to retention.
- **Cadence:** Claude/Codex JSONL changes wake the tailer through FSEvents with
  a two-second debounce. Full discovery runs at boot and every ten minutes,
  also recovering dropped watcher events. Without a watcher it polls every
  twenty seconds. Streaming Claude responses release after fifteen seconds of
  quiet.

#### One-time repartition (schema drift)

Tables created before the DDL had `PARTITION BY toYYYYMM(...)` stay
unpartitioned under `CREATE TABLE IF NOT EXISTS`, so every TTL expiry and the
rebuild purge rewrote the whole table. At engine init — before the writer
starts and before the engine reports ready, so nothing inserts mid-copy and
the purge above only touches affected months — a table whose
`system.tables.partition_key` is empty is copied into `<table>_repart` with
the current partitioning, row counts are compared, and the copy is swapped in
with `EXCHANGE TABLES`; the old data is then dropped. Any failure leaves the
original untouched and retries next start. Both tables use
`ttl_only_drop_parts = 1`.

---

## 4. The embedded ClickHouse engine

### 4.1 How it runs

Otto ships no embedded C++; it drives the *same* `clickhouse` binary you'd install
via `curl https://clickhouse.com/ | sh`, as one persistent **`clickhouse server`**
child bound to a random loopback HTTP port over the data dir
(`crates/otto-usage/src/clickhouse.rs`). Writes are batched (usage events every
90 s while any are buffered, or sooner when a dashboard reads; system metrics every
5 min).

**Idle-stop.** The server costs ~56 threads / ~100+ MB RSS even with nothing to
do, so after **15 min** with no query, insert or DDL (and none in flight) it is
stopped cleanly (SIGTERM) and the next request restarts it on demand — measured
~0.5–0.7 s to the first answer on a small data dir; the usage writer and the
metrics batch keep buffering in memory meanwhile. An idle-stopped server counts
as alive for the self-heal check (it did not crash); a restart that fails trips
the normal self-heal. Merge pools are sized for a few MB of data
(`background_pool_size` 4, paired with MergeTree's free-entry thresholds).

### 4.2 On-disk layout

With the default data dir (`$OTTO_DATA_DIR`, else `~/Library/Application Support/Otto`):

| Path | Contents |
|---|---|
| `…/Otto/clickhouse/` | The ClickHouse data directory (`usage_events`, `system_metrics`, merges, TTL). |
| `…/Otto/bin/clickhouse` | The binary, when installed via Otto's **Install ClickHouse** button. |
| `~/.local/bin/clickhouse` | A symlink onto `PATH` created by the installer so the binary is runnable daemon-wide. |
| `…/Otto/usage_tailer.json` | The tailer's per-file byte-offset cursors (see §3.4). |

### 4.3 Binary discovery

`ClickHouse::locate` resolves the binary in priority order, returning an absolute
path: (1) the configured `clickhouse_path` if it's a file; (2) `which clickhouse`;
(3) well-known locations — `/usr/local/bin/clickhouse`, `/opt/homebrew/bin/clickhouse`,
`~/clickhouse`, `~/.local/bin/clickhouse`, and `~/Library/Application Support/Otto/bin/clickhouse`.

### 4.4 Install / enable (`POST /usage/install`)

When no binary is found the dashboard shows a **"Set up usage tracking"** card with
an **Install ClickHouse** button and a field to point at an existing binary. The
install flow (`UsageEngine::install_clickhouse`):

1. Runs `curl -fsSL https://clickhouse.com/ | sh` with cwd = `…/Otto/bin`, dropping
   a `clickhouse` binary there. **The download is large (hundreds of MB) and can
   take a while.**
2. Symlinks it to `~/.local/bin/clickhouse` (ottod augments `PATH` with that dir).
3. Sets `clickhouse_path` + `enabled = true` and re-initializes the engine against
   the new binary (no daemon restart).

The route then persists the config and returns the fresh `UsageStatus`. Pointing at
an existing binary instead uses `PUT /usage/config` with `{enabled: true, clickhouse_path}`.

### 4.5 Graceful degradation

The engine **never fails to start.** If the binary can't be located or the schema
can't be created, `UsageEngine` becomes a no-op recorder: `record()` drops events,
queries return empty, and `status()` reports `available: false` while still
surfacing the resolved binary, data dir, retention, and `priced_as_of`. The rest of
the daemon is unaffected. `available()` (binary found **and** schema live) gates the
metrics sampler.

### 4.6 Schema (two MergeTree tables)

`usage_events` — one row per turn/tool-call/session action:

```
ts                 DateTime64(3) DEFAULT now64(3)
event_date         Date DEFAULT toDate(ts)            -- partition / TTL column
workspace_id       String                             -- 'external' for non-Otto
session_id         String
provider           LowCardinality(String)             -- 'claude' | 'codex' | …
model              LowCardinality(String)
kind               LowCardinality(String)             -- 'completion' | 'prompt' | 'tool' | …
input_tokens       UInt64
output_tokens      UInt64
cache_read_tokens  UInt64
cache_write_tokens UInt64
cost_usd           Float64
duration_ms        UInt64
-- work-graph attribution (added additively; DEFAULT '' = "not set")
repo_id, branch, pr_number, story_id, swarm_task_id,
workflow_id, channel, review_id, origin   LowCardinality(String) DEFAULT ''
ENGINE = MergeTree ORDER BY (event_date, provider, session_id, ts)
TTL event_date + INTERVAL <retention_days> DAY
```

`system_metrics` — one row per host/process sample:

```
ts, metric_date, host, cpu_pct, mem_used_mb, mem_total_mb, mem_pct,
load_avg_1, process_rss_mb, process_cpu_pct, active_sessions
ENGINE = MergeTree ORDER BY (metric_date, ts)
TTL metric_date + INTERVAL <retention_days> DAY
```

The nine work-attribution columns are added to pre-existing tables at startup via
idempotent `ALTER TABLE … ADD COLUMN IF NOT EXISTS` (warnings, never fatal).

### 4.7 Retention (TTL)

Retention is a per-table `MergeTree` `TTL` on the partition date column; old data is
dropped automatically during background merges. The window is **configurable, default
180 days** (`UsageConfig.retention_days`, clamped 1..=3650 at the route). Changing it
runs `ALTER TABLE … MODIFY TTL` **live** (`set_retention`) — no recreate, no restart.
The alter is skipped when the table's TTL (read from `system.tables.engine_full`)
already matches, and carries `materialize_ttl_after_modify = 0`, so neither boot nor a
self-heal queues a part-rewriting mutation; background TTL merges apply the window.

**Write path.** Each event is stamped with the time it was recorded (not the
time it is inserted), so a deferred row still lands in the right day, budget window
and swarm turn. The writer flushes buffered events every **90 s** (or at 2,000 events,
before any usage read — the read waits up to 2 s for that flush, so it sees every
row recorded before it — and on shutdown, which waits for that final flush), with server-side `async_insert=1&wait_for_async_insert=1`, and both
tables use `old_parts_lifetime = 60`. Each flush is one part that is later merged into
the month's partition, so the coarse cadence keeps merge I/O low (at 15 s a month's
partition was rewritten ~1,300 times in three days). Usage flushes are **background**
writes: they never wake an idle-stopped server or reset its idle clock. While it is
parked, events stay in memory until the next foreground request wakes it, or until
the oldest has waited 30 min (or 20,000 are buffered), when the writer wakes it once
without resetting the idle clock. A held buffer (parked, or a failed insert) is
retried on the timer, a read or the wake — not once per new event — and the parked
check runs before the batch is serialized.

**Background reads.** The budget sampler, budget gates (swarm turns, workflow steps,
goal loops) and a swarm turn's cost read spend WITHOUT resetting the idle clock, so
an always-on agent no longer keeps the server up through the budget path. While the
server is parked, budget checks reuse the last spend they saw (nothing new can be
written while it is parked); a swarm turn's cost may restart it briefly for an exact
total, still without resetting the idle clock.

---

## 5. Token breakdown & cost

### 5.1 The four token classes

Every turn splits tokens into four independently-priced classes, used identically by
the headline, provider bars, daily chart, by-feature rollup, and session rows. The
UI colors them (`TOKEN_CATS` in `UsagePage.svelte`):

| Class | Meaning | UI color |
|---|---|---|
| **Input** | Uncached prompt tokens. | accent (blue) |
| **Cache write** | Tokens written to the prompt cache. | `#f59e0b` (amber) |
| **Cache read** | Tokens served from the prompt cache ("cached" hits). | `#10b981` (green) |
| **Output** | Generated tokens (incl. Codex reasoning). | `#8b5cf6` (purple) |

`total_tokens` = `input + output + cache_read + cache_write`.

### 5.2 Cost estimation (`pricing.rs`)

When a recorder doesn't supply an explicit `cost_usd`, the engine estimates it from
the model id and token counts. Rates are **per 1M tokens** and track published list
prices as of **`PRICED_AS_OF` = `2026-09-25`** (surfaced in the UI as
*"Priced as of …"*). The four classes are priced independently:

- **input** — model base input rate.
- **output** — model output rate.
- **cache read** — `0.1 ×` the base input rate, except where the model lists its
  own: Opus 5.5 **$0.20** (0.05×), Fable/Mythos 5.1 **$0.25** (0.025×).
- **cache write** — `1.25 ×` the base input rate (the 5-minute-TTL cache rate the
  CLIs use; the transcripts report a single un-typed `cache_creation` count, so the
  common case is priced).

Rate card (per 1M, input / output), matched **case-insensitively, most-specific-first**:

| Model family (substring) | Input | Output |
|---|---|---|
| `fable-5-1` / `mythos-5-1` | $10 | $50 (cache read $0.25) |
| `fable` / `mythos` | $10 | $50 |
| `3-5-haiku` | $0.80 | $4 |
| `3-haiku` | $0.25 | $1.25 |
| `haiku` (4.5) | $1 | $5 |
| `opus-5-5` | $4 | $20 (cache read $0.20) |
| legacy Opus (`opus-4`, `opus-4-0`, `opus-4-1`, `3-opus`) | $15 | $75 |
| `opus` (5, 4.5–4.8) | $5 | $25 |
| `sonnet-5` (5, 5.5) | $2 | $10 |
| `sonnet` (4.x) | $3 | $15 |
| `gpt-4o-mini` / `o4-mini` / `-mini` | $0.15 | $0.60 |
| `gpt` / `codex` / `o3` / `o1` | $2.5 | $10 |

**Unknown models** fall back to a conservative non-zero tier (the Opus rate,
`$5/$25`) so a brand-new model id is **over-estimated rather than billed at $0**.
Such turns are flagged: `is_priced(model)` returns false, the session row carries
`fallback_priced: true`, and the UI renders the cost with an **"est."** tag and the
tooltip *"Estimated — model not in the rate table; priced at the Opus tier."*

> Cost is always an **estimate** — there is no live billing integration. Treat
> figures as directional. The UI never claims otherwise (it says "Est. cost" and
> "priced as of <date>").

---

### 5.3 Usage Report and the ccusage cross-check

- **Report** (`GET /usage/report`): daily, monthly, per-model and per-session
  tables over the window, tokens first and cost secondary, scoped like the
  summary — two ClickHouse scans (one grouped day×provider×model scan the other
  tables are re-aggregated from, plus the session leaderboard). The page loads
  the slim shape (100 sessions, no day×model table); **Show more sessions** and
  **Download HTML** fetch the full one once (`sessions_limit=1000&include=daily_models`)
  and write every table to one self-contained file (inline CSS, light/dark).
- **Compare with ccusage** (root, opt-in, `POST /usage/ccusage-check {days}`):
  the daemon runs `npx --yes ccusage daily --json --breakdown --since … --until …`
  on demand (no Otto dependency; npx downloads/caches ccusage on first use;
  120 s timeout; 1–90 days, default 7) and shows its numbers beside Otto's for
  the same dates — **external sessions included** on Otto's side, because
  ccusage reads every local transcript — with the difference per
  provider/model and per day. Models only ccusage sees (e.g. Hermes'
  `gpt-*`) are labelled `untracked`. Expect small cost differences: ccusage
  prices from LiteLLM's table, Otto from §5.2.

---

## 6. Rollups (provider / day / session / feature)

`GET /usage/summary?days=N&otto_only=B` returns the full dashboard payload
(`UsageSummary`). `days` defaults to **30** (clamped 1..=3650); `otto_only` defaults
to **true** (the **Otto / All** toggle — `false` includes the `external` workspace).
The window is "last N days inclusive of today" (`event_date >= today() - (N-1)`).

The three core rollups are dispatched as **one** `clickhouse local` process
(`query_batch`, sentinel-delimited result sets) for one spawn / one lock / one scan
window:

| Rollup | Shape (`crates/otto-usage/src/types.rs`) | Grouping |
|---|---|---|
| **Per provider** | `ProviderUsage` | `GROUP BY provider`, ordered by total tokens. |
| **Per day** | `DailyUsage` (`day` = `YYYY-MM-DD`) | `GROUP BY event_date`, ordered by date. |
| **Per session** | `SessionUsage` (top **50** by tokens) | `GROUP BY session_id`; `last_active = max(ts)`. |

Each rollup carries the four token classes, `total_tokens`, `events`, and a
6-dp-rounded `cost_usd`. The summary's `total_*` headline numbers are the sums across
providers.

**Server-side enrichment** (`routes/usage.rs`, all in a single SQLite scan, not N+1):

- **Top-session rows** are enriched with the Otto session `title` (pane name), a
  `kind` badge, `workspace_name`, and `fallback_priced`. External / unknown sessions
  stay un-enriched (and aren't click-through navigable in the UI).
- **By feature (`by_kind`)** — `GET /usage/by-kind` (and embedded in the summary) —
  folds every session's raw sums into per-feature buckets using a classifier over the
  session's SQLite metadata. Labels: `review` (Code review), `product` (Product AI),
  `channel` (Channels), `agent` (Ad-hoc agents), `connection` (Connections), `swarm`
  (Swarm), `external` (External). The label prefers the session meta `source` tag set
  by the review/product/channel runners, else the session kind.

### 6.1 Work-graph attribution & forecast (the drilldown)

`GET /usage/attribution?by=<dim>&days=N` answers *"why did this cost so much?"* by
`GROUP BY` one work-graph dimension over the window, returning cost / tokens / distinct
sessions per group (empty "not set" keys filtered out). Dimensions (`by=`):
`repo`, `branch`, `pr`, `story`, `swarm_task`, `workflow`, `channel`, `review`,
`origin` (default). The columns are populated from each session's
`meta_json["work"]` `WorkRef` at ingest time. Pre-migration installs that haven't
restarted simply return empty rows for the new columns until the migration runs.

`POST /usage/forecast` (`{feature, provider, est_tokens?}`) estimates the cost of a
future run, returning `{projected_cost_usd, basis}` (always `Ok`, with a "no data"
basis when there's no history):

- With `est_tokens` — priced directly, split evenly input/output (conservative).
- Without — derived from the average per-session cost for that `origin` (feature) +
  provider pair over the last **30 days**.

The dashboard surfaces this as the **CostForecastChip** next to the headline cost
(forecasting the next "agent" run with the most-used provider) and the
**AttributionDrilldown** panel.

---

## 7. System metrics

`crates/otto-usage/src/metrics.rs` samples host + process telemetry via `sysinfo`;
`spawn_metrics_sampler` (`monitor.rs`) drives it on a loop. CPU % needs two refreshes
a short window apart, so the sample sleeps ~200 ms and runs on a blocking thread. A
quick first sample is taken ~3 s after boot, then on the configured cadence (re-read
each loop, so a settings change takes effect within one interval). Sampling runs only
while `usage.available()` **and something needs it**: a live session, a
`/usage/metrics` read in the last 10 min, or usage recorded in the last 10 min — an
idle daemon nobody watches takes no samples and broadcasts no ticks (budgets have
nothing new to check then). One sampler lives across ticks, so the `ottod` CPU % is
measured over the interval. Samples are buffered in memory and inserted as one batch
every **5 min** (and on shutdown); `/usage/metrics` serves the unflushed tail from
memory, so the sparklines stay live.

Each sample → one `system_metrics` row:

| Column | Meaning |
|---|---|
| `host` | Hostname (for multi-host dashboards). |
| `cpu_pct` | Global CPU usage %. |
| `mem_used_mb` / `mem_total_mb` / `mem_pct` | System memory. |
| `load_avg_1` | 1-minute load average. |
| `process_rss_mb` / `process_cpu_pct` | The `ottod` process's own RSS / CPU %. |
| `active_sessions` | Live session count at sample time (`manager.live_count()`). |

`GET /usage/metrics?minutes=N` returns the time series (`MetricPoint[]`) for the last
`minutes` (default **60**, clamped 1..=43200). The dashboard renders CPU% and Memory%
sparklines plus a live readout (CPU / Mem / ottod RSS / active sessions). Interval is
`metrics_interval_secs`, default **60 s**, clamped 5..=3600.

---

## 8. Budgets & `budget_exceeded`

Per-workspace and per-provider USD **spend caps** over a window. Enforcement is
**opt-in**: `UsageBudgetConfig.enforce` defaults to `false`, so caps are purely
informational (and the daemon's budget check is a no-op) until a root user turns
them on. Persisted under the `usage_budgets` settings key.

`UsageBudgetConfig` fields: `enforce` (master opt-in), `block_on_exceed` (when on +
`enforce`, an exceeded cap is a hard block; otherwise warn-only), `window_days`
(default 30, clamped 1..=3650), `workspaces[]` (`{workspace_id, monthly_usd}`),
`providers[]` (`{provider, monthly_usd}`). A `monthly_usd <= 0` row is ignored.

`GET /usage/budgets` returns the config plus **live status rows** (`BudgetStatusRow`:
`scope`, `key`, `label`, `limit_usd`, `spent_usd`, `used_fraction`, `warning`,
`exceeded`) — computed even when enforcement is off, so the UI can preview caps. The
**warning** line is **80 %** of a cap; **exceeded** is ≥ 100 %. Spend is one pass over
per-session totals folded into per-workspace and per-provider buckets.

`PUT /usage/budgets` replaces + persists the config and returns refreshed status. The
UI editor (gated behind **Configure**) lets you toggle enforce / block, set the window,
and add per-workspace / per-provider caps; blank rows are dropped on save.

### 8.1 The `budget_exceeded` event

`spawn_budget_sampler` (`monitor.rs`) subscribes to the metrics tick and, on each
tick, re-checks budgets. When `enforce` is off it clears its de-dup state and does
nothing. When on, it compares each row's `exceeded` flag through `BudgetDedup`
(`crates/otto-usage/src/budget_dedup.rs`), which emits **exactly one** signal per
crossing:

- `Exceeded` — first tick a `(scope, key)` is above its cap.
- `Recovered` — it has dropped back below the cap.
- `NoChange` — no transition (no event).

Each non-`NoChange` signal broadcasts a `budget_exceeded` WS event:

```json
{ "type": "budget_exceeded", "workspace_id": "<id|''>", "provider": "<name|''>",
  "spend_usd": 42.5, "cap_usd": 40.0, "direction": "exceeded" }
```

- `direction` is `"exceeded"` or `"recovered"`.
- For a workspace cap, `provider` is empty; for a provider cap, `workspace_id` is empty.
- **Scope: `Everyone`** — all connected clients receive it (every admin should see it),
  not filtered per workspace.
- De-dup is in-memory; a daemon restart resets it (harmless re-alert at worst).
- The `channels.notify_budget_exceeded` setting (default **off**) also routes it to
  Slack / Telegram via `otto-channels/improve_notify.rs`.

In the UI a dismissible banner appears at the top of the Usage page; a `recovered`
event auto-clears it. The daemon-consultable `check_budget(ctx, workspace, provider)`
returns a `BudgetVerdict` (`blocked` / `exceeded` / `reason`) that callers can use to
gate or warn on work; it is a no-op when enforcement is off.

---

## 9. API / contract reference

`status`, `summary`, `report` and `by-kind` are Usage:View (non-root: redacted / own sessions); every other `/usage/*` route — `attribution` and `forecast` included — is root-only (a non-root Usage:View caller passes the guard, then gets 403). Paths are under `/api/v1`. Authoritative
contract: `docs/contracts/api.md` (§ "Usage tracking & system metrics") and
`docs/contracts/ws.md` (Usage metrics tick A9 + `budget_exceeded`).

| Method & path | Request | Response |
|---|---|---|
| `GET /usage/status` | — | `UsageStatus` (available, enabled, binary, version, data_dir, retention_days, metrics_interval_secs, usage_rows, metric_rows, disk_bytes, priced_as_of). |
| `GET /usage/summary` | `days?` (def 30), `otto_only?` (def true) | `UsageSummary` (totals + providers + daily + sessions + by_kind). |
| `GET /usage/by-kind` | `days?`, `otto_only?` | `FeatureUsage[]` (per-feature rollup). |
| `GET /usage/metrics` | `minutes?` (def 60) | `MetricPoint[]` (system metrics series). |
| `GET /usage/attribution` | `by?` (def `origin`), `days?` | `AttributionRow[]` (cost/tokens/sessions per work-graph dimension). |
| `POST /usage/forecast` | `ForecastReq {feature, provider, est_tokens?}` | `ForecastResp {projected_cost_usd, basis}`. |
| `PUT /usage/config` | partial `{enabled?, retention_days?, metrics_interval_secs?, clickhouse_path?}` | `UsageStatus` (applied live). |
| `POST /usage/install` | — | `UsageStatus` (installs ClickHouse, then activates; slow). |
| `GET /usage/budgets` | — | `UsageBudgetStatus` (config + live status rows). |
| `PUT /usage/budgets` | `UsageBudgetConfig` | `UsageBudgetStatus` (replace + persist). |
| `POST /ingest/usage` | `IngestUsageReq` + headers `X-Otto-Session` / `X-Otto-Token` | `204` always. |

**`POST /ingest/usage`** is the one **non-root** usage route: it is *unauthenticated*
in the RBAC policy (Exempt) but gated by the **per-session ingest token** Otto sets on
the agent PTY (`X-Otto-Session` + `X-Otto-Token`, verified via
`manager.verify_ingest_token`). It lets injected provider hooks report a turn's tokens
without a user bearer token; the event is attributed to the named session, cost is
estimated when not supplied, and work-graph dims are flattened from the session's
`meta_json["work"]`. Any verification miss returns `204` (silent no-op).

**Config persistence keys** (SQLite `settings` table): `usage` (`UsageConfig`) and
`usage_budgets` (`UsageBudgetConfig`).

### 9.1 WebSocket ticks

| Event | When | Payload |
|---|---|---|
| `usage_metrics_tick` | After each `system_metrics` sample (only while sampling is wanted — see §7). | `{"type":"usage_metrics_tick","ts":"<UTC ISO-8601>"}` |
| `budget_exceeded` | On a budget crossing (enforcement on). | see §8.1 |

The UI subscribes to `usage_metrics_tick` and calls `usage.applyMetricsTick()`, which
triggers a **throttled** `/usage/metrics` refresh (ignores ticks within 10 s of the
last fetch) so sparklines update in near-real-time without blind polling.

---

## 10. Capabilities & limitations

**Capabilities**

- Real per-turn token capture for Claude Code, custom Claude accounts, Codex, Grok, and Antigravity with **zero manual
  instrumentation** — survives resumes, channel sessions, and restarts.
- Four-class token breakdown + per-model USD cost estimate; attribution by provider,
  day, session, feature, and nine work-graph dimensions; pre-launch cost forecast.
- Live retention changes, live system-metrics sparklines, and opt-in spend budgets
  with a real-time crossing alert.
- Machine-wide picture (Otto + external usage), togglable.
- CSV / JSON exports for providers, daily, sessions, summary, and attribution.

**Limitations**

- **Cost is an estimate**, not billing. Unknown models are deliberately over-estimated
  at the Opus tier (flagged "est."). Rate table is a point-in-time snapshot
  (`PRICED_AS_OF`).
- **No backfill.** Only turns that occur after the engine is up are counted; pre-existing
  transcript history is seeded out.
- Legacy Antigravity `.pb` archives are not imported; SQLite conversation histories are supported.
- **Codex attribution is cwd-based** — ambiguous when two Codex sessions share a
  directory (those turns become `external`).
- **Macro-level only** — the dashboard is root-only and aggregates all workspaces;
  there is no per-user usage view.
- ClickHouse access is **serialized** (one process / one path lock at a time) — fine at
  Otto's volumes, not a high-QPS analytics store.
- Budgets approximate a calendar month with a rolling `window_days`; enforcement state
  is in-memory and resets on restart.

---

## 11. Security & permissions

- **Root-only dashboard.** All `/usage/*` read/admin routes require `root` (the page
  aggregates across every workspace). Non-root users see only an explanatory message.
- **Per-session ingest only.** `/ingest/usage` is the sole exception — unauthenticated
  in policy but gated by the per-session ingest token; a bad token is a silent `204`.
- **Loopback by default.** Like the rest of `ottod`, served on `127.0.0.1:7700` unless
  a network listener is explicitly enabled.
- **No secrets stored.** Usage rows contain token *counts*, model ids, and cost — no
  prompt or completion text, no credentials. Transcript content is never copied into
  ClickHouse; only the parsed counts are.
- **Local data.** The ClickHouse data dir and cursor file live under the user's
  Application Support directory; nothing is sent off-device. The only outbound call is
  the ClickHouse **installer download** you trigger explicitly.
- **Budget gating** is advisory by default (`block_on_exceed` off) and never blocks
  work unless a root user opts into both `enforce` and `block_on_exceed`.

---

## 12. Troubleshooting

**Dashboard shows the "Set up usage tracking" card (ClickHouse not installed).**
`status.available` is false — the binary wasn't located. Click **Install ClickHouse**
(runs the official installer into `…/Otto/bin`, symlinks to `~/.local/bin`), or point
**"…or point at an existing binary"** at a `clickhouse` you already have and press
**Use** (issues `PUT /usage/config {enabled, clickhouse_path}`). After either, status
should flip to `available: true` with a version string. The engine never crashes when
the binary is missing — it just records nothing.

**Install seems to hang.** The download is hundreds of MB; `POST /usage/install` is
intentionally slow. Watch daemon logs (`/logs/daemon`) for
`usage: clickhouse installed at …`.

**Engine available but no usage appears.** Newly supported Grok, Antigravity,
and custom Claude roots backfill available history using its original dates.
Default Claude/Codex legacy baselines remain intact to avoid importing previously
counted turns twice. Check the date range and switch to **All** to include CLI
sessions not linked to Otto. Allow a scan interval (up to 20 seconds) for new turns.

**Usage lands under "External" instead of a session.** Attribution requires the
exact provider session ID recorded by Otto. A shared working directory does not
establish ownership. Antigravity subagents can inherit their recorded parent/root
conversation's Otto attribution. Unmatched history remains visible in **All**.

**A session's cost shows "est."** The model id isn't in the rate table, so it was
priced at the conservative Opus fallback (`fallback_priced: true`). The number is an
estimate, not a provider invoice. Bump the rate table in `pricing.rs` for new models.

**Metrics sparklines are empty / "Collecting metrics…".** The sampler runs only while
`available()`; it needs ≥ 2 samples to draw a line. Wait one or two
`metrics_interval_secs` (default 60 s; first sample ~3 s after boot).

**Retention change didn't shrink storage immediately.** `MODIFY TTL` applies on the
next background merge; ClickHouse drops expired partitions lazily. `status.disk_bytes`
will fall once merges run.

**`budget_exceeded` never fires.** Enforcement must be on — set `enforce: true` (and
configure a non-zero cap). With enforcement off the sampler is a no-op. The event is
de-duped per crossing, so it fires once on the way over and once on recovery.

**Usage stopped updating; `GET /usage/status` shows `tailer.state: "stopped"`.**
The transcript tailer could not restore its checkpoint (`usage_tailer.json` is
corrupt), so it halted rather than re-reading history and double-counting; `cause`
says why. Move the file aside and restart the daemon: the next start re-seeds
existing files at their current size. `retrying` means a transient failure the
tailer is backing off from — it resumes by itself. (`tailer` is absent on a daemon
that does not report it.)

**Numbers look doubled after a crash.** Shouldn't happen — the byte-offset cursor
(`usage_tailer.json`) is the guard and is written atomically. If the cursor file is
deleted, the next startup re-seeds existing files to their current size (skipping
history), so you lose forward attribution for those files, not duplicate it.

---

## 13. Related docs

- **[Insights](./insights.md)** — the scheduled (daily / weekly / monthly) HTML usage
  reports are generated from this same `/usage/*` data; the `insight_ready` WS event
  and `/insights/*` routes sit alongside `/usage/*`.
- **[Daemon HTTP API](./daemon-http-api.md)** — auth, API tokens, and how to call
  `/usage/*` and subscribe to the WS ticks yourself.
- **Contracts (authoritative):** `docs/contracts/api.md` (§ Usage tracking & system
  metrics, § Insights) and `docs/contracts/ws.md` (Usage metrics tick A9,
  `budget_exceeded`).
- **Source:** `crates/otto-usage/`, `crates/ottod/src/usage_tailer.rs`,
  `crates/otto-server/src/routes/usage.rs`, `crates/otto-server/src/monitor.rs`,
  `ui/src/modules/usage/`.

## Per-session tokens on the Agents page

Each agent session's usage is visible where you work with it (review A5):

- **Pane details chip** — a *Usage* row, tokens first and cost second:
  *"2.1M tokens · $1.24"* (`GET /sessions/{id}/usage`, the session's whole
  recorded history; `null` = nothing recorded, so the row is simply absent).
  Loaded while the pane is focused, at most once a minute, and when the chip
  opens.
- **Sidebar** — *Sort: Tokens (most first, 30 days)* in the Agents sort menu,
  and a tokens/cost line in each row's tooltip
  (`GET /workspaces/{wid}/sessions/usage?days=30`; the daemon caches its
  rollup for 60 s). Sessions without usage keep their recent order after the
  rest.

Both need `Usage:View`; the per-session route also requires seeing the session
(owner or workspace admin), the workspace one returns only the sessions you
may see. With usage tracking unavailable the sort option is hidden.
