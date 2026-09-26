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

test.setTimeout(90_000);
test.beforeEach(async ({ page }) => cloudFixture(page));

test('Kubernetes Enter activates focused kind button', async ({ page }) => {
  await openPage(page, 'kubernetes/ux-cluster/pods');
  await expect(page.getByTestId('k8s-row')).toBeVisible();
  const deployments = page.getByTestId('k8s-kinds').getByRole('button', { name: 'Deployments' });
  await deployments.focus();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/deployments$/);
});

test('Kubernetes namespace popup closes on Tab and keeps focus after selection', async ({ page }) => {
  await openPage(page, 'kubernetes/ux-cluster/pods');
  const ns = page.getByRole('combobox', { name: 'Namespace' });
  await ns.focus();
  const list = page.getByRole('listbox', { name: 'Namespaces' });
  await expectFullyInViewport(page, list, 'long namespace list');
  await page.keyboard.press('Tab');
  await expect(list).toBeHidden();
  await ns.focus();
  await ns.fill('namespace-089');
  await page.keyboard.press('Enter');
  await expect(ns).toHaveValue('namespace-089');
  await expect(ns).toBeFocused();
});

test('Kubernetes detail arrow keys move selection and focus together', async ({ page }) => {
  await openPage(page, `kubernetes/ux-cluster/pods/default/${pod.name}`);
  const tabs = page.getByTestId('k8s-drawer').getByRole('tab');
  await tabs.filter({ hasText: /^Overview$/ }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(tabs.filter({ hasText: /^Manifest$/ })).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(tabs.filter({ hasText: /^Describe$/ })).toBeFocused();
});

test('AWS detail arrow keys move selection and focus together', async ({ page }) => {
  await openPage(page, 'aws/ux-account/ec2');
  await page.getByRole('cell', { name: 'checkout-api', exact: true }).click();
  const tabs = page.getByTestId('aws-drawer').getByRole('tab');
  await tabs.filter({ hasText: /^Overview$/ }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(tabs.filter({ hasText: /^Metrics$/ })).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(tabs.filter({ hasText: /^Raw JSON$/ })).toBeFocused();
});

for (const variant of [
  { name: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900, rtl: false },
  { name: 'native-dark', theme: 'native', scheme: 'dark', width: 1440, height: 900, rtl: false },
  { name: 'warm-dark', theme: 'warm', scheme: 'dark', width: 1440, height: 900, rtl: false },
  { name: 'phone-light', theme: 'native', scheme: 'light', width: 375, height: 812, rtl: false },
  { name: 'tablet-rtl', theme: 'native', scheme: 'dark', width: 1024, height: 768, rtl: true },
]) {
  test(`loaded cloud screens fit ${variant.name}`, async ({ page }) => {
    await page.setViewportSize(variant);
    await page.addInitScript(v => {
      localStorage.setItem('otto_theme', v.theme);
      localStorage.setItem('otto_scheme', v.scheme);
      localStorage.setItem('otto_direction', v.rtl ? 'rtl' : 'ltr');
    }, variant);
    await openPage(page, 'aws/ux-account/ec2');
    await page.getByRole('cell', { name: 'checkout-api', exact: true }).click();
    await expect(page.getByTestId('aws-drawer')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await expectFullyInViewport(page, page.getByTestId('aws-drawer').getByRole('button', { name: 'Close details' }));
    await page.screenshot({ path: `/tmp/otto-ux-cloud-${variant.name}-aws-loaded.png` });
    await openPage(page, `kubernetes/ux-cluster/pods/default/${pod.name}`);
    const drawer = page.getByTestId('k8s-drawer');
    await drawer.getByRole('tab', { name: 'Logs', exact: true }).click();
    await expect(drawer.getByText('checkout started', { exact: true })).toBeVisible();
    await drawer.getByRole('textbox', { name: 'Search logs' }).fill('complete');
    await expect(drawer.getByText('checkout started', { exact: true })).toBeHidden();
    await expectNoHorizontalOverflow(page);
    await expectFullyInViewport(page, drawer.getByRole('button', { name: 'Close details' }));
    await page.screenshot({ path: `/tmp/otto-ux-cloud-${variant.name}-kubernetes-loaded.png` });
  });
}

test('AWS all service routes render and retry a failed list', async ({ page }) => {
  for (const service of ['s3', 'sqs', 'ec2', 'athena', 'eks', 'rds']) {
    await openPage(page, `aws/ux-account/${service}`);
    if (service === 'athena') await expect(page.getByRole('combobox', { name: 'Workgroup', exact: true })).toBeVisible();
    else await expect(page.getByRole('heading', { name: service.toUpperCase(), exact: true, level: 2 })).toBeVisible();
    await expectNoHorizontalOverflow(page);
  }
  let failed = true;
  await page.route('**/api/v1/aws/accounts/ux-account/s3/buckets', r => r.fulfill(failed ? { status: 503, json: { code: 'unavailable', message: 'Cloud temporarily unavailable' } } : { json: { buckets: [{ name: 'recovered-bucket' }] } }));
  await openPage(page, 'aws/ux-account/s3');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByText('Cloud temporarily unavailable', { exact: false })).toBeVisible();
  failed = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByText('recovered-bucket', { exact: true })).toBeVisible();
});

test('cloud setup wizards support keyboard source selection and validation', async ({ page }) => {
  await page.route('**/api/v1/k8s/discover', r => r.fulfill({ json: { contexts: [] } }));
  await openPage(page, 'aws');
  await page.getByRole('button', { name: 'Add account', exact: true }).click();
  const awsDialog = page.getByRole('dialog', { name: 'Add AWS account' });
  await expectFullyInViewport(page, awsDialog);
  await awsDialog.getByRole('tab', { name: 'Use an existing AWS profile' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(awsDialog.getByRole('tab', { name: 'Enter access keys' })).toBeFocused();
  await expect(awsDialog.getByRole('button', { name: 'Next', exact: true })).toBeDisabled();
  await awsDialog.getByLabel('Access key ID', { exact: true }).fill('test-key');
  await awsDialog.getByLabel('Secret access key', { exact: true }).fill('fixture-secret');
  await awsDialog.getByRole('button', { name: 'Next', exact: true }).click();
  await expect(page.getByTestId('aws-wizard-save')).toBeDisabled();
  await page.keyboard.press('Escape');
});

test('Kubernetes setup source selection works with arrow keys', async ({ page }) => {
  await page.route('**/api/v1/k8s/discover', r => r.fulfill({ json: { contexts: [] } }));
  await openPage(page, 'kubernetes');
  await page.getByTestId('k8s-add-cluster').click();
  const k8sDialog = page.getByRole('dialog', { name: 'Add cluster' });
  await k8sDialog.getByRole('tab', { name: 'From kubeconfig' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(k8sDialog.getByRole('tab', { name: 'Paste kubeconfig' })).toBeFocused();
  await expect(k8sDialog.getByRole('button', { name: 'Next', exact: true })).toBeDisabled();
  await expect(k8sDialog.getByLabel('kubeconfig YAML')).toBeVisible();
  await page.keyboard.press('End');
  await expect(k8sDialog.getByRole('tab', { name: 'From EKS' })).toBeFocused();
  await expect(k8sDialog.getByRole('button', { name: 'Go to AWS' })).toBeVisible();
});

test('phone Kubernetes details cover navigation and contain keyboard focus', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await openPage(page, `kubernetes/ux-cluster/pods/default/${pod.name}`);
  const drawer = page.getByTestId('k8s-drawer');
  await expect(drawer).toHaveAttribute('role', 'dialog');
  await expect(drawer).toHaveAttribute('aria-modal', 'true');
  const overDrawer = await drawer.evaluate(el => el.contains(document.elementFromPoint(180, 790)));
  expect(overDrawer, 'phone navigation must not cover the detail sheet').toBe(true);
  const close = drawer.getByRole('button', { name: 'Close details' });
  await close.focus();
  await page.keyboard.press('Shift+Tab');
  expect(await drawer.evaluate(el => el.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
});
