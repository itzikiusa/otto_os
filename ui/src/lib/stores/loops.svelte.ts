// Goal Loops store: list + open-detail state, REST loaders, lifecycle actions,
// and live-event application. Intentionally does NOT import events.svelte.ts —
// the event dispatcher calls `loops.applyEvent(...)` on this singleton (matching
// the swarm store), so there is no import cycle.

import { api } from '../api/client';
import { loadErrorText } from '../loadError';
import type {
  CreateGoalLoopReq,
  DefineGoalReq,
  GoalLoop,
  GoalLoopDetail,
  GoalLoopDraft,
  OttoEvent,
} from '../api/types';

class LoopsStore {
  list: GoalLoop[] = $state([]);
  detail: GoalLoopDetail | null = $state(null);
  loadingList = $state(false);
  /** Last list-load failure (human text) — shown inline with Retry, never as "no loops". */
  listError = $state<string | null>(null);
  loadingDetail = $state(false);
  /** Bumped whenever the open detail should be considered stale (event tick). */
  tick = $state(0);

  private pollTimer: ReturnType<typeof setInterval> | null = null;

  private listWs = '';

  async loadList(workspaceId: string): Promise<void> {
    // Another workspace's loops are not "stale data" for this one.
    if (this.listWs !== workspaceId) this.list = [];
    this.listWs = workspaceId;
    this.loadingList = true;
    try {
      const list = await api.get<GoalLoop[]>(`/workspaces/${workspaceId}/goal-loops`);
      // A slower load for a workspace we've since left must not land here.
      if (this.listWs !== workspaceId) return;
      this.list = list;
      this.listError = null;
    } catch (e) {
      if (this.listWs === workspaceId) this.listError = loadErrorText(e);
    } finally {
      if (this.listWs === workspaceId) this.loadingList = false;
    }
  }

  async loadDetail(id: string): Promise<void> {
    this.loadingDetail = true;
    try {
      this.detail = await api.get<GoalLoopDetail>(`/goal-loops/${id}`);
    } catch {
      // leave the prior detail in place on a transient failure
    } finally {
      this.loadingDetail = false;
    }
  }

  closeDetail(): void {
    this.detail = null;
    this.stopPoll();
  }

  async define(workspaceId: string, req: DefineGoalReq): Promise<GoalLoopDraft> {
    return api.post<GoalLoopDraft>(`/workspaces/${workspaceId}/goal-loops/define`, req);
  }

  async create(workspaceId: string, req: CreateGoalLoopReq): Promise<GoalLoop> {
    const loop = await api.post<GoalLoop>(`/workspaces/${workspaceId}/goal-loops`, req);
    await this.loadList(workspaceId);
    return loop;
  }

  async start(id: string): Promise<void> {
    await this.lifecycle(id, 'start');
  }
  async pause(id: string): Promise<void> {
    await this.lifecycle(id, 'pause');
  }
  async resume(id: string): Promise<void> {
    await this.lifecycle(id, 'resume');
  }
  async stop(id: string): Promise<void> {
    await this.lifecycle(id, 'stop');
  }

  async verifyCriterion(id: string, criterion: string, evidence: string): Promise<void> {
    await api.post(`/goal-loops/${id}/criteria/${encodeURIComponent(criterion)}/verify`, { evidence });
    await this.loadDetail(id);
  }
  async answerQuestion(id: string, question: string, answer: string): Promise<void> {
    await api.post(`/goal-loops/${id}/questions/${question}/answer`, { answer });
    await this.loadDetail(id);
  }

  async retryExecutor(id: string, iterIdx: number, agentIndex: number): Promise<void> {
    await api.post(`/goal-loops/${id}/iterations/${iterIdx}/agents/${agentIndex}/retry`);
    await this.loadDetail(id);
  }

  async remove(id: string): Promise<void> {
    await api.del(`/goal-loops/${id}`);
    this.list = this.list.filter((l) => l.id !== id);
    if (this.detail?.loop.id === id) this.closeDetail();
  }

  private async lifecycle(id: string, action: 'start' | 'pause' | 'resume' | 'stop'): Promise<void> {
    const updated = await api.post<GoalLoop>(`/goal-loops/${id}/${action}`);
    this.mergeLoop(updated);
    if (this.detail?.loop.id === id) await this.loadDetail(id);
  }

  /** Apply a `goal_loop_updated` WS event: patch the list row and, when the open
   *  detail matches, re-fetch it (the event carries only summary fields). */
  applyEvent(ev: Extract<OttoEvent, { type: 'goal_loop_updated' }>): boolean {
    const row = this.list.find((l) => l.id === ev.loop_id);
    if (row) {
      row.status = ev.status;
      row.phase = ev.phase;
      row.current_iteration = ev.current_iteration;
      row.progress_pct = ev.progress_pct;
    }
    if (this.detail?.loop.id === ev.loop_id) {
      this.tick += 1;
      void this.loadDetail(ev.loop_id);
    }
    return true;
  }

  private mergeLoop(loop: GoalLoop): void {
    const i = this.list.findIndex((l) => l.id === loop.id);
    if (i >= 0) this.list[i] = loop;
    else this.list = [loop, ...this.list];
  }

  /** Low-frequency fallback poll for the open detail while it is active — covers
   *  any missed WS event (mirrors the Review panel's poll). */
  startPoll(id: string): void {
    this.stopPoll();
    this.pollTimer = setInterval(() => {
      const d = this.detail;
      if (!d || d.loop.id !== id) {
        this.stopPoll();
        return;
      }
      const active = d.loop.status === 'running' || d.loop.status === 'paused';
      if (!active) return;
      void this.loadDetail(id);
    }, 4000);
  }

  stopPoll(): void {
    if (this.pollTimer !== null) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }
}

export const loops = new LoopsStore();
