# Custom Plugins (runtime, out-of-process)

Otto's **Custom Plugins** let you add a whole new app section — its own UI **and**
its own backend — by installing an **external sidecar process at runtime**, with
**no app rebuild**. Otto supervises the process, reverse-proxies its HTTP behind
auth + RBAC, serves its UI in an iframe, and gives it a scoped "host API" to call
back into Otto (repos, Jira credentials, agent runs). Plugins can be written in
**any language** that can serve HTTP on a loopback port.

> This is the **user/operator-facing** overview: what a plugin is, how to install
> and manage one, what it can and cannot do, and how access is gated. To *write*
> a plugin (package layout, manifest, SDK, the `postMessage` init handshake),
> see the authoring guide: **[`../plugins/AUTHORING.md`](../plugins/AUTHORING.md)**.
> The API contract is authoritative in
> **[`../contracts/api.md`](../contracts/api.md)** ("## Custom Plugins (runtime,
> out-of-process)").

---

## 1. Overview & where it lives

A plugin is **not** a compiled-in feature and **not** a Tauri/webview extension.
It is a separate program on disk that Otto runs as a child process when the
plugin is **enabled**, and forwards traffic to. Because everything happens at
runtime in `ottod`, you install, enable, disable, and remove plugins from a
running Otto — you never rebuild the app or its UI bundle.

```
Otto UI (iframe)  ──HTTP──▶  ottod  ──reverse-proxy──▶  plugin sidecar (127.0.0.1:<port>)
                              ▲                              │
                              └────── scoped host API ◀──────┘  (Bearer OTTO_PLUGIN_TOKEN)
```

| Concern | Where it lives |
|---|---|
| Supervisor + reverse-proxy + host-API + iframe server | `crates/otto-server/src/plugins.rs` |
| Sidecar construction (plugins home, host-API base URL) | `crates/ottod/src/main.rs` (`PluginManager::new`) |
| Plugins home (where installs are stored) | `$OTTO_PLUGINS_HOME`, default `~/otto-plugins/<slug>/` |
| Per-plugin writable state dir | `<data_dir>/plugins/<slug>/` (passed as `$OTTO_PLUGIN_DATA_DIR`) |
| Registry table (installed plugins) | `crates/otto-state/migrations/0062_runtime_plugins.sql` → `plugins` |
| Slug-keyed RBAC grants | `crates/otto-state/migrations/0061_plugin_runtime.sql` → `plugin_feature_grants` |
| RBAC enforcement (proxy branch) | `crates/otto-server/src/feature_guard.rs` (plugin branch) |
| Route policy (host/list/admin) | `crates/otto-server/src/policy.rs` |
| Capabilities surface (`slug → capability`) | `crates/otto-server/src/routes/grants.rs` |
| Management UI (root) | `ui/src/modules/settings/PluginsSettings.svelte` |
| Iframe host component | `ui/src/modules/plugins/PluginFrame.svelte` |
| Sidebar nav (RBAC-filtered) | `ui/src/shell/Rail.svelte`, `ui/src/shell/Navigator.svelte` |
| Enabled-plugin store | `ui/src/lib/stores/plugins.svelte.ts` |
| Shipped examples | `examples/plugins/team-performance/` (Node), `examples/plugins/dora-metrics/` (Rust) |

---

## 2. The plugin model

### 2.1 Out-of-process sidecar (any language)

A plugin is an HTTP server you write in whatever language you like — the shipped
examples are **Node** (`node server.js`) and **Rust** (`cargo run --release`).
Otto only requires that:

- The program binds **`127.0.0.1:$OTTO_PLUGIN_PORT`** (Otto picks a free loopback
  port and passes it in the environment).
- It implements a **health endpoint** (default `GET /health`) that returns `2xx`.

Otto never loads the plugin's code into its own process; the plugin lives in its
own OS process with its own dependencies and runtime. That is the core safety
property — a crashing or misbehaving plugin cannot take `ottod` down with it.

### 2.2 The supervisor

`PluginManager` (in `otto-server`) owns the lifecycle of enabled sidecars:

- On daemon startup it spawns **every enabled** plugin (`start_enabled`),
  best-effort, logging per-plugin failures.
