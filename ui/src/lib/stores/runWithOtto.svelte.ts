// Run with Otto store: the workspace run list, a per-run detail cache, and the
// open run's stage-event timeline. Like the loops/scheduled-tasks stores it does
// NOT import events.svelte.ts — the event dispatcher calls
// `runWithOtto.applyEvent(...)` on this singleton (a matching `otto_run_updated`
// tick re-fetches the affected run + the list). applyEvent is invoked ONLY from
// the WS dispatcher (never from a $derived), so its writes + refetches are safe
// — keep it that way to avoid the reactive-loop CPU footgun.

import { runWithOttoApi } from '../api/runWithOtto';
import { loadErrorText } from '../loadError';
import type {
  ApproveRunReq,
  LaunchRunReq,
  OttoEvent,
  OttoRun,
  RunEvent,
} from '../api/types';
import { latestOnly } from '../latest';
import { announceModule } from '../lazyModule';

class RunWithOttoStore {
  /** The current workspace's runs (newest first). */
  list: OttoRun[] = $state([]);
  loadingList = $state(false);
  /** Last list-load failure (human text) — shown inline with Retry, never as "no runs". */
  listError = $state<string | null>(null);
  /** run_id → its full record (the open detail reads from here). */
  byId: Record<string, OttoRun> = $state({});
  /** run_id → its stage timeline (loaded when a run is opened). */
  eventsByRun: Record<string, RunEvent[]> = $state({});
  /** run_id → why its stage timeline failed to load (absent = ok). */
  eventsError: Record<string, string> = $state({});
  /** The run id whose detail panel is open, or null. */
  openId: string | null = $state(null);
  private wsId = '';
  private workspaceGeneration = 0;
  private listSeq = latestOnly();
  private requestSeq = 0;
  private runRequests = new Map<string, number>();
  private eventRequests = new Map<string, number>();
  private mutations = new Map<string, number>();

  /** The open run's record, if any (the detail panel reads this). */
  get openRun(): OttoRun | null {
    return this.openId ? (this.byId[this.openId] ?? null) : null;
  }

  async loadList(workspaceId: string): Promise<void> {
    // Another workspace's runs are not "stale data" for this one.
    if (this.wsId !== workspaceId) {
      this.workspaceGeneration++;
      this.list = [];
      this.byId = {};
      this.eventsByRun = {};
      this.eventsError = {};
      this.openId = null;
      this.runRequests.clear();
      this.eventRequests.clear();
      this.mutations.clear();
    }
    const ticket = this.listSeq.begin();
    const started = this.requestSeq;
    this.wsId = workspaceId;
    this.loadingList = true;
    try {
      const runs = await runWithOttoApi.list(workspaceId);
      // A slower load for a workspace we've since left must not land here.
      if (!ticket.current) return;
      this.list = runs.map((run) => (this.runRequests.get(run.id) ?? 0) > started && this.byId[run.id] ? this.byId[run.id] : run);
      const next = { ...this.byId };
      for (const r of this.list) next[r.id] = r;
      this.byId = next;
      this.listError = null;
    } catch (e) {
      if (ticket.current) this.listError = loadErrorText(e);
    } finally {
      if (ticket.current) this.loadingList = false;
    }
  }

  /** Re-fetch a single run into the cache + patch it in the list in place. */
  async refreshRun(id: string): Promise<void> {
    const generation = this.workspaceGeneration;
    const request = ++this.requestSeq;
    this.runRequests.set(id, request);
    try {
      const run = await runWithOttoApi.get(id);
      if (generation !== this.workspaceGeneration || this.runRequests.get(id) !== request) return;
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
    const generation = this.workspaceGeneration;
    const request = ++this.requestSeq;
    this.eventRequests.set(id, request);
    const current = () => generation === this.workspaceGeneration && this.eventRequests.get(id) === request;
    try {
      const events = await runWithOttoApi.events(id);
      if (!current()) return;
      this.eventsByRun = { ...this.eventsByRun, [id]: events };
      const { [id]: _drop, ...rest } = this.eventsError;
      this.eventsError = rest;
    } catch (e) {
      if (current()) this.eventsError = { ...this.eventsError, [id]: loadErrorText(e) };
    }
  }

  async launch(workspaceId: string, body: LaunchRunReq): Promise<OttoRun> {
    const generation = this.workspaceGeneration;
    const run = await runWithOttoApi.launch(workspaceId, body);
    if (generation !== this.workspaceGeneration || workspaceId !== this.wsId) return run;
    this.byId = { ...this.byId, [run.id]: run };
    await this.loadList(workspaceId);
    return run;
  }

  async approve(id: string, body: ApproveRunReq): Promise<void> {
    await this.mutate(id, () => runWithOttoApi.approve(id, body));
  }

  async cancel(id: string): Promise<void> {
    await this.mutate(id, () => runWithOttoApi.cancel(id));
  }

  private async mutate(id: string, action: () => Promise<OttoRun>): Promise<void> {
    const generation = this.workspaceGeneration;
    const request = ++this.requestSeq;
    this.mutations.set(id, request);
    const run = await action();
    if (generation !== this.workspaceGeneration || this.mutations.get(id) !== request) return;
    // Reads started before the confirmed action cannot restore its old status.
    this.runRequests.set(id, ++this.requestSeq);
    this.byId = { ...this.byId, [id]: run };
    this.list = this.list.map((r) => (r.id === id ? run : r));
    void this.loadEvents(id);
  }

  /** Open the drafted PR for a run, then re-fetch it so `pr_url` shows. */
  async openPr(id: string): Promise<void> {
    const generation = this.workspaceGeneration;
    await runWithOttoApi.openPr(id);
    if (generation !== this.workspaceGeneration) return;
    await this.refreshRun(id);
    void this.loadEvents(id);
  }

  /** Live WS tick: re-fetch the affected run + the list status. Only the run's
   *  own workspace data is on screen, so ignore other workspaces' ticks — and
   *  a run this store holds nothing for while no list is loaded (perf H1). */
  applyEvent(ev: Extract<OttoEvent, { type: 'otto_run_updated' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    if (!this.wsId && !(ev.run_id in this.byId)) return;
    void this.refreshRun(ev.run_id);
    if (this.wsId) void this.loadList(this.wsId);
    if (this.openId === ev.run_id) void this.loadEvents(ev.run_id);
  }
}

export const runWithOtto = new RunWithOttoStore();
// Routed by `peek()` in lib/events.svelte.ts (perf H1): let it see this store
// however it was first imported.
announceModule('runWithOtto', runWithOtto);
