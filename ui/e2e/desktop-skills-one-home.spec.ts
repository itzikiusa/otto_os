import { test, expect, type Page } from '@playwright/test';
import { apiCtx } from './seed';
import { expectFullyInViewport } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Skills Lab is the ONE home for skills (round 2):
//   • Settings → Skills and Settings → Skills evaluator left the Settings list;
//     their old routes forward to Skills Lab (Bundled filter / Evaluator
//     defaults sheet), and the page ⋯ holds Install all + Evaluator defaults.
//   • A save based on an older read conflicts (409) instead of silently
//     overwriting: the draft stays, "Keep mine" / "Reload theirs" decide.
//   • Deleting a skill is a danger confirm that says where it goes.
// Desktop-only: it drives the real isolated daemon's shared skill library.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

async function seedSkill(prefix: string) {
  const { ctx, base } = await apiCtx();
  const name = `${prefix}-${Date.now()}`;
  const body = '---\ndescription: One-home fixture\ncategory: review\n---\n\n# Fixture\n\nOriginal text.';
  expect((await ctx.post(`${base}/api/v1/library/skills`, { data: { name, category: 'review', description: 'One-home fixture', body } })).ok()).toBeTruthy();
  return { ctx, base, name };
}

async function openFiles(page: Page, name: string) {
  await page.goto('/#/skills-eval');
  await page.getByRole('searchbox', { name: 'Filter skills' }).fill(name);
  await page.getByTestId('skill-row').click();
  await page.getByRole('tab', { name: 'Files', exact: true }).click();
  await expect(page.getByTestId('skill-editor')).toContainText('Original text.');
}

test('API: file reads carry a hash and a stale if_hash is refused with the current one', async () => {
  const { ctx, base, name } = await seedSkill('one-home-api');
  try {
    const read = await (await ctx.get(`${base}/api/v1/library/skills/${name}/file?path=SKILL.md`)).json();
    expect(read.hash).toMatch(/^[0-9a-f]{64}$/);
    // Someone else writes first.
    expect((await ctx.put(`${base}/api/v1/library/skills/${name}/file`, { data: { path: 'SKILL.md', content: '# theirs' } })).ok()).toBeTruthy();
    const stale = await ctx.put(`${base}/api/v1/library/skills/${name}/file`, { data: { path: 'SKILL.md', content: '# mine', if_hash: read.hash } });
    expect(stale.status()).toBe(409);
    const problem = await stale.json();
    expect(problem.code).toBe('conflict');
    expect(problem.current_hash).toMatch(/^[0-9a-f]{64}$/);
    expect((await (await ctx.get(`${base}/api/v1/library/skills/${name}/file?path=SKILL.md`)).json()).content).toBe('# theirs');
    // Re-sent against their hash it lands.
    const kept = await ctx.put(`${base}/api/v1/library/skills/${name}/file`, { data: { path: 'SKILL.md', content: '# mine', if_hash: problem.current_hash } });
    expect(kept.ok()).toBeTruthy();
    // The bundled catalogue knows the "modified" state shape.
    const bundled = await (await ctx.get(`${base}/api/v1/library/bundled`)).json();
    for (const b of bundled) expect(['not_installed', 'up_to_date', 'update_available', 'ahead', 'modified']).toContain(b.state);
  } finally {
    await ctx.delete(`${base}/api/v1/library/skills/${name}`);
    await ctx.dispose();
  }
});

