// Canvas Studio store — scene list + the live editable Scene, autosave, coarse
// snapshot undo/redo, and agent assist. Reads `ws.currentId` only.
//
// Source-of-truth model: the store owns `scene`. A `rev` counter is the
// "reload the editor" signal — it bumps ONLY on external changes (open / undo /
// redo / assist insert / rename / template). The editor owns its live flow
// arrays for smooth dragging and pushes edits back via `commitFromEditor`,
// which updates `scene` + schedules a save + records an undo snapshot WITHOUT
// bumping `rev` (so the editor is not yanked mid-drag).

import { api, getToken } from '../api/client';
import { ws } from './workspace.svelte';
import { defaultAgentProvider } from '../providers';
import { loadErrorText } from '../loadError';
import { assistToNodes, emptyScene, parseScene } from '../../modules/canvas/scene';
import type {
  AssistMode,
  AssistResult,
  CanvasDoc,
  CanvasFormat,
  CanvasScene,
  CanvasSceneSummary,
  Scene,
} from '../../modules/canvas/types';

/** One inline conversation turn with the canvas agent (client-side). */
export interface CanvasTurn {
  role: 'user' | 'assistant';
  text: string;
  ts: number;
}

const AUTOSAVE_MS = 900;
const HISTORY_CAP = 50;

class CanvasStore {
  scenes = $state<CanvasSceneSummary[]>([]);
  listLoading = $state(false);
  listError = $state<string | null>(null);

  currentId = $state<string | null>(null);
  scene = $state<Scene | null>(null);
  /** Raw parsed doc for the open scene — Excalidraw `{elements,appState,files}`
   *  (the embedded editor reads this for initialData and writes it back). */
  rawDoc = $state<unknown>(null);
  /** The managed Otto session backing this scene's agent generation — resumed
   *  across "Ask AI" turns so the agent keeps refining the SAME source file. */
  sessionId = $state<string | null>(null);
  /** The file-backed source the agent edits (mermaid text or Excalidraw JSON).
   *  Non-null ⇒ the scene is agent/file-backed and renders from `source`. */
  source = $state<string | null>(null);
  /** The scene's source format (drives the render path). */
  format = $state<CanvasFormat>('mermaid');
  /** Which agent/provider drives this scene's Ask-AI (single choice). Seeds from
   *  the configured default agent, not a bare hardcoded 'claude'. */
  provider = $state<string>(defaultAgentProvider());
  /** Lightweight inline conversation with the canvas agent (client-side; the
   *  diagram is the durable artifact, persisted server-side). */
  convo = $state<CanvasTurn[]>([]);
  /** A scene id to auto-open once the Canvas module mounts (deep-link from e.g.
   *  the Discovery-Chat "Open in Canvas" action). CanvasPage consumes + clears it. */
  pendingOpenId = $state<string | null>(null);
  /** Bumps when the editor must reload from `scene` (open/undo/redo/assist). */
  rev = $state(0);

  saving = $state(false);
  savedAt = $state<number | null>(null);
  dirty = $state(false);
  /** Why the last `open()` failed (rendered inline with Retry by CanvasPage). */
  loadError = $state<string | null>(null);
  /** The scene id that `loadError` belongs to — what Retry re-opens. */
  loadErrorId = $state<string | null>(null);

  #openSequence = 0;
  // File-backed editors retain unsaved snapshots across scene/workspace changes.
  // A per-scene queue keeps delayed older PUTs from overwriting newer edits.
  #drafts = new Map<string, CanvasDoc>();
  docSaveErrors = $state<Record<string, string>>({});
  #writes = new Map<string, Promise<void>>();
  #authToken = getToken();
  #saveContext = 0;

