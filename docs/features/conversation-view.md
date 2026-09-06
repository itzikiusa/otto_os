# Conversation view, History, Tasks board & Outputs

Every agent session in Otto can be read as a **Claude/Codex-app-style
conversation** instead of (or beside) its raw terminal; every past conversation
— Otto's own sessions *and* the transcripts the CLIs already left on disk — is
searchable and resumable from a **History** page; the per-session **task
tracker** is two-way (the agent's plan comes in, board tasks go out); and the
files, PRs and images a session produced are listed with previews in an
**Outputs** panel. This is the end-user and operator reference for the feature;
`docs/design/conversation-view.md` is the design record and
`docs/contracts/api.md` / `ws.md` the API source of truth.

> The one decision everything else follows from: the conversation is **rebuilt
> from the provider's own transcript on disk** — Claude
> `~/.claude/projects/<cwd-slug>/<sid>.jsonl`, Codex
> `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<sid>.jsonl` — never from PTY
> scrollback. The parser is one shared crate (`otto-transcript`, extracted from
> `otto-usage`), so the chat header, the Usage module and cost tracking always
> agree.

---

## 1. Summary

| You want to… | Otto gives you |
|---|---|
| Read what an agent did without scrolling a terminal | **Chat view** — a segmented **Terminal · Chat · Split** toggle in every agent session header (`⌘⇧C` cycles); user bubbles, assistant prose, collapsible "Worked for 21m · 38 steps" work groups, diffs, images, subagent cards |
| Find that conversation from last week | **History** (`#/history`) — search + provider / folder / status / date filters, grouped by repo like the Claude/Codex app sidebars; read-only on the right |
| Continue a conversation the CLI had outside Otto | **Resume in Otto** — imports the on-disk transcript as a reconnectable session, then the normal resume path (`claude --resume` / Codex rollout) continues it |
| See the agent's plan and push work into it | **Task tracker** in the Activity panel — TodoWrite/TaskCreate tasks merged with **+ Add task** rows (`from board` badge, `queued` until the agent is nudged) |
| Drive many agents from one board | **Mission Control** cards show `done/total`, take an inline **+ Sub-task**, and open the session straight into Chat |
| Get the files/PRs/images a session produced | **Outputs** right-panel tab — list + sandboxed preview (HTML, Markdown, images, PDF), download for the rest; also inline under a History conversation |

---

## 2. Overview & where it lives

| Layer | Path | Responsibility |
|---|---|---|
| Parser crate | `crates/otto-transcript/` | `parse_claude_line` / `parse_codex_line` (moved from `otto-usage`, re-exported there), `fold(records) → Transcript`, the two-era Codex adapter, subagent tree reader, image extraction, the offset `Tailer`; fixtures under `fixtures/{claude,codex-new,codex-old}` |
| Transcript resolution | `crates/otto-sessions/src/lifecycle.rs` | `transcript_path(session)` — Claude by provider session id, Codex via the persisted `sessions.transcript_path` (migration 0121) |
| Routes | `crates/otto-server/src/routes/transcript.rs` | Transcript paging, images, artifacts, board tasks, inbox upload, History list/read/import/rescan (§5) |
| Tail supervisor | `crates/otto-server/src/transcript_tail.rs` | Polls live sessions' transcripts (700 ms), folds, broadcasts `transcript_appended` / `artifact_added`; reads the PTY screen for the `transcript_live` draft. Armed by an open chat (`GET` + 60 s `touch` pings) and by the workspace keep-alive for every live session you may read, ≤ 64 at once, stops 2 min after the last ping |
| Live draft | `ui/src/modules/agents/conversation/LiveDraft.svelte` | Sub-turn streaming: the screen text under the last folded turn, deduped against it |
| Tasks merge + nudge | `crates/otto-state` (`ActivityRepo::replace_tasks`, migration 0122), `crates/otto-server/src/agent_tasks_nudge.rs` | Agent rows replaced on sync, user rows merged by title/ext_id; one sweep hands board tasks to the PTY |
| History index | migration 0123 `transcript_index`, background walker | Head+tail scan of both provider roots; `history_index_progress` |
| Conversation UI | `ui/src/modules/agents/conversation/` + `SessionView.svelte`, `ui/src/lib/stores/transcript.svelte.ts` | `ConversationView` (`sessionId` or `transcriptPath` mode, `readonly`), the view toggle, composer |
| History UI | `ui/src/modules/agents/history/` | `HistoryPage.svelte`, the `history` store, the ⌘K command |
| Tasks / board UI | `ui/src/modules/panels/ActivityPanel.svelte`, `ui/src/modules/agents/MissionControl.svelte`, `ui/src/lib/stores/activity.svelte.ts` | Add task, badges, done/total strip, sub-tasks |
| Outputs UI | `ui/src/modules/panels/OutputsPanel.svelte`, `ui/src/shell/RightPanel.svelte` | The **Outputs** tab and the embedded History variant |
| Types | `ui/src/lib/api/types.ts` (`// ── Transcript` section) | Mirror of `otto-transcript/src/model.rs` |
| Contracts (authoritative) | `docs/contracts/api.md`, `docs/contracts/ws.md` | Routes, RBAC, event scopes |

Entry points in the app: the **Chat / Split** toggle in any agent session
header; **History** in the left nav (Agents group), the **History** button in the
Agents header and in Mission Control, and ⌘K **"Go to History"**; the
**Activity** and **Outputs** tabs of the right panel (⌘J) while an agent session
is focused.

---

## 3. Prerequisites & setup

Nothing to configure. The feature works for any `claude` or `codex` session
Otto spawned (it knows the provider session id, and persists the Codex rollout
path the moment its capture scan finds it) and for any transcript already under
`~/.claude/projects` or `~/.codex/sessions`.

- **Claude**: the transcript resolves from `sessions.provider_session_id`. A
  session that never got one (killed before the CLI wrote its id) shows the
  terminal only, with `unavailable_reason: 'no_provider_session_id'`.
- **Codex**: rollouts are matched by cwd once and then stored on the session row
  (`sessions.transcript_path`). Until the match lands the chat reports
  `codex_rollout_unresolved` and the toggle stays on Terminal.
- **agy (Antigravity)**: history is a SQLite store with encrypted usage, not
  JSONL — the adapter is a stub (`provider_unsupported`); the UI shows the
  terminal only.
- **History index**: a low-priority walk of both roots starts at daemon boot
  and skips unchanged `(mtime, size)` files; the **↻ Rescan** button on the
  History page re-runs it on demand.

---

## 4. Full feature walkthrough

### 4.1 The view toggle (Terminal · Chat · Split)

Every agent session header has a segmented control. **Terminal** is the default
for every session; **Chat** is opt-in (greyed-out empty state when no transcript
resolves). **Split** puts the chat
on the left and the live terminal on the right behind a draggable splitter
(three columns when the right panel is open; below 1200 px it degrades to Chat
with a "Terminal" tab). The choice is remembered per session
(`otto_session_view:<id>` in localStorage — the same key Mission Control and
History set to `chat` before opening a session). `⌘⇧C` cycles the three.

### 4.2 The conversation

- **Turns**: your prompts as right-aligned bubbles, the agent's prose full-width
  (Markdown through the sanitizing vault renderer). The last 60 turns load
  first; **Load earlier** pages back by the opaque `cursor`. Scrolling is
  anchored; when you are at the bottom the view follows live turns, otherwise a
  **↓ new** pill appears.
- **Work steps**: consecutive tool calls fold into one "Worked for … · N steps"
  row. Expand a step for the capped output, a diff (edits), a file chip (opens
  the Files panel), or a subagent card — subagents load lazily and nest from the
  sidecar tree, so agents spawned by *other* subagents still show up.
- **Markers, not bodies**: Claude persists `thinking` blocks with an empty body
  (a signature only), so the chat shows a **"Thought (n)"** marker. Codex
  reasoning is never recoverable — a footer says **"N reasoning steps (not
  recorded)"** from `stats.reasoning_steps` (the contract carries no per-turn
  count, so it renders once per conversation).
