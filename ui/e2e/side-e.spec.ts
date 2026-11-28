import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedGitRepo, seedWorkspace } from './seed';
import { enableMcpFixtureCatalog } from './mcp-catalog-fixture';
import { expectNoHorizontalOverflow, openPage } from './helpers';

// The rc.6 side surfaces of API, AWS, Kubernetes and MCP on the three sizes,
// driven with real taps on a phone: list → item → ‹ Back → another item; ⋯ ›
// Hide / Show (⌘B) on desktop; the Detail pane open and close; the Navigator
// (☰) and the Side panel still one tap away. With OTTO_SHOTS_DIR set it also
// saves light and dark screenshots of each state.

test.use({ serviceWorkers: 'block' });

const SHOTS = process.env.OTTO_SHOTS_DIR;
const PHONE = { width: 390, height: 844 };
const TABLET = { width: 820, height: 1180 };
const DESKTOP = { width: 1440, height: 900 };

async function shot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${SHOTS}/${name}.png` });
}

async function scheme(page: Page, s: 'light' | 'dark'): Promise<void> {
  await page.addInitScript((v) => localStorage.setItem('otto_scheme', v), s);
}

// ── API ──────────────────────────────────────────────────────────────────────
let apiWs = '';
test.beforeAll(async () => {
  const s = await apiCtx();
  apiWs = await seedWorkspace(s.ctx, s.base);
  const base = `${s.base}/api/v1/workspaces/${apiWs}/api-client`;
  const post = async (url: string, data: unknown): Promise<{ id: string }> => {
    const r = await s.ctx.post(url, { data });
    if (!r.ok()) throw new Error(`POST ${url} → ${r.status()} ${await r.text()}`);
    return r.json();
  };
  const req = (collection_id: string, name: string, method: string, url: string) =>
    post(`${base}/requests`, { collection_id, name, method, url, headers: [], query: [], body_mode: 'none', body: '', auth: { type: 'none' } });
  const billing = await post(`${base}/collections`, { name: 'Billing' });
  await req(billing.id, 'Get invoice', 'GET', '{{base_url}}/v1/invoices/{{invoice_id}}');
  await req(billing.id, 'Create invoice', 'POST', '{{base_url}}/v1/invoices');
  const users = await post(`${base}/collections`, { name: 'Users' });
  await req(users.id, 'Get user', 'GET', '{{base_url}}/v1/users/me');
  await post(`${base}/environments`, { name: 'Staging', variables: { base_url: 'https://staging.example.com' } });
  await post(`${base}/environments`, { name: 'Production', variables: { base_url: 'https://api.example.com' } });
  await s.ctx.dispose().catch(() => {});
});

async function openApi(page: Page, s: 'light' | 'dark'): Promise<void> {
  await scheme(page, s);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_api_side', 'collections');
  }, apiWs);
  await openPage(page, 'api');
}

for (const s of ['light', 'dark'] as const) {
  test.describe(`API side surfaces (${s})`, () => {
    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('list pane with views: header, Hide / Show under ⋯ and ⌘B, rows, environments', async ({ page }) => {
        await openApi(page, s);
        const pane = page.getByRole('complementary', { name: 'Collections' });
        await expect(pane).toBeVisible({ timeout: 20_000 });
        await expect(pane.getByRole('tab', { name: 'Collections' })).toHaveAttribute('aria-selected', 'true');
        await pane.locator('.req-open', { hasText: 'Get invoice' }).click();
        await expect(page.getByLabel('Request URL')).toHaveValue('{{base_url}}/v1/invoices/{{invoice_id}}');
        await expect(pane.getByRole('treeitem', { name: /Get invoice/ })).toHaveAttribute('aria-selected', 'true');
        await shot(page, `api-desktop-${s}`);
        // Keyboard: the shared walk over the tree.
        await pane.getByRole('treeitem', { name: /Billing/ }).focus();
        await page.keyboard.press('ArrowDown');
        await expect(pane.getByRole('treeitem', { name: /Get invoice/ })).toBeFocused();
        await page.keyboard.press('ArrowLeft');
        await expect(pane.getByRole('treeitem', { name: /Billing/ })).toBeFocused();
        await pane.getByRole('button', { name: 'Collections actions' }).click();
        await page.getByRole('menuitem', { name: /Hide Collections/ }).click();
        await expect(pane).toBeHidden();
        await page.getByTestId('show-list-pane').click();
        await expect(pane).toBeVisible();
        await page.locator('body').click({ position: { x: 900, y: 600 } });
        await page.keyboard.press('ControlOrMeta+b');
        await expect(pane).toBeHidden();
        await page.keyboard.press('ControlOrMeta+b');
        await expect(pane).toBeVisible();
        // History view: the same pane, its own search, filter row and ⋯.
        await pane.getByRole('tab', { name: 'History' }).click();
        const hist = page.getByRole('complementary', { name: 'History' });
        await expect(hist.getByLabel('Search request history')).toBeVisible();
        await expect(hist.getByRole('button', { name: 'Show only agent runs' })).toBeVisible();
        await shot(page, `api-history-desktop-${s}`);
        await hist.getByRole('tab', { name: 'Collections' }).click();
        // Environments: a list pane of its own inside the content.
        await page.getByRole('button', { name: /^Environment:/ }).click();
        await page.getByRole('menuitem', { name: 'Manage environments…' }).click();
        const envs = page.getByRole('complementary', { name: 'Environments' });
        await expect(envs).toBeVisible();
        await expect(envs.getByTestId('api-env-row')).toHaveCount(2);
        await envs.getByTestId('api-env-row').filter({ hasText: 'Production' }).click();
        await expect(page.getByRole('region', { name: 'Variables in Production' })).toBeVisible();
        await shot(page, `api-environments-desktop-${s}`);
      });
    });
    test.describe('tablet', () => {
      test.use({ viewport: TABLET, isMobile: true, hasTouch: true });
      test('list pane beside the editor, one header row', async ({ page }) => {
        await openApi(page, s);
        await expect(page.getByRole('complementary', { name: 'Collections' })).toBeVisible({ timeout: 20_000 });
        await page.locator('.req-open', { hasText: 'Get user' }).tap();
        await expect(page.getByLabel('Request URL')).toHaveValue('{{base_url}}/v1/users/me');
        await expectNoHorizontalOverflow(page);
        await shot(page, `api-tablet-${s}`);
      });
    });
    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('push: list → request → Back → another; ☰ and the Side panel', async ({ page }) => {
        await openApi(page, s);
        const rows = page.locator('.req-open');
        await expect(rows.first()).toBeVisible({ timeout: 20_000 });
        await expect(page.getByRole('tablist', { name: 'Show' })).toBeVisible();
        await expect(page.getByLabel('Request URL')).toBeHidden();
        await expectNoHorizontalOverflow(page);
        await shot(page, `api-phone-list-${s}`);
        await rows.filter({ hasText: 'Create invoice' }).tap();
        await expect(page.getByLabel('Request URL')).toHaveValue('{{base_url}}/v1/invoices');
        const back = page.getByRole('button', { name: 'Back to Collections', exact: true });
        await expect(back).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `api-phone-content-${s}`);
        await back.tap();
        await expect(page.getByRole('treeitem', { name: /Create invoice/ })).toHaveAttribute('aria-selected', 'true');
        await rows.filter({ hasText: 'Get user' }).tap();
        await expect(page.getByLabel('Request URL')).toHaveValue('{{base_url}}/v1/users/me');
        await back.tap();
        // The page's own actions stay one tap away under ⋯ on the list page.
        await page.getByRole('button', { name: 'More actions' }).tap();
        await expect(page.getByRole('menuitem', { name: /Manage environments/ })).toBeVisible();
        await page.keyboard.press('Escape');
        await page.getByTestId('open-navigator').tap();
        await expect(page.getByRole('dialog', { name: 'Navigator' })).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(page.getByTestId('open-side-panel')).toBeVisible();
      });
    });
  });
}

// ── AWS ──────────────────────────────────────────────────────────────────────
const ACCOUNTS = [
  { id: 'acc-stg', name: 'Staging', region: 'eu-west-1', auth_mode: 'profile', profile: 'stg', environment: 'staging', color: '#3b82f6', created_by: 'root', created_at: '', updated_at: '' },
  { id: 'acc-prod', name: 'Production', region: 'eu-west-1', auth_mode: 'profile', profile: 'prod', environment: 'prod', color: '#ef4444', created_by: 'root', created_at: '', updated_at: '' },
];
const INSTANCES = [
  { instance_id: 'i-0a1', name: 'checkout-api', state: 'running', type: 't3.small', az: 'eu-west-1a', tags: { team: 'payments' }, private_ip: '10.0.0.8' },
  { instance_id: 'i-0b2', name: 'billing-worker', state: 'stopped', type: 't3.medium', az: 'eu-west-1b', tags: {}, private_ip: '10.0.0.9' },
];
const QUEUES = ['orders', 'refunds', 'emails'].map((n) => ({ url: `https://sqs.eu-west-1.amazonaws.com/1/${n}`, name: n, fifo: n === 'orders' }));

