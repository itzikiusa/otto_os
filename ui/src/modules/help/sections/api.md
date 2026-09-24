---
id: api
title: API client
group: Infrastructure
route: api
summary: Build, send and save HTTP, SSE, WebSocket and gRPC requests from Otto's daemon, with collections, environments, scripts and automated checks.
---
## What it's for

The API page is a Postman-style workbench inside Otto. You build a request, send it, and read the status, timing, headers and body. You can save requests into collections, keep base URLs and tokens in **environments** as `{{variables}}`, and chain saved requests into **automations** that check each response.

Requests go out from Otto's daemon, not from the web view, so browser CORS rules don't apply. Every URL passes Otto's network guard first: localhost, private networks and cloud metadata addresses are blocked unless a workspace admin allows them.

Everything is scoped to the current workspace: collections, requests, environments, history, automations and the cookie jar.

## Getting started

1. Open **API** in the sidebar (Infrastructure group).
2. A workspace with nothing in it opens on **Create your first request**. Choose one of:
   - **New request** — a blank request tab with the URL field focused.
   - **Import from curl, Postman or OpenAPI…** — see Import below.
   - **Try an example request** — opens `GET https://httpbin.org/json`.
3. Type a URL, or paste a whole `curl …` command into the URL field (it is imported automatically).
4. Press **Send** or ⌘↵. The response appears in the pane below.
5. Press **Save** (⌘S). A new request asks for a name and where to save it: any collection or folder, **No collection**, or a new collection.
6. To use variables, open **Environment ▾** in the page header → **New environment…**, add `base_url` and friends, then write `{{base_url}}/users` in the URL.

## Everything it can do

**Layout**
- Page header: **Environment ▾** (always visible), **Import…**, **Sync with Git…** and **New request**.
- Left pane with three views: **Collections**, **Automations** and **History**. The choice is remembered on this device.
- Main area: request tabs, each with the editor above the response. Environments and an open automation appear as an extra tab.
- Tabs show the method, the name (or `METHOD /path`) and a dot while there are unsaved changes. Open tabs survive an app restart on this device.
- Drag the dividers to resize the sidebar and the editor/response split; double-click a divider to reset it.
- On a phone, the list and the editor are two screens with a back button.
- The same editor also lives in the right panel's **API** tab, sharing the same draft as the page.

**Request types**
- **HTTP** — `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`. REST and GraphQL.
- **SSE** — keeps the connection open and streams server-sent events.
- **WebSocket** — a two-way connection with a message box. Supports query parameters, headers, auth and a timeout; no body, SSH tunnel, redirects or TLS skipping.
- **gRPC** — unary or server-streaming calls. Upload a `.proto` file, or choose **Load from server** to use server reflection. Pick the method from the list; **Message** holds the JSON request and **Metadata** holds headers. The button reads **Invoke**.

**Request editor tabs** (HTTP and SSE)
- **Params** and **Headers** — key/value rows with a tick box to skip a row. Headers autocomplete common names and values.
- **Body** — No body, JSON, Text / XML / HTML (with a format picker), Form (URL-encoded), Multipart form (files) and GraphQL (query plus JSON variables, with **Load schema from server**). **Format** pretty-prints JSON, XML and HTML. Content-Type follows the body type unless you typed your own.
- **Auth** — No auth, Bearer token, Basic auth, API key (as a header or a query parameter) and OAuth 2.0. OAuth supports the browser flow (authorization code + PKCE), client credentials, username and password, and refresh token. **Get token** fetches the token from the daemon.
- **Scripts** — Postman-style JavaScript with a `pm` object. **Before sending** can change the request and set variables; **After the response (tests)** can read `pm.response` and declare `pm.test(…)` checks. Scripts run for HTTP only.
- **Docs** — Markdown notes with a live preview, saved with the request and exported to OpenAPI and Postman.
- **Settings** — timeout (empty means 60 seconds), follow redirects (up to 10), verify TLS certificates, and **Send through** an SSH tunnel from your Connections for IP-restricted APIs. The **Allow private addresses** workspace switch lives here too.

**Variables**
- Under the URL, every `{{variable}}` the request uses is listed with what it resolves to: a value, *secret*, *generated* or *not set*. Unset variables are highlighted and sent as typed.
- Built-in generated values: `{{$guid}}` / `{{$randomUUID}}`, `{{$timestamp}}`, `{{$isoTimestamp}}` and `{{$randomInt}}` (0–999).
- **Session variables…** (⋯ menu) are temporary values that override the environment and are cleared when Otto restarts. Scripts set them with `pm.environment.set()`.

**Environments**
- Switch the active one from **Environment ▾**, or open **Manage environments…** to edit them in the main area.
- A variables table with a **Secret** toggle per row. Secret values move to the macOS Keychain on save and show masked afterwards.
- ⋯ on an environment: **Rename…**, **Secure plaintext secrets…** (moves every plaintext credential in the workspace into the Keychain) and **Delete…**.

