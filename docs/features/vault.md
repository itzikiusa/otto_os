# Otto Vault — the docs home (file-backed markdown vaults, OKF)

The **Vault** is Otto's documentation home: point it at a folder of markdown
files on disk — including a **live Obsidian vault** — and Otto gives you an
Obsidian-parity knowledge base (file tree, editor ⇄ reading view, wikilinks,
backlinks, tags, full-text search, quick switcher, a scalable graph view) plus
things Obsidian doesn't have: a deterministic **OKF (Open Knowledge Format)**
validator, per-directory index generation, and **full MCP access** so every
agent session can read *and write* the same docs.

**Files are the source of truth.** The daemon keeps only a derived, rebuildable
SQLite index (notes, wikilinks/markdown links, tags, aliases, FTS5). There are
**no embeddings, no vector stores, and no remote backends anywhere** — Vault v3
deleted all of that (the v2 "Repo Brain": embedders, HNSW/Qdrant ANN, the
SurrealDB graph mirror, tree-sitter code intel, spawn-time brain injection).
"Smart indexing" now means agents writing linked, validated OKF docs that FTS
and the link graph make findable — durable, diffable, greppable.

This document is the end-user and operator reference for Vault v3. The design
(including its binding post-review amendments) is
`docs/superpowers/specs/2026-07-11-vault-docs-home-design.md`; the API contract
is `docs/contracts/api.md` → *Vault v3 — the docs home*.

---

## 1. Summary

| You want to… | The Vault gives you |
|---|---|
| Keep docs as plain files | A **vault = a registered local directory** of `.md` files; Otto never owns them |
| Use an existing Obsidian vault | Register its folder — wikilinks, tags, aliases, embeds all parse natively |
| Browse & edit | Virtualized file tree, CodeMirror editor with `[[` / `#` completion, autosave, reading view |
| Navigate knowledge | Backlinks with context snippets, outgoing links, outline, properties, quick switcher (⌘O) |
| Find things | FTS5 search with `tag:` / `path:` / `type:` operators; vault notes also appear in **global ⌘F** |
| See the shape of it | A Canvas2D graph view with a Web-Worker Barnes-Hut layout, built for 100k notes / millions of edges |
| Standardize docs | **OKF v0.1**: deterministic validator (E1–E3 / W1–W5), reserved `index.md`/`log.md`, concept templates, index generation |
| Let agents use it | `otto_vault_*` MCP session tools (read + Editor-gated write) and outward `otto.vault_*` control-plane tools |
| Stay safe | Deletion is a move to `<vault>/.trash/`; rename rewrites links across the vault; writes are optimistic-concurrency checked |

---

## 2. Overview & where it lives

| Layer | Path | Responsibility |
|---|---|---|
| Engine crate | `crates/otto-vault/` | Scan/parse/resolve/index, note CRUD, rename-with-link-rewrite, search, switcher, graph payloads, OKF validator |
| · parsing | `crates/otto-vault/src/parse.rs` | Frontmatter (YAML), wikilinks/md links/embeds, tags, headings, aliases — code fences excluded |
| · resolution | `crates/otto-vault/src/resolve.rs` | Obsidian shortest-path link resolution |
| · scanning | `crates/otto-vault/src/scan.rs`, `engine.rs` | Incremental (size+mtime) walk; freshness kicks |
| · OKF | `crates/otto-vault/src/okf.rs` | E1–E3 / W1–W5 conformance + `index.md` generation |
| · HTTP | `crates/otto-vault/src/http.rs` | The `/workspaces/{ws}/vault/*` router |
| Persistence | `crates/otto-state/migrations/0103_vault_docs.sql` | `vaults`, `vault_notes`, `vault_links`, `vault_tags`, `vault_files` (+ runtime FTS5 `vault_fts`); the same migration **dropped** the v2 vector/backends/code tables |
| MCP (session) | `crates/ottod/src/mcp_tools.rs` | `otto_vault_list/dir/read/search/backlinks/tags/graph/okf_validate` + `write/rename/delete` |
| MCP (outward) | `crates/otto-server/src/mcp_outward.rs` | `otto.vault_*` — reads default-enabled, writes approval-gated |
| Global search | `crates/otto-server/src/routes/search.rs` | ⌘F fans out to `vault_fts` (`kind: "vault_note"`) |
| UI | `ui/src/modules/vault/` | `VaultPage` + `FileTree`, `NoteView`, `mdRender`, `RightPanel`, `SearchPanel`, `TagsPanel`, `Switcher`, `NewNoteDialog`, `GraphView` + `graph.worker.ts`, store `vault.svelte.ts` |
| Contracts (authoritative) | `docs/contracts/api.md` → *Vault v3 — the docs home* | The REST surface; DTOs mirrored in `ui/src/lib/api/types.ts` |
| Bundled skills | `okf-authoring`, `vault-repo-docs` | The OKF doctrine for agents; "index a repo" = write a linked OKF bundle into the vault |

