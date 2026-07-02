# Canvas D2 + Session References + Product UX Refresh — Design

Date: 2026-07-02 · Branch: `feat/canvas-d2-product-polish` · Status: approved (autonomous run)

## Goals (requirements)

- **R1** — Canvas supports **D2** (terrastruct/d2) as a third scene format alongside Mermaid and Excalidraw.
- **R2** — Canvas quality upgrades ("top notch"): better export, view controls, error resilience, scene management.
- **R3** — Canvas interacts cleanly with **Product** (linked canvases show format/freshness; create-linked flow works for all formats).
- **R4** — Canvas is **accessible from every session**, and sessions can hold **references** to canvas scenes; agents inside sessions can create/edit scenes.
- **R5** — Product page **simplified/beautified** while **maintaining all functionality** (13 sub-views stay).
- **R6** — Full E2E coverage for the above; all existing gates stay green.
- **R7/R8** — Worktree → branch → verify → merge to local main → rebuild + reinstall the running app.

Explicitly out of scope (YAGNI): D2 in Mockups, splitting the 3,173-line `OverviewTab` god-component (noted as future work), WorkRef canvas attribution, canvas thumbnails, TALA layout engine.

## 1. D2 canvas mode (R1)

### Rendering approach — decision

**Chosen: client-side WASM via `@terrastruct/d2` (0.1.33), lazy-loaded.** Same architecture as Mermaid: the UI compiles+renders `source → SVG` locally.

Alternatives rejected:
- *Bundle the d2 Go binary as a Tauri sidecar* — ~50MB, universal-binary + codesign complexity, server round-trip for every render.
- *Require user-installed d2 CLI* — not self-contained; breaks on fresh machines.

`@terrastruct/d2` is a zero-dependency official package; the browser build is one self-contained 7.8MB file (WASM inlined, web-worker driven). It MUST be loaded with a dynamic `import()` from a dedicated module so Vite splits it into its own chunk, fetched only when a D2 scene is first opened.

### Backend

Format stays a validated string (no Rust enum exists today; keep the idiom):
- `crates/otto-server/src/canvas_assist.rs`
  - `doc_format()`: accept `"d2"` (default remains `mermaid`).
  - `file_name()`: `d2 → "canvas.d2"`.
  - `build_assist_prompt()`: third variant — "EDIT the D2 file `canvas.d2` IN PLACE" with a styling guide (containers, `direction`, edge labels, `classes`, `style.fill/stroke`, `shape: sql_table/sequence_diagram`, near/grid hints). Keeps the `OTTO_TASK: canvas_assist` sentinel.
- `crates/otto-orchestrator/src/e2e_stub.rs`: extend the `canvas_assist` branch to write a deterministic `canvas.d2` when the prompt targets D2 (keyed off the file name in the prompt, same as mermaid/json today).
- `crates/otto-canvas`: no change needed for CRUD (doc_json is opaque). Add `"d2"` wherever format strings are validated/defaulted if present.
- No migration: `canvas_scenes.doc_json.format` is opaque JSON.

### UI

- `ui/src/modules/canvas/d2.ts` — lazy singleton:
  `renderD2(src, {sketch, dark}) → {svg} | {error}`; maps app scheme to D2 themes (light `themeID: 0`, dark `themeID: 200` Dark Mauve — verify exact ID at impl); compile errors returned structured (message + line if available), never thrown to the component.
- `ui/src/modules/canvas/D2Canvas.svelte` — modeled 1:1 on `MermaidCanvas.svelte`: pan/zoom surface, Code panel (CodeMirror; small `StreamLanguage` for D2 keywords/strings/comments, same pattern as the redis language), autosave to the same `PUT` path, live `canvasDocBus` ingestion, **sketch-mode toggle** (persisted in doc as `sketch?: boolean`), SVG + PNG export, keep-last-good-render on parse error with an inline error strip.
- `ui/src/modules/canvas/types.ts`: `CanvasFormat = 'mermaid' | 'excalidraw' | 'd2'`; `CanvasDoc.sketch?: boolean`.
- `CanvasPage.svelte`: render branch for `format === 'd2'`; hero gets a third card; `SceneList.svelte` "New scene" menu gets D2.
- `ui/src/lib/stores/canvas.svelte.ts`: default D2 source template for new scenes (`createBlank('d2')`).
- `ui/package.json`: add `@terrastruct/d2@^0.1.33`.

### Skill

`crates/otto-skills/assets/skills/development/otto-canvas/` → v5: document the third mode (`canvas.d2`), add `references/d2-cheatsheet.md`, extend `scripts/canvas.mjs` with `add-d2`, update endpoints reference (formats + new session-ref endpoints from §3).

## 2. Canvas quality upgrades (R2)

All within the existing components; no architecture change:

