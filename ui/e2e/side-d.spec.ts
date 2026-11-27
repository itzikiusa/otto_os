import { test, expect, type Page } from '@playwright/test';
import { expectNoHorizontalOverflow, openPage } from './helpers';
import { mockContainersApi } from './containers-mock';

// The rc.6 side surfaces of Containers, Brokers and Database on the three
// sizes, driven with real taps on a phone: list → item → ‹ Back → another
// item; ⋯ › Hide / Show (⌘B) on desktop; the Navigator (☰) and the Side
// panel still one tap away. With OTTO_SHOTS_DIR set it also saves light and
// dark screenshots of each state.

test.use({ serviceWorkers: 'block' });

const SHOTS = process.env.OTTO_SHOTS_DIR;
const PHONE = { width: 390, height: 844 };
const TABLET = { width: 820, height: 1180 };
const DESKTOP = { width: 1440, height: 900 };

async function shot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: `${SHOTS}/${name}.png` });
}

async function scheme(page: Page, s: 'light' | 'dark'): Promise<void> {
  await page.addInitScript((v) => localStorage.setItem('otto_scheme', v), s);
}

for (const s of ['light', 'dark'] as const) {
  test.describe(`Containers side surfaces (${s})`, () => {
    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('list pane: header, Hide / Show under ⋯ and ⌘B, open on an item', async ({ page }) => {
        await scheme(page, s);
        await mockContainersApi(page, { initialOffline: false });
        await openPage(page, 'containers');
        const pane = page.getByTestId('containers-list');
        await expect(pane).toBeVisible();
        await expect(pane.getByRole('heading', { name: 'Containers' })).toBeVisible();
        await expect(page.getByTestId('container-detail')).toBeVisible();
        await expect(page.getByTestId('container-row').first()).toHaveAttribute('aria-current', 'true');
        await shot(page, `containers-desktop-${s}`);
        await page.getByTestId('containers-list-more').click();
        await page.getByRole('menuitem', { name: /Hide Containers/ }).click();
        await expect(pane).toBeHidden();
        await page.getByTestId('show-list-pane').click();
        await expect(pane).toBeVisible();
        await page.getByTestId('container-row').first().focus();
        await page.keyboard.press('ArrowDown');
        await expect(page.getByTestId('container-row').nth(1)).toBeFocused();
        await page.keyboard.press('Meta+b');
        await expect(pane).toBeHidden();
        await page.keyboard.press('Meta+b');
        await expect(pane).toBeVisible();
      });
    });
    test.describe('tablet', () => {
      test.use({ viewport: TABLET, isMobile: true, hasTouch: true });
      test('list pane beside the detail, one header row', async ({ page }) => {
        await scheme(page, s);
        await mockContainersApi(page, { initialOffline: false });
        await openPage(page, 'containers');
        await expect(page.getByTestId('containers-list')).toBeVisible();
        await expect(page.getByTestId('container-detail')).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `containers-tablet-${s}`);
      });
    });
    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('push: list → container → Back → another; ☰ and the Side panel', async ({ page }) => {
        await scheme(page, s);
        await mockContainersApi(page, { initialOffline: false });
        await openPage(page, 'containers');
        const rows = page.getByTestId('container-row');
        await expect(rows.first()).toBeVisible();
        await expect(page.getByTestId('container-detail')).toBeHidden();
        await expect(page.getByRole('tablist', { name: 'Containers sections' })).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `containers-phone-list-${s}`);
        await rows.nth(1).tap();
        await expect(page.getByTestId('container-detail')).toBeVisible();
        const back = page.getByRole('button', { name: 'Back to Containers', exact: true });
        await expect(back).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `containers-phone-content-${s}`);
        await back.tap();
        await expect(rows.nth(1)).toHaveAttribute('aria-current', 'true');
        await rows.first().tap();
        await expect(page.getByTestId('container-detail').getByRole('heading', { name: 'payments-api' })).toBeVisible();
        await back.tap();
        await page.getByTestId('open-navigator').tap();
        await expect(page.getByRole('dialog', { name: 'Navigator' })).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(page.getByTestId('open-side-panel')).toBeVisible();
      });
    });
  });
}
