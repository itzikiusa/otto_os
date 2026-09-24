---
id: history
title: History
group: Work
route: history
summary: Browse every past Claude and Codex conversation — Otto sessions and transcripts found on disk — read it, and resume it in Otto.
---

## What it's for

History is the archive of your agent conversations. It lists every Claude and Codex conversation Otto knows about: its own sessions (running, idle, exited, suspended and archived) plus transcripts already on disk that no Otto session owns, such as ones you ran in a normal terminal. Read any of them as a chat, then resume it in Otto to keep going.

## Getting started

1. Open **History** from the sidebar, the clock button in the Agents tab bar, or the **History** chip in the Work Queue.
2. The page opens on your last-read conversation, or the newest one. Conversations are grouped by repository or folder.
3. Narrow the list with search and the filters: workspace, provider, folder, status and date.
4. Select a row to read it on the right.
5. Press **Resume in Otto** (or **Open in Otto** if it's still running). Otto opens it as a session in the Chat view.

## Everything it can do

**Finding a conversation**
- Search titles and first prompts (results update as you type).
- Scope: the current workspace or **No workspace** (sessions started without one).
- Filters: provider (Claude, Codex), folder, status (Running, Idle, Exited, Resumable, On disk only) and date (Today, Last 7 days, Last 30 days).
- Groups by repository or working folder, with a count per group. Click a group header to collapse it.
- Each row shows the title (the provider's AI title, else the first prompt), status, last activity and number of turns. Rows marked **on disk** are transcripts no Otto session owns.
- The list loads 100 at a time; **Load more** fetches older conversations.
- **Rescan transcripts** re-indexes `~/.claude/projects` and `~/.codex/sessions`, with a progress bar while it runs.

**Reading**
- The conversation is shown read-only in the same chat view as Agents: turns, tool steps with diff stats, sub-agents, task lists, images and artifacts, plus cost, token and duration stats.
- Search inside the conversation (`⌘F`), show or hide system notes, reload the transcript and copy any message.
- **Outputs** under the conversation lists the files, PRs, images and reports it produced, with previews for Otto sessions.

**Acting on a conversation**
- **Resume in Otto**: imports an on-disk transcript as an Otto session if needed, restarts it with the provider's resume command, and opens it in the Chat view. A running or idle session just opens.
- **Open folder** reveals the working folder in Finder (the desktop app). In a browser the path is copied instead.
- **Copy path** copies the transcript file's path. The row's ⋯ menu (or right-click) also has **Copy folder path**.
- **Archive** stops an Otto session and keeps its history.
- Deep link to a conversation with `#/history/<session id>`.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| `↵` or `Space` | Open the focused conversation in the list |
| `⌘F` | Search the open conversation |
| `↵` / `⇧↵` | In conversation search: next / previous match |
| `Esc` | Close conversation search |

## Tips and limits

- History uses the **Agents** feature: View to read, Edit to resume or archive. A workspace Viewer can read but not resume.
- You see only your own Otto sessions. Transcripts found on disk (the **on disk** rows) are machine-wide, so only admins see them and only admins can resume them.
- Only Claude and Codex conversations are listed; other providers don't keep a transcript Otto can read.
- A conversation can be resumed only while the provider's transcript is still on disk and its conversation id is known. Otherwise the resume button is disabled.
- The date filter applies to the conversations already loaded. If nothing matches, use **Load more** to look further back.
- On a narrow window the page shows the list or the conversation, with a back button to return to the list.

## Related

- [Agents](#/walkthroughs/agents)
- [Mission Control](#/walkthroughs/mission-control)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
