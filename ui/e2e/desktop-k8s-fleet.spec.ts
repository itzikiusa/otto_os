import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// Kubernetes → Monitor → Fleet: the ClickHouse-only cross-cluster dashboard.
//
// The daemon's fleet routes are REAL (validation, cluster-name resolution, the
// `request_labels` config round-trip) but the isolated E2E daemon has no
// ClickHouse, so every `/k8s/monitor/fleet/*` read is answered by `page.route`
// fixtures. Two clusters are registered through the API so the pills carry
// real names; a third id exists only in the fixture to prove history for a
// removed cluster survives.

test.describe.configure({ mode: 'serial' });

let ctx: APIRequestContext;
let base: string;
let wsId = '';
let clusterA = '';
let clusterB = '';

const LS = 'otto_k8s_fleet';

function fixtures(a: string, b: string) {
  const cl = (id: string, name: string, env: string, rows: number) => ({ id, name, environment: env, color: '#0af', rows });
  const filters = {
    window: '24h',
    clusters: [cl(a, 'STG AWS', 'staging', 900), cl(b, 'Groove STG', 'dev', 120), cl('gone', 'gone', 'dev', 3)],
    namespaces: [
      { cluster_id: a, namespace: 'shop' },
      { cluster_id: a, namespace: 'ops' },
      { cluster_id: b, namespace: 'groove' },
    ],
    workloads: [
      { cluster_id: a, namespace: 'shop', workload: 'web' },
      { cluster_id: a, namespace: 'ops', workload: 'cron' },
      { cluster_id: b, namespace: 'groove', workload: 'games' },
    ],
    pods: [],
  };
  const row = (cid: string, name: string, ns: string, wl: string, pod: string, oom: number, rps: number) => ({
    cluster: { id: cid, name, environment: cid === a ? 'staging' : 'dev', color: '#0af' },
    cluster_id: cid,
    namespace: ns,
    workload: wl,
    pod,
    pods: pod ? 1 : 3,
    restarts: { oom, crash: 1, probe: 0, unknown: 0 },
    churn: 2,
    mem_last: 3e9,
    mem_avg: 2.5e9,
    mem_max: 3.2e9,
    rps,
    err_pct: oom > 0 ? 6.5 : 0.2,
    latency_kind: 'p95',
    latency_ms: 120,
  });
  const table = {
    window: '24h', group: 'workload', sort: 'restarts', dir: 'desc', total: 3, offset: 0,
    rows: [row(a, 'STG AWS', 'shop', 'web', '', 4, 400), row(b, 'Groove STG', 'groove', 'games', '', 0, 12), row('gone', 'gone', 'ops', 'cron', '', 0, 0)],
  };
  const podTable = {
    window: '24h', group: 'pod', sort: 'restarts', dir: 'desc', total: 2, offset: 0,
    rows: [row(a, 'STG AWS', 'shop', 'web', 'web-1', 3, 200), row(a, 'STG AWS', 'shop', 'web', 'web-2', 1, 200)],
  };
  const t0 = Date.now() - 3600_000 * 6;
  const pts = (n: number, base: number) => Array.from({ length: n }, (_, i) => ({ t: new Date(t0 + i * 600_000).toISOString(), v: base + (i % 3) }));
  const series = (metric: string) => ({
    window: '24h', metric, unit: metric === 'mem' ? 'bytes' : 'count', by: metric === 'restarts' ? 'class' : 'cluster', step_secs: 600,
    series: metric === 'restarts'
      ? [{ key: 'oom', label: 'oom', points: pts(36, 1) }, { key: 'crash', label: 'crash', points: pts(36, 0) }]
      : [{ key: a, label: 'STG AWS', points: pts(36, 100) }, { key: b, label: 'Groove STG', points: pts(36, 20) }],
  });
  const events = {
    window: '24h', sort: 'ts', dir: 'desc', total: 2, offset: 0,
    rows: [
      { ts: new Date().toISOString(), cluster_id: a, cluster: { id: a, name: 'STG AWS', environment: 'staging', color: '#0af' }, namespace: 'shop', workload: 'web', pod: 'web-1', container: 'web', kind: 'restart', class: 'oom', reason: 'OOMKilled', exit_code: 137, detail: { prev_restarts: 0 }, actor: '' },
      { ts: new Date(Date.now() - 60_000).toISOString(), cluster_id: b, cluster: { id: b, name: 'Groove STG', environment: 'dev', color: '#fa0' }, namespace: 'groove', workload: 'games', pod: 'games-2', container: '', kind: 'churn', class: 'planned', reason: 'rollout', exit_code: 0, detail: { planned_by: 'rollout' }, actor: '' },
    ],
  };
  const requests = (enabled: boolean) => ({
    window: '24h',
    enabled_on: enabled ? [{ id: a, name: 'STG AWS' }] : [],
    disabled_on: enabled ? [{ id: b, name: 'Groove STG' }] : [{ id: a, name: 'STG AWS' }, { id: b, name: 'Groove STG' }],
    rows: enabled ? [{ path: '/api/spin', method: 'POST', rps: 40, err_pct: 0.5, avg_ms: 18 }, { path: '/api/balance', method: 'GET', rps: 80, err_pct: 7, avg_ms: 9 }] : [],
  });
  return { filters, table, podTable, series, events, requests };
}

