# Threads (Experimental)

`Threads` (`#/threads` and `#/threads/<sessionId>`) is Otto's dedicated,
inbox-style conversational workspace for coding-agent sessions (`claude`,
`codex`, `agy`, and registered custom providers). It complements classic
terminal **Agents** (`#/agents`). New Claude, Codex, Gemini, Grok, opencode,
pi and Muse threads use the [structured runtime](threads-structured-runtime.md). Existing terminal coding
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

0. **Top bar:** four controls — sidebar toggle, the project's two-letter
   badge and a `project / thread` breadcrumb in reading size (the title
   bold; double-click or F2 renames), the **right panel**, and ⋯. The ⋯ holds
   the thread's actions (rename, pin, search, system notes, **Send to another
   thread…**, Continue on phone, Move to History, close), then New thread,
   Terminal drawer and **View ▸** (single · split · grid, **Thread details**
   while the card can float; a phone gets no View). Back / Forward are ⌘⇧← /
   ⌘⇧→.
   - **Thread details** (`ThreadDetailsCard`): one card for the thread's
     facts — the folder with **Open in ▾** (Files, Git, a terminal thread, copy
     path, and the folder's **project scripts**: run one, add one, remove
     one), the branch ▾, **one git action** for the checkout's state
     (**Commit** → **Push** / **Publish branch** → **Create PR** → **PR #N**;
     the chevron beside it holds all three; push asks where it goes first and
     Create PR uses the Git sheet that names the destination; the row hides
     when there is nothing to do), **Changes +N −M** ("No changes" at zero;
     opens Changes), then **Subagents · N running**: the thread it works under
     first ("↑ parent") when it is itself a subagent, each running subagent
     with its agent's glyph, status word (Working · Needs you · Done · Failed ·
     Stopped), elapsed time and **Stop**, and the rest under **Previous**. A
     row with its own thread opens it (on a phone the drawer gives way);
     a turn's own subagent opens Agents & tasks. Background commands and
     timers stay in Agents & tasks. It works the same for terminal threads
     (folder, git, their delegated threads).
   - It **floats top-right** of the conversation while the right panel is
     closed and one conversation (or the New thread stage) has a whole
     reading column left beside it; the conversation moves over
     (`--thread-card-reserve` on the stage), so the card never covers a
     message. Anywhere else (a split, a narrower window, the right panel open,
     a phone or tablet) the same card heads the right panel / details drawer.
     Its **×** hides the floating card on this device; ⋯ › View › **Thread
     details** brings it back.
   - **⌘K opens on your threads** (`lib/paletteThreads.svelte.ts`,
     `threadSwitcher.ts`, `threadSwitcherSource.svelte.ts`): in Threads the
     first group of the ⌘K bar (and of the phone's command sheet, More ›
     Commands) is **Threads**, ordered and worded by the sidebar's inbox
     (`threadInbox.ts`): what needs you (**Approval** / **Question**, read
     once per waiting thread when the bar opens), **Failed**, finished unread
     (**Done**), then **Working**, then the rest, in the sidebar's order inside
     each, with the project per row and the order held while it is open.
     Typing filters threads by title and project with the commands; the last
     command row is **New thread: <query>** (the New thread stage with the
     query as the draft). Enter opens in the focused pane, ⌘↵ beside it when
     a split is open. Outside Threads the bar gains **Threads needing you**
     only while one waits (there the word is "Needs you").
   Single view has no
   separate pane header. See
   [Threads layout](./threads-structured-runtime.md#threads-layout).
1. **Sidebar (`ThreadSidebar`):**
   - One search row with **filter by project** (several at once; a "Showing …
     · Clear" line appears while a filter is on), **Add project** (a folder that
     shows in the rail before its first thread) and **New thread**.
   - **An inbox, not a status board** (`threadInbox.ts`). One list across
     projects: threads that need you first (**Approval**, **Question**), then
     **Failed**, then finished-unread (**Done**), then by the time the thread
     last came back to you, newest first. Each row is one button with three
     lines: the project's badge and name … the status word (else the
     relative time); the title (bold while it needs you or is Done); the
     branch (worktree icon in a worktree, the folder's name when no branch is
     known) … the agent's glyph. Pin / Open beside / ⋯ appear on hover and
     focus; on touch they are in the row's long-press menu.
   - **Words, not dots.** Approval · Question · Failed · Done · Working 18s.
     Question vs Approval comes from the thread's pending requests (live
     `runtime_request.updated` events; after a reload, one compact read of
     the thread's newest turn — `threadInbox.svelte.ts`), as does the
     working time. A terminal thread waiting on you reads Approval.
   - **Done** is "the last turn ended after you last had the thread in front
     of you", on every device: the thread's `meta.thread_seen_at` (written
     once each time you focus it with the window visible, or when a turn ends
     while it is in front of you) against its `meta.last_turn_ended_at`
     (`threadMeta.svelte.ts`). A thread never marked seen anywhere falls back
     to this window's own flag (a turn it saw end off screen).
   - **Subagents** (threads started from a thread) fold under it by default;
     the row says "N subagents" and, when one of them needs you, the
     thread's own word becomes **Needs you** (the tooltip names it). Opened,
     they are two-line rows with Working · Needs you · Done · Failed ·
     Stopped. The focused thread's family opens itself.
   - **Pinned** sits on top when there are pins; **Working N** (running
     threads not waiting on you, open by default) and **History N** (moved to
     History + closed, folded) sit below the list. The focused thread stays
     visible inside a folded section.
   - **Group by project** in the filter menu brings back the per-project
     folders (IDE projects first, pinned projects first, each foldable with a
     "1 needs you" note when folded); remembered on this device.
   - **Undo, not confirm** (`threadUndo.ts`). Pin / unpin, Move to History /
     back and dismissing a side chat act at once with one "… · Undo" notice
     for 5 s; a newer action replaces it, and ⌘Z / Ctrl+Z runs it while focus
     is not in a text field, editor or terminal. Archiving a working session
     still asks first.
   - **Pins, History and "seen" live on the thread** (`meta.thread_pinned`,
     `thread_history`, `thread_seen_at`; `PATCH /runtime-sessions/{id}` or
     `PATCH /sessions/{id}` `meta`), so the Mac and a phone agree, live
     (`provider_session.updated` `thread` / `session_meta_updated`). A change
     shows at once; a refused write rolls back with the error toast. This
     browser's old local lists move onto the threads once. A shared / guest
     link cannot change them. The inbox's recency is when a thread's last
     turn ended, so a pin or a look never reorders it.
   - Footer: Settings, Git, a **usage meter** in the Usage slot — the focused
     thread's agent's tightest plan window ("5h 62%" used; warning from 80 %,
     danger from 95 %; every window and its reset in the tooltip; click opens
     Usage) when Usage has fresh numbers for that agent (Codex, Claude, agy,
     Grok, Muse logins), else the plain icon — and Refresh. The meter reads
     the Usage page's own `GET /usage/limits` poller
     (`lib/stores/usageLimits.svelte.ts`, 60 s while visible).
