# Canvas D2 + Session Canvas Refs + Product UX Refresh — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add D2 as a third Canvas format, expose canvas references inside every agent session (panel + MCP write tools), and consolidate the Product page chrome — all E2E-tested.

**Architecture:** D2 renders client-side via the official `@terrastruct/d2` WASM package (lazy chunk), mirroring the Mermaid path 1:1 (file-backed `canvas.d2`, assist prompt variant, E2E stub fence). Session references are a new `canvas_scene_refs` join table + three routes on the otto-canvas router + two MCP write tools + a new RightPanel tab. Product refresh is chrome-only: no view is removed.

**Tech Stack:** Rust (axum/sqlx), Svelte 5 runes, `@terrastruct/d2@^0.1.33`, CodeMirror 6, Playwright.

**Design doc:** `docs/design/canvas-d2-sessions-product-2026-07-02.md` (committed).

## Global Constraints

- Worktree: `/Users/itziklavon/otto_os-worktrees/canvas-d2-product`, branch `feat/canvas-d2-product-polish`. ALL work happens there.
- Migrations are append-only; new migration number is **0092** (provisional — renumber above main's max at merge).
- Contracts first: any endpoint/WS change updates `docs/contracts/api.md` + `docs/contracts/ws.md` + `ui/src/lib/api/types.ts` (or `ui/src/modules/canvas/types.ts`) in the same task.
- Match surrounding code idiom (dense doc comments, logical CSS properties, tokens, no hardcoded colors except existing error red).
- Desktop-only Playwright specs MUST be named `desktop-*.spec.ts` (config `testMatch`); all other specs run on 5 mobile/tablet projects.
- Format stays a **string** in Rust (validated set), a union in TS. Never introduce a Rust enum for it.
- Gates: `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`; `cd ui && npm run check && npm run build`.
- Commit after each task with conventional-commit messages; NEVER mention Claude/AI in commit messages.

---

### Task A — D2 canvas format (backend + stub + UI + skill) & canvas quality

**Files:**
- Modify: `crates/otto-server/src/canvas_assist.rs` (doc_format :231, base_source :249, file_name :260, build_assist_prompt :357, tests :499+)
- Modify: `crates/otto-orchestrator/src/e2e_stub.rs` (canvas_assist_reply :104, tests)
- Create: `ui/src/modules/canvas/d2.ts`
- Create: `ui/src/modules/canvas/D2Canvas.svelte`
- Create: `ui/src/modules/canvas/export.ts` (SVG→PNG + copy-source helpers shared by Mermaid/D2)
- Modify: `ui/src/modules/canvas/types.ts` (`CanvasFormat` :221, `AssistResult` :201)
- Modify: `ui/src/modules/canvas/CanvasPage.svelte` (render branch :88-92, blankDoc :47, hero :116-127)
- Modify: `ui/src/modules/canvas/SceneList.svelte` (new-scene menu ~:118, add Duplicate to the row menu near rename/move/delete)
- Modify: `ui/src/modules/canvas/MermaidCanvas.svelte` (zoombar: add PNG export + copy-source buttons)
- Modify: `ui/src/lib/stores/canvas.svelte.ts` (open() format parse :121, assist result)
- Modify: `ui/package.json` (+`@terrastruct/d2": "^0.1.33"`)
- Modify: `crates/otto-skills/assets/skills/development/otto-canvas/SKILL.md` (+`references/d2-cheatsheet.md`, `scripts/canvas.mjs`)
- Test: extend `ui/e2e/canvas.spec.ts` (see Task E for spec list)

**Interfaces produced (later tasks rely on these):**
- TS: `export type CanvasFormat = 'mermaid' | 'excalidraw' | 'd2'`
- TS: `renderD2(id: string, src: string, opts?: { sketch?: boolean; dark?: boolean }): Promise<{ svg?: string; error?: string }>` in `d2.ts` (same result shape as `renderMermaid`).
- Rust: `doc_format` returns `"d2"` for `{"format":"d2"}`; `file_name("d2") == "canvas.d2"`; `AssistResult` gains `pub d2: Option<String>` (serialized, skip-if-none like `excalidraw`).

**Steps:**

- [ ] **A1: Backend format plumbing (TDD).** Extend the existing unit tests in `canvas_assist.rs` FIRST:
  ```rust
  // in base_and_format_defaults:
  assert_eq!(doc_format(&serde_json::json!({"format":"d2"})), "d2");
  assert!(base_source("d2").contains("direction"));
  // new test:
  #[test]
  fn d2_prompt_points_at_d2_file() {
      let p = build_assist_prompt("payments arch", "d2", "canvas.d2", "direction: right\n");
      assert!(p.contains("OTTO_TASK: canvas_assist"));
      assert!(p.contains("canvas.d2"));
      assert!(p.contains("D2 file"));
      assert!(p.contains("sql_table"));   // prompt teaches key D2 shapes
      assert!(p.contains("payments arch"));
  }
  ```
  Run `cargo test -p otto-server canvas_assist` → FAIL. Then implement:
  - `doc_format`: `.filter(|f| *f == "mermaid" || *f == "excalidraw" || *f == "d2")`
  - `base_source`: `"d2" => "direction: right\n".to_string()`
  - `file_name`: `"d2" => "canvas.d2"`
  - `build_assist_prompt`: new `if format == "d2"` branch BEFORE the mermaid fallback: instructs editing the D2 file in place (one complete valid D2 diagram, no fences), covers: containers (`server: { api; db }`), edges + labels (`a -> b: label`), `direction: right|down`, shapes (`shape: sql_table | sequence_diagram | cylinder | queue | person`), classes (`classes: { critical: { style: { fill: "#fee2e2"; stroke: "#dc2626" } } }`), `style.fill/stroke/font-color`, icons (`icon: https://icons.terrastruct.com/...` — optional), near/grid hints, ends with "Reply with ONE short sentence describing what you changed.\n\nRequest: {user_prompt}".
  - `AssistResult`: add `#[serde(skip_serializing_if = "Option::is_none")] pub d2: Option<String>`; `result_for`: `"d2" => r.d2 = Some(source.to_string())`; `resolve_source` from_reply match: `"d2" => parsed.d2.clone()`; `parse_assist`: try `extract_fenced(raw, "d2")` before the mermaid fence, returning `AssistResult { d2: Some(src), note: prose_before_fence(raw), ..Default::default() }`.
  Run tests → PASS. Commit `feat(canvas): accept d2 as a scene format in the assist pipeline`.

- [ ] **A2: E2E stub D2 branch (TDD).** Add test in `e2e_stub.rs`:
  ```rust
  #[test]
  fn canvas_reply_d2_mode() {
      let d = canned_reply("OTTO_TASK: canvas_assist edit the D2 file `canvas.d2`");
      assert!(d.contains("```d2"));
      assert!(!d.contains("```mermaid"));
  }
  ```
  Implement in `canvas_assist_reply`, before the mermaid fallback:
  ```rust
  if prompt.contains("canvas.d2") || prompt.contains("D2 file") {
      return "Drew the order flow in D2 with a validation decision.\n\n\
  ```d2\n\
  direction: right\n\
  start: \"🚀 Start\" { style.fill: \"#dcfce7\" }\n\
  valid: \"❓ Valid?\" { shape: diamond }\n\
  process: \"⚙️ Process order\"\n\
  reject: \"❌ Reject\" { style.fill: \"#fee2e2\" }\n\
  start -> valid\n\
  valid -> process: yes\n\
  valid -> reject: no\n\
  ```"
          .to_string();
  }
  ```
  Run → PASS. Commit `feat(e2e-stub): d2-aware canvas assist reply`.

- [ ] **A3: `d2.ts` lazy renderer.** New file modeled on `mermaid.ts` (lazy + memoized module; never throws; result `{svg} | {error}`):
  ```ts
  // Lazy D2 bridge. @terrastruct/d2 is a 7.8MB WASM bundle so it MUST stay out of
  // the main chunk — dynamic import on first use, one shared instance after.
  type D2Api = {
    compile: (src: string, opts?: Record<string, unknown>) =>
      Promise<{ diagram: unknown; renderOptions: Record<string, unknown> }>;
    render: (diagram: unknown, opts?: Record<string, unknown>) => Promise<string>;
  };
  let _d2: D2Api | null = null;
  let _loading: Promise<D2Api> | null = null;
  async function load(): Promise<D2Api> {
    if (_d2) return _d2;
    _loading ??= import('@terrastruct/d2').then((m) => {
      _d2 = new m.D2() as unknown as D2Api;
      return _d2;
    });
    return _loading;
  }
  export async function renderD2(
    _id: string,
    src: string,
    opts: { sketch?: boolean; dark?: boolean } = {},
  ): Promise<{ svg?: string; error?: string }> {
    const text = src.trim();
    if (!text) return { error: 'Empty diagram' };
    try {
      const api = await load();
      const compiled = await api.compile(text, {
        sketch: opts.sketch ?? false,
        themeID: opts.dark ? 200 : 0,
      });
      const svg = await api.render(compiled.diagram, compiled.renderOptions);
      return { svg };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      return { error: msg.replace(/^Error:\s*/, '').trim() || 'Diagram error' };
    }
  }
  ```
  NOTE: verify the exact compile-options shape against the installed package's `index.d.ts` (options may nest under `{ options: { sketch, themeID } }`) and the dark theme id (200 = Dark Mauve) — adjust to the real API, keep the exported signature stable. Smoke-test in the browser (Task E) before assuming.
- [ ] **A4: `D2Canvas.svelte`.** Copy `MermaidCanvas.svelte` structurally (same pan/zoom surface, zoombar, code pane, live `canvasDocBus` effect, module-scope `liveId`, save guard against scene switches) with these deltas:
  - `renderMermaid` → `renderD2`; re-render also when `sketch` toggles.
  - Save writes `format: 'd2'` doc; `CodeEditor path="canvas.d2"`.
  - `generate()` reads `res.d2 ?? ''` and ingests `{type:'otto-canvas',version:1,format:'d2',source:src}`.
  - Mode chip label `D2`; a `Sketch` pill toggle next to the Code toggle; sketch state persists in the doc (`doc.sketch`) via the same save path and is read in `open()`/ingest.
  - `notMermaid` guard becomes `notD2` (JSON-looking source → "holds Excalidraw content" message).
  - Zoombar additionally gets PNG export + copy-source (from `export.ts`, step A5).
- [ ] **A5: `export.ts` + retrofit Mermaid.** New shared helpers:
  ```ts
  export function svgToPngDownload(svgEl: SVGSVGElement, filename: string, scale = 2): void
  export async function copyText(text: string): Promise<boolean>
  ```
  `svgToPngDownload`: serialize SVG → `new Image()` with `data:image/svg+xml` URL → draw on an offscreen `<canvas>` at `scale`× the SVG viewBox size → `toBlob('image/png')` → anchor download. Add PNG + copy-source buttons to `MermaidCanvas.svelte`'s zoombar and use in `D2Canvas.svelte`.
- [ ] **A6: Types + store + page wiring.**
  - `types.ts`: `CanvasFormat = 'mermaid' | 'excalidraw' | 'd2'`; `CanvasDoc` gains `sketch?: boolean`; `AssistResult` gains `d2?: string | null`.
  - `canvas.svelte.ts` `open()`: `this.format = doc?.format === 'excalidraw' ? 'excalidraw' : doc?.format === 'd2' ? 'd2' : 'mermaid';`
  - `CanvasPage.svelte`: `blankDoc('d2')` → `{ type:'otto-canvas', version:1, format:'d2', source:'' }`; render branch `{:else if canvas.format === 'd2'}<D2Canvas …/>`; hero third card (icon `layers`, title "D2 diagram", sub "Modern declarative diagrams — architecture, sequence & SQL tables").
  - `SceneList.svelte`: third new-menu entry (same copy as hero card); **Duplicate** action in the scene row menu — `GET /canvas/scenes/{id}` → `POST /workspaces/{ws}/canvas/scenes` with `{title: title + ' (copy)', doc: JSON.parse(doc_json), section}` → `loadScenes()`.
- [ ] **A7: npm dep + build check.** `cd ui && npm install @terrastruct/d2@^0.1.33` then `npm run check && npm run build`; confirm the d2 chunk is code-split (build output lists a separate multi-MB chunk, main bundle unchanged ±50KB).
- [ ] **A8: Skill v5.** `SKILL.md`: bump version, document THREE modes incl. `canvas.d2`; add `references/d2-cheatsheet.md` (shapes, containers, classes, styles, sql_table, sequence, direction — 1 page); `scripts/canvas.mjs`: `add-d2 <scene-title> <file.d2>` subcommand mirroring `add-mermaid`.
- [ ] **A9: Gates + commit.** `cargo test -p otto-server -p otto-orchestrator`, `cd ui && npm run check && npm run build` → all green. Commit `feat(canvas): D2 scene format — WASM renderer, sketch mode, exports, skill v5`.

### Task B — Session canvas references (migration + routes + events + MCP + panel)

**Files:**
- Create: `crates/otto-state/migrations/0092_canvas_scene_refs.sql`
- Modify: `crates/otto-state/src/canvas.rs` (repo methods + `CanvasRepo::delete` cascade)
- Modify: `crates/otto-canvas/src/http.rs` (3 new routes; `CanvasCtx` unchanged — repo already exposed) — **plus** resolve session→workspace via a new small trait method `sessions_ws()` on `CanvasCtx` OR (simpler, chosen) register the routes in `crates/otto-server/src/canvas_assist.rs`-style module `crates/otto-server/src/canvas_refs.rs` where `ServerCtx` has sessions + canvas_repo + events. **Decision: new module `canvas_refs.rs` in otto-server**, routes registered in `modules.rs` next to the assist routes (:520-527).
- Modify: `crates/otto-core/src/event.rs` (+`CanvasRefsChanged { workspace_id, session_id }`)
- Modify: `crates/otto-server/src/ws_events.rs` (deliver like `CanvasUpdated`, workspace-scoped)
- Modify: `crates/otto-server/src/policy.rs` (canvas-refs paths → `Feature::Canvas`; GET=View else Edit; the existing `/sessions/` prefix rule must NOT shadow it — check rule order, add explicit match on `/canvas-refs`)
- Modify: `crates/ottod/src/mcp_tools.rs` (2 write tools + catalog text update on the 2 read tools)
- Modify: `crates/otto-server/src/routes/search.rs` (canvas source)
- Create: `ui/src/modules/panels/CanvasPanel.svelte`
- Modify: `ui/src/shell/RightPanel.svelte` (tabs array :44-52 + body switch :135-164)
- Modify: `ui/src/lib/stores/ui.svelte.ts` (`RightTab` union + `'canvas'`)
- Modify: `ui/src/lib/events.svelte.ts` (dispatch `canvas_refs_changed` → bump a small `canvasRefsBus`)
- Modify: `ui/src/modules/agents/SessionView.svelte` (⋯ menu "Canvas" item)
- Modify: `ui/src/modules/canvas/types.ts` (no new types needed beyond `CanvasSceneSummary` reuse)
- Test: `crates/otto-state` inline repo tests; `crates/otto-server/tests/canvas_refs_api.rs` (minimal-router harness like `share_api.rs`); mcp_tools unit tests; `ui/e2e/desktop-canvas-panel.spec.ts` (Task E)

**Interfaces produced:**
- SQL table `canvas_scene_refs(scene_id, session_id, workspace_id, created_by, created_at, PK(scene_id, session_id))`
- Rust repo: `add_ref(&self, scene_id: &Id, session_id: &Id, workspace_id: &Id, user_id: &Id) -> Result<()>` (idempotent upsert), `remove_ref(scene_id, session_id) -> Result<()>`, `list_refs_for_session(session_id) -> Result<Vec<CanvasSceneSummary>>`
- HTTP: `GET /sessions/{sid}/canvas-refs` → `[CanvasSceneSummary]`; `POST /sessions/{sid}/canvas-refs {scene_id}` → 204; `DELETE /sessions/{sid}/canvas-refs/{scene_id}` → 204
- Event: `canvas_refs_changed {workspace_id, session_id}`
- MCP: `canvas_create_scene {title, format?, source?, section?}` → `{scene_id, workspace_id}`; `canvas_update_scene {scene_id, source}` → `{ok: true, format}`

**Steps:**

- [ ] **B1: Migration 0092** — exactly the design-doc SQL (table + `idx_canvas_refs_session` index). `cargo build -p otto-state` (sqlx migrate embeds).
- [ ] **B2: Repo methods (TDD).** Inline `#[cfg(test)]` in `otto-state/src/canvas.rs` following the crate's existing in-memory-pool test pattern: create scene → add_ref (twice, idempotent) → list_refs_for_session returns 1 summary → remove_ref → empty. Extend `CanvasRepo::delete` with `DELETE FROM canvas_scene_refs WHERE scene_id = ?` before the row delete. ALSO here (so Task C never touches otto-state): add `pub format: Option<String>` to `CanvasSceneSummary`, populated via `json_extract(doc_json,'$.format')` in ALL summary SELECTs (`list_for_workspace`, `list_for_story`, `list_for_user`, `list_refs_for_session`), mirrored in `ui/src/modules/canvas/types.ts` (`format?: CanvasFormat`). Run `cargo test -p otto-state canvas` → PASS. Commit.
- [ ] **B3: Routes module `canvas_refs.rs`.** Handlers resolve the session via `ctx.manager.get_session(&sid)` (or the sessions repo — mirror how `attach-product` at `modules.rs:4805` loads a session), take its `workspace_id`, role-check Viewer (GET) / Editor (mutations) via `crate::auth::require_ws_role`, verify the scene exists and (on POST) belongs to the same workspace → 400 otherwise, call repo, broadcast `Event::CanvasRefsChanged`. Register in `modules.rs` near the canvas assist routes. Policy: add a `/canvas-refs` match arm mapping to `Require(Canvas, View|Edit)` — insert BEFORE any generic `/sessions/` rule so it wins; run `cargo test -p otto-server policy_coverage` to prove coverage. Integration test `canvas_refs_api.rs` with the minimal-router harness (in-memory sqlite + `sqlx::migrate!` + synthetic AuthUser): GET empty → POST → GET has 1 → cross-workspace POST 400/404 → DELETE → GET empty. Commit.
- [ ] **B4: Event plumbing.** `event.rs` variant + `ws_events.rs` delivery (copy the `CanvasUpdated` arm — workspace-member scoped). `docs/contracts/ws.md`: add `canvas_updated`, `canvas_session_started`, `mockup_updated`, `mockup_session_started` (existing, undocumented) AND `canvas_refs_changed` to the catalog; fix the "16 variants" count. Commit.
- [ ] **B5: MCP write tools (TDD).** In `mcp_tools.rs`: catalog entries (clear descriptions: "Create a Canvas scene (format: mermaid | d2 | excalidraw) and reference it to this session — it appears in the session's Canvas panel and the Canvas module"), `run_tool` arms:
  - `canvas_create_scene`: resolve workspace from `ctx` env (`OTTO_WORKSPACE_ID`), POST `/workspaces/{ws}/canvas/scenes` with `{title, section?, doc: {type:"otto-canvas",version:1,format,source}}` (format validated against the 3-set, default `mermaid`), then POST `/sessions/{OTTO_SESSION_ID}/canvas-refs {scene_id}` (best-effort), return `{scene_id, workspace_id}`.
  - `canvas_update_scene`: GET `/canvas/scenes/{id}` → parse doc → PUT with `{doc: {…, source: new}}` preserving format/sketch. Return `{ok, format}`.
  - Update the 2 read-tool descriptions (drop "writes are not exposed").
  - Unit tests: catalog contains both names; args validation (missing title → err; bad format → err). Note per crate pattern: write tools post to real governed endpoints as the session owner — audited via existing `ctx.audit` path; follow `otto_db_query`'s arm shape.
  Commit.
