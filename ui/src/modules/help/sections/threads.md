---
id: threads
title: Threads
group: Work
route: threads
summary: Run Claude Code, Codex, Gemini and other agents as conversations in an inbox — approvals, subagents, compared models, git and review notes in one place.
---

## What it's for

Threads (Experimental) is a conversation-first workspace for coding agents. It sits beside the terminal-first **Agents** page: the same agents, read as chats, sorted like an inbox, with the thread's folder, branch, changes and subagents one glance away.

## What changed in this release

- The sidebar is an **inbox**: each row says what it waits on in a word, and a thread waiting for your approval never hides under **Working**.
- One set of words everywhere: subagents are **Working · Needs you · Done · Failed · Stopped**, compared models are "**2 compared**", approvals are **Allow once · Always allow · Deny · Always deny**.
- A **Thread details** card with the one git step that comes next (hidden when there is nothing to do).
- A calmer composer with one button, **Delegate to…**, the agent's own slash commands, prompt history and **Continue with another provider**.
- A **context meter**, **Resume at reset** after a usage limit, ready-to-work worktrees, **review notes** on the diff and a **minimap**.
- An MCP tool that needs your approval waits for your answer instead of failing.

## Getting started

1. Open **Threads** in the **Work** group of the sidebar (or press `⌘K` and run **Go to Threads**).
2. Click **New thread**, pick the agent and model on the agent chip, check the folder, and type your opening prompt. To keep the work apart, pick **New worktree** in the checkout chip: when you send, Otto names a branch from your message, makes the worktree and starts the thread in it.
3. Press `⌘K` in Threads: it opens on your threads, what needs you first. `↵` opens one, `⌘↵` opens it beside the current one.
4. `⌘`-click a thread (or choose **Open beside**) to read up to 4 threads side by side in **Split** or **Grid**.

## Everything it can do

**Inbox**
- **An inbox**: one list across projects. Threads that need you come first, then failed ones, then finished ones you have not opened (**Done**), then the most recent.
- **Each row says it in a word**: **Approval**, **Question**, **Failed**, **Done** or **Working 18s**, beside the project. Under the title: the branch (or the folder) and the agent.
- A thread with anything waiting on you — its agent's approval, an MCP server's question or a subagent's approval — reads **Approval** or **Question** and moves out of **Working** straight away.
- **Pinned** keeps key threads on top. **Working** holds threads that are busy and not waiting on you. **History** keeps threads you moved there and closed ones.
- **Group by project** (the filter menu) shows a folder per project instead. Otto remembers it on this device.
- **Undo**: pinning, moving to History and dismissing a side chat happen at once. The notice offers **Undo** for 5 seconds, or press **⌘Z** outside a text field.
- **Usage meter**: the footer shows how much of the focused agent's tightest plan limit is used (for example **5h 62%**). It turns amber at 80 % and red at 95 %. Hover it to see every limit and when it resets; click it to open Usage.
- Pins, History and what you have seen follow you: pin on the Mac and the phone shows it pinned straight away.
- Subagents fold under their thread ("2 subagents", or "2 compared" for compared models); when one needs you, the thread's word becomes **Needs you**.

**Thread**
- The top bar: the thread list, the project and title (double-click or `F2` renames), the right panel and ⋯ (rename, pin, search, **Send to another thread…**, Continue on phone, Move to History, close, **View**).
- Full GitHub-flavored Markdown, syntax-highlighted code blocks, inline workspace/local image thumbnails with zoomable lightbox, and expandable **Thinking** blocks (including full reasoning prose on Antigravity sessions).
- The conversation reads as one centred column in reading-size type: your message on the right, the agent's answer as plain prose. A finished turn's steps fold into one dim line ("Worked for 2m 13s · ran 4 commands and edited 3 files"); while a turn runs, its earlier steps fold to "N earlier steps" and the live one stays in view. Failures, approvals and questions never fold. A message's time and **Copy** show when you point at it.
- A turn that changed files ends with a card: "3 changed files +6 −2". Open it for the file list with each file's +/−; **Open diff** (or a file) opens the **Changes** panel on that file.
- Select text in an answer and **Quote** adds it to your message. `⌘F` finds in the whole thread.
- **Minimap**: a long thread (6+ of your messages) has a rail of ticks at the column's edge, one per message you sent. Click to jump, `⌥↑` / `⌥↓` to step; on a phone, ⋯ › **Jump to…**.
- The toolbar ⋯ › **View** › **Tool calls**: *show all* (default), *hide successful* or *hide all*. Failures, approvals, questions and replies always show.
- **Split** and **Grid** show 2–4 threads with a strip of their states and **Message every pane…**.
- The optional **Terminal** drawer gives raw PTY access; select text in it and **Add to chat** attaches it to your message (nothing is sent).

