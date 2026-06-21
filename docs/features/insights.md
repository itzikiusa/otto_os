# Insights — scheduled, multi-provider "catch-up" usage reports

Insights turn your recent Otto activity into **action-first**, self-contained
HTML reports — generated on a cadence (daily / weekly / monthly), or on demand,
and cached on disk. Each run spawns a real, headless agent session that executes
the bundled `insights` skill for one **closed** period (the previous day, ISO
week, or calendar month), classifies and compares the activity, and writes a
report HTML + a markdown summary + a metrics JSON. Reports are listed and opened
inside the app; a finished run also fires an `insight_ready` WebSocket event that
can be relayed to Slack / Telegram.

> Insights are **global** (not per-workspace) and **opt-in** — all three cadences
> default OFF. Runs are **catch-up**: a scheduled report missed while the app was
> closed is generated the next time the app is open, so a scheduled period is
> never silently skipped.

---

## 1. Overview

The daemon (`ottod`) owns an hourly-gated background **supervisor**. For each
*enabled* cadence whose currently-due period has **no report yet**, it spawns a
disposable agent session that runs the `insights` skill end-to-end. The session
writes its three artifacts to `<data_dir>/insights/<kind>/`; idempotency is keyed
off those files (the scheduler never re-runs a period that is already covered).
The UI lists what's on disk, opens a report's HTML in an in-app iframe, and lets
you trigger an ad-hoc run for any period/offset.

This is Otto's **product** insights (reports about your Otto/agent activity). It
is distinct from Claude Code's own `/insights` and `weekly-insights` commands,
even though the on-disk HTML uses the same report style (see §7).

### Where it lives

| Concern | Location |
|---|---|
| Insights view (Reports + Health tabs) | `ui/src/modules/insights/InsightsPage.svelte` |
| Health sub-tab (Capability & Health Registry) | `ui/src/modules/insights/CapabilitiesPage.svelte` (`#/insights/health`) |
| Scheduler enable/disable (Settings → Insights) | `ui/src/modules/settings/InsightsSettings.svelte` |
| UI API client | `ui/src/lib/api/insights.ts` |
| TypeScript DTOs | `ui/src/lib/api/types.ts` (`InsightsConfig`, `InsightReport`, `InsightKind`, `RunInsightsReq`, `RunInsightsResp`) |
| Backend (scheduler + generator + HTTP API) | `crates/otto-server/src/insights.rs` |
| Scheduler start-up wiring | `crates/ottod/src/main.rs` (`InsightsScheduler::new(ctx).start()`) |
| Route mount | `crates/otto-server/src/modules.rs` (`crate::insights::routes()`) |
| RBAC policy | `crates/otto-server/src/policy.rs` (the `Insights` feature) |
| WS event source | `crates/otto-server/src/insights.rs` → `Event::InsightReady` |
| Channel notifier | `crates/otto-channels/src/improve_notify.rs` (`channels.notify_insight_ready`) |
| Contract (authoritative) | `docs/contracts/api.md` (§ Insights) · `docs/contracts/ws.md` (`insight_ready`) |
| On-disk artifacts | `<data_dir>/insights/` (see §5) |
| Operator skill | `otto-insights` (drives the API over HTTP) |

Navigation: the **Insights** entry (gauge icon) appears in the left rail, the
navigator, and the mobile bottom nav whenever you hold at least **View** on the
`insights` feature; there is also a command-palette action *"Go to Insights"*.

---

## 2. What's in a report

A report is **action-first**: it leads with what to do, not with raw numbers.
The `insights` skill collects recent activity, classifies facet-less sessions,
compares the period against the prior comparable period, and renders a single,
self-contained HTML page (inline CSS, no external assets). The report opens with
an **"At a Glance"** block — *What's working · What's hindering · Quick wins ·
Ambitious workflows* — followed by stats and charts that explain the period:

- **Headline stats** — messages, lines added/removed, files touched, active
  days, messages/day.
- **What you work on** — project-area cards inferred from session summaries.
- **What you wanted / top tools used** — goal categories and tool counts.
- **Languages · session types · response-time distribution** (median/average).
- **Multi-Clauding** — concurrent-session overlap, when present.
- **Time of day · tool errors**, plus **"Impressive things you did"** (big-win
  cards) and **"Where things go wrong"** (friction cards with examples).
- **What helped most · outcomes · primary friction types · inferred
  satisfaction**, and a light "fun ending" closer.

