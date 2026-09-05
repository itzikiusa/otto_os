# Product → Design Arena + Epic tree

Status: **design, reviewed** (2026-09-05; rev 3 after two code-grounded review passes). Extends the Product module in two directions:

1. **Epic tree.** A story can have a parent (an *epic*) and a *folder* inside that
   epic. Swarm agents publish INTO the epic their project belongs to instead of
   creating a fresh top-level story per `otto-product` call.
2. **Design Arena.** The **Mockups** tab becomes **Design**: one arena with a
   Figma/Canva-style 2D side (screens, boards, diagrams) and a game-studio-style
   3D side (viewport + hierarchy + inspector), all agent-assisted and file-backed,
   with an optional Blender bridge (headless render/export + the Blender MCP).

Everything below builds on what already exists: `product_attachments` for the
artifacts, `mockup_assist.rs` for the in-place file-backed agent turn, the
`otto-*` helpers materialized into swarm agent cwds, and the skill seeding in
`crates/otto-product/src/skills.rs`.

---

## 1. Problem statements

### 1.1 "The swarm creates a lot of product pages"

`POST /ingest/swarm/product` (`routes/swarm_ingest.rs::product_ingest`) calls
`create_draft` on **every** invocation of the `otto-product` helper. In a swarm
project with a PO, an analyst and a designer, each agent (and each retry) yields
another top-level draft. Nothing ties the draft to the swarm project's story even
when `swarm_projects.story_id` is set (Plan → Swarm link).

### 1.2 Mockups are flat and 2D-only

The Mockups tab is a flat list of `kind:mockup` attachments (HTML / Mermaid /
image). There is no board (freeform), no artboards, no 3D, no per-object editing,
no export beyond "open the file". Canvas has Excalidraw but lives in a separate
module and links to a story only loosely (`LinkedCanvases`).

---

## 2. Data model

### 2.1 `product_stories` — epic tree (migration `0115_product_epic_tree.sql`)

```sql
ALTER TABLE product_stories ADD COLUMN parent_id TEXT;                          -- NULL = top level
ALTER TABLE product_stories ADD COLUMN tree_kind TEXT NOT NULL DEFAULT 'story'; -- 'story' | 'epic' | 'doc'
ALTER TABLE product_stories ADD COLUMN folder    TEXT NOT NULL DEFAULT '';      -- '' | 'Design' | 'PO' …
CREATE INDEX idx_product_stories_parent ON product_stories(parent_id);
```

- `tree_kind` is Otto's **tree role**, deliberately distinct from `source_kind`
  (jira/confluence/draft), Jira's `issue_type` (which may say `Epic`), and
  `product_story_versions.kind`. The UI treats a row as an epic when
  `tree_kind == 'epic'` **or** it has children — so a Jira story linked to a swarm
  project shows as the root of its tree **without Otto ever rewriting the row**.
- **One level of nesting.** A child never has children (server rejects
  `parent_id` pointing at a row that itself has a parent). Folders give the
  visual sub-hierarchy without recursive queries.
- `tree_kind:'doc'` is a lightweight child (a design note, a spec section) that
  hides analysis/plan tabs in the UI; `story` children get the full tab strip.
- Deleting a parent re-parents its children to top level (`delete_story` gains
  an `UPDATE … SET parent_id = NULL WHERE parent_id = ?`; never cascades).
  `delete_story` also (pre-existing gap) starts deleting the story's
  `product_attachments` + `product_mockup_annotations` rows, and the server-side
  delete route removes `data_dir/product/attachments/<sid>/` best-effort.
- `ProductStory` (Rust `otto-state/product.rs` + `ui/src/modules/product/types.ts`)
  gains `parent_id: Option<Id>`, `tree_kind: String`, `folder: String`.
  `NewStory` carries them too (so ingest creates children in one insert);
  `StoryPatch`/`UpdateStoryReq` accept them (`parent_id: Option<Option<Id>>` to
  allow clearing). The one exhaustive literal to fix is the `make_story` test
  helper in `otto-server/src/product_run.rs`.

### 2.2 Attachments — design artifacts (no new table)