In the app, the Vault is the **Vault** item in the left nav (globe icon); the
route is `#/vault`. Vaults are a **global library** (like connections): every
workspace sees every vault — the `{ws}` in the routes is auth context only.
RBAC rides the existing **Product** feature class (reads = View, writes =
Edit — including View-gated read-shaped POSTs for `search` and
`okf/validate`).

### The derived index, in one screen

Per note the index stores: `path`, `title` (frontmatter or filename stem),
`okf_type`, `description`, the full `frontmatter_json` (unknown keys preserved),
`tags_json` (frontmatter list *or* scalar + inline `#tags`, nested `#a/b`
supported), `aliases`, `headings_json` (outline + `#anchor` targets),
`word_count`, `size`, `mtime_ns`, and a content `hash` (sha256 — change
detection + optimistic concurrency). Links are rows of
`(src_path, raw_target, dst_path, kind: wiki|md|embed, anchor, alias)` —
`dst_path NULL` means **unresolved** (legal; rendered dashed). Non-markdown
files (images, PDFs…) are indexed as attachments in `vault_files` and served
via the traversal-guarded `GET …/asset?path=` route. Any of it can be rebuilt
from disk by a rescan.

**Link resolution** follows Obsidian's shortest-path rules: exact relative path
→ vault-root-relative → **unique** basename match anywhere in the vault
(case-insensitive, `.md` optional); OKF's `/`-bundle-absolute links and
`%20`-encoded markdown links also resolve. An **ambiguous basename stays
unresolved** — surfaced in the unresolved count, never silently picked.

---

## 3. Setup — add a vault

No external setup, accounts, or dependencies. From the Vault page:

1. **Add a vault** (the empty-state button, or the vault switcher menu →
   *Add vault…*). Give it a **name** and either:
   - a **folder path** — registers an *existing* directory. Point it at a real
     Obsidian vault (`~/Documents/Obsidian/MyVault`); the scanner skips
     `.obsidian/`, `.git/`, `.trash/`, hidden files and `node_modules`, so
     Obsidian and Otto coexist on the same files; or
   - **leave the folder blank** — Otto creates `~/.otto/vault/<slug(name)>`.
2. Tick **OKF vault** (default on) to enable Open Knowledge Format validation,
   templates, and the OKF panel. It's a per-vault flag you can toggle later.
3. Registration kicks a **full scan**; the header shows "Indexing vault…" while
   `scan_state = scanning`, then note/link/unresolved counts.

You can register **multiple vaults** — all of them visible from every
workspace (global library; a root path can be registered only once). The
header button switches between them (the last selection is remembered
globally). **Unregister
vault (keeps files)** removes only the registration + index — files on disk are
never touched. **Rescan** (toolbar / context menu) forces a full incremental
pass.

### Freshness model (no filesystem watcher)

- **Writes are eagerly indexed** — every write/rename/delete/folder op through
  the API or MCP re-scans before returning, so API/MCP writers always read
  their own writes.
- **Reads self-heal**: `GET /status` and every read (search, backlinks, graph,
  dir, note — including the MCP tools) kick a **background incremental scan**
  when the index is **>5 s stale**, so agents with no UI open stay fresh too.
- The UI polls `/status` every 5 s while the page is visible; **edits made
  externally (e.g. in Obsidian) appear within one poll cycle.** Concurrent scan
  kicks coalesce; a scan diffs `(size, mtime)` and re-parses only changes, then
  re-resolves links touching the changed paths.

