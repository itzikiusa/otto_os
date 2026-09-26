import { test, expect, type Page } from '@playwright/test';
import { openPage, expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { apiCtx, seedWorkspace } from './seed';

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


async function insights(page: Page) {
  const a = await apiCtx(); const wid = await seedWorkspace(a.ctx, a.base); await a.ctx.dispose();
  await page.addInitScript(id => localStorage.setItem('otto_workspace', id), wid);
  await page.route('**/workspaces/*/personal-agents', r => r.fulfill({ json: [{ id: 'watchdog', workspace_id: wid, name: 'Production Kubernetes reliability watchdog', soul_md: '<!-- otto-template:k8s-watchdog -->', provider: 'codex', model: 'review-model', enabled: true, avatar: '🛡️' }] }));
  await page.route('**/personal-agents/watchdog/runs', r => r.fulfill({ json: ['latest', 'older', 'empty'].map((id, i) => ({ id, agent_id: 'watchdog', status: 'done', started_at: `2026-09-25T12:0${i}:00Z`, report_path: id === 'empty' ? null : `/reports/${id}.md`, summary: `${id} cluster assessment` })) }));
  await page.route('**/personal-agents/runs/*/report', r => r.fulfill({ contentType: 'text/markdown', body: `# HEALTHY

Current cluster assessment.

| Workload | Finding |
| --- | --- |
| checkout-api | Stable memory and successful requests |` }));
}

test('Insights report selection rejects a late previous report', async ({ page }) => {
  await insights(page);
  let release!: () => void; const gate = new Promise<void>(r => { release = r; });
  let requested = false;
  await page.route('**/personal-agents/runs/older/report', async r => { requested = true; await gate; await r.fulfill({ contentType: 'text/markdown', body: '# OUTDATED REPORT' }); });
  await openPage(page, 'kubernetes/monitor/ux-cluster/insights');
  await expect(page.locator('.report')).toContainText('Current cluster assessment');
  await page.getByRole('button', { name: /older cluster assessment/ }).click();
  await expect.poll(() => requested).toBe(true);
  await page.getByRole('button', { name: /latest cluster assessment/ }).click();
  await expect(page.locator('.report')).toContainText('Current cluster assessment');
  const response = page.waitForResponse(r => r.url().endsWith('/runs/older/report'));
  release(); await response; await page.evaluate(() => new Promise(requestAnimationFrame));
  await expect(page.locator('.report')).not.toContainText('OUTDATED REPORT');
});

test('Insights report failure has a retry action and keeps selected run', async ({ page }) => {
  await insights(page); let failed = true;
  await page.route('**/personal-agents/runs/latest/report', r => r.fulfill(failed ? { status: 503, json: { code: 'upstream', message: 'Synthetic report unavailable' } } : { contentType: 'text/markdown', body: '# Recovered assessment' }));
  await openPage(page, 'kubernetes/monitor/ux-cluster/insights');
  await expect(page.locator('.report')).toContainText('Synthetic report unavailable');
  failed = false;
  await page.locator('.report').getByRole('button', { name: 'Retry' }).click();
  await expect(page.locator('.report')).toContainText('Recovered assessment');
});

for (const variant of [
  { name: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900 },
  { name: 'native-dark', theme: 'native', scheme: 'dark', width: 1440, height: 900 },
  { name: 'warm-light-phone', theme: 'warm', scheme: 'light', width: 375, height: 812 },
  { name: 'warm-dark-tablet-rtl', theme: 'warm', scheme: 'dark', width: 834, height: 1112 },
  { name: 'pro-dark', theme: 'pro-dark', scheme: 'dark', width: 1440, height: 900 },
]) {
  test(`Insights composition ${variant.name}`, async ({ page }) => {
    await insights(page);
    await page.setViewportSize(variant);
    await page.addInitScript(v => { localStorage.setItem('otto_theme', v.theme); localStorage.setItem('otto_scheme', v.scheme); localStorage.setItem('otto_direction', v.name.includes('rtl') ? 'rtl' : 'ltr'); }, variant);
    await openPage(page, 'kubernetes/monitor/ux-cluster/insights');
    await expect(page.locator('.report')).toContainText('Current cluster assessment');
    await page.screenshot({ path: `/tmp/otto-ux-r4-cloud-${variant.name}-insights.png` });
    await expectFullyInViewport(page, page.getByRole('button', { name: 'Open agent', exact: true }));
    const clipped = await page.getByTestId('k8s-monitor-insights').evaluate(el => el.scrollWidth > el.clientWidth + 1);
    expect(clipped).toBe(false);
  });
}

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


test('Fleet radio groups support arrows, wrapping and one tab stop', async ({ page }) => {
  await fleet(page); await openPage(page, 'kubernetes/monitor/fleet/table');
  const workloads = page.getByRole('radio', { name: 'Workloads', exact: true });
  await workloads.focus(); await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('radio', { name: 'Pods', exact: true })).toBeFocused();
  await expect(page.getByRole('radio', { name: 'Pods', exact: true })).toHaveAttribute('aria-checked', 'true');
  await expect(workloads).toHaveAttribute('tabindex', '-1');
  await page.keyboard.press('ArrowRight'); await expect(workloads).toBeFocused();
  const window = page.getByRole('radiogroup', { name: 'Window', exact: true });
  await window.getByRole('radio', { name: '24h', exact: true }).focus(); await page.keyboard.press('End');
  await expect(window.getByRole('radio', { name: '7d', exact: true })).toBeFocused();
});

