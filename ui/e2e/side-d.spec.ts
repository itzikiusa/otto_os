import { test, expect, type Page } from '@playwright/test';
import { expectNoHorizontalOverflow, openPage } from './helpers';
import { mockContainersApi } from './containers-mock';
import { mockBrokersApi, seedBrokersCluster } from './brokers-mock';

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
  // Fonts loaded and every finite animation (a sheet sliding in) settled.
  await page.evaluate(async () => {
    await document.fonts.ready;
    await Promise.all(document.getAnimations().filter((a) => a.effect?.getTiming().iterations !== Infinity).map((a) => a.finished.catch(() => {})));
  });
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

for (const s of ['light', 'dark'] as const) {
  test.describe(`Brokers side surfaces (${s})`, () => {
    let seed: { workspaceId: string; clusterId: string; clusterName: string };
    test.beforeAll(async () => {
      seed = await seedBrokersCluster(`side-d-${s}`);
    });
    async function open(page: Page): Promise<void> {
      await scheme(page, s);
      await mockBrokersApi(page, seed.clusterId);
      await page.addInitScript((wsId) => localStorage.setItem('otto_workspace', wsId as string), seed.workspaceId);
      await page.goto('/#/brokers');
      await expect(page.getByTestId('brokers-cluster-list')).toBeVisible({ timeout: 30_000 });
    }
    const row = (page: Page) => page.locator('.cluster', { hasText: seed.clusterName }).first();
    const tab = (page: Page, name: string) => page.getByRole('tablist', { name: 'Kafka cluster views' }).getByRole('tab', { name });

    test.describe('desktop', () => {
      test.use({ viewport: DESKTOP, isMobile: false, hasTouch: false });
      test('cluster list, nested Groups / Subjects lists and the message Detail pane', async ({ page }) => {
        await open(page);
        await row(page).locator('[data-side-row]').click();
        await expect(row(page).locator('[data-side-row]')).toHaveAttribute('aria-selected', 'true');
        await expect(page.locator('.cluster-head .ph-title')).toHaveText(seed.clusterName);
        await shot(page, `brokers-desktop-${s}`);
        // ⋯ › Hide Clusters, then Show Clusters in the page header.
        await page.getByTestId('brokers-cluster-list-more').click();
        await page.getByRole('menuitem', { name: /Hide Clusters/ }).click();
        await expect(page.getByTestId('brokers-cluster-list')).toBeHidden();
        await page.getByTestId('show-list-pane').click();
        await expect(page.getByTestId('brokers-cluster-list')).toBeVisible();
        // Consumer groups: a list pane that opens on the first group.
        await tab(page, 'Consumer groups').click();
        const groups = page.getByTestId('brokers-groups-list');
        await expect(groups.locator('.grow-row').first()).toHaveAttribute('aria-current', 'true');
        await expect(page.locator('.gid.big')).toHaveText('billing');
        await groups.locator('.grow-row', { hasText: 'audit' }).click();
        await expect(page.locator('.gid.big')).toHaveText('audit');
        await shot(page, `brokers-desktop-groups-${s}`);
        // Schema registry: subjects list pane, first subject open.
        await tab(page, 'Schema Registry').click();
        const subjects = page.getByTestId('brokers-subjects-list');
        await expect(subjects.locator('.srow').first()).toHaveAttribute('aria-current', 'true');
        await expect(page.locator('.sn-big')).toHaveText('orders-value');
        // Topic messages: Peek opens the first message in the Detail pane.
        await tab(page, 'Topics').click();
        await page.getByRole('row', { name: 'Open topic orders' }).click();
        await page.getByRole('button', { name: 'Peek', exact: true }).click();
        const detail = page.getByTestId('message-detail');
        await expect(detail).toBeVisible();
        await expect(detail.getByRole('heading', { name: 'P0 · offset 0' })).toBeVisible();
        await page.getByRole('button', { name: 'Inspect partition 0 offset 1' }).click();
        await expect(detail.locator('.payload').last()).toContainText('card_declined');
        await shot(page, `brokers-desktop-message-${s}`);
        await page.getByTestId('message-detail-close').click();
        await expect(detail).toBeHidden();
      });
    });

    test.describe('tablet', () => {
      test.use({ viewport: TABLET, isMobile: true, hasTouch: true });
      test('cluster list beside the cluster; the message detail fits or is a sheet', async ({ page }) => {
        await open(page);
        await row(page).locator('[data-side-row]').tap();
        await expect(page.locator('.cluster-head .ph-title')).toHaveText(seed.clusterName);
        await expectNoHorizontalOverflow(page);
        await shot(page, `brokers-tablet-${s}`);
        // No room for two lists beside the group: the groups push (Back in the page header).
        await tab(page, 'Consumer groups').tap();
        const groups = page.getByTestId('brokers-groups-list');
        await expect(groups.locator('.grow-row', { hasText: 'audit' })).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `brokers-tablet-groups-${s}`);
        await groups.locator('.grow-row', { hasText: 'audit' }).tap();
        await expect(page.locator('.gid.big')).toHaveText('audit');
        await expect(page.locator('.gid.big')).toBeInViewport();
        await shot(page, `brokers-tablet-group-${s}`);
        await page.getByRole('button', { name: 'Back to Groups', exact: true }).tap();
        await expect(groups.locator('.grow-row', { hasText: 'audit' })).toHaveAttribute('aria-current', 'true');
      });
    });

    test.describe('phone', () => {
      test.use({ viewport: PHONE, isMobile: true, hasTouch: true });
      test('push: clusters → cluster → Back → again; groups push inside; message sheet; ☰ and Side panel', async ({ page }) => {
        await open(page);
        const r = row(page).locator('[data-side-row]');
        await expect(r).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await shot(page, `brokers-phone-list-${s}`);
        await r.tap();
        const back = page.getByRole('button', { name: 'Back to Clusters', exact: true });
        await expect(back).toBeVisible();
        await expect(page.getByTestId('brokers-cluster-list')).toBeHidden();
        await expectNoHorizontalOverflow(page);
        await shot(page, `brokers-phone-content-${s}`);
        await back.tap();
        await expect(r).toHaveAttribute('aria-selected', 'true');
        await r.tap();
        // Consumer groups: the group list is the page, a group is pushed.
        await tab(page, 'Consumer groups').tap();
        const groups = page.getByTestId('brokers-groups-list');
        await expect(groups.locator('.grow-row', { hasText: 'audit' })).toBeVisible();
        await shot(page, `brokers-phone-groups-${s}`);
        await groups.locator('.grow-row', { hasText: 'audit' }).tap();
        await expect(page.locator('.gid.big')).toHaveText('audit');
        const backGroups = page.getByRole('button', { name: 'Back to Groups', exact: true });
        await expect(backGroups).toBeVisible();
        await shot(page, `brokers-phone-group-${s}`);
        await backGroups.tap();
        await expect(groups.locator('.grow-row', { hasText: 'audit' })).toHaveAttribute('aria-current', 'true');
        // A message opens as a full-width sheet.
        await tab(page, 'Topics').tap();
        await page.getByRole('row', { name: 'Open topic orders' }).tap();
        await page.getByRole('button', { name: 'Peek', exact: true }).tap();
        await page.getByRole('button', { name: 'Inspect partition 0 offset 1' }).tap();
        const detail = page.getByTestId('message-detail');
        await expect(detail).toBeVisible();
        await shot(page, `brokers-phone-message-${s}`);
        await page.getByTestId('message-detail-close').tap();
        await expect(detail).toBeHidden();
        await back.tap();
        await page.getByTestId('open-navigator').tap();
        await expect(page.getByRole('dialog', { name: 'Navigator' })).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(page.getByTestId('open-side-panel')).toBeVisible();
      });
    });
  });
}
