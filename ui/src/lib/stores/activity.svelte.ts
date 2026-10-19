// Per-session agent activity: live trail + normalized task tracker, plus a
// workspace-wide per-session roll-up for the multi-agent overview.
//
// Fed by REST loads (when a session is focused / workspace selected) and the
// events WS (trail_appended / tasks_updated). Keyed by session id so switching
// the focused session is instant; trail entries accumulate only for sessions
// whose trail was loaded (the rest just bump their roll-up's recency).

import { api, getToken } from '../api/client';
import { loadErrorText } from '../loadError';
import { createLimiter } from '../poll';
import type {
  AgentTask,
  AppendTrailReq,
  Artifact,
  CreateAgentTaskReq,
  OttoEvent,
  SessionActivitySummary,
  TrailEvent,
} from '../api/types';

/** Progress of the background History index walk (`history_index_progress`). */
export interface HistoryIndexProgress {
  scanned: number;
  total: number;
  done: boolean;
}

/** Keep the trail bounded in memory (matches the server's list cap). */
const TRAIL_CAP = 500;

/** Snapshot requests own their deltas and may commit only while still current. */
interface ActivityRead {
  token: string | null;
  controller: AbortController;
  trail: TrailEvent[];
  tasks?: AgentTask[];
  addedTasks: Map<string, AgentTask>;
}
interface ArtifactRead {
  token: string | null;
  controller: AbortController;
  deltas: Map<string, Artifact>;
}

function mergeArtifacts(snapshot: Artifact[], deltas: Iterable<Artifact>): Artifact[] {
  const byId = new Map(snapshot.map(a => [a.id, a]));
  for (const a of deltas) {
    const old = byId.get(a.id);
    if (!old || (a.produced_at ?? '') >= (old.produced_at ?? '')) byId.set(a.id, a);
  }
  return [...byId.values()].sort((a, b) => (b.produced_at ?? '').localeCompare(a.produced_at ?? ''));
}

class ActivityStore {
  /** session id -> trail entries, oldest→newest */
  trailBySession: Record<string, TrailEvent[]> = $state({});
  /** session id -> task list, in display order */
  tasksBySession: Record<string, AgentTask[]> = $state({});
  /** session id -> task roll-up (for sidebar chips); kept fresh from events */
  summaryBySession: Record<string, SessionActivitySummary> = $state({});
  /** session id -> artifacts the transcript folder registered (Outputs panel) */
  artifactsBySession: Record<string, Artifact[]> = $state({});
  /** Artifact fetch states stay scoped to the session while panels switch. */
  artifactsLoadingBySession: Record<string, boolean> = $state({});
  artifactsErrorBySession: Record<string, string> = $state({});
  /** Latest History-index walk progress; null until a walk reports in. Reset to
   *  `{…, done:false}` when a rescan is requested so the button can show it. */
  historyIndex: HistoryIndexProgress | null = $state(null);
  /** session id -> why its trail/tasks load failed (cleared on success), so
   *  the Activity panel shows "Couldn’t load" + Retry, not an empty board. */
  loadErrorBySession: Record<string, string> = $state({});
  /** session ids we've already fetched once (avoid refetch churn) */
  private loaded = new Set<string>();
  private artifactsLoaded = new Set<string>();
  private reads = new Map<string, ActivityRead>();
  private artifactReads = new Map<string, ArtifactRead>();
  private epoch = 0;
  private recoveryGeneration = 0;
  private removedRevision = new Map<string, number>();
  private summaryRead = 0;
  private summaryPending = false;
  private summaryDeltas = new Map<string, Partial<SessionActivitySummary>>();
  private watched = new Map<string, { workspaceId: string; refs: number }>();
  private watchedArtifacts = new Map<string, number>();
  private recover = createLimiter(2);

  private invalidateReads(): void {
    for (const read of this.reads.values()) read.controller.abort();
    for (const read of this.artifactReads.values()) read.controller.abort();
    this.reads.clear();
    this.artifactReads.clear();
  }

  /** Mounted consumers register interest; gaps refresh only these sessions. */
  watch(workspaceId: string, sessionId: string): () => void {
    const entry = this.watched.get(sessionId) ?? { workspaceId, refs: 0 };
    entry.refs++;
    this.watched.set(sessionId, entry);
    void this.load(workspaceId, sessionId);
    return () => { if (--entry.refs === 0 && this.watched.get(sessionId) === entry) this.watched.delete(sessionId); };
  }

