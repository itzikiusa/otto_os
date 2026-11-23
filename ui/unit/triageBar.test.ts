import test from 'node:test';
import assert from 'node:assert/strict';
import {
  buildTriageItems,
  canDecideTriageApproval,
  triageBlockedReason,
  triageKeyIntent,
} from '../src/shell/triageModel.ts';
import { loadSource } from './sourceHarness.ts';

test('buildTriageItems aggregates approvals and waiting sessions across ALL workspaces, approvals first', () => {
  const items = buildTriageItems({
    approvals: [
      {
        id: 'a-old',
        title: 'Run SQL migration',
        server_id: 'srv-1',
        server_name: 'prod-db',
        tool: 'query',
        workspace_id: 'ws-b',
        created_at: '2026-11-19T10:00:00Z',
      } as any,
      {
        id: 'a-new',
        title: 'Deploy release',
        server_id: null,
        server_name: null,
        tool: 'otto.deploy',
        workspace_id: 'ws-a',
        created_at: '2026-11-19T12:00:00Z',
      } as any,
    ],
    sessions: [
      {
        id: 's-1',
        title: 'Fix auth',
        provider: 'claude',
        workspace_id: 'ws-a',
        archived: false,
        last_active_at: '2026-11-19T11:00:00Z',
        meta: {},
      } as any,
      {
        id: 's-idle',
        title: 'Idle session',
        provider: 'claude',
        workspace_id: 'ws-a',
        archived: false,
        last_active_at: '2026-11-19T11:30:00Z',
        meta: {},
      } as any,
    ],
    otherWsSessions: [
      {
        id: 's-2',
        title: 'Review PR #42',
        provider: 'codex',
        workspace_id: 'ws-b',
        archived: false,
        last_active_at: '2026-11-19T12:30:00Z',
        meta: { scope: 'threads' },
      } as any,
    ],
    needsYou: { 's-1': true, 's-2': true, 's-idle': false },
    runtimeWaiting: [
      { id: 's-2', title: 'Duplicate thread', at: Date.parse('2026-11-19T12:30:00Z') },
      { id: 'rt-1', title: 'Runtime approval thread', at: Date.parse('2026-11-19T12:45:00Z') },
    ],
    workspaces: [
      { id: 'ws-a', name: 'Alpha' },
      { id: 'ws-b', name: 'Beta' },
    ],
    currentWorkspaceId: 'ws-a',
  });

  assert.deepEqual(
    items.map((i) => ({ id: i.id, kind: i.kind, ws: i.workspaceName })),
    [
      { id: 'approval:a-new', kind: 'approval', ws: 'Alpha' },
      { id: 'approval:a-old', kind: 'approval', ws: 'Beta' },
      { id: 'thread:rt-1', kind: 'thread', ws: 'Alpha' },
      { id: 'thread:s-2', kind: 'thread', ws: 'Beta' },
      { id: 'session:s-1', kind: 'session', ws: 'Alpha' },
    ],
  );
  assert.equal(items[1].detail, 'MCP approval · prod-db → query');
  assert.equal(items[3].route, 'threads/s-2');
});

test('triageKeyIntent maps J/K/arrows/A/D/Enter/R and ignores modified or composing events', () => {
  assert.equal(triageKeyIntent({ key: 'j' }), 'next');
  assert.equal(triageKeyIntent({ key: 'ArrowDown' }), 'next');
  assert.equal(triageKeyIntent({ key: 'K' }), 'prev');
  assert.equal(triageKeyIntent({ key: 'ArrowUp' }), 'prev');
  assert.equal(triageKeyIntent({ key: 'a' }), 'approve');
  assert.equal(triageKeyIntent({ key: 'D' }), 'deny');
  assert.equal(triageKeyIntent({ key: 'Enter' }), 'open');
  assert.equal(triageKeyIntent({ key: 'r' }), 'refresh');
  assert.equal(triageKeyIntent({ key: 'a', metaKey: true }), null);
  assert.equal(triageKeyIntent({ key: 'j', ctrlKey: true }), null);
  assert.equal(triageKeyIntent({ key: 'Enter', isComposing: true }), null);
});

