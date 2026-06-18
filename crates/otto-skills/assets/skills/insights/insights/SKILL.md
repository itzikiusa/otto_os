---
description: Generate an action-first coding-agent usage report across ALL providers Otto uses (Claude, Codex, agy/Gemini) for a chosen period (day/week/month or explicit range), compare it to the previous comparable period for trends, and emit a self-contained HTML report where every finding — even the good ones — carries Evidence/threshold → Action → Expected effect. Use when the user wants a usage/productivity insights report, a weekly/daily/monthly review, or trend tracking of how they work with AI agents.
category: insights
version: 2
---

# Insights

You produce a **decision-grade** report on how the user works with their coding agents —
not a wall of charts they nod at and forget. The original `weekly-insights` told people
"your sessions are long" and stopped. That is the failure mode this skill exists to kill.

Two principles drive everything:

1. **Action-first.** Every observation — including *what's working* — must end in a concrete
   action with a number attached. A bare note is a bug. See `references/actionability-contract.md`.
2. **Multi-provider, honest about depth.** You report across Claude, Codex, and agy/Gemini.
   Claude has rich facets (goals/friction/outcomes); Codex and agy do **not** — their
   insights are quantitative/behavioral only, and you must say so rather than fabricate
   narrative for them.

> Reference files sit alongside this SKILL.md — load them as you work:
> - `references/actionability-contract.md` — the Finding→Evidence/threshold→Action→Effect shape, the default thresholds table, the "level-up the good" rule, and vague-vs-actionable examples. **Load before writing any narrative.**
> - `references/html-template.md` — the extended HTML: provider switcher, Trend section, Action Plan, plus the original CSS/charts. **Load before rendering.**
> - `references/data-and-history.md` — provider transcript locations, what signal each yields, the history-dir layout, the three-artifact-per-period scheme, how trend comparison reads prior runs cheaply, **and the Otto-generated-facets cache (path/shape/precedence/idempotency/cap)**. **Load before Step 1, Step 1b, and Step 3.**
> - `assets/report-skeleton.html` — the fill-in HTML scaffold you populate.
> - `scripts/collect_insights.py` — the multi-provider, date-range, history-writing collector. **Run it; never re-implement it.** Flags include `--emit-unfaceted` (list facet-less sessions + a compact capped extraction) and `--extra-facets-dir` (merge your cached facets back in; real Claude facets win) — both used by Step 1b.

---

## Method

### Step 0 — Pick the period

Decide the window from the user's request and call the collector accordingly:

| User says | Invocation |
|---|---|
| "today" / "this day" | `--period day` |
| "yesterday" / "last day" | `--period day --offset 1` |
| "this week" | `--period week` |
| "last week" | `--period week --offset 1` |
| "this month" / "last month" | `--period month [--offset 1]` |
| "from X to Y" | `--start YYYY-MM-DD --end YYYY-MM-DD` |
| "last N days" (legacy) | pass `N` positionally |

Week = **Monday–Sunday** (ISO). Month = calendar month. `--offset 1` always means the
**previous** day/week/month.

**Idempotency (catch-up safe).** This skill is safe to invoke for a *specific past period*
repeatedly. After Step 1, check `history.already_generated`: if it's `true` (the period's
metrics row is in `index.json` **and** its HTML is on disk) and the user did **not** ask to
regenerate, **note "already generated for this period," point them at
`history.existing_report_path`, and stop — do NOT produce a duplicate.** To regenerate
on purpose, re-run the collector with `--force`. The Phase-2 daemon's missed-run catch-up
relies on exactly this per-period check so it never double-generates.

### Step 1 — Collect (run the script; parse its JSON)

Run the collector and detect the Israel timezone offset in parallel:

```bash
python3 <skill-dir>/scripts/collect_insights.py --period week --offset 1
```
```bash
python3 -c "from datetime import datetime; from zoneinfo import ZoneInfo; print(int(datetime.now(ZoneInfo('Asia/Jerusalem')).utcoffset().total_seconds()//3600))"
```

Parse the first command's JSON. If it has an `"error"` key, tell the user (e.g. "no
sessions in that period for any provider") and stop. The second command gives
`ISRAEL_UTC_OFFSET` (2 or 3) — the default for the Time-of-Day chart.

The JSON shape:
- `period` — kind/start/end/label.
- `providers_present` — which of `claude`/`codex`/`agy` had sessions.
- `combined` — the all-providers aggregate (same shape as the original report).
- `per_provider.{claude,codex,agy}` — each with `depth` (`full`|`basic`) and a `note`.
- `history` — `metrics_path`, `previous_metrics_path`, `previous_summary_path`,
  `index_path`, `summary_target`, `report_target`.

The collector has **already written** this run's compact metrics JSON. It did **not** write
the HTML or summary — that's you, in Step 5.