- [ ] **B6: Search source.** `search.rs`: query `ctx.canvas_repo.list_for_workspace(&ws)`, filter title/section contains-q (case-insensitive), `SearchHit{kind:"canvas", id, title, subtitle: section, actions:["Open in Canvas"]}` with the standard per-source cap; extend the module doc-comment source list. Commit.
- [ ] **B7: CanvasPanel.svelte.** New panel component (~250 lines), pattern-match `ActivityPanel`/`FilesPanel` styles:
  - Loads refs for `ws.activeSession.id` on mount + whenever `canvasRefsBus` ticks for this session or `canvasDocBus` ticks for a listed scene.
  - Each ref row: title, format chip, updated-at; expandable inline SVG preview for mermaid/d2 (call `renderMermaid`/`renderD2` with the fetched scene source — fetch on expand, cache per scene id + updated_at); Excalidraw rows show a static "board" card (no heavy React mount in the panel).
  - Row actions: Open in Canvas (`canvas.pendingOpenId = id; ui.go('canvas')` — mirror how `core.go-canvas` palette command navigates in `App.svelte`), Detach (DELETE).
  - Footer actions: "Attach scene…" (inline searchable list from `GET /canvas/scenes`, click = POST), "New scene" (POST create in `ws.currentId` with blank mermaid doc + auto-ref + open in Canvas).
  - Empty state via shared `EmptyState.svelte` ("No canvases referenced — attach one or ask the agent to draw").
