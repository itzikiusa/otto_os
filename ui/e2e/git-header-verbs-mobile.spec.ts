import { test, expect, type Locator, type Page } from '@playwright/test';
import { EXPECT, VERBS, seedVerbRepos, type VerbState } from './git-header-verbs-fixture';
import { expectFingerSized, expectNoHorizontalOverflow } from './helpers';

// Phone: the five everyday verbs are a second row of finger-sized buttons
// under the header (icon + short label, counts shown) on History — also
// through the legacy Changes link; the view switch stays where it is.

test.use({ serviceWorkers: 'block' });
const SHOTS = process.env.OTTO_E2E_SHOTS;
const PHONES = ['iphone-portrait', 'android-phone'];
let ids: Record<VerbState, string>;

test.beforeAll(async () => {
  ids = await seedVerbRepos(`hvm${Date.now().toString(36)}`);
});

const verb = (page: Page, name: string): Locator => page.getByTestId('git-verbs-row').getByRole('button', { name: new RegExp(`^${name}\\b`) }).first();

for (const s of ['synced', 'dirty', 'ahead', 'behind'] as VerbState[]) {
  test(`phone: the verbs row in the ${s} state`, async ({ page }, info) => {
    test.skip(!PHONES.includes(info.project.name), 'phone projects only');
    await page.addInitScript(() => localStorage.setItem('otto_firstrun_dismissed', '1'));
    for (const view of ['graph', 'changes']) {
      await page.goto(`/#/git/${ids[s]}/${view}`);
      await expect(page.locator('.gitpage')).toBeVisible({ timeout: 30_000 });
      await expect(page.getByTestId('git-verbs-row')).toBeVisible({ timeout: 25_000 });
      for (const v of VERBS) {
        const b = verb(page, v);
        await expect(b, `${s}/${view}: ${v}`).toBeVisible();
        await expect(b).toContainText(v);
        const want = EXPECT[s][v];
        if (want.enabled) await expect(b, `${s}: ${v} enabled`).toBeEnabled({ timeout: 15_000 });
        else await expect(b, `${s}: ${v} disabled`).toBeDisabled({ timeout: 15_000 });
        if (want.text) await expect(b).toContainText(want.text);
        await expectFingerSized(b);
        const box = await b.boundingBox();
        expect(box!.x + box!.width, `${v} inside the screen`).toBeLessThanOrEqual(page.viewportSize()!.width + 1);
      }
      // The view switch stays above the row.
      await expect(page.locator('.rv-tabs')).toBeVisible();
      await expectNoHorizontalOverflow(page);
      if (SHOTS && view === 'graph') {
        for (const scheme of ['light', 'dark'] as const) {
          await page.emulateMedia({ colorScheme: scheme });
          await page.screenshot({ path: `${SHOTS}/${info.project.name}-${s}-${scheme}.png` });
        }
        await page.emulateMedia({ colorScheme: 'light' });
      }
    }
    // Not on Pull requests (no git verbs belong there).
    await page.locator('.rv-tabs').getByRole('tab', { name: /^Pull requests/ }).click();
    await expect(page.getByTestId('git-verbs-row')).toHaveCount(0);
  });
}
