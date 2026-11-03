# Threads (Experimental)

`Threads` (`#/threads` and `#/threads/<sessionId>`) is Otto's dedicated,
inbox-style conversational workspace for coding-agent sessions (`claude`,
`codex`, `agy`, and registered custom providers). It complements classic
terminal **Agents** (`#/agents`) — both views operate on the exact same live
PTY-backed sessions in `ottod`, so you can switch between `#/threads` and
`#/agents` without restarting or duplicating any process.

## Layout & Workflows

0. **Top bar:** sidebar toggle, a `project / thread` breadcrumb, and on the
   right a layout menu (single · split · grid), three panel toggles —
   **Changes**, the focused thread's **Terminal** drawer, the **right panel** —
   and the thread's ⋯ menu (rename, pin, search, system notes, peer relay,
   room, settle, archive). Single view has no separate pane header.
1. **Sidebar (`ThreadSidebar`):**
   - One search row with **filter by project** (several at once; a "Showing …
     · Clear" line appears while a filter is on), **Add project** (a folder that
     shows in the rail before its first thread) and **New thread**.
   - **Pinned**, then each **project** with its threads — threads needing
     approval first, then working ones, then most recent; `delegated_by`
     workers nest under their lead.
   - **History** (settled + archived threads) folded at the bottom.
   - Footer: Settings, Git, Usage, and Refresh (sessions + git status).
2. **New Thread Stage (`NewThreadStage`):**
   - "What should we build in *project*?" (click the project to switch folder
     or workspace) and a single composer: attach a file (`@path`), one agent
     chip holding the harness, model and reasoning effort (plus "Start without
     a prompt" and "Open beside current thread"), and a round send button.
   - Under it: **Current checkout** (or another git worktree of the repo), the
     branch (switch or create), and the access mode (Full access · Accept
     edits · Plan, `⇧⇥`).
   - On wide windows a **project card** floats top-right; it also heads the
     right panel: folder, **Open in…** (Files, Git, a terminal thread, copy
     path), **project scripts** (saved per folder in this browser; running one
     opens a shell thread), branch, **Create PR** (the Git Create-PR sheet,
     which names the GitHub/Bitbucket destination before sending) and
     **Changes +N −M** (opens the Changes tab).
3. **Multi-Session Split & Grid Canvas (`ThreadCanvas` / `ThreadPane`):**
   - Switch between **single**, **Split** (2-up side-by-side), and **Grid**
     (2×2 up to 4 live threads) from the top-bar layout menu or `⌘K`.
   - Each pane renders rich folded turns (`ConversationView`: GFM Markdown,
     syntax-highlighted code blocks, inline workspace images via
     `GET /api/v1/fs/raw` with zoomable lightbox, expandable `Thinking` prose,
     MCP tool cards, and incoming peer-session attribution chips) plus an
     on-demand slide-up **Terminal** drawer attached to the live PTY.
   - Composer supports `@file` typeahead, image attachments, inline MCP
     permission docks, and **Steer** (interrupt/nudge immediately) vs **Queue**
     (hold until the active turn finishes).
4. **Workspace Inspector (`ThreadInspector`):**
   - **Files:** Workspace file tree + multi-tab file viewer (`Preview | Source`
     for Markdown and images, line-numbered code viewer, `file:line` jump).
   - **Changes:** Working-tree staged/unstaged status + Unified/Split git diff
     viewer. When the branch is ahead of its upstream, an **Uncommitted ·
     Unpushed N** switch reviews the local commits a push or Create PR would
     send — all of them as one range (`range:<upstream>...HEAD`) or one commit
     at a time (`commit:<sha>`), newest 50.
   - **Side chat:** ask about the focused thread without interrupting it —
     `/btw <question>` in its composer, the thread ⋯ menu, or this tab. The
     side chat is a normal agent session tagged `meta.side_of = <main id>`
     (hidden from the thread lists), started in Plan mode with the same
     harness/model, and seeded with the main thread's last 8 turns, folder,
     branch and uncommitted files. **Open as thread** clears the tag and opens
     it beside the main thread; **Dismiss** archives it. In Agents, `/btw` is
     passed to the CLI unchanged.
   - **Connections:** Workspace DB/SSH connections, live `db_mcp_activity`
     stream whenever an agent executes `otto_db_query`, and **Open
     side-by-side** to launch Database Explorer in Otto's split pane.
   - **Peers:** Direct session-to-session message relay (`POST /sessions/{id}/message`
     with `from_session_id`), multi-session broadcast, live cross-session peer
     timeline, and **Start Room** (`Session Rooms`).

## API & WebSocket Surface

- `GET /api/v1/sessions/{id}/transcript` (`?sub=<subagentId>` supported for
  Claude and Antigravity subagents)
- `POST /api/v1/sessions/{id}/message` (optional `from_session_id` records
  structured sender attribution on the workspace trail)
- `GET /api/v1/fs/tree`, `GET /api/v1/fs/read`, `GET /api/v1/fs/raw`
  (workspace-confined binary image streaming with `Content-Security-Policy:
  default-src 'none'` + `X-Content-Type-Options: nosniff`)
- WebSocket event `db_mcp_activity` (`connection_id`, `session_id`,
  `session_title`, `tool`, `sql`, `status`, `row_count`, `duration_ms`,
  `columns`, `rows_preview`)
