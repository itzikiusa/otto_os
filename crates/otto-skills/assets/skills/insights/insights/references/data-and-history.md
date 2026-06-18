# Data sources & history

How the collector finds sessions per provider, what signal each yields, and how the history
directory is laid out so trend comparison stays cheap no matter how long history grows.

## Provider transcript locations & signal

### claude — FULL signal
- **Transcripts:** `~/.claude/projects/<enc_cwd>/*.jsonl`
  (`<enc_cwd>` is the cwd with `/` → `-`, e.g. `-Users-itziklavon-claude_ade`).
- **Rich facets:** `~/.claude/usage-data/session-meta/<sid>.json` (per-session metadata:
  message timestamps, duration, lines/files, tokens, tool counts) **and**
  `~/.claude/usage-data/facets/<sid>.json` (the narrative signal: `goal_categories`,
  `friction_counts`, `friction_detail`, `outcome`, `brief_summary`, `session_type`,
  `primary_success`, `claude_helpfulness`, `user_satisfaction_counts`).
- The collector reads session-meta first, then falls back to parsing the JSONL transcript
  for any session missing from session-meta (keeps the original's two-source logic).
- **Yields:** everything — volume, tools, languages, hours, response times **plus** the
  narrative facets. Claude is the only provider whose findings can be narrative.

### codex — BASIC signal
- **Transcripts:** `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`.
- **Format (confirmed in this repo):**
  - line 1 — `{"type":"session_meta","payload":{"id","cwd","timestamp",...}}`
  - user turns — `{"type":"event_msg","payload":{"type":"user_message","message":...}}`
  - tools — `{"type":"response_item","payload":{"type":"function_call","name":...}}` and
    `{"type":"response_item","payload":{"type":"custom_tool_call","name":...}}`
    (real tool names seen: `exec_command`, `apply_patch`, `write_stdin`)
  - timing — `{"type":"event_msg","payload":{"type":"task_complete","duration_ms",...}}`
    (used as per-turn response time, in seconds)
- **No facets exist for codex.** Its `facet` is `{}`, so every facet-derived chart
  (goals/friction/outcomes/etc.) comes out empty. Findings are **quantitative/behavioral
  only**: message counts, session durations, tool counts, hour histogram, response times.
  The report must say so for codex — never fabricate narrative.

### agy / gemini — PLUGGABLE adapter (path NOT confirmed)
- The transcript path for agy/gemini is **not confirmed in this codebase**, so the collector
  does **not** hardcode one. `collect_agy()` *probes* a list of candidate dirs
  (`AGY_CANDIDATE_DIRS` in the script): `~/.agy/sessions`,
  `~/.gemini/antigravity-cli/sessions`, `~/.gemini/sessions`, `~/.gemini/tmp`.
- If a candidate holds `*.jsonl` session files, they're parsed with the **generic basic
  parser** (`_parse_generic_basic_jsonl`) — a conservative heuristic that counts user-looking
  turns, timestamps, and tool-looking calls. If nothing is found, agy gets an **empty
  provider section with a note** (never wrong data).
- **To wire agy up for real:** confirm the directory + schema, add the dir to
  `AGY_CANDIDATE_DIRS`, and if the schema differs from the heuristic, write a dedicated
  parser mirroring `_parse_codex_jsonl`. Do **not** guess the schema in the meantime.

## Output JSON shape (what the skill consumes)

```
{
  "period": { kind, start, end, label, offset, run_id },
  "providers_present": ["claude", "codex", ...],
  "combined":     { stats, charts, multi_clauding, session_summaries,
                    friction_details, project_sessions, provider_mix },
  "per_provider": {
     "claude": { depth:"full",  note, stats, charts, ... },
     "codex":  { depth:"basic", note, stats, charts, ... },
     "agy":    { depth:"basic", note, stats, charts, ... }   // empty if none found
  },
  "history": { kind, history_dir, metrics_path, previous_metrics_path,
               previous_summary_path, index_path, summary_target, report_target,
               already_generated, existing_report_path, force }
}
```

`combined` and each `per_provider` value share the **same aggregate shape as the original
report** (so the existing HTML rendering logic applies unchanged), with `depth` + `note`
added so the renderer can label provider depth.

## Date ranges

The collector accepts (Step 0 of SKILL.md):
- `--period {day|week|month} [--offset N]` — `offset 1` = the **previous** period.
  **Week = Monday–Sunday (ISO).** Month = calendar month.
- `--start YYYY-MM-DD --end YYYY-MM-DD` — explicit inclusive range (kind = `adhoc`).
- positional `days_back` (legacy back-compat) — last N days (kind = `adhoc`).

`adhoc`/explicit ranges are **not** stored to history by default (no comparable cadence to
trend against); named periods are.

## History directory layout

Root: `~/Library/Application Support/Otto/insights/`

```
insights/
  index.json                         ← single rolling file: full-history series + action ledger
  daily/
    metrics-daily-<start>_<end>.json   ← compact numbers  (written by the COLLECTOR)
    summary-daily-<start>_<end>.md     ← ≤10-sentence takeaways + this period's actions (SKILL)
    report-daily-<start>_<end>.html    ← full HTML report, HUMAN-ONLY                  (SKILL)
  weekly/   (same three files per period)
  monthly/  (same three files per period)
```

`<start>`/`<end>` are `YYYYMMDD`. `kind` ∈ `daily|weekly|monthly` (adhoc isn't persisted).

### Three artifacts per period
1. **`report-*.html`** — the full report. **Human-only. The skill must NEVER re-read a stored
   HTML report.** Re-reading fat reports is the token trap this whole design avoids.
2. **`metrics-*.json`** — compact stats + chart counts (no per-session narrative text).
   Written by the collector. This is what a trend run reads for exact per-metric deltas.
3. **`summary-*.md`** — key takeaways, **HARD CAP ≤ 10 sentences**, and it **must include this
   period's Action Plan items**. This tiny file (not the HTML) is what the next run reads for
   qualitative carry-forward.

### `index.json` — the rolling full-history file (read this, not the reports)
One small file at the insights root, appended every run. Two parts:
- **`series`** — one compact headline row per period (`period_key`, kind, label, start/end,
  `headline` = sessions/messages/active_days/msgs_per_day/achievement_rate/median_response_
  time/duration/tool_error_total, `provider_mix`). The whole trajectory across **all** periods
  in one file → read-cost stays ~constant as history grows. The collector writes these rows
  (idempotent: re-running a period replaces its row).
- **`action_ledger`** — the carry-forward action items: `id`, opened-period, `target_metric`,
  target value, `status` (`open` / `improved` / `closed`), latest value. The **skill** edits
  this: it appends new Action-Plan items as `open`, and flips prior items to `improved`/
  `closed` when their target metric moves. This ledger is how full-history "what improved /
  what still needs work" is tracked.

## Trend comparison (cheap reads ONLY)

To compute trends across full history without burning tokens, the skill reads **only**:
1. `index.json` (the whole series + ledger — one small file),
2. `previous_metrics_path` (last comparable period's numbers, for exact deltas),
3. `previous_summary_path` (last period's ≤10-sentence summary, for context).

It **never** reads a past `report-*.html`. `find_previous_metrics()` locates the most recent
stored `metrics-<kind>-*.json` whose period **ends before** this run's start — i.e. last week
for a weekly run, last month for a monthly run. If none exists, it's the first comparable run
→ the report says "No prior period to compare." `previous_summary_path` is derived by swapping
`metrics-…json` → `summary-…md` next to the previous metrics file (null if the skill hadn't
written one yet).

## Idempotency (for the catch-up scheduler)

The Phase-2 daemon does **missed-run catch-up** via an hourly due-check (cron said 07:00 but
the app opened at 10:00 → generate the period that was due). That relies on **per-period
idempotency** so it never double-generates.

`already_generated(kind, start, end)` returns true only when **both**:
- the `index.json` `series` has a row for this `period_key`, **and**
- the period's `report-*.html` exists on disk.

Requiring both means a half-finished run (collector wrote the metrics row but the agent never
produced the HTML) is correctly treated as **not done**, so catch-up retries it. The result
is surfaced as `history.already_generated` in the output.

- If `already_generated` is true and `--force` was **not** passed: the skill **notes "already
  generated for this period," points the user at `history.existing_report_path`, and does NOT
  regenerate.**
- `--force` (→ `history.force = true`, and `already_generated` is forced false) regenerates
  and overwrites the period's three artifacts and its `series` row.

This makes the collector/skill safe to invoke for any specific past period, repeatedly,
without duplicating work — exactly what the catch-up scheduler needs.
