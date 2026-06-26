# Canvas

A file-backed visual canvas built into Otto: describe a diagram in plain English
and an **agent draws it for you** by editing a per-scene source file, while the
board re-renders live as the file changes. Each scene is one of two modes —
an **Excalidraw board** (fully editable shapes; the agent writes `canvas.json`)
or a **Mermaid diagram** (auto-rendered flowchart / sequence / class / state /
ER; the agent writes `canvas.mermaid`). The conversation with the agent is the
agent's **own live shell**, embedded right in the page (the same `<Terminal>` the
Agents view uses), so refining a diagram is just chatting; you never have to hand-
compute coordinates or learn Mermaid syntax (though you can edit either by hand).

> **Where this lives in the code.** CRUD crate: `crates/otto-canvas/` (`http.rs`,
> `types.rs`, `lib.rs`). Agent-assist (needs the orchestrator): `crates/otto-server/src/canvas_assist.rs`.
> Persistence: `crates/otto-state/src/canvas.rs` (`CanvasRepo`), migrations
> `0072_canvas_scenes.sql` / `0074_session_links.sql` / `0075_canvas_scene_meta.sql`.
> RBAC: `Feature::Canvas` (`crates/otto-core/src/domain.rs`), gated in
> `crates/otto-server/src/policy.rs`. UI: `ui/src/modules/canvas/` + store
> `ui/src/lib/stores/canvas.svelte.ts`. The REST contract is in
> `docs/contracts/api.md` (#102–#108).

---

## 1. Overview

| | |
|---|---|
| **What it is** | A per-scene, file-backed canvas an agent draws on by editing a source file; the board renders that file live. |
| **Two modes** | **Excalidraw** (editable shapes, source = `canvas.json`) · **Mermaid** (auto-rendered diagrams, source = `canvas.mermaid`). Chosen at creation. |
| **Diagram types (Mermaid)** | flowchart (`TD`/`LR`), `sequenceDiagram`, `classDiagram` (UML), `erDiagram`, `stateDiagram-v2` — Mermaid's own renderer, fully offline (bundled, lazy-loaded). |
| **How you drive it** | The **Assistant** panel — the agent's embedded live shell + a composer. You describe a change; the agent edits the file in place and the board re-renders. |
| **Edit by hand too** | Excalidraw shapes are directly editable; Mermaid has a **Code** panel to edit the `.mermaid` source — both autosave to the same file the agent edits. |
| **Storage** | One row per scene in SQLite (`canvas_scenes.doc_json`); the source file lives under the daemon's data dir (`<data>/canvas/<scene_id>/`). |
| **Scope** | Scenes are created in a workspace; the Canvas page lists **all of your scenes across every workspace** (Canvas is a workspace-independent tool). Optionally linked to a product story. |
| **Where it lives** | The **Canvas** top-level nav module. RBAC feature key `canvas` (View to read, Edit to mutate / Ask AI). |
| **Daemon** | `ottod` on `127.0.0.1:7700`; routes under `/api/v1`. |

### Where each piece lives

| Concern | Source |
|---|---|
| Scene CRUD router + DTOs | `crates/otto-canvas/src/http.rs`, `types.rs` (`CanvasCtx`, `router`, `CreateSceneReq`, `UpdateSceneReq`, `empty_doc`) |
| Agent-assist (file-backed draw) | `crates/otto-server/src/canvas_assist.rs` (`assist_scene`, `assist_preview`) |
| One managed agent turn (create/resume) | `crates/otto-server/src/agent_session.rs` (`run_session_turn`) |
| Persistence + repo | `crates/otto-state/src/canvas.rs` (`CanvasScene`, `CanvasSceneSummary`, `CanvasRepo`) |
| Server wiring (router mount + assist routes) | `crates/otto-server/src/modules.rs` |
| RBAC policy | `crates/otto-server/src/policy.rs` (`/canvas/` + `/workspaces/{ws}/canvas/` ⇒ `Require(Canvas, …)`) |
| Live events | `crates/otto-core/src/event.rs` (`CanvasUpdated`, `CanvasSessionStarted`) |
| UI page + components | `ui/src/modules/canvas/{CanvasPage,SceneList,ExcalidrawCanvas,MermaidCanvas,ConversationPanel}.svelte` |
| Mermaid render bridge | `ui/src/modules/canvas/mermaid.ts` · Excalidraw element builder: `excalidraw-build.ts` |
| Store + live-event bus | `ui/src/lib/stores/canvas.svelte.ts` · `ui/src/lib/events.svelte.ts` (`canvasDocBus`) |
| API client types | `ui/src/modules/canvas/types.ts` (`CanvasScene`, `CanvasDoc`, `AssistResult`, …) |
| Agent skill | `crates/otto-skills/assets/skills/development/otto-canvas/` |
| API contract | `docs/contracts/api.md` #102–#108 |

> **Note on legacy code.** The tree still carries an earlier *node-graph* design
> (`CanvasFlow.svelte`, `Toolbar.svelte`, `PresentMode.svelte`, `ToolRail.svelte`,
> `nodes/*`, `scene.ts`, `templates.ts`, and the rich `Scene` schema in
> `types.ts`). Those components are **not mounted** by the current `CanvasPage`
> — the shipping canvas is the file-backed Excalidraw/Mermaid pair documented
> here. A few helpers from that era (`parseScene`, `emptyScene`) are still used as
> fallbacks by the store. See **Capabilities & limitations** for what this means
> for "Present mode" and JSON export.

---

## 2. The file-backed model

A scene's persisted `doc_json` is a small, opaque document the server and the UI
share (the Rust side never parses its meaning — see `canvas_assist.rs::build_doc`):

```jsonc
{
  "type": "otto-canvas",
  "version": 1,
  "format": "mermaid",          // "mermaid" (default) | "excalidraw"
  "source": "flowchart TD\n …"  // the Mermaid text, or the Excalidraw scene JSON
}
```

- **`format`** decides the render path and the on-disk file name:
  - `mermaid` → `canvas.mermaid` (the default; the agent edits *text*, so layout
    is automatic and clean).
  - `excalidraw` → `canvas.json` (a full Excalidraw scene).
- **`source`** is the literal file content. When an "Ask AI" turn runs, the server
  materializes `source` into `<data_dir>/canvas/<scene_id>/<file>`, lets the agent
  edit that file in its working directory, reads it back, and commits it as the new
  `source`. Because the agent works in the **same per-scene directory across turns**
  (resuming the same session), follow-ups *refine* the existing diagram instead of
  regenerating it.

A brand-new scene starts from a base (`flowchart TD\n` for Mermaid, an empty
`{type:"excalidraw",…,"elements":[]}` for Excalidraw). If a scene is created via
the raw API with no `doc`, `otto-canvas` stores a minimal legacy scene
(`empty_doc`); the UI always sends a proper file-backed `doc` on create.

---

## 3. Creating & managing scenes

Open **Canvas** in the sidebar. The left rail (`SceneList.svelte`) lists your
scenes; the right side is the board (or a "Start a new canvas" hero when nothing
is open).

**Create.** The hero and the **New scene** menu both offer the two modes:

- **Excalidraw board** — *"Editable shapes — draw & arrange by hand too."*
- **Mermaid diagram** — *"Auto-rendered flowchart / sequence / class — rich & clean."*

Creating posts to `POST /workspaces/{ws}/canvas/scenes` in the **current**
workspace with a blank doc of the chosen format, then opens it. New scenes are
titled "Untitled canvas" until renamed.

**Organize.** The list supports:

- **Search** — filters by title or section.
- **Sections** — a folder path (e.g. `Platform/Staging`) groups scenes into
  collapsible sections (root/ungrouped first, then sections alphabetically). Set
  one via a scene's **folder** action (*Move to section*); an empty value clears it.
- **Rename** — the row's edit action or a **double-click** (prompts for a new title).
- **Delete** — the trash action, with a confirm ("This can't be undone").

Each row shows a relative "updated" time; the list is sorted newest-updated-first.
Section + provider + title edits all go through `PUT /canvas/scenes/{id}`
(partial — only the fields you send change, server-side `COALESCE`).

**Autosave & title.** Manual board edits autosave (debounced; see §5). The scene
title is editable; the store tracks a `saving`/`dirty`/`savedAt` state used by the
header indicator.

---

## 4. Ask AI — the Assistant

Click **Ask AI** to open the **Assistant** panel (`ConversationPanel.svelte`).
It has three parts:

1. **A provider picker** — which agent draws this scene (`claude`, `codex`, …;
   `shell` is excluded because it isn't an agent). The choice persists on the
   scene (`PUT …/{id}` with `provider`), so each scene remembers its drawer.
2. **The agent's live shell** — an embedded `<Terminal>` bound to the scene's
   managed session (`/ws/term/{session_id}`). You watch the agent think and edit
   the file in real time. It appears the moment the first turn starts (the server
   emits `CanvasSessionStarted` at turn start so the panel can attach immediately).
3. **A composer** — type a request and press **Enter** (Shift+Enter for a newline).

### What a turn does (`assist_scene`)

`POST /api/v1/canvas/scenes/{id}/assist` (workspace **Editor**) runs **one** agent
turn:

1. Resolve the scene's current `source` + `format`, and **materialize** it into
   `<data_dir>/canvas/<scene_id>/<canvas.mermaid|canvas.json>`. The directory is
   marked trusted (`otto_sessions::trust::ensure_trusted`) so the PTY doesn't stall
   on a first-run trust prompt.
2. Start a background **file poll** (every ~900 ms): each time the file changes
   while the agent writes, the server broadcasts `CanvasUpdated`, so the diagram
   visibly *"draws itself"* mid-turn.
3. Run the turn via `run_session_turn` — **resuming** the scene's session
   (`scene.session_id`) when it exists, so the agent keeps the prior diagram and
   conversation in context. The first turn creates the session and stores its id
   (`CanvasRepo::set_session`).
4. Read the file back and commit it: the **edited file wins**; if the agent printed
   a fenced ```` ```mermaid ````/```` ```json ```` block instead of editing the
   file (or in the offline E2E stub), that block is used as a fallback and written
   back so the next resumed turn sees it; otherwise the prior source is kept
   (`resolve_source`).
5. Persist the new `doc_json` and broadcast a final `CanvasUpdated`. The HTTP
   response is an `AssistResult` so the UI can render immediately even before the
   event arrives.

`AssistResult` = `{ excalidraw?, mermaid?, format, nodes[], edges[], note }`.
`note` is the agent's one-line description (or an error explanation when nothing
was drawn).

### The prompts (mode hint)

`build_assist_prompt` emits an `OTTO_TASK: canvas_assist` sentinel (which routes
the deterministic E2E stub) and tailors the instructions to the format:

- **Mermaid** — "edit the MERMAID file in place"; pick the best diagram type for
  the request; presentation-grade style (short emoji-prefixed labels, rhombus
  decisions with labelled yes/no edges, `subgraph` lanes, and `classDef` colour
  coding by role). The file must always hold one complete, valid Mermaid diagram.
- **Excalidraw** — "edit `canvas.json`"; write the **complete** diagram each time
  using a **simplified element form** (shapes/arrows/text with no Excalidraw
  internals; arrows reference node ids and are routed by the app). The UI
  (`excalidraw-build.ts`) expands that simplified form into a real Excalidraw scene
  — binding labels and routing arrows — so the agent never hand-computes geometry.

The request body also accepts a `mode` hint (`auto` | `sequence` | `flow` | `uml`
| `nodes`); the Mermaid board sends `flow`.

---

## 5. Editing by hand

The agent and the user share **one file** — anything you draw or type is saved
back to the same `source` the agent edits next.

### Excalidraw board (`ExcalidrawCanvas.svelte`)

Excalidraw is React, mounted React-in-Svelte (a host div + `createRoot`). You get
the full Excalidraw editor (its own toolbar, shapes, arrows, text, selection,
its native export menu). Behavior:

- **source → board.** The stored scene is parsed and normalized: the agent's
  *simplified* form is **built** with controlled geometry and centred labels
  (`buildExcalidrawElements`), while a *full* saved scene is rescued (re-routing
  id-only arrows and re-centring any labels that collapsed to the origin) and run
  through Excalidraw's `restoreElements`.
- **board → source.** Any manual edit autosaves the **full** Excalidraw scene back
  to `canvas.json` (debounced ~700 ms) via `PUT /canvas/scenes/{id}`. Saves are
  carefully guarded against a scene switch landing mid-PUT, so an Excalidraw doc is
  never written into a Mermaid scene.
- **Live agent edits** arrive over `canvas_updated` and reload in place.
- On a phone the board is read-only (`viewModeEnabled`); tablets/desktops edit.

> The Excalidraw font/asset bundle is loaded from a CDN
> (`EXCALIDRAW_ASSET_PATH = unpkg.com/@excalidraw/excalidraw@0.18.1/…`) on first
> use, so an Excalidraw board needs network access the first time it mounts.

### Mermaid board (`MermaidCanvas.svelte`)

The Mermaid board renders the `.mermaid` source to SVG with Mermaid's own renderer
and lets you pan/zoom it. You edit it two ways, both writing `canvas.mermaid`:

- **Ask AI** (the Assistant) — described above.
- **Code** — a toggleable side panel with a CodeMirror editor over the raw Mermaid
  source; edits debounce-save (~500 ms) and re-render live.

Controls: **drag** to pan, **scroll** to zoom, a zoom bar with **fit-to-screen**
and a **Download SVG** button. If the source is actually Excalidraw JSON (a
mislabelled scene), the board shows a clear "this holds Excalidraw content" message
instead of a raw parse error. A render error keeps the last good SVG on screen and
shows the parser message in a banner.

---

## 6. Live updates (WebSocket)

Two events flow over `WS /ws/events` (defined in `crates/otto-core/src/event.rs`,
dispatched in `ui/src/lib/events.svelte.ts`):

| Event (`type`) | Payload | Effect |
|---|---|---|
| `canvas_updated` | `{ workspace_id, scene_id, doc }` | Emitted **live** per file change while an agent edits, and once more with the committed result. The open editor re-renders the matching scene in place (no refetch) via `canvasDocBus`. |
| `canvas_session_started` | `{ workspace_id, scene_id, session_id }` | Emitted at the **start** of an Ask-AI turn so the Assistant panel attaches the agent's shell immediately (sets the open scene's `session_id`). |

