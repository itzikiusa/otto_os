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
| **How usage is captured** | A background **tailer** reads `~/.claude/projects/**.jsonl` (Claude Code) and `~/.codex/sessions/**/rollout-*.jsonl` (Codex). Zero manual instrumentation. |
| **What is recorded** | One row per agent turn: `input / output / cache-read / cache-write` tokens, model, provider, an estimated `cost_usd`, and work-graph attribution. |
| **Storage engine** | Embedded **ClickHouse** in `clickhouse local --path` mode — no daemon, no port, data on disk. Degrades to a no-op if the binary is missing. |
| **On disk** | `~/Library/Application Support/Otto/clickhouse/` (data) · `~/Library/Application Support/Otto/bin/clickhouse` (Otto-installed binary). |
| **Retention** | A `MergeTree` `TTL`, default **180 days**, changeable live (no restart). |
| **System metrics** | CPU %, memory, 1-min load, the `ottod` process's own RSS/CPU, and the live-session count — sampled every **60 s** by default. |
| **Budgets** | Per-workspace / per-provider USD spend caps. **Opt-in** (`enforce` defaults off) — informational until you turn them on. |
| **Where it lives** | The **Usage & Metrics** page (top-level nav). Root-only. |
| **Daemon** | `ottod` on `127.0.0.1:7700`; routes under `/api/v1/usage/*` (all root-gated) plus the per-session `/api/v1/ingest/usage`. |
| **WS events** | `usage_metrics_tick` (after each metrics sample) and `budget_exceeded` (on a cap crossing). |

---

## 2. Overview & where it lives

The **Usage & Metrics** page is **root-only**. For any non-root user the page
renders only the message *"Usage analytics are available to the root account."*
The dashboard aggregates across **every** workspace, which is why the read/admin
routes are gated `root` rather than per-workspace — it mirrors the daemon-wide
Settings panels. (The one exception is the per-session ingest route; see §9.)

Two storage layers cooperate:

| Layer | Crate / file | Responsibility |
|---|---|---|
| **Capture** | `crates/ottod/src/usage_tailer.rs` | Tails Claude/Codex transcripts every 20 s, parses per-turn tokens, attributes them to an Otto session (or `external`), and records a [`UsageEvent`]. |
| **Engine façade** | `crates/otto-usage/src/engine.rs` (`UsageEngine`) | Owns the ClickHouse handle + a batched event writer; runs all the rollup queries; manages install / retention / status. |
| **ClickHouse wrapper** | `crates/otto-usage/src/clickhouse.rs` | A thin async wrapper that shells out to `clickhouse local --path <dir>`. |
| **Schema / DDL** | `crates/otto-usage/src/schema.rs` | `CREATE TABLE` for `usage_events` + `system_metrics`, TTL, and additive column migrations. |
| **Pricing** | `crates/otto-usage/src/pricing.rs` | Per-model USD rate table → `estimate_cost(...)`. |
| **Metrics sampler** | `crates/otto-usage/src/metrics.rs` + `crates/otto-server/src/monitor.rs` | Host/process telemetry via `sysinfo`, sampled on a loop. |
| **Budgets** | `crates/otto-server/src/routes/usage.rs` + `crates/otto-usage/src/budget_dedup.rs` | Spend caps, status rows, and de-duplicated crossing events. |
| **HTTP routes** | `crates/otto-server/src/routes/usage.rs` | All `/usage/*` endpoints + `/ingest/usage`. |
| **UI** | `ui/src/modules/usage/` (`UsagePage.svelte`, `AttributionDrilldown.svelte`, `CostForecastChip.svelte`) | The dashboard, attribution drilldown, and forecast chip. |

The engine is started once at daemon boot
(`UsageEngine::start(usage_config, data_dir)` in `crates/ottod/src/main.rs`) and
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
| **Claude Code** | `~/.claude/projects/<enc_cwd>/<session-uuid>.jsonl` | `type=="assistant"` lines carry `message.usage.*`; model at `message.model`. |
| **Codex** | `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` | `type=="event_msg"` + `payload.type=="token_count"` lines; counts from `payload.info.last_token_usage`. Model from the first `session_meta` line (else `"codex"`). |
| **agy** | — | **Unsupported** — token usage is encrypted on disk; logged once. |

### 3.2 Token field mapping (verified against the parsers)

Parsing is in `crates/otto-usage/src/tailer.rs`. Missing fields default to `0`;
non-matching lines, lines without usage, and parse failures are silently skipped.

**Claude** (`parse_claude_line`):

| Normalized field | Transcript source (`message.usage.*`) |
|---|---|
| `input` | `input_tokens` |
| `output` | `output_tokens` |
| `cache_read` | `cache_read_input_tokens` |
| `cache_write` | `cache_creation_input_tokens` |

**Codex** (`parse_codex_line`) — reads the **per-turn** `last_token_usage`, *not*
the cumulative `total_token_usage`, to avoid double-counting:

| Normalized field | Transcript source (`payload.info.last_token_usage.*`) |
|---|---|
| `input` | `input_tokens` |
| `output` | `output_tokens` **+** `reasoning_output_tokens` (reasoning bills as output) |
| `cache_read` | `cached_input_tokens` |
| `cache_write` | always `0` (Codex has no cache-creation concept) |

### 3.3 Attribution (which session a turn belongs to)

Each scan rebuilds an attribution index from the SQLite `sessions` table:

