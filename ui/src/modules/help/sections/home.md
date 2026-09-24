---
id: home
title: Home
group: Work
route: home
summary: Your personal desktop — a glance at what needs you today, plus up to 4 spaces of live widgets.
---
## What it's for

Home is the page to keep open when you want the whole picture at once. The top shows a greeting and four glance cards: what needs you, what is running, what is coming up and what you touched recently. Below them sit the active space's widgets — live tiles for your agents, Mission Control, a database dashboard, Kubernetes, Insights and Usage.

Spaces 01–04 on Home are the same spaces as the floating command bar's 01–04, so switching in one switches the other.

## Getting started

1. Open **Home** from the sidebar. On your first visit Otto creates a space called "Overview" with the widgets your role can see (Agents, Mission Control, Kubernetes, DB dashboard).
2. Choose **Add widget** in the toolbar and pick a widget.
3. Drag the corner handle to resize it, and drag the grip in its header onto another widget to reorder.
4. Choose **+** next to the space tabs to add another space (up to 4). With 2 or more spaces, Home cycles between them every 30 seconds — choose **30s** to pause.
5. Double-click a widget's header to zoom it to fill the page. Press `Esc` to exit.

## Everything it can do

**Today (the glance strip)**
- A greeting with your name, today's date and a one-line summary ("2 need you · 3 running", or "All quiet").
- **Needs you:** Assistant tasks waiting on you, pending MCP approvals, agent sessions waiting for input, work items awaiting approval, and unread warnings.
- **Running:** Assistant tasks in progress, working agent sessions and in-flight workflow runs.
- **Up next:** Assistant reminders due in the next 24 hours and today's notices.
- **Recent:** Design Hall artifacts and pull requests from the work graph.
- Every row opens the thing it describes. Each card has a "more" link to the page that owns it (Agents, Mission Control, Assistant → Tasks, Design Hall).
- Rows only appear for features your role can view. Fetched cards refresh every 60 seconds while Home is open.

**Spaces**
- Up to 4 spaces, numbered 01–04 and named by you. The active space's name shows in its tab.
- Switch with the tabs, `←` / `→`, a horizontal swipe on touch screens, ⌘K ("Home: space 01 · …"), or the floating bar's space picker.
- Right-click a tab, or choose **⋯**, to rename, add or delete a space. You can't delete the last space.
- Renaming a space in the floating bar renames it on Home, and the other way round.
- **Cycling:** with 2 or more spaces, Home moves to the next space every 30 seconds. A thin progress bar under the toolbar shows the countdown. Cycling pauses while a widget is zoomed, the Add widget sheet is open, you are resizing or dragging, or the window is hidden. Any manual switch restarts the countdown.

**Widgets** (up to 8 per space, on a 12-column grid)
- **Agents:** working, needs-you, idle and open counts, plus the live session list. Click a row to open the session.
- **Mission Control:** active, needs-approval, total and spend figures, plus the most recently updated work items. Refreshes live and every 30 seconds.
- **DB dashboard:** any Database Explorer dashboard, live. Pick the dashboard once; each tile runs on the dashboard's own refresh cadence.
- **Kubernetes:** one row per registered cluster — health, pods, restarts, memory, requests per second, error rate. Window picker: 1h, 6h, 24h or 7d. Refreshes every 60 seconds.
- **Insights:** the newest daily, weekly and monthly report summaries. Refreshes every 5 minutes.
- **Usage:** spend, tokens and events per provider for the last 1, 7 or 30 days, with a daily-spend sparkline. Refreshes every 60 seconds.
- **Widget controls:** Refresh, Zoom in / Exit zoom, and **⋯** (or right-click) for Open *module*, Move left / right, Fill width, Reset size, Move to another space, and Remove box.
- **Resize:** 3–12 columns wide and 2–12 rows tall (72 px per row). Focus the corner handle and use the arrow keys to resize one grid unit at a time.
- Widget refreshes back off after repeated failures and skip ticks while the window is hidden.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `←` / `→` | Previous / next space (when no field is focused) |
| `Esc` | Exit a zoomed widget |
| `←` `→` `↑` `↓` | Resize a widget one grid unit (with its corner handle focused) |
| `⌃1`–`⌃4` | Pick a space (while the floating command bar is focused) |

## Tips and limits

- Home is visible to everyone signed in. Each widget kind needs view access to the feature behind it (Agents, Mission Control, Database, Kubernetes, Insights, Usage). If a widget is missing from **Add widget**, your role can't view that feature.
- Hard limits: 4 spaces, 8 widgets per space. **Add widget** is disabled when a space is full.
- The layout is saved on this device only. It doesn't sync to your other Macs or to other people.
- Cycling is fixed at 30 seconds.
- On a phone every widget spans the full width, and resizing and drag-to-reorder are turned off. Use **⋯** → Move left / right instead.
- A DB dashboard widget shows the same tiles as the Database Explorer, so an editor can delete a tile from Home (Otto asks first). Edit tiles in the Database Explorer.
- A widget that shows an error keeps its last good data. Open its module for the full diagnostics.

## Related

- [Command bar](#/walkthroughs/command-bar)
- [Agents](#/walkthroughs/agents)
- [Mission Control](#/walkthroughs/mission-control)
- [Assistant](#/walkthroughs/assistant)
- [Insights](#/walkthroughs/insights)
- [Usage](#/walkthroughs/usage)
