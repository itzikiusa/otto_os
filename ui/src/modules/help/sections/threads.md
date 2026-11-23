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
2. Click **New thread** (in the sidebar or the toolbar ⋯) to open the welcome stage, pick **Claude Code**, **Codex**, **Antigravity** or any custom provider, choose your repository folder, and type your opening prompt.
3. Click any thread in the left rail to read its live conversation, or `⌘`-click (or choose **Open beside**) to compare up to 4 threads side-by-side in **Split** or **Grid** mode.

## Everything it can do

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
- The **Related threads** chip beside the title appears when a thread has related threads (a parent, workers, threads it opened, its side chat). Its popover lists them with their status and the thread's session rooms; each row opens, opens beside or messages. **Message a thread…** sends to any thread (or all of them), and **Start a room…** starts a **Session Room**. It replaces the old Peers tab and the pane's relay bar.

**Subagents: several agents and providers under one thread**
- Just ask: "do this with a Codex subagent and a Gemini subagent". The thread's agent checks which providers and models are enabled, delegates, and is woken when each one finishes. It never polls.
- Or delegate yourself: **Delegate to…** (the people icon beside Attach, or type `@cod`, `@opus`) adds provider chips; Send opens one subagent each. They get your message and the conversation so far, and their answers go to the thread's agent — tick **Answer to me only** to keep them with you.
- A turn's subagents are one row in the timeline: provider icons, "3 subagents · 2 working", elapsed. Open it for one row each; a row opens its full answer.
- Under the timeline: "Waiting on 2 subagents · <titles> · **Stop**". **Stop** (here or the composer's) stops the thread and all its subagents.
- A subagent's approval shows above your composer as "Approval needed from <title>" — answer it right there.
- **Open** puts a subagent in a tab on the right while the thread stays put; ⌘-click opens it in the main view. A checkbox in that tab makes the main view the default on this device. Inside a subagent, the first line says whose subagent it is, with **Open parent**.
- Statuses are always Working, Needs you, Done, Failed or Stopped.

**Compare models: one prompt, several models**
- In the new-thread stage, turn on **Compare models** beside the agent chip, then pick 2–4 models; the send button reads e.g. **Ask 3**.
- They open side by side (Split for 2, Grid for 3–4). Each is a normal, read-only thread with its own composer. ⌘↵ starts them in the background.
- The thread's **⋯** menu holds the rest: **Open side by side**, **Ask every model…** and **Compare answers with ▸** a judge, which compares each model's latest answer.

**Workspace inspector** (the right panel: Browser, Files, Diff, Pull request, Side chat)
- **Files**: browse the working tree and open multiple file tabs (`Preview` for Markdown, JSON, CSV/TSV and images, or `Source` with syntax highlighting).
- **Diff**: inspect working-tree git status with `+added / -removed` counts and inline `Unified` or `Split` diffs.
- **Connections** (added once the thread has database activity): watch live agent MCP database queries (`otto_db_query` / `otto_query_db`) with SQL, row counts and previews, or click **Open side-by-side** to dock the Database Explorer next to your thread.