async function legacyAccess(page: Page): Promise<void> {
  await page.route('**/api/v1/access/**/capabilities*', (r) => r.fulfill({ json: { mode: 'legacy', operations: {} } }));
  await page.route('**/api/v1/access/*/capabilities/batch', async (r) => {
    const ids = ((r.request().postDataJSON() ?? {}) as { ids?: string[] }).ids ?? [];
    return r.fulfill({ json: ids.map((id) => ({ resource_id: id, mode: 'legacy', operations: {} })) });
  });
}

async function awsFixture(page: Page): Promise<void> {
  await legacyAccess(page);
  await page.route('**/api/v1/aws/**', async (r) => {
    const url = new URL(r.request().url());
    const path = url.pathname;
    if (r.request().method() !== 'GET') return r.fulfill({ status: 403, json: { code: 'forbidden', message: 'fixture' } });
    let json: unknown = {};
    if (path.endsWith('/status')) json = { installed: true, version: '2.27.19', install: { state: 'idle' } };
    else if (path.endsWith('/accounts')) json = ACCOUNTS;
    else if (path.endsWith('/regions')) json = { regions: [{ code: 'eu-west-1', name: 'Ireland' }] };
    else if (path.endsWith('/permissions')) json = { services: { s3: 'allowed', sqs: 'allowed', ec2: 'allowed', athena: 'allowed', eks: 'allowed', rds: 'denied' } };
    else if (path.endsWith('/ec2/instances')) json = { instances: INSTANCES };
    else if (/\/ec2\/instances\/i-/.test(path)) json = { ...INSTANCES[0], raw: { InstanceId: 'i-0a1' } };
    else if (path.endsWith('/metrics')) json = { namespace: 'AWS/EC2', dim_name: 'InstanceId', dim_value: 'i-0a1', range: '1h', period_seconds: 60, start: '', end: '', series: [] };
    else if (path.endsWith('/s3/buckets')) json = { buckets: [{ name: 'review-artifacts', creation_date: '2026-09-01' }] };
    else if (path.endsWith('/objects')) json = { prefix: '', prefixes: [], objects: [{ key: 'report.csv', size: 128, last_modified: '2026-09-25T12:00:00Z', storage_class: 'STANDARD' }, { key: 'notes.txt', size: 64, last_modified: '2026-09-25T12:00:00Z', storage_class: 'STANDARD' }], is_truncated: false, next_token: null };
    else if (path.endsWith('/preview')) json = { binary: false, text: 'name,count\ncheckout,12\nbilling,8', content_type: 'text/csv', truncated: false };
    else if (path.endsWith('/sqs/queues')) json = { queues: QUEUES };
    else if (path.endsWith('/sqs/queues/attributes')) json = { attributes: { QueueArn: 'arn:aws:sqs:eu-west-1:1:q' }, approx_messages: 12, approx_not_visible: 2, approx_delayed: 0 };
    else if (path.endsWith('/athena/workgroups')) json = { workgroups: [{ name: 'primary', state: 'ENABLED', output_location: 's3://out/' }] };
    else if (path.endsWith('/athena/databases')) json = { databases: ['analytics', 'raw_events'] };
    else if (path.endsWith('/athena/tables')) json = { tables: [{ name: 'orders', columns: [{ name: 'id', type: 'string' }, { name: 'total', type: 'double' }] }, { name: 'users', columns: [{ name: 'id', type: 'string' }] }] };
    else if (path.endsWith('/athena/history')) json = { executions: [] };
    else if (path.endsWith('/logs/groups')) json = { groups: ['/aws/lambda/checkout', '/aws/lambda/billing', '/ecs/orders'].map((name, i) => ({ name, stored_bytes: 1024 * (i + 1) * 900, retention_days: 14 })), next_token: null };
    else if (path.endsWith('/logs/streams')) json = { streams: [] };
    else if (path.endsWith('/logs/events')) json = { events: [] };
    else if (path.endsWith('/eks/clusters')) json = { clusters: [] };
    else if (path.endsWith('/rds/instances')) json = { instances: [] };
    return r.fulfill({ json });
  });
}

