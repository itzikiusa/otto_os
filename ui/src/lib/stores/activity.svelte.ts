// Per-session agent activity: live trail + normalized task tracker, plus a
// workspace-wide per-session roll-up for the multi-agent overview.
//
// Fed by REST loads (when a session is focused / workspace selected) and the
// events WS (trail_appended / tasks_updated). Keyed by session id so switching
// the focused session is instant and background sessions keep accumulating.

import { api } from '../api/client';
import type {
  AgentTask,
  AppendTrailReq,
  OttoEvent,
  SessionActivitySummary,
  TrailEvent,
} from '../api/types';

/** Keep the trail bounded in memory (matches the server's list cap). */
const TRAIL_CAP = 500;

class ActivityStore {
  /** session id -> trail entries, oldest→newest */
  trailBySession: Record<string, TrailEvent[]> = $state({});
  /** session id -> task list, in display order */
  tasksBySession: Record<string, AgentTask[]> = $state({});
  /** session id -> task roll-up (for sidebar chips); kept fresh from events */
  summaryBySession: Record<string, SessionActivitySummary> = $state({});
  /** session ids we've already fetched once (avoid refetch churn) */
  private loaded = new Set<string>();

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
    if (!force && this.loaded.has(sessionId)) return;
    this.loaded.add(sessionId);
    const base = `/workspaces/${workspaceId}/sessions/${sessionId}`;
    try {
      const [trail, tasks] = await Promise.all([
        api.get<TrailEvent[]>(`${base}/trail`),
        api.get<AgentTask[]>(`${base}/tasks`),
      ]);
      this.trailBySession[sessionId] = trail;
      this.tasksBySession[sessionId] = tasks;
    } catch {
      this.loaded.delete(sessionId);
    }
  }

  /** Fetch the workspace-wide per-session roll-up (sidebar chips). */
  async loadSummary(workspaceId: string): Promise<void> {
    try {
      const rows = await api.get<SessionActivitySummary[]>(
        `/workspaces/${workspaceId}/activity/summary`,
      );
      const next: Record<string, SessionActivitySummary> = {};
      for (const r of rows) next[r.session_id] = r;
      this.summaryBySession = next;
    } catch {
      /* best-effort */
    }
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
        this.tasksBySession[ev.session_id] = ev.tasks;
        this.bumpSummary(ev.session_id, this.summarizeTasks(ev.tasks));
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
    const cur = this.summaryBySession[sessionId] ?? {
      session_id: sessionId,
      total: 0,
      done: 0,
      in_progress: null,
      last_ts: null,
    };
    this.summaryBySession[sessionId] = { ...cur, ...patch };
  }

  /** Drop a removed session's data. */
  forget(sessionId: string): void {
    delete this.trailBySession[sessionId];
    delete this.tasksBySession[sessionId];
    delete this.summaryBySession[sessionId];
    this.loaded.delete(sessionId);
  }
}

export const activity = new ActivityStore();
