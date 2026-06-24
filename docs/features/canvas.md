# Canvas Studio

Canvas Studio is an Excalidraw-style visual canvas built into Otto: sketch
mockups, draw **UML / sequence / flowchart / class / ER** diagrams, drop
**code blocks**, **JSON blocks**, sticky notes, shapes, connectors and freehand —
all stored as **one portable JSON document** (`Scene`). It includes a
PowerPoint-style **Present mode** that animates a scene step-by-step (including
sequence-diagram message playback), and an **Ask-AI** assistant that turns a
prompt into diagram blocks ("service A calls B; B does 10 things" → a diagram
with all ten steps drawn out).

This guide documents what the code in `crates/otto-canvas/`,
`crates/otto-server/src/canvas_assist.rs`, and `ui/src/modules/canvas/` does.

> Related: **[Discovery Chat](./discovery-chat.md)** — the product-page agent can
> propose a diagram and open it straight in Canvas Studio.

---

## 1. Summary

| | |
|---|---|
| **What it is** | A node-graph + freeform visual canvas; scenes are portable JSON. |
| **Block types** | Shape (rect/ellipse/diamond/…), text, sticky, connector, code (syntax-highlighted), JSON (tree), Mermaid diagram, image, frame, freehand. |
| **Diagrams** | Mermaid (sequence / flowchart / class(UML) / state / ER), rendered fully offline (lazy-loaded). |
| **Present mode** | Slides + reveal steps with fade/translate transitions; sequence diagrams play message-by-message (auto or manual, like PowerPoint). |
| **Ask AI** | One agent turn returns a Mermaid diagram or `{nodes,edges}` JSON; the blocks are inserted into the scene. |
| **Storage** | One row per scene in SQLite (`canvas_scenes.doc_json`); workspace-scoped, optionally linked to a product story. |
| **Where it lives** | The **Canvas** section (top-level nav). RBAC feature key `canvas` (View/Edit). |
| **Daemon** | `ottod` on `127.0.0.1:7700`; routes under `/api/v1` (contract #102–#108). |

---

## 2. Where everything lives

| Layer | Path |
|---|---|
| Scene CRUD crate | `crates/otto-canvas/` (`types.rs`, `http.rs` — `CanvasCtx`, router) |
| Agent assist | `crates/otto-server/src/canvas_assist.rs` (needs the orchestrator) |
| Persistence | `crates/otto-state/src/canvas.rs` (`CanvasRepo`); migration `0072_canvas_scenes.sql` |
| RBAC | `Feature::Canvas` (`crates/otto-core/src/domain.rs`); policy `crates/otto-server/src/policy.rs` (`/canvas/` family) |
| Server wiring | `crates/otto-server/src/modules.rs` (router mount + `CanvasCtx` impl) |
| UI module | `ui/src/modules/canvas/` (`CanvasPage`, `CanvasEditor`, `nodes/*`, `Toolbar`, `Inspector`, `PresentMode`, `AiPromptPill`, `templates.ts`, `scene.ts`, `mermaid.ts`) |
| UI store | `ui/src/lib/stores/canvas.svelte.ts` |
| Types (Scene schema) | `ui/src/modules/canvas/types.ts` |
| Agent skill | `crates/otto-skills/assets/skills/development/otto-canvas/` |
| API contract | `docs/contracts/api.md` #102–#108 |

---

## 3. The Scene document

A scene is a single JSON object persisted as `doc_json`:

```jsonc
{
  "schema": 1,
  "title": "Checkout flow",
  "nodes": [ { "id": "n1", "kind": "mermaid", "x": 80, "y": 80, "w": 520, "h": 360,
               "mermaid": { "src": "sequenceDiagram\n A->>B: pay" } } ],
  "edges": [ { "id": "e1", "source": "n1", "target": "n2", "kind": "arrow" } ],
  "slides": [ { "id": "s1", "title": "Step 1", "reveal": [ { "nodeIds": ["n1"] } ] } ],
  "appState": { "grid": true }
}
```

Node kinds: `shape | text | sticky | freehand | code | json | mermaid | image |
group | frame`. The Rust side treats `doc_json` as opaque text; the rich schema
and rendering live in `ui/src/modules/canvas/types.ts`.

---

## 4. Using it

1. Open **Canvas** in the sidebar and pick (or create) a scene.
2. **Draw**: pick a tool from the left rail (Select, Sticky, Text, Shape,
   Connector, then Mermaid / Code / JSON / Image / Frame / Freehand) and click
   the canvas. Drag to move, connect handles to draw arrows, select to edit
   properties in the right inspector. Undo/redo with ⌘Z / ⌘⇧Z. Scenes **autosave**
   (the top bar shows "Saving…/Saved").
3. **Ask AI**: click **Ask AI**, describe what you want ("service A calls B; B
   does 10 things"), pick a mode hint (Auto/Sequence/Flow/UML), press ⌘↵. The
   generated blocks land near the viewport center.
4. **Present**: click **Present** to step through the scene's slides. For a
   sequence diagram, each step reveals the next message. ←/→/Space/Esc control it.
5. **Export**: download the scene as JSON from the top bar.

---

## 5. Capabilities & limits

- **Offline**: Mermaid and code highlighting run fully client-side (no CDN).
  Mermaid is lazy-loaded only when a diagram node or Present mode is used.
- **v1 ships**: scene CRUD, the core block types, connectors, inspector,
  autosave, snapshot undo/redo, Present mode (slide/node reveal + best-effort
  sequence stepping), Ask-AI assist, JSON export.
- **Deferred (stretch)**: a freehand *drawing tool* (the `freehand` node type
  exists and renders, but a polished pen tool is not wired), roughjs hand-drawn
  fills, image upload, PNG/SVG raster export, multi-user presence, and converting
  Mermaid diagrams into individually-editable native nodes (they render as a
  single diagram node).
- **Mobile**: on a phone the editor is read-only (rail + inspector hidden); you
  can still **Present** and **Ask AI**. Tablets keep editing.

---

## 6. Troubleshooting

- **A Mermaid node shows an error** — the `src` is invalid Mermaid; the inline
  error is the parser message. Fix the syntax in the inspector.
- **Ask AI returns text but no blocks** — the agent didn't produce a parseable
  diagram; its note is shown as a toast. Try a more specific prompt or a mode hint.
- **Present mode shows the whole diagram instead of stepping** — sequence
  stepping is best-effort over Mermaid's SVG; if it can't find message elements it
  falls back to revealing the whole diagram (still a valid slide).
