# Authoring an Otto Custom Plugin (runtime, out-of-process)

A **plugin** adds a new Otto app section — its own UI **and** backend — as an
**external sidecar process** you install at **runtime** (no app rebuild). Otto
supervises the process, reverse-proxies its HTTP, serves its iframe UI, and gates
it with RBAC. Plugins can be written in **any language**.

> Two
> complete reference plugins ship under `examples/plugins/` — `dora-metrics`
> (Rust) and `team-performance` (Node) — read them alongside this guide.

## 1. Package layout

```
~/otto-plugins/<slug>/            (a git repo or a local folder)
  otto-plugin.json                manifest
  <your server>                   binds $OTTO_PLUGIN_PORT, serves your API + /health
  ui/                             static iframe assets (index.html) — served by Otto
```

`<slug>` must match `^[a-z][a-z0-9-]*$` and is the crate/permission/URL identity.

### `otto-plugin.json`

```json
{
  "slug": "team-performance",
  "name": "Team Performance",
  "icon": "gauge",
  "version": "0.1.0",
  "description": "…",
  "exec": ["node", "server.js"],   // run from the plugin dir; bind $OTTO_PLUGIN_PORT
  "ui": "ui",                       // iframe assets dir (index.html). Omit for backend-only.
  "health": "/health",              // health path on your server (default /health)
  "agent_tools": false              // optional; see "Agent runs" below (default false)
}
```

## 2. Install / lifecycle (runtime)

In the app: **Settings → Plugins** → install from a **local path** or **git URL**,
then **Enable** (spawns the sidecar) / **Disable** (stops it) / **Remove**. Or via API:

```
POST   /api/v1/plugin-admin/install        {"source": "<path-or-git-url>"}   # root
POST   /api/v1/plugin-admin/{slug}/enable                                    # root
POST   /api/v1/plugin-admin/{slug}/disable                                   # root
DELETE /api/v1/plugin-admin/{slug}                                           # root
GET    /api/v1/plugin-admin                                                  # root (list)
```

Installing copies/clones into `~/otto-plugins/<slug>` (disabled). Enabling spawns
the sidecar; the section appears in the sidebar immediately. **No rebuild.**

## 3. The sidecar

Otto spawns your `exec` from the plugin dir with these env vars:

| Env | Meaning |
|---|---|
| `OTTO_PLUGIN_PORT` | **Bind your HTTP server to `127.0.0.1:$OTTO_PLUGIN_PORT`.** |
| `OTTO_PLUGIN_TOKEN` | Bearer token to call the host API. |
| `OTTO_HOST_API` | Host-API base URL (`…/api/v1/plugin-host`). |
| `OTTO_PLUGIN_SLUG` | Your slug. |
| `OTTO_PLUGIN_DATA_DIR` | A writable dir for your state/config. |

Otto reverse-proxies `GET/POST/… /api/v1/plugins/<slug>/<rest>` → your server's
`/<rest>`. Implement at least `GET /health` (Otto health-checks it on spawn). The
caller's identity is forwarded as `X-Otto-User` / `X-Otto-User-Name`.

**Authenticate the proxy.** Every proxied request carries
`X-Otto-Plugin-Token: $OTTO_PLUGIN_TOKEN`. Any other local process can reach your
loopback port, so answer **401** to every request without that exact header
(compare in constant time; refuse everything when the env var is empty) — except
`/health`, which Otto probes without it. Both reference plugins do this
(`pluginTokenOk` in `team-performance/server.js`, `routes::authorized` in
`dora-metrics`). Proxied
response bodies are buffered up to 64 MiB; a declared or streamed body exceeding
that bound returns 502. The normal upstream status/content type and request
deadline are preserved. This API response limit does not apply to Otto's separate
static iframe asset route. Paginate large reports instead of returning unlimited
JSON from a sidecar.

## 4. The host API (capabilities)

Call back with `Authorization: Bearer $OTTO_PLUGIN_TOKEN`:

| Method & path | Returns |
|---|---|
| `GET  $OTTO_HOST_API/repos` | `[{id,name,path,remote_url}]` (run git yourself on `path`) |
| `GET  $OTTO_HOST_API/jira/accounts` | `[{id,label,base_url,email}]` |
| `GET  $OTTO_HOST_API/jira/credentials?account=<id>` | `{base_url,email,token}` (call Jira yourself) |
| `POST $OTTO_HOST_API/agents/run` | `{prompt,provider?,model?,cwd?}` → `{text}` (one agent turn; `provider` is `claude` or `codex`) |

