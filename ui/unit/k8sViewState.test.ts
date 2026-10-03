import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_MONITOR_UI, monitorPath, parseClusterUi, parseK8sRoute, resourcesPath } from '../src/modules/kubernetes/viewState.ts';

test('per-cluster view state round-trips and repairs malformed fields', () => {
  const ui = parseClusterUi(
    JSON.stringify({
      filters: { pods: 'api', services: 7 },
      drawerTab: 'logs',
      scroll: { 'c|pods|ns': 300, bad: -4, nan: 'x' },
      monitor: { ns: 'prod', filter: 'web', sortKey: 'rps', sortDir: 1, expanded: 'prod/web', classFilter: 'oom', scrollTop: 120 },
    }),
  );
  assert.deepEqual(ui.filters, { pods: 'api' });
  assert.equal(ui.drawerTab, 'logs');
  assert.deepEqual(ui.scroll, { 'c|pods|ns': 300 });
  assert.deepEqual(ui.monitor, { ns: 'prod', filter: 'web', sortKey: 'rps', sortDir: 1, expanded: 'prod/web', classFilter: 'oom', scrollTop: 120 });

  const bad = parseClusterUi(JSON.stringify({ drawerTab: 'nope', monitor: { sortDir: 3, expanded: 5 } }));
  assert.equal(bad.drawerTab, 'overview');
  assert.deepEqual(bad.monitor, DEFAULT_MONITOR_UI);
  assert.deepEqual(parseClusterUi('{not json').monitor, DEFAULT_MONITOR_UI);
  assert.deepEqual(parseClusterUi(null).filters, {});
});

test('route grammar: workspace monitor, legacy monitor, fleet and resources', () => {
  assert.deepEqual(parseK8sRoute(['kubernetes']), { view: 'overview' });
  assert.deepEqual(parseK8sRoute(['kubernetes', 'monitor']), { view: 'monitor-overview' });
  assert.deepEqual(parseK8sRoute(['kubernetes', 'monitor', 'fleet']), { view: 'fleet', tab: 'overview' });
  assert.deepEqual(parseK8sRoute(['kubernetes', 'monitor', 'c1', 'events']), { view: 'monitor', clusterId: 'c1', tab: 'events', legacy: true });
  assert.deepEqual(parseK8sRoute(['kubernetes', 'c1', 'monitor']), { view: 'monitor', clusterId: 'c1', tab: 'workloads', legacy: false });
  assert.deepEqual(parseK8sRoute(['kubernetes', 'c1', 'pods', 'ns', 'p']), { view: 'resources', clusterId: 'c1', kind: 'pods', ns: 'ns', name: 'p' });
  assert.equal(monitorPath('a b', 'events'), 'kubernetes/a%20b/monitor/events');
  assert.equal(resourcesPath('c', 'nodes', '', 'n1'), 'kubernetes/c/nodes/-/n1');
  assert.equal(resourcesPath('c', 'pods'), 'kubernetes/c/pods');
});
