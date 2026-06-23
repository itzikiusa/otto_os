---
description: Use when creating or editing Otto Canvas Studio scenes, or driving Discovery Chat on a Product story, from an agent session over HTTP. Covers the OTTO_API_TOKEN auth model, the Scene JSON schema (nodes/edges/slides; mermaid/shape/text/sticky/code/json/image/frame/freehand), the exact Canvas + Discovery-Chat endpoints, the canvas.mjs helper, a mermaid cheat sheet, and worked examples (service fan-out, sequence→slides).
category: development
version: 1
---

# Otto Canvas Studio + Discovery Chat over HTTP

Otto's **Canvas Studio** is an infinite-canvas diagram + presentation surface; a
scene is ONE JSON document (`doc_json`) the server stores opaquely while the UI
owns the rich schema. **Discovery Chat** is a conversational product-discovery
agent on a story that can *propose* a canvas (and questions/notes/draft) you then
apply. This skill teaches an agent in a session to drive both over the daemon's
HTTP API — the easiest path is the bundled `scripts/canvas.mjs` helper.

Reach for this skill whenever you're asked to **draw a diagram on the canvas**
("show how service A calls B", "sketch the auth flow", "turn this into slides"),
**create/edit a scene**, or **run a discovery chat** on a Product story from a
session.

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

## Scene schema (summary)

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
   `assist`/`assist --preview` return `{ mermaid?, nodes, edges, note }` and **do
   not mutate the scene** — you decide what to insert (use `add-mermaid` with the
   returned `mermaid`).

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