Store your own state under `$OTTO_PLUGIN_DATA_DIR` (files / sqlite — your choice).

### Agent runs

A plugin's prompt usually carries text it collected elsewhere (Jira issues, commit
messages…), which anyone who can write a ticket controls. So by default
`agents/run` is **text in, text out**: Claude runs with **no tools and no MCP**,
Seatbelt-confined, in a throwaway scratch directory (codex: `--sandbox read-only`
in the same kind of scratch dir); `cwd` is ignored. Fence third-party text in the
prompt as data anyway.

A plugin that really needs the agent to read or edit files sets
`"agent_tools": true` in its manifest. Then the turn has full tools, and `cwd`
(default: `$OTTO_PLUGIN_DATA_DIR`) must be inside that data dir or a repo
registered in Otto — any other path, and the daemon's own working directory,
answers 400. Otto writes an audit row (`plugin.agent_run`: provider, tools, cwd,
prompt size) for every run either way.

## 5. The UI (iframe)

Put static assets in `ui/` (an `index.html`). Otto serves them at
`/plugins/<slug>/ui/` and embeds them in an iframe. After load, the parent sends a
`postMessage` the iframe should handle:

```js
window.addEventListener('message', (ev) => {
  const m = ev.data;
  if (m?.type !== 'otto:init') return;
  // m.apiBase  -> call your backend: fetch(`${m.apiBase}/metrics`, {headers:{Authorization:`Bearer ${m.token}`}})
  // m.token    -> the user's bearer (for the gated /api/v1/plugins/<slug>/* calls)
  // m.theme    -> { '--bg', '--text', '--accent', ... } CSS vars to match Otto
  // m.scheme   -> 'light' | 'dark'
});
// Otto re-sends the vars when the person changes light/dark, theme or accent:
window.addEventListener('message', (ev) => {
  if (ev.data?.type === 'otto:theme') applyTheme(ev.data.theme, ev.data.scheme);
});
```

Your iframe's API calls go to `${apiBase}/<rest>` = `/api/v1/plugins/<slug>/<rest>`,
which Otto authorizes (RBAC) and proxies to your sidecar.

## 6. RBAC & sharing

- Every `/api/v1/plugins/<slug>/*` request is gated by a **per-slug** permission:
  **GET ⇒ view, other methods ⇒ edit**; **root bypasses**. The sidebar entry shows
  only if the user has ≥ view.
- Grant non-root users in **Settings → Users**, or:
  `PUT /api/v1/users/{id}/plugin-grants` with
  `{ "grants": [ { "feature": "<slug>", "capability": "view|edit|admin" } ] }`.
- New plugins are **root-only** until granted. Share/guest tokens never reach
  plugins (scoped to a single session).

## 7. Trust model

Plugins are **trusted, user-installed local processes** with a scoped token to the
host API (repo paths, Jira creds, agent runs). Install is an admin action; plugins
are **disabled by default**; disable/remove kills the process; everything is
loopback-only. Only install plugins you trust. The data a plugin feeds an agent is
NOT trusted: agent runs are tool-less unless the manifest opts in (above), and the
sidecar only answers Otto's proxy (`X-Otto-Plugin-Token`).

## 8. Verify

```
# enabled list (any authed user): GET /api/v1/plugins  -> [{slug,name,icon,has_ui}]
# unauth call:                    /api/v1/plugins/<slug>/health  -> 401
# without grant (non-root):       -> 403 ; with view grant / root -> 200 (proxied)
```

## Worked examples (`examples/plugins/`)

- **`team-performance`** (Node, zero-dependency): scans every issue of a Jira
  project (paginated + incremental, async scan job), splits each task's
  changelog time into design vs implementation business days (configurable
  status→phase map), correlates git delivery with a one-pass repo index,
  baselines "how long it should have taken" per (type, points) bucket,
  predicts timelines for open tasks and tracks per-dev goals across scans.
  Ships unit + sidecar-E2E tests (`node --test`). Runs with just `node`.
- **`dora-metrics`** (Rust sidecar): the four DORA keys from git signals
  (deploy = `*deploy*` tag, configurable; hotfix/release/feature merges) with
  Elite/High/Medium/Low tiers, weekly trend charts, deltas, a deterministic
  suggestions engine, and AI bottleneck analysis. Ships engine unit tests +
  a fixture-repo integration test (`cargo test`). `exec` is
  `cargo run --release` — it compiles on first enable.