Each generated period produces a **triple** of artifacts (HTML report + markdown
summary + metrics JSON) plus a rolling `index.json` ledger (§5). The UI's report
card shows the report `kind`, the period range, the created-at time, and a
plain-text **summary** (the first ~80 lines of the markdown summary) so you can
skim without opening the HTML.

---

## 3. Scheduling (cadence, enable/disable, Settings)

### Cadences

There are exactly three cadences. The wire / skill token is `day | week | month`;
the on-disk subdirectory and file prefixes use `daily | weekly | monthly`.

| Cadence | Wire token | On-disk word | Period it covers (`offset 1`) |
|---|---|---|---|
| Daily | `day` | `daily` | the **previous calendar day** (yesterday) |
| Weekly | `week` | `weekly` | the **previous ISO week** (Monday … Sunday) |
| Monthly | `month` | `monthly` | the **previous calendar month** (1st … last day) |

All dates are computed in **UTC**. The due period is always the most recent
**complete** period — i.e. `--offset 1`. Examples (from the unit tests): on
Thu 2026-06-18, the due daily period is 2026-06-17; the due weekly period is
Mon 2026-06-08 … Sun 2026-06-14; the due monthly period is 2026-05-01 …
2026-05-31. January rolls a month report back to the previous December.

> The Settings copy phrases these as "Daily — runs the next morning", "Weekly —
> runs on Sunday", "Monthly — runs on the 1st". Mechanically there is no fixed
> clock time: the supervisor checks **at most once per hour** and runs the due
> period as soon as it sees the period has closed and is still un-reported.

### The supervisor (how scheduling actually fires)

- The scheduler is started once at daemon boot (`InsightsScheduler::start()`).
- It **ticks every 60 s**, but an internal **hourly gate** means the real
  due-check runs **at most once per hour**. The first due-check runs
  **immediately on startup** — this is the catch-up path after the app was
  closed.
- On each due-check, for every *enabled* cadence: compute the currently-due
  period; if it is **not** already done (§5 idempotency) and nothing is already
  in flight for that cadence, **spawn a run** at `offset 1`.
- **One run at a time per cadence.** An in-flight set keyed by cadence word
  prevents double-spawning the same period; a slow weekly run does not block a
  daily run the next hour.
- **Bounded catch-up.** Only the *most recent* missed period per cadence is
  generated — Otto does not backfill a long gap of older periods automatically.
  Use a manual run with a larger `offset` to reach further back (§5).

### Enable / disable in Settings → Insights

Open **Settings → Insights** (`#/settings/insights`). Each cadence is an
independent toggle; all three are **off by default**. Toggling a row issues a
`PUT /insights/config` immediately (optimistic, reverting on failure) and the UI
confirms "Insights schedule updated". The config persists to
`<data_dir>/insights/config.json` as a flat object:

```json
{ "daily": false, "weekly": true, "monthly": false }
```

The settings page also notes the dependency on the `insights` skill and links to
**Settings → Skills** and the **Insights view**.

---

## 4. Multi-provider generation

Insights are **multi-provider** in two senses:

1. **What they report on.** The activity they summarize spans *all* providers /
   agent CLIs you run in Otto (Claude Code, Codex, …) — insights are global and
   not tied to one provider's sessions.
2. **How they are generated.** Each run is a real headless agent session created
   on the **global default provider** (`default_provider` setting, resolved via
   `otto_core::provider::resolve_provider`). The run executes in a **neutral
   cwd** (the Otto data dir, which always exists and is where the skill writes
   its history), and is auto-trusted for that path. The session title is
   `Insights: <kind>` with `meta.source = "insights"`. It runs headlessly — Otto
   injects the run prompt once the session TUI is ready, then lets it complete on
   its own; the disposable session is archived after the run window so it does
   not linger in the Agents list.

---

## 5. Manual run + caching

### Run on demand

From the **Insights** view header (Reports tab), the *Run now* control picks a
**period** (`Yesterday (day)` / `Last week` / `Last month`) and an **offset**
(`Previous` = 1, `2/3/4 periods ago`), then issues `POST /insights/run`. Over the
API:

```bash
otto POST /insights/run '{"period":"week","offset":1}'
# → { "started": true, "run_id": "<session_id>" }
```

- `period` is `day | week | month` (also accepts the `daily|weekly|monthly`
  spellings); anything else is a `400 Invalid`.