test('canDecideTriageApproval blocks direct self-requests while allowing agent-raised requests', () => {
  const selfDirect = { requested_by: 'u1', requested_by_kind: 'user', server_id: null, tool: null };
  assert.equal(canDecideTriageApproval(selfDirect, 'u1', () => true, () => true), false);
  assert.match(triageBlockedReason(selfDirect, 'u1'), /another admin/);

  const selfAgent = { requested_by: 'u1', requested_by_kind: 'agent', server_id: 'srv-1', tool: 'deploy' };
  assert.equal(canDecideTriageApproval(selfAgent, 'u1', () => true, () => false), true);
  assert.equal(canDecideTriageApproval(selfAgent, 'u1', () => false, () => true), false);
});

test('TriageStore decide & cross-workspace jumpTo work cleanly', async () => {
  const switched: string[] = [];
  const navSessions: string[] = [];
  const routes: string[] = [];
  const decided: Array<{ id: string; body: { approved: boolean; note?: string } }> = [];

  const mod = loadSource(
    new URL('../src/shell/triage.svelte.ts', import.meta.url),
    {
      '../lib/api/client': { api: { get: async () => [] } },
      '../lib/api/mcp': {
        mcpCpApi: {
          cpDecide: async (id: string, body: { approved: boolean; note?: string }) => {
            decided.push({ id, body: { ...body } });
            return {};
          },
        },
      },
      '../lib/latest': {
        latestOnly: () => ({
          begin: () => ({ current: true, signal: new AbortController().signal }),
        }),
      },
      '../lib/router.svelte': { router: { go: (r: string) => routes.push(r) } },
      '../lib/stores/auth.svelte': {
        auth: { me: { id: 'u1' }, can: () => true },
      },
      '../lib/stores/resource-access.svelte': {
        resourceAccess: { can: () => true, load: async () => {} },
      },
      '../lib/stores/runtimeAttention.svelte': { runtimeWaiting: () => [] },
      '../lib/stores/workspace.svelte': {
        ws: {
          currentId: 'ws-a',
          sessions: [],
          otherWsSessions: [],
          needsYou: {},
          workspaces: [{ id: 'ws-a', name: 'Alpha' }, { id: 'ws-b', name: 'Beta' }],
          loadOtherWorkspaces: async () => {},
          select: async (id: string) => { switched.push(id); },
          navigateToSession: (id: string) => { navSessions.push(id); },
        },
      },
      '../lib/toast.svelte': { toasts: { success() {} } },
      './triageModel': { buildTriageItems, canDecideTriageApproval, triageBlockedReason },
    },
  );

  const appr = {
    id: 'appr-9',
    title: 'Approve tool',
    server_id: null,
    server_name: null,
    tool: null,
    requested_by: 'u2',
    requested_by_kind: 'agent',
    workspace_id: 'ws-b',
    created_at: '2026-11-19T12:00:00Z',
  };
  mod.triage.approvals = [appr];
  const ok = await mod.triage.decide(appr, true);
  assert.equal(ok, true);
  assert.deepEqual(decided, [{ id: 'appr-9', body: { approved: true } }]);
  assert.equal(mod.triage.approvals.length, 0);

  mod.triage.open = true;
  await mod.triage.jumpTo({
    id: 'session:s-2',
    kind: 'session',
    title: 'Fix',
    detail: 'Agent',
    workspaceId: 'ws-b',
    workspaceName: 'Beta',
    sessionId: 's-2',
  });
  assert.equal(mod.triage.open, false);
  assert.deepEqual(switched, ['ws-b']);
  assert.deepEqual(navSessions, ['s-2']);

  await mod.triage.jumpTo({
    id: 'thread:t-7',
    kind: 'thread',
    title: 'Thread',
    detail: 'Thread',
    workspaceId: 'ws-a',
    workspaceName: 'Alpha',
    route: 'threads/t-7',
  });
  assert.deepEqual(routes, ['threads/t-7']);
});
