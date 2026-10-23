// The API client's automations (collection runner): the automations list,
// run history, and the live run the editor follows. Split out of
// apiClient.svelte.ts (arch-6-03); `apiClient` stays the one facade the UI
// imports and forwards these fields and methods here. The host gives the
// workspace's API base and the publication-ownership token, so a response
// that lands after the workspace changed never touches the new one.

import { api } from '../../api/client';
import { toasts } from '../../toast.svelte';
import { loadErrorText } from '../../loadError';
import type {
  ApiAutomation, ApiAutomationRun, ApiRunResult, Id, StartApiAutomationRunReq, UpsertApiAutomationReq,
} from '../../api/types';

/** Fallback delta-poll interval for a running automation; progress normally
 *  arrives as `api_run_progress` WS events (perf F2). */
const RUN_POLL_FALLBACK_MS = 2000;
/** Re-entering the API page within this long of a successful load reuses it
 *  (the store's own lists use the same window). */
const RELOAD_FRESH_MS = 60_000;
/** Minimum gap between a running automation's delta fetches: a fast run
 *  emits `api_run_progress` per step; the view refetches at most 4×/s
 *  (a terminal status skips the wait — perf2 N3). */
const RUN_DELTA_MIN_MS = 250;
/** A failed delta poll is retried after this (doubling per consecutive
 *  failure); after RUN_POLL_MAX_FAILURES in a row the view gives up on the
 *  run — it is still running server-side, so that is "lost track", never a
 *  failed run. */
const RUN_POLL_RETRY_MS = 500;
const RUN_POLL_MAX_FAILURES = 5;

export interface AutomationsHost {
  /** `/workspaces/{wid}/api-client`, or null with no workspace. */
  base(): string | null;
  /** Capture publication ownership (see `ApiClientStore.workspaceOwner`). */
  workspaceOwner(): () => boolean;
}

export class ApiAutomations {
  automations: ApiAutomation[] = $state([]);
  /** The automations list is loading / failed to (shown inline with Retry —
   *  a failed load must never read as "No automations yet"). */
  automationsLoading = $state(false);
  automationsError: string | null = $state(null);
  /** Last runAutomation() report, shown in the run panel. */
  // Raw states: run records are replaced wholesale (a 1,000-step report was
  // deep-proxied on every 500 ms poll).
  lastRun: ApiRunResult | null = $state.raw(null);
  /** In-flight automation run. */
  running = $state(false);
  automationRuns: ApiAutomationRun[] = $state.raw([]);
  /** Run history is loading / failed to (inline, with Retry). */
  runsLoading = $state(false);
  runsError: string | null = $state(null);
  currentRun: ApiAutomationRun | null = $state.raw(null);
  /** The run whose progress polls kept failing: the view stopped following
   *  it (it may still be running) — not a failed run. */
  runLost: Id | null = $state(null);
  /** When the list last loaded for which base (stale-while-revalidate). */
  automationsLoadedAt: { base: string; at: number } | null = null;

  constructor(private readonly host: AutomationsHost) {}

  /** A workspace switch: forget the previous workspace's run state. */
  resetRuns(): void {
    this.running = false; this.lastRun = null; this.currentRun = null; this.automationRuns = [];
    this.runLost = null; this.runsError = null; this.automationsError = null;
  }


  async loadAutomations(opts: { force?: boolean } = {}): Promise<void> {
    const current = this.host.workspaceOwner();
    const base = this.host.base();
    if (!base) return;
    const last = this.automationsLoadedAt;
    if (!opts.force && last?.base === base && Date.now() - last.at < RELOAD_FRESH_MS) return;
    this.automationsLoading = true;
    try {
      const rows = await api.get<ApiAutomation[]>(`${base}/automations`);
      if (!current()) return;
      this.automations = rows;
      this.automationsError = null;
      this.automationsLoadedAt = { base, at: Date.now() };
    } catch (e) {
      if (!current()) return;
      this.automationsError = loadErrorText(e);
    } finally {
      if (current()) this.automationsLoading = false;
    }
  }

