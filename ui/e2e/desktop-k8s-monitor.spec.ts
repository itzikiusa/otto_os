import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport } from './helpers';

// Kubernetes → Monitor: the overview cards, the per-cluster settings form
// (validation, presets, exclusions menu clamp) and the test-probes modal.
//
// The overview is fed from `page.route` fixtures (no real cluster is
// reachable from the isolated E2E daemon), while the cluster row and the
// settings round-trip are REAL calls: a cluster is registered through the
// API with a throwaway kubeconfig (registration does not validate the
// context), the settings PUT hits the daemon's validation, and the daemon's
// 409 when ClickHouse is off is exercised by trying to enable monitoring.

test.describe.configure({ mode: 'serial' });

let ctx: APIRequestContext;
let base: string;
let wsId = '';
let clusterId = '';

const OVERVIEW = [
  {
    cluster: { id: 'c-healthy', name: 'STG AWS', environment: 'staging', color: '#0af' },
    enabled: true,
    interval_secs: 60,
    status: {
      cluster_id: 'c-healthy', last_cycle_at: new Date().toISOString(), last_ok_at: new Date().toISOString(), last_error: '',
      transport_used: 'port_forward', metrics_server: 'ok', pods_seen: 108, pods_scraped: 106, pods_failed: 2, cycle_ms: 28417,
    },
    health: 'healthy',
    window: '24h',
    pods: { running: 108, pending: 0, failed: 0, crashloop: 0, total: 108 },
    restarts: { oom: 0, crash: 0, probe: 0, unknown: 0 },
    churn: 3,
    mem: { used: 12e9, limit: 40e9, pct: 30 },
    rps: 420.5,
    err_pct: 0.12,
    drift: [],
    workloads: 60,
  },
  {
    cluster: { id: 'c-degraded', name: 'Groove STG', environment: 'dev', color: '#fa0' },
    enabled: true,
    interval_secs: 60,
    status: {
      cluster_id: 'c-degraded', last_cycle_at: new Date().toISOString(), last_ok_at: new Date().toISOString(), last_error: '',
      transport_used: 'port_forward',
      metrics_server: 'forbidden: cluster RBAC: Error from server (Forbidden): pods.metrics.k8s.io is forbidden: User "u-x" cannot list resource "pods" in API group "metrics.k8s.io" in the namespace "groove"',
      pods_seen: 40, pods_scraped: 40, pods_failed: 0, cycle_ms: 9000,
    },
    health: 'degraded',
    window: '24h',
    pods: { running: 39, pending: 1, failed: 0, crashloop: 0, total: 40 },
    restarts: { oom: 2, crash: 1, probe: 0, unknown: 0 },
    churn: 0,
    mem: { used: 3e9, limit: 3.2e9, pct: 93 },
    rps: 12,
    err_pct: 4.5,
    drift: [{ workload: 'gamesmanagement', versions: ['5.02.28-205', '5.02.27-201'] }],
    workloads: 20,
  },
];

