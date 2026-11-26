// Unit tests for Wave 1 (MUST) fixes across workflows, loops, mcp, plugins, and usage.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { pluginFrameAction } from '../src/modules/plugins/pluginFrame.ts';
import { handleBarChartKey } from '../src/modules/usage/chartKeys.ts';

test('pluginFrameAction rejects mismatched origin even when source matches', () => {
  const win = { name: 'plugin' };
  const msg = { type: 'otto:keydown', key: 'k', metaKey: true };
  assert.equal(
    pluginFrameAction({ source: win, origin: 'https://evil.example', data: msg }, win, 'http://127.0.0.1:7700'),
    null,
  );
  assert.deepEqual(
    pluginFrameAction({ source: win, origin: 'http://127.0.0.1:7700', data: msg }, win, 'http://127.0.0.1:7700'),
    { kind: 'keydown', init: msg },
  );
});

test('handleBarChartKey navigates across bars with arrows, Home, and End', () => {
  const keys = ['2026-11-01', '2026-11-02', '2026-11-03'];
  const selected: string[] = [];
  const focused: string[] = [];
  const makeEvent = (key: string) => {
    let prevented = false;
    return {
      key,
      preventDefault: () => { prevented = true; },
      get prevented() { return prevented; },
      currentTarget: {
        closest: () => ({
          querySelector: (sel: string) => ({ focus: () => focused.push(sel) }),
        }),
      },
    } as unknown as KeyboardEvent & { prevented: boolean };
  };

  const right = makeEvent('ArrowRight');
  handleBarChartKey(right, keys, 0, 'data-daily-bar', (k) => selected.push(k));
  assert.equal(right.prevented, true);
  assert.deepEqual(selected, ['2026-11-02']);
  assert.equal(focused.length, 1);

  const end = makeEvent('End');
  handleBarChartKey(end, keys, 0, 'data-daily-bar', (k) => selected.push(k));
  assert.deepEqual(selected, ['2026-11-02', '2026-11-03']);

  const home = makeEvent('Home');
  handleBarChartKey(home, keys, 2, 'data-daily-bar', (k) => selected.push(k));
  assert.deepEqual(selected, ['2026-11-02', '2026-11-03', '2026-11-01']);

  // At boundary, ArrowLeft on index 0 is a no-op.
  const leftAtStart = makeEvent('ArrowLeft');
  handleBarChartKey(leftAtStart, keys, 0, 'data-daily-bar', (k) => selected.push(k));
  assert.equal(leftAtStart.prevented, false);
  assert.equal(selected.length, 3);
});

test('TriggersPanel formatPreviewTime handles empty or invalid timezone safely', () => {
  const fn = componentFunctions(
    new URL('../src/modules/workflows/TriggersPanel.svelte', import.meta.url),
    ['formatPreviewTime', 'outwardDeliveryInfo'],
    {
      destWho: (k: string, label: string) => `anyone in ${k}:${label}`,
    },
  );
  const utc = fn.formatPreviewTime('2026-11-20T10:00:00Z', '');
  assert.equal(typeof utc, 'string');
  assert.ok(utc.length > 0);
  const bad = fn.formatPreviewTime('2026-11-20T10:00:00Z', 'Not/A_Real_Zone');
  assert.equal(typeof bad, 'string');
  assert.ok(bad.length > 0);

  assert.equal(fn.outwardDeliveryInfo({ type: 'cron', cron: '0 9 * * *', tz: 'UTC' }), null);
  const slack = fn.outwardDeliveryInfo({
    type: 'cron',
    cron: '0 9 * * *',
    tz: 'UTC',
    result_channel: 'slack',
    result_chat_id: 'C123',
  });
  assert.ok(slack);
  assert.match(slack.where, /Slack/);
  const hook = fn.outwardDeliveryInfo({
    type: 'cron',
    cron: '0 9 * * *',
    tz: 'UTC',
    result_webhook: 'https://hooks.example.com/abc',
  });
  assert.ok(hook);
  assert.match(hook.where, /hooks\.example\.com/);
});

test('ToolsTab confirms disabling approval on non-read tools and downgrading dangerous tools', async () => {
  const asks: { msg: string; opts: unknown }[] = [];
  const patches: unknown[] = [];
  let allowConfirm = false;

  const tab = componentFunctions(
    new URL('../src/modules/mcp/ToolsTab.svelte', import.meta.url),
    ['patchTool', 'replaceTool', 'toggleRequireApproval', 'changeRiskLabel'],
    {
      busyTool: {},
      tools: [
        { id: 't-write', name: 'deploy', risk_label: 'write', require_approval: true },
        { id: 't-read', name: 'list', risk_label: 'read', require_approval: true },
        { id: 't-danger', name: 'drop_db', risk_label: 'dangerous', require_approval: true },
      ],
      confirmer: {
        ask: async (msg: string, opts: unknown) => {
          asks.push({ msg, opts });
          return allowConfirm;
        },
      },
      mcpCpApi: {
        cpPatchTool: async (id: string, body: unknown) => {
          patches.push([id, body]);
          return { id, ...((body as Record<string, unknown>) ?? {}) };
        },
      },
      toastError: () => assert.fail('no error'),
    },
  );

  // Declining confirmation blocks removing approval on a write tool.
  await tab.toggleRequireApproval(tab.tools[0]);
  assert.equal(asks.length, 1);
  assert.equal(patches.length, 0);

  // Read-only tools do not require confirmation to remove approval.
  await tab.toggleRequireApproval(tab.tools[1]);
  assert.equal(asks.length, 1);
  assert.equal(patches.length, 1);

  // Declining downgrade of a dangerous tool restores the <select> value.
  const selectEl = { value: 'write' } as HTMLSelectElement;
  await tab.changeRiskLabel(tab.tools[2], 'write', selectEl);
  assert.equal(asks.length, 2);
  assert.equal(selectEl.value, 'dangerous');
  assert.equal(patches.length, 1);

  // Approving confirmation lets the downgrade proceed.
  allowConfirm = true;
  await tab.changeRiskLabel(tab.tools[2], 'write', selectEl);
  assert.equal(asks.length, 3);
  assert.equal(patches.length, 2);
});