  async saveAutomation(
    req: UpsertApiAutomationReq,
    id?: Id,
  ): Promise<ApiAutomation | null> {
    const current = this.host.workspaceOwner();
    const base = this.host.base();
    if (!base) return null;
    try {
      const saved = id
        ? await api.patch<ApiAutomation>(`${base}/automations/${id}`, req)
        : await api.post<ApiAutomation>(`${base}/automations`, req);
      if (!current()) return saved;
      this.automations = this.automations.some((a) => a.id === saved.id)
        ? this.automations.map((a) => (a.id === saved.id ? saved : a))
        : [...this.automations, saved];
      return saved;
    } catch (e) {
      if (!current()) return null;
      toasts.error('Couldn’t save automation', loadErrorText(e));
      return null;
    }
  }

  async deleteAutomation(id: Id): Promise<void> {
    const current = this.host.workspaceOwner();
    const base = this.host.base();
    if (!base) return;
    try {
      await api.del(`${base}/automations/${id}`);
      if (!current()) return;
      this.automations = this.automations.filter((a) => a.id !== id);
      if (this.lastRun?.automation_id === id) this.lastRun = null;
    } catch (e) {
      if (!current()) return;
      toasts.error('Couldn’t delete automation', loadErrorText(e));
    }
  }

  /** The in-flight `runAutomation` loop's wake-up, armed while it waits. */
  private runWake: { runId: Id; wake: (final: boolean) => void } | null = null;
  private runGeneration = 0;

  /** Progress that arrived while no waiter was armed (the delta GET was in
   *  flight) — latched so the loop doesn't sleep through it (perf2 N3). */
  private runPending: { runId: Id; final: boolean } | null = null;

  /** `api_run_progress` for run `runId`: wake the running view's delta fetch
   *  (or latch it for the loop's next wait). `status` other than `running`
   *  skips the loop's minimum gap. */
  noteRunProgress(runId: Id, status?: string): void {
    const final = !!status && status !== 'running';
    if (this.runWake?.runId === runId) this.runWake.wake(final);
    else if (this.running) {
      this.runPending = { runId, final: final || (this.runPending?.runId === runId && this.runPending.final) };
    }
  }

  /** Wait for the next progress of run `runId`: a WS wake (or one latched
   *  while the last GET was in flight), at least RUN_DELTA_MIN_MS after the
   *  previous fetch unless the run ended; RUN_POLL_FALLBACK_MS if the socket
   *  is quiet. */
  private async waitRunProgress(runId: Id, lastFetch: number): Promise<void> {
    let final = false;
    const pending = this.runPending?.runId === runId ? this.runPending : null;
    this.runPending = null;
    if (pending) final = pending.final;
    else {
      final = await new Promise<boolean>((resolve) => {
        const timer = setTimeout(() => done(false), RUN_POLL_FALLBACK_MS);
        const done = (f: boolean): void => { clearTimeout(timer); this.runWake = null; resolve(f); };
        this.runWake = { runId, wake: done };
      });
    }
    const gap = RUN_DELTA_MIN_MS - (performance.now() - lastFetch);
    if (!final && gap > 0) {
      // Coalesce: wakes during the gap fold into this fetch (a terminal one
      // cuts the gap short).
      await new Promise<void>((resolve) => {
        const timer = setTimeout(done, gap);
        function done(): void { clearTimeout(timer); resolve(); }
        this.runWake = { runId, wake: (f) => { if (f) done(); } };
      });
      this.runWake = null;
      this.runPending = null;
    }
  }

  /** One whole run (every step + snapshot) — run-list rows carry neither. */
  async getAutomationRun(id: Id): Promise<ApiAutomationRun | null> {
    const base = this.host.base(); if (!base) return null;
    try {return await api.get<ApiAutomationRun>(`${base}/automation-runs/${id}`);}
    catch (e) {toasts.error('Couldn’t load run',loadErrorText(e)); return null;}
  }