2. **New Thread Stage (`NewThreadStage`):**
   - "What should we build in *project*?" (click the project to switch folder
     or workspace) and a single composer: attach a file (`@path`), one agent
     chip holding the harness, model and reasoning effort (plus "Start without
     a prompt" and "Open beside current thread"), and a round send button.
   - Under it: **Current checkout** (or another git worktree of the repo, or
     **New worktree** — see below), the branch (switch or create), the interaction mode (Full access ·
     Auto-accept edits · Plan, `⇧⇥`) and the access chip (below). A
     structured thread started in **Plan** stays in Plan: every follow-up asks
     for a plan until you turn **Plan** off in the thread's composer (or press
     `⇧⇥` there). The mode is kept in this window.
   - **New worktree** (the checkout chip's menu, or **Use a new worktree** on
     the "another thread is working in this checkout" line) records the
     intent only: the chip reads **New worktree from <base>** and nothing is
     asked. Sending the first message names the branch from it — a short
     kebab slug of its first words, under the repo's own prefix when most of
     its branches use one (`feat/…`), `-2`, `-3` on a collision, a dated
     `thread-YYYY-MM-DD-HHMM` when started without a prompt
     (`worktreeBranch.ts`) — creates it (`POST /repos/{id}/worktrees`) and
     then starts the thread there, whichever agent it is. **Name the branch…**
     in the same menu sets the name (prefilled with the generated one).
     Progress shows under the composer; a failure keeps the draft and its
     attachments and shows the error with **Retry**.
   - The thread details card (above) floats top-right where there is room
     and heads the right panel otherwise.
