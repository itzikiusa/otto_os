// RunSync, the Workflows page's run state (architecture-1-08): the run on screen and
// everything that keeps it live — the WS fast path, the single-flight
// rev-guarded refetch, the safety poll — plus the human-approval gate (its
// publication preview and the approve/deny write) and the run's final output.
// Lifted out of WorkflowsPage, which keeps the canvas and the editor and
// assigns `run` when it opens or starts one. RunMonitor.svelte (the inspector's
// run view), RunApprovals.svelte and RunContextSidebar.svelte render from it.
//
// `track()` wires the effects to the page: call it once while the page
// initialises (a `$effect` needs the component's context).

import { untrack } from 'svelte';
import { api, ApiError } from '../../lib/api/client';
import { browseRunContext, readRunContext, workflowNodeDetail, workflowProgress } from '../../lib/api/workflows';
import type { NodeRunState, WorkflowRun } from '../../lib/api/types';
import { workflowRunBus } from '../../lib/events.svelte';
import { liveQuery } from '../../lib/live';
import { loadErrorText } from '../../lib/loadError';
import { renderMarkdownGfm } from '../../lib/md';
import { ui } from '../../lib/stores/ui.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import type { ProductPublicationPreview } from '../product/types';
import { runContextPath } from './runContext';
import { applyLiveNode, mergeRunProgress } from './runProgress';

// ── final-output panel ──────────────────────────────────────────────────
// On a successful run, `<context_dir>/final-output.md` holds the run's
// deliverable (see workflow_context.rs's write_final_output). Rendered
// into a sandboxed iframe (srcdoc, no scripts run), never raw HTML in the
// page; the shared renderer also sanitizes it.
// The srcdoc is themed from the app's tokens (resolved values copied in,
// like PluginFrame's `otto:init` theme) so it reads in light AND dark; it
// re-renders when the theme/scheme/accent changes (`finalOutputSrcdoc`).
const FINAL_OUTPUT_CSS = `
    body { font: 14px/1.6 var(--font-ui); margin: 16px; color: var(--text); background: transparent; }
    h1,h2,h3 { line-height: 1.25; } h1,h2 { border-bottom: 1px solid var(--border); padding-bottom: .2em; }
    a { color: var(--accent-text); } code { background: var(--surface-2); padding: .15em .35em; border-radius: 4px; font-family: var(--font-mono); }
    pre { background: var(--surface-2); padding: 12px; border-radius: 6px; overflow: auto; } pre code { background: none; padding: 0; }
    table { border-collapse: collapse; } th,td { border: 1px solid var(--border); padding: 4px 8px; }
    blockquote { border-inline-start: 3px solid var(--border-strong); margin: 0; padding-inline-start: 12px; color: var(--text-dim); }
    img { max-width: 100%; }
  `;
const FINAL_OUTPUT_TOKENS = ['--text', '--text-dim', '--border', '--border-strong', '--surface-2', '--accent-text', '--font-ui', '--font-mono'];
/** Markdown → HTML body (stored); the themed document is `finalOutputSrcdoc`. */
/** Through the shared GFM renderer (lib/md): sanitized on top of the
 *  sandbox, and it never throws (a parse failure falls back to the escaped
 *  tiny renderer). */
function renderFinalOutputSrcdoc(md: string): string {
  // `breaks`: final-output.md is an agent report written line by line
  // ("Status: done" / "Files: 3" / "PR: …"), not reflowed prose.
  return renderMarkdownGfm(md, { breaks: true });
}
function themedFinalOutput(body: string, scheme: 'light' | 'dark'): string {
  const cs = getComputedStyle(document.documentElement);
  const vars = FINAL_OUTPUT_TOKENS.map((v) => `${v}: ${cs.getPropertyValue(v).trim()};`).join(' ');
  return `<!doctype html><html><head><meta charset="utf-8"><style>
  :root { color-scheme: ${scheme}; ${vars} }${FINAL_OUTPUT_CSS}</style></head><body>${body}</body></html>`;
}

