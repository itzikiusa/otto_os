# Actionability contract

This is the rule the whole skill exists to enforce. The original `weekly-insights` produced
**bare notes** — "your sessions are long", "you use a lot of tools" — with no number and no
next step. The reader learned nothing they could *do*. Every finding in an insights report
must instead be a small, complete unit of decision.

## The four-part shape (mandatory for EVERY finding)

> **Finding** — the one-line claim.
> **Evidence / threshold** — the actual number from the data **and** the threshold that makes
>   it "too much / too long / too slow / too high" (so the reader knows why it's flagged).
> **Action** — concrete, specific steps. Not "be more careful" — *what to do*, ideally a
>   habit, a setting, a command, or a workflow change.
> **Expected effect** — the target number or outcome. What "fixed" looks like, quantified.

If any of the four is missing, the finding is **not done**. A finding with no number is a
vibe; a finding with no action is trivia; a finding with no target can't be checked next
period.

## The "level-up the good" rule

Wins get the same shape. A win is not "nice job" — it is a finding whose **Action** is a
*take-it-to-the-next-level* move and whose **Expected effect** is an even better number.
"What's working" with no action wastes the win. Every good item answers: *given this is
working, what's the next 20% gain?*

## Default thresholds (quantify findings with these; tune to the data, but always state the number)

These are starting points. If the user's data clearly warrants a different line, move it —
but **print the number you used** so the finding stays falsifiable.

| Signal | "Healthy" | Flag when | Source field |
|---|---|---|---|
| **Long session** | median ≤ 90 user-msgs **and** ≤ 35 min | median > 90 user-msgs **or** > 35 min | `stats.total_messages / total_sessions`, `total_duration_minutes` |
| **Messages per active day** | 40–150 | > 200 (grind) or < 15 (barely used) | `stats.msgs_per_day` |
| **Tool errors** | < 1 repeated-error cluster / session | > 3 repeated errors before resolution in a session, or any error category > 10% of tool calls | `charts.tool_error_categories` |
| **Slow response (your think/iterate time)** | median < 60s | median ≥ 120s, or > 20% of turns in the `>2m`/`>15m` buckets | `stats.median_response_time`, `charts.response_time_buckets` |
| **Achievement rate** (Claude only) | ≥ 75% fully+mostly | < 60% | `stats.achievement_rate` |
| **Friction concentration** (Claude only) | no single type > 25% | one friction type > 35% of all friction | `charts.friction_types` |
| **Tool concentration** | top tool < 45% of calls | one tool > 60% of all calls (over-reliance / missing automation) | `charts.tool_counts` |
| **Parallel sessions (multi-clauding)** | intentional, < 30% of msgs | > 50% of messages in overlapping sessions (context-thrash risk) | `multi_clauding.overlap_messages_pct` |
| **Active days** | ≥ target cadence | week with < 3 active days when the goal is steady progress | `stats.active_days` |
| **Lines churned w/o commits** | commits track work | many lines added/removed, `total_commits` ≈ 0 (work not landing) | `stats.total_lines_*`, `stats.total_commits` |

Provider note: `achievement_rate`, `friction_types`, `goal_categories`, `outcomes`,
`satisfaction`, `success_types`, `helpfulness` exist **only for Claude** (facets). For Codex
and agy, restrict findings to the behavioral rows (sessions, messages, duration, tools,
hour-of-day, response time) and say the narrative signal isn't available.

## Good vs bad (vague → actionable)

| Bad (a bare note — never ship this) | Good (the four-part shape) |
|---|---|
| "Your sessions are long." | **Finding:** Sessions run long. **Evidence:** median 132 user-msgs / 48 min vs the 90-msg / 35-min threshold — 8 of 21 sessions exceeded both. **Action:** at ~80 msgs, stop and write a one-paragraph handoff, then `/clear` and start fresh from it; split multi-goal sessions at the goal boundary. **Effect:** median back under 90 msgs / 35 min, fewer "lost the thread" restarts. |
| "You use Bash a lot." | **Finding:** Tool use is concentrated in Bash. **Evidence:** Bash = 64% of 1,240 tool calls (threshold 60%); 180 of those were repeated `cargo build`. **Action:** add a `just check` recipe wrapping the 3 commands you re-run, and let the agent call it once. **Effect:** ~150 fewer Bash calls/week, less waiting on serial rebuilds. |
| "Good job, lots of work done!" (win, no level-up) | **Finding (working):** High landing rate. **Evidence:** 82% fully/mostly achieved (threshold 75%), 14 commits across the week. **Action (next level):** the 18% that stalled were all in `otto-server` refactors — pre-write a 3-line acceptance check before starting those so the agent has a target. **Effect:** push 82% → 90% and cut the stall-and-restart on refactors. |
| "Some tool errors happened." | **Finding:** A tool-error cluster is recurring. **Evidence:** `file_not_found` = 22 errors, 14% of tool calls (threshold 10%), mostly relative-path reads. **Action:** standardize on absolute paths in prompts and add the repo root to the session context up front. **Effect:** `file_not_found` under 5% of calls; fewer wasted retry turns. |
| "You sometimes work late." | **Finding (working):** Focused morning block. **Evidence:** 58% of messages fall in 08:00–12:00 local; almost none after 22:00. **Action (next level):** protect that block — batch agent-heavy tasks (reviews, big refactors) into it where the achievement rate is highest. **Effect:** more high-value work lands in your best hours. |

## The Action Plan section

After the per-section findings, collect the **top ~5 actions** ranked by **impact × effort**.
Each row: the action, the metric it targets, current value → target value, effort (S/M/L).
These are the items written into the `index.json` action-ledger and carried into next
period's Trend, where they're marked **improved / closed / still-open** as the target metric
moves. The Action Plan is the report's payload — if the user does only what's in it, the
report did its job.
