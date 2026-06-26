// Run with Otto store: the workspace run list, a per-run detail cache, and the
// open run's stage-event timeline. Like the loops/scheduled-tasks stores it does
// NOT import events.svelte.ts — the event dispatcher calls
// `runWithOtto.applyEvent(...)` on this singleton (a matching `otto_run_updated`
// tick re-fetches the affected run + the list). applyEvent is invoked ONLY from
// the WS dispatcher (never from a $derived), so its writes + refetches are safe
// — keep it that way to avoid the reactive-loop CPU footgun.

import { runWithOttoApi } from '../api/runWithOtto';
import type {
  ApproveRunReq,
  LaunchRunReq,
  OttoEvent,
  OttoRun,
  RunEvent,
} from '../api/types';

class RunWithOttoStore {
  /** The current workspace's runs (newest first). */
  list: OttoRun[] = $state([]);
  loadingList = $state(false);
  /** run_id → its full record (the open detail reads from here). */
  byId: Record<string, OttoRun> = $state({});
  /** run_id → its stage timeline (loaded when a run is opened). */
  eventsByRun: Record<string, RunEvent[]> = $state({});
  /** The run id whose detail panel is open, or null. */
  openId: string | null = $state(null);
  private wsId = '';

  /** The open run's record, if any (the detail panel reads this). */
  get openRun(): OttoRun | null {
    return this.openId ? (this.byId[this.openId] ?? null) : null;
  }

  async loadList(workspaceId: string): Promise<void> {
    this.wsId = workspaceId;
    this.loadingList = true;
    try {
      const runs = await runWithOttoApi.list(workspaceId);
      this.list = runs;
      const next = { ...this.byId };
      for (const r of runs) next[r.id] = r;
      this.byId = next;
    } catch {
      this.list = [];
    } finally {
      this.loadingList = false;
    }
  }

  /** Re-fetch a single run into the cache + patch it in the list in place. */
  async refreshRun(id: string): Promise<void> {
    try {
      const run = await runWithOttoApi.get(id);
      this.byId = { ...this.byId, [id]: run };
      this.list = this.list.map((r) => (r.id === id ? run : r));
    } catch {
      /* best-effort */
    }
  }

  /** Open a run's detail panel: cache it + load its stage timeline. */
  async open(id: string): Promise<void> {
    this.openId = id;
    await Promise.all([this.refreshRun(id), this.loadEvents(id)]);
  }

  closeDetail(): void {
    this.openId = null;
  }

  async loadEvents(id: string): Promise<void> {
    try {
      this.eventsByRun = { ...this.eventsByRun, [id]: await runWithOttoApi.events(id) };
    } catch {
      this.eventsByRun = { ...this.eventsByRun, [id]: [] };
    }
  }

  async launch(workspaceId: string, body: LaunchRunReq): Promise<OttoRun> {
    const run = await runWithOttoApi.launch(workspaceId, body);
    this.byId = { ...this.byId, [run.id]: run };
    await this.loadList(workspaceId);
    return run;
  }

  async approve(id: string, body: ApproveRunReq): Promise<void> {
    const run = await runWithOttoApi.approve(id, body);
    this.byId = { ...this.byId, [id]: run };
    this.list = this.list.map((r) => (r.id === id ? run : r));
    void this.loadEvents(id);
  }

  async cancel(id: string): Promise<void> {
    const run = await runWithOttoApi.cancel(id);
    this.byId = { ...this.byId, [id]: run };
    this.list = this.list.map((r) => (r.id === id ? run : r));
    void this.loadEvents(id);
  }

  /** Open the drafted PR for a run, then re-fetch it so `pr_url` shows. */
  async openPr(id: string): Promise<void> {
    await runWithOttoApi.openPr(id);
    await this.refreshRun(id);
    void this.loadEvents(id);
  }

  /** Live WS tick: re-fetch the affected run + the list status. Only the run's
   *  own workspace data is on screen, so ignore other workspaces' ticks. */
  applyEvent(ev: Extract<OttoEvent, { type: 'otto_run_updated' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    void this.refreshRun(ev.run_id);
    if (this.wsId) void this.loadList(this.wsId);
    if (this.openId === ev.run_id) void this.loadEvents(ev.run_id);
  }
}

export const runWithOtto = new RunWithOttoStore();
