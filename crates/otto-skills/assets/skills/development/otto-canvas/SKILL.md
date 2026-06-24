---
description: Use when drawing/generating a diagram for the Otto Canvas (the embedded Excalidraw editor), creating or editing canvas scenes, or driving Discovery Chat on a Product story, from an agent session over HTTP. Covers the EXCALIDRAW element-skeleton output format (top-notch flowcharts with code blocks, emoji icons, a colour palette and clean layout), the OTTO_API_TOKEN auth model, the Canvas + Discovery-Chat endpoints, the canvas.mjs helper, and a mermaid fallback.
category: development
version: 2
---

# Otto Canvas (Excalidraw) + Discovery Chat over HTTP

Otto's **Canvas** is the real **Excalidraw** editor embedded in the app. A scene
is ONE JSON document (`doc_json`) the server stores opaquely; for the canvas it
holds Excalidraw's `{ type:"excalidraw", elements, appState, files }`. **Discovery
Chat** is a conversational product-discovery agent on a story that can *propose* a
canvas you then apply. This skill teaches an agent to **generate a top-notch
diagram** for the canvas and to drive the canvas/discovery HTTP API.

Reach for this skill whenever you're asked to **draw/diagram something on the
canvas** ("show how service A calls B", "sketch the auth flow", "diagram the
order pipeline"), **create/edit a scene**, or **run a discovery chat** on a story.

## Drawing a diagram — output the Excalidraw element SKELETON (preferred)

When asked to draw, emit a SINGLE fenced ```json block `{"elements":[ … ]}`. The
app converts the skeleton into real, EDITABLE Excalidraw shapes (auto-routing
arrows). This is preferred over Mermaid because it gives **true code blocks,
emoji icons, frames and full styling**.

**Skeleton shape (use exactly this):**
- Shape — `{"type":"rectangle"|"ellipse"|"diamond","id":"n1","x":int,"y":int,"width":int,"height":int,"backgroundColor":"#hex","strokeColor":"#hex","fillStyle":"solid","roundness":{"type":3},"label":{"text":"…","fontSize":16,"fontFamily":2,"strokeColor":"#hex"}}`
- Arrow (connect by node id) — `{"type":"arrow","x":int,"y":int,"start":{"id":"n1"},"end":{"id":"n2"},"strokeColor":"#94a3b8","label":{"text":"yes"}}`
- `fontFamily`: `2` = normal, `3` = **code (monospace)**.

**Make it top-notch:**
- **Layout (you own coordinates):** left→right for pipelines, top→down for
  processes; space nodes ~80px apart with NO overlaps; unique id per node; connect
  everything with labeled arrows (label = the data/event flowing).
- **Size every box to its text** so nothing clips: `width ≥ 28 + 9·chars-of-longest-line`,
  `height ≥ 28 + 22·lines`.
- **Colour palette by role:** start `#dcfce7`/`#16a34a` · process `#eef2ff`/`#6366f1`
  · decision (DIAMOND) `#fef9c3`/`#ca8a04` · io `#f3e8ff`/`#9333ea` · data
  `#ecfeff`/`#0891b2` · done/error `#fee2e2`/`#dc2626`. Dark readable text.
- **Code blocks:** a rectangle, `backgroundColor:"#0f172a"`, `strokeColor:"#334155"`,
  `label` with `fontFamily:3`, `strokeColor:"#e2e8f0"`, the REAL code (`\n` between
  lines); make it wide + tall enough for every line.
- **Icons:** prefix labels with a fitting emoji (🌐🔐📦💳🧾📣🗄️⚡🛞✅❌⏳).
- **Decisions** are diamonds with labeled out-arrows (yes/no) + error/retry paths.

> **Mermaid fallback:** for `sequenceDiagram` / `classDiagram` / `erDiagram`
> (where auto-layout beats hand coordinates) emit a single ```mermaid block
> instead — the app renders it through mermaid-to-excalidraw. See
> `references/mermaid-cheatsheet.md`. Everything else: prefer the element skeleton.

## Auth model (do this first)

Everything is a call to the running `ottod` daemon:

- **Base URL:** `OTTO_BASE` (default `http://127.0.0.1:7700`), API prefix `/api/v1`.
- **Token:** export `OTTO_API_TOKEN` (a Bearer token). Every route below sends
  `Authorization: Bearer $OTTO_API_TOKEN`. Mint one with the `otto-api` skill's
  `otto-setup-token.sh` if you don't have it. Verify with
  `node scripts/canvas.mjs whoami` (the helper hits `GET /auth/me`).
- **Roles:** reads need workspace **Viewer**; all writes (create/update/delete a
  scene, assist, every discovery-chat mutation) need workspace **Editor**.
- **Errors:** non-2xx returns `{"code","message"}` (`unauthorized` 401,
  `forbidden` 403, `not_found` 404, `invalid` 400, `conflict` 409). The helper
  prints `HTTP <code>` + the body and exits non-zero — never swallow that.

You need a **workspace id** for the collection routes and (for discovery) a
**story id**. Resolve a workspace id with the `otto-api` client (`otto ws-id`).

## The `canvas.mjs` helper (your main tool)

A zero-dependency Node ESM script (uses global `fetch`). Run it from this skill
dir. Full usage is in its header (`node scripts/canvas.mjs --help`):

```bash
node scripts/canvas.mjs list-scenes   <wsId>
node scripts/canvas.mjs create-scene  <wsId> "<title>" [storyId]   # empty scene
node scripts/canvas.mjs get-scene     <sceneId>
node scripts/canvas.mjs add-mermaid   <sceneId> "<mermaid src>"    # append a mermaid node
node scripts/canvas.mjs add-slide     <sceneId> "<slide title>"    # slide revealing all nodes
node scripts/canvas.mjs assist        <sceneId> "<prompt>" [mode]  # assist on a scene
node scripts/canvas.mjs assist        --preview  "<prompt>" [mode] # assist with no scene
```

`add-mermaid` / `add-slide` do a read-modify-write: GET the scene, `JSON.parse`
its `doc_json`, append a node/slide per the schema, then `PUT` the whole doc back.

## Canvas doc (Excalidraw) + legacy Scene schema

The canvas now stores **Excalidraw JSON** in `doc_json`:
`{ "type":"excalidraw", "elements":[…], "appState":{…}, "files":{…} }` — the UI
owns it. To add to a canvas, generate the element skeleton (above) and let the UI
apply it; you don't hand-edit `doc_json`.

The schema below is the **legacy non-Excalidraw Scene** form (still accepted by
older scenes / `canvas.mjs add-mermaid`):

A `Scene` is `{ schema: 1, title, nodes[], edges[], slides[], appState? }`.

- **node** — `{ id, kind, x, y, w, h, z?, rotation?, label?, parent?, <payload>, style? }`.
  `x/y/w/h` are scene-space (pre-zoom). `kind` ∈ `shape | text | sticky | freehand
  | code | json | mermaid | image | group | frame`, and the matching **payload
  key** carries the content:
  - `shape: { variant: rect|roundrect|ellipse|diamond|triangle|cylinder|parallelogram, fill?, stroke?, sketch? }`
  - `text: { value, align?, size? }` · `sticky: { value, color? }`
  - `code: { value, lang? }` · `json: { value }` (raw JSON text)
  - **`mermaid: { src, kind? }`** — a whole diagram in one node (the killer path)
  - `image: { attachmentId?, dataUrl? }` · `freehand: { points: [[x,y,pressure?]…], color?, size? }`
  - `frame` / `group` nest children via their `parent` id (frames define slide viewports).
- **edge** — `{ id, source, target, sourceAnchor?, targetAnchor?, kind?: arrow|line|dashed, label?, style? }`.
  (Connect freeform nodes; mermaid diagrams carry their own internal arrows.)
- **slide** — `{ id, title?, frameNodeId?, mermaidNodeId?, reveal: RevealStep[], notes? }`.
  A `RevealStep` is `{ nodeIds?: string[], mermaidMessageRange?: [from,to] }`. To
  present a sequence step-by-step, set `mermaidNodeId` and reveal message ranges.
- **appState** — `{ background?, grid? }`.

> **Prefer one `mermaid` node** for sequence/flow/class(UML)/state/ER diagrams —
> it's the densest, most reliable form. Use freeform `nodes`+`edges` only for
> layouts mermaid can't express. Full schema: `references/scene-schema.md`.

## Two killer workflows

### 1) Draw a diagram (the common case)

When asked to visualize a flow ("service A calls B, B does 10 things"):

1. Pick/author the mermaid — a `sequenceDiagram` is usually right; **be
   exhaustive**, one message per sub-step so nothing is hidden (see
   `references/mermaid-cheatsheet.md` and `examples/service-fanout.md`).
2. Create a scene (or reuse one), then add the diagram:
   ```bash
   SID=$(node scripts/canvas.mjs create-scene "$WID" "Auth flow" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
   node scripts/canvas.mjs add-mermaid "$SID" "sequenceDiagram
     A->>B: request
     B->>B: step 1
     B-->>A: response"
   ```
3. **Let the agent draft it for you** instead of writing mermaid by hand — call
   `assist`, then add the returned `mermaid` (or `nodes`/`edges`):
   ```bash
   node scripts/canvas.mjs assist "$SID" "sequence: service A calls B, and B does 10 things" sequence
   ```
   `assist`/`assist --preview` return `{ excalidraw?, mermaid?, nodes, edges, note }`
   and **do not mutate the scene** — the canvas UI applies the returned diagram.
   Prefer `excalidraw` (the element skeleton above); `mermaid` is the fallback.

### 2) Present a scene as slides

After building a diagram, add a slide so it can be walked through:

```bash
node scripts/canvas.mjs add-slide "$SID" "Walkthrough"
```

`add-slide` appends a slide whose `reveal` exposes every current node. To step a
sequence message-by-message, set the slide's `mermaidNodeId` and use
`mermaidMessageRange` reveal steps — see `examples/sequence-to-slides.md`.

### Discovery Chat (drive product discovery from a story)

To research/shape a story conversationally (it can propose a canvas you apply):

```bash
# Start a chat on a story, then send a turn:
otto POST /product/stories/$SID/discovery-chats '{"title":"Discovery"}'    # → {id: CID, …}
otto POST /product/discovery-chats/$CID/messages '{"body":"How should login lockout work?"}'
otto GET  /product/discovery-chats/$CID                                    # chat + transcript
```

The agent replies in markdown and MAY emit a fenced `json` `{actions:[…]}` block.
**Actions are never auto-applied** — they come back on the agent message as
`actions_json`; apply ONE at a time:

```bash
otto POST /product/discovery-chats/$CID/apply \
  '{"action":{"type":"create_canvas","title":"Login flow","mermaid":"sequenceDiagram\n …"}}'
```

Supported action `type`s: `apply_draft` (sets the story draft), `add_questions`,
`add_notes`, `create_canvas` (creates a scene → returns `canvas_id`). Exact
bodies/responses: `references/endpoints.md`.

## Read-only MCP vs. writes through this skill

Otto's first-party MCP tool server (`ottod mcp-tools`) is **read-only by hard
invariant — it only issues HTTP `GET`s, never a write.** If the integrator wires
canvas read tools, you may see `canvas_list_scenes` / `canvas_get_scene` as MCP
tools for *inspecting* scenes. **All writes** — create/update/delete a scene,
`assist`, and every discovery-chat mutation — go through **this skill's
`canvas.mjs` HTTP scripts** (or the `otto` client) with `OTTO_API_TOKEN`. Don't
expect an MCP tool to mutate a canvas; there isn't one and won't be.

## References & examples

- `references/scene-schema.md` — the complete Scene schema (mirrors the UI's
  `canvas/types.ts`).
- `references/endpoints.md` — every Canvas + Discovery-Chat endpoint with
  method/path/body/response.
- `references/mermaid-cheatsheet.md` — sequence/flowchart/class/state/ER syntax +
  the fan-out pattern.
- `examples/service-fanout.md` — "A calls B; B does 10 things" → exhaustive
  sequence diagram on a scene.
- `examples/sequence-to-slides.md` — build a sequence, then present it step by step.

## Common mistakes

- Forgetting `OTTO_API_TOKEN` → every call 401. Check `node scripts/canvas.mjs whoami`.
- Hiding fan-out: collapsing "B does 10 things" into one box. Emit all 10 as
  distinct messages/nodes — the assist prompt itself demands exhaustiveness.
- Treating `assist` as a save: it only *returns* blocks. You must `add-mermaid`
  (or PUT nodes/edges) to persist them.
- Auto-applying discovery actions: they are proposals. `apply` exactly the one
  the user approves.
- Using a session id where a workspace/story id is needed: `list`/`create` scenes
  are **workspace-scoped** (`/workspaces/{ws}/canvas/scenes`); `get`/`put`/`delete`
  are **flat by scene id** (`/canvas/scenes/{id}`).
- Hand-editing `doc_json` as a string: always `JSON.parse` → mutate the object →
  PUT it back under `doc` (the helper does this for you).
