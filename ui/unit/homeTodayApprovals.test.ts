import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

interface StubApproval {
  id: string;
  title: string;
  server_id: string | null;
  server_name: string | null;
  tool: string | null;
  requested_by: string | null;
  requested_by_kind: string | null;
  workspace_id: string | null;
  created_at: string;
}

function makeToday(opts: {
  meId?: string;
  canAdmin?: boolean;
  canApproveServer?: boolean;
  decideImpl?: (id: string, body: { approved: boolean; note?: string }) => Promise<unknown>;
}) {
  const decidedCalls: Array<{ id: string; body: { approved: boolean; note?: string } }> = [];
  const toastLog: Array<{ kind: string; title: string; body?: string }> = [];

  const mod = loadSource(
    new URL('../src/modules/home/today.svelte.ts', import.meta.url),
    {
      '../../lib/api/client': { api: { get: async () => [] } },
      '../../lib/api/design': { listArtifacts: async () => [] },
      '../../lib/api/mcp': {
        mcpCpApi: {
          cpDecide: async (id: string, body: { approved: boolean; note?: string }) => {
            decidedCalls.push({ id, body: { ...body } });
            if (opts.decideImpl) return opts.decideImpl(id, body);
            return {};
          },
        },
      },
      '../../lib/api/scheduledTasks': { scheduledTasksApi: { list: async () => [] } },
      svelte: { untrack: <T>(fn: () => T) => fn() },
      '../../lib/router.svelte': { router: { go: () => {} } },
      '../../lib/stores/auth.svelte': {
        auth: {
          me: { id: opts.meId ?? 'u-admin' },
          can: (_f: string, level: string) => (level === 'admin' ? opts.canAdmin !== false : true),
        },
      },
      '../../lib/stores/notifications.svelte': {
        notifications: { notices: [], ensureLoaded: async () => {}, runAction: async () => {} },
      },
      '../../lib/stores/resource-access.svelte': {
        resourceAccess: {
          can: () => opts.canApproveServer !== false,
          load: async () => {},
        },
      },
      '../../lib/stores/workspace.svelte': {
        ws: {
          currentId: 'ws-1',
          listSettled: true,
          foregroundActive: [],
          activeSessions: [],
          sessionById: {},
          needsYou: {},
          statusMap: {},
          activeWorkflowRuns: [],
          threadsNeedsYouCount: 0,
        },
      },
      '../../lib/toast.svelte': {
        toasts: {
          success: (title: string, body?: string) => toastLog.push({ kind: 'success', title, body }),
        },
      },
      '../../lib/plural': { plural: (n: number, w: string) => `${n} ${w}` },
      '../../lib/stores/runtimeAttention.svelte': { runtimeWaiting: () => [] },
      './homeThreads': {
        threadRoute: (id: string) => `threads/${id}`,
        threadSessions: () => [],
        unlistedWaiting: () => 0,
      },
      './boxes/poll': { livePoll: () => ({ stop: () => {}, now: () => {} }) },
    },
  );

  return { today: mod.today, decidedCalls, toastLog };
}

function sampleApproval(overrides: Partial<StubApproval> = {}): StubApproval {
  return {
    id: 'appr-1',
    title: 'Run deploy tool',
    server_id: 'srv-1',
    server_name: 'prod-mcp',
    tool: 'deploy',
    requested_by: 'u-other',
    requested_by_kind: 'agent',
    workspace_id: 'ws-1',
    created_at: '2026-11-19T12:00:00Z',
    ...overrides,
  };
}

test('decideApproval approves optimistically, removes row, and clears busy state', async () => {
  const { today, decidedCalls, toastLog } = makeToday({});
  const a = sampleApproval();
  today.approvals = [a];

  const ok = await today.decideApproval(a, true);
  assert.equal(ok, true);
  assert.deepEqual(decidedCalls, [{ id: 'appr-1', body: { approved: true } }]);
  assert.equal(today.approvals.length, 0);
  assert.equal(today.deciding['appr-1'], undefined);
  assert.equal(today.decideErrors['appr-1'], undefined);
  assert.deepEqual(toastLog, [{ kind: 'success', title: 'Approved', body: 'Run deploy tool' }]);
});

test('decideApproval forwards trimmed denial reason', async () => {
  const { today, decidedCalls } = makeToday({});
  const a = sampleApproval({ id: 'appr-2' });
  today.approvals = [a];

  const ok = await today.decideApproval(a, false, '  wait for green CI  ');
  assert.equal(ok, true);
  assert.deepEqual(decidedCalls, [{ id: 'appr-2', body: { approved: false, note: 'wait for green CI' } }]);
  assert.equal(today.approvals.length, 0);
});

test('decideApproval retains row and records inline error on API failure', async () => {
  const { today } = makeToday({
    decideImpl: async () => {
      throw new Error('HTTP 503 service unavailable');
    },
  });
  const a = sampleApproval({ id: 'appr-3' });
  today.approvals = [a];

  const ok = await today.decideApproval(a, true);
  assert.equal(ok, false);
  assert.equal(today.approvals.length, 1);
  assert.equal(today.deciding['appr-3'], undefined);
  assert.ok(typeof today.decideErrors['appr-3'] === 'string' && today.decideErrors['appr-3'].length > 0);
});

test('self-requested non-agent approval is blocked from inline decision', async () => {
  const { today, decidedCalls } = makeToday({ meId: 'u-self' });
  const a = sampleApproval({ id: 'appr-4', requested_by: 'u-self', requested_by_kind: 'user' });
  today.approvals = [a];

  assert.equal(today.canDecideApproval(a), false);
  assert.match(today.decideBlockedReason(a), /another admin/);
  const ok = await today.decideApproval(a, true);
  assert.equal(ok, false);
  assert.equal(decidedCalls.length, 0);
  assert.equal(today.approvals.length, 1);
});