async function boot(page: Page, route: string): Promise<void> {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.goto(`/#/${route}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
}

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-k8s-'));
  const kube = join(dir, 'kube.yaml');
  writeFileSync(kube, 'apiVersion: v1\nkind: Config\n');
  const r = await ctx.post(`${base}/api/v1/k8s/clusters`, {
    data: { name: 'E2E cluster', source: 'kubeconfig', kubeconfig_path: kube, context_name: 'e2e', default_namespace: 'shop', environment: 'dev' },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  clusterId = (await r.json()).id as string;
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.route('**/api/v1/k8s/monitor/overview*', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(OVERVIEW) }),
  );
});

test('overview renders a healthy and a degraded card with the RBAC hint', async ({ page }) => {
  await boot(page, 'kubernetes/monitor');
  const cards = page.getByTestId('k8s-monitor-card');
  await expect(cards).toHaveCount(2);
  await expect(cards.nth(0)).toHaveAttribute('data-health', 'healthy');
  await expect(cards.nth(1)).toHaveAttribute('data-health', 'degraded');
  await expect(cards.nth(1)).toContainText('pods.metrics.k8s.io is forbidden');
  await expect(cards.nth(1)).toContainText('1 workload running mixed versions');
  await expect(cards.nth(1)).toContainText('OOM 2');
  // Window picker changes the query.
  const req = page.waitForRequest((r) => r.url().includes('/k8s/monitor/overview?window=6h'));
  await page.getByRole('radio', { name: '6h' }).click();
  await req;
});

test('settings form validates locally, fills a preset, and the daemon rejects enable without ClickHouse', async ({ page }) => {
  await boot(page, `kubernetes/monitor/${clusterId}/settings`);
  const settings = page.getByTestId('k8s-monitor-settings');
  await expect(settings).toBeVisible();

  // Preset fills probes.
  await page.getByTestId('k8s-monitor-preset').click();
  await page.getByRole('menuitem', { name: /Go actuator/ }).click();
  await expect(page.getByTestId('k8s-monitor-probe')).toHaveCount(3);

  // Local validation: interval 5 never reaches the daemon.
  await page.getByTestId('k8s-monitor-interval').fill('5');
  await page.getByTestId('k8s-monitor-save').click();
  await expect(settings).toContainText('Interval must be 15..3600');

  // Valid + enabled: on a machine without a `clickhouse` binary the daemon
  // answers 409 (usage engine off) and the toast carries that hint; with one
  // installed the save goes through. Either way the daemon's validation ran.
  await page.getByTestId('k8s-monitor-interval').fill('60');
  await page.getByTestId('k8s-monitor-save').click();
  await expect(page.locator('body')).toContainText('Monitoring saved');
  await page.getByTestId('k8s-monitor-enabled').getByRole('switch').click();
  await expect(page.locator('body')).toContainText(/usage engine|ClickHouse|Monitoring saved/i);

  // Disabled saves fine and round-trips.
  const monitoringSwitch = page.getByTestId('k8s-monitor-enabled').getByRole('switch');
  await expect(monitoringSwitch).toBeEnabled();
  if (await monitoringSwitch.isChecked()) await monitoringSwitch.click();
  await expect(monitoringSwitch).not.toBeChecked();
  await page.getByTestId('k8s-monitor-save').click();
  await expect(page.locator('body')).toContainText('Monitoring saved');
  const saved = await ctx.get(`${base}/api/v1/k8s/clusters/${clusterId}/monitor`);
  const cfg = (await saved.json()).config;
  expect(cfg.enabled).toBe(false);
  expect(cfg.probes).toHaveLength(3);
});

test('exclusions kind menu stays inside the viewport near the bottom of the page', async ({ page }) => {
  await boot(page, `kubernetes/monitor/${clusterId}/settings`);
  const btn = page.getByTestId('k8s-monitor-add-exclusion');
  await btn.scrollIntoViewIfNeeded();
  await btn.click();
  const menu = page.getByRole('menu');
  await expect(menu).toBeVisible();
  await expectFullyInViewport(page, menu);
  await page.getByRole('menuitem', { name: /Pod name glob/ }).click();
  await expect(page.locator('.ex .chip')).toHaveText('Pod');
});

test('monitor switch persists immediately and preserves unsaved probe settings', async ({ page }, info) => {
  const response = await ctx.get(`${base}/api/v1/k8s/clusters/${clusterId}/monitor`);
  const payload = await response.json();
  payload.config.enabled = true;
  payload.config.interval_secs = 60;
  payload.config.probes = payload.presets[0].probes;
  const writes: Array<Record<string, unknown>> = [];
  let release!: () => void;
  const blocked = new Promise<void>((resolve) => { release = resolve; });
  await page.route(`**/api/v1/k8s/clusters/${clusterId}/monitor`, async (route) => {
    if (route.request().method() === 'PUT') {
      writes.push(route.request().postDataJSON());
      await blocked;
      payload.config = writes.at(-1);
    }
    await route.fulfill({ status: 200, json: payload });
  });
  await boot(page, `kubernetes/monitor/${clusterId}/settings`);
  const toggle = page.getByTestId('k8s-monitor-enabled').getByRole('switch');
  await expect(toggle).toBeChecked();
  await page.getByTestId('k8s-monitor-interval').fill('120');
  try {
    await toggle.click();
    await expect.poll(() => writes.length).toBe(1);
    expect(writes[0].enabled).toBe(false);
    expect(writes[0].interval_secs).toBe(60);
    await expect(toggle).toBeChecked();
    await expect(toggle).toBeDisabled();
  } finally { release(); }
  await expect(toggle).not.toBeChecked();
  await expect(page.getByTestId('k8s-monitor-interval')).toHaveValue('120');
  await page.reload();
  await expect(toggle).not.toBeChecked();
  await expect(page.getByTestId('k8s-monitor-interval')).toHaveValue('60');
  await page.route(`**/api/v1/k8s/clusters/${clusterId}/monitor/test`, route => route.fulfill({ json: { namespace: 'shop', pod: 'fixture', transport: 'proxy', metrics_server: 'ok', probes: [] } }));
  await page.getByTestId('k8s-monitor-interval').fill('180');
  await page.getByRole('button', { name: 'Test probes', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'Run', exact: true }).click();
  await expect.poll(() => writes.length).toBe(2);
  await expect(dialog).toContainText('fixture');
  await page.keyboard.press('Escape');
  await toggle.click();
  await expect(toggle).toBeChecked();
  expect(writes[2].interval_secs, 'a toggle must preserve settings saved by Test probes').toBe(180);

  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await expect(page.locator('html')).toHaveAttribute('data-scheme', scheme);
    const path = info.outputPath(`monitor-switch-${scheme}.png`);
    await page.screenshot({ path, fullPage: true, animations: 'disabled' });
    await info.attach(`monitor-switch-${scheme}`, { path, contentType: 'image/png' });
  }
});

test('monitor switch keeps confirmed state when persistence fails', async ({ page }) => {
  const response = await ctx.get(`${base}/api/v1/k8s/clusters/${clusterId}/monitor`);
  const payload = await response.json();
  payload.config.enabled = true;
  await page.route(`**/api/v1/k8s/clusters/${clusterId}/monitor`, async (route) => {
    if (route.request().method() === 'PUT') {
      await route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Temporary storage failure' } });
    } else await route.fulfill({ status: 200, json: payload });
  });
  await boot(page, `kubernetes/monitor/${clusterId}/settings`);
  const toggle = page.getByTestId('k8s-monitor-enabled').getByRole('switch');
  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(page.getByTestId('k8s-monitor-settings')).toContainText('Temporary storage failure');
  await expect(toggle).toBeChecked();
  await expect(toggle).toBeEnabled();
});
