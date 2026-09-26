import { test, expect, type Page } from '@playwright/test';
import { openPage, expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

const account = { id: 'ux-account', name: 'Cloud review account', region: 'eu-west-1', auth_mode: 'profile', profile: 'review', environment: 'staging', created_by: 'root', created_at: '', updated_at: '' };
const caps = { server_version: 'v1.30.3', metrics_server: true, argo_rollouts: true, argocd: true, checked_at: '' };
const cluster = { id: 'ux-cluster', name: 'Cloud review cluster', source: 'kubeconfig', context_name: 'review', default_namespace: 'default', environment: 'staging', capabilities: caps, created_by: 'root', created_at: '', updated_at: '' };
const pod = { name: 'checkout-api-very-long-production-pod-name-7f8db5b4-abcde', namespace: 'default', kind: 'Pod', status: 'Running', health: 'ok', ready: '1/1', restarts: 0, age_seconds: 600, labels: { app: 'checkout' }, extra: {}, images: ['registry.example.test/checkout:v1'] };
const instance = { instance_id: 'i-review', name: 'checkout-api', state: 'running', type: 't3.small', az: 'eu-west-1a', tags: {}, private_ip: '10.0.0.8' };

async function cloudFixture(page: Page) {
  await page.addInitScript(() => localStorage.setItem('otto_k8s_autorefresh', '0'));
  await page.route('**/api/v1/access/**/capabilities*', r => r.fulfill({ json: { mode: 'legacy', operations: {} } }));
  await page.route('**/api/v1/aws/**', async r => {
    const path = new URL(r.request().url()).pathname;
    if (r.request().method() !== 'GET') return r.fulfill({ status: 403, json: { code: 'forbidden', message: 'Review fixture blocks cloud mutations' } });
    let json: unknown = {};
    if (path.endsWith('/status')) json = { installed: true, version: '2.27.19', install: { state: 'idle' } };
    else if (path.endsWith('/accounts')) json = [account];
    else if (path.endsWith('/regions')) json = { regions: [{ code: 'eu-west-1', name: 'Ireland' }] };
    else if (path.endsWith('/permissions')) json = { services: { s3: 'allowed', sqs: 'allowed', ec2: 'allowed', athena: 'allowed', eks: 'allowed', rds: 'allowed' } };
    else if (path.endsWith('/ec2/instances')) json = { instances: [instance] };
    else if (path.endsWith('/ec2/instances/i-review')) json = { ...instance, raw: { InstanceId: instance.instance_id } };
    else if (path.endsWith('/metrics')) json = { namespace: 'AWS/EC2', dim_name: 'InstanceId', dim_value: 'i-review', range: '1h', period_seconds: 60, start: '', end: '', series: [] };
    else if (path.endsWith('/s3/buckets')) json = { buckets: [{ name: 'review-artifacts', creation_date: '2026-09-01' }] };
    else if (path.endsWith('/sqs/queues')) json = { queues: [] };
    else if (path.endsWith('/eks/clusters')) json = { clusters: [] };
    else if (path.endsWith('/rds/instances')) json = { instances: [] };
    else if (path.endsWith('/workgroups')) json = { workgroups: [] };
    else if (path.endsWith('/databases')) json = { databases: [] };
    else if (path.endsWith('/history')) json = { executions: [] };
    else if (path.endsWith('/discover')) json = { profiles: [] };
    return r.fulfill({ json });
  });
  await page.route('**/api/v1/k8s/**', async r => {
    const path = new URL(r.request().url()).pathname;
    if (r.request().method() !== 'GET') return r.fulfill({ status: 403, json: { code: 'forbidden', message: 'Review fixture blocks cluster mutations' } });
    let json: unknown = {};
    if (path.endsWith('/status')) json = { kubectl: { installed: true, version: '1.30.3' }, k9s: { installed: true, version: '0.32.5' } };
    else if (path.endsWith('/clusters')) json = [cluster];
    else if (path.endsWith('/capabilities')) json = caps;
    else if (path.endsWith('/namespaces')) json = { namespaces: Array.from({ length: 90 }, (_, i) => ({ name: i ? `namespace-${String(i).padStart(3, '0')}` : 'default', status: 'Active' })) };
    else if (path.endsWith('/resources')) json = { kind: 'pods', items: [pod], has_metrics: false };
    else if (path.endsWith('/resource')) json = { manifest: { metadata: { name: pod.name }, spec: { containers: [{ name: 'app' }] } }, describe: 'Name: ' + pod.name, events: [] };
    else if (path.endsWith('/containers')) json = { containers: [{ name: 'app', image: 'checkout:v1', ready: true, state: 'running', restarts: 0, init: false }] };
    else if (path.endsWith('/logs')) return r.fulfill({ contentType: 'text/plain', body: 'checkout started\nrequest complete\n' });
    return r.fulfill({ json });
  });
}

test.use({ serviceWorkers: 'block' });
test.setTimeout(90_000);
test.beforeEach(async ({ page }) => cloudFixture(page));

const status = { cluster_id: 'ux-cluster', last_cycle_at: '2026-09-25T12:00:00Z', last_ok_at: '2026-09-25T12:00:00Z', last_error: '', transport_used: 'api', metrics_server: 'ok', pods_seen: 3, pods_scraped: 3, pods_failed: 0, cycle_ms: 240 };
const restartCounts = { oom: 0, crash: 1, probe: 0, unknown: 0 };
function workload(name: string) {
  return { namespace: 'default', workload: name, kind: 'Deployment', pods: 2, ready: 2, mem_bytes: 512000000, mem_limit: 1024000000, mem_pct: 50, mem_avg: 256000000, mem_max: 280000000, mem_max_pod: name + '-pod', mem_sampled: 2, pods_detail: [], mem_trend_pct: 12, restarts: restartCounts, churn_planned: 1, churn_unknown: 0, rps: 24, err_pct: 0.1, err_pct_baseline: 0.1, rps_baseline: 20, latency_kind: 'p95', latency_ms: 70, latency_baseline_ms: 60, versions: ['v2.4.0'], crashloop: 0, spark: { mem: [1, 2, 3], rps: [3, 2, 4] } };
}
function event(name: string, kind = 'crash') {
  return { ts: '2026-09-25 12:00:00', namespace: 'default', workload: name, pod: name + '-pod', container: 'app', kind: 'restart', class: kind, reason: 'Restarted', exit_code: 1, detail: { prev_restarts: 0, next_restarts: 1 }, actor: '' };
}
async function monitoring(page: Page) {
  await page.route('**/monitor/workloads?*', r => r.fulfill({ json: { window: '1h', step_secs: 60, enabled: true, status, namespaces: ['default', 'payments'], workloads: [workload('checkout-api'), workload('billing-worker')] } }));
  await page.route('**/monitor/events?*', r => r.fulfill({ json: [event('checkout-api')] }));
  await page.route('**/monitor/series?*', r => r.fulfill({ json: { metric: new URL(r.request().url()).searchParams.get('metric'), kind: 'gauge', step_secs: 60, points: [{ t: '2026-09-25T12:00:00Z', v: 200 }, { t: '2026-09-25T12:01:00Z', v: 260 }] } }));
}

test('monitor workload expansion and sorting work from the keyboard', async ({ page }) => {
  await monitoring(page);
  await openPage(page, 'kubernetes/monitor/ux-cluster/workloads');
  const toggle = page.getByRole('button', { name: 'checkout-api', exact: true });
  await expect(toggle).toBeVisible();
  await toggle.focus(); await page.keyboard.press('Enter');
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByTestId('k8s-monitor-pods')).toBeVisible();
  await page.getByRole('button', { name: 'Workload', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.wl-row').first()).toContainText('billing-worker');
});

test('monitor events ignore a previous filter response', async ({ page }) => {
  await monitoring(page);
  let release!: () => void;
  const gate = new Promise<void>(r => { release = r; });
  let requested = false;
  await page.route('**/monitor/events?*', async r => {
    const cls = new URL(r.request().url()).searchParams.get('class');
    if (cls === 'oom') { requested = true; await gate; }
    await r.fulfill({ json: [event(cls === 'oom' ? 'old-oom-workload' : 'current-crash-workload', cls || 'crash')] });
  });
  await openPage(page, 'kubernetes/monitor/ux-cluster/events');
  await page.getByLabel('Event class').selectOption('oom');
  await expect.poll(() => requested).toBe(true);
  await page.getByLabel('Event class').selectOption('crash');
  await expect(page.getByTestId('k8s-monitor-events')).toContainText('current-crash-workload');
  const response = page.waitForResponse(r => r.url().includes('/monitor/events?') && r.url().includes('class=oom'));
  release(); await response; await page.evaluate(() => new Promise(requestAnimationFrame));
  await expect(page.getByTestId('k8s-monitor-events')).not.toContainText('old-oom-workload');
});

test('monitor trends ignore a previously expanded workload response', async ({ page }) => {
  await monitoring(page);
  let release!: () => void;
  const gate = new Promise<void>(r => { release = r; });
  let requested = 0;
  await page.route('**/monitor/series?*', async r => {
    const query = new URL(r.request().url()).searchParams;
    if (query.get('workload') === 'checkout-api') { requested++; await gate; }
    await r.fulfill({ json: { metric: query.get('workload') === 'checkout-api' ? 'old-workload-metric' : 'current-workload-metric', kind: 'gauge', step_secs: 60, points: [{ t: '2026-09-25T12:00:00Z', v: 200 }] } });
  });
  await openPage(page, 'kubernetes/monitor/ux-cluster/workloads');
  await page.locator('.wl-row').filter({ hasText: 'checkout-api' }).click();
  await expect.poll(() => requested).toBe(2);
  await page.locator('.wl-row').filter({ hasText: 'billing-worker' }).click();
  await expect(page.locator('.detail')).toContainText('current-workload-metric');
  const response = page.waitForResponse(r => r.url().includes('/monitor/series?') && r.url().includes('workload=checkout-api'));
  release(); await response; await page.evaluate(() => new Promise(requestAnimationFrame));
  await expect(page.locator('.detail')).not.toContainText('old-workload-metric');
});

test('monitor failed trends show an inline retry and recover', async ({ page }) => {
  await monitoring(page);
  let failed = true;
  await page.route('**/monitor/series?*', r => failed ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic collector unavailable' } }) : r.fulfill({ json: { metric: 'recovered-metric', kind: 'gauge', step_secs: 60, points: [] } }));
  await openPage(page, 'kubernetes/monitor/ux-cluster/workloads');
  await page.locator('.wl-row').filter({ hasText: 'checkout-api' }).click();
  await expect(page.getByText("Couldn't load trends")).toBeVisible();
  failed = false;
  await page.getByRole('button', { name: 'Retry trends' }).click();
  await expect(page.locator('.detail')).toContainText('recovered-metric');
});

for (const variant of [
  { name: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900 },
  { name: 'native-dark', theme: 'native', scheme: 'dark', width: 1440, height: 900 },
  { name: 'warm-light-phone', theme: 'warm', scheme: 'light', width: 375, height: 812 },
  { name: 'warm-dark-tablet-rtl', theme: 'warm', scheme: 'dark', width: 834, height: 1112 },
  { name: 'pro-dark', theme: 'pro-dark', scheme: 'dark', width: 1440, height: 900 },
]) {
  test(`loaded monitoring inspection ${variant.name}`, async ({ page }) => {
    await page.setViewportSize(variant);
    await page.addInitScript(v => {
      localStorage.setItem('otto_theme', v.theme); localStorage.setItem('otto_scheme', v.scheme);
      localStorage.setItem('otto_direction', v.name.includes('rtl') ? 'rtl' : 'ltr');
    }, variant);
    await monitoring(page);
    await openPage(page, 'kubernetes/monitor/ux-cluster/workloads');
    await expect(page.locator('.wl-row')).toHaveCount(2);
    await expectFullyInViewport(page, page.getByRole('radio', { name: '7d', exact: true }));
    await page.screenshot({ path: `/tmp/otto-ux-r3-cloud-${variant.name}-workloads.png` });
    await page.getByLabel('Filter workloads').fill('checkout');
    await expect(page.locator('.wl-row')).toHaveCount(1);
    await expectNoHorizontalOverflow(page);
    await page.getByTestId('k8s-monitor-tab-events').click();
    await expect(page.getByTestId('k8s-monitor-events')).toContainText('checkout-api');
    await page.screenshot({ path: `/tmp/otto-ux-r3-cloud-${variant.name}-events.png` });
    await expectNoHorizontalOverflow(page);
    const clipped = await page.locator('.timeline').evaluate(el => el.scrollWidth > el.clientWidth + 1);
    expect(clipped).toBe(false);
    const overlaps = await page.locator('.timeline li').first().evaluate(el => {
      const range = document.createRange();
      range.selectNodeContents(el.querySelector('.tclass')!);
      const a = range.getBoundingClientRect();
      const b = el.querySelector('.twl')!.getBoundingClientRect();
      return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
    });
    expect(overlaps).toBe(false);
  });
}

test('S3 pagination preserves rows and CSV and binary previews', async ({ page }) => {
  let secondPage = false;
  await page.route('**/s3/buckets/review-artifacts/objects*', r => {
    const more = new URL(r.request().url()).searchParams.get('token');
    secondPage ||= more === 'next-page';
    return r.fulfill({ json: { prefix: '', prefixes: [], objects: [{ key: more ? 'archive.bin' : 'report.csv', size: 128, last_modified: '2026-09-25T12:00:00Z', storage_class: 'STANDARD' }], is_truncated: !more, next_token: more ? null : 'next-page' } });
  });
  await page.route('**/s3/buckets/review-artifacts/preview?*', r => {
    const binary = new URL(r.request().url()).searchParams.get('key') === 'archive.bin';
    return r.fulfill({ json: { binary, text: binary ? null : 'name,count\ncheckout,12\nbilling,8', content_type: binary ? 'application/octet-stream' : 'text/csv', truncated: false } });
  });
  await openPage(page, 'aws/ux-account/s3/review-artifacts');
  await page.getByRole('button', { name: 'Load more', exact: true }).click();
  await expect.poll(() => secondPage).toBe(true);
  await expect(page.getByRole('row').filter({ hasText: 'report.csv' })).toHaveCount(1);
  await page.getByRole('row').filter({ hasText: 'report.csv' }).dblclick();
  await expect(page.getByRole('cell', { name: 'checkout', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Close preview' }).click();
  await page.getByRole('row').filter({ hasText: 'archive.bin' }).dblclick();
  await expect(page.getByText('Binary content', { exact: false })).toBeVisible();
});

test('CloudWatch populated charts recover after refresh and select a new range', async ({ page }) => {
  let failed = false;
  let lastRange = '';
  await page.route('**/metrics?*', r => {
    lastRange = new URL(r.request().url()).searchParams.get('range') || '';
    if (failed) return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic CloudWatch unavailable' } });
    return r.fulfill({ json: { namespace: 'AWS/EC2', dim_name: 'InstanceId', dim_value: 'i-review', range: lastRange, period_seconds: 60, start: '2026-09-25T12:00:00Z', end: '2026-09-25T12:03:00Z', series: [{ id: 'cpu', metric: 'CPUUtilization', stat: 'Average', unit: 'percent', label: 'CPU', points: [12, 24, 36].map((v, i) => ({ t: `2026-09-25T12:0${i}:00Z`, v })), current: 36, min: 12, max: 36, sum: 72, avg: 24 }] } });
  });
  await openPage(page, 'aws/ux-account/ec2');
  await page.getByRole('row').filter({ hasText: 'checkout-api' }).click();
  await page.getByRole('tab', { name: 'Metrics', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'CPU utilization' })).toBeVisible();
  failed = true;
  await page.getByRole('button', { name: 'Refresh metrics' }).click();
  await expect(page.getByText('Showing the last successful load', { exact: false })).toBeVisible();
  failed = false;
  await page.getByRole('radio', { name: '24h', exact: true }).click();
  await expect.poll(() => lastRange).toBe('24h');
  await expect(page.getByText('Showing the last successful load', { exact: false })).toBeHidden();
  await expect(page.getByRole('heading', { name: 'CPU utilization' })).toBeVisible();
  await page.screenshot({ path: '/tmp/otto-ux-r3-cloud-cloudwatch.png' });
});

async function fleet(page: Page) {
  await page.route('**/monitor/fleet/filters?*', r => r.fulfill({ json: { window: '24h', clusters: [{ ...cluster, rows: 3 }], namespaces: [{ cluster_id: cluster.id, namespace: 'default' }], workloads: [{ cluster_id: cluster.id, namespace: 'default', workload: 'checkout-api' }], pods: [{ cluster_id: cluster.id, namespace: 'default', workload: 'checkout-api', pod: 'checkout-pod' }] } }));
  await page.route('**/monitor/fleet/table?*', r => {
    const q = new URL(r.request().url()).searchParams;
    const grouped = q.get('group') === 'pod';
    return r.fulfill({ json: { window: '24h', group: grouped ? 'pod' : 'workload', sort: 'restarts', dir: 'desc', offset: 0, total: 1, rows: [{ cluster, cluster_id: cluster.id, namespace: 'default', workload: 'checkout-api', pod: grouped ? 'checkout-pod' : '', pods: 1, restarts: restartCounts, churn: 1, mem_last: 240000000, mem_avg: 230000000, mem_max: 270000000, rps: 24, err_pct: 0.1, latency_kind: 'p95', latency_ms: 70 }] } });
  });
  await page.route('**/monitor/fleet/series?*', r => r.fulfill({ json: { window: '24h', metric: new URL(r.request().url()).searchParams.get('metric'), unit: 'count', by: 'cluster', step_secs: 60, series: [{ key: cluster.id, label: cluster.name, points: [2, 4, 3].map((v, i) => ({ t: `2026-09-25T12:0${i}:00Z`, v })) }] } }));
  await page.route('**/monitor/fleet/events?*', r => r.fulfill({ json: { window: '24h', sort: 'ts', dir: 'desc', total: 1, offset: 0, rows: [{ ...event('checkout-api'), cluster, cluster_id: cluster.id }] } }));
}

test('fleet keyboard drill-down reaches pod events and clear restores all workloads', async ({ page }) => {
  await fleet(page);
  await openPage(page, 'kubernetes/monitor/fleet/table');
  const workloadButton = page.getByRole('button', { name: 'Show pods for checkout-api', exact: true });
  await expect(workloadButton).toBeVisible();
  await workloadButton.focus(); await page.keyboard.press('Enter');
  await expect(page.getByRole('radio', { name: 'Pods', exact: true })).toHaveAttribute('aria-checked', 'true');
  const podButton = page.getByRole('button', { name: 'Show events for checkout-pod', exact: true });
  await podButton.focus(); await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/fleet\/events/);
  await expect(page.getByLabel('Pod', { exact: true })).toHaveValue('checkout-pod');
  await page.getByTestId('k8s-fleet-clear').click();
  await expect(page.getByLabel('Workload', { exact: true })).toHaveValue('');
  await page.getByTestId('k8s-fleet-tab-overview').click();
  await expect(page.getByTestId('k8s-fleet-kpis')).toContainText('24.0/s');
  await expect(page.getByTestId('k8s-fleet-charts').locator('.chart')).toHaveCount(5);
  await page.screenshot({ path: '/tmp/otto-ux-r3-cloud-fleet-desktop.png' });
  await page.setViewportSize({ width: 375, height: 812 });
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: '/tmp/otto-ux-r3-cloud-fleet-phone.png' });
});

test('phone event classification and resource identity occupy separate readable tracks', async ({ page }) => {
  await monitoring(page);
  await page.setViewportSize({ width: 375, height: 812 });
  await openPage(page, 'kubernetes/monitor/ux-cluster/events');
  const row = page.locator('.timeline li').first();
  await expect(row).toContainText('checkout-api');
  const overlaps = await row.evaluate(el => {
    const range = document.createRange();
      range.selectNodeContents(el.querySelector('.tclass')!);
      const a = range.getBoundingClientRect();
    const b = el.querySelector('.twl')!.getBoundingClientRect();
    return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
  });
  expect(overlaps).toBe(false);
});