1. **PNG export** for Mermaid + D2 (rasterize the SVG via offscreen `<canvas>`, 2x scale); SVG export for D2 (Mermaid already has it).
2. **View controls**: fit-to-view, zoom-in/out, 100% reset, zoom % readout — one small shared control cluster used by Mermaid + D2 surfaces.
3. **Keep-last-good rendering**: parse/compile errors show a dismissible inline strip; the previous SVG stays (Mermaid gets the same treatment as D2 if missing).
4. **Scene search** — filter box in `SceneList` (title + section match, client-side).
5. **Duplicate scene** — new action in the scene row menu (client-side: `POST` create with copied doc + " (copy)" title).
6. **Copy source** button in the Code panels.
7. **Docs/contract hygiene**: `docs/contracts/ws.md` gains the missing `canvas_updated`/`canvas_session_started` (+ mockup, + new §3 events) entries; `docs/contracts/api.md` canvas section updated (formats incl. `d2`, `provider`/`section`/`story_id` on update, assist-commits-scene note).

## 3. Canvas references in sessions (R4)

### Data model — migration `0092_canvas_scene_refs.sql` (provisional; renumber at merge)

```sql
CREATE TABLE canvas_scene_refs (
  scene_id     TEXT NOT NULL,
  session_id   TEXT NOT NULL,
  workspace_id TEXT NOT NULL,
  created_by   TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  PRIMARY KEY (scene_id, session_id)
);
CREATE INDEX idx_canvas_refs_session ON canvas_scene_refs(session_id);
```

Many-to-many: a session can reference many scenes; a scene can appear in many sessions. Distinct from `canvas_scenes.session_id` (which is the hidden assist-conversation session).

Repo methods in `crates/otto-state/src/canvas.rs`: `add_ref`, `remove_ref`, `list_refs_for_session` (returns `CanvasSceneSummary` join), `list_ref_sessions_for_scene`. Deleting a scene cascades ref deletion (explicit `DELETE` in `CanvasRepo::delete`).

### API (relative to `/api/v1`)

- `GET /sessions/{sid}/canvas-refs` → `[CanvasSceneSummary]` (Canvas View)
- `POST /sessions/{sid}/canvas-refs` `{scene_id}` (Canvas Edit)
- `DELETE /sessions/{sid}/canvas-refs/{scene_id}` (Canvas Edit)

Implemented in `crates/otto-canvas/src/http.rs` (owns the repo); policy: paths containing `/canvas-refs` map to `Feature::Canvas` in `policy.rs`. Caller must be a member of the session's workspace (resolve session → workspace, reuse the existing role checks).

New event `Event::CanvasRefsChanged { workspace_id, session_id }` broadcast on add/remove → UI refreshes the panel. Contract: `api.md` new rows (next free numbers), `ws.md` event entry, `ui/src/lib/api/types.ts` + `ui/src/modules/canvas/types.ts` mirrors.

### MCP write tools (agents in sessions create/edit scenes)

`crates/ottod/src/mcp_tools.rs`:
- `canvas_create_scene {title, format: mermaid|excalidraw|d2, source, section?}` → POST create (doc assembled server-side), then **auto-ref to the calling session** (`OTTO_SESSION_ID` env is already present) via the refs endpoint. Returns `{scene_id}`.
- `canvas_update_scene {scene_id, source}` → GET scene, PUT doc with new source (format preserved). Returns `{ok, format}`.
- Catalog text for the two read tools updated (writes now exist). Both write tools call the same governed HTTP routes as the session's owner, so RBAC applies; audited like the rest.
- Unit tests beside the existing `read_route()` tests.

Effect: any opted-in agent session can draw — create a D2/Mermaid scene, and it appears live in the session's Canvas panel (WS `canvas_updated` + `canvas_refs_changed`).

### UI surfacing

- **RightPanel "Canvas" tab** (`ui/src/shell/RightPanel.svelte` tabs array + body switch; `RightTab` union in `ui/src/lib/stores/ui.svelte.ts`): new `ui/src/modules/panels/CanvasPanel.svelte`
  - Lists the active session's refs (live via `canvas_refs_changed` + `canvas_updated`).
  - Inline SVG preview for mermaid/d2 (reuses `renderMermaid`/`renderD2`); Excalidraw shows a format card (open-in-canvas to edit).
  - Actions: open in Canvas (navigates module + opens scene), attach existing scene (search picker over `GET /canvas/scenes`), detach, new scene (creates + refs + navigates).
  - Gating unchanged (`agents` module, agent sessions) — matches existing right-panel behavior.
- **SessionView ⋯ menu**: "Canvas" item for every session kind → opens the right-panel canvas tab (agent sessions) or navigates to the Canvas module (other kinds). Satisfies "accessible from each session".
- **Cross-module search** (`crates/otto-server/src/routes/search.rs`): index canvas scenes (`kind:"canvas"`, title/section match) → palette can jump to a scene from anywhere.

## 4. Product interaction (R3)

