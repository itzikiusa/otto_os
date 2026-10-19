// Canvas Studio store — scene list, the open scene's file-backed doc
// (Excalidraw / Mermaid / D2 source), queued per-scene saves, and agent
// assist. Reads `ws.currentId` only.
//
// The editors (ExcalidrawCanvas / MermaidCanvas / D2Canvas) stage every edit
// through `persistDoc`, which queues one PUT per scene so a delayed older write
// never overwrites a newer one. `scene` is only the parsed title/metadata view;
// `rev` bumps when a scene is (re)opened. (The earlier node-graph editor and
// its undo/redo save path were removed — nothing mounted them.)

import { api, getToken } from '../api/client';
import { NO_WORKSPACE } from '../labels';
import { ws } from './workspace.svelte';
import { defaultAgentProvider } from '../providers';
import { loadErrorText } from '../loadError';
import { emptyScene, parseScene } from '../../modules/canvas/scene';
import type {
  AssistMode,
  AssistResult,
  CanvasDoc,
  CanvasFormat,
  CanvasScene,
  CanvasSceneSummary,
  CanvasSceneVersion,
  Scene,
} from '../../modules/canvas/types';

/** One inline conversation turn with the canvas agent (client-side). */
export interface CanvasTurn {
  role: 'user' | 'assistant';
  text: string;
  ts: number;
}


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
  /** Bumps when a scene is (re)opened — the editors reload from it. */
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

  /** Queue a save of `doc`. Resolves `true` only when THIS doc's PUT landed
   *  (false when it was superseded/skipped) — the Excalidraw board learns
   *  which inline images the server now holds from it. */
  async persistDoc(id: string, doc: CanvasDoc, context = this.#saveContext): Promise<boolean> {
    if (context !== this.#saveContext) return false;
    // Captured before any await. open/close/auth bump the visit; assist bumps
    // the turn. A completion from an older visit must not touch the guard.
    const stamp = {
      generation: this.#sceneGeneration,
      turnId: this.#turnId,
      duringTurn: this.#turnEpoch !== null && this.#guardSceneId === id && this.#turnSceneId === id,
    };
    this.stageDoc(id, doc, context);
    const previous = this.#writes.get(id);
    let sent = false;
    const write = (async () => {
      await previous?.catch(() => {});
      if (context !== this.#saveContext) return;
      // Superseded while queued: a newer draft is already chained behind this
      // write and will carry the latest doc — skip sending a stale multi-MB body.
      if (this.#drafts.get(id) !== doc) return;
      await api.put(`/canvas/scenes/${id}?summary=true`, { doc }); // list row back, not the doc (SD-22)
      sent = true;
      if (context !== this.#saveContext) return;
      if (this.#drafts.get(id) !== doc) return;
      this.#drafts.delete(id);
      delete this.docSaveErrors[id];
      // The board update follows the scene that is open now. The assist guard
      // follows the visit + turn captured when this write was queued, so a
      // late scene-A PUT cannot pin or advance scene B.
      if (this.currentId === id) {
        if (typeof doc.source === 'string') this.source = doc.source;
        this.markSaved(doc);
      }
      this.noteLandedSave(id, doc, stamp);
    })();
    this.#writes.set(id, write);
    try { await write; }
    catch (e) {
      if (context !== this.#saveContext) return false;
      this.docSaveErrors[id] = loadErrorText(e); throw e;
    }
    finally { if (this.#writes.get(id) === write) this.#writes.delete(id); }
    return sent;
  }


  async retryDoc(id: string): Promise<void> {
    const doc = this.#drafts.get(id);
    if (doc) await this.persistDoc(id, doc);
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
    if (!wsId) throw new Error(NO_WORKSPACE);
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
      // `?files=ref`: Excalidraw images stay `otto-canvas-file:<sha>` refs —
      // the board fetches each once (immutable) instead of a multi-MB doc.
      const row = await api.get<CanvasScene>(`/canvas/scenes/${id}?files=ref`);
      if (sequence !== this.#openSequence) return;
      this.#resetSceneGuard();
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
    this.#resetSceneGuard();
    this.currentId = null;
    this.scene = null;
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
      const row = await api.get<CanvasScene>(`/canvas/scenes/${this.currentId}?files=ref`);
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
  ingestDoc(doc: CanvasDoc, sceneId?: string | null): boolean {
    if (sceneId !== undefined && sceneId !== this.currentId) return false;
    if (typeof doc.source !== 'string') return false;
    if (this.dirty) return false;
    if (!this.serverDocAllowed(doc.source)) return false;
    this.source = doc.source;
    if (doc.format) this.format = doc.format;
    this.rawDoc = doc;
    this.savedAt = Date.now();
    return true;
  }

  /** Highest `canvasDocBus.tick` already consumed. Plain field (NOT `$state`):
   *  it is bookkeeping, and reading it must not subscribe the live effect. */
  #consumedPushTick = 0;
  /** Scene that owns `#savedSource`, `#turnEpoch`, `#pinnedSource`, `#stopHold`. */
  #guardSceneId: string | null = null;
  /** Source of the last document this client successfully PUT on `#guardSceneId`. */
  #savedSource: string | null = null;
  #saveEpoch = 0;
  /** `saveEpoch` at the start of the open Ask Otto request. */
  #turnEpoch: number | null = null;
  /** Scene whose Ask Otto turn `#turnEpoch` belongs to. */
  #turnSceneId: string | null = null;
  /** Bumped on every assist and on every scene/auth visit change. */
  #turnId = 0;
  /** Bumped on open, close, and auth change. In-flight saves keep the old one. */
  #sceneGeneration = 0;
  /** After a save during a turn, a server doc with any other source is stale
   *  (a live preview, or a stop that echoed the pre-turn board). */
  #pinnedSource: string | null = null;
  /** A terminal stop is reloading the stored scene. Events and the stop body
   *  from before that read stay out until the reload settles. */
  #stopHold = false;

  /** Drop guard fields without invalidating saves already stamped for this visit.
   *  Used when Ask Otto or a stop adopts the scene that is already open. */
  #clearSceneGuard(): void {
    this.#guardSceneId = null;
    this.#savedSource = null;
    this.#saveEpoch = 0;
    this.#turnEpoch = null;
    this.#turnSceneId = null;
    this.#pinnedSource = null;
    this.#stopHold = false;
  }

  /** A different scene visit or login. Late callbacks from the previous one no-op. */
  #resetSceneGuard(): void {
    this.#sceneGeneration += 1;
    this.#turnId += 1;
    this.#clearSceneGuard();
  }

  private noteLandedSave(
    id: string,
    doc: CanvasDoc,
    stamp: { generation: number; turnId: number; duringTurn: boolean },
  ): void {
    if (stamp.generation !== this.#sceneGeneration) return;
    if (id !== this.currentId) return;
    if (this.#guardSceneId !== null && this.#guardSceneId !== id) return;
    const source = typeof doc.source === 'string' ? doc.source : null;
    if (source !== null) this.#savedSource = source;
    const ownsGuard = this.#guardSceneId === id;
    // A newer save on this same visit replaces the echo the guard allows.
    if (ownsGuard && source !== null && this.#pinnedSource !== null) this.#pinnedSource = source;
    const landedInTurn = stamp.duringTurn && stamp.turnId === this.#turnId && ownsGuard;
    if (landedInTurn || (ownsGuard && this.#stopHold)) {
      if (source !== null) this.#pinnedSource = source;
      this.#saveEpoch += 1;
      return;
    }
    // A PUT queued before Ask Otto must not look like a save made during it.
    if (this.#turnEpoch === null) this.#saveEpoch += 1;
  }

  /** A remote doc may replace the board. A save made while Ask Otto is running
   *  stays; only an echo of that saved source is applied. */
  private serverDocAllowed(source: string): boolean {
    if (this.#guardSceneId !== this.currentId) return true;
    if (this.#stopHold) return false;
    if (this.#pinnedSource !== null && source !== this.#pinnedSource) return false;
    if (
      this.#turnEpoch !== null &&
      this.#saveEpoch !== this.#turnEpoch &&
      this.#savedSource !== null &&
      source !== this.#savedSource
    ) {
      return false;
    }
    return true;
  }

  /** True when this turn must not be undone back to the pre-turn board. */
  assistKeptSaved(note?: string | null, sceneId?: string | null): boolean {
    const sameScene =
      (sceneId === undefined || sceneId === this.currentId) &&
      this.#guardSceneId === this.currentId;
    const savedDuring = sameScene && this.#turnEpoch !== null && this.#saveEpoch !== this.#turnEpoch;
    const n = note ?? '';
    return (
      savedDuring ||
      n.startsWith('Stopped') ||
      n.includes('Kept your manual edits') ||
      n.includes('saved changes were preserved')
    );
  }

  /** End the Ask Otto guard. A save during the turn keeps rejecting other sources
   *  until the next Ask Otto request. */
  finishAssistTurn(sceneId?: string | null): void {
    // No scene id finishes only the turn that is still current. A late callback
    // from scene A (with or without its id) must not pin or end scene B.
    if (this.#turnSceneId === null || this.#turnSceneId !== this.currentId) return;
    if (sceneId !== undefined && sceneId !== this.#turnSceneId) return;
    if (this.#guardSceneId !== this.currentId) return;
    if (this.#turnEpoch !== null && this.#saveEpoch !== this.#turnEpoch && this.#savedSource !== null) {
      this.#pinnedSource = this.#savedSource;
    }
    this.#turnEpoch = null;
  }

  /** Consume a live `canvas_updated` push exactly ONCE (r3-02-01). The live
   *  effect re-ran on every `dirty` flip (ingestDoc reads it), so after the
   *  user's second save the same, now-stale push was re-applied and the board
   *  reverted to the agent's old drawing. A push is spent the first time it
   *  is seen — even when skipped because the user had unsaved edits or it is
   *  for another scene (that scene reloads from the server when opened).
   *  Returns true when the doc was applied. */
  ingestPush(tick: number, sceneId: string, doc: unknown): boolean {
    if (!tick || tick <= this.#consumedPushTick) return false;
    this.#consumedPushTick = tick;
    if (sceneId !== this.currentId) return false;
    const d = doc as CanvasDoc | null;
    if (!d || typeof d.source !== 'string' || this.dirty) return false;
    return this.ingestDoc(d);
  }

  /** A scene's version history, newest first (C5; no documents). */
  async listVersions(id: string): Promise<CanvasSceneVersion[]> {
    return api.get<CanvasSceneVersion[]>(`/canvas/scenes/${id}/versions`);
  }

  /** Restore a version. Pending saves land first (so they can't overwrite the
   *  restore afterwards); unsaved local edits are dropped — the server keeps
   *  the replaced doc as a `restore` version, so this is undoable. The restore
   *  is itself a link in the per-scene write chain (S18-306): a save issued
   *  while it is in flight (keystrokes typed after clicking Restore) queues
   *  BEHIND it instead of racing it, and — since that save will land last —
   *  the editor keeps showing it rather than the restored doc. */
  async restoreVersion(id: string, versionId: string): Promise<void> {
    const context = this.#saveContext;
    const previous = this.#writes.get(id);
    const dropped = this.#drafts.get(id); // the unsaved edits the restore replaces
    const posted = (async () => {
      await previous?.catch(() => {});
      return api.post<CanvasScene>(`/canvas/scenes/${id}/versions/${versionId}/restore?files=ref`, {});
    })();
    const write = posted.then(() => {});
    write.catch(() => {}); // the failure surfaces via `posted`; the chain link must not go unhandled
    this.#writes.set(id, write);
    let restored: CanvasScene;
    try {
      restored = await posted;
    } finally {
      if (this.#writes.get(id) === write) this.#writes.delete(id);
    }
    if (context !== this.#saveContext) return;
    // A newer draft was staged during the restore: its save is queued behind
    // the restore and wins on disk, so it must win on screen too.
    const draft = this.#drafts.get(id);
    if (draft !== undefined && draft !== dropped) return;
    this.#drafts.delete(id);
    delete this.docSaveErrors[id];
    if (this.currentId !== id) return;
    let doc: CanvasDoc | null = null;
    try {
      doc = JSON.parse(restored.doc_json) as CanvasDoc;
    } catch {
      doc = null;
    }
    this.dirty = false;
    if (doc) this.ingestDoc(doc);
    this.savedAt = Date.parse(restored.updated_at) || Date.now();
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
    await api.put(`/canvas/scenes/${id}?summary=true`, patch);
    if (this.currentId === id) {
      if (patch.title != null && this.scene) this.scene = { ...this.scene, title: patch.title };
      if (patch.provider != null) this.provider = patch.provider;
    }
    await this.loadScenes().catch(() => {});
  }

  /** Flush the open scene's staged draft now (phone back-nav). The
   *  file-backed editors stage every edit through `persistDoc`, so `dirty`
   *  always means "a draft is queued" — there is no other save path. */
  async saveNow(): Promise<void> {
    if (this.currentId && this.#drafts.has(this.currentId)) await this.retryDoc(this.currentId);
  }

  // -- agent assist ------------------------------------------------------
  async assist(prompt: string, mode: AssistMode = 'auto'): Promise<AssistResult> {
    if (this.#guardSceneId !== this.currentId) this.#clearSceneGuard();
    this.#guardSceneId = this.currentId;
    this.#turnSceneId = this.currentId;
    this.#turnId += 1;
    this.#turnEpoch = this.#saveEpoch;
    this.#pinnedSource = null;
    const body = { prompt, mode };
    if (this.currentId) {
      return api.post<AssistResult>(`/canvas/scenes/${this.currentId}/assist`, body);
    }
    return api.post<AssistResult>(`/canvas/assist/preview`, body);
  }

  /** Mark the open scene's running Ask AI turn as stopped. The turn commits
   *  nothing of its own and the server re-broadcasts the document currently
   *  stored — a save made during the turn stays. Pair with an Esc interrupt
   *  of the agent's PTY. */
  async stopAssist(): Promise<boolean> {
    if (!this.currentId) return false;
    const r = await api.post<{ stopping: boolean }>(`/canvas/scenes/${this.currentId}/assist/stop`, {});
    return r.stopping;
  }

  /** After a terminal stop, load the scene the server has now. The stop
   *  response is the document read at stop time; a save that lands after that
   *  read is the one that stays. An in-flight save is waited out first. A save
   *  that lands during the read wins, and the fetched doc is dropped. */
  async reloadAfterStop(sceneId: string): Promise<boolean> {
    if (!sceneId || this.currentId !== sceneId) return false;
    if (this.#guardSceneId !== sceneId) this.#clearSceneGuard();
    this.#guardSceneId = sceneId;
    this.#stopHold = true;
    const context = this.#saveContext;
    try {
      await this.#writes.get(sceneId)?.catch(() => {});
      if (context !== this.#saveContext || this.currentId !== sceneId) return false;
      const epoch = this.#saveEpoch;
      const pending = this.#drafts.get(sceneId);
      const row = await api.get<CanvasScene>(`/canvas/scenes/${sceneId}?files=ref`);
      if (context !== this.#saveContext || this.currentId !== sceneId) return false;
      if (this.#saveEpoch !== epoch || this.dirty || this.#drafts.get(sceneId) !== pending) {
        if (this.#savedSource !== null) this.#pinnedSource = this.#savedSource;
        return false;
      }
      let doc: CanvasDoc | null = null;
      try {
        doc = JSON.parse(row.doc_json) as CanvasDoc;
      } catch {
        doc = null;
      }
      if (!doc || typeof doc.source !== 'string') return false;
      this.#stopHold = false;
      const applied = this.ingestDoc(doc, sceneId);
      if (applied && typeof doc.source === 'string') this.#pinnedSource = doc.source;
      return applied;
    } catch {
      return false;
    } finally {
      if (this.#guardSceneId === sceneId) this.#stopHold = false;
    }
  }
}

export const canvas = new CanvasStore();

if (typeof window !== 'undefined') {
  window.addEventListener('otto:auth-changed', () => canvas.resetAuthContext());
}
