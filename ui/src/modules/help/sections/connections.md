---
id: connections
title: Connections
group: Infrastructure
route: connections
summary: One tree for every saved SSH host, database, Kafka cluster and custom CLI, and the place you open them.
---

## What it's for

Connections is the hub for everything Otto can reach on your behalf. SSH
hosts, MySQL, PostgreSQL, Redis, MongoDB and ClickHouse databases, Kafka
clusters and custom command-line tools all live in one shared, foldered tree.
Define a host, its credentials and its SSH tunnel once, and the same profile
feeds a terminal session, the [Database Explorer](#/walkthroughs/database) and
the [Message Brokers](#/walkthroughs/brokers) viewer.

Opening a row takes you to the right tool:

- **Database kinds** open in the Database Explorer workbench, right on this page.
- **SSH and custom profiles** open a live terminal as a tab on this page.
- **Kafka clusters** open the cluster viewer as a tab on this page.

Database Explorer and Message Brokers have no sidebar entries of their own.
You reach them from here, and the sidebar keeps Connections highlighted.

## Getting started

1. Open **Connections** in the sidebar (Infrastructure group).
2. Click **New connection** in the header, or **+** next to the filter box.
3. Optional: click **Paste connection URI…** and paste a URI such as
   `mysql://user:pass@host:3306/db`. Otto fills in the kind, host, port, user,
   database, password and TLS mode for you.
4. Pick a **Kind**: `ssh`, `mysql`, `postgres`, `redis`, `mongodb`,
   `clickhouse` or `custom`. You can't change the kind after you save.
5. Choose an **Environment** (`dev`, `staging` or `prod`), and tick
   **Read-only** if the profile must never write.
6. Fill in the host details and password. For a database behind a bastion,
   turn on **SSH tunnel**.
7. For database kinds, click **Test** to check the unsaved settings, then
   **Create Connection**. A new database connection opens straight into the
   workbench.
8. For a Kafka cluster, click the **New Kafka cluster** button (next to **+**)
   instead. See [Message Brokers](#/walkthroughs/brokers).

## Everything it can do

### The tree

- **Kinds.** SSH, MySQL, PostgreSQL, Redis, MongoDB, ClickHouse, Kafka and
  Custom rows share one tree. Each row shows a kind glyph and a kind tag.
- **Type filter chips.** Choose All, SSH, MySQL, PostgreSQL, Redis, MongoDB,
  ClickHouse, Kafka or Custom to narrow the tree. Your choice is remembered on
  this device.
- **Filter box.** Type in **Filter connections…** to replace the tree with a
  flat list. It matches name, kind, host and port, and folder path.
- **Sections (folders).** Use **New section**, then **Add sub-section**,
  **Rename section** and **Delete section** on a folder's hover buttons.
  Folders nest, and each one shows a count of the rows inside it.
- **Drag and drop.** Drag a connection or cluster onto a folder to file it.
  Drag a folder onto another folder to nest it. Drop either one on the
  top-level zone to ungroup it. Otto won't nest a folder inside itself.
- Deleting a folder removes its sub-folders too. The connections inside move
  to Ungrouped and are never deleted.
- **Environment badges.** `PROD`, `RO` (read-only) and `STG` badges appear on
  rows and tabs. A production tab draws a red rail down the work area.
- **Resizable sidebar.** Drag the divider to widen it, or double-click the
  divider to reset. The width is remembered.

### Opening things

- Click a **database** row to open it in the workbench. Click an **SSH or
  custom** row to open a terminal tab. Click a **Kafka** row to open the
  cluster viewer.
- **Tabs.** Database connections, Kafka clusters and terminals share one tab
  strip. A tab shows its folder name, a spinner while connecting and a red dot
  if the connection failed.
- Closing a terminal tab ends its session on the daemon.
- Open workbench tabs are restored when you come back, even after switching
  workspaces.
- **Right-click a database row or tab** for:
  - **Open beside agents (split)**: docks the database as a pane next to your
    agent on the Agents page.
  - **Open terminal client**: runs the engine's own CLI (`mysql`, `psql`,
    `redis-cli`, `mongosh` or `clickhouse-client`) in a terminal tab.
  - **Open in New Window** (desktop app only).
  - **Edit**, **Delete**, **Duplicate without password** and **Access**.
- **Right-click an SSH row** for **Open terminal session**, **Browse files
  (SFTP)**, **Edit**, **Delete** and **Access**.
- **Right-click a Kafka row** for **Open**, **Open in Message Brokers page**,
  **Edit** and **Delete**.

### The connection form

- **Name** and **Section** (pick a folder, or **＋ New** to create one inline).
- **SSH:** host, port, user and jump host. Leave the user empty to let `ssh`
  resolve it from `~/.ssh/config`, or fall back to your local username. You
  can also set an identity file (with a file picker).
- **MySQL, PostgreSQL, Redis and ClickHouse:** host, port, user and database.
  Redis asks for a DB index instead of a database.
- **MongoDB:** one connection string. `mongodb+srv://` works. When you save,
  the password moves into the Keychain and the string keeps `{secret}` in its
  place.
- **Custom:** a command template such as `psql -h {host} -U {user} {db}`.
  `{secret}` is filled in from the Keychain when the command runs.
- **Timezone** (MySQL, PostgreSQL and ClickHouse): the session time zone.
  The default is UTC.
- **TLS / SSL** (database kinds): choose Disabled, Preferred or Required. You
  can also turn off **Verify server certificate**, and set a CA certificate,
  client cert, client key and server name (SNI).
- **SSH tunnel** (database kinds): tunnel host, port, user and identity file.
  The Database Explorer uses it to reach a database through a bastion.
- **Connect via SSH** (jump host + identity): wraps a database CLI terminal
  through a bastion.
- **First command:** text typed into the terminal right after the client
  connects, for example `USE app_db; SHOW TABLES;`.
- **Test** checks the current, unsaved settings without saving anything. It
  appears for database kinds only.

### SFTP file browser (SSH only)

- Right-click an SSH row and choose **Browse files (SFTP)**. The browser opens
  in your remote home folder.
- Navigate with the breadcrumb, **Up** and **Refresh**. Double-click a folder
  to open it.
- **Filter this folder…** narrows the current listing.
- **View** a text file up to 1 MiB. Larger files are marked as truncated.
- **Download** to a folder on this Mac, **Upload** a file from this Mac, and
  use **New folder**, **Rename** and **Delete** (a folder must be empty to
  delete it).
- Every transfer shows its progress, byte count and elapsed time, with a
  **Cancel** button. A download never overwrites an existing local file.

### Import and export

- **Import** (header button, or the arrow next to **+**) reads saved profiles
  from MySQL Workbench, DBeaver, DataGrip or NoSQLBooster on this Mac. You
  don't pick a file.
- The preview lets you tick rows and choose **Create new**, **Update
  existing** or **Skip** for each one. Passwords are never imported.
- **Export** lives in Settings → Backup & Restore → Export connections. It
  writes JSON, CSV or a format a database app can import, for all workspaces
  or selected ones. Saved passwords are included only if you tick
  **Include saved passwords and credentials**.

### Access and sharing

- **Access** (the key button on a row) opens per-connection rules: shell, SFTP
  read and write for SSH, and browse, query, export and change operations for
  databases.
- **Duplicate without password** copies a profile's settings and protection
  flags, but not its password or access rules.

### Used elsewhere

- **Network profiles.** In **New session**, a network profile forwards chosen
  TCP endpoints through a saved SSH connection. The agent or shell gets
  `127.0.0.1` addresses and `OTTO_TUNNEL_<NAME>_HOST` / `_PORT` variables.
  Each profile has 1–8 endpoints.
- **Agents.** Agents can list your saved connections through Otto's MCP tools
  (`list_connections`), subject to the same access rules.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| ⌘B | Hide or show the sidebar (while the query editor has focus) |
| ↵ | In **Paste connection URI…**, fill the form from the URI |
| Esc | In **Paste connection URI…**, cancel |
| ↵ | In the SFTP browser, open the focused folder or file |

See [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts) for the global
keys.

## Tips and limits

- **Who can do what.** Connections appears when you have the Connections or
  Database feature.
- Only the owner (root) can create or import connections, run **Test**, or
  change credentials, kind, host settings and environment. Users granted
  configure access can still rename a profile or move it between folders.
- Opening an SSH or custom terminal needs the connection's `shell` access.
  The SFTP browser needs SFTP read, plus SFTP write to change files.
- **Secrets stay in the macOS Keychain.** Otto's database only holds a
  reference. Deleting a connection deletes its Keychain secret too.
- **Production and read-only.** In the Database Explorer, `prod` connections
  block writes and schema changes until you confirm, and read-only
  connections refuse them outright. These flags don't restrict a terminal
  session.
- **SSH uses your own setup.** Otto runs the system `ssh` and `sftp`, so your
  ssh-agent, `~/.ssh/config` and `known_hosts` apply. There's no SSH password
  prompt. A key file must be private (`chmod 600`) or `ssh` ignores it.
- **Tunnels need forwarding on the bastion.** If `sshd` sets
  `AllowTcpForwarding no`, tunnels fail straight away with "administratively
  prohibited". Allow forwarding on the bastion.
- **ClickHouse terminal passwords** travel on the command line, because
  `clickhouse-client` has no other channel. Other users of the same Mac can
  see them in `ps`. The form warns you about this.
- **ClickHouse ports.** Use the HTTP interface: 8123, or 8443 with TLS. The
  native ports 9000 and 9440 aren't supported.
- **Client tools.** Terminals need the client binaries on your `PATH`: `ssh`,
  `sftp`, `mysql`, `psql`, `redis-cli`, `mongosh` or `clickhouse-client`.
- **SFTP uses this Mac's disk.** Downloads and uploads read and write the disk
  of the Mac running the daemon, even when you're using Otto remotely.
  Transfers don't resume after a daemon restart.
- **URI paste.** Unsupported URI options are listed in a warning instead of
  being dropped silently.

## Related

- [Database Explorer](#/walkthroughs/database)
- [Message Brokers](#/walkthroughs/brokers)
- [Agents](#/walkthroughs/agents)
- [Settings](#/walkthroughs/settings)
- [MCP Control Plane](#/walkthroughs/mcp)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