### Step 1b — Generate missing facets (cached), then re-collect

The rich narrative (goal/friction/outcome/summary) comes from Claude Code's own facet files
at `~/.claude/usage-data/facets/<sid>.json`. **On most machines that dir is EMPTY** (that
instrumentation ships with Claude Code's own `/insights`), and Codex/agy never have facets at
all. Without facets the report degrades to bare counts/durations — the failure mode this
skill exists to avoid. So when a session lacks a facet, **you classify it yourself from the
transcript and CACHE the result**, and future runs reuse the cache instead of reclassifying.

This is a documented two-step you perform here, before any analysis:

**1) Ask the collector which sessions need a facet (compact, capped extraction):**

```bash
python3 <skill-dir>/scripts/collect_insights.py --period week --offset 1 --emit-unfaceted
```

This prints `unfaceted_sessions` — the in-window sessions that have **no** facet (neither a
real Claude facet nor an already-cached one), each with a **small capped** extraction from its
transcript: `first_user`, `last_user`, a few `sample_user` messages, `tool_mix`,
`tool_call_count`, `tool_error_count` + `error_samples`, `git_commits`/`git_pushes`, and
`user_msg_count`. It is intentionally tiny (caps in the script) so classifying one session is
cheap — it never dumps a whole transcript. It also returns `cache_dir`, the `facet_shape` to
produce, the per-run `cap`, the `counts` (`unfaceted_total`/`emitted`/`left_unclassified`),
and a `left_unclassified` list. **Use the same `--period`/`--offset`/range you used in Step 1.**

**2) Classify each emitted session and write a cached facet JSON** with exactly this shape
(same keys the collector already reads), to the session's `cache_path`:

```
~/Library/Application Support/Otto/insights/facets/<provider>/<session_id>.json
```

```json
{
  "goal_categories":   {"feature_work": 1},          // 1+ category : weight
  "friction_counts":   {"unclear_request": 1},       // friction type : count ({} if none)
  "outcome":           "fully_achieved",              // fully_achieved | mostly_achieved | partially_achieved | not_achieved
  "primary_success":   "shipped_feature",             // short tag of what landed
  "session_type":      "implementation",              // implementation | debugging | review | research | ops | ...
  "brief_summary":     "One-line plain-English summary of the session.",
  "claude_helpfulness":"high"                          // high | medium | low
}
```

Infer these from the extraction: the goal from `first_user`; `outcome`/`primary_success` from
outcome signals (`git_commits`/`git_pushes` > 0 and low `tool_error_count` → achieved; a
frustrated/abandoning `last_user` → lower); `friction_counts` from `error_samples` and
repeated retries; `session_type` from the `tool_mix`. Write to **that cache dir only — NEVER
to `~/.claude`.** Apply this to **codex** too (codex has no facets at all) — same cache,
provider-namespaced (`.../facets/codex/<sid>.json`).

**Cost cap.** `--emit-unfaceted` emits only the newest `--emit-cap` sessions (default 40) and
reports the rest in `left_unclassified` / `counts.left_unclassified`. Classify only what was
emitted. **If `counts.left_unclassified > 0`, note in the report that N sessions were left
unclassified this run (cost-bounded) and will be picked up next run** — do not exceed the cap.

**3) Re-run the collector pointing at the cache** so the generated facets flow into the report:

```bash
python3 <skill-dir>/scripts/collect_insights.py --period week --offset 1 \
  --extra-facets-dir "$HOME/Library/Application Support/Otto/insights/facets"
```

The collector merges cached facets in, **preferring a real Claude facet whenever one exists**
(generated only fills gaps). Use *this* re-collected JSON for Steps 2–5. **Idempotency:** a
session that already has a real or cached facet is skipped by `--emit-unfaceted`, so you never
reclassify — re-running this step only ever classifies the newly-missing sessions.

### Step 2 — Load history & compute the Trend (cheap reads ONLY)

To compare against full history without burning tokens, read **only** these — never a past
HTML report:

1. `history.index_path` → `index.json` — the whole trajectory (headline time series +
   action-item ledger) in one small file.
2. `history.previous_metrics_path` → last comparable period's numbers (for per-metric deltas).
3. `history.previous_summary_path` → last period's ≤10-sentence summary (carry-forward context).

For each headline metric (sessions, messages, msgs/day, achievement rate, median response
time, tool errors, active days), compute the delta vs the previous period and classify:
**▲ improved / ▼ regressed / ▬ flat** — *improved* means moved in the good direction (e.g.
fewer tool errors is ▲, slower response time is ▼). If `previous_metrics_path` is null, this
is the first comparable run → render "No prior period to compare." and skip deltas.