  watchArtifacts(sessionId: string): () => void {
    this.watchedArtifacts.set(sessionId, (this.watchedArtifacts.get(sessionId) ?? 0) + 1);
    void this.loadArtifacts(sessionId);
    return () => {
      const refs = (this.watchedArtifacts.get(sessionId) ?? 1) - 1;
      if (refs > 0) this.watchedArtifacts.set(sessionId, refs);
      else this.watchedArtifacts.delete(sessionId);
    };
  }

  /** Invalidate every snapshot after a gap; unopened caches rehydrate lazily. */
  async resync(): Promise<void> {
    this.loaded.clear();
    this.artifactsLoaded.clear();
    this.invalidateReads();
    this.artifactsLoadingBySession = {};
    const generation = ++this.recoveryGeneration;
    this.summaryRead++;
    this.summaryPending = false;
    this.summaryDeltas.clear();
    const token = getToken();
    const current = () => generation === this.recoveryGeneration && token === getToken();
    await Promise.all([
      ...[...this.watched].map(([id, entry]) => this.recover(async () => {
        if (current() && this.watched.get(id) === entry) await this.load(entry.workspaceId, id);
      })),
      ...[...this.watchedArtifacts.keys()].map(id => this.recover(async () => {
        if (current() && this.watchedArtifacts.has(id)) await this.loadArtifacts(id);
      })),
    ]);
  }

  trail(sessionId: string | null): TrailEvent[] {
    return sessionId ? (this.trailBySession[sessionId] ?? []) : [];
  }

  tasks(sessionId: string | null): AgentTask[] {
    return sessionId ? (this.tasksBySession[sessionId] ?? []) : [];
  }

  summary(sessionId: string | null): SessionActivitySummary | null {
    return sessionId ? (this.summaryBySession[sessionId] ?? null) : null;
  }

  /** Fetch a session's trail + tasks once (subsequent updates arrive via WS). */
  async load(workspaceId: string, sessionId: string, force = false): Promise<void> {
    if (!force && (this.loaded.has(sessionId) || this.reads.get(sessionId)?.token === getToken())) return;
    this.reads.get(sessionId)?.controller.abort();
    const read: ActivityRead = { token: getToken(), controller: new AbortController(), trail: [], addedTasks: new Map() };
    this.reads.set(sessionId, read);
    const current = () => this.reads.get(sessionId) === read && read.token === getToken();
    const base = `/workspaces/${workspaceId}/sessions/${sessionId}`;
    try {
      const [trail, tasks] = await Promise.all([
        api.get<TrailEvent[]>(`${base}/trail`, read.controller.signal),
        api.get<AgentTask[]>(`${base}/tasks`, read.controller.signal),
      ]);
      if (!current()) return;
      const merged = new Map(trail.map(e => [e.id, e]));
      for (const e of read.trail) merged.set(e.id, e);
      this.trailBySession[sessionId] = [...merged.values()].sort((a, b) => a.ts.localeCompare(b.ts)).slice(-TRAIL_CAP);
      const taskRows = new Map((read.tasks ?? tasks).map(t => [t.id, t]));
      for (const task of read.addedTasks.values()) if (!taskRows.has(task.id)) taskRows.set(task.id, task);
      this.tasksBySession[sessionId] = [...taskRows.values()];
      this.loaded.add(sessionId);
      if (this.loadErrorBySession[sessionId]) delete this.loadErrorBySession[sessionId];
    } catch (e) {
      if (!current()) return;
      this.loaded.delete(sessionId);
      this.loadErrorBySession[sessionId] = loadErrorText(e);
    } finally {
      if (this.reads.get(sessionId) === read) this.reads.delete(sessionId);
    }
  }

