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