`product_attachments.kind` gains `'design'`. `meta_json` carries
`{ "format": …, "assist_session_id": …, "group": "Screens" }` (`group` is the
arena's asset grouping — unrelated to the story `folder` of §2.1). Formats:

| format | file | mime | renders with |
|---|---|---|---|
| `html` | `design.html` | `text/html` | sandboxed iframe (existing `MockupViewer`) |
| `mermaid` | `design.mmd` | `text/vnd.mermaid` | existing Mermaid viewer |
| `excalidraw` | `design.excalidraw` | `application/vnd.excalidraw+json` | Excalidraw board (React island, already a dep) |
| `scene3d` | `scene.json` | `application/vnd.otto.scene3d+json` | three.js viewport (new, lazy-loaded) |
| `glb` | uploaded | `model/gltf-binary` | three.js viewer (as a `gltf` object or standalone) |
| `gltf` | uploaded | `model/gltf+json` | same (JSON sniff) |
| images | uploaded | `image/*` (existing) | `<img>` |

**No Python attachments.** A Blender script is an *export* generated server-side
from a validated `scene3d` document (§4.4), never an uploaded or agent-written
file, so `text/x-python` is NOT added to the mime allow-list.

Existing `kind:'mockup'` rows keep working; the arena lists `mockup`, `design`
and `image` kinds together. `product_media::allowed_mime` grows by
`application/vnd.excalidraw+json`, `application/vnd.otto.scene3d+json`,
`model/gltf-binary` and `model/gltf+json`; `sniff_ok` gains the `glTF` magic
header and a JSON case (`{` after whitespace). `ext_for_mime` maps them to
`.excalidraw` / `.json` / `.glb` / `.gltf` (today anything unlisted becomes
`.bin`), and `default_kind_for_mime` returns `'design'` for all four (today
`'file'`, which the arena would never list).

**One format enum.** `mockup_assist::normalize_format`, `swarm_ingest::mockup_ext
/ mockup_mime`, `file_name / ext_for / mime_for / title_for / base_stub`, the
fence language in `resolve_source`, `resolve_target`'s kind/mime acceptance, and
the UI `MockupFormat` union + `MockupsTab.isMockup` all move to a single
`DesignFormat { Html, Mermaid, Excalidraw, Scene3d }` (Rust) mirrored in TS.
Unknown formats are a **400**, never a silent fallback to HTML (the two tests
that currently assert the fallback are updated to assert the rejection).

**New endpoint** — save an edited artifact from the UI (the 3D inspector, the
Excalidraw board, the code view):

```
PUT /api/v1/product/attachments/{aid}/content   (ws editor)
  { "data_b64": "<base64>" }  → ProductAttachment
```
- Reuses the upload path's guards: `allowed_mime`, `sniff_ok`, `MAX_RAW_BYTES`,
  `confine_join` for the storage path, and the explicit `DefaultBodyLimit::max(40 MB)`
  layer (axum's default is 2 MB and there is no global limit).
- The row's `mime`/`filename` never change through this route (extension and
  served content-type stay consistent); persistence goes through the existing
  `AttachmentRepo::set_assist_result` which already updates `size_bytes` +
  `updated_at`.
- Broadcasts `Event::MockupUpdated` with `content: Option<String>` — text formats
  carry the source (unchanged behaviour), `glb`/large payloads carry `None`,
  serialized as an explicit `null` (no `skip_serializing_if`; TS side is
  `string | null`) and clients re-fetch. This is the one WS contract change
  (`ws.md`, `ui/src/lib/api/types.ts`, `ui/src/lib/events.svelte.ts`).

### 2.3 `scene3d` document (the agent-editable 3D format)

Small, declarative, human-readable; the browser renders it, the agent edits it,
the inspector round-trips it. Version 1:

```jsonc
{
  "type": "otto-scene3d", "version": 1,
  "background": "#0f172a", "grid": true,
  "camera": { "position": [6, 5, 8], "target": [0, 1, 0], "fov": 50 },
  "lights": [
    { "id": "sun", "type": "directional", "position": [5, 10, 5], "intensity": 1.2, "color": "#ffffff", "shadow": true },
    { "id": "amb", "type": "ambient", "intensity": 0.4 }
  ],
  "objects": [
    { "id": "floor", "name": "Floor", "type": "plane", "position": [0, 0, 0], "rotation": [-90, 0, 0],
      "scale": [20, 20, 1], "material": { "color": "#334155", "roughness": 0.9 } },
    { "id": "crate", "name": "Crate", "type": "box", "position": [0, 0.5, 0], "rotation": [0, 30, 0],
      "scale": [1, 1, 1], "material": { "color": "#f59e0b", "metalness": 0.1, "roughness": 0.6 } },
    { "id": "hero", "name": "Hero", "type": "gltf", "attachment_id": "<aid of a .glb attachment>",
      "position": [2, 0, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1] }
  ],
  "groups": [ { "id": "props", "name": "Props", "children": ["crate", "hero"] } ]
}
```

- `type ∈ box | sphere | cylinder | cone | torus | plane | text | gltf | group`.
- rotation is **degrees** (agents and humans think in degrees); the viewer converts.
- `material`: `color`, `metalness`, `roughness`, `opacity`, `emissive`, `wireframe`.
- `gltf` objects reference **`attachment_id`, never a URL**. The viewer resolves
  it through the existing authed-blob helper (`MockupViewer.svelte::authedBlobUrl`
  → `GET /product/attachments/{aid}`), so there is no URL surface to guard.
- The document is validated before render and before save (`scene3d.ts::validate`
  in TS, `design_scene3d.rs::validate` in Rust): known `type`s only, finite
  numbers, bounded array lengths (≤ 2 000 objects), `attachment_id` must be a
  safe id component.
- `scene3d → blender.py` is a deterministic **server-side** export
  (`design_scene3d.rs::to_blender_script`, a fixed template interpolating only
  validated numbers/enums/escaped names); the UI offers it as a download.

---

## 3. Epic tree behaviour

### 3.1 Swarm ingest — one epic per project, children in folders

`product_ingest` becomes:

1. Resolve `project_id` from the session meta (already present).
2. **Resolve the epic:**
   - `project.story_id` set → that story is the target root, **untouched** (the
     UI shows it as an epic because it has children).
   - else → create ONE `tree_kind:'epic'` draft named after the project goal and
     `update_project(story_id)`. `SwarmRepo::update_project` must map the unique
     violation to `Error::Conflict` the way `create_project` already does; on
     conflict the loser re-reads the project and files under the winner's epic.
   - no `project_id` in the session meta (a swarm without a project) → today's
     behaviour (top-level draft), unchanged.
3. **Resolve the child:** `otto-product --title T --folder F --kind doc|story`
   (defaults: folder = the agent's role title, kind = `doc`). The ingest body is
   `ProductIngestReq { title?, body_md, tree_kind?, folder? }` — the shell flag
   `--kind` maps to the JSON key `tree_kind`, the same name used by
   `POST …/children` and `UpdateStoryReq`. If a child with the
   same normalized title already exists under the epic → **update its body**
   (new `suggested` version) instead of creating another.
4. Broadcast `ProductChanged { section:"tree", status:"changed" }` (existing event
   shape, new `section` value) so the list pane refreshes.

Shell helpers (`swarm_workspace.rs`): `--folder` / `--kind` (and `--format` for
`otto-mockup`) are added **inside the `case` block** — the current `*)` catch-all
would append unknown flags to the markdown body. Both helpers stop swallowing
failures: `curl -f`, print the HTTP status on error, exit non-zero, so an agent
sees why nothing landed.

The swarm role prompt (`swarm_run.rs`) explains: *"the Product page already has an
epic for this project; `otto-product` files your draft under it — pass `--folder`
to group your work (e.g. `--folder Design`)."*

`otto-mockup` today only works inside a Discovery run (`ingest_mockup` requires a
`product_discovery_runs` row and silently 204s otherwise). It now resolves the
target as: discovery run's story **or** `project.story_id` **or** the epic
resolved by step 2, and accepts `--format html|mermaid|excalidraw|scene3d`.

### 3.2 UI — the tree list pane

`ProductPage.svelte` left pane renders a tree:

```
▾ 🗂 Loyalty programme (epic · 6)        ← click = epic overview; ▸ collapses
    Design/                              ← folder header (collapsible)
      ◻ Tier ladder screens      DRAFT
      ◻ Rewards 3D kiosk         DRAFT
    PO/
      ◻ Feature draft v2
  ○ GS-1204 Bonus wallet limits          ← top-level story, unchanged look
```

- Story header shows a breadcrumb `Epic › Folder › Title`.
- Context menu on a story: **Move to epic…** (picker), **Set folder…**,
  **Mark as epic** (sets `tree_kind`), **Detach from epic**. Drag-and-drop is a follow-up.
- Toolbar: **New ▾** → *Draft*, *Epic*. Inside an epic: **Add child ▾** → *Story*,
  *Doc*.
- Epic **Overview** tab adds a "Children" board (folder → cards) and rolls up
  counts; the epic's Design tab shows every child's design artifacts (filterable
  by child) so the epic is the single place to review the whole feature.
- Tag filter continues to work (applies to the flattened list).

Store: `product.stories` stays flat; a `$derived` `tree` groups by
`parent_id` → `folder`. `moveStory(id, parent_id, folder)` = `PATCH`.

---

## 4. Design Arena (the `design` tab)

Layout, desktop (mobile collapses to a segmented top bar + single pane, same as
the Database Explorer):

```
┌──────────────┬─────────────────────────────────────────────┬────────────────┐
│ ASSETS       │  ⟵ toolbar: New ▾ · Import · Export ▾ · ⋯  │ INSPECTOR      │
│ Screens (3)  │                                             │ (per format)   │
│  ◻ Home      │                                             │                │
│  ◻ Checkout  │            VIEWPORT / BOARD                 │  Transform     │
│ Boards (1)   │   html: sandboxed iframe (device frames)    │  Material      │
│  ◻ Flow map  │   excalidraw: React island, full editor     │  …             │
│ Diagrams (2) │   mermaid: existing renderer                │────────────────│
│ 3D (1)       │   scene3d: three.js + orbit + gizmo         │ ASSISTANT      │
│  ◈ Kiosk     │   blender: code view + render preview       │  (live agent   │
│ Images (4)   │                                             │   shell +      │
│──────────────│  status: 12 objects · saved 2s ago           │   composer)    │
│ + New        │                                             │                │
└──────────────┴─────────────────────────────────────────────┴────────────────┘
```

### 4.1 Common (all formats)
- **New ▾**: Screen (HTML) · Board (Excalidraw) · Diagram (Mermaid) · 3D scene ·
  Blender script · **From template** (Canva-like starters: mobile app screen,
  dashboard, landing page, user flow, game level blockout, product shot).
  Templates are static files under `ui/src/modules/product/design/templates/`.
- **Import**: files (existing upload) incl. `.glb/.gltf/.png/.svg/.excalidraw`.
- **Export ▾**: PNG (viewport/board snapshot), SVG (mermaid/excalidraw), source
  file, GLB (scene3d via three `GLTFExporter`), **Blender script** (scene3d).
- **Annotations**: the existing pin-comment layer (`MockupAnnotations`) stays
  available on html/image/mermaid/excalidraw. 3D annotations (camera-anchored
  pins) need a `meta_json` column on `product_mockup_annotations` and are
  **deferred** to a follow-up; the 3D inspector offers a per-object `notes`
  field in the document instead.
- **Assistant**: `MockupAssistPanel` generalized (format-aware placeholder,
  provider/model pick, live preview for every format, resumable per artifact).
- **Autosave** with a 600 ms debounce through `PUT …/content`; the status line
  shows "saved / saving / conflict". Live agent edits arrive via
  `MockupUpdated`; if the user has unsaved local edits the panel asks before
  replacing (no silent clobber).

### 4.2 2D — Figma/Canva side
- **Excalidraw board** is the freeform tool (frames, shapes, text, arrows,
  libraries, multi-select, align, groups — Excalidraw already provides these).
  Wrapped in `DesignBoard.svelte` (React island via `react-dom/client`, props
  `source`, `onchange`, `readonly`) — separate from Canvas'
  `ExcalidrawCanvas.svelte`, which is bound to the canvas store.
- **HTML screens** get a **device frame** chooser (iPhone / iPad / desktop /
  none) and light/dark toggle around the sandboxed iframe.
- The agent skill `otto-design-2d` teaches Excalidraw JSON authoring (frames as
  artboards, consistent 8-pt grid, palette, components) and HTML screens.

### 4.3 3D — game-studio side (`Scene3DViewport.svelte`, lazy `three`)
- Orbit/pan/zoom, grid + axes, shadows, HDRI-less PBR lighting from the doc.
- **Hierarchy** panel (left, replaces asset list while a scene is open — toggle):
  objects/groups, click to select, eye to hide, rename inline, duplicate, delete.
- **Gizmo** (`TransformControls`): `W` translate · `E` rotate · `R` scale ·
  `F` frame selected · `Del` delete · `⌘D` duplicate · `Esc` deselect.
- **Inspector** (right): transform (numeric, live), material (color, metalness,
  roughness, opacity, emissive, wireframe), light props, camera props, scene
  background/grid. Every change patches the JSON doc → autosave.
- **Add** menu: primitives, light, group, **import GLB** (upload → `gltf` object).
- **Play** toggle: hides gizmo/grid, uses the doc camera — a "presentation" view.
- Skill `otto-design-3d` teaches the `scene3d` schema, blockout conventions
  (units = metres, y-up, origin at floor), lighting recipes, and when to emit a
  `blender` script instead.

### 4.4 Blender bridge (optional, detected)
- `GET /api/v1/product/design/blender` → `{ installed: bool, path: string|null, version: string|null }`.
  Detection: `$OTTO_BLENDER`, then `PATH`, then
  `/Applications/Blender.app/Contents/MacOS/Blender`, using the existing
  `otto_k8s::install::which / locate_in` style lookup (no `which` subprocess).
- `POST /api/v1/product/stories/{sid}/design/{aid}/blender-render` (ws editor)
  → `202 { "id": "<job id>" }`. `GET /api/v1/product/design/jobs/{id}` (ws viewer)
  → `{ id, attachment_id, status: "queued"|"running"|"done"|"error", error:
  string|null, outputs: string[] /* new attachment ids */, started_at, finished_at:
  string|null }` — an in-memory map on `ServerCtx` (jobs are not persisted).
  Completion also broadcasts `MockupUpdated` for each output attachment. The server
  **generates** the Python from the validated `scene3d` document
  (`design_scene3d::to_blender_script`) — never a user or agent file — writes it
  to a fresh temp out-dir, and spawns
  `blender -b --python <generated.py> -- --out <dir>` with `kill_on_drop(true)`
  and a 120 s `tokio::time::timeout` (copy the shape of
  `otto-aws/src/cli.rs::run_raw`). The script renders `render.png` (Eevee, 1280×720)
  and exports `scene.glb`; each produced file is attached (`kind:'design'`,
  `meta.derived_from = aid`). Confinement: add `SandboxPolicy::for_tool(out_dir)`
  in `otto-sandbox` (writable roots = out-dir + Blender's own cache dirs, no
  network) and wrap the command with it, as agent sessions are wrapped today.
- **Download Blender script**: `GET …/design/{aid}/blender-script` (ws viewer)
  returns the generated `.py` for the user to open in Blender by hand.
- **MCP**: no DB seed at boot (`mcp_servers` rows need a workspace + creator).
  Instead the MCP page's *Add server* form gets a static **templates** list
  (`ui/src/modules/mcp/templates.ts`) with a **Blender** entry — `stdio`,
  `uvx blender-mcp`, description linking to the addon setup, `default_tool_access:
  "ask"`, disabled until the user saves it. The `otto-design-3d` skill tells the
  agent to prefer the Blender MCP tools when they are available and fall back to
  the `scene3d` file otherwise.

### 4.5 What we deliberately do NOT build
- No proprietary Figma import, no realtime multi-user cursors, no in-browser mesh
  sculpting/UV editing — that is Blender's job, via the bridge.
- No recursive epics (one level + folders covers Jira's epic → story shape).

---

## 5. API / contract changes (docs/contracts + types.ts in lockstep)

| Change | Where |
|---|---|
| `ProductStory { parent_id, tree_kind, folder }`; `NewStory`, `StoryPatch`, `UpdateStoryReq`; `delete_story` re-parents | `otto-state/product.rs`, `otto-product/types.rs`, `ui/src/modules/product/types.ts`, `docs/contracts/api.md` (§ Product rows) |
| `POST /product/stories/{sid}/children` `{ title, tree_kind, folder }` → `ProductStoryDetail` | `otto-product/http.rs` + `service.rs` |
| `PUT /product/attachments/{aid}/content` | `otto-server/product_media.rs`, `modules.rs` (with the 40 MB body layer) |
| `POST /product/stories/{sid}/mockups/assist` `format ∈ html\|mermaid\|excalidraw\|scene3d`, 400 otherwise | `mockup_assist.rs` (+ `DesignFormat` in a new `design_format.rs`) |
| `GET /product/design/blender`, `POST …/design/{aid}/blender-render`, `GET …/design/{aid}/blender-script` | new `otto-server/src/design_blender.rs`, `design_scene3d.rs`; `otto-sandbox::SandboxPolicy::for_tool` |
| `otto-product --folder --kind`, `otto-mockup --format --folder`, non-silent failures; ingest resolves the epic | `swarm_workspace.rs`, `routes/swarm_ingest.rs`, `swarm_run.rs` prompt; `otto-state/swarm.rs::update_project` conflict mapping |
| `Event::MockupUpdated.content: Option<String>`; `ProductChanged section:"tree"` | `otto-core/event.rs`, `docs/contracts/ws.md`, `ui/src/lib/stores/mockup-assist.svelte.ts` |
| Skills `otto-design-2d`, `otto-design-3d`; `otto-mockup` updated | `crates/otto-skills/assets/skills/development/` (NOT `otto-product`'s versioned array, whose bump overwrites every seeded product skill). Because `otto-skills` bundles are never auto-installed, `resolve_skill_inline` in `otto-server/src/modules.rs` gains an `otto_skills::bundled_body(name)` arm after the product-skill arm so the assist prompts can inline them |
| MCP **template** (UI-side) | `ui/src/modules/mcp/templates.ts`, `ServerForm.svelte` |
| e2e | update `ui/e2e/product-mockups.spec.ts` (tab label → **Design**), add `ui/e2e/product-design-arena.spec.ts` |

RBAC: all new routes ride the existing `/product/` → `Feature::Product` gate
(policy.rs already prefixes-matches). Blender render is Editor-only.

## 6. Risks & mitigations
- **Bundle size**: `three` (~650 kB) and the Excalidraw island load lazily on
  first use of that format, like D2's WASM.
- **Untrusted content**: HTML stays in the sandboxed iframe; `scene3d` JSON is
  validated on both sides before rendering/saving; GLB loads only by
  `attachment_id` through the authed fetch; Blender only ever runs a
  daemon-generated script, under a timeout and a sandbox profile.
- **Blender absent** (true on this machine): every 3D feature works without it;
  the bridge is additive.
- **Ingest race**: two agents publish first → `swarm_projects.story_id` unique
  index; `update_project` maps the violation to `Conflict`, the loser re-reads
  the project and files under the winner's epic.
- **Disk**: this machine has ~9 GB free with a 52 GB `target/`. Only the Rust
  track runs cargo, with `CARGO_INCREMENTAL=0`.
- **Existing mockups**: unchanged rows, still render; the tab id `mockups` is
  kept as an alias for deep links, the label becomes **Design**.

## 7. Delivery plan (3 parallel tracks, strict file ownership)

All tracks work on branch `feat/product-design-arena` in this checkout. A file is
owned by exactly one track; the shared contract between tracks is **this
document** (DTO names, routes, event shapes, the `scene3d` schema, component
props below). No track edits another track's files; cross-track needs are
written into the doc's §8 "hand-offs" list instead.

### Track A · Rust (only track that runs cargo; `CARGO_INCREMENTAL=0`)
Owns: `crates/**` EXCEPT `crates/otto-skills/assets/skills/development/{otto-design-2d,otto-design-3d,otto-mockup}/`,
plus `docs/contracts/**`. Does NOT touch `docs/features/product.md`; A posts the
API summary text for it into §8 and C folds it in.
1. `0115_product_epic_tree.sql`; `ProductStory/NewStory/StoryPatch` + row mapper;
   `delete_story` re-parent + attachment/annotation row cleanup (+ best-effort file
   cleanup in the server delete path); `get_children(parent_id)`; `make_story`
   test fix.
2. `otto-product`: `UpdateStoryReq` fields (+ one-level validation);
   `POST /product/stories/{sid}/children`; `ProductStoryDetail` unchanged.
3. `swarm.rs::update_project` → `Conflict` on unique violation.
4. `swarm_ingest.rs`: epic resolution (§3.1), title-dedupe under the epic,
   `otto-mockup` target resolution; helpers in `swarm_workspace.rs` (flags in the
   `case` block, `curl -f`, non-zero exit); `swarm_run.rs` prompt sentence.
5. `design_format.rs` (`DesignFormat` enum + `FromStr` → 400), used by
   `mockup_assist.rs` (+ excalidraw/scene3d stubs and prompts) and `swarm_ingest.rs`.
6. `product_media.rs`: mime allow-list + sniff + `ext_for_mime` +
   `default_kind_for_mime`; `PUT /product/attachments/{aid}/content`;
   `modules.rs` route + body-limit layer; `resolve_skill_inline` bundled-skill arm.
7. `otto-core/event.rs`: `MockupUpdated.content: Option<String>`; `ws.md`.
8. `design_scene3d.rs` (validate + `to_blender_script`), `design_blender.rs`
   (detect / render job + in-memory job map + `GET /design/jobs/{id}` / script
   download), `otto-sandbox::SandboxPolicy::for_tool`.
9. Unit tests for: format parsing, epic resolution helpers, scene3d validation,
   blender script generation (golden), content PUT guards. `cargo test -p` for the
   touched crates + `cargo clippy --workspace --all-targets -- -D warnings`.

### Track B · UI shell, tree, 2D arena
Owns: `ui/src/modules/product/**` EXCEPT `design/scene3d/**`, `ui/src/lib/stores/product.svelte.ts`,
`ui/src/lib/stores/mockup-assist.svelte.ts`, `ui/src/lib/api/types.ts` (the
`mockup_updated.content` type), `ui/src/lib/events.svelte.ts` (the null-content
branch), `ui/src/modules/mcp/templates.ts` + `ServerForm.svelte` (template picker
only), `ui/e2e/product-mockups.spec.ts`, `ui/e2e/product-design-arena.spec.ts`,
`ui/e2e/product-epic-tree.spec.ts`. B owns the **debounce (600 ms), dirty and
conflict state** for every artifact; C's components emit `onchange` on every
mutation, undebounced.
1. `types.ts`: `ProductStory` fields, `DesignFormat`, `CreateChildReq`,
   `BlenderStatus`, `MockupUpdated.content: string | null`.
2. Store: `tree` derived, `createChild`, `moveStory`, `saveAttachmentContent`
   (debounced PUT), `blenderStatus()`, `blenderRender()`.
3. Tree list pane + context menu + breadcrumb + "New ▾ / Add child ▾" (§3.2);
   epic Overview children board.
4. `design/DesignArena.svelte` (assets / viewport / inspector / assistant shell,
   §4), `design/DesignBoard.svelte` (Excalidraw island: props `source: string`,
   `readonly?: boolean`, `onchange(source: string)`), `design/DeviceFrame.svelte`,
   `design/templates/*.{html,excalidraw,mmd,json}` + `templates.ts`,
   `design/exporters.ts` (PNG/SVG/source). Mount `Scene3DViewport` from Track C
   via the props contract in §7-C; until C lands, render a placeholder.
5. Tab label **Design** (id `mockups` kept); `MockupsTab.svelte` becomes a thin
   re-export of `DesignArena` (or is deleted once specs are updated).
6. MCP Blender template.
7. e2e: update mockups spec label; new arena spec (create board via API seed →
   renders island; create scene3d → hierarchy lists objects; PUT content
   round-trip); new epic-tree spec (seed epic + 2 children in 2 folders → tree
   renders, move via context menu persists after reload).
8. `npm run check` + `npm run build` green; run the three product specs with
   `OTTO_E2E_BIN` pointing at Track A's build once it exists.

### Track C · 3D viewport + skills + docs
Owns: `ui/src/modules/product/design/scene3d/**`, `crates/otto-skills/assets/skills/development/{otto-design-2d,otto-design-3d,otto-mockup}/`,
`docs/features/product.md` (§ Design Arena walkthrough, not the API sections).
1. `scene3d/types.ts` + `scene3d/validate.ts` (mirror of the Rust validator),
   `scene3d/toBlender.ts` is NOT needed (server does it) — instead
   `scene3d/exportGlb.ts` (three `GLTFExporter`).
2. `scene3d/Scene3DViewport.svelte` — props: `doc: Scene3dDoc`, `readonly?:
   boolean`, `selectedId: string | null` (bindable), `play?: boolean`,
   `onchange(doc: Scene3dDoc)` (debounced by the caller), `resolveAttachment(aid:
   string): Promise<string>` (returns a blob URL; provided by Track B). Lazy
   `import('three')` + `examples/jsm/{OrbitControls,TransformControls,GLTFLoader}`.
   Grid, axes, shadows, gizmo with W/E/R/F/Del/⌘D/Esc, click-select, drag-select
   off (v1).
3. `scene3d/Hierarchy.svelte` (props: `doc`, `selectedId` bindable, `onchange`) —
   list/tree with groups, rename, hide, duplicate, delete, add-primitive menu.
4. `scene3d/Inspector.svelte` (props: `doc`, `selectedId`, `onchange`) — transform,
   material, light, camera, scene panels; numeric drag inputs.
5. `scene3d/index.ts` re-exporting the three components + `emptyScene()` +
   `SCENE3D_MIME`.
6. Skills: `otto-design-2d` (Excalidraw JSON + HTML screens: artboards, grid,
   palette, components, templates), `otto-design-3d` (the schema, blockout
   conventions, lighting recipes, prefer Blender MCP when present), update
   `otto-mockup` to point at the arena and the new formats.
7. All document mutations (add/duplicate/delete/rename/transform/material) live
   in `scene3d/ops.ts`, pure functions `(doc, …) => doc`; Viewport (⌘D/Del/gizmo)
   and Hierarchy/Inspector both call them — no component mutates `doc` directly.
8. Docs: the WHOLE `docs/features/product.md` update (Design Arena walkthrough +
   epic tree + the API summary A posts in §8); `npm run check` must pass for
   `scene3d/**`.

## 8. Hand-offs & gates (append-only during implementation)
- **Gate 0 (C, first thing):** commit a stub `ui/src/modules/product/design/scene3d/{types.ts,ops.ts,index.ts}`
  exporting `Scene3dDoc`, `emptyScene()`, `SCENE3D_MIME`, and placeholder
  `Scene3DViewport/Hierarchy/Inspector` components with the final props, so B's
  `npm run check` never blocks on C.
- **Gate 1 (A):** Rust builds + tests green → B runs the product e2e specs with
  `OTTO_E2E_BIN=<worktree ottod>`. Until then B's specs are written but not run.
- B → C: `resolveAttachment` is `product.attachmentBlobUrl(aid)` in the store.
- A → B: route/DTO names exactly as §5; the assist route returns the attachment
  immediately and streams via `MockupUpdated`/`MockupSessionStarted` as today.
- A → C: A appends the API summary paragraph for `docs/features/product.md` here.
- **Gate 0 (C): stub landed** — `ui/src/modules/product/design/scene3d/{types.ts,ops.ts,index.ts,Scene3DViewport.svelte,Hierarchy.svelte,Inspector.svelte}` export `Scene3dDoc`, `emptyScene()`, `SCENE3D_MIME` and the three components with the final §7-C props (`import … from "../design/scene3d"`). Hierarchy additionally takes `onimportGlb?: () => void` for B to wire the GLB upload.
- **C → A/B (skills + exports):** `otto-design-2d` / `otto-design-3d` / updated `otto-mockup` (v2) landed under `crates/otto-skills/assets/skills/development/`; `otto-skills` enumerates bundles via `include_dir!`, so **no Rust registration edit is needed** — only A's `resolve_skill_inline` bundled-skill arm (§5) for the assist prompts. `scene3d/index.ts` additionally exports `validate/parseScene/serializeScene`, `exportSceneToGlb(doc, resolveAttachment)` / `exportObjectToGlb(root)` / `glbFileName`, and the viewport instance exposes `snapshotPng()` (PNG blob) and `contentRoot()` (for GLB export) for B's Export ▾. Hierarchy/Inspector also accept `readonly?`. `docs/features/product.md` §5 "Design Arena" API table was written from §5 of this doc because A's summary had not been posted yet — A: correct it in place if a name differs.
- **A → C (API summary for `docs/features/product.md`):** The epic tree lives on `ProductStory` as `parent_id` (`null` = top level), `tree_kind` (`story` | `epic` | `doc`) and `folder`; `GET /workspaces/{ws}/product/stories` stays flat and the UI derives the tree. One level only: `PATCH /product/stories/{sid}` accepts `parent_id` (`null` detaches; absent = unchanged), `tree_kind` and `folder` and returns 400 when the parent is itself a child, the story already has children, it parents itself, or `tree_kind` is unknown; `POST /product/stories/{sid}/children { title?, tree_kind?: 'story'|'doc' (default doc), folder? }` files a draft child under an epic and returns `ProductStoryDetail`. Deleting a parent re-parents its children (never cascades) and also removes the story's attachments/annotations rows + files. Swarm agents publish with `./otto-product --title T [--folder F] [--kind doc|story] "<md>"`: with a swarm project the draft lands under the project's epic (its linked story, else ONE `tree_kind:'epic'` draft minted per project — race-safe), `folder` defaults to the agent's role title, and re-publishing the same normalized title UPDATES the child (new `suggested` version) instead of duplicating; `product_changed { section:"tree", status:"changed" }` is broadcast. Design artifacts are `product_attachments` rows of `kind` `mockup` (html/mermaid) or `design` (excalidraw / scene3d / uploaded glb/gltf) with `meta_json { format, group, assist_session_id?, derived_from? }`; the one `DesignFormat` enum is `html | mermaid | excalidraw | scene3d` (mimes `text/html`, `text/vnd.mermaid`, `application/vnd.excalidraw+json`, `application/vnd.otto.scene3d+json`; exts `.html/.mmd/.excalidraw/.json`) and an unknown format is a 400 on every path (`POST …/mockups/assist`, `otto-mockup --format`, content PUT). `PUT /product/attachments/{aid}/content { data_b64 }` saves an edited artifact (same guards as upload; `scene3d` schema-validated; mime/filename never change) and broadcasts `mockup_updated` whose `content` is now `string | null` (null for binaries / >4 MB → re-fetch `GET /product/attachments/{aid}`). `scene3d` is y-up, metres, degrees; primitives have a unit bounding box before `scale` (box 1×1×1, sphere Ø1, cylinder/cone Ø1×h1, plane 1×1, torus R 0.5 / r 0.2); ≤ 2 000 objects; `gltf` objects reference `attachment_id` only. Blender bridge: `GET /product/design/blender → { installed, path, version }` (`$OTTO_BLENDER` → PATH → Blender.app; `installed:false` cleanly when absent), `POST /product/stories/{sid}/design/{aid}/blender-render → 202 { id }` (409 when Blender is absent, 400 on an invalid document; Editor-only; runs the server-generated script under `SandboxPolicy::for_tool` with no network and a 120 s cap; outputs `render.png` + `scene.glb` are attached as `kind:'design'` with `meta.derived_from = aid` and announced via `mockup_updated { content: null }`), `GET /product/design/jobs/{id} → { id, attachment_id, status: queued|running|done|error, error, outputs, started_at, finished_at }` (in-memory, pruned ~1 h after finishing), `GET /product/stories/{sid}/design/{aid}/blender-script` downloads the generated `.py`. `otto-mockup --title T --format html|mermaid|excalidraw|scene3d [--folder G] "<content>"` now works outside Discovery too (target = discovery run's story → `project.story_id` → the project's epic) and both helpers fail loudly (HTTP status on stderr, non-zero exit). The `excalidraw`/`scene3d` assist prompts inline the bundled `otto-design-2d` / `otto-design-3d` skills via `resolve_skill_inline`'s new `otto_skills::bundled_body` arm.
- **B → A (deviation, MCP template):** the control plane's `default_tool_access` is `allow | deny` only (`McpToolAccess` in TS, the `mcp_servers` column comment) — there is no `ask`. The Blender template (`ui/src/modules/mcp/templates.ts`) ships with `deny` (the user grants tools explicitly) and disabled; if A adds an `ask` level, flip the template.
- **B ← C (wired):** the arena consumes C's `parseScene` / `serializeScene` (validation before render + save, issues listed in the stage) and `exportSceneToGlb(doc, resolveAttachment)` for Export ▾ → GLB; the scene3d PNG snapshot reads the viewport `<canvas>` via `toBlob` (C's renderer sets `preserveDrawingBuffer: true`).
- **B (deviation, New ▾):** "Blender script" is NOT a New ▾ entry — §2.2 says a Blender script is an export generated server-side from a validated `scene3d`, never an attachment — so it lives under Export ▾ (and the 3D inspector's Blender section) only.
- **B note (arena rows):** the arena lists `kind ∈ {mockup, design}` plus anything whose mime/filename classifies as a design format, an image, or a glb/gltf, so pre-`design` uploads of `.mmd`/`.html`/`.glb` still appear. New artifacts (New ▾ / templates / Import / GLB import) upload as `kind:'design'` with the §2.2 mimes; the arena depends on A's `allowed_mime` + `sniff_ok` additions for excalidraw / scene3d / glb / gltf.
- **Gate 1 (A): PASSED** — `CARGO_INCREMENTAL=0 cargo test -p otto-state -p otto-product -p otto-sandbox` (all green, incl. 3 new epic-tree repo tests, 2 new swarm link/conflict tests, 2 new product children/patch route tests, 1 sandbox `for_tool` test), `CARGO_INCREMENTAL=0 cargo test -p otto-server` (626 unit + all integration suites passed, 0 failed; new tests: design_format ×5, design_scene3d ×6 incl. golden blender script, design_blender ×4, product_media content-PUT guards ×3, mockup_assist ×6, swarm_ingest ×3, swarm_workspace helper-script ×1), `CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets -- -D warnings` (Finished, 0 warnings), `CARGO_INCREMENTAL=0 cargo build -p ottod`. Built daemon for `OTTO_E2E_BIN`: `/Users/tech-ai/otto_os/target/debug/ottod`.
- **B (e2e run, Gate 1):** `product-mockups` (5) + `product-design-arena` (5) + `product-epic-tree` (4) = **14 passed (48.9s)** against `OTTO_E2E_BIN=target/debug/ottod`. Ran on the installed Chromium at 1280×900 via a throwaway config (deleted): the repo's WebKit projects hang in `browser.newContext` on this machine with the freshly downloaded `webkit-2311` (quarantine cleared, still stalls) — not a test failure; re-run on WebKit once that launch issue is understood.
- **Gate 1 (A) re-run after code review (13:35):** helper scripts treat `--`/`---`-leading bodies as body (unknown-flag arm only when body empty + single token); Blender render capped at 2 concurrent + one in-flight per attachment (409 otherwise), `GET /product/design/blender` probe cached 60 s; assist poll/reads capped at 25 MB, invalid `scene3d` never broadcast, payload via `event_content`; `PUT …/content` accepts `base_updated_at` → 409 on a stale save; story delete also removes `product/mockup_assist/<aid>/` scratch dirs (tokio fs); ingest filenames sanitized. `cargo test -p otto-server` 630 passed / 0 failed; `cargo clippy -p otto-server --all-targets -- -D warnings` Finished; `cargo build -p ottod` → `/Users/tech-ai/otto_os/target/debug/ottod`.
