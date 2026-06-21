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
| Create / input DTOs | `crates/otto-core/src/api.rs` |
| New-session modal | `ui/src/modules/agents/NewSession.svelte` |
| Session pane (header, ⋯ menu, status, idle countdown) | `ui/src/modules/agents/SessionView.svelte` |
| Split panes + broadcast bar | `ui/src/modules/agents/Splits.svelte` |
| Tiled / grid view (live-tile budget) | `ui/src/modules/agents/TiledView.svelte` |
| Terminal component (xterm + WS) | `ui/src/lib/components/Terminal.svelte` |
| Activity trail + task tracker panel | `ui/src/modules/panels/ActivityPanel.svelte` |
| Per-device isolation toggle | `ui/src/modules/settings/Appearance.svelte`, `ui/src/lib/stores/ui.svelte.ts` |

---

## 2. Creating & opening a session

### From the UI

Open **New Session** (the agents page **New Session** button, or `⌘T`). The
modal (`NewSession.svelte`) offers:

- **Provider** — cards for `claude` ("Claude Code CLI"), `codex` ("Codex CLI"),
  `shell` ("Plain shell"), plus any custom providers from `GET /meta.providers`.
  The configured default provider is pre-selected and carries a **default**
  badge.
- **Title (optional)** — placeholder shows the auto-generated name. If left
  blank the daemon names the session `"{provider} #{n}"`, where `n` is one more
  than the current count of that provider in the workspace.
- **Working directory** — *"Defaults to the workspace root"* if blank. The
  daemon `mkdir -p`s the directory if it does not exist (a missing cwd would
  otherwise make the child fall back to `$HOME`).
- **Additional directories (optional)** — extra repos the agent may access,
  *"passed as `--add-dir`"*. Stored in `meta.extra_dirs` (an array). Honored by
  `claude` / `codex` / `agy`; ignored for `shell`.