/** The Context panel's FileTree source: the run-scoped context routes. */
export function contextFs(runId: string, contextDir: string) {
  return {
    lister: (path: string, signal?: AbortSignal) => browseRunContext(runId, runContextPath(contextDir, path), signal),
    reader: (path: string, signal?: AbortSignal) => readRunContext(runId, runContextPath(contextDir, path), signal),
  };
}

/** A run that is still pending or running. */
export function isLive(status: string): boolean {
  return status === 'pending' || status === 'running';
}

/** Which approval write a gate is busy with. */
export type ApprovalKind = 'approve' | 'deny';

export class RunSync {
  /** The run on screen. The page assigns it when it opens or starts a run;
   *  live updates merge INTO it (see `applyRunSnapshot`). */
  run = $state<WorkflowRun | null>(null);

  // ── live run sync ──────────────────────────────────────────────────────────
  // ONE path keeps the viewed run live, whatever started it. Updates are
  // MERGED INTO the existing `run` object (never a wholesale replacement), so
  // everything the user is looking at — an expanded step, the timeline
  // selection, scroll positions — survives every tick. The run's monotonic
  // `rev` guards against stale/out-of-order snapshots ever regressing the view.
  //
  // Sources, all funneled through applyRunSnapshot / the WS fast-path:
  //   1. workflow_run_updated events carrying the changed node + a contiguous
  //      rev → merged in place, no network at all.
  //   2. Events with a rev gap / no node payload / terminal status → ONE
  //      single-flight, rev-guarded GET of the full run.
  //   3. A 2.5s fallback poll while the viewed run is non-terminal (missed
  //      events, or no WS connection).

  // Product approval previews are full node bodies; progress polling carries
  // only summaries. Load exactly this pending gate/version before approving.
  approvalPreview = $state<ProductPublicationPreview | null>(null);
  approvalPreviewLoading = $state(false);
  approvalPreviewError = $state('');
  approvalPreviewReady = $state(false);
  approvalPreviewVersion = $state<string | null>(null);
  private approvalPreviewSequence = 0;
  private approvalPreviewAbort: AbortController | null = null;
  approvalNode = $derived(this.run?.nodes.find(node => node.node_id === this.run?.approval_node_id));
  approvalNeedsPreview = $derived(!!(this.approvalNode?.has_output || this.approvalNode?.output));
  approvalBody = $derived(this.approvalPreview?.kind === 'jira' && this.approvalPreview.request.reviewed_content.source_kind === 'confluence' && this.approvalPreview.request.reviewed_content.url
    ? `> RFC: ${this.approvalPreview.request.reviewed_content.url}\n\n${this.approvalPreview.body_md}` : (this.approvalPreview?.body_md ?? ''));
  /** True while an approve/deny write is in flight (`approvingKind` says which). */
  approving = $state(false);
  approvingKind = $state<ApprovalKind>('approve');

  // Final-output panel: `<context_dir>/final-output.md`, fetched once per
  // successful run id. `finalOutputRunId` doubles as the "already attempted"
  // guard so a run with no deliverable doesn't get refetched forever.
  finalOutputRunId = $state<string | null>(null);
  finalOutputHtml = $state('');
  finalOutputAvailable = $state(false);
  /** A failed read that isn't a plain "no deliverable" (404): shown inline with Retry. */
  finalOutputError = $state<{ runId: string; text: string } | null>(null);
  finalOutputSrcdoc = $derived.by(() => {
    // Theme inputs are read so the frame re-themes when they change.
    void ui.theme;
    void ui.accent;
    return this.finalOutputHtml ? themedFinalOutput(this.finalOutputHtml, ui.resolvedScheme) : '';
  });

  /** Set once the page is torn down: no read or wait outlives it. */
  destroyed = false;

  // Single-flight refetch: at most one GET in the air; a request that arrives
  // while one is flying coalesces into one trailing fetch.
  private runRead: AbortController | null = null;
  /** The run `runRead` is fetching. A queued refetch only coalesces into a
   *  read for the SAME run: Start's catch-up refetch used to be queued behind
   *  the PREVIOUS run's read, which the run switch then aborted — dropping the
   *  catch-up and leaving the new run on "Queued" for up to 15 s. */
  private runReadFor: string | null = null;
  private runRefetchQueued = false;