async function boot(page: Page, route = 'kubernetes/monitor/fleet'): Promise<void> {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    if (!sessionStorage.getItem('otto_e2e_fleet_reset')) {
      sessionStorage.setItem('otto_e2e_fleet_reset', '1');
      localStorage.removeItem('otto_k8s_fleet');
    }
  }, wsId);
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
}

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-fleet-'));
  const kube = join(dir, 'kube.yaml');
  writeFileSync(kube, 'apiVersion: v1\nkind: Config\n');
  for (const [name, env] of [['STG AWS', 'staging'], ['Groove STG', 'dev']] as const) {
    const r = await ctx.post(`${base}/api/v1/k8s/clusters`, {
      data: { name, source: 'kubeconfig', kubeconfig_path: kube, context_name: 'e2e', default_namespace: 'shop', environment: env },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    const id = (await r.json()).id as string;
    if (name === 'STG AWS') clusterA = id;
    else clusterB = id;
  }
});

let requestLabelsOn = false;
test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const fx = fixtures(clusterA, clusterB);
  const json = (body: unknown) => ({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  await page.route('**/api/v1/k8s/monitor/fleet/filters*', (r) => r.fulfill(json(fx.filters)));
  await page.route('**/api/v1/k8s/monitor/fleet/table*', (r) => {
    const u = new URL(r.request().url());
    r.fulfill(json(u.searchParams.get('group') === 'pod' ? fx.podTable : fx.table));
  });
  await page.route('**/api/v1/k8s/monitor/fleet/series*', (r) => {
    const u = new URL(r.request().url());
    r.fulfill(json(fx.series(u.searchParams.get('metric') ?? 'restarts')));
  });
  await page.route('**/api/v1/k8s/monitor/fleet/events*', (r) => r.fulfill(json(fx.events)));
  await page.route('**/api/v1/k8s/monitor/fleet/requests*', (r) => r.fulfill(json(fx.requests(requestLabelsOn))));
});

test('overview: cluster pills (incl. a removed cluster), KPIs from the table, five charts, window persists', async ({ page }) => {
  await boot(page);
  const fleet = page.getByTestId('k8s-fleet');
  await expect(fleet).toBeVisible();
  const pills = page.getByTestId('k8s-fleet-cluster');
  await expect(pills).toHaveCount(3);
  await expect(pills.nth(0)).toContainText('STG AWS');
  await expect(pills.nth(2)).toContainText('gone');
  // KPIs: 3 rows × (oom + crash) = 4+1 + 0+1 + 0+1 = 7 unplanned restarts, 6 churn.
  const kpis = page.getByTestId('k8s-fleet-kpis');
  await expect(kpis).toContainText('7');
  await expect(kpis).toContainText('OOM 4');
  await expect(kpis).toContainText('6');
  await expect(page.getByTestId('k8s-fleet-charts').locator('.chart')).toHaveCount(5);
  await expect(page.getByTestId('k8s-fleet-charts')).toContainText('Restarts by class');
  // Window change is requested and persisted.
  const req = page.waitForRequest((r) => r.url().includes('/fleet/table?') && r.url().includes('window=6h'));
  await page.getByRole('radio', { name: '6h' }).click();
  await req;
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible();
  await expect(page.getByRole('radio', { name: '6h' })).toHaveAttribute('aria-checked', 'true');
  expect(JSON.parse(await page.evaluate((k) => localStorage.getItem(k as string) ?? '{}', LS)).window).toBe('6h');
});