  /** Fetch the workspace-wide per-session roll-up (sidebar chips). */
  async loadSummary(workspaceId: string): Promise<void> {
    const epoch = this.epoch;
    const token = getToken();
    const revision = ++this.summaryRead;
    this.summaryPending = true;
    this.summaryDeltas.clear();
    try {
      const rows = await api.get<SessionActivitySummary[]>(
        `/workspaces/${workspaceId}/activity/summary`,
      );
      if (epoch !== this.epoch || token !== getToken() || revision !== this.summaryRead) return;
      const next: Record<string, SessionActivitySummary> = {};
      for (const r of rows) next[r.session_id] = r;
      for (const [id, delta] of this.summaryDeltas) {
        const live = this.summaryBySession[id];
        if (live) next[id] = { ...(next[id] ?? live), ...delta };
      }
      this.summaryBySession = next;
    } catch {
      /* best-effort */
    } finally {
      if (revision === this.summaryRead) {
        this.summaryPending = false;
        this.summaryDeltas.clear();
      }
    }
  }

  artifacts(sessionId: string | null): Artifact[] {
    return sessionId ? (this.artifactsBySession[sessionId] ?? []) : [];
  }

  /** Fetch a session's artifact list once (`artifact_added` keeps it fresh). */
  async loadArtifacts(sessionId: string, force = false): Promise<void> {
    if (!force && (this.artifactsLoaded.has(sessionId) || this.artifactReads.get(sessionId)?.token === getToken())) return;
    this.artifactReads.get(sessionId)?.controller.abort();
    const read: ArtifactRead = { token: getToken(), controller: new AbortController(), deltas: new Map() };
    this.artifactReads.set(sessionId, read);
    const current = () => this.artifactReads.get(sessionId) === read && read.token === getToken();
    this.artifactsLoadingBySession[sessionId] = true;
    delete this.artifactsErrorBySession[sessionId];
    try {
      const snapshot = await api.get<Artifact[]>(
        `/sessions/${sessionId}/artifacts`,
        read.controller.signal,
      );
      if (!current()) return;
      this.artifactsBySession[sessionId] = mergeArtifacts(snapshot, read.deltas.values());
      this.artifactsLoaded.add(sessionId);
    } catch (e) {
      if (!current()) return;
      this.artifactsLoaded.delete(sessionId);
      this.artifactsErrorBySession[sessionId] = loadErrorText(e);
    } finally {
      if (current()) this.artifactsLoadingBySession[sessionId] = false;
      if (this.artifactReads.get(sessionId) === read) this.artifactReads.delete(sessionId);
    }
  }

  /**
   * Push a task to the agent from the board / Activity panel
   * (`POST /sessions/{id}/tasks`, source=user, nudge_pending=1). The server
   * broadcasts `tasks_updated` with the merged list; we insert optimistically
   * so the row is visible before the frame lands (dedup by id on arrival).
   */
  async addTask(sessionId: string, title: string, description?: string): Promise<AgentTask> {
    const epoch = this.epoch;
    const token = getToken();
    const removed = this.removedRevision.get(sessionId);
    const body: CreateAgentTaskReq = description ? { title, description } : { title };
    const task = await api.post<AgentTask>(`/sessions/${sessionId}/tasks`, body);
    if (epoch !== this.epoch || token !== getToken() || removed !== this.removedRevision.get(sessionId)) return task;
    this.reads.get(sessionId)?.addedTasks.set(task.id, task);
    const list = this.tasksBySession[sessionId] ?? [];
    if (!list.some((t) => t.id === task.id)) {
      const next = [...list, task];
      this.tasksBySession[sessionId] = next;
      this.bumpSummary(sessionId, this.summarizeTasks(next));
    }
    return task;
  }

  /** Append a human note to a session's trail (source=user, kind=note). */
  async addNote(workspaceId: string, sessionId: string, summary: string): Promise<void> {
    const body: AppendTrailReq = { source: 'user', kind: 'note', summary };
    // The WS broadcast echoes it back, so we don't optimistically insert here.
    await api.post<TrailEvent>(`/workspaces/${workspaceId}/sessions/${sessionId}/trail`, body);
  }