  /** Editors capture this on mount; their cleanup must not save as a new login. */
  get saveContext(): number { return this.#saveContext; }

  resetAuthContext(): void {
    const token = getToken();
    if (token === this.#authToken) return;
    this.#authToken = token;
    this.#saveContext++;
    this.closeScene();
    this.#drafts.clear();
    this.docSaveErrors = {};
    this.scenes = [];
    this.rawDoc = null;
    this.source = null;
    this.convo = [];
    this.sessionId = null;
    this.pendingOpenId = null;
    this.loadError = null;
    this.loadErrorId = null;
    this.listError = null;
    this.listLoading = false;
    // Keep in-flight queues ordered, but invalidate all their old completions.
    if (token) void this.loadScenes().catch(() => {});
  }

  stageDoc(id: string, doc: CanvasDoc, context = this.#saveContext): void {
    if (context !== this.#saveContext) return;
    this.#drafts.set(id, doc);
    if (this.currentId === id) this.dirty = true;
  }

  async persistDoc(id: string, doc: CanvasDoc, context = this.#saveContext): Promise<void> {
    if (context !== this.#saveContext) return;
    this.stageDoc(id, doc, context);
    const previous = this.#writes.get(id);
    const write = (async () => {
      await previous?.catch(() => {});
      if (context !== this.#saveContext) return;
      await api.put(`/canvas/scenes/${id}`, { doc });
      if (context !== this.#saveContext) return;
      if (this.#drafts.get(id) !== doc) return;
      this.#drafts.delete(id);
      delete this.docSaveErrors[id];
      if (this.currentId === id) this.markSaved(doc);
    })();
    this.#writes.set(id, write);
    try { await write; }
    catch (e) {
      if (context !== this.#saveContext) return;
      this.docSaveErrors[id] = loadErrorText(e); throw e;
    }
    finally { if (this.#writes.get(id) === write) this.#writes.delete(id); }
  }

  #history: string[] = [];
  #future: string[] = [];
  #saveTimer: ReturnType<typeof setTimeout> | null = null;

  async retryDoc(id: string): Promise<void> {
    const doc = this.#drafts.get(id);
    if (doc) await this.persistDoc(id, doc);
  }

  get canUndo(): boolean {
    return this.#history.length > 0;
  }
  get canRedo(): boolean {
    return this.#future.length > 0;
  }

  // -- list --------------------------------------------------------------
  // Canvas is a GLOBAL tool: list the user's scenes across all workspaces (no
  // active-workspace requirement). Creating a scene still uses the current ws.
  async loadScenes(): Promise<void> {
    const context = this.#saveContext;
    this.listLoading = true;
    this.listError = null;
    try {
      const scenes = await api.get<CanvasSceneSummary[]>(`/canvas/scenes`);
      if (context === this.#saveContext) this.scenes = scenes;
    } catch (e) {
      if (context !== this.#saveContext) return;
      this.listError = loadErrorText(e);
      throw e;
    } finally {
      if (context === this.#saveContext) this.listLoading = false;
    }
  }

  async create(title: string, doc?: unknown, storyId?: string | null): Promise<CanvasScene> {
    const wsId = ws.currentId;
    if (!wsId) throw new Error('No workspace selected');
    const created = await api.post<CanvasScene>(`/workspaces/${wsId}/canvas/scenes`, {
      title,
      doc: doc ?? emptyScene(title),
      story_id: storyId ?? null,
    });
    await this.loadScenes().catch(() => {});
    return created;
  }

  async open(id: string): Promise<void> {
    const sequence = ++this.#openSequence;
    this.loadError = null;
    this.loadErrorId = null;
    try {
      const row = await api.get<CanvasScene>(`/canvas/scenes/${id}`);
      if (sequence !== this.#openSequence) return;
      this.currentId = row.id;
      this.scene = parseScene(row.doc_json, row.title);
      let doc: CanvasDoc | null = null;
      try {
        doc = JSON.parse(row.doc_json) as CanvasDoc;
      } catch {
        doc = null;
      }
      doc = this.#drafts.get(id) ?? doc;
      this.rawDoc = doc;
      this.source = typeof doc?.source === 'string' ? doc.source : null;
      this.format =
        doc?.format === 'excalidraw' ? 'excalidraw' : doc?.format === 'd2' ? 'd2' : 'mermaid';
      this.provider = (row as { provider?: string }).provider ?? defaultAgentProvider();
      this.convo = [];
      this.sessionId = row.session_id;
      this.#history = [];
      this.#future = [];
      this.dirty = this.#drafts.has(id);
      this.savedAt = Date.parse(row.updated_at) || null;
      this.rev += 1;
    } catch (e) {
      if (sequence !== this.#openSequence) return;
      this.loadError = loadErrorText(e);
      this.loadErrorId = id;
      throw e;
    }
  }

  closeScene(): void {
    ++this.#openSequence;
    if (this.#saveTimer) clearTimeout(this.#saveTimer);
    this.currentId = null;
    this.scene = null;
    this.#history = [];
    this.#future = [];
    this.dirty = false;
  }

  async del(id: string): Promise<void> {
    const context = this.#saveContext;
    await this.#writes.get(id)?.catch(() => {});
    if (context !== this.#saveContext) return;
    await api.del(`/canvas/scenes/${id}`);
    this.#drafts.delete(id);
    delete this.docSaveErrors[id];
    if (this.currentId === id) this.closeScene();
    await this.loadScenes().catch(() => {});
  }

  // -- edit / persist ----------------------------------------------------

  /** Re-read the open scene's `session_id` (set server-side on the first agent
   *  generation) so the conversation panel can open it. */
  async refreshSession(): Promise<void> {
    if (!this.currentId) return;
    try {
      const row = await api.get<CanvasScene>(`/canvas/scenes/${this.currentId}`);
      this.sessionId = row.session_id;
    } catch {
      /* best-effort */
    }
  }

  /** The board persisted the opaque doc itself — record it + mark saved. */
  markSaved(doc: unknown): void {
    this.rawDoc = doc;
    this.dirty = false;
    this.savedAt = Date.now();
  }

  /** Apply a canvas doc pushed from the server (a live agent edit or the final
   *  commit). Updates `source`/`format`/`rawDoc` (all PURE writes — safe to call
   *  from a render `$effect`). Ignores docs without a `source` (hand-drawn).
   *  Skipped while the user has UNSAVED edits in flight (`dirty`) — the agent's
   *  ~1s live poll would otherwise reset the source pane mid-keystroke; once
   *  the debounced save lands (dirty clears) live pushes apply again, and the
   *  server-side `expect_updated_at` guard arbitrates the final commit.
   *  `sceneId` binds a result to the scene it was produced for: an Ask-AI run
   *  can take minutes, and applying it to whichever scene is open by then
   *  wrote scene A's drawing into scene B on the next autosave. (The server
   *  already committed it to its own scene; opening that scene shows it.) */
  ingestDoc(doc: CanvasDoc, sceneId?: string | null): void {
    if (sceneId !== undefined && sceneId !== this.currentId) return;
    if (typeof doc.source !== 'string') return;
    if (this.dirty) return;
    this.source = doc.source;
    if (doc.format) this.format = doc.format;
    this.rawDoc = doc;
    this.savedAt = Date.now();
  }

  /** Append a turn to the inline conversation (of `sceneId` when given — a
   *  late reply for another scene is dropped, not appended to this one). */
  pushConvo(role: 'user' | 'assistant', text: string, sceneId?: string | null): void {
    if (sceneId !== undefined && sceneId !== this.currentId) return;
    this.convo = [...this.convo, { role, text, ts: Date.now() }];
  }

  /** Update a scene's metadata (title / section / provider) via PUT, then refresh
   *  the list. Keeps the open scene's local state in sync. */
  async updateMeta(
    id: string,
    patch: { title?: string; section?: string | null; provider?: string; story_id?: string },
  ): Promise<void> {
    await api.put(`/canvas/scenes/${id}`, patch);
    if (this.currentId === id) {
      if (patch.title != null && this.scene) this.scene = { ...this.scene, title: patch.title };
      if (patch.provider != null) this.provider = patch.provider;
    }
    await this.loadScenes().catch(() => {});
  }

  /** Persist a freshly-serialized Mermaid SOURCE (from a MANUAL edit on the
   *  board) as the scene's doc — same file the agent edits. `positions` carries
   *  the hand-arranged node coordinates (Mermaid has none) so layout survives a
   *  reload. */
  async saveSource(
    source: string,
    positions?: Record<string, { x: number; y: number }>,
  ): Promise<void> {
    const id = this.currentId;
    if (!id) return;
    this.source = source;
    const doc = {
      type: 'otto-canvas',
      version: 1,
      format: this.format,
      source,
      ...(positions ? { positions } : {}),
    };
    this.rawDoc = doc;
    try {
      await api.put(`/canvas/scenes/${id}`, { doc });
      this.savedAt = Date.now();
      this.dirty = false;
    } catch {
      this.dirty = true; // a later edit reschedules
    }
  }

  /** Editor → store: the canonical scene changed via direct manipulation. */
  commitFromEditor(next: Scene): void {
    if (!this.scene) return;
    this.#pushHistory(this.scene);
    this.scene = next;
    this.dirty = true;
    this.#scheduleSave();
  }

  /** External mutation (assist/template/rename): update + bump rev to reload. */
  setScene(next: Scene, recordHistory = true): void {
    if (recordHistory && this.scene) this.#pushHistory(this.scene);
    this.scene = next;
    this.dirty = true;
    this.rev += 1;
    this.#scheduleSave();
  }

  rename(title: string): void {
    if (!this.scene) return;
    this.setScene({ ...this.scene, title }, true);
  }

  /** Insert assist output near a viewport point. Returns inserted node count. */
  insertAssist(res: AssistResult, ox: number, oy: number): number {
    if (!this.scene) return 0;
    const { nodes, edges } = assistToNodes(res, ox, oy);
    if (!nodes.length) return 0;
    this.setScene(
      {
        ...this.scene,
        nodes: [...this.scene.nodes, ...nodes],
        edges: [...this.scene.edges, ...edges],
      },
      true,
    );
    return nodes.length;
  }

  #pushHistory(prev: Scene): void {
    this.#history.push(JSON.stringify(prev));
    if (this.#history.length > HISTORY_CAP) this.#history.shift();
    this.#future = [];
  }

  undo(): void {
    if (!this.scene || !this.#history.length) return;
    const snap = this.#history.pop()!;
    this.#future.push(JSON.stringify(this.scene));
    this.scene = JSON.parse(snap) as Scene;
    this.dirty = true;
    this.rev += 1;
    this.#scheduleSave();
  }

  redo(): void {
    if (!this.scene || !this.#future.length) return;
    const snap = this.#future.pop()!;
    this.#history.push(JSON.stringify(this.scene));
    this.scene = JSON.parse(snap) as Scene;
    this.dirty = true;
    this.rev += 1;
    this.#scheduleSave();
  }

  #scheduleSave(): void {
    if (this.#saveTimer) clearTimeout(this.#saveTimer);
    this.#saveTimer = setTimeout(() => void this.saveNow(), AUTOSAVE_MS);
  }

  async saveNow(): Promise<void> {
    if (this.currentId && this.#drafts.has(this.currentId)) {
      await this.retryDoc(this.currentId);
      return;
    }
    if (!this.currentId || !this.scene || this.saving) return;
    this.saving = true;
    try {
      await api.put<CanvasScene>(`/canvas/scenes/${this.currentId}`, {
        title: this.scene.title,
        doc: this.scene,
      });
      this.dirty = false;
      this.savedAt = Date.now();
    } catch {
      // keep dirty; a later edit reschedules. (Surface via toast at call site.)
    } finally {
      this.saving = false;
    }
  }

  // -- agent assist ------------------------------------------------------
  async assist(prompt: string, mode: AssistMode = 'auto'): Promise<AssistResult> {
    const body = { prompt, mode };
    if (this.currentId) {
      return api.post<AssistResult>(`/canvas/scenes/${this.currentId}/assist`, body);
    }
    return api.post<AssistResult>(`/canvas/assist/preview`, body);
  }
}

export const canvas = new CanvasStore();

if (typeof window !== 'undefined') {
  window.addEventListener('otto:auth-changed', () => canvas.resetAuthContext());
}
