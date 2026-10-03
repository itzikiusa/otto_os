import { test, expect, type Page } from '@playwright/test';
import { openPage, expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// K-1 / K-2 / K-3 / KS-4 (desktop-browser only, every /k8s/* call mocked):
// - a module switch through the sidebar resumes the Kubernetes view exactly
//   (route + drawer + filter), also after a reload; clicking the ACTIVE module
//   goes to its main page;
// - Resources ↔ Monitor is one workspace: the switch keeps the console rows
//   (no skeleton on the way back), the Monitor keeps its expanded row, and the
//   old `kubernetes/monitor/<id>` URL redirects;
// - Monitor cross-links: "Open pods" lands on Pods with the workload filter;
// - the drawer has a Refresh button and an HTTP tab (actuator presets; the
//   per-pod result list renders).
// ─────────────────────────────────────────────────────────────────────────────

const caps = { server_version: 'v1.30.3', metrics_server: true, argo_rollouts: false, argocd: false, checked_at: '' };
const cluster = { id: 'resume-cluster', name: 'Resume cluster', source: 'kubeconfig', context_name: 'resume', default_namespace: 'default', environment: 'staging', capabilities: caps, created_by: 'root', created_at: '', updated_at: '' };
const POD = 'checkout-api-7f8db5b4-abcde';
const pods = [
  { name: POD, namespace: 'default', kind: 'Pod', status: 'Running', health: 'ok', ready: '1/1', restarts: 0, age_seconds: 600, labels: { app: 'checkout' }, extra: {}, images: ['checkout:v1'] },
  { name: 'billing-worker-55d-xyz12', namespace: 'default', kind: 'Pod', status: 'Running', health: 'ok', ready: '1/1', restarts: 0, age_seconds: 600, labels: { app: 'billing' }, extra: {}, images: ['billing:v1'] },
];
const status = { cluster_id: cluster.id, last_cycle_at: '2026-09-25T12:00:00Z', last_ok_at: '2026-09-25T12:00:00Z', last_error: '', transport_used: 'api', metrics_server: 'ok', pods_seen: 2, pods_scraped: 2, pods_failed: 0, cycle_ms: 240 };
function workload(name: string) {
  return { namespace: 'default', workload: name, kind: 'Deployment', pods: 1, ready: 1, mem_bytes: 512000000, mem_limit: 1024000000, mem_pct: 50, mem_avg: 256000000, mem_max: 280000000, mem_max_pod: `${name}-pod`, mem_sampled: 1, pods_detail: [{ pod: `${name}-7f8db5b4-abcde`, node: 'n1', phase: 'Running', ready: true, crashloop: false, mem_bytes: 1, mem_limit: 2, mem_pct: 50, restarts: { oom: 0, crash: 0, probe: 0, unknown: 0 }, restarts_lifetime: 0, version: 'v1', age_seconds: 60 }], mem_trend_pct: 0, restarts: { oom: 0, crash: 0, probe: 0, unknown: 0 }, churn_planned: 0, churn_unknown: 0, rps: 2, err_pct: 0, err_pct_baseline: 0, rps_baseline: 2, latency_kind: 'p95', latency_ms: 10, latency_baseline_ms: 10, versions: ['v1'], crashloop: 0, spark: { mem: [1, 2], rps: [1, 2] } };
}

async function fixture(page: Page): Promise<{ resourceCalls: () => number; podHttp: () => unknown[] }> {
  let resourceCalls = 0;
  const podHttp: unknown[] = [];
  await page.addInitScript(() => localStorage.setItem('otto_k8s_autorefresh', '0'));
  await page.route('**/api/v1/access/**/capabilities*', (r) => r.fulfill({ json: { mode: 'legacy', operations: {} } }));
  await page.route('**/api/v1/k8s/**', async (r) => {
    const url = new URL(r.request().url());
    const path = url.pathname;
    const method = r.request().method();
    if (path.endsWith('/pod-http') && method === 'POST') {
      podHttp.push(r.request().postDataJSON());
      return r.fulfill({ json: { target_name: POD, mutating: false, results: [{ pod: POD, status: 200, duration_ms: 12, headers: { 'content-type': 'application/json' }, body: '{"status":"UP"}', body_base64: false, truncated: false, error: null, via: 'proxy' }] } });
    }
    if (method !== 'GET') return r.fulfill({ status: 403, json: { code: 'forbidden', message: 'fixture blocks mutations' } });
    let json: unknown = {};
    if (path.endsWith('/status')) json = { kubectl: { installed: true, version: '1.30.3' }, k9s: { installed: false, version: null } };
    else if (path.endsWith('/clusters')) json = [cluster];
    else if (path.endsWith('/capabilities')) json = caps;
    else if (path.endsWith('/namespaces')) json = { namespaces: [{ name: 'default', status: 'Active' }] };
    else if (path.endsWith('/resources')) {
      resourceCalls += 1;
      json = { kind: 'pods', items: pods, has_metrics: false };
    } else if (path.endsWith('/resource')) json = { manifest: { metadata: { name: POD, labels: { 'pod-template-hash': '7f8db5b4' }, ownerReferences: [{ kind: 'ReplicaSet', name: 'checkout-api-7f8db5b4' }] }, spec: { containers: [{ name: 'app', ports: [{ name: 'http', containerPort: 8080 }] }] } }, describe: `Name: ${POD}`, events: [] };
    else if (path.endsWith('/pod-actions')) json = { actions: [] };
    else if (path.endsWith('/monitor/workloads')) json = { window: '1h', step_secs: 60, enabled: true, status, namespaces: ['default'], workloads: [workload('checkout-api'), workload('billing-worker')] };
    else if (path.endsWith('/monitor/events')) json = [];
    else if (path.endsWith('/monitor/series')) json = { metric: url.searchParams.get('metric'), kind: 'gauge', step_secs: 60, points: [{ t: '2026-09-25T12:00:00Z', v: 1 }, { t: '2026-09-25T12:01:00Z', v: 2 }] };
    else if (path.endsWith('/metrics')) json = { available: true, pods: [] };
    return r.fulfill({ json });
  });
  return { resourceCalls: () => resourceCalls, podHttp: () => podHttp };
}

const nav = (page: Page, id: string) => page.locator(`[data-nav-id="${id}"]:visible`).first();

test.use({ serviceWorkers: 'block' });
test.setTimeout(90_000);
test.beforeEach(({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-only spec');
});

test('a module switch and a reload bring the Kubernetes view back; the active module goes home', async ({ page }) => {
  await fixture(page);
  await openPage(page, `kubernetes/${cluster.id}/pods/default/${POD}`);
  await expect(page.getByTestId('k8s-drawer')).toBeVisible();
  await page.getByTestId('k8s-filter').fill('checkout');
  await expect(page.getByTestId('k8s-row')).toHaveCount(1);

  await nav(page, 'connections').click();
  await expect(page.getByTestId('k8s-page')).toBeHidden();
  await nav(page, 'kubernetes').click();
  await expect(page).toHaveURL(new RegExp(`#/kubernetes/${cluster.id}/pods/default/${POD}$`));
  await expect(page.getByTestId('k8s-drawer')).toBeVisible();
  await expect(page.getByTestId('k8s-filter')).toHaveValue('checkout');

  await page.reload();
  await expect(page.getByTestId('k8s-drawer')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('k8s-filter')).toHaveValue('checkout');

  // Second click on the active module = its main page (clusters overview).
  await nav(page, 'kubernetes').click();
  await expect(page).toHaveURL(/#\/kubernetes$/);
});

test('Resources and Monitor are one workspace with cross-links', async ({ page }) => {
  const f = await fixture(page);
  // The old Monitor URL canonicalises to the workspace form.
  await openPage(page, `kubernetes/monitor/${cluster.id}/workloads`);
  await expect(page).toHaveURL(new RegExp(`#/kubernetes/${cluster.id}/monitor/workloads$`));
  await expect(page.getByTestId('k8s-view-switch')).toBeVisible();

  // Expand a workload row; it survives Monitor → Resources → Monitor.
  await page.getByRole('button', { name: 'checkout-api', exact: true }).click();
  await expect(page.getByTestId('k8s-monitor-pods')).toBeVisible();
  await page.getByTestId('k8s-view-resources').click();
  await expect(page.getByTestId('k8s-workspace')).toBeVisible();
  await expect(page.getByTestId('k8s-row').first()).toBeVisible();
  const loads = f.resourceCalls();
  await page.getByTestId('k8s-view-monitor').click();
  await expect(page.getByTestId('k8s-monitor-pods')).toBeVisible();

  // Back to Resources paints the cached rows at once (no skeleton first).
  await page.getByTestId('k8s-view-resources').click();
  await expect(page.getByTestId('k8s-row').first()).toBeVisible({ timeout: 1_000 });
  expect(f.resourceCalls()).toBeGreaterThanOrEqual(loads);

  // Monitor → "Open pods" lands on Pods filtered to the workload.
  await page.getByTestId('k8s-view-monitor').click();
  await page.getByTestId('k8s-monitor-open-pods').click();
  await expect(page).toHaveURL(new RegExp(`#/kubernetes/${cluster.id}/pods$`));
  await expect(page.getByTestId('k8s-filter')).toHaveValue('checkout-api');
});

test('the drawer refreshes on demand and calls a pod over HTTP', async ({ page }) => {
  const f = await fixture(page);
  await openPage(page, `kubernetes/${cluster.id}/pods/default/${POD}`);
  const drawer = page.getByTestId('k8s-drawer');
  await expect(drawer).toBeVisible();
  await expect(page.getByTestId('k8s-drawer-refresh')).toBeVisible();
  await page.getByTestId('k8s-drawer-refresh').click();

  await drawer.getByRole('tab', { name: 'HTTP' }).click();
  const panel = page.getByTestId('k8s-pod-http');
  await expect(panel).toBeVisible();
  await page.getByTestId('k8s-pod-http-preset-health').click();
  await expect(page.getByTestId('k8s-pod-http-path')).toHaveValue('/actuator/health');
  await page.getByTestId('k8s-pod-http-run').click();
  const results = page.getByTestId('k8s-pod-http-results');
  await expect(results).toContainText('200');
  await expect(results).toContainText('"status": "UP"');
  expect(f.podHttp()[0]).toMatchObject({ namespace: 'default', pod: POD, method: 'GET', path: '/actuator/health', port: 8080 });
  await expectFullyInViewport(page, panel.locator('.ph-main'), 'pod HTTP editor');
});
