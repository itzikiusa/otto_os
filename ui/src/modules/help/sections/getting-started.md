---
id: getting-started
title: Getting started
group: Basics
summary: Set up Otto, add a workspace and run your first agent session in a few minutes.
---
## What it's for

Otto is a macOS app that runs coding-agent CLIs (Claude Code, Codex, Antigravity, a plain shell or your own provider) as live sessions, and connects them to your repos, pull requests, databases, clusters, docs and chat tools.

Everything runs through a local daemon, `ottod`, on your Mac. The daemon owns the sessions, so they keep running when you close the window, reload the UI or open Otto from another device. The app is a view onto that daemon.

This page covers the first run and the shape of the window. Every other section of this guide covers one area of Otto in detail.

## Getting started

1. **Set the root password.** On first launch a setup wizard asks for a root password (at least 10 characters) and an optional display name. The root account manages users, workspaces and daemon settings.
2. **Create your first workspace.** Give it a name and the project folder it maps to. Sessions and repos live inside a workspace. You can skip this step and add one later.
3. **Check usage tracking.** Otto records tokens, cost and system metrics in an embedded ClickHouse engine. If ClickHouse isn't installed, you can install it later from **Usage → Install ClickHouse**.
4. **Check your tools.** The wizard lists the CLIs Otto found (such as `claude` and `codex`) and their versions. You need at least one coding-agent CLI to run an agent session.
5. **Launch your first agent.** Open **Agents**. With no sessions yet, a checklist confirms an agent CLI is installed, makes sure a workspace exists, offers a couple of recommended skills and then starts your first session.
6. **Start more sessions.** Press `⌘T` (or choose **New Session** in the File menu). Pick a provider, press `⌘↵` to create it and type into the terminal.
7. **Find anything.** Press `⌘K` and start typing: a module name, a command, a session or repo name, or a plain-English request for Otto.

## Everything it can do

**The window**
- **Sidebar**: every module you can use, grouped into Work, Automate, Build, Infrastructure, Insight and Plugins. Press `⌘1` to show or hide it. Reorder or hide items in **Settings → Appearance → Sidebar**, or use **Customize sidebar** at the bottom of the expanded sidebar.
- **Tab bar**: your open sessions and connections. Right-click a tab to close others, close to the right, reopen a closed tab, share a session, open it in a new window, or switch between the tabbed and tiled views.
- **Right panel**: Git, Files, Notes, Activity, Outputs, Canvas, Info, Browser and API tabs for the focused session. Press `⌘J` to toggle it. Drag its edge to resize it; double-click the edge to reset.
- **Status bar**: working agents, the network listener state, the current branch and a clock. The floating bar docks here as a small **Ask Otto** chip.
- **Floating bar**: the "Type or speak…" pill at the bottom of the window. It's the command bar and Ask Otto in one. See [Command bar](#/walkthroughs/command-bar).

**Workspaces and sessions**
- A workspace is a project folder. Switch workspaces from the sidebar or with ⌘K → "Switch Workspace: …". Add one with ⌘K → "Add Workspace", or **File → New Workspace…**.
- New sessions can also run with **No workspace**. They start in your home folder and are listed under "No workspace" in the sidebar.
- Sessions survive a UI reload (`⌘⇧R`), closing the window and quitting the app. Only archiving or deleting a session ends it.
- Choose what closing a tab does in **Settings → Appearance → Closing a session tab**: ask every time, always archive (resumable later) or always delete.

**Moving around**
- Every module has a ⌘K command, "Go to <module>". This includes modules you've hidden from the sidebar.
- `⌘⇧←` and `⌘⇧→` move back and forward through the pages you visited.
- Press `?` anywhere outside a text field for the keyboard shortcut sheet.
- **Help → Otto Help** in the menu bar, or **Help** at the bottom of the sidebar, opens this guide.

**Make it yours** (Settings → Appearance, saved per device)
- Theme: Native (macOS vibrancy and the system accent), Pro Dark or Warm. Scheme: Auto, Light or Dark.
- Accent colour, backdrop (None, Subtle or Wallpaper, including your own photo) and **Reduce transparency**.
- Layout direction: left-to-right or right-to-left.
- Terminal font (System, Cousine or Menlo) and an optional right-to-left mode for terminal text.
- Floating bar behaviour: Auto, Always full, Docked or Hidden.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `⌘K` | Open the command bar (commands, search and Ask Otto) |
| `⌘I` | Ask Otto in plain English |
| `⌘T` | New session |
| `⌘W` | Close the current tab |
| `⌘1` | Show or hide the sidebar |
| `⌘J` | Show or hide the right panel |
| `⌘,` | Open Settings |
| `?` | Show all keyboard shortcuts |

The full list is in [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts).

## Tips and limits

- The daemon listens on `127.0.0.1:7700`, so only your Mac can reach it by default. Reaching Otto from a phone or another computer is opt-in. See [Phone and remote](#/walkthroughs/phone-and-remote).
- Your role decides which modules you see. If a module is missing from the sidebar, ask a workspace admin to grant that feature.
- Otto starts agent CLIs for you but doesn't install them. Install `claude`, `codex` or another CLI yourself, then press ⌘U to update every installed CLI in one go later.
- Settings under Appearance are saved per device, so your phone and your Mac can look different.
- Tokens and passwords are stored in the macOS Keychain, never in Otto's database.

## Related

- [Command bar](#/walkthroughs/command-bar)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
- [Desktop app](#/walkthroughs/desktop-app)
- [Phone and remote](#/walkthroughs/phone-and-remote)
- [Agents](#/walkthroughs/agents)
- [Home](#/walkthroughs/home)