test('old Settings routes land in Skills Lab and leave the Settings list', async ({ page }) => {
  await page.goto('/#/settings/skills');
  await expect(page).toHaveURL(/#\/skills-eval\/bundled$/, { timeout: 30_000 });
  await expect(page.getByTestId('skills-browser')).toBeVisible();
  await expect(page.getByRole('group', { name: 'Filter by source' }).getByRole('button', { name: /^Bundled/ })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('bundled-summary')).toContainText('bundled skills installed');

  await page.goto('/#/settings/skill-eval');
  await expect(page.getByRole('dialog', { name: 'Evaluator defaults' })).toBeVisible({ timeout: 30_000 });
  await expect(page).toHaveURL(/#\/skills-eval\/evaluator$/);
  await page.getByRole('dialog', { name: 'Evaluator defaults' }).getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByRole('dialog', { name: 'Evaluator defaults' })).toHaveCount(0);

  await page.goto('/#/settings/appearance');
  const nav = page.locator('.settings-nav-label');
  await expect(nav.first()).toBeVisible({ timeout: 30_000 });
  await expect(nav.filter({ hasText: /^Skills$/ })).toHaveCount(0);
  await expect(nav.filter({ hasText: /^Skills evaluator$/ })).toHaveCount(0);
});

test('the Lab ⋯ holds Install all bundled skills and Evaluator defaults', async ({ page }) => {
  await page.goto('/#/skills-eval');
  await page.getByTestId('lab-more').click();
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitem', { name: /Import a skill/ })).toBeVisible();
  await menu.getByRole('menuitem', { name: /Install all bundled skills/ }).click();
  const confirm = page.getByRole('dialog', { name: /Install all bundled skills/ });
  await expect(confirm).toContainText('copies you edited are left as they are');
  await confirm.getByRole('button', { name: 'Cancel' }).click();

  await page.getByTestId('tab-evaluator').click();
  await page.getByTestId('lab-more').click();
  await page.getByRole('menu').getByRole('menuitem', { name: /Evaluator defaults/ }).click();
  const sheet = page.getByRole('dialog', { name: 'Evaluator defaults' });
  await expect(sheet.getByTestId('eval-defaults')).toBeVisible();
  await expectFullyInViewport(page, sheet);
});

test('a save that lost a race shows the conflict and never drops the draft', async ({ page }) => {
  const { ctx, base, name } = await seedSkill('one-home-conflict');
  try {
    await openFiles(page, name);
    const editor = page.getByTestId('skill-editor').locator('.cm-content');
    await editor.fill('# My draft');
    // The self-improvement engine (or a review fix) writes meanwhile.
    expect((await ctx.put(`${base}/api/v1/library/skills/${name}/file`, { data: { path: 'SKILL.md', content: '# Their edit' } })).ok()).toBeTruthy();
    await page.getByTestId('save-skill').click();
    const banner = page.getByTestId('skill-conflict');
    await expect(banner).toContainText('changed since you opened it');
    await expect(page.getByTestId('skill-editor')).toContainText('# My draft');
    expect((await (await ctx.get(`${base}/api/v1/library/skills/${name}/file?path=SKILL.md`)).json()).content).toBe('# Their edit');

    await banner.getByRole('button', { name: 'Keep mine' }).click();
    await expect(banner).toHaveCount(0);
    expect((await (await ctx.get(`${base}/api/v1/library/skills/${name}/file?path=SKILL.md`)).json()).content).toBe('# My draft');

    // Reload theirs takes the version on disk.
    await editor.fill('# Second draft');
    expect((await ctx.put(`${base}/api/v1/library/skills/${name}/file`, { data: { path: 'SKILL.md', content: '# Newer edit' } })).ok()).toBeTruthy();
    await page.getByTestId('save-skill').click();
    await page.getByTestId('skill-conflict').getByRole('button', { name: 'Reload theirs' }).click();
    await expect(page.getByTestId('skill-editor')).toContainText('# Newer edit');
  } finally {
    await ctx.delete(`${base}/api/v1/library/skills/${name}`);
    await ctx.dispose();
  }
});

test('deleting a skill is a danger confirm that names the backup folder', async ({ page }) => {
  const { ctx, base, name } = await seedSkill('one-home-delete');
  try {
    await openFiles(page, name);
    await page.getByRole('button', { name: `More actions for ${name}` }).click();
    await page.getByRole('menu').getByRole('menuitem', { name: /Delete from library/ }).click();
    const confirm = page.getByRole('dialog', { name: 'Delete skill' });
    await expect(confirm).toContainText('skills-backup');
    await confirm.getByRole('button', { name: 'Delete' }).click();
    await expect.poll(async () => (await ctx.get(`${base}/api/v1/library/skills/${name}`)).status()).toBe(404);
  } finally {
    await ctx.dispose();
  }
});
