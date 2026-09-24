---
id: usage
title: Usage
group: Insight
route: usage
summary: Tokens, estimated cost and system load for the agents you run — by provider, day, feature, session and piece of work, with optional spend caps.
---

## What it's for

Usage (the **Usage & Metrics** page) shows how many tokens your agent CLIs use
and roughly what that costs. Otto reads the transcript files Claude Code and
Codex already write, so there's nothing to set up in the agents themselves.
Every turn is stored in a small local ClickHouse database with its 4 token
classes (input, cache write, cache read, output), the model and an estimated
cost.

The same page samples your Mac's CPU and memory use, shows which features and
pieces of work drive spend, and lets you set spend caps per workspace or per
provider.

For written reports on how you work, open [Insights](#/walkthroughs/insights).

## Getting started

1. Sign in as the root account. Usage is root-only.
2. Open **Usage** from the sidebar (Insight group).
3. If you see **Set up usage tracking**, click **Install ClickHouse**. Otto
   downloads the engine (a large download that can take a few minutes) and
   turns tracking on. If you already have a `clickhouse` binary, enter its
   path under **…or point at an existing binary** and click **Use**.
4. Run an agent turn. New usage appears within about 20 seconds; click
   **Refresh** in the header to see it.
5. Choose a window (**7d**, **30d**, **90d**, **180d**) and a scope
   (**Otto** or **All**) in the header.

## Everything it can do

**Scope and time window**

- **Otto** counts only sessions run inside Otto. **All** adds every other
  Claude Code and Codex session on this Mac, grouped as External.
- Windows: last 7, 30, 90 or 180 days, including today.

**Headline cards**

- **Total tokens**, with a bar split into input, cache write, cache read and
  output.
- **Est. cost** over the window, plus a forecast chip that estimates the cost
  of your next agent run on the most-used provider. The forecast is based on
  the average cost per session over the last 30 days.
- **Activity** — how many events were recorded.
- **Providers** — how many providers were used, and how many sessions.

**Breakdowns**

- **Token breakdown** — the 4 token classes as a stacked bar with totals, using
  the same colours everywhere on the page.
- **By provider** — tokens and cost per provider (claude, codex, …).
- **Daily cost** — a bar chart of cost per day, each bar split by token class.
  Hover a bar for the day, cost and token split.
- **By feature** — cost and tokens by kind of work: Code review, Product AI,
  Channels, Ad-hoc agents, Connections, Swarm and External.
- **Cost attribution** — pick **Group by** to see cost, share of total, tokens
  and session count per origin, repo, branch, PR, story, swarm task, workflow,
  channel or review. Copy any row as JSON.
- **Top sessions** — the 50 sessions with the most tokens: session id and
  title, workspace, provider and model, events, tokens (with the class split),
  cost and last activity. Click an Otto session (or focus it and press Return
  or Space) to open it. External sessions aren't clickable.

**Cost estimates**

- Cost is estimated from a built-in rate table, per model family, with each
  token class priced separately. The date the rates were taken from shows as
  **Priced as of** in the settings panel.
- A model that isn't in the table is priced at the Opus rate so it's never
  shown as free. Those rows are marked **est.**

**System metrics**

- CPU % and memory % sparklines over the last 3 hours.
- A live readout: CPU, memory used of total, the Otto daemon's own memory, and
  the number of active sessions.
- Samples are taken every 60 seconds by default and update the page as they
  arrive.

**Budgets (spend caps)**

- Click **Configure** in the Budgets panel to add caps in USD **Per
  workspace** or **Per provider**, set the **Window (days)** they cover
  (30 by default), and click **Save budgets**. Rows without a target or amount
  are dropped.
- Each cap shows spend against its limit. At 80% it's flagged with the
  percentage, and at 100% it's marked **over**.
- Caps are informational until you tick **Enforce budgets**. With enforcement
  on, a banner appears at the top of the page when a cap is crossed, and
  clears itself when spend drops back below it.
- Tick **Block work when a cap is exceeded** to stop new work in an
  over-budget scope instead of only warning. This applies to code reviews,
  Product AI actions, swarm runs, workflow steps and goal loops.
- To get the alert in Slack or Telegram, turn on **Budget cap exceeded** in
  **Settings → Notifications**.

**Refresh and export**

- **Refresh** reloads everything. **Auto** turns on auto-refresh every
  60 seconds while the page is open (the button then reads **Live**; click it
  again to stop).
- **Export** in the header downloads the whole summary as JSON.
- **CSV** links export By provider, Daily cost and Top sessions.
- Cost attribution exports as **CSV** or **JSON**.

**Storage and retention (Settings button in the header)**

- **Retention (days)**: how long usage and metrics are kept (180 by default,
  1 to 3,650). Changes apply without a restart.
- **Metrics sample interval (s)**: 5 to 3,600 seconds.
- **ClickHouse binary**: the path to the engine. **Update ClickHouse**
  re-runs the installer.
- The panel shows the ClickHouse version, data folder, size on disk, row
  count, retention and the pricing date.

## Keyboard shortcuts

None specific to this page. In the Top sessions table, Return or Space opens
the focused session. For app-wide keys, see
[Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts).

## Tips and limits

- **Root only.** The sidebar entry follows the Usage feature, but the data
  covers every workspace, so only the root account can open it. Other users
  see "Usage is root-only".
- **Costs are estimates, not billing.** Treat them as a guide. They don't know
  about subscription plans or discounts, and there's no split by subscription
  account.
- **Supported agents:** Claude Code and Codex. agy sessions aren't counted
  because their transcripts are encrypted.
- **History:** Claude Code history already on disk is counted with its real
  dates. Codex history from before tracking started is skipped; only new
  Codex turns count.
- **Codex attribution** is by working folder. If 2 Codex sessions share a
  folder, their turns count as External.
- **Cost attribution needs a work reference.** Review, product and swarm runs
  record one automatically; sessions you start yourself show as origin
  "manual".
- **Retention shrinks storage lazily.** Old data is removed on ClickHouse's
  next background merge, not the moment you save.
- **Metrics need 2 samples** before the sparklines draw. Until then the page
  says "Collecting metrics…".
- **Everything stays on this Mac.** Only token counts, model names and costs
  are stored — never prompt or reply text. The only download is the
  ClickHouse installer you start yourself.
- If the ClickHouse engine is missing, Otto keeps working; it just records
  nothing until you install it.

## Related

- [Insights](#/walkthroughs/insights)
- [Settings](#/walkthroughs/settings)
- [Channels](#/walkthroughs/channels)
- [Agents](#/walkthroughs/agents)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
