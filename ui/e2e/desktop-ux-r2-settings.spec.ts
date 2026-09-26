import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });

async function skill(page: Page) {
  const { ctx, base } = await apiCtx();
  const name = `r2-drafts-${Date.now()}`;
  const body = '---\ndescription: Synthetic review fixture\n---\n\n# Draft workflow\n\nOriginal text.';
  expect((await ctx.post(`${base}/api/v1/library/skills`, { data: { name, category: 'review', description: 'Synthetic review fixture', body } })).ok()).toBeTruthy();
  for (const path of ['references/first.md', 'references/second.md']) {
    expect((await ctx.put(`${base}/api/v1/library/skills/${name}/file`, { data: { path, content: `# ${path}` } })).ok()).toBeTruthy();
  }
  await page.goto('/#/skills-eval');
  await page.getByRole('searchbox', { name: 'Search skills' }).fill(name);
  await page.getByTestId('skill-row').click();
  await page.getByRole('tab', { name: 'Files', exact: true }).click();
  await expect(page.getByTestId('skill-editor')).toContainText('Original text.');
  return { ctx, base, name };
}

test('bulk role failures never report that every workspace was updated', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  await seedWorkspace(ctx, base);
  await seedWorkspace(ctx, base);
  expect((await ctx.post(`${base}/api/v1/users`, { data: { username: `r2-${Date.now()}`, display_name: 'Synthetic reviewer', password: 'test-password-123' } })).ok()).toBeTruthy();
  let requests = 0;
  let failAll = true;
  let failPath = '';
  const writtenPaths: string[] = [];
  await page.route('**/api/v1/workspaces/*/members', async route => {
    if (route.request().method() !== 'PUT') return route.continue();
    requests++;
    writtenPaths.push(route.request().url());
    if (!failAll && route.request().url() !== failPath) return route.continue();
    await route.fulfill({ status: 503, json: { code: 'upstream', message: 'Role write unavailable' } });
  });
  await page.goto('/#/settings/users');
  await page.getByRole('button', { name: 'By user', exact: true }).click();
  const bulk = page.getByRole('group', { name: 'Set the role in every workspace' });
  await bulk.getByRole('button', { name: 'Viewer', exact: true }).click();
  await expect(page.getByText(/Couldn’t change the workspace role/).first()).toBeVisible();
  await expect(bulk.getByRole('button', { name: 'Viewer', exact: true })).toBeEnabled();
  expect(requests).toBeGreaterThanOrEqual(2);
  expect(await page.getByText('Set to viewer in all workspaces', { exact: true }).count()).toBe(0);
  await expect(page.getByRole('group', { name: /^Role in / }).first().getByRole('button', { name: 'None', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const total = requests;
  failAll = false;
  failPath = writtenPaths[0];
  await bulk.getByRole('button', { name: 'Viewer', exact: true }).click();
  await expect(page.getByRole('group', { name: /^Role in / }).getByRole('button', { name: 'Viewer', exact: true }).and(page.locator('[aria-pressed="true"]'))).toHaveCount(total - 1);
  await expect(bulk.getByRole('button', { name: 'Viewer', exact: true })).toBeEnabled();
  expect(await page.getByText('Set to viewer in all workspaces', { exact: true }).count()).toBe(0);
  const beforeRetry = requests;
  failPath = '';
  await bulk.getByRole('button', { name: 'Viewer', exact: true }).click();
  await expect(page.getByText('Set to viewer in all workspaces', { exact: true })).toBeVisible();
  expect(requests - beforeRetry).toBe(1);
  await ctx.dispose();
});

test('Skills latest file selection wins over a delayed earlier file load', async ({ page }) => {
  const { ctx, name } = await skill(page);
  let release!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  let firstStarted = false;
  await page.route(`**/api/v1/library/skills/${name}/file?path=*`, async route => {
    const path = new URL(route.request().url()).searchParams.get('path');
    if (path !== 'references/first.md') return route.continue();
    firstStarted = true;
    await held;
    await route.fulfill({ json: { content: '# Delayed first document', binary: false } });
  });
  await page.getByRole('button', { name: 'references/first.md', exact: true }).click();
  await expect.poll(() => firstStarted).toBe(true);
  await page.getByRole('button', { name: 'references/second.md', exact: true }).click();
  await expect(page.getByTestId('skill-editor')).toContainText('# references/second.md');
  const firstResponse = page.waitForResponse(r => r.url().includes('file?path=references%2Ffirst.md'));
  release();
  await (await firstResponse).finished();
  await expect(page.getByTestId('skill-editor')).toContainText('# references/second.md');
  await ctx.dispose();
});

test('Skills editing during Save keeps newer text dirty and persists it on the next Save', async ({ page }) => {
  const { ctx, base, name } = await skill(page);
  let release!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  let saves = 0;
  await page.route(`**/api/v1/library/skills/${name}/file`, async route => {
    if (route.request().method() !== 'PUT') return route.continue();
    saves++;
    if (saves === 1) await held;
    await route.continue();
  });
  const editor = page.getByTestId('skill-editor').locator('.cm-content');
  await editor.fill('# Submitted text');
  await page.getByTestId('save-skill').click();
  await expect.poll(() => saves).toBe(1);
  await editor.fill('# Newer unsaved text');
  release();
  await expect(page.getByTestId('save-skill')).toHaveText('Save');
  await expect(page.getByTestId('save-skill')).toBeEnabled();
  await expect(page.getByText('Unsaved', { exact: true })).toBeVisible();
  const saved = await ctx.get(`${base}/api/v1/library/skills/${name}/file?path=SKILL.md`);
  expect((await saved.json()).content).toBe('# Submitted text');
  await editor.press('Control+s');
  await expect(page.getByTestId('save-skill')).toBeDisabled();
  const newer = await ctx.get(`${base}/api/v1/library/skills/${name}/file?path=SKILL.md`);
  expect((await newer.json()).content).toBe('# Newer unsaved text');
  await ctx.dispose();
});

test('long plugin metadata leaves its name and actions usable on phone', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.route('**/api/v1/plugin-admin', route => route.fulfill({ json: [{
    slug: 'synthetic-plugin', name: 'Synthetic reporting plugin with a deliberately long name', version: '2026.09.25-preview.enterprise.integration.1234567890', description: 'Reports from local synthetic fixtures', source: '/tmp/synthetic-plugin', exec: [], ui_dir: null, health: 'unknown', enabled: false, installed_at: '2026-09-25T12:00:00Z', icon: 'box',
  }] }));
  await page.goto('/#/settings/plugins');
  await expect(page.locator('.plist .name-text')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  const row = page.locator('.plist .prow');
  const overflow = await row.evaluate(el => ({ width: el.clientWidth, scroll: el.scrollWidth, name: el.querySelector('.name-text')!.getBoundingClientRect().width }));
  expect(overflow.scroll).toBeLessThanOrEqual(overflow.width + 1);
  expect(overflow.name).toBeGreaterThan(50);
  await page.screenshot({ path: '/tmp/otto-ux-r2-settings-plugins-phone.png' });
  let mutations = 0;
  await page.route('**/api/v1/plugin-admin/**', route => {
    mutations++;
    return route.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic plugin unavailable' } });
  });
  await page.getByRole('button', { name: /^Remove Synthetic/ }).click();
  await expect(page.getByRole('dialog')).toContainText('Its files under ~/otto-plugins are kept');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(mutations).toBe(0);
  await page.getByRole('button', { name: 'Enable', exact: true }).click();
  await expect(page.getByText(/Couldn’t enable Synthetic/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Enable', exact: true })).toBeEnabled();
  await expect(page.locator('.plist')).toContainText('Disabled');
});

test('appearance choices persist across reload and Pro Dark explains its fixed scheme', async ({ page }) => {
  await page.goto('/#/settings/appearance');
  await page.getByRole('button', { name: /^Warm / }).click();
  await page.getByRole('group', { name: 'Scheme', exact: true }).getByRole('button', { name: 'Light', exact: true }).click();
  await page.reload();
  await expect(page.getByRole('button', { name: /^Warm / })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('group', { name: 'Scheme', exact: true }).getByRole('button', { name: 'Light', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: /^Pro Dark / }).click();
  await expect(page.getByRole('group', { name: 'Scheme', exact: true }).getByRole('button', { name: 'Light', exact: true })).toBeDisabled();
  await expect(page.getByText('Pro Dark is always dark. Pick Native or Warm to use a light scheme.')).toBeVisible();
});

test('failed permission setting preserves the saved value and can be retried', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  expect((await ctx.put(`${base}/api/v1/settings`, { data: { agent_skip_permissions: true } })).ok()).toBeTruthy();
  let fail = true;
  await page.route('**/api/v1/settings', route => route.request().method() === 'PUT' && fail
    ? route.fulfill({ status: 503, json: { code: 'upstream', message: 'Permission write unavailable' } }) : route.continue());
  await page.goto('/#/settings/providers');
  const toggle = page.getByRole('checkbox', { name: /Skip permission prompts/ });
  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(page.getByText('Couldn’t save the permission setting', { exact: true })).toBeVisible();
  await expect(toggle).toBeChecked();
  fail = false;
  await toggle.uncheck();
  await expect(page.getByText('Permission prompts enabled', { exact: true })).toBeVisible();
  await page.reload();
  await expect(toggle).not.toBeChecked();
  await ctx.dispose();
});

test('plugin install failure preserves source for keyboard retry without duplicate requests', async ({ page }) => {
  let posts = 0;
  let release!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  await page.route('**/api/v1/plugin-admin', route => route.fulfill({ json: [] }));
  await page.route('**/api/v1/plugin-admin/install', async route => {
    posts++;
    if (posts === 1) await held;
    await route.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic install unavailable' } });
  });
  await page.goto('/#/settings/plugins');
  const source = page.getByLabel('Plugin source');
  await source.fill('/tmp/synthetic-plugin');
  await source.press('Enter');
  await expect(page.getByRole('button', { name: 'Installing…' })).toBeDisabled();
  await source.press('Enter');
  expect(posts).toBe(1);
  release();
  await expect(page.getByRole('alert')).toContainText('Synthetic install unavailable');
  await expect(source).toHaveValue('/tmp/synthetic-plugin');
  await expect(source).toHaveAttribute('aria-invalid', 'true');
  await source.press('Enter');
  await expect.poll(() => posts).toBe(2);
  await expect(page.getByRole('button', { name: 'Install', exact: true })).toBeEnabled();
});

test('MCP delayed attachment save does not overwrite a newly selected workspace', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const a = await seedWorkspace(ctx, base);
  const b = await seedWorkspace(ctx, base);
  expect((await ctx.patch(`${base}/api/v1/workspaces/${a}`, { data: { name: 'Synthetic Alpha' } })).ok()).toBeTruthy();
  expect((await ctx.patch(`${base}/api/v1/workspaces/${b}`, { data: { name: 'Synthetic Beta' } })).ok()).toBeTruthy();
  await page.addInitScript(id => localStorage.setItem('otto_workspace', id), a);
  let release!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  let started = false;
  await page.route(`**/api/v1/workspaces/${a}/mcp/session-attach`, async route => {
    if (route.request().method() === 'GET') return route.fulfill({ json: { workspace_id: a, attached: true } });
    started = true;
    await held;
    await route.fulfill({ json: { workspace_id: a, attached: false } });
  });
  await page.route(`**/api/v1/workspaces/${b}/mcp/session-attach`, route => route.fulfill({ json: { workspace_id: b, attached: true } }));
  await page.goto('/#/mcp');
  const toggle = page.getByTestId('mcp-session-attach');
  await expect(toggle).toBeChecked();
  await toggle.uncheck();
  await expect.poll(() => started).toBe(true);
  await page.keyboard.press('Meta+k');
  await page.getByRole('combobox').fill('Switch workspace: Synthetic Beta');
  await page.keyboard.press('Enter');
  await expect(page.getByText('Attach to sessions in Synthetic Beta', { exact: true })).toBeVisible();
  const oldSave = page.waitForResponse(r => r.url().includes(`/workspaces/${a}/mcp/session-attach`) && r.request().method() === 'PATCH');
  release();
  await (await oldSave).finished();
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => resolve())));
  await expect(toggle).toBeEnabled();
  await expect(toggle).toBeChecked();
  await ctx.dispose();
});