- [ ] **B8: Shell wiring.** `ui.svelte.ts`: extend `RightTab` union with `'canvas'`. `RightPanel.svelte`: tab entry `{ id: 'canvas', icon: 'shapes', label: 'Canvas' }` + body branch `<CanvasPanel />`. `events.svelte.ts`: add `canvasRefsBus` (tick + sessionId, modeled on `CanvasDocBus` :192-205) + dispatch branch for `canvas_refs_changed`. `SessionView.svelte` ⋯ menu: "Canvas" item → agent session: `ui.openRight('canvas')`; other kinds: navigate to canvas module. Commit.
- [ ] **B9: Contracts.** `api.md`: new numbered rows for the 3 refs endpoints under the Canvas Studio section + note on MCP write tools; fix stale #105/#107 notes (update req fields `provider/section/story_id`; assist DOES commit). `ui/src/modules/canvas/types.ts` — no new DTOs needed (`CanvasSceneSummary` reused); ensure `api.md` documents that. Commit `feat(canvas): session canvas references — refs table, routes, MCP write tools, session panel`.
- [ ] **B10: Gates.** `cargo build/test/clippy` workspace + `npm run check` → green.

### Task C — Product page UX refresh (chrome consolidation, shared idioms)

**Files:**
- Modify: `ui/src/modules/product/ProductPage.svelte` (header rows :383-411, sidebar, stale comment :1-4, dead fallback :462-463, phone accordion :975-1145)
- Create: `ui/src/modules/product/ui/ListPane.svelte`
- Modify: `ui/src/modules/product/ChatTab.svelte`, `RefineTab.svelte`, `MockupsTab.svelte` (adopt ListPane)
- Modify: `ui/src/modules/product/ImportDialog.svelte`, `PublishDialog.svelte` (rebuild on `ui/src/lib/components/Modal.svelte`)
- Modify: empty states → `EmptyState.svelte` in `ProductPage.svelte` (:419), `ChatTab.svelte` (:128), `MockupsTab.svelte` (:236)
- Modify: `ui/src/modules/product/LinkedCanvases.svelte` (format chip + updated-at + create-new offers 3 formats)
- Modify: `ui/e2e/product-tabs.spec.ts` (assert the consolidated header; SAME semantics: 4 groups visible, Discover→Chat renders chat)
- Test: existing product specs must stay green unmodified except `product-tabs` (and `product-mobile` only if the accordion DOM changed — prefer not to change it)

