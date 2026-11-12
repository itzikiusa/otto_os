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

/** Open PR #51's Code tab (23 files, every one collapsed: > 5 files). */
async function openCode(page: Page): Promise<void> {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('tab', { name: 'Code', exact: true }).click();
  await expect(page.locator('.dfile-head').first()).toBeVisible();
}

test('Code tab: step between files, pick one from the sheet, mark it viewed', async ({ page }) => {
  test.skip((page.viewportSize()?.width ?? 0) > 1024, 'the file stepper is the ≤1024 px layout (wider shows the Files panel)');
  await openCode(page);
  const stepper = page.getByRole('group', { name: 'Files in this pull request' });
  await expect(stepper).toBeVisible();
  // Every control is a thumb-sized target, fully on screen.
  for (const b of await stepper.getByRole('button').all()) {
    const box = (await b.boundingBox())!;
    expect(box.height, 'touch target height').toBeGreaterThanOrEqual(44);
    expect(box.width, 'touch target width').toBeGreaterThanOrEqual(44);
    await expectFullyInViewport(page, b, 'file stepper control');
  }
  const counter = stepper.getByRole('button', { name: /pick a file/ });
  const fileNo = async (): Promise<number> => Number(/^File (\d+) of 23/.exec((await counter.getAttribute('aria-label')) ?? '')?.[1] ?? -1);
  await expect.poll(fileNo).toBe(1);
  // › and ‹ walk the files in diff order, scrolling the diff to each.
  await stepper.getByRole('button', { name: 'Next file' }).click();
  await expect.poll(fileNo).toBe(2);
  await stepper.getByRole('button', { name: 'Next file' }).click();
  await expect.poll(fileNo).toBe(3);
  await stepper.getByRole('button', { name: 'Previous file' }).click();
  await expect.poll(fileNo).toBe(2);
  await expectNoHorizontalOverflow(page);

  // The counter opens the file tree; a pick closes it and shows that file.
  await counter.click();
  const sheet = page.getByRole('dialog', { name: 'Files' });
  await expect(sheet).toBeVisible();
  await sheet.getByRole('searchbox', { name: 'Filter files' }).fill('player-details');
  await sheet.getByRole('button', { name: 'Go to libs/data-models/player-details.ts' }).click();
  await expect(sheet).toBeHidden();
  await expect(stepper.locator('.dfs-name')).toHaveText('player-details.ts');
  await expect(page.locator('[id="dfile-libs/data-models/player-details.ts"] .dfile-head')).toBeInViewport();

  // Viewed by thumb: the stepper's toggle and the file header agree.
  const viewed = stepper.getByRole('button', { name: 'Mark this file viewed' });
  await viewed.click();
  await expect(stepper.getByRole('button', { name: /^Viewed/ })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('checkbox', { name: 'Mark libs/data-models/player-details.ts as viewed' })).toBeChecked();
  // Next unviewed moves on to another file.
  await stepper.getByRole('button', { name: 'Next unviewed file' }).click();
  await expect(stepper.locator('.dfs-name')).not.toHaveText('player-details.ts');
  await expectNoHorizontalOverflow(page);
});

test('Code tab: a tap on a line opens its comment composer, clear of the toolbar and the keyboard', async ({ page }, ti) => {
  test.skip(!ti.project.use.hasTouch, 'touch devices');
  await openCode(page);
  // Open the first file and tap one of its added lines (not the gutter).
  await page.locator('.dfile-head').first().click();
  const code = page.locator('.diff-body .vrow.dline.add .code').first();
  await expect(code).toBeVisible();
  await code.tap();
  const box = page.getByRole('textbox', { name: 'Line comment' });
  await expect(box).toBeVisible();
  await box.tap();
  await box.fill('Can this come from config?');
  // Focus centres the composer: whole, on screen, and not under the pinned toolbar.
  await expect(async () => {
    await expectFullyInViewport(page, box, 'comment composer');
    await expectOnTop(page, box);
    await expectOnTop(page, page.getByRole('button', { name: 'Comment', exact: true }));
  }).toPass({ timeout: 5_000 });
  // A second tap on the same line keeps the composer (and the typed text).
  await code.tap();
  await expect(box).toHaveValue('Can this come from config?');
  await expectNoHorizontalOverflow(page);
});
