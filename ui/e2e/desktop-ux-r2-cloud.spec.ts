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

test('AWS phone detail contains focus and restores its resource row', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openPage(page, 'aws/ux-account/ec2');
  const row = page.getByRole('row').filter({ hasText: 'checkout-api' });
  await row.focus();
  await page.keyboard.press('Enter');
  const drawer = page.getByTestId('aws-drawer');
  await expect(drawer.getByRole('button', { name: 'Close details' })).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  expect(await drawer.evaluate(el => el.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
  await expect(row).toBeFocused();
});

test('tablet details keep a useful reading width with a persisted oversized drawer', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 768 });
  await page.addInitScript(() => localStorage.setItem('otto_k8s_drawer_w', '1500'));
  await openPage(page, `kubernetes/ux-cluster/pods/default/${pod.name}`);
  const drawer = page.getByTestId('k8s-drawer');
  await expectFullyInViewport(page, drawer);
  const bounds = await drawer.boundingBox();
  expect(bounds!.width).toBeGreaterThanOrEqual(320);
  await drawer.getByRole('button', { name: 'Close details' }).click();
  expect((await page.getByTestId('k8s-row').boundingBox())!.width).toBeGreaterThan(280);
});

test('Kubernetes splitter supports RTL pointer and keyboard resize', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.addInitScript(() => localStorage.setItem('otto_direction', 'rtl'));
  await openPage(page, `kubernetes/ux-cluster/pods/default/${pod.name}`);
  const splitter = page.getByRole('slider', { name: 'Resize details' });
  const drawer = page.getByTestId('k8s-drawer');
  const before = (await drawer.boundingBox())!.width;
  const box = (await splitter.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + 80);
  await page.mouse.down(); await page.mouse.move(box.x + 45, box.y + 80); await page.mouse.up();
  expect((await drawer.boundingBox())!.width).toBeGreaterThan(before + 20);
  await splitter.focus();
  const dragged = (await drawer.boundingBox())!.width;
  await page.keyboard.press('ArrowRight');
  expect((await drawer.boundingBox())!.width).toBeGreaterThan(dragged);
  await page.keyboard.press('Home');
  await expect(splitter).toHaveValue('320');
});

test('Kubernetes RTL logs retain LTR punctuation and metrics retry recovers', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('otto_direction', 'rtl'));
  let failed = true;
  await page.route('**/k8s/clusters/ux-cluster/metrics?*', r => r.fulfill(failed ? { status: 503, json: { code: 'upstream', message: 'Metrics temporarily unavailable' } } : { json: { available: true, pods: [{ name: pod.name, namespace: 'default', cpu_millicores: 125, mem_bytes: 1048576, containers: [{ name: 'app', cpu_millicores: 125, mem_bytes: 1048576 }] }] } }));
  await openPage(page, `kubernetes/ux-cluster/pods/default/${pod.name}`);
  const drawer = page.getByTestId('k8s-drawer');
  await drawer.getByRole('tab', { name: 'Logs', exact: true }).click();
  await expect(drawer.getByText('checkout started', { exact: true })).toHaveCSS('direction', 'ltr');
  await drawer.getByRole('tab', { name: 'Metrics', exact: true }).click();
  await expect(drawer.getByText('Metrics temporarily unavailable', { exact: false })).toBeVisible();
  failed = false;
  await drawer.getByRole('button', { name: 'Retry' }).click();
  await expect(drawer.getByRole('meter', { name: 'app CPU' })).toBeVisible();
});

