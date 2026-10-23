// W2P7 (architecture-1-08): the Workflows run monitor lifted out of
// WorkflowsPage (modules/workflows/runSync.svelte.ts). Driven through the
// real class with the real run merge (runProgress.ts) and fake transports:
// a terminal status stops the polling, and an approval resolves into the run.
// The approval-preview, approve/deny and final-output cases that used to drive
// WorkflowsPage's own functions (workflowPublicationPreview.test.ts and the
// final-output case of workflowInspector.test.ts) run against the class here.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

const runProgress = loadSource(new URL('../src/modules/workflows/runProgress.ts', import.meta.url), {});

class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) { super(message); this.status = status; }
}

interface Transport {
  progress?: (id: string) => Promise<any>;
  post?: (url: string, body: any) => Promise<any>;
  nodeDetail?: (runId: string, nodeId: string) => Promise<any>;
  readContext?: (runId: string, path: string) => Promise<any>;
}

function monitor(transport: Transport) {
  const effects: (() => (() => void) | void)[] = [];
  const polls: { id: string; stopped: boolean }[] = [];
  const toasts: string[] = [];
  const timers: (() => void)[] = [];
  const posts: { url: string; body: any }[] = [];
  const bus = { tick: 0, runId: '', rev: 0, status: '', node: null, waitingApproval: false };
  const mod = loadSource(new URL('../src/modules/workflows/runSync.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => unknown) => fn() },
    '../../lib/api/client': { api: { post: async (url: string, body: any) => { posts.push({ url, body }); return transport.post?.(url, body); } }, ApiError },
    '../../lib/api/workflows': {
      workflowProgress: (id: string) => transport.progress!(id),
      workflowNodeDetail: (runId: string, nodeId: string) => transport.nodeDetail!(runId, nodeId),
      browseRunContext() {},
      readRunContext: (runId: string, path: string) => transport.readContext!(runId, path),
    },
    '../../lib/events.svelte': { workflowRunBus: bus },
    '../../lib/live': { liveQuery(o: { run: () => void }) {
      const p = { id: '', stopped: false, run: o.run };
      polls.push(p);
      return { stop() { p.stopped = true; } };
    } },
    '../../lib/md': { renderMarkdownGfm: (md: string, opts?: { breaks?: boolean }) => (opts?.breaks ? md.replace(/\n/g, '<br>') : md) },
    '../../lib/stores/ui.svelte': { ui: { theme: 'system', accent: 'blue', resolvedScheme: 'light' } },
    '../../lib/toast.svelte': { toasts: { success: (t: string) => toasts.push(t) } },
    '../../lib/toastError': { toastError: (t: string) => toasts.push(t) },
    './runContext': { runContextPath: (_d: string, p: string) => p },
    './runProgress': runProgress,
  }, {
    $effect: (fn: () => (() => void) | void) => { effects.push(fn); },
    document: { hidden: false, addEventListener() {}, removeEventListener() {} },
    setTimeout: (fn: () => void) => { timers.push(fn); return timers.length; },
  });
  const mon = new mod.RunMonitor();
  mon.track();
  return { mon, effects, polls, toasts, timers, posts, bus };
}

const node = (id: string, status: string) => ({ node_id: id, status, detail_version: `${id}-${status}` });
const flush = () => new Promise((r) => setImmediate(r));

test('the safety poll runs only while the run is live; a terminal status stops it', () => {
  const { mon, effects, polls } = monitor({});
  // The poll effect is the one that starts a liveQuery for a live run.
  mon.run = { id: 'r1', status: 'running', rev: 1, nodes: [node('a', 'running')] };
  const cleanups = effects.map((fn) => fn());
  assert.equal(polls.length, 1, 'a running run is polled');
  // Svelte re-runs the effect when the status changes: cleanup, then re-run.
  mon.run.status = 'success';
  for (const c of cleanups) if (typeof c === 'function') c();
  effects.forEach((fn) => fn());
  assert.equal(polls[0].stopped, true, 'the live poll is stopped');
  assert.equal(polls.length, 1, 'a finished run starts no new poll');
});

