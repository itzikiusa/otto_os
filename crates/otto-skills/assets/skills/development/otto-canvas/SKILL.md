---
description: Use when drawing/generating a diagram for the Otto Canvas, creating or editing canvas scenes, or driving Discovery Chat on a Product story, from an agent session over HTTP. Covers the FILE-BACKED canvas model with TWO modes — Mermaid (edit a per-scene `canvas.mermaid`; any diagram type) and Excalidraw (edit a per-scene `canvas.json`; fully editable shapes) — top-notch pretty-Mermaid styling, the Excalidraw simplified element format, the OTTO_API_TOKEN auth model, the Canvas + Discovery-Chat endpoints, and the canvas.mjs helper.
category: development
version: 4
---

# Otto Canvas (Mermaid + Excalidraw modes) + Discovery Chat over HTTP

Otto's **Canvas** is **file-backed** with TWO modes the user picks per scene; the
agent edits the scene's file and the board re-renders (manual edits write back to
the same file too):

- **Mermaid mode** — the agent edits a per-scene **`canvas.mermaid`**. The UI renders
  it with Mermaid's own renderer, so ANY diagram type works (flowchart, sequence,
  class/UML, ER, state). Best for rich auto-laid-out diagrams.
- **Excalidraw mode** — the agent edits a per-scene **`canvas.json`** (an Excalidraw
  scene). The UI loads it in the real Excalidraw editor, so the user can also draw /
  move / restyle shapes by hand. Best for freeform, hand-editable boards.

The scene's `doc_json` stores `{ "type":"otto-canvas", "version":1, "format":"mermaid"|"excalidraw", "source":"…" }`
(`source` is the file's text). The agent refines the SAME file across the conversation,
so follow-ups *change* the diagram instead of regenerating it, and the work is never
lost. **Discovery Chat** is a conversational product-discovery agent on a story that
can *propose* a canvas you then apply.

## Excalidraw mode — edit canvas.json

In Excalidraw mode you EDIT `canvas.json`. Write the COMPLETE diagram as
`{"type":"excalidraw","elements":[ … ]}` using the SIMPLIFIED element form (the app
expands it into a real Excalidraw scene — do NOT include `seed`/`versionNonce`):
- Shape: `{"type":"rectangle"|"ellipse"|"diamond","id":"n1","x":int,"y":int,"width":int,"height":int,"backgroundColor":"#hex","strokeColor":"#hex","fillStyle":"solid","label":{"text":"…","fontSize":16,"fontFamily":2}}`
- Arrow (routed by id): `{"type":"arrow","start":{"id":"n1"},"end":{"id":"n2"},"label":{"text":"yes"}}`
- Text: `{"type":"text","x":int,"y":int,"text":"…","fontSize":20}` · `fontFamily:3` = code.
Lay nodes ~80px apart with no overlaps, size boxes to text, colour-code by role.

Reach for this skill whenever you're asked to **draw/diagram something on the
canvas** ("show how service A calls B", "sketch the auth flow", "diagram the
order pipeline"), **create/edit a scene**, or **run a discovery chat** on a story.

## Drawing a diagram — edit the source (Mermaid is the default)

When the canvas "Ask AI" turn runs, you are placed in the scene's directory with
the source file already there. **Read it, make the requested change by editing it
in place, and save it** — keep it the COMPLETE, valid diagram (no ``` fences
inside the file). Make minimal, additive edits across the conversation; don't
rewrite from scratch unless asked. Driving the API directly instead? Return the
diagram from `assist` (a ```mermaid block) and the server commits it for you.

Mermaid is the default because you edit TEXT (the layout engine places nodes) —
far faster and more reliable than hand-computing coordinates for dozens of shapes.

**Make the Mermaid top-notch (pretty by default):**
- `flowchart TD` (top-down) or `LR` (pipelines). Short labels, each with a leading
  emoji icon, e.g. `A["🚀 Start"]` (🌐🔐📦💳🧾📣🗄️⚡✅❌⏳).
- **Decisions** are rhombus nodes `B{"❓ Valid?"}` with LABELLED edges
  `B -->|yes| C` / `B -->|no| E`; include error/retry paths, not just the happy one.
- Group related steps with **`subgraph` lanes** (e.g. `Client` / `API` / `Data`).
- **Colour-code by role** with `classDef` + `class` (put these at the END):
  - `classDef start fill:#dcfce7,stroke:#16a34a,color:#064e3b;`
  - `classDef process fill:#eef2ff,stroke:#6366f1,color:#1e1b4b;`
  - `classDef decision fill:#fef9c3,stroke:#ca8a04,color:#422006;`
  - `classDef io fill:#f3e8ff,stroke:#9333ea,color:#3b0764;`
  - `classDef data fill:#ecfeff,stroke:#0891b2,color:#083344;`
  - `classDef error fill:#fee2e2,stroke:#dc2626,color:#7f1d1d;`
  then assign with `class A,B start;`. Code/commands go in a node label with
  `<br/>` between lines.
- Be accurate + reasonably complete, but keep node text short.

> **The board is editable.** Subgraph lanes, `classDef` colours, node shapes and
> edge labels all parse into editable nodes (and round-trip back to Mermaid when
> the user hand-edits), so keep the source clean flowchart Mermaid — that's what
> the user actually sees and edits. Sequence/class/ER diagrams render read-only.

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

## Canvas doc (file-backed) + legacy Scene schema

The canvas stores the **agent-edited source** in `doc_json`:
`{ "type":"otto-canvas", "version":1, "format":"mermaid", "source":"…", "positions":{…} }`.
The UI parses `source` into editable flow nodes (`positions` keeps the user's
hand-arranged layout, which Mermaid itself can't carry). You don't hand-edit
`doc_json` over the wire — call `assist` and the server commits the new source for
you (the UI also writes it back when the user drags/adds nodes).

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
   node scripts/canvas.mjs assist "$SID" "service A calls B, and B does 10 things"
   ```
   `assist` on a scene now EDITS the scene's source file and **commits it**
   (updating `doc_json`), then returns `{ mermaid?, excalidraw?, format, note }`
   and broadcasts the live update. `assist --preview` (no scene) just returns the
   blocks without persisting. The default `format` is `mermaid`.

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
- For a SCENE, `assist` now commits the edited source server-side (and returns
  it); only `assist --preview` (no scene) is non-persisting. Don't double-write.
- Auto-applying discovery actions: they are proposals. `apply` exactly the one
  the user approves.
- Using a session id where a workspace/story id is needed: `list`/`create` scenes
  are **workspace-scoped** (`/workspaces/{ws}/canvas/scenes`); `get`/`put`/`delete`
  are **flat by scene id** (`/canvas/scenes/{id}`).
- Hand-editing `doc_json` as a string: always `JSON.parse` → mutate the object →
  PUT it back under `doc` (the helper does this for you).
