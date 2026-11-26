// Older goal loops store (goal.md §5.2): the read-only history's list and
// open detail. Goal Loops are now goals in Workflows: no loop runs any more, so
// nothing here starts, steers or polls one; Delete history is the one write.

import { api } from '../api/client';
import { loadErrorText } from '../loadError';
import type { GoalLoop, GoalLoopDetail, GoalLoopIteration } from '../api/types';
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
  /** Single-flight detail loads: a burst of events queues ONE rerun. */
  private detailInflight: Promise<void> | null = null;
  private detailRerun: string | null = null;
  private detailId: string | null = null;
  private detailGeneration = 0;
  private lastDetailJson = '';
  /** Every detail fetch takes a ticket: a slower answer never lands last. */
  private detailSeq = latestOnly();

  /** The workspace the list was last loaded for. */
  listWs = '';
  private listGeneration = 0;

  async loadList(workspaceId: string): Promise<void> {
    // Another workspace's loops are not "stale data" for this one.
    if (this.listWs !== workspaceId) { this.list = []; this.listError = null; }
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
    if (this.detailId !== id) {
      this.detailId = id;
      this.detailGeneration++;
      this.detailSeq.cancel();
      this.detail = null;
      this.fullIterations = {};
      this.detailError = null;
    }
    if (this.detailInflight) {
      this.detailRerun = id;
      // Settle after the queued rerun, so `await loadDetail()` after an action
      // never resolves on a response that predates it.
      return this.detailInflight.then(() => this.detailInflight ?? undefined);
    }
    const p = this.fetchDetail(id).then(() => undefined).finally(() => {
      this.detailInflight = null;
      const next = this.detailRerun;
      this.detailRerun = null;
      if (next) void this.loadDetail(next);
    });
    this.detailInflight = p;
    return p;
  }

  /** Resolves false on failure; never rejects. */
  private async fetchDetail(id: string, signal?: AbortSignal): Promise<boolean> {
    const t = this.detailSeq.begin();
    this.loadingDetail = true;
    try {
      const path = `/goal-loops/${id}?summary=true`;
      const d = await api.get<GoalLoopDetail>(path, signal);
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
    const generation = this.detailGeneration;
    const full = await api.get<GoalLoopIteration>(`/goal-loops/${loopId}/iterations/${it.idx}`, signal);
    if (generation === this.detailGeneration) this.fullIterations = { ...this.fullIterations, [key]: full };
    return full;
  }

  closeDetail(): void {
    this.detailId = null;
    this.detailRerun = null;
    this.detailGeneration++;
    this.detailSeq.cancel();
    this.loadingDetail = false;
    this.detail = null;
    this.lastDetailJson = '';
    this.fullIterations = {};
    this.detailError = null;
  }

  async remove(id: string): Promise<void> {
    await api.del(`/goal-loops/${id}`);
    this.list = this.list.filter((l) => l.id !== id);
    if (this.detail?.loop.id === id) this.closeDetail();
  }
}

export const loops = new LoopsStore();