for (const s of ['light', 'dark'] as const) {
  test.describe(`AWS side surfaces (${s})`, () => {
    test.beforeEach(async ({ page }) => {
      await scheme(page, s);
      await awsFixture(page);
    });
    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('accounts pane, EC2 detail pane, Athena catalog, log groups, queues, S3 preview', async ({ page }) => {
        // The overview probes each account's IAM permissions (the rail's "denied").
        await openPage(page, 'aws');
        const pane = page.getByTestId('aws-accounts');
        await expect(pane).toBeVisible({ timeout: 20_000 });
        await page.getByTestId('aws-svc-acc-stg-ec2').click();
        await expect(pane.getByRole('heading', { name: 'Accounts' })).toBeVisible();
        await expect(page.getByTestId('aws-svc-acc-stg-ec2').getByRole('treeitem')).toHaveAttribute('aria-selected', 'true');
        await expect(page.getByTestId('aws-svc-acc-stg-rds')).toContainText('denied');
        await page.getByRole('cell', { name: 'checkout-api', exact: true }).click();
        const detail = page.getByTestId('aws-drawer');
        await expect(detail).toBeVisible();
        await expect(detail.getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-selected', 'true');
        await shot(page, `aws-desktop-detail-${s}`);
        await page.getByRole('button', { name: 'Close instance details' }).click();
        await expect(detail).toBeHidden();
        await shot(page, `aws-desktop-${s}`);
        await pane.getByRole('button', { name: 'Accounts actions' }).click();
        await page.getByRole('menuitem', { name: /Hide Accounts/ }).click();
        await expect(pane).toBeHidden();
        await page.getByTestId('show-list-pane').click();
        await expect(pane).toBeVisible();
        await page.getByTestId('aws-svc-acc-stg-athena').click();
        const catalog = page.getByTestId('athena-catalog');
        await expect(catalog.getByRole('treeitem', { name: /analytics/ })).toBeVisible();
        await catalog.getByRole('treeitem', { name: /analytics/ }).click();
        await expect(catalog.getByRole('treeitem', { name: /orders/ })).toBeVisible();
        await shot(page, `aws-athena-desktop-${s}`);
        await page.getByTestId('aws-svc-acc-stg-logs').click();
        const groups = page.getByTestId('aws-log-groups');
        await expect(groups.getByRole('button', { name: /\/aws\/lambda\/checkout/ })).toBeVisible();
        await shot(page, `aws-logs-desktop-${s}`);
        await page.getByTestId('aws-svc-acc-stg-sqs').click();
        const queues = page.getByTestId('aws-sqs-queues');
        await expect(queues.getByRole('button', { name: /orders/ })).toHaveAttribute('aria-current', 'true');
        await queues.getByRole('button', { name: /refunds/ }).click();
        await expect(page.getByText('refunds', { exact: true }).last()).toBeVisible();
        await shot(page, `aws-sqs-desktop-${s}`);
        await openPage(page, 'aws/acc-stg/s3/review-artifacts');
        await page.getByRole('row').filter({ hasText: 'report.csv' }).dblclick();
        const preview = page.getByRole('complementary', { name: 'Object preview' });
        await expect(preview.getByRole('cell', { name: 'checkout', exact: true })).toBeVisible();
        await shot(page, `aws-s3-preview-desktop-${s}`);
        await page.getByRole('button', { name: 'Close Object preview' }).click();
        await expect(preview).toBeHidden();
      });
    });
    test.describe('tablet', () => {
      test.use({ viewport: TABLET, isMobile: true, hasTouch: true });
      test('accounts pane beside the service; the detail pane is a sheet when there is no room', async ({ page }) => {
        await openPage(page, 'aws/acc-stg/ec2');
        await expect(page.getByTestId('aws-accounts')).toBeVisible({ timeout: 20_000 });
        await page.getByRole('cell', { name: 'checkout-api', exact: true }).tap();
        await expect(page.getByTestId('aws-drawer')).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `aws-tablet-detail-${s}`);
        await page.getByRole('button', { name: 'Close instance details' }).tap();
        await shot(page, `aws-tablet-${s}`);
      });
    });
    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('push: accounts → service → Back → another; nested lists push too; ☰ and the Side panel', async ({ page }) => {
        await openPage(page, 'aws');
        const ec2 = page.getByTestId('aws-svc-acc-stg-ec2');
        await expect(ec2).toBeVisible({ timeout: 20_000 });
        await expectNoHorizontalOverflow(page);
        await shot(page, `aws-phone-list-${s}`);
        await ec2.tap();
        await expect(page.getByRole('cell', { name: 'checkout-api', exact: true })).toBeVisible();
        const back = page.getByRole('button', { name: 'Back to Accounts', exact: true });
        await expect(back).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `aws-phone-content-${s}`);
        await page.getByRole('cell', { name: 'checkout-api', exact: true }).tap();
        await expect(page.getByTestId('aws-drawer')).toBeVisible();
        await shot(page, `aws-phone-detail-${s}`);
        await page.getByRole('button', { name: 'Close instance details' }).tap();
        await back.tap();
        await expect(ec2.getByRole('treeitem')).toHaveAttribute('aria-selected', 'true');
        // SQS: the queues are a list page of their own inside the service.
        await page.getByTestId('aws-svc-acc-stg-sqs').tap();
        const queues = page.getByTestId('aws-sqs-queues');
        await expect(queues.getByRole('button', { name: /refunds/ })).toBeVisible();
        await shot(page, `aws-phone-sqs-list-${s}`);
        await queues.getByRole('button', { name: /refunds/ }).tap();
        const qBack = page.getByRole('button', { name: 'Back to Queues', exact: true });
        await expect(qBack).toBeVisible();
        await qBack.tap();
        await expect(queues.getByRole('button', { name: /refunds/ })).toHaveAttribute('aria-current', 'true');
        await page.getByRole('button', { name: 'Back to Accounts', exact: true }).tap();
        // Athena: the editor first, "‹ Catalog" shows the catalog.
        await page.getByTestId('aws-svc-acc-stg-athena').tap();
        await page.getByRole('button', { name: 'Back to Catalog', exact: true }).tap();
        await expect(page.getByTestId('athena-catalog').getByRole('treeitem', { name: /analytics/ })).toBeVisible();
        await shot(page, `aws-phone-athena-catalog-${s}`);
        await page.getByRole('button', { name: 'Back to Accounts', exact: true }).tap();
        await page.getByTestId('open-navigator').tap();
        await expect(page.getByRole('dialog', { name: 'Navigator' })).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(page.getByTestId('open-side-panel')).toBeVisible();
      });
    });
  });
}