test('SQS and Athena tabs support composite keyboard navigation', async ({ page }) => {
  await queues(page);
  await openPage(page, 'aws/ux-account/sqs');
  await page.getByRole('cell', { name: 'review-orders.fifo FIFO', exact: true }).click();
  await page.getByRole('tab', { name: 'Messages', exact: true }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Send', exact: true })).toBeFocused();
  await page.keyboard.press('End');
  await expect(page.getByRole('tab', { name: 'Redrive', exact: true })).toBeFocused();
  await page.keyboard.press('Home');
  await expect(page.getByRole('tab', { name: 'Messages', exact: true })).toBeFocused();
  await openPage(page, 'aws/ux-account/athena');
  await page.getByRole('tab', { name: 'Results', exact: true }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'History', exact: true })).toBeFocused();
  await expect(page.getByText('No recent executions', { exact: false })).toBeVisible();
});

test('SQS delayed Peek cannot populate a different queue', async ({ page }) => {
  await queues(page);
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/sqs/queues/peek', async r => {
    await gate;
    await r.fulfill({ json: { messages: [{ message_id: 'old-queue-message', body: 'private payload from previous queue', receipt_handle: 'synthetic', attributes: {} }] } });
  });
  await openPage(page, 'aws/ux-account/sqs');
  await page.getByRole('cell', { name: 'review-orders.fifo FIFO', exact: true }).click();
  await page.getByRole('button', { name: 'Peek', exact: true }).click();
  await page.getByRole('cell', { name: 'review-other', exact: true }).click();
  const peekResponse = page.waitForResponse(r => r.url().endsWith('/sqs/queues/peek'));
  release();
  await peekResponse;
  await page.evaluate(() => new Promise(requestAnimationFrame));
  await expect(page.getByRole('button', { name: 'Peek', exact: true })).toBeEnabled();
  await expect(page.getByText('private payload from previous queue', { exact: true })).toBeHidden();
});

const monitorConfig = { enabled: false, interval_secs: 30, namespaces: ['default'], probes: [{ name: 'app-metrics', port: 9090, path: '/metrics', format: 'prometheus', mappings: [], include: ['http_*'], exclude: [], timeout_ms: 3000 }], exclusions: [], transport: 'auto', concurrency: 4, retention_days: 7, series_cap: 1500, metrics_server: true };
for (const variant of [
  { name: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900 },
  { name: 'native-dark', theme: 'native', scheme: 'dark', width: 1440, height: 900 },
  { name: 'warm-light-phone', theme: 'warm', scheme: 'light', width: 375, height: 812 },
  { name: 'warm-dark-tablet-rtl', theme: 'warm', scheme: 'dark', width: 1024, height: 768 },
  { name: 'pro-dark', theme: 'pro-dark', scheme: 'dark', width: 1440, height: 900 },
]) {
  test(`loaded SQS and monitor forms remain usable ${variant.name}`, async ({ page }) => {
    await page.setViewportSize(variant);
    await page.addInitScript(v => {
      localStorage.setItem('otto_theme', v.theme); localStorage.setItem('otto_scheme', v.scheme);
      localStorage.setItem('otto_direction', v.name.includes('rtl') ? 'rtl' : 'ltr');
    }, variant);
    await queues(page);
    await openPage(page, 'aws/ux-account/sqs');
    await page.getByRole('cell', { name: 'review-orders.fifo FIFO', exact: true }).click();
    await page.getByRole('tab', { name: 'Send', exact: true }).click();
    await page.getByLabel('Body', { exact: true }).fill(JSON.stringify({ event: 'review-order', details: 'Long synthetic attribute '.repeat(60) }));
    await expect(page.getByLabel('Body', { exact: true })).toHaveCSS('direction', 'ltr');
    await page.getByLabel('Message group ID').fill('review-group');
    for (let i = 0; i < 6; i++) await page.getByRole('button', { name: 'Attribute', exact: true }).click();
    await page.getByRole('button', { name: 'Send message', exact: true }).scrollIntoViewIfNeeded();
    await expectFullyInViewport(page, page.getByRole('button', { name: 'Send message', exact: true }));
    // Every input must fit its local form, even when document overflow is hidden.
    const clipped = await page.locator('.form input').evaluateAll(els => els.filter(el => {
      const b = el.getBoundingClientRect(); return b.left < 0 || b.right > innerWidth;
    }).length);
    expect(clipped).toBe(0);
    await expectNoHorizontalOverflow(page);
    await expectFullyInViewport(page, page.getByTestId('aws-add-account'));
    await expectFullyInViewport(page, page.getByRole('button', { name: 'Remove attribute', exact: true }).last());
    await page.screenshot({ path: `/tmp/otto-ux-r2-cloud-${variant.name}-sqs.png` });
    await page.route('**/k8s/clusters/ux-cluster/monitor', r => r.fulfill({ json: { config: monitorConfig, status: null, presets: [] } }));
    await openPage(page, 'kubernetes/monitor/ux-cluster/settings');
    await expect(page.getByTestId('k8s-monitor-settings')).toBeVisible();
    await page.getByTestId('k8s-monitor-interval').fill('1');
    await page.getByTestId('k8s-monitor-save').click();
    await expect(page.getByTestId('k8s-monitor-settings').getByText('Interval must be 15..3600 seconds.', { exact: true })).toBeVisible();
    await page.getByTestId('k8s-monitor-interval').fill('30');
    await page.getByRole('button', { name: 'Probe', exact: true }).click();
    await expect(page.getByTestId('k8s-monitor-probe')).toHaveCount(2);
    await expect(page.getByTestId('k8s-monitor-probe').first().getByLabel('Path', { exact: true })).toHaveCSS('direction', 'ltr');
    await expectNoHorizontalOverflow(page);
    const internalOverflow = await page.locator('.settings, .mon, .settings .card, .settings .probe').evaluateAll(els => els.filter(el => el.scrollWidth > el.clientWidth + 2).map(el => ({ class: el.className, overflow: el.scrollWidth - el.clientWidth })));
    expect(internalOverflow).toEqual([]);
    await page.screenshot({ path: `/tmp/otto-ux-r2-cloud-${variant.name}-monitor.png` });
  });
}