test('synthetic loaded pages remain readable across five themes and responsive directions', async ({ page }) => {
  test.setTimeout(120_000);
  const { ctx, name } = await skill(page);
  await page.route('**/api/v1/plugin-admin', route => route.fulfill({ json: [{
    slug: 'synthetic-plugin', name: 'Synthetic reporting plugin with a deliberately long name', version: '2026.09.25-preview.enterprise.integration.1234567890', description: 'Reports from local synthetic fixtures', source: '/tmp/synthetic-plugin', exec: [], ui_dir: null, health: 'unknown', enabled: false, installed_at: '2026-09-25T12:00:00Z', icon: 'box',
  }] }));
  const variants = [
    { key: 'native-light', theme: 'native', scheme: 'light', width: 1440, height: 900, direction: 'ltr' },
    { key: 'native-dark-phone', theme: 'native', scheme: 'dark', width: 375, height: 812, direction: 'ltr' },
    { key: 'warm-light-tablet-rtl', theme: 'warm', scheme: 'light', width: 834, height: 1112, direction: 'rtl' },
    { key: 'warm-dark', theme: 'warm', scheme: 'dark', width: 1440, height: 900, direction: 'ltr' },
    { key: 'pro-dark-phone', theme: 'pro-dark', scheme: 'dark', width: 375, height: 812, direction: 'ltr' },
  ];
  for (const v of variants) {
    await page.setViewportSize({ width: v.width, height: v.height });
    await page.evaluate(v => {
      localStorage.setItem('otto_theme', v.theme);
      localStorage.setItem('otto_scheme', v.scheme);
      localStorage.setItem('otto_direction', v.direction);
    }, v);
    await page.reload();
    for (const surface of ['appearance', 'plugins', 'mcp', 'skills']) {
      await page.goto(surface === 'mcp' ? '/#/mcp' : surface === 'skills' ? '/#/skills-eval' : `/#/settings/${surface}`);
      if (surface === 'skills') {
        await page.getByRole('searchbox', { name: 'Search skills' }).fill(name);
        await page.getByTestId('skill-row').click();
        await page.getByRole('tab', { name: 'Files', exact: true }).click();
        await expect(page.getByTestId('skill-editor')).toContainText('Original text.');
        expect((await page.getByTestId('skill-editor').boundingBox())!.width).toBeGreaterThan(200);
      } else if (surface === 'plugins') {
        await expect(page.locator('.plist .prow')).toBeVisible();
        await expect(page.getByLabel('Plugin source')).toHaveCSS('direction', 'ltr');
        await expect(page.locator('.plist .psrc')).toHaveCSS('direction', 'ltr');
      } else if (surface === 'mcp') {
        await expect(page.locator('.otto .grp-name').first()).toBeVisible();
      } else {
        await expect(page.getByRole('group', { name: 'Scheme', exact: true })).toBeVisible();
      }
      await expectNoHorizontalOverflow(page);
      await page.screenshot({ path: `/tmp/otto-ux-r2-settings-${surface}-${v.key}.png` });
    }
  }
  await ctx.dispose();
});