- `spawn(plugin)` allocates a fresh free loopback port, creates the plugin's
  data dir (`<data_dir>/plugins/<slug>/`), and runs the manifest's `exec` argv
  **from the plugin's own directory** (`current_dir`) with these environment
  variables:

  | Env var | Meaning |
  |---|---|
  | `OTTO_PLUGIN_SLUG` | The plugin's slug (its identity). |
  | `OTTO_PLUGIN_PORT` | The loopback port the sidecar **must** bind. |
  | `OTTO_PLUGIN_TOKEN` | Bearer secret for the scoped host API. |
  | `OTTO_HOST_API` | Host-API base URL (`http://127.0.0.1:<ottod-port>/api/v1/plugin-host`). |
  | `OTTO_PLUGIN_DATA_DIR` | A writable directory for the plugin's own state/config. |

- After spawning, the supervisor polls the plugin's health path (up to ~3s, 20
  attempts × 150ms) so the UI sees a ready process — but a slow start does **not**
  fail the spawn.
- The child is launched with `kill_on_drop`, and `stop(slug)` force-kills and
  reaps it. **Disable** and **remove** both stop the process; if `ottod` exits,
  every sidecar is killed.

### 2.3 The reverse-proxy

When a plugin is enabled, Otto forwards
`ANY /api/v1/plugins/<slug>/<rest>` → the sidecar's `http://127.0.0.1:<port>/<rest>`.
The proxy:

- preserves the HTTP **method**, **query string**, **body** (up to **16 MiB**),
  and `Content-Type`;
- forwards the **caller's identity** as `X-Otto-User` (user id) and
  `X-Otto-User-Name` (display name), so the sidecar knows who is asking;
- returns the sidecar's status, content-type, and body verbatim;
- returns **`502 Bad Gateway`** if the plugin is not running (e.g. crashed or
  disabled mid-request).

The proxy runs **after** auth + RBAC have already authorized the request (see
§7), so the sidecar can trust that whoever reaches it has the right grant.

### 2.4 The iframe UI

If the manifest declares a `ui` directory, Otto serves its static assets at
`GET /plugins/<slug>/ui/` (and `…/ui/<path>`), root-mounted and **public static**
(path-traversal-guarded — assets are contained within the plugin's `ui` dir).
The Svelte shell mounts that URL in an **iframe** (`PluginFrame.svelte`). After
the iframe loads, the parent hands it everything it needs via a `postMessage`
`otto:init` event:

```js
{ type: 'otto:init',
  slug,
  apiBase: '<baseUrl>/api/v1/plugins/<slug>',  // call your backend through here
  token:   '<the user's bearer>',              // for the gated proxy calls
  theme:   { '--bg', '--text', '--text-dim', '--accent', '--border' } }
```

So the iframe's data calls go to `${apiBase}/<rest>` → through the RBAC-gated
reverse-proxy → the sidecar. The theme vars let the plugin match Otto's
light/dark theme. (The handshake details live in
[`../plugins/AUTHORING.md`](../plugins/AUTHORING.md) §5.)

### 2.5 The scoped host API

The sidecar talks **back** to Otto through a small, fixed surface at
`$OTTO_HOST_API` (`…/api/v1/plugin-host/*`), authenticating with its
`OTTO_PLUGIN_TOKEN`. This is how a plugin gets repos, Jira credentials, or runs
an agent — without Otto exposing its whole internal API. See §6.

---

## 3. Installing & managing plugins

Management is **root-only** and lives in **Settings → Plugins**
(`PluginsSettings.svelte`). The page lets you:

- **Install** from a **local folder path** (with a Browse… folder picker rooted
  at `~/otto-plugins`) or a **git URL**.
- **Enable** / **Disable** each installed plugin (a toggle that spawns/stops the
  sidecar).
- **Remove** a plugin (with a confirm dialog; the plugin **files on disk are
  kept** — only the registry row is dropped).

The table shows each plugin's name+icon, source path, slug, version, and an
`enabled`/`disabled` badge.

### Lifecycle

| Action | What happens |
|---|---|
| **Install** | Resolve the source. A local path is **copied** into `~/otto-plugins/<slug>/` (skipping `.git`, `node_modules`, `target`); a git URL is shallow-**cloned** (`git clone --depth 1`). Otto reads `otto-plugin.json`, validates the slug, and registers the plugin **disabled**. Audited as `plugin.installed`. |
| **Enable** | Flip `enabled=true` and **spawn** the sidecar. If the process won't start, the enable is rolled back (`enabled=false`) and an error returned. The section appears in the sidebar immediately for users with a grant — **no rebuild**. |
| **Disable** | Flip `enabled=false` and **stop** (kill) the sidecar. The nav entry disappears. |
| **Remove** | Stop the sidecar and **delete the registry row**. Files under `~/otto-plugins/<slug>` are left on disk (re-install from the same path to restore). Audited as `plugin.removed`. |

You can also drive these from the API directly (see §9). New plugins are
**disabled by default** and only **root** can install/enable/disable/remove.

---

## 4. The manifest (`otto-plugin.json`)

Every plugin ships an `otto-plugin.json` at its root. Summary of the fields Otto
reads (`PluginManifestFile` in `plugins.rs`):

| Field | Required | Meaning |
|---|---|---|
| `slug` | yes | Identity for crate/permission/URL. Must match `^[a-z][a-z0-9-]*$`, ≤ 64 chars. |
| `name` | yes | Display name in the sidebar and settings table. |
| `icon` | no | Icon name (default `box`). |
| `version` | no | Shown in the settings table. |
| `description` | no | Free text. |
| `exec` | yes | argv to launch the sidecar, run from the plugin dir; **must not be empty**. Must bind `$OTTO_PLUGIN_PORT`. |
| `ui` | no | Iframe-assets dir (relative; must contain `index.html`). Omit for a **backend-only** plugin (no nav entry UI). |
| `health` | no | Health path Otto polls on spawn (default `/health`). |

> The manifest, package layout, and SDK helpers are documented in full in
> **[`../plugins/AUTHORING.md`](../plugins/AUTHORING.md)** §1. This page only
> summarizes the fields an operator needs to recognize.

---

## 5. What a plugin can — and cannot — do

The **scoped host API** is the *entire* surface a plugin can call back into Otto
through. It is authenticated by the sidecar's `OTTO_PLUGIN_TOKEN` (not user
auth), and validated **per handler** against the `plugins` registry (the token
must match an **enabled** plugin):