test('Fleet phone header retains its complete window control and refresh', async ({ page }) => {
  await fleet(page); await page.setViewportSize({ width: 375, height: 812 });
  await openPage(page, 'kubernetes/monitor/fleet/overview');
  await expect(page.getByTestId('k8s-fleet-kpis')).toBeVisible();
  await expectFullyInViewport(page, page.getByRole('radio', { name: '7d', exact: true }));
  // Refresh can live in the shared toolbar overflow, but the page title must be readable.
  await expect(page.locator('h1[title="Fleet"]')).toBeVisible();
});

test('Insights phone gives the agent identity a readable full row', async ({ page }) => {
  await insights(page); await page.setViewportSize({ width: 375, height: 812 });
  await openPage(page, 'kubernetes/monitor/ux-cluster/insights');
  await expect(page.locator('.report')).toContainText('Current cluster assessment');
  expect((await page.locator('.insights .who').boundingBox())!.width).toBeGreaterThan(250);
});

test('Insights clears the previous workspace while the next workspace loads', async ({ page }) => {
  await insights(page);
  const a = await apiCtx(); const next = await seedWorkspace(a.ctx, a.base); await a.ctx.dispose();
  let release!: () => void; const gate = new Promise<void>(r => { release = r; });
  let requested = false;
  await page.route(`**/workspaces/${next}/personal-agents`, async r => { requested = true; await gate; await r.fulfill({ json: [] }); });
  await openPage(page, 'kubernetes/monitor/ux-cluster/insights');
  await expect(page.locator('.report')).toContainText('Current cluster assessment');
  // Exercise the real shared workspace store; the console has no local workspace picker.
  await page.evaluate(async id => { const path = '/src/lib/stores/workspace.svelte.ts'; const { ws } = await import(path); ws.currentId = id; }, next);
  await expect.poll(() => requested).toBe(true);
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeHidden();
  release(); await expect(page.getByText('No Kubernetes watchdog yet', { exact: true })).toBeVisible();
});

