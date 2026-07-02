# Endpoint reference — Canvas Studio + Discovery Chat

All paths are under the API prefix **`/api/v1`** on the `ottod` daemon
(`OTTO_BASE`, default `http://127.0.0.1:7700`). Every route requires
`Authorization: Bearer $OTTO_API_TOKEN`. Roles are workspace roles
(`Viewer < Editor < Admin`); the required role is noted per route. Errors come
back as `{"code","message"}` with the HTTP status.

Verified against `crates/otto-canvas/src/http.rs`,
`crates/otto-server/src/canvas_assist.rs`, and
`crates/otto-server/src/product_chat.rs`.

---

## Canvas scenes

### `GET /workspaces/{ws}/canvas/scenes` — list (Viewer)
List a workspace's scenes. Returns `CanvasSceneSummary[]` (no `doc_json`).

```bash
otto GET /workspaces/$WID/canvas/scenes
```

### `POST /workspaces/{ws}/canvas/scenes` — create (Editor)
Create a scene. `201 Created` → full `CanvasScene`.

Body (`CreateSceneReq`):
```jsonc
{ "title": "Auth flow", "doc": { /* Scene — optional; empty doc stored if omitted */ }, "story_id": "story_…" /* optional */ }
```
```bash
otto POST /workspaces/$WID/canvas/scenes '{"title":"Auth flow"}'
```

### `GET /canvas/scenes/{id}` — read one (Viewer)
Returns the full `CanvasScene`, including `doc_json` (a STRING — `JSON.parse` it).
Workspace is resolved from the row.

```bash
otto GET /canvas/scenes/$SID
```

### `PUT /canvas/scenes/{id}` — update (Editor)
Partial update — any omitted field is left unchanged. Returns the updated
`CanvasScene`.

Body (`UpdateSceneReq`):
```jsonc
{ "title": "…", "doc": { /* full Scene */ }, "thumbnail": "data:image/png;base64,…" }
```
The read-modify-write pattern for editing a scene:
1. `GET /canvas/scenes/{id}` → `JSON.parse(doc_json)`
2. mutate the `Scene` object (append a node, a slide, …)
3. `PUT /canvas/scenes/{id}` with `{ "doc": <mutated Scene> }`

### `DELETE /canvas/scenes/{id}` — delete (Editor)
`204 No Content`. Permanent.

---

## Session ↔ scene references

Lets a scene show up in an agent session's Canvas panel. A session may
reference many scenes; a scene may be referenced by many sessions. Lives in
`crates/otto-server/src/canvas_refs.rs` (needs `SessionManager`, so it isn't in
`otto-canvas` alongside the CRUD routes above).

### `GET /sessions/{sid}/canvas-refs` — list a session's referenced scenes (Viewer)
Returns `CanvasSceneSummary[]`.

### `POST /sessions/{sid}/canvas-refs` — reference a scene (Editor)
Body: `{ "scene_id": "…" }`. `204 No Content`. Idempotent — referencing an
already-referenced scene is a no-op. The scene must belong to the SAME
workspace as the session, else `404`.

### `DELETE /sessions/{sid}/canvas-refs/{scene_id}` — detach a scene (Editor)
`204 No Content`. Detaching an already-detached scene is a silent no-op. The
scene itself is untouched.

---

## Canvas agent-assist (prompt → diagram blocks)

Both run a single headless agent turn and return `AssistResult`:
`{ mermaid: string|null, d2?: string, excalidraw?: object, format: string, nodes: object[], edges: object[], note: string }`.
The agent returns EITHER a mermaid/D2 source (the common path, per the scene's
`format`) OR an Excalidraw skeleton OR tier-2 `{nodes,edges}` JSON.
`/canvas/scenes/{id}/assist` EDITS the scene's backing file (`canvas.mermaid` /
`canvas.json` / `canvas.d2`) and **commits it** as the scene's new `doc_json` —
the result is already persisted, no separate save needed. `/canvas/assist/preview`
(no scene) returns the blocks WITHOUT persisting anything — you insert + save it
yourself.

`mode` is an optional hint: `auto` (default) | `sequence` | `flow` | `uml` |
`nodes`.

### `POST /canvas/scenes/{id}/assist` — assist on an existing scene (Editor)
Workspace resolved from the scene row.

Body (`AssistReq`):
```jsonc
{ "prompt": "service A calls B, and B does 10 things", "mode": "sequence" }
```
```bash
otto POST /canvas/scenes/$SID/assist '{"prompt":"auth login flow","mode":"sequence"}'
```