test('filters: cluster pills, namespace/workload selects hit the API and persist; Clear resets', async ({ page }) => {
  await boot(page);
  // Clicking a pill narrows every call to that cluster.
  const req = page.waitForRequest((r) => r.url().includes('/fleet/table?') && decodeURIComponent(r.url()).includes(`cluster=${clusterA}`));
  await page.getByTestId('k8s-fleet-cluster').nth(0).click();
  await req;
  await expect(page.getByTestId('k8s-fleet-cluster').nth(0)).toHaveAttribute('aria-pressed', 'true');
  // Namespace options follow the selected clusters (STG AWS → shop, ops; not groove).
  const nsSel = page.getByRole('combobox', { name: 'Namespace' });
  await expect(nsSel.locator('option')).toHaveCount(3);
  await expect(nsSel.locator('option', { hasText: 'groove' })).toHaveCount(0);
  const req2 = page.waitForRequest((r) => r.url().includes('/fleet/table?') && r.url().includes('ns=shop'));
  await nsSel.selectOption('shop');
  await req2;
  const wlSel = page.getByRole('combobox', { name: 'Workload' });
  await expect(wlSel.locator('option')).toHaveCount(2); // All + web
  const req3 = page.waitForRequest((r) => r.url().includes('/fleet/table?') && r.url().includes('workload=web'));
  await wlSel.selectOption('web');
  await req3;
  // Persisted across reload.
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible();
  await expect(page.getByTestId('k8s-fleet-cluster').nth(0)).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('combobox', { name: 'Namespace' })).toHaveValue('shop');
  await expect(page.getByRole('combobox', { name: 'Workload' })).toHaveValue('web');
  // Clear.
  await page.getByTestId('k8s-fleet-clear').click();
  await expect(page.getByTestId('k8s-fleet-cluster').nth(0)).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByRole('combobox', { name: 'Namespace' })).toHaveValue('');
  await expect(page.getByTestId('k8s-fleet-clear')).toHaveCount(0);
});

test('table: sortable by every column (server-side), group by pods, row drill-down', async ({ page }) => {
  await boot(page, 'kubernetes/monitor/fleet/table');
  const table = page.getByTestId('k8s-fleet-table');
  await expect(table.locator('tbody tr')).toHaveCount(3);
  await expect(page.getByTestId('k8s-fleet-table-count')).toContainText('3 workloads');
  // Every header is a sort control; clicking one re-queries with sort + dir.
  await expect(table.locator('thead .th-btn')).toHaveCount(12);
  for (const [label, key, first] of [['Memory', 'mem_last', 'desc'], ['5xx', 'err_pct', 'desc'], ['Namespace', 'namespace', 'asc']] as const) {
    const req = page.waitForRequest((r) => r.url().includes('/fleet/table?') && r.url().includes(`sort=${key}`) && r.url().includes(`dir=${first}`));
    await table.locator('thead .th-btn', { hasText: label }).click();
    await req;
  }
  // Second click flips the direction.
  const flip = page.waitForRequest((r) => r.url().includes('sort=namespace') && r.url().includes('dir=desc'));
  await table.locator('thead .th-btn', { hasText: 'Namespace' }).click();
  await flip;
  // Persisted.
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible();
  const saved = JSON.parse(await page.evaluate((k) => localStorage.getItem(k as string) ?? '{}', LS));
  expect(saved.sort).toBe('namespace');
  expect(saved.dir).toBe('desc');
  // Group by pods.
  const podReq = page.waitForRequest((r) => r.url().includes('/fleet/table?') && r.url().includes('group=pod'));
  await page.getByRole('radio', { name: 'Pods' }).click();
  await podReq;
  await expect(page.getByTestId('k8s-fleet-table').locator('tbody tr')).toHaveCount(2);
  await expect(page.getByTestId('k8s-fleet-table')).toContainText('web-1');
  await page.getByRole('radio', { name: 'Workloads' }).click();
  // Drill-down: a workload row narrows the filters and switches to pods.
  await page.getByTestId('k8s-fleet-row').first().click();
  await expect(page.getByRole('radio', { name: 'Pods' })).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByRole('combobox', { name: 'Workload' })).toHaveValue('web');
  await expect(page.getByTestId('k8s-fleet-cluster').nth(0)).toHaveAttribute('aria-pressed', 'true');
  // A pod row jumps to its events.
  await page.getByTestId('k8s-fleet-row').first().click();
  await expect(page).toHaveURL(/fleet\/events$/);
  await expect(page.getByRole('combobox', { name: 'Pod' })).toHaveValue('web-1');
});

