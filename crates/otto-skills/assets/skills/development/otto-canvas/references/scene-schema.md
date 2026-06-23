# Scene schema reference

The full Canvas Studio document schema. This mirrors the UI's single source of
truth, `ui/src/modules/canvas/types.ts`. A **scene is ONE JSON document** stored
server-side as the `doc_json` string of a `CanvasScene` row; the server treats it
as opaque, so the schema below is the real contract.

When you create or update a scene over HTTP you send the document under a `doc`
key (`CreateSceneReq.doc` / `UpdateSceneReq.doc`); the server `JSON.stringify`s it
into `doc_json`. When you read a scene back, parse `doc_json` with `JSON.parse`.

---

## `Scene` (the document)

```jsonc
{
  "schema": 1,            // literal 1 — the schema version
  "title": "Auth flow",   // scene title (mirrors the row's title)
  "nodes": [ /* CanvasNode[] */ ],
  "edges": [ /* CanvasEdge[] */ ],
  "slides": [ /* Slide[] */ ],
  "appState": { "background": "#fff", "grid": true }   // optional
}
```

The **empty document** the server stores when you create a scene with no `doc`:

```json
{ "schema": 1, "title": "<title>", "nodes": [], "edges": [], "slides": [], "appState": { "grid": true } }
```

---

## `CanvasNode`

```jsonc
{
  "id": "n1",            // string, unique within the scene
  "kind": "mermaid",     // NodeKind (see below) — selects which payload key applies
  "x": 80, "y": 80,      // scene-space top-left (pre-zoom), numbers
  "w": 520, "h": 360,    // size, numbers
  "z": 0,                // optional stacking order
  "rotation": 0,         // optional degrees
  "label": "…",          // optional caption
  "parent": "f1",        // optional parent frame/group id (nesting / slide membership)
  // EXACTLY ONE payload key matching `kind`:
  "shape":    { /* ShapePayload */ },
  "text":     { /* TextPayload */ },
  "sticky":   { /* StickyPayload */ },
  "freehand": { /* FreehandPayload */ },
  "code":     { /* CodePayload */ },
  "json":     { /* JsonPayload */ },
  "mermaid":  { /* MermaidPayload */ },
  "image":    { /* ImagePayload */ },
  "style":    { /* Record<string, string|number> */ }   // optional per-node style
}
```

### `NodeKind`

`shape` · `text` · `sticky` · `freehand` · `code` · `json` · `mermaid` · `image`
· `group` · `frame`.

(`group` and `frame` are container kinds — they have no dedicated payload; their
children point at them via `parent`. A `frame` can define a slide viewport.)

### Payloads (one per `kind`)

| Payload key | Fields |
|---|---|
| `shape` (`ShapePayload`) | `variant` (`ShapeVariant`, required), `fill?` (string), `stroke?` (string), `sketch?` (bool — hand-drawn/roughjs look; renderers may ignore) |
| `text` (`TextPayload`) | `value` (string, required), `align?` (`left`\|`center`\|`right`), `size?` (number) |
| `sticky` (`StickyPayload`) | `value` (string, required), `color?` (string) |
| `freehand` (`FreehandPayload`) | `points` (`[x, y, pressure?][]` — perfect-freehand input), `color?` (string), `size?` (number) |
| `code` (`CodePayload`) | `value` (string, required), `lang?` (string) |
| `json` (`JsonPayload`) | `value` (string — raw JSON text, rendered as a collapsible tree) |
| `mermaid` (`MermaidPayload`) | `src` (string — the mermaid source, required), `kind?` (string hint: `sequence`\|`flowchart`\|`class`\|`state`\|`er`\|…) |
| `image` (`ImagePayload`) | `attachmentId?` (string) **or** `dataUrl?` (string) |

### `ShapeVariant`

`rect` · `roundrect` · `ellipse` · `diamond` · `triangle` · `cylinder` ·
`parallelogram`.

---

## `CanvasEdge`

A connector between two freeform nodes. (Mermaid diagrams render their own arrows
internally — you usually don't add edges for a single mermaid node.)

```jsonc
{
  "id": "e1",
  "source": "n1",        // source node id
  "target": "n2",        // target node id
  "sourceAnchor": "right",   // optional anchor hint
  "targetAnchor": "left",    // optional anchor hint
  "kind": "arrow",       // optional: arrow | line | dashed
  "label": "calls",      // optional edge label
  "style": { }           // optional Record<string, string|number>
}
```

---

## `Slide` (presentation, PowerPoint-style)

```jsonc
{
  "id": "s1",
  "title": "Walkthrough",     // optional
  "frameNodeId": "f1",        // optional bounding frame node = the slide viewport
  "mermaidNodeId": "m1",      // optional — step THROUGH this mermaid node's messages
  "reveal": [ /* RevealStep[] */ ],
  "notes": "speaker notes"    // optional
}
```

### `RevealStep` (one progressive-disclosure step)

```jsonc
{
  "nodeIds": ["n1", "n2"],          // node ids revealed at this step (fade/translate in)
  "mermaidMessageRange": [0, 3]     // for sequence playback: reveal mermaid messages [from, to] inclusive
}
```

A slide with `reveal: [{ "nodeIds": [...all node ids...] }]` simply shows the
whole scene at once. To walk a sequence diagram message-by-message, set the
slide's `mermaidNodeId` and add one `RevealStep` per range
(`{ "mermaidMessageRange": [0, k] }`).

---

## `AppState`

```jsonc
{ "background": "#ffffff", "grid": true }   // both optional
```

---

## API DTOs (request/response envelopes)

These wrap the `Scene` document on the wire (see `references/endpoints.md` for
methods/paths).

### `CanvasScene` (full row, returned by GET/PUT/POST item routes)

```jsonc
{
  "id": "…",
  "workspace_id": "…",
  "story_id": "…" | null,
  "title": "…",
  "doc_json": "{…}",        // the Scene JSON as a STRING — JSON.parse it
  "thumbnail": "…" | null,
  "created_by": "…",
  "created_at": "…",        // RFC3339
  "updated_at": "…"
}
```

### `CanvasSceneSummary` (list route — no `doc_json`)

```jsonc
{ "id", "workspace_id", "story_id": "…"|null, "title", "thumbnail": "…"|null, "created_at", "updated_at" }
```

### `CreateSceneReq`

```jsonc
{ "title": "…", "doc": { /* Scene — optional; empty doc if omitted */ }, "story_id": "…" /* optional */ }
```

### `UpdateSceneReq` (partial — omitted fields unchanged)

```jsonc
{ "title": "…", "doc": { /* Scene */ }, "thumbnail": "data:image/png;base64,…" }
```

### `AssistReq` / `AssistResult`

```jsonc
// request
{ "prompt": "service A calls B …", "mode": "auto" }   // mode: auto|sequence|flow|uml|nodes (optional)

// response
{
  "mermaid": "sequenceDiagram\n …" | null,   // the common path — a mermaid source
  "nodes":  [ /* Partial<CanvasNode>[] — tier-2 freeform JSON */ ],
  "edges":  [ /* Partial<CanvasEdge>[] */ ],
  "note":   "the agent's one-line prose"
}
```

`assist` returns blocks; it does **not** mutate the scene. Insert the `mermaid`
(as a `mermaid` node) or the `nodes`/`edges` yourself and PUT the scene.
