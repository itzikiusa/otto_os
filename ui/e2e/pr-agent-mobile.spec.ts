import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The PR's agent thread on phones and tablets (MOCK mode — mock-runtime.ts):
// the left column does not exist ≤1024 px, so the thread opens in a
// full-height sheet from the PR header's ⋯, and nothing scrolls sideways.

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name.startsWith('desktop'), 'phones and tablets');
  await page.addInitScript(() => localStorage.setItem('otto_mock', '1'));
});

test('the agent thread opens in a sheet from ⋯ and fits the screen', async ({ page }) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('pr-agent-panel')).toHaveCount(0); // no column here

  await page.getByRole('button', { name: 'More pull request actions' }).click();
  await page.locator('.ctx-menu').getByText('Ask a question', { exact: true }).click();
  const sheet = page.getByRole('dialog', { name: 'Agent · PR #51' });
  await expect(sheet).toBeVisible();
  const ask = sheet.getByRole('textbox', { name: 'Ask about this pull request' });
  await expectFullyInViewport(page, ask, 'ask box');
  await expectNoHorizontalOverflow(page);

  await sheet.getByRole('button', { name: 'Explain this PR' }).click();
  await expect(sheet.locator('[data-testid^="runtime-thread-"]')).toBeVisible({ timeout: 10_000 });
  await expectNoHorizontalOverflow(page);

  // Closing the sheet leaves the PR where it was.
  await sheet.getByRole('button', { name: 'Close Agent · PR #51' }).click();
  await expect(sheet).toBeHidden();
  await expect(page.locator('.prd-title')).toBeVisible();
});

// Review screenshots are opt-in: set OTTO_E2E_SHOTS to a folder to keep them.
const SHOTS = process.env.OTTO_E2E_SHOTS ? `${process.env.OTTO_E2E_SHOTS}/embed` : '';

test('in the sheet the start composer’s agent and access chips are reachable, and the picks start the thread', async ({ page }, ti) => {
  await page.goto('/#/git/rep_otto/pr/51');
  await expect(page.locator('.prd-title')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'More pull request actions' }).click();
  await page.locator('.ctx-menu').getByText('Ask a question', { exact: true }).click();
  const sheet = page.getByRole('dialog', { name: 'Agent · PR #51' });
  const draft = sheet.getByTestId('embedded-runtime-draft');
  await expect(draft).toBeVisible({ timeout: 10_000 });
  for (const id of ['picker-agent', 'runtime-access', 'new-thread-send-btn', 'new-thread-input']) {
    await expectFullyInViewport(page, draft.getByTestId(id), id);
  }
  await expectNoHorizontalOverflow(page);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/pr-sheet-draft-${ti.project.name}-light.png`, animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'dark' });
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/pr-sheet-draft-${ti.project.name}-dark.png`, animations: 'disabled' });
  await page.emulateMedia({ colorScheme: 'light' });

  // The model picker opens inside the screen and scrolls rather than spilling.
  await draft.getByTestId('picker-agent').click();
  const picker = page.getByTestId('model-picker');
  await expect(picker).toBeVisible();
  await expectFullyInViewport(page, picker, 'model picker');
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/pr-sheet-picker-${ti.project.name}-light.png`, animations: 'disabled' });
  await picker.getByTestId('model-picker-provider-codex').click();
  await picker.locator('.mp-row:not([disabled])').filter({ hasNotText: 'Default model' }).first().click();
  if (await picker.isVisible()) await page.getByRole('button', { name: 'Close model picker' }).click({ force: true });
  await expect(draft.getByTestId('picker-agent')).toHaveAttribute('aria-label', /Codex/);

  // The access menu fits the screen too.
  await draft.getByTestId('runtime-access').click();
  await expectFullyInViewport(page, page.locator('.ctx-menu'), 'access menu');
  await page.keyboard.press('Escape');

  await draft.getByTestId('new-thread-input').fill('Which callers change?');
  await draft.getByTestId('new-thread-send-btn').click();
  await expect(sheet.locator('[data-testid^="runtime-thread-"]')).toBeVisible({ timeout: 10_000 });
  const log = await page.evaluate(() => (window as unknown as { __mockRuntime: { creates: Record<string, unknown>[] } }).__mockRuntime);
  expect(log.creates[0]).toMatchObject({ provider: 'codex', access_mode: 'approval-required', source: 'pr-agent' });
  expect(typeof log.creates[0].model).toBe('string');
  await expectNoHorizontalOverflow(page);
});
