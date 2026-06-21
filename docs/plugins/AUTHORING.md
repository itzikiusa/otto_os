# Authoring an Otto Custom Plugin (runtime, out-of-process)

A **plugin** adds a new Otto app section — its own UI **and** backend — as an
**external sidecar process** you install at **runtime** (no app rebuild). Otto
supervises the process, reverse-proxies its HTTP, serves its iframe UI, and gates
it with RBAC. Plugins can be written in **any language**.

> Design: `docs/superpowers/specs/2026-06-21-runtime-plugins-design.md`. Two
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
  "health": "/health"               // health path on your server (default /health)
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

## 4. The host API (capabilities)

Call back with `Authorization: Bearer $OTTO_PLUGIN_TOKEN`:

| Method & path | Returns |
|---|---|
| `GET  $OTTO_HOST_API/repos` | `[{id,name,path,remote_url}]` (run git yourself on `path`) |
| `GET  $OTTO_HOST_API/jira/accounts` | `[{id,label,base_url,email}]` |
| `GET  $OTTO_HOST_API/jira/credentials?account=<id>` | `{base_url,email,token}` (call Jira yourself) |
| `POST $OTTO_HOST_API/agents/run` | `{prompt,cwd?,model?}` → `{text}` (runs claude) |

Store your own state under `$OTTO_PLUGIN_DATA_DIR` (files / sqlite — your choice).

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
loopback-only. Only install plugins you trust.

## 8. Verify

```
# enabled list (any authed user): GET /api/v1/plugins  -> [{slug,name,icon,has_ui}]
# unauth call:                    /api/v1/plugins/<slug>/health  -> 401
# without grant (non-root):       -> 403 ; with view grant / root -> 200 (proxied)
```

## Worked examples (`examples/plugins/`)

- **`team-performance`** (Node, zero-dependency): Jira stories-only by assignee vs.
  git delivery (done = last merge to `develop` via `merge-base --is-ancestor`),
  estimation-improvement target (default 10%), met/missed + concurrency. Runs with
  just `node`.
- **`dora-metrics`** (Rust sidecar): DORA metrics from git tags (`*deployed*`) +
  branch-merge classification, with agent bottleneck analysis. `exec` is
  `cargo run --release` — it compiles on first enable.