  /** Wire the live sync, the approval preview and the final-output load to
   *  the page (call once during its init). */
  track(): void {
    $effect(() => () => {
      this.destroyed = true;
    });

    $effect(() => {
      const run = this.run;
      const id = run?.id, waiting = run?.waiting_approval, node = this.approvalNode;
      void node?.detail_version;
      if (id && waiting && node && this.approvalNeedsPreview) untrack(() => void this.loadApprovalPreview(id, node));
      else untrack(() => { this.approvalPreview = null; this.approvalPreviewReady = false; this.approvalPreviewVersion = null; this.approvalPreviewError = ''; this.approvalPreviewLoading = false; });
      return () => { this.approvalPreviewAbort?.abort(); };
    });

    $effect(() => {
      const id = this.run?.id;
      // Abort only THIS run's read: by the time the switch's cleanup runs,
      // `startRun` has already made `runRead` the NEW run's catch-up — aborting
      // it left a re-run on "Queued" until the 15 s poll (S17-301).
      return () => {if (id && this.runReadFor === id) {this.runRead?.abort(); this.runRead = null; this.runReadFor = null;}};
    });
    $effect(() => {
      const visibility = () => {
        if (document.hidden) {this.runRead?.abort();this.runRead = null;this.runReadFor = null;}
        else if (this.run) void this.refetchRun(this.run.id);
      };
      document.addEventListener('visibilitychange',visibility);
      return () => document.removeEventListener('visibilitychange',visibility);
    });

    // (1) WS fast-path: apply the event to the viewed run without refetching
    // when it carries the changed node and the very next rev.
    $effect(() => {
      const _tick = workflowRunBus.tick; // dependency: re-run on each WS event
      void _tick;
      untrack(() => this.applyBusEvent());
    });

    // (2) Guaranteed: a safety poll while the viewed run is non-terminal, so
    // the view still converges with no WS connection at all — 2.5 s while the
    // event socket is down, 15 s while `workflow_run_updated` flows (and a
    // refetch after a reconnect/lag resync). Shared chain: no overlap, paused
    // while hidden.
    $effect(() => {
      const cur = this.run;
      if (!cur) return;
      if (!isLive(cur.status)) return;
      const id = cur.id;
      const p = liveQuery({ run: () => this.refetchRun(id), on: [], fallbackMs: 2500, safetyMs: 15_000, immediate: false });
      return () => p.stop();
    });

    $effect(() => {
      const cur = this.run;
      if (!cur || cur.status !== 'success' || !cur.context_dir) return;
      const runId = cur.id;
      untrack(() => {
        if (this.finalOutputRunId === runId) return;
        void this.loadFinalOutput(runId);
      });
    });
  }

  /** Merge a fresh snapshot into the viewed run (rev-guarded in
   *  mergeRunProgress). Object fields (output/logs/sessions) are only
   *  reassigned when they change, so unchanged step subtrees don't re-render. */
  applyRunSnapshot = (nr: WorkflowRun): void => {
    if (this.run) mergeRunProgress(this.run, nr);
  };

  /** One workflow_run_updated event for the viewed run: applied in place
   *  when it is the contiguous next rev, else one refetch. */
  applyBusEvent(): void {
    const cur = this.run;
    if (!cur || workflowRunBus.runId !== cur.id) return;
    const evRev = workflowRunBus.rev;
    const curRev = cur.rev ?? 0;
    if (evRev > 0 && evRev <= curRev) return; // already have this state
    // Perf W5: the event carries the node's summary (the exact `/progress`
    // node shape, with its `detail_version`) — apply a contiguous one in
    // place. A gap, a status change, a new node or a checkpointed run still
    // polls the small projection.
    if (applyLiveNode(cur, {
      rev: evRev,
      status: workflowRunBus.status,
      node: workflowRunBus.node,
      waitingApproval: workflowRunBus.waitingApproval,
    })) return;
    void this.refetchRun(cur.id);
  }

