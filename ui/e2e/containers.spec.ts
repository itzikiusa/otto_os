import { test, expect } from '@playwright/test';
import { mockContainersApi } from './containers-mock';
import {
  expectContentHasHeight,
  expectFullyInViewport,
  expectNoHorizontalOverflow,
  openPage,
} from './helpers';

// Real Playwright E2E coverage for Otto's native Containers module:
//   - Offline daemon + 1-click "Install & Start Kiri" flow + fallback context switcher
//   - Online Compose stack grouping, auto-selection of first running container
//   - Overview env secret masking + click-to-reveal, Logs find, Stats cards
//   - Images / Volumes / Networks tabs, Run container modal, viewport-clamped ⋯ menu
//   - Responsive desktop, tablet, and phone (push nav) + light/dark parity + zero overflow

test.use({ serviceWorkers: 'block' });

test.describe('Containers — desktop', () => {
  test.use({ viewport: { width: 1280, height: 800 }, isMobile: false, hasTouch: false });

  test('offline state offers 1-click Install & Start Kiri and transitions into live containers', async ({ page }) => {
    const tracker = await mockContainersApi(page, { initialOffline: true, manyContexts: 18 });
    await openPage(page, 'containers');

    await expect(page.getByTestId('containers-page')).toBeVisible();
    await expect(page.getByRole('heading', { name: /Kiri isn’t running/i })).toBeVisible();

    const installBtn = page.getByRole('button', { name: 'Install & Start Kiri' });
    await expect(installBtn).toBeVisible();
    await expect(page.getByRole('button', { name: 'Switch to colima' })).toBeVisible();

    // Overflow ⋯ menu stays clamped inside viewport and offers Install Kiri runtime….
    const moreBtn = page.getByRole('button', { name: 'More actions', exact: true });
    await moreBtn.click();
    const menu = page.getByRole('menu', { name: 'Context menu' });
    await expect(menu.getByRole('menuitem', { name: 'Install Kiri runtime…' })).toBeVisible();
    await expectFullyInViewport(page, menu, 'Containers more menu');
    await page.keyboard.press('Escape');

    // Trigger 1-click Install & Start Kiri → transitions to online view.
    await installBtn.click();
    await expect.poll(() => tracker.installCalls()).toBe(1);
    await expect(page.getByTestId('container-detail')).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('container-detail').getByRole('heading', { name: 'payments-api' })).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });

  test('online views cover Compose groups, masked env reveal, logs, stats, resource tabs, and Run modal', async ({
    page,
  }) => {
    await mockContainersApi(page, { initialOffline: false, manyContexts: 18 });
    await openPage(page, 'containers');

    await expect(page.getByTestId('containers-page')).toBeVisible();
    await expectContentHasHeight(page);

    // Opens directly on first running container (no empty "pick one" pane).
    const detail = page.getByTestId('container-detail');
    await expect(detail.getByRole('heading', { name: 'payments-api' })).toBeVisible();

    // Every row says its state in a word; the detail has one state action
    // (Stop) and Remove lives in ⋯, never beside Stop.
    await expect(page.getByTestId('container-state').first()).toHaveText('Running');
    await expect(detail.getByRole('button', { name: 'Stop' })).toBeVisible();
    await expect(detail.getByRole('button', { name: 'Remove container' })).toHaveCount(0);
    await detail.getByTestId('container-more').click();
    await expect(page.getByRole('menu', { name: 'Context menu' }).getByRole('menuitem', { name: 'Remove container…' })).toBeVisible();
    await page.keyboard.press('Escape');

    // Masked secret reveals on demand via click-to-reveal eye button.
    await expect(detail.getByText('••••••••')).toBeVisible();
    await detail.getByRole('button', { name: 'Reveal DATABASE_PASSWORD' }).click();
    await expect(detail.getByText('s3cr3t-pass-42')).toBeVisible({ timeout: 10_000 });

    // Logs tab: renders log lines and supports in-buffer find.
    await detail.getByRole('tab', { name: 'Logs' }).click();
    await expect(detail.getByText('Payments API listening on :3000')).toBeVisible({ timeout: 10_000 });
    await detail.getByLabel('Find in logs').fill('ledger');
    await expect(detail.getByText('WARN retrying ledger sync')).toBeVisible();
    await expect(detail.getByText('Payments API listening on :3000')).toHaveCount(0);

    // Stats tab: polls live stats snapshot.
    await detail.getByRole('tab', { name: 'Stats' }).click();
    await expect(detail.getByText('4.2%')).toBeVisible({ timeout: 10_000 });
    await expect(detail.getByText('12.5%')).toBeVisible();

    // Images, Volumes, Networks tabs render cleanly without overflow.
    const sections = page.getByRole('tablist', { name: 'Containers sections' });
    await sections.getByRole('tab', { name: 'Images' }).click();
    await expect(page.getByText('ghcr.io/acme/payments')).toBeVisible({ timeout: 10_000 });
    await expectNoHorizontalOverflow(page);

    // Run from an image row: an unlisted image suggests the TCP ports it declares.
    await page.getByRole('button', { name: 'Run ghcr.io/acme/payments:1.4' }).click();
    const rowModal = page.getByRole('dialog', { name: 'Run container' });
    await expect(rowModal.getByTestId('run-suggestion')).toContainText('Suggested for acme/payments: 3000:3000');
    await rowModal.getByRole('button', { name: 'Cancel' }).click();

    await sections.getByRole('tab', { name: 'Volumes' }).click();
    await expect(page.getByText('payments_data')).toBeVisible({ timeout: 10_000 });
    await expectNoHorizontalOverflow(page);

    await sections.getByRole('tab', { name: 'Networks' }).click();
    await expect(page.getByText('payments_default')).toBeVisible({ timeout: 10_000 });
    await expectNoHorizontalOverflow(page);

    // Run… modal opens cleanly inside viewport.
    await page.getByRole('button', { name: 'Run…' }).click();
    const modal = page.getByRole('dialog', { name: 'Run container' });
    await expectFullyInViewport(page, modal, 'Run container modal');
    // A well-known image suggests its ports + env; Use fills only the empty fields.
    await modal.getByLabel('Image', { exact: true }).fill('mongo:9.0');
    await expect(modal.getByTestId('run-suggestion')).toContainText(
      'Suggested for mongo: 27017:27017 · MONGO_INITDB_ROOT_USERNAME, MONGO_INITDB_ROOT_PASSWORD',
    );
    await modal.getByLabel('Container name').fill('db');
    await modal.getByLabel('Environment variables').fill('TZ=UTC');
    await modal.getByRole('button', { name: 'Use suggested settings for mongo' }).click();
    await expect(modal.getByLabel('Ports', { exact: true })).toHaveValue('27017:27017');
    await expect(modal.getByLabel('Environment variables')).toHaveValue('TZ=UTC');
    await expectFullyInViewport(page, modal, 'Run container modal with a suggestion');
    await modal.getByRole('button', { name: 'Cancel' }).click();

    // More menu stays clamped inside viewport even with 18 contexts.
    await page.getByRole('button', { name: 'More actions', exact: true }).click();
    const overflowMenu = page.getByRole('menu', { name: 'Context menu' });
    await expect(overflowMenu.getByRole('menuitem').first()).toBeVisible();
    await expectFullyInViewport(page, overflowMenu, 'Containers overflow menu');
    await page.keyboard.press('Escape');

    await expectNoHorizontalOverflow(page);
  });
});

