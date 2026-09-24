---
id: settings
title: Settings
group: Basics
route: settings
summary: Every preference, account, integration and admin control in Otto, grouped into General, Integrations, Agents, System and People.
---
## What it's for

Settings is where you shape how Otto looks and behaves on this device, connect
the accounts Otto works with (Git, Jira, Slack, Telegram, MCP servers), tune how
agents are launched and what context they get, and — if you administer Otto —
run the daemon, manage people and access, read logs and take backups.

The left column lists the panes in five groups. You only see the panes your role
can open: admin-only panes are hidden rather than shown disabled.

## Getting started

1. Open Settings with **⌘,**, the gear at the bottom of the sidebar, or
   **Open Settings** in ⌘K. It opens on **Appearance**.
2. Pick a pane in the left column. Each pane has its own address
   (`#/settings/<pane>`), so Back and Forward work between panes.
3. Most panes save as you change them. Forms with several fields (Providers,
   Workspace context, Self-Improvement, Assistant, Daemon) have a **Save** button.
4. Leave with the **×** next to the Settings title (or **⌘⇧←**). It returns to the
   page you came from, or to Agents if there is nothing to go back to.

## Everything it can do

### General

**Appearance** — saved per device
- Theme (Native, Pro Dark, Warm), scheme (Auto follows macOS, Light, Dark),
  layout direction (left-to-right or right-to-left) and a custom accent colour
  with Reset.
- Backdrop behind the sidebar, toolbar and status bar: None, Subtle, or
  Wallpaper (generated, or your own photo, which stays on this device). A
  **Reduce transparency** switch makes the chrome solid; Otto also follows the
  macOS setting.
- Terminal font (System, Cousine, Menlo) and an experimental **Right-to-left
  text in the terminal** switch for Hebrew and other RTL text (reloads open
  terminals).
- Floating bar mode: Auto, Always full, Docked or Hidden (Hidden makes ⌘K open
  the command palette sheet instead). Desktop window only.
- **Isolate sessions to this device** — show only sessions started here.
- What closing a session tab does: ask every time, always archive, or always
  delete.
- Database Explorer auto-vertical: per engine (MongoDB, MySQL, PostgreSQL,
  ClickHouse, Redis), open results in the Vertical view when they have more than
  N columns (1–500). On for MongoDB at 10 columns by default, off for the rest. A
  view you pick on a tab (Grid / Vertical / JSON, or ⇧⌘V) always wins.
- Sidebar: show, hide and reorder modules within their section, including
  plugin pages, and **Reset to default**. You can also drag to reorder with
  **Customize sidebar** at the bottom of the expanded sidebar.

**Session Names**
- Choose the theme new agent sessions are named from, or **Numbered**
  (`claude #1`, `codex #2`).
- Create custom themes (a label plus one name per line) and delete them. When a
  custom list runs out, names repeat with #2, #3….

**Notifications**
- Days of warning before a stored credential (Git or Jira token) expires, 1–30.
- Native macOS notifications for warnings and errors, and alerts when a session
  finishes or is waiting for input.
- Root only: one-line Slack or Telegram pushes for self-improvement events,
  finished code reviews, finished swarms, ready Insights reports and exceeded
  budget caps. All are off by default.