test('S3 folder response cannot replace newer navigation and text preview retries', async ({ page }) => {
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const object = (key: string) => ({ key, size: 40, last_modified: '2026-09-01', storage_class: 'STANDARD' });
  await page.route('**/s3/buckets/review-artifacts/objects*', async r => {
    const prefix = new URL(r.request().url()).searchParams.get('prefix');
    if (prefix === 'old/') await gate;
    await r.fulfill({ json: { prefixes: prefix ? [] : ['old/', 'new/'], objects: prefix ? [object(prefix + 'readme.txt')] : [], is_truncated: false } });
  });
  let failed = true;
  await page.route('**/s3/buckets/review-artifacts/preview?*', r => r.fulfill(failed ? { status: 503, json: { code: 'upstream', message: 'Preview temporarily unavailable' } } : { json: { text: 'Synthetic preview recovered', content_type: 'text/plain', binary: false, truncated: false } }));
  await openPage(page, 'aws/ux-account/s3/review-artifacts');
  const oldRequest = page.waitForRequest(r => r.url().includes('prefix=old'));
  await page.getByRole('cell', { name: 'old/', exact: true }).click();
  await oldRequest;
  await page.evaluate(() => { location.hash = '#/aws/ux-account/s3/review-artifacts?prefix=new%2F'; });
  await expect(page.getByRole('cell', { name: 'readme.txt', exact: true })).toHaveAttribute('title', 'new/readme.txt');
  const oldResponse = page.waitForResponse(r => r.url().includes('prefix=old'));
  release();
  await oldResponse;
  await expect(page.getByRole('cell', { name: 'readme.txt', exact: true })).toHaveAttribute('title', 'new/readme.txt');
  await page.getByRole('cell', { name: 'readme.txt', exact: true }).click();
  await expect(page.getByText('Preview temporarily unavailable', { exact: false })).toBeVisible();
  failed = false;
  await page.getByRole('button', { name: 'Retry preview', exact: true }).click();
  await expect(page.getByText('Synthetic preview recovered')).toBeVisible();
});

