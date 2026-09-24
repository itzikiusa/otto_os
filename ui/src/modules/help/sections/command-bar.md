---
id: command-bar
title: Command bar
group: Basics
summary: The floating "Type or speak… ⌘K" bar — run any command, search your workspace or ask Otto in plain English.
---
## What it's for

The command bar is Otto's front door. It's the pill that floats at the bottom of the window with "Type or speak…" and `⌘K` in it. One box does 3 things:

- **Run a command**: go to any module, focus a session, open a repo, change the theme and more.
- **Search your workspace**: repos, workflows, product stories, API requests, Swarm tasks and projects, Kafka clusters and saved memory.
- **Ask Otto**: type a plain-English request ("open 2 claude sessions", "send to session 1: run the tests") and Otto plans it.

The bar has 4 **spaces** (01–04). Each space remembers its own name, workspace, agent, model and conversation thread, so you can keep work and personal requests apart.

## Getting started

1. Press `⌘K` from anywhere in Otto. The bar opens with your recent commands.
2. Type a few letters of what you want, such as "git" or "go to usage". Use `↑` and `↓` to pick a row and press `↵` to run it.
3. To ask Otto instead, type a sentence and press `⌘↵`. The answer appears in the space's thread above the bar.
4. If Otto proposes a plan, read the numbered steps and choose **Run plan** or **Cancel**. Nothing runs until you confirm.
5. Press `⌃1`, `⌃2`, `⌃3` or `⌃4` in the bar to switch spaces. Click the active space's number again to change its settings.
6. Press `Esc` to clear what you typed, and `Esc` again to close the bar.

## Everything it can do

**Commands**
- Every command Otto registers: "Go to <module>" for each module you can see (including hidden ones), "Focus Session: …", "Open Repo: …", "Switch Workspace: …", "Connect: …" for saved connections, session actions (restart, archive, rename, hand over, attach a Jira issue or product story), layout presets, themes, a "Guide: …" command for every Help guide, and more.
- Ranking is fuzzy over the title, keywords and group, and boosted by how often and how recently you ran each command.
- Rows show the command's group and its shortcut, if it has one.
- With nothing typed, the bar lists your 6 most recent commands.

**Search**
- After 2 characters, the bar also searches the workspace the space is using. Results appear under "In this workspace", after the commands, and open the module that owns them.

**Ask Otto**
- Free text defaults to Ask Otto: when what you typed doesn't read like a command name, the **Ask Otto** row comes first and `↵` sends it. `⌘↵` always asks, whatever row is selected.
- Otto understands these requests without an AI call:
  - closing sessions ("close sessions 1,2", "close all claude sessions", "close <name>");
  - deleting sessions permanently ("delete …"), which always asks you to confirm first;
  - sending text to sessions ("send to <name> hi", "session 1: hi", "all: hi");
  - opening sessions ("open 2 claude sessions").
- Anything else goes to the AI planner, which returns a plan (open sessions, send a message, broadcast, open a connection). You confirm with **Run plan**.
- Every answer is labelled with its source ("Ask Otto" or "Ask Otto · planner") and the time. Answers that point somewhere have an **Open** button.
- A dot on the bar tells you a reply arrived while the bar was closed.

**Spaces**
- 4 spaces, named Personal, Work, Research and Home by default. Each keeps the last 20 turns of its thread.
- Space settings: **Name** (up to 24 characters), **Workspace** (a fixed workspace, or follow the one selected in Otto), **Agent** (the provider) and **Model**. **Clear thread** empties the conversation.
- Switching to a space that's pinned to a workspace also switches Otto to that workspace.
- The spaces are the same 01–04 spaces as [Home](#/walkthroughs/home): switching in one switches the other, and renaming a space renames the Home view.
- The model chip on the bar shows the space's agent and model. Click it to open the space settings.

**How the bar stays out of the way** (Settings → Appearance → Floating bar)
- **Auto** (the default): full on Home. On other pages it docks as a small **Ask Otto ⌘K** chip in the status bar, and it also docks while you scroll or type in a terminal or editor.
- **Always full**: the whole pill stays up, and it shrinks only while a terminal or editor has the keyboard.
- **Docked**: always the status-bar chip, so it never covers content. `⌘K` still opens it in full.
- **Hidden**: no bar. `⌘K` opens the command palette sheet instead.
- The bar steps aside whenever a dialog, sheet or the palette is open.

**The palette sheet**
- On a phone or tablet, in a pop-out window, or with the bar hidden, `⌘K` opens a palette sheet with 2 modes: **Commands** and **Plain English**. Press `⇥` to switch.
- `⌘I` opens the sheet straight in Plain English mode on any device. It has 2 toggles: **optimize** (rewrite your request before planning) and **AI fallback** (send requests Otto can't parse to the AI planner; on by default). Press `⌘↵` to plan.

**Outside Otto**
- In the desktop app, `⌥Space` opens the same bar over any app. See [Desktop app](#/walkthroughs/desktop-app).

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `⌘K` | Open or close the command bar |
| `⌘I` | Ask Otto in plain English (palette sheet) |
| `↑` / `↓` | Move through the results |
| `↵` | Run the selected row |
| `⌘↵` | Ask Otto with what you typed, whatever row is selected |
| `⌃1` – `⌃4` | Switch to space 01–04 (while the bar has focus) |
| `Esc` | Close space settings, then clear the text, then close the bar |
| `⇥` | In the palette sheet: switch between Commands and Plain English |
| `⌥Space` | Open the bar over any app (desktop app) |

## Tips and limits

- The microphone button is a placeholder. Voice input isn't available yet, so type your request.
- Spaces and their threads are stored on this Mac only, per device. They don't sync to your phone.
- Ask Otto plans with Otto's orchestrator. The model you pick for a space is saved with the space, but the planner doesn't use it yet.
- A plan that was waiting for confirmation isn't kept if you reload or close the bar. The thread notes it as "Not run".
- Outside the bar, `⌃1`–`⌃9` jump to session tabs. Inside the bar, `⌃1`–`⌃4` switch spaces instead.
- Search needs a workspace. With no workspace selected, the bar only lists commands.

## Related

- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
- [Home](#/walkthroughs/home)
- [Assistant](#/walkthroughs/assistant)
- [Desktop app](#/walkthroughs/desktop-app)
- [Getting started](#/walkthroughs/getting-started)
