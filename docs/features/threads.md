# Threads (Experimental)

`Threads` (`#/threads` and `#/threads/<sessionId>`) is Otto's dedicated,
inbox-style conversational workspace for coding-agent sessions (`claude`,
`codex`, `agy`, and registered custom providers). It complements classic
terminal **Agents** (`#/agents`) — both views operate on the exact same live
PTY-backed sessions in `ottod`, so you can switch between `#/threads` and
`#/agents` without restarting or duplicating any process.

## Layout & Workflows

1. **Inbox & Project Rail (`ThreadSidebar`):**
   - **Pinned:** Quick-access threads pinned locally per browser profile.
   - **Active:** Threads currently running or waiting on human input/approval.
   - **Projects:** Threads grouped by workspace/repository with nested
     `delegated_by` sub-worker rows and branch badges.
   - **History:** Completed or settled threads that remain one click away.
2. **New Thread Stage (`NewThreadStage`):**
   - Centered launch stage with harness cards (**Claude Code · Codex ·
     Antigravity**), workspace/folder picker, git branch chip, model selector,
     and initial prompt composer.
3. **Multi-Session Split & Grid Canvas (`ThreadCanvas` / `ThreadPane`):**
   - Switch between **`1-up`**, **`Split`** (2-up side-by-side), and **`Grid`**
     (2×2 up to 4 live threads) from the header or `⌘K`.
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
     viewer.
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