test('Skills failed file load retries and draft-discard prompt preserves editing on cancel', async ({ page }) => {
  const { ctx, name } = await skill(page);
  let fail = true;
  await page.route(`**/api/v1/library/skills/${name}/file?path=*`, route => fail
    ? route.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic file unavailable' } }) : route.continue());
  await page.getByRole('button', { name: 'references/first.md', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Synthetic file unavailable');
  await expect(page.getByTestId('save-skill')).toHaveCount(0);
  fail = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  const editor = page.getByTestId('skill-editor').locator('.cm-content');
  await expect(editor).toContainText('references/first.md');
  await editor.fill('# Keep this draft');
  await page.getByRole('button', { name: 'references/second.md', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Discard unsaved changes?');
  await dialog.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(editor).toContainText('# Keep this draft');
  await page.getByRole('button', { name: 'Revert', exact: true }).click();
  await expect(editor).toContainText('references/first.md');
  await expect(page.getByTestId('save-skill')).toBeDisabled();
  await ctx.dispose();
});

test('MCP catalog error retries and failed exposure update preserves saved state', async ({ page }) => {
  let failLoad = true;
  await page.route('**/api/v1/mcp/otto-server', route => {
    if (route.request().method() === 'PATCH') return route.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic MCP update unavailable' } });
    return failLoad ? route.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic catalog unavailable' } }) : route.continue();
  });
  await page.goto('/#/mcp');
  const error = page.getByTestId('load-error');
  await expect(error).toContainText('Synthetic catalog unavailable');
  await expect(page.getByTestId('mcp-outward-enabled')).toBeDisabled();
  failLoad = false;
  await error.getByRole('button', { name: 'Retry' }).click();
  const toggle = page.getByTestId('mcp-outward-enabled');
  await expect(toggle).toBeEnabled();
  const before = await toggle.isChecked();
  // A failed write may revert before WebKit's setChecked postcondition runs.
  await toggle.click();
  await expect(page.getByText('Update failed', { exact: true })).toBeVisible();
  expect(await toggle.isChecked()).toBe(before);
});
