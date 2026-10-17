import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

// MCP → Audit: the tool box applies on Enter / Apply. Load more must page the
// filters of the rows on screen, not a tool name typed but not yet applied.

test('Load more pages the applied filters, not the half-typed tool box', async () => {
  const calls: Record<string, unknown>[] = [];
  const s = componentFunctions(
    new URL('../src/modules/mcp/AuditTab.svelte', import.meta.url),
    ['query', 'load', 'loadMore'],
    {
      PAGE: 200, generation: 0, nextOffset: 0, hasMore: false, loading: false, loadingMore: false,
      moreError: null, loadError: null, rows: [],
      applied: { server: '', tool: '', decision: '' },
      fServer: 'srv', fTool: ' fs.read ', fDecision: 'denied',
      loadErrorText: (e: Error) => e.message,
      mcpCpApi: {
        cpAudit: async (q: Record<string, unknown>) => {
          calls.push(q);
          return { rows: [{ id: calls.length }], next_offset: 200 * calls.length, has_more: true };
        },
      },
    },
  );
  await s.load();
  s.fTool = 'shell.ex'; // typed, not applied
  s.fDecision = '';
  await s.loadMore();
  assert.deepEqual(calls.map((c) => [c.server_id, c.tool, c.decision, c.offset]), [
    ['srv', 'fs.read', 'denied', 0],
    ['srv', 'fs.read', 'denied', 200],
  ]);
  assert.equal(s.rows.length, 2);
});
