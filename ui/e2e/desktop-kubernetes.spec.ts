import { test, expect } from '@playwright/test';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Kubernetes console (desktop-browser only).
//
// The backend lands in parallel with the UI: every test skips when the daemon
// doesn't serve `GET /api/v1/k8s/status` yet (404). With the route present the
// page must render EITHER the kubectl first-run panel OR the clusters overview
// — never a blank page — with no console errors; the "Add cluster" wizard must
// sit fully inside the viewport; and a cluster row created over the API (a
// throwaway kubeconfig with a bogus server) must render as a card with its
// name + environment pill and open into the workspace shell (kinds rail +
// namespace combobox) even though every kubectl call against it fails — the
// table shows an error state, not nothing.
// ─────────────────────────────────────────────────────────────────────────────

interface K8sStatus {
  kubectl: { installed: boolean };
}

let status: K8sStatus | null = null;
let statusCode = 0;
let clusterId: string | null = null;
const CLUSTER_NAME = `e2e-k8s-${Date.now().toString(36)}`;

const KUBECONFIG = `apiVersion: v1
kind: Config
clusters:
- name: e2e-bogus
  cluster:
    server: https://127.0.0.1:1
contexts:
- name: e2e-bogus
  context:
    cluster: e2e-bogus
    user: e2e-bogus
    namespace: default
current-context: e2e-bogus
users:
- name: e2e-bogus
  user:
    token: not-a-real-token
`;

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  const r = await ctx.get(`${base}/api/v1/k8s/status`);
  statusCode = r.status();
  if (r.ok()) status = (await r.json()) as K8sStatus;
  if (r.ok()) {
    const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-k8s-'));
    const path = join(dir, 'kubeconfig.yaml');
    writeFileSync(path, KUBECONFIG);
    const c = await ctx.post(`${base}/api/v1/k8s/clusters`, {
      data: {
        name: CLUSTER_NAME,
        source: 'kubeconfig',
        kubeconfig_path: path,
        context_name: 'e2e-bogus',
        default_namespace: 'default',
        environment: 'staging',
      },
    });
    if (c.ok()) clusterId = ((await c.json()) as { id: string }).id;
    else console.warn(`k8s cluster seed failed: ${c.status()} ${await c.text()}`);
  }
  await ctx.dispose();
});

test.afterAll(async () => {
  if (!clusterId) return;
  const { ctx, base } = await apiCtx();
  await ctx.delete(`${base}/api/v1/k8s/clusters/${clusterId}`).catch(() => {});
  await ctx.dispose();
});

test.beforeEach(({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-only spec');
  test.skip(statusCode === 404, 'daemon has no /k8s/* routes yet (backend not landed)');
});

function collectErrors(page: import('@playwright/test').Page): string[] {
  const errors: string[] = [];
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(m.text());
  });
  page.on('pageerror', (e) => errors.push(String(e)));
  return errors;
}

test('#/kubernetes renders the install panel or the clusters overview', async ({ page }) => {
  const errors = collectErrors(page);
  await openPage(page, 'kubernetes');
  const pageRoot = page.getByTestId('k8s-page');
  await expect(pageRoot).toBeVisible();
  const install = page.getByTestId('k8s-install-panel');
  const overview = page.locator('.page-header h1', { hasText: 'Kubernetes' });
  await expect(install.or(overview).first()).toBeVisible({ timeout: 15_000 });
  if (status && !status.kubectl.installed) {
    await expect(install).toBeVisible();
    await expect(install).toContainText('kubectl');
  } else {
    await expect(overview).toBeVisible();
  }
  expect(errors, `console errors: ${errors.join('\n')}`).toEqual([]);
});

test('the Add cluster wizard is fully inside the viewport', async ({ page }) => {
  await openPage(page, 'kubernetes');
  if (status && !status.kubectl.installed) {
    // kubectl missing → the first-run panel gates the module; step past it.
    await page.getByRole('button', { name: 'Continue without installing' }).click();
  }
  await page.getByTestId('k8s-add-cluster').click();
  const sheet = page.locator('.sheet', { has: page.getByTestId('k8s-cluster-wizard') });
  await expect(sheet).toBeVisible();
  await page.waitForTimeout(150); // sheet-in animation
  await expectFullyInViewport(page, sheet, 'Add cluster sheet');
  // All three source paths are offered.
  await expect(sheet.getByRole('tab', { name: 'From kubeconfig' })).toBeVisible();
  await expect(sheet.getByRole('tab', { name: 'Paste kubeconfig' })).toBeVisible();
  await expect(sheet.getByRole('tab', { name: 'From EKS' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(sheet).toBeHidden();
});

test('a seeded cluster renders as a card and opens into a workspace that survives a dead cluster', async ({ page }) => {
  test.skip(clusterId === null, 'cluster seed failed (POST /k8s/clusters)');
  const errors = collectErrors(page);
  await openPage(page, 'kubernetes');
  if (status && !status.kubectl.installed) {
    await page.getByRole('button', { name: 'Continue without installing' }).click();
  }
  const card = page.getByTestId('k8s-cluster-card').filter({ hasText: CLUSTER_NAME });
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card.locator('.env-badge')).toHaveText('STG');

  await card.click();
  await expect(page).toHaveURL(new RegExp(`#/kubernetes/${clusterId}`));
  const wsp = page.getByTestId('k8s-workspace');
  await expect(wsp).toBeVisible();
  // Shell renders regardless of kubectl failing against the bogus server.
  await expect(page.getByTestId('k8s-kinds')).toBeVisible();
  await expect(page.getByTestId('k8s-kinds').getByRole('button', { name: /^Pods/ })).toBeVisible();
  await expect(page.getByTestId('k8s-ns-picker')).toBeVisible();
  await expect(page.getByTestId('k8s-cluster-switcher')).toHaveValue(clusterId!);
  // The resource load fails → a visible error state (not a blank/loading-forever table).
  await expect(page.getByTestId('k8s-table-error')).toBeVisible({ timeout: 45_000 });
  await expect(page.getByTestId('k8s-table-error')).toContainText(/Couldn't load pods/i);
  // Kinds rail still navigates.
  await page.getByTestId('k8s-kinds').getByRole('button', { name: /^Deployments/ }).click();
  await expect(page).toHaveURL(new RegExp(`#/kubernetes/${clusterId}/deployments`));
  // `?` opens the shortcut sheet; Esc closes it.
  await page.keyboard.press('?');
  const hints = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(hints).toBeVisible();
  await expectFullyInViewport(page, hints, 'shortcuts sheet');
  await page.keyboard.press('Escape');
  await expect(hints).toBeHidden();
  // Failing kubectl calls surface as UI error states, never as console errors.
  const real = errors.filter((e) => !/Failed to load resource/.test(e));
  expect(real, `console errors: ${real.join('\n')}`).toEqual([]);
});