| Method & path (under `$OTTO_HOST_API`) | Returns / does |
|---|---|
| `GET  /plugin-host/repos` | `[{id, name, path, remote_url}]` — every Git repo Otto knows about. The plugin runs git itself against `path`. |
| `GET  /plugin-host/jira/accounts` | `[{id, label, base_url, email}]` — configured Jira accounts (no tokens). |
| `GET  /plugin-host/jira/credentials?account=<id>` | `{base_url, email, token}` — the Jira **token** for one account, so the plugin can call Jira directly. |
| `POST /plugin-host/agents/run` | `{prompt, cwd?, model?}` → `{text}` — runs a `claude` agent (180s timeout) and returns its text. |

**A plugin CAN:** serve its own UI + backend; read the repo list and run git on
those paths; read Jira accounts and fetch a Jira account's credentials; run a
one-shot agent prompt; persist its own state under `$OTTO_PLUGIN_DATA_DIR`
(files, SQLite, whatever it wants); know the calling user via `X-Otto-User`.

**A plugin CANNOT:** reach any other Otto endpoint (the host API is exactly the
four routes above — there is no generic passthrough); touch the SQLite state DB,
the Keychain, sessions, or other users' data; receive a **share/guest token**
(those are scoped to a single session and never reach a plugin); listen on a
non-loopback interface (it is spawned on `127.0.0.1` only); or run while
disabled (its token stops authenticating the moment it is disabled or removed).

> Note: a plugin is **trusted, user-installed local code** — fetching Jira
> credentials and running agents are real capabilities. Treat installing a
> plugin the way you would treat running any local binary. See §11.

---

## 6. Slug-keyed RBAC

Plugins are gated by a **string-keyed permission axis** parallel to Otto's
built-in `Feature` enum, keyed on the plugin's **slug** (table
`plugin_feature_grants`, migration `0061`). The capability ladder is the same as
everywhere else in Otto: **`none` < `view` < `edit` < `admin`**, where `none`
means simply *no grant row*.

How it is enforced (the **plugin branch** in `feature_guard.rs`, evaluated
before the generic policy table):

- Every `/api/v1/plugins/<slug>/*` (proxy) request requires a grant on `<slug>`:
  **`GET` ⇒ `view`, any other method ⇒ `edit`**.
- **Root bypasses** — root has effective `admin` on every plugin.
- The sidebar nav entry for a plugin shows only if the user has **≥ view**. The
  UI reads each plugin's `slug → capability` from `/auth/capabilities`
  (`canPlugin(slug, 'view')`) to filter `Rail.svelte` / `Navigator.svelte`.
- New plugins are **root-only** until you grant access.

Granting (root): **Settings → Users**, or via the API:

```
GET /api/v1/users/{id}/plugin-grants   → { grants: [{feature:"<slug>", capability:"view|edit|admin"}] }
PUT /api/v1/users/{id}/plugin-grants     { grants: [{feature:"<slug>", capability:"view|edit|admin"}] }
```

The `feature` field carries the **plugin slug** (granting before a plugin is
installed is harmless — the row is simply unused). A `PUT` atomically replaces
the user's plugin grants and writes a `plugin_grant.changed` audit entry
(`{old, new}`); it 404s if the target user doesn't exist.

The two exemptions: `GET /plugins` (the enabled-plugins **list** for the
sidebar) is allowed for any authed member (the UI then filters by grant), and
the host API (`/plugin-host/*`) is sidecar-token authed, not user-gated.

---

## 7. Shipped examples (`examples/plugins/`)

Two complete reference plugins ship in the repo; install either from its folder
(`~/otto_os/examples/plugins/<name>`) to see the whole flow end-to-end.

### `team-performance` (Node, zero-dependency)

```json
{ "slug": "team-performance", "name": "Team Performance", "icon": "gauge",
  "exec": ["node", "server.js"], "ui": "ui", "health": "/health" }
```

Per-assignee **Jira story throughput vs. git delivery** ("done" = the last merge
to `develop`, detected via `git merge-base --is-ancestor`), plus an AI-era
**estimation-improvement tracker** (default 10% target, met/missed +
concurrency). Runs with just `node` — **no `npm install`**. Files:
`server.js`, `otto-plugin.json`, `ui/index.html`.

### `dora-metrics` (Rust sidecar)

```json
{ "slug": "dora-metrics", "name": "DORA Metrics", "icon": "gauge",
  "exec": ["cargo", "run", "--release", "--quiet"], "ui": "ui", "health": "/health" }
```

**DORA delivery metrics** computed from git tags (`*deployed*` = a deploy) +
branch-merge classification (hotfix/release/feature → `develop`), with **agent
bottleneck analysis** via `POST /plugin-host/agents/run`. Because `exec` is
`cargo run`, it **compiles on first enable** (the initial enable can take a
while). Files: `Cargo.toml`, `src/main.rs`, `otto-plugin.json`, `ui/index.html`.

---

## 8. API / contract reference

`docs/contracts/api.md` is authoritative. All paths are under `/api/v1`.

### Management (root)