**Constraints (hard):**
- ALL 13 sub-views keep their components, props, and behavior. NO store changes (`product.tab` strings unchanged — E2E and deep links depend on them).
- Keep logical CSS properties, tokens, `color-mix` hovers; phone `@media (max-width: 640px)` accordion preserved.

**Steps:**

- [ ] **C1: Header consolidation.** Move the Stories|Learnings toggle into the sidebar head (segmented mini-toggle above the story list). Merge group strip + sub strip into ONE header band: group tabs (icon + label, segmented) inline-start, active group's sub-tabs as smaller pills inline-end (wrap on narrow); when a group has 1 sub, no pills. Keep `GROUPS` data + `selectGroup()` logic verbatim. Fix the stale header comment (describe the real 4-group/13-sub structure), delete the unreachable `{:else}` fallback.
- [ ] **C2: ListPane extraction.** Component API:
  ```svelte
  <ListPane title="Chats" width={220}>
    {#snippet actions()}…new/import buttons…{/snippet}
    {#snippet children()}…rows…{/snippet}
    {#snippet empty()}…EmptyState…{/snippet}
  </ListPane>
  ```
  Adopt in ChatTab/RefineTab/MockupsTab; delete their duplicated `.pane-head`/`.toolbar-btn` CSS.
- [ ] **C3: Dialogs on shared Modal.** Rebuild ImportDialog + PublishDialog contents inside `Modal.svelte` (keep internal form markup/logic; drop bespoke overlay/backdrop/escape code).
- [ ] **C4: Empty states + sidebar polish + button consistency.** EmptyState adoption (3 sites); story rows: source icon (jira/confluence/draft), status pill, 2-line clamp; define `.p-btn`/`.p-btn.primary`/`.p-tab` once in ProductPage (global-scoped to the module via `:global(.product-root …)` or a small `product.css` imported by ProductPage) and swap the per-file duplicates in the files this task already touches (do NOT churn untouched tabs).
- [ ] **C5: LinkedCanvases upgrade.** Format chip using `CanvasSceneSummary.format` (added server-side in Task B2 — render the chip ONLY when the field is present so this task is safe to land before/parallel to Task B). Updated-at relative label. "New canvas" menu offers the 3 formats (creates with `story_id`, navigates to Canvas).
- [ ] **C6: product-tabs.spec update + run product specs.** `npx playwright test product- --project=iphone-portrait` (and ipad) green; `npm run check`. Commit `refactor(product): consolidate chrome — single header band, shared ListPane/Modal/EmptyState, sidebar polish`.

