---
id: desktop-app
title: Desktop app
group: Basics
summary: What the macOS app adds on top of the browser UI — the ⌥Space bar, the menu-bar item, extra windows, pop-outs and native glass.
---
## What it's for

Otto's macOS app wraps the same UI you'd see in a browser, and adds the parts only a native app can do:

- a bar you can summon over any app with `⌥Space`;
- a menu-bar item that shows whether agents are running or need you;
- more than one Otto window, restored when you relaunch;
- pop-out windows for a single session, database, artifact or page;
- system-wide shortcuts, such as snipping;
- macOS vibrancy behind the sidebar and toolbar.

The app also keeps the Otto daemon running. It installs `ottod` as a background service, checks that it's healthy and starts it if needed, so your sessions keep running after you close the window.

## Getting started

1. Press `⌥Space` in any app. The Otto bar appears near the bottom of the screen you're working on.
2. Type a command or a question and press `↵`. Press `Esc` or `⌥Space` again to hide the bar.
3. Click the ✦ icon in the macOS menu bar to see what needs you, what's running and today's notices.
4. Choose **File → New Window** (`⌘⇧N`) for a second Otto window with its own tabs and module.
5. Right-click a session tab and choose **Open in New Window** to give that session a window of its own.

## Everything it can do

**The ⌥Space bar**
- The same command bar as inside Otto, in a floating glass panel over any app and every desktop Space. It opens centred near the bottom of the screen under your pointer.
- It grows upward as answers arrive, up to 560 points tall, and always stays on screen.
- Commands available there: **Open Otto**, "Go to <module>" for every module you can see, **Open Settings**, "Focus <session>" and "Open repository …" for the space's workspace. Each one brings the main Otto window forward at that page.
- Ask Otto works the same as in the main window. Answers have an **Open in Otto** button.
- Spaces 01–04 are shared with the main window's bar. See [Command bar](#/walkthroughs/command-bar).
- If you're signed out, the bar says so and offers **Open Otto**.

**The menu-bar item**
- The ✦ icon has 3 states: plain when idle, with a dot while agents are working, and amber with a dot when something needs you.
- Hover for a summary tooltip.
- Click it for a popover with:
  - **Ask Otto…**, which opens the ⌥Space bar;
  - **Needs you**: pending MCP approvals, sessions waiting for your input and unread warnings;
  - **Running**: working agent sessions and their workspaces;
  - **Today**: today's notifications;
  - **Open Otto** and **Settings** buttons.
- Every row opens the main window at the right page. The popover refreshes every 20 seconds and when you open it, and it hides when you click elsewhere or press `Esc`.
- Right-click the icon for a menu: **Open Otto**, **Ask Otto…**, **Settings…** and **Quit Otto**.

**Windows**
- **File → New Window** (`⌘⇧N`) opens another full Otto window. Each window has its own module, workspace, tabs, split panes and view mode. Sessions are shared, because they live in the daemon.
- Closing a window forgets it. Quitting with `⌘Q` (or closing the last window) saves every window's position, size, screen and full-screen state, and the next launch reopens the same set.
- If a monitor is gone at relaunch, windows move back onto a screen you have.

**Pop-out windows**
- **Open in New Window** appears in the menus of session tabs, sidebar sessions, database connections and tabs, and Design Hall artifacts. In ⌘K, **Open in New Window** pops out the page you're on.
- A pop-out is a real macOS window without the sidebar or status bar, with a slim title strip.
- Each page has at most one pop-out: asking again brings the open one forward.
- Pop-outs remember their size and position per page, but aren't reopened at launch.

**Updates**
- A release build checks for a new Otto a minute after it starts and every six hours. When one is out, a quiet **Update to <version>** row appears above **Help** in the sidebar (an icon with a dot in the collapsed rail). Only the root account sees it, and only in the desktop app.
- Click it for the release notes, date and size, and one button: **Download and install** → **Downloading… 42 %** → **Restart to finish**. Before restarting, Otto lists what keeps running and what ends (connection terminals, background agents, debug runs…), and asks once more only when something ends.
- **⋯** in the dialog: **Skip this version**, **View release on GitHub**, **Install updates automatically** (downloads in the background; Otto never restarts on its own).
- **Otto → Check for Updates…** checks now and says so when you're up to date or offline. **Settings → Updates** shows the version, the last check, automatic download and a skipped version (**Unskip**).
- After the restart a toast says **Updated to <version>**. If the new version doesn't start, Otto puts the old one back and the row says **Update failed and was undone · Details**.
- A copy built from source says "This is a source build — updates come from git": pull and run `packaging/deploy.sh`.

**Menus**
- **Otto**: About Otto, Check for Updates…, Settings… (`⌘,`), Hide, Quit Otto (`⌘Q`).
- **File**: New Window (`⌘⇧N`), New Session (`⌘T`), New Workspace…, Take Snip, Close Tab (`⌘W`).
- **Edit**: Undo, Redo, Cut, Copy, Paste, Select All.
- **View**: Toggle Navigator (`⌘1`), Toggle Right Panel (`⌘J`), Zoom In, Zoom Out, Actual Size, full screen.
- **Session**: Restart Session (asks first if the agent is mid-turn) and End Session… (for the focused session; does what closing its tab does, per Settings → Appearance → Closing a session tab).
- **Window**: Minimise, Zoom.
- **Help**: Otto Help, which opens this guide.
- Menu commands act on the window you're using, never on every window at once.

**Snipping**
- `⌘⌃⇧2` works in any app while Otto is running. Drag a region (`Space` switches to window capture, `Esc` cancels) and the annotation editor opens with the capture already on your clipboard.
- Inside Otto, `⌘⇧S` or **File → Take Snip** does the same.
- Change or turn off the system-wide chord in **Settings → Snipping**.

**Look and feel**
- With the Native theme, macOS vibrancy shows through the sidebar and toolbar. Pages, tables, editors and terminals stay solid so they're easy to read.
- **Settings → Appearance → Reduce transparency** makes the sidebar, toolbar and menus solid. Otto also follows the macOS Reduce transparency setting.
- Links to websites open in your default browser.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `⌥Space` | Show or hide the Otto bar over any app |
| `⌘⌃⇧2` | Take a snip from any app |
| `⌘⇧N` | New window |
| `⌘Q` | Quit and remember your windows |
| `⌘H` | Hide Otto |
| `⌘M` | Minimise the window |
| `⌃⌘F` | Enter or exit full screen |
| `Esc` | Hide the ⌥Space bar or the menu-bar popover |

## Tips and limits

- The app is macOS-only. In a browser you get the same UI, but not the ⌥Space bar, the menu-bar item, pop-outs, extra windows or system-wide chords.
- If another app already uses `⌥Space` or `⌘⌃⇧2`, only that chord stops working. Change the snip chord in **Settings → Snipping**.
- A hold-to-talk voice chord exists but is off by default, and voice input isn't available yet.
- Quitting the app doesn't stop your sessions. They keep running in the daemon until you archive or delete them.
- Window positions are saved in `~/Library/Application Support/Otto/windows.json`, and pop-out positions in `popouts.json` in the same folder.

## Related

- [Command bar](#/walkthroughs/command-bar)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
- [Phone and remote](#/walkthroughs/phone-and-remote)
- [Getting started](#/walkthroughs/getting-started)
- [Design Hall](#/walkthroughs/design)
