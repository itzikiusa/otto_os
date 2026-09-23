# Design Hall — the artifact graph

> **Status: Phase 0 + the Phase 1 agent pipeline.** The graph, its REST/WS
> surface, the legacy import, the Design Hall UI (§8), the unified
> `design_assist` agent turn (every studio / format), variants, verified
> reference citations, the suggest-only Learning v1 and the design MCP tools
> (reads + approval-gated writes), the Brand Kit v1 editor (§9) and **3D
> Studio 1.5** (§10: scene3d v2, the studio layout, Generate, exports and the
> embed runtime) ship now. Export/publish and Site Studio are the next phases
> (see the proposal's roadmap). The existing Product → Design arena and Canvas
> keep working unchanged meanwhile.

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
  variants, agent drafts, edits after an agent draft, review comments, brand
  corrections, a11y fixes, status changes, shipped). Learning v1 (§5.3) turns
  repeated ones into team-rule proposals a person approves.

Code: `crates/otto-design` (router, service, store, extractor, import,
`variants`, `cite`, `learn`), `crates/otto-server/src/design_hall.rs` (glue),
`crates/otto-server/src/design_assist.rs` (the agent pipeline),
`crates/otto-improve/src/design.rs` (the `design` evidence source), migration
`…_design_graph.sql` (no new migration for the pipeline).
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
`scene3d` `gltf.attachment_id` / v2 `gltf.src` → `embeds`, v2 `brand` →
`uses_tokens`; JSON formats by key — `src`
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
other end that you can view. `GET /design/links?artifact_ids=a,b,c&dir=…`
does the same for up to 100 artifacts in one call (each link once; ids you
can't view are skipped) — what Product's design strip uses.

### Search (the References drawer)

`GET /design/search?q=hero card&studio=site&status=shipped` — FTS5 over
titles, tags, extracted text (HTML copy, Mermaid/D2 source, Excalidraw text,
3D object names and notes, brand token names), linked story keys + titles and
the project name. Terms are AND-ed, the last one prefix-matched; results come
shipped → approved → review → draft, then by relevance, each with a snippet,
`reference_count` and the `story_ids` it implements.

### Listing (the Lobby)

`GET /design/artifacts?story_id=S&limit=100` lists newest-updated first. Every
row carries `story_ids` (so the Lobby can group by story without a search
call), `created_by_name` and the head's `last_editor_id` / `_kind` / `_name`
(versions carry `author_name`) — all resolved from the users table on read. A
full page answers `X-Next-Cursor: <updated_at>|<id>`; send it back as
`cursor=` for the next page (clients can also build it from the last row).

### Signals

`POST /design/signals {artifact_id, kind, version_id?, payload}` records a
bounded signal (≤ 8 KB JSON object). Captured automatically:
`edit_after_draft` (a human save within 2 h of an agent version — the payload
is a structural summary, never the content), `status_change`, `shipped`,
`agent_draft` (every committed assist turn) and `variant_accepted` /
`variant_rejected` (a variant accept). Clients record `restored` (an older
version saved again — `payload.from_version_id`), `reference_added`
(`payload.target_artifact_id`) and `forked` (on the new artifact —
`payload.source_artifact_id`); those three are refused (400) without their
key. `GET /design/signals` reads the log; §5.3 turns repeated signals into
team-rule proposals.

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

### 5.1 One assist pipeline for every studio

`POST /design/artifacts/{id}/assist {prompt, mode?, selection?, references?}`
runs ONE agent turn on any text/JSON artifact — HTML screens, SVG, Mermaid,
D2, Excalidraw boards, `scene3d`, whiteboards (`otto-canvas`: the agent edits
the inner `source.mmd` / `source.d2` / `source.excalidraw.json`), and the
`otto-site` / `-layout` / `-brand` / `-exhibit` JSON documents. Binaries
(images, GLB, PDF) are refused.

```
<data>/design/<id>/work/
  <file>            the working copy (materialized from the head) — the agent edits it in place
  CONTEXT.md        the context brief (story + AC, links, brand kit, [R1..Rn], team rules, design memories)
  refs/R1.json …    reference excerpts (+ R1.png thumbnails when the artifact has one)
  render/current.png  the artifact's current thumbnail, when present
  provenance.json   optional, written by the agent: {"refs":["R1"],"why":"…"}
  findings.json     critique / a11y findings, written by the agent
```

1. The call answers **202** with the turn as soon as the agent session is
   live (its `session_id` lets the UI attach the live terminal); the turn keeps
   running and reports over WS (`design_assist_updated`). Each valid save the
   agent makes is broadcast as `design_artifact_updated {change:"live"}`.
2. At the end the file is validated for its format (and `scene3d` against its
   schema); an invalid result commits nothing and the working copy is restored.
3. A valid result becomes ONE new version: `kind: agent`, author `agent`, the
   session id, the agent's one-line summary as the message, and a
   `provenance` that records the references offered, the ones it cited
   (verified — see 5.2), the brand-kit version, the team rules in force and a
   prompt summary. An `agent_draft` signal is recorded.
4. If someone saved while the agent worked, nothing is clobbered: the draft is
   kept as the side version `variant/<turn>/1` (status `conflict`) and can be
   compared and accepted later.

Modes: `generate` (a fresh design), `refine` (default — change what was
asked, keep the rest), `critique` (never edits; findings in the turn), `a11y`
(fixes contrast / alt text / tap targets / heading order in place, and lists
them). `selection` focuses the change on one node/section. A main turn resumes
the artifact's assist session (`meta.assist`) when the provider matches.

Costs stay bounded: one agent run per artifact at a time (409 otherwise), 20
minutes per turn (the session is stopped past it), at most 4 variants.

Under the opt-in process sandbox (`process_sandbox`, macOS Seatbelt) the
agent may write `<data>/design/<artifact>/work/**` — per artifact, by
pattern — and nothing else of `<data>/design/`: the blob store is denied
again after that grant. Variant turns work in `design/<artifact>/variants/…`,
which the sandbox does NOT open yet, so run variants with the sandbox off.

**Variants.** `POST …/variants {prompt, n ≤ 4, providers?, directions?}` runs
n fresh turns in parallel, one direction each (`defaults` follows the team
rules, `explore` deliberately ignores soft preferences, then `calm`, `story`),
optionally across providers (`["claude","codex"]`). Each lands on
`variant/<run>/<k>` — the head and the working copy never move.
`design_variants_ready` fires when all are done; `GET …/variants` lists runs.
`POST …/variants/{version}/accept` fast-forwards main to the pick (a new main
version with `provenance.accepted_variant`; 409 if main moved meanwhile unless
`force`) and records `variant_accepted` + a `variant_rejected` per sibling.

`mockup_assist` (Product arena) and `canvas_assist` (Canvas) keep serving
their legacy routes unchanged for this release: they write legacy storage and
return legacy shapes, and the import mirrors their results into the graph as
`sync` versions. Graph artifacts — including the imported ones — are assisted
here.

### 5.2 References and citations

Every turn is offered up to 8 numbered references `[R1] <title> (<studio>,
<status>, v12)`: the request's `references` first, then explicit
`references` / `derived_from` links, then a library search in the artifact's
workspace (its title, then the prompt's most salient terms — shipped work
first). The agent names what it borrowed inline (`layout rhythm from [R1]`)
or in `provenance.json`. The server VERIFIES each citation against the offered
set: an unlisted label, an unknown artifact or a different version is dropped
from the provenance (listed as `citations_unverified`). Each verified citation
becomes a pinned `references` link, so "used as reference in N designs" grows.

### 5.3 Learning v1 — suggest-only team rules

A deterministic extractor (`otto_design::learn`) reads the workspace's last 90
days of signals and proposes a rule when a pattern repeats ≥ 3 times across ≥ 2
artifacts: a variant direction the team keeps picking, a reject reason, a
property humans keep changing right after agent drafts, an accessibility fix
they keep accepting. `POST /design/learned/extract {workspace_id}` (and every
variant accept, in the background) hands the ready ones to otto-improve's
`design` evidence source, which files each as a **pending** edit of the
workspace skill `design-team-style` — one `- [rule:<key>] <text>` line, never
auto-applied, never in a bundled skill. A person approves, rejects or rolls
back each through the self-improvement edit flow (`/improvement/edits/{id}/…`);
`GET /design/learned?workspace_id=` shows active rules with their evidence,
the pending ones, the history and the current candidates. Approved rules are
in every design turn's brief and prompt ("Applied your team rules: …"),
together with matching memories from the `otto-memory` `design` collection.
Set the workspace setting `design_learning` to `"off"` to stop proposals.

### 5.4 MCP tools

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

Two write tools are governed and **approval-gated** (DANGEROUS, off by
default; an operator may exempt one tool at a time in MCP → Otto server):

| governed (`otto.*`) | stdio (bridge only) | route |
|---|---|---|
| `otto.design_assist` | `otto_design_assist` | `POST /design/artifacts/{id}/assist` |
| `otto.design_link` | `otto_design_link` | `POST /design/artifacts/{id}/links` |

Both are scoped to the target artifact's own workspace (a workspace-pinned
token can't write elsewhere). No tool approves a version — approval stays
with people.

## 6. Capabilities & limits

- Assist turn state lives in memory (`GET …/assist`, the `turns` of `GET
  …/variants`); after a daemon restart only the committed versions, signals and
  sessions remain. Agents can't see a fresh render yet — only the thumbnail the
  UI last stored (`render/current.png`); `design_render_request` is a later
  phase.
- Learning v1 only proposes; rules apply after a person approves them. A rule
  approved after another pending one was approved re-appears as `conflict` and
  is re-proposed on the next pass.
- The Product Design tab and Canvas keep their own storage and routes. Edits
  made there reach the graph through the import's `sync` versions (next start
  or `POST /design/admin/import`), not live — so Design Hall opens imported
  artifacts read-only, with "Open in Product/Canvas" and "Make an editable
  copy" (a `derived_from` fork).
