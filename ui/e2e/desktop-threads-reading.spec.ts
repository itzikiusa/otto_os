import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
import { at, ITEMS, mockThread, TURNS } from './threadsReadingFixture';

// ─────────────────────────────────────────────────────────────────────────────
// The Threads reading experience on a mocked, finished thread (the shape every
// provider's driver emits: user / assistant messages, commands, file changes
// with a unified diff or old/new text): the 760 px reading column in reading
// type, the finished turn's one-line work fold, the end-of-turn changes card
// and the composer card. OTTO_E2E_SHOTS=<dir> keeps light/dark screenshots.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });
const SHOTS = process.env.OTTO_E2E_SHOTS ?? '';
let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base, 'Reading WS');
  await ctx.dispose();
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

async function shoot(page: Page, name: string): Promise<void> {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: `${SHOTS}/${name}-${scheme}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}


test('a finished thread reads as one calm column', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await mockThread(page, workspaceId, 'read-1', ITEMS, TURNS);
  await page.goto('/#/threads/read-1');
  const thread = page.getByTestId('runtime-thread-read-1');
  await expect(thread.getByText('Offline edits now survive')).toBeVisible({ timeout: 30_000 });
  // R1: one 760 px column in reading type.
  const prose = thread.locator('.rt-assistant .md-body').first();
  expect(await prose.evaluate((el) => getComputedStyle(el).fontSize)).toBe('15px');
  const col = (await thread.locator('.rt-col').boundingBox())!;
  const dock = (await thread.locator('.rt-composer').boundingBox())!;
  expect(col.width).toBeLessThanOrEqual(760 + 2 * 24 + 1);
  expect(Math.abs(dock.x + dock.width / 2 - (col.x + col.width / 2))).toBeLessThan(2);
  // R2: the finished turn is one line that says what was done.
  await expect(thread.locator('[data-testid^="runtime-fold-"] > summary')).toHaveText(/^\s*Worked for 2m 13s · ran 4 commands and edited 3 files\s*$/);
  // R3: the end-of-turn changes card, names only where an edit carries no text.
  const card = thread.getByTestId('runtime-turn-changes');
  await expect(card).toContainText('3 changed files');
  await expect(card).toContainText('+6');
  await expect(card).toContainText('−2');
  await expect(card.getByRole('button', { name: /3 changed files/ })).toHaveAttribute('aria-expanded', 'true'); // ≤ 5 files: open
  await expect(card.locator('li')).toHaveText([/src\/sync\/queue\.ts\s*\+4\s*−1/, /src\/notes\.ts\s*\+2\s*−1/, /^\s*README\.md\s*$/]);
  // R5: times show on hover only (still in the accessibility tree).
  const time = thread.getByTestId('runtime-assistant-time');
  await page.mouse.move(5, 500);
  await expect(time).toHaveCSS('opacity', '0');
  await thread.locator('.rt-assistant').hover();
  await expect(time).toHaveCSS('opacity', '1');
  // R4: the composer row keeps only "+", the non-default access, the model and the primary.
  await expect(thread.getByTestId('runtime-plan')).toHaveCount(0);
  await thread.getByTestId('runtime-plus').click();
  const menu = page.locator('.ctx-menu');
  await expect(menu.getByRole('menuitem', { name: /Attach an image or PDF/ })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /Mention a file or agent/ })).toBeVisible();
  await expect(menu.getByRole('menuitemcheckbox', { name: /Access: Supervised/ })).toBeVisible();
  await page.keyboard.press('Escape');
  await shoot(page, 'desktop');
  await card.getByTestId('runtime-turn-changes-open').click();
  await expect(page.getByTestId('threads-inspector').or(page.locator('[data-testid^="thread-changes"]')).first()).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(thread.getByText('Offline edits now survive')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await shoot(page, 'phone');
});

test('while a turn runs, its finished steps fold to "N earlier steps" and the live one shows', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const live = ITEMS.slice(0, 8).map((i) => (i.id === 'c4' ? { ...i, status: 'running', completed_at: null } : i));
  await mockThread(page, workspaceId, 'read-2', live, [{ id: 't1', ordinal: 1, status: 'running', started_at: at(0), completed_at: null }]);
  await page.goto('/#/threads/read-2');
  const thread = page.getByTestId('runtime-thread-read-2');
  const earlier = thread.getByTestId('runtime-earlier-steps');
  await expect(earlier).toContainText('6 earlier steps', { timeout: 30_000 });
  await expect(thread.locator('[data-item-type="command_execution"]').filter({ hasText: 'npm run lint' })).toBeVisible();
  const older = thread.locator('[data-item-type="command_execution"]').filter({ hasText: 'npm test -- sync' });
  await expect(older).toBeHidden();
  await earlier.locator('summary').click();
  await expect(older).toBeVisible();
  await shoot(page, 'running');
});