  async refetchRun(runId: string): Promise<void> {
    if (this.destroyed || document.hidden || this.run?.id !== runId) return;
    if (this.runRead && this.runReadFor === runId) {this.runRefetchQueued = true; return;}
    this.runRead?.abort();
    const ac = new AbortController();
    this.runRead = ac;
    this.runReadFor = runId;
    try {
      do {
        this.runRefetchQueued = false;
        const result = await workflowProgress(runId,this.run?.summary ? this.run.rev : undefined,ac.signal);
        if (ac.signal.aborted || document.hidden || this.run?.id !== runId) return;
        if (result.changed) this.applyRunSnapshot(result.run);
      } while (this.runRefetchQueued && this.run?.id === runId && !this.destroyed);
    } catch { /* The visible fallback poll heals transient errors. */ }
    finally {if (this.runRead === ac) {this.runRead = null; this.runReadFor = null;}}
  }

  /** Await a run's terminal status WITHOUT driving the view: while the run is
   *  the one on screen, its live-merged state is authoritative (no network);
   *  once the user switches the view elsewhere, fall back to a cheap poll.
   *  This is what lets a user inspect another run while one is in flight —
   *  nothing here ever writes `run`. */
  async waitRunTerminal(runId: string): Promise<{ status: string; error?: string | null }> {
    // Off-screen: the run's WS events (workflowRunBus) say when it settles; a
    // rev-guarded `/progress?after_rev=` is only a slow safety net (every
    // OFFSCREEN_POLL_MS, skipped while the window is hidden). It used to GET
    // the full progress every second for the whole run — ~3.6k requests/hour.
    const OFFSCREEN_POLL_MS = 15_000;
    let lastRev: number | undefined;
    let lastStatus = 'running';
    let lastError: string | null | undefined = null;
    let nextFetch = 0;
    const fetchProgress = async () => {
      try {
        const g = await workflowProgress(runId, lastRev);
        if (g.changed) {
          lastRev = g.rev;
          lastStatus = g.run.status;
          lastError = g.run.error;
        }
      } catch {
        /* transient fetch error — keep waiting */
      }
      nextFetch = Date.now() + OFFSCREEN_POLL_MS;
    };
    for (;;) {
      if (this.destroyed) return { status: 'canceled', error: null };
      const cur = untrack(() => this.run);
      let status: string;
      let error: string | null | undefined;
      if (cur?.id === runId) {
        status = cur.status;
        error = cur.error;
      } else {
        const bus = untrack(() => ({ id: workflowRunBus.runId, status: workflowRunBus.status }));
        const busSettled = bus.id === runId && bus.status !== '' && !isLive(bus.status);
        // Settled per the bus → one read for the final status + error.
        if (busSettled || (!document.hidden && Date.now() >= nextFetch)) await fetchProgress();
        status = busSettled && isLive(lastStatus) ? bus.status : lastStatus;
        error = lastError;
      }
      if (!isLive(status)) return { status, error };
      await new Promise((res) => setTimeout(res, 1000));
    }
  }

  async loadApprovalPreview(runId: string, node: NodeRunState): Promise<void> {
    const sequence = ++this.approvalPreviewSequence;
    this.approvalPreviewAbort?.abort();
    const ctl = new AbortController(); this.approvalPreviewAbort = ctl;
    this.approvalPreview = null; this.approvalPreviewError = ''; this.approvalPreviewReady = false; this.approvalPreviewVersion = null; this.approvalPreviewLoading = true;
    const current = () => !ctl.signal.aborted && sequence === this.approvalPreviewSequence && this.run?.id === runId && this.run.waiting_approval && this.run.approval_node_id === node.node_id;
    try {
      const result = await workflowNodeDetail(runId, node.node_id, ctl.signal);
      if (!current()) return;
      if (node.detail_version && result.detail_version !== node.detail_version) throw new Error('The approval preview changed. Retry to load the current review.');
      const output = result.body.output as { publication_preview?: ProductPublicationPreview } | null;
      const preview = output?.publication_preview;
      if (preview && ((preview.kind !== 'jira' && preview.kind !== 'rfc') || typeof preview.body_md !== 'string' || !preview.request?.reviewed_content)) throw new Error('The publication preview is incomplete. Run a new preview before approving.');
      this.approvalPreview = preview ?? null; this.approvalPreviewVersion = result.detail_version; this.approvalPreviewReady = true;
    } catch (error) {
      if (current()) this.approvalPreviewError = loadErrorText(error);
    } finally { if (current()) this.approvalPreviewLoading = false; }
  }