test.describe('Containers — responsive & themes', () => {
  for (const scheme of ['light', 'dark'] as const) {
    test(`renders cleanly in ${scheme} scheme with zero horizontal overflow and phone push nav`, async ({
      page,
    }) => {
      await mockContainersApi(page, { initialOffline: false });
      await page.addInitScript((s) => {
        localStorage.setItem('otto_scheme', s);
      }, scheme);
      await openPage(page, 'containers');
      await expect(page.getByTestId('containers-page')).toBeVisible();
      await expectNoHorizontalOverflow(page);

      const width = page.viewportSize()?.width ?? 1280;
      if (width <= 640) {
        // Phone uses push navigation: the list is the page; tap a row → its
        // detail with "‹ Containers" in the header → Back → the list, the row
        // still marked → another row.
        const rows = page.getByTestId('container-row');
        await rows.first().tap();
        await expect(page.getByTestId('container-detail')).toBeVisible();
        await expect(rows.first()).toBeHidden();
        await expectNoHorizontalOverflow(page);
        await page.getByRole('button', { name: 'Back to Containers', exact: true }).tap();
        await expect(rows.first()).toBeVisible();
        await expect(rows.first()).toHaveAttribute('aria-current', 'true');
        await expect(page.getByTestId('container-detail')).toBeHidden();
        await rows.nth(1).tap();
        await expect(page.getByTestId('container-detail')).toBeVisible();
        await expect(page.getByRole('button', { name: 'Back to Containers', exact: true })).toBeVisible();
      }
    });
  }
});
