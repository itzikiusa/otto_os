# Threads (Experimental)

`Threads` (`#/threads` and `#/threads/<sessionId>`) is Otto's dedicated,
inbox-style conversational workspace for coding-agent sessions (`claude`,
`codex`, `agy`, and registered custom providers). It complements classic
terminal **Agents** (`#/agents`). New Claude, Codex, Gemini, Grok, opencode
and pi threads use the [structured runtime](threads-structured-runtime.md). Existing terminal coding
sessions can also open in Threads through their transcript view, keeping the
same live PTY and conversation.

## Continue an agent in Threads

From an agent's **⋯ → Open in Threads** (or the labelled **Threads** button
on a phone), continue reading and messaging the same session as a chat. The
session id, running process, provider conversation, permissions and delegated
workers stay attached. Only its preferred surface changes; unrelated metadata
is preserved. If saving the preference fails, Otto stays on the current view
and shows the error.

Use **Thread actions → Open in Terminal** to return to Agents. This action is
available for existing coding-agent PTYs, including nested coding agents; it
does not convert a structured runtime thread or a plain shell into a new agent.
Structured threads retain their existing project terminal drawer.

On compact screens the sidebar and inspector open on demand. File links,
`/files`, `/diff` and `/btw` open the corresponding inspector, and Escape or the
close button returns to the conversation. Unsent runtime messages, attachments,
model, effort and access choices belong to each thread. Structured runtime
drafts also recover after reload or restart, as described below; they do not
carry into another conversation.

## Layout & Workflows

