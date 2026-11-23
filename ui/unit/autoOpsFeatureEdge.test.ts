// Unit tests for Feature Edge upgrades in auto-ops:
//   1. Workflows Side-Effect Preflight (sideEffects.ts)
//   2. MCP Approvals Keyboard Triage (approvalKeys.ts)
//   3. Usage Budget Burn Forecast (budgetForecast.ts)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { Connection, WorkflowGraph, WorkflowNode } from '../src/lib/api/types.ts';
import { graphHasDbQuery, graphSideEffects, nodeSideEffect } from '../src/modules/workflows/sideEffects.ts';
import { approvalTriageAction } from '../src/modules/mcp/approvalKeys.ts';
import { computeBudgetForecast, recentDailyPace } from '../src/modules/usage/budgetForecast.ts';

function conn(id: string, name: string, environment: 'prod' | 'staging' | 'dev'): Connection {
  return {
    id,
    workspace_id: 'ws-1',
    name,
    kind: 'postgres',
    environment,
    host: 'db.example.com',
    port: 5432,
    username: 'app',
    database: 'main',
    auth_mode: 'password',
    has_secret: true,
    created_at: '2026-11-01T00:00:00Z',
    updated_at: '2026-11-01T00:00:00Z',
  } as unknown as Connection;
}

test('Feature Edge 1: Workflows side-effect preflight flags outward/mutating and prod DB steps', () => {
  const conns = [conn('c-prod', 'Orders Prod', 'prod'), conn('c-dev', 'Orders Dev', 'dev')];
  const nodes: WorkflowNode[] = [
    { id: 'n-notify', kind: 'channel_notify', name: 'Notify Slack', x: 0, y: 0, params: { channel: 'slack', chat_id: '#ops' } },
    { id: 'n-http-get', kind: 'http_request', name: 'Fetch status', x: 0, y: 0, params: { method: 'GET', url: 'https://api.example.com/health' } },
    { id: 'n-http-post', kind: 'http_request', name: 'Trigger deploy', x: 0, y: 0, params: { method: 'POST', url: 'https://deploy.example.com/run' } },
    { id: 'n-db-dev', kind: 'db_query', name: 'Query dev', x: 0, y: 0, params: { connection_id: 'c-dev' } },
    { id: 'n-db-prod', kind: 'db_query', name: 'Query prod', x: 0, y: 0, params: { connection_id: 'c-prod' } },
    { id: 'n-pr', kind: 'git_pr', name: 'Open PR', x: 0, y: 0, params: {} },
    { id: 'n-pub', kind: 'product_publish', name: 'Publish PRD', x: 0, y: 0, params: {} },
    {
      id: 'n-loop',
      kind: 'loop',
      name: 'Retry loop',
      x: 0,
      y: 0,
      params: { steps: [{ name: 'Prod check', kind: 'db_query', params: { connection_id: 'c-prod' } }] },
    },
  ];
  const graph: WorkflowGraph = { nodes, edges: [] };

  assert.equal(nodeSideEffect(nodes[3], conns), null, 'dev db_query is not flagged');
  assert.equal(nodeSideEffect(nodes[4], conns)?.badge, 'Prod DB');
  assert.equal(nodeSideEffect(nodes[4], conns)?.danger, true);
  assert.equal(nodeSideEffect(nodes[1], conns)?.danger, false);
  assert.equal(nodeSideEffect(nodes[2], conns)?.badge, 'POST');
  assert.equal(nodeSideEffect(nodes[2], conns)?.danger, true);
  assert.equal(nodeSideEffect(nodes[7], conns)?.badge, 'Prod DB');
  assert.equal(graphHasDbQuery(graph), true);

  const all = graphSideEffects(graph, conns);
  assert.equal(all.length, 7);
  // Danger items are sorted ahead of informational ones.
  const firstNonDanger = all.findIndex((x) => !x.danger);
  assert.ok(firstNonDanger > 0);
  assert.ok(all.slice(0, firstNonDanger).every((x) => x.danger));
  assert.ok(all.slice(firstNonDanger).every((x) => !x.danger));
});