Both are workspace-scoped (carry `workspace_id`) and reach workspace members per
the standard event scoping. (These two variants postdate the WS "full event
catalog" prose in `docs/contracts/ws.md`; `event.rs` is authoritative.)

---

## 7. Discovery-Chat bridge

Canvas is also reachable from the Product page's **Discovery Chat**: when that
agent proposes a `create_canvas` action, applying it creates a scene **linked to
the story** (`story_id`) and deep-links into Canvas (the Discovery action sets
`canvas.pendingOpenId` and navigates; `CanvasPage` consumes it on mount and opens
the scene).

There is also a scene-less assist endpoint, `POST /api/v1/canvas/assist/preview`,
used by the empty-canvas hero / Discovery bridge: it requires a `workspace_id` in
the body, runs a **throwaway** agent session (killed immediately after), and
returns a Mermaid `AssistResult` — there's no scene to own a file, so nothing is
persisted. See **[Discovery Chat](./discovery-chat.md)** and
**[Product](./product.md)**.

---

## 8. REST / WS surface

`docs/contracts/api.md` is authoritative; `ui/src/modules/canvas/types.ts` mirrors
the DTOs. Reads require workspace **Viewer**; mutations and Ask-AI require
workspace **Editor**. The `Feature::Canvas` axis is enforced upstream by the
deny-by-default policy middleware (`/canvas/` and `/workspaces/{ws}/canvas/` ⇒
`Require(Canvas, View)` on GET, `Edit` otherwise). Item routes resolve the
workspace from the scene row.

| # | Method & path | Auth | Purpose |
|---|---|---|---|
| 102 | `GET /api/v1/workspaces/{ws}/canvas/scenes` | ws viewer | list a workspace's scenes (summaries) |
| — | `GET /api/v1/canvas/scenes` | Canvas View | list **all of the caller's** scenes across workspaces (what the page uses) |
| 103 | `POST /api/v1/workspaces/{ws}/canvas/scenes` | ws editor | create a scene (`{title, doc?, story_id?, provider?, section?}`) → 201 |
| 104 | `GET /api/v1/canvas/scenes/{id}` | ws viewer | full scene incl. `doc_json`, `session_id`, `provider`, `section` |
| 105 | `PUT /api/v1/canvas/scenes/{id}` | ws editor | partial update (`{title?, doc?, thumbnail?, provider?, section?, story_id?}`; omitted fields unchanged) |
| 106 | `DELETE /api/v1/canvas/scenes/{id}` | ws editor | delete → 204 |
| 107 | `POST /api/v1/canvas/scenes/{id}/assist` | ws editor | one file-backed agent turn → `AssistResult`; **commits** the scene + broadcasts `CanvasUpdated` |
| 108 | `POST /api/v1/canvas/assist/preview` | Canvas Edit | scene-less draw (`{prompt, mode?, workspace_id}`); throwaway session → `AssistResult` |

> **Contract drift to be aware of.** The api.md row for #107 still reads "one agent
> turn; does not mutate the scene" and lists the PUT body as `{title?, doc?,
> thumbnail?}`. The shipping code (`canvas_assist.rs`) **does** commit the result to
> `doc_json` and broadcast it, and `UpdateSceneReq` additionally accepts `provider`,
> `section`, and `story_id` — this guide documents the code's real behavior. (Those
> entries predate the file-backed redesign; the contract should be reconciled.)

`POST /api/v1/product/stories/{sid}/linked-canvases` is the related Product route
that lists the Canvas scenes linked to a story.

---

## 9. Capabilities & limitations

- **Two modes only**, chosen at creation: Excalidraw (`canvas.json`) and Mermaid
  (`canvas.mermaid`). There's no in-place conversion between them — a Mermaid scene
  stays Mermaid; create a new Excalidraw scene to draw editable shapes.
- **Agent-first.** The intended flow is: describe → the agent edits the file → the
  board re-renders. Hand-editing is supported (Excalidraw shapes; the Mermaid
  **Code** panel) but there is no separate native shape palette for Mermaid scenes.
- **Mermaid is fully offline** (the `mermaid` package is bundled and lazy-loaded,
  no CDN). **Excalidraw loads its asset/font bundle from the unpkg CDN** on first
  mount.
- **No Present mode in the current canvas.** Present mode (PowerPoint-style slide
  stepping) and the top **Toolbar** (Undo/Redo, **Export JSON**, Present) belong to
  the older node-graph design and are **not wired** into the shipping file-backed
  page. What ships today: the Mermaid board's **Download SVG**, Excalidraw's own
  native export menu, and per-board pan/zoom/fit. (The store still has
  snapshot undo/redo, but no mounted UI invokes it.)
- **Live "draws itself"** preview is a best-effort file poll (~900 ms), not a
  byte-stream — large diagrams update in visible steps.
- **One open scene at a time**; switching scenes remounts the board (keyed by id)
  so each loads its own source. Saves are guarded against scene-switch races.
- **Provider** is per-scene and persisted; only real agents are offered (no
  `shell`). Resume requires a provider that supports `--resume` (claude); other
  providers run fresh each turn.
- **Mobile**: the Excalidraw board is read-only on a phone; the scene list
  collapses once a board is open. Tablets keep editing.

---

## 10. Security & guards

- **RBAC.** Every route is behind `Feature::Canvas` (deny-by-default policy) **and**
  a per-scene workspace-role check (Viewer to read, Editor to mutate / Ask AI).
  Item routes resolve the owning workspace from the scene row, so you can't reach a
  scene in a workspace you lack a role in.
- **Loopback by default.** `ottod` listens on `127.0.0.1`; the source file lives on
  the **daemon host's** disk (`<data_dir>/canvas/<scene_id>/`), not the browser.
- **Agent working directory.** An Ask-AI turn runs the agent in the scene's own
  per-scene directory, which is marked trusted so the PTY doesn't block on a trust
  prompt. The throwaway preview turn runs in the workspace root and its session is
  killed immediately after.
- **No secrets in the doc.** A scene stores only its title, opaque `doc_json`
  source, and metadata; it carries no credentials.
- **Resumed sessions** are the scene's own managed Otto session (`session_id`),
  subject to the same session isolation/RBAC as any agent session.

> Roles and per-session isolation are documented in
> **[Multi-user RBAC](../MULTI-USER-RBAC.md)**.

---

## 11. Troubleshooting

- **The Mermaid board shows "Diagram error: …".** The `source` isn't valid Mermaid;
  the banner is the parser's own message and the last good render stays on screen.
  Fix it in the **Code** panel, or ask the agent to correct it.
- **"This canvas holds Excalidraw content."** A scene labelled `mermaid` actually
  contains an Excalidraw scene JSON. Create a new **Excalidraw** canvas to edit
  those shapes.
- **Ask AI replies but nothing is drawn.** The agent produced no parseable
  diagram/file edit; the `note` is shown as a toast ("Nothing to draw"). Try a more
  specific prompt or a mode hint, and watch the embedded shell for what the agent
  did.
- **The Assistant shell stays empty.** The session attaches when the turn starts
  (`canvas_session_started`). If it never appears, the turn likely failed to spawn —
  check the toast, and that the chosen provider is installed.
- **My hand edits "disappeared" after an agent turn.** The committed source is the
  file the agent left; an agent turn that rewrites the whole file replaces hand
  layout. Use the Mermaid **Code** panel / Excalidraw edits *between* turns, and ask
  the agent to "refine" rather than "redraw".
- **The Excalidraw board is blank / fonts look wrong.** Its asset bundle loads from
  the unpkg CDN on first mount; confirm the daemon host has network access, or that
  `EXCALIDRAW_ASSET_PATH` is reachable.
- **A scene I created in another workspace isn't in the list.** It is — the page
  lists *your* scenes across all workspaces (`GET /canvas/scenes`); use search.

---

## 12. Related docs

- **[Discovery Chat](./discovery-chat.md)** — the Product-page agent that can
  propose a diagram and open it straight in Canvas (`create_canvas` action).
- **[Product](./product.md)** — the product-owner workflow Canvas scenes can link
  into (via `story_id`).
- **[Agent sessions](./agent-sessions.md)** — the managed-session/PTY machinery
  the Assistant's embedded shell reuses.
- **[Usage & cost](./usage-and-cost.md)** — token/cost tracking, including the
  agent turns Ask-AI spends.
- **[Multi-user RBAC](../MULTI-USER-RBAC.md)** — roles and per-session isolation.
- **API contract**: `docs/contracts/api.md` #102–#108.