**Snipping** — see [Snipping tool](#/walkthroughs/snip)
- Record, reset (default ⌘⌃⇧2) or disable the system-wide capture shortcut.
  Desktop app only.
- **Take a snip now**, plus a reminder of the in-app triggers.

**Browser** — shown when you can view the Browser; see [Browser](#/walkthroughs/browser)
- Where live tabs render on this device: this Mac's web view, or Otto's
  Chromium (streams anywhere and agents can drive it).
- Browser admins: download the browser engine (Apple silicon only, checked
  against a pinned checksum), show the window on this Mac (needs Chrome for
  Testing), and keep downloaded files in quarantine or block them.

**API Tokens**
- Create personal access tokens for scripts, CI and the Otto API. The secret is
  shown once — copy it before you dismiss it.
- Filter by personal, session or deleted-session tokens; see last-used and
  expiry; revoke one, or revoke all tokens left by deleted sessions.
- Tokens carry your permissions. You can't create one while impersonating.

### Integrations

**Git Accounts**
- Add, edit and delete GitHub, Bitbucket and GitLab accounts: label, username,
  token, optional organisation / workspace / group for repo search and cloning,
  API base URL for self-hosted GitLab, and optional token expiry.
- Tokens live in the macOS Keychain and are used for PR actions and HTTPS
  pushes.

**Jira**
- Add, edit and delete Jira accounts (label, base URL, email, API token,
  optional expiry). Expired or soon-to-expire tokens show a badge.

**Channels** — see [Channels](#/walkthroughs/channels)
- Slack, Telegram and inbound webhook integrations for the current workspace.
  Tokens and webhook keys are stored in the Keychain.

**MCP Servers** — see [MCP Control Plane](#/walkthroughs/mcp)
- Per-workspace MCP servers that are merged into the workspace's `.mcp.json`
  when an agent session starts. Adding a server needs root.

**Language Servers**
- Shows which language servers (Go, Python, TypeScript, JavaScript, Rust, JSON,
  HTML, CSS, SCSS, Markdown, Java and more) are found on your PATH, with their
  install commands.
- **Install** one, or **Install all missing**; installs run in a shell session.

**Sharing**
- A Gmail sender (with an App Password) so Otto can email a 6-digit code to a
  guest before they attach to a shared session.
- The public link domain used in share links and in the emailed link.

### Agents

**Assistant**
- Subscriptions: your Claude and Codex sign-ins, whether a usage limit is hit,
  and each one's share of this week's load.
- Rules: which provider and model handle conversation, code, "think hard" and
  voice requests, plus extra keywords that mean "code" or "think hard". A
  `@claude` or `@codex` prefix, or the model chip on a thread, always overrides.
- When a limit is reached: ask before switching provider, or switch
  automatically.
- Memory: review memories before Otto keeps them, or let it save them with Undo.

**Providers** — needs Settings admin
- **Update all CLIs** now, or daily at a set time (UTC or local), optionally
  reloading open sessions onto the new version.
- Default agent for new sessions and channel replies, and the model used to
  draft PR descriptions and commit messages.
- **Skip permission prompts** (on by default) launches built-in agents with
  their bypass flag; turn it off to use each CLI's own permission mode.
- Hide any provider from every picker, refresh the models catalog, and add
  custom CLIs (command, arguments, resume arguments, update command, model flag).

**Workspace context** — for the workspace selected in the sidebar
- Which library skills are active, the soul (persona), goal, shared
  instructions, workspace memory, decisions, references and artifacts.
- Include MEMORY.md inline and inject a tree-sitter repo map.
- **Materialize** the context files for a provider now, and **Preview** exactly
  what a session start would write. Only workspace admins can edit.

**Self-Improvement** — for the selected workspace
- Enable periodic reviews of recent sessions: cadence in minutes, look-back in
  hours, providers, skill allow-list, and live evolve after each interaction.
- Autonomy: Tiered (safe edits apply, risky ones wait), Propose (every edit
  waits) or Auto.
- **Run now**, evolve the active session, approve or reject pending edits with
  a before/after diff, and see recent runs.

**Insights** — see [Insights](#/walkthroughs/insights)
- Turn on daily, weekly and monthly HTML reports (all off by default) and pick
  the provider that writes them. Missed runs catch up. Needs the `insights`
  skill.

**Skills** — needs Settings admin
- Browse the skills that ship with Otto by category; install, update or remove
  one, or install a whole category. Installing adds the skill to Claude, Codex
  and agy, and backs up your edited copy first.

**Skills Evaluator** — needs Settings admin
- Defaults for the Skills Lab start form: iterations (1–10), validation passes
  (1–3), improver agent and model, and the default validations with the CLIs
  each runs on.

**Context Library** — needs Settings admin
- Edit the Otto-owned library of skills, souls and context snippets that is
  written into each workspace's CLIs, and set the global default soul.

### System — all need Settings admin

**Daemon**
- Network listener: bind `0.0.0.0` on a chosen port so other devices can reach
  the login page. Off (loopback only) by default.
- Process sandbox: confine agent sessions with macOS Seatbelt so they can only
  write to the workspace, its git folder, CLI caches and temp. Network: Full,
  Loopback only or No network.
- The log file location and a link to the log viewer.

**Plugins** — see [Plugins](#/walkthroughs/plugins)
- Install, enable, disable and remove runtime plugins.

**Trust & Safety**
- Security posture: network listener, binding and the number of active API
  tokens.
- The append-only audit log, filtered by action and date (with 24h, 7d and 30d
  presets), 100 entries per page; copy any entry as JSON.

**Logs**
- View one or all daemon log files, the full file or a tail of up to 50,000
  lines, filter by text, auto-refresh and follow the end.

**Backup & Restore**
- **Otto data archive**: download saved data (workflows, scheduled tasks,
  connections, API collections, session records, documents and Vault files).
  Restore previews first, keeps existing items, and leaves restored schedules
  inactive until you enable them.
- **Git backup sync**: write a snapshot of portable configuration into a local
  Git repository, commit it, fetch, pull fast-forward or push, and restore from
  it with a preview.
- **Export connections** for other apps in JSON, CSV and other formats, with or
  without passwords.
- **Settings**: export or import the daemon settings (secrets are filtered),
  download a settings backup with a manifest, or restore one after typing
  `restore` to unlock it.

### People

**Users** — needs Users admin
- Create, enable and disable users, filter the list, copy a username or a user
  as JSON, and impersonate a user.
- Workspace roles (none, viewer, editor, admin), by workspace or by user, with
  **Set all** for one user.
- Feature grants: none, view, edit or admin per feature for each non-root user.

**Groups & Access** — root only
- Create groups and manage their members.
- Role presets: named sets of operations (and grantable operations) for a
  resource type, which you copy into a resource's access rules.

**Sessions** — needs Users admin
- Every session across every user, with owner, kind, status and viewer count.
- Terminate or remove sessions one at a time or in bulk, and remove all exited
  sessions.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| ⌘, | Open Settings (Appearance) |
| ⌘⇧← | Close Settings and go back to the previous page |
| ↵ | In API Tokens' label field: create the token |
| ↵ | In Plugins' source field: install the plugin |
| ↵ | In Logs' line-count field: reload the log |
| Esc | While recording the snip shortcut: cancel recording |

Global shortcuts are listed in [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts).

## Tips and limits

- **Per device vs shared.** Appearance (theme, backdrop, floating bar, sidebar,
  auto-vertical, session isolation, close-tab choice) is saved on this device
  only. Accounts, providers, context and admin settings live on the daemon.
- **Roles.** Providers, Skills, Skills Evaluator, Context Library and every
  System pane need Settings admin. Users and Sessions need Users admin. Groups &
  Access and the channel notification switches need root. Opening a pane you
  can't use shows Appearance instead.
- **Workspace panes.** Workspace context, Self-Improvement, Channels and MCP
  Servers apply to the workspace selected in the sidebar. The old Projects page
  (`#/projects`) now opens Workspace context, as does **Workspace context** in a
  workspace's menu in the sidebar.
- **Secrets stay in the Keychain.** Git, Jira, channel and email-sender tokens
  are stored in the macOS Keychain; settings exports and backups filter secrets
  out. A connections export with passwords contains readable credentials — keep
  it private and out of Git.
- **Network listener.** Turning it on lets anyone on your network reach the
  login page. Enable it only on trusted networks.
- **Sandbox network.** Loopback only or No network stops agent CLIs reaching
  their model API; use them only for offline shells.
- **Skip permission prompts** is on by default so agents never block. Turn it
  off if you want each tool use approved in the terminal; it applies to new
  sessions only.
- **Token secrets are shown once.** If you lose one, revoke it and create a new
  one.
- Right-click a pane in the left column for **Open <pane>**.

## Related

- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
- [Channels](#/walkthroughs/channels)
- [Plugins](#/walkthroughs/plugins)
- [Snipping tool](#/walkthroughs/snip)
- [Browser](#/walkthroughs/browser)
- [MCP Control Plane](#/walkthroughs/mcp)
- [Insights](#/walkthroughs/insights)
- [Database Explorer](#/walkthroughs/database)
