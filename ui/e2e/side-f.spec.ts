import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { expectNoHorizontalOverflow, openPage } from './helpers';

// The rc.6 side surfaces of Vault, Product, Canvas and Design Hall on the
// three sizes, driven with real taps on a phone: list → item → ‹ Back → another
// item; ⋯ › Hide / Show (⌘B) on desktop; the Detail pane opened and closed;
// the Navigator (☰) and the Side panel still one tap away. With
// OTTO_SHOTS_DIR set it also saves light and dark screenshots of each state.

test.use({ serviceWorkers: 'block' });
// One worker, one seed: the stories / scenes below are looked up by title.
test.describe.configure({ mode: 'serial' });

const SHOTS = process.env.OTTO_SHOTS_DIR;
const PHONE = { width: 390, height: 844 };
const TABLET = { width: 820, height: 1180 };
const DESKTOP = { width: 1440, height: 900 };

let ws = '';
let sceneId = '';
let designId = '';
let vaultId = 0;
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  ws = await seedWorkspace(ctx, base, 'Side F');
  ({ vaultId } = await seedVaultDir(ctx, base, ws));
  for (const title of ['Checkout journey', 'Refund policy', 'Payouts v2']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/product/drafts`, { data: { title } });
    expect(r.ok()).toBeTruthy();
  }
  const sc = await ctx.post(`${base}/api/v1/workspaces/${ws}/canvas/scenes`, {
    data: { title: 'Payments flow', doc: { type: 'otto-canvas', version: 1, format: 'mermaid', source: 'flowchart LR\n A[Cart] --> B[Pay] --> C[Done]' } },
  });
  expect(sc.ok()).toBeTruthy();
  sceneId = ((await sc.json()) as { id: string }).id;
  await ctx.post(`${base}/api/v1/workspaces/${ws}/canvas/scenes`, {
    data: { title: 'Onboarding', section: 'Growth', doc: { type: 'otto-canvas', version: 1, format: 'mermaid', source: 'flowchart TD\n A[Sign up] --> B[Verify]' } },
  });
  const d = await ctx.post(`${base}/api/v1/design/artifacts`, {
    data: { workspace_id: ws, format: 'html', studio: 'frames', title: 'Rewards screen', content: '<!doctype html><h1 style="margin:40px">Rewards</h1>' },
  });
  expect(d.ok()).toBeTruthy();
  designId = ((await d.json()) as { artifact: { id: string } }).artifact.id;
  await ctx.dispose();
});

async function shot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: `${SHOTS}/${name}.png` });
}
async function prep(page: Page, s: 'light' | 'dark'): Promise<void> {
  await page.addInitScript(({ v, w, id }) => {
    localStorage.setItem('otto_scheme', v);
    localStorage.setItem('otto_workspace', w);
    localStorage.setItem(`otto_vault_last:${w}`, String(id));
  }, { v: s, w: ws, id: vaultId });
}
/** A right sheet has finished sliding in (full width, left edge at 0). */
async function settled(sheet: ReturnType<Page['getByRole']>): Promise<void> {
  await expect.poll(async () => Math.round((await sheet.boundingBox())?.x ?? 99)).toBeLessThanOrEqual(1);
}
/** The phone chrome stays one tap away: ☰ on a list page, the Side panel button always. */
async function phoneChrome(page: Page, list: boolean): Promise<void> {
  if (list) await expect(page.getByTestId('open-navigator')).toBeVisible();
  await expect(page.getByTestId('open-side-panel')).toBeVisible();
}

for (const s of ['light', 'dark'] as const) {
  test.describe(`Vault (${s})`, () => {
    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('views header, Hide / Show, Note details', async ({ page }) => {
        await prep(page, s);
        await openPage(page, 'vault');
        const pane = page.getByTestId('vault-files');
        await expect(pane).toBeVisible({ timeout: 20_000 });
        await expect(page.getByRole('tablist', { name: 'Vault views' })).toBeVisible();
        await pane.getByText('services', { exact: true }).click();
        await pane.getByText('auth-api', { exact: true }).click();
        await expect(page.locator('.read')).toBeVisible();
        const toggle = page.getByTestId('vault-details-toggle');
        if ((await toggle.getAttribute('aria-pressed')) !== 'true') await toggle.click();
        await expect(page.getByTestId('vault-details')).toBeVisible();
        await shot(page, `vault-desktop-detail-${s}`);
        await page.getByTestId('vault-details-close').click();
        await shot(page, `vault-desktop-${s}`);
        await page.getByTestId('vault-files-more').click();
        await page.getByRole('menuitem', { name: /Hide Files/ }).click();
        await expect(pane).toBeHidden();
        await page.getByTestId('show-list-pane').click();
        await expect(pane).toBeVisible();
        await page.keyboard.press('Meta+b');
        await expect(pane).toBeHidden();
        await page.keyboard.press('Meta+b');
        await expect(pane).toBeVisible();
      });
    });
    test.describe('tablet', () => {
      test.use({ viewport: TABLET, isMobile: true, hasTouch: true });
      test('list beside the note, one header row', async ({ page }) => {
        await prep(page, s);
        await openPage(page, 'vault');
        const pane = page.getByTestId('vault-files');
        await expect(pane).toBeVisible({ timeout: 20_000 });
        await pane.getByText('runbooks', { exact: true }).tap();
        await pane.getByText('deploy', { exact: true }).tap();
        await expect(page.locator('.read')).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `vault-tablet-${s}`);
      });
    });
    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('push: list → note → Back → another note; details sheet', async ({ page }) => {
        await prep(page, s);
        await openPage(page, 'vault');
        const pane = page.getByTestId('vault-files');
        await expect(pane).toBeVisible({ timeout: 20_000 });
        await phoneChrome(page, true);
        await shot(page, `vault-phone-list-${s}`);
        await pane.getByText('services', { exact: true }).tap();
        await pane.getByText('auth-api', { exact: true }).tap();
        await expect(page.locator('.read')).toBeVisible();
        await expect(pane).toBeHidden();
        await phoneChrome(page, false);
        await shot(page, `vault-phone-content-${s}`);
        await page.getByTestId('vault-details-toggle').tap();
        const sheet = page.getByRole('dialog', { name: 'Note details' });
        await expect(sheet).toBeVisible();
        await settled(sheet);
        await shot(page, `vault-phone-detail-${s}`);
        await sheet.getByRole('button', { name: 'Close Note details' }).tap();
        await page.getByTestId('phone-back').tap();
        await expect(pane).toBeVisible();
        await expect(pane.locator('.side-row.selected')).toContainText('auth-api');
        await pane.getByText('orders-api', { exact: true }).tap();
        await expect(page.locator('.read')).toContainText(/charging/);
        await page.getByTestId('phone-back').tap();
        await expect(pane).toBeVisible();
        await expectNoHorizontalOverflow(page);
      });
    });
  });

  test.describe(`Product (${s})`, () => {
    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('stories list: header, Hide / Show, open on a story', async ({ page }) => {
        await prep(page, s);
        await openPage(page, 'product');
        const pane = page.getByTestId('product-stories');
        await expect(pane).toBeVisible({ timeout: 20_000 });
        await expect(pane.getByRole('heading', { name: 'Stories' })).toBeVisible();
        await pane.locator('.story-row', { hasText: 'Refund policy' }).click();
        await expect(pane.locator('.story-row-wrap.selected')).toContainText('Refund policy');
        await shot(page, `product-desktop-${s}`);
        await page.getByTestId('product-stories-more').click();
        await page.getByRole('menuitem', { name: /Hide Stories/ }).click();
        await expect(pane).toBeHidden();
        await page.getByTestId('show-list-pane').click();
        await expect(pane).toBeVisible();
      });
    });
    test.describe('tablet', () => {
      test.use({ viewport: TABLET, isMobile: true, hasTouch: true });
      test('stories beside the story', async ({ page }) => {
        await prep(page, s);
        await openPage(page, 'product');
        const pane = page.getByTestId('product-stories');
        await expect(pane).toBeVisible({ timeout: 20_000 });
        await pane.locator('.story-row', { hasText: 'Checkout journey' }).tap();
        await expectNoHorizontalOverflow(page);
        await shot(page, `product-tablet-${s}`);
      });
    });
    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('push: list → story → Back → another story', async ({ page }) => {
        await prep(page, s);
        await openPage(page, 'product');
        const pane = page.getByTestId('product-stories');
        await expect(pane).toBeVisible({ timeout: 20_000 });
        await phoneChrome(page, true);
        await shot(page, `product-phone-list-${s}`);
        await pane.locator('.story-row', { hasText: 'Refund policy' }).tap();
        await expect(page.getByTestId('phone-back')).toBeVisible();
        await expect(pane).toBeHidden();
        await phoneChrome(page, false);
        await shot(page, `product-phone-content-${s}`);
        await page.getByTestId('phone-back').tap();
        await expect(pane.locator('.story-row-wrap.selected')).toContainText('Refund policy');
        await pane.locator('.story-row', { hasText: 'Payouts v2' }).tap();
        await expect(page.getByTestId('phone-back')).toBeVisible();
        await page.getByTestId('phone-back').tap();
        await expect(pane).toBeVisible();
        await expectNoHorizontalOverflow(page);
      });
    });
  });

  test.describe(`Canvas (${s})`, () => {
    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('scenes list, the assistant Detail pane', async ({ page }) => {
        await prep(page, s);
        await page.goto(`/#/canvas/${sceneId}/editor`);
        const pane = page.getByTestId('canvas-scenes');
        await expect(pane).toBeVisible({ timeout: 30_000 });
        await expect(pane.getByRole('heading', { name: 'Scenes' })).toBeVisible();
        await shot(page, `canvas-desktop-${s}`);
        await page.getByRole('button', { name: 'Ask Otto', exact: true }).click();
        await expect(page.getByTestId('canvas-assistant')).toBeVisible();
        await shot(page, `canvas-desktop-detail-${s}`);
        await page.getByTestId('canvas-assistant-close').click();
        await expect(page.getByTestId('canvas-assistant')).toHaveCount(0);
      });
    });
    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('push: board → Back → list keeps the scene marked', async ({ page }) => {
        await prep(page, s);
        await page.goto(`/#/canvas/${sceneId}/editor`);
        await expect(page.locator('.board').first()).toBeVisible({ timeout: 30_000 });
        await shot(page, `canvas-phone-content-${s}`);
        await page.getByTestId('phone-back').tap();
        const pane = page.getByTestId('canvas-scenes');
        await expect(pane).toBeVisible();
        await phoneChrome(page, true);
        await expect(pane.locator('.side-row.selected')).toContainText('Payments flow');
        await shot(page, `canvas-phone-list-${s}`);
        await pane.locator('.side-row-main', { hasText: 'Payments flow' }).tap();
        await expect(page.locator('.board').first()).toBeVisible();
      });
    });
  });

  test.describe(`Design Hall (${s})`, () => {
    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('design details is the Detail pane', async ({ page }) => {
        await prep(page, s);
        await page.goto(`/#/design/a/${designId}`);
        const details = page.getByTestId('design-details');
        await expect(details).toBeVisible({ timeout: 30_000 });
        await expect(page.getByRole('tablist', { name: 'Details panel' })).toBeVisible();
        await shot(page, `design-desktop-detail-${s}`);
        await page.getByTestId('design-details-close').click();
        await expect(details).toHaveCount(0);
        await shot(page, `design-desktop-${s}`);
        await page.getByTestId('design-details-toggle').click();
        await expect(details).toBeVisible();
      });
    });
    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('details open as a sheet on request', async ({ page }) => {
        await prep(page, s);
        await page.goto(`/#/design/a/${designId}`);
        await expect(page.getByTestId('design-stage')).toBeVisible({ timeout: 30_000 });
        await shot(page, `design-phone-content-${s}`);
        await page.getByTestId('design-details-toggle').tap();
        const sheet = page.getByRole('dialog', { name: 'Design details' });
        await expect(sheet).toBeVisible();
        await settled(sheet);
        await shot(page, `design-phone-detail-${s}`);
        await sheet.getByRole('button', { name: 'Close Design details' }).tap();
        await expect(sheet).toBeHidden();
        await phoneChrome(page, false);
      });
    });
  });
}
