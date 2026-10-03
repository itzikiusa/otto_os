# Otto Agent Sessions — User & Operator Guide

Agent Sessions are Otto's core: it runs coding-agent CLIs — `claude`, `codex`,
`agy`, and a plain `shell` — as **real PTY-backed terminals** you can watch,
split, tile, type into, and walk away from. Sessions survive daemon restarts
(they are *resumable*), idle-suspend to free RAM without losing the
conversation, and have their workspace folder *auto-trusted* so an unattended
agent never stalls on a "do you trust this folder?" prompt. This guide is the
authoritative end-user + operator reference: every endpoint, WebSocket frame,
setting key, and UI label below is grounded in the code, not invented.

> The wire contracts are authoritative in `docs/contracts/api.md` (REST) and
> `docs/contracts/ws.md` (WebSocket). This doc explains *how the feature
> behaves*; when in doubt, the contract wins.

---

## 1. Overview

A **session** is a row in the daemon's SQLite store plus, when live, a PTY child
process the daemon owns. Two kinds exist (`SessionKind`):

- **`agent`** — an agent CLI (`claude` / `codex` / `agy`) or a plain `shell`.
- **`connection`** — a terminal opened from a saved connection profile (SSH,
  MySQL, Redis, MongoDB, ClickHouse, custom). Connection sessions are created by
  `POST /connections/{id}/open`, not by the agent-create path, but they share
  the same PTY, terminal-WS, lifecycle, suspend, and trail machinery described
  here. See `./connections.md`.

Channels (Slack/Telegram), the Agent Swarm, and PR Review all spawn **agent
sessions under the hood** — every agent Otto runs is one of these sessions, with
the same terminal, trail, resume, and suspend behavior (see §13).

The daemon (`ottod`) listens on `127.0.0.1:7700` (loopback only by default). The
Svelte UI talks to it over HTTP + two WebSockets. The architecture:

```
Svelte UI ──HTTP+WS──▶ ottod (127.0.0.1:7700) ──spawns──▶ claude / codex / agy / shell  (PTY)
```

### Where it lives