### `POST /canvas/assist/preview` — assist with no scene (any member)
For the empty-canvas hero / Discovery-Chat "Open in Canvas". Any authenticated
member may preview (capability enforced upstream).

```bash
otto POST /canvas/assist/preview '{"prompt":"payment capture sequence"}'
```

---

## Discovery Chat (product story)

A conversational discovery agent on a Product **story** (`{sid}`). Each turn
assembles a relevance-bounded context bundle and may propose **actions** that are
**never auto-applied** — you apply them one at a time.

### `POST /product/stories/{sid}/discovery-chats` — start a chat (Editor)
Returns a `DiscoveryChat` (note its `id` = the chat id `cid`).

Body (`CreateChatReq`, optional):
```jsonc
{ "title": "Discovery" }   // optional; defaults to "Discovery"
```
```bash
otto POST /product/stories/$SID/discovery-chats '{"title":"Login discovery"}'
```

### `GET /product/stories/{sid}/discovery-chats` — list a story's chats (Viewer)
Returns `DiscoveryChat[]`.

### `GET /product/discovery-chats/{cid}` — chat + transcript (Viewer)
Returns `ChatDetail`:
```jsonc
{ "chat": { /* DiscoveryChat */ }, "messages": [ /* DiscoveryChatMessage[] */ ] }
```
Each agent `DiscoveryChatMessage` carries `body` (markdown prose) and
`actions_json` (a JSON string of the proposed `actions` array, or null).

### `POST /product/discovery-chats/{cid}/messages` — send one turn (Editor)
Body (`SendMessageReq`):
```jsonc
{ "body": "How should account lockout work?" }
```
Returns a `ChatTurn`:
```jsonc
{ "user_message": { /* DiscoveryChatMessage */ }, "agent_message": { /* … with actions_json? */ } }
```
```bash
otto POST /product/discovery-chats/$CID/messages '{"body":"Draft the login story"}'
```

### `POST /product/discovery-chats/{cid}/archive` — archive (Editor)
Sets status `archived`. Returns the `DiscoveryChat`.

### `POST /product/discovery-chats/{cid}/apply` — apply ONE proposed action (Editor)
Dispatched by the action's `type`. Returns an `ApplyResult`:
```jsonc
{ "story_updated": false, "created_question_ids": [], "created_note_ids": [], "canvas_id": "…"|null }
```

Body (`ApplyReq`) — `{ "action": <one action object> }`. Supported `type`s and
their shapes:

| `type` | Action object | Effect / result field |
|---|---|---|
| `apply_draft` | `{ "type":"apply_draft", "title":"…", "body_md":"full story markdown" }` | updates the story draft → `story_updated: true` |
| `add_questions` | `{ "type":"add_questions", "questions":[{ "text":"…", "rationale":"…", "category":"…" }] }` | creates questions → `created_question_ids` (skips blank `text`; `category` defaults `other`) |
| `add_notes` | `{ "type":"add_notes", "notes":[{ "body":"…" }] }` | creates discovery-section notes → `created_note_ids` (skips blank `body`) |
| `create_canvas` | `{ "type":"create_canvas", "title":"…", "mermaid":"sequenceDiagram\n…" }` *or* `{ …, "nodes":[…], "edges":[…] }` | creates a scene (linked to the story) → `canvas_id`. With `mermaid`, it builds a one-node mermaid scene; otherwise it uses the given `nodes`/`edges`. |

An unknown `type` returns `400 invalid`.

```bash
otto POST /product/discovery-chats/$CID/apply \
  '{"action":{"type":"create_canvas","title":"Login flow","mermaid":"sequenceDiagram\n  U->>S: login"}}'
```

---

## MCP tools note

The first-party MCP server (`ottod mcp-tools`) is read-only, with exactly TWO
named exceptions for Canvas: `canvas_create_scene` and `canvas_update_scene`.
Both post/put to the same HTTP routes above, as the calling session's owner
(same `WorkspaceRole::Editor` gate a human hits) — no elevated access. If
wired, `canvas_list_scenes` / `canvas_get_scene` also appear for inspecting
scenes.

- `canvas_create_scene {title, format?, source?, section?}` → `{scene_id,
  workspace_id}` — creates a scene (`POST /workspaces/{ws}/canvas/scenes`) and
  best-effort references it to the calling session
  (`POST /sessions/{sid}/canvas-refs`).
- `canvas_update_scene {scene_id, source}` → `{ok: true, format}` — replaces a
  scene's diagram source, preserving its `format`/`sketch`.

Deleting a scene, every discovery-chat mutation, and every field NOT covered
by the two tools above still go through the HTTP routes with `OTTO_API_TOKEN` —
there is no MCP tool for those.