// ── Kubernetes ───────────────────────────────────────────────────────────────
const CAPS = { server_version: 'v1.30.3', metrics_server: true, argo_rollouts: true, argocd: false, checked_at: '' };
const CLUSTER = { id: 'k-stg', name: 'Staging cluster', source: 'kubeconfig', context_name: 'stg', default_namespace: 'default', environment: 'staging', capabilities: CAPS, created_by: 'root', created_at: '', updated_at: '' };
const PODS = ['checkout-api-7f8db5b4-abcde', 'billing-worker-5c9d8-xyz12', 'orders-api-6b7c9-qwe34'].map((name, i) => ({ name, namespace: 'default', kind: 'Pod', status: i === 1 ? 'CrashLoopBackOff' : 'Running', health: i === 1 ? 'bad' : 'ok', ready: i === 1 ? '0/1' : '1/1', restarts: i, age_seconds: 600 + i * 60, labels: { app: name.split('-')[0] }, extra: {}, images: ['registry.example.test/app:v1'] }));

async function k8sFixture(page: Page): Promise<void> {
  await legacyAccess(page);
  await page.addInitScript(() => localStorage.setItem('otto_k8s_autorefresh', '0'));
  await page.route('**/api/v1/k8s/**', async (r) => {
    const path = new URL(r.request().url()).pathname;
    if (r.request().method() !== 'GET') return r.fulfill({ status: 403, json: { code: 'forbidden', message: 'fixture' } });
    let json: unknown = {};
    if (path.endsWith('/status')) json = { kubectl: { installed: true, version: '1.30.3' }, k9s: { installed: false, version: null } };
    else if (path.endsWith('/clusters')) json = [CLUSTER];
    else if (path.endsWith('/capabilities')) json = CAPS;
    else if (path.endsWith('/namespaces')) json = { namespaces: [{ name: 'default', status: 'Active' }, { name: 'payments', status: 'Active' }] };
    else if (path.endsWith('/resources')) json = { kind: 'pods', items: PODS, has_metrics: false };
    else if (path.endsWith('/resource')) json = { manifest: { metadata: { name: PODS[0].name }, spec: { containers: [{ name: 'app' }] } }, describe: 'Name: ' + PODS[0].name, events: [] };
    else if (path.endsWith('/containers')) json = { containers: [{ name: 'app', image: 'app:v1', ready: true, state: 'running', restarts: 0, init: false }] };
    else if (path.endsWith('/logs')) return r.fulfill({ contentType: 'text/plain', body: 'checkout started\n' });
    return r.fulfill({ json });
  });
}