  /** Retry the preview of the gate on screen. */
  retryApprovalPreview(): void {
    if (this.run && this.approvalNode) void this.loadApprovalPreview(this.run.id, this.approvalNode);
  }

  /** Whether approving waits on the publication preview (not loaded, failed
   *  or stale). */
  get approvalBlocked(): boolean {
    return this.approvalNeedsPreview && (!this.approvalPreviewReady || !this.approvalPreviewVersion || this.approvalPreviewLoading || !!this.approvalPreviewError);
  }

  async approveRun(approved: boolean, note?: string | null): Promise<void> {
    const run = this.run;
    if (!run?.waiting_approval || !run.approval_node_id || this.approving) return;
    if (this.approvalBlocked) return;
    const runId = run.id, nodeId = run.approval_node_id;
    const reviewedVersion = this.approvalNeedsPreview ? this.approvalPreviewVersion : null;
    this.approving = true;
    this.approvingKind = approved ? 'approve' : 'deny';
    try {
      await api.post(`/workflow-runs/${runId}/approve`, {
        node_id: nodeId,
        ...(reviewedVersion ? { expected_detail_version: reviewedVersion } : {}),
        approved,
        ...(note ? { note } : {}),
      });
      toasts.success(approved ? 'Approved — run resuming' : 'Denied — the run will stop with an error');
    } catch (e) {
      if (reviewedVersion && this.run?.id === runId && this.run.approval_node_id === nodeId && (e as {status?: number})?.status === 409) {
        this.approvalPreviewReady = false; this.approvalPreviewVersion = null;
        this.approvalPreviewError = 'The publication preview changed. Select Retry and review it again before deciding.';
        void this.refetchRun(runId);
      }
      toastError('Couldn’t record the approval', e);
    } finally {
      this.approving = false;
    }
  }

  async loadFinalOutput(runId: string): Promise<void> {
    this.finalOutputRunId = runId; // mark attempted up front — no duplicate fetches
    this.finalOutputAvailable = false;
    this.finalOutputHtml = '';
    this.finalOutputError = null;
    try {
      // The run-scoped route (ws Viewer), not /fs/read: that refuses the Otto
      // data dir — where every run's context lives — to everyone but root.
      const data = await readRunContext(runId, 'final-output.md');
      if (this.run?.id !== runId) return; // the view moved on while this was in flight
      this.finalOutputHtml = renderFinalOutputSrcdoc(data.content);
      this.finalOutputAvailable = true;
    } catch (e) {
      if (this.run?.id !== runId) return;
      // No final-output.md (404) is normal: the panel just stays hidden.
      // Anything else (403, a timeout, a restarting daemon) says so with Retry,
      // and forgets the attempt so the next run update tries again.
      if (e instanceof ApiError && e.status === 404) return;
      this.finalOutputError = { runId, text: loadErrorText(e) };
      this.finalOutputRunId = null;
    }
  }

  /** Forget the deliverable (another workflow opened). */
  resetFinalOutput(): void {
    this.finalOutputRunId = null;
    this.finalOutputHtml = '';
    this.finalOutputAvailable = false;
    this.finalOutputError = null;
  }

  /** The run's deliverable is loaded and belongs to `run`. */
  hasFinalOutput(run: WorkflowRun): boolean {
    return run.status === 'success' && this.finalOutputAvailable && this.finalOutputRunId === run.id;
  }
}