- `otto-site`, `otto-layout`, `otto-exhibit` are stored and link-indexed as
  generic JSON; their own validators land with their studios. `otto-brand`
  has its own validator and indexer (§9).
- `#node` validation covers JSON formats (any `"id"`; `scene3d` also
  `view:<camera id>` and `state:<state id>`) and HTML/SVG (`id="…"`);
  Mermaid/D2 nodes are accepted as-is.
- Brand-token references (`token:color.primary`, `var(--brand-color-primary)`)
  are read by the Brand Kit impact preview (§9) and resolved in the UI with
  `resolveToken` / `brandCssVars` (`brand/tokens.ts`) — the 3D Studio and the
  3D embed runtime included (§10); the server doesn't rewrite them into
  consumer documents (it validates scene3d token syntax and indexes the names).
- The `edit_after_draft` summary is a bounded heuristic (changed JSON paths /
  line counts); the per-format structural diff arrives with Compare.
- Thumbnails: the UI stores what it rendered with `PUT
  …/thumbnail` (PNG or WebP, ≤ 2 MB; never bumps `updated_at`). Agents only
  get PNG thumbnails (`render/current.png`, `refs/R<n>.png`) — a WebP one is
  not offered to them.
- The saved-state archive carries the design rows and — best-effort, last,
  within the 256 MiB budget — the blobs they reference (versions +
  thumbnails; see `docs/features/state-archive.md`). Skipped blobs are listed
  in the archive's `excluded` notes; working copies (`<id>/work/`) are not
  archived (they are re-materialized from the head).