- **System notes**: reminders, hook output, attachments and `<task-notification>`
  payloads collapse into one muted chip per turn; **Show system** reveals them.
  Queued prompts show as "Queued: …" chips until the CLI dequeues them.
- **Composer** (live session, editor role): `⏎` sends (via the same submit path
  the PTY uses, so slash commands pass through), `⇧⏎` newline; paste or drop an
  image and it is uploaded to the session inbox and referenced as
  `[Image: <path>]`. The status line shows the session status and how many
  board tasks are waiting to be nudged in. An exited session shows **Resume**.

### 4.3 History (`#/history`)

The left pane is the list: a search box (titles + first prompts, server-side),
then **provider**, **folder**, **status** and **date** filters. Rows group by
repository (or shortened cwd) with the newest group first; each row shows the
provider glyph (**C** Claude / **◇** Codex), the AI title or the first prompt,
a status dot (`running` / `idle` / `exited` / `resumable` / hollow `on disk`),
relative time and the turn count. **Load older** pages by keyset
(`before = last_active_at`).

Select a row and the right pane shows a **read-only** conversation: an `on_disk`
row is read through the path route, any other row through its session. The
header offers:

| Action | What it does |
|---|---|
| **Resume in Otto** / **Open in Otto** | `on_disk` → `POST …/history/import` creates a `reconnectable` session, then `POST /sessions/{id}/restart` continues it with the provider's resume args; `exited`/`reconnectable` → restart; live → just opens. Always lands in **Chat**. |
| **Open folder** | Reveals the cwd in the OS file manager (desktop app; falls back to copying the path when no bridge is available — the toast says so). |
| **Copy path** | Copies the transcript path (right-click → also the folder path). |
| **Archive** | Archives the Otto session (not offered for `on_disk` rows — there is nothing to archive). |