for (const s of ['light', 'dark'] as const) {
  test.describe(`Kubernetes side surfaces (${s})`, () => {
    test.beforeEach(async ({ page }) => {
      await scheme(page, s);
      await k8sFixture(page);
    });
    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('kinds pane, resource Detail pane, Hide / Show and ⌘B', async ({ page }) => {
        await openPage(page, 'kubernetes/k-stg/pods');
        const pane = page.getByTestId('k8s-kinds');
        await expect(pane).toBeVisible({ timeout: 20_000 });
        await expect(pane.getByRole('heading', { name: 'Resources', exact: true })).toBeVisible();
        await expect(pane.getByRole('button', { name: /^Pods/ })).toHaveAttribute('aria-current', 'true');
        await expect(page.getByTestId('k8s-row').first()).toBeVisible();
        await shot(page, `k8s-desktop-${s}`);
        await page.getByTestId('k8s-row').first().click();
        const detail = page.getByTestId('k8s-drawer');
        await expect(detail).toBeVisible();
        await expect(detail.getByRole('tab', { name: 'Overview', exact: true })).toHaveAttribute('aria-selected', 'true');
        await shot(page, `k8s-desktop-detail-${s}`);
        await detail.getByRole('button', { name: 'Pod details actions' }).click();
        await expect(page.getByRole('menuitem', { name: 'kubectl describe' })).toBeVisible();
        await page.keyboard.press('Escape');
        await page.getByRole('button', { name: 'Close Pod details' }).click();
        await expect(detail).toBeHidden();
        await pane.getByRole('button', { name: 'Resources actions' }).click();
        await page.getByRole('menuitem', { name: /Hide Resources/ }).click();
        await expect(pane).toBeHidden();
        await page.getByTestId('show-list-pane').click();
        await expect(pane).toBeVisible();
        await pane.getByRole('button', { name: /^Pods/ }).focus();
        await page.keyboard.press('ArrowDown');
        await page.keyboard.press('Enter');
        await expect(page).toHaveURL(/\/deployments$/);
      });
    });
    test.describe('tablet', () => {
      test.use({ viewport: TABLET, isMobile: true, hasTouch: true });
      test('kinds pane beside the table; details as a sheet when there is no room', async ({ page }) => {
        await openPage(page, 'kubernetes/k-stg/pods');
        await expect(page.getByTestId('k8s-kinds')).toBeVisible({ timeout: 20_000 });
        await expectNoHorizontalOverflow(page);
        await shot(page, `k8s-tablet-${s}`);
        await page.getByTestId('k8s-row').first().tap();
        await expect(page.getByTestId('k8s-drawer')).toBeVisible();
        await shot(page, `k8s-tablet-detail-${s}`);
      });
    });
    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('push: kinds → table → Back → another kind; ☰ and the Side panel', async ({ page }) => {
        await openPage(page, 'kubernetes/k-stg');
        const pane = page.getByTestId('k8s-kinds');
        await expect(pane.getByRole('button', { name: /^Deployments/ })).toBeVisible({ timeout: 20_000 });
        await expect(page.getByRole('combobox', { name: 'Resource kind' })).toHaveCount(0);
        await expectNoHorizontalOverflow(page);
        await shot(page, `k8s-phone-list-${s}`);
        await pane.getByRole('button', { name: /^Pods/ }).tap();
        await expect(page.getByTestId('k8s-row').first()).toBeVisible();
        const back = page.getByRole('button', { name: 'Back to Resources', exact: true });
        await expect(back).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `k8s-phone-content-${s}`);
        await page.getByTestId('k8s-row').first().tap();
        await expect(page.getByTestId('k8s-drawer')).toBeVisible();
        await shot(page, `k8s-phone-detail-${s}`);
        await page.getByRole('button', { name: 'Close Pod details' }).tap();
        await back.tap();
        await expect(pane.getByRole('button', { name: /^Pods/ })).toHaveAttribute('aria-current', 'true');
        await pane.getByRole('button', { name: /^Deployments/ }).tap();
        await expect(page).toHaveURL(/\/deployments$/);
        await back.tap();
        await page.getByTestId('open-navigator').tap();
        await expect(page.getByRole('dialog', { name: 'Navigator' })).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(page.getByTestId('open-side-panel')).toBeVisible();
      });
    });
  });
}