  /** Route the activity-related WS events into the per-session maps. */
  applyEvent(ev: OttoEvent): boolean {
    switch (ev.type) {
      case 'trail_appended': {
        const pending = this.reads.get(ev.session_id);
        if (pending) pending.trail = [...pending.trail, ev.event].slice(-TRAIL_CAP);
        // Only sessions whose trail was fetched (an Activity panel / chat
        // opened them) keep entries. Every other session used to accumulate
        // 500 entries with their tool inputs in a deep $state record — for
        // every session that emitted while the window was open, in every
        // document (r3-05-03). Opening the panel loads the trail fresh anyway.
        if (!this.loaded.has(ev.session_id) && !pending) {
          this.bumpSummary(ev.session_id, { last_ts: ev.event.ts });
          return true;
        }
        const list = this.trailBySession[ev.session_id] ?? [];
        if (list.some((e) => e.id === ev.event.id)) return true;
        const next = [...list, ev.event];
        this.trailBySession[ev.session_id] =
          next.length > TRAIL_CAP ? next.slice(next.length - TRAIL_CAP) : next;
        // Keep the roll-up's recency fresh.
        this.bumpSummary(ev.session_id, { last_ts: ev.event.ts });
        return true;
      }
      case 'tasks_updated': {
        const pending = this.reads.get(ev.session_id);
        if (pending) pending.tasks = ev.tasks;
        this.tasksBySession[ev.session_id] = ev.tasks;
        this.bumpSummary(ev.session_id, this.summarizeTasks(ev.tasks));
        return true;
      }
      case 'artifact_added': {
        this.artifactReads.get(ev.session_id)?.deltas.set(ev.artifact.id, ev.artifact);
        const list = this.artifactsBySession[ev.session_id] ?? [];
        // Dedup is per path server-side (last producing turn wins) — replace.
        this.artifactsBySession[ev.session_id] = mergeArtifacts(list, [ev.artifact]);
        return true;
      }
      case 'history_index_progress': {
        this.historyIndex = { scanned: ev.scanned, total: ev.total, done: ev.done };
        return true;
      }
      default:
        return false;
    }
  }

  /** Derive a roll-up's task fields from a task list. */
  private summarizeTasks(tasks: AgentTask[]): Partial<SessionActivitySummary> {
    return {
      total: tasks.length,
      done: tasks.filter((t) => t.status === 'completed').length,
      in_progress: tasks.find((t) => t.status === 'in_progress')?.title ?? null,
    };
  }

  /** Merge a partial update into a session's roll-up, creating it if needed. */
  private bumpSummary(sessionId: string, patch: Partial<SessionActivitySummary>): void {
    if (this.summaryPending) this.summaryDeltas.set(sessionId, { ...this.summaryDeltas.get(sessionId), ...patch });
    const cur = this.summaryBySession[sessionId] ?? {
      session_id: sessionId,
      total: 0,
      done: 0,
      in_progress: null,
      last_ts: null,
    };
    this.summaryBySession[sessionId] = { ...cur, ...patch };
  }

  /** Mark a History rescan as requested (progress frames overwrite this). */
  beginHistoryRescan(): void {
    this.historyIndex = { scanned: 0, total: 0, done: false };
  }

  /** The signed-in identity changed (S13-02): drop every cached trail, task
   *  list, roll-up and artifact list — they were read with the previous
   *  identity's token. Panels refetch through `load` / `loadSummary`. */
  reset(): void {
    this.epoch++;
    this.recoveryGeneration++;
    this.summaryRead++;
    this.summaryPending = false;
    this.removedRevision.clear();
    this.invalidateReads();
    this.summaryDeltas.clear();
    this.trailBySession = {};
    this.tasksBySession = {};
    this.summaryBySession = {};
    this.artifactsBySession = {};
    this.artifactsLoadingBySession = {};
    this.artifactsErrorBySession = {};
    this.loadErrorBySession = {};
    this.historyIndex = null;
    this.loaded.clear();
    this.artifactsLoaded.clear();
  }

  /** Drop a removed session's data. */
  forget(sessionId: string): void {
    this.removedRevision.set(sessionId, (this.removedRevision.get(sessionId) ?? 0) + 1);
    this.reads.get(sessionId)?.controller.abort();
    this.artifactReads.get(sessionId)?.controller.abort();
    this.reads.delete(sessionId);
    this.artifactReads.delete(sessionId);
    this.summaryRead++;
    this.summaryPending = false;
    this.summaryDeltas.delete(sessionId);
    delete this.trailBySession[sessionId];
    delete this.tasksBySession[sessionId];
    delete this.summaryBySession[sessionId];
    delete this.artifactsBySession[sessionId];
    delete this.artifactsLoadingBySession[sessionId];
    delete this.artifactsErrorBySession[sessionId];
    this.loaded.delete(sessionId);
    this.artifactsLoaded.delete(sessionId);
  }
}

export const activity = new ActivityStore();