3. **Multi-Session Split & Grid Canvas (`ThreadCanvas` / `ThreadPane`):**
   - Switch between **single**, **Split** (2-up side-by-side), and **Grid**
     (2×2 up to 4 live threads) from the top-bar ⋯ or `⌘K`.
   - Each pane renders rich folded turns (`ConversationView`: GFM Markdown,
     syntax-highlighted code blocks, inline workspace images via
     `GET /api/v1/fs/raw` with zoomable lightbox, expandable `Thinking` prose,
     MCP tool cards, and incoming peer-session attribution chips) plus an
     on-demand slide-up **Terminal** drawer attached to the live PTY.
   - Composer supports `@file` typeahead, image attachments, inline MCP
     permission docks, and **Queue** (hold until the active turn finishes) vs
     **Steer** (the agent reads it at its next step) — see *Composer and
     timeline* below.
4. **Workspace Inspector (`ThreadInspector`):**
   - **Agents & tasks** (structured threads, hidden when there are none):
     every subagent and background task — running first with its elapsed
     time, the last five finished under one "N finished" line
     (`runtime/agentTasks.ts`, `ThreadAgentsTasks.svelte`). The strip above
     the composer names one background task or counts several ("2 running in
     the background"); tapping it opens this section.
   - **Files:** Workspace file tree + multi-tab file viewer (`Preview | Source`
     for Markdown and images, line-numbered code viewer, `file:line` jump).
   - **Changes** (key **C**; was "Diff"): Working-tree staged/unstaged status + Unified/Split git diff
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
   - Session-to-session messages, broadcasts, opening a related thread beside
     and **Start a room** live in ⋯ › **Send to another thread…**
     (`POST /sessions/{id}/message` with `from_session_id` for a terminal
     thread); the thread's subagents are listed in the details card.

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
| Attachments | 20; PNG, JPEG, GIF, WebP or PDF, up to 10 MiB decoded each and 16 MiB decoded in total per message |
| Review/diff context | 32 chips; up to 128 KiB text per chip |
| Whole draft | 24 MiB serialized, including base64 attachments |
| Per person, per workspace | 64 MiB and 128 drafts with unsent content |
| Retention | 30 days since the last save; expiry also removes attachments and preferences |

The composer refuses a file that would pass the per-file or per-message
attachment limit and says which limit, under the composer; the files already
attached stay. A draft the daemon refuses for its size shows the daemon's
reason with **Retry**, and is not uploaded again until it gets smaller.
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

Select text in an agent's answer and a **Quote** button appears above it:
it adds the passage to the composer as a markdown quote (`> …`) below what
you already typed, caret on a free line under it (T3 Code #33124, #34181).
It is UI-only, so every provider has it. The button is
`lib/components/SelectionQuote.svelte`, the text `lib/quote/quote.ts`.

## Give an agent a secret privately

An agent that needs a token or a password calls `otto.request_secret`
(`otto_request_secret` in a session; on by default). A card appears above
the thread's composer, for the thread's owner only: **The agent asks for a
secret: …**, a password field, **Send** and **Decline**. The value goes only
into that tool call's result. It is never put in the composer, a draft, the
timeline or Otto's database: the request lives in the daemon's memory for up
to about 3 minutes, and the thread masks the value as `••••••` in everything
it shows or stores afterwards (values shorter than 4 characters are not
masked). The agent CLI's own transcript on disk still records the tool
result, as it does for any tool. Threads only (T3 Code #15907, #30257).

## This round's thread view (2026-11)

- **Attention dock.** What the agent waits on — an approval, a question form
  or a secret request — sits in one dock directly above the composer
  (`runtime/RuntimeAttentionDock.svelte`, model `runtime/attentionDock.ts`),
  with its live controls; the timeline row only records the request
  ("Waiting for your approval · Answer", which jumps to the dock). Several
  requests step with ‹ / › ("1 of 3"). From an empty composer **⌘↵** approves
  and **⌘⌫** declines. The dock is capped to a share of its pane (45 % with a
  mouse, 60 % on touch) and never pushes the composer off screen.
- **Stop.** While a turn runs with nothing typed, Stop is the composer's one
  button; with a draft it is a small control beside Queue / Steer. **Esc**
  stops (press it twice when the composer has text) and **⌘.** stops
  (`runtime/stopKeys.ts`).
- **A failed turn** is one block (`runtime/turnFailure.ts`) that names the
  cause — not signed in, usage limit, CLI missing, CLI unsupported, stopped
  unexpectedly, provider error — with Details and Revive.
- **"Reply from …" rows**: a child thread's or another agent's reply that came
  back into this thread is attributed to its sender, not shown as yours.
- **Token usage per turn**: a finished turn shows its token count
  (`runtime/turnUsage.ts`); child threads and council seats show their total.
- **Read-only seats**: a council seat's access chip is a static
  **Read-only** chip (seats always run under the read-only sandbox).
- **New-thread stage defaults**: the folder starts at the project you last
  used in this workspace, else a repo of a recent thread, else the workspace's
  repo or root, else **Home** — shown, never silent (`newThreadDefaults.ts`).
  The access chip starts from Settings › Providers › *Skip permission
  prompts* (on = Full access, off = Supervised) and the first turn is sent
  with exactly what the chip shows (`threadAccessDefault.svelte.ts`).
- **Vocabulary**: **Move to History** (was Settle), **Reopen thread** for a
  closed thread vs **Revive thread** for a stopped agent, **Changes** (was
  Diff), "panel" (was surface), **Auto-accept edits**, and the toolbar's
  **Thread list** / **Right panel** toggles.
- **Closing a thread with child threads** says they keep running and offers
  to close them too (`threadClosePlan.ts`).
- **Split / Grid strip.** With 2+ panes a pane strip shows one status chip
  per pane — Working / Needs you / Answered / Failed / Stopped / Ready — that
  focuses its pane, and **Message every pane…**. When every pane is one
  council it is the council strip instead: "Council · *title*", seat chips
  "Seat N · model", **Ask every seat…** (a multi-line sheet that says which
  stopped seats it restarts), **Synthesize** (disabled, with its reason,
  until a seat has answered and none is working), **Stop all** while a seat
  works, **Continue with this answer** (opens the new-thread stage with the
  quoted answer; nothing is sent) and **Send to a thread ▸**
  (`runtime/PaneStrip.svelte`, `runtime/paneStrip.ts`). Pane headers read
  "Seat N · model · effort" with token counts once the conversation is
  loaded. On a phone a council thread shows a one-tap seat switcher and one
  **Council ▾** menu instead. The **Add a thread** slot of Split / Grid lists
  structured threads too (needs you → working → recent).
- **Message a thread › All open threads in this project** also reaches the
  structured threads in the same folder and reports who could not receive it
  (`threadRelay.ts`).
- **The sidebar is an inbox** (rows, words, Working / History folds, Group by
  project, subagents, Undo, the usage meter): see Layout › Sidebar above.

## Composer and timeline (2026-11 redesign)

What a structured (runtime) thread's reading view and composer do; every
provider (Claude, Codex, ACP agents — Gemini, Antigravity, Grok, OpenCode,
Muse — and pi) unless a line says otherwise.

- **Reading column.** Conversation and composer share one column, 760 px
  wide and centred in the pane (single, Split, Grid; a narrower pane or a
  phone fills it with `--sp-6` gutters). Messages are `--fs-l` / 1.6: your
  message a right-aligned bubble (`--surface-2`), the agent's answer plain
  prose. Turns sit `--sp-9` apart; wide code and tables scroll inside the
  column. A transcript (PTY) thread in Threads gets the same column and type
  (`ThreadPane.svelte`); the classic Agents view is unchanged. A message's
  time and **Copy** appear on hover / focus (always on touch; always in the
  accessibility tree).
- **Work fold.** A finished stretch of steps is one dim line with a hairline
  under it: "Worked for 2m 13s · ran 4 commands and edited 3 files"
  (`runtime/workLog.ts` `foldLine`, T3's `summarizeToolGroup` sentence).
  While a turn runs, the finished steps before the live one fold to "N earlier
  steps" (`earlierSteps`); a failure, an approval or a question — and what
  follows it — never folds.
- **Tool calls** (toolbar ⋯ › View): *show all* (default), *hide successful*
  or *hide all* (`workLog.ts` `visibleTools`, per device). Failures, gated
  calls, threads the agent opened, replies, questions and notices always show.
- **Changes card.** A finished turn that changed files ends with "3 changed
  files +6 −2 · Open diff", listing each file's +/− (open up to five files,
  folded past that; `runtime/turnChanges.ts`,
  `RuntimeTurnChanges.svelte`). The counts come from the turn's own
  file-change items, the same shape from every driver (a unified `diff_str`,
  or `old_str` / `new_str`); a file whose items carry no text shows its name
  only. **Open diff** or a file opens the Changes panel on that file
  (`threads.changesFocus`).
- **Composer card.** Input at `--fs-l`, two rows tall, growing to 40 % of the
  pane. Bottom row: **+** (`RuntimeComposerPlus.svelte`: attach an image or
  PDF, mention a file or agent, **Plan mode**, **Access: …**), the **Ask
  another agent** icon, then only non-default state — the Plan chip while
  Plan is on, the access chip when the thread is not on your default access
  (Settings › Providers › *Skip permission prompts*: on = Full access, off =
  Supervised), a read-only seat, or an embed that fixes the access — then
  the model chip (`RuntimeModelControl.svelte`) and one primary button.
- **One primary button** (`runtime/composerSend.ts`): **Send** when idle;
  **Stop** while the agent works and nothing is typed; with a draft, **Queue**
  ("sends when this turn ends", clock icon) or **Steer** ("the agent reads it
  at its next step", bolt icon), Stop then a small quiet control beside it.
  **↵** does the default mid-turn delivery and **⌘↵** (Ctrl+↵ off macOS) the
  other; on a touch-first device ⌘↵ stays the default (it is the only
  keyboard send there). The default is per device: toolbar ⋯ › View ›
  *While working, messages queue / steer* (`threadPrefs.svelte.ts`, also on a
  phone). Steer goes the queue bar's way (`delivery: steer`). An agent
  without the `steer` capability (Gemini, Antigravity, OpenCode, Muse — ACP
  agents other than Grok) only queues: the button says "… can't take a
  message mid-turn", ⌘↵ shows a notice, and a steer the daemon queued anyway
  is reported as "Queued instead".
- **Slash popup.** `/` lists Otto's commands, then "<Agent> commands": the
  agent's own commands and skills (`commands.updated` — Claude's initialize
  list, Codex `skills/list`, ACP `available_commands_update`, pi
  `get_commands`), fuzzy filtered, with argument hints. Choosing one types
  `/name ` to finish and send as a normal prompt. An agent command Otto takes
  itself (`/review`, `/clear`, the view commands the host handles) is left
  out. The list scrolls and keeps the active row in view.
- **↑ recalls prompts** (`runtime/promptHistory.ts`): in an empty composer
  ↑ fills the last prompt you sent in this thread, ↑ again goes back, ↓
  forward and past the newest back to empty; any edit leaves history. Not in
  your own draft, with the mention or slash menu open, or during IME
  composition. Text only.
- **Drop files** on the thread (`runtime/composerDrop.ts`): they attach
  through the same intake as paste (images and PDFs, the same per-file and
  per-message limits); a type the agent cannot take is refused by name in
  the composer; only the composer box lights up. The desktop app keeps the
  webview's own HTML5 drop (`dragDropEnabled: false`).
- **Terminal selection → Add to chat** (`TerminalAddToChat.svelte`): select
  text in the thread's terminal drawer or the Threads terminal dock and a
  small **Add to chat** button attaches it to the focused structured
  thread's draft as a "Terminal · N lines" chip (sent as "Terminal output"
  context). Nothing is sent and the draft stays.
- **Notices above the composer** show one quiet line at a time (an urgent
  one always shows); the rest fold into "N more: …" (`runtime/banners.ts`).
  On a phone on its side or with the on-screen keyboard up, every quiet
  notice (the queue strip too) folds into that one line, and a pane under
  420 px tall gets a one-row input with tighter padding, so the latest
  message stays in sight (the timeline keeps following the bottom as the
  keyboard opens).

## On a phone and a tablet

Threads is the main way to steer agents on the Mac from a phone, so the page
keeps the conversation readable and the composer usable with the on-screen
keyboard up, upright and on its side.

- **One header row.** On a phone (upright or on its side) and on a tablet the
  shell's own top bar steps aside on Threads; the Threads header is the only
  row: thread list, the thread's title, right panel and ⋯. On a phone on its
  side it also holds **Open navigator** (the Navigator drawer carries the
  bell), the row is 40 px, and the thread list is a drawer. Upright, Otto's
  modules are in the bottom bar.
- **The keyboard** (`lib/viewportFit.ts`, `lib/stores/keyboard.svelte.ts`).
  Otto decides "the keyboard is open" once for every page: a text field has
  focus and the visible viewport dropped well below the keyboard-free height
  at this width (iOS shrinks only the visual viewport; Android is told to do
  the same through `interactive-widget=resizes-visual`, and an older
  "resizes-content" Android is measured against the tallest height seen at
  that width). While it is open, `html[data-kb="open"]` hides the bottom bar
  (and the Agents action bar) on every page and `--kb-h` holds its height.
- **Typing in a thread** folds what the reply does not need, the least needed
  first (`threadsTyping.ts`): under 560 px of visible height the terminal
  drawer; under 420 px the plan line and the queued-messages fold; under
  340 px (a phone on its side, an iPhone SE) the header row, and the composer
  becomes one row — text, attach, Stop/Send — with the attention dock showing
  its one-line head while the caret is in the composer. Everything returns
  when the keyboard closes. The composer never leaves the screen: the notices
  and the dock shrink (and scroll inside) first, the timeline keeps at least
  a one-line peek, and a timeline that was following the latest message stays
  at its end as the screen shrinks.
- **Composer at 375 px**: one foot row — Send/Stop, attach, ask-agent, the
  model chip (truncated, never below ~5 letters) and the access mode as its
  icon (its name stays in the label and the menu).
- **Code blocks wrap** on a phone (upright or on its side); the block's
  **Wrap** button turns wrapping off. Diffs keep one line per line and scroll
  sideways — their +/− column is the meaning — and fenced ```` ```diff ````
  blocks show added / removed lines in green / red in both themes.
- **iPad**: Otto's navigator folds to its icon rail while Threads is open
  (`pageChrome.foldNav`; expanding the rail brings it back) and the thread
  list is a column beside the conversation; the right panel stays a drawer.
  Other pages keep the tablet's navigator column.
- **Safe areas**: the installed app draws under the status bar and notch
  (`viewport-fit=cover`); the shell pads each edge once from
  `--safe-top/-bottom/-start/-end` (`app.css`), the bottom bar pads the home
  indicator, and nothing is added above an open keyboard.
- **Menus** are an opaque surface on touch screens, so the conversation never
  shows through a ⋯ menu.

## Find in a thread

⌘F opens the app's find bar (next / previous, match count, highlight) over
the thread like any page. A long thread mounts only its newest 200 rows, so
past that the thread registers a find provider
(`modules/threads/runtime/threadFind.ts`) over every loaded message's text:
a match in an older message widens the mounted window to show it. History
not loaded yet (above "Load older") is not searched. A focused terminal or
editor keeps its own ⌘F (T3 Code #10439).

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