Also read the `action_ledger` in `index.json`: for each open action item, check whether its
target metric moved this period and mark it improved/closed/still-open (Step 5 writes the
updates back).

### Step 3 — Analyze (action-first; load the actionability contract first)

Load `references/actionability-contract.md`. Mine `combined` and each `per_provider` block
for findings. **Every** finding — problems and wins alike — must be written in the shape:

> **Finding** → **Evidence/threshold** (the actual number + what counts as too much/long/slow)
> → **Action** (concrete, specific steps) → **Expected effect** (the target number/outcome).

Use the **default thresholds table** in the contract to quantify ("long session = median
> 90 user-msgs or > 35 min; high tool-error = > 3 repeated errors before resolution"). You
may tune a threshold to the data, but you must always **state the number**. Good items get a
**"take it to the next level"** action — never a bare compliment.

Respect provider depth: Claude findings can be narrative (goals, friction, outcomes); Codex
and agy findings are **behavioral only** (volume, tools, durations, hours). Do not invent
facet-style narrative for a basic provider — note the limitation instead.

### Step 4 — Build the Action Plan

Pick the **top ~5 actions** ranked by **impact × effort**. Each gets: the action, the metric
it targets, the current value, the target value, and effort (S/M/L). These are the items you
carry into the ledger so next period shows "improved / closed / still open."

### Step 5 — Render + store three artifacts

Load `references/html-template.md`. Build the report from `assets/report-skeleton.html`:
**Combined view first**, then per-provider sections/tabs (provider switcher), with the
**Trend** section and the **Action Plan** section near the top, and every narrative section
rewritten action-first. Self-contained: inline CSS/JS, `file://`-openable. Default the
Time-of-Day chart to `ISRAEL_UTC_OFFSET`.

Then write **three** artifacts to the history dir (paths are in the `history` block):

1. **The full HTML** → `history.report_target`. **Human-only.** You must **NEVER re-read a
   stored HTML report** — re-reading fat reports is the token trap this whole history design
   avoids.
2. **The compact metrics JSON** — already written by the collector. Don't touch it.
3. **`summary-<kind>-<start>_<end>.md`** → `history.summary_target`. Key takeaways, **HARD
   CAP ≤ 10 sentences — no more.** It MUST include this period's **Action Plan items** (the
   set you carry forward). This tiny file — not the HTML — is what the next run reads.

Finally, **update `index.json`'s `action_ledger`** (at `history.index_path`): append new
action items (id, opened-period, target metric, target value, status `open`), and flip any
prior item to `improved`/`closed` when its target metric moved, recording the latest value.
The numeric `series` row was already appended by the collector.

### Step 6 — Report to the user

Tell the user:
- The `file://` path to the HTML report.
- A 2–3 line summary: period, providers covered, sessions/messages/active-days, achievement
  rate (Claude), top trend movement (one ▲ and one ▼), and the #1 action from the plan.

---

## Anti-patterns

| Anti-pattern | Why it fails |
|---|---|
| A bare note ("sessions are long") | The original's core failure. No threshold, no action, no target = useless. Every finding needs all four parts. |
| Praising without a next level | "What's working" with no level-up action wastes the win. Good items still get an action. |
| A vague action ("be more efficient") | Not executable. Name the concrete steps and the target number. |
| Inventing narrative for Codex/agy | They have no facets. Fabricated friction/outcome analysis is a lie — state the depth limit. |
| Re-reading a past HTML report for trends | The token trap. Read `index.json` + the prior `metrics.json`/`summary.md` only. |
| A summary.md over 10 sentences | Breaks the cheap-read contract; the cap is hard. |
| Mixing providers without saying so | Combined facet sections reflect Claude only — say which signal came from where. |
| One threshold for everyone, never tuned | Thresholds are defaults; tune to the data — but always print the number you used. |
| Writing generated facets into `~/.claude` | Generated facets go **only** to the Otto cache (`~/Library/Application Support/Otto/insights/facets/<provider>/`). Never pollute Claude Code's own facets dir. |
| Reclassifying sessions that already have a facet | `--emit-unfaceted` skips real/cached facets; only classify what it emits. Re-doing cached ones wastes tokens and breaks idempotency. |
| Classifying past the per-run cap | The cap (`--emit-cap`, default 40) bounds cost. If `left_unclassified > 0`, note it in the report — don't blow the budget classifying everything at once. |

## Quality bar

A great insights report is one the user **acts on**. Every section ends in a specific,
numbered action — including the good news. The Trend section shows real movement vs last
period (or honestly says it's the first run). The Action Plan is five things ranked by
impact, each carried forward so next period proves whether it worked. Provider depth is
honest: Claude rich, Codex/agy behavioral. And the whole thing was produced reading small
files only — the history stays cheap to compare no matter how long it grows.
