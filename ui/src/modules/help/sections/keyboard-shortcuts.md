---
id: keyboard-shortcuts
title: Keyboard shortcuts
group: Basics
summary: Every shortcut that works across Otto, from the command bar to sessions, panes, zoom and the system-wide chords.
---
## What it's for

This page lists every shortcut that works everywhere in Otto, plus the system-wide chords the desktop app adds. Shortcuts that only work on one page are listed in that page's guide.

You can also press `?` anywhere outside a text field to open the shortcut sheet over the current page.

## Getting started

1. Press `?` to open the shortcut sheet. Press `Esc` to close it.
2. Press `⌘K` and type any command name. If a command has a shortcut, the result row shows it on the right.
3. Learn the core 4 first: `⌘K` (command bar), `⌘T` (new session), `⌘W` (close tab) and `⌘1` (sidebar).

## Everything it can do

- **Shortcut sheet**: press `?` outside a text field for a sheet of the global shortcuts, grouped into General, Sessions, API client, View and Zoom.
- **Shortcuts in the command bar**: every ⌘K result that has a shortcut shows it, so searching for a command also teaches its keys.
- **App-wide shortcuts**: navigation, panels, find, reload, zoom and CLI updates work on every page.
- **Sessions and panes**: open, close, reopen and cycle tabs, jump to a tab by number, split the view and move panes around.
- **Terminal keys**: new lines in an agent prompt, copy and paste, scrollback search and a per-terminal font size.
- **Desktop menus**: new windows, quit, hide, minimise and full screen from the macOS menu bar.
- **System-wide chords**: the desktop app adds chords that work in any app: the Otto bar and snipping.
- **Page overrides**: a few pages reuse a global chord for their own action while you're on them. They're listed at the end of the tables below.

## Keyboard shortcuts

**App and navigation**

| Keys | Action |
|---|---|
| `⌘K` | Open the floating command bar (the palette sheet on phone, tablet, in a pop-out or with the bar hidden) |
| `⌘I` | Ask Otto in plain English |
| `⌘,` | Open Settings |
| `?` | Show the keyboard shortcut sheet |
| `⌘1` | Show or hide the sidebar |
| `⌘J` | Show or hide the right panel |
| `⌘⇧←` / `⌘⇧→` | Go back / forward through the pages you visited (not while typing in a field) |
| `⌘F` | Find: in the focused terminal or query editor, otherwise on the page |
| `⌘⇧R` | Reload the UI. Sessions keep running in the daemon |
| `⌘U` / `⌘⇧U` | Update all installed agent CLIs |
| `⌘⇧S` | Take a snip: capture a screen region and annotate it |
| `⌘⇧B` | Broadcast a message to your sessions |

**Command bar**

| Keys | Action |
|---|---|
| `↑` / `↓` | Move through the results |
| `↵` | Run the selected row |
| `⌘↵` | Ask Otto with what you typed |
| `⌃1` – `⌃4` | Switch space (while the bar has focus) |
| `Esc` | Close space settings, clear the text, then close the bar |
| `⇥` | In the palette sheet: switch between Commands and Plain English |
| `⌘↵` | In Plain English mode: plan the request |

**Sessions and tabs**

| Keys | Action |
|---|---|
| `⌘T` | New session |
| `⌃⇧T` | New session, when Otto runs in a browser tab (the browser keeps `⌘T`) |
| `⌘W` | Close the current tab |
| `⌃⇧W` | Close the current tab, when Otto runs in a browser tab |
| `⌘⇧T` | Reopen the last closed tab |
| `⌃⇥` / `⌃⇧⇥` | Next / previous tab |
| `⌘]` / `⌘[` | Next / previous session |
| `⌃1` – `⌃9` | Jump to session tab 1–9 |
| `←` / `→` | Move between tabs when a tab has focus (`Home` / `End` for the first / last) |
| `⌘⇧C` | Cycle the focused agent session between terminal, chat and split views |

**New session dialog**

| Keys | Action |
|---|---|
| `←` / `→` | Choose a provider |
| `+` / `-` | Add or remove one more session of the selected provider |
| `⌘↵` | Create the sessions |

**Split panes**

| Keys | Action |
|---|---|
| `⌘D` | Split vertically |
| `⌘⇧D` | Split horizontally |
| `⌘⌥←` / `⌘⌥→` | Move the focused pane left / right |
| `⌘⌥↑` / `⌘⌥↓` | Move the focused pane up / down |
| `⌘⌥S` | Swap the focused pane with the next one |

**Terminal**

| Keys | Action |
|---|---|
| `⇧↵` | New line in an agent's prompt instead of sending it |
| `⌘C` | Copy the selected terminal text (`⌃C` still interrupts) |
| `⌘V` | Paste |
| `⌘F` | Search the scrollback |
| `↵` / `⇧↵` | In terminal search: next / previous match |
| `⌘+` / `⌘-` / `⌘0` | Terminal font bigger / smaller / reset (while the terminal has focus) |

**Find on page**

| Keys | Action |
|---|---|
| `↵` / `⇧↵` | Next / previous match |
| `Esc` | Close find |

**Zoom**

| Keys | Action |
|---|---|
| `⌘+` | Zoom in (the terminal font when a terminal has focus) |
| `⌘-` | Zoom out |
| `⌘0` | Reset zoom |

**Desktop app menus**

| Keys | Action |
|---|---|
| `⌘⇧N` | New window |
| `⌘Q` | Quit Otto and remember your windows for next launch |
| `⌘H` | Hide Otto |
| `⌘M` | Minimise the window |
| `⌃⌘F` | Enter or exit full screen |
| `⌘A` | Select all |

**System-wide (desktop app, work in any app)**

| Keys | Action |
|---|---|
| `⌥Space` | Show or hide the Otto bar over any app |
| `⌘⌃⇧2` | Take a snip |

**Pages that change a global shortcut**

| Keys | Action |
|---|---|
| `⌘T` | On the API page: new request tab instead of a new session |
| `⌘D` | On the API page: duplicate the request instead of splitting |
| `⌘⌥←` / `⌘⌥→` | In a Database pane: previous / next query tab instead of moving the pane |
| `?` | In a Kubernetes cluster view: that view's own key hints instead of this sheet |

## Tips and limits

- Shortcuts shown with `⌃` (like `⌃⇥` and `⌃1`) need the Control key. `⌘1` shows or hides the sidebar; `⌃1` jumps to the first session tab.
- None of the global shortcuts use `⌥`. A chord with `⌥` added never triggers the plain `⌘` action, so pages can use their own `⌥⌘` chords.
- In a terminal, `⌃C` still interrupts the running program. `⌘C` copies only when text is selected.
- In a browser tab, the browser keeps `⌘T`, `⌘W` and `⌘⇧T` for itself. Use `⌃⇧T` and `⌃⇧W` instead, or install Otto as an app.
- On a phone, a quick-action bar gives you the chords you can't press: Palette, New, Close, Find and Broadcast.
- The system-wide chords only work while the desktop app is running. Change or turn off the snip chord in **Settings → Snipping**. If another app already holds a chord, only that chord stops working.
- Backspace outside a text field does nothing, so a missed click never navigates away from the page.

## Related

- [Command bar](#/walkthroughs/command-bar)
- [Desktop app](#/walkthroughs/desktop-app)
- [Phone and remote](#/walkthroughs/phone-and-remote)
- [Agents](#/walkthroughs/agents)
- [Getting started](#/walkthroughs/getting-started)