- `offset` defaults to **1** (the most-recent *complete* period) and is clamped
  to `>= 0`. `offset 0` = the *current, in-progress* period; `offset 2` = the one
  before the previous, etc. This mirrors the skill's `--offset`.
- On success the response is `{ started: true, run_id }` where `run_id` is the
  spawned session id — you can watch it with the `otto-sessions` skill
  (`GET /sessions/{run_id}`, or attach to its terminal).
- If a run cannot start, the response is `{ started: false, reason }`. The two
  reasons are: the **`insights` skill is not installed** in the Library (it is a
  manual-install skill), or **no non-archived workspace exists** to host the run
  (a session row needs a valid workspace + creator). The UI surfaces this as an
  *"Insights skill not available"* card linking to **Settings → Skills**.

After a successful start, the UI **polls** `GET /insights/reports` every 3 s (up
to 20 attempts ≈ 1 minute) and toasts "Insights report ready" when a new report
appears. A scheduled run instead fires the `insight_ready` WS event (§8) when its
period lands.

### Caching & idempotency

There is no separate cache layer — **the on-disk artifacts are the cache.** A
period is treated as **done** (and therefore skipped) when *any* of its expected
artifacts is present:

- the rolling `index.json` `series` array has a row whose `period_key` matches
  `<word>:<startYMD>_<endYMD>` (e.g. `weekly:20260610_20260616`), **or**
- the period's `report-*.html` exists, **or**
- the period's `metrics-*.json` exists.

This makes de-dup maximally permissive so the scheduler never re-generates a
period the skill already covered. The run prompt itself also tells the skill to
stop early if it finds the period already generated.

On-disk layout under `<data_dir>/insights/` (where `<data_dir>` is the parent of
the context-library root, `<data_dir>/library`):

```text
<data_dir>/insights/
  config.json                                  ← scheduler opt-in (this module)
  index.json                                   ← rolling series + action ledger
  <kind>/                                       (daily | weekly | monthly)
    metrics-<kind>-<start>_<end>.json
    summary-<kind>-<start>_<end>.md
    report-<kind>-<start>_<end>.html
```

`<start>`/`<end>` are `YYYYMMDD`. To regenerate a period, you would remove its
artifacts (and its `index.json` row) so the de-dup no longer marks it done — Otto
itself never deletes report files.

---

## 6. Viewing reports (list & open the HTML)

The **Insights → Reports** tab lists every stored report, **newest first** (by
period end, then start). Each card shows the cadence chip, the period range, the
created-at time, and the text summary. `GET /insights/reports` returns a
`ReportView[]`:

| Field | Meaning |
|---|---|
| `kind` | `daily` \| `weekly` \| `monthly` (the report's cadence word) |
| `period_start` / `period_end` | inclusive period bounds (`YYYY-MM-DD`) |
| `html_path` | absolute path of the `report-*.html`, or `null` if only the summary/metrics exist yet |
| `summary` | first ~80 lines of the `summary-*.md` (plain text) |
| `created_at` | RFC3339 mtime of the freshest artifact (HTML preferred) |

> A report can appear with `html_path: null` if a run hasn't finished writing the
> HTML yet (metrics/summary landed first) — re-list shortly. The UI's filter bar
> also lists an **Ad-hoc** kind; the backend currently only writes
> `daily`/`weekly`/`monthly` artifacts, so that filter is forward-looking.

**Opening a report.** Clicking a card resolves the HTML through
`GET /insights/report?path=<html_path>` (the daemon reads the file with the auth
token and hands back a revocable object URL) and renders it in a full-screen
**iframe overlay**. From the overlay you can **Open in new tab** (Tauri webview),
**Download** the HTML to a file, or **Close**. Because the HTML is fully
self-contained, the downloaded file opens anywhere.

Filtering: the chip row (`All / Daily / Weekly / Monthly / Ad-hoc`) filters the
list client-side by `kind`.

### Days-back filtering (the `weekly-insights` skill)

Outside the scheduled-report machinery, Otto also bundles a user-invocable
`weekly-insights` skill (`/weekly-insights <days_back>`) that renders a report
for the **last N days** in the same `/insights` style — e.g. `/weekly-insights 7`
for the last week. This is a free-form, days-back report you generate from the
agent prompt; it is **not** stored under `<data_dir>/insights/` and does not feed
the scheduler. Use the scheduled cadences for calendar-aligned, deduped,
catch-up reports; use `weekly-insights` for an ad-hoc rolling window.

---

## 7. Multi-provider report style

The rendered HTML follows the built-in `/insights` template (Inter font, a
golden "At a Glance" box, a TOC nav bar, stat rows, and a sequence of chart rows
covering goals, tools, languages, session types, response times, time-of-day,
errors, friction, outcomes, and satisfaction). It is intentionally identical in
look to Claude Code's own insights so the two read the same — but Otto's report
is generated by the daemon's scheduler/run path and stored on disk, whereas
`/insights` and `weekly-insights` are agent-prompt commands.

---

## 8. API & contract reference

`docs/contracts/api.md` and `docs/contracts/ws.md` are **authoritative**; the
TypeScript DTOs in `ui/src/lib/api/types.ts` mirror them.

### REST (mounted under `/api/v1`)

| Method & path | Effective auth (RBAC) | Request | Response |
|---|---|---|---|
| `GET /insights/config` | `Insights` · **View** | — | `InsightsConfig` `{daily, weekly, monthly}` (all booleans) |
| `PUT /insights/config` | `Insights` · **Admin** + handler **root** | `InsightsConfig` | the persisted `InsightsConfig` |
| `GET /insights/reports` | `Insights` · **View** | — | `ReportView[]` (newest first) |
| `GET /insights/report` | `Insights` · **View** | query `?path=<absolute html_path>` | the report's HTML |
| `POST /insights/run` | `Insights` · **Edit** + handler **root** | `{ period, offset? }` | `{ started, run_id?, reason? }` |

DTOs:

```ts
interface InsightsConfig { daily: boolean; weekly: boolean; monthly: boolean }

type InsightKind = 'daily' | 'weekly' | 'monthly' | 'adhoc';
interface InsightReport {
  kind: InsightKind; period_start: string; period_end: string;
  html_path: string; summary: string; created_at: string;
}

type InsightRunPeriod = 'day' | 'week' | 'month';
interface RunInsightsReq  { period: InsightRunPeriod; offset?: number }
interface RunInsightsResp { started: boolean; run_id?: string | null; reason?: string | null }
```

> **Contract note.** `docs/contracts/api.md` labels every Insights endpoint
> `root`. In the running code the **RBAC policy layer** maps them to the
> `Insights` feature at View/Edit/Admin tiers (reads = View, run = Edit, config =
> Admin), and the `PUT /insights/config` and `POST /insights/run` **handlers also
> hard-require root** as a belt-and-suspenders. A global **root** user satisfies
> both gates everywhere; a non-root user granted `Insights` can *read* reports
> and config (View), but **config + run remain root-only** because of the
> in-handler `require_root` check. See §10.

### WebSocket — `insight_ready`

Emitted by `crates/otto-server/src/insights.rs` after a **scheduled** run
completes (conditioned on the period's report actually landing,
`period_done() == true`). Scope is **Everyone** — all connected clients receive
it.

```json
{
  "type": "insight_ready",
  "period": "daily 2026-06-20",
  "session_id": "<session_id | null>"
}
```

- `period` — a human-readable label = `<word> <YYYY-MM-DD>` (cadence word + the
  run's start date), e.g. `weekly 2026-06-08`.
- `session_id` — the originating session id, or `null` for a background run with
  no session id.
- TypeScript: `{ type: 'insight_ready'; period: string; session_id?: Id | null }`.

When the `channels.notify_insight_ready` setting is **on** (default off), this
event is relayed to enabled Slack / Telegram integrations as
`Insights report ready: <period>` via `otto-channels/improve_notify.rs`. Because
insights are global, the notification has **no workspace scope** and is delivered
to all enabled integrations.

---

## 9. Capabilities & limitations

**Capabilities**

- Three calendar-aligned cadences (daily / weekly / monthly), each independently
  opt-in.
- Catch-up generation: a period missed while the app was closed is produced on
  the next run after start-up — nothing is silently dropped.
- On-demand runs for any `period` + `offset` (reach back beyond the previous
  period).
- Self-contained HTML reports, listed newest-first, openable in-app, in a new
  tab, or downloadable.
- A plain-text summary per report (no HTML needed to skim).
- Cross-client push via the `insight_ready` WS event and optional Slack/Telegram
  relay.

**Limitations**

- **Requires the `insights` skill** to be installed in the Library (manual
  install). Without it, both scheduled and manual runs are skipped/return
  `started:false`.
- **Requires a non-archived workspace** to host the headless run (session FK).
- **Bounded catch-up**: the scheduler backfills only the *single most recent*
  missed period per cadence; older gaps need manual runs.
- **UTC periods**; there is no per-cadence custom clock time — the hourly gate
  decides when within the day a due period actually runs.
- **No fixed minute**: "next morning / Sunday / 1st" describes the period, not a
  precise scheduled instant.
- The `adhoc` kind exists in the UI filter but the generator currently writes
  only `daily`/`weekly`/`monthly` artifacts.
- Otto never deletes report files; regeneration means removing artifacts +
  the `index.json` row yourself.

---

## 10. Security & permissions

- **RBAC feature: `Insights`.** Reads (`/insights/config`, `/insights/reports`,
  `/insights/report`) require **View**; running (`/insights/run`) requires
  **Edit**; configuring the scheduler (`PUT /insights/config`) requires
  **Admin** (`crates/otto-server/src/policy.rs`).
- **Handler-level root.** `PUT /insights/config` and `POST /insights/run` *also*
  call `require_root` in the handler — so changing the schedule and triggering
  runs are **root-only** today, regardless of feature grants. A global root user
  bypasses all feature checks. Reads are governed by the `Insights` View grant.
- **Report serving is path-gated.** `GET /insights/report?path=` canonicalizes
  the requested path and rejects (`403 Forbidden`) anything that does not resolve
  **inside** the insights directory — an authed caller can never read arbitrary
  files off disk through this endpoint. A missing/unreadable file returns
  `404 Not Found`.
- **Auth token.** The webview loads the report HTML with the bearer token
  (`authedBlobUrl`); the daemon listens on **loopback only** by default.
- **No secrets in reports.** Reports are activity summaries written to the data
  dir; they contain no tokens/passwords (secrets live in the macOS Keychain, not
  in Otto state or reports).

---

## 11. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| *Run now* returns "Insights skill not available" / `started:false` | The `insights` skill isn't installed. Install it from **Settings → Skills**; the scheduler and runs depend on it. |
| `started:false` with a "no workspace available" reason | Every workspace is archived; the headless run needs a non-archived workspace + member to attribute the session to. Create/unarchive one. |
| `403` on `PUT /insights/config` or `POST /insights/run` | Needs a **root** token (handler `require_root`). Mint one per the `otto-api` skill. |
| `403` on `GET /insights/report?path=` | The path resolved outside the insights directory — only paths under `<data_dir>/insights/` are served. Use the exact `html_path` from `GET /insights/reports`. |
| Report card shows but won't open / `404` | `html_path` is `null` or the file isn't written yet (a run still in progress). Re-list after a moment. |
| Scheduled report never appears | Cadence is off (check **Settings → Insights**); the period isn't closed yet (only `offset 1`, i.e. *completed*, periods run); the app wasn't open after the period closed (catch-up runs on next start-up); or the `insights` skill is missing. |
| Older periods never backfilled | Catch-up is bounded to the most recent missed period per cadence. Run them manually with a larger `offset`. |
| No Slack/Telegram on `insight_ready` | `channels.notify_insight_ready` is off by default — enable it, and ensure an integration is connected. |
| Manual run "Running…" spins past ~1 minute | The UI polls 20× at 3 s. The run may still be working (its window is up to 15 minutes); the report will appear on the next list refresh and a scheduled run will fire `insight_ready`. |

For deeper inspection, watch the spawned run with the `otto-sessions` skill
(`GET /sessions/{run_id}`), or inspect `<data_dir>/insights/` directly.

---

## 12. Related docs

- [Usage tracking & cost](./usage-and-cost.md) — the embedded-ClickHouse
  token/cost/metrics engine. Insights are the *narrative, action-first* counterpart
  to Usage's *quantitative* token & spend breakdowns; both surface under the
  Usage/Insights cluster.
- [Daemon HTTP API](./daemon-http-api.md) — how to authenticate against `ottod`
  and call the REST/WS surface used here (and the `otto-api`/`otto-insights`
  operator skills).
- `docs/contracts/api.md` (§ Insights) and `docs/contracts/ws.md`
  (`insight_ready`) — the authoritative API & event contracts.
- `docs/MULTI-USER-RBAC.md` — the role/grant model behind the `Insights` feature
  tiers in §10.