test('Feature Edge 2: MCP approvals keyboard triage handles J/K/A/D and respects focus guards', () => {
  const pending = ['a1', 'a2', 'a3'];

  assert.deepEqual(approvalTriageAction({ key: 'j' }, pending, null), { kind: 'select', id: 'a1' });
  assert.deepEqual(approvalTriageAction({ key: 'j' }, pending, 'a1'), { kind: 'select', id: 'a2' });
  assert.deepEqual(approvalTriageAction({ key: 'j' }, pending, 'a3'), { kind: 'select', id: 'a3' });
  assert.deepEqual(approvalTriageAction({ key: 'k' }, pending, 'a2'), { kind: 'select', id: 'a1' });
  assert.deepEqual(approvalTriageAction({ key: 'k' }, pending, 'a1'), { kind: 'select', id: 'a1' });

  assert.deepEqual(approvalTriageAction({ key: 'a' }, pending, 'a2'), { kind: 'approve', id: 'a2' });
  assert.deepEqual(approvalTriageAction({ key: 'D' }, pending, 'a2'), { kind: 'deny', id: 'a2' });
  assert.deepEqual(approvalTriageAction({ key: 'a' }, pending, null), { kind: 'approve', id: 'a1' });

  // Guarded inside inputs, dialogs, modifiers, open modals, or non-decidable rows.
  assert.equal(approvalTriageAction({ key: 'a', target: { tagName: 'INPUT' } }, pending, 'a1'), null);
  assert.equal(approvalTriageAction({ key: 'd', target: { tagName: 'TEXTAREA' } }, pending, 'a1'), null);
  assert.equal(approvalTriageAction({ key: 'j', target: { isContentEditable: true } }, pending, 'a1'), null);
  assert.equal(approvalTriageAction({ key: 'a', metaKey: true }, pending, 'a1'), null);
  assert.equal(approvalTriageAction({ key: 'a' }, pending, 'a1', { modalOpen: true }), null);
  assert.equal(approvalTriageAction({ key: 'a' }, pending, 'a1', { canDecideId: () => false }), null);
  assert.equal(approvalTriageAction({ key: 'j' }, [], null), null);
});

test('Feature Edge 3: Usage budget burn forecast computes 7d pace and exhaustion projections', () => {
  const days = [
    { day: '2026-11-10', cost_usd: 10 },
    { day: '2026-11-11', cost_usd: 10 },
    { day: '2026-11-12', cost_usd: 10 },
    { day: '2026-11-13', cost_usd: 10 },
    { day: '2026-11-14', cost_usd: 10 },
    { day: '2026-11-15', cost_usd: 10 },
    { day: '2026-11-16', cost_usd: 10 },
  ];
  const pace = recentDailyPace(days, 7, 30);
  assert.equal(pace?.paceUsd, 10);
  assert.equal(pace?.spanDays, 7);

  // Warning: 5 days left at $10/day
  const warn = computeBudgetForecast(
    { scope: 'workspace', key: 'ws-1', spent_usd: 50, limit_usd: 100, exceeded: false },
    { windowDays: 30, summaryWorkspaceId: 'ws-1', daily: days },
  );
  assert.equal(warn?.tone, 'warn');
  assert.equal(warn?.daysUntilCap, 5);
  assert.match(warn?.label ?? '', /7d pace.*~5 days/);

  // Urgent: <1 day left
  const urgent = computeBudgetForecast(
    { scope: 'provider', key: 'claude', spent_usd: 95, limit_usd: 100, exceeded: false },
    {
      windowDays: 30,
      dailyModels: days.map((d) => ({ day: d.day, provider: 'claude', cost_usd: d.cost_usd })),
    },
  );
  assert.equal(urgent?.tone, 'bad');
  assert.match(urgent?.label ?? '', /<1 day/);

  // On track: cap far away (> windowDays)
  const ok = computeBudgetForecast(
    { scope: 'workspace', key: 'ws-1', spent_usd: 10, limit_usd: 1000, exceeded: false },
    { windowDays: 30, summaryWorkspaceId: 'ws-1', daily: days },
  );
  assert.equal(ok?.tone, 'ok');
  assert.match(ok?.label ?? '', /On track/);

  // Exceeded
  const over = computeBudgetForecast(
    { scope: 'workspace', key: 'ws-1', spent_usd: 110, limit_usd: 100, exceeded: true },
    { windowDays: 30, summaryWorkspaceId: 'ws-1', daily: days },
  );
  assert.equal(over?.tone, 'bad');
  assert.equal(over?.daysUntilCap, 0);
  assert.match(over?.label ?? '', /Cap reached/);

  // Zero spend
  const idle = computeBudgetForecast(
    { scope: 'workspace', key: 'ws-other', spent_usd: 0, limit_usd: 100, exceeded: false },
    { windowDays: 30, summaryWorkspaceId: 'ws-1', daily: days },
  );
  assert.equal(idle?.tone, 'dim');
  assert.equal(idle?.daysUntilCap, null);
});