test('waitRunTerminal returns once the on-screen run settles, with no network', async () => {
  let fetched = 0;
  const { mon, timers } = monitor({ progress: async () => { fetched++; return { changed: false }; } });
  mon.run = { id: 'r1', status: 'running', rev: 1, nodes: [] };
  let done: { status: string } | null = null;
  void mon.waitRunTerminal('r1').then((d: { status: string }) => { done = d; });
  await flush();
  assert.equal(done, null);
  assert.equal(timers.length, 1, 'still waiting: one 1 s tick scheduled');
  mon.run.status = 'error';
  mon.run.error = 'boom';
  timers.shift()!();
  await flush();
  assert.deepEqual({ ...done! }, { status: 'error', error: 'boom' });
  assert.equal(timers.length, 0, 'no further polling after the terminal status');
  assert.equal(fetched, 0, 'the on-screen run is authoritative');
});

test('an approval posts the gate decision and the next snapshot resolves into the same run', async () => {
  const progress = deferred<any>();
  const { mon, posts, toasts } = monitor({ progress: () => progress.promise, post: async () => ({}) });
  const run = { id: 'r1', status: 'running', rev: 3, waiting_approval: true, approval_node_id: 'gate', nodes: [node('gate', 'waiting')] };
  mon.run = run;
  // No publication preview on this gate (a plain human_approval node).
  mon.approvalNeedsPreview = false;
  await mon.approveRun(true, 'ship it');
  assert.deepEqual(posts.map((p) => ({ ...p, body: { ...p.body } })), [
    { url: '/workflow-runs/r1/approve', body: { node_id: 'gate', approved: true, note: 'ship it' } },
  ]);
  assert.deepEqual(toasts, ['Approved — run resuming']);
  assert.equal(mon.approving, false);
  // The resumed run's snapshot merges INTO the run on screen (same object).
  const refetch = mon.refetchRun('r1');
  progress.resolve({ changed: true, run: { id: 'r1', status: 'running', rev: 4, waiting_approval: false, approval_node_id: null, approved_at: 't', nodes: [node('gate', 'success')] } });
  await refetch;
  assert.equal(mon.run, run, 'merged in place, not replaced');
  assert.equal(run.waiting_approval, false);
  assert.equal(run.nodes[0].status, 'success');
  // A stale (older-rev) snapshot never regresses it.
  mon.applyRunSnapshot({ id: 'r1', status: 'running', rev: 2, waiting_approval: true, nodes: [node('gate', 'waiting')] });
  assert.equal(run.waiting_approval, false);
});

test('approving is refused while the publication preview is missing or stale', async () => {
  const { mon, posts } = monitor({ post: async () => ({}) });
  mon.run = { id: 'r1', status: 'running', rev: 1, waiting_approval: true, approval_node_id: 'gate', nodes: [node('gate', 'waiting')] };
  mon.approvalNeedsPreview = true;
  mon.approvalPreviewReady = false;
  await mon.approveRun(true);
  assert.equal(posts.length, 0, 'no decision without the reviewed preview');
  mon.approvalPreviewReady = true;
  mon.approvalPreviewVersion = 'v7';
  await mon.approveRun(false, 'not yet');
  assert.deepEqual({ ...posts[0].body }, { node_id: 'gate', expected_detail_version: 'v7', approved: false, note: 'not yet' });
});

// ── Publication preview (ported from workflowPublicationPreview.test.ts) ────

const preview = { kind: 'jira', story_id: 'story', account_label: 'Reviewed account', account_url: 'https://example.test', body_md: 'Full content\n'.repeat(1000), request: { project_key: 'PRJ', reviewed_content: { title: 'Reviewed title', url: '', source_kind: 'draft' } } };
const gate = { node_id: 'gate', detail_version: 'v1' };
const waiting = (id = 'run') => ({ id, status: 'running', rev: 1, waiting_approval: true, approval_node_id: 'gate', nodes: [gate] });

test('workflow approval loads the full exact pending gate preview', async () => {
  const { mon } = monitor({ nodeDetail: async () => ({ detail_version: 'v1', body: { output: { publication_preview: preview } } }) });
  mon.run = waiting();
  await mon.loadApprovalPreview('run', gate);
  assert.equal(mon.approvalPreviewReady, true);
  assert.equal(mon.approvalPreview.body_md, preview.body_md);
  assert.equal(mon.approvalPreview.request.project_key, 'PRJ');
});

