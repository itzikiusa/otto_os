import { test, expect, type Locator, type Page } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The PR workspace on phones and tablets (MOCK mode — ui/src/lib/api/mock-prs.ts).
// A PR must be fully workable on a 6–7" screen: every header action reachable
// (they wrap, never clip), the Code tab's toolbar whole and pinned while the
// diff scrolls (it used to collapse to one row and hide its search row under
// the first file), and nothing scrolls the page sideways.

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name.startsWith('desktop'), 'phones and tablets');
  await page.addInitScript(() => localStorage.setItem('otto_mock', '1'));
});

/** The element actually on top at `loc`'s centre is `loc` (or inside it). */
async function expectOnTop(page: Page, loc: Locator): Promise<void> {
  const box = await loc.boundingBox();
  expect(box, 'element has a box').not.toBeNull();
  const hit = await loc.evaluate((el, [x, y]) => {
    const top = document.elementFromPoint(x, y);
    return !!top && (el === top || el.contains(top));
  }, [box!.x + box!.width / 2, box!.y + box!.height / 2]);
  expect(hit, 'not covered by another element').toBe(true);
}

test('PR header: every action is on screen and reachable', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const actions = page.locator('.prd-actions');
  for (const name of ['Request changes', 'Approve', 'Merge', 'Check out']) {
    const b = actions.getByRole('button', { name, exact: name !== 'Check out' });
    await expectFullyInViewport(page, b.first(), name);
  }
  await expectFullyInViewport(page, actions.getByRole('button', { name: 'More pull request actions' }));
  await expectNoHorizontalOverflow(page);
});

test('Code tab: the toolbar is whole, on top, and stays pinned while scrolling', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
  await expect(page.locator('.dfile-head').first()).toBeVisible();
  const search = page.getByRole('textbox', { name: /search/i }).first();
  await expect(search).toBeVisible();
  await search.scrollIntoViewIfNeeded();
  await expectOnTop(page, search);
  // Open every file so there is something long to scroll through.
  await page.getByRole('button', { name: /expand all/i }).first().click().catch(() => {});
  const tb = page.locator('.diff-toolbar.pr-toolbar');
  const firstFile = page.locator('.dfile-head').first();
  const before = (await firstFile.boundingBox())!.y;
  // Scroll whatever scrolls the diff (its nearest scrolling ancestor) by a lot.
  await tb.evaluate((el) => {
    for (let p = el.parentElement; p; p = p.parentElement) {
      if (p.scrollHeight > p.clientHeight + 1 && /auto|scroll/.test(getComputedStyle(p).overflowY)) { p.scrollTop += 1500; return; }
    }
  });
  // The files moved up under the toolbar…
  await expect.poll(async () => (await firstFile.boundingBox())?.y ?? -1e9).toBeLessThan(before - 200);
  // …while the toolbar stayed whole, on screen and on top.
  await expectFullyInViewport(page, tb, "diff toolbar");
  await expectOnTop(page, search);
  await expectNoHorizontalOverflow(page);
});