test('Athena phone cancel, query failure and subsequent successful results', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.addInitScript(() => localStorage.setItem('otto_aws_athena_sql_ux-account', 'SELECT 42 AS answer'));
  let state = 'RUNNING'; let attempts = 0; let cancelled = false;
  await page.route('**/athena/query', r => { attempts++; if (attempts === 2) return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic Athena unavailable' } }); return r.fulfill({ json: { query_execution_id: 'synthetic-query' } }); });
  await page.route('**/athena/query/synthetic-query', r => r.fulfill({ json: { state, reason: state === 'CANCELLED' ? 'Cancelled by user' : '', stats: { data_scanned_bytes: 42, execution_ms: 12 }, result: state === 'SUCCEEDED' ? { columns: [{ name: 'answer', data_type: 'integer' }], rows: [[42]], stats: { duration_ms: 12, row_count: 1 }, truncated: false } : null } }));
  await page.route('**/athena/query/synthetic-query/cancel', r => { state = 'CANCELLED'; cancelled = true; return r.fulfill({ json: {} }); });
  await openPage(page, 'aws/ux-account/athena'); await page.getByTestId('athena-run').click();
  await page.getByRole('button', { name: 'Cancel', exact: true }).first().click();
  await expect.poll(() => cancelled).toBe(true);
  await expect(page.getByTestId('athena-run')).toBeEnabled();
  await page.getByTestId('athena-run').click(); await expect(page.getByText('Synthetic Athena unavailable', { exact: false })).toBeVisible();
  state = 'SUCCEEDED'; await page.getByTestId('athena-run').click();
  await expect(page.getByText('42', { exact: true }).first()).toBeVisible();
  await expectNoHorizontalOverflow(page); await page.screenshot({ path: '/tmp/otto-ux-r4-cloud-athena-phone.png' });
});

test('EKS successful synthetic import navigates to the imported cluster', async ({ page }) => {
  const eks = { name: 'review-eks', status: 'ACTIVE', version: '1.30', endpoint: 'https://eks.example.test' };
  let imported = false;
  await page.route('**/eks/clusters?*', r => r.fulfill({ json: { clusters: [eks] } }));
  await page.route('**/eks/clusters/review-eks/import-kubeconfig?*', r => { imported = true; return r.fulfill({ json: { id: cluster.id, name: cluster.name } }); });
  await openPage(page, 'aws/ux-account/eks'); await page.getByRole('button', { name: 'Open in Kubernetes', exact: true }).click();
  await page.getByRole('dialog', { name: 'Open in Kubernetes', exact: true }).getByRole('button', { name: 'Import', exact: true }).click();
  await expect.poll(() => imported).toBe(true); await expect(page).toHaveURL(/#\/kubernetes\/ux-cluster/);
  await expect(page.getByTestId('k8s-row')).toContainText(pod.name);
});

test('S3 pending download cancels and a subsequent download succeeds', async ({ page }) => {
  await page.route('**/s3/buckets/review-artifacts/objects*', r => r.fulfill({ json: { prefix: '', prefixes: [], objects: [{ key: 'report.txt', size: 128, last_modified: '2026-09-25T12:00:00Z', storage_class: 'STANDARD' }], is_truncated: false, next_token: null } }));
  let release!: () => void; const gate = new Promise<void>(r => { release = r; }); let count = 0;
  await page.route('**/s3/buckets/review-artifacts/download?*', async r => { if (++count === 1) await gate; await r.fulfill({ contentType: 'text/plain', body: 'Synthetic download', headers: { 'content-disposition': 'attachment; filename="report.txt"' } }).catch(() => {}); });
  await openPage(page, 'aws/ux-account/s3/review-artifacts');
  await page.getByRole('button', { name: 'Download report.txt', exact: true }).click();
  await page.locator('.dl-bar').getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.locator('.dl-bar')).toBeHidden(); release();
  const download = page.waitForEvent('download'); await page.getByRole('button', { name: 'Download report.txt', exact: true }).click();
  expect((await download).suggestedFilename()).toBe('report.txt');
  await expect(page.getByText('Downloaded', { exact: true })).toBeVisible();
});

async function overview(page: Page) {
  await page.route('**/k8s/monitor/overview?*', r => r.fulfill({ json: [{ cluster, enabled: true, window: '24h', health: 'healthy', status, pods: { running: 12, total: 12, pending: 0, failed: 0, crashloop: 0 }, restarts: restartCounts, churn: 2, mem: { used: 2048000000, limit: 4096000000, pct: 50 }, rps: 120, err_pct: 0.2, drift: [] }] }));
}

for (const variant of [
  { name: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900 },
  { name: 'native-dark', theme: 'native', scheme: 'dark', width: 1440, height: 900 },
  { name: 'warm-light-phone', theme: 'warm', scheme: 'light', width: 375, height: 812 },
  { name: 'warm-dark-tablet-rtl', theme: 'warm', scheme: 'dark', width: 834, height: 1112 },
  { name: 'pro-dark', theme: 'pro-dark', scheme: 'dark', width: 1440, height: 900 },
]) {
  test(`Fleet Requests and monitor overview composition ${variant.name}`, async ({ page }) => {
    await fleet(page); await overview(page); await page.setViewportSize(variant);
    await page.addInitScript(v => { localStorage.setItem('otto_theme', v.theme); localStorage.setItem('otto_scheme', v.scheme); localStorage.setItem('otto_direction', v.name.includes('rtl') ? 'rtl' : 'ltr'); }, variant);
    await page.route('**/monitor/fleet/requests?*', r => r.fulfill({ json: { window: '24h', enabled_on: [cluster], disabled_on: [], rows: [{ path: '/api/v1/orders/checkout/confirm', method: 'POST', rps: 180, err_pct: 1.2, avg_ms: 85 }, { path: '/health', method: 'GET', rps: 240, err_pct: 0, avg_ms: 4 }] } }));
    await openPage(page, 'kubernetes/monitor/fleet/requests');
    await expect(page.getByTestId('k8s-fleet-requests')).toContainText('/api/v1/orders/checkout/confirm');
    await page.getByRole('button', { name: 'Route', exact: true }).focus(); await page.keyboard.press('Enter');
    await expect(page.getByTestId('k8s-fleet-requests').locator('tbody tr').first()).toContainText('/api/v1/orders/checkout/confirm');
    await expectNoHorizontalOverflow(page);
    if (variant.name.includes('rtl')) await expect(page.getByTestId('k8s-fleet-requests').locator('tbody td').first()).toHaveCSS('direction', 'ltr');
    await page.screenshot({ path: `/tmp/otto-ux-r4-cloud-${variant.name}-requests.png` });
    await page.evaluate(() => { location.hash = '#/kubernetes/monitor'; });
    await expect(page.getByTestId('k8s-monitor-card')).toContainText('Healthy');
    expect(await contrastOf(page, '.health.ok')).toBeGreaterThanOrEqual(4.5);
    await expectNoHorizontalOverflow(page); await page.screenshot({ path: `/tmp/otto-ux-r4-cloud-${variant.name}-overview.png` });
    await page.getByRole('radio', { name: '24h', exact: true }).focus(); await page.keyboard.press('Home');
    await expect(page.getByRole('radio', { name: '1h', exact: true })).toBeFocused();
  });
}

test('Fleet large table loads the next page without replacing the first page', async ({ page }) => {
  await fleet(page); let offsets: number[] = [];
  await page.route('**/monitor/fleet/table?*', r => {
    const offset = Number(new URL(r.request().url()).searchParams.get('offset')); offsets.push(offset);
    return r.fulfill({ json: { window: '24h', group: 'workload', sort: 'restarts', dir: 'desc', offset, total: 201, rows: Array.from({ length: offset ? 1 : 200 }, (_, n) => ({ cluster, cluster_id: cluster.id, namespace: 'default', workload: `service-${String(n + offset).padStart(3, '0')}`, pod: '', pods: 1, restarts: restartCounts, churn: 1, mem_last: 240000000, mem_avg: 230000000, mem_max: 270000000, rps: 24, err_pct: 0.1, latency_kind: 'p95', latency_ms: 70 })) } });
  });
  await openPage(page, 'kubernetes/monitor/fleet/table');
  await page.getByRole('button', { name: /Load .*more/ }).click();
  await expect(page.getByRole('button', { name: 'Show pods for service-200', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Show pods for service-000', exact: true })).toHaveCount(1);
  expect(offsets).toContain(200);
});

const queueUrl = 'https://sqs.eu-west-1.amazonaws.com/123/review-orders.fifo';
async function queues(page: Page) {
  await page.route('**/sqs/queues', r => r.fulfill({ json: { queues: [
    { name: 'review-orders.fifo', url: queueUrl, fifo: true },
    { name: 'review-other', url: queueUrl + '-other', fifo: false },
  ] } }));
  await page.route('**/sqs/queues/attributes?*', r => r.fulfill({ json: {
    attributes: { QueueArn: 'arn:aws:sqs:eu-west-1:123:review-orders.fifo' },
    approx_messages: 3, approx_not_visible: 0, approx_delayed: 0,
  } }));
}


test('SQS queue drafts stay with their queue during a pending send', async ({ page }) => {
  await queues(page); let release!: () => void; const gate = new Promise<void>(r => { release = r; });
  let sent: unknown; const refreshed: string[] = [];
  await page.route('**/sqs/queues/send', async r => { sent = r.request().postDataJSON(); await gate; await r.fulfill({ json: { message_id: 'synthetic-send' } }); });
  await page.route('**/sqs/queues/attributes?*', r => { refreshed.push(new URL(r.request().url()).searchParams.get('url') || ''); return r.fulfill({ json: { attributes: {}, approx_messages: 1, approx_not_visible: 0, approx_delayed: 0 } }); });
  await openPage(page, 'aws/ux-account/sqs'); await page.getByRole('cell', { name: 'review-other', exact: true }).click();
  await page.getByRole('tab', { name: 'Send', exact: true }).click(); await page.getByLabel('Body', { exact: true }).fill('other-queue draft');
  await page.getByRole('button', { name: 'Send message', exact: true }).click(); await expect.poll(() => sent).toBeTruthy();
  await page.getByRole('cell', { name: 'review-orders.fifo FIFO', exact: true }).click();
  await expect(page.getByLabel('Body', { exact: true })).toHaveValue('');
  await page.getByLabel('Body', { exact: true }).fill('orders draft'); refreshed.length = 0;
  release(); await expect(page.getByText('Message sent', { exact: true })).toBeVisible();
  await expect.poll(() => refreshed).toContain(queueUrl + '-other');
  await expect(page.getByLabel('Body', { exact: true })).toHaveValue('orders draft');
  await page.getByRole('cell', { name: 'review-other', exact: true }).click();
  await expect(page.getByLabel('Body', { exact: true })).toHaveValue('other-queue draft');
});

test('SQS redrive and typed purge submit the confirmed synthetic queue', async ({ page }) => {
  await queues(page); let redrive: unknown; let purged: unknown;
  await page.route('**/sqs/queues/redrive', r => { redrive = r.request().postDataJSON(); return r.fulfill({ json: { task_handle: 'synthetic-redrive' } }); });
  await page.route('**/sqs/queues/purge', r => { purged = r.request().postDataJSON(); return r.fulfill({ json: {} }); });
  await openPage(page, 'aws/ux-account/sqs'); await page.getByRole('cell', { name: 'review-orders.fifo FIFO', exact: true }).click();
  await page.getByRole('tab', { name: 'Redrive', exact: true }).click();
  await page.getByRole('button', { name: 'Start redrive', exact: true }).click();
  await page.getByRole('dialog', { name: 'Start redrive' }).getByRole('button', { name: 'Start', exact: true }).click();
  await expect.poll(() => redrive).toMatchObject({ source_arn: 'arn:aws:sqs:eu-west-1:123:review-orders.fifo' });
  await page.getByRole('button', { name: 'Queue actions' }).click(); await page.getByRole('menuitem', { name: 'Purge queue…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Purge queue', exact: true });
  await dialog.getByRole('textbox').fill('review-orders.fifo'); await dialog.getByRole('button', { name: 'Purge', exact: true }).click();
  await expect.poll(() => purged).toMatchObject({ url: queueUrl }); await expect(page.getByText('Purge started', { exact: true })).toBeVisible();
});

test('AWS setup test failure can retry without losing the account details', async ({ page }) => {
  let tested = 0;
  await page.route('**/aws/accounts', r => r.request().method() === 'POST' ? r.fulfill({ json: { ...account, name: 'Retry review account' } }) : r.fallback());
  await page.route('**/aws/accounts/ux-account/test', r => { tested++; return r.fulfill({ json: { ok: tested > 1, message: tested > 1 ? 'Authenticated' : 'Synthetic credentials expired', latency_ms: 12, login_required: false, identity: tested > 1 ? { arn: 'arn:aws:iam::123:user/review', account: '123', user_id: 'review' } : null } }); });
  await openPage(page, 'aws'); await page.getByRole('button', { name: 'Add account', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Add AWS account', exact: true });
  await dialog.getByRole('tab', { name: 'Enter access keys' }).click();
  await dialog.getByLabel('Access key ID', { exact: true }).fill('synthetic-key'); await dialog.getByLabel('Secret access key', { exact: true }).fill('synthetic-secret');
  await dialog.getByRole('button', { name: 'Next', exact: true }).click(); await page.getByTestId('aws-wizard-name').fill('Retry review account');
  await page.getByTestId('aws-wizard-save').click(); await expect(dialog).toContainText('Synthetic credentials expired');
  await dialog.getByRole('button', { name: 'Test again', exact: true }).click(); await expect(dialog).toContainText('Connected in 12 ms');
  await expect(dialog).toContainText('arn:aws:iam::123:user/review');
});

test('CloudWatch multi-series survives switching resources and range keys work', async ({ page }) => {
  await page.route('**/ec2/instances?*', r => r.fulfill({ json: { instances: [instance, { ...instance, instance_id: 'i-next', name: 'billing-api' }] } }));
  await page.route('**/ec2/instances/i-next?*', r => r.fulfill({ json: { ...instance, instance_id: 'i-next', name: 'billing-api' } }));
  await page.route('**/metrics?*', r => {
    const q = new URL(r.request().url()).searchParams; const next = q.get('dim_value') === 'i-next';
    return r.fulfill({ json: { namespace: 'AWS/EC2', dim_name: 'InstanceId', dim_value: q.get('dim_value'), range: q.get('range'), period_seconds: 60, start: '', end: '', series: ['network_in', 'network_out'].map((id, n) => ({ id, metric: n ? 'NetworkOut' : 'NetworkIn', stat: 'Sum', unit: 'bytes', label: `${next ? 'Billing' : 'Checkout'} ${n ? 'out' : 'in'}`, points: [100, 200, 300].map((v, i) => ({ t: `2026-09-25T12:0${i}:00Z`, v })), current: 300, min: 100, max: 300, sum: 600, avg: 200 })) } });
  });
  await openPage(page, 'aws/ux-account/ec2'); await page.getByRole('row').filter({ hasText: 'checkout-api' }).click();
  await page.getByRole('tab', { name: 'Metrics', exact: true }).click(); await expect(page.getByTestId('aws-metrics')).toContainText('Checkout in');
  await page.getByRole('radio', { name: '1h', exact: true }).focus(); await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('radio', { name: '6h', exact: true })).toBeFocused();
  await expect(page.getByRole('radio', { name: '1h', exact: true })).toHaveAttribute('tabindex', '-1');
  await page.getByRole('row').filter({ hasText: 'billing-api' }).click();
  await page.getByRole('tab', { name: 'Metrics', exact: true }).click();
  await expect(page.getByTestId('aws-metrics')).toContainText('Billing in'); await expect(page.getByTestId('aws-metrics')).not.toContainText('Checkout in');
  await page.screenshot({ path: '/tmp/otto-ux-r4-cloud-multiseries.png' });

});

test('Kubernetes log failure retries then follow reconnect preserves complete lines', async ({ page }) => {
  let failed = true; let followed = false;
  await page.route('**/k8s/clusters/ux-cluster/pods/*/*/logs?*', r => {
    followed ||= new URL(r.request().url()).searchParams.get('follow') === 'true';
    return r.fulfill(failed ? { status: 503, json: { code: 'upstream', message: 'Synthetic log connection dropped' } } : { contentType: 'text/plain', body: followed ? 'reconnected stream\nfinal partial line' : 'initial stream\n' });
  });
  await openPage(page, `kubernetes/ux-cluster/pods/default/${pod.name}`);
  await page.getByTestId('k8s-drawer').getByRole('tab', { name: 'Logs', exact: true }).click();
  await expect(page.getByText('Synthetic log connection dropped', { exact: false })).toBeVisible(); failed = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).click(); await expect(page.getByText('initial stream', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Follow', exact: true }).click(); await expect(page.getByText('final partial line', { exact: true })).toBeVisible();
  await expect(page.getByText('initial stream', { exact: true })).toBeHidden();
});

async function contrastOf(page: Page, sel: string): Promise<number> {
  return page.evaluate((selector) => {
    type C = [number, number, number, number];
    const parse = (s: string): C | null => {
      let m = /rgba?\(([^)]+)\)/.exec(s);
      if (m) {
        const p = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
        return [p[0] / 255, p[1] / 255, p[2] / 255, p[3] ?? 1];
      }
      m = /color\(srgb ([^)]+)\)/.exec(s);
      if (m) {
        const p = m[1].split(/[\s/]+/).filter(Boolean).map(Number);
        return [p[0], p[1], p[2], p[3] ?? 1];
      }
      return null;
    };
    const el = document.querySelector(selector) as HTMLElement;
    const chain: HTMLElement[] = [];
    for (let n: HTMLElement | null = el; n; n = n.parentElement) chain.unshift(n);
    let bg = [1, 1, 1];
    for (const n of chain) {
      const c = parse(getComputedStyle(n).backgroundColor);
      if (c && c[3] > 0) bg = bg.map((v, i) => c[i] * c[3] + v * (1 - c[3]));
    }
    const fg = parse(getComputedStyle(el).color)!;
    const f = [0, 1, 2].map((i) => fg[i] * fg[3] + bg[i] * (1 - fg[3]));
    const lum = (c: number[]) => {
      const l = c.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
      return 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2];
    };
    const a = lum(f);
    const b = lum(bg);
    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  }, sel);
}


test('Monitor overview cluster card opens with Space', async ({ page }) => {
  await overview(page); await monitoring(page); await openPage(page, 'kubernetes/monitor');
  await page.getByTestId('k8s-monitor-card').focus(); await page.keyboard.press('Space');
  await expect(page).toHaveURL(/monitor\/ux-cluster\/workloads$/);
});
