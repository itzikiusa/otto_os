import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
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
