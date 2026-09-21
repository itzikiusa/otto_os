import { test, expect } from '@playwright/test';
const config = (goal: string, version = 0) => ({ skills: null, soul: null, extra_context_md: '', include_memory: true, include_repo_map: false, goal_md: goal, memory_md: '', decisions_md: '', references: [], artifacts: [], context_version: version });

test('workspace context previews all shared fields, sends version and preserves draft on conflict', async ({ page }) => {
  let stored = config('Initial goal', 3);
  let rejectSave = false;
  let saved: Record<string, unknown> | undefined;
  let preview: Record<string, unknown> | undefined;
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith('/context/preview')) { preview = route.request().postDataJSON(); return route.fulfill({ json: { providers: [] } }); }
    if (path.endsWith('/context') && route.request().method() === 'PUT') {
      saved = route.request().postDataJSON();
      if (rejectSave) return route.fulfill({ status: 409, json: { code: 'conflict', message: 'Workspace context changed; reload before saving.' } });
      stored = { ...stored, ...saved, context_version: stored.context_version + 1 };
      return route.fulfill({ json: stored });
    }
    if (path.endsWith('/context')) return route.fulfill({ json: stored });
    return route.fulfill({ json: [] });
  });
  await page.goto('/e2e/fixtures/workspace-context.html');
  await expect(page.getByRole('heading', { name: 'Workspace context', exact: true })).toBeVisible();
  await expect(page.getByLabel('Goal', { exact: true })).toHaveValue('Initial goal');
  await page.getByLabel('Shared instructions').fill('Keep changes local');
  await page.getByLabel('Workspace memory', { exact: true }).fill('Use the office bastion');
  await page.getByLabel('References', { exact: true }).fill('docs/contracts/api.md\nhttps://example.test/design');
  await page.getByRole('button', { name: 'Save workspace context' }).click();
  await expect.poll(() => saved?.context_version).toBe(3);
  expect(saved?.references).toEqual(['docs/contracts/api.md', 'https://example.test/design']);
  expect(saved?.memory_md).toBe('Use the office bastion');
  await page.getByRole('button', { name: 'Preview context', exact: true }).click();
  await expect.poll(() => preview?.memory_md).toBe('Use the office bastion');
  rejectSave = true;
  await page.getByLabel('Goal', { exact: true }).fill('Unsaved goal');
  await page.getByRole('button', { name: 'Save workspace context' }).click();
  await expect(page.getByRole('alert')).toContainText('changed');
  await expect(page.getByLabel('Goal', { exact: true })).toHaveValue('Unsaved goal');
  await page.getByRole('button', { name: 'Reload saved context' }).click();
  await expect(page.getByLabel('Goal', { exact: true })).toHaveValue('Initial goal');
});

test('late load and save responses cannot overwrite a different workspace draft', async ({ page }) => {
  let releaseLoad!: () => void;
  const loadBlocked = new Promise<void>((resolve) => releaseLoad = resolve);
  let firstLoad = true;
  let releaseSave!: () => void;
  const saveBlocked = new Promise<void>((resolve) => releaseSave = resolve);
  let saveStarted = false;
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith('/context/preview')) return route.fulfill({ json: { providers: [] } });
    if (path.endsWith('/context')) {
      const one = path.includes('/one/');
      if (one && route.request().method() === 'PUT') { saveStarted = true; await saveBlocked; return route.fulfill({ json: config('Old workspace saved', 1) }); }
      if (one && firstLoad) { firstLoad = false; await loadBlocked; }
      return route.fulfill({ json: config(one ? 'Goal one' : 'Goal two') });
    }
    return route.fulfill({ json: [] });
  });
  await page.goto('/e2e/fixtures/workspace-context.html');
  await expect.poll(() => firstLoad).toBe(false);
  await page.getByRole('button', { name: 'Workspace two', exact: true }).click();
  await expect(page.getByLabel('Goal', { exact: true })).toHaveValue('Goal two');
  const loaded = page.waitForResponse((r) => r.url().endsWith('/workspaces/one/context') && r.request().method() === 'GET');
  releaseLoad(); await loaded;
  await page.evaluate(() => new Promise(requestAnimationFrame));
  await expect(page.getByLabel('Goal', { exact: true })).toHaveValue('Goal two');
  await page.getByRole('button', { name: 'Workspace one', exact: true }).click();
  await expect(page.getByLabel('Goal', { exact: true })).toHaveValue('Goal one');
  await page.getByRole('button', { name: 'Save workspace context' }).click();
  await expect.poll(() => saveStarted).toBe(true);
  await page.getByRole('button', { name: 'Workspace two', exact: true }).click();
  await expect(page.getByLabel('Goal', { exact: true })).toHaveValue('Goal two');
  await page.getByLabel('Goal', { exact: true }).fill('Workspace two draft');
  const savedResponse = page.waitForResponse((r) => r.url().endsWith('/workspaces/one/context') && r.request().method() === 'PUT');
  releaseSave(); await savedResponse;
  await page.evaluate(() => new Promise(requestAnimationFrame));
  await expect(page.getByLabel('Goal', { exact: true })).toHaveValue('Workspace two draft');
  await expect(page.getByRole('button', { name: 'Save workspace context' })).toBeEnabled();
});

test('a delayed preview cannot be relabeled as a different provider or newer draft', async ({ page }) => {
  let release!: () => void;
  const blocked = new Promise<void>((resolve) => release = resolve);
  let started = false;
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith('/context/preview')) {
      started = true; await blocked;
      return route.fulfill({ json: { providers: [{ provider: 'claude', skipped: false, skills: [], soul: null, files: [], generated_instructions: 'OLD PREVIEW', instructions_file_name: 'CLAUDE.md', generated_hooks: null }] } });
    }
    if (path.endsWith('/context')) return route.fulfill({ json: config('Initial goal') });
    return route.fulfill({ json: [] });
  });
  await page.goto('/e2e/fixtures/workspace-context.html');
  await page.getByRole('button', { name: 'Preview context', exact: true }).click();
  await expect.poll(() => started).toBe(true);
  await page.getByRole('combobox', { name: 'Provider', exact: true }).selectOption('codex');
  await page.getByLabel('Goal', { exact: true }).fill('New draft');
  const response = page.waitForResponse((r) => r.url().endsWith('/context/preview'));
  release(); await response;
  await page.evaluate(() => new Promise(requestAnimationFrame));
  await expect(page.getByRole('button', { name: 'Preview context', exact: true })).toBeEnabled();
  await expect(page.getByText('OLD PREVIEW', { exact: true })).toHaveCount(0);
});
