// Goal Loops store: list + open-detail state, REST loaders, lifecycle actions,
// and live-event application. Intentionally does NOT import events.svelte.ts —
// the event dispatcher calls `loops.applyEvent(...)` on this singleton (matching
// the swarm store), so there is no import cycle.

import { api } from '../api/client';
import { loadErrorText } from '../loadError';
import type { Poller } from '../poll';
import { liveQuery } from '../live';
import type {
  CreateGoalLoopReq,
  DefineGoalReq,
  GoalLoop,
  GoalLoopDetail,
  GoalLoopDraft,
  GoalLoopIteration,
  GoalLoopLimits,
  OttoEvent,
  UpdateGoalLoopReq,
} from '../api/types';
import { announceModule } from '../lazyModule';
import { latestOnly } from '../latest';

class LoopsStore {
  list: GoalLoop[] = $state([]);
  /** Raw: replaced wholesale per fetch (and only when it changed). The detail
   *  is the SUMMARY shape — older iterations' `plan`/`context_*` are blank and
   *  read on expand via {@link loadIteration} (backlog B6 / SE-08). */
  detail: GoalLoopDetail | null = $state.raw(null);
  /** Full iterations fetched on expand, keyed `${loopId}:${idx}:${status}` —
   *  a finished iteration never changes, a live one refetches per status. */
  fullIterations: Record<string, GoalLoopIteration> = $state.raw({});
  loadingList = $state(false);
  /** Last list-load failure (human text) — shown inline with Retry, never as "no loops". */
  listError = $state<string | null>(null);
  loadingDetail = $state(false);
  /** Last detail-load failure (human text) — shown inline with Retry. A prior
   *  detail for the SAME loop stays in place (transient failure); a failure
   *  opening a DIFFERENT loop clears it so the page never shows the wrong loop. */
  detailError = $state<string | null>(null);
  /** Bumped whenever the open detail should be considered stale (event tick). */
  tick = $state(0);

  private poller: Poller | null = null;
  /** Single-flight detail loads: a burst of events queues ONE rerun. */
  private detailInflight: Promise<void> | null = null;
  private detailRerun: string | null = null;
  private lastDetailJson = '';
  /** Every detail fetch (interactive or the background poll) takes a ticket:
   *  a poll response that was in flight when Pause/Resume/Stop re-fetched used
   *  to land LAST and put "Running · Pause" back for up to 30 s. */
  private detailSeq = latestOnly();

  /** The workspace the list was last loaded for. */
  listWs = '';
  private listGeneration = 0;
  private workspaceGeneration = 0;

  async loadList(workspaceId: string): Promise<void> {
    // Another workspace's loops are not "stale data" for this one.
    if (this.listWs !== workspaceId) { this.list = []; this.listError = null; this.workspaceGeneration++; }
    const generation = ++this.listGeneration;
    this.listWs = workspaceId;
    this.loadingList = true;
    try {
      const list = await api.get<GoalLoop[]>(`/workspaces/${workspaceId}/goal-loops`);
      // A slower load for a workspace we've since left must not land here.
      if (this.listWs !== workspaceId || generation !== this.listGeneration) return;
      this.list = list;
      this.listError = null;
    } catch (e) {
      if (this.listWs === workspaceId && generation === this.listGeneration) this.listError = loadErrorText(e);
    } finally {
      if (this.listWs === workspaceId && generation === this.listGeneration) this.loadingList = false;
    }
  }

  /** Load the open detail. Coalesced: while a load is in flight, further
   *  calls queue a single rerun (the latest id wins) instead of stacking. */
  loadDetail(id: string): Promise<void> {
    if (this.detailInflight) {
      this.detailRerun = id;
      // Settle after the queued rerun, so `await loadDetail()` after an action
      // never resolves on a response that predates it.
      return this.detailInflight.then(() => this.detailInflight ?? undefined);
    }
    const p = this.fetchDetail(id, false).then(() => undefined).finally(() => {
      this.detailInflight = null;
      const next = this.detailRerun;
      this.detailRerun = null;
      if (next) void this.loadDetail(next);
    });
    this.detailInflight = p;
    return p;
  }

