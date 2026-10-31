---
id: threads
title: Threads
group: Work
route: threads
summary: Run Claude Code, Codex and Antigravity side-by-side in an inbox-style textual workspace with pins, history, multi-session splits, peer relays and a workspace inspector.
---

## What it's for

Threads (Experimental) is a conversation-first workspace for coding agents. It keeps your existing terminal-first **Agents** page intact while giving you a calm, readable surface built around threads, pins, project folders, multi-session split views, live file and diff inspection, and direct session-to-session messaging.

## Getting started

1. Open **Threads** in the **Work** group of the sidebar (or press `⌘K` and run **Go to Threads**).
2. Click **+ New Thread** to open the welcome stage, pick **Claude Code**, **Codex**, **Antigravity** or any custom provider, choose your repository folder, and type your opening prompt.
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
- Switch the header segmented control between **1-up**, **Split** (2-up resizable) and **Grid** (up to 4 live threads at once).
- Use the **Peers** inspector tab (or a pane's **Send to peer…** bar) to relay a message from one session to another or broadcast across live threads, watch the live cross-session timeline, and start a **Session Room**.

**Workspace inspector**
- **Files**: browse the working tree and open multiple file tabs (`Preview` for Markdown, JSON, CSV/TSV and images, or `Source` with syntax highlighting).
- **Changes**: inspect working-tree git status with `+added / -removed` counts and inline `Unified` or `Split` diffs.
- **Connections**: watch live agent MCP database queries (`otto_db_query` / `otto_query_db`) with SQL, row counts and previews, or click **Open side-by-side** to dock the Database Explorer next to your thread.