test('SQS send validates delay and labels dynamic message attributes', async ({ page }) => {
  await queues(page);
  await openPage(page, 'aws/ux-account/sqs');
  await page.getByRole('cell', { name: 'review-other', exact: true }).click();
  await page.getByRole('tab', { name: 'Send', exact: true }).click();
  await page.getByLabel('Body', { exact: true }).fill('synthetic-message');
  await page.getByLabel('Delay (s)').fill('901');
  await expect(page.getByRole('button', { name: 'Send message', exact: true })).toBeDisabled();
  await page.getByLabel('Delay (s)').fill('0');
  await page.getByRole('button', { name: 'Attribute', exact: true }).click();
  await page.getByRole('textbox', { name: 'Attribute 1 name', exact: true }).fill('review');
  await page.getByRole('textbox', { name: 'Attribute 1 value', exact: true }).fill('synthetic');
  let sent: unknown;
  await page.route('**/sqs/queues/send', async r => { sent = r.request().postDataJSON(); await r.fulfill({ json: { message_id: 'sent-synthetic' } }); });
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByText('Message sent', { exact: true })).toBeVisible();
  expect(sent).toMatchObject({ body: 'synthetic-message', message_attributes: { review: { DataType: 'String', StringValue: 'synthetic' } } });
});

test('monitor settings reject empty numeric fields before sending', async ({ page }) => {
  let writes = 0;
  await page.route('**/k8s/clusters/ux-cluster/monitor', r => {
    if (r.request().method() !== 'GET') writes++;
    return r.fulfill({ json: { config: monitorConfig, status: null, presets: [] } });
  });
  await openPage(page, 'kubernetes/monitor/ux-cluster/settings');
  await page.getByTestId('k8s-monitor-interval').fill('');
  await page.getByTestId('k8s-monitor-save').click();
  await expect(page.getByTestId('k8s-monitor-settings').getByText('Interval must be 15..3600 seconds.', { exact: true })).toBeVisible();
  expect(writes).toBe(0);
});

test('Athena executes a synthetic query and opens its history result', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('otto_aws_athena_sql_ux-account', 'SELECT 42 AS answer'));
  await page.route('**/athena/workgroups', r => r.fulfill({ json: { workgroups: [{ name: 'primary', state: 'ENABLED' }] } }));
  await page.route('**/athena/databases*', r => r.fulfill({ json: { databases: ['review'] } }));
  await page.route('**/athena/query', r => r.fulfill({ json: { query_execution_id: 'synthetic-query' } }));
  await page.route('**/athena/query/synthetic-query*', r => r.fulfill({ json: { state: 'SUCCEEDED', stats: { data_scanned_bytes: 42, execution_ms: 12 }, result: { columns: [{ name: 'answer', data_type: 'integer' }], rows: [[42]], stats: { duration_ms: 12, row_count: 1 }, truncated: false } } }));
  await page.route('**/athena/history*', r => r.fulfill({ json: { executions: [{ id: 'synthetic-query', query: 'SELECT 42 AS answer', state: 'SUCCEEDED', submitted_at: '2026-09-25T12:00:00Z' }] } }));
  await openPage(page, 'aws/ux-account/athena');
  await page.getByTestId('athena-run').click();
  await expect(page.getByText('42', { exact: true }).first()).toBeVisible();
  await page.getByRole('tab', { name: 'History', exact: true }).click();
  await page.getByRole('cell', { name: 'SELECT 42 AS answer', exact: true }).click();
  await expect(page.getByRole('tab', { name: 'Results', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByText('42', { exact: true }).first()).toBeVisible();
  await page.screenshot({ path: '/tmp/otto-ux-r2-cloud-athena-result.png' });
});