---

## 4. Walkthrough — the UI

Three-pane, Obsidian-style layout: left sidebar (Files / Search / Tags), center
(note or graph), right panel (Backlinks / Outgoing / Outline / Properties /
OKF). Both side panes are drag-resizable and persisted; the status bar shows
**backlinks · words · characters · OKF badge · vault root path**. On phones the
layout stacks (left pane becomes a top strip).

The center pane is **tabbed**: every opened note/file is a tab (⌘-click or
middle-click a tree row — or its context menu → "Open in new tab" — to open
WITHOUT replacing the current tab; × or middle-click a tab closes it). Tabs,
the active tab, and the center mode (note / file / graph / docs-agents) are
**persisted per vault** — switching to another module and back, or fully
restarting the app, restores exactly the view you left, including a
docs-agents run you were watching. When any docs-agent run is active, the
topbar shows a pulsing **"N agent runs active"** chip regardless of the
current view — click it to jump to the runs.

### 4.1 File explorer

A **virtualized, lazily-loaded tree** (directory levels fetch on expand — big
vaults stay cheap). Row click opens a note; context menu:

- **New note here / New folder here** (folders only)
- **Rename** (inline) and **Move to…** (path prompt) — both are the same
  server-side rename: the file/folder is moved on disk, then **every
  referencing wikilink and markdown link across the vault is rewritten on
  disk** (aliases and `#anchors` preserved, link style preserved); a toast
  reports "N links updated". Rename refuses to overwrite an existing target;
  case-only renames use a two-step move (APFS is case-insensitive); after a
  rename the whole vault re-resolves, so a basename that just became ambiguous
  surfaces as unresolved rather than silently re-pointing.
- **Delete (→ .trash)** — a **soft delete**: the note moves to
  `<vault>/.trash/…` inside the vault. Nothing is ever destroyed.

You can also **drag a file onto a folder** to move it (same link-rewriting
rename). Attachments appear in the tree; reserved OKF files (`index.md`,
`log.md`) are styled dimmer. **⌘N** (or the + toolbar button) opens the
new-note dialog — in an OKF vault it offers a **concept template** picker
(Service / Reference / Decision / Runbook / Playbook / Metric / Dataset) that
pre-fills `type/title/description/tags/timestamp` frontmatter plus
`# Overview` / `# Citations` sections.

### 4.2 Editor ⇄ reading view

Each note opens with a breadcrumb and an **edit ⇄ read toggle** (**⌘E**); the
chosen mode is remembered per vault.

**Editing** is CodeMirror (markdown), with:

- **Autosave** — 800 ms debounce after you stop typing, plus **⌘S** to save
  now. Saves send the note's last-known content hash (`if_hash`).
- **Conflict banner** — if the file changed on disk meanwhile (Obsidian, an
  agent, another session), the write returns **409** and a banner offers
  **Reload disk version** / **Overwrite**. It never auto-retries.
- **`[[` wikilink completion** — live, server-side fuzzy over titles,
  **aliases**, and paths (the same endpoint as the quick switcher). Picking an
  alias inserts `[[File|alias]]`.
- **`#` tag completion** from the vault's existing tags (with counts).

**Reading view** renders GFM through an **allowlist sanitizer** (no scripts,
iframes, or event handlers survive), with the Obsidian constructs:

- **Wikilinks** in all forms: `[[note]]`, `[[note|alias]]`,
  `[[note#heading]]` (opens and scrolls to the heading), `[[#heading]]`
  (same-note anchor), and `#^block` block anchors (parsed and resolved to the
  note). Unresolved links render dashed at reduced opacity — **click one to
  create that note**.
- **Embeds** — `![[note]]` renders the target inline in a bordered card
  (depth 1, cycle-guarded — an embed never re-embeds); `![[image.png]]` and
  `![](image.png)` render the attachment via an authenticated blob URL.
