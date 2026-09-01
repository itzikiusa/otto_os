# Browser

A lightweight, workspace-scoped **web browser inside Otto**: reader-mode tabs that
fetch and render a URL as clean markdown, DOM annotations you can drop on a page and
later send into a live agent session, and one-click paths to summarize a page or
save it (plus its marks) into a Vault note. Fetching runs through
`otto-browser`'s Lightpanda-sidecar-or-plain-fetch engine, started lazily on the
first fetch. Every URL an Otto-initiated fetch touches is netguard-checked first —
loopback, private, and cloud-metadata addresses are refused with a `400`.

## What's in it

- **Reader tabs** — open a URL, get back extracted markdown + title. A tab is
  `mode:"reader"` (fetched/rendered) or `mode:"live"` (a real embedded page the
  daemon never fetches — a native Tauri child webview in the desktop app; off
  Tauri the pane falls back to reader, since there's nothing to host it in).
  Navigating a reader tab re-fetches and adopts the new page's title. See "Live
  tabs (desktop app)" below for how live mode is driven.
- **DOM annotations** — a mark on a URL (selector + excerpt + your comment), keyed
  on the URL rather than the tab id so it survives the tab being closed and
  reattaches to any tab that later opens the same page.
- **Selector query** — pull just the nodes matching a CSS selector out of a page
  instead of the whole thing.
- **Summarize** — one short-lived agent turn condenses a fetched page's markdown
  into a few sentences for a developer notebook.
- **Send to session** — write an annotation's `[Browser mark] …` context block
  straight into a live agent session's input.
- **Save to Vault** — write an OKF-flavored note (front-matter, summary, one
  `## Mark N` section per annotation) into a doc vault.
- **Agent MCP tools** — `browser_navigate` / `browser_page` / `browser_query` /
  `browser_summarize` / `browser_login`, so an agent session can drive the same
  fetch pipeline (and, opt-in, sign in to a stored credential) without going
  through the UI.
- **Site Credentials** — save a domain/username/password Otto can autofill on
  request; the password lives in the macOS Keychain, never in the database or a
  log line, and an agent only gets to use one you've explicitly opted in
  (`allow_agent_use`).

## Setup

Nothing to configure to get plain-fetch browsing working — it's on by default.
For JS-rendered pages, install the [Lightpanda](https://github.com/lightpanda-io/browser)
headless browser: `brew install lightpanda-io/tap/lightpanda`, or download a release
binary. Otto locates it in priority order — an explicit `OTTO_LIGHTPANDA_BIN` path,
then `PATH`, then well-known install locations (`/usr/local/bin`, `/opt/homebrew/bin`,
`~/.local/bin`, the slot an auto-download would use) — and starts it as a managed
sidecar (CDP over a loopback port) on first use. **Lightpanda is beta software**: it
covers most JS-rendered pages, but isn't a full Chromium — treat `degraded:true`
responses (see below) as expected on some sites, not necessarily a misconfiguration.
If no binary is found or the sidecar fails to start, every fetch transparently falls
back to a script-free plain fetch (`degraded:true` on the response) rather than
failing outright. A host that fails against the primary engine three times in a row
is skipped straight to the fallback until it next succeeds.

RBAC: gated by `Feature::Browser` (`View` for reads, `Edit` for writes — including
`/page` and `/query`, since they fetch on the caller's behalf) plus the normal
workspace-role axis. The flat by-id routes (`/browser/tabs/{id}`,
`/browser/annotations/{id}`) load the row first and check the role on its
`workspace_id` — the IDOR guard, since the feature axis alone is workspace-blind.

## Walkthrough

1. Open the Browser tab in a workspace, paste a URL — a reader tab opens showing
   the extracted markdown.
2. Select text/an element on the rendered page and add a comment to drop an
   annotation; it stays attached to that URL across tab closes/reopens.
3. **Summarize** the current page for a quick notebook-style blurb, or **Save to
   Vault** to write the page (+ its marks) as a note.
4. **Send** an annotation into any live session in the same workspace — the target
   session receives a fenced `[Browser mark] …` block in its input, submitted like
   a normal message.
5. Save a **Site Credential** (domain/username/password) and flip it to "allow
   agent use" if you want an unattended agent session to be able to sign in to
   that site with it. See "Site Credentials & agent login" below.

## API surface

See `docs/contracts/api.md` § "Browser (reader/annotate tabs + on-demand page
fetch)" for the full request/response table. Summary:

| Route | Effect |
|---|---|
| `GET/POST /workspaces/{wid}/browser/tabs` | list / create reader tabs |
| `PATCH/DELETE /browser/tabs/{id}` | navigate (re-fetches in reader mode) / close |
| `GET /workspaces/{wid}/browser/page?url=…` | fetch a URL → `{url,title,markdown,html,engine,degraded}` |
| `GET /workspaces/{wid}/browser/query?url=…&selector=…` | fetch + CSS-selector match → `{matches:[{selector,outer_html,text}]}` |
| `GET/POST /workspaces/{wid}/browser/annotations` | list / create marks (`?url=` filters) |
| `PATCH/DELETE /browser/annotations/{id}` | edit comment / remove a mark |
| `POST /workspaces/{wid}/browser/summarize` | fetch + one agent turn → `{summary,engine,degraded}` |
| `POST /workspaces/{wid}/browser/annotations/{id}/send` | write the mark's context block into a session's input |
| `POST /workspaces/{wid}/browser/vault-save` | write an OKF note for the page → `{note_path}` |
| `GET/POST /workspaces/{wid}/browser/credentials` | list (password never included) / create a Site Credential — password goes straight to the Keychain |
| `PATCH/DELETE /browser/credentials/{id}` | update (optionally rotate the password) / remove a credential (also deletes its Keychain entry) |
| `POST /browser/credentials/{id}/reveal` | `{confirm:true}` → `{password}` — the only route that ever returns the plaintext password |
| `POST /workspaces/{wid}/browser/login` | `{domain}` → `{logged_in,engine}` — governed agent sign-in, see below |

WS events: `browser_tab_updated{tab}`, `browser_annotation_added{annotation}` (see
`docs/contracts/ws.md`).

## Agent MCP tools

Every tool-capable session gets five `browser_*` tools alongside the rest of
Otto's first-party MCP surface (`crates/ottod/src/mcp_tools.rs`), authorizing as
the session's own owner (the same `WorkspaceRole::Editor` gate a human hits — no
more):

| Tool | Args | Returns |
|---|---|---|
| `browser_navigate` | `url` | `{ok,title}` — opens a reader tab (the only mutating browser tool: it creates a `browser_tabs` row) |
| `browser_page` | `url` | `{markdown,title,engine,degraded}` |
| `browser_query` | `url, selector` | `{matches:[{selector,outer_html,text}]}` |
| `browser_summarize` | `url` | `{summary,engine,degraded}` |
| `browser_login` | `domain` | `{logged_in,engine}` — signs in with a stored credential the user marked "allow agent use"; the password never enters the tool call, its result, or the audit log |

`browser_page` drops the route's `url`/`html` fields from what the agent sees — the
caller already has the URL, and the raw markup is large relative to the extracted
markdown most agents actually want. `browser_summarize` doesn't persist anything:
the summarize turn runs in an ephemeral, unresumed session (same pattern as
`db_assist`), so it's treated as a read even though it's a `POST`.

## Site Credentials & agent login

A **Site Credential** is a domain/username/password Otto can use to sign a
browser session in. Creating or rotating one writes the password straight to
the macOS Keychain (`otto-keychain`) and stores only the resulting
`keychain_ref` in the database — a `BrowserCredential` row has **no password
field at all**, so it's structurally impossible for the API to leak the secret
by serializing the row. `allow_agent_use` defaults to `false` everywhere
(migration column, create form, UI toggle): an unattended agent session only
gets to use a credential you've explicitly opted in, one at a time, per
domain.

`POST /browser/login` (and the `browser_login` MCP tool) is the governed
agent-facing sign-in path: the daemon resolves the password from the Keychain
server-side and hands it directly to the fill-and-submit JS run inside the CDP
session — it never appears in the request, the response, the MCP tool result,
or a log line (only the domain and a `logged_in` boolean are logged). It's
rate-limited to 3 attempts/minute per domain (429 `Retry-After` past that).

**Known limitation:** the login flow always targets `https://{domain}/` — there
is no separate stored "login page URL" field, so a site whose sign-in form
lives at a different path (and doesn't redirect there from the domain root)
isn't reachable via `browser_login` today; sign in manually in a reader/live
tab instead. Login also requires the Lightpanda engine (`login()` runs JS) — a
fallback-only daemon answers every `browser_login` call with a `502`.

## Prompt-injection defense

`/summarize`, `/annotations/{id}/send`, and `/vault-save` all embed
attacker-controlled page content (fetched markdown, an annotation's
excerpt/comment) into text handed to a tool-using agent. Every embed goes through
`fence_untrusted`/`build_context_block` in `routes/browser.rs`: it's wrapped in a
boundary tagged with a fresh per-call nonce the page author couldn't have known in
advance (so it can't forge a matching close tag), and any line that starts with one
of the block's own structural prefixes (`[Browser mark]`, `Selector:`, `Excerpt:`,
`Note from user:`) is neutralized first so a hostile page can't impersonate a
second, fabricated instruction line once inside the fence.

## Capabilities & limits

- **Lightpanda is beta**: it's a real but young headless-browser project, not a
  full Chromium — some JS-heavy or unusual pages will `degraded:true` (or
  outright fail) even with a working sidecar. Treat reader mode as
  best-effort JS rendering, not a guarantee.
- **Live tabs are desktop-app only.** Reader mode (fetch → markdown) works
  everywhere Otto's UI runs; a real embedded page (the Live toggle, the
  element-picker overlay) needs a native Tauri child webview and simply stays
  in reader mode outside the desktop app (plain browser, PWA, remote share).
- Fetches are size-capped (`otto_browser::PAGE_BYTE_CAP`, streamed so a huge
  response is aborted mid-flight rather than buffered) and time-capped
  (`PAGE_TIMEOUT_SECS`).
- `/summarize` and `/vault-save`'s fresh-fetch path cap the markdown handed to the
  prompt at 30,000 chars; `/annotations/{id}/send` caps the excerpt at 2,000.
- `browser_navigate`'s title comes from a real fetch (it PATCHes the new tab with
  its URL, which runs the same reader-mode fetch pipeline `PATCH .../tabs/{id}`
  does), not from the create call.