  /** `background`: the poll's request lane (never starves interactive fetches).
   *  Resolves false on failure (the poller backs off); never rejects. */
  private async fetchDetail(id: string, background: boolean, signal?: AbortSignal): Promise<boolean> {
    const t = this.detailSeq.begin();
    this.loadingDetail = true;
    try {
      const path = `/goal-loops/${id}?summary=true`;
      const d = background
        ? await api.bg.get<GoalLoopDetail>(path, signal)
        : await api.get<GoalLoopDetail>(path, signal);
      // A newer fetch (or closeDetail) superseded this one: its answer wins.
      if (!t.current) return true;
      const json = JSON.stringify(d);
      // Unchanged (a quiet poll tick / duplicate event): keep the same object
      // so nothing downstream re-derives or re-renders.
      if (json !== this.lastDetailJson || this.detail?.loop.id !== id) {
        this.lastDetailJson = json;
        this.detail = d;
      }
      this.detailError = null;
      return true;
    } catch (e) {
      if (signal?.aborted || !t.current) return true;
      // Leave the prior detail in place on a transient failure of the same loop.
      if (this.detail && this.detail.loop.id !== id) this.detail = null;
      this.detailError = loadErrorText(e);
      return false;
    } finally {
      if (t.current) this.loadingDetail = false;
    }
  }

  /** Cached full iteration (plan/context bodies), if fetched. */
  fullIteration(loopId: string, it: GoalLoopIteration): GoalLoopIteration | null {
    return this.fullIterations[`${loopId}:${it.idx}:${it.status}`] ?? null;
  }

  /** Fetch one iteration's full bodies (on expand). */
  async loadIteration(loopId: string, it: GoalLoopIteration, signal?: AbortSignal): Promise<GoalLoopIteration> {
    const key = `${loopId}:${it.idx}:${it.status}`;
    const hit = this.fullIterations[key];
    if (hit) return hit;
    const full = await api.get<GoalLoopIteration>(`/goal-loops/${loopId}/iterations/${it.idx}`, signal);
    this.fullIterations = { ...this.fullIterations, [key]: full };
    return full;
  }

  closeDetail(): void {
    this.detailSeq.cancel();
    this.loadingDetail = false;
    this.detail = null;
    this.lastDetailJson = '';
    this.fullIterations = {};
    this.detailError = null;
    this.stopPoll();
  }

  async define(workspaceId: string, req: DefineGoalReq): Promise<GoalLoopDraft> {
    return api.post<GoalLoopDraft>(`/workspaces/${workspaceId}/goal-loops/define`, req);
  }

  async create(workspaceId: string, req: CreateGoalLoopReq): Promise<GoalLoop> {
    const generation = this.workspaceGeneration;
    const loop = await api.post<GoalLoop>(`/workspaces/${workspaceId}/goal-loops`, req);
    if (this.listWs === workspaceId && generation === this.workspaceGeneration) await this.loadList(workspaceId);
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

  /** Raise a paused/blocked/exhausted loop's limits (`PATCH /goal-loops/{id}
   *  {limits}`) — Resume alone re-exhausts at once when a cap was hit. */
  async updateLimits(id: string, limits: GoalLoopLimits): Promise<void> {
    const updated = await api.patch<GoalLoop>(`/goal-loops/${id}`, { limits } satisfies UpdateGoalLoopReq);
    this.mergeLoop(updated);
  }

  async verifyCriterion(id: string, criterion: string, evidence: string): Promise<void> {
    await api.post(`/goal-loops/${id}/criteria/${encodeURIComponent(criterion)}/verify`, { evidence });
    await this.loadDetail(id);
  }
  async answerQuestion(id: string, question: string, answer: string): Promise<void> {
    await api.post(`/goal-loops/${id}/questions/${encodeURIComponent(question)}/answer`, { answer });
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
    if (
      row &&
      (row.status !== ev.status ||
        row.phase !== ev.phase ||
        row.current_iteration !== ev.current_iteration ||
        row.progress_pct !== ev.progress_pct)
    ) {
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

  /** Low-frequency fallback poll for the open detail — covers any missed WS
   *  event. Only while the loop is RUNNING and the window is visible (shared
   *  poll helper: in-flight guard, hidden pause, backoff, abort on stop); a
   *  paused/finished loop changes only through events or the user's actions. */
  startPoll(id: string): void {
    this.stopPoll();
    // `goal_loop_updated` already re-fetches the open detail (applyEvent):
    // while the event socket is up this is only a 30 s safety net; the 4 s
    // cadence returns while it is down.
    this.poller = liveQuery({
      run: async (signal) => {
        const d = this.detail;
        if (!d || d.loop.id !== id) return;
        if (d.loop.status !== 'running') return;
        if (this.detailInflight) return;
        return this.fetchDetail(id, true, signal);
      },
      on: [],
      fallbackMs: 4000,
      safetyMs: 30_000,
      immediate: false,
    });
  }

  stopPoll(): void {
    this.poller?.stop();
    this.poller = null;
  }
}

export const loops = new LoopsStore();
// Routed by `peek()` in lib/events.svelte.ts (perf H1): let it see this store
// however it was first imported.
announceModule('loops', loops);
