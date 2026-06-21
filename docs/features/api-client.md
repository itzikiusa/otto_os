# API Client — the built-in REST workbench

Otto ships a "Postman"-style HTTP workbench inside the desktop app: organize
**requests** into **collections**, parameterize them with **environment
variables**, fire them off through the daemon, and read a rich **response**
viewer (status, timing, size, headers, body, trace). It also runs **automations**
(saved requests chained with assertions + variable extraction), captures
**history** and a shared **cookie jar**, and speaks **GraphQL, Server-Sent
Events, WebSocket, and unary/server-streaming gRPC** in addition to plain HTTP.

> This document describes the **in-app REST workbench** — the thing you use to
> call *other people's* APIs. It is **not** the same as the ottod HTTP API (the
> daemon's own control surface that the Otto UI talks to). For that, see
> [`./daemon-http-api.md`](./daemon-http-api.md).

Every outbound request is made **by the daemon**, not the webview, so requests
dodge browser CORS/CSP and get real connect/TLS/TTFB timing. Every outbound URL
is screened by an **SSRF guard** (`otto-netguard`) before a single byte is sent —
this is the hard limit on what the workbench can reach, and it is covered in full
under [Capabilities & limitations](#10-capabilities--limitations) and
[Security](#11-security).

---

## 1. Summary

| | |
|---|---|
| **What it is** | A collaborative REST/GraphQL/gRPC/streaming client embedded in Otto. |
| **Where requests run** | Inside `ottod` (the daemon), via a shared `reqwest` client — *not* the webview. |
| **Persistence** | Workspace-scoped SQLite (collections, requests, environments, history, automations). |
| **Scoping & auth** | All data is per-workspace. Reads need workspace **Viewer**; mutations + execution need **Editor**. |
| **Hard limit** | SSRF guard blocks loopback, private, link-local (incl. `169.254.169.254`), CGNAT, etc. — you **cannot** call internal/localhost hosts. |
| **Secrets** | Stored as plaintext JSON in the workspace DB — **not** the macOS Keychain. Treat tokens/passwords accordingly. |

---

## 2. Overview & where it lives

The API client is a workspace feature. Pick a workspace first; an empty/no
workspace selection shows nothing to load.

| Surface | File | What you get |
|---|---|---|
| **Full page** | `ui/src/modules/api/ApiPage.svelte` | Left sidebar (Collections / Automations / History / Env tabs) + a center column with request **tabs**, the request **builder**, and the **response** viewer stacked vertically. |
| **Right-side panel** | `ui/src/modules/api/ApiPanel.svelte` | A compact version: a "Load…" dropdown (saved requests + recent history), the same builder (compact), a foldable environment section, and the response viewer. Reuses the same store, so the page and panel edit the *same* draft. |

Center column (full page):

- **Request tabs** — open many drafts at once; `+` (or `⌘T`) opens a new one,
  `×` closes one. Each tab shows the method + a label (the saved name, else
  `METHOD /path`, else `New Request`).
- **Builder pane** — `RequestBuilder.svelte`.
- **Response pane** — `ResponseViewer.svelte`.

Sidebar tabs:

- **Collections** — `CollectionsTree.svelte`
- **Automations** — `AutomationsView.svelte`
- **History** — `HistoryList.svelte`
- **Env** — `EnvSelector.svelte` (a dot appears on the tab when an environment is active)

Everything for the current workspace is loaded together (collections, requests,
environments, history) plus automations when the page mounts or the workspace
changes (`apiClient.loadAll()` + `loadAutomations()`).

---

## 3. Collections & requests

Collections are named folders that hold saved requests. They nest arbitrarily
via `parent_id` (a collection inside a collection is shown as a "folder").

### Collections tree (`CollectionsTree.svelte`)

- **New request** (`+`) — opens a blank draft tab. (Available to everyone,
  including Viewers — but a Viewer can't *save* it.)
- **New collection** / **New folder** — Editor only. "New folder" on a collection
  row creates a child collection under it.
- **Rename** / **Delete** collection — Editor only. Deleting a collection removes
  its descendant folders too; **requests inside become ungrouped, not deleted**
  (confirmed both client-side and server-side).
- **Ungrouped** — requests with no `collection_id` are listed under an "Ungrouped"
  header at the bottom.
- **Request rows** — click to load the request into the builder; the trash icon
  (Editor only, appears on hover) deletes it after a confirm.
- Collections sort by `position` then name; requests likewise.

### Saving a request

From the builder, **Save** (`⌘S`):

1. Prompts for a request **name** (prefilled with the saved name, or `METHOD URL`).
2. If any collections exist, prompts which collection to save into (type the
   number from the list, or leave blank for ungrouped). An already-saved request
   keeps its existing collection unless you change it.
3. Viewers are refused with a "Read-only" toast.

> **What gets saved (and what does NOT):** A saved request persists exactly these
> fields: `name`, `method`, `url`, `headers`, `query`, `body_mode`, `body`,
> `auth`, plus its `collection_id` and `position`. **Scripts, Docs, Settings
> (timeout/redirects/TLS), GraphQL variables, and the transport kind
> (SSE/WS/gRPC) are part of the in-memory draft only — they are NOT stored with
> the request and are lost when you reload it.** See
> [§10](#10-capabilities--limitations).

### Import / export

- **Import** (Editor only) — file picker accepting `.json/.har/.yaml/.yml`. The
  format is auto-detected and parsed **client-side** into a new collection (with
  nested folders):
  - **Postman v2.1** (`info` + `item[]`)
  - **OpenAPI 3 / Swagger** (`openapi`/`swagger`)
  - **HAR** (`log.entries`)
  Anything else throws "Unrecognized format".
- **Export OpenAPI** — per-collection download icon. Calls
  `GET …/collections/{id}/openapi` and saves the returned OpenAPI 3 JSON as
  `<name>.openapi.json`.
- **Git sync** (Editor only, branch icon) — pull/push Postman collection files
  to/from a git repo connected to the workspace:
  - **Pull** imports every recognized collection file (stored under
    `collections/`) via `POST /repos/{id}/api-collections/pull`.
  - **Commit & Push** exports each *root* collection to a
    `<name>.postman_collection.json` file and commits via
    `POST /repos/{id}/api-collections/push` (optional target branch → open a PR
    from the Git tab afterwards).

---

## 4. The request builder (all fields)

`RequestBuilder.svelte`. Shared by the full page and the compact panel.

### Transport kind

A selector chooses the transport: **HTTP · SSE · WebSocket · gRPC**. The tab
strip and URL bar change with it:

| Kind | Method shown | Tabs available | How it runs |
|---|---|---|---|
| **HTTP** | yes | Params, Authorization, Headers, Body, Scripts, Docs, Settings | `POST …/execute` |
| **SSE** | yes | (same as HTTP) | Streaming relay over `ws://…/ws/api-client/stream` |
| **WebSocket** | no | Headers, Authorization, Settings | Streaming relay (same socket) |
| **gRPC** | no | Message, Metadata | `POST …/grpc/invoke` |

### URL bar

- **Method** — `GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS` (HTTP/SSE only).
- **URL** — `{{var}}` tokens are highlighted live. Placeholder hints differ by
  kind (`https://…`, `wss://…`, `https://grpc…:443`).
  - **Paste a `curl …` command into the URL field** and it is imported
    automatically (parsed by the daemon).
  - **Enter** sends the request; **⌘↵** also sends from anywhere.
- **Send** (label is `Send` / `Connect`/`Disconnect` / `Invoke` per kind).
- **Stop** appears while an HTTP request is in flight or a stream is connected;
  it aborts the request / disconnects.

### Toolbar

- **Import curl** — paste box; parses via `POST /api-client/import-curl`.
- **Copy as curl** — builds a `curl` string from the current draft (client-side).
- **Code** — generate a request snippet in **cURL, JavaScript (fetch), TypeScript
  (fetch), Python (requests), or Go (net/http)** and copy it.
- **Cookies** — opens the shared cookie jar (see [§6](#6-sending--reading-responses)).
- **Vars** — session/runtime variables editor (see [§5](#5-environments--variables)).
- **Active environment** chip — shows the active env name.
- **Save** — save the draft (see [§3](#3-collections--requests)).

### Tabs

**Params** — query-param key/value rows. Each row has an **enabled** checkbox,
key, value, and remove. Disabled or empty-key rows are dropped before sending.

**Headers** — same key/value/enabled grid, with autocompletion: a datalist of
~30 common header names, and value suggestions for known headers
(`Content-Type`, `Accept`, `Accept-Encoding`, `Cache-Control`, `Authorization`,
…).

**Body** — Postman-style radio modes:

| Mode | Wire format | Notes |
|---|---|---|
| **none** | — | no body |
| **form-data** (`multipart`) | `multipart/form-data` | key/value rows; each row can be **Text** or **File**. Files are read to base64 in the browser and reassembled server-side; the daemon sets the boundary. |
| **x-www-form-urlencoded** (`form`) | `application/x-www-form-urlencoded` | key/value rows encoded as `k=v&…`. |
| **raw** | text | sub-type dropdown: **Text / JavaScript / JSON / HTML / XML** — drives the editor language and the auto Content-Type. |
| **binary** | — | **shown but disabled** (not supported). |
| **GraphQL** | `application/json` | a Query editor + a separate **Variables (JSON)** editor; **Introspect schema** lists types/fields. The query + variables are wrapped into `{ "query", "variables" }` at send time. |

The Content-Type header is auto-managed: choosing a body mode sets the implied
Content-Type, and only overwrites a value you didn't hand-type.
**Beautify** pretty-prints JSON/XML/HTML raw bodies.

**Authorization** — type selector:

| Type | Fields | Effect on the wire |
|---|---|---|
| **none** | — | nothing added |
| **bearer** | Token | `Authorization: Bearer <token>` |
| **basic** | Username, Password | `Authorization: Basic base64(user:pass)` |
| **api key** | Key, Value, **Add to: Header \| Query param** | header or query param |
| **oauth2** | Grant (Client Credentials / Password / Refresh Token), Token URL, Client ID/Secret, Username/Password (password grant), Refresh Token (refresh grant), Scope | **Get New Token** calls `POST …/oauth2/token` server-side; the returned `access_token` is stored on the draft and attached as `Authorization: <token_type> <access_token>`. |

All auth fields accept `{{var}}` substitution.
**oauth2 authorization-code grant (browser redirect) is not supported** — only
the three server-to-server grants above.

**Scripts** *(HTTP/SSE only)* — two JS editors:

- **Pre-request Script** — runs before sending; can mutate the request
  (`pm.request.method/url/body`, `pm.request.headers.add/upsert/remove/get`) and
  set variables (`pm.environment.set(...)`, `pm.variables`, `pm.globals`).
- **Post-response Script (Tests)** — runs after; reads `pm.response`
  (`.code`, `.status`, `.responseTime`, `.headers`, `.text()`, `.json()`),
  declares tests with `pm.test('name', () => pm.expect(...).toBe(...))`, and can
  set variables for chaining. Supported matchers: `toBe`, `toEqual`/`eql`,
  `toContain`, `toBeTruthy`, `toBeFalsy`, `above`, `below`.

> **Scripts run in the webview, client-side** (`new Function(...)`), **not** in
> the daemon, and are explicitly *not a security sandbox* — they are a
> convenience runtime for your own endpoints. They are also **not persisted**
> with the saved request (draft-only).

**Docs** — free-form **Markdown** notes for the request, with a rendered preview.
(Draft-only; not persisted.)

**Settings** — per-request execution options:

- **Request timeout** (ms; blank = daemon default **60 s**).
- **Automatically follow redirects** (default on).
- **Verify TLS certificate** (default on; turning it off accepts self-signed /
  invalid certs for that request). (Draft-only; not persisted.)

### gRPC panel

Shown when kind = gRPC: **Upload .proto** (parsed via `…/grpc/describe`) or
**Reflect** (list services via server reflection, `…/grpc/reflect`, no proto
needed). Pick a method from the grouped dropdown; the **Message** tab holds the
JSON request, **Metadata** holds headers. **Invoke** calls `…/grpc/invoke`.

---

## 5. Environments & variables

`EnvSelector.svelte`. Environments are named bags of string key/value variables,
scoped to the workspace. Exactly one can be **active** at a time.

- **New / Rename / Delete** environment — Editor only.
- **Activate** — the radio on each row sets it active
  (`POST …/environments/{id}/activate`); the active env's name shows as a chip in
  the builder toolbar and a dot on the sidebar Env tab.
- **Edit variables** — inline key/value rows (Editor, full page only). Empty keys
  are dropped on save.

### Substitution (`{{var}}`)

`{{name}}` placeholders are resolved **on the daemon at send time** in the URL,
query values, header values, body, and auth fields. Resolution order:

1. **Dynamic built-ins** (always win): `{{$guid}}`/`{{$randomUUID}}`,
   `{{$timestamp}}` (unix seconds), `{{$isoTimestamp}}`, `{{$randomInt}}`
   (0–999).
2. **Runtime/session variables** (the "Vars" panel; also written by scripts and
   automation extractions) — sent as `vars` overrides and layered **on top of**
   the environment.
3. **Active (or explicitly selected) environment** variables.
4. Unknown placeholders are **left intact** (e.g. `{{missing}}` stays literally).

Runtime variables are an in-memory session map in the UI store; scripts read/write
them via `pm.*`, and automation **extracts** write into the run's chained map.

---

## 6. Sending & reading responses

`ResponseViewer.svelte`. On **Send**, the draft is POSTed to `…/execute`; the
daemon substitutes variables, applies auth, sends via the shared `reqwest`
client, records a history entry, and returns the response.

### Response header

- **Status pill** — colored by class (2xx green / 3xx accent / 4xx orange / 5xx
  red), with status text.
- **Timing** — `duration_ms`. **Size** — human-readable. **Content-Type**.
- **Save** — download the response body to disk (uses the full base64 bytes;
  filename from `Content-Disposition` when present, else `response.<ext>`).
- **To agent** — send the response (status/content-type/body) to a running agent
  session via the context-packet dialog.

### Body tab

- **Pretty / Raw** toggle. JSON is pretty-printed (memoized, size-gated: bodies
  over **256 KB** are shown raw to avoid blocking the UI).
- **JSONPath filter** — for JSON bodies, a `$.a.b[0].c`-style filter narrows the
  view (debounced).
- **Images** (`image/*`) are previewed inline from the base64 body.
- **Truncation** — text display is capped at **512 KB**; a banner says "Showing
  the first 512 KB… use **Save** to get the full response."
- **Too large** — bodies over **25 MB** are not inlined at all; a panel explains
  this (re-run against a smaller payload to inspect inline).

### Other tabs

- **Headers** — full response header table.
- **Trace** — per-phase steps: Request → Sent → Waiting (TTFB, incl. connect +
  TLS) → Redirected (if any) → Downloaded → Completed, each with timings.
- **Tests** — present when a post-response script ran: per-test pass/fail (`✓/✕`,
  `passed/total` badge) plus a **Console** of `console.*` + `[pre]`/`[test]`
  logs.

### Streaming (SSE / WebSocket)

When kind is SSE or WebSocket, the viewer becomes a **live console**: a status
pill, a message count, a **Clear** button, and a virtualized log capped to the
**last 500** messages. SSE shows named events; WebSocket shows ▲ sent / ▼ recv
and has an input box to send a message while connected. The daemon bridges to the
upstream over `ws://…/ws/api-client/stream` (so secrets stay server-side and CORS
is bypassed).

---

## 7. History

`HistoryList.svelte`. Every executed HTTP request (success *or* failure) is
recorded per-workspace.

- Rows show method, URL, status (color-classed; null for network failures), and
  a formatted timestamp; the list is virtualized.
- **Click a row** to reload its request snapshot into the builder
  (`loadHistoryIntoDraft`). The snapshot captures method/url/headers/query/
  body_mode/body/auth as executed.
- **Clear history** empties the workspace's history (`DELETE …/history`, Editor).
- History is bounded server-side: default **100**, max **500** entries returned.

The compact panel also surfaces the last 15 history entries in its "Load…"
dropdown.

---

## 8. Automations

`AutomationsView.svelte`. An automation is an **ordered list of steps**; each step
runs one **saved request** and can assert on its response and extract values into
variables for later steps (request chaining).

### Authoring

- **New / Rename / Delete** automation — Editor only. The selected automation
  loads into a working copy (edits aren't live until **Save**).
- **Add step** — appends a step bound to a saved request (you must have at least
  one saved request). Reorder with up/down; remove with the trash icon.
- **Per-step assertions** — each is `kind` + `op` + `value` (+ a `path` for
  `json_path`):
  - **kind**: `status`, `json_path`, `duration_ms`
  - **op**: `eq`, `ne`, `contains`, `lt`, `gt`
  - With **no assertions**, the step passes on any successful (2xx) response.
- **Per-step extracts** — `JSONPath → var`: pull a value from the response body
  into a `{{var}}` available to subsequent steps. (Incomplete rows — missing path
  or var — are dropped on save.)

### Running

**Run** (saves first if dirty) calls `POST …/automations/{id}/run`. The daemon:

- Seeds the chained variable map from the workspace's **active environment**.
- Runs **every step in order** against its saved request, reusing the exact
  `/execute` send path. A failing step is recorded but **never aborts the run**;
  errors are captured, not thrown.
- Evaluates assertions against status / duration / the JSON body, then applies
  extractions into the chained map for later steps.
- Returns a report: an overall pass/fail banner (`passed = every step ok`),
  plus per-step status, duration, error, and per-assertion `✓/✕`.

After a run the environments are refreshed (extracts may have updated variables).

> Automations run the request's **stored** fields only — they do **not** run
> per-request pre/post Scripts (those are draft-only and not persisted), and they
> don't take an explicit environment id (they always use the active one).

---

## 9. API / contract reference

Authoritative contract: `docs/contracts/api.md` (the "API client (Postman)"
section). All workspace-scoped routes are under
`/api/v1/workspaces/{wid}/api-client/…`. **Reads require workspace Viewer;
mutations and execution require Editor.** Cross-workspace IDs 404
(`ensure_in_workspace`).

| Method & path | Role | Request → Response |
|---|---|---|
| `GET /collections` | Viewer | → `Collection[]` |
| `POST /collections` | Editor | `UpsertApiCollectionReq` → `Collection` |
| `PATCH /collections/{id}` | Editor | `UpsertApiCollectionReq` → `Collection` |
| `DELETE /collections/{id}` | Editor | → 204 (orphans its requests) |
| `GET /collections/{id}/openapi` | Viewer | → OpenAPI 3 JSON |
| `GET /requests` (`?collection_id`) | Viewer | → `Request[]` |
| `POST /requests` | Editor | `UpsertApiRequestReq` → `Request` |
| `GET /requests/{id}` | Viewer | → `Request` |
| `PATCH /requests/{id}` | Editor | `UpsertApiRequestReq` → `Request` |
| `DELETE /requests/{id}` | Editor | → 204 |
| `GET /environments` | Viewer | → `Environment[]` |
| `POST /environments` | Editor | `UpsertApiEnvironmentReq` → `Environment` |
| `PATCH /environments/{id}` | Editor | `UpsertApiEnvironmentReq` → `Environment` |
| `DELETE /environments/{id}` | Editor | → 204 |
| `POST /environments/{id}/activate` | Editor | → `Environment` (sets active) |
| `GET /history` (`?limit`, default 100, max 500) | Viewer | → `ApiHistoryEntry[]` |
| `DELETE /history` | Editor | → 204 |
| `POST /execute` | Editor | `ExecuteApiReq` → `ApiResponse` |
| `POST /grpc/describe` | Editor | `{proto}` → service/method descriptors |
| `POST /grpc/invoke` | Editor | `{url, proto, method, body, headers}` → `ApiResponse` |
| `POST /grpc/reflect` | Editor | `{url, headers}` → service listing |
| `POST /oauth2/token` | Editor | `OAuth2TokenReq` → token JSON |
| `GET /cookies` | Viewer | → cookie jar |
| `DELETE /cookies` | Editor | → 204 (clears jar) |
| `GET /automations` | Viewer | → `Automation[]` |
| `POST /automations` | Editor | `UpsertApiAutomationReq` → `Automation` |
| `PATCH /automations/{id}` | Editor | `UpsertApiAutomationReq` → `Automation` |
| `DELETE /automations/{id}` | Editor | → 204 |
| `POST /automations/{id}/run` | Editor | → `ApiRunResult` |
| `POST /api-client/import-curl` *(not workspace-scoped)* | member | `{curl}` → `ParsedCurl` |
| `GET /ws/api-client/stream?token=…` *(root WS)* | token | SSE/WebSocket relay |

### Persisted shapes (the source of truth for what survives a reload)

```
ApiRequest      { id, workspace_id, collection_id?, name, method, url,
                  headers[], query[], body_mode, body, auth, position,
                  created_at, updated_at }
UpsertApiRequestReq { collection_id?, name, method, url, headers, query,
                  body_mode, body, auth }    // ← exactly these are saved
ApiEnvironment  { id, workspace_id, name, variables{key:value}, is_active, created_at }
ApiHistoryEntry { id, workspace_id, method, url, status?, duration_ms?,
                  request(snapshot), response(snapshot), executed_at }
ApiResponse     { status, status_text, headers[], body, body_base64,
                  truncated, too_large, duration_ms, size_bytes, content_type, trace[] }
```

`body_mode` persists as one of `none | json | raw | form | graphql`
(form-data/multipart and the raw sub-types are encoded into `body`/headers).
**Scripts, Docs, Settings, GraphQL variables, transport kind, and `.proto` are
not part of `ApiRequest`** — they live only in the UI draft.

---

## 10. Capabilities & limitations

### You CAN

- Send **HTTP** requests (any of GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS) to public
  hosts, with query params, headers, and JSON / raw (Text/JS/JSON/HTML/XML) /
  `x-www-form-urlencoded` / `multipart/form-data` bodies.
- **Upload files** in `multipart/form-data` (read to base64 in the browser,
  reassembled by the daemon with a guessed MIME type).
- Use **auth helpers**: Bearer, Basic, API-key (header or query), and OAuth2
  (client-credentials / password / refresh-token grants) with a server-side
  **Get New Token**.
- Run **GraphQL** (query + JSON variables) and **introspect** a schema.
- Open **Server-Sent Events** and **WebSocket** streams (bridged through the
  daemon), including sending WS messages live.
- Call **gRPC** **unary** and **server-streaming** methods, from an uploaded
  `.proto` or via **server reflection**.
- Organize requests into nested **collections/folders**; **import** Postman v2.1 /
  OpenAPI 3 / HAR; **export** a collection to OpenAPI; **sync** collections
  with git as Postman files.
- Parameterize with **environments** + `{{var}}` + dynamic `{{$guid}}` etc.;
  set **session/runtime variables** by hand or from scripts.
- Run **pre-request / post-response JS scripts** (Postman-ish `pm` API) for
  request mutation, variable chaining, and tests/assertions.
- Chain saved requests into **automations** with assertions + JSONPath extraction.
- Read responses with pretty/raw view, JSONPath filtering, image preview, header
  table, a timing **trace**, and **save to disk**; review **history** and the
  shared **cookie jar**; **copy as curl** or generate code in 5 languages.
- Configure **per-request timeout**, redirect-following, and TLS verification.

### You CANNOT

- **Reach internal/loopback/private hosts.** The SSRF guard blocks the request
  *before* connecting (see [§11](#11-security)). `http://localhost`, `127.0.0.1`,
  `10.x/172.16–31/192.168.x` (RFC1918), `169.254.169.254` and all link-local,
  `100.64.0.0/10` (CGNAT), `0.0.0.0`/`::`, multicast/broadcast, IPv6 ULA
  (`fc00::/7`) and `::1`, IPv4-mapped forms (`::ffff:127.0.0.1`) — **all
  rejected**, including DNS names that resolve to them and any redirect that
  bounces toward them. *This is deliberate and not configurable from the UI.*
- **Use non-fetchable schemes.** Only `http(s)` / `ws(s)` / `grpc(s)` are
  allowed; `file:`, `data:`, etc. are rejected.
- **Send a `binary` (single-file) body** — the radio exists but is disabled.
- **Use OAuth2 authorization-code grant** (browser redirect) — only the three
  server-side grants are implemented.
- **Call client-streaming or bidirectional gRPC** — only unary and
  server-streaming are supported.
- **Persist Scripts, Docs, Settings, GraphQL variables, or the transport kind
  with a saved request** — these are draft-only and are lost on reload/save.
  (They are *not* sent to git or OpenAPI export either.)
- **Run scripts inside automations** — automation steps execute the request's
  stored fields only; per-request pre/post scripts don't run there.
- **Run scripts server-side / sandboxed** — scripts execute in the webview and
  are not a security boundary.
- **Inspect bodies over 25 MB inline** (not loaded) or see more than the first
  512 KB of text without **Save**; JSON pretty-print is skipped above 256 KB.
- **See more than the last 500 streamed messages** (older ones are dropped from
  the console).
- **Store secrets in the Keychain** — environment variables and auth credentials
  live in the workspace SQLite DB as plaintext JSON (see [§11](#11-security)).
- Requests **time out at 60 s** by default (override per request in Settings).

---

## 11. Security

### SSRF guard (`otto-netguard`)

Because the daemon makes outbound requests **on the user's behalf**, every
user-supplied URL is screened by `otto-netguard` (`crates/otto-netguard/src/lib.rs`,
"audit S1") before any connection — on the HTTP execute path, the SSE/WebSocket
relay, the gRPC invoke/reflect paths, and the OAuth2 token fetch.

How it works (`check_url`):

1. Parse the URL; reject any scheme other than `http(s)/ws(s)/grpc(s)` and any
   URL without a host.
2. If the host is an IP literal, classify it directly; otherwise **resolve DNS**
   and classify **every** resolved address.
3. **Reject** if any address is loopback, RFC1918 private, link-local (incl. the
   `169.254.169.254` cloud-metadata endpoint and `fe80::/10`), CGNAT
   (`100.64.0.0/10`), unspecified (`0.0.0.0`/`::`), multicast/broadcast,
   documentation range, IPv6 ULA (`fc00::/7`), or IPv4-mapped forms of any of
   these (`::ffff:127.0.0.1`).
4. **Redirects** are capped at **10 hops** and **each hop's host is re-validated**
   with the same rules and **fails closed** (a redirect it can't validate isn't
   followed) — so an upstream `30x` can't bounce the fetch into the private
   network.

The same classifier is shared (one definition) with the Message-Brokers metrics /
schema-registry fetches, so the rules can't drift.

### Where the daemon listens

ottod binds **loopback only** (`127.0.0.1:7700`) by default. The API client is a
UI on top of it. (Note the irony made useful: the workbench's own SSRF guard means
you can't point the workbench at the daemon — or any other localhost service.)

### Secrets storage — important

Unlike the rest of Otto (which routes tokens/passwords through the macOS
**Keychain** via `otto-keychain`), the API client **does not** use the Keychain.
**Environment variables and request auth — bearer tokens, basic-auth passwords,
API keys, OAuth client secrets and access tokens — are stored as plaintext JSON
in the workspace SQLite state DB**, and request/response **history snapshots**
(which may contain auth headers and response bodies) are persisted too. The
**cookie jar is daemon-global** (shared across the whole daemon, not isolated per
workspace) and is captured automatically from `Set-Cookie` and resent on matching
requests. Treat the workbench data as sensitive; use `{{var}}` references rather
than inlining long-lived secrets where practical, and clear history/cookies when
appropriate.

### Roles

All workspace-scoped routes enforce workspace role: **Viewer** for reads,
**Editor** for any mutation *and for execution* (executing/streaming/invoking
counts as Editor). The Viewer/Editor split matches the contract; see
[`./daemon-http-api.md`](./daemon-http-api.md) and the RBAC docs.

---

## 12. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `blocked address … (SSRF guard)` | The URL (or a DNS result, or a redirect target) is an internal/loopback/private/link-local/CGNAT address. You can't reach localhost or internal hosts from the workbench — by design. |
| `blocked url scheme: …` / `url has no host` | Only `http(s)/ws(s)/grpc(s)` with a host are allowed. No `file:`/`data:`. |
| `request timed out after 60s` | Default timeout. Raise it in **Settings → Request timeout** (ms). |
| TLS / certificate errors | Turn off **Settings → Verify TLS certificate** for that request to accept self-signed/invalid certs. |
| "Read-only" toast on Save/edit | You have **Viewer** access to this workspace; you need **Editor** to save, mutate, or execute. |
| Nothing loads / empty lists | No workspace is selected — the API client is workspace-scoped. |
| My Scripts / Docs / Settings vanished after saving | Expected — those are draft-only and not persisted with a saved request (see [§10](#10-capabilities--limitations)). |
| Saved request lost its body type as form-data | form-data/multipart and raw sub-types are encoded into `body`/headers; the stored `body_mode` is `none/json/raw/form/graphql` only. |
| `client-streaming gRPC methods are not supported` | Only unary and server-streaming gRPC are implemented. |
| OAuth2 "Get New Token" fails | Check the **Token URL** and grant; the authorization-code (redirect) grant isn't supported — use client-credentials / password / refresh-token. |
| Streamed log seems to drop messages | The console caps at the **last 500** messages. |
| Big response shows a "too large"/"truncated" banner | >25 MB isn't inlined; text >512 KB is truncated for display — use **Save** for the full body. |
| Import says "Unrecognized format" | Only Postman v2.1, OpenAPI 3/Swagger, and HAR JSON are recognized. |

---

## 13. Related docs

- [`./daemon-http-api.md`](./daemon-http-api.md) — the **ottod HTTP API** (the
  daemon's own control surface the Otto UI talks to). Distinct from this in-app
  REST workbench.
- [`./workflows.md`](./workflows.md) — the Workflows feature (a different
  automation surface from the API client's request-chaining automations).
- `docs/contracts/api.md` — **authoritative** REST/WS contract (see the "API
  client (Postman)" section). The TypeScript types in
  `ui/src/lib/api/types.ts` mirror it.
- `docs/MULTI-USER-RBAC.md` — workspace roles (Viewer/Editor/Admin) that gate
  this feature.

### Source map

| Area | File(s) |
|---|---|
| UI page / panel | `ui/src/modules/api/ApiPage.svelte`, `ApiPanel.svelte` |
| Builder / response / sidebar | `RequestBuilder.svelte`, `ResponseViewer.svelte`, `CollectionsTree.svelte`, `EnvSelector.svelte`, `HistoryList.svelte`, `AutomationsView.svelte` |
| UI store / scripts / import / codegen | `ui/src/lib/stores/apiClient.svelte.ts`, `apiStream.svelte.ts`, `ui/src/lib/api/scripts.ts`, `importers.ts`, `codegen.ts` |
| Backend (REST) | `crates/otto-server/src/routes/api_client.rs`, `grpc.rs`, `api_stream.rs` |
| SSRF guard | `crates/otto-netguard/src/lib.rs` |
| Domain / contract DTOs | `crates/otto-core/src/domain.rs`, `crates/otto-core/src/api.rs`, `docs/contracts/api.md` |