- No cookies/sessions/auth are carried into a fetch — every request is anonymous,
  so pages behind a login return their logged-out view (or fail).
- `mode:"live"` tabs are never fetched by the daemon at all — they're a plain
  iframe URL/title record.

## Live tabs (desktop app)

Inside the Otto desktop app, flipping a tab to **Live** (the Reader/Live toggle
next to the URL bar) hosts a real Tauri child webview over the pane instead of
a fetched/rendered page — `window.__TAURI_INTERNALS__` gates this
(`ui/src/lib/nativeBrowser.ts`); off Tauri (a plain browser / PWA / remote
share) the same toggle simply keeps showing reader mode, since there's no
native webview to host. The daemon never fetches a live tab at all — see
"Capabilities & limits" above. The Tauri commands (`apps/desktop/src-tauri/src/
browser.rs`: `browser_open/bounds/navigate/eval/reload/show/hide/close/…`) are
shared with the right-panel Browser tab's own always-native tab strip
(`ui/src/modules/panels/BrowserPanel.svelte`) — both host webviews under the
same `otto-browser-<id>` label scheme, keyed by their own (non-overlapping) tab
ids, so the two can be open at once without one's cleanup tearing down the
other's tabs.

**Manual checklist** (desktop app only — a plain `cargo build`/`npm run
build` can't exercise the native webview itself):

1. Open the Browser module, paste a URL, hit **Go** — a reader tab opens.
2. Click the **Live** toggle — the pane switches to a real embedded page (no
   reader markdown fetch fires for it).
3. Resize the window / toggle the right panel / change app zoom — the live
   pane's webview stays aligned to the pane's rect.
4. Click a link inside the live page — the address bar tracks the in-page
   navigation without a page reload.
5. Close the tab (✕ in the tab strip) — the child webview is destroyed, not
   just hidden.

### Element picker overlay (live tabs)

A live tab gets its own "Mark element" equivalent: the crosshair button next
to the Reader/Live toggle. Unlike reader mode's click-to-annotate (which runs
inside Otto's own DOM), the live tab is a real native webview, and — same as
every other webview `nativeBrowser` opens — it's **denied Tauri IPC**, so a
page inside it has no way to call back into the app. The picker overlay
(`ui/src/modules/browser/overlay.js`) is therefore injected via
`browser_eval` (raw source text, `?raw`-imported — see the file's own header)
and driven by **polling**, not a callback: `BrowserView.svelte` calls
`window.__ottoOverlay.tick(highlightJson)` over `browser_eval` on an interval
whenever a live tab is active. `tick` applies the given highlight list (the
existing marks for that URL, so returning to a page re-highlights them) to the
page, then drains and returns whatever the overlay queued since the last poll
— marks made by clicking an element while picking is armed. The overlay is
re-injected on every navigation (a fresh page has a fresh JS context), and
re-injection restores pick mode if it was armed mid-navigation.

Marks made this way save immediately with no comment (a live page can't host
Otto's inline note composer) — add a comment afterward from the Marks rail,
same as any other mark. The selector algorithm
(`ui/src/modules/browser/selector.ts`, hand-duplicated in plain JS inside
`overlay.js` since the injected copy can't `import` it) prefers `#id`, then a
`data-testid`/`data-test`/`data-id`/`data-qa` attribute, then an nth-of-type
tag-path from `<body>` — unlike reader mode's own click-to-annotate (which
almost always falls through to the tag-path, since its sanitized render
rarely carries an id or test attribute from the original page).

**Manual checklist** (desktop app only):

1. Flip a tab to **Live**, navigate to a real page, click the crosshair
   button — hovering an element outlines it; clicking one adds a mark (dashed
   hover outline vs. a solid highlight box for a saved mark).
2. Check the Marks rail — the new mark appears with an excerpt, same as a
   reader-mode mark.
3. Reload the page (or navigate away and back) — the mark's highlight box
   reappears without re-clicking.
4. Click the crosshair button again to disarm picking — hovering/clicking no
   longer outlines or marks anything.

## Troubleshooting

- **A URL 400s immediately** — netguard blocked it (loopback/private/link-local/
  metadata address). This is intentional SSRF protection, not a bug.
- **Every page comes back `degraded:true`** — no working `lightpanda` binary was
  found/started; plain-fetch (no JS) is being used for everything. Check the
  daemon log for a `browser: lightpanda sidecar failed to start` warning.
- **One host always comes back `degraded:true` even though others don't** — it
  failed against the primary engine 3 times in a row and is denylisted; it clears
  automatically on the next success.
- **`browser_login` (or `POST /browser/login`) returns 502** — no working
  Lightpanda sidecar: login needs JS execution, so a fallback-only daemon can
  never satisfy it. Fix the engine per the `degraded:true` bullets above.
- **`browser_login` returns 404 / 403** — 404 means no Site Credential exists
  for that domain yet; 403 (`"credential not enabled for agents"`) means one
  exists but `allow_agent_use` is still `false` — flip it on in the
  Credentials panel first.
- **`browser_login` returns 429** — the per-domain rate limit (3/min) tripped;
  retry after the `Retry-After` window.
- **Live tab / element picker does nothing** — both require the desktop app's
  native Tauri webview; outside Tauri the Live toggle silently stays in
  reader mode (see "Capabilities & limits").
