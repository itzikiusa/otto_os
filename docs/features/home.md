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
| **Classrooms** | `agents` | Otto School — a 3D school: a corridor with a door per workspace, classrooms where every session is a kid at a PC showing its live terminal, a robot headmaster on patrol — see [Classrooms](#classrooms-otto-school) | live (session events + the workspace store; screens every 2 s) |

### Classrooms (Otto School)

A real 3D school of everything you have running. Add it from **Add widget →
Classrooms** (default 8 × 6; it is not in the first-visit seed); **Fill the
page** (⤢ in the widget bar) gives it the whole Home area.

**The corridor.** The school opens in a corridor with lockers, a ceiling and
lamps, and **one door per workspace** — the current workspace first, then the
busiest. Each door has a name plate and a big live sign ("✋ 1 needs you ·
● 2 working · ○ 3 idle"); a room that needs you lights its sign amber (red
when a session failed) and floats a ✋ N badge over its door. With nobody
waiting, a chip at the top-left walks you straight into the **busiest room**.
Drag to look around, **W A S D / arrow keys** to walk, the wheel to step
(when the school has focus or fills the page); hover a door for its summary
and **click it to walk in** (Enter walks through the door you face).

**The classroom.** Behind each door is a classroom built from the Blender
kit: tiled floor, a ceiling with light panels, windows on one wall, a green
board with the workspace name, a bookshelf, posters, a corkboard, plants, a
clock, the headmaster's desk, a detention bench — and a full grid of student
desks, each with a PC. **Light** follows the theme: the light theme is a sunny
afternoon (a low sun through the windows throws soft desk shadows, warm lamps
fill in); the dark theme is evening (moonlight, the lamps carry the room, the
window views go dusky). Every
agent session is a **kid at a desk**, one character per provider (Claude,
Codex, Grok, Antigravity, Shell; any other provider gets the generic kid).
Seating is stable (oldest session first) so a status change never shuffles
the room. **Engine sessions** (workflow steps, swarm, review agents, scheduled
tasks… — live ones only) sit in the back row. Up to 36 front + 6 back seats
per room; the rest are counted in the list.

| Session state | Kid | Pin over its head | PC |
|---|---|---|---|
| working (producing output) | types | green | the live terminal scrolls |
| needs you | hand up, a pulsing amber ring on the floor; the headmaster walks over, reads the screen, turns to you and waves | amber, bobbing | amber "Waiting for you" banner |
| idle / running (alive, quiet) | sits back — and may get up and wander; after 15 min idle it **dozes** (head down) | — | last output |
| just stopped working (turn finished / ended / now waits for you) | stands up, **cheers** (waves) and sits back down; the headmaster gives a thumbs-up | — | — |
| reconnecting (events socket down) | sits still | grey | last output |
| ended (exit 0) | stays, relaxed | blue | last output + green "✓ Finished" |
| failed (non-zero exit, when known) | slumps; the headmaster checks it right after the needs-you kids | red | red "Exited with an error" banner |
| suspended (resumable) | empty chair | — | sleeping screen |
| archived | on the detention bench, head down | — | — |

Every animation is driven by the session's real state or a real change of it
— nobody cheers, raises a hand or slumps at random. Pins hide while you lean
over a monitor (Look at screen). The headmaster is drawn a little smaller than
the kit's model so he never hides a whole row.

**Live screens.** Each monitor shows that session's current terminal screen
(`GET /sessions/{id}/screen`), polled every 2 s for the kids the camera can
see in the room you are in (at most 12), and never while the widget is off
screen. The monitor of the selected kid (and the one you lean over with Look
at screen) is repainted at 2× so it stays crisp up close.

**The room lives.** Idle kids get up now and then (at most a third of them at
once) and walk the aisles to the windows, the bookshelf, the board, the
posters or a classmate's desk to chat — and hurry back the moment their
session starts working or needs you. **Otto, the robot headmaster**, patrols
the lanes: kids that need you first (he reads their screen and nods), then
working kids he hasn't checked lately, and in between he points at the board.
Under `prefers-reduced-motion` everybody stays put and camera moves cut.

**Needs you, at a glance.** A **queue** on the stage (top-right in the
corridor, top-left in a room, where the kid card owns the right) lists
every session that needs you, school-wide — longest waiting first, then the
failed ones (collapsible; the first five, the rest in the list). Click a row
to walk into its room onto that kid (its card opens); ↗ opens the session
directly. The List (and the keyboard / screen-reader list in 3D) starts with
the same **Needs you** section.

**Interaction.** Hover a kid for its tooltip — title, provider, state, when it
was last active, its branch and a **peek** at the last 8 lines of its live
terminal as real text; **click** for its card (title,
provider, status, last active, folder, branch) with **Open session**, **Look
at screen** (the camera leans over the kid's shoulder onto the monitor),
**Check on** (sends the headmaster over), **Detention** and **Kick out…**;
**double-click** opens the session. Right-click a kid for the same menu. Drag
orbits, W A S D pans, the wheel zooms; Esc steps back (screen → room →
corridor), and the breadcrumb in the widget bar does the same.

**Headmaster powers** (only for sessions you can manage — Agents:Edit and a
non-viewer role in that workspace; the daemon re-checks):

- **Kick out…** deletes the session (`DELETE /sessions/{id}`) after a danger
  confirm naming the session, its workspace and that its history goes with it
  (no Undo; a working agent or an engine-owned session gets an extra warning).
  The kid stands up and walks out of the door while the headmaster scolds; a
  failed delete walks it back and toasts the reason.
- **Detention** archives the session (resumable; the usual toast with Undo);
  the kid walks to the detention bench. The bench seats the room's three most
  recently archived sessions — **Release from detention** unarchives one. In the
  list, clicking a detention row selects the kid, or shows its menu in List view.
  It never unarchives. Unarchiving is the row's own **Release** button, which is
  disabled, with the reason, for people who can't manage the session. Closing the
  card, or a detention or kick from it, returns focus to the 3D stage.

**List view and accessibility.** The 3D / List switch (stored per widget)
shows the same classrooms as real buttons; in 3D the list stays in the page
for keyboard and screen-reader users (focusing a kid walks into its room and
selects it). Without WebGL — or after the 3D context is lost, with Retry — the
widget falls back to the list.

**Assets and performance.** The characters and the kit are glTF files in
`ui/public/school/`, built headlessly by Blender from
`ui/assets-src/school/` (`blender/build_school.py`; CC0 sources and licences
in `LICENSES.md`; the node / clip contract in `CONTRACT.md`). `three` and the
models load on first mount only (bundle-budget's `LAZY_ONLY_PKG` keeps three
out of the Home chunk). Only the room you are in is built; static kit pieces
are `InstancedMesh`es, the status pins and the needs-you rings are one
`InstancedMesh` each (re-placed per frame without allocating), the lamp count
is fixed (no shader recompiles on a theme switch). The loop runs at ≤ 30 fps
while the box is on screen and active — it pauses when the widget is
scrolled out of view, its space is inactive or the document is hidden — and
the corridor renders on demand. A software renderer drops shadows and half
the lamps. A lost WebGL context remounts the school on its own (at most twice
a minute, keeping the room) before falling back to the list with Retry. Live screens poll
on the open room's id, so a session status event does not restart the poller.
A status change can refresh the screens early, at most once every 2 s. The
session list loads through `freshness()` like the other boxes, so scrolling
back, sliding back or leaving zoom does not refetch it. While the box is zoomed,
the grid copy releases its 3D school, so only one WebGL context is live.

**Data.** `GET /sessions?archived=false&limit=1000` plus
`GET /sessions?archived=true&limit=60` for the benches (#17b, every workspace
you can see, owner-scoped), refetched on `session_created` /
`session_removed` / `session_archive_changed` / `session_renamed`, overlaid
with the workspace store's rows; status and "needs you" come from the store's
event-fed maps.

Every box's poll is a **setTimeout chain** with ±failure back-off (×2 per
consecutive failure, capped ×8) and skips ticks while the tab is hidden, so a
Home page left open overnight doesn't hammer a broken backend.

## API / WS surface

Home has **no endpoints of its own**. Boxes reuse the read endpoints of their
modules (`/sessions` and `/sessions/{id}/screen` for Classrooms, plus `DELETE /sessions/{id}`,
`POST /sessions/{id}/archive` and its unarchive for kick-out / detention / release, `/workspaces/{ws}/workgraph/summary|items`, `/workspaces/{ws}/db/dashboards|widgets`
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