Right-click (or the ⋯ button) on any row opens the same actions in the clamped
context menu. Below the conversation an **Outputs** section (collapsed) lists
the artifacts of that conversation — for `on_disk` rows these are folded out of
the transcript itself, listed without previews until the conversation is
resumed. **↻ Rescan** kicks the index walk and shows `scanned/total` from the
`history_index_progress` events, reloading the list when done. Deep link:
`#/history/<sessionId>`.

### 4.4 Tasks — the Activity panel

The **Task tracker** section (right panel → Activity) shows the agent's own
plan (Claude's TodoWrite / TaskCreate / TaskUpdate) merged with tasks you add:

- **+ Add task** opens an inline form (title, optional description) →
  `POST /sessions/{id}/tasks`. The row appears at once with a **from board**
  badge (`source:'user'`) and a **queued** badge while `nudge_pending` is set.
- **Delivery**: one sweep in the daemon hands each pending task to the agent as
  a single prompt — `Otto board: new task — "<title>". <description> Add it to
  your task list and do it next.` — when the session is idle, or after at most
  120 s while it is working (never while a permission prompt is pending). The
  header count shows how many are queued.
- **Merge, not replace**: when the agent republishes its list, only
  `source:'agent'` rows are swapped; a user row whose title matches (or whose
  `ext_id` a TaskUpdate names) takes the agent's status — ids stay stable.
- **Codex** has no task tool, so its list contains only what you add; the panel
  says so ("Codex does not publish a plan").
- The **done/total** everywhere (Navigator chips, session header, Mission
  Control) counts all rows, user-added included.

### 4.5 Mission Control

Any card backed by a session gets a **task strip**: `done/total`, a progress
bar and the in-progress task, from `GET /workspaces/{wid}/activity/summary`
(kept live by `tasks_updated`). **+ Sub-task** opens an inline input on the card
(`⏎` adds, `Esc` cancels) → the same `POST /sessions/{id}/tasks`. Clicking a
card opens the session in **Chat** (the view key is set before navigating). The
header's **History** button jumps to `#/history`.

### 4.6 Outputs

The right panel's **Outputs** tab lists every artifact the transcript folder
registered for the focused agent session — files written (`Write`/`Edit`/
Codex `FileChange`), PRs (`pr-link` + PR URLs), images, reports (previewable
files under scratch/temp/data dirs: `.html .md .png .jpg .svg .pdf .csv .json`).
Selecting one previews it:

| Type | Rendering |
|---|---|
| HTML | `<iframe sandbox="">` via `srcdoc` — no scripts, forms, popups or same-origin (the MockupViewer approach) |
| Markdown | `vault/mdRender.ts` (marked + allowlist sanitizer) |
| Images | `<img>` from an authenticated blob URL |
| PDF | sandboxed `<iframe>` |
| Text / CSV / JSON | `<pre>`, capped at 200 KB in the DOM |
| PR / URL | external link |
| Anything else | download link |

Bytes are fetched **only** by opaque artifact id
(`GET /sessions/{id}/artifacts/{artifact_id}`); the panel never sends a path.
`artifact_added` events append to the list live.

---

## 5. API / contract reference

All routes are under `/api/v1`. RBAC: `Agents` **View** for every GET,
`Agents` **Edit** for every POST (explicit `policy.rs` arms above the
`/sessions/{id}/*` catch-all; share-scoped tokens keep their feature guard).

| Route | Body | Returns | Notes |
|---|---|---|---|
| `GET /sessions/{id}/transcript?before=&limit=&sub=` | — | `Transcript` | Omit `before` = last `limit` turns; `has_earlier` drives "Load earlier"; `sub=<agent_id>` reads a subagent (turns + tool stats only). A session with no resolvable transcript returns `200 { unavailable_reason }`, never 404 |
| `GET /sessions/{id}/transcript/images/{img_id}` | — | image bytes | `inline`, `nosniff`; images are extracted once to `<data>/transcripts/<sid>/img/` |
| `GET /sessions/{id}/artifacts` | — | `Artifact[]` | Dedup per path, last producing turn wins |
| `GET /sessions/{id}/artifacts/{artifact_id}` | — | bytes | Opaque id → server-side path → canonicalize + allow/deny list + 25 MB cap; inline for images/HTML, attachment otherwise |
| `POST /sessions/{id}/tasks` | `{ title, description? }` | `AgentTask` | Inserted `source:'user'`, `nudge_pending:1` |
| `POST /sessions/{id}/inbox` | `{ filename, mime, data_b64 }` | `{ path }` | `image/*` only, 10 MB, stored under `<data>/sessions/<id>/inbox/` |
| `GET /workspaces/{wid}/history?q=&provider=&cwd=&status=&before=&limit=` | — | `HistoryEntry[]` | Otto sessions (all statuses, archived included) merged with unclaimed index rows (`status:'on_disk'`) |
| `GET /workspaces/{wid}/history/transcript?path=&before=&limit=&sub=` | — | `Transcript` | The one route that accepts a client path — it must resolve (symlink-aware) under `~/.claude/projects` or `~/.codex/sessions` |
| `POST /workspaces/{wid}/history/import` | `{ provider, transcript_path }` | `Session` | Creates a `reconnectable` row with `provider_session_id` + `transcript_path` |
| `POST /workspaces/{wid}/history/rescan` | — | `202` | Background index refresh |
| `GET /sessions/{id}/slash-commands` | — | `SlashCommand[]` | Composer completion after `/`: provider built-ins + `~/.claude/{commands,skills}` + `<cwd>/.claude/{commands,skills}` (Codex: skills dirs) |
| `POST /workspaces/{wid}/transcript/touch` | — | `{armed}` | Workspace-wide keep-alive (every 60 s while visible + on switch/focus): every live session you may read in the current workspace stays tailed; other workspaces lapse after 2 min and are re-armed when you return |
| `POST /sessions/{id}/transcript/touch` | — | `204` / `409` | Keep-alive from an open chat (every 60 s); the tail stops 2 min after the last touch, so only open conversations are tailed. The same ping holds the session against the idle-suspend sweep for 3 min (`VIEW_HOLD`) — a chat mounts no terminal, so this is its attachment; the workspace-wide touch does NOT hold (it only warms tails). `409` = live session without a transcript yet (the view retries the GET every 5 s) |
| `GET /workspaces/{wid}/activity/summary` | — | `SessionActivitySummary[]` | Existing; `done/total` count ALL task rows |

### WebSocket events (`/ws/events`)

| Event | Scope | Payload |
|---|---|---|
| `transcript_appended` | Session | `{ workspace_id, session_id, cursor, turns }` — ≤ 64 KB per frame, else the client re-fetches |
| `transcript_live` | Session | `{ workspace_id, session_id, text, input, status, branch }` — the in-progress response read off the PTY screen (≤ 16 KB), at most one frame per 700 ms poll and only on change; the chat renders it as a "Streaming from the terminal" draft while the session is `working` and hides it once the folded turn covers it |
| `artifact_added` | Session | `{ workspace_id, session_id, artifact }` |
| `history_index_progress` | Workspace | `{ workspace_id, scanned, total, done }` |
| `tasks_updated` | Session | existing — carries the merged list incl. `source` / `nudge_pending` |

Types live in `ui/src/lib/api/types.ts` under `// ── Transcript`
(`Transcript`, `Turn`, `Block`, `ToolResult`, `Artifact`, `HistoryEntry`,
`SubagentMeta`, …) mirroring `crates/otto-transcript/src/model.rs`. Track A's
corpus numbers and any route-level deviations are recorded in
`docs/design/conversation-view.md` §8.

---

## 6. Capabilities & limitations

- **Lossless, quiet parser.** Every record maps to a block, a system note or a
  stat; unknown records become a `notice` and count in `stats.unknown_records`
  (the corpus test requires 0 outside synthetic temp dirs).
- **Thinking is a marker.** Claude persists `thinking` blocks empty (4,186 of
  4,192 in the census) — the UI shows "Thought (n)", never a body.
- **Codex reasoning is counted, not shown.** `summary`/`raw_content` are empty in
  100% of the corpus; you get "N reasoning steps (not recorded)".
- **Two Codex eras, decided per file.** CLI ≥ 0.147 writes `item_completed`
  items; older rollouts are rebuilt from `event_msg agent_message/user_message/
  patch_apply_end` + `response_item function_call/_output`. In a new-era file
  `response_item` records feed stats only, so nothing is double-counted.
- **Subagents** come from the `<sid>/subagents/*.meta.json` sidecars, so depth-2/3
  agents spawned by sibling subagents (45% of them) are reachable even when the
  parent's `Agent` result never mentions them.
- **Cost/tokens** are deduped by `(message.id, requestId)` exactly as
  `otto-usage` does — the chat header and the Usage module never disagree.
- **Sizes.** Paging is by turns (60 first); tool output is capped at 64 KB per
  block (`truncated:true`); images are served by id, never inlined; the History
  index reads only the head 64 KB + tail 16 KB of each file; artifact bytes cap
  at 25 MB; the Outputs text preview caps at 200 KB.
- **Live tails** run for at most 32 sessions at once (700 ms poll); beyond that
  reads still work, only the push stops. A tail stops 60 s after exit or after
  5 min without a subscriber.
- **Idle ≠ finished.** Otto's `idle` means "no PTY output for 5 s", so a board
  task may be handed over while the agent is mid-turn after the 120 s max defer
  — the CLI queues typed input, so nothing is lost, but the task shows up as a
  prompt in the conversation.
- **`on_disk` rows have no session**, so their Outputs list has no previews and
  they cannot be archived; **Resume in Otto** turns them into sessions.
- **Open folder** needs the desktop app with the opener permission; elsewhere it
  copies the path and says so.
- **agy** sessions are terminal-only (no JSONL transcript).
- Rows only ever reflect Claude/Codex; shell sessions do not appear in History.

---

## 7. Security & permissions

- Every new route has an explicit `policy.rs` arm; WS events are `Scope::Session`
  (`history_index_progress` is `Scope::Workspace`) — transcript prose and tool
  output never reach another workspace's clients.
- The server derives every file path itself, except `history/transcript?path=`,
  which accepts only paths resolving (symlink-aware) under the two provider
  roots. Artifacts are served by opaque id with the `routes/fs.rs` discipline:
  canonicalize → allow-list (session cwd, daemon data dir, the per-user temp
  dir — not `/tmp`) + deny-list (`.git`, `.env*`, keys) → 25 MB cap → `nosniff`.
- Rendering: Markdown and tool text only through the sanitizing `mdRender`;
  HTML only inside `sandbox=""` iframes (opaque origin — cannot reach the daemon
  or cookies); images `nosniff`.
- Input: the composer submits exactly what you typed (plus an explicit
  `[Image: <path>]` line). The only Otto-authored prompt is the board-task
  nudge, and it is sent only by the single nudge sweep.
- Secrets are never in transcripts Otto writes; transcripts it *reads* are the
  CLIs' own files and are shown as-is to members with `Agents` View.

---

## 8. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| The toggle is stuck on **Terminal**; chat says "no transcript" | `unavailable_reason` tells you which: `no_provider_session_id` (the CLI never reported its id — resume to get one), `codex_rollout_unresolved` (the cwd match hasn't happened yet — it lands on the next capture scan), `transcript_missing` (the file was deleted/moved), `provider_unsupported` (agy/shell). |
| A conversation you had in the terminal is missing from History | The index skips unchanged files; if the file is new, press **↻ Rescan** and watch `scanned/total`. Files under synthetic temp project dirs are excluded on purpose. |
| **Resume in Otto** created a session but the agent starts fresh | The provider needs its resume args: Claude by `provider_session_id`, Codex by the persisted rollout path. Check the imported row has both (`GET /sessions/{id}`); a Codex rollout from another machine cannot be resumed here. |
| An added task never reaches the agent (stays **queued**) | The sweep waits for `idle`, at most 120 s while `working`, and indefinitely while a permission prompt is pending — answer the prompt in the terminal. `nudged_at` on the row confirms delivery. |
| Board tasks vanish when the agent republishes its list | They shouldn't — only `source:'agent'` rows are replaced. If a title collides with an agent task the user row *adopts* the agent's status (that is the merge, not a loss). |
| Outputs shows a file but the preview is "Forbidden" | The path fell outside the allow-list (session cwd / data dir / user temp) or matched the deny-list (`.git`, `.env*`, keys). Copy the path and open it locally. |
| Mission Control strip shows `0/0` though the Activity panel lists tasks | The summary is loaded with the board; press **↻** or wait for the next `tasks_updated`. |
| A chat that sat idle for ~5 min goes dead (composer shows **Resume**, no new turns; a reload or opening the terminal brings it back) | The idle-suspend sweep judged the session *unattached*: a Chat-mode pane mounts no terminal WS. Fixed by counting the chat's 60 s `POST …/transcript/touch` as a viewer (`VIEW_HOLD`, 3 min); if you still see it, check the daemon log for `suspended idle, unattached session` and that the touch pings are reaching the daemon (they fail with "offline" in WKWebView when every network interface is down, e.g. right after sleep). |
| `Duplicate identifier` / type errors after pulling | `ui/src/lib/api/types.ts` mirrors `model.rs` — rebuild both sides together (see §8 of the design doc for the current hand-offs). |

---

## 9. Related docs

- [`../design/conversation-view.md`](../design/conversation-view.md) — design record, corpus census, delivery tracks and the append-only hand-off log (§8)
- [`./agent-sessions.md`](./agent-sessions.md) — sessions, PTY protocol, resume, the activity trail
- [`./mission-control.md`](./mission-control.md) — the work graph the board projects
- [`./usage-and-cost.md`](./usage-and-cost.md) — the usage tailer the parser was extracted from
- [`../contracts/api.md`](../contracts/api.md), [`../contracts/ws.md`](../contracts/ws.md) — authoritative API/WS contracts