test('workflow approval rejects a different detail version and supports Retry', async () => {
  let version = 'v2';
  const { mon } = monitor({ nodeDetail: async () => ({ detail_version: version, body: { output: { publication_preview: preview } } }) });
  mon.run = waiting();
  await mon.loadApprovalPreview('run', gate);
  assert.equal(mon.approvalPreviewReady, false);
  assert.match(mon.approvalPreviewError, /changed/);
  version = 'v1';
  mon.approvalNode = gate;
  mon.retryApprovalPreview();
  await flush();
  assert.equal(mon.approvalPreviewReady, true);
  assert.equal(mon.approvalPreviewError, '');
});

test('departed run cannot adopt a late approval preview', async () => {
  const response = deferred<unknown>();
  const { mon } = monitor({ nodeDetail: () => response.promise as Promise<any> });
  mon.run = waiting();
  const pending = mon.loadApprovalPreview('run', gate);
  mon.run = waiting('other');
  response.resolve({ detail_version: 'v1', body: { output: { publication_preview: preview } } });
  await pending;
  assert.equal(mon.approvalPreview, null);
  assert.equal(mon.approvalPreviewReady, false);
});

function productGate(productGate = true, post: Transport['post'] = async () => ({})) {
  const fx = monitor({ post, progress: async () => ({ changed: false }) });
  fx.mon.run = { id: 'same-run', status: 'running', rev: 1, waiting_approval: true, approval_node_id: 'same-gate', nodes: [{ node_id: 'same-gate', detail_version: 'new-summary-v2' }] };
  Object.assign(fx.mon, {
    approvalNeedsPreview: productGate, approvalPreviewReady: true, approvalPreviewLoading: false, approvalPreviewError: '',
    approvalPreviewVersion: 'displayed-body-v1', approvalPreview: productGate ? preview : null,
  });
  return fx;
}

for (const approved of [true, false]) {
  test(`Product ${approved ? 'approve' : 'deny'} submits the displayed preview identity, not the latest summary`, async () => {
    const { mon, posts } = productGate();
    await mon.approveRun(approved);
    assert.equal(posts.length, 1);
    assert.equal(posts[0].body.expected_detail_version, 'displayed-body-v1');
    assert.equal(posts[0].body.approved, approved);
  });
}

test('Product approval cannot submit without a successfully displayed preview', async () => {
  const { mon, posts } = productGate();
  mon.approvalPreviewReady = false;
  mon.approvalPreviewVersion = null;
  await mon.approveRun(true);
  assert.equal(posts.length, 0);
});

test('stale Product approval conflict invalidates the displayed identity before another click', async () => {
  const { mon } = productGate(true, async () => { throw Object.assign(new Error('Preview replaced'), { status: 409 }); });
  await mon.approveRun(true);
  assert.equal(mon.approvalPreviewReady, false);
  assert.equal(mon.approvalPreviewVersion, null);
  assert.match(mon.approvalPreviewError, /review|preview/i);
});

test('ordinary non-Product approvals keep their existing request shape', async () => {
  const { mon, posts } = productGate(false);
  await mon.approveRun(true, 'Looks good');
  assert.deepEqual(JSON.parse(JSON.stringify(posts[0].body)), { node_id: 'same-gate', approved: true, note: 'Looks good' });
});

// ── Final output (ported from workflowInspector.test.ts) ─────────────────────

test('workflow final output renders line breaks and hides silently only when there is none', async () => {
  const fixture = (read: Transport['readContext']) => { const { mon } = monitor({ readContext: read }); mon.run = { id: 'run', status: 'success', nodes: [] }; return mon; };
  const ok = fixture(async () => ({ content: 'Status: done\nFiles: 3' }));
  await ok.loadFinalOutput('run');
  assert.equal(ok.finalOutputAvailable, true);
  assert.equal(ok.finalOutputHtml, 'Status: done<br>Files: 3', 'agent reports keep their line-by-line breaks');
  assert.equal(ok.hasFinalOutput(ok.run), true);

  const missing = fixture(async () => { throw new ApiError(404, 'not found'); });
  await missing.loadFinalOutput('run');
  assert.equal(missing.finalOutputError, null);
  assert.equal(missing.finalOutputRunId, 'run', 'a run without a deliverable is not refetched');

  const denied = fixture(async () => { throw new ApiError(403, 'forbidden'); });
  await denied.loadFinalOutput('run');
  assert.equal(JSON.stringify(denied.finalOutputError), JSON.stringify({ runId: 'run', text: 'You don’t have access to this.' }), 'the real load-error copy');
  assert.equal(denied.finalOutputRunId, null, 'a failed read is forgotten so Retry / the next update can try again');
});