### Task D — E2E specs + docs + full gates

**Files:**
- Modify: `ui/e2e/canvas.spec.ts` (D2 coverage; reuse `seedScene` helper with `format:'d2'`)
- Create: `ui/e2e/desktop-canvas-panel.spec.ts`
- Modify: `docs/features/canvas.md` (D2 mode + session refs + panel), `docs/features/` sessions doc (canvas tab mention)
- Verify: `docs/contracts/api.md` + `ws.md` complete (from Tasks A/B)

**Steps:**

- [ ] **D1: canvas.spec.ts D2 tests** (mobile projects — match existing canvas tests' style):
  1. New-scene menu shows D2 → create → D2 empty state visible.
  2. Seeded `d2` scene (`seedScene(..., 'd2', 'direction: right\na -> b: hi')`) renders SVG in `.content svg`.
  3. Code panel edit → autosave PUT to the scene + preview re-renders (mirror the mermaid Code test :409).
  4. Ask-AI on a D2 scene → stub writes `canvas.d2` on disk (mirror :225) + preview shows the stub diagram nodes.
  5. Sketch toggle → doc PUT carries `sketch:true` and persists on reload.
  6. Export buttons present (PNG + SVG) on a rendered D2 scene; Duplicate action creates "(copy)" scene in the list.
- [ ] **D2: desktop-canvas-panel.spec.ts** (desktop-browser project only; self-skip otherwise like `desktop-shell.spec.ts:18`): seed an agent session + a mermaid scene via API → open Agents → open right panel Canvas tab → "Attach scene…" → scene listed → expand → SVG preview renders → POST ref visible via API → "Open in Canvas" navigates to canvas module with the scene open → back → Detach → list empties. Also: `POST /sessions/{sid}/canvas-refs` from the API + `canvas_refs_changed` → panel updates without reload.
- [ ] **D3: Feature docs** — `docs/features/canvas.md`: three modes incl. D2 (WASM client render, sketch, exports), session references section (panel + MCP tools + endpoints).
- [ ] **D4: Full gates in the worktree.**
  ```bash
  cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
  cd ui && npm run check && npm run build
  cargo build --release -p ottod   # for OTTO_E2E_BIN
  # RELEVANT specs only (user directive: never the full suite — overkill):
  OTTO_E2E_BIN=$PWD/../target/release/ottod OTTO_SECRETS=file OTTO_E2E_SLOT=3 \
    npx playwright test canvas desktop-canvas-panel product-tabs product-mobile product-mockups product-refine discovery-chat
  ```
  (Slot 3 avoids the default 7799 + any concurrent agent runs; kill stale daemons from `.auth-3/daemon.json` if a previous run crashed.) The listed specs = every spec whose surface this branch touches (canvas module, session right panel, product chrome/tabs incl. ListPane adopters). All of them green. Commit.

### Task E — Review, merge, deploy

- [ ] **E1: Requirements audit** — walk R1–R8 against the diff; fix gaps.
- [ ] **E2: Code review** (superpowers:requesting-code-review) on `git diff main...HEAD`; fix findings; re-run gates.
- [ ] **E3: Merge.** Sync `main` (other agents may have merged): `git fetch . && git -C <mainrepo> log --oneline -3`; check `crates/otto-state/migrations/` max on main — if ≥0092 exists, renumber ours to max+1 (single commit `chore: renumber migration NNNN→MMMM`). Merge branch → local main (merge commit, no push). Re-run `cargo test --workspace` + `npm run check` on merged main.
- [ ] **E4: Deploy.** From the MAIN checkout: `mkdir -p apps/desktop/src-tauri/binaries; PATH="$HOME/.hermes/node/bin:$PATH" ./deploy.sh`; verify `curl -s http://127.0.0.1:7700/api/v1/health`, app relaunched, daemon PID stable past the supervisor window (deploy.sh verifies). Sanity: open a D2 scene in the running app via API seed if feasible.

## Self-review

- Spec coverage: R1→A, R2→A5/A6 (+already-existing zoom/fit/search noted), R3→C5, R4→B, R5→C, R6→D, R7/R8→E. ✓
- No placeholders: every step names exact files/anchors; code included for novel pieces; A3 flags the one API detail to verify against the installed package rather than guessing. ✓
- Type consistency: `CanvasFormat` union extended once (A6) and consumed by B7/C5; `renderD2` signature defined in A3, used in B7/D1; refs endpoints defined in B3, consumed by B5/B7/D2. `CanvasSceneSummary.format` added in C5 — B7's format chip must tolerate its absence until C5 lands (render chip only when present). ✓