| Method & path | Notes |
|---|---|
| `GET /plugin-admin` | Full installed-plugin records (no secrets). Root only. |
| `POST /plugin-admin/install` | `{source}` = local path **or** git URL → installs into the plugins home (**disabled**). Root only. |
| `POST /plugin-admin/{slug}/enable` | Enable + spawn the sidecar (rolls back if it won't start). Root only. |
| `POST /plugin-admin/{slug}/disable` | Disable + stop the sidecar (`204`). Root only. |
| `DELETE /plugin-admin/{slug}` | Stop + unregister; **files kept** (`204`). Root only. |

### Runtime (gated)

| Method & path | Auth |
|---|---|
| `GET /plugins` | Any member (enabled list `[{slug,name,icon,has_ui}]`; UI filters by grant). |
| `ANY /plugins/{slug}` · `ANY /plugins/{slug}/{*rest}` | Plugin `<slug>` grant (`GET`=view, else=edit); root bypass. Reverse-proxied to the sidecar. |
| `GET /plugins/{slug}/ui` · `GET /plugins/{slug}/ui/{*path}` | Public static (iframe assets). |

### Host API (sidecar token, not user auth)

| Method & path | Returns |
|---|---|
| `GET /plugin-host/repos` | `[{id,name,path,remote_url}]` |
| `GET /plugin-host/jira/accounts` | `[{id,label,base_url,email}]` |
| `GET /plugin-host/jira/credentials?account=<id>` | `{base_url,email,token}` |
| `POST /plugin-host/agents/run` | `{prompt,cwd?,model?}` → `{text}` |

### Grants

| Method & path | Notes |
|---|---|
| `GET /users/{id}/plugin-grants` | A user's per-slug grants. Root only. |
| `PUT /users/{id}/plugin-grants` | Atomically replace; audited `plugin_grant.changed`. Root only. |

---

## 9. Capabilities & limitations

- **No rebuild to add a feature.** Install → enable → it appears in the sidebar.
  This is the whole point: a plugin is data + a child process, not compiled code.
- **Any language.** As long as it serves HTTP on `$OTTO_PLUGIN_PORT` and answers
  the health check.
- **Backend-only plugins** are allowed (omit `ui`): they still get a proxied API
  and host-API access, just no nav entry/iframe.
- **Crash isolation:** a sidecar runs in its own process; if it dies, proxied
  calls return `502` but `ottod` and the rest of the app keep working.

### ⚠️ Installed-app rebuild caveat (read this)

The currently **installed 0.1.0 app does not yet expose plugins** — Settings →
Plugins and the runtime plugin routes only appear after the app is **rebuilt and
redeployed** with the plugin code. If you are running the shipped 0.1.0 build and
don't see the **Plugins** settings tab, that is expected: rebuild + redeploy Otto
(sidecar copy + Tauri build + reinstall) before plugins are available. The
feature works in `cargo run -p ottod` / `npm run dev` against this tree today.

---

## 10. Security

- **Process isolation.** Plugins never run in `ottod`'s address space. They are
  separate OS processes spawned on **loopback only**, killed on disable/remove
  and on daemon exit.
- **Scoped host API, not a passthrough.** The sidecar can reach **exactly four**
  host routes (repos, Jira accounts, Jira credentials, agent run) — nothing
  else. Each is validated against the sidecar's `OTTO_PLUGIN_TOKEN`, which only
  authenticates while the plugin is **enabled**.
- **RBAC on every proxied call.** Per-slug grants gate access (`GET`=view,
  else=edit); root bypasses; the nav entry hides without ≥ view. Share/guest
  tokens (scoped to one session) never reach a plugin.
- **Admin install.** Install/enable/disable/remove are root-only and audited
  (`plugin.installed`, `plugin.removed`). New plugins are disabled by default.
- **Asset path containment.** The iframe asset server canonicalizes paths and
  refuses anything outside the plugin's `ui` dir (no directory traversal).
- **Trust model.** A plugin is **trusted, user-installed local code** with real
  capabilities (it can read repo paths, fetch Jira tokens, and run agents). Only
  install plugins you trust — installing one is comparable to running any local
  binary on your machine.

---

## 11. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| No **Plugins** tab in Settings | Installed 0.1.0 build predates plugins — rebuild + redeploy (§9 caveat). |
| Enable fails with a spawn error | Bad `exec` argv, missing runtime (`node`/`cargo`), or the sidecar didn't bind `$OTTO_PLUGIN_PORT`. The enable is rolled back; check the daemon log (`plugins: spawn failed`). |
| Plugin shows but calls return **502** | Sidecar not running (crashed, slow start, or disabled). Re-enable; verify it binds the port and answers `GET /health` with 2xx. |
| First enable of `dora-metrics` hangs | `cargo run` is **compiling** on first enable — wait for the build. |
| Nav entry missing for a non-root user | No grant — give them ≥ `view` on the slug in Settings → Users. |
| Proxied call returns **403** | Caller lacks the needed grant (`GET`=view, write=edit) on `<slug>`. |
| Host-API call returns **401** | Missing/invalid `OTTO_PLUGIN_TOKEN`, or the plugin is disabled (token only authenticates while enabled). |
| Iframe loads but stays blank | The UI never handled `otto:init` (`apiBase`/`token`), or `ui` points at a dir without `index.html`. See [`../plugins/AUTHORING.md`](../plugins/AUTHORING.md) §5. |
| Install from local path "did nothing" | Local installs **copy** into `~/otto-plugins/<slug>`; edits to the original folder won't take effect — re-install, or point the source at the home copy. |
| "Could not reach the daemon" in Settings → Plugins | `ottod` briefly restarted (e.g. after an app update). Click **Retry**. |

---

## 12. Related docs

- **[`../plugins/AUTHORING.md`](../plugins/AUTHORING.md)** — how to *write* a
  plugin: package layout, manifest, the SDK, the iframe `otto:init` handshake,
  and the two worked examples in depth. **Start here to build one.**
- **[`./daemon-http-api.md`](./daemon-http-api.md)** — the daemon's HTTP/WS API,
  auth, tokens, and how to drive `ottod` over the wire.
- **[`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md)** — the multi-user roles,
  capability ladder (`none < view < edit < admin`), per-feature grants, and
  sharing model the plugin RBAC axis parallels.
- **[`../contracts/api.md`](../contracts/api.md)** — authoritative API contract
  ("## Custom Plugins (runtime, out-of-process)" + the grants section).
- Design note: `docs/superpowers/specs/2026-06-21-runtime-plugins-design.md`.