| Concern | Location |
|---|---|
| Session manager, PTY ownership, status tasks, suspend/resume, trust call, ingest tokens | `crates/otto-sessions/src/manager.rs` |
| PTY plumbing + scrollback ring buffer | `crates/otto-pty/src/{lib.rs,ring.rs}` |
| Provider registry (claude/codex/agy/shell launch + resume args) | `crates/otto-sessions/src/providers.rs` |
| Pre-spawn folder trust (writes each CLI's trust config) | `crates/otto-sessions/src/trust.rs` |
| Runtime prompt-guard (auto-accepts stray approval prompts) | `crates/otto-sessions/src/prompt_guard.rs` |
| Resumability checks (transcript existence) | `crates/otto-sessions/src/lifecycle.rs` |
| Terminal WebSocket server (frames, scrollback, search, role gate) | `crates/otto-sessions/src/ws.rs` |
| Activity trail + task tracker ingest endpoints | `crates/otto-server/src/routes/activity.rs` |
| RBAC route → capability map | `crates/otto-server/src/policy.rs` |
| Domain types (`Session`, `SessionStatus`, `TrailEvent`, `AgentTask`, …) | `crates/otto-core/src/domain.rs` |
| Scratch workspace (`SCRATCH_WORKSPACE_ID`, `ensure_scratch`, implicit Editor, `GET /workspaces/scratch`, 409 guards) | `crates/otto-core/src/domain.rs`, `crates/otto-state/src/workspaces.rs`, `crates/otto-server/src/routes/workspaces.rs`, `crates/ottod/src/main.rs` (boot) |
| Create / input DTOs | `crates/otto-core/src/api.rs` |
| New-session modal | `ui/src/modules/agents/NewSession.svelte` |
| Session pane (header, ⋯ menu, status, idle countdown) | `ui/src/modules/agents/SessionView.svelte` |
| Split panes + broadcast bar | `ui/src/modules/agents/Splits.svelte` |
| Tiled / grid view (live-tile budget) | `ui/src/modules/agents/TiledView.svelte` |
| Terminal component (xterm + WS) | `ui/src/lib/components/Terminal.svelte` |
| Activity trail + task tracker panel | `ui/src/modules/panels/ActivityPanel.svelte` |
| Right-panel Canvas tab (referenced diagrams) | `ui/src/modules/panels/CanvasPanel.svelte`; routes `crates/otto-server/src/canvas_refs.rs` |
| Per-device isolation toggle | `ui/src/modules/settings/Appearance.svelte`, `ui/src/lib/stores/ui.svelte.ts` |

---

## 2. Creating & opening a session

### From the UI

Open **New Session** (the agents page **New Session** button, or `⌘T`). The
modal (`NewSession.svelte`) offers:

- **Workspace** — a two-way switch: the **current workspace** (its name) or
  **No workspace**. With no workspace at all only "No workspace" exists and is
  pre-selected. "No workspace" starts a *workspace-less* session (below); the
  palette command **New Session (no workspace)** and the sidebar's **New
  session (no workspace)…** menu items open the sheet with it pre-selected.
- **Provider** — cards for `claude` ("Claude Code CLI"), `codex` ("Codex CLI"),
  `shell` ("Plain shell"), plus any custom providers from `GET /meta.providers`.
  The configured default provider is pre-selected and carries a **default**
  badge. Clicking a card (or `←`/`→`) selects it exclusively, as always.
- **How many — the `−` / `+` stepper on each card** (`+`/`-` on the keyboard for
  the focused card). Counts add up ACROSS cards, so "2 codex, 3 claude and 1
  shell" is one trip through the sheet: the footer button becomes **Start 6
  Sessions**, a line under the grid spells the batch out, and the sessions are
  started provider by provider in grid order, then opened **tiled** (exactly
  where a `spawn 3 claude agents` command-palette plan lands them). Capped at 20
  per provider. A typed **Title** becomes a base name numbered per session
  (`"fix flaky tests 1" … "fix flaky tests 6"`); the **model** pin is only
  offered for a batch that targets a single provider, since model ids are
  provider-specific.
- **Title (optional)** — placeholder shows the auto-generated name. If left
  blank the daemon names the session `"{provider} #{n}"`, where `n` is one more
  than the current count of that provider in the workspace.
- **Working directory** — *"Defaults to the workspace root"* if blank. Any
  folder on the daemon host works; it does **not** have to be inside a
  workspace, so a one-off job somewhere else needs no new workspace. **Browse…**
  opens the shared daemon-side folder picker (`GET /fs/browse`, using the OS
  permissions of the daemon account — see [daemon-http-api](./daemon-http-api.md)), and the
  picker provides **Back**, **Forward**, **Up**, and clickable path segments
  for jumping to any ancestor. Search still filters the current listing, with
  **Show hidden** available. Hidden folders such as `.ssh` follow those same OS
  permissions. If opening a folder fails, the error identifies the attempted path;
  the breadcrumb is labeled **Last opened folder**, with a button to return there.
  **Favorites** stores folders you pin, and **Recents**
  lists the last 20 distinct folders opened. These shortcuts persist locally
  for the current daemon/user; every jump still checks access permissions. The same navigation appears in every shared
  folder/file picker across Otto. The working-directory field offers your recently used directories (this workspace's root first, then
  the cwds of its existing sessions) as a datalist. The daemon `mkdir -p`s the
  directory if it does not exist (a missing cwd would otherwise make the child
  fall back to `$HOME`).
- **Additional directories (optional)** — extra repos the agent may access,
  *"passed as `--add-dir`"*. Stored in `meta.extra_dirs` (an array). Honored by
  `claude` / `codex` / `agy`; ignored for `shell`. Same **Browse…** picker.
- **Browser tools** — shown only for `claude` / `codex`: *"Give the agent a real
  browser via MCP (navigate, click, read pages)."* Stored as `meta.browser:
  true`; wires an MCP browser server into the workspace `.mcp.json`.
- **Preview context** — for `claude` / `codex`, expands to show exactly what
  Otto would inject (skills/soul/context) before spawning.
- **First message (optional)** — shown when the batch has an agent provider.
  When filled, each agent session is created through
  `POST /workspaces/{wid}/sessions/open` with `prompt`: the daemon waits for the
  CLI to be ready (cold start, trust dialog), pastes the message, verifies the
  echo and presses Enter — no client-side timer racing the spawn. The sheet
  always sends `meta.work = {origin: "manual"}` (the route otherwise stamps
  `delegation`, which would make it engine-owned with the 5-minute idle grace).
  A batch sends the message to every agent; `shell` cards in the batch start
  normally. `⌘↩` still creates. The first-run coach's **Start first session**
  uses the same path for its starter prompt.

Press **Start Session**. A session can also pin a model: when `meta.model` is
set, the daemon appends `--model <name>` for `claude` / `codex` (silently
omitted for `agy` / `shell`).

### Workspace-less (scratch) sessions

A session does not have to belong to one of your workspaces. Pick **No
workspace** in the sheet (or the palette command / sidebar menu item) and the
session is created in the daemon's **scratch workspace** — a system-owned,
hidden workspace row with the fixed id `scratch`, created (and healed) on every
daemon boot by `WorkspacesRepo::ensure_scratch`, whose `root_path` is the
daemon user's `$HOME`. Because it *is* a workspace row, everything that keys on
`workspace_id` — RBAC, WS delivery, archive / resume / restart, handover,
trail, transcripts, shares — works unchanged. What differs:

- **Hidden.** It never appears in `GET /workspaces`, the workspace picker, the
  sidebar Workspaces section, onboarding, or the host selection of vault /
  insights / self-improvement runs. `GET /workspaces/scratch` is its one read
  route; `PATCH` / `DELETE /workspaces/scratch` and member edits answer 409.
- **Everyone is an Editor there** (implicitly — no membership rows), so any
  authenticated user can `POST /workspaces/scratch/sessions`. Isolation still
  holds: `GET /workspaces/scratch/sessions` lists only the caller's own
  sessions (root: all), and every session route stays owner-or-admin. Root
  is Admin everywhere, including scratch.
- **Default folder is `~`.** In scratch mode the sheet defaults the working
  directory to the scratch `root_path` (the daemon `$HOME`), offers the cwds of
  your other scratch sessions as recents, uses the *global* default provider
  (there is no workspace setting), hides the workspace-scoped **Preview
  context**, and shows the one-line notice *"Home folder: the agent is trusted
  for, and may write anywhere under, ~"* whenever the folder is the home
  directory (see §5 — trust follows the session cwd). Any other folder is
  confined exactly like a workspace session.
- **Sidebar.** Scratch sessions are listed in a **No workspace** group (home
  icon, tooltip "Sessions not tied to any workspace") under the flat Agents
  list — present in every workspace and when you have none. Right-click the
  group (or any Agents row / header) for **New session (no workspace)…**. They
  are loaded beside the current workspace's sessions on every refresh, so
  tabs, panes, status dots, archive and rename behave as usual; with zero
  workspaces the tab/pane layout is persisted under the `scratch` key
  (`otto_tabs_scratch` / `otto_panes_scratch`) so it survives reloads too. The
  **tiled grid** stays the current workspace's, though: a scratch session joins
  it once you open it (it is then an open tab), and with no workspace selected
  the grid is made of them alone.
- **Handover stays same-workspace.** The daemon rejects cross-workspace
  handovers, so a scratch session hands over only to a *new* agent or to
  another scratch session; the target picker lists exactly those.

### Default provider resolution

New sessions, channel replies, and review agents fall back to a configured
**default agent** when no provider is chosen. Resolution order:

1. The workspace's `default_provider` setting (per-workspace).
2. The global `default_provider` setting.
3. `"claude"`.

`GET /meta` reports the resolved global default in `default_provider` and the
full provider list in `providers`.

### What the daemon does on create

`SessionManager::create` (`manager.rs`):

1. Resolves `cwd` (request `cwd` → else workspace `root_path`).
2. Generates a provider session id (a UUID — `claude --session-id` requires
   one), builds the launch `CommandSpec` from the provider registry, and appends
   `--add-dir` (from `meta.extra_dirs`) and `--model` (from `meta.model`) args.
   `provider_session_id` is stored up front only for claude (the id Otto
   assigned); codex/agy mint their own and it is captured from disk shortly
   after spawn, and a `shell` gets one only if you launch an agent CLI inside it
   (see [A terminal you ran an agent in](#a-terminal-you-ran-an-agent-in)).
3. Writes the session row, then `mkdir -p`s the cwd.
4. **Pre-trusts** the folder for the provider (`trust::ensure_trusted`, §6).
5. **Reconciles** MCP servers into the workspace `.mcp.json` in one guarded
   read→write (browser if opted in, the user's enabled `mcp-servers`, and
   Otto's first-party `otto` tool server when the workspace opted in via
   `otto_mcp_enabled`). An `ottoManagedServers` marker tracks what Otto wrote:
   disabled/deleted servers are removed on the next spawn or restart, while
   hand-added entries are never touched. Codex gets the user servers as
   per-spawn `-c` overrides; grok as `.grok/config.toml` tables. The resolved
   name list is snapshotted into `meta.mcp_servers`. All opt-in, best-effort,
   never blocks the spawn.
6. Runs the context pre-spawn hook (materializes skills/soul/context) — skipped
   for review sessions.
7. Injects the **ingest env** (`OTTO_INGEST_BASE`, `OTTO_SESSION_ID`,
   `OTTO_INGEST_TOKEN`) so the agent's hooks can post activity back (§7).
8. Restores the saved terminal grid (`meta.pty_cols` / `meta.pty_rows`, else
   80×24) and spawns the PTY, then starts the per-session status task.

The **provider registry** (`providers.rs`) launch specs — each agent CLI runs
*without* `-p`, with its own skip-permissions flag so an unattended session
never blocks on a tool-approval prompt:

| Provider | Launch args | Resume |
|---|---|---|
| `claude` | `--session-id {sid} --dangerously-skip-permissions` | **yes** — `--resume {sid} --dangerously-skip-permissions` |
| `codex` | `--dangerously-bypass-approvals-and-sandbox --search` | no |
| `agy` | `--dangerously-skip-permissions --add-dir={cwd}` | no |
| `shell` | `$SHELL -l` (default `/bin/zsh -l`) | no |

Custom providers can be added/overridden live via the `providers` setting JSON
(`{"<name>":{"cmd","args","resume_args","update_command"}}`); `{sid}` and
`{cwd}` are template-expanded. Reloads without a daemon restart; existing
sessions keep running.

---

## 3. The terminal

Each pane mounts an xterm.js terminal wired to the daemon over a per-session
WebSocket.

### WebSocket protocol — `WS /ws/term/{session_id}`

**Auth:** a bearer token validated **before** the upgrade. Preferred:
`Sec-WebSocket-Protocol: otto-bearer, <token>` (the server echoes `otto-bearer`,
keeping the token out of the URL). `?token=<bearer>` is accepted as a
backward-compatible fallback. An IP that fails token validation too many times
is locked out (HTTP 429) — see §11.

**Role:** workspace **viewer** may attach read-only; **editor**+ may send
input/resize. Input frames from a viewer are dropped server-side, and a single
`{"type":"error","code":"forbidden","message":"viewers cannot send input"}` is
sent once. (For mobile share-links the input right comes from the share's role.)

**Client → server** (JSON text frames):

```json
{"type":"input","data":"<base64 bytes>"}
{"type":"resize","cols":120,"rows":32}
{"type":"scrollback","lines":2000}
{"type":"search","query":"foo"}
```

**Server → client:**

- **Binary frames** — raw PTY output bytes, written straight into xterm.
- **JSON text frames:**

```json
{"type":"scrollback","data":"<base64 bytes>"}      // reply to a scrollback request; sent BEFORE live bytes resume
{"type":"status","status":"running|working|idle|exited|reconnectable"}
{"type":"exit","code":0}                            // child exited; socket stays OPEN so you can read final output
{"type":"terminated"}                               // force-terminated (admin terminate / share-link revoke); socket closes right after
{"type":"error","code":"forbidden","message":"..."}
{"type":"search_result","query":"foo","matches":[{"line":42,"text":"foo bar baz"},...]}  // up to 200 matches
```

On attach the server immediately sends the current `status`. Multiple clients
may attach to one session at once; all receive the same output broadcast and
input is interleaved in arrival order. The server pings every 30s.

### Input & output

Output streams as binary frames. Input is base64-encoded bytes in an `input`
frame. When Otto *submits a message as if you typed it* (handover injection,
broadcast, programmatic `submit_text`), it wraps the text in a **bracketed
paste** (`ESC[200~ … ESC[201~`), waits ~200 ms for the TUI to absorb it, then
sends a separate carriage return (`\r`). Writing `"text\n"` in one burst would
make a bracketed-paste TUI (Claude Code, Codex) treat the trailing newline as
pasted content and *not* submit — so the paste-then-Enter split is the reliable
"actually send" path. The same bracketed-paste wrapping is applied to UI-driven
injections (`ws.injections`).

### Scrollback

The daemon keeps a persistent **ring buffer of 10,000 lines / 2 MiB** per
session (`otto-pty/ring.rs`) that **survives WS reconnects**. On every
(re)connect the client sends `{"type":"scrollback","lines":2000}`; the server
replies with one coherent payload: up to `lines` rows of off-screen history
(plain text, scrolled into xterm's own buffer) **followed by** the current
screen frame (`ESC[2J ESC[H` + formatted state, cursor and input box included),
so the live screen redraws once with no double-render. A `lines` of `0` is
substituted with `DEFAULT_ATTACH_HISTORY_LINES` (1000) so even a minimal client
restores ample context. Over-asking (2000 requested, ≤10,000 retained) is
clamped, never an error.

The attach request also carries the pane's measured grid
(`{"type":"scrollback","lines":…,"cols":…,"rows":…}`): a pane that holds size
authority has the PTY resized to that grid **before** the snapshot is taken,
so opening a pane costs exactly one snapshot. The depth a client asked for is
reused for every snapshot the daemon later pushes on its own (lag, flow-control
recovery, a respawned process), so a 2000-row tile is never sent 4000 rows.

**Memory.** The daemon's emulator keeps 4000 rows of formatted history per live
session. Rows that scrolled off are stored trimmed of trailing blanks and shared
between snapshot copies, so a session of short lines at 200 columns holds about
2.4 MB instead of 25 MB, and taking a snapshot no longer copies the history. A
live terminal **nobody has viewed for 10 minutes** keeps only its newest 1000
rows of emulator history; the next viewer restores the 4000-row cap and history
grows again from there (the 10,000-line raw ring used by search is unaffected).
For a session that survives daemon restarts, the PTY holder's own emulator (the
copy a restarted daemon re-adopts) follows the same cap, and the holder keeps no
raw ring of its own (search is daemon-side).

In the app, a primary pane keeps **4000 rows** of xterm scrollback, the same as
the daemon: a snapshot can never restore more, so deeper local history only cost
memory until the next rebuild. Grid tiles and embedded previews keep 2000.
Terminals parked while you are elsewhere in the app (so coming back needs no
replay) are bounded by count (12) and by an estimated **48 MB** of buffer; the
least recently parked go first.

**Two searches:**
- **In-viewport** — the xterm `SearchAddon` over the currently rendered buffer
  (instant, but lost on reconnect).
- **Server-side ring search** — the `{"type":"search"}` frame greps the full
  10,000-line ring (plain substring, case-insensitive, ANSI-stripped) and
  returns up to 200 matches in buffer order (the newest 200 when more match; the
scan runs off the daemon's async workers and never blocks the session's
output). Use it after reopening a session or
  to find output that scrolled off. The UI find bar runs both: local first, then
  a 300 ms-debounced server query.

The find bar searches upward from the newest output (`↵` older, `⇧↵` newer) and
highlights every local hit with an `N/M` count. The server hits list under the
bar; `↑`/`↓` or a click steps through them, and the viewport only jumps there
by itself when the client buffer has no hit. `⌘F` reaches the terminal of the
active session pane even without keyboard focus in the xterm (the header search
button does the same). A focused query editor, text field or open modal keeps
`⌘F`, and so does the Chat view, where the page find covers the transcript.
Routing: `routeFind` in `ui/src/lib/keys.ts`; the terminal opts in with
`Terminal findRank`.

### Resize

`{"type":"resize","cols","rows}"` triggers `manager.resize`, which both resizes
the PTY and persists the grid to `meta.pty_cols` / `meta.pty_rows` so a future
respawn frames its first snapshot correctly. The client only sends a resize on
an *actual* dimension change (not on every `fit()`), to avoid SIGWINCH flicker
on `claude`/`codex` repaints.

Agent panes go one step further: while the pane's box is still settling (a tab
switch, a split animation, a window restore) they only *measure*, and resize
the local xterm once the same grid has measured twice (≈350 ms). A passing size
never reflows the TUI's screen. If the local grid did change and came back to
the size the PTY already has (no SIGWINCH, so no repaint), the pane asks for a
fresh snapshot instead. Such a "compact" only runs when the grid grew wider or
the height changed by more than two rows, one pane at a time per window (the
focused pane first); panes that are off-screen or in a hidden window compact
when they come back into view. A pane parked by a tab switch is put back to the
PTY's grid first, and while parked (or in a hidden window, unless focused) it
stops acknowledging output: the daemon holds or drops what it would have sent
and the pane catches up with one snapshot when it returns. If a pane still looks garbled, **⋯ → Redraw terminal** (or ⌘K
"Redraw terminal") rebuilds the screen from the session without reconnecting.

### Watching, splitting, tiling

- **Split view** (`Splits.svelte` + `SplitNode.svelte`) — a nested split **tree**:
  split any pane left/right/up/down, drag **any** gutter (each split node has its
  own fraction, 0.1–0.9, and a drag refuses to take either side below 160 px), up
  to **15** panes. Presets — *Equal columns*, *Equal rows*, *One above two*, *One
  beside two*, *Grid* — sit in the pane ⋯ menu (and on the Database pane's ✕
  right-click) and in `⌘K`. The layout persists per workspace in
  `otto_panes_<ws>` as v2 `{v, tree, focused}`; an old v1 `{panes, axis}` payload
  still restores, with the old per-window `otto_split_col_frac` /
  `otto_split_row_frac` as the root fraction. With ≥2 panes and ≥2 session targets
  a **broadcast bar** appears: *"↗ broadcast"* sends one line to all visible
  sessions via `POST /workspaces/{id}/broadcast {text, session_ids}`.
- **Reordering panes** — drag a pane by its **title** (or the grip that appears
  at the header's leading edge on hover) in its header: drop on
  another pane's **centre** to swap the two sessions, on an **edge quarter** to
  move the pane into a new split beside it. Without a mouse: `⌘⌥←/→/↑/↓` move the
  focused pane to its geometric neighbour and `⌘⌥S` swaps it with the next (in a
  Database pane `⌘⌥←/→` stay with the query editor's tab switch), and the same
  moves plus every preset are `⌘K` commands.
- **Sidebar order** — the flat Agents list is rendered in the order the daemon
  returns (`GET /workspaces/{id}/sessions` is `ORDER BY created_at` — creation
  order, oldest first; the control still calls it *Recent*) until you drag a row.
  Dragging switches the list to *Manual* (the sort control in the group header);
  there, sessions the manual order has never seen go on TOP by `last_active_at`,
  newest first, and *Reset to recent* returns to the daemon's order.
  Telegram/Slack lists and other-workspace groups cannot be reordered, nor can a
  filtered list. Persisted as `otto_session_order_<ws>`.
- **Pane header** — one 30 px row whose priority is *status dot + title*: the
  title is the only item that grows, everything else shrinks or folds before it
  does. Inline: the Terminal · Chat toggle, the details chip, zoom (tiled) and
  ⋯ (+ ✕ in a split). The **details chip** is the provider icon (+ name, idle /
  suspend countdown and folder on a wide pane); its tooltip and its click menu
  hold the rest — themed full name, subscription account, state, tasks, the
  full cwd (*Copy folder path*) and the handover source. The **⋯ menu** holds
  the pane controls that used to be header buttons — *Terminal font larger /
  smaller / Reset* (`⌘+` `⌘−` `⌘0` in the terminal still work), *Copy on
  select*, *Restart session*, *Allow UI control* — plus every session action.
  The UI-control toggle only shows inline while it is ON (or the agent asked
  again). The active pane reads at full contrast; the others dim their title
  and controls until hovered.
- **Pane header at narrow widths** — CSS container queries on the pane
  (`ui/src/lib/paneHeader.ts` holds the breakpoints and the script twin that
  adds the hidden controls back into ⋯):

  | Tier | Pane width | Header |
  |---|---|---|
  | full | ≥ 720 px | dot · title · task / now / handover chips · details chip with text · labelled toggle · zoom · ⋯ · ✕ |
  | compact | 420–719 | same, but the toggle and chips are icon-only and "Now: …" is hidden |
  | minimal | 200–419 | dot · title · one *Switch to …* button · ⋯ — zoom, ✕ and the details move into ⋯ |
  | micro | < 200 | dot · title · ⋯ — *Terminal view* / *Chat view* are ⋯ rows |

  The header height never changes and nothing is ever clipped: the title
  ellipsizes (full text in its tooltip) instead.
- **Tiled view** (`TiledView.svelte`) — see every session at once in a grid (1→2
  →3→4 columns by count). Drag a tile onto another to reorder; the order is
  remembered per workspace (`otto_tile_order_<ws>`). To preserve the idle-suspend
  memory design, **at most `MAX_LIVE_TILES` = 15 tiles are live** (open a WS + resume): always the focused
  tile, then user-pinned tiles, then visible tiles (tracked by an
  `IntersectionObserver`), up to the cap. Everything else is a lightweight
  **placeholder** — a header + status dot + provider chip + *"Click to attach"* —
  that opens **no terminal, no WebSocket, no resume**. Clicking a placeholder
  pins and focuses it; scrolling a live tile off-screen (over budget) tears down
  its WS so the session can re-suspend. Without this, opening a tiled view of M
  suspended sessions would wake all M agents (~200 MB each) at once.

### Clickable URLs and file references

In a local agent terminal, click an HTTP(S) URL to open the system browser, or a
file reference such as `ui/src/App.svelte:42:3`, `../Cargo.toml#L8`, or
`/tmp/report.md` to open the Files viewer. Hover shows the resolved target.
Relative paths and source basenames such as `SKILL.md` resolve from the session's
saved working folder. A CLI may print only a basename for a nested file; Otto
cannot infer its hidden directory. Use the full path if the resolved file is
missing; the viewer error includes the attempted path. Changing directories
inside a shell command does not change that saved session folder.

Soft-wrapped paths remain clickable across rows. Explicit filesystem paths can
include spaces, such as `/Users/me/Library/Application Support/report.md`.
Provider-rendered paths may also span hard terminal rows: Otto recognizes bounded
continuations of a single styled or delimited path, retaining the complete target
when either fragment is clicked. Separate file references are not concatenated.
Quoted paths, Markdown destinations and OSC 8 hyperlinks make ambiguous filename
boundaries explicit. Native OSC 8 destinations take
precedence over their visible labels. Only HTTP(S) and local file destinations
are dispatched; text is never executed as a shell command, and agent task names
such as `/root/git_fetch` do not become application routes. Remote connection
terminals and guest-share terminals provide URL links only.

Opening an explicit file navigates the primary Files section to that file's
parent folder without changing the active workspace or the session's cwd. The
file remains visible even if its parent cannot be listed, or no workspace is
selected. File reads and folder listings follow the daemon's macOS permissions;
a failed read displays its path and error. New clicks replace older pending
reads, so a slow response cannot replace the latest file.

### Copy & paste

The terminal is a canvas, not a document, so the browser has no DOM selection to
copy from — xterm only syncs its selection into the hidden textarea on
**right-click**. Otto therefore handles the chords itself:

> **⌥-drag to select in an agent session.** Agent CLIs (claude, codex) — like
> vim or htop — turn on **mouse reporting**, and a terminal that reports the
> mouse gives drags to the *application*, not to selection: xterm hands the
> mousedown to the app and cancels it, so no selection is created at all. The
> macOS escape hatch is **⌥ Option + drag** (`macOptionClickForcesSelection`,
> set in `Terminal.svelte`) — the same gesture iTerm2 and Terminal.app use.
> Plain drag still belongs to the app, deliberately: stealing it would break the
> TUI's own click handling.
>
> This is worth knowing because of how it FAILS: with no selection there is
> nothing to copy, so ⌘C does nothing, right-click ▸ Copy shows up **greyed**,
> and copy-on-select never fires. That looks exactly like a broken clipboard and
> sends you hunting through permissions and secure contexts — the clipboard is
> fine; there is simply no selection.

| Gesture | Behaviour |
|---|---|
| ⌥-drag (mouse-reporting sessions) | Forces a local selection. Without it, agent sessions cannot be selected at all on macOS. |
| Drag-select + `⌘C` (or `Ctrl+Shift+C`) | Copies the terminal selection. Only claimed when a selection exists, so bare `Ctrl+C` remains SIGINT. |
| Right-click → Copy | Native browser copy (xterm's own path). |
| Copy-on-select | ⋯ → *Copy on select* in a session pane (the terminal toolbar `copy` toggle elsewhere) — any new selection is copied immediately. Off by default, and stored per-origin in `localStorage`, so enabling it locally does **not** enable it on a remote origin. |
| `⌘V` / `Ctrl+V` | Native paste, handled by xterm from the `paste` event. |
| `Ctrl+Shift+V` | Programmatic paste via `navigator.clipboard.readText()` — the one clipboard call that needs a secure context *and* a permission grant; silently declines if refused (`⌘V` still works). |
| Paste an **image** | Uploaded to the daemon via `POST /snips`, then the stored PNG's absolute path is typed into the PTY as a bracketed paste. |

Clipboard **writes** everywhere in the UI go through `ui/src/lib/clipboard.ts`,
which prefers `navigator.clipboard` and falls back to a hidden-textarea
`execCommand('copy')`. That fallback is what keeps Copy buttons working on
origins the browser de-privileges — the self-signed `0.0.0.0` TLS listener, a
LAN reverse proxy, an older WKWebView — where `navigator.clipboard` is simply
absent. Note that reading `.writeText` off an absent `navigator.clipboard`
throws a *synchronous* `TypeError`, so the common
`navigator.clipboard.writeText(x).catch(…)` shape does not catch it; never
reintroduce it.

Image paste goes through the daemon on purpose: an agent CLI opens an image by
**file path**, and that path has to resolve on the machine the CLI runs on. When
Otto is driven from a browser on another machine, a browser-side path is
meaningless — so the bytes are uploaded first and the daemon's path is what
reaches the PTY.

### RTL & touch

The terminal supports right-to-left/bidi reflow and a touch-first phone mode
(soft-keyboard toggle, drag-to-scroll, zoom buttons, min 44×44 tap targets,
phone font floor). See **`./rtl-and-responsive.md`** for the full mobile/RTL
behavior.

---

## 4. Lifecycle: status, resume, idle-suspend, restart, close

### Status

`SessionStatus` is derived from PTY activity by a per-session status task that
ticks every **2 s** (`STATUS_TICK`):

| Status | Meaning |
|---|---|
| `running` | Child alive; no recent output classified yet (initial state on spawn/resume). |
| `working` | Output flowed within the last **5 s** (`WORKING_WINDOW`) — the agent is doing work. |
| `idle` | No output for ≥5 s. |
| `exited` | Child process exited. The terminal WS stays open so you can read the final output. |
| `reconnectable` | The PTY is gone (idle-suspend, or a daemon restart of a session that was not kept running) but the conversation can be resumed on demand — **0 RAM**. |

Status changes broadcast as `session_status` events on `/ws/events` (§9).

### Sessions survive daemon restarts

**Sessions you start yourself keep running when the daemon restarts** — a
deploy (`launchctl kickstart`), a crash, quitting and reopening Otto. The
terminal drops for a moment, reconnects on its own and shows the same process
with its screen and scrollback: a shell keeps its variables, its `cd`, the
command it was running; an agent CLI keeps everything it held in memory, mid
turn or not. Setting: **Settings → Daemon → Sessions → "Keep sessions running
when the daemon restarts"** (`session_persistence`, default **on**).

How it works (`crates/otto-pty/src/holder.rs`, `held.rs`):

- A session the user started (`is_user_started`: an Agents-page agent or
  shell — foreground, `work.origin` absent or `manual`) is spawned through a
  **PTY holder**: a small detached process (`ottod pty-holder`, the daemon
  binary re-executed) that owns ONE session's PTY master and child, keeps its
  own screen emulator (4000 rows of history, like the daemon's) and serves a
  single client over a unix socket. The holder runs in its own session
  (`setsid`), outside the daemon's process group, so launchd's job teardown
  and the PTY-master hangup that used to SIGHUP every shell and CLI on a
  restart no longer reach it. The launch request (command, environment,
  credentials) travels on the holder's stdin, never in `argv`.
- Sockets live in `<data dir>/pty-holders/` (a 0700 directory, 0600 sockets,
  random names; a per-user temp directory when that path is too long for a
  unix socket). The daemon keeps its usual local mirror (emulator, raw ring,
  broadcast), fed by the holder's output stream, so every terminal, chat,
  search and snapshot API behaves exactly as for an in-process PTY; input is
  acknowledged by the holder once it reached the tty (same ordering, queue and
  "not accepting input" semantics as before), resize and kill (the same
  HUP → TERM → KILL escalation) are forwarded to it.
- **On shutdown** the daemon *detaches* held sessions instead of killing them:
  their rows stay `running`/`idle`, their MCP credentials stay valid. Every
  other live PTY is still killed and marked exited.
- **On boot** (`restore_all` → `adopt_holders`), before anything else, the new
  daemon connects to every holder socket, checks the protocol version, reads
  which session it serves (holder metadata) and re-adopts it: the snapshot is
  replayed into a fresh emulator, the session is registered live again, its
  ingest token (baked into the child's environment) is accepted again, and it
  is marked `running` (trail: *Reattached after a daemon restart (process kept
  running)*). Clients re-attach to it like after any dropped socket and rebuild
  from the snapshot (new `epoch`). The boot credential sweep spares these
  sessions. A pending codex/agy id capture is re-armed. **A restart is not
  activity:** re-adoption writes the status without touching `last_active_at`,
  and the adopted handle's last-output clock is back-dated to the holder's
  `last_output_unix_ms` (protocol 1.1, `HolderInfo`), so frequent deploys no
  longer reset the idle-suspend clock of kept sessions.
- **Restart summary.** `restore_all` returns how many sessions were kept
  running and how many lost their process (now dormant); `hello_ack` carries it
  as `boot_restore` and the UI toasts *"Otto restarted — 5 sessions kept
  running · 2 suspended — they resume when you open them"* once per boot id.
- **Cleanup — nothing leaks.** Closing, archiving, deleting, suspending,
  restarting or killing a held session kills its child through the holder and
  releases it; the holder exits once the child is gone and removes its socket.
  On boot, a holder whose session no longer exists, is archived or is
  engine-owned, a duplicate holder for one session (the newest wins), and a
  holder speaking an incompatible protocol are all ended. A holder whose child
  exited while no daemon was attached keeps the exit code and final screen for
  10 minutes, then exits; the session then restores like any other. A holder
  no daemon has attached to for **24 hours**, or whose socket file vanished
  (its directory was wiped), ends its child and exits.
- **Version handshake.** Frames are `[kind u8][len u32][payload]`; the client
  says HELLO with its protocol version, the holder answers with its own
  (`HolderInfo`, JSON — unknown fields ignored). A new daemon adopts holders of
  the same protocol *major*, so a holder spawned by an older build survives an
  upgrade. HELLO, HELLO_ACK, KILL and RELEASE are frozen forever, so any daemon
  can always end any holder.

What is **not** kept: engine-owned sessions (workflow steps, reviews, channel
threads, swarm agents, …) — their driving engine dies with the daemon and
recovers its own way, so their processes still restart as before; connection
terminals (SSH/DB clients); the per-session network forwards of a held session
(they belong to the daemon — restart the session to reopen them); and anything
when the setting is off. If a holder cannot be started (binary missing, socket
path problem) the session spawns in-process and the daemon logs `pty holder
unavailable`.

> **launchd.** The holders leave the daemon's process group with `setsid`, so
> the job plist does not need `AbandonProcessGroup` (which would also stop
> launchd from cleaning up the daemon's genuinely-orphaned helpers after a
> crash). If a future macOS kills a job's whole coalition on `bootout`, held
> sessions would fall back to the old behaviour (reconnectable / fresh shell) —
> nothing worse.

### Resumability across daemon restarts

For every session that was **not** kept running (above), on daemon boot,
`SessionManager::restore_all` deliberately does **not** respawn
any agent processes (keeping every historical session resident would cost
~200 MB each). Instead every restorable session is marked `reconnectable` and
resumed **lazily** the moment a client opens it. The boot pass is one
set-based UPDATE (`SessionsRepo::mark_dormant_except`): rows that are already
`reconnectable` are not touched, `last_active_at` is never stamped (a restart is
not activity — migration `0148` repaired rows earlier boots had stamped, from
the agent trail), and only the changed rows are broadcast. Sessions that were
live get `meta.suspended = {reason: "restart", at}`. Then `ensure_live` sees a
non-live but resumable session and calls `restart`, which spawns the provider
with its **resume args**. Claude keeps the full conversation in its on-disk
JSONL transcript, so `--resume <provider_session_id>` restores it completely.

A session is **resumable** iff it is an `agent` kind, has a
`provider_session_id`, **and** its provider supports resume: `claude` (Otto
assigns the id at launch with `--session-id`), `codex` and `agy` (they mint
their own, which Otto captures from disk after spawn). A session whose id was
never captured is not resumed; reopening it starts a fresh process (no
lost-work risk is taken — see suspend below).

#### A terminal you ran an agent in

`shell` has no resume args of its own, so a plain terminal used to dead-end on
reopen: `ensure_live` had nothing to do, and the session looked like it no
longer existed even though nothing had been lost. Two things now happen instead
(`otto-sessions/src/nested.rs`):

- **A shell always respawns on open.** A login shell is cheap and stateless, so
  reopening one lands on a working prompt rather than a dead screen.
- **An agent you launched inside it is resumed.** Every ~30s
  (`capture_nested_agents`) the daemon looks through each live shell session's
  descendant processes for `claude` / `codex` / `agy`, asks the OS for that
  process's real cwd (you may have `cd`-ed first), and matches the transcript it
  created — for claude, the `<id>.jsonl` under `~/.claude/projects/<encoded
  cwd>/` *born* during that launch. The id is stored in the row's
  `provider_session_id` (so the session shows as **suspended / resumable**, like
  any other) plus `meta.nested_provider` / `nested_cwd` / `nested_pid`.
  Reopening then respawns the shell and types the provider's own resume command
  into it — `claude --resume <id>`, `codex resume <id>`, `agy --conversation
  <id>`, preceded by a `cd` when the agent was launched elsewhere.

Deliberate limits: the scan reads the live process tree, so it only ever sees an
agent that is **running right now** (exit the CLI and the last capture is what
gets resumed; start another and it re-captures, keyed on the pid). Ids another
session already owns are never re-claimed, and a launch window holding more than
one unclaimed candidate is skipped rather than guessed at — the same "never
guess" rule as the codex rollout capture, for the same reason (adopting someone
else's conversation forks it). The resume command carries no Otto flags: you
launched that CLI by hand, so you get your own permission mode back.

> **Transcript pruning (background sessions only).** For non-live `claude`
> sessions, the daemon checks whether the on-disk transcript
> (`~/.claude/projects/<encoded-cwd>/<sid>.jsonl`, with a directory-scan
> fallback) still exists; a row is pruned only when the transcript is
> *positively confirmed gone* — and only for **background/automation**
> sessions (channel tickets, review agents, workflow steps, …; the
> `BACKGROUND_SESSION_SOURCES` list in `otto-core`). **Foreground sessions —
> everything listed under the sidebar's Agents group — are never auto-pruned:
> they stay listed indefinitely (30+ days idle is fine) until you archive or
> delete them.** If the provider CLI has meanwhile cleaned its transcript
> (claude's `cleanupPeriodDays`), reopening such a session cannot restore the
> conversation — the row and its trail survive either way. Unknowable cases
> (other providers, no `$HOME`) are always kept (`lifecycle.rs`).

### Idle-suspend (save memory)

A background sweep (`suspend_idle_unattached`) frees the RAM of a live session
when **all** of these hold:

1. **Resumable** — agent kind + `provider_session_id` + provider supports resume
   (so codex/agy/shell are never auto-suspended; their work would be lost).
2. **Idle** — no PTY output for the full grace window. Default **5 minutes**
   (`SUSPEND_GRACE`), overridable via the `idle_suspend_grace_secs` setting.
3. **Unattached** — no WS viewer is currently watching (tracked by an
   `AttachGuard` reference count that decrements on every WS teardown path),
   no conversation view pinged within the last 3 minutes, **and no engine turn
   driver is still consuming the session** (a `TurnHold` taken by the workflow
   step / review / channel / assist drivers for the whole turn, dropped on
   every return path). An engine-owned session "loses nothing when reclaimed"
   only *after* its driver has returned: the turn oracle's bounded holds —
   handoff linger, background-task linger — keep a quiet session open for up
   to 15 minutes, three times this grace, and suspending it there made the
   engine re-run the whole step in a fresh session.
4. **Not pinned** — `meta.keep_alive` is not `true`.
5. **Engine-owned, or past the manual grace** — the session was started by a
   background origin: a `meta.work.origin` of `workflow` / `review` / `swarm` /
   `delegation` / `product` / `personal_agent` / …, **or** a background
   `meta.source` such as `channel` (the `BACKGROUND_SESSION_SOURCES` list).
   **Sessions you started yourself from the Agents page** (`work.origin` is
   `manual`, or the row pre-dates that stamp and carries no work ref at all)
   get a much longer grace instead: they are suspended only after **24
   hours** with no PTY output (`MANUAL_IDLE_SUSPEND`, setting
   `manual_idle_suspend_secs`, in **Settings → Daemon → Sessions**; `0` =
   never, the pre-2026-09-28 rule; it was 30 minutes until 2026-10-02, which
   suspended a session you had merely stepped away from) — and still only
   when every other guard here passes (no viewer, not pinned, no open turn,
   no descendant CPU). Plain shells are never suspended (they cannot be
   resumed losslessly).

   **Opening a session only to look at it does not pin its CLI.** Reopening a
   suspended session (a terminal attach or a chat view) resumes it with
   `--resume` so its history repaints. Until somebody actually types into it
   (or an engine sends it a turn) that process is a *passive resume*: it gets
   **no** manual grace, only the engine one above, so it is suspended again a
   few minutes after you leave. Emulator replies an attach produces on its own
   (cursor/device-attribute answers) do not count as typing. Before this, every
   session opened since the daemon started kept its agent CLI (150–400 MB of
   Node plus an MCP sidecar) alive for good.

   **Panes that only show a session never resume it.** Tiles in the tiled
   grid, the embedded agent-output viewers (code-review, docs, skill-review,
   skill-eval, product-analysis and workflow agents) and *every* automatic
   reconnect (a dropped socket, a daemon restart, the window regaining focus)
   attach view-only (`/ws/term/{id}?view=1`, docs/contracts/ws.md §1). A
   suspended session shows **Suspended — type or Resume to continue**; the
   first keystroke (it is not delivered — the CLI is still starting) or the
   Resume button brings the CLI back. Opening a session in an Agents pane, a
   loop or a swarm still resumes it as before.
6. **No open agent turn** — the provider's own on-disk record says the last
   turn finished. For `claude` the tail of the transcript JSONL must end in an
   assistant message with `stop_reason: "end_turn"` and carry no later
   `<task-notification>` line (a pending harness wake-up means the agent is
   about to speak again) — except one riding an `attachment` line, which claude
   staples onto your *next* prompt rather than waking the agent, and which is
   the normal tail of a finished session that used sub-agents. For `codex` the
   rollout's latest `task_started` must have its matching `task_complete` /
   `turn_aborted`. Only the last ~256 KiB is read, and an unreadable or
   unparseable file is "unknown" — never "suspend".

   **The open-turn hold is bounded by transcript freshness.** It only applies
   while the artifact's mtime is younger than `REAP_UNRESUMABLE_GRACE`
   (**30 minutes**). A turn that has looked "open" for longer is stuck, not
   live — every engine watcher gives up inside 3 minutes, and an agent really
   polling a background watcher appends a `tool_result` on every poll. Without
   the bound, one un-answered notification tail would pin an engine session's
   PTY, agent process and MCP sidecar forever, re-opening the exact fd leak
   that grace exists to close.

**Live-session cap.** After the idle pass the same sweep enforces a soft cap on
live agent CLIs (every agent provider except the plain `shell`; connection
terminals don't count): **12** by default, setting `max_live_agent_sessions`
(`0` = no cap). While more are live it suspends the **least-recently-used**
ones first — ordered by the later of their last PTY output and their last use
(typing, a terminal attach or detach, a chat ping). The cap overrides the
manual grace, but never takes a session that is watched (terminal viewer,
chat view, engine turn driver), pinned, `working`, printed anything in the
last 2 minutes (or the idle grace, if set lower), has a busy process tree, has
a fresh open turn, or cannot be resumed. If everything over the cap is in use
the cap stays exceeded until the next sweep and the daemon logs
`live-session cap exceeded`; it never kills work to get under it.

| Setting | Default | Meaning |
|---|---|---|
| `idle_suspend_grace_secs` | `300` | quiet time before an engine-owned (or passively resumed) session is suspended |
| `manual_idle_suspend_secs` | `86400` (24 h) | quiet time before a session you started is suspended; `0` = never |
| `session_persistence` | `true` | sessions you start keep running across daemon restarts (see *Sessions survive daemon restarts*) |
| `max_live_agent_sessions` | `12` | soft cap on live agent CLIs; `0` = no cap |

> **Why 5 and 6 exist.** "Idle" here means *no PTY output*, which is not the
> same as *done*. An agent that hands work to background watchers and
> `sleep`-polls them prints nothing, burns no descendant CPU and is squarely
> mid-turn; the sweep used to suspend exactly that, yanking the PTY out from
> under a live interactive session three times in one afternoon. Every guard
> that holds a session names itself in the daemon log — `engine turn`,
> `keep_alive`, `origin=manual`, `turn open`, `descendant CPU` — at `info` the first time it
> applies and on every later change of guard, `debug` while it simply persists
> (a held session is re-held every 60 s, for as long as it lives).

One older guard rides alongside those: the sweep compares each candidate's
**descendant-process CPU** against the previous pass and skips any session
whose tree accrued >200 ms (a quiet build or test run). Descendants only — the
agent CLI's own idle TUI redraws accrue CPU forever.

On suspend the daemon kills and drops the live PTY (freeing memory), **keeps the
row** with its `provider_session_id` intact, sets status `reconnectable`, and
records a lifecycle trail entry. Reopening auto-resumes (above). Every suspend
stamps **why and when** as `meta.suspended = {reason, at}` — `idle` (the sweep),
`cap` (the live-session cap), `restart` (the boot restore), `released` (an
explicit suspend by an engine/API) — and a resume clears it. A suspended pane
shows it in its details and the *Suspended* note's tooltip: *"Suspended 3h ago
· idle too long"*. The session pane shows the idle time for an idle, non-pinned
agent (*"4h idle"*) and adds the countdown (*"· suspends in 30m"*) only when the
suspend is under **2 hours** away — further out it is noise, and false for any
pane you are watching (a watched session is never suspended). The label updates
once a minute (once a second only during the first minute).

**Pin to keep alive:** the ⋯ menu offers *"Pin (keep alive)"* / *"Unpin (allow
auto-suspend)"* (agent sessions only), toggling `meta.keep_alive`. A pinned
session is never auto-suspended, by the idle sweep or by the live-session cap.

**Opt-in auto-archive:** set the `session_auto_archive_days` setting to N > 0
and an hourly sweep archives any non-archived agent session whose
`last_active_at` is older than N days — never a live, attached, or
`keep_alive`-pinned one. Archive keeps the row + history and is reversible via
unarchive; the default (absent or `0`) is **off**.

**Non-resumable background sessions are reaped, not leaked.** A live agent
session that can *never* be suspended (no captured provider id — e.g. a codex
review agent whose rollout pick was ambiguous across a same-cwd fan-out) used
to hold its PTY (~3 fds), agent process and MCP sidecar forever; review fleets
accumulated hundreds of descriptors and pushed the daemon over launchd's
256-fd soft cap ("Too many open files", failing `accept()`, seconds-long
keystrokes). The same sweep now **kills** such a session — same unattached /
CPU-quiet / not-pinned / no-open-turn guards — once it has been idle for
`REAP_UNRESUMABLE_GRACE` (**30 minutes**, far beyond every engine's stall
window), but **only engine-owned (background) sessions**: the owning engine
already consumed the turn output, so nothing is lost. The user's own
(foreground) sessions and connection terminals are never touched.

### Restart

`POST /api/v1/sessions/{id}/restart` (or the pane's ⋯ → *Restart session*) respawns the session: it kills any live PTY, rebuilds the
spec, **uses the resume args when `provider_session_id` is set** (so a claude
restart resumes the same conversation; others start fresh), re-applies
`--add-dir`/`--model` from `meta`, re-trusts the folder, re-wires the ingest env,
restores the saved grid, and records a *"Session resumed"* trail entry. Returns
the updated `Session`.

### Close, archive, delete

- **Close pane** (the pane `×`, tooltip *"Close pane (keeps running)"*) only
  detaches the UI; the session keeps running on the daemon.
- **Close tab** — closing a tab whose session is still LIVE (the `live` flag on
  the session payload, status heuristic as fallback) asks what you mean:
  *Close tab* (UI only, session keeps running), *Archive session* (stop it,
  keep history), or Cancel — with a "remember my choice" checkbox (reset under
  **Settings → Appearance**). A dead/suspended session's tab closes without
  asking. Bulk closes (Close Others / Close to the Right / Close All in the
  tab's context menu) show ONE dialog for the whole set. Closed tabs are
  reopenable with **⌘⇧T** (close never deletes anything).
- **Archive** — `POST /sessions/{id}/archive` kills any live PTY and keeps the
  row + history, hidden in the "Archived" section; `…/unarchive` restores it as
  `reconnectable`. Both return the updated `Session`. Channel-spawned sessions
  auto-archive after long idleness. An archived session can NOT be resumed or
  restarted (409) until unarchived — attaching to its terminal no longer
  silently revives it.
- **Kill (keep listed)** — `POST /sessions/{id}/kill` stops the process but
  keeps the row un-archived in the list; resumable providers reopen on demand.
- **Bulk** — `POST /sessions/bulk` `{action: archive|delete|kill, ids}` applies
  one action across many sessions (per-id owner check; failures reported
  per-id, the batch continues).
- **Delete** — `DELETE /api/v1/sessions/{id}` kills the PTY and removes the row
  **and its history**. The UI marks this action as danger and always confirms.
- **Retention** — sessions in the **Agents** group are durable: no background
  sweep archives or deletes them, ever. Archive and delete (above) are the
  only ways they leave the list. Channel-spawned (ticket/chat) sessions keep
  their own lifecycle: auto-archive after 1 h idle, purge 30 days after
  archival.
- **Quit hook** — `POST /api/v1/app/kill-sessions` terminates every live PTY
  (the desktop app's quit hook).

**How the sidebar loads (performance).** The Agents sidebar asks the daemon
only for the rows it shows — `?archived=false&foreground=true&with_sources=channel`
(live connections, foreground agents, Slack/Telegram tickets). Background
engine sessions (review agents, workflow steps, assists, …) are never bulk
downloaded: open tabs and panes are fetched by id (`GET /sessions?ids=…`) in
the same round trip, a background session opened from its panel (or a
notification) is fetched by id on demand, and the swarm views add `swarm` to
`with_sources` while mounted. The Archived section loads lazily, 100 rows at a
time, the first time it is expanded ("Load more" pages further back); the
header shows whenever a 1-row probe finds any archived session. The per-session
tokens rollup is polled every minute only while the **Tokens** sort is active.

---

## 5. Workspace auto-trust & the prompt-guard

Otto workspaces are folders the user explicitly chose, so agents should never
stall on an interactive "do you trust this folder?" dialog. Two layers ensure
that, both best-effort and never fatal:

1. **Pre-trust (deterministic, `trust.rs`).** Before every agent spawn the
   daemon writes the provider's own trust config:
   - **claude** → `~/.claude.json` → `projects.<path>.hasTrustDialogAccepted =
     true` (also `hasCompletedProjectOnboarding`), written for **every path
     variant** (literal, symlink-resolved, and the `/private` prefix macOS adds
     for `/var` and `/tmp`) so a resolved-path comparison can't re-trigger the
     dialog. Written atomically (temp file + rename).
   - **codex** → `~/.codex/config.toml` → `[projects."<path>"] trust_level =
     "trusted"` (every path variant, same as claude).
   - **grok** → `~/.grok/trusted_folders.toml` → folder-trust grant (the same
     store `/hooks-trust` / `--trust` write). Needed because Otto often
     materializes `.mcp.json` in the cwd, which triggers Grok's "Trust the
     authors of this folder…?" gate. Over-broad roots (home, `/`) are skipped.
   - Other unknown providers are left alone here.

2. **Prompt-guard (runtime backstop, `prompt_guard.rs`).** An `OutputScanner`
   watches each session's PTY output; when a **known approval prompt** for the
   provider appears in the recent tail it writes the accepting keystroke back.
   Detection is intentionally narrow — specific full phrases (e.g. *"do you trust
   the files in this folder"*, *"allow codex to work in this folder"*, *"trust
   the authors of this folder"*, *"press enter to continue"*) so it never injects
   keys into the agent's real work on a false positive — and it is debounced per
   session (≤ once / 5 s). claude is accepted with `1\r` (select "Yes");
   codex/agy with `\r`; **custom providers (incl. grok)** with `y\r` plus the
   shared continue phrases. `shell` is never auto-approved. This catches what
   pre-trust can't: providers without a known trust config and unexpected
   first-run dialogs. Anything it does *not* match is caught by the analysis
   stuck-detector (idle → retry → notify), so no prompt hangs forever. Each
   auto-approval is recorded on the session's activity trail.

   **Recommended Grok custom-provider args** (Settings → Providers):
   `args: --session-id {sid} --always-approve`, `resume_args: --resume {sid}`.
   Optionally pin in `~/.grok/config.toml`: `[ui] permission_mode = "always-approve"`
   and `[hints] project_picker_disabled = true` so sessions started from home /
   Desktop / `/tmp` don't open the "choose a project folder" picker.

**Trust follows the session cwd, not the workspace.** Both layers key on the
SESSION's `cwd` (`trust::ensure_trusted(&session.provider, &session.cwd)`), as
does the process sandbox (`SandboxPolicy::for_agent(&cwd, …)`, whose first
writable root is the cwd); a workspace's `root_path` is only the *default* a
session's cwd falls back to when the request omits one — nothing passes it to
trust or the sandbox. So a session started in a project folder is confined to
that folder whether or not it belongs to a workspace, and the scratch
workspace's `root_path = $HOME` is inert by itself. The consequence for a
**workspace-less session started in the home folder** (the scratch default):
the sandbox's writable root is the whole home directory and the provider trust
grant is written for `$HOME` — a grant Claude Code honours for that folder and,
via its ancestor lookup, for every folder under it (codex likewise; grok
refuses over-broad roots, so no grok grant is written and its prompt-guard
approval covers the session instead). The New Session sheet says so under the
folder field whenever the cwd is `~`; pick a narrower folder there to get the
usual confinement.

---

## 6. Activity trail & task tracker (live agent telemetry)

Every session has an append-only **activity trail** and a normalized **task
tracker**, surfaced in `ActivityPanel.svelte`.

- **`TrailEvent`** — `{id, session_id, workspace_id, ts, source, kind, level,
  summary, detail?}`.
  - `source` (`TrailSource`): `user` (a human note / injected command), `agent`
    (a tool/skill/reply from the CLI), `otto` (lifecycle: spawned, resumed,
    suspended, archived).
  - `kind` (`TrailKind`): `session`, `prompt`, `skill`, `command`, `tool`,
    `file`, `web`, `task`, `note`, `other` — drives the row icon.
  - `level` (`TrailLevel`): `info` | `warn` | `error`.
- **`AgentTask`** — `{id, session_id, workspace_id, ext_id?, title, status,
  position, …}`. `status` (`TaskStatus`): `pending`, `in_progress`, `completed`,
  `blocked`, `cancelled` (the union over Claude's TodoWrite states plus
  blocked/cancelled).

**What writes them.** Otto auto-records lifecycle entries (session started /
resumed / suspended / archived), submitted user messages, and prompt-guard
approvals. The agent's **own activity** (tool calls, skill loads, commands,
file edits, task-list changes) is written by the provider's injected hooks,
which `POST` to the per-session **ingest** endpoints:

- `POST /api/v1/ingest/claude` and `POST /api/v1/ingest/codex` — provider
  activity hooks.
- These routes are **unauthenticated by bearer** but gated by the per-session
  **ingest token**: at spawn the daemon sets `OTTO_INGEST_BASE`,
  `OTTO_SESSION_ID`, and `OTTO_INGEST_TOKEN` (a per-session UUID) in the agent's
  environment; the hook config presents that token. The token is verified by
  `verify_ingest_token` and revoked when the session is removed.

**Reading them (bearer-authed UI/API):**

| Method & path | Auth | Result |
|---|---|---|
| `GET /workspaces/{wid}/sessions/{sid}/trail` | ws viewer + session owner/admin/root | `TrailEvent[]` (newest 500, oldest→newest) |
| `POST /workspaces/{wid}/sessions/{sid}/trail` | ws editor + owner/admin | append one entry (UI "notes" → `source=user, kind=note`) |
| `GET /workspaces/{wid}/sessions/{sid}/tasks` | ws viewer + owner/admin/root | `AgentTask[]` |
| `PUT /workspaces/{wid}/sessions/{sid}/tasks` | ws editor + owner/admin | replace the task list (manual override) |
| `GET /workspaces/{wid}/activity/summary` | ws viewer | per-session roll-up (`SessionActivitySummary[]`) — admins see all users' sessions, non-admins only their own |

Writes mirror to `/ws/events` as `trail_appended` / `tasks_updated`. The panel
shows a task progress bar (`done/total`), source-filter tabs (All / Agent / You
/ Otto), per-kind icons, expandable JSON detail, and a *"Add a note to this
session…"* input. The session pane shows a compact `done/total` task chip and a
*"now: <in-progress task>"* hint. A session waiting on you (input or permission)
shows a **"Needs you"** amber badge.

**Finding out while you are elsewhere.** In the desktop app the **dock badge**
counts the current workspace's sessions that need you (`ws.needsYouCount`,
debounced 250 ms, written through the shell's `set_badge_count`; cleared on
sign-out). The info *"Session awaiting input"* notice (`…:waiting` — every
provider, not only Claude's permission hook) also raises a **native macOS
banner** when you are not watching that session (window hidden or unfocused,
or another session active). Toggle: Settings → Notifications → *Banner when a
session is waiting on you* (`native_on_waiting`, default on; needs native
notifications on).

### The right panel's other tabs

Activity is one tab of the desktop right panel (Git / Files / Notes / Activity /
**Canvas** / Info / Browser / API — a drawer on tablet/phone), shown only for
`agent`-kind sessions. **Canvas** (`CanvasPanel.svelte`) lists the Canvas scenes
*referenced* by the focused session — attach an existing scene or create a new
one, expand a row for an inline SVG preview, jump to the full editor ("Open in
Canvas"), or detach it. Live updates arrive over `canvas_refs_changed` when a
scene is attached/detached from anywhere (this panel, the Canvas module, or an
MCP write tool). See **[Canvas §7a](./canvas.md#7a-session-references)** for the
full behavior and the `GET/POST/DELETE /sessions/{id}/canvas-refs` endpoints.

---

## 7. Driving a session programmatically

You can drive sessions over HTTP from a script or another agent. See
**`./daemon-http-api.md`** for tokens, base URL, and the WS auth handshake;
the session-specific surface:

```bash
# Create an agent session in a workspace
curl -sS -X POST "$BASE/api/v1/workspaces/$WS/sessions" \
  -H "Authorization: Bearer $OTTO_API_TOKEN" -H 'content-type: application/json' \
  -d '{"kind":"agent","provider":"claude","cwd":"/path/to/repo",
       "meta":{"extra_dirs":["/path/to/other-repo"]}}'

# Send a prompt (submit:true → append a newline so the agent runs it now)
curl -sS -X POST "$BASE/api/v1/sessions/$SID/input" \
  -H "Authorization: Bearer $OTTO_API_TOKEN" -H 'content-type: application/json' \
  -d '{"text":"run the tests and summarize failures","submit":true}'

# Read the trail / tasks
curl -sS "$BASE/api/v1/workspaces/$WS/sessions/$SID/trail" -H "Authorization: Bearer $OTTO_API_TOKEN"
```

`POST /sessions/{id}/input` (`SendInputReq{text, submit?}`) writes into the PTY:
`submit` omitted/`true` appends a `\n` so the agent executes immediately;
`submit:false` sends the text verbatim so a human can inspect/edit before
pressing Enter. To **watch** the live terminal, open
`WS /ws/term/{session_id}` with `Sec-WebSocket-Protocol: otto-bearer, <token>`.
The `otto-sessions` operating skill wraps these calls.

---

## 8. API & contract reference

REST (under `/api/v1`, bearer auth, JSON snake_case, ULID ids). Item routes
resolve the owning workspace from the row and role-check against it.

| Method & path | Auth | Notes |
|---|---|---|
| `GET /meta` | public | `MetaResp` — `providers`, `default_provider`, `tools` |
| `GET /workspaces/{id}/sessions` | ws viewer (`Agents:View`) | `Session[]` (you see your own; ws-admin/root see all); optional `?archived=&kind=&source=&status=&limit=&before=&foreground=&with_sources=&ids=` filters (`foreground=true` = what the sidebar lists: connections + foreground agents + any `with_sources`; `ids` ≤ 64); rows carry transient `live` + `viewers` |
| `POST /workspaces/{id}/sessions` | ws editor (`Agents:Edit`) | `CreateSessionReq` → `Session` |
| `GET /workspaces/scratch` | `Agents:View` | the hidden scratch `Workspace` (`id: "scratch"`, `root_path` = daemon `$HOME`); every user is an implicit Editor there, so `…/scratch/sessions` starts / lists workspace-less sessions (§2); `PATCH`/`DELETE` + member edits → 409 |
| `GET /sessions/{id}` | owner-or-admin | `Session` (with transient `live`, `viewers`) |
| `PATCH /sessions/{id}` | owner-or-admin | `UpdateSessionReq{title?, meta?}` → `Session` |
| `DELETE /sessions/{id}` | owner-or-admin | 204 — kills PTY, removes row |
| `POST /sessions/{id}/restart` | owner-or-admin | respawn (resume when `provider_session_id` set) → `Session`; 409 when archived |
| `POST /sessions/{id}/input` | ws editor + owner-or-admin | `SendInputReq{text, submit?}` → 200 |
| `POST /sessions/{id}/archive` / `…/unarchive` | owner-or-admin | → `Session` |
| `POST /sessions/{id}/kill` | owner-or-admin | stop the PTY, keep the row un-archived → `Session` |
| `POST /sessions/bulk` | per-id owner-or-admin | `{action: archive\|delete\|kill, ids}` → per-id results |
| `POST /sessions/{id}/handover` / `…/handover/brief` | ws editor + owner-or-admin | start a handover / generate its brief (see §10) |
| `POST /sessions/{session_id}/attach-product` | ws editor | `{story_id}` — attach a product story |
| `POST /workspaces/{id}/broadcast` | ws editor | `BroadcastReq{text, session_ids?}` → `BroadcastResp{session_ids}` |
| `POST /app/kill-sessions` | root only | terminate every live PTY |
| `GET/POST /workspaces/{wid}/sessions/{sid}/trail` | viewer / editor (+owner) | activity trail (§6) |
| `GET/PUT /workspaces/{wid}/sessions/{sid}/tasks` | viewer / editor (+owner) | task tracker (§6) |
| `GET /workspaces/{wid}/activity/summary` | ws viewer | per-session roll-up |
| `POST /ingest/claude`, `POST /ingest/codex` | per-session ingest token | provider activity hooks (§6) |

**WebSockets:**
- `WS /ws/term/{session_id}` — the terminal stream (frames in §3).
- `WS /ws/events` — the event stream (§9).

**Key DTOs** (`crates/otto-core/src/api.rs`, `…/domain.rs`):

```rust
CreateSessionReq { kind: SessionKind, provider: Option<String>, title: Option<String>,
                   cwd: Option<String>, connection_id: Option<Id>, meta: Option<Value> }
UpdateSessionReq { title: Option<String>, meta: Option<Value> }
SendInputReq     { text: String, submit: Option<bool> }   // None/true ⇒ append "\n"
Session          { id, workspace_id, kind, provider, title, status, cwd,
                   provider_session_id, connection_id, created_by, created_at,
                   last_active_at, archived, meta }
```

Recognized `meta` keys: `extra_dirs:[string]`, `model:string`, `browser:bool`,
`keep_alive:bool`, `pty_cols`/`pty_rows`, `client_id` (per-device, §11),
`source` (e.g. `"review"`), `handover_from`.

### Event catalog (session-family)

On `/ws/events`, session-family events reach only the session's **owner**, a
workspace **admin**, or **root** (after the `viewer`+ gate on the workspace):

```json
{"type":"session_status","session_id":"…","workspace_id":"…","status":{…SessionStatus…}}
{"type":"session_created","session":{…Session…}}
{"type":"session_meta_updated","session_id":"…","workspace_id":"…","meta":{…}}
{"type":"session_removed","session_id":"…","workspace_id":"…"}
{"type":"trail_appended","workspace_id":"…","session_id":"…","event":{…TrailEvent…}}
{"type":"tasks_updated","workspace_id":"…","session_id":"…","tasks":[{…AgentTask…}]}
```

`session_meta_updated` carries the full merged `meta` so the UI updates a cached
session in place (e.g. live handover-progress flags).

---

## 9. Capabilities & limitations

**Can:**
- Run claude / codex / agy / shell as real PTYs you can watch, type into, split
  (up to 15 panes, free-form nested layouts), and tile; add custom providers via
  settings without a rebuild.
- Survive daemon restarts and idle-suspend without losing a claude conversation.
- Multi-attach: several clients watch the same session, output broadcast to all.
- Persist 10,000 lines of scrollback across reconnects, grep-searchable
  server-side.
- Auto-trust the session folder and auto-clear stray approval prompts.
- Start a session without any workspace (the hidden scratch workspace, default
  cwd `~`) — sidebar "No workspace" group, archive / resume / hand over as usual.
- Stream a live activity trail + task tracker, and drive everything over HTTP/WS.

**Limitations / by design:**
- **Resume needs a captured conversation id.** claude gets one at launch;
  codex/agy are captured from disk after spawn, and a session whose capture came
  up empty (or ambiguous) starts fresh instead of adopting a conversation that
  might belong to someone else. Such a session is also never *auto*-suspended,
  since its work could not be brought back.
- **A `shell` is respawned, not resumed.** Reopening one always gives a fresh
  login shell — its scrollback, env and background jobs are gone. What survives
  is an agent CLI you ran inside it: that conversation is captured while it runs
  and resumed by typing the provider's resume command into the new shell.
- Restart-resume relies on the on-disk claude JSONL transcript; if that
  transcript is gone, resume can't reconstruct the conversation.
- The daemon listens on **loopback only** unless a network listener is
  explicitly enabled.
- Bracketed-paste submission assumes the target TUI honors bracketed paste
  (Claude Code, Codex do); plain shells just receive the bytes.
- The prompt-guard matches a **narrow** phrase set; novel approval wording is
  handled by the stuck-detector (retry/notify), not silently accepted.
- The PTY ring buffer caps history at 10,000 lines / 2 MiB per session; older
  output ages out.
- **Handover is same-workspace only** — a workspace-less (scratch) session can
  hand over to a new agent or to another scratch session, never into a
  workspace session (and vice versa).

---

## 10. Handover (move context between agents)

A **handover** pushes the working context of one agent into another (e.g.
Claude → Codex) so you don't re-explain a task by hand. `POST
/sessions/{id}/handover` (Editor on the source's workspace) spawns the target in
the **same workspace + cwd** (a scratch session's target is a scratch session —
the daemon rejects an existing target from another workspace, and the picker
only offers same-workspace agents), returns it immediately, then in the background
gathers the source's recent work (claude transcript digest, else PTY
scrollback), summarizes it into a structured brief, and injects it into the
target as one bracketed-paste block. The request shape carries the target
(`{kind:"new_agent",provider}` or `{kind:"existing_session",session_id}`) plus
optional `brief?`, `include_git?`, `fast?`, `archive_source?`. Progress shows as
an in-pane *"⏳ handover…"* badge and a source→target breadcrumb, driven by live
`session_meta_updated` events. Summarizer failure degrades to the raw digest;
the handover never fails just because summarization did. Launch it from the
pane's *"Hand over to…"* menu item (`Handover.svelte`).

---

## 11. Security & permissions

- **RBAC (`Agents` feature).** `policy.rs` maps every session route to an
  `Agents` capability: list/inspect = `Agents:View`; create / restart / archive
  / input / handover / broadcast / orchestrate = `Agents:Edit`. There is no
  Admin tier for Agents. Activity trail/tasks reads = `Agents:View`, writes =
  `Agents:Edit`. The feature gate is default-deny — no grant means a `403` and
  the feature is hidden in the nav. See `./multi-user-rbac.md` (`docs/MULTI-USER-RBAC.md`).
- **Per-session ownership / isolation.** A user sees, attaches to, and controls
  only **their own** sessions (`created_by`); workspace-admins and root see all.
  The terminal WS (`/ws/term`) enforces the same owner-or-admin gate before
  upgrade — a non-owner viewer/editor gets `403`. The activity summary restricts
  non-admins to their own sessions. The scratch workspace grants every
  authenticated user **Editor** (implicitly, never Admin), so its sessions are
  owner-scoped like any other: you list, attach to and control only the
  workspace-less sessions you created; root sees all. **Only the session routes
  exist under `scratch`** — `/workspaces/scratch/sessions…`, plus
  `…/broadcast` and `…/activity/summary`. Every other `/workspaces/scratch/…`
  family (api-client, workflows, mcp-servers, connections, vault, …) answers
  **404**, so that implicit Editor is a session grant and nothing more.
- **Viewer = read-only terminal.** A workspace viewer may attach and watch but
  cannot send input/resize (frames dropped server-side). Editor+ may drive it.
- **Share-link throttle.** WS token validation is rate-limited per IP: **10**
  failures within a **15-minute** window locks that IP out for **15 minutes**
  (HTTP 429 + `retry-after`). A successful auth clears the IP's tally.
- **Force-terminate.** An admin terminate or a revoked mobile share-link evicts
  every attached viewer with a `{"type":"terminated"}` frame and an immediate
  socket close.
- **Folder trust** is granted only for the session's cwd (the workspace folder
  by default; `~` for a workspace-less session started in the home folder —
  which then covers everything under it, see §5) and its path variants; the
  prompt-guard accepts only a narrow phrase set (§5).
- **Secrets.** Connection-session secrets live in the macOS Keychain, never in
  the session row. Ingest tokens are per-session and revoked on removal.

### Per-device session view (opt-in)

By default you see every session you created, regardless of which device started
it. **Settings → Appearance → "Sessions on this device" → "Isolate sessions to
this device"** flips this on: *"Only show sessions started on this device. Other
devices' sessions stay hidden here (they still run on the daemon)."* This is a
**client-side** filter:

- Each browser/device gets a stable `client_id` (UUID in localStorage key
  `otto_client_id`). When you create a session the UI stamps it into
  `meta.client_id`.
- The toggle is the localStorage setting `otto_session_isolation` (default off,
  `ui.sessionIsolation`). When on, the workspace store filters the session list
  to rows whose `meta.client_id` matches this device. It hides nothing on the
  daemon — the sessions keep running and are visible again when you turn it off
  or open another device.

---

## 12. Troubleshooting

- **A session is stuck on a trust/approval prompt.** Pre-trust + the prompt-guard
  should clear known dialogs automatically. If a *novel* prompt blocks it, attach
  the terminal and accept it manually; consider filing the exact phrase so it can
  be added to the prompt-guard table.
- **Agent "woke up" / RAM spiked when I opened the tiled view.** Expected only
  up to 6 tiles go live at once; the rest are placeholders. If you pinned many
  tiles, each pinned tile stays live. Unpin or scroll them out of view.
- **Reopening a session lost its context.** Check the row for a
  `provider_session_id` (`GET /sessions/{id}`): without one there was nothing to
  resume — the spawn-time capture found no unambiguous match (`grep "won't
  auto-resume" ~/Library/Logs/Otto/ottod.log*`). Pin (`keep_alive`) a session
  you must keep warm; note that pinning only prevents *auto*-suspend, a daemon
  restart still starts an uncaptured session fresh.
- **A terminal came back as a bare shell after I had claude running in it.**
  The nested capture only sees an agent that is running at scan time (every
  ~30s), and claude files its transcript when the first prompt is sent — a
  daemon restart in that gap leaves nothing to resume. `grep "captured a nested
  agent" ~/Library/Logs/Otto/ottod.log*` shows what was captured and when. The
  shell itself still respawns, so the terminal is usable either way.
- **My session disappeared from the list.** Check whether **"Isolate sessions to
  this device"** is on (you may be on a different device than the one that
  created it), or whether it was archived (look in the Archived section) or
  deleted.
- **Scrollback vanished after reconnect.** It shouldn't — the ring buffer
  survives reconnects and the client requests 2000 lines on attach. If you see
  only the visible screen, the session may have been restarted (new PTY = empty
  ring) or the client sent `lines:0` and got the 1000-line default.
- **"forbidden: viewers cannot send input."** Your workspace role is Viewer (or
  your share-link is view-only). You need Editor to type.
- **429 / locked out of the terminal WS.** Too many failed token attempts from
  your IP; wait out the 15-minute lockout.
- **Status shows `reconnectable` and the terminal says "reconnecting…".** The
  PTY was suspended (idle) or the daemon restarted without keeping it;
  opening/focusing the session resumes it. The terminal overlay offers a
  **Now** / **Reconnect** / **Resume** button. A pane whose socket dropped
  right after an exit (the daemon going away) re-attaches once on its own and
  shows the session's real state — live again, or *Suspended — type or Resume
  to continue* (a shell too: typing or Resume respawns it).
- **A session did not survive a daemon restart.** Check that it is one you
  started (Agents page — workflow/review/channel sessions always restart) and
  that *Keep sessions running when the daemon restarts* is on. Then
  `grep -E "pty holder|re-adopted|left .* running" ~/Library/Logs/Otto/ottod.log*`:
  `pty holder unavailable` means it was spawned in-process; `re-adopted live
  session` is the success line; `not re-adoptable` / `speaks protocol` explain
  a holder that was ended on boot. Live holders are `ottod pty-holder`
  processes (`pgrep -fl pty-holder`), one per kept session.
- **Custom provider not appearing.** Confirm it's in the `providers` settings
  JSON and that `cmd` is on `PATH` (`GET /meta.tools` reports detected tools).
- **A TUI pane is garbled after switching tabs** (blank rows, fragments on the
  wrong rows, stray characters at the right edge). Use **⋯ → Redraw terminal**
  (⌘K "Redraw terminal"). To see which layout step produced the stray grid,
  turn on the latency flag below: every local xterm resize is logged to the
  webview console as `local resize A → B (reason, pty C)`.
- **Typing or scrolling lags but the CPU is idle.** Turn on the latency HUD:
  run `localStorage.setItem('otto.debug.termLatency','1')` in the webview
  console (Develop menu) and reopen the pane. A corner overlay shows `rtt`
  (websocket + daemon loop), `echo` (the CLI answering a keystroke, measured in
  the daemon), `wire`/`parse`/`paint` (the keystroke's trip through the
  webview), timer `drift`, the `rAF` interval, page visibility and the
  renderer. High `echo` means the CLI itself: a long claude session re-renders
  its whole UI on every key, so `/compact` or a new session helps. Low `rtt`
  with high `parse`/`paint`/`drift` means the webview is throttled or busy.
  High `rtt` means the daemon. `render: dom` on a pane that should use the GPU
  recovers on window focus or when the pane is focused. Remove the key to turn
  it off.

---

## 13. Related docs

- **`./agent-swarm.md`** — teams of role-specialized agents; each swarm agent is
  an agent session.
- **`./channels-slack-telegram.md`** — Slack/Telegram bridges that reply through
  agent sessions.
- **`./code-review.md`** — multi-agent PR review; reviewer agents run as
  sessions (with `meta.source="review"`).
- **`./connections.md`** — connection (SSH/DB) terminals that share this PTY and
  terminal-WS machinery.
- **`./canvas.md`** — the file-backed diagram tool; §7a covers the right panel's
  Canvas tab (session-referenced scenes) documented above.
- **`./daemon-http-api.md`** — tokens, base URL, and the WS auth handshake for
  driving Otto over HTTP.
- **`./rtl-and-responsive.md`** — RTL/bidi and touch/mobile terminal behavior.
- **`docs/MULTI-USER-RBAC.md`** — the full RBAC, ownership, and isolation model.
- **`docs/contracts/api.md`** / **`docs/contracts/ws.md`** — the authoritative
  REST and WebSocket contracts.

## September 2026 reliability and recovery updates

Chat drafts and uploaded attachments belong to their session when switching panes.
Workspace selection and History pagination reject obsolete responses, including a
second refresh that finishes while the first selection is still restoring saved tabs.
Session archive, removal, kill, suspend, unarchive, and restart share a lifecycle lock,
so an archived session cannot finish a pending restart and continue invisibly.

History's scope selector includes **No workspace**, and broadcasting works across
scratch sessions when all recipients belong to that same scope. Mixed-workspace panes
show why broadcasting is unavailable. New Session, onboarding and Handover show shared
provider readiness from the configured executable, including custom providers.

Mission saved views can be authored with status, provider and repository selectors;
advanced JSON remains available and its filters combine with the regular cost filter.

Handover accepts reasoning agents only. On the target, expand **Handover** to inspect
the saved brief and delivery state. A failed/interrupted delivery can be retried after
inspecting the target. **Sent** means the PTY accepted paste and Enter; **Confirm
received** records the operator's receipt confirmation. If source archival was selected,
it happens only after that confirmation. Failed or interrupted deliveries retain the
brief and source session for recovery.

### Pending messages and context-dependent providers

A pending chat send belongs to its session across view changes. Reopening the
composer keeps Send disabled until that request settles, while allowing a new
draft to be edited. Completion clears only the submitted draft and attachments.
Provider executable templates containing `{cwd}` or `{sid}` are resolved at
launch, when those values exist; global readiness shows them as unchecked and
keeps them selectable. Mission Control ignores stale workspace loads and save
completions, so switching workspaces preserves the destination's view and form.