- Content caps: 25 MB raw per version, 4 MB inline in live events, 256 KiB
  inline in `design_get`.

## 7. Troubleshooting

- **409 "an agent is already working on design artifact …"** — one assist turn
  or variants run per artifact; wait for its `design_assist_updated` /
  `design_variants_ready`.
- **Turn `conflict`** — someone saved while the agent worked; the draft is the
  side version in `version_id` (`GET …/variants` lists it) — accept it with
  `force` or discard it.
- **Turn `failed` with "invalid … document"** — the agent's file didn't pass
  the format check; nothing was committed and the working copy was restored.
- **A citation is missing from the provenance** — it wasn't one of the offered
  `[R…]` references (see the turn's `unverified_citations`).

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

## 8. The UI (`#/design`, `ui/src/modules/design-hall/`)

The sidebar's **Design Hall** entry (Build group) replaces Canvas; `#/canvas`
still routes (it is the Whiteboard studio, one ⌘K "Go to Canvas" away).

| Route | View |
|---|---|
| `#/design` · `#/design/spatial` | Lobby — Grid (prompt hero, the seven studios, Continue, Projects, Linked to Product, the learning + agent-activity rail) or the Spatial (beta) CSS gallery of project bays |
| `#/design/a/<id>` | One artifact: breadcrumb, status ▾ (Approve is explicit and human-only), version strip, Compare (side by side / text Changes; Restore saves a NEW version), right panel Links (Uses / Used in, add/remove explicit links) + References (library search, Add as reference, Start from this, Compare, provenance lineage); a `scene3d` design opens the 3D Studio (§10) |
| `#/design/p/<id>` · `#/design/studio/<studio>` · `#/design/story/<id>` | A project / a studio (with its classic-or-planned note) / the designs implementing a story |
| `#/design/brand[/<id>]` · `#/design/learned[/rules\|/memory]` | The Brand Kit editor (§9; without an id it opens the most recent kit) · What Otto learned: Pending, Rules, Memory, Signals and Settings (approve / reject / roll back are human-only, via the self-improvement edit flow) |

- **Generate.** The lobby prompt creates a draft in the chosen studio (the
  brief is kept in `meta.brief`) and starts an agent turn — or a variants run
  when Variants is 2–4 — opening the design on its Otto tab
  (`#/design/a/<id>/otto`); "Draft only" skips generation. "Use references"
  sends the team's closest past designs first as R1–R3. Frames/Graphics
  edit HTML (and SVG) as source with a live preview, 3D opens 3D Studio 1.5
  (§10), Whiteboard edits Mermaid/D2/Excalidraw; Site Studio and Spatial Hall
  show their planned release instead of an editor.
- **Saving** is explicit (Save / ⌘S): `PUT …/content` with `base_version`; a
  409 asks "Save mine on top" or "Load vN (discard my edits)".
- **Otto tab** (every studio): agent + model picker, live status from
  `design_assist_updated`, an attributed thread with verified citation chips
  (unverified ones flagged), quick actions (more engaging, accessibility,
  on-brand, mobile, real copy from story, 3 variants), and a variants tray
  (Apply / Compare / reject with a reason). A finished turn is a new version;
  Restore undoes it.
- **Signals the UI posts:** `restored`, `forked`, `reference_added`,
  `variant_rejected` (with a reason, or when a person keeps their edits over an
  agent draft), `a11y_fix` and `critique_finding`. Approve / status / shipped /
  agent drafts / accepts / edit-after-draft are recorded by the daemon, not
  posted twice.
- **Live:** the `design_*` events refresh the lobby, the open artifact (a new
  head while you have unsaved edits shows a "newer version" notice instead of
  replacing your copy), its Links panel and the learning log.
- **Product:** the story's Design tab shows a small graph of the designs that
  implement it (an epic: its children's too) above the unchanged arena, with
  "Open in Design Hall".
- **Approve** says what moves: "Rewards+ landing page follows Approved → now
  shows v8 · Launch social kit pins an older version (update available)".

## 9. Brand Kit (`#/design/brand/<id>`, `otto-brand/1`)

A brand kit is one versioned artifact (studio `brand`, format `otto-brand`)
that every studio reads by token name. Code: `crates/otto-design/src/brand/`
(validator, indexer, exporters, contrast, impact) and
`ui/src/modules/design-hall/brand/` (editor + the shared `tokens.ts`).
Contract: [`api.md` § Brand Kit](../contracts/api.md).

### The document

```json
{ "$schema": "otto-brand/1", "name": "Acme Brand Kit",
  "color":  { "primary": { "$value": "#5B3DF5", "$description": "Buttons and links" } },
  "font":   { "display": { "$value": "-apple-system, system-ui, sans-serif", "weights": [700, 800] } },
  "type":   { "display": { "size": 64, "line": 72, "weight": 800 } },
  "radius": { "md": { "$value": 14 } },
  "space":  { "md": { "$value": 16 } },
  "logos":  [ { "name": "Full logo", "kind": "full", "asset": "otto://design/<image id>" } ],
  "imagery": { "summary": "…", "do": ["…"], "dont": ["…"] },
  "voice":   { "summary": "Warm, confident, never salesy.", "do": ["…"], "dont": ["…"] } }
```

Every save (a person, the editor, an agent turn) is validated — hex colours,
token names `[A-Za-z0-9][A-Za-z0-9_-]{0,63}` (≤ 64 per group), font roles
`display` / `body` / `mono` with CSS-safe stacks, positive type sizes, px
radius/space, ≤ 16 logos whose asset is an `otto://design/…` image or
`blob:<sha256>` (empty = a placeholder slot), ≤ 20 do/don't lines — and a bad
document is a 400 naming the paths (`color.primary.$value: …`). Fonts are
local family stacks only: nothing is downloaded from a web-font CDN.

### How studios use a kit

- **Link it**: a document's `brand` / `brand_kit` / `tokens` / `theme` key set
  to `otto://design/<kit>` (or an explicit **Uses tokens** link) makes a
  `uses_tokens` edge. Consumers follow the kit's **approved** version by
  default; `@latest` follows every save, `@vN` pins.
- **Name tokens**: `token:color.primary`, `token:type.display.size`, or the
  CSS property `var(--brand-color-primary)`. The CSS names are
  `--brand-<group>-<name>` (kebab-cased: `surfaceAlt` → `surface-alt`); type
  styles expand to `--brand-type-<name>-size|-line|-weight`.
- **Resolve** in the UI with `brandCssVars(doc)` (all properties, e.g. set on a
  preview root) and `resolveToken(doc, "token:color.primary")` → `#5B3DF5`
  (a type style without a sub-property resolves to a `font` shorthand).

### The editor

- **Left column:** Colors (colour input + hex, the CSS variable, live WCAG
  badges vs white and the kit's ink, "Try a primary" presets), Typography
  (font roles + the type scale as live specimens), Spacing & radius, Logos
  (uploading a file imports it as a Graphics design and links it), Imagery and
  Voice (summary + do/don't), and **Used in** (every consumer, its studio,
  status, policy and the tokens it names).
- **Right column — Applied to:** a landing hero, a social tile and a 3D card
  swatch re-tint as you type (CSS custom properties; nothing is saved).
- **Rule from team** chips: approved learned rules (`GET /design/learned`)
  that mention brand words, a token name or a colour's hue ("amber is never
  text" for a `#FFB547` accent) link to What Otto learned.
- **Impact before save.** While token changes are unsaved, a banner says how
  many designs in how many studios they reach (`POST …/brand/impact`,
  debounced). **Save as vN** shows the impact preview first whenever designs
  use the kit (the token diff with before → after, the affected designs and
  when each moves: *follows approved* → on approve, *follows latest* → on
  save, *pinned* → never until updated, plus contrast warnings), then commits
  a named version with `base_version` (409 → "Save mine on top" / "Load vN").
  A `brand_correction` signal records which tokens changed.
- **Approve vN** is a separate, confirmed step; it moves the approved version
  and the daemon's `design_link_updated {reason: "target_approved"}` tells
  every following consumer.
- **Export ▾** copies or downloads the saved kit as CSS variables, a Tailwind
  v4 `@theme` block or W3C DTCG JSON (`GET …/brand/export`) — local only.
- **New brand kit** starts from one of three starter kits (Vivid, Editorial,
  Minimal), each contrast-checked.

### Troubleshooting

- **"invalid otto-brand document: …" on save** — the message names the path;
  the editor shows the first problem in a red banner and keeps Save disabled.
- **A design is missing from Used in** — it has no `uses_tokens` link to the
  kit (set its `brand` field or add a Uses tokens link), it is archived, or it
  lives in a workspace you can't view (counted as "more in workspaces you
  can't see").
- **Used in says "the whole kit"** — the design links the kit but names no
  single token (a theme), or its head isn't readable text; every token change
  counts as reaching it.
- **Saved but designs didn't change** — they follow the approved kit; approve
  the new version.

## 10. 3D Studio 1.5 (`ui/src/modules/design-hall/studio3d/`)

A `scene3d` design opens the studio layout (mockup S4). It edits the same
working copy as every other design — Save / ⌘S, `base_version` conflicts,
versions, Compare and Restore are the artifact view's — so everything below
is either an edit of the scene document, an agent turn, or a local export.

```
┌ HIERARCHY ─────┬ ✨ Generate ▾ · Turntable · Play · Export ▾ · Source ─┬ Inspector · Otto · Links · Refs ┐
│ tree (+ add)   │ VIEWPORT (three.js)                    view cube ⌝  │ transform · corner radius      │
│ Environment    │                                                     │ material preset grid           │
│ Views          │                                                     │ brand swatches · sliders       │
│ Scene card     │ ⌞ States [Idle | Hover | Flipped] +   stats pill ⌟  │ notes · Used in                │
└────────────────┴─────────────────────────────────────────────────────┴────────────────────────────────┘
```

### 10.1 scene3d v2 (v1 still read)

Every v2 field is optional; a v1 document renders unchanged and is written
back as `version: 2` on its next save. The Rust validator
(`design_scene3d.rs`) accepts both versions, validates v2 fields whenever
present, and refuses a token colour in a v1 document.

| Field | What it does |
|---|---|
| `material.preset` | `glossy-plastic` · `brushed-metal` · `frosted-glass` · `matte-paper` · `satin` — defaults for the physical fields; explicit fields win (`scene3d/presets.ts`) |
| `material.{clearcoat, clearcoat_roughness, transmission, ior, thickness, sheen, emissive_intensity}` | `MeshPhysicalMaterial` fields (0..1; ior 1..2.333; thickness 0..10 m; emissive 0..100) |
| `material.color` / `emissive`, state `color` | `#rrggbb` or `token:color.<name>` — a brand-kit colour |
| `brand` | `otto://design/<kit>@approved` — the kit tokens resolve against (a `uses_tokens` link). Unset: the project's `brand_kit_id`, else its first `otto-brand` kit; picking a swatch pins the kit |
| `environment` | `{preset: studio-soft \| sunset \| night \| none, intensity?, background?, rotation?}` — procedural IBL (a gradient sky + softbox panels prefiltered with PMREM) and a gradient backdrop. Nothing is fetched and no HDRI ships |
| `cameras[]` | Named views (`{id, name, position, target, fov}`) — "Hero angle". Embeds ask for one as `otto://design/<id>@approved#view:<id>` |
| `states[]`, `default_state` | Named states (`{id, name, duration_ms ≤ 10 000, easing: linear \| ease-in \| ease-out \| ease-in-out \| spring, overrides: {objectId: {position, rotation, scale, visible, opacity, color, emissive}}}`); the viewer tweens between them. `#state:<id>` addresses one |
| `turntable` | `{enabled?, speed?}` (deg/s) — the studio's Turntable button and embeds' auto-rotate |
| object `radius` | Rounded corners for a `box` (0..0.5 of the unit box) |
| gltf `src` | `otto://design/<glb artifact>[@approved\|@latest\|@vN]` instead of `attachment_id` (exactly one of the two) — the model is fetched through the authed Design Hall client, never a URL |

The Blender script export keeps its v1 output: token colours fall back to the
neutral default and model references never reach the script.

### 10.2 Editing

- **Hierarchy** (add primitives, lights, groups, Import → a project GLB or an
  upload that becomes a GLB design), **Environment** (preset + backdrop),
  **Views** (save the current view, fly to, update, rename, copy the embed
  reference, delete) and a scene card naming the brand kit.
- **Viewport**: W/E/R gizmo, F frame, view cube (click a face to snap), the
  **States** bar (pick → tween; ⋯ rename / duration / easing / start state /
  duplicate / reset selection / delete) and a stats pill ("4 objects · 2 lights
  · draft v8"). While a non-default state shows, gizmo and inspector transform
  edits go to **that state's overrides** (the inspector says so).
- **Inspector**: transform, corner radius, the material preset grid, the kit's
  colour swatches (a click stores `token:color.<name>`; × detaches to the hex),
  roughness / metalness / clearcoat / glass / sheen / opacity, notes, and a
  **Used in** card (what embeds this scene and at which version).
- **Source** swaps the viewport for the scene JSON beside a plain preview.
- **Play** hides the helpers and looks through the document camera; hovering
  the viewport shows the `hover` state when the scene has one.
- After every save the studio renders a thumbnail and stores it with `PUT
  …/thumbnail` (a cache, not a version), so 3D cards stop showing a placeholder.

### 10.3 ✨ Generate — agents ask first, providers are opt-in

Every item runs ONE design-assist turn (§5.1) and lands as a reviewable
`agent` version (the toolbar shows "Otto · …" while it runs, then a toast; a
draft that isn't approved shows "Showing vN draft by Otto · not approved").
Unsaved edits must be saved first.

| Item | What happens |
|---|---|
| Blockout from prompt (scene JSON) | `mode: generate` with a scene3d v2 brief (studio environment, key + rim, presets, a `hero` view, Idle/Hover) |
| Text → 3D model | A provider picker. **Local: Otto + Blender MCP** runs a `refine` turn that adds the model as a new group of primitives (Draft / Standard quality); the agent may use the Blender MCP tools, and each of those calls asks for approval in the MCP control plane. **Cloud: Tripo / Meshy** are listed with their cost and "sends your prompt to …", and stay **disabled** — this build has no Keychain key store for them, so they never run |
| Image → 3D model | Same picker; the image is saved as a reference design in the project first and handed to the agent as `[R1]` (nothing leaves the Mac) |
| Refine in Blender (MCP) | A confirmation that states whether Blender is installed (`GET /product/design/blender`), that Blender MCP calls ask first and that Blender runs model-generated code without guards; then a `refine` turn |

The provider interface is `studio3d/providers.ts` (availability rules are
unit-tested). A cloud provider needs, before it can be enabled: a Keychain
key, its host on the netguard allow-list, and a daemon route — none exist yet.

### 10.4 Export and Optimize for web

Export ▾: **GLB** (three `GLTFExporter` from the live scene), **USDZ** (three
`USDZExporter`, for iOS AR Quick Look), **PNG snapshot**, **PNG turntable** (8
frames around the target in one sprite sheet), **Scene JSON**. The Blender
script stays a Product-arena export for now.

**Optimize for web** exports the GLB and runs gltf-transform + meshoptimizer
in the webview (a lazy chunk, loaded on first use; no network): dedup → prune
→ weld → meshopt (reorder + quantize + `EXT_meshopt_compression`), falling
back to plain quantization if the encoder can't start. The dialog shows before
/ after / saved and a budget bar against 2 MB, with **Download** and **Save as
GLB design** (a `derived_from` fork you can embed). Draco is not offered: its
encoder isn't bundled and the app stays offline. Every Otto viewer decodes
meshopt.

### 10.5 The embed runtime (`studio3d/embed.ts`)

For the Site Studio's 3D block (and any card or preview):

```ts
const h = await mountScene3dEmbed(el, {
  src: 'otto://design/<id>@approved#view:hero', // a scene3d or GLB design
  autoRotate: true, drag: true, tiltOnHover: true, poster: thumbUrl,
});
h.setState('flipped'); h.setView('hero'); h.info(); h.destroy();
```

The URI selector picks the version (`version` overrides it), `view` /
`#view:` a named camera, `#state:` a start state. Hovering tweens to the
scene's `hover` state (or `hoverState`) and `tiltOnHover` leans toward the
pointer. The poster shows until the first frame and stays on failure;
`prefers-reduced-motion` turns auto-rotate and tilt off and makes state
changes instant; off-screen embeds stop rendering. `embedSrc(id, {selector,
view})` builds the reference a block stores. Exported static sites inject
their own loaders via `resolve` (the default uses the authed client).

### 10.6 Limits

- Text/image → 3D produce primitive models through the agent; mesh
  generation from cloud providers is not wired yet (disabled, see §10.3).
- States override objects, not lights or the camera; groups carry no
  transform, so a state moves each child it overrides.
- The PNG turntable is one sprite sheet (no GIF/video); USDZ carries meshes
  and materials only (no lights or states).
- `token:` colours resolve in 3D (studio + embeds); the Blender script and GLB
  export use the resolved hex (GLB) or a neutral fallback (Blender).
