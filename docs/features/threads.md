# Threads (Experimental)

`Threads` (`#/threads` and `#/threads/<sessionId>`) is Otto's inbox-style
conversational workspace for coding-agent sessions. It complements the
classic terminal **Agents** page (`#/agents`). New Claude, Codex, Gemini,
Grok, opencode, pi and Muse threads use the
[structured runtime](threads-structured-runtime.md); an existing terminal
coding session (`claude`, `codex`, `agy`, a custom provider) can open in
Threads through its transcript view, keeping the same live PTY and
conversation. The Help page's Threads guide is the short version of this
page.

## What changed in this release

- **The sidebar is an inbox.** One list across projects, worded rows
  (Approval · Question · Failed · Done · Working 18s), Working and History
  folds, Undo instead of confirm. Any pending request — a provider's, an MCP
  server's or a subagent's — lifts the thread out of Working.
- **One vocabulary.** Subagents (threads a thread started) are Working · Needs
  you · Done · Failed · Stopped; compared models are "N compared"; every
  approval is **Allow once · Always allow · Deny · Always deny**; **Move to
  History** (was Settle), **Changes** (was Diff), **Reopen** a closed thread
  vs **Revive** a stopped agent.
- **Thread details card** with one git action for the checkout's state (it
  hides when there is nothing to do), the changes count and the subagents.
- **Composer**: one primary button (Send · Stop · Queue · Steer), **Delegate
  to…**, the agent's own slash commands, ↑ prompt history, drop files,
  recoverable drafts, **Continue with another provider**.
- **Context meter** and **usage limit as a state** with **Resume at reset**.
- **Ready-to-work worktrees** (`.worktreeinclude`, worktree setup).
- **Review notes** on the diff, the **minimap**, **Settings › Threads**.
- **MCP approvals hold the call**: the agent's tool call goes on as soon as
  you answer; nothing is retried.
- **Phone**: one header row, a keyboard-aware composer, wrapped code blocks.

## Inbox

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
  working time. A terminal thread waiting on you reads Approval. Any
  pending request — a runtime approval, an MCP server's ask (`mcpAsks`), a
  subagent's approval — makes the row Approval / Question and lifts it out
  of Working, even while the provider still reports the turn as working
  (`inboxClass` with the live request count).
- **Done** is "the last turn ended after you last had the thread in front
  of you", on every device: the thread's `meta.thread_seen_at` (written
  once each time you focus it with the window visible, or when a turn ends
  while it is in front of you) against its `meta.last_turn_ended_at`
  (`threadMeta.svelte.ts`). A thread never marked seen anywhere falls back
  to this window's own flag (a turn it saw end off screen).
- **Subagents** (threads started from a thread) fold under it by default;
  the row says "N subagents" (or "N compared" for compared models) and, when one of them needs you, the
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

## Thread

The **top bar** has four controls — sidebar toggle, the project's two-letter
badge and a `project / thread` breadcrumb in reading size (the title
bold; double-click or F2 renames), the **right panel**, and ⋯. The ⋯ holds
the thread's actions (rename, pin, search, system notes, **Send to another
thread…**, Continue on phone, Move to History, close), then New thread,
Terminal drawer and **View ▸** (single · split · grid, **Thread details**
while the card can float; a phone gets no View). Back / Forward are ⌘⇧← /
⌘⇧→.

**Continue an agent in Threads.** From an agent's **⋯ → Open in Threads** (or the labelled **Threads** button
on a phone), continue reading and messaging the same session as a chat. The
session id, running process, provider conversation, permissions and subagents
stay attached. Only its preferred surface changes; unrelated metadata
is preserved. If saving the preference fails, Otto stays on the current view
and shows the error.

Use **Thread actions → Open in Terminal** to return to Agents. This action is
available for existing coding-agent PTYs, including nested coding agents; it
does not convert a structured runtime thread or a plain shell into a new agent.
Structured threads retain their existing project terminal drawer.

