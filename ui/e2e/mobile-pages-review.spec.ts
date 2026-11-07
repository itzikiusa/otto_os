import { test, expect, type Page, type TestInfo } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { seedProductStory } from './seed-product';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });

async function workspace(page: Page): Promise<string> {
  const { ctx, base } = await apiCtx();
  const id = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_right_open', '1');
    localStorage.setItem('otto_scheme', 'light');
    (window as unknown as { __ottoTermProbe: unknown[] }).__ottoTermProbe = [];
  }, id);
  return id;
}

async function capture(page: Page, info: TestInfo, name: string): Promise<void> {
  for (const scheme of ['light', 'dark'] as const) {
    await page.evaluate(async (scheme) => {
      const path = '/src/lib/stores/ui.svelte.ts';
      const { ui } = await import(/* @vite-ignore */ path);
      ui.setScheme(scheme);
    }, scheme);
    await expect(page.locator('html')).toHaveAttribute('data-scheme', scheme);
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`${name}-${scheme}.png`) });
  }
}

async function vaultPage(page: Page): Promise<void> {
  const ws = await workspace(page);
  const { ctx, base } = await apiCtx();
  const { vaultId } = await seedVaultDir(ctx, base, ws);
  await ctx.dispose();
  await page.addInitScript(({ ws, vaultId }) => {
    localStorage.setItem(`otto_vault_last:${ws}`, String(vaultId));
    localStorage.setItem('otto_vault_right_open', '1');
  }, { ws, vaultId });
  await page.goto('/#/vault');
  await expect(page.locator('.tree')).toBeVisible();
  await page.locator('.tree').getByText('services', { exact: true }).click();
}

async function vaultMenu(page: Page): Promise<void> {
  await page.locator('.vault-header').getByRole('button', { name: /^(More vault actions|More actions)$/ }).locator('visible=true').last().click();
  const submenu = page.getByRole('menuitem', { name: 'More vault actions…', exact: true });
  const details = page.getByRole('menuitemcheckbox', { name: /Note details/ });
  await expect(submenu.or(details)).toBeVisible();
  if (await submenu.isVisible()) await submenu.click();
}

test('phone Vault opens note details only on request and keeps them reachable', async ({ page }, info) => {
  await vaultPage(page);
  await page.locator('.tree').getByText('auth-api', { exact: true }).click();
  await expect(page.locator('.read').getByRole('heading', { name: 'Overview' })).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'Side panel', exact: true })).toBeHidden();
  await vaultMenu(page);
  await page.getByRole('menuitemcheckbox', { name: /Note details/ }).click();
  const details = page.getByRole('dialog', { name: 'Side panel', exact: true });
  await expect(details).toBeVisible();
  await expect(details).toContainText('Backlinks');
  await expectFullyInViewport(page, details);
  await capture(page, info, 'vault-details');
  await details.getByRole('button', { name: 'Close Side panel', exact: true }).click();
  await expect(details).toBeHidden();
  await expectNoHorizontalOverflow(page);
  await capture(page, info, 'vault-note');
});

test('phone Vault shows a failed note open with Retry while browsing files', async ({ page }, info) => {
  await vaultPage(page);
  let fail = true;
  await page.route('**/vaults/*/note?*', async (r) => {
    if (fail && new URL(r.request().url()).searchParams.get('path') === 'services/auth-api.md') {
      await r.fulfill({ status: 503, json: { error: 'Fixture temporarily unavailable' } });
    } else await r.continue();
  });
  await page.locator('.tree').getByText('auth-api', { exact: true }).click();
  const error = page.getByTestId('note-open-error');
  await expect(error).toBeVisible();
  await expect(error).toContainText('auth-api.md');
  await expectFullyInViewport(page, error);
  await capture(page, info, 'vault-retry');
  fail = false;
  await error.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.locator('.read').getByRole('heading', { name: 'Overview' })).toBeVisible();
  await expect(error).toHaveCount(0);
});