0. **Top bar:** four controls — sidebar toggle, a `project / thread`
   breadcrumb (with the **Related threads** chip while the thread has any),
   the **right panel**, and ⋯. The ⋯ holds the thread's actions (rename, pin,
   search, system notes, Related threads…, Continue on phone, settle,
   archive), then New thread, Project card, Terminal drawer and the layout
   (single · split · grid). Back / Forward are ⌘⇧← / ⌘⇧→. Single view has no
   separate pane header. See
   [Threads layout](./threads-structured-runtime.md#threads-layout).
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
     (2×2 up to 4 live threads) from the top-bar ⋯ or `⌘K`.
   - Each pane renders rich folded turns (`ConversationView`: GFM Markdown,
     syntax-highlighted code blocks, inline workspace images via
     `GET /api/v1/fs/raw` with zoomable lightbox, expandable `Thinking` prose,
     MCP tool cards, and incoming peer-session attribution chips) plus an
     on-demand slide-up **Terminal** drawer attached to the live PTY.
   - Composer supports `@file` typeahead, image attachments, inline MCP
     permission docks, and **Steer** (interrupt/nudge immediately) vs **Queue**
     (hold until the active turn finishes).
4. **Workspace Inspector (`ThreadInspector`):**
   - **Agents & tasks** (structured threads, hidden when there are none):
     every subagent and background task — running first with its elapsed
     time, the last five finished under one "N finished" line
     (`runtime/agentTasks.ts`, `ThreadAgentsTasks.svelte`). The strip above
     the composer names one background task or counts several ("2 running in
     the background"); tapping it opens this section.
   - **Files:** Workspace file tree + multi-tab file viewer (`Preview | Source`
     for Markdown and images, line-numbered code viewer, `file:line` jump).
   - **Changes:** Working-tree staged/unstaged status + Unified/Split git diff
     viewer. When the branch is ahead of its upstream, an **Uncommitted ·
     Unpushed N** switch reviews the local commits a push or Create PR would
     send — all of them as one range (`range:<upstream>...HEAD`) or one commit
     at a time (`commit:<sha>`), newest 50. **Add file diff to chat**, **Add
     hunk to chat**, or a line-number button attaches exactly that selection
     to the focused runtime thread's draft, with old/new line numbers.
   - **Pull request:** The branch's open PR and its review conversations,
     including replies and outdated/resolved markers. **Add to chat** attaches
     a conversation with its author, PR URL and file/line location.
   - **Side chat:** ask about the focused thread without interrupting it —
     `/btw <question>` in its composer, the thread ⋯ menu, or this tab. The
     side chat is a normal agent session tagged `meta.side_of = <main id>`
     (hidden from the thread lists), started in Plan mode with the same
     harness/model, and seeded with the main thread's last 8 turns, folder,
     branch and uncommitted files. **Open as thread** clears the tag and opens
     it beside the main thread; **Dismiss** archives it. In Agents, `/btw` is
     passed to the CLI unchanged.
   - **Connections** (offered once the thread has database activity):
     Workspace DB/SSH connections, live `db_mcp_activity` stream whenever an
     agent executes `otto_db_query`, and **Open side-by-side** to launch
     Database Explorer in Otto's split pane.
   - Session-to-session messages, broadcasts and **Start a room** live in the
     header's **Related threads** popover (`POST /sessions/{id}/message` with
     `from_session_id` for a terminal thread).

## Recoverable runtime drafts

The composer saves unsent text, attachments, selected review/diff context,
model, reasoning effort, access mode and the **Next message starts fresh**
choice to the daemon's SQLite state, scoped to your user, workspace and
thread. It does not store them in browser local storage. The composer
says nothing while saving works and shows **Draft not saved · Retry** when it
fails; recovery never sends a
message automatically. You can type while a draft loads, but send waits for
recovery. A late load preserves edits made in the meantime.

If saving fails, the draft remains on screen with **Draft not saved** and
**Retry**. When another window has saved a competing version, automatic writes
stop. **Retry** explicitly saves this window's version against the latest
revision; it does not merge competing text. Wait for **Saved** before relying
on recovery after closing the app. A failed send retains the draft. An accepted
send or queued message removes only the submitted content, preserving edits or
attachments added during the request; model/effort/access choices remain.

Draft limits are explicit, without silent truncation:

| Limit | Value |
|---|---|
| Text | 1 MiB |
| Attachments | 12; PNG, JPEG, GIF, WebP or PDF, up to 10 MiB decoded each |
| Review/diff context | 32 chips; up to 128 KiB text per chip |
| Whole draft | 24 MiB serialized, including base64 attachments |
| Per person, per workspace | 64 MiB and 128 drafts with unsent content |
| Retention | 30 days since the last save; expiry also removes attachments and preferences |

Attachment support at send time still depends on the provider. To resolve a
size error, remove an attachment, choose a smaller diff excerpt, or clear an
older draft, then Retry. Preference-only drafts do not use a nonempty-draft
slot. Deleting a thread deletes its saved draft.

## Answer an agent's questions

A supported provider question appears under **Needs your answer**, separately
from permission approvals. Choose one option, several options, or type an
answer according to the form. **Write another answer** appears where free text
is allowed. **Send answer** replies to that request; it does not send a normal
chat message or approve a tool. Required answers and offered values are checked
before sending. **Skip question** dismisses the request without granting a
permission. Completed or skipped requests remain under **Previous questions**.

Supported forms are Claude `AskUserQuestion`, Codex
`item/tool/requestUserInput`, and session-scoped ACP `elicitation/create` forms
from providers that emit them (including the Gemini/Grok/opencode transport).
ACP support is limited to flat text and string-choice fields, including
multiple choices. Numeric/boolean fields, nested forms, validation constraints,
URL/auth/request-scoped elicitations and secret questions are refused explicitly.
The pi and Antigravity adapters do not provide typed question forms.

Several questions can remain open and be answered independently. A provider
withdrawal, Stop or close makes the affected requests no longer answerable. A
daemon restart does not: with session persistence on (the default) the agent
keeps running and its open questions and approvals stay answerable
([how](threads-structured-runtime.md#4-history-replay-and-restarts)). Only an
agent that did not survive the restart must ask again; an old card cannot
answer a new process's request.

## Start fresh in the same thread

Enter `/clear` to arm **Next message starts fresh**; **Cancel** disarms it.
Nothing is sent or stopped by arming it. Your next accepted message starts a
new provider conversation in the same visible thread, with a **New session**
divider. Earlier messages remain readable, but the new agent conversation has
no memory of them unless you include that context. The thread, folder,
workspace and files remain the same. `/new` still opens a different thread.

Fresh sends require no active turn, background work, queued messages or pending
requests. Finish or stop that work, answer or skip requests, and send or remove
queued messages before retrying. A closed idle thread can start fresh directly.
If initialization or local prompt validation fails, no divider is added, the
previous provider identity is restored for resume, and the draft stays armed.

## Add context and inspect tool images

Review conversations and selected diff files, hunks or lines become removable
chips above the runtime composer. Selecting the same item again updates its
chip in place. The existing draft text stays untouched; selection neither
sends a message nor adds an instruction to fix anything. Tap/click a chip to
read its full content, or remove it before sending. On a phone, adding context
closes the inspector and returns focus to the composer. Thread switches discard
late inspector responses so a previous thread's selection cannot enter another
draft. Context is attributed and assembled with your text only when you send.

Expand a tool row to see image previews alongside its text and structured
result. Claude/MCP, Codex MCP and ACP/OpenCode results retain supported inline
PNG, JPEG, GIF and WebP images: at most four, 1 MiB decoded per image and 2 MiB
total per result. Click or tap a thumbnail to enlarge it; Escape or Close returns
to the thumbnail. Unsupported, malformed, remote-only or over-limit images
produce an omission notice. Otto does not fetch tool image URLs or render
HTML/SVG as previews. Images remain available in stored runtime history.

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

For structured threads, the corresponding routes are
`GET/PUT /api/v1/runtime-sessions/{id}/draft`,
`POST /api/v1/runtime-sessions/{id}/prompt` (`fresh_context?`), and
`POST /api/v1/runtime-sessions/{id}/requests/{request_id}` (question answers,
skip/cancel, or permission decisions). Live state and replay use
`agent_runtime_event` and `GET /api/v1/runtime-sessions/{id}/events`.
See the [runtime guide](threads-structured-runtime.md) and authoritative
[API](../contracts/api.md) / [WebSocket](../contracts/ws.md) contracts for wire
shapes and authorization.