- **Claude** — by transcript filename stem (= the CLI session UUID =
  `provider_session_id` on the Otto session row).
- **Codex** — by the rollout file's `cwd` (from its `session_meta` line) →
  the unique Codex session whose `cwd` matches. Attribution applies **only when
  exactly one** Codex session matches that directory; otherwise the turn is
  treated as external.

A turn that maps to an Otto session inherits that session's `workspace_id` and id.
A turn that maps to nothing is recorded under the special workspace id
**`external`** (constant `EXTERNAL_WORKSPACE`) with the transcript UUID as the
session id. External usage gives a complete machine-wide picture but is excluded by
default in the dashboard (the **Otto / All** toggle — see §6).

### 3.4 No double-counting, no misdated backfill

ClickHouse stamps `ts = now()` on insert and has **no idempotency column**, so the
tailer is the only guard against re-counting:

- **Byte-offset cursor.** A persistent `absolute-path → byte offset` map lives at
  `~/Library/Application Support/Otto/usage_tailer.json` (`CursorStore`, written
  atomically via tmp-file + rename). Only complete lines up to the last `\n` are
  consumed; a partial trailing line is left for the next scan. A file truncation /
  rotation (cursor > size) resets the cursor to 0.
- **History seeding.** At startup every *existing* transcript is seeded with
  `cursor = file size`, so pre-existing history is **skipped** (replaying it would
  misdate every turn to "now"). Files that appear *later* — new real-time sessions —
  start at offset 0 and are captured in full.
- **Cadence.** The loop scans every **20 s** (`SCAN_INTERVAL`). A bad file or line
  is logged and skipped; the loop never panics.

> Consequence: usage only starts accruing for sessions that run **after** the
> daemon (with the engine enabled) is up. Historical transcripts that pre-date
> first launch are intentionally not back-filled.

---

## 4. The embedded ClickHouse engine

### 4.1 How it runs

Otto ships no embedded C++; it drives the *same* `clickhouse` binary you'd install
via `curl https://clickhouse.com/ | sh`, in **`clickhouse local --path <dir>`**
mode — a serverless, portless, on-disk database. Because `clickhouse local` takes
an exclusive lock on its `--path`, every call is serialized through a single mutex
in `ClickHouse` (`crates/otto-usage/src/clickhouse.rs`); process startup is tens of
ms, writes are batched, so this is cheap.

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
prices as of **`PRICED_AS_OF` = `2026-06-19`** (surfaced in the UI as
*"Priced as of …"*). The four classes are priced independently:

- **input** — model base input rate.
- **output** — model output rate.
- **cache read** — `0.1 ×` the base input rate.
- **cache write** — `1.25 ×` the base input rate (the 5-minute-TTL cache rate the
  CLIs use; the transcripts report a single un-typed `cache_creation` count, so the
  common case is priced).

Rate card (per 1M, input / output), matched **case-insensitively, most-specific-first**:

| Model family (substring) | Input | Output |
|---|---|---|
| `fable` / `mythos` | $10 | $50 |
| `haiku` | $1 | $5 |
| `opus` | $5 | $25 |
| `sonnet` | $3 | $15 |
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
while `usage.available()`.

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

All `/usage/*` routes are **root**-only; paths are under `/api/v1`. Authoritative
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
| `usage_metrics_tick` | After each `system_metrics` sample is stored. | `{"type":"usage_metrics_tick","ts":"<UTC ISO-8601>"}` |
| `budget_exceeded` | On a budget crossing (enforcement on). | see §8.1 |

The UI subscribes to `usage_metrics_tick` and calls `usage.applyMetricsTick()`, which
triggers a **throttled** `/usage/metrics` refresh (ignores ticks within 10 s of the
last fetch) so sparklines update in near-real-time without blind polling.

---

## 10. Capabilities & limitations

**Capabilities**

- Real per-turn token capture for Claude Code and Codex with **zero manual
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
- **agy provider is unsupported** (encrypted transcripts).
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

**Engine available but no usage appears.** Expected for sessions that pre-date the
engine — history is seeded out (§3.4). Run a *new* agent turn and wait up to the 20 s
scan interval. Confirm transcripts exist under `~/.claude/projects/**` /
`~/.codex/sessions/**`. Check `status.usage_rows` is climbing. agy sessions are never
counted (encrypted).

**Codex usage lands under "External" instead of a session.** Codex is attributed by
`cwd`, and only when exactly one Codex session matches that directory. Two Codex
sessions in the same cwd → both treated as external. Claude attribution is exact
(by session UUID) and not subject to this.

**A session's cost shows "est."** The model id isn't in the rate table, so it was
priced at the conservative Opus fallback (`fallback_priced: true`). The number is an
over-estimate, not an error. Bump the rate table in `pricing.rs` for new models.

**Metrics sparklines are empty / "Collecting metrics…".** The sampler runs only while
`available()`; it needs ≥ 2 samples to draw a line. Wait one or two
`metrics_interval_secs` (default 60 s; first sample ~3 s after boot).

**Retention change didn't shrink storage immediately.** `MODIFY TTL` applies on the
next background merge; ClickHouse drops expired partitions lazily. `status.disk_bytes`
will fall once merges run.

**`budget_exceeded` never fires.** Enforcement must be on — set `enforce: true` (and
configure a non-zero cap). With enforcement off the sampler is a no-op. The event is
de-duped per crossing, so it fires once on the way over and once on recovery.

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
