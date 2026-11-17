import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { bootPreviewIde } from './ide-preview-fixtures';

// The IDE quick-open on a phone: ⋯ › Search everywhere opens a full-width sheet at
// the top of the screen (not a centred dialog), its field is on top and usable,
// results scroll inside it, and nothing scrolls the page sideways.

const PHONES = ['iphone-portrait', 'iphone-landscape', 'iphone-se', 'android-phone'];

for (const scheme of ['dark', 'light'] as const) {
  test(`phone: quick-open is a top sheet with a usable field (${scheme})`, async ({ page }, info) => {
    test.skip(!PHONES.includes(info.project.name), 'phones');
    test.setTimeout(120_000);
    const p = info.project.name;
    await bootPreviewIde(page, scheme);
    await expect(page.getByTestId('ide-page')).toHaveClass(/\bstack\b/);

    await page.getByTestId('ide-more-btn').click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: /^Search everywhere/ }).click();
    const qo = page.getByTestId('ide-quick-open');
    await expect(qo).toBeVisible();
    await expectFullyInViewport(page, qo, `${p}: quick-open`);
    const vw = page.viewportSize()!.width;
    const box = (await qo.boundingBox())!;
    expect(box.width, `${p}: full width`).toBeGreaterThanOrEqual(vw - 2);
    expect(box.y, `${p}: anchored at the top`).toBeLessThan(60);

    const input = qo.getByTestId('ide-search-input');
    const ib = (await input.boundingBox())!;
    const onTop = await page.evaluate(
      ({ x, y }) => document.elementFromPoint(x, y)?.closest('[data-testid="ide-search-input"]') !== null,
      { x: ib.x + ib.width / 2, y: ib.y + ib.height / 2 },
    );
    expect(onTop, `${p}: the search field is on top`).toBe(true);
    await input.fill('main');
    await expect(qo.locator('[data-testid="ide-qo-row"][data-kind="hit"]').first()).toBeVisible({ timeout: 15_000 });
    await expect(qo.getByTestId('ide-se-find-options')).toHaveCount(0);
    expect((await qo.boundingBox())!.y, `${p}: top edge holds`).toBeCloseTo(box.y, 0);
    await expectNoHorizontalOverflow(page);
    const dir = process.env.OTTO_E2E_SHOTS;
    if (dir) {
      const out = join(dir, 'fix-IDE-QUICKOPEN');
      mkdirSync(out, { recursive: true });
      await page.screenshot({ path: join(out, `qo-${p}-${scheme}.png`) });
    }
    await page.keyboard.press('Escape');
    await expect(qo).toHaveCount(0);
  });
}
