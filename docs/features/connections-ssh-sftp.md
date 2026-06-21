# Connections — SSH, Databases, Tunnels & the SFTP Browser

A **Connection** is a saved, reusable profile for reaching a remote endpoint —
an SSH host, a MySQL / Redis / MongoDB / ClickHouse server, or any custom CLI.
Opening a profile drops you into a **live terminal session** sitting side-by-side
with your agents (same session machinery), driven by the system client binary
(`ssh`, `mysql`, `redis-cli`, `mongosh`, `clickhouse-client`, …). The very same
Connection objects feed the **Database Explorer** (native data access — see
[`./database-explorer.md`](./database-explorer.md)) and the **Kafka viewer** (see
[`./message-brokers.md`](./message-brokers.md)); a connection's SSH-tunnel block is
the shared bridge those features use to reach private databases and brokers.

For **SSH** connections only, Otto also exposes an **SFTP file browser** — list,
open, download, upload, mkdir, rename, and delete remote files over the exact same
SSH auth, by driving the system `sftp` binary.

Secrets (passwords, key passphrases) **never** live in the SQLite state DB or in
the repo — only in the **macOS Keychain**, referenced by an opaque key. The daemon
listens on `127.0.0.1:7700` (loopback) and runs on *your* Mac, so SFTP
download/upload read and write *your* real local disk.

---

## 1. Overview & where it lives