test('events: cross-cluster rows, class filter and sort hit the API', async ({ page }) => {
  await boot(page, 'kubernetes/monitor/fleet/events');
  const ev = page.getByTestId('k8s-fleet-events');
  await expect(ev.locator('tbody tr')).toHaveCount(2);
  await expect(ev).toContainText('STG AWS');
  await expect(ev).toContainText('Groove STG');
  await expect(ev).toContainText('OOMKilled');
  await expect(page.getByTestId('k8s-fleet-events-count')).toContainText('2 events');
  const req = page.waitForRequest((r) => r.url().includes('/fleet/events?') && r.url().includes('class=oom'));
  await page.getByRole('combobox', { name: 'Event class' }).selectOption('oom');
  await req;
  const sortReq = page.waitForRequest((r) => r.url().includes('/fleet/events?') && r.url().includes('sort=workload') && r.url().includes('dir=asc'));
  await ev.locator('thead .th-btn', { hasText: 'Workload' }).click();
  await sortReq;
});

test('requests: explains the opt-in, links to settings, then lists routes once a cluster keeps labels', async ({ page }) => {
  requestLabelsOn = false;
  await boot(page, 'kubernetes/monitor/fleet/requests');
  const off = page.getByTestId('k8s-fleet-requests-off');
  await expect(off).toBeVisible();
  await expect(off.getByRole('button', { name: 'STG AWS settings' })).toBeVisible();
  // The settings form carries the new toggle and the daemon persists it.
  await off.getByRole('button', { name: 'STG AWS settings' }).click();
  const toggle = page.getByTestId('k8s-monitor-request-labels');
  await expect(toggle).toBeVisible();
  await toggle.check();
  await page.getByTestId('k8s-monitor-save').click();
  await expect(page.locator('body')).toContainText('Monitoring saved');
  const saved = await ctx.get(`${base}/api/v1/k8s/clusters/${clusterA}/monitor`);
  expect((await saved.json()).config.request_labels).toBe(true);
  // With labels on, the fleet page lists routes (fixture) sorted by rps.
  requestLabelsOn = true;
  await page.goto('/#/kubernetes/monitor/fleet/requests');
  const rq = page.getByTestId('k8s-fleet-requests');
  await expect(rq.locator('tbody tr')).toHaveCount(2);
  await expect(rq.locator('tbody tr').nth(0)).toContainText('/api/balance');
  await rq.locator('thead .th-btn', { hasText: '5xx' }).click();
  await expect(rq.locator('tbody tr').nth(0)).toContainText('/api/balance');
  await rq.locator('thead .th-btn', { hasText: 'Route' }).click();
  await expect(rq.locator('tbody tr').nth(0)).toContainText('/api/balance');
  await rq.locator('thead .th-btn', { hasText: 'Route' }).click();
  await expect(rq.locator('tbody tr').nth(0)).toContainText('/api/spin');
});

test('the Monitor overview links to the fleet and the page never overflows', async ({ page }) => {
  await boot(page, 'kubernetes/monitor');
  await page.getByTestId('k8s-monitor-fleet-link').click();
  await expect(page).toHaveURL(/kubernetes\/monitor\/fleet$/);
  await expect(page.getByTestId('k8s-fleet')).toBeVisible();
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(1);
});