// ── MCP ──────────────────────────────────────────────────────────────────────
async function mcpWorkspace(withApproval: boolean): Promise<{ ws: string; title: string }> {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  let title = '';
  if (withApproval) {
    await enableMcpFixtureCatalog(ctx, base);
    const { repoId } = await seedGitRepo(ctx, base, ws);
    title = `Side approval ${Date.now()}`;
    const r = await ctx.post(`${base}/api/v1/mcp/otto-tools/invoke`, {
      data: { tool: 'otto.create_pr', arguments: { repo_id: repoId, title, description: 'Body', source_branch: `e2e/side-${Date.now()}`, target_branch: 'main' } },
    });
    expect(((await r.json()) as { decision: string }).decision).toBe('pending_approval');
  }
  await ctx.dispose();
  return { ws, title };
}

for (const s of ['light', 'dark'] as const) {
  test.describe(`MCP side surfaces (${s})`, () => {
    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('profiles list pane, Rules and the approval as Detail panes', async ({ page }) => {
        test.setTimeout(90_000);
        const { ws, title } = await mcpWorkspace(true);
        await scheme(page, s);
        await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), ws);
        await page.goto('/#/mcp');
        const row = page.getByTestId('mcp-home-approval').filter({ hasText: title });
        await expect(row).toBeVisible({ timeout: 30_000 });
        await row.locator('.tr-main').click();
        const approval = page.getByTestId('mcp-home-approval-detail');
        await expect(approval).toBeVisible();
        await expect(approval.getByText('Who asks')).toBeVisible();
        await shot(page, `mcp-approval-desktop-${s}`);
        await page.getByRole('button', { name: /^Close otto\.create_pr/ }).click();
        await expect(approval).toBeHidden();
        await page.goto('/#/mcp/profiles');
        const pane = page.getByTestId('mcp-profiles').getByRole('complementary', { name: 'Profiles' });
        await expect(pane).toBeVisible({ timeout: 20_000 });
        const rows = pane.locator('.side-row');
        await expect(rows.nth(1)).toBeVisible();
        await rows.nth(1).click();
        await expect(rows.nth(1).getByRole('button').first()).toHaveAttribute('aria-current', 'true');
        await expect(page.getByTestId('mcp-profile-config')).toBeVisible();
        await shot(page, `mcp-profiles-desktop-${s}`);
        await pane.getByRole('button', { name: 'Profiles actions' }).click();
        await page.getByRole('menuitem', { name: /Hide Profiles/ }).click();
        await expect(pane).toBeHidden();
        await page.getByTestId('show-list-pane').click();
        await expect(pane).toBeVisible();
        await page.goto('/#/mcp/servers');
        await page.getByTestId('mcp-rules-btn').click();
        const rules = page.getByTestId('mcp-rules-drawer');
        await expect(rules).toBeVisible();
        await shot(page, `mcp-rules-desktop-${s}`);
        await page.getByRole('button', { name: 'Close Rules', exact: true }).click();
        await expect(rules).toBeHidden();
      });
    });
    test.describe('tablet', () => {
      test.use({ viewport: TABLET, isMobile: true, hasTouch: true });
      test('profiles pane beside the profile; Rules', async ({ page }) => {
        const { ws } = await mcpWorkspace(false);
        await scheme(page, s);
        await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), ws);
        await page.goto('/#/mcp/profiles');
        await expect(page.getByTestId('mcp-profile-config')).toBeVisible({ timeout: 20_000 });
        await expectNoHorizontalOverflow(page);
        await shot(page, `mcp-profiles-tablet-${s}`);
        await page.goto('/#/mcp/servers');
        await page.getByTestId('mcp-rules-btn').tap();
        await expect(page.getByTestId('mcp-rules-drawer')).toBeVisible();
        await shot(page, `mcp-rules-tablet-${s}`);
      });
    });
    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('push: profiles → profile → Back → another; the approval sheet; ☰ and the Side panel', async ({ page }) => {
        test.setTimeout(90_000);
        const { ws, title } = await mcpWorkspace(true);
        await scheme(page, s);
        await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), ws);
        await page.goto('/#/mcp/profiles');
        const rows = page.getByRole('complementary', { name: 'Profiles' }).locator('.side-row');
        await expect(rows.nth(1)).toBeVisible({ timeout: 20_000 });
        await expect(page.getByTestId('mcp-profile-config')).toBeHidden();
        await expectNoHorizontalOverflow(page);
        await shot(page, `mcp-phone-list-${s}`);
        await rows.nth(1).getByRole('button').first().tap();
        await expect(page.getByTestId('mcp-profile-config')).toBeVisible();
        const back = page.getByRole('button', { name: 'Back to Profiles', exact: true });
        await expect(back).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `mcp-phone-content-${s}`);
        await back.tap();
        await expect(rows.nth(1).getByRole('button').first()).toHaveAttribute('aria-current', 'true');
        await rows.nth(0).getByRole('button').first().tap();
        await expect(page.getByTestId('mcp-profile-config')).toBeVisible();
        await back.tap();
        await page.goto('/#/mcp');
        const row = page.getByTestId('mcp-home-approval').filter({ hasText: title });
        await expect(row).toBeVisible({ timeout: 30_000 });
        await row.locator('.tr-main').tap();
        await expect(page.getByRole('dialog', { name: 'otto.create_pr' })).toBeVisible();
        await shot(page, `mcp-phone-approval-${s}`);
        await page.getByRole('button', { name: /^Close otto\.create_pr/ }).tap();
        await page.getByTestId('open-navigator').tap();
        await expect(page.getByRole('dialog', { name: 'Navigator' })).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(page.getByTestId('open-side-panel')).toBeVisible();
      });
    });
  });
}
