# Home — the personal dashboard

> **Route:** `#/home` · **Sidebar:** first entry (ungated) · **Code:** `ui/src/modules/home/`
> _Last verified against the codebase: **2026-09-11**._

Home is a **general dashboard** that fronts the rest of the app: up to **4 views**,
each a **12-column grid of up to 8 live boxes** (Agents, Mission Control, a DB
dashboard, Kubernetes, Insights, Usage). Views **slide** — arrows, dots, `←`/`→`,
swipe — and **auto-rotate every 30 seconds**; every box can be **resized** in
grid units and **zoomed** to fill the page. The layout is a per-device preference
(localStorage), not workspace data: nothing about Home goes through the daemon.

## Setup

None. Home is visible to every authenticated member. On first visit it seeds one
view ("Overview") with the box kinds the current role may view — Agents, Mission
Control, Kubernetes, DB dashboard — so the page is never blank. Each **box kind**
is gated by the same RBAC feature as the module it fronts (see the table below);
the "Add box" picker only offers kinds the user can view.

## Walkthrough

### Views

| Action | How |
|--------|-----|
| Switch view | `‹` / `›` arrows, the dots, `←` / `→` (when no input is focused), a horizontal swipe, or ⌘K → *Home: show "…"* |
| Add a view (max 4) | the dashed `+` next to the dots, or the view-name menu → *Add view* |
| Rename / delete a view | click the view name (or right-click) → *Rename view…* / *Delete view* |
| Auto-rotation | the **30s / Paused** toggle (appears with ≥2 views). Default **on**; the thin progress bar under the toolbar shows the countdown. Persisted. ⌘K → *Home: pause/resume auto-rotation* |

Rotation is **suspended** while a box is zoomed, a sheet is open, or the user is
mid-gesture (resizing / dragging), and while the tab is hidden — a view is never
swapped out from under an interaction. Any manual navigation restarts the
countdown from zero.

### Boxes

| Action | How |
|--------|-----|
| Add (max 8 per view) | **Add box** → pick a kind |
| Resize | drag the **corner handle** (bottom-end); or focus it and use the arrow keys (one grid unit per press). Width `3..12` columns, height `2..12` rows of 72 px |
| Reorder | drag the **grip** at the start of the header onto another box; or the box menu → *Move left / right* |
| Zoom | the **⤢** button, **double-click the header**, or the box menu → *Zoom in*. `Esc` or **⤡** exits |
| Move to another view | box menu → *Move to "…"* |
| Refresh / open the module / fill width / reset size / remove | box menu (`•` or right-click) |

On phones every box spans the full width (height still applies); resize and
drag-reorder are desktop/tablet affordances.

### Box kinds

| Kind | Feature gate | What it shows | Refresh |
|------|--------------|---------------|---------|
| **Agents** | `agents` | working / need-you / idle / open counts and the live session roster (click a row to open it) | live (workspace store) |
| **Mission Control** | `mission_control` | active / needs-approval / total / spend, the by-status chips, and the most recently updated work items (active first). Rows open the item | live `work_graph_updated` + 30 s |
| **DB dashboard** | `database` | any Database Explorer dashboard — the same `WidgetCard` tiles, each on the dashboard's own refresh cadence. Pick the dashboard once; the choice is stored in the box | per dashboard |
| **Kubernetes** | `kubernetes` | one row per registered cluster from the Monitor overview: health, pods, crash/pending, memory vs limits, rps, error %, version drift. Window picker (1h / 6h / 24h / 7d) stored per box. Falls back to the registry (name / env / version) when the monitor isn't collecting | live `k8s_monitor_cycle` + 60 s |
| **Insights** | `insights` | the newest report of each kind with its plain-text summary (zoomed: every report) | 5 min |
| **Usage** | `usage` | spend / tokens / output / events for 1, 7 or 30 days (per box), a daily-spend sparkline, per-provider bars | 60 s |

Every box's poll is a **setTimeout chain** with ±failure back-off (×2 per
consecutive failure, capped ×8) and skips ticks while the tab is hidden, so a
Home page left open overnight doesn't hammer a broken backend.

## API / WS surface

Home has **no endpoints of its own**. Boxes reuse the read endpoints of their
modules (`/workspaces/{ws}/workgraph/summary|items`, `/workspaces/{ws}/db/dashboards|widgets`
+ `/db/widgets/{id}/run`, `/k8s/clusters`, `/k8s/monitor/overview`, `/insights/reports`,
`/usage/status`, `/usage/summary`) and the existing WS ticks (`work_graph_updated`,
`k8s_monitor_cycle`). See [`docs/contracts/api.md`](../contracts/api.md).

## Capabilities & limits

- **4 views × 8 boxes**, hard caps (`MAX_VIEWS`, `MAX_BOXES` in `home.svelte.ts`).
- **Per device.** Layout lives in `localStorage` (`otto_home_views`,
  `otto_home_active`, `otto_home_rotate`); it does not sync between machines or
  users. A malformed / stale persisted layout is sanitized on load (unknown kinds
  dropped, sizes clamped) rather than crashing the page.
- **Auto-rotation is fixed at 30 s** (`ROTATE_MS`).
- The DB-dashboard box renders the same `WidgetCard` as the Dashboards tab, so an
  **editor can delete a widget from Home** (with the usual confirm). Editing a
  widget still happens in the Database Explorer.
- Home is **not** the default landing route — `#/` still opens Agents; the desktop
  app restores whatever route the window last showed.

## Troubleshooting

- **A box shows "unavailable" with an error.** The module behind it is off or
  unreachable (e.g. the usage engine / ClickHouse disabled, Kubernetes console
  off). The box keeps its last good data and backs off; open the module for the
  real diagnostics.
- **Rotation never advances.** Needs ≥2 views, the toggle on, no zoomed box, no
  open sheet, and a visible tab.
- **A kind is missing from "Add box".** Your role can't view that feature.
- **Reset everything.** Remove the three `otto_home_*` keys from localStorage
  (DevTools) — the next visit re-seeds the default view.