test('Kubernetes workload scale validates input and nested sheets keep focus ownership', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  const deployment = { ...pod, name: 'review-deployment', kind: 'Deployment', extra: { desired: '2', selector: 'app=review' } };
  await page.route('**/k8s/clusters/ux-cluster/resources?*', r => r.fulfill({ json: { kind: 'deployments', items: [deployment], has_metrics: false } }));
  await openPage(page, 'kubernetes/ux-cluster/deployments/default/review-deployment');
  const drawer = page.getByTestId('k8s-drawer');
  await drawer.getByRole('button', { name: 'Scale…', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Scale Deployment', exact: true });
  await expectFullyInViewport(page, dialog);
  await dialog.getByRole('spinbutton').fill('-1');
  await expect(dialog.getByRole('button', { name: 'Scale', exact: true })).toBeDisabled();
  await dialog.getByRole('spinbutton').fill('3');
  await expect(dialog.getByRole('button', { name: 'Scale', exact: true })).toBeEnabled();
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(drawer).toBeVisible();
  await expect(drawer.getByRole('button', { name: 'Scale…', exact: true })).toBeFocused();
});

test('EKS import confirmation and RDS loaded detail use synthetic accounts', async ({ page }) => {
  const eks = { name: 'review-eks', status: 'ACTIVE', version: '1.30', endpoint: 'https://eks.example.test' };
  const db = { identifier: 'review-postgres', engine: 'postgres', engine_version: '16.2', class: 'db.t3.small', status: 'available', multi_az: true, publicly_accessible: false, tags: {}, endpoint: 'review.example.test', port: 5432 };
  await page.route('**/eks/clusters?*', r => r.fulfill({ json: { clusters: [eks] } }));
  await page.route('**/eks/clusters/review-eks?*', r => r.fulfill({ json: { cluster: eks, nodegroups: [{ name: 'review-workers', status: 'ACTIVE', desired: 2, min: 1, max: 4, instance_types: ['t3.small'] }] } }));
  await page.route('**/rds/instances?*', r => r.fulfill({ json: { instances: [db] } }));
  await page.route('**/rds/instances/review-postgres?*', r => r.fulfill({ json: { ...db, raw: db } }));
  await openPage(page, 'aws/ux-account/eks');
  await page.getByRole('cell', { name: 'review-eks', exact: true }).click();
  await expect(page.getByText('review-workers', { exact: true })).toBeVisible();
  const detail = page.getByRole('dialog');
  await detail.getByRole('button', { name: 'Open in Kubernetes', exact: false }).click();
  const confirm = page.getByRole('dialog', { name: 'Open in Kubernetes', exact: true });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(detail).toBeVisible();
  await page.keyboard.press('Escape');
  await openPage(page, 'aws/ux-account/rds');
  await page.getByRole('cell', { name: 'review-postgres', exact: true }).click();
  await expect(page.getByTestId('aws-drawer').getByText('review.example.test', { exact: false }).first()).toBeVisible();
  await page.getByTestId('aws-drawer').getByRole('tab', { name: 'Raw JSON', exact: true }).click();
  await expect(page.getByTestId('aws-drawer').getByRole('tabpanel')).toContainText('postgres');
});

test('SQS browse-only permissions retain a keyboard entry into enabled tabs', async ({ page }) => {
  await queues(page);
  await page.route('**/api/v1/access/**/capabilities*', r => r.fulfill({ json: { mode: 'enforced', operations: { sqs_view: { allowed: true } } } }));
  await openPage(page, 'aws/ux-account/sqs');
  await page.getByRole('cell', { name: 'review-other', exact: true }).click();
  await expect(page.getByRole('tab', { name: 'Messages', exact: true })).toBeDisabled();
  const attributes = page.getByRole('tab', { name: 'Attributes', exact: true });
  await expect(attributes).toHaveAttribute('tabindex', '0');
  await attributes.focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Metrics', exact: true })).toBeFocused();
  await page.keyboard.press('End');
  await expect(page.getByRole('tab', { name: 'Metrics', exact: true })).toBeFocused();
});