- **Markdown links** — `[text](other-note.md)` (OKF's link form) resolves like
  a wikilink, `%20` decoded; external URLs open in a new tab.
- **Callouts** — `> [!note]`-style blockquotes get a styled card (warning /
  caution / danger / bug variants get their own colors).
- **`%%comments%%`** — hidden in reading view (single- and multi-line, outside
  code fences), kept verbatim in the raw file.
- **Tags** — inline `#tag` renders as a chip; clicking it runs a `tag:` search.
- **Frontmatter** is stripped from the body (the Properties panel shows it);
  code blocks are syntax-highlighted.
- **Diagrams** — ` ```mermaid ` and ` ```d2 ` fences render as live diagrams
  (lazy-loaded, same engines as the Canvas). A **bare fence whose first line
  is a mermaid grammar keyword** (`flowchart LR`, `sequenceDiagram`, …) also
  renders — agents often omit the language tag. Parse errors keep the source
  visible with the error message above it.

### 4.2b Non-markdown file viewers

Clicking a non-`.md` file in the tree opens it in a matching viewer (same
tabs / persistence as notes):

- **OpenAPI / Swagger** (`.json`/`.yaml`/`.yml` with an `openapi`/`swagger` +
  `paths` root) — a structured spec view: info header, servers, operations
  grouped by tag with method chips, parameters table, request-body schema
  tree ($ref-resolved, cycle-guarded), examples, and responses. A **Source**
  toggle shows the raw file.
- **JSON** — pretty-printed + syntax-highlighted.
- **CSV / TSV** — rendered as a table (quote-aware, first 5000 rows).
- **Images** (`png/jpg/gif/webp/svg/…`) and **PDF** — displayed inline via an
  authenticated blob URL.
- **Everything else** — syntax-highlighted code (language from the
  extension), capped at 10k lines / 2 MB with a truncation notice.

### 4.3 Right panel — backlinks, outgoing, outline, properties

- **Backlinks** ("linked mentions") — every note linking *to* this one, each
  with a **context snippet**; click to jump. The count also shows in the
  status bar.
- **Outgoing links** — this note's links (embeds marked `⧉`, unresolved
  flagged).
- **Outline** — the heading tree; click to scroll the reading view.
- **Properties** — the frontmatter as a key/value table (OKF fields —
  `type`, `title`, `description`, `resource`, `tags`, `timestamp` — are
  highlighted in OKF vaults; unparseable YAML shows a warning). Properties are
  edited in the editor, not in the table.
- **OKF card** (OKF vaults) — see §4.6.

### 4.4 Search & tags

Left-sidebar **Search** mode runs FTS5 (`bm25`-ranked) over titles + bodies
with **highlighted snippets**. Operators compose inside the query:

```
deploy tag:runbook          # full-text AND tag filter
path:services/ kafka        # restrict to a subtree
type:Decision retention     # OKF type filter
```

**Tags** mode lists every tag with its count (frontmatter + inline, nested
`a/b` tags included); clicking a tag jumps to a `tag:` search. Notes **>4 MiB**
are indexed metadata-only (title/links/tags but no FTS body) so a giant log
can't bloat the index.

Vault notes are also **first-class results in the global ⌘F search**
(`kind: "vault_note"`), routed back to `#/vault` with the right vault + note
selected.

### 4.5 Quick switcher (⌘O)

**⌘O** opens the switcher: server-side fuzzy matching over **title, aliases,
and path** (subsequence scoring), so a 100k-note vault never ships its full
note list to the client. Alias hits display as `alias → real title`. **Enter**
opens; **Shift+Enter** (or Enter with no hits) **creates a note by that name**.
Reserved OKF files are excluded from switcher results.

### 4.6 The OKF card — Open Knowledge Format

OKF v0.1 is the vault's documentation standard: markdown + YAML frontmatter
concepts, `index.md`/`log.md` reserved files, markdown links between concepts.
For OKF vaults the right panel's **OKF** section (and the status-bar badge)
exposes:

- **Validate** — the deterministic conformance checker ("never eyeball
  conformance"). Findings are clickable (opens the offending note):

  | Rule | Severity | Meaning |
  |---|---|---|
  | E1 | error | missing or unparseable frontmatter |
  | E2 | error | missing/empty `type` |
  | E3 | error | reserved-file structure (only the bundle-root `index.md` may carry frontmatter, and only `okf_version`; `log.md` never) |
  | W1 | warning | missing `title` or `description` |
  | W2 | warning | broken internal link |
  | W3 | warning | no `timestamp` |
  | W4 | warning | directory without an `index.md` |
  | W5 | warning | `log.md` `##` headings not ISO `YYYY-MM-DD` |

  Warnings never fail a bundle — permissive consumption is intentional, and a
  malformed note is indexed with a parse-error flag rather than aborting a scan.
- **Generate indexes** — regenerates per-directory `index.md` files from the
  notes' frontmatter descriptions (root index carries `okf_version`).

`index.md`/`log.md` are flagged `reserved` everywhere: excluded from the
switcher and (by default) the graph, never validated as concepts, searchable
but marked. The OKF `type` drives the `type:` search operator and graph
grouping. Two bundled skills make agents fluent: **`okf-authoring`** (the full
OKF doctrine — produce/maintain/consume, augment-don't-rewrite, validate via
tool not eyeballs) and **`vault-repo-docs`** ("index a repo" = read the code
and write a linked OKF bundle — services/, endpoints/, decisions/, runbooks/ —
into the vault, then validate).

### 4.7 Graph view

The graph toolbar button switches the center pane to a **Canvas2D graph of the
whole vault** — engineered so scale is a rendering problem, not a feature
limit (design budget: **100k nodes / 1–2M edges on an M-series laptop**):

- **Wire format** — one compact JSON payload of parallel arrays
  (`paths/titles/groups/flags` + a flat `[src,dst,…]` edge index list); ~1M
  edges is 8–14 MB of local JSON, no per-object overhead.
- **Layout** — a Web Worker (`graph.worker.ts`) runs Barnes-Hut quadtree
  repulsion + springs + gravity over `Float32Array`s, posting positions back as
  transferable, double-buffered buffers; above 30k nodes it seeds positions by
  group cluster so layout converges in bounded time. **Stop/Resume layout** is
  a button; dragging pins a node (double-click unpins; double-click the
  background re-fits).
- **Rendering honesty** — every edge exists and is reachable, but past a
  ~150k-segment per-frame draw budget the renderer **deterministically samples
  edges and fades them**; zooming in restores full detail through viewport
  culling. Labels are budgeted to the top-degree nodes in view and fade in with
  zoom (the *Text fade* slider). Node size scales with degree. This is the same
  trade Obsidian makes, at a much higher ceiling — full simultaneous rendering
  of 2M edges is beyond any tool.
- **Filters** — a title filter (dims non-matches, client-side) and server-side
  toggles: **Tags** (tag nodes), **Orphans**, **Unresolved** (ghost nodes),
  **Reserved files**; **Group by** folder or OKF `type` colors clusters with a
  stable palette.
- **Forces / Display** — Obsidian-parity sliders: center / repel / link force /
  link distance (live-tuned in the worker), node size / link width / text fade
  (render-only).
- **Full-graph edge budget** — `mode=full` enforces a server-side,
  degree-prioritized edge budget (default 2M, `?edge_budget=` override); the
  status strip shows a **truncated** chip when it was hit.
- **Local graph** — the server does BFS neighborhoods (`mode=local`, `path=`,
  `depth ≤ 3`) so the common case never ships the whole graph; the GraphView
  component supports a local mode with a depth slider, and local is the
  **default mode of the `otto_vault_graph` MCP tool**. The shipped Vault page
  currently mounts the full-vault graph; local neighborhoods are served over
  the API/MCP.

Clicking a (non-ghost, non-tag) node opens the note.

---

## 5. API surface

Authoritative contract: `docs/contracts/api.md` → **Vault v3 — the docs home**
(DTOs mirrored in `ui/src/lib/api/types.ts`). The shape in brief — all under
`/api/v1/workspaces/{ws}/vault/…`, reads `ws viewer`, writes `ws editor`:

| Area | Routes |
|---|---|
| Vaults | `GET/POST /vault/vaults`, `PATCH/DELETE /vault/vaults/{id}`, `POST …/rescan`, `GET …/status` |
| Files | `GET …/dir?path=`, `GET/PUT/DELETE …/note`, `POST …/rename` (→ `{links_updated}`), `POST …/folder`, `GET …/asset?path=` |
| Knowledge | `GET …/backlinks?path=`, `POST …/search`, `GET …/switcher?q=`, `GET …/tags`, `GET …/graph?mode=full\|local&…` |
| OKF | `POST …/okf/validate`, `POST …/okf/indexes` |

Notable semantics (see the contract for the full notes): `PUT …/note` takes
`if_hash` for optimistic concurrency (`""` = must-not-exist; mismatch → 409)
and auto-creates parent folders; `DELETE …/note` moves to `.trash/`;
`DELETE /vault/vaults/{id}` unregisters **only** (files untouched); every file
op canonicalizes paths and rejects traversal/symlink escapes.

There are **no vault WebSocket events** — the UI stays fresh via the 5 s
status poll + scan-completion refreshes.

## 6. MCP surface — agents read & write the docs home

**Session tools** (the first-party `otto` server injected into agent sessions
via `.mcp.json` / codex `--config`; calls run **as the session owner**, so
workspace RBAC applies — an agent can only do what its owner could):

| Tool | Kind | Maps to |
|---|---|---|
| `otto_vault_list` | read | list vaults (+counts, scan state) |
| `otto_vault_dir` | read | one level of the tree (dirs / notes / attachments) |
| `otto_vault_read` | read | note raw + meta + outgoing (+backlinks inline) |
| `otto_vault_search` | read | FTS search (`tag:`/`path:`/`type:` operators) |
| `otto_vault_backlinks` | read | linked mentions with snippets |
| `otto_vault_tags` | read | tag counts |
| `otto_vault_graph` | read | graph payload (**local** neighborhood by default; `mode=full` opt-in) |
| `otto_vault_okf_validate` | read | the deterministic OKF report |
| `otto_vault_write` | **write** | create/update a note (Editor-gated; parent folders auto-created; `if_hash` honored) |
| `otto_vault_rename` | **write** | move + rewrite links across the vault |
| `otto_vault_delete` | **write** | soft delete → `.trash/` |

These three writers (plus the two canvas tools) are the only mutating tools in
an otherwise read-only session-MCP surface. Every read runs the >5 s staleness
check first, so agents always see fresh indexes.

**Outward tools** (`otto.vault_*` on the outward MCP server, for external MCP
clients): the eight reads are in the **default-enabled** set once the server is
on; `otto.vault_write` / `otto.vault_rename` / `otto.vault_delete` are
classified **DANGEROUS** — off by default and approval-gated by the control
plane like every other write. See
[`./mcp-control-plane.md`](./mcp-control-plane.md).

## 6b. Docs agents — AI writes the documentation

Two agent surfaces live directly in the vault, reusing the standard agent
infrastructure (managed sessions with the otto MCP tools, provider/model
selection from the shared registry, inline live terminals):

- **Docs agent (create)** — the ✨ toolbar button (or a folder's context menu →
  "Docs agent here"). Configure 1–4 writer agents (per-agent provider + model)
  and a summarizer, describe what to document, Run. **Prepared prompts**: a
  template picker (repo deep-dive, flow catalog, API+OpenAPI, datastores
  audit, messaging map, incremental "changes since last scan") — pick one,
  fill the repo path, Insert, edit freely. Templates demand a flow inventory
  and ONE NOTE PER FLOW (all flows) with an explicit anti-bloat bar, and they
  attach library skills to the run (`RunReq.skills`, e.g. `vault-repo-docs`);
  the form shows every injected skill as a clickable chip that opens the
  skill's full text. Full scans record `commit:`/`scanned_at:` in
  overview.md's frontmatter so the update template can diff from there. Writers fan out as REAL
  sessions (each row has an Open toggle mounting its live terminal, multiple at
  once). With one writer it writes final notes straight into the target folder;
  with several, each drafts under `_drafts/docs-run-*/agent-N/` and the
  summarizer consolidates into final notes, after which drafts are moved to
  `.trash/`. Finished runs list every written note as a link. Runs are
  **durable** (`vault_docs_runs`): the Runs section lists current + history
  newest-first and survives tab switches and daemon restarts (a restart flips
  non-terminal runs to `interrupted` and soft-trashes orphaned drafts). While
  anything is running the vault topbar shows the pulsing **"N agent runs
  active"** chip from any view.
- **Refine with AI (edit)** — the ✨ button in an open note's header opens a
  drawer: one resumable session per note; each Send applies an instruction to
  the note via `otto_vault_write` with optimistic `if_hash` (conflicts re-read
  and re-apply). The note view reloads after each turn unless you have unsaved
  edits.

**OKF enforcement**: on OKF vaults every agent is instructed to produce
conformant notes (frontmatter `type` + one-sentence `description`, markdown
links, index/log conventions) — claude sessions get the bundled `okf-authoring`
skill via `--add-dir`, codex/agy get the skill text inlined — and the
summarizer must run `otto_vault_okf_validate` and fix every error before
finishing.

Routes: `POST …/vault/vaults/{id}/docs-agents/run`, `GET /vault/docs-agents/runs/{run_id}`,
`POST /vault/docs-agents/runs/{run_id}/cancel`, `POST …/docs-agents/refine`,
`GET …/docs-agents/refine-session?path=` (see `docs/contracts/api.md`).

## 7. The memories layer (what remains of v1/v2)

The **workspace memory store** (`otto-memory`: `memories` / `memory_links`,
lifecycle + governance, Product story ingest, `otto_search_memory`) is a
separate feature that **stays** — but it is now **keyword-only** (FTS5 → LIKE
fallback). `MemoryQuery.mode` still accepts `hybrid` / `semantic` as tolerated
aliases, and **all modes execute the keyword path** — no embeddings are
computed anywhere. Memories have **no Vault UI anymore** (the vault page is
docs-only); the Product page and MCP/API are the memory consumers. The optional
`OTTO_MEMORY_VAULT_DIR` markdown mirror and the `OTTO_MEMORY_REMOTE_URL`
keyword-proxy remain. Contract: `docs/contracts/api.md` → *Memory layer*.

---

## 8. Capabilities & limitations

**Capabilities**

- Multiple vaults, global across workspaces; a vault is any local folder of markdown —
  a live Obsidian vault works unmodified, and stays portable (nothing Otto adds
  to your files except edits you ask for).
- Derived-only index: everything in SQLite is rebuildable from disk; `.trash/`
  soft deletes; rename rewrites links vault-wide on disk.
- Obsidian-parity reading/editing: wikilinks (all forms), embeds, callouts,
  `%%comments%%`, tags, aliases, unresolved-link create, per-vault edit/read
  mode, autosave with conflict detection.
- FTS5 search with operators; server-side fuzzy switcher; scalable graph.
- OKF: deterministic validation, index generation, templates, reserved files,
  `type`-driven grouping/filtering; agent skills for authoring + repo docs.
- Full MCP read/write access with RBAC + approval gates.

**Limitations / honest caveats**

- **No filesystem watcher.** External edits are picked up by the freshness
  model (§3): within one 5 s poll cycle while the page is open, or at the next
  API/MCP read. They are not pushed instantly.
- **Notes >4 MiB** are indexed metadata-only — no full-text body search.
- **Ambiguous basenames stay unresolved** by design (never silently picked);
  fix by qualifying the link path.
- **Reserved files** (`index.md`/`log.md`) are excluded from the switcher and
  (by default) the graph; searchable but flagged.
- **Graph LOD**: past the draw budget, edges are sampled/faded until you zoom
  in; `mode=full` may report `truncated` when the server edge budget is hit.
- The shipped Vault page mounts the **full** graph; local neighborhoods are an
  API/MCP capability (the component's local mode isn't currently mounted).
- **No semantic search** — that's the point. Recall = FTS + links + tags +
  types. (The memory layer likewise accepts but coerces `semantic`/`hybrid`.)
- Frontmatter is edited as text in the editor; the Properties panel is
  read-only.

**Non-goals** (deliberate — plugin territory, not core parity): canvas boards
(Otto has its own [Canvas](./canvas.md) module), daily notes, sync/publish,
PDF annotation, community plugins.

## 9. Security & permissions

- **RBAC** — the `/workspaces/{ws}/vault/*` prefix rides the **Product**
  feature class: reads need View, mutations Editor; the read-shaped POSTs
  (`search`, `okf/validate`) are explicitly View-gated.
- **Path safety** — every file op canonicalizes and guards: no `..`, no
  absolute paths, no hidden/`.trash/` segments, symlink escapes rejected;
  writes are restricted to the vault root. Attachments are served through the
  same guard.
- **Never destructive** — delete = `.trash/` move; unregister touches no files;
  rename refuses to overwrite an existing target.
- **Sanitized rendering** — reading-view HTML passes an allowlist sanitizer
  (`ui/src/lib/sanitize.ts`): no `script`/`style`/`iframe`/`on*` handlers /
  `javascript:` URLs, so a hostile note can't script the app.
- **MCP writes are governed** — session tools act as the session owner
  (Editor gate applies); outward write tools are off by default and
  approval-gated.
- **Local-first** — files and the index never leave the machine; no API keys,
  no network calls, nothing to configure.

## 10. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Header shows **"index error"** / routes return 409 with a scan message | The vault root disappeared (unmounted disk, moved folder) — `scan_state = error:<msg>`. Restore the folder or unregister the vault (files elsewhere are untouched). |
| **409 on save** / a yellow banner appears | The note changed on disk while you edited (Obsidian, an agent, another session). The conflict banner offers **Reload disk version** or **Overwrite** — nothing auto-retries. |
| Edits made in Obsidian don't show immediately | No fs watcher — they appear within one 5 s status-poll cycle while the page is open (or at the next read). Hit **Rescan** to force it. |
| A link renders dashed (unresolved) after a rename | The rename made that basename ambiguous (two files now share it) — vault-wide re-resolve surfaces this instead of guessing. Qualify the link with its path. |
| Clicking an unresolved link asks to create a note | That's the feature — Obsidian-style click-to-create for ghost links. |
| **"Diagram error: Refused to evaluate a string as JavaScript…"** on a D2 diagram | The packaged app's CSP blocked D2's engine — it needs `'wasm-unsafe-eval'` (WASM) and `'unsafe-eval'` (its worker's ELK layout loader uses `new Function`). Both are in `apps/desktop/src-tauri/tauri.conf.json`'s `script-src` **deliberately**: script SOURCES stay `'self'`-only and rendered markdown is sanitized, so don't remove them without replacing the D2 engine. Mermaid never needed either. |
| A diagram shows "Diagram error: Parse error …" with the source | The agent wrote invalid mermaid (unquoted special chars in labels is the usual cause). The source stays visible by design; the prepared-prompt templates instruct agents to verify fences before finishing. |
| Big note isn't found by body search | Notes >4 MiB are metadata-only in the index (title/tags/links still work). |
| `index.md` missing from switcher/graph | Reserved OKF files are excluded by default; the graph has a **Reserved files** toggle. |
| Graph shows a **truncated** chip | The full-mode server edge budget (default 2M, degree-prioritized) was hit; use the filters/local mode or raise `?edge_budget=`. |
| Writes fail with 403 | Vault mutations need workspace **Editor** (Product:Edit); reads only View. |
| OKF errors on `index.md`/`log.md` frontmatter | E3: only the bundle-root `index.md` may carry frontmatter, and only `okf_version`; `log.md` never has frontmatter. |

## 11. Related docs

- `docs/contracts/api.md` — **Vault v3 — the docs home** (authoritative REST
  surface) and **Memory layer** (the keyword memory store that remains).
- `docs/superpowers/specs/2026-07-11-vault-docs-home-design.md` — the v3 design
  + binding amendments.
- [`./mcp-control-plane.md`](./mcp-control-plane.md) — how the `otto_vault_*` /
  `otto.vault_*` tools are surfaced and governed.
- [`./product.md`](./product.md) — Product's story→memory ingest (the memory
  layer, not the docs vault).
- [`./canvas.md`](./canvas.md) — Otto's canvas module (why canvas boards are a
  vault non-goal).
- Bundled skills: `okf-authoring` (write/validate OKF in the vault),
  `vault-repo-docs` (document a repo as an OKF bundle).