- `LinkedCanvases.svelte`: add format badge (mermaid/excalidraw/d2) + relative updated-at on each card; "New canvas" from the story offers all three formats and pre-links `story_id` (verify existing create-linked path; add if missing).
- Discovery Chat `create_canvas` action: pass-through format so D2 canvases can be created from discovery (default stays mermaid).
- Canvas hero/new-scene flows expose D2 uniformly, so Product-linked scenes can be any format.

## 5. Product page UX refresh (R5)

Principle: **regroup nothing, remove nothing — consolidate chrome, unify idioms, adopt shared components.** All 13 sub-views keep their component + behavior.

1. **Header consolidation (3 rows → 1–2)**: Stories|Learnings toggle moves into the sidebar header; the group strip and sub-strip merge into one compact band (groups as segmented tabs with icons; sub-tabs as a pill row directly beneath, only when >1). Update `product-tabs.spec.ts` assertions to the new DOM while keeping the same semantic checks (4 groups, Discover→Chat renders chat).
2. **Shared `ListPane`** (`ui/src/modules/product/ui/ListPane.svelte`): the 220–240px list-pane idiom used by ChatTab, RefineTab, MockupsTab — one component (header, actions slot, item list slot, empty slot).
3. **Modals on `Modal.svelte`**: `ImportDialog` + `PublishDialog` rebuilt on the shared modal (focus trap/escape/backdrop for free), keeping their internals.
4. **Empty states on `EmptyState.svelte`**: ProductPage no-story, ChatTab, MockupsTab, and other bespoke empties adopt the shared component with consistent copy tone.
5. **Button/tab consistency**: a small scoped stylesheet `ui/src/modules/product/product.css` (or shared classes in ProductPage) defining `.p-btn`, `.p-btn.primary`, `.p-tab` used across the module — replaces the per-file re-declarations it touches (no app-wide Button refactor).
6. **Sidebar polish**: story rows get source icon (Jira/Confluence/draft), status pill, tighter spacing; New draft/Import as icon buttons with labels.
7. **Hygiene**: fix the stale 13-tab header comment; remove the unreachable "coming soon" fallback.
8. RTL/logical-properties + tokens + light/dark discipline maintained throughout; phone accordion behavior preserved (product-mobile spec must stay green).

## 6. Testing (R6)

- **Rust**: `cargo test --workspace` — new tests: refs repo (otto-state), refs routes + policy (otto-canvas/otto-server tests), mcp_tools catalog/dispatch for the two write tools, `doc_format`/`file_name` d2 cases.
- **E2E (Playwright)**:
  - `canvas.spec.ts` additions: D2 hero/new-scene → SVG renders; D2 code edit → re-render + autosave PUT; Ask-AI writes `canvas.d2` (stub); sketch toggle persists; PNG/SVG export buttons present; scene search filters; duplicate scene.
  - New `desktop-canvas-panel.spec.ts` (desktop-browser project): agent session → right-panel Canvas tab → attach scene → preview renders → detach; new-scene-from-panel navigates; `⋯` menu Canvas action.
  - Product: existing 9 specs stay green (functionality preserved); `product-tabs.spec.ts` updated for the consolidated header; spot-add: import dialog opens as shared modal, empty states render.
- **Gates**: `cargo build/test/clippy -D warnings`, `ui npm run check`, `npm run build`, full `npm run test:e2e` with `OTTO_E2E_BIN` pointed at the branch-built ottod (Keychain gotcha: `OTTO_SECRETS=file`).

## 7. Delivery (R7/R8)

Worktree `/Users/itziklavon/otto_os-worktrees/canvas-d2-product`, branch `feat/canvas-d2-product-polish`. At merge time: re-check main's max migration (currently 0091; two other worktrees active — renumber `0092` upward if claimed), merge to local main only (no push), then `deploy.sh` (`mkdir -p apps/desktop/src-tauri/binaries`; `~/.hermes/node/bin` on PATH) and verify health on 7700.

## Risks & mitigations

- **D2.js in Vite/webview**: worker+WASM bundling quirks → smoke-test the dynamic import in `npm run build` + a real render early (first implementation step); fallback is pinning the worker URL via the package's `/worker` export.
- **7.8MB chunk**: lazy import keeps first-load unaffected; desktop app so size is acceptable.
- **Migration collision**: known dance — renumber at merge, FF-last, reconcile sqlx `(version,checksum)` before deploy.
- **product-tabs/product-mobile spec drift**: update assertions in the same commit as the header change; keep semantics identical.
- **RightPanel regressions**: new tab is additive; gating untouched; desktop-shell spec unaffected (shell sessions keep no right panel).

## Self-review notes

- No placeholders/TBDs; each requirement maps to a section (R1→§1, R2→§2, R3→§4, R4→§3, R5→§5, R6→§6, R7/R8→§7).
- Consistency check: format stays a string everywhere (no half-enum); refs are a join table distinct from the assist `session_id`; D2 render is UI-only so CRUD/back-end stays format-agnostic.
- Scope check: one branch, ~4 workstreams, no cross-cutting refactors beyond the product chrome; OverviewTab split explicitly deferred.
