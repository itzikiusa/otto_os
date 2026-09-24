---
id: plugins
title: Plugins
group: Plugins
route: settings/plugins
summary: Add whole new sections to Otto at runtime — each plugin is its own local program with its own page, installed without rebuilding the app.
---

## What it's for

A plugin adds a new section to Otto: its own page in the sidebar and its own backend. It is a separate program on your Mac that Otto starts, supervises and puts behind its sign-in and permissions. Plugins can be written in any language that can serve HTTP on a local port, and you install, enable and remove them while Otto is running.

You manage plugins in **Settings → Plugins**. Each enabled plugin with a page appears in the sidebar's **Plugins** group and opens at `#/plugin/<slug>`.

## Getting started

1. Open **Settings → Plugins** (System group). Only the owner can install and manage plugins.
2. Type a local folder path (for example `~/otto-plugins/dora-metrics`) or a git URL in the install box, or choose **Browse…** to pick a folder. Press Return or choose **Install**.
3. The plugin is added **disabled**. Choose **Enable** to start it.
4. The plugin now shows in the sidebar under **Plugins**. Open it like any other section.
5. To stop it, choose **Disable**. To unregister it, choose **Remove** and confirm.

## Everything it can do

**Managing plugins (Settings → Plugins)**
- **Install** from a local folder (copied into `~/otto-plugins/<slug>/`, skipping `.git`, `node_modules` and `target`) or a git URL (a shallow clone). Otto reads the plugin's `otto-plugin.json` and registers it disabled.
- **Browse…** opens a folder picker that starts in `~/otto-plugins`.
- The table lists each plugin's icon and name, source, slug, version and an **enabled** / **disabled** badge.
- **Enable** starts the plugin's process on a free local port and waits briefly for its health check. If it won't start, the enable is rolled back and you see the error.
- **Disable** stops the process; its sidebar entry disappears.
- **Remove** stops it and unregisters it. Its files under `~/otto-plugins` are kept, so you can reinstall from the same folder.
- If the daemon is briefly unreachable (for example right after an update), the page retries once and then shows **Retry**.

**Plugin pages**
- Every enabled plugin gets a sidebar entry in the **Plugins** group, using its own icon and name, for users allowed to view it.
- The page uses Otto's standard header with the plugin's name; the plugin's own interface fills the body.
- Otto hands the page its API address, your sign-in, the current light or dark theme colours and your list of agent providers, so the plugin can match Otto's look and call its backend as you.
- ⌘K includes a **Go to _plugin name_** command for each plugin you can open.
- You can reorder or hide plugin entries like any other module in **Settings → Appearance**.

**What a plugin can do**
- Serve its own page and backend. Every request goes through Otto's sign-in and permission checks first, and the plugin learns who is calling.
- Keep its own data in a private folder Otto gives it.
- Call back into a small, fixed set of Otto services: the list of git repositories Otto knows, your configured Jira accounts and one account's credentials, and a one-shot agent run (Claude, or Codex when asked for, using your default provider otherwise).

**What a plugin cannot do**
- Reach any other part of Otto's API, the state database, the Keychain or your sessions.
- Listen on anything but the local loopback address.
- Run, or use its Otto token, while disabled or removed.
- Receive a share or guest link's access.

**Examples in the repository**
- `examples/plugins/team-performance` — Jira story throughput against git delivery per assignee (Node, no install step).
- `examples/plugins/dora-metrics` — DORA delivery metrics from git tags and merges (Rust; it compiles on first enable, so the first start takes a while).

## Keyboard shortcuts

| Keys | Action |
|---|---|
| ↩ | Install from the path or URL in the install box |

Inside a plugin page, the plugin can forward shortcut chords (such as ⌘K) back to Otto, so app-wide shortcuts keep working. See [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts).

## Tips and limits

- **Owner only.** Installing, enabling, disabling and removing plugins require the owner account. The Settings entry appears for Settings admins, but other admins can't load the list.
- **Access for other users.** A new plugin is visible only to the owner. Other users need a per-plugin grant: view to open it and make read requests, edit for anything that changes data. There is no screen for these grants yet; set them with `PUT /api/v1/users/{id}/plugin-grants`. The Settings → Plugins note that points to Settings → Users is out of date.
- **Trust.** A plugin is local code with real capabilities: it can read repository paths, fetch a Jira token and run agents. Install only plugins you trust, as you would any program.
- **Local installs are copies.** Editing the original folder doesn't change the installed plugin; install again, or edit the copy in `~/otto-plugins/<slug>`.
- **Enable fails.** Check the plugin's start command and that its runtime (`node`, `cargo`…) is installed, and that it listens on the port Otto passes in `OTTO_PLUGIN_PORT`. Details are in the daemon log (Settings → Logs).
- **Page shows errors or 502.** The plugin's process isn't running. Disable and enable it again.
- **Blank page.** The plugin didn't finish its start-up handshake, or it has no page at all. A backend-only plugin still gets a sidebar entry, but there is nothing to show.
- To write your own plugin, see `docs/plugins/AUTHORING.md` in the repository.

## Related

- [Settings](#/walkthroughs/settings)
- [MCP Control Plane](#/walkthroughs/mcp)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
