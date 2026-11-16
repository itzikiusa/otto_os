import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { loadSource } from './sourceHarness.ts';

// delve reuses variablesReference numbers on every stop: keyed on them, the
// Locals pane kept the first stop's variables after Continue.
describe('IDE debug stop variables', () => {
  const mod = loadSource(
    new URL('../src/modules/ide/store/ideDebugSnapshotSync.ts', import.meta.url),
    {
      '../../../lib/api/client': { api: {} },
      '../../../lib/loadError': { loadErrorText: String },
      '../shell/ideProjectHelpers': { canonicalizeLocalProjectPath: (p: string) => p },
      './ideTypes': { streamIdeSse() {} },
    },
    {},
    { relative: true },
  );
  const stop = (line: number, names: string[]): any => ({
    id: 's', state: 'paused', active_thread_id: 6, active_frame_id: 1000, paused_file: '/x_test.go', paused_line: line,
    scopes: [{ name: 'Locals', variables_reference: 1000, variables: names.map((name) => ({ name, value: '', variables_reference: 0 })) }],
  });

  it('a new stop replaces the cached locals of the previous one', () => {
    const host: any = { expandedVariables: {} };
    const first = stop(11, ['t', 'table']);
    mod.syncStopVariables(host, null, first);
    host.expandedVariables[2000] = [{ name: 'child' }];
    mod.syncStopVariables(host, first, first);
    assert.ok(host.expandedVariables[2000], 'same stop keeps expansions');
    mod.syncStopVariables(host, first, stop(16, ['t', 'table', 'id', 'query']));
    assert.deepEqual(host.expandedVariables[1000].map((v: any) => v.name), ['t', 'table', 'id', 'query']);
    assert.equal(host.expandedVariables[2000], undefined);
  });
});
