---
id: threads
title: Threads
group: Work
route: threads
summary: Run Claude Code, Codex and Antigravity side-by-side in an inbox-style textual workspace with pins, history, multi-session splits, related threads, councils and a workspace inspector.
---

## What it's for

Threads (Experimental) is a conversation-first workspace for coding agents. It keeps your existing terminal-first **Agents** page intact while giving you a calm, readable surface built around threads, pins, project folders, multi-session split views, live file and diff inspection, and direct session-to-session messaging.

## Getting started

1. Open **Threads** in the **Work** group of the sidebar (or press `⌘K` and run **Go to Threads**).
2. Click **New thread** (in the sidebar or the toolbar ⋯) to open the welcome stage, pick **Claude Code**, **Codex**, **Antigravity** or any custom provider, choose your repository folder, and type your opening prompt. To keep the work apart, pick **New worktree** in the checkout chip under the composer: nothing is asked — when you send, Otto names a branch from your message (under the repo's usual prefix, like `feat/`), makes the worktree and starts the thread in it. **Name the branch…** in the same menu picks the name yourself. If making it fails, your message stays and **Retry** tries again.
3. Press `⌘K` in Threads: it opens on your threads — what needs you first (**Approval**, **Question**, **Failed**), then **Done**, then **Working**, then the rest. Type to filter threads and commands together; **New thread: …** starts one with what you typed. `↵` opens a thread, `⌘↵` opens it beside the current one in a split. Elsewhere in Otto, `⌘K` shows **Threads needing you** while any wait. On a phone it is **More › Commands**.
4. Click any thread in the left rail to read its live conversation, or `⌘`-click (or choose **Open beside**) to compare up to 4 threads side-by-side in **Split** or **Grid** mode.

## Everything it can do

**Thread details**
- One card holds the thread's facts: its folder (**Open in ▾**: Files, Git, a terminal, copy path, project scripts), its branch, the one git step that comes next (**Commit**, **Push**, **Create PR** or **PR #N**), **Changes +N −M**, and its **Subagents** — running ones with their status, time and **Stop**, earlier ones under **Previous**, and the thread it works under when it is a subagent itself.
- On a wide window it floats at the top right beside the conversation (which moves over, so nothing is covered). Otherwise it is at the top of the right panel; on a phone, tap the right-panel button. Hide the floating card with its **×**; ⋯ › **View** › **Thread details** brings it back.

**Thread rail**
- **An inbox**: one list across projects. Threads that need you come first, then failed ones, then finished ones you have not opened (**Done**), then the most recent.
- **Each row says it in a word**: **Approval**, **Question**, **Failed**, **Done** or **Working 18s**, beside the project. Under the title: the branch (or the folder) and the agent.
- **Pinned** keeps key threads on top. **Working** holds threads that are busy and not waiting on you. **History** keeps threads you moved there and closed ones.
- **Group by project** (the filter menu) shows a folder per project instead. Otto remembers it on this device.
- **Undo**: pinning, moving to History and dismissing a side chat happen at once. The notice offers **Undo** for 5 seconds, or press **⌘Z** outside a text field.
- **Usage meter**: the footer shows how much of the focused agent's tightest plan limit is used (for example **5h 62%**). It turns amber at 80 % and red at 95 %. Hover it to see every limit and when it resets; click it to open Usage.
- Pins, History and what you have seen follow you: pin on the Mac and the phone shows it pinned straight away.

**Rich conversation & composer**
- Full GitHub-flavored Markdown, syntax-highlighted code blocks, inline workspace/local image thumbnails with zoomable lightbox, and expandable **Thinking** blocks (including full reasoning prose on Antigravity sessions).
- **Inline approval dock** right above the composer when an agent pauses on a tool permission (`Approve`, `Always allow`, `Deny`, or custom reply).
- The conversation reads as one centred column in reading-size type: your message on the right, the agent's answer as plain prose. A finished turn's steps fold into one dim line ("Worked for 2m 13s · ran 4 commands and edited 3 files"); while a turn runs, its earlier steps fold to "N earlier steps" and the live one stays in view. Failures, approvals and questions never fold. A message's time and **Copy** show when you point at it.
- A turn that changed files ends with a card: "3 changed files +6 −2". Open it for the file list with each file's +/−; **Open diff** (or a file) opens the **Changes** panel on that file.
- **The composer**: one card with **+** (attach an image or PDF, mention a file or agent, **Plan mode**, the thread's access), then only what differs from the default (the Plan chip while Plan is on, the access chip when the thread is not on your default access), the model chip and one button that says what it will do: **Send**; **Stop** while the agent works and nothing is typed; **Queue** (sends when the turn ends) or **Steer** (the agent reads it at its next step) when you type while it works. **↵** does the default and **⌘↵** the other; the default is in the toolbar ⋯ › **View** › *While working, messages queue / steer*. An agent that cannot take a message mid-turn only queues, and the button says so.
- **Context meter**: the small ring beside the model chip shows how full the agent's context window is (the percent appears from 80 %). Click it for the token count and, when the agent has one, **Compact the conversation**. It only appears once the agent has reported its window.
- **Continue with another provider**: the model chip lists the thread's provider first, then the others. Pick a model of another one and the line above the composer reads **Next message continues on Codex · Cancel**; nothing is sent yet. Your next message continues the same thread there, after a **Context handoff · Claude Code → Codex** divider; open it to see what was handed over (recent turns, files changed, an open plan; earlier images are not). Switching back is another handoff.
- Type `/` for Otto's commands and, under "<Agent> commands", the agent's own commands and skills (fuzzy filtered); picking one types `/name ` for you to finish and send.
- In an empty composer **↑** brings back the prompts you sent in this thread (**↓** goes forward, then back to empty).
- Type `@` to attach workspace files or ask another agent; paste or drop images and PDFs anywhere on the thread (a type the agent cannot take is refused by name).
- **Review notes**: in **Changes**, click a line number (⇧-click another line for a range) to write a note for the agent. Notes show under their lines and in the **Review · N notes** bar; **Send to agent** puts them all in the thread's message as one item (add a sentence or press Enter). Sent notes can be resolved; a note whose lines the agent changed reads **Outdated**.
- **Minimap**: a long thread (6+ of your messages) has a rail of ticks at the column's edge, one per message you sent (failed and approval turns marked). Click to jump, ⌥↑ / ⌥↓ to step; on a phone, ⋯ › **Jump to…**.
- **Settings › Threads** holds this device's Threads choices (Queue / Steer, tool calls, where a subagent opens, sidebar grouping, the details card).
- The toolbar ⋯ › **View** › **Tool calls**: *show all* (default), *hide successful* or *hide all*. Failures, approvals, questions and replies always show.
- Slide up the optional **Terminal** drawer on any thread pane whenever you want raw PTY access without leaving Threads. Select text in it and **Add to chat** attaches the selection to the thread's message as a "Terminal · N lines" chip (nothing is sent).

**Multi-session & peer collaboration**
- Pick the layout from the toolbar ⋯: **Single thread**, **Split** (2-up resizable) or **Grid** (up to 4 live threads at once).
- ⋯ › **Send to another thread…** lists the threads this one is connected to with their status and the thread's session rooms; each row opens, opens beside or messages. **Message a thread…** sends to any thread (or all of them), and **Start a room…** starts a **Session Room**.

**Ask another agent from a thread**
- In a thread's composer, **Ask another agent** (the people icon beside **+**) opens the agent menu. Each model you pick becomes a chip above the text; you can also type `@` and part of a provider or model name (`@cod`, `@opus`).
- With chips set, **Send** asks those agents instead of this thread's agent. Each one gets its own new thread, linked to this one and in the same folder. Pick up to 4.
- The chip row says what they may do. They get this thread's access or a narrower one you pick, never wider.
- The parent shows a live card per child: status, elapsed time, its latest step, and its answer's first line when it ends, with **Stop**, **Open beside** and **Open**.
- The sidebar folds these subagents under the thread that started them; the row says how many. When one needs you, the thread says **Needs you**. Open them with the row's chevron.

**Council: one prompt, several agents**
- In the new-thread stage, turn on **Ask several** beside the agent chip, then pick 2–4 models. Each one is a seat; the send button reads e.g. **Ask 3**.
- The seats open side by side (Split for 2, Grid for 3–4). Each one is a normal thread with its own composer: to follow up with one agent, type in its thread. Every seat is read-only, so agents working in one folder never clash on files. ⌘↵ starts them in the background and a toast offers **Show**.
- The sidebar folds the seats under seat 1 (open them with its chevron). On a phone, the thread details (the right-panel button) list the other seats; tap one to switch.
- The thread's **⋯** menu holds the rest: **Open council side by side**, **Ask every seat…** (the same follow-up to each seat; a busy seat queues it) and **Synthesize with ▸** a judge (seat 1's agent first). The judge is one more read-only thread, opened beside the seats, that compares each seat's latest answer: where they agree, where they differ, and a pick or a merge.

**Workspace inspector** (the right panel: Browser, Files, Diff, Pull request, Side chat)
- **Files**: browse the working tree and open multiple file tabs (`Preview` for Markdown, JSON, CSV/TSV and images, or `Source` with syntax highlighting).
- **Diff**: inspect working-tree git status with `+added / -removed` counts and inline `Unified` or `Split` diffs.
- **Connections** (added once the thread has database activity): watch live agent MCP database queries (`otto_db_query` / `otto_query_db`) with SQL, row counts and previews, or click **Open side-by-side** to dock the Database Explorer next to your thread.