**Response viewer**
- Status, time and size chips, plus **Copy** for the body as shown.
- **Body** in **Pretty**, **Raw**, **Tree** and **Preview**. Pretty takes a JSONPath filter such as `$.data[0].id`; Tree has **Find a key or value**, and right-click on a node copies its JSONPath or value. Preview renders HTML in a sandboxed frame and shows images.
- **Headers**, **Cookies** (when the response set any), **Timeline** (request, waiting, redirects, download) and **Tests** (passed/failed plus the script console).
- ⋯ menu: **Add to Docs as example**, **Download response…** and **Send to an agent…**.
- A failed send explains what happened. A blocked private address offers **Allow private addresses…**; a timeout links to the Settings tab.
- SSE and WebSocket show a live message console (last 500 messages) with **Clear**.

**Request menu (⋯ next to Save)**
- Duplicate, Save as a copy…, Copy as curl, Generate code… (cURL, JavaScript fetch, TypeScript fetch, Python requests, Go net/http), Import from curl…, Session variables…, Cookie jar… and Delete request….

**Collections**
- Search requests, create collections and nested folders, and right-click (or ⋯) for **New request here**, **New folder…**, **Export as OpenAPI**, **Rename…** and **Delete…**. Deleting a collection keeps its requests as **Ungrouped**.

**Import and Git sync**
- **Import…** has three tabs: a pasted **curl command**, a **File** (Postman v2.1 collections and environments, OpenAPI 3 / Swagger in JSON or YAML, and HAR recordings), and a whole **Postman account** via a Postman API key (optionally remembered in the Keychain).
- **Sync with Git…** pulls collection files from a repository connected to the workspace, or commits and pushes every top-level collection as Postman files under `collections/`, optionally to a named branch. Secrets are exported as `***`.

**History**
- Every request you send, newest first, with method, path, host, status and time. Search matches URL, method and status.
- **Agent runs only** shows requests agents sent through Otto's tools.
- Opening an entry loads it into a tab; it is not re-sent.
- ⋯ → **Retention…** keeps only the newest N requests and/or deletes requests older than D days (0 means no limit). ⋯ → **Clear history…** deletes every entry.

**Automations**
- An ordered list of saved requests. Each step can **check** its response (status code, JSON value at a JSONPath, or response time, with equals, does not equal, contains, less than, greater than) and **save** a value from the response into a variable for later steps, like a login token.
- Run options: environment, **Stop at the first failed step**, and **Run once per data row** (a JSON array of objects, each run through every step).
- **Run** saves first, then runs in the daemon. You get a per-step report, **Cancel run** while it runs, and a **Run history** with the request versions each run used.

**Agents**
- Agent sessions get these Otto tools: `otto_api_list`, `otto_api_get_request` and `otto_api_history` (read-only, secrets masked), plus `otto_api_execute`, `otto_api_upsert_request` and `otto_api_run_automation`, which send real requests or save changes.
- For agents, methods other than GET, HEAD and OPTIONS need explicit confirmation, secrets are resolved on the daemon and scrubbed from results, and JWTs come back as decoded claims.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| ⌘↵ | Send, connect or invoke the current request |
| ↵ | Send, when the URL field is focused |
| ⌘S | Save the request (asks for a name and place if it's new) |
| ⌘T | New request tab (replaces the global new-session shortcut on this page) |
| ⌘D | Duplicate the current request (replaces the global split shortcut on this page) |
| ← / → | Move between the sidebar views, editor tabs, response tabs or import tabs while one is focused |
| ← / → or ↑ / ↓ | Resize the focused divider (hold ⇧ for bigger steps) |

⌘K also offers **New API request**, **Duplicate API request**, **Import into API…**, **Manage API environments** and **Show API history**. See [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts) for the global ones.

## Tips and limits

- **Access:** the page needs the API client feature. Workspace viewers can look around but not send or change anything; sending, saving, importing files, Git sync, environments and automations need editor access. **Allow private addresses** is admin-only and applies to everyone in the workspace.
- **Secrets:** auth tokens and passwords move to the macOS Keychain when you save. They are never shown again or exported. To share one secret across requests, put it in an environment as a secret variable.
- **Secrets are bound to hosts.** Sending a stored secret to a host it hasn't gone to before asks **Send a secret to a new host?** first.
- Localhost and private networks (10.x, 192.168.x…) are blocked by default. An SSH tunnel (**Send through**) sends from the bastion instead, for public APIs that only accept whitelisted IPs.
- Requests time out after 60 seconds unless you set a timeout.
- Response bodies over 256 KB are shown raw; text over 512 KB is truncated (download it from ⋯); bodies over 25 MB aren't loaded at all.
- Interactive scripts run in a sandboxed worker for up to 5 seconds. They are not a security sandbox; only run scripts you trust.
- Automation datasets are limited to 1 MiB and 1,000 requests per run.
- History keeps response bodies. Set a retention limit if you call APIs that return sensitive data.
- Unset `{{variables}}` are sent literally. Check the variables line before you send.

## Related

- [Connections](#/walkthroughs/connections)
- [Git](#/walkthroughs/git)
- [Agents](#/walkthroughs/agents)
- [MCP Control Plane](#/walkthroughs/mcp)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