test('phone Vault edits autosave and remain readable after reload', async ({ page }, info) => {
  await vaultPage(page);
  await page.locator('.tree').getByText('auth-api', { exact: true }).click();
  await page.getByRole('button', { name: 'Edit', exact: true }).click();
  const editor = page.locator('.note-view .cm-content');
  const saved = page.waitForResponse(r => r.request().method() === 'PUT' && r.url().endsWith('/note') && r.ok());
  await editor.fill('# Edited on phone\n\nA local note saved from the mobile editor.');
  await saved;
  await page.getByRole('button', { name: 'Read', exact: true }).click();
  await page.reload();
  await expect(page.locator('.read').getByRole('heading', { name: 'Edited on phone' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await capture(page, info, 'vault-edited');
});

test('phone Connections can create and edit a profile and use its terminal', async ({ page }, info) => {
  await workspace(page);
  const name = `Phone terminal ${info.project.name}`;
  await page.goto('/#/connections');
  await page.getByRole('button', { name: 'New connection', exact: true }).first().click();
  const form = page.getByRole('dialog', { name: 'New connection', exact: true });
  await expectFullyInViewport(page, form);
  await form.getByLabel('Name', { exact: true }).fill(name);
  await form.getByRole('button', { name: 'Custom', exact: true }).click();
  await form.locator('#cf-template').fill('/bin/sh');
  await form.getByRole('button', { name: 'Create connection', exact: true }).click();
  await expect(form).toHaveCount(0);
  const row = page.locator('.conn-row', { hasText: name });
  await row.getByRole('button', { name: `Actions for ${name}` }).click();
  await expectFullyInViewport(page, page.getByRole('menu'));
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  const edit = page.getByRole('dialog', { name: `Edit ${name}`, exact: true });
  await expect(edit.locator('#cf-template')).toHaveValue('/bin/sh');
  await capture(page, info, 'connections-edit');
  await edit.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(edit).toHaveCount(0);
  const opened = page.waitForResponse(r => r.url().endsWith('/open') && r.request().method() === 'POST' && r.ok());
  await row.locator('.conn-item').click();
  const session = await (await opened).json() as { id: string };
  const terminal = page.locator('.term-pane .term-host');
  await expect(terminal).toBeVisible();
  expect((await terminal.boundingBox())!.height, 'terminal must have usable height on a phone').toBeGreaterThan(180);
  const { ctx, base } = await apiCtx();
  const input = await ctx.post(`${base}/api/v1/sessions/${session.id}/input`, { data: { text: "printf 'PHONE-OUTPUT-%s\\n' ready", submit: true } });
  expect(input.ok()).toBeTruthy();
  await ctx.dispose();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __ottoTermProbe: { disposed: boolean; text(): string }[] }).__ottoTermProbe.find(p => !p.disposed)?.text() ?? '')).toContain('PHONE-OUTPUT-ready');
  await terminal.scrollIntoViewIfNeeded();
  await expectNoHorizontalOverflow(page);
  await capture(page, info, 'connections-terminal');
  await page.getByRole('button', { name: 'Close terminal tab', exact: true }).click();
});

test('phone Product supports a populated story, draft editing and every workflow group', async ({ page }, info) => {
  test.setTimeout(90_000);
  const ws = await workspace(page);
  const { ctx, base } = await apiCtx();
  const { storyId } = await seedProductStory(ctx, base, ws);
  await ctx.dispose();
  await page.goto(`/#/product/${storyId}`);
  await expect(page.locator('.product-main')).toBeVisible();
  await expect(page.locator('.draft-layout')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  if (page.viewportSize()!.width <= 640) {
    await expect(page.locator('.story-details')).not.toHaveAttribute('open');
    await page.locator('.story-details > summary').click();
    await expect(page.getByLabel('Add tag', { exact: true })).toBeVisible();
    await page.locator('.story-details > summary').click();
  }
  await capture(page, info, 'product-story');
  for (const [group, sub, selector] of [
    ['Discover', 'Questions', '.qtab'],
    ['Discover', 'Notes', '.ntab'],
    ['Deliver', 'Test cases', '.tc-tab'],
    ['Deliver', 'Plan', '.plan-tab'],
    ['Log', '', '.history-tab'],
    ['Story', 'Overview', '.draft-layout'],
  ]) {
    await page.locator('.tab-strip:not(.sub-tab-strip) .st', { hasText: group }).click();
    if (sub) await page.locator('.sub-tab-strip .st', { hasText: sub }).click();
    await expect(page.locator(selector)).toBeVisible();
    await expectNoHorizontalOverflow(page);
  }
  const editedTitle = `Phone edited ${storyId}`;
  const title = page.locator('#draft-title');
  await title.fill(editedTitle);
  await page.getByRole('button', { name: 'Save draft', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Saved', exact: true })).toBeDisabled();
  await page.reload();
  await expect(page.locator('#draft-title')).toHaveValue(editedTitle);
  if (page.viewportSize()!.width <= 640) {
    await page.getByRole('button', { name: 'Back to stories', exact: true }).click();
    await expect(page.locator('.story-list')).toBeVisible();
    await page.locator('.story-row', { hasText: editedTitle }).click();
    await expect(page.locator('#draft-title')).toBeVisible();
  }
  await page.getByRole('tab', { name: 'Learnings', exact: true }).locator('visible=true').click();
  await expect(page.locator('.learnings-view')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await capture(page, info, 'product-learnings');
});