| Concern | Location |
|---|---|
| Connection list / sidebar (SSH + Custom) | `ui/src/modules/connections/ConnectionsPage.svelte` |
| Create / edit form (all kinds) | `ui/src/modules/connections/ConnectionForm.svelte` |
| SFTP browser UI | `ui/src/modules/connections/SftpBrowser.svelte` |
| SFTP client store (per-connection cwd) | `ui/src/lib/stores/sftp.svelte.ts` |
| Profile CRUD + open/test service | `crates/otto-connections/src/service.rs` |
| Per-kind command builders | `crates/otto-connections/src/builders.rs` |
| REST router (connections + SFTP + sections) | `crates/otto-connections/src/http.rs` |
| Shared SSH-tunnel helper (`-L` / `-D`) | `crates/otto-ssh/src/lib.rs` |
| SFTP-over-`sftp` engine | `crates/otto-ssh/src/sftp.rs` |
| Domain types (`Connection`, `ConnectionKind`, `Environment`, `ConnectionSection`) | `crates/otto-core/src/domain.rs` |
| Request/response shapes (`UpsertConnectionReq`, `Sftp*`) | `crates/otto-core/src/api.rs` |
| Authoritative contract | `docs/contracts/api.md` (§ Connections #25–30, § SFTP, § sections, § MCP) |

> **DB engines vs. the Connections page.** The Connections page shows only
> `ssh` and `custom` profiles; MySQL / Redis / MongoDB / ClickHouse profiles are
> created and managed inside the **Database Explorer** (they are the *same*
> `Connection` rows — just filtered into a different page). All kinds can still be
> *opened as a terminal* and *tested* through the endpoints documented here.

---

## 2. Defining a connection

A profile is one `UpsertConnectionReq`:

```jsonc
{
  "name": "staging mysql",          // required, display name
  "kind": "mysql",                  // ssh | mysql | redis | mongodb | clickhouse | custom
  "params": { /* per-kind, below */ },
  "secret": "s3cr3t",               // write-only → Keychain; omit on PATCH to keep
  "first_command": "USE app_db; SHOW TABLES;",  // optional; typed into the PTY after connect
  "section_id": null,               // sidebar grouping; null = ungrouped
  "environment": "dev",             // dev | staging | prod  (write-gate, DB Explorer)
  "read_only": false                // hard write-lock regardless of environment
}
```

- **`kind` is immutable.** A PATCH that changes `kind` is rejected (`400 invalid`,
  *"connection kind cannot be changed — create a new connection"*). Pick the right
  kind up front.
- **`environment` / `read_only` use true PATCH semantics.** On *create*, absent ⇒
  `dev` / `false`. On *PATCH*, absent ⇒ **keep the stored value** — a partial PATCH
  can never silently downgrade a `prod` or read-only profile and disable its
  write-guard. `prod` and `read_only` are danger flags consumed by the DB Explorer
  (writes/DDL require an explicit confirm); they don't restrict terminal sessions.
- The returned `Connection` **never** contains the secret — only `secret_ref`
  (a Keychain item name like `conn-<id>`) is stored, and even that is internal.

### Per-kind `params`

`params` is a free-form JSON object; the builder reads only the keys it needs.
**Missing required params don't fail** — the profile falls back to a plain login
shell (`$SHELL -l`) so a `first_command` can still run. Validation happens at
create/update via `validate_params`, which simply tries to build the command.

| Kind | Required | Optional `params` | Client binary | Secret channel |
|---|---|---|---|---|
| `ssh` | `host` | `port`, `user`, `identity_file`, `jump` (`ProxyJump`) | `ssh` | ssh-agent / key — no password in argv |
| `mysql` | `host` | `port`, `user`, `db`, `jump`, `identity_file` | `mysql` | `MYSQL_PWD` env |
| `redis` | `host` | `port`, `db` (→ `-n`), `jump`, `identity_file` | `redis-cli` | `REDISCLI_AUTH` env |
| `mongodb` | `conn_string` | (`{secret}` placeholder inside the URI) | `mongosh` | substituted into the URI |
| `clickhouse` | `host` | `port`, `user`, `db`, `jump`, `identity_file` | `clickhouse-client` | `--password <x>` argv ⚠️ |
| `custom` | `command_template` | any `{placeholder}` keys + `{secret}` | parsed from template | `{secret}` substitution |

Notes:
- **SSH:** built as `ssh [-i identity] [-p port] [-J jump] user@host`. With no
  `host`, you get a login shell.
- **MongoDB:** you supply the whole connection string as `conn_string`. If it
  contains the literal token `{secret}`, the stored secret is substituted in
  (`mongodb://app:{secret}@host/db`); a `{secret}` reference with no stored secret
  is a real `400`. Atlas `mongodb+srv://…` URIs are fully supported.
- **ClickHouse (terminal kind):** `clickhouse-client` has **no** env or stdin
  password channel, so the password is unavoidably placed in argv. The builder
  returns `warn_argv = true` and the UI shows a warning; other people on the same
  machine can see it in the process list. Prefer key/agent paths where possible.
- **Custom:** the `command_template` is rendered by substituting every other
  `params` key as `{key}`, plus `{secret}` from the Keychain, then split with
  shell-words quoting. Example: `psql -h {host} -U {user} {db}` with
  `host`/`user`/`db` params, or
  `kubectl exec -it pod -- sh -c 'tail -f /var/log/app.log'`.

### Authentication: password vs. key

- **Password / secret** → stored in the macOS Keychain (`secret`, write-only).
  Injected per kind via env (`MYSQL_PWD`, `REDISCLI_AUTH`), URI substitution
  (Mongo), or argv (ClickHouse only). On *PATCH*, omit `secret` to keep the
  existing one; provide it to replace.
- **SSH key / agent** → set `identity_file` (a path to a private key on disk; a
  "Browse…" picker is in the form) or rely on your ssh-agent and `~/.ssh/config`.
  Otto shells out to the system `ssh`/`sftp`, so it honours **ssh-agent,
  `~/.ssh/config`, and `known_hosts`** automatically — there is no password
  prompt channel for SSH. The form hint reads *"Identity OR password — both
  optional."*
- Deleting a profile (`DELETE`) also deletes its Keychain item.

### Engine-only params (DB Explorer / native drivers)

The form persists two extra `params` blocks for the **DB-engine kinds** that are
consumed by the native Database Explorer / Kafka drivers, **not** by the terminal
builders above:

- **`params.tls`** — `{ mode: disabled|preferred|required, verify, ca_cert,
  client_cert, client_key, server_name }` for TLS/SSL to the database.
- **`params.ssh`** — a structured SSH-tunnel config `{ host, port?, user?,
  identity_file? }` (an `otto_ssh::SshTunnelConfig`) used by the warm-tunnel pool.

These are distinct from the flat `jump` / `identity_file` keys, which the
*terminal* builder uses to wrap a DB CLI through a bastion (see §5). The DB
Explorer doc covers `tls` / `params.ssh` in full.

---

## 3. Connection sessions (terminal)

`POST /connections/{id}/open` builds the per-kind command, spawns it as a PTY
session in a workspace, and stamps `last_opened_at` for recency ordering. The
session lives in the normal session manager (split panes, attach, restart) — see
the sessions docs.

- **Body:** `{ "title": null, "workspace_id": null }` (both optional). `title`
  defaults to the connection name. `workspace_id` is **required only for global
  connections** (which have no workspace of their own); a 400 is returned if it's
  missing for one.
- **`first_command`** is written into the PTY ~1.5s after spawn, followed by a
  newline — handy for `USE db; SHOW TABLES;` or `cd /var/log`.
- **SSH shell:** an interactive login on the remote host over the PTY — a real
  terminal, scrollback, resize, copy/paste, the lot.
- **DB CLIs:** `mysql` / `redis-cli` / `mongosh` / `clickhouse-client` open
  interactively against the target; you type SQL/commands directly.
- The Connections page also offers **"Open beside"** — the new terminal lands in a
  split pane next to the current session instead of replacing the active tab.

### Test connection

`POST /connections/{id}/test` runs a **headless** probe (10s timeout) and returns
`TestConnectionResp { ok, latency_ms, message, warn_argv }`:

| Kind | Probe |
|---|---|
| `ssh` | `ssh -o BatchMode=yes -o ConnectTimeout=5 … <target> exit` |
| `mysql` / `clickhouse` | client + `SELECT 1;` on stdin |
| `redis` | client + `PING` on stdin |
| `mongodb` | `mongosh --quiet --eval "db.runCommand({ping:1})"` |
| `custom` | runs the command as-is |

On failure the **first non-empty stderr line** is surfaced, but credentials are
**redacted** first: `scheme://user:pass@host` userinfo is stripped and
`--password <x>` / `-p <x>` / `--password=<x>` argv is replaced with
`<redacted>`. For DB-kind connections, when the server has wired a `DbTester`
(the Database Explorer's warm-tunnel pool), the probe is routed through the native
driver path and **reuses a cached `ssh -L` forward** instead of spawning a fresh
`ssh -J` child — a second test on an already-open connection skips the SSH
handshake entirely. SSH and Custom always use the CLI subprocess path.

### Pinning & recency

`PATCH /connections/{id}/pin` `{pinned: bool}` toggles a flag that surfaces the
connection in a **Pinned** group above **Recent**. The sidebar's "Recent" group is
pinned-first, then most-recently-opened (`last_opened_at`), capped to a fixed
count.

---

## 4. SSH tunnels (`crates/otto-ssh`)

Otto has **no embedded SSH stack** — every tunnel shells out to the system `ssh`,
so it inherits ssh-agent, `~/.ssh/config`, and `known_hosts`. There are two
forwarding modes, sharing the same liveness/teardown machinery. Both add these
baseline `ssh` options: `-N` (no remote command), `BatchMode=yes`
(non-interactive auth), `ExitOnForwardFailure=yes` (fail fast if the local port
can't bind), `ConnectTimeout=10`, and `ServerAliveInterval=15` (keep-alive). The
tunnel `ssh` child is **killed on drop**, and `open*()` waits up to **12s** for the
local port to accept a TCP connection before returning (or surfaces ssh's stderr).

### Local forward — `ssh -N -L`

```
ssh -N -L 127.0.0.1:<ephemeral>:<remote_host>:<remote_port> user@bastion
```

A fixed local port maps to **one** remote endpoint; the driver connects to the
local end. Used for the **single-endpoint** database engines: **MySQL, Redis,
ClickHouse**. (`SshTunnel::open(cfg, remote_host, remote_port)`.)

### Dynamic SOCKS5 — `ssh -N -D`

```
ssh -N -D 127.0.0.1:<ephemeral> user@bastion
```

A local SOCKS5 proxy through which a SOCKS-aware client **resolves and dials
arbitrary hosts** from the SSH server's network. Used for:

- **MongoDB (Atlas / replica sets).** A `mongodb+srv://` URI triggers an SRV DNS
  lookup that returns **multiple** member hostnames the driver must each reach,
  and the per-host SNI/TLS handshake must match those private names — a single
  `-L` forward (one fixed `local:remote` pair) **cannot** represent that topology.
  SOCKS5 lets the driver resolve + connect to every member through the proxy as if
  it were inside the VPC, with SNI preserved. **This is why Mongo/Atlas needs `-D`,
  not `-L`.**
- **Kafka brokers (MSK).** MSK advertises per-broker private DNS names that a
  single local forward likewise can't represent; the Kafka viewer uses the SOCKS
  proxy (with metadata/coordinator rewriting — see the brokers doc).

(`SshTunnel::open_socks(cfg)`.)

### Bastion / jump-host config

A tunnel config is `SshTunnelConfig { host, port (default 22), user,
identity_file? }`. In the **DB-engine form** this is the *"SSH tunnel (reach the
DB through a bastion)"* section: **Tunnel host**, **Port**, **Tunnel user**,
**Identity file** (with a Browse… picker). It is stored as `params.ssh` and used
by the Database Explorer / Kafka driver paths.

For a **terminal** DB session, the flat `jump` param instead wraps the CLI:

```
ssh -t [-i identity] <jump> -- mysql -h db.internal -P 3306 -u root mydb
```

i.e. the DB client runs *on the bastion* (`maybe_wrap_ssh_tunnel` in
`builders.rs`); the password still travels via env, never argv.

### `AllowTcpForwarding` gotcha

`-L` and `-D` forwarding only works if the bastion's `sshd` permits it. If
`/etc/ssh/sshd_config` sets `AllowTcpForwarding no` (the common hardened default
on locked-down jump hosts), the forward is refused and — because Otto passes
`ExitOnForwardFailure=yes` — `ssh` exits immediately. You'll see *"ssh tunnel
exited early"* / *"administratively prohibited: open failed"*. Fix it on the
bastion: `AllowTcpForwarding yes` (or `local`/`all`), then reload `sshd`.

---

## 5. SFTP file browser (`crates/otto-ssh/src/sftp.rs`)

Browse, read, and transfer files over an **SSH** connection's existing auth.
Otto drives the system **`sftp`** binary (`sftp -b -`, batch commands on stdin),
reusing the connection's keys/ssh-agent/`~/.ssh/config`/`ProxyJump` — there is
**no separate password**. Because the daemon runs on your Mac, `get`/`put`
read/write the **daemon host's real local disk**.

- **SSH only.** Every SFTP route requires `kind == ssh`; anything else → `400`
  (*"SFTP is only available for SSH connections"*). The browser button only shows
  on SSH rows.
- **Warm multiplexed connection.** Each browse session owns a private
  `ControlMaster` socket (`ControlPersist=60s`) under a unique temp dir, removed
  on drop — so the many small `sftp` invocations a browse session makes reuse one
  multiplexed connection, fast even through a bastion.

### UI flow

Click the **SFTP / "Browse files"** action on an SSH connection row. The browser
opens at the remote home directory (empty path ⇒ remote `pwd`, then list), with:

- A **breadcrumb** of the absolute path; click any segment to jump.
- A table of entries (Name / Size / Modified / Perms), directories navigable.
- **Open file** → fetches up to **1 MiB** of UTF-8 text inline (a `truncated`
  flag warns when the file was larger).
- **Download** → pick a local destination directory (on the daemon host) via the
  folder picker; toasts the saved path + byte count.
- **Upload** → pick a local file (on the daemon host); lands it in the current
  remote dir.
- mkdir / rename / remove operate on the current directory.

### Path handling & guards

- A leading `~` in a **local** path expands to the daemon user's `$HOME`.
- For **download**, the parent dir is created automatically; if the local path is
  an existing directory, the remote file's basename is used.
- **Control-character guard (local-RCE defence).** `sftp -b` is line-oriented and
  supports a `!cmd` local-shell escape, so a remote filename containing a newline
  could split the batch and smuggle a `!command` onto its own line. Every path is
  run through `quote_checked`, which **rejects any control character** (newline,
  CR, tab, NUL, …) before quoting — *"path contains a control character (rejected
  for safety)"*. No legitimate path needs one. Ordinary names (spaces, quotes,
  backslashes) are double-quoted/escaped and pass fine.
- Listings are parsed from `ls -la` longname output, tolerating command echoes,
  the `sftp>` prompt, and `total N` headers; symlinks are split into
  `name -> target`.
- `sftp` client errors (permission denied, no such file) surface as a
  `502 upstream` carrying the first stderr line.

---

## 6. Sidebar sections / grouping

Connections are organised into a **single global tree** of user-defined
`ConnectionSection`s, nestable via `parent_id`. The Connections page and the
Database Explorer **share the same section tree** (the historical `scope` query
param — `connections` / `db` — is accepted for backward compatibility but ignored;
both pages render one tree).

- **Create / rename / delete / reorder / reparent** via the section endpoints
  (§7). Deleting a section removes its sub-sections too; their connections fall
  back to **Ungrouped** (never deleted).
- **Drag a connection onto a section** to file it (`PATCH …/connections/{id}` with
  `section_id`); **drag a section onto another** to nest it (`…/move`). Reparenting
  rejects cycles (a section can't become its own descendant).
- Global (root-managed) connections are **not** assignable to a workspace section.
- A search box flattens the tree into a flat result list (matched by name /
  host-user / kind / section name).

---

## 7. API / contract reference

`docs/contracts/api.md` is authoritative; all paths are under `/api/v1`. Auth is
the connection's workspace role (global connections: root for mutations).

### Connection CRUD (#25–30)

| Method & path | Auth | Request → Response |
|---|---|---|
| `GET /workspaces/{id}/connections` | ws viewer | — → `Connection[]` (incl. globals; secret never present) |
| `POST /workspaces/{id}/connections` | ws editor | `UpsertConnectionReq` → `Connection` (created workspace-independent / global) |
| `PATCH /connections/{id}` | ws editor (global: root) | `UpsertConnectionReq` (PATCH semantics) → `Connection` |
| `PATCH /connections/{id}/pin` | ws editor (global: root) | `{pinned}` → `Connection` |
| `DELETE /connections/{id}` | ws editor (global: root) | — → 204 (deletes Keychain secret) |
| `POST /connections/{id}/open` | ws editor | `{title?, workspace_id?}` → `Session` |
| `POST /connections/{id}/test` | ws editor | — → `TestConnectionResp{ok, latency_ms, message, warn_argv}` |

When `connections.owner_private` (a daemon setting) is **on**, non-root users see
and may mutate only connections they created.

### SFTP — `/connections/{id}/sftp/*` (SSH only)

| Method & path | Auth | Request → Response |
|---|---|---|
| `GET …/sftp/list?path=` | ws viewer | — → `SftpListResp{path, entries: SftpEntry[]}` (empty path ⇒ `pwd` then list) |
| `GET …/sftp/read?path=` | ws viewer | — → `SftpReadResp{text, truncated}` (≤ 1 MiB UTF-8) |
| `POST …/sftp/download` | ws editor | `{remote_path, local_path}` → `SftpDownloadResp{local_path, bytes}` |
| `POST …/sftp/upload` | ws editor | `{local_path, remote_path}` → 200 |
| `POST …/sftp/mkdir` | ws editor | `{path}` → 200 |
| `POST …/sftp/remove` | ws editor | `{path, dir?}` → 200 (`dir:true` ⇒ `rmdir`, else `rm`) |
| `POST …/sftp/rename` | ws editor | `{from, to}` → 200 |

`SftpEntry { name, kind: "dir"|"file"|"symlink"|"other", size, mtime?, perms,
symlink_target? }`.

### Connection sections

| Method & path | Auth | Request → Response |
|---|---|---|
| `GET /workspaces/{id}/connection-sections` | ws viewer | — → `ConnectionSection[]` (the one global tree) |
| `POST /workspaces/{id}/connection-sections` | ws editor | `UpsertSectionReq{name, parent_id?, scope?}` → `ConnectionSection` |
| `POST /workspaces/{id}/connection-sections/reorder` | ws editor | `{ids:[…]}` → 204 |
| `PATCH /connection-sections/{id}` | ws editor | `{name}` → `ConnectionSection` |
| `DELETE /connection-sections/{id}` | ws editor | — → 204 |
| `POST /connection-sections/{id}/move` | ws editor | `{parent_id?}` (null = top-level) → `ConnectionSection` |

### Workspace MCP servers (`.mcp.json` entries)

Per-workspace, user-configured MCP servers. *Enabled* servers are merged into the
workspace's `.mcp.json` (alongside Otto's own managed entries) when an agent
session spawns there. Never auto-enabled: `enabled` defaults `false`, and a server
is only written once you flip it on **and** a session then spawns in the workspace.
`GET/POST /workspaces/{id}/mcp-servers`, `PATCH/DELETE /mcp-servers/{id}`
(`McpServer{name, command, args, env, enabled, …}`; `env` is plaintext for now —
keep long-lived secrets in your own MCP config until Keychain secret-refs land).

> Database-Explorer engine routes (`/connections/{id}/db/*`) reuse these same
> connection profiles — see [`./database-explorer.md`](./database-explorer.md).

---

## 8. Capabilities & limitations

- ✅ Five first-class kinds + a `custom` escape hatch for any CLI.
- ✅ One profile feeds three features: terminal session, Database Explorer, Kafka
  viewer — define host/auth/tunnel once.
- ✅ SSH tunnels via the system `ssh` (agent / config / known_hosts honoured),
  both `-L` and SOCKS5 `-D`.
- ✅ SFTP browse/read/transfer with a warm multiplexed connection and a hard
  control-char guard.
- ✅ `prod` / `read_only` write-gating (DB Explorer) with PATCH-safe semantics.
- ⚠️ **ClickHouse terminal password is in argv** (`warn_argv`) — `clickhouse-client`
  exposes no env/stdin password channel.
- ⚠️ SFTP transfers touch the **daemon host's** disk (your Mac), not the browser
  client's — intended, since the daemon is local, but worth knowing for remote/PWA
  access.
- ⚠️ SFTP text view is capped at **1 MiB**; larger files report `truncated` (use
  Download for the whole file). No in-place file editing.
- ⚠️ The Connections page lists only `ssh`/`custom`; DB kinds are managed in the
  Database Explorer.
- ⚠️ Requires the relevant client binaries on `PATH` (`ssh`, `sftp`, `mysql`,
  `redis-cli`, `mongosh`, `clickhouse-client`).

---

## 9. Security model

- **Secrets in the Keychain only.** The SQLite state DB stores an opaque
  `secret_ref` (`conn-<id>`); the secret itself lives in the macOS Keychain and is
  fetched only at open/test time. It is never serialized back to the client.
- **No password in argv** except `clickhouse-client` (flagged `warn_argv`);
  MySQL/Redis use env vars, Mongo substitutes into the URI, SSH uses keys/agent.
- **Test-connect error redaction** scrubs userinfo and `--password`/`-p` argv
  before surfacing stderr.
- **SFTP control-char guard** rejects newline/CR/etc. in any path to block
  `!command` local-shell injection from a hostile remote filename.
- **Loopback by default.** The daemon binds `127.0.0.1:7700`; tunnels and SOCKS
  proxies bind `127.0.0.1:<ephemeral>` too. Don't widen the listener casually.
- **RBAC.** Reads = ws Viewer (`Connections:View`); mutations/transfers = ws
  Editor (`Connections:Edit`); global connections are root-managed. See
  [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md).

---

## 10. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| **Test fails: "Access denied" / "authentication failed"** | Wrong secret, or absent for a kind that needs one. Re-enter the password (omitting it on PATCH keeps the old one). For SSH, ensure the key is in the agent or `identity_file` is correct. |
| **"failed to start mysql/mongosh/…"** | The client binary isn't on the daemon's `PATH`. Install it / fix `PATH` for the launchd environment. |
| **Tunnel: "ssh tunnel exited early" / "administratively prohibited: open failed"** | Bastion `sshd` has `AllowTcpForwarding no`. Enable forwarding on the bastion and reload `sshd`. Also check the jump host's reachability and your key. |
| **"ssh tunnel did not become ready within 12s"** | Bastion slow/unreachable, host-key prompt blocking `BatchMode`, or the remote endpoint isn't listening. Verify `ssh user@bastion` works non-interactively first. |
| **MongoDB `+srv` / Atlas won't connect through a bastion** | Atlas requires the **SOCKS5 (`-D`)** path, not a local `-L` forward — used automatically for Mongo. Ensure the bastion can resolve + reach the Atlas member hostnames and that `AllowTcpForwarding` is on. |
| **ClickHouse won't connect on 9000/9440** | Use the HTTP interface — port **8123** (plain) or **8443** (TLS). The native protocol ports aren't supported by the engine. |
| **SFTP: "path contains a control character (rejected for safety)"** | A filename held a control char; this is the injection guard. Rename the remote file from a real shell. |
| **SFTP: 400 "SFTP is only available for SSH connections"** | The connection isn't `kind == ssh`. SFTP is SSH-only. |
| **SFTP download "saved" but file is empty / wrong place** | Remember `get`/`put` hit the **daemon host's** disk; `~` expands to the daemon user's home. If the local path is an existing dir, the remote basename is used. |
| **"opening a global connection requires 'workspace_id'"** | A global (root-managed) connection has no workspace — pass `workspace_id` in the open body. |

---

## 11. Related docs

- [`./database-explorer.md`](./database-explorer.md) — native data access over the
  same connection profiles (schema tree, query, export, `tls`/`params.ssh`).
- [`./message-brokers.md`](./message-brokers.md) — the Kafka viewer, which reuses
  the SOCKS5 (`-D`) tunnel path for MSK.
- [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — per-feature roles
  (`Connections:View` / `Connections:Edit`) and `owner_private`.
- `docs/contracts/api.md` — authoritative endpoint, request, and response shapes.