On compact screens the sidebar and the right panel open on demand. File links,
`/files`, `/diff` and `/btw` open the matching right-panel section, and Escape or the
close button returns to the conversation. Unsent runtime messages, attachments,
model, effort and access choices belong to each thread. Structured runtime
drafts also recover after reload or restart, as described below; they do not
carry into another conversation.

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
- **A failed turn** is one block (`runtime/turnFailure.ts`) that names the
  cause — not signed in, usage limit, CLI missing, CLI unsupported, stopped
  unexpectedly, provider error — with Details and Revive.
- **"Reply from …" rows**: a child thread's or another agent's reply that came
  back into this thread is attributed to its sender, not shown as yours.
- **Token usage per turn**: a finished turn shows its token count
  (`runtime/turnUsage.ts`); subagents and compared models show their total.
- **Closing a thread with child threads** says they keep running and offers
  to close them too (`threadClosePlan.ts`).
- **Quote.** Select text in an agent's answer and a **Quote** button appears above it:
  it adds the passage to the composer as a markdown quote (`> …`) below what
  you already typed, caret on a free line under it (T3 Code #33124, #34181).
  It is UI-only, so every provider has it. The button is
  `lib/components/SelectionQuote.svelte`, the text `lib/quote/quote.ts`.
- **Find.** ⌘F opens the app's find bar (next / previous, match count, highlight) over
  the thread like any page. A long thread mounts only its newest 200 rows, so
  past that the thread registers a find provider
  (`modules/threads/runtime/threadFind.ts`) over every loaded message's text:
  a match in an older message widens the mounted window to show it. History
  not loaded yet (above "Load older") is not searched. A focused terminal or
  editor keeps its own ⌘F (T3 Code #10439).
- **Minimap** (`runtime/RuntimeMinimap.svelte`, `runtime/minimap.ts`):
  - A thread with six or more of your messages gets a slim rail at the
    reading column's edge: one tick per message, a failed turn and an
    approval turn marked.
  - The turn you are reading is lit; hover or focus shows the message's
    first line; a click jumps.
  - ⌥↑ / ⌥↓ step between your messages (not while typing).
  - On a phone the toolbar ⋯ › **Jump to…** opens the same list as a sheet.

**Split and Grid** (`ThreadCanvas` / `ThreadPane`):

- Switch between **single**, **Split** (2-up side-by-side), and **Grid**
  (2×2 up to 4 live threads) from the top-bar ⋯ or `⌘K`.
- Each pane renders rich folded turns (`ConversationView`: GFM Markdown,
  syntax-highlighted code blocks, inline workspace images via
  `GET /api/v1/fs/raw` with zoomable lightbox, expandable `Thinking` prose,
  MCP tool cards, and "Reply from …" attribution) plus an
  on-demand slide-up **Terminal** drawer attached to the live PTY.
- **Pane strip.** With 2+ panes a strip shows one status chip per pane —
  Working / Needs you / Answered / Failed / Stopped / Ready — that focuses its
  pane, and **Message every pane…**. The **Add a thread** slot lists
  structured threads too (needs you → working → recent). For compared models
  see *Subagents and other providers* below.
- **Message a thread › All open threads in this project** also reaches the
  structured threads in the same folder and reports who could not receive it
  (`threadRelay.ts`).

## Composer

**Starting a thread** (`NewThreadStage`):

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
- **New-thread stage defaults**: the folder starts at the project you last
  used in this workspace, else a repo of a recent thread, else the workspace's
  repo or root, else **Home** — shown, never silent (`newThreadDefaults.ts`).
  The access chip starts from Settings › Providers › *Skip permission
  prompts* (on = Full access, off = Supervised) and the first turn is sent
  with exactly what the chip shows (`threadAccessDefault.svelte.ts`).

**In a thread** (every provider unless a line says otherwise):

- **Composer card.** Input at `--fs-l`, two rows tall, growing to 40 % of the
  pane. Bottom row: **+** (`RuntimeComposerPlus.svelte`: attach an image or
  PDF, mention a file or agent, **Plan mode**, **Access: …**), **Delegate to…** (the people icon), then only non-default state — the Plan chip while
  Plan is on, the access chip when the thread is not on your default access
  (Settings › Providers › *Skip permission prompts*: on = Full access, off =
  Supervised), a compared model's thread (always read-only), or an embed that fixes the access — then
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
- **Stop.** While a turn runs with nothing typed, Stop is the composer's one
  button; with a draft it is a small control beside Queue / Steer. **Esc**
  stops (press it twice when the composer has text) and **⌘.** stops
  (`runtime/stopKeys.ts`).
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
- **Context meter** (`RuntimeContextMeter.svelte`, `contextMeter.ts`): a
  small ring beside the model chip once the provider has reported how full
  the context window is (`ProviderThread.context_usage`, pushed with
  `provider_thread.updated`, no polling). It shows the percent from 80 %
  (`--warning`) and turns `--danger` at 95 %. Its menu names the tokens and,
  when the agent publishes a `compact` command, offers **Compact the
  conversation** (types `/compact` into the composer; nothing is sent). The
  details card has the same **Context** row. Providers that report a window:
  Claude Code (the result frame's `modelUsage[model].contextWindow`), Codex
  (`thread/tokenUsage/updated` `modelContextWindow`), ACP agents (Gemini,
  agy, Grok, opencode, Muse) when they send `usage_update`, pi
  (`context_window`). Until one does, there is no meter.
- **Usage limit as a state** (`usageLimit.ts`, `RuntimeUsageLimit.svelte`,
  `runtime/usage_hook/resume_at.rs`): a turn that failed on the provider's
  usage limit reads **Limited** (sidebar, ⌘K switcher, details card; it ranks
  with Failed) and the composer has ONE line, *"Codex limit reached · resets
  14:00"*, with **Resume at reset** and **Continue with <other provider>**
  (arms the provider switch). The reset time comes from the provider's own
  words (Claude's `…limit reached|<epoch>` / "resets 3pm", Codex "try again
  in 2 hours", Gemini "retry in 41s", an ISO time) or else the Usage meter's
  limits snapshot (the shared 60 s visible-only poller, mounted only while
  the line shows and the error does not say); unknown → the line says
  *reset time unknown* and offers only the switch. **Resume at reset** calls
  `POST /runtime-sessions/{id}/resume-at`: the daemon keeps the time in the
  thread's `meta.resume_at` (every client sees *"resumes at 14:00 · Cancel"*
  live), sleeps until then and sends one short continue, unless it was
  cancelled, the thread was closed or its last turn no longer ended on the
  limit. Stop (a family Stop too) and any new message — a provider switch,
  a fresh start — cancel it. A daemon restart re-arms it (`recover`). Detection: Codex's
  structured `usageLimitExceeded`, and for Claude, Gemini / ACP agents and pi
  their limit text ("usage limit", "rate limit", "quota", "429",
  `RESOURCE_EXHAUSTED`), the same lists the daemon's `error_class` uses.

**Context chips and tool images.** Review conversations and selected diff files, hunks or lines become removable
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

**Recoverable drafts.** The composer saves unsent text, attachments, selected review/diff context,
model, reasoning effort, access mode, the **Next message starts fresh**
choice and an armed provider switch to the daemon's SQLite state, scoped to your user, workspace and
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

**Start fresh in the same thread.** Enter `/clear` to arm **Next message starts fresh**; **Cancel** disarms it.
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

## Subagents and other providers

- **Subagents** are threads a thread started: the agent's own (a
  provider's subagent such as Claude's Task, or `otto.thread_open`) or yours through
  **Delegate to…** (the people icon, or `@cod`, `@opus` in the composer).
  Each one gets your message and a short summary of the conversation; its
  answer comes back to the thread's agent (or to you with **Answer to me
  only**). A turn's subagents are one timeline row ("3 subagents · 2
  working"), the line under the timeline reads "Waiting on 2 subagents ·
  **Stop**", and the details card lists them. Stop stops the thread and all
  of its subagents. Their status words are always Working · Needs you · Done
  · Failed · Stopped. **Open** puts one in the right panel's **Subagent** tab
  (⌘-click: the main view; Settings › Threads changes the default). See
  [Threads inside a thread](threads-structured-runtime.md#threads-inside-a-thread).
- **Continue with another provider.** The model chip lists the thread's own provider and, below a divider, every
  other provider that is enabled and installed (named accounts too). Browsing
  one changes nothing; picking one of its models (or **Default model**) arms the
  switch: **Next message continues on Codex · Cancel** sits above the composer.
  Nothing is sent or stopped by arming it.
  
  Your next accepted message continues the same thread (id, sidebar row,
  folder, worktree, files, draft) on the new provider. The timeline shows a
  **Context handoff · Claude Code → Codex** divider; open it to read what was
  handed over and how much was left out. The new provider's first prompt is
  that block followed by your message, whole. The block holds whole turns,
  newest first, within about 16,000 tokens (never cut mid-message): your
  messages, the agent's final answers, the files each turn changed, a count of
  its tool calls (tool output is not replayed) and an open plan. Earlier images
  are not handed over, and the divider says how many were left. History before a
  plain **/clear** is not handed over.
  
  The rules are the fresh-start rules: no active turn, background work, queued
  messages or pending requests (the refusal names which one and how to clear
  it). If the new provider fails to start or refuses the message, no divider is
  added, the thread stays on the provider it ran (its conversation resumes), the
  draft stays armed and the reason shows above the composer. MCP servers,
  access mode, model defaults, sandbox and usage are set up for the new provider
  as for a new thread. The chip, the title glyph and the details card show the
  provider the thread runs on now. Switching back is another handoff: a fresh
  conversation on that provider, never a resume of the old one.
- **Compare models.** In the new-thread stage turn on **Compare models**
  beside the agent chip and pick 2–4 models; the send button reads **Ask 3**.
  Each model gets a normal, read-only thread (its access chip is a static
  **Read-only**); they open side by side (Split for 2, Grid for 3–4) and fold
  under the first one in the sidebar as "N compared". With all of them on
  screen the pane strip reads **Compare models · *title***: chips "2 · Opus",
  **Ask every model…** (a sheet that says which stopped models it restarts),
  **Compare answers** (disabled, with its reason, until a model has answered
  and none is working; the caret picks the model that compares), **Stop all**
  while one works and **Continue with this answer** (opens the new-thread
  stage with the quoted answer; nothing is sent). Pane headers read "2 · Opus
  · High". On a phone the strip is a one-tap switcher and one **Compare ▾**
  menu. The thread's ⋯ holds **Open side by side**, **Ask every model…** and
  **Compare answers with ▸** (`runtime/PaneStrip.svelte`,
  `runtime/paneStrip.ts`, `runtime/councilStore.svelte.ts`).

## Approvals

- **Attention dock.** What the agent waits on — an approval, a question form
  or a secret request — sits in one dock directly above the composer
  (`runtime/RuntimeAttentionDock.svelte`, model `runtime/attentionDock.ts`),
  with its live controls; the timeline row only records the request
  ("Waiting for your approval · Answer", which jumps to the dock). Several
  requests step with ‹ / › ("1 of 3"). From an empty composer **⌘↵** allows once
  and **⌘⌫** denies. The dock is capped to a share of its pane (45 % with a
  mouse, 60 % on touch) and never pushes the composer off screen.
- **The buttons** are the same everywhere — a provider's approval, a
  subagent's ("Approval needed from *title*", answered in the parent),
  an MCP server's ask, an ACP agent's: **Allow once**, **Always allow**,
  **Deny**, **Always deny** (the "always" ones only when offered), anything
  else under More (`runtime/approvalOptions.ts`). The timeline records the
  outcome (Denied, Withdrawn by Claude, Expired).
- **In the inbox** any of them makes the thread **Approval** and lifts it out
  of Working (see *Inbox*).

**Questions.** A supported provider question appears in the attention dock under **Needs your answer**, separately
from permission approvals. Choose one option, several options, or type an
answer according to the form. **Other** (with a text box) appears where free text
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

**Secrets.** An agent that needs a token or a password calls `otto.request_secret`
(`otto_request_secret` in a session; on by default). A card appears above
the thread's composer, for the thread's owner only: **The agent asks for a
secret: …**, a password field, **Send** and **Decline** (a refusal to share, not a permission decision). The value goes only
into that tool call's result. It is never put in the composer, a draft, the
timeline or Otto's database: the request lives in the daemon's memory for up
to about 3 minutes, and the thread masks the value as `••••••` in everything
it shows or stores afterwards (values shorter than 4 characters are not
masked). The agent CLI's own transcript on disk still records the tool
result, as it does for any tool. Threads only (T3 Code #15907, #30257).

## Details card and git

- **Thread details** (`ThreadDetailsCard`): one card for the thread's
  facts — the folder with **Open in ▾** (Files, Git, a terminal thread, copy
  path, and the folder's **project scripts**: run one, add one, remove
  one), the branch ▾, **one git action** for the checkout's state
  (**Commit** → **Push** / **Publish branch** → **Create PR** → **PR #N**;
  the chevron beside it holds all three; push asks where it goes first and
  Create PR uses the Git sheet that names the destination; the row hides
  when there is nothing to do — no changes, nothing to push, no PR to
  create), **Changes +N −M** ("No changes" at zero;
  opens Changes; "N files" when the changes carry no line counts), then **Subagents** ("Subagents · 1 needs you · 1 working"): the thread it works under
  first ("↑ parent") when it is itself a subagent, each running subagent
  with its agent's glyph, status word (Working · Needs you · Done · Failed ·
  Stopped), elapsed time and **Stop**, and the rest under **Previous · N**. A
  row with its own thread opens it (on a phone the drawer gives way);
  a turn's own subagent opens its row. Background commands and
  timers stay in the right panel's Background tasks. It works the same for
  terminal threads (folder, git, their subagents).
- It **floats top-right** of the conversation while the right panel is
  closed and one conversation (or the New thread stage) has a whole
  reading column left beside it; the conversation moves over
  (`--thread-card-reserve` on the stage), so the card never covers a
  message. Anywhere else (a split, a narrower window, the right panel open,
  a phone or tablet) the same card heads the right panel / details drawer.
  Its **×** hides the floating card on this device; ⋯ › View › **Thread
  details** brings it back.
- **Ready-to-work worktree** (`otto-git/src/worktree_ready.rs`,
  `worktreeReady.ts`): a `.worktreeinclude` at the repo root names, in
  gitignore patterns, the untracked or ignored files a new worktree gets.
  They are copied from the MAIN checkout (`.env`, local config; at most
  100 MB in all, the rest is named in a toast with why); the patterns after a
  `[clone]` line (dependency folders) are cloned copy-on-write (`cp -c`,
  APFS clonefile) or, only when the disk cannot clone, symlinked to the main
  checkout (the toast says which). Git itself matches the patterns
  (`ls-files --others --ignored --directory`), so nothing outside the
  checkout and no symlink is ever followed. Example:

  ```
  .env
  config/local.json
  [clone]
  node_modules/
  ```

  A repository's **worktree setup** (Details › Open in › **Worktree setup**:
  one of the project's scripts or another command) is kept on the daemon
  (setting `worktree_setup`, by main checkout), so a worktree made from the
  phone runs it too. After the worktree is made the progress line reads
  *Setting up: npm ci · <its latest output line>* (streamed, at most once a
  second); a failure shows inline (*Setup "npm ci" failed (exit
  1): …*) with **Retry** (runs only the setup again) and **Start without
  setup**; the message is never lost. It runs before any provider starts, so
  it is the same for every agent.

**The right panel** (`ThreadInspector`; the launcher's letters open each
section):

- **Background tasks** (structured threads, hidden when there are none):
  the thread's commands and timers — running first with their elapsed
  time, the last finished under one "N finished" line
  (`runtime/agentTasks.ts`, `ThreadAgentsTasks.svelte`). Subagents are not
  listed here: they are the timeline's subagent row, the waiting line, the
  attention dock and the details' Subagents section (see
  threads-structured-runtime.md "Threads inside a thread"). The strip
  above the composer names one background task or counts several ("2
  background tasks"); tapping it opens this section.
- **Subagent** tab: a subagent opened from its row while the parent stays
  put (⌘-click: the main view instead).
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

## Review notes

In **Changes** (`ThreadChangesTab.svelte`, `runtime/reviewNotes.ts`,
`DiffNote*.svelte`, `DiffReviewBar.svelte`):

- In Changes (uncommitted, the unpushed range or one commit), a line number
  starts a note; ⇧-click another line in the same hunk for a range.
- Notes show under their last line and in one **Review · N notes** bar,
  with **Send to agent** and **Clear**, and Edit / Resolve / Delete on
  each note.
- **Send to agent** puts every draft note into the focused thread's
  message as one context item (file, new / old lines, quoted lines,
  comment). Add a sentence, or press Enter to send as is.
- After sending, notes read **Sent** and can be resolved. A note whose
  lines are gone from its file's diff (the agent changed them) reads
  **Outdated**; it is not removed.
- With nothing typed, the note box can still add the lines as plain
  context.
- Notes are kept per thread on this device (localStorage, newest 50
  threads × 200 notes). The daemon draft's context items have no field
  for a note's state.
- On a phone the note box is a sheet above the keyboard.

## MCP and ACP

- **⋯ › MCP servers** (`runtime/RuntimeMcpServers.svelte`) lists every
  server the thread's agent has — Otto's tools, this workspace's servers, the
  agent's own config — each with a word (Connected, Added, Needs sign-in,
  Failed, Disabled, Not supported) and why when it is not connected.
- **A governed tool call holds for you** (`otto-mcp` `approval_wait`): it
  asks in the attention dock and goes on as soon as you answer; nothing has to
  be retried. Otto's reversible tools also offer **Always allow in this
  workspace**. An MCP server's question shows in the same place.
- Turning a server on or off while the agent runs shows **MCP servers
  changed · Restart agent to apply**; the conversation continues.
- **ACP agents** (Gemini, Antigravity, Grok, opencode, Muse) answer
  session-scoped `elicitation/create` forms with flat text and string-choice
  fields (see *Approvals › Questions*), report their context window through
  `usage_update`, and only queue mid-turn messages unless they have the
  `steer` capability (Grok does).

## Phone and tablet

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

## Settings

**Settings › Threads** (General, this device; `ThreadsSettings.svelte`):
  - While the agent works, Enter: Queue / Steer.
  - Tool calls: Show all / Hide successful / Hide all.
  - Open a subagent in: side panel / main view (`threadPrefs.subagentIn`,
    read by the subagent views).
  - Group the sidebar by project.
  - Show the thread details card.

  The toolbar ⋯ › View switches the same prefs.

## Shortcuts

| Keys | Action |
|---|---|
| `⌘T` | New thread (instead of a terminal session) |
| `⌥⌘↓` / `⌥⌘↑` | Next / previous thread in the list |
| `⌥⌘N` | Open the next thread that needs you |
| `⌘K` | Threads first: `↵` opens one, `⌘↵` opens it beside the current one |
| `F2` | Rename the thread (or double-click its title) |
| `⌘Z` | Undo the last pin, Move to History or side-chat dismiss (outside a text field) |
| `↵` / `⌘↵` | Send; while the agent works, the default delivery (Queue or Steer) / the other one |
| `Esc` / `⌘.` | Stop the agent (`Esc` twice when the composer has text) |
| `⌘↵` / `⌘⌫` | With an approval waiting and the composer empty: Allow once / Deny |
| `⇧⇥` | Turn Plan mode on or off |
| `↑` / `↓` | In an empty composer: earlier / later prompts you sent in this thread |
| `⌥↑` / `⌥↓` | In a long thread: previous / next message of yours (not while typing) |
| `⌘F` | Find in the thread, older loaded messages included |

The same rows are in the **Threads** table of the keyboard-shortcuts guide
and the `?` sheet.

## API and WebSocket surface

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
