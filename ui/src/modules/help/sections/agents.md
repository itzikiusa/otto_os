---
id: agents
title: Agents
group: Work
route: agents
summary: Run Claude Code, Codex, Antigravity, custom agent CLIs and plain shells as live terminal sessions you can split, tile, chat with and resume.
---

## What it's for

Agents is where your coding agents run. Each session is a real terminal on this Mac, owned by the Otto daemon, so it keeps running when you close the tab, the window or the app. You can watch many at once, type into any of them, read them as a chat, hand work from one agent to another, and pick a conversation back up after a restart.

## Getting started

1. Press `⌘T` (or **New session** in the tab bar) to open the New Session sheet.
2. Choose where it runs: the current workspace, or **No workspace** for a one-off job in your home folder.
3. Pick a provider card: `claude`, `codex`, `agy`, `shell`, or a custom provider. A card is greyed out when its CLI isn't installed; the tooltip says why.
4. Optionally set a title, a working directory (**Browse…** opens the folder picker) and extra directories the agent may read.
5. Press **Start Session** (or `⌘↵`). The session opens as a tab and its folder is trusted automatically, so the agent doesn't stop on a "do you trust this folder?" prompt.
6. Type into the terminal, or switch the pane to **Chat** to read the conversation and send messages from a composer.

## Everything it can do

**Starting sessions**
- Providers: `claude` (Claude Code CLI), `codex` (Codex CLI), `agy` (Antigravity), `shell` (a login shell), plus custom providers added in Settings → Providers.
- Start several at once: use the `−` / `+` stepper on each card (up to 20 per provider). "2 codex, 3 claude and 1 shell" is one trip; the batch opens tiled. A title becomes a numbered base name ("fix tests 1", "fix tests 2"…).
- Pin a model for the session (single-provider batches only), pick a subscription account for `claude` or `codex` (**Add account**, **Sign in**, **Check sign-in**), and choose a network profile that tunnels named endpoints through an SSH bastion.
- Working directory can be any folder on this Mac, inside a workspace or not. Recent folders are suggested. Missing folders are created.
- **Additional directories** are passed to the agent as `--add-dir` (ignored for `shell`).
- **Browser tools** (`claude`, `codex`): gives the agent a real browser through MCP.
- **Preview context** (`claude`, `codex`, in a workspace): shows exactly which skills, soul and context Otto will inject before the agent starts.
- **New session (no workspace)**: from ⌘K or by right-clicking the Agents list. These sessions appear in a **No workspace** group in the sidebar.
- Session names come from your naming theme (Settings → Session Names) when you leave the title blank.