- **Browser tools** — shown only for `claude` / `codex`: *"Give the agent a real
  browser via MCP (navigate, click, read pages)."* Stored as `meta.browser:
  true`; wires an MCP browser server into the workspace `.mcp.json`.
- **Preview context** — for `claude` / `codex`, expands to show exactly what
  Otto would inject (skills/soul/context) before spawning.

Press **Start Session**. A session can also pin a model: when `meta.model` is
set, the daemon appends `--model <name>` for `claude` / `codex` (silently
omitted for `agy` / `shell`).

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
   `provider_session_id` is stored **only** when the provider supports resume
   (claude); for codex/agy/shell it stays `None`.
3. Writes the session row, then `mkdir -p`s the cwd.
4. **Pre-trusts** the folder for the provider (`trust::ensure_trusted`, §6).
5. Merges MCP servers into the workspace `.mcp.json` (browser if opted in,
   the user's enabled `mcp-servers`, and Otto's first-party `otto` tool server
   when the workspace opted in via `otto_mcp_enabled`). All opt-in, best-effort,
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

**Two searches:**
- **In-viewport** — the xterm `SearchAddon` over the currently rendered buffer
  (instant, but lost on reconnect).
- **Server-side ring search** — the `{"type":"search"}` frame greps the full
  10,000-line ring (plain substring, case-insensitive, ANSI-stripped) and
  returns up to 200 matches in buffer order. Use it after reopening a session or
  to find output that scrolled off. The UI find bar runs both: local first, then
  a 300 ms-debounced server query.

### Resize

`{"type":"resize","cols","rows}"` triggers `manager.resize`, which both resizes
the PTY and persists the grid to `meta.pty_cols` / `meta.pty_rows` so a future
respawn frames its first snapshot correctly. The client only sends a resize on
an *actual* dimension change (not on every `fit()`), to avoid SIGWINCH flicker
on `claude`/`codex` repaints.

### Watching, splitting, tiling

- **Split view** (`Splits.svelte`) — arrange panes side-by-side/stacked; drag
  the gutter to resize (column/row fraction clamped 0.2–0.8). With ≥2 panes and
  ≥2 session targets a **broadcast bar** appears: *"↗ broadcast"* sends one line
  to all visible sessions via `POST /workspaces/{id}/broadcast {text,
  session_ids}`.
- **Tiled view** (`TiledView.svelte`) — see every session at once in a grid (1→2
  →3→4 columns by count). To preserve the idle-suspend memory design, **at most
  `MAX_LIVE_TILES` = 6 tiles are live** (open a WS + resume): always the focused
  tile, then user-pinned tiles, then visible tiles (tracked by an
  `IntersectionObserver`), up to the cap. Everything else is a lightweight
  **placeholder** — a header + status dot + provider chip + *"Click to attach"* —
  that opens **no terminal, no WebSocket, no resume**. Clicking a placeholder
  pins and focuses it; scrolling a live tile off-screen (over budget) tears down
  its WS so the session can re-suspend. Without this, opening a tiled view of M
  suspended sessions would wake all M agents (~200 MB each) at once.

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
| `reconnectable` | The PTY is gone (idle-suspend or daemon restart) but the conversation can be resumed on demand — **0 RAM**. |

Status changes broadcast as `session_status` events on `/ws/events` (§9).

### Resumability across daemon restarts

On daemon boot, `SessionManager::restore_all` deliberately does **not** respawn
any agent processes (keeping every historical session resident would cost
~200 MB each). Instead every restorable session is marked `reconnectable` and
resumed **lazily** the moment a client opens it: `ensure_live` sees a
non-live but resumable session and calls `restart`, which spawns the provider
with its **resume args**. Claude keeps the full conversation in its on-disk
JSONL transcript, so `--resume <provider_session_id>` restores it completely.

A session is **resumable** iff it is an `agent` kind, has a
`provider_session_id`, **and** its provider supports resume — which today means
**only `claude`**. `codex`, `agy`, and `shell` never store a
`provider_session_id` and are not resumed; reopening them after suspend/restart
starts a fresh process (no lost-work risk is taken — see suspend below).

> **Transcript pruning.** For non-live `claude` sessions, the daemon checks
> whether the on-disk transcript (`~/.claude/projects/<encoded-cwd>/<sid>.jsonl`,
> with a directory-scan fallback) still exists; a row is pruned only when the
> transcript is *positively confirmed gone*. Unknowable cases (other providers,
> no `$HOME`) are always kept (`lifecycle.rs`).

### Idle-suspend (save memory)

A background sweep (`suspend_idle_unattached`) frees the RAM of a live session
when **all** of these hold:

1. **Resumable** — agent kind + `provider_session_id` + provider supports resume
   (so codex/agy/shell are never auto-suspended; their work would be lost).
2. **Idle** — no PTY output for the full grace window. Default **5 minutes**
   (`SUSPEND_GRACE`), overridable via the `idle_suspend_grace_secs` setting.
3. **Unattached** — no WS viewer is currently watching (tracked by an
   `AttachGuard` reference count that decrements on every WS teardown path).
4. **Not pinned** — `meta.keep_alive` is not `true`.

On suspend the daemon kills and drops the live PTY (freeing memory), **keeps the
row** with its `provider_session_id` intact, sets status `reconnectable`, and
records a lifecycle trail entry. Reopening auto-resumes (above). The session pane
shows a live countdown for an idle, non-pinned agent: *"Nm idle · suspends in
Mm"*, with the tooltip *"Session is idle. Auto-suspend frees its RAM while
keeping it resumable."*

**Pin to keep alive:** the ⋯ menu offers *"Pin (keep alive)"* / *"Unpin (allow
auto-suspend)"* (agent sessions only), toggling `meta.keep_alive`. A pinned
session is never auto-suspended.

### Restart

`POST /api/v1/sessions/{id}/restart` (or the pane's refresh button, tooltip
*"Restart session"*) respawns the session: it kills any live PTY, rebuilds the
spec, **uses the resume args when `provider_session_id` is set** (so a claude
restart resumes the same conversation; others start fresh), re-applies
`--add-dir`/`--model` from `meta`, re-trusts the folder, re-wires the ingest env,
restores the saved grid, and records a *"Session resumed"* trail entry. Returns
the updated `Session`.

### Close, archive, delete

- **Close pane** (the pane `×`, tooltip *"Close pane (keeps running)"*) only
  detaches the UI; the session keeps running on the daemon.
- **Archive** — `POST /sessions/{id}/archive` keeps the row + history but hides
  it from the active list (shown in an "Archived" section); `…/unarchive`
  restores it as `reconnectable`. Channel-spawned sessions auto-archive after
  long idleness.
- **Delete** — `DELETE /api/v1/sessions/{id}` kills the PTY and removes the row.
  The UI marks this action as danger.
- **Quit hook** — `POST /api/v1/app/kill-sessions` terminates every live PTY
  (the desktop app's quit hook).

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
     "trusted"`.
   - Unknown providers (incl. `agy`) are left alone here.

2. **Prompt-guard (runtime backstop, `prompt_guard.rs`).** An `OutputScanner`
   watches each session's PTY output; when a **known approval prompt** for the
   provider appears in the recent tail it writes the accepting keystroke back.
   Detection is intentionally narrow — specific full phrases (e.g. *"do you trust
   the files in this folder"*, *"allow codex to work in this folder"*, *"press
   enter to continue"*) so it never injects keys into the agent's real work on a
   false positive — and it is debounced per session (≤ once / 5 s). claude is
   accepted with `1\r` (select "Yes"); codex/agy with `\r`. This catches what
   pre-trust can't: providers without a known trust config and unexpected
   first-run dialogs. Anything it does *not* match is caught by the analysis
   stuck-detector (idle → retry → notify), so no prompt hangs forever. Each
   auto-approval is recorded on the session's activity trail.

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
| `GET /workspaces/{id}/sessions` | ws viewer (`Agents:View`) | `Session[]` (you see your own; ws-admin/root see all) |
| `POST /workspaces/{id}/sessions` | ws editor (`Agents:Edit`) | `CreateSessionReq` → `Session` |
| `GET /sessions/{id}` | ws viewer | `Session` |
| `PATCH /sessions/{id}` | ws editor | `UpdateSessionReq{title?, meta?}` → `Session` |
| `DELETE /sessions/{id}` | ws editor | 204 — kills PTY, removes row |
| `POST /sessions/{id}/restart` | ws editor | respawn (resume when `provider_session_id` set) → `Session` |
| `POST /sessions/{id}/input` | ws editor | `SendInputReq{text, submit?}` → 200 |
| `POST /sessions/{id}/archive` / `…/unarchive` | ws editor | 204 |
| `POST /sessions/{id}/handover` / `…/handover/brief` | ws editor | start a handover / generate its brief (see §10) |
| `POST /sessions/{session_id}/attach-product` | ws editor | `{story_id}` — attach a product story |
| `POST /workspaces/{id}/broadcast` | ws editor | `BroadcastReq{text, session_ids?}` → `BroadcastResp{session_ids}` |
| `POST /app/kill-sessions` | member | terminate every live PTY |
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
- Run claude / codex / agy / shell as real PTYs you can watch, type into, split,
  and tile; add custom providers via settings without a rebuild.
- Survive daemon restarts and idle-suspend without losing a claude conversation.
- Multi-attach: several clients watch the same session, output broadcast to all.
- Persist 10,000 lines of scrollback across reconnects, grep-searchable
  server-side.
- Auto-trust the workspace folder and auto-clear stray approval prompts.
- Stream a live activity trail + task tracker, and drive everything over HTTP/WS.

**Limitations / by design:**
- **Resume is claude-only.** codex/agy/shell are not resumed; after a daemon
  restart or (for codex/agy/shell never) auto-suspend they start fresh. They are
  also never *auto*-suspended (their work would be lost).
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

---

## 10. Handover (move context between agents)

A **handover** pushes the working context of one agent into another (e.g.
Claude → Codex) so you don't re-explain a task by hand. `POST
/sessions/{id}/handover` (Editor on the source's workspace) spawns the target in
the **same workspace + cwd**, returns it immediately, then in the background
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
  non-admins to their own sessions.
- **Viewer = read-only terminal.** A workspace viewer may attach and watch but
  cannot send input/resize (frames dropped server-side). Editor+ may drive it.
- **Share-link throttle.** WS token validation is rate-limited per IP: **10**
  failures within a **15-minute** window locks that IP out for **15 minutes**
  (HTTP 429 + `retry-after`). A successful auth clears the IP's tally.
- **Force-terminate.** An admin terminate or a revoked mobile share-link evicts
  every attached viewer with a `{"type":"terminated"}` frame and an immediate
  socket close.
- **Folder trust** is granted only for the workspace folder the user chose (and
  its path variants); the prompt-guard accepts only a narrow phrase set (§5).
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
- **Reopening a codex/agy/shell session lost its context.** Those providers are
  not resumable — only `claude` resumes. Use claude for long-lived resumable
  work, or pin (`keep_alive`) a codex/shell session you must keep warm (note:
  pinning prevents *auto*-suspend, but a daemon restart still starts it fresh
  since it has no resume).
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
  PTY was suspended (idle) or the daemon restarted; opening/focusing the session
  resumes it. The terminal overlay offers a **Now** / **Reconnect** / **Resume**
  button.
- **Custom provider not appearing.** Confirm it's in the `providers` settings
  JSON and that `cmd` is on `PATH` (`GET /meta.tools` reports detected tools).

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
- **`./daemon-http-api.md`** — tokens, base URL, and the WS auth handshake for
  driving Otto over HTTP.
- **`./rtl-and-responsive.md`** — RTL/bidi and touch/mobile terminal behavior.
- **`docs/MULTI-USER-RBAC.md`** — the full RBAC, ownership, and isolation model.
- **`docs/contracts/api.md`** / **`docs/contracts/ws.md`** — the authoritative
  REST and WebSocket contracts.
