// MCP discovery review: a tool new to a reviewed server, or one whose
// description / schema changed since review, is badged in the Tools tab with
// an Acknowledge action that PATCHes `acknowledge: true`.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

test('the badge names both review states, changed winning over new', () => {
  const badge = componentFunctions(new URL('../src/modules/mcp/ToolReviewBadge.svelte', import.meta.url), ['reviewState'], {});
  const plain = (v: unknown) => JSON.parse(JSON.stringify(v));
  assert.equal(badge.reviewState({ unreviewed: false, changed_since_review: false }), null);
  assert.deepEqual(
    plain(badge.reviewState({ unreviewed: true, changed_since_review: false })),
    { label: 'New — not reviewed', tone: 'info', hint: badge.reviewState({ unreviewed: true, changed_since_review: false }).hint },
  );
  const changed = plain(badge.reviewState({ unreviewed: true, changed_since_review: true }));
  assert.equal(changed.label, 'Changed since reviewed');
  assert.equal(changed.tone, 'warn');
  assert.match(changed.hint, /needs approval/);
});

test('Acknowledge goes through the tool PATCH and replaces the row', async () => {
  const sent: unknown[] = [];
  const tab = componentFunctions(new URL('../src/modules/mcp/ToolsTab.svelte', import.meta.url), ['patchTool', 'replaceTool'], {
    busyTool: {},
    tools: [{ id: 't1', changed_since_review: true }],
    mcpCpApi: { cpPatchTool: async (id: string, body: unknown) => { sent.push([id, body]); return { id, changed_since_review: false }; } },
    toastError: () => assert.fail('no error'),
  });
  await tab.patchTool({ id: 't1' }, { acknowledge: true });
  assert.deepEqual(JSON.parse(JSON.stringify(sent)), [['t1', { acknowledge: true }]]);
  assert.equal(tab.tools[0].changed_since_review, false);
  assert.deepEqual(JSON.parse(JSON.stringify(tab.busyTool)), {});
});
