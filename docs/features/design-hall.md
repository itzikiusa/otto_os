# Design Hall — the artifact graph

> **Status: Phase 0 backend.** The graph, its REST/WS surface, the legacy
> import and the read-only agent tools ship now; the Lobby UI, the unified
> `design_assist` agent turn, variants, export/publish and learning are later
> phases (see the proposal's roadmap). The existing Product → Design arena and
> Canvas keep working unchanged meanwhile.

Design Hall is the home for everything visual a product team makes — UI
frames, marketing graphics, sites, 3D scenes, whiteboards and brand kits —
built on **one artifact graph**:

- **Projects** — named collections of artifacts (the unit the Lobby browses),
  optionally bound to an epic (`epic_story_id`) and/or a swarm project.
- **Artifacts** — one design each, in a **studio** (`frames`, `graphics`,
  `site`, `3d`, `whiteboard`, `brand`, `spatial`) and a **format** (`html`,
  `mermaid`, `d2`, `excalidraw`, `scene3d`, `otto-canvas`, `otto-site`,
  `otto-layout`, `otto-brand`, `otto-exhibit`, `svg`, `png`/`jpeg`/`gif`/`webp`,
  `pdf`, `glb`, `gltf`), with a review **status** (`draft` → `review` →
  `approved` → `shipped`, or `archived`).
- **Versions** — every content save commits an immutable version; nothing is
  ever overwritten in place.
- **Links** — typed, version-aware edges between artifacts (and to stories,
  sessions, PRs, URLs …), extracted from the documents themselves plus explicit
  links.
- **Signals** — design decisions captured from day one (accepted/rejected
  variants, edits after an agent draft, review comments, brand corrections,
  a11y fixes, status changes, shipped). Nothing learns from them yet.

Code: `crates/otto-design` (router, service, store, extractor, import),
`crates/otto-server/src/design_hall.rs` (glue), migration `…_design_graph.sql`.
Contract: [`docs/contracts/api.md` § Design Hall](../contracts/api.md) and the
`design_*` events in [`ws.md`](../contracts/ws.md).

## 1. Setup

Nothing to configure. On every daemon start Otto:

1. creates the FTS5 search index `design_search_fts` (if the linked SQLite has
   no FTS5, search falls back to a LIKE scan over titles and tags), and
2. runs the **legacy import** in the background (see §4).

Access is the `design` feature grant (Settings → Users → Design Hall): View
reads, Edit writes, Admin runs the import/prune actions. The migration grants
`design` to every user who had a `canvas` grant, at the same level. Root always
has access. The workspace role still applies per artifact (Viewer reads,
Editor writes, Admin hard-deletes).

## 2. Storage model

```
<data>/design/
  blobs/<sha256>            immutable, content-addressed (dedup'd) version + thumbnail bytes
  <artifact_id>/work/<file> the editable working copy (text/JSON formats only)
otto.db
  design_projects · design_artifacts · design_versions
  design_links · design_signals · design_publishes
  design_search_fts         (runtime FTS5, derived — rebuildable)
```

- A save writes the bytes to the blob store (skipped when the blob exists),
  then — in one SQLite transaction — moves the artifact's head and inserts the
  version (`seq` 1, 2, 3 … per artifact). The working copy is mirrored after.
- The working copy is where an agent edits in place; `POST …/versions`
  snapshots it as a **named** version.
- Byte-identical saves write nothing (`created: false`).
- Rows hold metadata only; there are no foreign keys (child rows are removed
  explicitly), so the saved-state archive can restore tables in any order.

### Retention — opt-in only

The daemon never deletes a version or a blob on its own. `POST
/design/admin/prune` (dry run unless `apply: true`) squashes `autosave`
versions to the last one per editing window (default 10 minutes). It never
touches the head, the approved version, versions a link pins or was
extracted from, published or publish-pinned versions, versions a signal
cites, or any non-autosave version (`named`, `agent`, `import`, `sync`,
`restore`). On apply it garbage-collects blobs no version or thumbnail
references any more. Pruning a single artifact needs workspace Admin; the
whole library needs root.

## 3. Walkthrough

### Create and save

```http
POST /api/v1/design/projects        {"workspace_id":"W","name":"Rewards+"}
POST /api/v1/design/artifacts       {"workspace_id":"W","project_id":"P","format":"html",
                                     "title":"Landing","content":"<h1>Win</h1>","story_id":"S"}
→ 201 {artifact, version: {seq: 1, kind: "named"}, created: true, links: {…}}
PUT  /api/v1/design/artifacts/A/content {"content":"<h1>Win more</h1>","base_version":"<v1 id>"}
→ 200 {… version: {seq: 2, kind: "autosave"}}
```

`base_version` is the head the editor loaded; if someone (an agent, another
tab) saved in between you get **409** and should reload — no silent clobber.
Omit it for an unconditional save. Content is validated before anything is
written: UTF-8 for text formats, a JSON object for JSON formats, a magic-byte
sniff for binaries, 25 MB raw (40 MB request body for base64), and `scene3d`
documents go through the same schema check as the Product arena.

`derived_from: {artifact_id, version_id?}` forks an existing version (default:
its approved version, else its head) and records a pinned `derived_from`
link — the "Start from this" action of the References drawer.

### Versions

`GET …/versions` lists them newest first; `GET …/versions/v3/content` (or a
version id) returns the bytes. `POST …/versions {"message":"Hero v2"}` makes a
named commit from the working copy. Restoring an older version is simply
saving its bytes again — history never rewinds.

### Approve and propagation

`POST …/approve {version_id?}` moves `approved_version_id` (default: head) and
sets status `approved`. Consumers that embed the artifact with the default
`follow_approved` policy receive `design_link_updated {reason:
"target_approved"}` so their UI can show "now v8"; `follow_latest` consumers
hear about every new head (`target_updated`); `pinned` consumers never move.
Only humans approve — the route is Editor-gated and no agent tool writes it.

### Links

Documents reference other artifacts only through a URI:

```
otto://design/<artifact_id>[@approved|@latest|@v<seq>][#<node_id>]
```

Every save re-extracts them (html `src=` → `embeds`, other attributes →
`references`; Mermaid `click Node "otto://…"` / D2 `node.link:` → `describes`;
`scene3d` `gltf.attachment_id` → `embeds`; JSON formats by key — `src`
embeds, `component` uses_component, `brand`/`tokens` uses_tokens, …). The
`@` selector sets the policy (`@approved` follow_approved, `@latest`
follow_latest, `@vN` pinned); without one, render links follow the approved
version and provenance links are pinned. The save's `links` report lists:

- `broken` — malformed URIs, missing artifacts/versions/nodes (stored with
  `broken: true` so "Uses"/"Used in" can show a badge; never a crash);
- `cycles` — render links (embeds / uses_component / uses_tokens) that would
  close a cycle: reported, **not stored**;
- `depth_exceeded` — render chains deeper than 4 (stored; rendering stops at 4).

Explicit links (`POST …/links`) cover what documents can't say:
`implements` a story, `references` a URL, `describes` frames … A render link
that would close a cycle is a 409. Extracted links can't be deleted over the
API (409) — edit the document.

`GET …/links?dir=out|in|both` returns the links plus the artifacts on the
other end that you can view.

### Search (the References drawer)

`GET /design/search?q=hero card&studio=site&status=shipped` — FTS5 over
titles, tags, extracted text (HTML copy, Mermaid/D2 source, Excalidraw text,
3D object names and notes, brand token names), linked story keys + titles and
the project name. Terms are AND-ed, the last one prefix-matched; results come
shipped → approved → review → draft, then by relevance, each with a snippet,
`reference_count` and the `story_ids` it implements.

### Signals

`POST /design/signals {artifact_id, kind, version_id?, payload}` records a
bounded signal (≤ 8 KB JSON object). Captured automatically:
`edit_after_draft` (a human save within 2 h of an agent version — the payload
is a structural summary, never the content), `status_change` and `shipped`.
`GET /design/signals` reads the log.

## 4. Legacy import

Idempotent, runs at startup and on `POST /design/admin/import`:

| Source | Becomes | Links |
|---|---|---|
| `product_attachments` of kind `design`/`mockup`, plus every `image/*` and `model/gltf-*` attachment | one artifact (format from the mime; studio from the format; the arena's `group` as a tag) | `implements` → its story; Blender outputs (`meta.derived_from`) `derived_from` → the mirrored source |
| `canvas_scenes` | one `otto-canvas` whiteboard artifact (the scene document verbatim; section as a tag) | `implements` → its story, if any |

- **Nothing moves.** The original rows, files and routes are untouched; the
  graph row records the original id in `source_kind`/`source_id` (unique) and
  in `meta.imported_from`.
- A source whose `updated_at` is unchanged is skipped without reading its
  bytes. A changed source gets a new `sync` version (history kept). A source
  that disappears leaves its artifact alone.
- Imported artifacts start unfiled (`project_id: null`) with a v1 of kind
  `import`, authored by `system:import`.

## 5. Agents

Four read-only tools let agents find and cite earlier work:

| stdio (`ottod mcp-tools`) | governed (`otto.*`, default-enabled) | route |
|---|---|---|
| `design_list` | `otto.design_list` | `GET /design/artifacts` |
| `design_get` | `otto.design_get` | `GET /design/artifacts/{id}?content=true[&version=]` |
| `design_links` | `otto.design_links` | `GET /design/artifacts/{id}/links` |
| `design_search` | `otto.design_search` | `GET /design/search` |

`design_get` returns the text source (≤ 256 KiB), the working-copy path and the
thumbnail's file path (a PNG in the blob store) so a multimodal agent can look
at it. Agents should cite what they borrow as `otto://design/<id>@v<seq>`.

## 6. Capabilities & limits

- Phase 0 has no UI of its own yet; the Product Design tab and Canvas keep
  their own storage and routes. Edits made there reach the graph through the
  import's `sync` versions (next start or `POST /design/admin/import`), not
  live.
- `otto-site`, `otto-layout`, `otto-brand`, `otto-exhibit` are stored and
  link-indexed as generic JSON; their own validators land with their studios.
- `#node` validation covers JSON formats (any `"id"`) and HTML/SVG (`id="…"`);
  Mermaid/D2 nodes are accepted as-is.
- Brand-token references (`token:color.primary`) are not resolved yet.
- The `edit_after_draft` summary is a bounded heuristic (changed JSON paths /
  line counts); the per-format structural diff arrives with Compare.
- Design blobs are not part of the saved-state archive's file roots yet (the
  DB rows are); back up `<data>/design/` with the data dir.
- Content caps: 25 MB raw per version, 4 MB inline in live events, 256 KiB
  inline in `design_get`.

## 7. Troubleshooting

- **409 on save** — the head moved since you loaded it; re-fetch
  `GET …/content` (its `X-Design-Version` header is the new base).
- **A link shows `broken`** — the target artifact, version (`@vN`) or node
  (`#id`) doesn't exist; the save's `links.broken[].reason` says which.
- **Search finds nothing** — check the daemon log for
  `design: FTS5 unavailable` (LIKE fallback searches titles/tags only).
- **Imported artifacts missing** — `POST /design/admin/import` returns a report
  with per-row `errors` (e.g. a missing attachment file); the log line
  `design: legacy import done` has the counts.
- **`design:` 403 for a non-root user** — grant the `design` feature in
  Settings → Users (it was copied from `canvas` once, at upgrade).
