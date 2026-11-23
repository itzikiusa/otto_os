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
- **Pinned**: keep key threads at the top across restarts.
- **Active**: live sessions with status dots, **Needs approval** badges, git branch chips, and indented delegated workers (`delegated_by`).
- **Projects**: folders grouped by repository/working directory with one-click **+** to start a thread in that repo.
- **History**: settled and archived conversations you can search, reopen, or view beside an active thread.

**Rich conversation & composer**
- Full GitHub-flavored Markdown, syntax-highlighted code blocks, inline workspace/local image thumbnails with zoomable lightbox, and expandable **Thinking** blocks (including full reasoning prose on Antigravity sessions).
- **Inline approval dock** right above the composer when an agent pauses on a tool permission (`Approve`, `Always allow`, `Deny`, or custom reply).
- Type `@` in the composer to attach workspace files with keyboard-navigable typeahead, paste or drop screenshots, and choose **Steer** (`⇧⌘↵`) or **Queue** (`↵`) while an agent is working.
- Slide up the optional **Terminal** drawer on any thread pane whenever you want raw PTY access without leaving Threads.

**Multi-session & peer collaboration**
- Pick the layout from the toolbar ⋯: **Single thread**, **Split** (2-up resizable) or **Grid** (up to 4 live threads at once).
- ⋯ › **Send to another thread…** lists the threads this one is connected to with their status and the thread's session rooms; each row opens, opens beside or messages. **Message a thread…** sends to any thread (or all of them), and **Start a room…** starts a **Session Room**.

**Ask another agent from a thread**
- In a thread's composer, **Ask another agent** (the people icon beside Attach) opens the agent menu. Each model you pick becomes a chip above the text; you can also type `@` and part of a provider or model name (`@cod`, `@opus`).
- With chips set, **Send** asks those agents instead of this thread's agent. Each one gets its own new thread, linked to this one and in the same folder. Pick up to 4.
- The chip row says what they may do. They get this thread's access or a narrower one you pick, never wider.
- The parent shows a live card per child: status, elapsed time, its latest step, and its answer's first line when it ends, with **Stop**, **Open beside** and **Open**.
- The sidebar nests child threads under their parent. Fold them with the parent row's chevron.

**Council: one prompt, several agents**
- In the new-thread stage, turn on **Ask several** beside the agent chip, then pick 2–4 models. Each one is a seat; the send button reads e.g. **Ask 3**.
- The seats open side by side (Split for 2, Grid for 3–4). Each one is a normal thread with its own composer: to follow up with one agent, type in its thread. Every seat is read-only, so agents working in one folder never clash on files. ⌘↵ starts them in the background and a toast offers **Show**.
- The sidebar nests the seats under seat 1. On a phone, the thread details (the right-panel button) list the other seats; tap one to switch.
- The thread's **⋯** menu holds the rest: **Open council side by side**, **Ask every seat…** (the same follow-up to each seat; a busy seat queues it) and **Synthesize with ▸** a judge (seat 1's agent first). The judge is one more read-only thread, opened beside the seats, that compares each seat's latest answer: where they agree, where they differ, and a pick or a merge.

**Workspace inspector** (the right panel: Browser, Files, Diff, Pull request, Side chat)
- **Files**: browse the working tree and open multiple file tabs (`Preview` for Markdown, JSON, CSV/TSV and images, or `Source` with syntax highlighting).
- **Diff**: inspect working-tree git status with `+added / -removed` counts and inline `Unified` or `Split` diffs.
- **Connections** (added once the thread has database activity): watch live agent MCP database queries (`otto_db_query` / `otto_query_db`) with SQL, row counts and previews, or click **Open side-by-side** to dock the Database Explorer next to your thread.
