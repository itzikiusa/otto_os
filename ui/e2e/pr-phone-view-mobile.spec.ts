import { test, expect, type Page } from '@playwright/test';
import { expectFingerSized, expectNoHorizontalOverflow } from './helpers';

// The PR view on a phone (owner report 2026-11-25: "the text was smudged in
// the description, the buttons are weird, the PR's commits are hard to
// find"). MOCK mode (ui/src/lib/api/mock-prs.ts): #49 carries a hostile
// description — headings, nested and task lists, a wide table and code line,
// a long URL and identifier, an image, a quote, <details>, RTL text.

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name.startsWith('desktop'), 'phones and tablets');
  await page.addInitScript(() => localStorage.setItem('otto_mock', '1'));
});

type Box = { x: number; y: number; w: number; h: number; what: string };
/** Every text line box and control inside the description card. */
function boxes(page: Page): Promise<Box[]> {
  return page.locator('.prd-desc').evaluate((card) => {
    const out: Box[] = [];
    const walker = document.createTreeWalker(card, NodeFilter.SHOW_TEXT);
    let n: Node | null;
    let i = 0;
    while ((n = walker.nextNode())) {
      if (!n.textContent?.trim()) continue;
      const host = n.parentElement!;
      // A control's own label (measured as the control below), and a closed
      // <details> body (laid out, never painted) are not text on screen.
      if (host.closest('button')) continue;
      const det = host.closest('details');
      if (det && !det.open && !host.closest('summary')) continue;
      const r = document.createRange();
      r.selectNodeContents(n);
      const id = `text${i++}:${n.textContent.trim().slice(0, 24)}`;
      for (const b of r.getClientRects()) if (b.width > 0 && b.height > 0) out.push({ x: b.x, y: b.y, w: b.width, h: b.height, what: id });
    }
    for (const el of card.querySelectorAll('button, img')) {
      const b = el.getBoundingClientRect();
      if (b.width > 0 && b.height > 0) out.push({ x: b.x, y: b.y, w: b.width, h: b.height, what: el.tagName + ':' + (el.getAttribute('aria-label') ?? '') });
    }
    return out;
  });
}
const hit = (a: Box, b: Box): boolean =>
  a.x + a.w > b.x + 1 && b.x + b.w > a.x + 1 && a.y + a.h > b.y + 1 && b.y + b.h > a.y + 1;

test('a long, hostile description: nothing overlaps, nothing scrolls the page sideways, all of it is reachable', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/49');
  await expect(page.locator('.prd-desc .md-body')).toContainText('VeryLongIdentifier', { timeout: 15_000 });
  const all = await boxes(page);
  const clashes: string[] = [];
  for (let i = 0; i < all.length; i++) for (let j = i + 1; j < all.length; j++) {
    if (all[i].what !== all[j].what && hit(all[i], all[j])) clashes.push(`${all[i].what} × ${all[j].what}`);
  }
  expect(clashes, 'overlapping text in the description').toEqual([]);
  await expectNoHorizontalOverflow(page);
  // The last line scrolls into view and is what a finger lands on.
  const last = page.locator('.prd-desc .md-body > :last-child');
  await last.scrollIntoViewIfNeeded();
  await expect(last).toBeInViewport();
  const b = (await last.boundingBox())!;
  expect(await last.evaluate((el, [x, y]) => el.contains(document.elementFromPoint(x, y)), [b.x + 4, b.y + b.height / 2])).toBe(true);
});

test('the PR actions fill one row of finger-sized buttons', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const actions = page.locator('.prd-actions');
  const btns = await actions.locator('.btn:visible').all();
  await expectFingerSized(...btns);
  const ys = await Promise.all(btns.map(async (x) => Math.round((await x.boundingBox())!.y)));
  expect(new Set(ys).size, 'one row').toBe(1);
  // The row spans the header: the last button ends at the row's end, not mid-screen.
  const row = (await actions.boundingBox())!;
  const lastBtn = (await btns[btns.length - 1].boundingBox())!;
  expect(row.x + row.width - (lastBtn.x + lastBtn.width)).toBeLessThan(2);
});

test("the PR's commits are one tap away, and a commit shows its changes", async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  const tab = page.getByRole('tab', { name: 'Commits', exact: true });
  await expect(tab).toBeVisible();
  await expect(tab).toContainText('6');
  await expectFingerSized(tab);
  await tab.click();
  await expect(page.locator('.tlw')).toContainText('active pins page');
  await page.locator('.tlw .commits li', { hasText: 'active pins page' }).locator('.sha').click();
  await expect(page.getByRole('tab', { name: 'Files', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('.prd-commit-pick').first()).toContainText('active pins page');
  await expectNoHorizontalOverflow(page);
});

// A short description: the floated Edit (44 px on a touch screen) stays in
// the card — the card contains its float — for one line, for none, and for a
// PR template whose only content is an HTML comment (it renders nothing).
// (.prd-desc is a flex item of .prd-main, so it contains the float itself.)
test('a one-line, an empty and a comment-only description keep Edit inside the card', async ({ page }) => {
  const inside = async (): Promise<void> => {
    const card = (await page.locator('.prd-desc').boundingBox())!;
    const edit = (await page.getByRole('button', { name: 'Edit the title and description' }).boundingBox())!;
    expect(edit.y + edit.height, 'Edit bottom vs card bottom').toBeLessThanOrEqual(card.y + card.height + 0.5);
    expect(edit.y).toBeGreaterThanOrEqual(card.y - 0.5);
  };
  await page.goto('/#/git/rep_otto/pr/52');
  await expect(page.locator('.prd-desc .md-body')).toContainText('Caches', { timeout: 15_000 });
  await inside();
  await page.getByRole('button', { name: 'Edit the title and description' }).click();
  await page.getByRole('textbox', { name: 'Pull request description' }).fill('');
  await page.getByRole('button', { name: /^Save to / }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Update PR' }).click();
  await expect(page.locator('.prd-desc .md-body')).toContainText('No description', { timeout: 15_000 });
  await inside();
  await page.getByRole('button', { name: 'Edit the title and description' }).click();
  await page.getByRole('textbox', { name: 'Pull request description' }).fill('<!-- Describe your change. Link the ticket. -->');
  await page.getByRole('button', { name: /^Save to / }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Update PR' }).click();
  await expect(page.locator('.prd-desc .md-body')).not.toContainText('No description', { timeout: 15_000 });
  await inside();
});
