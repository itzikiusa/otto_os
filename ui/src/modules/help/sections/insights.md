---
id: insights
title: Insights
group: Insight
route: insights
summary: Action-first reports on how you work with your agents — what's working, what's slowing you down, and what to change next.
---

## What it's for

Insights turns your recent agent activity into a written report. An agent reads
your session transcripts for one closed period (a day, a week or a month),
compares it with the period before, and writes a report that leads with what to
do: an action plan with targets, key numbers with their trend, and a full HTML
report with charts.

Reports cover activity across every agent CLI you run in Otto (Claude Code,
Codex and others), for the whole machine rather than one workspace. The page
also has a **Health** tab that shows which parts of Otto are ready, degraded or
not set up yet.

Insights is the narrative side of your activity. For raw token counts, cost
and CPU or memory use, open [Usage](#/walkthroughs/usage).

## Getting started

1. Install the `insights` skill: open **Settings → Skills** and add it to the
   library. Every report is written by this skill, so nothing runs without it.
2. Open **Insights** from the sidebar (Insight group).
3. With no reports yet, click **Run yesterday's report**. With reports already
   there, pick a period in the header (Yesterday, 2 days ago, Last week,
   2 weeks ago, Last month, 2 months ago) and click **Run now**.
4. Wait while the banner says the report is being generated. An agent session
   reads your transcripts; this can take a few minutes. The page checks for the
   new report every 3 seconds and shows "Insights report ready" when it lands.
5. To get reports automatically, click the gear (**Schedule settings**) in the
   header, or open **Settings → Insights**, and turn on **Daily**, **Weekly**
   and/or **Monthly**.

## Everything it can do

**Reports list**

- A timeline of every report, newest first. It opens on the report you viewed
  last, or the newest one.
- Filter chips by cadence: All, Daily, Weekly, Monthly, and Ad-hoc. A chip only
  shows when reports of that kind exist, and each shows its count.
- Each row shows the cadence, the period, when it was generated, the report's
  one-line headline, and up to 3 key numbers (sessions, tool errors,
  achievement %) with an up or down arrow against the previous report of the
  same cadence. Green means better, amber means worse.
- Each row also sums up the action plan: how many actions, and how many
  regressed, improved or are new.

**Report view: Preview, Markdown, HTML**

- **Preview** — the key findings, then the rest of the summary:
  - **KPI tiles** for sessions, turns, tool errors, spend and achievement %,
    each with its change since the previous report and a trend line over the
    last 12 reports of that cadence. Hover a tile label to see whether the
    number came from the report summary or the metrics index.
  - **Action plan** as a checklist: each item's title, the metric it targets
    (`current → target`), effort (S, M or L), a status (Regressed, Improved,
    Closed, New or Carried over) and its action id. **Ledger** expands the
    history of that action across reports (opened, latest, target, status).
  - The rest of the summary rendered as formatted text.
- **Markdown** — the summary source, exactly as the agent wrote it.
- **HTML** — the full report with charts: headline stats, what you worked on,
  goals and top tools, languages, session types, response times, time of day,
  tool errors, wins, friction, outcomes and satisfaction. It runs in a
  sandboxed frame that can't reach your Otto login or storage. This view is
  disabled while a period has no HTML yet.
- Otto remembers your last choice of Preview, Markdown or HTML on this device.
- If a summary doesn't have the expected shape, the key-findings block is left
  out and the summary shows as written.

**Export and share**

- **Export .md** downloads the summary as a Markdown file.
- **Download HTML report** saves the full report file.
- **Open in new window** pops the report out into its own window in the
  desktop app (only the report, no list). In a browser it opens a new tab.
- Every report has its own link, `#/insights/r/<kind>/<start>/<end>`, that
  opens it directly.

**Scheduled reports (Settings → Insights)**

- 3 independent toggles, all off by default:
  - **Daily** covers the previous day.
  - **Weekly** covers the previous week (Monday to Sunday).
  - **Monthly** covers the previous calendar month.
- **Provider** and **Model** choose which agent writes the reports. Leave
  Provider on "default" to use your default agent provider; custom providers
  are listed too.
- Runs catch up: if the app was closed when a period ended, the report is
  written the next time Otto is running, so a scheduled period isn't skipped.
- Otto checks for a due report about once an hour, and once right after it
  starts. A period that already has a report is never generated again.

**Notifications and automation**

- When a scheduled report is ready, Otto can post "Insights report ready" to
  Slack or Telegram. Turn on **Insights report ready** in
  **Settings → Notifications** (off by default).
- Workflows can start on the **Insight ready** trigger.

**Health tab**

- One card per area — agent sessions, language servers, MCP servers, channels
  (Slack / Telegram), Git accounts, issue trackers (Jira), database connections
  and message brokers (Kafka) — with a status of Ready, Degraded or Not set up.
- Summary chips count how many areas are ready, degraded or not set up.
  Problem areas are listed first.
- Each card lists what's wrong and how to fix it. Click a card (or focus it and
  press Return) to see every dependency it checked and whether it passed.
- **Fix** (on cards that aren't ready) opens the setup page for that area:
  Providers for agent sessions, Channels, Connections, or Message Brokers.
  For language servers, MCP servers, Git accounts and Jira it currently lands
  on Settings → Appearance; pick the matching pane from the Settings list.
- **Download support bundle** saves a JSON file with your settings (secrets
  removed), the health report, recent audit entries and the database schema
  version. A toast says how many secret values were removed.

## Keyboard shortcuts

None specific to this page. From anywhere, ⌘K runs these Insights commands:

| Command (⌘K) | Action |
|---|---|
| Run yesterday's insights report | Starts a daily report for yesterday |
| Run last week's insights report | Starts a weekly report for last week |
| Export insights summary as Markdown | Downloads the open report's summary |
| Open insights report in new window | Pops the open report out |

The last 2 appear only while a report is open. For app-wide keys, see
[Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts).

## Tips and limits

- **The `insights` skill is required.** If it's missing, runs don't start and
  the page shows "Couldn't start the insights run" with a link to skills
  settings.
- **A workspace is needed.** Each run is a real agent session, so Otto needs at
  least one workspace that isn't archived to host it. The session is archived
  automatically after the run.
- **Permissions.** Viewing reports needs the Insights feature. Starting a run
  and changing the schedule are for the root account only. The Health tab and
  the support bundle are also root-only.
- **Periods are in UTC.** "Daily" means the previous UTC day. There is no fixed
  clock time; the hourly check decides when a due report starts.
- **Catch-up covers one period.** Only the most recent missed period per
  cadence is generated. For older ones, run them from the header (up to
  2 periods back).
- **The page waits about a minute.** It checks 20 times, 3 seconds apart. A run
  can take up to 15 minutes; if it finishes later, it appears the next time the
  list loads.
- **Figures are the agent's reading** of your transcripts. Open the HTML view
  for the charts it computed.
- **Otto never deletes report files.** Reports live in the `insights` folder of
  Otto's data directory. To regenerate a period, remove its files there first.
- **Ad-hoc reports.** The Ad-hoc chip only appears if such reports exist;
  scheduled and **Run now** reports are always daily, weekly or monthly.

## Related

- [Usage](#/walkthroughs/usage)
- [Settings](#/walkthroughs/settings)
- [Channels](#/walkthroughs/channels)
- [Workflows](#/walkthroughs/workflows)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