**Views**
- **Tabs**: one tab per open session. Drag tabs to reorder; middle-click closes. Right-click a tab for Rename, Close others, Close to the right, Close all tabs, Reopen closed tab, Share…, Open in New Window (desktop app) and view switching.
- **Split panes**: split any pane left/right/up/down, up to 15 panes. Drag any divider (arrow keys, `Home` and `End` also work on a focused divider). Drag a pane by its grip onto another pane's centre to swap, or onto an edge to split beside it. Layout presets: Equal columns, Equal rows, One above two, One beside two, Grid (pane ⋯ menu or ⌘K). The layout is remembered per workspace.
- **Tiled view**: every session in a grid. Up to 15 tiles are live at once; the rest show a light placeholder and only connect when you click **Click to attach**, so opening the grid doesn't wake every suspended agent. Drag tiles to reorder. **Free layout** turns the grid into split panes.
- **Work Queue**: the tab bar's gauge button shows your sessions and runs in 6 buckets — Needs You, Working, Review Ready, Waiting, Failed, Budget Warning. Save filtered views by status, provider and repository (or advanced JSON), and push a sub-task to a running agent.
- Per pane: **Terminal**, **Chat** (the conversation rebuilt from the agent's transcript) or **Split** (chat beside the terminal, on wide windows). Chat works for `claude` and `codex`.

**Terminal**
- Full scrollback survives reconnects (the daemon keeps 10,000 lines per session). Find (`⌘F`) searches the screen and the daemon's full buffer.
- Click a URL to open it in your browser, or a file reference like `src/App.svelte:42` to open it in the Files panel.
- `⌥`-drag selects text in agent CLIs (they capture the mouse). Copy-on-select is a toggle in the pane header.
- Paste an image and Otto uploads it, then types the saved file's path into the terminal.
- `⇧↵` inserts a newline in the agent's prompt instead of submitting.
- Terminal font size: the `−` / `+` buttons in the pane header, or the zoom keys while the terminal is focused.

**Chat view**
- Turns, tool steps (with diff stats), sub-agent cards, task lists, images and artifacts, with cost, token and duration stats in the header.
- Search the conversation, show or hide system notes, reload the transcript, copy any message.
- Composer: `↵` sends, `⇧↵` adds a newline, `/` lists the provider's slash commands and your skills, paste or drop images to attach them. Messages sent while the agent is busy show as queued.

**Broadcast**
- With 2 or more panes open, **↗ broadcast** sends one line to every visible session. `⌘⇧B` opens Ask Otto pre-filled to broadcast to the sessions you name.
- Broadcast works only when every target is in the same workspace (or all are workspace-less); the bar says when it isn't available.

**Hand over**
- **Hand over to…** (pane ⋯ menu or ⌘K) moves an agent's working context to a new agent or an existing one in the same workspace. Otto summarises the recent work into a brief and types it into the target.
- Options: a focus note, include git state, review the brief before sending, fast summary, and archive the source once you confirm the target received it. Failed deliveries keep the brief and can be retried.

**Session lifecycle**
- Status dots: running, working, idle, exited, suspended (resumable). A **Needs you** badge marks a session waiting for your input or a permission.
- Sessions resume on open after a daemon restart. `claude`, `codex` and `agy` conversations resume when their conversation id was captured. A `shell` always comes back as a fresh shell, and an agent you started inside it is resumed with its own resume command.
- Sessions you start here are never auto-suspended. Background sessions (workflows, reviews, swarms, channels…) are suspended after 5 idle minutes to free memory, and the pane header shows the countdown. **Pin (keep alive)** stops that.
- **Restart session** respawns the process (resuming the conversation when possible). It's in the pane header and, while an agent is running, in the sidebar row menu (for a stuck process). If the agent is mid-turn, Otto asks first.
- Close a tab to only hide it (the session keeps running), **Archive** to stop it and keep its history, or **Delete** to remove it and its history (always confirmed). Closed tabs reopen with `⌘⇧T`.
- Select many sessions in the sidebar to archive or delete them together. Archived sessions live in the Archived section, where you can restore them.

**Around the session**
- Pane ⋯ menu: Rename…, Additional directories…, Hand over to…, Attach Jira issue…, Attach product story… (injects the refined story into the session), Canvas…, Pin, Archive, Delete.
- Right panel (`⌘J`): Git, Files, Notes, Activity (the agent's tool calls, commands, file edits and task progress, plus your own notes), Outputs (files, PRs, images and reports the agent produced, with previews), Canvas, Info, Browser and API.
- **Share…** creates a link to watch (Viewer) or type in (Editor) a session, with an expiry, an optional label and an optional emailed one-time code.
- Sidebar: search all sessions, filter to **Needs you**, show sessions from all workspaces, sort by recent or drag to a manual order, and see Slack and Telegram sessions in their own groups.
- **Update all CLIs** (`⌘U`) runs each provider's update command in a session.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| `⌘T` | New session (`⌃⇧T` when Otto runs in a browser tab) |
| `⌘↵` | Start the session from anywhere in the New Session sheet |
| `←` `→` `↑` `↓` | Switch provider card in the New Session sheet |
| `+` / `−` | One more / one less session for the selected provider card |
| `⌘W` | Close tab (`⌃⇧W` in a browser tab) |
| `⌘⇧T` | Reopen closed tab |
| `⌃Tab` / `⌃⇧Tab` | Next / previous tab |
| `⌘]` / `⌘[` | Next / previous session |
| `⌃1`…`⌃9` | Jump to session 1–9 |
| `⌘D` | Split vertically |
| `⌘⇧D` | Split horizontally |
| `⌘⌥←` `⌘⌥→` `⌘⌥↑` `⌘⌥↓` | Move the focused pane left, right, up or down |
| `⌘⌥S` | Swap the focused pane with the next |
| `⌘⇧C` | Cycle the pane between Terminal, Chat and Split |
| `⌘F` | Find in the terminal, or search the conversation in Chat |
| `↵` / `⇧↵` | In Chat search: next / previous match |
| `⌘C` / `⌃⇧C` | Copy the terminal selection |
| `⌃⇧V` | Paste into the terminal from the clipboard |
| `⇧↵` | New line in the agent's prompt (terminal and composer) |
| `⌥`-drag | Select text in an agent terminal |
| `⌘+` / `⌘−` / `⌘0` | Terminal font larger / smaller / reset (terminal focused) |
| `⌘⇧B` | Broadcast to sessions |
| `⌘U` | Update all agent CLIs |
| `⌘J` | Toggle the right panel |

## Tips and limits

- Access needs the **Agents** feature: View to watch, Edit to start, type into, restart, archive or hand over. You see only your own sessions; workspace admins see everyone's. A workspace Viewer can watch a terminal but not type.
- Resume depends on the provider's transcript on disk. If the CLI has cleaned it up, the session reopens without its earlier conversation.
- Chat view needs a readable transcript: it's available for `claude` and `codex`, and appears after the first prompt.
- By default agents run with their skip-permissions flag so they never block on tool approval. Turn this off in Settings → Providers (applies to new sessions).
- The OS sandbox is off by default. Turn it on in Settings → Daemon → Process sandbox to limit where agents can write.
- A session started in your home folder is trusted for, and may write anywhere under, `~`. Pick a narrower folder to keep it confined.
- Hand over only works within one workspace, and only to reasoning agents (not `shell`).
- Pop-out windows (**Open in New Window**) are only in the desktop app.
- "Isolate sessions to this device" in Settings → Appearance hides sessions started on other devices. They keep running; check it if a session seems to be missing.

## Related

- [History](#/walkthroughs/history)
- [Mission Control](#/walkthroughs/mission-control)
- [Run with Otto](#/walkthroughs/run-with-otto)
- [Command bar](#/walkthroughs/command-bar)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
- [Phone and remote access](#/walkthroughs/phone-and-remote)
- [Connections](#/walkthroughs/connections)
