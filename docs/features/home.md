# Home — the personal dashboard

> **Route:** `#/home` · **Sidebar:** first entry (ungated) · **Code:** `ui/src/modules/home/`
> _Last verified against the codebase: **2026-10-05**._

Home is a **general dashboard** that fronts the rest of the app: up to **4 views**,
each a **12-column grid of up to 8 live boxes** (Agents, Mission Control, a DB
dashboard, Kubernetes, Insights, Usage, Classrooms). Views **slide** — arrows, dots, `←`/`→`,
swipe — and can **auto-rotate every 30 seconds** (opt-in); every box can be **resized** in
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
| Auto-rotation | the **30s / Paused** toggle (appears with ≥2 views). Default **off** (moving content is opt-in, WCAG 2.2.2); the thin progress bar under the toolbar shows the countdown, and cycling pauses while the pointer or focus is inside the widgets. Persisted. ⌘K → *Home: pause/resume auto-rotation* |

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
| **Classrooms** | `agents` | a live 3D campus: every workspace is a classroom, every session a student at a desk, you are the headmaster at the door of the current classroom — see [Classrooms](#classrooms) | live (session events + the workspace store) |

### Classrooms

A 3D, at-a-glance picture of everything that is running. Add it from **Add
widget → Classrooms** (default 8 × 6; it is not in the first-visit seed).

**The scene.** Each workspace is a classroom: a floor tile with low walls, a
door gap in the front wall, a board on the back wall and the room name +
counts ("1 needs you · 2 working · 3 idle") floating above it. Rooms sit on a
grid; the **current workspace comes first** and is tinted with the accent
(floor, walls, board, name). Workspace-less sessions get a "No workspace"
room. Each session is a **student** — a low-poly capsule figure at a desk with
a small screen:

| Session state | Student |
|---|---|
| working / running | the figure types (a small bob), the screen glows (`--success`) |
| needs you | the hand is up with a pulsing marker (`--warning`), the screen turns warning |
| idle | still and slightly dimmer, the screen dim |
| suspended / ended / failed | a translucent ghost at an unlit desk |
| reconnecting (events socket down) | still and dim — no "working" claims |

The figure's colour is its **provider** (`--cat-*`: Claude, Codex, Antigravity,
Shell fixed; custom providers hash onto the remaining categories) and a small
floating tag shows its initials (CL / CX / AG / SH / monogram) — on every
student on a small campus, the current room's on a large one (> 60 students).
**Background engine sessions** (workflow steps, swarm, review agents,
scheduled tasks… — `BACKGROUND_SOURCES`) sit in a separate, slightly smaller
**back row** behind a floor line, live ones only; exited engine steps are not
shown. Front rows hold up to 36 students per room and the back row 6; the rest
show as "+N more" on the room label. The **headmaster** — you — stands at the
door of the current classroom in the accent colour, labelled "You · headmaster".

**Interaction.** Hover a student for a card (title, provider, status,
workspace, last active, folder and branch when known) with **Open**,
**Detention** and **Kick out…**; click opens the session (switching workspace
when needed). On touch, the first tap shows the card and a second tap opens.
Right-click a student for the same menu. Drag to orbit, scroll/pinch to zoom
(clamped), and **Reset view** (↻ in the widget bar) returns to the overview. A
vertical one-finger swipe over the scene still scrolls the page on a phone.

**Headmaster powers** (only for sessions you can manage — Agents:Edit and a
non-viewer role in that session's workspace; the daemon re-checks):

- **Kick out…** deletes the session through the same path as every other
  delete (`DELETE /sessions/{id}`), after a danger confirm naming the session,
  its workspace and that its whole history goes with it (there is no Undo); a
  working agent gets an extra "it is mid-turn" warning. The student stands up
  and walks out of the door (instant under reduced motion), its desk stays
  empty, and a "Kicked out …" toast confirms it. A failed delete puts the
  student back and toasts the reason.
- **Detention** archives the session (resumable) with the usual "Session
  archived" toast and its Undo.

**List view and accessibility.** The 3D / List switch (stored per widget) shows
the same classrooms as a list of real buttons. In 3D view that list is still in
the page for keyboard and screen-reader users: focusing a student highlights it
in the scene and shows its card; each row has a "More actions" menu. Without
WebGL the widget falls back to the list with a note.

**Performance.** `three` is lazy-loaded on first mount (bundle-budget's
`LAZY_ONLY_PKG` keeps it out of the Home and Agents page chunks); every repeated
part is one `InstancedMesh`, so 20 rooms × 30 students is a constant number of
draw calls. Frames render on demand: only while something animates (ambient
motion at ~30 fps), the camera moves or data/theme changes — never while the
widget is off screen, its space is inactive, or the tab is hidden; nothing
animates under `prefers-reduced-motion`. Colours are read from the CSS tokens
and re-read on every theme / scheme / accent change.

**Data.** `GET /sessions?archived=false&limit=1000` (#17b, every workspace you
can see, owner-scoped) refetched on `session_created` / `session_removed` /
`session_archive_changed` / `session_renamed` (30 s only while the event
socket is down), overlaid with the workspace store's rows; status and "needs
you" come from the store's event-fed maps.

Every box's poll is a **setTimeout chain** with ±failure back-off (×2 per
consecutive failure, capped ×8) and skips ticks while the tab is hidden, so a
Home page left open overnight doesn't hammer a broken backend.

## API / WS surface

Home has **no endpoints of its own**. Boxes reuse the read endpoints of their
modules (`/sessions` for Classrooms, plus `DELETE /sessions/{id}` and
`POST /sessions/{id}/archive` for its kick-out / detention, `/workspaces/{ws}/workgraph/summary|items`, `/workspaces/{ws}/db/dashboards|widgets`
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
- **Every space stays mounted.** Switching spaces only toggles `hidden` +
  `inert`; boxes in an off-screen space (or under a zoomed box) get
  `active={false}`, stop polling and keep their data, and resume without an
  immediate fetch while that data is within their cadence. A DB-dashboard
  widget set to **manual only** runs once per widget, never again because its
  space came back on screen.
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