  async loadAutomationRuns(automationId?: Id, before?: Id): Promise<void> {
    const base = this.host.base(); if (!base) return;
    const query = new URLSearchParams();
    if (automationId) query.set('automation_id',automationId);
    if (before) query.set('before',before);
    // "Load older runs" over a loaded list keeps it: only a first page shows
    // the inline loading / error state.
    if (!before) this.runsLoading = true;
    try {
      const runs = await api.get<ApiAutomationRun[]>(`${base}/automation-runs?${query}`);
      if (base === this.host.base()) {
        this.automationRuns = before ? [...this.automationRuns,...runs] : runs;
        if (!before) this.runsError = null;
      }
    } catch (e) {
      if (base !== this.host.base()) return;
      if (before) toasts.error('Couldn’t load older runs',loadErrorText(e));
      else this.runsError = loadErrorText(e);
    } finally {
      if (!before && base === this.host.base()) this.runsLoading = false;
    }
  }
  async cancelAutomationRun(id: Id): Promise<void> {
    const base = this.host.base(); if (!base) return;
    try {await api.post(`${base}/automation-runs/${id}/cancel`,{});}
    catch (e) {toasts.error('Couldn’t cancel run',loadErrorText(e));}
  }
  async runAutomation(id: Id, options: StartApiAutomationRunReq = {}): Promise<ApiRunResult | null> {
    const base = this.host.base(); if (!base) return null;
    const ownsWorkspace = this.host.workspaceOwner();
    const generation = ++this.runGeneration;
    const current = () => ownsWorkspace() && this.runGeneration === generation;
    this.runWake?.wake(true);
    this.running = true;
    this.runLost = null;
    try {
      let run = await api.post<ApiAutomationRun>(`${base}/automations/${id}/runs`,options);
      if (!current()) return null;
      this.currentRun = run; this.lastRun = run.report;
      this.runPending = null;
      let lastFetch = 0;
      let pollFailures = 0;
      while (current()) {
        if (run.status !== 'running') {void this.loadAutomationRuns(id); return run.report;}
        await this.waitRunProgress(run.id, lastFetch);
        if (!current()) break;
        // Delta poll: only the steps after the ones we have (+ status); the
        // snapshot is not re-sent. Nothing new → no reassignment, no re-render.
        const have = run.report.steps.length;
        lastFetch = performance.now();
        let delta: ApiAutomationRun;
        try {
          delta = await api.get<ApiAutomationRun>(`${base}/automation-runs/${run.id}?after=${have}`);
          pollFailures = 0;
        } catch (e) {
          // One failed poll (a daemon blip, a timeout, a 5xx) is not a failed
          // run: keep following it with backoff, and only after repeated
          // failures say we lost track — never "Last run failed".
          if (!current()) return null;
          if (++pollFailures >= RUN_POLL_MAX_FAILURES) {
            this.runLost = run.id;
            toasts.error('Lost track of this run', `It may still be running. Refresh run history to see how it ended. (${loadErrorText(e)})`);
            void this.loadAutomationRuns(id);
            return null;
          }
          await new Promise<void>((resolve) => setTimeout(resolve, RUN_POLL_RETRY_MS * 2 ** (pollFailures - 1)));
          continue;
        }
        if (!current()) return null;
        if (delta.report.steps.length === 0 && delta.status === run.status && delta.error === run.error) continue;
        // Append in place: the arrays are this loop's own (raw state, so the
        // reassignment below is what re-renders) — O(delta), not O(run).
        const steps = run.report.steps;
        for (const s of delta.report.steps) steps.push(s);
        for (const r of delta.result_rows) run.result_rows.push(r);
        for (const r of delta.result_ids) run.result_ids.push(r);
        run = { ...delta, snapshot: run.snapshot, report: { ...delta.report, steps }, result_rows: run.result_rows, result_ids: run.result_ids };
        this.currentRun = run; this.lastRun = run.report;
      }
      return null;
    } catch (e) {if (current()) toasts.error('Couldn’t run automation',loadErrorText(e)); return null;}
    finally {if (current()) this.running = false;}
  }
}