**Composer**
- **The composer**: one card with **+** (attach an image or PDF, mention a file or agent, **Plan mode**, the thread's access), then only what differs from the default (the Plan chip while Plan is on, the access chip when the thread is not on your default access), the model chip and one button that says what it will do: **Send**; **Stop** while the agent works and nothing is typed; **Queue** (sends when the turn ends) or **Steer** (the agent reads it at its next step) when you type while it works. **↵** does the default and **⌘↵** the other; the default is in the toolbar ⋯ › **View** › *While working, messages queue / steer*. An agent that cannot take a message mid-turn only queues, and the button says so.
- Type `/` for Otto's commands and, under "<Agent> commands", the agent's own commands and skills; picking one types `/name ` for you to finish.
- In an empty composer **↑** brings back the prompts you sent in this thread (**↓** goes forward).
- Type `@` to attach workspace files; paste or drop images and PDFs anywhere on the thread (a type the agent cannot take is refused by name).
- Your unsent message, attachments and choices are saved per thread and come back after a reload or restart. `/clear` makes the next message start a fresh conversation in the same thread.
- **Context meter**: the small ring beside the model chip shows how full the agent's context window is (the percent appears from 80 %). Click it for the token count and, when the agent has one, **Compact the conversation**. It only appears once the agent has reported its window.
- **Usage limit**: a thread whose agent hit its usage limit reads **Limited**. The line above the composer says when the limit resets and offers **Resume at reset** (Otto sends a short "continue" at that time, once, even after a restart; **Cancel** undoes it) or **Continue with** another agent. When the reset time is unknown, only the switch is offered.

**Subagents and other providers**
- Just ask: "do this with a Codex subagent and a Gemini subagent". The thread's agent checks which providers and models are enabled, delegates, and is woken when each one finishes.
- Or delegate yourself: **Delegate to…** (the people icon, or type `@cod`, `@opus`) adds provider chips; Send opens one subagent each. Their answers go to the thread's agent — tick **Answer to me only** to keep them with you.
- A turn's subagents are one row in the timeline: provider icons, "3 subagents · 2 working", elapsed. Open it for one row each; a row opens its full answer.
- Under the timeline: "Waiting on 2 subagents · <titles> · **Stop**". **Stop** (here or the composer's) stops the thread and all its subagents.
- **Open** puts a subagent in a tab on the right while the thread stays put; ⌘-click opens it in the main view. Settings ▸ Threads makes the main view the default on this device. Inside a subagent, the first line says whose subagent it is, with **Open parent**.
- **Continue with another provider**: the model chip lists the thread's provider first, then the others. Pick a model of another one and the line above the composer reads **Next message continues on Codex · Cancel**; nothing is sent yet. Your next message continues the same thread there, after a **Context handoff · Claude Code → Codex** divider; open it to see what was handed over (recent turns, files changed, an open plan; earlier images are not). Switching back is another handoff.
- **Compare models**: in the new-thread stage turn on **Compare models** beside the agent chip and pick 2–4 models; the send button reads e.g. **Ask 3**. Each model gets its own read-only thread, side by side. The strip (or the thread's ⋯) has **Ask every model…**, **Compare answers** and **Continue with this answer**.

**Approvals**
- What the agent waits on — an approval, a question, a secret — sits in one dock right above the composer. Approvals always offer **Allow once**, **Always allow**, **Deny** and **Always deny** (the "always" ones when the agent offers them), the same for a subagent's ("Approval needed from …") and an MCP server's request. From an empty composer `⌘↵` allows once and `⌘⌫` denies.
- A question form has its options, **Other** for your own words, **Send answer** and **Skip question**.
- An agent that needs a token asks for it privately: the value goes only to that tool call and is masked everywhere after.

**Details card and git**
- One card holds the thread's facts: its folder (**Open in ▾**: Files, Git, a terminal, copy path, project scripts), its branch, the one git step that comes next (**Commit**, **Push**, **Create PR** or **PR #N**; hidden when there is nothing to do), **Changes +N −M**, and its **Subagents** — running ones with their status, time and **Stop**, earlier ones under **Previous**, and the thread it works under when it is a subagent itself.
- On a wide window it floats at the top right beside the conversation (which moves over, so nothing is covered). Otherwise it is at the top of the right panel; on a phone, tap the right-panel button. Hide the floating card with its **×**; ⋯ › **View** › **Thread details** brings it back.
- **New worktree, ready to work**: put a `.worktreeinclude` in the repo root listing files to copy into every new worktree (`.env`), and after a `[clone]` line the folders to clone instantly (`node_modules/`). **Worktree setup** in the details card's Open in menu picks a command (`npm ci`) that runs in each new worktree before the agent starts; if it fails, **Retry** or **Start without setup**.
- The right panel also has **Files**, **Changes** (working tree, or the commits a push would send), **Pull request** (review conversations, **Add to chat**), **Side chat** (ask about the thread without interrupting it), **Background tasks** and, once the thread queries a database, **Connections**.

**Review notes**
- **Review notes**: in **Changes**, click a line number (⇧-click another line for a range) to write a note for the agent. Notes show under their lines and in the **Review · N notes** bar; **Send to agent** puts them all in the thread's message as one item (add a sentence or press Enter). Sent notes can be resolved; a note whose lines the agent changed reads **Outdated**.

**MCP and ACP**
- **⋯ → MCP servers** lists every server the thread's agent has (Otto's tools, this workspace's servers, the agent's own config), each with a word: Connected, Added, Needs sign-in, Failed, Disabled or Not supported, and why when it isn't connected.
- When the agent calls a tool that needs your approval, it asks right above the composer: **Allow once**, **Deny**, or (for Otto's reversible tools) **Always allow in this workspace**. The call goes on as soon as you answer; nothing has to be retried.
- An MCP server that asks you something shows its question in the same place.
- Turned a server on or off while the agent runs? The thread says **MCP servers changed** with **Restart agent to apply**; the conversation continues.
- ACP agents (Gemini, Antigravity, Grok, opencode, Muse) answer simple question forms and report their context window; most of them queue a message you send mid-turn instead of steering, and the button says so.

**Phone**
- One header row: the thread list, the title, the right panel and ⋯. The composer stays above the on-screen keyboard and folds what you don't need while you type.
- Code blocks wrap; the details card and the right panel open from the right-panel button; long-press a row for its actions.

**Settings**
- **Settings › Threads** holds this device's Threads choices (Queue / Steer, tool calls, where a subagent opens, sidebar grouping, the details card). The toolbar ⋯ › **View** switches the same choices.

## Keyboard shortcuts

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

## Related

- [Agents](#/walkthroughs/agents)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
- [Phone and remote](#/walkthroughs/phone-and-remote)
