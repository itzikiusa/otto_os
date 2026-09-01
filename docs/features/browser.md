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
  `mode:"reader"` (fetched/rendered) or `mode:"live"` (an embedded iframe the
  daemon never fetches). Navigating a reader tab re-fetches and adopts the new
  page's title.
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
  `browser_summarize`, so an agent session can drive the same fetch pipeline
  without going through the UI.

## Setup

Nothing to configure to get plain-fetch browsing working — it's on by default.
For JS-rendered pages, Otto looks for a `lightpanda` binary (configurable path) and
starts it as a sidecar on first use; if it isn't found or fails to start, every
fetch transparently falls back to a script-free plain fetch (`degraded:true` on the
response) rather than failing. A host that fails against the primary engine three
times in a row is skipped straight to the fallback until it next succeeds.

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

WS events: `browser_tab_updated{tab}`, `browser_annotation_added{annotation}` (see
`docs/contracts/ws.md`).

## Agent MCP tools

Every tool-capable session gets four `browser_*` tools alongside the rest of
Otto's first-party MCP surface (`crates/ottod/src/mcp_tools.rs`), authorizing as
the session's own owner (the same `WorkspaceRole::Editor` gate a human hits — no
more):

| Tool | Args | Returns |
|---|---|---|
| `browser_navigate` | `url` | `{ok,title}` — opens a reader tab (the only mutating browser tool: it creates a `browser_tabs` row) |
| `browser_page` | `url` | `{markdown,title,engine,degraded}` |
| `browser_query` | `url, selector` | `{matches:[{selector,outer_html,text}]}` |
| `browser_summarize` | `url` | `{summary,engine,degraded}` |

`browser_page` drops the route's `url`/`html` fields from what the agent sees — the
caller already has the URL, and the raw markup is large relative to the extracted
markdown most agents actually want. `browser_summarize` doesn't persist anything:
the summarize turn runs in an ephemeral, unresumed session (same pattern as
`db_assist`), so it's treated as a read even though it's a `POST`.

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

## Troubleshooting

- **A URL 400s immediately** — netguard blocked it (loopback/private/link-local/
  metadata address). This is intentional SSRF protection, not a bug.
- **Every page comes back `degraded:true`** — no working `lightpanda` binary was
  found/started; plain-fetch (no JS) is being used for everything. Check the
  daemon log for a `browser: lightpanda sidecar failed to start` warning.
- **One host always comes back `degraded:true` even though others don't** — it
  failed against the primary engine 3 times in a row and is denylisted; it clears
  automatically on the next success.
